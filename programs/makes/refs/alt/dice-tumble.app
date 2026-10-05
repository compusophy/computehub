// Dice: press Roll to throw a six-sided die; it tumbles, then settles on a face.
// icon: loop 4 4 20 4 20 20 4 20 dot 12 12 4
state face = 1;      // the face shown, 1 to 6
state rolls = 0;     // tumble steps left; 0 = settled
state speed = 0;     // ms a tumble step; 0 while the die rests

// The pips of a face: a square with one circle per pip, at the quarter points.
fn scene() {
  rect(20, 20, 80, 80, 7);
  if face == 1 || face == 3 || face == 5 { circle(60, 60, 8, 9); }
  if face >= 2 { circle(40, 40, 8, 9); circle(80, 80, 8, 9); }
  if face >= 4 { circle(40, 80, 8, 9); circle(80, 40, 8, 9); }
  if face == 6 { circle(40, 60, 8, 9); circle(80, 60, 8, 9); }
  if rolls > 0 { text("...", 60, 108, 8, 10); }
}

every speed {
  rolls -= 1;
  face = 1 + random(6);
  if rolls <= 0 { speed = 0; }
}

label "Dice";
canvas 120, 120, scene();
row {
  button "Roll" {
    rolls = 8;
    speed = 60;
  }
  label "You rolled " + face;
}
