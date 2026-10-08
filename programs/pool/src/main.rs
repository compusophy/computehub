//! The `pool` program (see the library): run by the desktop on a console of its own, raw, it reads
//! [`uiwire::relay`] frames and answers them, its first frame READY. Run in a terminal it says
//! what it is. Exit status: 0 at the console's end, 1 when it cannot be raw.

#![forbid(unsafe_code)]

use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::process::ExitCode;

use uiwire::relay::{Frame, to_desk};

fn main() -> ExitCode {
    let raw =
        OpenOptions::new().write(true).open("/dev/consctl").and_then(|mut f| f.write_all(b"rawon"));
    if raw.is_err() || std::env::args().len() > 1 {
        eprintln!(
            "pool: the mesh's program, which the desktop runs; pair devices in Activity's Pool"
        );
        return ExitCode::FAILURE;
    }
    let mut hub = pool::Hub::default();
    Frame { op: to_desk::READY, ..Frame::default() }.put(&mut hub.out);
    let (mut stdin, mut stdout) = (io::stdin().lock(), io::stdout().lock());
    let (mut buf, mut chunk) = (Vec::new(), vec![0; 64 << 10]);
    loop {
        if stdout.write_all(&std::mem::take(&mut hub.out)).and_then(|()| stdout.flush()).is_err() {
            return ExitCode::FAILURE;
        }
        let n = match stdin.read(&mut chunk) {
            Ok(0) | Err(_) => return ExitCode::SUCCESS,
            Ok(n) => n,
        };
        buf.extend_from_slice(&chunk[..n]);
        while let Some((f, used)) = Frame::take(&buf) {
            buf.drain(..used);
            hub.frame(f);
        }
        hub.pump();
    }
}
