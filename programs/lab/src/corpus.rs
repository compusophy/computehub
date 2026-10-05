//! The corpus: each program [`collect::find`] finds that compiles and shows a widget, once (by
//! the FNV-1a hash of its text: its content address), with every place it was found; each
//! one's [`augment`]ed variants that compile too; whether each runs clean as Studio checks a
//! made app; and which are held out, by [`shape`]. All derived from the repo:
//! [`Corpus::manifest`] lists it, and [`Corpus::id`] names it by what it holds.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use tiny::{Rng, fnv};

use crate::{augment, collect};

/// The variants tried for each program found.
pub const VARIANTS: u64 = 7;

/// One program of the corpus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    /// The FNV-1a 64 hash of `text`.
    pub hash: u64,
    /// Its text, [`normal`].
    pub text: String,
    /// Its [`shape`]: what it is but for names, comments and spacing.
    pub shape: u64,
    /// Whether it runs clean: [`runs`].
    pub runs: bool,
    /// Held out: never trained on, only measured. The found programs of a tenth of the shapes
    /// (0 mod 10), with every program sharing a shape with one held and all their variants:
    /// no program on one side is one on the other renamed.
    pub held: bool,
    /// The program it was made from and how (`rename#3`, `rename+states#4`); none if found.
    pub of: Option<(u64, String)>,
    /// Where it was found (`path:line`, each place); none for a variant.
    pub from: Vec<String>,
}

/// The programs: each found one, then its variants, in hash order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Corpus {
    pub programs: Vec<Program>,
}

/// `text` as the corpus keeps it: LF line ends, no trailing blank lines or spaces, one final
/// newline.
pub fn normal(text: &str) -> String {
    let t = text.replace("\r\n", "\n");
    [t.trim_end(), "\n"].concat()
}

/// The shape of `src`: its tokens but comments, each name replaced by the order it first
/// appears in, hashed. Programs of one shape differ only in names, comments and spacing, so a
/// renamed copy of a program is no new program.
pub fn shape(src: &str) -> u64 {
    let mut names: Vec<&str> = Vec::new();
    let mut out = String::new();
    for (span, class) in applang::highlight(src) {
        let text = &src[span.start..span.end];
        match class {
            applang::Class::Comment => continue,
            applang::Class::Name => {
                let i = names.iter().position(|n| *n == text).unwrap_or_else(|| {
                    names.push(text);
                    names.len() - 1
                });
                out += &format!("#{i}");
            }
            _ => out += text,
        }
        out.push(' ');
    }
    fnv(out.as_bytes())
}

/// Whether `src` is a program: it compiles and has a widget.
pub fn compiles(src: &str) -> bool {
    applang::compile(src).is_ok_and(|p| !p.widgets().is_empty())
}

/// Whether `src` runs clean as Studio checks a made app: it compiles, the smoke test finds no
/// fault on seeds 1 to 3, and its icon line, if any, draws (`coder::ai::fault` finds nothing).
pub fn runs(src: &str) -> bool {
    compiles(src) && coder::ai::fault(src, "", 3).is_none()
}

impl Corpus {
    /// The corpus of the repo at `root`, its smoke tests on `threads` threads.
    pub fn build(root: &Path, threads: usize) -> Corpus {
        let mut found: BTreeMap<u64, Program> = BTreeMap::new();
        for f in collect::find(root) {
            let text = normal(&f.text);
            if !compiles(&text) {
                continue;
            }
            let hash = fnv(text.as_bytes());
            let p = found.entry(hash).or_insert_with(|| Program {
                hash,
                shape: shape(&text),
                text,
                runs: false,
                held: false,
                of: None,
                from: Vec::new(),
            });
            p.from.push([f.path.as_str(), ":", &f.line.to_string()].concat());
        }
        let mut seen: BTreeSet<u64> = found.keys().copied().collect();
        let mut programs = Vec::new();
        for base in found.into_values() {
            let made = variants(&base, &mut seen);
            programs.push(base);
            programs.extend(made);
        }
        hold_out(&mut programs);
        let verdicts = par_map(&programs, threads, |p| runs(&p.text));
        for (p, r) in programs.iter_mut().zip(verdicts) {
            p.runs = r;
        }
        Corpus { programs }
    }

    /// The programs trained on.
    pub fn train(&self) -> impl Iterator<Item = &Program> {
        self.programs.iter().filter(|p| !p.held)
    }

    /// The programs held out.
    pub fn held(&self) -> impl Iterator<Item = &Program> {
        self.programs.iter().filter(|p| p.held)
    }

    /// The manifest: a line per program (hash, shape, bytes, lines, runs, split, made of, how,
    /// found at), under a header saying what it is and how it was made.
    pub fn manifest(&self) -> String {
        let found = self.programs.iter().filter(|p| p.of.is_none()).count();
        let runs = self.programs.iter().filter(|p| p.runs).count();
        let shapes: BTreeSet<u64> = self.programs.iter().map(|p| p.shape).collect();
        let mut out = String::from(
            "# The applang corpus: each program in the repo that compiles and shows a widget, once\n\
             # by the FNV-1a 64 hash of its text, and the variants of each (names renamed, states\n\
             # reordered) that compile too. shape: its hash but for names, comments and spacing.\n\
             # runs: no fault on smoke seeds 1 to 3, icon drawable. held: measured, never trained\n\
             # on; a tenth of the shapes, with every program of them. The corpus's id hashes each\n\
             # line but its last column (where a program was found). Derived: `cargo run -p\n\
             # compusophy-lab --release -- corpus` writes it; never edit it by hand.\n",
        );
        out += &format!(
            "# {} programs: {} found, {} variants, {} shapes; {} run clean; {} held out.\n",
            self.programs.len(),
            found,
            self.programs.len() - found,
            shapes.len(),
            runs,
            self.held().count()
        );
        out += "hash\tshape\tbytes\tlines\truns\tsplit\tof\thow\tfrom\n";
        for p in &self.programs {
            let (of, how) = match &p.of {
                Some((h, how)) => (format!("{h:016x}"), how.as_str()),
                None => ("-".into(), "-"),
            };
            let from = if p.from.is_empty() { "-".into() } else { p.from.join(",") };
            out += &format!(
                "{:016x}\t{:016x}\t{}\t{}\t{}\t{}\t{of}\t{how}\t{from}\n",
                p.hash,
                p.shape,
                p.text.len(),
                p.text.lines().count(),
                if p.runs { "runs" } else { "faults" },
                if p.held { "held" } else { "train" },
            );
        }
        out
    }

    /// The corpus's own name: the hash of its manifest's [`content`], what it holds and not
    /// where in the repo each program sits (so moving a test's lines renames nothing).
    pub fn id(&self) -> u64 {
        fnv(content(&self.manifest()).as_bytes())
    }
}

/// What a manifest says the corpus holds: its rows (and their header) without the last
/// column, where each program was found; the comment lines go too.
pub fn content(manifest: &str) -> String {
    let rows = manifest.lines().filter(|l| !l.starts_with('#'));
    rows.flat_map(|l| [l.rsplit_once('\t').map_or(l, |(row, _)| row), "\n"]).collect()
}

/// Marks the programs held out: each found program whose shape is 0 mod 10, with its
/// variants; then, until there is none, each program of a shape a held one has, with its found
/// program and their variants. So no shape is on both sides.
pub(crate) fn hold_out(programs: &mut [Program]) {
    let family = |p: &Program| p.of.as_ref().map_or(p.hash, |o| o.0);
    let mut held: BTreeSet<u64> =
        programs.iter().filter(|p| p.of.is_none() && p.shape % 10 == 0).map(|p| p.hash).collect();
    loop {
        let shapes: BTreeSet<u64> =
            programs.iter().filter(|p| held.contains(&family(p))).map(|p| p.shape).collect();
        let more: Vec<u64> = programs
            .iter()
            .filter(|p| shapes.contains(&p.shape) && !held.contains(&family(p)))
            .map(family)
            .collect();
        if more.is_empty() {
            break;
        }
        held.extend(more);
    }
    for p in programs {
        p.held = held.contains(&family(p));
    }
}

/// `base`'s variants that compile and are new (their hashes added to `seen`): the odd ones
/// renamed, the even ones renamed and their states reordered, each drawn from the base's hash.
fn variants(base: &Program, seen: &mut BTreeSet<u64>) -> Vec<Program> {
    let mut out = Vec::new();
    for i in 1..=VARIANTS {
        let mut rng = Rng::new(base.hash ^ i);
        let mut how = Vec::new();
        let mut text = base.text.clone();
        if let Some(t) = augment::rename(&text, &mut rng) {
            (text, how) = (t, vec!["rename"]);
        }
        if i % 2 == 0 {
            if let Some(t) = augment::shuffle_states(&text, &mut rng) {
                text = t;
                how.push("states");
            }
        }
        let text = normal(&text);
        let hash = fnv(text.as_bytes());
        if how.is_empty() || seen.contains(&hash) || !compiles(&text) {
            continue;
        }
        seen.insert(hash);
        let of = Some((base.hash, format!("{}#{i}", how.join("+"))));
        let shape = shape(&text);
        out.push(Program { hash, text, shape, runs: false, held: false, of, from: Vec::new() });
    }
    out
}

/// `f` of each item, on up to `threads` threads (each taking the next item left), in the
/// items' order.
pub fn par_map<T: Sync, R: Send>(
    items: &[T],
    threads: usize,
    f: impl Fn(&T) -> R + Sync,
) -> Vec<R> {
    let (next, f) = (&AtomicUsize::new(0), &f);
    let mut done: Vec<(usize, R)> = thread::scope(|s| {
        let work = move || {
            let mut mine = Vec::new();
            loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(item) = items.get(i) else { return mine };
                mine.push((i, f(item)));
            }
        };
        let handles: Vec<_> = (0..threads.max(1)).map(|_| s.spawn(work)).collect();
        let join = |h: thread::ScopedJoinHandle<'_, Vec<(usize, R)>>| match h.join() {
            Ok(mine) => mine,
            Err(panic) => std::panic::resume_unwind(panic),
        };
        handles.into_iter().flat_map(join).collect()
    });
    done.sort_by_key(|d| d.0);
    done.into_iter().map(|d| d.1).collect()
}
