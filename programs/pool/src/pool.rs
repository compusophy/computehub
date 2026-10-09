//! The pool: this tab, the tabs linked to it, and one job they share. Pure: the clock comes in
//! as page ms, and what to do goes out as [`Act`]s.
//!
//! A job is a program (`/bin/<name>`, which every tab has: a link never ships code) and its
//! chunks, each a line. The tab that starts it holds the queue. Each of its workers takes the
//! next chunk as it goes idle; a linked tab's workers take theirs in batches it asks for
//! ([`Msg::Want`]: its idle workers' worth and half as many again, so they never wait on the
//! link). When the queue is empty an idle worker here takes back the chunk out longest on another
//! tab (work stealing: the first answer counts, a second is compared with it). A chunk whose
//! worker or link is gone goes back to the queue's front. A worker answers a chunk with its
//! fuel and the SHA-256 of its result; every [`CHECK`]th chunk another tab answered is replayed
//! here and the hashes compared, so a result can be checked as well as addressed.
//!
//! A worker is a process of the job's program, `work` its argument, on a console of its own: it
//! says `ready` once it reads lines without echoing them, then answers each line
//! `<index> <input>` with one line `<index> <fuel> <sha256> <result>`.

use std::collections::VecDeque;

use uiwire::pool::{self, Snap};

use crate::model::{Asking, Shared};
use crate::msg::Msg;
use crate::test::{Measured, Test};

/// Every this many chunks, one another tab answered is replayed here.
pub const CHECK: u32 = 8;
/// The most workers a tab runs, whatever its cores.
pub const MAX_WORKERS: u16 = 32;
/// ms between a tab's stats to its links, between pings, and that idle workers wait before
/// they end.
pub const TICK: u64 = 1000;
pub const PING: u64 = 2000;
pub const IDLE: u64 = 30_000;
/// A link silent this long (no stats, no pong) is gone: its tab closed, slept or lost its way.
pub const SILENT: u64 = 15_000;
/// A measurement's bytes each way, in messages of [`PROBE`] bytes.
pub const PROBE_BYTES: u32 = 1 << 20;
pub const PROBE: u32 = 16 << 10;
/// ms a tab waits to ask again after an empty answer.
const DRY: u64 = 500;

/// What the pool asks of the desktop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    /// Send a message on a link.
    Send(u32, Msg),
    /// Write a line (a newline follows) to a worker's console.
    Feed(u32, String),
    /// Start this many more workers of `/bin/<name> work`, telling [`Pool::spawned`] each pid.
    Spawn(String, u16),
    /// End these workers.
    Stop(Vec<u32>),
    /// Close a link that went silent.
    Unlink(u32),
    /// Tell the job's window that chunk `index` was answered, by device `node`.
    Done { window: u32, index: u32, node: u8, out: String },
    /// Post a chat to the model this tab shares ([`crate::model`]): its server's URL and body.
    Post { url: String, body: String },
    /// Get what the model's server says of itself (its `/props`): [`Pool::props`] hears it.
    Get(String),
}

/// What a tab has to lend ([`Msg::Hello`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Info {
    pub name: String,
    pub kind: String,
    pub cores: u16,
    pub ram_mb: u32,
    pub quota_mb: u32,
    pub gpu: bool,
    pub model: String,
    pub tok: u32,
    pub ctx: u32,
    pub measured: Measured,
}

/// A linked tab: what it has and says (workers, busy, chunks, units), when it linked and was last
/// heard, whether its key was pinned before, the round trip (ms), bytes sent and heard, the
/// throughput measured each way (bytes a second), and the run of a measurement being heard
/// (when, bytes).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Peer {
    pub link: u32,
    pub info: Info,
    pub stats: (u16, u16, u32, u64),
    pub since: u64,
    pub last: u64,
    pub known: bool,
    pub rtt: u32,
    pub tx: u64,
    pub rx: u64,
    pub up: u32,
    pub down: u32,
    probe: Option<(u64, u32)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Own,
    Check,
    Help,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Task {
    job: u32,
    index: u32,
    kind: Kind,
}

/// A worker: its pid, whether it reads lines yet, its chunk and that chunk's line, and output
/// short of a line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worker {
    pub pid: u32,
    ready: bool,
    task: Option<Task>,
    input: String,
    buf: Vec<u8>,
}

/// A chunk of this tab's job: waiting in the queue, out on a link (0: here) since when, or done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Waiting,
    Out(u32, u64),
    Done,
}

/// The job this tab started, for the window `window`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub id: u32,
    pub name: String,
    pub window: u32,
    inputs: Vec<String>,
    state: Vec<State>,
    hashes: Vec<String>,
    queue: VecDeque<u32>,
    checks: VecDeque<u32>,
    pub done: u32,
    pub steals: u32,
    pub requeued: u32,
    pub checked: u32,
    pub mismatched: u32,
    /// Chunks answered by link (0 here).
    per: Vec<(u32, u32)>,
    started: u64,
    pub ended: Option<u64>,
}

/// A job another tab started that this one helps: the link it came on, its id and program, the
/// chunks given and not begun, whether an ask is out, when to ask again, chunks answered here,
/// when it started, last answered here and ended (kept after it ends, for the Pool page's record
/// of it).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Help {
    owner: u32,
    job: u32,
    name: String,
    leased: VecDeque<(u32, String)>,
    asked: bool,
    next: u64,
    done: u32,
    used: u32,
    started: u64,
    last: u64,
    ended: Option<u64>,
}

/// This tab's pool.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pool {
    pub me: Info,
    pub peers: Vec<Peer>,
    pub workers: Vec<Worker>,
    program: String,
    pub job: Option<Job>,
    help: Option<Help>,
    last_job: u32,
    /// Workers asked for and not yet started.
    pending: u16,
    /// Chunks the workers here answered, and their fuel (cumulative).
    pub chunks: u32,
    pub units: u64,
    idle: u64,
    next_tick: u64,
    next_ping: u64,
    /// The model this tab shares, the answer it asked the pool's for, and its last ask's name.
    pub shared: Shared,
    pub asking: Option<Asking>,
    pub last_ask: u32,
    /// The device's test, under way or over, and the Unix time as the hub last read it.
    pub test: Option<Test>,
    pub clock: u32,
    pub out: Vec<Act>,
}

/// Leading decimal digits of `s` and what follows the space after them.
pub fn num(s: &str) -> (Option<u64>, &str) {
    let end = s.bytes().position(|b| !b.is_ascii_digit()).unwrap_or(s.len());
    let n = s.as_bytes()[..end]
        .iter()
        .try_fold(0u64, |n, d| n.checked_mul(10)?.checked_add(u64::from(d - b'0')));
    (n.filter(|_| end > 0), s.get(end + 1..).unwrap_or(""))
}

/// `n` in decimal after `out`.
pub fn dec(out: &mut String, n: u64) {
    if n >= 10 {
        dec(out, n / 10);
    }
    out.push(char::from(b'0' + (n % 10) as u8));
}

impl Pool {
    pub fn new(me: Info) -> Pool {
        Pool { me, ..Pool::default() }
    }

    pub(crate) fn send_all(&mut self, m: &Msg) {
        for p in &self.peers {
            self.out.push(Act::Send(p.link, m.clone()));
        }
    }

    pub(crate) fn hello(&self) -> Msg {
        let i = &self.me;
        let (name, kind, model) = (i.name.clone(), i.kind.clone(), i.model.clone());
        Msg::Hello {
            name,
            kind,
            cores: i.cores,
            ram_mb: i.ram_mb,
            quota_mb: i.quota_mb,
            gpu: i.gpu,
            model,
            tok: i.tok,
            ctx: i.ctx,
            cpu1: i.measured.cpu1,
            cpun: i.measured.cpun,
            mem: i.measured.mem,
            tested: i.measured.tested,
        }
    }

    /// The device index of `link` in a [`Snap`]: 0 here, else its place among the peers plus 1.
    pub fn node(&self, link: u32) -> u8 {
        let i = self.peers.iter().position(|p| p.link == link);
        i.map_or(0, |i| (i + 1).min(255) as u8)
    }

    /// Link `link` opened at `now`: say what this tab has, and the job on it.
    pub fn linked(&mut self, link: u32, known: bool, now: u64) {
        let rtt = pool::UNKNOWN;
        self.peers.push(Peer { link, since: now, last: now, known, rtt, ..Peer::default() });
        self.out.push(Act::Send(link, self.hello()));
        if let Some(j) = self.job.as_ref().filter(|j| j.ended.is_none()) {
            self.out.push(Act::Send(link, Msg::Job { job: j.id, name: j.name.clone() }));
        }
        self.next_tick = self.next_tick.min(now + TICK);
    }

    /// Link `link` is gone at `now`: its chunks go back to the queue, and a job it started ends.
    pub fn unlinked(&mut self, link: u32, now: u64) {
        self.peers.retain(|p| p.link != link);
        self.lost(link);
        if let Some(j) = &mut self.job {
            for i in (0..j.state.len()).rev() {
                if matches!(j.state[i], State::Out(l, _) if l == link) {
                    (j.state[i], j.requeued) = (State::Waiting, j.requeued + 1);
                    j.queue.push_front(i as u32);
                }
            }
        }
        self.end_help(|h| h.owner == link, now);
    }

    /// The job helped ends, if `which`: its chunks not begun are dropped.
    fn end_help(&mut self, which: impl Fn(&Help) -> bool, now: u64) {
        if let Some(h) = self.help.as_mut().filter(|h| h.ended.is_none() && which(h)) {
            (h.ended, h.asked) = (Some(now), false);
            h.leased.clear();
        }
    }

    /// The window `window` starts a job of `/bin/<name>` on `inputs`, ending the last (and a test
    /// under way).
    pub fn start(&mut self, window: u32, name: &str, inputs: Vec<String>, now: u64) {
        if let Some(t) = self.test.as_mut().filter(|t| !t.over) {
            t.finish("A job started: test again", &mut self.out);
        }
        if let Some(old) = self.job.take().filter(|j| j.ended.is_none()) {
            self.end(&old);
        }
        self.last_job += 1;
        let n = inputs.len();
        self.job = Some(Job {
            id: self.last_job,
            name: name.into(),
            window,
            inputs,
            state: vec![State::Waiting; n],
            hashes: vec![String::new(); n],
            queue: (0..n as u32).collect(),
            checks: VecDeque::new(),
            done: 0,
            steals: 0,
            requeued: 0,
            checked: 0,
            mismatched: 0,
            per: Vec::new(),
            started: now,
            ended: (n == 0).then_some(now),
        });
        self.send_all(&Msg::Job { job: self.last_job, name: name.into() });
        self.want_workers(name);
        self.assign(now);
    }

    /// Workers of `name`, one a core: those of another program end first.
    fn want_workers(&mut self, name: &str) {
        if self.program != name {
            if !self.workers.is_empty() {
                self.out.push(Act::Stop(self.workers.drain(..).map(|w| w.pid).collect()));
            }
            self.program = name.into();
        }
        let want = usize::from(self.me.cores.clamp(1, MAX_WORKERS));
        let have = self.workers.len() + usize::from(self.pending);
        if let Some(n) = want.checked_sub(have).filter(|n| *n > 0) {
            self.pending += n as u16;
            self.out.push(Act::Spawn(name.into(), n as u16));
        }
    }

    /// Tests this device at `now` ([`crate::test`]): never while a job runs or is helped; idle
    /// job workers end first, so the test's run alone.
    pub fn test(&mut self, now: u64) {
        let job = self.job.as_ref().is_some_and(|j| j.ended.is_none());
        let help = self.help.as_ref().is_some_and(|h| h.ended.is_none());
        if job || help || self.pending > 0 {
            self.test = Some(Test::refused("Busy with a job: test when it ends"));
            return;
        }
        if self.test.as_ref().is_some_and(|t| !t.over) {
            return;
        }
        if !self.workers.is_empty() {
            self.out.push(Act::Stop(self.workers.drain(..).map(|w| w.pid).collect()));
            self.program.clear();
        }
        self.test = Some(Test::start(self.me.cores, now, &mut self.out));
    }

    /// A test just over: what it measured is this tab's, told to every link.
    fn tested(&mut self) {
        let Some(t) = self.test.as_ref().filter(|t| t.over && t.measured.cpu1 > 0) else { return };
        if self.me.measured.tested == 0 || t.measured.cpu1 != self.me.measured.cpu1 {
            self.me.measured = Measured { tested: self.clock.max(1), ..t.measured };
            let hello = self.hello();
            self.send_all(&hello);
        }
    }

    /// The desktop started worker `pid` (`None`: could not).
    pub fn spawned(&mut self, pid: Option<u32>) {
        if let Some(t) = self.test.as_mut().filter(|t| t.starting()) {
            t.spawned(pid, &mut self.out);
            return self.tested();
        }
        self.pending = self.pending.saturating_sub(1);
        let (task, input, buf) = (None, String::new(), Vec::new());
        if let Some(pid) = pid {
            self.workers.push(Worker { pid, ready: false, task, input, buf });
        }
    }

    /// Worker `pid` ended: its chunk goes back.
    pub fn ended(&mut self, pid: u32, now: u64) {
        if let Some(t) = self.test.as_mut().filter(|t| t.has(pid)) {
            t.ended(pid, &mut self.out);
            return self.tested();
        }
        let Some(i) = self.workers.iter().position(|w| w.pid == pid) else { return };
        let w = self.workers.remove(i);
        if let Some(t) = w.task {
            match (t.kind, &mut self.job, &mut self.help) {
                (Kind::Own, Some(j), _) if j.id == t.job => {
                    if j.state[t.index as usize] != State::Done {
                        (j.state[t.index as usize], j.requeued) = (State::Waiting, j.requeued + 1);
                        j.queue.push_front(t.index);
                    }
                }
                (Kind::Check, Some(j), _) if j.id == t.job => j.checks.push_front(t.index),
                (Kind::Help, _, Some(h)) if h.job == t.job => {
                    h.leased.push_front((t.index, w.input))
                }
                _ => {}
            }
        }
        self.assign(now);
    }

    /// Output of worker `pid`: each whole line is its answer.
    pub fn output(&mut self, pid: u32, bytes: &[u8], now: u64) {
        if let Some(t) = self.test.as_mut().filter(|t| t.has(pid)) {
            t.output(pid, bytes, now, &mut self.out);
            return self.tested();
        }
        let Some(w) = self.workers.iter_mut().find(|w| w.pid == pid) else { return };
        w.buf.extend_from_slice(bytes);
        let mut lines = Vec::new();
        while let Some(nl) = w.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = w.buf.drain(..=nl).filter(|b| *b != b'\r' && *b != b'\n').collect();
            lines.push(String::from_utf8_lossy(&line).into_owned());
        }
        lines.into_iter().for_each(|l| self.line(pid, &l, now));
        self.assign(now);
    }

    fn line(&mut self, pid: u32, line: &str, now: u64) {
        let Some(w) = self.workers.iter_mut().find(|w| w.pid == pid) else { return };
        if line == "ready" {
            w.ready = true;
            return;
        }
        let (index, out) = num(line);
        // Not its chunk's answer (a stray line): it still works on the chunk.
        let Some(t) = w.task.filter(|t| index == Some(u64::from(t.index))) else { return };
        w.task = None;
        self.chunks += 1;
        self.units += num(out).0.unwrap_or(0);
        match t.kind {
            Kind::Own => self.answered(0, t.job, t.index, out.into(), now),
            Kind::Check => {
                if let Some(j) = self.job.as_mut().filter(|j| j.id == t.job) {
                    j.checked += 1;
                    j.mismatched += u32::from(j.hashes[t.index as usize] != hash(out));
                }
            }
            Kind::Help => {
                if let Some(h) = self.help.as_mut().filter(|h| h.job == t.job) {
                    (h.done, h.last) = (h.done + 1, now);
                    let done = Msg::Done { job: t.job, index: t.index, out: out.into() };
                    self.out.push(Act::Send(h.owner, done));
                }
            }
        }
    }

    /// Chunk `index` of job `id` answered on link `link` (0 here).
    fn answered(&mut self, link: u32, id: u32, index: u32, out: String, now: u64) {
        let node = self.node(link);
        let Some(j) = self.job.as_mut().filter(|j| j.id == id) else { return };
        let Some(st) = j.state.get_mut(index as usize) else { return };
        let h = hash(&out);
        if *st == State::Done {
            // A second answer, as a stolen chunk's: a free check.
            j.checked += 1;
            j.mismatched += u32::from(j.hashes[index as usize] != h);
            return;
        }
        (*st, j.done) = (State::Done, j.done + 1);
        j.hashes[index as usize] = h.into();
        match j.per.iter_mut().find(|p| p.0 == link) {
            Some(p) => p.1 += 1,
            None => j.per.push((link, 1)),
        }
        if link != 0 && index % CHECK == 0 {
            j.checks.push_back(index);
        }
        self.out.push(Act::Done { window: j.window, index, node, out });
        if j.done as usize == j.state.len() {
            j.ended = Some(now);
            let j = j.clone();
            self.end(&j);
        }
    }

    /// Tells each link that job `j` is over, and how many of its answers were used.
    fn end(&mut self, j: &Job) {
        for p in &self.peers {
            let used = j.per.iter().find(|u| u.0 == p.link).map_or(0, |u| u.1);
            self.out.push(Act::Send(p.link, Msg::End { job: j.id, used }));
        }
    }

    /// Gives each idle, ready worker a chunk: this tab's queue, its checks, another tab's
    /// chunks, then (the queue empty) the one out longest elsewhere.
    pub fn assign(&mut self, now: u64) {
        for w in 0..self.workers.len() {
            if !self.workers[w].ready || self.workers[w].task.is_some() {
                continue;
            }
            let Some((task, input)) = self.next(now) else { break };
            let mut line = String::new();
            dec(&mut line, u64::from(task.index));
            line.push(' ');
            line.push_str(&input);
            (self.workers[w].task, self.workers[w].input) = (Some(task), input);
            self.out.push(Act::Feed(self.workers[w].pid, line));
        }
        if self.workers.iter().any(|w| w.task.is_some())
            || self.job.as_ref().is_some_and(|j| j.ended.is_none())
        {
            self.idle = now;
        }
        self.ask(now);
    }

    fn next(&mut self, now: u64) -> Option<(Task, String)> {
        if let Some(j) = self.job.as_mut().filter(|j| j.ended.is_none() || !j.checks.is_empty()) {
            let job = j.id;
            if let Some(i) = j.queue.pop_front() {
                j.state[i as usize] = State::Out(0, now);
                return Some((
                    Task { job, index: i, kind: Kind::Own },
                    j.inputs[i as usize].clone(),
                ));
            }
            if let Some(i) = j.checks.pop_front() {
                return Some((
                    Task { job, index: i, kind: Kind::Check },
                    j.inputs[i as usize].clone(),
                ));
            }
        }
        if let Some(h) = &mut self.help {
            if let Some((i, line)) = h.leased.pop_front() {
                return Some((Task { job: h.job, index: i, kind: Kind::Help }, line));
            }
        }
        let j = self.job.as_mut().filter(|j| j.ended.is_none())?;
        let oldest = (0..j.state.len()).filter_map(|i| match j.state[i] {
            State::Out(l, t) if l != 0 => Some((t, i)),
            _ => None,
        });
        let (_, i) = oldest.min()?;
        (j.state[i], j.steals) = (State::Out(0, now), j.steals + 1);
        Some((Task { job: j.id, index: i as u32, kind: Kind::Own }, j.inputs[i].clone()))
    }

    /// Helping: asks the job's tab for chunks when it has few, unless an ask is out.
    fn ask(&mut self, now: u64) {
        let idle = self.workers.iter().filter(|w| w.task.is_none()).count();
        let half = self.workers.len() / 2;
        let help = self.help.as_mut().filter(|h| h.ended.is_none() && !h.asked && h.next <= now);
        let Some(h) = help else { return };
        let n = (idle + half).saturating_sub(h.leased.len()).min(usize::from(u16::MAX));
        if n > 0 && !self.workers.is_empty() {
            h.asked = true;
            self.out.push(Act::Send(h.owner, Msg::Want { job: h.job, n: n as u16 }));
        }
    }

    /// Measures every link: a run of [`PROBE_BYTES`] out, asking for as many back.
    pub fn measure(&mut self) {
        for i in 0..self.peers.len() {
            self.run(self.peers[i].link, PROBE_BYTES, PROBE_BYTES);
        }
    }

    fn run(&mut self, link: u32, bytes: u32, back: u32) {
        let n = bytes.div_ceil(PROBE).max(1);
        for k in 0..n {
            let last = k + 1 == n;
            let fill = vec![0; PROBE as usize];
            self.out.push(Act::Send(
                link,
                Msg::Probe { last, back: if last { back } else { 0 }, fill },
            ));
        }
    }

    /// Message `m` came on link `link`.
    pub fn heard(&mut self, link: u32, m: Msg, now: u64) {
        let Some(k) = self.peers.iter().position(|p| p.link == link) else { return };
        let p = &mut self.peers[k];
        p.last = now;
        match m {
            Msg::Hello {
                name,
                kind,
                cores,
                ram_mb,
                quota_mb,
                gpu,
                model,
                tok,
                ctx,
                cpu1,
                cpun,
                mem,
                tested,
            } => {
                let measured = Measured { cpu1, cpun, mem, tested };
                p.info =
                    Info { name, kind, cores, ram_mb, quota_mb, gpu, model, tok, ctx, measured }
            }
            Msg::Ask { ask, text } => self.write(link, ask, &text),
            Msg::Words { ask, text } => self.words(link, ask, &text),
            Msg::Answered { ask, tok, why } => self.finished(link, ask, tok, why),
            Msg::Stats { workers, busy, chunks, units } => p.stats = (workers, busy, chunks, units),
            Msg::Ping { t } => self.out.push(Act::Send(link, Msg::Pong { t })),
            Msg::Pong { t } => p.rtt = (now as u32).wrapping_sub(t),
            Msg::Probe { last, back, fill } => {
                // The run is timed from its first message: the bytes after it count.
                match &mut p.probe {
                    None => p.probe = Some((now, 0)),
                    Some(run) => run.1 = run.1.saturating_add(fill.len() as u32),
                }
                if last {
                    let (t0, bytes) = p.probe.take().unwrap_or_default();
                    let ms = now.saturating_sub(t0).max(1) as u32;
                    p.down = rate(bytes, ms);
                    self.out.push(Act::Send(link, Msg::Heard { bytes, ms }));
                    if back > 0 {
                        self.run(link, back.min(PROBE_BYTES), 0);
                    }
                }
            }
            Msg::Heard { bytes, ms } => p.up = rate(bytes, ms),
            Msg::Job { job, name } => {
                let (leased, program, started, last) = (VecDeque::new(), name.clone(), now, now);
                let (asked, next, done, used, ended) = (false, 0, 0, 0, None);
                let owner = link;
                let h = Help {
                    owner,
                    job,
                    name,
                    leased,
                    asked,
                    next,
                    done,
                    used,
                    started,
                    last,
                    ended,
                };
                self.help = Some(h);
                self.want_workers(&program);
                self.ask(now);
            }
            Msg::Want { job, n } => {
                let mut chunks = Vec::new();
                if let Some(j) = self.job.as_mut().filter(|j| j.id == job && j.ended.is_none()) {
                    // As many as asked for while the message stays under its cap, a chunk's
                    // bytes and 8 each; the first always goes (a job's chunks each fit).
                    let mut bytes = 0;
                    while chunks.len() < usize::from(n) {
                        let Some(&i) = j.queue.front() else { break };
                        let len = j.inputs[i as usize].len() + 8;
                        if !chunks.is_empty() && bytes + len > crate::msg::MAX - 1024 {
                            break;
                        }
                        j.queue.pop_front();
                        j.state[i as usize] = State::Out(link, now);
                        bytes += len;
                        chunks.push((i, j.inputs[i as usize].clone()));
                    }
                }
                self.out.push(Act::Send(link, Msg::Give { job, chunks }));
            }
            Msg::Give { job, chunks } => {
                if let Some(h) = self.help.as_mut().filter(|h| h.owner == link && h.job == job) {
                    h.next = if chunks.is_empty() { now + DRY } else { now };
                    h.asked = false;
                    h.leased.extend(chunks);
                    self.assign(now);
                }
            }
            Msg::Done { job, index, out } => self.answered(link, job, index, out, now),
            Msg::End { job, used } => {
                if let Some(h) = self.help.as_mut().filter(|h| h.owner == link && h.job == job) {
                    h.used = used;
                }
                self.end_help(|h| h.owner == link && h.job == job, now);
            }
        }
    }

    /// Bytes sent to and heard from link `link`.
    pub fn count(&mut self, link: u32, tx: usize, rx: usize) {
        if let Some(p) = self.peers.iter_mut().find(|p| p.link == link) {
            (p.tx, p.rx) = (p.tx + tx as u64, p.rx + rx as u64);
        }
    }

    /// When [`Pool::tick`] is next due: never with no link and no worker.
    pub fn due(&self) -> Option<u64> {
        (!self.peers.is_empty() || !self.workers.is_empty()).then_some(self.next_tick)
    }

    /// Each second: stats to the links, a ping every other, an ask, and idle workers' end.
    pub fn tick(&mut self, now: u64) {
        if now < self.next_tick {
            return;
        }
        self.next_tick = now + TICK;
        self.quiet(now);
        if let Some(t) = self.test.as_mut() {
            t.tick(now, &mut self.out);
            self.tested();
        }
        let silent: Vec<u32> =
            self.peers.iter().filter(|p| now >= p.last + SILENT).map(|p| p.link).collect();
        for link in silent {
            self.out.push(Act::Unlink(link));
            self.unlinked(link, now);
        }
        let busy = self.workers.iter().filter(|w| w.task.is_some()).count() as u16;
        let workers = self.workers.len() as u16;
        let stats = Msg::Stats { workers, busy, chunks: self.chunks, units: self.units };
        self.send_all(&stats);
        if now >= self.next_ping {
            // Hello again with each ping: a tab whose channel opened late lost the first.
            self.next_ping = now + PING;
            self.send_all(&self.hello());
            self.send_all(&Msg::Ping { t: now as u32 });
        }
        self.ask(now);
        let working = busy > 0
            || self.help.as_ref().is_some_and(|h| h.ended.is_none())
            || self.job.as_ref().is_some_and(|j| j.ended.is_none());
        if working {
            self.idle = now;
        } else if !self.workers.is_empty() && now >= self.idle + IDLE {
            self.out.push(Act::Stop(self.workers.drain(..).map(|w| w.pid).collect()));
            self.program.clear();
        }
    }

    /// The pool as Activity shows it, at `now`, pairing saying `pairing` with `code`.
    pub fn snap(&self, now: u64, pairing: &str, code: &str) -> Snap {
        let busy = self.workers.iter().filter(|w| w.task.is_some()).count() as u16;
        let i = &self.me;
        let mut devices = vec![pool::Device {
            name: i.name.clone(),
            kind: i.kind.clone(),
            cores: i.cores,
            ram_mb: i.ram_mb,
            quota_mb: i.quota_mb,
            gpu: i.gpu,
            up_ms: now as u32,
            known: true,
            workers: self.workers.len() as u16,
            busy,
            chunks: self.chunks,
            units: self.units,
            rtt: 0,
            model: i.model.clone(),
            tok: i.tok,
            ctx: i.ctx,
            cpu1: i.measured.cpu1,
            cpun: i.measured.cpun,
            mem: i.measured.mem,
            tested: i.measured.tested,
            ..pool::Device::default()
        }];
        for p in &self.peers {
            let i = &p.info;
            devices.push(pool::Device {
                name: i.name.clone(),
                kind: i.kind.clone(),
                cores: i.cores,
                ram_mb: i.ram_mb,
                quota_mb: i.quota_mb,
                gpu: i.gpu,
                up_ms: now.saturating_sub(p.since) as u32,
                known: p.known,
                workers: p.stats.0,
                busy: p.stats.1,
                chunks: p.stats.2,
                units: p.stats.3,
                rtt: p.rtt,
                tx: p.tx,
                rx: p.rx,
                up: p.up,
                down: p.down,
                model: i.model.clone(),
                tok: i.tok,
                ctx: i.ctx,
                cpu1: i.measured.cpu1,
                cpun: i.measured.cpun,
                mem: i.measured.mem,
                tested: i.measured.tested,
            });
        }
        // A job helped now, else this tab's own, else the last helped (its record).
        let helping = self.help.as_ref().is_some_and(|h| h.ended.is_none());
        let job = match (self.job.as_ref().filter(|_| !helping), &self.help) {
            (Some(j), _) => {
                let mut per = vec![0; devices.len()];
                j.per.iter().for_each(|&(l, n)| {
                    let k = usize::from(self.node(l));
                    if l == 0 || k > 0 {
                        per[k] += n;
                    }
                });
                Some(pool::Job {
                    name: j.name.clone(),
                    mine: true,
                    total: j.state.len() as u32,
                    done: j.done,
                    queued: j.queue.len() as u32,
                    steals: j.steals,
                    requeued: j.requeued,
                    checked: j.checked,
                    mismatched: j.mismatched,
                    ms: j.ended.unwrap_or(now).saturating_sub(j.started) as u32,
                    used: 0,
                    busy: 0,
                    per,
                })
            }
            (_, Some(h)) => Some(pool::Job {
                name: h.name.clone(),
                done: h.done,
                queued: h.leased.len() as u32,
                ms: h.ended.unwrap_or(now).saturating_sub(h.started) as u32,
                used: h.used,
                busy: h.last.saturating_sub(h.started) as u32,
                per: vec![h.done],
                ..pool::Job::default()
            }),
            (None, None) => None,
        };
        let (pairing, code) = (pairing.into(), code.into());
        let (serve, serving) = (self.shared.url.clone(), self.shared.note.clone());
        let answer = self.asking.as_ref().map(|a| a.answer.clone());
        let testing = self.test.as_ref().map_or(String::new(), |t| t.note.clone());
        Snap { at: now as u32, pairing, code, devices, job, serve, serving, answer, testing }
    }
}

/// An answer's hash: the word after its fuel.
fn hash(out: &str) -> &str {
    let rest = num(out).1;
    rest.split(' ').next().unwrap_or("")
}

/// Bytes a second, from bytes in ms.
fn rate(bytes: u32, ms: u32) -> u32 {
    (u64::from(bytes) * 1000 / u64::from(ms.max(1))).min(u64::from(u32::MAX)) as u32
}
