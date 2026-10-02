// Breakout: bounce the ball off the paddle to break every brick. Drag the paddle, or press left
// and right.
state bricks = [1; 40];    // 8 a row, 5 rows; 0 broken
state px = 64;             // the paddle's left edge
state bx = 80;             // the ball, and its step
state by = 90;
state vx = 2;
state vy = -2;
state speed = 0;           // ms a step; 0 while nothing moves
state score = 0;
state lives = 3;
saved state best = 0;

fn scene() {
  for i in 0..40 {
    if bricks[i] == 1 { rect((i % 8) * 20 + 1, (i / 8) * 8 + 11, 18, 6, 1 + i / 8); }
  }
  rect(px, 112, 32, 4, 9);
  circle(bx, by, 2, 3);
  for i in 0..lives { circle(6 + i * 7, 4, 2, 1); }
  if speed == 0 && lives == 0 { text("Game over", 80, 75, 12, 9); }
  if speed == 0 && lives > 0 { text("Tap Start", 80, 75, 10, 10); }
}
fn steer(x: int) { px = min(128, max(0, x)); }
fn left() -> int {
  let n = 0;
  for i in 0..40 { n += bricks[i]; }
  return n;
}
fn serve() {
  bx = px + 16;
  by = 104;
  vx = 2;
  vy = -2;
}
fn start() {
  if lives == 0 {
    for i in 0..40 { bricks[i] = 1; }
    lives = 3;
    score = 0;
  }
  serve();
  speed = 33;
}

every speed {
  bx += vx;
  by += vy;
  if bx <= 2 || bx >= 157 { vx = -vx; }
  if by <= 2 { vy = -vy; }
  // A brick under the ball breaks, and the ball turns back.
  if by >= 11 && by < 51 {
    let i = ((by - 11) / 8) * 8 + min(7, max(0, bx / 20));
    if bricks[i] == 1 {
      bricks[i] = 0;
      score += 1;
      vy = -vy;
    }
  }
  if vy > 0 && by >= 110 && by <= 114 && bx >= px - 2 && bx <= px + 34 {
    vy = -vy;
    vx = max(-3, min(3, (bx - px - 16) / 5));
    if vx == 0 { vx = 1; }
  }
  if by > 120 {
    lives -= 1;
    speed = 0;
    best = max(best, score);
  }
  // All broken: a new wall.
  if left() == 0 {
    for i in 0..40 { bricks[i] = 1; }
    serve();
  }
}
on key "left" { steer(px - 8); }
on key "right" { steer(px + 8); }

label "Score " + score + "   Best " + best;
canvas 160, 120, scene() { steer(x - 16); }
if speed == 0 && lives > 0 { button "Start" { start(); } }
if lives == 0 { button "Play again" { start(); } }
