//! The page beyond the canvas: [`Effect`]s applied, and the events they cause.

use std::rc::Rc;

use js_sys::{Promise, Uint8Array};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{
    CompositionEvent, Document, Event as DomEvent, HtmlTextAreaElement, InputEvent, Response,
    Storage,
};

use crate::ctl::{Effect, is_relative_url};
use crate::{Event, Shared, arm, dispatch, has, js_text, proc, request_frame};

/// Invisible and out of the way; 16 px so iOS does not zoom when it focuses.
const SINK_STYLE: &str = "position:fixed;left:0;top:0;width:1px;height:1px;margin:0;\
border:0;padding:0;opacity:0;resize:none;overflow:hidden;font-size:16px;pointer-events:none";

/// Appends the hidden `<textarea>` that all text input goes through.
pub(crate) fn text_sink(document: &Document) -> Result<HtmlTextAreaElement, JsValue> {
    let sink: HtmlTextAreaElement = document.create_element("textarea")?.unchecked_into();
    let attrs = [
        ("style", SINK_STYLE),
        ("autocapitalize", "off"),
        ("autocomplete", "off"),
        ("autocorrect", "off"),
        ("spellcheck", "false"),
        ("aria-hidden", "true"),
        ("tabindex", "-1"),
    ];
    for (name, value) in attrs {
        sink.set_attribute(name, value)?;
    }
    document.body().ok_or("no <body>")?.append_child(&sink)?;
    Ok(sink)
}

/// Applies an app's requests in order, with no app borrow held.
pub(crate) fn apply(s: &Rc<Shared>, effects: Vec<Effect>) {
    for fx in effects {
        match fx {
            Effect::TextInput(on) => text_input(s, on),
            Effect::Fetch { id, url } => fetch(s, id, &url),
            Effect::RequestFrame => request_frame(s),
            Effect::Cursor(c) => {
                if s.cursor.replace(c) != c {
                    let _ = s.canvas.style().set_property("cursor", c);
                }
            }
            Effect::Store { key, value } => _ = storage().map(|st| st.set_item(&key, &value)),
            Effect::Spawn { pid, sab } => proc::spawn(s, pid, sab),
            Effect::Start { pid, msg, program } => proc::post(s, pid, &msg, Some(program)),
            Effect::Send { pid, msg } => proc::post(s, pid, &msg, None),
            Effect::Reply { pid, errno, data } => proc::reply(s, pid, errno, &data),
            Effect::Word { pid, index, value } => proc::store(s, pid, &[], &[(index, value)]),
            Effect::Kill(pid) => proc::kill(s, pid),
            Effect::Wake(ms) => arm(s, &s.wake_timer, &s.wake_fn, ms),
        }
    }
}

/// `window.localStorage`, or `None` when it is missing or throws.
pub(crate) fn storage() -> Option<Storage> {
    crate::window()?.local_storage().ok().flatten()
}

fn text_input(s: &Shared, on: bool) {
    s.typing.set(on);
    // Blurring a focused sink first brings back a phone keyboard the user
    // dismissed; blurring one without focus does nothing.
    let _ = s.sink.blur();
    if on {
        let _ = s.sink.focus();
    }
}

/// Sends an `input` event's inserted text (not while an IME composes: that
/// text comes with `compositionend`), then empties the sink.
pub(crate) fn on_input(s: &Rc<Shared>, e: &DomEvent) {
    let Some(ie) = e.dyn_ref::<InputEvent>() else { return };
    if ie.is_composing() {
        return;
    }
    let text = if inserts_text(&ie.input_type()) {
        ie.data().filter(|d| !d.is_empty()).unwrap_or_else(|| s.sink.value())
    } else {
        String::new()
    };
    s.sink.set_value("");
    if !text.is_empty() {
        dispatch(s, Event::Text(text));
    }
}

/// Sends a composition's committed string, then empties the sink.
pub(crate) fn on_composition_end(s: &Rc<Shared>, e: &DomEvent) {
    let data = e.dyn_ref::<CompositionEvent>().and_then(CompositionEvent::data);
    s.sink.set_value("");
    if let Some(text) = data.filter(|t| !t.is_empty()) {
        dispatch(s, Event::Text(text));
    }
}

/// Whether an `input_type` carries text: `insert*` but not the composition
/// types, which browsers order inconsistently with `compositionend`.
pub(crate) fn inserts_text(input_type: &str) -> bool {
    input_type.starts_with("insert") && !has(input_type, "Composition")
}

fn fetch(s: &Rc<Shared>, id: u32, url: &str) {
    if !is_relative_url(url) {
        let result = Err(["not a same-origin relative URL: ", url].concat());
        return later(s, Event::Fetched { id, result });
    }
    settle(s, id, &s.window.fetch_with_str(url), on_response);
}

/// What runs when a fetch promise settles: `(state, fetch id, value)`.
type Settled = fn(&Rc<Shared>, u32, JsValue);

/// Runs `ok` with what `p` resolves to, or fails fetch `id` with what it
/// rejects with. Both callbacks are one closure type; only one runs.
fn settle(s: &Rc<Shared>, id: u32, p: &Promise, ok: Settled) {
    let cb = |f: Settled| {
        let s = s.clone();
        Closure::<dyn FnMut(JsValue)>::new(move |v| f(&s, id, v))
    };
    let (ok, err) = (cb(ok), cb(|s, id, e| fetched(s, id, Err(js_text(&e)))));
    let _ = p.then2(&ok, &err);
    ok.forget();
    err.forget();
}

/// Reads the body of a 2xx `Response`, else fails the fetch.
fn on_response(s: &Rc<Shared>, id: u32, v: JsValue) {
    let r: Response = v.unchecked_into();
    let err = if !r.ok() {
        http_error(r.status())
    } else {
        match r.array_buffer() {
            Ok(body) => return settle(s, id, &body, on_body),
            Err(e) => js_text(&e),
        }
    };
    fetched(s, id, Err(err));
}

fn on_body(s: &Rc<Shared>, id: u32, buf: JsValue) {
    fetched(s, id, Ok(Uint8Array::new(&buf).to_vec()));
}

/// `"HTTP <status>"`.
pub(crate) fn http_error(status: u16) -> String {
    let mut out = String::from("HTTP ");
    push_num(&mut out, status.into());
    out
}

/// Appends `n` in decimal, without the formatting machinery.
pub(crate) fn push_num(out: &mut String, n: u32) {
    let mut p = 1_000_000_000;
    while p > 1 && n < p {
        p /= 10;
    }
    while p > 0 {
        out.push(char::from(b'0' + (n / p % 10) as u8));
        p /= 10;
    }
}

fn fetched(s: &Rc<Shared>, id: u32, result: Result<Vec<u8>, String>) {
    dispatch(s, Event::Fetched { id, result });
}

/// Dispatches `ev` from a microtask, after the current DOM event; events
/// queued together go out in order from one microtask.
fn later(s: &Shared, ev: Event) {
    let mut queue = s.later.borrow_mut();
    if queue.is_empty() {
        s.window.queue_microtask(&s.later_fn);
    }
    queue.push(ev);
}

/// The microtask of [`later`].
pub(crate) fn flush_later(s: &Rc<Shared>) {
    let evs = core::mem::take(&mut *s.later.borrow_mut());
    for ev in evs {
        dispatch(s, ev);
    }
}
