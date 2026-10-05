// Memory on a grid widget: two cards that differ turn back after a second.
// icon: loop 3 4 10 4 10 20 3 20 loop 14 4 21 4 21 20 14 20 dot 6 12 2 dot 17 12 2
state cards = [0; 16];
state faces = [0; 16];     // what the grid shows: 0 face down, else the card's color
state found = [false; 16];
state first = -1;
state second = -1;
state moves = 0;
state pairs = 0;
state speed = 0;

fn deal() {
  for i in 0..16 { cards[i] = 1 + i / 2; }
  for k in 0..16 {
    let i = 15 - k;
    let j = random(i + 1);
    let t = cards[i];
    cards[i] = cards[j];
    cards[j] = t;
  }
  for i in 0..16 {
    faces[i] = 0;
    found[i] = false;
  }
  first = -1;
  second = -1;
  moves = 0;
  pairs = 0;
  speed = 0;
}

every speed {
  faces[first] = 0;
  faces[second] = 0;
  first = -1;
  second = -1;
  speed = 0;
}

label "Moves: " + moves;
if pairs == 8 { label "You win!"; }
grid 4, faces {
  if speed == 0 && !found[cell] && cell != first {
    if cards[0] == 0 { deal(); }
    faces[cell] = cards[cell];
    if first < 0 {
      first = cell;
    } else {
      moves += 1;
      if cards[cell] == cards[first] {
        found[cell] = true;
        found[first] = true;
        pairs += 1;
        first = -1;
      } else {
        second = cell;
        speed = 1000;
      }
    }
  }
}
button "Start" { deal(); }
