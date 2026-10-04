//! compusophyOS's built-in app: the [`Terminal`] (the guest shell on an xterm screen). It draws
//! through [`ui::Ui`] in the frame's [`ui::Theme`] and reaches the world only through
//! [`ui::Cx`]. About, Feedback, Files, Settings and Welcome are programs (the `system` crate), off
//! the boot download.

#![forbid(unsafe_code)]

mod kit;
mod terminal;

pub use terminal::Terminal;

/// The names [`open`] knows.
pub const NAMES: &[&str] = &["terminal"];

/// A new instance of the built-in app `name` (one of [`NAMES`]).
pub fn open(name: &str) -> Option<Box<dyn ui::App>> {
    Some(match name {
        "terminal" => Box::new(Terminal::default()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
