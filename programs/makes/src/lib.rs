//! Suite 1 of compusophyOS's evals, Studio makes: apps described precisely enough to be
//! checked, from tiny (a counter) to hard (tetris), each graded by a functional checker that
//! drives the app made headlessly ([`probe::Probe`]) and reads what it shows: its labels, its
//! canvas's texts and colors, its boards' squares, never the names inside it. A description says
//! what the checker looks for (a button labelled +, a 3 x 3 board, a time like 25:00); a checker
//! accepts every design the description allows (a board as a grid widget, pixels, a lattice of
//! rects or the whole canvas; a button or a key). [`judge`] grades a program: it compiles, runs
//! clean through the smoke test on 3 seeds, its icon line draws, and its checker passes. The
//! suite's [`hash`] covers its tasks, checkers and probe. `refs/` holds a program for each task
//! that passes it, and the tests break each one to show its checker catches the break. The
//! harness that makes the apps and keeps the records is `evals`.

#![forbid(unsafe_code)]

mod checks;
pub mod probe;
#[cfg(test)]
mod tests;

use checks::*;

/// A task's checker: `Ok`, or why the app fails it.
pub type Check = fn(&str) -> Result<(), String>;

/// One task: its id, size (tiny, small, medium, hard), what Studio is asked, and its checker.
#[derive(Clone, Copy, Debug)]
pub struct Task {
    pub id: &'static str,
    pub size: &'static str,
    pub ask: &'static str,
    pub check: Check,
}

/// The suite's id.
pub const ID: &str = "studio";

/// What the suite's hash covers: its tasks and checkers, and how they drive and read an app.
pub const SOURCES: [&str; 3] =
    [include_str!("lib.rs"), include_str!("checks.rs"), include_str!("probe.rs")];

const fn task(id: &'static str, size: &'static str, ask: &'static str, check: Check) -> Task {
    Task { id, size, ask, check }
}

/// The tasks, smallest first.
pub const TASKS: [Task; 24] = [
    task(
        "counter",
        "tiny",
        "a counter: a label shows the count, starting at 0; a button labelled + adds one and a \
         button labelled - takes one away",
        counter,
    ),
    task(
        "greeter",
        "tiny",
        "a greeter: a text input for a name and a label that greets it as it is typed, saying \
         Hello, then the name (Hello, Ada), with no button",
        greeter,
    ),
    task(
        "temperature",
        "small",
        "a Celsius to Fahrenheit converter: a text input for whole degrees Celsius and a label \
         that shows the Fahrenheit value as it is typed, F = C * 9 / 5 + 32 in whole numbers (37 \
         shows 98, -40 shows -40)",
        temperature,
    ),
    task(
        "tip",
        "small",
        "a tip calculator: a text input for the bill in whole dollars, buttons labelled 10%, 15% \
         and 20% that choose the tip, and labels showing the tip and the total in whole dollars, \
         rounded down (a bill of 200 at 15% shows a tip of 30 and a total of 230)",
        tip,
    ),
    task(
        "dice",
        "small",
        "a dice roller: a button labelled Roll throws a six-sided die at random; the face is \
         drawn on a canvas as a square with one circle per pip (no other circles), and a label \
         shows the number rolled",
        dice,
    ),
    task(
        "traffic",
        "small",
        "a traffic light on a canvas: three circles stacked top to bottom, the top red (1), the \
         middle yellow (3) and the bottom green (2) while lit, gray (8) while not, one lit at a \
         time; it starts green and cycles green 3 seconds, yellow 1 second, red 3 seconds, with \
         every; a button labelled Pause stops it and starts it again",
        traffic,
    ),
    task(
        "stopwatch",
        "small",
        "a stopwatch: buttons labelled Start, Stop and Reset, and the time shown as seconds and \
         tenths, like 12.3, counting with every 100; Stop keeps the time, Start goes on from it, \
         Reset sets it to 0.0",
        stopwatch,
    ),
    task(
        "rps",
        "small",
        "rock paper scissors against the computer: buttons labelled Rock, Paper and Scissors; the \
         computer picks at random; it shows Computer: and its pick, then You win, You lose or \
         Draw, and the score of wins and losses",
        rps,
    ),
    task(
        "guess",
        "small",
        "guess the number: the app picks a secret whole number from 1 to 100 at random; type a \
         guess in a text input and press a button labelled Guess; it says Too high, Too low or \
         Correct, and counts the guesses",
        guess,
    ),
    task(
        "poll",
        "small",
        "a poll: buttons labelled Cats, Dogs and Birds each add a vote; a bar chart on a canvas \
         shows one bar (a rect) per option, left to right in that order, standing on one \
         baseline, its height in proportion to its votes (twice the votes, twice as tall), with \
         each count written as text",
        poll,
    ),
    task(
        "tictactoe",
        "medium",
        "tic-tac-toe for two players on one device: a 3 x 3 board, a grid widget or a canvas the \
         board fills, where a tap on an empty square places X or O in turn, X first; it says X \
         wins, O wins or Draw when the game ends, and a button labelled New game empties the board",
        tictactoe,
    ),
    task(
        "connect4",
        "medium",
        "connect four for two players on one device: 7 columns and 6 rows, a grid widget or a \
         canvas the board fills; a tap anywhere in a column drops a disc of the player to move \
         (Red first, then Yellow) to the lowest empty square of that column; four in a row \
         across, down or diagonally wins and it says Red wins or Yellow wins; a button labelled \
         New game empties the board",
        connect4,
    ),
    task(
        "pomodoro",
        "medium",
        "a pomodoro timer: a 25 minute work countdown shown as minutes and seconds, 25:00, with \
         buttons labelled Start, Pause and Reset; at 0:00 it says Break and counts down 5:00, \
         then work again; it counts with every 1000",
        pomodoro,
    ),
    task(
        "clock",
        "medium",
        "an analog clock on a canvas: a ring for the face and hour, minute and second hands drawn \
         as lines from its center; there is no real clock, so it starts at 12:00:00 and moves \
         with every 1000; it also shows the time as text, like 12:00:05",
        clock,
    ),
    task(
        "calculator",
        "medium",
        "a calculator: buttons labelled 0 to 9, +, -, *, /, = and C, and a display showing what \
         is typed and then the result; whole numbers, / rounds toward zero, dividing by zero \
         shows Error, C clears; 12 + 7 = shows 19",
        calculator,
    ),
    task(
        "paint",
        "medium",
        "a paint app: a 32 x 32 board of squares drawn with pixels on a canvas; a tap or a drag \
         paints the square under it in the current color; a palette picks the color, either \
         buttons labelled Red, Green, Blue and Yellow or swatches of those colors (1, 2, 4 and \
         3) drawn on the canvas beside the board; it shows the current color, and a button \
         labelled Clear empties the board",
        paint,
    ),
    task(
        "drawpad",
        "medium",
        "a drawing pad: a 64 x 64 board drawn with pixels on a canvas, one unit a square; a tap \
         or a drag inks the square under it (ink is color 9); a button labelled Mirror turns \
         mirror mode on and off: while it is on, each square inked is also inked at the mirrored \
         place, left to right (x becomes 63 - x); a button labelled Clear wipes the board",
        drawpad,
    ),
    task(
        "whack",
        "medium",
        "whack-a-mole: a 3 x 3 board, a grid widget or a canvas the board fills; a button \
         labelled Start begins a 30 second game; every 800 ms the mole moves to a random square, \
         drawn unlike the empty squares; tapping the mole scores a point, shown as Score: N; at \
         the end it says Game over",
        whack,
    ),
    task(
        "bounce",
        "medium",
        "a bouncing ball: one circle moving across a 160 x 120 canvas with every 33, bouncing off \
         all four edges and never leaving it; a button labelled Faster makes it move faster",
        bounce,
    ),
    task(
        "life",
        "hard",
        "Conway's Game of Life on a 20 x 20 board, a grid widget or pixels on a canvas, its edges \
         not wrapping: a tap toggles a square alive or dead; a button labelled Step advances one \
         generation, Play runs it with every 200 and Pause stops it (or one button that toggles \
         Play and Pause), and Clear kills every square",
        life,
    ),
    task(
        "minesweeper",
        "hard",
        "minesweeper on a 9 x 9 board, a grid widget or a canvas the board fills, with 10 mines \
         placed at random: a tap reveals a square; a mine ends the game and it says Game over; \
         any other shows how many mines touch it, and a square that touches none reveals its \
         neighbours too; the first tap is never a mine; a button labelled Flag switches taps \
         between revealing and flagging; revealing every safe square says You win",
        minesweeper,
    ),
    task(
        "memory",
        "hard",
        "a memory game: 16 cards face down on a 4 x 4 board, a grid widget or a canvas the board \
         fills, hiding 8 pairs shuffled at random; a tap turns a card face up, showing its color \
         or symbol; two face-up cards that match stay up, two that do not turn face down again \
         at the next tap or after a second; it counts moves (each two cards turned is one) as \
         Moves: N and says You win when every pair is found",
        memory,
    ),
    task(
        "2048",
        "hard",
        "2048 on a 4 x 4 board on a canvas: each tile a square with its number written on it as \
         text, empty squares without a number; the arrow keys and buttons labelled Up, Down, Left \
         and Right slide the tiles; two equal tiles that meet merge into one of their sum; after \
         each move that changes the board a 2 or a 4 appears on a random empty square; it starts \
         with two tiles, shows the score in a label above the canvas and says Game over when no \
         move is left",
        game2048,
    ),
    task(
        "tetris",
        "hard",
        "tetris on a 10 x 20 board drawn with pixels on a canvas, 10 squares a row: a button \
         labelled Start begins; the piece falls with every; the arrow keys and buttons labelled \
         Left, Right, Rotate and Drop move, rotate and drop it; a full row clears and adds to the \
         score; it says Game over when a new piece cannot enter",
        tetris,
    ),
];

/// The task named `id`.
pub fn find(id: &str) -> Option<&'static Task> {
    TASKS.iter().find(|t| t.id == id)
}

/// How the program `src` fares at `task`: whether it passes, the first stage that failed (`ok`
/// if none: `compile`, `smoke`, `icon` or `check`) and why. It passes when it compiles, runs
/// clean through the smoke test on seeds 1 to 3 ([`coder::ai::fault`], as the coder checks it),
/// has an icon line that draws, and passes the task's checker.
pub fn judge(task: &Task, src: &str) -> (bool, &'static str, String) {
    let fail = |stage, why: String| (false, stage, coder::ai::clip(&why, 300));
    if let Some(f) = coder::ai::fault(src, "", 3) {
        let stage = if !f.compiles {
            "compile"
        } else if f.runs {
            "icon"
        } else {
            "smoke"
        };
        return fail(stage, f.said);
    }
    if icons::made::header(src.as_bytes()).is_none() {
        return fail("icon", "it has no icon line".into());
    }
    let checked = std::panic::catch_unwind(|| (task.check)(src));
    match checked.unwrap_or_else(|_| Err("the checker panicked".into())) {
        Ok(()) => (true, "ok", String::new()),
        Err(why) => fail("check", why),
    }
}

/// The suite's content hash, FNV-1a 64 of its id and the text of its tasks, checkers and probe
/// (line ends aside): any change to what is asked or how it is graded is a new suite.
pub fn hash() -> u64 {
    let text = [ID].iter().chain(&SOURCES).flat_map(|s| s.bytes()).filter(|&b| b != b'\r');
    text.fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

/// `h` as 16 hex digits.
pub fn hex(h: u64) -> String {
    (0..16).rev().map(|i| char::from(b"0123456789abcdef"[(h >> (i * 4) & 15) as usize])).collect()
}

/// The suite as `evals/suites/studio.jsonl` lists it: a line with its id, hash and size, then a
/// line per task: its id, size and ask.
pub fn listing() -> String {
    use coder::json::put;
    let mut o = String::from("{\"suite\":");
    put(&mut o, ID);
    o += ",\"hash\":";
    put(&mut o, &hex(hash()));
    o += ",\"tasks\":";
    coder::ai::put_num(&mut o, TASKS.len() as u64);
    o += "}\n";
    for t in &TASKS {
        o += "{\"task\":";
        put(&mut o, t.id);
        o += ",\"size\":";
        put(&mut o, t.size);
        o += ",\"ask\":";
        put(&mut o, t.ask);
        o += "}\n";
    }
    o
}
