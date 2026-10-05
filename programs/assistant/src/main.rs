//! The `assistant` program, the desktop's overlay (see the library's `agent`). Exit status: 0
//! when its window closes or its events end, 1 when its devices fail.

#![forbid(unsafe_code)]

use std::io::ErrorKind;
use std::process::ExitCode;

/// What the card says when the kept chats are not there.
const SET_ASIDE: &str =
    "Your chats could not be read: they are set aside as ~/.assistant/chats.bad";
const UNREAD: &str = "Your chats could not be read; they are left as they are, and this session's \
                      are not kept";
const NEWER: &str = "Your chats were kept by a newer compusophyOS: reload the page to see them; \
                     this session's are not kept";

fn main() -> ExitCode {
    let mut agent = assistant::agent::Agent::default();
    // Its chats, in its own folder of the person's home, which its file tools never reach
    // (`agent::compact`). A file a newer OS kept stays as it is; one that does not read is set
    // aside, never written over; and one that cannot be read or set aside stays, nothing kept
    // over it. The card says which.
    let mut file = std::env::var("HOME").ok().map(|home| [&home, files::OWN, "/chats"].concat());
    let told = match file.as_deref().map(std::fs::read) {
        Some(Ok(text)) => match String::from_utf8_lossy(&text) {
            text if agent.load(&text) => None,
            text if chats::newer(&text) => Some(NEWER),
            _ => match file.as_deref().map(|f| std::fs::rename(f, [f, ".bad"].concat())) {
                Some(Ok(())) => Some(SET_ASIDE),
                _ => Some(UNREAD),
            },
        },
        Some(Err(e)) if e.kind() != ErrorKind::NotFound => Some(UNREAD),
        _ => None,
    };
    if let Some(line) = told {
        agent.tell(line);
        file = file.filter(|_| line == SET_ASIDE);
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
