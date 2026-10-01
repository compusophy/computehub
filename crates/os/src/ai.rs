//! The free AI: no key and no provider to pick. A program's [`Request::Ai`] is POSTed to [`URL`],
//! compusophy's own server function, which forwards it to the model and streams the response body
//! back: to the program as [`Event::AiData`]s ([`CHUNK`] at most), then an [`Event::AiEnd`]. The
//! one setting, the model (one of [`MODELS`]), lives in `localStorage` ([`MODEL`]); programs hear
//! it as [`Event::Config`] (after their first Resize, again on each change), apps as [`AiStatus`].

use std::cell::RefCell;
use std::mem;
use std::rc::Rc;

use platform::{Ctl, Event as Heard};
use ui::{AiStatus, kernel::Kernel};
use uiwire::{Event, Request};

/// The same-origin endpoint every request goes to.
pub const URL: &str = "/api/ai";
/// The `localStorage` key of the model, the models on offer (the first the default, which
/// anything else stored becomes) and the largest AiData.
pub const MODEL: &str = "compusophy.ai.model";
pub const MODELS: [&str; 2] = ["zai/glm-5.3", "zai/glm-5.3-flash"];
pub const DEFAULT_MODEL: &str = MODELS[0];
pub const CHUNK: usize = 32 << 10;

/// The page's AI state, shared (clones are one) by the desktop and its program windows.
#[derive(Clone, Default)]
pub struct Ai(pub(crate) Rc<RefCell<Hub>>);

/// The model (an index of [`MODELS`]), the asks, requests in flight (stream id, pid, id; two a
/// process), the last stream id, who to tell the settings and whether to.
#[derive(Default)]
pub(crate) struct Hub {
    model: usize,
    asks: Vec<(u32, Request)>,
    live: Vec<(u32, u32, u32)>,
    last: u32,
    told: Vec<u32>,
    retell: bool,
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
        self.live.push((self.last, pid, id));
        let json = vec![("Content-Type", "application/json".into())];
        ctl.stream(self.last, URL, json, body.into());
    }
}

impl Ai {
    /// A new desktop: the model from storage (told at the next [`Ai::pump`]), nothing in flight.
    pub fn load(&self, ctl: &Ctl) {
        let mut h = Hub::default();
        h.set(&ctl.storage_get(MODEL).unwrap_or_default());
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
        ctl.storage_set(MODEL, MODELS[h.model]);
    }

    /// The settings for process `pid`, which hears them again on every change.
    pub fn hello(&self, pid: u32) -> Event {
        self.0.borrow_mut().told.push(pid);
        self.0.borrow().config()
    }

    /// Process `pid` asks for `r`, for [`Ai::pump`]: Ai, AiCancel, or Close (done: abort all).
    pub fn ask(&self, pid: u32, r: Request) {
        self.0.borrow_mut().asks.push((pid, r));
    }

    /// A stream's news ([`Heard::Chunk`], [`Heard::StreamEnd`]) for the process that asked.
    pub fn heard(&self, k: &mut Kernel, ev: Heard) {
        let h = &mut *self.0.borrow_mut();
        let (Heard::Chunk { id: sid, .. } | Heard::StreamEnd { id: sid, .. }) = ev else { return };
        let Some(i) = h.live.iter().position(|l| l.0 == sid) else { return };
        let (_, pid, id) = h.live[i];
        if let Heard::StreamEnd { status, error, .. } = ev {
            h.live.remove(i);
            end(k, pid, id, status, &error);
        } else if let Heard::Chunk { data, .. } = ev {
            let data = data.chunks(CHUNK).map(|d| Event::AiData { id, data: d.to_vec() });
            data.for_each(|ev| k.post_event(pid, &ev.encode()));
        }
    }

    /// Carries out the asks (streams and aborts to `ctl`, events to the processes through
    /// `k`) and retells changed settings; whether they changed.
    pub fn pump(&self, ctl: &mut Ctl, k: &mut Kernel) -> bool {
        let h = &mut *self.0.borrow_mut();
        for (pid, r) in mem::take(&mut h.asks) {
            // A cancel ends its request; a process done aborts all of its own, unanswered.
            let cancel = match r {
                Request::Ai { id, body } => {
                    h.start(ctl, k, (pid, id), body);
                    continue;
                }
                Request::AiCancel { id } => Some(id),
                _ => None,
            };
            h.told.retain(|&p| p != pid || cancel.is_some());
            let gone = |l: &(u32, u32, u32)| l.1 == pid && cancel.is_none_or(|c| c == l.2);
            while let Some(i) = h.live.iter().position(gone) {
                let (sid, _, id) = h.live.remove(i);
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
