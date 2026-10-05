// Tic-tac-toe on a grid widget: tap a square for X, then O.
// icon: line 2 7 10 15 line 10 7 2 15 ring 17 11 4
state board = [0; 9];
state marks = ["", "", "", "", "", "", "", "", ""];
state turn = 1;
state winner = 0;

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
  if winner == 1 { return "X wins!"; }
  if winner == 2 { return "O wins!"; }
  if winner == 3 { return "It's a tie"; }
  return "Your move";
}

label status();
grid 3, board, marks {
  if winner == 0 && board[cell] == 0 {
    board[cell] = turn;
    if turn == 1 { marks[cell] = "X"; } else { marks[cell] = "O"; }
    turn = 3 - turn;
    winner = won();
  }
}
button "New game" {
  for i in 0..9 {
    board[i] = 0;
    marks[i] = "";
  }
  turn = 1;
  winner = 0;
}
