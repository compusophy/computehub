//! About: what compusophyOS is, what it is made of, and who made it.

use icons::Glyph;
use uiwire::{Event, Frame, Node, Style};

use crate::{Disk, View, space, text};

const NAME: &str = "compusophy";
/// The version, and the build (`COMPUSOPHY_BUILD`, which scripts/build-web.sh sets).
pub(crate) const VERSION: &str =
    concat!("version ", env!("CARGO_PKG_VERSION"), " \u{b7} early beta");
pub(crate) const BUILD: &str = match option_env!("COMPUSOPHY_BUILD") {
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
/// Every crate and its role.
#[rustfmt::skip]
pub(crate) const STACK: [(&str, &str); 40] = [
    ("os", "The wasm entry: fonts, files, program windows"),
    ("report", "Telemetry: notes, reports and their outbox"),
    ("platform", "The browser boundary: canvas, WebGL2, input"),
    ("logon", "The welcome: the mark, the start's record, sign-in"),
    ("profiles", "Profiles: their keys, faces, names and PINs"),
    ("shell", "The desktop: top bar, windows, keys"), ("home", "The home screen, dock and touch"),
    ("host", "Windows and the apps in them"), ("wm", "Window manager; deterministic"),
    ("apps", "The console a terminal's shell runs on"),
    ("system", "About, Editor, Feedback, Files, Welcome, Settings; serves Activity"),
    ("activity", "The resource monitor: CPU, memory, frames, AI"),
    ("ui", "Widgets, themes and the App trait"), ("icons", "The mark and the vector icons"),
    ("text", "Fonts and glyphs on the atlas"), ("font", "TrueType reader and rasterizer"),
    ("gfx", "Draw lists and the WebGL2 shaders"), ("vfs", "In-memory filesystem; deterministic"),
    ("kernel", "Processes, consoles and files; deterministic"),
    ("wasi", "WASI for programs, in their worker"), ("cpu", "The program worker"),
    ("uiwire", "How GUI programs draw and hear"), ("uiview", "How the desktop draws them"),
    ("canvas", "How it draws their canvases: shapes and pixels"),
    ("studio", "Write, check and run apps"), ("coder", "The AI that writes and fixes apps"),
    ("evals", "Measured gains: runs, records, replays"),
    ("makes", "Evals' Suite 1: apps made, driven, graded"),
    ("assistant", "Ask; it does it on the desktop"), ("chats", "The Assistant's chats, kept"),
    ("files", "The Assistant's file tools: yours, from your home"),
    ("terminal", "The Terminal: an xterm screen of the shell"),
    ("sh", "The shell the Terminal runs"), ("term", "Terminal screen model"),
    ("vt", "Terminal escape-sequence parser"), ("applang", "The tier 0 app language"),
    ("applang-syntax", "applang's lexer and parser"), ("lang", "Diagnostics, lexer and parser kit"),
    ("fuel", "Fuel and byte budgets"), ("toolbox", "Test programs"),
];
/// The mark's side; the width of the stack's name column, and the least width with the roles
/// beside the names (else under them).
const MARK: u16 = 89;
const NAMES_W: u16 = 144;
const BESIDE: u16 = NAMES_W + 233;

/// About compusophyOS: the mark, the name, the version and build, what it is, the stack (each
/// crate and its role, beside its name or, narrow, under it), credits and where the source lives.
#[derive(Debug, Default)]
pub struct About {
    /// The content width, as the last Resize said; whether a frame went yet.
    width: u16,
    framed: bool,
}

impl View for About {
    fn event(&mut self, ev: &Event, _: &mut dyn Disk) -> bool {
        let Event::Resize { w, .. } = *ev else { return false };
        // The layout changes only when the roles move beside the names or under them.
        let wide = |w: u16| w.saturating_sub(40) >= BESIDE;
        let changed = !self.framed || wide(w) != wide(self.width);
        self.width = w;
        changed
    }

    fn frame(&mut self) -> Frame {
        let head = vec![
            space(14),
            Node::Glyph { glyph: Glyph::Mark as u8, size: MARK },
            space(21),
            text(Style::Title, NAME),
            space(5),
            text(Style::Small, &[VERSION, " \u{b7} build ", BUILD].concat()),
        ];
        let mut nodes = vec![Node::Center { id: 0, gap: 0, children: head }, space(13)];
        nodes.extend([text(Style::Dim, WHAT), space(12), text(Style::Heading, "The stack")]);
        let beside = self.width.saturating_sub(40) >= BESIDE;
        let stack = STACK.iter().map(|&(name, role)| {
            let (name, role) = (text(Style::Mono, name), text(Style::Dim, role));
            match beside {
                true => {
                    let names = Node::Pane { id: 0, w: NAMES_W, children: vec![name] };
                    Node::Row { id: 0, gap: 0, children: vec![names, role] }
                }
                false => Node::Col { id: 0, gap: 0, children: vec![name, role] },
            }
        });
        nodes.push(Node::Col { id: 0, gap: 5, children: stack.collect() });
        nodes.extend([space(12), text(Style::Heading, "Credits")]);
        nodes.extend([FONTS, FORKS, AUTHOR].map(|t| text(Style::Body, t)));
        nodes.extend([space(12), text(Style::Heading, "Source"), text(Style::Accent, SOURCE)]);
        nodes.push(space(14));
        self.framed = true;
        Frame { seq: 0, title: "About".into(), requests: Vec::new(), nodes }
    }
}
