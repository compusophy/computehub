//! The toolbox: compusophyOS's test programs in one std binary for wasm32-wasip1, so they share one
//! copy of std. It runs the applet named by the file name of argv\[0\] (less a `.wasm`), or by
//! argv\[1\] when argv\[0\] names the toolbox itself; the /bin marker of every applet points here.
//! The exit status is the applet's: 127 for no applet, and 1 for one not built yet (all but
//! `hello`, `rev`, `wc`, `spin` and `fstest`), which says so.

#![forbid(unsafe_code)]

mod fstest;

use std::io::{self, Read, Write};
use std::process::ExitCode;

/// Every applet, in the order `toolbox` lists them.
const APPLETS: [&str; 9] =
    ["hello", "rev", "wc", "spin", "nap", "fstest", "keys", "bench", "selftest"];

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    let (name, args) = applet(&argv);
    let (mut out, mut err) = (io::stdout().lock(), io::stderr().lock());
    ExitCode::from(run(name, args, &mut io::stdin().lock(), &mut out, &mut err))
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

/// Runs applet `name` on `args` (its own name first) and its input; its exit status.
fn run(
    name: &str,
    args: &[String],
    inp: &mut dyn Read,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let words = args.get(1..).unwrap_or(&[]);
    match name {
        "hello" => hello(words, out, err),
        "rev" | "wc" => filter(name, words, inp, out, err),
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

/// `rev`: each line of the input, its characters reversed. `wc [-l|-w|-c]`: the input's lines,
/// words and bytes, or the one count asked for. 1 if the input cannot be read or the output
/// written, 2 for an option `wc` does not know.
fn filter(
    name: &str,
    words: &[String],
    inp: &mut dyn Read,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let mut data = Vec::new();
    if let Err(e) = inp.read_to_end(&mut data) {
        let _ = writeln!(err, "{name}: {e}");
        return 1;
    }
    let text = String::from_utf8_lossy(&data);
    let said = match (name, words.first().map(String::as_str)) {
        ("rev", _) => text.lines().map(|l| l.chars().rev().collect::<String>() + "\n").collect(),
        (_, None) => {
            format!("{} {} {}\n", text.lines().count(), text.split_whitespace().count(), data.len())
        }
        (_, Some("-l")) => format!("{}\n", text.lines().count()),
        (_, Some("-w")) => format!("{}\n", text.split_whitespace().count()),
        (_, Some("-c")) => format!("{}\n", data.len()),
        (_, Some(other)) => {
            let _ = writeln!(err, "wc: unknown option {other}; use -l, -w or -c");
            return 2;
        }
    };
    match out.write_all(said.as_bytes()).and_then(|()| out.flush()) {
        Ok(()) => 0,
        Err(e) => {
            let _ = writeln!(err, "{name}: {e}");
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

    /// The same on `input`.
    fn piped(argv: &[String], input: &str) -> (u8, String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let (name, rest) = applet(argv);
        let status = run(name, rest, &mut input.as_bytes(), &mut out, &mut err);
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
    fn rev_and_wc_read_their_input() {
        let ok = |s: &str| (0, s.to_owned(), String::new());
        assert_eq!(piped(&args("rev"), "abc\nhé\n\n"), ok("cba\néh\n\n"));
        assert_eq!(piped(&args("wc"), "one two\nthree\n"), ok("2 3 14\n"));
        let counts = ["-l", "-w", "-c"].map(|f| piped(&args(&format!("wc {f}")), "a b\n").1);
        assert_eq!(counts, ["1\n", "2\n", "4\n"]);
        assert_eq!(piped(&args("wc -x"), "").0, 2);
        assert_eq!((exec(&args("rev")), exec(&args("wc"))), (ok(""), ok("0 0 0\n")));
    }
}
