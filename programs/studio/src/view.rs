//! Studio's frames: one column at every width. With nothing open, only the prompt, centered.
//! Else what the program's first comment says, the app (or its code, or while it is made the
//! program streaming in), a status with `</>` (and Send to compusophy after a make that failed,
//! or by the app's fault), and the prompt with Make (Stop while making).

use crate::edit::{MAKE, SEND, STOP, TOGGLE, spans};
use crate::make::now;
use crate::{Studio, file_name, text};
use coder::ai::clip;
use uiwire::{Frame, Node, Request, Style, Variant};

/// Narrower than this, a make takes the keyboard away.
const NARROW: u16 = 600;
/// The centered prompt's width.
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
        let title = match self.path.is_empty() {
            true => "Studio".into(),
            false => ["Studio \u{2014} ", file_name(&self.path)].concat(),
        };
        let blank = self.path.is_empty() && self.text.is_empty() && self.make.is_none();
        let nodes = match blank && self.status.1.is_empty() {
            // A little above the middle reads as centered.
            true => vec![
                fill(Vec::new()),
                row(
                    0,
                    vec![
                        flex(),
                        Node::Pane { id: 0, w: CENTER, children: vec![self.bar()] },
                        flex(),
                    ],
                ),
                fill(Vec::new()),
                Node::Spacer { px: 40 },
            ],
            false => self.working(),
        };
        let mut requests = std::mem::take(&mut self.requests);
        if !std::mem::replace(&mut self.framed, true) {
            requests.insert(0, Request::Size { w: SIZE.0, h: SIZE.1 });
        }
        // The app's timer runs while it shows, not its code.
        let play = self.live.as_mut().map_or((0, false), |live| live.play());
        let play = if self.code || self.make.is_some() { (0, play.1) } else { play };
        crate::run::ask(&mut self.asked, play, &mut requests);
        Frame { seq: 0, title, requests, nodes }
    }

    /// The prompt and Make, or Stop while a make runs.
    fn bar(&self) -> Node {
        let (id, value) = (self.prompt_id(), self.prompt.clone());
        let placeholder = if self.path.is_empty() { "Describe an app" } else { "Change it" };
        let input = Node::Input { id, value, placeholder: placeholder.into() };
        let action = match self.make {
            Some(_) => button(STOP, Variant::Normal, "Stop"),
            None => button(MAKE, Variant::Primary, "Make"),
        };
        row(8, vec![input, action])
    }

    /// The caption, the app (or code, or draft), the status and the prompt.
    fn working(&self) -> Vec<Node> {
        let making = self.make.as_ref().map(|mk| &mk.m);
        let caption = making.map_or_else(|| self.caption.clone(), |m| m.plan());
        let mut nodes = Vec::new();
        if !caption.is_empty() {
            nodes.push(text(Style::Small, &clip(caption.lines().next().unwrap_or(""), 160)));
        }
        let code = |id, numbers, text: &str, mark| {
            let (spans, text) = (spans(text, mark), text.to_string());
            fill(vec![Node::Code { id, version: self.version, line_numbers: numbers, text, spans }])
        };
        match making {
            Some(m) => {
                // The newest lines streaming in, as many as fit (a Code cannot scroll itself to
                // its end); or the program being fixed, whole, its problem marked.
                let (draft, streaming) = m.draft();
                let fit = usize::from(self.height.saturating_sub(140) / 18).max(6);
                let (mut from, mut ends) = (0, 0);
                for (i, b) in draft.bytes().enumerate().rev() {
                    if b == b'\n' && streaming {
                        ends += 1;
                        if ends > fit {
                            from = i + 1;
                            break;
                        }
                    }
                }
                let tail = &draft[from..];
                nodes.push(code(0, !streaming, tail, m.mark().filter(|_| !streaming)));
            }
            None if self.code => nodes.push(code(self.code_id(), true, &self.text, self.mark)),
            None => {
                nodes.extend(self.live.as_ref().map(|live| live.nodes(1)).unwrap_or_default());
                // The app keeps its height; the rest goes to the bottom.
                nodes.push(fill(Vec::new()));
            }
        }
        let status = match making {
            Some(m) => (Style::Small, m.status(now())),
            None => self.status.clone(),
        };
        let mut line = vec![text(status.0, &status.1), flex()];
        line.extend(self.offer().filter(|_| making.is_none()).map(|sent| match sent {
            false => button(SEND, Variant::Chip, "Send to compusophy"),
            true => text(Style::Small, "Sent"),
        }));
        if !self.text.is_empty() && self.make.is_none() {
            line.push(button(TOGGLE, Variant::Quiet, "</>"));
        }
        nodes.push(row(8, line));
        nodes.push(self.bar());
        nodes
    }
}
