#!/usr/bin/env python
"""The fix skill's data: a helper that repairs the cloud model's drafts (DESIGN.md, "The team")
learns from the turns that repaired one. tools/eval `team --traces` writes every helper turn:
the request as it was sent (`body`: the system prompt and the fix asked, with the fault's
account), the reply, what it held (a program, edit blocks, a unified diff read as edit blocks,
blocks that missed, nothing) and the program's problem before and after (0: it runs clean).

  python train/fixdata.py --traces traces.jsonl [--traces more.jsonl] --held held.txt
                          --suite S --out fix.jsonl [--night N]

A turn becomes a record {messages: [system, user, assistant], task, family, kind: "fix", by} when
it took a program that did not run clean to one that does, on a task whose family is not held
out (sft.py refuses those too). The assistant's message is the reply in the form the make loop
reads: a diff's hunks rewritten as SEARCH/REPLACE blocks (team's own reading of them), else the
reply as it came. A task's turn written twice (a rerun) counts once, its last writing.
"""
import argparse
import json

import sftdata

FENCES = ("```diff", "```patch")


def header(line):
    return line.startswith(("diff --git", "index ", "--- ", "+++ "))


def unnumbered(line):
    t = line.lstrip()
    digits = len(t) - len(t.lstrip("0123456789"))
    if digits and t[digits:digits + 1] == "|":
        return t[digits + 1:]
    return line


def hunk_block(hunk):
    """One hunk as a SEARCH/REPLACE block (programs/team/src/diff.rs, in Python), or None."""
    search, replace, changed = [], [], False
    for raw in hunk:
        line = unnumbered(raw)
        if line.startswith("-"):
            changed = True
            search.append(line[1:])
        elif line.startswith("+"):
            changed = True
            replace.append(line[1:])
        else:
            same = line[1:] if line.startswith(" ") else line
            search.append(same)
            replace.append(same)

    def trim(v):
        while v and not v[0].strip():
            v.pop(0)
        while v and not v[-1].strip():
            v.pop()

    trim(search)
    trim(replace)
    if not changed or not search:
        return None
    if all(not unnumbered(l).strip() or unnumbered(l).startswith(("- ", "+ ")) for l in hunk):
        search = [l[1:] if l.startswith(" ") else l for l in search]
        replace = [l[1:] if l.startswith(" ") else l for l in replace]
    return "<<<<<<< SEARCH\n" + "".join(l + "\n" for l in search) + "=======\n" \
        + "".join(l + "\n" for l in replace) + ">>>>>>> REPLACE\n"


def blocks(reply):
    """The edit blocks a reply's diff fences mean, or None."""
    out, lines, i = [], reply.split("\n"), 0
    while i < len(lines):
        if lines[i].strip() not in FENCES:
            i += 1
            continue
        i += 1
        hunk = []
        while i < len(lines) and not lines[i].lstrip().startswith("```"):
            line = lines[i]
            if line.startswith("@@"):
                b = hunk_block(hunk)
                if b:
                    out.append(b)
                hunk = []
            elif not header(line):
                hunk.append(line)
            i += 1
        b = hunk_block(hunk)
        if b:
            out.append(b)
        i += 1
    return "".join(out) or None


def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--traces", action="append", required=True)
    p.add_argument("--held", required=True, help="the held-out families, one a line")
    p.add_argument("--suite", required=True, help="the IQ suite (each task's family)")
    p.add_argument("--out", required=True)
    p.add_argument("--night", default="")
    p.add_argument("--helper", default="base3b", help="the helper whose turns these are")
    a = p.parse_args()

    family = {}
    for line in sftdata.read_lines(a.suite):
        t = json.loads(line)
        family[t["id"]] = t.get("family", "")
    held = {sftdata.root(l.strip()) for l in sftdata.read_lines(a.held) if l.strip() and not l.startswith("#")}

    turns = {}
    for path in a.traces:
        for line in sftdata.read_lines(path):
            t = json.loads(line)
            turns[(t["task"], t["turn"])] = t
    counts = {"turns": len(turns), "held": 0, "no body": 0, "not fixed": 0, "kept": 0, "diff": 0}
    out = []
    for (task, turn), t in sorted(turns.items()):
        fam = family.get(task, "")
        if not fam or sftdata.root(fam) in held:
            counts["held"] += 1
            continue
        if "body" not in t:
            counts["no body"] += 1
            continue
        if t["after"] != 0 or t["before"] == 0 or t["held"] not in ("Program", "Edits", "Diff"):
            counts["not fixed"] += 1
            continue
        messages = [m for m in json.loads(t["body"])["messages"] if m.get("role") in ("system", "user", "assistant")]
        reply = t["reply"]
        if t["held"] == "Diff":
            reply = blocks(reply) or reply
            counts["diff"] += 1
        messages.append({"role": "assistant", "content": reply})
        out.append({"messages": messages, "task": task, "family": fam, "kind": "fix",
                    "by": {"helper": a.helper, "night": a.night, "turn": turn, "held": t["held"],
                           "before": t["before"]}})
        counts["kept"] += 1
    with open(a.out, "w", encoding="utf-8", newline="\n") as f:
        for r in out:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    print("fixdata: %s" % json.dumps(counts))


if __name__ == "__main__":
    main()
