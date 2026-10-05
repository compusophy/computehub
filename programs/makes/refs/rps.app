// Rock paper scissors: pick one; the computer picks at random.
// icon: ring 7 8 4 loop 13 4 21 4 21 12 13 12 line 4 20 10 14 line 4 14 10 20
state me = -1;
state them = -1;
state wins = 0;
state losses = 0;

fn name(i: int) -> string {
  if i == 0 { return "Rock"; }
  if i == 1 { return "Paper"; }
  return "Scissors";
}

fn result() -> string {
  if me < 0 { return "Pick one"; }
  if me == them { return "Draw"; }
  if (me + 3 - them) % 3 == 1 { return "You win"; }
  return "You lose";
}

fn play(i: int) {
  me = i;
  them = random(3);
  if result() == "You win" { wins += 1; }
  if result() == "You lose" { losses += 1; }
}

row {
  button "Rock" { play(0); }
  button "Paper" { play(1); }
  button "Scissors" { play(2); }
}
if me >= 0 { label "You: " + name(me) + "   Computer: " + name(them); }
label result();
label "Wins " + wins + "   Losses " + losses;
