//! A device measured, not reported: the work the pool's "Test this device" gives its workers
//! (`/bin/gauge work`). The browser's own figures are guesses (Firefox gives no memory at all,
//! Chrome rounds it and caps it), so the pool times real work instead:
//!
//! - **CPU**: SHA-256, one block at a time, each block holding the last hash ([`cpu`]): integer
//!   work as the pool's real jobs are, deterministic, so every device does the same and any peer
//!   can check the answer, and nothing large allocated (big buffers copied every round scaled
//!   badly across workers). Its speed is mebibytes' worth of blocks hashed a second (16,384 a
//!   mebibyte), on one core, then on all at once.
//! - **Memory**: mebibytes taken in steps and written through, every byte, so they are really
//!   held ([`Held::take`]), until a step fails; the pool steps across workers, times each step
//!   and stops at a failure, a step that slows as a swapping device's does, or its ceiling.
//!
//! A worker reads lines `<index> cpu <mib>`, `<index> ram <mib>` and answers each with one line
//! `<index> <mib> <hash> <result>` ([`answer`]), as a pool worker answers a chunk; a CPU answer's
//! result is `ok` and the milliseconds the work took, timed by the worker itself, so no delay in
//! carrying lines counts.

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

/// Memory taken and written through, in blocks.
#[derive(Debug, Default)]
pub struct Held {
    blocks: Vec<Vec<u8>>,
}

impl Held {
    /// Takes `mib` more mebibytes and writes every byte; whether it could (it never aborts: a
    /// failed reservation is a refusal, not a crash).
    pub fn take(&mut self, mib: u32) -> bool {
        let n = mib as usize * MIB;
        let mut block: Vec<u8> = Vec::new();
        if block.try_reserve_exact(n).is_err() {
            return false;
        }
        block.resize(n, 0xA5);
        self.blocks.push(block);
        true
    }

    /// Mebibytes held.
    pub fn mib(&self) -> usize {
        self.blocks.iter().map(Vec::len).sum::<usize>() / MIB
    }
}

/// The answer to one line: `<index> <mib> <hash> <result>` (for memory, the mebibytes held now,
/// `-`, and `ok` or `no`); `None` for a line that is not a request.
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
            let took = held.take(mib);
            let result = if took { " - ok" } else { " - no" };
            [&index.to_string(), " ", &held.mib().to_string(), result].concat()
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
