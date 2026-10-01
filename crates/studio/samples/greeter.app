// A greeter: type a name and it says hello.
state name = "";
state waves = 0;

label "What is your name?";
input name;
if name == "" {
  label "Type your name above.";
} else {
  label "Hello, " + name + "!";
  button "Wave" { waves = waves + 1; }
  if waves > 0 {
    label "You waved " + waves + " times.";
  }
}
