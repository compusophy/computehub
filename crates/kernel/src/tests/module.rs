use super::Rng;
use crate::module::cap_memory;

/// A module of `(id, body)` sections, each under 128 bytes.
fn module(sections: &[(u8, &[u8])]) -> Vec<u8> {
    let body = sections.iter().flat_map(|&(id, b)| [&[id, b.len() as u8][..], b].concat());
    [&b"\0asm\x01\0\0\0"[..], &body.collect::<Vec<u8>>()].concat()
}
/// One memory (count 1), then its limits; 4,096 is `0x80 0x20`.
fn mem(limits: &[u8]) -> Vec<u8> {
    module(&[(1, &[0]), (5, &[&[1], limits].concat()), (10, &[0])])
}

#[test]
fn a_maximum_is_added_or_lowered_to_the_cap_padding_reencoded_and_nothing_else_changed() {
    let capped = mem(&[1, 2, 0x80, 0x20]);
    assert_eq!(cap_memory(&mem(&[0, 2]), 4_096), Ok(capped.clone()));
    assert_eq!(cap_memory(&mem(&[1, 2, 0xFF, 0xFF, 3]), 4_096), Ok(capped.clone()));
    assert_eq!(cap_memory(&capped, 4_096), Ok(capped));
    assert_eq!(cap_memory(&mem(&[1, 0, 3]), 4_096), Ok(mem(&[1, 0, 3])));
    assert_eq!(cap_memory(&mem(&[0, 1]), 1), Ok(mem(&[1, 1, 1])));
    // No memory at all is left alone (cpu then refuses it: no `memory`).
    assert_eq!(cap_memory(&module(&[(1, &[0])]), 4_096), Ok(module(&[(1, &[0])])));
    // A custom section with a padded size, then a padded size, min (2) and max (5).
    let custom = [&b"\0asm\x01\0\0\0"[..], &[0, 0x84, 0x80, 0x80, 0x80, 0, 1, b'x', 0, 0]].concat();
    let pad = |n: u8| [n | 0x80, 0x80, 0x80, 0x80, 0];
    let memory = [&[5][..], &pad(12), &[1, 1], &pad(2), &pad(5)].concat();
    let out = cap_memory(&[custom.clone(), memory].concat(), 4_096).unwrap();
    assert_eq!(out, [&custom[..], &[5, 4, 1, 1, 2, 5]].concat());
    assert_eq!(cap_memory(&out, 4_096).as_ref(), Ok(&out));
    assert_eq!(cap_memory(&out, 3), Ok([&custom[..], &[5, 4, 1, 1, 2, 3]].concat()));
}

#[test]
fn shared_64_bit_oversized_imported_memories_non_modules_and_malformed_sections_are_refused() {
    let refused = |wasm: &[u8], why| assert_eq!(cap_memory(wasm, 4_096), Err(why), "{wasm:?}");
    refused(&mem(&[3, 1, 2]), "shared memory is not supported");
    refused(&mem(&[2, 1]), "shared memory is not supported");
    refused(&mem(&[4, 1]), "64-bit memory is not supported");
    refused(&mem(&[5, 1, 2]), "64-bit memory is not supported");
    refused(&mem(&[8, 1]), "malformed wasm module");
    refused(&mem(&[0, 0x81, 0x20]), "the program needs more memory than a process may have");
    let import = |kind: &[u8]| module(&[(2, &[&[1, 1, b'e', 1, b'm'], kind].concat())]);
    let only = "a program may import only functions";
    for kind in [&[1, 0x70, 0, 0][..], &[2, 0, 1], &[3, 0x7F, 0], &[4, 0, 0]] {
        refused(&import(kind), only);
    }
    let funcs = module(&[(2, &[2, 1, b'e', 1, b'f', 0, 0, 1, b'e', 1, b'g', 0, 0x80, 0])]);
    assert_eq!(cap_memory(&funcs, 4_096), Ok(funcs));
    for wasm in [&b""[..], b"#!wasm bin/x.wasm\n", b"\0asm\x0d\0\x01\0", b"\0asm\x01\0\0"] {
        assert_eq!(cap_memory(wasm, 4_096), Err("not a wasm module"));
    }
    let whole = mem(&[1, 2, 3]);
    for n in 9..whole.len() {
        // Cut inside a section; cuts at a section's end leave a valid prefix.
        let malformed = cap_memory(&whole[..n], 4_096) == Err("malformed wasm module");
        assert!(malformed != [11, 17].contains(&n), "{n}");
    }
    let bad = |s: &[u8]| cap_memory(&module(&[(5, s)]), 4_096).err();
    let malformed = Some("malformed wasm module");
    assert_eq!([bad(&[1, 0, 1, 9]), bad(&[2, 0, 1]), bad(&[1, 1, 1])], [malformed; 3]);
    let leb = [&b"\0asm\x01\0\0\0"[..], &[1, 0x80, 0x80, 0x80, 0x80, 0x10]].concat();
    assert_eq!(cap_memory(&leb, 4_096).err(), malformed);
}

#[test]
fn random_and_mutated_modules_never_panic() {
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let valid = mem(&[1, 2, 0x80, 0x20]);
    for i in 0..10_000 {
        let mut b = valid.clone();
        if i % 2 == 0 {
            let len = (rng.next() % 32) as usize;
            b = [&b[..8], &rng.bytes(len)[..]].concat();
        } else {
            let at = rng.next() as usize % b.len();
            b[at] = rng.next() as u8;
        }
        if let Ok(out) = cap_memory(&b, 4_096) {
            assert_eq!(cap_memory(&out, 4_096).as_ref(), Ok(&out), "{b:?}");
        }
    }
}
