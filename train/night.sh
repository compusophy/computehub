#!/usr/bin/env bash
# The night run of the applang model: the 3B fine-tuned on apps and their checks, scored one
# shot and by choosing among its samples with its own checks, the untuned 3B beside it, the report;
# then, with --until-woken, rounds on the data the night's waves grow.
#
#   bash train/night.sh [--until-woken] [--force] [--wait] [--check] [--report] [--root DIR]
#                       [--no-3b] [--with-05] [--no-self]
#
# It reads, from <root>/iq/ (made by day.sh: the teacher's tasks in evals/suites/iq.jsonl, then
# `teach prompts`, `teach replies`, `teach export` and train/checkdata.py): sft.jsonl,
# solutions.jsonl, prompts-held.jsonl, prompts-train.jsonl, held.txt, the suite, and when there,
# checks.jsonl and prompts-held-checks.jsonl. It copies them once into <root>/iq/night-<night>/
# (with their sha256s in inputs.sha256) and reads only the copies, so tomorrow's data can be made
# while it runs (the night's workflow imports new tasks) and tonight trains and scores on what it
# began with. Then, each step skipped when its output exists, so a rerun after a freeze resumes
# on the same copies (sft.py from its checkpoint, the scorers from their last answers):
#   1. the 3B by LoRA from its base (Q3_EPOCHS, 1) on the apps and, when there, their checks
#      written from the asks (one run, two skills); scored on the held-out tasks one shot (q3, 4
#      samples a task), then made to write each held-out ask's check (q3-checks), and its sample
#      that its own check passes chosen (q3-sel, train/select.py: else one that compiles and runs);
#   2. the untuned 3B, answered once ever (new held-out tasks as they come): what tonight adds;
#   3. only with --with-05: the untuned and the tuned 0.5B, and a self-taught round (unless
#      --no-self): tonight's 0.5B answers the train tasks 8 times, its answers that pass join the
#      teacher's (at most 2 distinct programs a task), and a 0.5B is trained on both and scored;
#   4. report.py: every model's held-out pass rate tonight, with its interval by family, beside
#      the nights before (<root>/iq/report-<night>.md, iq-history.jsonl), and against what was
#      predicted for it before it was scored (<root>/iq/predictions-<night>.jsonl, when written);
#      a file short of its answers is partial.
# By hand, without --until-woken, the 3B starts only before Q3_BY o'clock and when it can end by
# 07:45 (its LoRA at about 850 tokens a second, its answers about 22 minutes for 268), and the
# self-taught round only before SELF_BY o'clock.
# The GPU is compusophy's from 08:00 to 22:00: it refuses to start then, and from 07:45 no new
# step starts; --force lifts both, and the start-by hours with them. --wait, started then, waits
# for 22:00 instead (and gives up if <root>/iq/PAUSE appears). --check says what tonight would
# do and whether it can (the hour, the inputs, the disk, the GPU, a night to resume), runs
# nothing, and works at any hour. --report writes the night's report from the answers it has,
# runs nothing on the GPU, and works at any hour (the morning's, when a step was stopped). One
# night.sh runs at a time (<root>/iq/night.pid).
# --until-woken (how /night starts it): no clock stops it, only <root>/iq/PAUSE, which the session
# touches when compusophy says they are up; the start-by hours are lifted; and when tonight's
# plan is done the GPU keeps going in rounds: the 3B trained again, from its base, on the data the
# night's waves have grown (ROUND_EPOCHS 1, once ROUND_MIN 200 new records are in), scored on
# tonight's held-out tasks as q3-r2, q3-r3, ...
# NIGHT_DRY=1 (with a scratch --root) runs the whole flow with made-up answers and runs, no model
# and no GPU: a by-day test of the night's control flow.
# Environment: COMPUTEHUB_DATA, PYTHON, MIN_FREE_GB (50), Q3_BY (3), SELF_BY (5), Q3_EPOCHS (1),
# ROUND_EPOCHS (1), ROUND_MIN (200), ROUND_WAIT (300 s), NIGHT_DRY.
# To pause: touch <root>/iq/PAUSE (no new step starts), then stop the running step's python.
set -uo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/.." && pwd)
PY=${PYTHON:-python}
FORCE=0 SELF=1 BIG=1 WAIT=0 CHECK=0 REPORT=0 UNTIL=0 SMALL_TOO=0
DRY=${NIGHT_DRY:-0}
ROOT_ARG=()
while [ $# -gt 0 ]; do
  case "$1" in
    --force) FORCE=1; shift ;;
    --wait) WAIT=1; shift ;;
    --check) CHECK=1; shift ;;
    --report) REPORT=1; shift ;;
    --until-woken) UNTIL=1; shift ;;
    --no-self) SELF=0; shift ;;
    --no-3b) BIG=0; shift ;;
    --with-05) SMALL_TOO=1; shift ;;
    --root) ROOT_ARG=(--root "$2"); shift 2 ;;
    *) echo "night: unknown argument: $1" >&2; exit 2 ;;
  esac
done
ROOT=$("$PY" "$HERE/common.py" root "${ROOT_ARG[@]}") || exit 2
if [ "$DRY" = 1 ] && [ ${#ROOT_ARG[@]} -eq 0 ]; then
  echo "night: NIGHT_DRY makes up answers and runs: give it a scratch data root (--root)" >&2; exit 2
fi
export COMPUTEHUB_DATA="$ROOT"   # sft.py and generate.py read it
D=$(cygpath -u "$ROOT" 2>/dev/null || echo "$ROOT")/iq   # this shell's path
W=$(cygpath -m "$D" 2>/dev/null || echo "$D")            # the tools' path
RUNS=${D%/iq}/runs                                       # sft.py's runs, by name
SUITE=$(cygpath -m "$REPO/evals/suites/iq.jsonl" 2>/dev/null || echo "$REPO/evals/suites/iq.jsonl")
SMALL=Qwen/Qwen2.5-Coder-0.5B-Instruct
LARGE=Qwen/Qwen2.5-Coder-3B-Instruct
# Samples a task: a baseline's, a fine-tune's on the held-out tasks, the student's on the train ones.
KB=2 KR=4 KS=8

INPUTS="sft.jsonl solutions.jsonl prompts-held.jsonl prompts-train.jsonl held.txt"
# By hand, the GPU is compusophy's from 08:00 to 22:00; with --until-woken their word started the
# night, whatever the hour.
daytime() { local h=$((10#$(date +%H))); [ "$FORCE" = 0 ] && [ "$UNTIL" = 0 ] && [ "$h" -ge 8 ] && [ "$h" -lt 22 ]; }
# Another night.sh running or waiting: its pid in night.pid, alive. An empty night.pid is none.
running() { [ -s "$D/night.pid" ] && kill -0 "$(cat "$D/night.pid")" 2>/dev/null; }
if [ "$CHECK" = 1 ]; then
  ok=1
  if daytime; then echo "hour: $(date +%H:%M), the GPU is compusophy's until 22:00 (--wait waits for it)"
  else echo "hour: $(date +%H:%M), the GPU may run"; fi
  [ -e "$D/PAUSE" ] && { echo "PAUSE: $D/PAUSE exists, so nothing starts (remove it to run)"; ok=0; }
  running && { echo "running: another night.sh (pid $(cat "$D/night.pid")) is running or waiting"; ok=0; }
  for f in $INPUTS; do
    if [ -s "$D/$f" ]; then echo "input: $f, $(wc -l < "$D/$f") lines"; else echo "input: $f MISSING"; ok=0; fi
  done
  free=$("$PY" -c "import shutil, sys; print(shutil.disk_usage(sys.argv[1]).free // 2**30)" "$ROOT")
  echo "disk: $free GB free (${MIN_FREE_GB:-50} needed)"; [ "$free" -ge "${MIN_FREE_GB:-50}" ] || ok=0
  if command -v nvidia-smi > /dev/null; then
    # Every window holds a graphics context, so only memory, load and a training or serving
    # process say the GPU is taken.
    echo "GPU: $(nvidia-smi --query-gpu=memory.used,memory.total,utilization.gpu --format=csv,noheader 2>/dev/null)"
    nvidia-smi --query-compute-apps=pid,process_name,used_memory --format=csv,noheader 2>/dev/null \
      | grep -i "python\|llama" | sed 's/^/  running: /'
  fi
  LLAMA_CUDA="${LLAMA_CPP_DIR:-C:/llama-cpp}/build-cuda/bin/llama-server.exe"
  if [ "${SCORER:-}" != hf ] && [ -f "$LLAMA_CUDA" ]; then echo "scorer: llama-server (CUDA)"; else echo "scorer: generate.py"; fi
  h=$((10#$(date +%H))); if [ "$h" -lt 12 ]; then n=n$(date -d yesterday +%Y%m%d); else n=n$(date +%Y%m%d); fi
  [ -d "$D/night-$n" ] && echo "resume: night $n began; a run now resumes it on its copied inputs"
  # The suite this checkout holds must hold every task the prompts ask (the night copies it).
  if [ -s "$D/prompts-held.jsonl" ]; then
    "$PY" - "$REPO/evals/suites/iq.jsonl" "$W/prompts-held.jsonl" "$W/prompts-train.jsonl" <<'PY' || ok=0
import json, sys
ids = {json.loads(l)["id"] for l in open(sys.argv[1], encoding="utf-8") if l.strip()}
miss = sorted({json.loads(l)["task"] for f in sys.argv[2:] for l in open(f, encoding="utf-8") if l.strip()} - ids)
print("suite: %d tasks%s" % (len(ids), "" if not miss else "; MISSING %d the prompts ask: %s" % (len(miss), " ".join(miss[:5]))))
sys.exit(1 if miss else 0)
PY
  fi
  [ "$ok" = 1 ] && echo "ready" && exit 0
  echo "not ready"; exit 1
fi
if [ "$REPORT" = 0 ]; then
  running && { echo "night: another night.sh (pid $(cat "$D/night.pid")) is running or waiting" >&2; exit 7; }
  echo $$ > "$D/night.pid"
  trap ': > "$D/night.pid"' EXIT
fi
if daytime && [ "$WAIT" = 1 ] && [ "$REPORT" = 0 ]; then
  echo "night: it is $(date +%H:%M); waiting for 22:00 (touch $D/PAUSE to give up)"
  while daytime; do
    [ -e "$D/PAUSE" ] && { echo "night: PAUSE appeared while waiting; not starting" >&2; exit 0; }
    sleep 60
  done
fi
hour=$((10#$(date +%H)))
if daytime && [ "$REPORT" = 0 ]; then
  echo "night: it is $(date +%H:%M); the GPU is compusophy's from 08:00 to 22:00 (--wait waits, --force overrides)" >&2
  exit 3
fi
for f in $INPUTS; do
  [ -s "$D/$f" ] || { echo "night: $D/$f is missing: make the day's data first" >&2; exit 5; }
done
free=$("$PY" -c "import shutil, sys; print(shutil.disk_usage(sys.argv[1]).free // 2**30)" "$ROOT") || exit 2
[ "$free" -ge "${MIN_FREE_GB:-50}" ] || { echo "night: only $free GB free; ${MIN_FREE_GB:-50} needed" >&2; exit 4; }
# The night is named for the evening it began, so a rerun at 03:00 resumes the run begun at 23:00.
if [ "$hour" -lt 12 ]; then N=n$(date -d yesterday +%Y%m%d); else N=n$(date +%Y%m%d); fi
LOG="$D/night-$N.log"
say() { echo "[$(date +%H:%M:%S)] $*" | tee -a "$LOG"; }
quiet() { grep -v "Warning\|warn(\|attn_output\|FutureWarning" | tee -a "$LOG" | tail -3; }

# Tonight's inputs, copied once: a rerun resumes on the same bytes, whatever the day's tools have
# made since. S is this shell's path to them, I the tools'.
S="$D/night-$N" I="$W/night-$N"
if [ "$REPORT" = 1 ] && [ ! -s "$S/inputs.sha256" ]; then
  echo "night: no night $N began here (no $S/inputs.sha256): nothing to report" >&2; exit 1
fi
if [ ! -s "$S/inputs.sha256" ]; then
  mkdir -p "$S"
  for f in $INPUTS; do cp "$D/$f" "$S/$f" || { say "could not copy $f"; exit 5; }; done
  cp "$REPO/evals/suites/iq.jsonl" "$S/iq.jsonl" || { say "could not copy the suite"; exit 5; }
  # The check-first skill's data and prompts, when day.sh made them (train/checkdata.py).
  for f in checks.jsonl prompts-held-checks.jsonl; do
    [ -s "$D/$f" ] && cp "$D/$f" "$S/$f"
  done
  # GLM's answers too: an import tonight adds its answers to new tasks this suite lacks.
  [ -s "$D/answers-glm.jsonl" ] && cp "$D/answers-glm.jsonl" "$S/answers-glm.jsonl"
  (cd "$S" && sha256sum $INPUTS iq.jsonl $(ls checks.jsonl prompts-held-checks.jsonl 2>/dev/null) \
    > inputs.sha256.part && mv inputs.sha256.part inputs.sha256)
  say "inputs copied to night-$N/ ($(wc -l < "$S/sft.jsonl") records, $(wc -l < "$S/prompts-held.jsonl") held-out tasks)"
fi
SUITE="$I/iq.jsonl"

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
  if [ "$DRY" = 1 ]; then  # NIGHT_DRY: K made-up answers a prompt, no model, no GPU
    say "dry: answers $out (k $k)"
    "$PY" - "$I/$prompts" "$W/$out" "$k" <<'PY'
import json, sys
rows = [json.loads(l) for l in open(sys.argv[1], encoding="utf-8") if l.strip()]
open(sys.argv[2], "w", encoding="utf-8").write("".join(
    json.dumps({"task": r["task"], "model": "dry", "reply": "dry"}) + "\n" for r in rows for _ in range(int(sys.argv[3]))))
PY
    return
  fi
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
          && "$PY" train/ask.py --prompts "$I/$prompts" --out "$W/$out" --name "$name" --k "$k" --jobs 16) 2>&1 | quiet; then
      (cd "$REPO" && "$PY" train/serve.py --stop) 2>&1 | quiet
      return
    fi
    (cd "$REPO" && "$PY" train/serve.py --stop) 2>&1 | quiet
    # A pause or the morning may be why it failed (its server stopped): then nothing more starts.
    morning
    say "llama-server failed for $(basename "$out"): generate.py answers it"
  fi
  say "generate: $(basename "$out") ($*, k $k)"
  (cd "$REPO" && "$PY" train/generate.py --prompts "$I/$prompts" --out "$W/$out" --k "$k" --batch "$batch" "$@") 2>&1 | quiet
}
sft() {  # sft RUN BASE [sft.py args]
  local run=$1 base=$2; shift 2
  if [ "$DRY" = 1 ]; then  # NIGHT_DRY: a run that says it finished, no model, no GPU
    say "dry: train $run on $base ($*)"
    mkdir -p "$RUNS/$run" && printf '{\n  "status": "done"\n}\n' > "$RUNS/$run/manifest.json"
    return
  fi
  say "train: $run on $base"
  (cd "$REPO" && "$PY" train/sft.py --held "$I/held.txt" --tasks "$SUITE" --base "$base" --run "$run" "$@") 2>&1 | quiet
}
started() { [ -e "$RUNS/$1/manifest.json" ]; }                                  # started RUN
finished() { grep -q '^  "status": "done"' "$RUNS/$1/manifest.json" 2>/dev/null; }  # finished RUN
complete() {  # complete ANSWERS PROMPTS K: every prompt's task has its K answers (1 if greedy)
  "$PY" - "$W/$1" "$I/$2" "$3" <<'PY'
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
  local files=() fk pred=()
  # What was predicted for tonight before it was scored (the night's workflow writes it).
  [ -s "$D/predictions-$N.jsonl" ] && pred=(--predictions "$W/predictions-$N.jsonl")
  # Each file with its samples a task, so report.py marks one its step left short as partial.
  # GLM's answers as copied with tonight's inputs (the live file may hold tasks they lack).
  [ -s "$S/answers-glm.jsonl" ] && files+=("$I/answers-glm.jsonl=1")
  for fk in "answers-base-q05.jsonl=$KB" "answers-base-q3.jsonl=$KB"; do
    [ -s "$D/${fk%=*}" ] && files+=("$W/$fk")
  done
  # Tonight's roles, the rounds among them: K samples a task, a choice (-sel) one; the models'
  # own checks (-checks) are not programs, so select.py's summary reports them.
  for fk in "$D/answers-$N"-*.jsonl; do
    [ -s "$fk" ] || continue
    case "$fk" in
      *-checks.jsonl) ;;
      *-sel.jsonl) files+=("$W/$(basename "$fk")=1") ;;
      *) files+=("$W/$(basename "$fk")=$KR") ;;
    esac
  done
  if [ ${#files[@]} -gt 0 ]; then
    (cd "$REPO" && "$PY" train/report.py --night "$N" --iq "$IQ" --suite "$SUITE" --data "$W" \
      --prompts "$I/prompts-held.jsonl" "${pred[@]}" "${files[@]}") 2>&1 | tee -a "$LOG"
  fi
  say "night $N: $1; the GPU is idle"
  exit 0
}
morning() {  # from 07:45 the GPU is compusophy's again, or whenever <root>/iq/PAUSE exists: no new
  # step starts (a rerun before noon the next day resumes the night; a later /night begins a new
  # one on that day's inputs). With --until-woken only PAUSE stops it: compusophy says when they
  # are up, and the session touches it.
  [ -e "$D/PAUSE" ] && finish "paused (iq/PAUSE)"
  [ "$UNTIL" = 1 ] && return 0
  local hm=$((10#$(date +%H) * 60 + 10#$(date +%M)))
  [ "$FORCE" = 0 ] && [ "$hm" -ge 465 ] && [ "$hm" -lt 1320 ] && finish "stopped for the morning"
  return 0
}
late() {  # late HOUR: past HOUR o'clock this night (the evening is before every start-by hour)
  local h=$((10#$(date +%H)))
  [ "$FORCE" = 0 ] && [ "$UNTIL" = 0 ] && [ "$h" -lt 18 ] && [ "$h" -ge "$1" ]
}
ends_by_morning() {  # ends_by_morning MINUTES: a step begun now and lasting MINUTES ends by 07:45
  local hm=$((10#$(date +%H) * 60 + 10#$(date +%M)))
  [ "$hm" -ge 1080 ] && hm=$((hm - 1440))   # the evening, as minutes before midnight
  [ "$FORCE" = 1 ] || [ "$UNTIL" = 1 ] || [ $((hm + $1)) -le 465 ]
}
q3_minutes() {  # the 3B's minutes: its LoRA unless done (about 6,400 tokens an app record, 1,500 a
  # check, at 850 a second), and its answers (about 22 minutes for 268, through llama-server)
  local records checks=0 held score
  records=$(wc -l < "$S/sft.jsonl")
  [ -s "$S/checks.jsonl" ] && checks=$(wc -l < "$S/checks.jsonl")
  held=$(wc -l < "$S/prompts-held.jsonl")
  score=$((held * KR * 22 / 268 + 10))
  if finished "$N-q3"; then echo "$score"; else
    echo $(((records * 6400 + checks * 1500) * ${Q3_EPOCHS:-1} / 850 / 60 + score)); fi
}

three_b() {  # three_b RUN ROLE SUB EPOCHS: the 3B trained on <night>/SUB's apps (and checks), scored
  # one shot as ROLE, and chosen among its samples by its own checks as ROLE-sel
  local run=$1 role=$2 sub=$3 epochs=$4 chk=()
  local s="$S${sub:+/$sub}" i="$I${sub:+/$sub}"
  [ -s "$s/checks.jsonl" ] && chk=(--data "$i/checks.jsonl")
  morning
  # Two sequences a micro-batch (16 a step, as 4 x 4): at 4, night n20261006's records (up to
  # 8,322 tokens) and the desktop's own share of the card overfilled its 24 GB, and Windows paged
  # it to system memory over PCIe (no step 5 in 90 minutes). A checkpoint every half epoch, so a
  # stopped run keeps what it learned.
  sft "$run" "$LARGE" --data "$i/sft.jsonl" "${chk[@]}" --epochs "$epochs" --batch 2 --accum 8 --save-every 13
  finished "$run" || return 1
  morning
  gen "answers-$N-$role.jsonl" prompts-held.jsonl "$KR" 32 --run "$run" --name "$role"
  if [ ${#chk[@]} -gt 0 ] && [ -s "$S/prompts-held-checks.jsonl" ]; then
    morning
    gen "answers-$N-$role-checks.jsonl" prompts-held-checks.jsonl 1 32 --run "$run" --name "$role-checks"
    "$PY" "$HERE/select.py" --iq "$IQ" --suite "$SUITE" --programs "$W/answers-$N-$role.jsonl" \
      --checks "$W/answers-$N-$role-checks.jsonl" --out "$W/answers-$N-$role-sel.jsonl" \
      --summary "$W/select-$N-$role.json" 2>&1 | tee -a "$LOG"
  fi
}

[ "$REPORT" = 1 ] && finish "report written (--report)"
say "night $N: $(wc -l < "$S/sft.jsonl") training records, $(wc -l < "$S/prompts-held.jsonl") held-out tasks"

# The 3B first: the model that learns (night n20261006: 6% held out, 34% of tier 1, against the
# 0.5B's 0.4%). One run learns two skills: the apps (sft.jsonl) and their checks, written from the
# ask alone (checks.jsonl, train/checkdata.py); then it is scored one shot (q3) and by choosing,
# among its samples, the one its own check passes (q3-sel, train/select.py).
if [ "$BIG" = 0 ]; then
  say "3B skipped (--no-3b)"
elif late "${Q3_BY:-3}"; then
  say "3B skipped: past ${Q3_BY:-3}:00"
elif need=$(q3_minutes); ! ends_by_morning "$need"; then
  say "3B skipped: it needs about $need minutes, which run past 07:45"
else
  three_b "$N-q3" q3 "" "${Q3_EPOCHS:-1}"
fi
# The untuned 3B, once ever (new held-out tasks are answered as they come): what tonight's adds.
morning
[ "$BIG" = 1 ] && gen answers-base-q3.jsonl prompts-held.jsonl "$KB" 32 --base "$LARGE"

# The 0.5B, a control, only when asked (--with-05): it learns the format, not the meaning (night
# n20261006: 1 of 268), and its GPU hours are the 3B's.
if [ "$SMALL_TOO" = 1 ]; then
  morning
  gen answers-base-q05.jsonl prompts-held.jsonl "$KB" 64 --base "$SMALL"
  morning
  sft "$N-q05" "$SMALL" --data "$I/sft.jsonl" --full
  morning
  gen "answers-$N-q05.jsonl" prompts-held.jsonl "$KR" 64 --run "$N-q05" --name q05
fi

if [ "$SMALL_TOO" = 0 ] || [ "$SELF" = 0 ]; then
  say "self-taught round skipped (it trains the 0.5B: --with-05)"
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
      "$TEACH" export --solutions "$W/solutions-self-$N.jsonl" --held "$I/held.txt" --out "$W/sft-self-$N.jsonl" 2>&1 | tee -a "$LOG"
    fi
  fi
  if [ "$ready" = 1 ] && [ ! -s "$D/sft-self-$N.jsonl" ]; then
    say "self-taught round: no answer of the student passed; nothing to train on"
  elif [ "$ready" = 1 ]; then
    morning
    sft "$N-q05-self" "$SMALL" --data "$I/sft.jsonl" --data "$W/sft-self-$N.jsonl" --full
    morning
    gen "answers-$N-q05-self.jsonl" prompts-held.jsonl "$KR" 64 --run "$N-q05-self" --name q05-self
  fi
fi

# Until compusophy wakes (--until-woken), the GPU keeps learning. Each round trains the 3B again,
# from its base, on the training data the night's waves have grown since the last round (one
# epoch, ROUND_EPOCHS, to fit), and scores it on tonight's held-out tasks as q3-r<k>: whether
# more data helps, read the same night on the same tasks. A round waits for ROUND_MIN (200) new
# records; its data is copied once, so a rerun resumes it on the same bytes.
round=2 fails=0
while [ "$UNTIL" = 1 ] && [ "$BIG" = 1 ]; do
  morning
  if ! started "$N-q3-r$round"; then
    if [ "$round" -gt 2 ]; then prev=$(wc -l < "$S/r$((round - 1))/sft.jsonl"); else prev=$(wc -l < "$S/sft.jsonl"); fi
    now=$(wc -l < "$D/sft.jsonl")
    if [ "$now" -lt $((prev + ${ROUND_MIN:-200})) ]; then
      say "round $round waits: $now records, $prev in the last round's data"
      sleep "${ROUND_WAIT:-300}"
      continue
    fi
    mkdir -p "$S/r$round"
    cp "$D/sft.jsonl" "$S/r$round/sft.jsonl.part"
    # A whole file (day.sh data may be writing it): every line reads.
    if ! "$PY" -c "import json, sys; [json.loads(l) for l in open(sys.argv[1], encoding='utf-8') if l.strip()]" \
         "$I/r$round/sft.jsonl.part"; then
      say "round $round: sft.jsonl was being written; again in a minute"
      sleep 60
      continue
    fi
    mv "$S/r$round/sft.jsonl.part" "$S/r$round/sft.jsonl"
    [ -s "$D/checks.jsonl" ] && cp "$D/checks.jsonl" "$S/r$round/checks.jsonl"
    (cd "$S/r$round" && sha256sum sft.jsonl $(ls checks.jsonl 2>/dev/null) > inputs.sha256)
    say "round $round: $(wc -l < "$S/r$round/sft.jsonl") records ($prev before)"
  fi
  if ! three_b "$N-q3-r$round" "q3-r$round" "r$round" "${ROUND_EPOCHS:-1}"; then
    fails=$((fails + 1))
    [ "$fails" -ge 2 ] && finish "round $round's training failed twice"
    continue
  fi
  round=$((round + 1)) fails=0
done

finish "done"
