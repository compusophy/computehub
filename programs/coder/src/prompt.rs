//! The words of a make: one system prompt that never changes (so the provider's prefix cache
//! can hold it across turns and makes) and a fresh user message for each turn.

use crate::ai::{HONEST, ICON, SHORTER, put_numbered};

/// Who the model writes for.
const INTRO: &str = "You write apps for Studio, the app maker of compusophyOS, a desktop that runs \
                     in a browser tab. Apps are written in applang:\n\n";
/// How apps are made, after applang's card: what is seen is drawn on a canvas.
const RULES: &str = "\n\nWhat is to be seen (a game, a board, drawing, animation, a clock face, a \
                     chart) is drawn on a canvas, never spelled out in labels, buttons or a grid \
                     of words. A timer's or a count's digits may be a label. A game remembers \
                     its world in states and lists of its own (what \
                     has landed, where things are) and draws itself anew from them: pixels for \
                     a board of squares (tetris, snake, a painting, life), the other shapes for \
                     what moves freely or is a picture (a ball and paddles, hands, X and O); it \
                     moves with every, is steered with on key and with taps and drags on its \
                     canvas (a phone has no arrow keys; a drag taps each unit it crosses, so a \
                     tap steers, aims or paints, and turning, dropping or firing is a button), \
                     and shows Start until it runs. A board keeps its size and place as the \
                     widgets around it come and go, once each has shown: show the same ones \
                     before a game as after it (Start, hidden while it plays) and say Game over \
                     on the canvas. A paint app paints where a tap or drag goes, shows the color \
                     it paints (a swatch) and its squares' edges faintly under the paint (lines \
                     a unit wide in gray, 8). A grid's cell is the index of the square tapped, \
                     never what it holds: a palette reads its list, state pal = [1, 2, 3, 0]; \
                     grid 4, pal { color = pal[cell]; }. Saved states come back as they \
                     were kept, also after a change: a saved list keeps its old length (one a \
                     change adds starts as declared), so check a list's length before indexing \
                     it. Write small functions instead of repeating code; keep programs under \
                     200 lines. Decide quickly what applang can make, then write it: your reply \
                     has room for the program, not for long deliberation.\n\n";
/// What a reply holds.
const REPLIES: &str = "A new app: the complete program in one fenced block whose info string is \
                       app, and nothing else; after its first comment, a label naming the app.\n\
                       A fix or a change to a program shown with line numbers: edit blocks, and \
                       nothing else:\n<<<<<<< SEARCH\nlines copied exactly from the program, \
                       without their numbers\n=======\nthe lines to put there\n>>>>>>> REPLACE\n\
                       Each SEARCH matches one place only: quote whole lines, enough to be \
                       unique. If most of the program changes, reply with the complete program in \
                       one app block instead. If applang can make nothing close to what was \
                       asked, reply with only a // comment saying why.\n\n";

/// The system prompt: who the model writes for, applang's card, how apps are made, what a reply
/// holds, [`HONEST`], [`ICON`] and the example replies ([`applang::SHOTS`], each tested to compile and pass
/// the smoke test). Nothing in it varies: no model, path or date.
pub fn system() -> String {
    let mut out = [INTRO, applang::REFERENCE, RULES, REPLIES, HONEST, ICON].concat();
    for (i, (ask, src)) in applang::SHOTS.iter().enumerate() {
        let and = if i == 0 { "" } else { "\nAnd for " };
        out += &[and, "\"", ask, "\":\n```app\n", src, "```\n"].concat();
    }
    out
}

/// A first make's message.
pub fn write(ask: &str) -> String {
    let mut out = String::from("Make: ");
    out += ask;
    out
}

/// A change's message: the program it changes, numbered, and what to change.
pub fn change(ask: &str, base: &str) -> String {
    let mut out = String::from("The program, numbered:\n");
    put_numbered(&mut out, base, 1);
    out += "\nChange it: ";
    out += ask;
    out += "\nKeep its first comment true.";
    out
}

/// What was asked, the program numbered, `why` (what is wrong with it) and `then`: a fix's or a
/// rewrite's message.
fn shown(ask: &str, src: &str, why: &str, then: &str) -> String {
    let mut out = String::from("You were asked: ");
    out += ask;
    out += "\n\nThe program, numbered:\n";
    put_numbered(&mut out, src, 1);
    out.push('\n');
    out += why;
    out += then;
    out
}

/// A fix's message: what was asked, the program numbered and what is wrong with it.
pub fn fix(ask: &str, src: &str, account: &str) -> String {
    shown(ask, src, account, "\nReply with edit blocks.")
}

/// The one rewrite: what was asked, the program, what twice failed to clear (`why`).
pub fn rewrite(ask: &str, src: &str, why: &str) -> String {
    let again = "\nTwo tries did not clear this. Write the program again, simpler, as one complete \
                 app block.";
    shown(ask, src, why, again)
}

/// `asked` again, then `more`.
fn again(asked: &str, more: &[&str]) -> String {
    let mut out = String::from(asked);
    for m in more {
        out += m;
    }
    out
}

/// The message after edits that did not apply (`why`), in reply to `asked`.
pub fn missed(asked: &str, why: &str) -> String {
    let lead = "\n\nYour edits did not apply, so the program is unchanged: ";
    let copy = "\nCopy SEARCH lines exactly as the program has them, without their numbers.";
    again(asked, &[lead, why, copy])
}

/// A new app's message after a reply cut off before its program ended, or one that thought
/// past its budget.
pub fn shorter(first: &str) -> String {
    again(first, &["\n\n", SHORTER])
}

/// A change's message after a reply cut off before it ended (`asked` again): edits, never the
/// app cut down to fit.
pub fn edits_only(asked: &str) -> String {
    let only =
        "\n\nYour reply ran out of room. Reply with edit blocks only, never the whole program.";
    again(asked, &[only])
}

/// `asked` again after a reply that thought past its budget and wrote nothing.
pub fn rush(asked: &str) -> String {
    let rush = "\n\nYou thought past your room and wrote nothing. Decide quickly, then reply.";
    again(asked, &[rush])
}

/// The message after a reply that held neither a program nor edits.
pub fn format(asked: &str) -> String {
    let only = "\n\nYour reply held no app block and no edit blocks. Reply with one complete app \
                block, or with edit blocks, and nothing else.";
    again(asked, &[only])
}
