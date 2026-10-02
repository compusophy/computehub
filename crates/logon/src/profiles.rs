//! Profiles: each its own home in this browser, one in memory per page. A profile keeps its
//! files, theme, dock, home order, grain, AI model, report consent and outbox under its own
//! `localStorage` keys ([`key`], [`PER_PROFILE`]): profile 0's are the keys from before
//! profiles (`compusophy.<k>`), so nothing was ever moved, and profile n's are
//! `compusophy.<n>.<k>`. The device keeps the list ([`LIST`]), the profile that signed in last
//! ([`LAST`]) and whether a welcome said hello ([`crate::SEEN`]).
//!
//! The list, `CSPR 1 <next id>`, then a line a profile: its id, its seed (8 hex: its face),
//! `-` (or, from v1.2, its PIN's record) and its name, the rest of the line. Absent, it is the
//! implied `0 <fnv("guest")> - guest`, written only at the first change. Ids are never reused;
//! a new profile also skips any id whose keys are still there, so no profile inherits another's
//! files. Failures are coded: a damaged list is set aside (only profile 0 is offered), a newer
//! one is read-only.

use std::cell::Cell;

/// The device's keys: the list, a damaged one set aside, the profile that signed in last.
pub const LIST: &str = "compusophy.profiles";
pub const LIST_BAD: &str = "compusophy.profiles.bad";
pub const LAST: &str = "compusophy.last";
/// What a profile keeps, each under [`key`].
pub const PER_PROFILE: [&str; 10] = [
    "home",
    "home.bad",
    "home.mark",
    "theme",
    "dock",
    "home.order",
    "grain",
    "ai.model",
    "reports",
    "outbox",
];
/// The most profiles a browser holds, and a name's longest (chars).
pub const MAX: usize = 8;
pub const NAME_MAX: usize = 24;

/// The refusals, as said.
pub const NO_NAME: &str = "Give it a name.";
pub const TAKEN: &str = "That name is taken.";
pub const FULL: &str = "This browser holds eight profiles at most.";
pub const LAST_ONE: &str = "A browser keeps at least one profile.";
pub const NEWER: &str = "made by a newer compusophyOS";
pub const GONE: &str = "That profile is gone.";
/// A damaged list, as reported.
pub const DAMAGED: &str = "Profiles did not read back (damaged); set aside";

thread_local! {
    /// The profile signed in to, once one is: the one whose keys [`own`] names.
    static ACTIVE: Cell<Option<u32>> = const { Cell::new(None) };
}

/// Profile `id` is signed in to: its keys are [`own`]'s from now on.
pub fn sign(id: u32) {
    ACTIVE.with(|a| a.set(Some(id)));
}

/// The profile signed in to, if any.
pub fn active() -> Option<u32> {
    ACTIVE.with(Cell::get)
}

/// The `localStorage` key of profile `id`'s `k` (`theme`, `home`, ...).
pub fn key(id: u32, k: &str) -> String {
    let mut out = String::from("compusophy.");
    if id > 0 {
        ui::push_num(&mut out, id as usize);
        out.push('.');
    }
    out + k
}

/// The signed-in profile's `k` (or profile 0's before a sign-in); `k` may be a first profile's
/// whole key (`compusophy.theme`).
pub fn own(k: &str) -> String {
    key(active().unwrap_or(0), k.strip_prefix("compusophy.").unwrap_or(k))
}

/// FNV-1a of `s`, as a `.app` file's sigil is seeded: profile 0's face is `guest`'s.
pub fn fnv(s: &str) -> u32 {
    s.bytes().fold(2_166_136_261, |h, b| (h ^ u32::from(b)).wrapping_mul(16_777_619))
}

/// One profile: its id, its face's seed, its PIN's record (none until v1.2), its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub id: u32,
    pub seed: u32,
    pub pin: Option<String>,
    pub name: String,
}

/// The list: the next id to give, and the profiles (one at least, [`MAX`] at most).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profiles {
    pub next: u32,
    pub list: Vec<Profile>,
}

/// A change to the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Add { name: String, seed: u32, pin: Option<String> },
    Rename(u32, String),
    SetPin(u32, Option<String>),
    Remove(u32),
}

impl Profiles {
    /// The one profile there is before any change: `guest`.
    pub fn implied() -> Profiles {
        let guest = Profile { id: 0, seed: fnv("guest"), pin: None, name: "guest".into() };
        Profiles { next: 1, list: vec![guest] }
    }

    /// The list as stored (absent: [`Profiles::implied`]), and why it could not be read as is:
    /// [`DAMAGED`] (then the implied list stands in) or [`NEWER`] (read-only).
    pub fn read(stored: Option<&str>) -> (Profiles, Option<&'static str>) {
        let Some(s) = stored else { return (Profiles::implied(), None) };
        let mut lines = s.split('\n');
        let head: Vec<&str> = lines.next().unwrap_or_default().split(' ').collect();
        let (version, next) = match head[..] {
            ["CSPR", v, n] => (v.parse::<u32>().ok(), n.parse::<u32>().ok()),
            _ => (None, None),
        };
        let newer = version.is_some_and(|v| v > 1);
        let list = next.filter(|_| version == Some(1) || newer).and_then(|next| {
            let mut list: Vec<Profile> = Vec::new();
            for l in lines {
                let p = line(l).filter(|p| p.id < next && list.len() < MAX)?;
                let dup = |q: &Profile| q.id == p.id || q.name.eq_ignore_ascii_case(&p.name);
                if list.iter().any(dup) {
                    return None;
                }
                list.push(p);
            }
            (!list.is_empty()).then_some(Profiles { next, list })
        });
        match (list, newer) {
            (Some(list), false) => (list, None),
            (Some(list), true) => (list, Some(NEWER)),
            (None, true) => (Profiles::implied(), Some(NEWER)),
            (None, false) => (Profiles::implied(), Some(DAMAGED)),
        }
    }

    /// The list as stored.
    pub fn format(&self) -> String {
        let mut out = String::from("CSPR 1 ");
        ui::push_num(&mut out, self.next as usize);
        for p in &self.list {
            out.push('\n');
            ui::push_num(&mut out, p.id as usize);
            out.push(' ');
            (0..8).rev().for_each(|i| out.push(hex(p.seed >> (4 * i))));
            out = out + " " + p.pin.as_deref().unwrap_or("-") + " " + &p.name;
        }
        out
    }

    /// Profile `id`.
    pub fn get(&self, id: u32) -> Option<&Profile> {
        self.list.iter().find(|p| p.id == id)
    }

    /// Makes the change `op` (a new profile's id is the first from `next` whose keys are not
    /// `taken`); the id it touched, or why not.
    pub fn apply(&mut self, op: Op, taken: &dyn Fn(u32) -> bool) -> Result<u32, &'static str> {
        let unique = |list: &[Profile], name: &str, id: Option<u32>| {
            let clash = |p: &Profile| Some(p.id) != id && p.name.eq_ignore_ascii_case(name);
            if list.iter().any(clash) { Err(TAKEN) } else { Ok(()) }
        };
        match op {
            Op::Add { name, seed, pin } => {
                if self.list.len() >= MAX {
                    return Err(FULL);
                }
                unique(&self.list, &name, None)?;
                let id = (self.next..self.next.saturating_add(64)).find(|&id| !taken(id));
                let id = id.filter(|&id| id < u32::MAX).ok_or(FULL)?;
                self.next = id + 1;
                self.list.push(Profile { id, seed, pin, name });
                Ok(id)
            }
            Op::Rename(id, name) => {
                unique(&self.list, &name, Some(id))?;
                self.list.iter_mut().find(|p| p.id == id).ok_or(GONE)?.name = name;
                Ok(id)
            }
            Op::SetPin(id, pin) => {
                self.list.iter_mut().find(|p| p.id == id).ok_or(GONE)?.pin = pin;
                Ok(id)
            }
            Op::Remove(id) => {
                self.get(id).ok_or(GONE)?;
                if self.list.len() <= 1 {
                    return Err(LAST_ONE);
                }
                self.list.retain(|p| p.id != id);
                Ok(id)
            }
        }
    }
}

/// A list line: `<id> <seed hex> <pin or -> <name>`.
fn line(l: &str) -> Option<Profile> {
    let mut f = l.splitn(4, ' ');
    let id = f.next()?.parse().ok()?;
    let seed = f.next().filter(|s| s.len() == 8).and_then(|s| u32::from_str_radix(s, 16).ok())?;
    let pin = Some(f.next()?).filter(|p| *p != "-").map(str::to_string);
    let name = f.next()?;
    (clean(name).ok()? == name).then(|| Profile { id, seed, pin, name: name.into() })
}

/// A name as kept: control chars dropped, trimmed, at most [`NAME_MAX`] chars; or why not.
pub fn clean(name: &str) -> Result<String, &'static str> {
    let kept: String = name.chars().filter(|c| !c.is_control()).collect();
    let cut: String = kept.trim().chars().take(NAME_MAX).collect();
    let cut = cut.trim_end();
    if cut.is_empty() { Err(NO_NAME) } else { Ok(cut.into()) }
}

/// The hex digit of `n`'s low four bits.
fn hex(n: u32) -> char {
    char::from(b"0123456789abcdef"[n as usize & 15])
}

/// Whether the stored list (`stored`) still holds the signed-in profile: a profile removed in
/// another tab keeps nothing more here.
pub fn listed(stored: Option<&str>) -> bool {
    let id = active().unwrap_or(0);
    Profiles::read(stored).0.get(id).is_some()
}

/// Whether a panic stays unreported: the signed-in profile turned reports off, or, before
/// anyone signed in, any listed profile did (`get` reads `localStorage`).
pub fn quiet(get: &dyn Fn(&str) -> Option<String>) -> bool {
    let off = |id: u32| get(&key(id, "reports")).as_deref() == Some("off");
    match active() {
        Some(id) => off(id),
        None => Profiles::read(get(LIST).as_deref()).0.list.iter().any(|p| off(p.id)),
    }
}
