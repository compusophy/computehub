// Calculator: whole numbers; + - * / and =; C clears.
// icon: loop 5 2 19 2 19 22 5 22 line 8 6 16 6 dot 9 12 2 dot 15 12 2 dot 9 18 2 dot 15 18 2
state shown = "0";
state acc = 0;
state op = "";
state fresh = true;
state err = false;

fn big(a: int) -> bool { return a > 999999999 || a < -999999999; }

fn apply(a: int, b: int) -> int {
  if op == "+" { return a + b; }
  if op == "-" { return a - b; }
  if op == "*" { return a * b; }
  if op == "/" { return a / b; }
  return b;
}

fn digit(d: int) {
  if err { return; }
  if fresh || shown == "0" {
    shown = "" + d;
  } else if len(shown) < 9 {
    shown = shown + d;
  }
  fresh = false;
}

fn equals() {
  if err { return; }
  let b = parse(shown, 0);
  if (op == "/" && b == 0) || big(acc) || big(b) {
    err = true;
    shown = "Error";
    return;
  }
  acc = apply(acc, b);
  shown = "" + acc;
  op = "";
  fresh = true;
}

fn oper(o: string) {
  if err { return; }
  if op != "" && !fresh { equals(); } else { acc = parse(shown, 0); }
  op = o;
  fresh = true;
}

fn wipe() {
  shown = "0";
  acc = 0;
  op = "";
  fresh = true;
  err = false;
}

label shown;
row { button "7" { digit(7); } button "8" { digit(8); } button "9" { digit(9); } button "/" { oper("/"); } }
row { button "4" { digit(4); } button "5" { digit(5); } button "6" { digit(6); } button "*" { oper("*"); } }
row { button "1" { digit(1); } button "2" { digit(2); } button "3" { digit(3); } button "-" { oper("-"); } }
row { button "0" { digit(0); } button "C" { wipe(); } button "=" { equals(); } button "+" { oper("+"); } }
