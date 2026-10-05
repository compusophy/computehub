//! The verifier's hash, which every task's provenance carries (`by.verifier`): FNV-1a 64 of the
//! files of this crate and of each workspace crate it stands on (the probe and its suite, the
//! coder's smoke test, applang, its syntax and runtime, and what they stand on), as `evals`'
//! build script hashes its harness. The crates are read from the manifests (this crate's
//! dependencies, theirs, and so on, dev-dependencies aside, each found by its path in the
//! workspace's table); their files are taken by path, tests, licenses and binaries (`main.rs`)
//! aside, line ends aside. A change to the checker language, the grade, the probe or the smoke
//! test is a new verifier, and the tasks it did not verify say so.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

/// FNV-1a 64 of `bytes` onto `h`, carriage returns aside.
fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes.iter().filter(|&&b| b != b'\r') {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// The workspace crates the manifest `toml` depends on, by name (dev-dependencies aside).
fn deps(toml: &str) -> Vec<String> {
    let (mut section, mut out) = ("", Vec::new());
    for line in toml.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line;
        } else if section.ends_with("dependencies]") && !section.contains("dev-") {
            let name = line.split(['.', ' ', '=']).next().unwrap_or_default();
            if name.starts_with("compusophy-") {
                out.push(name.to_string());
            }
        }
    }
    out
}

/// The files under `dir`, tests, licenses and binaries aside.
fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    for e in read.flatten() {
        let (path, name) = (e.path(), e.file_name());
        let skip = ["tests", "tests.rs", "LICENSE", "target", "main.rs"];
        if skip.iter().any(|s| name == *s) {
            continue;
        }
        if path.is_dir() { files(&path, out) } else { out.push(path) }
    }
}

fn main() {
    let here = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets it");
    let root = Path::new(&here).join("..").join("..");
    let read = |p: &Path| std::fs::read_to_string(p).unwrap_or_default();
    let table = read(&root.join("Cargo.toml"));
    // A workspace crate's directory: `name = { path = "dir", ... }` in the table.
    let dir_of = |name: &str| {
        let line = table.lines().find(|l| l.split([' ', '=']).next() == Some(name))?;
        let at = line.find("path = \"")? + 8;
        Some(line[at..].split('"').next()?.to_string())
    };
    let (mut todo, mut crates) = (vec!["programs/iq".to_string()], Vec::new());
    while let Some(dir) = todo.pop() {
        if !crates.contains(&dir) {
            for d in deps(&read(&root.join(&dir).join("Cargo.toml"))) {
                todo.push(dir_of(&d).unwrap_or_else(|| panic!("{d} is not in the workspace")));
            }
            crates.push(dir);
        }
    }
    crates.sort();
    let mut h = 0xcbf2_9ce4_8422_2325;
    for dir in &crates {
        let base = root.join(dir);
        println!("cargo:rerun-if-changed={}", base.display());
        let mut all = Vec::new();
        files(&base, &mut all);
        // Each file by its path from the root, `/` between its parts, so every system agrees.
        let mut named: Vec<(String, PathBuf)> = all
            .into_iter()
            .map(|p| {
                let rel = p.strip_prefix(&base).unwrap_or(&p);
                let parts: Vec<String> = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect();
                ([dir.as_str(), "/", &parts.join("/")].concat(), p)
            })
            .collect();
        named.sort();
        for (name, path) in named {
            h = fnv(fnv(h, name.as_bytes()), &[0]);
            h = fnv(h, &std::fs::read(&path).unwrap_or_default());
        }
    }
    println!("cargo:rerun-if-changed={}", root.join("Cargo.toml").display());
    println!("cargo:rustc-env=IQ_VERIFIER={h:016x}");
}
