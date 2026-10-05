// Minesweeper on a grid widget, numbers written in the squares; Flag marks.
// icon: dot 12 12 6 line 12 3 12 21 line 3 12 21 12
state mines = [false; 81];
state open = [false; 81];
state flags = [false; 81];
state shown = [8; 81];     // the grid: 8 hidden, 4 flagged, 7 open, 1 a mine
state words = [""; 81];
state placed = false;
state lost = false;
state won = false;
state flagging = false;

fn mine(c: int, r: int) -> int {
  if c < 0 || c >= 9 || r < 0 || r >= 9 { return 0; }
  if mines[r * 9 + c] { return 1; }
  return 0;
}

fn near(i: int) -> int {
  let n = 0;
  for k in 0..9 { n += mine(i % 9 + k % 3 - 1, i / 9 + k / 3 - 1); }
  return n;
}

fn refresh() {
  let safe = 0;
  for i in 0..81 {
    words[i] = "";
    if open[i] && mines[i] {
      shown[i] = 1;
    } else if open[i] {
      shown[i] = 7;
      safe += 1;
      if near(i) > 0 { words[i] = "" + near(i); }
    } else if flags[i] {
      shown[i] = 4;
    } else {
      shown[i] = 8;
    }
  }
  won = safe == 71;
}

fn reveal(first: int) {
  open[first] = true;
  let more = true;
  repeat 81 {
    if more {
      more = false;
      for i in 0..81 {
        if open[i] && !mines[i] && near(i) == 0 {
          for k in 0..9 {
            let c = i % 9 + k % 3 - 1;
            let r = i / 9 + k / 3 - 1;
            if c >= 0 && c < 9 && r >= 0 && r < 9 && !open[r * 9 + c] {
              open[r * 9 + c] = true;
              more = true;
            }
          }
        }
      }
    }
  }
}

label "Mines: 10";
if lost { label "Game over"; } else if won { label "You won!"; }
grid 9, shown, words {
  if !lost && !won {
    if flagging {
      if !open[cell] { flags[cell] = !flags[cell]; }
    } else if !flags[cell] && !open[cell] {
      if !placed {
        let n = 0;
        repeat 2000 {
          let i = random(81);
          if n < 10 && i != cell && !mines[i] {
            mines[i] = true;
            n += 1;
          }
        }
        placed = true;
      }
      if mines[cell] {
        open[cell] = true;
        lost = true;
      } else {
        reveal(cell);
      }
    }
    refresh();
  }
}
button "Flag" { flagging = !flagging; }
