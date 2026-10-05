//! The commands' work, apart from their files and flags (`main.rs`) and the wire
//! ([`crate::wire`]): tasks written and verified, tasks solved and graded round by round,
//! verified solutions exported for supervised fine-tuning, and the models being taught asked.

use coder::edits::{self, Reply};
use coder::json::{Json, put};

use crate::claude::{Msg, Replies, Req, Teacher};
use crate::cost::{self, Ledger};
use crate::seam::{By, Judge, Task, is_held};
use crate::wire::Chat;
use crate::{Fail, codes, fnv, hex16, prompts};

/// A run of the teacher: who answers, who verifies, the ledger (and budget), whether it asks in
/// batches, each request's effort and room, and the day its lines say.
pub struct Run<'a> {
    pub teacher: &'a mut dyn Teacher,
    pub judge: &'a dyn Judge,
    pub ledger: Ledger,
    pub batch: bool,
    pub effort: String,
    pub max_tokens: u32,
    pub day: String,
}

/// What `tasks` wrote: the lines kept, and those refused with why (coded).
#[derive(Debug, Default)]
pub struct Written {
    pub kept: Vec<String>,
    pub refused: Vec<(String, String)>,
}

/// A sample's chain of attempts: its task (an index of the tasks solved), its number, its last
/// program and whether that passed.
struct Chain {
    task: usize,
    sample: u32,
    program: String,
    pass: bool,
}

impl Run<'_> {
    fn req(&self, system: &str, user: String) -> Req {
        Req {
            system: system.into(),
            user,
            max_tokens: self.max_tokens,
            effort: self.effort.clone(),
        }
    }

    /// Each request's reply, in the order asked, each in the ledger as it comes. In a batch, as
    /// many as the budget holds at their worst ([`cost::worst`]), the rest refused unsent
    /// (E0987); live, one by one, each while its worst still fits. Past the budget after all
    /// (a batch's cost is known only when it ends), it says so.
    pub fn ask(&mut self, reqs: Vec<(String, Req)>) -> Replies {
        let model = self.teacher.model().to_string();
        let over = || Err(Fail::new(codes::BUDGET, "the budget has no room for this request"));
        let mut out: Replies = Vec::new();
        if self.batch {
            let mut worst = 0;
            let fit = reqs.iter().take_while(|(_, r)| {
                worst += cost::worst(r, &model, true);
                self.ledger.fits(worst)
            });
            let (send, rest) = reqs.split_at(fit.count());
            if !send.is_empty() {
                let at_most =
                    cost::usd(send.iter().map(|(_, r)| cost::worst(r, &model, true)).sum());
                eprintln!("  a batch of {} requests: ${at_most} at most", send.len());
                let got = self.teacher.batch(send);
                for (id, _) in send {
                    let none = || Err(Fail::new(codes::BATCH, "the batch's results lack it"));
                    let mine = match &got {
                        Ok(rs) => rs.iter().find(|r| r.0 == *id).map_or_else(none, |r| r.1.clone()),
                        Err(f) => Err(f.clone()),
                    };
                    if let Ok(m) = &mine {
                        self.ledger.note(id, m, true).unwrap_or_else(|e| eprintln!("  {e}"));
                    }
                    out.push((id.clone(), mine));
                }
            }
            out.extend(rest.iter().map(|(id, _)| (id.clone(), over())));
        } else {
            for (id, r) in &reqs {
                let got = match self.ledger.fits(cost::worst(r, &model, false)) {
                    true => self.teacher.call(r),
                    false => over(),
                };
                if let Ok(m) = &got {
                    self.ledger.note(id, m, false).unwrap_or_else(|e| eprintln!("  {e}"));
                }
                out.push((id.clone(), got));
            }
        }
        if self.ledger.cap.is_some_and(|c| self.ledger.spent > c) {
            let spent = cost::usd(self.ledger.spent);
            eprintln!(
                "  {}",
                Fail::new(codes::BUDGET, format!("the run spent ${spent}, past its budget"))
            );
        }
        out
    }

    /// Asks for `per` tasks of tier `tier` in each of `families` (a request each, the writer's
    /// prompt over the checker language's `card`); keeps those asked for (that tier and family,
    /// an id neither `taken` nor repeated) that the verifier lets stand, each with its `by`.
    pub fn tasks(
        &mut self,
        card: &str,
        tier: u8,
        families: &[String],
        per: u32,
        taken: &[String],
    ) -> Written {
        let system = prompts::writer(card);
        let (prompt, verifier) = (hex16(fnv(system.as_bytes())), self.judge.id());
        let mut taken = taken.to_vec();
        let reqs = families.iter().enumerate().map(|(i, f)| {
            let mine: Vec<String> =
                taken.iter().filter(|t| t.starts_with(&[f, "-"].concat())).cloned().collect();
            (format!("f{i}"), self.req(&system, prompts::writer_user(tier, f, per, &mine)))
        });
        let reqs = reqs.collect();
        let mut w = Written::default();
        for ((_, got), f) in self.ask(reqs).into_iter().zip(families) {
            let m = match got
                .and_then(|m| m.short().filter(|s| s.code == codes::REFUSED).map_or(Ok(m), Err))
            {
                Ok(m) => m,
                Err(e) => {
                    w.refused.push((e.to_string(), format!("tier {tier}, family {f}")));
                    continue;
                }
            };
            for t in prompts::tasks_in(&m.text) {
                let mut t = match t {
                    Ok(t) => t,
                    Err((why, line)) => {
                        w.refused.push((Fail::new(codes::TASK_REFUSED, why).to_string(), line));
                        continue;
                    }
                };
                t.by = By {
                    teacher: m.model.clone(),
                    prompt: prompt.clone(),
                    verifier: verifier.clone(),
                    day: self.day.clone(),
                };
                let (line, asked) = (t.line(), t.tier == tier && t.family == *f);
                let stands = match () {
                    _ if !asked => Err(format!(
                        "asked for tier {tier} {f}, it is tier {} {}",
                        t.tier, t.family
                    )),
                    _ if taken.contains(&t.id) => Err(format!("the id {} is taken", t.id)),
                    _ => self.judge.verify_task(&line),
                };
                match stands {
                    Ok(said) => {
                        eprintln!("  kept {}: {said}", t.id);
                        taken.push(t.id);
                        w.kept.push(line);
                    }
                    Err(why) => {
                        w.refused.push((Fail::new(codes::TASK_REFUSED, why).to_string(), line))
                    }
                }
            }
            if let Some(cut) = m.short() {
                w.refused.push((
                    cut.to_string(),
                    format!("tier {tier}, family {f}: its last task may be lost"),
                ));
            }
        }
        w
    }

    /// Solves each of `tasks` but the `held` families' `k` times, in `rounds`: the first asks
    /// for a new app (the coder's first turn); each later one sends the coder's fix turn to each
    /// sample whose program faults as the coder sees faults ([`prompts::fix`]; a program that
    /// runs clean but fails its check has no turn the coder would send, so its chain ends).
    /// Every attempt's line goes to `keep` round by round; the passes are counted.
    pub fn solve(
        &mut self,
        tasks: &[Task],
        held: &[String],
        k: u32,
        rounds: u32,
        keep: &mut dyn FnMut(&str) -> Result<(), Fail>,
    ) -> Result<usize, Fail> {
        let system = prompts::solver();
        let (prompt, verifier) = (hex16(fnv(system.as_bytes())), self.judge.id());
        let train: Vec<&Task> = tasks.iter().filter(|t| in_split("train", t, held)).collect();
        let mut chains: Vec<Chain> = (0..train.len())
            .flat_map(|t| {
                (0..k).map(move |s| Chain {
                    task: t,
                    sample: s,
                    program: String::new(),
                    pass: false,
                })
            })
            .collect();
        let mut passes = 0;
        for round in 1..=rounds {
            let (mut reqs, mut whose) = (Vec::new(), Vec::new());
            for (c, ch) in chains.iter().enumerate() {
                let t = train[ch.task];
                let user = match round {
                    1 => Some(prompts::solve(&t.ask)),
                    _ if ch.pass || ch.program.is_empty() => None,
                    _ => prompts::fix(&t.ask, &ch.program),
                };
                if let Some(user) = user {
                    reqs.push((
                        format!("r{round}-t{}-s{}", ch.task, ch.sample),
                        self.req(&system, user),
                    ));
                    whose.push(c);
                }
            }
            if reqs.is_empty() {
                break;
            }
            let users: Vec<String> = reqs.iter().map(|r| r.1.user.clone()).collect();
            let mut lines = String::new();
            for (((_, got), c), user) in self.ask(reqs).into_iter().zip(whose).zip(users) {
                let ch = &mut chains[c];
                let t = train[ch.task];
                let mut a = Attempt { round, user, ..Attempt::default() };
                a.by = By {
                    teacher: self.teacher.model().into(),
                    prompt: prompt.clone(),
                    verifier: verifier.clone(),
                    day: self.day.clone(),
                };
                match got {
                    Ok(m) => a.read(m, &ch.program),
                    Err(f) => {
                        (a.stage, a.code, a.why) = (
                            if f.code == codes::BUDGET { "budget" } else { "ai" }.into(),
                            f.code,
                            f.why,
                        )
                    }
                }
                if !a.program.is_empty() {
                    let g = self.judge.grade(&t.id, &t.ask, &a.program);
                    (a.pass, a.stage, a.code, a.why) = (g.pass, g.stage, g.code, g.why);
                    (ch.program, ch.pass) = (a.program.clone(), g.pass);
                    passes += usize::from(g.pass);
                }
                lines += &a.line(t, ch.sample);
            }
            keep(&lines)?;
        }
        Ok(passes)
    }
}

/// One attempt at a task: its round (1: write; later: fix), the user message sent, the reply,
/// the program it gave (none when it gave none) and its grade, and who made it.
#[derive(Debug, Default)]
struct Attempt {
    round: u32,
    user: String,
    reply: String,
    stop: String,
    program: String,
    pass: bool,
    stage: String,
    code: u16,
    why: String,
    by: By,
}

impl Attempt {
    /// Reads the reply `m` to a request about `before` (the program a fix was shown): a write's
    /// program, or a fix's edits applied (refused, as the coder refuses them, when they drop over
    /// a quarter of its lines) or the program it gave whole.
    fn read(&mut self, m: Msg, before: &str) {
        self.by.teacher = m.model;
        self.stop = m.stop;
        if self.stop == "refusal" {
            (self.stage, self.code, self.why) = ("refusal".into(), codes::REFUSED, m.category);
            return;
        }
        self.reply = m.text;
        let no = |why: &str| Err(("reply".to_string(), codes::NO_PROGRAM, why.to_string()));
        let got = match edits::read(&self.reply, self.stop == "max_tokens") {
            Reply::Program(src) => Ok(src),
            Reply::Edits(es) if self.round > 1 => match edits::apply(before, &es) {
                Ok(src) if edits::lines(&src) * 4 < edits::lines(before) * 3 => {
                    no("its edits dropped over a quarter of the program's lines")
                }
                Ok(src) => Ok(src),
                Err(why) => no(&why),
            },
            Reply::Cut => Err(("cut".into(), codes::CUT, "the reply ran out of max_tokens".into())),
            _ => no("the reply held no program it was asked for"),
        };
        match got {
            Ok(src) => self.program = src,
            Err((stage, code, why)) => (self.stage, self.code, self.why) = (stage, code, why),
        }
    }

    /// Its line, for `t`'s sample `sample`.
    fn line(&self, t: &Task, sample: u32) -> String {
        let mut o = String::from("{\"task\":");
        put(&mut o, &t.id);
        o += ",\"family\":";
        put(&mut o, &t.family);
        o +=
            &format!(",\"tier\":{},\"sample\":{sample},\"round\":{},\"kind\":", t.tier, self.round);
        put(&mut o, if self.round == 1 { "write" } else { "fix" });
        let texts = [
            ("user", &self.user),
            ("reply", &self.reply),
            ("stop", &self.stop),
            ("program", &self.program),
        ];
        for (k, v) in texts {
            o += &[",\"", k, "\":"].concat();
            put(&mut o, v);
        }
        o += &format!(",\"pass\":{},\"stage\":", self.pass);
        put(&mut o, &self.stage);
        o += &format!(",\"code\":{},\"why\":", self.code);
        put(&mut o, &self.why);
        o += ",\"by\":";
        self.by.put(&mut o);
        o += "}\n";
        o
    }
}

/// What `export` left out, and why, and what it wrote.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Exported {
    pub records: usize,
    pub failed: usize,
    pub held: usize,
    pub drift: usize,
    pub dupes: usize,
    pub unreadable: usize,
}

/// The supervised fine-tuning records of `solutions` (attempt lines), sorted by task, write
/// before fix, round and sample: one for each attempt that passed, of a family not `held`, made
/// under the coder's system prompt as it is now (one made under another would teach a prompt
/// never sent: E0993, left out), its program not one exported for its task already. Each holds
/// the conversation as the coder sends it: its system prompt and the one user message (a write's
/// `Make: <ask>`, or a fix's turn with the program numbered and its fault's account), and the
/// reply as the teacher gave it.
pub fn export(solutions: &str, held: &[String]) -> (String, Exported) {
    let system = prompts::solver();
    let prompt = hex16(fnv(system.as_bytes()));
    let mut n = Exported::default();
    let mut kept = Vec::new();
    for line in solutions.lines().filter(|l| !l.trim().is_empty()) {
        let Some(j) = Json::parse(line) else {
            n.unreadable += 1;
            continue;
        };
        let s = |k: &str| j.get(k).and_then(Json::text).unwrap_or("").to_string();
        let num = |k: &str| s(k).parse::<u32>().unwrap_or(0);
        let (by, task) = (By::read(j.get("by")), s("task"));
        match () {
            _ if j.get("pass") != Some(&Json::Bool(true)) => n.failed += 1,
            _ if is_held(held, &s("family"), &task) => n.held += 1,
            _ if by.prompt != prompt => n.drift += 1,
            _ if s("user").is_empty() || s("reply").is_empty() || s("program").is_empty() => {
                n.unreadable += 1
            }
            _ => kept.push((
                task,
                num("round"),
                num("sample"),
                s("program"),
                s("user"),
                s("reply"),
                by,
            )),
        }
    }
    kept.sort_by(|a, b| (&a.0, a.1, a.2, &a.3).cmp(&(&b.0, b.1, b.2, &b.3)));
    let mut out = String::new();
    let mut seen: Vec<(&str, &str)> = Vec::new();
    for (task, round, _, program, user, reply, by) in &kept {
        if seen.contains(&(task.as_str(), program.as_str())) {
            n.dupes += 1;
            continue;
        }
        seen.push((task, program));
        out += "{\"messages\":[{\"role\":\"system\",\"content\":";
        put(&mut out, &system);
        out += "},{\"role\":\"user\",\"content\":";
        put(&mut out, user);
        out += "},{\"role\":\"assistant\",\"content\":";
        put(&mut out, reply);
        out += "}],\"task\":";
        put(&mut out, task);
        out +=
            if *round == 1 { ",\"kind\":\"write\",\"by\":" } else { ",\"kind\":\"fix\",\"by\":" };
        by.put(&mut out);
        out += "}\n";
        n.records += 1;
    }
    (out, n)
}

/// Whether `t` is in `split`: `held` (the held families'), `train` (the rest's) or `all`.
pub fn in_split(split: &str, t: &Task, held: &[String]) -> bool {
    match split {
        "held" => is_held(held, &t.family, &t.id),
        "train" => !is_held(held, &t.family, &t.id),
        _ => true,
    }
}

/// The temperature the coder's make asks a new app at (after its room, `Knobs::write_tokens`).
const WRITE_OPTIONS: &str = ",\"temperature\":0.3";

/// Asks `model` behind `chat` for each of `tasks` in `split`, as Studio asks for a new app: the
/// coder's chat-completions body ([`coder::ai::chat`]) with its system prompt and first turn, its
/// room and temperature. A line per task, `{"task","model","reply"}`, and `"error"` with no reply.
pub fn answers(
    chat: &mut dyn Chat,
    model: &str,
    tasks: &[Task],
    split: &str,
    held: &[String],
) -> String {
    let (system, mut out) = (prompts::solver(), String::new());
    let options =
        format!(",\"max_tokens\":{}{WRITE_OPTIONS}", coder::Knobs::default().write_tokens);
    for t in tasks.iter().filter(|t| in_split(split, t, held)) {
        let got = chat.chat(&coder::ai::chat(model, &options, &system, &prompts::solve(&t.ask)));
        out += "{\"task\":";
        put(&mut out, &t.id);
        out += ",\"model\":";
        put(&mut out, model);
        out += ",\"reply\":";
        put(&mut out, got.as_deref().unwrap_or(""));
        if let Err(e) = &got {
            eprintln!("  {}: {e}", t.id);
            out += ",\"error\":";
            put(&mut out, &e.to_string());
        }
        out += "}\n";
    }
    out
}

/// What [`answers`] would send for each of `tasks` in `split`, a line a task, for a model
/// answered elsewhere in batches (`train/generate.py`): `{"task","family","max_tokens",
/// "temperature","messages":[system, user]}`, the room and temperature Studio asks at.
pub fn prompts(tasks: &[Task], split: &str, held: &[String]) -> String {
    let (system, mut out) = (prompts::solver(), String::new());
    for t in tasks.iter().filter(|t| in_split(split, t, held)) {
        out += "{\"task\":";
        put(&mut out, &t.id);
        out += ",\"family\":";
        put(&mut out, &t.family);
        out += &format!(",\"max_tokens\":{}", coder::Knobs::default().write_tokens);
        out += WRITE_OPTIONS;
        out += ",\"messages\":[{\"role\":\"system\",\"content\":";
        put(&mut out, &system);
        out += "},{\"role\":\"user\",\"content\":";
        put(&mut out, &prompts::solve(&t.ask));
        out += "}]}\n";
    }
    out
}
