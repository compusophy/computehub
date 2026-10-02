// Catch: stars fall; drag the basket (or press left and right) to catch them. Three missed and
// the game is over.
state bx = 70;                               // the basket's left edge
state sx = [20, 60, 100, 140, 80];           // the stars: x, and y (above the top: waiting)
state sy = [-10, -40, -70, -100, -130];
state score = 0;
state missed = 0;
state speed = 0;                             // ms a step; 0 while nothing moves
saved state best = 0;

// A star: a sprite 5 units wide, centered on its place.
fn star(i: int) {
  if sy[i] >= 0 { sprite(["..3..", "33333", ".333.", "3...3"], sx[i] - 2, sy[i] - 2, 1); }
}

fn scene() {
  rect(0, 112, 160, 8, 2);
  for i in 0..len(sx) { star(i); }
  rect(bx, 104, 20, 6, 6);
  text("Score " + score, 30, 6, 6, 9);
  for i in 0..missed { circle(152 - i * 8, 6, 2, 1); }
  if speed == 0 { text("Catch the stars", 80, 50, 10, 11); }
}

fn steer(x: int) { bx = min(140, max(0, x)); }

// A star back above the top, somewhere new.
fn again(i: int) {
  sx[i] = 8 + random(144);
  sy[i] = -10 - random(60);
}

fn start() {
  score = 0;
  missed = 0;
  speed = 33;
  for i in 0..len(sx) { again(i); }
}

every speed {
  for i in 0..len(sx) {
    sy[i] += 1 + score / 10;
    if sy[i] >= 104 && sy[i] <= 110 && sx[i] >= bx && sx[i] < bx + 20 {
      score += 1;
      again(i);
    } else if sy[i] > 120 {
      missed += 1;
      again(i);
    }
  }
  if missed >= 3 {
    speed = 0;
    best = max(best, score);
  }
}
on key "left" { steer(bx - 6); }
on key "right" { steer(bx + 6); }

label "Catch the stars";
canvas 160, 120, scene() { steer(x - 10); }
row {
  if speed == 0 { button "Start" { start(); } }
  label "Best " + best;
}
