#!/usr/bin/env python
"""The IQ history and a night's report: every answers file of a night scored by `iq score`, a
line each in <data>/iq-history.jsonl, and <data>/report-<night>.md, which sets tonight beside
the nights before.

  python train/report.py --night N --iq IQ.exe --suite S --data DIR answers-*.jsonl

A line holds the night, the answers file, the model, and its passes on the held-out families
(the number that has to climb), on the train split, by tier, and by stage. A night already in
the history is replaced, not doubled, so a rerun after a freeze reports once.
"""
import argparse
import json
import os
import re
import subprocess
import sys

import common


def score(iq, suite, answers):
    """iq score's numbers for one answers file."""
    out = subprocess.run([iq, "score", answers, "--suite", suite], capture_output=True, text=True,
                         encoding="utf-8")
    if out.returncode != 0:
        sys.exit("error: iq score %s: %s" % (answers, out.stderr.strip()))
    text = out.stdout
    m = re.match(r"(.*?): \d+ answers", text)   # a model's name may hold a colon (run:NAME)
    r = {"model": m.group(1).strip() if m else "?"}

    def frac(label):
        m = re.search(r"\b%s (\d+)/(\d+)" % label, text)
        return [int(m.group(1)), int(m.group(2))] if m else None
    r["all"], r["train"], r["held"] = frac("pass"), frac("train"), frac("held")
    r["tiers"] = {t: [int(p), int(n)] for t, p, n in re.findall(r"\b(\d): (\d+)/(\d+)", text)}
    m = re.search(r"stages\s+(.*)", text)
    r["stages"] = {k: int(v) for k, v in re.findall(r"(\w+) (\d+)", m.group(1))} if m else {}
    return r


def pct(f):
    return "-" if not f or not f[1] else "%d/%d (%d%%)" % (f[0], f[1], round(100 * f[0] / f[1]))


def main():
    common.utf8_stdio()
    h = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    h.add_argument("--night", required=True)
    h.add_argument("--iq", required=True)
    h.add_argument("--suite", required=True)
    h.add_argument("--data", required=True)
    h.add_argument("answers", nargs="+")
    a = h.parse_args()
    rows = []
    for f in a.answers:
        r = score(a.iq, a.suite, f)
        r["night"], r["file"] = a.night, os.path.basename(f)
        rows.append(r)
    path = os.path.join(a.data, "iq-history.jsonl")
    old = []
    if os.path.exists(path):
        with open(path, encoding="utf-8") as f:
            old = [json.loads(l) for l in f if l.strip()]
    history = [r for r in old if r["night"] != a.night] + rows
    common.write_atomic(path, "".join(json.dumps(r, sort_keys=True) + "\n" for r in history))
    nights = sorted({r["night"] for r in history})
    lines = ["# IQ, night %s" % a.night, "",
             "Held-out families: the number that has to climb. Train: tasks the models may have",
             "learned from. Tiers: 1 a counter ... 6 an ambitious game.", "",
             "| model | answers | held | train | " + " | ".join("T%d" % t for t in range(1, 7)) + " |",
             "|---|---|---|---|" + "---|" * 6]
    for r in rows:
        tiers = " | ".join(pct(r["tiers"].get(str(t))) for t in range(1, 7))
        lines.append("| %s | %s | **%s** | %s | %s |" % (r["model"], r["file"], pct(r["held"]), pct(r["train"]), tiers))
    lines += ["", "## Held-out pass rate by night", "", "| model | " + " | ".join(nights) + " |",
              "|---|" + "---|" * len(nights)]
    for model in sorted({r["model"] for r in history}):
        cells = []
        for n in nights:
            hit = [r for r in history if r["model"] == model and r["night"] == n]
            cells.append(pct(hit[-1]["held"]) if hit else "")
        lines.append("| %s | %s |" % (model, " | ".join(cells)))
    lines += ["", "## Where they fail (held + train, by stage)", ""]
    for r in rows:
        lines.append("- %s: %s" % (r["model"], ", ".join("%s %d" % kv for kv in r["stages"].items())))
    out = os.path.join(a.data, "report-%s.md" % a.night)
    common.write_atomic(out, "\n".join(lines) + "\n")
    print("\n".join(lines))
    print("\nwrote %s and %s" % (out, path))


if __name__ == "__main__":
    main()
