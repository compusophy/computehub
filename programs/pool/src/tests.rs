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
/// started and ready at once, lines fed kept for [`work`].
fn acts(t: &mut Tab, other: &mut Vec<Msg>) {
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
            }
        }
    }
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
    assert!(polls.iter().any(|s| s.0 == d::POST && s.2 == "poll\nK7000"), "{polls:?}");
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
