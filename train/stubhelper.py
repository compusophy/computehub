#!/usr/bin/env python
"""A stand-in for every model a team night serves, for testing with no model (a test tool:
nothing it says is trained on or scored): chat completions on 127.0.0.1, streamed as
llama-server streams them, each reply with its usage chunk, or whole when the request says
"stream": false; GET /health says ok.

  python train/stubhelper.py [--port 8090] [--overflow WORD] [--never WORD] [--delay 0.2]

A fix's program is read back from its numbered lines (coder's prompt::fix), and the reply is, by
the request's seed mod 3 (unseeded: by the count of requests):
  0  a fixing edit: one SEARCH/REPLACE block putting a counter as long as the program in its
     place, which runs clean;
  1  a no-op edit: a line of the program for itself;
  2  the whole program, as it is, in an app block.
A request with no program listed (ask.py's prompts: a task to write, GLM's notes to finish) gets,
by the same seed: 0 a counter in an app block (it runs clean), 1 a program that does not compile,
2 reasoning with no program; one asking for a check (checkdata.py's), a check that only asks the
program to run. A judge's question (judge.py's "Does it do everything asked?") gets one token,
yes or no, with top logprobs: yes at -len(program)/1000, so a judge of it is the length baseline.
A request whose message holds --overflow gets llama-server's HTTP 400 (the request exceeds the
available context size); one holding --never only no-op edits. --delay waits before each reply,
so that a run can be stopped midway.
"""
import argparse
import itertools
import json
import math
import os
import re
import sys
import time

# train/select.py would stand in for the standard library's select, which http.server serves
# with: this folder is off the path while it is imported.
HERE, PATH = os.path.dirname(os.path.abspath(__file__)), sys.path[:]
sys.path[:] = [p for p in sys.path if os.path.abspath(p or os.curdir) != HERE]
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer  # noqa: E402
sys.path[:] = PATH

import common  # noqa: E402

LISTED = re.compile(r"^ *\d+\| ?(.*)$")
COUNTER = ["// Counter: + adds one.", "// icon: ring 12 12 9", "state n = 0;", 'label "Counter";',
           "label n;", 'button "+" { n += 1; }']
TOO_LONG = {"error": {"code": 400, "message": "the request exceeds the available context size, try increasing it",
                      "type": "exceed_context_size_error", "n_prompt_tokens": 20480, "n_ctx": 16384}}
BROKEN = ["// Broken: an unclosed button.", "state n = 0;", 'button "+" { n += 1;']
CHECK = '```check\n# runs clean only\nexpect not says "qqzzxq never shown"\n```'
JUDGED = re.compile(r"```app\n(.*)\n```\n\nDoes it do everything asked\?", re.S)
COUNT = itertools.count()


def program(msg):
    """The program a fix lists, numbered: its lines, numbers off."""
    out = []
    for line in msg.split("The program, numbered:\n", 1)[-1].split("\n"):
        m = LISTED.match(line)
        if not m:
            break
        out.append(m.group(1))
    return out


def reply(msg, seed, never, system=""):
    src = program(msg)
    if not src:  # a prompt to write a program (or a check), none listed
        if "You write the check" in system:
            return CHECK
        return ["```app\n%s\n```" % "\n".join(COUNTER), "```app\n%s\n```" % "\n".join(BROKEN),
                "First the state, then the rows of buttons; the board is drawn each tick..."][seed % 3]
    kind = 1 if never and never in msg else seed % 3
    block = "<<<<<<< SEARCH\n%s\n=======\n%s\n>>>>>>> REPLACE\n"
    if kind == 0:
        pad = sum(1 for l in src if l.strip()) - len(COUNTER)
        return block % ("\n".join(src), "\n".join(COUNTER + ['label "%d";' % i for i in range(max(pad, 0))]))
    once = next((l for l in src if l.strip() and src.count(l) == 1), None)
    if kind == 1 and once is not None:
        return block % (once, once)
    return "```app\n%s\n```" % "\n".join(src)


def judged(msg):
    """A judge's one token, as llama-server answers with logprobs: yes at -len(program)/1000."""
    yes = -len(JUDGED.search(msg).group(1)) / 1000.0
    no = math.log(max(1e-9, 1.0 - math.exp(yes)))
    word = "Yes" if yes >= no else "No"
    top = [{"token": "Yes", "logprob": yes}, {"token": "No", "logprob": no}, {"token": "maybe", "logprob": -9.0}]
    return word, {"content": [{"token": word, "logprob": max(yes, no), "top_logprobs": top}]}


def sse(obj):
    return ("data: " + json.dumps(obj, separators=(",", ":")) + "\n\n").encode("utf-8")


class Stub(BaseHTTPRequestHandler):
    def whole(self, status, obj):
        data = json.dumps(obj, separators=(",", ":")).encode("utf-8")
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        if self.path == "/health":
            return self.whole(200, {"status": "ok"})
        self.whole(404, {"error": {"code": 404, "message": "not found"}})

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["content-length"])))
        msg = body["messages"][-1]["content"]
        system = next((m["content"] for m in body["messages"] if m.get("role") == "system"), "")
        time.sleep(A.delay)
        if A.overflow and A.overflow in msg:
            return self.whole(400, TOO_LONG)
        usage = {"prompt_tokens": len(msg) // 4}
        if not body.get("stream", False):
            if JUDGED.search(msg):
                text, lp = judged(msg)
            else:
                text, lp = reply(msg, body.get("seed", next(COUNT)), A.never, system), None
            usage["completion_tokens"] = max(1, len(text) // 4)
            usage["total_tokens"] = usage["prompt_tokens"] + usage["completion_tokens"]
            choice = {"index": 0, "message": {"role": "assistant", "content": text}, "finish_reason": "stop"}
            if lp is not None and body.get("logprobs"):
                choice["logprobs"] = lp
            return self.whole(200, {"model": "stub", "choices": [choice], "usage": usage})
        text = reply(msg, body.get("seed", next(COUNT)), A.never, system)
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.end_headers()
        half = len(text) // 2
        for part in (text[:half], text[half:]):
            self.wfile.write(sse({"choices": [{"index": 0, "delta": {"content": part}, "finish_reason": None}]}))
        self.wfile.write(sse({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}))
        usage["completion_tokens"] = len(text) // 4
        usage["total_tokens"] = usage["prompt_tokens"] + usage["completion_tokens"]
        self.wfile.write(sse({"choices": [], "usage": usage}))
        self.wfile.write(b"data: [DONE]\n\n")

    def log_message(self, *args):
        pass


if __name__ == "__main__":
    common.utf8_stdio()
    h = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    h.add_argument("--port", type=int, default=8090)
    h.add_argument("--overflow", help="a request whose message holds this gets HTTP 400 (context)")
    h.add_argument("--never", help="a request whose message holds this gets only no-op edits")
    h.add_argument("--delay", type=float, default=0.2, help="seconds before each reply")
    A = h.parse_args()
    print("stub helper on 127.0.0.1:%d" % A.port, flush=True)
    ThreadingHTTPServer(("127.0.0.1", A.port), Stub).serve_forever()
