//! compusophyOS's built-in apps, each a [`ui::App`] that draws through [`ui::Ui`]
//! and reaches the world only through [`ui::Cx`]. A new crate, not a fork.
//!
//! - [`Terminal`]: xterm-compatible. Paired with computehub-node (the page
//!   opened with its `#node=<port>&token=<hex>` link) it runs a real shell
//!   on your machine: PowerShell, bash, `vim`, the `claude` CLI. Otherwise
//!   it runs a built-in guest shell over the VFS.
//! - [`Welcome`]: what compusophyOS is, its keys, buttons to start.
//! - [`Launcher`]: a floating search over the built-in apps and every
//!   `*.app` in `/apps` and the guest's home.
//! - [`About`]: the version, the stack, credits and provenance.
//!
//! ```
//! let term = apps::open("terminal").unwrap();
//! assert_eq!(term.title(), "Terminal");
//! assert!(term.wants_text_input());
//! assert!(apps::NAMES.iter().all(|name| apps::open(name).is_some()));
//! ```

#![forbid(unsafe_code)]

mod info;
mod launcher;
mod terminal;

pub use info::{About, Welcome};
pub use launcher::Launcher;
pub use terminal::Terminal;

/// The names [`open`] knows.
pub const NAMES: &[&str] = &["welcome", "terminal", "launcher", "about"];

/// A new instance of the built-in app `name` (one of [`NAMES`]).
pub fn open(name: &str) -> Option<Box<dyn ui::App>> {
    Some(match name {
        "welcome" => Box::new(Welcome::default()),
        "terminal" => Box::new(Terminal::default()),
        "launcher" => Box::new(Launcher::default()),
        "about" => Box::new(About::default()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
