//! The /home snapshot homed keeps in IndexedDB, under [`KEYS`]: `"CSHM"`, u8
//! version 1, u8 0, u16 0, u64 seq, then the body (u32 count, then entries
//! of u8 kind (0 dir, 1 file), str path relative to /home (sorted, parents
//! first) and, for files, u32 len and the bytes), then a u64 FNV-1a-64 of
//! everything before it. [`Save`] and [`Restore`] are sans-IO walks over
//! [`crate::wire`] file ops, run by homed and natively by tests.

/// The two IndexedDB keys; a [`Pick`] names one by index.
pub const KEYS: [&str; 2] = ["home.a", "home.b"];

/// FNV-1a, 64-bit.
pub fn fnv64(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325, |h, &x| (h ^ u64::from(x)).wrapping_mul(0x100_0000_01b3))
}

/// A record holding `body` as sequence number `seq`. Stub: empty.
pub fn seal(seq: u64, body: &[u8]) -> Vec<u8> {
    let _ = (seq, body);
    Vec::new()
}

/// The sequence number and body of a record that verifies. Stub: none does.
pub fn open(image: &[u8]) -> Option<(u64, &[u8])> {
    let _ = image;
    None
}

/// What to do with the two stored records at load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pick {
    /// The record to restore (an index into [`KEYS`]): the highest seq that
    /// verifies. `None` when neither does.
    pub restore: Option<usize>,
    /// That record's seq, 0 for none: the next save is `seq + 1`.
    pub seq: u64,
    /// The record the next save overwrites: never the newest good one.
    pub write: usize,
    /// Both records exist and neither verifies: save nothing, ever.
    pub read_only: bool,
    /// What to tell the user ("restored an older copy of /home"), or "".
    pub note: &'static str,
}

/// Chooses between the records under [`KEYS`] (`None`: absent). Stub:
/// nothing to restore, write `home.a`.
pub fn pick(a: Option<&[u8]>, b: Option<&[u8]>) -> Pick {
    let _ = (a, b);
    Pick { restore: None, seq: 0, write: 0, read_only: false, note: "" }
}

/// One step of a walk: a request to send, the result, or a failure errno.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Send(Vec<u8>),
    Done(Vec<u8>),
    Fail(u16),
}

/// Reads the tree under a root with LIST and READ into a snapshot body.
#[derive(Debug)]
pub struct Save {
    _state: (),
}

impl Save {
    pub fn new(root: &str) -> Save {
        let _ = root;
        Save { _state: () }
    }

    /// The next step, given the reply to the last request (`None` at first).
    /// Ends with `Done(body)`. Stub: fails with ENOSYS.
    pub fn step(&mut self, reply: Option<(u16, &[u8])>) -> Step {
        let _ = reply;
        Step::Fail(crate::wire::ENOSYS)
    }
}

/// Writes a snapshot body under a root with MKDIR, OPEN CREAT|EXCL (files
/// made before the restore win) and WRITE.
#[derive(Debug)]
pub struct Restore {
    _state: (),
}

impl Restore {
    /// `None` when `body` does not parse. Stub: never parses.
    pub fn new(root: &str, body: &[u8]) -> Option<Restore> {
        let _ = (root, body);
        None
    }

    /// The next step, given the reply to the last request (`None` at first).
    /// Ends with `Done` and no bytes. Stub: fails with ENOSYS.
    pub fn step(&mut self, reply: Option<(u16, &[u8])>) -> Step {
        let _ = reply;
        Step::Fail(crate::wire::ENOSYS)
    }
}
