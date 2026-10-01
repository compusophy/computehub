//! Welcome and About: the two pages to read.

use ui::{App, AppEvent, BUTTON_H, Cx, FontId, PAD, SPACING, TextStyle, Ui, WidgetId, theme};

/// Wheel scrolling for a page that may be taller than its window: the
/// offset, and its most, measured at each draw.
#[derive(Debug, Default)]
struct Scroll {
    y: f32,
    max: f32,
}

impl Scroll {
    /// Draws `page` moved up by the offset, then measures it.
    fn page(&mut self, ui: &mut Ui<'_>, page: impl FnOnce(&mut Ui<'_>)) {
        let r = ui.rect();
        ui.set_cursor(r.x + PAD, r.y + PAD - self.y);
        page(ui);
        let height = ui.cursor().1 + self.y - r.y + PAD - SPACING;
        self.max = (height - r.h).max(0.0);
        self.y = self.y.min(self.max);
    }

    /// Scrolls on the wheel; a resize redraws.
    fn event(&mut self, ev: &AppEvent) -> bool {
        let old = self.y;
        match *ev {
            AppEvent::Wheel { dy, .. } if dy.is_finite() => {
                self.y = (old + dy).max(0.0).min(self.max)
            }
            AppEvent::Resized { .. } => return true,
            _ => {}
        }
        self.y != old
    }
}

const WHAT: &str = "A tiling desktop in one canvas: Rust compiled to WebAssembly, no \
JavaScript beyond a two-line loader, and every pixel drawn by the GPU.";
const FONTS: &str = "Fonts: Inter, JetBrains Mono, Noto Sans Symbols and Noto Sans Symbols 2, \
under the SIL Open Font License 1.1. Their license texts are at /licenses/ on the site.";
const FORKS: &str = "fuel, cap, lang, wasmgen, applang-syntax and applang are forks of \
litelite 0.2.0 (commit 4f5e056, 2026-07-20), under Apache-2.0.";
const KEYS: [(&str, &str); 12] = [
    ("Alt+Space", "Launcher"),
    ("Alt+Enter", "Terminal"),
    ("Alt+Shift+Enter", "Floating terminal"),
    ("Alt+Q", "Close the window"),
    ("Alt+arrows or H/J/K/L", "Focus"),
    ("Alt+Shift+arrows", "Move"),
    ("Alt+Ctrl+arrows", "Resize"),
    ("Alt+F", "Float"),
    ("Alt+O", "Flip the split"),
    ("Alt+1-4", "Workspace"),
    ("Alt+Shift+1-4", "Send to a workspace"),
    ("Cmd", "Works like Alt on a Mac"),
];

/// What compusophyOS is, its keys, and buttons to start with.
#[derive(Debug, Default)]
pub struct Welcome {
    scroll: Scroll,
}

impl App for Welcome {
    fn title(&self) -> String {
        "Welcome".to_string()
    }

    fn draw(&mut self, ui: &mut Ui<'_>) {
        self.scroll.page(ui, |ui| {
            ui.heading("compusophyOS");
            ui.label(WHAT);
            ui.subheading("Keys");
            for (key, what) in KEYS {
                ui.key_value(key, what);
            }
            ui.space(PAD);
            buttons(ui);
            ui.label("Build an app: open Studio, edit counter.app, press Run.");
        });
    }

    fn event(&mut self, ev: AppEvent, cx: &mut Cx<'_>) -> bool {
        match ev {
            AppEvent::Click(WidgetId(1)) => cx.open("terminal"),
            AppEvent::Click(WidgetId(2)) => cx.open("studio"),
            AppEvent::Click(WidgetId(3)) => cx.open_floating("launcher"),
            AppEvent::Click(WidgetId(4)) => cx.open("about"),
            ev => return self.scroll.event(&ev),
        }
        true
    }
}

/// Welcome's buttons, the first the main one, as many to a row as fit: a
/// button is its label (Sans 14) and 14 px each side.
fn buttons(ui: &mut Ui<'_>) {
    const NAMES: [&str; 4] = ["Open Terminal", "Open Studio", "Launcher", "About"];
    let style = TextStyle::new(FontId::Sans, 14.0, theme::TEXT);
    let ((x0, mut y), right) = (ui.cursor(), ui.rect().x + ui.rect().w - PAD);
    let mut x = x0;
    for (i, name) in NAMES.into_iter().enumerate() {
        let w = (ui.text_system().measure(name, style) + 28.0).ceil();
        if x > x0 && x + w > right {
            (x, y) = (x0, y + BUTTON_H + SPACING);
        }
        ui.set_cursor(x, y);
        let pill = [Ui::button_primary, Ui::button][usize::from(i > 0)];
        let r = pill(ui, WidgetId(i as u32 + 1), name);
        x = r.x + r.w + SPACING;
    }
    ui.set_cursor(x0, y + BUTTON_H + SPACING);
}

const STACK: [(&str, &str); 18] = [
    ("wm", "Tiling window manager; deterministic and replayable"),
    ("shell", "Panel, window chrome and key bindings"),
    ("gfx", "Instanced-quad draw list and the WebGL2 shaders"),
    ("font", "TrueType parser and glyph rasterizer"),
    ("ui", "Text, widgets, the theme and the App trait"),
    ("apps", "Terminal, Welcome, Launcher and About"),
    ("studio", "Write, check and run apps"),
    ("vt", "Terminal escape-sequence parser"),
    ("term", "Terminal screen model"),
    ("vfs", "In-memory filesystem"),
    ("applang", "The tier 0 app language, with applang-syntax"),
    ("lang", "Diagnostics, lexer and parser kit"),
    ("fuel", "Fuel and byte budgets"),
    ("cap", "Capability tables: the syscall table"),
    ("wasmgen", "Wasm module builder"),
    ("platform", "The browser boundary: canvas, WebGL2, input"),
    ("os", "The wasm entry point"),
    ("node", "computehub-node: real shells for the terminal"),
];

/// The version, the stack, credits and provenance.
#[derive(Debug, Default)]
pub struct About {
    scroll: Scroll,
}

impl App for About {
    fn title(&self) -> String {
        "About".to_string()
    }

    fn draw(&mut self, ui: &mut Ui<'_>) {
        self.scroll.page(ui, |ui| {
            ui.heading("compusophyOS 0.1");
            ui.label("A tiling desktop OS in one browser canvas, and a platform for apps.");
            ui.subheading("The stack");
            for (name, role) in STACK {
                ui.key_value(name, role);
            }
            ui.subheading("Credits");
            ui.label(FONTS);
            ui.label(FORKS);
            ui.label("Author: compusophy");
        });
    }

    fn event(&mut self, ev: AppEvent, _cx: &mut Cx<'_>) -> bool {
        self.scroll.event(&ev)
    }
}
