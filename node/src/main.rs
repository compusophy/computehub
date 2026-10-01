//! The computehub node's command line. Everything else is in the library.

#![forbid(unsafe_code)]

use computehub_node::auth::{self, Origins, Token};
use computehub_node::session::{self, Config};
use std::ffi::OsString;
use std::net::{Ipv4Addr, TcpListener};
use std::process::ExitCode;

const DEFAULT_URL: &str = "https://computehub-sigma.vercel.app";
const DEFAULT_PORT: u16 = 7878;
const DEFAULT_MAX_SESSIONS: usize = 8;
const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "\
computehub-node: serves real shells on this machine to compusophyOS terminals

usage: computehub-node [options] [-- <shell program> [args]...]

options:
  --port <n>             listen on 127.0.0.1:<n> (default 7878; 0 picks one)
  --allow-origin <o>     also accept pages from origin <o>, e.g.
                         http://localhost:3000 (repeatable)
  --url <page url>       the OS page the pairing link opens
                         (default https://computehub-sigma.vercel.app);
                         its origin is the only one allowed by default
  --max-sessions <n>     concurrent shells (default 8)
  --help                 print this and exit
  --version              print the version and exit

The node listens on loopback only. Each connection must present the token
from the pairing link it prints at startup.
";

#[derive(Debug, PartialEq)]
struct Cli {
    port: u16,
    origins: Vec<String>,
    url: String,
    max_sessions: usize,
    shell: Option<Vec<OsString>>,
}

/// `Ok(None)` means `--help` or `--version` was handled.
fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Option<Cli>, String> {
    let mut cli = Cli {
        port: DEFAULT_PORT,
        origins: Vec::new(),
        url: DEFAULT_URL.into(),
        max_sessions: DEFAULT_MAX_SESSIONS,
        shell: None,
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let arg = arg.into_string().map_err(|a| format!("not UTF-8: {}", a.to_string_lossy()))?;
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (arg, None),
        };
        let mut value = || {
            let next = || args.next().and_then(|v| v.into_string().ok());
            inline.clone().or_else(next).ok_or_else(|| format!("{flag} needs a value"))
        };
        match flag.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("computehub-node {VERSION}");
                return Ok(None);
            }
            "--port" => {
                let v = value()?;
                cli.port = v.parse().map_err(|_| format!("bad port: {v}"))?;
            }
            "--allow-origin" => cli.origins.push(value()?),
            "--url" => cli.url = value()?,
            "--max-sessions" => {
                let v = value()?;
                let n = v.parse().ok().filter(|n| (1..=256).contains(n));
                cli.max_sessions = n.ok_or_else(|| format!("--max-sessions wants 1..=256: {v}"))?;
            }
            "--" => {
                let rest: Vec<OsString> = args.by_ref().collect();
                if rest.is_empty() {
                    return Err("-- needs a shell program".into());
                }
                cli.shell = Some(rest);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(Some(cli))
}

fn main() -> ExitCode {
    let result = match parse(std::env::args_os().skip(1)) {
        Ok(Some(cli)) => run(cli),
        Ok(None) => return ExitCode::SUCCESS,
        Err(e) => {
            eprintln!(
                "computehub-node: {e}

{USAGE}"
            );
            return ExitCode::from(2);
        }
    };
    let Err(e) = result;
    eprintln!("computehub-node: {e}");
    ExitCode::FAILURE
}

/// Runs the node; returns only if it cannot start.
fn run(cli: Cli) -> Result<std::convert::Infallible, String> {
    let url = cli.url.trim_end_matches('/');
    if url.contains('#') {
        return Err(format!("--url must not have a #fragment: {url}"));
    }
    let origins = allowlist(url, &cli.origins)?;
    let token = Token::generate().map_err(|e| format!("no random source for the token: {e}"))?;
    let shell = cli.shell.unwrap_or_else(session::default_shell);
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, cli.port))
        .map_err(|e| format!("cannot listen on 127.0.0.1:{}: {e}", cli.port))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();

    println!("computehub node {VERSION}");
    println!("listening on 127.0.0.1:{port} (loopback only)");
    println!("shell: {}   max sessions: {}", session::shell_label(&shell), cli.max_sessions);
    println!("allowed origins:");
    for o in origins.iter() {
        println!("  {o}");
    }
    println!();
    println!("pair a compusophyOS terminal by opening this link (it holds the token):");
    println!("{url}/#node={port}&token={}", token.hex());
    println!();

    session::serve(listener, Config { token, origins, max_sessions: cli.max_sessions, shell });
    Err("the listener stopped".into())
}

/// The pages that may connect: the `--url` page's origin, then each
/// `--allow-origin`. Nothing else, not even a local dev server, by default.
fn allowlist(url: &str, extra: &[String]) -> Result<Origins, String> {
    let page = auth::origin_of(url).ok_or_else(|| format!("--url is not http(s): {url}"))?;
    let mut origins = Origins::default();
    for o in [&page].into_iter().chain(extra) {
        origins.add(o)?;
    }
    Ok(origins)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &str) -> Result<Option<Cli>, String> {
        parse(args.split_whitespace().map(OsString::from))
    }

    #[test]
    fn defaults() {
        let cli = p("").expect("parse").expect("run");
        assert_eq!((cli.port, cli.url.as_str(), cli.max_sessions), (7878, DEFAULT_URL, 8));
        assert!(cli.origins.is_empty() && cli.shell.is_none());
    }

    #[test]
    fn flags_values_and_shell() {
        let args = "--port 9000 --allow-origin http://localhost:3000 --allow-origin=http://a:1                     --url=http://localhost:8080 --max-sessions 2 -- bash -l --port";
        let cli = p(args).expect("parse").expect("run");
        assert_eq!(cli.port, 9000);
        assert_eq!(cli.origins, ["http://localhost:3000", "http://a:1"]);
        assert_eq!(cli.url, "http://localhost:8080");
        assert_eq!(cli.max_sessions, 2);
        assert_eq!(cli.shell, Some(vec!["bash".into(), "-l".into(), "--port".into()]));
    }

    #[test]
    fn only_the_page_and_extras_are_trusted() {
        let o = allowlist(DEFAULT_URL, &[]).expect("default");
        assert_eq!(o.iter().collect::<Vec<_>>(), [DEFAULT_URL]);
        for dev in ["http://localhost:8080", "http://127.0.0.1:8080"] {
            assert!(!o.allows(Some(dev)), "{dev} trusted by default");
        }
        let o = allowlist("http://localhost:8080/os", &["http://a:1".into()]).expect("dev");
        assert_eq!(o.iter().collect::<Vec<_>>(), ["http://localhost:8080", "http://a:1"]);
        assert!(allowlist("x", &[]).is_err() && allowlist(DEFAULT_URL, &["*".into()]).is_err());
    }

    #[test]
    fn errors() {
        for bad in
            ["--port", "--port 70000", "--max-sessions 0", "--max-sessions=x", "--", "--x", "x"]
        {
            assert!(p(bad).is_err(), "{bad}");
        }
    }
}
