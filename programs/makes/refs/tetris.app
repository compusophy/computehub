// Tetris: the arrows or the buttons move and rotate, Drop drops; full rows clear.
// icon: fill 3 15 9 15 9 21 3 21 fill 9 15 15 15 15 21 9 21 fill 9 9 15 9 15 15 9 15 fill 15 15 21 15 21 21 15 21
state cells = [0; 200];   // the landed blocks
state disp = [0; 200];    // landed plus the falling piece
state ptype = 0;
state prot = 0;
state px = 3;
state py = 0;
state score = 0;
state playing = false;
state over = false;

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

// A block's place turned r quarter turns: x, then y.
fn qx(t: int, r: int, i: int) -> int {
  let qa = bx(t, i);
  let qb = by(t, i);
  for k in 0..r { let na = qb; let nb = 0 - qa; qa = na; qb = nb; }
  return qa;
}

fn qy(t: int, r: int, i: int) -> int {
  let qa = bx(t, i);
  let qb = by(t, i);
  for k in 0..r { let na = qb; let nb = 0 - qa; qa = na; qb = nb; }
  return qb;
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
    let qa = cx(t, r, i) + ox;
    let qb = cy(t, r, i) + oy;
    if qa < 0 || qa >= 10 || qb < 0 || qb >= 20 { return false; }
    if cells[qb * 10 + qa] != 0 { return false; }
  }
  return true;
}

fn paint() {
  for i in 0..200 { disp[i] = cells[i]; }
  if playing {
    for i in 0..4 {
      disp[(py + cy(ptype, prot, i)) * 10 + px + cx(ptype, prot, i)] = ptype + 1;
    }
  }
}

fn clearRows() {
  for ln in 0..20 {
    let full = true;
    for c in 0..10 {
      if cells[ln * 10 + c] == 0 { full = false; }
    }
    if full {
      score += 10;
      for k in 0..ln {
        let s = ln - k - 1;
        for c in 0..10 { cells[(s + 1) * 10 + c] = cells[s * 10 + c]; }
      }
      for c in 0..10 { cells[c] = 0; }
    }
  }
}

fn lock() {
  for i in 0..4 {
    cells[(py + cy(ptype, prot, i)) * 10 + px + cx(ptype, prot, i)] = ptype + 1;
  }
  clearRows();
}

fn spawn() {
  ptype = random(7);
  prot = 0;
  px = 3;
  py = 0;
  if !fits(ptype, prot, px, py) {
    playing = false;
    over = true;
  }
}

fn start() {
  for i in 0..200 { cells[i] = 0; }
  score = 0;
  playing = true;
  over = false;
  spawn();
  paint();
}

fn left() { if playing && fits(ptype, prot, px - 1, py) { px -= 1; paint(); } }
fn right() { if playing && fits(ptype, prot, px + 1, py) { px += 1; paint(); } }
fn turn() {
  if playing && fits(ptype, (prot + 1) % 4, px, py) {
    prot = (prot + 1) % 4;
    paint();
  }
}
fn drop() {
  if playing {
    for i in 0..20 {
      if fits(ptype, prot, px, py + 1) { py += 1; }
    }
    lock();
    spawn();
    paint();
  }
}

every 400 {
  if playing {
    if fits(ptype, prot, px, py + 1) {
      py += 1;
    } else {
      lock();
      spawn();
    }
    paint();
  }
}

on key "left" { left(); }
on key "right" { right(); }
on key "up" { turn(); }
on key "down" { if playing && fits(ptype, prot, px, py + 1) { py += 1; paint(); } }
on key "space" { drop(); }

fn scene() {
  pixels(disp, 0, 0, 10, 8);
  if over { text("Game over", 40, 80, 12, 11); }
}

label "Score " + score;
canvas 80, 160, scene();
if playing {
  row {
    button "Left" { left(); }
    button "Rotate" { turn(); }
    button "Right" { right(); }
    button "Drop" { drop(); }
  }
} else {
  button "Start" { start(); }
}
