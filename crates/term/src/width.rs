//! Character widths and the DEC special graphics set.

/// Zero-width ranges, as sorted inclusive `lo, hi` pairs: combining marks,
/// zero-width spaces and joiners, bidi marks, variation selectors, the BOM,
/// emoji skin-tone modifiers (so a toned emoji stays two cells) and tags.
const ZERO: &[u32] = &[
    0x0300, 0x036F, 0x1AB0, 0x1AFF, 0x1DC0, 0x1DFF, 0x200B, 0x200F, 0x2060, 0x2064, 0x20D0, 0x20FF,
    0xFE00, 0xFE0F, 0xFE20, 0xFE2F, 0xFEFF, 0xFEFF, 0x1F3FB, 0x1F3FF, 0xE0000, 0xE007F, 0xE0100,
    0xE01EF,
];

/// Double-width ranges, as sorted inclusive `lo, hi` pairs: East Asian Wide
/// and Fullwidth, and the emoji that are wide by default.
const WIDE: &[u32] = &[
    0x1100, 0x115F, 0x231A, 0x231B, 0x2329, 0x232A, 0x23E9, 0x23EC, 0x23F0, 0x23F0, 0x23F3, 0x23F3,
    0x25FD, 0x25FE, 0x2614, 0x2615, 0x2648, 0x2653, 0x267F, 0x267F, 0x2693, 0x2693, 0x26A1, 0x26A1,
    0x26AA, 0x26AB, 0x26BD, 0x26BE, 0x26C4, 0x26C5, 0x26CE, 0x26CE, 0x26D4, 0x26D4, 0x26EA, 0x26EA,
    0x26F2, 0x26F3, 0x26F5, 0x26F5, 0x26FA, 0x26FA, 0x26FD, 0x26FD, 0x2705, 0x2705, 0x270A, 0x270B,
    0x2728, 0x2728, 0x274C, 0x274C, 0x274E, 0x274E, 0x2753, 0x2755, 0x2757, 0x2757, 0x2795, 0x2797,
    0x27B0, 0x27B0, 0x27BF, 0x27BF, 0x2B1B, 0x2B1C, 0x2B50, 0x2B50, 0x2B55, 0x2B55, 0x2E80, 0x303E,
    0x3041, 0x33FF, 0x3400, 0x4DBF, 0x4E00, 0x9FFF, 0xA000, 0xA4CF, 0xA960, 0xA97F, 0xAC00, 0xD7A3,
    0xF900, 0xFAFF, 0xFE10, 0xFE19, 0xFE30, 0xFE6F, 0xFF00, 0xFF60, 0xFFE0, 0xFFE6, 0x16FE0,
    0x16FE4, 0x17000, 0x18CFF, 0x1AFF0, 0x1B2FF, 0x1F004, 0x1F004, 0x1F0CF, 0x1F0CF, 0x1F18E,
    0x1F18E, 0x1F191, 0x1F19A, 0x1F200, 0x1F2FF, 0x1F300, 0x1F64F, 0x1F680, 0x1F6FF, 0x1F7E0,
    0x1F7EB, 0x1F900, 0x1F9FF, 0x1FA70, 0x1FAFF, 0x20000, 0x2FFFD, 0x30000, 0x3FFFD,
];

/// Whether `u` falls in one of the sorted inclusive pairs of `table`.
fn in_table(table: &[u32], u: u32) -> bool {
    // Past an odd number of bounds means inside a pair; landing on a bound
    // counts too.
    let i = table.partition_point(|&b| b < u);
    i % 2 == 1 || table.get(i) == Some(&u)
}

/// The number of cells `c` takes on screen: 2 for East Asian wide and
/// fullwidth characters and wide emoji, 0 for controls, combining marks,
/// zero-width spaces and joiners, variation selectors and skin-tone
/// modifiers, else 1. Terminal and program must agree on these, or text
/// lays out misaligned; they follow what Ink's `string-width` counts for
/// single code points.
pub fn char_width(c: char) -> u8 {
    let u = u32::from(c);
    if u < 0x7F {
        return u8::from(u >= 0x20);
    }
    if u < 0xA0 {
        return 0;
    }
    if u < 0x300 {
        1
    } else if in_table(ZERO, u) {
        0
    } else {
        1 + u8::from(in_table(WIDE, u))
    }
}

/// DEC special graphics for `_` (0x5F) through `~` (0x7E).
const DEC: [char; 32] = [
    ' ', '◆', '▒', '␉', '␌', '␍', '␊', '°', '±', '␤', '␋', '┘', '┐', '┌', '└', '┼', '⎺', '⎻', '─',
    '⎼', '⎽', '├', '┤', '┴', '┬', '│', '≤', '≥', 'π', '≠', '£', '·',
];

/// `c` in the DEC special graphics set (`ESC ( 0`): line drawing and a few
/// symbols in place of `_` through `~`.
pub(crate) fn dec_graphics(c: char) -> char {
    match u32::from(c) {
        u @ 0x5F..=0x7E => DEC[(u - 0x5F) as usize],
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_sorted_pairs() {
        for t in [ZERO, WIDE] {
            assert_eq!(t.len() % 2, 0);
            assert!(
                t.windows(2).enumerate().all(|(i, w)| w[0] < w[1] || (i % 2 == 0 && w[0] == w[1]))
            );
        }
    }

    #[test]
    fn widths() {
        let text = "a\0\x1b\x7f\u{85}\u{a0}é\u{301}\u{200d}\u{fe0f}─⏺世한\u{3000}\u{303f}\
            \u{ff01}\u{ff61}✅⌚😀🚀🤖\u{1f004}\u{1f005}\u{20000}\u{e0100}👍\u{1f3fd}";
        let want =
            [1, 0, 0, 0, 0, 1, 1, 0, 0, 0, 1, 1, 2, 2, 2, 1, 2, 1, 2, 2, 2, 2, 2, 2, 1, 2, 0, 2, 0];
        assert_eq!(text.chars().map(char_width).collect::<Vec<_>>(), want);
    }
}
