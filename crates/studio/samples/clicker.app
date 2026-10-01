// Clicker: mine coins, then spend them on a better pick.
state coins = 0;
state power = 1;
state cost = 10;
state swings = 0;

label "Coins: " + coins;
label "Pick power: " + power;
row {
  button "Mine" {
    coins = coins + power;
    swings = swings + 1;
  }
  button "Upgrade" {
    if coins >= cost {
      coins = coins - cost;
      power = power + 1;
      cost = cost * 2;
    }
  }
}
label "Next upgrade: " + cost + " coins";
if swings >= 50 { label "Seasoned miner: " + swings + " swings"; }
