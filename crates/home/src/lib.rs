//! The compusophyOS home screen, no browser: the parts of the desktop around and behind the
//! windows, which the shell wires to its input and draws in the frame's [`ui::Theme`].
//!
//! - [`bar`]: the top bar, its buttons and clock;
//! - [`icons`]: every app as an icon in a grid behind the windows, in the person's order;
//! - [`grid`]: the grid as it behaves: listed, carried to a new place, selected by a box;
//! - [`dock`]: the bottom row, one tile high: the dock at its left (the person's favorites, kept
//!   as the `dock` preference and moved as icons are, then the others running) and the
//!   Assistant alone at the bottom-right corner;
//! - [`menu`]: context menus, for right clicks and long presses;
//! - [`touch`]: a finger scrolling what it holds, flinging it on, or long-pressing.
//!
//! Logical pixels, origin top-left; screens are `(w, h)`. A narrow screen (under
//! [`host::NARROW`] px) gets touch-sized targets.

#![forbid(unsafe_code)]

pub mod bar;
pub mod dock;
pub mod grid;
pub mod icons;
pub mod menu;
pub mod touch;

use gfx::{DrawList, RectF};
use ui::{AppIcon, TextSystem, Theme};

/// What the work area leaves free at the bottom: the bottom row ([`dock::ROW`]).
pub const CLEAR: f32 = dock::ROW;

/// Whether a screen `w` wide is a phone's.
pub fn narrow(w: f32) -> bool {
    w < host::NARROW as f32
}

/// The names of a stored list (joined by commas), each once.
pub fn names(stored: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in stored.split(',') {
        if !name.is_empty() && !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
    }
    out
}

/// `names` as a stored list: joined by commas, each once.
pub fn joined(names: &[String]) -> String {
    let mut out = String::new();
    for n in names.iter().filter(|n| !n.contains(',')) {
        if !out.is_empty() {
            out.push(',');
        }
        out.push_str(n);
    }
    out
}

/// An app's tile in the square `r`: its glyph's ([`ui::icon::tile`]), or its sigil's if a `.app`
/// file's.
pub fn tile(
    list: &mut DrawList,
    text: &mut TextSystem,
    r: RectF,
    (icon, sigil): (AppIcon, Option<u32>),
    theme: &Theme,
) {
    match sigil {
        Some(seed) => ui::icon::sigil_tile(list, text, r, seed, icon.hue, theme),
        None => ui::icon::tile(list, text, r, icon.glyph, icon.hue, theme),
    }
}

#[cfg(test)]
mod tests;
