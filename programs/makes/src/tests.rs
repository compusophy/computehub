use super::*;

/// The reference program for `id`: one that does what its task asks.
fn reference(id: &str) -> String {
    let path = [env!("CARGO_MANIFEST_DIR"), "/refs/", id, ".app"].concat();
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn every_reference_passes_its_checker() {
    let mut bad = Vec::new();
    for t in &TASKS {
        let t0 = std::time::Instant::now();
        let (pass, stage, why) = judge(t, &reference(t.id));
        eprintln!("{:<12} {stage} {:?} {why}", t.id, t0.elapsed());
        if !pass {
            bad.push(format!("{}: {stage} {why}", t.id));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

/// Other designs the descriptions allow pass too (`refs/alt/<task>-<how>.app`): boards as grid
/// widgets, cards that turn back on a timer, a die that tumbles, a count drawn on a canvas, a
/// pick written apart from its heading, a Start the description never asked for, a score written
/// inside a board's square.
#[test]
fn every_alternative_design_passes() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/refs/alt");
    let mut bad = Vec::new();
    for path in std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()) {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let t = find(name.split('-').next().unwrap()).unwrap();
        let (pass, stage, why) = judge(t, &std::fs::read_to_string(&path).unwrap());
        if !pass {
            bad.push(format!("{name}: {stage} {why}"));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

/// Programs one mistake away from a reference, and the stage that must catch it: each checker
/// tells a working app from a broken one.
const MUTANTS: &[(&str, &str, &str, &str)] = &[
    ("counter", "n += 1", "n += 2", "check"),
    ("counter", "// icon: ring 12 12 9 line 8 12 16 12 line 12 8 12 16\n", "", "icon"),
    ("greeter", "\"Hello, \" + name", "\"Hi \" + name", "check"),
    ("temperature", "+ 32 +", "+ 30 +", "check"),
    ("tip", "* rate / 100", "* rate / 10", "check"),
    ("dice", "pip(face == 6, 30, 50);", "", "check"),
    ("dice", "1 + random(6)", "1 + random(2)", "check"),
    ("traffic", "light(2, t < 30, 2);", "light(2, t < 20, 2);", "check"),
    ("traffic", "if speed == 0 { speed = 100; } else { speed = 0; }", "speed = 100;", "check"),
    ("stopwatch", "button \"Reset\" { tenths = 0; }", "button \"Reset\" { tenths = 1; }", "check"),
    ("stopwatch", "every speed { tenths += 1; }", "every speed { tenths += 2; }", "check"),
    ("rps", "% 3 == 1 { return \"You win\"; }", "% 3 == 2 { return \"You win\"; }", "check"),
    ("rps", "them = random(3);", "them = 0;", "check"),
    ("guess", "said = \"Too high\";", "said = \"Too low\";", "check"),
    ("poll", "votes[i] * 80 / most()", "min(votes[i], 1) * 80", "check"),
    ("tictactoe", "turn = 3 - turn;\n    winner = won();", "turn = 3 - turn;", "check"),
    ("tictactoe", "if winner == 3 { return \"Draw\"; }", "", "check"),
    ("connect4", "let r = 5 - k;", "let r = k;", "check"),
    ("connect4", "four(c, r, 0, 1)", "0", "check"),
    ("pomodoro", "button \"Pause\" { speed = 0; }", "button \"Pause\" { speed = 1000; }", "check"),
    ("pomodoro", "if rest { left = 300; }", "if rest { left = 1500; }", "check"),
    ("clock", "hand((t % 60) * 6, 50, 1, 1);", "hand((t % 60) * 3, 50, 1, 1);", "check"),
    ("clock", "every 1000 { t += 1; }", "every 1000 { t += 2; }", "check"),
    ("calculator", "if op == \"-\" { return a - b; }", "if op == \"-\" { return b - a; }", "check"),
    ("calculator", "(op == \"/\" && b == 0) || ", "", "check"),
    ("paint", "(px - 2) / 11", "(px - 2) / 11 + 1", "check"),
    ("paint", "for i in 0..len(art) { art[i] = -1; }", "pen = 1;", "check"),
    ("drawpad", "art[py * 64 + 63 - px] = 9;", "art[py * 64 + px] = 9;", "check"),
    ("whack", "score += 1;", "score += 2;", "check"),
    ("whack", "if ticks >= 300 {", "if ticks >= 3000 {", "check"),
    ("bounce", "dx = dx + dx / abs(dx);\n  dy = dy + dy / abs(dy);", "", "check"),
    ("bounce", "if by > 115 { by = 115; dy = 0 - abs(dy); }", "", "check"),
    ("life", "if n == 3 || (n == 2", "if n == 3 || (n == 1", "check"),
    ("life", "cells[i] = 1 - cells[i];", "cells[i] = 1;", "check"),
    ("minesweeper", "if n < 10 {", "if n < 9 {", "check"),
    ("minesweeper", "if i != first && mines[i] == 0", "if mines[i] == 0", "check"),
    (
        "minesweeper",
        "if open[i] == 0 { open[i] = 2; }",
        "if open[i] == 0 { open[i] = 1; }",
        "check",
    ),
    (
        "minesweeper",
        "if mines[i] == 0 && open[i] != 1",
        "if i < 80 && mines[i] == 0 && open[i] != 1",
        "check",
    ),
    ("minesweeper", "if won() { over = 2; }", "", "check"),
    ("memory", "up[first] = 0;\n    up[second] = 0;", "", "check"),
    ("memory", "moves += 1;", "moves += 2;", "check"),
    ("2048", "if random(10) == 0 { b[i] = 4; } else { b[i] = 2; }", "b[i] = 8;", "check"),
    ("2048", "push(out, vals[k] * 2);", "push(out, vals[k]); push(out, vals[k]);", "check"),
    ("tetris", "playing = false;\n    over = true;", "playing = false;", "check"),
    ("tetris", "prot = (prot + 1) % 4;\n    paint();", "paint();", "check"),
    ("tetris", "px -= 1;", "px += 1;", "check"),
];

#[test]
fn every_checker_fails_a_broken_app() {
    let mut missed = Vec::new();
    for &(id, from, to, stage) in MUTANTS {
        let src = reference(id);
        assert!(src.contains(from), "{id}: no {from:?} to change");
        let t = find(id).unwrap();
        let (pass, got, why) = judge(t, &src.replacen(from, to, 1));
        if pass || got != stage {
            missed.push(format!("{id}: {from:?} -> {to:?}: {got} {why}"));
        }
    }
    assert!(missed.is_empty(), "{missed:#?}");
}

/// A checker is fair to every game its task allows, not only the reference's: minesweeper's
/// references with their mines drawn elsewhere (still at random, the first tap still safe), so
/// that a sweep's flood fill wins before its last tap, pass too.
#[test]
fn minesweeper_passes_on_other_layouts() {
    let t = find("minesweeper").unwrap();
    let alt = concat!(env!("CARGO_MANIFEST_DIR"), "/refs/alt/minesweeper-grid.app");
    let mut bad = Vec::new();
    for (name, src) in
        [("ref", reference("minesweeper")), ("grid", std::fs::read_to_string(alt).unwrap())]
    {
        assert!(src.contains("random(81)"), "{name}: no draw to shift");
        for k in [1, 8, 19, 30, 47, 62] {
            let shifted = src.replace("random(81)", &format!("(random(81) + {k}) % 81"));
            let (pass, stage, why) = judge(t, &shifted);
            if !pass {
                bad.push(format!("{name} +{k}: {stage} {why}"));
            }
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
fn the_suite_listing_is_kept() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../evals/suites/studio.jsonl");
    let kept = std::fs::read_to_string(path).unwrap_or_default().replace('\r', "");
    assert_eq!(kept, listing(), "run: cargo run -p eval -- list > evals/suites/studio.jsonl");
    let ids: Vec<&str> = TASKS.iter().map(|t| t.id).collect();
    assert!(ids.len() >= 20 && ids.iter().all(|id| ids.iter().filter(|j| *j == id).count() == 1));
}
