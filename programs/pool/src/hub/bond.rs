//! Devices linked for good (compusophy, 2026-10-08: "I shouldn't have to link them every single
//! time ... the link should be different than connect/disconnect"). A link is made once, with a
//! code, and kept for the profile ([`Bond`]): the other device's pinned DTLS fingerprint, a secret
//! the two made together over their first link (each sends a nonce, [`Msg::Bond`]; the secret is
//! the SHA-256 of the nonce of the side that showed the code, then the other's), which side
//! offers when they meet again (the one that showed the code), and what the device last said of
//! itself. A connection is live: whenever both are open and apart, they meet again with no code.
//! Both post to `/api/signal` under a mailbox only the two can name ([`mailbox`], from the
//! secret): the offering side keeps an offer there (`meet`) and asks for the answer (`poll`); the
//! other asks for the offer (`join`) and answers it (`answer`). Each checks that the other's
//! description carries the pinned fingerprint before it goes on, so the server, or anyone who
//! learned the mailbox, could at worst keep them apart, never stand between them. While the other
//! is away, tries back off from [`FIRST`] to [`MOST`] ms; an offer is made afresh every [`OFFER`]
//! ms (the server forgets one after three minutes). Unlink forgets the device here and asks it to
//! forget this one ([`Msg::Unbond`]).

use uiwire::pool;
use uiwire::relay::to_desk;

use super::{Hub, OPEN, Op, fingerprint};
use crate::msg::Msg;
use crate::test::Measured;

/// How a bond's kept line starts (the desktop looks for it to start the pool at sign-in).
pub const BOND: &str = "bond ";
/// ms before the first try again, the most between tries, an offer's life, and the longest a
/// description may take to make (ICE gathering takes four seconds at most).
pub const FIRST: u64 = 2000;
pub const MOST: u64 = 15_000;
pub const OFFER: u64 = 150_000;
pub const MAKE: u64 = 15_000;

/// Where a bond is in meeting its device again.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Step {
    /// Apart: the next try at `at`.
    #[default]
    Idle,
    /// Offering: the offer being made, kept at the server, then the answer asked for at `at`.
    Making,
    Offered,
    Polling,
    /// Answering: the offer asked for, then the answer being made.
    Finding,
    Answering,
    /// The answer given or taken: the link opening.
    Opening,
    /// Linked.
    Open,
}

/// A device linked for good: its pinned fingerprint, whether this side offers, the secret the two
/// share (hex), what it last said (name, kind, cores, what testing measured) and when it was last
/// here (Unix seconds); then, live, where meeting it stands, the link trying or open, when the
/// next try or poll is due, the wait between tries now, and the step's deadline.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bond {
    pub fp: String,
    pub host: bool,
    pub secret: String,
    pub name: String,
    pub kind: String,
    pub cores: u16,
    pub measured: Measured,
    pub seen: u32,
    pub step: Step,
    pub link: u32,
    pub at: u64,
    pub wait: u64,
    pub until: u64,
}

impl Bond {
    /// As kept: `bond <h|j> <secret> <seen> <cpu1> <cpun> <mem> <tested> <cores>`, then its name,
    /// kind and fingerprint, each after a tab.
    pub fn line(&self) -> String {
        let m = self.measured;
        let n = [self.seen, m.cpu1, m.cpun, m.mem, m.tested, self.cores.into()];
        let n = n.map(|n| n.to_string()).join(" ");
        let role = if self.host { "h " } else { "j " };
        [BOND, role, &self.secret, " ", &n, "\t", &self.name, "\t", &self.kind, "\t", &self.fp]
            .concat()
    }

    /// A kept line's bond (apart, its first try due at once), or `None` if it is not one.
    pub fn parse(line: &str) -> Option<Bond> {
        let mut parts = line.strip_prefix(BOND)?.split('\t');
        let head: Vec<&str> = parts.next()?.split(' ').collect();
        let (name, kind, fp) = (parts.next()?, parts.next()?, parts.next()?);
        let [role, secret, n @ ..] = &head[..] else { return None };
        let n: Vec<u32> = n.iter().map(|w| w.parse().ok()).collect::<Option<_>>()?;
        let [seen, cpu1, cpun, mem, tested, cores] = n[..] else { return None };
        let ok = secret.len() == 64 && fp.starts_with("sha-256 ") && matches!(*role, "h" | "j");
        ok.then(|| Bond {
            fp: fp.into(),
            host: *role == "h",
            secret: (*secret).into(),
            name: name.into(),
            kind: kind.into(),
            cores: cores.min(1024) as u16,
            measured: Measured { cpu1, cpun, mem, tested },
            seen,
            wait: FIRST,
            ..Bond::default()
        })
    }

    /// What Activity shows of it.
    pub fn shown(&self) -> pool::Bond {
        let state = match self.step {
            Step::Open => pool::ONLINE,
            Step::Opening => pool::CONNECTING,
            _ => pool::OFFLINE,
        };
        let m = self.measured;
        pool::Bond {
            name: self.name.clone(),
            kind: self.kind.clone(),
            key: self.fp.clone(),
            state,
            seen: self.seen,
            cores: self.cores,
            cpu1: m.cpu1,
            cpun: m.cpun,
            mem: m.mem,
            tested: m.tested,
        }
    }
}

/// The mailbox two bonded devices meet at, from their secret: 24 hex digits only they can name.
pub fn mailbox(secret: &str) -> String {
    let h = sha::sha256(["computehub rendezvous ", secret].concat().as_bytes());
    sha::hex(&h[..12]).to_ascii_uppercase()
}

/// A nonce for a new bond: 32 bytes of the system's randomness (std's hasher keys, which WASI's
/// `random_get` makes) and the time, hashed; hex.
pub fn nonce(now: u64, clock: u32) -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut seed = Vec::new();
    for i in 0..4u8 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u8(i);
        seed.extend_from_slice(&h.finish().to_le_bytes());
    }
    seed.extend_from_slice(&now.to_le_bytes());
    seed.extend_from_slice(&clock.to_le_bytes());
    sha::hex(&sha::sha256(&seed))
}

/// The wait after `wait`: half again, at most [`MOST`].
fn longer(wait: u64) -> u64 {
    (wait * 3 / 2).clamp(FIRST, MOST)
}

impl Hub {
    /// The bond whose link (trying or open) is `link`.
    fn bond_at(&self, link: u32) -> Option<usize> {
        self.bonds.iter().position(|b| b.link == link && link != 0)
    }

    /// The fingerprint heard for `link`.
    fn fp_of(&self, link: u32) -> String {
        let fp = self.heard_fp.iter().rev().find(|h| h.0 == link);
        fp.map_or(String::new(), |h| h.1.clone())
    }

    /// Bond `i`'s try ends: its link closed unless open, the next try at `again`.
    fn abandon(&mut self, i: usize, again: u64) {
        let b = &mut self.bonds[i];
        let link = std::mem::take(&mut b.link);
        (b.step, b.at, b.wait) = (Step::Idle, again, longer(b.wait));
        if link != 0 && self.pool.peers.iter().all(|p| p.link != link) {
            self.send(to_desk::UNLINK, link, &[]);
        }
        self.heard_fp.retain(|h| h.0 != link);
    }

    /// Bonds' next steps at `now`: a try due, a poll due, a step past its deadline.
    pub(super) fn meet(&mut self, now: u64) {
        for i in 0..self.bonds.len() {
            let b = &self.bonds[i];
            let (step, at, until, host, fp) = (b.step, b.at, b.until, b.host, b.fp.clone());
            let mb = mailbox(&b.secret);
            if !matches!(step, Step::Idle | Step::Open) && now >= until {
                // An offer outlives the server's memory of it: a fresh one at once. Else later.
                let again = if step == Step::Polling { now } else { now + b.wait };
                self.abandon(i, again);
                continue;
            }
            match step {
                Step::Idle if now >= at && host => {
                    self.last_link += 1;
                    let link = self.last_link;
                    let b = &mut self.bonds[i];
                    (b.step, b.link, b.until) = (Step::Making, link, now + MAKE);
                    self.send(to_desk::LINK, link, &[]);
                }
                Step::Idle if now >= at => {
                    let b = &mut self.bonds[i];
                    (b.step, b.until) = (Step::Finding, now + MAKE);
                    self.post(
                        Op::Find(fp),
                        &[
                            "join
", &mb,
                        ]
                        .concat(),
                    );
                }
                Step::Polling if now >= at => {
                    self.bonds[i].at = u64::MAX;
                    let link = self.bonds[i].link;
                    self.post(
                        Op::Wait(link),
                        &[
                            "poll
", &mb,
                        ]
                        .concat(),
                    );
                }
                _ => {}
            }
        }
    }

    /// When a bond next needs the clock.
    pub(super) fn meet_due(&self) -> Option<u64> {
        let due = |b: &Bond| match b.step {
            Step::Idle => Some(b.at),
            Step::Open => None,
            Step::Polling => Some(b.at.min(b.until)),
            _ => Some(b.until),
        };
        self.bonds.iter().filter_map(due).min()
    }

    /// This tab's description for link `link` is made: whether it was a bond's (its offer kept at
    /// the mailbox, or its answer given there).
    pub(super) fn meet_signal(&mut self, link: u32, sdp: &str) -> bool {
        let Some(i) = self.bond_at(link) else { return false };
        let now = self.now;
        let b = &mut self.bonds[i];
        let mb = mailbox(&b.secret);
        match b.step {
            Step::Making => {
                (b.step, b.until) = (Step::Offered, now + OFFER);
                self.post(
                    Op::Meet(link),
                    &[
                        "meet
", &mb, "
", sdp,
                    ]
                    .concat(),
                );
            }
            Step::Answering => {
                (b.step, b.until) = (Step::Opening, now + OPEN);
                self.post(
                    Op::Reply(link),
                    &[
                        "answer
", &mb, "
", sdp,
                    ]
                    .concat(),
                );
            }
            _ => {}
        }
        true
    }

    /// The server answered a bond's post `op` with `status` and `body`.
    pub(super) fn met(&mut self, op: Op, status: u32, body: &str) {
        let now = self.now;
        let i = match &op {
            Op::Find(fp) => self.bonds.iter().position(|b| &b.fp == fp && b.step == Step::Finding),
            Op::Meet(l) | Op::Wait(l) | Op::Reply(l) => self.bond_at(*l),
            _ => None,
        };
        let Some(i) = i else { return };
        let ok = status == 200 && !body.is_empty();
        // The other device's description, with the key pinned for it, or nothing.
        let theirs = fingerprint(body) == self.bonds[i].fp;
        let (fp, wait) = (self.bonds[i].fp.clone(), self.bonds[i].wait);
        match op {
            Op::Meet(_) if status == 200 => {
                let b = &mut self.bonds[i];
                (b.step, b.at) = (Step::Polling, now + FIRST);
            }
            Op::Wait(link) if ok && theirs => {
                let b = &mut self.bonds[i];
                (b.step, b.until) = (Step::Opening, now + OPEN);
                self.heard_fp.push((link, fp));
                self.send(to_desk::ACCEPT, link, body.as_bytes());
            }
            // Nobody yet: ask again, less often the longer the other is away.
            Op::Wait(_) if status == 204 => {
                let b = &mut self.bonds[i];
                (b.at, b.wait) = (now + wait, longer(wait));
            }
            Op::Find(_) if ok && theirs => {
                self.last_link += 1;
                let link = self.last_link;
                let b = &mut self.bonds[i];
                (b.step, b.link, b.until) = (Step::Answering, link, now + MAKE);
                self.heard_fp.push((link, fp));
                self.send(to_desk::LINK, link, body.as_bytes());
            }
            Op::Reply(_) if status == 200 => {}
            _ => self.abandon(i, now + wait),
        }
    }

    /// Link `link` opened (after the pool heard of it): a bond's try succeeded, or pairing made
    /// a link, whose bond the two now make (each sends its nonce).
    pub(super) fn bond_linked(&mut self, link: u32, paired: Option<bool>) {
        let (now, clock) = (self.now, self.pool.clock);
        let fp = self.fp_of(link);
        let mine = |b: &Bond| b.link == link || (!fp.is_empty() && b.fp == fp);
        if let Some(i) = self.bonds.iter().position(mine) {
            let b = &mut self.bonds[i];
            (b.step, b.link, b.wait, b.seen) = (Step::Open, link, FIRST, clock.max(b.seen));
            self.keep();
        }
        if let Some(host) = paired {
            let mine = nonce(now, clock);
            self.pool.out.push(crate::pool::Act::Send(link, Msg::Bond { nonce: mine.clone() }));
            self.nonces.push((link, mine, host));
        }
    }

    /// Link `link` is gone: its bond is apart (tried again soon), or its try failed.
    pub(super) fn bond_lost(&mut self, link: u32) {
        self.nonces.retain(|n| n.0 != link);
        let Some(i) = self.bond_at(link) else { return };
        let now = self.now;
        if self.bonds[i].step == Step::Open {
            let b = &mut self.bonds[i];
            (b.step, b.link, b.at, b.wait) = (Step::Idle, 0, now + FIRST, FIRST);
            b.seen = self.pool.clock.max(b.seen);
            self.keep();
        } else {
            let again = now + self.bonds[i].wait;
            self.abandon(i, again);
        }
    }

    /// A message on link `link` the hub answers itself (a nonce, a goodbye) or learns from (what
    /// a bonded device says of itself); whether the pool need not hear it.
    pub(super) fn bond_heard(&mut self, link: u32, m: &Msg) -> bool {
        let fp = self.fp_of(link);
        match m {
            Msg::Bond { nonce } => {
                let Some(k) = self.nonces.iter().position(|n| n.0 == link) else { return true };
                let (_, mine, host) = self.nonces.remove(k);
                let seed = if host { [&mine, nonce.as_str()] } else { [nonce.as_str(), &mine] };
                let secret = sha::hex(&sha::sha256(seed.concat().as_bytes()));
                // The same profile on this device (one certificate): nothing to keep.
                if fp.is_empty() || fp == self.own_fp || nonce.len() != 64 {
                    return true;
                }
                let peer = self.pool.peers.iter().find(|p| p.link == link);
                let info = peer.map(|p| p.info.clone()).unwrap_or_default();
                let (name, kind, cores, measured) =
                    (info.name, info.kind, info.cores, info.measured);
                self.bonds.retain(|b| b.fp != fp);
                let seen = self.pool.clock;
                let (step, wait) = (Step::Open, FIRST);
                let b = Bond {
                    fp,
                    host,
                    secret,
                    name,
                    kind,
                    cores,
                    measured,
                    seen,
                    step,
                    link,
                    wait,
                    ..Bond::default()
                };
                self.bonds.push(b);
                self.keep();
                true
            }
            Msg::Unbond => {
                if let Some(i) = self.bonds.iter().position(|b| b.fp == fp && !fp.is_empty()) {
                    self.bonds.remove(i);
                    self.pins.retain(|p| *p != fp);
                    self.keep();
                }
                self.close(link);
                true
            }
            Msg::Hello { name, kind, cores, cpu1, cpun, mem, tested, .. } => {
                let clock = self.pool.clock;
                let measured = Measured { cpu1: *cpu1, cpun: *cpun, mem: *mem, tested: *tested };
                let Some(b) = self.bonds.iter_mut().find(|b| b.fp == fp && !fp.is_empty()) else {
                    return false;
                };
                let said = (name.clone(), kind.clone(), *cores, measured);
                let changed = said != (b.name.clone(), b.kind.clone(), b.cores, b.measured);
                // Last seen, to the minute: kept no more often.
                let stale = clock >= b.seen.saturating_add(60);
                (b.name, b.kind, b.cores, b.measured) = said;
                if stale {
                    b.seen = clock;
                }
                if changed || stale {
                    self.keep();
                }
                false
            }
            _ => false,
        }
    }

    /// This tab closes link `link` itself (the page tells nothing then): the pool and the bonds
    /// hear it here.
    pub(super) fn close(&mut self, link: u32) {
        self.send(to_desk::UNLINK, link, &[]);
        self.pool.unlinked(link, self.now);
        self.bond_lost(link);
    }

    /// Activity asks of a bond: `reconnect <key>` (try now) or `unlink <key>`.
    pub(super) fn link_ask(&mut self, what: &str) {
        let now = self.now;
        let (verb, key) = what.split_once(' ').unwrap_or((what, ""));
        let Some(i) = self.bonds.iter().position(|b| b.fp == key) else { return };
        match verb {
            "reconnect" if self.bonds[i].step != Step::Open => {
                self.abandon(i, now);
                self.bonds[i].wait = FIRST;
            }
            "unlink" => {
                let b = self.bonds.remove(i);
                self.pins.retain(|p| *p != b.fp);
                self.keep();
                if b.step == Step::Open {
                    // Asked to forget this tab too, the link closes a second later.
                    self.pool.out.push(crate::pool::Act::Send(b.link, Msg::Unbond));
                    self.bye.push((b.link, now + 1000));
                } else if b.link != 0 {
                    self.send(to_desk::UNLINK, b.link, &[]);
                }
            }
            _ => {}
        }
    }

    /// Links said goodbye to whose second is up close.
    pub(super) fn goodbyes(&mut self, now: u64) {
        let due: Vec<u32> = self.bye.iter().filter(|b| now >= b.1).map(|b| b.0).collect();
        self.bye.retain(|b| now < b.1);
        due.into_iter().for_each(|l| self.close(l));
    }
}
