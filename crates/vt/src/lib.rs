//! A dependency-free parser for the byte stream a terminal receives: Paul
//! Williams' DEC ANSI state machine (<https://vt100.net/emu/dec_ansi_parser>),
//! the one xterm-compatible terminals follow, with UTF-8 decoding built in.
//!
//! [`Parser::advance`] takes bytes in chunks of any size and reports what they
//! mean through a [`Perform`]: printable characters, C0 controls, and complete
//! CSI, ESC and OSC sequences. It keeps no screen state; that is the
//! terminal's job (the `term` crate). A new crate, not a fork.
//!
//! # Behavior
//!
//! - **UTF-8.** Text is decoded incrementally, so a character split across
//!   `advance` calls comes out whole. Invalid input yields U+FFFD once per
//!   maximal invalid subsequence (the WHATWG rule); a byte that interrupts a
//!   character yields U+FFFD and is then handled as usual.
//! - **No C1 controls.** Bytes 0x80 to 0x9F are UTF-8 continuation bytes,
//!   never 8-bit CSI, OSC or ST. A C1 code point that arrives UTF-8 encoded is
//!   printed like any other character.
//! - **Controls.** C0 controls are executed wherever they appear, even in the
//!   middle of a CSI or ESC sequence, but not inside strings. DEL is ignored.
//! - **Aborts.** CAN (0x18) and SUB (0x1A) cancel the sequence in progress
//!   without dispatching it, and are then executed. ESC always starts a new
//!   sequence; inside an OSC it first ends and dispatches the OSC. The `\` of
//!   a string terminator (ESC `\`) is swallowed, not dispatched.
//! - **Ignored strings.** DCS, SOS, PM and APC are consumed up to their ST and
//!   reported nowhere.
//! - **Limits.** A CSI keeps [`MAX_PARAMS`] parameters of at most
//!   [`MAX_SUBPARAMS`] colon-separated values each; values saturate at
//!   `u16::MAX` and extras are dropped. More than [`MAX_INTERMEDIATES`]
//!   intermediate bytes, or a parameter or private-marker byte out of place,
//!   drop the whole sequence. An OSC keeps its first [`MAX_OSC`] bytes and
//!   splits into at most [`MAX_OSC_PARAMS`] parts. Nothing panics and memory
//!   stays bounded, whatever the input.
//!
//! # Example
//!
//! ```
//! use vt::{Params, Parser, Perform};
//!
//! #[derive(Default)]
//! struct Screen {
//!     text: String,
//!     sgr: Vec<Vec<Option<u16>>>,
//! }
//!
//! impl Perform for Screen {
//!     fn print(&mut self, c: char) {
//!         self.text.push(c);
//!     }
//!     fn csi(&mut self, params: &Params, _: &[u8], _: Option<u8>, action: u8) {
//!         if action == b'm' {
//!             self.sgr.extend(params.iter().map(|group| group.to_vec()));
//!         }
//!     }
//! }
//!
//! let (mut parser, mut screen) = (Parser::new(), Screen::default());
//! let bytes = "\x1b[1;38:2::255:128:0mhé!".as_bytes();
//! // Chunk boundaries don't matter, even inside a sequence or a character.
//! let (a, b) = bytes.split_at(bytes.len() - 2);
//! parser.advance(a, &mut screen);
//! parser.advance(b, &mut screen);
//! assert_eq!(screen.text, "hé!");
//! let truecolor = vec![Some(38), Some(2), None, Some(255), Some(128), Some(0)];
//! assert_eq!(screen.sgr, [vec![Some(1)], truecolor]);
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use core::fmt;

/// The most parameters a CSI keeps; later ones are dropped.
pub const MAX_PARAMS: usize = 32;
/// The most colon-separated values one CSI parameter keeps, its own value
/// included; later ones are dropped.
pub const MAX_SUBPARAMS: usize = 8;
/// The most intermediate bytes a CSI or ESC may carry; one more drops it.
pub const MAX_INTERMEDIATES: usize = 2;
/// The most OSC payload bytes kept; the rest of a longer OSC is dropped.
pub const MAX_OSC: usize = 8192;
/// The most `;`-separated parts an OSC is split into; the last part keeps
/// any further `;` unsplit.
pub const MAX_OSC_PARAMS: usize = 16;

const REPLACEMENT: char = '\u{FFFD}';

/// What the parser reports. Every method has an empty default, so an
/// implementor overrides only what it handles.
#[allow(unused_variables)]
pub trait Perform {
    /// A printable character, UTF-8 decoded (U+FFFD for invalid input).
    fn print(&mut self, c: char) {}

    /// A C0 control (0x00 to 0x1F, never ESC), including a CAN or SUB that
    /// aborted a sequence.
    fn execute(&mut self, byte: u8) {}

    /// A complete CSI sequence: `ESC [`, an optional private marker (one of
    /// `? > < =`), parameters, intermediates (0x20 to 0x2F) and the final
    /// `action` byte (0x40 to 0x7E).
    fn csi(&mut self, params: &Params, intermediates: &[u8], private: Option<u8>, action: u8) {}

    /// A complete escape sequence other than CSI and strings: `ESC`,
    /// intermediates (0x20 to 0x2F), then the final `byte` (0x30 to 0x7E).
    fn esc(&mut self, intermediates: &[u8], byte: u8) {}

    /// An OSC string split on `;` (so there is always at least one part),
    /// terminated by BEL, ST (ESC `\`) or any other ESC. The bytes are raw:
    /// text in them is UTF-8 left undecoded.
    fn osc(&mut self, params: &[&[u8]]) {}
}

/// The numeric parameters of a CSI sequence.
///
/// Parameters are separated by `;`. Each is a group of up to
/// [`MAX_SUBPARAMS`] values separated by `:`, the first being the parameter's
/// own value, as in the ITU T.416 color `38:2::255:128:0`. An empty value (the
/// first in `ESC [ ; 5 H`) is `None`, so the caller applies its own default.
/// Values saturate at `u16::MAX`.
#[derive(Clone)]
pub struct Params {
    vals: [[Option<u16>; MAX_SUBPARAMS]; MAX_PARAMS],
    lens: [u8; MAX_PARAMS],
    len: u8,
}

// Written out: a derived `Default` unrolls into about 4 KB of wasm stores.
impl Default for Params {
    fn default() -> Params {
        Params { vals: [[None; MAX_SUBPARAMS]; MAX_PARAMS], lens: [0; MAX_PARAMS], len: 0 }
    }
}

impl Params {
    /// The number of parameters (`;`-separated groups). `ESC [ m` has none,
    /// `ESC [ ; m` has two, both empty.
    pub fn len(&self) -> usize {
        usize::from(self.len)
    }

    /// Whether there are no parameters at all.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The value of parameter `i`: `None` when it is empty or missing.
    pub fn get(&self, i: usize) -> Option<u16> {
        self.sub(i).first().copied().flatten()
    }

    /// Parameter `i`'s colon-separated values, its own value first, so a
    /// plain parameter gives one value and `38:5:196` gives three. Empty for
    /// a missing parameter.
    pub fn sub(&self, i: usize) -> &[Option<u16>] {
        if i < self.len() { &self.vals[i][..usize::from(self.lens[i])] } else { &[] }
    }

    /// Every parameter's group of values, in order (see [`Params::sub`]).
    pub fn iter(&self) -> impl Iterator<Item = &[Option<u16>]> {
        (0..self.len()).map(move |i| self.sub(i))
    }
}

impl fmt::Debug for Params {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    Ground,
    Escape,
    EscInter,
    CsiEntry,
    CsiParam,
    CsiInter,
    CsiIgnore,
    Osc,
    /// Inside a DCS, SOS, PM or APC string, all of it ignored.
    Str,
    /// Just after the ESC that ended an OSC or ignored string: like
    /// `Escape`, except that `\` (completing ST) is swallowed.
    StrEsc,
}

/// The DEC ANSI parser with UTF-8 decoding. Feed it with [`Parser::advance`].
#[derive(Clone, Debug, Default)]
pub struct Parser {
    state: State,
    params: Params,
    /// Drop digits and colons until the next `;` (a group overflowed), or for
    /// good (the parameters did).
    skip: bool,
    private: Option<u8>,
    inter: [u8; MAX_INTERMEDIATES],
    ninter: u8,
    /// Too many intermediates: consume the sequence but don't dispatch it.
    overflow: bool,
    osc: Vec<u8>,
    /// Continuation bytes the UTF-8 character in progress still needs, the
    /// code point so far, and the range the next byte must fall in.
    need: u8,
    cp: u32,
    lo: u8,
    hi: u8,
}

impl Parser {
    /// A parser in the ground state.
    pub fn new() -> Parser {
        Parser::default()
    }

    /// Parses `bytes`, reporting to `out`. A sequence or character may span
    /// any number of calls: the parser keeps its place between them.
    pub fn advance(&mut self, bytes: &[u8], out: &mut impl Perform) {
        for &b in bytes {
            self.byte(b, out);
        }
    }

    fn byte<P: Perform>(&mut self, b: u8, out: &mut P) {
        if self.need > 0 {
            if (self.lo..=self.hi).contains(&b) {
                self.cp = (self.cp << 6) | u32::from(b & 0x3F);
                self.need -= 1;
                (self.lo, self.hi) = (0x80, 0xBF);
                if self.need == 0 {
                    out.print(char::from_u32(self.cp).unwrap_or(REPLACEMENT));
                }
                return;
            }
            // The character is cut short: replace it, then handle `b` anew.
            self.need = 0;
            out.print(REPLACEMENT);
        }
        if b == 0x18 || b == 0x1A {
            self.state = State::Ground;
            return out.execute(b);
        }
        if b == 0x1B {
            let next = match self.state {
                State::Osc | State::Str => State::StrEsc,
                _ => State::Escape,
            };
            if self.state == State::Osc {
                self.osc_dispatch(out);
            }
            self.clear();
            self.state = next;
            return;
        }
        match self.state {
            State::Ground => self.ground(b, out),
            State::Escape => self.escape(b, out),
            State::StrEsc if b == b'\\' => self.state = State::Ground,
            State::StrEsc => self.escape(b, out),
            State::EscInter => match b {
                0x00..=0x1F => out.execute(b),
                0x20..=0x2F => self.collect(b),
                0x30..=0x7E => {
                    self.state = State::Ground;
                    if !self.overflow {
                        out.esc(&self.inter[..usize::from(self.ninter)], b);
                    }
                }
                0x7F => {}
                _ => {
                    self.state = State::Ground;
                    self.ground(b, out);
                }
            },
            State::CsiEntry | State::CsiParam | State::CsiInter => match b {
                0x00..=0x1F => out.execute(b),
                0x20..=0x2F => {
                    self.state = State::CsiInter;
                    self.collect(b);
                }
                0x40..=0x7E => {
                    self.state = State::Ground;
                    if !self.overflow {
                        let inter = &self.inter[..usize::from(self.ninter)];
                        out.csi(&self.params, inter, self.private, b);
                    }
                }
                0x7F => {}
                _ if self.state == State::CsiInter => self.state = State::CsiIgnore,
                b'0'..=b'9' | b':' | b';' => {
                    self.state = State::CsiParam;
                    self.param(b);
                }
                b'<'..=b'?' if self.state == State::CsiEntry => {
                    self.state = State::CsiParam;
                    self.private = Some(b);
                }
                _ => self.state = State::CsiIgnore,
            },
            State::CsiIgnore => match b {
                0x00..=0x1F => out.execute(b),
                0x40..=0x7E => self.state = State::Ground,
                _ => {}
            },
            State::Osc => match b {
                0x07 => {
                    self.state = State::Ground;
                    self.osc_dispatch(out);
                }
                0x20.. if self.osc.len() < MAX_OSC => self.osc.push(b),
                _ => {}
            },
            State::Str => {}
        }
    }

    fn ground<P: Perform>(&mut self, b: u8, out: &mut P) {
        let (need, lo, hi) = match b {
            0x00..=0x1F => return out.execute(b),
            0x20..=0x7E => return out.print(char::from(b)),
            0x7F => return,
            0xC2..=0xDF => (1, 0x80, 0xBF),
            0xE0 => (2, 0xA0, 0xBF),
            0xE1..=0xEC | 0xEE..=0xEF => (2, 0x80, 0xBF),
            0xED => (2, 0x80, 0x9F),
            0xF0 => (3, 0x90, 0xBF),
            0xF1..=0xF3 => (3, 0x80, 0xBF),
            0xF4 => (3, 0x80, 0x8F),
            _ => return out.print(REPLACEMENT),
        };
        self.cp = u32::from(b & (0x3F >> need));
        (self.need, self.lo, self.hi) = (need, lo, hi);
    }

    fn escape<P: Perform>(&mut self, b: u8, out: &mut P) {
        match b {
            0x00..=0x1F => out.execute(b),
            0x20..=0x2F => {
                self.state = State::EscInter;
                self.collect(b);
            }
            b'[' => self.state = State::CsiEntry,
            b']' => {
                self.state = State::Osc;
                self.osc.clear();
            }
            b'P' | b'X' | b'^' | b'_' => self.state = State::Str,
            0x30..=0x7E => {
                self.state = State::Ground;
                out.esc(&[], b);
            }
            0x7F => {}
            _ => {
                self.state = State::Ground;
                self.ground(b, out);
            }
        }
    }

    fn clear(&mut self) {
        self.params.len = 0;
        (self.skip, self.overflow) = (false, false);
        (self.private, self.ninter) = (None, 0);
    }

    fn collect(&mut self, b: u8) {
        match self.inter.get_mut(usize::from(self.ninter)) {
            Some(slot) => (*slot, self.ninter) = (b, self.ninter + 1),
            None => self.overflow = true,
        }
    }

    fn param(&mut self, b: u8) {
        let p = &mut self.params;
        if p.len == 0 {
            (p.len, p.lens[0], p.vals[0][0]) = (1, 1, None);
        }
        let i = usize::from(p.len) - 1;
        let n = usize::from(p.lens[i]);
        match b {
            b';' => {
                self.skip = i + 1 == MAX_PARAMS;
                if !self.skip {
                    p.len += 1;
                    (p.lens[i + 1], p.vals[i + 1][0]) = (1, None);
                }
            }
            b':' if n == MAX_SUBPARAMS => self.skip = true,
            b':' if !self.skip => (p.lens[i], p.vals[i][n]) = (p.lens[i] + 1, None),
            b'0'..=b'9' if !self.skip => {
                let v = &mut p.vals[i][n - 1];
                let d = u16::from(b - b'0');
                *v = Some(v.unwrap_or(0).saturating_mul(10).saturating_add(d));
            }
            _ => {}
        }
    }

    fn osc_dispatch<P: Perform>(&self, out: &mut P) {
        let mut parts: [&[u8]; MAX_OSC_PARAMS] = [&[]; MAX_OSC_PARAMS];
        let mut n = 0;
        for part in self.osc.splitn(MAX_OSC_PARAMS, |&c| c == b';') {
            (parts[n], n) = (part, n + 1);
        }
        out.osc(&parts[..n]);
    }
}

#[cfg(test)]
mod tests;
