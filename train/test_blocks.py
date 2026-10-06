#!/usr/bin/env python
"""blocks.py against the coder's rules (programs/coder/src/ai.rs blocks, edits.rs marked,
honest and program; applang's lexer): where generate.py stops a sample and cuts its reply.

  python train/test_blocks.py         # with IQ=path/to/iq.exe, also against iq's own grades

No GPU, no torch. With IQ set, crafted replies are graded whole and cut by `iq score` (Rust's
coder::edits::program inside), and every grade must be the same."""
import json
import os
import random
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from blocks import block_end, blocks, honest, marked, program  # noqa: E402

F = "```"
GOOD = "// a\nlabel \"A\";\n"


def app(body, opener=F + "app", closer=F):
    return opener + "\n" + body + closer


class Rules(unittest.TestCase):
    def test_first_honest_block(self):
        r = "Here:\n" + app(GOOD) + "\nThat is all."
        self.assertEqual(r[:block_end(r)], "Here:\n" + app(GOOD))
        self.assertEqual(program(r), (GOOD, True))

    def test_marked_block_is_skipped(self):
        # coder::edits::program filters !marked: a SEARCH/REPLACE block is never the program.
        first = "// a\n<<<<<<< SEARCH\nx\n=======\ny\n>>>>>>> REPLACE\n"
        r = app(first) + "\n" + app("// real\nlabel \"A\";\n")
        end = block_end(r)
        self.assertEqual(end, len(r))
        self.assertEqual(program(r[:end]), program(r))
        self.assertEqual(program(r), ("// real\nlabel \"A\";\n", True))
        self.assertTrue(marked("x\n  <<<<<<< SEARCH\n"))   # trim_start takes Unicode spaces
        self.assertFalse(marked("x <<<<<<< SEARCH\n"))

    def test_lines_split_on_newline_only(self):
        for brk in ("\r", "\x0b", "\x0c", "\x1c", "\x1d", "\x1e", "\x85", " ", " "):
            r = F + "app" + brk + "// a\n" + F + "\n"
            self.assertIsNone(block_end(r), repr(brk))   # one line "```app<brk>// a": no opener
        r = app("// a\r\nlabel \"A\";\r\n", opener=F + "app\r", closer=F + "\r\n")
        self.assertEqual(r[:block_end(r)], r[:-2])          # \r\n is a line break with its \r

    def test_opener(self):
        for opener in (F + " app", F + "app  ", "  " + F + "\tapp", "　" + F + "app"):
            self.assertIsNotNone(block_end(app(GOOD, opener=opener)), repr(opener))
        for opener in (F + "application", F + "App", F + "ap p", F, "\x1c" + F + "app", "x" + F + "app"):
            self.assertIsNone(block_end(app(GOOD, opener=opener)), repr(opener))

    def test_closer(self):
        # Open, any line that trims to ``` and more closes it, an app fence too.
        for closer in (F, "  " + F + "  ", F + "app", F + "python", F + " and more", " " + F):
            r = app(GOOD, closer=closer) + "\nmore"
            self.assertEqual(r[:block_end(r)], app(GOOD, closer=closer), repr(closer))
        self.assertIsNone(block_end(app(GOOD, closer="``")))
        self.assertIsNone(block_end(app(GOOD, closer="\x1c" + F)))

    def test_honest(self):
        for src in ("// a", " \t\r\n// a", "/* a */ label", "/* unterminated", "\n\n//"):
            self.assertTrue(honest(src), repr(src))
        for src in ("", "label \"A\";", " // a", "\x0c// a", "　// a", "/ / a", "#// a"):
            self.assertFalse(honest(src), repr(src))

    def test_dishonest_then_honest(self):
        r = app("label \"no\";\n") + "\n" + app(" // no\n") + "\n" + app(GOOD) + "\n" + app("// later\n")
        end = block_end(r)
        self.assertEqual(r[end - len(app(GOOD)):end], app(GOOD))
        self.assertEqual(program(r[:end]), program(r))

    def test_open_block_never_stops(self):
        self.assertIsNone(block_end(F + "app\n// a\nlabel"))
        self.assertEqual(blocks(F + "app\n// a\n"), [("// a\n", False, len(F + "app\n// a\n"))])

    def test_longest_unmarked_fallback(self):
        # Rust compares byte lengths: "é" is two; of equals, the first.
        r = app("abc\n") + "\n" + app("éé\n") + "\n" + app("<<<<<<< x\nlonger and longer\n")
        self.assertIsNone(block_end(r))
        self.assertEqual(program(r), ("éé\n", True))
        self.assertEqual(program(app("ab\n") + "\n" + app("é\n")), ("ab\n", True))
        self.assertEqual(program(app("ab\n") + "\n" + app("open\n", closer="")), ("open\n", False))


PIECES = ["```app", "``` app", "```", "``` x", "  ```  ", "```app\r", "// note", "/* c */", " // sp",
          " // nbsp", "label \"A\";", "<<<<<<< SEARCH", "=======", ">>>>>>> REPLACE", "", "text",
          " ```app", "\x1c```", "\r", "\t// tab", "é", "```application"]


def random_reply(rng):
    return "".join(rng.choice(PIECES) + rng.choice(["\n", "\n", "\n", "\r\n", ""]) for _ in range(rng.randint(1, 14)))


class Properties(unittest.TestCase):
    def test_cut_keeps_the_program(self):
        # The grade on reply[:block_end(reply)] is the grade on the whole reply.
        rng = random.Random(7)
        stops = 0
        for _ in range(20000):
            r = random_reply(rng)
            end = block_end(r)
            if end is None:
                continue
            stops += 1
            text, closed = program(r)
            self.assertTrue(closed and honest(text) and not marked(text), repr(r))
            self.assertEqual(program(r[:end]), (text, closed), repr(r))
            self.assertEqual(block_end(r[:end]), end, repr(r))
        self.assertGreater(stops, 1000)

    def test_a_prefix_stops_where_the_whole_does(self):
        # Streaming: once a prefix stops, the whole reply's cut holds the same program.
        rng = random.Random(11)
        for _ in range(1500):
            r = random_reply(rng)
            whole = block_end(r)
            for cut in range(len(r) + 1):
                p = r[:cut]
                end = block_end(p)
                if end is not None:
                    self.assertIsNotNone(whole, repr(r))
                    self.assertEqual(program(p[:end]), program(r[:whole]), repr((r, cut)))
                    break


def iq_binary():
    path = os.environ.get("IQ")
    return path if path and os.path.exists(path) else None


@unittest.skipUnless(iq_binary(), "set IQ to an iq binary to grade against Rust")
class AgainstIq(unittest.TestCase):
    """Each crafted reply graded by `iq score` whole and as generate.py cuts it: the same grade."""

    def test_same_grade_whole_and_cut(self):
        suite = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "evals", "suites", "iq.jsonl")
        with open(suite, encoding="utf-8") as f:
            task = next(t for t in (json.loads(l) for l in f if l.strip()) if t["id"] == "counter")
        ref = task["ref"].rstrip("\n") + "\n"
        bad = "// broken: no such word\nlabel count count;\n"
        search = "// x\n<<<<<<< SEARCH\nlabel \"0\";\n=======\nlabel \"1\";\n>>>>>>> REPLACE\n"
        replies = [
            app(ref),
            app(search) + "\n" + app(ref),                        # marked, then the program
            app("label \"Hi\";\n") + "\n" + app(ref),             # not honest, then the program
            app(" " + bad) + "\n" + app(ref),                # NBSP: not a comment to applang
            app(bad) + "\n" + app(ref),                           # honest and broken: it is the one
            app(ref, opener=F + " app") + "\nnotes\n" + app(bad),  # "``` app" opens a block
            F + "app " + bad + F + "\n" + app(ref),          # U+2028 is no line break
            "\x1c" + F + "app\n" + bad + F + "\n" + app(ref),     # \x1c is no space to Rust
            app(ref, closer=F + "app") + "\n" + bad + F,           # an app fence closes one open
        ]
        cut = [r[:block_end(r)] if block_end(r) is not None else r for r in replies]
        with tempfile.TemporaryDirectory() as d:
            grades = []
            for name, rs in (("whole", replies), ("cut", cut)):
                path = os.path.join(d, name + ".jsonl")
                with open(path, "w", encoding="utf-8", newline="\n") as f:
                    for r in rs:
                        f.write(json.dumps({"task": "counter", "model": name, "reply": r}, ensure_ascii=False) + "\n")
                out = subprocess.run([iq_binary(), "score", path, "--suite", suite, "--each"], capture_output=True)
                self.assertEqual(out.returncode, 0, out.stderr)
                lines = [json.loads(l) for l in out.stdout.decode("utf-8").split("\n") if l.startswith("{")]
                grades.append([(g["pass"], g["stage"], g["code"], g["message"]) for g in lines])
        self.assertEqual(len(grades[0]), len(replies))
        self.assertEqual(grades[0], grades[1])
        self.assertTrue(all(g[0] for i, g in enumerate(grades[0]) if i != 4), grades[0])
        self.assertEqual(grades[0][4][1], "compile")
        self.assertEqual(sum(c != r for c, r in zip(cut, replies)), 3)   # cut: the 5th, 6th and 9th


if __name__ == "__main__":
    unittest.main()
