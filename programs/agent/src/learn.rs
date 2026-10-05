//! Lessons: how the agent hardens itself. A task that met failures on its way (a tool's error,
//! an app that did not check, a reply cut off) and still ended is a level beaten: after it, a
//! meta-prompt hands the model the task as it went and the lessons kept, and asks for the one
//! lesson that would have avoided the first failure, or NONE. A lesson that is new goes to
//! [`path`] as a line `- ...` and into every later system prompt ([`crate::Agent::system`]).
//! Past [`MAX_LINES`] lines or [`MAX_BYTES`] bytes, the model merges them into fewer, the most
//! useful first; a merge that comes back malformed keeps them as they were. The file is the
//! person's too: `cat` it, edit it, `/forget` empties it.

use coder::ai::{clip, put_clip, put_num};
use vfs::Vfs;

use crate::{Agent, DIM, MAX_NOTES, Msg, PLAIN, Task, World};

/// The most lines and bytes the lessons hold before they are merged, and a lesson's bytes.
pub const MAX_LINES: usize = 24;
pub const MAX_BYTES: usize = 3 << 10;
const MAX_LESSON: usize = 240;

/// What asks for a lesson.
const REFLECT: &str = "You improve a coding agent that works in compusophyOS's terminal (its \
tools: read_file, write_file, edit_file, list_dir, search, shell, check_app, applang_guide). \
Below is a task it finished after running into problems on the way. Write the ONE lesson that \
would have let it avoid the first problem next time: a single imperative line under 200 \
characters, specific to this OS, its tools, its shell or applang, and true beyond this task \
(name the rule, not the file). Do not repeat a lesson it already keeps. If the problems teach \
nothing general (an unclear request, a one-off), reply NONE. Reply with the line alone.";

/// What asks for the lessons merged.
const MERGE: &str = "You keep a coding agent's lessons, its standing instructions. Merge the \
lessons below into at most 12 lines: the most useful first, those that repeat or contradict \
another joined or dropped, each one imperative line starting with \"- \". Reply with the lines \
alone.";

/// Where the lessons are kept: `~/.agent/lessons.md`.
pub fn path() -> String {
    [Vfs::HOME, "/.agent/lessons.md"].concat()
}

/// The lessons kept ("" for none).
pub fn load(w: &mut impl World) -> String {
    let data = w.read(&path()).unwrap_or_default();
    clip(&String::from_utf8_lossy(&data), MAX_NOTES)
}

/// After task `t`, which ended past failures: asks for a lesson, keeps it if it is new (saying
/// so), and merges the lessons once they grow past their bounds.
pub fn reflect(a: &mut Agent, t: &Task, w: &mut impl World) {
    let story = story(a, t);
    let Some(reply) = a.quietly(REFLECT, &story, w) else { return };
    let Some(lesson) = lesson(&reply, &a.lessons) else { return };
    let mut kept = a.lessons.trim_end().to_string();
    if !kept.is_empty() {
        kept.push('\n');
    }
    kept += "- ";
    kept += &lesson;
    kept.push('\n');
    if save(w, &kept) {
        w.say(&[DIM, "\u{2726} learned: ", &lesson, PLAIN, "\n"].concat());
        a.lessons = kept;
    }
    if a.lessons.lines().count() > MAX_LINES || a.lessons.len() > MAX_BYTES {
        merge(a, w);
    }
}

/// The lesson in `reply`, if it is one and new: its first line, a list mark or quotes taken off,
/// at most 240 bytes, and in none of `kept`'s lines.
pub fn lesson(reply: &str, kept: &str) -> Option<String> {
    let line = reply.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.trim_start_matches(['-', '*', ' ']).trim_matches(['"', '`', ' ']);
    let none = line.len() < 12 || line.to_ascii_uppercase().starts_with("NONE");
    let low = line.to_ascii_lowercase();
    let known = kept.lines().any(|k| {
        let k = k.trim_start_matches(['-', ' ']).to_ascii_lowercase();
        !k.is_empty() && (k.contains(&low) || low.contains(&k))
    });
    (!none && !known).then(|| clip(line, MAX_LESSON))
}

/// Asks for the lessons merged, and keeps the merge if every line of it is a lesson and it is
/// shorter.
fn merge(a: &mut Agent, w: &mut impl World) {
    let lessons = a.lessons.clone();
    let Some(reply) = a.quietly(MERGE, &lessons, w) else { return };
    let lines: Vec<&str> = reply.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let ok =
        !lines.is_empty() && lines.len() <= MAX_LINES && lines.iter().all(|l| l.starts_with("- "));
    let merged: String = lines.iter().flat_map(|l| [*l, "\n"]).collect();
    if ok && merged.len() < lessons.len() && save(w, &merged) {
        let mut said = String::from("\u{2726} merged the lessons into ");
        put_num(&mut said, lines.len() as u64);
        w.say(&[DIM, &said, " lines", PLAIN, "\n"].concat());
        a.lessons = merged;
    }
}

/// Writes `lessons` to [`path`] (its folder made): whether it took.
fn save(w: &mut impl World, lessons: &str) -> bool {
    let path = path();
    let _ = w.mkdir(&path[..path.rfind('/').unwrap_or(0)], true);
    w.write(&path, lessons.as_bytes(), false).is_ok()
}

/// Task `t` as it went, for a lesson: the prompt, each call and the first line of its result,
/// the problems met, the answer; then the lessons kept.
fn story(a: &Agent, t: &Task) -> String {
    let mut out = String::new();
    for m in a.history.get(a.start..).unwrap_or_default() {
        match m {
            Msg::User(text) => {
                out += "User: ";
                put_clip(&mut out, text, 1000);
            }
            Msg::Reply(text, calls) => {
                if !text.is_empty() {
                    out += "Agent: ";
                    put_clip(&mut out, text, 600);
                    out.push('\n');
                }
                for c in calls {
                    out += "Call: ";
                    out += &c.name;
                    out.push(' ');
                    put_clip(&mut out, &c.args, 300);
                    out.push('\n');
                }
                continue;
            }
            Msg::Tool { text, .. } => {
                out += "Result: ";
                put_clip(&mut out, text.lines().next().unwrap_or_default(), 300);
            }
        }
        out.push('\n');
    }
    out += "\nProblems met, in order:\n";
    for f in &t.failures {
        out += "- ";
        put_clip(&mut out, f, 400);
        out.push('\n');
    }
    out += "\nLessons it already keeps:\n";
    out += if a.lessons.trim().is_empty() { "(none)" } else { a.lessons.trim_end() };
    out
}
