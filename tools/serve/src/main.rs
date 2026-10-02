//! Development-only static file server for `dist/`; never shipped.
//! `serve <dir> <port> [--plain]` answers GET and HEAD on `127.0.0.1:<port>`:
//! `.../` is `index.html`, `.wasm` is `application/wasm`, nothing may be
//! cached, and unless `--plain` the page is cross-origin isolated, as
//! deploy.sh makes it (programs need that). A path that could leave `<dir>`
//! (a `.`, `..` or empty segment, a backslash or a drive colon) is a 404;
//! paths are not percent-decoded, so `%2e` hides none. Two POSTs stand in for
//! the site's server functions: `/api/ai` streams a mock model's answer
//! ([`sse`]; to a request with tools, a scripted agent's next step, [`agent`]);
//! `/api/feedback` prints the body to stdout, prefixed with
//! `feedback: `, and answers 201 with `{"url":"local"}`.

#![forbid(unsafe_code)]

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::{env, fs, process, thread};

/// The headers every response sends unless `--plain`.
const ISOLATION: &str = "Cross-Origin-Opener-Policy: same-origin\r\n\
    Cross-Origin-Embedder-Policy: require-corp\r\nCross-Origin-Resource-Policy: same-origin\r\n";

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let (dir, port, extra) = match args.as_slice() {
        [dir, port] => (dir, port, ISOLATION),
        [dir, port, plain] if plain == "--plain" => (dir, port, ""),
        _ => fail("usage: serve <dir> <port> [--plain]"),
    };
    let port: u16 = port.parse().unwrap_or_else(|_| fail(&format!("bad port {port:?}")));
    let root = PathBuf::from(dir);
    if !root.is_dir() {
        fail(&format!("{dir} is not a directory"));
    }
    let listener = TcpListener::bind(("127.0.0.1", port))
        .unwrap_or_else(|e| fail(&format!("cannot listen on 127.0.0.1:{port}: {e}")));
    println!("serving {} on http://127.0.0.1:{port}/", root.display());
    for stream in listener.incoming().flatten() {
        let root = root.clone();
        let serve =
            move || handle(stream, &root, extra).unwrap_or_else(|e| eprintln!("serve: {e}"));
        thread::spawn(serve);
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("serve: {msg}");
    process::exit(2);
}

/// Answers one request with `extra` headers, then closes the connection.
fn handle(mut stream: TcpStream, root: &Path, extra: &str) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?.take(2 << 20));
    let (mut line, mut header, mut len) = (String::new(), String::new(), 0);
    reader.read_line(&mut line)?;
    // Of the headers only a body's length matters.
    while reader.read_line(&mut header)? > 0 && !header.trim_end().is_empty() {
        if let Some(("content-length", n)) = header.to_ascii_lowercase().split_once(':') {
            len = n.trim().parse().unwrap_or(0).min(1 << 20);
        }
        header.clear();
    }
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    if method == "POST" && matches!(target, "/api/ai" | "/api/feedback") {
        let mut body = vec![0; len];
        reader.read_exact(&mut body)?;
        let body = String::from_utf8_lossy(&body);
        if target == "/api/feedback" {
            println!("feedback: {body}");
            let (json, close) = (r#"{"url":"local"}"#, "Connection: close");
            let head = "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\n";
            let n = json.len();
            write!(stream, "{head}Content-Length: {n}\r\n{close}\r\n{extra}\r\n{json}")?;
            return stream.flush();
        }
        println!("{method} {target}");
        let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n";
        write!(stream, "{head}{extra}\r\n")?;
        // Uneven parts, apart in time: lines and characters split across reads.
        for part in sse(&body).as_bytes().chunks(77) {
            stream.write_all(part)?;
            stream.flush()?;
            thread::sleep(Duration::from_millis(20));
        }
        return Ok(());
    }
    let head = method == "HEAD";
    let found = || resolve(root, target).and_then(|p| Some((mime(&p), fs::read(p).ok()?)));
    let (status, ty, body) = match (method == "GET" || head).then(found) {
        None => ("405 Method Not Allowed", "text/plain", b"405\n".to_vec()),
        Some(Some((ty, body))) => ("200 OK", ty, body),
        Some(None) => ("404 Not Found", "text/plain", b"404\n".to_vec()),
    };
    println!("{method} {target} {status}");
    let len = body.len();
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {ty}\r\nContent-Length: {len}\r\n\
         Cache-Control: no-store\r\nAllow: GET, HEAD\r\nConnection: close\r\n{extra}\r\n"
    )?;
    stream.write_all(if head { &[] } else { &body })?;
    stream.flush()
}

/// The file under `root` a request target names; `None` if it could leave it.
fn resolve(root: &Path, target: &str) -> Option<PathBuf> {
    let path = target.split(['?', '#']).next()?.strip_prefix('/')?;
    let index = if path.is_empty() || path.ends_with('/') { "index.html" } else { "" };
    let mut out = root.to_path_buf();
    for seg in [path, index].concat().split('/') {
        if seg.is_empty() || seg == "." || seg == ".." || seg.contains(['\\', ':']) {
            return None;
        }
        out.push(seg);
    }
    Some(out)
}

/// The mock's answer to a chat request `body`: chat.completion.chunk lines, a
/// usage chunk, `[DONE]` and the [`receipt`]. With tools, [`agent`]'s step. When the last message's
/// content (else the body) says "app", a sentence and a fenced `app` block of
/// Studio's counter, a line a chunk; else "Hello from the mock model.", a word a chunk.
fn sse(body: &str) -> String {
    if body.contains("\"tools\":[") {
        return agent(body);
    }
    let last = body.rfind("\"content\"").map_or(body, |i| &body[i..]);
    let pieces: Vec<&str> = if last.contains("app") {
        let program = include_str!("../../../programs/studio/samples/counter.app");
        let lines = program.split_inclusive('\n');
        ["Here is a counter.\n\n", "```app\n"].into_iter().chain(lines).chain(["```\n"]).collect()
    } else {
        "Hello from the mock model.".split_inclusive(' ').collect()
    };
    let head = r#"data: {"id":"mock","object":"chat.completion.chunk","created":0,"model":"mock","#;
    let mut out = String::new();
    for p in &pieces {
        let p = p.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
        out +=
            &format!("{head}\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{p}\"}}}}]}}\n\n");
    }
    let (i, o) = (body.len() / 4, pieces.len());
    let usage = format!("\"prompt_tokens\":{i},\"completion_tokens\":{o}");
    out + &format!("{head}\"choices\":[],\"usage\":{{{usage}}}}}\n\ndata: [DONE]\n\n")
        + &receipt(i, o)
}

/// The receipt the site's function ends a stream with: tokens in and out, and their cost in
/// millionths of a dollar at the default model's list price ($1.40 and $4.40 a million).
fn receipt(i: usize, o: usize) -> String {
    format!("\n: receipt in={i} out={o} microusd={}\n\n", (i * 14 + o * 44) / 10)
}

/// The scripted agent's next step for a request with tools, read off the raw JSON `body` (its
/// latest screen's lines, quotes escaped): asked about error reports, as a person turns them
/// off: open Settings, its Privacy tab, the switch, close Settings, then say so. Anything else
/// it answers in words. A call comes as providers stream one: its name, then its arguments in
/// pieces.
fn agent(body: &str) -> String {
    // A line break in a JSON string may come as \n or as \u000a.
    let body = &body.replace(r"\u000a", r"\n");
    let screen = body.rfind("Screen ").map_or("", |i| &body[i..]);
    let line = |want: &str| screen.split("\\n").map(str::trim).find(|l| l.contains(want));
    let first = |l: &str| l.split(' ').next().unwrap_or("").to_string();
    let (head, end) = (r#"data: {"id":"mock","choices":[{"index":0,"delta":"#, "}]}\n\n");
    // The task: the last message the user wrote (the system prompt names reports too).
    let task = body.rsplit(r#""role":"user","content":""#).next().unwrap_or("");
    let reports = task.split("\\n").next().unwrap_or("").to_ascii_lowercase().contains("report");
    let step = if !reports {
        None
    } else if !body.contains(r#""role":"tool""#) {
        Some(("open_app", r#"{"name":"settings"}"#.to_string()))
    } else if let Some(l) = line(r#"tab \"Privacy\""#).filter(|l| !l.ends_with("selected")) {
        Some(("click", format!(r#"{{"ref":"{}"}}"#, first(l))))
    } else if let Some(l) = line(r#"switch \"Send error reports automatically\" on"#) {
        Some(("click", format!(r#"{{"ref":"{}"}}"#, first(l))))
    } else {
        let w = line("(settings)").map(first);
        w.map(|w| ("window", format!(r#"{{"window":"{w}","action":"close"}}"#)))
    };
    let mut out = String::new();
    let say = |text: &str| format!("{head}{{\"content\":\"{text}\"}}{end}");
    match step {
        Some((name, args)) => {
            let call = format!(
                r#"{{"index":0,"id":"call_{name}","type":"function","function":{{"name":"{name}","arguments":""}}}}"#
            );
            out += &format!("{head}{{\"tool_calls\":[{call}]}}{end}");
            for piece in
                args.as_bytes().chunks(6).map(|p| String::from_utf8_lossy(p).replace('"', "\\\""))
            {
                let delta = format!(
                    r#"{{"tool_calls":[{{"index":0,"function":{{"arguments":"{piece}"}}}}]}}"#
                );
                out += &format!("{head}{delta}{end}");
            }
        }
        None if reports => {
            out += &say("Error reports are off.");
        }
        None => out += &say("I am the mock model: ask me to turn error reports off."),
    }
    out + r#"data: {"id":"mock","choices":[],"usage":{"prompt_tokens":1000,"completion_tokens":20}}"#
        + "\n\ndata: [DONE]\n\n"
        + &receipt(1000, 20)
}

fn mime(path: &Path) -> &'static str {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    #[rustfmt::skip]
    let types = [("html", "text/html; charset=utf-8"), ("js", "text/javascript; charset=utf-8"),
        ("mjs", "text/javascript; charset=utf-8"), ("wasm", "application/wasm"),
        ("css", "text/css; charset=utf-8"), ("svg", "image/svg+xml"), ("png", "image/png"),
        ("ico", "image/x-icon"), ("json", "application/json"),
        ("txt", "text/plain; charset=utf-8")];
    types.iter().find(|t| t.0 == ext).map_or("application/octet-stream", |t| t.1)
}

#[test]
fn paths_stay_inside_the_root() {
    let root = Path::new("dist");
    let ok = |target: &str, file: &str| Some(root.join(file)) == resolve(root, target);
    assert!(ok("/", "index.html") && ok("/?v=1", "index.html"));
    assert!(ok("/os_bg.wasm#x", "os_bg.wasm") && ok("/a/b.js", "a/b.js"));
    assert!(ok("/a/", "a/index.html") && ok("/%2e%2e/x", "%2e%2e/x"));
    let bad = r" x * http://h/x /.. /../x /a/../../x /./x //x /a//b /a\..\x /C:/x /c:x";
    for target in bad.split(' ') {
        assert_eq!(resolve(root, target), None, "{target}");
    }
    let t = ["a/os_bg.WASM", "index.html", "LICENSE"].map(|p| mime(Path::new(p)));
    assert_eq!(t, ["application/wasm", "text/html; charset=utf-8", "application/octet-stream"]);
}

#[test]
fn responses_isolate_the_page() {
    let lines: Vec<&str> = ISOLATION.split_terminator("\r\n").collect();
    assert_eq!(lines[0], "Cross-Origin-Opener-Policy: same-origin");
    assert_eq!(lines[1], "Cross-Origin-Embedder-Policy: require-corp");
    assert_eq!(lines[2..], ["Cross-Origin-Resource-Policy: same-origin"]);
}

#[test]
fn the_mock_streams_hello_or_an_app() {
    let hi = sse(r#"[{"role":"system","content":"apps"},{"role":"user","content":"hi"}]"#);
    let lines: Vec<&str> = hi.split_terminator("\n\n").collect();
    assert!(lines[0].ends_with(r#""choices":[{"index":0,"delta":{"content":"Hello "}}]}"#));
    assert!(lines.len() == 8 && lines[5].contains(r#""usage":{"#) && lines[6] == "data: [DONE]");
    // The receipt last, as the site's function ends a stream: the usage's tokens, priced.
    assert_eq!(lines[7], "\n: receipt in=16 out=5 microusd=44");
    let app = sse(r#"[{"role":"user","content":"a counter app"}]"#);
    assert!(app.contains(r#":"```app\n"}"#) && app.contains(r#":"label \"Counter\";\n"}"#));
}

#[test]
fn the_mock_agent_turns_error_reports_off_a_step_at_a_time() {
    // A step's call: its name and its arguments, joined from their pieces.
    let call = |sse: String| {
        let name = sse.split(r#""name":""#).nth(1).and_then(|n| n.split('"').next());
        let pieces = sse.split(r#""arguments":""#).skip(1).map(|p| p.split(r#""}}]}"#).next());
        let args: String = pieces.flatten().collect();
        (name.unwrap_or("").to_string(), args.replace("\\\"", "\""))
    };
    let ask = r#"{"tools":[],"messages":[{"role":"user","content":"turn off error reports"}]}"#;
    let step = |screen: &str| {
        let tool =
            [r#"turn off error reports"},{"role":"tool","content":"Screen 1x1\n"#, screen].concat();
        call(sse(&ask.replace("turn off error reports", &tool)))
    };
    assert_eq!(call(sse(ask)), ("open_app".into(), r#"{"name":"settings"}"#.into()));
    let nav =
        r#"w1 \"Settings\" (settings)\n  e1 tab \"Appearance\" selected\n  e3 tab \"Privacy\"\n"#;
    assert_eq!(step(nav), ("click".into(), r#"{"ref":"e3"}"#.into()));
    // Line breaks escaped as the Assistant's JSON writes them.
    let unicode = nav.replace(r"\n", r"\u000a");
    assert_eq!(step(&unicode), ("click".into(), r#"{"ref":"e3"}"#.into()));
    let on = r#"w1 \"Settings\" (settings)\n  e3 tab \"Privacy\" selected
  e8 switch \"Send error reports automatically\" on\n"#
        .replace('\n', "\\n");
    assert_eq!(step(&on), ("click".into(), r#"{"ref":"e8"}"#.into()));
    let off = on.replace("\" on", "\" off");
    assert_eq!(step(&off), ("window".into(), r#"{"window":"w1","action":"close"}"#.into()));
    let done = agent(
        &ask.replace("reports\"}", "reports\"},{\"role\":\"tool\",\"content\":\"Screen 1x1\"}"),
    );
    assert!(
        done.contains(r#"{"content":"Error reports are off."}"#)
            && done.ends_with("[DONE]\n\n\n: receipt in=1000 out=20 microusd=1488\n\n")
    );
    let other = agent(r#"{"tools":[],"messages":[{"content":"hi"}]}"#);
    assert!(other.contains("ask me to turn error reports off"));
}

#[test]
fn feedback_is_printed_and_acknowledged() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let body = r#"{"text":"hi"}"#;
    let len = body.len();
    write!(client, "POST /api/feedback HTTP/1.1\r\nContent-Length: {len}\r\n\r\n{body}").unwrap();
    handle(listener.accept().unwrap().0, Path::new("dist"), "").unwrap();
    let mut got = String::new();
    client.read_to_string(&mut got).unwrap();
    assert!(got.starts_with("HTTP/1.1 201 Created\r\n"), "{got}");
    assert!(got.contains("Content-Length: 15\r\n") && got.ends_with("\r\n\r\n{\"url\":\"local\"}"));
}
