// 2048: slide the tiles with the arrows or the buttons; equal tiles merge.
// icon: loop 3 3 21 3 21 21 3 21 line 12 3 12 21 line 3 12 21 12
state b = [0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0];
state score = 0;
state over = false;
state moved = false;

fn spawn() {
  let free = 0;
  for i in 0..16 {
    if b[i] == 0 { free += 1; }
  }
  let k = random(max(1, free));
  for i in 0..16 {
    if b[i] == 0 {
      if k == 0 {
        if random(10) == 0 { b[i] = 4; } else { b[i] = 2; }
      }
      k -= 1;
    }
  }
}

// The square of a line that is k-th from where the tiles go, sliding d: 0 left, 1 right, 2 up,
// 3 down.
fn cell(d: int, ln: int, k: int) -> int {
  if d == 0 { return ln * 4 + k; }
  if d == 1 { return ln * 4 + 3 - k; }
  if d == 2 { return k * 4 + ln; }
  return (3 - k) * 4 + ln;
}

fn slide(d: int) {
  for ln in 0..4 {
    let vals = [0; 0];
    for k in 0..4 {
      let v = b[cell(d, ln, k)];
      if v != 0 { push(vals, v); }
    }
    let out = [0; 0];
    let k = 0;
    repeat 4 {
      if k < len(vals) {
        let pair = false;
        if k + 1 < len(vals) {
          if vals[k] == vals[k + 1] { pair = true; }
        }
        if pair {
          push(out, vals[k] * 2);
          score += vals[k] * 2;
          k += 2;
        } else {
          push(out, vals[k]);
          k += 1;
        }
      }
    }
    for j in 0..4 {
      let v = 0;
      if j < len(out) { v = out[j]; }
      let i = cell(d, ln, j);
      if b[i] != v {
        b[i] = v;
        moved = true;
      }
    }
  }
}

fn stuck() -> bool {
  for i in 0..16 {
    if b[i] == 0 { return false; }
    if i % 4 < 3 {
      if b[i] == b[i + 1] { return false; }
    }
    if i < 12 {
      if b[i] == b[i + 4] { return false; }
    }
  }
  return true;
}

fn go(d: int) {
  if over { return; }
  moved = false;
  slide(d);
  if moved { spawn(); }
  over = stuck();
}

fn start() {
  for i in 0..16 { b[i] = 0; }
  score = 0;
  over = false;
  spawn();
  spawn();
}

fn tint(v: int) -> int {
  if v == 0 { return 10; }
  if v == 2 { return 7; }
  if v == 4 { return 3; }
  if v == 8 { return 1; }
  if v == 16 { return 5; }
  if v == 32 { return 4; }
  if v == 64 { return 6; }
  return 2;
}

fn scene() {
  rect(0, 0, 160, 160, 8);
  for i in 0..16 {
    let px = (i % 4) * 40;
    let py = (i / 4) * 40;
    rect(px + 2, py + 2, 36, 36, tint(b[i]));
    if b[i] > 0 { text(b[i], px + 20, py + 20, 12, 9); }
  }
  if over { text("Game over", 80, 80, 16, 11); }
}

on key "left" { go(0); }
on key "right" { go(1); }
on key "up" { go(2); }
on key "down" { go(3); }

label "Score " + score;
canvas 160, 160, scene();
row {
  button "Left" { go(0); }
  button "Right" { go(1); }
  button "Up" { go(2); }
  button "Down" { go(3); }
}
button "New game" { start(); }
