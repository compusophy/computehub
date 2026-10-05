#!/usr/bin/env bash
# The night run: train -> export -> serve -> score -> stop, then one line in
# <root>/runs.jsonl. Everything it prints goes to <root>/logs/night-<run>.log.
#
#   train/night.sh --data FILE [--data FILE...] (--held FILE [--tasks FILE] | --held-tasks LIST | --no-held)
#                  [--base ID] [--run NAME] [--quant q8_0] [--root DIR] [--force] [-- more sft.py args]
#
# It refuses to start from 08:00 to 22:00 local (the owner games on this GPU
# by day) unless --force, with under 50 GB free under the data root, or while
# another night run is alive. Rerun after a crash: the same night's run name
# resumes training from its newest checkpoint, and finished stages are skipped.
#
# Environment:
#   SCORE_CMD  run against the served model (e.g. `teach ask` then `iq score`); it sees
#              IQ_URL (http://127.0.0.1:8081/v1), IQ_MODEL, RUN, RUN_DIR, COMPUTEHUB_DATA.
#              Its output is kept in RUN_DIR/score.txt; its last line, if JSON, joins runs.jsonl.
#   DATA_CMD   run first, before training (e.g. `teach export` writing the --data file).
#   COMPUTEHUB_DATA, PYTHON, MIN_FREE_GB (50), DAY_START (8), DAY_END (22), PORT (8081).
set -uo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
PY=${PYTHON:-python}
BASE=Qwen/Qwen2.5-Coder-0.5B-Instruct
QUANT=q8_0
RUN=""
FORCE=0
ROOT_ARG=()
SFT=()
while [ $# -gt 0 ]; do
  case "$1" in
    --data|--held|--tasks|--held-tasks) SFT+=("$1" "$2"); shift 2 ;;
    --no-held) SFT+=("$1"); shift ;;
    --base) BASE=$2; shift 2 ;;
    --run) RUN=$2; shift 2 ;;
    --quant) QUANT=$2; shift 2 ;;
    --root) ROOT_ARG=(--root "$2"); shift 2 ;;
    --force) FORCE=1; shift ;;
    --) shift; SFT+=("$@"); break ;;
    *) echo "night: unknown argument: $1" >&2; exit 2 ;;
  esac
done

ROOT=$("$PY" "$HERE/common.py" root "${ROOT_ARG[@]}") || exit 2
ROOT_SH=$(cygpath -u "$ROOT" 2>/dev/null || echo "$ROOT")
PORT=${PORT:-8081}

hour=$((10#$(date +%H)))
if [ "$FORCE" = 0 ] && [ "$hour" -ge "${DAY_START:-8}" ] && [ "$hour" -lt "${DAY_END:-22}" ]; then
  echo "night: it is $(date +%H:%M); the GPU is the owner's from ${DAY_START:-8}:00 to ${DAY_END:-22}:00 (--force overrides)" >&2
  exit 3
fi
free=$("$PY" -c "import shutil, sys; print(shutil.disk_usage(sys.argv[1]).free // 2**30)" "$ROOT") || exit 2
if [ "$free" -lt "${MIN_FREE_GB:-50}" ]; then
  echo "night: only $free GB free under $ROOT; ${MIN_FREE_GB:-50} needed" >&2
  exit 4
fi
lock="$ROOT_SH/night.pid"
if [ -f "$lock" ]; then
  other=$(cat "$lock")
  if [ -n "$other" ] && [ "$other" != done ] && kill -0 "$other" 2>/dev/null; then
    echo "night: another night run (pid $other) is alive" >&2
    exit 5
  fi
fi
echo $$ > "$lock"

# The night's name: a run started after midnight belongs to the evening before,
# so a rerun after a crash at 03:00 resumes the run begun at 22:00.
if [ -z "$RUN" ]; then
  day=$(date +%F)
  if [ "$hour" -lt 12 ]; then day=$(date -d yesterday +%F); fi
  slug=$(echo "${BASE##*/}" | tr 'A-Z' 'a-z')
  RUN="night-$day-$slug"
fi
RUN_DIR="$ROOT/runs/$RUN"
mkdir -p "$ROOT_SH/logs" "$ROOT_SH/runs/$RUN"
LOG="$ROOT/logs/night-$RUN.log"
exec > >(tee -a "$(cygpath -u "$LOG" 2>/dev/null || echo "$LOG")") 2>&1

STARTED=$(date -Iseconds)
STAGE=start
SCORE_OUT="$RUN_DIR/score.txt"
SCORE_ARG=()  # set once SCORE_CMD runs, so an old score.txt is never reported
finish() {
  code=$?
  trap - EXIT
  "$PY" "$HERE/serve.py" "${ROOT_ARG[@]}" --port "$PORT" --stop || true
  status=ok
  if [ "$code" != 0 ]; then status="failed:$STAGE"; fi
  "$PY" "$HERE/common.py" result --root "$ROOT" --run "$RUN" --status "$status" "${SCORE_ARG[@]}" \
    --log "$LOG" --started "$STARTED" || true
  echo done > "$lock"
  echo "night: $status at $(date -Iseconds); GPU now: $(nvidia-smi --query-gpu=utilization.gpu,memory.used \
    --format=csv,noheader 2>/dev/null || echo unknown)"
  exit "$code"
}
trap finish EXIT
trap 'exit 130' INT TERM

echo "night: $RUN from $STARTED; root $ROOT, $free GB free"
nvidia-smi --query-gpu=name,utilization.gpu,memory.used,memory.total --format=csv,noheader 2>/dev/null || true

STAGE=data
if [ -n "${DATA_CMD:-}" ]; then
  echo "night: DATA_CMD: $DATA_CMD"
  bash -c "$DATA_CMD" || exit 1
fi

STAGE=train
"$PY" "$HERE/sft.py" "${ROOT_ARG[@]}" --run "$RUN" --base "$BASE" "${SFT[@]}" || exit 1

STAGE=export
"$PY" "$HERE/export.py" "${ROOT_ARG[@]}" --run "$RUN" --quant "$QUANT" || exit 1

STAGE=serve
"$PY" "$HERE/serve.py" "${ROOT_ARG[@]}" --run "$RUN" --quant "$QUANT" --port "$PORT" || exit 1
URL="http://127.0.0.1:$PORT/v1"

STAGE=score
if [ -n "${SCORE_CMD:-}" ]; then
  echo "night: SCORE_CMD: $SCORE_CMD"
  SCORE_ARG=(--score "$SCORE_OUT")
  IQ_URL=$URL IQ_MODEL=$RUN RUN=$RUN RUN_DIR=$RUN_DIR COMPUTEHUB_DATA=$ROOT bash -c "$SCORE_CMD" 2>&1 \
    | tee "$(cygpath -u "$SCORE_OUT" 2>/dev/null || echo "$SCORE_OUT")"
  [ "${PIPESTATUS[0]}" = 0 ] || exit 1
else
  echo "night: no SCORE_CMD, so nothing scored"
fi
STAGE=done
