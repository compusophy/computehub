//! The toolbox: compusophyOS's test programs in one std binary for wasm32-wasip1, so they share one
//! copy of std. It runs the applet named by the file name of argv\[0\] (less a `.wasm`), or by
//! argv\[1\] when argv\[0\] names the toolbox itself; the /bin marker of every applet points here.
//! The exit status is the applet's: 127 for no applet, and 1 for one not built yet (all but
//! `hello`, `rev`, `wc`, `spin` and `fstest`), which says so.

#![forbid(unsafe_code)]

mod fstest;
mod text;

use std::io::{self, Read, Write};
use std::process::ExitCode;

use text::Files;

/// Every applet, in the order `toolbox` lists them.
const APPLETS: [&str; 9] =
    ["hello", "rev", "wc", "spin", "nap", "fstest", "keys", "bench", "selftest"];

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    let (name, args) = applet(&argv);
    let (mut out, mut err) = (io::stdout().lock(), io::stderr().lock());
    let files = &mut |path: &str| std::fs::read(path);
    ExitCode::from(run(name, args, &mut io::stdin().lock(), files, &mut out, &mut err))
}

/// The applet `argv` names and its arguments, its own name first.
fn applet(argv: &[String]) -> (&str, &[String]) {
    match argv {
        [first, second, ..] if base(first) == "toolbox" => (base(second), &argv[1..]),
        [first, ..] => (base(first), argv),
        [] => ("", argv),
    }
}

/// A path's file name, less a `.wasm`.
fn base(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".wasm").unwrap_or(name)
}

/// Runs applet `name` on `args` (its own name first), its input and the files; its exit status.
fn run(
    name: &str,
    args: &[String],
    inp: &mut dyn Read,
    files: Files<'_>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let words = args.get(1..).unwrap_or(&[]);
    match name {
        "hello" => hello(words, out, err),
        "rev" | "wc" => text::filter(name, words, inp, files, out, err),
        "spin" => spin(),
        "fstest" => fstest::fstest(args, out),
        _ if APPLETS.contains(&name) => {
            // A failed write to stderr leaves nothing to report it on.
            let _ = writeln!(err, "{name}: not built yet");
            1
        }
        _ => {
            let _ = writeln!(err, "toolbox: no applet {name:?}; applets: {}", APPLETS.join(" "));
            127 // the shell's "not found"
        }
    }
}

/// `hello [word...]`: the words joined by spaces and a newline, in one write; 1 if it fails.
fn hello(words: &[String], out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let line = words.join(" ") + "\n";
    match out.write_all(line.as_bytes()).and_then(|()| out.flush()) {
        Ok(()) => 0,
        Err(e) => {
            let _ = writeln!(err, "hello: {e}");
            1
        }
    }
}

/// `spin`: loops forever without a syscall, to test that the kernel kills a
/// guest mid-loop (Ctrl+C gives 130, closing the window 137).
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

    /// Runs `argv` as `main` would, on no input: (status, stdout, stderr).
    fn exec(argv: &[String]) -> (u8, String, String) {
        piped(argv, "")
    }

    /// The files the applets read in tests: a directory (no text), and the rest missing.
    const FILES: [(&str, Option<&str>); 3] =
        [("a", Some("one two\nthree\n")), ("/b/c", Some("é x")), ("d", None)];

    /// The same on `input`.
    fn piped(argv: &[String], input: &str) -> (u8, String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let (name, rest) = applet(argv);
        let files = &mut |path: &str| match FILES.iter().find(|f| f.0 == path) {
            Some((_, Some(text))) => Ok(text.as_bytes().to_vec()),
            Some((_, None)) => Err(io::Error::from(io::ErrorKind::IsADirectory)),
            None => Err(io::Error::from(io::ErrorKind::NotFound)),
        };
        let status = run(name, rest, &mut input.as_bytes(), files, &mut out, &mut err);
        let text = |b: Vec<u8>| String::from_utf8(b).unwrap();
        (status, text(out), text(err))
    }

    #[test]
    fn applets_resolve_from_argv_and_exit_with_their_status() {
        let name = |s| applet(&args(s)).0.to_owned();
        let argvs = ["hello a b", "/bin/rev.wasm", "toolbox wc -l", "bin/toolbox.wasm spin"];
        assert_eq!(argvs.map(name), ["hello", "rev", "wc", "spin"]);
        assert_eq!(name("toolbox"), "toolbox");
        assert_eq!(applet(&args("toolbox wc -l")).1, args("wc -l"));
        assert_eq!(applet(&[]).0, "");
        // hello prints its arguments joined by spaces.
        let ok = |s: &str| (0, s.to_owned(), String::new());
        let runs = ["hello a b", "/bin/hello", "toolbox hello x y"].map(|s| exec(&args(s)));
        assert_eq!(runs, ["a b\n", "\n", "x y\n"].map(ok));
        assert_eq!(exec(&args("bin/toolbox.wasm hello.wasm 1")), ok("1\n"));
        // Each argument is kept whole, spaces and empty ones included.
        let argv = ["hello", "a b", "", "c"].map(String::from);
        assert_eq!(exec(&argv), ok("a b  c\n"));
        // A stdout that takes nothing is reported.
        let (mut full, mut err): (&mut [u8], _) = (&mut [], Vec::new());
        assert_eq!(hello(&args("a"), &mut full, &mut err), 1);
        assert_eq!(err, b"hello: failed to write whole buffer\n");
        // Unbuilt applets exit 1, and unknown ones 127.
        for name in APPLETS.iter().filter(|n| !["hello", "rev", "wc", "spin", "fstest"].contains(n))
        {
            let msg = format!("{name}: not built yet\n");
            assert_eq!(exec(&[(*name).to_owned()]), (1, String::new(), msg));
            assert_eq!(exec(&args(&format!("/bin/{name} x"))).0, 1);
        }
        let list = "applets: hello rev wc spin nap fstest keys bench selftest\n";
        let (status, out, err) = exec(&args("/bin/nope a"));
        assert_eq!((status, out.as_str()), (127, ""));
        assert_eq!(err, format!("toolbox: no applet \"nope\"; {list}"));
        assert_eq!((exec(&args("toolbox")).0, exec(&[]).0), (127, 127));
    }

    #[test]
    fn rev_and_wc_read_their_input_when_they_name_no_file() {
        let ok = |s: &str| (0, s.to_owned(), String::new());
        assert_eq!(piped(&args("rev"), "abc\nhé\n\n"), ok("cba\néh\n\n"));
        assert_eq!(piped(&args("wc"), "one two\nthree\n"), ok("2 3 14\n"));
        let counts = ["-l", "-w", "-c", "-m"].map(|f| piped(&args(&format!("wc {f}")), "a é\n").1);
        assert_eq!(counts, ["1\n", "2\n", "5\n", "4\n"]);
        // Lines are line breaks: a last line without one is not counted.
        assert_eq!(piped(&args("wc -l"), "a\nb").1, "1\n");
        assert_eq!((exec(&args("rev")), exec(&args("wc"))), (ok(""), ok("0 0 0\n")));
    }

    #[test]
    fn rev_and_wc_read_the_files_they_name() {
        let ok = |s: &str| (0, s.to_owned(), String::new());
        let wc = |argv: &str| piped(&args(argv), "x\n");
        assert_eq!(wc("wc a"), ok("2 3 14 a\n"));
        assert_eq!(wc("wc a /b/c"), ok("2 3 14 a\n0 2 4 /b/c\n2 5 18 total\n"));
        // Flags pick counts, always printed lines, words, characters, bytes.
        assert_eq!(wc("wc -l a"), ok("2 a\n"));
        assert_eq!(wc("wc -cm -w /b/c"), ok("2 3 4 /b/c\n"));
        // `-` is the input, `--` ends the flags.
        assert_eq!(wc("wc -w - a"), ok("1 -\n3 a\n4 total\n"));
        let missing = "wc: -l: no such file or directory\n";
        assert_eq!(wc("wc -- -l"), (1, String::new(), missing.into()));
        // A file that cannot be read is said, the rest still are, and the status is 1.
        let (status, out, err) = wc("wc nope a d");
        assert_eq!((status, out.as_str()), (1, "2 3 14 a\n2 3 14 total\n"));
        assert_eq!(err, "wc: nope: no such file or directory\nwc: d: is a directory\n");
        let empty = "wc: : no such file or directory\n";
        assert_eq!(wc("wc "), (1, String::new(), empty.into()));
        // An unknown option is a misuse: status 2, nothing read.
        assert_eq!(wc("wc -lx a"), (2, String::new(), "wc: unknown option -x\n".into()));
        assert_eq!(piped(&args("rev a /b/c"), ""), ok("owt eno\neerht\nx é\n"));
        assert_eq!(piped(&args("rev nope"), "").2, "rev: nope: no such file or directory\n");
        assert_eq!(
            piped(&args("rev -l a"), ""),
            (2, String::new(), "rev: unknown option -l\n".into())
        );
    }
}
