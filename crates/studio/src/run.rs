//! One applang program running, as wire nodes and events ([`Live`]); and [`AppHost`], one `.app`
//! file running in a window.

use crate::{Disk, View, file_name, text};
use applang::{App, Limits, Node as A};
use assistant::ai::{clip, problem};
use uiwire::{Event, Frame, MAX_DEPTH, Node, Request, Style, Variant};

/// The app's buttons are this plus applang's ids, its inputs [`INPUT`] plus their state's place
/// in declaration order: ids below [`APP`] are the window's own.
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
/// (a handler or render that failed) shows as an error under it until the next event.
#[derive(Debug)]
pub struct Live {
    src: String,
    /// The app, or its problem and the problem's line with carets under it (or "").
    app: Result<App, (String, String)>,
    nodes: Vec<A>,
    fault: Option<String>,
}

impl Live {
    /// `src` compiled and started.
    pub fn new(src: &str) -> Live {
        let app = applang::compile(src).map(|p| App::new(p, Limits::default())).map_err(|d| {
            let snip = d.span.and_then(|s| lang::diag::render_snippet(src, s));
            let snip = snip.as_deref().and_then(|s| s.split_once('\n')).map(|(_, s)| s);
            (problem(&d, src), clip(snip.unwrap_or_default(), 4096))
        });
        let mut live = Live { src: src.into(), app, nodes: Vec::new(), fault: None };
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
            },
            |src| Live::new(&src),
        )
    }

    /// Whether the program compiled.
    pub fn runs(&self) -> bool {
        self.app.is_ok()
    }

    fn render(&mut self) {
        let Ok(app) = &self.app else { return };
        match app.render() {
            Ok(nodes) => self.nodes = nodes,
            Err(d) => (self.nodes, self.fault) = (Vec::new(), Some(problem(&d, &self.src))),
        }
    }

    /// Runs `ev` if it is the app's (a click of its button, a change of its input); whether it was.
    pub fn event(&mut self, ev: &Event) -> bool {
        let Ok(app) = &mut self.app else { return false };
        let ev = match ev {
            &Event::Click { id } if (APP..INPUT).contains(&id) => {
                applang::Event::Click { id: id - APP }
            }
            Event::Change { id, text, .. } => {
                let i = id.checked_sub(INPUT).and_then(|i| usize::try_from(i).ok());
                let state = i.and_then(|i| app.state().nth(i)).map(|(n, _)| n.to_string());
                let Some(state) = state else { return false };
                applang::Event::Input { state, text: text.clone() }
            }
            _ => return false,
        };
        self.fault = app.handle(&ev).err().map(|d| problem(&d, &self.src));
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

/// Puts `nodes` (at `depth`; `names` the app's states) into `out` as wire
/// nodes while they fit in `left` and [`MAX_DEPTH`]; whether all of them did.
fn wire(nodes: &[A], depth: usize, left: &mut usize, names: &[&str], out: &mut Vec<Node>) -> bool {
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
                let id = APP.checked_add(*id).filter(|&id| id < INPUT).unwrap_or(0);
                Node::Button { id, variant: Variant::Normal, label: text.clone() }
            }
            A::Input { state, value } => {
                let i = names.iter().position(|n| n == state).and_then(|i| u32::try_from(i).ok());
                let id = i.and_then(|i| INPUT.checked_add(i)).unwrap_or(0);
                Node::Input { id, value: value.clone(), placeholder: state.clone() }
            }
            A::Row { .. } => Node::Row { id: 0, gap: 8, children },
            A::Col { .. } => Node::Col { id: 0, gap: 8, children },
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
            Some(ev @ Event::Change { .. }) => _ = live.event(ev),
            Some(ev) if live.event(ev) => {}
            _ => return fresh,
        }
        true
    }

    fn frame(&mut self) -> Frame {
        let mut nodes = Vec::new();
        if let Some(live) = &self.live {
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
        }
        let requests = std::mem::take(&mut self.requests);
        Frame { seq: 0, title: file_name(&self.path).into(), requests, nodes }
    }
}
