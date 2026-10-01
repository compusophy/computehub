use crate::snap::*;

#[test]
fn fnv64_matches_the_reference_vectors() {
    assert_eq!(fnv64(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(fnv64(b"a"), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(fnv64(b"foobar"), 0x8594_4171_f739_67e8);
}
