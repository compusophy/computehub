//! The screen as the model reads it: a [`Scene`] as text, each window's widgets as elements with
//! refs (`e4`) that keep their number for the whole session, their role, label, value and state,
//! and the text around them in reading order.
//!
//! - A run of text belongs to the smallest hit holding its middle (a region that scrolls holds
//!   none: it is no element): the joined runs are that element's label (80 chars at most). A text
//!   field's run is its value, or, when it holds none, its placeholder. Runs in no hit are the
//!   window's text, a row a line (160 chars at most). Refs are given in reading order.
//! - The focused window in full (60 elements, 40 lines of text), the other shown windows their
//!   elements alone (20), minimized ones their titles; the whole at most [`MAX_TEXT`] bytes.

use uiwire::scene::{Hit, Mark, Scene, Win, state};

/// The most bytes of screen text a request carries.
pub const MAX_TEXT: usize = 12 << 10;
/// The roles of `ui::sem`, by code; 0 is the hit's sense's.
const ROLES: [&str; 12] = [
    "", "button", "tab", "switch", "option", "textbox", "item", "code", "terminal", "link", "grid",
    "canvas",
];
/// A canvas's role (`ui::sem::CANVAS`): its value is its size in units, then its shapes, a line
/// each (pixels as `pixels X Y, W x H squares of S units`, then on a small board its rows).
pub const CANVAS: u8 = 11;
/// A switch's role and its flag while on; a text field's role, its value what it holds.
pub const SWITCH: (u8, u8) = (3, 2);
pub const TEXTBOX: u8 = 5;
/// The most refs a session keeps; past it they start over.
const MAX_REFS: usize = 4096;

/// The session's refs: `e<n>` is entry `n - 1`, a window and the id its app gives a widget.
#[derive(Debug, Default)]
pub struct Refs(Vec<(u32, u32)>);

impl Refs {
    /// The ref number of widget `id` of window `win`, new if it has none.
    pub fn of(&mut self, win: u32, id: u32) -> usize {
        if let Some(i) = self.0.iter().position(|r| *r == (win, id)) {
            return i + 1;
        }
        if self.0.len() >= MAX_REFS {
            self.0.clear();
        }
        self.0.push((win, id));
        self.0.len()
    }

    /// The window and widget `e<n>` names.
    pub fn get(&self, n: usize) -> Option<(u32, u32)> {
        self.0.get(n.checked_sub(1)?).copied()
    }
}

/// An element as last shown: its ref, window, widget id, role and label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Elem {
    pub n: usize,
    pub win: u32,
    pub id: u32,
    pub role: &'static str,
    pub name: String,
}

/// `scene` as text, opened over window `over` (0: none), and the elements it shows.
pub fn render(scene: &Scene, refs: &mut Refs, over: u32) -> (String, Vec<Elem>) {
    let mut out = format!("Screen {}x{}, theme {}", scene.w, scene.h, scene.theme);
    if scene.focus != 0 {
        out += &format!(", focused w{}", scene.focus);
    }
    out.push('.');
    if over != 0 {
        out += &format!(" You were opened over w{over}.");
    }
    out += &format!("\nApps: {}\n", scene.apps.join(", "));
    let mut elems = Vec::new();
    for w in &scene.wins {
        if out.len() >= MAX_TEXT {
            break;
        }
        let r = w.rect;
        let st =
            [", maximized", ", minimized", ", snapped"].get(usize::from(w.state).wrapping_sub(1));
        let focused = if w.id == scene.focus { ", focused" } else { "" };
        out += &format!(
            "w{} {} ({}) {}x{} at {},{}",
            w.id,
            quoted(&w.title),
            w.app,
            r[2],
            r[3],
            r[0],
            r[1]
        );
        out += &[focused, st.copied().unwrap_or("")].concat();
        out.push('\n');
        if w.state != state::MIN {
            window(w, w.id == scene.focus, refs, &mut out, &mut elems);
        }
    }
    (clip(&out, MAX_TEXT), elems)
}

/// One shown window's elements, and in full its text, in reading order.
fn window(w: &Win, full: bool, refs: &mut Refs, out: &mut String, elems: &mut Vec<Elem>) {
    // Each widget once, where its topmost hit is.
    let mut hits: Vec<Hit> = Vec::new();
    for h in &w.hits {
        match hits.iter_mut().find(|k| k.id == h.id) {
            Some(k) => *k = *h,
            None => hits.push(*h),
        }
    }
    hits.retain(|h| h.sense != 2);
    let area = |r: [i16; 4]| i32::from(r[2]) * i32::from(r[3]);
    let mut names = vec![String::new(); hits.len()];
    let mut lines: Vec<([i16; 4], String)> = Vec::new();
    for run in &w.runs {
        let (cx, cy) = (
            i32::from(run.rect[0]) + i32::from(run.rect[2]) / 2,
            i32::from(run.rect[1]) + i32::from(run.rect[3]) / 2,
        );
        let holds = |h: &&Hit| inside(h.rect, cx, cy);
        let owner = hits.iter().filter(holds).min_by_key(|h| area(h.rect));
        match owner.and_then(|o| hits.iter().position(|h| h.id == o.id)) {
            Some(i) => join(&mut names[i], &run.text, 80),
            // A row of text is one line.
            None => match lines.last_mut().filter(|l| (l.0[1] - run.rect[1]).abs() <= 2) {
                Some(l) => join(&mut l.1, &run.text, 160),
                None => lines.push((run.rect, clip(&run.text, 160))),
            },
        }
    }
    // Elements (refs given as they come) and lines of text, in reading order.
    let rects: Vec<[i16; 4]> =
        hits.iter().map(|h| h.rect).chain(lines.iter().map(|l| l.0)).collect();
    let key = reading(&rects);
    let mut order: Vec<usize> = (0..rects.len()).collect();
    order.sort_by_key(|&i| key(rects[i]));
    let (most, mut shown, mut said) = (if full { 60 } else { 20 }, 0, 0);
    for i in order {
        let Some(h) = hits.get(i) else {
            if full && said < 40 {
                said += 1;
                *out += &["  ", &quoted(&lines[i - hits.len()].1), "\n"].concat();
            }
            continue;
        };
        if shown == most {
            continue;
        }
        let mark = w.marks.iter().rev().find(|m| m.id == h.id);
        let role = mark.and_then(|m| ROLES.get(usize::from(m.role))).filter(|r| !r.is_empty());
        let role = role.copied().unwrap_or(if h.sense == 1 { "textbox" } else { "button" });
        let (n, name) = (refs.of(w.id, h.id), std::mem::take(&mut names[i]));
        *out += &format!("  e{n} {role}{}\n", describe(role, &name, mark));
        elems.push(Elem { n, win: w.id, id: h.id, role, name });
        shown += 1;
    }
}

/// How `rects` read: top to bottom, left to right; but where a vertical line crosses none of
/// them with some on each side (a column of tabs beside a page), the left side first.
fn reading(rects: &[[i16; 4]]) -> impl Fn([i16; 4]) -> (bool, i16, i16) {
    let right = |r: &[i16; 4]| r[0].saturating_add(r[2]);
    let clear = |c: i16| rects.iter().all(|r| right(r) <= c || r[0] >= c);
    let split =
        rects.iter().map(right).filter(|&c| clear(c) && rects.iter().any(|r| r[0] >= c)).min();
    move |r| (split.is_some_and(|c| r[0] >= c), r[1], r[0])
}

/// An element's label, value and state after its role.
fn describe(role: &str, name: &str, mark: Option<&Mark>) -> String {
    let (flags, value) = mark.map_or((0, ""), |m| (m.flags, m.value.as_str()));
    let mut out = String::new();
    match role {
        "textbox" if !value.is_empty() => out += &format!(" value {}", quoted(&clip(value, 160))),
        "textbox" if !name.is_empty() => out += &format!(" placeholder {}", quoted(name)),
        "textbox" => out += " empty",
        // The text in its squares, then its columns and a row of square colors (0 empty) a line.
        "grid" => out += &format!(" {} squares {}", quoted(name), quoted(value)),
        // The text drawn on it, then its size in units and its shapes, a line each: pixels by
        // where they are and their size, so a tap can aim at a square of them (and on a small
        // board its squares, a row a line).
        "canvas" => out += &format!(" {} shapes {}", quoted(name), quoted(value)),
        _ if !name.is_empty() => out += &[" ", &quoted(name)].concat(),
        _ => {}
    }
    let words =
        [(1, " selected"), (2, " on"), (4, " focused"), (8, " disabled"), (16, " more below")];
    for (bit, word) in words {
        if flags & bit != 0 {
            out += word;
        }
    }
    if role == "switch" && flags & 2 == 0 {
        out += " off";
    }
    out
}

/// Whether `(x, y)` lies in `r`.
fn inside(r: [i16; 4], x: i32, y: i32) -> bool {
    let [rx, ry, w, h] = r.map(i32::from);
    (rx..rx + w).contains(&x) && (ry..ry + h).contains(&y)
}

/// `to` and `more`, a space between, cut to `max` bytes.
fn join(to: &mut String, more: &str, max: usize) {
    if !to.is_empty() {
        to.push(' ');
    }
    to.push_str(more);
    *to = clip(to, max);
}

/// `s` in double quotes, its own quotes and line breaks escaped as in JSON.
fn quoted(s: &str) -> String {
    let mut out = String::from('"');
    for c in s.chars() {
        match c {
            '"' | '\\' => out.extend(['\\', c]),
            '\n' => out.push_str("\\n"),
            c if c < ' ' => {}
            c => out.push(c),
        }
    }
    out + "\""
}

/// `s` cut to at most `max` bytes on a char boundary, ending in `…` if cut.
pub fn clip(s: &str, max: usize) -> String {
    crate::ai::clip(s, max)
}
