//! /home kept across reloads: a [`ui::kernel::snap`] of it in `localStorage` ([`KEY`]), a byte a
//! char (U+0000 to U+00FF), put back as the desktop starts. A change under /home is kept at once,
//! then at most once a [`GAP_MS`] (the one-shot timer brings the last), and when the page hides
//! or goes away; a snapshot like the last kept is not written again. When it cannot be:
//!
//! - The page will not store one (full, or storage blocked): that is reported, and said in
//!   Settings → Privacy ([`Home::unkept`]) until a write takes or /home is back as last kept. It
//!   is tried again as the page hides.
//! - The stored one does not read back (damaged, from a newer OS, or refused by the filesystem):
//!   it is set aside under [`BAD`], and the desktop starts with a fresh /home, kept over it at
//!   once. One that cannot be set aside stays, and nothing is kept over it (unkept).
//! - Another tab kept its own /home since this one read or kept it ([`MARK`] moved), or removed
//!   this profile: nothing is kept over it (unkept), and a reload shows that tab's files.
//!
//! The keys are the signed-in profile's ([`logon::own`]); these are the first profile's.

use logon::own;
use platform::Ctl;
use ui::kernel::snap::{self, SnapError};
use vfs::Vfs;

use crate::report::{self, Reports};

/// The `localStorage` keys of the snapshot, of one set aside, and of the mark each write moves
/// (the snapshot's sum), which tells a tab that another wrote since.
pub const KEY: &str = "compusophy.home";
pub const BAD: &str = "compusophy.home.bad";
pub const MARK: &str = "compusophy.home.mark";
/// The least time between two writes, in ms.
pub const GAP_MS: f64 = 1000.0;

/// What was kept: the generation of /home (which holds [`Vfs::HOME`]) last looked at, the
/// snapshot last kept and its seq,
/// the [`MARK`] as last read or written, when the next may be written (page clock ms), whether a
/// change waits, whether storage holds what this page must not write over, and whether files go
/// unkept (and whether apps were told so).
#[derive(Debug, Default)]
pub struct Home {
    seen: u64,
    kept: Vec<u8>,
    seq: u64,
    mark: Option<String>,
    next: f64,
    waits: bool,
    held: bool,
    pub unkept: bool,
    told: bool,
}

impl Home {
    /// Puts the stored snapshot back into `vfs` (a fresh one, at start).
    pub fn restore(&mut self, vfs: &mut Vfs, ctl: &mut Ctl, report: &mut Reports) {
        self.mark = ctl.storage_get(&own(MARK));
        let Some(stored) = ctl.storage_get(&own(KEY)) else { return };
        let bytes: Option<Vec<u8>> = stored.chars().map(|c| u8::try_from(c).ok()).collect();
        match bytes.ok_or(SnapError::Damaged).and_then(|b| snap::give(vfs, &b).map(|s| (b, s))) {
            Ok((b, seq)) => (self.kept, self.seq) = (b, seq),
            Err(e) => {
                // Set aside, the fresh /home goes over it; one that cannot be stays.
                let aside = ctl.storage_put(&own(BAD), &stored);
                (self.waits, self.held, self.unkept) = (aside, !aside, !aside);
                let why = match e {
                    SnapError::Newer => "a newer OS's",
                    SnapError::Damaged => "damaged",
                    SnapError::Vfs(_) => "refused",
                };
                let then = if aside { "); set aside" } else { "); left, and not written over" };
                let message = ["Your saved files did not read back (", why, then].concat();
                report.failed("error", &message, "home unread");
            }
        }
        self.seen = vfs.generation_of(Vfs::HOME);
    }

    /// Keeps /home if it changed and may be written now (or `now_too`, the page hiding); else
    /// in how many ms it may.
    pub fn keep(
        &mut self,
        vfs: &Vfs,
        ctl: &mut Ctl,
        report: &mut Reports,
        now_too: bool,
    ) -> Option<u32> {
        let generation = vfs.generation_of(Vfs::HOME);
        // What was not kept is tried again as the page hides.
        self.waits |= generation != self.seen || (self.unkept && now_too);
        self.seen = generation;
        let now = ctl.monotonic_ms();
        if self.held || !self.waits || (now < self.next && !now_too) {
            return (self.waits && !self.held).then(|| (self.next - now) as u32 + 1);
        }
        (self.waits, self.next) = (false, now + GAP_MS);
        let moved = ctl.storage_get(&own(MARK)).is_some_and(|m| self.mark.as_ref() != Some(&m));
        self.held = moved || !logon::listed(ctl.storage_get(logon::profiles::LIST).as_deref());
        if self.held {
            report::note("home kept by another tab");
            self.unkept = true;
            return None;
        }
        let bytes = snap::take(vfs, self.seq + 1);
        if entries(&bytes) == entries(&self.kept) {
            self.unkept = false; // as stored
            return None;
        }
        let kept = ctl.storage_put(&own(KEY), &chars(&bytes));
        if kept {
            let mark = chars(&bytes[bytes.len() - 8..]);
            if ctl.storage_put(&own(MARK), &mark) {
                self.mark = Some(mark);
            }
            (self.kept, self.seq) = (bytes, self.seq + 1);
        } else if !self.unkept {
            report.failed(
                "error",
                "Files could not be kept in this browser's storage",
                "home unkept",
            );
        }
        self.unkept = !kept;
        None
    }
}

impl Home {
    /// The bytes of /home as last kept (its snapshot's).
    pub fn kept_len(&self) -> usize {
        self.kept.len()
    }

    /// Whether apps should hear [`Home::unkept`] again: it changed since they last did.
    pub fn retell(&mut self) -> bool {
        std::mem::replace(&mut self.told, self.unkept) != self.unkept
    }
}

/// A snapshot's entries, its head (with the seq) and sum aside.
fn entries(b: &[u8]) -> &[u8] {
    b.get(16..b.len().saturating_sub(8)).unwrap_or_default()
}

/// Bytes as `localStorage` holds them, a char each.
fn chars(b: &[u8]) -> String {
    b.iter().map(|&b| char::from(b)).collect()
}
