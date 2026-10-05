// Life on a grid widget: tap squares; Step, one Play/Pause button, Clear.
// icon: fill 4 4 9 4 9 9 4 9 fill 10 10 15 10 15 15 10 15 fill 16 4 21 4 21 9 16 9
state cells = [0; 400];
state speed = 0;

fn alive(c: int, r: int) -> int {
  if c < 0 || c >= 20 || r < 0 || r >= 20 { return 0; }
  return min(1, cells[r * 20 + c]);
}

fn step() {
  let next = [0; 400];
  for i in 0..400 {
    let n = 0;
    for k in 0..9 {
      if k != 4 { n += alive(i % 20 + k % 3 - 1, i / 20 + k / 3 - 1); }
    }
    if n == 3 || (n == 2 && cells[i] > 0) { next[i] = 6; }
  }
  cells = next;
}

fn play() -> string {
  if speed == 0 { return "Play"; }
  return "Pause";
}

every speed { step(); }

grid 20, cells { if cells[cell] > 0 { cells[cell] = 0; } else { cells[cell] = 6; } }
row {
  button "Step" { step(); }
  button play() { if speed == 0 { speed = 250; } else { speed = 0; } }
  button "Clear" { for i in 0..400 { cells[i] = 0; } }
}
