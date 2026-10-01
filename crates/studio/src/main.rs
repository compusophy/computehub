//! The `studio` program (see the library). Exit status: 0 when its window
//! closes or its events end, 1 when its devices fail, 2 for bad arguments.

#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(mut view) = studio::view(&args) else {
        eprintln!("usage: studio [edit <path> | run <path>]");
        return ExitCode::from(2);
    };
    let served = uiwire::client::open()
        .and_then(|mut ui| studio::serve(&mut ui, view.as_mut(), &mut studio::Fs));
    if let Err(e) = served {
        eprintln!("studio: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
