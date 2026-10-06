#!/usr/bin/env bash
# The day's data for the applang model, around the teacher (Claude Code sessions writing tasks
# and solving them; see DESIGN.md, "The applang model"). No GPU.
#
#   bash train/day.sh import   the teachers' tasks (<root>/iq/stage/*.jsonl) into the suite, each
#                              verified by iq; the prompts and the held-out list made again; GLM
#                              5.3 (today's Studio) asked the held-out tasks it has not answered
#   bash train/day.sh data     the solvers' replies (<root>/iq/replies-*.jsonl) and the suite's
#                              references graded (iq's check, and an icon that draws), exported
#                              as <root>/iq/sft.jsonl for train/night.sh
#
# Each stage file is imported once (a .imported marker beside it). Environment: COMPUTEHUB_DATA,
# PYTHON, GLM_GAP (33000 ms between requests: the free endpoint allows 110 an hour).
set -uo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/.." && pwd)
PY=${PYTHON:-python}
ROOT=$("$PY" "$HERE/common.py" root) || exit 2
D=$(cygpath -u "$ROOT" 2>/dev/null || echo "$ROOT")/iq
W=$(cygpath -m "$D" 2>/dev/null || echo "$D")
SUITE=$(cygpath -m "$REPO/evals/suites/iq.jsonl" 2>/dev/null || echo "$REPO/evals/suites/iq.jsonl")
mkdir -p "$D/stage"
LOG="$D/day.log"
say() { echo "[$(date +%H:%M:%S)] $*" | tee -a "$LOG"; }
(cd "$REPO" && cargo build -q --release -p compusophy-teach -p compusophy-iq) || { say "build failed"; exit 6; }
TEACH="$REPO/target/release/teach"
IQ="$REPO/target/release/iq"

import() {
  for f in "$D"/stage/*.jsonl; do
    [ -e "$f" ] && [ ! -e "$f.imported" ] || continue
    say "import: $(basename "$f")"
    "$TEACH" import --from "$(cygpath -m "$f" 2>/dev/null || echo "$f")" --out "$SUITE" 2>&1 | tee -a "$LOG" \
      && touch "$f.imported"
  done
  "$IQ" verify "$SUITE" --stamp "$(date +%F)" 2>&1 | tail -1 | tee -a "$LOG"
  "$IQ" split "$SUITE" 2>&1 | tail -1 | tee -a "$LOG"
  "$TEACH" prompts --suite "$SUITE" --split held --out "$W/prompts-held.jsonl" 2>&1 | tee -a "$LOG"
  "$TEACH" prompts --suite "$SUITE" --split train --out "$W/prompts-train.jsonl" 2>&1 | tee -a "$LOG"
  "$PY" - "$W/prompts-held.jsonl" "$W/held.txt" <<'PY'
import json, sys
fs = sorted({json.loads(l)["family"] for l in open(sys.argv[1], encoding="utf-8") if l.strip()})
open(sys.argv[2], "w", encoding="utf-8").write("".join(f + "\n" for f in fs))
print("held families: %d" % len(fs))
PY
  # GLM answers only the held-out tasks it has not: a suite of those, asked whole.
  "$PY" - "$SUITE" "$W/prompts-held.jsonl" "$W/answers-glm.jsonl" "$W/glm-todo.jsonl" <<'PY'
import json, os, sys
held = {json.loads(l)["task"] for l in open(sys.argv[2], encoding="utf-8") if l.strip()}
done = set()
if os.path.exists(sys.argv[3]):
    done = {json.loads(l)["task"] for l in open(sys.argv[3], encoding="utf-8") if l.strip()}
todo = [l for l in open(sys.argv[1], encoding="utf-8") if l.strip() and json.loads(l)["id"] in held - done]
open(sys.argv[4], "w", encoding="utf-8").write("".join(todo))
print("GLM: %d held-out tasks to answer" % len(todo))
PY
  if [ -s "$D/glm-todo.jsonl" ]; then
    say "GLM 5.3 on the new held-out tasks (free endpoint, paced)"
    "$TEACH" ask --url https://computehub-sigma.vercel.app/api/ai --model zai/glm-5.3 --suite "$W/glm-todo.jsonl" \
      --split all --out "$W/glm-new.jsonl" --gap "${GLM_GAP:-33000}" 2>&1 | tail -2 | tee -a "$LOG"
    cat "$D/glm-new.jsonl" >> "$D/answers-glm.jsonl"
  fi
  say "import done: $(wc -l < "$D/prompts-train.jsonl") train, $(wc -l < "$D/prompts-held.jsonl") held-out tasks"
}

data() {
  "$PY" - "$SUITE" "$W/replies-ref.jsonl" <<'PY'
import json, sys
nl, fence = chr(10), chr(96) * 3
with open(sys.argv[2], "w", encoding="utf-8") as out:
    for line in open(sys.argv[1], encoding="utf-8"):
        t = json.loads(line)
        out.write(json.dumps({"task": t["id"], "reply": fence + "app" + nl + t["ref"].rstrip(nl) + nl + fence}) + nl)
PY
  : > "$D/solutions.jsonl"
  for f in "$D"/replies-*.jsonl; do
    "$TEACH" replies --suite "$SUITE" --replies "$(cygpath -m "$f" 2>/dev/null || echo "$f")" \
      --teacher claude-code/claude-opus-5-5 --out "$W/solutions.jsonl" 2>&1 | sed "s|^|$(basename "$f"): |" | tee -a "$LOG"
  done
  "$TEACH" export --solutions "$W/solutions.jsonl" --suite "$SUITE" --out "$W/sft.jsonl" 2>&1 | tee -a "$LOG"
  say "sft.jsonl: $(wc -l < "$D/sft.jsonl") records"
}

case "${1:-}" in
  import) import ;;
  data) data ;;
  *) echo "usage: train/day.sh import|data" >&2; exit 2 ;;
esac
