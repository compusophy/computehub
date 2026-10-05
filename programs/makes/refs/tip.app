// Tip: type the bill and pick a tip; the tip and the total in whole dollars.
// icon: ring 12 12 9 line 12 6 12 18 line 9 9 15 9 line 9 15 15 15
state bill = "";
state rate = 15;

fn tip() -> int { return parse(bill, 0) * rate / 100; }

input bill;
row {
  button "10%" { rate = 10; }
  button "15%" { rate = 15; }
  button "20%" { rate = 20; }
}
label "Tip " + tip();
label "Total " + (parse(bill, 0) + tip());
