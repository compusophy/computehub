//! The compusophyOS home screen, no browser: the parts of the desktop around and behind the
//! windows, which the shell wires to its input and draws in the frame's [`ui::Theme`].
//!
//! - [`dock`]: the person's favorite apps (kept as the `dock` preference), the other running
//!   ones, and the Apps button, on a shelf;
//! - [`field`]: the everything bar, where anything typed opens an app or asks the Assistant,
//!   with [`panel`], the launcher's results, anchored above it;
//! - [`icons`]: the desktop's icon grid behind the windows;
//! - [`menu`]: context menus, for right clicks and long presses;
//! - [`touch`]: a finger scrolling what it holds, flinging it on, or long-pressing.
//!
//! Logical pixels, origin top-left; screens are `(w, h)`. A narrow screen (under
//! [`host::NARROW`] px) gets touch-sized targets and full-width sheets.

#![forbid(unsafe_code)]

pub mod dock;
pub mod field;
pub mod icons;
pub mod menu;
pub mod panel;
pub mod touch;

/// What the work area leaves free at the bottom: the everything bar, the dock, a gap above each.
pub const CLEAR: f32 = field::BOTTOM + field::H + dock::GAP + dock::H + dock::GAP;

/// Whether a screen `w` wide is a phone's.
pub fn narrow(w: f32) -> bool {
    w < host::NARROW as f32
}

#[cfg(test)]
mod tests;
