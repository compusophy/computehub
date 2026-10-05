// Life: tap squares alive or dead; Step, Play and Pause, Clear.
// icon: fill 4 4 9 4 9 9 4 9 fill 10 10 15 10 15 15 10 15 fill 16 4 21 4 21 9 16 9
state cells = [0; 400];    // 20 x 20, row by row: 1 alive
state speed = 0;

fn alive(c: int, r: int) -> int {
  if c < 0 || c >= 20 || r < 0 || r >= 20 { return 0; }
  return cells[r * 20 + c];
}

fn near(c: int, r: int) -> int {
  let n = 0;
  for k in 0..9 {
    if k != 4 { n += alive(c + k % 3 - 1, r + k / 3 - 1); }
  }
  return n;
}

fn step() {
  let next = [0; 400];
  for i in 0..400 {
    let n = near(i % 20, i / 20);
    if n == 3 || (n == 2 && cells[i] == 1) { next[i] = 1; }
  }
  cells = next;
}

fn scene() {
  let b = [-1; 400];
  for i in 0..400 {
    if cells[i] == 1 { b[i] = 2; }
  }
  pixels(b, 0, 0, 20, 6);
}

every speed { step(); }

canvas 120, 120, scene() {
  let i = (y / 6) * 20 + x / 6;
  cells[i] = 1 - cells[i];
}
row {
  button "Step" { step(); }
  if speed == 0 { button "Play" { speed = 200; } } else { button "Pause" { speed = 0; } }
  button "Clear" { for i in 0..400 { cells[i] = 0; } }
}
