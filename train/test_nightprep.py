#!/usr/bin/env python
"""nightprep.py and ask.py's exit status, without a GPU or a model.

  python train/test_nightprep.py      # with IQ=path/to/iq.exe, also on the night's real inputs

Without IQ: the comment stripper against applang's lexer rules, FNV-1a 64, the AUC, the dev,
finish and pick choices on made-up records, the subcommands' files (kept once made, their sha
recorded once, a wrong sha refused, a missing input exit 2), clean, and ask.py against a dead
URL (exit 3) and against train/stubhelper.py (a prompt beyond the context left out). With IQ and the data root's files (COMPUTEHUB_DATA, else C:\\sept30\\computehub-data):
every number the night plan states of them, graded by iq."""
import json
import os
import socket
import subprocess
import sys
import tempfile
import time
import unittest
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import nightprep as np  # noqa: E402

SCRIPT = os.path.join(HERE, "nightprep.py")


def run(*args):
    """nightprep.py args: (exit status, stdout, stderr)."""
    r = subprocess.run([sys.executable, SCRIPT] + list(args), capture_output=True,
                       env=dict(os.environ, PYTHONIOENCODING="utf-8"))
    return r.returncode, r.stdout.decode("utf-8"), r.stderr.decode("utf-8")


def write(path, rows):
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write("".join((r if isinstance(r, str) else json.dumps(r, ensure_ascii=False)) + "\n" for r in rows))


def read(path):
    with open(path, encoding="utf-8") as f:
        return [json.loads(l) for l in f if l.strip()]


class Comments(unittest.TestCase):
    def test_whole_lines_and_trailing(self):
        src = ("// Wrong: the timer never stops\n// icon: ring 12 12 9\nstate n = 0;  // the count\n"
               "\n\n\nlabel \"a // b\";\nlabel \"say \\\"hi\\\" // x\"; // gone\nbutton \"+\" { n += 1; }\n")
        self.assertEqual(np.strip_comments(src),
                         "state n = 0;\n\nlabel \"a // b\";\nlabel \"say \\\"hi\\\" // x\";\nbutton \"+\" { n += 1; }")
        self.assertEqual(np.comments(np.strip_comments(src)), 0)
        self.assertEqual(np.comments(src), 4)

    def test_block_comments_nest(self):
        src = "a; /* one /* two */ still */ b;\n/* open\n\nspans // lines\n*/ c;\n/* whole */\nd;"
        self.assertEqual(np.strip_comments(src), "a;  b;\n c;\nd;")
        self.assertEqual(np.comments(src), 6)

    def test_strings_hold_no_comment(self):
        for line in ('label "http://x";', 'label "/* no */";', 'label "\\\\" ;', 'label "a\\nb//";',
                     'label "unterminated // to the end'):
            self.assertEqual(np.strip_comments(line), line, line)
            self.assertEqual(np.comments(line), 0, line)
        # An escaped backslash ends before the quote: the string closes there.
        self.assertEqual(np.strip_comments('label "\\\\"; // c'), 'label "\\\\";')

    def test_blank_runs(self):
        self.assertEqual(np.strip_comments("\n\n// a\n\nx;\n\n \n\t\ny;\n\n"), "x;\n\ny;")


class Numbers(unittest.TestCase):
    def test_fnv1a64(self):
        self.assertEqual(np.fnv1a64(""), 0xcbf29ce484222325)
        self.assertEqual(np.fnv1a64("a"), 0xaf63dc4c8601ec8c)
        self.assertEqual(np.fnv1a64("foobar"), 0x85944171f73967e8)

    def test_auc(self):
        self.assertEqual(np.auc([3, 2, 1, 0], [True, True, False, False]), 1.0)
        self.assertEqual(np.auc([0, 1, 2, 3], [True, True, False, False]), 0.0)
        self.assertEqual(np.auc([1, 1, 1], [True, False, True]), 0.5)
        self.assertEqual(np.auc([1, 2, 2, 3], [False, True, False, True]), 0.875)
        self.assertIsNone(np.auc([1, 2], [True, True]))


class Choices(unittest.TestCase):
    def test_dev(self):
        def p(task, kind, program="// x\nstate a = 1;\n", arm="repair"):
            return {"task": task, "kind": kind, "code": 301, "arm": arm, "program": program}
        pool = [p("one", "name"), p("one", "state"), p("one", "decl"), p("two", "name"), p("two", "cast"),
                p("three", "bound1", "// x\n\n\nstate a = 1;"), p("four", "state", arm="review"),
                p("five", "guard", "// first\n"), p("five", "guard", "// second\n"), p("six", "odd")]
        rows = np.dev_pick(pool, {"one": "one-fam"}, {"held"})
        self.assertEqual([r["task"] for r in rows], ["five", "one", "two"])
        one = min(("state", "decl"), key=lambda k: np.fnv1a64("one" + k))
        two = min(("name", "cast"), key=lambda k: np.fnv1a64("two" + k))
        self.assertEqual([r["kind"] for r in rows], ["guard", one, two])
        self.assertEqual(rows[0], {"task": "five", "model": "mut-guard", "kind": "guard", "code": 301,
                                   "reply": "```app\n// first\n```"})
        with self.assertRaises(np.Fail):
            np.dev_pick(pool, {"one": "held-out"}, {"held"})

    def test_finish(self):
        prompt = lambda t, sys_len=100: {"task": t, "family": t, "max_tokens": 6144, "temperature": 0.3,
                                         "messages": [{"role": "system", "content": "s" * sys_len},
                                                      {"role": "user", "content": "Make: " + t}]}
        each = [{"task": "a", "stage": "reply"}, {"task": "b", "stage": "compile"}, {"task": "c", "stage": "reply"}]
        answers = [{"task": "a", "reply": "x" * 100 + "y" * 8000}, {"task": "c", "reply": "short"},
                   {"task": "c", "reply": "a later answer"}]
        room = (16384 - 6144) * 2.96
        # c's system prompt leaves room for 2 characters of notes: "short" is cut to "rt".
        full = int(room) - 1 - len("Make: c") - len(np.NOTES) - len(np.WRITE_NOW) - 2
        rows = np.finish_prompts(answers, each, [prompt("c", full), prompt("b"), prompt("a")])
        self.assertEqual([r["task"] for r in rows], ["c", "a"])
        a = rows[1]["messages"][-1]["content"]
        self.assertEqual(a, "Make: a" + np.NOTES + "y" * 8000 + np.WRITE_NOW)
        self.assertEqual((rows[1]["max_tokens"], rows[1]["temperature"]), (6144, 0.3))
        self.assertEqual(rows[0]["messages"][-1]["content"], "Make: c" + np.NOTES + "rt" + np.WRITE_NOW)
        for r in rows:
            self.assertLess(sum(len(m["content"]) for m in r["messages"]), room)
        with self.assertRaises(np.Fail):   # no answer to a task at stage reply
            np.finish_prompts([], each, [prompt("a")])
        with self.assertRaises(np.Fail):   # no room even without notes
            np.finish_prompts(answers, each, [prompt("a", int(room))])

    def test_pick(self):
        s = [{"task": t} for t in ("a", "a", "a", "b", "b", "c", "c")]
        g = [{"stage": x} for x in ("compile", "pass", "pass", "smoke", "compile", "compile", "compile")]
        self.assertEqual(np.choose(s, g), [1, 3, 5])


class Files(unittest.TestCase):
    def test_suite_kept_once_sha_once(self):
        with tempfile.TemporaryDirectory() as d:
            src, out = os.path.join(d, "iq.jsonl"), os.path.join(d, "suite-wait.jsonl")
            write(src, [{"id": "t1", "check": "expect number = 1\n", "ask": "é"}, {"id": "t2", "check": "x"}])
            self.assertEqual(run("suite", "--suite", src, "--out", out)[0], 0)
            self.assertEqual([t["check"] for t in read(out)], [np.WAIT_CHECK] * 2)
            self.assertEqual(read(out)[0]["ask"], "é")
            with open(out, "rb") as f:
                self.assertNotIn(b"\r", f.read())
            write(src, [{"id": "t3", "check": "x"}])
            code, said, _ = run("suite", "--suite", src, "--out", out)
            self.assertEqual((code, "there already" in said), (0, True))
            self.assertEqual([t["id"] for t in read(out)], ["t1", "t2"])   # kept as it was
            log = os.path.join(d, "inputs.sha256")
            with open(log, encoding="utf-8") as f:
                self.assertEqual(f.read().splitlines(), ["%s *suite-wait.jsonl" % np.common.sha256_file(out)])
            with open(log, "w", encoding="utf-8", newline="\n") as f:   # a last line with no line break
                f.write("0 *iq.jsonl")
            run("suite", "--suite", src, "--out", out)
            with open(log, encoding="utf-8") as f:
                self.assertEqual(f.read(), "0 *iq.jsonl\n%s *suite-wait.jsonl\n" % np.common.sha256_file(out))

    def test_pre_sha(self):
        with tempfile.TemporaryDirectory() as d:
            src, out = os.path.join(d, "mx.jsonl"), os.path.join(d, "answers-pre.jsonl")
            write(src, [{"task": "a", "model": "m", "reply": "r"}])
            sha = np.common.sha256_file(src)
            code, _, err = run("pre", "--src", src, "--sha", "0" * 16 if sha[0] != "0" else "1" * 16, "--out", out)
            self.assertEqual((code, os.path.exists(out)), (1, False), err)
            self.assertEqual(run("pre", "--src", src, "--sha", "", "--out", out)[0], 1)   # no prefix
            self.assertEqual(run("pre", "--src", src, "--sha", sha[:16], "--out", out)[0], 0)
            self.assertEqual(np.common.sha256_file(out), sha)
            code, _, err = run("pre", "--src", os.path.join(d, "none.jsonl"), "--sha", "0", "--out", out + "2")
            self.assertEqual(code, 2)
            self.assertIn("needs", err)

    def test_clean(self):
        with tempfile.TemporaryDirectory() as d:
            src, out = os.path.join(d, "team-x.jsonl"), os.path.join(d, "clean.jsonl")
            good = {"task": "a", "model": "m", "reply": "r", "team": {"turns": 1}}
            write(src, [good, '{"task": "b", "model": "m", "rep', "", '{"task": "c", "model": "m"}', "[1]",
                        dict(good, task="d")])
            code, said, _ = run("clean", src, out)
            self.assertEqual(code, 0)
            self.assertIn("2 lines kept, 3 dropped", said)
            self.assertEqual([r["task"] for r in read(out)], ["a", "d"])


class Ask(unittest.TestCase):
    def test_dead_url_exits_3(self):
        with socket.socket() as s:   # a port nothing listens on
            s.bind(("127.0.0.1", 0))
            port = s.getsockname()[1]
        with tempfile.TemporaryDirectory() as d:
            prompts, out = os.path.join(d, "p.jsonl"), os.path.join(d, "a.jsonl")
            write(prompts, [{"task": "t", "max_tokens": 16, "temperature": 0.3,
                             "messages": [{"role": "user", "content": "hi"}]}])
            r = subprocess.run([sys.executable, os.path.join(HERE, "ask.py"), "--prompts", prompts, "--out", out,
                                "--name", "x", "--url", "http://127.0.0.1:%d" % port, "--k", "2", "--jobs", "2"],
                               capture_output=True, env=dict(os.environ, PYTHONIOENCODING="utf-8"))
            self.assertEqual(r.returncode, 3, r.stderr)
            self.assertIn("2 of 2 requests failed", r.stderr.decode("utf-8"))

    def test_stub_replies_and_a_prompt_beyond_the_context(self):
        # train/stubhelper.py as the server: by seed a program that does not compile (1), reasoning
        # read to the stream's end, past its usage chunk (2), a counter (3 mod 3 = 0); a prompt
        # holding the overflow word gets llama-server's 400, left out without failing the run.
        with socket.socket() as s:
            s.bind(("127.0.0.1", 0))
            port = s.getsockname()[1]
        stub = subprocess.Popen([sys.executable, os.path.join(HERE, "stubhelper.py"), "--port", str(port),
                                 "--overflow", "zzlong", "--delay", "0"], stdout=subprocess.DEVNULL,
                                stderr=subprocess.DEVNULL)
        try:
            for _ in range(100):
                try:
                    with urllib.request.urlopen("http://127.0.0.1:%d/health" % port, timeout=1) as resp:
                        if json.loads(resp.read())["status"] == "ok":
                            break
                except OSError:
                    time.sleep(0.1)
            with tempfile.TemporaryDirectory() as d:
                prompts, out = os.path.join(d, "p.jsonl"), os.path.join(d, "a.jsonl")
                ask = lambda t, text: {"task": t, "max_tokens": 16, "temperature": 0.3,
                                       "messages": [{"role": "user", "content": text}]}
                write(prompts, [ask("t", "a counter"), ask("long", "zzlong")])
                r = subprocess.run([sys.executable, os.path.join(HERE, "ask.py"), "--prompts", prompts, "--out", out,
                                    "--name", "x", "--url", "http://127.0.0.1:%d" % port, "--k", "3", "--jobs", "2"],
                                   capture_output=True, env=dict(os.environ, PYTHONIOENCODING="utf-8"))
                self.assertEqual(r.returncode, 0, r.stderr)
                self.assertIn("beyond the server's context, left out: long", r.stdout.decode("utf-8"))
                replies = sorted(x["reply"] for x in read(out))
                self.assertEqual([x["task"] for x in read(out)], ["t"] * 3)
                self.assertTrue(replies[0].startswith("First the state"), replies)
                self.assertTrue(replies[1].startswith("```app\n// Broken"), replies)
                self.assertTrue(replies[2].startswith("```app\n// Counter"), replies)
        finally:
            stub.terminate()
            stub.wait()


ROOT = os.environ.get("COMPUTEHUB_DATA") or r"C:\sept30\computehub-data"
TEAM = os.path.join(ROOT, "iq", "team-n20261009")
PREP = os.path.join(ROOT, "scratch", "prep")
NEEDED = [os.path.join(TEAM, f) for f in ("iq.jsonl", "answers-glm.jsonl", "held.txt", "prompts-held.jsonl")] + [
    os.path.join(PREP, f) for f in ("mx-suite-wait.jsonl", "mx-answers-pre.jsonl", "eng/pool-dev.jsonl",
                                    "judge-train.jsonl", "judge-eval-glm.jsonl", "judge-eval-alt.jsonl")] + [
    os.path.join(ROOT, "iq", "answers-n20261007-q3-gram.jsonl")]


def iq_binary():
    path = os.environ.get("IQ")
    return path if path and os.path.exists(path) else None


def each(answers, suite):
    r = subprocess.run([iq_binary(), "score", answers, "--suite", suite, "--each"], capture_output=True)
    assert r.returncode == 0, r.stderr
    return [json.loads(l) for l in r.stdout.decode("utf-8").split("\n") if l.startswith("{")]


@unittest.skipUnless(iq_binary() and all(os.path.exists(p) for p in NEEDED),
                     "set IQ to an iq binary, with the night's inputs in the data root")
class Night(unittest.TestCase):
    """The night plan's numbers, from the real inputs, made into a scratch folder."""

    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.d = cls.tmp.name

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def out(self, name):
        return os.path.join(self.d, name)

    def make(self, *args):
        code, said, err = run(*args)
        self.assertEqual(code, 0, err)
        print(said.strip())
        return said

    def wait(self):
        out = self.out("suite-wait.jsonl")
        if not os.path.exists(out):
            self.make("suite", "--suite", os.path.join(TEAM, "iq.jsonl"), "--out", out)
        return out

    def pre(self):
        out = self.out("answers-pre.jsonl")
        if not os.path.exists(out):
            self.make("pre", "--src", os.path.join(PREP, "mx-answers-pre.jsonl"), "--sha", "2e24176c71a87e68",
                      "--out", out)
        return out

    def test_suite_wait_grades_as_before(self):
        glm = os.path.join(TEAM, "answers-glm.jsonl")
        key = lambda g: (g["task"], g["pass"], g["stage"], g["code"], g["message"])
        ours, theirs = each(glm, self.wait()), each(glm, os.path.join(PREP, "mx-suite-wait.jsonl"))
        self.assertEqual(len(ours), 347)
        self.assertEqual([key(g) for g in ours], [key(g) for g in theirs])

    def test_pre(self):
        self.assertTrue(np.common.sha256_file(self.pre()).startswith("2e24176c71a87e68"))
        grades = each(self.pre(), os.path.join(TEAM, "iq.jsonl"))
        stages = {s: sum(g["stage"] == s for g in grades) for s in np.STAGES}
        self.assertEqual(stages, {"reply": 25, "compile": 28, "smoke": 11, "harness": 0, "check": 114, "pass": 169})

    def test_dev(self):
        out = self.out("answers-dev.jsonl")
        self.make("dev", "--pool", os.path.join(PREP, "eng", "pool-dev.jsonl"), "--held",
                  os.path.join(TEAM, "held.txt"), "--out", out)
        rows = read(out)
        self.assertEqual(len(rows), 149)
        self.assertFalse(any("\n\n\n" in r["reply"] for r in rows))
        family = {t["id"]: t["family"] for t in read(os.path.join(TEAM, "iq.jsonl"))}
        roots = np.held_roots(os.path.join(TEAM, "held.txt"))
        self.assertEqual([r["task"] for r in rows if np.root(family[r["task"]]) in roots], [])
        grades = each(out, os.path.join(TEAM, "iq.jsonl"))
        self.assertEqual({g["stage"] for g in grades}, {"compile", "smoke"})
        self.assertEqual(len(grades), 149)

    def test_judge(self):
        said = self.make("judge", "--src", PREP, "--out", self.d, "--held", os.path.join(TEAM, "held.txt"))
        roots = np.held_roots(os.path.join(TEAM, "held.txt"))
        for name, n in (("judge-train", 1838), ("judge-eval-glm", 226), ("judge-eval-alt", 257)):
            rows = read(self.out(name + "-s.jsonl"))
            self.assertEqual(len(rows), n)
            self.assertFalse(any("// Wrong" in json.dumps(r, ensure_ascii=False) for r in rows))
            for r in rows:
                prog = np.JUDGE_USER.match(r["messages"][1]["content"]).group(2)
                self.assertEqual(np.comments(prog), 0)
            if name == "judge-train":
                self.assertEqual([r["task"] for r in rows if np.root(r["family"]) in roots], [])
        self.assertIn("0.860 before stripping", said)

    def test_finish(self):
        each_pre = self.out("each-pre.txt")
        r = subprocess.run([iq_binary(), "score", self.pre(), "--suite", os.path.join(TEAM, "iq.jsonl"), "--each"],
                           capture_output=True)
        with open(each_pre, "wb") as f:
            f.write(r.stdout)
        out = self.out("prompts-fin.jsonl")
        self.make("finish", "--answers", os.path.join(TEAM, "answers-glm.jsonl"), "--each", each_pre,
                  "--prompts", os.path.join(TEAM, "prompts-held.jsonl"), "--out", out)
        rows = read(out)
        self.assertEqual(len(rows), 25)
        for r in rows:
            self.assertEqual(r["max_tokens"], 6144)
            self.assertLess(sum(len(m["content"]) for m in r["messages"]), (16384 - 6144) * 2.96)
            self.assertTrue(r["messages"][-1]["content"].endswith(np.WRITE_NOW))

    def test_pick_q3rec_makes_171(self):
        q3 = self.out("q3rec.jsonl")
        self.make("q3rec", "--src", os.path.join(ROOT, "iq", "answers-n20261007-q3-gram.jsonl"), "--out", q3)
        suite = os.path.join(TEAM, "iq.jsonl")
        before = {g["task"]: g for g in each(self.pre(), suite)}
        stuck = {t for t, g in before.items() if g["stage"] in ("reply", "compile", "smoke")}
        self.assertEqual(len(stuck), 64)
        samples = self.out("q3rec-stuck.jsonl")
        write(samples, [r for r in read(q3) if r["task"] in stuck])
        picked = self.out("q3pick.jsonl")
        self.make("pick", "--samples", samples, "--suite-wait", self.wait(), "--iq", iq_binary(), "--out", picked)
        clean = {r["task"]: r for r in read(picked) if r["pick"]["stage"] == "pass"}
        both = self.out("answers-pre-q3rec.jsonl")
        write(both, [dict(r, reply=clean[r["task"]]["reply"]) if r["task"] in clean else r for r in read(self.pre())])
        after = {g["task"]: g for g in each(both, suite)}
        self.assertEqual(sum(g["pass"] for g in after.values()), 171)
        self.assertEqual(sorted(t for t in after if after[t]["pass"] and not before[t]["pass"]),
                         ["bedtime-routine", "screensaver-bounce"])


if __name__ == "__main__":
    unittest.main()
