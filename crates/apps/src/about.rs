//! About: what compusophyOS is, what it is made of, and who made it.

use gfx::RectF;
use ui::{App, AppEvent, AppIcon, Cx, PAD, Ui};

use crate::kit::{self, Scroll};

const NAME: &str = "compusophy";
/// The version, and the build (`COMPUSOPHY_BUILD`, which scripts/build-web.sh sets).
const VERSION: &str = concat!("version ", env!("CARGO_PKG_VERSION"), " \u{b7} early beta");
const BUILD: &str = match option_env!("COMPUSOPHY_BUILD") {
    Some(b) => b,
    None => "dev",
};
const WHAT: &str = "A computer in one browser tab. Rust compiled to WebAssembly draws every \
pixel with the GPU: nothing to install, and your files stay in the tab. AI is built in and \
free while compusophy is in early beta.";
const FONTS: &str = "Inter, JetBrains Mono, and Noto Sans Symbols 1 and 2, under the SIL Open \
Font License 1.1; their license texts are at /licenses/ on the site.";
const FORKS: &str = "fuel, lang, applang-syntax and applang are forks of litelite 0.2.0 \
(commit 4f5e056, 2026-07-20), under Apache-2.0.";
const AUTHOR: &str = "Made by compusophy. Apache-2.0.";
const SOURCE: &str = "github.com/compusophy/computehub";
#[rustfmt::skip]
const STACK: [(&str, &str); 26] = [
    ("os", "The wasm entry: fonts, files, telemetry"),
    ("platform", "The browser boundary: canvas, WebGL2, input"),
    ("shell", "The desktop: header, dock, launcher, keys"),
    ("host", "Windows and the apps in them"), ("wm", "Window manager; deterministic"),
    ("apps", "Welcome, Files, Settings, About, Feedback, Terminal"),
    ("ui", "Widgets, themes and the App trait"), ("icons", "The mark and the vector icons"),
    ("text", "Fonts and glyphs on the atlas"), ("font", "TrueType reader and rasterizer"),
    ("gfx", "Draw lists and the WebGL2 shaders"), ("vfs", "In-memory filesystem; deterministic"),
    ("kernel", "Processes, consoles and files; deterministic"),
    ("wasi", "WASI for programs, in their worker"), ("cpu", "The program worker"),
    ("uiwire", "How GUI programs draw and hear"), ("studio", "Write, check and run apps"),
    ("assistant", "Ask anything; it builds apps"), ("guest", "The shell the Terminal runs"),
    ("term", "Terminal screen model"), ("vt", "Terminal escape-sequence parser"),
    ("applang", "The tier 0 app language"), ("applang-syntax", "applang's lexer and parser"),
    ("lang", "Diagnostics, lexer and parser kit"), ("fuel", "Fuel and byte budgets"),
    ("toolbox", "Test programs"),
];
/// The mark's side.
const MARK: f32 = 89.0;

/// About compusophyOS: the mark, the name, the version and build, what it is, the stack (each
/// crate and its role), credits and where the source lives; it scrolls.
#[derive(Debug, Default)]
pub struct About {
    scroll: Scroll,
}

impl App for About {
    fn title(&self) -> String {
        "About".to_string()
    }

    fn draw(&mut self, ui: &mut Ui<'_>) {
        let (r, t) = (ui.rect(), ui.theme());
        let (x, w) = (ui.text_system().snap(r.x + PAD), (r.w - 2.0 * PAD).max(0.0));
        let start = ui.text_system().snap(r.y + 34.0 - self.scroll.y);
        let at = ui.snapped(RectF::new(r.x + (r.w - MARK) / 2.0, start, MARK, MARK));
        kit::mark(ui, at, f64::INFINITY);
        let mut y = at.y + MARK + 21.0;
        y += kit::lines(ui, &[NAME], t.title(), (x, w), y, true) + 5.0;
        let version = [VERSION, " \u{b7} build ", BUILD].concat();
        y += kit::para(ui, &version, t.small(), (x, w), y, true) + 21.0;
        // The rest flows down the window's padded width.
        ui.set_cursor(x, y);
        ui.wrapped(WHAT, t.body().with_color(t.text_dim));
        ui.heading("The stack");
        stack(ui);
        ui.heading("Credits");
        ui.label(FONTS);
        ui.label(FORKS);
        ui.label(AUTHOR);
        ui.heading("Source");
        let end = ui.wrapped(SOURCE, t.mono().with_color(t.accent));
        self.scroll.measure(end.y + end.h - start + 34.0 + 34.0, r.h);
        self.scroll.thumb(ui, r);
    }

    fn event(&mut self, ev: AppEvent, _: &mut Cx<'_>) -> bool {
        match ev {
            AppEvent::Wheel { dy, .. } => self.scroll.wheel(dy),
            AppEvent::Resized { .. } => true,
            _ => false,
        }
    }

    fn preferred_size(&self) -> Option<(f32, f32)> {
        Some((560.0, 640.0))
    }

    fn icon(&self) -> AppIcon {
        kit::ABOUT
    }

    fn compact(&self) -> bool {
        true
    }
}

/// The stack as a table: crate names in dim mono, roles beside them (under them when narrow).
fn stack(ui: &mut Ui<'_>) {
    let t = ui.theme();
    let (key, val) = (t.mono().with_color(t.text_dim), t.body());
    let ((x, mut y), w) = (ui.cursor(), ui.width());
    let ts = ui.text_system();
    let widest = STACK.iter().map(|(k, _)| ts.measure(k, key)).fold(0.0, f32::max);
    let (lh, a, d) = (ts.line_height(val), ts.ascent(val), ts.descent(val));
    let kw = (widest + 21.0).ceil();
    let beside = w >= kw + 233.0;
    for (name, role) in STACK {
        let base = ui.text_system().snap(y + (lh - a - d) / 2.0 + a);
        ui.text(x, base, name, key);
        let (vx, vy) = if beside { (x + kw, y) } else { (x, y + lh) };
        y = vy + kit::para(ui, role, val, (vx, x + w - vx), vy, false) + 5.0;
    }
    ui.advance_to(y - 5.0);
}
