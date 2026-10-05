// Clock: an analog clock from 12:00:00, its hands and the time as text.
// icon: ring 12 12 10 line 12 12 12 5 line 12 12 17 12
state t = 0;      // seconds since 12:00:00

fn two(n: int) -> string {
  if n < 10 { return "0" + n; }
  return "" + n;
}
fn hour() -> int {
  let h = (t / 3600) % 12;
  if h == 0 { return 12; }
  return h;
}
fn hand(deg: int, long: int, width: int, color: int) {
  line(60, 60, 60 + sin(deg) * long / 1000, 60 - cos(deg) * long / 1000, width, color);
}
fn scene() {
  ring(60, 60, 56, 2, 9);
  hand((t % 43200) / 120, 30, 4, 9);
  hand((t % 3600) / 10, 44, 3, 9);
  hand((t % 60) * 6, 50, 1, 1);
  text(hour() + ":" + two((t / 60) % 60) + ":" + two(t % 60), 60, 130, 10, 9);
}

every 1000 { t += 1; }

canvas 120, 140, scene();
