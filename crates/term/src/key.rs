//! Input: the bytes a key press or a paste sends to the program.

/// A key the host reports: text arrives as `Char`, already shifted (`'A'`, `'!'`); of the
/// function keys `F`, F1 to F12 send bytes and others nothing.
#[allow(missing_docs)] // the variants are the keys they name
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[rustfmt::skip]
pub enum Key {
    Char(char), Enter, Backspace, Tab, Escape, Up, Down, Left, Right, Home, End, PageUp, PageDown,
    Insert, Delete, F(u8),
}

/// The modifiers held with a key; alt sends an ESC prefix or a modifier code.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[rustfmt::skip]
pub struct KeyMods { pub shift: bool, pub ctrl: bool, pub alt: bool }

/// The bytes xterm sends for `key`; with `app_cursor` ([`crate::Term::app_cursor_keys`]) arrows,
/// Home and End send `ESC O` forms. Modified keys send `CSI 1 ; m X` or `CSI n ; m ~` (m = 1 +
/// shift + 2 alt + 4 ctrl); ctrl makes C0 controls, alt prefixes ESC.
pub fn encode_key(key: Key, mods: KeyMods, app_cursor: bool) -> Vec<u8> {
    let m = 1 + u8::from(mods.shift) + 2 * u8::from(mods.alt) + 4 * u8::from(mods.ctrl);
    // CSI n ; m x without `; m` for m = 1 and n for 0. Not `format!`: this
    // ships in the boot download, which has no room for `core::fmt`.
    let csi = |n: u8, x: u8| {
        let n = [n / 10, n % 10].into_iter().skip_while(|&d| d == 0).map(|d| b'0' + d);
        let with = [b';', b'0' + m].into_iter().filter(|_| m > 1);
        [0x1B, b'['].into_iter().chain(n).chain(with).chain([x]).collect()
    };
    let letter = |ss3: bool, x: u8| {
        if m == 1 && ss3 { vec![0x1B, b'O', x] } else { csi(u8::from(m > 1), x) }
    };
    let tilde = |n: u8| csi(n, b'~');
    let alt = |bytes: &[u8]| [&b"\x1b"[..usize::from(mods.alt)], bytes].concat();
    match key {
        Key::Char(c) if mods.ctrl && matches!(c, '@'..='_' | 'a'..='z' | ' ' | '?') => {
            alt(&[if c == '?' { 0x7F } else { c as u8 & 0x1F }])
        }
        Key::Char(c) => alt(c.encode_utf8(&mut [0; 4]).as_bytes()),
        Key::Enter => alt(b"\r"),
        Key::Backspace => alt(if mods.ctrl { b"\x08" } else { b"\x7f" }),
        Key::Tab if mods.shift => b"\x1b[Z".to_vec(),
        Key::Tab => alt(b"\t"),
        Key::Escape => alt(b"\x1b"),
        Key::Up => letter(app_cursor, b'A'),
        Key::Down => letter(app_cursor, b'B'),
        Key::Right => letter(app_cursor, b'C'),
        Key::Left => letter(app_cursor, b'D'),
        Key::Home => letter(app_cursor, b'H'),
        Key::End => letter(app_cursor, b'F'),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::F(n @ 1..=4) => letter(true, b'O' + n),
        Key::F(n @ 5..=12) => tilde([15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)]),
        Key::F(_) => Vec::new(),
    }
}

/// The bytes that paste `text`: line breaks become `\r` (as Enter) and ESC is
/// dropped, so the text cannot end a bracketed paste (`bracketed`) early.
pub fn paste(text: &str, bracketed: bool) -> Vec<u8> {
    let text = text.replace("\r\n", "\r").replace('\n', "\r").replace('\x1b', "");
    let [open, close] = if bracketed { ["\x1b[200~", "\x1b[201~"] } else { ["", ""] };
    [open, &text, close].concat().into_bytes()
}
