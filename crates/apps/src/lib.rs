//! The console a terminal's shell runs on. The Terminal is a program (`programs/terminal`, off
//! the boot download) that draws its screen as a [`uiwire::Node::Screen`]; its window holds a
//! [`Console`]: [`SHELL`] (a program too, `programs/sh`) on a console the size the Terminal
//! asks for ([`uiwire::Request::Tty`]), what it types ([`uiwire::Request::Input`]) going in, and
//! what the shell writes and its end going back as events. The window's keys, text and wheel go
//! to the Terminal as events too ([`input`]); [`wire_key`] and [`wire_mods`] say keys as the wire
//! does. A program the shell starts may ask the AI ([`asks`]), as the coding agent does.

#![forbid(unsafe_code)]

use std::mem;

use ui::kernel::{Spawn, wire};
use ui::{AppEvent, Cx, Key, Mods};
use uiwire::{Event, Frame, Request};
use vfs::Vfs;

/// The shell a terminal runs.
pub const SHELL: &str = "/bin/sh";

/// Hands `ask` what process `pid`, if the window's shell started it, asks of the AI in `frame`,
/// a frame it wrote to /dev/draw: its Ai and AiCancel requests, answered on its /dev/events as
/// any program's. It has no window, so the rest of the frame shows nothing; nor does any other
/// process's frame, which is not even read.
pub fn asks(cx: &Cx<'_>, pid: u32, frame: &[u8], ask: &mut dyn FnMut(Request)) {
    let ai = |r: &Request| matches!(r, Request::Ai { .. } | Request::AiCancel { .. });
    let frame = cx.kernel.owns(pid).then(|| Frame::decode(frame)).flatten();
    frame.into_iter().flat_map(|f| f.requests).filter(ai).for_each(ask);
}

/// A terminal's shell, while it runs, and whether it started (it starts once).
#[derive(Debug, Default)]
pub struct Console {
    pub pid: Option<u32>,
    started: bool,
}

impl Console {
    /// The Terminal (process `to`) asked for a console `cols` x `rows`: the first time the shell
    /// starts on one (in the guest's home, the whole tree its roots; the symbol fonts load then
    /// too), later it is resized. If the shell cannot start, the Terminal hears why, and its end
    /// (127).
    pub fn tty(&mut self, cx: &mut Cx<'_>, to: Option<u32>, cols: u16, rows: u16) {
        let told = self.start(cx, cols, rows);
        tell(cx, to, told);
    }

    fn start(&mut self, cx: &mut Cx<'_>, cols: u16, rows: u16) -> Vec<Event> {
        if let Some(pid) = self.pid {
            cx.kernel.resize(pid, cols, rows);
        }
        if mem::replace(&mut self.started, true) {
            return Vec::new();
        }
        cx.load_fallback_fonts();
        let (argv, cwd, roots) = (vec!["sh".into()], Vfs::HOME.into(), vec!["/".into()]);
        let (tty, stdout) = (Some((cols, rows)), wire::Stdout::Console);
        let started = ui::kernel::program(cx.vfs, SHELL)
            .map_err(|missing| if missing { "not found" } else { "cannot execute" })
            .and_then(|program| cx.kernel.spawn(Spawn { argv, program, cwd, tty, stdout, roots }));
        match started {
            Ok(pid) => (self.pid = Some(pid), Vec::new()).1,
            Err(why) => {
                let data = ["sh: ", why, "\r\n"].concat().into_bytes();
                vec![Event::Output { data }, Event::Ended { status: 127 }]
            }
        }
    }

    /// Bytes the Terminal typed go to the shell's console.
    pub fn input(&self, cx: &mut Cx<'_>, data: &[u8]) {
        if let Some(pid) = self.pid.filter(|_| !data.is_empty()) {
            cx.kernel.input(pid, data);
        }
    }

    /// What the shell wrote since, and its end: the Terminal (process `to`) hears them.
    pub fn io(&mut self, cx: &mut Cx<'_>, to: Option<u32>) {
        let heard = self.heard(cx);
        tell(cx, to, heard);
    }

    fn heard(&mut self, cx: &mut Cx<'_>) -> Vec<Event> {
        let Some(pid) = self.pid else { return Vec::new() };
        let data = cx.kernel.take_output(pid);
        let mut out = Vec::new();
        if !data.is_empty() {
            out.push(Event::Output { data });
        }
        if let Some(status) = cx.kernel.reap(pid) {
            (self.pid, _) = (None, out.push(Event::Ended { status }));
        }
        out
    }
}

/// Process `to`, if any, hears `events`.
fn tell(cx: &mut Cx<'_>, to: Option<u32>, events: Vec<Event>) {
    if let Some(to) = to {
        events.iter().for_each(|ev| cx.kernel.post_event(to, &ev.encode()));
    }
}

/// `key` as the wire says it, and the char that goes with it (a character key's, a function
/// key's number), if the wire has it.
pub fn wire_key(key: Key) -> Option<(uiwire::Key, char)> {
    use uiwire::Key as W;
    Some(match key {
        Key::Char(c) => (W::Char, c),
        Key::Space => (W::Char, ' '),
        Key::F(n) => (W::F, char::from(n)),
        Key::Other => return None,
        k => {
            #[rustfmt::skip]
            const ALL: [(Key, W); 14] = [(Key::Enter, W::Enter), (Key::Escape, W::Escape),
                (Key::Tab, W::Tab), (Key::Up, W::Up), (Key::Down, W::Down), (Key::Left, W::Left),
                (Key::Right, W::Right), (Key::Backspace, W::Backspace), (Key::Delete, W::Delete),
                (Key::Home, W::Home), (Key::End, W::End), (Key::PageUp, W::PageUp),
                (Key::PageDown, W::PageDown), (Key::Insert, W::Insert)];
            (ALL.iter().find(|p| p.0 == k)?.1, '\0')
        }
    })
}

/// Modifiers as the wire's bits ([`uiwire::mods`]).
pub fn wire_mods(m: Mods) -> u8 {
    [m.meta, m.alt, m.ctrl, m.shift].iter().fold(0, |b, &on| b << 1 | u8::from(on))
}

/// The window's own input as the Terminal hears it: a key as xterm needs it (printable text
/// comes as text, so a character key only with Ctrl or Alt), text, the wheel.
pub fn input(ev: &AppEvent) -> Option<Event> {
    Some(match ev {
        AppEvent::Key { key, mods } => {
            let (key, ch) = wire_key(*key)?;
            if key == uiwire::Key::Char && !mods.ctrl && !mods.alt {
                return None;
            }
            Event::Key { id: 0, key, mods: wire_mods(*mods), ch }
        }
        AppEvent::Text(text) => Event::Text { text: text.clone() },
        // `as` saturates, and NaN is 0.
        AppEvent::Wheel { dy, .. } => Event::Wheel { dy: *dy as i32 },
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
