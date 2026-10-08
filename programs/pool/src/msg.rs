//! What tabs say to each other on a link: one [`Msg`] a data channel message, little-endian in
//! uiwire's writer, its kind's byte first. A message is whole in itself, bounded and named by
//! its kind (an unknown one is dropped), so it could cross any network, as a kernel message
//! could: the link is only the way it goes.

use uiwire::{Out, Reader};

/// The most bytes of one message, and of a chunk's line or answer.
pub const MAX: usize = 64 << 10;

/// A message on a link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Msg {
    /// What the tab has to lend (sent once linked): its name and kind, cores, RAM and storage in
    /// MB (0 when unknown), whether it has a GPU to compute on, and the model it shares (empty:
    /// none) and its speed, tenths of a token a second.
    Hello {
        name: String,
        kind: String,
        cores: u16,
        ram_mb: u32,
        quota_mb: u32,
        gpu: bool,
        model: String,
        tok: u32,
    },
    /// Its workers and how many are busy, the chunks it answered and their fuel (cumulative):
    /// each second while linked.
    Stats {
        workers: u16,
        busy: u16,
        chunks: u32,
        units: u64,
    },
    /// A round trip: the sender's clock, sent back as it came.
    Ping {
        t: u32,
    },
    Pong {
        t: u32,
    },
    /// Measuring the link: filler; the `last` of a run says it ends, and asks the other tab to
    /// send `back` bytes the same way.
    Probe {
        last: bool,
        back: u32,
        fill: Vec<u8>,
    },
    /// The run's bytes the other tab heard, in how many ms.
    Heard {
        bytes: u32,
        ms: u32,
    },
    /// A job starts on the sender: `/bin/<name> work` answers its chunks.
    Job {
        job: u32,
        name: String,
    },
    /// Up to `n` chunks of `job`, please.
    Want {
        job: u32,
        n: u16,
    },
    /// Chunks of `job` for the asker: each its index and line (none: none waiting).
    Give {
        job: u32,
        chunks: Vec<(u32, String)>,
    },
    /// Chunk `index` of `job` answered: the worker's line.
    Done {
        job: u32,
        index: u32,
        out: String,
    },
    /// The job is over: drop its chunks. `used`: how many of the answers this tab sent were the
    /// first for their chunk (the rest were taken back and answered elsewhere first).
    End {
        job: u32,
        used: u32,
    },
    /// A question for the model the other tab shares, the asker's `ask` its name.
    Ask {
        ask: u32,
        text: String,
    },
    /// More of the answer to `ask`, as the model writes it.
    Words {
        ask: u32,
        text: String,
    },
    /// The answer to `ask` is whole: the model's speed, or why it failed (empty: it did not).
    Answered {
        ask: u32,
        tok: u32,
        why: String,
    },
}

impl Msg {
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Out(Vec::new());
        match self {
            Msg::Hello { name, kind, cores, ram_mb, quota_mb, gpu, model, tok } => {
                o.u8(1).str(name).str(kind).u16(*cores).u32(*ram_mb).u32(*quota_mb);
                o.u8((*gpu).into()).str(model).u32(*tok)
            }
            Msg::Stats { workers, busy, chunks, units } => {
                o.u8(2).u16(*workers).u16(*busy).u32(*chunks).u64(*units)
            }
            Msg::Ping { t } => o.u8(3).u32(*t),
            Msg::Pong { t } => o.u8(4).u32(*t),
            Msg::Probe { last, back, fill } => o.u8(5).u8((*last).into()).u32(*back).bytes(fill),
            Msg::Heard { bytes, ms } => o.u8(6).u32(*bytes).u32(*ms),
            Msg::Job { job, name } => o.u8(7).u32(*job).str(name),
            Msg::Want { job, n } => o.u8(8).u32(*job).u16(*n),
            Msg::Give { job, chunks } => {
                let o = o.u8(9).u32(*job).len(chunks.len());
                chunks.iter().fold(o, |o, (i, line)| o.u32(*i).str(line))
            }
            Msg::Done { job, index, out } => o.u8(10).u32(*job).u32(*index).str(out),
            Msg::End { job, used } => o.u8(11).u32(*job).u32(*used),
            Msg::Ask { ask, text } => o.u8(12).u32(*ask).str(text),
            Msg::Words { ask, text } => o.u8(13).u32(*ask).str(text),
            Msg::Answered { ask, tok, why } => o.u8(14).u32(*ask).u32(*tok).str(why),
        };
        o.0
    }

    /// Exactly one message, or `None`: malformed, trailing, unknown or over [`MAX`].
    pub fn decode(b: &[u8]) -> Option<Msg> {
        let mut r = Reader(Some(b).filter(|b| b.len() <= MAX)?);
        let m = match r.u8()? {
            1 => Msg::Hello {
                name: r.str()?,
                kind: r.str()?,
                cores: r.u16()?,
                ram_mb: r.u32()?,
                quota_mb: r.u32()?,
                gpu: r.bool()?,
                model: r.str()?,
                tok: r.u32()?,
            },
            2 => {
                Msg::Stats { workers: r.u16()?, busy: r.u16()?, chunks: r.u32()?, units: r.u64()? }
            }
            3 => Msg::Ping { t: r.u32()? },
            4 => Msg::Pong { t: r.u32()? },
            5 => Msg::Probe { last: r.bool()?, back: r.u32()?, fill: r.bytes()?.to_vec() },
            6 => Msg::Heard { bytes: r.u32()?, ms: r.u32()? },
            7 => Msg::Job { job: r.u32()?, name: r.str()? },
            8 => Msg::Want { job: r.u32()?, n: r.u16()? },
            9 => {
                let (job, n) = (r.u32()?, r.count()?);
                // Each chunk takes at least 8 bytes: a count past that is malformed.
                let n = Some(n).filter(|n| *n <= r.0.len() / 8)?;
                let chunks = (0..n).map(|_| Some((r.u32()?, r.str()?)));
                Msg::Give { job, chunks: chunks.collect::<Option<_>>()? }
            }
            10 => Msg::Done { job: r.u32()?, index: r.u32()?, out: r.str()? },
            11 => Msg::End { job: r.u32()?, used: r.u32()? },
            12 => Msg::Ask { ask: r.u32()?, text: r.str()? },
            13 => Msg::Words { ask: r.u32()?, text: r.str()? },
            14 => Msg::Answered { ask: r.u32()?, tok: r.u32()?, why: r.str()? },
            _ => return None,
        };
        r.0.is_empty().then_some(m)
    }
}
