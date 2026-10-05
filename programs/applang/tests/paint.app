// Paint: tap or drag on the canvas to paint squares in the color picked under it. Kept.
// icon: fill 4 4 10 4 10 10 4 10 fill 14 14 20 14 20 20 14 20 ring 17 7 3 dot 7 17 3
saved state art = [-1; 1024];   // 32 x 32 squares, row by row: -1 blank, else a color
state pen = 1;                  // the color a tap paints; -1 erases

// The squares' edges, faint (gray) under the paint; the picture, 5 units a square; under it the
// twelve colors, the pen's ringed in ink, and a swatch of what a tap paints.
fn scene() {
  for i in 1..32 {
    line(i * 5, 0, i * 5, 159, 1, 8);
    line(0, i * 5, 159, i * 5, 1, 8);
  }
  pixels(art, 0, 0, 32, 5);
  for c in 0..12 {
    if c == pen { rect(c * 11 + 1, 163, 11, 14, 9); }
    rect(c * 11 + 2, 164, 9, 12, c);
  }
  rect(136, 163, 22, 14, 9);
  rect(137, 164, 20, 12, max(0, pen));
}

fn paint(x: int, y: int) {
  if y < 160 {
    art[(y / 5) * 32 + x / 5] = pen;
  } else if y >= 162 && x < 134 {
    pen = min(11, max(0, (x - 2) / 11));
  }
}

label "Paint";
canvas 160, 180, scene() { paint(x, y); }
row {
  button "Eraser" { pen = -1; }
  button "Clear" { for i in 0..len(art) { art[i] = -1; } }
}
