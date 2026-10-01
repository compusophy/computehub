//! End to end, in process: a real server on an ephemeral loopback port, a
//! tungstenite client, and a real shell in a real PTY.

#![forbid(unsafe_code)]

use computehub_node::auth::{Origins, Token};
use computehub_node::proto::{self, NodeMsg};
use computehub_node::session::{CLOSE_UNAUTHORIZED, Config, PENDING_MAX, serve};
use std::ffi::OsString;
use std::io::ErrorKind::{TimedOut, WouldBlock};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};
use tungstenite::client::IntoClientRequest;
use tungstenite::handshake::HandshakeError;
use tungstenite::http::HeaderValue;
use tungstenite::{Error, Message, WebSocket};

const TOKEN: [u8; 32] = [0x5a; 32];
const ORIGIN: &str = "http://localhost:8080";

fn start(shell: &[&str], max_sessions: usize) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let mut origins = Origins::default();
    origins.add(ORIGIN).expect("origin");
    let config = Config {
        token: Token::from_bytes(TOKEN),
        origins,
        max_sessions,
        shell: shell.iter().map(OsString::from).collect(),
    };
    thread::spawn(move || serve(listener, config));
    port
}

fn echo_hello() -> &'static [&'static str] {
    if cfg!(windows) {
        &["cmd.exe", "/c", "echo", "hello"]
    } else {
        &["/bin/sh", "-c", "echo hello"]
    }
}

fn long_running() -> &'static [&'static str] {
    if cfg!(windows) {
        &["cmd.exe", "/c", "ping", "-n", "30", "127.0.0.1"]
    } else {
        &["/bin/sh", "-c", "sleep 30"]
    }
}

fn connect(port: u16, origin: Option<&str>) -> Result<WebSocket<TcpStream>, Error> {
    let mut req = format!("ws://127.0.0.1:{port}/").into_client_request()?;
    if let Some(o) = origin {
        req.headers_mut().insert("origin", HeaderValue::from_str(o).expect("header"));
    }
    let stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    match tungstenite::client(req, stream) {
        Ok((ws, _)) => Ok(ws),
        Err(HandshakeError::Failure(e)) => Err(e),
        Err(HandshakeError::Interrupted(_)) => panic!("handshake interrupted"),
    }
}

/// Everything the node said until it closed.
#[derive(Default)]
struct Transcript {
    ready: Option<(u16, u16, String)>,
    output: Vec<u8>,
    exit: Option<i32>,
    error: Option<String>,
    close: Option<u16>,
}

/// Reads until the node closes. Like a minimal terminal, it never answers
/// cursor reports: the node must answer ConPTY's opening query itself.
fn drain(ws: &mut WebSocket<TcpStream>) -> Transcript {
    let mut t = Transcript::default();
    loop {
        match ws.read() {
            Ok(Message::Binary(b)) => match proto::decode_node(&b).expect("node message") {
                NodeMsg::Ready { cols, rows, info } => t.ready = Some((cols, rows, info.into())),
                NodeMsg::Data(d) => t.output.extend_from_slice(d),
                NodeMsg::Exit(c) => t.exit = Some(c),
                NodeMsg::Error(e) => t.error = Some(e.into()),
            },
            Ok(Message::Close(f)) => t.close = f.map(|f| f.code.into()),
            Ok(_) => {}
            Err(Error::ConnectionClosed | Error::AlreadyClosed) => return t,
            Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::ConnectionReset => return t,
            Err(e) => panic!("read: {e}"),
        }
    }
}

#[test]
fn runs_a_command_and_reports_its_exit() {
    let port = start(echo_hello(), 2);
    let mut ws = connect(port, Some(ORIGIN)).expect("handshake");
    ws.send(Message::binary(proto::hello(&TOKEN))).expect("hello");
    let t = drain(&mut ws);
    let os = std::env::consts::OS;
    let shell = if cfg!(windows) { "cmd" } else { "sh" };
    assert_eq!(t.ready, Some((80, 24, format!("{os}/{shell}"))));
    let text = String::from_utf8_lossy(&t.output);
    assert!(text.contains("hello"), "output: {text:?}");
    assert!(!text.contains("\x1b[6n"), "cursor query leaked: {text:?}");
    assert_eq!(t.exit, Some(0));
    assert_eq!(t.error, None);
    assert_eq!(t.close, Some(1000));
}

#[test]
fn wrong_token_is_closed_with_4401_after_a_delay() {
    let port = start(echo_hello(), 2);
    let mut ws = connect(port, Some(ORIGIN)).expect("handshake");
    let started = Instant::now();
    ws.send(Message::binary(proto::hello(&[0x5b; 32]))).expect("hello");
    let t = drain(&mut ws);
    assert!(started.elapsed() >= Duration::from_millis(900));
    assert_eq!(t.close, Some(CLOSE_UNAUTHORIZED));
    assert_eq!(t.ready, None);
    assert!(t.output.is_empty());
}

/// The token, and only the token, decides: wrong tokens from anyone else
/// (no token needed to send them) never lock out the client that has it.
#[test]
fn wrong_tokens_never_keep_the_right_one_out() {
    let port = start(echo_hello(), 2);
    let tries: Vec<_> = (0..6)
        .map(|_| {
            let mut ws = connect(port, Some(ORIGIN)).expect("handshake");
            thread::spawn(move || {
                ws.send(Message::binary(proto::hello(&[0; 32]))).expect("hello");
                drain(&mut ws).close
            })
        })
        .collect();
    for t in tries {
        assert_eq!(t.join().expect("try"), Some(CLOSE_UNAUTHORIZED));
    }
    let mut ws = connect(port, Some(ORIGIN)).expect("refused after wrong tokens");
    ws.send(Message::binary(proto::hello(&TOKEN))).expect("hello");
    let t = drain(&mut ws);
    assert!(t.ready.is_some() && t.exit == Some(0), "not admitted: {:?}", t.error);
}

/// Waiting connections cannot shut out a client: when all PENDING_MAX
/// places are taken, a newcomer evicts the one that waited longest.
#[test]
fn parked_connections_cannot_keep_a_client_out() {
    let port = start(echo_hello(), 2);
    let parked: Vec<_> =
        (0..PENDING_MAX).map(|_| connect(port, Some(ORIGIN)).expect("handshake")).collect();
    let started = Instant::now();
    let mut ws = connect(port, Some(ORIGIN)).expect("dropped while others wait");
    ws.send(Message::binary(proto::hello(&TOKEN))).expect("hello");
    let t = drain(&mut ws);
    assert!(t.ready.is_some() && t.exit == Some(0), "not admitted: {:?}", t.error);
    assert!(started.elapsed() < Duration::from_secs(4), "waited out the parked ones");
    // The oldest was shut at once, not at its deadline.
    let mut oldest = parked.into_iter().next().expect("parked");
    oldest.get_ref().set_read_timeout(Some(Duration::from_secs(1))).expect("timeout");
    match oldest.read() {
        Err(Error::Io(e)) if matches!(e.kind(), WouldBlock | TimedOut) => panic!("still open"),
        other => assert!(other.is_err(), "{other:?}"),
    }
}

/// The admission deadline spans the HTTP upgrade too: a handshake dribbled
/// a byte at a time cannot hold its place past it.
#[test]
fn a_dribbled_handshake_is_cut_off_at_the_deadline() {
    let port = start(echo_hello(), 2);
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_millis(250))).expect("timeout");
    s.write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n").expect("request line");
    let started = Instant::now();
    let closed = loop {
        let mut buf = [0u8; 64];
        match s.write_all(b"x").and_then(|()| s.read(&mut buf)) {
            Err(e) if matches!(e.kind(), WouldBlock | TimedOut) => {}
            _ => break started.elapsed(),
        }
        assert!(started.elapsed() < Duration::from_secs(9), "never cut off");
    };
    assert!(closed >= Duration::from_secs(4), "cut off early: {closed:?}");
}

#[test]
fn bad_or_missing_origin_is_refused_with_403() {
    let port = start(echo_hello(), 2);
    for origin in [Some("https://evil.example"), Some("http://127.0.0.1:8080"), Some("null"), None]
    {
        match connect(port, origin) {
            Err(Error::Http(resp)) => assert_eq!(resp.status(), 403, "{origin:?}"),
            Err(e) => panic!("{origin:?}: {e}"),
            Ok(_) => panic!("{origin:?} was accepted"),
        }
    }
    // The allowlist does not lock anyone out: a good origin still gets in.
    assert!(connect(port, Some(ORIGIN)).is_ok());
}

#[test]
fn protocol_errors_get_error_then_close() {
    let port = start(echo_hello(), 2);
    let mut ws = connect(port, Some(ORIGIN)).expect("handshake");
    ws.send(Message::text("hi")).expect("send");
    let t = drain(&mut ws);
    assert_eq!(t.error.as_deref(), Some("text messages are not part of the protocol"));

    let mut ws = connect(port, Some(ORIGIN)).expect("handshake");
    ws.send(Message::binary(proto::data(b"ls\r"))).expect("send");
    assert_eq!(drain(&mut ws).error.as_deref(), Some("expected hello"));

    let mut ws = connect(port, Some(ORIGIN)).expect("handshake");
    ws.send(Message::binary(vec![0x42])).expect("send");
    assert_eq!(drain(&mut ws).error.as_deref(), Some("unknown opcode 0x42"));
}

#[test]
fn extra_sessions_are_busy() {
    let port = start(long_running(), 1);
    let mut first = connect(port, Some(ORIGIN)).expect("handshake");
    first.send(Message::binary(proto::hello(&TOKEN))).expect("hello");
    let ready = first.read().expect("ready");
    assert!(matches!(proto::decode_node(&ready.into_data()), Ok(NodeMsg::Ready { .. })));

    let mut second = connect(port, Some(ORIGIN)).expect("handshake");
    second.send(Message::binary(proto::hello(&TOKEN))).expect("hello");
    let t = drain(&mut second);
    assert_eq!(t.error.as_deref(), Some("busy"));
    assert_eq!(t.ready, None);

    // Closing the first frees its slot (and kills its shell).
    first.close(None).expect("close");
    let _ = drain(&mut first);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let mut third = connect(port, Some(ORIGIN)).expect("handshake");
        third.send(Message::binary(proto::hello(&TOKEN))).expect("hello");
        let msg = third.read().expect("reply").into_data();
        match proto::decode_node(&msg) {
            Ok(NodeMsg::Ready { .. }) => break,
            other => assert!(Instant::now() < deadline, "slot never freed: {other:?}"),
        }
        thread::sleep(Duration::from_millis(100));
    }
}
