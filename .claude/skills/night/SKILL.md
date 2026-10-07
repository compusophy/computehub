---
name: night
description: The overnight loop, from when compusophy goes to bed (they say so, or type /night, ultracode on) until they say they are up. The GPU runs train/night.sh --until-woken; the night workflow's waves keep growing the IQ suite; "I'm up" stops both and ships; "pause" stops everything at once. Also how to smoke-test it by day.
---

# The night

compusophy says when the night starts ("going to bed", or `/night`, the effort on ultracode)
and when it ends ("I'm up", "I woke up"). Both are theirs: never start a night unasked, and
never end one on a clock. Between the two, neither the GPU nor the agents sit idle.

- **The GPU** (`train/night.sh --until-woken`, detached, started at once): baselines, tonight's fine-tunes
  (0.5B full, 3B LoRA, a self-taught round), scored on the held-out tasks; then rounds until
  `PAUSE`: the 3B trained again on the data the waves have grown, scored on the same tasks as
  `q3-r2`, `q3-r3`, ... It copies its inputs once (`iq/night-<night>/`), so the waves may change
  them meanwhile. One runs at a time (`iq/night.pid`).
- **The data side** (the workflow `.claude/workflows/night.js`), one wave after another: Predict
  (tonight's held-out rates, written before they are scored), Plan (new families under new
  roots), then per batch Teach (a draft), Attack (an independent agent tries to break each
  task; only it writes the stage file, so nothing unattacked is ever imported) and Solve, then
  Import (`train/day.sh import` and `data`), which grows the data the GPU's next round trains on.
  Agents stop only on `PAUSE`; Import, which uses no GPU, always runs to its end.

`D` below is `C:\sept30\computehub-data\iq`; the repo is this session's root.

## Launch ("going to bed", or /night)

Run these in order; stop and tell compusophy if one fails.

1. **Build the tools** the agents call: `cargo build -q --release -p compusophy-teach -p compusophy-iq`.
2. **Lift the pause.** If `D/PAUSE` exists, rename it (`mv` to `PAUSE.off-<yyyymmddHHMM>`; never
   `rm` an absolute path).
3. **Preflight:** `bash train/night.sh --check` (every input present, every task the prompts ask
   in this checkout's suite, the disk over 50 GB, no python or llama process on the GPU, no other
   night.sh). Its line about the hour is for runs by hand: compusophy's word starts this one.
4. **Start the GPU half** now, detached, running until `PAUSE`. In PowerShell (the command quoted,
   or Start-Process splits it):
   ```powershell
   $repo = (git rev-parse --show-toplevel)
   $cmd = "bash '$repo/train/night.sh' --until-woken >> '/c/sept30/computehub-data/iq/night.out' 2>&1"
   Start-Process -FilePath 'C:\Program Files\Git\bin\bash.exe' -ArgumentList "-lc `"$cmd`"" -WindowStyle Hidden
   ```
   Then confirm it runs: `D/night.out` grows, and `D/night.pid` holds its pid.
5. **The first wave.** Let `k` be the highest wave with a plan, `D/work/w<k>/plan.json`. If it has
   no `D/work/w<k>/imported.json`, it did not finish: run `w<k>` again, which resumes it. Else run
   `w<k+1>`.
6. **Start it**: `Workflow({scriptPath: '<repo>/.claude/workflows/night.js', args})` (by path: a
   session sees a named workflow only if it existed when the session began), in the background,
   with `night` (`n` + the evening's date, as night.sh names it; after midnight and before noon,
   yesterday's), `day` (today, `YYYY-MM-DD`), `wave`, `repo` (a Windows path) and `data`
   (`C:\sept30\computehub-data`). The defaults (6 tiers, 2 teachers a tier, 8 families each, 2
   tasks a family, loose asks from tier 3: about 190 tasks a wave) suit ultracode.
7. **Watch by events, never by the clock.** A `Monitor` (ToolSearch) on night.sh's log, emitting
   its failures and its end (`tail -F D/night-<night>.log | grep --line-buffered -E "failed|error|Traceback|refused|the GPU is idle"`),
   re-armed when it expires; the waves report themselves when they finish. No timed job.
8. **Tell compusophy**, in a few lines: the night and wave, both halves running, and that "I'm up"
   ends the night and "pause" stops everything.

## While they sleep

When a wave's workflow finishes and `D/PAUSE` does not exist, start the next wave at once
(`w<k+1>`, the same night), so the data never stops growing; read the finished one's result and
note what failed (a batch the attacker never finished, a solver that did not run) in
`D/heartbeat-<night>.log`. Never improvise GPU work outside night.sh: the rounds are its own.

## When an event comes (the monitor, a wave's end)

Check night.sh (its log, whether its bash still runs, `nvidia-smi`), the running wave, and the
disk. Fix what is broken as this skill and `train/README.md` say (resume, never GPU work by hand).
If no wave runs and there is no `PAUSE`, start the next. Append one line of what was seen and done
to `D/heartbeat-<night>.log`.

## Wrap-up ("I'm up", "I woke up")

1. **Stop the night.** Touch `D/PAUSE` (no new step or task starts) and stop the monitor
   (`TaskStop`).
2. **The GPU is theirs now.** End only the running step, so night.sh writes its report: with
   llama-server up, `python train/serve.py --stop`; with `sft.py` or `generate.py` running, stop
   that python alone, by its command line (below; a round's training keeps its last half-epoch
   checkpoint). Wait for "the GPU is idle" in `D/night-<night>.log`, a few minutes at most; if it
   never comes, stop everything as for a pause and run `bash train/night.sh --report`. Check
   `nvidia-smi`: no python, no llama.
3. **The data side.** A wave's agents stop at their next task; an Import already running finishes
   (it uses no GPU): let it. If stage files wait unimported, run `bash train/day.sh import` and
   then `bash train/day.sh data` in the background (GLM's pacing makes it slow, and it is light),
   and mark the finished waves imported (`D/work/w<k>/imported.json`).
4. **Read the night:** `D/report-<night>.md` (rates with their intervals by family, predictions
   against what happened, the rounds beside the first 3B on the same tasks), the night log, the
   waves' results, `D/day.log`.
5. **Ship the data.** If `evals/suites/iq.jsonl` changed: `cargo test -q -p compusophy-iq -p
   compusophy-teach --no-fail-fast` (at below-normal priority by day) and `bash scripts/caps.sh`;
   `git add evals/suites/iq.jsonl` alone and commit; `git -C C:/sept30/computehub merge --ff-only
   <this branch>`, `git -C C:/sept30/computehub push origin main`, and push this branch. If a test
   fails, commit on the branch anyway and say so. The suite does not ship in `dist/`: no deploy.
6. **Write `D/morning-<night>.md`** (what trained and how it scored, with intervals: a gain inside
   the noise is called noise; the predictions' misses; the data grown; what broke; what next),
   have it checked against the data, **remember** the night in the spine memory, and tell
   compusophy.

## Pause ("pause", "I'm gaming")

Everything stops, at once, and stays stopped. In PowerShell:

1. Touch `D/PAUSE`.
2. Stop night.sh's whole tree, found by command line (Start-Process's own pid is only a launcher):
   ```powershell
   Get-CimInstance Win32_Process -Filter "Name='bash.exe'" | Where-Object { $_.CommandLine -like '*train/night.sh*' } | ForEach-Object { taskkill /T /F /PID $_.ProcessId }
   ```
3. Stop the GPU's processes: `python train/serve.py --stop`, then every python whose command line
   matches `train[\\/](sft|generate|ask|export|serve)\.py`:
   ```powershell
   Get-CimInstance Win32_Process -Filter "Name='python.exe'" | Where-Object { $_.CommandLine -match 'train[\\/](sft|generate|ask|export|serve)\.py' } | ForEach-Object { taskkill /T /F /PID $_.ProcessId }
   ```
4. Stop the workflow and the monitor (`TaskStop` on each).
5. A minute later, check that nothing restarted: `nvidia-smi`, and the two queries above empty.

A paused night resumes when night.sh runs again before noon the next day (it names a night by its
evening); a later `/night` begins a new night on that day's inputs. The data side resumes an
unfinished wave on any later `/night` (Launch, step 5).

## By day: smoke tests

- The GPU half's control flow, no model and no GPU: `NIGHT_DRY=1 ROUND_MIN=5 ROUND_WAIT=3 bash
  train/night.sh --until-woken --force --root <a scratch data root>` (made-up answers and runs;
  refused without `--root`).
- The data side: `Workflow({scriptPath: '<repo>/.claude/workflows/night.js', args: {smoke: true,
  repo, data: <a scratch data root with an iq/ folder holding stage/ and work/>}})`: one tier,
  one teacher, one family, no Predict, no Import.
