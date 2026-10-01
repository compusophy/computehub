//! compusophyOS's built-in apps: [`Welcome`] (the first screen and a list of the apps),
//! [`Files`] (the filesystem, a folder at a time), [`Settings`] (themes, the AI, privacy),
//! [`About`] (what compusophyOS is and is made of), [`Feedback`] (straight to compusophy) and
//! [`Terminal`] (the guest shell on an xterm screen). Each draws through [`ui::Ui`] in the
//! frame's [`ui::Theme`] and reaches the world only through [`ui::Cx`].

#![forbid(unsafe_code)]

mod about;
mod feedback;
mod files;
mod kit;
mod settings;
mod terminal;
mod welcome;

pub use about::About;
pub use feedback::Feedback;
pub use files::Files;
pub use settings::Settings;
pub use terminal::Terminal;
pub use welcome::Welcome;

/// The names [`open`] knows; it also opens `files:<dir>`, Files at that folder.
pub const NAMES: &[&str] = &["welcome", "terminal", "settings", "about", "feedback", "files"];

/// A new instance of the built-in app `name` (one of [`NAMES`], or `files:<dir>`).
pub fn open(name: &str) -> Option<Box<dyn ui::App>> {
    if let Some(dir) = name.strip_prefix("files:") {
        return Some(Box::new(Files::new(dir)));
    }
    Some(match name {
        "welcome" => Box::new(Welcome::default()),
        "terminal" => Box::new(Terminal::default()),
        "settings" => Box::new(Settings::default()),
        "about" => Box::new(About::default()),
        "feedback" => Box::new(Feedback::default()),
        "files" => Box::new(Files::default()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
