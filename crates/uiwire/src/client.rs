//! The process side: a GUI program's loop is [`open`], then [`Client::show`]
//! a frame after each [`Client::next_event`] until [`Event::Close`].
//!
//! ```no_run
//! let mut ui = uiwire::client::open()?;
//! while ui.next_event()? != uiwire::Event::Close {
//!     ui.show(&uiwire::Frame { title: "Hello".into(), ..Default::default() })?;
//! }
//! # Ok::<(), std::io::Error>(())
//! ```

use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};

use crate::{Event, Frame, MAX_FRAME};

/// The device a program reads its events from.
pub const EVENTS: &str = "/dev/events";
/// The device a program writes its frames to.
pub const DRAW: &str = "/dev/draw";
/// The most bytes one read of [`EVENTS`] returns; a longer event comes in parts this long.
pub const PART: usize = 1 << 16;

/// A window's two channels: events in (one per read, or in [`PART`]s), frames out (one per write).
pub struct Client<R: Read, W: Write> {
    events: R,
    draw: W,
    buf: Vec<u8>,
}

/// Opens [`EVENTS`] and [`DRAW`].
pub fn open() -> io::Result<Client<File, File>> {
    Ok(Client::new(File::open(EVENTS)?, OpenOptions::new().write(true).open(DRAW)?))
}

impl<R: Read, W: Write> Client<R, W> {
    /// A client over any reader and writer (tests, other transports).
    pub fn new(events: R, draw: W) -> Self {
        Self { events, draw, buf: vec![0; MAX_FRAME + 1] }
    }

    /// Blocks for the next event, joining the [`PART`]s of a long one. The
    /// end of the events is `UnexpectedEof`; a malformed event is `InvalidData`.
    pub fn next_event(&mut self) -> io::Result<Event> {
        let mut n = 0;
        loop {
            let got = match self.events.read(&mut self.buf[n..]) {
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                read => read?,
            };
            n += got;
            match Event::decode(&self.buf[..n]) {
                Some(ev) => return Ok(ev),
                None if n == 0 => return Err(ErrorKind::UnexpectedEof.into()),
                None if got == PART && n < self.buf.len() => {}
                None => return Err(io::Error::new(ErrorKind::InvalidData, "malformed event")),
            }
        }
    }

    /// Sends `frame` in one write. One that breaks a cap (size, nodes, depth: checked without
    /// decoding it, which would ship the decoder in every program) is `InvalidInput` and sends
    /// nothing; a partial write is `WriteZero`.
    pub fn show(&mut self, frame: &Frame) -> io::Result<()> {
        let Some(bytes) = frame.encode_checked() else {
            return Err(io::Error::new(ErrorKind::InvalidInput, "frame breaks the protocol"));
        };
        loop {
            match self.draw.write(&bytes) {
                Ok(n) if n == bytes.len() => return self.draw.flush(),
                Ok(_) => return Err(io::Error::new(ErrorKind::WriteZero, "frame written in part")),
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }
}
