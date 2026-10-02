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
//!   500 ms (or a right-click) opens its menu: Rename, Remove (confirmed, with its files).
//!   Signing in flies the mark to the bar's (220 ms) as the welcome fades off the desktop. A
//!   reload of a signed-in tab skips it ([`SESSION`]). Escape always goes back a step.
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
}

/// The profile a tab's [`SESSION`] (`session`, as stored) goes straight back to on a reload,
/// if it is still listed (`get` reads `localStorage`).
pub fn session(session: Option<&str>, get: &dyn Fn(&str) -> Option<String>) -> Option<u32> {
    let id = session?.parse().ok()?;
    let list = Profiles::read(get(LIST).as_deref()).0;
    list.get(id).filter(|p| p.pin.is_none()).map(|p| p.id)
}

/// What the welcome shows: a first visit's hello; the circles to pick from; a name being
/// given (a new profile, or profile `id` renamed); a removal to confirm; the card over the
/// desktop when its files could not be kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Hello,
    Pick,
    Name(Option<u32>),
    Confirm(u32),
    Unkept,
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
}

/// What a circle's menu does to its profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    Rename,
    Remove,
}

/// The welcome: the screen, the theme and grain of the profile that signed in last, whether
/// a welcome said hello on this device, what shows, the clock; when it first drew (the
/// reveal's clock) and when someone signed in (the flight's); the profiles as last read (and
/// whether a newer OS wrote them: read-only); the keys' focus (a circle, Start, or past them
/// the record) and whether the keys moved it; the name being typed; a line under the block
/// (an error if so); the pointer (over, pressed, when and where, a finger's); the card; a
/// circle's menu (asked for, then open) for its profile; whether text input is on (and asked
/// again); what was drawn where, last frame.
#[derive(Debug)]
pub struct Logon {
    size: (f32, f32),
    theme: &'static Theme,
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
    /// What to ask of the page at once: a damaged list set aside and reported.
    pub fn new(size: (f32, f32), get: &dyn Fn(&str) -> Option<String>) -> (Logon, Vec<Out>) {
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
            theme: ui::theme(&get(&key(id, "theme")).unwrap_or_default()),
            grain: get(&key(id, "grain")).as_deref() != Some("off"),
            seen,
            state: if first { State::Hello } else { State::Pick },
            read_only: why == Some(profiles::NEWER),
            list,
            focus,
            ..Logon::unkept(size, "")
        };
        (l, outs)
    }

    /// The card over the desktop when sign-out could not keep the files, in the theme named
    /// `theme`: Stay, or Sign out anyway.
    pub fn unkept(size: (f32, f32), theme: &str) -> Logon {
        Logon {
            size,
            theme: ui::theme(theme),
            grain: false,
            seen: true,
            state: State::Unkept,
            clock: None,
            start: None,
            leaving: None,
            reduced: false,
            list: Profiles::implied(),
            read_only: false,
            focus: 0,
            keyed: false,
            typed: String::new(),
            note: None,
            hover: None,
            press: None,
            down: None,
            finger: false,
            card: false,
            asked: None,
            menu: None,
            files: 0,
            ime: false,
            retype: false,
            hits: Vec::new(),
            fresh: [0; 20],
        }
    }

    /// The theme it is drawn in (whose base clears the frame).
    pub fn theme(&self) -> &'static Theme {
        self.theme
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
                if matches!(self.state, State::Name(_)) {
                    let room =
                        profiles::NAME_MAX - self.typed.chars().count().min(profiles::NAME_MAX);
                    self.typed.extend(s.chars().filter(|c| !c.is_control()).take(room));
                    (r.redraw, r.consumed) = (true, true);
                }
            }
            Input::Wheel { .. } => {}
        }
        let want = matches!(self.state, State::Name(_)) && self.leaving.is_none();
        if want != self.ime || std::mem::take(&mut self.retype) {
            self.ime = want;
            r.text_input = Some(want);
        }
        (r, outs)
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
            (Key::Enter, State::Name(_)) => self.act(Target::Next, now, get, outs),
            (Key::Backspace, State::Name(_)) => _ = self.typed.pop(),
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
            State::Name(_) | State::Confirm(_) => self.go(State::Pick),
            State::Unkept => outs.push(Out::Close),
            State::Hello | State::Pick => self.keyed = false,
        }
    }

    /// To `state`, with nothing typed and nothing said.
    fn go(&mut self, state: State) {
        (self.state, self.note, self.typed) = (state, None, String::new());
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
        match t {
            Target::Record => self.card = !self.card,
            Target::Start => self.sign_in(0, now, outs),
            Target::Circle(i) => {
                self.focus = i;
                match self.list.list.get(i).map(|p| p.id) {
                    Some(id) => self.sign_in(id, now, outs),
                    None if self.list.list.len() >= profiles::MAX => self.say(profiles::FULL, true),
                    None => self.go(State::Name(None)),
                }
            }
            Target::Field => self.retype = true,
            Target::Cancel => self.back(outs),
            Target::Stay => outs.push(Out::Close),
            Target::Anyway => outs.push(Out::SignOut),
            Target::Next => self.name(now, get, outs),
            Target::Remove => {
                if let State::Confirm(id) = self.state {
                    if self.change(Op::Remove(id), get, outs) {
                        self.go(State::Pick);
                    }
                }
            }
            Target::Card | Target::Back => {}
        }
    }

    /// Gives the name typed: a new profile (then signed in to) or a profile's new name.
    fn name(&mut self, now: f64, get: &dyn Fn(&str) -> Option<String>, outs: &mut Vec<Out>) {
        let name = match profiles::clean(&self.typed) {
            Ok(name) => name,
            Err(why) => return self.say(why, true),
        };
        let seed = u32::from_le_bytes([self.fresh[0], self.fresh[1], self.fresh[2], self.fresh[3]]);
        let op = match self.state {
            State::Name(Some(id)) => Op::Rename(id, name),
            _ => Op::Add { name, seed, pin: None },
        };
        let added = matches!(op, Op::Add { .. });
        if self.change(op, get, outs) {
            self.go(State::Pick);
            if added {
                let id = self.list.list.last().map_or(0, |p| p.id);
                self.focus = self.list.list.len() - 1;
                self.sign_in(id, now, outs);
            }
        }
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
            Some(_) if self.read_only => self.say(profiles::NEWER, false),
            Some(p) => (self.asked, self.focus) = (Some((p.id, at)), i),
            None => {}
        }
    }

    /// Does a circle's menu item for profile `id`: Rename (the name to edit), Remove (to
    /// confirm with the size of its files, unless it is the last).
    fn menu_act(&mut self, id: u32, act: Act, get: &dyn Fn(&str) -> Option<String>) {
        let Some(name) = self.list.get(id).map(|p| p.name.clone()) else { return };
        match act {
            Act::Rename => {
                self.go(State::Name(Some(id)));
                self.typed = name;
            }
            Act::Remove if self.list.list.len() <= 1 => self.say(profiles::LAST_ONE, true),
            Act::Remove => {
                self.go(State::Confirm(id));
                self.files = get(&key(id, "home")).map_or(0, |h| h.chars().count());
            }
        }
    }

    /// Signs in to profile `id`: the device has said hello and remembers who signed in last,
    /// the tab keeps its session, and the flight begins.
    fn sign_in(&mut self, id: u32, now: f64, outs: &mut Vec<Out>) {
        let mut n = String::new();
        ui::push_num(&mut n, id as usize);
        if !self.seen {
            outs.push(Out::Set(SEEN.into(), "1".into()));
        }
        outs.extend([Out::Set(LAST.into(), n.clone()), Out::Session(n), Out::SignIn(id)]);
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
