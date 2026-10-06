//! Tests, all offline: recorded replies read (a Messages reply, a stream, a refusal, a fallback,
//! a batch's results, an OpenAI-compatible chat), cost math and the ledger, the prompts' bytes,
//! the seam's stand-in, and the commands run against a fake teacher.

use applang::SHOTS;
use coder::json::{Json, quote};

use crate::claude::{self, Msg, Replies, Req, Teacher, Usage};
use crate::cost::{self, Ledger};
use crate::run::{self, Run};
use crate::seam::{By, Grade, Iq, Judge, Task};
use crate::wire::{self, Chat};
use crate::{Fail, codes, day, fnv, hex16, prompts};

/// A recorded Messages reply: a thinking block (empty, as Opus 5.5 returns it by default), then
/// two text blocks.
const REPLY: &str = r#"{"id":"msg_01","type":"message","role":"assistant","model":"claude-opus-5-5",
"content":[{"type":"thinking","thinking":"","signature":"EqQB"},{"type":"text","text":"```app\n// A counter.\nlabel 1;\n```"},
{"type":"text","text":"\n"}],"stop_reason":"end_turn","stop_details":null,
"usage":{"input_tokens":12,"cache_creation_input_tokens":2500,"cache_read_input_tokens":0,
"cache_creation":{"ephemeral_5m_input_tokens":2500,"ephemeral_1h_input_tokens":0},"output_tokens":900}}"#;

/// A refusal: its partial text is never read.
const REFUSAL: &str = r#"{"id":"msg_02","type":"message","model":"claude-opus-5-5",
"content":[{"type":"text","text":"Sure, here"}],"stop_reason":"refusal",
"stop_details":{"type":"refusal","category":"cyber","explanation":"declined"},
"usage":{"input_tokens":30,"output_tokens":4}}"#;

/// A reply a fallback served: each attempt in `usage.iterations`.
const FELL: &str = r#"{"id":"msg_04","type":"message","model":"claude-opus-5",
"content":[{"type":"fallback","from":{"model":"claude-opus-5-5"},"to":{"model":"claude-opus-5"}},
{"type":"text","text":"ok"}],"stop_reason":"end_turn",
"usage":{"input_tokens":100,"output_tokens":50,"iterations":[
{"type":"message","input_tokens":100,"output_tokens":0},
{"type":"fallback_message","input_tokens":100,"output_tokens":50}]}}"#;

/// A streamed reply, as it comes.
const SSE: &str = "event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_03\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-opus-5-5\",\"content\":[],\"stop_reason\":null,\"usage\":{\"input_tokens\":12,\"cache_read_input_tokens\":2500,\"output_tokens\":1}}}\n\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\",\"signature\":\"\"}}\n\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"abc\"}}\n\n\
event: ping\ndata: {\"type\":\"ping\"}\n\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\", world\"}}\n\n\
event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":321}}\n\n\
event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";

fn json(s: &str) -> Json {
    Json::parse(&s.replace('\n', "")).expect("a fixture is JSON")
}

#[test]
fn a_reply_is_read_by_its_stop_reason_then_its_blocks_by_type() {
    let m = Msg::read(&json(REPLY), "claude-opus-5-5").unwrap();
    assert_eq!((m.id.as_str(), m.stop.as_str()), ("msg_01", "end_turn"));
    assert_eq!(m.text, "```app\n// A counter.\nlabel 1;\n```\n");
    let u = Usage { input: 12, output: 900, write_5m: 2500, ..Usage::default() };
    assert_eq!(m.parts, vec![("claude-opus-5-5".to_string(), u)]);
    assert_eq!(m.short(), None);
    // A refusal: its content never read, its category kept.
    let r = Msg::read(&json(REFUSAL), "claude-opus-5-5").unwrap();
    assert_eq!((r.text.as_str(), r.category.as_str()), ("", "cyber"));
    assert_eq!(r.short().map(|f| f.code), Some(codes::REFUSED));
    // A fallback: each attempt by the model that ran it, the text the server's.
    let f = Msg::read(&json(FELL), "claude-opus-5-5").unwrap();
    assert_eq!((f.model.as_str(), f.text.as_str()), ("claude-opus-5", "ok"));
    let models: Vec<&str> = f.parts.iter().map(|p| p.0.as_str()).collect();
    assert_eq!(models, ["claude-opus-5-5", "claude-opus-5"]);
    // 100 in at $4 a million, then 100 in and 50 out at $5 and $25.
    assert_eq!(cost::msg_nanos(&f, false), 100 * 4_000 + 100 * 5_000 + 50 * 25_000);
    // A writer's cache write with no breakdown is counted as the dearer hour's.
    let j =
        json(r#"{"type":"message","usage":{"input_tokens":1,"cache_creation_input_tokens":7}}"#);
    assert_eq!(Msg::read(&j, "m").unwrap().parts[0].1.write_1h, 7);
    // An error body is coded: busy ones are retried, others are final.
    let busy =
        json(r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#);
    assert_eq!(
        Msg::read(&busy, "m").unwrap_err(),
        Fail::new(codes::BUSY, "overloaded_error: Overloaded")
    );
    let bad = json(r#"{"type":"error","error":{"type":"invalid_request_error","message":"no"}}"#);
    assert_eq!(claude::api_error(&bad).unwrap().code, codes::HTTP);
}

#[test]
fn a_stream_assembles_into_the_message_it_streams() {
    let m = claude::from_sse(SSE, "claude-opus-5-5").unwrap();
    assert_eq!(
        (m.id.as_str(), m.stop.as_str(), m.text.as_str()),
        ("msg_03", "end_turn", "Hello, world")
    );
    let u = Usage { input: 12, output: 321, cache_read: 2500, ..Usage::default() };
    assert_eq!(m.parts, vec![("claude-opus-5-5".to_string(), u)]);
    // Cut before its end, it is unreadable, never a short reply taken whole.
    let cut = SSE.split("event: message_delta").next().unwrap();
    assert_eq!(claude::from_sse(cut, "m").unwrap_err().code, codes::UNREADABLE);
    // An error mid-stream is the API's, coded.
    let over = "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n";
    assert_eq!(claude::from_sse(over, "m").unwrap_err().code, codes::BUSY);
}

#[test]
fn a_batchs_results_map_by_custom_id() {
    let ok = REPLY.replace('\n', "");
    let lines = [
        format!(r#"{{"custom_id":"r1-t0-s1","result":{{"type":"succeeded","message":{ok}}}}}"#),
        r#"{"custom_id":"r1-t0-s0","result":{"type":"errored","error":{"type":"error","error":{"type":"invalid_request_error","message":"too big"}}}}"#.into(),
        r#"{"custom_id":"r1-t1-s0","result":{"type":"expired"}}"#.into(),
    ];
    let got: Replies =
        lines.iter().filter_map(|l| claude::batch_line(l, "claude-opus-5-5")).collect();
    let find = |id: &str| got.iter().find(|g| g.0 == id).map(|g| g.1.clone()).unwrap();
    assert_eq!(find("r1-t0-s1").unwrap().id, "msg_01");
    let e = find("r1-t0-s0").unwrap_err();
    assert_eq!(
        (e.code, e.why.as_str()),
        (codes::BATCH, "the request errored: invalid_request_error: too big")
    );
    assert_eq!(find("r1-t1-s0").unwrap_err(), Fail::new(codes::BATCH, "the request was expired"));
}

#[test]
fn an_openai_compatible_chat_reply_is_read_streamed_or_whole() {
    let sse = "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"Hel\"}}]}\n\n\
               data: {\"choices\":[{\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"stop\"}]}\n\n\
               data: {\"choices\":[],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":2}}\n\ndata: [DONE]\n";
    assert_eq!(wire::chat_reply(sse).unwrap(), "Hello");
    let whole = r#"{"id":"c1","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"Hi"},"finish_reason":"stop"}]}"#;
    assert_eq!(wire::chat_reply(whole).unwrap(), "Hi");
    let error = "data: {\"error\":{\"message\":\"model not loaded\"}}\n";
    assert_eq!(wire::chat_reply(error).unwrap_err(), Fail::new(codes::HTTP, "model not loaded"));
    assert_eq!(wire::chat_reply("<html>").unwrap_err().code, codes::UNREADABLE);
}

#[test]
fn the_key_comes_from_the_environment_and_goes_only_to_curls_stdin() {
    for v in [None, Some(""), Some("sk\"x"), Some("a b"), Some("k\nheader = x")] {
        assert_eq!(wire::key_from(v.map(String::from)).unwrap_err().code, codes::NO_KEY);
    }
    let key = "sk-ant-api03-Ab_9";
    assert_eq!(wire::key_from(Some(key.into())).unwrap(), key);
    let cfg = wire::config("https://api.anthropic.com/v1/messages", &[format!("x-api-key: {key}")]);
    assert_eq!(
        cfg,
        format!("url = \"https://api.anthropic.com/v1/messages\"\nheader = \"x-api-key: {key}\"\n")
    );
    assert_eq!(wire::config("u", &["a\"b\\c".into()]), "url = \"u\"\nheader = \"a\\\"b\\\\c\"\n");
    // The command line holds no header and no URL: both come on stdin.
    let args = wire::args("POST", 60, Some("body.json")).join(" ");
    assert!(
        args.starts_with("-sS --config - -X POST --max-time 60 -w")
            && args.ends_with("--data-binary @body.json")
    );
    assert!(!args.contains("x-api-key") && !args.contains("https://"));
}

#[test]
fn dollars_come_from_tokens_at_list_prices_and_batches_at_half() {
    let m = |u: Usage| cost::nanos("claude-opus-5-5", &u, false);
    let million = 1_000_000;
    assert_eq!(cost::usd(m(Usage { input: million, ..Usage::default() })), "4.000000");
    assert_eq!(cost::usd(m(Usage { output: million, ..Usage::default() })), "20.000000");
    assert_eq!(cost::usd(m(Usage { cache_read: million, ..Usage::default() })), "0.200000");
    assert_eq!(cost::usd(m(Usage { write_5m: million, ..Usage::default() })), "5.000000");
    assert_eq!(cost::usd(m(Usage { write_1h: million, ..Usage::default() })), "8.000000");
    let both = Usage { input: million, output: million, ..Usage::default() };
    assert_eq!(cost::usd(cost::nanos("claude-opus-5-5", &both, true)), "12.000000");
    assert_eq!(cost::usd(cost::nanos("claude-opus-5", &both, false)), "30.000000");
    assert_eq!(cost::usd(1_234_567), "0.001234");
    for (s, n) in [
        ("12", 12_000_000_000),
        ("0.5", 500_000_000),
        ("3.25", 3_250_000_000),
        (".5", 500_000_000),
        ("1.", 1_000_000_000),
    ] {
        assert_eq!(cost::parse_usd(s), Some(n), "{s}");
    }
    for s in ["", ".", "1.2.3", "abc", "-1", "1e3"] {
        assert_eq!(cost::parse_usd(s), None, "{s}");
    }
    // The worst case: a token per 3 bytes in, all the room out.
    let r = Req {
        system: "x".repeat(300),
        user: String::new(),
        max_tokens: 1000,
        effort: "high".into(),
    };
    assert_eq!(cost::worst(&r, "claude-opus-5-5", false), 100 * 4_000 + 1000 * 20_000);
    assert_eq!(cost::worst(&r, "claude-opus-5-5", true), (100 * 4_000 + 1000 * 20_000) / 2);
}

/// A ledger in the temp directory, empty, for the test `name`.
fn ledger(name: &str, cmd: &str, cap: Option<u64>) -> Ledger {
    let path = std::env::temp_dir().join(format!("teach-test-{}-{name}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&path);
    Ledger {
        path: path.to_string_lossy().into(),
        cmd: cmd.into(),
        day: "2026-10-05".into(),
        cap,
        spent: 0,
    }
}

#[test]
fn the_ledger_sums_by_day_and_command() {
    let mut l = ledger("sums", "solve", None);
    let m = Msg::read(&json(REPLY), "claude-opus-5-5").unwrap();
    l.note("r1-t0-s0", &m, false).unwrap();
    l.note("r1-t0-s1", &m, true).unwrap();
    l.cmd = "tasks".into();
    l.day = "2026-10-06".into();
    l.note("f0", &m, false).unwrap();
    let text = std::fs::read_to_string(&l.path).unwrap();
    let first = text.lines().next().unwrap();
    assert!(first.starts_with(r#"{"day":"2026-10-05","cmd":"solve","model":"claude-opus-5-5","id":"r1-t0-s0","msg":"msg_01","stop":"end_turn","batch":false,"in":12,"out":900,"cache_read":0,"cache_write":2500,"nanos":30548000,"usd":"0.030548"}"#), "{first}");
    let one = 12 * 4_000 + 900 * 20_000 + 2500 * 5_000;
    assert_eq!(l.spent, one + one / 2 + one);
    let s = cost::summary(&text);
    let rows: Vec<&str> = s.lines().collect();
    assert_eq!(rows.len(), 4, "{s}");
    assert!(
        rows[1].starts_with("2026-10-05  solve       2") && rows[1].ends_with("0.045822"),
        "{s}"
    );
    assert!(rows[2].starts_with("2026-10-06  tasks       1"), "{s}");
    assert!(
        rows[3].starts_with("total                   3") && rows[3].ends_with("0.076370"),
        "{s}"
    );
}

#[test]
fn the_solver_sends_the_coders_own_bytes() {
    let ask = "a counter: a label shows the count, starting at 0";
    assert_eq!(prompts::solver(), coder::prompt::system());
    assert_eq!(prompts::solve(ask), coder::prompt::write(ask));
    let r = Req {
        system: prompts::solver(),
        user: prompts::solve(ask),
        max_tokens: 32_000,
        effort: "high".into(),
    };
    for live in [true, false] {
        let j = Json::parse(&claude::body(&r, "claude-opus-5-5", live)).unwrap();
        let system = j.get("system").and_then(|s| s.at(0));
        assert_eq!(
            system.and_then(|s| s.get("text")).and_then(Json::text),
            Some(coder::prompt::system().as_str())
        );
        let user = j.get("messages").and_then(|m| m.at(0)).and_then(|m| m.get("content"));
        assert_eq!(user.and_then(Json::text), Some(["Make: ", ask].concat().as_str()));
        let at = |k: &str, m: &str| {
            j.get(k).and_then(|o| o.get(m)).and_then(Json::text).map(String::from)
        };
        assert_eq!(at("thinking", "type").as_deref(), Some("adaptive"));
        assert_eq!(at("output_config", "effort").as_deref(), Some("high"));
        // Live: streamed, with fallbacks by refusal category; a batch takes neither.
        assert_eq!(j.get("fallbacks").and_then(Json::text), live.then_some("default"));
        assert_eq!(j.get("stream").is_some(), live);
        let ttl = system.and_then(|s| s.get("cache_control")).and_then(|c| c.get("ttl"));
        assert_eq!(ttl.and_then(Json::text), Some("1h"));
    }
    let b = claude::batch_body(
        &[("r1-t0-s0".into(), r.clone()), ("r1-t0-s1".into(), r)],
        "claude-opus-5-5",
    );
    let j = Json::parse(&b).unwrap();
    let second = j.get("requests").and_then(|q| q.at(1)).unwrap();
    assert_eq!(second.get("custom_id").and_then(Json::text), Some("r1-t0-s1"));
    assert_eq!(
        second.get("params").and_then(|p| p.get("max_tokens")).and_then(Json::text),
        Some("32000")
    );
}

/// The snake shot, one semicolon short, and the edit block that puts it back.
fn broken_snake() -> (String, String) {
    let src = SHOTS[0].1.replacen("state speed = 0;", "state speed = 0", 1);
    let bad = src.lines().find(|l| l.starts_with("state speed")).unwrap();
    let good = SHOTS[0].1.lines().find(|l| l.starts_with("state speed")).unwrap();
    let edit = format!("<<<<<<< SEARCH\n{bad}\n=======\n{good}\n>>>>>>> REPLACE\n");
    (src, edit)
}

#[test]
fn the_fixer_sends_the_coders_own_fix_turn() {
    let (src, _) = broken_snake();
    let f = coder::ai::fault(&src, "", 3).unwrap();
    assert!(!f.compiles);
    assert_eq!(
        prompts::fix("make snake", &src),
        Some(coder::prompt::fix("make snake", &src, &f.account))
    );
    assert_eq!(prompts::fix("make snake", SHOTS[0].1), None);
}

/// The writer's prompt's hash with the card `CARD`: a change to it is a decision, never a drift.
const WRITER_FNV: u64 = 0x9c27_6f11_bd53_79ab;

#[test]
fn the_writer_is_given_applang_the_card_and_the_ladder() {
    let w = prompts::writer("CARD: press \"Start\"; expect label \"0\"\n");
    assert!(w.contains(applang::REFERENCE) && w.contains(&coder::prompt::system()));
    assert!(SHOTS.iter().all(|(ask, src)| w.contains(ask) && w.contains(src)));
    assert!(w.contains(
        "<checker_language>\nCARD: press \"Start\"; expect label \"0\"\n</checker_language>"
    ));
    for rung in [
        "1. A counter",
        "2. A small tool",
        "3. A simple game",
        "4. A full classic",
        "5. A game with levels",
        "6. Ambitious",
    ] {
        assert!(w.contains(rung), "{rung}");
    }
    assert_eq!(fnv(w.as_bytes()), WRITER_FNV, "{:#x}", fnv(w.as_bytes()));
    let u = prompts::writer_user(3, "snake", 2, &["snake-old".into()]);
    assert_eq!(
        u,
        "Tier 3, family snake: write 2 tasks. Ids taken already (use none again, nor make the same app): snake-old."
    );
    assert_eq!(
        prompts::writer_user(1, "counter", 3, &[]),
        "Tier 1, family counter: write 3 tasks."
    );
}

fn task(id: &str, family: &str, reference: &str) -> Task {
    let ask = "make snake: arrows steer; Start starts".to_string();
    Task {
        id: id.into(),
        tier: 3,
        family: family.into(),
        ask,
        check: "press Start".into(),
        reference: reference.into(),
        by: By::default(),
    }
}

#[test]
fn a_task_line_keeps_the_schema_and_the_stand_in_verifies_it() {
    let mut t = task("snake-wrap", "snake", "label 1;");
    t.by = By {
        teacher: "claude-opus-5-5".into(),
        prompt: "0123456789abcdef".into(),
        verifier: "fedcba9876543210".into(),
        day: "2026-10-05".into(),
    };
    let want = r#"{"id":"snake-wrap","tier":3,"family":"snake","ask":"make snake: arrows steer; Start starts","check":"press Start","ref":"label 1;","by":{"teacher":"claude-opus-5-5","prompt":"0123456789abcdef","verifier":"fedcba9876543210","day":"2026-10-05"}}"#;
    assert_eq!(t.line(), [want, "\n"].concat());
    assert_eq!(Task::parse(want), Some(t));
    let s = Smoke;
    assert_eq!(s.id().len(), 16);
    let line = |t: Task| t.line();
    assert!(s.verify_task(&line(task("snake-wrap", "snake", SHOTS[0].1))).is_ok());
    let (broken, _) = broken_snake();
    let refused = s.verify_task(&line(task("snake-semi", "snake", &broken))).unwrap_err();
    assert!(refused.starts_with("its ref fails at compile: E0"), "{refused}");
    for (id, fam) in [("wrap", "snake"), ("snake-", "snake"), ("Snake-x", "snake"), ("snake-x", "")]
    {
        assert!(s.verify_task(&line(task(id, fam, SHOTS[0].1))).is_err(), "{id}");
    }
    assert_eq!(s.verify_task("{}").unwrap_err(), "not a task line");
    let g = s.grade("x", "y", SHOTS[1].1);
    assert_eq!((g.pass, g.stage.as_str(), g.code), (true, "ok", 0));
    let g = s.grade("x", "y", &broken);
    assert_eq!((g.pass, g.stage.as_str()), (false, "compile"));
    let g = s.grade("x", "y", "// A.\nlabel 1 / 0;");
    assert_eq!((g.pass, g.stage.as_str(), g.code), (false, "smoke", 203));
    let held = crate::seam::held("snake\n\n# held for iq\n tetris \n");
    assert_eq!(held, ["snake", "tetris"]);
    let is = |family: &str, id: &str| crate::seam::is_held(&held, family, id);
    assert!(is("snake", "x") && is("", "snake-wrap") && is("tetris", "tetris-t"));
    assert!(!is("snakes", "snakes-wrap") && !is("", "snakes") && !is("memory", "memory-snake"));
    assert!(is("snake-ai", "snake-ai-chase") && is("", "snake"), "a near twin is held by its root");
}

/// The held-out list teach derives from a suite is every family [`iq::held`] holds, so every
/// family under a held root (pong-ai with pong, level-editor with platformer), and `is_held` of
/// that list agrees with `iq::held` on every task; a list naming a twin holds its root's every
/// family too.
#[test]
fn derived_held_lists_hold_near_twins_with_their_root() {
    let t =
        |id: &str, family: &str| Task { id: id.into(), family: family.into(), ..Task::default() };
    let tasks = [
        t("pong-computer", "pong"),
        t("pong-ai-spin", "pong-ai"),
        t("pong-ai-first-to-3", "pong-ai"),
        t("tip-basic", "tip"),
        t("counter", "counter"),
        t("lamp-switch-on", "lamp-switch"),
        t("level-editor-platform", "level-editor"),
    ];
    assert!(iq::held("pong") && iq::held("counter") && !iq::held("tip") && !iq::held("lamp"));
    let held = crate::seam::held_of(&tasks);
    assert_eq!(held, ["counter", "level-editor", "pong", "pong-ai"]);
    // Joined roots ([`iq::JOINED`]): a list naming only `platformer` holds the level editors.
    let platformer = ["platformer".to_string()];
    assert!(crate::seam::is_held(&platformer, "level-editor", "level-editor-platform"));
    assert!(crate::seam::is_held(&platformer, "", "level-editor-boxes"));
    for task in &tasks {
        let is = crate::seam::is_held(&held, &task.family, &task.id);
        assert_eq!(is, iq::held(&task.family), "{}", task.id);
        assert_eq!(run::in_split("held", task, &held), is);
        assert_eq!(run::in_split("train", task, &held), !is);
    }
    // A list older than the twin, or naming only the twin, still holds the whole root.
    for list in [vec!["pong".to_string()], vec!["pong-ai".to_string()]] {
        assert!(tasks[..3].iter().all(|t| crate::seam::is_held(&list, &t.family, &t.id)));
        assert!(!crate::seam::is_held(&list, "tip", "tip-basic"));
    }
}

/// A teacher that answers each request with `answer`, keeping each request; its batches answer
/// in reverse, as a batch may.
struct Fake {
    answer: fn(&Req) -> String,
    asked: Vec<Req>,
    batches: usize,
}

impl Teacher for Fake {
    fn model(&self) -> &str {
        "claude-opus-5-5"
    }

    fn call(&mut self, req: &Req) -> Result<Msg, Fail> {
        self.asked.push(req.clone());
        let usage = Usage { input: 10, output: 20, ..Usage::default() };
        let id = format!("msg_{}", self.asked.len());
        let (model, stop, text) = ("claude-opus-5-5".into(), "end_turn".into(), (self.answer)(req));
        Ok(Msg {
            id,
            model,
            stop,
            category: String::new(),
            text,
            parts: vec![("claude-opus-5-5".into(), usage)],
        })
    }

    fn batch(&mut self, reqs: &[(String, Req)]) -> Result<Replies, Fail> {
        self.batches += 1;
        Ok(reqs.iter().map(|(id, r)| (id.clone(), self.call(r))).rev().collect())
    }
}

fn run<'a>(fake: &'a mut Fake, judge: &'a Smoke, name: &str, batch: bool) -> Run<'a> {
    let (effort, day) = ("high".into(), "2026-10-05".into());
    Run {
        teacher: fake,
        judge,
        ledger: ledger(name, "solve", None),
        batch,
        effort,
        max_tokens: 1000,
        day,
    }
}

#[test]
fn a_batch_answers_in_the_order_asked_and_the_budget_holds_requests_back() {
    let mut fake = Fake { answer: |r| r.user.clone(), asked: Vec::new(), batches: 0 };
    let req = |u: &str| Req {
        system: "s".into(),
        user: u.into(),
        max_tokens: 1000,
        effort: "high".into(),
    };
    let reqs = vec![("a".to_string(), req("1")), ("b".to_string(), req("2"))];
    let mut r = run(&mut fake, &Smoke, "order", true);
    let got = r.ask(reqs.clone());
    let texts: Vec<(&str, &str)> =
        got.iter().map(|(id, m)| (id.as_str(), m.as_ref().unwrap().text.as_str())).collect();
    assert_eq!(texts, [("a", "1"), ("b", "2")]);
    assert_eq!(r.ledger.spent, 2 * (10 * 4_000 + 20 * 20_000) / 2);
    // Room for one at its worst: the second is refused unsent, live or batched.
    for batch in [true, false] {
        r.batch = batch;
        r.ledger.cap = Some(r.ledger.spent + cost::worst(&reqs[0].1, "claude-opus-5-5", batch));
        let got = r.ask(reqs.clone());
        assert!(got[0].1.is_ok() && got[1].1.as_ref().unwrap_err().code == codes::BUDGET);
    }
    assert_eq!((fake.asked.len(), fake.batches), (4, 2));
}

#[test]
fn tasks_are_kept_only_as_asked_and_as_verified() {
    let mut fake = Fake {
        answer: |_| {
            let good = |id: &str, family: &str, src: &str| {
                format!(
                    r#"{{"id":"{id}","tier":3,"family":"{family}","ask":"make snake","check":"press Start","ref":{}}}"#,
                    quote(src)
                )
            };
            let fenced = ["```app\n", SHOTS[0].1, "```\n"].concat();
            [
                "Here are the tasks:".into(),
                good("snake-classic", "snake", &fenced),
                good("memory-pairs", "memory", SHOTS[0].1),
                "{not json".into(),
                good("snake-broken", "snake", "state x = ;"),
                good("snake-old", "snake", SHOTS[0].1),
            ]
            .join("\n")
        },
        asked: Vec::new(),
        batches: 0,
    };
    let mut r = run(&mut fake, &Smoke, "tasks", false);
    let card = "CARD";
    let w = r.tasks(card, 3, &["snake".into()], 2, &["snake-old".into()]);
    assert_eq!(w.kept.len(), 1, "{:?}", w.refused);
    let t = Task::parse(&w.kept[0]).unwrap();
    assert_eq!((t.id.as_str(), t.reference.as_str()), ("snake-classic", SHOTS[0].1));
    let prompt = hex16(fnv(prompts::writer(card).as_bytes()));
    let by = By {
        teacher: "claude-opus-5-5".into(),
        prompt,
        verifier: Smoke.id(),
        day: "2026-10-05".into(),
    };
    assert_eq!(t.by, by);
    let whys: Vec<&str> = w.refused.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(whys.len(), 4, "{whys:?}");
    assert!(whys.iter().all(|w| w.starts_with("E0994 ")), "{whys:?}");
    assert!(
        whys[0].contains("asked for tier 3 snake, it is tier 3 memory")
            && whys[3].contains("snake-old is taken")
    );
    assert_eq!(fake.asked[0].user, prompts::writer_user(3, "snake", 2, &["snake-old".into()]));
    assert_eq!(fake.asked[0].system, prompts::writer(card));
}

/// Answers a write with the snake shot one semicolon short, and a fix with the edit that mends it.
fn snake_teacher(r: &Req) -> String {
    let (src, edit) = broken_snake();
    match r.user.starts_with("Make: ") {
        true => ["```app\n", &src, "```\n"].concat(),
        false => edit,
    }
}

/// An attempt line as `solve` writes one, for `export`.
fn attempt(
    task: &str,
    family: &str,
    round: u32,
    program: &str,
    pass: bool,
    prompt: &str,
) -> String {
    let kind = if round == 1 { "write" } else { "fix" };
    format!(
        r#"{{"task":"{task}","family":"{family}","tier":1,"sample":0,"round":{round},"kind":"{kind}","user":"Make: x","reply":"r","stop":"end_turn","program":{},"pass":{pass},"stage":"ok","code":0,"why":"","by":{{"teacher":"claude-opus-5-5","prompt":"{prompt}","verifier":"v","day":"2026-10-05"}}}}"#,
        quote(program)
    )
}

#[test]
fn solving_writes_then_fixes_with_the_coders_turns_and_never_touches_held_tasks() {
    for batch in [false, true] {
        let mut fake = Fake { answer: snake_teacher, asked: Vec::new(), batches: 0 };
        let tasks = [task("snake-classic", "snake", ""), task("tetris-classic", "tetris", "")];
        let mut tasks = tasks.to_vec();
        tasks[1].ask = "make tetris, held out".into();
        let mut lines = String::new();
        let mut r = run(&mut fake, &Smoke, "solve", batch);
        let passes = r
            .solve(&tasks, &["tetris".into()], 1, 3, &mut |l| {
                lines += l;
                Ok(())
            })
            .unwrap();
        assert_eq!(passes, 1);
        // A write, then the fix turn the coder sends; the third round has nothing to send.
        let (src, edit) = broken_snake();
        let f = coder::ai::fault(&src, "", 3).unwrap();
        let users: Vec<&str> = fake.asked.iter().map(|q| q.user.as_str()).collect();
        assert_eq!(
            users,
            [
                coder::prompt::write(&tasks[0].ask),
                coder::prompt::fix(&tasks[0].ask, &src, &f.account)
            ]
        );
        assert!(
            fake.asked
                .iter()
                .all(|q| q.system == coder::prompt::system() && !q.user.contains("tetris"))
        );
        assert_eq!(fake.batches, if batch { 2 } else { 0 });
        let rows: Vec<Json> = lines.lines().map(|l| Json::parse(l).unwrap()).collect();
        let s = |j: &Json, k: &str| j.get(k).and_then(Json::text).unwrap_or("").to_string();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            (s(&rows[0], "kind"), s(&rows[0], "stage"), s(&rows[0], "program")),
            ("write".into(), "compile".into(), src.clone())
        );
        assert_eq!(
            (s(&rows[1], "kind"), s(&rows[1], "stage"), s(&rows[1], "reply")),
            ("fix".into(), "ok".into(), edit.clone())
        );
        assert_eq!(rows[1].get("pass"), Some(&Json::Bool(true)));
        assert_eq!(s(&rows[1], "program"), SHOTS[0].1);
        // Exported: the fix alone, as the coder sends it.
        let (sft, n) = run::export(&lines, &["tetris".into()]);
        assert_eq!((n.records, n.failed), (1, 1));
        let j = Json::parse(sft.trim_end()).unwrap();
        let content = |i: usize| {
            j.get("messages")
                .and_then(|m| m.at(i))
                .and_then(|m| m.get("content"))
                .and_then(Json::text)
                .map(String::from)
        };
        assert_eq!(content(1), Some(users[1].to_string()));
        assert_eq!(content(2), Some(edit));
    }
}

#[test]
fn export_keeps_verified_records_in_the_schema_and_never_a_held_family() {
    let prompt = hex16(fnv(coder::prompt::system().as_bytes()));
    let lines = [
        attempt("tip-basic", "tip", 2, "label 2;", true, &prompt),
        attempt("tip-basic", "tip", 1, "label 1;", true, &prompt),
        attempt("tip-basic", "tip", 1, "label 1;", true, &prompt),
        attempt("tip-basic", "tip", 1, "label 3;", false, &prompt),
        attempt("snake-wrap", "snake", 1, "label 4;", true, &prompt),
        attempt("snake-lost", "", 1, "label 6;", true, &prompt),
        attempt("clock-face", "clock", 1, "label 5;", true, "0000000000000000"),
        "{oops".into(),
    ];
    let (out, n) = run::export(&lines.join("\n"), &["snake".into()]);
    let want = run::Exported { records: 2, failed: 1, held: 2, drift: 1, dupes: 1, unreadable: 1 };
    assert_eq!(n, want);
    assert!(!out.contains("snake-wrap") && !out.contains("snake-lost"));
    let records: Vec<&str> = out.lines().collect();
    let head = format!(
        "{{\"messages\":[{{\"role\":\"system\",\"content\":{}}},{{\"role\":\"user\",\"content\":\"Make: x\"}},{{\"role\":\"assistant\",\"content\":\"r\"}}],\"task\":\"tip-basic\",",
        quote(&coder::prompt::system())
    );
    let by = format!(
        "\"by\":{{\"teacher\":\"claude-opus-5-5\",\"prompt\":\"{prompt}\",\"verifier\":\"v\",\"day\":\"2026-10-05\"}}}}"
    );
    assert_eq!(
        records,
        [format!("{head}\"kind\":\"write\",{by}"), format!("{head}\"kind\":\"fix\",{by}")]
    );
    // The same set in another order exports the same bytes.
    let mut back = lines.to_vec();
    back.reverse();
    assert_eq!(run::export(&back.join("\n"), &["snake".into()]).0, out);
}

/// A chat endpoint that keeps each body and answers with its length.
struct Echo(Vec<String>);

impl Chat for Echo {
    fn chat(&mut self, body: &str) -> Result<String, Fail> {
        self.0.push(body.into());
        Ok(body.len().to_string())
    }
}

#[test]
fn the_models_taught_are_asked_as_studio_asks() {
    let tasks = [task("snake-wrap", "snake", ""), task("tip-basic", "tip", "")];
    let held = ["snake".to_string()];
    let mut e = Echo(Vec::new());
    let out = run::answers(&mut e, "qwen", &tasks, "held", &held);
    let body = coder::ai::chat(
        "qwen",
        ",\"max_tokens\":6144,\"temperature\":0.3",
        &coder::prompt::system(),
        &coder::prompt::write(&tasks[0].ask),
    );
    let line =
        format!("{{\"task\":\"snake-wrap\",\"model\":\"qwen\",\"reply\":\"{}\"}}\n", body.len());
    assert_eq!((e.0, out), (vec![body], line));
    e = Echo(Vec::new());
    assert_eq!(run::answers(&mut e, "qwen", &tasks, "train", &held).lines().count(), 1);
    assert_eq!(run::answers(&mut e, "qwen", &tasks, "all", &held).lines().count(), 2);
}

#[test]
fn days_and_codes_read_as_they_should() {
    assert_eq!(
        (day(0), day(86_399), day(86_400)),
        ("1970-01-01".into(), "1970-01-01".into(), "1970-01-02".into())
    );
    assert_eq!(day(951_868_800), "2000-03-01");
    assert_eq!(Fail::new(codes::BUDGET, "spent").to_string(), "E0987 spent");
    assert_eq!(hex16(fnv(b"")), "cbf29ce484222325");
}

#[test]
fn the_iq_judge_grades_by_the_suites_checks() {
    let suite = include_str!("../../../evals/suites/iq.jsonl");
    let judge = Iq { tasks: iq::read_suite(suite).unwrap() };
    assert_eq!(judge.id(), iq::verifier_hash());
    let line = suite.lines().next().unwrap();
    let t = iq::read_task(line).unwrap();
    assert!(judge.verify_task(line).is_ok(), "{:?}", judge.verify_task(line));
    let good = judge.grade(&t.id, &t.ask, &t.reference);
    assert!(good.pass && good.stage == "ok", "{good:?}");
    let bad = judge.grade(&t.id, &t.ask, "label \"hi\";");
    assert!(!bad.pass && bad.stage == "check", "{bad:?}");
    assert_eq!(judge.grade("no-such-task", "", &t.reference).stage, "harness");
    let bad_icon = t.reference.replace("// icon: ring 12 12 9 ", "// icon: ring 12 12 9 2 ");
    let g = judge.grade(&t.id, &t.ask, &bad_icon);
    assert!(!g.pass && g.stage == "icon", "an icon that does not draw never teaches: {g:?}");
}

#[test]
fn prompts_carry_the_bytes_studio_sends() {
    let tasks: Vec<Task> =
        include_str!("../../../evals/suites/iq.jsonl").lines().filter_map(Task::parse).collect();
    let text = run::prompts(&tasks, "all", &[]);
    assert_eq!(text.lines().count(), tasks.len());
    let first = Json::parse(text.lines().next().unwrap()).unwrap();
    let msg = |i: usize| first.get("messages").and_then(|m| m.at(i)).and_then(|m| m.get("content"));
    assert_eq!(msg(0).and_then(Json::text), Some(coder::prompt::system().as_str()));
    let user = coder::prompt::write(&tasks[0].ask);
    assert_eq!(msg(1).and_then(Json::text), Some(user.as_str()));
    assert!(text.lines().next().unwrap().contains(",\"max_tokens\":6144,\"temperature\":0.3,"));
}

/// The stand-in verifier: a task stands when it is well formed and its reference passes
/// `Judge::grade`; a program passes when the coder finds nothing wrong with it
/// (`coder::ai::fault`: compiles, runs clean on seeds 1 to 3, its icon draws). Its checks are
/// never run: it has no checker language.
struct Smoke;

impl Judge for Smoke {
    fn id(&self) -> String {
        hex16(fnv(b"teach::seam::Smoke 1: coder::ai::fault(program, \"\", 3)"))
    }

    fn verify_task(&self, line: &str) -> Result<String, String> {
        let t = Task::parse(line).ok_or("not a task line")?;
        well_formed(&t)?;
        let g = self.grade(&t.id, &t.ask, &t.reference);
        match g.pass {
            true => Ok(format!(
                "its ref runs clean ({} lines); its check is unrun",
                t.reference.lines().count()
            )),
            false => Err(format!("its ref fails at {}: {}", g.stage, g.why)),
        }
    }

    fn grade(&self, _task_id: &str, _ask: &str, program: &str) -> Grade {
        let seeds = coder::Knobs::default().seeds;
        let Some(f) = coder::ai::fault(program, "", seeds) else {
            return Grade { pass: true, stage: "ok".into(), code: 0, why: String::new() };
        };
        let stage = match (f.compiles, f.runs) {
            (false, _) => "compile",
            (true, false) => "smoke",
            (true, true) => "icon",
        };
        Grade { pass: false, stage: stage.into(), code: f.diag.code.unwrap_or(0), why: f.said }
    }
}

/// Whether `t` reads as a task should: an id of 3 to 64 of `a-z0-9-` that begins with its family
/// and `-`, a family of `a-z0-9-`, a tier from 1 to 6, and an ask, a check and a ref.
fn well_formed(t: &Task) -> Result<(), String> {
    let name =
        |s: &str| s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    let fam = [t.family.as_str(), "-"].concat();
    let why = match () {
        _ if t.family.is_empty() || !name(&t.family) => "a family of a-z, 0-9 and -",
        _ if !(3..=64).contains(&t.id.len()) || !name(&t.id) => "an id of 3 to 64 of a-z, 0-9, -",
        _ if !t.id.starts_with(&fam) || t.id.len() == fam.len() => "an id that begins family-",
        _ if !(1..=6).contains(&t.tier) => "a tier from 1 to 6",
        _ if t.ask.trim().is_empty() || t.check.trim().is_empty() => "an ask and a check",
        _ if t.reference.trim().is_empty() => "a ref",
        _ => return Ok(()),
    };
    Err(["a task needs ", why].concat())
}
