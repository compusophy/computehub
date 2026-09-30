//! Development-only static file server. Never shipped.
//!
//! `cargo run -p serve --release -- <dir> <port>` serves `<dir>` on
//! `127.0.0.1:<port>`: GET and HEAD, `/` (and any `.../`) is `index.html`,
//! `.wasm` is `application/wasm`, and every response says
//! `Cache-Control: no-store`. A path that could leave `<dir>` (a `..` or `.`
//! segment, an empty segment such as `//x`, a backslash or a drive colon) is
//! a 404. Paths are not percent-decoded, so an escape cannot hide in `%2e`.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::{env, fs, process, thread};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let (root, port) = match args.as_slice() {
        [dir, port] => match port.parse::<u16>() {
            Ok(port) => (PathBuf::from(dir), port),
            Err(_) => fail(&format!("bad port {port:?}")),
        },
        _ => fail("usage: serve <dir> <port>"),
    };
    if !root.is_dir() {
        fail(&format!("{} is not a directory", root.display()));
    }
    let listener = TcpListener::bind(("127.0.0.1", port))
        .unwrap_or_else(|e| fail(&format!("cannot listen on 127.0.0.1:{port}: {e}")));
    println!("serving {} on http://127.0.0.1:{port}/", root.display());
    for stream in listener.incoming().flatten() {
        let root = root.clone();
        thread::spawn(move || {
            if let Err(e) = handle(stream, &root) {
                eprintln!("serve: {e}");
            }
        });
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("serve: {msg}");
    process::exit(2);
}

/// Answers one request, then closes the connection.
fn handle(stream: TcpStream, root: &Path) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?.take(16 * 1024));
    let mut line = String::new();
    reader.read_line(&mut line)?;
    // Skip the headers: nothing here depends on them.
    let mut header = String::new();
    while reader.read_line(&mut header)? > 0 && !header.trim_end().is_empty() {
        header.clear();
    }
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let head = method == "HEAD";
    let (status, ty, body) = if method != "GET" && !head {
        ("405 Method Not Allowed", "text/plain", b"405\n".to_vec())
    } else {
        match resolve(root, target).and_then(|p| Some((mime(&p), fs::read(p).ok()?))) {
            Some((ty, body)) => ("200 OK", ty, body),
            None => ("404 Not Found", "text/plain", b"404\n".to_vec()),
        }
    };
    println!("{method} {target} {status}");
    let mut out = stream;
    write!(
        out,
        "HTTP/1.1 {status}\r\nContent-Type: {ty}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nAllow: GET, HEAD\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    if !head {
        out.write_all(&body)?;
    }
    out.flush()
}

/// The file under `root` a request target names, or `None` if it could
/// name anything outside `root`.
fn resolve(root: &Path, target: &str) -> Option<PathBuf> {
    let path = target.split(['?', '#']).next()?.strip_prefix('/')?;
    let path = if path.is_empty() || path.ends_with('/') {
        format!("{path}index.html")
    } else {
        path.to_owned()
    };
    let mut out = root.to_path_buf();
    for seg in path.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." || seg.contains(['\\', ':']) {
            return None;
        }
        out.push(seg);
    }
    Some(out)
}

/// The Content-Type for a file, by extension.
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(mime(Path::new("a/os_bg.WASM")), "application/wasm");
        assert_eq!(mime(Path::new("index.html")), "text/html; charset=utf-8");
        assert_eq!(mime(Path::new("LICENSE")), "application/octet-stream");
    }
}
