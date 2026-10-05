use super::*;

/// The reference program for `id` (in the `makes` crate): one that does what its task asks.
fn reference(id: &str) -> String {
    let path = [env!("CARGO_MANIFEST_DIR"), "/../makes/refs/", id, ".app"].concat();
    std::fs::read_to_string(path).unwrap()
}

/// A scripted free AI: each request answered by the next reply (its content, finish reason and
/// HTTP status), streamed in chunks of 37 bytes 50 ms apart, ending with a usage chunk and the
/// receipt; what the make wants of the rest is honored as curl's wire does.
struct Script {
    replies: Vec<(String, &'static str, u16)>,
    sent: Vec<String>,
}

impl run::Wire for Script {
    fn post(
        &mut self,
        _: &run::At,
        body: &str,
        on: &mut dyn FnMut(&[u8], u64) -> run::Feed,
    ) -> run::Answer {
        self.sent.push(body.into());
        let Some((content, finish, status)) = self.replies.get(self.sent.len() - 1).cloned() else {
            return run::Answer { error: "no reply".into(), ..run::Answer::default() };
        };
        let delta = |k: &str, t: &str| {
            format!(
                "data: {{\"choices\":[{{\"delta\":{{\"{k}\":{}}}}}]}}\n\n",
                coder::json::quote(t)
            )
        };
        let mut sse = match status {
            200 => delta("reasoning", "Thinking it over\u{2026}"),
            _ => "{\"error\":{\"message\":\"rate limited\"}}".into(),
        };
        if status == 200 {
            let chars: Vec<char> = content.chars().collect();
            sse.extend(chars.chunks(23).map(|c| delta("content", &c.iter().collect::<String>())));
            sse += &format!(
                "data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"{finish}\"}}]}}\n\n"
            );
            sse += "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":3000,\"completion_tokens\":1500,\
                    \"prompt_tokens_details\":{\"cached_tokens\":1900},\"completion_tokens_details\":\
                    {\"reasoning_tokens\":400}}}\n\ndata: [DONE]\n\n: receipt in=3000 out=1500 microusd=8123\n\n";
        }
        let (mut t, mut tail, mut feeding) = (0, Vec::new(), true);
        for chunk in sse.as_bytes().chunks(37) {
            t += 50;
            tail.extend_from_slice(chunk);
            if feeding {
                match on(chunk, t) {
                    run::Feed::More => {}
                    run::Feed::Drain => feeding = false,
                    run::Feed::Stop => {
                        return run::Answer { status, ms: t, ..run::Answer::default() };
                    }
                }
            }
        }
        run::Answer {
            status,
            error: String::new(),
            ms: t + 30,
            receipt: uiwire::stat::receipt(&tail),
        }
    }
}

fn meta() -> record::Meta {
    record::Meta {
        run: "test-run".into(),
        date: "2026-10-05".into(),
        commit: "abc1234".into(),
        model: "zai/glm-5.3".into(),
    }
}

/// `task` made over `replies`, then replayed from its exchanges' lines: both records and the
/// exchanges again, which must be the same.
fn round_trip(
    task: &str,
    replies: Vec<(String, &'static str, u16)>,
) -> (record::Record, Vec<replay::Exchange>) {
    let t = makes::find(task).unwrap();
    let mut wire = Script { replies, sent: Vec::new() };
    let (r, log) = run::task(&meta(), t, 1, &mut wire);
    let text: String = log.iter().map(|x| x.line()).collect();
    let mut again = replay::Replay::load(&text);
    assert_eq!(again.exchanges(), &log[..], "the exchanges read back");
    let (r2, log2) = run::task(&meta(), t, 1, &mut again);
    assert!(again.diverged.is_empty(), "{:?}", again.diverged);
    assert_eq!(r2.line(), r.line());
    assert_eq!(log2, log);
    assert_eq!(record::Record::parse(&r.line()), Some(r.clone()));
    (r, log)
}

fn app(src: &str) -> String {
    format!("Here it is.\n```app\n{src}```\nEnjoy it, and tell me what to change next.\n")
}

#[test]
fn a_run_replays_from_its_exchanges_to_the_same_record() {
    // One request: the program is in, the make stops it there and the wire reads on for the
    // receipt.
    let (r, log) = round_trip("counter", vec![(app(&reference("counter")), "stop", 200)]);
    assert_eq!((r.pass, r.stage.as_str(), r.tries, r.est), (true, "ok", 1, 0));
    assert_eq!((r.tokens_in, r.tokens_out, r.usd_micros), (3000, 1500, 8123));
    assert!(log[0].end.is_none() && log[0].receipt == Some([3000, 1500, 8123]));
    // A program that does not compile, fixed by an edit.
    let broken = reference("counter").replace("n += 1;", "n += 1");
    let fix = "<<<<<<< SEARCH\n  button \"+\" { n += 1 }\n=======\n  button \"+\" { n += 1; }\n>>>>>>> REPLACE\n";
    let (r, log) =
        round_trip("counter", vec![(app(&broken), "stop", 200), (fix.into(), "stop", 200)]);
    assert_eq!((r.pass, r.tries, log.len()), (true, 2, 2));
    assert!(log[1].end.is_some(), "edits are read to the reply's end");
    // A program that runs, but not as asked.
    let (r, _) = round_trip(
        "counter",
        vec![(app(&reference("counter").replace("n += 1", "n += 2")), "stop", 200)],
    );
    assert_eq!((r.pass, r.stage.as_str()), (false, "check"));
    // The free AI refusing is the AI's failure, not the model's.
    let (r, _) = round_trip("counter", vec![(String::new(), "stop", 429)]);
    assert_eq!((r.pass, r.stage.as_str(), r.usd_micros), (false, "ai", 0));
    // A reply cut off by its token limit, then no reply at all.
    let cut = format!("```app\n{}", &reference("tetris")[..900]);
    let (r, log) = round_trip("tetris", vec![(cut, "length", 200)]);
    assert_eq!((r.stage.as_str(), log.len()), ("ai", 2));
}

#[test]
fn an_exchange_feeds_the_make_what_its_stream_did() {
    let x = replay::Exchange {
        text: "a \"quote\", \\ a line\nbreak, \u{e9}\u{1f600} \"usage\":{\"prompt_tokens\":9 \"error\":1".into(),
        thought: 1234,
        finish: "length".into(),
        said: "busy \"now\"".into(),
        usage: Some(coder::json::Usage { input: 1, cached: 2, output: 3, reasoning: 4 }),
        other: "{\"error\":{\"message\":\"x\"}}".into(),
        rest: "data: {\"choices\":[{\"delta\":{\"content\":\"tail\"".into(),
        ..replay::Exchange::default()
    };
    let mut s = coder::json::Stream::default();
    let mut text = String::new();
    s.feed(&x.sse(), &mut text, coder::ai::MAX_REPLY);
    assert_eq!((text.as_str(), s.thought, s.finish.as_str()), (x.text.as_str(), 1234, "length"));
    assert_eq!((s.error.as_str(), s.usage), ("busy \"now\"", x.usage));
    s.end(&mut text, coder::ai::MAX_REPLY);
    assert_eq!(text, x.text.clone() + "tail");
    assert_eq!(replay::Exchange::parse(&x.line()), Some(x.clone()));
}

#[test]
fn a_receipt_says_which_model_answered() {
    // Live receipts of GLM 5.3 requests: one it answered (none of its input cached), one Flash
    // answered for it, and Flash's own.
    let (glm, flash) = run::MODELS.into();
    assert_eq!(run::answered(glm, [4741, 734, 9856]), glm);
    assert_eq!(run::answered(glm, [5201, 433, 3818]), glm);
    assert_eq!(run::answered(glm, [4795, 1552, 1495]), flash);
    assert_eq!(run::answered(flash, [4741, 359, 891]), flash);
}

#[test]
fn a_summary_reads_a_gain() {
    let (lo, hi) = summary::wilson(15, 24);
    // 15 of 24: 62.5%, its 95% Wilson interval 42.7% to 78.8%.
    assert!((lo - 0.4271).abs() < 0.0001 && (hi - 0.7884).abs() < 0.0001, "{lo} {hi}");
    assert_eq!(summary::wilson(0, 0), (0.0, 1.0));
    assert!((summary::mcnemar(0, 5) - 0.0625).abs() < 1e-9 && summary::mcnemar(3, 3) == 1.0);
    let rec = |run: &str, task: &str, pass: bool, stage: &str| record::Record {
        meta: record::Meta { run: run.into(), model: run.into(), ..meta() },
        task: task.into(),
        size: "tiny".into(),
        pass,
        stage: stage.into(),
        usd_micros: 1000,
        ..record::Record::default()
    };
    let all = [
        rec("a", "x", true, "ok"),
        rec("a", "y", false, "check"),
        rec("a", "z", false, "ai"),
        rec("b", "x", false, "smoke"),
        rec("b", "y", true, "ok"),
        rec("b", "z", true, "ok"),
    ];
    let text = summary::compare("a", &summary::select(&all, "a"), "b", &summary::select(&all, "b"));
    assert!(text.contains("passed by a only: x") && text.contains("passed by b only: y"), "{text}");
    assert!(text.contains("b - a: +17 points"), "{text}");
    let table = summary::table(&all);
    assert!(
        table.contains(
            "| a | a | abc1234 | 1/2 | 50% (9%-91%) | 1/2 / 0/0 / 0/0 / 0/0 | 0 | 0.0030 | 0 | 1 | 0/0 |"
        ),
        "{table}"
    );
}

/// Every recorded run (`evals/replays/studio/<run>.jsonl`) replays offline to the records kept
/// for it (`evals/results/studio.jsonl`), byte for byte, while the harness and the suite are the
/// ones that made them: the same prompt, knobs and suite, and every request the same. Once they
/// are not (the coder or a checker changed), its records are stale until it is graded again
/// (`cargo run -p eval -- replay --run <run> --write`, offline), and it must at least replay
/// the same way twice, each exchange by its place.
#[test]
fn recorded_runs_replay_to_their_records() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../evals");
    let read = |p: String| std::fs::read_to_string(p).unwrap_or_default();
    let results = record::load(&read(format!("{dir}/results/studio.jsonl")));
    let Ok(files) = std::fs::read_dir(format!("{dir}/replays/studio")) else { return };
    let ours = (prompt_hash(), knobs_hash(&coder::Knobs::default()), makes::hash());
    for path in files.map(|f| f.unwrap().path()) {
        let run = path.file_stem().unwrap().to_string_lossy().to_string();
        let text = std::fs::read_to_string(&path).unwrap();
        let kept: Vec<&record::Record> = results.iter().filter(|r| r.meta.run == run).collect();
        assert!(!kept.is_empty(), "{run}: a recorded run with no records");
        let replay = |loose: bool| {
            let mut wire = replay::Replay::load(&text);
            wire.loose = loose;
            let lines: Vec<String> = kept
                .iter()
                .map(|r| {
                    run::task(&r.meta, makes::find(&r.task).unwrap(), r.trial, &mut wire).0.line()
                })
                .collect();
            (lines, wire.diverged)
        };
        let (lines, diverged) = replay(false);
        let same = kept.iter().all(|r| (r.prompt, r.knobs, r.suite_hash) == ours);
        if same && diverged.is_empty() {
            for (r, line) in kept.iter().zip(&lines) {
                assert_eq!(
                    *line,
                    r.line(),
                    "{run}: {} trial {} replays differently",
                    r.task,
                    r.trial
                );
            }
        } else {
            eprintln!(
                "{run}: made by another harness or suite ({} requests differ): stale",
                diverged.len()
            );
            assert_eq!(replay(true), replay(true), "{run} replays loosely the same twice");
        }
    }
}
