#!/usr/bin/env python
"""The team night's inputs, each made once and the same way every time (so a rerun's sft.py
config hash stays the same), and the two small steps team.sh runs between a model and `iq score`.

  python train/nightprep.py suite --suite iq.jsonl --out suite-wait.jsonl
  python train/nightprep.py pre --src mx-answers-pre.jsonl --sha 2e24176c71a87e68 --out answers-pre.jsonl
  python train/nightprep.py dev --pool pool-dev.jsonl --held held.txt [--suite iq.jsonl] --out answers-dev.jsonl
  python train/nightprep.py judge --src DIR --out DIR [--held held.txt]
  python train/nightprep.py finish --answers answers-glm.jsonl --each each-pre.txt
                                   --prompts prompts-held.jsonl --out prompts-fin.jsonl
  python train/nightprep.py q3rec --src answers-n20261007-q3-gram.jsonl --out q3rec.jsonl
  python train/nightprep.py pick --samples F --suite-wait S --iq IQ.exe --out OUT
  python train/nightprep.py clean IN OUT

suite   every check made `# runs clean only`: graded by it, a program passes when it runs clean
        through its smoke test, and the hidden check is never run (the make loop's own test).
pre     the harness's answers (the last clean block, salvage, the fixers), copied only when their
        sha256 begins with --sha.
dev     known-answer repair problems: one faulting mutant of each dev task's reference (train
        roots, never held), its kind chosen by FNV-1a 64 from the first kind group the task has;
        a mutant with a triple line break (a deleted line's giveaway) is never one.
judge   judge-train, judge-eval-glm and judge-eval-alt (DIR/<name>.jsonl) with the comments of the
        program they show stripped, as <name>-s.jsonl: 60 of judge-train's 855 failing programs
        and 15 of judge-eval-alt's open with a '// Wrong:' line, which a judge could read instead
        of the program. Prints each eval set's length baseline (a shorter prompt says yes) AUC.
finish  the held prompts of the tasks pre left at stage reply (GLM reasoned until it ran out of
        room), each with the last 8,000 characters of GLM's reply appended as notes.
q3rec   the recorded q3 grammar samples (four a task), copied.
pick    one sample a task, chosen without the hidden check: graded under --suite-wait in file
        order, the first that passes (it runs clean), else the one that got furthest (smoke over
        compile), the first of equals.
clean   IN without the lines `iq score` cannot read (not JSON, or no string task, model, reply).

Outputs are written whole or not at all (common.write_atomic). Those of suite, pre, dev, judge,
finish and q3rec are left alone once they exist, and each one's sha256 goes into inputs.sha256
beside it (once); pick is made again when its samples are newer; clean is made every time.
Exit status: 0, 1 when something is wrong (a sha, a held root, iq), 2 when an input is missing.
"""
import argparse
import json
import os
import re
import subprocess
import sys
import tempfile

import common
from sftdata import read_lines, root

WAIT_CHECK = '# runs clean only\nexpect not says "qqzzxq never shown"\n'
# A task's mutant comes from the first group it has a kind of: the kinds the helper must reason
# about first, then the pattern kinds.
GROUPS = (("bound1", "size1", "size", "guard", "state", "fn", "decl", "builtin"),
          ("name", "clear", "toplevel", "letstate", "ret", "cast", "shadow"))
JUDGE_SETS = ("judge-train", "judge-eval-glm", "judge-eval-alt")
# judgedata.rec's user message: the ask, the program in an app block, the question.
JUDGE_USER = re.compile(r"\A(Asked: .*?\n\nProgram:\n```app\n)(.*)"
                        r"(\n```\n\nDoes it do everything asked\?)\Z", re.S)
NOTES = "\n\nNotes from a first attempt that ran out of room before it wrote the program " \
        "(they may be unfinished or wrong):\n"
WRITE_NOW = "\n\nWrite the whole program now, in one app block."
NOTE_CHARS = 8000
CTX, CHARS_PER_TOKEN = 16384, 2.96   # a slot's context (team.sh's serve), characters a token
STAGES = ("reply", "compile", "smoke", "harness", "check", "pass")   # iq::Stage, in order


class Fail(Exception):
    """What is wrong, said; exit status 1."""


def need(*paths):
    for p in paths:
        if not os.path.exists(p):
            print("nightprep: needs %s" % p, file=sys.stderr)
            sys.exit(2)


def jsonl(path):
    with open(path, encoding="utf-8") as f:
        return [json.loads(l) for l in f if l.strip()]


def lines_of(rows):
    return "".join(json.dumps(r, ensure_ascii=False) + "\n" for r in rows)


def fnv1a64(text):
    h = 0xcbf29ce484222325
    for b in text.encode("utf-8"):
        h = ((h ^ b) * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
    return h


def there(out):
    """Whether out exists already: then it is kept as it is (its sha still recorded)."""
    if os.path.exists(out):
        print("%s: there already, kept" % os.path.basename(out))
        record(out)
        return True
    return False


def record(path):
    """path's sha256 in inputs.sha256 beside it, as sha256sum writes it, once."""
    line = "%s *%s" % (common.sha256_file(path), os.path.basename(path))
    log = os.path.join(os.path.dirname(os.path.abspath(path)), "inputs.sha256")
    had = ""
    if os.path.exists(log):
        with open(log, encoding="utf-8") as f:
            had = f.read()
        if line in (l.strip() for l in had.split("\n")):
            return
    with open(log, "a", encoding="utf-8", newline="\n") as f:
        f.write(("\n" if had and not had.endswith("\n") else "") + line + "\n")


def held_roots(path):
    return {root(f) for f in read_lines(path)}


# --- comments -------------------------------------------------------------------------------

def scan(line, depth):
    """One line of applang as its lexer reads it: (the code, whether a comment was in it, the
    /* */ depth after it). Strings are one line, with \\-escapes; // ends the line's code; /* */
    nest; neither opens inside a string."""
    code, had, i, quoted = [], depth > 0, 0, False
    while i < len(line):
        if depth:
            if line.startswith("/*", i):
                depth, i = depth + 1, i + 2
            elif line.startswith("*/", i):
                depth, i = depth - 1, i + 2
            else:
                i += 1
            continue
        c = line[i]
        if quoted:
            n = 2 if c == "\\" else 1
            code.append(line[i:i + n])
            quoted, i = c != '"', i + n
            continue
        if line.startswith("//", i):
            had = True
            break
        if line.startswith("/*", i):
            depth, had, i = 1, True, i + 2
            continue
        quoted = c == '"'
        code.append(c)
        i += 1
    return "".join(code), had or depth > 0, depth


def strip_comments(src):
    """src without its comments: a line that was only comment goes, a comment after code goes
    with the spaces before it, and runs of blank lines (at the ends, none) become one."""
    out, depth = [], 0
    for line in src.split("\n"):
        code, had, depth = scan(line, depth)
        if had:
            code = code.rstrip()
            if not code.strip():
                continue
        out.append(code if code.strip() else "")
    kept = []
    for line in out:
        if line or (kept and kept[-1]):
            kept.append(line)
    while kept and not kept[-1]:
        kept.pop()
    return "\n".join(kept)


def comments(src):
    """How many lines of src hold a comment outside strings."""
    n, depth = 0, 0
    for line in src.split("\n"):
        _, had, depth = scan(line, depth)
        n += had
    return n


def auc(scores, labels):
    """The chance a yes scores above a no (ties half): Mann-Whitney, by mid-ranks."""
    order = sorted(range(len(scores)), key=lambda i: scores[i])
    ranks, i = [0.0] * len(scores), 0
    while i < len(order):
        j = i
        while j + 1 < len(order) and scores[order[j + 1]] == scores[order[i]]:
            j += 1
        for k in range(i, j + 1):
            ranks[order[k]] = (i + j) / 2 + 1
        i = j + 1
    pos = sum(1 for l in labels if l)
    neg = len(labels) - pos
    if not pos or not neg:
        return None
    return (sum(r for r, l in zip(ranks, labels) if l) - pos * (pos + 1) / 2) / (pos * neg)


# --- the subcommands --------------------------------------------------------------------------

def suite(a):
    need(a.suite)
    if there(a.out):
        return
    tasks = jsonl(a.suite)
    for t in tasks:
        t["check"] = WAIT_CHECK
    common.write_atomic(a.out, "".join(json.dumps(t, ensure_ascii=False, separators=(",", ":")) + "\n"
                                       for t in tasks))
    record(a.out)
    print("suite: %d tasks, each check `runs clean only`, in %s" % (len(tasks), os.path.basename(a.out)))


def pre(a):
    need(a.src)
    if there(a.out):
        return
    if not re.fullmatch(r"[0-9a-fA-F]{8,64}", a.sha):
        raise Fail("pre: --sha %r is not 8 to 64 hex digits" % a.sha)
    sha = common.sha256_file(a.src)
    if not sha.startswith(a.sha.lower()):
        raise Fail("pre: %s has sha256 %s, not %s..." % (os.path.basename(a.src), sha[:16], a.sha))
    with open(a.src, "rb") as f:
        common.write_atomic(a.out, f.read())
    record(a.out)
    print("pre: %d answers, sha256 %s" % (len(jsonl(a.out)), sha[:16]))


def dev_pick(pool, family_of, roots):
    """One mutant a task (sorted by task): from the first group of GROUPS the task has a kind of,
    the kind with the least fnv1a64(task + kind), its first record. Raises Fail on a held root."""
    by = {}
    for p in pool:
        if p.get("arm") == "repair" and "\n\n\n" not in p["program"]:
            by.setdefault(p["task"], {}).setdefault(p["kind"], p)
    out = []
    for task in sorted(by):
        r = root(family_of.get(task, task))
        if r in roots:
            raise Fail("dev: %s's root %s is held out" % (task, r))
        kinds = next((g for g in ([k for k in grp if k in by[task]] for grp in GROUPS) if g), None)
        if not kinds:
            continue
        kind = min(kinds, key=lambda k: fnv1a64(task + k))
        p = by[task][kind]
        out.append({"task": task, "model": "mut-" + kind, "kind": kind, "code": p["code"],
                    "reply": "```app\n" + p["program"].rstrip("\n") + "\n```"})
    return out


def dev(a):
    a.suite = a.suite or os.path.join(os.path.dirname(os.path.abspath(a.held)), "iq.jsonl")
    need(a.pool, a.held, a.suite)
    if there(a.out):
        return
    family_of = {t["id"]: t["family"] for t in jsonl(a.suite)}
    pool = jsonl(a.pool)
    rows = dev_pick(pool, family_of, held_roots(a.held))
    if not rows:
        raise Fail("dev: no repair problem in %s" % os.path.basename(a.pool))
    common.write_atomic(a.out, lines_of(rows))
    record(a.out)
    kinds = {}
    for r in rows:
        kinds[r["kind"]] = kinds.get(r["kind"], 0) + 1
    first = sum(n for k, n in kinds.items() if k in GROUPS[0])
    print("dev: %d problems (%d of the first kind group, %d of the second), 0 of held roots: %s" % (
        len(rows), first, len(rows) - first, ", ".join("%s %d" % kv for kv in sorted(kinds.items()))))


def judge_strip(rows, name):
    """rows with their programs' comments stripped; raises Fail if a record does not read."""
    out = []
    for i, r in enumerate(rows):
        m = JUDGE_USER.match(r["messages"][1]["content"]) if len(r["messages"]) > 1 else None
        if not m or r["messages"][1]["role"] != "user":
            raise Fail("judge: %s line %d is not judgedata's record" % (name, i + 1))
        prog = strip_comments(m.group(2)).strip("\n")
        if not prog.strip():
            raise Fail("judge: %s line %d is only comments" % (name, i + 1))
        msgs = [dict(x) for x in r["messages"]]
        msgs[1]["content"] = m.group(1) + prog + m.group(3)
        out.append(dict(r, messages=msgs))
    return out


def judge_length_auc(rows):
    """The length baseline: shorter user messages (the ask and the program) say yes."""
    return auc([-len(r["messages"][1]["content"]) for r in rows], [bool(r["label"]) for r in rows])


def judge(a):
    a.held = a.held or os.path.join(a.out, "held.txt")
    srcs = [os.path.join(a.src, n + ".jsonl") for n in JUDGE_SETS]
    outs = [os.path.join(a.out, n + "-s.jsonl") for n in JUDGE_SETS]
    need(a.held, *srcs)
    roots = held_roots(a.held)
    for name, src, out in zip(JUDGE_SETS, srcs, outs):
        if there(out):
            continue
        rows = jsonl(src)
        kept = judge_strip(rows, name)
        wrong = sum("// Wrong" in r["messages"][1]["content"] for r in kept)
        left = sum(comments(JUDGE_USER.match(r["messages"][1]["content"]).group(2)) > 0 for r in kept)
        held = sum(root(r["family"]) in roots for r in kept) if name == "judge-train" else 0
        if wrong or left or held:
            raise Fail("judge: %s keeps %d '// Wrong', %d programs with comments, %d records of held roots"
                       % (name, wrong, left, held))
        common.write_atomic(out, lines_of(kept))
        record(out)
        said = "%s: %d of %d records kept, %d yes, %d no, 0 '// Wrong', 0 comments" % (
            os.path.basename(out), len(kept), len(rows), sum(bool(r["label"]) for r in kept),
            sum(not r["label"] for r in kept))
        if name == "judge-train":
            said += ", 0 held roots"
        else:
            said += "; length AUC (shorter says yes) %.3f, %.3f before stripping" % (
                judge_length_auc(kept), judge_length_auc(rows))
        print(said)


def finish_prompts(answers, each, prompts):
    """The held prompts of the tasks each leaves at stage reply, in the prompts' order, GLM's
    notes appended to the last user message (cut from the front to fit a slot's room)."""
    stuck = {g["task"] for g in each if g.get("stage") == "reply"}
    reply = {}
    for x in answers:
        reply.setdefault(x["task"], x["reply"])
    out = []
    for p in prompts:
        if p["task"] not in stuck:
            continue
        if p["task"] not in reply:
            raise Fail("finish: no answer to %s" % p["task"])
        room = int((CTX - int(p["max_tokens"])) * CHARS_PER_TOKEN)
        msgs = [dict(m) for m in p["messages"]]
        last = max(i for i, m in enumerate(msgs) if m["role"] == "user")
        size = sum(len(m["content"]) for m in msgs) + len(NOTES) + len(WRITE_NOW)
        if size >= room:
            raise Fail("finish: %s's prompt has no room for notes (%d characters, %d fit)" % (p["task"], size, room))
        notes = reply[p["task"]][-NOTE_CHARS:]
        if size + len(notes) >= room:
            notes = notes[len(notes) - max(0, room - 1 - size):]
            print("finish: %s's notes cut to %d characters to fit" % (p["task"], len(notes)))
        msgs[last]["content"] += NOTES + notes + WRITE_NOW
        out.append(dict(p, messages=msgs))
    return out


def finish(a):
    need(a.answers, a.each, a.prompts)
    if there(a.out):
        return
    with open(a.each, encoding="utf-8") as f:   # iq score --each: a JSON line an answer, then the tally
        each = [json.loads(l) for l in f if l.startswith("{")]
    rows = finish_prompts(jsonl(a.answers), each, jsonl(a.prompts))
    if not rows:
        raise Fail("finish: %s leaves no task at stage reply" % os.path.basename(a.each))
    common.write_atomic(a.out, lines_of(rows))
    record(a.out)
    print("finish: %d prompts, the longest %d characters" % (
        len(rows), max(sum(len(m["content"]) for m in r["messages"]) for r in rows)))


def q3rec(a):
    need(a.src)
    if there(a.out):
        return
    with open(a.src, "rb") as f:
        common.write_atomic(a.out, f.read())
    record(a.out)
    print("q3rec: %d samples, sha256 %s" % (len(jsonl(a.out)), common.sha256_file(a.out)))


def readable(line):
    """The answer on line, if `iq score` can read it: a JSON object with string task, model, reply."""
    try:
        x = json.loads(line)
    except ValueError:
        return None
    if isinstance(x, dict) and all(isinstance(x.get(k), str) for k in ("task", "model", "reply")):
        return x
    return None


def choose(samples, grades):
    """Per task, in order of first appearance, the index of its sample to keep: the first that
    passes, else the one that got furthest (STAGES), the first of equals."""
    best = {}
    for i, (s, g) in enumerate(zip(samples, grades)):
        rank = STAGES.index(g["stage"]) if g["stage"] in STAGES else -1
        if s["task"] not in best or rank > best[s["task"]][0]:
            best[s["task"]] = (rank, i)
    return [i for _, i in best.values()]


def grade(iq, answers, suite_path):
    """iq score --each over answers under suite_path: a grade an answer, in order."""
    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, "answers.jsonl")
        with open(path, "w", encoding="utf-8", newline="\n") as f:
            f.write(lines_of({k: x[k] for k in ("task", "model", "reply")} for x in answers))
        r = subprocess.run([iq, "score", path, "--suite", suite_path, "--each"], capture_output=True)
    each = [json.loads(l) for l in r.stdout.decode("utf-8", "replace").split("\n") if l.startswith("{")]
    if r.returncode != 0 or len(each) != len(answers):
        raise Fail("pick: iq score failed (%d): %s" % (r.returncode, r.stderr.decode("utf-8", "replace")[:400]))
    return each


def pick(a):
    need(a.samples, a.suite_wait, a.iq)
    if os.path.exists(a.out) and os.path.getmtime(a.out) >= os.path.getmtime(a.samples):
        print("%s: there already, newer than its samples, kept" % os.path.basename(a.out))
        return
    ids = {t["id"] for t in jsonl(a.suite_wait)}
    with open(a.samples, encoding="utf-8") as f:
        read = [readable(l) for l in f if l.strip()]
    samples = [x for x in read if x is not None and x["task"] in ids]
    if not samples:
        raise Fail("pick: no sample in %s to a task of the suite" % os.path.basename(a.samples))
    grades = grade(a.iq, samples, a.suite_wait)
    nth, of = [], {}   # each sample's place among its task's, and how many its task has
    for s in samples:
        nth.append(of.get(s["task"], 0))
        of[s["task"]] = nth[-1] + 1
    out, stages = [], {}
    for i in choose(samples, grades):
        g = grades[i]
        out.append(dict(samples[i], pick={"sample": nth[i], "of": of[samples[i]["task"]],
                                          "stage": g["stage"], "code": g["code"]}))
        stages[g["stage"]] = stages.get(g["stage"], 0) + 1
    common.write_atomic(a.out, lines_of(out))
    print("pick: %d tasks from %d samples (%d unreadable or not in the suite): %s" % (
        len(out), len(samples), len(read) - len(samples),
        ", ".join("%s %d" % (s, stages[s]) for s in reversed(STAGES) if s in stages)))


def clean(a):
    need(a.inp)
    kept, dropped = [], 0
    with open(a.inp, encoding="utf-8", errors="replace") as f:
        for line in f:
            if readable(line) is None:
                dropped += bool(line.strip())
            else:
                kept.append(line.rstrip("\r\n") + "\n")
    if os.path.abspath(a.inp) == os.path.abspath(a.out) and not dropped:
        print("clean: %d lines, none dropped" % len(kept))
        return
    common.write_atomic(a.out, "".join(kept))
    print("clean: %d lines kept, %d dropped (unreadable) from %s" % (len(kept), dropped, os.path.basename(a.inp)))


def parse_args(argv=None):
    h = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = h.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("suite", help="the suite with every check `runs clean only`")
    s.add_argument("--suite", required=True)
    s.add_argument("--out", required=True)
    s = sub.add_parser("pre", help="the harness's answers, copied if their sha matches")
    s.add_argument("--src", required=True)
    s.add_argument("--sha", required=True, help="the sha256's first hex digits")
    s.add_argument("--out", required=True)
    s = sub.add_parser("dev", help="one mutant of each dev task's reference to repair")
    s.add_argument("--pool", required=True)
    s.add_argument("--held", required=True)
    s.add_argument("--suite", help="maps tasks to families (default: iq.jsonl beside --held)")
    s.add_argument("--out", required=True)
    s = sub.add_parser("judge", help="the judge sets with their programs' comments stripped")
    s.add_argument("--src", required=True)
    s.add_argument("--out", required=True)
    s.add_argument("--held", help="held families (default: --out's held.txt)")
    s = sub.add_parser("finish", help="the unfinished replies' prompts, with GLM's notes")
    s.add_argument("--answers", required=True)
    s.add_argument("--each", required=True)
    s.add_argument("--prompts", required=True)
    s.add_argument("--out", required=True)
    s = sub.add_parser("q3rec", help="the recorded q3 samples, copied")
    s.add_argument("--src", required=True)
    s.add_argument("--out", required=True)
    s = sub.add_parser("pick", help="one sample a task, by suite-wait")
    s.add_argument("--samples", required=True)
    s.add_argument("--suite-wait", required=True)
    s.add_argument("--iq", required=True)
    s.add_argument("--out", required=True)
    s = sub.add_parser("clean", help="an answers file without the lines iq cannot read")
    s.add_argument("inp", metavar="IN")
    s.add_argument("out", metavar="OUT")
    return h.parse_args(argv)


def main(argv=None):
    common.utf8_stdio()
    a = parse_args(argv)
    try:
        globals()[a.cmd](a)
    except Fail as e:
        print("nightprep: %s" % e, file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
