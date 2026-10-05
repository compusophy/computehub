// Drawing pad: tap or drag to ink; Mirror inks both halves; Clear wipes it.
// icon: line 4 20 10 8 14 14 20 4 ring 18 18 3
state art = [-1; 4096];
state mirror = false;

fn ink(px: int, py: int) {
  art[py * 64 + px] = 9;
  if mirror { art[py * 64 + 63 - px] = 9; }
}

fn scene() {
  rect(0, 0, 64, 64, 0);
  pixels(art, 0, 0, 64, 1);
}

fn mode() -> string {
  if mirror { return "Mirror on"; }
  return "Mirror off";
}

label mode();
canvas 64, 64, scene() { ink(x, y); }
row {
  button "Mirror" { mirror = !mirror; }
  button "Clear" { for i in 0..4096 { art[i] = -1; } }
}
