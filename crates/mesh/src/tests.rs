use kernel::wire::{self, Msg};
use kernel::{Effect as K, Kernel};
use platform::{Ctl, Effect, Event};
use uiwire::Request;
use uiwire::relay::{Frame, to_desk as d, to_pool as p};
use vfs::Vfs;

use crate::{FIRST, OWNER, Relay};

/// The frames in `b`.
fn frames(mut b: &[u8]) -> Vec<Frame> {
    let mut out = Vec::new();
    while let Some((f, n)) = Frame::take(b) {
        out.push(f);
        b = &b[n..];
    }
    out
}

/// The frames the pool reads now from its console (its read's reply, the last to it).
fn read(k: &mut Kernel, fs: &mut Vfs, pool: u32) -> Vec<Frame> {
    k.message(fs, pool, &Msg::ConsRead { max: 1 << 16 }.encode());
    let reply = k.take_effects().into_iter().rev().find_map(|e| match e {
        K::Reply { pid, data, .. } if pid == pool => Some(data),
        _ => None,
    });
    frames(&reply.unwrap_or_default())
}

fn say(out: &mut Vec<u8>, op: u8, a: u32, data: &[u8]) {
    Frame { op, a, data: data.into(), ..Frame::default() }.put(out);
}

#[test]
fn the_pool_starts_when_needed_hears_frames_once_ready_and_its_asks_are_done() {
    let (mut k, mut fs, mut ctl, mut r) =
        (Kernel::new(), Vfs::new(), Ctl::default(), Relay::default());
    k.set_isolated(true);
    fs.mkdir_all("/bin").unwrap();
    ["pool", "fractal"].iter().for_each(|n| kernel::install(&mut fs, n, n).unwrap());
    r.load("me");
    assert_eq!(r.pump(&mut ctl, &mut k, &fs, Some(4)), None);
    assert!(k.procs().is_empty(), "nothing asked, nothing runs");
    // Asked: the pool starts, owned by no window, with nothing for it until it is ready.
    r.ask(9, &Request::Pair { code: String::new() }, 0.0);
    r.pump(&mut ctl, &mut k, &fs, Some(4));
    let (pool, ..) = k.procs()[0].clone();
    assert!(k.take_effects().iter().any(|e| matches!(e, K::Spawn { pid, .. } if *pid == pool)));
    assert!(ctl.effects().contains(&Effect::Estimate));
    k.message(&mut fs, pool, &[wire::READY, wire::VERSION]);
    k.message(&mut fs, pool, &Msg::ConsMode { bits: wire::MODE_RAW }.encode());
    // Ready, it asks for a link, a post, two workers and a wake.
    let mut out = Vec::new();
    say(&mut out, d::READY, 0, &[]);
    say(&mut out, d::LINK, 1, &[]);
    say(&mut out, d::POST, 1, b"/api/signal\nhost\nv=0");
    say(&mut out, d::POST, 2, b"no URL line");
    say(&mut out, d::SPAWN, 2, b"fractal");
    say(&mut out, d::SPAWN, 1, b"../x");
    say(&mut out, d::WAKE, 1500, &[]);
    k.message(&mut fs, pool, &Msg::ConsWrite { data: &out }.encode());
    let mut ctl = Ctl::default();
    assert_eq!(r.pump(&mut ctl, &mut k, &fs, Some(4)), Some(1500));
    let fx = ctl.effects();
    assert!(fx.contains(&Effect::Link { id: 1, cert: "me.cert".into(), offer: None }));
    let posted = |e: &Effect| match e {
        Effect::Stream { id, url, body, .. } => Some((*id, url.clone(), body.clone())),
        _ => None,
    };
    let posts: Vec<_> = fx.iter().filter_map(posted).collect();
    assert_eq!(posts, [(FIRST + 1, "/api/signal".into(), b"host\nv=0".to_vec())]);
    assert!(!r.streams(FIRST + 2), "a post names its URL, or it is not made");
    let workers: Vec<u32> = k.procs().iter().map(|p| p.0).filter(|&w| w != pool).collect();
    assert_eq!(workers.len(), 2, "a name that is no word starts nothing");
    // What it was told: the device first, then the ask, the watcher, the workers.
    let told = read(&mut k, &mut fs, pool);
    let ops: Vec<u8> = told.iter().map(|f| f.op).collect();
    assert_eq!(&ops[..2], [p::INFO, p::ASK]);
    assert!(ops.contains(&p::WATCH) && told.iter().filter(|f| f.op == p::SPAWNED).count() == 3);
    assert!(told.iter().any(|f| f.op == p::SPAWNED && f.a == 0));
    // A post's answer as it comes, its end, a link's news, a worker's output: each goes on as
    // a frame.
    r.heard(Event::Chunk { id: FIRST + 1, data: b"K7".to_vec() }, 2.0);
    r.heard(Event::StreamEnd { id: FIRST + 1, status: 200, error: String::new() }, 2.0);
    r.heard(Event::LinkData { id: 1, data: vec![7] }, 2.0);
    assert!(!r.streams(FIRST + 1));
    let (w, mut out) = (workers[0], Vec::new());
    k.message(&mut fs, w, &[wire::READY, wire::VERSION]);
    k.message(&mut fs, w, &Msg::ConsWrite { data: b"ready\n" }.encode());
    say(&mut out, d::FEED, w, b"0 tile\n");
    say(&mut out, d::KILL, pool, &[]);
    k.message(&mut fs, pool, &Msg::ConsWrite { data: &out }.encode());
    r.pump(&mut ctl, &mut k, &fs, Some(4));
    let told = read(&mut k, &mut fs, pool);
    let at = |op| told.iter().position(|f| f.op == op);
    assert!(at(p::PART) < at(p::HTTP), "the body, then its end");
    let part = told.iter().find(|f| f.op == p::PART).map(|f| (f.a, f.data.clone()));
    let http = told.iter().find(|f| f.op == p::HTTP).map(|f| (f.a, f.b, f.data.clone()));
    assert_eq!((part, http), (Some((1, b"K7".to_vec())), Some((1, 200, vec![]))));
    assert!(told.iter().any(|f| f.op == p::DATA && f.data == [7]));
    assert!(told.iter().any(|f| f.op == p::OUT && f.a == w && f.data == b"ready\r\n"));
    // Only its own workers may be fed or ended: the pool still runs.
    assert!(k.runs(pool) && k.procs().iter().all(|p| p.0 == pool || workers.contains(&p.0)));
    let _ = OWNER;
}

#[test]
fn a_profile_with_devices_linked_for_good_starts_the_pool_at_once() {
    for (kept, starts) in
        [("sha-256 AB", false), ("sha-256 AB\nbond h 00 0\tA\tc\tsha-256 AB", true)]
    {
        let (mut k, mut fs, mut ctl, mut r) =
            (Kernel::new(), Vfs::new(), Ctl::default(), Relay::default());
        k.set_isolated(true);
        fs.mkdir_all("/bin").unwrap();
        kernel::install(&mut fs, "pool", "pool").unwrap();
        ctl.storage_set("me.pins", kept);
        r.load("me");
        r.pump(&mut ctl, &mut k, &fs, None);
        assert_eq!(!k.procs().is_empty(), starts, "{kept}");
        // Once: it is not looked for again until the next sign-in.
        r.pump(&mut ctl, &mut k, &fs, None);
        assert_eq!(k.procs().len(), usize::from(starts));
    }
}
