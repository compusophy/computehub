//! The remote UI protocol: a GUI program writes its window as a widget tree,
//! never pixels (one [`Frame`] per `write()` to `/dev/draw`); the desktop draws
//! it in its own toolkit and theme and answers with [`Event`]s (one per `read()`
//! of `/dev/events`, over 64 KiB in 64 KiB parts). [`client`] wraps both.
//!
//! Little-endian; a string is a `u32` byte length plus UTF-8, bytes the same but
//! raw, a `bool` a `u8` 0 or 1, a `char` a `u32`. Frame: `u8` [`VERSION`], `u32`
//! seq, title, `u16` request count, the [`Request`]s, `u32` node count, the
//! [`Node`]s in pre-order. Node: `u8` kind, `u32` id (0 = none), `u16` child count, fields,
//! children. Request, Event: `u8` kind, fields. Kinds and [`Key`]s number the
//! variants from 1 in order; [`Style`], [`Variant`], [`Class`] and [`Shape`] from 0.
//! Decoding never panics and is strict, so the encoding is canonical: anything
//! malformed, trailing, unknown or over a cap is `None`. Codes are only ever added, after the
//! last, so a desktop reads the frames of every program older than it.
//!
//! While the user edits an Input, Code or Area the host keeps its text, sending [`Event::Change`]
//! with its edit count as the version; a Code at that version takes its spans, above it its text.
//!
//! The overlay (the Assistant over the desktop) may also act as a person would: [`Request::Act`]
//! is answered by one [`Event::Acted`] once the screen settles, carrying the [`scene`] as it is
//! then; [`Event::Halt`] says the person took over. A request carries its [`Act`] as the act's
//! own bytes, so only the overlay links the act's encoder and only the desktop its decoder.
//!
//! The OS's own windows alone (Activity's and Settings', which run its `system` program) may
//! watch the desktop's meters ([`Request::Watch`], answered by [`Event::Stats`] in the [`stat`]
//! format), end a process ([`Request::End`]) and set a preference ([`Request::Pref`]), and hear
//! what Settings shows ([`Event::Prefs`]); the desktop drops those requests from any other.

#![forbid(unsafe_code)]

pub mod client;
pub mod stat;
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
/// A [`Node::Glyph`]'s `glyph` with this bit added comes in the first time the window shows it:
/// the mark from its center out, ring by ring, in 618 ms (another glyph draws whole).
pub const REVEAL: u8 = 0x80;
/// A [`Node::Entry`]'s `glyph` that is a `.app` file's tile: the sigil of `hue` read as a seed
/// (FNV-1a of the file's name), on that seed's tint, as the home screen draws the app.
pub const SIGIL: u8 = u8::MAX;

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
    /// How a [`Node::Text`] is set (Dim is secondary text, Accent is body text in the accent,
    /// Display a first screen's name, larger than a Title, Warning small print in the error
    /// color). Text wraps to the width.
    Style { Body = 0, Title = 1, Heading = 2, Subheading = 3, Small = 4, Mono = 5, Dim = 6,
        Error = 7, Success = 8, Accent = 9, Display = 10, Warning = 11, }
    /// How a [`Node::Button`] looks: an ordinary, the main or a destructive action, a chip
    /// (a small, quiet suggestion), a chip that is on (the chosen one of a set), quiet (its
    /// label alone, dim until the pointer is over it: a crumb, a toolbar's), or a link (its
    /// label in the accent and a chevron after it, in a row [`LINK_H`] tall across the width,
    /// washed under the pointer: it leads elsewhere).
    Variant { Normal = 0, Primary = 1, Danger = 2, Chip = 3, On = 4, Quiet = 5, Link = 6, }
    /// The highlight class of a [`Span`]; Error is drawn underlined.
    Class { Plain = 0, Keyword = 1, String = 2, Number = 3, Comment = 4, Name = 5, Punct = 6,
        Error = 7, }
    /// The key of an [`Event::Key`]: the four arrows are Up to Right, and Char is a
    /// character key (the event's `ch` says which).
    Key { Enter = 1, Escape = 2, Tab = 3, Up = 4, Down = 5, Left = 6, Right = 7, Char = 8, }
    /// What an [`Act::Window`] does to a window: the title bar's controls, and raising it.
    WinOp { Focus = 0, Close = 1, Minimize = 2, Maximize = 3, Restore = 4, }
    /// What a [`Draw`] of a [`Node::Canvas`] draws.
    Shape { Rect = 0, Circle = 1, Ring = 2, Line = 3, Text = 4, Sprite = 5, }
}

impl Default for Style {
    /// Body text.
    fn default() -> Style {
        Style::Body
    }
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

/// The codes of an [`Event::Acted`]: done, or why not. [`acted::BUSY`] is soft (the act was
/// done; a window still works).
pub mod acted {
    pub const OK: u16 = 0;
    /// The window is gone.
    pub const GONE: u16 = 911;
    /// The element is not on screen (scrolled out, or the page changed).
    pub const OFF_SCREEN: u16 = 912;
    pub const NOT_TEXT: u16 = 913;
    pub const UNKNOWN_APP: u16 = 914;
    pub const BUSY: u16 = 915;
    /// Not the overlay's to ask, or aimed at the overlay.
    pub const REFUSED: u16 = 916;
    /// Another act is still settling.
    pub const IN_FLIGHT: u16 = 917;
    /// An unknown key, a scroll out of range, a tap of no grid's square (or a grid clicked).
    pub const MALFORMED: u16 = 918;
}

/// What the overlay does on the desktop, the way a person's pointer and keys do: windows by id
/// (`win` 0 for a Key is the focused one), widgets by the id their app gives them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    /// Nothing: let `ms` pass (at most 5,000), then see the screen.
    Wait {
        ms: u16,
    },
    /// Press and release widget `id` of window `win` (not a Grid: its squares are tapped).
    Click {
        win: u32,
        id: u32,
    },
    /// Focus text field `id`, type `text`, then Enter if `submit`.
    Type {
        win: u32,
        id: u32,
        text: String,
        submit: bool,
    },
    /// A key by its `KeyboardEvent.code` (`"KeyS"`, `"Enter"`) with [`mods`] bits.
    Key {
        win: u32,
        code: String,
        mods: u8,
    },
    /// The wheel over widget `id` (0: the content's middle), `dy` px down.
    Scroll {
        win: u32,
        id: u32,
        dy: i16,
    },
    /// Open an app by name, or bring its window up.
    Open {
        name: String,
    },
    Window {
        win: u32,
        op: WinOp,
    },
    Theme {
        name: String,
    },
    /// Tap square `cell` of Grid `id` of window `win`.
    Tap {
        win: u32,
        id: u32,
        cell: u32,
    },
}

/// A highlighted byte range of a [`Node::Code`]'s text: `len` bytes from `start`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub len: u32,
    pub class: Class,
}

/// One widget of a window and, for the eight containers, its children. Ids
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
    /// Children laid out top to bottom in a column `w` logical px wide, or the width it is
    /// given if less; in a Row it takes `w` and its flexible siblings share the rest.
    Pane { id: u32, w: u16, children: Vec<Node> },
    /// A vector icon `size` logical px square in the text color: `glyph` is an `icons::Glyph`
    /// (its index in `Glyph::ALL`), plus [`REVEAL`] to bring it in; one the desktop does not
    /// know is empty space.
    Glyph { glyph: u8, size: u16 },
    /// A list row: `glyph` on an app tile of `hue` (0xRRGGBB; [`SIGIL`] for a `.app` file's),
    /// `text` (a second line, after a `\n`, small under the first), `detail` dim at the right
    /// and, when `more` (it leads somewhere, as a folder does), a chevron; a click sends
    /// [`Event::Click`].
    Entry { id: u32, glyph: u8, hue: u32, text: String, detail: String, more: bool },
    /// A row saying `label`, with a switch at the right that is `on`; a click sends
    /// [`Event::Click`] (the program flips it).
    Toggle { id: u32, on: bool, label: String },
    /// A multi-line input whose text wraps and grows it: [`Event::Change`] per edit; Enter is a
    /// new line (Ctrl or Meta with it is an [`Event::Key`]).
    Area { id: u32, value: String, placeholder: String },
    /// Children laid out top to bottom, `gap` logical px apart, each centered across the width
    /// (a Text line by line).
    Center { id: u32, gap: u8, children: Vec<Node> },
    /// Children laid out top to bottom that scroll inside it, the wheel over it moving them,
    /// while the rest of the window stays (a list under a bar). As a Fill, it takes the height
    /// the others leave; a new `id` starts at the top (a list of another folder).
    Scroll { id: u32, children: Vec<Node> },
    /// Children laid out left to right on one line, `gap` logical px apart, each its own width
    /// (a Text unwrapped); when they do not fit, the first slide out to the left, so the last
    /// stays in view (a path bar's crumbs).
    Strip { id: u32, gap: u8, children: Vec<Node> },
    /// Squares `cols` a row (at least 1), each a color: 0 empty, 1 to 8 the theme's palette (at
    /// most [`GRID_COLOR`]); `texts` empty or one per square, written in it. With an id, a press
    /// on a square, and the pointer dragged onto another while pressed, sends [`Event::Tap`].
    Grid { id: u32, cols: u16, cells: Vec<u8>, texts: Vec<String> },
    /// A picture `w` units wide and `h` tall (each 1 to [`MAX_SIDE`]), scaled to fit with square
    /// units: its draws in order, each over those before, [`MAX_INK`] ink at most. With an id, a
    /// press on it, and the pointer dragged onto another unit while pressed, sends
    /// [`Event::Tap`] with `cell` the unit's `y * w + x`.
    Canvas { id: u32, w: u16, h: u16, draws: Vec<Draw> },
    /// A frame's first node only (elsewhere nothing): the window's pages, `labels` one a line,
    /// page `i` the button `id + i` and page `on` lit; a column at the window's left and a line
    /// right of it when the window is [`WIDE`] or wider, else a segmented control across its
    /// top. The frame's other nodes are laid out beside or under it, and scroll there; a new
    /// `on` starts them at the top.
    Pages { id: u32, on: u8, labels: String },
    /// The desktop's themes as cards, each a miniature of its desktop over its name (the
    /// current one ringed and checked in the accent): the default first, as many across as fit
    /// [`THEME_MIN`] to [`THEME_MAX`] px wide. Theme `i`, in the desktop's own order, is the
    /// button `id + i`.
    Themes { id: u32 },
    /// A raised card [`CARD_H`] tall across the width offering a choice: `text` (a second line,
    /// after a `\n`, small under the first), ringed and checked in the accent when it is `on`
    /// (the chosen one of a set); a click sends [`Event::Click`].
    Choice { id: u32, on: bool, text: String },
    /// A raised card [`CARD_H`] tall across the width saying `label`, with a switch at its right
    /// that is `on`; a click sends [`Event::Click`] (the program flips it).
    Switch { id: u32, on: bool, label: String },
}

/// The narrowest window whose [`Node::Pages`] are a column.
pub const WIDE: u16 = 520;
/// A [`Node::Choice`]'s or [`Node::Switch`]'s height, and a [`Variant::Link`]'s.
pub const CARD_H: u16 = 56;
pub const LINK_H: u16 = 44;
/// The narrowest and the widest card of [`Node::Themes`].
pub const THEME_MIN: u16 = 150;
pub const THEME_MAX: u16 = 216;

/// The highest color of a [`Node::Grid`]'s square.
pub const GRID_COLOR: u8 = 8;
/// The highest color of a [`Draw`]: 0 the canvas itself, 1 to 8 a Grid's, 9 ink, 10 dim ink and
/// 11 the accent, each the theme's.
pub const CANVAS_COLOR: u8 = 11;
/// The most units a [`Node::Canvas`] has a side.
pub const MAX_SIDE: u16 = 1024;
/// The most ink a [`Node::Canvas`] holds ([`Draw::ink`]).
pub const MAX_INK: usize = 4096;

/// One shape on a [`Node::Canvas`], in its units: x to the right and y down from its top-left
/// unit, a point at its unit's middle. `at` holds, by shape (the slots after, 0; the sizes never
/// negative, and 0 draws nothing):
///
/// | shape  | `at`              | draws                                                         |
/// |--------|-------------------|---------------------------------------------------------------|
/// | Rect   | x y w h           | units x to x + w - 1 and y to y + h - 1                       |
/// | Circle | x y r             | a disc of radius r                                            |
/// | Ring   | x y r width       | a circle's outline, width thick inside r                      |
/// | Line   | x1 y1 x2 y2 width | a segment with round ends                                     |
/// | Text   | x y size          | `text`, one line, size units tall, centered on x, y           |
/// | Sprite | x y side          | `text`'s lines down from x, y, a char a square side units wide |
///
/// A Sprite's digit paints its square that color; any other char shows through.
/// `color` is at most [`CANVAS_COLOR`] (0 for a Sprite); only a Text and a Sprite have a `text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draw {
    pub shape: Shape,
    pub color: u8,
    pub at: [i16; 5],
    pub text: String,
}

impl Draw {
    /// The slots of `at` each shape uses, and those that are sizes, as bits from slot 0.
    #[rustfmt::skip]
    const SLOTS: [(u8, u8); 6] = [(0b1111, 0b1100), (0b111, 0b100), (0b1111, 0b1100),
        (0b11111, 0b10000), (0b111, 0b100), (0b111, 0b100)];

    /// Its ink: 1, and 1 more for each char of a Text and each painted square of a Sprite.
    pub fn ink(&self) -> usize {
        1 + match self.shape {
            Shape::Text => self.text.chars().count(),
            Shape::Sprite => self.text.bytes().filter(u8::is_ascii_digit).count(),
            _ => 0,
        }
    }

    /// Whether it decodes: see [`Draw`].
    fn valid(&self) -> bool {
        let (used, sizes) = Self::SLOTS[self.shape as usize];
        let slots =
            self.at.iter().enumerate().all(|(i, &v)| match (used >> i & 1, sizes >> i & 1) {
                (0, _) => v == 0,
                (_, 1) => v >= 0,
                _ => true,
            });
        let text = match self.shape {
            Shape::Text => !self.text.as_bytes().contains(&b'\n'),
            Shape::Sprite => self.color == 0,
            _ => self.text.is_empty(),
        };
        self.color <= CANVAS_COLOR && slots && text
    }
}

/// Whether a Canvas `w` x `h` of `draws` decodes.
fn canvas(w: u16, h: u16, draws: &[Draw]) -> bool {
    let side = |s: u16| (1..=MAX_SIDE).contains(&s);
    let ink = draws.iter().map(Draw::ink).sum::<usize>();
    side(w) && side(h) && ink <= MAX_INK && draws.iter().all(Draw::valid)
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
    /// Ask the AI: `body` is an OpenAI-style chat-completions request; AiData, AiEnd answer.
    Ai { id: u32, body: String },
    /// Stop AI request `id`.
    AiCancel { id: u32 },
    /// Put the keyboard in the Input, Code or Area `id` of this frame.
    Focus { id: u32 },
    /// Send feedback the person wrote to compusophy: `kind` ("bug", "idea" or "love"), the
    /// `text`, and with `context` the desktop's (build, device, windows, recent events).
    Feedback { kind: String, text: String, context: bool },
    /// The overlay only: do the act `act` encodes ([`Act::encode`]); one at a time, answered by
    /// [`Event::Acted`].
    Act { id: u32, act: Vec<u8> },
    /// The overlay only: whether it works on a task (the desktop shows it, and the person's
    /// own input outside the overlay then halts it).
    Status { working: bool },
    /// Send [`Event::Tick`] about every `ms` while the window shows (0: stop).
    Timer { ms: u32 },
    /// Whether plain keys (arrows, letters, digits, space) come as [`Event::Key`] while no text
    /// field of the window has the keyboard.
    Keys { on: bool },
    /// Activity's window only: send [`Event::Stats`] when something loud changes, at most once a
    /// second (`false` stops). Dropped from any other window.
    Watch { on: bool },
    /// Activity's window only: end process `pid` now (status 137, as a kill). Dropped from any
    /// other window.
    End { pid: u32 },
    /// The OS's own windows only: set the preference `key` to `value`, as the desktop stores
    /// them (`ai.model`, `reports`, `grain`; `theme` switches the desktop's theme). Dropped from
    /// any other window.
    Pref { key: String, value: String },
}

/// Something that happened in the window, host to program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A Button or Item was pressed.
    Click { id: u32 },
    /// An Input's or Code's text changed; `version` counts the host's edits.
    Change { id: u32, version: u32, text: String },
    /// A Ctrl, Alt or Meta combination, or Enter or Escape in an input; and the plain keys
    /// [`Request::Keys`] asks for. `id` is the focused node or 0; `ch` is the character for
    /// [`Key::Char`].
    Key { id: u32, key: Key, mods: u8, ch: char },
    /// The content size in logical px; also the first event, before any frame.
    Resize { w: u16, h: u16 },
    /// The window is closing: exit.
    Close,
    /// Enter in an Input.
    Submit { id: u32 },
    /// The AI settings, the model the AI answers with: after the first Resize and on every
    /// change.
    Config { model: String },
    /// More of AI request `id`'s response body (SSE text), at most 32 KiB.
    AiData { id: u32, data: Vec<u8> },
    /// AI request `id` ended: the HTTP status (0: none) and the host's error, if any.
    AiEnd { id: u32, status: u16, error: String },
    /// A prompt from the desktop's everything bar, as if typed in the window and sent.
    Ask { text: String },
    /// The window gained (`on`) or lost the keyboard focus: a time to look again at what it
    /// shows (Files lists its folder anew).
    Focus { on: bool },
    /// Act `id` settled with `code` ([`acted`]) and the host's `note`; `scene` is the screen
    /// now ([`scene::Scene`] bytes).
    Acted { id: u32, code: u16, note: String, scene: Vec<u8> },
    /// The person took over (a press, key or wheel outside the overlay): stop acting.
    Halt,
    /// `ms` passed since the last tick ([`Request::Timer`]).
    Tick { ms: u32 },
    /// Square `cell` of Grid `id` was pressed, or dragged onto (or across).
    Tap { id: u32, cell: u32 },
    /// The desktop's meters now ([`Request::Watch`]): [`stat::Stats`] bytes.
    Stats { data: Vec<u8> },
    /// The OS's own windows only, after the first Resize and on every change: what Settings
    /// shows, whether automatic error reports go, the backdrop's grain lives and the person's
    /// files are kept.
    Prefs { reports: bool, grain: bool, kept: bool },
}

/// The public `encode` and `decode` of each message, from its `put` and `get`.
macro_rules! wire {
    ($($t:ident)*) => {$(
        impl $t {
            /// The bytes (a [`Node`](crate::Node)'s are its tree, in pre-order).
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

wire!(Frame Node Request Event Act);

pub mod scene;

impl Node {
    /// The children; empty for a leaf.
    pub fn children(&self) -> &[Node] {
        match self {
            Self::Col { children, .. }
            | Self::Row { children, .. }
            | Self::Card { children, .. }
            | Self::Fill { children, .. }
            | Self::Pane { children, .. }
            | Self::Center { children, .. }
            | Self::Scroll { children, .. }
            | Self::Strip { children, .. } => children,
            _ => &[],
        }
    }

    /// How many nodes the tree holds, this one included.
    pub fn count(&self) -> usize {
        1 + self.children().iter().map(Node::count).sum::<usize>()
    }

    /// Whether the tree, at `depth`, decodes as it is: it is no deeper than [`MAX_DEPTH`] and
    /// each Code's spans are in order, apart and on its text's char boundaries.
    fn valid(&self, depth: usize) -> bool {
        let spans = match self {
            Self::Grid { cols, cells, texts, .. } => grid(*cols, cells, texts.len()),
            Self::Canvas { w, h, draws, .. } => canvas(*w, *h, draws),
            Self::Code { text, spans, .. } => {
                let mut end = 0;
                spans.iter().all(|s| {
                    let start = s.start as usize;
                    let stop = start.saturating_add(s.len as usize);
                    let ok = start >= end && text.is_char_boundary(start);
                    end = stop;
                    ok && text.is_char_boundary(stop)
                })
            }
            _ => true,
        };
        depth <= MAX_DEPTH && spans && self.children().iter().all(|c| c.valid(depth + 1))
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
            Self::Pane { id, w, .. } => o.head(12, *id, n).u16(*w),
            Self::Glyph { glyph, size } => o.head(13, 0, n).u8(*glyph).u16(*size),
            Self::Entry { id, glyph, hue, text, detail, more } => {
                let o = o.head(14, *id, n).u8(*glyph).u32(*hue);
                o.str(text).str(detail).u8((*more).into())
            }
            Self::Toggle { id, on, label } => o.head(15, *id, n).u8((*on).into()).str(label),
            Self::Area { id, value, placeholder } => o.head(16, *id, n).str(value).str(placeholder),
            Self::Center { id, gap, .. } => o.head(17, *id, n).u8(*gap),
            Self::Scroll { id, .. } => o.head(18, *id, n),
            Self::Strip { id, gap, .. } => o.head(19, *id, n).u8(*gap),
            Self::Grid { id, cols, cells, texts } => {
                let o = o.head(20, *id, n).u16(*cols).bytes(cells).len(texts.len());
                texts.iter().fold(o, |o, t| o.str(t))
            }
            Self::Canvas { id, w, h, draws } => {
                let o = o.head(21, *id, n).u16(*w).u16(*h).len(draws.len());
                draws.iter().fold(o, |o, d| {
                    let o = d.at.iter().fold(o.u8(d.shape as u8).u8(d.color), |o, v| o.i16(*v));
                    match d.shape {
                        Shape::Text | Shape::Sprite => o.str(&d.text),
                        _ => o,
                    }
                })
            }
            Self::Pages { id, on, labels } => o.head(22, *id, n).u8(*on).str(labels),
            Self::Themes { id } => o.head(23, *id, n),
            Self::Choice { id, on, text } => o.head(24, *id, n).u8((*on).into()).str(text),
            Self::Switch { id, on, label } => o.head(25, *id, n).u8((*on).into()).str(label),
        };
        self.children().iter().for_each(|child| child.put(o));
    }

    fn get(r: &mut Reader<'_>) -> Option<Self> {
        Self::read(r, 1, &mut { MAX_NODES })
    }

    /// One node at `depth` and its children, each taken from `budget`; a
    /// leaf with children, or an id on a Separator, Spacer or Glyph, is malformed.
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
            12 => Self::Pane { id, w: r.u16()?, children: Vec::new() },
            13 if id == 0 => Self::Glyph { glyph: r.u8()?, size: r.u16()? },
            14 => {
                let (glyph, hue, text) = (r.u8()?, r.u32()?, r.str()?);
                Self::Entry { id, glyph, hue, text, detail: r.str()?, more: r.bool()? }
            }
            15 => Self::Toggle { id, on: r.bool()?, label: r.str()? },
            16 => Self::Area { id, value: r.str()?, placeholder: r.str()? },
            17 => Self::Center { id, gap: r.u8()?, children: Vec::new() },
            18 => Self::Scroll { id, children: Vec::new() },
            19 => Self::Strip { id, gap: r.u8()?, children: Vec::new() },
            20 => {
                let (cols, cells) = (r.u16()?, r.bytes()?.to_vec());
                let n = r.count().filter(|n| *n <= r.0.len() / 4 && grid(cols, &cells, *n))?;
                let mut texts = Vec::with_capacity(n);
                for _ in 0..n {
                    texts.push(r.str()?);
                }
                Self::Grid { id, cols, cells, texts }
            }
            21 => Self::read_canvas(r, id)?,
            22 => Self::Pages { id, on: r.u8()?, labels: r.str()? },
            23 => Self::Themes { id },
            24 => Self::Choice { id, on: r.bool()?, text: r.str()? },
            25 => Self::Switch { id, on: r.bool()?, label: r.str()? },
            _ => return None,
        };
        match &mut node {
            Self::Col { children, .. }
            | Self::Row { children, .. }
            | Self::Card { children, .. }
            | Self::Fill { children, .. }
            | Self::Pane { children, .. }
            | Self::Center { children, .. }
            | Self::Scroll { children, .. }
            | Self::Strip { children, .. } => {
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

    /// A Canvas's fields: its size, then its draws, 12 bytes each at least.
    fn read_canvas(r: &mut Reader<'_>, id: u32) -> Option<Self> {
        let (w, h) = (r.u16()?, r.u16()?);
        let n = r.count().filter(|n| *n <= r.0.len() / 12)?;
        let mut draws = Vec::with_capacity(n);
        for _ in 0..n {
            let (shape, color) = (Shape::from_u8(r.u8()?)?, r.u8()?);
            let at = [r.i16()?, r.i16()?, r.i16()?, r.i16()?, r.i16()?];
            let text = match shape {
                Shape::Text | Shape::Sprite => r.str()?,
                _ => String::new(),
            };
            draws.push(Draw { shape, color, at, text });
        }
        canvas(w, h, &draws).then_some(Self::Canvas { id, w, h, draws })
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

/// Whether a Grid of `cols`, `cells` and `texts` texts decodes.
fn grid(cols: u16, cells: &[u8], texts: usize) -> bool {
    cols > 0 && cells.iter().all(|c| *c <= GRID_COLOR) && (texts == 0 || texts == cells.len())
}

impl Frame {
    /// The bytes, if they decode: within the caps (bytes, nodes, depth, requests), every Code's
    /// spans in order, apart and on char boundaries. Checked without the decoder, which a
    /// program then need not carry.
    pub fn encode_checked(&self) -> Option<Vec<u8>> {
        let nodes = self.nodes.iter().map(Node::count).sum::<usize>();
        let ok = self.requests.len() <= usize::from(u16::MAX) && nodes <= MAX_NODES;
        let bytes = self.encode();
        let ok = ok && bytes.len() <= MAX_FRAME && self.nodes.iter().all(|n| n.valid(1));
        ok.then_some(bytes)
    }

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
            Self::Ai { id, body } => o.u8(4).u32(*id).str(body),
            Self::AiCancel { id } => o.u8(5).u32(*id),
            Self::Focus { id } => o.u8(6).u32(*id),
            Self::Feedback { kind, text, context } => {
                o.u8(7).str(kind).str(text).u8((*context).into())
            }
            Self::Act { id, act } => o.u8(8).u32(*id).bytes(act),
            Self::Status { working } => o.u8(9).u8((*working).into()),
            Self::Timer { ms } => o.u8(10).u32(*ms),
            Self::Keys { on } => o.u8(11).u8((*on).into()),
            Self::Watch { on } => o.u8(12).u8((*on).into()),
            Self::End { pid } => o.u8(13).u32(*pid),
            Self::Pref { key, value } => o.u8(14).str(key).str(value),
        }
    }

    fn get(r: &mut Reader<'_>) -> Option<Self> {
        Some(match r.u8()? {
            1 => Self::Open { name: r.str()? },
            2 => Self::Close,
            3 => Self::Size { w: r.u16()?, h: r.u16()? },
            4 => Self::Ai { id: r.u32()?, body: r.str()? },
            5 => Self::AiCancel { id: r.u32()? },
            6 => Self::Focus { id: r.u32()? },
            7 => Self::Feedback { kind: r.str()?, text: r.str()?, context: r.bool()? },
            8 => {
                let (id, act) = (r.u32()?, r.bytes()?);
                Act::decode(act).map(|_| Self::Act { id, act: act.into() })?
            }
            9 => Self::Status { working: r.bool()? },
            10 => Self::Timer { ms: r.u32()? },
            11 => Self::Keys { on: r.bool()? },
            12 => Self::Watch { on: r.bool()? },
            13 => Self::End { pid: r.u32()? },
            14 => Self::Pref { key: r.str()?, value: r.str()? },
            _ => return None,
        })
    }
}

impl Act {
    fn put(&self, o: &mut Out) {
        _ = match self {
            Self::Wait { ms } => o.u8(1).u16(*ms),
            Self::Click { win, id } => o.u8(2).u32(*win).u32(*id),
            Self::Type { win, id, text, submit } => {
                o.u8(3).u32(*win).u32(*id).str(text).u8((*submit).into())
            }
            Self::Key { win, code, mods } => o.u8(4).u32(*win).str(code).u8(*mods),
            Self::Scroll { win, id, dy } => o.u8(5).u32(*win).u32(*id).i16(*dy),
            Self::Open { name } => o.u8(6).str(name),
            Self::Window { win, op } => o.u8(7).u32(*win).u8(*op as u8),
            Self::Theme { name } => o.u8(8).str(name),
            Self::Tap { win, id, cell } => o.u8(9).u32(*win).u32(*id).u32(*cell),
        };
    }

    fn get(r: &mut Reader<'_>) -> Option<Self> {
        Some(match r.u8()? {
            1 => Self::Wait { ms: r.u16()? },
            2 => Self::Click { win: r.u32()?, id: r.u32()? },
            3 => Self::Type { win: r.u32()?, id: r.u32()?, text: r.str()?, submit: r.bool()? },
            4 => Self::Key {
                win: r.u32()?,
                code: r.str()?,
                mods: r.u8().filter(|m| m & !mods::ALL == 0)?,
            },
            5 => Self::Scroll { win: r.u32()?, id: r.u32()?, dy: r.i16()? },
            6 => Self::Open { name: r.str()? },
            7 => Self::Window { win: r.u32()?, op: WinOp::from_u8(r.u8()?)? },
            8 => Self::Theme { name: r.str()? },
            9 => Self::Tap { win: r.u32()?, id: r.u32()?, cell: r.u32()? },
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
            Self::Config { model } => o.u8(7).str(model),
            Self::AiData { id, data } => o.u8(8).u32(*id).bytes(data),
            Self::AiEnd { id, status, error } => o.u8(9).u32(*id).u16(*status).str(error),
            Self::Ask { text } => o.u8(10).str(text),
            Self::Focus { on } => o.u8(11).u8((*on).into()),
            Self::Acted { id, code, note, scene } => {
                o.u8(12).u32(*id).u16(*code).str(note).bytes(scene)
            }
            Self::Halt => o.u8(13),
            Self::Tick { ms } => o.u8(14).u32(*ms),
            Self::Tap { id, cell } => o.u8(15).u32(*id).u32(*cell),
            Self::Stats { data } => o.u8(16).bytes(data),
            Self::Prefs { reports, grain, kept } => {
                o.u8(17).u8((*reports).into()).u8((*grain).into()).u8((*kept).into())
            }
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
            7 => Self::Config { model: r.str()? },
            8 => Self::AiData { id: r.u32()?, data: r.bytes()?.to_vec() },
            9 => Self::AiEnd { id: r.u32()?, status: r.u16()?, error: r.str()? },
            10 => Self::Ask { text: r.str()? },
            11 => Self::Focus { on: r.bool()? },
            12 => Self::Acted {
                id: r.u32()?,
                code: r.u16()?,
                note: r.str()?,
                scene: r.bytes()?.to_vec(),
            },
            13 => Self::Halt,
            14 => Self::Tick { ms: r.u32()? },
            15 => Self::Tap { id: r.u32()?, cell: r.u32()? },
            16 => Self::Stats { data: r.bytes()?.to_vec() },
            17 => Self::Prefs { reports: r.bool()?, grain: r.bool()?, kept: r.bool()? },
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

    fn len(&mut self, n: usize) -> &mut Out {
        self.u32(u32::try_from(n).unwrap_or(u32::MAX))
    }

    fn bytes(&mut self, b: &[u8]) -> &mut Out {
        self.len(b.len()).put(b)
    }

    fn str(&mut self, s: &str) -> &mut Out {
        self.bytes(s.as_bytes())
    }

    /// A node's kind, id and child count.
    fn head(&mut self, kind: u8, id: u32, children: usize) -> &mut Out {
        self.u8(kind).u32(id).u16(u16::try_from(children).unwrap_or(u16::MAX))
    }
}

/// Each integer's write on [`Out`] and read on [`Reader`], little-endian.
macro_rules! ints {
    ($($t:ident)*) => {
        impl Out {
            $(fn $t(&mut self, v: $t) -> &mut Out { self.put(&v.to_le_bytes()) })*
        }
        impl Reader<'_> {
            $(fn $t(&mut self) -> Option<$t> {
                Some($t::from_le_bytes(self.take(size_of::<$t>())?.try_into().ok()?))
            })*
        }
    };
}

ints!(u8 u16 u32 i16);

/// The unread input; every read is bounds-checked.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (head, tail) = self.0.split_at_checked(n)?;
        self.0 = tail;
        Some(head)
    }

    fn count(&mut self) -> Option<usize> {
        usize::try_from(self.u32()?).ok()
    }

    fn bool(&mut self) -> Option<bool> {
        self.u8().filter(|b| *b < 2).map(|b| b == 1)
    }

    fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = self.count()?;
        self.take(n)
    }

    fn str(&mut self) -> Option<String> {
        core::str::from_utf8(self.bytes()?).ok().map(str::to_owned)
    }
}
