//! Real makes replayed: what the live free AI (GLM 5.3) answered, stored as its content and
//! thinking's length (not the half-megabyte SSE), fed back through the loop as it streamed. A
//! change to checks, matching or stop rules shows here as a different turn.

use coder::json::quote;
use coder::{Done, Knobs, Make, Out, Outcome, Task, Turn};

/// A reply as it streamed: `thought` chars of reasoning, then `content`, 40 chars a delta, then
/// how it finished.
fn sse(thought: usize, content: &str) -> Vec<u8> {
    let delta = |k: &str, t: &str| {
        format!("data: {{\"choices\":[{{\"delta\":{{\"{k}\":{}}}}}]}}\n\n", quote(t))
    };
    let mut out = delta("reasoning", &"x".repeat(thought));
    let chars: Vec<char> = content.chars().collect();
    out.extend(chars.chunks(40).map(|c| delta("content", &c.iter().collect::<String>())));
    out += "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
    out.into_bytes()
}

/// A make of `ask`, each request answered by the next of `replies` (thought chars, content), 50
/// ms a chunk of 512 bytes: the user messages asked and its end.
fn replay(ask: &str, replies: &[(usize, &str)]) -> (Vec<String>, Done) {
    let task = Task { ask: ask.into(), model: "zai/glm-5.3".into(), ..Task::default() };
    let (mut m, mut out) = Make::start(task, Knobs::default(), 0);
    let (mut asked, mut now, mut replies) = (Vec::new(), 0, replies.iter());
    loop {
        let body = match out {
            Out::Ask(body) => body,
            Out::Done(done) => return (asked, done),
            Out::Cancel => panic!("a cancel outside a reply"),
        };
        let user = body.rsplit("{\"role\":\"user\",\"content\":").next().unwrap();
        asked.push(user.to_string());
        let (thought, content) = replies.next().expect("a reply for each request");
        let mut cancelled = false;
        for chunk in sse(*thought, content).chunks(512) {
            now += 50;
            if let Some(o) = m.data(chunk, now) {
                assert!(matches!(o, Out::Cancel));
                cancelled = true;
                break;
            }
        }
        out = match cancelled {
            true => m.check(now).unwrap(),
            false => m.end(200, "", now).unwrap_or_else(|| m.check(now).unwrap()),
        };
    }
}

#[test]
fn tetris_a_stray_name_is_fixed_and_it_runs() {
    // v2runs/tetris/0: its first write used `stack`, which it never declared; the second reply
    // (to the fix) was the whole program again, fixed.
    let (one, two) = (include_str!("fixtures/tetris-1.txt"), include_str!("fixtures/tetris-2.txt"));
    let (asked, done) = replay("make a tetris game", &[(3074, one), (3485, two)]);
    assert!(asked[0].starts_with("\"Make: make a tetris game\""));
    let fix = &asked[1];
    assert!(
        fix.contains("Your program did not compile: E0302") && fix.contains("`stack`"),
        "{fix}"
    );
    assert!(fix.contains("Reply with edit blocks.") && fix.contains("  1| // Tetris: arrows move"));
    let turns: Vec<Turn> = done.receipt.turns.iter().map(|t| t.turn).collect();
    assert_eq!(
        (done.outcome, turns),
        (Outcome::Ready, vec![Turn::Write, Turn::Fix]),
        "{}",
        done.why
    );
    assert!(done.install && done.plan.starts_with("Tetris: arrows move and rotate"));
    // Both requests stopped at their block's end: estimated, about the 2.6k and 2.5k tokens out
    // the live replies counted (by the usage that came after).
    let out: Vec<u32> = done.receipt.turns.iter().map(|t| t.output).collect();
    assert!(done.receipt.est && out.iter().all(|o| (2000..3200).contains(o)), "{out:?}");
}

#[test]
fn snake_is_one_request() {
    let (asked, done) = replay("make snake", &[(3600, include_str!("fixtures/snake-1.txt"))]);
    assert_eq!(
        (asked.len(), done.outcome, done.said().split(' ').next()),
        (1, Outcome::Ready, Some("ready"))
    );
}
