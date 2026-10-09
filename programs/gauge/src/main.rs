//! `gauge`, the pool's measure of a device (see the library): `gauge work` is a worker of the
//! pool's "Test this device": echo off, `ready`, then each line answered with one line.

#![forbid(unsafe_code)]

use std::io::{BufRead, Write};

fn main() {
    if std::env::args().nth(1).as_deref() != Some("work") {
        eprintln!("usage: gauge work");
        std::process::exit(2);
    }
    let ctl = std::fs::OpenOptions::new().write(true).open("/dev/consctl");
    _ = ctl.and_then(|mut c| c.write_all(b"echooff"));
    let mut out = std::io::stdout().lock();
    if writeln!(out, "ready").and_then(|()| out.flush()).is_err() {
        std::process::exit(1);
    }
    let mut held = gauge::Held::default();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { std::process::exit(1) };
        if let Some(a) = gauge::answer(&line, &mut held) {
            if writeln!(out, "{a}").and_then(|()| out.flush()).is_err() {
                std::process::exit(1);
            }
        }
    }
}
