//! compusophyOS's start, drawn natively in the boot (no program, no worker): the peaceful welcome
//! a new tab opens on, and logon to a profile.
//!
//! - **The welcome is the first frame.** It costs less than the desktop's would: no /home is
//!   put back and no shell is made until someone signs in.
//! - **The mark is art.** compusophy's mark comes in once from the center out (618 ms, as 365
//!   round fills: no atlas), never labeled as progress. The name follows when its font lands.
//! - **The record is truth** ([`record`]): under the column a hairline holds one segment per
//!   stage of the real start, at its measured times, gaps kept; its caption says
//!   `Loading fonts…`, then `Ready in 412 ms · 218 KB`. A tap (or the keys' focus, then Enter)
//!   opens a card of the stages. Nothing is simulated, and waiting draws no frames.
//! - **Logon.** A first visit says hello and starts with **Start**. A return shows the
//!   [`profiles`] as circles (people are round, apps rounded squares), the one that signed in
//!   last in focus, and **Add**: a tap, or the arrows and Enter, signs in; holding a circle
//!   500 ms (or a right-click) opens its menu: Rename, Set a PIN (Change PIN, Remove PIN),
//!   Remove (confirmed, with its files). Signing in flies the mark to the bar's (220 ms) as the
//!   welcome fades off the desktop. A reload of a signed-in tab skips it ([`SESSION`]), never a
//!   PIN's. Escape always goes back a step.
//! - **A PIN** (4 to 8 digits, optional) is a curtain, not a lock: files are not encrypted, and
//!   the copy says so. It is typed on the phone's number pad, checked by the browser's own
//!   PBKDF2 ([`Out::Derive`], a random salt), and never kept or sent: the list keeps its
//!   record ([`profiles::Pin`]). It guards signing in, renaming and changing it; removing a
//!   profile never needs it (a forgotten PIN means just that).
//! - **Signing out** happens on the desktop; if its files could not be kept, a card over the
//!   desktop offers Stay or Sign out anyway ([`Logon::unkept`]).
//!
//! Pure: no browser, no clock, no randomness. `os` hands in the page clock, storage reads and
//! fresh random bytes ([`Logon::fresh`]), and carries out what comes back ([`Out`]). Logical
//! pixels, origin top-left; every color from the theme of the profile that signed in last.

#![forbid(unsafe_code)]

mod paint;
pub mod profiles;
pub mod record;

use gfx::RectF;
use home::menu::Menu;
use host::{Input, LocalTime, Response};
use profiles::{LAST, LIST, LIST_BAD, Op, PER_PROFILE, Profiles};
use ui::{Key, Theme};

pub use profiles::{active, key, listed, own, quiet, sign};

/// The device's mark that a welcome said hello (`localStorage`), and the tab's session
/// (`sessionStorage`): the profile it signed in to, which a reload goes straight back to.
pub const SEEN: &str = "compusophy.seen";
pub const SESSION: &str = "compusophy.session";
/// How long a press on a circle opens its menu, and how far a finger may wander meanwhile.
const HOLD_MS: f64 = 500.0;
const WANDER: f32 = 8.0;

/// What the welcome asks of the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Out {
    /// Sign in to profile `id`: put back its home, read its preferences, make the desktop.
    SignIn(u32),
    /// Store the value under the `localStorage` key, or remove the key.
    Set(String, String),
    Remove(String),
    /// Keep the tab's [`SESSION`]: a profile a reload goes straight back to.
    Session(String),
    /// Report a coded failure: its message, and the note it leaves.
    Failed(&'static str, &'static str),
    /// Sign out after all, the files unkept; or close the card over the desktop (Stay).
    SignOut,
    Close,
    /// The text input's keyboard: a phone's number pad (for a PIN) or its letters.
    Numeric(bool),
    /// Derive 32 bytes from `pin` by PBKDF2-HMAC-SHA-256 ([`Logon::derived`] hears them as
    /// `id`): the only thing that ever carries a PIN's digits, and only to the browser's own
    /// WebCrypto.
    Derive {
        id: u32,
        pin: Vec<u8>,
        salt: [u8; 16],
        iterations: u32,
    },
}

/// A PIN's words, as said.
const ENTER: &str = "Enter your PIN";
const WRONG: &str = "Wrong PIN. Try again.";
const CHOOSE: &str = "Choose a PIN (4 to 8 digits)";
const AGAIN: &str = "Enter it again";
const MISMATCH: &str = "They did not match. Try again.";
const INSECURE: &str = "PINs need a secure page (https).";
const UNCHECKED: &str = "The PIN could not be checked.";

/// The profile a tab's [`SESSION`] (`session`, as stored) goes straight back to on a reload,
/// if it is still listed (`get` reads `localStorage`).
pub fn session(session: Option<&str>, get: &dyn Fn(&str) -> Option<String>) -> Option<u32> {
    let id = session?.parse().ok()?;
    let list = Profiles::read(get(LIST).as_deref()).0;
    list.get(id).filter(|p| p.pin.is_none()).map(|p| p.id)
}

/// What the welcome shows: a first visit's hello; the circles to pick from; a name being
/// given (a new profile, or profile `id` renamed); whether a new profile wants a PIN; a new PIN
/// being chosen (for the new profile, or profile `id`); profile `id`'s PIN asked, then what it
/// opens; a removal to confirm; the card over the desktop when its files could not be kept.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    Hello,
    #[default]
    Pick,
    Name(Option<u32>),
    Ask,
    NewPin(Option<u32>),
    Pin(u32, Then),
    Confirm(u32),
    Unkept,
}

/// What a PIN, once right, opens: the desktop, a rename, a new PIN, or none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Then {
    SignIn,
    Rename,
    Change,
    Unpin,
}

/// A derivation awaited: a PIN to check against its hash (then what it opens), or a new PIN's
/// record to make (its digits' count and salt).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    Check([u8; 32]),
    Set(usize, [u8; 16]),
}

/// What a press lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Start,
    Record,
    Card,
    Back,
    /// A profile's circle, by its place in the list; past the last, Add.
    Circle(usize),
    Field,
    Cancel,
    Next,
    Remove,
    Stay,
    Anyway,
    Skip,
    SetPin,
}

/// What a circle's menu does to its profile: rename it, set (or change) its PIN, remove its
/// PIN, remove it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    Rename,
    SetPin,
    Unpin,
    Remove,
}

/// The welcome: the screen, the theme and grain of the profile that signed in last, whether
/// a welcome said hello on this device, what shows, the clock; when it first drew (the
/// reveal's clock) and when someone signed in (the flight's); the profiles as last read (and
/// whether a newer OS wrote them: read-only); the keys' focus (a circle, Start, or past them
/// the record) and whether the keys moved it; the name being typed; a line under the block
/// (an error if so); the pointer (over, pressed, when and where, a finger's); the card; a
/// circle's menu (asked for, then open) for its profile; whether text input is on (and asked
/// again, and a number pad); what was drawn where, last frame. For a PIN: whether the page can
/// derive (a secure one), a new profile's name, a new PIN's first entry, the derivation awaited
/// (its id, the last asked), when a wrong PIN shook.
#[derive(Debug, Default)]
pub struct Logon {
    secure: bool,
    naming: String,
    first: String,
    awaited: Option<(u32, Pending)>,
    asks: u32,
    shook: Option<f64>,
    numeric: bool,
    size: (f32, f32),
    look: &'static str,
    grain: bool,
    seen: bool,
    state: State,
    clock: Option<LocalTime>,
    start: Option<f64>,
    leaving: Option<f64>,
    reduced: bool,
    list: Profiles,
    read_only: bool,
    focus: usize,
    keyed: bool,
    typed: String,
    note: Option<(&'static str, bool)>,
    hover: Option<Target>,
    press: Option<Target>,
    down: Option<(f64, (f32, f32))>,
    finger: bool,
    card: bool,
    asked: Option<(u32, (f32, f32))>,
    menu: Option<(u32, Menu<Act>)>,
    files: usize,
    ime: bool,
    retype: bool,
    hits: Vec<(RectF, Target)>,
    /// Random bytes for the next new profile's face, renewed by `os` before every input.
    pub fresh: [u8; 20],
}

impl Logon {
    /// The welcome for a screen `size`, reading `localStorage` through `get`: a first visit
    /// (no list, no welcome said hello, no files kept) says hello; a return picks a profile.
    /// PINs need a `secure` page (WebCrypto). What to ask of the page at once: a damaged list
    /// set aside and reported.
    pub fn new(
        size: (f32, f32),
        get: &dyn Fn(&str) -> Option<String>,
        secure: bool,
    ) -> (Logon, Vec<Out>) {
        let stored = get(LIST);
        let (list, why) = Profiles::read(stored.as_deref());
        let mut outs = Vec::new();
        if let (Some(profiles::DAMAGED), Some(s)) = (why, stored.as_ref()) {
            outs.push(Out::Set(LIST_BAD.into(), s.clone()));
            outs.push(Out::Failed(profiles::DAMAGED, "profiles unread"));
        }
        let last = get(LAST).and_then(|s| s.parse::<u32>().ok());
        let focus = list.list.iter().position(|p| Some(p.id) == last).unwrap_or(0);
        let id = list.list[focus].id;
        let seen = get(SEEN).is_some();
        let first = stored.is_none() && !seen && get(&key(0, "home")).is_none();
        let l = Logon {
            size,
            look: ui::theme(&get(&key(id, "theme")).unwrap_or_default()).name,
            grain: get(&key(id, "grain")).as_deref() != Some("off"),
            seen,
            state: if first { State::Hello } else { State::Pick },
            read_only: why == Some(profiles::NEWER),
            list,
            focus,
            secure,
            ..Default::default()
        };
        (l, outs)
    }

    /// The card over the desktop when sign-out could not keep the files, in the theme named
    /// `theme`: Stay, or Sign out anyway.
    pub fn unkept(size: (f32, f32), theme: &str) -> Logon {
        let look = ui::theme(theme).name;
        Logon { size, look, seen: true, state: State::Unkept, ..Default::default() }
    }

    /// The theme it is drawn in (whose base clears the frame).
    pub fn theme(&self) -> &'static Theme {
        ui::theme(self.look)
    }

    /// Whether someone signed in: the desktop takes the input, and [`Logon::layer`] draws the
    /// flight over it.
    pub fn leaving(&self) -> bool {
        self.leaving.is_some()
    }

    /// Whether the flight is over at `now`: the welcome can go.
    pub fn gone(&self, now: f64) -> bool {
        self.leaving.is_some_and(|t| now - t >= paint::FLIGHT)
    }

    /// One input at `now` (page ms), `get` reading `localStorage` (changes to the list start
    /// from it, never from a copy, so another tab's survive): what to draw and ask of the page.
    /// Text input follows the state, said inside the input that changed it (a phone shows its
    /// keyboard only then), and again for a tap on the field.
    pub fn input(
        &mut self,
        input: &Input,
        now: f64,
        get: &dyn Fn(&str) -> Option<String>,
    ) -> (Response, Vec<Out>) {
        let (mut r, mut outs) = (Response::default(), Vec::new());
        match *input {
            Input::Resize { w, h } => {
                self.size = (finite(w), finite(h));
                (self.menu, r.redraw) = (None, true);
            }
            Input::Tick { time } => r.redraw = self.clock.replace(time) != Some(time),
            Input::PointerMove { x, y } => {
                let at = self.at(x, y);
                let far = |(_, (x0, y0)): (f64, (f32, f32))| (x - x0).hypot(y - y0) > WANDER;
                self.down = self.down.filter(|&d| !far(d));
                let sel = self.menu.as_mut().map(|m| (m.1.sel, m.1.at(x, y).flatten()));
                if let (Some(m), Some((_, now_sel))) = (&mut self.menu, sel) {
                    m.1.sel = now_sel;
                }
                r.redraw = std::mem::replace(&mut self.hover, at) != at
                    || sel.is_some_and(|(was, is)| was != is);
                r.consumed = true;
            }
            Input::PointerLeave => r.redraw = self.hover.take().is_some(),
            Input::PointerDown { x, y, button, touch } => {
                (self.finger, r.consumed, r.redraw) = (touch, true, true);
                if self.menu.as_ref().is_some_and(|m| m.1.rect.contains(x, y)) {
                    return (r, outs);
                }
                self.menu = None;
                match (self.at(x, y), button) {
                    (Some(Target::Circle(i)), 2) => self.ask_menu(i, (x, y)),
                    (t, 0) => (self.press, self.down) = (t, Some((now, (x, y)))),
                    _ => {}
                }
            }
            Input::PointerUp { x, y, .. } => {
                (r.consumed, r.redraw) = (true, true);
                let menu = self.menu.as_ref();
                let item = menu.and_then(|(id, m)| Some((*id, m.act(m.at(x, y).flatten()?)?)));
                if let Some((id, act)) = item {
                    self.menu = None;
                    self.menu_act(id, act, get);
                } else if let Some(t) = self.press.take().filter(|&t| Some(t) == self.at(x, y)) {
                    self.act(t, now, get, &mut outs);
                }
                (self.press, self.down) = (None, None);
            }
            Input::Key { key, mods } => {
                r.consumed = self.key(key, mods.shift, now, get, &mut outs);
                r.redraw = r.consumed;
            }
            Input::Text(ref s) => {
                r.redraw = true;
                match self.state {
                    State::Name(_) => {
                        let room =
                            profiles::NAME_MAX - self.typed.chars().count().min(profiles::NAME_MAX);
                        self.typed.extend(s.chars().filter(|c| !c.is_control()).take(room));
                    }
                    State::Pin(..) | State::NewPin(_) => {
                        s.chars()
                            .filter(char::is_ascii_digit)
                            .for_each(|c| self.digit(c, &mut outs));
                    }
                    _ => r.redraw = false,
                }
            }
            Input::Wheel { .. } => {}
        }
        self.settle(&mut r, &mut outs);
        (r, outs)
    }

    /// Text input as the state wants it (a name's letters, a PIN's number pad), said when it
    /// changes, or again when asked (a tap on the field brings back a dismissed keyboard).
    fn settle(&mut self, r: &mut Response, outs: &mut Vec<Out>) {
        let numeric = matches!(self.state, State::Pin(..) | State::NewPin(_));
        let want = (numeric || matches!(self.state, State::Name(_))) && self.leaving.is_none();
        if want && numeric != self.numeric {
            (self.numeric, self.retype) = (numeric, true);
            outs.push(Out::Numeric(numeric));
        }
        // Asked again or not, it is said once.
        let again = std::mem::take(&mut self.retype);
        if want != self.ime || again {
            self.ime = want;
            r.text_input = Some(want);
        }
    }

    /// A digit typed for a PIN (none while one is checked): the PIN asked is checked at its
    /// length; a new one is at most 8, and its second entry is compared at the first's length.
    fn digit(&mut self, c: char, outs: &mut Vec<Out>) {
        let most = match self.state {
            State::Pin(id, _) => self.pin(id).map_or(0, |p| p.len),
            _ if self.first.is_empty() => 8,
            _ => self.first.len(),
        };
        if self.awaited.is_some() || self.typed.len() >= most {
            return;
        }
        self.typed.push(c);
        if self.typed.len() < most {
            return;
        }
        match self.state {
            State::Pin(id, _) => {
                let Some(p) = self.pin(id) else { return };
                self.derive(Pending::Check(p.hash), p.salt, p.iterations, outs);
            }
            _ if self.first.is_empty() => {}
            _ if self.typed == self.first => {
                let salt: [u8; 16] = std::array::from_fn(|i| self.fresh[4 + i]);
                let len = self.typed.len();
                self.derive(Pending::Set(len, salt), salt, profiles::ITERATIONS, outs);
            }
            _ => {
                self.first.clear();
                self.typed.clear();
                self.say(MISMATCH, true);
            }
        }
    }

    /// Profile `id`'s PIN, if it has one.
    fn pin(&self, id: u32) -> Option<profiles::Pin> {
        profiles::Pin::read(self.list.get(id)?.pin.as_deref()?)
    }

    /// Asks the page to derive from the digits typed (which go, here and now), awaiting `then`.
    fn derive(&mut self, then: Pending, salt: [u8; 16], iterations: u32, outs: &mut Vec<Out>) {
        self.asks += 1;
        let pin = std::mem::take(&mut self.typed).into_bytes();
        outs.push(Out::Derive { id: self.asks, pin, salt, iterations });
        self.awaited = Some((self.asks, then));
    }

    /// Derivation `id` came back at `now` (`get` reading `localStorage`): a PIN right opens
    /// what it guards, one wrong shakes and clears; a new PIN's record is stored (a new profile
    /// with it, then signed in to). A failure says why. What to draw and ask of the page.
    pub fn derived(
        &mut self,
        id: u32,
        got: Result<Vec<u8>, String>,
        now: f64,
        get: &dyn Fn(&str) -> Option<String>,
    ) -> (Response, Vec<Out>) {
        let (mut r, mut outs) = (Response { redraw: true, ..Response::default() }, Vec::new());
        let Some((_, then)) = self.awaited.take_if(|a| a.0 == id) else { return (r, outs) };
        let bytes: Option<[u8; 32]> = got.ok().and_then(|b| b.try_into().ok());
        match (then, bytes, self.state) {
            (Pending::Check(hash), Some(b), State::Pin(id, then)) => {
                // All 32 bytes compared, whatever the first that differs.
                if b.iter().zip(hash).fold(0, |d, (x, y)| d | (x ^ y)) == 0 {
                    self.opened(id, then, now, get, &mut outs);
                } else {
                    (self.shook, self.note) = (Some(now), Some((WRONG, true)));
                }
            }
            (Pending::Set(len, salt), Some(hash), State::NewPin(of)) => {
                let iterations = profiles::ITERATIONS;
                let pin = Some(profiles::Pin { len, iterations, salt, hash }.format());
                match of {
                    Some(id) => {
                        if self.change(Op::SetPin(id, pin), get, &mut outs) {
                            self.go(State::Pick);
                        }
                    }
                    None => self.add(pin, now, get, &mut outs),
                }
            }
            _ => self.say(if self.secure { UNCHECKED } else { INSECURE }, true),
        }
        self.settle(&mut r, &mut outs);
        (r, outs)
    }

    /// What profile `id`'s PIN, right, opens.
    fn opened(
        &mut self,
        id: u32,
        then: Then,
        now: f64,
        get: &dyn Fn(&str) -> Option<String>,
        outs: &mut Vec<Out>,
    ) {
        match then {
            Then::SignIn => self.sign_in(id, now, outs),
            Then::Rename => {
                self.go(State::Name(Some(id)));
                self.typed = self.list.get(id).map(|p| p.name.clone()).unwrap_or_default();
            }
            Then::Change => self.choose(Some(id)),
            Then::Unpin => {
                if self.change(Op::SetPin(id, None), get, outs) {
                    self.go(State::Pick);
                }
            }
        }
    }

    /// A new PIN to choose, for profile `of` (or the new profile named): not on an insecure
    /// page, which cannot derive.
    fn choose(&mut self, of: Option<u32>) {
        if !self.secure {
            return self.say(INSECURE, true);
        }
        self.go(State::NewPin(of));
        (self.first, self.note) = (String::new(), Some((CHOOSE, false)));
    }

    /// A key: Escape goes back a step; in the menu, the arrows and Enter; on the circles (or
    /// Start), the arrows and Tab move the focus (on past them to the record) and Enter or
    /// Space does what has it; Enter gives a name, Backspace takes a char back. Whether it was
    /// the welcome's.
    fn key(
        &mut self,
        key: Key,
        shift: bool,
        now: f64,
        get: &dyn Fn(&str) -> Option<String>,
        outs: &mut Vec<Out>,
    ) -> bool {
        if let Some((id, menu)) = &mut self.menu {
            match key {
                Key::Enter => {
                    let (id, act) = (*id, menu.sel.and_then(|i| menu.act(i)));
                    self.menu = None;
                    act.into_iter().for_each(|a| self.menu_act(id, a, get));
                }
                Key::Escape => self.menu = None,
                k => return menu.key(k),
            }
            return true;
        }
        let stops = self.stops();
        match (key, self.state) {
            (Key::Escape, _) => self.back(outs),
            (Key::Tab | Key::Right | Key::Left, _) if stops > 0 => {
                let back = (shift && key == Key::Tab) || key == Key::Left;
                self.focus = (self.focus + if back { stops - 1 } else { 1 }) % stops;
                self.keyed = true;
            }
            (Key::Enter | Key::Space, State::Hello | State::Pick) => {
                let t = match (self.state, self.focus) {
                    (_, f) if f + 1 == stops => Target::Record,
                    (State::Hello, _) => Target::Start,
                    (_, f) => Target::Circle(f),
                };
                self.act(t, now, get, outs);
            }
            (Key::Enter, State::Name(_) | State::NewPin(_)) => {
                self.act(Target::Next, now, get, outs)
            }
            (Key::Enter, State::Ask) => self.act(Target::SetPin, now, get, outs),
            (Key::Backspace, State::Name(_) | State::NewPin(_) | State::Pin(..)) => {
                if self.awaited.is_none() {
                    self.typed.pop();
                }
            }
            (Key::Enter, State::Confirm(_)) => self.act(Target::Remove, now, get, outs),
            (Key::Enter, State::Unkept) => outs.push(Out::Close),
            _ => return false,
        }
        true
    }

    /// One step back: the card closes, a name or a removal is let go, the card over the
    /// desktop goes (Stay).
    fn back(&mut self, outs: &mut Vec<Out>) {
        match self.state {
            _ if self.card => self.card = false,
            State::NewPin(of) if !self.first.is_empty() => self.choose(of),
            State::NewPin(None) => self.go(State::Ask),
            State::Ask => {
                self.go(State::Name(None));
                self.typed = self.naming.clone();
            }
            State::Name(_) | State::Confirm(_) | State::Pin(..) | State::NewPin(_) => {
                self.go(State::Pick)
            }
            State::Unkept => outs.push(Out::Close),
            State::Hello | State::Pick => self.keyed = false,
        }
    }

    /// To `state`, with nothing typed, nothing said and no derivation awaited (one that comes
    /// back later is let go).
    fn go(&mut self, state: State) {
        (self.state, self.note, self.typed, self.awaited) = (state, None, String::new(), None);
    }

    /// Says `note` under the block (`error`: in the danger color).
    fn say(&mut self, note: &'static str, error: bool) {
        self.note = Some((note, error));
    }

    /// The circles: the profiles, then Add (none on a list a newer OS wrote).
    fn circles(&self) -> usize {
        self.list.list.len() + usize::from(!self.read_only)
    }

    /// Does what a tap on `t` does: a tap anywhere outside an open card only closes it.
    fn act(
        &mut self,
        t: Target,
        now: f64,
        get: &dyn Fn(&str) -> Option<String>,
        outs: &mut Vec<Out>,
    ) {
        if self.card && !matches!(t, Target::Card | Target::Record) {
            self.card = false;
            return;
        }
        match (t, self.state) {
            (Target::Record, _) => self.card = !self.card,
            (Target::Start, _) => self.sign_in(0, now, outs),
            // A PIN asked: a tap off its dots goes back to the circles.
            (Target::Circle(_) | Target::Back, State::Pin(..)) => self.go(State::Pick),
            (Target::Circle(i), _) => {
                self.focus = i;
                match self.list.list.get(i).map(|p| p.id) {
                    Some(id) if self.pin(id).is_some() => self.ask_pin(id, Then::SignIn),
                    Some(id) => self.sign_in(id, now, outs),
                    None if self.list.list.len() >= profiles::MAX => self.say(profiles::FULL, true),
                    None => self.go(State::Name(None)),
                }
            }
            (Target::Field, _) => self.retype = true,
            (Target::Cancel, _) => self.back(outs),
            (Target::Stay, _) => outs.push(Out::Close),
            (Target::Anyway, _) => outs.push(Out::SignOut),
            (Target::Next, State::NewPin(_)) => {
                if self.first.is_empty() && self.typed.len() >= 4 {
                    self.first = std::mem::take(&mut self.typed);
                    self.say(AGAIN, false);
                }
            }
            (Target::Next, _) => self.name(now, get, outs),
            (Target::Skip, _) => self.add(None, now, get, outs),
            (Target::SetPin, _) => self.choose(None),
            (Target::Remove, State::Confirm(id)) if self.change(Op::Remove(id), get, outs) => {
                self.go(State::Pick)
            }
            _ => {}
        }
    }

    /// Gives the name typed: a profile's new name; or a new profile's, which may then take a
    /// PIN (on a page that can derive one) before it is added and signed in to.
    fn name(&mut self, now: f64, get: &dyn Fn(&str) -> Option<String>, outs: &mut Vec<Out>) {
        let name = match profiles::clean(&self.typed) {
            Ok(name) => name,
            Err(why) => return self.say(why, true),
        };
        if let State::Name(Some(id)) = self.state {
            if self.change(Op::Rename(id, name), get, outs) {
                self.go(State::Pick);
            }
            return;
        }
        // Refused now (a name taken, eight already) rather than after a PIN.
        let mut dry = Profiles::read(get(LIST).as_deref()).0;
        let op = Op::Add { name: name.clone(), seed: 0, pin: None };
        if let Err(why) = dry.apply(op, &|_| false) {
            return self.say(why, true);
        }
        self.naming = name;
        if self.secure { self.go(State::Ask) } else { self.add(None, now, get, outs) }
    }

    /// Adds the profile named with `pin` (its face from fresh bytes), and signs in to it.
    fn add(
        &mut self,
        pin: Option<String>,
        now: f64,
        get: &dyn Fn(&str) -> Option<String>,
        outs: &mut Vec<Out>,
    ) {
        let seed = u32::from_le_bytes([self.fresh[0], self.fresh[1], self.fresh[2], self.fresh[3]]);
        let name = self.naming.clone();
        if self.change(Op::Add { name, seed, pin }, get, outs) {
            self.go(State::Pick);
            let id = self.list.list.last().map_or(0, |p| p.id);
            self.focus = self.list.list.len() - 1;
            self.sign_in(id, now, outs);
        }
    }

    /// Asks for profile `id`'s PIN, `then` to open what it guards (none on an insecure page,
    /// which cannot check one).
    fn ask_pin(&mut self, id: u32, then: Then) {
        if !self.secure {
            return self.say(INSECURE, true);
        }
        self.go(State::Pin(id, then));
        self.say(ENTER, false);
    }

    /// Makes the change `op` on the list as stored now and stores it (a removal takes the
    /// profile's keys with it); whether it could (if not, the line under the block says why).
    fn change(
        &mut self,
        op: Op,
        get: &dyn Fn(&str) -> Option<String>,
        outs: &mut Vec<Out>,
    ) -> bool {
        let (mut list, why) = Profiles::read(get(LIST).as_deref());
        let taken = |id: u32| PER_PROFILE.iter().any(|k| get(&key(id, k)).is_some());
        let removal = if let Op::Remove(id) = op { Some(id) } else { None };
        let done = match why {
            Some(profiles::NEWER) => Err(profiles::NEWER),
            _ => list.apply(op, &taken),
        };
        if let Err(why) = done {
            self.say(why, true);
            return false;
        }
        if let Some(id) = removal {
            outs.extend(PER_PROFILE.iter().map(|k| Out::Remove(key(id, k))));
        }
        outs.push(Out::Set(LIST.into(), list.format()));
        self.list = list;
        self.focus = self.focus.min(self.circles() - 1);
        true
    }

    /// Opens circle `i`'s menu at `at` (from the next frame, which can measure it): a
    /// profile's, never Add's, nor on a list a newer OS wrote.
    fn ask_menu(&mut self, i: usize, at: (f32, f32)) {
        match self.list.list.get(i) {
            _ if self.state != State::Pick => {}
            Some(_) if self.read_only => self.say(profiles::NEWER, false),
            Some(p) => (self.asked, self.focus) = (Some((p.id, at)), i),
            None => {}
        }
    }

    /// Does a circle's menu item for profile `id`: Rename (the name to edit), Set a PIN (or
    /// change it), Remove PIN, each behind its PIN if it has one; Remove (to confirm with the
    /// size of its files, unless it is the last), which never needs the PIN: the browser's own
    /// "clear site data" removes anything, and a forgotten PIN means just that.
    fn menu_act(&mut self, id: u32, act: Act, get: &dyn Fn(&str) -> Option<String>) {
        let Some(name) = self.list.get(id).map(|p| p.name.clone()) else { return };
        let pinned = self.pin(id).is_some();
        match act {
            Act::Rename if pinned => self.ask_pin(id, Then::Rename),
            Act::Rename => {
                self.go(State::Name(Some(id)));
                self.typed = name;
            }
            Act::SetPin if pinned => self.ask_pin(id, Then::Change),
            Act::SetPin => self.choose(Some(id)),
            Act::Unpin => self.ask_pin(id, Then::Unpin),
            Act::Remove if self.list.list.len() <= 1 => self.say(profiles::LAST_ONE, true),
            Act::Remove => {
                self.go(State::Confirm(id));
                self.files = get(&key(id, "home")).map_or(0, |h| h.chars().count());
            }
        }
    }

    /// Signs in to profile `id`: the device has said hello and remembers who signed in last,
    /// the tab keeps its session (never a PIN profile's: its PIN is asked at every page load),
    /// and the flight begins.
    fn sign_in(&mut self, id: u32, now: f64, outs: &mut Vec<Out>) {
        let mut n = String::new();
        ui::push_num(&mut n, id as usize);
        if !self.seen {
            outs.push(Out::Set(SEEN.into(), "1".into()));
        }
        outs.push(Out::Set(LAST.into(), n.clone()));
        if self.pin(id).is_none() {
            outs.push(Out::Session(n));
        }
        // The desktop's keyboard is letters again, not a PIN's number pad.
        if std::mem::take(&mut self.numeric) {
            outs.push(Out::Numeric(false));
        }
        outs.push(Out::SignIn(id));
        self.leaving = Some(if self.reduced { f64::NEG_INFINITY } else { now });
    }

    /// What is at `(x, y)` as last drawn.
    fn at(&self, x: f32, y: f32) -> Option<Target> {
        let hit = self.hits.iter().rev().find(|h| h.0.contains(x, y)).map(|h| h.1);
        Some(hit.unwrap_or(Target::Back))
    }
}

/// `v` if finite and positive, else 0.
fn finite(v: f32) -> f32 {
    if v.is_finite() { v.max(0.0) } else { 0.0 }
}

#[cfg(test)]
mod tests;
