//! What the teacher costs, in whole nano-dollars (no floats): each reply's usage priced by the
//! model that ran each attempt, a batch's at half; a ledger line per reply, appended as it
//! comes; a budget that refuses a request whose worst case would pass it; and the ledger summed
//! by day and command (`teach cost`).

use std::collections::BTreeMap;

use coder::json::{Json, put};

use crate::claude::{Msg, Req, Usage};
use crate::{Fail, append};

/// `model`'s list prices in nano-dollars a token: in, out, cache read, cache write for 5 minutes
/// and for an hour. Claude Opus 5.5's: $4, $20, $0.20, $5 and $8 a million. A fallback's
/// (Claude Opus 5, Opus 4.8), and any model not known, at Opus 5's: $5, $25, $0.50, $6.25 and
/// $10, the dearer, so a ledger never reads low.
pub fn rates(model: &str) -> [u64; 5] {
    match model {
        "claude-opus-5-5" => [4_000, 20_000, 200, 5_000, 8_000],
        _ => [5_000, 25_000, 500, 6_250, 10_000],
    }
}

/// What `u` costs on `model`; in a batch, half (the discount applies to every kind of token).
pub fn nanos(model: &str, u: &Usage, batch: bool) -> u64 {
    let r = rates(model);
    let n = u.input * r[0]
        + u.output * r[1]
        + u.cache_read * r[2]
        + u.write_5m * r[3]
        + u.write_1h * r[4];
    if batch { n / 2 } else { n }
}

/// What the reply `m` cost: each attempt at its model's prices.
pub fn msg_nanos(m: &Msg, batch: bool) -> u64 {
    m.parts.iter().map(|(model, u)| nanos(model, u, batch)).sum()
}

/// The most `req` can cost on `model`: its input uncached at a token per 3 bytes (an
/// overcount) and all of its `max_tokens` out.
pub fn worst(req: &Req, model: &str, batch: bool) -> u64 {
    let input = ((req.system.len() + req.user.len()) / 3) as u64;
    nanos(model, &Usage { input, output: req.max_tokens.into(), ..Usage::default() }, batch)
}

/// `n` nano-dollars as dollars to the micro-dollar, rounded down: `0.012345`.
pub fn usd(n: u64) -> String {
    format!("{}.{:06}", n / 1_000_000_000, n % 1_000_000_000 / 1000)
}

/// Dollars as typed (`12`, `0.5`, `3.25`, up to 9 decimals) in nano-dollars.
pub fn parse_usd(s: &str) -> Option<u64> {
    let (whole, frac) = s.split_once('.').unwrap_or((s, ""));
    let digits = |t: &str| t.len() <= 9 && t.bytes().all(|b| b.is_ascii_digit());
    if (whole.is_empty() && frac.is_empty()) || !digits(whole) || !digits(frac) {
        return None;
    }
    let w: u64 = if whole.is_empty() { 0 } else { whole.parse().ok()? };
    Some(w * 1_000_000_000 + format!("{frac:0<9}").parse::<u64>().ok()?)
}

/// The ledger of a run of `cmd`: where its lines go, the day they say, the budget (nano-dollars;
/// none: no cap) and what the run has spent.
#[derive(Clone, Debug)]
pub struct Ledger {
    pub path: String,
    pub cmd: String,
    pub day: String,
    pub cap: Option<u64>,
    pub spent: u64,
}

impl Ledger {
    /// Whether `more` nano-dollars still fit the budget.
    pub fn fits(&self, more: u64) -> bool {
        self.cap.is_none_or(|c| self.spent + more <= c)
    }

    /// Notes the reply `m` to the request `id`: its line appended, its cost spent.
    pub fn note(&mut self, id: &str, m: &Msg, batch: bool) -> Result<(), Fail> {
        let n = msg_nanos(m, batch);
        self.spent += n;
        let u = m.parts.iter().fold(Usage::default(), |a, (_, u)| Usage {
            input: a.input + u.input,
            output: a.output + u.output,
            cache_read: a.cache_read + u.cache_read,
            write_5m: a.write_5m + u.write_5m,
            write_1h: a.write_1h + u.write_1h,
        });
        let mut o = String::new();
        let texts = [("day", self.day.as_str()), ("cmd", &self.cmd), ("model", &m.model)];
        for (i, (k, v)) in
            texts.into_iter().chain([("id", id), ("msg", &m.id), ("stop", &m.stop)]).enumerate()
        {
            o += if i == 0 { "{\"" } else { ",\"" };
            o += k;
            o += "\":";
            put(&mut o, v);
        }
        o += if batch { ",\"batch\":true" } else { ",\"batch\":false" };
        let tokens = [u.input, u.output, u.cache_read, u.write_5m + u.write_1h, n];
        for (k, v) in ["in", "out", "cache_read", "cache_write", "nanos"].iter().zip(tokens) {
            o += &format!(",\"{k}\":{v}");
        }
        o += &format!(",\"usd\":\"{}\"}}\n", usd(n));
        append(&self.path, &o)
    }
}

/// The ledger `text` summed by day and command: calls, tokens and dollars, then the total.
pub fn summary(text: &str) -> String {
    let mut rows: BTreeMap<(String, String), [u64; 6]> = BTreeMap::new();
    for j in text.lines().filter_map(Json::parse) {
        let s = |k: &str| j.get(k).and_then(Json::text).unwrap_or("?").to_string();
        let n = |k: &str| j.get(k).and_then(Json::text).and_then(|t| t.parse().ok()).unwrap_or(0);
        let row = rows.entry((s("day"), s("cmd"))).or_default();
        let add = [1, n("in"), n("out"), n("cache_read"), n("cache_write"), n("nanos")];
        row.iter_mut().zip(add).for_each(|(a, b)| *a += b);
    }
    let line = |day: &str, cmd: &str, r: &[u64; 6]| {
        let usd = usd(r[5]);
        format!(
            "{day:<10}  {cmd:<6} {:>6} {:>11} {:>11} {:>11} {:>11} {usd:>12}\n",
            r[0], r[1], r[2], r[3], r[4]
        )
    };
    let mut out = format!(
        "{:<10}  {:<6} {:>6} {:>11} {:>11} {:>11} {:>11} {:>12}\n",
        "day", "cmd", "calls", "in", "out", "cache_read", "cache_write", "usd"
    );
    let mut total = [0; 6];
    for ((day, cmd), r) in &rows {
        out += &line(day, cmd, r);
        total.iter_mut().zip(r).for_each(|(a, b)| *a += b);
    }
    out + &line("total", "", &total)
}
