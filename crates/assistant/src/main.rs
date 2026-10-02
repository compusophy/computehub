//! The `assistant` program, the desktop's overlay (see the library's `agent`). Exit status: 0
//! when its window closes or its events end, 1 when its devices fail.

#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut agent = assistant::agent::Agent::default();
    let served =
        uiwire::client::open().and_then(|mut ui| assistant::agent::serve(&mut ui, &mut agent));
    if let Err(e) = served {
        eprintln!("assistant: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
