//! The OS inside itself (the fractal): the Monitor app holds a whole desktop of its own, a
//! [`Shell`] with its home screen, windows and programs, laid out at the window's size (narrow,
//! it is a phone's) and drawn into the window: its draw list replayed there, vector, its glyphs
//! drawn from the desktop's own atlas (its text system lent it while it draws), so it is as crisp
//! as the desktop around it. Its own kernel runs its programs in the page beside the desktop's,
//! in a pid range of its own ([`SPAN`] a desktop, past [`FIRST`]): what it asks of the page goes
//! out through the window ([`Cx::kernel_out`]) and their messages come back to it
//! ([`AppEvent::Proc`]), and to it those of the desktops it holds ([`App::runs`]). Inside, a
//! Monitor holds a desktop again, [`MAX_DEPTH`] deep. The held desktop is a guest's: nothing of
//! it is kept, and it has no fetches (the terminal's symbol fonts), no AI and no telemetry.

use std::cell::Cell;

use gfx::{DrawList, RectF};
use shell::{Effect, Input, KernelIn, LocalTime, Prefs, Response, Shell};
use ui::icon::Glyph;
use ui::{App, AppEvent, AppIcon, Cx, Rgba, Sense, TextSystem, Ui, WidgetId};

/// The page's kernel's pids run below `FIRST`; held desktops' kernels', `SPAN` each, from it.
pub const FIRST: u32 = 1 << 31;
pub const SPAN: u32 = 1 << 22;
/// The most desktops held inside one another (the page's is 0).
pub const MAX_DEPTH: u8 = 3;
pub const MONITOR_ICON: AppIcon = AppIcon { glyph: Glyph::Window, hue: Rgba::hex(0x38bdf8) };
/// The hit over the whole window, so a release on it is heard (as its Click).
const SCREEN: WidgetId = WidgetId(1);

thread_local! {
    /// Whether the page is cross-origin isolated (programs need it) and its local time, as the
    /// desktop last heard them ([`note`]); the next pid range (a page's are never reused).
    static PAGE: Cell<(bool, Option<LocalTime>)> = const { Cell::new((false, None)) };
    static NEXT: Cell<u32> = const { Cell::new(0) };
}

/// The page is (or is not) `isolated`, and its local time is `time`: what held desktops are told.
pub fn note(isolated: bool, time: Option<LocalTime>) {
    PAGE.with(|p| {
        let (_, was) = p.get();
        p.set((isolated, time.or(was)));
    });
}

/// A desktop in a window: see the module docs.
pub struct Monitor {
    ai: crate::ai::Ai,
    depth: u8,
    shell: Option<Shell>,
    /// The first pid of its kernel's range; whether the pointer is down in it, and where it was
    /// last.
    base: u32,
    pressed: bool,
    at: (f32, f32),
    /// What the held desktop last said: it wants text input, it animates; the time it was told.
    typing: bool,
    animating: bool,
    told: Option<LocalTime>,
    /// Whether the page has no pid range left for it (it opened 512 desktops).
    full: bool,
    list: DrawList,
    /// What its kernel asked of the page while it drew, sent on the next tick; the page clock it
    /// was last told.
    outbox: Vec<ui::kernel::Effect>,
    now: f64,
}

impl Monitor {
    /// A Monitor at `depth` (the page's desktop holds those at 1).
    pub fn new(ai: &crate::ai::Ai, depth: u8) -> Monitor {
        Monitor {
            ai: ai.clone(),
            depth,
            shell: None,
            base: 0,
            pressed: false,
            at: (0.0, 0.0),
            typing: false,
            animating: false,
            told: None,
            full: false,
            list: DrawList::new(),
            outbox: Vec::new(),
            now: 0.0,
        }
    }

    /// The held desktop at `w` x `h`, its kernel's pids in a range of its own.
    fn start(&mut self, w: f32, h: f32) {
        let k = NEXT.with(Cell::get);
        let range = k.checked_mul(SPAN).and_then(|o| FIRST.checked_add(o));
        let Some((base, end)) = range.and_then(|b| Some((b, b.checked_add(SPAN - 1)?))) else {
            self.full = true;
            return;
        };
        let Ok(text) = TextSystem::new(crate::SANS.to_vec()) else { return };
        let (isolated, time) = PAGE.with(Cell::get);
        let prefs = Prefs {
            theme: String::new(),
            dock: None,
            home: None,
            folders: None,
            seen: true,
            grain_off: true,
            isolated,
        };
        let registry = crate::registry(self.ai.clone(), self.depth);
        let mut shell = Shell::new(w, h, text, crate::system_vfs(), registry, prefs);
        NEXT.with(|n| n.set(k + 1));
        self.base = base;
        shell.kernel_mut().set_pids(base, end);
        if let Some(time) = time {
            shell.input(Input::Tick { time });
            self.told = Some(time);
        }
        self.shell = Some(shell);
    }

    /// What the held desktop answered: its kernel's asks of the page go out through the window
    /// (the rest it may not ask), and whether to redraw.
    fn answer(&mut self, r: Response, cx: &mut Cx<'_>) -> bool {
        let pending = self.shell.as_mut().map(Shell::take_effects).unwrap_or_default();
        for k in self.outbox.drain(..) {
            cx.kernel_out(k);
        }
        for e in r.effects.into_iter().chain(pending) {
            if let Effect::Kernel(k) = e {
                cx.kernel_out(k);
            }
        }
        if let Some(on) = r.text_input {
            self.typing = on;
        }
        self.animating = r.animating;
        r.redraw || r.animating
    }

    /// `input` for the held desktop, told the page clock first.
    fn input(&mut self, input: Input, now_ms: f64, cx: &mut Cx<'_>) -> bool {
        let Some(shell) = &mut self.shell else { return false };
        self.now = now_ms;
        shell.set_now(now_ms);
        let r = shell.input(input);
        self.answer(r, cx)
    }

    /// `ev` for its kernel.
    fn kernel(&mut self, ev: KernelIn, cx: &mut Cx<'_>) -> bool {
        let Some(shell) = &mut self.shell else { return false };
        let r = shell.kernel(ev);
        self.answer(r, cx)
    }
}

impl App for Monitor {
    fn title(&self) -> String {
        "Monitor".into()
    }

    fn preferred_size(&self) -> Option<(f32, f32)> {
        Some((960.0, 620.0))
    }

    fn icon(&self) -> AppIcon {
        MONITOR_ICON
    }

    fn wants_text_input(&self) -> bool {
        self.typing
    }

    fn frame_in(&self, now_ms: f64) -> Option<u32> {
        if self.animating || !self.outbox.is_empty() {
            return Some(0);
        }
        // Its timers count from the clock it was last told.
        let past = (now_ms - self.now).max(0.0) as u32;
        self.shell.as_ref().and_then(Shell::frame_in).map(|ms| ms.saturating_sub(past))
    }

    fn has_kernel(&self) -> bool {
        self.shell.is_some()
    }

    fn runs(&self, pid: u32) -> bool {
        let own = pid.checked_sub(self.base).is_some_and(|o| o < SPAN);
        self.shell.as_ref().is_some_and(|s| own || s.runs(pid))
    }

    fn event(&mut self, ev: AppEvent, cx: &mut Cx<'_>) -> bool {
        let now = cx.now_ms;
        match ev {
            AppEvent::Resized { w, h } => match self.shell.is_some() {
                false => {
                    self.start(w, h);
                    true
                }
                true => self.input(Input::Resize { w, h }, now, cx),
            },
            AppEvent::PointerDown { x, y, .. } => {
                (self.pressed, self.at) = (true, (x, y));
                self.input(Input::PointerDown { x, y, button: 0, touch: false }, now, cx)
            }
            AppEvent::Drag { x, y } => {
                self.at = (x, y);
                self.input(Input::PointerMove { x, y }, now, cx)
            }
            // The release (on the window's one hit).
            AppEvent::Click(_) if self.pressed => {
                self.pressed = false;
                let (x, y) = self.at;
                self.input(Input::PointerUp { x, y, button: 0 }, now, cx)
            }
            AppEvent::Key { key, mods } => self.input(Input::Key { key, mods }, now, cx),
            AppEvent::Text(s) => self.input(Input::Text(s), now, cx),
            AppEvent::Wheel { x, y, dy } => self.input(Input::Wheel { x, y, dy }, now, cx),
            AppEvent::Tick { now_ms } => {
                let time = PAGE.with(Cell::get).1;
                match time.filter(|t| self.told != Some(*t)) {
                    Some(time) => {
                        self.told = Some(time);
                        self.input(Input::Tick { time }, now_ms, cx)
                    }
                    None => {
                        let sent = !self.outbox.is_empty();
                        let r = Response { animating: self.animating, ..Response::default() };
                        self.answer(r, cx) || sent
                    }
                }
            }
            AppEvent::Focus(false) => self.input(Input::PointerLeave, now, cx),
            AppEvent::Proc { pid, msg } => self.kernel(KernelIn::Msg { pid, msg }, cx),
            AppEvent::ProcError { pid } => self.kernel(KernelIn::Error { pid }, cx),
            AppEvent::ProcWake => self.kernel(KernelIn::Wake, cx),
            _ => false,
        }
    }

    fn closing(&mut self, cx: &mut Cx<'_>) {
        // Its windows close (the desktops and programs they hold end), then what still runs.
        let Some(shell) = &mut self.shell else { return };
        shell.close_all();
        let pids: Vec<u32> =
            shell.kernel_mut().procs().into_iter().filter(|p| p.2).map(|p| p.0).collect();
        for pid in pids {
            shell.kernel_mut().kill(pid, ui::kernel::wire::KILLED);
        }
        let r = shell.kernel(KernelIn::Wake);
        self.answer(r, cx);
    }

    fn draw(&mut self, ui: &mut Ui<'_>) {
        let r = ui.rect();
        ui.hit(SCREEN, r, Sense::Click);
        let Some(shell) = &mut self.shell else {
            ui.small(match self.full {
                true => "No room for another desktop: reload the page.",
                false => "Starting a desktop\u{2026}",
            });
            return;
        };
        ui.fill(r, 0.0, shell.clear_color());
        // Its glyphs on the desktop's own atlas: the text system is lent it while it draws.
        shell.set_dpr(ui.text_system().dpr());
        self.now = ui.state().now_ms;
        shell.set_now(self.now);
        std::mem::swap(shell.text_mut(), ui.text_system());
        self.list.clear();
        self.animating = shell.draw(&mut self.list);
        std::mem::swap(shell.text_mut(), ui.text_system());
        let asked = shell.take_effects().into_iter().filter_map(|e| match e {
            Effect::Kernel(k) => Some(k),
            _ => None,
        });
        self.outbox.extend(asked);
        let at = host::motion::Vis {
            rect: RectF::new(0.0, 0.0, r.w, r.h),
            s: 1.0,
            dx: r.x,
            dy: r.y,
            a: 1.0,
        };
        host::motion::replay(ui.list(), &self.list, at);
    }
}
