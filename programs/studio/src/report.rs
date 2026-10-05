//! Back to compusophy: a make that ends without an app that runs clean, or the app's last fault
//! in the preview, offers "Send to compusophy" by the status. The person's tap is the consent:
//! nothing goes without it, and nothing twice. It goes as [`Request::Feedback`] (the desktop's
//! outbox, as the Feedback app's), never with the desktop's context: what was asked, how the
//! make ended (its code and problem, why it stopped, each request's kind, tokens, time and the
//! problem it left) and the program, clipped, so compusophy sees what applang or the coder
//! lacked.

use crate::{Studio, file_name};
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
/// its text, the problem it names (the fault of the program it installed goes with it), and
/// whether it went.
#[derive(Debug)]
pub(crate) struct Report {
    kind: &'static str,
    text: String,
    why: String,
    sent: bool,
}

/// The report of the make of `task` that ended as `done` (its program still in it); none when
/// it ran clean, the person stopped it with no problem showing, or the free AI was busy or out
/// of credit before any program came back (nothing applang or the coder lacked; the desktop
/// reports an AI that did not answer, or answered with a 5xx, itself, but nothing else).
pub(crate) fn made(task: &Task, done: &Done) -> Option<Report> {
    let kind = match done.outcome {
        Outcome::Ready => return None,
        Outcome::Stopped if done.code == 0 => return None,
        Outcome::Failed if matches!(done.code, 902 | 903) && done.draft.trim().is_empty() => {
            return None;
        }
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
    Some(Report { kind, text: t, why: done.why.clone(), sent: false })
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
    /// Keeps the fault the app shows in the preview (at first, why it started afresh) once a
    /// frame shows the app: it stays, under the app, after the app's next event takes it away,
    /// until another shows or the program changes.
    pub(crate) fn saw(&mut self) {
        let shows = !self.code && self.make.is_none();
        let Some(live) = self.live.as_ref().filter(|_| shows) else { return };
        let (fault, afresh) = live.fault();
        let now = fault.or(afresh.filter(|_| self.fault.is_empty()));
        if let Some(f) = now.filter(|f| *f != self.fault) {
            self.fault = f.into();
        }
    }

    /// What the status offers: `Some(false)` is "Send to compusophy", `Some(true)` "Sent". A
    /// failed make's report until it goes; then the app's kept fault (while the app shows, not
    /// its code), sent if it went; then that the report went.
    pub(crate) fn offer(&self) -> Option<bool> {
        let fault = Some(&self.fault).filter(|f| !f.is_empty() && !self.code);
        let fault = fault.map(|f| self.told.contains(f));
        match self.report.as_ref().map(|r| r.sent) {
            Some(false) => Some(false),
            made => fault.or(made),
        }
    }

    /// The tap on "Send to compusophy": what it offers goes, once; whether it did.
    pub(crate) fn send(&mut self) -> bool {
        if self.offer() != Some(false) {
            return false;
        }
        let (kind, text) = match &mut self.report {
            Some(r) if !r.sent => {
                r.sent = true;
                self.told.push(std::mem::take(&mut r.why));
                (r.kind, std::mem::take(&mut r.text))
            }
            _ => {
                let Some(live) = &self.live else { return false };
                let mut t = ["Studio app faulted: ", file_name(&self.path), "\n\nFault: "].concat();
                put_clip(&mut t, &self.fault, MAX_WHY);
                program(&mut t, "The program", live.src());
                self.told.push(self.fault.clone());
                ("bug", t)
            }
        };
        self.requests.push(Request::Feedback { kind: kind.into(), text, context: false });
        true
    }
}
