use coder::json::quote;
use vfs::VfsError;

use super::*;

/// The agent's world in a test: a VFS; the AI's replies scripted (an HTTP status and an SSE body
/// each, sent in two pieces split mid-line); the person's answers scripted (none left: no); and
/// what was sent, shown and asked kept. A job ends with status 0, `hi` on its console if it
/// writes there.
#[derive(Default)]
struct Fake {
    fs: Vfs,
    replies: Vec<(u16, String)>,
    pending: Vec<Heard>,
    sent: Vec<String>,
    shown: String,
    answers: Vec<Answer>,
    asked: Vec<String>,
    jobs: Vec<Vec<u8>>,
    ran: String,
}

fn why(e: VfsError) -> sh::Why {
    match e {
        VfsError::NotFound => sh::NOT_FOUND,
        VfsError::NotADir => sh::NOT_A_DIR,
        VfsError::IsADir => sh::IS_A_DIR,
        _ => "invalid path",
    }
}

impl sh::Sys for Fake {
    fn list(&mut self, path: &str) -> Result<Vec<sh::Entry>, sh::Why> {
        self.fs.list(path).map_err(why)
    }
    fn read(&mut self, path: &str) -> Result<Vec<u8>, sh::Why> {
        self.fs.read(path).map(<[u8]>::to_vec).map_err(why)
    }
    fn write(&mut self, path: &str, data: &[u8], append: bool) -> Result<(), sh::Why> {
        if append { self.fs.append(path, data) } else { self.fs.write(path, data) }.map_err(why)
    }
    fn mkdir(&mut self, path: &str, parents: bool) -> Result<(), sh::Why> {
        if parents { self.fs.mkdir_all(path) } else { self.fs.mkdir(path) }.map_err(why)
    }
    fn remove(&mut self, path: &str, all: bool) -> Result<(), sh::Why> {
        self.fs.remove(path, all).map_err(why)
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<(), sh::Why> {
        self.fs.rename(from, to).map_err(why)
    }
    fn kind(&mut self, path: &str) -> Option<bool> {
        self.fs.exists(path).then(|| self.fs.is_dir(path))
    }
    fn run(&mut self, shown: &str, job: &[u8]) -> Result<i32, u16> {
        self.ran += shown;
        if capture(job, "/x").is_some() {
            self.ran += "hi\n";
        }
        self.jobs.push(job.to_vec());
        Ok(0)
    }
}

impl World for Fake {
    fn ask(&mut self, body: &str) {
        assert!(Json::parse(body).is_some(), "a request is JSON: {body}");
        self.sent.push(body.into());
        let (status, sse) =
            if self.replies.is_empty() { (0, String::new()) } else { self.replies.remove(0) };
        let mid = (0..=sse.len() / 2).rev().find(|&i| sse.is_char_boundary(i)).unwrap_or(0);
        let (a, b) = sse.split_at(mid);
        let end = if status == 0 { "no more replies" } else { "" };
        self.pending =
            vec![Heard::Data(a.into()), Heard::Data(b.into()), Heard::End(status, end.into())];
    }
    fn heard(&mut self) -> Heard {
        self.pending.remove(0)
    }
    fn say(&mut self, text: &str) {
        self.shown += text;
    }
    fn confirm(&mut self, question: &str, always: bool) -> Answer {
        self.asked.push([question, if always { "" } else { " (each time)" }].concat());
        if self.answers.is_empty() { Answer::No } else { self.answers.remove(0) }
    }
    fn ran(&mut self) -> String {
        mem::take(&mut self.ran)
    }
}

/// A world whose home holds `~/notes/todo.txt` (`buy milk`) and `~/apps/`, its AI to give
/// `replies` (each 200).
fn fake(replies: &[String]) -> Fake {
    let mut fs = Vfs::new();
    let home = |p: &str| p.replace('~', Vfs::HOME);
    fs.mkdir_all(&home("~/notes")).and(fs.mkdir_all(&home("~/apps"))).unwrap();
    fs.write(&home("~/notes/todo.txt"), b"buy milk\nwalk\n").unwrap();
    let replies = replies.iter().map(|r| (200, r.clone())).collect();
    Fake { fs, replies, ..Fake::default() }
}

/// A chat-completion stream of `deltas`, ending for `finish`, with its usage.
fn sse(deltas: &[String], finish: &str) -> String {
    let head = r#"data: {"id":"t","choices":[{"index":0,"delta":"#;
    let mut out = String::new();
    for d in deltas {
        out += &[head, d, "}]}\n\n"].concat();
    }
    out += &[head, "{},\"finish_reason\":\"", finish, "\"}]}\n\n"].concat();
    out += "data: {\"id\":\"t\",\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":10}}\n\n";
    out + "data: [DONE]\n\n"
}

/// A reply in words.
fn says(text: &str) -> String {
    sse(&[["{\"content\":", &quote(text), "}"].concat()], "stop")
}

/// A reply calling each of `list` (a tool's name and its arguments' JSON).
fn calls(list: &[(&str, &str)]) -> String {
    let delta = |(i, (name, args)): (usize, &(&str, &str))| {
        let id = ["\"call_", &i.to_string(), "_", name, "\""].concat();
        let f = ["{\"name\":\"", name, "\",\"arguments\":", &quote(args), "}"].concat();
        let index = i.to_string();
        [
            "{\"tool_calls\":[{\"index\":",
            &index,
            ",\"id\":",
            &id,
            ",\"type\":\"function\",\"function\":",
            &f,
            "}]}",
        ]
        .concat()
    };
    sse(&list.iter().enumerate().map(delta).collect::<Vec<_>>(), "tool_calls")
}

fn call(name: &str, args: &str) -> String {
    calls(&[(name, args)])
}

/// The text of tool result `n` (from 0) in `a`'s conversation.
fn result(a: &Agent, n: usize) -> &str {
    let mut tools = a.history.iter().filter_map(|m| match m {
        Msg::Tool { text, .. } => Some(text.as_str()),
        _ => None,
    });
    tools.nth(n).expect("that many tool results")
}

fn file(w: &Fake, path: &str) -> String {
    String::from_utf8_lossy(w.fs.read(&path.replace('~', Vfs::HOME)).unwrap()).into_owned()
}

#[test]
fn a_task_reads_then_edits_with_a_yes_then_answers() {
    let path = r#"{"path":"~/notes/todo.txt"}"#;
    let edit = r#"{"path":"notes/todo.txt","old_text":"buy milk","new_text":"buy oat milk"}"#;
    let mut w = fake(&[call("read_file", path), call("edit_file", edit), says("Done: oat milk.")]);
    w.answers = vec![Answer::Yes];
    let mut a = Agent::new(Vfs::HOME, &mut w);
    assert!(a.task("make it oat milk", &mut w));
    assert_eq!(file(&w, "~/notes/todo.txt"), "buy oat milk\nwalk\n");
    assert_eq!(w.asked, ["edit ~/notes/todo.txt"], "a read asks nothing; an edit asks first");
    assert_eq!(result(&a, 0), "   1| buy milk\n   2| walk\n");
    assert!(
        result(&a, 1).starts_with("Edited ~/notes/todo.txt; it now reads:\n   1| buy oat milk\n")
    );
    // Each step sends what came before, the tools and the system prompt.
    assert_eq!(w.sent.len(), 3);
    assert!(w.sent[0].contains("\"tools\":[") && w.sent[0].contains("You are agent"));
    assert!(
        w.sent[1].contains(
            r#""role":"tool","tool_call_id":"call_0_read_file","content":"   1| buy milk"#
        )
    );
    assert!(!w.sent[0].contains("\"model\""), "the free AI's own model");
    for shown in
        ["\u{25cf} \x1b[m\x1b[1mread_file\x1b[m ~/notes/todo.txt", "- buy milk", "+ buy oat milk"]
    {
        assert!(w.shown.contains(shown), "{shown:?} in {:?}", w.shown);
    }
    assert!(
        w.shown.contains("Done: oat milk.\n")
            && w.shown.contains("3 steps \u{b7} 300 in \u{b7} 30 out")
    );
    assert_eq!(a.usage, (300, 30));
    assert!(!w.fs.exists(&learn::path()), "nothing failed: nothing to learn");
}

#[test]
fn a_no_changes_nothing_and_always_asks_once_a_session() {
    let write = |p: &str| ["{\"path\":\"", p, "\",\"content\":\"x\\n\"}"].concat();
    let (a, b, c) = (write("/tmp/a"), write("/tmp/b"), write("/tmp/c"));
    let replies =
        [call("write_file", &a), says("ok"), call("write_file", &b), call("write_file", &c)];
    let mut w = fake(&[&replies[..], &[says("ok")]].concat());
    w.answers = vec![Answer::No, Answer::Always];
    let mut a = Agent::new("/tmp", &mut w);
    assert!(a.task("write a", &mut w));
    assert!(!w.fs.exists("/tmp/a"));
    assert!(result(&a, 0).starts_with("The user declined: write /tmp/a (1 line)."));
    assert!(a.task("write b and c", &mut w));
    assert!(w.fs.exists("/tmp/b") && w.fs.exists("/tmp/c"));
    assert_eq!(
        w.asked,
        ["write /tmp/a (1 line)", "write /tmp/b (1 line)"],
        "always: c goes unasked"
    );
    // -y asks nothing at all.
    let mut w = fake(&[call("write_file", &write("/tmp/d")), says("ok")]);
    let mut a = Agent::new("/tmp", &mut w);
    a.auto = true;
    assert!(a.task("write d", &mut w) && w.fs.exists("/tmp/d") && w.asked.is_empty());
}

#[test]
fn the_shell_tool_is_the_os_shell_its_directory_stays_and_its_asks_reach_the_desktop() {
    let mut w = fake(&[
        call("shell", r#"{"command":"cd apps"}"#),
        calls(&[
            ("shell", r#"{"command":"ls ~/notes"}"#),
            ("shell", r#"{"command":"open studio"}"#),
        ]),
        calls(&[
            ("shell", r#"{"command":"hi there > out.txt"}"#),
            ("shell", r#"{"command":"hi | hi"}"#),
        ]),
        says("ok"),
    ]);
    w.fs.mkdir_all("/bin").and(w.fs.write("/bin/hi", b"#!wasm bin/toolbox.wasm\n")).unwrap();
    w.answers = vec![Answer::Yes, Answer::Yes, Answer::Yes];
    let mut a = Agent::new(Vfs::HOME, &mut w);
    assert!(a.task("look around", &mut w));
    assert_eq!(a.cwd, [Vfs::HOME, "/apps"].concat());
    assert_eq!(result(&a, 0), "(no output)\n(The working directory is now ~/apps)");
    assert_eq!(result(&a, 1), "todo.txt\n", "the shell's colors go");
    assert_eq!(result(&a, 2), "[asked the desktop: open;studio]\n");
    assert!(w.shown.contains("\x1b]1729;open;studio\x07"), "the Terminal sees the ask");
    // cd and ls only read; open, and programs, ask first.
    assert_eq!(w.asked, ["run open studio", "run hi there > out.txt (each time)", "run hi | hi"]);
    // A program's output is caught, unless it goes to a file already.
    assert_eq!((result(&a, 3), result(&a, 4)), ("(no output)", "hi\n"));
    let stdout = |j: &[u8]| capture(j, "/x").is_some();
    assert_eq!(w.jobs.iter().map(|j| stdout(j)).collect::<Vec<_>>(), [false, true]);
    // The next task says where it works.
    let mut w2 = fake(&[says("ok")]);
    a.task("and now?", &mut w2);
    assert!(w2.sent[0].contains("and now?\\u000a(The working directory: ~/apps)"));
}

#[test]
fn capture_sends_only_a_jobs_console_output_to_the_file() {
    let stages = [("/bin/hi".to_string(), vec!["hi".to_string()])];
    let job = |stdout: (u8, &str)| sh::job("/tmp", (2, ""), stdout, &stages, b"in").unwrap();
    assert_eq!(capture(&job((0, "")), "/tmp/o"), Some(job((1, "/tmp/o"))));
    assert_eq!(capture(&job((1, "/a")), "/tmp/o"), None, "a redirect stays");
    assert_eq!(capture(&job((4, "")), "/tmp/o"), None, "so does nothing");
    assert_eq!(capture(b"\x05\x00ab", "/tmp/o"), None, "not a job");
}

#[test]
fn asks_go_to_the_desktop_and_other_escapes_go() {
    let out = "\x1b[1;34mapps\x1b[m  a\r\n\x1b]1729;open;studio\x07\x1b]0;t\x1b\\x\n";
    let (asks, text) = super::asks(out, true);
    assert_eq!(asks, "\x1b]1729;open;studio\x07");
    assert_eq!(text, "apps  a\n[asked the desktop: open;studio]\nx\n");
    assert_eq!(super::asks(out, false).1, "apps  a\n[not sent: open;studio]\nx\n");
}

#[test]
fn reads_come_numbered_in_parts_and_edits_take_copied_numbers() {
    let long: String = (1..=450).map(|n| ["line ", &n.to_string(), "\n"].concat()).collect();
    let numbered = r#"{"path":"~/notes/todo.txt","old_text":"   2| walk","new_text":"run"}"#;
    let mut w = fake(&[
        call("read_file", r#"{"path":"/tmp/long"}"#),
        call("read_file", r#"{"path":"/tmp/long","offset":449,"limit":5}"#),
        call("edit_file", numbered),
        call("edit_file", r#"{"path":"~/notes/todo.txt","old_text":"nope","new_text":"x"}"#),
        says("ok"),
    ]);
    w.fs.write("/tmp/long", long.as_bytes()).unwrap();
    w.answers = vec![Answer::Yes];
    let mut a = Agent::new(Vfs::HOME, &mut w);
    a.learn = false;
    assert!(a.task("read", &mut w));
    assert!(
        result(&a, 0).starts_with("   1| line 1\n")
            && result(&a, 0).ends_with("(lines 1-400 of 450; read on with offset 401)")
    );
    assert_eq!(result(&a, 1), " 449| line 449\n 450| line 450\n(lines 449-450 of 450)");
    assert_eq!(file(&w, "~/notes/todo.txt"), "buy milk\nrun\n");
    assert!(result(&a, 3).starts_with("Error: old_text is not in ~/notes/todo.txt"));
}

#[test]
fn search_and_list_see_the_files_but_dot_folders_and_devices() {
    let mut w = fake(&[
        call("search", r#"{"text":"MILK","ignore_case":true}"#),
        calls(&[
            ("list_dir", r#"{"path":"notes"}"#),
            ("search", r#"{"text":"x","path":"/nowhere"}"#),
        ]),
        says("ok"),
    ]);
    w.fs.mkdir_all(&[Vfs::HOME, "/.agent"].concat()).unwrap();
    w.fs.write(&[Vfs::HOME, "/.agent/milk"].concat(), b"milk").unwrap();
    let mut a = Agent::new(Vfs::HOME, &mut w);
    a.learn = false;
    assert!(a.task("find milk", &mut w));
    assert_eq!(result(&a, 0), "~/notes/todo.txt:1: buy milk\n");
    assert_eq!(result(&a, 1), "todo.txt  14\n");
    assert_eq!(result(&a, 2), "Error: /nowhere: not found");
}

#[test]
fn check_app_says_ok_or_the_fault_and_a_failure_overcome_is_a_lesson() {
    let good = applang::SHOTS[1].1;
    let bad = good.replacen("state", "stat", 1);
    let write =
        |src: &str| ["{\"path\":\"~/apps/todo.app\",\"content\":", &quote(src), "}"].concat();
    let check = r#"{"path":"~/apps/todo.app"}"#;
    let lesson = "Run check_app after every change to an app, before saying it is done.";
    let mut w = fake(&[
        call("applang_guide", ""),
        call("write_file", &write(&bad)),
        call("check_app", check),
        call("write_file", &write(good)),
        call("check_app", check),
        says("Your todo app is ready."),
        says(&["- ", lesson].concat()),
    ]);
    let mut a = Agent::new(Vfs::HOME, &mut w);
    a.auto = true;
    assert!(a.task("make a todo app", &mut w));
    assert!(
        result(&a, 0).starts_with("applang: a small TOTAL language")
            && result(&a, 0).contains("\"make snake\":\n```app\n")
    );
    assert!(result(&a, 2).starts_with("Error: Your program did not compile: E0"));
    assert!(result(&a, 4).starts_with("ok: it compiles"));
    // The lesson's request tells the task as it went, its problems, and the lessons kept.
    let asked = Json::parse(&w.sent[6]).unwrap();
    let story = asked.get("messages").and_then(|m| m.at(1)?.get("content")?.text()).unwrap();
    assert!(story.starts_with("User: make a todo app\nCall: applang_guide"));
    assert!(
        story.contains("Problems met, in order:\n- check_app: Error: Your program did not compile")
    );
    assert!(story.ends_with("Lessons it already keeps:\n(none)"));
    assert!(!w.sent[6].contains("\"tools\""), "a lesson's request has no tools");
    assert_eq!(file(&w, &learn::path()), ["- ", lesson, "\n"].concat());
    assert!(w.shown.contains(&["\u{2726} learned: ", lesson].concat()));
    // The next session's system prompt holds it.
    let mut w2 = fake(&[says("hi")]);
    w2.fs = mem::take(&mut w.fs);
    let mut a = Agent::new(Vfs::HOME, &mut w2);
    a.task("hello", &mut w2);
    let kept = ["Lessons from your past sessions here (follow them):\\u000a- ", lesson].concat();
    assert!(w2.sent[0].contains(&kept));
}

#[test]
fn a_lesson_is_one_new_line_and_none_is_none() {
    let kept = "- Run check_app after every change.\n";
    assert_eq!(learn::lesson("NONE", kept), None);
    assert_eq!(learn::lesson("none: it was unclear", kept), None);
    assert_eq!(
        learn::lesson("\n- \"Quote paths with spaces in the shell.\"\nmore", kept),
        Some("Quote paths with spaces in the shell.".into())
    );
    assert_eq!(learn::lesson("run check_app after every change.", kept), None, "kept already");
    assert_eq!(learn::lesson("ok", kept), None, "too short to be one");
    assert!(learn::lesson(&"x".repeat(500), "").is_some_and(|l| l.len() <= 243));
}

#[test]
fn lessons_past_their_bounds_are_merged_when_the_merge_is_sound() {
    let many: String =
        (0..30).map(|i| ["- lesson number ", &i.to_string(), " is a rule\n"].concat()).collect();
    let merged = "- lesson one\n- lesson two\n";
    let mut w = fake(&[
        call("read_file", r#"{"path":"/nowhere"}"#),
        says("ok"),
        says("- A brand new lesson about reading files."),
        says(merged),
    ]);
    w.fs.mkdir_all(&[Vfs::HOME, "/.agent"].concat()).unwrap();
    w.fs.write(&learn::path(), many.as_bytes()).unwrap();
    let mut a = Agent::new(Vfs::HOME, &mut w);
    assert!(a.task("read nowhere", &mut w));
    assert_eq!(file(&w, &learn::path()), merged);
    assert_eq!(a.lessons, merged);
    assert!(w.shown.contains("merged the lessons into 2 lines"));
    // A merge that is not lessons keeps them as they were.
    let mut w = fake(&[
        call("read_file", r#"{"path":"/nowhere"}"#),
        says("ok"),
        says("- Another new lesson about paths."),
        says("Sure! Here they are."),
    ]);
    w.fs.mkdir_all(&[Vfs::HOME, "/.agent"].concat()).unwrap();
    w.fs.write(&learn::path(), many.as_bytes()).unwrap();
    let mut a = Agent::new(Vfs::HOME, &mut w);
    a.task("read nowhere", &mut w);
    assert!(file(&w, &learn::path()).starts_with(&many) && a.lessons.lines().count() == 31);
}

#[test]
fn a_reply_cut_off_mid_call_runs_nothing_and_is_told_to_write_less() {
    let cut = sse(&[r#"{"tool_calls":[{"index":0,"id":"c1","type":"function","function":{"name":"write_file","arguments":"{\"path\":\"/tmp/x\",\"content\":\"aaa"}}]}"#.into()], "length");
    let mut w = fake(&[cut, says("ok"), says("NONE")]);
    let mut a = Agent::new(Vfs::HOME, &mut w);
    assert!(a.task("write x", &mut w));
    assert!(!w.fs.exists("/tmp/x"));
    assert_eq!(a.history[1], Msg::Reply("(cut off)".into(), vec![]));
    assert!(
        matches!(&a.history[2], Msg::User(t) if t.starts_with("[agent] Your reply ran out of room"))
    );
    assert_eq!(w.sent.len(), 3, "the third asks for a lesson: NONE keeps none");
    assert!(!w.fs.exists(&learn::path()));
}

#[test]
fn a_call_past_8_kb_runs_nothing_and_says_why() {
    let big = ["{\"path\":\"/tmp/big\",\"content\":", &quote(&"y".repeat(9000)), "}"].concat();
    let mut w = fake(&[call("write_file", &big), says("ok")]);
    let mut a = Agent::new(Vfs::HOME, &mut w);
    (a.auto, a.learn) = (true, false);
    assert!(a.task("write big", &mut w));
    assert!(!w.fs.exists("/tmp/big"));
    assert!(
        result(&a, 0).starts_with("Error: write_file: the arguments were cut off (8 KB at most)")
    );
}

#[test]
fn an_ai_failure_ends_the_task_and_odd_calls_are_made_safe() {
    let mut w = fake(&[]);
    w.replies = vec![(429, "{\"error\":{\"message\":\"rate limited\"}}".into())];
    let mut a = Agent::new(Vfs::HOME, &mut w);
    assert!(!a.task("hi", &mut w));
    assert!(
        w.shown.contains(
            "\x1b[31mE0903 the free AI is busy, try again in a minute: rate limited\x1b[m"
        )
    );
    // An id and a name the free AI would refuse back are made ones it takes.
    let odd = sse(&[r#"{"tool_calls":[{"index":0,"id":"","type":"function","function":{"name":"read-file","arguments":"{}"}}]}"#.into()], "tool_calls");
    let mut w = fake(&[odd, says("ok")]);
    let mut a = Agent::new(Vfs::HOME, &mut w);
    a.learn = false;
    assert!(a.task("odd", &mut w));
    assert!(
        matches!(&a.history[1], Msg::Reply(_, c) if c[0].id == "call_a1" && c[0].name == "invalid")
    );
    assert_eq!(result(&a, 0), "Error: there is no tool invalid");
}

#[test]
fn requests_fold_then_drop_to_fit_the_free_ai() {
    let mut w = fake(&[]);
    let mut a = Agent::new(Vfs::HOME, &mut w);
    let big = "x".repeat(9000);
    let call = |n: u32| Call {
        id: ["c", &n.to_string()].concat(),
        name: "read_file".into(),
        args: "{}".into(),
        cut: false,
    };
    // An old task of big results, then this one of many steps.
    a.history.push(Msg::User("old".into()));
    for n in 0..6 {
        a.history.push(Msg::Reply(String::new(), vec![call(n)]));
        a.history.push(Msg::Tool {
            id: ["c", &n.to_string()].concat(),
            text: [&big, "\nmore"].concat(),
        });
    }
    a.history.push(Msg::Reply("done".into(), vec![]));
    a.start = a.history.len();
    a.history.push(Msg::User("new".into()));
    for n in 10..45 {
        a.history.push(Msg::Reply(String::new(), vec![call(n)]));
        a.history.push(Msg::Tool { id: ["c", &n.to_string()].concat(), text: "small".into() });
    }
    let long = Call { args: ["{\"content\":", &quote(&big), "}"].concat(), ..call(40) };
    a.history.push(Msg::Reply(String::new(), vec![long]));
    a.history.push(Msg::Tool { id: "c40".into(), text: big.clone() });
    let body = a.body().unwrap();
    assert!(
        body.len() <= MAX_BODY && a.history.len() < MAX_MESSAGES,
        "{} {}",
        body.len(),
        a.history.len()
    );
    assert!(Json::parse(&body).is_some());
    // The old task went whole; this one keeps its prompt (told), and its latest step whole.
    assert_eq!(a.start, 0);
    assert!(matches!(&a.history[0], Msg::User(p) if p.starts_with("new") && p.ends_with(DROPPED)));
    assert!(matches!(a.history.last(), Some(Msg::Tool { text, .. }) if *text == big));
    // Folded results keep their first line.
    let mut m = Msg::Tool { id: "c".into(), text: [&big, "\nmore"].concat() };
    assert!(fold(&mut m) && !fold(&mut m));
    assert!(matches!(&m, Msg::Tool { text, .. } if text.len() < 200 && text.ends_with(FOLD)));
    let mut r = Msg::Reply(
        String::new(),
        vec![Call {
            args: ["{\"path\":\"a\",\"content\":", &quote(&big), "}"].concat(),
            ..call(1)
        }],
    );
    assert!(fold(&mut r));
    assert!(
        matches!(&r, Msg::Reply(_, c) if c[0].args.starts_with("{\"path\":\"a\",\"content\":\"xxx") && c[0].args.ends_with("[9000 bytes]\"}"))
    );
}

#[test]
fn tokens_read_short() {
    assert_eq!([tokens(950), tokens(12_345), tokens(250_000)], ["950", "12.3k", "250k"]);
    assert_eq!(quoted("a \"b\" \\c"), "\"a \\\"b\\\" \\\\c\"");
}

#[test]
fn only_a_lone_read_runs_unasked_and_what_may_destroy_asks_each_time() {
    for line in ["ls -a ~/apps", "cat \"a;b\" notes/*.txt", "cd ..", "echo hi"] {
        assert!(tools::reads(line) && !tools::risky(line), "{line}");
    }
    // Joined, piped, redirected, a device or no read at all: asked.
    let asked = ["cat a; rm -r ~", "ls && mv a b", "echo rm | sh", "echo x > f", "cat /dev/tty"];
    for line in asked.iter().chain(&["open studio", "hi < f", "cat 'a"]) {
        assert!(!tools::reads(line), "{line}");
    }
    // As the shell reads it: quotes, escapes and paths hide no rm, and a pattern could be any.
    let risky = ["ls; \\r\"m\" x", "/bin/sh -c x", "a || mv a b", "echo x > f", "r? -r ~"];
    assert!(risky.iter().chain(&asked[..4]).all(|l| tools::risky(l)));
    assert!(
        !["echo x >> f", "open studio", "hi | hi", "mkdir -p a"].iter().any(|l| tools::risky(l))
    );
    // Always covers commands, but never one that may destroy; -y covers all.
    let mut w = fake(&[
        calls(&[("shell", r#"{"command":"mkdir a"}"#), ("shell", r#"{"command":"mkdir b"}"#)]),
        calls(&[("shell", r#"{"command":"rm -r a"}"#), ("shell", r#"{"command":"rm -r b"}"#)]),
        says("ok"),
    ]);
    w.answers = vec![Answer::Always, Answer::Always, Answer::No];
    let mut a = Agent::new("/tmp", &mut w);
    a.learn = false;
    assert!(a.task("tidy", &mut w));
    assert_eq!(w.asked, ["run mkdir a", "run rm -r a (each time)", "run rm -r b (each time)"]);
    assert!(!w.fs.exists("/tmp/a") && w.fs.exists("/tmp/b"));
    assert!(result(&a, 3).starts_with("The user declined: run rm -r b."));
}

#[test]
fn what_the_model_writes_shows_as_text_and_only_a_line_let_run_asks_the_desktop() {
    let esc = "\u{1b}]1729;open;studio\u{7}";
    let mut w = fake(&[
        calls(&[
            ("shell", &["{\"command\":", &quote(&["ls ", esc].concat()), "}"].concat()),
            ("write_file", &["{\"path\":\"/tmp/x\",\"content\":", &quote(esc), "}"].concat()),
            ("read_file", r#"{"path":"/dev/events"}"#),
            ("write_file", r#"{"path":"/dev/job","content":"x"}"#),
        ]),
        says(&["Done", esc, "\r\n"].concat()),
    ]);
    let mut a = Agent::new("/tmp", &mut w);
    (a.auto, a.learn) = (true, false);
    assert!(a.task("try", &mut w));
    assert!(!w.shown.contains('\u{7}'), "no escape of the model's reaches the console");
    assert!(w.shown.contains("ls ^[]1729;open;studio^G") && w.shown.contains("Done^[]1729"));
    assert!(w.shown.contains("+ ^[]1729;open;studio^G"));
    assert!(result(&a, 0).starts_with("Error: one command line at a time"));
    assert!(result(&a, 2).starts_with("Error: /dev/events: a device"));
    assert!(result(&a, 3).starts_with("Error: /dev/job: a device"));
    // A file whose name asks: listing it runs unasked, so its ask goes nowhere.
    let mut w = fake(&[call("shell", r#"{"command":"ls"}"#), says("ok")]);
    w.fs.write(&["/tmp/a", esc].concat(), b"").unwrap();
    let mut a = Agent::new("/tmp", &mut w);
    a.learn = false;
    assert!(a.task("look", &mut w) && w.asked.is_empty());
    assert!(result(&a, 0).contains("[not sent: open;studio]") && !w.shown.contains(esc));
}
