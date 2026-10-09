#!/usr/bin/env python
"""Can a local model tell a program that passes its task's hidden check from one that runs clean
but fails it, better than the program's length can? The spine's "checks / selection" role tested
where it is cheapest: one token a program, served by llama-server (train/serve.py).

  python train/judge.py --url U --suite iq.jsonl --pre answers-pre.jsonl --pre-each each-pre.txt
                        [--sets judge-eval-glm-s.jsonl judge-eval-alt-s.jsonl]
                        [--candidates tries-*.jsonl] [--iq IQ.exe]
                        --out judge-NAME.jsonl --fit judgefit-NAME.json [--jobs 8] [--timeout S]

The prompt is judgedata's (rec()): its system prompt, then "Asked: ASK", the program in an app
block with its comments stripped (strip(): a "// Wrong:" header gave 140 of 855 failing
alternates away), and "Does it do everything asked?". One request a distinct prompt (to --url:
the server's root, its /v1 or its chat endpoint), non-streaming: max_tokens 1, temperature 0,
seed 1, logprobs with the top 20. P(yes) is the probability of the top tokens that read "yes"
(stripped, any case) over that and the same for "no". A reply without logprobs, or with neither
word among them, is read by its text (yes 1, else 0) and counted as nolp. A prompt beyond the
server's context is recorded as such and left out of every AUC; any other failure is not
recorded, so a rerun asks it again, and the run exits 3 without writing --fit (2: an input
missing). --out holds every scored record and is the cache: a prompt whose sha
(common.prompt_hash of its messages) is there is never asked again.

Sets:
  pre   --pre's programs that run clean (--pre-each stage check or pass), labelled by pass: the
        set that decides
  glm, alt  the --sets files (judgedata records); alt also within tasks, over its pass/fail
        pairs (a diagnostic). A set is named glm or alt by its file name, else by the name itself
  cand  the --candidates tries files' clean programs (eval team --tries-out), each distinct
        program graded once by the real check (--iq, else $IQ, else bin/iq.exe beside --suite):
        judge@k, the judge's pick among a draft's clean tries of its first k (the highest
        P(yes), the earliest on a tie), beside the first clean try (first-clean@k) and any
        (oracle@k, a ceiling no system reaches)

--fit, per set: AUC with a 95% interval from a family bootstrap (2,000 resamples of the families,
seed 1) for the judge, for length (shorter reads as yes) and for tier (lower reads as yes); the
rank-average of judge and length, and its lift over length with the paired bootstrap's interval;
AUC within tasks; then judge@k by tries file, and the requests, errors, nolp and seconds.
"""
import argparse
import glob
import json
import math
import os
import random
import re
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor

import common
from blocks import program as program_of

SYSTEM = ("You review apps written in applang, the app language of Studio. You get what a person "
          "asked for and the program made for it. Say whether the program does everything the ask "
          "says, exactly as it says it: every label, button, number, color, rule and timing. "
          "Answer with one word: yes or no.")
QUESTION = "Does it do everything asked?"
# A judgedata user message: the ask, then the program.
USER = re.compile(r"Asked: (.*?)\n\nProgram:\n```app\n(.*)\n```\n\nDoes it do everything asked\?", re.S)
CONTEXT = "exceeds the available context"
BOOT = 2000
KS = (1, 2, 4, 8)
# What a judgement keeps, in --out and its cache.
RESULT = ("p", "how", "yes", "no", "text", "ms", "model")


def parse_args(argv=None):
    h = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    h.add_argument("--url", required=True, help="the llama-server: its root, /v1 or /v1/chat/completions")
    h.add_argument("--suite", required=True, help="the real suite: asks, families, tiers")
    h.add_argument("--pre", required=True, help="answers-pre.jsonl")
    h.add_argument("--pre-each", required=True, help="iq score --each of --pre")
    h.add_argument("--sets", nargs="*", default=[], help="judgedata records, comments stripped (nightprep)")
    h.add_argument("--candidates", nargs="*", default=[], help="tries files (a glob is expanded)")
    h.add_argument("--iq", help="grades the candidates (default $IQ, else bin/iq.exe beside --suite)")
    h.add_argument("--out", required=True, help="every scored record; the cache")
    h.add_argument("--fit", required=True)
    h.add_argument("--jobs", type=int, default=8, help="requests at once (serve.py's --parallel)")
    h.add_argument("--timeout", type=float, default=600, help="seconds a request may take")
    return h.parse_args(argv)


def jl(path):
    """A JSON-lines file's objects; a cut or malformed line is skipped."""
    out = []
    if not os.path.exists(path):
        return out
    with open(path, encoding="utf-8") as f:
        for line in f:
            if line.startswith("{"):
                try:
                    out.append(json.loads(line))
                except ValueError:
                    pass
    return out


def strip(src):
    """src without its comments, as nightprep's strip_comments strips the judge sets (kept here,
    so the judge stands without it; test_judge.py holds the two equal). Comments as applang's
    lexer reads them: // to the line's end, /* */ nested and across lines, neither inside a "..."
    string (one line, with \\-escapes). A line that was only comment goes, a comment after code
    goes with the spaces before it, and a run of blank lines becomes one empty line (none at the
    ends). strip(strip(s)) == strip(s)."""
    out, depth = [], 0
    for line in src.split("\n"):
        code, had, quoted, i = [], depth > 0, False, 0
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
        code = "".join(code)
        if had or depth:
            code = code.rstrip()
            if not code.strip():
                continue  # a comment's own line goes, never left blank
        if code.strip() or (out and out[-1]):
            out.append(code if code.strip() else "")
    while out and not out[-1]:
        out.pop()
    return "\n".join(out)


def render(ask, prog):
    """judgedata.rec()'s prompt (its messages but the answer): the ask, then the program."""
    user = "Asked: " + ask + "\n\nProgram:\n```app\n" + prog.strip("\n") + "\n```\n\n" + QUESTION
    return [{"role": "system", "content": SYSTEM}, {"role": "user", "content": user}]


def item(name, suite, task, prog, label, ask=None, family=None):
    """A record to judge: the program as the judge sees it (comments stripped), its length, the
    prompt and its sha."""
    t = suite.get(task, {})
    shown = strip(prog).strip("\n")
    messages = render(ask if ask is not None else t.get("ask", ""), shown)
    return {"set": name, "task": task, "family": family or t.get("family") or task, "tier": t.get("tier") or 0,
            "label": bool(label), "len": len(shown), "sha": common.prompt_hash(messages), "messages": messages}


def pre_set(suite, answers, each):
    """answers' programs that run clean, labelled by the hidden check: (items, without a program)."""
    grades = {g["task"]: g for g in jl(each) if "task" in g and "stage" in g}
    out, seen, missing = [], set(), 0
    for a in jl(answers):
        t = a.get("task")
        g = grades.get(t)
        if t in seen or t not in suite or not g or g["stage"] not in ("check", "pass"):
            continue
        seen.add(t)
        p = program_of(a.get("reply") or "")
        if p is None:
            missing += 1
            continue
        out.append(item("pre", suite, t, p[0], g.get("pass")))
    return out, missing


def set_name(path):
    base = os.path.basename(path)
    for name in ("glm", "alt"):
        if name in base:
            return name
    return os.path.splitext(base)[0]


def file_set(name, path, suite):
    """A judgedata file's records, each program stripped again (idempotent) and re-rendered:
    (items, unread)."""
    out, bad = [], 0
    for r in jl(path):
        user = next((m["content"] for m in r.get("messages", []) if m.get("role") == "user"), "")
        m = USER.fullmatch(user)
        if not m or "task" not in r or "label" not in r:
            bad += 1
            continue
        out.append(item(name, suite, r["task"], m.group(2), r["label"], ask=m.group(1), family=r.get("family")))
    return out, bad


def expand(paths):
    """Each path, or what it matches as a glob (a shell leaves an unmatched one as it is)."""
    out = []
    for p in paths:
        for f in ([p] if os.path.exists(p) else sorted(glob.glob(p))):
            if f not in out:
                out.append(f)
    return out


def tries_of(path, suite):
    """A tries file's drafts: {task: [the try's program if it ran clean, else None, ...]} in try
    order. A (task, try) logged twice (a run cut off and resumed) keeps its last line."""
    by = {}
    for x in jl(path):
        if x.get("task") in suite and isinstance(x.get("try"), int):
            by.setdefault(x["task"], {})[x["try"]] = x
    out = {}
    for t, d in by.items():
        progs = []
        for k in sorted(d):
            x = d[k]
            p = program_of(x.get("reply") or "") if x.get("clean") is True and not x.get("error") else None
            progs.append(p[0] if p else None)
        out[t] = progs
    return out


def grade(iq, suite_path, pairs):
    """[(task, program)] graded by `iq score --each` against the real suite: [passed], or None
    without an iq."""
    if not iq or not os.path.exists(iq):
        return None
    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, "answers.jsonl")
        with open(path, "w", encoding="utf-8", newline="\n") as f:
            for t, p in pairs:
                reply = "```app\n" + p.rstrip("\n") + "\n```"
                f.write(json.dumps({"task": t, "model": "judge-cand", "reply": reply}, ensure_ascii=False) + "\n")
        r = subprocess.run([iq, "score", path, "--suite", suite_path, "--each"], capture_output=True)
    each = [json.loads(l) for l in r.stdout.decode("utf-8", "replace").split("\n") if l.startswith("{")]
    if len(each) != len(pairs) or any(e.get("task") != t for e, (t, _) in zip(each, pairs)):
        raise SystemExit("iq score gave %d grades for %d programs: %s" % (
            len(each), len(pairs), r.stderr.decode("utf-8", "replace")[:400]))
    return [bool(e.get("pass")) for e in each]


def endpoint(url):
    u = url.rstrip("/")
    if u.endswith("/chat/completions"):
        return u
    return u + ("/chat/completions" if u.endswith("/v1") else "/v1/chat/completions")


def lse(xs):
    m = max(xs)
    return m + math.log(sum(math.exp(x - m) for x in xs))


def p_yes(choice):
    """(P(yes), how) from a choice's first token's top logprobs: how is lp, or nolp (no logprobs)
    or noyn (neither word among them), when P(yes) is None; and the yes and no log masses."""
    content = (choice.get("logprobs") or {}).get("content") or []
    top = (content[0].get("top_logprobs") or []) if content else []
    if not top:
        return None, "nolp", None, None
    word = {"yes": [], "no": []}
    for x in top:
        w = str(x.get("token", "")).strip().lower()
        if w in word and isinstance(x.get("logprob"), (int, float)):
            word[w].append(float(x["logprob"]))
    if not word["yes"] and not word["no"]:
        return None, "noyn", None, None
    yes = lse(word["yes"]) if word["yes"] else None
    no = lse(word["no"]) if word["no"] else None
    if yes is None:
        return 0.0, "lp", yes, no
    if no is None:
        return 1.0, "lp", yes, no
    m = max(yes, no)  # shifted, so two tiny masses never divide 0 by 0
    return math.exp(yes - m) / (math.exp(yes - m) + math.exp(no - m)), "lp", yes, no


def ask(url, messages, timeout):
    """One judgement of messages, as RESULT's fields. A prompt beyond the server's context comes
    back with how "context"; any other failure raises."""
    body = {"messages": messages, "max_tokens": 1, "temperature": 0, "seed": 1, "logprobs": True,
            "top_logprobs": 20, "stream": False}
    req = urllib.request.Request(url, data=json.dumps(body).encode("utf-8"),
                                 headers={"content-type": "application/json"})
    t0 = time.time()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            got = json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        said = e.read().decode("utf-8", "replace")
        if CONTEXT in said:
            return {"p": None, "how": "context", "yes": None, "no": None, "text": said[:200],
                    "ms": int((time.time() - t0) * 1000), "model": None}
        raise RuntimeError("HTTP %d %s" % (e.code, said[:300]))
    ms = int((time.time() - t0) * 1000)
    choice = got["choices"][0]
    text = (choice.get("message") or {}).get("content") or ""
    p, how, yes, no = p_yes(choice)
    if p is None:  # no logprobs to read: the word itself
        p = 1.0 if text.strip().lower().startswith("yes") else 0.0
    return {"p": p, "how": how, "yes": yes, "no": no, "text": text, "ms": ms, "model": got.get("model")}


def ranks(xs):
    """1-based ranks of xs, ascending; ties share their mean rank."""
    order = sorted(range(len(xs)), key=xs.__getitem__)
    r = [0.0] * len(xs)
    i = 0
    while i < len(order):
        j = i
        while j + 1 < len(order) and xs[order[j + 1]] == xs[order[i]]:
            j += 1
        for k in range(i, j + 1):
            r[order[k]] = (i + j) / 2 + 1
        i = j + 1
    return r


def auc(scores, labels):
    """P(a passing program scores above a failing one), a tie counting half (Mann-Whitney by
    ranks); None without both kinds."""
    pos = sum(1 for x in labels if x)
    neg = len(labels) - pos
    if not pos or not neg:
        return None
    r = ranks(scores)
    return (sum(x for x, y in zip(r, labels) if y) - pos * (pos + 1) / 2) / (pos * neg)


def within(rows, key):
    """AUC over the (pass, fail) pairs of one task: {pairs, tasks, auc}."""
    by = {}
    for r in rows:
        by.setdefault(r["task"], ([], []))[0 if r["label"] else 1].append(key(r))
    s, pairs, tasks = 0.0, 0, 0
    for pos, neg in by.values():
        if pos and neg:
            tasks += 1
            for a in pos:
                for b in neg:
                    pairs += 1
                    s += 1.0 if a > b else 0.5 if a == b else 0.0
    return {"pairs": pairs, "tasks": tasks, "auc": s / pairs if pairs else None}


def interval(vals):
    """The 2.5th and 97.5th percentiles of vals (nearest rank)."""
    v = sorted(vals)
    if not v:
        return None, None
    return v[int(0.025 * (len(v) - 1))], v[int(math.ceil(0.975 * (len(v) - 1)))]


def fit_set(rows):
    """One set's fit: the AUCs of the judge, length, tier and the judge-length rank-average, each
    with its family-bootstrap interval; the rank-average's lift over length with the paired
    interval (the same resamples); the AUCs within tasks."""
    left = sum(1 for r in rows if r["p"] is None)
    rows = [r for r in rows if r["p"] is not None]
    labels = [r["label"] for r in rows]
    scores = {"judge": [r["p"] for r in rows], "length": [-r["len"] for r in rows],
              "tier": [-r["tier"] for r in rows]}
    rj, rl = ranks(scores["judge"]), ranks(scores["length"])
    scores["rankavg"] = [(x + y) / 2 for x, y in zip(rj, rl)]
    est = {k: auc(v, labels) for k, v in scores.items()}
    fams = {}
    for i, r in enumerate(rows):
        fams.setdefault(r["family"], []).append(i)
    keys = sorted(fams)
    rng = random.Random(1)
    boot, lift = {k: [] for k in scores}, []
    for _ in range(BOOT if keys else 0):
        idx = [i for f in [rng.choice(keys) for _ in keys] for i in fams[f]]
        lab = [labels[i] for i in idx]
        if all(lab) or not any(lab):
            continue
        got = {k: auc([v[i] for i in idx], lab) for k, v in scores.items()}
        for k, v in got.items():
            boot[k].append(v)
        lift.append(got["rankavg"] - got["length"])
    out = {"n": len(rows), "yes": sum(labels), "no": len(rows) - sum(labels), "families": len(keys),
           "left_out": left, "nolp": sum(1 for r in rows if r["how"] != "lp"), "resamples": len(lift), "auc": {}}
    for k in scores:
        lo, hi = interval(boot[k])
        out["auc"][k] = {"auc": est[k], "lo": lo, "hi": hi}
    lo, hi = interval(lift)
    gain = None if est["rankavg"] is None else est["rankavg"] - est["length"]
    out["lift"] = {"auc": gain, "lo": lo, "hi": hi}
    rank = dict(zip((id(r) for r in rows), scores["rankavg"]))
    out["within"] = {"judge": within(rows, lambda r: r["p"]), "length": within(rows, lambda r: -r["len"]),
                     "rankavg": within(rows, lambda r: rank[id(r)])}
    # D4's two tests on this set: the judge's interval above length's AUC, or a rank-average
    # lift of 0.03 or more with its interval above 0.
    j, ln = out["auc"]["judge"], est["length"]
    out["above_length"] = bool(j["lo"] is not None and ln is not None and j["lo"] > ln)
    out["lift_ok"] = bool(gain is not None and lo is not None and gain >= 0.03 and lo > 0)
    return out


def at_k(tries, p, passed, ks):
    """judge@k beside first-clean@k and oracle@k: for each k, of the drafts, how many have a
    clean try among their first k (clean), more than one distinct clean program there (choices),
    and how many pass with the first clean try (first), with the judge's pick (judge) and with
    any (oracle). tries: {task: [program or None]}; p, passed: by (task, program)."""
    out = {}
    for k in ks:
        row = {"drafts": len(tries), "clean": 0, "choices": 0, "first": 0, "judge": 0, "oracle": 0}
        for t, progs in tries.items():
            clean = [q for q in progs[:k] if q is not None]
            if not clean:
                continue
            row["clean"] += 1
            row["choices"] += len(set(clean)) > 1
            score = [-1.0 if p.get((t, q)) is None else p[(t, q)] for q in clean]
            pick = max(range(len(clean)), key=lambda i: (score[i], -i))
            row["first"] += passed[(t, clean[0])]
            row["judge"] += passed[(t, clean[pick])]
            row["oracle"] += any(passed[(t, q)] for q in clean)
        out[str(k)] = row
    return out


def num(x):
    return "-" if x is None else "%.3f" % x


def main(argv=None):
    common.utf8_stdio()
    a = parse_args(argv)
    t0 = time.time()
    url = endpoint(a.url)
    suite = {t["id"]: t for t in jl(a.suite) if "id" in t}
    if not suite:
        print("error: no tasks in %s" % a.suite, file=sys.stderr)
        return 2
    for f in (a.pre, a.pre_each):
        if not os.path.exists(f):
            print("error: no %s" % f, file=sys.stderr)
            return 2
    items, missing = pre_set(suite, a.pre, a.pre_each)
    if not items:
        print("error: no program of %s runs clean by %s" % (a.pre, a.pre_each), file=sys.stderr)
        return 2
    print("pre: %d programs run clean (%d pass), %d without a program" % (
        len(items), sum(r["label"] for r in items), missing))
    names = ["pre"]
    for path in a.sets:
        name = set_name(path)
        if not os.path.exists(path):
            print("WARNING: no %s: set %s left out" % (path, name))
            continue
        rows, bad = file_set(name, path, suite)
        print("%s: %d records (%d pass), %d unread, from %s" % (name, len(rows), sum(r["label"] for r in rows),
                                                                  bad, os.path.basename(path)))
        items += rows
        names.append(name)
    # The candidates: every tries file's drafts, each distinct clean program graded once.
    files, drafts, pairs = expand(a.candidates), {}, []
    for f in files:
        drafts[f] = tries_of(f, suite)
        for t, progs in sorted(drafts[f].items()):
            pairs += [(t, q) for q in progs if q is not None]
    pairs = list(dict.fromkeys(pairs))
    passed = {}
    if pairs:
        iq = a.iq or os.environ.get("IQ") or os.path.join(os.path.dirname(os.path.abspath(a.suite)), "bin", "iq.exe")
        grades = grade(iq, a.suite, pairs)
        if grades is None:
            print("WARNING: no iq at %s: the candidates are left out" % iq)
            files, pairs = [], []
        else:
            passed = dict(zip(pairs, grades))
            print("cand: %d distinct clean programs (%d pass) from %d tries files" % (
                len(pairs), sum(grades), len(files)))
    elif a.candidates:
        print("cand: no clean try in %d tries files (%s)" % (len(files), " ".join(a.candidates)))
    cand = {}
    for t, q in pairs:
        cand[(t, q)] = item("cand", suite, t, q, passed[(t, q)])
    items += list(cand.values())
    if cand:
        names.append("cand")

    # The judgements: from the cache, else asked.
    cache = {}
    for x in jl(a.out):
        if "sha" in x and x.get("how") in ("lp", "nolp", "noyn", "context"):
            cache[x["sha"]] = {k: x.get(k) for k in RESULT}
    if os.path.exists(a.out) and os.path.getsize(a.out):
        with open(a.out, "rb") as f:
            f.seek(-1, os.SEEK_END)
            cut = f.read(1) != b"\n"
        if cut:  # a line cut off by a crash: the next one starts on its own line
            with open(a.out, "a", encoding="utf-8", newline="\n") as f:
                f.write("\n")
    first = {}
    for r in items:
        first.setdefault(r["sha"], r)
    todo = [r for s, r in first.items() if s not in cache]
    print("%d prompts: %d in the cache, %d to ask at %s" % (len(first), len(first) - len(todo), len(todo), url))
    lock, count = threading.Lock(), {"requests": 0, "errors": 0}

    def line(r, res):
        x = {k: r[k] for k in ("set", "task", "family", "tier", "label", "len", "sha")}
        x.update(res)
        return json.dumps(x, ensure_ascii=False)

    def one(r):
        try:
            res = ask(url, r["messages"], a.timeout)
        except Exception as e:  # the server gone, a timeout: counted, not recorded, asked again next run
            with lock:
                count["requests"] += 1
                count["errors"] += 1
                if count["errors"] <= 5:
                    print("error: %s %s" % (r["task"], e), file=sys.stderr)
            return
        with lock:
            count["requests"] += 1
            cache[r["sha"]] = res
            with open(a.out, "a", encoding="utf-8", newline="\n") as f:
                f.write(line(r, res) + "\n")
            if count["requests"] % 50 == 0:
                print("%d/%d asked, %.0fs" % (count["requests"], len(todo), time.time() - t0), flush=True)

    with ThreadPoolExecutor(max_workers=max(1, a.jobs)) as pool:
        list(pool.map(one, todo))
    done = [r for r in items if r["sha"] in cache]
    common.write_atomic(a.out, "".join(line(r, cache[r["sha"]]) + "\n" for r in done))
    for r in items:
        r.update(cache.get(r["sha"], {"p": None, "how": "error"}))

    asked = [cache[s] for s in first if s in cache]
    models = sorted({x["model"] for x in asked if x.get("model")})
    fit = {"url": url, "model": models[0] if len(models) == 1 else models, "sets": {}, "cand": {},
           "requests": count["requests"], "errors": count["errors"], "cached": len(first) - len(todo),
           "prompts": len(first), "nolp": sum(1 for x in asked if x["how"] in ("nolp", "noyn")),
           "noyn": sum(1 for x in asked if x["how"] == "noyn"),
           "context": sum(1 for x in asked if x["how"] == "context"),
           "request_ms": sum(x.get("ms") or 0 for x in asked)}
    for name in names:
        rows = [r for r in items if r["set"] == name and r["how"] != "error"]
        f = fit["sets"][name] = fit_set(rows)
        au, lift = f["auc"], f["lift"]
        print("%-5s %3d (%d yes / %d no, %d families%s): judge %s [%s, %s]  length %s [%s, %s]  tier %s  "
              "rank-average %s, lift %s [%s, %s]%s" % (
                  name, f["n"], f["yes"], f["no"], f["families"],
                  ", %d left out" % f["left_out"] if f["left_out"] else "",
                  num(au["judge"]["auc"]), num(au["judge"]["lo"]), num(au["judge"]["hi"]),
                  num(au["length"]["auc"]), num(au["length"]["lo"]), num(au["length"]["hi"]),
                  num(au["tier"]["auc"]), num(au["rankavg"]["auc"]),
                  num(lift["auc"]), num(lift["lo"]), num(lift["hi"]), "  (decides)" if name == "pre" else ""))
        w = f["within"]
        if w["judge"]["pairs"]:
            print("      within tasks, %d pairs of %d tasks: judge %s  length %s" % (
                w["judge"]["pairs"], w["judge"]["tasks"], num(w["judge"]["auc"]), num(w["length"]["auc"])))
    p = {k: r["p"] for k, r in cand.items()}
    for f in files:
        progs = drafts[f]
        most = max((len(v) for v in progs.values()), default=0)
        ks = [k for k in KS if k < most] + ([most] if most else [])
        name = os.path.basename(f)
        name = name[len("tries-"):] if name.startswith("tries-") else name
        name = name[:-len(".jsonl")] if name.endswith(".jsonl") else name
        fit["cand"][name] = at_k(progs, p, passed, ks)
        print("judge@k %s: %s" % (name, "  ".join(
            "@%s first %d judge %d oracle %d (%d clean, %d with a choice)" % (
                k, v["first"], v["judge"], v["oracle"], v["clean"], v["choices"])
            for k, v in fit["cand"][name].items())))
    fit["seconds"] = round(time.time() - t0, 1)
    print("judge: %d requests, %d errors, %d nolp (%d without yes or no), %d beyond the context, %d cached; %.0fs" % (
        fit["requests"], fit["errors"], fit["nolp"], fit["noyn"], fit["context"], fit["cached"], fit["seconds"]))
    if count["errors"]:
        print("unwritten %d: %s not written; a rerun asks only those" % (count["errors"], a.fit))
        return 3
    common.write_json(a.fit, fit)
    return 0


if __name__ == "__main__":
    sys.exit(main())
