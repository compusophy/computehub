//! The toolbox: compusophyOS's test programs in one std binary for
//! wasm32-wasip1, so they share one copy of std. It runs the applet named by
//! the file name of argv\[0\] (less a `.wasm`), or by argv\[1\] when argv\[0\]
//! names the toolbox itself; the /bin marker of every applet points here.
//! The exit status is the applet's; an unknown applet is 127.
//!
//! Step 1 of R2: `hello` and `spin` run; the other applets say they are not
//! built yet and exit 1.

#![forbid(unsafe_code)]

use std::io::{self, Write};
use std::process::ExitCode;

/// Every applet, in the order `toolbox` lists them.
const APPLETS: [&str; 9] =
    ["hello", "rev", "wc", "spin", "nap", "fstest", "keys", "bench", "selftest"];

/// The status for a name that is no applet: the shell's "not found".
const NOT_FOUND: u8 = 127;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    let (name, args) = applet(&argv);
    let status = run(name, args, &mut io::stdout().lock(), &mut io::stderr().lock());
    ExitCode::from(status)
}

/// The applet `argv` names and its arguments, its own name first.
fn applet(argv: &[String]) -> (&str, &[String]) {
    match argv {
        [first, rest @ ..] if base(first) == "toolbox" && !rest.is_empty() => {
            (base(&rest[0]), rest)
        }
        [first, ..] => (base(first), argv),
        [] => ("", argv),
    }
}

/// A path's file name, less a `.wasm`.
fn base(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".wasm").unwrap_or(name)
}

/// Runs applet `name` on `args` (its own name first), writing to `out` and
/// `err`; its exit status.
fn run(name: &str, args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let words = args.get(1..).unwrap_or(&[]);
    match name {
        "hello" => hello(words, out, err),
        "spin" => spin(),
        _ if APPLETS.contains(&name) => {
            // A failed write to stderr leaves nothing to report it on.
            let _ = writeln!(err, "{name}: not built yet");
            1
        }
        _ => {
            let list = APPLETS.join(" ");
            let _ = writeln!(err, "toolbox: no applet {name:?}; applets: {list}");
            NOT_FOUND
        }
    }
}

/// `hello [word...]`: prints the words joined by spaces, then a newline, in
/// one write. 0, or 1 when stdout fails.
fn hello(words: &[String], out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let mut line = words.join(" ");
    line.push('\n');
    match out.write_all(line.as_bytes()).and_then(|()| out.flush()) {
        Ok(()) => 0,
        Err(e) => {
            let _ = writeln!(err, "hello: {e}");
            1
        }
    }
}

/// `spin`: loops forever without a syscall, so only a kill ends it (Ctrl+C
/// gives 130, closing the window 137): the test that the kernel stops a guest
/// mid-loop.
fn spin() -> ! {
    loop {
        std::hint::spin_loop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split(' ').map(String::from).collect()
    }

    /// Runs `argv` as `main` would: (status, stdout, stderr).
    fn exec(argv: &[String]) -> (u8, String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let (name, rest) = applet(argv);
        let status = run(name, rest, &mut out, &mut err);
        let text = |b: Vec<u8>| String::from_utf8(b).unwrap();
        (status, text(out), text(err))
    }

    /// A stdout whose every write fails.
    struct Closed;

    impl Write for Closed {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("closed"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn the_applet_comes_from_argv0_or_after_toolbox() {
        let name = |s| applet(&args(s)).0.to_owned();
        assert_eq!(name("hello a b"), "hello");
        assert_eq!(name("/bin/rev.wasm"), "rev");
        assert_eq!(name("toolbox wc -l"), "wc");
        assert_eq!(name("bin/toolbox.wasm spin"), "spin");
        assert_eq!(name("toolbox"), "toolbox");
        assert_eq!(applet(&args("toolbox wc -l")).1, args("wc -l"));
        assert_eq!(applet(&[]).0, "");
    }

    #[test]
    fn hello_prints_its_arguments_joined_by_spaces() {
        let ok = |s: &str| (0, s.to_owned(), String::new());
        assert_eq!(exec(&args("hello a b")), ok("a b\n"));
        assert_eq!(exec(&args("/bin/hello")), ok("\n"));
        assert_eq!(exec(&args("toolbox hello x y")), ok("x y\n"));
        assert_eq!(exec(&args("bin/toolbox.wasm hello.wasm 1")), ok("1\n"));
        // Each argument is kept whole, spaces and empty ones included.
        let argv = ["hello", "a b", "", "c"].map(String::from);
        assert_eq!(exec(&argv), ok("a b  c\n"));
    }

    #[test]
    fn hello_reports_a_failed_stdout() {
        let mut err = Vec::new();
        assert_eq!(hello(&args("a"), &mut Closed, &mut err), 1);
        assert_eq!(err, b"hello: closed\n");
    }

    #[test]
    fn unbuilt_applets_say_so_and_exit_1() {
        for name in APPLETS.iter().filter(|n| !["hello", "spin"].contains(n)) {
            let msg = format!("{name}: not built yet\n");
            assert_eq!(exec(&[(*name).to_owned()]), (1, String::new(), msg));
            assert_eq!(exec(&args(&format!("/bin/{name} x"))).0, 1);
        }
    }

    #[test]
    fn an_unknown_applet_is_not_found() {
        let list = "applets: hello rev wc spin nap fstest keys bench selftest\n";
        let (status, out, err) = exec(&args("/bin/nope a"));
        assert_eq!((status, out.as_str()), (127, ""));
        assert_eq!(err, format!("toolbox: no applet \"nope\"; {list}"));
        assert_eq!(exec(&args("toolbox")).0, 127);
        assert_eq!(exec(&[]).0, 127);
    }
}
