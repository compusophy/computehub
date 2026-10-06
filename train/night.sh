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
#   1. baselines, answered once ever: the untuned Qwen2.5-Coder 0.5B and 3B on the held-out tasks;
#   2. tonight's fine-tunes, each from its base: 0.5B in full, 3B by LoRA, scored on the held-out tasks;
#   3. a self-taught round (unless --no-self, or past SELF_BY o'clock): tonight's 0.5B answers the
#      train tasks 8 times, its answers that pass (iq's check and an icon that draws) join the
#      teacher's, at most 2 a task, and a 0.5B is trained on both from its base and scored;
#   4. report.py: every model's held-out pass rate tonight beside the nights before
#      (<root>/iq/report-<night>.md, iq-history.jsonl).
# The GPU is compusophy's from 08:00 to 22:00: it refuses to start then unless --force.
# Environment: COMPUTEHUB_DATA, PYTHON, MIN_FREE_GB (50), SELF_BY (4), Q3_EPOCHS (2: the 3B LoRA
# trains at about 380 tokens a second on these records, so 3 epochs of a few hundred take hours).
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
SUITE=$(cygpath -m "$REPO/evals/suites/iq.jsonl" 2>/dev/null || echo "$REPO/evals/suites/iq.jsonl")
SMALL=Qwen/Qwen2.5-Coder-0.5B-Instruct
LARGE=Qwen/Qwen2.5-Coder-3B-Instruct

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

gen() {  # gen OUT PROMPTS K BATCH (--base ID | --run NAME)
  local out=$1 prompts=$2 k=$3 batch=$4; shift 4
  say "generate: $(basename "$out") ($*, k $k)"
  (cd "$REPO" && "$PY" train/generate.py --prompts "$W/$prompts" --out "$W/$out" --k "$k" --batch "$batch" "$@") 2>&1 | quiet
}
sft() {  # sft RUN BASE [sft.py args]
  local run=$1 base=$2; shift 2
  say "train: $run on $base"
  (cd "$REPO" && "$PY" train/sft.py --held "$W/held.txt" --tasks "$SUITE" --base "$base" --run "$run" "$@") 2>&1 | quiet
}

finish() {  # the report, from whatever answers tonight has, then stop
  local files=()
  for f in answers-glm.jsonl answers-base-q05.jsonl answers-base-q3.jsonl "answers-$N-q05.jsonl"            "answers-$N-q05-self.jsonl" "answers-$N-q3.jsonl"; do
    [ -s "$D/$f" ] && files+=("$W/$f")
  done
  (cd "$REPO" && "$PY" train/report.py --night "$N" --iq "$IQ" --suite "$SUITE" --data "$W" "${files[@]}") 2>&1 | tee -a "$LOG"
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

say "night $N: $(wc -l < "$D/sft.jsonl") training records, $(wc -l < "$D/prompts-held.jsonl") held-out tasks"
morning
gen answers-base-q05.jsonl prompts-held.jsonl 2 64 --base "$SMALL"
morning
[ "$BIG" = 1 ] && gen answers-base-q3.jsonl prompts-held.jsonl 2 32 --base "$LARGE"

morning
sft "$N-q05" "$SMALL" --data "$W/sft.jsonl" --full
morning
gen "answers-$N-q05.jsonl" prompts-held.jsonl 4 64 --run "$N-q05"

hour=$((10#$(date +%H)))
if [ "$SELF" = 1 ] && { [ "$hour" -ge 18 ] || [ "$hour" -lt "${SELF_BY:-4}" ]; }; then
  morning
  gen "self-$N-q05.jsonl" prompts-train.jsonl 8 64 --run "$N-q05"
  : > "$D/solutions-self-$N.jsonl"
  "$TEACH" replies --suite "$SUITE" --replies "$W/self-$N-q05.jsonl" --teacher "$N-q05" \
    --out "$W/solutions-self-$N.jsonl" 2>&1 | tee -a "$LOG"
  "$PY" - "$W/solutions-self-$N.jsonl" <<'PY'
import json, sys
# At most 2 passing answers a task, so easy tasks the student already solves do not crowd out the rest.
rows, kept, per = [json.loads(l) for l in open(sys.argv[1], encoding="utf-8") if l.strip()], [], {}
for r in rows:
    if r.get("pass") and per.get(r["task"], 0) < 2:
        per[r["task"]] = per.get(r["task"], 0) + 1
        kept.append(r)
open(sys.argv[1], "w", encoding="utf-8").write("".join(json.dumps(r) + "\n" for r in kept))
print("self-taught: %d answers kept for %d tasks" % (len(kept), len(per)))
PY
  "$TEACH" export --solutions "$W/solutions-self-$N.jsonl" --held "$W/held.txt" --out "$W/sft-self-$N.jsonl" 2>&1 | tee -a "$LOG"
  morning
  sft "$N-q05-self" "$SMALL" --data "$W/sft.jsonl" --data "$W/sft-self-$N.jsonl" --full
  morning
  gen "answers-$N-q05-self.jsonl" prompts-held.jsonl 4 64 --run "$N-q05-self"
else
  say "self-taught round skipped (--no-self, or past ${SELF_BY:-4}:00)"
fi

if [ "$BIG" = 1 ]; then
  morning
  sft "$N-q3" "$LARGE" --data "$W/sft.jsonl" --epochs "${Q3_EPOCHS:-2}"
  morning
  gen "answers-$N-q3.jsonl" prompts-held.jsonl 4 32 --run "$N-q3"
fi

finish "done"
