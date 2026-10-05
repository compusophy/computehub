// Guess the number: from 1 to 100; it says too high or too low.
// icon: ring 12 9 6 line 12 15 12 18 dot 12 20 2
state secret = 0;
state guess = "";
state tries = 0;
state said = "Guess a number from 1 to 100";

fn check() {
  if secret == 0 { secret = 1 + random(100); }
  let g = parse(guess, 0);
  tries += 1;
  if g > secret {
    said = "Too high";
  } else if g < secret {
    said = "Too low";
  } else {
    said = "Correct, in " + tries + " guesses";
    secret = 0;
    tries = 0;
  }
}

input guess;
button "Guess" { check(); }
label said;
label "Guesses " + tries;
