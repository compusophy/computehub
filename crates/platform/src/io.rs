//! The page beyond the canvas, as [`Ctl`](crate::Ctl) effects and the events
//! they cause: the hidden `<textarea>`, fetch, frames, the cursor and
//! `localStorage`.

use std::rc::Rc;

use js_sys::{ArrayBuffer, Promise, Uint8Array};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{
    CompositionEvent, Document, Element, Event as DomEvent, HtmlTextAreaElement, InputEvent,
    Response, Storage,
};

use crate::ctl::{Cursor, Effect, is_relative_url};
use crate::{Event, Shared, dispatch, has, js_text, request_frame};

/// The text sink's inline style: invisible, out of the way, and 16 px so
/// iOS does not zoom the page when it takes focus.
const SINK_STYLE: &str = "position:fixed;left:0;top:0;width:1px;height:1px;margin:0;\
border:0;padding:0;opacity:0;resize:none;overflow:hidden;font-size:16px;pointer-events:none";

/// Creates the hidden `<textarea>` that text input goes through and appends
/// it to the body.
pub(crate) fn text_sink(document: &Document) -> Result<HtmlTextAreaElement, JsValue> {
    let sink: HtmlTextAreaElement = document
        .create_element("textarea")?
        .dyn_into()
        .map_err(|_| "<textarea> is not an HTMLTextAreaElement")?;
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

/// Applies an app's [`Ctl`](crate::Ctl) requests in order. No app borrow is
/// held, so events they cause synchronously can be dispatched.
pub(crate) fn apply(s: &Rc<Shared>, effects: Vec<Effect>) {
    for fx in effects {
        match fx {
            Effect::TextInput(on) => text_input(s, on),
            Effect::Fetch { id, url } => fetch(s, id, &url),
            Effect::RequestFrame => request_frame(s),
            Effect::Cursor(c) => cursor(s, c),
            Effect::Store { key, value } => {
                if let Some(st) = storage() {
                    let _ = st.set_item(&key, &value);
                }
            }
        }
    }
}

/// `window.localStorage`, or `None` when there is no window or reading it
/// throws (storage blocked, some private modes).
pub(crate) fn storage() -> Option<Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// Shows `c` over the canvas, writing `style.cursor` only when it changes.
fn cursor(s: &Shared, c: Cursor) {
    if s.cursor.replace(c) != c {
        let _ = s.canvas.style().set_property("cursor", c.css());
    }
}

fn text_input(s: &Shared, on: bool) {
    s.typing.set(on);
    let sink: &Element = s.sink.as_ref();
    let focused = s.document.active_element().as_ref() == Some(sink);
    // Blurring a focused sink first brings back a phone keyboard that the
    // user dismissed; focus() alone would do nothing.
    if focused {
        let _ = s.sink.blur();
    }
    if on {
        let _ = s.sink.focus();
    }
}

/// An `input` event on the sink: sends the inserted text, unless an IME is
/// composing (its text comes with `compositionend`), then empties the sink.
pub(crate) fn on_input(s: &Rc<Shared>, e: &DomEvent) {
    let Some(ie) = e.dyn_ref::<InputEvent>() else {
        return;
    };
    if ie.is_composing() {
        return;
    }
    let text = if inserts_text(&ie.input_type()) {
        let data = ie.data().filter(|d| !d.is_empty());
        data.unwrap_or_else(|| s.sink.value())
    } else {
        String::new()
    };
    s.sink.set_value("");
    if !text.is_empty() {
        dispatch(s, Event::Text(text));
    }
}

/// `compositionend`: sends the committed string and empties the sink.
pub(crate) fn on_composition_end(s: &Rc<Shared>, e: &DomEvent) {
    let data = e.dyn_ref::<CompositionEvent>().and_then(CompositionEvent::data);
    s.sink.set_value("");
    if let Some(text) = data.filter(|t| !t.is_empty()) {
        dispatch(s, Event::Text(text));
    }
}

/// Whether an `input` event of `input_type` carries text to send: the
/// `insert*` types, except the composition ones (browsers disagree on
/// whether those come before or after `compositionend`, which already has
/// the text).
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

/// Runs `ok` with what `p` resolves to, or reports fetch `id` failed with
/// what it rejects with. Both callbacks are one closure type.
fn settle(s: &Rc<Shared>, id: u32, p: &Promise, ok: Settled) {
    let cb = |f: Settled| {
        let s = s.clone();
        Closure::<dyn FnMut(JsValue)>::new(move |v| f(&s, id, v))
    };
    let (ok, err) = (cb(ok), cb(|s, id, e| fetched(s, id, Err(js_text(&e)))));
    let _ = p.then2(&ok, &err);
    // Only one of the two runs; the JS garbage collector frees both.
    ok.forget();
    err.forget();
}

/// A fetch resolved: reads the body of a 2xx `Response`, else reports why not.
fn on_response(s: &Rc<Shared>, id: u32, v: JsValue) {
    let err = match v.dyn_into::<Response>() {
        Ok(r) if r.ok() => match r.array_buffer() {
            Ok(body) => return settle(s, id, &body, on_body),
            Err(e) => js_text(&e),
        },
        Ok(r) => http_error(r.status()),
        Err(_) => "fetch resolved to a non-Response".to_owned(),
    };
    fetched(s, id, Err(err));
}

/// A body was read: sends its bytes.
fn on_body(s: &Rc<Shared>, id: u32, buf: JsValue) {
    let bytes = buf.dyn_ref::<ArrayBuffer>().map(|b| Uint8Array::new(b).to_vec());
    fetched(s, id, bytes.ok_or_else(|| "no body".to_owned()));
}

/// `"HTTP <status>"`, in decimal without the formatting machinery.
pub(crate) fn http_error(status: u16) -> String {
    let (mut digits, mut n, mut i) = ([0u8; 5], status, 5);
    loop {
        i -= 1;
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    let mut out = String::from("HTTP ");
    out.extend(digits[i..].iter().map(|&d| char::from(d)));
    out
}

fn fetched(s: &Rc<Shared>, id: u32, result: Result<Vec<u8>, String>) {
    dispatch(s, Event::Fetched { id, result });
}

/// Dispatches `ev` from a microtask: after the current DOM event, never in
/// the middle of the effects that produced it. Events queued together are
/// dispatched in order by one microtask.
fn later(s: &Shared, ev: Event) {
    let mut queue = s.later.borrow_mut();
    if queue.is_empty() {
        s.window.queue_microtask(&s.later_fn);
    }
    queue.push(ev);
}

/// The microtask of [`later`]: dispatches what it queued.
pub(crate) fn flush_later(s: &Rc<Shared>, _: &DomEvent) {
    let evs = core::mem::take(&mut *s.later.borrow_mut());
    for ev in evs {
        dispatch(s, ev);
    }
}
