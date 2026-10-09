//! The pool program's dealings with the desktop: [`Frame`]s in (what the page heard, asks,
//! workers' output, the clock) and out (links to make, posts, workers, events to tell).
//!
//! Pairing, the tab that shows a code: it makes an offer (LINK), posts it (`host`) and gets the
//! code, then asks for the answer (`poll`) every [`POLL`] ms until the other tab gives one or
//! [`TTL`] passes. The tab that enters the code gets the offer (`join`, again on a miss, [`TRIES`]
//! times: the server keeps codes in its own memory, which another instance lacks), answers it and
//! posts the answer (`answer`). Each body is lines of text: the op, then the code and the
//! description as they apply. Once the channel opens the link joins the pool, its DTLS
//! fingerprint (from the other tab's description) pinned for this profile: a device seen before
//! is `known`, and the two make a bond: linked for good, they meet again whenever both are open
//! ([`bond`]). The model this tab shares, its server's URL, is kept with the pins, so sharing
//! starts again with the pool.

pub mod bond;
#[cfg(test)]
mod tests;

use uiwire::relay::{Frame, to_desk, to_pool};
use uiwire::{Event, Request};

use crate::msg::Msg;
use crate::pool::{Act, Info, Pool};

/// ms between polls for an answer, how long a code lasts, how often a join tries, how long a
/// link may take to open once answered.
pub const POLL: u64 = 1500;
pub const TTL: u64 = 180_000;
pub const TRIES: u32 = 8;
pub const OPEN: u64 = 30_000;
/// A chunk's longest line, and a job's most chunks.
pub const LINE: usize = 4096;
pub const CHUNKS: usize = 1 << 16;
/// The same-origin endpoint pairing posts to.
pub const SIGNAL: &str = "/api/signal";
/// The line kept with the pins that says the model this tab shares, and the one that says what
/// testing this device measured (`tested <cpu1> <cpun> <mem> <when> <floor>`, the last 1 when its
/// memory is "at least"; absent in what older tabs kept).
const SERVE: &str = "serve ";
const TESTED: &str = "tested ";
/// The line that keeps this profile's own key, once a description has said it.
const KEY: &str = "key ";

#[derive(Clone, Debug, PartialEq, Eq)]
enum Op {
    Host,
    Poll,
    Join,
    Answer,
    /// A job's chunks to fetch for process `.0`: its program and the lines a chunk holds.
    Fetch(u32, String, usize),
    /// A chat to the model this tab shares: its body goes to the pool as it comes.
    Model,
    /// What the model's server says of itself: gathered, then to the pool.
    Props,
    /// A bond meeting its device again ([`bond`]): an offer kept at the mailbox for link `.0`,
    /// the answer asked for, the offer asked for (the bond's fingerprint), the answer given.
    Meet(u32),
    Wait(u32),
    Find(String),
    Reply(u32),
}

/// Pairing under way: the link, the code (as typed or given), whether this tab shows it, the
/// next try's time, the tries so far, and when it gives up.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Pairing {
    link: u32,
    code: String,
    host: bool,
    at: Option<u64>,
    tries: u32,
    until: u64,
}

/// The hub: the pool, what pairing says and is doing, the pinned fingerprints and those heard
/// for each link, the devices linked for good, this tab's own fingerprint, the nonces sent on
/// links just paired (link, nonce, whether this tab showed the code), links that said goodbye and
/// when they close; posts in flight (each its id, what it is and its body so far), the last post
/// and link ids, the process watching, the snapshot last sent and to whom, when the next may go
/// and when a wake was last asked for, the clock (page ms), what testing measured as last kept,
/// and the frames for the desktop.
#[derive(Debug, Default)]
pub struct Hub {
    pub pool: Pool,
    pub note: String,
    pairing: Option<Pairing>,
    pub(crate) pins: Vec<String>,
    heard_fp: Vec<(u32, String)>,
    pub bonds: Vec<bond::Bond>,
    own_fp: String,
    nonces: Vec<(u32, String, bool)>,
    bye: Vec<(u32, u64)>,
    posts: Vec<(u32, Op, Vec<u8>)>,
    last_post: u32,
    last_link: u32,
    watcher: u32,
    told: (Vec<u8>, Vec<u32>),
    next_snap: u64,
    woken: Option<u64>,
    pub now: u64,
    kept_test: crate::test::Measured,
    pub out: Vec<u8>,
}

/// The Unix time now (WASI's realtime clock), seconds; 0 if none.
fn wall() -> u32 {
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    since.map_or(0, |d| d.as_secs().min(u64::from(u32::MAX)) as u32)
}

/// A browser's agent as a device's name and kind: `Windows · Chrome`, `computer`.
pub fn named(agent: &str, touch: bool) -> (String, String) {
    let os = [("Android", "Android"), ("iPhone", "iPhone"), ("iPad", "iPad"), ("CrOS", "ChromeOS")];
    let os = os.iter().chain(&[("Windows", "Windows"), ("Mac", "Mac"), ("Linux", "Linux")]);
    let os = os.clone().find(|(k, _)| agent.contains(k)).map_or("", |o| o.1);
    let web =
        [("Edg/", "Edge"), ("Firefox", "Firefox"), ("Chrome", "Chrome"), ("Safari", "Safari")];
    let web = web.iter().find(|(k, _)| agent.contains(k)).map_or("Browser", |w| w.1);
    let kind = match os {
        "Android" | "iPhone" if touch => "phone",
        "iPad" => "tablet",
        _ => "computer",
    };
    ([os, if os.is_empty() { "" } else { " \u{b7} " }, web].concat(), kind.into())
}

/// The DTLS fingerprint a description gives: `sha-256 AB:CD:...`.
pub fn fingerprint(sdp: &str) -> String {
    let line = sdp.lines().find_map(|l| l.strip_prefix("a=fingerprint:"));
    line.unwrap_or("").trim().into()
}

/// A code as typed: its letters and digits, upper case; empty if it has none or over 8.
pub fn code(typed: &str) -> String {
    let c: String = typed.chars().filter(char::is_ascii_alphanumeric).collect();
    if c.len() > 8 { String::new() } else { c.to_ascii_uppercase() }
}

impl Hub {
    fn send(&mut self, op: u8, a: u32, data: &[u8]) {
        Frame { op, a, data: data.into(), ..Frame::default() }.put(&mut self.out);
    }

    /// Posts `body` to `url` as `op`.
    fn post_to(&mut self, url: &str, op: Op, body: &str) {
        self.last_post += 1;
        self.posts.push((self.last_post, op, Vec::new()));
        self.send(to_desk::POST, self.last_post, [url, "\n", body].concat().as_bytes());
    }

    fn post(&mut self, op: Op, body: &str) {
        self.post_to(SIGNAL, op, body);
    }

    /// Keeps the pins, the bonds and the model shared for this profile.
    fn keep(&mut self) {
        let mut kept = self.pins.join("\n");
        if !self.own_fp.is_empty() {
            kept = [&kept, "\n", KEY, &self.own_fp].concat();
        }
        for b in &self.bonds {
            kept = [&kept, "\n", &b.line()].concat();
        }
        if !self.pool.shared.url.is_empty() {
            kept = [&kept, "\n", SERVE, &self.pool.shared.url].concat();
        }
        let m = self.pool.me.measured;
        if m.tested != 0 {
            let n = [m.cpu1, m.cpun, m.mem, m.tested, m.floor.into()];
            let n = n.map(|n| n.to_string()).join(" ");
            kept = [&kept, "\n", TESTED, &n].concat();
        }
        self.kept_test = m;
        self.send(to_desk::PINS, 0, kept.trim_start_matches('\n').as_bytes());
    }

    /// Pairing stops: with `why` said, and its link closed unless it opened.
    fn stop(&mut self, why: &str) {
        if let Some(p) = self.pairing.take() {
            if !self.pool.peers.iter().any(|q| q.link == p.link) {
                self.send(to_desk::UNLINK, p.link, &[]);
            }
        }
        self.note = why.into();
    }

    /// One frame from the desktop.
    pub fn frame(&mut self, f: Frame) {
        self.now += u64::from(f.now.wrapping_sub(self.now as u32));
        self.pool.clock = wall();
        let now = self.now;
        let text = || String::from_utf8_lossy(&f.data).into_owned();
        match f.op {
            to_pool::INFO => {
                let text = text();
                let mut lines = text.lines();
                let mut n =
                    lines.next().unwrap_or("").split(' ').map(|w| w.parse().unwrap_or(0u32));
                let mut n = || n.next().unwrap_or(0);
                let (cores, ram_mb, gpu, touch) = (n(), n(), n() == 1, n() == 1);
                let (name, kind) = named(lines.next().unwrap_or(""), touch);
                let cores = cores.min(1024) as u16;
                let info = Info { name, kind, cores, ram_mb, gpu, ..Info::default() };
                self.pool = Pool::new(info);
                let ours =
                    |l: &&str| [SERVE, TESTED, KEY, bond::BOND].iter().any(|p| l.starts_with(p));
                let (kept, pins): (Vec<&str>, _) = lines.partition(ours);
                self.pins = pins.into_iter().map(String::from).collect();
                self.bonds = kept.iter().filter_map(|l| bond::Bond::parse(l)).collect();
                let key = kept.iter().find_map(|l| l.strip_prefix(KEY));
                self.own_fp = key.unwrap_or("").into();
                let tested = kept.iter().rev().find_map(|l| l.strip_prefix(TESTED));
                let n: Vec<u32> = tested
                    .map_or(Vec::new(), |t| t.split(' ').filter_map(|w| w.parse().ok()).collect());
                if let [cpu1, cpun, mem, tested, ..] = n[..] {
                    let floor = n.get(4) == Some(&1);
                    self.pool.me.measured =
                        crate::test::Measured { cpu1, cpun, mem, tested, floor };
                    self.kept_test = self.pool.me.measured;
                }
                let served: Vec<&str> = kept.into_iter().filter(|l| l.starts_with(SERVE)).collect();
                if let Some(url) = served.last().and_then(|l| l.strip_prefix(SERVE)) {
                    self.pool.serve(url);
                }
            }
            // Pair (its code), Measure, Job (its program, then its chunks, a line each), Ask (its
            // question) or Serve (its URL).
            to_pool::ASK => {
                let text = text();
                let mut lines = text.split('\n');
                let r = match f.b {
                    1 => Request::Pair { code: text.clone() },
                    2 => Request::Measure,
                    3 => {
                        let name = lines.next().unwrap_or("").into();
                        Request::Job { name, chunks: lines.map(String::from).collect() }
                    }
                    4 => Request::Ask { text },
                    5 => Request::Serve { url: text },
                    6 => Request::Test,
                    7 => Request::Link { what: text },
                    8 => Request::Memory,
                    _ => return,
                };
                self.ask(f.a, r);
            }
            to_pool::SIGNAL => {
                let sdp = text();
                // This tab's own key: kept the first time it is heard (or if it changed).
                let fp = fingerprint(&sdp);
                if !fp.is_empty() && fp != self.own_fp {
                    self.own_fp = fp;
                    self.keep();
                }
                if self.meet_signal(f.a, &sdp) {
                    return;
                }
                let Some(p) = self.pairing.as_ref().filter(|p| p.link == f.a) else { return };
                let body = match p.host {
                    true => (Op::Host, ["host\n", &text()].concat()),
                    false => (Op::Answer, ["answer\n", &p.code, "\n", &text()].concat()),
                };
                self.post(body.0, &body.1);
            }
            to_pool::LINKED => self.linked(f.a),
            to_pool::DATA => {
                // A message on a link being made before its open was heard: it is open.
                let pairing = self.pairing.as_ref().is_some_and(|p| p.link == f.a);
                let meeting = self.bonds.iter().any(|b| b.link == f.a);
                if (pairing || meeting) && self.pool.peers.iter().all(|p| p.link != f.a) {
                    self.linked(f.a);
                }
                self.pool.count(f.a, 0, f.data.len());
                if let Some(m) = Msg::decode(&f.data) {
                    if !self.bond_heard(f.a, &m) {
                        self.pool.heard(f.a, m, now);
                    }
                }
            }
            to_pool::UNLINKED => {
                self.pool.unlinked(f.a, now);
                self.bond_lost(f.a);
                if self.pairing.as_ref().is_some_and(|p| p.link == f.a) {
                    self.stop("The link failed: try again, both devices on one network");
                }
            }
            to_pool::PART => {
                let Some(p) = self.posts.iter_mut().find(|p| p.0 == f.a) else { return };
                match p.1 {
                    Op::Model => self.pool.part(&f.data),
                    _ => p.2.extend_from_slice(&f.data),
                }
            }
            to_pool::HTTP => {
                let Some(i) = self.posts.iter().position(|p| p.0 == f.a) else { return };
                let (_, op, mut body) = self.posts.remove(i);
                if op == Op::Model {
                    return self.pool.posted(f.b);
                }
                body.extend_from_slice(&f.data);
                self.answered(op, f.b, &String::from_utf8_lossy(&body));
            }
            to_pool::OUT => self.pool.output(f.a, &f.data, now),
            to_pool::GONE => self.pool.ended(f.a, now),
            to_pool::SPAWNED => self.pool.spawned(Some(f.a).filter(|p| *p != 0)),
            to_pool::WATCH => self.watcher = f.a,
            to_pool::QUOTA => self.pool.me.quota_mb = f.a,
            _ => {}
        }
    }

    /// Link `link` opened: the device joins the pool, its key pinned for this profile (`known`
    /// if it was before), and pairing is done. Once only.
    fn linked(&mut self, link: u32) {
        if self.pool.peers.iter().any(|p| p.link == link) {
            return;
        }
        let fp = self.heard_fp.iter().find(|h| h.0 == link).map(|h| h.1.clone());
        let fp = fp.unwrap_or_default();
        let known = !fp.is_empty() && self.pins.contains(&fp);
        if !known && !fp.is_empty() {
            self.pins.push(fp);
            self.keep();
        }
        let paired = self.pairing.as_ref().filter(|p| p.link == link).map(|p| p.host);
        if paired.is_some() {
            (self.pairing, self.note) = (None, String::new());
        }
        self.pool.linked(link, known, self.now);
        self.bond_linked(link, paired);
    }

    /// Process `pid` asked for `r`: Pair, Measure, Ask, Serve and Test (from Activity's window
    /// alone), Job.
    pub fn ask(&mut self, pid: u32, r: Request) {
        let now = self.now;
        match r {
            Request::Pair { code: typed } => {
                self.stop("");
                self.last_link += 1;
                let (link, until, code) = (self.last_link, now + TTL, code(&typed));
                let host = code.is_empty();
                if host {
                    self.send(to_desk::LINK, link, &[]);
                    self.note = "Making a code".into();
                } else {
                    self.post(Op::Join, &["join\n", &code].concat());
                    self.note = "Looking for that code".into();
                }
                self.pairing = Some(Pairing { link, code, host, until, ..Pairing::default() });
            }
            Request::Measure => self.pool.measure(),
            Request::Ask { text } => self.pool.question(&text),
            Request::Serve { url } => {
                self.pool.serve(&url);
                self.keep();
            }
            Request::Test => self.pool.test(now),
            Request::Link { what } => self.link_ask(&what),
            Request::Memory => self.pool.test_memory(now),
            // A job whose one chunk is `@<url> <per>`: its chunks are the lines of that
            // same-origin file, `per` a chunk (tab-joined), fetched first.
            Request::Job { name, chunks } if chunks.len() == 1 && chunks[0].starts_with('@') => {
                let at = chunks[0].get(1..).unwrap_or("");
                let (url, per) = at.split_once(' ').unwrap_or((at, "1"));
                let per = per.parse().unwrap_or(1usize).clamp(1, 64);
                let url = url.to_string();
                self.last_post += 1;
                self.posts.push((self.last_post, Op::Fetch(pid, name, per), Vec::new()));
                self.send(to_desk::FETCH, self.last_post, url.as_bytes());
                self.note = ["Fetching ", &url].concat();
            }
            Request::Job { name, chunks } => {
                let word = name.bytes().all(|b| b.is_ascii_lowercase());
                let line =
                    |c: &String| c.len() <= LINE && c.bytes().all(|b| (0x20..0x7f).contains(&b));
                let ok = !name.is_empty() && name.len() <= 32 && word && chunks.len() <= CHUNKS;
                // A fetched file's chunks may be long (tasks of a suite): each must fit a message
                // with room to spare, and never hold a newline.
                let fits = |c: &String| c.len() <= crate::msg::MAX - 1024 && !c.contains('\n');
                if ok && (chunks.iter().all(line) || chunks.iter().all(fits)) {
                    self.pool.start(pid, &name, chunks, now);
                } else {
                    let why = " was refused: a chunk too long, or not text";
                    self.note = ["The job ", &name, why].concat();
                }
            }
            _ => {}
        }
    }

    fn answered(&mut self, op: Op, status: u32, body: &str) {
        let now = self.now;
        if matches!(op, Op::Meet(_) | Op::Wait(_) | Op::Find(_) | Op::Reply(_)) {
            return self.met(op, status, body);
        }
        if op == Op::Props {
            return self.pool.props(status, body);
        }
        if let Op::Fetch(pid, name, per) = op {
            if status != 200 {
                return self.note = ["The job's file could not be fetched: ", body].concat();
            }
            let lines: Vec<&str> = body.lines().filter(|l| !l.trim().is_empty()).collect();
            let chunks = lines.chunks(per).map(|c| c.join("\t")).collect();
            self.note.clear();
            return self.ask(pid, Request::Job { name, chunks });
        }
        let Some(p) = self.pairing.as_mut() else { return };
        let (ok, link) = (status == 200, p.link);
        match op {
            Op::Host if ok => {
                (p.code, p.at) = (code(body), Some(now + POLL));
                self.note = "Enter this code on the other device".into();
            }
            Op::Poll if ok && !body.is_empty() => {
                (p.at, p.until) = (None, now + OPEN);
                self.heard_fp.push((link, fingerprint(body)));
                self.send(to_desk::ACCEPT, link, body.as_bytes());
                self.note = "Linking".into();
            }
            Op::Poll => p.at = Some(now + POLL),
            Op::Join if ok => {
                p.until = now + OPEN;
                self.heard_fp.push((link, fingerprint(body)));
                self.send(to_desk::LINK, link, body.as_bytes());
                self.note = "Linking".into();
            }
            Op::Join if status == 404 && p.tries + 1 < TRIES => {
                (p.tries, p.at) = (p.tries + 1, Some(now + POLL));
            }
            Op::Join => self.stop("No device shows that code"),
            Op::Answer if ok => {}
            _ => self.stop("Pairing failed: the server did not answer"),
        }
    }

    /// After a batch of frames: pairing's next try or its end, the pool's tick, what the pool
    /// asks, the snapshot for the watcher and the job's window, and the next wake.
    pub fn pump(&mut self) {
        let now = self.now;
        if let Some(p) = self.pairing.as_mut() {
            if now >= p.until {
                let why = if p.host { "The code expired" } else { "Could not link the devices" };
                self.stop(why);
            } else if p.at.is_some_and(|t| now >= t) {
                p.at = None;
                let (op, verb) = if p.host { (Op::Poll, "poll\n") } else { (Op::Join, "join\n") };
                let body = [verb, &p.code].concat();
                self.post(op, &body);
            }
        }
        self.meet(now);
        self.goodbyes(now);
        self.pool.tick(now);
        self.acts();
        // What a test measured (the CPU's half, then the memory too) is kept for this profile.
        if self.pool.me.measured != self.kept_test {
            self.keep();
        }
        let mut to: Vec<u32> = Some(self.watcher).filter(|w| *w != 0).into_iter().collect();
        to.extend(self.pool.job.as_ref().map(|j| j.window).filter(|w| !to.contains(w)));
        let code = self.pairing.as_ref().filter(|p| p.host).map_or("", |p| p.code.as_str());
        let mut snap = self.pool.snap(now, &self.note, code);
        snap.bonds = self.bonds.iter().map(bond::Bond::shown).collect();
        snap.key.clone_from(&self.own_fp);
        let bytes = snap.encode();
        // `at` (bytes 1 to 4) changes every time: the rest says whether anything did. A change
        // goes at most once a second (four times while a model writes an answer here), unless it
        // is to someone new.
        let (fresh, mut later) = (to != self.told.1, None);
        if (fresh || bytes.get(5..) != self.told.0.get(5..)) && !to.is_empty() {
            if fresh || now >= self.next_snap {
                let ev = Event::Pool { data: bytes.clone() }.encode();
                to.iter().for_each(|&pid| self.send(to_desk::TELL, pid, &ev));
                let gap = if self.pool.answering() { 250 } else { 1000 };
                (self.told, self.next_snap) = ((bytes, to), now + gap);
            } else {
                later = Some(self.next_snap);
            }
        }
        let pairing = self.pairing.as_ref().map(|p| p.at.unwrap_or(p.until));
        let bye = self.bye.iter().map(|b| b.1).min();
        let due = [self.pool.due(), pairing, later, self.meet_due(), bye];
        if let Some(t) = due.into_iter().flatten().min().filter(|&t| self.woken != Some(t)) {
            self.woken = Some(t);
            self.send(to_desk::WAKE, t.saturating_sub(now).min(60_000) as u32, &[]);
        }
    }

    fn acts(&mut self) {
        while !self.pool.out.is_empty() {
            for a in std::mem::take(&mut self.pool.out) {
                match a {
                    Act::Send(link, m) => {
                        let data = m.encode();
                        self.pool.count(link, data.len(), 0);
                        self.send(to_desk::SEND, link, &data);
                    }
                    Act::Feed(pid, mut line) => {
                        line.push('\n');
                        self.send(to_desk::FEED, pid, line.as_bytes());
                    }
                    Act::Spawn(name, n) => self.send(to_desk::SPAWN, n.into(), name.as_bytes()),
                    Act::Stop(pids) => {
                        pids.into_iter().for_each(|p| self.send(to_desk::KILL, p, &[]))
                    }
                    // A link gone silent: the pool dropped it, and its bond tries again.
                    Act::Unlink(link) => {
                        self.send(to_desk::UNLINK, link, &[]);
                        self.bond_lost(link);
                    }
                    Act::Done { window, index, node, out } => {
                        let ev = Event::Done { index, node, out }.encode();
                        self.send(to_desk::TELL, window, &ev);
                    }
                    Act::Post { url, body } => self.post_to(&url, Op::Model, &body),
                    Act::Get(url) => self.post_to(&url, Op::Props, ""),
                }
            }
            self.pool.assign(self.now);
        }
    }
}
