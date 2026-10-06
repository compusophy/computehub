#!/usr/bin/env python3
"""Fine-tune a base model on SFT JSONL (what `teach export` writes): LoRA, or --full.

    python train/sft.py --data D.jsonl --held H.txt [--tasks evals/suites/iq.jsonl] [--base ID] [--run NAME]

Each record's `messages` are rendered by the base model's own chat template,
as the server renders them at inference; the loss falls on the last assistant
message only. Records of held-out tasks are refused; over-long ones, and any
whose rendering does not begin with its prompt's, dropped and counted. Writes
<root>/runs/<run>/: manifest.json (provenance), train.log,
ckpt-a/ and ckpt-b/ (resume points, written in turn every --save-every steps),
and adapter/ (LoRA) or model/ (--full). After a crash, rerun the same command:
it resumes from the newest whole checkpoint.
"""
import argparse
import datetime
import json
import math
import os
import random
import sys
import time
import warnings

import common  # first: it sets the Hugging Face offline switches
import sftdata as data
import torch
import torch.nn.functional as F
from torch.utils.checkpoint import checkpoint

LORA_TARGETS = "q_proj,k_proj,v_proj,o_proj,gate_proj,up_proj,down_proj"
FULL_MAX_PARAMS = 1.6e9  # fp32 weights + grads + Adam: ~16 bytes a parameter, on 24 GB
SLOTS = ("ckpt-a", "ckpt-b")
# torch's own checkpoint code calls its deprecated CPU autocast on every step.
warnings.filterwarnings("ignore", message=r".*torch\.cpu\.amp\.autocast.*")


def parse_args():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--data", action="append", required=True, help="SFT JSONL file (repeatable)")
    p.add_argument("--base", default=common.DEFAULT_BASE, help="hub id in the local HF cache, or a folder")
    p.add_argument("--root", help="data root (default $COMPUTEHUB_DATA, else %s)" % common.DEFAULT_ROOT)
    p.add_argument("--run", help="run name (default <day>-<base>-<mode>); the same name resumes")
    g = p.add_argument_group("held-out set: one of --held, --held-tasks, --no-held is required")
    g.add_argument("--held", help="file: one held-out task family per line (# comments)")
    g.add_argument("--tasks", help="suite JSONL, an id and a family per task (default evals/suites/iq.jsonl)")
    g.add_argument("--held-tasks", help="file (one id per line) or comma list of held-out task ids")
    g.add_argument("--no-held", action="store_true", help="no held-out set (smoke tests only)")
    h = p.add_argument_group("training")
    h.add_argument("--full", action="store_true", help="full fine-tune, not LoRA (models under 1.6B)")
    h.add_argument("--lora-r", type=int, default=32)
    h.add_argument("--lora-alpha", type=int, default=64)
    h.add_argument("--lora-dropout", type=float, default=0.05)
    h.add_argument("--lora-targets", default=LORA_TARGETS)
    h.add_argument("--epochs", type=float, default=3)
    h.add_argument("--max-steps", type=int, default=0, help="this many optimizer steps (overrides --epochs)")
    h.add_argument("--lr", type=float, help="peak learning rate (default 2e-4 LoRA, 5e-5 full)")
    h.add_argument("--warmup", type=float, default=0.05, help="warmup, a fraction of the steps")
    h.add_argument("--weight-decay", type=float, default=0.0)
    h.add_argument("--batch", type=int, default=4, help="sequences per micro-batch")
    h.add_argument("--accum", type=int, default=4, help="micro-batches per optimizer step")
    h.add_argument("--max-len", type=int, default=12288, help="tokens; longer records are dropped, counted")
    h.add_argument("--val-frac", type=float, default=0.05, help="fraction of tasks kept out for eval loss")
    h.add_argument("--val-max", type=int, default=64, help="at most this many eval records")
    h.add_argument("--save-every", type=int, default=50, help="checkpoint (and eval) every N steps")
    h.add_argument("--log-every", type=int, default=5)
    h.add_argument("--seed", type=int, default=1234)
    h.add_argument("--lm-chunk", type=int, default=2048, help="answer tokens per vocab projection chunk")
    return p.parse_args()


def collate(items, idxs, pad):
    rows = [items[j] for j in idxs]
    width = max(len(r["ids"]) for r in rows)
    ids = torch.full((len(rows), width), pad, dtype=torch.long)
    mask = torch.zeros((len(rows), width), dtype=torch.long)
    labels = torch.full((len(rows), width), -100, dtype=torch.long)
    for i, r in enumerate(rows):
        n = len(r["ids"])
        ids[i, :n] = torch.tensor(r["ids"])
        mask[i, :n] = 1
        labels[i, r["start"]:n] = ids[i, r["start"]:n]
    return ids.cuda(), mask.cuda(), labels.cuda()


def loss_sum(lm, ids, mask, labels, chunk):
    """Summed cross-entropy over the answer tokens. The vocab projection runs
    only where a position predicts an answer token, in checkpointed chunks:
    whole logits (tokens x 152k vocab, fp32) would not fit."""
    hidden = lm.model(input_ids=ids, attention_mask=mask, use_cache=False).last_hidden_state
    keep = labels[:, 1:] != -100
    h, y = hidden[:, :-1][keep], labels[:, 1:][keep]

    def ce(hc, yc):
        return F.cross_entropy(lm.lm_head(hc).float(), yc, reduction="sum")
    total = torch.zeros((), device=ids.device, dtype=torch.float32)
    for i in range(0, len(y), chunk):
        part = (h[i:i + chunk], y[i:i + chunk])
        total = total + (checkpoint(ce, *part, use_reentrant=False) if torch.is_grad_enabled() else ce(*part))
    return total


def evaluate(model, lm, items, a, pad):
    model.eval()
    order = sorted(range(len(items)), key=lambda j: len(items[j]["ids"]))
    total, n = 0.0, 0
    with torch.no_grad():
        for k in range(0, len(order), a.batch):
            mb = order[k:k + a.batch]
            with torch.autocast("cuda", dtype=torch.bfloat16):
                total += loss_sum(lm, *collate(items, mb, pad), a.lm_chunk).item()
            n += sum(len(items[j]["ids"]) - items[j]["start"] for j in mb)
    model.train()
    return round(total / n, 4)


def trainable(model):
    return {n: p for n, p in model.named_parameters() if p.requires_grad}


def latest_slot(rdir, config):
    """The newest whole checkpoint made under this run's config, or None."""
    best = None
    for s in SLOTS:
        d = os.path.join(rdir, s)
        st = common.read_json(os.path.join(d, "state.json"))
        if not st or not st.get("valid") or st.get("config") != config:
            continue
        if any(not os.path.exists(os.path.join(d, f)) or os.path.getsize(os.path.join(d, f)) != n
               for f, n in st["files"].items()):
            continue
        if best is None or st["step"] > best[1]["step"]:
            best = (d, st)
    return best


def save_slot(rdir, state, model, opt, sched, current):
    """Write the slot that does not hold the newest checkpoint: mark it
    invalid, write each file atomically, then its state.json last. A crash
    at any point leaves the other slot whole."""
    d = os.path.join(rdir, SLOTS[1] if current and os.path.basename(current) == SLOTS[0] else SLOTS[0])
    os.makedirs(d, exist_ok=True)
    common.write_json(os.path.join(d, "state.json"), {"valid": False})
    rng = {"python": random.getstate(), "torch": torch.get_rng_state(), "cuda": torch.cuda.get_rng_state_all()}
    files = {}
    for name, obj in (("weights.pt", {n: p.detach().cpu() for n, p in trainable(model).items()}),
                      ("optimizer.pt", opt.state_dict()),
                      ("rest.pt", {"scheduler": sched.state_dict(), "rng": rng})):
        path = os.path.join(d, name)
        torch.save(obj, path + ".tmp")
        common.fsync_file(path + ".tmp")
        common.replace(path + ".tmp", path)
        files[name] = os.path.getsize(path)
    common.write_json(os.path.join(d, "state.json"), dict(state, valid=True, files=files))
    return d


def resume(slot, model, opt, sched):
    sd = torch.load(os.path.join(slot, "weights.pt"), map_location="cpu", weights_only=True)
    named = trainable(model)
    if set(named) != set(sd):
        sys.exit("error: %s does not hold this model's trainable weights" % slot)
    with torch.no_grad():
        for n, p in named.items():
            p.copy_(sd[n])
    opt.load_state_dict(torch.load(os.path.join(slot, "optimizer.pt"), map_location="cpu", weights_only=False))
    rest = torch.load(os.path.join(slot, "rest.pt"), map_location="cpu", weights_only=False)
    sched.load_state_dict(rest["scheduler"])
    random.setstate(rest["rng"]["python"])
    torch.set_rng_state(rest["rng"]["torch"])
    torch.cuda.set_rng_state_all(rest["rng"]["cuda"])


def load_model(a, base):
    from transformers import AutoModelForCausalLM
    model = AutoModelForCausalLM.from_pretrained(base["path"], dtype=torch.float32 if a.full else torch.bfloat16,
                                                 attn_implementation="sdpa").cuda()
    n = sum(p.numel() for p in model.parameters())
    if a.full and n > FULL_MAX_PARAMS:
        sys.exit("error: %.1fB parameters will not fully fine-tune on 24 GB; use LoRA" % (n / 1e9))
    model.config.use_cache = False
    model.gradient_checkpointing_enable(gradient_checkpointing_kwargs={"use_reentrant": False})
    if not a.full:
        from peft import LoraConfig, get_peft_model
        model = get_peft_model(model, LoraConfig(r=a.lora_r, lora_alpha=a.lora_alpha, lora_dropout=a.lora_dropout,
                                                 target_modules=a.lora_targets.split(","), task_type="CAUSAL_LM"))
    return model, (model if a.full else model.get_base_model()), n


def main():
    common.utf8_stdio()
    a = parse_args()
    root = common.data_root(a.root)
    base = common.resolve_base(a.base, root)
    mode = "full" if a.full else "lora"
    run = a.run or "%s-%s-%s" % (datetime.date.today().isoformat(), common.slug(a.base), mode)
    rdir = common.run_dir(root, run)
    mpath = os.path.join(rdir, "manifest.json")
    m = common.read_json(mpath)
    if m and m.get("status") == "done":
        print("run %s is done (%s); a new run needs a new --run" % (run, m["results"]["out"]))
        return
    families, held_tasks, family_of, held_info = data.held_out(a)
    files, records = data.load_records(a.data)
    os.makedirs(rdir, exist_ok=True)
    log = common.Log(os.path.join(rdir, "train.log"))
    log("== sft %s (pid %d), base %s @ %s" % (run, os.getpid(), base["id"], base["revision"]))
    kept, counts, held_by = data.select(records, families, held_tasks, family_of)
    if counts["held"]:
        log("WARNING: refused %d records of held-out tasks: %s" % (counts["held"], held_by))
    if held_info["none"]:
        log("WARNING: no held-out set: every task may be trained on (smoke tests only)")
    from transformers import AutoTokenizer, get_cosine_schedule_with_warmup
    tok = AutoTokenizer.from_pretrained(base["path"])
    pad = tok.pad_token_id if tok.pad_token_id is not None else tok.eos_token_id
    items, over, unmatched = data.tokenize(kept, tok, a.max_len)
    train, val = data.split(items, a.val_frac, a.val_max)
    counts.update(too_long=len(over), unmatched=len(unmatched), train=len(train), val=len(val))
    log("records: %s" % counts)
    if over:
        log("WARNING: dropped %d records over --max-len %d tokens (longest %d)" % (len(over), a.max_len, max(over)))
    if unmatched:
        log("WARNING: dropped %d records whose rendering does not begin with their prompt's: %s" % (
            len(unmatched), ", ".join("%s (%s)" % (u["task"], u["at"]) for u in unmatched)))
    if not train:
        sys.exit("error: no records left to train on")

    hyper = {k: v for k, v in vars(a).items() if k not in ("data", "root", "run", "save_every", "log_every",
                                                          "held", "tasks", "held_tasks", "no_held")}
    hyper.update(mode=mode, lr=a.lr or (5e-5 if a.full else 2e-4), max_grad_norm=1.0, schedule="cosine",
                 optimizer="AdamW", precision="bf16 autocast, " + ("fp32 weights" if a.full else "bf16 base, fp32 LoRA"))
    config = common.sha256_text(json.dumps({"data": [f["sha256"] for f in files], "held": held_info,
                                            "base": [base["id"], base["revision"]], "hyper": hyper}, sort_keys=True))
    if m and m.get("config") != config:
        sys.exit("error: run %s exists with other data or settings; name a new --run" % run)
    if not m:
        m = {"schema": "computehub-sft/1", "run": run, "config": config, "created": common.now(),
             "repo": common.git_commit(), "script_sha256": common.sha256_file(os.path.abspath(__file__)),
             "base": base, "data": files, "held_out": held_info, "records": counts, "held_by": held_by,
             "unmatched": unmatched}
        m.update(data.summary(items, train, val, over))
        m.update(hyper=hyper, env=common.environment(), attempts=[])
    attempt = {"started": common.now(), "pid": os.getpid(), "commit": common.git_commit()["commit"]}
    m["attempts"].append(attempt)
    m["status"] = "running"
    common.write_json(mpath, m)
    t = m["tokens"]["total"]
    log("tokens/record: min %d p50 %d p90 %d max %d; answer tokens in all %d" % (
        t["min"], t["p50"], t["p90"], t["max"], m["tokens"]["answer"]["sum"]))

    torch.manual_seed(a.seed)
    random.seed(a.seed)
    torch.backends.cuda.matmul.allow_tf32 = True
    t_load = time.time()
    model, lm, nparams = load_model(a, base)
    params = list(trainable(model).values())
    opt = torch.optim.AdamW(params, lr=hyper["lr"], weight_decay=a.weight_decay)
    per_epoch = len(data.plan(train, 0, a))
    total = a.max_steps or max(1, math.ceil(a.epochs * per_epoch))
    sched = get_cosine_schedule_with_warmup(opt, int(a.warmup * total), total)
    log("model: %.0fM params, %.1fM trainable (%s), loaded in %.1fs; %d steps (%d an epoch, %d seqs a step)" % (
        nparams / 1e6, sum(p.numel() for p in params) / 1e6, mode, time.time() - t_load, total, per_epoch,
        a.batch * a.accum))

    state = {"step": 0, "seconds": 0.0, "tokens": 0, "answer_tokens": 0, "history": [], "evals": []}
    slot = None
    found = latest_slot(rdir, config)
    if found:
        slot, st = found
        resume(slot, model, opt, sched)
        state = {k: st[k] for k in state}
        attempt["resumed_from"] = {"slot": os.path.basename(slot), "step": state["step"]}
        log("resumed from %s at step %d/%d" % (os.path.basename(slot), state["step"], total))

    def checkpoint_now():
        state["seconds"] = round(prior + time.time() - t0, 1)
        new = save_slot(rdir, dict(state, config=config), model, opt, sched, slot)
        m["progress"] = {"step": state["step"], "of": total, "at": common.now()}
        common.write_json(mpath, m)
        return new

    model.train()
    torch.cuda.reset_peak_memory_stats()
    t0, prior, first, toks, window, plans = time.time(), state["seconds"], state["step"], 0, [0.0, 0], {}
    try:
        while state["step"] < total:
            epoch, k = divmod(state["step"], per_epoch)
            if epoch not in plans:
                plans = {epoch: data.plan(train, epoch, a)}
            micro = plans[epoch][k]
            n_ans = sum(len(train[j]["ids"]) - train[j]["start"] for mb in micro for j in mb)
            for mb in micro:
                ids, mask, labels = collate(train, mb, pad)
                with torch.autocast("cuda", dtype=torch.bfloat16):
                    s = loss_sum(lm, ids, mask, labels, a.lm_chunk)
                (s / n_ans).backward()  # a token-weighted mean over the whole step
                window[0] += s.item()
                n_tok = sum(len(train[j]["ids"]) for j in mb)
                toks += n_tok
                state["tokens"] += n_tok
            window[1] += n_ans
            gnorm = float(torch.nn.utils.clip_grad_norm_(params, 1.0))
            opt.step()
            sched.step()
            opt.zero_grad(set_to_none=True)
            state["step"] += 1
            state["answer_tokens"] += n_ans
            step, el = state["step"], time.time() - t0
            if step % a.log_every == 0 or step == total:
                loss, window = window[0] / window[1], [0.0, 0]
                state["history"].append([step, round(loss, 4)])
                log("step %d/%d epoch %.2f loss %.4f lr %.2e gnorm %.2f | %.2f step/s %.0f tok/s | %.1f GB" % (
                    step, total, step / per_epoch, loss, sched.get_last_lr()[0], gnorm, (step - first) / el,
                    toks / el, torch.cuda.max_memory_reserved() / 2 ** 30))
            if step % a.save_every == 0 or step == total:
                if val:
                    state["evals"].append([step, evaluate(model, lm, val, a, pad)])
                    log("eval loss %.4f (%d records)" % (state["evals"][-1][1], len(val)))
                slot = checkpoint_now()
                log("checkpoint %s at step %d" % (os.path.basename(slot), step))
    except KeyboardInterrupt:
        log("interrupted at step %d; saving a checkpoint" % state["step"])
        checkpoint_now()
        sys.exit(130)

    el = time.time() - t0
    out = os.path.join(rdir, "model" if a.full else "adapter")
    os.makedirs(out, exist_ok=True)
    if a.full:
        model.to(torch.bfloat16).save_pretrained(out, safe_serialization=True, max_shard_size="100GB")
    else:
        model.save_pretrained(out)
    tok.save_pretrained(out)
    common.write_json(os.path.join(out, common.PROVENANCE), {"run": run, "config": config, "base": base})
    done = total - first
    m["results"] = {
        "out": out, "steps": total, "epochs": round(total / per_epoch, 3), "steps_per_epoch": per_epoch,
        "train_loss": state["history"][-1][1], "eval_loss": state["evals"][-1][1] if state["evals"] else None,
        "train_seconds": state["seconds"], "tokens": state["tokens"], "answer_tokens": state["answer_tokens"],
        "this_attempt": {"steps": done, "seconds": round(el, 1),
                         "steps_per_s": round(done / el, 3) if done else None,
                         "tokens_per_s": round(toks / el) if done else None},
        "peak_vram_gb": {"allocated": round(torch.cuda.max_memory_allocated() / 2 ** 30, 2),
                         "reserved": round(torch.cuda.max_memory_reserved() / 2 ** 30, 2)},
        "history": state["history"], "evals": state["evals"],
        "files": {f: common.sha256_file(os.path.join(out, f)) for f in sorted(os.listdir(out))},
    }
    m["status"], m["ended"] = "done", common.now()
    attempt["ended"] = m["ended"]
    common.write_json(mpath, m)
    r = m["results"]
    log("done: %s; train loss %s, eval loss %s, %s tok/s, peak %.1f GB reserved" % (
        out, r["train_loss"], r["eval_loss"], r["this_attempt"]["tokens_per_s"], r["peak_vram_gb"]["reserved"]))


if __name__ == "__main__":
    main()
