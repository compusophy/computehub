//! A parser for the bytes a terminal receives: Paul Williams' DEC ANSI state
//! machine (<https://vt100.net/emu/dec_ansi_parser>), as xterm follows it, with
//! UTF-8 decoding. [`Parser::advance`] takes chunks of any size and reports
//! printable characters, C0 controls and complete CSI, ESC and OSC sequences to
//! a [`Perform`]; screen state is the `term` crate's job. A new crate.
//!
//! Invariants: chunk boundaries never change what is reported. Invalid UTF-8
//! yields U+FFFD once per maximal invalid subsequence; 0x80 to 0x9F are never
//! C1 controls. C0 controls execute even inside CSI and ESC (not strings), DEL
//! is ignored, CAN and SUB abort a sequence, ESC always starts a new one (first
//! dispatching an OSC; the `\` of ST is swallowed), and DCS, SOS, PM and APC
//! are consumed unreported. Parameters, intermediates and OSC bytes are capped
//! by the `MAX_` constants (a misplaced byte or one intermediate too many drops
//! the sequence), so nothing panics and memory stays bounded.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use core::fmt;

/// The most parameters a CSI keeps; later ones are dropped.
pub const MAX_PARAMS: usize = 32;
/// The most `:`-separated values one CSI parameter keeps (its own included).
pub const MAX_SUBPARAMS: usize = 8;
/// The most intermediate bytes a CSI or ESC may carry; one more drops it.
pub const MAX_INTERMEDIATES: usize = 2;
/// The most OSC payload bytes kept; the rest is dropped.
pub const MAX_OSC: usize = 8192;
/// The most `;`-separated parts of an OSC; the last keeps any further `;`.
pub const MAX_OSC_PARAMS: usize = 16;

const REPLACEMENT: char = '\u{FFFD}';

/// What the parser reports; every method defaults to doing nothing.
#[allow(unused_variables)]
pub trait Perform {
    /// A printable character (U+FFFD for invalid UTF-8).
    fn print(&mut self, c: char) {}
    /// A C0 control other than ESC, including a CAN or SUB that aborted.
    fn execute(&mut self, byte: u8) {}
    /// `ESC [`, an optional private marker (`? > < =`), parameters,
    /// intermediates (0x20 to 0x2F) and the final `action` (0x40 to 0x7E).
    fn csi(&mut self, params: &Params, intermediates: &[u8], private: Option<u8>, action: u8) {}
    /// `ESC`, intermediates (0x20 to 0x2F) and a final `byte` (0x30 to 0x7E).
    fn esc(&mut self, intermediates: &[u8], byte: u8) {}
    /// An OSC split on `;` (at least one part), ended by BEL or ESC; raw bytes.
    fn osc(&mut self, params: &[&[u8]]) {}
}

/// A CSI's parameters: `;`-separated groups of up to [`MAX_SUBPARAMS`]
/// `:`-separated values (`38:2::255:128:0`); empty is `None`, values saturate.
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
    /// The number of groups: none in `ESC [ m`, two empty ones in `ESC [ ; m`.
    pub fn len(&self) -> usize {
        usize::from(self.len)
    }

    /// Whether there are no parameters.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The value of parameter `i`: `None` when it is empty or missing.
    pub fn get(&self, i: usize) -> Option<u16> {
        self.sub(i).first().copied().flatten()
    }

    /// Parameter `i`'s values, its own first (`38:5:196` gives three); empty
    /// when missing.
    pub fn sub(&self, i: usize) -> &[Option<u16>] {
        if i < self.len() { &self.vals[i][..usize::from(self.lens[i])] } else { &[] }
    }

    /// Every group, in order (see [`Params::sub`]).
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
    /// Inside an ignored DCS, SOS, PM or APC string.
    Str,
    /// After the ESC ending a string: `Escape`, but `\` (ST) is swallowed.
    StrEsc,
}

/// The DEC ANSI parser with UTF-8 decoding.
#[derive(Clone, Debug, Default)]
pub struct Parser {
    state: State,
    params: Params,
    /// Drop digits and colons until the next `;` (or for good, past the last).
    skip: bool,
    private: Option<u8>,
    inter: [u8; MAX_INTERMEDIATES],
    ninter: u8,
    /// Too many intermediates: consume the sequence but don't dispatch it.
    overflow: bool,
    osc: Vec<u8>,
    /// UTF-8: continuation bytes still needed, the code point so far, and the
    /// range the next byte must fall in.
    need: u8,
    cp: u32,
    lo: u8,
    hi: u8,
}

impl Parser {
    /// Parses `bytes`, reporting to `out`; sequences may span calls.
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
            // Cut short: replace it, then handle `b` anew.
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
            State::StrEsc if b == b'\\' => self.state = State::Ground,
            State::Escape | State::StrEsc | State::EscInter => {
                let fresh = self.state != State::EscInter;
                match b {
                    0x00..=0x1F => out.execute(b),
                    0x20..=0x2F => {
                        self.state = State::EscInter;
                        self.collect(b);
                    }
                    b'[' if fresh => self.state = State::CsiEntry,
                    b']' if fresh => {
                        self.state = State::Osc;
                        self.osc.clear();
                    }
                    b'P' | b'X' | b'^' | b'_' if fresh => self.state = State::Str,
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
                }
            }
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
