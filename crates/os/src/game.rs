//! Games in windows (the fractal, the other way): a cartridge from another site (secretspace's
//! games) runs in a window as an app of the OS, never in an iframe. [`platform::cart`] loads its
//! module and copies its frames into a texture the window draws (an image in the draw list); the
//! window takes its input whole ([`App::raw`]) and hands it over: every key down and up, the
//! pointer's every move and button, the wheel, typed text while a field of it has the keys, and
//! the pointer locked while it asks. Battlestation is handed a module of this OS's own files
//! (`platform::files_url`), so a desk inside the OS shows the OS on its monitor, and inside
//! that, the desk again.

use platform::cart::{self, Spec, State};
use ui::icon::Glyph;
use ui::{App, AppEvent, AppIcon, Cx, Raw, Rgba, Sense, Ui, WidgetId};

/// Where secretspace serves its games (each `<id>/pkg/<id>.js`, open to other sites).
pub const SECRETSPACE: &str = "https://secretspace.compusophy.com";

/// A cartridge the desktop knows: its registry name, title, icon, whether it shows the OS on a
/// screen of its own (it is handed the OS's module), and whether it is the OS itself.
pub struct Kind {
    pub name: &'static str,
    pub title: &'static str,
    pub icon: AppIcon,
    pub os: bool,
    pub own: bool,
}

/// The games (in Games), and "computer": compusophyOS itself as a cartridge, another computer in
/// a window (a whole other instance from its own files, as a desk's monitor runs it).
pub static GAMES: [Kind; 4] = [
    game("wyrm", "Wyrm", icon(Glyph::Wyrm, 0x4ade80), false),
    game("wandfall", "Wandfall", icon(Glyph::Wand, 0xa78bfa), false),
    game("battlestation", "Battlestation", icon(Glyph::Desk, 0xf472b6), true),
    Kind { own: true, ..game("computer", "Computer", icon(Glyph::Window, 0x38bdf8), false) },
];

const fn game(name: &'static str, title: &'static str, icon: AppIcon, os: bool) -> Kind {
    Kind { name, title, icon, os, own: false }
}

const fn icon(glyph: Glyph, hue: u32) -> AppIcon {
    AppIcon { glyph, hue: Rgba::hex(hue) }
}

/// The hit over the whole window.
const SCREEN: WidgetId = WidgetId(1);

/// A game in a window: see the module docs.
pub struct Game {
    kind: &'static Kind,
    /// Its cartridge, once the window has a size; the size and pixel ratio it was last given.
    cart: Option<u32>,
    size: (f32, f32, f32),
    /// Whether the pointer is locked to it; why it failed.
    locked: bool,
    failed: Option<String>,
}

impl Game {
    pub fn new(kind: &'static Kind) -> Game {
        Game { kind, cart: None, size: (0.0, 0.0, 1.0), locked: false, failed: None }
    }

    /// Its cartridge, starting it at `w` x `h` CSS px and `dpr` the first time.
    fn start(&mut self, w: f32, h: f32, dpr: f32) {
        let name = self.kind.name;
        let (base, module) = match self.kind.own {
            true => (platform::files_url(), "os"),
            false => ([SECRETSPACE, "/", name, "/pkg/"].concat(), name),
        };
        let spec = Spec {
            url: [base.as_str(), module, ".js"].concat(),
            w,
            h,
            dpr,
            base,
            ns: ["compusophy.game.", name, "."].concat(),
            query: String::new(),
            os: self.kind.os.then(platform::files_url),
        };
        self.cart = Some(cart::load(spec));
        self.size = (w, h, dpr);
    }

    /// The running cartridge.
    fn running(&self) -> Option<u32> {
        self.cart.filter(|&id| cart::state(id) == Some(State::Ready))
    }

    /// After input it took: the pointer locked or let go as it now asks (asked here, inside the
    /// press or key that may lock it, as browsers allow).
    fn lock(&mut self, id: u32, cx: &mut Cx<'_>) {
        let want = cart::capture(id);
        if want != self.locked {
            cx.lock_pointer(want);
        }
    }
}

impl App for Game {
    fn title(&self) -> String {
        self.kind.title.into()
    }

    fn preferred_size(&self) -> Option<(f32, f32)> {
        Some((1024.0, 640.0))
    }

    fn icon(&self) -> AppIcon {
        self.kind.icon
    }

    fn raw(&self) -> bool {
        true
    }

    // It draws on its own clock: while it loads or runs, the window shows each new frame.
    fn frame_in(&self, _now_ms: f64) -> Option<u32> {
        let live = self.cart.and_then(cart::state);
        matches!(live, Some(State::Loading | State::Ready)).then_some(0)
    }

    fn event(&mut self, ev: AppEvent, cx: &mut Cx<'_>) -> bool {
        let Some(id) = self.running() else {
            return matches!(ev, AppEvent::Tick { .. }) && self.cart.is_some();
        };
        match ev {
            AppEvent::Raw(Raw::Pointer { kind, x, y, button }) => {
                cart::pointer(id, kind, x, y, button);
                self.lock(id, cx);
            }
            AppEvent::Raw(Raw::Key { code, key, down, repeat, mods }) => {
                cart::key(id, down, &code, &key, mods, repeat);
                // A printable key types while a field of it has the keys: the host types for it
                // (the window takes its keys whole, so no text input of the page's comes).
                let chord = mods & (Raw::CTRL | Raw::META) != 0;
                if down && !chord && key.chars().count() == 1 && cart::typing(id) {
                    cart::text(id, &key);
                }
                self.lock(id, cx);
            }
            AppEvent::Raw(Raw::Wheel { x, y, dy }) => _ = cart::wheel(id, x, y, dy),
            AppEvent::Raw(Raw::Motion { dx, dy }) => cart::motion(id, dx, dy),
            AppEvent::Raw(Raw::Locked(on)) => {
                self.locked = on;
                cart::locked(id, on);
            }
            AppEvent::Text(s) => cart::text(id, &s),
            AppEvent::Focus(false) if self.locked => cx.lock_pointer(false),
            AppEvent::Tick { .. } => {
                if cart::closed(id) {
                    cx.close_self();
                } else if self.locked && !cart::capture(id) {
                    cx.lock_pointer(false);
                }
            }
            _ => return false,
        }
        true
    }

    fn closing(&mut self, cx: &mut Cx<'_>) {
        if self.locked {
            cx.lock_pointer(false);
        }
        if let Some(id) = self.cart.take() {
            cart::end(id);
        }
    }

    fn draw(&mut self, ui: &mut Ui<'_>) {
        let r = ui.rect();
        ui.hit(SCREEN, r, Sense::Click);
        let dpr = ui.text_system().dpr();
        ui.fill(r, 0.0, Rgba(0, 0, 0, 255));
        if self.cart.is_none() && r.w >= 1.0 && r.h >= 1.0 {
            self.start(r.w, r.h, dpr);
        }
        let Some(id) = self.cart else { return };
        match cart::state(id) {
            Some(State::Ready) => {
                if self.size != (r.w, r.h, dpr) {
                    self.size = (r.w, r.h, dpr);
                    cart::resize(id, r.w, r.h, dpr);
                }
                let error = cart::error(id);
                if error.is_empty() {
                    ui.list().image(r, 0.0, id, 255);
                } else {
                    self.failed = Some(error);
                }
            }
            Some(State::Failed(why)) => self.failed = Some(why),
            Some(State::Loading) | None => {
                _ = ui.small(&["Starting ", self.kind.title, "\u{2026}"].concat())
            }
        }
        if let Some(why) = &self.failed {
            ui.small(&[self.kind.title, " stopped: ", why].concat());
        }
    }
}
