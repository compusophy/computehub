// Blocks: tetris on a canvas. Arrows, or taps at its sides, middle and bottom: move, turn, drop.
// icon: fill 3 13 9 13 9 19 3 19 fill 9 13 15 13 15 19 9 19 fill 9 7 15 7 15 13 9 13
state cells = [-1; 200];  // the landed squares, 10 x 20: -1 empty, else a color
state ptype = 0;
state prot = 0;
state px = 3;
state py = 0;
state score = 0;
state speed = 0;          // ms a step; 0 while nothing falls
saved state best = 0;
// The seven pieces unturned, four squares each: square i of piece t is column bx[t * 4 + i],
// row by[t * 4 + i].
state bx = [0, 1, 2, 3, 0, 1, 0, 1, 0, 1, 2, 1, 1, 2, 0, 1, 0, 1, 1, 2, 0, 0, 1, 2, 2, 0, 1, 2];
state by = [0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 1, 1, 1, 0, 1, 1, 1];

// Square i of piece t turned r quarters, as an offset from its corner: column (`across`) or row.
fn turned(t: int, r: int, i: int, across: bool) -> int {
  let least = 99;
  let mine = 0;
  for k in 0..4 {
    let x = bx[t * 4 + k];
    let y = by[t * 4 + k];
    for q in 0..r { let nx = y; y = 0 - x; x = nx; }
    let v = y;
    if across { v = x; }
    least = min(least, v);
    if k == i { mine = v; }
  }
  return mine - least;
}

fn fits(t: int, r: int, ox: int, oy: int) -> bool {
  for i in 0..4 {
    let x = turned(t, r, i, true) + ox;
    let y = turned(t, r, i, false) + oy;
    if x < 0 || x >= 10 || y < 0 || y >= 20 { return false; }
    if cells[y * 10 + x] != -1 { return false; }
  }
  return true;
}

// The landed squares, the falling piece over them, the score.
fn scene() {
  let b = cells;
  if speed > 0 {
    for i in 0..4 {
      b[(py + turned(ptype, prot, i, false)) * 10 + px + turned(ptype, prot, i, true)] = ptype + 1;
    }
  }
  pixels(b, 0, 0, 10, 8);
  text(score, 40, 8, 8, 9);
  if speed == 0 { text("Blocks", 40, 70, 12, 11); }
}

fn land() {
  for i in 0..4 {
    cells[(py + turned(ptype, prot, i, false)) * 10 + px + turned(ptype, prot, i, true)] = ptype + 1;
  }
  for y in 0..20 {
    let full = true;
    for x in 0..10 { if cells[y * 10 + x] == -1 { full = false; } }
    if full {
      score += 10;
      for k in 0..y {
        for x in 0..10 { cells[(y - k) * 10 + x] = cells[(y - k - 1) * 10 + x]; }
      }
      for x in 0..10 { cells[x] = -1; }
    }
  }
}

fn spawn() {
  ptype = random(7);
  prot = 0;
  px = 3;
  py = 0;
  if !fits(ptype, prot, px, py) {
    speed = 0;
    best = max(best, score);
  }
}

fn start() {
  for i in 0..200 { cells[i] = -1; }
  score = 0;
  speed = 400;
  spawn();
}

fn move(dx: int) { if speed > 0 && fits(ptype, prot, px + dx, py) { px += dx; } }
fn turn() { if speed > 0 && fits(ptype, (prot + 1) % 4, px, py) { prot = (prot + 1) % 4; } }
fn drop() {
  if speed > 0 {
    for i in 0..20 { if fits(ptype, prot, px, py + 1) { py += 1; } }
    land();
    spawn();
  }
}

every speed {
  if fits(ptype, prot, px, py + 1) {
    py += 1;
  } else {
    land();
    spawn();
  }
}

on key "left" { move(-1); }
on key "right" { move(1); }
on key "up" { turn(); }
on key "space" { drop(); }

label "Blocks";
label "Best " + best;
canvas 80, 160, scene() {
  if y >= 128 { drop(); } else if x < 27 { move(-1); } else if x >= 53 { move(1); } else { turn(); }
}
if speed == 0 { button "Start" { start(); } }
