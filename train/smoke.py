#!/usr/bin/env python3
"""Write a tiny smoke-test SFT set under <root>/smoke/: the reference apps
(programs/makes/refs/*.app) as the assistant's answers, under system and user
text marked SMOKE (placeholders: the real records come from `teach export`).
A few records are "fix" conversations, so the loss mask meets several turns.
Also writes a task suite (tasks.jsonl) and a held-out family list (held.txt)
so the smoke run exercises the held-out refusal.

    python train/smoke.py [--root DIR]
"""
import argparse
import datetime
import glob
import json
import os

import common

HELD = ["tetris", "minesweeper"]
FIXES = ["counter", "tip", "dice", "tetris"]
SYSTEM = "SMOKE placeholder system prompt. Write the app in applang. Answer with the program only."


def main():
    common.utf8_stdio()
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--root")
    a = p.parse_args()
    out = os.path.join(common.data_root(a.root), "smoke")
    os.makedirs(out, exist_ok=True)
    refs = sorted(glob.glob(os.path.join(common.REPO, "programs", "makes", "refs", "*.app")))
    if not refs:
        raise SystemExit("error: no programs/makes/refs/*.app")
    day = datetime.date.today().isoformat()
    by = {"teacher": "smoke", "prompt": "0" * 16, "verifier": "0" * 16, "day": day}
    records, tasks = [], []
    for path in refs:
        name = os.path.splitext(os.path.basename(path))[0]
        with open(path, encoding="utf-8") as f:
            app = f.read().strip() + "\n"
        task = "smoke-" + name
        tasks.append({"id": task, "family": name})
        ask = "SMOKE placeholder task: make the %s app." % name
        records.append({"messages": [{"role": "system", "content": SYSTEM}, {"role": "user", "content": ask},
                                     {"role": "assistant", "content": app}],
                        "task": task, "kind": "write", "by": by})
        if name in FIXES:
            draft = "// SMOKE placeholder: a draft that does not compile\n" + app.splitlines()[-1] + "\n"
            records.append({"messages": [{"role": "system", "content": SYSTEM}, {"role": "user", "content": ask},
                                         {"role": "assistant", "content": draft},
                                         {"role": "user", "content": "SMOKE placeholder: it does not compile; fix it."},
                                         {"role": "assistant", "content": app}],
                            "task": task, "kind": "fix", "by": by})
    files = {"sft.jsonl": "".join(json.dumps(r) + "\n" for r in records),
             "tasks.jsonl": "".join(json.dumps(t) + "\n" for t in tasks),
             "held.txt": "# SMOKE held-out families\n" + "".join(h + "\n" for h in HELD)}
    for name, text in files.items():
        common.write_atomic(os.path.join(out, name), text)
    print("wrote %d records (%d tasks, %d held-out families) to %s" % (len(records), len(tasks), len(HELD), out))
    print("train: python train/sft.py --data %s --tasks %s --held %s --run smoke --max-steps 30" % (
        os.path.join(out, "sft.jsonl"), os.path.join(out, "tasks.jsonl"), os.path.join(out, "held.txt")))


if __name__ == "__main__":
    main()
