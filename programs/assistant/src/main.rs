//! The `assistant` program, the desktop's overlay (see the library's `agent`). Exit status: 0
//! when its window closes or its events end, 1 when its devices fail.

#![forbid(unsafe_code)]

use std::process::ExitCode;

/// Where its chats are kept, under the person's home (`agent::compact`).
const FILE: &str = "/.assistant/chats";

fn main() -> ExitCode {
    let mut agent = assistant::agent::Agent::default();
    let file = std::env::var("HOME").ok().map(|home| home + FILE);
    if let Some(text) = file.as_ref().and_then(|f| std::fs::read(f).ok()) {
        agent.load(&String::from_utf8_lossy(&text));
    }
    let mut keep = |text: &str| file.iter().for_each(|f| save(f, text));
    let served = uiwire::client::open()
        .and_then(|mut ui| assistant::agent::serve(&mut ui, &mut agent, &mut keep));
    if let Err(e) = served {
        eprintln!("assistant: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// Puts `text` in the file at `path`, whole: written beside it, then moved over it, its folder
/// made if missing; a write that fails leaves what was there.
fn save(path: &str, text: &str) {
    let Some((dir, name)) = path.rsplit_once('/') else { return };
    _ = std::fs::create_dir(dir);
    let part = [dir, "/.", name, ".saving"].concat();
    if std::fs::write(&part, text).and_then(|()| std::fs::rename(&part, path)).is_err() {
        _ = std::fs::remove_file(&part);
    }
}
