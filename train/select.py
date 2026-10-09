#!/usr/bin/env python
"""Choose among a model's program samples by the check it wrote itself, from the ask alone,
before any program (train/checkdata.py): the one the model's own check passes, else one that
compiles and runs, else the first. Only the choice is then graded by the real check, as one
answer a task (`report.py`'s FILE=1).

  python train/select.py --iq IQ.exe --suite S --programs answers-q3.jsonl --checks answers-q3-checks.jsonl
                         --out answers-q3-sel.jsonl [--summary FILE.json]

It also says how good the model's checks are: how many read at all, how many a correct program
(the task's own reference) passes (fairness), how they judge the programs beside the real check
(kept, falsely rejected, truly rejected, missed), and the best a perfect chooser could do (any of
the samples passes the real check).
"""
import argparse
import json
import os
import re
import subprocess
import tempfile

import common

FENCE = re.compile(r"```(?:check)?[^\n]*\n(.*?)```", re.S)


def check_of(reply):
    """The script in a reply's first fenced block, or None."""
    m = FENCE.search(reply or "")
    return m.group(1).rstrip("\n") + "\n" if m and m.group(1).strip() else None


def score(iq, answers, suite):
    """iq score --each over answers (lines), against suite (task lines): each answer's grade."""
    with tempfile.TemporaryDirectory() as d:
        a, s = os.path.join(d, "answers.jsonl"), os.path.join(d, "suite.jsonl")
        open(a, "w", encoding="utf-8").write("".join(json.dumps(x, ensure_ascii=False) + "\n" for x in answers))
        open(s, "w", encoding="utf-8").write("".join(json.dumps(t, ensure_ascii=False) + "\n" for t in suite))
        out = subprocess.run([iq, "score", a, "--suite", s, "--each"], capture_output=True)
        lines = out.stdout.decode("utf-8", "replace").split("\n")
        each = [json.loads(l) for l in lines if l.startswith("{")]
        if out.returncode != 0 and len(each) != len(answers):
            raise SystemExit("iq score failed: %s" % out.stderr.decode("utf-8", "replace")[:400])
        return each


def main():
    common.utf8_stdio()
    h = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    h.add_argument("--iq", required=True)
    h.add_argument("--suite", required=True)
    h.add_argument("--programs", required=True)
    h.add_argument("--checks", required=True)
    h.add_argument("--out", required=True)
    h.add_argument("--summary")
    a = h.parse_args()
    tasks = {t["id"]: t for t in (json.loads(l) for l in open(a.suite, encoding="utf-8") if l.strip())}
    programs = [json.loads(l) for l in open(a.programs, encoding="utf-8") if l.strip()]
    programs = [p for p in programs if p["task"] in tasks]
    checks = {}
    for l in open(a.checks, encoding="utf-8"):
        if l.strip():
            x = json.loads(l)
            checks.setdefault(x["task"], check_of(x.get("reply")))
    # The suite as the model would grade it: each task with its own check in place of the real one.
    own = [dict(tasks[t], check=c) for t, c in checks.items() if c and t in tasks]
    own_ids = {t["id"] for t in own}
    by_own = {}
    if own:
        # A check that does not read grades every answer at the harness: not a choice.
        mine = [p for p in programs if p["task"] in own_ids]
        for p, g in zip(mine, score(a.iq, mine, own)):
            by_own.setdefault(p["task"], []).append(g)
    real = score(a.iq, programs, list(tasks.values()))
    by_real = {}
    for p, g in zip(programs, real):
        by_real.setdefault(p["task"], []).append((p, g))
    chosen, how = [], {"own check": 0, "runs": 0, "first": 0}
    for task, pg in by_real.items():
        own_g = by_own.get(task, [])
        pick = next((i for i, g in enumerate(own_g) if g.get("pass")), None)
        why = "own check"
        if pick is None:
            pick = next((i for i, (_, g) in enumerate(pg) if g["stage"] in ("check", "pass")), None)
            why = "runs"
        if pick is None:
            pick, why = 0, "first"
        how[why] += 1
        chosen.append(dict(pg[pick][0]))
    common.write_atomic(a.out, "".join(json.dumps(c, ensure_ascii=False) + "\n" for c in chosen))
    # How good the model's checks are: does the task's own reference pass its check (fair)?
    fair = 0
    if own:
        refs = [{"task": t["id"], "model": "reference", "reply": "```app\n" + t["ref"].rstrip("\n") + "\n```"} for t in own]
        fair = sum(1 for g in score(a.iq, refs, own) if g.get("pass"))
    # How the model's checks judge the programs, beside the real check: a check that keeps what
    # passes and rejects what fails can choose among samples; one that rejects passing programs
    # would throw working ones away.
    agree = {"keep": 0, "false_reject": 0, "true_reject": 0, "miss": 0, "unread": 0}
    for task, pg in by_real.items():
        for (_, g), o in zip(pg, by_own.get(task, [])):
            if o.get("stage") == "harness":
                agree["unread"] += 1
            elif g.get("pass"):
                agree["keep" if o.get("pass") else "false_reject"] += 1
            else:
                agree["miss" if o.get("pass") else "true_reject"] += 1
    summary = {
        "tasks": len(by_real),
        "agree": agree,
        "checks_read": len(own), "checks_written": sum(1 for c in checks.values() if c),
        "fair": fair,
        "chosen_by": how,
        "one_shot": sum(1 for g in real if g.get("pass")) / max(1, len(real)),
        # The chosen answers' real grades (one a task), and the best a perfect chooser could do.
        "chosen": sum(1 for g in score(a.iq, chosen, list(tasks.values())) if g.get("pass")),
        "best_of_k": sum(1 for pg in by_real.values() if any(g.get("pass") for _, g in pg)),
    }
    print("select: %d tasks; the model's checks: %d written, %d read, %d fair to the reference; chosen by %s; "
          "one shot %.1f%%, chosen %d, best of the samples %d; against the real check %s" % (
              summary["tasks"], summary["checks_written"], summary["checks_read"], fair, how,
              100 * summary["one_shot"], summary["chosen"], summary["best_of_k"], agree))
    if a.summary:
        common.write_atomic(a.summary, json.dumps(summary, indent=1) + "\n")


if __name__ == "__main__":
    main()
