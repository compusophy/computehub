//! A device measured, not reported: the work the pool's "Test this device" gives its workers
//! (`/bin/gauge work`). The browser's own figures are guesses (Firefox gives no memory at all,
//! Chrome rounds it and caps it), so the pool times real work instead:
//!
//! - **CPU**: SHA-256, one block at a time, each block holding the last hash ([`cpu`]): integer
//!   work as the pool's real jobs are, deterministic, so every device does the same and any peer
//!   can check the answer, and nothing large allocated (big buffers copied every round scaled
//!   badly across workers). Its speed is mebibytes' worth of blocks hashed a second (16,384 a
//!   mebibyte), on one core, then on all at once.
//! - **Memory**: mebibytes taken in steps and written through, every byte, with bytes no memory
//!   compressor can shrink, so they are really held ([`Held::take`]); a step past its limit stops
//!   writing at once. The pool steps across workers, never past its cap, and stops at a refusal,
//!   a slow step (a device short of memory swaps, and swapping shows as time), or its timers.
//!
//! A worker reads lines `<index> cpu <mib>`, `<index> ram <mib> <limit ms>` and answers each with
//! one line `<index> <mib> <hash> <result>` ([`answer`]), as a pool worker answers a chunk; the
//! result is `ok` (`no`, `slow`) and the milliseconds the work took, timed by the worker itself,
//! so no delay in carrying lines counts.

#![forbid(unsafe_code)]

/// A mebibyte.
pub const MIB: usize = 1 << 20;

/// SHA-256 blocks in a mebibyte.
pub const BLOCKS: u32 = (MIB / 64) as u32;

/// `mib` mebibytes' worth of SHA-256 blocks for chunk `index`: each the hash of a 40-byte
/// message (the last hash, the chunk, the block's number), one block once padded, compressed in
/// place (`sha::compress`: nothing allocated, as `sha::sha256` would for every block, which
/// scaled badly across workers). The last hash, in hex.
pub fn cpu(index: u32, mib: u32) -> String {
    let mut block = [0u8; 64];
    block[32..36].copy_from_slice(&index.to_le_bytes());
    // The padding of a 40-byte message: a one bit, then its length in bits.
    block[40] = 0x80;
    block[56..].copy_from_slice(&(40u64 * 8).to_be_bytes());
    for n in 0..mib.saturating_mul(BLOCKS) {
        block[36..40].copy_from_slice(&n.to_le_bytes());
        let mut h = sha::INIT;
        sha::compress(&mut h, &block);
        block[..32].copy_from_slice(&sha::bytes(&h));
    }
    sha::hex(&block[..32])
}

/// Words of eight bytes in a mebibyte.
const WORDS: usize = MIB / 8;

/// How a step of memory went: taken, in ms; refused; or stopped past its limit, at ms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Took {
    Ok(u64),
    No,
    Slow(u64),
}

/// Memory taken and written through, in blocks, and the xorshift state its bytes come from.
#[derive(Debug, Default)]
pub struct Held {
    blocks: Vec<Vec<u64>>,
    x: u64,
}

impl Held {
    /// Takes `mib` more mebibytes and writes every byte, a mebibyte at a time, with xorshift
    /// words (a page of one repeated byte, compressed or deduplicated by the system, would not
    /// be held at all); stops writing once `limit` ms have passed (what it wrote stays held
    /// until the worker ends). It never aborts: a failed reservation is a refusal, not a crash.
    pub fn take(&mut self, mib: u32, limit: u64) -> Took {
        let t0 = std::time::Instant::now();
        let mut block: Vec<u64> = Vec::new();
        if block.try_reserve_exact(mib as usize * WORDS).is_err() {
            return Took::No;
        }
        let mut x = self.x | 1;
        let mut took = Took::Ok(0);
        for _ in 0..mib {
            for _ in 0..WORDS {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                block.push(x);
            }
            let ms = t0.elapsed().as_millis() as u64;
            took = if ms >= limit { Took::Slow(ms) } else { Took::Ok(ms) };
            if ms >= limit {
                break;
            }
        }
        self.x = x;
        self.blocks.push(block);
        took
    }

    /// Mebibytes held.
    pub fn mib(&self) -> usize {
        self.blocks.iter().map(Vec::len).sum::<usize>() / WORDS
    }
}

/// The answer to one line: `<index> <mib> <hash> <result>` (for memory, the mebibytes held now,
/// `-`, and `ok`, `no` or `slow`, with ms but for `no`); `None` for a line that is not a request.
pub fn answer(line: &str, held: &mut Held) -> Option<String> {
    let mut words = line.split_whitespace();
    let index: u32 = words.next()?.parse().ok()?;
    let (what, mib): (&str, u32) = (words.next()?, words.next()?.parse().ok()?);
    let mib = mib.min(1024);
    Some(match what {
        "cpu" => {
            let t0 = std::time::Instant::now();
            let hash = cpu(index, mib);
            let ms = t0.elapsed().as_millis().max(1).to_string();
            [&index.to_string(), " ", &mib.to_string(), " ", &hash, " ok ", &ms].concat()
        }
        "ram" => {
            let limit = words.next().and_then(|w| w.parse().ok()).unwrap_or(u64::MAX);
            let result = match held.take(mib, limit) {
                Took::Ok(ms) => [" - ok ", &ms.max(1).to_string()].concat(),
                Took::No => " - no".into(),
                Took::Slow(ms) => [" - slow ", &ms.to_string()].concat(),
            };
            [&index.to_string(), " ", &held.mib().to_string(), &result].concat()
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
