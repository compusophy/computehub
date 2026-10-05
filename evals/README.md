# compusophyOS evals

Fixed, versioned suites with automatic, deterministic checkers, so that a change to a model, a
prompt, the harness or the language shows as a measured gain or loss, not an impression.

| where | what |
|---|---|
| `programs/makes` | Suite 1, Studio makes: the tasks, their checkers, the probe that drives made apps, a reference program for each task (`refs/`) |
| `programs/evals` | the harness: a task's make driven over a wire, its record, its AI exchanges, replays, run comparisons |
| `tools/eval` | the dev-only runner (never shipped): the live wire (`curl` to the free AI), files, summaries |
| `evals/suites/studio.jsonl` | the suite as it is: its hash, then each task's id, size and ask (a test keeps it current) |
| `evals/results/studio.jsonl` | every run's records, a line per task and trial, appended |
| `evals/replays/studio/<run>.jsonl` | every AI exchange of a run, so it replays offline |

## Running

From the repository root, in a debug build as below (a checker that panics fails its app only
where panics unwind; the release profile aborts, so the runner refuses to run there). `--dir D`
reads and writes `D/results` and `D/replays` instead of `evals/`, made if missing:

```sh
# A live run: each task made by the model, graded, recorded. At most --max requests in all;
# it stops early when the AI fails twice running or is out of credit for the day.
cargo run -p eval -- run --model zai/glm-5.3 --trials 1 --max 150
# Another trial of the same run id, or the tasks a stopped run did not reach.
cargo run -p eval -- run --model zai/glm-5.3 --first 2 --run 2026-10-05-glm-5.3
cargo run -p eval -- run --model zai/glm-5.3 --run 2026-10-05-glm-5.3 --resume
# Offline and free: replay a recorded run and say, task by task, whether it grades the same.
cargo run -p eval -- replay --run 2026-10-05-glm-5.3
# After a change to a checker or to applang: grade the same transcripts again and rewrite that
# run's records in place. Written only when exact: every request as recorded, under the prompt
# and knobs the run was made with; else it says why not, and the change needs a live run.
cargo run -p eval -- replay --run 2026-10-05-glm-5.3 --write
# Compare two runs, or two models (all of each one's runs): the latest two by default.
cargo run -p eval -- summary --a zai/glm-5.3-flash --b zai/glm-5.3
cargo run -p eval -- summary --markdown        # the table below
cargo run -p eval -- list > evals/suites/studio.jsonl
```

The runner POSTs to `https://computehub-sigma.vercel.app/api/ai` as the desktop does: the
page's own origin, a streamed chat-completions body (the coder's, unchanged), one of the models
the endpoint allows (`zai/glm-5.3`, `zai/glm-5.3-flash`; it would answer any other name with
GLM 5.3, so the runner refuses one). It paces itself under the endpoint's limits (30 requests a
minute and 120 an hour per client: it keeps to 25 and 110, 2 s apart at least), counting this
machine's requests of the last hour across runs (their times are kept in the system's temp
directory, `compusophy-eval-sent`). The endpoint also keeps a day's spend per instance, half of
it for one client, so a long run of the larger model can meet `402` (E0902): the run stops, and
`--resume` later finishes it.

## Suite 1, Studio makes (`studio`)

24 apps described precisely enough to check, from tiny to hard. Each is made as Studio makes it
(`coder::Make` with Studio's knobs: 5 requests, $0.08 and 150 s at most) and graded as it was
installed, in stages; the first that fails is the record's `stage`:

1. `ai`: the AI failed (busy, out of credit, unreachable): not the model's fault, so it is
   counted apart and never as a fail.
2. `make`: the make installed nothing (it never compiled, or the model said applang cannot).
3. `compile`, `smoke`: it does not compile, or faults in the coder's smoke test on seeds 1 to 3
   (rendered, clicked, ticked, keyed, tapped, typed into, closed and opened again).
   `harness`: the smoke test's own fault (`E0213`, an event for something no longer shown; see
   the baseline's last note), neither a pass nor a fail: counted apart, like `ai`.
4. `icon`: it has no icon line, or one the desktop cannot draw.
5. `check`: its checker drives it headlessly through applang and reads what it shows.

The checkers press buttons by their labels (case aside; `Start` finds `Start game`), type into
inputs by their place, tap board squares and canvas units, press keys, and let time pass in ticks
of the app's own timer. They read labels and a canvas's texts (never button labels or what an
input holds), the numbers and times in them, the color drawn at a point, and how a board's
square looks. A board is found as a grid widget, as pixels of its shape, as a lattice of equal
rects or circles, or else as the whole canvas, so every design a description allows passes. A
description says what its checker looks for, and the checker accepts the alternatives it names
(a button or a key, buttons or swatches). After a roll, a drop or a slide a checker lets 3 s
pass, so an animation settles (time passes only while the app runs a timer), and where the
description asks for no Start of its own, a Start the app shows anyway is pressed first.

| task | size | what its checker does |
|---|---|---|
| counter | tiny | 0 at first; + three times shows 3; - then shows 2 |
| greeter | tiny | typing Ada shows Hello, Ada; then Grace replaces it |
| temperature | small | 100, -40, 37, 0 show 212, -40, 98, 32 |
| tip | small | 200 at 15% shows 30 and 230; at 20% 40 and 240; 99 at 10% 9 and 108 |
| dice | small | 36 rolls on 3 seeds: the circles drawn equal the number shown, 1 to 6, at least 4 faces seen |
| traffic | small | lit light at 0.5 to 7.5 s: green, green, green, yellow, red, red, green; Pause holds it; pressed again it goes on |
| stopwatch | small | 0.0; 2.5 s after Start 2.5; Stop holds; Start goes on to 3.5; Reset 0.0 |
| rps | small | 30 picks on 3 seeds: the outcome said matches Computer: and its pick; the computer varies |
| guess | small | halving guesses hear Too high / Too low and Correct within 8, on 3 seeds with secrets that differ |
| poll | small | 3 dogs, 1 cat: counts written, the dog bar three times the cat's; 5 birds make theirs 5/3 the dogs' |
| tictactoe | medium | X on 0, 4, 8 says X wins; New game empties the board; a full board with no line says Draw |
| connect4 | medium | a disc falls to the bottom; four red in a column say Red wins; four yellow, Yellow wins |
| pomodoro | medium | 25:00; 24:00 a minute after Start; Pause holds; at 25 minutes Break and 5:00; then 4:00; Reset 25:00 |
| clock | medium | 12:00:00 with three hands up from the face's center; 12:00:15 with one pointing right; 12:01:00; at 3:00:00 one points right |
| calculator | medium | 12+7= 19, 9*8= 72, 7-9= -2, 7/2= 3; C clears; 8/0= Error |
| paint | medium | Blue, Red, Yellow (buttons, or swatches off the board) paint the tapped squares in their colors; Clear empties them |
| drawpad | medium | a tap inks (9) one square; with Mirror its mirror too; Mirror again turns it off; Clear wipes |
| whack | medium | Start; in 12 waits of 800 ms the one odd square, tapped, raises Score: by one (5 times at least); 30 s on, Game over |
| bounce | medium | the circle stays on the canvas for 10 s and turns back across and down; Faster moves it farther |
| life | hard | taps toggle squares; a blinker steps right; lone squares die; a glider moves (1, 1) in 4 steps; Play runs, Pause stops |
| minesweeper | hard | first taps never a mine (38 seeds); exactly 10 squares end the game; Flag marks rather than reveals; safe squares tapped in turn say You win, and only once none is hidden |
| memory | hard | each card turned alone shows a face found on exactly one other; differing cards turn back; all pairs say You win and Moves: 8 |
| 2048 | hard | two tiles of 2 or 4; every move that changes the board adds exactly 2 or 4 to the sum; 64 reached (or Game over) in 300 moves |
| tetris | hard | Start shows a piece that falls; Left and Right move it to the edges; Rotate turns one of four pieces; Drop lands it on the bottom row; Game over within 80 drops |

Each checker is shown to be passable, fair and discerning: `refs/` holds a program for each task
that passes it (`makes`' tests); `refs/alt/` nine other designs that pass too (boards on grid
widgets, cards turned back by a timer, a count drawn on a canvas, and four the models made: a
pick written apart from its heading, a clock with a Start, a die that tumbles, a score written
inside a board's square); minesweeper's two designs pass with their mines laid six other ways;
and 46 one-line breaks of the references each fail at the stage expected. The live runs found
five checks unfair (a tumbling die read mid-tumble, a pick apart from its heading, an unasked
Start, a score inside a square, and an unclear reason), and a review a sixth (minesweeper's win
was read only from the sweep's last tap, though a flood fill usually wins sooner, so most
correct games failed); each was fixed and the runs graded again from their transcripts. What
the suite does not check: line clears in tetris, the direction tiles slide in 2048,
minesweeper's numbers, scores beyond whack-a-mole's, and how anything looks beyond what the
checks read. A checker lets at most 250,000 ticks of an app's timer pass in one wait; an app
whose timer is too fast for a wait (a clock that ticks every 16 ms, waited 3 hours) fails with
that reason, never read early. The references are the suite's answer keys, so `lab`'s corpus
never takes them (it skips `programs/makes` and `programs/evals`): a model lab trains may be
measured on them, never trained on them.

Two hashes say what graded a record. The suite's (`suite_hash`) is FNV-1a 64 of its id and the
source of its tasks, checkers and probe, line ends aside: a change to what is asked or how the
checkers drive and read an app is a different suite, and comparing records across suite hashes
compares different measurements (the summary warns). The harness's (`harness`) covers the code
between the AI and a record: every file of the crates `evals` stands on but the suites (the
coder, applang, its syntax and runtime, the icon reader, the wire's types, `evals` itself;
tests aside), read by `programs/evals/build.rs`. Comparing harnesses is what the evals are for.

## Records

A line of `evals/results/studio.jsonl` per task and trial:

| field | what |
|---|---|
| `v` | the record's format: 2 (1 had no `harness`) |
| `suite`, `suite_hash` | the suite and its content hash |
| `run`, `date`, `commit`, `model` | the run's id, its date (given, never read from a clock in the library), the commit its AI was asked at (a record graded again keeps it), the model |
| `prompt`, `knobs` | FNV-1a 64 of the coder's system prompt (as its own test pins it) and of its knobs |
| `harness` | the harness's hash when the record was made or last graded again |
| `task`, `size`, `trial` | which task, how big, which trial |
| `pass`, `stage`, `reason` | the grade, the first stage that failed (or `ok`), and the checker's or the make's reason |
| `outcome`, `tries` | how the make ended (`ready`, `faulting`, `broken`, `cant`, `stopped`, `failed`), its requests |
| `in`, `out`, `usd_micros`, `est` | tokens in and out and micro-dollars, from each request's receipt (`: receipt in= out= microusd=`, the stream's last line); `est` counts requests with no receipt, whose figures are the coder's own reckoning |
| `fallback` | requests the other model answered: the free AI answers a GLM 5.3 request with Flash when its provider is busy, and the stream may not say so, but the receipt's cost does (each model's prices bound it, and Flash's output costs a ninth) |
| `ms` | the make's time: the AI's, as the make's clock counts it (pacing and checks take none) |
| `lines` | the installed program's lines |

A resumed task appends a new line; readers keep the last line of each run's task and trial.

## Replays

Each exchange keeps the request's hash and what its response did to the make: the reply's text,
its thinking's length, its finish reason, the AI's error and usage, the lines that were not
events, the last partial line, when the make last heard from it, when it ended (or that the make
stopped it there, its program in) and its receipt. Fed back as one chunk at that time, it leaves
the make exactly as the stream did (the make decides only after each chunk, from what has come and
the time), so a run replays offline, deterministically and for free. A replay that meets a
request it did not record (the coder changed) says so; `--loose` serves exchanges by their place
instead, to see roughly how a changed harness runs on old answers.

A replay is a digest, not the stream (whole streams, mostly thinking, would be tens of
megabytes for one baseline): it keeps what this coder reads, exactly, and drops the thinking's
text (its length is kept), the chunks' times and what came after the program. So it is exact
for the coder as it reads streams now, and a `--loose` replay of a harness that reads
differently (its time limits, its runaway guard, its thinking) is an approximation, never
written to the records. Suite 3's agent, which calls tools, will need its exchanges kept whole.

`evals` has a test (`recorded_runs_replay_to_their_records`) that replays every recorded run,
and checks that every run with records has its exchanges kept:

- When every request is one recorded, the records are exactly what the code gives now, so they
  must be the ones kept, field for field (the `harness` hash aside: a change to the coder or
  applang that grades every make the same leaves them standing). When a checker, the suite or
  the language changed a grade, the test fails until the run is graded again offline
  (`replay --run <run> --write`), so the change shows in the records as a gain or a loss.
- When a request is not one recorded, a run made by this very harness (its prompt, knobs and
  harness hashes) fails the test: its recording is damaged, or the harness does not replay. A
  run made by another harness is stale, said so, until a live run replaces it.

The runner reads a stream to its end after the make has its program (Studio stops it there),
for the receipt: so a recorded make cost a little more than Studio's would, by what the model
wrote after its program.

## How to read a gain

- **Pass rate.** Passes over makes graded (the `ai` and `harness` stages aside, the summary's
  `errors`), with a 95% Wilson interval. 24 tasks is a coarse ruler: at 60% the interval is
  about 40% to 77%, so one run tells only large differences apart.
- **Trials are not independent.** A task one trial fails, the next often fails too, so k trials
  of 24 tasks are not 24k draws. The interval takes the rate's variance clustered by task and is
  Wilson's at the number of independent draws that amounts to (`n eff`): 24k when trials vary
  as chance would, as few as 24 when each task passes all its trials or none. GLM 5.3's 48
  records below are worth 41. More trials narrow the interval only as far as they tell more,
  and never past what the 24 tasks allow.
- **Two runs, same tasks.** The summary gives the difference with its 95% Newcombe interval
  (each side at its `n eff`; it treats the sides as independent, which on the same tasks makes
  it the cautious one) and, task by task (a task passes on a side when most of its trials did),
  the tasks that flipped with McNemar's exact p: the paired test, and the sharper one. 6 flips
  all one way is p = 0.03; 5 against 1 is p = 0.22. Call it a gain when the interval of the
  difference excludes 0, or the flips are one-sided with p < 0.05; else run more trials
  (`--first 2`), since a model samples (writes at temperature 0.3) and one trial is one draw.
- **Same measurement.** The summary warns when a side mixes suites, prompts, knobs or harnesses,
  or when the sides were graded by different suites: grade the older run again first
  (`replay --write`). Sides that differ in prompt, knobs or harness are what is being compared:
  it names the hashes.
- **Cost.** Tokens and dollars per pass, and seconds per make: a model or prompt that passes as
  often for less is a gain too.
- **What changed.** A change to a checker or to applang is measured offline on the same
  transcripts (`replay`), free and exact while every request replays; a change to the prompt,
  the knobs, the coder's loop or the model needs live runs, compared on the same suite hash.

## Baseline

2026-10-05: Suite 1 at hash `ca4f2f7a761e192a`, the coder's prompt at `0d7cd8ab731dfcab`,
Studio's knobs, the harness at `81d703122e0435d3`. GLM 5.3 ran two trials (the second at a later
commit, the coder the same), Flash one: 124 requests and $0.74 in all. The runs were graded again
from their transcripts after the checkers' fixes, so every record carries the hashes above; and
again, every grade the same, when the hash moved from `81ab7e04340b535a` as the harness's build
script came to forbid unsafe code. The prompt has since moved to `09141db7e75a4b45` (a grid's cell
is the square's index, never its color, shown by a palette), so no request replays as recorded:
these runs are stale (kept as they were, never graded again) until live runs at it replace them.

| run | model | commit | pass | rate (95% CI) | tiny / small / medium / hard | tokens/pass | $/pass | s/make | errors | other model |
|---|---|---|---|---|---|---|---|---|---|---|
| 2026-10-05-glm-5.3 | zai/glm-5.3 | 3ae876c, 3341bf8 | 37/48 | 77% (62%-87%) | 4/4 / 13/16 / 17/18 / 3/10 | 15109 | 0.0182 | 43 | 0 | 14/79 |
| 2026-10-05-glm-5.3-flash | zai/glm-5.3-flash | 63f293b | 20/23 | 87% (68%-95%) | 2/2 / 8/8 / 9/9 / 1/4 | 15620 | 0.0035 | 33 | 1 | 0/45 |

```text
                                pass  rate     95% CI n eff  tok/pass   $/pass s/make errors
zai/glm-5.3-flash             20/23    87%   68%-95%     23     15620   0.0035     33      1
zai/glm-5.3                   37/48    77%   62%-87%     41     15109   0.0182     43      0
zai/glm-5.3 - zai/glm-5.3-flash: -10 points, 95% CI -27 to +12; 0 tasks flipped, McNemar p = 1.00; 7 passed half their trials on a side
zai/glm-5.3: 14 of 79 requests were answered by the other model; the tasks whose every request it answered itself pass 29/36
passed by zai/glm-5.3-flash only: none
passed by zai/glm-5.3 only: none
```

Each make's stage and requests (`*`: the other model answered at least one request):

| task | size | GLM 5.3, trial 1 | GLM 5.3, trial 2 | Flash |
|---|---|---|---|---|
| counter | tiny | pass (1) | pass (1) | pass (1) |
| greeter | tiny | pass (1) | pass (1) | pass (1) |
| temperature | small | pass (2) | pass (1) | pass (1) |
| tip | small | pass (1) | pass (1) | pass (1) |
| dice | small | check (1) | pass (1) | pass (1) |
| traffic | small | pass (1)* | check (1) | pass (1) |
| stopwatch | small | pass (1)* | pass (1) | pass (1) |
| rps | small | pass (2) | pass (1) | pass (3) |
| guess | small | check (2)* | pass (1) | pass (2) |
| poll | small | pass (2)* | pass (1) | pass (1) |
| tictactoe | medium | pass (1) | pass (3)* | pass (3) |
| connect4 | medium | pass (1) | check (1)* | pass (3) |
| pomodoro | medium | pass (1) | pass (2)* | pass (1) |
| clock | medium | pass (1) | pass (1) | pass (1) |
| calculator | medium | pass (1)* | pass (1) | pass (1) |
| paint | medium | pass (2)* | pass (2) | pass (5) |
| drawpad | medium | pass (1)* | pass (3) | pass (1) |
| whack | medium | pass (1) | pass (1) | pass (1) |
| bounce | medium | pass (1) | pass (2) | pass (1) |
| life | hard | check (2) | pass (3) | pass (1) |
| minesweeper | hard | check (2) | make (3) | check (2) |
| memory | hard | pass (4) | check (1) | check (2) |
| 2048 | hard | smoke (4) | check (3)* | harness (5) |
| tetris | hard | pass (2) | smoke (5)* | smoke (5) |

What it says:

- **No measured difference between the models.** The difference is -10 points with an interval
  from -27 to +12, and no task flipped (7 passed half their trials on one side). Flash costs a
  fifth as much per pass and makes an app in three quarters of the time. To tell them apart this
  suite needs more tasks, and harder ones: more trials narrow the interval only as far as its 24
  tasks allow.
- **The room is in the hard tasks**: 3 of 10 and 1 of 4. Tiny to medium tasks pass 53 of 57.
- **The fails are real bugs**, read from the transcripts: a die whose face 2 draws one pip, a
  guess game that picks its secret only on New game (shown only after a win), life's Play with
  no `every`, minesweeper that never sets its win or can mine the first tap, memory that never
  deals or shows card values always, a traffic light that lights green as yellow, connect four
  whose `for r in 5..-1` never runs, 2048 packing tiles into decimal digits, tetris clearing its
  board to length 0.
- **GLM 5.3 is not always GLM 5.3**: the free AI answered 14 of its 79 requests with Flash (its
  provider busy), so its numbers are partly Flash's.
- **A harness bug**: applang's smoke test, within a tick, picks its key, tap and click from the
  render before any of them, so a key that ends a game leaves the click on a button no longer
  shown (E0213, "nothing shown has id 2"). Flash's 2048 met it, and the coder sent it to the
  model as the program's fault, five times; it is graded `harness`, counted apart, since the
  smoke test never finished. It is not rare in the hard games: the tetris reference itself meets
  it for about 1 piece order in 6 (4 of 24 shifts of its draws). So a hard game's make can be
  spent on a fault that is not its own, and the hard tasks' rates are the harness's as well as
  the model's until the smoke test is fixed (in `programs/applang/src/smoke.rs`: each event of a
  tick taken from the render after the one before). That fix changes what the coder checks:
  after it, a recorded run whose requests all still replay is graded again offline (`replay
  --write`, which the test asks for), and one whose requests change needs a live run.

## Next suites (designed, not built)

### Suite 2, the Assistant's desktop tasks

The Assistant uses the desktop for the person; grade it by the state it leaves, not its words.

- **Harness.** A headless desktop as `programs/assistant`'s tests build one: `host::Host` with
  the wm, a VFS home, the overlay, and the system program in process (its frames drawn by
  `uiview`, so `host::agent` reads the scene the model sees), its clock injected. The
  `assistant::Agent` is sans-IO: its model requests go over the same `Wire` (so records and
  replays are the same kind), its acts to the host as the overlay's. A program may not depend on
  `host`, so the tasks and graders live in a program crate behind a `Desktop` trait (state in,
  acts out) and `tools/eval` implements it with the real host.
- **Tasks**, each a starting desktop (a fixture: windows, files, settings), a prompt and a
  grader over the end state: settings (turn error reports off, switch to Mono Light, turn the
  living grain off), windows (open Settings and a Terminal side by side; close or minimize
  them all), home (move Studio into Productivity, add it to the dock), files (write
  `~/notes/todo.txt` with three given lines; rename a file; answer which note mentions a word),
  apps (open a made calculator and report 12 * 7; start a made game), questions (which theme is
  on, how many apps the Games folder holds), and safety (asked to send feedback, it must ask,
  and on a no send nothing; it never writes outside the home).
- **Grades.** Pass when the end state matches (theme, prefs, wm windows and rects, dock and home
  order, VFS contents, the answer's words) and nothing forbidden happened; also steps, failed
  acts, tokens, cost and time. Deterministic: the host's clock, the wm and the VFS replay.

### Suite 3, the Terminal agent's coding tasks

The Terminal's agent has landed (`programs/agent`, `agent` in a Terminal):

- **Harness.** The kernel (deterministic) with a project fixture in the VFS, driving
  `agent::Agent` through a `World` over them: an `sh::Sys` (files, and `run` for a program) and
  ask, heard, say, confirm and ran; its tests' fake world is the shape. The model requests go
  over the same `Wire`. A shell line runs as in the Terminal: the shell's own commands in
  process (so `cd` stays), each program as a kernel job (the Terminal's agent writes it to
  `/dev/job`; the toolbox). `confirm` answers as the task says (yes, always, or no, to grade
  what it does when refused).
- **Tasks.** Shell work graded by hidden checks the agent never sees: a script that counts the
  lines of every `.txt` under `~/notes`; rename every `.TXT` to `.txt`; fix `~/apps/broken.app`
  so it compiles and its + adds one (graded by a `makes` checker); write a `.app` from a spec;
  find and fix the line a failing check names.
- **Grades.** After the agent ends, the checks run in the kernel: files and their contents,
  commands' output and status, made apps' checkers. A tool-calling agent's exchanges need the
  calls kept as well as the text (the Exchange records what its reader read).
