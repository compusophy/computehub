//! /home kept across reloads: a [`ui::kernel::snap`] of it in `localStorage` ([`KEY`]), a byte a
//! char (U+0000 to U+00FF), put back as the desktop starts. A change is kept at once, then at
//! most once a [`GAP_MS`] (the one-shot timer brings the last), and when the page is hidden
//! (a reload or a closed tab hides it first); a snapshot like the last kept is not written
//! again. One the page will not store (full, or storage blocked) is reported and said in
//! Settings → Privacy; one that does not read back (damaged, or from a newer OS) is set aside
//! under [`BAD`], and the desktop starts with a fresh /home. Two tabs keep what each last saw.

use platform::Ctl;
use ui::kernel::snap::{self, SnapError};
use vfs::Vfs;

use crate::report::Reports;

/// The `localStorage` keys of the snapshot and of one set aside.
pub const KEY: &str = "compusophy.home";
pub const BAD: &str = "compusophy.home.bad";
/// The least time between two writes, in ms.
pub const GAP_MS: f64 = 1000.0;

/// What was kept: the generation last looked at, the snapshot last kept and its seq, when the
/// next may be written (page clock ms), whether a change waits, and whether the last write
/// failed (and whether apps were told so).
#[derive(Debug, Default)]
pub struct Home {
    seen: u64,
    kept: Vec<u8>,
    seq: u64,
    next: f64,
    waits: bool,
    pub unkept: bool,
    told: bool,
}

impl Home {
    /// Puts the stored snapshot back into `vfs` (a fresh one, at start).
    pub fn restore(&mut self, vfs: &mut Vfs, ctl: &mut Ctl, report: &mut Reports) {
        let Some(stored) = ctl.storage_get(KEY) else { return };
        let bytes: Option<Vec<u8>> = stored.chars().map(|c| u8::try_from(c).ok()).collect();
        match bytes.ok_or(SnapError::Damaged).and_then(|b| snap::give(vfs, &b).map(|s| (b, s))) {
            Ok((b, seq)) => (self.kept, self.seq) = (b, seq),
            Err(e) => {
                ctl.storage_set(BAD, &stored);
                let why = if e == SnapError::Newer { "a newer OS's" } else { "damaged" };
                let message = ["Your saved files did not read back (", why, "); set aside"];
                report.failed("error", &message.concat(), "home unread");
            }
        }
        self.seen = vfs.generation();
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
        let generation = vfs.generation();
        self.waits |= generation != self.seen;
        self.seen = generation;
        let now = ctl.monotonic_ms();
        if !self.waits || (now < self.next && !now_too) {
            return self.waits.then(|| (self.next - now) as u32 + 1);
        }
        (self.waits, self.next) = (false, now + GAP_MS);
        let bytes = snap::take(vfs, self.seq + 1);
        if entries(&bytes) == entries(&self.kept) {
            return None;
        }
        let kept = ctl.storage_put(KEY, &bytes.iter().map(|&b| char::from(b)).collect::<String>());
        if kept {
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
    /// Whether apps should hear [`Home::unkept`] again: it changed since they last did.
    pub fn retell(&mut self) -> bool {
        std::mem::replace(&mut self.told, self.unkept) != self.unkept
    }
}

/// A snapshot's entries, its head (with the seq) and sum aside.
fn entries(b: &[u8]) -> &[u8] {
    b.get(16..b.len().saturating_sub(8)).unwrap_or_default()
}
