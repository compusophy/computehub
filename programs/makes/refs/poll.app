// Poll: vote for cats, dogs or birds; the bars show the votes.
// icon: fill 4 20 8 20 8 12 4 12 fill 10 20 14 20 14 6 10 6 fill 16 20 20 20 20 14 16 14
state votes = [0, 0, 0];

fn most() -> int { return max(1, max(votes[0], max(votes[1], votes[2]))); }

fn bar(i: int) {
  let h = votes[i] * 80 / most();
  rect(20 + i * 50, 100 - h, 30, h, i + 1);
  text(votes[i], 35 + i * 50, 108, 8, 9);
}

fn scene() {
  for i in 0..3 { bar(i); }
  line(10, 100, 170, 100, 1, 10);
}

canvas 180, 116, scene();
row {
  button "Cats" { votes[0] += 1; }
  button "Dogs" { votes[1] += 1; }
  button "Birds" { votes[2] += 1; }
}
