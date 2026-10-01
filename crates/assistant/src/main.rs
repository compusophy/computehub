//! The `assistant` program (see the library). Exit status: 0 when its window
//! closes or its events end, 1 when its devices fail.

#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut assistant = assistant::Assistant::default();
    let served = uiwire::client::open()
        .and_then(|mut ui| assistant::serve(&mut ui, &mut assistant, &mut assistant::Fs));
    if let Err(e) = served {
        eprintln!("assistant: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
