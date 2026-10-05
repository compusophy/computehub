# tiny writes applang: measured

Made by `cargo run -p compusophy-lab --release -- measure` from the weights `-- train` kept (`programs/lab/out/`, never committed); seeded, so run again it says the same. Derived: never edit it by hand.

**Corpus** `489422ff8da37794` (`manifest.tsv`): 570 programs, 100 found in the repo and 470 variants of them (names renamed, states reordered), 391 of them running clean; 13 found programs held out with their variants, never trained on.

**tiny**: 989952 parameters: vocab 512 (the bytes, the end token and BPE merges), context 1024, width 128, 4 layers of 4 heads, f32, trained on 8 CPU threads.

- tiny, last: corpus 489422ff8da37794 seed 1729 step 600 of 600, batch 8 x 1024, 4915200 tokens, lr 0.003, 8 threads, 42.6 minutes; loss 0.5977, held-out 4.2925.
- tiny, lowest held-out loss: corpus 489422ff8da37794 seed 1729 step 180 of 600, batch 8 x 1024, 1474560 tokens, lr 0.003, 8 threads, 13.7 minutes; loss 2.3067, held-out 3.5327.
- 3-gram: counts of the training tokens, sampled from the longest context seen (the lowest held-out loss).
- 8-gram: counts of the training tokens, sampled from the longest context seen (the longest context).

**Held-out loss**, nats a token over the held-out programs (lower is better):

| model | loss |
|---|---|
| tiny, last | 4.293 |
| tiny, lowest held-out loss | 3.533 |
| 2-gram, Witten-Bell | 3.809 |
| 3-gram, Witten-Bell | 3.724 |
| 4-gram, Witten-Bell | 3.924 |
| 5-gram, Witten-Bell | 4.027 |
| 6-gram, Witten-Bell | 4.076 |
| 7-gram, Witten-Bell | 4.116 |
| 8-gram, Witten-Bell | 4.129 |
| uniform | 6.238 |

**Programs written**: 10 prompts (an app's header line each, below) x 10, sampled at temperature 0.8 (tiny from its top 40 tokens), up to 2048 tokens or the end token. *Compile*: applang's checker passes it and it has a widget. *Run clean*: the smoke test (render, click, tick, key, tap, type) finds no fault on seeds 1 to 3 and its icon line, if any, draws, as Studio checks a made app. *Novel*: no program of the corpus has its shape (its tokens but comments, names numbered as they first appear): not a copy, renamed or not. *Clean*: the share of what it wrote that comes before the compiler's first error (1 when it compiles), the mean.

| writer | programs | ended | compile | run clean | novel | novel, compile | novel, run clean | clean |
|---|---|---|---|---|---|---|---|---|
| tiny, last | 100 | 98 | 3 | 3 | 98 | 1 | 1 | 0.31 |
| tiny, lowest held-out loss | 100 | 99 | 0 | 0 | 100 | 0 | 0 | 0.19 |
| 3-gram | 100 | 100 | 0 | 0 | 100 | 0 | 0 | 0.28 |
| 8-gram | 100 | 97 | 2 | 2 | 98 | 0 | 0 | 0.36 |

At other temperatures (the same prompts and seeds):

| writer | programs | ended | compile | run clean | novel | novel, compile | novel, run clean | clean |
|---|---|---|---|---|---|---|---|---|
| tiny, last, temperature 0.2 | 100 | 88 | 0 | 0 | 100 | 0 | 0 | 0.30 |
| tiny, last, temperature 0.5 | 100 | 96 | 0 | 0 | 100 | 0 | 0 | 0.36 |
| tiny, last, temperature 1 | 100 | 100 | 2 | 2 | 98 | 0 | 0 | 0.30 |
| tiny, lowest held-out loss, temperature 0.2 | 100 | 50 | 0 | 0 | 100 | 0 | 0 | 0.24 |
| tiny, lowest held-out loss, temperature 0.5 | 100 | 84 | 0 | 0 | 100 | 0 | 0 | 0.19 |
| tiny, lowest held-out loss, temperature 1 | 100 | 99 | 0 | 0 | 100 | 0 | 0 | 0.18 |

Why the rest did not run clean (the first error's code; 0: an icon that does not draw):

- tiny, last: E0004 x 52, E0001 x 28, E0003 x 10, E0101 x 7
- tiny, lowest held-out loss: E0004 x 50, E0001 x 44, E0101 x 3, E0003 x 2, E0221 x 1
- 3-gram: E0004 x 62, E0101 x 23, E0001 x 14, E0302 x 1
- 8-gram: E0101 x 81, E0302 x 17

Per prompt, compile / run clean of 10:

| prompt | tiny, last | tiny, lowest held-out loss | 3-gram | 8-gram |
|---|---|---|---|---|
| `// Counter: two buttons change one number; Reset sets it back to 0.` | 1 / 1 | 0 / 0 | 0 / 0 | 0 / 0 |
| `// Todo: type a task and press Add; tap its box to check it off.` | 1 / 1 | 0 / 0 | 0 / 0 | 0 / 0 |
| `// Snake: arrows or a tap steer the snake to the food; walls and its tail end it.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `// Paint: tap or drag on the canvas to paint squares in the color picked.` | 0 / 0 | 0 / 0 | 0 / 0 | 2 / 2 |
| `// Tic-tac-toe: two players take turns marking X and O on a 3 x 3 board.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `// Dice: press Roll to throw two dice and show their total.` | 1 / 1 | 0 / 0 | 0 / 0 | 0 / 0 |
| `// Timer: Start counts down from 60 seconds; Stop pauses it.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `// Clock: a round face whose hands show the seconds since it opened.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `// Pong: drag the paddle to keep the ball in play; each return scores.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `// Quiz: three questions, each with buttons for its answers, and the score at the end.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |

The longest novel program tiny wrote that compiles (tiny, last; it runs clean):

```app
// Todo: type a task and press Add; tap its box to check it off.
label 11;
```
