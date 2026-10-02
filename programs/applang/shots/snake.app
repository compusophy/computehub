// Snake: the arrows or the buttons steer; eat the food; the walls and your tail end it.
state cells = [0; 300];   // what the grid shows, 20 x 15: drawn anew from the states below
state xs = [0; 0];        // the body, head last
state ys = [0; 0];
state dx = 1;             // the way it moves
state dy = 0;
state nx = 1;             // the way its next step goes: a key waits for the step
state ny = 0;
state fx = 0;
state fy = 0;
state speed = 0;          // ms a step; 0 stops the timer while nothing moves
saved state best = 0;

fn at(x: int, y: int) -> int { return y * 20 + x; }

// Whether the body is on (x, y): the game asks its states, never the grid.
fn body(x: int, y: int) -> bool {
  for i in 0..len(xs) {
    if xs[i] == x && ys[i] == y { return true; }
  }
  return false;
}

// The squares from what the game remembers: the body and the food.
fn paint() {
  for i in 0..len(cells) { cells[i] = 0; }
  for i in 0..len(xs) { cells[at(xs[i], ys[i])] = 2; }
  cells[at(fx, fy)] = 1;
}

// Food on a free square: the first one from a random place on.
fn food() {
  let k = random(300);
  for j in 0..300 {
    let i = (k + j) % 300;
    if !body(i % 20, i / 20) {
      fx = i % 20;
      fy = i / 20;
      return;
    }
  }
}

fn start() {
  clear(xs);
  clear(ys);
  for i in 3..6 { push(xs, i); push(ys, 7); }
  dx = 1;
  dy = 0;
  nx = 1;
  ny = 0;
  food();
  speed = 150;
  paint();
}

// A turn, taken at the next step; never straight back.
fn turn(x: int, y: int) {
  if x != -dx || y != -dy {
    nx = x;
    ny = y;
  }
}

every speed {
  dx = nx;
  dy = ny;
  let hx = xs[len(xs) - 1] + dx;
  let hy = ys[len(ys) - 1] + dy;
  if hx < 0 || hx >= 20 || hy < 0 || hy >= 15 || body(hx, hy) {
    speed = 0;
    best = max(best, len(xs) - 3);
  } else {
    push(xs, hx);
    push(ys, hy);
    if hx == fx && hy == fy {
      food();
    } else {
      remove(xs, 0);
      remove(ys, 0);
    }
    paint();
  }
}

on key "left" { turn(-1, 0); }
on key "right" { turn(1, 0); }
on key "up" { turn(0, -1); }
on key "down" { turn(0, 1); }

label "Snake";
label "Length " + len(xs) + "   Best " + best;
grid 20, cells;
row {
  button "Left" { turn(-1, 0); }
  button "Up" { turn(0, -1); }
  button "Down" { turn(0, 1); }
  button "Right" { turn(1, 0); }
}
if speed == 0 {
  if len(xs) > 0 { label "Game over"; }
  button "Start" { start(); }
}
