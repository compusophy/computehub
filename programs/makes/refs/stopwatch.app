// Stopwatch: Start, Stop and Reset; seconds and tenths.
// icon: ring 12 13 8 line 12 13 12 8 line 10 3 14 3
state tenths = 0;
state speed = 0;

fn time() -> string { return (tenths / 10) + "." + (tenths % 10); }

every speed { tenths += 1; }

label time() + " s";
row {
  if speed == 0 { button "Start" { speed = 100; } } else { button "Stop" { speed = 0; } }
  button "Reset" { tenths = 0; }
}
