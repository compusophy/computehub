// Whack-a-mole: Start, then tap the mole before it moves; 30 seconds a game.
// icon: ring 12 14 7 dot 9 12 2 dot 15 12 2 line 3 21 21 21
state mole = -1;
state score = 0;
state ticks = 0;
state speed = 0;
state over = false;

every speed {
  ticks += 1;
  if ticks % 8 == 0 { mole = random(9); }
  if ticks >= 300 {
    speed = 0;
    mole = -1;
    over = true;
  }
}

fn start() {
  score = 0;
  ticks = 0;
  speed = 100;
  over = false;
  mole = random(9);
}

fn scene() {
  for i in 0..9 {
    let c = 8;
    if i == mole { c = 5; }
    rect((i % 3) * 40 + 2, (i / 3) * 40 + 2, 36, 36, c);
  }
  if over { text("Game over", 60, 60, 14, 9); }
}

label "Score: " + score;
canvas 120, 120, scene() {
  let i = (y / 40) * 3 + x / 40;
  if i == mole && speed > 0 {
    score += 1;
    mole = -1;
  }
}
if speed == 0 { button "Start" { start(); } }
