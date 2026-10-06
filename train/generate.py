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

A sample stops where Studio stops reading: at its first closed ```app block that begins with a
comment (coder::edits::program), or at its end-of-turn token; its reply ends there, so the
grade is the same as on the whole reply, and a batch no longer waits on samples drafting past
their program. Answers already in --out are kept and their tasks skipped (a baseline is
answered once, ever; a run cut off by a freeze resumes), and --out is rewritten after every
batch.
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
    h.add_argument("--out", required=True, help="answers; tasks already answered there are skipped")
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
    # The base model's tokenizer, as sft.py trained with: a fine-tune's saved copy can load with a
    # different pre-tokenizer (transformers warns of an "incorrect regex pattern"), which would
    # hand the model token ids it never saw in training.
    tok = AutoTokenizer.from_pretrained(path if a.base else m["base"]["path"])
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


def block_end(text):
    """Where the first closed ```app block of text that begins with a comment ends (just past
    its closing fence), or None: the program Studio's coder takes (coder::edits::program)."""
    at, open_at, body = 0, None, []
    for line in text.splitlines(keepends=True):
        t = line.strip()
        if open_at is None:
            if t == "```app":
                open_at, body = at, []
        elif t.startswith("```"):
            first = next((b.strip() for b in body if b.strip()), "")
            if first.startswith("//") or first.startswith("/*"):
                return at + len(line.rstrip("\r\n"))
            open_at = None
        else:
            body.append(line)
        at += len(line)
    return None


class BlockStop:
    """Stops each sequence of a batch once its first program block closes (decoding only what
    is new each step)."""

    def __init__(self, tok, width, rows):
        self.tok, self.width = tok, width
        self.text, self.seen = [""] * rows, [0] * rows
        self.stopped = torch.zeros(rows, dtype=torch.bool)

    def __call__(self, input_ids, scores, **kwargs):
        for i in range(input_ids.shape[0]):
            if self.stopped[i]:
                continue
            new = input_ids[i, self.width + self.seen[i]:]
            if len(new):
                self.text[i] += self.tok.decode(new, skip_special_tokens=True)
                self.seen[i] += len(new)
                if block_end(self.text[i]) is not None:
                    self.stopped[i] = True
        return self.stopped.to(input_ids.device)


def read_done(path):
    """The answer lines already in path, and how many each task has."""
    lines, count = [], {}
    if os.path.exists(path):
        with open(path, encoding="utf-8") as f:
            for line in f:
                if line.strip():
                    lines.append(line.rstrip("\n"))
                    t = json.loads(line)["task"]
                    count[t] = count.get(t, 0) + 1
    return lines, count


def main():
    common.utf8_stdio()
    a = parse_args()
    root = common.data_root(a.root)
    log = print
    out, have = read_done(a.out)
    rows = [r for r in read_prompts(a.prompts) if have.get(r["task"], 0) < a.k]
    if not rows:
        log("every task of %s is answered in %s already" % (a.prompts, a.out))
        return
    model, tok, name = load(a, root, log)
    for r in rows:
        r["ids"] = tok.apply_chat_template(r["messages"], tokenize=True, add_generation_prompt=True)
    # Like with like: one settings group at a time, its prompts by length, so little padding
    # is wasted and a batch never mixes temperatures or rooms.
    groups = {}
    for r in rows:
        groups.setdefault((float(r["temperature"]), int(r["max_tokens"])), []).append(r)
    per = max(1, a.batch // a.k)
    made, done, t0 = 0, 0, time.time()
    from transformers import StoppingCriteriaList
    for (temp, room), group in sorted(groups.items()):
        group.sort(key=lambda r: len(r["ids"]))
        for at in range(0, len(group), per):
            chunk = group[at:at + per]
            width = max(len(r["ids"]) for r in chunk)
            pad = [width - len(r["ids"]) for r in chunk]
            ids = torch.tensor([[tok.pad_token_id] * p + r["ids"] for p, r in zip(pad, chunk)]).cuda()
            mask = torch.tensor([[0] * p + [1] * len(r["ids"]) for p, r in zip(pad, chunk)]).cuda()
            torch.manual_seed(a.seed + done)
            if temp > 0:
                sample = {"do_sample": True, "temperature": temp, "top_p": 1.0, "top_k": 0}
            else:
                sample = {"do_sample": False}
            stop = BlockStop(tok, width, len(chunk) * a.k)
            with torch.no_grad():
                gen = model.generate(input_ids=ids, attention_mask=mask, max_new_tokens=room,
                                     num_return_sequences=a.k, stopping_criteria=StoppingCriteriaList([stop]),
                                     **sample)
            for j, seq in enumerate(gen):
                new = seq[width:]
                made += int((new != tok.pad_token_id).sum())
                reply = tok.decode(new, skip_special_tokens=True)
                end = block_end(reply)
                reply = reply if end is None else reply[:end]
                out.append(json.dumps({"task": chunk[j // a.k]["task"], "model": name, "reply": reply},
                                      ensure_ascii=False))
            done += len(chunk)
            out.sort()
            common.write_atomic(a.out, ("\n".join(out) + "\n").encode("utf-8"))
            log("%d/%d tasks, %d tokens, %.0f tokens/s" % (done, len(rows), made, made / (time.time() - t0)))
    log("%d answers in %s; this run %.0fs" % (len(out), a.out, time.time() - t0))


if __name__ == "__main__":
    main()
