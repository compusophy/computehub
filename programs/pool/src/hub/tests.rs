//! Two hubs linked for good: paired once, then, rebuilt from what each kept (a reload), meeting
//! again with no code, through a signal server and links simulated here.

use uiwire::relay::{Frame, to_desk as d, to_pool as p};

use super::bond::{self, Step};
use super::{Hub, fingerprint};

const AGENTS: [&str; 2] = [
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/141.0",
    "Mozilla/5.0 (X11; Linux x86_64; rv:146.0) Firefox/146.0",
];
const KEYS: [&str; 2] = ["sha-256 AA:AA", "sha-256 BB:BB"];

/// Two hubs and what lies between them: the server's codes and mailboxes as api/signal keeps
/// them, the links open (each end's hub and link), what each kept, every body posted, the clock.
struct Net {
    hubs: [Hub; 2],
    codes: Vec<(String, String, Option<String>)>,
    open: Vec<[(usize, u32); 2]>,
    kept: [String; 2],
    posted: Vec<String>,
    now: u32,
}

/// Hub `h`'s description for its link `l`: its key, and where it came from.
fn sdp(h: usize, l: u32) -> String {
    ["v=0\r\na=fingerprint:", KEYS[h], "\r\na=mid:", &h.to_string(), ".", &l.to_string(), "\r\n"]
        .concat()
}

/// Where a description came from: its hub and link.
fn from(sdp: &str) -> Option<(usize, u32)> {
    let mid = sdp.lines().find_map(|l| l.strip_prefix("a=mid:"))?;
    let (h, l) = mid.split_once('.')?;
    Some((h.parse().ok()?, l.parse().ok()?))
}

impl Net {
    /// Two tabs started with what they kept.
    fn new(kept: [&str; 2]) -> Net {
        let hubs = [0, 1].map(|h| {
            let mut hub = Hub::default();
            let info = ["8 0 0 0\n", AGENTS[h], "\n", kept[h]].concat();
            hub.frame(Frame { op: p::INFO, data: info.into_bytes(), ..Frame::default() });
            hub
        });
        let kept = kept.map(String::from);
        Net { hubs, codes: Vec::new(), open: Vec::new(), kept, posted: Vec::new(), now: 0 }
    }

    fn tell(&mut self, h: usize, op: u8, a: u32, b: u32, data: Vec<u8>) {
        self.hubs[h].frame(Frame { op, a, b, now: self.now, data });
    }

    /// The other end of hub `h`'s link `l`.
    fn other(&self, h: usize, l: u32) -> Option<(usize, u32)> {
        let ends = self.open.iter().find(|e| e.contains(&(h, l)))?;
        Some(if ends[0] == (h, l) { ends[1] } else { ends[0] })
    }

    /// api/signal's answer to `body`.
    fn signal(&mut self, body: &str) -> (u32, String) {
        self.posted.push(body.into());
        let (op, rest) = body.split_once('\n').unwrap_or((body, ""));
        let (code, sdp) = rest.split_once('\n').unwrap_or((rest, ""));
        let i = self.codes.iter().position(|c| c.0 == code);
        match (op, i) {
            ("host", _) => {
                let code = ["K", &self.codes.len().to_string()].concat();
                self.codes.push((code.clone(), rest.into(), None));
                (200, code)
            }
            ("meet", _) if code.len() == 24 => {
                self.codes.retain(|c| c.0 != code);
                self.codes.push((code.into(), sdp.into(), None));
                (200, String::new())
            }
            ("join", Some(i)) if self.codes[i].2.is_none() => (200, self.codes[i].1.clone()),
            ("answer", Some(i)) => {
                self.codes[i].2 = Some(sdp.into());
                (200, String::new())
            }
            ("poll", Some(i)) => match self.codes[i].2.clone() {
                Some(answer) => {
                    self.codes.remove(i);
                    (200, answer)
                }
                None => (204, String::new()),
            },
            _ => (404, "no such code".into()),
        }
    }

    /// Carries out what hub `h` asked of its page.
    fn serve(&mut self, h: usize) {
        let bytes = std::mem::take(&mut self.hubs[h].out);
        let mut b = &bytes[..];
        while let Some((f, n)) = Frame::take(b) {
            b = &b[n..];
            let text = String::from_utf8_lossy(&f.data).into_owned();
            match f.op {
                // An offer made, or an answer to the offer given.
                d::LINK => self.tell(h, p::SIGNAL, f.a, 0, sdp(h, f.a).into_bytes()),
                d::ACCEPT => {
                    let Some(there) = from(&text) else { continue };
                    self.open.push([(h, f.a), there]);
                    self.tell(h, p::LINKED, f.a, 0, Vec::new());
                    self.tell(there.0, p::LINKED, there.1, 0, Vec::new());
                }
                d::SEND => {
                    if let Some((h2, l2)) = self.other(h, f.a) {
                        self.tell(h2, p::DATA, l2, 0, f.data);
                    }
                }
                d::UNLINK => {
                    if let Some((h2, l2)) = self.other(h, f.a) {
                        self.open.retain(|e| !e.contains(&(h, f.a)));
                        self.tell(h2, p::UNLINKED, l2, 0, Vec::new());
                    }
                }
                d::POST => {
                    let body = text.split_once('\n').map_or("", |b| b.1).to_string();
                    let (status, answer) = self.signal(&body);
                    self.tell(h, p::HTTP, f.a, status, answer.into_bytes());
                }
                d::PINS => self.kept[h] = text,
                _ => {}
            }
        }
    }

    /// `ms` of both tabs running, a tick every 50.
    fn run(&mut self, ms: u32) {
        for _ in 0..ms / 50 {
            self.now += 50;
            for h in 0..2 {
                self.tell(h, p::TICK, 0, 0, Vec::new());
                self.hubs[h].pump();
                self.serve(h);
            }
        }
    }

    /// Whether the two are linked, each the other's one peer.
    fn linked(&self) -> bool {
        self.open.len() == 1 && self.hubs.iter().all(|h| h.pool.peers.len() == 1)
    }
}

/// Two tabs paired by a code, then left to make their bond.
fn paired() -> Net {
    let mut net = Net::new(["", ""]);
    net.tell(0, p::ASK, 9, 1, Vec::new());
    net.run(200);
    let code = net.hubs[0].pairing.as_ref().map(|p| p.code.clone()).unwrap();
    net.tell(1, p::ASK, 9, 1, code.into_bytes());
    net.run(3000);
    net
}

#[test]
fn paired_once_two_tabs_keep_a_bond_and_meet_again_after_a_reload_with_no_code() {
    let net = paired();
    assert!(net.linked());
    // Each kept the other's key and the same secret; the one that showed the code offers.
    let lines: Vec<&str> =
        net.kept.iter().map(|k| k.lines().find(|l| l.starts_with(bond::BOND)).unwrap()).collect();
    let (a, b) = (bond::Bond::parse(lines[0]).unwrap(), bond::Bond::parse(lines[1]).unwrap());
    assert_eq!((a.host, a.fp.as_str(), b.host, b.fp.as_str()), (true, KEYS[1], false, KEYS[0]));
    assert_eq!((a.secret.len(), a.secret == b.secret), (64, true));
    assert_eq!(
        (a.name.as_str(), b.name.as_str()),
        ("Linux \u{b7} Firefox", "Windows \u{b7} Chrome")
    );
    let snap = net.hubs[0].pool.snap(0, "", "");
    assert!(snap.bonds.is_empty(), "the hub adds them");
    // Both reload: each starts from what it kept, and nothing is asked of either.
    let kept = net.kept.clone();
    let mut net = Net::new([&kept[0], &kept[1]]);
    assert_eq!(net.hubs[0].bonds.len(), 1);
    net.run(6000);
    assert!(net.linked(), "{:?}", net.posted);
    assert!(net.hubs.iter().all(|h| h.bonds[0].step == Step::Open && h.pool.peers[0].known));
    // They met at their mailbox, never by a code.
    let mb = bond::mailbox(&a.secret);
    assert_eq!(mb.len(), 24);
    assert!(net.posted.iter().all(|p| p.contains(&mb)), "{:?}", net.posted);
    assert!(net.posted.iter().any(|p| p.starts_with("meet\n")));
}

#[test]
fn a_bonded_device_away_is_tried_less_often_shown_offline_and_met_when_it_returns() {
    let net = paired();
    let kept = net.kept.clone();
    // Only the first tab comes back: it offers, then polls, less often the longer it waits.
    let mut net = Net::new([&kept[0], ""]);
    net.run(60_000);
    let polls = net.posted.iter().filter(|p| p.starts_with("poll\n")).count();
    assert!((4..=12).contains(&polls), "{polls}: backing off to one in {} ms", bond::MOST);
    let b = &net.hubs[0].bonds[0];
    assert!(matches!(b.step, Step::Polling | Step::Offered) && b.wait == bond::MOST);
    assert_eq!(b.shown().state, uiwire::pool::OFFLINE);
    // The other comes back: they meet within a poll.
    net.hubs[1] = Net::new(["", &kept[1]]).hubs.into_iter().nth(1).unwrap();
    net.run(bond::MOST as u32 + 2000);
    assert!(net.linked(), "{:?}", net.posted.iter().rev().take(6).collect::<Vec<_>>());
    assert_eq!(net.hubs[0].bonds[0].shown().state, uiwire::pool::ONLINE);
}

#[test]
fn a_description_without_the_pinned_key_is_never_answered() {
    let net = paired();
    let kept = net.kept.clone();
    let mut net = Net::new(["", &kept[1]]);
    // Someone else's offer at the mailbox: the second tab asks for it and goes no further.
    let mb = bond::mailbox(&net.hubs[1].bonds[0].secret);
    let evil = "v=0\r\na=fingerprint:sha-256 EV:IL\r\na=mid:0.1\r\n";
    net.codes.push((mb.clone(), evil.into(), None));
    net.run(3000);
    assert!(net.posted.iter().any(|p| *p == ["join\n", &mb].concat()));
    assert!(net.posted.iter().all(|p| !p.starts_with("answer\n")), "{:?}", net.posted);
    assert!(net.open.is_empty() && fingerprint(evil) != KEYS[0]);
}

#[test]
fn unlinking_forgets_the_device_on_both_sides_and_a_link_gone_is_tried_again() {
    let mut net = paired();
    // The link drops: the bond is apart, last seen now, and tried again.
    let ends = net.open[0];
    net.serve(0);
    net.hubs[0].frame(Frame { op: p::UNLINKED, a: ends[0].1, now: net.now, ..Frame::default() });
    net.hubs[1].frame(Frame { op: p::UNLINKED, a: ends[1].1, now: net.now, ..Frame::default() });
    net.open.clear();
    assert!(net.hubs.iter().all(|h| h.bonds[0].step == Step::Idle && h.bonds[0].seen > 0));
    net.run(8000);
    assert!(net.linked());
    // Unlink from the first: both forget, the link closes, and neither tries again.
    let key = net.hubs[0].bonds[0].fp.clone();
    net.tell(0, p::ASK, 9, 7, ["unlink ", &key].concat().into_bytes());
    net.run(3000);
    assert!(net.hubs.iter().all(|h| h.bonds.is_empty() && h.pool.peers.is_empty()));
    // Neither the bond nor the pin is kept; each still keeps its own key.
    for (h, kept) in net.kept.iter().enumerate() {
        let lines: Vec<&str> = kept.lines().collect();
        assert_eq!(lines, [["key ", KEYS[h]].concat()], "{kept:?}");
        assert_eq!(net.hubs[h].own_fp, KEYS[h]);
    }
    let posts = net.posted.len();
    net.run(30_000);
    assert_eq!(net.posted.len(), posts);
}
