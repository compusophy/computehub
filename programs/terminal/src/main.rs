//! The `terminal` program (see the library): the Terminal's window. Run in a terminal, it says how
//! to open one instead. Exit status: 0 when its window closes or its events end (or after the
//! hint), 1 when its devices fail.

#![forbid(unsafe_code)]

use std::io::{self, IsTerminal};
use std::process::ExitCode;

fn main() -> ExitCode {
    // A terminal's tty on stdout, not a window's console: no window would ever hear from it.
    if io::stdout().is_terminal() {
        println!("terminal: an app with a window; open it with: open terminal");
        return ExitCode::SUCCESS;
    }
    let served = uiwire::client::open()
        .and_then(|mut ui| terminal::serve(&mut ui, &mut terminal::Terminal::default()));
    let Err(e) = served else { return ExitCode::SUCCESS };
    eprintln!("terminal: {e}");
    ExitCode::FAILURE
}
