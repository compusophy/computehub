//! What reports need of the page beyond [`crate::Ctl`]: the device, and a last word as it goes
//! down. Plain `Reflect` reads rather than web-sys's `Navigator`: less glue in the boot download.

use js_sys::{Function, Reflect};
use wasm_bindgen::{JsCast, JsValue};

/// The device a report describes: `navigator.userAgent`, and whether it has a touch screen
/// (`maxTouchPoints`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Device {
    pub agent: String,
    pub touch: bool,
}

/// `navigator[key]`, or undefined.
fn navigator(key: &str) -> (JsValue, JsValue) {
    let nav = crate::window().and_then(|w| Reflect::get(&w, &"navigator".into()).ok());
    let nav = nav.unwrap_or_default();
    let value = Reflect::get(&nav, &key.into()).unwrap_or_default();
    (nav, value)
}

/// The device now; natively, a blank one.
pub fn device() -> Device {
    if !cfg!(target_arch = "wasm32") {
        return Device::default();
    }
    let agent = navigator("userAgent").1.as_string().unwrap_or_default();
    Device { agent, touch: navigator("maxTouchPoints").1.as_f64().is_some_and(|n| n > 0.0) }
}

/// `navigator.sendBeacon(url, body)`: the browser POSTs `body` (as text) to `url` (any: the caller
/// vouches for it) even while the page goes down, as after a panic; whether it took it. Natively
/// false.
pub fn beacon(url: &str, body: &str) -> bool {
    if !cfg!(target_arch = "wasm32") {
        return false;
    }
    let (nav, send) = navigator("sendBeacon");
    let send = send.dyn_into::<Function>().ok();
    send.and_then(|f| f.call2(&nav, &url.into(), &body.into()).ok()).is_some_and(|v| v.is_truthy())
}
