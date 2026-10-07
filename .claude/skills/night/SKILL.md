---
name: night
description: Launch the overnight loop when compusophy goes to bed (they say so, or type /night, ultracode on). The GPU runs train/night.sh; the night workflow predicts tonight's scores and grows the IQ suite; the morning stops, reports, and ships the data. Also how to pause it and how to smoke-test it by day.
---

# The night

compusophy types `/night` (or says they are going to bed) with the effort mode on ultracode.
That is the only go: never start a night unasked, never schedule one ahead. Two halves run
until morning, and neither waits on the other:

- **The GPU** (`train/night.sh`, detached): baselines, tonight's fine-tunes (0.5B full, 3B LoRA,
  a self-taught round), scored on the held-out tasks, `report.py` at the end. It copies its
  inputs once (`iq/night-<night>/`, GLM's answers among them), so the data side may change
  them meanwhile. From 07:45 it starts no new step. One runs at a time (`iq/night.pid`).
- **The data side** (the workflow `.claude/workflows/night.js`): Predict (tonight's held-out
  rates, written before they are scored), Plan (new families under new roots), then per batch
  Teach (a draft), Attack (an independent agent tries to break each task; only it writes the
  stage file, so nothing unattacked is ever imported) and Solve, then Import (`train/day.sh
  import` and `data`: tomorrow night's inputs). Its agents start nothing after 07:30 or under
  `PAUSE`, but Import, which uses no GPU, always runs to the end.

`D` below is `C:\sept30\computehub-data\iq`; the repo is this session's root.

## Launch

Run these in order; stop and tell compusophy if one fails.

1. **Build the tools** the agents call: `cargo build -q --release -p compusophy-teach -p compusophy-iq`.
2. **Lift the pause.** If `D/PAUSE` exists, rename it (`mv` to `PAUSE.off-<yyyymmddHHMM>`; never
   `rm` an absolute path).
3. **Preflight:** `bash train/night.sh --check` must end "ready": every input present, every task
   the prompts ask in this checkout's suite, the disk over 50 GB, no python or llama process on
   the GPU, no other night.sh running or waiting. Before 22:00 it says the GPU is compusophy's:
   fine, step 4 waits.
4. **Start the GPU half**, detached and waiting for 22:00 if need be. In PowerShell (the command
   quoted, or Start-Process splits it):
   ```powershell
   $repo = (git rev-parse --show-toplevel)
   $cmd = "bash '$repo/train/night.sh' --wait >> '/c/sept30/computehub-data/iq/night.out' 2>&1"
   Start-Process -FilePath 'C:\Program Files\Git\bin\bash.exe' -ArgumentList "-lc `"$cmd`"" -WindowStyle Hidden
   ```
   Then confirm it runs: `D/night.out` grows ("waiting for 22:00", or tonight's first steps),
   and `D/night.pid` holds its pid.
5. **The wave.** Let `k` be the highest wave with a plan, `D/work/w<k>/plan.json`. If that wave
   has no `D/work/w<k>/imported.json`, it did not finish (a pause, a freeze): run `w<k>` again,
   which resumes it. Else run `w<k+1>`. With no plan.json at all (waves 2 and 3 predate them),
   one more than the highest `w<k>` among `D/stage/*.jsonl`: `w4`.
6. **Start the data half**: `Workflow({scriptPath: '<repo>/.claude/workflows/night.js', args})`
   (by path: a session sees a named workflow only if it existed when the session began), in
   the background, with
   - `night`: `n` + the evening's date, as night.sh names it (`n20261006` for the night that
     begins on 2026-10-06; launched after midnight, before noon, yesterday's date);
   - `day`: today's date (`YYYY-MM-DD`), for the tasks' `by.day`;
   - `wave`: step 5's; `repo`: the repo root as a Windows path; `data`: `C:\sept30\computehub-data`.
   The defaults (6 tiers, 2 teachers a tier, 8 families each, 2 tasks a family, loose asks from
   tier 3: about 190 tasks) suit ultracode; fewer if compusophy says so.
7. **The morning:** load `CronCreate` (ToolSearch) and make one job, not a recurring one:
   `CronCreate({cron: "48 7 <day> <month> *", recurring: false, prompt: "Night wrap-up: follow
   the night skill's Morning section."})`, for the coming morning's date.
8. **Tell compusophy**, in a few lines: the night and wave, both halves running (or waiting for
   22:00), when the wrap-up fires, and that "pause" stops everything.

While it runs, a workflow notification is not a reason to wake anyone: read its result, note
what failed, and leave the rest to the morning.

## Morning

At 07:48 (the job), or when compusophy says they are up:

1. **The data side first.** If the workflow still runs, its batches stop starting work at 07:30
   and Import runs to its end: wait for it (it uses no GPU). If the workflow has ended without
   Import (`imported` null, or no `D/work/<wave>/imported.json`), run `bash train/day.sh import`
   and then `bash train/day.sh data` yourself.
2. **The GPU is theirs.** night.sh stops itself after the step running at 07:45: its log ends
   "the GPU is idle". If a step still runs, end only it, so night.sh writes its report: with
   llama-server up, `python train/serve.py --stop` (the scorer's requests fail and night.sh goes
   to its report); with `sft.py` or `generate.py` running, stop that python alone (by its command
   line, below). Wait for "the GPU is idle", ten minutes at most; if it never comes, stop
   everything as for a pause and write the report: `bash train/night.sh --report`. Then touch
   `D/PAUSE`, so nothing restarts by day, and check `nvidia-smi`: no python, no llama.
3. **Read the night:** `D/report-<night>.md` (held-out rates with their intervals by family,
   predictions against what happened), `D/night-<night>.log`, the workflow's batches (staged,
   fixed and dropped by Attack, unattacked ones, asks that misled the solvers), the end of
   `D/day.log`.
4. **Ship the data.** If `evals/suites/iq.jsonl` changed:
   `cargo test -q -p compusophy-iq -p compusophy-teach --no-fail-fast` and `bash scripts/caps.sh`;
   then `git add evals/suites/iq.jsonl` (that file alone) and commit ("The IQ suite grows to N
   tasks: wave wK ..."); `git -C C:/sept30/computehub merge --ff-only <this branch>` and
   `git -C C:/sept30/computehub push origin main`, and push this branch. If a test fails, commit on
   the branch anyway, say so in the morning file, and launch the next night from this branch. The
   suite does not ship in `dist/`: no deploy.
5. **Write `D/morning-<night>.md`:** the wave, what trained and how it scored (with intervals; a
   gain inside the noise is called noise: DESIGN.md, Evolution, "The gate"), the predictions'
   misses, the data grown, what broke, and what to change next.
6. **Remember** the night in the spine memory, in two or three lines.
7. When compusophy asks, summarize from the morning file; never claim a gain the intervals do
   not support.

## Pause ("pause", "I'm gaming")

Everything stops, at once, and stays stopped. In PowerShell:

1. Touch `D/PAUSE`: no new step starts, in night.sh or the workflow's agents.
2. Stop night.sh's whole tree, found by command line (Start-Process's own pid is only a launcher;
   the bash running night.sh is not under it):
   ```powershell
   Get-CimInstance Win32_Process -Filter "Name='bash.exe'" | Where-Object { $_.CommandLine -like '*train/night.sh*' } | ForEach-Object { taskkill /T /F /PID $_.ProcessId }
   ```
3. Stop the GPU's processes: `python train/serve.py --stop`, then every python whose command line
   matches `train[\\/](sft|generate|ask|export|serve)\.py`:
   ```powershell
   Get-CimInstance Win32_Process -Filter "Name='python.exe'" | Where-Object { $_.CommandLine -match 'train[\\/](sft|generate|ask|export|serve)\.py' } | ForEach-Object { taskkill /T /F /PID $_.ProcessId }
   ```
4. Stop the workflow (`TaskStop` on its task).
5. A minute later, check that nothing restarted: `nvidia-smi`, and the two queries above empty.

A paused or morning-stopped night resumes when `/night` runs again before noon the next day
(night.sh names a night by its evening). A `/night` on a later evening begins a new night on
that day's inputs; the old run folders stay in `runs/`. The data side resumes its unfinished
wave on any later `/night` (step 5): each phase skips what is done, and its unattacked batches
wait for their attacker.

## By day: a smoke test

`Workflow({scriptPath: '<repo>/.claude/workflows/night.js', args: {smoke: true, repo, data: <a
scratch data root with an iq/ folder holding stage/ and work/>}})`: one tier, one teacher, one
family, no Predict, no Import, no deadline, nothing in the real data root. No GPU at any hour of
the day.
