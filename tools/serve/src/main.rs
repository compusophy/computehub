//! Development-only static file server for `dist/`; never shipped.
//! `serve <dir> <port>` answers GET and HEAD on `127.0.0.1:<port>`: `.../`
//! is `index.html`, `.wasm` is `application/wasm`, nothing may be cached. A
//! path that could leave `<dir>` (a `.`, `..` or empty segment, a backslash or
//! a drive colon) is a 404; paths are not percent-decoded, so `%2e` hides none.

#![forbid(unsafe_code)]

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::{env, fs, process, thread};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let [dir, port] = args.as_slice() else { fail("usage: serve <dir> <port>") };
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
        thread::spawn(move || handle(stream, &root).unwrap_or_else(|e| eprintln!("serve: {e}")));
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("serve: {msg}");
    process::exit(2);
}

/// Answers one request, then closes the connection.
fn handle(mut stream: TcpStream, root: &Path) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?.take(16 * 1024));
    let (mut line, mut header) = (String::new(), String::new());
    reader.read_line(&mut line)?;
    // Skip the headers: nothing here depends on them.
    while reader.read_line(&mut header)? > 0 && !header.trim_end().is_empty() {
        header.clear();
    }
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
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
         Cache-Control: no-store\r\nAllow: GET, HEAD\r\nConnection: close\r\n\r\n"
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

fn mime(path: &Path) -> &'static str {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext.to_ascii_lowercase().as_str() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "wasm" => "application/wasm",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "json" => "application/json",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
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
