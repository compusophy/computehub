//! The /home snapshot the desktop keeps in the page's storage: `"CSHM"`, u8 version 1, u8 0, u16
//! 0, u64 seq, the body (u32 count, then entries of u8 kind (0 dir, 1 file), str path relative to
//! /home (sorted, parents first) and, for files, u32 len and the bytes), then a u64 FNV-1a-64 of
//! everything before it. [`take`] makes one; [`give`] reads one whole, then puts it back.

use vfs::{Vfs, VfsError};

use crate::wire::{Reader, Writer};

const MAGIC: &[u8] = b"CSHM";
pub const VERSION: u8 = 1;
const ROOT: &str = "/home";

/// Paths (from /home, or absolute once read back) and, for files, the bytes.
type Entries<'a> = Vec<(String, Option<&'a [u8]>)>;

/// Why a snapshot was not put back: it is not one, or is damaged; a newer OS wrote it; or the
/// filesystem refused an entry ([`VfsError`]), which stops it there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapError {
    Damaged,
    Newer,
    Vfs(VfsError),
}

/// FNV-1a, 64-bit.
pub fn fnv64(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325, |h, &x| (h ^ u64::from(x)).wrapping_mul(0x100_0000_01b3))
}

/// The snapshot of /home in `vfs`, numbered `seq`.
pub fn take(vfs: &Vfs, seq: u64) -> Vec<u8> {
    let mut entries = Vec::new();
    walk(vfs, ROOT, "", &mut entries);
    let head = Writer::default().bytes(MAGIC).u8(VERSION).u8(0).u16(0).u64(seq);
    let mut w = head.u32(entries.len() as u32);
    for (path, data) in entries {
        w = match data {
            None => w.u8(0).str(&path),
            Some(d) => w.u8(1).str(&path).u32(d.len() as u32).bytes(d),
        };
    }
    let sum = fnv64(&w.0);
    w.u64(sum).done()
}

/// The entries under `dir` (`rel` from /home), parents first, each directory's sorted.
fn walk<'a>(vfs: &'a Vfs, dir: &str, rel: &str, out: &mut Entries<'a>) {
    for e in vfs.list(dir).unwrap_or_default() {
        let path = if rel.is_empty() { e.name.clone() } else { [rel, "/", &e.name].concat() };
        let abs = [dir, "/", &e.name].concat();
        if e.is_dir {
            out.push((path.clone(), None));
            walk(vfs, &abs, &path, out);
        } else {
            out.push((path, vfs.read(&abs).ok()));
        }
    }
}

/// Puts the snapshot `b` back into `vfs` once all of it reads (a path that leaves /home is
/// damage); its seq.
pub fn give(vfs: &mut Vfs, b: &[u8]) -> Result<u64, SnapError> {
    let (body, sum) = b.split_at_checked(b.len().wrapping_sub(8)).ok_or(SnapError::Damaged)?;
    if sum.try_into().ok().map(u64::from_le_bytes) != Some(fnv64(body)) {
        return Err(SnapError::Damaged);
    }
    let mut r = Reader(body);
    match (r.take(4), r.u8()) {
        (Some(MAGIC), Some(VERSION)) => {}
        (Some(MAGIC), Some(v)) if v > VERSION => return Err(SnapError::Newer),
        _ => return Err(SnapError::Damaged),
    }
    let (seq, entries) = read(&mut r).ok_or(SnapError::Damaged)?;
    for (path, data) in entries {
        match data {
            None => vfs.mkdir_all(&path),
            Some(d) => vfs.write(&path, d),
        }
        .map_err(SnapError::Vfs)?;
    }
    Ok(seq)
}

/// The seq and entries (absolute paths) after the version, if all of it reads.
fn read<'a>(r: &mut Reader<'a>) -> Option<(u64, Entries<'a>)> {
    let (_, _, seq, n) = (r.u8()?, r.u16()?, r.u64()?, r.u32()?);
    let mut entries = Vec::new();
    for _ in 0..n {
        let kind = r.u8()?;
        let path = Vfs::normalize(ROOT, r.str()?).ok();
        let path = path.filter(|p| p.strip_prefix(ROOT).is_some_and(|r| r.starts_with('/')))?;
        let data = match kind {
            0 => None,
            1 => Some(r.u32().and_then(|n| r.take(n as usize))?),
            _ => return None,
        };
        entries.push((path, data));
    }
    r.end().map(|()| (seq, entries))
}
