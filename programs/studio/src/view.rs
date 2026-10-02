//! Studio's frames. With nothing open: a centered prompt and example chips. With an app: a
//! header (its name; Code or Preview, Open in window, New), the app running in a card (or its
//! code), and the prompt with the line saying how the make goes and the prompts made so far:
//! beside the app when the window is wide, under it (the prompt last) when it is narrow.

use crate::edit::{CHECK, CHIP, MAKE, NEW, OPEN, STOP, TOGGLE, spans};
use crate::make::EXAMPLES;
use crate::{Studio, file_name, text};
use uiwire::{Frame, Node, Request, Style, Variant};

/// Narrower than this, the window stacks.
const NARROW: u16 = 600;
/// The side column's width beside the app, and the centered prompt's.
const SIDE: u16 = 280;
const CENTER: u16 = 520;
/// The size Studio asks for (the desktop opens it at this size too: os::remote).
const SIZE: (u16, u16) = (880, 560);

fn button(id: u32, variant: Variant, label: &str) -> Node {
    Node::Button { id, variant, label: label.into() }
}

fn row(gap: u8, children: Vec<Node>) -> Node {
    Node::Row { id: 0, gap, children }
}

/// Room that takes the rest: of a Row's width (an empty Col), of the window's height (a Fill).
fn flex() -> Node {
    Node::Col { id: 0, gap: 0, children: Vec::new() }
}

fn fill(children: Vec<Node>) -> Node {
    Node::Fill { id: 0, children }
}

impl Studio {
    pub(crate) fn narrow(&self) -> bool {
        self.width < NARROW
    }

    pub(crate) fn draw(&mut self) -> Frame {
        let (nodes, title) = match self.path.is_empty() {
            true => (self.start(), "Studio".into()),
            false => (self.working(), ["Studio \u{2014} ", file_name(&self.path)].concat()),
        };
        let mut requests = std::mem::take(&mut self.requests);
        if !std::mem::replace(&mut self.framed, true) {
            requests.insert(0, Request::Size { w: SIZE.0, h: SIZE.1 });
        }
        Frame { seq: 0, title, requests, nodes }
    }

    /// The prompt and Make, or Stop while a make runs.
    fn prompt_bar(&self, placeholder: &str) -> Node {
        let (id, value) = (self.prompt_id(), self.prompt.clone());
        let input = Node::Input { id, value, placeholder: placeholder.into() };
        let action = match self.make {
            Some(_) => button(STOP, Variant::Normal, "Stop"),
            None => button(MAKE, Variant::Primary, "Make"),
        };
        row(8, vec![input, action])
    }

    /// The status line, then the prompts made, newest first.
    fn notes(&self) -> Vec<Node> {
        let status = (!self.status.1.is_empty()).then(|| text(self.status.0, &self.status.1));
        let made = self
            .made
            .iter()
            .rev()
            .map(|p| text(Style::Small, &["\u{201c}", p, "\u{201d}"].concat()));
        status.into_iter().chain(made).collect()
    }

    /// Nothing open: what to make, centered.
    fn start(&self) -> Vec<Node> {
        let mut block =
            vec![text(Style::Title, "What do you want to make?"), Node::Spacer { px: 4 }];
        block.push(self.prompt_bar("Describe an app\u{2026}"));
        if self.make.is_none() {
            let chip = |i: usize| button(CHIP + i as u32, Variant::Chip, EXAMPLES[i]);
            block.extend([row(8, vec![chip(0), chip(1)]), row(8, vec![chip(2), chip(3)])]);
        }
        block.extend(self.notes());
        let block = Node::Pane { id: 0, w: CENTER, children: block };
        // A little above the middle reads as centered.
        vec![
            fill(Vec::new()),
            row(0, vec![flex(), block, flex()]),
            fill(Vec::new()),
            Node::Spacer { px: 40 },
        ]
    }

    /// An app open: its header, the app or its code, the prompt and the notes.
    fn working(&self) -> Vec<Node> {
        let name = text(Style::Heading, &name(&self.path));
        let toggle = button(TOGGLE, Variant::Normal, if self.code { "Preview" } else { "Code" });
        let actions = [
            toggle,
            button(OPEN, Variant::Normal, "Open in window"),
            button(NEW, Variant::Normal, "New"),
        ];
        let narrow = self.narrow();
        let main = self.main(if narrow { 2 } else { 3 });
        if !narrow {
            let header = row(8, [vec![name, flex()], actions.into()].concat());
            let side = [vec![self.prompt_bar("Describe a change\u{2026}")], self.notes()].concat();
            let side = Node::Pane { id: 0, w: SIDE, children: side };
            return vec![header, row(16, vec![main, side])];
        }
        let mut nodes = vec![name, row(8, actions.into()), main];
        // The app keeps its height; the prompt goes to the bottom.
        nodes.extend((!self.code).then(|| fill(Vec::new())));
        nodes.extend(self.notes());
        nodes.push(self.prompt_bar("Describe a change\u{2026}"));
        nodes
    }

    /// The app running in a card (its widgets at `depth`), or its code with Check under it.
    fn main(&self, depth: usize) -> Node {
        if self.code {
            let (spans, text) = (spans(&self.text, self.mark), self.text.clone());
            let code = Node::Code {
                id: self.code_id(),
                version: self.version,
                line_numbers: true,
                text,
                spans,
            };
            let hint = crate::text(Style::Small, "Ctrl+Enter");
            let check = row(8, vec![button(CHECK, Variant::Normal, "Check"), hint]);
            return Node::Col { id: 0, gap: 8, children: vec![fill(vec![code]), check] };
        }
        let mut app = self.live.as_ref().map(|live| live.nodes(depth)).unwrap_or_default();
        if app.is_empty() {
            let empty = match self.live {
                Some(_) => "This app shows nothing yet.",
                None => "Nothing here yet",
            };
            app.push(text(Style::Dim, empty));
        }
        Node::Card { id: 0, children: app }
    }
}

/// What a person calls the app at `path`: `/apps/tip-calculator.app` is "Tip calculator".
fn name(path: &str) -> String {
    let file = file_name(path);
    let mut name: String = file.strip_suffix(".app").unwrap_or(file).replace(['-', '_'], " ");
    if let Some(first) = name.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    name
}
