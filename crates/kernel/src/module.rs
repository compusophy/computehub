//! Loader-side rewrites of a guest module, before it compiles.

use crate::wire::Reader;

const BAD: &str = "malformed wasm module";

/// `wasm` with each memory's maximum set to at most `max_pages` (added when
/// absent, lowered when above); only the memory section changes. Refuses
/// what is not a wasm 1 module; a malformed section header, import or memory
/// section; an import of anything but a function (an imported memory would
/// escape the cap); and a shared or 64-bit memory, or one whose minimum is
/// above `max_pages`. The browser validates the rest.
pub fn cap_memory(wasm: &[u8], max_pages: u32) -> Result<Vec<u8>, &'static str> {
    let (mut r, mut out) = (Reader(wasm), b"\0asm\x01\0\0\0".to_vec());
    if r.take(8) != Some(&out[..]) {
        return Err("not a wasm module");
    }
    while let Some(id) = r.u8() {
        let at = wasm.len() - r.0.len() - 1;
        let body = Reader(bytes(&mut r).ok_or(BAD)?);
        match id {
            2 => imports(body)?,
            5 => {
                let body = memories(body, max_pages)?;
                out.push(5);
                leb_to(&mut out, body.len() as u32);
                out.extend(body);
                continue;
            }
            _ => {}
        }
        out.extend_from_slice(&wasm[at..wasm.len() - r.0.len()]);
    }
    Ok(out)
}

/// Checks an import section: functions only.
fn imports(mut r: Reader<'_>) -> Result<(), &'static str> {
    for _ in 0..leb(&mut r).ok_or(BAD)? {
        match bytes(&mut r).and(bytes(&mut r)).and(r.u8()).ok_or(BAD)? {
            0 => leb(&mut r).ok_or(BAD)?,
            _ => return Err("a program may import only functions"),
        };
    }
    r.end().ok_or(BAD)
}

/// A memory section with every maximum capped at `cap`.
fn memories(mut r: Reader<'_>, cap: u32) -> Result<Vec<u8>, &'static str> {
    let (n, mut out) = (leb(&mut r).ok_or(BAD)?, Vec::new());
    leb_to(&mut out, n);
    for _ in 0..n {
        match r.u8().ok_or(BAD)? {
            f if f & 2 != 0 => return Err("shared memory is not supported"),
            f if f & 4 != 0 => return Err("64-bit memory is not supported"),
            f @ (0 | 1) => {
                let min = leb(&mut r).ok_or(BAD)?;
                let max = if f == 1 { leb(&mut r).ok_or(BAD)? } else { cap };
                if min > cap {
                    return Err("the program needs more memory than a process may have");
                }
                out.push(1);
                leb_to(&mut out, min);
                leb_to(&mut out, max.min(cap));
            }
            _ => return Err(BAD),
        }
    }
    r.end().ok_or(BAD).map(|()| out)
}

/// A LEB128 length, then that many bytes.
fn bytes<'a>(r: &mut Reader<'a>) -> Option<&'a [u8]> {
    let n = leb(r)?;
    r.take(n as usize)
}

/// An unsigned LEB128 u32: at most 5 bytes, padding allowed, no bits past 32.
fn leb(r: &mut Reader<'_>) -> Option<u32> {
    let mut v = 0;
    for shift in [0, 7, 14, 21, 28] {
        let b = r.u8()?;
        v |= u32::from(b & 0x7F) << shift;
        if b < 0x80 {
            return (shift < 28 || b < 0x10).then_some(v);
        }
    }
    None
}

fn leb_to(out: &mut Vec<u8>, mut v: u32) {
    while v > 0x7F {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}
