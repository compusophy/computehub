#!/usr/bin/env python
"""compose.py and paired.py: their rules on their own (no iq, no data), then, with iq.exe and the
data root, the night's figures reproduced and a small fixture night run end to end.

  python train/test_paired.py      # IQ=path/to/iq.exe, COMPUTEHUB_DATA=root override the defaults

The fixture: eight held-out tasks of the real suite, each program one of four kinds whose grade
the real iq gives (the task's reference: passes; a stub: runs clean, fails the check; E0101: does
not compile; E0215: faults when it runs); GLM's and pre's answers; two team runs with try logs in
`eval team --tries-out`'s format (b3: three tries a draft, every try run; b7: two); the recorded
samples; a dev set; a finish pick; a judge fit; decisions and predictions. TEAM_STUB=DIR also
checks item 1's stub run there (each tries-NAME.jsonl beside its team-NAME.jsonl).
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import common  # noqa: E402
import compose  # noqa: E402
import paired  # noqa: E402

ROOT = os.environ.get("COMPUTEHUB_DATA") or common.DEFAULT_ROOT
IQ = os.environ.get("IQ") or os.path.join(ROOT, "iq", "team-n20261009", "bin", "iq.exe")
SUITE = os.path.join(ROOT, "iq", "team-n20261009", "iq.jsonl")
PREP = os.path.join(ROOT, "scratch", "prep")
NIGHT8 = os.path.join(ROOT, "iq", "team-n20261008")
DATA = all(os.path.exists(p) for p in (IQ, SUITE, PREP, NIGHT8))
WAIT = '# runs clean only\nexpect not says "qqzzxq never shown"\n'


def line(task, i, clean, rank, turns=1, held="Edits", reply="x"):
    return {"task": task, "try": i, "clean": clean, "rank": rank, "turns": turns, "held": held, "ms": 1000 * turns,
            "in": 500 * turns, "out": 100 * turns, "reply": reply, "_pass": False}


def run_of(drafts, recs=None, helper="base3b"):
    """A run as compose.load_run makes it, from drafts given whole."""
    return {"name": "x", "recs": recs or {}, "rec_pass": {}, "drafts": drafts, "errors": [], "helper": helper,
            "slots": compose.slots(helper), "K": max(len(d["lines"]) for d in drafts.values())}


class Rules(unittest.TestCase):
    def test_cut(self):
        a, b, c = line("t", 0, False, 1), line("t", 1, True, 3), line("t", 2, False, 2)
        self.assertIs(paired.cut([a, b, c], 3), b)            # the first clean
        self.assertIs(paired.cut([a, b, c], 1, 0), a)         # else the best rank above the draft's
        self.assertIsNone(paired.cut([a, b, c], 1, 1))        # not above: the draft
        d = line("t", 3, False, 2)
        self.assertIs(paired.cut([a, c, d], 3, 0), c)         # the earliest of a tie
        old = [{"task": "t", "try": 0, "clean": False, "reply": "p"},
               {"task": "t", "try": 1, "clean": False, "reply": "q"}]
        self.assertIs(paired.cut(old, 2), old[0])             # the older log kept try 1's

    def test_spent(self):
        a, b, c = line("t", 0, False, 0), line("t", 1, True, 3), line("t", 2, True, 3)
        self.assertEqual(paired.spent([a, b, c], 3), [a, b])
        self.assertEqual(paired.spent([a, b, c], 1), [a])
        self.assertEqual(paired.spent([a], 4), [a])

    def test_read_tries(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "tries-x.jsonl")
            # eval's overflow line: the draft, at the draft's rank, not clean, with what its turns cost.
            over = {"task": "b", "try": 1, "temp": 0.7, "seed": 17, "error": "context", "clean": False, "rank": 1,
                    "turns": 2, "held": "", "ms": 900, "in": 9000, "out": 50, "reply": "draft"}
            rows = [line("a", 0, False, 0, reply="old"), line("a", 1, True, 3), line("a", 0, False, 1, reply="new"),
                    line("b", 0, False, 0), over]
            paired.write_text(p, "".join(json.dumps(r) + "\n" for r in rows) + '{"task": "c", "tr')
            by, errors = paired.read_tries(p)
            self.assertEqual([r["try"] for r in by["a"]], [0, 1])
            self.assertEqual(by["a"][0]["reply"], "new")       # a rerun's line wins
            self.assertEqual(errors, {"b"})
            self.assertIsNone(paired.cut(by["b"], 2, 1))       # never kept
            self.assertEqual([r["turns"] for r in paired.spent(by["b"], 2)], [1, 2])   # but it cost
            self.assertNotIn("c", by)                          # a cut last line is not read

    def test_readable(self):
        with tempfile.TemporaryDirectory() as d:
            each = os.path.join(d, "each")
            os.makedirs(each)
            p = os.path.join(d, "team-x.jsonl")
            rec = lambda t: json.dumps({"task": t, "model": "glm+pre", "reply": "x"}) + "\n"
            paired.write_text(p, rec("a") + rec("b"))
            self.assertEqual(paired.readable(p, each), p)                # iq reads every line: graded as it is
            # A kill cut c's line mid-append; team.sh ended it at the next launch, the rerun wrote it whole.
            paired.write_text(p, rec("a") + rec("b") + rec("c")[:25] + "\n" + rec("c"))
            q = paired.readable(p, each)
            self.assertEqual(q, os.path.join(each, "clean-team-x.jsonl"))
            self.assertEqual(paired.read_text(q), rec("a") + rec("b") + rec("c"))
            os.utime(q, (1, 1))
            self.assertEqual(paired.readable(p, each), q)
            self.assertEqual(os.path.getmtime(q), 1)                    # not rewritten: its grades are kept
            for bad in ("\n", '{"task": "d", "model": "m"}\n', '{"task": "d", "model": "m", "reply": 3}\n', "[1]\n"):
                self.assertFalse(paired.iq_reads(bad), bad)

    def test_carriers(self):
        on = {"glm": [], "pre": [], "b3": ["pre"], "b3@t1": ["pre"], "b3raw": ["glm"], "final": ["b3", "pre", "q3rec"]}
        c = paired.carriers(on, ["pre"])
        self.assertEqual({k: sorted(v) for k, v in c.items()},
                         {"glm": [], "pre": ["pre"], "b3": ["pre"], "b3@t1": ["pre"], "b3raw": [], "final": ["pre"]})
        self.assertEqual(paired.carriers(on, [])["final"], frozenset())
        self.assertEqual(paired.carriers({"a": ["b"], "b": ["a"]}, ["a"])["b"], frozenset(["a"]))   # a loop ends

    def test_unfinished(self):
        with tempfile.TemporaryDirectory() as d:
            def put(name, text):
                paired.write_text(os.path.join(d, name), text)
            row = lambda t, **k: json.dumps(dict({"task": t, "model": "pre+b7", "reply": "y",
                                                  "team": {"lead": "glm+pre"}}, **k)) + "\n"
            put("answers-pre.jsonl", "".join(json.dumps({"task": t, "model": "glm+pre", "reply": "x"}) + "\n"
                                             for t in ("a", "b", "c", "not-in-the-suite")))
            suite = {"a": {}, "b": {}, "c": {}}
            inputs = compose.Inputs(d, None)
            put("team-b7.jsonl", row("a") + row("b") + row("c")[:30] + "\n")   # c's line cut: no record
            self.assertEqual(compose.unfinished(d, "b7", inputs, suite), "2 of 3 records written")
            put("team-b7.jsonl", row("a") + row("b") + row("c")[:30] + "\n" + row("c"))
            self.assertIsNone(compose.unfinished(d, "b7", inputs, suite))
            put("team-b5.jsonl", row("a"))
            for f in ("score-b5.txt", "each-b5.txt", "score-r2.txt", "each-r2.txt"):
                put(f, "pass 1/3\n")
            # Marks copied beside a run cut short do not finish it: its input says what is missing.
            self.assertEqual(compose.unfinished(d, "b5", inputs, suite), "1 of 3 records written")
            put("team-r2.jsonl", row("a", team={"lead": "base3b"}))       # its drafts (drafts-r2.jsonl) not here
            self.assertIsNone(compose.unfinished(d, "r2", inputs, suite))  # but team.sh marked it done
            put("tries-q.jsonl", json.dumps(line("a", 0, False, 0)) + "\n")
            self.assertEqual(compose.unfinished(d, "q", inputs, suite), "0 records written, its drafts not here")

    def test_unfinished_runs_decide_nothing(self):
        m = {"partial": {"b7": "2 of 5 records written"}, "equal": {"names": ["b3", "b7"], "arms": {}},
             "cascade_runs": ["b3", "b7"], "runs": {"b3raw": {"pass": 166}}, "header": {"base": "pre", "base_pass": 169},
             "curves": {}}
        v = {x["id"]: x for x in compose.verdicts([{"id": x} for x in ("D1", "D2", "D3", "D6")], m)}
        for x in ("D1", "D2"):
            self.assertIsNone(v[x]["fires"])
            self.assertEqual(v[x]["verdict"], "not decided: b7 did not finish (b7: 2 of 5 records written)")
        self.assertIn("not decided: b3 did not run", v["D3"]["verdict"])
        self.assertTrue(v["D6"]["fires"])                                # b3raw finished: decided
        m["partial"]["b3raw"] = "40 of 347 records written"
        self.assertIn("b3raw did not finish", compose.verdicts([{"id": "D6"}], m)[0]["verdict"])

    def test_family_test_is_mcnemar_for_single_runs(self):
        fam = {"a": "f1", "b": "f2", "c": "f3", "d": "f4", "e": "f5", "f": "f6"}
        dd = {"a": 1.0, "b": 1.0, "c": 1.0, "d": 1.0, "e": 1.0, "f": 0.0}
        g, l, net, se, p = paired.family_test(dd, fam, sorted(set(fam.values())))
        self.assertEqual((g, l, net), (5, 0, 5))
        self.assertAlmostEqual(p, paired.mcnemar(5, 0))
        self.assertEqual(paired.needs(6, 0.05), 6)               # the bar the plan quotes: +6 all one way

    def test_compose_never_worse(self):
        base = ("pre", [("a", "A0", True), ("b", "B0", False), ("c", "C0", False), ("d", "D0", False)])
        b3 = ("b3", [("a", "A1", True), ("b", "B1", False), ("c", "C1", True)])
        rec = ("q3rec", [("b", "B2", False), ("b", "B3", True), ("b", "B4", True), ("c", "C2", True)])
        out = compose.compose([base, b3, rec])
        self.assertEqual(out["a"], ("pre", "A0", True))       # the base ran clean: kept
        self.assertEqual(out["b"], ("q3rec", "B3", False))    # the first clean sample, in file order
        self.assertEqual(out["c"], ("b3", "C1", False))       # the first source in order
        self.assertEqual(out["d"], ("pre", "D0", False))      # nothing ran clean: the base's

    def test_inputs_are_the_pick_not_its_samples(self):
        # fin7r repairs fin7's pick (answers-fin7.jsonl); the samples it was picked from carry the same
        # model, and sort first: a draft's pass is the pick's, never its first sample's.
        class Grades:
            def grades(self, path, wait=False):
                return {(r["task"], r["reply"]): {"pass": r["reply"] == "good"} for r in paired.jl(path)}
        with tempfile.TemporaryDirectory() as d:
            rows = lambda *xs: "".join(json.dumps({"task": "t", "model": "fin7", "reply": x}) + "\n" for x in xs)
            paired.write_text(os.path.join(d, "answers-fin7-samples.jsonl"), rows("bad", "good"))
            paired.write_text(os.path.join(d, "answers-fin7.jsonl"), rows("good"))
            paired.write_text(os.path.join(d, "answers-fin7r-at-t1.jsonl"), rows("bad"))
            self.assertEqual(compose.Inputs(d, Grades()).passes({"fin7"}), {"t": True})

    def test_ks(self):
        self.assertEqual(compose.ks_for(8), [1, 2, 4, 8])
        self.assertEqual(compose.ks_for(4), [1, 2, 4])
        self.assertEqual(compose.ks_for(3), [1, 2, 3])
        self.assertEqual(compose.ks_for(1), [1])
        self.assertEqual(compose.ks_for(0), [])

    def test_curve(self):
        def p(r, ok):
            r["_pass"] = ok
            return r
        drafts = {
            "e": {"lines": [line("e", 0, False, 0, 2, "Edits Missed"), p(line("e", 1, True, 3, 1, "Program"), False),
                            p(line("e", 2, True, 3, 1, "Program"), True)],
                  "draft_rank": 0, "draft_pass": False, "code": 101},
            "f": {"lines": [p(line("f", 0, True, 3, 1, "Diff"), True), line("f", 1, False, 0, 3, "Edits Edits Edits"),
                            line("f", 2, False, 1, 3, "Inline Edits Nothing")],
                  "draft_rank": 1, "draft_pass": False, "code": 215},
            "g": {"lines": [line("g", 0, False, 1, 3, "Edits Edits Edits"),
                            line("g", 1, False, 0, 3, "Missed Missed Missed"),
                            line("g", 2, False, 1, 3, "Edits Inline Nothing")],
                  "draft_rank": 0, "draft_pass": False, "code": 101},
        }
        recs = {t: {"team": {"clean": t in ("e", "f"), "turns": 1}} for t in drafts}
        c = compose.curve(run_of(drafts, recs))
        self.assertEqual([r["k"] for r in c["rows"]], [1, 2, 3])
        self.assertEqual([r["clean"] for r in c["rows"]], [1, 2, 2])
        self.assertEqual([r["first"] for r in c["rows"]], [1, 1, 1])
        self.assertEqual([r["oracle"] for r in c["rows"]], [1, 1, 2])
        self.assertEqual([r["calls"] for r in c["rows"]], [6, 10, 13])   # tries up to the first clean
        self.assertEqual(c["records_clean"], c["rows"][-1]["clean"])
        last = c["rows"][-1]
        self.assertEqual((last["pass"], last["extra"]), (1, 1))
        self.assertAlmostEqual(last["gpu_s"], 13.0 / 16)
        self.assertAlmostEqual(last["gpu_s_per_extra"], 13.0 / 16)
        self.assertAlmostEqual(last["sample_clean"], (2 / 3 + 1 / 3 + 0) / 3)
        self.assertAlmostEqual(last["sample_pass"], (1 / 3 + 1 / 3 + 0) / 3)
        self.assertEqual(c["held"]["turns"], 20)
        self.assertAlmostEqual(c["held"]["share"]["Edits"], 9 / 20)
        self.assertEqual(list(c["held"]["share"])[:6], list(compose.HELD))
        self.assertEqual(sorted(c["codes"]), ["101", "215"])
        self.assertEqual([r["clean"] for r in c["codes"]["101"]], [0, 1, 1])
        for r in c["rows"]:
            self.assertGreaterEqual(r["oracle"], r["first"])

    def test_knee(self):
        rows = [{"k": 1, "clean": 10, "calls": 100}, {"k": 2, "clean": 14, "calls": 200},
                {"k": 4, "clean": 15, "calls": 400}]
        self.assertEqual(compose.knee(rows, 1)[0], 2)          # 2 -> 4: 0.5 a 100 calls
        self.assertEqual(compose.knee(rows[:2], 1), (2, None))

    def test_d1(self):
        arms = {"dev3@1": {"pass_pct": 30.0}, "dev7@1": {"pass_pct": 50.0}, "dev3w@1": {"pass_pct": 42.0}}
        m = {"dev": {"arms": arms, "pairs": {"dev3@1:dev7@1": {"p": 0.01}}},
             "equal": {"k": 4, "pairs": {"b3:b7": {"net": 1.0}}}}
        fires, v = compose.d1({}, m)
        self.assertTrue(fires)
        self.assertIn("escalation tier", v)
        self.assertIn("reply format", v)                        # 12 of 20 points closed
        arms["dev3w@1"]["pass_pct"] = 35.0
        self.assertIn("the gap is size", compose.d1({}, m)[1])
        arms["dev7@1"]["pass_pct"] = 33.0
        fires, v = compose.d1({}, m)
        self.assertFalse(fires)
        self.assertIn("size is not the lever", v)
        m["equal"]["pairs"] = {}
        v = compose.verdicts([{"id": "D1"}], m)[0]
        self.assertIsNone(v["fires"])
        self.assertIn("not decided", v["verdict"])

    def test_d2_d5_d6(self):
        arms = {"b3": {"pass": 5, "gpu_s": 100.0, "gpu_s_per_extra": 20.0},
                "b7": {"pass": 6, "gpu_s": 600.0, "gpu_s_per_extra": 100.0},
                "b05": {"pass": 2, "gpu_s": 10.0, "gpu_s_per_extra": 5.0}}
        both = {"both": True, "label": "b3→b7", "pass": 6, "gpu_s": 300.0, "gpu_s_per_extra": 50.0}
        m = {"equal": {"arms": arms}, "cascade": {"rows": {"b3→b7": both}},
             "finish": {"tasks": 25, "rows": {"fin7": {"pass": 3, "clean": 12}}},
             "runs": {"b3raw": {"pass": 166}}, "header": {"base": "pre", "base_pass": 169}}
        self.assertIn("default helper: b3 ", compose.d2({"within": 1}, m)[1])
        self.assertTrue(compose.d5({}, m)[0])
        self.assertTrue(compose.d6({}, m)[0])
        m["runs"]["b3raw"]["pass"] = 165                      # under pre - 3
        self.assertIn("ship pre's reader parts", compose.d6({}, m)[1])
        v = compose.verdicts([{"id": "D9", "name": "Later"}], m)[0]
        self.assertIsNone(v["fires"])

    def test_header(self):
        hd = {"glm": 152, "read": 157, "read_by_day": True, "base": "pre", "base_pass": 169, "rec": 171,
              "rec_name": "q3rec", "final": 174}
        self.assertEqual(compose.header_line(hd),
                         "glm 152 | read 157 (by day) | pre 169 (harness, fitted on held; out of sample on 52 "
                         "fresh GLM drafts, 4 of 13 made clean and 1 passed) | pre+q3rec 171 | final 174")

    def test_auc_of(self):
        self.assertEqual(compose.auc_of(0.8), (0.8, None, None))
        self.assertEqual(compose.auc_of({"auc": 0.6, "ci": [0.5, 0.7]}), (0.6, 0.5, 0.7))
        self.assertEqual(compose.auc_of({"lift": 0.01, "lo": -0.02, "hi": 0.03}), (0.01, -0.02, 0.03))
        self.assertIsNone(compose.auc_of("x"))


def app(src):
    return "```app\n" + src.rstrip("\n") + "\n```"


STUB = "// stub\nlabel \"qq\";\n"
BROKEN = "// broken\nlabel \"qq\"\n"
RUNTIME = "// rt\nstate a = [0; 2];\nlabel a[5];\n"
TASKS = ["counter", "char-count-live", "countdown-seconds", "flashcards-capitals", "loan-simple-interest",
         "word-count-goal", "score-keeper-darts", "password-generator-kinds"]
A, B, C, D, E, F, G, H = TASKS


def fixture(d):
    """A night in d: see the module's docs."""
    suite = {j["id"]: j for j in paired.jl(SUITE)}
    prog = {"ref": lambda t: app(suite[t]["ref"]), "stub": lambda t: app(STUB), "broken": lambda t: app(BROKEN),
            "runtime": lambda t: app(RUNTIME), "none": lambda t: "I ran out of room before the program."}
    code = {"broken": 101, "runtime": 215}
    rank = {"ref": 3, "stub": 3, "runtime": 1, "broken": 0, "none": 0}

    def put(name, rows):
        paired.write_text(os.path.join(d, name), "".join(json.dumps(r) + "\n" for r in rows))

    put("iq.jsonl", [suite[t] for t in TASKS])
    put("suite-wait.jsonl", [dict(suite[t], check=WAIT) for t in TASKS])
    glm = {A: "ref", B: "ref", C: "broken", D: "stub", E: "broken", F: "runtime", G: "broken", H: "none"}
    pre = dict(glm, **{C: "ref"})
    put("answers-glm.jsonl", [{"task": t, "model": "zai/glm-5.3", "reply": prog[glm[t]](t)} for t in TASKS])
    put("answers-pre.jsonl", [{"task": t, "model": "glm+pre", "reply": prog[pre[t]](t)} for t in TASKS])

    def team(name, helper, ms, plan, lead="glm+pre", drafts=pre, tasks=TASKS):
        """plan: {task: [(kind, turns, held)] a try}: the try log and the out records eval writes;
        lead: the drafts' model, or {task: model}."""
        tries, recs = [], []
        for t in tasks:
            kind = drafts[t]
            tl = []
            for i, (k, turns, held) in enumerate(plan.get(t, [])):
                tl.append({"task": t, "try": i, "temp": 0.2 if i == 0 else 0.7, "seed": 1 + 16 * i,
                           "clean": rank[k] == 3, "rank": rank[k], "before": code.get(kind, 0), "after": code.get(k, 0),
                           "turns": turns, "held": held, "kept": [3, 3], "ms": ms * turns, "in": 500 * turns,
                           "out": 100 * turns, "reply": prog[k](t)})
            tries += tl
            pick = paired.cut(tl, len(tl), rank[kind]) if tl else None
            reply = pick["reply"] if pick else prog[kind](t)
            info = {"lead": lead[t] if isinstance(lead, dict) else lead, "helper": helper, "tries": len(tl),
                    "turns": pick["turns"] if pick else 0, "held": pick["held"] if pick else "",
                    "clean": bool(pick["clean"]) if pick else rank[kind] == 3, "before": code.get(kind, 0),
                    "ms": sum(x["ms"] for x in tl), "clean_tries": sum(x["clean"] for x in tl),
                    "kept_try": tl.index(pick) if pick else -1, "draft_rank": rank[kind],
                    "best_rank": max([x["rank"] for x in tl] + [rank[kind]]), "calls": sum(x["turns"] for x in tl),
                    "in": sum(x["in"] for x in tl), "out": sum(x["out"] for x in tl)}
            recs.append({"task": t, "model": "glm+" + name, "reply": reply, "team": info})
        put("tries-%s.jsonl" % name, tries)
        put("team-%s.jsonl" % name, recs)

    team("b3", "base3b", 1000, {
        A: [("ref", 0, "")],   # clean already: a try that took no turn, not a draft
        E: [("broken", 2, "Edits Missed"), ("stub", 1, "Program"), ("ref", 1, "Program")],
        F: [("ref", 1, "Diff"), ("broken", 3, "Edits Edits Edits"), ("runtime", 3, "Inline Edits Nothing")],
        G: [("runtime", 3, "Edits Edits Edits"), ("broken", 3, "Missed Missed Missed"),
            ("runtime", 3, "Edits Inline Nothing")]})
    team("b7", "base7b", 4000, {
        E: [("ref", 1, "Program"), ("ref", 1, "Program")],
        F: [("broken", 3, "Edits Edits Edits"), ("broken", 3, "Edits Edits Edits")],
        G: [("stub", 2, "Program Program"), ("ref", 1, "Program")]})
    dev, kinds = {E: "broken", F: "runtime", G: "broken"}, {E: "decl", F: "bound1", G: "toplevel"}
    mut = {t: "mut-" + k for t, k in kinds.items()}
    put("answers-dev.jsonl", [{"task": t, "model": mut[t], "kind": kinds[t], "code": code[dev[t]],
                               "reply": prog[dev[t]](t)} for t in (E, F, G)])
    team("dev3", "base3b", 1000, {E: [("broken", 1, "Edits"), ("ref", 1, "Program")],
                                  F: [("ref", 1, "Program"), ("ref", 1, "Program")],
                                  G: [("broken", 1, "Edits"), ("broken", 1, "Edits")]}, mut, dev, [E, F, G])
    team("dev7", "base7b", 4000, {E: [("ref", 1, "Program")], F: [("ref", 1, "Program")], G: [("stub", 1, "Program")]},
         mut, dev, [E, F, G])
    put("q3rec.jsonl", [{"task": H, "model": "q3-gram", "gen": 0, "reply": prog["broken"](H)},
                        {"task": H, "model": "q3-gram", "gen": 1, "reply": prog["ref"](H)},
                        {"task": E, "model": "q3-gram", "gen": 0, "reply": prog["ref"](E)}])
    put("prompts-fin.jsonl", [{"task": H, "messages": []}])
    put("answers-fin7.jsonl", [{"task": H, "model": "fin7", "reply": prog["ref"](H)}])
    # judge.py's shape: each set's AUCs under "auc", the rank-average's lift over length beside them.
    paired.write_text(os.path.join(d, "judgefit-j3.json"), json.dumps({"sets": {"pre": {
        "auc": {"judge": {"auc": 0.6, "lo": 0.55, "hi": 0.65}, "length": {"auc": 0.8, "lo": 0.75, "hi": 0.85},
                "tier": {"auc": 0.7, "lo": 0.6, "hi": 0.8}, "rankavg": {"auc": 0.81, "lo": 0.76, "hi": 0.86}},
        "lift": {"auc": 0.01, "lo": -0.02, "hi": 0.03}}}}))
    paired.write_text(os.path.join(d, "judgefit-j7.json"), json.dumps({"pre": {"judge": 0.9, "length": 0.8}}))  # flat
    put("predictions.jsonl", [{"system": "final", "lo": 5, "hi": 6},
                              {"system": "b3", "what": "made_clean", "lo": 3, "hi": 4},
                              {"system": "dev7@1", "what": "pass_pct", "lo": 40, "hi": 70},
                              {"system": "b05", "lo": 1, "hi": 2}])
    rules = [{"id": x, "name": x, "rule": "rule %s" % x} for x in ("D1", "D2", "D3", "D4", "D5", "D6", "D7")]
    paired.write_text(os.path.join(d, "decisions.json"), json.dumps({"rules": rules}))


def py(*args):
    r = subprocess.run([sys.executable, *args], capture_output=True)
    if r.returncode != 0:
        raise AssertionError("%s: %s" % (" ".join(args[:1]), r.stderr.decode("utf-8", "replace")[-2000:]))
    return r.stdout.decode("utf-8", "replace")


def text(path):
    return paired.read_text(path).replace("\r\n", "\n")   # the day's prototype wrote CRLF


@unittest.skipUnless(DATA, "needs iq.exe and the data root (%s)" % ROOT)
class Night(unittest.TestCase):
    def test_paired_reproduces_n20261008(self):
        with tempfile.TemporaryDirectory() as t:
            each = os.path.join(t, "each")
            os.makedirs(each)
            for f in os.listdir(os.path.join(PREP, "each")):
                shutil.copy2(os.path.join(PREP, "each", f), each)   # their times, so the grades are kept
            py(os.path.join(HERE, "paired.py"), "--dir", NIGHT8, "--each-dir", each, "--out", os.path.join(t, "p.md"),
               "--json", os.path.join(t, "p.json"), "--vs", "base3b:fix3b", "--vs", "base3b-plain:base3b",
               "--vs", "q3-plain:base3b", "--vs", "base3b:base05-plain", "--vs", "base3b@t1:base3b-t4",
               "--same", "base3b-plain:base3b")
            self.assertEqual(text(os.path.join(t, "p.md")), text(os.path.join(PREP, "paired-n20261008.md")))
            self.assertEqual(text(os.path.join(t, "p.json")), text(os.path.join(PREP, "paired-n20261008.json")))
            md = text(os.path.join(t, "p.md"))
            self.assertIn("| base3b | 1 | 158 | 45.5 | 100/125 | 58/222 | +6 −0 | +6 | [+1.3, +10.7] | 0.031 |", md)
            self.assertIn("| fix3b against base3b | 157 | 158 | +1 −2 | -1 |", md)
            self.assertIn("no difference shown |", md)
            self.assertEqual(paired.read_text(os.path.join(t, "p.md")).count("\r"), 0)

    def test_sys_pre_against_glm(self):
        with tempfile.TemporaryDirectory() as t:
            for f in os.listdir(os.path.join(PREP, "each")):
                shutil.copy2(os.path.join(PREP, "each", f), t)
            py(os.path.join(HERE, "paired.py"), "--dir", NIGHT8, "--each-dir", t, "--iq", IQ,
               "--sys", "pre=" + os.path.join(PREP, "mx-answers-pre.jsonl"), "--vs", "glm:pre",
               "--json", os.path.join(t, "p.json"))
            dec = json.loads(text(os.path.join(t, "p.json")))["decisions"][0]
            self.assertEqual((dec["a"], dec["b"], dec["gained"], dec["lost"]), ("glm", "pre", 17, 0))

    def test_compose_pre_and_q3rec_is_171(self):
        with tempfile.TemporaryDirectory() as t:
            shutil.copy(SUITE, os.path.join(t, "iq.jsonl"))
            shutil.copy(os.path.join(PREP, "mx-suite-wait.jsonl"), os.path.join(t, "suite-wait.jsonl"))
            shutil.copy(os.path.join(PREP, "mx-answers-pre.jsonl"), os.path.join(t, "answers-pre.jsonl"))
            shutil.copy(os.path.join(ROOT, "iq", "answers-n20261007-q3-gram.jsonl"), os.path.join(t, "q3rec.jsonl"))
            py(os.path.join(HERE, "compose.py"), "--dir", t, "--iq", IQ)
            m = json.loads(text(os.path.join(t, "report.json")))
            self.assertEqual((m["header"]["base_pass"], m["header"]["rec"], m["header"]["final"]), (169, 171, 171))
            self.assertEqual(m["missing"], ["b3", "b7", "fin7", "fin7r"])
            self.assertEqual(m["composition"]["lost"], 0)
            self.assertEqual(m["equal"]["drafts"], 39)
            pre = {j["task"]: j for j in paired.jl(os.path.join(PREP, "mx-answers-pre-each.txt")) if "pass" in j}
            fin = {j["task"]: j for j in paired.jl(os.path.join(t, "each-final.txt")) if "pass" in j}
            gained = sorted(x for x in fin if fin[x]["pass"] and not pre[x]["pass"])
            self.assertEqual(gained, ["bedtime-routine", "screensaver-bounce"])
            froms = {r["task"]: r["from"] for r in paired.jl(os.path.join(t, "answers-final.jsonl"))}
            self.assertEqual({froms[x] for x in gained}, {"q3rec"})
            self.assertIn("pass 171/347", text(os.path.join(t, "score-final.txt")))

    def test_fixture_night(self):
        with tempfile.TemporaryDirectory() as t:
            fixture(t)
            out = py(os.path.join(HERE, "compose.py"), "--dir", t, "--iq", IQ, "--equal-k", "2").replace("\r\n", "\n")
            m = json.loads(text(os.path.join(t, "report.json")))
            hd = m["header"]
            self.assertEqual((hd["glm"], hd["base_pass"], hd["rec"], hd["final"]), (2, 3, 5, 5))
            froms = {r["task"]: r["from"] for r in paired.jl(os.path.join(t, "answers-final.jsonl"))}
            self.assertEqual(froms, {A: "pre", B: "pre", C: "pre", D: "pre", E: "b3", F: "b3", G: "b7", H: "fin7"})
            self.assertEqual(m["harness"], {"gained": 1, "lost": 0})
            self.assertEqual((m["models"]["gained"], m["models"]["lost"]), (2, 0))
            c = m["curves"]["b3"]
            self.assertEqual((c["K"], c["drafts"], c["slots"]), (3, 3, 16))
            self.assertEqual([r["clean"] for r in c["rows"]], [1, 2, 2])
            self.assertEqual([r["first"] for r in c["rows"]], [1, 1, 1])
            self.assertEqual([r["oracle"] for r in c["rows"]], [1, 1, 2])
            self.assertEqual([r["calls"] for r in c["rows"]], [6, 10, 13])
            self.assertEqual([r["run_pass"] for r in c["rows"]], [4, 4, 4])
            self.assertEqual(c["rows"][-1]["clean"], c["records_clean"])        # made clean@3 = the out records' clean
            self.assertEqual(sum(1 for r in paired.jl(os.path.join(t, "team-b3.jsonl"))
                                 if r["team"]["clean"] and r["team"]["turns"] > 0), 2)
            for cv in m["curves"].values():
                for r in cv["rows"]:
                    self.assertGreaterEqual(r["oracle"], r["first"])
            self.assertEqual(m["curves"]["b7"]["slots"], 8)
            eq = m["equal"]
            self.assertEqual((eq["k"], eq["drafts"], eq["missing"]), (2, 3, ["b05", "b3w"]))
            self.assertEqual([(eq["arms"][x]["pass"], eq["arms"][x]["clean"]) for x in ("b3", "b7")], [(1, 2), (1, 2)])
            self.assertEqual((eq["pairs"]["b3:b7"]["gained"], eq["pairs"]["b3:b7"]["lost"]), (1, 1))
            both = m["cascade"]["rows"]["b3→b7"]
            self.assertEqual(both["pass"], 1)
            self.assertAlmostEqual(both["gpu_s"], 3 / 16 + 1 / 16 + 6 / 16 + 8 / 8)
            dv = m["dev"]["arms"]
            self.assertEqual([dv[x]["pass"] for x in ("dev3@1", "dev3@2", "dev7@1")], [1, 2, 2])
            self.assertEqual(dv["dev3@1"]["groups"], {"reasoning": {"n": 2, "pass": 1, "clean": 1},
                                                      "pattern": {"n": 1, "pass": 0, "clean": 0}})
            self.assertEqual(sorted(m["dev"]["missing"]), ["dev05@1", "dev3w@1"])
            f = m["finish"]
            self.assertEqual((f["tasks"], f["rows"]["fin7"]["clean"], f["rows"]["fin7"]["pass"]), (1, 1, 1))
            self.assertEqual(f["overlap"]["both"], 1)
            v = {x["id"]: x for x in m["decisions"]}
            self.assertIn("neither branch", v["D1"]["verdict"])
            self.assertIn("default helper: b3 ", v["D2"]["verdict"])
            self.assertIn("b3 at 2", v["D3"]["verdict"])
            self.assertIn("is not worth building", v["D3"]["verdict"])
            self.assertFalse(v["D4"]["fires"])
            self.assertEqual(m["judges"]["j3"], {"judge": [0.6, 0.55, 0.65], "length": [0.8, 0.75, 0.85],
                                                 "lift": [0.01, -0.02, 0.03]})
            self.assertEqual(m["judges"]["j7"]["judge"], [0.9, None, None])
            self.assertFalse(v["D5"]["fires"])
            self.assertIsNone(v["D6"]["fires"])
            self.assertIsNone(v["D7"]["fires"])
            states = [(p["system"], p["state"]) for p in m["predictions"]]
            self.assertEqual(states, [("final", "inside"), ("b3", "below"), ("dev7@1", "inside"), ("b05", "not run")])
            self.assertTrue(out.startswith("# Night report"))
            self.assertIn("ceiling: chosen by the hidden check, never a system", out)
            self.assertEqual(text(os.path.join(t, "report.md")) + "\n", out)      # printed whole
            for name in ("report.md", "report.json", "answers-final.jsonl", "score-final.txt", "each-final.txt"):
                self.assertEqual(paired.read_text(os.path.join(t, name)).count("\r"), 0, name)

            # paired.py on the same night: b3 and b7 measured against pre, their own input, and cut at fewer tries.
            py(os.path.join(HERE, "paired.py"), "--dir", t, "--iq", IQ, "--sys", "pre=answers-pre.jsonl",
               "--out", os.path.join(t, "p.md"), "--json", os.path.join(t, "p.json"),
               "--predictions", os.path.join(t, "predictions.jsonl"))
            s = json.loads(text(os.path.join(t, "p.json")))["systems"]
            self.assertEqual({x: s[x]["pass"] for x in ("glm", "pre", "b3", "b3@t1", "b3@t2", "b7", "b7@t1")},
                             {"glm": 2, "pre": 3, "b3": 4, "b3@t1": 4, "b3@t2": 4, "b7": 4, "b7@t1": 4})
            self.assertEqual({x: s[x]["helper_calls"] for x in ("b3", "b3@t1", "b3@t2", "b7", "b7@t1")},
                             {"b3": 13, "b3@t1": 6, "b3@t2": 10, "b7": 9, "b7@t1": 6})
            md = text(os.path.join(t, "p.md"))
            self.assertIn("Helper calls per extra solve are against each run's own input: "
                          "b3, b3@t1, b3@t2, b7, b7@t1 on pre.", md)
            self.assertIn("| b3 | 1 | 4 | 50.0 |", md)
            self.assertIn("| 13 | 2.00 | 13 |", md)      # 13 calls, 8 / 4 cloud calls a solve, +1 over pre

            # pre, the harness, fitted on these tasks: what tells its gain apart gets counts, never a test.
            py(os.path.join(HERE, "paired.py"), "--dir", t, "--iq", IQ, "--sys", "pre=answers-pre.jsonl",
               "--sys", "final=answers-final.jsonl", "--in-sample", "pre", "--out", os.path.join(t, "q.md"),
               "--json", os.path.join(t, "q.json"), "--vs", "glm:pre", "--vs", "pre:b3", "--vs", "pre:final",
               "--vs", "glm:b3", "--vs", "b3:b7")
            q = json.loads(text(os.path.join(t, "q.json")))
            dec = {(c["a"], c["b"]): c for c in q["decisions"]}
            for pair in (("glm", "pre"), ("glm", "b3")):
                c = dec[pair]
                self.assertEqual((c["in_sample"], c["p"], c["lo"], c["needs"]), (["pre"], None, None, None), pair)
                self.assertTrue(c["verdict"].startswith("in-sample (pre, fitted on these tasks)"), pair)
            self.assertEqual((dec[("glm", "pre")]["gained"], dec[("glm", "pre")]["lost"]), (1, 0))
            for pair in (("pre", "b3"), ("pre", "final"), ("b3", "b7")):   # both sides carry pre: tested
                self.assertNotIn("in_sample", dec[pair], pair)
                self.assertIsInstance(dec[pair]["p"], float, pair)
            self.assertIsNone(q["vs_glm"]["final"]["p"])
            md = text(os.path.join(t, "q.md"))
            row = next(x for x in md.splitlines() if x.startswith("| pre | 1 | 3 |"))
            self.assertIn("| +1 −0 | +1 | — | in-sample | — |", row)
            self.assertIn("| pre against glm | 3 | 2 | +1 −0 | +1 | — | in-sample | — |", md)
            self.assertIn("in-sample: against GLM alone, the nets of pre and of what is built on it hold its gain", md)

    def test_cut_line_and_unfinished_run(self):
        with tempfile.TemporaryDirectory() as t:
            fixture(t)
            # A kill cut b3's last out line mid-append; team.sh ended it at the next launch, and the
            # rerun wrote the whole line after it.
            p = os.path.join(t, "team-b3.jsonl")
            rows = paired.read_text(p).splitlines(True)
            paired.write_text(p, "".join(rows[:-1]) + rows[-1][:60] + "\n" + rows[-1])
            # b7 stopped midway (a step left for the morning): F's tries logged but not its record, G's neither.
            for name, keep in (("team-b7.jsonl", lambda r: r["task"] not in (F, G)),
                               ("tries-b7.jsonl", lambda r: r["task"] != G)):
                q = os.path.join(t, name)
                paired.write_text(q, "".join(json.dumps(r) + "\n" for r in paired.jl(q) if keep(r)))
            out = py(os.path.join(HERE, "compose.py"), "--dir", t, "--iq", IQ, "--equal-k", "2").replace("\r\n", "\n")
            m = json.loads(text(os.path.join(t, "report.json")))
            self.assertEqual(m["partial"], {"b7": "6 of 8 records written"})
            self.assertNotIn("b7", m["curves"])
            self.assertEqual(m["curves"]["b3"]["rows"][-1]["clean"], 2)      # b3 read whole past its cut line
            self.assertIn("b7", m["missing"])                                  # out of the composition too
            froms = {r["task"]: r["from"] for r in paired.jl(os.path.join(t, "answers-final.jsonl"))}
            self.assertEqual(froms[G], "pre")
            self.assertEqual(m["equal"]["missing"], ["b05", "b3w", "b7"])
            self.assertNotIn("cascade", m)
            v = {x["id"]: x for x in m["decisions"]}
            for x in ("D1", "D2"):
                self.assertIsNone(v[x]["fires"], x)
                self.assertEqual(v[x]["verdict"], "not decided: b7 did not finish (b7: 6 of 8 records written)")
            self.assertIn("b7 did not finish", v["D3"]["verdict"])
            self.assertIn("is not worth building", v["D3"]["verdict"])        # b3 and dev3 decide it
            self.assertIn("- did not finish, so left out of every table and verdict (rerun its step, then the "
                          "summary): b7 (6 of 8 records written).", out)
            self.assertIn("Not run: b05, b3w, b7 (did not finish).", out)

            # paired.py grades b3 through a copy without its cut line, and leaves the unfinished b7 out.
            r = subprocess.run([sys.executable, os.path.join(HERE, "paired.py"), "--dir", t, "--iq", IQ,
                                "--sys", "pre=answers-pre.jsonl", "--json", os.path.join(t, "p.json")],
                               capture_output=True)
            self.assertEqual(r.returncode, 0, r.stderr.decode("utf-8", "replace")[-2000:])
            self.assertIn("b7 grades 6 of 8 tasks (a partial run): left out", r.stderr.decode("utf-8", "replace"))
            s = json.loads(text(os.path.join(t, "p.json")))["systems"]
            self.assertEqual(s["b3"]["pass"], 4)
            self.assertNotIn("b7", s)
            self.assertEqual(len(paired.jl(os.path.join(t, "clean-team-b3.jsonl"))), len(TASKS))

    @unittest.skipUnless(os.environ.get("TEAM_STUB"), "TEAM_STUB names item 1's stub run")
    def test_stub_run(self):
        d = os.environ["TEAM_STUB"]
        with tempfile.TemporaryDirectory() as t:
            g = compose.Grader(IQ, SUITE, None, t)
            names = sorted(f[len("tries-"):-len(".jsonl")] for f in os.listdir(d)
                           if f.startswith("tries-") and f.endswith(".jsonl"))
            self.assertTrue(names)
            for name in names:
                run = compose.load_run(d, name, g, compose.Inputs(d, g))
                if not run["drafts"]:
                    continue   # an empty log (a run killed before its first draft was written)
                c = compose.curve(run)
                self.assertEqual(c["rows"][-1]["clean"], c["records_clean"], name)
                for r in c["rows"]:
                    self.assertGreaterEqual(r["oracle"], r["first"], name)


if __name__ == "__main__":
    unittest.main()
