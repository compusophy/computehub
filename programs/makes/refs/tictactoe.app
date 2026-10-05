// Tic-tac-toe: tap a square to play X, then O; three in a row wins.
// icon: line 2 7 10 15 line 10 7 2 15 ring 17 11 4
state board = [0; 9];      // 0 empty, 1 X, 2 O
state turn = 1;
state winner = 0;          // 0 none yet, 1 X, 2 O, 3 a draw

fn same(a: int, b: int, c: int) -> int {
  if board[a] != 0 && board[a] == board[b] && board[b] == board[c] { return board[a]; }
  return 0;
}
fn won() -> int {
  let w = max(same(0, 1, 2), max(same(3, 4, 5), same(6, 7, 8)));
  w = max(w, max(same(0, 3, 6), max(same(1, 4, 7), same(2, 5, 8))));
  w = max(w, max(same(0, 4, 8), same(2, 4, 6)));
  if w == 0 { for i in 0..9 { if board[i] == 0 { return 0; } } return 3; }
  return w;
}
fn status() -> string {
  if winner == 1 { return "X wins"; }
  if winner == 2 { return "O wins"; }
  if winner == 3 { return "Draw"; }
  if turn == 1 { return "X to play"; }
  return "O to play";
}

// One square's mark: an X of two lines, an O a ring.
fn mark(i: int) {
  let px = (i % 3) * 100 + 50;
  let py = (i / 3) * 100 + 50;
  if board[i] == 1 {
    line(px - 28, py - 28, px + 28, py + 28, 8, 1);
    line(px - 28, py + 28, px + 28, py - 28, 8, 1);
  }
  if board[i] == 2 { ring(px, py, 30, 8, 4); }
}
fn scene() {
  for k in 1..3 {
    line(k * 100, 8, k * 100, 291, 3, 8);
    line(8, k * 100, 291, k * 100, 3, 8);
  }
  for i in 0..9 { mark(i); }
}

label status();
canvas 300, 300, scene() {
  let i = (y / 100) * 3 + x / 100;
  if winner == 0 && board[i] == 0 {
    board[i] = turn;
    turn = 3 - turn;
    winner = won();
  }
}
button "New game" { for i in 0..9 { board[i] = 0; } turn = 1; winner = 0; }
