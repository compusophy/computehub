//! The `studio` program (see the library). Exit status: 0 when its window closes or its events
//! end, 1 when its devices fail, 2 for bad arguments. It says nothing on stderr: it lives in a
//! window, and the formatting that would say it costs about 900 bytes of the download.

#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    // As text, never panicking on an argument that is not UTF-8 (`env::args` would, and would
    // ship the formatting that says so).
    let args: Vec<String> =
        std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    let Some(mut view) = studio::view(&args) else { return ExitCode::from(2) };
    let served = uiwire::client::open()
        .and_then(|mut ui| studio::serve(&mut ui, view.as_mut(), &mut studio::Fs));
    if served.is_ok() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
