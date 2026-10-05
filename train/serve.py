#!/usr/bin/env python3
"""Serve a GGUF with llama.cpp's llama-server on 127.0.0.1 (OpenAI-compatible, /v1).

    python train/serve.py --run NAME [--quant q8_0]    # a run's export
    python train/serve.py --base-only [--base ID]      # a base model's export
    python train/serve.py --gguf FILE [--tokenizer DIR]
    python train/serve.py --stop | --status            # [--port 8081]

Starts the server detached (every layer on the GPU, --ctx tokens a slot),
waits until /health is ok, then checks that the server renders and tokenizes
a chat exactly as training did (the template the GGUF carries, run by
llama.cpp, against the tokenizer's own), prints the URL and exits. The PID
and start time go to <root>/serve/llama-server-<port>.json; --stop stops that
process only, and only while it still is the one this script started.
"""
import argparse
import json
import os
import re
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request

import common

PROBE = [{"role": "system", "content": "You write apps in applang."},
         {"role": "user", "content": "Make a counter: a label and a + button."}]


def parse_args():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--run", help="serve this run's GGUF")
    p.add_argument("--quant", default="q8_0")
    p.add_argument("--base-only", action="store_true", help="serve the base model's GGUF (export.py --base-only)")
    p.add_argument("--base", default=common.DEFAULT_BASE)
    p.add_argument("--gguf", help="serve this GGUF file")
    p.add_argument("--tokenizer", help="with --gguf: the tokenizer folder for the template check")
    p.add_argument("--port", type=int, default=8081)
    p.add_argument("--ctx", type=int, default=8192, help="context tokens for each slot")
    p.add_argument("--parallel", type=int, default=1, help="slots (concurrent requests)")
    p.add_argument("--timeout", type=float, default=180, help="seconds to wait for /health")
    p.add_argument("--stop", action="store_true")
    p.add_argument("--status", action="store_true")
    p.add_argument("--root")
    p.add_argument("--llama", default=os.environ.get("LLAMA_CPP_DIR", common.DEFAULT_LLAMA))
    return p.parse_args()


def http(url, body=None, timeout=10):
    req = urllib.request.Request(url, data=None if body is None else json.dumps(body).encode("utf-8"),
                                 headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read().decode("utf-8") or "null")


def healthy(url):
    try:
        return (http(url + "/health", timeout=2) or {}).get("status") == "ok"
    except (urllib.error.URLError, OSError, ValueError):
        return False


def port_busy(port):
    with socket.socket() as s:
        s.settimeout(0.5)
        return s.connect_ex(("127.0.0.1", port)) == 0


def ours(info):
    """The process the PID file names, if it still is the one started (same PID, start time, name)."""
    import psutil
    try:
        p = psutil.Process(info["pid"])
        if abs(p.create_time() - info["create_time"]) < 1 and p.name().lower().startswith("llama-server"):
            return p
    except (psutil.Error, KeyError, TypeError):
        pass
    return None


def stop(pidfile, log):
    import psutil
    info = common.read_json(pidfile)
    if not info or info.get("status") not in ("starting", "running"):
        log("nothing to stop on this port")
        return
    p = ours(info)
    if p:
        p.terminate()
        try:
            p.wait(15)
        except psutil.TimeoutExpired:
            p.kill()
            p.wait(15)
        log("stopped llama-server pid %d (%s)" % (info["pid"], os.path.basename(info["gguf"])))
    else:
        log("pid %d is gone or no longer ours; nothing stopped" % info["pid"])
    info.update(status="stopped", stopped=common.now())
    common.write_json(pidfile, info)


def offload(log_path):
    """How many layers llama-server put on the GPU, from its log."""
    with open(log_path, encoding="utf-8", errors="replace") as f:
        text = f.read()
    found = re.search(r"offloaded (\d+)/(\d+) layers to GPU", text)
    if found:
        return {"layers": int(found.group(1)), "of": int(found.group(2))}
    return {"layers": 0, "why": "no usable GPU (a CPU-only llama.cpp build?)" if "no usable GPU" in text else None}


def template_check(url, tok_dir):
    """Does the server feed the model what training fed it? Rendered text and token ids, both."""
    from transformers import AutoTokenizer
    tok = AutoTokenizer.from_pretrained(tok_dir)
    want = tok.apply_chat_template(PROBE, tokenize=False, add_generation_prompt=True)
    want_ids = tok.apply_chat_template(PROBE, tokenize=True, add_generation_prompt=True)
    got = http(url + "/apply-template", {"messages": PROBE})["prompt"]
    got_ids = http(url + "/tokenize", {"content": got, "add_special": True, "parse_special": True})["tokens"]
    return {"text": got == want, "tokens": got_ids == want_ids, "server": got, "train": want}


def pick(a, root):
    """(gguf, alias, tokenizer folder) for what to serve."""
    if a.run:
        rdir = common.run_dir(root, a.run)
        m = common.read_json(os.path.join(rdir, "manifest.json")) or {}
        e = (m.get("exports") or {}).get(a.quant)
        if not e or not os.path.exists(e["file"]):
            sys.exit("error: run %s has no %s export; run export.py --run %s first" % (a.run, a.quant, a.run))
        return e["file"], a.run, os.path.join(rdir, "model" if m["hyper"]["mode"] == "full" else "adapter")
    if a.base_only:
        base = common.resolve_base(a.base, root)
        name = common.slug(a.base) + ("-" + base["revision"][:8] if base["revision"] else "")
        gguf = os.path.join(root, "base", "%s-%s.gguf" % (name, a.quant))
        if not os.path.exists(gguf):
            sys.exit("error: no %s; run export.py --base-only --base %s first" % (gguf, a.base))
        return gguf, name, base["path"]
    if a.gguf:
        return os.path.abspath(a.gguf), os.path.splitext(os.path.basename(a.gguf))[0], a.tokenizer
    sys.exit("error: give --run, --base-only or --gguf (or --stop, --status)")


def start(a, root, pidfile, log):
    import psutil
    gguf, alias, tok_dir = pick(a, root)
    url = "http://127.0.0.1:%d" % a.port
    info = common.read_json(pidfile)
    if info and info.get("status") in ("starting", "running") and ours(info):
        if os.path.normcase(info["gguf"]) == os.path.normcase(gguf) and healthy(url):
            log("already serving %s at %s/v1" % (alias, url))
            print(url + "/v1")
            return
        sys.exit("error: already serving %s on port %d; --stop first" % (info["gguf"], a.port))
    if port_busy(a.port):
        sys.exit("error: something else listens on 127.0.0.1:%d; it is not ours, so it is left alone" % a.port)
    server = common.llama_bin(a.llama, "llama-server")
    if not server:
        sys.exit("error: no built llama-server under " + a.llama)
    cmd = [server, "-m", gguf, "--host", "127.0.0.1", "--port", str(a.port), "-c", str(a.ctx * a.parallel),
           "-np", str(a.parallel), "-ngl", "999", "--jinja", "--alias", alias, "--no-webui"]
    out = os.path.join(root, "logs", "llama-server-%d.log" % a.port)
    os.makedirs(os.path.dirname(out), exist_ok=True)
    flags = 0x00000008 | 0x00000200 if os.name == "nt" else 0  # DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
    with open(out, "wb") as f:
        proc = subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=f, stderr=subprocess.STDOUT,
                                creationflags=flags, close_fds=True, start_new_session=os.name != "nt")
    info = {"pid": proc.pid, "create_time": psutil.Process(proc.pid).create_time(), "port": a.port,
            "url": url + "/v1", "gguf": gguf, "alias": alias, "cmd": cmd, "log": out,
            "started": common.now(), "status": "starting"}
    common.write_json(pidfile, info)
    log("started llama-server pid %d: %s" % (proc.pid, os.path.basename(gguf)))
    t = time.time()
    while not healthy(url):
        if proc.poll() is not None or time.time() - t > a.timeout:
            stop(pidfile, log)
            with open(out, encoding="utf-8", errors="replace") as f:
                tail = "".join(f.readlines()[-20:])
            sys.exit("error: llama-server %s\n%s" % ("exited" if proc.poll() is not None else "timed out", tail))
        time.sleep(0.5)
    gpu = offload(out)
    log("healthy in %.1fs; %s" % (time.time() - t, "%d/%d layers on the GPU" % (gpu["layers"], gpu["of"])
                                  if "of" in gpu else "on the CPU"))
    if gpu["layers"] == 0 or gpu["layers"] < gpu.get("of", 0):
        log("WARNING: not every layer is on the GPU: %s. A CUDA build of llama.cpp in %s is used first." % (
            gpu.get("why") or "see " + out, os.path.join(a.llama, "build-cuda")))
    info.update(status="running", ready=common.now(), gpu=gpu)
    if tok_dir:
        check = template_check(url, tok_dir)
        info["template"] = {k: check[k] for k in ("text", "tokens")}
        if not (check["text"] and check["tokens"]):
            common.write_json(pidfile, info)
            stop(pidfile, log)
            sys.exit("error: the server does not render the chat as training did\n server: %r\n train:  %r" % (
                check["server"], check["train"]))
        log("template check: the server renders and tokenizes the chat exactly as training did")
    else:
        log("WARNING: no tokenizer given, so no template check")
    common.write_json(pidfile, info)
    print(url + "/v1")


def main():
    common.utf8_stdio()
    a = parse_args()
    root = common.data_root(a.root)
    pidfile = os.path.join(root, "serve", "llama-server-%d.json" % a.port)
    os.makedirs(os.path.dirname(pidfile), exist_ok=True)
    log = common.Log(os.path.join(root, "logs", "serve.log"))
    if a.stop:
        stop(pidfile, log)
    elif a.status:
        info = common.read_json(pidfile) or {}
        alive = bool(info) and ours(info) is not None
        print(json.dumps(dict(info, alive=alive, healthy=alive and healthy("http://127.0.0.1:%d" % a.port)),
                         indent=2))
    else:
        start(a, root, pidfile, log)


if __name__ == "__main__":
    main()
