//! AppHost: one applang program running in a window.

use applang::{App, Limits, Node as A};
use uiwire::{Event, Frame, MAX_DEPTH, Node, Request, Style, Variant};

use crate::{Disk, View, clip, file_name, problem};

/// Inputs are this plus their state's place in declaration order; buttons
/// are applang's ids plus one (0 is no id on the wire).
pub(crate) const INPUT: u32 = 1 << 31;
/// "Edit in Studio", shown when the file cannot run.
pub(crate) const EDIT: u32 = INPUT - 1;
/// The most of the app's own widgets a frame shows (a frame holds 4,096).
const MAX_WIDGETS: usize = 4000;
/// Shown, as an error, under a render that did not fit in a frame.
pub(crate) const TOO_BIG: &str =
    "renders more than 4,000 widgets or nests them deeper than 32; the rest is not shown";

#[derive(Debug, Default)]
enum Run {
    /// Not read yet.
    #[default]
    Pending,
    Live(App),
    /// Unreadable or not compiling: the problem, and its line with carets under it (or "").
    Broken(String, String),
}

/// Runs one `.app` file, compiled at its first event, with [`applang::App`]
/// and [`Limits::default`]; faults and files that cannot run show as errors.
#[derive(Debug, Default)]
pub struct AppHost {
    path: String,
    src: String,
    run: Run,
    nodes: Vec<A>,
    fault: Option<String>,
    requests: Vec<Request>,
}

impl AppHost {
    /// A host for the file at `path`, read at its first event.
    pub fn new(path: &str) -> AppHost {
        AppHost { path: path.to_string(), ..AppHost::default() }
    }

    fn load(&mut self, disk: &mut dyn Disk) {
        let src = match disk.read(&self.path) {
            Ok(src) => src,
            Err(e) => {
                let message = format!("error cannot read {}: {e}", self.path);
                self.run = Run::Broken(message, String::new());
                return;
            }
        };
        match applang::compile(&src) {
            Ok(program) => {
                self.run = Run::Live(App::new(program, Limits::default()));
                self.src = src;
                self.render();
            }
            Err(d) => {
                let snip = d.span.and_then(|s| lang::diag::render_snippet(&src, s));
                let snip = snip.as_deref().and_then(|s| s.split_once('\n')).map(|(_, s)| s);
                let snip = clip(snip.unwrap_or_default().into(), 4096);
                self.run = Run::Broken(problem(&d, &src), snip);
            }
        }
    }

    fn render(&mut self) {
        let Run::Live(app) = &self.run else { return };
        match app.render() {
            Ok(nodes) => self.nodes = nodes,
            Err(d) => {
                self.nodes.clear();
                self.fault = Some(problem(&d, &self.src));
            }
        }
    }

    fn fire(&mut self, ev: &applang::Event) {
        if let Run::Live(app) = &mut self.run {
            self.fault = app.handle(ev).err().map(|d| problem(&d, &self.src));
            self.render();
        }
    }

    /// The name of the state input `id` is bound to.
    fn state(&self, id: u32) -> Option<String> {
        let Run::Live(app) = &self.run else { return None };
        let i = usize::try_from(id.checked_sub(INPUT)?).ok()?;
        app.state().nth(i).map(|(name, _)| name.to_string())
    }
}

/// Puts `nodes` (at `depth`; `names` the app's states) into `out` as wire
/// nodes while they fit in `left` and [`MAX_DEPTH`]; whether all of them did.
fn wire(nodes: &[A], depth: usize, left: &mut usize, names: &[&str], out: &mut Vec<Node>) -> bool {
    for n in nodes {
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
                let id = id.checked_add(1).filter(|&id| id < EDIT).unwrap_or(0);
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
        if !all {
            return false;
        }
    }
    true
}

pub(crate) fn text(style: Style, text: &str) -> Node {
    Node::Text { id: 0, style, text: text.into() }
}

impl View for AppHost {
    fn event(&mut self, ev: Option<&Event>, disk: &mut dyn Disk) -> bool {
        let fresh = matches!(self.run, Run::Pending);
        if fresh {
            self.load(disk);
        }
        let live = matches!(self.run, Run::Live(_));
        let changed = match ev {
            Some(&Event::Click { id: EDIT }) if !live => {
                self.requests.push(Request::Open { name: ["studio:", &self.path].concat() });
                true
            }
            Some(&Event::Click { id }) if live && (1..EDIT).contains(&id) => {
                self.fire(&applang::Event::Click { id: id - 1 });
                true
            }
            // Every Change gets a frame: the desktop sends the next one then.
            Some(Event::Change { id, text, .. }) => {
                if let Some(state) = self.state(*id) {
                    self.fire(&applang::Event::Input { state, text: text.clone() });
                }
                true
            }
            _ => false,
        };
        changed || fresh
    }

    fn frame(&mut self) -> Frame {
        let mut nodes = Vec::new();
        match &self.run {
            Run::Pending => {}
            Run::Broken(problem, snippet) => {
                nodes.push(text(Style::Heading, &[file_name(&self.path), " cannot run"].concat()));
                nodes.push(text(Style::Error, problem));
                if !snippet.is_empty() {
                    nodes.push(text(Style::Mono, snippet));
                }
                let label = "Edit in Studio".into();
                nodes.push(Node::Button { id: EDIT, variant: Variant::Primary, label });
            }
            Run::Live(app) => {
                let names: Vec<&str> = app.state().map(|(name, _)| name).collect();
                if !wire(&self.nodes, 1, &mut { MAX_WIDGETS }, &names, &mut nodes) {
                    nodes.push(text(Style::Error, TOO_BIG));
                }
                if let Some(fault) = &self.fault {
                    nodes.push(text(Style::Error, fault));
                }
            }
        }
        let requests = std::mem::take(&mut self.requests);
        Frame { seq: 0, title: file_name(&self.path).into(), requests, nodes }
    }
}
