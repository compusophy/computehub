//! compusophyOS's mesh, the desktop's side: tabs linked tab to tab share one job on every core
//! of every device. What only the page can do lives here, kept small (it ships with the boot):
//! the links (WebRTC, [`platform::Ctl::link`]), the posts it asks for (pairing's, a shared
//! model's to its local server), the workers (kernel processes on consoles of their own, owned by
//! no window ([`OWNER`]) and given no files) and the pinned keys in storage. Everything else, the queue, work stealing, receipts, pairing and what
//! tabs say to each other, is the pool program (`/bin/pool`), started the first time something
//! needs it and told everything as [`uiwire::relay`] frames on its console, which it answers the
//! same way.

#![forbid(unsafe_code)]

use kernel::{Kernel, Spawn, wire};
use platform::{Ctl, Event as Heard};
use uiwire::Request;
use uiwire::relay::{Frame, to_desk, to_pool};
use vfs::Vfs;

/// The kernel owner of the pool and its workers: no window.
pub const OWNER: u32 = u32::MAX - 7;
/// Stream ids from here up are the pool's posts.
pub const FIRST: u32 = 0x6d65_0000;
/// How a device linked for good starts its line in the kept pins (the pool's `hub`).
pub const BOND: &str = "bond ";

/// The relay: the pool's pid and whether it reads frames yet, frames waiting for it, its output
/// short of a frame, posts and fetches in flight (their stream ids), the workers, the process watching,
/// when the pool wants waking, where this profile keeps the mesh, and whether its links were looked
/// for since it loaded.
#[derive(Default)]
pub struct Relay {
    pid: Option<u32>,
    ready: bool,
    queue: Vec<u8>,
    buf: Vec<u8>,
    posts: Vec<u32>,
    workers: Vec<u32>,
    watcher: u32,
    wake: Option<f64>,
    prefix: String,
    looked: bool,
}

impl Relay {
    /// This profile's mesh, kept under `prefix`.
    pub fn load(&mut self, prefix: &str) {
        *self = Relay { prefix: prefix.into(), ..Relay::default() };
    }

    fn tell(&mut self, op: u8, a: u32, b: u32, data: Vec<u8>, now: f64) {
        Frame { op, a, b, now: now as u64 as u32, data }.put(&mut self.queue);
    }

    /// Whether stream `id` is a pool's post.
    pub fn streams(&self, id: u32) -> bool {
        self.posts.contains(&id)
    }

    /// Process `pid` asked for `r`: Pair (from Activity's window alone) as its code, Measure,
    /// Job as its program's name, then its chunks, a line each; Ask its question, Serve its URL;
    /// Test; Link what it asks.
    pub fn ask(&mut self, pid: u32, r: &Request, now: f64) {
        let (kind, mut data) = match r {
            Request::Pair { code } => (1, code.clone()),
            Request::Measure => (2, String::new()),
            Request::Job { name, .. } => (3, name.clone()),
            Request::Ask { text } => (4, text.clone()),
            Request::Serve { url } => (5, url.clone()),
            Request::Test => (6, String::new()),
            Request::Link { what } => (7, what.clone()),
            _ => return,
        };
        if let Request::Job { chunks, .. } = r {
            for c in chunks {
                data.push('\n');
                data.push_str(c);
            }
        }
        self.tell(to_pool::ASK, pid, kind, data.into_bytes(), now);
    }

    /// The page's news for the mesh: links, the posts' and fetches' answers, the storage quota.
    pub fn heard(&mut self, ev: Heard, now: f64) {
        let (op, a, b, data) = match ev {
            Heard::Signal { id, sdp } => (to_pool::SIGNAL, id, 0, sdp.into_bytes()),
            Heard::Linked { id } => (to_pool::LINKED, id, 0, vec![]),
            Heard::LinkData { id, data } => (to_pool::DATA, id, 0, data),
            Heard::Unlinked { id } => (to_pool::UNLINKED, id, 0, vec![]),
            Heard::Estimated { quota_mb } => (to_pool::QUOTA, quota_mb, 0, vec![]),
            Heard::Chunk { id, data } => (to_pool::PART, id - FIRST, 0, data),
            Heard::StreamEnd { id, status, .. } => {
                self.posts.retain(|p| *p != id);
                (to_pool::HTTP, id - FIRST, status.into(), vec![])
            }
            Heard::Fetched { id, result } => {
                self.posts.retain(|p| *p != id);
                let (status, body) = result.map_or_else(|e| (0, e.into_bytes()), |b| (200, b));
                (to_pool::HTTP, id - FIRST, status, body)
            }
            _ => return,
        };
        self.tell(op, a, b, data, now);
    }

    /// Hands the pool what waits for it (starting it if need be), carries out what it asks,
    /// passes it the workers' output and ends; when to come back (ms).
    pub fn pump(
        &mut self,
        ctl: &mut Ctl,
        k: &mut Kernel,
        vfs: &Vfs,
        watcher: Option<u32>,
    ) -> Option<u32> {
        let now = ctl.monotonic_ms();
        if self.wake.is_some_and(|t| now >= t) {
            self.wake = None;
            self.tell(to_pool::TICK, 0, 0, vec![], now);
        }
        // A profile with devices linked for good starts the pool at once: they find each other
        // again whenever both are open.
        if !self.looked {
            self.looked = true;
            let kept = ctl.storage_get(&self.pins()).unwrap_or_default();
            // Line by line: `str::contains` would ship a substring searcher in the boot.
            if kept.split('\n').any(|l| l.starts_with(BOND)) {
                self.tell(to_pool::TICK, 0, 0, vec![], now);
            }
        }
        let watcher = watcher.unwrap_or(0);
        if self.pid.is_some() && watcher != self.watcher {
            self.watcher = watcher;
            self.tell(to_pool::WATCH, watcher, 0, vec![], now);
        }
        let mut i = 0;
        while let Some(&w) = self.workers.get(i) {
            let out = k.take_output(w);
            if !out.is_empty() {
                self.tell(to_pool::OUT, w, 0, out, now);
            }
            if k.reap(w).is_some() {
                self.workers.remove(i);
                self.tell(to_pool::GONE, w, 0, vec![], now);
            } else {
                i += 1;
            }
        }
        if self.pid.is_none() && !self.queue.is_empty() {
            // The device first: cores, RAM, a GPU, touch; the agent; the keys pinned.
            let (hw, dev) = (platform::hardware(), platform::device());
            let mut info = String::new();
            for n in [hw.cores.into(), hw.ram_mb, hw.gpu.into(), dev.touch.into()] {
                push_num(&mut info, n);
                info.push(' ');
            }
            info =
                [&info, "\n", &dev.agent, "\n", &ctl.storage_get(&self.pins()).unwrap_or_default()]
                    .concat();
            let queue = core::mem::take(&mut self.queue);
            self.tell(to_pool::INFO, 0, 0, info.into_bytes(), now);
            self.queue.extend(queue);
            self.pid = self.spawn(k, vfs, "pool", "");
            (self.watcher, self.ready) = (0, false);
            ctl.estimate();
        }
        let Some(pid) = self.pid else {
            self.queue.clear();
            return None;
        };
        self.buf.extend(k.take_output(pid));
        while let Some((f, used)) = Frame::take(&self.buf) {
            self.buf.drain(..used);
            self.ready = true;
            self.act(f, ctl, k, vfs, now);
        }
        if k.reap(pid).is_some() {
            // The pool ended: its workers too; the next ask starts it again.
            self.workers.drain(..).for_each(|w| k.kill(w, wire::KILLED));
            (self.pid, self.buf, self.wake) = (None, Vec::new(), None);
        } else if self.ready && !self.queue.is_empty() {
            k.input(pid, &core::mem::take(&mut self.queue));
        }
        self.wake.map(|t| (t - now).clamp(0.0, 6e4) as u32)
    }

    fn pins(&self) -> String {
        [&self.prefix, ".pins"].concat()
    }

    /// Starts `/bin/<name>` (with `arg`, if any) on a console of its own.
    fn spawn(&self, k: &mut Kernel, vfs: &Vfs, name: &str, arg: &str) -> Option<u32> {
        k.set_owner(OWNER);
        let program = kernel::program(vfs, &["/bin/", name].concat()).ok()?;
        let argv = [name, arg].into_iter().filter(|a| !a.is_empty()).map(String::from).collect();
        let (cwd, tty, stdout, roots) = ("/".into(), Some((80, 4)), wire::Stdout::Console, vec![]);
        k.spawn(Spawn { argv, program, cwd, tty, stdout, roots }).ok()
    }

    fn act(&mut self, f: Frame, ctl: &mut Ctl, k: &mut Kernel, vfs: &Vfs, now: f64) {
        let text = String::from_utf8_lossy(&f.data).into_owned();
        match f.op {
            to_desk::LINK => {
                let offer = Some(text).filter(|o| !o.is_empty());
                ctl.link(f.a, &[&self.prefix, ".cert"].concat(), offer);
            }
            to_desk::ACCEPT => ctl.accept(f.a, text),
            to_desk::SEND => ctl.link_send(f.a, f.data),
            to_desk::UNLINK => ctl.unlink(f.a),
            to_desk::POST | to_desk::FETCH => {
                let id = FIRST + (f.a & 0xffff);
                match (f.op, text.split_once('\n')) {
                    // text/plain: a simple request, which a local server needs no preflight for.
                    (to_desk::POST, Some((url, body))) => {
                        let plain = vec![("content-type", "text/plain".into())];
                        ctl.stream(id, url, plain, body.as_bytes().to_vec())
                    }
                    (to_desk::POST, None) => return,
                    _ => ctl.fetch(id, &text),
                }
                self.posts.push(id);
            }
            to_desk::SPAWN => {
                for _ in 0..f.a.min(64) {
                    // A program's name is a lowercase word: never a path.
                    let word = !text.is_empty() && text.bytes().all(|b| b.is_ascii_lowercase());
                    let pid = if word { self.spawn(k, vfs, &text, "work") } else { None };
                    self.workers.extend(pid);
                    self.tell(to_pool::SPAWNED, pid.unwrap_or(0), 0, vec![], now);
                }
            }
            to_desk::FEED if self.workers.contains(&f.a) => k.input(f.a, &f.data),
            to_desk::KILL if self.workers.contains(&f.a) => k.kill(f.a, wire::KILLED),
            to_desk::TELL => k.post_event(f.a, &f.data),
            to_desk::PINS => ctl.storage_set(&self.pins(), &text),
            to_desk::WAKE => self.wake = Some(now + f64::from(f.a)),
            _ => {}
        }
    }
}

/// `n` in decimal after `out`, without the formatting machinery.
fn push_num(out: &mut String, n: u32) {
    if n >= 10 {
        push_num(out, n / 10);
    }
    out.push(char::from(b'0' + (n % 10) as u8));
}

#[cfg(test)]
mod tests;
