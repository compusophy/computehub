//! The /home snapshot homed will keep in IndexedDB (`home.a`, `home.b`): `"CSHM"`, u8 version 1,
//! u8 0, u16 0, u64 seq, the body (u32 count, then entries of u8 kind (0 dir, 1 file), str path
//! relative to /home (sorted, parents first) and, for files, u32 len and the bytes), then a u64
//! FNV-1a-64 of everything before it. Only the hash exists yet.

/// FNV-1a, 64-bit.
pub fn fnv64(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325, |h, &x| (h ^ u64::from(x)).wrapping_mul(0x100_0000_01b3))
}
