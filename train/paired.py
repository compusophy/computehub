#!/usr/bin/env python
"""The team night's paired report: every team run of a night set beside GLM alone and beside each
other, task by task, on the same held-out drafts.

  python train/paired.py --dir IQ/team-<night> [--iq IQ.exe] [--each-dir DIR] [--out paired.md]
                         [--json paired.json] [--vs A:B ...] [--same RUN1:RUN2 ...] [--alpha 0.05]
                         [--sys NAME=FILE ...] [--in-sample NAME ...] [--predictions FILE]

Runs. Each team-NAME.jsonl in --dir whose tasks are the held-out ones is a run (team-train-* are
the train split and are left out). Runs named NAME-sK (K a number) are replicates of NAME under
another seed set, NAME itself being s1. A run with a try log, tries-NAME.jsonl (a line a try: task,
try, clean, rank, turns, reply ...; from `eval team --tries-out`, or the older --each-try's task,
try, reply, clean), also gives NAME@t1 .. NAME@t(K-1): the same run cut at k tries, exactly paired
with NAME, each draft kept as eval keeps it (`cut`: the first clean try, else the try of highest
rank above the draft's, else the draft; the older log: else try 1's). --sys NAME=FILE registers an
answers file as a system of its own (pre: GLM's drafts read and mended by the harness, no model;
final: compose.py's merge), one cloud call a task. Every answers file is graded once by `iq score
--each` (kept as each-<file>.txt in --each-dir, regraded when older than its answers). iq refuses
a whole file for one line it cannot read, so a file with such a line (one cut by a kill: team.sh
ends it, and the rerun writes the whole line after it) is graded through a copy without them,
clean-<file> in --each-dir, rewritten only when they change. The baseline is answers-glm.jsonl:
GLM alone, one cloud call a task.

In-sample. --in-sample NAME names a system fitted on these held tasks (pre: the harness, its
fixers made from these very failures), whose gain on them is no evidence. A system carries it when
it is it, repairs its answers (a run whose input it is) or merges them (an --sys file whose records
say "from": compose.py's final). A pair whose two sides do not carry the same such systems is told
apart in part by that gain: it gets its counts, never an interval, a p-value or a verdict (GLM
against pre, against final or against a run on pre; pre against a run on GLM's drafts). Pairs on
the same side (pre against a run on pre, pre against final) are tested as any other.

Formulas. Per task i, system s with R_s runs: y_si = its pass fraction over the runs. Against A:
d_i = y_Bi - y_Ai; gained = sum max(d_i, 0); lost = sum max(-d_i, 0); net = sum d_i. Tasks of one
family are not independent, so the test is a paired sign-flip by family: D_f = sum of d_i over
family f; p = share of the 2^F' sign patterns (F' families with D_f != 0; exact up to 2^20, else
200,000 flips from seed 1) with |sum +-D_f| >= |net|. With one run each and one discordant task a
family it is McNemar's exact test. 95% interval: net +- 1.96 * sqrt(sum_f D_f^2 - net^2 / F) (F:
all families). "needs": the smallest |net| that reads as a difference at this many discordant
tasks (b + c, single runs). "clean": the same test on whether the helper made the draft run clean
(the helper's own skill; many more events than passes). Noise: for R >= 2 runs, SD of the
per-run passes, and sigma from flips = sqrt(R/(R-1) * sum_i y_i (1 - y_i)).

Cost. Cloud calls C = one a task with a GLM answer (plus any "cloud" a team record counts, for
escalation); cloud calls per solve = C / passes. Helper calls H = distinct (task, try, turn) in
traces-NAME.jsonl (else the records' turns); with a try log that counts turns, the turns of the
tries a run stopping at its first clean try makes (`spent`: --all-tries runs on past it, to
measure). Helper calls per extra solve = H / net against the run's own input: the system whose
answers' model is the records' lead (b3 and b7 repair pre's drafts, so they are measured against
pre when pre is registered by --sys), else GLM.
"""
import argparse
import glob
import itertools
import json
import math
import os
import random
import re
import subprocess
import sys

EVERYDAY = (1, 2)
GAMES = (3, 4, 5, 6)


def jl(path):
    out = []
    if not os.path.exists(path):
        return out
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line.startswith("{"):
                try:
                    out.append(json.loads(line))
                except ValueError:
                    pass
    return out


def read_text(path):
    """A file's text as written (its line ends untouched)."""
    with open(path, encoding="utf-8", newline="") as f:
        return f.read()


def write_text(path, text):
    """text to path, its line ends LF."""
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)


def each_path(answers, each_dir=None, prefix="each-"):
    return os.path.join(each_dir or os.path.dirname(answers),
                        prefix + os.path.basename(answers)[:-len(".jsonl")] + ".txt")


def grade_lines(iq, suite, answers, each):
    """iq score --each on answers: a grade a line, in file order, cached in `each` (regraded when
    older than the answers)."""
    if not (os.path.exists(each) and os.path.getmtime(each) >= os.path.getmtime(answers)):
        r = subprocess.run([iq, "score", answers, "--suite", suite, "--each"], capture_output=True)
        if r.returncode != 0:
            sys.exit("iq score %s: %s" % (answers, r.stderr.decode("utf-8", "replace")))
        with open(each, "wb") as f:
            f.write(r.stdout)
    return [j for j in jl(each) if "pass" in j]


def iq_reads(line):
    """Whether `iq score` reads line: a JSON object whose task, model and reply are strings (a
    blank line it refuses too)."""
    try:
        x = json.loads(line)
    except ValueError:
        return False
    return isinstance(x, dict) and all(isinstance(x.get(k), str) for k in ("task", "model", "reply"))


def readable(answers, each_dir=None):
    """The file iq grades for answers: answers itself when iq reads every line of it, else a copy
    without the lines it cannot read (clean-<file> in each_dir, rewritten only when they change, so
    its grades are kept): one cut line must not fail the grading of the rest."""
    lines = read_text(answers).splitlines(True)
    keep = [x for x in lines if iq_reads(x)]
    if len(keep) == len(lines):
        return answers
    out = os.path.join(each_dir or os.path.dirname(answers), "clean-" + os.path.basename(answers))
    text = "".join(x.rstrip("\r\n") + "\n" for x in keep)
    if not (os.path.exists(out) and read_text(out) == text):
        write_text(out, text)
    return out


def grade(iq, suite, answers, each_dir=None):
    """iq score --each on answers (what iq reads of it: readable): {task: {pass, stage, code}},
    cached in each_dir."""
    path = readable(answers, each_dir)
    return {j["task"]: j for j in grade_lines(iq, suite, path, each_path(path, each_dir))}


def carriers(on, fitted):
    """{system: the fitted systems it carries}: on is {system: the systems it is built on}; a
    system carries each fitted one it reaches through them, itself included."""
    out = {}
    for k in on:
        seen, todo = set(), [k]
        while todo:
            x = todo.pop()
            if x not in seen:
                seen.add(x)
                todo.extend(on.get(x, ()))
        out[k] = frozenset(seen & set(fitted))
    return out


def read_tries(path):
    """A try log: {task: its try lines by try index}, a (task, try)'s last line kept (a rerun after
    a kill tries the task again), and the tasks with a line that carries an error. Such a line (a
    context overflow, the last try of its draft) holds the draft and its rank, and is not clean, so
    `cut` never keeps it; it stays for what its turns cost."""
    by, errors = {}, set()
    for r in jl(path):
        if not isinstance(r.get("task"), str) or not isinstance(r.get("reply"), str):
            continue
        if "error" in r:
            errors.add(r["task"])
        by.setdefault(r["task"], {})[int(r.get("try", 0))] = r
    return {t: [v[i] for i in sorted(v)] for t, v in by.items()}, errors


def cut(lines, k, draft_rank=0):
    """What `eval team` keeps of a draft's first k tries (its try lines in order): the first that
    ran clean, else the one of highest rank strictly above the draft's (the earliest of a tie),
    else None: the draft unchanged. The older log, without ranks, kept the first try's."""
    first = lines[:k]
    for r in first:
        if r.get("clean"):
            return r
    if first and "rank" not in first[0]:
        return first[0]
    best = None
    for r in first:
        if r["rank"] > (best["rank"] if best is not None else draft_rank):
            best = r
    return best


def spent(lines, k):
    """The tries a run that stops at its first clean try makes of the first k: those up to it."""
    out = []
    for r in lines[:k]:
        out.append(r)
        if r.get("clean"):
            break
    return out


def mcnemar(b, c):
    n = b + c
    if n == 0:
        return 1.0
    return min(1.0, 2 * sum(math.comb(n, k) for k in range(min(b, c) + 1)) / 2 ** n)


def needs(discordant, alpha):
    """Smallest |b - c| that reaches p < alpha with b + c = discordant (exact McNemar)."""
    for x in range(discordant % 2, discordant + 1, 2):
        if mcnemar((discordant + x) // 2, (discordant - x) // 2) < alpha:
            return x
    return None


def signflip(D, net, reps=200000):
    """Two-sided p of the paired sign-flip test on cluster sums D."""
    nz = [x for x in D if abs(x) > 1e-12]
    if not nz:
        return 1.0
    eps = 1e-9
    if len(nz) <= 20:
        hit = sum(1 for signs in itertools.product((1, -1), repeat=len(nz))
                  if abs(sum(s * x for s, x in zip(signs, nz))) >= abs(net) - eps)
        return hit / 2 ** len(nz)
    rng = random.Random(1)
    hit = sum(1 for _ in range(reps) if abs(sum(x if rng.random() < 0.5 else -x for x in nz)) >= abs(net) - eps)
    return (hit + 1) / (reps + 1)


def family_test(dd, fam, fams):
    """The paired test of per-task differences dd ({task: d}), tasks of one family together:
    (gained, lost, net, se, p)."""
    gained = sum(max(x, 0) for x in dd.values())
    lost = sum(max(-x, 0) for x in dd.values())
    net = gained - lost
    D = {}
    for t in dd:
        D[fam[t]] = D.get(fam[t], 0.0) + dd[t]
    Dv = [D.get(f, 0.0) for f in fams]
    se = math.sqrt(max(0.0, sum(x * x for x in Dv) - net * net / len(fams))) if fams else 0.0
    return gained, lost, net, se, signflip(Dv, net)


def pv(p):
    return "<0.001" if p < 0.001 else "%.3f" % p


def fmt(x):
    return ("%d" % round(x)) if abs(x - round(x)) < 1e-9 else ("%.2f" % x)


def signed(x):
    return "0" if abs(x) < 1e-12 else (("+" if x > 0 else "") + fmt(x))


def main():
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
    h = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    h.add_argument("--dir", required=True)
    h.add_argument("--iq")
    h.add_argument("--each-dir", help="where graded lines and derived answers go (default: --dir)")
    h.add_argument("--out")
    h.add_argument("--json")
    h.add_argument("--vs", action="append", default=[], help="A:B, a decision to report (system B against A)")
    h.add_argument("--same", action="append", default=[], help="RUN1:RUN2, two runs of one setting (replay check)")
    h.add_argument("--sys", action="append", default=[], help="NAME=FILE, an answers file as a system (pre, final)")
    h.add_argument("--in-sample", action="append", default=[],
                   help="NAME, a system fitted on these held tasks (pre): a pair its gain tells apart gets no test")
    h.add_argument("--alpha", type=float, default=0.05)
    h.add_argument("--predictions", help='written before the night: {"system", "lo", "hi"} a line (passes; '
                                         'a line with another "what" is compose.py\'s)')
    a = h.parse_args()
    pred = {r["system"]: r for r in jl(a.predictions) if r.get("what", "pass") == "pass"} if a.predictions else {}
    d = a.dir
    ed = a.each_dir or d
    iq = a.iq or os.path.join(d, "bin", "iq.exe")
    suite_path = os.path.join(d, "iq.jsonl")
    suite = {j["id"]: j for j in jl(suite_path)}
    night = os.path.basename(os.path.normpath(d)).replace("team-", "")

    glm_answers = {r["task"]: r for r in jl(os.path.join(d, "answers-glm.jsonl"))}
    base = grade(iq, suite_path, os.path.join(d, "answers-glm.jsonl"), ed)
    tasks = sorted(base)
    n = len(tasks)
    fam = {t: suite[t]["family"] for t in tasks}
    tier = {t: suite[t]["tier"] for t in tasks}
    fams = sorted(set(fam.values()))
    everyday = [t for t in tasks if tier[t] in EVERYDAY]
    games = [t for t in tasks if tier[t] in GAMES]

    def new_system():
        return {"runs": [], "grades": [], "clean": [], "touched": [], "files": [], "helper_calls": 0.0, "cloud": 0.0,
                "input": "glm"}

    systems = {"glm": new_system()}
    g0 = systems["glm"]
    g0["runs"], g0["grades"], g0["files"], g0["cloud"] = ["glm"], [base], ["answers-glm.jsonl"], float(len(glm_answers))
    g0["clean"], g0["touched"] = [{}], [{}]
    g0["input"] = None
    # Each answers system's replies, and whose answers a model's are: a team run's input is the
    # system its records' lead names.
    answers_of = {"glm": glm_answers}
    model_of = {r.get("model"): "glm" for r in glm_answers.values()}
    for spec in a.sys:
        name, _, path = spec.partition("=")
        path = path if os.path.isabs(path) else os.path.join(d, path)
        ans = {r["task"]: r for r in jl(path)}
        g = grade(iq, suite_path, path, ed)
        if set(g) != set(tasks):
            print("note: --sys %s grades %d of %d tasks: left out" % (name, len(g), n), file=sys.stderr)
            continue
        s = systems.setdefault(name, new_system())
        s["runs"], s["grades"], s["files"], s["cloud"] = [name], [g], [os.path.basename(path)], float(len(ans))
        s["clean"], s["touched"], s["input"] = [{}], [{}], None
        # A merge (compose.py's final) names the system each task's program came from.
        s["from"] = sorted({r["from"] for r in ans.values() if isinstance(r.get("from"), str)} - {name})
        answers_of[name] = ans
        for r in ans.values():
            model_of.setdefault(r.get("model"), name)
    runs = {}  # run name -> (records, grades)

    for path in sorted(glob.glob(os.path.join(d, "team-*.jsonl"))):
        name = os.path.basename(path)[len("team-"):-len(".jsonl")]
        recs = {r["task"]: r for r in jl(path)}
        if not recs or not set(recs) <= set(tasks):
            continue  # the train split, or another suite
        g = grade(iq, suite_path, path, ed)
        if set(g) != set(tasks):
            print("note: %s grades %d of %d tasks (a partial run): left out" % (name, len(g), n), file=sys.stderr)
            continue
        runs[name] = (recs, g)
        m = re.match(r"^(.*)-s(\d+)$", name)
        s = systems.setdefault(m.group(1) if m else name, new_system())
        leads = sorted({r.get("team", {}).get("lead") for r in recs.values()} - {None})
        s["input"] = next((model_of[x] for x in leads if x in model_of), "glm")
        s["runs"].append(name)
        s["grades"].append(g)
        s["files"].append(os.path.basename(path))
        s["clean"].append({t: bool(recs[t].get("team", {}).get("clean")) for t in tasks})
        s["touched"].append({t: recs[t].get("team", {}).get("turns", 0) > 0 or recs[t].get("team", {}).get("tries", 0) > 1 for t in tasks})
        tr = jl(os.path.join(d, "traces-%s.jsonl" % name))
        calls = {(r["task"], r.get("try", 0), r["turn"]) for r in tr if r.get("task") in recs}
        by, _ = read_tries(os.path.join(d, "tries-%s.jsonl" % name))
        by = {t: v for t, v in by.items() if t in recs}
        counted = bool(by) and all("turns" in r for v in by.values() for r in v)
        if counted:
            # A run that stops at its first clean try: what --all-tries made after it is left out.
            s["helper_calls"] += sum(r["turns"] for v in by.values() for r in spent(v, len(v)))
        else:
            s["helper_calls"] += len(calls) if tr else sum(r.get("team", {}).get("turns", 0) for r in recs.values())
        s["cloud"] += sum(1 + r.get("team", {}).get("cloud", 0) for r in recs.values())

        # The same run cut at fewer tries, from its try log.
        if by:
            K = max(len(v) for v in by.values())
            drafts = answers_of.get(s["input"], {})
            for k in range(1, K):
                dname = "%s@t%d" % (name, k)
                out_path = os.path.join(ed, "answers-%s.jsonl" % dname.replace("@", "-at-"))
                lines, cl = [], {}
                for t in tasks:
                    v = by.get(t)
                    team = recs[t].get("team", {})
                    if v:
                        pick = cut(v, k, team.get("draft_rank", 0))
                        if pick is not None:
                            reply, cl[t] = pick["reply"], bool(pick.get("clean"))
                        else:
                            # The draft unchanged: the input's reply (a tried draft does not run,
                            # so an unknown input's is graded as no program).
                            kept = recs[t]["reply"] if team.get("kept_try", 0) == -1 else ""
                            reply, cl[t] = drafts[t]["reply"] if t in drafts else kept, False
                    else:
                        reply, cl[t] = recs[t]["reply"], bool(team.get("clean"))
                    lines.append(json.dumps({"task": t, "model": dname, "reply": reply}, ensure_ascii=False))
                if not os.path.exists(out_path) or os.path.getmtime(out_path) < os.path.getmtime(os.path.join(d, "tries-%s.jsonl" % name)):
                    write_text(out_path, "\n".join(lines) + "\n")
                gd = grade(iq, suite_path, out_path, ed)
                m2 = re.match(r"^(.*)-s(\d+)$", name)
                ds = systems.setdefault("%s@t%d" % (m2.group(1) if m2 else name, k), new_system())
                ds["input"] = s["input"]
                ds["runs"].append(dname)
                ds["grades"].append(gd)
                ds["files"].append(os.path.basename(out_path))
                ds["clean"].append(cl)
                ds["touched"].append(s["touched"][-1])
                if counted:
                    ds["helper_calls"] += sum(r["turns"] for v in by.values() for r in spent(v, k))
                else:
                    ds["helper_calls"] += len({c for c in calls if c[1] < k})
                ds["cloud"] += sum(1 + r.get("team", {}).get("cloud", 0) for r in recs.values())

    for name, s in systems.items():
        R = len(s["runs"])
        s["y"] = {t: sum(g[t]["pass"] for g in s["grades"]) / R for t in tasks}
        s["c"] = {t: sum(c.get(t, False) for c in s["clean"]) / R for t in tasks}
        s["per_run"] = [sum(g[t]["pass"] for t in tasks) for g in s["grades"]]
        s["helper_calls"] /= R
        s["cloud"] /= R
        s["n_touched"] = sum(sum(tc.get(t, False) for t in tasks) for tc in s["touched"]) / R
        s["n_clean"] = sum(sum(1 for t in tasks if tc.get(t) and c.get(t)) for tc, c in zip(s["touched"], s["clean"])) / R
        s["clean_gain"] = sum(sum(1 for t in tasks if c.get(t) and tc.get(t) and g[t]["pass"] and not base[t]["pass"])
                              for g, c, tc in zip(s["grades"], s["clean"], s["touched"])) / R

    def test(dd):
        return family_test(dd, fam, fams)

    # The --in-sample systems each carries: a run its input's, a merge its sources'.
    carried = carriers({k: s.get("from") or ([s["input"]] if s["input"] else []) for k, s in systems.items()},
                       a.in_sample)

    def compare(A, B):
        sa, sb = systems[A], systems[B]
        dd = {t: sb["y"][t] - sa["y"][t] for t in tasks}
        gained, lost, net, se, p = test(dd)
        one = len(sa["runs"]) == 1 and len(sb["runs"]) == 1
        disc = int(round(gained + lost))
        need = needs(disc, a.alpha) if one else None
        verdict = ("better" if net > 0 else "worse") if p < a.alpha and abs(net) > 1e-12 else "no difference shown"
        lo, hi = net - 1.96 * se, net + 1.96 * se
        # Told apart in part by a gain fitted on these tasks: its counts, never a test.
        ins = sorted(carried[A] ^ carried[B])
        if ins:
            p = lo = hi = need = None
            verdict = "in-sample (%s, fitted on these tasks): counts only, no test" % ", ".join(ins)
        ev_lost = sum(max(-dd[t], 0) for t in everyday)
        if ev_lost > 0:
            verdict += "; loses %s everyday" % fmt(ev_lost)
        # the helper's own skill: made clean, on the drafts both took a turn on
        both = [t for t in tasks if all(tc.get(t) for tc in sa["touched"] + sb["touched"])] if A != "glm" else []
        cd = {t: (sb["c"][t] - sa["c"][t]) if t in both else 0.0 for t in tasks}
        cg, cl, cn, _, cp = test(cd)
        grp = {}
        for label, ts in (("everyday", everyday), ("games", games)):
            grp[label] = {"gained": sum(max(dd[t], 0) for t in ts), "lost": sum(max(-dd[t], 0) for t in ts)}
        out = {"a": A, "b": B, "gained": gained, "lost": lost, "net": net, "se": se, "lo": lo,
               "hi": hi, "p": p, "discordant": disc, "needs": need, "single": one, "verdict": verdict,
               "groups": grp, "clean": {"both": len(both), "a": sum(sa["c"][t] for t in both),
                                        "b": sum(sb["c"][t] for t in both), "net": cn, "p": None if ins else cp},
               "gained_tasks": sorted([t for t in tasks if dd[t] > 0], key=lambda t: (-dd[t], t)),
               "lost_tasks": sorted([t for t in tasks if dd[t] < 0], key=lambda t: (dd[t], t)), "d": dd}
        if ins:
            out["in_sample"] = ins
        return out

    P = lambda k: sum(systems[k]["y"].values())
    order = ["glm"] + sorted([k for k in systems if k != "glm"], key=lambda k: (-P(k), k))
    vs_glm = {k: compare("glm", k) for k in order if k != "glm"}
    # Net against each system's own input, for helper calls per extra solve.
    vs_input = {k: vs_glm[k]["net"] if systems[k]["input"] in (None, "glm") else compare(systems[k]["input"], k)["net"]
                for k in order if k != "glm"}
    decisions = []
    for spec in a.vs:
        A, B = spec.split(":")
        missing = [x for x in (A, B) if x not in systems]
        decisions.append(compare(A, B) if not missing else {"a": A, "b": B, "missing": missing})

    def need_s(c):
        if "in_sample" in c:
            return "—"
        if not c["single"]:
            return "(replicates)"
        if c["needs"] is None:
            return "none at %d" % c["discordant"]
        return "±%d" % c["needs"]

    def ci_s(c):
        return "—" if c["lo"] is None else "[%+.1f, %+.1f]" % (c["lo"], c["hi"])

    def p_s(p):
        return "in-sample" if p is None else pv(p)

    L = []
    w = L.append
    w("# Paired report: team-%s, %d held-out tasks" % (night, n))
    w("")
    w("%d families (a family's tasks are tested together); everyday = tiers 1-2 (%d tasks), games = tiers 3-6 (%d). "
      "Every system repairs the same GLM drafts. p: the paired sign-flip test by family (McNemar's exact test "
      "for single runs); a difference is shown at p < %.2f. needs: the net a single run must reach at its "
      "discordance to be shown." % (len(fams), len(everyday), len(games), a.alpha))
    w("")
    w("## The combined system")
    w("")
    w("| system | runs | pass | % | everyday | games | vs GLM | net | 95% CI | p | needs | made clean | clean→gain | helper calls | cloud calls/solve | helper calls/extra solve | predicted |")
    w("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|")
    for k in order:
        s = systems[k]
        p_ = P(k)
        ev = sum(s["y"][t] for t in everyday)
        ga = sum(s["y"][t] for t in games)
        cps = s["cloud"] / p_ if p_ else float("inf")
        R = len(s["runs"])
        runs_s = "1" if R == 1 else "%d (%s)" % (R, " ".join(str(x) for x in s["per_run"]))
        if k == "glm":
            w("| GLM alone | 1 | %s | %.1f | %s/%d | %s/%d | | | | | | | | 0 | %.2f | | |" % (
                fmt(p_), 100 * p_ / n, fmt(ev), len(everyday), fmt(ga), len(games), cps))
            continue
        c = vs_glm[k]
        hpe = ("%.0f" % (s["helper_calls"] / vs_input[k])) if vs_input[k] > 0 else "—"
        pr = pred.get(k)
        pr_s = ("[%s, %s] %s" % (fmt(pr["lo"]), fmt(pr["hi"]), "inside" if pr["lo"] <= p_ <= pr["hi"] else ("above" if p_ > pr["hi"] else "below"))) if pr else ""
        w("| %s | %s | %s | %.1f | %s/%d | %s/%d | +%s −%s | %s | %s | %s | %s | %s/%s | %s | %s | %.2f | %s | %s |" % (
            k, runs_s, fmt(p_), 100 * p_ / n, fmt(ev), len(everyday), fmt(ga), len(games),
            fmt(c["gained"]), fmt(c["lost"]), signed(c["net"]), ci_s(c), p_s(c["p"]), need_s(c),
            fmt(s["n_clean"]), fmt(s["n_touched"]), fmt(s["clean_gain"]), fmt(s["helper_calls"]), cps, hpe, pr_s))
    w("")
    fitted = sorted({x for k in order[1:] for x in vs_glm[k].get("in_sample", [])})
    if fitted:
        w("in-sample: against GLM alone, the nets of %s and of what is built on it hold its gain, fitted on these "
          "held tasks: counts only, never a test (a run on it is tested against it, in Decisions)." % " and ".join(fitted))
        w("")
    own = sorted({systems[k]["input"] for k in order if systems[k]["input"] not in (None, "glm")})
    if own:
        w("Helper calls per extra solve are against each run's own input: " + "; ".join(
            "%s on %s" % (", ".join(k for k in order if systems[k]["input"] == x), x) for x in own) + ".")
        w("")
    if decisions:
        w("## Decisions")
        w("")
        w("| B against A | B | A | +gained −lost | net | 95% CI | p | needs | everyday ± | games ± | made clean B vs A (p) | verdict |")
        w("|---|---|---|---|---|---|---|---|---|---|---|---|")
        for c in decisions:
            if "missing" in c:
                w("| %s against %s | | | | | | | | | | | not run: %s |" % (c["b"], c["a"], ", ".join(c["missing"])))
                continue
            cc = c["clean"]
            cl = ("%s vs %s of %d (%s)" % (fmt(cc["b"]), fmt(cc["a"]), cc["both"], p_s(cc["p"]))) if cc["both"] else ""
            w("| %s against %s | %s | %s | +%s −%s | %s | %s | %s | %s | +%s −%s | +%s −%s | %s | %s |" % (
                c["b"], c["a"], fmt(P(c["b"])), fmt(P(c["a"])), fmt(c["gained"]), fmt(c["lost"]), signed(c["net"]),
                ci_s(c), p_s(c["p"]), need_s(c),
                fmt(c["groups"]["everyday"]["gained"]), fmt(c["groups"]["everyday"]["lost"]),
                fmt(c["groups"]["games"]["gained"]), fmt(c["groups"]["games"]["lost"]), cl, c["verdict"]))
        w("")
    w("## Run-to-run noise")
    w("")
    reps = [k for k in order if len(systems[k]["runs"]) > 1]
    if reps:
        w("| system | runs | passes each run | mean | SD (runs) | tasks that flip | σ from flips | made clean each run |")
        w("|---|---|---|---|---|---|---|---|")
        for k in reps:
            s = systems[k]
            R = len(s["runs"])
            pr = s["per_run"]
            mean = sum(pr) / R
            sd = math.sqrt(sum((x - mean) ** 2 for x in pr) / (R - 1))
            flips = sum(1 for t in tasks if 0 < s["y"][t] < 1)
            sig = math.sqrt(sum(s["y"][t] * (1 - s["y"][t]) for t in tasks) * R / (R - 1))
            cleans = " ".join(str(sum(1 for t in tasks if tc.get(t) and c.get(t))) for tc, c in zip(s["touched"], s["clean"]))
            w("| %s | %d | %s | %.1f | %.2f | %d | %.2f | %s |" % (k, R, " ".join(map(str, pr)), mean, sd, flips, sig, cleans))
    else:
        w("No system has more than one run here: a single run's passes carry the helper's own sampling noise.")
    for spec in a.same:
        A, B = spec.split(":")
        if A not in runs or B not in runs:
            w("")
            w("Replay %s / %s: not run." % (A, B))
            continue
        ta = {(r["task"], r.get("try", 0), r["turn"]): r for r in jl(os.path.join(d, "traces-%s.jsonl" % A))}
        tb = {(r["task"], r.get("try", 0), r["turn"]): r for r in jl(os.path.join(d, "traces-%s.jsonl" % B))}
        low = {}
        for k in ta:
            if k[1] == 0:
                low[k[0]] = min(low.get(k[0], k[2]), k[2])
        firsts = [k for k in ta if k in tb and k[1] == 0 and k[2] == low[k[0]]]
        key = "body" if all("body" in x[k] for x in (ta, tb) for k in firsts) else "asked"
        same_req = [k for k in firsts if ta[k][key] == tb[k][key]]
        same_rep = sum(1 for k in same_req if ta[k]["reply"] == tb[k]["reply"])
        (ra, ga), (rb, gb) = runs[A], runs[B]
        cflip = sum(1 for t in tasks if bool(ra[t].get("team", {}).get("clean")) != bool(rb[t].get("team", {}).get("clean")))
        pflip = sum(1 for t in tasks if ga[t]["pass"] != gb[t]["pass"])
        w("")
        w("Replay %s / %s: %d identical first requests (by %s), %d identical replies (%.0f%%); %d tasks flip clean, %d flip pass; passes %d / %d." % (
            A, B, len(same_req), key, same_rep, 100.0 * same_rep / max(1, len(same_req)), cflip, pflip,
            sum(ga[t]["pass"] for t in tasks), sum(gb[t]["pass"] for t in tasks)))
    w("")
    w("## Cost")
    w("")
    w("| system | cloud calls | per solve | everyday per solve | games per solve | helper calls | per solve | per extra solve |")
    w("|---|---|---|---|---|---|---|---|")
    for k in order:
        s = systems[k]
        ev = sum(s["y"][t] for t in everyday)
        ga = sum(s["y"][t] for t in games)
        ce = sum(1 for t in everyday if t in glm_answers)
        cg = sum(1 for t in games if t in glm_answers)
        net = vs_input[k] if k != "glm" else 0
        w("| %s | %s | %.2f | %.2f | %.2f | %s | %.2f | %s |" % (
            "GLM alone" if k == "glm" else k, fmt(s["cloud"]), s["cloud"] / P(k), ce / ev if ev else 0, cg / ga if ga else 0,
            fmt(s["helper_calls"]), s["helper_calls"] / P(k), ("%.0f" % (s["helper_calls"] / net)) if net > 0 else "—"))
    w("")
    w("## By tier")
    w("")
    w("| system | " + " | ".join("tier %d (%d)" % (x, sum(1 for t in tasks if tier[t] == x)) for x in range(1, 7)) + " |")
    w("|---|" + "---|" * 6)
    for k in order:
        s = systems[k]
        w("| %s | " % ("GLM alone" if k == "glm" else k) + " | ".join(
            fmt(sum(s["y"][t] for t in tasks if tier[t] == x)) for x in range(1, 7)) + " |")
    w("")
    w("## Tasks gained and lost against GLM alone")
    w("")
    for k in order[1:]:
        c = vs_glm[k]
        g_ = ", ".join("%s (t%d%s)" % (t, tier[t], "" if c["d"][t] == 1 else " %.2f" % c["d"][t]) for t in c["gained_tasks"]) or "none"
        l_ = ", ".join("%s (t%d%s)" % (t, tier[t], "" if c["d"][t] == -1 else " %.2f" % c["d"][t]) for t in c["lost_tasks"]) or "none"
        w("- **%s**: gained %s; lost %s." % (k, g_, l_))
    text = "\n".join(L) + "\n"
    print(text)
    if a.out:
        write_text(a.out, text)
    if a.json:
        dump = {"night": night, "tasks": n, "families": len(fams),
                "systems": {k: {"runs": v["runs"], "per_run": v["per_run"], "pass": P(k),
                                "everyday": sum(v["y"][t] for t in everyday), "games": sum(v["y"][t] for t in games),
                                "touched": v["n_touched"], "made_clean": v["n_clean"], "clean_gain": v["clean_gain"],
                                "helper_calls": v["helper_calls"], "cloud_calls": v["cloud"],
                                "cloud_per_solve": v["cloud"] / P(k) if P(k) else None}
                            for k, v in systems.items()},
                "vs_glm": {k: {x: c[x] for x in c if x != "d"} for k, c in vs_glm.items()},
                "decisions": [{x: c[x] for x in c if x != "d"} for c in decisions]}
        write_text(a.json, json.dumps(dump, indent=1) + "\n")


if __name__ == "__main__":
    main()
