//! Telemetry: what the page saw lately, and reports to compusophy's inbox ([`URL`],
//! api/feedback.mjs, which files each as a GitHub issue).
//!
//! - **Notes**: short lines about what happened (`ai 503 network`, `proc 7 hello failed`,
//!   `feedback idea`), the last [`NOTES`] kept; a report carries the last [`SHOWN`].
//! - **Reports**: feedback a person sends ([`Reports::feedback`]); and, unless the [`REPORTS`]
//!   preference is `"off"`, an error the page saw ([`Reports::failed`]: a program that would not
//!   run, the AI's server failing or out of reach), once a session per signature ([`sig`]), and
//!   a panic, by beacon ([`install`]). Each is JSON, `{kind, title, body, sig}`: the body is the
//!   text and a context block (build, browser, screen, touch, theme, windows by [`app`], notes),
//!   never a file, a prompt or anything typed but the feedback itself; feedback's sig is its own
//!   id, made once, so the inbox can tell a retry from new feedback.
//! - **Outbox**: every report waits in `localStorage` ([`OUTBOX`], at most [`KEEP`], oldest
//!   dropped) until a 2xx; it is sent at once, and again after boot and with each new report.
//!   A 400 or 413 drops it (it will never go); anything else (503 when the inbox is not set up,
//!   429, no network) keeps it. While reports are off, only feedback waits there.

use platform::{Ctl, Device};
use std::cell::RefCell;

/// The inbox, and the `localStorage` keys of the outbox (reports as JSON, a line each) and of
/// the reports preference.
pub const URL: &str = "/api/feedback";
pub const OUTBOX: &str = "compusophy.outbox";
pub const REPORTS: &str = "compusophy.reports";
/// The build: `COMPUSOPHY_BUILD` as scripts/build-web.sh exports it (the commit, `-dirty` if the
/// tree was), else `dev`.
pub const BUILD: &str = match option_env!("COMPUSOPHY_BUILD") {
    Some(b) => b,
    None => "dev",
};
/// Notes kept, notes a report shows, a note's longest (bytes), reports the outbox keeps, a
/// title's longest (chars), a body's longest (bytes).
pub const NOTES: usize = 100;
pub const SHOWN: usize = 50;
pub const NOTE_MAX: usize = 120;
pub const KEEP: usize = 20;
pub const TITLE_MAX: usize = 80;
pub const BODY_MAX: usize = 24 << 10;
/// Report streams' ids, clear of the AI's (which count up from 1).
const FIRST_ID: u32 = 1 << 31;

thread_local! {
    /// The notes, oldest first: outside the desktop, so a panic can still read them.
    static RING: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Adds `new` to the notes (if any; the oldest beyond [`NOTES`] goes), then gives the last `n`,
/// oldest first, a line each. One function, so the ring's access is built once. None while the
/// ring is busy (a panic in it) or gone (`try_with`: `with`'s panic formats its error).
fn ring(new: Option<String>, n: usize) -> String {
    let notes = RING.try_with(|r| {
        let Ok(mut r) = r.try_borrow_mut() else { return String::new() };
        if let Some(s) = new {
            if r.len() >= NOTES {
                r.remove(0);
            }
            r.push(s);
        }
        let mut out = String::new();
        for s in &r[r.len().saturating_sub(n)..] {
            out = out + s + "\n";
        }
        out
    });
    notes.unwrap_or_default()
}

/// Notes `text`, control chars as spaces, cut to [`NOTE_MAX`] bytes.
pub fn note(text: &str) {
    let mut s = String::new();
    for c in text.chars() {
        if s.len() + c.len_utf8() > NOTE_MAX {
            break;
        }
        s.push(if c.is_control() { ' ' } else { c });
    }
    ring(Some(s), 0);
}

/// The last `n` notes, oldest first, each ending in `\n`.
pub fn notes(n: usize) -> String {
    ring(None, n)
}

/// The parts of `s` between each byte `at` (a char pattern costs boot bytes).
fn split(s: &str, at: u8) -> Vec<&str> {
    let (mut out, mut start) = (Vec::new(), 0);
    for (i, b) in s.bytes().enumerate() {
        if b == at {
            out.push(s.get(start..i).unwrap_or_default());
            start = i + 1;
        }
    }
    out.push(s.get(start..).unwrap_or_default());
    out
}

/// The window or favorite `name` as a report says it: its app, never a file it shows (a file
/// open in Studio, `studio:<path>`, is `studio`); an app that is a file (a `.app`) is `app`.
pub fn app(name: &str) -> &str {
    let head = split(name, b':')[0];
    if head.bytes().any(|b| b == b'/' || b == b'.') { "app" } else { head }
}

/// Where a report came from: the device, its screen (CSS width, height, pixel ratio; zero if
/// unknown), the theme, the windows open.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Context {
    pub device: Device,
    pub screen: (f32, f32, f32),
    pub theme: String,
    pub windows: String,
}

/// A short signature of a report's kind and message: FNV-1a, 8 hex digits. The inbox files one
/// issue per signature; the page sends each once a session.
pub fn sig(kind: &str, message: &str) -> String {
    let mut h: u32 = 2_166_136_261;
    for b in [kind, "\n", message].iter().flat_map(|s| s.bytes()) {
        h = (h ^ u32::from(b)).wrapping_mul(16_777_619);
    }
    let mut out = String::new();
    for i in (0..8).rev() {
        out.push(hex(h >> (4 * i)));
    }
    out
}

/// The hex digit of `n`'s low four bits.
fn hex(n: u32) -> char {
    char::from(b"0123456789abcdef"[n as usize & 15])
}

/// A title: `text`'s first non-empty line, at most [`TITLE_MAX`] chars (an ellipsis if cut).
pub fn title(text: &str) -> String {
    let mut line = "";
    for l in &split(text, b'\n') {
        line = l.trim();
        if !line.is_empty() {
            break;
        }
    }
    match line.char_indices().nth(TITLE_MAX - 1).and_then(|(cut, _)| line.split_at_checked(cut)) {
        Some((head, tail)) if tail.chars().nth(1).is_some() => {
            [head.trim_end(), "\u{2026}"].concat()
        }
        _ => line.to_string(),
    }
}

/// A report's body: `text`, then a context block: the build, and with `ctx` the device, theme,
/// windows and the last [`SHOWN`] notes. At most [`BODY_MAX`] bytes, cut in `text`.
pub fn body(text: &str, ctx: Option<&Context>) -> String {
    let mut text = text.trim();
    if text.len() > BODY_MAX / 2 {
        let cut = (0..=BODY_MAX / 2).rev().find(|&i| text.is_char_boundary(i)).unwrap_or(0);
        text = text.get(..cut).unwrap_or_default();
    }
    let mut out = [text, "\n\n```text\nbuild    ", BUILD].concat();
    let Some(c) = ctx else { return out + "\n```" };
    let (d, (w, h, dpr)) = (&c.device, c.screen);
    let mut screen = String::new();
    push(&mut screen, w.round());
    screen.push('x');
    push(&mut screen, h.round());
    screen.push_str(" @");
    push(&mut screen, dpr);
    let touch = if d.touch { "yes" } else { "no" };
    let windows = if c.windows.is_empty() { "none" } else { &c.windows };
    for (k, v) in [("agent", d.agent.as_str()), ("screen", &screen), ("touch", touch)] {
        out = out + "\n" + k + "         ".get(k.len()..).unwrap_or_default() + v;
    }
    out = out + "\ntheme    " + &c.theme + "\nwindows  " + windows + "\n```";
    out + "\n\nRecent events, oldest first:\n```text\n" + &notes(SHOWN) + "```"
}

/// Appends `v` to two decimals, trailing zeros dropped (no float formatting).
fn push(out: &mut String, v: f32) {
    // Constant bounds: no panic path. NaN becomes 0.
    let c = (v.clamp(0.0, 1e7) * 100.0).round() as usize;
    ui::push_num(out, c / 100);
    if c % 100 != 0 {
        out.push('.');
        ui::push_num(out, c % 100 / 10);
        if c % 10 != 0 {
            ui::push_num(out, c % 10);
        }
    }
}

/// A report as the inbox takes it: `{"kind":..,"title":..,"body":..,"sig":..}`.
pub fn report(kind: &str, title: &str, body: &str, sig: &str) -> String {
    let mut out = String::from("{");
    for (i, (k, v)) in
        [("kind", kind), ("title", title), ("body", body), ("sig", sig)].iter().enumerate()
    {
        if i > 0 {
            out.push(',');
        }
        json(&mut out, k);
        out.push(':');
        json(&mut out, v);
    }
    out + "}"
}

/// Appends `s` as a JSON string.
fn json(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => {
                out.push_str("\\u00");
                out.push(hex(c as u32 >> 4));
                out.push(hex(c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// The page's reports: the outbox, the ones in flight, what was sent this session, what apps were
/// told, and what waits for the next [`Reports::pump`] (which can say what is open).
#[derive(Debug, Default)]
pub struct Reports {
    /// The screen as the page last measured it, for [`Context::screen`].
    pub screen: (f32, f32, f32),
    loaded: bool,
    outbox: Vec<String>,
    flying: Vec<(u32, String)>,
    last: u32,
    sent: Vec<String>,
    /// Automatic reports are off; the last report did not get through.
    off: bool,
    held: bool,
    told: Option<(bool, bool)>,
    /// Feedback and errors to build and send, and how many were built ([`Reports::id`]).
    asked: Vec<Asked>,
    made: u32,
}

/// A report to build at the next pump: its kind, title, text, whether the context goes with it,
/// and its signature (none for feedback, which gets its own id when built).
#[derive(Debug)]
struct Asked {
    kind: &'static str,
    title: String,
    text: String,
    context: bool,
    sig: String,
}

impl Reports {
    /// After every event and frame: once, loads the outbox and the preference and sends what
    /// waits; then builds and sends what was asked, with the context `ctx` gives.
    pub fn pump(&mut self, ctl: &mut Ctl, ctx: impl FnOnce() -> Context) {
        if !std::mem::replace(&mut self.loaded, true) {
            let stored = ctl.storage_get(OUTBOX).unwrap_or_default();
            for r in split(&stored, b'\n').into_iter().filter(|l| !l.is_empty()) {
                self.outbox.push(r.to_string());
            }
            self.off = ctl.storage_get(REPORTS).as_deref() == Some("off");
            self.held = !self.outbox.is_empty();
            if self.off {
                self.drop_automatic(ctl);
            }
            note(&["boot ", BUILD].concat());
            self.send(ctl);
        }
        if self.asked.is_empty() {
            return;
        }
        let (mut c, asked) = (ctx(), std::mem::take(&mut self.asked));
        c.screen = self.screen;
        for a in asked {
            let body = body(&a.text, a.context.then_some(&c));
            let sig = if a.sig.is_empty() { self.id(ctl, &body) } else { a.sig };
            self.outbox.push(report(a.kind, &a.title, &body, &sig));
        }
        while self.outbox.len() > KEEP {
            self.outbox.remove(0);
        }
        self.store(ctl);
        self.send(ctl);
    }

    /// The person sent feedback: `kind` and `text`, with the context if `context`.
    pub fn feedback(&mut self, kind: &str, text: &str, context: bool) {
        note(&["feedback ", kind].concat());
        let mut head = kind.to_string();
        if let Some(first) = head.get_mut(..1) {
            first.make_ascii_uppercase();
        }
        let (title, text) = (title(&[&head, ": ", text.trim()].concat()), text.to_string());
        self.asked.push(Asked { kind: "feedback", title, text, context, sig: String::new() });
    }

    /// Something failed: noted as `note_as`; reported as `kind` ("error" or "panic") with
    /// `message` unless reports are off or this signature went already this session.
    pub fn failed(&mut self, kind: &'static str, message: &str, note_as: &str) {
        note(note_as);
        let sig = sig(kind, message);
        if !self.off && !self.sent.contains(&sig) {
            self.sent.push(sig.clone());
            let (title, text) = (title(message), message.to_string());
            self.asked.push(Asked { kind, title, text, context: true, sig });
        }
    }

    /// An AI stream ended with `status` (0, and `error` `"network"`, if no answer came): anything
    /// but a 2xx is noted; a 5xx or no answer is reported.
    pub fn ai_ended(&mut self, status: u16, error: &str) {
        let mut code = String::new();
        ui::push_num(&mut code, status.into());
        let n = ["ai ", &code, " ", error].concat();
        match status {
            200..300 => {}
            0 => self.failed("error", "The AI request got no answer", n.trim_end()),
            500.. => {
                let message = ["The AI request failed: HTTP ", &code].concat();
                self.failed("error", &message, n.trim_end());
            }
            _ => note(n.trim_end()),
        }
    }

    /// The worker of process `pid`, running `argv0`, failed to load or threw.
    pub fn proc_failed(&mut self, pid: u32, argv0: &str) {
        let name = argv0.get(argv0.bytes().rposition(|b| b == b'/').map_or(0, |i| i + 1)..);
        let name = name.unwrap_or_default();
        let mut n = String::from("proc ");
        ui::push_num(&mut n, pid as usize);
        let message = ["The program ", name, " failed to load or run"].concat();
        self.failed("error", &message, &[&n, " ", name, " failed"].concat());
    }

    /// The status apps see, `base` with whether automatic reports are off and one is held.
    pub fn status(&self, base: ui::AiStatus) -> ui::AiStatus {
        ui::AiStatus { reports_off: self.off, held: self.held, ..base }
    }

    /// A preference an app set: the reports switch is stored and followed (off, the automatic
    /// reports still held are dropped); each is noted, the dock's favorites and the home
    /// screen's order by their [`app`].
    pub fn pref(&mut self, ctl: &mut Ctl, key: &str, value: &str) {
        let mut said = value.to_string();
        if key == "dock" || key == "home.order" {
            said = split(value, b',').into_iter().map(app).collect::<Vec<_>>().join(",");
        }
        note(&["pref ", key, " ", &said].concat());
        if key == ui::REPORTS {
            self.off = value == "off";
            ctl.storage_set(REPORTS, value);
            if self.off {
                self.drop_automatic(ctl);
            }
        }
    }

    /// Reports are off: the error reports waiting to be built or sent are dropped (a report's kind
    /// is its first field); feedback stays.
    fn drop_automatic(&mut self, ctl: &mut Ctl) {
        let n = self.outbox.len();
        self.outbox.retain(|r| r.starts_with(r#"{"kind":"feedback""#));
        self.asked.retain(|a| a.kind == "feedback");
        if self.outbox.len() != n {
            self.store(ctl);
        }
        self.held &= !self.outbox.is_empty();
    }

    /// A new report's own id, which its retries keep, so the inbox files it once: FNV-1a (64
    /// bits, 16 hex digits) of the moment (page clock, local time, a count) and its `body`.
    fn id(&mut self, ctl: &Ctl, body: &str) -> String {
        self.made = self.made.wrapping_add(1);
        let t = ctl.local_time();
        let mut seed = ctl.monotonic_ms().to_bits().to_le_bytes().to_vec();
        seed.extend(self.made.to_le_bytes().into_iter().chain(t.year.to_le_bytes()));
        seed.extend([t.month, t.day, t.hour, t.minute]);
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in seed.into_iter().chain(body.bytes()) {
            h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
        }
        (0..16).rev().map(|i| hex((h >> (4 * i)) as u32)).collect()
    }

    /// Stream `id` ended with `status`; whether it was a report's. Delivered (or refused for
    /// good: 400, 413), it leaves the outbox; else it stays for the next try.
    pub fn ended(&mut self, ctl: &mut Ctl, id: u32, status: u16) -> bool {
        let Some(i) = self.flying.iter().position(|f| f.0 == id) else { return false };
        let (_, body) = self.flying.remove(i);
        let done = (200..300).contains(&status);
        if done || status == 400 || status == 413 {
            if let Some(k) = self.outbox.iter().position(|b| *b == body) {
                self.outbox.remove(k);
                self.store(ctl);
            }
        }
        if !done {
            let mut n = String::from("report ");
            ui::push_num(&mut n, status.into());
            note(&n);
        }
        self.held = !done && status != 400 && status != 413;
        true
    }

    /// Whether apps should hear [`Reports::status`] again: it changed since they last did.
    pub fn retell(&mut self) -> bool {
        let now = (self.off, self.held);
        self.told.replace(now) != Some(now)
    }

    /// The reports waiting to go, oldest first.
    pub fn outbox(&self) -> &[String] {
        &self.outbox
    }

    fn store(&self, ctl: &mut Ctl) {
        let mut all = String::new();
        for r in &self.outbox {
            all = all + r + "\n";
        }
        ctl.storage_set(OUTBOX, all.trim_end_matches('\n'));
    }

    /// Sends every report in the outbox not already on its way.
    fn send(&mut self, ctl: &mut Ctl) {
        for r in &self.outbox {
            if !self.flying.iter().any(|f| f.1 == *r) {
                self.last = self.last.wrapping_add(1);
                let id = FIRST_ID | self.last;
                let json = vec![("Content-Type", "application/json".to_string())];
                ctl.stream(id, URL, json, r.as_bytes().to_vec());
                self.flying.push((id, r.clone()));
            }
        }
    }
}

/// Reports each panic by beacon (the module traps right after it: no fetch can finish), unless
/// reports are off: its message and place, and the context the page can still read.
pub fn install() {
    std::panic::set_hook(Box::new(|info| {
        // Its message and place, without the formatting machinery.
        let mut at = String::new();
        if let Some(l) = info.location() {
            at = [l.file(), ":"].concat();
            ui::push_num(&mut at, l.line() as usize);
        }
        let p = info.payload();
        let said = p.downcast_ref::<String>().map(String::as_str);
        let said = said.or_else(|| p.downcast_ref::<&str>().copied()).unwrap_or("panic");
        let text = [said, "\nat ", &at].concat();
        note(&["panic ", &text].concat());
        if Ctl::default().storage_get(REPORTS).as_deref() == Some("off") {
            return;
        }
        let ctx = Context { device: platform::device(), ..Context::default() };
        let json = report("panic", &title(&text), &body(&text, Some(&ctx)), &sig("panic", &text));
        platform::beacon(URL, &json);
    }));
}

#[cfg(test)]
mod tests;
