//! The `system` program (see the library): About, Editor, Feedback, Files, Welcome, Activity or
//! Settings, as its name says. Run in a terminal (on any of its std fds: `about > out.txt` too),
//! it says how to open its window instead. Exit status: 0 when its window closes or its events end (or after the hint),
//! 1 when its devices fail, 2 for a name or arguments it does not know.

#![forbid(unsafe_code)]

use std::io::{self, IsTerminal};
use std::process::ExitCode;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    let Some((name, mut view)) = system::view(&argv) else {
        eprintln!(
            "usage: about | editor [file] | feedback | files [dir] | welcome | activity | settings"
        );
        return ExitCode::from(2);
    };
    // A terminal's tty on any std fd, not a window's console: no window would ever hear from it.
    let ttys = [io::stdin().is_terminal(), io::stdout().is_terminal(), io::stderr().is_terminal()];
    if let Some(fd) = system::hint_to(ttys) {
        match fd {
            1 => println!("{}", system::hint(name)),
            _ => eprintln!("{}", system::hint(name)),
        }
        return ExitCode::SUCCESS;
    }
    let served = uiwire::client::open()
        .and_then(|mut ui| system::serve(&mut ui, view.as_mut(), &mut system::Fs));
    let Err(e) = served else { return ExitCode::SUCCESS };
    eprintln!("system: {e}");
    ExitCode::FAILURE
}
