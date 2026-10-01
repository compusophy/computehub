//! compusophyOS's built-in apps: [`Welcome`] (the first screen and a list of the apps),
//! [`Settings`] (themes, the AI, privacy) and [`Terminal`] (the guest shell on an xterm screen).
//! Each draws through [`ui::Ui`] in the frame's [`ui::Theme`] and reaches the world only through
//! [`ui::Cx`]. About, Feedback and Files are programs (the `system` crate), off the boot download.

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
