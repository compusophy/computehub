// Counter: + adds one, - takes one away.
// icon: ring 12 12 9 line 8 12 16 12 line 12 8 12 16
state n = 0;

label "Count " + n;
row {
  button "+" { n += 1; }
  button "-" { n -= 1; }
}
