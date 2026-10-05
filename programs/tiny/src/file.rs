//! The weights file: a model with its tokenizer and a note, ending in the FNV-1a hash of every
//! byte before it, which is also the file's name for itself. All numbers little-endian:
//!
//! | bytes | what |
//! |---|---|
//! | 4 | `TNY1` |
//! | 4 | the format's version, 1 |
//! | 4 x 5 | vocab, ctx, dim, layers, heads (u32) |
//! | 4 | merges, m (u32); vocab is 257 + m |
//! | 8 x m | each merge's two tokens (u32, u32): token 257 + i is merge i |
//! | 4 + n | the note's length (u32), then the note (UTF-8): what made the weights |
//! | 8 | the parameters, p (u64): the config's count |
//! | 4 x p | the parameters (f32), in the model's order |
//! | 8 | FNV-1a 64 of every byte above (u64) |

use crate::{Config, Error, Model, Tokenizer};

const MAGIC: &[u8; 4] = b"TNY1";
const VERSION: u32 = 1;

/// FNV-1a, 64 bits.
pub fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// `model`, `tok` (whose vocab must be the model's) and `note` as one file.
pub fn save(model: &Model, tok: &Tokenizer, note: &str) -> Result<Vec<u8>, Error> {
    let c = model.cfg;
    if c.check().is_err() || tok.vocab() != c.vocab || model.params.len() != c.params() {
        return Err(Error::Config);
    }
    let mut out = Vec::with_capacity(64 + note.len() + 8 * tok.merges().len() + 4 * c.params());
    out.extend_from_slice(MAGIC);
    for n in [VERSION as usize, c.vocab, c.ctx, c.dim, c.layers, c.heads, tok.merges().len()] {
        out.extend_from_slice(&(n as u32).to_le_bytes());
    }
    for &(a, b) in tok.merges() {
        out.extend_from_slice(&a.to_le_bytes());
        out.extend_from_slice(&b.to_le_bytes());
    }
    out.extend_from_slice(&(note.len() as u32).to_le_bytes());
    out.extend_from_slice(note.as_bytes());
    out.extend_from_slice(&(model.params.len() as u64).to_le_bytes());
    for p in &model.params {
        out.extend_from_slice(&p.to_le_bytes());
    }
    let h = fnv(&out);
    out.extend_from_slice(&h.to_le_bytes());
    Ok(out)
}

/// The model, tokenizer and note in `bytes`, once its hash, size, shape and merges check.
pub fn load(bytes: &[u8]) -> Result<(Model, Tokenizer, String), Error> {
    if bytes.len() < 8 || &bytes[..4] != MAGIC {
        return Err(Error::Magic);
    }
    let (body, tail) = bytes.split_at(bytes.len() - 8);
    let want = u64::from_le_bytes(tail.try_into().map_err(|_| Error::Size)?);
    let got = fnv(body);
    if want != got {
        return Err(Error::Hash { want, got });
    }
    let mut r = Reader { bytes: body, at: 4 };
    let version = r.u32()?;
    if version != VERSION {
        return Err(Error::Version(version));
    }
    let mut n = [0usize; 6];
    for n in &mut n {
        *n = r.u32()? as usize;
    }
    let [vocab, ctx, dim, layers, heads, m] = n;
    let cfg = Config { vocab, ctx, dim, layers, heads };
    cfg.check()?;
    if m.checked_add(257) != Some(vocab) {
        return Err(Error::Config);
    }
    let mut merges = Vec::with_capacity(m);
    for _ in 0..m {
        merges.push((r.u32()?, r.u32()?));
    }
    let tok = Tokenizer::from_merges(merges)?;
    let len = r.u32()? as usize;
    let note = String::from_utf8_lossy(r.take(len)?).into_owned();
    let count = r.take(8)?.try_into().map(u64::from_le_bytes).map_err(|_| Error::Size)?;
    if count != cfg.params() as u64 {
        return Err(Error::Config);
    }
    let raw = r.take(4 * cfg.params())?;
    if r.at != body.len() {
        return Err(Error::Size);
    }
    let params =
        raw.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    Ok((Model { cfg, params }, tok, note))
}

/// A cursor over a file's bytes.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.bytes.len()).ok_or(Error::Size)?;
        let out = &self.bytes[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn u32(&mut self) -> Result<u32, Error> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}
