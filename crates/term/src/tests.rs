use super::*;
use Color::{Default as D, Indexed as I, Rgb};

fn term(cols: u16, rows: u16, bytes: &str) -> Term {
    let mut t = Term::new(cols, rows);
    t.feed(bytes.as_bytes());
    t
}

/// A row's text, wide tails skipped and trailing spaces trimmed.
fn text(line: &[Cell]) -> String {
    let s: String = line.iter().filter(|c| c.width != 0).map(|c| c.ch).collect();
    s.trim_end().to_string()
}

/// The screen as text, rows joined with `|`.
fn screen(t: &Term) -> String {
    let rows: Vec<String> = (0..t.rows()).map(|r| text(t.row(r))).collect();
    rows.join("|")
}

/// The screen after `bytes` in a new terminal.
fn run(cols: u16, rows: u16, bytes: &str) -> String {
    screen(&term(cols, rows, bytes))
}

/// The cursor after `bytes` in a new terminal.
fn at(cols: u16, rows: u16, bytes: &str) -> (u16, u16) {
    term(cols, rows, bytes).cursor()
}

fn check(t: &Term, want: &str, cursor: (u16, u16)) {
    assert_eq!((screen(t).as_str(), t.cursor()), (want, cursor));
}

fn cell(t: &Term, r: u16, c: usize) -> Cell {
    t.row(r)[c]
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
fn new_clamps_and_starts_blank() {
    let t = Term::new(0, 0);
    assert_eq!((t.cols(), t.rows()), (1, 1));
    let t = Term::new(5000, 5000);
    assert_eq!((t.cols(), t.rows()), (1000, 500));
    let t = Term::new(80, 24);
    assert_eq!(t.row(23).len(), 80);
    assert!(t.row(24).is_empty() && t.scrollback_row(0).is_empty());
    assert!(t.row(0).iter().all(|c| *c == Cell::default()));
    assert_eq!((t.cursor(), t.title()), ((0, 0), ""));
    assert!(t.cursor_visible() && !t.alt_screen());
    assert!(!t.bracketed_paste() && !t.app_cursor_keys());
}

#[test]
fn pending_wrap() {
    let mut t = term(5, 3, "abcde");
    check(&t, "abcde||", (0, 4));
    t.feed(b"f");
    check(&t, "abcde|f|", (1, 1));
    // CR LF right after a full line does not leave a blank line.
    assert_eq!(run(5, 3, "abcde\r\nx"), "abcde|x|");
    // Cursor motion clears the flag; BS steps back from the last column.
    assert_eq!(run(5, 3, "abcde\x08X"), "abcXe||");
    assert_eq!(run(5, 3, "abcde\x1b[GX"), "Xbcde||");
    assert_eq!(run(5, 3, "abcde\x1b[KX"), "abcdX||");
    // Without DECAWM the last column is overwritten.
    assert_eq!(run(5, 3, "\x1b[?7labcdefg"), "abcdg||");
    // Wrapping at the bottom scrolls into the scrollback.
    let t = term(3, 2, "abcdefg");
    assert_eq!(screen(&t), "def|g");
    assert_eq!(text(t.scrollback_row(0)), "abc");
    // DECSC and DECRC keep the flag.
    assert_eq!(run(3, 2, "abc\x1b7\x1b[H\x1b8d"), "abc|d");
    // DSR reports the last column while the flag is pending.
    assert_eq!(term(5, 3, "abcde\x1b[6n").take_replies(), b"\x1b[1;5R");
}

/// Rows a to e with the scroll region on rows 2 to 4, then `extra`.
fn region(extra: &str) -> String {
    run(3, 5, &format!("a\r\nb\r\nc\r\nd\r\ne\x1b[2;4r{extra}"))
}

#[test]
fn scroll_regions() {
    assert_eq!(at(3, 5, "\x1b[5;5Hx\x1b[2;4r"), (0, 0));
    assert_eq!(region("\x1b[4H\n"), "a|c|d||e");
    assert_eq!(region("\x1b[4H\x1bD"), "a|c|d||e");
    assert_eq!(region("\x1b[4H\x1bE"), "a|c|d||e");
    assert_eq!(region("\x1b[2H\x1bM"), "a||b|c|e");
    assert_eq!(region("\x1b[3H\x1b[L"), "a|b||c|e");
    assert_eq!(region("\x1b[3H\x1b[9L"), "a|b|||e");
    assert_eq!(region("\x1b[2H\x1b[M"), "a|c|d||e");
    assert_eq!(region("\x1b[2H\x1b[2M"), "a|d|||e");
    assert_eq!(region("\x1b[S"), "a|c|d||e");
    assert_eq!(region("\x1b[2T"), "a|||b|e");
    // IL, DL, LF and RI outside the region do not scroll it; an empty or
    // inverted region is ignored.
    assert_eq!(region("\x1b[5H\x1b[L\x1b[M\n"), "a|b|c|d|e");
    assert_eq!(region("\x1b[1H\x1bM"), "a|b|c|d|e");
    assert_eq!(region("\x1b[3;3r\x1b[4;2r\x1b[5H\n"), "a|b|c|d|e");
    // IL and DL move to the first column.
    assert_eq!(at(3, 5, "\x1b[2;4r\x1b[3;3H\x1b[L"), (2, 0));
    // Origin mode: rows count from the top margin and stay in the region.
    assert_eq!(region("\x1b[?6h\x1b[1;1HX"), "a|X|c|d|e");
    assert_eq!(region("\x1b[?6h\x1b[9;1HX"), "a|b|c|X|e");
    let mut t = term(3, 5, "\x1b[2;4r\x1b[?6h\x1b[2;2H\x1b[6n");
    assert_eq!(t.take_replies(), b"\x1b[2;2R");
    // CUU and CUD stop at the margins.
    assert_eq!(at(3, 5, "\x1b[2;4r\x1b[3H\x1b[9A"), (1, 0));
    assert_eq!(at(3, 5, "\x1b[2;4r\x1b[3H\x1b[9B"), (3, 0));
    assert_eq!(at(3, 5, "\x1b[2;4r\x1b[5H\x1b[9A"), (1, 0));
    assert_eq!(at(3, 5, "\x1b[2;4r\x1b[H\x1b[9A"), (0, 0));
    // Only a region starting at the top feeds the scrollback.
    assert_eq!(term(3, 5, "\x1b[2;5r\x1b[5H\n\n").scrollback_len(), 0);
    assert_eq!(term(3, 5, "\x1b[1;3r\x1b[3H\n\n").scrollback_len(), 2);
}

#[test]
fn erase() {
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
    assert_eq!(at(5, 3, "ab\x1b[2J"), (0, 2));
    // Erased cells take the background color (bce), not other attributes.
    let c = cell(&term(5, 3, "ab\x1b[1;31;42m\x1b[2K"), 0, 0);
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
    let expect = [
        ('A', D, D, Attrs(0xFF)),
        ('B', D, D, none),
        ('C', I(1), I(2), none),
        ('D', I(9), I(10), none),
        ('E', I(196), Rgb(1, 2, 3), none),
        ('F', Rgb(10, 20, 30), I(7), none),
        ('G', Rgb(1, 2, 3), I(7), u),
        ('H', D, D, none),
        ('I', D, D, none),
        ('J', D, D, u),
        ('K', D, D, none),
        // 58 (underline color) takes its arguments with it.
        ('L', D, D, b),
        // An empty parameter is 0.
        ('M', D, D, u),
        // Out-of-range colors are ignored; a short colon RGB means 0.
        ('N', Rgb(0, 1, 2), D, u),
        // Private SGR (xterm's modifyOtherKeys) is not SGR.
        ('O', Rgb(0, 1, 2), D, u),
        ('P', Rgb(0, 1, 2), D, u | b),
    ];
    for (i, (ch, fg, bg, attrs)) in expect.into_iter().enumerate() {
        let c = cell(&t, 0, i);
        assert_eq!((c.ch, c.fg, c.bg, c.attrs), (ch, fg, bg, attrs), "{ch}");
    }
    let mut a = Attrs::BOLD | Attrs::STRIKE;
    a.remove(Attrs::BOLD);
    assert!(a.contains(Attrs::STRIKE) && !a.contains(Attrs::BOLD) && !a.is_empty());
    assert_eq!((a.bits(), Attrs::default()), (128, Attrs::empty()));
}

#[test]
fn alternate_screen() {
    let mut t = term(10, 3, "main\x1b[2;3H\x1b[?1049h");
    assert!(t.alt_screen());
    check(&t, "||", (1, 2));
    t.feed(b"\x1b[Halt\r\n\n\n\n\x1b[3;9H");
    assert_eq!(t.scrollback_len(), 0);
    t.feed(b"\x1b[?1049l");
    assert!(!t.alt_screen());
    check(&t, "main||", (1, 2));
    // Entering again clears it.
    t.feed(b"\x1b[?1049hx\x1b[?1049l\x1b[?1049h");
    assert_eq!(screen(&t), "||");
    t.feed(b"\x1b[?1049l\x1b[?1049l");
    check(&t, "main||", (1, 2));
    // 47 keeps the alternate screen's content; 1047 clears it on the way out.
    t.feed(b"\x1b[?47hkeep\x1b[?47l\x1b[?47h");
    assert_eq!(screen(&t), "|  keep|");
    t.feed(b"\x1b[?1047l\x1b[?1047h");
    assert_eq!(screen(&t), "||");
    t.feed(b"\x1b[?1047l\x1b[?1048h\x1b[3;3H\x1b[?1048l");
    assert_eq!((t.alt_screen(), t.cursor()), (false, (1, 6)));
}

#[test]
fn save_and_restore_cursor() {
    let mut t = Term::new(10, 3);
    t.feed(b"\x1b[2;4H\x1b[1;31m\x1b(0\x1b7\x1b[H\x1b[0m\x1b(Bq\x1b8q");
    let (a, b) = (cell(&t, 0, 0), cell(&t, 1, 3));
    assert_eq!((a.ch, a.fg, a.attrs), ('q', D, Attrs(0)));
    assert_eq!((b.ch, b.fg, b.attrs), ('─', I(1), Attrs::BOLD));
    assert_eq!(t.cursor(), (1, 4));
    // CSI s and u do the same; restoring without a save homes the cursor.
    assert_eq!(at(10, 3, "\x1b[2;4H\x1b[s\x1b[H\x1b[u"), (1, 3));
    let t = term(10, 3, "\x1b[2;4H\x1b[7m\x1b8x");
    assert_eq!((t.cursor(), cell(&t, 0, 0).attrs), ((0, 1), Attrs(0)));
    // A saved position past a shrink is clamped.
    let mut t = term(10, 3, "\x1b[3;9H\x1b7");
    t.resize(4, 2);
    t.feed(b"\x1b8");
    assert_eq!(t.cursor(), (1, 3));
}

#[test]
fn replies() {
    let mut t = Term::new(10, 5);
    t.feed(b"\x1b[5n\x1b[3;4H\x1b[6n\x1b[c\x1b[0c\x1b[>c\x1b[?2026$p\x1b[?2026h\x1b[?2026$p");
    t.feed(b"\x1b[4$p\x1b[?9999$p\x1b[?1049$p\x1b[1c\x1b[7n");
    let want: &[u8] = b"\x1b[0n\x1b[3;4R\x1b[?62;22c\x1b[?62;22c\x1b[>0;10;1c\x1b[?2026;2$y\
        \x1b[?2026;1$y\x1b[4;2$y\x1b[?9999;0$y\x1b[?1049;2$y";
    assert_eq!(t.take_replies(), want);
    assert!(t.take_replies().is_empty() && t.synchronized());
    // Queries alone are not a visible change.
    let g = t.generation();
    t.feed(b"\x1b[6n\x1b[c\x1b]11;?\x07\x1b[18t");
    assert_eq!(t.generation(), g);
    assert_eq!(t.take_replies(), b"\x1b[3;4R\x1b[?62;22c");
    t.feed(b"x");
    assert_eq!(t.generation(), g + 1);
    // Unread replies are capped.
    t.feed("\x1b[6n".repeat(20_000).as_bytes());
    assert!(t.take_replies().len() <= 1 << 16);
}

#[test]
fn dec_line_drawing() {
    assert_eq!(run(10, 1, "\x1b(0lqwqk\x1b(Bq"), "┌─┬─┐q");
    assert_eq!(run(10, 1, "\x1b)0a\x0ex\x0fx"), "a│x");
    let all = run(40, 1, "\x1b(0`abcdefghijklmnopqrstuvwxyz{|}~_AZ");
    assert_eq!(all, "◆▒␉␌␍␊°±␤␋┘┐┌└┼⎺⎻─⎼⎽├┤┴┬│≤≥π≠£· AZ");
    // RIS and DECSTR go back to ASCII.
    assert_eq!(run(5, 1, "\x1b(0\x1bcq"), "q");
    assert_eq!(run(5, 1, "\x1b(0\x1b[!pq"), "q");
}

#[test]
fn wide_chars() {
    let t = term(5, 2, "abcd世");
    check(&t, "abcd|世", (1, 2));
    assert_eq!((cell(&t, 1, 0).width, cell(&t, 1, 1).width), (2, 0));
    assert_eq!(run(5, 2, "\x1b[?7labcd世"), "abcd|");
    assert_eq!(run(1, 2, "世x"), "|x");
    let t = term(4, 1, "ab世");
    assert_eq!((t.cursor(), cell(&t, 0, 2).ch), ((0, 3), '世'));
    // Overwriting either half blanks the other.
    assert_eq!(run(5, 1, "世界\x1b[2Gx"), " x界");
    assert_eq!(run(5, 1, "世界\x1b[Gx"), "x 界");
    assert_eq!(run(5, 1, "a世界\x1b[3G世"), "a 世");
    wide_ok(term(5, 1, "世界\x1b[2G世").row(0));
    // Combining marks, ZWJ, variation selectors and C1 controls are dropped.
    assert_eq!(run(9, 1, "e\u{301}\u{200d}\u{fe0f}x\u{85}"), "ex");
    check(&term(9, 1, "😀🚀x"), "😀🚀x", (0, 5));
    // Insert mode makes room for both halves.
    assert_eq!(run(6, 1, "abcd\x1b[G\x1b[4h世"), "世abcd");
}

#[test]
fn tabs() {
    let t = term(20, 1, "\tx");
    assert_eq!((t.cursor(), cell(&t, 0, 8).ch), ((0, 9), 'x'));
    assert_eq!(at(20, 1, "\t\t\t"), (0, 19));
    assert_eq!(at(20, 1, "\x1b[4G\x1bH\x1b[G\t"), (0, 3));
    assert_eq!(at(20, 1, "\x1b[3g\t"), (0, 19));
    assert_eq!(at(20, 1, "\x1b[9G\x1b[g\x1b[G\t"), (0, 16));
    assert_eq!(at(20, 1, "\x1b[20G\x1b[Z"), (0, 16));
    assert_eq!(at(20, 1, "\x1b[20G\x1b[2Z"), (0, 8));
    assert_eq!(at(20, 1, "\x1b[20G\x1b[9Z"), (0, 0));
    assert_eq!(at(20, 1, "\x1b[2I"), (0, 16));
    // Stops survive a resize; new columns get the default ones.
    let mut t = term(10, 1, "\x1b[3g\x1b[5G\x1bH");
    t.resize(30, 1);
    t.feed(b"\x1b[G\t\t\t");
    assert_eq!(t.cursor(), (0, 24));
}

#[test]
fn resize() {
    let mut t = term(5, 3, "abcde\r\nfghij\r\nklm\x1b[2;4H");
    let g = t.generation();
    t.resize(3, 2);
    check(&t, "abc|fgh", (1, 2));
    assert!(t.generation() > g);
    t.resize(6, 4);
    check(&t, "abc|fgh||", (1, 2));
    assert_eq!(t.row(0).len(), 6);
    // The scroll region resets to the whole screen.
    let mut t = term(3, 4, "a\r\nb\x1b[1;2r");
    t.resize(3, 3);
    t.feed(b"\x1b[3H\nc");
    assert_eq!(screen(&t), "b||c");
    // A cut wide character is blanked; the alternate screen resizes too.
    let mut t = term(4, 1, "a世\x1b[?1049h世");
    t.resize(2, 1);
    assert_eq!((screen(&t).as_str(), cell(&t, 0, 1).width), ("世", 0));
    t.feed(b"\x1b[?1049l");
    assert_eq!((screen(&t).as_str(), cell(&t, 0, 1).width), ("a", 1));
    t.resize(5000, 0);
    assert_eq!((t.cols(), t.rows()), (1000, 1));
}

#[test]
fn shrinking_keeps_the_cursor_row() {
    // The top rows go into the scrollback, so the prompt stays on screen.
    let mut t = term(10, 6, "l1\r\nl2\r\nl3\r\nl4\r\nl5\r\n$ prompt");
    t.resize(10, 3);
    check(&t, "l4|l5|$ prompt", (2, 8));
    assert_eq!(t.scrollback_len(), 3);
    assert_eq!(text(t.scrollback_row(2)), "l3");
    t.resize(10, 5);
    check(&t, "l4|l5|$ prompt||", (2, 8));
    // Rows below the cursor are cut first.
    let mut t = term(10, 6, "a\r\nb\r\nc\r\nd\x1b[2H");
    t.resize(10, 3);
    check(&t, "a|b|c", (1, 0));
    assert_eq!(t.scrollback_len(), 0);
    // On the alternate screen the main screen keeps its saved cursor's row,
    // where leaving it lands.
    let mut t = term(10, 6, "l1\r\nl2\r\nl3\r\nl4\r\n$ \x1b[?1049h\x1b[Hvim");
    t.resize(10, 2);
    check(&t, "vim|", (0, 3));
    t.feed(b"\x1b[?1049l");
    check(&t, "l4|$", (1, 2));
    assert_eq!(t.scrollback_len(), 3);
    assert_eq!(text(t.scrollback_row(2)), "l3");
}

#[test]
fn scrollback() {
    let mut t = Term::new(10, 2);
    let lines: String = (0..5010).map(|i| format!("{i}\r\n")).collect();
    t.feed(lines.as_bytes());
    assert_eq!(t.scrollback_len(), SCROLLBACK);
    assert_eq!(text(t.scrollback_row(0)), "9");
    assert_eq!(text(t.scrollback_row(SCROLLBACK - 1)), "5008");
    assert_eq!(t.scrollback_row(0).len(), 1);
    assert!(t.scrollback_row(SCROLLBACK).is_empty());
    assert_eq!(screen(&t), "5009|");
    // Colored blanks are kept; the alternate screen never scrolls into it.
    t.feed(b"\x1b[44m\x1b[2K\n\x1b[m\n\x1b[?1049h\n\n\n\x1b[?1049l");
    assert_eq!(t.scrollback_row(SCROLLBACK - 1).len(), 10);
    t.feed(b"\x1b[3J");
    assert_eq!((t.scrollback_len(), screen(&t).as_str()), (0, "|"));
    // CSI S scrolls into it too.
    assert_eq!(term(3, 2, "a\x1b[2S").scrollback_len(), 2);
}

#[test]
fn title() {
    let mut t = term(5, 1, "\x1b]0;hi\x07");
    assert_eq!(t.title(), "hi");
    t.feed(b"\x1b]2;a;b\x1b\\");
    assert_eq!(t.title(), "a;b");
    t.feed("\x1b]2;x\u{9b}y\x08\x7fz\u{1b}\\".as_bytes());
    assert_eq!(t.title(), "xyz");
    t.feed(format!("\x1b]0;{}\x07", "é".repeat(300)).as_bytes());
    assert_eq!(t.title().chars().count(), 256);
    t.feed(b"\x1b]2;\xff\x07");
    assert_eq!(t.title(), "\u{fffd}");
    // Clipboard writes, the icon name, links and color queries are ignored.
    t.feed(b"\x1b]52;c;aGk=\x07\x1b]1;icon\x07\x1b]8;;http://x\x07\x1b]10;?\x07\x1b]0\x07");
    assert!(t.take_replies().is_empty());
    assert_eq!(t.title(), "\u{fffd}");
    // RIS keeps the title.
    t.feed(b"\x1bc");
    assert_eq!(t.title(), "\u{fffd}");
}

#[test]
fn modes_and_resets() {
    let mut t = Term::new(8, 3);
    t.feed(b"\x1b[?25l\x1b[?2004h\x1b[?1h\x1b[?1002h\x1b[?1006h\x1b[?1004h");
    assert!(!t.cursor_visible() && t.bracketed_paste() && t.app_cursor_keys());
    assert!(t.mouse_mode() == 1002 && t.mouse_sgr() && t.focus_reporting());
    t.feed(b"\x1b[?1000l\x1b[!p");
    assert!(t.cursor_visible() && t.bracketed_paste() && !t.app_cursor_keys());
    assert_eq!(t.mouse_mode(), 0);
    // Insert mode, REP (after a printed character only) and DECALN.
    assert_eq!(run(5, 1, "abc\x1b[G\x1b[4hXY\x1b[4lZ"), "XYZbc");
    assert_eq!(run(9, 1, "ab\x1b[3b"), "abbbb");
    assert_eq!(run(9, 1, "a\r\x1b[3b"), "a");
    // REP stops at the end of the line (one more after a pending wrap), so
    // chained REPs cost a line each, not a screen.
    check(&term(10, 3, "ab\x1b[65535b"), "abbbbbbbbb||", (0, 9));
    assert_eq!(run(10, 3, "ab\x1b[65535b\x1b[9b"), "abbbbbbbbb|b|");
    let t = term(10, 3, &format!("x{}", "\x1b[65535b".repeat(1000)));
    assert_eq!(t.scrollback_len(), 498);
    assert_eq!(run(3, 2, "\x1b[2;2H\x1b#8"), "EEE|EEE");
    // RIS clears both screens and the modes but keeps the scrollback.
    let t = term(4, 2, "a\r\nb\r\nc\x1b[?25l\x1b[31m\x1b[?1049h\x1bc");
    check(&t, "|", (0, 0));
    assert!(t.scrollback_len() == 1 && t.cursor_visible() && !t.alt_screen());
    let mut t = term(4, 2, "\x1b[?47hab\x1b[?47lcd\x1bc");
    assert_eq!(screen(&t), "|");
    t.feed(b"\x1b[?47h");
    assert_eq!(screen(&t), "|");
    // Cursor movement clamps to the screen.
    assert_eq!(at(8, 3, "\x1b[99;99H"), (2, 7));
    assert_eq!(at(8, 3, "\x1b[99C\x1b[2D\x1b[9B\x1b[A"), (1, 5));
    assert_eq!(at(8, 3, "\x1b[3;5H\x1b[F"), (1, 0));
    assert_eq!(at(8, 3, "\x1b[5E"), (2, 0));
    assert_eq!(at(8, 3, "\x1b[2d\x1b[4`"), (1, 3));
    assert_eq!(at(8, 3, "\x1b[;5H\x1b[3a\x1b[e"), (1, 7));
}

#[test]
fn keys() {
    use Key::*;
    let key = |k: Key, mods: &str, app: bool| {
        let has = |c| mods.contains(c);
        let (shift, ctrl, alt) = (has('s'), has('c'), has('a'));
        encode_key(k, KeyMods { shift, ctrl, alt }, app)
    };
    // Cursor keys: CSI, SS3 in application mode, CSI 1;m with modifiers.
    let cursor = [Up, Down, Right, Left, Home, End];
    for (k, x) in cursor.into_iter().zip("ABCDHF".chars()) {
        assert_eq!(key(k, "", false), format!("\x1b[{x}").as_bytes());
        assert_eq!(key(k, "", true), format!("\x1bO{x}").as_bytes());
        assert_eq!(key(k, "c", true), format!("\x1b[1;5{x}").as_bytes());
        assert_eq!(key(k, "s", false), format!("\x1b[1;2{x}").as_bytes());
        assert_eq!(key(k, "a", false), format!("\x1b[1;3{x}").as_bytes());
        assert_eq!(key(k, "sca", false), format!("\x1b[1;8{x}").as_bytes());
    }
    // Editing keys and F5-F12: CSI n ~ and CSI n;m ~; F1-F4: SS3 P-S.
    let tilde =
        [Insert, Delete, PageUp, PageDown, F(5), F(6), F(7), F(8), F(9), F(10), F(11), F(12)];
    let codes = [2, 3, 5, 6, 15, 17, 18, 19, 20, 21, 23, 24];
    for (k, n) in tilde.into_iter().zip(codes) {
        assert_eq!(key(k, "", true), format!("\x1b[{n}~").as_bytes());
        assert_eq!(key(k, "c", false), format!("\x1b[{n};5~").as_bytes());
    }
    for (f, x) in (1..=4).zip("PQRS".chars()) {
        assert_eq!(key(F(f), "", false), format!("\x1bO{x}").as_bytes());
        assert_eq!(key(F(f), "s", true), format!("\x1b[1;2{x}").as_bytes());
    }
    let controls = [1, 26, 3, 0, 0, 27, 28, 29, 30, 31, 127];
    for (c, b) in "azC@ [\\]^_?".chars().zip(controls) {
        assert_eq!(key(Char(c), "c", false), [b], "ctrl {c}");
    }
    let cases: &[(Key, &str, &[u8])] = &[
        (Enter, "", b"\r"),
        (Enter, "a", b"\x1b\r"),
        (Backspace, "", b"\x7f"),
        (Backspace, "c", b"\x08"),
        (Backspace, "a", b"\x1b\x7f"),
        (Tab, "", b"\t"),
        (Tab, "s", b"\x1b[Z"),
        (Escape, "", b"\x1b"),
        (Escape, "a", b"\x1b\x1b"),
        (F(0), "", b""),
        (F(13), "c", b""),
        (Char('a'), "", b"a"),
        (Char('A'), "s", b"A"),
        (Char('\u{e9}'), "", "\u{e9}".as_bytes()),
        (Char('1'), "c", b"1"),
        (Char('x'), "a", b"\x1bx"),
        (Char('c'), "sca", b"\x1b\x03"),
    ];
    for &(k, mods, want) in cases {
        assert_eq!(key(k, mods, false), want, "{k:?} {mods}");
    }
}

#[test]
fn pasting() {
    assert_eq!(paste("a\r\nb\nc\rd", false), b"a\rb\rc\rd");
    assert_eq!(paste("x\x1b[201~y", true), b"\x1b[200~x[201~y\x1b[201~");
    assert_eq!(paste("é\t", false), "é\t".as_bytes());
    assert_eq!(paste("", true), b"\x1b[200~\x1b[201~");
}

#[test]
fn split_feeds_match_one_feed() {
    let bytes = "a\x1b[1;38:2::1:2:3m世\x1b]0;t\x07\u{1f600}\x1b[2;3Hq".as_bytes();
    let whole = term(10, 3, std::str::from_utf8(bytes).unwrap());
    let mut t = Term::new(10, 3);
    for b in bytes {
        t.feed(&[*b]);
    }
    check(&t, &screen(&whole), whole.cursor());
    assert_eq!((t.row(0), t.title()), (whole.row(0), "t"));
}

/// 200k bytes biased to escape sequences, in random chunks with random
/// resizes: nothing panics, the cursor stays on screen, rows keep their
/// width and wide characters stay whole.
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
                    if rand(3) == 0 {
                        chunk.push(b"?>=<"[rand(4)]);
                    }
                    for k in 0..rand(6) {
                        if k > 0 {
                            chunk.push(if rand(5) == 0 { b':' } else { b';' });
                        }
                        let v = [65535, rand(2000), rand(12), rand(12)][rand(4)];
                        if rand(6) > 0 {
                            chunk.extend_from_slice(v.to_string().as_bytes());
                        }
                    }
                    if rand(8) == 0 {
                        chunk.push(b"$! "[rand(3)]);
                    }
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
