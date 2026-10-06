#!/usr/bin/env python
"""Answer `teach prompts` lines with a local model on the GPU, in batches, for `iq score`.

  python train/generate.py --prompts P.jsonl --out answers.jsonl (--base ID | --run NAME)
                           [--k 1] [--batch 8] [--seed 1] [--name MODEL] [--root DIR]

Each prompt line holds the messages Studio sends for a new app (the coder's system prompt and
first turn), the room it asks for (max_tokens) and its temperature. They are rendered as token
ids by the model's own chat template with the generation prompt, exactly as sft.py renders a
record's prompt, so a model is measured on the input it was trained on. A run's model is its
base with its LoRA adapter folded in, or its model/ (--full), from the data root; --base is an
untouched base model from the local Hugging Face cache, for a baseline.

The sampler is what Studio asks for, a temperature and nothing else: top_p 1, top_k 0 and a
repetition penalty of 1 are passed, since the Qwen bases' generation_config.json sets top_p 0.8,
top_k 20 and a repetition penalty of 1.05 (which weighs down every token of the prompt, the
applang reference with it), and a fine-tune's saved config carries them on. A task is sampled
k times; at temperature 0 it is decoded greedily, once, as every sample would be the same. Each
sample is one {"task","model","reply","gen"} line, the shape `iq score` reads, "gen" holding its
settings (temperature, top_p, top_k, repetition_penalty, max_new_tokens). Each batch is seeded
with --seed plus the line number in --prompts of its first task, and its samples share that one
random stream: a run resumed after a freeze draws what the uninterrupted run would have
wherever its batches come out the same (the same --batch, --k and prompts, and as much free GPU
memory, which caps a batch).

A sample stops where Studio stops reading (blocks.py): at its first closed ```app block without
edit markers that begins with a comment, or at its end-of-turn token; its reply ends there, so
the grade is the same as on the whole reply, and a batch no longer waits on samples drafting
past their program. Answers already in --out made with this run's settings are kept and their
tasks skipped (a baseline is answered once, ever; a run cut off by a freeze resumes); a task
with fewer such answers than it needs (--k was raised, or they were made with other settings or
before answers recorded theirs) is answered again whole, replacing what it had. --out is
rewritten after every batch.
"""
import argparse
import json
import os
import sys
import time

import torch

import common
from blocks import block_end

# Neutral, so only the temperature shapes a sample (see above).
SAMPLER = {"top_p": 1.0, "top_k": 0, "repetition_penalty": 1.0}


def parse_args():
    h = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    h.add_argument("--prompts", required=True, help="teach prompts' output")
    h.add_argument("--out", required=True, help="answers; tasks already answered there are skipped")
    g = h.add_mutually_exclusive_group(required=True)
    g.add_argument("--base", help="a base model id in the local HF cache (a baseline)")
    g.add_argument("--run", help="a training run's name under the data root")
    h.add_argument("--k", type=int, default=1, help="samples a task (1 at temperature 0: greedy)")
    h.add_argument("--batch", type=int, default=8, help="sequences generated at once")
    h.add_argument("--seed", type=int, default=1)
    h.add_argument("--name", help="the model's name in the answers (default: the base id or run:NAME)")
    h.add_argument("--root", help="data root (default: COMPUTEHUB_DATA or C:\\sept30\\computehub-data)")
    a = h.parse_args()
    if a.k < 1:
        h.error("--k is at least 1")
    return a


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
            r["line"] = i
            rows.append(r)
    return rows


def samples(temp, k):
    """How many answers a task at temp gets: k, or 1 when greedy (all k would be the same)."""
    return k if temp > 0 else 1


def settings(r):
    """What a prompt's samples are made with, as each of its answers records it ("gen")."""
    return {"temperature": float(r["temperature"]), "top_p": SAMPLER["top_p"], "top_k": SAMPLER["top_k"],
            "repetition_penalty": SAMPLER["repetition_penalty"], "max_new_tokens": int(r["max_tokens"])}


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
            if not len(new):
                continue
            text = self.tok.decode(new, skip_special_tokens=True)
            # A character split across tokens decodes as U+FFFD until its last byte comes (four
            # at most), and the reply is decoded whole: read it as that will.
            if text.endswith("\ufffd") and len(new) < 4:
                continue
            self.text[i] += text
            self.seen[i] += len(new)
            if block_end(self.text[i]) is not None:
                self.stopped[i] = True
        return self.stopped.to(input_ids.device)


def read_done(path):
    """The answer lines already in path, each as (task, its settings or None, the line)."""
    out = []
    if os.path.exists(path):
        with open(path, encoding="utf-8") as f:
            for line in f:
                if line.strip():
                    r = json.loads(line)
                    out.append((r["task"], r.get("gen"), line.rstrip("\n")))
    return out


def main():
    common.utf8_stdio()
    a = parse_args()
    root = common.data_root(a.root)
    log = print
    prompts = read_prompts(a.prompts)
    want = {r["task"]: settings(r) for r in prompts}
    out = read_done(a.out)
    # An answer counts only if made as this run would make it: one with other settings, or none
    # recorded (made before the sampler was neutral), is answered again, so a file's answers and
    # the runs beside it come from one sampler.
    have, stale = {}, 0
    for task, gen, _ in out:
        if gen == want.get(task):
            have[task] = have.get(task, 0) + 1
        elif task in want:
            stale += 1
    rows = [r for r in prompts if have.get(r["task"], 0) < samples(float(r["temperature"]), a.k)]
    if not rows:
        log("every task of %s is answered in %s already" % (a.prompts, a.out))
        return
    if stale:
        log("%d answers in %s were made with other settings: their tasks are answered again" % (stale, a.out))
    model, tok, name = load(a, root, log)
    for r in rows:
        r["ids"] = tok.apply_chat_template(r["messages"], tokenize=True, add_generation_prompt=True)
    # Like with like: one settings group at a time, its prompts by length, so little padding
    # is wasted and a batch never mixes temperatures or rooms.
    groups = {}
    for r in rows:
        groups.setdefault((float(r["temperature"]), int(r["max_tokens"])), []).append(r)
    # Sequences a batch, at most what the GPU holds: the KV cache a token (layers x kv heads x
    # head size x k and v x 2 bytes) over the longest prompt plus the room, in a third of what
    # is free (the cache grows by copies). Past that, Windows spills into shared memory and a
    # batch crawls: 32 untuned 3B samples drafting to their room took 43 minutes unfinished.
    c = model.config
    head = getattr(c, "head_dim", None) or c.hidden_size // c.num_attention_heads
    kv = c.num_hidden_layers * getattr(c, "num_key_value_heads", c.num_attention_heads) * head * 2 * 2
    longest = max(len(r["ids"]) for r in rows) + max(int(r["max_tokens"]) for r in rows)
    fits = max(1, int(torch.cuda.mem_get_info()[0] / 3 // (kv * longest)))
    if fits < a.batch:
        log("batch %d -> %d sequences: %d KB of cache a token, %d tokens long" % (a.batch, fits, kv // 1024, longest))
    made, done, t0 = 0, 0, time.time()
    from transformers import StoppingCriteriaList
    for (temp, room), group in sorted(groups.items()):
        n = samples(temp, a.k)
        per = max(1, min(a.batch, fits) // n)
        if temp > 0:
            sample = dict(SAMPLER, do_sample=True, temperature=temp)
        else:
            sample = dict(SAMPLER, do_sample=False)
        made_with = settings(group[0])
        group.sort(key=lambda r: len(r["ids"]))
        for at in range(0, len(group), per):
            chunk = group[at:at + per]
            width = max(len(r["ids"]) for r in chunk)
            pad = [width - len(r["ids"]) for r in chunk]
            ids = torch.tensor([[tok.pad_token_id] * p + r["ids"] for p, r in zip(pad, chunk)]).cuda()
            mask = torch.tensor([[0] * p + [1] * len(r["ids"]) for p, r in zip(pad, chunk)]).cuda()
            torch.manual_seed(a.seed + chunk[0]["line"])
            stop = BlockStop(tok, width, len(chunk) * n)
            with torch.no_grad():
                gen = model.generate(input_ids=ids, attention_mask=mask, max_new_tokens=room,
                                     num_return_sequences=n, stopping_criteria=StoppingCriteriaList([stop]),
                                     **sample)
            # A task answered again replaces what it had (too few: --k was raised; or made with
            # other settings).
            redo = {r["task"] for r in chunk}
            out = [x for x in out if x[0] not in redo]
            for j, seq in enumerate(gen):
                new = seq[width:]
                made += int((new != tok.pad_token_id).sum())
                reply = tok.decode(new, skip_special_tokens=True)
                end = block_end(reply)
                reply = reply if end is None else reply[:end]
                task = chunk[j // n]["task"]
                out.append((task, made_with, json.dumps({"task": task, "model": name, "reply": reply,
                                                         "gen": made_with}, ensure_ascii=False)))
            done += len(chunk)
            out.sort(key=lambda x: (x[0], x[2]))
            common.write_atomic(a.out, ("\n".join(x[2] for x in out) + "\n").encode("utf-8"))
            log("%d/%d tasks, %d tokens, %.0f tokens/s" % (done, len(rows), made, made / (time.time() - t0)))
    log("%d answers in %s; this run %.0fs" % (len(out), a.out, time.time() - t0))


if __name__ == "__main__":
    main()
