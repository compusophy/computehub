//! One applang program running, as wire nodes and events ([`Live`]); and [`AppHost`], one `.app`
//! file running in a window.

use crate::{Disk, View, file_name, text};
use applang::{App, Limits, Node as A};
use assistant::ai::{HOME, clip, problem};
use uiwire::{Event, Frame, Key, MAX_DEPTH, Node, Request, Style, Variant, mods};

/// The app's buttons and grids are this plus applang's ids, its inputs [`INPUT`] plus their
/// state's place in declaration order: ids below [`APP`] are the window's own.
pub const APP: u32 = 1 << 30;
pub const INPUT: u32 = 1 << 31;
/// "Edit in Studio", shown when the file cannot run.
pub(crate) const EDIT: u32 = 1;
/// The most of the app's own widgets a frame shows (a frame holds 4,096).
const MAX_WIDGETS: usize = 4000;
/// Shown, as an error, under a render that did not fit in a frame.
pub(crate) const TOO_BIG: &str =
    "renders more than 4,000 widgets or nests them deeper than 32; the rest is not shown";

/// A program compiled and running with [`Limits::default`], or why it cannot run; a fault
/// (a handler or render that failed), or what its saved states said as they came back, shows
/// under it until the next event. Its `saved` states live in `~/.appdata/<name>.state` (its
/// file's name), read when it starts and written when they change.
#[derive(Debug)]
pub struct Live {
    src: String,
    /// The app, or its problem and the problem's line with carets under it (or "").
    app: Result<App, (String, String)>,
    nodes: Vec<A>,
    fault: Option<String>,
    /// Where its saved states go ("": nowhere), and what was last written there.
    state: String,
    saved: String,
}

/// The file the saved states of the app at `path` go to ("" for none).
pub fn state_path(path: &str) -> String {
    let name = file_name(path);
    let stem = name.strip_suffix(".app").unwrap_or(name);
    if stem.is_empty() { String::new() } else { [HOME, "/.appdata/", stem, ".state"].concat() }
}

/// A seed for `random`: the clock's nanoseconds, so each run deals afresh.
fn seed() -> u64 {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    now.map_or(1, |d| d.as_nanos() as u64)
}

impl Live {
    /// `src` (the app at `path`, "" for none) compiled and started, its saved states back.
    pub fn new(src: &str, path: &str, disk: &mut dyn Disk) -> Live {
        let app = applang::compile(src).map(|p| App::new(p, Limits::default(), seed()));
        let app = app.map_err(|d| {
            let snip = d.span.and_then(|s| lang::diag::render_snippet(src, s));
            let snip = snip.as_deref().and_then(|s| s.split_once('\n')).map(|(_, s)| s);
            (problem(&d, src), clip(snip.unwrap_or_default(), 4096))
        });
        let state = state_path(path);
        let mut live = Live {
            src: src.into(),
            app,
            nodes: Vec::new(),
            fault: None,
            state,
            saved: String::new(),
        };
        if let Ok(app) = &mut live.app {
            live.saved = app.saved();
            let back = (!live.saved.is_empty()).then(|| disk.read(&live.state).ok()).flatten();
            let notes = back.map(|text| app.restore(&text)).unwrap_or_default();
            live.fault = (!notes.is_empty()).then(|| notes.join("; "));
            live.saved = app.saved();
        }
        live.render();
        live
    }

    /// The file at `path` running, or why it cannot be read.
    pub fn load(path: &str, disk: &mut dyn Disk) -> Live {
        disk.read(path).map_or_else(
            |e| Live {
                src: String::new(),
                app: Err((format!("error cannot read {path}: {e}"), String::new())),
                nodes: Vec::new(),
                fault: None,
                state: String::new(),
                saved: String::new(),
            },
            |src| Live::new(&src, path, disk),
        )
    }

    /// Whether the program compiled.
    pub fn runs(&self) -> bool {
        self.app.is_ok()
    }

    /// What it wants of the window: a tick every so many ms (0: none), and plain keys.
    pub fn play(&mut self) -> (u32, bool) {
        self.app.as_mut().map_or((0, false), |app| (app.timer(), app.keys()))
    }

    fn render(&mut self) {
        let Ok(app) = &mut self.app else { return };
        match app.render() {
            Ok(nodes) => self.nodes = nodes,
            Err(d) => (self.nodes, self.fault) = (Vec::new(), Some(problem(&d, &self.src))),
        }
    }

    /// Runs `ev` if it is the app's (a click of its button, a tap of its grid, a change of its
    /// input, a tick, a key it may handle); whether the window changed (a tick always answers,
    /// so the desktop sends the next).
    pub fn event(&mut self, ev: &Event, disk: &mut dyn Disk) -> bool {
        let Ok(app) = &mut self.app else { return false };
        let ev = match ev {
            &Event::Click { id } if (APP..INPUT).contains(&id) => {
                applang::Event::Click { id: id - APP }
            }
            &Event::Tap { id, cell } if (APP..INPUT).contains(&id) => {
                applang::Event::Tap { id: id - APP, cell }
            }
            &Event::Tick { ms } => applang::Event::Tick { ms },
            // Chords are the window's own.
            &Event::Key { key, mods, ch, .. } if mods & !mods::SHIFT == 0 => {
                match key_name(key, ch) {
                    Some(name) => applang::Event::Key { name },
                    None => return false,
                }
            }
            Event::Change { id, text, .. } => {
                let i = id.checked_sub(INPUT).and_then(|i| usize::try_from(i).ok());
                let state = i.and_then(|i| app.state().nth(i)).map(|(n, _)| n.to_string());
                let Some(state) = state else { return false };
                applang::Event::Input { state, text: text.clone() }
            }
            _ => return false,
        };
        let tick = matches!(ev, applang::Event::Tick { .. });
        let ran = app.handle(&ev);
        if let Ok(false) = ran {
            return tick;
        }
        self.fault = ran.err().map(|d| problem(&d, &self.src));
        let saved = app.saved();
        if saved != self.saved && !self.state.is_empty() {
            match disk.write(&self.state, &saved) {
                Ok(()) => self.saved = saved,
                Err(e) => self.fault = Some(format!("its saved state was not kept: {e}")),
            }
        }
        self.render();
        true
    }

    /// The app as wire nodes, the outermost at `depth`: its widgets while they fit in a frame,
    /// then its fault; or its problem and the problem's line.
    pub fn nodes(&self, depth: usize) -> Vec<Node> {
        let mut out = Vec::new();
        match &self.app {
            Err((problem, snippet)) => {
                out.push(text(Style::Error, problem));
                out.extend((!snippet.is_empty()).then(|| text(Style::Mono, snippet)));
            }
            Ok(app) => {
                let names: Vec<&str> = app.state().map(|(name, _)| name).collect();
                if !wire(&self.nodes, depth, &mut { MAX_WIDGETS }, &names, &mut out) {
                    out.push(text(Style::Error, TOO_BIG));
                }
                out.extend(self.fault.as_deref().map(|f| text(Style::Error, f)));
            }
        }
        out
    }
}

/// The name an `on key` handler gives `key` (`ch` for a character key), if it has one.
fn key_name(key: Key, ch: char) -> Option<String> {
    let name = match key {
        Key::Left => "left",
        Key::Right => "right",
        Key::Up => "up",
        Key::Down => "down",
        Key::Enter => "enter",
        Key::Escape => "escape",
        Key::Char if ch == ' ' => "space",
        Key::Char if ch.is_ascii_alphanumeric() => return Some(ch.to_ascii_lowercase().into()),
        _ => return None,
    };
    Some(name.into())
}

/// Asks for the timer and keys of `now` where they differ from those `asked` before.
pub(crate) fn ask(asked: &mut (u32, bool), now: (u32, bool), out: &mut Vec<Request>) {
    let was = std::mem::replace(asked, now);
    out.extend((was.0 != now.0).then_some(Request::Timer { ms: now.0 }));
    out.extend((was.1 != now.1).then_some(Request::Keys { on: now.1 }));
}

/// Puts `nodes` (at `depth`; `names` the app's states) into `out` as wire
/// nodes while they fit in `left` and [`MAX_DEPTH`]; whether all of them did.
fn wire(nodes: &[A], depth: usize, left: &mut usize, names: &[&str], out: &mut Vec<Node>) -> bool {
    let app = |id: u32| APP.checked_add(id).filter(|&id| id < INPUT).unwrap_or(0);
    nodes.iter().all(|n| {
        let kids = match n {
            A::Row { children } | A::Col { children } => children.as_slice(),
            _ => &[],
        };
        if *left == 0 || (depth >= MAX_DEPTH && !kids.is_empty()) {
            return false;
        }
        *left -= 1;
        let mut children = Vec::new();
        let all = wire(kids, depth + 1, left, names, &mut children);
        out.push(match n {
            A::Label { text } => Node::Text { id: 0, style: Style::Body, text: text.clone() },
            A::Button { text, id } => {
                Node::Button { id: app(*id), variant: Variant::Normal, label: text.clone() }
            }
            A::Input { state, value } => {
                let i = names.iter().position(|n| n == state).and_then(|i| u32::try_from(i).ok());
                let id = i.and_then(|i| INPUT.checked_add(i)).unwrap_or(0);
                Node::Input { id, value: value.clone(), placeholder: state.clone() }
            }
            A::Row { .. } => Node::Row { id: 0, gap: 8, children },
            A::Col { .. } => Node::Col { id: 0, gap: 8, children },
            A::Grid { id, cols, cells, texts } => {
                let (cells, texts) = (cells.clone(), texts.clone());
                Node::Grid { id: id.map_or(0, app), cols: *cols, cells, texts }
            }
        });
        all
    })
}

/// Runs one `.app` file, read at its first event, in a window of its own; a file that cannot
/// run says why and offers Studio.
#[derive(Debug, Default)]
pub struct AppHost {
    path: String,
    live: Option<Live>,
    requests: Vec<Request>,
    /// The timer and keys asked for.
    asked: (u32, bool),
}

impl AppHost {
    /// A host for the file at `path`, read at its first event.
    pub fn new(path: &str) -> AppHost {
        AppHost { path: path.to_string(), ..AppHost::default() }
    }
}

impl View for AppHost {
    fn event(&mut self, ev: Option<&Event>, disk: &mut dyn Disk) -> bool {
        let fresh = self.live.is_none();
        let live = self.live.get_or_insert_with(|| Live::load(&self.path, disk));
        match ev {
            Some(&Event::Click { id: EDIT }) if !live.runs() => {
                self.requests.push(Request::Open { name: ["studio:", &self.path].concat() })
            }
            // Every Change gets a frame: the desktop sends the next one then.
            Some(ev @ Event::Change { .. }) => _ = live.event(ev, disk),
            Some(ev) if live.event(ev, disk) => {}
            _ => return fresh,
        }
        true
    }

    fn frame(&mut self) -> Frame {
        let mut nodes = Vec::new();
        let mut requests = std::mem::take(&mut self.requests);
        if let Some(live) = &mut self.live {
            let name = file_name(&self.path);
            let broken = !live.runs();
            nodes.extend(broken.then(|| text(Style::Heading, &[name, " cannot run"].concat())));
            nodes.extend(live.nodes(1));
            let edit = || Node::Button {
                id: EDIT,
                variant: Variant::Primary,
                label: "Edit in Studio".into(),
            };
            nodes.extend(broken.then(edit));
            ask(&mut self.asked, live.play(), &mut requests);
        }
        Frame { seq: 0, title: file_name(&self.path).into(), requests, nodes }
    }
}
