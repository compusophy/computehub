use super::*;
use Color::{Default as D, Indexed as I, Rgb};

fn term(cols: u16, rows: u16, bytes: &str) -> Term {
    let mut t = Term::new(cols, rows);
    t.feed(bytes.as_bytes());
    t
}

/// Feeds `bytes` to `t` and lends it back, for a check.
fn fed(t: &mut Term, bytes: impl AsRef<[u8]>) -> &mut Term {
    t.feed(bytes.as_ref());
    t
}

/// Resizes `t` and lends it back.
fn sized(t: &mut Term, cols: u16, rows: u16) -> &mut Term {
    t.resize(cols, rows);
    t
}

/// A row's text, wide tails skipped and trailing spaces trimmed.
fn text(line: &[Cell]) -> String {
    let s: String = line.iter().filter(|c| c.width != 0).map(|c| c.ch).collect();
    s.trim_end().to_string()
}

/// The screen as text, rows joined with `|`.
fn screen(t: &Term) -> String {
    (0..t.rows()).map(|r| text(t.row(r))).collect::<Vec<_>>().join("|")
}

/// The screen after `bytes` in a new terminal.
fn run(cols: u16, rows: u16, bytes: &str) -> String {
    screen(&term(cols, rows, bytes))
}

fn check(t: &Term, want: &str, cursor: (u16, u16)) {
    assert_eq!((screen(t).as_str(), t.cursor()), (want, cursor));
}

/// Every wide head is followed by its tail, and every tail follows a head.
fn wide_ok(line: &[Cell]) {
    for (i, c) in line.iter().enumerate() {
        match c.width {
            2 => assert_eq!(line.get(i + 1).map(|c| c.width), Some(0), "{line:?}"),
            0 => assert_eq!(i.checked_sub(1).map(|j| line[j].width), Some(2), "{line:?}"),
            w => assert_eq!(w, 1),
        }
    }
}

#[test]
fn screens_cursors_and_wrap() {
    #[rustfmt::skip]
    let screens: &[(u16, u16, &str, &str)] = &[
        // Pending wrap: cleared by motion, kept by DECSC; no DECAWM overwrites.
        (5, 3, "abcde\r\nx", "abcde|x|"), (5, 3, "abcde\x08X", "abcXe||"),
        (5, 3, "abcde\x1b[GX", "Xbcde||"), (5, 3, "abcde\x1b[KX", "abcdX||"),
        (3, 2, "abc\x1b7\x1b[H\x1b8d", "abc|d"), (5, 3, "\x1b[?7labcdefg", "abcdg||"),
        // DEC line drawing, in G0 or G1; RIS and DECSTR go back to ASCII.
        (10, 1, "\x1b(0lqwqk\x1b(Bq", "┌─┬─┐q"), (10, 1, "\x1b)0a\x0ex\x0fx", "a│x"),
        (40, 1, "\x1b(0`abcdefghijklmnopqrstuvwxyz{|}~_AZ", "◆▒␉␌␍␊°±␤␋┘┐┌└┼⎺⎻─⎼⎽├┤┴┬│≤≥π≠£· AZ"),
        (5, 1, "\x1b(0\x1bcq", "q"), (5, 1, "\x1b(0\x1b[!pq", "q"),
        // Wide characters wrap whole; overwriting a half blanks the other.
        (5, 2, "\x1b[?7labcd世", "abcd|"), (1, 2, "世x", "|x"), (5, 1, "世界\x1b[2Gx", " x界"),
        (5, 1, "世界\x1b[Gx", "x 界"), (5, 1, "a世界\x1b[3G世", "a 世"),
        (9, 1, "e\u{301}\u{200d}\u{fe0f}x\u{85}", "ex"), (9, 1, "😀🚀x", "😀🚀x"),
        (6, 1, "abcd\x1b[G\x1b[4h世", "世abcd"),
        // Insert mode, REP (after a printed character, to the line's end), DECALN.
        (5, 1, "abc\x1b[G\x1b[4hXY\x1b[4lZ", "XYZbc"), (9, 1, "ab\x1b[3b", "abbbb"),
        (9, 1, "a\r\x1b[3b", "a"), (10, 3, "ab\x1b[65535b\x1b[9b", "abbbbbbbbb|b|"),
        (3, 2, "\x1b[2;2H\x1b#8", "EEE|EEE"),
    ];
    for &(cols, rows, bytes, want) in screens {
        assert_eq!(run(cols, rows, bytes), want, "{bytes:?}");
    }
    #[rustfmt::skip]
    let cursors: &[(u16, u16, &str, (u16, u16))] = &[
        // Movement clamps to the screen.
        (8, 3, "\x1b[99;99H", (2, 7)), (8, 3, "\x1b[99C\x1b[2D\x1b[9B\x1b[A", (1, 5)),
        (8, 3, "\x1b[3;5H\x1b[F", (1, 0)), (8, 3, "\x1b[5E", (2, 0)),
        (8, 3, "\x1b[2d\x1b[4`", (1, 3)), (8, 3, "\x1b[;5H\x1b[3a\x1b[e", (1, 7)),
        (5, 3, "ab\x1b[2J", (0, 2)), (10, 3, "\x1b[2;4H\x1b[s\x1b[H\x1b[u", (1, 3)),
        (9, 1, "😀🚀x", (0, 5)),
        // A region homes the cursor and stops CUU and CUD; IL goes to column 0.
        (3, 5, "\x1b[5;5Hx\x1b[2;4r", (0, 0)), (3, 5, "\x1b[2;4r\x1b[3H\x1b[9A", (1, 0)),
        (3, 5, "\x1b[2;4r\x1b[3H\x1b[9B", (3, 0)), (3, 5, "\x1b[2;4r\x1b[5H\x1b[9A", (1, 0)),
        (3, 5, "\x1b[2;4r\x1b[H\x1b[9A", (0, 0)), (3, 5, "\x1b[2;4r\x1b[3;3H\x1b[L", (2, 0)),
        // Tabs: default stops every 8, HTS, TBC, CBT and CHT.
        (20, 1, "\t\t\t", (0, 19)), (20, 1, "\x1b[4G\x1bH\x1b[G\t", (0, 3)),
        (20, 1, "\x1b[3g\t", (0, 19)), (20, 1, "\x1b[9G\x1b[g\x1b[G\t", (0, 16)),
        (20, 1, "\x1b[20G\x1b[Z", (0, 16)), (20, 1, "\x1b[20G\x1b[2Z", (0, 8)),
        (20, 1, "\x1b[20G\x1b[9Z", (0, 0)), (20, 1, "\x1b[2I", (0, 16)),
    ];
    for &(cols, rows, bytes, want) in cursors {
        assert_eq!(term(cols, rows, bytes).cursor(), want, "{bytes:?}");
    }
    // Split feeds, down to single bytes, match one feed.
    let bytes = "a\x1b[1;38:2::1:2:3m世\x1b]0;t\x07\u{1f600}\x1b[2;3Hq";
    let (whole, mut t) = (term(10, 3, bytes), Term::new(10, 3));
    bytes.as_bytes().iter().for_each(|b| t.feed(&[*b]));
    check(&t, &screen(&whole), whole.cursor());
    assert_eq!((t.row(0), t.title()), (whole.row(0), "t"));
    let mut t = term(5, 3, "abcde");
    check(&t, "abcde||", (0, 4));
    check(fed(&mut t, "f"), "abcde|f|", (1, 1));
    // Wrapping at the bottom scrolls into the scrollback.
    let t = term(3, 2, "abcdefg");
    assert_eq!((screen(&t).as_str(), text(t.scrollback_row(0)).as_str()), ("def|g", "abc"));
    // DSR reports the last column while the flag is pending.
    assert_eq!(term(5, 3, "abcde\x1b[6n").take_replies(), b"\x1b[1;5R");
    let t = term(5, 2, "abcd世");
    check(&t, "abcd|世", (1, 2));
    assert_eq!((t.row(1)[0].width, t.row(1)[1].width), (2, 0));
    let t = term(4, 1, "ab世");
    assert_eq!((t.cursor(), t.row(0)[2].ch), ((0, 3), '世'));
    wide_ok(term(5, 1, "世界\x1b[2G世").row(0));
    let t = term(20, 1, "\tx");
    assert_eq!((t.cursor(), t.row(0)[8].ch), ((0, 9), 'x'));
}

#[test]
fn scroll_regions_and_erase() {
    let region = |extra: &str| run(3, 5, &format!("a\r\nb\r\nc\r\nd\r\ne\x1b[2;4r{extra}"));
    // IL, DL, LF and RI outside the region, or a bad region, change nothing. In origin mode rows
    // count from the top margin and stay in the region.
    #[rustfmt::skip]
    let cases = [
        ("\x1b[4H\n", "a|c|d||e"), ("\x1b[4H\x1bD", "a|c|d||e"), ("\x1b[4H\x1bE", "a|c|d||e"),
        ("\x1b[2H\x1b[M", "a|c|d||e"), ("\x1b[S", "a|c|d||e"), ("\x1b[2H\x1bM", "a||b|c|e"),
        ("\x1b[3H\x1b[L", "a|b||c|e"), ("\x1b[3H\x1b[9L", "a|b|||e"), ("\x1b[2T", "a|||b|e"),
        ("\x1b[2H\x1b[2M", "a|d|||e"), ("\x1b[5H\x1b[L\x1b[M\n", "a|b|c|d|e"),
        ("\x1b[1H\x1bM", "a|b|c|d|e"), ("\x1b[3;3r\x1b[4;2r\x1b[5H\n", "a|b|c|d|e"),
        ("\x1b[?6h\x1b[1;1HX", "a|X|c|d|e"), ("\x1b[?6h\x1b[9;1HX", "a|b|c|X|e"),
    ];
    for (extra, want) in cases {
        assert_eq!(region(extra), want, "{extra:?}");
    }
    let mut t = term(3, 5, "\x1b[2;4r\x1b[?6h\x1b[2;2H\x1b[6n");
    assert_eq!(t.take_replies(), b"\x1b[2;2R");
    // Only a region starting at the top feeds the scrollback.
    assert_eq!(term(3, 5, "\x1b[2;5r\x1b[5H\n\n").scrollback_len(), 0);
    assert_eq!(term(3, 5, "\x1b[1;3r\x1b[3H\n\n").scrollback_len(), 2);
    let t = |extra: &str| run(5, 3, &format!("abcde\r\nfghij\r\nklmno{extra}"));
    assert_eq!(t("\x1b[2;3H\x1b[K"), "abcde|fg|klmno");
    assert_eq!(t("\x1b[2;3H\x1b[1K"), "abcde|   ij|klmno");
    assert_eq!(t("\x1b[2;3H\x1b[2K"), "abcde||klmno");
    assert_eq!(t("\x1b[2;3H\x1b[J"), "abcde|fg|");
    assert_eq!(t("\x1b[2;3H\x1b[1J"), "|   ij|klmno");
    assert_eq!(t("\x1b[2J"), "||");
    assert_eq!(t("\x1b[2;2H\x1b[2X"), "abcde|f  ij|klmno");
    assert_eq!(t("\x1b[2;2H\x1b[99X"), "abcde|f|klmno");
    assert_eq!(t("\x1b[2;2H\x1b[2P"), "abcde|fij|klmno");
    assert_eq!(t("\x1b[2;2H\x1b[2@"), "abcde|f  gh|klmno");
    assert_eq!(t("\x1b[2;2H\x1b[99@"), "abcde|f|klmno");
    // Erased cells take the background color (bce), not other attributes.
    let c = term(5, 3, "ab\x1b[1;31;42m\x1b[2K").row(0)[0];
    assert_eq!((c.ch, c.fg, c.bg, c.attrs), (' ', D, I(2), Attrs(0)));
    // Erasing half of a wide character blanks all of it.
    let t = |extra: &str| run(6, 1, &format!("a世b{extra}"));
    assert_eq!(t("\x1b[3G\x1b[1K"), "   b");
    assert_eq!(t("\x1b[2G\x1b[K"), "a");
    assert_eq!(t("\x1b[3G\x1b[X"), "a  b");
    assert_eq!(t("\x1b[3G\x1b[P"), "a b");
    assert_eq!(t("\x1b[3G\x1b[@"), "a   b");
    wide_ok(term(4, 1, "a世b\x1b[G\x1b[@").row(0));
}

#[test]
fn sgr() {
    let mut t = Term::new(20, 1);
    t.feed(b"\x1b[1;2;3;4;5;7;8;9mA\x1b[22;23;24;25;27;28;29mB\x1b[31;42mC\x1b[91;102mD");
    t.feed(b"\x1b[38;5;196;48;2;1;2;3mE\x1b[38:2::10:20:30;48:5:7mF\x1b[38:2:1:2:3;4:3mG");
    t.feed(b"\x1b[4:0;39;49mH\x1b[0mI\x1b[21mJ\x1b[mK\x1b[58;2;1;2;3;1mL\x1b[;4mM");
    t.feed(b"\x1b[38;5;300;38:2::1:2mN\x1b[>4;2mO\x1b[1;38;5mP");
    let (none, u, b) = (Attrs(0), Attrs::UNDERLINE, Attrs::BOLD);
    // L: 58 takes its arguments; M: empty is 0; N: bad colors are ignored.
    #[rustfmt::skip]
    let expect = [
        ('A', D, D, Attrs(0xFF)), ('B', D, D, none), ('C', I(1), I(2), none),
        ('D', I(9), I(10), none), ('E', I(196), Rgb(1, 2, 3), none),
        ('F', Rgb(10, 20, 30), I(7), none), ('G', Rgb(1, 2, 3), I(7), u), ('H', D, D, none),
        ('I', D, D, none), ('J', D, D, u), ('K', D, D, none), ('L', D, D, b), ('M', D, D, u),
        ('N', Rgb(0, 1, 2), D, u), ('O', Rgb(0, 1, 2), D, u), ('P', Rgb(0, 1, 2), D, u | b),
    ];
    for (i, (ch, fg, bg, attrs)) in expect.into_iter().enumerate() {
        let c = t.row(0)[i];
        assert_eq!((c.ch, c.fg, c.bg, c.attrs), (ch, fg, bg, attrs), "{ch}");
    }
    assert!((u | b).contains(b) && !u.contains(u | b));
}

#[test]
fn alternate_screen_and_saved_cursor() {
    let mut t = term(10, 3, "main\x1b[2;3H\x1b[?1049h");
    assert!(t.alt_screen());
    check(&t, "||", (1, 2));
    assert_eq!(fed(&mut t, "\x1b[Halt\r\n\n\n\n\x1b[3;9H").scrollback_len(), 0);
    check(fed(&mut t, "\x1b[?1049l"), "main||", (1, 2));
    assert!(!t.alt_screen());
    // Entering again clears it.
    assert_eq!(screen(fed(&mut t, "\x1b[?1049hx\x1b[?1049l\x1b[?1049h")), "||");
    check(fed(&mut t, "\x1b[?1049l\x1b[?1049l"), "main||", (1, 2));
    // 47 keeps the alternate screen's content; 1047 clears it on the way out.
    assert_eq!(screen(fed(&mut t, "\x1b[?47hkeep\x1b[?47l\x1b[?47h")), "|  keep|");
    assert_eq!(screen(fed(&mut t, "\x1b[?1047l\x1b[?1047h")), "||");
    t.feed(b"\x1b[?1047l\x1b[?1048h\x1b[3;3H\x1b[?1048l");
    assert_eq!((t.alt_screen(), t.cursor()), (false, (1, 6)));
    // DECSC saves the pen and the charsets; without a save, DECRC homes.
    let t = term(10, 3, "\x1b[2;4H\x1b[1;31m\x1b(0\x1b7\x1b[H\x1b[0m\x1b(Bq\x1b8q");
    let (a, b) = (t.row(0)[0], t.row(1)[3]);
    assert_eq!((a.ch, a.fg, a.attrs), ('q', D, Attrs(0)));
    assert_eq!((b.ch, b.fg, b.attrs, t.cursor()), ('─', I(1), Attrs::BOLD, (1, 4)));
    let t = term(10, 3, "\x1b[2;4H\x1b[7m\x1b8x");
    assert_eq!((t.cursor(), t.row(0)[0].attrs), ((0, 1), Attrs(0)));
    // A saved position past a shrink is clamped.
    let mut t = term(10, 3, "\x1b[3;9H\x1b7");
    assert_eq!(fed(sized(&mut t, 4, 2), "\x1b8").cursor(), (1, 3));
}

#[test]
fn replies_and_title() {
    let mut t = Term::new(10, 5);
    t.feed(b"\x1b[5n\x1b[3;4H\x1b[6n\x1b[c\x1b[0c\x1b[>c\x1b[?2026$p\x1b[?2026h\x1b[?2026$p");
    t.feed(b"\x1b[4$p\x1b[?9999$p\x1b[?1049$p\x1b[1c\x1b[7n");
    let want: &[u8] = b"\x1b[0n\x1b[3;4R\x1b[?62;22c\x1b[?62;22c\x1b[>0;10;1c\x1b[?2026;2$y\
        \x1b[?2026;1$y\x1b[4;2$y\x1b[?9999;0$y\x1b[?1049;2$y";
    assert_eq!(t.take_replies(), want);
    assert!(t.take_replies().is_empty() && t.sync);
    // Queries alone are not a visible change.
    let g = t.generation();
    assert_eq!(fed(&mut t, "\x1b[6n\x1b[c\x1b]11;?\x07\x1b[18t").generation(), g);
    assert_eq!(t.take_replies(), b"\x1b[3;4R\x1b[?62;22c");
    assert_eq!(fed(&mut t, "x").generation(), g + 1);
    // Unread replies are capped.
    assert!(fed(&mut t, "\x1b[6n".repeat(20_000)).take_replies().len() <= 1 << 16);
    // Titles lose controls, stop at 256 chars and survive RIS; other OSCs do nothing.
    let mut t = term(5, 1, "\x1b]0;hi\x07");
    assert_eq!(t.title(), "hi");
    assert_eq!(fed(&mut t, "\x1b]2;a;b\x1b\\").title(), "a;b");
    assert_eq!(fed(&mut t, "\x1b]2;x\u{9b}y\x08\x7fz\u{1b}\\").title(), "xyz");
    let long = format!("\x1b]0;{}\x07", "é".repeat(300));
    assert_eq!(fed(&mut t, &long).title().chars().count(), 256);
    assert_eq!(fed(&mut t, b"\x1b]2;\xff\x07").title(), "\u{fffd}");
    t.feed(b"\x1b]52;c;aGk=\x07\x1b]1;icon\x07\x1b]8;;http://x\x07\x1b]10;?\x07\x1b]0\x07\x1bc");
    assert_eq!((t.take_replies(), t.title()), (Vec::new(), "\u{fffd}"));
}

#[test]
fn resize_keeps_the_cursor_row() {
    let sizes = [Term::new(0, 0), Term::new(5000, 5000)].map(|t| (t.cols(), t.rows()));
    assert_eq!(sizes, [(1, 1), (1000, 500)]);
    let t = Term::new(80, 24);
    assert!(t.row(23).len() == 80 && t.row(24).is_empty() && t.scrollback_row(0).is_empty());
    assert!(t.row(0).iter().all(|c| *c == Cell::default()));
    assert_eq!((t.cursor(), t.title()), ((0, 0), ""));
    assert!(t.cursor_visible() && !t.alt_screen() && !t.bracketed_paste() && !t.app_cursor_keys());
    let mut t = term(5, 3, "abcde\r\nfghij\r\nklm\x1b[2;4H");
    let g = t.generation();
    check(sized(&mut t, 3, 2), "abc|fgh", (1, 2));
    assert!(t.generation() > g);
    check(sized(&mut t, 6, 4), "abc|fgh||", (1, 2));
    assert_eq!(t.row(0).len(), 6);
    // The scroll region resets to the whole screen.
    let mut t = term(3, 4, "a\r\nb\x1b[1;2r");
    assert_eq!(screen(fed(sized(&mut t, 3, 3), "\x1b[3H\nc")), "b||c");
    // A cut wide character is blanked; the alternate screen resizes too.
    let mut t = term(4, 1, "a世\x1b[?1049h世");
    t.resize(2, 1);
    assert_eq!((screen(&t).as_str(), t.row(0)[1].width), ("世", 0));
    t.feed(b"\x1b[?1049l");
    assert_eq!((screen(&t).as_str(), t.row(0)[1].width), ("a", 1));
    t.resize(5000, 0);
    assert_eq!((t.cols(), t.rows()), (1000, 1));
    // Tab stops survive; new columns get the default ones.
    let mut t = term(10, 1, "\x1b[3g\x1b[5G\x1bH");
    assert_eq!(fed(sized(&mut t, 30, 1), "\x1b[G\t\t\t").cursor(), (0, 24));
    // The top rows go into the scrollback, so the prompt stays on screen.
    let mut t = term(10, 6, "l1\r\nl2\r\nl3\r\nl4\r\nl5\r\n$ prompt");
    check(sized(&mut t, 10, 3), "l4|l5|$ prompt", (2, 8));
    assert_eq!((t.scrollback_len(), text(t.scrollback_row(2)).as_str()), (3, "l3"));
    check(sized(&mut t, 10, 5), "l4|l5|$ prompt||", (2, 8));
    // Rows below the cursor are cut first.
    let mut t = term(10, 6, "a\r\nb\r\nc\r\nd\x1b[2H");
    check(sized(&mut t, 10, 3), "a|b|c", (1, 0));
    assert_eq!(t.scrollback_len(), 0);
    // On the alternate screen, the main screen keeps its saved cursor's row.
    let mut t = term(10, 6, "l1\r\nl2\r\nl3\r\nl4\r\n$ \x1b[?1049h\x1b[Hvim");
    check(sized(&mut t, 10, 2), "vim|", (0, 3));
    check(fed(&mut t, "\x1b[?1049l"), "l4|$", (1, 2));
    assert_eq!((t.scrollback_len(), text(t.scrollback_row(2)).as_str()), (3, "l3"));
}

#[test]
fn scrollback_and_resets() {
    let mut t = term(10, 2, &(0..5010).map(|i| format!("{i}\r\n")).collect::<String>());
    assert_eq!((t.scrollback_len(), screen(&t).as_str()), (SCROLLBACK, "5009|"));
    assert_eq!(text(t.scrollback_row(0)), "9");
    assert_eq!(text(t.scrollback_row(SCROLLBACK - 1)), "5008");
    assert!(t.scrollback_row(0).len() == 1 && t.scrollback_row(SCROLLBACK).is_empty());
    // Colored blanks are kept; the alternate screen never scrolls into it.
    t.feed(b"\x1b[44m\x1b[2K\n\x1b[m\n\x1b[?1049h\n\n\n\x1b[?1049l");
    assert_eq!(t.scrollback_row(SCROLLBACK - 1).len(), 10);
    t.feed(b"\x1b[3J");
    assert_eq!((t.scrollback_len(), screen(&t).as_str()), (0, "|"));
    // CSI S scrolls into it too; chained REPs cost a line each, not a screen.
    assert_eq!(term(3, 2, "a\x1b[2S").scrollback_len(), 2);
    check(&term(10, 3, "ab\x1b[65535b"), "abbbbbbbbb||", (0, 9));
    assert_eq!(term(10, 3, &format!("x{}", "\x1b[65535b".repeat(1000))).scrollback_len(), 498);
    // Modes are recorded; DECSTR resets some.
    let mut t = term(8, 3, "\x1b[?25l\x1b[?2004h\x1b[?1h\x1b[?1002h\x1b[?1006h\x1b[?1004h");
    assert!(!t.cursor_visible() && t.bracketed_paste() && t.app_cursor_keys());
    assert!(t.mouse == 1002 && t.mouse_sgr && t.focus);
    t.feed(b"\x1b[?1000l\x1b[!p");
    assert!(t.cursor_visible() && t.bracketed_paste() && !t.app_cursor_keys() && t.mouse == 0);
    // RIS clears both screens and the modes but keeps the scrollback.
    let t = term(4, 2, "a\r\nb\r\nc\x1b[?25l\x1b[31m\x1b[?1049h\x1bc");
    check(&t, "|", (0, 0));
    assert!(t.scrollback_len() == 1 && t.cursor_visible() && !t.alt_screen());
    let mut t = term(4, 2, "\x1b[?47hab\x1b[?47lcd\x1bc");
    assert_eq!(screen(&t), "|");
    assert_eq!(screen(fed(&mut t, "\x1b[?47h")), "|");
}

#[test]
fn keys_and_paste() {
    use Key::*;
    let key = |k: Key, mods: &str, app: bool| {
        let has = |c| mods.contains(c);
        encode_key(k, KeyMods { shift: has('s'), ctrl: has('c'), alt: has('a') }, app)
    };
    // Cursor keys: CSI, SS3 in application mode, CSI 1;m with modifiers.
    for (k, x) in [Up, Down, Right, Left, Home, End].into_iter().zip("ABCDHF".chars()) {
        assert_eq!(key(k, "", false), format!("\x1b[{x}").as_bytes());
        assert_eq!(key(k, "", true), format!("\x1bO{x}").as_bytes());
        assert_eq!(key(k, "c", true), format!("\x1b[1;5{x}").as_bytes());
        assert_eq!(key(k, "s", false), format!("\x1b[1;2{x}").as_bytes());
        assert_eq!(key(k, "a", false), format!("\x1b[1;3{x}").as_bytes());
        assert_eq!(key(k, "sca", false), format!("\x1b[1;8{x}").as_bytes());
    }
    // Editing keys and F5-F12: CSI n ~ and CSI n;m ~; F1-F4: SS3 P-S.
    let tilde = [Insert, Delete, PageUp, PageDown].into_iter().chain((5..=12).map(F));
    for (k, n) in tilde.zip([2, 3, 5, 6, 15, 17, 18, 19, 20, 21, 23, 24]) {
        assert_eq!(key(k, "", true), format!("\x1b[{n}~").as_bytes());
        assert_eq!(key(k, "c", false), format!("\x1b[{n};5~").as_bytes());
    }
    for (f, x) in (1..=4).zip("PQRS".chars()) {
        assert_eq!(key(F(f), "", false), format!("\x1bO{x}").as_bytes());
        assert_eq!(key(F(f), "s", true), format!("\x1b[1;2{x}").as_bytes());
    }
    for (c, b) in "azC@ [\\]^_?".chars().zip([1, 26, 3, 0, 0, 27, 28, 29, 30, 31, 127]) {
        assert_eq!(key(Char(c), "c", false), [b], "ctrl {c}");
    }
    #[rustfmt::skip]
    let cases: &[(Key, &str, &[u8])] = &[
        (Enter, "", b"\r"), (Enter, "a", b"\x1b\r"), (Backspace, "", b"\x7f"),
        (Backspace, "c", b"\x08"), (Backspace, "a", b"\x1b\x7f"), (Tab, "", b"\t"),
        (Tab, "s", b"\x1b[Z"), (Escape, "", b"\x1b"), (Escape, "a", b"\x1b\x1b"), (F(0), "", b""),
        (F(13), "c", b""), (Char('a'), "", b"a"), (Char('A'), "s", b"A"),
        (Char('\u{e9}'), "", "\u{e9}".as_bytes()), (Char('1'), "c", b"1"),
        (Char('x'), "a", b"\x1bx"), (Char('c'), "sca", b"\x1b\x03"),
    ];
    for &(k, mods, want) in cases {
        assert_eq!(key(k, mods, false), want, "{k:?} {mods}");
    }
    assert_eq!(paste("a\r\nb\nc\rd", false), b"a\rb\rc\rd");
    assert_eq!(paste("x\x1b[201~y", true), b"\x1b[200~x[201~y\x1b[201~");
    assert_eq!(paste("é\t", false), "é\t".as_bytes());
    assert_eq!(paste("", true), b"\x1b[200~\x1b[201~");
}

/// 200k bytes of mostly sequences, chunked and resized at random: nothing
/// panics, the cursor stays on screen, rows keep their width, wide chars whole.
#[test]
fn fuzz() {
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut rand = move |n: usize| {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005);
        seed = seed.wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) as usize % n
    };
    let bits: Vec<&str> = "\x1b,\x1b]0;,\x07,\x1b\\,\x1b(0,\x1b(B,\x1b)0,\x0e,\x0f,\r,\n,\x08,\t,\
        世,😀,é,\u{301},\x1b7,\x1b8,\x1bM,\x1bD,\x1bE,\x1bH,\x1bc,\x1b#8,\x1bP1$r\x1b\\,\x18,\u{9b},\
        \x1b[?1049h,\x1b[?1049l,\x1b[?6h,\x1b[?6l,\x1b[?7l,\x1b[?7h,\x1b[4h,\x1b[4l"
        .split(',')
        .collect();
    let finals = b"@ABCDEFGHIJKLMPSTXZ`abcdeghlmnpqrstuy";
    let mut t = Term::new(12, 6);
    let (mut fed, mut chunk) = (0, Vec::new());
    while fed < 200_000 {
        chunk.clear();
        for _ in 0..1 + rand(8) {
            match rand(12) {
                0..=4 => {
                    chunk.extend_from_slice(b"\x1b[");
                    chunk.extend((rand(3) == 0).then(|| b"?>=<"[rand(4)]));
                    for k in 0..rand(6) {
                        chunk.extend((k > 0).then(|| if rand(5) == 0 { b':' } else { b';' }));
                        let v = [65535, rand(2000), rand(12), rand(12)][rand(4)];
                        if rand(6) > 0 {
                            chunk.extend_from_slice(v.to_string().as_bytes());
                        }
                    }
                    chunk.extend((rand(8) == 0).then(|| b"$! "[rand(3)]));
                    chunk.push(finals[rand(finals.len())]);
                }
                5..=7 => chunk.extend_from_slice(bits[rand(bits.len())].as_bytes()),
                8 => chunk.push(rand(256) as u8),
                9 => chunk.extend_from_slice(&b"hello, world"[..rand(13)]),
                10 => chunk.extend_from_slice(&[0x1B, 0x20 + rand(0x5F) as u8]),
                _ if rand(10) == 0 => t.resize(1 + rand(30) as u16, 1 + rand(10) as u16),
                _ => chunk.extend_from_slice("世界😀".as_bytes()),
            }
        }
        fed += chunk.len();
        t.feed(&chunk);
        let (y, x, cols) = (t.cursor().0, t.cursor().1, t.cols());
        assert!(y < t.rows() && x < cols, "cursor {y},{x} in {cols}x{}", t.rows());
        for r in 0..t.rows() {
            assert_eq!(t.row(r).len(), usize::from(cols));
            wide_ok(t.row(r));
        }
        if rand(50) == 0 {
            t.take_replies();
        }
    }
    assert!(t.scrollback_len() <= SCROLLBACK);
    (0..t.scrollback_len()).for_each(|i| wide_ok(t.scrollback_row(i)));
}

#[test]
fn rotate_is_rotate_left() {
    for (len, k) in (0..9u8).flat_map(|len| (0..=len).map(move |k| (len, k))) {
        let (mut a, mut b): (Vec<u8>, Vec<u8>) = ((0..len).collect(), (0..len).collect());
        rotate(&mut a, k.into());
        b.rotate_left(k.into());
        assert_eq!(a, b, "{len} {k}");
    }
}
