//! The `clock` program (see the library): a window showing the clock its name says (`clock`,
//! `grandfather` or `shop`), or run in a terminal, how to open one. Exit status: 0 when its
//! window closes or its input ends, 1 when it cannot draw.

#![forbid(unsafe_code)]

use std::io::{self, IsTerminal};
use std::process::ExitCode;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    if io::stdout().is_terminal() {
        println!("clock: an app with a window; open it with: open clock (or grandfather, shop)");
        return ExitCode::SUCCESS;
    }
    let served = uiwire::client::open()
        .and_then(|mut ui| clock::serve(&mut ui, &mut clock::Clock::new(&argv)));
    let Err(e) = served else { return ExitCode::SUCCESS };
    eprintln!("clock: {e}");
    ExitCode::FAILURE
}
