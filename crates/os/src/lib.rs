//! compusophyOS's wasm entry: `start` runs a desktop on [`platform::run`] with the boot font, a
//! [`Vfs`] holding the `/bin` markers and Studio's samples, and a [`Registry`] of [`apps::open`]
//! then [`remote::open`] (the GUI programs: About, Feedback, Files, Welcome, Studio, the
//! Assistant, `.app` files). The [`Shell`] is made at the first Resize that leaves a work area;
//! until then input is dropped (a missed Tick is replayed) and frames clear to the default theme's
//! base.
//! The theme is kept in `localStorage` ([`THEME_KEY`]), as are the preferences of [`PREFS`]
//! (`compusophy.<key>`), which apps and the shell set ([`shell::Effect::Pref`]) and the shell
//! reads when it is made ([`shell::Prefs`]).
//!
//! Fonts: boot (Inter Regular, in the wasm); deferred (Inter SemiBold and JetBrains Mono, fetched
//! after the first frame under the top two fetch ids, which the shell never reaches; a failure
//! leaves bold as Regular and mono cells empty); lazy (symbol fallbacks the shell fetches). A
//! key-down that types text is never prevented while text input is on: text reaches apps only
//! through the platform's textarea.

#![forbid(unsafe_code)]

pub mod ai;
pub mod home;
pub mod remote;
pub mod report;

use gfx::{DrawList, Rgba};
use platform::{App, Ctl, Event, Handled, Renderer};
use shell::{Effect, Input, KernelIn, Key, LocalTime, Mods, Registry, Response, Shell};
use ui::kernel::{self, Effect as K};
use ui::{FontId, TextSystem};
use vfs::Vfs;
use wasm_bindgen::prelude::*;

const SANS: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");

/// The deferred fonts as (fetch id, slot, URL).
const DEFERRED: [(u32, FontId, &str); 2] = [
    (u32::MAX - 1, FontId::SansBold, "fonts/deferred/Inter-SemiBold.ttf"),
    (u32::MAX, FontId::Mono, "fonts/deferred/JetBrainsMono-Regular.ttf"),
];

/// The `localStorage` key of the theme's name.
pub const THEME_KEY: &str = "compusophy.theme";
/// The preferences kept as `compusophy.<key>`: the AI model (which the AI hub keeps, see [`ai`]),
/// the dock's favorites (registry names, comma-separated), `"1"` once Welcome was shown on a
/// first visit, `"off"` to stop automatic error reports, the home screen's order (as the dock's)
/// and `"off"` to still the grain. Other keys are dropped.
pub const PREFS: [&str; 6] = [ui::AI_MODEL, "dock", "seen", "reports", "home.order", ui::GRAIN];
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
}

impl Desktop {
    fn new() -> Result<Desktop, String> {
        let text = TextSystem::new(SANS.to_vec())?;
        let mut vfs = Vfs::new();
        let _ = vfs.mkdir_all("/bin"); // Not mkdir: Vfs::new ships mkdir_all already.
        for name in APPLETS {
            let _ = vfs.write(&["/bin/", name].concat(), b"#!wasm bin/toolbox.wasm\n");
        }
        let _ = vfs.write(remote::STUDIO, b"#!wasm bin/studio.wasm\n");
        let _ = vfs.write(remote::ASSISTANT, b"#!wasm bin/assistant.wasm\n");
        for (name, ..) in remote::SYSTEM {
            let _ = vfs.write(&["/bin/", name].concat(), b"#!wasm bin/system.wasm\n");
        }
        Ok(Desktop { parts: Some((text, vfs)), ..Desktop::default() })
    }

    /// Hands `input` to the shell, first making it at the first usable size.
    fn input(&mut self, input: Input, ctl: &mut Ctl) -> Option<Response> {
        if let Some(shell) = &mut self.shell {
            return Some(shell.input(input));
        }
        self.missed_tick |= matches!(input, Input::Tick { .. });
        match input {
            Input::Resize { w, h } if w >= 1.0 && h >= shell::BAR_H + shell::DOCK_CLEAR + 1.0 => {
                let (text, mut vfs) = self.parts.take()?;
                self.home.restore(&mut vfs, ctl, &mut self.report);
                let prefs = prefs(ctl);
                let shell = Shell::new(w, h, text, vfs, registry(self.ai.clone()), prefs);
                let shell = self.shell.insert(shell);
                self.ai.load(ctl);
                self.saved = shell.theme_name();
                shell.set_now(ctl.monotonic_ms());
                let mut r = shell.input(input);
                if self.missed_tick {
                    r = merge(r, shell.input(Input::Tick { time: local(ctl.local_time()) }));
                }
                Some(r)
            }
            _ => None,
        }
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
                Some(slot) => return self.set_font(slot, result),
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
            ev => input_of(ev).and_then(|input| self.input(input, ctl)),
        };
        let Some(r) = r else { return Handled::default() };
        let asked = r.text_input.is_some();
        apply(r.effects, ctl, (&self.ai, &mut self.wake), &mut self.report);
        if let Some(on) = r.text_input {
            self.typing = on;
            ctl.set_text_input(on);
        }
        if let Some(c) = r.cursor {
            ctl.set_cursor(CURSORS[c as usize]);
        }
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

    /// Keeps /home (at once if `hiding`), pumps AI and the shell's queued effects (twice: each
    /// can cause the other), then reports; stores a theme.
    fn flush(&mut self, ctl: &mut Ctl, hiding: bool) {
        let Some(shell) = &mut self.shell else { return };
        if let Some(ms) = self.home.keep(shell.vfs(), ctl, &mut self.report, hiding) {
            if arm(&mut self.wake, ctl, ms) {
                ctl.wake_in(ms);
            }
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
            ctl.storage_set(THEME_KEY, theme);
        }
    }

    /// Draws into the draw list; the clear color, and whether an animation runs.
    fn paint(&mut self, dpr: f32, ctl: &Ctl) -> (Rgba, bool) {
        let Some(shell) = &mut self.shell else {
            self.list.clear();
            return (ui::theme("").base, false);
        };
        shell.set_now(ctl.monotonic_ms());
        shell.set_dpr(dpr);
        shell.set_reduced_motion(ctl.reduced_motion());
        let animating = shell.draw(&mut self.list);
        (shell.clear_color(), animating)
    }

    /// After a frame: the next while animating (else the living grain's, by timer), the
    /// deferred fonts after the first, what the shell queued.
    fn drawn(&mut self, animating: bool, ctl: &mut Ctl) {
        if animating {
            ctl.request_frame();
        } else if let Some(ms) = self.shell.as_ref().and_then(Shell::grain_in) {
            ctl.frame_in(ms);
        }
        if self.deferred.is_none() {
            self.deferred = Some([true; 2]);
            for (id, _, url) in DEFERRED {
                ctl.fetch(id, url);
            }
        }
        self.flush(ctl, false);
    }

    /// The slot of the deferred font fetch `id` brings, if still awaited.
    fn take_deferred(&mut self, id: u32) -> Option<FontId> {
        let waiting = self.deferred.as_mut()?;
        let i = DEFERRED.iter().position(|d| d.0 == id)?;
        std::mem::take(&mut waiting[i]).then_some(DEFERRED[i].1)
    }

    /// Fills a deferred font's slot and redraws, or quietly leaves it empty.
    fn set_font(&mut self, slot: FontId, got: Result<Vec<u8>, String>) -> Handled {
        let text = text_of(&mut self.shell, &mut self.parts);
        let redraw = match (got, text) {
            (Ok(bytes), Some(text)) => text.set_font(slot, bytes).is_ok(),
            _ => false,
        };
        Handled { redraw, prevent_default: false }
    }

    /// Whether what takes the keys (the focused window, or the overlay) is at `(x, y)`.
    fn over_focus(&self, x: f32, y: f32) -> bool {
        self.shell.as_ref().is_some_and(|s| s.types_at(x, y))
    }
}

impl App for Desktop {
    fn event(&mut self, ev: Event, ctl: &mut Ctl) -> Handled {
        let hiding = matches!(ev, Event::Hidden);
        let h = self.handle(ev, ctl);
        self.flush(ctl, hiding);
        h
    }

    fn frame(&mut self, r: &mut Renderer, ctl: &mut Ctl) {
        let (bg, animating) = self.paint(r.dpr(), ctl);
        if let Some(text) = text_of(&mut self.shell, &mut self.parts) {
            r.draw(&self.list, bg, text.atlas_mut());
        }
        self.drawn(animating, ctl);
    }
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
        Effect::Feedback { .. } => {}
        // The host hands frames to the apps; none reach here.
        Effect::Kernel(K::Draw { .. }) => {}
    }
}

/// Stores a preference of [`PREFS`] as `compusophy.<key>` (the AI model through the AI hub,
/// which checks it); any other key is dropped.
fn pref(key: &str, value: &str, ctl: &mut Ctl, ai: &ai::Ai) {
    match key {
        ui::AI_MODEL => ai.set_model(ctl, value),
        key if PREFS.contains(&key) => ctl.storage_set(&["compusophy.", key].concat(), value),
        _ => {}
    }
}

/// What the shell starts from: the stored theme, favorites, home screen order, first-visit
/// mark and grain; whether programs can run (before the first-visit Welcome, one, opens).
fn prefs(ctl: &Ctl) -> shell::Prefs {
    let get = |key: &str| ctl.storage_get(&["compusophy.", key].concat());
    let (theme, seen) = (ctl.storage_get(THEME_KEY).unwrap_or_default(), get("seen").is_some());
    let (grain_off, isolated) = (get(ui::GRAIN).as_deref() == Some("off"), ctl.isolated());
    shell::Prefs { theme, dock: get("dock"), home: get("home.order"), seen, grain_off, isolated }
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
    Box::new(move |name| apps::open(name).or_else(|| remote::open(name, &ai)))
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
