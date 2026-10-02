//! The make loop: turns, checks, the best so far, budgets and the status a person reads.

use crate::ai::{self, Fault, MAX_BODY, MAX_REPLY, ROOM, about, failure, fenced, num};
use crate::edits::{self, Reply, marked};
use crate::json::{Stream, Usage};
use crate::receipt::price;
use crate::{Done, Knobs, Out, Outcome, Receipt, Task, Turn, TurnLog, prompt};
use applang::{Class, Span};

/// Where a make is: a request streaming, its reply in (to check), or over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Streaming,
    Complete,
    Over,
}

/// Why a reply was cut short by the make: thinking ran past its budget, or time ran out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Early {
    No,
    Runaway,
    Late,
}

/// Why a make ended, besides a clean program or the AI's own codes (E0901 to E0905).
const GAVE_UP: u16 = 1;
const CANT: u16 = 2;
const STOPPED: u16 = 3;
const NO_PROGRAM: u16 = 906;
const NO_ROOM: u16 = 907;
const SPENT: u16 = 908;
const RUNAWAY: u16 = 909;
const TOO_BIG: u16 = 910;

/// Why a fix's edits that dropped over a quarter of the program's lines were not applied.
const SHRANK: &str = "Your edits dropped over a quarter of the program's lines. Keep the app \
                      whole: fix the line, don't drop features.";

/// The tries of each kind so far, by these.
const SHORTERS: usize = 0;
const FORMATS: usize = 1;
const MISSES: usize = 2;
const REWRITES: usize = 3;
const RUNAWAYS: usize = 4;

/// A program the make checked: its text and its problem (none: it runs clean).
#[derive(Debug)]
struct Cand {
    src: String,
    fault: Option<Fault>,
}

/// One make: see the crate docs.
#[derive(Debug)]
pub struct Make {
    task: Task,
    k: Knobs,
    system: String,
    /// When it started, and when the request in flight was sent.
    t0: u64,
    sent: u64,
    /// The turn in flight, what it is doing (`fixing line 43`), its message, the first turn's
    /// message and the body's bytes.
    turn: Turn,
    word: String,
    asked: String,
    first: String,
    body: usize,
    /// The `max_tokens` of the request in flight.
    room: u32,
    stream: Stream,
    reply: String,
    phase: Phase,
    early: Early,
    /// Whether the request in flight is in the log yet.
    logged: bool,
    /// The programs checked, oldest first (for a change, the program it changes first): the
    /// last is the one the next turn works on. Which is the best so far, and the key of the last
    /// problem (its code and its line's text, so a moved line still repeats).
    cands: Vec<Cand>,
    best: Option<usize>,
    key: String,
    tried: [u8; 5],
    log: Vec<TurnLog>,
    est: bool,
}

impl Make {
    /// A make of `task` within `k`, at `now` ms: the first request, or its end (too big to ask).
    pub fn start(task: Task, k: Knobs, now: u64) -> (Make, Out) {
        let change = !task.base.is_empty();
        let cands = match change {
            true => vec![Cand {
                src: task.base.clone(),
                fault: ai::fault(&task.base, &task.kept, k.seeds),
            }],
            false => Vec::new(),
        };
        let (turn, first) = match change {
            true => (Turn::Change, prompt::change(&task.ask, &task.base)),
            false => (Turn::Write, prompt::write(&task.ask)),
        };
        let mut m = Make {
            task,
            k,
            system: prompt::system(),
            t0: now,
            sent: now,
            turn,
            word: String::new(),
            asked: String::new(),
            first: first.clone(),
            body: 0,
            room: 0,
            stream: Stream::default(),
            reply: String::new(),
            phase: Phase::Over,
            early: Early::No,
            logged: true,
            best: change.then_some(0),
            cands,
            key: String::new(),
            tried: [0; 5],
            log: Vec::new(),
            est: false,
        };
        let out = m.ask(now, turn, first);
        (m, out)
    }

    /// What it makes.
    pub fn task(&self) -> &Task {
        &self.task
    }

    /// Whether its reply is in: [`Make::check`] comes next.
    pub fn complete(&self) -> bool {
        self.phase == Phase::Complete
    }

    /// More of the response. Once its program's block closes the rest is never used (a model
    /// may go on drafting), so the request stops; so it does when the model thinks past its
    /// budget, or the make's time is up. [`Out::Cancel`] then; [`Make::check`] comes next.
    pub fn data(&mut self, bytes: &[u8], now: u64) -> Option<Out> {
        if self.phase != Phase::Streaming {
            return None;
        }
        self.stream.feed(bytes, &mut self.reply, MAX_REPLY);
        let thought = self.stream.thought as u64 * 10 / 34;
        self.early = match () {
            _ if fenced(&self.reply).is_some_and(|(_, closed)| closed) => Early::No,
            // The guard is for a request of `write_tokens`; a smaller one's is as much smaller.
            _ if self.reply.trim().is_empty()
                && thought * u64::from(self.k.write_tokens)
                    >= u64::from(self.k.runaway) * u64::from(self.room) =>
            {
                Early::Runaway
            }
            _ if now.saturating_sub(self.t0) >= self.k.ms => Early::Late,
            _ => return None,
        };
        self.phase = Phase::Complete;
        self.log_turn(now);
        Some(Out::Cancel)
    }

    /// The response ended with HTTP `status` and the host's `error`: the make's end if the AI
    /// failed; else `None`, and [`Make::check`] comes next.
    pub fn end(&mut self, status: u16, error: &str, now: u64) -> Option<Out> {
        if self.phase != Phase::Streaming {
            return None;
        }
        self.stream.end(&mut self.reply, MAX_REPLY);
        self.phase = Phase::Complete;
        self.log_turn(now);
        let (code, why) = failure(status, error, &self.stream.error)?;
        let code = if code == 0 { STOPPED } else { code };
        Some(Out::Done(self.finish(now, code, why)))
    }

    /// The person stopped it: the best so far, under the usual rules.
    pub fn stop(&mut self, now: u64) -> Done {
        self.log_turn(now);
        self.finish(now, STOPPED, "stopped".into())
    }

    /// Reads the reply that is in and checks what it holds: the make's end, or the next request.
    /// `None` if no reply is in.
    pub fn check(&mut self, now: u64) -> Option<Out> {
        if self.phase != Phase::Complete {
            return None;
        }
        self.phase = Phase::Over;
        let cut = self.stream.finish == "length";
        let out = match self.early {
            Early::Runaway => self.runaway(now),
            Early::Late => self.over(now, SPENT, "out of time".into()),
            Early::No => match edits::read(&self.reply, cut) {
                Reply::Program(src) if only_comments(&src) => self.cant(now, &src),
                Reply::Program(src) => self.candidate(src, now),
                Reply::Edits(blocks) => match self.cands.last() {
                    None => self.format(now),
                    Some(cur) => match edits::apply(&cur.src, &blocks) {
                        Ok(src) if self.shrank(&cur.src, &src) => self.missed(SHRANK.into(), now),
                        Ok(src) => self.candidate(src, now),
                        Err(m) => self.missed(m, now),
                    },
                },
                Reply::Unclosed(n) => {
                    let why = ["SEARCH block ", &num(n as u64), " has no >>>>>>> REPLACE line."];
                    self.missed(why.concat(), now)
                }
                Reply::Cut if self.thinking() => self.runaway(now),
                Reply::Cut if self.tried[SHORTERS] == 0 => {
                    self.tried[SHORTERS] += 1;
                    self.ask(now, Turn::Shorter, prompt::shorter(&self.first))
                }
                Reply::Cut => self.over(now, NO_ROOM, ROOM.into()),
                Reply::Nothing if only_comments(&self.reply) => self.cant(now, &self.reply.clone()),
                Reply::Nothing => self.format(now),
            },
        };
        Some(out)
    }

    /// A program the reply gave or its edits made: checked, kept if it is the best so far; the
    /// end if it runs clean, else a fix (or the rewrite, for the same problem again).
    fn candidate(&mut self, src: String, now: u64) -> Out {
        let fault = ai::fault(&src, &self.task.kept, self.k.seeds);
        if let Some(log) = self.log.last_mut() {
            log.code = fault.as_ref().map_or(0, |f| f.diag.code.unwrap_or(0));
        }
        self.cands.push(Cand { src, fault });
        let n = self.cands.len() - 1;
        // How good one is: compiles, then runs clean, then newer.
        let rank = |c: &Cand| (c.fault.as_ref().is_none_or(|f| f.compiles), c.fault.is_none());
        if self.best.is_none_or(|b| rank(&self.cands[n]) >= rank(&self.cands[b])) {
            self.best = Some(n);
        }
        let cand = &self.cands[n];
        let Some(f) = &cand.fault else { return self.over(now, 0, String::new()) };
        let at = f.diag.span.map_or(0, |s| s.start);
        let from = cand.src.get(..at).and_then(|s| s.rfind('\n')).map_or(0, |i| i + 1);
        let line = cand.src[from..].lines().next().unwrap_or("").trim();
        let mut key = num(f.diag.code.unwrap_or(0).into());
        key.push(' ');
        key += line;
        let (account, same) = (f.account.clone(), key == self.key);
        let msg = prompt::fix(&self.task.ask, &cand.src, &account);
        self.key = key;
        match same {
            false => self.ask(now, Turn::Fix, msg),
            true => self.rewrite(now, &account),
        }
    }

    /// Edits that did not apply: once, the same request with why; then the rewrite.
    fn missed(&mut self, why: String, now: u64) -> Out {
        self.tried[MISSES] += 1;
        match self.tried[MISSES] {
            1 => self.ask(now, Turn::Missed, prompt::missed(&self.asked, &why)),
            _ => self.rewrite(now, &why),
        }
    }

    /// The one rewrite, of the program the turns work on; past it, the end.
    fn rewrite(&mut self, now: u64, why: &str) -> Out {
        let Some(cur) = self.cands.last().filter(|_| self.tried[REWRITES] < self.k.rewrites) else {
            return self.over(now, GAVE_UP, why.into());
        };
        let msg = prompt::rewrite(&self.task.ask, &cur.src, why);
        self.tried[REWRITES] += 1;
        self.ask(now, Turn::Rewrite, msg)
    }

    /// The model thought past its budget: the same request again, once.
    fn runaway(&mut self, now: u64) -> Out {
        self.tried[RUNAWAYS] += 1;
        if self.tried[RUNAWAYS] >= 2 {
            return self.over(now, RUNAWAY, "the AI kept thinking past its budget".into());
        }
        let (turn, asked) = (self.turn, self.asked.clone());
        self.ask(now, turn, asked)
    }

    /// A reply with neither a program nor edits: once, the same asking for them.
    fn format(&mut self, now: u64) -> Out {
        self.tried[FORMATS] += 1;
        if self.tried[FORMATS] >= 2 {
            return self.over(now, NO_PROGRAM, "the AI replied without a program".into());
        }
        let msg = prompt::format(&self.asked);
        self.ask(now, Turn::Format, msg)
    }

    fn cant(&mut self, now: u64, comment: &str) -> Out {
        let why = about(comment);
        self.over(now, CANT, why)
    }

    /// Whether a fix's edits dropped over a quarter of the program's lines.
    fn shrank(&self, before: &str, after: &str) -> bool {
        let lines = |s: &str| s.lines().filter(|l| !l.trim().is_empty()).count();
        matches!(self.turn, Turn::Fix | Turn::Missed) && lines(after) * 4 < lines(before) * 3
    }

    /// Whether a reply cut off by the token limit spent its room thinking.
    fn thinking(&self) -> bool {
        let reasoning = self.stream.usage.map_or(0, |u| u.reasoning);
        self.stream.thought > self.reply.len() || reasoning > self.room / 2
    }

    /// Sends `msg` as the next request of kind `turn`, unless a budget is spent or it is too big.
    fn ask(&mut self, now: u64, turn: Turn, msg: String) -> Out {
        let k = self.k;
        let out: u32 = self.log.iter().map(|t| t.output).sum();
        let usd: u32 = self.log.iter().map(|t| t.usd_micros).sum();
        let late = now.saturating_sub(self.t0) >= k.ms;
        if self.log.len() >= usize::from(k.requests)
            || out >= k.out_tokens
            || usd >= k.usd_micros
            || late
        {
            let why = "out of budget for one make (5 requests, $0.08 or 150 s)";
            return self.over(now, SPENT, why.into());
        }
        let fix = matches!(turn, Turn::Fix | Turn::Missed);
        let (tokens, reasoning, temp) = match fix {
            true => (k.fix_tokens, k.fix_reasoning, "0.2"),
            false => (k.write_tokens, k.write_reasoning, "0.3"),
        };
        let mut options = String::from(",\"max_tokens\":");
        ai::put_num(&mut options, tokens.into());
        options += ",\"temperature\":";
        options += temp;
        if reasoning > 0 {
            options += ",\"reasoning\":{\"max_tokens\":";
            ai::put_num(&mut options, reasoning.into());
            options.push('}');
        }
        let body = ai::chat(&self.task.model, &options, &self.system, &msg);
        if body.len() > MAX_BODY {
            return self.over(now, TOO_BIG, "too big for the AI".into());
        }
        self.word = match turn {
            Turn::Fix | Turn::Missed => {
                let src = self.cands.last().map_or("", |c| c.src.as_str());
                let line = self.mark().map_or(0, |s| ai::line_col(src, s.start).0);
                let mut word = String::from("fixing line ");
                ai::put_num(&mut word, line as u64);
                word
            }
            Turn::Rewrite => "rewriting".into(),
            Turn::Change => "changing".into(),
            _ => "writing".into(),
        };
        (self.turn, self.asked, self.body, self.sent, self.room) =
            (turn, msg, body.len(), now, tokens);
        (self.stream, self.reply) = (Stream::default(), String::new());
        (self.phase, self.early, self.logged) = (Phase::Streaming, Early::No, false);
        Out::Ask(body)
    }

    /// The request in flight in the log, once: what its usage said or, when none came (it was
    /// cancelled), an estimate from what streamed: the body's chars / 3 in (as the free AI
    /// reckons), reasoning chars / 3.4 and content chars / 2.4 out.
    fn log_turn(&mut self, now: u64) {
        if std::mem::replace(&mut self.logged, true) {
            return;
        }
        let u = self.stream.usage.unwrap_or_else(|| {
            self.est = true;
            let reasoning = (self.stream.thought * 10 / 34) as u32;
            let content = (self.reply.chars().count() * 10 / 24) as u32;
            Usage {
                input: (self.body / 3) as u32,
                output: reasoning + content,
                reasoning,
                ..Usage::default()
            }
        });
        self.log.push(TurnLog {
            turn: self.turn,
            input: u.input,
            cached: u.cached,
            output: u.output,
            reasoning: u.reasoning,
            usd_micros: price(&self.task.model, &u),
            ms: now.saturating_sub(self.sent) as u32,
            code: 0,
        });
    }

    fn over(&mut self, now: u64, code: u16, why: String) -> Out {
        Out::Done(self.finish(now, code, why))
    }

    /// The end, for `stop` (0: a clean program; or a code saying why): the best so far installed
    /// if it runs clean, or a new app's that compiles; else nothing, the version open running on.
    fn finish(&mut self, now: u64, stop: u16, why: String) -> Done {
        self.phase = Phase::Over;
        let change = !self.task.base.is_empty();
        // Installed: one that runs clean (for a change, not the program it changes), or a new
        // app's that compiles.
        let best = self.best.map(|b| (b, &self.cands[b])).filter(|(b, c)| match &c.fault {
            None => *b > 0 || !change,
            Some(f) => f.compiles && !change,
        });
        let best = best.map(|(_, c)| c);
        let install = best.is_some();
        let outcome = match stop {
            STOPPED => Outcome::Stopped,
            901..=905 => Outcome::Failed,
            CANT => Outcome::Cant,
            _ => match best {
                Some(b) if b.fault.is_none() => Outcome::Ready,
                Some(_) => Outcome::Faulting,
                None => Outcome::Broken,
            },
        };
        // What the person is shown: the program installed, else the last one checked (never the
        // one a change changes), else what the reply held of one; and its problem, if any, else
        // why the make ended.
        let shown = best.or(self.cands.last().filter(|_| self.cands.len() > usize::from(change)));
        let partial = || fenced(&self.reply).map_or(String::new(), |(s, _)| s.to_string());
        let draft = shown.map_or_else(partial, |c| c.src.clone());
        let fault = shown.and_then(|c| c.fault.as_ref()).filter(|_| stop != CANT);
        let (mut code, mut line, mut said, mut mark) = (stop, 0, why, None);
        if let (Some(f), false) = (fault, outcome == Outcome::Failed) {
            code = f.diag.code.unwrap_or(0);
            mark = f.diag.span;
            line = mark.map_or(0, |s| ai::line_col(&draft, s.start).0 as u32);
            said = f.said.clone();
        }
        let plan = if stop == CANT { said.clone() } else { about(&draft) };
        let usd = self.log.iter().map(|t| t.usd_micros).sum();
        let ms = now.saturating_sub(self.t0);
        let turns = std::mem::take(&mut self.log);
        let receipt = Receipt { turns, ms, usd_micros: usd, est: self.est };
        Done { outcome, install, draft, mark, code, line, why: said, plan, change, receipt }
    }

    /// What a person reads while it works: `asking`, `thinking · 6 s`, `writing · 48 lines`,
    /// `changing · 2 edits`, `testing`, `fixing line 43 · 1 edit`, `rewriting · 6 s`.
    pub fn status(&self, now: u64) -> String {
        let r = &self.reply;
        let (n, unit) = match () {
            _ if self.phase != Phase::Streaming => return "testing".into(),
            _ if marked(r) => {
                (r.lines().filter(|l| l.trim_start().starts_with(">>>>>>>")).count(), " edit")
            }
            _ if !r.is_empty() => (r.matches('\n').count(), " line"),
            _ => ((now.saturating_sub(self.sent) / 1000) as usize, " s"),
        };
        // A program's first turns ask, then think, before they write; a fix fixes all along.
        let first = unit == " s" && matches!(self.turn, Turn::Write | Turn::Change | Turn::Shorter);
        let word = match first {
            true if self.stream.thought == 0 => return "asking".into(),
            true => "thinking",
            false => &self.word,
        };
        let mut out = String::from(word);
        out += " \u{b7} ";
        ai::put_num(&mut out, n as u64);
        out += unit;
        if n != 1 && unit != " s" {
            out.push('s');
        }
        out
    }

    /// The program showing: the one streaming in (and `true`), or the one the turns work on.
    pub fn draft(&self) -> (&str, bool) {
        match fenced(&self.reply) {
            Some((src, _)) if self.phase == Phase::Streaming && !marked(src) => (src, true),
            _ => (self.cands.last().map_or("", |c| c.src.as_str()), false),
        }
    }

    /// What the program showing says it is (its first comment).
    pub fn plan(&self) -> String {
        about(self.draft().0)
    }

    /// The problem's bytes in the program the turns work on.
    pub fn mark(&self) -> Option<Span> {
        self.cands.last()?.fault.as_ref()?.diag.span
    }
}

/// Whether `src` holds comments and nothing else.
fn only_comments(src: &str) -> bool {
    let toks = applang::highlight(src);
    !toks.is_empty() && toks.iter().all(|(_, c)| *c == Class::Comment)
}
