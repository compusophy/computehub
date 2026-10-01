//! compusophyOS's built-in apps, each a [`ui::App`] that draws through [`ui::Ui`]
//! in the frame's [`ui::Theme`] and reaches the world only through
//! [`ui::Cx`]. A new crate, not a fork.
//!
//! - [`Terminal`]: an xterm-compatible screen running the built-in guest
//!   shell over the VFS.
//! - [`Welcome`]: the first screen: what this is, and three ways to start.
//! - [`Settings`]: the themes, and what compusophyOS is made of.
//!
//! ```
//! let term = apps::open("terminal").unwrap();
//! assert_eq!((term.title().as_str(), term.icon().glyph), ("Terminal", ">_"));
//! assert!(term.wants_text_input());
//! assert!(apps::NAMES.iter().all(|name| apps::open(name).is_some()));
//! assert!(apps::open("launcher").is_none(), "the shell owns the launcher");
//! ```

#![forbid(unsafe_code)]

mod kit;
mod settings;
mod terminal;
mod welcome;

pub use settings::Settings;
pub use terminal::Terminal;
pub use welcome::Welcome;

/// The names [`open`] knows.
pub const NAMES: &[&str] = &["welcome", "terminal", "settings"];

/// A new instance of the built-in app `name` (one of [`NAMES`]).
pub fn open(name: &str) -> Option<Box<dyn ui::App>> {
    Some(match name {
        "welcome" => Box::new(Welcome::default()),
        "terminal" => Box::new(Terminal::default()),
        "settings" => Box::new(Settings::default()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
