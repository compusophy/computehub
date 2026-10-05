#!/usr/bin/env python3
"""Export a trained run to GGUF for llama-server: merge the LoRA adapter into
its base, save that as a Hugging Face model, convert it with llama.cpp's
convert_hf_to_gguf.py, and quantize it.

    python train/export.py --run NAME [--quant q8_0|q4_k_m|f16|bf16]
    python train/export.py --base-only [--base ID] [--quant q8_0]   # an untouched base, for baselines

A run's GGUF is <root>/runs/<run>/model-<quant>.gguf, recorded in its
manifest (size, SHA-256, llama.cpp build). A base's is
<root>/base/<base>-<revision>-<quant>.gguf, with a .json beside it. The merged
model and any f16 intermediate live in <root>/scratch/, one per base,
overwritten in place by the next export.
"""
import argparse
import os
import re
import subprocess
import sys
import time

import common

QUANTS = ("q8_0", "q4_k_m", "f16", "bf16")
DIRECT = ("q8_0", "f16", "bf16")  # convert_hf_to_gguf.py writes these itself; the rest need llama-quantize


def parse_args():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--run", help="a done run under <root>/runs")
    p.add_argument("--base-only", action="store_true", help="convert an untouched base model (--base)")
    p.add_argument("--base", default=common.DEFAULT_BASE, help="with --base-only: hub id or folder")
    p.add_argument("--quant", default="q8_0", choices=QUANTS)
    p.add_argument("--root")
    p.add_argument("--llama", default=os.environ.get("LLAMA_CPP_DIR", common.DEFAULT_LLAMA),
                   help="llama.cpp checkout with a build (default $LLAMA_CPP_DIR, else %s)" % common.DEFAULT_LLAMA)
    p.add_argument("--force", action="store_true", help="export again even if the GGUF exists")
    a = p.parse_args()
    if bool(a.run) == a.base_only:
        p.error("give --run NAME or --base-only")
    return a


def llama_tools(d):
    conv = os.path.join(d, "convert_hf_to_gguf.py")
    quant, server = common.llama_bin(d, "llama-quantize"), common.llama_bin(d, "llama-server")
    if not os.path.exists(conv) or not quant:
        sys.exit("error: no convert_hf_to_gguf.py and built llama-quantize under " + d)
    build = None
    if server:
        r = subprocess.run([server, "--version"], capture_output=True, text=True, timeout=60)
        found = re.search(r"version: (\S+ \(\w+\))", r.stdout + r.stderr)
        build = found.group(1) if found else None
    return {"convert": conv, "quantize": quant, "build": build, "convert_sha256": common.sha256_file(conv)}


def run(cmd, log_path, log):
    log("$ " + " ".join('"%s"' % c if " " in c else c for c in cmd))
    with open(log_path, "ab") as f:
        r = subprocess.run(cmd, stdout=f, stderr=subprocess.STDOUT)
    if r.returncode:
        with open(log_path, encoding="utf-8", errors="replace") as f:
            tail = f.readlines()[-25:]
        sys.exit("error: exit %d from %s\n%s" % (r.returncode, os.path.basename(cmd[1] if cmd[0] == sys.executable
                                                                                else cmd[0]), "".join(tail)))


def to_gguf(tools, src, out, quant, scratch, name, log_path, log):
    """src (a Hugging Face model folder) -> out, written as out.tmp then renamed."""
    tmp = out + ".tmp"
    if quant in DIRECT:
        run([sys.executable, tools["convert"], src, "--outfile", tmp, "--outtype", quant], log_path, log)
    else:
        f16 = os.path.join(scratch, name + "-f16.gguf")
        run([sys.executable, tools["convert"], src, "--outfile", f16, "--outtype", "f16"], log_path, log)
        run([tools["quantize"], f16, tmp, quant.upper()], log_path, log)
    common.replace(tmp, out)
    return {"file": out, "bytes": os.path.getsize(out), "sha256": common.sha256_file(out), "quant": quant,
            "llama_cpp": tools["build"], "converter_sha256": tools["convert_sha256"], "at": common.now()}


def merge(base, adapter, out, provenance, log):
    """The base with the adapter folded into its weights (bf16), as a Hugging Face model."""
    import torch
    from peft import PeftModel
    from transformers import AutoModelForCausalLM, AutoTokenizer
    t = time.time()
    model = AutoModelForCausalLM.from_pretrained(base["path"], dtype=torch.bfloat16)
    model = PeftModel.from_pretrained(model, adapter).merge_and_unload()
    os.makedirs(out, exist_ok=True)
    model.save_pretrained(out, safe_serialization=True, max_shard_size="100GB")
    AutoTokenizer.from_pretrained(adapter).save_pretrained(out)
    common.write_json(os.path.join(out, common.PROVENANCE), provenance)
    log("merged %s into %s in %.1fs" % (adapter, out, time.time() - t))


def main():
    common.utf8_stdio()
    a = parse_args()
    root = common.data_root(a.root)
    tools = llama_tools(a.llama)
    scratch = os.path.join(root, "scratch")
    os.makedirs(scratch, exist_ok=True)

    if a.base_only:
        base = common.resolve_base(a.base, root)
        name = common.slug(a.base) + ("-" + base["revision"][:8] if base["revision"] else "")
        os.makedirs(os.path.join(root, "base"), exist_ok=True)
        out = os.path.join(root, "base", "%s-%s.gguf" % (name, a.quant))
        log = common.Log(os.path.join(root, "logs", "export-base.log"))
        if os.path.exists(out) and os.path.exists(out + ".json") and not a.force:
            log("exists: %s (--force to redo)" % out)
            print(out)
            return
        log("== export base %s @ %s -> %s" % (base["id"], base["revision"], a.quant))
        info = to_gguf(tools, base["path"], out, a.quant, scratch, name, log.path, log)
        common.write_json(out + ".json", dict(info, base=base, repo=common.git_commit()))
        log("wrote %s (%.0f MB)" % (out, info["bytes"] / 2 ** 20))
        print(out)
        return

    rdir = common.run_dir(root, a.run)
    mpath = os.path.join(rdir, "manifest.json")
    m = common.read_json(mpath)
    if not m or m.get("status") != "done":
        sys.exit("error: run %s is not done training (%s)" % (a.run, mpath))
    log = common.Log(os.path.join(rdir, "export.log"))
    out = os.path.join(rdir, "model-%s.gguf" % a.quant)
    old = (m.get("exports") or {}).get(a.quant)
    if old and os.path.exists(out) and os.path.getsize(out) == old["bytes"] and not a.force:
        log("exists: %s (--force to redo)" % out)
        print(out)
        return
    log("== export %s -> %s" % (a.run, a.quant))
    base = common.resolve_base(m["base"]["id"], root)
    if base["revision"] != m["base"]["revision"]:
        sys.exit("error: the cache holds %s @ %s, but the run trained on @ %s" % (
            base["id"], base["revision"], m["base"]["revision"]))
    name = common.slug(base["id"])
    if m["hyper"]["mode"] == "full":
        src = os.path.join(rdir, "model")
    else:
        src = os.path.join(scratch, "merged-" + name)
        merge(base, os.path.join(rdir, "adapter"), src, {"run": a.run, "config": m["config"], "base": base}, log)
    info = to_gguf(tools, src, out, a.quant, scratch, name, log.path, log)
    m = common.read_json(mpath)  # re-read: write only our key
    m.setdefault("exports", {})[a.quant] = info
    common.write_json(mpath, m)
    log("wrote %s (%.0f MB)" % (out, info["bytes"] / 2 ** 20))
    print(out)


if __name__ == "__main__":
    main()
