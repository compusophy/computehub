// Celsius to Fahrenheit: type whole degrees Celsius.
// icon: ring 12 6 3 line 12 9 12 17 dot 12 19 3
state c = "";

label "Celsius";
input c;
label parse(c, 0) * 9 / 5 + 32 + " F";
