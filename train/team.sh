#!/usr/bin/env bash
# The team night (DESIGN.md, "The team: many models, one mind"): the cloud model and a local one
# working as one, scored as one. The cloud model's drafts are GLM's recorded answers (no cloud
# call is made); a local helper repairs the ones that do not run clean, as Studio asks a fix
# (tools/eval `team`), and the pair is scored on the held-out tasks: the combined pass rate, and
# cloud calls per solved task (GLM's one call a task over the tasks passed).
#
#   bash train/team.sh [--root DIR] [--check] STEP...
#
# Steps, each skipped once its output is there (a rerun resumes), every model on llama-server at
# TEAM_PORT (8081, the night's):
#   inputs         train/nightprep.py's inputs, each made once: suite-wait.jsonl (every check only
#                  "runs clean"), answers-dev.jsonl (mutants on train roots: known answers, real
#                  checks), the judge sets with their comments stripped (judge-*-s.jsonl), and
#                  q3rec.jsonl (the 3B's recorded grammar samples); from TEAM_PREP (<root>/scratch/
#                  prep) and TEAM_Q3REC
#   pre            answers-pre.jsonl, the harness alone (the last clean block, salvage, the fixers;
#                  copied only when its sha begins with TEAM_PRE_SHA), scored; then prompts-fin.jsonl
#                  (the replies pre left with no program, GLM's own notes appended)
#   glm            GLM alone on the held-out tasks (no GPU)
#   team:NAME:SPEC[:TRIES[:ANSWERS[:FLAGS]]]  ANSWERS' drafts (answers-glm.jsonl) repaired by a
#                  helper, NAME, served from SPEC (base3b, base05, base7b: Qwen's own q8_0 GGUF in
#                  <root>/models, a run of tonight's by its short name, any run's full name), up to
#                  TRIES (1) repairs a draft, every try run and logged (tries-NAME.jsonl), seeded
#                  (a NAME ending -sK by K, else 1), samplers pinned, icon-only drafts left alone;
#                  FLAGS +-joined eval words (whole: --whole, whole programs asked for); traces of
#                  every turn, never trained on
#   finish:NAME:SPEC:K  GLM's replies that ran out before a program, written whole by SPEC from
#                  GLM's notes (prompts-fin.jsonl), K samples a task (answers-NAME-samples.jsonl),
#                  the first that runs clean picked (answers-NAME.jsonl; nightprep.py pick)
#   judge:NAME:SPEC  train/judge.py: does SPEC tell a passing program from a clean failing one
#                  better than its length does? (judge-NAME.jsonl, judgefit-NAME.json)
#   from:NIGHT     an earlier night's fix data and train drafts copied in (what is not here yet)
#   drafts         the untuned 3B writes each train task once (the drafts to repair)
#   train-traces   the untuned 3B repairs its own train drafts: the fix traces
#   mutants        eval mutants: each train task's reference broken one way GLM breaks programs,
#                  with the block that restores it (fix data at no cost, correct by construction)
#   fixdata        train/fixdata.py: the turns that made a program run clean, as SFT records
#   fix3b          a LoRA of the untuned 3B on those and the mutants (FIX_EPOCHS, 1 epoch)
#   train:NAME:FILE[+FILE...][:EPOCHS[:BATCH[:ACCUM]]]  a LoRA of the untuned 3B, <night>-NAME,
#                  on those files here (TRAIN_EPOCHS 1, TRAIN_BATCH 1 x TRAIN_ACCUM 16 unless
#                  given: long records page at batch 2)
#   checks:NAME    the check writer: the 3B trained (CHECK_EPOCHS, 2) on the train tasks' checks
#                  no longer than CHECK_LINES (80) lines, made to write each held-out ask's check,
#                  and those checks judged beside the real ones on GLM's programs (select.py):
#                  read, fair to the reference, kept / falsely rejected / truly rejected / missed
#   rounds         until PAUSE: ROUND_SLICE (300) more train tasks drafted, repaired by the newest
#                  helper (ROUND_FIRST first, <night>-fix3b; ROUND_TRIES tries, 1), every fix turn
#                  so far and ROUND_EXTRA (fix-mutants.jsonl; +-joined) trained into the next
#                  (fix3b-r2, -r3, ...), scored as a team
#   summary        every score tonight; then train/compose.py (the stages in TEAM_ORDER composed
#                  offline, never lowering a grade: answers-final.jsonl; the try curves; report.md,
#                  with the verdict of each rule in decisions.json, into the log) and
#                  train/paired.py (paired.md: the pairs in TEAM_VS, task by task; pre's
#                  in-sample gain counted, never tested)
#   replicates     until PAUSE, for k = 2 .. 9: team:b3-sk (base3b, 8 tries) and team:b7-sk
#                  (base7b, 4) on answers-pre.jsonl, then summary: the noise of a run, measured
#
# It cannot end silently. A step that fails is retried once at once (the server stopped first),
# then the night goes on; a step whose input is not there says "skip: needs F"; every failed or
# skipped step runs once more after the list. So a "failed" line never means team.sh ended: only
# "the GPU is idle" does, written on every exit. No step begins while <root>/iq/PAUSE exists, and
# a launch with PAUSE there is refused, unless no step needs the GPU (glm, inputs, pre, summary,
# from:, run by hand while the night is paused). One team.sh runs at a time (<root>/iq/team.pid).
# A model is served only once the GPU holds under 3,000 MiB (TEAM_VRAM_MAX), waiting 10 minutes
# for it at most, and a server of the same helper is kept between steps; training wants 20 GB of
# disk free too. A watchdog logs the GPU's load every 15 minutes and a STALL when nothing has been
# written for 40 (TEAM_WATCH, TEAM_STALL: seconds); it only logs. --check says, PASS or FAIL each,
# whether a night could start now, and exits 1 on any FAIL.
#
# Its inputs are copied once into <root>/iq/team-<night>/ (the suite, GLM's answers, the held
# families, the train prompts), and the tools (EVAL, IQ: this checkout's debug builds unless set)
# at its first launch, pinned there (bin/pinned, their sha256; EVAL_REFRESH=1 copies them again),
# so the checkout can be rebuilt meanwhile. An eval whose `features` lack the seeds and try logs
# runs degraded: the first clean of K tries kept, no try logs. The night's name is TEAM_NIGHT,
# else the one the night's first launch wrote in <root>/iq/team-night (under 36 hours ago), else
# its evening's (before noon, yesterday's). TEAM_NOSERVE=1, for tests by day only, serves nothing.
set -uo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/.." && pwd)
PY=${PYTHON:-python}
ROOT=${COMPUTEHUB_DATA:-C:/sept30/computehub-data}
CHECK=0
while [ $# -gt 0 ]; do
  case $1 in
    --root) ROOT=${2:?--root needs a folder}; shift 2 ;;
    --check) CHECK=1; shift ;;
    *) break ;;
  esac
done
export COMPUTEHUB_DATA="$ROOT"
D=$ROOT/iq
[ -d "$D" ] || { echo "team: no $D (a data root holds iq/)" >&2; exit 2; }
LARGE=Qwen/Qwen2.5-Coder-3B-Instruct
SMALL=Qwen/Qwen2.5-Coder-0.5B-Instruct
PORT=${TEAM_PORT:-8081}
NOSERVE=${TEAM_NOSERVE:-0}
PREP=${TEAM_PREP:-$ROOT/scratch/prep}
PRE_SHA=${TEAM_PRE_SHA:-2e24176c71a87e68}
Q3REC=${TEAM_Q3REC:-$D/answers-n20261007-q3-gram.jsonl}
VRAM_MAX=${TEAM_VRAM_MAX:-3000}
VRAM_WAIT=${TEAM_VRAM_WAIT:-600}
WATCH=${TEAM_WATCH:-300}
STALL=${TEAM_STALL:-2400}
# The stages compose.py merges, in order, and the pairs paired.py tests. pre, the harness, was
# fitted on these held tasks: paired.py gives a pair its gain tells apart counts, never a test
# (--in-sample pre), so GLM against pre is no decision (compose.py's report counts it, apart).
ORDER=${TEAM_ORDER:-pre,b3,b7,fin7,fin7r,q3rec}
read -ra VS <<< "${TEAM_VS:-pre:final pre:b3 pre:b7 b3:b7 b3:b3w b05:b3 pre:b3raw}"

# The night's name: TEAM_NIGHT, else what its first launch wrote under 36 hours ago, else its
# evening's (before noon, yesterday's: a night's rounds may run past midnight into the morning).
if [ "$((10#$(date +%H)))" -lt 12 ]; then NIGHT=n$(date -d yesterday +%Y%m%d); else NIGHT=n$(date +%Y%m%d); fi
NAMED="the clock"
FRESH=
[ -s "$D/team-night" ] && [ -n "$(find "$D/team-night" -mmin -2160 2>/dev/null)" ] && FRESH=$(head -1 "$D/team-night")
if [ -n "${TEAM_NIGHT:-}" ]; then NIGHT=$TEAM_NIGHT NAMED=TEAM_NIGHT
elif [ -n "$FRESH" ]; then NIGHT=$FRESH NAMED="$D/team-night"
fi
case $NIGHT in ''|*[!A-Za-z0-9._-]*) echo "team: the night's name '$NIGHT' is not a folder's name" >&2; exit 2 ;; esac
T=$D/team-$NIGHT
W=$(cygpath -m "$T" 2>/dev/null || echo "$T")
PREPW=$(cygpath -m "$PREP" 2>/dev/null || echo "$PREP")
LOG=$D/team-$NIGHT.log
EVAL=${EVAL:-$REPO/target/debug/eval.exe}
IQ=${IQ:-$REPO/target/debug/iq.exe}
BASE=http://127.0.0.1:$PORT
URL=$BASE/v1/chat/completions
# The 7B, a GGUF as Qwen publish it (no training, so no export): <root>/models/<name>/.
SEVEN=$(cygpath -m "$ROOT/models/qwen2.5-coder-7b-instruct" 2>/dev/null || echo "$ROOT/models/qwen2.5-coder-7b-instruct")
# What the night's eval must do for every step to run in full (`eval features`).
WANT=(seed pin all-tries tries-out skip-icon)

say() { echo "$(date '+%F %T') $*" | tee -a "$LOG"; }
HAND=0  # 1: launched by hand while PAUSE exists, with steps that need no GPU
SERVED=  # the helper llama-server serves for this run, if any
paused() { [ "$HAND" = 0 ] && [ -e "$D/PAUSE" ] && { say "PAUSE: $1 not begun"; return 0; }; return 1; }
vram() { nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits 2>/dev/null | head -1 | tr -dc 0-9; }
free_gb() { "$PY" -c "import shutil, sys; print(shutil.disk_usage(sys.argv[1]).free // 2**30)" "$ROOT" | tr -dc 0-9; }
busy() {  # busy PORT: something listens on 127.0.0.1:PORT
  "$PY" -c "import socket, sys; s = socket.socket(); s.settimeout(1); sys.exit(s.connect_ex(('127.0.0.1', int(sys.argv[1]))) != 0)" "$1"
}
holder() {  # the pid of a team.sh that holds the lock, if one lives
  local pid
  pid=$(head -1 "$D/team.pid" 2>/dev/null)
  case $pid in ''|*[!0-9]*) return 1 ;; esac
  kill -0 "$pid" 2>/dev/null || return 1
  # A pid reused by another program is not a team.sh.
  if [ -r "/proc/$pid/cmdline" ]; then tr '\0' ' ' < "/proc/$pid/cmdline" | grep -q 'team\.sh' || return 1; fi
  echo "$pid"
}
has() { case " $FEATURES " in *" $1 "*) return 0 ;; esac; return 1; }  # has WORD: eval team does it
features() { "$1" features 2>/dev/null | grep '^team:' | tr '\n' ' '; }

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
gpu_free() {  # until the GPU holds under VRAM_MAX MiB (no other model on it), VRAM_WAIT s at most
  local used waited=0
  command -v nvidia-smi > /dev/null || return 0
  while :; do
    used=$(vram)
    if [ -z "$used" ] || [ "$used" -lt "$VRAM_MAX" ]; then return 0; fi
    if [ "$waited" -ge "$VRAM_WAIT" ] || [ -e "$D/PAUSE" ]; then break; fi
    sleep 15
    waited=$((waited + 15))
  done
  say "failed: VRAM held by others ($used MiB)"
  return 1
}
healthy() { curl -sf -m 5 "$BASE/health" 2> /dev/null | grep -q '"ok"'; }
serve() {  # serve NAME: the helper NAME on llama-server, its GGUF made if need be; kept if it is up
  local s
  [ "$NOSERVE" = 1 ] && return 0
  [ "$SERVED" = "$1" ] && healthy && return 0
  unserve
  gpu_free || return 1
  s=$(spec "$1")
  # shellcheck disable=SC2086
  if [ "${s#--gguf}" != "$s" ]; then
    (cd "$REPO" && "$PY" train/serve.py $s --port "$PORT" --parallel "$(slots "$s")" --ctx 16384) >> "$LOG" 2>&1
  else
    (cd "$REPO" && "$PY" train/export.py $s --quant q8_0 \
       && "$PY" train/serve.py $s --port "$PORT" --quant q8_0 --parallel "$(slots "$s")" --ctx 16384) >> "$LOG" 2>&1
  fi || { say "failed: serve $1"; return 1; }
  SERVED=$1
}
unserve() {
  [ "$NOSERVE" = 1 ] && return 0
  SERVED=
  (cd "$REPO" && "$PY" train/serve.py --stop --port "$PORT") >> "$LOG" 2>&1
}

# Every step is known before anything begins: a misspelt one is refused at launch, not at 3 am.
num() { case $1 in ''|*[!0-9]*) return 1 ;; esac; }
valid() {
  local name sp tries answers flags files epochs batch accum k extra w
  local words=()
  case $1 in
    glm|inputs|pre|summary|replicates|drafts|train-traces|mutants|fixdata|fix3b|rounds) return 0 ;;
    from:?*|checks:?*) return 0 ;;
    team:*)
      IFS=: read -r _ name sp tries answers flags extra <<< "$1"
      if [ -z "$name" ] || [ -z "$sp" ] || [ -n "$extra" ]; then return 1; fi
      [ -z "$tries" ] || num "$tries" || return 1
      IFS=+ read -ra words <<< "$flags"
      for w in "${words[@]}"; do [ "$w" = whole ] || return 1; done ;;
    finish:*)
      IFS=: read -r _ name sp k extra <<< "$1"
      [ -n "$name" ] && [ -n "$sp" ] && num "$k" && [ -z "$extra" ] ;;
    judge:*)
      IFS=: read -r _ name sp extra <<< "$1"
      [ -n "$name" ] && [ -n "$sp" ] && [ -z "$extra" ] ;;
    train:*)
      IFS=: read -r _ name files epochs batch accum extra <<< "$1"
      if [ -z "$name" ] || [ -z "$files" ] || [ -n "$extra" ]; then return 1; fi
      for w in "$epochs" "$batch" "$accum"; do [ -z "$w" ] || num "$w" || return 1; done ;;
    *) return 1 ;;
  esac
}
cpu_only() {  # every step given needs no GPU
  local s
  for s in "$@"; do case $s in glm|inputs|pre|summary|from:*) ;; *) return 1 ;; esac; done
}

if [ "$CHECK" = 1 ]; then
  bad=0
  ok() { echo "PASS $*"; }
  no() { echo "FAIL $*"; bad=1; }
  if [ -e "$D/PAUSE" ]; then no "PAUSE: $D/PAUSE exists (rename it to start)"; else ok "PAUSE: absent"; fi
  if pid=$(holder); then no "lock: team.sh runs (pid $pid)"; else ok "lock: free"; fi
  if busy "$PORT"; then no "port: something listens on 127.0.0.1:$PORT"; else ok "port: $PORT free"; fi
  used=$(vram)
  if [ -z "$used" ]; then no "VRAM: nvidia-smi unreadable"
  elif [ "$used" -lt "$VRAM_MAX" ]; then ok "VRAM: $used MiB used (under $VRAM_MAX)"
  else no "VRAM: $used MiB used ($VRAM_MAX at most: another program holds the GPU)"; fi
  free=$(free_gb)
  if [ "${free:-0}" -ge 30 ]; then ok "disk: $free GB free"; else no "disk: ${free:-?} GB free (30 needed)"; fi
  FEATURES=$(features "$EVAL")
  lack=
  for w in "${WANT[@]}"; do has "$w" || lack="$lack $w"; done
  if [ ! -x "$EVAL" ] || [ ! -x "$IQ" ]; then no "tools: no $EVAL or $IQ (cargo build -p eval -p compusophy-iq)"
  elif [ -n "$lack" ]; then no "eval features: lacks$lack (the night would run degraded)"
  else ok "eval features: ${FEATURES% }"; fi
  if [ -s "$T/answers-pre.jsonl" ]; then pre=$T/answers-pre.jsonl; else pre=$PREP/mx-answers-pre.jsonl; fi
  sha=$(sha256sum "$pre" 2>/dev/null | cut -c1-${#PRE_SHA})
  if [ "$sha" = "$PRE_SHA" ]; then ok "pre: $(basename "$pre"), sha $sha"; else no "pre: $pre's sha is '${sha:-missing}', not $PRE_SHA"; fi
  for f in "$PREP/eng/pool-dev.jsonl" "$PREP/judge-train.jsonl" "$PREP/judge-eval-glm.jsonl" \
           "$PREP/judge-eval-alt.jsonl" "$Q3REC" "$SEVEN/qwen2.5-coder-7b-instruct-q8_0.gguf"; do
    if [ -s "$f" ]; then ok "input: $(basename "$f")"; else no "input: $f missing"; fi
  done
  for b in qwen2.5-coder-3b-instruct qwen2.5-coder-0.5b-instruct; do
    for g in "$ROOT"/base/"$b"-*-q8_0.gguf; do [ -s "$g" ] && break; g=; done
    if [ -n "$g" ]; then ok "input: $(basename "$g")"; else no "input: no $ROOT/base/$b-*-q8_0.gguf"; fi
  done
  if [ -s "$T/inputs.sha256" ]; then ok "inputs: copied into $T already"
  else
    miss=
    for f in answers-glm.jsonl held.txt prompts-train.jsonl prompts-held.jsonl; do [ -s "$D/$f" ] || miss="$miss $f"; done
    if [ -z "$miss" ]; then ok "inputs: in $D, to be copied"; else no "inputs: $D lacks$miss"; fi
  fi
  if [ "$NAMED" = "the clock" ]; then no "night: $NIGHT, from the clock (name it: TEAM_NIGHT=...)"
  else ok "night: $NIGHT (from $NAMED)"; fi
  for f in nightprep judge compose paired; do [ -s "$HERE/$f.py" ] || echo "  note: no train/$f.py: its steps fail or skip"; done
  [ -d "$T" ] && echo "  note: $T exists: a launch resumes it"
  if [ "$bad" = 0 ]; then echo "ready"; exit 0; fi
  echo "not ready"; exit 1
fi

[ $# -gt 0 ] || { echo "usage: bash train/team.sh [--root DIR] [--check] STEP... (see its head)" >&2; exit 2; }
for step in "$@"; do valid "$step" || { say "unknown step: $step; nothing begun"; exit 2; }; done

# Launch guards: PAUSE, then the lock. Neither refusal touches what a running night holds.
if [ -e "$D/PAUSE" ]; then
  if cpu_only "$@"; then HAND=1; say "PAUSE exists: $* by hand (no GPU)"
  else say "failed: PAUSE exists at launch; the GPU is idle"; exit 1; fi
fi
if pid=$(holder); then say "failed: team.sh already runs (pid $pid)"; exit 1; fi
echo $$ > "$D/team.pid"
REASON="its steps ran"
WATCHER=
ended() {
  local rc=$?
  [ -n "$WATCHER" ] && kill "$WATCHER" 2> /dev/null
  unserve
  echo ended > "$D/team.pid"   # overwritten, never removed
  say "the GPU is idle: team.sh ended ($REASON; exit $rc)"
}
trap ended EXIT
trap 'REASON="stopped by INT"; exit 130' INT
trap 'REASON="stopped by TERM"; exit 143' TERM
trap 'REASON="stopped by HUP"; exit 129' HUP
say "team.sh $* (pid $$, night $NIGHT from $NAMED, port $PORT)"
# The name holds for the night's later launches: a relaunch after noon, the summary by hand.
[ "$NAMED" != "$D/team-night" ] && [ "$FRESH" != "$NIGHT" ] && echo "$NIGHT" > "$D/team-night"

# The inputs, once.
mkdir -p "$T/bin"
if [ ! -s "$T/inputs.sha256" ]; then
  cp "$REPO/evals/suites/iq.jsonl" "$D/answers-glm.jsonl" "$D/held.txt" "$D/prompts-train.jsonl" \
     "$D/prompts-held.jsonl" "$T/" || { REASON="its inputs could not be copied"; exit 1; }
  (cd "$T" && sha256sum iq.jsonl answers-glm.jsonl held.txt prompts-train.jsonl prompts-held.jsonl \
     > inputs.sha256)
  say "inputs: $(wc -l < "$T/iq.jsonl") tasks, $(wc -l < "$T/answers-glm.jsonl") GLM answers"
fi
# A line cut by a kill: ended, so the next append starts a line of its own (and the cut one is
# dropped as unreadable where it is read).
for f in "$T"/team-*.jsonl "$T"/traces-*.jsonl "$T"/tries-*.jsonl "$T"/answers-*.jsonl; do
  if [ -s "$f" ] && [ -n "$(tail -c 1 "$f")" ]; then
    printf '\n' >> "$f"
    say "repaired: $(basename "$f") ended mid-line (a newline added)"
  fi
done

# The tools, copied and pinned at the night's first launch (EVAL_REFRESH=1: again).
tools() {
  if [ ! -s "$T/bin/pinned" ] || [ "${EVAL_REFRESH:-0}" = 1 ]; then
    if ! cp "$EVAL" "$T/bin/eval.exe" || ! cp "$IQ" "$T/bin/iq.exe"; then return 1; fi
    (cd "$T" && sha256sum bin/eval.exe bin/iq.exe) > "$T/bin/pinned"
    cat "$T/bin/pinned" >> "$T/inputs.sha256"
    say "tools pinned: $(cut -c1-16 "$T/bin/pinned" | tr '\n' ' ')(eval, iq)"
  elif ! (cd "$T" && sha256sum -c --quiet bin/pinned > /dev/null 2>&1); then
    say "WARNING: $T/bin no longer holds what bin/pinned says"
  fi
}
tools || { REASON="no tools to pin ($EVAL, $IQ)"; exit 1; }
FEATURES=$(features "$T/bin/eval.exe")
DEGRADED=0
lack=
for w in "${WANT[@]}"; do has "$w" || lack="$lack $w"; done
if [ -n "$lack" ]; then
  DEGRADED=1
  say "degraded: eval lacks$lack (the first clean of K tries kept, no seeds, no try logs)"
fi

prep() {  # prep SUBCOMMAND ARG...: train/nightprep.py; its exit 2 (an input missing) is a skip
  "$PY" "$HERE/nightprep.py" "$@" >> "$LOG" 2>&1
  case $? in
    0) return 0 ;;
    2) say "skip: needs nightprep.py $1's input (its line above)"; return 2 ;;
    *) say "failed: nightprep.py $1"; return 1 ;;
  esac
}
scored() { [ -s "$T/score-$1.txt" ] && [ -s "$T/each-$1.txt" ]; }
score() {  # score NAME FILE: FILE (here) graded on the real suite (each-NAME.txt, score-NAME.txt)
  local name=$1 file=$2 rc
  [ -s "$T/$file" ] || { say "skip: needs $file"; return 2; }
  [ -s "$HERE/nightprep.py" ] || { say "skip: needs train/nightprep.py"; return 2; }
  # The lines iq cannot read dropped first: one cut line must not fail the grading of the rest.
  "$PY" "$HERE/nightprep.py" clean "$W/$file" "$W/scored-$name.jsonl" >> "$LOG" 2>&1 \
    || { say "failed: score $name: nightprep.py clean $file"; return 1; }
  "$T/bin/iq.exe" score "$W/scored-$name.jsonl" --suite "$W/iq.jsonl" --each > "$T/each-$name.txt.tmp" 2>> "$LOG"
  rc=$?
  [ "$rc" = 0 ] || { say "failed: score $name: iq exited $rc, so nothing is scored"; return 1; }
  mv -f "$T/each-$name.txt.tmp" "$T/each-$name.txt"
  grep -v '^{' "$T/each-$name.txt" > "$T/score-$name.txt.tmp" && mv -f "$T/score-$name.txt.tmp" "$T/score-$name.txt"
  say "score-$name: $(grep -m1 ' pass ' "$T/score-$name.txt" | sed 's/^ *//')"
}
tasks() {  # tasks ANSWERS OUT: the suite's tasks ANSWERS answers; how many OUT holds, in how many lines
  "$PY" - "$W/iq.jsonl" "$@" <<'EOF'
import json, os, sys
def tasks(path):
    out = []
    for line in open(path, encoding="utf-8") if os.path.exists(path) else []:
        try:
            out.append(json.loads(line)["task"])
        except (ValueError, KeyError, TypeError):
            pass
    return out
suite = {json.loads(l)["id"] for l in open(sys.argv[1], encoding="utf-8") if l.strip()}
want = set(tasks(sys.argv[2])) & suite
got = [t for t in tasks(sys.argv[3]) if t in want]
print(len(want), len(set(got)), len(got))
EOF
}

team() {  # team NAME ANSWERS SPEC TRIES [FLAGS]: ANSWERS' drafts repaired by the helper SPEC
  local name=$1 answers=$2 sp=$3 tries=$4 flags=${5:-} seed=1 lead w rc try want got lines
  local args=() words=()
  scored "$name" && return 0
  [ -s "$T/$answers" ] || { say "skip: needs $answers"; return 2; }
  paused "team $name" && return 3
  [[ $name =~ -s([0-9]+)$ ]] && seed=${BASH_REMATCH[1]}
  args+=(--seed "$seed")
  IFS=+ read -ra words <<< "$flags"
  for w in "${words[@]}"; do
    # An eval that ignores what it does not know would run the arm without it: not run at all.
    has "$w" || { say "skip: team $name needs eval --$w (degraded)"; return 2; }
    args+=(--"$w")
  done
  [ "$DEGRADED" = 1 ] || args+=(--pin --all-tries --skip-icon --tries-out "$W/tries-$name.jsonl")
  lead=${answers%.jsonl}
  lead=${lead#answers-}
  say "team $name: $answers repaired by $sp, $tries tries${flags:+, $flags}, seed $seed"
  serve "$sp" || return 1
  for try in 1 2; do
    "$T/bin/eval.exe" team --suite "$W/iq.jsonl" --answers "$W/$answers" --helper-url "$URL" \
      --helper "$sp" --name "$lead+$name" --out "$W/team-$name.jsonl" \
      --traces "$W/traces-$name.jsonl" --jobs "$(slots "$(spec "$sp")")" --tries "$tries" \
      "${args[@]}" >> "$LOG" 2>&1
    rc=$?
    # 3: requests failed and their answers were not written (the server gone?): once more.
    if [ "$rc" != 3 ] || [ "$try" = 2 ]; then break; fi
    say "team $name: eval left answers unwritten (exit 3): serving again, once more"
    serve "$sp" || return 1
  done
  [ "$rc" = 0 ] || { say "failed: team $name: eval exited $rc"; return 1; }
  read -r want got lines < <(tasks "$W/$answers" "$W/team-$name.jsonl" | tr -d '\r')
  [ "$want" = "$got" ] || { say "failed: team $name wrote ${got:-?} of ${want:-?} answers"; return 1; }
  [ "$lines" = "$got" ] || say "WARNING: team-$name.jsonl holds $lines lines for its $got tasks"
  score "$name" "team-$name.jsonl"
}

finish() {  # finish NAME SPEC K: GLM's unfinished replies written whole from its notes, K samples
  local name=$1 sp=$2 k=$3 rc try f
  scored "$name" && return 0
  for f in prompts-fin.jsonl suite-wait.jsonl; do [ -s "$T/$f" ] || { say "skip: needs $f"; return 2; }; done
  if [ ! -e "$T/answers-$name-samples.done" ]; then
    paused "finish $name" && return 3
    say "finish $name: $(wc -l < "$T/prompts-fin.jsonl") unfinished replies written whole by $sp, $k samples"
    serve "$sp" || return 1
    for try in 1 2; do
      (cd "$REPO" && "$PY" train/ask.py --prompts "$W/prompts-fin.jsonl" --out "$W/answers-$name-samples.jsonl" \
         --name "$name" --k "$k" --jobs "$(slots "$(spec "$sp")")" --url "$BASE") >> "$LOG" 2>&1
      rc=$?
      [ "$rc" = 0 ] && break
      [ "$try" = 2 ] && { say "failed: finish $name: ask.py exited $rc"; return 1; }
      say "finish $name: ask.py exited $rc: serving again, once more"
      serve "$sp" || return 1
    done
    touch "$T/answers-$name-samples.done"
  fi
  prep pick --samples "$W/answers-$name-samples.jsonl" --suite-wait "$W/suite-wait.jsonl" \
    --iq "$W/bin/iq.exe" --out "$W/answers-$name.jsonl" || return
  score "$name" "answers-$name.jsonl"
}

judge() {  # judge NAME SPEC: train/judge.py on SPEC's one-token yes or no
  local name=$1 sp=$2 f rc try
  [ -s "$T/judgefit-$name.json" ] && return 0
  [ -s "$HERE/judge.py" ] || { say "skip: needs train/judge.py"; return 2; }
  for f in answers-pre.jsonl each-pre.txt judge-eval-glm-s.jsonl judge-eval-alt-s.jsonl; do
    [ -s "$T/$f" ] || { say "skip: needs $f"; return 2; }
  done
  paused "judge $name" && return 3
  say "judge $name: does $sp tell a passing program from a failing one better than its length?"
  serve "$sp" || return 1
  for try in 1 2; do
    # The tries files as a glob: judge.py expands it (none yet is none).
    (cd "$REPO" && "$PY" train/judge.py --url "$BASE" --suite "$W/iq.jsonl" --pre "$W/answers-pre.jsonl" \
       --pre-each "$W/each-pre.txt" --sets "$W/judge-eval-glm-s.jsonl" "$W/judge-eval-alt-s.jsonl" \
       --candidates "$W/tries-*.jsonl" --iq "$W/bin/iq.exe" --out "$W/judge-$name.jsonl" \
       --fit "$W/judgefit-$name.json" --jobs 8) >> "$LOG" 2>&1
    rc=$?
    if [ "$rc" != 3 ] || [ "$try" = 2 ]; then break; fi
    say "judge $name: requests failed (exit 3): serving again, once more"
    serve "$sp" || return 1
  done
  if [ "$rc" != 0 ] || [ ! -s "$T/judgefit-$name.json" ]; then say "failed: judge $name: judge.py exited $rc"; return 1; fi
  say "judge $name: judgefit-$name.json"
}

train() {  # train RUN EPOCHS BATCH ACCUM FILE...: a LoRA of the untuned 3B on FILEs (here), resumed if begun
  local run=$1 epochs=$2 batch=$3 accum=$4 f free data=()
  shift 4
  grep -q '^  "status": "done"' "$ROOT/runs/$run/manifest.json" 2>/dev/null && return 0
  for f in "$@"; do
    [ -s "$T/$f" ] || { say "skip: needs $f"; return 2; }
    data+=(--data "$W/$f")
  done
  paused "train $run" && return 3
  unserve
  gpu_free || return 1
  free=$(free_gb)
  [ "${free:-0}" -ge 20 ] || { say "failed: train $run: ${free:-?} GB free on the disk, 20 needed"; return 1; }
  say "train: $run on $(cd "$T" && cat "$@" | wc -l) records ($*), $epochs epochs, batch $batch x $accum"
  (cd "$REPO" && "$PY" train/sft.py --held "$W/held.txt" --tasks "$W/iq.jsonl" --base "$LARGE" \
     --run "$run" "${data[@]}" --epochs "$epochs" --batch "$batch" --accum "$accum" --save-every 13) >> "$LOG" 2>&1 \
     || { say "failed: $run training (a rerun resumes it)"; return 1; }
}

summary() {  # every score, then the composed headline and the decisions (compose.py), then paired.py
  local f rc=0 args=() v
  say "summary:"
  for f in "$T"/score-*.txt; do
    [ -s "$f" ] || continue
    printf '  %-22s %s\n' "$(basename "$f" .txt | sed 's/^score-//')" \
      "$(grep -m1 ' pass ' "$f" | sed 's/^ *//')" | tee -a "$LOG"
  done
  [ -s "$HERE/compose.py" ] || { say "skip: needs train/compose.py"; return 2; }
  [ -s "$T/suite-wait.jsonl" ] || { say "skip: needs suite-wait.jsonl"; return 2; }
  args=(--dir "$W" --iq "$W/bin/iq.exe" --suite "$W/iq.jsonl" --suite-wait "$W/suite-wait.jsonl" --order "$ORDER"
        --out "$W/answers-final.jsonl" --report "$W/report.md" --json "$W/report.json")
  [ -s "$T/decisions.json" ] && args+=(--decisions "$W/decisions.json")
  (cd "$REPO" && "$PY" train/compose.py "${args[@]}") 2>&1 | tee -a "$LOG"
  [ "${PIPESTATUS[0]}" = 0 ] || { say "failed: summary: compose.py"; rc=1; }
  [ -s "$HERE/paired.py" ] || { say "skip: needs train/paired.py"; return 2; }
  args=(--dir "$W" --iq "$W/bin/iq.exe" --out "$W/paired.md" --json "$W/paired.json")
  for v in pre final; do [ -s "$T/answers-$v.jsonl" ] && args+=(--sys "$v=$W/answers-$v.jsonl"); done
  [ -s "$T/answers-pre.jsonl" ] && args+=(--in-sample pre)
  for v in "${VS[@]}"; do args+=(--vs "$v"); done
  [ -s "$T/predictions.jsonl" ] && args+=(--predictions "$W/predictions.jsonl")
  (cd "$REPO" && "$PY" train/paired.py "${args[@]}") >> "$LOG" 2>&1 || { say "failed: summary: paired.py"; rc=1; }
  [ "$rc" = 0 ] && say "summary: $T/report.md, $T/paired.md"
  return "$rc"
}

# A step: 0 done, 1 failed, 2 skipped (an input not there yet), 3 not begun (PAUSE).
worse() { [ "$ok" = 1 ] || [ "$1" = 0 ] || ok=$1; }  # ok keeps the worst code: 1 over 2 over 0
run_step() {
  local step=$1 name sp tries answers flags files epochs batch accum src f ok rc k prev run
  local fs=() extra=() traces=()
  case $step in
    inputs)
      # Each made once (nightprep.py leaves what exists alone); all are tried, the worst said.
      ok=0
      [ -s "$HERE/nightprep.py" ] || { say "skip: needs train/nightprep.py"; return 2; }
      [ -s "$T/suite-wait.jsonl" ] || prep suite --suite "$W/iq.jsonl" --out "$W/suite-wait.jsonl"
      worse $?
      [ -s "$T/answers-dev.jsonl" ] || prep dev --pool "$PREPW/eng/pool-dev.jsonl" --held "$W/held.txt" \
        --suite "$W/iq.jsonl" --out "$W/answers-dev.jsonl"
      worse $?
      for f in judge-train judge-eval-glm judge-eval-alt; do [ -s "$T/$f-s.jsonl" ] || break; f=; done
      [ -z "$f" ] || prep judge --src "$PREPW" --out "$W" --held "$W/held.txt"
      worse $?
      [ -s "$T/q3rec.jsonl" ] || prep q3rec --src "$(cygpath -m "$Q3REC" 2>/dev/null || echo "$Q3REC")" \
        --out "$W/q3rec.jsonl"
      worse $?
      [ "$ok" = 0 ] && say "inputs: suite-wait, $(wc -l < "$T/answers-dev.jsonl") dev problems, the judge sets, q3rec"
      return "$ok" ;;
    pre)
      [ -s "$T/answers-pre.jsonl" ] || prep pre --src "$PREPW/mx-answers-pre.jsonl" --sha "$PRE_SHA" \
        --out "$W/answers-pre.jsonl" || return
      scored pre || score pre answers-pre.jsonl || return
      [ -s "$T/prompts-fin.jsonl" ] || prep finish --answers "$W/answers-glm.jsonl" --each "$W/each-pre.txt" \
        --prompts "$W/prompts-held.jsonl" --out "$W/prompts-fin.jsonl" || return
      return 0 ;;
    glm)
      scored glm || score glm answers-glm.jsonl ;;
    team:*)
      IFS=: read -r _ name sp tries answers flags <<< "$step"
      team "$name" "${answers:-answers-glm.jsonl}" "$sp" "${tries:-1}" "$flags" ;;
    finish:*)
      IFS=: read -r _ name sp k <<< "$step"
      finish "$name" "$sp" "$k" ;;
    judge:*)
      IFS=: read -r _ name sp <<< "$step"
      judge "$name" "$sp" ;;
    train:*)
      IFS=: read -r _ name files epochs batch accum <<< "$step"
      IFS=+ read -ra fs <<< "$files"
      train "$NIGHT-$name" "${epochs:-${TRAIN_EPOCHS:-1}}" "${batch:-${TRAIN_BATCH:-1}}" \
        "${accum:-${TRAIN_ACCUM:-16}}" "${fs[@]}" ;;
    summary)
      summary ;;
    replicates)
      # The noise of a run: the same arms again under other seeds, until PAUSE.
      for k in 2 3 4 5 6 7 8 9; do
        for f in "team:b3-s$k:base3b:8:answers-pre.jsonl" "team:b7-s$k:base7b:4:answers-pre.jsonl" summary; do
          attempt "$f"
          rc=$?
          [ "$rc" = 3 ] && return 3
        done
      done
      return 0 ;;
    from:*)
      # An earlier night's fix data and drafts, so tonight need not make them again.
      src=$D/team-${step#from:}
      [ -d "$src" ] || { say "skip: needs $src"; return 2; }
      for f in fix.jsonl fix-mutants.jsonl fix-mutants2.jsonl fix-m2.jsonl refs.jsonl \
               drafts-base3b.jsonl drafts-base3b.done traces-train-base3b.jsonl; do
        [ -e "$src/$f" ] && [ ! -e "$T/$f" ] && cp "$src/$f" "$T/$f" && say "from ${step#from:}: $f"
      done
      return 0 ;;
    checks:*)
      name=${step#checks:}
      [ -s "$T/checkfit-$name.json" ] && return 0
      paused "checks $name" && return 3
      if [ ! -s "$T/checks-short.jsonl" ]; then
        (cd "$REPO" && "$PY" train/checkdata.py --iq "$W/bin/iq.exe" --suite "$W/iq.jsonl" \
           --held-prompts "$W/prompts-held.jsonl" --train-out "$W/checks.jsonl" \
           --held-out "$W/prompts-held-checks.jsonl" --day "$(date +%F)") >> "$LOG" 2>&1 \
           || { say "failed: checks $name: checkdata.py"; return 1; }
        # Short checks only: last night's writer ran out of room on most (216 of 246 unclosed).
        "$PY" - "$W/checks.jsonl" "$W/checks-short.jsonl" "${CHECK_LINES:-80}" <<'EOF' | tee -a "$LOG"
import json, sys
src, out, most = sys.argv[1], sys.argv[2], int(sys.argv[3])
rows = [json.loads(l) for l in open(src, encoding="utf-8") if l.strip()]
short = [r for r in rows if r["messages"][-1]["content"].count("\n") <= most + 1]
open(out, "w", encoding="utf-8", newline="\n").writelines(json.dumps(r, ensure_ascii=False) + "\n" for r in short)
print("checks: %d of %d no longer than %d lines" % (len(short), len(rows), most))
EOF
        [ -s "$T/checks-short.jsonl" ] || { say "failed: checks $name: none short enough"; return 1; }
      fi
      train "$NIGHT-$name" "${CHECK_EPOCHS:-2}" 2 8 checks-short.jsonl
      rc=$?
      [ "$rc" = 0 ] || return "$rc"
      if [ ! -e "$T/answers-$name-checks.done" ]; then
        paused "checks $name: writing" && return 3
        serve "$name" || return 1
        (cd "$REPO" && "$PY" train/ask.py --prompts "$W/prompts-held-checks.jsonl" \
           --out "$W/answers-$name-checks.jsonl" --name "$name-checks" --k 1 --jobs 16 --url "$BASE") >> "$LOG" 2>&1
        rc=$?
        [ "$rc" = 0 ] || { say "failed: checks $name: writing ($rc)"; return 1; }
        touch "$T/answers-$name-checks.done"
      fi
      (cd "$REPO" && "$PY" train/select.py --iq "$W/bin/iq.exe" --suite "$W/iq.jsonl" \
         --programs "$W/answers-glm.jsonl" --checks "$W/answers-$name-checks.jsonl" \
         --out "$W/sel-$name.jsonl" --summary "$W/checkfit-$name.json") >> "$LOG" 2>&1 \
         || { say "failed: checks $name: select.py"; return 1; }
      say "checks $name: checkfit-$name.json" ;;
    drafts)
      [ -e "$T/drafts-base3b.done" ] && return 0
      paused drafts && return 3
      say "drafts: the untuned 3B writes each train task once"
      serve base3b || return 1
      (cd "$REPO" && "$PY" train/ask.py --prompts "$W/prompts-train.jsonl" \
         --out "$W/drafts-base3b.jsonl" --name base3b --k 1 --jobs 16 --url "$BASE") >> "$LOG" 2>&1
      rc=$?
      [ "$rc" = 0 ] || { say "failed: drafts ($rc): a rerun resumes them"; return 1; }
      touch "$T/drafts-base3b.done"
      say "drafts: $(wc -l < "$T/drafts-base3b.jsonl")" ;;
    train-traces)
      team train-base3b drafts-base3b.jsonl base3b 1 ;;
    mutants)
      # Fix data at no cost: each train task's reference broken one way GLM breaks programs.
      [ -s "$T/fix-mutants.jsonl" ] && return 0
      cat "$D"/refs-*.jsonl > "$T/refs.jsonl"
      "$T/bin/eval.exe" mutants --suite "$W/iq.jsonl" --refs "$W/refs.jsonl" --held "$W/held.txt" \
        --out "$W/fix-mutants.jsonl" --per "${MUTANTS_PER:-1}" --max "${MUTANTS_MAX:-800}" 2>&1 \
        | tee -a "$LOG"
      [ "${PIPESTATUS[0]}" = 0 ] || { say "failed: mutants"; return 1; } ;;
    fixdata)
      [ -s "$T/fix.jsonl" ] && return 0
      [ -s "$T/traces-train-base3b.jsonl" ] || { say "skip: needs traces-train-base3b.jsonl"; return 2; }
      (cd "$REPO" && "$PY" train/fixdata.py --traces "$W/traces-train-base3b.jsonl" \
         --held "$W/held.txt" --suite "$W/iq.jsonl" --out "$W/fix.jsonl" --night "$NIGHT") \
         >> "$LOG" 2>&1 || { say "failed: fixdata"; return 1; } ;;
    fix3b)
      fs=(fix.jsonl)
      [ -s "$T/fix-mutants.jsonl" ] && fs+=(fix-mutants.jsonl)
      train "$NIGHT-fix3b" "${FIX_EPOCHS:-1}" 2 8 "${fs[@]}" ;;
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
open(out, "w", encoding="utf-8", newline="\n").writelines(left[:n])
print("round prompts: %d of %d left" % (min(n, len(left)), len(left)))
PY
          [ -s "$T/prompts-r$k.jsonl" ] || { say "rounds: every train task drafted"; return 0; }
          serve base3b || return 1
          (cd "$REPO" && "$PY" train/ask.py --prompts "$W/prompts-r$k.jsonl" \
             --out "$W/drafts-r$k.jsonl" --name base3b --k 1 --jobs 16 --url "$BASE") >> "$LOG" 2>&1
          rc=$?
          [ "$rc" = 0 ] || { say "failed: round $k drafts ($rc)"; return 1; }
          touch "$T/drafts-r$k.done"
        fi
        team "train-r$k" "drafts-r$k.jsonl" "$prev" "${ROUND_TRIES:-1}" || return
        if [ ! -s "$T/fix-r$k.jsonl" ]; then
          traces=()
          for f in "$T"/traces-train-*.jsonl; do traces+=(--traces "$(cygpath -m "$f" 2>/dev/null || echo "$f")"); done
          (cd "$REPO" && "$PY" train/fixdata.py "${traces[@]}" --held "$W/held.txt" --suite "$W/iq.jsonl" \
             --out "$W/fix-r$k.jsonl" --night "$NIGHT") >> "$LOG" 2>&1 \
             || { say "failed: round $k fixdata"; return 1; }
        fi
        IFS=+ read -ra extra <<< "${ROUND_EXTRA:-fix-mutants.jsonl}"
        train "$run" "${TRAIN_EPOCHS:-1}" "${TRAIN_BATCH:-1}" "${TRAIN_ACCUM:-16}" "fix-r$k.jsonl" "${extra[@]}" || return
        team "r$k" answers-glm.jsonl "$run" "${ROUND_TRIES:-1}" || return
        k=$((k + 1))
      done
      return 3 ;;
  esac
}

# A step that fails is tried once more at once, with the server stopped first; then the night goes
# on, and the step runs once more after the list, as a skipped one does.
AGAIN=()
attempt() {  # attempt STEP: run_step's code, once its retry is spent
  local s=$1 rc
  say "step $s"
  run_step "$s"
  rc=$?
  if [ "$rc" = 1 ]; then
    paused "step $s again" && return 3
    unserve
    say "step $s failed: retrying once"
    run_step "$s"
    rc=$?
    [ "$rc" = 1 ] && say "step $s failed: going on"
  fi
  if [ "$rc" = 1 ] || [ "$rc" = 2 ]; then
    case " ${AGAIN[*]} " in *" $s "*) ;; *) AGAIN+=("$s") ;; esac
  fi
  return "$rc"
}

# The watchdog: the GPU's load every 3rd wake, and a STALL line when nothing under the night's
# folder and nothing in its log (but the watchdog's own lines) is written for STALL seconds. It
# only logs; it ends when team.sh does, and says so if team.sh died without its EXIT line.
watchdog() {
  local n=0 t=0 now newest last mine
  last=$(date +%s)
  mine=$(wc -c < "$LOG")
  while kill -0 "$MAIN" 2> /dev/null; do
    sleep 5
    t=$((t + 5))
    [ "$t" -lt "$WATCH" ] && continue
    t=0 n=$((n + 1)) now=$(date +%s)
    newest=$(find "$T" -type f -printf '%T@\n' 2> /dev/null | sort -n | tail -1)
    newest=${newest%.*}
    [ "${newest:-0}" -gt "$last" ] && last=$newest
    [ "$(wc -c < "$LOG")" != "$mine" ] && last=$now
    if [ $((n % 3)) = 0 ] && command -v nvidia-smi > /dev/null; then
      say "gpu: $(nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader,nounits 2> /dev/null \
        | head -1 | sed 's/, */% /') MiB"
    fi
    if [ $((now - last)) -ge "$STALL" ]; then
      say "STALL: nothing written for $(((now - last) / 60)) min during $(grep -E \
        '^[0-9-]{10} [0-9:]{8} step [^ ]+( \(once more\))?$' "$LOG" | tail -1 | cut -d' ' -f4)"
      last=$now
    fi
    mine=$(wc -c < "$LOG")
  done
  # team.sh is gone and its EXIT trap never ran (a hard kill): said for it.
  say "failed: team.sh (pid $MAIN) is gone without its EXIT line: the GPU may be idle"
}
MAIN=$$
watchdog &
WATCHER=$!

LEFT=()
again() {  # each failed or skipped step once more; 3 on PAUSE
  local s rc
  [ ${#AGAIN[@]} -gt 0 ] || return 0
  say "once more, the steps that failed or were skipped: ${AGAIN[*]}"
  set -- "${AGAIN[@]}"
  AGAIN=()
  for s in "$@"; do
    paused "$s" && return 3
    say "step $s (once more)"
    run_step "$s"
    rc=$?
    case $rc in
      1) LEFT+=("$s"); say "step $s failed again: left for the morning" ;;
      2) LEFT+=("$s") ;;
      3) return 3 ;;
    esac
  done
  return 0
}
for step in "$@"; do
  paused "$step and what follows" && { REASON=PAUSE; break; }
  # An open-ended step fills the night until PAUSE: what failed or was skipped goes first.
  case $step in replicates|rounds) again || { REASON=PAUSE; break; } ;; esac
  attempt "$step"
  rc=$?
  [ "$rc" = 3 ] && { REASON=PAUSE; break; }
done
if [ "$REASON" != PAUSE ]; then again || REASON=PAUSE; fi
[ ${#LEFT[@]} -gt 0 ] && REASON="$REASON; not done: ${LEFT[*]}"
case $REASON in PAUSE*) exit 0 ;; *"not done"*) exit 1 ;; esac
exit 0
