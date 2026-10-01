//! The `system` program (see the library): About, Feedback or Files, as its name says. Exit
//! status: 0 when its window closes or its events end, 1 when its devices fail, 2 for a name or
//! arguments it does not know.

#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    let Some(mut view) = system::view(&argv) else {
        eprintln!("usage: about | feedback | files [dir]");
        return ExitCode::from(2);
    };
    let served = uiwire::client::open()
        .and_then(|mut ui| system::serve(&mut ui, view.as_mut(), &mut system::Fs));
    let Err(e) = served else { return ExitCode::SUCCESS };
    eprintln!("system: {e}");
    ExitCode::FAILURE
}
