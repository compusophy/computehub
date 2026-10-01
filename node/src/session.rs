//! Sockets and PTYs: accept on loopback, check the Origin at the handshake,
//! authenticate HELLO, then pump bytes between one WebSocket and one shell in
//! its own PTY until either side ends.
//!
//! Threads per session: this one reads the socket (client → PTY), a pump
//! reads the PTY (PTY → client), and a waiter reaps the shell. The two socket
//! halves share one TCP connection; see `Half`.
//!
//! Admission (TCP accept, WebSocket handshake, HELLO) has one deadline, and
//! nothing a connection does there can keep another one out: no lockout, and
//! when [`PENDING_MAX`] connections are waiting a newcomer evicts the oldest.

use crate::auth::{Origins, Token};
use crate::proto::{self, ClientMsg};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::sync::mpsc::{self, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};
use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::http::StatusCode;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::{CloseFrame, Role, WebSocketConfig};
use tungstenite::{Message, WebSocket};

/// HELLO must arrive this soon after the TCP connection opens: one deadline
/// for the handshake and HELLO together, however the bytes trickle in.
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
/// A wrong token is answered only after this delay (that connection only).
const BAD_TOKEN_DELAY: Duration = Duration::from_secs(1);
/// How long a closing side waits for the peer's close reply.
const CLOSE_GRACE: Duration = Duration::from_secs(2);
/// Connections allowed to wait between TCP accept and a matching token. A
/// newcomer beyond this evicts the one that has waited longest: a real
/// client is through in milliseconds, so parked connections cannot starve it.
pub const PENDING_MAX: usize = 16;
/// Every shell starts at this size, until the client sends RESIZE.
const INITIAL_SIZE: (u16, u16) = (80, 24);
/// Close code for a wrong token.
pub const CLOSE_UNAUTHORIZED: u16 = 4401;
const TEXT_REFUSED: &str = "text messages are not part of the protocol";

/// What the node serves.
#[derive(Debug)]
pub struct Config {
    pub token: Token,
    pub origins: Origins,
    pub max_sessions: usize,
    /// The shell program and its arguments; see [`default_shell`].
    pub shell: Vec<OsString>,
}

/// The default shell: on Windows `pwsh.exe -NoLogo` when it is on `PATH`,
/// else `powershell.exe -NoLogo`; elsewhere `$SHELL`, else `/bin/sh`.
pub fn default_shell() -> Vec<OsString> {
    if cfg!(windows) {
        let on_path = |exe: &str| {
            std::env::var_os("PATH")
                .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(exe).is_file()))
        };
        let program = if on_path("pwsh.exe") { "pwsh.exe" } else { "powershell.exe" };
        vec![program.into(), "-NoLogo".into()]
    } else {
        let shell = std::env::var_os("SHELL").filter(|s| !s.is_empty());
        vec![shell.unwrap_or_else(|| "/bin/sh".into())]
    }
}

/// `<os>/<program file name>`, e.g. `windows/powershell`: never a path.
pub fn shell_label(shell: &[OsString]) -> String {
    let name = shell
        .first()
        .and_then(|p| Path::new(p).file_stem())
        .map(|s| s.to_string_lossy().to_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "shell".into());
    format!("{}/{name}", std::env::consts::OS)
}

/// Accepts connections on `listener` forever, one thread per connection.
pub fn serve(listener: TcpListener, config: Config) {
    let node =
        Arc::new(Node { config, waiting: Mutex::default(), active: Arc::new(AtomicUsize::new(0)) });
    for (id, conn) in (1u64..).zip(listener.incoming()) {
        let Ok(stream) = conn else { continue };
        let Ok(ctl) = stream.try_clone() else { continue };
        if let Some(old) = node.wait(id, ctl) {
            eprintln!("node: #{old}: dropped, waited longest of {PENDING_MAX}");
        }
        let n = Arc::clone(&node);
        let spawned = thread::Builder::new()
            .name(format!("session-{id}"))
            .spawn(move || n.connection(stream, id));
        if let Err(e) = spawned {
            node.done_waiting(id);
            eprintln!("node: #{id}: could not start a thread: {e}");
        }
    }
}

struct Node {
    config: Config,
    /// Connections not yet admitted, oldest first, each with a handle that
    /// can shut it down. A waiting thread blocks only in socket reads and
    /// writes, so a shut-down one ends at once.
    waiting: Mutex<VecDeque<(u64, TcpStream)>>,
    active: Arc<AtomicUsize>,
}

/// One unit of a bounded counter, given back on drop.
struct Slot(Arc<AtomicUsize>);

impl Slot {
    fn take(counter: &Arc<AtomicUsize>, max: usize) -> Option<Slot> {
        counter
            .fetch_update(SeqCst, SeqCst, |n| (n < max).then_some(n + 1))
            .ok()
            .map(|_| Slot(Arc::clone(counter)))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, SeqCst);
    }
}

/// One side of a TCP connection shared by two WebSockets: the session thread
/// reads through one, the pump writes through the other. Each reads its own
/// handle; every write holds one lock and writes all it is given, and
/// tungstenite only ever hands `write` whole frames, so frames from the two
/// halves never interleave.
struct Half {
    rd: TcpStream,
    wr: Arc<Mutex<TcpStream>>,
    /// While set, reads end by then: each read gets the time that is left.
    deadline: Option<Instant>,
}

impl Half {
    fn lock(&self) -> io::Result<MutexGuard<'_, TcpStream>> {
        self.wr.lock().map_err(|_| io::Error::other("socket lock poisoned"))
    }
}

impl Read for Half {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(deadline) = self.deadline {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(io::ErrorKind::TimedOut.into());
            }
            self.rd.set_read_timeout(Some(left))?;
        }
        self.rd.read(buf)
    }
}

impl Write for Half {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.lock()?.write_all(buf)?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.lock()?.flush()
    }
}

type Socket = WebSocket<Half>;

fn ws_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .read_buffer_size(16 * 1024)
        .write_buffer_size(0)
        .max_message_size(Some(1 << 20))
        .max_frame_size(Some(1 << 20))
}

fn refuse(status: StatusCode) -> ErrorResponse {
    let mut r = ErrorResponse::new(status.canonical_reason().map(String::from));
    *r.status_mut() = status;
    r
}

fn timed_out(e: &tungstenite::Error) -> bool {
    matches!(e, tungstenite::Error::Io(e)
        if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut))
}

/// Sends a Close frame, then waits briefly for the peer's reply so the code
/// arrives before the TCP connection goes away.
fn close(ws: &mut Socket, code: u16, reason: &str) {
    let frame = CloseFrame { code: CloseCode::from(code), reason: reason.into() };
    if ws.close(Some(frame)).and_then(|()| ws.flush()).is_ok() {
        drain_for(ws, CLOSE_GRACE);
    }
}

/// Reads and drops whatever arrives for `wait`, or until the socket ends.
fn drain_for(ws: &mut Socket, wait: Duration) {
    ws.get_mut().deadline = Some(Instant::now() + wait);
    while ws.read().is_ok() {}
}

/// ERROR, then close.
fn fail(ws: &mut Socket, code: CloseCode, text: &str) {
    let _ = ws.send(Message::binary(proto::error(text)));
    close(ws, code.into(), "error");
}

impl Node {
    fn waiting(&self) -> MutexGuard<'_, VecDeque<(u64, TcpStream)>> {
        self.waiting.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Lists connection `id` as waiting. When [`PENDING_MAX`] already are,
    /// shuts the oldest down and returns its id.
    fn wait(&self, id: u64, ctl: TcpStream) -> Option<u64> {
        let mut waiting = self.waiting();
        let oldest = if waiting.len() < PENDING_MAX { None } else { waiting.pop_front() };
        waiting.push_back((id, ctl));
        let (old, sock) = oldest?;
        let _ = sock.shutdown(Shutdown::Both);
        Some(old)
    }

    /// Takes `id` off the waiting list; false if it was evicted.
    fn done_waiting(&self, id: u64) -> bool {
        let mut waiting = self.waiting();
        waiting.iter().position(|(i, _)| *i == id).and_then(|at| waiting.remove(at)).is_some()
    }

    fn connection(&self, stream: TcpStream, id: u64) {
        struct Leave<'a>(&'a Node, u64);
        impl Drop for Leave<'_> {
            fn drop(&mut self) {
                self.0.done_waiting(self.1);
            }
        }
        let _leave = Leave(self, id);
        let deadline = Some(Instant::now() + HELLO_TIMEOUT);
        let _ = stream.set_nodelay(true);
        let clones = (stream.try_clone(), stream.try_clone(), stream.try_clone());
        let (Ok(ctl), Ok(wr), Ok(rd2)) = clones else {
            eprintln!("node: #{id}: could not clone the socket");
            return;
        };
        let wr = Arc::new(Mutex::new(wr));
        let half = Half { rd: stream, wr: Arc::clone(&wr), deadline };
        let Some(mut ws) = self.handshake(half, id) else { return };
        // The token is the first and only test; once it passes, no eviction.
        if !self.hello(&mut ws, id) || !self.done_waiting(id) {
            return;
        }
        let Some(active) = Slot::take(&self.active, self.config.max_sessions) else {
            eprintln!("node: #{id}: busy ({} sessions)", self.config.max_sessions);
            fail(&mut ws, CloseCode::Again, "busy");
            return;
        };
        ws.get_mut().deadline = None;
        if ws.get_ref().rd.set_read_timeout(None).is_err() {
            return;
        }
        let out = Half { rd: rd2, wr, deadline: None };
        let out = WebSocket::from_raw_socket(out, Role::Server, Some(ws_config()));
        self.session(ws, out, ctl, id);
        drop(active);
    }

    /// The HTTP upgrade: a missing or unlisted Origin gets 403.
    #[allow(clippy::result_large_err)] // tungstenite's callback signature
    fn handshake(&self, half: Half, id: u64) -> Option<Socket> {
        let check = |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
            let origin = req.headers().get("origin").and_then(|v| v.to_str().ok());
            if !self.config.origins.allows(origin) {
                let shown: String = origin.unwrap_or("none").chars().take(80).collect();
                eprintln!("node: #{id}: refused origin {shown}");
                return Err(refuse(StatusCode::FORBIDDEN));
            }
            Ok(resp)
        };
        tungstenite::accept_hdr_with_config(half, check, Some(ws_config())).ok()
    }

    /// Waits for HELLO (the socket's deadline bounds the wait). True when
    /// the token matched. A wrong one is answered after [`BAD_TOKEN_DELAY`],
    /// spent reading, so an eviction still ends it at once.
    fn hello(&self, ws: &mut Socket, id: u64) -> bool {
        let bytes = loop {
            match ws.read() {
                Ok(Message::Binary(b)) => break b,
                Ok(Message::Text(_)) => {
                    fail(ws, CloseCode::Protocol, TEXT_REFUSED);
                    return false;
                }
                Ok(Message::Close(_)) => return false,
                Ok(_) => {}
                Err(e) if timed_out(&e) => {
                    eprintln!("node: #{id}: no hello in time");
                    close(ws, 1008, "hello timeout");
                    return false;
                }
                Err(_) => return false,
            }
        };
        let error = match proto::decode_client(&bytes) {
            Ok(ClientMsg::Hello { token }) if self.config.token.matches(&token) => return true,
            Ok(ClientMsg::Hello { .. }) => {
                eprintln!("node: #{id}: wrong token");
                drain_for(ws, BAD_TOKEN_DELAY);
                close(ws, CLOSE_UNAUTHORIZED, "unauthorized");
                return false;
            }
            Ok(_) => "expected hello".into(),
            Err(e) => e.to_string(),
        };
        fail(ws, CloseCode::Protocol, &error);
        false
    }

    fn session(&self, mut ws: Socket, out: Socket, ctl: TcpStream, id: u64) {
        let (cols, rows) = INITIAL_SIZE;
        let label = shell_label(&self.config.shell);
        let pty = match Pty::open(&self.config.shell, cols, rows) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("node: #{id}: could not start {label}: {e}");
                fail(&mut ws, CloseCode::Error, "could not start the shell");
                return;
            }
        };
        let Pty { master, reader, writer, mut child, mut killer } = pty;
        let master = Arc::new(Mutex::new(Some(master)));
        let writer = Arc::new(Mutex::new(writer));
        eprintln!("node: #{id}: session started ({label})");
        // READY goes out before the pump exists, so it always comes first.
        let ready = ws.send(Message::binary(proto::ready(cols, rows, &label))).is_ok();

        let exited = Arc::new(AtomicBool::new(false));
        let closing = Arc::new(AtomicBool::new(false));
        let (code_tx, code_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let waiter = {
            let (master, exited) = (Arc::clone(&master), Arc::clone(&exited));
            thread::spawn(move || {
                // Exit codes are u32 on Windows; EXIT carries the same bits.
                let code = child.wait().map_or(-1, |s| s.exit_code() as i32);
                exited.store(true, SeqCst);
                let _ = code_tx.send(code);
                // ConPTY ends the output pipe only once the pseudoconsole
                // closes, which flushes the last frame for the pump.
                drop(take(&master));
            })
        };
        let pump = {
            let (pty, ctl) = ((reader, Arc::clone(&writer)), ctl.try_clone().ok());
            let closing = Arc::clone(&closing);
            thread::spawn(move || pump(pty, out, (code_rx, done_rx), ctl, &closing))
        };

        if ready {
            read_loop(&mut ws, &writer, &master, &closing);
        }

        // The client left, broke the protocol, or acknowledged EXIT. Shut the
        // socket first: a pump stuck writing to it must get back to draining
        // the PTY, or closing the pseudoconsole would wait on it forever.
        drop(done_tx);
        let _ = ctl.shutdown(Shutdown::Both);
        if !exited.load(SeqCst) {
            let _ = killer.kill();
        }
        drop(take(&master));
        drop(writer);
        let code = pump.join().ok().flatten();
        let _ = waiter.join();
        match code {
            Some(c) => eprintln!("node: #{id}: session ended, exit code {c}"),
            None => eprintln!("node: #{id}: session ended, client left"),
        }
    }
}

type Master = Arc<Mutex<Option<Box<dyn MasterPty + Send>>>>;
type PtyIn = Arc<Mutex<Box<dyn Write + Send>>>;

fn take(master: &Master) -> Option<Box<dyn MasterPty + Send>> {
    master.lock().unwrap_or_else(PoisonError::into_inner).take()
}

/// Writes to the shell's input. A failure means the shell is gone, and EXIT
/// follows, so it is not an error here.
fn input(pty_in: &PtyIn, bytes: &[u8]) {
    let mut w = pty_in.lock().unwrap_or_else(PoisonError::into_inner);
    let _ = w.write_all(bytes).and_then(|()| w.flush());
}

/// ConPTY (Windows) opens by asking the terminal where its cursor is and
/// renders nothing until it hears back. A new session's cursor is at the
/// origin, so the node answers for the client and drops the question: a
/// client that never answers cursor reports still gets its prompt, and one
/// that does cannot answer twice (a stray answer would read as a keypress).
const CURSOR_QUERY: &[u8] = b"\x1b[6n";
const CURSOR_AT_ORIGIN: &[u8] = b"\x1b[1;1R";
/// How much of a session's first output may hold that question.
const CURSOR_QUERY_WINDOW: usize = if cfg!(windows) { 4096 } else { 0 };

/// Client → PTY until the client closes, leaves, or breaks the protocol.
fn read_loop(ws: &mut Socket, pty_in: &PtyIn, master: &Master, closing: &AtomicBool) {
    loop {
        let bytes = match ws.read() {
            Ok(Message::Binary(b)) => b,
            Ok(Message::Text(_)) => return fail(ws, CloseCode::Protocol, TEXT_REFUSED),
            Ok(Message::Close(_)) => {
                // Answer a close the client started; ours was answered.
                if !closing.load(SeqCst) {
                    let _ = ws.flush();
                }
                return;
            }
            Ok(_) => continue,
            Err(_) => return,
        };
        match proto::decode_client(&bytes) {
            Ok(ClientMsg::Data(d)) => input(pty_in, d),
            Ok(ClientMsg::Resize { cols, rows }) => {
                let size = PtySize { rows, cols, pixel_width: 0, pixel_height: 0 };
                if let Some(m) = master.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
                    let _ = m.resize(size);
                }
            }
            Ok(ClientMsg::Hello { .. }) => {
                return fail(ws, CloseCode::Protocol, "unexpected hello");
            }
            Err(e) => return fail(ws, CloseCode::Protocol, &e.to_string()),
        }
    }
}

/// Takes the first [`CURSOR_QUERY`] out of `chunk`, if it holds one.
fn strip_cursor_query(chunk: &[u8]) -> Option<Vec<u8>> {
    let at = chunk.windows(CURSOR_QUERY.len()).position(|w| w == CURSOR_QUERY)?;
    Some([&chunk[..at], &chunk[at + CURSOR_QUERY.len()..]].concat())
}

/// PTY → client until the PTY ends; then EXIT with the shell's code (from
/// the waiter) and a close, unless the session thread is already `done`.
/// Keeps draining after the client is gone, because ConPTY will not close
/// while its output is unread. Returns the code if EXIT was sent.
fn pump(
    (mut pty_out, pty_in): (Box<dyn Read + Send>, PtyIn),
    mut out: Socket,
    (code, done): (mpsc::Receiver<i32>, mpsc::Receiver<()>),
    ctl: Option<TcpStream>,
    closing: &AtomicBool,
) -> Option<i32> {
    let mut buf = vec![0u8; proto::MAX_DATA];
    let mut open = true;
    let mut query_window = CURSOR_QUERY_WINDOW;
    loop {
        let n = match pty_out.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            // EIO on Unix once the last process on the terminal is gone.
            Err(_) => break,
        };
        let mut chunk = &buf[..n];
        let stripped;
        if query_window > 0 {
            query_window = query_window.saturating_sub(n);
            if let Some(rest) = strip_cursor_query(chunk) {
                input(&pty_in, CURSOR_AT_ORIGIN);
                query_window = 0;
                stripped = rest;
                chunk = &stripped;
            }
        }
        if !chunk.is_empty() {
            open = open && out.send(Message::binary(proto::data(chunk))).is_ok();
        }
    }
    let code = code.recv().unwrap_or(-1);
    let mut sent = None;
    if open && matches!(done.try_recv(), Err(TryRecvError::Empty)) {
        closing.store(true, SeqCst);
        if out.send(Message::binary(proto::exit(code))).is_ok() {
            sent = Some(code);
        }
        let frame = CloseFrame { code: CloseCode::Normal, reason: "exit".into() };
        let _ = out.close(Some(frame)).and_then(|()| out.flush());
        // The session thread ends when the client answers the close.
        let _ = done.recv_timeout(CLOSE_GRACE);
    }
    if let Some(ctl) = ctl {
        let _ = ctl.shutdown(Shutdown::Both);
    }
    sent
}

/// A shell in a fresh PTY, split into the parts the three threads own.
struct Pty {
    master: Box<dyn MasterPty + Send>,
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

impl Pty {
    fn open(shell: &[OsString], cols: u16, rows: u16) -> Result<Pty, String> {
        let (program, args) = shell.split_first().ok_or("no shell program")?;
        let size = PtySize { rows, cols, pixel_width: 0, pixel_height: 0 };
        let pair = native_pty_system().openpty(size).map_err(|e| e.to_string())?;
        let reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        let mut cmd = CommandBuilder::new(program);
        cmd.args(args);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
        if let Some(home) = home.filter(|h| Path::new(h).is_dir()) {
            cmd.cwd(home);
        }
        let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
        // The slave keeps the pseudoconsole alive on Windows; only the master
        // may hold it, so that dropping the master ends the session.
        drop(pair.slave);
        let killer = child.clone_killer();
        Ok(Pty { master: pair.master, reader, writer, child, killer })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_carry_only_the_program_name() {
        let os = std::env::consts::OS;
        let label = |v: &[&str]| shell_label(&v.iter().map(OsString::from).collect::<Vec<_>>());
        assert_eq!(label(&["powershell.exe", "-NoLogo"]), format!("{os}/powershell"));
        assert_eq!(label(&["/usr/bin/zsh"]), format!("{os}/zsh"));
        assert_eq!(label(&["PWSH.EXE"]), format!("{os}/pwsh"));
        assert_eq!(label(&[]), format!("{os}/shell"));
        assert!(!shell_label(&default_shell()).contains(['\\', ':']));
    }

    #[test]
    fn the_cursor_query_is_taken_out_once() {
        assert_eq!(strip_cursor_query(b"\x1b[6n"), Some(vec![]));
        assert_eq!(
            strip_cursor_query(b"\x1b[?9001h\x1b[6nPS> \x1b[6n"),
            Some(b"\x1b[?9001hPS> \x1b[6n".to_vec())
        );
        assert_eq!(strip_cursor_query(b"\x1b[6"), None);
        assert_eq!(strip_cursor_query(b""), None);
    }
}
