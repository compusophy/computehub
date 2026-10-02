//! Making with the AI: Studio runs a [`coder::Make`], which drives (writes, checks, fixes with
//! edits, keeps the best so far, stops by budget). Its requests go out as [`Request::Ai`], the
//! reply streams back in, its status and the program streaming in show as they change (a frame
//! only then), and its end installs the program it keeps: saved (a first make names the file),
//! run, and recorded in `~/.ai/makes.jsonl` (and, when it runs clean, the corpus). Unless the
//! code was edited meanwhile: then the edits stay.

use crate::{Disk, Studio};
use coder::ai::{CORPUS, corpus_line, state_path};
use coder::receipt::{self, MAKES};
use coder::{Done, Knobs, Make, Out, Outcome, Task};
use uiwire::{Request, Style};

/// A make in flight: its request's id and the make.
#[derive(Debug)]
pub(crate) struct Making {
    pub(crate) id: u32,
    pub(crate) m: Make,
}

pub(crate) use crate::clock as now;

impl Studio {
    /// Makes the app the prompt describes, or changes the one open as it says.
    pub(crate) fn make(&mut self, disk: &mut dyn Disk) {
        let ask = self.prompt.trim().to_string();
        if ask.is_empty() || self.make.is_some() {
            return;
        }
        if let Some(why) = self.locked() {
            self.status = (Style::Error, ["not made: ", why].concat());
            return;
        }
        let base = if self.fresh() { String::new() } else { self.text.clone() };
        // What it will really start from: the states the app at this path keeps.
        let kept = match self.path.is_empty() {
            true => String::new(),
            false => disk.read(&state_path(&self.path)).unwrap_or_default(),
        };
        let task = Task { ask, base, model: self.model().into(), kept };
        let (m, out) = Make::start(task, Knobs::default(), now());
        self.make = Some(Making { id: 0, m });
        // On a phone the keyboard goes, so the whole window shows the make.
        if self.narrow() {
            self.requests.push(Request::Focus { id: 0 });
        }
        self.out(out, disk);
    }

    /// Whether a make makes a new app: until a make names its file, or with no program. A draft
    /// a first make left (nothing in it compiled, or it was stopped) shows, but is not the
    /// program: Make again makes the app anew.
    fn fresh(&self) -> bool {
        self.path.is_empty() || self.text.trim().is_empty()
    }

    /// Does what the make wants next.
    fn out(&mut self, out: Out, disk: &mut dyn Disk) {
        let Some(mk) = &mut self.make else { return };
        match out {
            Out::Ask(body) => {
                self.last_id = self.last_id.wrapping_add(1).max(1);
                mk.id = self.last_id;
                self.requests.push(Request::Ai { id: mk.id, body });
            }
            Out::Cancel => self.requests.push(Request::AiCancel { id: mk.id }),
            Out::Done(done) => self.done(done, disk),
        }
    }

    pub(crate) fn stop(&mut self, disk: &mut dyn Disk) {
        let Some(mk) = &mut self.make else { return };
        if !mk.m.complete() {
            self.requests.push(Request::AiCancel { id: mk.id });
        }
        let done = mk.m.stop(now());
        self.done(done, disk);
    }

    /// More of the reply; whether what shows changed.
    pub(crate) fn data(&mut self, data: &[u8], disk: &mut dyn Disk) -> bool {
        let Some(mk) = &mut self.make else { return false };
        if let Some(out) = mk.m.data(data, now()) {
            self.out(out, disk);
        }
        self.shows()
    }

    /// The request ended.
    pub(crate) fn end(&mut self, status: u16, error: &str, disk: &mut dyn Disk) {
        let Some(mk) = &mut self.make else { return };
        if let Some(out) = mk.m.end(status, error, now()) {
            self.out(out, disk);
        }
    }

    /// Checks a reply that is in, once its status (testing) shows; whether there was one.
    pub(crate) fn verify(&mut self, disk: &mut dyn Disk) -> bool {
        let Some(out) = self.make.as_mut().and_then(|mk| mk.m.check(now())) else { return false };
        self.out(out, disk);
        true
    }

    /// Whether the make's status changed since the last frame showed it: a frame a second while
    /// it thinks, one a line (or an edit) while it writes, none else.
    pub(crate) fn shows(&mut self) -> bool {
        let Some(mk) = &self.make else { return true };
        let now = mk.m.status(now());
        now != std::mem::replace(&mut self.seen, now.clone())
    }

    /// The make's end: what it keeps installed (saved, run, in the corpus when clean), its draft
    /// in the code view when a new app has nothing that compiles, its line in makes.jsonl.
    fn done(&mut self, mut done: Done, disk: &mut dyn Disk) {
        let Some(mk) = self.make.take() else { return };
        let (task, ready) = (mk.m.task(), done.outcome == Outcome::Ready);
        let mut said = done.said();
        let bad = !ready && done.outcome != Outcome::Stopped;
        let mut style = if bad { Style::Error } else { Style::Small };
        let edited = self.text != task.base && !(task.base.is_empty() && self.fresh());
        let (mut version, src) = (0, std::mem::take(&mut done.draft));
        if edited {
            said = "your edits kept, not the AI's".into();
        } else if done.install {
            if !self.name(&src, disk) {
                (style, said) = (Style::Error, crate::edit::TAKEN.into());
            } else {
                self.replace(src, disk);
                self.mark = done.mark.filter(|_| !ready);
                if let (Style::Error, why) = self.save(disk, "") {
                    (style, said) = (Style::Error, why);
                }
                version = 1;
                if ready {
                    let n = done.receipt.turns.len() as u32;
                    let line = corpus_line(&task.ask, &self.text, &task.base, n, &task.model);
                    let _ = disk.append(CORPUS, &line);
                }
                if self.prompt.trim() == task.ask {
                    self.set_prompt("");
                    // A game that takes keys has the keyboard at once.
                    if !self.code && self.live.as_mut().is_some_and(|live| live.play().1) {
                        self.requests.push(Request::Focus { id: 0 });
                    }
                }
            }
        } else if task.base.is_empty() && !src.is_empty() {
            // Nothing that compiles: the last draft, marked, for the code view; never saved.
            self.replace(src, disk);
            self.mark = done.mark;
        }
        let lines = if version > 0 { self.text.lines().count() } else { 0 };
        let line = receipt::line(&self.path, version, lines, &done);
        self.caption = done.plan;
        let old = disk.read(MAKES).unwrap_or_default();
        let _ = match receipt::rotate(&old) {
            Some(kept) => disk.write(MAKES, &(kept + &line)),
            None => disk.append(MAKES, &line),
        };
        self.status = (style, said);
    }
}
