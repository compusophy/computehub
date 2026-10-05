// Minesweeper: 9 x 9 and 10 mines; tap to reveal, Flag to mark; the first tap is safe.
// icon: dot 12 12 6 line 12 3 12 21 line 3 12 21 12 line 6 6 18 18 line 6 18 18 6
state mines = [0; 81];
state open = [0; 81];     // 0 hidden, 1 shown, 2 flagged
state placed = false;
state over = 0;           // 0 playing, 1 lost, 2 won
state flagging = false;

fn mine(c: int, r: int) -> int {
  if c < 0 || c >= 9 || r < 0 || r >= 9 { return 0; }
  return mines[r * 9 + c];
}

fn count(i: int) -> int {
  let n = 0;
  for k in 0..9 { n += mine(i % 9 + k % 3 - 1, i / 9 + k / 3 - 1); }
  return n;
}

fn place(first: int) {
  let n = 0;
  repeat 1000 {
    if n < 10 {
      let i = random(81);
      if i != first && mines[i] == 0 {
        mines[i] = 1;
        n += 1;
      }
    }
  }
  placed = true;
}

// Squares that touch no mine show their neighbours, until nothing more opens.
fn flood() {
  let more = true;
  repeat 81 {
    if more {
      more = false;
      for i in 0..81 {
        if open[i] == 1 && mines[i] == 0 && count(i) == 0 {
          for k in 0..9 {
            let c = i % 9 + k % 3 - 1;
            let r = i / 9 + k / 3 - 1;
            if c >= 0 && c < 9 && r >= 0 && r < 9 {
              if open[r * 9 + c] == 0 {
                open[r * 9 + c] = 1;
                more = true;
              }
            }
          }
        }
      }
    }
  }
}

fn won() -> bool {
  for i in 0..81 {
    if mines[i] == 0 && open[i] != 1 { return false; }
  }
  return true;
}

fn tap(i: int) {
  if over != 0 { return; }
  if flagging {
    if open[i] == 0 { open[i] = 2; } else if open[i] == 2 { open[i] = 0; }
    return;
  }
  if open[i] != 0 { return; }
  if !placed { place(i); }
  open[i] = 1;
  if mines[i] == 1 {
    over = 1;
    return;
  }
  flood();
  if won() { over = 2; }
}

fn square(i: int) {
  let px = (i % 9) * 12;
  let py = (i / 9) * 12;
  if open[i] == 1 {
    rect(px, py, 11, 11, 7);
    if mines[i] == 1 {
      circle(px + 5, py + 5, 3, 1);
    } else if count(i) > 0 {
      text(count(i), px + 5, py + 6, 8, 9);
    }
  } else if open[i] == 2 {
    rect(px, py, 11, 11, 4);
  } else {
    rect(px, py, 11, 11, 8);
  }
}

fn scene() {
  for i in 0..81 { square(i); }
}

fn status() -> string {
  if over == 1 { return "Game over"; }
  if over == 2 { return "You win"; }
  if flagging { return "Flagging"; }
  return "Revealing";
}

label status();
canvas 108, 108, scene() { tap((y / 12) * 9 + x / 12); }
row {
  button "Flag" { flagging = !flagging; }
  button "New game" {
    for i in 0..81 {
      mines[i] = 0;
      open[i] = 0;
    }
    placed = false;
    over = 0;
    flagging = false;
  }
}
