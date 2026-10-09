#!/usr/bin/env python
"""judge.py without a model: a stub llama-server (http.server on 127.0.0.1, a free port) answers
each judgement with logprobs that say yes as much as the program is short, so the judge's AUC
must equal the length baseline's exactly, intervals and all; then without logprobs (the text
read instead, counted as nolp), failing (exit 3, nothing fitted, a rerun asks only what failed),
and beyond the context (recorded, left out of the AUCs).

  python train/test_judge.py     # with nightprep.py beside it, its stripping must agree; with
                                 # the data root's judge sets, the rendering must be judgedata's

No GPU, no network beyond 127.0.0.1."""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
# train/select.py would shadow the standard select, which socket, selectors and http.server's
# socketserver need: the real one first, with this folder off the path.
_path = sys.path[:]
sys.path[:] = [p for p in sys.path if os.path.normcase(os.path.abspath(p or ".")) != os.path.normcase(HERE)]
import select  # noqa: E402,F401
import http.server  # noqa: E402
import itertools  # noqa: E402
import json  # noqa: E402
import random  # noqa: E402
import socket  # noqa: E402
import subprocess  # noqa: E402
import tempfile  # noqa: E402
import threading  # noqa: E402
import unittest  # noqa: E402

sys.path[:] = [HERE] + _path
import judge  # noqa: E402

ROOT = os.environ.get("COMPUTEHUB_DATA") or r"C:\sept30\computehub-data"
PREP = os.path.join(ROOT, "scratch", "prep")


def program_in(user):
    """The program the prompt shows, read independently of judge.USER."""
    return user.split("```app\n", 1)[1].rsplit("\n```\n\nDoes it do everything asked?", 1)[0]


class Stub(http.server.BaseHTTPRequestHandler):
    """A llama-server's /v1/chat/completions, as the server's mode says: "lp" (yes the more the
    program is short: yes logprob -len/1000, split over "yes" and " Yes"; no fixed), "text" (no
    logprobs: yes for a program under 60 characters). Programs in .fail get HTTP 500, in .overflow
    the server's context error."""

    def do_POST(self):
        s = self.server
        body = json.loads(self.rfile.read(int(self.headers["content-length"])).decode("utf-8"))
        prog = program_in(body["messages"][1]["content"])
        with s.lock:
            s.bodies.append(body)
        if prog in s.fail:
            return self.send(500, {"error": {"code": 500, "message": "boom"}})
        if prog in s.overflow:
            return self.send(400, {"error": {"code": 400, "type": "exceed_context_size_error",
                                             "message": "the request exceeds the available context size, try increasing it"}})
        n = len(prog)
        choice = {"index": 0, "finish_reason": "length",
                  "message": {"role": "assistant", "content": "yes" if n < 60 else "no"}}
        if s.mode == "lp":
            top = [{"token": "yes", "logprob": -n / 1000}, {"token": "no", "logprob": -1.0},
                   {"token": " Yes", "logprob": -n / 1000 - 2}, {"token": "No", "logprob": -3.0},
                   {"token": "maybe", "logprob": -0.5}, {"token": "yes!", "logprob": -0.1}]
            choice["logprobs"] = {"content": [{"token": top[0]["token"], "logprob": top[0]["logprob"],
                                               "top_logprobs": top}]}
        self.send(200, {"choices": [choice], "model": "stub-" + s.mode})

    def send(self, code, obj):
        data = json.dumps(obj).encode("utf-8")
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass


def serve(mode):
    s = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Stub)
    s.mode, s.lock, s.bodies, s.fail, s.overflow = mode, threading.Lock(), [], set(), set()
    threading.Thread(target=s.serve_forever, daemon=True).start()
    return s


def free_port():
    with socket.socket() as k:
        k.bind(("127.0.0.1", 0))
        return k.getsockname()[1]


def prog(i, n):
    """A program of task i, about n characters of code, with comments the judge never sees."""
    body = "".join('label "%s";\n' % ("x" * (j % 7 + 1)) for j in range(n // 12 + 1))
    return "// Wrong: a giveaway\n// icon: dot 12 12 2\nstate n%d = 0; // a count\n\n\n%s" % (i, body)


def fixture(d):
    """A suite of 12 tasks in 6 families (tiers 1-6), pre's answers and grades, a glm set, an alt
    set (three programs a task), a tries file: {name: path}, and the programs by role."""
    rng = random.Random(3)
    tasks = [{"id": "t%d" % i, "family": "f%d" % (i // 2), "tier": i // 2 + 1, "ask": "ask %d: a label" % i,
              "check": "", "ref": ""} for i in range(12)]
    p = {"suite": os.path.join(d, "iq.jsonl"), "pre": os.path.join(d, "answers-pre.jsonl"),
         "each": os.path.join(d, "each-pre.txt"), "glm": os.path.join(d, "judge-eval-glm-s.jsonl"),
         "alt": os.path.join(d, "judge-eval-alt-s.jsonl"), "tries": os.path.join(d, "tries-b3.jsonl")}
    def w(path, rows):
        with open(path, "w", encoding="utf-8", newline="\n") as f:
            f.write("".join(json.dumps(r) + "\n" for r in rows))

    def rec(t, q, ok):
        return {"messages": judge.render(t["ask"], q) + [{"role": "assistant", "content": "yes" if ok else "no"}],
                "task": t["id"], "family": t["family"], "kind": "judge", "label": ok}

    def fence(q):
        return "```app\n" + q + "\n```"

    w(p["suite"], tasks)
    stages = ["pass", "check", "pass", "compile", "check", "pass", "pass", "check", "reply", "pass", "check", "pass"]
    pre = {t["id"]: prog(i, rng.randrange(20, 400)) for i, t in enumerate(tasks)}
    w(p["pre"], [{"task": t, "model": "glm+pre", "reply": "```app\n" + q + "```"} for t, q in pre.items()])
    w(p["each"], [{"task": t["id"], "model": "glm+pre", "pass": s == "pass", "stage": s, "code": 0}
                  for t, s in zip(tasks, stages)])
    # Every program distinct (its own state name), so every record is its own prompt.
    w(p["glm"], [rec(t, prog(100 + i, rng.randrange(20, 400)), i % 3 != 0) for i, t in enumerate(tasks)])
    w(p["alt"], [rec(t, prog(200 + i * 10 + k, rng.randrange(20, 400)), k == 0)
                 for i, t in enumerate(tasks[:6]) for k in range(3)])
    # Tries: t0 clean at try 0 (long, fails) and 2 (short, passes); t1 never clean; t2 clean at
    # try 1 only; t3 the same program three times.
    long_bad, short_ok = prog(300, 300) + 'label "bad";\n', prog(301, 30) + 'label "ok";\n'
    mid_ok, same = prog(302, 100) + 'label "ok";\n', prog(303, 50) + 'label "bad";\n'
    tries = [("t0", 0, long_bad, True), ("t0", 1, short_ok, False), ("t0", 2, short_ok, True),
             ("t1", 0, long_bad, False), ("t1", 1, short_ok, False), ("t1", 2, mid_ok, False),
             ("t2", 0, long_bad, False), ("t2", 1, mid_ok, True), ("t2", 2, long_bad, False),
             ("t3", 0, same, True), ("t3", 1, same, True), ("t3", 2, same, True)]
    rows = [{"task": t, "try": k, "clean": c, "reply": fence(q), "seed": 1} for t, k, q, c in tries]
    rows.insert(2, {"task": "t0", "try": 2, "clean": False, "reply": fence(long_bad)})  # cut off, then rerun
    rows.append({"task": "t4", "try": 0, "error": "context"})
    w(p["tries"], rows)
    return p


def fake_grade(iq, suite, pairs):
    """The real check, as the fixture means it: a program that says "ok" passes."""
    return ['label "ok";' in q for _, q in pairs]


class Strip(unittest.TestCase):
    CASES = [
        '// Wrong: x\n// icon: dot 1 1 1\nstate n = 0; // count\n\n\n\nlabel "http://x"; // c\nlabel "a\\"//b";\n',
        'a\n  \n\t\nb\n\n',
        '/* one\n  /* nested */ still\n*/ label "x"; /* in */ label "y";\n// tail\n',
        'label "/* not */ //";\nlabel "\\\\"; // after an escaped backslash\n',
        '\n\n// only\n\nstate a = 1;   \n',
        'label "open // string\nlabel "b";\n',
    ]

    def test_cases(self):
        self.assertEqual(judge.strip(self.CASES[0]), 'state n = 0;\n\nlabel "http://x";\nlabel "a\\"//b";')
        self.assertEqual(judge.strip(self.CASES[1]), "a\n\nb")
        self.assertEqual(judge.strip(self.CASES[2]), ' label "x";  label "y";')
        self.assertEqual(judge.strip(self.CASES[3]), 'label "/* not */ //";\nlabel "\\\\";')
        self.assertEqual(judge.strip(self.CASES[4]), "state a = 1;   ")

    def test_idempotent(self):
        for c in self.CASES:
            self.assertEqual(judge.strip(judge.strip(c)), judge.strip(c), repr(c))

    def test_as_nightprep(self):
        try:
            import nightprep
        except ImportError:
            self.skipTest("no nightprep.py")
        progs = list(self.CASES)
        for name in ("judge-eval-glm.jsonl", "judge-eval-alt.jsonl"):
            for r in judge.jl(os.path.join(PREP, name)):
                m = judge.USER.fullmatch(r["messages"][1]["content"])
                if m:
                    progs.append(m.group(2))
        for q in progs:
            self.assertEqual(judge.strip(q), nightprep.strip_comments(q), repr(q[:200]))
        print("strip: as nightprep's on %d programs" % len(progs))


class Render(unittest.TestCase):
    def test_render(self):
        m = judge.render("a counter", '\n// c\nlabel "x";\n')
        self.assertEqual(m[0], {"role": "system", "content": judge.SYSTEM})
        self.assertEqual(m[1], {"role": "user", "content": 'Asked: a counter\n\nProgram:\n```app\n// c\nlabel "x";'
                                                           '\n```\n\nDoes it do everything asked?'})
        self.assertEqual(judge.USER.fullmatch(m[1]["content"]).groups(), ("a counter", '// c\nlabel "x";'))

    def test_as_judgedata(self):
        # judgedata.rec() made these from the suite's asks: rendered again, each is the same.
        suite = os.path.join(ROOT, "iq", "team-n20261008", "iq.jsonl")
        src = os.path.join(PREP, "judge-eval-glm.jsonl")
        if not (os.path.exists(suite) and os.path.exists(src)):
            self.skipTest("no judge data")
        asks = {t["id"]: t["ask"] for t in judge.jl(suite)}
        rows = judge.jl(src)
        for r in rows:
            prog = judge.USER.fullmatch(r["messages"][1]["content"]).group(2)
            self.assertEqual(judge.render(asks[r["task"]], prog), r["messages"][:2])
        print("render: as judgedata's on %d records" % len(rows))


class Read(unittest.TestCase):
    def choice(self, top):
        return {"logprobs": {"content": [{"token": "x", "logprob": 0, "top_logprobs": top}]}}

    def test_yes_over_yes_and_no(self):
        import math
        top = [{"token": "Yes", "logprob": math.log(0.3)}, {"token": " yes ", "logprob": math.log(0.1)},
               {"token": "NO", "logprob": math.log(0.2)}, {"token": "maybe", "logprob": math.log(0.4)}]
        p, how, yes, no = judge.p_yes(self.choice(top))
        self.assertEqual(how, "lp")
        self.assertAlmostEqual(p, 0.4 / 0.6)
        self.assertAlmostEqual(yes, math.log(0.4))

    def test_tiny_masses(self):
        p, how, _, _ = judge.p_yes(self.choice([{"token": "yes", "logprob": -1000.0}, {"token": "no", "logprob": -1001.0}]))
        self.assertEqual(how, "lp")
        self.assertGreater(p, 0.7)
        self.assertEqual(judge.p_yes(self.choice([{"token": "no", "logprob": -2.0}]))[:2], (0.0, "lp"))

    def test_without(self):
        self.assertEqual(judge.p_yes({"message": {"content": "yes"}})[:2], (None, "nolp"))
        self.assertEqual(judge.p_yes(self.choice([{"token": "maybe", "logprob": -0.1}]))[:2], (None, "noyn"))

    def test_endpoint(self):
        for u in ("http://h:1", "http://h:1/", "http://h:1/v1", "http://h:1/v1/chat/completions"):
            self.assertEqual(judge.endpoint(u), "http://h:1/v1/chat/completions")


class Stats(unittest.TestCase):
    def test_auc(self):
        self.assertEqual(judge.auc([1, 2, 3, 4], [False, False, True, True]), 1.0)
        self.assertEqual(judge.auc([1, 1], [True, False]), 0.5)
        self.assertIsNone(judge.auc([1, 2], [True, True]))
        rng = random.Random(5)
        for _ in range(50):
            s = [rng.randrange(6) for _ in range(30)]
            y = [rng.random() < 0.5 for _ in range(30)]
            if all(y) or not any(y):
                continue
            pairs = [(a, b) for a, la in zip(s, y) if la for b, lb in zip(s, y) if not lb]
            want = sum(1.0 if a > b else 0.5 if a == b else 0.0 for a, b in pairs) / len(pairs)
            self.assertAlmostEqual(judge.auc(s, y), want)

    def test_within(self):
        rows = [{"task": "a", "label": True, "s": 3}, {"task": "a", "label": False, "s": 1},
                {"task": "a", "label": False, "s": 3}, {"task": "b", "label": True, "s": 0}]
        self.assertEqual(judge.within(rows, lambda r: r["s"]), {"pairs": 2, "tasks": 1, "auc": 0.75})

    def test_at_k(self):
        tries = {"a": ["long", None, "short"], "b": [None, None, None], "c": [None, "mid", None],
                 "d": ["same", "same", "same"]}
        p = {("a", "long"): 0.2, ("a", "short"): 0.9, ("c", "mid"): 0.5, ("d", "same"): 0.4}
        passed = {("a", "long"): False, ("a", "short"): True, ("c", "mid"): True, ("d", "same"): False}
        got = judge.at_k(tries, p, passed, [1, 2, 3])
        self.assertEqual(got["1"], {"drafts": 4, "clean": 2, "choices": 0, "first": 0, "judge": 0, "oracle": 0})
        self.assertEqual(got["2"], {"drafts": 4, "clean": 3, "choices": 0, "first": 1, "judge": 1, "oracle": 1})
        self.assertEqual(got["3"], {"drafts": 4, "clean": 3, "choices": 1, "first": 1, "judge": 2, "oracle": 2})
        # A tie goes to the earliest; an unjudged program (beyond the context) is never picked over one judged.
        self.assertEqual(judge.at_k({"a": ["x", "y"]}, {("a", "x"): 0.5, ("a", "y"): 0.5},
                                    {("a", "x"): False, ("a", "y"): True}, [2])["2"]["judge"], 0)
        self.assertEqual(judge.at_k({"a": ["x", "y"]}, {("a", "x"): None, ("a", "y"): 0.0},
                                    {("a", "x"): False, ("a", "y"): True}, [2])["2"]["judge"], 1)


class StubRuns(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.d = self.tmp.name
        self.p = fixture(self.d)
        self.grade = judge.grade
        judge.grade = fake_grade

    def tearDown(self):
        judge.grade = self.grade
        for s in getattr(self, "servers", []):
            s.shutdown()
            s.server_close()
        self.tmp.cleanup()

    def stub(self, mode):
        s = serve(mode)
        self.servers = getattr(self, "servers", []) + [s]
        return s

    def argv(self, url, name, cand=True):
        p = self.p
        return ["--url", url, "--suite", p["suite"], "--pre", p["pre"], "--pre-each", p["each"],
                "--sets", p["glm"], p["alt"], "--candidates"] + ([os.path.join(self.d, "tries-*.jsonl")] if cand else []) + \
               ["--iq", "fake", "--out", os.path.join(self.d, "judge-%s.jsonl" % name),
                "--fit", os.path.join(self.d, "judgefit-%s.json" % name), "--jobs", "4", "--timeout", "30"]

    def fit(self, name):
        with open(os.path.join(self.d, "judgefit-%s.json" % name), encoding="utf-8") as f:
            return json.load(f)

    def out(self, name):
        return judge.jl(os.path.join(self.d, "judge-%s.jsonl" % name))

    def test_length_judge_is_length(self):
        s = self.stub("lp")
        url = "http://127.0.0.1:%d" % s.server_address[1]
        self.assertEqual(judge.main(self.argv(url, "lp")), 0)
        f = self.fit("lp")
        self.assertEqual(sorted(f["sets"]), ["alt", "cand", "glm", "pre"])
        self.assertEqual((f["sets"]["pre"]["n"], f["sets"]["pre"]["yes"]), (10, 6))   # compile and reply left out
        self.assertEqual((f["sets"]["glm"]["n"], f["sets"]["alt"]["n"]), (12, 18))
        for name, st in f["sets"].items():
            j, ln = st["auc"]["judge"], st["auc"]["length"]
            self.assertIsNotNone(j["auc"], name)
            self.assertEqual(j, ln, name)                      # the AUC and its interval, exactly
            self.assertEqual(st["auc"]["rankavg"], ln, name)
            self.assertEqual(st["lift"], {"auc": 0.0, "lo": 0.0, "hi": 0.0}, name)
            self.assertEqual(st["within"]["judge"], st["within"]["length"], name)
            self.assertEqual(st["nolp"], 0, name)
            self.assertLessEqual(j["lo"], j["auc"])
            self.assertLessEqual(j["auc"], j["hi"])
            self.assertGreater(st["resamples"], 1900)
        self.assertEqual(f["sets"]["alt"]["within"]["judge"]["pairs"], 12)
        self.assertEqual(f["sets"]["alt"]["within"]["judge"]["tasks"], 6)
        self.assertNotEqual(f["sets"]["pre"]["auc"]["tier"], f["sets"]["pre"]["auc"]["length"])
        # Every distinct prompt asked once, as specified, the comments never shown.
        bodies = s.bodies
        self.assertEqual(f["requests"], len(bodies))
        self.assertEqual(f["prompts"], len(bodies))
        self.assertEqual(len({json.dumps(b["messages"]) for b in bodies}), len(bodies))
        for b in bodies:
            self.assertEqual({k: b[k] for k in ("max_tokens", "temperature", "seed", "logprobs", "top_logprobs", "stream")},
                             {"max_tokens": 1, "temperature": 0, "seed": 1, "logprobs": True, "top_logprobs": 20,
                              "stream": False})
            self.assertNotIn("//", program_in(b["messages"][1]["content"]))
            self.assertNotIn("\n\n\n", b["messages"][1]["content"])
        for r in self.out("lp"):
            self.assertEqual(r["how"], "lp")
        # judge@k: t0's short clean try (2) passes where its first clean (0) does not; t3's three
        # tries are one program; t1 never ran clean; t4's one line was an error.
        self.assertEqual(f["cand"]["b3"]["1"], {"drafts": 5, "clean": 2, "choices": 0, "first": 0, "judge": 0, "oracle": 0})
        self.assertEqual(f["cand"]["b3"]["2"], {"drafts": 5, "clean": 3, "choices": 0, "first": 1, "judge": 1, "oracle": 1})
        self.assertEqual(f["cand"]["b3"]["3"], {"drafts": 5, "clean": 3, "choices": 1, "first": 1, "judge": 2, "oracle": 2})
        self.assertEqual(f["sets"]["cand"]["n"], 4)
        # Again: every judgement from the cache, the same fit.
        s.bodies.clear()
        self.assertEqual(judge.main(self.argv(url, "lp")), 0)
        g = self.fit("lp")
        self.assertEqual((g["requests"], g["cached"], len(s.bodies)), (0, f["prompts"], 0))
        self.assertEqual(g["sets"], f["sets"])
        self.assertEqual(g["cand"], f["cand"])

    def test_length_is_the_program_shown(self):
        s = self.stub("lp")
        self.assertEqual(judge.main(self.argv("http://127.0.0.1:%d/v1" % s.server_address[1], "len", cand=False)), 0)
        by = {}
        for b in s.bodies:
            by[judge.common.prompt_hash(b["messages"])] = len(program_in(b["messages"][1]["content"]))
        rows = self.out("len")
        self.assertEqual(len(rows), 10 + 12 + 18)
        for r in rows:
            self.assertEqual(r["len"], by[r["sha"]])

    def test_text_fallback(self):
        s = self.stub("text")
        self.assertEqual(judge.main(self.argv("http://127.0.0.1:%d" % s.server_address[1], "text")), 0)
        f = self.fit("text")
        self.assertEqual(f["nolp"], f["prompts"])
        self.assertEqual(f["noyn"], 0)
        for st in f["sets"].values():
            self.assertEqual(st["nolp"], st["n"])
        for r in self.out("text"):
            self.assertEqual(r["how"], "nolp")
            self.assertEqual(r["p"], 1.0 if r["len"] < 60 else 0.0)

    def test_failure_then_resume(self):
        s = self.stub("lp")
        url = "http://127.0.0.1:%d" % s.server_address[1]
        bad = program_in(judge.jl(self.p["alt"])[4]["messages"][1]["content"])
        s.fail.add(judge.strip(bad))
        self.assertEqual(judge.main(self.argv(url, "f", cand=False)), 3)
        self.assertFalse(os.path.exists(os.path.join(self.d, "judgefit-f.json")))
        rows = self.out("f")
        self.assertEqual(len(rows), 10 + 12 + 18 - 1)
        s.fail.clear()
        s.bodies.clear()
        self.assertEqual(judge.main(self.argv(url, "f", cand=False)), 0)
        self.assertEqual(len(s.bodies), 1)
        f = self.fit("f")
        self.assertEqual((f["requests"], f["errors"], f["sets"]["alt"]["n"]), (1, 0, 18))

    def test_context(self):
        s = self.stub("lp")
        url = "http://127.0.0.1:%d" % s.server_address[1]
        r = judge.jl(self.p["pre"])[0]
        s.overflow.add(judge.strip(judge.program_of(r["reply"])[0]))
        self.assertEqual(judge.main(self.argv(url, "c", cand=False)), 0)
        f = self.fit("c")
        self.assertEqual((f["context"], f["sets"]["pre"]["n"], f["sets"]["pre"]["left_out"]), (1, 9, 1))
        self.assertEqual(f["sets"]["pre"]["auc"]["judge"], f["sets"]["pre"]["auc"]["length"])

    def test_cut_cache_line(self):
        s = self.stub("lp")
        url = "http://127.0.0.1:%d" % s.server_address[1]
        self.assertEqual(judge.main(self.argv(url, "cut", cand=False)), 0)
        out = os.path.join(self.d, "judge-cut.jsonl")
        with open(out, encoding="utf-8") as f:
            text = f.read()
        with open(out, "w", encoding="utf-8", newline="\n") as f:
            f.write(text[:-40])   # a crash mid-line
        s.bodies.clear()
        self.assertEqual(judge.main(self.argv(url, "cut", cand=False)), 0)
        self.assertEqual(len(s.bodies), 1)
        self.assertEqual(len(self.out("cut")), 10 + 12 + 18)

    def test_cli_exit_codes(self):
        # A dead server: every request fails, exit 3, no fit. A missing input: exit 2.
        url = "http://127.0.0.1:%d" % free_port()
        a = self.argv(url, "dead", cand=False)
        r = subprocess.run([sys.executable, os.path.join(HERE, "judge.py")] + a, capture_output=True, timeout=120)
        self.assertEqual(r.returncode, 3, r.stderr.decode("utf-8", "replace")[-500:])
        self.assertIn(b"unwritten 40", r.stdout)
        self.assertFalse(os.path.exists(os.path.join(self.d, "judgefit-dead.json")))
        a[a.index("--pre") + 1] = os.path.join(self.d, "nothing.jsonl")
        r = subprocess.run([sys.executable, os.path.join(HERE, "judge.py")] + a, capture_output=True, timeout=120)
        self.assertEqual(r.returncode, 2)


class RealIq(unittest.TestCase):
    """With IQ set: the candidates graded by iq itself (the suite's references pass, a broken one
    does not)."""

    def test_grade(self):
        iq = os.environ.get("IQ")
        suite = os.path.join(ROOT, "iq", "team-n20261008", "iq.jsonl")
        if not iq or not os.path.exists(suite):
            self.skipTest("IQ not set")
        tasks = list(itertools.islice(judge.jl(suite), 3))
        pairs = [(t["id"], t["ref"]) for t in tasks] + [(tasks[0]["id"], "// broken\nlabel \"x\";\n")]
        self.assertEqual(judge.grade(iq, suite, pairs), [True, True, True, False])


if __name__ == "__main__":
    unittest.main()
