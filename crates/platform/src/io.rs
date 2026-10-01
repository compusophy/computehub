//! The page beyond the canvas, as [`Ctl`](crate::Ctl) effects and the events
//! they cause: the hidden `<textarea>`, WebSockets, fetch, the location hash
//! and the guard on leaving the page.

use std::rc::Rc;

use js_sys::{ArrayBuffer, Promise, Uint8Array};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{
    BinaryType, CloseEvent, CompositionEvent, Document, Element, Event as DomEvent, EventTarget,
    HtmlTextAreaElement, InputEvent, MessageEvent, Response, WebSocket, Window,
};

use crate::ctl::{Effect, is_relative_url};
use crate::{Event, Shared, WsEvent, dispatch, has, js_text};

/// The text sink's inline style: invisible, out of the way, and 16 px so
/// iOS does not zoom the page when it takes focus.
const SINK_STYLE: &str = "position:fixed;left:0;top:0;width:1px;height:1px;margin:0;\
border:0;padding:0;opacity:0;resize:none;overflow:hidden;font-size:16px;pointer-events:none";

/// An open (or opening) WebSocket. Its events are matched to it by their
/// target, so a socket that was replaced or closed is simply no longer found.
pub(crate) struct Sock {
    id: u32,
    ws: WebSocket,
    /// Sends made before `open`.
    queue: Vec<Vec<u8>>,
}

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
            Effect::WsOpen { id, url } => ws_open(s, id, &url),
            Effect::WsSend { id, bytes } => ws_send(s, id, bytes),
            Effect::WsClose { id } => ws_close(s, id),
            Effect::Fetch { id, url } => fetch(s, id, &url),
            Effect::ClearHash => clear_hash(&s.window),
            Effect::GuardUnload(on) => guard_unload(s, on),
        }
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

fn ws_open(s: &Shared, id: u32, url: &str) {
    ws_close(s, id);
    let ws = match WebSocket::new(url) {
        Ok(ws) => ws,
        Err(e) => {
            let (ws, reason) = (|ev| Event::Ws { id, ev }, js_text(&e));
            later(s, ws(WsEvent::Error));
            return later(s, ws(WsEvent::Closed { code: 1006, reason }));
        }
    };
    ws.set_binary_type(BinaryType::Arraybuffer);
    let f = Some(&s.ws_fn);
    ws.set_onopen(f);
    ws.set_onmessage(f);
    ws.set_onerror(f);
    ws.set_onclose(f);
    s.sockets.borrow_mut().push(Sock { id, ws, queue: Vec::new() });
}

fn ws_send(s: &Shared, id: u32, bytes: Vec<u8>) {
    let mut socks = s.sockets.borrow_mut();
    let Some(sock) = socks.iter_mut().find(|k| k.id == id) else {
        return;
    };
    if sock.ws.ready_state() == WebSocket::CONNECTING {
        sock.queue.push(bytes);
    } else {
        // Throws only when closing or closed: the close event will tell.
        let _ = sock.ws.send_with_u8_array(&bytes);
    }
}

fn ws_close(s: &Shared, id: u32) {
    let mut socks = s.sockets.borrow_mut();
    if let Some(i) = socks.iter().position(|k| k.id == id) {
        let _ = socks.swap_remove(i).ws.close_with_code(1000);
    }
}

/// Every WebSocket event: its target is the socket.
pub(crate) fn on_ws(s: &Rc<Shared>, e: &DomEvent) {
    let Some(target) = e.target() else {
        return;
    };
    let mut socks = s.sockets.borrow_mut();
    let Some(i) = socks.iter().position(|k| *k.ws == target) else {
        return;
    };
    let id = socks[i].id;
    let ev = match &*e.type_() {
        "open" => {
            let sock = &mut socks[i];
            for bytes in core::mem::take(&mut sock.queue) {
                let _ = sock.ws.send_with_u8_array(&bytes);
            }
            WsEvent::Open
        }
        "message" => {
            let Some(data) = e.dyn_ref::<MessageEvent>().map(MessageEvent::data) else {
                return;
            };
            WsEvent::Data(match data.dyn_ref::<ArrayBuffer>() {
                Some(buf) => Uint8Array::new(buf).to_vec(),
                None => data.as_string().unwrap_or_default().into_bytes(),
            })
        }
        "error" => WsEvent::Error,
        "close" => {
            socks.swap_remove(i);
            let c = e.dyn_ref::<CloseEvent>();
            let code = c.map_or(1006, CloseEvent::code);
            let reason = c.map(CloseEvent::reason).unwrap_or_default();
            WsEvent::Closed { code, reason }
        }
        _ => return,
    };
    drop(socks);
    dispatch(s, Event::Ws { id, ev });
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

/// Removes the hash from the address bar with `history.replaceState`.
pub(crate) fn clear_hash(window: &Window) {
    let loc = window.location();
    let (Ok(path), Ok(query), Ok(history)) = (loc.pathname(), loc.search(), window.history())
    else {
        return;
    };
    let state = history.state().unwrap_or(JsValue::NULL);
    let url = [path, query].concat();
    let _ = history.replace_state_with_url(&state, "", Some(&url));
}

/// Adds (`on`) or removes the `beforeunload` listener that makes leaving the
/// page ask first; asking for the state it is in does nothing.
fn guard_unload(s: &Shared, on: bool) {
    if s.guarded.get() == on {
        return;
    }
    let (win, f): (&EventTarget, _) = (&s.window, &s.unload_fn);
    let done = if on {
        win.add_event_listener_with_callback("beforeunload", f)
    } else {
        win.remove_event_listener_with_callback("beforeunload", f)
    };
    if done.is_ok() {
        s.guarded.set(on);
    }
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
