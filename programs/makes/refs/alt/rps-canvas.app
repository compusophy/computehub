// Rock Paper Scissors: tap Rock, Paper or Scissors; the computer picks at random and the
// hands and the result show on the board; the score of wins and losses is kept between runs.
// icon: ring 7 12 5 line 15 7 19 17 line 19 7 15 17
saved state wins = 0;
saved state losses = 0;
state draws = 0;
state pick = 0;            // 1 rock, 2 paper, 3 scissors; 0 none yet
state comp = 0;
state result = "";

fn name(p: int) -> string {
  if p == 1 { return "Rock"; }
  if p == 2 { return "Paper"; }
  return "Scissors";
}

// A hand: rock a fist, paper a sheet, scissors two blades.
fn hand(p: int, x: int) {
  if p == 1 { circle(x, 40, 12, 9); }
  if p == 2 { rect(x - 10, 28, 20, 24, 9); line(x - 10, 40, x + 10, 40, 1, 10); }
  if p == 3 {
    line(x - 10, 28, x + 2, 44, 2, 9);
    line(x + 10, 28, x - 2, 44, 2, 9);
    circle(x, 48, 2, 9);
  }
}

fn scene() {
  rect(0, 0, 160, 120, 0);
  text("You", 40, 10, 7, 10);
  text("Computer", 120, 10, 7, 10);
  if pick > 0 { hand(pick, 40); text(name(pick), 40, 66, 7, 10); }
  if comp > 0 { hand(comp, 120); text(name(comp), 120, 66, 7, 10); }
  if pick > 0 {
    text(result, 80, 88, 10, 11);
  } else {
    text("Pick one", 80, 88, 10, 10);
  }
  text("Wins " + wins + "   Losses " + losses + "   Draws " + draws, 80, 110, 6, 10);
}

// 1 rock, 2 paper, 3 scissors: paper beats rock, scissors beats paper, rock beats scissors.
fn beats(a: int, b: int) -> bool {
  if a == 2 && b == 1 { return true; }
  if a == 3 && b == 2 { return true; }
  if a == 1 && b == 3 { return true; }
  return false;
}

fn play(p: int) {
  pick = p;
  comp = 1 + random(3);
  if pick == comp {
    draws += 1;
    result = "Draw";
  } else if beats(pick, comp) {
    wins += 1;
    result = "You win";
  } else {
    losses += 1;
    result = "You lose";
  }
}

label "Rock Paper Scissors";
canvas 160, 120, scene();
row {
  button "Rock" { play(1); }
  button "Paper" { play(2); }
  button "Scissors" { play(3); }
}
