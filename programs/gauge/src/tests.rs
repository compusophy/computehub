use super::*;

#[test]
fn the_cpu_work_is_sha_256_of_each_message_as_the_sha_crate_hashes_it() {
    // The same chain through sha::sha256, which allocates: the hashes must agree.
    let mut msg = [0u8; 40];
    msg[32..36].copy_from_slice(&7u32.to_le_bytes());
    for n in 0..BLOCKS * 2 {
        msg[36..].copy_from_slice(&n.to_le_bytes());
        let h = sha::sha256(&msg);
        msg[..32].copy_from_slice(&h);
    }
    assert_eq!(cpu(7, 2), sha::hex(&msg[..32]));
}

#[test]
fn the_cpu_work_is_the_same_everywhere_and_depends_on_its_chunk_and_size() {
    let a = cpu(3, 2);
    assert_eq!(a.len(), 64);
    assert_eq!(a, cpu(3, 2), "deterministic");
    assert_ne!(a, cpu(4, 2));
    assert_ne!(a, cpu(3, 3));
}

#[test]
fn memory_is_held_written_through_and_answered() {
    let mut held = Held::default();
    let a = answer("7 ram 2 60000", &mut held).unwrap();
    assert!(a.starts_with("7 2 - ok "), "{a}");
    let b = answer("8 ram 3", &mut held).unwrap();
    assert!(b.starts_with("8 5 - ok "), "{b}");
    assert_eq!(held.mib(), 5);
    // Written with bytes nothing compresses: no two words alike in a block, none zero.
    let words = &held.blocks[0];
    assert_eq!(words.len(), 2 * WORDS);
    assert!(words.iter().all(|&w| w != 0) && words[0] != words[1] && words[1] != words[2]);
    // Past its limit, a step stops writing: a mebibyte held, not the many asked.
    let mut slow = Held::default();
    assert!(matches!(slow.take(4, 0), Took::Slow(_)));
    assert_eq!(slow.mib(), 1);
    assert_eq!(
        answer("3 ram 4 0", &mut slow).as_deref().map(|a| a.get(..11)),
        Some(Some("3 2 - slow "))
    );
    let cpu_line = answer("9 cpu 1", &mut held).unwrap();
    assert!(cpu_line.starts_with("9 1 ") && cpu_line.contains(" ok "), "{cpu_line}");
    for bad in ["", "x cpu 1", "1 gpu 2", "1 cpu", "1 cpu many"] {
        assert_eq!(answer(bad, &mut held), None, "{bad}");
    }
}
