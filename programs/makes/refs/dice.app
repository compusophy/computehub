// Dice: Roll throws a die; its face shows the pips.
// icon: loop 4 4 20 4 20 20 4 20 dot 8 8 2 dot 12 12 2 dot 16 16 2
state face = 1;

fn pip(on: bool, px: int, py: int) {
  if on { circle(px, py, 6, 9); }
}

fn scene() {
  rect(10, 10, 80, 80, 7);
  pip(face % 2 == 1, 50, 50);
  pip(face >= 2, 30, 30);
  pip(face >= 2, 70, 70);
  pip(face >= 4, 70, 30);
  pip(face >= 4, 30, 70);
  pip(face == 6, 30, 50);
  pip(face == 6, 70, 50);
}

label "Rolled " + face;
canvas 100, 100, scene();
button "Roll" { face = 1 + random(6); }
