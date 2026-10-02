// Tetris: arrows move and rotate, space drops, full lines clear.
state cells = [0; 200];   // the landed blocks
state disp = [0; 200];    // landed plus the falling piece
state ptype = 0;
state prot = 0;
state px = 3;
state py = 0;
state score = 0;
state playing = false;
saved state best = 0;

fn bx(t: int, i: int) -> int {
  if t == 0 { if i == 0 { return 0; } if i == 1 { return 1; } if i == 2 { return 2; } return 3; }
  if t == 1 { if i == 0 { return 0; } if i == 1 { return 1; } if i == 2 { return 0; } return 1; }
  if t == 2 { if i == 0 { return 0; } if i == 1 { return 1; } if i == 2 { return 2; } return 1; }
  if t == 3 { if i == 0 { return 1; } if i == 1 { return 2; } if i == 2 { return 0; } return 1; }
  if t == 4 { if i == 0 { return 0; } if i == 1 { return 1; } if i == 2 { return 1; } return 2; }
  if t == 5 { if i == 0 { return 0; } if i == 1 { return 0; } if i == 2 { return 1; } return 2; }
  if i == 0 { return 2; } if i == 1 { return 0; } if i == 2 { return 1; } return 2;
}

fn by(t: int, i: int) -> int {
  if t == 0 { return 0; }
  if t == 1 { if i == 0 { return 0; } if i == 1 { return 0; } if i == 2 { return 1; } return 1; }
  if t == 2 { if i == 0 { return 0; } if i == 1 { return 0; } if i == 2 { return 0; } return 1; }
  if t == 3 { if i == 0 { return 0; } if i == 1 { return 0; } if i == 2 { return 1; } return 1; }
  if t == 4 { if i == 0 { return 0; } if i == 1 { return 0; } if i == 2 { return 1; } return 1; }
  if t == 5 { if i == 0 { return 0; } if i == 1 { return 1; } if i == 2 { return 1; } return 1; }
  if i == 0 { return 0; } if i == 1 { return 1; } if i == 2 { return 1; } return 1;
}

fn qx(t: int, r: int, i: int) -> int {
  let x = bx(t, i);
  let y = by(t, i);
  if r >= 1 { let nx = y; let ny = 0 - x; x = nx; y = ny; }
  if r >= 2 { let nx = y; let ny = 0 - x; x = nx; y = ny; }
  if r >= 3 { let nx = y; let ny = 0 - x; x = nx; y = ny; }
  return x;
}

fn qy(t: int, r: int, i: int) -> int {
  let x = bx(t, i);
  let y = by(t, i);
  if r >= 1 { let nx = y; let ny = 0 - x; x = nx; y = ny; }
  if r >= 2 { let nx = y; let ny = 0 - x; x = nx; y = ny; }
  if r >= 3 { let nx = y; let ny = 0 - x; x = nx; y = ny; }
  return y;
}

fn minx(t: int, r: int) -> int {
  let m = 99;
  for i in 0..4 { m = min(m, qx(t, r, i)); }
  return m;
}

fn miny(t: int, r: int) -> int {
  let m = 99;
  for i in 0..4 { m = min(m, qy(t, r, i)); }
  return m;
}

fn cx(t: int, r: int, i: int) -> int { return qx(t, r, i) - minx(t, r); }

fn cy(t: int, r: int, i: int) -> int { return qy(t, r, i) - miny(t, r); }

fn fits(t: int, r: int, ox: int, oy: int) -> bool {
  for i in 0..4 {
    let x = cx(t, r, i) + ox;
    let y = cy(t, r, i) + oy;
    if x < 0 || x >= 10 || y < 0 || y >= 20 { return false; }
    if cells[y * 10 + x] != 0 { return false; }
  }
  return true;
}

fn paint() {
  for i in 0..200 { disp[i] = cells[i]; }
  for i in 0..4 {
    disp[(py + cy(ptype, prot, i)) * 10 + px + cx(ptype, prot, i)] = ptype + 1;
  }
}

fn clearLines() {
  for y in 0..20 {
    let full = true;
    for x in 0..10 { if cells[y * 10 + x] == 0 { full = false; } }
    if full {
      score = score + 10;
      for k in 0..y {
        let s = y - k - 1;
        let d = y - k;
        for x in 0..10 { cells[d * 10 + x] = cells[s * 10 + x]; }
      }
      for x in 0..10 { cells[x] = 0; }
    }
  }
}

fn lock() {
  for i in 0..4 {
    cells[(py + cy(ptype, prot, i)) * 10 + px + cx(ptype, prot, i)] = ptype + 1;
  }
  clearLines();
}

fn spawn() {
  ptype = random(7);
  prot = 0;
  px = 3;
  py = 0;
  if !fits(ptype, prot, px, py) {
    playing = false;
    best = max(best, score);
  }
}

fn start() {
  for i in 0..200 { cells[i] = 0; }
  score = 0;
  playing = true;
  spawn();
  paint();
}

every 400 {
  if playing {
    if fits(ptype, prot, px, py + 1) {
      py = py + 1;
    } else {
      lock();
      spawn();
    }
    paint();
  }
}

on key "left" { if playing && fits(ptype, prot, px - 1, py) { px = px - 1; paint(); } }
on key "right" { if playing && fits(ptype, prot, px + 1, py) { px = px + 1; paint(); } }
on key "up" {
  if playing && fits(ptype, (prot + 1) % 4, px, py) { prot = (prot + 1) % 4; paint(); }
}
on key "down" { if playing && fits(ptype, prot, px, py + 1) { py = py + 1; paint(); } }
on key "space" {
  if playing {
    for i in 0..20 { if fits(ptype, prot, px, py + 1) { py = py + 1; } }
    lock();
    spawn();
    paint();
  }
}

label "Tetris";
label "Score " + score + "   Best " + best;
grid 10, disp;
if !playing {
  button "Start" { start(); }
}
