//! The `assistant` program, the desktop's overlay (see the library's `agent`), its chats kept in
//! the person's home (`agent::chats`). Exit status: 0 when its window closes or its events end, 1
//! when its devices fail.

#![forbid(unsafe_code)]

use std::process::ExitCode;

use assistant::agent::{Agent, chats, serve};

fn main() -> ExitCode {
    let mut agent = Agent::default();
    let file = std::env::var("HOME").ok().map(|home| [home.as_str(), chats::FILE].concat());
    if let Some(text) = file.as_ref().and_then(|f| std::fs::read(f).ok()) {
        agent.load(&String::from_utf8_lossy(&text));
    }
    let mut keep = |text: &str| file.iter().for_each(|f| chats::keep(f, text));
    let served = uiwire::client::open().and_then(|mut ui| serve(&mut ui, &mut agent, &mut keep));
    if let Err(e) = served {
        eprintln!("assistant: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
