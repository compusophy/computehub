"""What the train/ scripts share: the data root, run folders, atomic writes,
hashes, provenance, and the base model resolved from the local Hugging Face
cache (never downloaded).

As a script (night.sh uses both):
    python train/common.py root                                         # print the data root
    python train/common.py result --run NAME --status ok [--score FILE]  # a line in <root>/runs.jsonl
"""
import argparse
import datetime
import hashlib
import json
import os
import re
import subprocess
import sys
import time

# Offline before anything imports huggingface_hub or transformers: the bases
# come from the local cache, never the network.
os.environ.setdefault("HF_HUB_OFFLINE", "1")
os.environ.setdefault("TRANSFORMERS_OFFLINE", "1")
os.environ.setdefault("HF_HUB_DISABLE_TELEMETRY", "1")

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
DEFAULT_ROOT = r"C:\sept30\computehub-data"
DEFAULT_BASE = "Qwen/Qwen2.5-Coder-0.5B-Instruct"
DEFAULT_LLAMA = r"C:\llama-cpp"
PROVENANCE = "computehub-run.json"  # marks a fine-tune's weights: never a base


def utf8_stdio():
    """Windows consoles default to cp1252; tokens and logs are UTF-8."""
    for s in (sys.stdout, sys.stderr):
        try:
            s.reconfigure(encoding="utf-8", errors="replace", line_buffering=True)
        except (AttributeError, ValueError):
            pass


def inside(path, parent):
    path = os.path.normcase(os.path.abspath(path))
    parent = os.path.normcase(os.path.abspath(parent))
    return path == parent or path.startswith(parent.rstrip(os.sep) + os.sep)


def data_root(arg=None):
    """The data root: --root, else $COMPUTEHUB_DATA, else the default. Outside the repo."""
    root = os.path.abspath(arg or os.environ.get("COMPUTEHUB_DATA") or DEFAULT_ROOT)
    if inside(root, REPO):
        sys.exit("error: the data root must be outside the repository: " + root)
    os.makedirs(root, exist_ok=True)
    return root


def run_dir(root, run):
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,99}", run or ""):
        sys.exit("error: a run name is letters, digits, '.', '_' and '-': %r" % run)
    return os.path.join(root, "runs", run)


def now():
    return datetime.datetime.now().astimezone().isoformat(timespec="seconds")


def slug(model_id):
    """'Qwen/Qwen2.5-Coder-0.5B-Instruct' -> 'qwen2.5-coder-0.5b-instruct'."""
    return re.sub(r"[^a-z0-9._-]+", "-", model_id.split("/")[-1].lower()).strip("-")


def replace(src, dst):
    """os.replace, retried a while: on Windows a virus scanner or indexer may
    hold a fresh file open for a moment, and the night has no one to retry."""
    for i in range(20):
        try:
            os.replace(src, dst)
            return
        except PermissionError:
            if i == 19:
                raise
            time.sleep(0.5)


def write_atomic(path, data):
    """Write to path.tmp, flush it to the disk, then rename over path: a crash
    leaves the old file or the new one, never half of one."""
    if isinstance(data, str):
        data = data.encode("utf-8")
    tmp = path + ".tmp"
    with open(tmp, "wb") as f:
        f.write(data)
        f.flush()
        os.fsync(f.fileno())
    replace(tmp, path)


def write_json(path, obj):
    write_atomic(path, json.dumps(obj, indent=2) + "\n")


def read_json(path, default=None):
    if not os.path.exists(path):
        return default
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def append_line(path, line):
    with open(path, "a", encoding="utf-8") as f:
        f.write(line.rstrip("\n") + "\n")
        f.flush()
        os.fsync(f.fileno())


def fsync_file(path):
    with open(path, "r+b") as f:
        os.fsync(f.fileno())


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def sha256_text(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def prompt_hash(messages):
    """16 hex digits of a prompt's messages: an answer records it, so one made from another
    prompt (an older language card) is never kept as this one's."""
    return sha256_text(json.dumps(messages, sort_keys=True, ensure_ascii=False))[:16]


def git_commit():
    """The repo's HEAD and whether tracked files differ from it."""
    def git(*args):
        try:
            r = subprocess.run(["git", "-C", REPO] + list(args), capture_output=True, text=True, timeout=30)
            return r.stdout.strip() if r.returncode == 0 else None
        except OSError:
            return None
    head = git("rev-parse", "HEAD")
    status = git("status", "--porcelain", "--untracked-files=no")
    return {"commit": head, "dirty": bool(status) if status is not None else None}


class Log:
    """Lines to stdout and appended to a file, flushed each time (a hard
    freeze loses at most the line being written)."""

    def __init__(self, path):
        self.path = path
        os.makedirs(os.path.dirname(path), exist_ok=True)
        self.f = open(path, "a", encoding="utf-8")

    def __call__(self, *parts):
        line = "%s %s" % (datetime.datetime.now().strftime("%H:%M:%S"), " ".join(str(p) for p in parts))
        print(line, flush=True)
        self.f.write(line + "\n")
        self.f.flush()


def llama_bin(llama_dir, name):
    """A built llama.cpp tool (llama-server, llama-quantize), or None. A CUDA
    build in build-cuda/ is preferred to build/."""
    exe = name + (".exe" if os.name == "nt" else "")
    for d in (os.path.join(llama_dir, b, "bin", r) for b in ("build-cuda", "build") for r in ("Release", "")):
        if os.path.exists(os.path.join(d, exe)):
            return os.path.join(d, exe)
    return None


def resolve_base(base, root=None):
    """A base model: a hub id found in the local HF cache, or a local folder.
    Returns {"id", "revision", "path"}. A fine-tune is refused: every round
    starts from the untouched base, never from last night's model."""
    if os.path.isdir(base):
        path, revision = os.path.abspath(base), None
    else:
        from huggingface_hub import snapshot_download
        try:
            path = snapshot_download(base, local_files_only=True)
        except Exception as e:  # huggingface_hub raises several kinds, by version
            sys.exit("error: %s is not in the local Hugging Face cache (%s)" % (base, type(e).__name__))
        revision = os.path.basename(os.path.normpath(path))
    if os.path.exists(os.path.join(path, "adapter_config.json")):
        sys.exit("error: %s is a LoRA adapter, not a base model" % base)
    if os.path.exists(os.path.join(path, PROVENANCE)):
        sys.exit("error: %s is a fine-tune (it has %s); train from the base model" % (base, PROVENANCE))
    if root and inside(path, root):
        sys.exit("error: %s is under the data root; train from the base model" % base)
    return {"id": base, "revision": revision, "path": path}


def environment():
    import platform
    env = {"python": platform.python_version(), "platform": platform.platform()}
    for name in ("torch", "transformers", "peft", "accelerate"):
        try:
            env[name] = __import__(name).__version__
        except ImportError:
            env[name] = None
    try:
        import torch
        if torch.cuda.is_available():
            env["cuda"] = torch.version.cuda
            env["gpu"] = torch.cuda.get_device_name(0)
    except ImportError:
        pass
    return env


def result_line(root, run, status, score_path=None, extra=None):
    """One line for <root>/runs.jsonl: what the night trained, on what, and how it scored."""
    m = read_json(os.path.join(run_dir(root, run), "manifest.json"), {}) or {}
    res = m.get("results", {})
    line = {
        "run": run,
        "day": datetime.date.today().isoformat(),
        "at": now(),
        "status": status,
        "base": (m.get("base") or {}).get("id"),
        "mode": (m.get("hyper") or {}).get("mode"),
        "records": (m.get("records") or {}).get("train"),
        "steps": res.get("steps"),
        "train_loss": res.get("train_loss"),
        "eval_loss": res.get("eval_loss"),
        "hours": round(res.get("train_seconds", 0) / 3600, 3) if res else None,
        "gguf": {q: e.get("file") for q, e in (m.get("exports") or {}).items()},
        "commit": (m.get("repo") or {}).get("commit"),
    }
    if score_path and os.path.exists(score_path):
        with open(score_path, encoding="utf-8", errors="replace") as f:
            lines = [ln.strip() for ln in f if ln.strip()]
        try:
            line["score"] = json.loads(lines[-1]) if lines else None
        except ValueError:
            line["score"] = {"text": lines[-1][:500]}
    line.update(extra or {})
    return line


def main():
    utf8_stdio()
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("root", help="print the data root").add_argument("--root")
    r = sub.add_parser("result", help="append a night's result to <root>/runs.jsonl")
    r.add_argument("--root")
    r.add_argument("--run", required=True)
    r.add_argument("--status", required=True)
    r.add_argument("--score", help="the score command's output; its last line, if JSON, is kept")
    r.add_argument("--log", help="the night's log file")
    r.add_argument("--started")
    a = p.parse_args()
    root = data_root(a.root)
    if a.cmd == "root":
        print(root)
        return
    line = result_line(root, a.run, a.status, a.score,
                       {"log": os.path.normpath(a.log) if a.log else None, "started": a.started})
    append_line(os.path.join(root, "runs.jsonl"), json.dumps(line))
    print(json.dumps(line))


if __name__ == "__main__":
    main()
