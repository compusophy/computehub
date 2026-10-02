use super::*;
use crate::apply;
use crate::tests::{desktop, run};
use platform::{App as _, Effect as Fx, Event};
use shell::Effect;

/// A context as a phone would give it.
fn phone() -> Context {
    let device = Device { agent: "Phone/1.0".into(), touch: true };
    let (theme, windows) = ("Mono".into(), "2 open".into());
    Context { device, screen: (390.0, 844.0, 3.0), theme, windows }
}

/// The reports `ctl` streamed, as (id, body).
fn streamed(ctl: &Ctl) -> Vec<(u32, String)> {
    let text = |body: &[u8]| String::from_utf8_lossy(body).into_owned();
    let posts = ctl.effects().iter().filter_map(|e| match e {
        Fx::Stream { id, url, body, .. } if url == URL => Some((*id, text(body))),
        _ => None,
    });
    posts.collect()
}

/// The sig of report `json`.
fn sig_of(json: &str) -> &str {
    json[json.rfind(r#""sig":""#).expect("a sig") + 7..].trim_end_matches("\"}")
}

#[test]
fn a_report_is_json_with_a_context_block_and_nothing_else() {
    ["ai 503 network", "tab\there and a line\nbreak", &"x".repeat(500)].into_iter().for_each(note);
    let all = notes(NOTES);
    let all: Vec<&str> = all.lines().collect();
    assert_eq!(all[all.len() - 2], "tab here and a line break", "control chars are spaces");
    assert_eq!(all.last().map(|n| n.len()), Some(120), "cut");
    let body = body("  It froze.\nTwice.  ", Some(&phone()));
    let block = [
        "It froze.\nTwice.\n\n```text\nbuild    ",
        BUILD,
        "\nagent    Phone/1.0\nscreen   390x844 @3\ntouch    yes\ntheme    Mono\nwindows  2 open\n```\n\n\
         Recent events, oldest first:\n```text\n",
    ]
    .concat();
    assert!(body.starts_with(&block), "{body}");
    assert!(body.contains("ai 503 network\n") && body.ends_with("```"), "{body}");
    // Without the context: the text and the build, no device, windows or events.
    let bare = super::body("Love it", None);
    assert_eq!(bare, ["Love it\n\n```text\nbuild    ", BUILD, "\n```"].concat());
    // Fractional ratios, two places at most.
    let mut c = phone();
    (c.screen.0, c.screen.2, c.device.touch) = (1280.4, 1.25, false);
    assert!(super::body("", Some(&c)).contains("screen   1280x844 @1.25\ntouch    no\n"));
    // Windows and the dock's favorites by app, never a file: Studio on one, a .app.
    let names = ["studio:/d/diary.txt", "files:~/a", "/d/x.app", "x.app", "/a:b.app", "about"];
    assert_eq!(names.map(app), ["studio", "files", "app", "app", "app", "about"]);
    Reports::default().pref(&mut Ctl::default(), "dock", "studio,/d/x.app,files");
    assert_eq!(notes(1), "pref dock studio,app,files\n");
    // JSON: quotes, backslashes, newlines and control chars escaped; the rest as is.
    let json = report("feedback", "Bug: \"it\" \\ broke", "a\nb\u{1}\u{2014}", "");
    let raw = r#"{"kind":"feedback","title":"Bug: \"it\" \\ broke","body":"a\nb\u0001—","sig":""}"#;
    assert_eq!(json, raw);
    // Titles: the first line with text, at most 80 chars; signatures: 8 hex digits, stable.
    assert_eq!(title("\n  \n  First line  \nsecond"), "First line");
    let long = title(&"wörd ".repeat(30));
    assert!(long.chars().count() <= 80 && long.ends_with('\u{2026}'), "{long}");
    assert_eq!(title(&"é".repeat(80)), "é".repeat(80));
    assert_eq!((sig("error", "x"), sig("error", "x").len()), (sig("error", "x"), 8));
    assert!(sig("error", "x") != sig("panic", "x") && sig("error", "x") != sig("error", "y"));
    // Huge feedback is cut so the report stays under the inbox's limit.
    let huge = super::body(&"é".repeat(40_000), Some(&phone()));
    assert!(huge.len() < 30 << 10, "{}", huge.len());
}

#[test]
fn reports_wait_in_the_outbox_until_the_inbox_takes_them() {
    let (mut r, mut ctl) = (Reports::default(), Ctl::default());
    // Boot: nothing waits; nothing goes.
    r.pump(&mut ctl, phone);
    assert!(streamed(&ctl).is_empty() && r.retell() && !r.status(Default::default()).held);
    // Feedback goes at the next pump, with its context and own id, kept until answered.
    r.feedback("idea", "Dark mode for the dock\nplease", true);
    r.pump(&mut ctl, phone);
    let sent = streamed(&ctl);
    let [(id, json)] = &sent[..] else { panic!("{sent:?}") };
    let start = r#"{"kind":"feedback","title":"Idea: Dark mode for the dock","body":"Dark mode"#;
    assert!(json.starts_with(start));
    assert!(json.contains("windows  2 open") && sig_of(json).len() == 16, "{json}");
    assert_eq!(ctl.storage_get(OUTBOX).as_deref(), Some(json.as_str()));
    // No inbox yet (503): it stays, held; the next report sends both, the first as it was.
    assert!(r.ended(&mut ctl, *id, 503));
    assert!(r.retell() && r.status(Default::default()).held);
    assert!(!r.ended(&mut ctl, 7, 200), "not a report's stream");
    let mut ctl = Ctl::default();
    r.feedback("bug", "It froze", false);
    r.pump(&mut ctl, phone);
    let again = streamed(&ctl);
    assert!(again.len() == 2 && again[0].1 == *json && sig_of(&again[1].1) != sig_of(json));
    assert!(again[1].1.contains(r#""title":"Bug: It froze""#) && !again[1].1.contains("agent"));
    // Taken (201): gone from the outbox, and no longer held.
    assert!(r.ended(&mut ctl, again[0].0, 201));
    assert!(r.outbox().len() == 1 && !r.status(Default::default()).held);
    // Refused for good (400): dropped.
    assert!(r.ended(&mut ctl, again[1].0, 400));
    assert!(r.outbox().is_empty() && ctl.storage_get(OUTBOX).as_deref() == Some(""));
    // A new page finds what waited and sends it at once; the outbox keeps the newest 20.
    let mut ctl = Ctl::default();
    let lines: Vec<String> = (0..25).map(|i| report("feedback", &i.to_string(), "b", "")).collect();
    ctl.storage_set(OUTBOX, &lines.join("\n"));
    let mut r = Reports::default();
    r.pump(&mut ctl, || unreachable!("nothing new to build"));
    assert_eq!(streamed(&ctl).len(), 25, "all of them, once");
    assert!(r.status(Default::default()).held, "until one gets through");
    (0..2).for_each(|_| r.feedback("love", "Beautiful", false));
    r.pump(&mut ctl, phone);
    assert_eq!(r.outbox().len(), KEEP);
    assert!(r.outbox()[0].contains(r#""title":"7""#) && r.outbox()[19].contains("Love: Beautiful"));
    assert_eq!(streamed(&ctl).len(), 27, "only the new ones are sent again");
    assert!(sig_of(&r.outbox()[18]) != sig_of(&r.outbox()[19]), "the same words, two reports");
}

#[test]
fn errors_are_reported_once_a_session_unless_reports_are_off() {
    let (mut r, mut ctl) = (Reports::default(), Ctl::default());
    r.pump(&mut ctl, phone);
    // A program that fails twice is one report; AI: 429 only noted, 503 and no answer reported.
    [7, 9].into_iter().for_each(|pid| r.proc_failed(pid, "/bin/hello"));
    for (status, error) in [(200, ""), (429, ""), (503, ""), (0, "network"), (0, "network")] {
        r.ai_ended(status, error);
    }
    r.pump(&mut ctl, phone);
    let titles: Vec<String> = streamed(&ctl).iter().map(|s| s.1.clone()).collect();
    let has = |t: &str| titles.iter().any(|j| j.contains(&["\"title\":\"", t, "\""].concat()));
    assert_eq!(titles.len(), 3, "{titles:?}");
    assert!(has("The program hello failed to load or run"));
    assert!(has("The AI request failed: HTTP 503") && has("The AI request got no answer"));
    let error = |j: &String| j.starts_with(r#"{"kind":"error""#) && !j.ends_with(r#""sig":""}"#);
    assert!(titles.iter().all(error));
    let recent = notes(6).lines().collect::<Vec<_>>().join(" | ");
    assert!(recent.ends_with("proc 7 hello failed | proc 9 hello failed | ai 429 | ai 503 | ai 0 network | ai 0 network"), "{recent}");
    // Unanswered, they wait; the switch, stored and told, drops them; typed feedback still goes.
    streamed(&ctl).iter().for_each(|s| _ = r.ended(&mut ctl, s.0, 0));
    assert!(r.outbox().len() == 3 && r.status(Default::default()).held);
    r.pref(&mut ctl, ui::REPORTS, "off");
    assert_eq!(ctl.storage_get(REPORTS).as_deref(), Some("off"));
    assert!(r.retell() && r.status(Default::default()).reports_off);
    assert!(r.outbox().is_empty() && ctl.storage_get(OUTBOX).as_deref() == Some(""));
    let mut ctl = Ctl::default();
    r.proc_failed(8, "studio");
    r.feedback("bug", "Saw an error", true);
    r.pump(&mut ctl, phone);
    let sent = streamed(&ctl);
    assert!(sent.iter().all(|s| !s.1.starts_with(r#"{"kind":"error""#)), "no error report");
    assert!(sent.iter().any(|s| s.1.contains("Saw an error")));
    assert!(notes(3).lines().any(|n| n == "proc 8 studio failed"), "still noted");
    // Off as stored, from the first pump of a new page: of what waited, only feedback goes.
    let mut ctl = Ctl::default();
    ctl.storage_set(REPORTS, "off");
    let love = report("feedback", "F", "b", "2");
    ctl.storage_set(OUTBOX, &[&report("error", "E", "b", "1"), "\n", &love].concat());
    let mut r = Reports::default();
    r.ai_ended(500, "");
    r.pump(&mut ctl, phone);
    r.ai_ended(502, "");
    r.pump(&mut ctl, phone);
    assert_eq!(streamed(&ctl), [(FIRST_ID | 1, love.clone())]);
    assert!(ctl.storage_get(OUTBOX) == Some(love) && r.status(Default::default()).reports_off);
}

#[test]
fn the_desktop_sends_feedback_and_reports_failures_and_tells_apps() {
    // A terminal opens a file in Editor.
    let mut desk = desktop(true);
    run(&mut desk, "edit diary-2026.txt");
    // Feedback an app asked for leaves at the flush after it, with what is open: apps, no files.
    let mut ctl = Ctl::default();
    let fx = Effect::Feedback { kind: "bug".into(), text: "Dock flickers".into(), context: true };
    apply(vec![fx], &mut ctl, (&desk.ai.clone(), &mut 0.0), &mut desk.report);
    // Of two timers asked for, the sooner stands (the page clock reads 0 here).
    let (mut timer, ai) = (Ctl::default(), desk.ai.clone());
    let wake = |ms| Effect::Kernel(ui::kernel::Effect::Wake { ms });
    apply([1500, 2000, 100].map(wake).into(), &mut timer, (&ai, &mut 0.0), &mut desk.report);
    assert_eq!(timer.effects(), [Fx::Wake(1500), Fx::Wake(100)]);
    desk.flush(&mut ctl, false);
    let sent = streamed(&ctl);
    assert_eq!(sent.len(), 1);
    assert!(sent[0].1.contains(r"windows  welcome, terminal, editor\n"), "{sent:?}");
    assert!(sent[0].1.contains("Bug: Dock flickers") && !sent[0].1.contains("diary"));
    // Its answer is the report's, not the AI's; a 503 holds it, and apps hear so.
    let mut ctl = Ctl::default();
    desk.event(Event::StreamEnd { id: sent[0].0, status: 503, error: String::new() }, &mut ctl);
    assert!(desk.report.outbox().len() == 1 && desk.report.status(Default::default()).held);
    // A worker that fails is noted with its program and reported.
    let mut ctl = Ctl::default();
    desk.event(Event::ProcError { pid: 5 }, &mut ctl);
    let sent = streamed(&ctl);
    assert!(sent.iter().any(|s| s.1.contains(r#""kind":"error""#)), "{sent:?}");
    assert!(notes(1).starts_with("proc 5"));
    // An AI request's failure is noted (its stream is not a report's); others' ends are not.
    desk.ai.ask(5, uiwire::Request::Ai { id: 1, body: "{}".into() });
    let mut ctl = Ctl::default();
    desk.flush(&mut ctl, false);
    let asked = |e: &Fx| matches!(e, Fx::Stream { id: 1, url, .. } if url == crate::ai::URL);
    assert!(ctl.effects().iter().any(asked), "the AI's first stream");
    let end = |id, status| Event::StreamEnd { id, status, error: String::new() };
    desk.event(end(1, 429), &mut Ctl::default());
    desk.event(end(77, 500), &mut Ctl::default());
    assert_eq!(notes(1), "ai 429\n");
}
