//! `eval mutants`: fix data made at no cost, correct by construction (DESIGN.md, "The team"). A
//! helper's job is to repair the cloud model's drafts that do not run clean; GLM's real failures
//! on the held-out tasks are, most of them, a few kinds: the `clear` trap (E0311), an icon line the
//! desktop cannot draw, a name that does not exist, a syntax slip. Here each train task's verified
//! reference program is broken one such way at a time, kept only when the make loop's own test
//! (`coder::ai::fault`) says it does not run clean, and paired with the request a helper gets for
//! it (`coder::prompt::system` and `fix`, with the fault's account, as `team::Repair` asks its
//! first turn) and the answer that restores the program: one SEARCH/REPLACE block, checked to
//! apply (`coder::edits::apply`) and give the reference back. Held-out families (by root, as the
//! suite splits them) are never used.
//!
//!   eval mutants --suite S --refs R.jsonl[,R2.jsonl] --held H.txt --out fix-mutants.jsonl
//!                [--per 1] [--max N] [--kinds fn,decl,size]

use coder::edits::{self, Edit};
use coder::json::{Json, quote};
use coder::{ai, prompt};

use crate::{arg, read};

/// The kinds of break, in the order a task tries them (rotated by the task, so each kind gets its
/// share): its name, and the program broken that way at its `n`th chance, if it can be.
const KINDS: [&str; 9] =
    ["clear", "icon", "name", "semicolon", "brace", "paren", "fn", "decl", "size"];

/// A family's root: its name before the first '-', `level` joined with `platformer` (iq::root).
fn root(family: &str) -> &str {
    let r = family.split('-').next().unwrap_or(family);
    if r == "level" { "platformer" } else { r }
}

/// A small, stable hash of `s` (FNV-1a): which line a kind breaks, which kind a task tries first.
fn hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

/// Whether `w` is a word character.
fn word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The names a program declares (`fn name(`, `state name`, `let name`), longer than two letters.
fn names(lines: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in lines {
        let t = l.trim_start();
        for kw in ["fn ", "state ", "let "] {
            if let Some(rest) = t.strip_prefix(kw) {
                let n: String = rest.chars().take_while(|&c| word(c)).collect();
                if n.len() > 2 && !out.contains(&n) {
                    out.push(n);
                }
            }
        }
    }
    out
}

/// `lines` broken the way `kind` says, at the `n`th place it can be: lines `a..b` put as `new`
/// (none: removed), or `None`. The last three are GLM's own misses on held-out tasks (2026-10-09):
/// a function it calls but never wrote, a local it uses but never declared, a list too short for
/// what it indexes.
fn broken(kind: &str, lines: &[&str], n: u64) -> Option<(usize, usize, Vec<String>)> {
    let pick = |cands: Vec<usize>| -> Option<usize> {
        (!cands.is_empty()).then(|| cands[(n % cands.len() as u64) as usize])
    };
    match kind {
        // A top-level function gone, its calls left.
        "fn" => {
            let s = pick((0..lines.len()).filter(|&i| lines[i].starts_with("fn ")).collect())?;
            let (mut depth, mut seen) = (0i32, false);
            for (e, l) in lines.iter().enumerate().skip(s) {
                depth += l.matches('{').count() as i32 - l.matches('}').count() as i32;
                seen |= l.contains('{');
                if seen && depth <= 0 {
                    return Some((s, e + 1, Vec::new()));
                }
            }
            None
        }
        // A one-line local declaration gone, its uses left.
        "decl" => {
            let i = pick(
                (0..lines.len())
                    .filter(|&i| {
                        lines[i].trim_start().starts_with("let ")
                            && lines[i].trim_end().ends_with(';')
                    })
                    .collect(),
            )?;
            Some((i, i + 1, Vec::new()))
        }
        // A list made a tenth shorter than what it is indexed for.
        "size" => {
            let i = pick(
                (0..lines.len())
                    .filter(|&i| lines[i].contains("; ") && lines[i].contains(']'))
                    .collect(),
            )?;
            let at = lines[i].find("; ")? + 2;
            let digits: String = lines[i][at..].chars().take_while(char::is_ascii_digit).collect();
            let size: u32 = digits.parse().ok().filter(|&v| v >= 4)?;
            let shorter = (size - (size / 10).max(1)).to_string();
            let new = [&lines[i][..at], &shorter, &lines[i][at + digits.len()..]].concat();
            Some((i, i + 1, vec![new]))
        }
        _ => one(kind, lines, n).map(|(i, line)| (i, i + 1, vec![line])),
    }
}

/// Line `i` of `lines` broken the way `kind` says, at the `n`th place it can be, or `None`.
fn one(kind: &str, lines: &[&str], n: u64) -> Option<(usize, String)> {
    let code = |l: &str| !l.trim_start().starts_with("//");
    let pick = |cands: Vec<usize>| -> Option<usize> {
        (!cands.is_empty()).then(|| cands[(n % cands.len() as u64) as usize])
    };
    match kind {
        // `name = [0; 144];` made `clear(name);`: the trap a never-grown list sets.
        "clear" => {
            let c: Vec<usize> = (0..lines.len())
                .filter(|&i| {
                    let t = lines[i].trim();
                    t.ends_with("];")
                        && t.contains(" = [")
                        && t.split(" = [").next().is_some_and(|v| v.chars().all(word))
                })
                .collect();
            let i = pick(c)?;
            let indent: String = lines[i].chars().take_while(|c| c.is_whitespace()).collect();
            let v = lines[i].trim().split(" = [").next()?.to_string();
            Some((i, [indent.as_str(), "clear(", &v, ");"].concat()))
        }
        // The icon line's first number pushed off the drawing (24 x 24).
        "icon" => {
            let i = lines.iter().position(|l| l.trim_start().starts_with("// icon:"))?;
            let words: Vec<&str> = lines[i].split(' ').collect();
            let at = words
                .iter()
                .position(|w| !w.is_empty() && w.bytes().all(|b| b.is_ascii_digit()))?;
            let mut w: Vec<String> = words.iter().map(|s| s.to_string()).collect();
            w[at] = "31".into();
            Some((i, w.join(" ")))
        }
        // A declared name used on another line, misspelled there.
        "name" => {
            let ns = names(lines);
            let name = ns.get((n % ns.len().max(1) as u64) as usize)?;
            let uses: Vec<usize> = (0..lines.len())
                .filter(|&i| {
                    let t = lines[i].trim_start();
                    code(lines[i])
                        && !t.starts_with("fn ")
                        && !t.starts_with("state ")
                        && !t.starts_with("let ")
                        && lines[i].match_indices(name.as_str()).any(|(at, _)| {
                            let before = lines[i][..at].chars().last().is_none_or(|c| !word(c));
                            let after =
                                lines[i][at + name.len()..].chars().next().is_none_or(|c| !word(c));
                            before && after
                        })
                })
                .collect();
            let i = pick(uses)?;
            let mut wrong: Vec<char> = name.chars().collect();
            let k = wrong.len() / 2;
            wrong.swap(k - 1, k);
            let wrong: String = wrong.into_iter().collect();
            (wrong != *name).then(|| (i, lines[i].replacen(name.as_str(), &wrong, 1)))
        }
        // A statement's semicolon dropped.
        "semicolon" => {
            let c = (0..lines.len())
                .filter(|&i| code(lines[i]) && lines[i].trim_end().ends_with(';'))
                .collect();
            let i = pick(c)?;
            let t = lines[i].trim_end();
            Some((i, t[..t.len() - 1].to_string()))
        }
        // A closing brace alone on its line, gone.
        "brace" => {
            let c = (0..lines.len()).filter(|&i| lines[i].trim() == "}").collect();
            let i = pick(c)?;
            Some((i, String::new()))
        }
        // A call's closing parenthesis dropped.
        "paren" => {
            let c =
                (0..lines.len()).filter(|&i| code(lines[i]) && lines[i].contains(");")).collect();
            let i = pick(c)?;
            Some((i, lines[i].replacen(");", ";", 1)))
        }
        _ => None,
    }
}

/// The block that restores `orig` from `mutant` (`orig`'s lines `a..b` put as `m` lines there):
/// those lines and as much context either side as makes it match one place, checked to apply and
/// give `orig` back.
fn restore(
    orig: &[&str],
    mutant: &[String],
    at: (usize, usize, usize),
    want: &str,
) -> Option<Edit> {
    let (a, b, m) = at;
    let joined = mutant.join("\n");
    for k in 0..4 {
        let lo = a.saturating_sub(k);
        let (hi_m, hi_o) = ((a + m + k).min(mutant.len()), (b + k).min(orig.len()));
        let search: Vec<String> = mutant[lo..hi_m].to_vec();
        let replace: Vec<String> = orig[lo..hi_o].iter().map(|s| s.to_string()).collect();
        if search.iter().all(|l| l.trim().is_empty()) {
            continue;
        }
        let edit = Edit { search, replace };
        if let Ok(fixed) = edits::apply(&joined, std::slice::from_ref(&edit)) {
            if fixed.trim_end() == want.trim_end() {
                return Some(edit);
            }
        }
    }
    None
}

/// `edit` as the reply a helper gives.
fn block(edit: &Edit) -> String {
    let mut out = String::from("<<<<<<< SEARCH\n");
    for l in &edit.search {
        out += l;
        out.push('\n');
    }
    out += "=======\n";
    for l in &edit.replace {
        out += l;
        out.push('\n');
    }
    out + ">>>>>>> REPLACE\n"
}

pub fn mutants(args: &[String]) {
    let need = |n: &str| arg(args, n).unwrap_or_else(|| panic!("eval mutants: {n} is needed"));
    let (suite, refs, held, out) = (need("--suite"), need("--refs"), need("--held"), need("--out"));
    let per: usize = arg(args, "--per").and_then(|v| v.parse().ok()).unwrap_or(1);
    let max: usize = arg(args, "--max").and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
    // The kinds to make: all unless named (comma-separated).
    let named = arg(args, "--kinds").unwrap_or_default();
    let kinds: Vec<&str> = KINDS
        .iter()
        .copied()
        .filter(|k| named.is_empty() || named.split(',').any(|n| n == *k))
        .collect();
    let text = |j: &Json, k: &str| j.get(k).and_then(Json::text).unwrap_or("").to_string();
    let held: Vec<String> = read(&held)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| root(l).to_string())
        .collect();
    // Train tasks: (id, family, ask), held-out roots left out.
    let tasks: Vec<(String, String, String)> = read(&suite)
        .lines()
        .filter_map(Json::parse)
        .map(|j| (text(&j, "id"), text(&j, "family"), text(&j, "ask")))
        .filter(|t| !t.1.is_empty() && !held.iter().any(|h| h == root(&t.1)))
        .collect();
    let system = prompt::system();
    let seeds = coder::Knobs::default().seeds;
    let (mut lines_out, mut counts) = (String::new(), [0usize; KINDS.len()]);
    let (mut seen, mut no_ref, mut dirty) = (0, 0, 0);
    for path in refs.split(',') {
        for j in read(path).lines().filter_map(Json::parse) {
            let id = text(&j, "task");
            let Some((_, family, ask)) = tasks.iter().find(|t| t.0 == id) else { continue };
            seen += 1;
            let reply = text(&j, "reply");
            let Some((src, _)) = edits::program(&reply) else {
                no_ref += 1;
                continue;
            };
            let src = src.to_string();
            // Only a reference that runs clean: its break is then the only problem.
            if ai::fault(&src, "", seeds).is_some() {
                dirty += 1;
                continue;
            }
            let orig: Vec<&str> = src.lines().collect();
            let h = hash(&id);
            let mut made = 0;
            for r in 0..KINDS.len() {
                if made >= per || counts.iter().sum::<usize>() >= max {
                    break;
                }
                let k = ((h as usize) + r) % KINDS.len();
                if !kinds.contains(&KINDS[k]) {
                    continue;
                }
                let Some((a, b, new)) = broken(KINDS[k], &orig, h >> 8) else { continue };
                let m = new.len();
                let mut mutant: Vec<String> = orig[..a].iter().map(|s| s.to_string()).collect();
                mutant.extend(new);
                mutant.extend(orig[b..].iter().map(|s| s.to_string()));
                let mutated = mutant.join("\n") + "\n";
                let Some(f) = ai::fault(&mutated, "", seeds) else { continue };
                let Some(edit) = restore(&orig, &mutant, (a, b, m), &src) else { continue };
                let user = prompt::fix(ask, &mutated, &f.account);
                let code = f.diag.code.unwrap_or(1);
                lines_out += &format!(
                    "{{\"messages\":[{{\"role\":\"system\",\"content\":{}}},{{\"role\":\"user\",\"content\":{}}},{{\"role\":\"assistant\",\"content\":{}}}],\"task\":{},\"family\":{},\"kind\":\"fix\",\"by\":{{\"mutation\":{},\"code\":{code},\"compiles\":{}}}}}\n",
                    quote(&system),
                    quote(&user),
                    quote(&block(&edit)),
                    quote(&id),
                    quote(family),
                    quote(KINDS[k]),
                    f.compiles
                );
                counts[k] += 1;
                made += 1;
            }
        }
    }
    std::fs::write(&out, lines_out).unwrap_or_else(|e| panic!("{out}: {e}"));
    let by: Vec<String> = KINDS.iter().zip(counts).map(|(k, n)| format!("{k} {n}")).collect();
    eprintln!(
        "mutants: {} train tasks with a reference ({no_ref} without a program, {dirty} not clean): {} records ({})",
        seen,
        counts.iter().sum::<usize>(),
        by.join(", ")
    );
}
