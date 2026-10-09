//! The mesh as Activity's Pool page hears it: one [`Event::Pool`](crate::Event::Pool) snapshot,
//! little-endian, a `str` a u32 length and UTF-8:
//!
//! ```text
//! u8 VERSION | u32 at (page ms) | str pairing | str code | u16 n, n device | u8 has job, job
//!         | str serve | str serving | u8 has answer, answer | str testing
//! device: str name | str kind | u16 cores | u32 ram_mb | u32 quota_mb | u8 gpu | u32 up_ms
//!         | u8 known | u16 workers | u16 busy | u32 chunks | u64 units | u32 rtt | u64 tx
//!         | u64 rx | u32 up | u32 down | str model | u32 tok | u32 ctx | u32 cpu1 | u32 cpun
//!         | u32 mem | u32 tested
//! job:    str name | u8 mine | u32 total | u32 done | u32 queued | u32 steals | u32 requeued
//!         | u32 checked | u32 mismatched | u32 ms | u32 used | u32 busy | u16 m, m u32
//!         (chunks by device)
//! answer: str question | str by | str text | u8 done | u32 tok | str why
//! ```
//!
//! Device 0 is this tab, the rest each linked tab in the order it joined. Counts (`chunks`,
//! `units`, `tx`, `rx`) are cumulative, so a reader takes rates from two snapshots; the others
//! are as they are now: 0 for what a device does not say (`ram_mb`, `quota_mb`, `up`, `down`),
//! `u32::MAX` for a link not yet measured (`rtt`).

use crate::{Out, Reader};

/// The format's version, a snapshot's first byte.
pub const VERSION: u8 = 6;
/// An unknown round trip.
pub const UNKNOWN: u32 = u32::MAX;
/// Each device's canvas color, by its place in a snapshot (cycling): cyan here, then yellow,
/// magenta, green, red and blue, so a device looks the same wherever it is drawn.
pub const TINTS: [u8; 6] = [6, 3, 5, 2, 1, 4];

/// One snapshot: when, what pairing says (empty when nothing is pairing) and the code it shows,
/// the devices and the job; the model server this tab shares (empty: none) and what sharing
/// says (empty when it is well), the answer the pool's model is writing here or wrote last, and
/// what testing this device says (empty when no test runs).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snap {
    pub at: u32,
    pub pairing: String,
    pub code: String,
    pub devices: Vec<Device>,
    pub job: Option<Job>,
    pub serve: String,
    pub serving: String,
    pub answer: Option<Answer>,
    pub testing: String,
}

/// A device of the pool: its name and kind, what it has (cores, RAM and storage the browser
/// grants in MB, a GPU it can compute on), how long it has been linked (ms), whether its key was
/// pinned before; its workers and how many are busy, the chunks it answered and their fuel; the
/// link's round trip (ms), bytes sent to it and heard from it, and the throughput last measured
/// each way (bytes a second); the model it shares (empty: none), its speed in tenths of a token
/// a second (0: not measured) and its context in tokens (0: unknown); what testing it measured,
/// none of it until then: its CPU's speed on one core and on all at once (mebibytes of SHA-256 a
/// second), the memory a tab could hold (MiB) and when (Unix seconds; 0: never tested). This
/// tab's link figures are 0.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    pub kind: String,
    pub cores: u16,
    pub ram_mb: u32,
    pub quota_mb: u32,
    pub gpu: bool,
    pub up_ms: u32,
    pub known: bool,
    pub workers: u16,
    pub busy: u16,
    pub chunks: u32,
    pub units: u64,
    pub rtt: u32,
    pub tx: u64,
    pub rx: u64,
    pub up: u32,
    pub down: u32,
    pub model: String,
    pub tok: u32,
    pub ctx: u32,
    pub cpu1: u32,
    pub cpun: u32,
    pub mem: u32,
    pub tested: u32,
}

/// The job on the pool: the program, whether this tab started it, its chunks in all, answered and
/// waiting; how often a device took chunks from the queue and how many came back to it (a worker
/// or a link gone); answers replayed here to check them and those that differed; ms since it
/// started; for a job helped, how many of the answers here its tab used (the first for their
/// chunk: the rest were taken back and answered there first) and the ms from its start to the
/// last answer here (how long this tab was busy with it); and the chunks each device answered.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Job {
    pub name: String,
    pub mine: bool,
    pub total: u32,
    pub done: u32,
    pub queued: u32,
    pub steals: u32,
    pub requeued: u32,
    pub checked: u32,
    pub mismatched: u32,
    pub ms: u32,
    pub used: u32,
    pub busy: u32,
    pub per: Vec<u32>,
}

/// An answer from the pool's model: the question, the device whose model answers, what it wrote
/// so far, whether it is done, its speed (tenths of a token a second) and, if it failed, why.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Answer {
    pub question: String,
    pub by: String,
    pub text: String,
    pub done: bool,
    pub tok: u32,
    pub why: String,
}

impl Snap {
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Out(Vec::new());
        o.u8(VERSION).u32(self.at).str(&self.pairing).str(&self.code);
        o.u16(self.devices.len() as u16);
        for d in &self.devices {
            o.str(&d.name).str(&d.kind).u16(d.cores).u32(d.ram_mb).u32(d.quota_mb);
            o.u8(d.gpu.into()).u32(d.up_ms).u8(d.known.into()).u16(d.workers).u16(d.busy);
            o.u32(d.chunks).u64(d.units).u32(d.rtt).u64(d.tx).u64(d.rx).u32(d.up).u32(d.down);
            o.str(&d.model).u32(d.tok).u32(d.ctx).u32(d.cpu1).u32(d.cpun).u32(d.mem).u32(d.tested);
        }
        o.u8(self.job.is_some().into());
        if let Some(j) = &self.job {
            o.str(&j.name).u8(j.mine.into()).u32(j.total).u32(j.done).u32(j.queued);
            o.u32(j.steals).u32(j.requeued).u32(j.checked).u32(j.mismatched);
            o.u32(j.ms).u32(j.used).u32(j.busy);
            j.per.iter().fold(o.u16(j.per.len() as u16), |o, n| o.u32(*n));
        }
        o.str(&self.serve).str(&self.serving).u8(self.answer.is_some().into());
        if let Some(a) = &self.answer {
            o.str(&a.question).str(&a.by).str(&a.text).u8(a.done.into()).u32(a.tok).str(&a.why);
        }
        o.str(&self.testing);
        o.0
    }

    /// One snapshot, or `None` if the bytes are malformed, trailing or of another version.
    pub fn decode(b: &[u8]) -> Option<Snap> {
        let mut r = Reader(b);
        r.u8().filter(|v| *v == VERSION)?;
        let (at, pairing, code) = (r.u32()?, r.str()?, r.str()?);
        let mut devices = Vec::new();
        for _ in 0..r.u16()? {
            devices.push(Device {
                name: r.str()?,
                kind: r.str()?,
                cores: r.u16()?,
                ram_mb: r.u32()?,
                quota_mb: r.u32()?,
                gpu: r.bool()?,
                up_ms: r.u32()?,
                known: r.bool()?,
                workers: r.u16()?,
                busy: r.u16()?,
                chunks: r.u32()?,
                units: r.u64()?,
                rtt: r.u32()?,
                tx: r.u64()?,
                rx: r.u64()?,
                up: r.u32()?,
                down: r.u32()?,
                model: r.str()?,
                tok: r.u32()?,
                ctx: r.u32()?,
                cpu1: r.u32()?,
                cpun: r.u32()?,
                mem: r.u32()?,
                tested: r.u32()?,
            });
        }
        let job = match r.bool()? {
            false => None,
            true => Some(Job {
                name: r.str()?,
                mine: r.bool()?,
                total: r.u32()?,
                done: r.u32()?,
                queued: r.u32()?,
                steals: r.u32()?,
                requeued: r.u32()?,
                checked: r.u32()?,
                mismatched: r.u32()?,
                ms: r.u32()?,
                used: r.u32()?,
                busy: r.u32()?,
                per: (0..r.u16()?).map(|_| r.u32()).collect::<Option<_>>()?,
            }),
        };
        let (serve, serving) = (r.str()?, r.str()?);
        let answer = match r.bool()? {
            false => None,
            true => Some(Answer {
                question: r.str()?,
                by: r.str()?,
                text: r.str()?,
                done: r.bool()?,
                tok: r.u32()?,
                why: r.str()?,
            }),
        };
        let testing = r.str()?;
        let snap = Snap { at, pairing, code, devices, job, serve, serving, answer, testing };
        r.0.is_empty().then_some(snap)
    }
}
