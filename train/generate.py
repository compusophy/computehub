#!/usr/bin/env python
"""Answer `teach prompts` lines with a local model on the GPU, in batches, for `iq score`.

  python train/generate.py --prompts P.jsonl --out answers.jsonl (--base ID | --run NAME)
                           [--k 1] [--batch 8] [--seed 1] [--name MODEL] [--root DIR]

Each prompt line holds the messages Studio sends for a new app (the coder's system prompt and
first turn), the room it asks for (max_tokens) and its temperature. They are rendered as token
ids by the model's own chat template with the generation prompt, exactly as sft.py renders a
record's prompt, so a model is measured on the input it was trained on. Each task is sampled k
times at its temperature (top_p 1, seeded: seed + sample index), and every sample is one
{"task","model","reply"} line, the shape `iq score` reads. A run's model is its base with its
LoRA adapter folded in, or its model/ (--full), from the data root; --base is an untouched base
model from the local Hugging Face cache, for a baseline.
"""
import argparse
import json
import os
import sys
import time

import torch

import common


def parse_args():
    h = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    h.add_argument("--prompts", required=True, help="teach prompts' output")
    h.add_argument("--out", required=True, help="answers, written afresh")
    g = h.add_mutually_exclusive_group(required=True)
    g.add_argument("--base", help="a base model id in the local HF cache (a baseline)")
    g.add_argument("--run", help="a training run's name under the data root")
    h.add_argument("--k", type=int, default=1, help="samples a task")
    h.add_argument("--batch", type=int, default=8, help="sequences generated at once")
    h.add_argument("--seed", type=int, default=1)
    h.add_argument("--name", help="the model's name in the answers (default: the base id or run:NAME)")
    h.add_argument("--root", help="data root (default: COMPUTEHUB_DATA or C:\\sept30\\computehub-data)")
    return h.parse_args()


def load(a, root, log):
    from transformers import AutoModelForCausalLM, AutoTokenizer
    t = time.time()
    if a.base:
        base = common.resolve_base(a.base)
        path, name = base["path"], a.name or a.base
        model = AutoModelForCausalLM.from_pretrained(path, dtype=torch.bfloat16, attn_implementation="sdpa")
    else:
        rdir = common.run_dir(root, a.run)
        m = common.read_json(os.path.join(rdir, "manifest.json"))
        if not m or m.get("status") not in (None, "done"):
            sys.exit("error: run %s has no finished manifest under %s" % (a.run, rdir))
        name = a.name or "run:" + a.run
        full, adapter = os.path.join(rdir, "model"), os.path.join(rdir, "adapter")
        if os.path.isdir(full):
            path = full
            model = AutoModelForCausalLM.from_pretrained(full, dtype=torch.bfloat16, attn_implementation="sdpa")
        elif os.path.isdir(adapter):
            from peft import PeftModel
            path = adapter
            model = AutoModelForCausalLM.from_pretrained(m["base"]["path"], dtype=torch.bfloat16,
                                                         attn_implementation="sdpa")
            model = PeftModel.from_pretrained(model, adapter).merge_and_unload()
        else:
            sys.exit("error: run %s has neither model/ nor adapter/" % a.run)
    tok = AutoTokenizer.from_pretrained(path)
    tok.padding_side = "left"
    if tok.pad_token_id is None:
        tok.pad_token = tok.eos_token
    model = model.cuda().eval()
    model.generation_config.pad_token_id = tok.pad_token_id
    log("loaded %s in %.1fs" % (name, time.time() - t))
    return model, tok, name


def read_prompts(path):
    rows = []
    with open(path, encoding="utf-8") as f:
        for i, line in enumerate(f, 1):
            if not line.strip():
                continue
            r = json.loads(line)
            for k in ("task", "messages", "max_tokens", "temperature"):
                if k not in r:
                    sys.exit("error: %s line %d has no %s" % (path, i, k))
            rows.append(r)
    return rows


def main():
    common.utf8_stdio()
    a = parse_args()
    root = common.data_root(a.root)
    log = print
    rows = read_prompts(a.prompts)
    model, tok, name = load(a, root, log)
    for r in rows:
        r["ids"] = tok.apply_chat_template(r["messages"], tokenize=True, add_generation_prompt=True)
    # Like with like: one settings group at a time, its prompts by length, so little padding
    # is wasted and a batch never mixes temperatures or rooms.
    groups = {}
    for r in rows:
        groups.setdefault((float(r["temperature"]), int(r["max_tokens"])), []).append(r)
    per = max(1, a.batch // a.k)
    out, made, done, t0 = [], 0, 0, time.time()
    for (temp, room), group in sorted(groups.items()):
        group.sort(key=lambda r: len(r["ids"]))
        for at in range(0, len(group), per):
            chunk = group[at:at + per]
            width = max(len(r["ids"]) for r in chunk)
            pad = [width - len(r["ids"]) for r in chunk]
            ids = torch.tensor([[tok.pad_token_id] * p + r["ids"] for p, r in zip(pad, chunk)]).cuda()
            mask = torch.tensor([[0] * p + [1] * len(r["ids"]) for p, r in zip(pad, chunk)]).cuda()
            torch.manual_seed(a.seed + done)
            sample = {"do_sample": True, "temperature": temp, "top_p": 1.0, "top_k": 0} if temp > 0 else                 {"do_sample": False}
            with torch.no_grad():
                gen = model.generate(input_ids=ids, attention_mask=mask, max_new_tokens=room,
                                     num_return_sequences=a.k, **sample)
            for j, seq in enumerate(gen):
                new = seq[width:]
                made += int((new != tok.pad_token_id).sum())
                reply = tok.decode(new, skip_special_tokens=True)
                out.append(json.dumps({"task": chunk[j // a.k]["task"], "model": name, "reply": reply},
                                      ensure_ascii=False))
            done += len(chunk)
            log("%d/%d tasks, %d tokens, %.0f tokens/s" % (done, len(rows), made, made / (time.time() - t0)))
    out.sort()
    common.write_atomic(a.out, ("\n".join(out) + "\n").encode("utf-8"))
    log("wrote %d answers to %s in %.0fs" % (len(out), a.out, time.time() - t0))


if __name__ == "__main__":
    main()
