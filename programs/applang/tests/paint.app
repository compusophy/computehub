// Paint: tap or drag on the canvas to paint squares in the color picked under it. Kept.
// icon: fill 4 4 10 4 10 10 4 10 fill 14 14 20 14 20 20 14 20 ring 17 7 3 dot 7 17 3
saved state art = [-1; 1024];   // 32 x 32 squares, row by row: -1 blank, else a color
state pen = 1;                  // the color a tap paints; -1 erases

// The picture, 5 units a square, and under it the twelve colors, the pen's ringed in ink.
fn scene() {
  pixels(art, 0, 0, 32, 5);
  for c in 0..12 {
    if c == pen { rect(c * 13 + 1, 163, 13, 14, 9); }
    rect(c * 13 + 2, 164, 11, 12, c);
  }
}

fn paint(x: int, y: int) {
  if y < 160 {
    art[(y / 5) * 32 + x / 5] = pen;
  } else if y >= 162 {
    pen = min(11, max(0, (x - 2) / 13));
  }
}

label "Paint";
canvas 160, 180, scene() { paint(x, y); }
row {
  button "Eraser" { pen = -1; }
  button "Clear" { for i in 0..len(art) { art[i] = -1; } }
}
