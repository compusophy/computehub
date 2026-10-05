// Snake: the arrows, or a tap toward a square, steer; eat the food; walls and your tail end it.
// icon: line 4 20 4 13 11 13 11 20 19 20 19 9 dot 19 6 3 dot 7 5 2
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

// Whether the body is on (x, y): the game asks its states, never what it drew.
fn body(x: int, y: int) -> bool {
  for i in 0..len(xs) {
    if xs[i] == x && ys[i] == y { return true; }
  }
  return false;
}

// The board, 20 x 15 squares of 8 units, made anew from what the game remembers; over it, its
// name until it starts, then Game over.
fn scene() {
  let cells = [-1; 300];
  for i in 0..len(xs) { cells[at(xs[i], ys[i])] = 2; }
  if len(xs) > 0 { cells[at(fx, fy)] = 1; }
  pixels(cells, 0, 0, 20, 8);
  if speed == 0 && len(xs) == 0 { text("Snake", 80, 60, 16, 11); }
  if speed == 0 && len(xs) > 0 { text("Game over", 80, 60, 16, 11); }
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
}

// A turn, taken at the next step; never straight back.
fn turn(x: int, y: int) {
  if x != -dx || y != -dy {
    nx = x;
    ny = y;
  }
}

// A tap at square (x, y) turns toward it, across or down, whichever is farther of the ways
// that are not straight back.
fn toward(x: int, y: int) {
  if len(xs) == 0 { return; }
  let ax = x - xs[len(xs) - 1];
  let ay = y - ys[len(ys) - 1];
  if ax * dx < 0 { ax = 0; }
  if ay * dy < 0 { ay = 0; }
  if abs(ax) > abs(ay) {
    turn(ax / abs(ax), 0);
  } else if ay != 0 {
    turn(0, ay / abs(ay));
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
  }
}

on key "left" { turn(-1, 0); }
on key "right" { turn(1, 0); }
on key "up" { turn(0, -1); }
on key "down" { turn(0, 1); }

label "Snake";
label "Length " + len(xs) + "   Best " + best;
canvas 160, 120, scene() { toward(x / 8, y / 8); }
if speed == 0 { button "Start" { start(); } }
