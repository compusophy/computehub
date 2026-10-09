//! The free AI: no key and no provider to pick. A program's [`Request::Ai`] is POSTed to [`URL`],
//! compusophy's own server function, which forwards it to the model and streams the response body
//! back: to the program as [`Event::AiData`]s ([`CHUNK`] at most), then an [`Event::AiEnd`]. The
//! one setting, the model (one of [`MODELS`]), lives in `localStorage` ([`MODEL`]); programs hear
//! it as [`Event::Config`] (after their first Resize, again on each change), apps as [`AiStatus`],
//! and a request that names no model (a Terminal's program, which hears no Config) asks for it.
//!
//! The hub also keeps what Activity reads ([`uiwire::stat`]): the process watching (the newest
//! [`Request::Watch`]), and since the tab opened the requests, the failed (an HTTP status not
//! 2xx, or none), those that ended with no receipt (cut off, cancelled) and what the receipts
//! said: the server ends a stream with one, `: receipt in=<n> out=<n> microusd=<n>` (an SSE
//! comment, which readers skip), its tokens in and out and its cost, read from the stream's last
//! [`TAIL`] bytes (whole, wherever chunks split it).

use std::cell::RefCell;
use std::mem;
use std::rc::Rc;

use logon::own;
use platform::{Ctl, Event as Heard};
use ui::AiStatus;
use ui::kernel::{Kernel, wire::KILLED};
use uiwire::{Event, Request};

/// The same-origin endpoint every request goes to.
pub const URL: &str = "/api/ai";
/// The `localStorage` key of the model, the models on offer (the first the default, which
/// anything else stored becomes) and the largest AiData.
pub const MODEL: &str = "compusophy.ai.model";
pub const MODELS: [&str; 2] = ["zai/glm-5.3", "zai/glm-5.3-flash"];
pub const DEFAULT_MODEL: &str = MODELS[0];
pub const CHUNK: usize = 32 << 10;
/// The bytes of a stream kept for its receipt, its last line.
pub const TAIL: usize = 64;

/// The page's AI state, shared (clones are one) by the desktop and its program windows.
#[derive(Clone, Default)]
pub struct Ai(pub(crate) Rc<RefCell<Hub>>);

/// The model (an index of [`MODELS`]), the asks, requests in flight (stream id, pid, id, its last
/// bytes; two a process), the last stream id, who to tell the settings and whether to; the
/// watcher and whether it is new, and the counts from [`uiwire::stat::ASKED`] on.
#[derive(Default)]
pub(crate) struct Hub {
    model: usize,
    asks: Vec<(u32, Request)>,
    live: Vec<(u32, u32, u32, Vec<u8>)>,
    last: u32,
    told: Vec<u32>,
    retell: bool,
    pub(crate) watch: Option<u32>,
    pub(crate) fresh: bool,
    /// The mesh's asks (Pair, Measure, Job, Ask, Serve, Test), for the desktop's [`mesh::Hub`].
    pub(crate) mesh: Vec<(u32, Request)>,
    pub(crate) counts: [u32; 6],
}

impl Hub {
    /// Picks `model`, or the default if it is not on offer; tells the programs at the next pump.
    fn set(&mut self, model: &str) {
        self.model = MODELS.iter().position(|m| *m == model).unwrap_or(0);
        self.retell = true;
    }

    fn config(&self) -> Event {
        Event::Config { model: MODELS[self.model].into() }
    }

    /// Starts request `id` of `pid`, or ends it at once if it is the process's third.
    fn start(&mut self, ctl: &mut Ctl, k: &mut Kernel, (pid, id): (u32, u32), body: String) {
        if self.live.iter().filter(|l| l.1 == pid).count() >= 2 {
            return end(k, pid, id, 0, "busy");
        }
        self.last += 1;
        self.counts[0] = self.counts[0].wrapping_add(1);
        self.live.push((self.last, pid, id, Vec::new()));
        // The chosen model first, which a body's own "model" overrides (the endpoint's JSON.parse
        // keeps a key's last value): a program that heard no Config, as one a Terminal's shell
        // runs, asks for the person's choice too.
        let body = match body.strip_prefix('{') {
            Some(rest) => {
                let comma = if rest.starts_with('}') { "" } else { "," };
                ["{\"model\":\"", MODELS[self.model], "\"", comma, rest].concat()
            }
            None => body,
        };
        let json = vec![("Content-Type", "application/json".into())];
        ctl.stream(self.last, URL, json, body.into());
    }
}

impl Ai {
    /// A new desktop: the model from storage (told at the next [`Ai::pump`]), nothing in flight.
    pub fn load(&self, ctl: &Ctl) {
        let mut h = Hub::default();
        h.set(&ctl.storage_get(&own(MODEL)).unwrap_or_default());
        *self.0.borrow_mut() = h;
    }

    /// Whether stream `id` is a request in flight.
    pub fn streams(&self, id: u32) -> bool {
        self.0.borrow().live.iter().any(|l| l.0 == id)
    }

    /// What apps see.
    pub fn status(&self) -> AiStatus {
        AiStatus { model: MODELS[self.0.borrow().model].into(), ..AiStatus::default() }
    }

    /// Saves the model an app chose (one not on offer is the default).
    pub fn set_model(&self, ctl: &mut Ctl, model: &str) {
        let mut h = self.0.borrow_mut();
        h.set(model);
        ctl.storage_set(&own(MODEL), MODELS[h.model]);
    }

    /// The settings for process `pid`, which hears them again on every change.
    pub fn hello(&self, pid: u32) -> Event {
        self.0.borrow_mut().told.push(pid);
        self.0.borrow().config()
    }

    /// Process `pid` asks for `r`, for [`Ai::pump`]: Ai, AiCancel, Close (done: abort all), and
    /// from Activity's window alone, Watch (the newest watcher wins) and End.
    pub fn ask(&self, pid: u32, r: Request) {
        self.0.borrow_mut().asks.push((pid, r));
    }

    /// A stream's news ([`Heard::Chunk`], [`Heard::StreamEnd`]) for the process that asked.
    pub fn heard(&self, k: &mut Kernel, ev: Heard) {
        let h = &mut *self.0.borrow_mut();
        let (Heard::Chunk { id: sid, .. } | Heard::StreamEnd { id: sid, .. }) = ev else { return };
        let Some(i) = h.live.iter().position(|l| l.0 == sid) else { return };
        let (pid, id) = (h.live[i].1, h.live[i].2);
        if let Heard::StreamEnd { status, error, .. } = ev {
            let (tail, c) = (h.live.remove(i).3, &mut h.counts);
            match uiwire::stat::receipt(&tail) {
                _ if status / 100 != 2 => c[1] = c[1].wrapping_add(1),
                Some(r) => (3..6).for_each(|i| c[i] = c[i].wrapping_add(r[i - 3])),
                None => c[2] = c[2].wrapping_add(1),
            }
            end(k, pid, id, status, &error);
        } else if let Heard::Chunk { data, .. } = ev {
            let tail = &mut h.live[i].3;
            tail.extend_from_slice(&data);
            if let Some(cut) = tail.len().checked_sub(TAIL) {
                *tail = tail[cut..].to_vec();
            }
            let data = data.chunks(CHUNK).map(|d| Event::AiData { id, data: d.to_vec() });
            data.for_each(|ev| k.post_event(pid, &ev.encode()));
        }
    }

    /// Carries out the asks (streams and aborts to `ctl`, events to the processes through
    /// `k`) and retells changed settings; whether they changed.
    pub fn pump(&self, ctl: &mut Ctl, k: &mut Kernel) -> bool {
        let h = &mut *self.0.borrow_mut();
        // A process that ended (Ctrl+C in a terminal, a kill) hears no more: its requests stop
        // as at its Close, though no window closed for it, before another byte comes.
        for l in &h.live {
            if !k.runs(l.1) {
                h.asks.push((l.1, Request::Close));
            }
        }
        for (pid, r) in mem::take(&mut h.asks) {
            // A cancel ends its request; a process done aborts all of its own, unanswered.
            let cancel = match r {
                // One whose process ended since it asked (in this flush) is never sent.
                Request::Ai { id, body } => {
                    if k.runs(pid) {
                        h.start(ctl, k, (pid, id), body);
                    }
                    continue;
                }
                Request::AiCancel { id } => Some(id),
                Request::Watch { on } => {
                    h.watch = if on { Some(pid) } else { h.watch.filter(|&w| w != pid) };
                    h.fresh |= on;
                    continue;
                }
                Request::End { pid } => {
                    k.kill(pid, KILLED);
                    continue;
                }
                r @ (Request::Pair { .. }
                | Request::Measure
                | Request::Job { .. }
                | Request::Ask { .. }
                | Request::Serve { .. }
                | Request::Test) => {
                    h.mesh.push((pid, r));
                    continue;
                }
                _ => None,
            };
            h.told.retain(|&p| p != pid || cancel.is_some());
            if cancel.is_none() && h.watch == Some(pid) {
                h.watch = None;
            }
            let gone = |l: &(u32, u32, u32, Vec<u8>)| l.1 == pid && cancel.is_none_or(|c| c == l.2);
            while let Some(i) = h.live.iter().position(gone) {
                let (sid, _, id, _) = h.live.remove(i);
                h.counts[2] = h.counts[2].wrapping_add(1);
                ctl.abort(sid);
                cancel.into_iter().for_each(|_| end(k, pid, id, 0, "cancelled"));
            }
        }
        let config = mem::take(&mut h.retell).then(|| h.config().encode());
        config.iter().for_each(|c| h.told.iter().for_each(|&pid| k.post_event(pid, c)));
        config.is_some()
    }
}

fn end(k: &mut Kernel, pid: u32, id: u32, status: u16, error: &str) {
    k.post_event(pid, &Event::AiEnd { id, status, error: error.into() }.encode());
}
