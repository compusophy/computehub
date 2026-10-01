//! One file per part of the kernel, so each has one owner.

mod module;
mod shared;
mod snap;
mod wasi;
mod wire;

/// A deterministic xorshift64 for fuzzing; never 0 from a nonzero seed.
pub(crate) struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// `len` random bytes.
    pub fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.next() as u8).collect()
    }
}
