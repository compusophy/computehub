//! Studio: write applang apps, check them and run them, each in its own
//! window.
//!
//! - [`Studio`] is the editor: a Mono 13 text area with a line-number
//!   gutter, Run / Save / New, and a list of problems after a failed Run.
//! - [`AppHost`] runs one compiled `.app` file with `applang::App` and draws
//!   its widget tree with the `ui` toolkit.
//! - [`open`] turns a window name into one of them; [`install_samples`]
//!   puts the sample apps in `/apps`.
//!
//! Both apps read the filesystem only through the [`ui::Cx`] of an event,
//! so [`open`] returns them unloaded and they load on their first event.
//! [`open_in`] loads at once from a filesystem at hand.
//!
//! ```
//! use ui::App;
//!
//! let mut fs = vfs::Vfs::new();
//! studio::install_samples(&mut fs);
//! let host = studio::open_in("/apps/counter.app", &fs).unwrap();
//! assert_eq!(host.title(), "counter.app");
//! assert_eq!(studio::open("studio").unwrap().title(), "Studio — counter.app");
//! assert!(studio::open("terminal").is_none());
//! ```

#![forbid(unsafe_code)]

mod editor;
mod host;
mod studio;

use std::fmt;

use lang::Diag;
use vfs::Vfs;

pub use editor::Editor;
pub use host::AppHost;
pub use studio::Studio;

/// The file [`open`]`("studio")` edits.
pub const DEFAULT_FILE: &str = "/apps/counter.app";

/// The sample apps [`install_samples`] writes, as `(path, source)`.
pub const SAMPLES: [(&str, &str); 3] =
    [(DEFAULT_FILE, COUNTER), ("/apps/greeter.app", GREETER), ("/apps/clicker.app", CLICKER)];

const COUNTER: &str = "\
// A counter: two buttons change one number.
state count = 0;

label \"Counter\";
row {
  button \"-\" { count = count - 1; }
  label count;
  button \"+\" { count = count + 1; }
}
button \"Reset\" { count = 0; }
if count >= 10 {
  label \"That is a lot of clicks.\";
}
";

const GREETER: &str = "\
// A greeter: type a name and it says hello.
state name = \"\";
state waves = 0;

label \"What is your name?\";
input name;
if name == \"\" {
  label \"Type your name above.\";
} else {
  label \"Hello, \" + name + \"!\";
  button \"Wave\" { waves = waves + 1; }
  if waves > 0 {
    label \"You waved \" + waves + \" times.\";
  }
}
";

const CLICKER: &str = "\
// Clicker: mine coins, then spend them on a better pick.
state coins = 0;
state power = 1;
state cost = 10;
state swings = 0;

label \"Coins: \" + coins;
label \"Pick power: \" + power;
row {
  button \"Mine\" {
    coins = coins + power;
    swings = swings + 1;
  }
  button \"Upgrade\" {
    if coins >= cost {
      coins = coins - cost;
      power = power + 1;
      cost = cost * 2;
    }
  }
}
label \"Next upgrade: \" + cost + \" coins\";
if swings >= 50 { label \"Seasoned miner: \" + swings + \" swings\"; }
";

/// What New starts a file with.
const NEW_APP: &str = "\
// A new app. Run it with the Run button or Ctrl+Enter.
state count = 0;

label \"Hello from applang!\";
button \"Click me\" { count = count + 1; }
label \"Clicks: \" + count;
";

/// Writes each of [`SAMPLES`] that is not there yet, making `/apps` if
/// needed; a file that exists is left as it is. Failures (a full
/// filesystem) are skipped.
pub fn install_samples(vfs: &mut Vfs) {
    let _ = vfs.mkdir_all("/apps");
    for (path, src) in SAMPLES {
        if !vfs.exists(path) {
            let _ = vfs.write(path, src.as_bytes());
        }
    }
}

/// The app for `name`, loaded from `vfs` if given.
fn build(name: &str, vfs: Option<&Vfs>) -> Option<Box<dyn ui::App>> {
    let abs = |p: &str| Vfs::normalize("/apps", p).ok().filter(|_| !p.is_empty());
    let studio = match name.strip_prefix("studio:") {
        Some(p) => Some(abs(p)?),
        None => (name == "studio").then(|| DEFAULT_FILE.to_string()),
    };
    if let Some(path) = studio {
        let mut s = Studio::new(&path);
        if let Some(fs) = vfs {
            s.load(fs);
        }
        Some(Box::new(s))
    } else if name.ends_with(".app") {
        let mut h = AppHost::new(&abs(name)?);
        if let Some(fs) = vfs {
            h.load(fs);
        }
        Some(Box::new(h))
    } else {
        None
    }
}

/// The app for a window name: `"studio"` is [`Studio`] on
/// [`DEFAULT_FILE`], `"studio:<path>"` is Studio on that file, and any path
/// ending in `.app` is an [`AppHost`] running it. Anything else is `None`.
/// A relative path is under `/apps`. The app reads its file on its first
/// event.
pub fn open(name: &str) -> Option<Box<dyn ui::App>> {
    build(name, None)
}

/// [`open`], with the file read from `vfs` at once.
pub fn open_in(name: &str, vfs: &Vfs) -> Option<Box<dyn ui::App>> {
    build(name, Some(vfs))
}

/// One diagnostic as Studio and AppHost list it: `E0302 3:7 message`,
/// with a 1-based line and a column in chars. A diagnostic with no code
/// shows `error` in its place; one with no span has no position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// The stable code (see `applang::codes`).
    pub code: Option<u16>,
    /// Line and column where it starts.
    pub pos: Option<(usize, usize)>,
    /// What went wrong.
    pub message: String,
}

impl Problem {
    /// A diagnostic from compiling or running `src`.
    pub fn new(d: &Diag, src: &str) -> Problem {
        Problem {
            code: d.code,
            pos: d.span.map(|s| lang::diag::line_col(src, s.start)),
            message: d.message.clone(),
        }
    }

    /// The code (`E0302` or `error`) and the position (`3:7`, or empty).
    fn head(&self) -> (String, String) {
        let mut code = String::new();
        match self.code {
            Some(c) => {
                code.push('E');
                push_num(&mut code, c.into(), 4);
            }
            None => code.push_str("error"),
        }
        let mut pos = String::new();
        if let Some((l, c)) = self.pos {
            push_num(&mut pos, l, 1);
            pos.push(':');
            push_num(&mut pos, c, 1);
        }
        (code, pos)
    }

    /// The whole line: `E0302 3:7 message`, or `error message`.
    pub(crate) fn line(&self) -> String {
        let (mut out, pos) = self.head();
        out.push(' ');
        if !pos.is_empty() {
            out.push_str(&pos);
            out.push(' ');
        }
        out.push_str(&self.message);
        out
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.line())
    }
}

/// The last name in `path`.
fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// `parts` joined into one string.
fn join(parts: &[&str]) -> String {
    let mut out = String::new();
    for p in parts {
        out.push_str(p);
    }
    out
}

/// Appends `n` in decimal, zero-padded to at least `width` digits.
fn push_num(out: &mut String, mut n: usize, width: usize) {
    let mut digits = [b'0'; 20];
    let mut i = digits.len();
    loop {
        i -= 1;
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    for &d in &digits[i.min(digits.len().saturating_sub(width))..] {
        out.push(char::from(d));
    }
}

#[cfg(test)]
mod tests;
