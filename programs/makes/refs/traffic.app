// Traffic light: green 3 s, yellow 1 s, red 3 s; Pause stops it and starts it again.
// icon: loop 8 2 16 2 16 22 8 22 dot 12 6 2 dot 12 12 2 dot 12 18 2
state t = 0;          // tenths of a second into the cycle
state speed = 100;

fn light(i: int, lit: bool, color: int) {
  if lit { circle(30, 20 + i * 30, 12, color); } else { circle(30, 20 + i * 30, 12, 8); }
}

fn scene() {
  rect(14, 4, 32, 92, 10);
  light(0, t >= 40, 1);
  light(1, t >= 30 && t < 40, 3);
  light(2, t < 30, 2);
}

every speed { t = (t + 1) % 70; }

canvas 60, 100, scene();
button "Pause" { if speed == 0 { speed = 100; } else { speed = 0; } }
