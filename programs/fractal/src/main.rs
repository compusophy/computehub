//! The `fractal` program (see the library): with `work`, a worker answering tiles on its console
//! (the pool runs it so); else Fractal's window, or run in a terminal, how to open one. Exit
//! status: 0 when its window closes or its input ends, 1 when its devices fail.

#![forbid(unsafe_code)]

use std::io::{self, IsTerminal};
use std::process::ExitCode;

fn main() -> ExitCode {
    let served = if std::env::args().nth(1).as_deref() == Some("work") {
        fractal::work()
    } else if io::stdout().is_terminal() {
        println!("fractal: an app with a window; open it with: open fractal");
        return ExitCode::SUCCESS;
    } else {
        uiwire::client::open()
            .and_then(|mut ui| fractal::serve(&mut ui, &mut fractal::Fractal::default()))
    };
    let Err(e) = served else { return ExitCode::SUCCESS };
    eprintln!("fractal: {e}");
    ExitCode::FAILURE
}
