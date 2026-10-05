// Pomodoro: 25 minutes of work, then a 5 minute break.
// icon: ring 12 13 8 line 12 13 12 8 line 12 13 16 15 line 9 3 15 3
state left = 1500;      // seconds
state rest = false;
state speed = 0;

fn two(n: int) -> string {
  if n < 10 { return "0" + n; }
  return "" + n;
}
fn clock() -> string { return (left / 60) + ":" + two(left % 60); }
fn word() -> string {
  if rest { return "Break"; }
  return "Work";
}

every speed {
  left -= 1;
  if left <= 0 {
    rest = !rest;
    if rest { left = 300; } else { left = 1500; }
  }
}

label word();
label clock();
row {
  if speed == 0 { button "Start" { speed = 1000; } } else { button "Pause" { speed = 0; } }
  button "Reset" { speed = 0; rest = false; left = 1500; }
}
