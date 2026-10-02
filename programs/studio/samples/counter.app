// A counter: two buttons change one number.
state count = 0;

label "Counter";
row {
  button "-" { count = count - 1; }
  label count;
  button "+" { count = count + 1; }
}
button "Reset" { count = 0; }
if count >= 10 {
  label "That is a lot of clicks.";
}
