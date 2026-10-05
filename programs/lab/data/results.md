# tiny writes applang: measured

Made by `cargo run -p compusophy-lab --release -- measure` from the weights `-- train` kept (`programs/lab/out/`, never committed); seeded, so run again on the corpus and weights it names it says the same (`lab train` and `measure` refuse any other corpus). Derived: never edit it by hand.

**Corpus** `501396c028bf6da2` (`manifest.tsv`): 570 programs, 100 found in the repo and 470 variants of them (names renamed; 6 of those also with their states reordered): 90 shapes in all (a shape is a program but for its names, comments and spacing); 391 run clean. Trained on: 234305 bytes, 28778 of them the found programs'. Held out, never trained on: the 15 found programs of a tenth of the shapes, with every program sharing a shape with them and their variants; the held-out loss is over those 15 found programs, each once (5158 bytes).

**tiny**: 989952 parameters: vocab 512 (the bytes, the end token and BPE merges), context 1024, width 128, 4 layers of 4 heads, f32.

- tiny, last: corpus 501396c028bf6da2 seed 1729: step 600 of 600, batch 8 x 1024, 4915200 tokens, lr 0.003; loss 0.5103, held-out 4.4848; the file c982a043714c7fa9.
- tiny, lowest held-out loss: corpus 501396c028bf6da2 seed 1729: step 210 of 600, batch 8 x 1024, 1720320 tokens, lr 0.003; loss 2.2130, held-out 3.5859; the file 226d1f37b5d2d44f.
- 3-gram: counts of the training tokens, sampled from the longest context seen (the lowest held-out loss).
- 8-gram: counts of the training tokens, sampled from the longest context seen (the longest context).

**Held-out loss**, nats a token over the held-out programs (lower is better). The lowest-loss weights, and the n-gram sampled for its loss, were picked by this same loss, which flatters them a little:

| model | loss |
|---|---|
| tiny, last | 4.485 |
| tiny, lowest held-out loss | 3.586 |
| 2-gram, Witten-Bell | 3.824 |
| 3-gram, Witten-Bell | 3.534 |
| 4-gram, Witten-Bell | 3.656 |
| 5-gram, Witten-Bell | 3.734 |
| 6-gram, Witten-Bell | 3.779 |
| 7-gram, Witten-Bell | 3.811 |
| 8-gram, Witten-Bell | 3.837 |
| uniform | 6.238 |

**Programs written**: 10 prompts (an app's header line each, below) x 10, sampled at temperature 0.8 (tiny from its top 40 tokens), up to 2048 tokens or the end token. Each writer reads the end token and the header but its last token, and redraws that token with what follows it (BPE joins a line's end to the next line's start, as `\nstate `). *Compile*: applang's checker passes it and it has a widget. *Run clean*: the smoke test (render, click, tick, key, tap, type) finds no fault on seeds 1 to 3 and its icon line, if any, draws, as Studio checks a made app. *Novel*: no program of the corpus has its shape (its tokens but comments, names numbered as they first appear): not a copy, renamed or not. *Clean*: the share of what it wrote that comes before the compiler's first error (1 when it compiles), the mean.

| writer | programs | ended | compile | run clean | novel | novel, compile | novel, run clean | clean |
|---|---|---|---|---|---|---|---|---|
| tiny, last | 100 | 97 | 0 | 0 | 100 | 0 | 0 | 0.38 |
| tiny, lowest held-out loss | 100 | 95 | 0 | 0 | 100 | 0 | 0 | 0.26 |
| 3-gram | 100 | 100 | 0 | 0 | 100 | 0 | 0 | 0.37 |
| 8-gram | 100 | 97 | 2 | 2 | 98 | 0 | 0 | 0.43 |

At other temperatures (the same prompts and seeds):

| writer | programs | ended | compile | run clean | novel | novel, compile | novel, run clean | clean |
|---|---|---|---|---|---|---|---|---|
| tiny, last, temperature 0.2 | 100 | 81 | 0 | 0 | 100 | 0 | 0 | 0.39 |
| tiny, last, temperature 0.5 | 100 | 96 | 0 | 0 | 100 | 0 | 0 | 0.38 |
| tiny, last, temperature 1 | 100 | 98 | 0 | 0 | 100 | 0 | 0 | 0.27 |
| tiny, lowest held-out loss, temperature 0.2 | 100 | 35 | 0 | 0 | 100 | 0 | 0 | 0.26 |
| tiny, lowest held-out loss, temperature 0.5 | 100 | 86 | 0 | 0 | 100 | 0 | 0 | 0.19 |
| tiny, lowest held-out loss, temperature 1 | 100 | 98 | 0 | 0 | 100 | 0 | 0 | 0.19 |

Why the rest did not run clean (the first error's code; 0: an icon that does not draw):

- tiny, last: E0004 x 59, E0003 x 21, E0001 x 16, E0101 x 3, E0002 x 1
- tiny, lowest held-out loss: E0004 x 65, E0001 x 25, E0003 x 7, E0101 x 3
- 3-gram: E0004 x 42, E0101 x 38, E0001 x 16, E0221 x 4
- 8-gram: E0101 x 65, E0302 x 30, E0221 x 2, E0308 x 1

Per prompt, compile / run clean of 10:

| prompt | tiny, last | tiny, lowest held-out loss | 3-gram | 8-gram |
|---|---|---|---|---|
| `// Counter: two buttons change one number; Reset sets it back to 0.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `// Todo: type a task and press Add; tap its box to check it off.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `// Snake: arrows or a tap steer the snake to the food; walls and its tail end it.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `// Paint: tap or drag on the canvas to paint squares in the color picked.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `// Tic-tac-toe: two players take turns marking X and O on a 3 x 3 board.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `// Dice: press Roll to throw two dice and show their total.` | 0 / 0 | 0 / 0 | 0 / 0 | 1 / 1 |
| `// Timer: Start counts down from 60 seconds; Stop pauses it.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `// Clock: a round face whose hands show the seconds since it opened.` | 0 / 0 | 0 / 0 | 0 / 0 | 1 / 1 |
| `// Pong: drag the paddle to keep the ball in play; each return scores.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `// Quiz: three questions, each with buttons for its answers, and the score at the end.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |

Every program that compiled, at any temperature (2):

8-gram (runs clean, a copy):

```app
// Clock: a round face whose hands show the seconds since it opened.
state n = 0;
label "Tally";
label n;
```

8-gram (runs clean, a copy):

```app
// Dice: press Roll to throw two dice and show their total.
state stack = 0;
label stack;
```
