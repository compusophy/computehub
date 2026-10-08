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
//! is `known`.

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Host,
    Poll,
    Join,
    Answer,
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
/// while pairing, posts in flight, the last post and link ids, the process watching, the
/// snapshot last sent and to whom, when the next may go and when a wake was last asked for, the
/// clock (page ms), and the frames for the desktop.
#[derive(Debug, Default)]
pub struct Hub {
    pub pool: Pool,
    pub note: String,
    pairing: Option<Pairing>,
    pins: Vec<String>,
    heard_fp: Vec<(u32, String)>,
    posts: Vec<(u32, Op)>,
    last_post: u32,
    last_link: u32,
    watcher: u32,
    told: (Vec<u8>, Vec<u32>),
    next_snap: u64,
    woken: Option<u64>,
    pub now: u64,
    pub out: Vec<u8>,
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

    fn post(&mut self, op: Op, body: &str) {
        self.last_post += 1;
        self.posts.push((self.last_post, op));
        self.send(to_desk::POST, self.last_post, body.as_bytes());
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
                self.pool = Pool::new(Info { name, kind, cores, ram_mb, quota_mb: 0, gpu });
                self.pins = lines.map(String::from).collect();
            }
            // Pair (its code), Measure, or Job (its program, then its chunks, a line each).
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
                    _ => return,
                };
                self.ask(f.a, r);
            }
            to_pool::SIGNAL => {
                let Some(p) = self.pairing.as_ref().filter(|p| p.link == f.a) else { return };
                let body = match p.host {
                    true => (Op::Host, ["host\n", &text()].concat()),
                    false => (Op::Answer, ["answer\n", &p.code, "\n", &text()].concat()),
                };
                self.post(body.0, &body.1);
            }
            to_pool::LINKED => self.linked(f.a),
            to_pool::DATA => {
                // A message on the pairing link before its open was heard: it is open.
                let pairing = self.pairing.as_ref().is_some_and(|p| p.link == f.a);
                if pairing && self.pool.peers.iter().all(|p| p.link != f.a) {
                    self.linked(f.a);
                }
                self.pool.count(f.a, 0, f.data.len());
                if let Some(m) = Msg::decode(&f.data) {
                    self.pool.heard(f.a, m, now);
                }
            }
            to_pool::UNLINKED => {
                self.pool.unlinked(f.a, now);
                if self.pairing.as_ref().is_some_and(|p| p.link == f.a) {
                    self.stop("The link failed: try again, both devices on one network");
                }
            }
            to_pool::HTTP => {
                let Some(i) = self.posts.iter().position(|p| p.0 == f.a) else { return };
                let op = self.posts.remove(i).1;
                self.answered(op, f.b, &text());
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
            let pins = self.pins.join("\n");
            self.send(to_desk::PINS, 0, pins.as_bytes());
        }
        if self.pairing.as_ref().is_some_and(|p| p.link == link) {
            (self.pairing, self.note) = (None, String::new());
        }
        self.pool.linked(link, known, self.now);
    }

    /// Process `pid` asked for `r`: Pair and Measure (from Activity's window alone), Job.
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
            Request::Job { name, chunks } => {
                let word = name.bytes().all(|b| b.is_ascii_lowercase());
                let line =
                    |c: &String| c.len() <= LINE && c.bytes().all(|b| (0x20..0x7f).contains(&b));
                let ok = !name.is_empty() && name.len() <= 32 && word && chunks.len() <= CHUNKS;
                if ok && chunks.iter().all(line) {
                    self.pool.start(pid, &name, chunks, now);
                }
            }
            _ => {}
        }
    }

    fn answered(&mut self, op: Op, status: u32, body: &str) {
        let now = self.now;
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
        self.pool.tick(now);
        self.acts();
        let mut to: Vec<u32> = Some(self.watcher).filter(|w| *w != 0).into_iter().collect();
        to.extend(self.pool.job.as_ref().map(|j| j.window));
        let code = self.pairing.as_ref().filter(|p| p.host).map_or("", |p| p.code.as_str());
        let bytes = self.pool.snap(now, &self.note, code).encode();
        // `at` (bytes 1 to 4) changes every time: the rest says whether anything did. A change
        // goes at most once a second, unless it is to someone new.
        let (fresh, mut later) = (to != self.told.1, None);
        if (fresh || bytes.get(5..) != self.told.0.get(5..)) && !to.is_empty() {
            if fresh || now >= self.next_snap {
                let ev = Event::Pool { data: bytes.clone() }.encode();
                to.iter().for_each(|&pid| self.send(to_desk::TELL, pid, &ev));
                (self.told, self.next_snap) = ((bytes, to), now + 1000);
            } else {
                later = Some(self.next_snap);
            }
        }
        let due = [self.pool.due(), self.pairing.as_ref().map(|p| p.at.unwrap_or(p.until)), later];
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
                    Act::Unlink(link) => self.send(to_desk::UNLINK, link, &[]),
                    Act::Done { window, index, node, out } => {
                        let ev = Event::Done { index, node, out }.encode();
                        self.send(to_desk::TELL, window, &ev);
                    }
                }
            }
            self.pool.assign(self.now);
        }
    }
}
