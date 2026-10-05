//! The screen as the overlay's AI reads it, sent in each [`crate::Event::Acted`]: the screen's
//! size, the theme, the focused window, the home screen's apps, and each window top first with
//! its frame, its widgets' hit regions, their marks (role, state, value) and the text it shows;
//! then, on a touch screen, a `u8` 1 (nothing on another, so a scene from a desktop older than
//! the flag reads as one). Little-endian as the rest of the protocol; a rect is four `i16` (x,
//! y, w, h) in logical px; a list is a `u32` count, then its items. Decoding is strict, so the
//! encoding is canonical.

use crate::{MAX_FRAME, Out, Reader};

/// A rect in logical px: x, y, w, h.
pub type Rect = [i16; 4];

/// One look at the desktop: `w` x `h`, whether it is a `touch` screen (the person's last press
/// was a finger's: they tap, with no keys but an on-screen keyboard's; sent last), the theme's
/// name, the focused window (0: none), the apps the home screen offers, and the windows, top
/// first (minimized ones last).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scene {
    pub w: u16,
    pub h: u16,
    pub touch: bool,
    pub theme: String,
    pub focus: u32,
    pub apps: Vec<String>,
    pub wins: Vec<Win>,
}

/// A window: its id, the app in it, its title, its frame, its [`state`], and for a shown one its
/// content's hits, marks and runs of text (none for a minimized one).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Win {
    pub id: u32,
    pub app: String,
    pub title: String,
    pub rect: Rect,
    pub state: u8,
    pub hits: Vec<Hit>,
    pub marks: Vec<Mark>,
    pub runs: Vec<Run>,
}

/// A [`Win`]'s state: free, maximized, minimized, snapped.
pub mod state {
    pub const FREE: u8 = 0;
    pub const MAX: u8 = 1;
    pub const MIN: u8 = 2;
    pub const SNAPPED: u8 = 3;
}

/// A region that answers the pointer: widget `id`, its sense (0 click, 1 text, 2 scroll), where.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hit {
    pub id: u32,
    pub sense: u8,
    pub rect: Rect,
}

/// What widget `id` says of itself: its role and state flags (the `ui::sem` codes) and value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mark {
    pub id: u32,
    pub role: u8,
    pub flags: u8,
    pub value: String,
}

/// A line of text the window shows, and where.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Run {
    pub rect: Rect,
    pub text: String,
}

wire!(Scene);

impl Scene {
    fn put(&self, o: &mut Out) {
        o.u16(self.w).u16(self.h).str(&self.theme).u32(self.focus).len(self.apps.len());
        self.apps.iter().for_each(|a| _ = o.str(a));
        o.len(self.wins.len());
        for w in &self.wins {
            rect(o.u32(w.id).str(&w.app).str(&w.title), w.rect).u8(w.state).len(w.hits.len());
            w.hits.iter().for_each(|h| _ = rect(o.u32(h.id).u8(h.sense), h.rect));
            o.len(w.marks.len());
            w.marks.iter().for_each(|m| _ = o.u32(m.id).u8(m.role).u8(m.flags).str(&m.value));
            o.len(w.runs.len());
            w.runs.iter().for_each(|r| _ = rect(o, r.rect).str(&r.text));
        }
        if self.touch {
            o.u8(1);
        }
    }

    fn get(r: &mut Reader<'_>) -> Option<Self> {
        let (w, h, theme, focus) = (r.u16()?, r.u16()?, r.str()?, r.u32()?);
        let apps = list(r, 4, |r| r.str())?;
        let wins = list(r, 33, |r| {
            let (id, app, title, rect) = (r.u32()?, r.str()?, r.str()?, get_rect(r)?);
            let state = r.u8().filter(|s| *s <= state::SNAPPED)?;
            let hits = list(r, 13, |r| {
                let (id, sense) = (r.u32()?, r.u8().filter(|s| *s <= 2)?);
                Some(Hit { id, sense, rect: get_rect(r)? })
            })?;
            let marks = list(r, 10, |r| {
                let (id, role, flags) = (r.u32()?, r.u8()?, r.u8()?);
                Some(Mark { id, role, flags, value: r.str()? })
            })?;
            let runs = list(r, 12, |r| Some(Run { rect: get_rect(r)?, text: r.str()? }))?;
            Some(Win { id, app, title, rect, state, hits, marks, runs })
        })?;
        // The touch flag, there only when set: 1, or nothing.
        let touch = !r.0.is_empty() && r.u8().filter(|t| *t == 1)? == 1;
        Some(Scene { w, h, touch, theme, focus, apps, wins })
    }
}

fn rect(o: &mut Out, r: Rect) -> &mut Out {
    r.iter().fold(o, |o, v| o.i16(*v))
}

fn get_rect(r: &mut Reader<'_>) -> Option<Rect> {
    Some([r.i16()?, r.i16()?, r.i16()?, r.i16()?])
}

/// A counted list of items at least `min` bytes each, so a count can never outrun the input.
fn list<T>(
    r: &mut Reader<'_>,
    min: usize,
    item: fn(&mut Reader<'_>) -> Option<T>,
) -> Option<Vec<T>> {
    let n = r.count().filter(|n| *n <= r.0.len() / min)?;
    (0..n).map(|_| item(r)).collect()
}
