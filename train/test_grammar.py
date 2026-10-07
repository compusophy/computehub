#!/usr/bin/env python3
"""train/applang.gbnf, checked by llama.cpp's own grammar engine (tests/test-gbnf-validator.cpp):
every suite reference must pass, as the reply a model would write ("```app\\n" ref "\\n```"), and
so must the edge cases below that applang parses; the programs below that applang's lexer or
parser refuses, and the replies Studio would not read whole, must fail. With iq (applang's own
compiler, `iq grade`) each hand-written case is checked against it too: an accepted one must
compile, a refused one must fail with the code it names. Each reply the grammar accepts must
also be one Studio reads whole (blocks.block_end).

  python train/test_grammar.py [--validator EXE] [--iq EXE] [--sft SFT.jsonl] [--jobs N]
  python train/test_grammar.py --build DIR      # compile the validator into DIR first (Windows)

The validator: llama.cpp builds test-gbnf-validator only where tests/CMakeLists.txt allows it
(not a Windows shared-library build). --build compiles it straight from llama.cpp's sources, the
grammar engine (src/llama-grammar.cpp, src/unicode*.cpp) unchanged, with stubs for the three
llama_vocab methods it never calls (it inits the grammar with no vocab), using MSVC at below-
normal priority, CPU only. Elsewhere: cmake --build build --target test-gbnf-validator.
Exit status: 0 when every check holds."""

import argparse
import glob
import json
import os
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
sys.path.insert(0, HERE)
from blocks import block_end  # noqa: E402

LOW = getattr(subprocess, "BELOW_NORMAL_PRIORITY_CLASS", 0)
LLAMA = os.environ.get("LLAMA_DIR", r"C:\llama-cpp" if os.name == "nt" else os.path.expanduser("~/llama.cpp"))
EXE = "test-gbnf-validator" + (".exe" if os.name == "nt" else "")


def reply(src):
    """The reply a model writes for src: the fence, src, a line break if it has none, the fence."""
    return "```app\n" + src + ("" if src.endswith("\n") else "\n") + "```"


H = "// t\n"  # the comment an honest program starts with

# Programs applang compiles (so they parse): what the suite's references may not show.
ACCEPT = [
    ("no spaces", H + "state n = 0;\nif(n>0){label n;}else if(n<0){label-n;}else{label\"zero\";}\n"
                      "button\"+\"{n+=1;}button(\"-\"){n-=1;}\n"),
    ("stray semicolons", H + ";;state n = 0;;\n;label n;;\nrow { ; label 1; ; };\nbutton \"b\" { ; n = 1;; }\n"),
    ("nested block comments", "/* a /* b */ c */\n/***/ /* ** / */ /*/ */ /* //* */ */ /* x/**/y */\n"
                              "label /* in */ 1 /* between */;\n"),
    ("comments as separators", H + "state/*a*/n = 0;\nlabel/*b*/n;\nbutton \"b\" {\n  let//c\nx = 6/*d*/ / /*e*/ 2;\n"
                               "  n = x/2;\n  n = x/ 2 //f\n  ;\n}\n"),
    ("trailing commas", H + "state xs = [1, 2,];\nstate ss = [\"a\",];\nlabel len([1, 2,]) + len(xs) + len(ss);\n"),
    ("fills", H + "state a = [0; 4096];\nstate b = [\"\"; 3];\nstate c = [true; 0];\nstate d = [-1;1];\n"
                  "label len(a) + len(b) + len(c) + len(d) + len([0; 5]);\n"),
    ("negative literals", H + "state n = -5;\nstate xs = [-1, - 2, 3];\nlabel n + xs[0];\n"),
    ("names holding keywords", H + "state rows = 0;\nstate iffy = 1;\nstate inputs = 2;\nstate return_ = 3;\n"
                               "state _in = 4;\nstate forx = 5;\nstate Label = 6;\nstate truth = 7;\nstate fnord = 8;\n"
                               "label rows + iffy + inputs + return_ + _in + forx + Label + truth + fnord;\n"
                               "button \"b\" { let elseif = 1; let colour = elseif; rows = colour; }\n"),
    ("words that are names", H + "state grid = 0;\nstate canvas = 1;\nstate on = 2;\nstate key = 3;\n"
                             "state every = 4;\nstate saved = 5;\nstate int = 6;\n"
                             "label grid + canvas + on + key + every + saved + int;\n"
                             "button \"b\" { grid = 1; saved = 2; every += 1; }\n"),
    ("ints", H + "label 1_000_000 + 0 + 999999999999999999 + 1__2_;\nlabel 9223372036854775807;\n"
             "label 0009_223_372_036_854_775_807;\nlabel 1_000_000_000_000_000_000 + 00 + 0_0_;\n"),
    ("the least int", H + "state n = -9223372036854775808;\nstate xs = [-0_9_223_372_036_854_775_808, 0];\nlabel n;\n"),
    ("fill counts", H + "state a = [0; 04_096];\nstate b = [0; 0];\nstate c = [0; 000];\nstate d = [0; 4_0_9_5_];\n"
                    "label len(a) + len(b) + len(c) + len(d);\n"),
    ("strings", H + "label \"a\\\"b\\\\c\\nd\";\nlabel \"\u00b0C \u2666 ``` `x`\";\nlabel \"// not /* a comment\";\n"),
    ("backticks in a line comment", "// Name: uses `x` and ``` here\n// icon: dot 12 12\nlabel 1;\n"),
    ("if chains", H + "state n = 0;\nif n == 0 { label 0; } else if n == 1 { label 1; }\nelse if(n==2){label 2;}else{label 3;}\n"
                  "button \"b\" { if n > 0 { n = 0; } else if n < 0 { n = 1; } else { n = 2; } }\n"),
    ("functions", H + "fn add(a: int, b: int) -> int { return a + b; }\nfn f() { return; }\n"
                  "fn g()->bool{return true;}\nfn h(s:string,b:bool)->string{if b{return s;}return\"\";}\n"
                  "label add(1, 2);\nbutton \"b\" { f(); }\nlabel h(\"x\", g());\n"),
    ("every and on key", H + "state n = 0;\nevery 1000 { n += 1; }\nevery(500){n-=1;}\n"
                         "on key \"left\" { n -= 1; }\non key\"right\"{n+=1;}\non/**/key \"up\" { }\nlabel n;\n"),
    ("grid", H + "state cells = [0; 9];\ngrid 3, cells;\ngrid 3, cells, [\"\"; 9];\ngrid(3),cells{ cells[0] = 1; }\n"),
    ("canvas", H + "state n = 0;\nfn scene() { rect(0, 0, 10, 10, 1); }\ncanvas 100, 80, scene();\n"
               "canvas 100, 80, (scene()) { n = x + y; }\ncanvas(10),10,((scene()));\nlabel n;\n"),
    ("unary chains", H + "state n = 1;\nlabel - -n;\nlabel --n;\nlabel !!true;\nlabel n<-1;\nlabel n!=-1;\n"
                     "label 6/2;\nlabel 6 /2;\nlabel (((n)));\nlabel -(n) * -n % 3;\n"),
    ("index updates", H + "state xs = [0, 0];\nbutton \"b\" { xs[0] += 1; xs[1] = 2; xs [0] -= 1; let i = 1; xs[i - 1] = xs [i]; }\n"
                      "label xs[0] + xs [1];\n"),
    ("loops", H + "state xs = [1, 2, 3];\nfor i in 0..len(xs) { label xs[i]; }\nfor i in(0)..3{label i;}\n"
              "for i in -1..-(-2) { label i; }\nbutton \"b\" { repeat 3 { xs[0] += 1; } repeat(2){} for j in 0..2 { xs[j] = j; } }\n"),
    ("input", H + "state name = \"\";\ninput name;\ninput/**/name;\nlabel name;\n"),
    ("only comments", "// applang has no network: a chat needs one.\n"),
    ("a comment ends it", H + "label 1; // the end\n"),
    ("blank lines first", "\n\n  // t\n\nlabel 1;\n\n\n"),
    ("block comment first", "/* t */ label 1;\n"),
    ("saved state", H + "saved state best = 0;\nsaved   state   names = [\"a\"];\nstate n = 0;\nlabel best + n + len(names);\n"),
    ("lists in expressions", H + "fn f(n: int) -> int { let xs = [0; n]; let ys = [n, n + 1,]; return len(xs) + len(ys); }\n"
                             "label f(2);\n"),
    # What models write that applang took in on 2026-10-07.
    ("ternaries", H + "state n = 0;\nlabel n > 0 ? \"a\" : n < 0 ? \"b\" : \"c\";\nlabel (n == 0?1:2) + (true ? n : -n);\n"
                  "button n>0?\"x\":\"y\" { n = n > 5 ? 0 : (n < 0 ? 1 : n + 1); }\n"),
    ("while, break and continue", H + "state n = 0;\nbutton \"b\" { while n < 9 { n += 1; if n == 3 { continue; }"
                                  " if n > 6 { break ; } }\n  while(n>0){n-=1;}\n  repeat 3 { break; }\n"
                                  "  for i in 0..3 { continue/**/; } }\nlabel n;\n"),
    ("functions in any order, lists passed", H + "fn a(xs: [int]) -> [int] { return b(xs); }\n"
                                             "fn b(ys:[ int ]) -> [int] { return ys; }\n"
                                             "fn c(s: [string], t: [bool]) -> int { return len(s) + len(t); }\n"
                                             "label a([1, 2])[0] + c([\"x\"], [true]);\n"),
    ("row and col as names", H + "state row = 0;\nstate col = 1;\nfn at(row: int, col: int) -> int { return row * 3 + col; }\n"
                             "row { label at(row, col); }\ncol { label row; }\n"
                             "button \"b\" { let r = row; row = col; col = r; for row in 0..2 { col += row; } }\n"),
    ("indexing anything", H + "state s = \"abc\";\nlabel [1, 2, 3][1] + \"xyz\"[0] + s[2] + (s)[0] + [[\"a\"][0]][0];\n"),
    ("operator assignments", H + "state n = 10;\nstate xs = [1, 2];\nbutton \"b\" { n *= 2; n /= 3; n %= 4; xs[0] *= 5;"
                             " xs[1] /=2; xs [0]%=3; }\nlabel n;\n"),
    ("names near the new keywords", H + "state whilst = 1;\nstate breaks = 2;\nstate continu = 3;\nstate continues = 4;\n"
                                    "state wh = 5;\nstate b = 6;\nstate co = 7;\nstate rows = 8;\nstate w = 9;\n"
                                    "label whilst + breaks + continu + continues + wh + b + co + rows + w;\n"),
]

# Programs applang refuses before it checks names or types, and the code it gives.
S = H + "state n = 0;\n"
REJECT = [
    ("keyword as a let", S + "button \"b\" { let while = 1; }\n", 101),
    ("keyword as a loop variable", S + "for break in 0..3 { label 1; }\n", 101),
    ("keyword as a state", H + "state continue = 0;\nlabel 1;\n", 101),
    ("keyword as a function", H + "fn input() { }\nlabel 1;\n", 101),
    ("keyword as a target", S + "button \"b\" { label = 1; }\n", 101),
    ("keyword glued to a name", S + "labeln;\n", 101),
    ("keyword glued to a number", H + "fn f() -> int { return1; }\nlabel f();\n", 101),
    ("missing semicolon", S + "label n\nlabel 2;\n", 101),
    ("unterminated string", S + "label \"abc;\n", 4),
    ("bad escape", S + "label \"a\\tb\";\n", 5),
    ("unterminated block comment", S + "/* abc\nlabel 1;\n", 2),
    ("unterminated nested comment", S + "/* a /* b */\nlabel 1;\n", 2),
    ("// opens a nested comment inside one", S + "/* //* */\nlabel 1;\n", 2),
    ("unexpected character", S + "label n @ 2;\n", 1),
    ("a float", S + "label 1.5;\n", 1),
    ("malformed int", S + "label 12ab;\n", 3),
    ("int out of range", S + "label 99999999999999999999;\n", 3),
    ("int past i64::MAX", S + "label 9223372036854775808;\n", 3),
    ("2^63 in an expression", S + "label -9223372036854775808;\n", 3),
    ("2^63 unnegated in a literal", H + "state m = 9_223_372_036_854_775_808;\nlabel m;\n", 3),
    ("past 2^63 in a literal", H + "state m = -9223372036854775809;\nlabel m;\n", 3),
    ("fill past 4096 by one", H + "state xs = [0; 0_4_097];\nlabel 1;\n", 216),
    ("equality as a statement", S + "button \"b\" { n == 1; }\n", 101),
    ("expression as a statement", S + "button \"b\" { n + 1; }\n", 101),
    ("number as a statement", S + "button \"b\" { 5; }\n", 101),
    ("trailing comma in a call", S + "label max(1, 2,);\n", 101),
    ("trailing comma in parameters", H + "fn f(a: int,) { }\nlabel 1;\n", 101),
    ("state after a widget", S + "label 1;\nstate m = 0;\n", 101),
    ("empty state list", H + "state xs = [];\nlabel 1;\n", 101),
    ("mixed state list", H + "state xs = [1, true];\nlabel 1;\n", 303),
    ("fill past 4096", H + "state xs = [0; 5000];\nlabel 1;\n", 216),
    ("state from an expression", H + "state n = 1 + 2;\nlabel n;\n", 101),
    ("state from a name", H + "state m = 1;\nstate n = m;\nlabel n;\n", 101),
    ("negative bool", H + "state b = -true;\nlabel 1;\n", 101),
    ("=+", S + "button \"b\" { n =+ 1; }\n", 101),
    ("- =", S + "button \"b\" { n - = 1; }\n", 101),
    ("?: without :", S + "label n > 0 ? 1;\n", 101),
    ("?: missing a side", S + "label n > 0 ? : 2;\n", 101),
    ("while without braces", S + "button \"b\" { while n < 3 n += 1; }\n", 101),
    ("break without ;", S + "button \"b\" { repeat 2 { break } }\n", 101),
    ("break with a value", S + "button \"b\" { repeat 2 { break 1; } }\n", 101),
    ("while as a widget", S + "while n < 3 { label n; }\n", 101),
    ("row without braces", S + "row label 1;\n", 101),
    ("**=", S + "button \"b\" { n **= 2; }\n", 101),
    ("list type with a count", H + "fn f(a: [int; 2]) { }\nlabel 1;\n", 101),
    ("list of lists type", H + "fn f(a: [[int]]) { }\nlabel 1;\n", 101),
    ("index of a canvas's call", S + "fn scene() { }\ncanvas 10, 10, scene()[0];\n", 101),
    ("else without braces", S + "if n > 0 { label 1; } else label 2;\n", 101),
    ("elseif", S + "if n > 0 { label 1; } elseif n < 0 { label 2; }\n", 101),
    ("if without braces", S + "if n > 0 label 1;\n", 101),
    ("assignment as a condition", S + "if n = 1 { label 1; }\n", 101),
    ("single &", S + "if n > 0 & n < 3 { label 1; }\n", 1),
    ("++", S + "label n++;\n", 101),
    ("arrow in an expression", S + "label n->n;\n", 101),
    ("three dots", S + "for i in 0...3 { label i; }\n", 1),
    ("for with =", S + "for i = 0..3 { label i; }\n", 101),
    ("repeat without a count", S + "button \"b\" { repeat { n += 1; } }\n", 101),
    ("every without an interval", S + "every { n += 1; }\nlabel n;\n", 101),
    ("saved without state", H + "saved n = 0;\nlabel n;\n", 101),
    ("state without ;", H + "state n = 0\nlabel n;\n", 101),
    ("input of a string", S + "input \"x\";\n", 101),
    ("grid without a comma", H + "state cells = [0; 9];\ngrid 3 cells;\n", 101),
    ("on key without quotes", S + "on key left { n -= 1; }\nlabel n;\n", 101),
    ("on without key", S + "on \"left\" { n -= 1; }\nlabel n;\n", 101),
    ("canvas of a name", S + "fn scene() { }\ncanvas 10, 10, scene;\n", 101),
    ("canvas of a sum", S + "fn scene() -> int { return 1; }\ncanvas 10, 10, scene() + 1;\n", 101),
    ("canvas of a negation", S + "fn scene() -> int { return 1; }\ncanvas 10, 10, -scene();\n", 101),
    ("// after /", S + "label 6 //* c */ 2;\nlabel 1;\n", 101),
    ("/* right after /", S + "label 6 /*/ 2;\nlabel 1;\n", 2),
    ("stray */", S + "label 1 */ 2;\n", 101),
    ("list type", H + "fn f(a: list) { }\nlabel 1;\n", 101),
    ("void result", H + "fn f() -> void { }\nlabel 1;\n", 101),
    ("result without ->", H + "fn f() int { return 1; }\nlabel f();\n", 101),
    ("let without a value", S + "button \"b\" { let x; }\n", 101),
    ("call without ;", H + "fn f() { }\nbutton \"b\" { f() }\n", 101),
    ("two strings", S + "label \"a\" \"b\";\n", 101),
    ("unclosed brace", S + "row { label 1;\n", 101),
    ("extra brace", S + "label 1; }\n", 101),
    ("non-ASCII name", H + "state caf\u00e9 = 1;\nlabel 1;\n", 1),
    ("parameter without a type", H + "fn f(a) { }\nlabel 1;\n", 101),
    ("empty parentheses", S + "label ();\n", 101),
    ("+ as a sign", S + "label +1;\n", 101),
    ("widget inside a handler", S + "button \"b\" { label 1; }\n", 101),
    ("statement as a widget", S + "n = 1;\n", 101),
    ("fn inside a row", S + "row { fn f() { } }\n", 101),
]

# Whole replies, for Studio's framing: (name, reply, whether the grammar takes it).
FRAMES = [
    ("not honest", "```app\nlabel 1;\n```", False),
    ("fence ends the last line", "```app\n// t\nlabel 1;```", False),
    ("text after the fence", "```app\n// t\nlabel 1;\n```\nEnjoy.", False),
    ("text before the fence", "Here:\n```app\n// t\nlabel 1;\n```", False),
    ("fence inside a block comment", "```app\n// t\n/*\n```\n*/\nlabel 1;\n```", False),
    ("another language", "```rust\n// t\nlabel 1;\n```", False),
    ("an empty block", "```app\n```", False),
    ("unclosed", "```app\n// t\nlabel 1;\n", False),
    ("the shape of every SFT reply", "```app\n// Counter: + adds one.\n// icon: dot 12 12\nstate n = 0;\nlabel n;\n```", True),
]

STUBS = r"""// Link stubs for test-gbnf-validator outside CMake (written by train/test_grammar.py --build):
// the validator inits its grammar with no vocab, so no llama_vocab method here is ever called.
#include "llama-impl.h"
#include "llama-vocab.h"
#include <cstdarg>
#include <cstdio>
#include <cstdlib>
void llama_log_internal(ggml_log_level, const char * format, ...) {
    va_list args; va_start(args, format); vfprintf(stderr, format, args); va_end(args);
}
void ggml_abort(const char * file, int line, const char * fmt, ...) {
    fprintf(stderr, "%s:%d: ", file, line);
    va_list args; va_start(args, fmt); vfprintf(stderr, fmt, args); va_end(args);
    fprintf(stderr, "\n"); abort();
}
[[noreturn]] static void no_vocab() { fprintf(stderr, "llama_vocab stub called\n"); abort(); }
int32_t llama_vocab::tokenize(const char *, int32_t, llama_token *, int32_t, bool, bool) const { no_vocab(); }
const std::string & llama_vocab::token_to_piece(llama_token) const { no_vocab(); }
bool llama_vocab::is_eog(llama_token) const { no_vocab(); }
"""


def build(out, llama):
    """Compiles test-gbnf-validator into out with MSVC, at below-normal priority."""
    bats = sorted(glob.glob(r"C:\Program Files*\Microsoft Visual Studio\*\*\VC\Auxiliary\Build\vcvars64.bat"))
    if not bats:
        sys.exit("error: no vcvars64.bat (Visual Studio C++ tools) found")
    os.makedirs(os.path.join(out, "obj"), exist_ok=True)
    with open(os.path.join(out, "stubs.cpp"), "w", encoding="utf-8") as f:
        f.write(STUBS)
    srcs = " ".join(os.path.join(llama, p) for p in (r"tests\test-gbnf-validator.cpp", r"src\llama-grammar.cpp",
                                                     r"src\unicode.cpp", r"src\unicode-data.cpp"))
    inc = " ".join("/I " + os.path.join(llama, p) for p in ("include", r"ggml\include", "src"))
    bat = os.path.join(out, "build.bat")
    with open(bat, "w", encoding="utf-8") as f:
        f.write('@call "%s" >nul\n@cd /d "%s"\n' % (bats[-1], out))
        f.write("@cl /nologo /std:c++17 /EHsc /O2 /MD /utf-8 /DNDEBUG /D_CRT_SECURE_NO_WARNINGS %s "
                "/Foobj\\ /Fe:%s %s stubs.cpp\n" % (inc, EXE, srcs))
    r = subprocess.run(["cmd", "/c", bat], capture_output=True, text=True, creationflags=LOW)
    if r.returncode != 0:
        sys.exit("error: the validator did not build:\n" + r.stdout[-3000:] + r.stderr[-2000:])
    return os.path.join(out, EXE)


def find_validator(given):
    if given:
        return given
    if os.environ.get("GBNF_VALIDATOR"):
        return os.environ["GBNF_VALIDATOR"]
    for b in ("build", "build-cpu", "build-gbnf"):
        for d in (os.path.join(LLAMA, b, "bin", "Release"), os.path.join(LLAMA, b, "bin"), os.path.join(LLAMA, b)):
            if os.path.exists(os.path.join(d, EXE)):
                return os.path.join(d, EXE)
    sys.exit("error: no %s: pass --validator, set GBNF_VALIDATOR, or --build DIR" % EXE)


class Validator:
    def __init__(self, exe, grammar, scratch):
        self.exe, self.grammar, self.scratch = exe, grammar, scratch

    def __call__(self, i, text):
        """(accepted, why) for text, as llama.cpp's grammar engine sees it."""
        path = os.path.join(self.scratch, "case-%d.txt" % i)
        with open(path, "wb") as f:  # overwritten in place, run to run
            f.write(text.encode("utf-8"))
        r = subprocess.run([self.exe, self.grammar, path], capture_output=True, creationflags=LOW)
        out = r.stdout.decode("utf-8", "replace")
        if "Input string is valid" in out:
            return True, ""
        if "Input string is invalid" in out:
            return False, next((ln for ln in out.splitlines() if ln.startswith("Error:")), "?")
        raise RuntimeError("the validator failed: %s %s" % (out[-500:], r.stderr.decode("utf-8", "replace")[-1500:]))


def iq_grade(iq, scratch, i, src):
    """(stage, code) of applang's own compiler on src, by `iq grade`."""
    path = os.path.join(scratch, "prog-%d.app" % i)
    with open(path, "wb") as f:
        f.write(src.encode("utf-8"))
    r = subprocess.run([iq, "grade", "counter", path], capture_output=True, creationflags=LOW)
    g = json.loads(r.stdout.decode("utf-8").strip().splitlines()[-1])
    return g.get("stage", "pass" if g.get("pass") else "?"), g.get("code")


def main():
    h = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    h.add_argument("--grammar", default=os.path.join(HERE, "applang.gbnf"))
    h.add_argument("--suite", default=os.path.join(REPO, "evals", "suites", "iq.jsonl"))
    h.add_argument("--validator", help="test-gbnf-validator's path (else GBNF_VALIDATOR, else llama.cpp's builds)")
    h.add_argument("--build", metavar="DIR", help="first compile the validator into DIR (Windows, MSVC)")
    h.add_argument("--iq", default=os.path.join(REPO, "target", "release", "iq" + (".exe" if os.name == "nt" else "")),
                   help="applang's compiler, to check the hand-written cases against (skipped if absent)")
    h.add_argument("--sft", help="an sft.jsonl: every assistant reply in it must pass too")
    h.add_argument("--jobs", type=int, default=8)
    a = h.parse_args()

    exe = build(a.build, LLAMA) if a.build else find_validator(a.validator)
    scratch = os.path.join(tempfile.gettempdir(), "applang-gbnf")
    os.makedirs(scratch, exist_ok=True)
    check = Validator(exe, a.grammar, scratch)

    # (kind, name, text, want): every reply the grammar must take or refuse.
    cases = []
    with open(a.suite, encoding="utf-8") as f:
        for line in f:
            if line.strip():
                t = json.loads(line)
                cases.append(("suite", t["id"], reply(t["ref"]), True))
    cases += [("accept", n, reply(src), True) for n, src in ACCEPT]
    cases += [("reject", n, reply(src), False) for n, src, _ in REJECT]
    cases += [("frame", n, text, ok) for n, text, ok in FRAMES]
    if a.sft:
        with open(a.sft, encoding="utf-8") as f:
            for k, line in enumerate(f):
                if line.strip():
                    m = [x for x in json.loads(line)["messages"] if x["role"] == "assistant"]
                    cases.append(("sft", "line %d" % (k + 1), m[-1]["content"], True))

    with ThreadPoolExecutor(a.jobs) as pool:
        got = list(pool.map(lambda ic: check(ic[0], ic[1][2]), enumerate(cases)))

    bad = 0
    tally = {}
    for (kind, name, text, want), (ok, why) in zip(cases, got):
        t = tally.setdefault(kind, [0, 0])
        t[0 if ok == want else 1] += 1
        if ok != want:
            bad += 1
            print("FAIL %s %s: the grammar %s it%s" % (kind, name, "takes" if ok else "refuses", " (%s)" % why if why else ""))
        if ok and block_end(text) != len(text):  # an accepted reply Studio would not read whole
            bad += 1
            print("FAIL %s %s: the grammar takes it, but Studio would not read it whole" % (kind, name))

    for kind, (good, wrong) in tally.items():
        verb = {"suite": "references accepted", "accept": "edge cases accepted", "reject": "syntax errors refused",
                "frame": "framing cases right", "sft": "SFT replies accepted"}[kind]
        print("%-7s %d/%d %s" % (kind, good, good + wrong, verb))

    if os.path.exists(a.iq):
        progs = [("accept", n, src, None) for n, src in ACCEPT] + [("reject", n, src, c) for n, src, c in REJECT]
        with ThreadPoolExecutor(a.jobs) as pool:
            graded = list(pool.map(lambda ip: iq_grade(a.iq, scratch, ip[0], ip[1][2]), enumerate(progs)))
        agree = 0
        for (kind, name, _, code), (stage, got_code) in zip(progs, graded):
            ok = stage != "compile" if kind == "accept" else (stage == "compile" and got_code == code)
            agree += ok
            if not ok:
                bad += 1
                print("FAIL iq %s %s: applang says stage %s, code %s (want %s)" % (
                    kind, name, stage, got_code, "a compile" if code is None else "E%04d" % code))
        print("iq      %d/%d hand-written cases as applang's own compiler judges them" % (agree, len(progs)))
    else:
        print("iq      skipped (no %s)" % a.iq)

    print("grammar %s: %d lines, %d bytes; %s" % (os.path.basename(a.grammar), sum(1 for _ in open(a.grammar, encoding="utf-8")),
                                                  os.path.getsize(a.grammar), "all good" if bad == 0 else "%d failures" % bad))
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
