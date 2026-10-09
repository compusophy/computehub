#!/usr/bin/env bash
# The team night (DESIGN.md, "The team: many models, one mind"): the cloud model and a local one
# working as one, scored as one. The cloud model's drafts are GLM's recorded answers (no cloud
# call is made); a local helper repairs the ones that do not run clean, as Studio asks a fix
# (tools/eval `team`), and the pair is scored on the held-out tasks: the combined pass rate, and
# cloud calls per solved task (GLM's one call a task over the tasks passed).
#
#   bash train/team.sh [--root DIR] STEP...
#
# Steps, each skipped once its output is there (a rerun resumes), none begun while
# <root>/iq/PAUSE exists, every model on llama-server at 8081 (the night's port):
#   glm            GLM alone on the held-out tasks (no GPU)
#   team:NAME:SPEC  GLM's drafts repaired by a helper, NAME, served from SPEC (base3b, base05, a
#                  run's name); held-out tasks; traces of every turn, never trained on
#   drafts         the untuned 3B writes each train task once (the drafts to repair)
#   train-traces   the untuned 3B repairs its own train drafts: the fix traces
#   mutants        eval mutants: each train task's reference broken one way GLM breaks programs,
#                  with the block that restores it (fix data at no cost, correct by construction)
#   fixdata        train/fixdata.py: the turns that made a program run clean, as SFT records
#   fix3b          a LoRA of the untuned 3B on those and the mutants (FIX_EPOCHS, 1 epoch)
#   summary        every score tonight, side by side
# Its inputs are copied once into <root>/iq/team-<night>/ (the suite, GLM's answers, the held
# families, the train prompts), and the tools (EVAL, IQ: this checkout's debug builds unless set)
# are copied there before each use, so the checkout can be rebuilt meanwhile.
set -uo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/.." && pwd)
PY=${PYTHON:-python}
ROOT=${COMPUTEHUB_DATA:-C:/sept30/computehub-data}
if [ "${1:-}" = --root ]; then ROOT=$2; shift 2; fi
export COMPUTEHUB_DATA="$ROOT"
D=$ROOT/iq
LARGE=Qwen/Qwen2.5-Coder-3B-Instruct
SMALL=Qwen/Qwen2.5-Coder-0.5B-Instruct
# A night is named by its evening: before noon, yesterday's.
if [ "$((10#$(date +%H)))" -lt 12 ]; then NIGHT=n$(date -d yesterday +%Y%m%d); else NIGHT=n$(date +%Y%m%d); fi
NIGHT=${TEAM_NIGHT:-$NIGHT}
T=$D/team-$NIGHT
W=$(cygpath -m "$T" 2>/dev/null || echo "$T")
LOG=$D/team-$NIGHT.log
EVAL=${EVAL:-$REPO/target/debug/eval.exe}
IQ=${IQ:-$REPO/target/debug/iq.exe}
URL=http://127.0.0.1:8081/v1/chat/completions

say() { echo "$(date '+%F %T') $*" | tee -a "$LOG"; }
paused() { [ -e "$D/PAUSE" ] && { say "PAUSE: $1 not begun"; return 0; }; return 1; }

# The inputs, once.
mkdir -p "$T/bin"
if [ ! -s "$T/inputs.sha256" ]; then
  cp "$REPO/evals/suites/iq.jsonl" "$D/answers-glm.jsonl" "$D/held.txt" "$D/prompts-train.jsonl" \
     "$D/prompts-held.jsonl" "$T/"
  (cd "$T" && sha256sum iq.jsonl answers-glm.jsonl held.txt prompts-train.jsonl prompts-held.jsonl \
     > inputs.sha256)
  say "inputs: $(wc -l < "$T/iq.jsonl") tasks, $(wc -l < "$T/answers-glm.jsonl") GLM answers"
fi

tools() { cp "$EVAL" "$T/bin/eval.exe" && cp "$IQ" "$T/bin/iq.exe"; }
spec() {  # spec NAME: export.py/serve.py's arguments for a helper
  case $1 in
    base3b) echo "--base-only --base $LARGE" ;;
    base05) echo "--base-only --base $SMALL" ;;
    *) echo "--run $1" ;;
  esac
}
serve() {  # serve SPEC: its GGUF made if need be, then on llama-server, 16 slots
  # shellcheck disable=SC2046
  (cd "$REPO" && "$PY" train/export.py $1 --quant q8_0 \
     && "$PY" train/serve.py $1 --quant q8_0 --parallel 16 --ctx 16384) >> "$LOG" 2>&1
}
unserve() { (cd "$REPO" && "$PY" train/serve.py --stop) >> "$LOG" 2>&1; }
score() {  # score ANSWERS OUT
  "$T/bin/iq.exe" score "$W/$1" --suite "$W/iq.jsonl" > "$T/$2" 2>&1
  say "$2: $(grep -m1 ' pass ' "$T/$2" | sed 's/^ *//')"
}

team() {  # team NAME ANSWERS SPEC: ANSWERS' drafts repaired by the helper SPEC
  local name=$1 answers=$2 sp=$3
  [ -s "$T/score-$name.txt" ] && return 0
  paused "team $name" && return 1
  say "team $name: $answers repaired by $sp"
  tools || return 1
  serve "$(spec "$sp")" || { say "serve failed: $sp"; unserve; return 1; }
  "$T/bin/eval.exe" team --suite "$W/iq.jsonl" --answers "$W/$answers" --helper-url "$URL" \
    --helper "$sp" --name "glm+$name" --out "$W/team-$name.jsonl" \
    --traces "$W/traces-$name.jsonl" --jobs 16 >> "$LOG" 2>&1
  local ok=$?
  unserve
  [ "$ok" = 0 ] || { say "team $name failed ($ok): rerun resumes it"; return 1; }
  score "team-$name.jsonl" "score-$name.txt"
}

for step in "$@"; do
  case $step in
    glm)
      [ -s "$T/score-glm.txt" ] || { tools && score answers-glm.jsonl score-glm.txt; } ;;
    team:*)
      IFS=: read -r _ name sp <<< "$step"
      team "$name" answers-glm.jsonl "$sp" || exit 1 ;;
    drafts)
      [ -e "$T/drafts-base3b.done" ] && continue
      paused drafts && exit 1
      say "drafts: the untuned 3B writes each train task once"
      serve "$(spec base3b)" || { unserve; exit 1; }
      (cd "$REPO" && "$PY" train/ask.py --prompts "$W/prompts-train.jsonl" \
         --out "$W/drafts-base3b.jsonl" --name base3b --k 1 --jobs 16) >> "$LOG" 2>&1
      ok=$?
      unserve
      [ "$ok" = 0 ] || { say "drafts failed ($ok): rerun resumes them"; exit 1; }
      touch "$T/drafts-base3b.done"
      say "drafts: $(wc -l < "$T/drafts-base3b.jsonl")" ;;
    train-traces)
      team train-base3b drafts-base3b.jsonl base3b || exit 1 ;;
    mutants)
      # Fix data at no cost: each train task's reference broken one way GLM breaks programs.
      [ -s "$T/fix-mutants.jsonl" ] && continue
      cat "$D"/refs-*.jsonl > "$T/refs.jsonl"
      tools || exit 1
      "$T/bin/eval.exe" mutants --suite "$W/iq.jsonl" --refs "$W/refs.jsonl" --held "$W/held.txt" \
        --out "$W/fix-mutants.jsonl" --per "${MUTANTS_PER:-1}" 2>&1 | tee -a "$LOG" ;;
    fixdata)
      [ -s "$T/fix.jsonl" ] && continue
      (cd "$REPO" && "$PY" train/fixdata.py --traces "$W/traces-train-base3b.jsonl" \
         --held "$W/held.txt" --suite "$W/iq.jsonl" --out "$W/fix.jsonl") 2>&1 | tee -a "$LOG" ;;
    fix3b)
      grep -q '^  "status": "done"' "$ROOT/runs/$NIGHT-fix3b/manifest.json" 2>/dev/null && continue
      paused fix3b && exit 1
      data=(--data "$W/fix.jsonl")
      [ -s "$T/fix-mutants.jsonl" ] && data+=(--data "$W/fix-mutants.jsonl")
      say "train: $NIGHT-fix3b, the untuned 3B on $(cat "$T/fix.jsonl" "$T"/fix-mutants.jsonl 2>/dev/null | wc -l) fix records"
      (cd "$REPO" && "$PY" train/sft.py --held "$W/held.txt" --tasks "$W/iq.jsonl" --base "$LARGE" \
         --run "$NIGHT-fix3b" "${data[@]}" --epochs "${FIX_EPOCHS:-1}" --batch 2 --accum 8 \
         --save-every 13) >> "$LOG" 2>&1 || { say "fix3b training failed: rerun resumes it"; exit 1; } ;;
    summary)
      say "summary:"
      for f in "$T"/score-*.txt; do
        printf '  %-22s %s\n' "$(basename "$f" .txt | sed 's/^score-//')" \
          "$(grep -m1 ' pass ' "$f" | sed 's/^ *//')" | tee -a "$LOG"
      done ;;
    *) say "unknown step: $step"; exit 2 ;;
  esac
done
