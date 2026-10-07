#!/usr/bin/env python
"""The IQ history and a night's report: every answers file of a night scored by `iq score`, a
line each in <data>/iq-history.jsonl, and <data>/report-<night>.md, which sets tonight beside
the nights before.

  python train/report.py --night N --iq IQ.exe --suite S --data DIR [--prompts HELD.jsonl]
                         answers-X.jsonl[=K] ...

A row is named by its role, from its file's name, so a role's nights line up: answers-glm.jsonl
is glm, answers-base-q05.jsonl base-q05, answers-<night>-q05-self.jsonl q05-self. It holds the
night, the file, the model named in it, its passes on the held-out families (the number that
has to climb), on the train split, by tier and by stage, and each held-out task it was scored
on with its passes. The held-out set grows as tasks are imported, so beside each rate on every
task a row answered stands its rate on the tasks every complete row shares: those compare
across rows and nights. Given --prompts (the held-out prompts) and K (samples a task), a file
with fewer than K answers to some prompt's task (1 at temperature 0, as generate.py answers
it) is partial (its step was paused or stopped), is marked so, and does not narrow the shared
tasks. A night already in the history is
replaced, not doubled, so a rerun after a freeze reports once.

Each held-out rate carries its 90% interval by family (a cluster bootstrap: families drawn again
with replacement, 2,000 times, from a fixed seed), since the tasks of one family, and the samples
of one task, are not independent (DESIGN.md, Evolution, "The gate"). With --predictions (a line a
role: {"role", "held", "lo", "hi"}, rates as fractions, written before the night was scored), the
report sets each prediction beside what happened: inside its interval or not, and by how much.
"""
import argparse
import json
import os
import random
import re
import subprocess
import sys

import common

# The roles night.sh and day.sh make, in the order the tables list them; others follow.
ROLES = ["glm", "base-q05", "base-q3", "q05", "q05-self", "q3"]


class Unscored(Exception):
    pass


def role(path, night=None):
    """A row's role from its answers file's name: answers-n20261005-q05.jsonl is q05."""
    name = os.path.basename(path)
    name = name[len("answers-"):] if name.startswith("answers-") else name
    name = name[:-len(".jsonl")] if name.endswith(".jsonl") else name
    if night and name.startswith(night + "-"):
        return name[len(night) + 1:]
    return re.sub(r"^n\d{8}-", "", name)


def iq_out(args):
    """What an iq command printed, split on \\n alone (a message may hold U+2028)."""
    out = subprocess.run(args, capture_output=True)
    text = out.stdout.decode("utf-8", errors="replace")
    if out.returncode != 0:
        why = (out.stderr.decode("utf-8", errors="replace") or text).strip()
        raise Unscored("iq %s exited %d: %s" % (args[1], out.returncode, why))
    return text.split("\n")


def held_ids(iq, suite):
    """The held-out task ids, as `iq split` lists them: `family held id id ...`."""
    ids = set()
    for line in iq_out([iq, "split", "--suite", suite]):
        parts = line.split()
        if len(parts) >= 3 and parts[1] == "held":
            ids.update(parts[2:])
    return ids


def frac(label, text):
    m = re.search(r"\b%s (\d+)/(\d+)" % label, text)
    return [int(m.group(1)), int(m.group(2))] if m else [0, 0]


def add(f, g):
    return [f[0] + g[0], f[1] + g[1]]


def score(iq, suite, held, path, k, prompts):
    """One answers file's row: iq score's tallies, summed over the models named in it, and each
    held-out task's passes from its --each lines (checked against those tallies)."""
    each, blocks = [], []
    for line in iq_out([iq, "score", path, "--suite", suite, "--each"]):
        if line.startswith("{"):
            each.append(json.loads(line))
        elif line.strip() and not line.startswith(" "):
            m = re.fullmatch(r"(.*): (\d+) answers, \d+ ms a grade", line.rstrip("\r"))
            if not m:
                raise Unscored("iq score printed a line it does not print: %r" % line[:200])
            blocks.append({"model": m.group(1), "n": int(m.group(2)), "text": ""})
        elif blocks:
            blocks[-1]["text"] += line + "\n"
    r = {"model": " + ".join(b["model"] for b in blocks), "answers": len(each),
         "all": [0, 0], "train": [0, 0], "held": [0, 0], "tiers": {}, "stages": {}}
    for b in blocks:
        t = b["text"]
        for key, label in (("all", "pass"), ("train", "train"), ("held", "held")):
            r[key] = add(r[key], frac(label, t))
        for tier, p, n in re.findall(r"\b(\d): (\d+)/(\d+)", t):
            r["tiers"][tier] = add(r["tiers"].get(tier, [0, 0]), [int(p), int(n)])
        m = re.search(r"stages\s+(.*)", t)
        for stage, n in re.findall(r"(\w+) (\d+)", m.group(1)) if m else []:
            r["stages"][stage] = r["stages"].get(stage, 0) + int(n)
    r["tiers"] = dict(sorted(r["tiers"].items()))
    count, tasks = {}, {}
    for e in each:
        count[e["task"]] = count.get(e["task"], 0) + 1
        if e["task"] in held and e["stage"] != "harness":   # a harness grade is counted apart
            tasks[e["task"]] = add(tasks.get(e["task"], [0, 0]), [int(bool(e["pass"])), 1])
    r["held_tasks"] = dict(sorted(tasks.items()))
    summed = [sum(v[0] for v in tasks.values()), sum(v[1] for v in tasks.values())]
    if summed != r["held"] or len(each) != sum(b["n"] for b in blocks):
        raise Unscored("iq score's held tally %s and its answers %s disagree (%d answers)" % (
            r["held"], summed, len(each)))
    r["answered"], r["expected"], r["partial"] = None, None, None
    if k is not None and prompts is not None:
        need = {t: k if temp > 0 else 1 for t, temp in prompts.items()}   # greedy: once (generate.py)
        r["answered"] = sum(min(count.get(t, 0), n) for t, n in need.items())
        r["expected"] = sum(need.values())
        r["partial"] = r["answered"] < r["expected"]
    return r


def pct(f):
    return "-" if not f or not f[1] else "%d/%d (%d%%)" % (f[0], f[1], round(100 * f[0] / f[1]))


def families(suite):
    """Each task's family, from the suite."""
    with open(suite, encoding="utf-8") as f:
        return {t["id"]: t["family"] for t in (json.loads(l) for l in f if l.strip())}


def by_family(tasks, fam_of, reps=2000, seed=1):
    """The 90% interval of a held-out rate, in percent, by a cluster bootstrap over families:
    the families drawn again with replacement, each with all its tasks and samples."""
    fams = {}
    for t, (p, n) in tasks.items():
        f = fams.setdefault(fam_of.get(t, t), [0, 0])
        f[0], f[1] = f[0] + p, f[1] + n
    groups = list(fams.values())
    if not groups or not sum(n for _, n in groups):
        return None
    rng, rates = random.Random(seed), []
    for _ in range(reps):
        draw = [groups[rng.randrange(len(groups))] for _ in groups]
        n = sum(g[1] for g in draw)
        rates.append(100.0 * sum(g[0] for g in draw) / n if n else 0.0)
    rates.sort()
    return [round(rates[int(0.05 * reps)], 1), round(rates[int(0.95 * reps) - 1], 1)]


def ci(r):
    """A row's held-out rate with its interval by family."""
    c = r.get("held_ci")
    return pct(r["held"]) + (" [%d, %d]" % (round(c[0]), round(c[1])) if c else "")


def shared(rows):
    """The held-out tasks every complete row that keeps its tasks was scored on, or None."""
    sets = [set(r["held_tasks"]) for r in rows if "held_tasks" in r and not r.get("partial")]
    return set.intersection(*sets) if sets else None


def on(r, tasks):
    """r's held-out passes on tasks, if it was scored on all of them."""
    if not tasks or "held_tasks" not in r or not tasks <= set(r["held_tasks"]):
        return None
    return [sum(r["held_tasks"][t][0] for t in tasks), sum(r["held_tasks"][t][1] for t in tasks)]


def answered(r):
    """Its answers; with its prompts, how many of the answers they ask it holds."""
    if r.get("expected") is None:
        return str(r["answers"])
    return "%d of %d%s" % (r["answered"], r["expected"], ", partial" if r["partial"] else "")


def main():
    common.utf8_stdio()
    h = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    h.add_argument("--night", required=True)
    h.add_argument("--iq", required=True)
    h.add_argument("--suite", required=True)
    h.add_argument("--data", required=True)
    h.add_argument("--prompts", help="the held-out prompts: with FILE=K, a file short of K a task is partial")
    h.add_argument("--predictions", help="what was predicted for each role before the night was scored")
    h.add_argument("answers", nargs="+", help="answers-X.jsonl, or answers-X.jsonl=K (its samples a task)")
    a = h.parse_args()
    specs = []
    for s in a.answers:
        path, eq, k = s.rpartition("=")
        specs.append((path, int(k)) if eq and k.isdigit() else (s, None))
    if any(k is not None for _, k in specs) and not a.prompts:
        h.error("FILE=K needs --prompts")
    prompts = None
    if a.prompts:
        with open(a.prompts, encoding="utf-8") as f:
            prompts = {r["task"]: float(r.get("temperature", 1)) for r in (json.loads(l) for l in f if l.strip())}
    rows, unscored = [], []
    try:
        held = held_ids(a.iq, a.suite)
    except Unscored as e:
        sys.exit("error: %s" % e)
    fam_of = families(a.suite)
    predicted, skipped = {}, []
    if a.predictions and os.path.exists(a.predictions):
        # An agent writes these in the night: a bad line is skipped, never the report.
        with open(a.predictions, encoding="utf-8-sig") as f:
            for i, line in enumerate(f, 1):
                if not line.strip():
                    continue
                try:
                    p = json.loads(line)
                    mid, lo, hi = (float(p.get(k, p.get("held"))) for k in ("held", "lo", "hi"))
                    if not isinstance(p.get("role"), str) or not 0 <= lo <= mid <= hi <= 1:
                        raise ValueError("needs a role and 0 <= lo <= held <= hi <= 1")
                    predicted[p["role"]] = dict(p, held=mid, lo=lo, hi=hi)
                except (ValueError, TypeError, AttributeError) as e:
                    skipped.append("prediction line %d: %s" % (i, e))
                    print("report: prediction line %d skipped: %s" % (i, e), file=sys.stderr)
    for path, k in specs:
        try:
            r = score(a.iq, a.suite, held, path, k, prompts)
        except (Unscored, ValueError, KeyError) as e:
            unscored.append("%s: %s" % (os.path.basename(path), e))
            print("report: %s not scored: %s" % (path, e), file=sys.stderr)
            continue
        r["night"], r["file"], r["role"] = a.night, os.path.basename(path), role(path, a.night)
        r["held_ci"] = by_family(r["held_tasks"], fam_of)
        if r["role"] in predicted:
            r["predicted"] = predicted[r["role"]]
        rows.append(r)
    hpath = os.path.join(a.data, "iq-history.jsonl")
    old = []
    if os.path.exists(hpath):
        with open(hpath, encoding="utf-8") as f:
            old = [json.loads(l) for l in f if l.strip()]
    history = [r for r in old if r["night"] != a.night] + rows
    common.write_atomic(hpath, "".join(json.dumps(r, sort_keys=True) + "\n" for r in history))
    for r in history:   # rows written before rows had roles
        r.setdefault("role", role(r["file"], r["night"]))
    nights = sorted({r["night"] for r in history})
    roles = sorted({r["role"] for r in history}, key=lambda x: (ROLES.index(x) if x in ROLES else len(ROLES), x))
    common_tasks = shared(history)
    n_common = len(common_tasks) if common_tasks else 0
    lines = ["# IQ, night %s" % a.night, "",
             "Held: on every held-out task a row was scored on (the number that has to climb), with its",
             "90% interval by family (a cluster bootstrap). Common: on",
             "the %d held-out tasks every complete row of the history shares, so it compares across rows" % n_common,
             "and nights. Train: tasks the models may have learned from. Tiers: 1 a counter ... 6 an",
             "ambitious game. A partial row's step was paused or stopped: it holds fewer answers than",
             "its prompts times its samples.", "",
             "| role | model | answers | held | common | train | " + " | ".join("T%d" % t for t in range(1, 7)) + " |",
             "|---|---|---|---|---|---|" + "---|" * 6]
    for r in rows:
        tiers = " | ".join(pct(r["tiers"].get(str(t))) for t in range(1, 7))
        lines.append("| %s | %s | %s | **%s** | %s | %s | %s |" % (
            r["role"], r["model"], answered(r), ci(r), pct(on(r, common_tasks)), pct(r["train"]), tiers))
    # The two benchmarks that matter apart: everyday apps (tiers 1 and 2, what most people ask)
    # and ambitious games (tiers 3 to 6, the hill to climb).
    def band(r, tiers):
        p = sum(r["tiers"].get(str(t), [0, 0])[0] for t in tiers)
        n = sum(r["tiers"].get(str(t), [0, 0])[1] for t in tiers)
        return pct([p, n])
    lines += ["", "## Everyday apps and ambitious games", "",
              "| role | everyday apps (tiers 1-2) | ambitious games (tiers 3-6) |", "|---|---|---|"]
    for r in rows:
        lines.append("| %s | %s | %s |" % (r["role"], band(r, (1, 2)), band(r, (3, 4, 5, 6))))
    # A partial row holds only the first (easiest) prompts' answers: never compared.
    guessed = [r for r in rows if r.get("predicted") and r["held"][1] and not r.get("partial")]
    scored = {r["role"]: r for r in rows}
    missed = ["%s: %s, not compared" % (x, "partial (%s of %s answers)" % (scored[x]["answered"], scored[x]["expected"])
                                         if x in scored else "not scored")
              for x in predicted if x not in {r["role"] for r in guessed}]
    if guessed or missed or skipped:
        lines += ["", "## Predicted before scoring", "",
                  "Each role's held-out rate as predicted before tonight was scored, its 80% interval,",
                  "and what happened.", ""]
    if guessed:
        lines += ["| role | predicted | actual | inside | off by |", "|---|---|---|---|---|"]
        hits = 0
        for r in guessed:
            p, got = r["predicted"], 100.0 * r["held"][0] / r["held"][1]
            lo, mid, hi = 100 * p["lo"], 100 * p["held"], 100 * p["hi"]
            hits += lo <= got <= hi
            lines.append("| %s | %d%% [%d, %d] | %d%% | %s | %+d points |" % (
                r["role"], round(mid), round(lo), round(hi), round(got),
                "yes" if lo <= got <= hi else "no", round(got - mid)))
        lines += ["", "%d of %d inside their interval (80%% would be calibrated, over many nights)." % (
            hits, len(guessed))]
    lines += ["- " + m for m in missed + skipped]

    def by_night(cell):
        out = ["| role | " + " | ".join(nights) + " |", "|---|" + "---|" * len(nights)]
        for x in roles:
            cells = []
            for n in nights:
                hit = [r for r in history if r["role"] == x and r["night"] == n]
                cells.append(cell(hit[-1]) if hit else "")
            out.append("| %s | %s |" % (x, " | ".join(cells)))
        return out
    lines += ["", "## Held-out pass rate by night, on the same tasks", "",
              "On the %d held-out tasks every complete row shares (-: a partial row, or one from before" % n_common,
              "rows kept their tasks).", ""]
    lines += by_night(lambda r: pct(on(r, common_tasks)))
    lines += ["", "## Held-out pass rate by night, on every task each was scored on", ""]
    lines += by_night(lambda r: pct(r["held"]) + (", partial" if r.get("partial") else ""))
    lines += ["", "## Where they fail (held + train, by stage)", ""]
    for r in rows:
        lines.append("- %s (%s): %s" % (r["role"], r["model"], ", ".join("%s %d" % kv for kv in r["stages"].items())))
    if unscored:
        lines += ["", "## Not scored", ""] + ["- " + u for u in unscored]
    out = os.path.join(a.data, "report-%s.md" % a.night)
    common.write_atomic(out, "\n".join(lines) + "\n")
    print("\n".join(lines))
    print("\nwrote %s and %s" % (out, hpath))
    if unscored:
        sys.exit(1)


if __name__ == "__main__":
    main()
