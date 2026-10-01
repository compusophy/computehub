//! Studio: write applang apps, check them and run them, each in its own
//! window. [`Studio`] is the editor; [`AppHost`] runs one `.app` file with
//! `applang::App` and draws its widget tree with `ui`. [`open`] turns a
//! window name into one of them and [`install_samples`] puts the sample apps
//! in `/apps`. Both draw in the frame's [`ui::Theme`] and read the
//! filesystem only through an event's [`ui::Cx`], so they load on their
//! first event.

#![forbid(unsafe_code)]

mod editor;
mod host;
mod studio;

use lang::Diag;
use ui::{AppIcon, Rgba};
use vfs::Vfs;

pub use editor::Editor;
pub use host::AppHost;
pub use studio::Studio;

/// Studio's icon: braces on violet.
pub const STUDIO_ICON: AppIcon = AppIcon { glyph: "{ }", hue: Rgba::hex(0x8b7bff) };
/// The icon of every `.app` an [`AppHost`] runs: angle brackets on amber.
pub const APP_ICON: AppIcon = AppIcon { glyph: "<>", hue: Rgba::hex(0xf59e0b) };

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

/// Writes each of [`SAMPLES`] that is not there yet (making `/apps`),
/// leaving existing files alone; failures are skipped.
pub fn install_samples(vfs: &mut Vfs) {
    let _ = vfs.mkdir_all("/apps");
    for (path, src) in SAMPLES {
        if !vfs.exists(path) {
            let _ = vfs.write(path, src.as_bytes());
        }
    }
}

/// The app for a window name: `"studio"` is [`Studio`] on [`DEFAULT_FILE`],
/// `"studio:<path>"` is Studio on that file, and a path ending in `.app` is
/// an [`AppHost`] running it; a relative path is under `/apps`.
pub fn open(name: &str) -> Option<Box<dyn ui::App>> {
    let abs = |p: &str| Vfs::normalize("/apps", p).ok().filter(|_| !p.is_empty());
    if let Some(path) = name.strip_prefix("studio:") {
        Some(Box::new(Studio::new(&abs(path)?)))
    } else if name == "studio" {
        Some(Box::new(Studio::new(DEFAULT_FILE)))
    } else if name.ends_with(".app") {
        Some(Box::new(AppHost::new(&abs(name)?)))
    } else {
        None
    }
}

/// One diagnostic as Studio and AppHost list it: `E0302 3:7 message`, the
/// line 1-based and the column in chars; `error` when it has no code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Problem {
    pub(crate) code: Option<u16>,
    pub(crate) pos: Option<(usize, usize)>,
    pub(crate) message: String,
}

impl Problem {
    /// A diagnostic from compiling or running `src`.
    pub(crate) fn new(d: &Diag, src: &str) -> Problem {
        let pos = d.span.map(|s| lang::diag::line_col(src, s.start));
        Problem { code: d.code, pos, message: d.message.clone() }
    }

    /// A problem with no code and no position.
    pub(crate) fn plain(message: String) -> Problem {
        Problem { code: None, pos: None, message }
    }

    /// The code (`E0302` or `error`) and the position (`3:7`, or empty).
    fn head(&self) -> (String, String) {
        let (mut code, mut pos) = (String::new(), String::new());
        match self.code {
            Some(c) => {
                code.push('E');
                push_num(&mut code, c.into(), 4);
            }
            None => code.push_str("error"),
        }
        if let Some((l, c)) = self.pos {
            push_num(&mut pos, l, 1);
            pos.push(':');
            push_num(&mut pos, c, 1);
        }
        (code, pos)
    }

    pub(crate) fn line(&self) -> String {
        let (code, pos) = self.head();
        join(&[&code, " ", &pos, if pos.is_empty() { "" } else { " " }, &self.message])
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// `parts` joined into one string (no `format!` in shipped code).
fn join(parts: &[&str]) -> String {
    let mut out = String::new();
    parts.iter().for_each(|p| out.push_str(p));
    out
}

/// Appends `n` in decimal, zero-padded to at least `width` digits.
fn push_num(out: &mut String, n: usize, width: usize) {
    if n >= 10 || width > 1 {
        push_num(out, n / 10, width.saturating_sub(1));
    }
    out.push(char::from(b'0' + (n % 10) as u8));
}

#[cfg(test)]
mod tests;
