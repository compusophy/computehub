// Whack-a-mole: press Start, then tap the mole in 30 seconds; it hops every 800 ms. Score: N.
// icon: fill 2 2 22 2 22 22 2 22 dot 12 12 5
state mole = 4;            // the square the mole is in, 0 to 8
state score = 0;
state ticks = 0;           // 800 ms hops since Start; 37 of them is 30 s
state speed = 0;           // ms a hop; 0 rests before the game and after it
saved state best = 0;

// The board, 3 x 3 squares of 40 units on a 160 x 120 canvas, grass with the mole on its
// square; over it, its name until it starts, then Game over.
fn scene() {
  let cells = [2; 9];
  cells[mole] = 5;
  pixels(cells, 20, 0, 3, 40);
  sprite([".55.", "5555", "5995", "5555"], 20 + (mole % 3) * 40 + 4, (mole / 3) * 40 + 4, 8);
  text("Score: " + score, 80, 116, 8, 9);
  if speed == 0 && ticks == 0 { text("Whack-a-mole", 80, 55, 14, 9); }
  if speed == 0 && ticks > 0 { text("Game over", 80, 55, 14, 9); }
}

fn start() {
  score = 0;
  ticks = 0;
  mole = random(9);
  speed = 800;
}

every speed {
  ticks += 1;
  if ticks >= 37 {
    speed = 0;
    best = max(best, score);
  } else {
    mole = random(9);
  }
}

label "Whack-a-mole";
canvas 160, 120, scene() {
  let s = (y / 40) * 3 + (x - 20) / 40;
  if speed > 0 && x >= 20 && x < 140 && s == mole {
    score += 1;
    mole = random(9);
  }
}
row {
  if speed == 0 { button "Start" { start(); } }
  label "Best " + best;
}
