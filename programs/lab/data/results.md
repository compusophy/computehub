# tiny writes applang: measured

Made by `cargo run -p compusophy-lab --release -- measure` from the weights `-- train` kept (`programs/lab/out/`, never committed); seeded, so run again on the corpus and weights it names it says the same (`lab train` and `measure` refuse any other corpus). Derived: never edit it by hand. The last section, the runs before, is kept as their own measure wrote them, each under a heading of its own.

**Corpus** `1b1d2c85b801fb4f` (`manifest.tsv`): 976 programs, 100 found in the repo and 876 variants of them (854 with names renamed, 6 with their states reordered, 69 with a line dropped, 213 with numbers changed): 356 shapes in all (a shape is a program but for its names, comments and spacing). A line dropped or numbers changed makes a near-copy that is another program: those add 266 shapes (the rest are renamed copies of one another). 627 run clean. Trained on: 410731 bytes, 28778 of them the found programs'. Held out, never trained on: the 15 found programs of a tenth of the shapes, with every program sharing a shape with them and their variants; the held-out loss is over those 15 found programs, each once (5158 bytes).

**tiny**: 989952 parameters: vocab 512 (the bytes, the end token and BPE merges), context 1024, width 128, 4 layers of 4 heads, f32.

- tiny, last: corpus 1b1d2c85b801fb4f seed 1729: step 300 of 300, batch 8 x 1024, 2457600 tokens, lr 0.003; loss 2.0211, held-out 3.6480; the file ac4c7e381cd1626a.
- tiny, lowest held-out loss: corpus 1b1d2c85b801fb4f seed 1729: step 240 of 300, batch 8 x 1024, 1966080 tokens, lr 0.003; loss 2.2266, held-out 3.6093; the file 99cabdc70eb4ddc5.
- 3-gram: counts of the training tokens, sampled from the longest context seen (the lowest held-out loss).
- 8-gram: counts of the training tokens, sampled from the longest context seen (the longest context).

**Held-out loss**, nats a token over the held-out programs (lower is better), and a byte (2151 tokens over 5158 bytes; a byte's loss compares across tokenizers, a token's does not). The lowest-loss weights, and the n-gram sampled for its loss, were picked by this same loss, which flatters them a little; training stops by it too:

| model | a token | a byte |
|---|---|---|
| tiny, last | 3.648 | 1.521 |
| tiny, lowest held-out loss | 3.609 | 1.505 |
| 2-gram, Witten-Bell | 3.842 | 1.602 |
| 3-gram, Witten-Bell | 3.574 | 1.491 |
| 4-gram, Witten-Bell | 3.724 | 1.553 |
| 5-gram, Witten-Bell | 3.821 | 1.593 |
| 6-gram, Witten-Bell | 3.881 | 1.619 |
| 7-gram, Witten-Bell | 3.917 | 1.634 |
| 8-gram, Witten-Bell | 3.947 | 1.646 |
| uniform | 6.238 | 2.602 |

**Programs written**: 10 prompts (an app's header line each, below) x 10, sampled at temperature 0.8 (tiny from its top 40 tokens), up to 2048 tokens or the end token. Each writer reads the end token and the header but its last token, and redraws that token with what follows it (BPE joins a line's end to the next line's start, as `\nstate `). *Compile*: applang's checker passes it and it has a widget. *Run clean*: the smoke test (render, click, tick, key, tap, type) finds no fault on seeds 1 to 3 and its icon line, if any, draws, as Studio checks a made app. *Novel*: no program of the corpus has its shape (its tokens but comments, names numbered as they first appear): not a copy, renamed or not. *Clean*: the share of what it wrote that comes before its first error (1 when it compiles), the mean. The compiler lexes a whole program before it parses, so a lexer error hides any parser error before it: the program cut at a lexer error is compiled again, and a parser error there comes first. The checker's errors (names, types) count only where nothing before them is wrong. The first run, below, took the compiler's first reported error, a lexer error anywhere before any parser error: its clean shares overstate.

Each writer writes three ways. *Free*: any token. *Lexer-constrained*: only tokens that keep the program lexically valid by applang's lexer (strings closed on their line, no character the language lacks, no letter run into a number) with its brackets matched, and the end token only where all is closed. *Parser-constrained*: that, and applang's parser finds no error before the end of what is written (its last token, while it may grow, judged by each thing it can still become), and the end token only where the program parses; so nothing it writes has a lexer or parser error, and one cut at the limit has its first error at its end. A token the parser refuses is drawn again without it; after 32 refusals every token is judged and the draw is among those it lets through. *Dead end*: the rule lets no token through, and the writer stops there. Neither rule looks at names or types, or runs anything: the checker and the smoke test judge every program the same way.

| writer | programs | ended | dead end | compile | run clean | novel | novel, compile | novel, run clean | clean |
|---|---|---|---|---|---|---|---|---|---|
| tiny, last | 100 | 96 | 0 | 0 | 0 | 100 | 0 | 0 | 0.03 |
| tiny, lowest held-out loss | 100 | 98 | 0 | 0 | 0 | 100 | 0 | 0 | 0.03 |
| 3-gram | 100 | 100 | 0 | 0 | 0 | 100 | 0 | 0 | 0.17 |
| 8-gram | 100 | 99 | 0 | 4 | 3 | 96 | 0 | 0 | 0.45 |
| tiny, last, lexer-constrained | 100 | 0 | 0 | 0 | 0 | 100 | 0 | 0 | 0.01 |
| tiny, lowest held-out loss, lexer-constrained | 100 | 1 | 0 | 0 | 0 | 100 | 0 | 0 | 0.01 |
| 3-gram, lexer-constrained | 100 | 14 | 0 | 0 | 0 | 100 | 0 | 0 | 0.08 |
| 8-gram, lexer-constrained | 100 | 71 | 0 | 4 | 3 | 96 | 0 | 0 | 0.39 |
| tiny, last, parser-constrained | 100 | 1 | 0 | 0 | 0 | 100 | 0 | 0 | 0.69 |
| tiny, lowest held-out loss, parser-constrained | 100 | 4 | 0 | 0 | 0 | 100 | 0 | 0 | 0.60 |
| 3-gram, parser-constrained | 100 | 20 | 0 | 2 | 2 | 98 | 0 | 0 | 0.76 |
| 8-gram, parser-constrained | 100 | 58 | 0 | 4 | 3 | 96 | 0 | 0 | 0.68 |

Why the rest did not run clean (the first error's code; 0: an icon that does not draw):

- tiny, last: E0101 x 97, E0001 x 2, E0004 x 1
- tiny, lowest held-out loss: E0101 x 93, E0001 x 5, E0004 x 2
- 3-gram: E0101 x 92, E0004 x 5, E0221 x 3
- 8-gram: E0101 x 72, E0302 x 21, E0301 x 2, E0203 x 1, E0221 x 1
- tiny, last, lexer-constrained: E0101 x 99, E0004 x 1
- tiny, lowest held-out loss, lexer-constrained: E0101 x 100
- 3-gram, lexer-constrained: E0101 x 96, E0221 x 3, E0002 x 1
- 8-gram, lexer-constrained: E0101 x 63, E0302 x 30, E0301 x 2, E0203 x 1, E0221 x 1
- tiny, last, parser-constrained: E0101 x 44, E0002 x 40, E0004 x 11, E0221 x 3, E0303 x 2
- tiny, lowest held-out loss, parser-constrained: E0002 x 53, E0101 x 37, E0004 x 6, E0221 x 2, E0302 x 2
- 3-gram, parser-constrained: E0101 x 60, E0302 x 13, E0004 x 11, E0002 x 9, E0221 x 5
- 8-gram, parser-constrained: E0302 x 48, E0101 x 37, E0301 x 5, E0102 x 2, E0308 x 2, E0004 x 1, E0203 x 1, E0221 x 1

Per prompt, compile / run clean of 10:

| prompt | tiny, last | tiny, lowest held-out loss | 3-gram | 8-gram | tiny, last, lexer-constrained | tiny, lowest held-out loss, lexer-constrained | 3-gram, lexer-constrained | 8-gram, lexer-constrained | tiny, last, parser-constrained | tiny, lowest held-out loss, parser-constrained | 3-gram, parser-constrained | 8-gram, parser-constrained |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `// Counter: two buttons change one number; Reset sets it back to 0.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `// Todo: type a task and press Add; tap its box to check it off.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `// Snake: arrows or a tap steer the snake to the food; walls and its tail end it.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `// Paint: tap or drag on the canvas to paint squares in the color picked.` | 0 / 0 | 0 / 0 | 0 / 0 | 1 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 1 / 0 | 0 / 0 | 0 / 0 | 1 / 1 | 1 / 0 |
| `// Tic-tac-toe: two players take turns marking X and O on a 3 x 3 board.` | 0 / 0 | 0 / 0 | 0 / 0 | 1 / 1 | 0 / 0 | 0 / 0 | 0 / 0 | 1 / 1 | 0 / 0 | 0 / 0 | 0 / 0 | 1 / 1 |
| `// Dice: press Roll to throw two dice and show their total.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 1 / 1 | 0 / 0 |
| `// Timer: Start counts down from 60 seconds; Stop pauses it.` | 0 / 0 | 0 / 0 | 0 / 0 | 2 / 2 | 0 / 0 | 0 / 0 | 0 / 0 | 2 / 2 | 0 / 0 | 0 / 0 | 0 / 0 | 2 / 2 |
| `// Clock: a round face whose hands show the seconds since it opened.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `// Pong: drag the paddle to keep the ball in play; each return scores.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `// Quiz: three questions, each with buttons for its answers, and the score at the end.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |

Every program that compiled (6):

3-gram, parser-constrained (runs clean, a copy):

```app
// Dice: press Roll to throw two dice and show their total.
state bx = 0;
label 2;
```

8-gram; 8-gram, lexer-constrained; 8-gram, parser-constrained (faults, a copy):

```app
// Paint: tap or drag on the canvas to paint squares in the color picked.
state p = 0;
state enemy = 0;
label "avg " + p / enemy;
```

3-gram, parser-constrained (runs clean, a copy):

```app
// Paint: tap or drag on the canvas to paint squares in the color picked.
state vy = 0;
label 2;
```

8-gram; 8-gram, lexer-constrained; 8-gram, parser-constrained (runs clean, a copy):

```app
// Tic-tac-toe: two players take turns marking X and O on a 3 x 3 board.
state mode = 0;
label mode;
```

8-gram; 8-gram, lexer-constrained; 8-gram, parser-constrained (runs clean, a copy):

```app
// Timer: Start counts down from 60 seconds; Stop pauses it.
// icon: dot 12 12 3  
label 1;
```

8-gram; 8-gram, lexer-constrained; 8-gram, parser-constrained (runs clean, a copy):

```app
// Timer: Start counts down from 60 seconds; Stop pauses it. Kept between runs.
// icon: dot 12 12 3
// B
label 1;
```

## Earlier runs

### 2026-10-05, the second run: corpus `1b1d2c85b801fb4f`, stopped early, its cosine planned over 600 steps

Measured again after two fixes, so not the numbers first committed for it: the parser rule judges a name by every word it may still become (it had let the start of a word through unchecked, then refused all that followed), and *clean* lets a parser error come before a later lexer error (it had put tiny's first error, free or lexer-constrained, 11 to 51% of the way through; it comes 2 to 3% in).

**Corpus** `1b1d2c85b801fb4f` (`manifest.tsv`): 976 programs, 100 found in the repo and 876 variants of them (854 with names renamed, 6 with their states reordered, 69 with a line dropped, 213 with numbers changed): 356 shapes in all (a shape is a program but for its names, comments and spacing). A line dropped or numbers changed makes a near-copy that is another program: those add 266 shapes (the rest are renamed copies of one another). 627 run clean. Trained on: 410731 bytes, 28778 of them the found programs'. Held out, never trained on: the 15 found programs of a tenth of the shapes, with every program sharing a shape with them and their variants; the held-out loss is over those 15 found programs, each once (5158 bytes).

**tiny**: 989952 parameters: vocab 512 (the bytes, the end token and BPE merges), context 1024, width 128, 4 layers of 4 heads, f32.

- tiny, last: corpus 1b1d2c85b801fb4f seed 1729: step 270 of 600, batch 8 x 1024, 2211840 tokens, lr 0.003; loss 1.9699, held-out 3.6842; the file 13e1cc83a3445267.
- tiny, lowest held-out loss: corpus 1b1d2c85b801fb4f seed 1729: step 150 of 600, batch 8 x 1024, 1228800 tokens, lr 0.003; loss 2.8327, held-out 3.6699; the file 1860c6d143497fb9.
- 3-gram: counts of the training tokens, sampled from the longest context seen (the lowest held-out loss).
- 8-gram: counts of the training tokens, sampled from the longest context seen (the longest context).

**Held-out loss**, nats a token over the held-out programs (lower is better), and a byte (2151 tokens over 5158 bytes; a byte's loss compares across tokenizers, a token's does not). The lowest-loss weights, and the n-gram sampled for its loss, were picked by this same loss, which flatters them a little; training stops by it too:

| model | a token | a byte |
|---|---|---|
| tiny, last | 3.684 | 1.536 |
| tiny, lowest held-out loss | 3.670 | 1.530 |
| 2-gram, Witten-Bell | 3.842 | 1.602 |
| 3-gram, Witten-Bell | 3.574 | 1.491 |
| 4-gram, Witten-Bell | 3.724 | 1.553 |
| 5-gram, Witten-Bell | 3.821 | 1.593 |
| 6-gram, Witten-Bell | 3.881 | 1.619 |
| 7-gram, Witten-Bell | 3.917 | 1.634 |
| 8-gram, Witten-Bell | 3.947 | 1.646 |
| uniform | 6.238 | 2.602 |

**Programs written**: 10 prompts (an app's header line each, below) x 10, sampled at temperature 0.8 (tiny from its top 40 tokens), up to 2048 tokens or the end token. Each writer reads the end token and the header but its last token, and redraws that token with what follows it (BPE joins a line's end to the next line's start, as `\nstate `). *Compile*: applang's checker passes it and it has a widget. *Run clean*: the smoke test (render, click, tick, key, tap, type) finds no fault on seeds 1 to 3 and its icon line, if any, draws, as Studio checks a made app. *Novel*: no program of the corpus has its shape (its tokens but comments, names numbered as they first appear): not a copy, renamed or not. *Clean*: the share of what it wrote that comes before its first error (1 when it compiles), the mean. The compiler lexes a whole program before it parses, so a lexer error hides any parser error before it: the program cut at a lexer error is compiled again, and a parser error there comes first. The checker's errors (names, types) count only where nothing before them is wrong. The earlier runs, below, took the compiler's first reported error, a lexer error anywhere before any parser error: their clean shares overstate.

Each writer writes three ways. *Free*: any token. *Lexer-constrained*: only tokens that keep the program lexically valid by applang's lexer (strings closed on their line, no character the language lacks, no letter run into a number) with its brackets matched, and the end token only where all is closed. *Parser-constrained*: that, and applang's parser finds no error before the end of what is written (its last token, while it may grow, judged by each thing it can still become), and the end token only where the program parses; so nothing it writes has a lexer or parser error, and one cut at the limit has its first error at its end. A token the parser refuses is drawn again without it; after 32 refusals every token is judged and the draw is among those it lets through. *Dead end*: the rule lets no token through, and the writer stops there. Neither rule looks at names or types, or runs anything: the checker and the smoke test judge every program the same way.

| writer | programs | ended | dead end | compile | run clean | novel | novel, compile | novel, run clean | clean |
|---|---|---|---|---|---|---|---|---|---|
| tiny, last | 100 | 90 | 0 | 0 | 0 | 100 | 0 | 0 | 0.03 |
| tiny, lowest held-out loss | 100 | 74 | 0 | 0 | 0 | 100 | 0 | 0 | 0.03 |
| 3-gram | 100 | 100 | 0 | 0 | 0 | 100 | 0 | 0 | 0.17 |
| 8-gram | 100 | 99 | 0 | 4 | 3 | 96 | 0 | 0 | 0.45 |
| tiny, last, lexer-constrained | 100 | 2 | 0 | 0 | 0 | 100 | 0 | 0 | 0.02 |
| tiny, lowest held-out loss, lexer-constrained | 100 | 4 | 0 | 0 | 0 | 100 | 0 | 0 | 0.02 |
| 3-gram, lexer-constrained | 100 | 14 | 0 | 0 | 0 | 100 | 0 | 0 | 0.08 |
| 8-gram, lexer-constrained | 100 | 71 | 0 | 4 | 3 | 96 | 0 | 0 | 0.39 |
| tiny, last, parser-constrained | 100 | 2 | 0 | 0 | 0 | 100 | 0 | 0 | 0.81 |
| tiny, lowest held-out loss, parser-constrained | 100 | 1 | 0 | 0 | 0 | 100 | 0 | 0 | 0.54 |
| 3-gram, parser-constrained | 100 | 20 | 0 | 2 | 2 | 98 | 0 | 0 | 0.76 |
| 8-gram, parser-constrained | 100 | 58 | 0 | 4 | 3 | 96 | 0 | 0 | 0.68 |

Why the rest did not run clean (the first error's code; 0: an icon that does not draw):

- tiny, last: E0101 x 92, E0001 x 3, E0004 x 3, E0003 x 2
- tiny, lowest held-out loss: E0101 x 87, E0001 x 6, E0004 x 5, E0002 x 2
- 3-gram: E0101 x 92, E0004 x 5, E0221 x 3
- 8-gram: E0101 x 72, E0302 x 21, E0301 x 2, E0203 x 1, E0221 x 1
- tiny, last, lexer-constrained: E0101 x 100
- tiny, lowest held-out loss, lexer-constrained: E0101 x 97, E0002 x 2, E0221 x 1
- 3-gram, lexer-constrained: E0101 x 96, E0221 x 3, E0002 x 1
- 8-gram, lexer-constrained: E0101 x 63, E0302 x 30, E0301 x 2, E0203 x 1, E0221 x 1
- tiny, last, parser-constrained: E0101 x 60, E0002 x 26, E0004 x 11, E0302 x 2, E0303 x 1
- tiny, lowest held-out loss, parser-constrained: E0002 x 59, E0101 x 35, E0004 x 5, E0221 x 1
- 3-gram, parser-constrained: E0101 x 60, E0302 x 13, E0004 x 11, E0002 x 9, E0221 x 5
- 8-gram, parser-constrained: E0302 x 48, E0101 x 37, E0301 x 5, E0102 x 2, E0308 x 2, E0004 x 1, E0203 x 1, E0221 x 1

Per prompt, compile / run clean of 10:

| prompt | tiny, last | tiny, lowest held-out loss | 3-gram | 8-gram | tiny, last, lexer-constrained | tiny, lowest held-out loss, lexer-constrained | 3-gram, lexer-constrained | 8-gram, lexer-constrained | tiny, last, parser-constrained | tiny, lowest held-out loss, parser-constrained | 3-gram, parser-constrained | 8-gram, parser-constrained |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `// Counter: two buttons change one number; Reset sets it back to 0.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `// Todo: type a task and press Add; tap its box to check it off.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `// Snake: arrows or a tap steer the snake to the food; walls and its tail end it.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `// Paint: tap or drag on the canvas to paint squares in the color picked.` | 0 / 0 | 0 / 0 | 0 / 0 | 1 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 1 / 0 | 0 / 0 | 0 / 0 | 1 / 1 | 1 / 0 |
| `// Tic-tac-toe: two players take turns marking X and O on a 3 x 3 board.` | 0 / 0 | 0 / 0 | 0 / 0 | 1 / 1 | 0 / 0 | 0 / 0 | 0 / 0 | 1 / 1 | 0 / 0 | 0 / 0 | 0 / 0 | 1 / 1 |
| `// Dice: press Roll to throw two dice and show their total.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 1 / 1 | 0 / 0 |
| `// Timer: Start counts down from 60 seconds; Stop pauses it.` | 0 / 0 | 0 / 0 | 0 / 0 | 2 / 2 | 0 / 0 | 0 / 0 | 0 / 0 | 2 / 2 | 0 / 0 | 0 / 0 | 0 / 0 | 2 / 2 |
| `// Clock: a round face whose hands show the seconds since it opened.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `// Pong: drag the paddle to keep the ball in play; each return scores.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `// Quiz: three questions, each with buttons for its answers, and the score at the end.` | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |

Every program that compiled (6):

3-gram, parser-constrained (runs clean, a copy):

```app
// Dice: press Roll to throw two dice and show their total.
state bx = 0;
label 2;
```

8-gram; 8-gram, lexer-constrained; 8-gram, parser-constrained (faults, a copy):

```app
// Paint: tap or drag on the canvas to paint squares in the color picked.
state p = 0;
state enemy = 0;
label "avg " + p / enemy;
```

3-gram, parser-constrained (runs clean, a copy):

```app
// Paint: tap or drag on the canvas to paint squares in the color picked.
state vy = 0;
label 2;
```

8-gram; 8-gram, lexer-constrained; 8-gram, parser-constrained (runs clean, a copy):

```app
// Tic-tac-toe: two players take turns marking X and O on a 3 x 3 board.
state mode = 0;
label mode;
```

8-gram; 8-gram, lexer-constrained; 8-gram, parser-constrained (runs clean, a copy):

```app
// Timer: Start counts down from 60 seconds; Stop pauses it.
// icon: dot 12 12 3  
label 1;
```

8-gram; 8-gram, lexer-constrained; 8-gram, parser-constrained (runs clean, a copy):

```app
// Timer: Start counts down from 60 seconds; Stop pauses it. Kept between runs.
// icon: dot 12 12 3
// B
label 1;
```

### 2026-10-05, the first run: corpus `501396c028bf6da2`, free decoding, no early stop

Its held-out loss a byte, worked out later (its tokenizer rebuilt from these 570 programs, which the second run's corpus holds; 2159 held-out tokens over the same 5158 bytes): tiny, lowest held-out loss, 1.501 nats; the 3-gram 1.479. Its *clean* is to the compiler's first reported error, which puts a lexer error anywhere before any parser error: it overstates.

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
