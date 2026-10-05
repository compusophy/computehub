// Clock: a face with hour, minute and second hands, from 12:00:00, one second every 1000 ms.
// icon: ring 12 12 9 line 12 12 12 6 line 12 12 17 12 line 12 12 14 14
state sec = 0;              // seconds since 12:00:00
state run = false;

fn two(n: int) -> string {
  if n < 10 { return "0" + n; }
  return "" + n;
}

// The time as text, like 12:00:05.
fn clock() -> string {
  let h = (sec / 3600) % 12;
  if h == 0 { h = 12; }
  return h + ":" + two((sec / 60) % 60) + ":" + two(sec % 60);
}

// A hand's end: d degrees clockwise from 12, r units out from the center.
fn hx(d: int, r: int) -> int { return 80 + r * sin(d) / 1000; }
fn hy(d: int, r: int) -> int { return 60 - r * cos(d) / 1000; }

fn scene() {
  ring(80, 60, 50, 2, 9);
  for i in 0..12 { line(hx(i * 30, 44), hy(i * 30, 44), hx(i * 30, 50), hy(i * 30, 50), 1, 10); }
  line(80, 60, hx(sec / 120, 28), hy(sec / 120, 28), 3, 9);        // hour
  line(80, 60, hx(sec / 10, 40), hy(sec / 10, 40), 2, 9);          // minute
  line(80, 60, hx(sec * 6, 45), hy(sec * 6, 45), 1, 1);            // second
  circle(80, 60, 2, 9);
  text(clock(), 80, 116, 7, 9);
}

every 1000 {
  if run { sec += 1; }
}

label "Clock";
canvas 160, 120, scene();
row {
  if run { button "Stop" { run = false; } }
  if !run { button "Start" { run = true; } }
  label clock();
}
