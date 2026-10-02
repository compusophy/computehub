// Snake: the arrows steer; eat the food; the walls and your tail end it.
state cells = [0; 300];   // 20 x 15 squares
state xs = [0; 0];        // the body, head last
state ys = [0; 0];
state dx = 1;
state dy = 0;
state fx = 0;
state fy = 0;
state over = false;
saved state best = 0;

fn at(x: int, y: int) -> int { return y * 20 + x; }

fn paint() {
  for i in 0..len(cells) { cells[i] = 0; }
  for i in 0..len(xs) { cells[at(xs[i], ys[i])] = 2; }
  cells[at(fx, fy)] = 1;
}

fn start() {
  clear(xs);
  clear(ys);
  for i in 3..6 { push(xs, i); push(ys, 7); }
  dx = 1;
  dy = 0;
  fx = random(20);
  fy = random(15);
  over = false;
  paint();
}

every 150 {
  if !over && len(xs) > 0 {
    let hx = xs[len(xs) - 1] + dx;
    let hy = ys[len(ys) - 1] + dy;
    if hx < 0 || hx >= 20 || hy < 0 || hy >= 15 || cells[at(hx, hy)] == 2 {
      over = true;
      best = max(best, len(xs) - 3);
    } else {
      push(xs, hx);
      push(ys, hy);
      if hx == fx && hy == fy {
        fx = random(20);
        fy = random(15);
      } else {
        remove(xs, 0);
        remove(ys, 0);
      }
      paint();
    }
  }
}

on key "left" { if dx == 0 { dx = -1; dy = 0; } }
on key "right" { if dx == 0 { dx = 1; dy = 0; } }
on key "up" { if dy == 0 { dx = 0; dy = -1; } }
on key "down" { if dy == 0 { dx = 0; dy = 1; } }

label "Snake";
label "Length " + len(xs) + "   Best " + best;
grid 20, cells;
if over || len(xs) == 0 {
  button "Start" { start(); }
}
