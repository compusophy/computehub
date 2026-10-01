//! The remote UI protocol: a GUI program writes its window as a widget tree,
//! never pixels (one [`Frame`] per `write()` to `/dev/draw`); the desktop draws
//! it in its own toolkit and theme and answers with [`Event`]s (one per `read()`
//! of `/dev/events`, over 64 KiB in 64 KiB parts). [`client`] wraps both.
//!
//! Little-endian; a string is a `u32` byte length plus UTF-8, a `bool` a `u8`
//! 0 or 1, a `char` a `u32`. Frame: `u8` [`VERSION`], `u32` seq, title, `u16`
//! request count, the [`Request`]s, `u32` node count, the [`Node`]s in
//! pre-order. Node: `u8` kind, `u32` id (0 = none), `u16` child count, fields,
//! children. Request, Event: `u8` kind, fields. Kinds and [`Key`]s number the
//! variants from 1 in order; [`Style`], [`Variant`] and [`Class`] from 0.
//! Decoding never panics and is strict, so the encoding is canonical: anything
//! malformed, trailing, unknown or over a cap is `None`.
//!
//! While the user edits an Input or Code the host keeps its text, sending [`Event::Change`]
//! with its edit count as the version; a Code at that version takes its spans, above it its text.

#![forbid(unsafe_code)]

pub mod client;
#[cfg(test)]
mod tests;

/// The protocol version, the first byte of every frame.
pub const VERSION: u8 = 1;
/// The largest encoded frame or event, in bytes.
pub const MAX_FRAME: usize = 1 << 20;
/// The most nodes a frame (or one decoded [`Node`] tree) holds.
pub const MAX_NODES: usize = 4096;
/// The deepest nesting; a top-level node is at depth 1.
pub const MAX_DEPTH: usize = 32;

macro_rules! codes {
    ($($(#[$doc:meta])* $name:ident { $($v:ident = $n:literal,)* })*) => {$(
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(u8)]
        pub enum $name {
            $($v = $n,)*
        }

        impl $name {
            /// The variant with this wire code, if any.
            pub fn from_u8(code: u8) -> Option<Self> {
                match code {
                    $($n => Some(Self::$v),)*
                    _ => None,
                }
            }
        }
    )*};
}

codes! {
    /// How a [`Node::Text`] is set (Dim is secondary text). Text wraps to the width.
    Style { Body = 0, Title = 1, Heading = 2, Subheading = 3, Small = 4, Mono = 5, Dim = 6,
        Error = 7, Success = 8, }
    /// How a [`Node::Button`] looks: an ordinary, the main or a destructive action.
    Variant { Normal = 0, Primary = 1, Danger = 2, }
    /// The highlight class of a [`Span`]; Error is drawn underlined.
    Class { Plain = 0, Keyword = 1, String = 2, Number = 3, Comment = 4, Name = 5, Punct = 6,
        Error = 7, }
    /// The key of an [`Event::Key`]: the four arrows are Up to Right, and Char is a
    /// character key (the event's `ch` says which).
    Key { Enter = 1, Escape = 2, Tab = 3, Up = 4, Down = 5, Left = 6, Right = 7, Char = 8, }
}

/// The modifier bits of an [`Event::Key`]: Shift, Control, Alt (Option) and
/// Meta (Command, Windows); a bit outside [`mods::ALL`] is malformed.
pub mod mods {
    pub const SHIFT: u8 = 1;
    pub const CTRL: u8 = 2;
    pub const ALT: u8 = 4;
    pub const META: u8 = 8;
    pub const ALL: u8 = 15;
}

/// A highlighted byte range of a [`Node::Code`]'s text: `len` bytes from `start`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub len: u32,
    pub class: Class,
}

/// One widget of a window and, for the four containers, its children. Ids
/// name the nodes events come from; 0 means none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    /// Children laid out top to bottom, `gap` logical px apart.
    Col { id: u32, gap: u8, children: Vec<Node> },
    /// Children laid out left to right, `gap` logical px apart.
    Row { id: u32, gap: u8, children: Vec<Node> },
    /// Text wrapped to the width.
    Text { id: u32, style: Style, text: String },
    /// A button; a press sends [`Event::Click`].
    Button { id: u32, variant: Variant, label: String },
    /// A single-line input: [`Event::Change`] per edit, [`Event::Submit`] on Enter.
    Input { id: u32, value: String, placeholder: String },
    /// A multi-line code editor with highlight spans, in ascending order and
    /// apart, on char boundaries of `text`.
    Code { id: u32, version: u32, line_numbers: bool, text: String, spans: Vec<Span> },
    /// A thin rule.
    Separator,
    /// Empty space, `px` logical px along the parent's axis.
    Spacer { px: u16 },
    /// A rounded surface around its children, laid out top to bottom.
    Card { id: u32, children: Vec<Node> },
    /// A clickable list row; a click sends [`Event::Click`].
    Item { id: u32, text: String, detail: String, selected: bool },
    /// Its children take the parent's remaining height (around a Code, say).
    Fill { id: u32, children: Vec<Node> },
}

/// One complete picture of a window, program to host: the program's frame
/// counter, the window title, what it asks of the desktop (in order) and
/// the top-level nodes, laid out top to bottom.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub seq: u32,
    pub title: String,
    pub requests: Vec<Request>,
    pub nodes: Vec<Node>,
}

/// Something a frame asks of the desktop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Open an app, by registry name or path, in a new window.
    Open { name: String },
    /// Close this window.
    Close,
    /// The preferred content size in logical px; honored on the first frame only.
    Size { w: u16, h: u16 },
}

/// Something that happened in the window, host to program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A Button or Item was pressed.
    Click { id: u32 },
    /// An Input's or Code's text changed; `version` counts the host's edits.
    Change { id: u32, version: u32, text: String },
    /// A Ctrl, Alt or Meta combination, or Enter or Escape in an input. `id`
    /// is the focused node or 0; `ch` is the character for [`Key::Char`].
    Key { id: u32, key: Key, mods: u8, ch: char },
    /// The content size in logical px; also the first event, before any frame.
    Resize { w: u16, h: u16 },
    /// The window is closing: exit.
    Close,
    /// Enter in an Input.
    Submit { id: u32 },
}

/// The public `encode` and `decode` of each message, from its `put` and `get`.
macro_rules! wire {
    ($($t:ident)*) => {$(
        impl $t {
            /// The bytes (a [`Node`]'s are its tree, in pre-order).
            pub fn encode(&self) -> Vec<u8> {
                let mut o = Out(Vec::new());
                self.put(&mut o);
                o.0
            }

            /// Exactly one, or `None` if the bytes are malformed, trailing or over a cap.
            pub fn decode(bytes: &[u8]) -> Option<Self> {
                let mut r = Reader(Some(bytes).filter(|b| b.len() <= MAX_FRAME)?);
                Self::get(&mut r).filter(|_| r.0.is_empty())
            }
        }
    )*};
}

wire!(Frame Node Request Event);

impl Node {
    /// The children; empty for a leaf.
    pub fn children(&self) -> &[Node] {
        match self {
            Self::Col { children, .. }
            | Self::Row { children, .. }
            | Self::Card { children, .. }
            | Self::Fill { children, .. } => children,
            _ => &[],
        }
    }

    /// How many nodes the tree holds, this one included.
    pub fn count(&self) -> usize {
        1 + self.children().iter().map(Node::count).sum::<usize>()
    }

    fn put(&self, o: &mut Out) {
        let n = self.children().len();
        _ = match self {
            Self::Col { id, gap, .. } => o.head(1, *id, n).u8(*gap),
            Self::Row { id, gap, .. } => o.head(2, *id, n).u8(*gap),
            Self::Text { id, style, text } => o.head(3, *id, n).u8(*style as u8).str(text),
            Self::Button { id, variant, label } => o.head(4, *id, n).u8(*variant as u8).str(label),
            Self::Input { id, value, placeholder } => o.head(5, *id, n).str(value).str(placeholder),
            Self::Code { id, version, line_numbers, text, spans } => {
                let o = o.head(6, *id, n).u32(*version).u8((*line_numbers).into());
                let o = o.str(text).len(spans.len());
                spans.iter().fold(o, |o, s| o.u32(s.start).u32(s.len).u8(s.class as u8))
            }
            Self::Separator => o.head(7, 0, n),
            Self::Spacer { px } => o.head(8, 0, n).u16(*px),
            Self::Card { id, .. } => o.head(9, *id, n),
            Self::Item { id, text, detail, selected } => {
                o.head(10, *id, n).str(text).str(detail).u8((*selected).into())
            }
            Self::Fill { id, .. } => o.head(11, *id, n),
        };
        self.children().iter().for_each(|child| child.put(o));
    }

    fn get(r: &mut Reader<'_>) -> Option<Self> {
        Self::read(r, 1, &mut { MAX_NODES })
    }

    /// One node at `depth` and its children, each taken from `budget`; a
    /// leaf with children, or an id on a Separator or Spacer, is malformed.
    fn read(r: &mut Reader<'_>, depth: usize, budget: &mut usize) -> Option<Self> {
        *budget = budget.checked_sub(1).filter(|_| depth <= MAX_DEPTH)?;
        let (kind, id, count) = (r.u8()?, r.u32()?, r.u16()?);
        let mut node = match kind {
            1 => Self::Col { id, gap: r.u8()?, children: Vec::new() },
            2 => Self::Row { id, gap: r.u8()?, children: Vec::new() },
            3 => Self::Text { id, style: Style::from_u8(r.u8()?)?, text: r.str()? },
            4 => Self::Button { id, variant: Variant::from_u8(r.u8()?)?, label: r.str()? },
            5 => Self::Input { id, value: r.str()?, placeholder: r.str()? },
            6 => Self::read_code(r, id)?,
            7 if id == 0 => Self::Separator,
            8 if id == 0 => Self::Spacer { px: r.u16()? },
            9 => Self::Card { id, children: Vec::new() },
            10 => Self::Item { id, text: r.str()?, detail: r.str()?, selected: r.bool()? },
            11 => Self::Fill { id, children: Vec::new() },
            _ => return None,
        };
        match &mut node {
            Self::Col { children, .. }
            | Self::Row { children, .. }
            | Self::Card { children, .. }
            | Self::Fill { children, .. } => {
                children.reserve(usize::from(count).min(*budget));
                for _ in 0..count {
                    children.push(Self::read(r, depth + 1, budget)?);
                }
            }
            _ if count != 0 => return None,
            _ => {}
        }
        Some(node)
    }

    fn read_code(r: &mut Reader<'_>, id: u32) -> Option<Self> {
        let (version, line_numbers, text) = (r.u32()?, r.bool()?, r.str()?);
        let n = r.count().filter(|n| *n <= r.0.len() / 9)?;
        let mut spans = Vec::with_capacity(n);
        let mut end = 0;
        for _ in 0..n {
            let span = Span { start: r.u32()?, len: r.u32()?, class: Class::from_u8(r.u8()?)? };
            let start = usize::try_from(span.start).ok()?;
            let stop = start.checked_add(usize::try_from(span.len).ok()?)?;
            if start < end || !text.is_char_boundary(start) || !text.is_char_boundary(stop) {
                return None;
            }
            end = stop;
            spans.push(span);
        }
        Some(Self::Code { id, version, line_numbers, text, spans })
    }
}

impl Frame {
    fn put(&self, o: &mut Out) {
        o.u8(VERSION).u32(self.seq).str(&self.title);
        o.u16(u16::try_from(self.requests.len()).unwrap_or(u16::MAX));
        self.requests.iter().for_each(|request| request.put(o));
        o.len(self.nodes.iter().map(Node::count).sum());
        self.nodes.iter().for_each(|node| node.put(o));
    }

    fn get(r: &mut Reader<'_>) -> Option<Self> {
        r.u8().filter(|v| *v == VERSION)?;
        let (seq, title, n) = (r.u32()?, r.str()?, r.u16()?);
        let mut requests = Vec::with_capacity(usize::from(n).min(r.0.len()));
        for _ in 0..n {
            requests.push(Request::get(r)?);
        }
        let mut budget = r.count().filter(|n| *n <= MAX_NODES)?;
        let mut nodes = Vec::new();
        while budget > 0 {
            nodes.push(Node::read(r, 1, &mut budget)?);
        }
        Some(Self { seq, title, requests, nodes })
    }
}

impl Request {
    fn put(&self, o: &mut Out) {
        _ = match self {
            Self::Open { name } => o.u8(1).str(name),
            Self::Close => o.u8(2),
            Self::Size { w, h } => o.u8(3).u16(*w).u16(*h),
        }
    }

    fn get(r: &mut Reader<'_>) -> Option<Self> {
        Some(match r.u8()? {
            1 => Self::Open { name: r.str()? },
            2 => Self::Close,
            3 => Self::Size { w: r.u16()?, h: r.u16()? },
            _ => return None,
        })
    }
}

impl Event {
    fn put(&self, o: &mut Out) {
        _ = match self {
            Self::Click { id } => o.u8(1).u32(*id),
            Self::Change { id, version, text } => o.u8(2).u32(*id).u32(*version).str(text),
            Self::Key { id, key, mods, ch } => {
                o.u8(3).u32(*id).u8(*key as u8).u8(*mods).u32(u32::from(*ch))
            }
            Self::Resize { w, h } => o.u8(4).u16(*w).u16(*h),
            Self::Close => o.u8(5),
            Self::Submit { id } => o.u8(6).u32(*id),
        }
    }

    fn get(r: &mut Reader<'_>) -> Option<Self> {
        Some(match r.u8()? {
            1 => Self::Click { id: r.u32()? },
            2 => Self::Change { id: r.u32()?, version: r.u32()?, text: r.str()? },
            3 => Self::Key {
                id: r.u32()?,
                key: Key::from_u8(r.u8()?)?,
                mods: r.u8().filter(|m| m & !mods::ALL == 0)?,
                ch: char::from_u32(r.u32()?)?,
            },
            4 => Self::Resize { w: r.u16()?, h: r.u16()? },
            5 => Self::Close,
            6 => Self::Submit { id: r.u32()? },
            _ => return None,
        })
    }
}

/// Bytes being written, little-endian; each write returns the writer.
struct Out(Vec<u8>);

impl Out {
    fn put(&mut self, b: &[u8]) -> &mut Out {
        self.0.extend_from_slice(b);
        self
    }

    fn u8(&mut self, v: u8) -> &mut Out {
        self.put(&[v])
    }

    fn u16(&mut self, v: u16) -> &mut Out {
        self.put(&v.to_le_bytes())
    }

    fn u32(&mut self, v: u32) -> &mut Out {
        self.put(&v.to_le_bytes())
    }

    fn len(&mut self, n: usize) -> &mut Out {
        self.u32(u32::try_from(n).unwrap_or(u32::MAX))
    }

    fn str(&mut self, s: &str) -> &mut Out {
        self.len(s.len()).put(s.as_bytes())
    }

    /// A node's kind, id and child count.
    fn head(&mut self, kind: u8, id: u32, children: usize) -> &mut Out {
        self.u8(kind).u32(id).u16(u16::try_from(children).unwrap_or(u16::MAX))
    }
}

/// The unread input; every read is bounds-checked.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (head, tail) = self.0.split_at_checked(n)?;
        self.0 = tail;
        Some(head)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn count(&mut self) -> Option<usize> {
        usize::try_from(self.u32()?).ok()
    }

    fn bool(&mut self) -> Option<bool> {
        self.u8().filter(|b| *b < 2).map(|b| b == 1)
    }

    fn str(&mut self) -> Option<String> {
        let n = self.count()?;
        core::str::from_utf8(self.take(n)?).ok().map(str::to_owned)
    }
}
