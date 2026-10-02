// Todo: type a task and press Add; tap its box to check it off; x removes it. Kept between runs.
saved state tasks = [""; 0];
saved state done = [false; 0];
state text = "";

fn box(on: bool) -> string {
  if on { return "[x]"; }
  return "[ ]";
}

fn left() -> int {
  let n = 0;
  for i in 0..len(done) {
    if !done[i] { n += 1; }
  }
  return n;
}

label "Todo";
row {
  input text;
  button "Add" {
    if text != "" {
      push(tasks, text);
      push(done, false);
      text = "";
    }
  }
}
for i in 0..len(tasks) {
  row {
    button box(done[i]) { done[i] = !done[i]; }
    label tasks[i];
    button "x" { remove(tasks, i); remove(done, i); }
  }
}
label left() + " left";
