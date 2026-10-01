//! AI routing. Settings live only in `localStorage` ([`PROVIDER`], [`KEY`], [`MODEL`]); programs
//! hear them as [`Event::Config`] (after their first Resize, again on each change), apps as
//! [`AiStatus`], neither ever the key. A program's [`Request::Ai`] goes to the provider with the
//! key, the response body back as [`Event::AiData`]s ([`CHUNK`] at most), then an [`Event::AiEnd`].

use std::cell::RefCell;
use std::mem;
use std::rc::Rc;

use platform::{Ctl, Event as Heard};
use ui::{AiStatus, kernel::Kernel};
use uiwire::{Event, Request};

/// The `localStorage` keys, the default model and the largest AiData.
pub const PROVIDER: &str = "compusophy.ai.provider";
pub const KEY: &str = "compusophy.ai.key";
pub const MODEL: &str = "compusophy.ai.model";
pub const DEFAULT_MODEL: &str = "zai/glm-5.3";
pub const CHUNK: usize = 32 << 10;
/// Each provider's chat-completions URL; the mock is tools/serve's, on localhost only.
const URLS: [(&str, &str); 3] = [
    ("gateway", "https://ai-gateway.vercel.sh/v1/chat/completions"),
    ("openrouter", "https://openrouter.ai/api/v1/chat/completions"),
    ("mock", "/mock/chat"),
];

/// The page's AI state, shared (clones are one) by the desktop and its program windows.
#[derive(Clone, Default)]
pub struct Ai(pub(crate) Rc<RefCell<Hub>>);

/// The settings (provider: an index of [`URLS`]), whether on localhost, the asks, requests in
/// flight (stream id, pid, id; two a process), the last stream id, who to tell settings and if.
#[derive(Default)]
pub(crate) struct Hub {
    provider: usize,
    model: String,
    key: String,
    pub(crate) localhost: bool,
    asks: Vec<(u32, Request)>,
    live: Vec<(u32, u32, u32)>,
    last: u32,
    told: Vec<u32>,
    retell: bool,
}

impl Hub {
    /// An unknown provider (or the mock off localhost) is the gateway, an empty model the
    /// default; `None` keeps the key. (Settings trims what it saves.)
    fn set(&mut self, provider: &str, key: Option<&str>, model: &str) {
        let known = URLS.iter().position(|u| u.0 == provider && (u.0 != "mock" || self.localhost));
        self.model = if model.is_empty() { DEFAULT_MODEL } else { model }.into();
        self.key = key.map_or(mem::take(&mut self.key), Into::into);
        (self.provider, self.retell) = (known.unwrap_or(0), true);
    }

    fn config(&self) -> Event {
        let (provider, model) = (URLS[self.provider].0.into(), self.model.clone());
        Event::Config { provider, model, has_key: (!self.key.is_empty()).into() }
    }

    /// Starts request `id` of `pid`, or ends it at once: a third, or no key.
    fn start(&mut self, ctl: &mut Ctl, k: &mut Kernel, (pid, id): (u32, u32), body: String) {
        let (mock, url) = (self.provider == 2, URLS[self.provider].1);
        let busy = self.live.iter().filter(|l| l.1 == pid).count() >= 2;
        if busy || !mock && self.key.is_empty() {
            return end(k, pid, id, 0, if busy { "busy" } else { "no key" });
        }
        self.last += 1;
        self.live.push((self.last, pid, id));
        let json = ("Content-Type", "application/json".into());
        let auth = ("Authorization", ["Bearer ", &self.key].concat());
        ctl.stream(self.last, url, if mock { vec![] } else { vec![auth, json] }, body.into());
    }
}

impl Ai {
    /// A new desktop: the settings from storage (told at the next [`Ai::pump`]), nothing in flight.
    pub fn load(&self, ctl: &Ctl) {
        let (host, get) = (ctl.hostname(), |k| ctl.storage_get(k).unwrap_or_default());
        let localhost = matches!(host.as_deref(), Some("localhost" | "127.0.0.1"));
        let mut h = Hub { localhost, ..Hub::default() };
        h.set(&get(PROVIDER), Some(&get(KEY)), &get(MODEL));
        *self.0.borrow_mut() = h;
    }

    /// What apps see (the key's last 4 chars at most), and whether the page is on localhost.
    pub fn status(&self) -> (AiStatus, bool) {
        let h = self.0.borrow();
        (AiStatus::default().saved(URLS[h.provider].0, Some(&h.key), &h.model), h.localhost)
    }

    /// Saves settings an app chose; `key` `None` keeps the stored one, `Some("")` clears it.
    pub fn configure(&self, ctl: &mut Ctl, provider: &str, key: Option<String>, model: &str) {
        let mut h = self.0.borrow_mut();
        h.set(provider, key.as_deref(), model);
        ctl.storage_set(PROVIDER, URLS[h.provider].0);
        ctl.storage_set(MODEL, &h.model);
        ctl.storage_set(KEY, &h.key);
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
