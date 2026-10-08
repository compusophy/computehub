//! The page beyond the canvas: [`Effect`]s applied, and the events they cause.

use std::mem::ManuallyDrop;
use std::rc::Rc;

use js_sys::{Array, Function, Object, Promise, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{
    AbortController, CompositionEvent, Document, Event as DomEvent, HtmlTextAreaElement,
    InputEvent, ReadableStream, ReadableStreamDefaultReader, Response, Storage,
};

use crate::ctl::{Effect, is_relative_url};
use crate::{Event, Shared, arm, bytes, dispatch, has, js_text, proc, request_frame};

/// Invisible and out of the way; 16 px so iOS does not zoom when it focuses.
const SINK_STYLE: &str = "position:fixed;left:0;top:0;width:1px;height:1px;margin:0;\
border:0;padding:0;opacity:0;resize:none;overflow:hidden;font-size:16px;pointer-events:none";

/// Appends the hidden `<textarea>` that all text input goes through.
pub(crate) fn text_sink(document: &Document) -> Result<HtmlTextAreaElement, JsValue> {
    let sink: HtmlTextAreaElement = document.create_element("textarea")?.unchecked_into();
    #[rustfmt::skip]
    let attrs = [("style", SINK_STYLE), ("autocapitalize", "off"), ("autocomplete", "off"),
        ("autocorrect", "off"), ("spellcheck", "false"), ("aria-hidden", "true"),
        ("tabindex", "-1")];
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
            Effect::FrameIn(ms) => arm(s, &s.frame_timer, &s.frame_fn, ms),
            Effect::Stream { id, url, headers, body } => stream(s, id, &url, headers, &body),
            Effect::Abort(id) => _ = take(s, id).map(|a| a.abort()),
            Effect::Remove(key) => _ = storage().map(|st| st.remove_item(&key)),
            Effect::Erase(prefix) => [storage(), session()].into_iter().flatten().for_each(|st| {
                // From the last: a removal moves the keys after it down.
                for i in (0..st.length().unwrap_or(0)).rev() {
                    let key = st.key(i).ok().flatten().filter(|k| k.starts_with(prefix.as_str()));
                    _ = key.map(|k| st.remove_item(&k));
                }
            }),
            Effect::Session { key, value } => {
                let st = session();
                _ = st.map(|st| value.map_or(st.remove_item(&key), |v| st.set_item(&key, &v)));
            }
            Effect::Reload => _ = s.window.location().reload(),
            Effect::InputMode(numeric) => {
                let _ = match numeric {
                    true => s.sink.set_attribute("inputmode", "numeric"),
                    false => s.sink.remove_attribute("inputmode"),
                };
            }
            Effect::Derive { id, pin, salt, iterations } => derive(s, id, &pin, &salt, iterations),
            Effect::Link { id, cert, offer } => crate::link::link(s, id, &cert, offer),
            Effect::Accept { id, answer } => crate::link::accept(s, id, &answer),
            Effect::LinkSend { id, data } => crate::link::send(s, id, &data),
            Effect::Unlink(id) => crate::link::close(s, id),
            Effect::Estimate => crate::link::estimate(s),
        }
    }
}

/// `window.localStorage`, or `None` when it is missing or throws.
pub(crate) fn storage() -> Option<Storage> {
    crate::window()?.local_storage().ok().flatten()
}

/// `window.sessionStorage`, or `None` when it is missing or throws.
pub(crate) fn session() -> Option<Storage> {
    crate::window()?.session_storage().ok().flatten()
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
    let text = inserted(&ie.input_type(), ie.data(), || s.sink.value());
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

/// The text an `input` event of `input_type` inserted: its `data`, else what
/// the `sink` holds (a paste). Text of many lines typed in at once comes as
/// an event a line, each break one with no data, and the first event finds
/// the sink holding all of it: so a typed break with the sink empty, or
/// holding text that starts with a break, is one break. None but from
/// `insert*` types, nor from the composition ones, which browsers order
/// inconsistently with `compositionend`.
pub(crate) fn inserted(
    input_type: &str,
    data: Option<String>,
    sink: impl FnOnce() -> String,
) -> String {
    if !input_type.starts_with("insert") || has(input_type, "Composition") {
        return String::new();
    }
    let typed = matches!(input_type, "insertText" | "insertLineBreak" | "insertParagraph");
    match (data.filter(|d| !d.is_empty()), typed) {
        (Some(d), _) => d,
        (None, false) => sink(),
        (None, true) => match sink() {
            held if held.is_empty() || held.starts_with(['\n', '\r']) => "\n".into(),
            held => held,
        },
    }
}

fn fetch(s: &Rc<Shared>, id: u32, url: &str) {
    if !is_relative_url(url) {
        let result = Err(String::from("not a same-origin relative URL: ") + url);
        return later(s, Event::Fetched { id, result });
    }
    settle(s, id, &s.window.fetch_with_str(url), (on_response, failed));
}

/// What runs when a fetch promise settles: `(state, fetch id, value)`.
type Settled = fn(&Rc<Shared>, u32, JsValue);

/// `f` as a promise callback for fetch or stream `id`: every one is this closure type.
fn callback(s: &Rc<Shared>, id: u32, f: Settled) -> Closure<dyn FnMut(JsValue)> {
    let s = s.clone();
    Closure::new(move |v| f(&s, id, v))
}

/// Runs `ok` with what `p` resolves to, or `err` with what it rejects with; only one runs.
fn settle(s: &Rc<Shared>, id: u32, p: &Promise, (ok, err): (Settled, Settled)) {
    let (ok, err) = (callback(s, id, ok), callback(s, id, err));
    let _ = p.then2(&ok, &err);
    ok.forget();
    err.forget();
}

/// Fails fetch `id` with what its promise rejected with.
fn failed(s: &Rc<Shared>, id: u32, e: JsValue) {
    fetched(s, id, Err(js_text(&e)));
}

/// Reads the body of a 2xx `Response`, else fails the fetch.
fn on_response(s: &Rc<Shared>, id: u32, v: JsValue) {
    let r: Response = v.unchecked_into();
    let err = if !r.ok() {
        http_error(r.status())
    } else {
        match r.array_buffer() {
            Ok(body) => return settle(s, id, &body, (on_body, failed)),
            Err(e) => js_text(&e),
        }
    };
    fetched(s, id, Err(err));
}

/// `crypto.subtle[f](...args)`'s promise, if the page has it (a secure one) and it did not throw.
fn subtle(f: &str, args: &Array) -> Option<Promise> {
    let w = crate::window()?;
    let subtle = Reflect::get(&Reflect::get(&w, &"crypto".into()).ok()?, &"subtle".into()).ok()?;
    let f: Function = Reflect::get(&subtle, &f.into()).ok()?.dyn_into().ok()?;
    f.apply(&subtle, args).ok()?.dyn_into().ok()
}

/// PBKDF2 ([`crate::Ctl::derive`]): imports the PIN as a key; [`on_key`] derives from it with
/// the algorithm, which waits in the state's `derives`.
fn derive(s: &Rc<Shared>, id: u32, pin: &[u8], salt: &[u8], iterations: u32) {
    let (algo, set) = (Object::new(), |o: &Object, k: &str, v: &JsValue| {
        _ = Reflect::set(o, &k.into(), v);
    });
    set(&algo, "name", &"PBKDF2".into());
    set(&algo, "hash", &"SHA-256".into());
    set(&algo, "salt", &Uint8Array::from(salt));
    set(&algo, "iterations", &iterations.into());
    let (raw, use_) = (Uint8Array::from(pin), Array::of1(&"deriveBits".into()));
    let args = Array::of5(&"raw".into(), &raw, &"PBKDF2".into(), &false.into(), &use_);
    match subtle("importKey", &args) {
        Some(p) => {
            s.derives.borrow_mut().push((id, algo));
            settle(s, id, &p, (on_key, underived));
        }
        None => {
            later(s, Event::Derived { id, result: Err("no crypto.subtle: needs https".into()) })
        }
    }
}

/// The PIN's key: derives 256 bits from it.
fn on_key(s: &Rc<Shared>, id: u32, key: JsValue) {
    let mut list = s.derives.borrow_mut();
    let algo = list.iter().position(|d| d.0 == id).map(|i| list.remove(i).1);
    drop(list);
    let args = Array::of3(&algo.unwrap_or_default(), &key, &256.into());
    match subtle("deriveBits", &args) {
        Some(p) => settle(s, id, &p, (on_bits, underived)),
        None => underived(s, id, "deriveBits".into()),
    }
}

fn on_bits(s: &Rc<Shared>, id: u32, buf: JsValue) {
    dispatch(s, Event::Derived { id, result: Ok(bytes(&Uint8Array::new(&buf))) });
}

fn underived(s: &Rc<Shared>, id: u32, e: JsValue) {
    s.derives.borrow_mut().retain(|d| d.0 != id);
    dispatch(s, Event::Derived { id, result: Err(js_text(&e)) });
}

fn on_body(s: &Rc<Shared>, id: u32, buf: JsValue) {
    fetched(s, id, Ok(bytes(&Uint8Array::new(&buf))));
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
pub(crate) fn later(s: &Shared, ev: Event) {
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

/// A stream ([`crate::Ctl::stream`]): its abort handle, status, body reader, and the callback
/// its promises settle to (kept: one may yet run). Plain objects, not web-sys setters: less glue.
pub(crate) struct Stream {
    id: u32,
    abort: AbortController,
    status: u16,
    reader: Option<ReadableStreamDefaultReader>,
    cb: ManuallyDrop<Closure<dyn FnMut(JsValue)>>,
}

fn stream(s: &Rc<Shared>, id: u32, url: &str, headers: Vec<(&str, String)>, body: &[u8]) {
    let Ok(abort) = AbortController::new() else { return };
    let set = |o: &Object, k: &str, v: &JsValue| _ = Reflect::set(o, &k.into(), v);
    let (init, map) = (Object::new(), Object::new());
    headers.iter().for_each(|(k, v)| set(&map, k, &v.into()));
    set(&init, "method", &(if body.is_empty() { "GET" } else { "POST" }).into());
    set(&init, "headers", &map);
    if !body.is_empty() {
        set(&init, "body", &Uint8Array::from(body));
    }
    set(&init, "signal", &Reflect::get(&abort, &"signal".into()).unwrap_or_default());
    let cb = ManuallyDrop::new(callback(s, id, on_stream));
    let _ = s.window.fetch_with_str_and_init(url, init.unchecked_ref()).then2(&cb, &cb);
    s.streams.borrow_mut().push(Stream { id, abort, status: 0, reader: None, cb });
}

/// The response (its status kept, its body read), a read (a chunk, then the next read), or a
/// failure (an `Error`, as a rejection is).
fn on_stream(s: &Rc<Shared>, id: u32, v: JsValue) {
    if v.is_instance_of::<js_sys::Error>() {
        return end(s, id, 0, "network");
    }
    let field = |k: &str| Reflect::get(&v, &k.into()).unwrap_or_default();
    let mut list = s.streams.borrow_mut();
    let Some(t) = list.iter_mut().find(|t| t.id == id) else { return };
    let data = match &t.reader {
        None => {
            let body = field("body").unchecked_into::<ReadableStream>();
            t.reader = body.is_truthy().then(|| body.get_reader().unchecked_into());
            t.status = v.unchecked_ref::<Response>().status();
            t.reader.as_ref().map(|_| Vec::new())
        }
        Some(_) if field("done").is_truthy() => None,
        Some(_) => Some(bytes(&Uint8Array::new(&field("value")))),
    };
    let status = t.status;
    drop(list);
    let Some(data) = data else { return end(s, id, status, "") };
    if !data.is_empty() {
        dispatch(s, Event::Chunk { id, data });
    }
    // The next read, unless the chunk's handler aborted the stream.
    if let Some(t) = s.streams.borrow().iter().find(|t| t.id == id) {
        let _ = t.reader.as_ref().map(|r| r.read().then2(&t.cb, &t.cb));
    }
}

/// Removes stream `id` if it is on; its abort handle.
fn take(s: &Shared, id: u32) -> Option<AbortController> {
    let i = s.streams.borrow().iter().position(|t| t.id == id)?;
    Some(s.streams.borrow_mut().remove(i).abort)
}

fn end(s: &Rc<Shared>, id: u32, status: u16, error: &str) {
    _ = take(s, id).map(|_| dispatch(s, Event::StreamEnd { id, status, error: error.into() }));
}
