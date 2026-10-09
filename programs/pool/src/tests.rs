use crate::hub::fingerprint;
use crate::msg::{MAX, Msg};
use crate::pool::{Act, CHECK, Info, PROBE, PROBE_BYTES, Pool, dec, num};

/// A tab for the tests: its pool, its next pid, the lines fed to its workers not yet answered,
/// what its job's window heard, and the answer each worker gives (`None`: the right one).
struct Tab {
    pool: Pool,
    pid: u32,
    fed: Vec<(u32, String)>,
    told: Vec<(u32, u8, String)>,
    wrong: Option<u32>,
}

fn tab(name: &str, cores: u16) -> Tab {
    let me = Info { name: name.into(), kind: "computer".into(), cores, ..Info::default() };
    Tab { pool: Pool::new(me), pid: 100, fed: Vec::new(), told: Vec::new(), wrong: None }
}

/// A worker's answer to `line` (`<index> <input>`): fuel 10, the hash `h<input>`, the result.
fn answer(line: &str, wrong: bool) -> String {
    let (i, input) = num(line);
    let mut out = String::new();
    dec(&mut out, i.unwrap());
    let h = if wrong { "bad" } else { input };
    [out.as_str(), " 10 h", h, " r", input, "\n"].concat()
}

/// Carries out `t`'s acts: messages to `other` (each side calls the other link 1), workers
/// started and ready at once, lines fed kept for [`work`], posts to a model's server left for
/// [`serve`] (and what it says of itself, unasked here).
fn acts(t: &mut Tab, other: &mut Vec<Msg>) {
    let mut posts = Vec::new();
    while !t.pool.out.is_empty() {
        for a in std::mem::take(&mut t.pool.out) {
            match a {
                Act::Send(1, m) => other.push(m),
                Act::Send(..) => {}
                Act::Spawn(_, n) => {
                    for _ in 0..n {
                        t.pid += 1;
                        t.pool.spawned(Some(t.pid));
                        t.pool.output(t.pid, b"re", 0);
                        t.pool.output(t.pid, b"ady\r\n", 0);
                    }
                }
                Act::Feed(pid, line) => t.fed.push((pid, line)),
                Act::Stop(pids) => t.fed.retain(|f| !pids.contains(&f.0)),
                Act::Unlink(_) => {}
                Act::Done { index, node, out, .. } => t.told.push((index, node, out)),
                a @ Act::Post { .. } => posts.push(a),
                Act::Get(_) => {}
            }
        }
    }
    t.pool.out.extend(posts);
}

/// Every worker of `t` answers its line at `now`.
fn work(t: &mut Tab, now: u64) {
    for (pid, line) in std::mem::take(&mut t.fed) {
        let wrong = t.wrong == Some(pid);
        t.pool.output(pid, answer(&line, wrong).as_bytes(), now);
    }
}

/// Messages both ways until neither tab says more.
fn talk(a: &mut Tab, b: &mut Tab, now: u64) {
    let (mut to_a, mut to_b) = (Vec::new(), Vec::new());
    loop {
        acts(a, &mut to_b);
        acts(b, &mut to_a);
        if to_a.is_empty() && to_b.is_empty() {
            break;
        }
        to_b.drain(..).for_each(|m| b.pool.heard(1, m, now));
        to_a.drain(..).for_each(|m| a.pool.heard(1, m, now));
    }
}

fn lines(n: u32) -> Vec<String> {
    (0..n).map(|i| ["c", &i.to_string()].concat()).collect()
}

#[test]
fn every_message_comes_back_whole_and_nothing_malformed_does() {
    let all = [
        Msg::Hello {
            name: "Linux \u{b7} Firefox".into(),
            kind: "computer".into(),
            cores: 8,
            ram_mb: 0,
            quota_mb: 9,
            gpu: false,
            model: "qwen2.5-3b-instruct".into(),
            tok: 95,
            ctx: 4096,
            cpu1: 250,
            cpun: 900,
            mem: 8192,
            tested: 1_791_500_000,
        },
        Msg::Stats { workers: 8, busy: 3, chunks: 40, units: 1 << 40 },
        Msg::Ping { t: 7 },
        Msg::Pong { t: 7 },
        Msg::Probe { last: true, back: 9, fill: vec![0; 5] },
        Msg::Heard { bytes: 1, ms: 2 },
        Msg::Job { job: 3, name: "fractal".into() },
        Msg::Want { job: 3, n: 12 },
        Msg::Give { job: 3, chunks: vec![(0, "a".into()), (5, String::new())] },
        Msg::Done { job: 3, index: 5, out: "10 ab r".into() },
        Msg::End { job: 3, used: 9 },
        Msg::Ask { ask: 2, text: "Why is the sky blue?".into() },
        Msg::Words { ask: 2, text: "Rayleigh".into() },
        Msg::Answered { ask: 2, tok: 95, why: String::new() },
    ];
    for m in &all {
        let b = m.encode();
        assert_eq!(Msg::decode(&b).as_ref(), Some(m));
        assert_eq!(Msg::decode(&[&b[..], &[0]].concat()), None, "trailing");
        assert_eq!(Msg::decode(&b[..b.len() - 1]), None, "cut short");
    }
    // An unknown kind, a count past the bytes, a message over the cap.
    assert_eq!(Msg::decode(&[99]), None);
    let mut give = Msg::Give { job: 1, chunks: vec![] }.encode();
    give[5] = 200;
    assert_eq!(Msg::decode(&give), None);
    assert_eq!(
        Msg::decode(&Msg::Probe { last: false, back: 0, fill: vec![0; MAX] }.encode()),
        None
    );
}

#[test]
fn a_job_alone_runs_on_every_core_and_answers_each_chunk_once() {
    let mut a = tab("A", 3);
    a.pool.start(7, "fractal", lines(7), 0);
    talk(&mut a, &mut tab("-", 1), 0);
    // Three workers, each fed its chunk once it said it was ready.
    assert_eq!((a.pool.workers.len(), a.fed.len()), (3, 3));
    assert!(a.fed.iter().map(|f| f.1.as_str()).eq(["0 c0", "1 c1", "2 c2"]));
    for now in 1..4 {
        work(&mut a, now);
        talk(&mut a, &mut tab("-", 1), now);
    }
    let j = a.pool.job.as_ref().unwrap();
    assert_eq!((j.done, j.ended, a.pool.chunks, a.pool.units), (7, Some(3), 7, 70));
    assert!(a.told.iter().all(|t| t.1 == 0) && a.told.len() == 7);
    assert_eq!(a.told[0].2, "10 hc0 rc0");
    // A stray line is no answer; idle workers end after a while.
    a.pool.output(101, b"hello\n", 4);
    a.pool.tick(40_000);
    assert!(matches!(a.pool.out.last(), Some(Act::Stop(p)) if p.len() == 3));
}

#[test]
fn two_tabs_share_a_job_steal_its_tail_and_check_each_others_answers() {
    let (mut a, mut b) = (tab("A", 2), tab("B", 2));
    a.pool.linked(1, false, 0);
    b.pool.linked(1, true, 0);
    talk(&mut a, &mut b, 0);
    assert_eq!(a.pool.peers[0].info.name, "B");
    let n = 2 * CHECK + 4;
    a.pool.start(7, "fractal", lines(n), 0);
    talk(&mut a, &mut b, 0);
    // B asked for its idle workers' worth and half as many again, and works on two of them.
    assert_eq!((b.fed.len(), a.fed.len()), (2, 2));
    let snap = a.pool.snap(0, "", "");
    assert_eq!(snap.job.as_ref().map(|j| (j.total, j.queued)), Some((n, n - 2 - 3)));
    // B's workers answer; A's sit on theirs. B keeps asking and takes the rest.
    for now in 1..20 {
        work(&mut b, now);
        talk(&mut a, &mut b, now);
    }
    let j = a.pool.job.as_ref().unwrap();
    assert_eq!(j.done, n - 2, "A's two are still out");
    // A answers: its two, then its idle workers replay B's every CHECK-th chunk (8 and 16).
    work(&mut a, 30);
    talk(&mut a, &mut b, 30);
    work(&mut a, 31);
    talk(&mut a, &mut b, 31);
    let j = a.pool.job.as_ref().unwrap();
    assert_eq!((j.done, j.checked, j.mismatched), (n, 2, 0));
    let snap = a.pool.snap(32, "", "");
    assert_eq!(snap.job.unwrap().per, [2, n - 2]);
    assert!(a.told.iter().filter(|t| t.1 == 1).count() == (n - 2) as usize);
    // B heard the end: it helps no more, and keeps a record of what it answered.
    let record = b.pool.snap(32, "", "").job.unwrap();
    assert_eq!((record.mine, record.done, record.used, record.queued), (false, n - 2, n - 2, 0));
    assert!(0 < record.busy && record.busy < record.ms, "busy to its last answer, not the end");
    b.pool.tick(33);
    assert!(!b.pool.out.iter().any(|a| matches!(a, Act::Send(_, Msg::Want { .. }))));
    b.pool.out.clear();

    // A new job: B's worker 0 answers wrong. A's workers steal B's tail once the queue is
    // empty; B answers first, so A's answers, second, check B's: one differs.
    a.pool.start(8, "fractal", lines(4), 40);
    b.wrong = Some(b.pool.workers[0].pid);
    talk(&mut a, &mut b, 40);
    work(&mut a, 41);
    talk(&mut a, &mut b, 41);
    let j = a.pool.job.as_ref().unwrap();
    assert!(j.steals > 0 && j.done == 2, "{j:?}");
    work(&mut b, 42);
    talk(&mut a, &mut b, 42);
    work(&mut a, 43);
    talk(&mut a, &mut b, 43);
    let j = a.pool.job.as_ref().unwrap();
    assert_eq!((j.done, j.mismatched), (4, 1));
}

#[test]
fn a_give_of_long_chunks_stays_under_the_message_cap() {
    let mut a = tab("A", 1);
    a.pool.linked(1, false, 0);
    // Half a message, then nearly a whole one: they go in two Gives, not one too long to decode.
    let sizes = [32_000, 63_000, 20_000, 20_000, 20_000, 1];
    a.pool.start(7, "iq", sizes.iter().map(|&n| "t".repeat(n)).collect(), 0);
    let job = a.pool.job.as_ref().unwrap().id;
    let mut per = Vec::new();
    for now in 1..6 {
        a.pool.out.clear();
        a.pool.heard(1, Msg::Want { job, n: 8 }, now);
        let Some(Act::Send(1, give)) = a.pool.out.pop() else { panic!("no Give") };
        let bytes = give.encode();
        assert!(bytes.len() <= MAX && Msg::decode(&bytes).as_ref() == Some(&give));
        let Msg::Give { chunks, .. } = give else { panic!("{give:?}") };
        per.push(chunks.len());
    }
    assert_eq!(per, [1, 1, 4, 0, 0]);
}

#[test]
fn a_gone_link_or_worker_gives_its_chunks_back_and_a_new_link_hears_the_job() {
    let (mut a, mut b) = (tab("A", 1), tab("B", 4));
    a.pool.linked(1, false, 0);
    b.pool.linked(1, false, 0);
    a.pool.start(7, "fractal", lines(10), 0);
    talk(&mut a, &mut b, 0);
    let out = a.pool.snap(0, "", "").job.unwrap().queued;
    assert_eq!(out, 10 - 1 - 6);
    a.pool.unlinked(1, 1);
    let j = a.pool.snap(1, "", "").job.unwrap();
    assert_eq!((j.queued, j.requeued), (9, 6));
    // A worker gone: its chunk too, to the queue's front.
    let pid = a.pool.workers[0].pid;
    a.pool.ended(pid, 2);
    let j = a.pool.snap(2, "", "").job.unwrap();
    assert_eq!((j.queued, j.requeued), (10, 7));
    // A link silent too long is closed and dropped.
    a.pool.linked(9, false, 2);
    a.pool.tick(2 + crate::pool::SILENT);
    assert!(a.pool.out.contains(&Act::Unlink(9)) && a.pool.peers.iter().all(|p| p.link != 9));
    a.pool.out.clear();
    // A link made now hears the job and helps.
    let mut c = tab("C", 1);
    a.pool.linked(1, false, 3);
    c.pool.linked(1, false, 3);
    talk(&mut a, &mut c, 3);
    assert_eq!(c.fed.len(), 1);
}

#[test]
fn measuring_a_link_times_a_run_each_way_and_pings_time_the_round_trip() {
    let (mut a, mut b) = (tab("A", 1), tab("B", 1));
    a.pool.linked(1, false, 0);
    b.pool.linked(1, false, 0);
    talk(&mut a, &mut b, 0);
    a.pool.measure();
    let runs = PROBE_BYTES / PROBE;
    let (mut to_a, mut to_b) = (Vec::new(), Vec::new());
    acts(&mut a, &mut to_b);
    assert_eq!(to_b.iter().filter(|m| matches!(m, Msg::Probe { .. })).count(), runs as usize);
    // B hears the run over 100 ms: 63 messages after the first.
    let n = to_b.len();
    for (k, m) in to_b.drain(..).enumerate() {
        b.pool.heard(1, m, 1000 + (k as u64 * 100 / (n as u64 - 1)));
    }
    acts(&mut b, &mut to_a);
    let want = u64::from(PROBE_BYTES - PROBE) * 1000 / 100;
    assert_eq!(u64::from(b.pool.peers[0].down), want);
    // A hears what B measured, then B's run back, over 50 ms.
    let n = to_a.len();
    for (k, m) in to_a.drain(..).enumerate() {
        a.pool.heard(1, m, 2000 + (k as u64 * 50 / (n as u64 - 2).max(1)));
    }
    assert_eq!(u64::from(a.pool.peers[0].up), want);
    assert!(a.pool.peers[0].down > 0);
    // A ping is answered as it came; its round trip is the clock's difference.
    a.pool.tick(5000);
    acts(&mut a, &mut to_b);
    to_b.drain(..).for_each(|m| b.pool.heard(1, m, 5010));
    acts(&mut b, &mut to_a);
    to_a.drain(..).for_each(|m| a.pool.heard(1, m, 5023));
    assert_eq!(a.pool.peers[0].rtt, 23);
    assert_eq!(b.pool.peers[0].stats.0, 0, "A has no workers yet");
}

#[test]
fn a_description_gives_its_fingerprint() {
    let sdp = "v=0\r\na=group:BUNDLE 0\r\na=fingerprint:sha-256 AB:CD:EF\r\na=setup:actpass\r\n";
    assert_eq!(fingerprint(sdp), "sha-256 AB:CD:EF");
    assert_eq!(fingerprint("v=0\r\n"), "");
}

/// Frames as the desktop sends them, at `now`.
fn frame(op: u8, a: u32, b: u32, now: u32, data: &str) -> uiwire::relay::Frame {
    uiwire::relay::Frame { op, a, b, now, data: data.as_bytes().to_vec() }
}

/// What the hub asked of the desktop since the last call, as (op, a, data).
fn asked(h: &mut crate::Hub) -> Vec<(u8, u32, String)> {
    let (mut out, bytes) = (Vec::new(), std::mem::take(&mut h.out));
    let mut b = &bytes[..];
    while let Some((f, n)) = uiwire::relay::Frame::take(b) {
        out.push((f.op, f.a, String::from_utf8_lossy(&f.data).into_owned()));
        b = &b[n..];
    }
    out
}

#[test]
fn pairing_shows_a_code_polls_for_the_answer_and_pins_the_device_once_linked() {
    use uiwire::relay::{to_desk as d, to_pool as p};
    let agent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/141.0";
    let mut h = crate::Hub::default();
    h.frame(frame(p::INFO, 0, 0, 0, &["16 8192 1 0 \n", agent, "\nsha-256 OLD"].concat()));
    assert_eq!(
        (h.pool.me.name.as_str(), h.pool.me.cores, h.pool.me.gpu),
        ("Windows \u{b7} Chrome", 16, true)
    );
    // Show a code: an offer is made; once described it is posted; the code comes back.
    h.frame(frame(p::ASK, 9, 1, 10, ""));
    h.pump();
    let made = asked(&mut h);
    assert_eq!(made[0], (d::LINK, 1, String::new()));
    h.frame(frame(p::SIGNAL, 1, 0, 20, "v=0\r\na=fingerprint:sha-256 NEW\r\n"));
    assert_eq!(asked(&mut h)[0].0, d::POST);
    h.frame(frame(p::HTTP, 1, 200, 30, "K7000"));
    h.pump();
    let said = asked(&mut h);
    assert_eq!(h.note, "Enter this code on the other device");
    let wake = said.iter().find(|s| s.0 == d::WAKE).map(|s| s.1);
    assert_eq!(wake, Some(crate::hub::POLL as u32));
    // Polled: no answer yet (204), then the answer, which is accepted.
    h.frame(frame(p::TICK, 0, 0, 30 + crate::hub::POLL as u32, ""));
    h.pump();
    let polls = asked(&mut h);
    assert!(polls.iter().any(|s| s.0 == d::POST && s.2 == "/api/signal\npoll\nK7000"), "{polls:?}");
    h.frame(frame(p::HTTP, 2, 204, 1600, ""));
    h.frame(frame(p::HTTP, 3, 200, 1700, "v=0\r\na=fingerprint:sha-256 NEW\r\n"));
    // Post 3 was never asked for: dropped. The poll is asked again, and answered.
    h.frame(frame(p::TICK, 0, 0, 3100, ""));
    h.pump();
    let again = asked(&mut h);
    let post = again.iter().find(|s| s.0 == d::POST).map(|s| s.1).unwrap();
    h.frame(frame(p::HTTP, post, 200, 3200, "v=0\r\na=fingerprint:sha-256 NEW\r\n"));
    assert_eq!(asked(&mut h)[0], (d::ACCEPT, 1, "v=0\r\na=fingerprint:sha-256 NEW\r\n".into()));
    // Linked: the new key pinned with the old, the device in the pool, pairing done.
    h.frame(frame(p::LINKED, 1, 0, 3300, ""));
    let pins = asked(&mut h);
    assert!(pins.iter().any(|s| s.0 == d::PINS && s.2 == "sha-256 OLD\nsha-256 NEW"), "{pins:?}");
    assert!(h.note.is_empty() && h.pool.peers.len() == 1 && !h.pool.peers[0].known);
}

#[test]
fn a_job_asked_for_starts_workers_and_tells_its_window_each_answer() {
    use uiwire::relay::{to_desk as d, to_pool as p};
    let mut h = crate::Hub::default();
    h.frame(frame(p::INFO, 0, 0, 0, "2 0 0 0\nagent\n"));
    // A job of three chunks from process 9; and one whose program is no word, dropped.
    h.frame(frame(p::ASK, 9, 3, 1, "fractal\na\nb\nc"));
    h.frame(frame(p::ASK, 8, 3, 1, "../x\na"));
    h.pump();
    let spawn = asked(&mut h);
    assert!(spawn.iter().any(|s| *s == (d::SPAWN, 2, "fractal".into())), "{spawn:?}");
    for pid in [20, 21] {
        h.frame(frame(p::SPAWNED, pid, 0, 2, ""));
        h.frame(frame(p::OUT, pid, 0, 2, "ready\r\n"));
    }
    h.pump();
    let fed = asked(&mut h);
    assert!(
        fed.contains(&(d::FEED, 20, "0 a\n".into()))
            && fed.contains(&(d::FEED, 21, "1 b\n".into()))
    );
    h.frame(frame(p::OUT, 20, 0, 3, "0 5 h r\n"));
    h.pump();
    let told = asked(&mut h);
    let done = uiwire::Event::Done { index: 0, node: 0, out: "5 h r".into() }.encode();
    assert!(told.iter().any(|s| s.0 == d::TELL && s.1 == 9 && s.2.as_bytes() == done), "{told:?}");
    assert!(told.contains(&(d::FEED, 20, "2 c\n".into())));
}

/// A stream as llama-server wrote one (its ids cut): a role, then words, then the end with its
/// timings, then `[DONE]`.
const STREAM: &str = concat!(
    "data: {\"choices\":[{\"finish_reason\":null,\"index\":0,\"delta\":{\"role\":\"assistant\",",
    "\"content\":null}}],\"model\":\"C:\\\\models\\\\qwen2.5-coder-0.5b-instruct-q8_0.gguf\",",
    "\"object\":\"chat.completion.chunk\"}\n\n",
    "data: {\"choices\":[{\"finish_reason\":null,\"index\":0,\"delta\":{\"content\":\"Hello\"}}]}\n\n",
    "data: {\"choices\":[{\"finish_reason\":null,\"index\":0,\"delta\":{\"content\":\"! How\"}}]}\n\n",
    "data: {\"choices\":[{\"finish_reason\":\"length\",\"index\":0,\"delta\":{}}],",
    "\"timings\":{\"prompt_n\":6,\"predicted_n\":6,\"predicted_per_second\":56.130372144367314}}\n\n",
    "data: [DONE]\n\n",
);

#[test]
fn a_servers_stream_reads_in_pieces_of_any_size_and_only_a_local_one_is_shared() {
    use crate::model::{Reading, local, why};
    for size in [1, 7, 64, STREAM.len()] {
        let (mut r, mut said) = (Reading::default(), String::new());
        STREAM.as_bytes().chunks(size).for_each(|c| said.push_str(&r.feed(c)));
        said.push_str(&r.end());
        assert_eq!((said.as_str(), r.text.as_str()), ("Hello! How", "Hello! How"), "{size}");
        assert_eq!((r.model.as_str(), r.tok), ("qwen2.5-coder-0.5b-instruct-q8_0", 561));
        assert_eq!(why(200, &r), "");
    }
    // An error's body, not a stream; no answer at all; another status.
    let mut r = Reading::default();
    r.feed(b"{\"error\":{\"code\":400,\"message\":\"the request exceeds the context\"}}");
    r.end();
    assert_eq!(why(400, &r), "the request exceeds the context");
    assert!(why(0, &Reading::default()).contains("CORS"));
    assert_eq!(why(503, &Reading::default()), "HTTP 503");
    let near = [
        "http://localhost:8080",
        "http://127.0.0.1:8081/",
        "https://localhost",
        "http://[::1]:8080",
    ];
    near.iter().for_each(|u| assert!(local(u), "{u}"));
    let far = [
        "http://localhost.evil.example",
        "http://192.168.1.2:8080",
        "file:///x",
        "http://localhost:80 x",
    ];
    far.iter().chain(&["localhost:8080"]).for_each(|u| assert!(!local(u), "{u}"));
}

/// Carries out a tab's post to its model's server: its body, then how it ended.
fn serve(t: &mut Tab, stream: &str, status: u32) {
    let post = t.pool.out.iter().position(|a| matches!(a, Act::Post { .. }));
    let Some(Act::Post { url, body }) = post.map(|i| t.pool.out.remove(i)) else {
        panic!("no post")
    };
    assert!(url.ends_with("/v1/chat/completions") && body.contains("\"stream\":true"), "{body}");
    t.pool.part(stream.as_bytes());
    t.pool.posted(status);
}

#[test]
fn a_tab_shares_its_model_once_checked_and_a_linked_tab_asks_it_and_hears_it_write() {
    let (mut a, mut b) = (tab("A", 2), tab("B", 2));
    a.pool.linked(1, false, 0);
    b.pool.linked(1, true, 0);
    // B shares its server: checked first, then said in its Hello.
    b.pool.serve("http://localhost:8080/");
    assert_eq!(b.pool.shared.note, "Reaching http://localhost:8080");
    serve(&mut b, STREAM, 200);
    assert!(b.pool.shared.note.is_empty() && b.pool.me.tok == 561);
    talk(&mut a, &mut b, 1);
    let model = a.pool.peers[0].info.model.clone();
    assert_eq!(model, "qwen2.5-coder-0.5b-instruct-q8_0");
    let snap = a.pool.snap(1, "", "");
    assert_eq!((snap.devices[1].model.as_str(), snap.devices[1].tok), (model.as_str(), 561));
    // A asks the pool's model: B's writes, a piece at a time, back to A.
    a.pool.question("Say hi.");
    talk(&mut a, &mut b, 2);
    let (head, tail) = STREAM.split_at(STREAM.find("! How").unwrap());
    let post = b.pool.out.iter().position(|x| matches!(x, Act::Post { .. })).unwrap();
    b.pool.out.remove(post);
    b.pool.part(head.as_bytes());
    talk(&mut a, &mut b, 3);
    let answer = &a.pool.asking.as_ref().unwrap().answer;
    assert_eq!((answer.text.as_str(), answer.by.as_str(), answer.done), ("Hello", "B", false));
    assert!(a.pool.answering());
    // Another tab's question while B writes: refused, and said so.
    b.pool.write(2, 1, "Me too?");
    let busy = |a: Option<&Act>| matches!(a, Some(Act::Send(2, Msg::Answered { why, .. })) if why.contains("busy"));
    assert!(busy(b.pool.out.last()));
    b.pool.out.pop();
    b.pool.part(tail.as_bytes());
    b.pool.posted(200);
    talk(&mut a, &mut b, 4);
    let answer = a.pool.asking.as_ref().unwrap().answer.clone();
    let got = (answer.text.as_str(), answer.done, answer.tok, answer.why.as_str());
    assert_eq!(got, ("Hello! How", true, 561, ""));
    assert_eq!(a.pool.snap(4, "", "").answer, Some(answer));
    // Asked again, B goes before answering: the answer says so.
    a.pool.question("And again?");
    talk(&mut a, &mut b, 5);
    a.pool.unlinked(1, 6);
    let answer = &a.pool.asking.as_ref().unwrap().answer;
    assert!(answer.done && answer.why.contains("left"), "{answer:?}");
    // B finishes it unheard: an end that comes late changes nothing.
    serve(&mut b, STREAM, 200);
    a.pool.finished(1, a.pool.last_ask, 561, String::new());
    assert!(a.pool.asking.as_ref().unwrap().answer.why.contains("left"));
    // B stops sharing while it writes: the answer still comes whole; new questions are refused.
    b.pool.out.clear();
    b.pool.unlinked(1, 6);
    a.pool.linked(1, false, 6);
    b.pool.linked(1, true, 6);
    talk(&mut a, &mut b, 6);
    a.pool.question("Once more?");
    talk(&mut a, &mut b, 7);
    b.pool.serve("");
    talk(&mut a, &mut b, 8);
    assert!(a.pool.peers[0].info.model.is_empty());
    serve(&mut b, STREAM, 200);
    talk(&mut a, &mut b, 9);
    let answer = &a.pool.asking.as_ref().unwrap().answer;
    assert_eq!((answer.text.as_str(), answer.done, answer.why.as_str()), ("Hello! How", true, ""));
    b.pool.write(1, 99, "And now?");
    let refused = |x: Option<&Act>| matches!(x, Some(Act::Send(1, Msg::Answered { why, .. })) if why.contains("no model"));
    assert!(refused(b.pool.out.last()));
    b.pool.out.clear();
    // Shared anew while it writes: the new server is checked once the answer ends.
    b.pool.serve("http://localhost:8080");
    serve(&mut b, STREAM, 200);
    talk(&mut a, &mut b, 10);
    a.pool.question("Last one?");
    talk(&mut a, &mut b, 11);
    b.pool.serve("http://localhost:8081");
    let posts = |t: &Tab| t.pool.out.iter().filter(|x| matches!(x, Act::Post { .. })).count();
    assert_eq!(posts(&b), 1, "the answer's post alone");
    serve(&mut b, STREAM, 200);
    let check = b.pool.out.iter().find(|x| matches!(x, Act::Post { .. }));
    assert!(
        matches!(check, Some(Act::Post { url, .. }) if url.starts_with("http://localhost:8081"))
    );
    serve(&mut b, STREAM, 200);
    assert_eq!((b.pool.shared.url.as_str(), b.pool.me.tok), ("http://localhost:8081", 561));
    talk(&mut a, &mut b, 12);
    // B goes quiet mid-answer: A gives up after QUIET, keeping what came.
    a.pool.question("Quiet?");
    talk(&mut a, &mut b, 13);
    let post = b.pool.out.iter().position(|x| matches!(x, Act::Post { .. })).unwrap();
    b.pool.out.remove(post);
    b.pool.part(head.as_bytes());
    talk(&mut a, &mut b, 14);
    a.pool.tick(14 + crate::model::QUIET);
    let answer = &a.pool.asking.as_ref().unwrap().answer;
    let gave_up = answer.done && answer.text == "Hello" && answer.why.contains("stopped answering");
    assert!(gave_up, "{answer:?}");
    b.pool.posted(0);
    a.pool.unlinked(1, 30_000);
    // With no model in the pool, asking says so at once; a server elsewhere is never shared.
    a.pool.question("Anyone?");
    assert!(a.pool.asking.as_ref().unwrap().answer.why.contains("No device"));
    a.pool.serve("http://192.168.1.2:8080");
    assert!(a.pool.shared.note.starts_with("Only a server on this device"));
    assert!(a.pool.out.iter().all(|x| !matches!(x, Act::Post { .. })));
    // Asked while its own model is checked: asked once the check is done.
    let mut c = tab("C", 1);
    c.pool.serve("http://localhost:8080");
    c.pool.question("Early?");
    assert!(!c.pool.asking.as_ref().unwrap().answer.done);
    serve(&mut c, STREAM, 200);
    serve(&mut c, STREAM, 200);
    let answer = &c.pool.asking.as_ref().unwrap().answer;
    assert_eq!((answer.text.as_str(), answer.done, answer.by.as_str()), ("Hello! How", true, "C"));
    // A server that fails its check is not shared, and the Hello says no model.
    b.pool.serve("http://localhost:9");
    serve(&mut b, "", 0);
    let note = &b.pool.shared.note;
    assert!(note.starts_with("Could not share http://localhost:9: the server did not answer"));
    assert!(matches!(b.pool.hello(), Msg::Hello { model, .. } if model.is_empty()));
}

#[test]
fn the_model_shared_is_kept_with_the_pins_and_shared_again_when_the_pool_starts() {
    use uiwire::relay::{to_desk as d, to_pool as p};
    let mut h = crate::Hub::default();
    h.frame(frame(p::INFO, 0, 0, 0, "2 0 0 0\nagent\nsha-256 OLD"));
    h.frame(frame(p::ASK, 9, 5, 1, "http://localhost:8080"));
    h.pump();
    let said = asked(&mut h);
    let kept = (d::PINS, 0, "sha-256 OLD\nserve http://localhost:8080".into());
    assert!(said.contains(&kept), "{said:?}");
    let check = said.iter().find(|s| s.0 == d::POST).map(|s| (s.1, s.2.clone())).unwrap();
    let url = "http://localhost:8080/v1/chat/completions\n{\"messages\"";
    assert!(check.1.starts_with(url), "{check:?}");
    // Its answer comes in parts: the pool reads them as they come.
    h.frame(frame(p::PART, check.0, 0, 2, &STREAM[..40]));
    h.frame(frame(p::PART, check.0, 0, 2, &STREAM[40..]));
    h.frame(frame(p::HTTP, check.0, 200, 3, ""));
    assert_eq!(h.pool.me.model, "qwen2.5-coder-0.5b-instruct-q8_0");
    // Checked: what the server says of itself is asked for (a post with no body is a get), and
    // its context comes into the Hello.
    h.pump();
    let said = asked(&mut h);
    let props = said.iter().find(|s| s.0 == d::POST).map(|s| (s.1, s.2.clone())).unwrap();
    assert_eq!(props.1, "http://localhost:8080/props\n");
    let body = "{\"default_generation_settings\":{\"params\":{},\"n_ctx\":4096},\"total_slots\":1}";
    h.frame(frame(p::PART, props.0, 0, 4, &body[..20]));
    h.frame(frame(p::HTTP, props.0, 200, 4, &body[20..]));
    assert!(matches!(h.pool.hello(), Msg::Hello { ctx: 4096, .. }));
    assert_eq!(h.pool.snap(4, "", "").devices[0].ctx, 4096);
    // Another start: the pins as they were, the model shared again.
    let mut h = crate::Hub::default();
    h.frame(frame(p::INFO, 0, 0, 0, "2 0 0 0\nagent\nsha-256 OLD\nserve http://localhost:8080"));
    h.pump();
    let again = asked(&mut h);
    assert!(again.iter().any(|s| s.0 == d::POST && s.2.starts_with("http://localhost:8080/v1/")));
    assert_eq!(h.pool.shared.url, "http://localhost:8080");
    // Stopping forgets it.
    h.frame(frame(p::ASK, 9, 5, 4, ""));
    assert!(asked(&mut h).contains(&(d::PINS, 0, "sha-256 OLD".into())));
}

/// Every line fed to the test's workers since the last call, as (pid, line), and the acts.
fn fed(t: &mut Tab) -> (Vec<(u32, String)>, Vec<Act>) {
    let out = std::mem::take(&mut t.pool.out);
    let lines = out.iter().filter_map(|a| match a {
        Act::Feed(pid, l) => Some((*pid, l.clone())),
        _ => None,
    });
    (lines.collect(), out)
}

/// Worker `pid` answers `line` at `now`.
fn says(t: &mut Tab, pid: u32, line: &str, now: u64) {
    t.pool.output(pid, [line, "\n"].concat().as_bytes(), now);
}

#[test]
fn testing_this_device_times_the_cpu_keeps_it_then_takes_memory_until_a_step_slows() {
    use crate::test::{CPU_MIB, REST};
    let mut t = tab("A", 2);
    t.pool.clock = 1_791_500_000;
    t.pool.test(0);
    t.pool.test.as_mut().unwrap().memory = true;
    assert!(matches!(&t.pool.out[..], [Act::Spawn(p, 2)] if p == "gauge"));
    assert_eq!(t.pool.test.as_ref().unwrap().cap, 2048, "a computer whose browser says nothing");
    t.pool.out.clear();
    t.pool.spawned(Some(201));
    t.pool.spawned(Some(202));
    says(&mut t, 201, "ready", 1);
    says(&mut t, 202, "ready", 1);
    // Warm-up on each in turn, then one core alone.
    assert_eq!(fed(&mut t).0, [(201, "0 cpu 4".into())]);
    says(&mut t, 201, "0 4 h ok", 10);
    assert_eq!(fed(&mut t).0, [(202, "1 cpu 4".into())]);
    says(&mut t, 202, "1 4 h ok", 12);
    let (one, _) = fed(&mut t);
    assert_eq!(one, [(201, ["0 cpu ", &CPU_MIB.to_string()].concat())]);
    says(&mut t, 201, "0 16 h ok 100", 112);
    // 16 MiB in 100 ms, as the worker timed it: 160 MiB/s on one core. Then both at once, each
    // timing itself: 160 and 80 MiB/s, 240 in all.
    let (all, _) = fed(&mut t);
    assert_eq!(all.len(), 2);
    says(&mut t, 202, "1 16 h ok 100", 200);
    says(&mut t, 201, "0 16 h ok 200", 412);
    // The CPU's figures are this tab's, told and kept, before any memory is taken.
    let me = t.pool.me.measured;
    assert_eq!((me.cpu1, me.cpun, me.mem, me.tested), (160, 240, 0, 1_791_500_000));
    assert!(fed(&mut t).0.is_empty());
    assert_eq!(t.pool.due(), Some(412 + REST));
    t.pool.tick(411 + REST);
    assert!(fed(&mut t).0.is_empty());
    t.pool.tick(412 + REST);
    // Memory: steps on the first worker until it is full, then the next. The first steps' limit
    // is a second; then twice the usual step, at least 150 ms. The worker times each step.
    let mut now = 412 + REST;
    let mut take = |t: &mut Tab, answer: &str, ms: u64| {
        let (lines, _) = fed(t);
        let (pid, line) = lines.last().cloned().unwrap();
        now += ms;
        says(t, pid, &[&line[..1], " ", answer].concat(), now);
        (pid, line.get(2..).unwrap_or("").to_string())
    };
    assert_eq!(take(&mut t, "64 - ok 40", 41), (201, "ram 64 1000".into()));
    assert_eq!(take(&mut t, "128 - ok 40", 41), (201, "ram 64 1000".into()));
    assert_eq!(take(&mut t, "128 - no", 5), (201, "ram 64 1000".into()), "full: the next");
    assert_eq!(take(&mut t, "64 - ok 50", 51), (202, "ram 64 1000".into()));
    assert_eq!(take(&mut t, "128 - ok 60", 61), (202, "ram 64 1000".into()));
    // The usual step is 50 ms: the limit is 150.
    assert_eq!(take(&mut t, "192 - ok 100", 101), (202, "ram 64 150".into()));
    assert_eq!(take(&mut t, "200 - slow 151", 152), (202, "ram 64 150".into()));
    let test = t.pool.test.as_ref().unwrap();
    assert!(test.over && test.note.contains("a slow step"), "{}", test.note);
    // The slow step is not counted; every worker ends, its memory freed.
    assert_eq!(test.measured.mem, 320);
    assert!(t.pool.out.iter().any(|a| matches!(a, Act::Stop(p) if p.len() == 2)));
    let me = t.pool.me.measured;
    assert_eq!((me.cpu1, me.cpun, me.mem, me.tested), (160, 240, 320, 1_791_500_000));
    let hello = t.pool.hello();
    assert!(matches!(hello, Msg::Hello { cpu1: 160, mem: 320, tested: 1_791_500_000, .. }));
    let snap = t.pool.snap(now, "", "");
    assert_eq!((snap.devices[0].cpun, snap.devices[0].mem), (240, 320));
    assert!(snap.testing.starts_with("Tested: 320 MB of memory usable"), "{}", snap.testing);
    assert_eq!(t.pool.due(), None);
}

/// A one-core test through its CPU half (160 MiB/s) and its rest, on a computer whose browser
/// says `ram_mb`: the first memory step fed, at the time returned.
fn to_memory(ram_mb: u32) -> (Tab, u64) {
    let mut t = tab("A", 1);
    t.pool.me.ram_mb = ram_mb;
    t.pool.test(0);
    t.pool.test.as_mut().unwrap().memory = true;
    t.pool.spawned(Some(201));
    says(&mut t, 201, "ready", 1);
    says(&mut t, 201, "0 4 h ok 2", 2);
    says(&mut t, 201, "0 16 h ok 100", 100);
    says(&mut t, 201, "0 16 h ok 100", 200);
    let now = 200 + crate::test::REST;
    t.pool.tick(now);
    assert!(fed(&mut t).0.last().is_some_and(|l| l.1.starts_with("0 ram 64 ")));
    (t, now)
}

#[test]
fn a_fresh_worker_refused_memory_is_the_browsers_limit_and_a_job_waits_for_no_test() {
    let (mut t, now) = to_memory(0);
    says(&mut t, 201, "0 0 - no", now + 10);
    let test = t.pool.test.as_ref().unwrap();
    assert!(test.over && test.note.contains("browser's limit"), "{}", test.note);
    assert!(test.note.starts_with("Tested the CPU; memory not measured"), "{}", test.note);
    assert_eq!((t.pool.me.measured.cpu1, t.pool.me.measured.mem), (160, 0));
    // A job runs: no test until it ends.
    let mut b = tab("B", 1);
    b.pool.start(7, "fractal", lines(3), 0);
    b.pool.test(1);
    assert!(b.pool.test.as_ref().unwrap().note.starts_with("Busy with a job"));
}

#[test]
fn memory_stops_at_the_cap_and_the_pools_own_timers_end_a_step_that_hangs() {
    use crate::test::{FIRST, GRACE, MOST, SMALL, TIME, cap};
    // A quarter of what the browser says, at most 2 GB, in whole steps; when it says nothing,
    // 2 GB on a computer, 512 MB on a phone or tablet.
    assert_eq!((cap(0, true), cap(0, false)), (MOST, SMALL));
    assert_eq!(
        [cap(32_768, true), cap(4096, false), cap(1000, true), cap(100, true)],
        [2048, 1024, 192, 64]
    );
    // At the cap: a gigabyte said, 256 MB taken.
    let (mut t, mut now) = to_memory(1024);
    for held in [64, 128, 192, 256] {
        now += 31;
        says(&mut t, 201, &["0 ", &held.to_string(), " - ok 30"].concat(), now);
    }
    let test = t.pool.test.as_ref().unwrap();
    let note = "Tested: 256 MB of memory usable, stopped by the test's cap (it takes no more)";
    assert_eq!((test.over, test.note.as_str(), t.pool.me.measured.mem), (true, note, 256));
    assert!(t.pool.out.iter().any(|a| matches!(a, Act::Stop(p) if p == &[201])));
    // A step that never answers: the pool's timer ends it, a second past its limit.
    let (mut t, now) = to_memory(0);
    assert_eq!(t.pool.due(), Some(now + FIRST + GRACE));
    t.pool.tick(now + FIRST + GRACE - 1);
    assert!(!t.pool.test.as_ref().unwrap().over);
    t.pool.tick(now + FIRST + GRACE);
    let test = t.pool.test.as_ref().unwrap();
    assert!(test.over && test.note.contains("did not answer in time"), "{}", test.note);
    assert!(fed(&mut t).1.iter().any(|a| matches!(a, Act::Stop(p) if p == &[201])));
    says(&mut t, 201, "0 64 - ok 2400", now + 2400);
    assert_eq!((t.pool.me.measured.cpu1, t.pool.me.measured.mem), (160, 0), "late: not counted");
    // Quick steps slowly carried: the memory half ends at its time.
    let (mut t, start) = to_memory(0);
    let mut now = start;
    while !t.pool.test.as_ref().unwrap().over {
        now += 1000;
        let held = t.pool.test.as_ref().unwrap().measured.mem + 64;
        says(&mut t, 201, &["0 ", &held.to_string(), " - ok 30"].concat(), now);
    }
    let test = t.pool.test.as_ref().unwrap();
    assert!(test.note.ends_with("stopped by the test's time"), "{}", test.note);
    assert_eq!((now - start, test.measured.mem), (TIME, 8 * 64));
}

#[test]
fn what_testing_measured_is_kept_with_the_pins_and_comes_back() {
    use uiwire::relay::{to_desk as d, to_pool as p};
    let mut h = crate::Hub::default();
    h.frame(frame(p::INFO, 0, 0, 0, "2 0 0 0\nagent\nsha-256 OLD\ntested 250 900 8192 1791500000"));
    let m = h.pool.me.measured;
    assert_eq!((m.cpu1, m.cpun, m.mem, m.tested), (250, 900, 8192, 1_791_500_000));
    assert_eq!(h.pins, ["sha-256 OLD"]);
    // A new test's figures are kept once it ends.
    h.pool.me.measured.tested = 1_791_600_000;
    h.pump();
    let kept = asked(&mut h);
    let line = "sha-256 OLD\ntested 250 900 8192 1791600000";
    assert!(kept.iter().any(|s| s.0 == d::PINS && s.2 == line), "{kept:?}");
}

#[test]
fn with_the_memory_half_off_a_test_measures_the_cpu_and_takes_no_memory() {
    let mut t = tab("A", 1);
    t.pool.clock = 7;
    t.pool.test(0);
    t.pool.test.as_mut().unwrap().memory = false;
    t.pool.spawned(Some(201));
    says(&mut t, 201, "ready", 1);
    says(&mut t, 201, "0 4 h ok 10", 2);
    says(&mut t, 201, "0 16 h ok 100", 120);
    says(&mut t, 201, "0 16 h ok 100", 240);
    let (lines, acts) = fed(&mut t);
    assert!(lines.iter().all(|l| !l.1.contains("ram")), "{lines:?}");
    assert!(acts.iter().any(|a| matches!(a, Act::Stop(p) if p == &[201])));
    let test = t.pool.test.as_ref().unwrap();
    assert!(test.over && test.note.starts_with("Tested the CPU"), "{}", test.note);
    let me = t.pool.me.measured;
    assert_eq!((me.cpu1, me.cpun, me.mem, me.tested), (160, 160, 0, 7));
}
