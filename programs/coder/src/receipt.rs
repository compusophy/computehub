//! Receipts: what a make cost, at the free AI's prices, and its line in `~/.ai/makes.jsonl`, the
//! local record of every make (the overlay's sign that one ended). Nothing leaves the device.

use crate::ai::{num, put_num};
use crate::json::put;
use crate::{Done, Outcome};

/// One line per make that ended, of any outcome.
pub const MAKES: &str = concat!("/home/", "guest", "/.ai/makes.jsonl");
/// Past this many bytes, [`rotate`] keeps the newest half.
pub const MAX_MAKES: usize = 256 * 1024;

/// `model`'s list prices in nano-dollars a token, in and out, as `api/ai.mjs` has them (a test
/// reads its `MODELS` line; a model it does not list costs as its first).
pub fn rates(model: &str) -> (u64, u64) {
    match model {
        "zai/glm-5.3-flash" => (150, 500),
        _ => (1400, 4400),
    }
}

/// What `u` costs `model`, in micro-dollars: its [`rates`], cached tokens in at 0.186 of the
/// price in (GLM 5.3: $0.26 a million against $1.40, as the gateway charged it, measured to the
/// micro-dollar).
pub fn price(model: &str, u: &crate::json::Usage) -> u32 {
    let (pin, pout) = rates(model);
    let cached = u64::from(u.cached.min(u.input));
    let nano = (u64::from(u.input) - cached) * pin
        + cached * pin * 186 / 1000
        + u64::from(u.output) * pout;
    (nano / 1000) as u32
}

/// `micros` as dollars with 4 decimals, rounded: `0.0262`.
pub fn usd(micros: u32) -> String {
    let n = (micros + 50) / 100;
    [&num((n / 10_000).into()), ".", &num((n % 10_000 + 10_000).into())[1..]].concat()
}

/// The makes.jsonl line for `done`, a make on the file at `path` ("" if it has none), which
/// installed version `version` (0: none) of `lines` lines.
pub fn line(path: &str, version: u32, lines: usize, done: &Done) -> String {
    let r = &done.receipt;
    let outcome = match done.outcome {
        Outcome::Ready => "ready",
        Outcome::Faulting => "faulting",
        Outcome::Broken => "broken",
        Outcome::Cant => "cant",
        Outcome::Stopped => "stopped",
        Outcome::Failed => "failed",
    };
    let fault = if done.outcome == Outcome::Ready { "" } else { done.why.as_str() };
    let mut o = String::from("{\"v\":1,\"path\":");
    put(&mut o, path);
    o += if done.change { ",\"kind\":\"change" } else { ",\"kind\":\"make" };
    o += "\",\"outcome\":\"";
    o += outcome;
    o += "\",\"version\":";
    put_num(&mut o, version.into());
    o += ",\"lines\":";
    put_num(&mut o, lines as u64);
    o += ",\"plan\":";
    put(&mut o, &done.plan);
    o += ",\"requests\":";
    put_num(&mut o, r.turns.len() as u64);
    o += ",\"usd\":";
    o += &usd(r.usd_micros);
    o += if r.est { ",\"est\":true,\"ms\":" } else { ",\"est\":false,\"ms\":" };
    put_num(&mut o, r.ms);
    o += ",\"fault\":";
    put(&mut o, fault);
    o += "}\n";
    o
}

/// `text` (the makes.jsonl file) kept to its newest half, from a line's start, once it is over
/// [`MAX_MAKES`] bytes; `None` while it is not.
pub fn rotate(text: &str) -> Option<String> {
    let from = text.len().checked_sub(MAX_MAKES / 2).filter(|_| text.len() > MAX_MAKES)?;
    let at = text.as_bytes()[from..].iter().position(|&b| b == b'\n')?;
    Some(text[from + at + 1..].into())
}
