// Bounce: a ball bounces around the canvas; Faster speeds it up.
// icon: ring 12 12 6 line 3 21 8 16 line 16 8 21 3
state bx = 80;
state by = 60;
state dx = 2;
state dy = 1;

every 33 {
  bx += dx;
  by += dy;
  if bx < 5 { bx = 5; dx = abs(dx); }
  if bx > 155 { bx = 155; dx = 0 - abs(dx); }
  if by < 5 { by = 5; dy = abs(dy); }
  if by > 115 { by = 115; dy = 0 - abs(dy); }
}

fn scene() { circle(bx, by, 5, 11); }

canvas 160, 120, scene();
button "Faster" {
  dx = dx + dx / abs(dx);
  dy = dy + dy / abs(dy);
}
