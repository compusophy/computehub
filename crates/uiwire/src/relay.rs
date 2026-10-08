//! How the desktop and the pool program (`/bin/pool`, the mesh's queue, links and pairing) talk:
//! [`Frame`]s on the program's console, raw, little-endian. The desktop holds what only the page
//! can do (the links, posts to `/api/signal`, workers, storage, the clock) and does what a frame
//! asks; the pool holds the rest. Each frame says the page's clock when it was sent (`now`, page
//! ms, wrapping).
//!
//! ```text
//! u8 op | u32 a | u32 b | u32 now | u32 n | n bytes
//! ```

use crate::{Out, Reader};

/// What the desktop tells the pool: `a`, `b` and the bytes as each says.
pub mod to_pool {
    /// The device first: `cores ram_mb gpu touch` (0 or 1), its agent, then the pinned keys,
    /// a line each.
    pub const INFO: u8 = 1;
    /// Process `a` asked for a request (its bytes): Pair, Measure or Job.
    pub const ASK: u8 = 2;
    /// Link `a`'s description for the other tab.
    pub const SIGNAL: u8 = 3;
    pub const LINKED: u8 = 4;
    /// A message on link `a`.
    pub const DATA: u8 = 5;
    pub const UNLINKED: u8 = 6;
    /// Post `a` ended with status `b` (0: none came), its body having come in [`PART`]s; or
    /// fetch `a` answered, 200 and the body (0 and why not).
    pub const HTTP: u8 = 7;
    /// Worker `a` wrote this; worker `a` ended; a worker started as `a` (0: none could).
    pub const OUT: u8 = 8;
    pub const GONE: u8 = 9;
    pub const SPAWNED: u8 = 10;
    /// The time asked for came ([`super::to_desk::WAKE`]).
    pub const TICK: u8 = 11;
    /// Activity's process watches now as `a` (0: none).
    pub const WATCH: u8 = 12;
    /// The page's storage quota, `a` MB.
    pub const QUOTA: u8 = 13;
    /// More of post `a`'s body, as it comes (a model's answer streams).
    pub const PART: u8 = 14;
}

/// What the pool asks of the desktop.
pub mod to_desk {
    /// The pool is ready for frames (its first).
    pub const READY: u8 = 0;
    /// Make link `a`: an offer, or the answer to the offer given.
    pub const LINK: u8 = 1;
    /// Link `a`'s answer from the other tab.
    pub const ACCEPT: u8 = 2;
    /// Send the bytes on link `a`.
    pub const SEND: u8 = 3;
    pub const UNLINK: u8 = 4;
    /// Post the bytes after the first line to the URL on it as post `a` (none: get it): the
    /// same origin's `/api/signal`, or a model's local server.
    pub const POST: u8 = 5;
    /// Start `a` workers of the program named.
    pub const SPAWN: u8 = 6;
    /// Write the bytes to worker `a`'s console; end worker `a`.
    pub const FEED: u8 = 7;
    pub const KILL: u8 = 8;
    /// Post the bytes (one uiwire event) to process `a`: Activity's, or one that asked.
    pub const TELL: u8 = 9;
    /// Keep the bytes as this profile's pinned keys.
    pub const PINS: u8 = 10;
    /// Wake the pool in `a` ms ([`super::to_pool::TICK`]).
    pub const WAKE: u8 = 11;
    /// Fetch the same-origin relative URL named, as post `a` (its answer an HTTP: 200 and the
    /// body, or 0 and why not).
    pub const FETCH: u8 = 12;
}

/// One frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub op: u8,
    pub a: u32,
    pub b: u32,
    pub now: u32,
    pub data: Vec<u8>,
}

/// Its header's bytes.
pub const HEAD: usize = 17;

impl Frame {
    /// Appends the frame's bytes to `out`.
    pub fn put(&self, out: &mut Vec<u8>) {
        let mut o = Out(core::mem::take(out));
        o.u8(self.op).u32(self.a).u32(self.b).u32(self.now).bytes(&self.data);
        *out = o.0;
    }

    /// The first whole frame of `b` and the bytes it took; `None` until one is whole.
    pub fn take(b: &[u8]) -> Option<(Frame, usize)> {
        let mut r = Reader(b);
        let (op, a, bb, now) = (r.u8()?, r.u32()?, r.u32()?, r.u32()?);
        let data = r.bytes()?.to_vec();
        let used = HEAD + data.len();
        Some((Frame { op, a, b: bb, now, data }, used))
    }
}
