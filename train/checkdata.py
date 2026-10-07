#!/usr/bin/env python
"""The check-first skill's data: from the IQ suite, a model learns to write a task's check from its
ask alone, before any program, so that among its program samples the one passing its own check
can be chosen (DESIGN.md, Evolution, "Prediction first"; CodeT, Chen et al. 2023).

  python train/checkdata.py --iq IQ.exe --suite S --held-prompts prompts-held.jsonl
                            --train-out checks.jsonl --held-out prompts-held-checks.jsonl [--day D]

Every task not held out becomes a record {messages: [system, user, assistant], task, family,
kind: "check", by}: the system prompt is iq's own card of the check language (`iq card`) and how
to answer, the user's message the ask, the assistant's the check in a ```check block. The
held-out tasks become prompts in `teach prompts`' form, for train/ask.py; train/select.py reads
what the model writes for them.
"""
import argparse
import hashlib
import json
import subprocess

import common

HOW = """

You write the check for an app request, before any program exists: a script in this language
that every app doing what the request asks passes, whatever its design, and an app that does not
do it fails. Test only what the request promises: the labels, buttons and numbers it names, what
each action must change; where it leaves a word or a layout open, accept the likely ones ("Add|+")
or test the behavior instead. Reply with the script alone, in one fenced block:
```check
...
```"""


def system_prompt(iq):
    card = subprocess.run([iq, "card"], capture_output=True, check=True).stdout.decode("utf-8")
    return card.rstrip("\n") + HOW


def main():
    common.utf8_stdio()
    h = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    h.add_argument("--iq", required=True)
    h.add_argument("--suite", required=True)
    h.add_argument("--held-prompts", required=True, help="teach prompts --split held: which tasks are held out")
    h.add_argument("--train-out", required=True)
    h.add_argument("--held-out", required=True)
    h.add_argument("--day", default="")
    h.add_argument("--temperature", type=float, default=0.3)
    h.add_argument("--max-tokens", type=int, default=2048)
    a = h.parse_args()
    system = system_prompt(a.iq)
    prompt_id = hashlib.sha256(system.encode("utf-8")).hexdigest()[:16]
    held = {json.loads(l)["task"] for l in open(a.held_prompts, encoding="utf-8") if l.strip()}
    tasks = [json.loads(l) for l in open(a.suite, encoding="utf-8") if l.strip()]
    train, prompts = [], []
    for t in tasks:
        user = "Request: " + t["ask"]
        if t["id"] in held:
            prompts.append({"task": t["id"], "family": t["family"], "max_tokens": a.max_tokens,
                            "temperature": a.temperature,
                            "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}]})
            continue
        by = t.get("by") or {}
        train.append({"messages": [{"role": "system", "content": system}, {"role": "user", "content": user},
                                   {"role": "assistant", "content": "```check\n" + t["check"].rstrip("\n") + "\n```"}],
                      "task": t["id"], "family": t["family"], "kind": "check",
                      "by": {"teacher": by.get("teacher", ""), "prompt": prompt_id,
                             "verifier": by.get("verifier", ""), "day": a.day or by.get("day", "")}})
    common.write_atomic(a.train_out, "".join(json.dumps(r, ensure_ascii=False) + "\n" for r in train))
    common.write_atomic(a.held_out, "".join(json.dumps(r, ensure_ascii=False) + "\n" for r in prompts))
    print("checks: %d training records, %d held-out prompts (system prompt %s, %d chars)" % (
        len(train), len(prompts), prompt_id, len(system)))


if __name__ == "__main__":
    main()
