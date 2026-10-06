#!/usr/bin/env bash
# The night run of the applang model: baselines, fine-tunes, a self-taught round, the IQ report.
#
#   bash train/night.sh [--force] [--root DIR] [--no-self] [--no-3b]
#
# It reads, from <root>/iq/ (made by day: the teacher's tasks in evals/suites/iq.jsonl, then
# `teach prompts`, `teach replies` and `teach export`): sft.jsonl, solutions.jsonl,
# prompts-held.jsonl, prompts-train.jsonl, held.txt. Then, each step skipped when its output
# exists, so a rerun after a freeze resumes (sft.py from its checkpoint, generate.py from its
# last batch):
#   1. baselines, answered once ever (again only if the sampler's settings change): the untuned
#      Qwen2.5-Coder 0.5B and 3B on the held-out tasks;
#   2. tonight's fine-tunes, each from its base: 0.5B in full, 3B by LoRA, scored on the held-out tasks;
#   3. a self-taught round (unless --no-self, or past SELF_BY o'clock): tonight's 0.5B answers the
#      train tasks 8 times, its answers that pass (iq's check and an icon that draws) join the
#      teacher's, at most 2 distinct programs a task, and a 0.5B is trained on both from its base
#      and scored; not when none passed. Its data is made once: from its training's start on, a
#      rerun trains on the same bytes;
#   4. report.py: every model's held-out pass rate tonight beside the nights before
#      (<root>/iq/report-<night>.md, iq-history.jsonl); a file short of its answers is partial.
# The 3B round (LoRA and its answers) runs before the self-taught round, so the bigger model's
# fine-tune is never crowded out by it; it starts only before Q3_BY o'clock, and only when it can
# end by 07:45: its LoRA at about 380 tokens a second over the records (about 6,000 tokens each)
# Q3_EPOCHS times, and 40 minutes to answer. The self-taught round then starts only before
# SELF_BY o'clock.
# The GPU is compusophy's from 08:00 to 22:00: it refuses to start then, and from 07:45 no new
# step starts; --force lifts both, and the start-by hours with them.
# Environment: COMPUTEHUB_DATA, PYTHON, MIN_FREE_GB (50), Q3_BY (3), SELF_BY (5), Q3_EPOCHS (2).
# To pause: touch <root>/iq/PAUSE (no new step starts), then stop the running step's python.
set -uo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/.." && pwd)
PY=${PYTHON:-python}
FORCE=0 SELF=1 BIG=1
ROOT_ARG=()
while [ $# -gt 0 ]; do
  case "$1" in
    --force) FORCE=1; shift ;;
    --no-self) SELF=0; shift ;;
    --no-3b) BIG=0; shift ;;
    --root) ROOT_ARG=(--root "$2"); shift 2 ;;
    *) echo "night: unknown argument: $1" >&2; exit 2 ;;
  esac
done
ROOT=$("$PY" "$HERE/common.py" root "${ROOT_ARG[@]}") || exit 2
export COMPUTEHUB_DATA="$ROOT"   # sft.py and generate.py read it
D=$(cygpath -u "$ROOT" 2>/dev/null || echo "$ROOT")/iq   # this shell's path
W=$(cygpath -m "$D" 2>/dev/null || echo "$D")            # the tools' path
RUNS=${D%/iq}/runs                                       # sft.py's runs, by name
SUITE=$(cygpath -m "$REPO/evals/suites/iq.jsonl" 2>/dev/null || echo "$REPO/evals/suites/iq.jsonl")
SMALL=Qwen/Qwen2.5-Coder-0.5B-Instruct
LARGE=Qwen/Qwen2.5-Coder-3B-Instruct
# Samples a task: a baseline's, a fine-tune's on the held-out tasks, the student's on the train ones.
KB=2 KR=4 KS=8

hour=$((10#$(date +%H)))
if [ "$FORCE" = 0 ] && [ "$hour" -ge 8 ] && [ "$hour" -lt 22 ]; then
  echo "night: it is $(date +%H:%M); the GPU is compusophy's from 08:00 to 22:00 (--force overrides)" >&2
  exit 3
fi
for f in sft.jsonl solutions.jsonl prompts-held.jsonl prompts-train.jsonl held.txt; do
  [ -s "$D/$f" ] || { echo "night: $D/$f is missing: make the day's data first" >&2; exit 5; }
done
free=$("$PY" -c "import shutil, sys; print(shutil.disk_usage(sys.argv[1]).free // 2**30)" "$ROOT") || exit 2
[ "$free" -ge "${MIN_FREE_GB:-50}" ] || { echo "night: only $free GB free; ${MIN_FREE_GB:-50} needed" >&2; exit 4; }
# The night is named for the evening it began, so a rerun at 03:00 resumes the run begun at 23:00.
if [ "$hour" -lt 12 ]; then N=n$(date -d yesterday +%Y%m%d); else N=n$(date +%Y%m%d); fi
LOG="$D/night-$N.log"
say() { echo "[$(date +%H:%M:%S)] $*" | tee -a "$LOG"; }
quiet() { grep -v "Warning\|warn(\|attn_output\|FutureWarning" | tee -a "$LOG" | tail -3; }

(cd "$REPO" && cargo build -q --release -p compusophy-teach -p compusophy-iq) || { say "build failed"; exit 6; }
TEACH="$REPO/target/release/teach"
IQ=$(cygpath -m "$REPO/target/release/iq.exe" 2>/dev/null || echo "$REPO/target/release/iq")

# The scorer: llama-server's CUDA build when it is there (continuous batching over 16 slots, the
# coder's system prompt cached once, each stream closed where Studio stops reading, so no batch
# waits on its longest sample: generate.py's 3B took 5 minutes a task), else generate.py's
# batches. SCORER=hf forces generate.py. A failed server attempt falls back to generate.py, which
# answers again what the server made (another engine, another "gen").
LLAMA_CUDA="${LLAMA_CPP_DIR:-C:/llama-cpp}/build-cuda/bin/llama-server.exe"
if [ "${SCORER:-}" != hf ] && [ -f "$LLAMA_CUDA" ]; then SERVER=1; else SERVER=0; fi
gen() {  # gen OUT PROMPTS K BATCH (--base ID | --run NAME) [--name ROLE]
  local out=$1 prompts=$2 k=$3 batch=$4; shift 4
  if [ "$SERVER" = 1 ]; then
    local spec=() name="" args=("$@")
    while [ $# -gt 0 ]; do
      case "$1" in
        --base) spec=(--base-only --base "$2"); name=${name:-$2}; shift 2 ;;
        --run) spec=(--run "$2"); name=${name:-run:$2}; shift 2 ;;
        --name) name=$2; shift 2 ;;
        *) shift ;;
      esac
    done
    set -- "${args[@]}"
    say "score: $(basename "$out") (${spec[*]}, k $k) on llama-server"
    if (cd "$REPO" && "$PY" train/export.py "${spec[@]}" --quant q8_0 \
          && "$PY" train/serve.py "${spec[@]}" --quant q8_0 --parallel 16 --ctx 12288 \
          && "$PY" train/ask.py --prompts "$W/$prompts" --out "$W/$out" --name "$name" --k "$k" --jobs 16) 2>&1 | quiet; then
      (cd "$REPO" && "$PY" train/serve.py --stop) 2>&1 | quiet
      return
    fi
    (cd "$REPO" && "$PY" train/serve.py --stop) 2>&1 | quiet
    say "llama-server failed for $(basename "$out"): generate.py answers it"
  fi
  say "generate: $(basename "$out") ($*, k $k)"
  (cd "$REPO" && "$PY" train/generate.py --prompts "$W/$prompts" --out "$W/$out" --k "$k" --batch "$batch" "$@") 2>&1 | quiet
}
sft() {  # sft RUN BASE [sft.py args]
  local run=$1 base=$2; shift 2
  say "train: $run on $base"
  (cd "$REPO" && "$PY" train/sft.py --held "$W/held.txt" --tasks "$SUITE" --base "$base" --run "$run" "$@") 2>&1 | quiet
}
started() { [ -e "$RUNS/$1/manifest.json" ]; }                                  # started RUN
finished() { grep -q '^  "status": "done"' "$RUNS/$1/manifest.json" 2>/dev/null; }  # finished RUN
complete() {  # complete ANSWERS PROMPTS K: every prompt's task has its K answers (1 if greedy)
  "$PY" - "$W/$1" "$W/$2" "$3" <<'PY'
import json, os, sys
have = {}
if os.path.exists(sys.argv[1]):
    for l in open(sys.argv[1], encoding="utf-8"):
        if l.strip():
            t = json.loads(l)["task"]
            have[t] = have.get(t, 0) + 1
rows = [json.loads(l) for l in open(sys.argv[2], encoding="utf-8") if l.strip()]
need = lambda r: int(sys.argv[3]) if float(r["temperature"]) > 0 else 1
sys.exit(0 if all(have.get(r["task"], 0) >= need(r) for r in rows) else 1)
PY
}

finish() {  # the report, from whatever answers tonight has, then stop
  local files=() fk
  # Each file with its samples a task, so report.py marks one its step left short as partial.
  for fk in "answers-glm.jsonl=1" "answers-base-q05.jsonl=$KB" "answers-base-q3.jsonl=$KB" \
            "answers-$N-q05.jsonl=$KR" "answers-$N-q05-self.jsonl=$KR" "answers-$N-q3.jsonl=$KR"; do
    [ -s "$D/${fk%=*}" ] && files+=("$W/$fk")
  done
  if [ ${#files[@]} -gt 0 ]; then
    (cd "$REPO" && "$PY" train/report.py --night "$N" --iq "$IQ" --suite "$SUITE" --data "$W" \
      --prompts "$W/prompts-held.jsonl" "${files[@]}") 2>&1 | tee -a "$LOG"
  fi
  say "night $N: $1; the GPU is idle"
  exit 0
}
morning() {  # from 07:45 the GPU is compusophy's again, or whenever <root>/iq/PAUSE exists: no new
  # step starts (the rest resumes the next night)
  [ -e "$D/PAUSE" ] && finish "paused (iq/PAUSE)"
  local hm=$((10#$(date +%H) * 60 + 10#$(date +%M)))
  [ "$FORCE" = 0 ] && [ "$hm" -ge 465 ] && [ "$hm" -lt 1320 ] && finish "stopped for the morning"
  return 0
}
late() {  # late HOUR: past HOUR o'clock this night (the evening is before every start-by hour)
  local h=$((10#$(date +%H)))
  [ "$FORCE" = 0 ] && [ "$h" -lt 18 ] && [ "$h" -ge "$1" ]
}
ends_by_morning() {  # ends_by_morning MINUTES: a step begun now and lasting MINUTES ends by 07:45
  local hm=$((10#$(date +%H) * 60 + 10#$(date +%M)))
  [ "$hm" -ge 1080 ] && hm=$((hm - 1440))   # the evening, as minutes before midnight
  [ "$FORCE" = 1 ] || [ $((hm + $1)) -le 465 ]
}
q3_minutes() {  # the 3B round's minutes: its LoRA unless done, and its answers
  local records
  records=$(wc -l < "$D/sft.jsonl")
  if finished "$N-q3"; then echo 40; else echo $((records * 6000 * ${Q3_EPOCHS:-2} / 380 / 60 + 40)); fi
}

say "night $N: $(wc -l < "$D/sft.jsonl") training records, $(wc -l < "$D/prompts-held.jsonl") held-out tasks"
morning
gen answers-base-q05.jsonl prompts-held.jsonl "$KB" 64 --base "$SMALL"
morning
[ "$BIG" = 1 ] && gen answers-base-q3.jsonl prompts-held.jsonl "$KB" 32 --base "$LARGE"

morning
sft "$N-q05" "$SMALL" --data "$W/sft.jsonl" --full
morning
gen "answers-$N-q05.jsonl" prompts-held.jsonl "$KR" 64 --run "$N-q05" --name q05

if [ "$BIG" = 0 ]; then
  say "3B round skipped (--no-3b)"
elif late "${Q3_BY:-3}"; then
  say "3B round skipped: past ${Q3_BY:-3}:00"
elif need=$(q3_minutes); ! ends_by_morning "$need"; then
  say "3B round skipped: it needs about $need minutes, which run past 07:45"
else
  morning
  sft "$N-q3" "$LARGE" --data "$W/sft.jsonl" --epochs "${Q3_EPOCHS:-2}"
  morning
  gen "answers-$N-q3.jsonl" prompts-held.jsonl "$KR" 32 --run "$N-q3" --name q3
fi

if [ "$SELF" = 0 ]; then
  say "self-taught round skipped (--no-self)"
elif late "${SELF_BY:-5}"; then
  say "self-taught round skipped: past ${SELF_BY:-5}:00"
elif ! finished "$N-q05"; then
  say "self-taught round skipped: $N-q05 did not finish"
else
  ready=1
  # Its data is made once: from its training's start on, sft.py takes only the same bytes.
  if ! started "$N-q05-self"; then
    morning
    gen "self-$N-q05.jsonl" prompts-train.jsonl "$KS" 64 --run "$N-q05"
    morning
    if ! complete "self-$N-q05.jsonl" prompts-train.jsonl "$KS"; then
      say "self-taught round skipped: self-$N-q05.jsonl is short of answers (a rerun finishes it)"
      ready=0
    else
      : > "$D/solutions-self-$N.jsonl"
      "$TEACH" replies --suite "$SUITE" --replies "$W/self-$N-q05.jsonl" --teacher "$N-q05" \
        --out "$W/solutions-self-$N.jsonl" 2>&1 | tee -a "$LOG"
      "$PY" - "$W/solutions-self-$N.jsonl" <<'PY'
import json, sys
# At most 2 distinct passing programs a task, so easy tasks the student already solves do not
# crowd out the rest (export keeps one copy of a program, so a second copy would be no answer).
rows, kept, per = [json.loads(l) for l in open(sys.argv[1], encoding="utf-8") if l.strip()], [], {}
for r in rows:
    seen = per.setdefault(r["task"], set())
    if r.get("pass") and r.get("program") not in seen and len(seen) < 2:
        seen.add(r.get("program"))
        kept.append(r)
open(sys.argv[1], "w", encoding="utf-8").write("".join(json.dumps(r) + "\n" for r in kept))
print("self-taught: %d answers kept for %d tasks" % (len(kept), sum(1 for s in per.values() if s)))
PY
      : > "$D/sft-self-$N.jsonl"
      "$TEACH" export --solutions "$W/solutions-self-$N.jsonl" --held "$W/held.txt" --out "$W/sft-self-$N.jsonl" 2>&1 | tee -a "$LOG"
    fi
  fi
  if [ "$ready" = 1 ] && [ ! -s "$D/sft-self-$N.jsonl" ]; then
    say "self-taught round: no answer of the student passed; nothing to train on"
  elif [ "$ready" = 1 ]; then
    morning
    sft "$N-q05-self" "$SMALL" --data "$W/sft.jsonl" --data "$W/sft-self-$N.jsonl" --full
    morning
    gen "answers-$N-q05-self.jsonl" prompts-held.jsonl "$KR" 64 --run "$N-q05-self" --name q05-self
  fi
fi

finish "done"
