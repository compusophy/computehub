//! Input: the bytes a key press or a paste sends to the program.

/// A key the host reports. Text arrives as `Char`, already shifted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// A character key, with shift already applied (`'A'`, `'!'`).
    Char(char),
    /// Enter or Return.
    Enter,
    /// Backspace.
    Backspace,
    /// Tab.
    Tab,
    /// Escape.
    Escape,
    /// Arrow up.
    Up,
    /// Arrow down.
    Down,
    /// Arrow left.
    Left,
    /// Arrow right.
    Right,
    /// Home.
    Home,
    /// End.
    End,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// Insert.
    Insert,
    /// Delete (forward).
    Delete,
    /// A function key, F1 to F12; others send nothing.
    F(u8),
}

/// The modifiers held with a key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyMods {
    /// Shift.
    pub shift: bool,
    /// Control.
    pub ctrl: bool,
    /// Alt (Option on a Mac); sent as an ESC prefix or as a modifier code.
    pub alt: bool,
}

/// The bytes xterm sends for `key` with `mods`. `app_cursor` is
/// [`Term::app_cursor_keys`](crate::Term::app_cursor_keys): arrows, Home and
/// End then send `ESC O` forms.
///
/// Modified cursor and function keys use xterm's `CSI 1 ; m X` and
/// `CSI n ; m ~` forms, with m = 1 + shift + 2 alt + 4 ctrl. Ctrl with a
/// letter or one of `@ [ \ ] ^ _` and space sends the C0 control, and ctrl
/// with `?` sends DEL; alt prefixes ESC.
pub fn encode_key(key: Key, mods: KeyMods, app_cursor: bool) -> Vec<u8> {
    let m = 1 + u8::from(mods.shift) + 2 * u8::from(mods.alt) + 4 * u8::from(mods.ctrl);
    let letter = |ss3: bool, x: u8| match (m, ss3) {
        (1, true) => vec![0x1B, b'O', x],
        (1, false) => vec![0x1B, b'[', x],
        _ => format!("\x1b[1;{m}{}", char::from(x)).into_bytes(),
    };
    let tilde = |n: u8| match m {
        1 => format!("\x1b[{n}~").into_bytes(),
        _ => format!("\x1b[{n};{m}~").into_bytes(),
    };
    let alt = |bytes: &[u8]| {
        let mut out = if mods.alt { vec![0x1B] } else { Vec::new() };
        out.extend_from_slice(bytes);
        out
    };
    match key {
        Key::Char(c) => {
            let control = match c {
                '@'..='_' | 'a'..='z' => Some(c as u8 & 0x1F),
                ' ' => Some(0),
                '?' => Some(0x7F),
                _ => None,
            };
            match control {
                Some(b) if mods.ctrl => alt(&[b]),
                _ => alt(c.encode_utf8(&mut [0; 4]).as_bytes()),
            }
        }
        Key::Enter => alt(b"\r"),
        Key::Backspace if mods.ctrl => alt(b"\x08"),
        Key::Backspace => alt(b"\x7f"),
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

/// The bytes that paste `text`: line breaks (`\r\n` or `\n`) become `\r`,
/// as typed Enter would send, and ESC bytes are removed so the text cannot
/// smuggle in sequences or end a bracketed paste early. With `bracketed`
/// ([`Term::bracketed_paste`](crate::Term::bracketed_paste)) the result is
/// wrapped in `ESC [ 200 ~` and `ESC [ 201 ~`.
pub fn paste(text: &str, bracketed: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 12);
    if bracketed {
        out.extend_from_slice(b"\x1b[200~");
    }
    let mut bytes = text.bytes().peekable();
    while let Some(b) = bytes.next() {
        match b {
            b'\r' => {
                bytes.next_if_eq(&b'\n');
                out.push(b'\r');
            }
            b'\n' => out.push(b'\r'),
            0x1B => {}
            _ => out.push(b),
        }
    }
    if bracketed {
        out.extend_from_slice(b"\x1b[201~");
    }
    out
}
