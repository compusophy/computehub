#!/usr/bin/env python
"""The night's headline, composed offline from stages that each ran on the same drafts, and the
decisions written before the night, judged (train/paired.py sets the runs side by side task by
task; this reads them as one system).

  python train/compose.py --dir T [--iq IQ.exe] [--suite S] [--suite-wait W]
                          [--order pre,b3,b7,fin7,fin7r,q3rec] [--out answers-final.jsonl]
                          [--report report.md] [--json report.json] [--decisions decisions.json]
                          [--predictions predictions.jsonl] [--equal b05,b3,b3w,b7] [--equal-k 4]
                          [--dev dev05@1,dev3@1,dev3w@1,dev7@1,dev3@2] [--cascade b3,b7]
                          [--finish fin7,fin7r] [--rec q3rec] [--each-dir DIR]

Paths are in --dir (team-<night>) unless absolute; --iq defaults to its bin/iq.exe, --suite to
its iq.jsonl, --suite-wait to its suite-wait.jsonl (every check "runs clean only": a program
passes it when it runs clean, and the hidden check is never read).

Composition. The first --order name is the base (pre: GLM's drafts read and mended by the harness).
final[t] = the base's program if it runs clean under suite-wait; else the first source in --order
whose program for t runs clean (team-NAME.jsonl, a team run's out records: b3, b7, fin7r;
answers-NAME.jsonl: fin7's pick; NAME.jsonl: q3rec, the recorded samples, the first clean in file
order); else the base's. Each stage ran on the same drafts and none repairs another's failed edits;
the merge is judged only by "runs clean", so it never lowers a grade (a program that does not run
fails every check). answers-final.jsonl records "from" on each task, and is graded with the real
suite into each-final.txt (a line a task, then iq's summary) and score-final.txt (the summary).

Curves, per tries-NAME.jsonl (`eval team --tries-out`, a line a try: task, try, temp, seed, clean,
rank, before, after, turns, held, kept, ms, in, out, reply; a line with "error" is a context
overflow, the draft at its own rank). A draft is one with a try that took a turn. Every try's
program is graded once with the real suite. At k = 1, 2, 4, 8 (those under the run's K, and K):
made clean (the try eval keeps of the first k ran clean: paired.cut), passes with the first clean
try, the oracle (any clean try of the first k passes: a ceiling, chosen by the hidden check, never
a system), the per-draft per-sample clean and pass rates, the run's passes (eval's choice, else the
draft), extra solves (those passes less the drafts' own), and helper calls, tokens and GPU-seconds
(the ms of the tries a run stopping at its first clean try makes, over the helper's slots: 8 for a
7B, else 16) per extra solve; the same by the draft's fault code; and each Held kind's share of the
turns.

Equal tries: the --equal runs cut at --equal-k on the base's residual (its drafts with a program
that does not run clean), passes and made clean, with the paired sign-flip test by family
(paired.py's) for --equal-pairs. Dev: the --dev arms (RUN@k) on answers-dev.jsonl's mutants of
train roots (known answers, real checks): pass and clean overall and by kind group (reasoning:
bound1 size1 size guard state fn decl builtin; pattern: name clear toplevel letstate ret cast
shadow), the same test for --dev-pairs. Cascade: per residual draft, the first --cascade run's
tries until clean (at most --equal-k), else all of them and then the second's until clean; passes
and GPU-seconds per extra solve for each alone and both in turn. Finish: the pick (fin7) and its
repair (fin7r) on the finish tasks (prompts-fin.jsonl), clean and pass, beside the recorded
samples (q3rec). Judges: judge.py's judgefit-NAME.json, its set pre ({"sets": {"pre": {"auc":
{"judge": A, "length": A, ...}, "lift": A}}}; also read flat, {"pre": {"judge", "length",
"lift"}}), an A {"auc", "lo", "hi"}, {"auc", "ci": [lo, hi]} or a number. Replicates: team runs
NAME-sK beside NAME. A run without a try log (an older eval) is read from its out records alone:
what it cost is unknown. A team run that did not finish (no record for some task of its input,
or, its input not here, no team.sh mark that it did: a step left for the morning, or one the
wrap-up stopped) is left out of all of it, the composition too, and named; a rule that rests on
it is not decided.

The report heads with glm | read | the base (the harness, in-sample) | base+q3rec | final, and the
cloud calls per solve (one a task: tasks / passes); then the harness and the models apart (the
models, final against the base, paired by family; the harness, fitted on held, gets no p-value);
then the tables; then predictions.jsonl's ranges beside what came ({"system", "lo", "hi"} a line,
with "what": pass (default), made_clean, pass_pct, fin_clean, fin_pass or auc); then every
decisions.json rule ({"rules": [{"id": "D1", "name", "rule", ...its thresholds}]}) with its verdict.
"""
import argparse
import glob
import json
import math
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from paired import (cut, each_path, family_test, fmt, grade_lines, jl, needs, pv, read_text, read_tries,  # noqa: E402
                    signed, spent, write_text)

KS = (1, 2, 4, 8)
REASONING = ("bound1", "size1", "size", "guard", "state", "fn", "decl", "builtin")
PATTERN = ("name", "clear", "toplevel", "letstate", "ret", "cast", "shadow")
HELD = ("Program", "Edits", "Diff", "Inline", "Missed", "Nothing")
CEILING = "ceiling: chosen by the hidden check, never a system"
# Measured by day (2026-10-09), before the night: the reader alone (last clean block, salvage),
# and the harness out of sample.
READ_BY_DAY = 157
HARNESS = "harness, fitted on held; out of sample on 52 fresh GLM drafts, 4 of 13 made clean and 1 passed"
COSTS = ("calls", "tokens", "gpu_s", "calls_per_extra", "tokens_per_extra", "gpu_s_per_extra")


def write_if_changed(path, text):
    """Write text (LF) unless the file holds it already: an unchanged file keeps its grades."""
    if not (os.path.exists(path) and read_text(path) == text):
        write_text(path, text)


def slots(helper):
    """Requests at once on the helper's server (team.sh's slots): 8 for a 7B, else 16."""
    return 8 if "7b" in (helper or "").lower() else 16


def source(d, name):
    """A stage's file: its team out records, else its answers, else NAME.jsonl; None if not run."""
    for pattern in ("team-%s.jsonl", "answers-%s.jsonl", "%s.jsonl"):
        path = os.path.join(d, pattern % name)
        if os.path.exists(path):
            return path
    return None


class Grader:
    """iq score --each through a derived file a source (graded-<name>.jsonl in each_dir: its
    distinct (task, reply) pairs, rewritten only when they change), under the real suite or
    suite-wait: a program is graded once a file, and again only when the file changes."""

    def __init__(self, iq, suite, wait, each_dir):
        self.iq, self.suite, self.wait, self.each_dir, self.memo = iq, suite, wait, each_dir, {}

    def grades(self, path, wait=False):
        """{(task, reply): grade} for the lines of path that carry a reply."""
        if (path, wait) in self.memo:
            return self.memo[(path, wait)]
        pairs, seen = [], set()
        for r in jl(path):
            if isinstance(r.get("reply"), str) and isinstance(r.get("task"), str):
                key = (r["task"], r["reply"])
                if key not in seen:
                    seen.add(key)
                    pairs.append(key)
        name = os.path.basename(path)[:-len(".jsonl")]
        derived = os.path.join(self.each_dir, "graded-%s.jsonl" % name)
        write_if_changed(derived, "".join(json.dumps({"task": t, "model": name, "reply": r}, ensure_ascii=False) + "\n"
                                          for t, r in pairs))
        out = {}
        if pairs:
            each = each_path(derived, self.each_dir, "each-wait-" if wait else "each-")
            g = grade_lines(self.iq, self.wait if wait else self.suite, derived, each)
            if len(g) != len(pairs):
                sys.exit("compose: %s graded %d of %d programs" % (derived, len(g), len(pairs)))
            out = dict(zip(pairs, g))
        self.memo[(path, wait)] = out
        return out


class Inputs:
    """The drafts a team run repaired: the answers whose model is its records' lead."""

    def __init__(self, d, grader):
        self.d, self.grader, self.models = d, grader, None

    def paths(self, leads):
        """The answers files that hold those leads' drafts."""
        if self.models is None:
            self.models = {}
            for path in sorted(glob.glob(os.path.join(self.d, "answers-*.jsonl"))):
                # Not paired.py's cuts, nor a finish's samples (its model's drafts are the pick).
                base = os.path.basename(path)
                if "-at-t" not in base and not base.endswith("-samples.jsonl"):
                    for r in jl(path):
                        self.models.setdefault(r.get("model"), path)
        return sorted({self.models[x] for x in leads if x in self.models})

    def tasks(self, leads):
        """The tasks of the answers that hold those leads' drafts."""
        return {r["task"] for path in self.paths(leads) for r in jl(path) if isinstance(r.get("task"), str)}

    def passes(self, leads):
        """{task: the draft's real pass} for those leads' answers."""
        out = {}
        for path in self.paths(leads):
            g = self.grader.grades(path)
            for r in jl(path):
                key = (r.get("task"), r.get("reply"))
                if r.get("model") in leads and key[0] not in out and key in g:
                    out[key[0]] = bool(g[key]["pass"])
        return out


def unfinished(d, name, inputs, suite):
    """How much of a team run was written, if it did not finish; None if it did. Finished: a record
    is here for every task of its input (the answers its records' lead names, those in the suite:
    what team.sh asks of a run before it scores one); when its input is not here, team.sh's marks
    (score-NAME.txt and each-NAME.txt, written only then). A step that failed twice and was left,
    or one the wrap-up stopped, did not finish: its drafts never reached would read as tried and
    failed, at no cost, so the run is left out of every table and verdict (as paired.py leaves it)."""
    recs = [r for r in jl(os.path.join(d, "team-%s.jsonl" % name)) if isinstance(r.get("task"), str)]
    leads = {r["team"].get("lead") for r in recs if isinstance(r.get("team"), dict)} - {None}
    want = inputs.tasks(leads) & set(suite)
    if want:
        got = {r["task"] for r in recs} & want
        return None if got == want else "%d of %d records written" % (len(got), len(want))
    if all(os.path.exists(os.path.join(d, "%s-%s.txt" % (x, name))) for x in ("score", "each")):
        return None
    return "%d records written, its drafts not here" % len({r["task"] for r in recs})


def load_run(d, name, grader, inputs):
    """A team run: its out records, their real passes, and its drafts (a task with a try that took a
    turn: its tries in order, each with its real pass; the draft's rank, pass and fault code). None
    if it did not run."""
    team, tries = os.path.join(d, "team-%s.jsonl" % name), os.path.join(d, "tries-%s.jsonl" % name)
    if not os.path.exists(team) and not os.path.exists(tries):
        return None
    recs = {r["task"]: r for r in jl(team) if "task" in r}
    rg = grader.grades(team) if recs else {}
    rec_pass = {t: bool(rg[(t, r["reply"])]["pass"]) for t, r in recs.items() if (t, r.get("reply")) in rg}
    by, errors = read_tries(tries)
    tg = grader.grades(tries) if by else {}
    helper = next((r["team"]["helper"] for r in recs.values() if r.get("team", {}).get("helper")), name)
    leads = {r.get("team", {}).get("lead") for r in recs.values()} - {None}
    draft_pass = inputs.passes(leads) if leads else {}
    drafts = {}
    for t, lines in by.items():
        if not any(r.get("turns", 1) > 0 for r in lines):
            continue  # it ran clean already, or overflowed at once: no turn was taken
        for r in lines:
            r["_pass"] = bool(tg.get((t, r["reply"]), {}).get("pass"))
        team_ = recs.get(t, {}).get("team", {})
        drafts[t] = {"lines": lines, "draft_rank": team_.get("draft_rank", 0), "draft_pass": draft_pass.get(t, False),
                     "code": int(lines[0].get("before", team_.get("before", 0)) or 0)}
    return {"name": name, "recs": recs, "rec_pass": rec_pass, "drafts": drafts,
            "errors": sorted(e for e in errors if e), "helper": helper, "slots": slots(helper),
            "K": max([len(v["lines"]) for v in drafts.values()] + [0])}


def draft_at(lines, k, draft_rank=0, draft_pass=False):
    """A draft's first k tries, as eval keeps them (paired.cut) and as a run stopping at its first
    clean try spends them (paired.spent)."""
    first = lines[:k]
    pick = cut(lines, k, draft_rank)
    sp = spent(lines, k)
    clean = bool(pick is not None and pick.get("clean"))
    return {"tried": True, "picked": pick is not None, "clean": clean,
            "first": clean and pick["_pass"],
            "oracle": any(r.get("clean") and r["_pass"] for r in first),
            "pass": pick["_pass"] if pick is not None else draft_pass, "draft_pass": draft_pass,
            "sample_clean": sum(1 for r in first if r.get("clean")) / len(first) if first else 0.0,
            "sample_pass": sum(1 for r in first if r["_pass"]) / len(first) if first else 0.0,
            "calls": sum(r.get("turns", 0) for r in sp), "tokens": sum(r.get("in", 0) + r.get("out", 0) for r in sp),
            "ms": sum(r.get("ms", 0) for r in sp)}


def untried(run, t, default_pass=False):
    """A task the run took no turn on (skipped, overflowed, clean already): as its record says."""
    rec = run["recs"].get(t, {}) if run else {}
    return {"tried": False, "picked": False, "clean": bool(rec.get("team", {}).get("clean")), "first": False,
            "oracle": False, "pass": run["rec_pass"].get(t, default_pass) if run else default_pass,
            "draft_pass": default_pass, "sample_clean": 0.0, "sample_pass": 0.0, "calls": 0, "tokens": 0, "ms": 0}


def arm(run, tasks, k, default_pass):
    """Per task of `tasks`, the run cut at k tries."""
    out = {}
    for t in tasks:
        dr = run["drafts"].get(t) if run else None
        if dr:
            out[t] = draft_at(dr["lines"], k, dr["draft_rank"], dr["draft_pass"])
        else:
            out[t] = untried(run, t, default_pass.get(t, False))
    return out


def per(x, extra):
    return x / extra if extra > 0 else None


def tally(results, k, n_slots):
    """The sums of draft_at's results over drafts at k."""
    rs = list(results.values())
    n = len(rs)
    row = {"k": k, "drafts": n}
    for key in ("clean", "first", "oracle", "pass", "draft_pass", "calls", "tokens", "ms"):
        row[key] = sum(r[key] for r in rs)
    row["sample_clean"] = sum(r["sample_clean"] for r in rs) / n if n else 0.0
    row["sample_pass"] = sum(r["sample_pass"] for r in rs) / n if n else 0.0
    row["extra"] = row["pass"] - row["draft_pass"]
    row["gpu_s"] = row["ms"] / 1000.0 / n_slots
    for key in ("calls", "tokens", "gpu_s"):
        row[key + "_per_extra"] = per(row[key], row["extra"])
    return row


def ks_for(K):
    return [k for k in KS if k < K] + [K] if K else []


def held_shares(drafts):
    """Each Held kind's share of the turns of every try."""
    count, turns = {}, 0
    for dr in drafts.values():
        for r in dr["lines"]:
            held = r.get("held", "")
            for kind in (held.split() if isinstance(held, str) else list(held)):
                count[kind] = count.get(kind, 0) + 1
                turns += 1
    return {"turns": turns, "share": {kind: count.get(kind, 0) / turns if turns else 0.0
                                      for kind in list(HELD) + sorted(set(count) - set(HELD))}}


def curve(run):
    """A run's curve: a row per k, the same by the draft's fault code, the Held shares."""
    drafts = run["drafts"]
    ks = ks_for(run["K"])

    def rows(sub):
        return [tally({t: draft_at(dr["lines"], k, dr["draft_rank"], dr["draft_pass"]) for t, dr in sub.items()},
                      k, run["slots"]) for k in ks]
    total = []
    for row in rows(drafts):
        # The run's passes on every task had it stopped at k: eval's choice on the drafts, else the record.
        row["run_pass"] = row["pass"] + sum(p for t, p in run["rec_pass"].items() if t not in drafts)
        total.append(row)
    codes = {}
    for code in sorted({dr["code"] for dr in drafts.values()}):
        codes[str(code)] = rows({t: dr for t, dr in drafts.items() if dr["code"] == code})
    clean_out = sum(1 for t, r in run["recs"].items() if t in drafts and r.get("team", {}).get("clean"))
    return {"helper": run["helper"], "slots": run["slots"], "K": run["K"], "drafts": len(drafts),
            "errors": len(run["errors"]), "rows": total, "codes": codes, "held": held_shares(drafts),
            "records_clean": clean_out}


def pair(A, B, ra, rb, tasks, fam, alpha, key="pass"):
    """B against A on tasks, paired by family: paired.py's test, and its needs at this discordance."""
    fams = sorted({fam[t] for t in tasks})
    dd = {t: float(rb[t][key]) - float(ra[t][key]) for t in tasks}
    gained, lost, net, se, p = family_test(dd, fam, fams)
    disc = int(round(gained + lost))
    shown = p < alpha and abs(net) > 1e-12
    return {"a": A, "b": B, "what": key, "gained": gained, "lost": lost, "net": net, "lo": net - 1.96 * se,
            "hi": net + 1.96 * se, "p": p, "needs": needs(disc, alpha), "discordant": disc,
            "verdict": ("better" if net > 0 else "worse") if shown else "no difference shown"}


def compose(sources):
    """sources: [(name, [(task, reply, runs_clean)] in file order)], the base first. {task: (from,
    reply, base_ran_clean)}: the base's if it ran clean, else the first source's first clean, else
    the base's."""
    base_name, base_lines = sources[0]
    first = []
    for name, lines in sources[1:]:
        f = {}
        for t, reply, clean in lines:
            if clean and t not in f:
                f[t] = reply
        first.append((name, f))
    out = {}
    for t, reply, clean in base_lines:
        if t in out:
            continue
        if clean:
            out[t] = (base_name, reply, True)
            continue
        hit = next(((name, f[t]) for name, f in first if t in f), None)
        out[t] = (hit[0], hit[1], False) if hit else (base_name, reply, False)
    return out


def auc_of(x):
    """An AUC entry as (value, lo, hi); None if it is not one."""
    if isinstance(x, (int, float)) and not isinstance(x, bool):
        return (float(x), None, None)
    if isinstance(x, dict):
        v = next((x[k] for k in ("auc", "value", "mean", "lift") if isinstance(x.get(k), (int, float))), None)
        if v is None:
            return None
        lo, hi = x.get("lo"), x.get("hi")
        if isinstance(x.get("ci"), list) and len(x["ci"]) == 2:
            lo, hi = x["ci"]
        return (float(v), lo, hi)
    return None


def judges(d):
    """judgefit-NAME.json's AUCs on pre's clean programs: {NAME: {"judge", "length", "lift"}} (each
    (value, lo, hi) or None), and the files not in a known shape."""
    out, odd = {}, []
    for path in sorted(glob.glob(os.path.join(d, "judgefit-*.json"))):
        name = os.path.basename(path)[len("judgefit-"):-len(".json")]
        try:
            fit = json.loads(read_text(path))
        except ValueError:
            odd.append(name)
            continue
        s = ((fit.get("sets") or {}).get("pre") or fit.get("pre")) if isinstance(fit, dict) else None
        s = s if isinstance(s, dict) else {}
        # judge.py's: {"auc": {"judge": .., "length": .., ...}, "lift": ..}
        au = s["auc"] if isinstance(s.get("auc"), dict) else s
        e = {"judge": auc_of(au.get("judge")), "length": auc_of(au.get("length")), "lift": auc_of(s.get("lift"))}
        if e["judge"] is None or e["length"] is None:
            odd.append(name)
        out[name] = e
    return out, odd


def replicates(d, grader, names, tasks, partial=()):
    """Team runs NAME and NAME-sK, those that finished: the passes of each, their SD and sigma from
    flips (paired.py's)."""
    out = {}
    for name in names:
        paths = [p for p in sorted(glob.glob(os.path.join(d, "team-%s*.jsonl" % name)))
                 if re.match(r"^team-%s(-s\d+)?\.jsonl$" % re.escape(name), os.path.basename(p))
                 and os.path.basename(p)[len("team-"):-len(".jsonl")] not in partial]
        per_run, ys = [], {t: 0.0 for t in tasks}
        for p in paths:
            g = grader.grades(p)
            recs = {r["task"]: r for r in jl(p) if "task" in r}
            if not set(tasks) <= set(recs):
                continue  # a partial run
            ok = {t: bool(g[(t, recs[t]["reply"])]["pass"]) for t in tasks}
            per_run.append(sum(ok.values()))
            for t in tasks:
                ys[t] += ok[t]
        R = len(per_run)
        if R == 0:
            continue
        e = {"runs": R, "per_run": per_run}
        if R > 1:
            mean = sum(per_run) / R
            y = {t: ys[t] / R for t in tasks}
            e["sd"] = math.sqrt(sum((x - mean) ** 2 for x in per_run) / (R - 1))
            e["sigma"] = math.sqrt(sum(v * (1 - v) for v in y.values()) * R / (R - 1))
        out[name] = e
    return out


def reads(rule, m):
    """The team runs a rule's verdict rests on (with its function's defaults)."""
    rid = rule.get("id", "")
    if rid == "D1":
        return [rule.get(k, v).partition("@")[0] for k, v in (
            ("small", "dev3@1"), ("large", "dev7@1"), ("whole", "dev3w@1"), ("res_small", "b3"), ("res_large", "b7"))]
    if rid == "D2":
        return list(m.get("equal", {}).get("names", [])) + list(m.get("cascade_runs", []))
    if rid == "D3":
        return [rule.get("run", "b3"), rule.get("dev_run", "dev3")]
    if rid == "D6":
        return [rule.get("run", "b3raw")]
    return []


def verdicts(rules, m):
    """Each decisions.json rule with its verdict on the measures m: {"id", "name", "rule", "fires"
    (True, False, or None: not decided), "verdict"}. A rule that rests on a run that did not finish
    is not decided."""
    out = []
    for rule in rules:
        rid = rule.get("id", "")
        f = DECIDE.get(rid)
        left = sorted({x for x in reads(rule, m) if x in m.get("partial", {})})
        try:
            if left:
                fires, verdict = None, "not decided: %s did not finish (%s)" % (
                    ", ".join(left), "; ".join("%s: %s" % (x, m["partial"][x]) for x in left))
            else:
                fires, verdict = f(rule, m) if f else (None, "no rule here to judge it: by hand")
        except KeyError as e:
            fires, verdict = None, "not decided: %s did not run" % e.args[0]
        out.append({"id": rid, "name": rule.get("name", ""), "rule": rule.get("rule", ""), "fires": fires,
                    "verdict": verdict})
    return out


def need(table, key, what):
    if key not in table:
        raise KeyError(what)
    return table[key]


def d1(rule, m):
    dev, eq = m.get("dev", {}), m.get("equal", {})
    small, large, whole = rule.get("small", "dev3@1"), rule.get("large", "dev7@1"), rule.get("whole", "dev3w@1")
    s = need(dev.get("arms", {}), small, small)["pass_pct"]
    lg = need(dev.get("arms", {}), large, large)["pass_pct"]
    gap = lg - s
    p = need(dev.get("pairs", {}), "%s:%s" % (small, large), "the pair %s:%s" % (small, large))["p"]
    rs, rl = rule.get("res_small", "b3"), rule.get("res_large", "b7")
    res = need(eq.get("pairs", {}), "%s:%s" % (rs, rl), "the residual pair %s:%s" % (rs, rl))
    nums = "%s %.1f%% vs %s %.1f%% on dev: gap %+.1f points, p %s; residual %s against %s at %d tries: net %s" % (
        large, lg, small, s, gap, pv(p), rl, rs, eq.get("k", 0), signed(res["net"]))
    if gap >= rule.get("gap", 10) and p < rule.get("alpha", 0.05) and res["net"] >= 0:
        v = ("fires (%s): the 7B becomes the escalation tier (3B first, 7B on what it leaves); "
             "ask compusophy to OK a 30B-A3B test" % nums)
        w = dev["arms"].get(whole)
        if w is not None:
            closed = (w["pass_pct"] - s) / gap
            v += "; %s closes %.0f%% of the gap: %s" % (
                whole, 100 * closed, "reply format, not size: whole-program replies become the 3B's default"
                if closed >= rule.get("half", 0.5) else "the gap is size")
        return True, v
    if gap < rule.get("flat", 5):
        return False, "size is not the lever (%s): richer fault accounts come next" % nums
    return False, "neither branch (%s): no change" % nums


def d2(rule, m):
    cands = dict(need(m, "equal", "the equal-tries runs")["arms"])
    casc = m.get("cascade", {}).get("rows", {})
    both = next((r for r in casc.values() if r.get("both")), None)
    if both:
        cands[both["label"]] = both
    if not cands:
        raise KeyError("the equal-tries runs")
    best = max(c["pass"] for c in cands.values())
    within = {k: c for k, c in cands.items() if c["pass"] >= best - rule.get("within", 1)}
    inf = float("inf")
    cost = lambda k: (inf if within[k]["gpu_s_per_extra"] is None else within[k]["gpu_s_per_extra"],
                      inf if within[k]["gpu_s"] is None else within[k]["gpu_s"], k)
    pick = min(within, key=cost)
    c = within[pick]
    per_s = ("its cost unknown (no try log)" if c["gpu_s"] is None else "no extra solve"
             if c["gpu_s_per_extra"] is None else "%s GPU-s per extra solve" % num(c["gpu_s_per_extra"]))
    return True, "default helper: %s (%s passes on the residual, %s; the best %s; within %s: %s)" % (
        pick, fmt(c["pass"]), per_s, fmt(best), rule.get("within", 1), ", ".join(sorted(within)))


def knee(rows, per100):
    """The k past which another step makes fewer than per100 clean per 100 helper calls."""
    for a, b in zip(rows, rows[1:]):
        calls = b["calls"] - a["calls"]
        rate = 100.0 * (b["clean"] - a["clean"]) / calls if calls > 0 else 0.0
        if rate < per100:
            return a["k"], rate
    return rows[-1]["k"], None


def d3(rule, m):
    curves = m.get("curves", {})
    parts = []
    for name in rule.get("runs", ["b3", "b7", "b05", "b3w"]):
        c = curves.get(name)
        if c and c["rows"]:
            k, rate = knee(c["rows"], rule.get("knee", 1))
            parts.append("%s at %d%s" % (name, k, (" (next step %.1f per 100 calls)" % rate)
                                         if rate is not None else " (every step pays)"))
        elif name in m.get("partial", {}):
            parts.append("%s did not finish" % name)
    run = need(curves, rule.get("run", "b3"), rule.get("run", "b3"))["rows"][-1]
    gap = run["oracle"] - run["first"]
    dev_s, dev_gap = "", None
    dc = curves.get(rule.get("dev_run", "dev3"))
    if dc and dc["rows"]:
        r2 = next((r for r in dc["rows"] if r["k"] == rule.get("dev_k", 2)), dc["rows"][-1])
        dev_gap = 100.0 * (r2["oracle"] - r2["first"]) / r2["drafts"] if r2["drafts"] else 0.0
        dev_s = "; %s@%d: oracle − first clean %.1f points" % (rule.get("dev_run", "dev3"), r2["k"], dev_gap)
    sel = gap >= rule.get("oracle_gap", 3) or (dev_gap is not None and dev_gap >= rule.get("oracle_gap_dev", 8))
    return sel, "stop adding tries at the knee: %s. %s@%d: oracle − first clean = %s%s: selection among repairs %s" % (
        "; ".join(parts) or "no curve", rule.get("run", "b3"), run["k"], fmt(gap), dev_s,
        "is worth building" if sel else "is not worth building")


def d4(rule, m):
    js = {k: v for k, v in m.get("judges", {}).items() if v.get("judge") and v.get("length")}
    if not js:
        raise KeyError("a judge (judgefit-*.json with pre's AUCs)")
    q = lambda x: "?" if x is None else "%.3f" % x
    lines, fire = [], False
    for name, e in sorted(js.items()):
        j, ln, lift = e["judge"], e["length"], e.get("lift")
        above = j[1] is not None and j[1] > ln[0]
        lifted = lift is not None and lift[0] >= rule.get("lift", 0.03) and lift[1] is not None and lift[1] > 0
        fire = fire or above or lifted
        lines.append("%s %.3f [%s, %s] vs length %.3f%s" % (
            name, j[0], q(j[1]), q(j[2]), ln[0],
            ("; lift %+.3f [%s, %s]" % (lift[0], q(lift[1]), q(lift[2]))) if lift else ""))
    if fire:
        return True, "build judge-gated selection and escalation (%s)" % "; ".join(lines)
    return False, ("stop the local-verifier line (judge and checks:chk); attack the 114 with ask-derived probe "
                   "rules and a review turn (%s)" % "; ".join(lines))


def d5(rule, m):
    fin = need(m, "finish", "the finish")
    e = need(fin["rows"], rule.get("run", "fin7"), rule.get("run", "fin7"))
    ok = e["pass"] >= rule.get("pass", 3) and e["clean"] >= rule.get("clean", 10)
    nums = "%s: %d of %d pass, %d run clean" % (rule.get("run", "fin7"), e["pass"], fin["tasks"], e["clean"])
    if ok:
        return True, "put 'finish from the plan' into Studio's make loop in place of the rush re-ask (%s)" % nums
    return False, "keep the rush re-ask (%s)" % nums


def d6(rule, m):
    run = rule.get("run", "b3raw")
    raw = need(m.get("runs", {}), run, run)["pass"]
    base = m["header"]["base_pass"]
    nums = "%s %d against %s %d" % (run, raw, m["header"]["base"], base)
    if raw >= base - rule.get("margin", 3):
        return True, "tries alone reach mend (%s)" % nums
    return False, ("ship pre's reader parts (last clean block, salvage) to Studio now, and mend after an "
                   "out-of-sample check (%s)" % nums)


def d7(rule, m):
    reps = {k: v for k, v in m.get("replicates", {}).items() if v["runs"] > 1 and k in rule.get("runs", ["b3", "b7"])}
    if not reps:
        raise KeyError("a replicate (NAME-sK)")
    return True, "no difference is claimed below paired.py's needs bar (single runs) or 2σ: %s" % "; ".join(
        "%s σ %.2f over %d runs (%s)" % (k, v["sigma"], v["runs"], " ".join(map(str, v["per_run"])))
        for k, v in sorted(reps.items()))


DECIDE = {"D1": d1, "D2": d2, "D3": d3, "D4": d4, "D5": d5, "D6": d6, "D7": d7}


def measured(line, m):
    """What came for a prediction line, or None."""
    what, s = line.get("what", "pass"), line.get("system")
    h = m.get("header", {})
    try:
        if what == "pass":
            if s == "final":
                return h.get("final")
            if s == h.get("base"):
                return h.get("base_pass")
            if s in ("glm", "read", "rec"):
                return h.get(s)
            if s == "%s+%s" % (h.get("base"), h.get("rec_name")):
                return h.get("rec")
            return m["runs"][s]["pass"]
        if what == "made_clean":
            rows = m["curves"][s]["rows"]
            return next((r["clean"] for r in rows if r["k"] == line.get("k")), rows[-1]["clean"])
        if what == "pass_pct":
            return m["dev"]["arms"][s]["pass_pct"]
        if what in ("fin_clean", "fin_pass"):
            return m["finish"]["rows"][s][what[4:]]
        if what == "auc":
            name, _, part = s.partition("-")
            js = m.get("judges", {})
            if name == "length":
                e = next((v["length"] for v in js.values() if v.get("length")), None)
            else:
                e = js[name]["lift" if part == "lift" else "judge"]
            return e[0] if e else None
    except (KeyError, IndexError, TypeError):
        return None
    return None


def main():
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
    h = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    h.add_argument("--dir", required=True)
    h.add_argument("--iq")
    h.add_argument("--suite")
    h.add_argument("--suite-wait")
    h.add_argument("--each-dir", help="where derived answers and grades go (default: --dir)")
    h.add_argument("--order", default="pre,b3,b7,fin7,fin7r,q3rec")
    h.add_argument("--out", default="answers-final.jsonl")
    h.add_argument("--report", default="report.md")
    h.add_argument("--json", default="report.json")
    h.add_argument("--decisions", default="decisions.json")
    h.add_argument("--predictions", default="predictions.jsonl")
    h.add_argument("--rec", default="q3rec", help="the recorded samples: the header's base+REC")
    h.add_argument("--equal", default="b05,b3,b3w,b7")
    h.add_argument("--equal-k", type=int, default=4)
    h.add_argument("--equal-pairs", default="b3:b7,b3:b3w,b3:b05,b3w:b7", help="A:B, B against A")
    h.add_argument("--dev", default="dev05@1,dev3@1,dev3w@1,dev7@1,dev3@2")
    h.add_argument("--dev-pairs", default="dev3@1:dev7@1,dev3@1:dev3w@1,dev3@1:dev05@1,dev3@1:dev3@2,dev3w@1:dev7@1")
    h.add_argument("--dev-answers", default="answers-dev.jsonl")
    h.add_argument("--cascade", default="b3,b7")
    h.add_argument("--finish", default="fin7,fin7r", help="the pick and its repair")
    h.add_argument("--replicates", default="b3,b7,b05,b3w")
    h.add_argument("--alpha", type=float, default=0.05)
    a = h.parse_args()
    d = a.dir
    at = lambda p: p if os.path.isabs(p) else os.path.join(d, p)
    ed = a.each_dir or d
    os.makedirs(ed, exist_ok=True)
    iq = a.iq or os.path.join(d, "bin", "iq.exe")
    suite_path, wait_path = at(a.suite or "iq.jsonl"), at(a.suite_wait or "suite-wait.jsonl")
    for p in (iq, suite_path, wait_path):
        if not os.path.exists(p):
            sys.exit("compose: no %s" % p)
    suite = {j["id"]: j for j in jl(suite_path)}
    G = Grader(iq, suite_path, wait_path, ed)
    inputs = Inputs(d, G)
    night = os.path.basename(os.path.abspath(d)).replace("team-", "")

    # The team runs here (not the train split's); those that did not finish are left out of all
    # that follows, the composition too, and named in the report.
    stem = lambda p, pre: os.path.basename(p)[len(pre):-len(".jsonl")]
    names = sorted(x for x in {stem(p, "tries-") for p in glob.glob(os.path.join(d, "tries-*.jsonl"))} |
                   {stem(p, "team-") for p in glob.glob(os.path.join(d, "team-*.jsonl"))} if not x.startswith("train-"))
    partial = {}
    for name in names:
        why = unfinished(d, name, inputs, suite)
        if why:
            partial[name] = why
    src = lambda name: None if name in partial else source(d, name)

    # The composition.
    order = [x for x in a.order.split(",") if x]
    base_name = order[0]
    base_path = source(d, base_name)
    if not base_path:
        sys.exit("compose: no %s (the base)" % base_name)
    base_recs = {}
    for r in jl(base_path):
        base_recs.setdefault(r["task"], r)
    tasks = sorted(base_recs)
    n = len(tasks)
    fam = {t: suite[t]["family"] for t in tasks}

    def lines_of(path):
        wg = G.grades(path, wait=True)
        return [(r["task"], r["reply"], bool(wg[(r["task"], r["reply"])]["pass"]))
                for r in jl(path) if (r.get("task"), r.get("reply")) in wg]

    srcs, missing = [], []
    for name in order:
        path = src(name)
        if path:
            srcs.append((name, lines_of(path)))
        else:
            missing.append(name)
    final = compose(srcs)
    out_path = at(a.out)
    write_if_changed(out_path, "".join(json.dumps({"task": t, "model": "final", "reply": final[t][1],
                                                   "from": final[t][0]}, ensure_ascii=False) + "\n" for t in tasks))
    each_final = os.path.join(ed, "each-final.txt")
    fg = {j["task"]: j for j in grade_lines(iq, suite_path, out_path, each_final)}
    write_if_changed(os.path.join(ed, "score-final.txt"),
                     "".join(x for x in read_text(each_final).splitlines(True) if not x.startswith("{")))
    final_pass = {t: bool(fg[t]["pass"]) for t in tasks}
    bg = G.grades(base_path)
    base_pass = {t: bool(bg[(t, base_recs[t]["reply"])]["pass"]) for t in tasks}
    base_clean = {}
    for t, _, clean in srcs[0][1]:
        base_clean.setdefault(t, clean)
    residual = [t for t in tasks if not base_clean[t] and bg[(t, base_recs[t]["reply"])]["stage"] != "reply"]

    def real_pass(path):
        g = G.grades(path)
        recs = {}
        for r in jl(path):
            recs.setdefault(r.get("task"), r)
        return {t: bool(g[(t, r["reply"])]["pass"]) for t, r in recs.items() if (t, r.get("reply")) in g}

    # The header: GLM alone, the reader, the base (the harness), base + the recorded samples, final.
    hd = {"base": base_name, "base_pass": sum(base_pass.values()), "final": sum(final_pass.values()), "tasks": n}
    for key, name in (("glm", "answers-glm.jsonl"), ("read", "answers-read.jsonl")):
        if os.path.exists(at(name)):
            hd[key] = sum(v for t, v in real_pass(at(name)).items() if t in base_recs)
    if "read" not in hd:
        hd["read"], hd["read_by_day"] = READ_BY_DAY, True
    rec_src = next((s for s in srcs if s[0] == a.rec), None)
    if rec_src and a.rec != base_name:
        both = compose([srcs[0], rec_src])
        rec_path = os.path.join(ed, "composed-%s-%s.jsonl" % (base_name, a.rec))
        write_if_changed(rec_path, "".join(json.dumps({"task": t, "model": "%s+%s" % (base_name, a.rec),
                                                       "reply": both[t][1], "from": both[t][0]},
                                                      ensure_ascii=False) + "\n" for t in tasks))
        rp = real_pass(rec_path)
        hd["rec"], hd["rec_name"] = sum(rp.get(t, False) for t in tasks), a.rec
    m = {"night": night, "tasks": n, "header": hd, "missing": missing, "partial": partial}

    took = {}
    for t in tasks:
        k = final[t][0] if not final[t][2] else base_name + " (runs clean)"
        if k == base_name:
            k = base_name + " (nothing ran clean)"
        took.setdefault(k, [0, 0])
        took[k][0] += 1
        took[k][1] += final_pass[t] and not base_pass[t]
    m["composition"] = {"from": {k: v[0] for k, v in took.items()}, "gained": {k: v[1] for k, v in took.items()},
                        "lost": sum(1 for t in tasks if base_pass[t] and not final_pass[t])}
    m["models"] = pair(base_name, "final", {t: {"pass": base_pass[t]} for t in tasks},
                       {t: {"pass": final_pass[t]} for t in tasks}, tasks, fam, a.alpha)
    if "glm" in hd:
        gp = real_pass(at("answers-glm.jsonl"))
        m["harness"] = {"gained": sum(1 for t in tasks if base_pass[t] and not gp.get(t)),
                        "lost": sum(1 for t in tasks if gp.get(t) and not base_pass[t])}

    # Every finished team run with a try log: its curve. Every one on every task: its passes as written.
    runs, curves, loaded = {}, {}, {}
    for name in names:
        if name in partial:
            continue
        run = load_run(d, name, G, inputs)
        if run is None:
            continue
        loaded[name] = run
        if set(tasks) <= set(run["rec_pass"]):
            runs[name] = {"pass": sum(run["rec_pass"][t] for t in tasks), "helper": run["helper"]}
        if run["drafts"]:
            curves[name] = curve(run)
    m["runs"], m["curves"] = runs, curves

    def summary(res, run, label, k):
        """A run's sums at k; what it cost unknown (None) without a try log (an older eval)."""
        row = tally(res, k, run["slots"])
        row["label"], row["K"] = label, run["K"]
        if not run["K"]:
            for key in ("oracle",) + COSTS:
                row[key] = None
        return row

    # Equal tries on the residual drafts.
    eq_names = [x for x in a.equal.split(",") if x]
    k_eq = a.equal_k
    res_draft = {t: base_pass[t] for t in residual}
    arms = {name: arm(loaded[name], residual, k_eq, res_draft) for name in eq_names if name in loaded}
    m["equal"] = {"k": k_eq, "drafts": len(residual), "arms": {}, "pairs": {}, "names": eq_names,
                  "missing": [x for x in eq_names if x not in arms]}
    for name, res in arms.items():
        m["equal"]["arms"][name] = summary(res, loaded[name], name, k_eq)
    for spec in [x for x in a.equal_pairs.split(",") if x]:
        A, B = spec.split(":")
        if A in arms and B in arms:
            m["equal"]["pairs"][spec] = pair(A, B, arms[A], arms[B], residual, fam, a.alpha)
            m["equal"]["pairs"][spec]["clean"] = pair(A, B, arms[A], arms[B], residual, fam, a.alpha, "clean")

    # Dev: the mutants of train roots.
    dev = {}
    for r in jl(at(a.dev_answers)):
        dev.setdefault(r["task"], r.get("kind") or r.get("model", "").replace("mut-", ""))
    dev_tasks = sorted(t for t in dev if t in suite)
    dfam = {t: suite[t]["family"] for t in dev_tasks}
    group = lambda kind: "reasoning" if kind in REASONING else "pattern" if kind in PATTERN else "other"
    m["dev"] = {"tasks": len(dev_tasks), "arms": {}, "pairs": {}, "missing": []}
    dev_res = {}
    for spec in [x for x in a.dev.split(",") if x]:
        name, _, k = spec.partition("@")
        if name not in loaded or not dev_tasks:
            m["dev"]["missing"].append(spec)
            continue
        res = arm(loaded[name], dev_tasks, int(k or 1), {})
        dev_res[spec] = res
        row = summary(res, loaded[name], spec, int(k or 1))
        row["pass_pct"] = 100.0 * row["pass"] / len(dev_tasks)
        row["clean_pct"] = 100.0 * row["clean"] / len(dev_tasks)
        row["groups"] = {}
        for g in ("reasoning", "pattern", "other"):
            ts = [t for t in dev_tasks if group(dev[t]) == g]
            if ts:
                row["groups"][g] = {"n": len(ts), "pass": sum(res[t]["pass"] for t in ts),
                                    "clean": sum(res[t]["clean"] for t in ts)}
        m["dev"]["arms"][spec] = row
    for spec in [x for x in a.dev_pairs.split(",") if x]:
        A, B = spec.split(":")
        if A in dev_res and B in dev_res:
            m["dev"]["pairs"][spec] = pair(A, B, dev_res[A], dev_res[B], dev_tasks, dfam, a.alpha)
            m["dev"]["pairs"][spec]["clean"] = pair(A, B, dev_res[A], dev_res[B], dev_tasks, dfam, a.alpha, "clean")

    # The cascade: the first run's tries until clean, then the second's (what each try cost: their logs).
    c1, c2 = (a.cascade.split(",") + ["", ""])[:2]
    m["cascade_runs"] = [x for x in (c1, c2) if x]
    if c1 in loaded and c2 in loaded and loaded[c1]["K"] and loaded[c2]["K"]:
        A, B = (arms.get(c) or arm(loaded[c], residual, k_eq, res_draft) for c in (c1, c2))
        sa, sb = loaded[c1]["slots"], loaded[c2]["slots"]
        rows = {}
        for label, res, sl in ((c1, A, sa), (c2, B, sb)):
            row = tally(res, k_eq, sl)
            row["label"] = "%s only" % label
            rows[label] = row
        both = {}
        gpu = 0.0
        for t in residual:
            x, y = A[t], B[t]
            if x["clean"]:
                both[t] = dict(x)
                gpu += x["ms"] / 1000.0 / sa
            else:
                # The second's choice, unless only the first kept a try; what both spent.
                z = dict(y if y["picked"] or not x["picked"] else x)
                z["calls"], z["tokens"], z["ms"] = x["calls"] + y["calls"], x["tokens"] + y["tokens"], x["ms"] + y["ms"]
                z["oracle"] = x["oracle"] or y["oracle"]
                both[t] = z
                gpu += x["ms"] / 1000.0 / sa + y["ms"] / 1000.0 / sb
        row = tally(both, k_eq, 1)
        row["gpu_s"] = gpu
        row["gpu_s_per_extra"] = per(gpu, row["extra"])
        row["label"], row["both"] = "%s→%s" % (c1, c2), True
        rows[row["label"]] = row
        m["cascade"] = {"k": k_eq, "drafts": len(residual), "rows": rows}

    # Finish: the 7B writing GLM's unfinished programs from its notes.
    pick, repair = (a.finish.split(",") + ["", ""])[:2]
    fin_tasks = sorted({r["task"] for r in jl(at("prompts-fin.jsonl")) if "task" in r} or
                       {r["task"] for r in jl(at("answers-%s.jsonl" % pick)) if "task" in r})
    if fin_tasks and src(pick):
        frows = {}

        def one(path, label):
            wg, rg = G.grades(path, wait=True), G.grades(path)
            got = {}
            for r in jl(path):
                key = (r.get("task"), r.get("reply"))
                if key[0] in fin_tasks and key in wg and key[0] not in got:
                    got[key[0]] = (bool(wg[key]["pass"]), bool(rg[key]["pass"]))
            frows[label] = {"clean": sum(c for c, _ in got.values()), "pass": sum(p for _, p in got.values()),
                            "passed": sorted(t for t, (_, p) in got.items() if p)}
        one(src(pick), pick)
        if repair and src(repair):
            one(src(repair), repair)
        samples = at("answers-%s-samples.jsonl" % pick)
        if os.path.exists(samples):
            wg, rg = G.grades(samples, wait=True), G.grades(samples)
            ss = [(r["task"], r["reply"]) for r in jl(samples)
                  if (r.get("task"), r.get("reply")) in wg and r["task"] in fin_tasks]
            frows["samples"] = {"n": len(ss), "clean": sum(wg[x]["pass"] for x in ss),
                                "pass": sum(rg[x]["pass"] for x in ss),
                                "any_pass": len({x[0] for x in ss if rg[x]["pass"]})}
        if rec_src:
            rpath = source(d, a.rec)
            wg, rg = G.grades(rpath, wait=True), G.grades(rpath)
            got = {}
            for r in jl(rpath):
                key = (r.get("task"), r.get("reply"))
                if key[0] in fin_tasks and key in wg and wg[key]["pass"] and key[0] not in got:
                    got[key[0]] = bool(rg[key]["pass"])
            frows[a.rec] = {"clean": len(got), "pass": sum(got.values()),
                            "passed": sorted(t for t, p in got.items() if p)}
        fin = set(frows[pick]["passed"]) | set(frows.get(repair, {}).get("passed", []))
        rec = set(frows.get(a.rec, {}).get("passed", []))
        m["finish"] = {"tasks": len(fin_tasks), "rows": frows,
                       "overlap": {"both": len(fin & rec), "finish_only": len(fin - rec), "rec_only": len(rec - fin)}}

    m["judges"], odd_judges = judges(d)
    m["replicates"] = replicates(d, G, [x for x in a.replicates.split(",") if x], tasks, partial)
    preds = []
    for line in jl(at(a.predictions)):
        got, lo, hi = measured(line, m), line.get("lo"), line.get("hi")
        state = ("no range" if lo is None or hi is None else "not run" if got is None else
                 "inside" if lo <= got <= hi else "above" if got > hi else "below")
        preds.append(dict(line, measured=got, state=state))
    m["predictions"] = preds
    rules = (json.loads(read_text(at(a.decisions))) or {}).get("rules", []) if os.path.exists(at(a.decisions)) else []
    m["decisions"] = verdicts(rules, m)

    text = report(m, odd_judges)
    print(text)
    write_if_changed(at(a.report), text)
    write_if_changed(at(a.json), json.dumps(m, indent=1) + "\n")


def header_line(hd):
    parts = []
    if "glm" in hd:
        parts.append("glm %s" % fmt(hd["glm"]))
    parts.append("read %s%s" % (fmt(hd["read"]), " (by day)" if hd.get("read_by_day") else ""))
    parts.append("%s %s (%s)" % (hd["base"], fmt(hd["base_pass"]), HARNESS))
    if "rec" in hd:
        parts.append("%s+%s %s" % (hd["base"], hd["rec_name"], fmt(hd["rec"])))
    parts.append("final %s" % fmt(hd["final"]))
    return " | ".join(parts)


def cnt(x):
    return "—" if x is None else "%d" % x


def num(x):
    """A cost: none, or as many decimals as its size needs."""
    if x is None:
        return "—"
    return "%.0f" % x if abs(x) >= 100 else "%.1f" % x if abs(x) >= 10 else "%.2f" % x


def needs_s(c):
    return ("±%d" % c["needs"]) if c["needs"] is not None else "none at %d" % c["discordant"]


def report(m, odd_judges=()):
    L = []
    w = L.append
    hd, n = m["header"], m["tasks"]
    w("# Night report: team-%s, %d held-out tasks" % (m["night"], n))
    w("")
    w(header_line(hd))
    w("")
    cps = lambda p: ("%d / %s = %.2f" % (n, fmt(p), n / p)) if p else "—"
    w("Cloud calls per solve (one a task): final %s%s; %s %s." % (
        cps(hd["final"]), ("; GLM alone %s" % cps(hd["glm"])) if "glm" in hd else "", hd["base"], cps(hd["base_pass"])))
    w("")
    if "harness" in m:
        w("- **Harness** (%s against GLM alone): +%d −%d, in-sample: fitted on these held drafts, so no p-value." % (
            hd["base"], m["harness"]["gained"], m["harness"]["lost"]))
    c = m["models"]
    w("- **Models** (final against %s, paired by family): +%s −%s, net %s [%+.1f, %+.1f], p %s, needs %s: %s." % (
        hd["base"], fmt(c["gained"]), fmt(c["lost"]), signed(c["net"]), c["lo"], c["hi"], pv(c["p"]), needs_s(c),
        c["verdict"]))
    comp = m["composition"]
    w("- final takes: " + "; ".join("%s %d (+%d)" % (k, v, comp["gained"][k])
                                     for k, v in sorted(comp["from"].items(), key=lambda x: -x[1]))
      + "; loses %d of %s's passes (never-worse merge)." % (comp["lost"], hd["base"]))
    part = m.get("partial", {})
    nr = lambda x: x + (" (did not finish)" if x.partition("@")[0] in part else "")
    if m["missing"]:
        w("- not run: %s." % ", ".join(nr(x) for x in m["missing"]))
    if part:
        w("- did not finish, so left out of every table and verdict (rerun its step, then the summary): %s." % ", ".join(
            "%s (%s)" % (x, part[x]) for x in sorted(part)))
    w("")
    w("## Curves")
    w("")
    w("A draft is one the helper took a turn on. made clean: the try eval keeps of the first k ran clean; first clean: "
      "passes with it; oracle: any clean try of the first k passes (%s); run passes: the run's on every task had it "
      "stopped at k; costs: the tries a run stopping at its first clean try makes; GPU-s = Σms / slots." % CEILING)
    for name, cv in sorted(m["curves"].items()):
        w("")
        w("### %s (%s, %d drafts, up to %d tries, %d slots%s)" % (
            name, cv["helper"], cv["drafts"], cv["K"], cv["slots"],
            (", %d context overflows" % cv["errors"]) if cv["errors"] else ""))
        w("")
        w("| k | made clean | first clean passes | oracle (ceiling) | sample clean | sample pass | run passes | "
          "extra solves | helper calls | per extra solve | tokens | per extra solve | GPU-s | per extra solve |")
        w("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|")
        for r in cv["rows"]:
            w("| %d | %d | %d | %d | %.0f%% | %.0f%% | %d | %d | %d | %s | %d | %s | %s | %s |" % (
                r["k"], r["clean"], r["first"], r["oracle"], 100 * r["sample_clean"], 100 * r["sample_pass"],
                r["run_pass"], r["extra"], r["calls"], num(r["calls_per_extra"]), r["tokens"],
                num(r["tokens_per_extra"]), num(r["gpu_s"]), num(r["gpu_s_per_extra"])))
        if cv["records_clean"] != cv["rows"][-1]["clean"]:
            w("")
            w("note: made clean at %d is %d, but %d out records say clean." % (
                cv["K"], cv["rows"][-1]["clean"], cv["records_clean"]))
        w("")
        w("By the draft's fault code (at k = %s):" % "/".join(str(r["k"]) for r in cv["rows"]))
        w("")
        w("| code | drafts | made clean | first clean passes | oracle | sample clean @%d | calls per extra solve @%d "
          "| GPU-s per extra solve @%d |" % (cv["K"], cv["K"], cv["K"]))
        w("|---|---|---|---|---|---|---|---|")
        for code, rows in sorted(cv["codes"].items(), key=lambda x: (-x[1][0]["drafts"], int(x[0]))):
            last = rows[-1]
            w("| %s | %d | %s | %s | %s | %.0f%% | %s | %s |" % (
                "E%04d" % int(code) if int(code) else "none", last["drafts"], "/".join(str(r["clean"]) for r in rows),
                "/".join(str(r["first"]) for r in rows), "/".join(str(r["oracle"]) for r in rows),
                100 * last["sample_clean"], num(last["calls_per_extra"]), num(last["gpu_s_per_extra"])))
        hs = cv["held"]
        w("")
        w("Held, of %d turns: %s." % (
            hs["turns"], ", ".join("%s %.0f%%" % (k, 100 * v) for k, v in hs["share"].items())))
    eq = m["equal"]
    w("")
    w("## Equal tries: @%d on %d residual drafts" % (eq["k"], eq["drafts"]))
    w("")
    if eq["arms"]:
        w("| run | tries | made clean | passes | oracle (ceiling) | extra solves | helper calls | GPU-s "
          "| GPU-s per extra solve |")
        w("|---|---|---|---|---|---|---|---|---|")
        for name, r in eq["arms"].items():
            tries = "no try log" if not r["K"] else "%d%s" % (
                min(eq["k"], r["K"]), " (of %d)" % r["K"] if r["K"] != eq["k"] else "")
            w("| %s | %s | %d | %d | %s | %d | %s | %s | %s |" % (
                name, tries, r["clean"], r["pass"], cnt(r["oracle"]), r["extra"], cnt(r["calls"]), num(r["gpu_s"]),
                num(r["gpu_s_per_extra"])))
        pairs_table(w, eq["pairs"])
    if eq["missing"]:
        if eq["arms"]:
            w("")
        w("Not run: %s." % ", ".join(nr(x) for x in eq["missing"]))
    dv = m["dev"]
    w("")
    w("## Dev: %d mutants of train roots (known answers, real checks)" % dv["tasks"])
    w("")
    if dv["arms"]:
        w("| arm | passes | % | made clean | % | reasoning pass/clean (n) | pattern pass/clean (n) | helper calls "
          "| GPU-s |")
        w("|---|---|---|---|---|---|---|---|---|")
        for name, r in dv["arms"].items():
            gs = lambda g: ("%d/%d (%d)" % (r["groups"][g]["pass"], r["groups"][g]["clean"], r["groups"][g]["n"])
                            if g in r["groups"] else "")
            w("| %s | %d | %.1f | %d | %.1f | %s | %s | %s | %s |" % (
                name, r["pass"], r["pass_pct"], r["clean"], r["clean_pct"], gs("reasoning"), gs("pattern"),
                cnt(r["calls"]), num(r["gpu_s"])))
        pairs_table(w, dv["pairs"])
    if dv["missing"]:
        if dv["arms"]:
            w("")
        w("Not run: %s." % ", ".join(nr(x) for x in dv["missing"]))
    if "cascade" in m:
        cs = m["cascade"]
        w("")
        w("## Cascade: @%d on %d residual drafts" % (cs["k"], cs["drafts"]))
        w("")
        w("| helpers | made clean | passes | extra solves | helper calls | GPU-s | GPU-s per extra solve |")
        w("|---|---|---|---|---|---|---|")
        for r in cs["rows"].values():
            w("| %s | %d | %d | %d | %d | %s | %s |" % (
                r["label"], r["clean"], r["pass"], r["extra"], r["calls"], num(r["gpu_s"]), num(r["gpu_s_per_extra"])))
    if "finish" in m:
        f = m["finish"]
        w("")
        w("## Finish: %d replies that ran out before their program" % f["tasks"])
        w("")
        w("| from | run clean | pass |")
        w("|---|---|---|")
        for name, r in f["rows"].items():
            if name == "samples":
                w("| %s (%d samples) | %d | %d (any passes: %d tasks, %s) |" % (
                    name, r["n"], r["clean"], r["pass"], r["any_pass"], CEILING))
            else:
                w("| %s | %d | %d |" % (name, r["clean"], r["pass"]))
        o = f["overlap"]
        w("")
        w("Passes: both %d, the finish only %d, the recorded samples only %d." % (
            o["both"], o["finish_only"], o["rec_only"]))
    if m["judges"] or odd_judges:
        w("")
        w("## Judges on %s's clean programs" % hd["base"])
        w("")
        auc = lambda x: "—" if not x else "%.3f%s" % (
            x[0], (" [%.3f, %.3f]" % (x[1], x[2])) if x[1] is not None and x[2] is not None else "")
        for name, e in sorted(m["judges"].items()):
            w("- %s: judge %s, length %s, lift %s." % (
                name, auc(e.get("judge")), auc(e.get("length")), auc(e.get("lift"))))
        if odd_judges:
            w("- not in a known shape: %s." % ", ".join("judgefit-%s.json" % x for x in odd_judges))
    if m["replicates"]:
        w("")
        w("## Replicates")
        w("")
        for name, e in sorted(m["replicates"].items()):
            w("- %s: %d run%s, passes %s%s." % (
                name, e["runs"], "s" if e["runs"] > 1 else "", " ".join(map(str, e["per_run"])),
                (", SD %.2f, σ from flips %.2f" % (e["sd"], e["sigma"])) if "sd" in e else ""))
    if m["predictions"]:
        w("")
        w("## Predictions (written before the night)")
        w("")
        w("| system | what | predicted | came | |")
        w("|---|---|---|---|---|")
        for p in m["predictions"]:
            got, lo, hi = p["measured"], p.get("lo"), p.get("hi")
            w("| %s | %s | [%s, %s] | %s | %s |" % (
                p.get("system"), p.get("what", "pass"), "?" if lo is None else fmt(lo), "?" if hi is None else fmt(hi),
                "" if got is None else fmt(got), p["state"]))
    w("")
    w("## Decisions (written before the night)")
    w("")
    if not m["decisions"]:
        w("No decisions.json.")
    for v in m["decisions"]:
        w("- **%s %s.** %s" % (v["id"], v["name"], v["rule"]))
        w("  **Verdict:** %s." % v["verdict"])
    return "\n".join(L) + "\n"


def pairs_table(w, pairs):
    if not pairs:
        return
    w("")
    w("| B against A | +gained −lost | net | 95% CI | p | needs | made clean: net (p) | verdict |")
    w("|---|---|---|---|---|---|---|---|")
    for c in pairs.values():
        cl = c["clean"]
        w("| %s against %s | +%s −%s | %s | [%+.1f, %+.1f] | %s | %s | %s (%s) | %s |" % (
            c["b"], c["a"], fmt(c["gained"]), fmt(c["lost"]), signed(c["net"]), c["lo"], c["hi"], pv(c["p"]),
            needs_s(c), signed(cl["net"]), pv(cl["p"]), c["verdict"]))


if __name__ == "__main__":
    main()
