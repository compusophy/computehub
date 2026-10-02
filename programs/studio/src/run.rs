//! One applang program running, as wire nodes and events ([`Live`]); and [`AppHost`], one `.app`
//! file running in a window.

use crate::{Disk, View, file_name, text};
use applang::{App, Limits, Node as A};
use assistant::ai::{clip, problem, shown, state_path};
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
/// file's name: [`state_path`]), read when it starts and written when they change. Other runs
/// of it (its windows, Studio's preview) share that file: what they wrote comes back before
/// each event but a tick, and a write keeps their states it did not change. States it cannot
/// start from (its first render faults with them) it starts without, keeping nothing, so the
/// file waits for a program that runs from it; that shows under it while it runs.
#[derive(Debug)]
pub struct Live {
    src: String,
    /// The app, or its problem and the problem's line with carets under it (or "").
    app: Result<App, (String, String)>,
    nodes: Vec<A>,
    fault: Option<String>,
    /// Whether its first render faulted, from its saved states or not; and why it started
    /// without them, if it did.
    failed: bool,
    afresh: Option<String>,
    /// Where its saved states go ("": nowhere); its own lines as last read or written there (to
    /// see what an event changed), and the file's text then (to see what another run wrote).
    state: String,
    saved: String,
    kept: String,
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
        let mut live = Live::stopped(src, app);
        // Only an app with saved states keeps a file.
        if let Some(app) = live.app.as_ref().ok().filter(|app| !app.saved().is_empty()) {
            (live.saved, live.state) = (app.saved(), state_path(path));
        }
        live.sync(disk);
        live.failed = !live.render();
        if live.failed && !live.kept.is_empty() {
            // The states kept make it fault (a change put a list beside one kept, say): it runs
            // without them, if it can, and keeps nothing over them.
            let fresh = Live::new(src, "", disk);
            if !fresh.failed {
                let why = live.fault.unwrap_or_default();
                let at = shown(&live.state);
                let note = format!(
                    "{why}, from the states kept in {at}: it started afresh, and keeps nothing \
                     until it runs from them"
                );
                live = Live { afresh: Some(note), failed: true, ..fresh };
            }
        }
        live
    }

    /// `app` (compiled from `src`, or its problem) before it starts: no saved states, nothing
    /// shown.
    fn stopped(src: &str, app: Result<App, (String, String)>) -> Live {
        let (nodes, fault, failed, afresh) = (Vec::new(), None, false, None);
        let (state, saved, kept) = (String::new(), String::new(), String::new());
        Live { src: src.into(), app, nodes, fault, failed, afresh, state, saved, kept }
    }

    /// The file at `path` running, or why it cannot be read.
    pub fn load(path: &str, disk: &mut dyn Disk) -> Live {
        disk.read(path).map_or_else(
            |e| Live::stopped("", Err((format!("error cannot read {path}: {e}"), String::new()))),
            |src| Live::new(&src, path, disk),
        )
    }

    /// Whether its first render faulted: it cannot start, or not from the states it kept (it
    /// started without them).
    pub fn faults(&self) -> bool {
        self.failed
    }

    /// Whether the program compiled.
    pub fn runs(&self) -> bool {
        self.app.is_ok()
    }

    /// What it wants of the window: a tick every so many ms (0: none), and plain keys.
    pub fn play(&mut self) -> (u32, bool) {
        self.app.as_mut().map_or((0, false), |app| (app.timer(), app.keys()))
    }

    /// Renders the app; whether it did (else its fault shows).
    fn render(&mut self) -> bool {
        let Ok(app) = &mut self.app else { return false };
        let nodes = app.render().map_err(|d| problem(&d, &self.src));
        let done = nodes.is_ok();
        match nodes {
            Ok(nodes) => self.nodes = nodes,
            Err(fault) => (self.nodes, self.fault) = (Vec::new(), Some(fault)),
        }
        done
    }

    /// Takes back the saved states the file holds if another run wrote them since this one
    /// last read or wrote it (at the start, if any are kept); whether they came back. What they
    /// said as they came back (a type that changed) is its fault now.
    fn sync(&mut self, disk: &mut dyn Disk) -> bool {
        let Ok(app) = &mut self.app else { return false };
        let text = (!self.state.is_empty()).then(|| disk.read(&self.state).ok()).flatten();
        let Some(text) = text.filter(|text| *text != self.kept) else { return false };
        let notes = app.restore(&text);
        self.fault = (!notes.is_empty()).then(|| notes.join("; "));
        (self.saved, self.kept) = (app.saved(), text);
        true
    }

    /// Writes the saved states if an event changed them, over what the file holds now: another
    /// run's states this one did not change, or does not know (another version's), stay and
    /// come in, unless this program cannot read them back.
    fn keep(&mut self, disk: &mut dyn Disk) {
        let Ok(app) = &mut self.app else { return };
        let mine = app.saved();
        if mine == self.saved || self.state.is_empty() {
            return;
        }
        let mut text = merge(&self.saved, &mine, &disk.read(&self.state).unwrap_or_default());
        if text != mine && !app.restore(&text).is_empty() {
            app.restore(&mine);
            text.clone_from(&mine);
        }
        match disk.write(&self.state, &text) {
            Ok(()) => (self.saved, self.kept) = (app.saved(), text),
            Err(e) => self.fault = Some(format!("its saved state was not kept: {e}")),
        }
    }

    /// Runs `ev` if it is the app's (a click of its button, a tap of its grid, a change of its
    /// input, a tick, a key it may handle while no text field has the keyboard) after taking
    /// back what other runs kept (a tick does not wait for that: ticks come too often, and a
    /// write merges); whether the window changed (a tick always answers, so the desktop sends
    /// the next). Its window gaining the keyboard is a time to take that back too.
    pub fn event(&mut self, ev: &Event, disk: &mut dyn Disk) -> bool {
        if let Event::Focus { on: true } = ev {
            let back = self.sync(disk);
            if back {
                self.render();
            }
            return back;
        }
        let Ok(app) = &mut self.app else { return false };
        let ev = match ev {
            &Event::Click { id } if (APP..INPUT).contains(&id) => {
                applang::Event::Click { id: id - APP }
            }
            &Event::Tap { id, cell } if (APP..INPUT).contains(&id) => {
                applang::Event::Tap { id: id - APP, cell }
            }
            &Event::Tick { ms } => applang::Event::Tick { ms },
            // Escape leaving a text field is the field's, and chords are the window's own.
            &Event::Key { id: 0, key, mods, ch } if mods & !mods::SHIFT == 0 => {
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
        let synced = !tick && self.sync(disk);
        let Ok(app) = &mut self.app else { return false };
        let ran = app.handle(&ev);
        if let Ok(false) = ran {
            if synced {
                self.render();
            }
            return tick || synced;
        }
        // What came back says so, unless the event faulted.
        if !synced || ran.is_err() {
            self.fault = ran.err().map(|d| problem(&d, &self.src));
        }
        self.keep(disk);
        self.render();
        true
    }

    /// The app as wire nodes, the outermost at `depth`: its widgets while they fit in a frame,
    /// then its fault and why it started afresh; or its problem and the problem's line.
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
                let notes = [&self.fault, &self.afresh].map(Option::as_deref);
                out.extend(notes.into_iter().flatten().map(|f| text(Style::Error, f)));
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

/// `mine` (an app's saved lines, `name = literal;` each) over `file` (what its file holds now):
/// its line for each state it changed since `before`, the file's for the rest (another run may
/// have changed them), then the file's lines for states it does not save.
fn merge(before: &str, mine: &str, file: &str) -> String {
    /// The line of `text` that names the state `line` does, if any.
    fn find<'a>(text: &'a str, line: &str) -> Option<&'a str> {
        fn name(line: &str) -> &str {
            line.split_once('=').map_or("", |(name, _)| name.trim())
        }
        text.lines().find(|l| !name(l).is_empty() && name(l) == name(line))
    }
    let mut out = String::new();
    for line in mine.lines() {
        let theirs = find(file, line).filter(|_| before.lines().any(|b| b == line));
        out = out + theirs.unwrap_or(line) + "\n";
    }
    for line in file.lines() {
        if find(mine, line).is_none() && !line.trim().is_empty() {
            out = out + line + "\n";
        }
    }
    out
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
/// run, or faults as it starts, says why and offers Studio.
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
            Some(&Event::Click { id: EDIT }) if !live.runs() || live.faults() => {
                self.requests.push(Request::Open { name: ["studio:", &self.path].concat() })
            }
            // Every Change gets a frame: the desktop sends the next one then.
            Some(ev @ Event::Change { .. }) => _ = live.event(ev, disk),
            Some(ev) if live.event(ev, disk) => {
                // A press on an app that takes keys gives it the keyboard, as in Studio.
                if matches!(ev, Event::Click { .. } | Event::Tap { .. }) && self.asked.1 {
                    self.requests.push(Request::Focus { id: 0 });
                }
            }
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
            nodes.extend((broken || live.faults()).then(edit));
            ask(&mut self.asked, live.play(), &mut requests);
        }
        Frame { seq: 0, title: file_name(&self.path).into(), requests, nodes }
    }
}
