---
name: night
description: The overnight loop, from when compusophy goes to bed (they say so, or type /night, ultracode on) until they say they are up. Since 2026-10-08 the GPU runs train/team.sh (the two-models night - GLM's recorded drafts repaired by a local helper, a repair helper trained, scored as a team - no cloud call); the Opus data waves run only if compusophy agreed to their plan and token cost. "I'm up" stops it and ships; "pause" stops everything at once. Also the older solo night (train/night.sh) and how to smoke-test it by day.
---

# The night

compusophy says when the night starts ("going to bed", or `/night`, the effort on ultracode)
and when it ends ("I'm up", "I woke up"). Both are theirs: never start a night unasked, and
never end one on a clock. Between the two, neither the GPU nor the agents sit idle.

## The two-models night (the spine since 2026-10-08: run this one)

compusophy: the local model SUPPORTS the cloud model's loop (repair, checks, selection); score
the combined system. So the GPU half is `train/team.sh` (DESIGN.md, "The team"), not the solo
climb below, and the data waves do NOT run unless compusophy has agreed to the plan and its
token cost (a wave is about 9.5M Opus tokens). `team.sh` uses no cloud call: GLM's drafts are
its recorded answers. First night: n20261008 (`D/team-n20261008/`, `D/team-n20261008.log`).

1. Lift the pause (rename `D/PAUSE`), build this checkout's debug `eval` and `iq`
   (`cargo build -p eval -p compusophy-iq`; team.sh pins them in the night's folder at its first
   launch).
2. Start it detached (as night.sh below, by `Start-Process` with the command quoted), e.g.
   `bash train/team.sh glm team:base3b:base3b drafts mutants train-traces fixdata fix3b
   team:fix3b:<night>-fix3b rounds summary`. Each step is skipped once its output is there, so a
   rerun resumes; none begins while `D/PAUSE` exists.
3. Watch its log by events (`score-`, `failed`, `train:`, `drafts:`), never by the clock.
4. Pause: touch `D/PAUSE`, then stop team.sh's bash and its python (`train/ask.py`, `sft.py`,
   `serve.py --stop`) by command line, as for night.sh below.

What it measured on 2026-10-09 is in DESIGN.md; the solo night below stays for reference.

**The second night (n20261009), from the first's lessons.** Name it at launch
(`TEAM_NIGHT=n20261009`): the night's name flips at noon, and the first night's rounds, still
running at 12:44, began the next night's folder. In order of what each is likely to add:

```
TEAM_NIGHT=n20261009 ROUND_FIRST=n20261009-m2 ROUND_TRIES=4 ROUND_EXTRA=fix-m2.jsonl \
  bash train/team.sh from:n20261008 glm team:base3b-t4:base3b:4 train:m2:fix-m2.jsonl \
  team:m2:m2 team:m2-t4:m2:4 checks:chk team:fix3b-t4:n20261008-fix3b:4 summary rounds summary
```

1. `team:base3b-t4`: up to 4 repairs of each draft, the first that runs clean kept. By day, on
   16 drafts the base 3B could not make clean in one try, 4 tries made 7 clean and 3 passed the
   hidden check (~30 min).
2. `train:m2`: the first night's unfinished helper (GLM's own misses as mutants: functions
   called and never written, locals never declared, lists too short), at batch 1 (it paged at
   batch 2; ~2 h), then scored with 1 and 4 tries.
3. `checks:chk`: the check writer again, on short checks only. Last night's: 216 of 246 never
   closed; of the 30 that did, 2 passed the task's own reference and 16 of 17 passing programs
   were rejected. Selection by self-written checks waits until most read, most are fair to the
   reference, and false rejects are rare (`checkfit-chk.json`).
4. `rounds` until "I'm up": train drafts repaired with 4 tries (more clean turns to learn from
   than last night's 11 of 1,589), trained with the m2 data into the next helper.

**The third night (n20261009): team.sh cannot die silently.** Launch it from the worktree on
branch `night3` (`.claude/worktrees/mesh`), not from main. Before bed:

1. Build there, into its own target (`cargo build -p eval -p compusophy-iq`), and run the gates.
2. Write `D/team-n20261009/predictions.jsonl` and `decisions.json` (the plan's "expected").
3. Preflight: `TEAM_NIGHT=n20261009 bash train/team.sh --check`, a PASS or FAIL a line (PAUSE,
   the lock, port 8081, VRAM under 3,000 MiB, 30 GB of disk, the checkout's `eval features`, every
   input: pre's sha, pool-dev, the judge sets, the q3 samples, the 7B, 3B and 0.5B GGUFs; the
   night's name). All must PASS apart from PAUSE; any FAIL exits 1.
4. At "going to bed", rename `D/PAUSE` (`PAUSE.off-<yyyymmddHHMM>`): a launch with PAUSE there is
   refused ("failed: PAUSE exists at launch; the GPU is idle").
5. Launch it detached, as night.sh below (Start-Process, the command quoted), into `D/team.out`:
   ```
   TEAM_NIGHT=n20261009 bash train/team.sh inputs pre glm \
     team:b3:base3b:8:answers-pre.jsonl team:dev3:base3b:2:answers-dev.jsonl \
     team:b7:base7b:4:answers-pre.jsonl finish:fin7:base7b:4 team:fin7r:base7b:2:answers-fin7.jsonl \
     team:dev7:base7b:1:answers-dev.jsonl judge:j7:base7b \
     team:b3w:base3b:8:answers-pre.jsonl:whole team:dev3w:base3b:2:answers-dev.jsonl:whole judge:j3:base3b \
     team:b05:base05:8:answers-pre.jsonl team:dev05:base05:1:answers-dev.jsonl summary \
     train:j1:judge-train-s.jsonl:2:4:4 judge:j1:j1 summary \
     team:b3raw:base3b:8:answers-glm.jsonl summary checks:chk summary replicates
   ```
   A misspelt step is refused before anything begins ("unknown step"). The first launch copies the
   tools and pins them (`bin/pinned`; `EVAL_REFRESH=1` copies them again) and writes the night's
   name to `D/team-night`, so a later launch and the summary by hand find it without TEAM_NIGHT.
6. A `Monitor` on its log, re-armed when it expires:
   `tail -F D/team-n20261009.log | grep --line-buffered -E 'failed|STALL|PAUSE|the GPU is idle|unknown step|Traceback|WARNING|degraded' | grep --line-buffered -v 'exceeds the available context'`

Reading it:

- **A `failed` line no longer means the script ended.** A failed step is retried once at once
  (the server stopped first), then the night goes on; a step whose input is not there says
  `skip: needs F`; each failed or skipped step runs once more after the list (before `replicates`,
  which fills the night until PAUSE). Only `the GPU is idle: team.sh ended (why)` says it ended;
  every exit writes it but a hard kill (`taskkill /F`).
- **Never relaunch while `D/team.pid`'s process lives** (team.sh refuses: "failed: team.sh
  already runs"); `ended` in it means none runs. A relaunch resumes: a step whose output is there
  is skipped, and a line cut by a kill is ended first ("repaired: ...").
- `STALL: nothing written for 40 min during STEP`: look (the log, `nvidia-smi`, the step's
  process); it only logs, and a 7B step can be slow. `gpu: U% M MiB` comes every 15 minutes.
- `degraded: eval lacks ...`: the pinned eval is older than the plan's; every step runs but the
  whole-program arms (`skip: ... needs eval --whole`), with no try logs and no seeds.
- `failed: VRAM held by others (N MiB)`: another program held the GPU for 10 minutes; nothing
  was served.

Wrap-up at "I'm up": touch `D/PAUSE`; stop the tree by command line (team.sh's bash, `eval.exe`,
the python of `ask`, `sft`, `judge` and `export`, then `python train/serve.py --stop`; as Pause
below says, with `train/team.sh` for `train/night.sh`); then run `bash train/team.sh summary` by
hand: it needs no GPU, so PAUSE does not stop it, and it takes the night's name from
`D/team-night`. Read `report.md` (the composed headline, the curves, each decision's verdict, also
in the log) and `paired.md`, then ship (merge night3, push). A run the wrap-up stopped, or a step
left for the morning, is named under "did not finish" and left out of every table and verdict (a
rule that rests on it says "not decided"): rerun that step, then the summary, for it to count.

## The solo night (before 2026-10-08)

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
   matches `train[\\/](sft|generate|ask|export|serve|judge)\.py`, and (team.sh) every `eval.exe`
   whose command line holds `--helper-url`:
   ```powershell
   Get-CimInstance Win32_Process -Filter "Name='python.exe'" | Where-Object { $_.CommandLine -match 'train[\\/](sft|generate|ask|export|serve|judge)\.py' } | ForEach-Object { taskkill /T /F /PID $_.ProcessId }
   Get-CimInstance Win32_Process -Filter "Name='eval.exe'" | Where-Object { $_.CommandLine -like '*--helper-url*' } | ForEach-Object { taskkill /T /F /PID $_.ProcessId }
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
