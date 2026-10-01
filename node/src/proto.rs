//! The wire protocol: pure encode and decode, no I/O.
//!
//! Every message is one binary WebSocket message. The first byte is the
//! opcode; integers are little-endian. Text messages are a protocol error.
//!
//! | op   | name   | direction     | payload                                                  |
//! |------|--------|---------------|----------------------------------------------------------|
//! | 0x01 | HELLO  | client → node | `[u8 version = 1][32-byte token]`                        |
//! | 0x02 | READY  | node → client | `[u8 version = 1][u16 cols][u16 rows][UTF-8 os/shell]`   |
//! | 0x03 | DATA   | both          | raw bytes; node → client at most 64 KiB per message      |
//! | 0x04 | RESIZE | client → node | `[u16 cols][u16 rows]`, clamped to 1..=1000 x 1..=500    |
//! | 0x05 | EXIT   | node → client | `[i32 exit code]`, then the node closes                  |
//! | 0x06 | ERROR  | node → client | UTF-8 message, then the node closes                      |

use std::fmt;

/// The protocol version carried by HELLO and READY.
pub const VERSION: u8 = 1;
pub const HELLO: u8 = 0x01;
pub const READY: u8 = 0x02;
pub const DATA: u8 = 0x03;
pub const RESIZE: u8 = 0x04;
pub const EXIT: u8 = 0x05;
pub const ERROR: u8 = 0x06;

/// Length of the pairing token in bytes.
pub const TOKEN_LEN: usize = 32;
/// The largest message the node sends: one opcode byte plus output.
pub const MAX_MESSAGE: usize = 64 * 1024;
/// The most PTY output one DATA message carries.
pub const MAX_DATA: usize = MAX_MESSAGE - 1;
pub const MAX_COLS: u16 = 1000;
pub const MAX_ROWS: u16 = 500;

/// Why a message could not be decoded. The `Display` text is what the node
/// sends back in ERROR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// A zero-length message: no opcode.
    Empty,
    /// An opcode this side does not accept.
    UnknownOpcode(u8),
    /// The payload has the wrong length for its opcode.
    Length { opcode: u8, len: usize },
    /// HELLO or READY carried a version other than [`VERSION`].
    Version(u8),
    /// A text field is not UTF-8.
    Utf8,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Error::Empty => f.write_str("empty message"),
            Error::UnknownOpcode(op) => write!(f, "unknown opcode 0x{op:02x}"),
            Error::Length { opcode, len } => {
                write!(f, "bad payload length {len} for opcode 0x{opcode:02x}")
            }
            Error::Version(v) => write!(f, "unsupported protocol version {v}"),
            Error::Utf8 => f.write_str("text field is not UTF-8"),
        }
    }
}

impl std::error::Error for Error {}

/// A message from a client (a compusophyOS terminal) to the node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientMsg<'a> {
    Hello {
        token: [u8; TOKEN_LEN],
    },
    Data(&'a [u8]),
    /// Already clamped by [`clamp`].
    Resize {
        cols: u16,
        rows: u16,
    },
}

/// A message from the node to a client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeMsg<'a> {
    Ready { cols: u16, rows: u16, info: &'a str },
    Data(&'a [u8]),
    Exit(i32),
    Error(&'a str),
}

/// Clamps a terminal size to 1..=[`MAX_COLS`] x 1..=[`MAX_ROWS`].
pub fn clamp(cols: u16, rows: u16) -> (u16, u16) {
    (cols.clamp(1, MAX_COLS), rows.clamp(1, MAX_ROWS))
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn split(msg: &[u8]) -> Result<(u8, &[u8]), Error> {
    msg.split_first().map(|(op, rest)| (*op, rest)).ok_or(Error::Empty)
}

fn exact(opcode: u8, payload: &[u8], len: usize) -> Result<(), Error> {
    if payload.len() == len { Ok(()) } else { Err(Error::Length { opcode, len: payload.len() }) }
}

/// Decodes one client → node message.
pub fn decode_client(msg: &[u8]) -> Result<ClientMsg<'_>, Error> {
    let (op, p) = split(msg)?;
    match op {
        HELLO => {
            let (&version, rest) = p.split_first().ok_or(Error::Length { opcode: op, len: 0 })?;
            if version != VERSION {
                return Err(Error::Version(version));
            }
            let token = rest.try_into().map_err(|_| Error::Length { opcode: op, len: p.len() })?;
            Ok(ClientMsg::Hello { token })
        }
        DATA => Ok(ClientMsg::Data(p)),
        RESIZE => {
            exact(op, p, 4)?;
            let (cols, rows) = clamp(u16_at(p, 0), u16_at(p, 2));
            Ok(ClientMsg::Resize { cols, rows })
        }
        other => Err(Error::UnknownOpcode(other)),
    }
}

/// Decodes one node → client message (what a client runs; the tests use it).
pub fn decode_node(msg: &[u8]) -> Result<NodeMsg<'_>, Error> {
    let (op, p) = split(msg)?;
    match op {
        READY => {
            if p.len() < 5 {
                return Err(Error::Length { opcode: op, len: p.len() });
            }
            if p[0] != VERSION {
                return Err(Error::Version(p[0]));
            }
            let info = std::str::from_utf8(&p[5..]).map_err(|_| Error::Utf8)?;
            Ok(NodeMsg::Ready { cols: u16_at(p, 1), rows: u16_at(p, 3), info })
        }
        DATA if p.len() <= MAX_DATA => Ok(NodeMsg::Data(p)),
        DATA => Err(Error::Length { opcode: op, len: p.len() }),
        EXIT => {
            exact(op, p, 4)?;
            Ok(NodeMsg::Exit(i32::from_le_bytes([p[0], p[1], p[2], p[3]])))
        }
        ERROR => std::str::from_utf8(p).map(NodeMsg::Error).map_err(|_| Error::Utf8),
        other => Err(Error::UnknownOpcode(other)),
    }
}

fn message(op: u8, payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(1 + payload.len());
    v.push(op);
    v.extend_from_slice(payload);
    v
}

pub fn hello(token: &[u8; TOKEN_LEN]) -> Vec<u8> {
    let mut v = message(HELLO, &[VERSION]);
    v.extend_from_slice(token);
    v
}

pub fn ready(cols: u16, rows: u16, info: &str) -> Vec<u8> {
    let mut v = message(READY, &[VERSION]);
    v.extend_from_slice(&cols.to_le_bytes());
    v.extend_from_slice(&rows.to_le_bytes());
    v.extend_from_slice(info.as_bytes());
    v
}

pub fn data(bytes: &[u8]) -> Vec<u8> {
    message(DATA, bytes)
}

pub fn resize(cols: u16, rows: u16) -> Vec<u8> {
    let mut v = message(RESIZE, &cols.to_le_bytes());
    v.extend_from_slice(&rows.to_le_bytes());
    v
}

pub fn exit(code: i32) -> Vec<u8> {
    message(EXIT, &code.to_le_bytes())
}

pub fn error(text: &str) -> Vec<u8> {
    message(ERROR, text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_messages_round_trip() {
        let token = [0xab; TOKEN_LEN];
        assert_eq!(decode_client(&hello(&token)), Ok(ClientMsg::Hello { token }));
        assert_eq!(decode_client(&data(b"ls\r")), Ok(ClientMsg::Data(b"ls\r")));
        assert_eq!(decode_client(&data(b"")), Ok(ClientMsg::Data(b"")));
        assert_eq!(decode_client(&resize(132, 43)), Ok(ClientMsg::Resize { cols: 132, rows: 43 }));
    }

    #[test]
    fn node_messages_round_trip() {
        let r = ready(80, 24, "windows/powershell");
        assert_eq!(&r[..6], &[READY, 1, 80, 0, 24, 0]);
        assert_eq!(
            decode_node(&r),
            Ok(NodeMsg::Ready { cols: 80, rows: 24, info: "windows/powershell" })
        );
        assert_eq!(decode_node(&data(b"\x1b[0m")), Ok(NodeMsg::Data(b"\x1b[0m")));
        for code in [0, 1, -1, i32::MIN, i32::MAX, 0xC000_013Au32 as i32] {
            assert_eq!(decode_node(&exit(code)), Ok(NodeMsg::Exit(code)));
        }
        assert_eq!(exit(-2), vec![EXIT, 0xfe, 0xff, 0xff, 0xff]);
        assert_eq!(decode_node(&error("busy")), Ok(NodeMsg::Error("busy")));
        let big = vec![7u8; MAX_DATA];
        assert_eq!(data(&big).len(), MAX_MESSAGE);
        assert_eq!(decode_node(&data(&big)), Ok(NodeMsg::Data(&big[..])));
    }

    #[test]
    fn resize_is_little_endian_and_clamped() {
        assert_eq!(resize(0x0102, 0x0304), vec![RESIZE, 0x02, 0x01, 0x04, 0x03]);
        let r = |c, r| match decode_client(&resize(c, r)) {
            Ok(ClientMsg::Resize { cols, rows }) => (cols, rows),
            other => panic!("{other:?}"),
        };
        assert_eq!(r(0, 0), (1, 1));
        assert_eq!(r(u16::MAX, u16::MAX), (1000, 500));
        assert_eq!(r(1000, 500), (1000, 500));
        assert_eq!(r(1001, 501), (1000, 500));
        assert_eq!(r(7, 0), (7, 1));
    }

    #[test]
    fn malformed_client_messages_are_rejected() {
        let token = [1u8; TOKEN_LEN];
        let good = hello(&token);
        assert_eq!(decode_client(&[]), Err(Error::Empty));
        assert_eq!(decode_client(&[HELLO]), Err(Error::Length { opcode: HELLO, len: 0 }));
        assert_eq!(
            decode_client(&good[..good.len() - 1]),
            Err(Error::Length { opcode: HELLO, len: 32 })
        );
        let mut long = good.clone();
        long.push(0);
        assert_eq!(decode_client(&long), Err(Error::Length { opcode: HELLO, len: 34 }));
        let mut v2 = good;
        v2[1] = 2;
        assert_eq!(decode_client(&v2), Err(Error::Version(2)));
        assert_eq!(
            decode_client(&[RESIZE, 1, 2, 3]),
            Err(Error::Length { opcode: RESIZE, len: 3 })
        );
        assert_eq!(
            decode_client(&[RESIZE, 1, 2, 3, 4, 5]),
            Err(Error::Length { opcode: RESIZE, len: 5 })
        );
        // Node → client opcodes, and anything unassigned, are not accepted.
        for op in [0x00, READY, EXIT, ERROR, 0x07, 0xff] {
            assert_eq!(decode_client(&[op, 0, 0, 0, 0]), Err(Error::UnknownOpcode(op)));
        }
    }

    #[test]
    fn malformed_node_messages_are_rejected() {
        assert_eq!(decode_node(&[READY, 1, 80, 0]), Err(Error::Length { opcode: READY, len: 3 }));
        assert_eq!(decode_node(&[READY, 9, 80, 0, 24, 0]), Err(Error::Version(9)));
        assert_eq!(decode_node(&[READY, 1, 80, 0, 24, 0, 0xff]), Err(Error::Utf8));
        assert_eq!(decode_node(&[EXIT, 0, 0]), Err(Error::Length { opcode: EXIT, len: 2 }));
        assert_eq!(decode_node(&[ERROR, 0xc3]), Err(Error::Utf8));
        assert_eq!(decode_node(&[HELLO]), Err(Error::UnknownOpcode(HELLO)));
        let big = data(&vec![0u8; MAX_DATA + 1]);
        assert_eq!(decode_node(&big), Err(Error::Length { opcode: DATA, len: MAX_DATA + 1 }));
    }

    #[test]
    fn errors_read_as_plain_text() {
        assert_eq!(Error::UnknownOpcode(0x2a).to_string(), "unknown opcode 0x2a");
        assert_eq!(Error::Version(3).to_string(), "unsupported protocol version 3");
    }
}
