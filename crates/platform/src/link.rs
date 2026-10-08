//! Links ([`crate::Ctl::link`]): a WebRTC data channel to another tab, always encrypted (DTLS),
//! direct where the network allows (a public STUN server finds the way out of a home network; no
//! relay). The tabs swap their descriptions themselves ([`crate::Event::Signal`],
//! [`crate::Ctl::accept`]): each is sent whole, its ICE candidates gathered in (or after
//! [`GATHER_MS`], whatever was gathered by then). Every link of the page uses one certificate,
//! kept in IndexedDB under the key the first link names (made there once, for a year), so this
//! tab's DTLS fingerprint stays the same across reloads and a peer can pin it; without
//! IndexedDB each link makes its own. A link whose channel closes, or whose setup fails, is
//! gone; one that never opens is the asker's to give up on. Plain `Reflect` calls rather than
//! web-sys's WebRTC types: less glue in the boot download.

use std::rc::Rc;

use js_sys::{Array, Function, Object, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue};

use crate::io::later;
use crate::{Event, Handler, Shared, bytes, dispatch, handler};

/// How long a description waits for its ICE candidates.
pub const GATHER_MS: i32 = 4000;
const DB: &str = "compusophy-mesh";
const STORE: &str = "certs";

/// A link: its id, peer connection, data channel (undefined until it has one), and whether its
/// description went out.
pub(crate) struct Link {
    id: u32,
    pc: JsValue,
    dc: JsValue,
    said: bool,
}

/// The page's certificate: its IndexedDB key and database, the certificate (null: none, each
/// link makes its own) once loaded, and the links waiting for it as (id, the offer to answer).
#[derive(Default)]
pub(crate) struct Cert {
    key: String,
    db: JsValue,
    cert: Option<JsValue>,
    waiting: Vec<(u32, Option<String>)>,
}

fn get(o: &JsValue, k: &str) -> JsValue {
    Reflect::get(o, &k.into()).unwrap_or_default()
}

fn set(o: &JsValue, k: &str, v: &JsValue) {
    _ = Reflect::set(o, &k.into(), v);
}

/// `o[f](...args)` with exactly the arguments given (WebRTC's methods have older overloads
/// that more arguments, even undefined ones, would pick); undefined if it threw or is no
/// function.
fn call(o: &JsValue, f: &str, args: &[&JsValue]) -> JsValue {
    let Ok(f) = get(o, f).dyn_into::<Function>() else { return JsValue::UNDEFINED };
    match args {
        [] => f.call0(o),
        [a] => f.call1(o, a),
        [a, b, ..] => f.call2(o, a, b),
    }
    .unwrap_or_default()
}

/// `o[ev]` runs `f` for link `id`.
fn on(s: &Rc<Shared>, o: &JsValue, ev: &str, id: u32, f: Handler) {
    set(o, ev, &handler(&Rc::downgrade(s), id, f));
}

/// When `p` resolves, `ok` runs for link `id`; if it rejects (or is no promise), the link goes.
fn then(s: &Rc<Shared>, p: &JsValue, id: u32, ok: Handler) {
    let me = Rc::downgrade(s);
    let (ok, err) = (handler(&me, id, ok), handler(&me, id, |s, id, _| unlinked(s, id)));
    if call(p, "then", &[&ok, &err]).is_undefined() {
        later(s, Event::Unlinked { id });
    }
}

fn global(k: &str) -> JsValue {
    crate::window().map(|w| get(&w, k)).unwrap_or_default()
}

/// The object store, in a read-write transaction.
fn store(s: &Shared) -> JsValue {
    let tx = call(&s.cert.borrow().db, "transaction", &[&STORE.into(), &"readwrite".into()]);
    call(&tx, "objectStore", &[&STORE.into()])
}

/// Makes link `id` once the certificate (kept under `key`) is loaded: an offer, or the answer to
/// `offer`.
pub(crate) fn link(s: &Rc<Shared>, id: u32, key: &str, offer: Option<String>) {
    if s.cert.borrow().cert.is_some() {
        return make(s, id, offer);
    }
    let mut c = s.cert.borrow_mut();
    c.waiting.push((id, offer));
    if c.waiting.len() > 1 {
        return;
    }
    c.key = key.into();
    drop(c);
    let req = call(&global("indexedDB"), "open", &[&DB.into(), &1.into()]);
    if req.is_undefined() {
        return certified(s, JsValue::NULL);
    }
    on(s, &req, "onupgradeneeded", 0, |_, _, e| {
        _ = call(&get(&get(e, "target"), "result"), "createObjectStore", &[&STORE.into()]);
    });
    on(s, &req, "onerror", 0, |s, _, _| make_cert(s));
    on(s, &req, "onsuccess", 0, |s, _, e| {
        s.cert.borrow_mut().db = get(&get(e, "target"), "result");
        let key: JsValue = s.cert.borrow().key.as_str().into();
        let got = Some(call(&store(s), "get", &[&key])).filter(|g| !g.is_undefined());
        let Some(got) = got else { return make_cert(s) };
        on(s, &got, "onerror", 0, |s, _, _| make_cert(s));
        on(s, &got, "onsuccess", 0, |s, _, e| {
            let cert = get(&get(e, "target"), "result");
            let fresh =
                get(&cert, "expires").as_f64().is_some_and(|t| t > js_sys::Date::now() + 864e5);
            if fresh { certified(s, cert) } else { make_cert(s) }
        });
    });
}

/// Makes a certificate (ECDSA P-256, a year), keeps it in IndexedDB and uses it.
fn make_cert(s: &Rc<Shared>) {
    let algo = Object::new();
    set(&algo, "name", &"ECDSA".into());
    set(&algo, "namedCurve", &"P-256".into());
    set(&algo, "expires", &3.15e10.into());
    let made = call(&global("RTCPeerConnection"), "generateCertificate", &[&algo]);
    let me = Rc::downgrade(s);
    let ok = handler(&me, 0, |s, _, cert| {
        let (cert, key): (&JsValue, JsValue) = (cert, s.cert.borrow().key.as_str().into());
        _ = call(&store(s), "put", &[cert, &key]);
        certified(s, cert.clone());
    });
    let failed = handler(&me, 0, |s, _, _| certified(s, JsValue::NULL));
    if call(&made, "then", &[&ok, &failed]).is_undefined() {
        certified(s, JsValue::NULL);
    }
}

/// The certificate is `cert` (null: none): the waiting links go on.
fn certified(s: &Rc<Shared>, cert: JsValue) {
    let waiting = {
        let mut c = s.cert.borrow_mut();
        c.cert = Some(cert);
        core::mem::take(&mut c.waiting)
    };
    waiting.into_iter().for_each(|(id, offer)| make(s, id, offer));
}

/// A description: `{type, sdp}`.
fn desc(ty: &str, sdp: &str) -> JsValue {
    let d = Object::new().into();
    set(&d, "type", &ty.into());
    set(&d, "sdp", &sdp.into());
    d
}

fn make(s: &Rc<Shared>, id: u32, offer: Option<String>) {
    let (cfg, server) = (Object::new(), Object::new());
    set(&server, "urls", &"stun:stun.l.google.com:19302".into());
    set(&cfg, "iceServers", &Array::of1(&server));
    let cert = s.cert.borrow().cert.clone().unwrap_or_default();
    if cert.is_object() {
        set(&cfg, "certificates", &Array::of1(&cert));
    }
    let class = global("RTCPeerConnection").dyn_into::<Function>();
    let Ok(pc) = class.and_then(|f| Reflect::construct(&f, &Array::of1(&cfg))) else {
        return later(s, Event::Unlinked { id });
    };
    on(s, &pc, "onicecandidate", id, |s, id, e| {
        if get(e, "candidate").is_null() {
            said(s, id);
        }
    });
    let mut dc = JsValue::UNDEFINED;
    match offer {
        None => {
            dc = call(&pc, "createDataChannel", &[&"mesh".into()]);
            wire(s, id, &dc);
            then(s, &call(&pc, "createOffer", &[]), id, describe);
        }
        Some(sdp) => {
            on(s, &pc, "ondatachannel", id, |s, id, e| {
                let dc = get(e, "channel");
                wire(s, id, &dc);
                find(s, id, |l| l.dc = dc);
            });
            let set_remote = call(&pc, "setRemoteDescription", &[&desc("offer", &sdp)]);
            then(s, &set_remote, id, |s, id, _| {
                let answer = find(s, id, |l| call(&l.pc, "createAnswer", &[]));
                then(s, &answer.unwrap_or_default(), id, describe);
            });
        }
    }
    s.links.borrow_mut().push(Link { id, pc, dc, said: false });
    let f = handler(&Rc::downgrade(s), id, |s, id, _| said(s, id));
    _ = s.window.set_timeout_with_callback_and_timeout_and_arguments_0(&f, GATHER_MS);
}

/// The link's own description: set it, which starts gathering candidates.
fn describe(s: &Rc<Shared>, id: u32, d: &web_sys::Event) {
    let set_local = find(s, id, |l| call(&l.pc, "setLocalDescription", &[d]));
    then(s, &set_local.unwrap_or_default(), id, |_, _, _| {});
}

/// Wires a data channel's open, messages and close to link `id`.
fn wire(s: &Rc<Shared>, id: u32, dc: &JsValue) {
    set(dc, "binaryType", &"arraybuffer".into());
    on(s, dc, "onopen", id, |s, id, _| _ = dispatch(s, Event::Linked { id }));
    on(s, dc, "onmessage", id, |s, id, e| {
        let data = bytes(&Uint8Array::new(&get(e, "data")));
        dispatch(s, Event::LinkData { id, data });
    });
    on(s, dc, "onclose", id, |s, id, _| unlinked(s, id));
}

/// Sends the link's description, once: candidates gathered, or the time for them is up.
fn said(s: &Rc<Shared>, id: u32) {
    let sdp = find(s, id, |l| match core::mem::replace(&mut l.said, true) {
        true => None,
        false => get(&get(&l.pc, "localDescription"), "sdp").as_string(),
    });
    if let Some(sdp) = sdp.flatten().filter(|d| !d.is_empty()) {
        dispatch(s, Event::Signal { id, sdp });
    }
}

/// `f` of link `id`, if there is one.
fn find<T>(s: &Shared, id: u32, f: impl FnOnce(&mut Link) -> T) -> Option<T> {
    s.links.borrow_mut().iter_mut().find(|l| l.id == id).map(f)
}

/// The peer's answer for the offer of link `id`.
pub(crate) fn accept(s: &Rc<Shared>, id: u32, sdp: &str) {
    let set_remote = find(s, id, |l| call(&l.pc, "setRemoteDescription", &[&desc("answer", sdp)]));
    then(s, &set_remote.unwrap_or_default(), id, |_, _, _| {});
}

/// Sends `data` on link `id` (nothing if its channel is not open).
pub(crate) fn send(s: &Shared, id: u32, data: &[u8]) {
    find(s, id, |l| call(&l.dc, "send", &[&Uint8Array::from(data)]));
}

/// Closes link `id`, quietly.
pub(crate) fn close(s: &Shared, id: u32) {
    let mut links = s.links.borrow_mut();
    if let Some(i) = links.iter().position(|l| l.id == id) {
        let l = links.remove(i);
        call(&l.dc, "close", &[]);
        call(&l.pc, "close", &[]);
    }
}

/// Link `id` failed or closed: it goes, and the app hears so once (after the current event:
/// this may run while effects apply).
fn unlinked(s: &Rc<Shared>, id: u32) {
    if find(s, id, |_| ()).is_some() {
        close(s, id);
        later(s, Event::Unlinked { id });
    }
}

/// The browser's storage quota for the page, in MB, as [`Event::Estimated`].
pub(crate) fn estimate(s: &Rc<Shared>) {
    let made = call(&get(&global("navigator"), "storage"), "estimate", &[]);
    then(s, &made, 0, |s, _, e| {
        let quota = get(e, "quota").as_f64().unwrap_or(0.0) / 1e6;
        dispatch(s, Event::Estimated { quota_mb: quota.min(4e9) as u32 });
    });
}
