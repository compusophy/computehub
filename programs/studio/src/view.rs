//! Studio's frames: one column at every width. With nothing open, only the prompt, centered.
//! Else what the program's first comment says, the app and its last fault (or its code, or while
//! it is made the program streaming in), a status with New app and `</>` (and Send to
//! compusophy after a make that failed, or by the app's fault; on a phone, the status over
//! them), and the prompt with Make (Stop while making).

use crate::edit::{DRAFT, MAKE, NEW, SEND, STOP, TOGGLE, spans};
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
        self.saw();
        // Each frame's draft is newer than what the desktop holds of it (see `DRAFT`).
        if self.make.is_some() {
            self.drafts = self.drafts.wrapping_add(1);
        }
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
        let caption = clip(caption.lines().next().unwrap_or(""), 160);
        let mut nodes = Vec::new();
        if !caption.is_empty() {
            nodes.push(text(Style::Small, &caption));
        }
        let code = |(id, version), numbers, text: &str, spans| {
            let text = text.to_string();
            fill(vec![Node::Code { id, version, line_numbers: numbers, text, spans }])
        };
        match making {
            Some(m) => {
                // The newest lines streaming in, as many as fit (a Code cannot scroll itself to
                // its end); or the program being fixed, whole, its problem marked. A Code the
                // desktop owns, as the code view's: kept to its room, the wheel scrolls it.
                let (draft, streaming) = m.draft();
                let fit = self.rows(caption.chars().count());
                // The line being written takes a row too, and no highlight yet: half a string
                // would show as an error.
                let (mut from, mut rows) = (0, usize::from(!draft.ends_with('\n')));
                for (i, b) in draft.bytes().enumerate().rev().filter(|_| streaming) {
                    if b == b'\n' {
                        if rows == fit {
                            from = i + 1;
                            break;
                        }
                        rows += 1;
                    }
                }
                let tail = draft.get(from..).unwrap_or("");
                let lit =
                    if streaming { tail.rfind('\n').map_or(0, |i| i + 1) } else { tail.len() };
                let marks = spans(tail.get(..lit).unwrap_or(""), m.mark().filter(|_| !streaming));
                nodes.push(code((DRAFT, self.drafts), !streaming, tail, marks));
            }
            None if self.code => {
                let marks = spans(&self.text, self.mark);
                nodes.push(code((self.code_id(), self.version), true, &self.text, marks))
            }
            None => {
                if let Some(live) = &self.live {
                    nodes.extend(live.nodes(1));
                    // The kept fault stays under the app after the app's next event takes it.
                    let (fault, afresh) = live.fault();
                    let kept = Some(self.fault.as_str()).filter(|f| !f.is_empty());
                    if kept.is_some() && kept != fault && kept != afresh {
                        nodes.push(text(Style::Error, &self.fault));
                    }
                }
                // The app keeps its height; the rest goes to the bottom.
                nodes.push(fill(Vec::new()));
            }
        }
        let status = match making {
            Some(m) => (Style::Small, m.status(now())),
            None => self.status.clone(),
        };
        let mut line = vec![flex()];
        line.extend(self.offer().filter(|_| making.is_none()).map(|sent| match sent {
            false => button(SEND, Variant::Chip, "Send to compusophy"),
            true => text(Style::Small, "Sent"),
        }));
        if self.held() {
            line.push(button(NEW, Variant::Quiet, "New app"));
        }
        if making.is_none() && !self.text.is_empty() {
            line.push(button(TOGGLE, Variant::Quiet, "</>"));
        }
        // On a phone the status has its own line, over its buttons: beside them it would get
        // a sliver, and wrap word by word.
        match self.narrow() {
            true => {
                nodes.extend((!status.1.is_empty()).then(|| text(status.0, &status.1)));
                nodes.extend((line.len() > 1).then(|| row(8, line)));
            }
            false => {
                line.insert(0, text(status.0, &status.1));
                nodes.push(row(8, line));
            }
        }
        nodes.push(self.bar());
        nodes
    }

    /// How many rows of code the draft has room for under a caption of `chars` chars: the
    /// window's height but for the padding (40 px), the status (15), the prompt (34), the gaps
    /// between them (8 each), the caption's lines (15 each; Small text is some 6.5 px a char
    /// at most) and the Code's insets (12). A row is 17 px, taken as 18 to spare one; a Code
    /// has 3 at least.
    fn rows(&self, chars: usize) -> usize {
        let wide = usize::from(self.width.saturating_sub(40)).max(1);
        let lines = (chars * 13).div_ceil(2 * wide);
        let rest = 117 + if lines > 0 { 8 + 15 * lines } else { 0 };
        (usize::from(self.height).saturating_sub(rest) / 18).max(3)
    }
}
