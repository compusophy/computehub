// Counter drawn on a canvas: + adds one, - takes one away.
// icon: ring 12 12 9 line 8 12 16 12 line 12 8 12 16
state n = 0;

fn scene() {
  rect(0, 0, 100, 40, 0);
  text("Count", 50, 10, 8, 10);
  text(n, 50, 28, 14, 9);
}

canvas 100, 40, scene();
row {
  button "- 1" { n -= 1; }
  button "+ 1" { n += 1; }
}
