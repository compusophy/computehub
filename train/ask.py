#!/usr/bin/env python
"""Answer `teach prompts` lines through a llama-server (train/serve.py), many at a time, for
`iq score`: the fast path of a night's scoring, where generate.py's batches wait on their
longest sample.

  python train/ask.py --prompts P.jsonl --out answers.jsonl --name MODEL
                      [--url http://127.0.0.1:8081] [--k 4] [--jobs 16] [--seed 1]

The server renders each prompt's messages by the model's own chat template (serve.py checks it
renders them as the Hugging Face tokenizer does, which training used) and samples at the
prompt's temperature with every other sampler off (top_k 0, top_p 1, min_p 0, repeat penalty
1), up to its room; its prompt cache reuses the coder's long system prompt across requests.
Each reply streams in and is closed where Studio stops reading (blocks.block_end: its first
closed app block that begins with a comment); the server stops a request whose client is gone,
so a slot never drafts past a program. Every answer records how it was made ("gen", with the
engine), and answers in --out made the same way are kept and their tasks skipped, so a run cut
off by a freeze resumes. --out is rewritten after every 8 answers.
"""
import argparse
import json
import os
import sys
import threading
import time
import urllib.request
from concurrent.futures import ThreadPoolExecutor

import common
from blocks import block_end

ENGINE = "llama.cpp"


def parse_args():
    h = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    h.add_argument("--prompts", required=True)
    h.add_argument("--out", required=True)
    h.add_argument("--name", required=True, help="the model's name in the answers")
    h.add_argument("--url", default="http://127.0.0.1:8081")
    h.add_argument("--k", type=int, default=4)
    h.add_argument("--jobs", type=int, default=16, help="requests at once (serve.py's --parallel)")
    h.add_argument("--seed", type=int, default=1)
    h.add_argument("--grammar", help="a GBNF file: the server samples only what it accepts (train/applang.gbnf)")
    return h.parse_args()


def settings(r):
    return {"temperature": float(r["temperature"]), "top_p": 1.0, "top_k": 0, "repetition_penalty": 1.0,
            "max_new_tokens": int(r["max_tokens"]), "engine": ENGINE, "prompt": common.prompt_hash(r["messages"])}


def ask(url, r, seed, grammar=None):
    """One sample of r: its reply, cut where Studio stops reading."""
    body = {"messages": r["messages"], "max_tokens": int(r["max_tokens"]), "temperature": float(r["temperature"]),
            "top_p": 1.0, "top_k": 0, "min_p": 0.0, "repeat_penalty": 1.0, "seed": seed, "stream": True,
            "cache_prompt": True}
    if grammar:
        body["grammar"] = grammar
    req = urllib.request.Request(url + "/v1/chat/completions", data=json.dumps(body).encode("utf-8"),
                                 headers={"content-type": "application/json"})
    text = ""
    with urllib.request.urlopen(req, timeout=3600) as resp:
        for raw in resp:
            line = raw.decode("utf-8", "replace").strip()
            if not line.startswith("data:"):
                continue
            data = line[5:].strip()
            if data == "[DONE]":
                break
            delta = json.loads(data)["choices"][0].get("delta", {}).get("content") or ""
            text += delta
            if "```" in delta or text.count("```") >= 2:
                end = block_end(text)
                if end is not None:
                    return text[:end]   # closing the stream cancels the rest on the server
    end = block_end(text)
    return text if end is None else text[:end]


def main():
    common.utf8_stdio()
    a = parse_args()
    prompts = [json.loads(l) for l in open(a.prompts, encoding="utf-8") if l.strip()]
    want = {r["task"]: settings(r) for r in prompts}
    if a.grammar:  # an answer under a grammar is another way of making it
        g = common.sha256_text(open(a.grammar, encoding="utf-8").read())[:16]
        for v in want.values():
            v["grammar"] = g
    kept, have = [], {}
    if os.path.exists(a.out):
        for line in open(a.out, encoding="utf-8"):
            if line.strip():
                x = json.loads(line)
                # Only answers to these prompts, made the same way: an old line with no "gen" for
                # a task they lack (want.get gives None too) would otherwise be kept forever.
                if x["task"] in want and x.get("gen") == want[x["task"]]:
                    kept.append(line.rstrip("\n"))
                    have[x["task"]] = have.get(x["task"], 0) + 1
    # A task short of its k is answered again whole: its old lines go.
    short = {r["task"] for r in prompts if have.get(r["task"], 0) < a.k}
    kept = [l for l in kept if json.loads(l)["task"] not in short]
    jobs = [(i, s, r) for i, r in enumerate(prompts) if r["task"] in short for s in range(a.k)]
    if not jobs:
        print("every task of %s is answered in %s already" % (a.prompts, a.out))
        return
    grammar = open(a.grammar, encoding="utf-8").read() if a.grammar else None
    lock, out, t0 = threading.Lock(), list(kept), time.time()

    def one(job):
        i, s, r = job
        reply = ask(a.url, r, a.seed + i * 1000 + s, grammar)
        line = json.dumps({"task": r["task"], "model": a.name, "reply": reply, "gen": want[r["task"]]},
                          ensure_ascii=False)
        with lock:
            out.append(line)
            n = len(out) - len(kept)
            if n % 8 == 0 or n == len(jobs):
                common.write_atomic(a.out, "\n".join(sorted(out)) + "\n")
                print("%d/%d answers, %.0fs" % (n, len(jobs), time.time() - t0), flush=True)

    with ThreadPoolExecutor(max_workers=a.jobs) as pool:
        for f in [pool.submit(one, j) for j in jobs]:
            try:
                f.result()
            except Exception as e:  # one request's failure (the server gone, a timeout) is reported, not fatal
                print("error: %s" % e, file=sys.stderr)
    common.write_atomic(a.out, "\n".join(sorted(out)) + "\n")
    print("%d answers in %s; this run %.0fs" % (len(out), a.out, time.time() - t0))


if __name__ == "__main__":
    main()
