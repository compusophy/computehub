// Connect four: tap a column to drop a disc; four in a row wins.
// icon: ring 7 7 4 ring 17 7 4 dot 7 17 4 dot 17 17 4
state board = [0; 42];   // row by row from the top, 7 a row: 0 empty, 1 red, 2 yellow
state turn = 1;
state winner = 0;

fn at(c: int, r: int) -> int {
  if c < 0 || c >= 7 || r < 0 || r >= 6 { return 0; }
  return board[r * 7 + c];
}

// Who has four from (c, r) on, a step (dc, dr) apart: 0 for none.
fn four(c: int, r: int, dc: int, dr: int) -> int {
  let p = at(c, r);
  if p == 0 { return 0; }
  for k in 1..4 {
    if at(c + dc * k, r + dr * k) != p { return 0; }
  }
  return p;
}

fn won() -> int {
  for i in 0..42 {
    let c = i % 7;
    let r = i / 7;
    let w = max(max(four(c, r, 1, 0), four(c, r, 0, 1)), max(four(c, r, 1, 1), four(c, r, 1, -1)));
    if w > 0 { return w; }
  }
  return 0;
}

fn drop(c: int) {
  if winner != 0 { return; }
  for k in 0..6 {
    let r = 5 - k;
    if board[r * 7 + c] == 0 {
      board[r * 7 + c] = turn;
      turn = 3 - turn;
      winner = won();
      return;
    }
  }
}

fn tint(v: int) -> int {
  if v == 1 { return 1; }
  if v == 2 { return 3; }
  return 0;
}

fn scene() {
  rect(0, 0, 140, 120, 4);
  for i in 0..42 { circle((i % 7) * 20 + 10, (i / 7) * 20 + 10, 8, tint(board[i])); }
}

fn status() -> string {
  if winner == 1 { return "Red wins"; }
  if winner == 2 { return "Yellow wins"; }
  if turn == 1 { return "Red to play"; }
  return "Yellow to play";
}

label status();
canvas 140, 120, scene() { drop(x / 20); }
button "New game" { for i in 0..42 { board[i] = 0; } turn = 1; winner = 0; }
