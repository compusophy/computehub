// Memory: find the 8 pairs; two that differ turn back at the next tap.
// icon: loop 3 4 10 4 10 20 3 20 loop 14 4 21 4 21 20 14 20 dot 6 12 2 dot 17 12 2
state cards = [0; 16];    // each card's pair, 0 to 7
state up = [0; 16];       // 0 down, 1 up, 2 found
state first = -1;
state second = -1;
state moves = 0;
state dealt = false;

fn deal() {
  for i in 0..16 { cards[i] = i / 2; }
  for k in 0..16 {
    let i = 15 - k;
    let j = random(i + 1);
    let t = cards[i];
    cards[i] = cards[j];
    cards[j] = t;
  }
  for i in 0..16 { up[i] = 0; }
  first = -1;
  second = -1;
  moves = 0;
  dealt = true;
}

fn found() -> int {
  let n = 0;
  for i in 0..16 {
    if up[i] == 2 { n += 1; }
  }
  return n;
}

fn tap(i: int) {
  if !dealt { deal(); }
  if second >= 0 {
    up[first] = 0;
    up[second] = 0;
    first = -1;
    second = -1;
  }
  if up[i] != 0 { return; }
  up[i] = 1;
  if first < 0 {
    first = i;
    return;
  }
  moves += 1;
  if cards[i] == cards[first] {
    up[i] = 2;
    up[first] = 2;
    first = -1;
  } else {
    second = i;
  }
}

fn scene() {
  for i in 0..16 {
    let px = (i % 4) * 30;
    let py = (i / 4) * 30;
    if up[i] == 0 {
      rect(px + 1, py + 1, 28, 28, 10);
    } else {
      rect(px + 1, py + 1, 28, 28, 0);
      circle(px + 15, py + 15, 9, cards[i] + 1);
    }
  }
}

label "Moves: " + moves;
if found() == 16 { label "You win"; }
canvas 120, 120, scene() { tap((y / 30) * 4 + x / 30); }
button "New game" { deal(); }
