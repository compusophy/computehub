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
#   from:NIGHT     an earlier night's fix data and train drafts copied in (what is not here yet)
#   glm            GLM alone on the held-out tasks (no GPU)
#   team:NAME:SPEC[:TRIES]  GLM's drafts repaired by a helper, NAME, served from SPEC (base3b,
#                  base05, base7b: Qwen's own q8_0 GGUF in <root>/models, a run of tonight's by its
#                  short name, any run's full name), up to TRIES
#                  repairs a draft (1; those after the first sampled hotter, the first that runs
#                  clean kept); held-out tasks; traces of every turn, never trained on
#   drafts         the untuned 3B writes each train task once (the drafts to repair)
#   train-traces   the untuned 3B repairs its own train drafts: the fix traces
#   mutants        eval mutants: each train task's reference broken one way GLM breaks programs,
#                  with the block that restores it (fix data at no cost, correct by construction)
#   fixdata        train/fixdata.py: the turns that made a program run clean, as SFT records
#   fix3b          a LoRA of the untuned 3B on those and the mutants (FIX_EPOCHS, 1 epoch)
#   train:NAME:FILE[+FILE...]  a LoRA of the untuned 3B, <night>-NAME, on those files here
#                  (TRAIN_EPOCHS 1, TRAIN_BATCH 1 x TRAIN_ACCUM 16: long records page at batch 2)
#   checks:NAME    the check writer: the 3B trained (CHECK_EPOCHS, 2) on the train tasks' checks
#                  no longer than CHECK_LINES (80) lines, made to write each held-out ask's check,
#                  and those checks judged beside the real ones on GLM's programs (select.py):
#                  read, fair to the reference, kept / falsely rejected / truly rejected / missed
#   rounds         until PAUSE: ROUND_SLICE (300) more train tasks drafted, repaired by the newest
#                  helper (ROUND_FIRST first, <night>-fix3b; ROUND_TRIES tries, 1), every fix turn
#                  so far and ROUND_EXTRA (fix-mutants.jsonl; +-joined) trained into the next
#                  (fix3b-r2, -r3, ...), scored as a team
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
# The 7B, a GGUF as Qwen publish it (no training, so no export): <root>/models/<name>/.
SEVEN=$(cygpath -m "$ROOT/models/qwen2.5-coder-7b-instruct" 2>/dev/null || echo "$ROOT/models/qwen2.5-coder-7b-instruct")
spec() {  # spec NAME: export.py/serve.py's arguments for a helper
  case $1 in
    base3b) echo "--base-only --base $LARGE" ;;
    base05) echo "--base-only --base $SMALL" ;;
    base7b) echo "--gguf $SEVEN/qwen2.5-coder-7b-instruct-q8_0.gguf --tokenizer $SEVEN" ;;
    *) if [ -d "$ROOT/runs/$NIGHT-$1" ]; then echo "--run $NIGHT-$1"; else echo "--run $1"; fi ;;
  esac
}
slots() {  # slots SPEC: requests at once; the 7B's weights (8.1 GB) leave room for 8 contexts
  case $1 in *7b*) echo 8 ;; *) echo 16 ;; esac
}
serve() {  # serve SPEC: its GGUF made if need be, then on llama-server, slots SPEC's slots
  # shellcheck disable=SC2046
  if [ "${1#--gguf}" != "$1" ]; then
    (cd "$REPO" && "$PY" train/serve.py $1 --parallel "$(slots "$1")" --ctx 16384) >> "$LOG" 2>&1
  else
    (cd "$REPO" && "$PY" train/export.py $1 --quant q8_0 \
       && "$PY" train/serve.py $1 --quant q8_0 --parallel "$(slots "$1")" --ctx 16384) >> "$LOG" 2>&1
  fi
}
unserve() { (cd "$REPO" && "$PY" train/serve.py --stop) >> "$LOG" 2>&1; }
score() {  # score ANSWERS OUT
  "$T/bin/iq.exe" score "$W/$1" --suite "$W/iq.jsonl" > "$T/$2" 2>&1
  say "$2: $(grep -m1 ' pass ' "$T/$2" | sed 's/^ *//')"
}

team() {  # team NAME ANSWERS SPEC [TRIES]: ANSWERS' drafts repaired by the helper SPEC
  local name=$1 answers=$2 sp=$3 tries=${4:-1}
  [ -s "$T/score-$name.txt" ] && return 0
  paused "team $name" && return 1
  say "team $name: $answers repaired by $sp, $tries tries"
  tools || return 1
  serve "$(spec "$sp")" || { say "serve failed: $sp"; unserve; return 1; }
  "$T/bin/eval.exe" team --suite "$W/iq.jsonl" --answers "$W/$answers" --helper-url "$URL" \
    --helper "$sp" --name "glm+$name" --out "$W/team-$name.jsonl" \
    --traces "$W/traces-$name.jsonl" --jobs "$(slots "$(spec "$sp")")" --tries "$tries" >> "$LOG" 2>&1
  local ok=$?
  unserve
  [ "$ok" = 0 ] || { say "team $name failed ($ok): rerun resumes it"; return 1; }
  score "team-$name.jsonl" "score-$name.txt"
}

train() {  # train RUN FILE...: a LoRA of the untuned 3B on FILEs (here), resumed if begun
  local run=$1 f data=()
  shift
  grep -q '^  "status": "done"' "$ROOT/runs/$run/manifest.json" 2>/dev/null && return 0
  paused "train $run" && return 1
  for f in "$@"; do
    [ -s "$T/$f" ] || { say "train $run: no $f"; return 1; }
    data+=(--data "$W/$f")
  done
  say "train: $run on $(cd "$T" && cat "$@" | wc -l) records ($*)"
  (cd "$REPO" && "$PY" train/sft.py --held "$W/held.txt" --tasks "$W/iq.jsonl" --base "$LARGE" \
     --run "$run" "${data[@]}" --epochs "${TRAIN_EPOCHS:-1}" --batch "${TRAIN_BATCH:-1}" \
     --accum "${TRAIN_ACCUM:-16}" --save-every 13) >> "$LOG" 2>&1 \
     || { say "$run training failed: rerun resumes it"; return 1; }
}

for step in "$@"; do
  case $step in
    from:*)
      # An earlier night's fix data and drafts, so tonight need not make them again.
      src=$D/team-${step#from:}
      [ -d "$src" ] || { say "from: no $src"; exit 1; }
      for f in fix.jsonl fix-mutants.jsonl fix-mutants2.jsonl fix-m2.jsonl refs.jsonl \
               drafts-base3b.jsonl drafts-base3b.done traces-train-base3b.jsonl; do
        [ -e "$src/$f" ] && [ ! -e "$T/$f" ] && cp "$src/$f" "$T/$f" && say "from ${step#from:}: $f"
      done ;;
    glm)
      [ -s "$T/score-glm.txt" ] || { tools && score answers-glm.jsonl score-glm.txt; } ;;
    team:*)
      IFS=: read -r _ name sp tries <<< "$step"
      team "$name" answers-glm.jsonl "$sp" "${tries:-1}" || exit 1 ;;
    train:*)
      IFS=: read -r _ name files <<< "$step"
      IFS=+ read -ra fs <<< "$files"
      train "$NIGHT-$name" "${fs[@]}" || exit 1 ;;
    checks:*)
      name=${step#checks:}
      [ -s "$T/checkfit-$name.json" ] && continue
      paused "checks $name" && exit 1
      tools || exit 1
      if [ ! -s "$T/checks-short.jsonl" ]; then
        (cd "$REPO" && "$PY" train/checkdata.py --iq "$W/bin/iq.exe" --suite "$W/iq.jsonl" \
           --held-prompts "$W/prompts-held.jsonl" --train-out "$W/checks.jsonl" \
           --held-out "$W/prompts-held-checks.jsonl" --day "$(date +%F)") >> "$LOG" 2>&1 \
           || { say "checkdata failed"; exit 1; }
        # Short checks only: last night's writer ran out of room on most (216 of 246 unclosed).
        "$PY" - "$W/checks.jsonl" "$W/checks-short.jsonl" "${CHECK_LINES:-80}" <<'EOF' | tee -a "$LOG"
import json, sys
src, out, most = sys.argv[1], sys.argv[2], int(sys.argv[3])
rows = [json.loads(l) for l in open(src, encoding="utf-8") if l.strip()]
short = [r for r in rows if r["messages"][-1]["content"].count("\n") <= most + 1]
open(out, "w", encoding="utf-8").writelines(json.dumps(r, ensure_ascii=False) + "\n" for r in short)
print("checks: %d of %d no longer than %d lines" % (len(short), len(rows), most))
EOF
        [ -s "$T/checks-short.jsonl" ] || { say "checks: none short enough"; exit 1; }
      fi
      TRAIN_EPOCHS=${CHECK_EPOCHS:-2} TRAIN_BATCH=2 TRAIN_ACCUM=8 train "$NIGHT-$name" checks-short.jsonl \
        || exit 1
      if [ ! -e "$T/answers-$name-checks.done" ]; then
        paused "checks $name: writing" && exit 1
        serve "$(spec "$name")" || { unserve; exit 1; }
        (cd "$REPO" && "$PY" train/ask.py --prompts "$W/prompts-held-checks.jsonl" \
           --out "$W/answers-$name-checks.jsonl" --name "$name-checks" --k 1 --jobs 16) >> "$LOG" 2>&1
        ok=$?
        unserve
        [ "$ok" = 0 ] || { say "checks $name: writing failed ($ok)"; exit 1; }
        touch "$T/answers-$name-checks.done"
      fi
      (cd "$REPO" && "$PY" train/select.py --iq "$W/bin/iq.exe" --suite "$W/iq.jsonl" \
         --programs "$W/answers-glm.jsonl" --checks "$W/answers-$name-checks.jsonl" \
         --out "$W/sel-$name.jsonl" --summary "$W/checkfit-$name.json") 2>&1 | tail -1 | tee -a "$LOG" ;;
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
        --out "$W/fix-mutants.jsonl" --per "${MUTANTS_PER:-1}" --max "${MUTANTS_MAX:-800}" 2>&1 \
        | tee -a "$LOG" ;;
    fixdata)
      [ -s "$T/fix.jsonl" ] && continue
      (cd "$REPO" && "$PY" train/fixdata.py --traces "$W/traces-train-base3b.jsonl" \
         --held "$W/held.txt" --suite "$W/iq.jsonl" --out "$W/fix.jsonl" --night "$NIGHT") \
         >> "$LOG" 2>&1 || { say "fixdata failed"; exit 1; } ;;
    fix3b)
      grep -q '^  "status": "done"' "$ROOT/runs/$NIGHT-fix3b/manifest.json" 2>/dev/null && continue
      paused fix3b && exit 1
      data=(--data "$W/fix.jsonl")
      [ -s "$T/fix-mutants.jsonl" ] && data+=(--data "$W/fix-mutants.jsonl")
      say "train: $NIGHT-fix3b, the untuned 3B on $(cat "$T/fix.jsonl" "$T"/fix-mutants.jsonl 2>/dev/null | wc -l) fix records"
      (cd "$REPO" && "$PY" train/sft.py --held "$W/held.txt" --tasks "$W/iq.jsonl" --base "$LARGE" \
         --run "$NIGHT-fix3b" "${data[@]}" --epochs "${FIX_EPOCHS:-1}" --batch 2 --accum 8 \
         --save-every 13) >> "$LOG" 2>&1 || { say "fix3b training failed: rerun resumes it"; exit 1; } ;;
    rounds)
      # Until PAUSE: the train tasks not drafted yet drafted by the untuned 3B (a slice a round),
      # repaired by the newest helper, every fix turn so far and the mutants trained into a new
      # helper from the base, scored as a team on the held-out tasks: fix3b-r2, -r3, ...
      k=2
      while ! paused "round $k"; do
        prev=${ROUND_FIRST:-$NIGHT-fix3b}
        [ "$k" -gt 2 ] && prev=$NIGHT-fix3b-r$((k - 1))
        run=$NIGHT-fix3b-r$k
        if [ ! -e "$T/drafts-r$k.done" ]; then
          "$PY" - "$W/prompts-train.jsonl" "$W" "$W/prompts-r$k.jsonl" "${ROUND_SLICE:-300}" <<'PY'
import glob, json, os, sys
prompts, w, out, n = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
done = set()
for f in glob.glob(os.path.join(w, "drafts-*.jsonl")):
    for line in open(f, encoding="utf-8"):
        try:
            done.add(json.loads(line)["task"])
        except Exception:
            pass
left = [l for l in open(prompts, encoding="utf-8") if json.loads(l)["task"] not in done]
open(out, "w", encoding="utf-8").writelines(left[:n])
print("round prompts: %d of %d left" % (min(n, len(left)), len(left)))
PY
          [ -s "$T/prompts-r$k.jsonl" ] || { say "rounds: every train task drafted"; break; }
          serve "$(spec base3b)" || { unserve; exit 1; }
          (cd "$REPO" && "$PY" train/ask.py --prompts "$W/prompts-r$k.jsonl" \
             --out "$W/drafts-r$k.jsonl" --name base3b --k 1 --jobs 16) >> "$LOG" 2>&1
          ok=$?
          unserve
          [ "$ok" = 0 ] || { say "round $k drafts failed ($ok)"; exit 1; }
          touch "$T/drafts-r$k.done"
        fi
        team "train-r$k" "drafts-r$k.jsonl" "$prev" "${ROUND_TRIES:-1}" || exit 1
        if [ ! -s "$T/fix-r$k.jsonl" ]; then
          traces=()
          for f in "$T"/traces-train-*.jsonl; do traces+=(--traces "$(cygpath -m "$f" 2>/dev/null || echo "$f")"); done
          (cd "$REPO" && "$PY" train/fixdata.py "${traces[@]}" --held "$W/held.txt" --suite "$W/iq.jsonl" \
             --out "$W/fix-r$k.jsonl" --night "$NIGHT") >> "$LOG" 2>&1 \
             || { say "round $k fixdata failed"; exit 1; }
        fi
        IFS=+ read -ra extra <<< "${ROUND_EXTRA:-fix-mutants.jsonl}"
        train "$run" "fix-r$k.jsonl" "${extra[@]}" || exit 1
        team "r$k" answers-glm.jsonl "$run" "${ROUND_TRIES:-1}" || exit 1
        k=$((k + 1))
      done ;;
    summary)
      say "summary:"
      for f in "$T"/score-*.txt; do
        printf '  %-22s %s\n' "$(basename "$f" .txt | sed 's/^score-//')" \
          "$(grep -m1 ' pass ' "$f" | sed 's/^ *//')" | tee -a "$LOG"
      done ;;
    *) say "unknown step: $step"; exit 2 ;;
  esac
done
