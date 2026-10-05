//! compusophyOS's wasm entry: `start` runs a desktop on [`platform::run`] with the boot font, a
//! [`Vfs`] holding the `/bin` markers and Studio's samples, and a [`Registry`] of
//! [`remote::open`] (the GUI programs: the Terminal, About, Feedback, Files, Welcome, Studio, the
//! Assistant, `.app` files). A new tab's first size shows the welcome ([`logon`]: the mark, the
//! record of this start, sign-in); a reload of a signed-in tab goes straight on. The [`Shell`]
//! (and /home put back) waits for a sign-in and a size that leaves a work area; until then
//! input is the welcome's or dropped (a missed Tick is replayed) and frames clear to its theme's
//! base. The welcome marks the device `seen` as it signs in, so the shell opens no Welcome.
//! The theme is kept in `localStorage` ([`THEME_KEY`]), as are the preferences of [`PREFS`]
//! (`compusophy.<key>`), which apps and the shell set ([`shell::Effect::Pref`]) and the shell
//! reads when it is made ([`shell::Prefs`]): each under the signed-in profile's key
//! ([`logon::own`]; these are the first profile's). Sign out keeps /home and reloads; a reset
//! (Settings', the person's own) erases every key the device keeps ([`ALL`]) and reloads.
//!
//! Fonts: boot (Inter Regular, in the wasm); deferred (Inter SemiBold and JetBrains Mono, fetched
//! after the first frame under the top two fetch ids, which the shell never reaches; a failure
//! leaves bold as Regular and mono cells empty); lazy (symbol fallbacks the shell fetches). A
//! key-down that types text is never prevented while text input is on: text reaches apps only
//! through the platform's textarea.
//!
//! The meters Activity watches ([`uiwire::stat`]): each frame is counted under its cause (the
//! person's input, motion, programs, the watcher's own, a timer's such as the grain's, else
//! other), the time spent in events and frames adds up, and while a watcher is set a sample is
//! taken as the desktop flushes, when its [`stat::Pace`] says (by the one-shot timer, at most
//! once a second), and posted to it if it changed. Nothing samples, wakes or posts on a still
//! desktop.

#![forbid(unsafe_code)]

use std::mem;

pub mod ai;
pub mod home;
pub mod remote;
/// Telemetry: notes, feedback and error reports, the outbox (its own crate).
pub use ::report;

use gfx::{DrawList, Rgba};
use logon::profiles::{LIST, set_face};
use logon::record::{self, Record};
use logon::{Logon, Out};
use platform::{App, Ctl, Event, Handled, Renderer};
use shell::{Effect, Input, KernelIn, Key, LocalTime, Mods, Registry, Response, Shell};
use ui::kernel::{self, Effect as K};
use ui::{FontId, TextSystem};
use uiwire::stat;
use vfs::Vfs;
use wasm_bindgen::prelude::*;

const SANS: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");

/// The deferred fonts as (fetch id, slot, URL).
const DEFERRED: [(u32, FontId, &str); 2] = [
    (u32::MAX - 1, FontId::SansBold, "fonts/deferred/Inter-SemiBold.ttf"),
    (u32::MAX, FontId::Mono, "fonts/deferred/JetBrainsMono-Regular.ttf"),
];

/// The `localStorage` key of the theme's name, and what every key the desktop keeps starts with.
pub const THEME_KEY: &str = "compusophy.theme";
pub const ALL: &str = "compusophy.";
/// The preferences kept as `compusophy.<key>`: the AI model (which the AI hub keeps, see [`ai`]),
/// the dock's favorites (registry names, comma-separated), `"1"` once Welcome was shown on a
/// first visit, `"off"` to stop automatic error reports, the home screen's order (as the dock's),
/// `"off"` to still the grain and the folders' apps. Other keys are dropped.
pub const PREFS: [&str; 7] =
    [ui::AI_MODEL, "dock", "seen", "reports", "home.order", ui::GRAIN, "folders"];
/// The applets of `bin/toolbox.wasm`, each a `/bin` marker file (as are the GUI programs:
/// [`remote::STUDIO`] for `bin/studio.wasm`, and those of [`remote::SYSTEM`] for one
/// `bin/system.wasm`).
const APPLETS: [&str; 9] =
    ["hello", "rev", "wc", "spin", "nap", "fstest", "keys", "bench", "selftest"];

// The wasm entry point. A plain comment: a doc comment would ship in os.js.
#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    report::install();
    platform::run(Desktop::new()?)
}

#[derive(Default)]
struct Desktop {
    /// The text system and filesystem, until the shell takes them.
    parts: Parts,
    shell: Option<Shell>,
    /// The welcome while it shows (and flies off the desktop), the start's record, the profile
    /// signed in to, and when the welcome wants its next frame.
    logon: Option<Logon>,
    record: Record,
    signed: Option<u32>,
    wish: Option<u32>,
    /// Whether a Tick came before the shell did, and whether text input is on.
    missed_tick: bool,
    typing: bool,
    /// Which [`DEFERRED`] fonts are on their way; `None` before the first frame.
    deferred: Option<[bool; 2]>,
    /// The theme's name as storage has it, the AI state the program windows share, and when
    /// the one-shot timer fires (page clock ms; past, or once it fired: unarmed).
    saved: &'static str,
    ai: ai::Ai,
    wake: f64,
    /// Notes, feedback and error reports, and the outbox; /home as kept.
    report: report::Reports,
    home: home::Home,
    list: DrawList,
    /// Why the next frame is drawn (bit `i` the cause frames are counted under at `i`: input,
    /// motion, programs, the watcher's own, a timer's (the grain's, an app's); none, other), the
    /// frames by cause, the ms spent in events and frames, the watcher as last seen, and when to
    /// sample the meters.
    why: u8,
    watcher: Option<u32>,
    frames: [u32; 6],
    busy: f64,
    pace: stat::Pace,
}

/// The bits of [`Desktop::why`].
const INPUT: u8 = 1;
const MOTION: u8 = 2;
const PROGRAMS: u8 = 4;
const SELF: u8 = 8;
const TIMER: u8 = 16;

impl Desktop {
    fn new() -> Result<Desktop, String> {
        let text = TextSystem::new(SANS.to_vec())?;
        let mut vfs = Vfs::new();
        let _ = vfs.mkdir_all("/bin"); // Not mkdir: Vfs::new ships mkdir_all already.
        for name in APPLETS {
            let _ = vfs.write(&["/bin/", name].concat(), b"#!wasm bin/toolbox.wasm\n");
        }
        let _ = vfs.write(apps::SHELL, b"#!wasm bin/sh.wasm\n");
        let _ = vfs.write(remote::TERMINAL, b"#!wasm bin/terminal.wasm\n");
        let _ = vfs.write(remote::STUDIO, b"#!wasm bin/studio.wasm\n");
        let _ = vfs.write(remote::ASSISTANT, b"#!wasm bin/assistant.wasm\n");
        for (name, ..) in remote::SYSTEM {
            let _ = vfs.write(&["/bin/", name].concat(), b"#!wasm bin/system.wasm\n");
        }
        Ok(Desktop { parts: Some((text, vfs)), ..Desktop::default() })
    }

    /// Hands `input` to the welcome while it shows, else to the shell; at the first usable size
    /// makes the one (a new tab) or the other (a reload of a signed-in tab, [`logon::SESSION`]).
    fn input(&mut self, input: Input, ctl: &mut Ctl) -> Option<Response> {
        self.missed_tick |= matches!(input, Input::Tick { .. });
        if let Some(l) = self.logon.as_mut().filter(|l| !l.leaving()) {
            ctl.random(&mut l.fresh);
            let (r, outs) = l.input(&input, ctl.monotonic_ms(), &|k| ctl.storage_get(k));
            return Some(self.outs(outs, r, ctl));
        }
        if let Some(shell) = &mut self.shell {
            return Some(shell.input(input));
        }
        let Input::Resize { w, h } = input else { return None };
        if self.signed.is_none() && w >= 1.0 && h >= 1.0 {
            let get = |k: &str| ctl.storage_get(k);
            self.signed = logon::session(ctl.session_get(logon::SESSION).as_deref(), &get);
            if self.signed.is_none() {
                let (l, outs) = Logon::new((w, h), &get, ctl.secure());
                self.logon = Some(l);
                return Some(self.outs(outs, Response { redraw: true, ..Default::default() }, ctl));
            }
        }
        self.desk(ctl)
    }

    /// Makes the signed-in profile's desktop once the screen leaves a work area: /home put back
    /// (noted with its KB and ms), the preferences, the shell (told the time if a Tick came).
    fn desk(&mut self, ctl: &mut Ctl) -> Option<Response> {
        let (w, h, _) = self.report.screen;
        if self.signed.is_none() || w < 1.0 || h < shell::BAR_H + shell::DOCK_CLEAR + 1.0 {
            return None;
        }
        logon::sign(self.signed?, ctl.storage_get(LIST).as_deref());
        let ((text, mut vfs), t) = (self.parts.take()?, ctl.monotonic_ms());
        self.home.restore(&mut vfs, ctl, &mut self.report);
        report::note(&record::home_note(self.home.kept_len(), ctl.monotonic_ms() - t));
        let prefs = prefs(ctl);
        let shell = Shell::new(w, h, text, vfs, registry(self.ai.clone()), prefs);
        let shell = self.shell.insert(shell);
        self.ai.load(ctl);
        self.saved = shell.theme_name();
        shell.set_now(ctl.monotonic_ms());
        let mut r = shell.input(Input::Resize { w, h });
        if self.missed_tick {
            r = merge(r, shell.input(Input::Tick { time: local(ctl.local_time()) }));
        }
        Some(r)
    }

    /// Carries out what the welcome asked, `r` its answer; signing in makes the desktop.
    fn outs(&mut self, outs: Vec<Out>, mut r: Response, ctl: &mut Ctl) -> Response {
        for out in outs {
            match out {
                Out::Set(key, value) => ctl.storage_set(&key, &value),
                Out::Remove(key) => ctl.storage_remove(&key),
                Out::Session(id) => ctl.session_set(logon::SESSION, Some(&id)),
                Out::Failed(message, note) => self.report.failed("error", message, note),
                Out::SignOut => self.sign_out(ctl, true),
                Out::Close => self.logon = None,
                Out::Numeric(on) => ctl.input_mode(on),
                Out::Derive { id, pin, salt, iterations } => ctl.derive(id, pin, salt, iterations),
                Out::SignIn(id) => {
                    self.signed = Some(id);
                    if let Some(d) = self.desk(ctl) {
                        r = merge(r, d);
                    }
                }
            }
        }
        r
    }

    /// Signs out: /home kept at once, then the tab forgets its profile and reloads (to the
    /// welcome). Files that could not be kept get a card over the desktop first, unless
    /// `anyway`.
    fn sign_out(&mut self, ctl: &mut Ctl, anyway: bool) {
        let Some(shell) = &self.shell else { return };
        self.home.keep(shell.vfs(), ctl, &mut self.report, true);
        if self.home.unkept && !anyway {
            let (w, h, _) = self.report.screen;
            self.logon = Some(Logon::unkept((w, h), shell.theme_name()));
            return;
        }
        ctl.session_set(logon::SESSION, None);
        ctl.reload();
    }

    /// One event, but for what the shell queued outside its response.
    fn handle(&mut self, ev: Event, ctl: &mut Ctl) -> Handled {
        if let Some(shell) = &mut self.shell {
            shell.set_now(ctl.monotonic_ms());
        }
        let types = types_text(&ev);
        if let Event::Resize { w, h, dpr } = ev {
            self.report.screen = (w, h, dpr);
        }
        let release =
            if let Event::PointerUp { x, y, button: 0 } = ev { Some((x, y)) } else { None };
        let r = match ev {
            Event::Key { down: false, ref code, .. } => return key_up(code),
            Event::Fetched { id, result } => match self.take_deferred(id) {
                Some(slot) => return self.set_font(slot, result, ctl),
                None => self.shell.as_mut().map(|s| s.fetched(id, result)),
            },
            ev @ (Event::Chunk { .. } | Event::StreamEnd { .. }) => {
                // A report's answer is the report's; an AI request's failure is noted.
                if let Event::StreamEnd { id, status, ref error } = ev {
                    if self.report.ended(ctl, id, status) {
                        return Handled::default();
                    }
                    if self.ai.streams(id) {
                        self.report.ai_ended(status, error);
                    }
                }
                if let Some(shell) = &mut self.shell {
                    self.ai.heard(shell.kernel_mut(), ev);
                }
                return Handled::default();
            }
            Event::Proc { pid, msg } => self.kernel(KernelIn::Msg { pid, msg }),
            Event::ProcError { pid } => {
                let procs = self.shell.as_mut().map(|s| s.kernel_mut().procs()).unwrap_or_default();
                let argv0 = procs.iter().find(|p| p.0 == pid).map_or("", |p| p.1.as_str());
                self.report.proc_failed(pid, argv0);
                self.kernel(KernelIn::Error { pid })
            }
            // The timer is spent, even if it fired a little early: what is still due arms it again.
            Event::Wake => {
                self.wake = f64::NEG_INFINITY;
                self.kernel(KernelIn::Wake)
            }
            Event::Hidden => self.kernel(KernelIn::Hidden),
            Event::Derived { id, result } => {
                let (now, get) = (ctl.monotonic_ms(), |k: &str| ctl.storage_get(k));
                let got = self.logon.as_mut().map(|l| l.derived(id, result, now, &get));
                got.map(|(r, outs)| self.outs(outs, r, ctl))
            }
            ev => input_of(ev).and_then(|input| self.input(input, ctl)),
        };
        let Some(r) = r else { return Handled::default() };
        let (asked, out) = (r.text_input.is_some(), r.effects.contains(&Effect::SignOut));
        let reset = r.effects.contains(&Effect::Reset);
        apply(r.effects, ctl, (&self.ai, &mut self.wake), &mut self.report);
        if reset {
            // All the device keeps (every profile's files, settings and PIN, the outbox, the
            // session) goes, to the welcome as a first visit: with no shell, nothing is kept.
            self.shell = None;
            ctl.storage_erase(ALL);
            ctl.reload();
        } else if out {
            self.sign_out(ctl, false);
        }
        if let Some(on) = r.text_input {
            self.typing = on;
            ctl.set_text_input(on);
        }
        r.cursor.iter().for_each(|&c| ctl.set_cursor(CURSORS[c as usize]));
        // A tap on the focused window while typing asks for text input again,
        // inside its user activation: that brings back a dismissed keyboard. A
        // finger's scroll, wander or long press does not; a long press on
        // content that opened no menu is a tap.
        let tap = release.filter(|_| !r.gesture);
        if tap.is_some_and(|(x, y)| !asked && self.typing && self.over_focus(x, y)) {
            ctl.set_text_input(true);
        }
        let prevent_default = r.consumed && !(types && self.typing);
        Handled { redraw: r.redraw || r.animating, prevent_default }
    }

    fn kernel(&mut self, ev: KernelIn) -> Option<Response> {
        self.shell.as_mut().map(|s| s.kernel(ev))
    }

    /// Keeps /home (at once if `hiding`), samples the meters, pumps AI and the shell's queued
    /// effects (twice: each can cause the other), then reports; stores a theme.
    fn flush(&mut self, ctl: &mut Ctl, hiding: bool) {
        self.meter(ctl);
        let Some(shell) = &mut self.shell else { return };
        let ms = self.home.keep(shell.vfs(), ctl, &mut self.report, hiding);
        if let Some(ms) = ms.filter(|&ms| arm(&mut self.wake, ctl, ms)) {
            ctl.wake_in(ms);
        }
        for _ in 0..2 {
            let told = self.ai.pump(ctl, shell.kernel_mut()) | self.home.retell();
            if self.report.retell() | told {
                let status = self.report.status(self.ai.status());
                shell.set_ai(ui::AiStatus { unkept: self.home.unkept, ..status });
            }
            apply(shell.take_effects(), ctl, (&self.ai, &mut self.wake), &mut self.report);
        }
        self.report.pump(ctl, || context(shell));
        let theme = shell.theme_name();
        if theme != self.saved {
            self.saved = theme;
            ctl.storage_set(&logon::own(THEME_KEY), theme);
        }
    }

    /// With a watcher, samples the meters if due and posts the sample if it changed (before
    /// the effects are applied, so the answer leaves now); arms the timer while one will be due.
    fn meter(&mut self, ctl: &mut Ctl) {
        let (watch, h) = {
            let h = &mut *self.ai.0.borrow_mut();
            self.pace.fresh |= mem::take(&mut h.fresh);
            (h.watch, h.counts)
        };
        self.watcher = watch;
        let (Some(shell), Some(watcher)) = (&mut self.shell, watch) else { return };
        let now = ctl.monotonic_ms() as u64;
        if self.pace.due(now) {
            let (f, home, grain) = (self.frames, &self.home, shell.grain_in().is_some());
            let k = shell.kernel_mut();
            // As uiwire's Event::Stats: its code, the sample's length (set last), the sample.
            let mut v = vec![16, 0, 0, 0, 0, stat::VERSION];
            v.extend_from_slice(&(now as u32).to_le_bytes());
            let (kept, unkept) = (home.kept_len() as u32, home.unkept.into());
            let mut loud = [f[0], f[1], f[2], grain.into(), kept, unkept, 0, 0, 0, 0, 0, 0];
            loud[stat::ASKED..].copy_from_slice(&h);
            words(&mut v, &loud);
            k.table(watcher, &mut v);
            // The other workers' meters (whether one runs: hot); the watcher's are quiet.
            let stats = ctl.proc_stats();
            let others = stats.iter().filter(|m| m.0 != watcher);
            v.extend_from_slice(&(others.clone().count() as u16).to_le_bytes());
            let mut hot = false;
            for &(pid, [busy, kb, run]) in others {
                v.extend_from_slice(&pid.to_le_bytes());
                words(&mut v, &[busy, kb]);
                hot |= run == 1;
            }
            let loud = stat::hash(&v[10..]);
            words(&mut v, &[f[3], f[4], f[5], (self.busy * 1000.0) as u64 as u32, desktop_kb()]);
            let own = stats.iter().find(|m| m.0 == watcher);
            words(&mut v, own.map_or(&[][..], |m| &m.1[..2]));
            let n = (v.len() - 5) as u32;
            v[1..5].copy_from_slice(&n.to_le_bytes());
            if self.pace.took(now, loud, hot) {
                k.post_event(watcher, &v);
            }
        }
        let ms = self.pace.wait(now).map(|ms| ms + 1);
        if let Some(ms) = ms.filter(|&ms| arm(&mut self.wake, ctl, ms)) {
            ctl.wake_in(ms);
        }
    }

    /// Draws into the draw list (the desktop, then the welcome: over it while it leaves); the
    /// clear color. The welcome goes once its flight is over.
    fn paint(&mut self, dpr: f32, ctl: &Ctl) -> Rgba {
        let (now, reduced, mut base) =
            (ctl.monotonic_ms(), ctl.reduced_motion(), ui::theme("").base);
        self.list.clear();
        if let Some(shell) = &mut self.shell {
            shell.set_now(now);
            shell.set_dpr(dpr);
            shell.set_reduced_motion(reduced);
            shell.draw(&mut self.list);
            base = shell.clear_color();
        }
        // Over a desktop the welcome is a layer: its flight, or the card files went unkept.
        let (over, logon) = (self.shell.is_some(), &mut self.logon);
        self.wish = match (logon, text_of(&mut self.shell, &mut self.parts)) {
            (Some(l), Some(text)) if over => l.layer(&mut self.list, text, &self.record, now),
            (Some(l), Some(text)) => {
                text.set_dpr(dpr);
                base = l.theme().base;
                l.draw(&mut self.list, text, &self.record, (now, reduced))
            }
            _ => None,
        };
        if self.logon.as_ref().is_some_and(|l| l.gone(now)) {
            self.logon = None;
        }
        base
    }

    /// After a frame: the next when the shell or the welcome wants it (at once while anything
    /// moves, else by timer: the living grain's, an app's timer's), the deferred fonts after the
    /// first (when the start's record reads its timings), what the shell queued.
    fn drawn(&mut self, ctl: &mut Ctl) {
        let shell = self.shell.as_ref().and_then(Shell::frame_in);
        let next = [shell, self.wish].into_iter().flatten().min();
        match next {
            Some(0) => ctl.request_frame(),
            Some(ms) => ctl.frame_in(ms),
            None => {}
        }
        self.why |= next.map_or(0, |ms| if ms == 0 { MOTION } else { TIMER });
        if self.deferred.is_none() {
            self.deferred = Some([true; 2]);
            self.record.first(ctl.monotonic_ms(), ctl.timings());
            DEFERRED.iter().for_each(|d| ctl.fetch(d.0, d.2));
        }
        self.flush(ctl, false);
    }

    /// The slot of the deferred font fetch `id` brings, if still awaited.
    fn take_deferred(&mut self, id: u32) -> Option<FontId> {
        let waiting = self.deferred.as_mut()?;
        let i = DEFERRED.iter().position(|d| d.0 == id)?;
        std::mem::take(&mut waiting[i]).then_some(DEFERRED[i].1)
    }

    /// Fills a deferred font's slot and redraws, or quietly leaves it empty; the start's record
    /// hears either (and, once both landed, notes how the start went).
    fn set_font(&mut self, slot: FontId, got: Result<Vec<u8>, String>, ctl: &Ctl) -> Handled {
        let text = text_of(&mut self.shell, &mut self.parts);
        let ok = text.zip(got.ok()).is_some_and(|(text, b)| text.set_font(slot, b).is_ok());
        let (i, now) = (usize::from(slot == FontId::Mono), ctl.monotonic_ms());
        self.record.font(i, ok, now, ctl.timings()).inspect(|n| report::note(n));
        Handled { redraw: ok || self.logon.is_some(), prevent_default: false }
    }

    /// Whether what takes the keys (the focused window, or the overlay) is at `(x, y)`.
    fn over_focus(&self, x: f32, y: f32) -> bool {
        self.shell.as_ref().is_some_and(|s| s.types_at(x, y))
    }
}

impl App for Desktop {
    /// The event's cause (the person's input, a program, the watcher's own) is the next frame's
    /// if it redraws; all but the watcher's own stir the meters.
    fn event(&mut self, ev: Event, ctl: &mut Ctl) -> Handled {
        let (t, hiding) = (ctl.monotonic_ms(), matches!(ev, Event::Hidden));
        let bit = match ev {
            Event::Key { .. } | Event::Text(_) | Event::Wheel { .. } => INPUT,
            Event::PointerMove { .. } | Event::PointerDown { .. } => INPUT,
            Event::PointerUp { .. } | Event::PointerLeave => INPUT,
            Event::Proc { pid, .. } if Some(pid) == self.watcher => SELF,
            Event::Proc { .. } | Event::ProcError { .. } | Event::Wake => PROGRAMS,
            _ => 0,
        };
        self.pace.stir |= bit != SELF;
        let h = self.handle(ev, ctl);
        self.why |= if h.redraw { bit } else { 0 };
        self.flush(ctl, hiding);
        self.busy += ctl.monotonic_ms() - t;
        h
    }

    /// Counts the frame under its first cause; the watcher's own and a timer's stir nothing (an
    /// app's timer stirs by what its program then draws).
    fn frame(&mut self, r: &mut Renderer, ctl: &mut Ctl) {
        let t = ctl.monotonic_ms();
        let cause = (self.why.trailing_zeros() as usize).min(5);
        self.frames[cause] = self.frames[cause].wrapping_add(1);
        self.pace.stir |= !matches!(cause, 3 | 4);
        self.why = 0;
        let bg = self.paint(r.dpr(), ctl);
        if let Some(text) = text_of(&mut self.shell, &mut self.parts) {
            r.draw(&self.list, bg, text.atlas_mut());
        }
        self.drawn(ctl);
        self.busy += ctl.monotonic_ms() - t;
    }
}

/// `n` as [`stat`] writes counts: a u16 count, then each u32.
fn words(v: &mut Vec<u8>, n: &[u32]) {
    v.extend_from_slice(&(n.len() as u16).to_le_bytes());
    n.iter().for_each(|n| v.extend_from_slice(&n.to_le_bytes()));
}

/// The desktop's wasm memory in KB (none natively).
fn desktop_kb() -> u32 {
    #[cfg(target_arch = "wasm32")]
    return core::arch::wasm32::memory_size::<0>() as u32 * 64;
    #[cfg(not(target_arch = "wasm32"))]
    0
}

type Parts = Option<(TextSystem, Vfs)>;

/// The text system: the shell's, or the one waiting for it.
fn text_of<'a>(shell: &'a mut Option<Shell>, parts: &'a mut Parts) -> Option<&'a mut TextSystem> {
    match shell {
        Some(shell) => Some(shell.text_mut()),
        None => parts.as_mut().map(|p| &mut p.0),
    }
}

/// Applies `fx` in order, telemetry seeing feedback and preferences first; of two timers asked
/// for, the sooner stands.
fn apply(
    fx: Vec<Effect>,
    ctl: &mut Ctl,
    (ai, wake): (&ai::Ai, &mut f64),
    report: &mut report::Reports,
) {
    for fx in fx {
        match &fx {
            Effect::Feedback { kind, text, context } => report.feedback(kind, text, *context),
            Effect::Pref { key, value } => report.pref(ctl, key, value),
            Effect::Kernel(K::Wake { ms }) if !arm(wake, ctl, *ms) => continue,
            _ => {}
        }
        effect(fx, ctl, ai);
    }
}

/// Whether the one-shot timer should be armed for `ms` (`wake`: when it fires): not when it
/// fires sooner already.
fn arm(wake: &mut f64, ctl: &Ctl, ms: u32) -> bool {
    let (now, at) = (ctl.monotonic_ms(), ctl.monotonic_ms() + f64::from(ms));
    let sooner = *wake > now && *wake <= at;
    if !sooner {
        *wake = at;
    }
    !sooner
}

fn effect(fx: Effect, ctl: &mut Ctl, ai: &ai::Ai) {
    match fx {
        Effect::Fetch { id, url } => ctl.fetch(id, &url),
        Effect::Kernel(K::Spawn { pid, sab }) => ctl.spawn(pid, sab),
        Effect::Kernel(K::Start { pid, msg, program }) => ctl.start(pid, msg, load(program)),
        Effect::Kernel(K::Send { pid, msg }) => ctl.send(pid, msg),
        Effect::Kernel(K::Reply { pid, errno, data }) => ctl.reply(pid, errno, data),
        Effect::Kernel(K::Word { pid, index, value }) => ctl.word(pid, index, value),
        Effect::Kernel(K::Kill { pid }) => ctl.kill(pid),
        Effect::Kernel(K::Wake { ms }) => ctl.wake_in(ms),
        Effect::Pref { key, value } => pref(&key, &value, ctl, ai),
        // Telemetry took it ([`apply`]): it goes at the next flush, which can say what is open.
        // Signing out and resetting are the desktop's own, after the effects before them.
        Effect::Feedback { .. } | Effect::SignOut | Effect::Reset => {}
        // The host hands frames to the apps; none reach here.
        Effect::Kernel(K::Draw { .. }) => {}
    }
}

/// Stores a preference of [`PREFS`] as `compusophy.<key>` (the AI model through the AI hub,
/// which checks it; the signed-in profile's face in the list); any other key is dropped.
fn pref(key: &str, value: &str, ctl: &mut Ctl, ai: &ai::Ai) {
    match key {
        ui::AI_MODEL => ai.set_model(ctl, value),
        "face" => {
            let list = set_face(ctl.storage_get(LIST).as_deref(), value);
            list.iter().for_each(|list| ctl.storage_set(LIST, list));
        }
        key if PREFS.contains(&key) => ctl.storage_set(&logon::own(key), value),
        _ => {}
    }
}

/// What the shell starts from: the stored theme, favorites, home screen order, first-visit
/// mark and grain; whether programs can run (before the first-visit Welcome, one, opens).
fn prefs(ctl: &Ctl) -> shell::Prefs {
    let get = |key: &str| ctl.storage_get(&logon::own(key));
    let (theme, seen) = (get("theme").unwrap_or_default(), ctl.storage_get(logon::SEEN).is_some());
    let (grain_off, isolated) = (get(ui::GRAIN).as_deref() == Some("off"), ctl.isolated());
    let (dock, home, folders) = (get("dock"), get("home.order"), get("folders"));
    shell::Prefs { theme, dock, home, folders, seen, grain_off, isolated }
}

/// Where a report comes from: the device, the theme, the windows open.
fn context(shell: &Shell) -> report::Context {
    let theme = shell.theme_name().to_string();
    report::Context {
        device: platform::device(),
        theme,
        windows: windows(shell),
        ..Default::default()
    }
}

/// The windows open as a report says them: their apps ([`report::app`]), never a file's path.
fn windows(shell: &Shell) -> String {
    let apps: Vec<&str> = shell.open_apps().into_iter().map(report::app).collect();
    apps.join(", ")
}

/// A Start's program as the platform takes it.
fn load(program: kernel::Load) -> platform::Load {
    match program {
        kernel::Load::None => platform::Load::None,
        kernel::Load::Bytes(b) => platform::Load::Bytes(b),
        kernel::Load::Url(u) => platform::Load::Url(u),
    }
}

/// The CSS `cursor` keyword of each [`Cursor`], in its order.
const CURSORS: [&str; 8] =
    ["default", "text", "grab", "grabbing", "ew-resize", "ns-resize", "nwse-resize", "nesw-resize"];

fn local(t: platform::LocalTime) -> LocalTime {
    let platform::LocalTime { year, month, day, weekday, hour, minute } = t;
    LocalTime { year, month, day, weekday, hour, minute }
}

/// Makes apps by name: the built-ins, then the GUI programs and `.app` files.
fn registry(ai: ai::Ai) -> Registry {
    Box::new(move |name| remote::open(name, &ai))
}

/// `a` then `b`, as one response.
fn merge(mut a: Response, b: Response) -> Response {
    a.redraw |= b.redraw;
    a.consumed |= b.consumed;
    a.animating |= b.animating;
    a.text_input = b.text_input.or(a.text_input);
    a.cursor = b.cursor.or(a.cursor);
    a.effects.extend(b.effects);
    a
}

/// The shell input for an event; `None` for key-ups, fetches and kernel events.
fn input_of(ev: Event) -> Option<Input> {
    Some(match ev {
        Event::Key { down: false, .. } | Event::Fetched { .. } => return None,
        Event::Chunk { .. } | Event::StreamEnd { .. } => return None,
        Event::Proc { .. } | Event::ProcError { .. } | Event::Wake | Event::Hidden => return None,
        Event::Derived { .. } => return None,
        Event::Key { code, key, shift, ctrl, alt, meta, altgr, .. } => {
            let (ctrl, alt) = without_altgr(ctrl, alt, altgr);
            let mods = Mods { shift, ctrl, alt, meta };
            Input::Key { key: key_of(&code, &key, ctrl || alt || meta), mods }
        }
        Event::Text(s) => Input::Text(s),
        Event::PointerMove { x, y } => Input::PointerMove { x, y },
        Event::PointerDown { x, y, button, touch } => Input::PointerDown { x, y, button, touch },
        Event::PointerUp { x, y, button } => Input::PointerUp { x, y, button },
        Event::PointerLeave => Input::PointerLeave,
        Event::Wheel { x, y, dy } => Input::Wheel { x, y, dy },
        Event::Resize { w, h, .. } => Input::Resize { w, h },
        Event::Tick { time } => Input::Tick { time: local(time) },
    })
}

/// The shell key for a key-down: by `code`, or with no code (phone keyboards) by `key`. With a
/// `chord` (Ctrl, Alt or Meta) an ASCII letter `key` is that letter (AZERTY's Ctrl+Z is not
/// Ctrl+W); a NumLock-off keypad key is the key it names; `Backquote` is a backquote everywhere.
fn key_of(code: &str, key: &str, chord: bool) -> Key {
    let one = key.chars().next().filter(|c| c.len_utf8() == key.len());
    if let Some(c) = one.filter(|c| chord && c.is_ascii_alphabetic()) {
        return Key::Char(c.to_ascii_lowercase());
    }
    let pad = code.strip_prefix("Numpad").is_some_and(|k| k.len() == 1 || k == "Decimal");
    if pad && Key::from_code(key) != Key::Other {
        return Key::from_code(key);
    }
    match code {
        "Backquote" => Key::Char('`'),
        "" | "Unidentified" => match one {
            Some(' ') => Key::Space,
            Some(c) if c.is_ascii_alphanumeric() || c == '`' => Key::Char(c.to_ascii_lowercase()),
            _ => Key::from_code(key),
        },
        code => Key::from_code(code),
    }
}

/// Ctrl and Alt as the shell sees them: AltGr reported as both types text.
fn without_altgr(ctrl: bool, alt: bool, altgr: bool) -> (bool, bool) {
    let text = altgr && ctrl && alt;
    (ctrl && !text, alt && !text)
}

/// Whether a key-down types into a focused textarea unless prevented: its `key`
/// is text or a dead key's, IME's or phone keyboard's, with no Ctrl, Alt or Meta.
fn types_text(ev: &Event) -> bool {
    let Event::Key { key, down: true, ctrl, alt, meta, altgr, .. } = ev else { return false };
    let (ctrl, alt) = without_altgr(*ctrl, *alt, *altgr);
    // Named key values are ASCII words; text is one character or a cluster.
    let named = key.len() > 1 && key.bytes().all(|b| b.is_ascii_alphanumeric());
    let text = !key.is_empty() && !named && !key.contains(char::is_control);
    let owned = matches!(key.as_str(), "Dead" | "Process" | "Unidentified");
    !(ctrl || alt || *meta) && (text || owned)
}

/// A key-up draws nothing; releasing Alt or Meta is prevented, so Firefox
/// does not focus its menu bar.
fn key_up(code: &str) -> Handled {
    let prevent_default = matches!(code, "AltLeft" | "AltRight" | "MetaLeft" | "MetaRight");
    Handled { redraw: false, prevent_default }
}

#[cfg(test)]
mod tests;
