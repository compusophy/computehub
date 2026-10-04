//! Context menus: a rounded panel of items (a label, maybe a hint such as a shortcut) and
//! hairline separators, opened at a point, kept on screen, and driven by the pointer and keys.

use std::ops::{Deref, DerefMut};

use gfx::{DrawList, RectF, Rgba};
use host::paint::{cap_baseline, px, sheen};
use ui::{FontId, Key, TextStyle, TextSystem, Theme};

/// An item's height for a mouse and for touch, a separator's, the padding around the items, the
/// labels' inset in their items, the least width, the space before a hint, the margin kept from
/// the screen's edges, the panel's corner radius, and the label's size.
const ITEM_H: f32 = 34.0;
const TOUCH_H: f32 = 44.0;
const SEP_H: f32 = 13.0;
const PAD: f32 = 5.0;
const INSET: f32 = 13.0;
const MIN_W: f32 = 180.0;
const HINT_GAP: f32 = 34.0;
const MARGIN: f32 = 8.0;
const RADIUS: f32 = 13.0;
const SIZE: f32 = 14.0;

/// One line of a menu: its label, hint and action; a separator has none.
pub type Item<A> = (&'static str, &'static str, Option<A>);

/// An open menu: its panel ([`Panel`], which it derefs to) and each item's action.
#[derive(Clone, Debug, PartialEq)]
pub struct Menu<A> {
    pub panel: Panel,
    acts: Vec<Option<A>>,
}

/// A menu's panel, whatever its actions are (one copy of its code serves every menu): its items
/// (a label, a hint, and whether it acts: a separator does not), its rect, its items' height,
/// and the selected item (hovered or chosen by the keys).
#[derive(Clone, Debug, PartialEq)]
pub struct Panel {
    pub items: Vec<(&'static str, &'static str, bool)>,
    pub rect: RectF,
    pub row: f32,
    pub sel: Option<usize>,
}

impl<A> Deref for Menu<A> {
    type Target = Panel;
    fn deref(&self) -> &Panel {
        &self.panel
    }
}

impl<A> DerefMut for Menu<A> {
    fn deref_mut(&mut self) -> &mut Panel {
        &mut self.panel
    }
}

/// The labels' and the hints' style in `color` and `dim`.
fn styles(color: Rgba, dim: Rgba) -> [TextStyle; 2] {
    [TextStyle::new(FontId::Sans, SIZE, color), TextStyle::new(FontId::Sans, 12.0, dim)]
}

impl<A: Copy> Menu<A> {
    /// `items` at `at` on a `screen`, opening down and right of the point (else up or left),
    /// sized to its text; items touch-high if `touch`.
    pub fn new(
        items: &[Item<A>],
        at: (f32, f32),
        screen: (f32, f32),
        touch: bool,
        text: &mut TextSystem,
    ) -> Menu<A> {
        let rows = items.iter().map(|i| (i.0, i.1, i.2.is_some())).collect();
        let acts = items.iter().map(|i| i.2).collect();
        Menu { panel: Panel::new(rows, at, screen, touch, text), acts }
    }

    /// The action of item `i`.
    pub fn act(&self, i: usize) -> Option<A> {
        *self.acts.get(i)?
    }
}

impl Panel {
    fn new(
        items: Vec<(&'static str, &'static str, bool)>,
        (x, y): (f32, f32),
        screen: (f32, f32),
        touch: bool,
        text: &mut TextSystem,
    ) -> Panel {
        let row = if touch { TOUCH_H } else { ITEM_H };
        let [label, hint] = styles(Rgba(0, 0, 0, 0), Rgba(0, 0, 0, 0));
        let (mut w, mut h) = (MIN_W, 2.0 * PAD);
        for i in &items {
            let tip = if i.1.is_empty() { 0.0 } else { HINT_GAP + text.measure(i.1, hint) };
            w = w.max((text.measure(i.0, label) + tip + 2.0 * (PAD + INSET)).ceil());
            h += if i.2 { row } else { SEP_H };
        }
        let fit = |p: f32, len: f32, max: f32| {
            let p = if p + len > max - MARGIN { p - len } else { p };
            p.min(max - MARGIN - len).max(MARGIN).round()
        };
        let rect = RectF::new(fit(x, w, screen.0), fit(y, h, screen.1), w, h);
        Panel { items, rect, row, sel: None }
    }

    /// Item `i`'s rect.
    pub fn item(&self, i: usize) -> RectF {
        let mut y = self.rect.y + PAD;
        for it in self.items.iter().take(i) {
            y += if it.2 { self.row } else { SEP_H };
        }
        let sep = self.items.get(i).is_some_and(|i| !i.2);
        RectF::new(
            self.rect.x + PAD,
            y,
            self.rect.w - 2.0 * PAD,
            if sep { SEP_H } else { self.row },
        )
    }

    /// The action at `(x, y)`: `Some(Some(i))` on one, `Some(None)` elsewhere inside, `None`
    /// outside.
    pub fn at(&self, x: f32, y: f32) -> Option<Option<usize>> {
        let on = |i: &usize| self.items[*i].2 && self.item(*i).contains(x, y);
        self.rect.contains(x, y).then(|| (0..self.items.len()).find(on))
    }

    /// Up and Down move the selection over the actions, around the ends; whether `key` did.
    pub fn key(&mut self, key: Key) -> bool {
        let n = self.items.len();
        let step = match key {
            Key::Down => 1,
            Key::Up => n.saturating_sub(1),
            _ => return false,
        };
        let mut i = self.sel.unwrap_or(if step == 1 { n.saturating_sub(1) } else { 0 });
        for _ in 0..n {
            i = (i + step) % n;
            if self.items[i].2 {
                self.sel = Some(i);
                break;
            }
        }
        true
    }

    /// The panel and its items, the selected one washed (more while `held`).
    pub fn draw(&self, list: &mut DrawList, text: &mut TextSystem, theme: &Theme, held: bool) {
        let (r, line) = (self.rect, px(text, 1.0));
        let [label, hint] = styles(theme.text, theme.text_dim);
        list.shadow_offset(r, RADIUS, 34.0, 13.0, theme.shadow);
        list.fill(r, RADIUS, theme.base);
        list.fill(r, RADIUS, theme.surface);
        list.border(r, RADIUS, line, theme.border);
        sheen(list, r, RADIUS, line, theme.highlight);
        for (i, &(name, tip, acts)) in self.items.iter().enumerate() {
            let it = self.item(i);
            if !acts {
                let y = text.snap(it.y + it.h / 2.0);
                list.fill(RectF::new(it.x + INSET, y, it.w - 2.0 * INSET, line), 0.0, theme.border);
                continue;
            }
            if self.sel == Some(i) {
                list.fill(it, RADIUS - PAD, theme.wash(held));
            }
            text.draw_text(
                list,
                text.snap(it.x + INSET),
                cap_baseline(text, it.y, it.h, SIZE),
                name,
                label,
            );
            let x = it.x + it.w - INSET - text.measure(tip, hint);
            text.draw_text(list, text.snap(x), cap_baseline(text, it.y, it.h, 12.0), tip, hint);
        }
    }
}
