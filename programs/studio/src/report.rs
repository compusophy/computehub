//! Back to compusophy: a make that ends without an app that runs clean, or the app's fault
//! while it shows, offers "Send to compusophy" by the status. The person's tap is the consent:
//! nothing goes without it, and nothing twice. It goes as [`Request::Feedback`] (the desktop's
//! outbox, as the Feedback app's), never with the desktop's context: what was asked, how the
//! make ended (its code and problem, why it stopped, each request's kind, tokens, time and the
//! problem it left) and the program, clipped, so compusophy sees what applang or the coder
//! lacked.

use crate::{Live, Studio, file_name};
use coder::ai::{put_clip, put_code, put_num};
use coder::receipt::usd;
use coder::{Done, Outcome, Task, Turn};
use uiwire::Request;

/// The most bytes of the program a report carries, of the ask and of a problem: all of it well
/// under the 12 KiB of a report's text the desktop sends (`os::report::body` cuts the rest).
const MAX_SRC: usize = 8 * 1024;
const MAX_ASK: usize = 1000;
const MAX_WHY: usize = 1024;

/// A failed make's report: its kind (a "bug"; an "idea" when applang can make nothing close),
/// its text, and whether it went.
#[derive(Debug)]
pub(crate) struct Report {
    kind: &'static str,
    text: String,
    sent: bool,
}

/// The report of the make of `task` that ended as `done` (its program still in it); none when
/// it ran clean, the AI failed (the desktop reports that itself), or the person stopped it with
/// no problem showing.
pub(crate) fn made(task: &Task, done: &Done) -> Option<Report> {
    let kind = match done.outcome {
        Outcome::Ready | Outcome::Failed => return None,
        Outcome::Stopped if done.code == 0 => return None,
        Outcome::Cant => "idea",
        _ => "bug",
    };
    let mut t = String::from("Studio make failed: ");
    put_clip(&mut t, task.ask.lines().next().unwrap_or_default(), 100);
    t += "\n\nAsked: ";
    put_clip(&mut t, &task.ask, MAX_ASK);
    t += if done.change { "\nA change, by " } else { "\nA new app, by " };
    t += &task.model;
    t += "\nEnded: ";
    t += &done.said();
    if !done.why.is_empty() {
        t += "\nProblem: ";
        put_clip(&mut t, &done.why, MAX_WHY);
    }
    if done.ended != 0 && done.ended != done.code {
        t += "\nStopped by: ";
        put_code(&mut t, done.ended);
    }
    let r = &done.receipt;
    t += if r.turns.is_empty() { "\n\nRequests: none" } else { "\n\nRequests:" };
    for (i, turn) in r.turns.iter().enumerate() {
        let word = match turn.turn {
            Turn::Write => "write",
            Turn::Change => "change",
            Turn::Fix => "fix",
            Turn::Missed => "fix (edits missed)",
            Turn::Rewrite => "rewrite",
            Turn::Shorter => "shorter",
            Turn::Format => "format",
        };
        t.push('\n');
        put_num(&mut t, i as u64 + 1);
        t = t + ". " + word + ": ";
        put_num(&mut t, turn.input.into());
        t += " in (";
        put_num(&mut t, turn.cached.into());
        t += " cached), ";
        put_num(&mut t, turn.output.into());
        t += " out (";
        put_num(&mut t, turn.reasoning.into());
        t += " reasoning), ";
        put_num(&mut t, (turn.ms / 1000).into());
        t = t + " s, $" + &usd(turn.usd_micros);
        if turn.code != 0 {
            t += ", left ";
            put_code(&mut t, turn.code);
        }
    }
    t += "\nIn all: ";
    put_num(&mut t, r.ms / 1000);
    t = t + " s, $" + &usd(r.usd_micros) + if r.est { ", estimated" } else { "" };
    program(&mut t, "The program it ended with", &done.draft);
    Some(Report { kind, text: t, sent: false })
}

/// Appends `src`, after `what` and its lines, clipped to [`MAX_SRC`] bytes; or that there was
/// none.
fn program(t: &mut String, what: &str, src: &str) {
    if src.trim().is_empty() {
        *t += "\n\nNo program came back.";
        return;
    }
    *t += "\n\n";
    *t += what;
    *t += " (";
    put_num(t, src.lines().count() as u64);
    *t += " lines):\n```app\n";
    put_clip(t, src, MAX_SRC);
    if !t.ends_with('\n') {
        t.push('\n');
    }
    *t += "```";
}

impl Studio {
    /// What the status offers: `Some(false)` is "Send to compusophy", `Some(true)` "Sent". A
    /// failed make's report until the next make; else the app's fault while it shows (the app,
    /// not its code), sent if that one went.
    pub(crate) fn offer(&self) -> Option<bool> {
        if let Some(r) = &self.report {
            return Some(r.sent);
        }
        let fault = self.live.as_ref().and_then(Live::fault).filter(|_| !self.code)?;
        Some(fault == self.told)
    }

    /// The tap on "Send to compusophy": what it offers goes, once; whether it did.
    pub(crate) fn send(&mut self) -> bool {
        if self.offer() != Some(false) {
            return false;
        }
        let (kind, text) = match (&mut self.report, &self.live) {
            (Some(r), _) => {
                r.sent = true;
                (r.kind, std::mem::take(&mut r.text))
            }
            (None, Some(live)) => {
                let fault = live.fault().unwrap_or_default();
                let mut t = ["Studio app faulted: ", file_name(&self.path), "\n\nFault: "].concat();
                put_clip(&mut t, fault, MAX_WHY);
                program(&mut t, "The program", live.src());
                self.told = fault.into();
                ("bug", t)
            }
            (None, None) => return false,
        };
        self.requests.push(Request::Feedback { kind: kind.into(), text, context: false });
        true
    }
}
