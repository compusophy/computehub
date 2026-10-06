"""What sft.py trains on: the SFT JSONL read and checked, held-out tasks
refused, duplicates dropped, each record rendered by the base model's chat
template and split into prompt and answer, the eval slice, the batch plan,
and the numbers the manifest keeps about all of it."""
import json
import os
import random
import sys

import common


def read_lines(path):
    with open(path, encoding="utf-8") as f:
        return [s for s in (ln.split("#", 1)[0].strip() for ln in f) if s]


def held_out(a):
    """The held-out task ids and families, the task -> family map, and their provenance."""
    if not (a.held or a.held_tasks or a.no_held):
        sys.exit("error: name the held-out set: --held FILE (families, with --tasks), --held-tasks, or --no-held")
    info, families, tasks, family_of = {}, set(), set(), {}
    suite = a.tasks or os.path.join(common.REPO, "evals", "suites", "iq.jsonl")
    if os.path.exists(suite):
        with open(suite, encoding="utf-8") as f:
            for ln in f:
                o = json.loads(ln) if ln.strip() else None
                if isinstance(o, dict) and "id" in o:
                    family_of[str(o["id"])] = str(o.get("family") or o["id"])
        info["tasks"] = {"file": os.path.abspath(suite), "sha256": common.sha256_file(suite), "count": len(family_of)}
    elif a.tasks:
        sys.exit("error: no such suite file: " + a.tasks)
    if a.held:
        if not family_of:
            sys.exit("error: --held names families, but no suite (--tasks) maps tasks to them; use --held-tasks")
        families = set(read_lines(a.held))
        tasks |= {t for t, fam in family_of.items() if fam in families}
        info["held"] = {"file": os.path.abspath(a.held), "sha256": common.sha256_file(a.held),
                        "families": sorted(families),
                        "not_in_suite": sorted(families - set(family_of.values()))}
    if a.held_tasks:
        ids = read_lines(a.held_tasks) if os.path.exists(a.held_tasks) else a.held_tasks.split(",")
        ids = sorted({t.strip() for t in ids if t.strip()})
        tasks |= set(ids)
        info["held_tasks"] = ids
    info["held_task_ids"] = len(tasks)
    info["none"] = bool(a.no_held and not (a.held or a.held_tasks))
    return families, tasks, family_of, info


def invalid(r):
    """Why a record breaks the SFT schema, or None."""
    if not isinstance(r, dict):
        return "is not a JSON object"
    m = r.get("messages")
    if not isinstance(m, list) or len(m) < 2:
        return "has fewer than two messages"
    for x in m:
        if not isinstance(x, dict) or x.get("role") not in ("system", "user", "assistant") \
                or not isinstance(x.get("content"), str):
            return "has a message that is not {role: system|user|assistant, content: string}"
    if m[-1]["role"] != "assistant" or not m[-1]["content"].strip():
        return "does not end with a non-empty assistant message"
    if not any(x["role"] == "user" for x in m[:-1]):
        return "has no user message"
    if not isinstance(r.get("task"), str) or not r["task"]:
        return "has no task id"
    return None


def load_records(paths):
    """Every record of every file, or exit naming the first bad line."""
    files, records = [], []
    for path in paths:
        n = 0
        with open(path, encoding="utf-8") as f:
            for i, ln in enumerate(f, 1):
                if not ln.strip():
                    continue
                try:
                    r = json.loads(ln)
                except ValueError as e:
                    sys.exit("error: %s:%d is not JSON (%s)" % (path, i, e))
                why = invalid(r)
                if why:
                    sys.exit("error: %s:%d %s" % (path, i, why))
                r["_at"] = "%s:%d" % (os.path.basename(path), i)
                records.append(r)
                n += 1
        files.append({"path": os.path.abspath(path), "sha256": common.sha256_file(path), "records": n})
    return files, records


def select(records, families, tasks, family_of):
    """Refuse held-out records and drop exact duplicates; count both."""
    counts, held_by, kept, seen = {"read": len(records), "held": 0, "duplicate": 0}, {}, [], set()
    for r in records:
        fam = r.get("family") or family_of.get(r["task"])
        if r["task"] in tasks or (fam is not None and fam in families):
            counts["held"] += 1
            held_by[fam or r["task"]] = held_by.get(fam or r["task"], 0) + 1
            continue
        key = common.sha256_text(json.dumps(r["messages"], sort_keys=True))
        if key in seen:
            counts["duplicate"] += 1
            continue
        seen.add(key)
        kept.append(r)
    return kept, counts, held_by


def tokenize(records, tok, max_len):
    """Prompt: every message but the last, plus the template's generation
    prompt, which is what the server feeds the model. Answer: the rest of the
    full rendering, through the last end-of-turn token. Over-long records are
    dropped, and their lengths returned to be counted. A record whose full
    rendering does not begin with its prompt's is dropped too, and returned
    ({"task", "at"}) to be counted: training on it could not match inference
    (Qwen2.5's template renders an answer that begins with a newline into one
    token with the generation prompt's own), and one such record, a student's
    sample, must not cost the whole run."""
    eos, items, over, unmatched = tok.eos_token_id, [], [], []
    for r in records:
        prompt = tok.apply_chat_template(r["messages"][:-1], tokenize=True, add_generation_prompt=True)
        full = tok.apply_chat_template(r["messages"], tokenize=True)
        if full[:len(prompt)] != prompt:
            unmatched.append({"task": r["task"], "at": r["_at"]})
            continue
        answer = full[len(prompt):]
        if eos not in answer:
            sys.exit("error: %s: the rendered answer has no end-of-turn token" % r["_at"])
        answer = answer[:len(answer) - answer[::-1].index(eos)]
        if len(prompt) + len(answer) > max_len:
            over.append(len(prompt) + len(answer))
            continue
        items.append({"ids": prompt + answer, "start": len(prompt), "task": r["task"], "rec": r})
    return items, over, unmatched


def unit(task):
    return int(common.sha256_text("val:" + task)[:8], 16) / 2.0 ** 32


def split(items, frac, vmax):
    """The eval slice: whole tasks picked by a hash of their id, so it is the
    same night after night and no eval task is also trained on."""
    tasks = sorted({x["task"] for x in items}, key=unit)
    if frac <= 0 or len(tasks) < 2:
        return items, []
    size = {}
    for x in items:
        size[x["task"]] = size.get(x["task"], 0) + 1
    chosen, n = set(), 0
    for t in tasks[:len(tasks) // 2]:
        if chosen and (unit(t) >= frac or n + size[t] > vmax):
            break
        chosen.add(t)
        n += size[t]
    return [x for x in items if x["task"] not in chosen], [x for x in items if x["task"] in chosen]


def plan(items, epoch, a):
    """An epoch's optimizer steps, each a list of micro-batches of item
    indexes: shuffled by (seed, epoch), sorted by length within windows so
    little is padding. The same epoch always gets the same plan (resume)."""
    rng = random.Random(a.seed * 1000003 + epoch)
    order = list(range(len(items)))
    rng.shuffle(order)
    win, micro = a.batch * a.accum * 8, []
    for i in range(0, len(order), win):
        w = sorted(order[i:i + win], key=lambda j: len(items[j]["ids"]))
        micro += [w[k:k + a.batch] for k in range(0, len(w), a.batch)]
    rng.shuffle(micro)
    return [micro[k:k + a.accum] for k in range(0, len(micro), a.accum)]


def stats(xs):
    if not xs:
        return None
    s = sorted(xs)

    def q(f):
        return s[min(len(s) - 1, int(f * len(s)))]
    return {"n": len(s), "min": s[0], "mean": round(sum(s) / len(s), 1), "p50": q(0.5), "p90": q(0.9),
            "p99": q(0.99), "max": s[-1], "sum": sum(s)}


def tally(records, key):
    out = {}
    for r in records:
        k = str(key(r))
        out[k] = out.get(k, 0) + 1
    return dict(sorted(out.items(), key=lambda kv: -kv[1]))


def summary(items, train, val, over):
    """The manifest's account of what was trained on."""
    recs = [x["rec"] for x in items]
    return {
        "tasks": {"train": len({x["task"] for x in train}), "val": len({x["task"] for x in val}),
                  "val_ids": sorted({x["task"] for x in val})},
        "kinds": tally(recs, lambda r: r.get("kind")),
        "teachers": tally(recs, lambda r: (r.get("by") or {}).get("teacher")),
        "days": tally(recs, lambda r: (r.get("by") or {}).get("day")),
        # Distinct system prompts: training must use the one inference uses.
        "system_prompts": tally(recs, lambda r: common.sha256_text(r["messages"][0]["content"])[:16]
                                if r["messages"][0]["role"] == "system" else None),
        "tokens": {"prompt": stats([x["start"] for x in items]),
                   "answer": stats([len(x["ids"]) - x["start"] for x in items]),
                   "total": stats([len(x["ids"]) for x in items]),
                   "dropped_over_max_len": stats(over)},
    }
