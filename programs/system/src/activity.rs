//! Activity: what the desktop is doing now, in one honest word, and what each program uses.

use icons::Glyph;
use uiwire::stat::{self, Proc, Stats};
use uiwire::{Event, Frame, Key, Node, Request, SIGIL, Style, Variant};

use crate::{Disk, View, space, text};

/// Node ids: back to the list, End, the files row, the desktop's row; a process's row is
/// `ROW` plus its pid.
pub(crate) const BACK: u32 = 1;
pub(crate) const END: u32 = 2;
pub(crate) const FILES: u32 = 3;
pub(crate) const DESKTOP: u32 = 4;
pub(crate) const ROW: u32 = 16;
/// What the browser keeps of a page's storage, about: the storage meter's whole.
const KEPT: u64 = 5_000_000;
/// The storage meter's squares.
const SQUARES: u64 = 24;
/// The longest gap between two samples that rates are taken over, in ms: a busy page's timer may
/// fire late on the second between looks; a rate never averages over a long silence.
const RATE_MS: u32 = 3000;
/// The system apps and Studio's and the Assistant's tiles by name, as the desktop draws them.
#[rustfmt::skip]
const TILES: [(&str, &str, Glyph, u32); 7] = [
    ("about", "About", Glyph::About, 0xfbbf24), ("feedback", "Feedback", Glyph::Bug, 0x34d399),
    ("files", "Files", Glyph::Folder, 0x60a5fa), ("welcome", "Welcome", Glyph::Mark, 0xf472b6),
    ("activity", "Activity", Glyph::Pulse, 0x22d3ee),
    ("assistant", "Assistant", Glyph::Assistant, 0xa78bfa),
    ("studio", "Studio", Glyph::Studio, 0x8b7bff),
];
const TERMINAL: (Glyph, u32) = (Glyph::Terminal, 0x2dd4bf);
const DESKTOP_TILE: (Glyph, u32) = (Glyph::Mark, 0x94a3b8);
const DESKTOP_DOES: &str = "The desktop runs the home screen, the windows, Terminal and Settings, \
and serves programs\u{2019} files. Reloading the page restarts it.";
const FOOTER: &str = "Percents are of one core. Memory is wasm memory, which never shrinks; the \
browser\u{2019}s own isn\u{2019}t shown.";
const DOT: &str = " \u{b7} ";

/// Activity: the word for the whole machine (Busy, Drawing, Resting, Still) and why; then what
/// runs (the desktop first, each program by its command line, this window last) with its state,
/// its share of a core and its memory; your files against what the browser keeps; the AI since
/// the tab opened. A row opens its page, which can End the process (two deliberate taps; Back
/// or Escape returns). It watches the desktop's meters from its first size
/// ([`Request::Watch`]): the desktop sends a sample only when something changed, at most once a
/// second, so a still desktop wakes nothing here; and it draws only what changed. Rates come
/// from the last two samples, when they are at most 3 s apart; what it cannot know reads `—`.
#[derive(Debug, Default)]
pub struct Activity {
    /// The last two samples, the newer last.
    pub(crate) prev: Option<Stats>,
    pub(crate) cur: Option<Stats>,
    /// The page shown: the desktop's (0) or a process's; none, the list.
    pub(crate) page: Option<u32>,
    /// Processes ended from here, left out until the desktop no longer lists them.
    ended: Vec<u32>,
    /// The nodes last framed, the requests since, whether a frame went yet.
    shown: Vec<Node>,
    requests: Vec<Request>,
    framed: bool,
}

/// A process as a row says it: its name, tile, where it runs, its share of a core, its memory.
struct Row<'a> {
    p: &'a Proc,
    name: String,
    tile: (u8, u32),
    terminal: bool,
    cpu: Option<u32>,
    mem: Option<u32>,
}

impl View for Activity {
    fn event(&mut self, ev: &Event, _: &mut dyn Disk) -> bool {
        match ev {
            Event::Resize { .. } if !self.framed => self.requests.push(Request::Watch { on: true }),
            Event::Stats { data } => {
                if let Some(s) = Stats::decode(data) {
                    self.ended.retain(|&pid| s.procs.iter().any(|p| p.pid == pid));
                    self.prev = self.cur.replace(s);
                }
            }
            Event::Click { id: BACK } | Event::Key { key: Key::Escape, .. } => self.page = None,
            Event::Click { id: FILES } => {
                self.requests.push(Request::Open { name: "files".into() })
            }
            Event::Click { id: DESKTOP } => self.page = Some(0),
            Event::Click { id: END } => {
                if let Some(pid) = self.page.filter(|&p| p != 0) {
                    self.requests.push(Request::End { pid });
                    self.ended.push(pid);
                    self.page = None;
                }
            }
            Event::Click { id } if *id > ROW => self.page = Some(id - ROW),
            _ => {}
        }
        // A page whose process is gone shows the list.
        let rows = self.rows();
        if self.page.is_some_and(|pid| pid != 0 && !rows.iter().any(|r| r.p.pid == pid)) {
            self.page = None;
        }
        let nodes = self.nodes();
        let changed = !self.framed || !self.requests.is_empty() || nodes != self.shown;
        self.shown = nodes;
        changed
    }

    fn frame(&mut self) -> Frame {
        self.framed = true;
        let requests = std::mem::take(&mut self.requests);
        Frame { seq: 0, title: "Activity".into(), requests, nodes: self.shown.clone() }
    }
}

impl Activity {
    /// The ms between the last two samples, if rates may be taken over them.
    fn dt(&self) -> Option<u32> {
        let (prev, cur) = (self.prev.as_ref()?, self.cur.as_ref()?);
        Some(cur.at.wrapping_sub(prev.at)).filter(|&dt| dt > 0 && dt <= RATE_MS)
    }

    /// How much what `of` reads of a sample (a count, wrapping) grew since the last one.
    fn grew(&self, of: impl Fn(&Stats) -> Option<u32>) -> Option<u32> {
        let (c, p) = (of(self.cur.as_ref()?)?, of(self.prev.as_ref()?)?);
        Some(growth(p, c))
    }

    /// Per mille of a core that `busy` (a meter in ms) of the last two samples says.
    fn share(&self, busy: impl Fn(&Stats) -> Option<u32>) -> Option<u32> {
        let dt = self.dt()?;
        Some(self.grew(busy)?.min(dt) * 1000 / dt)
    }

    /// Per mille of a core the desktop itself used (its µs over the ms between the samples).
    fn desktop_share(&self) -> Option<u32> {
        let dt = self.dt()?;
        Some((self.grew(|s| s.quiet.get(stat::DESKTOP_US).copied())? / dt).min(1000))
    }

    /// The processes the desktop lists, but those ended from here.
    fn rows(&self) -> Vec<Row<'_>> {
        let Some(cur) = &self.cur else { return Vec::new() };
        let shown = cur.procs.iter().filter(|p| !self.ended.contains(&p.pid));
        shown.map(|p| row(p, self, cur)).collect()
    }

    /// The word for the whole machine, and the line under it.
    fn word(&self, rows: &[Row<'_>]) -> (&'static str, String) {
        let Some(cur) = &self.cur else { return ("Measuring", "Asking the desktop.".into()) };
        if self.prev.is_none() {
            return ("Measuring", "The rates come with the next look, in a second.".into());
        }
        let hot: Vec<&Row<'_>> = rows.iter().filter(|r| r.cpu.is_some_and(|c| c >= 500)).collect();
        if let Some(r) = hot.first() {
            let mut line = [&r.name, " is using ", &percent(r.cpu), " of a core."].concat();
            if hot.len() > 1 {
                line += &[" And ", &count(hot.len() - 1, "more", "more"), "."].concat();
            }
            return ("Busy", line);
        }
        let frames = [stat::INPUT, stat::MOTION, stat::PROGRAMS]
            .map(|i| self.grew(|s| s.loud.get(i).copied()).unwrap_or(0));
        let n = frames.iter().copied().max().unwrap_or(0);
        if n > 0 {
            // By the largest cause, input first: with a rate, `12 frames a second, following
            // you.`; without one, `Following you.`
            let drawer =
                rows.iter().max_by_key(|r| self.draws(r.p)).filter(|r| self.draws(r.p) > 0);
            let who = drawer.map_or("A program", |r| r.name.as_str());
            let drawing = [who, " is drawing."].concat();
            let (sep, why, alone) = match frames.iter().position(|&f| f == n) {
                Some(0) => (", ", "following you.", "Following you."),
                Some(1) => (": ", "something is moving.", "Something is moving."),
                _ => (": ", drawing.as_str(), drawing.as_str()),
            };
            let line = match self.dt() {
                Some(dt) => {
                    let rate = ((n * 1000 + dt / 2) / dt) as usize;
                    [&count(rate, "frame a second", "frames a second"), sep, why].concat()
                }
                None => alone.to_string(),
            };
            return ("Drawing", line);
        }
        match cur.loud.get(stat::GRAIN_ON) {
            Some(1) => (
                "Resting",
                "Only the living grain draws, 8 frames a second. Settings\u{a0}\u{203a} \
                Appearance can still it."
                    .into(),
            ),
            _ => ("Still", "Nothing is drawing.".into()),
        }
    }

    /// DRAWs process `p` made since the last sample.
    fn draws(&self, p: &Proc) -> u32 {
        let prev = self.prev.as_ref().and_then(|s| s.procs.iter().find(|q| q.pid == p.pid));
        let at = |p: &Proc| p.counts.get(stat::DRAWS).copied();
        prev.and_then(at).zip(at(p)).map_or(0, |(a, b)| growth(a, b))
    }

    fn nodes(&self) -> Vec<Node> {
        let rows = self.rows();
        match self.page {
            Some(0) => self.desktop_page(),
            Some(pid) => {
                rows.iter().find(|r| r.p.pid == pid).map_or_else(Vec::new, |r| self.page_of(r))
            }
            None => self.list(&rows),
        }
    }

    /// The word, what runs, your files, the AI, a note on the numbers.
    fn list(&self, rows: &[Row<'_>]) -> Vec<Node> {
        let (word, line) = self.word(rows);
        let mut nodes = vec![text(Style::Title, word), text(Style::Dim, &line)];
        let Some(cur) = &self.cur else { return nodes };
        nodes.extend([space(14), text(Style::Heading, "Running")]);
        let (glyph, hue) = DESKTOP_TILE;
        let desk = self.desktop_share();
        let desk_mem = cur.quiet.get(stat::DESKTOP_KB).copied();
        let line = ["the home screen and windows", DOT, &kb(desk_mem)].concat();
        let detail = desk.filter(|&c| c > 0).map_or_else(String::new, |c| percent(Some(c)));
        nodes.push(entry(DESKTOP, (glyph as u8, hue), ["The desktop", &line], detail, true));
        for r in rows {
            let detail = r.cpu.filter(|&c| c > 0).map_or_else(String::new, |c| percent(Some(c)));
            let line = self.state(r, true);
            nodes.push(entry(ROW + r.p.pid, r.tile, [&r.name, &line], detail, true));
        }
        let own = self.share(|s| s.own.get(stat::BUSY_MS).copied());
        let line = ["this window", DOT, &kb(cur.own.get(stat::MEM_KB).copied())].concat();
        let detail = own.filter(|&c| c > 0).map_or_else(String::new, |c| percent(Some(c)));
        nodes.push(entry(0, (Glyph::Pulse as u8, 0x22d3ee), ["Activity", &line], detail, false));
        nodes.push(space(14));
        nodes.extend(storage(cur));
        nodes.extend([space(14), text(Style::Heading, "AI since this tab opened")]);
        nodes.extend(ai(cur));
        nodes.extend([space(14), text(Style::Small, FOOTER)]);
        nodes
    }

    /// What `r` is doing, as its row says it (`short`: `in Terminal · working · 1 MB`) or its
    /// page (`Working in a Terminal, using 99% of a core.`).
    fn state(&self, r: &Row<'_>, short: bool) -> String {
        let draws = self.dt().map(|dt| (self.draws(r.p) * 1000 + dt / 2) / dt).filter(|&n| n > 0);
        let (row, page) = match (r.p.state, draws, r.cpu) {
            (stat::ENDED, ..) => ("ended".to_string(), "Ended".to_string()),
            (_, Some(n), _) => {
                (format!("drawing {n} a second"), format!("Drawing {n} frames a second"))
            }
            (stat::IDLE, ..) => ("idle".into(), "Idle, waiting for you".into()),
            (_, _, Some(0)) => ("waiting".into(), "Waiting".into()),
            _ => ("working".into(), "Working".into()),
        };
        if short {
            let place = if r.terminal { ["in Terminal", DOT].concat() } else { String::new() };
            return [&place, &row, DOT, &kb(r.mem)].concat();
        }
        let place = if r.terminal { " in a Terminal" } else { "" };
        let cpu = r
            .cpu
            .filter(|&c| c > 0)
            .map(|c| [", using ", &percent(Some(c)), " of a core"].concat());
        [&page, place, &cpu.unwrap_or_default(), "."].concat()
    }

    /// A process's page: its name, command line, what it does, its memory and pid, and End with
    /// what ending it does.
    fn page_of(&self, r: &Row<'_>) -> Vec<Node> {
        let mem = [&kb(r.mem), " of memory", DOT, "process ", &r.p.pid.to_string()].concat();
        let mut nodes = vec![back(), space(8), text(Style::Title, &r.name)];
        let cmd = r.p.argv.join(" ");
        if cmd != r.name {
            nodes.push(text(Style::Mono, &cmd));
        }
        nodes.extend([space(8), text(Style::Body, &self.state(r, false)), text(Style::Dim, &mem)]);
        if r.p.state != stat::ENDED {
            let what = match (r.terminal, r.p.window) {
                (true, _) => "It stops at once, as Ctrl+C would. What it had not saved is lost.",
                (_, 0) => "Ends the Assistant now. It starts again when you call it.",
                _ => "Ends it and closes its window. What it had not saved is lost.",
            };
            let label = ["End ", &r.name].concat();
            nodes.extend([space(14), Node::Button { id: END, variant: Variant::Danger, label }]);
            nodes.extend([space(6), text(Style::Small, what)]);
        }
        nodes
    }

    /// The desktop's page: what it does, its share of a core and its memory; no End.
    fn desktop_page(&self) -> Vec<Node> {
        let cur = self.cur.as_ref();
        let mem = kb(cur.and_then(|s| s.quiet.get(stat::DESKTOP_KB).copied()));
        let line = [&percent(self.desktop_share()), " of a core", DOT, &mem].concat();
        let title = text(Style::Title, "The desktop");
        vec![
            back(),
            space(8),
            title,
            space(8),
            text(Style::Body, DESKTOP_DOES),
            text(Style::Dim, &line),
        ]
    }
}

/// Process `p` of sample `cur` as a row: named by its command line, never its window's title (a
/// program sets that).
fn row<'a>(p: &'a Proc, a: &Activity, cur: &Stats) -> Row<'a> {
    let base = |s: &str| {
        let s = s.rsplit('/').next().unwrap_or(s);
        s.strip_suffix(".wasm").unwrap_or(s).to_string()
    };
    let arg = |i: usize| p.argv.get(i).map_or("", String::as_str);
    let file = |s: &str| s.rsplit('/').next().unwrap_or(s).to_string();
    let first = base(arg(0));
    let known = TILES.iter().find(|t| t.0 == first);
    let (name, tile, terminal) = match (known, arg(1)) {
        (Some(t), "run") if t.0 == "studio" => {
            let name = file(arg(2));
            let seed = name
                .bytes()
                .fold(2_166_136_261, |h, b| (h ^ u32::from(b)).wrapping_mul(16_777_619));
            (name, (SIGIL, seed), false)
        }
        (Some(t), "edit") if t.0 == "studio" => {
            ([t.1, ": ", &file(arg(2))].concat(), (t.2 as u8, t.3), false)
        }
        (Some(t), _) => (t.1.to_string(), (t.2 as u8, t.3), false),
        (None, _) if first.is_empty() => {
            ("A program".to_string(), (TERMINAL.0 as u8, TERMINAL.1), true)
        }
        (None, _) => (first, (TERMINAL.0 as u8, TERMINAL.1), true),
    };
    let meters = cur.meters_of(p.pid);
    let busy = |s: &Stats| s.meters_of(p.pid).get(stat::BUSY_MS).copied();
    let cpu = if meters.is_empty() { None } else { a.share(busy) };
    let mem = meters.get(stat::MEM_KB).copied();
    Row { p, name, tile, terminal, cpu, mem }
}

/// Your files against what the browser keeps, in squares; a line if they go unkept.
fn storage(cur: &Stats) -> Vec<Node> {
    let (home, unkept) =
        (cur.loud.get(stat::HOME).copied(), cur.loud.get(stat::UNKEPT) == Some(&1));
    let size = home.map_or("\u{2014}".into(), |n| bytes(u64::from(n)));
    let line = [&size, " of about 5 MB"].concat();
    let (glyph, hue) = (Glyph::Folder as u8, 0x60a5fa);
    let mut nodes = vec![entry(FILES, (glyph, hue), ["Your files", &line], String::new(), true)];
    let n = u64::from(home.unwrap_or(0));
    let filled = (n * SQUARES).div_ceil(KEPT).min(SQUARES) as usize;
    let color = if unkept {
        1
    } else if n * 10 >= KEPT * 8 {
        3
    } else {
        2
    };
    let mut cells = vec![color; filled];
    cells.resize(SQUARES as usize, 0);
    nodes.extend([space(6), Node::Grid { id: 0, cols: SQUARES as u16, cells, texts: Vec::new() }]);
    if unkept {
        let why = "This browser is not keeping your files now: what changes is lost at reload.";
        nodes.extend([space(6), text(Style::Error, why)]);
    }
    nodes
}

/// The AI's requests, tokens, failures and those with no token count, as people say them.
fn ai(cur: &Stats) -> Vec<Node> {
    let n = |i: usize| cur.loud.get(i).copied();
    let Some(asked) = n(stat::ASKED) else { return vec![text(Style::Dim, "\u{2014}")] };
    if asked == 0 {
        return vec![text(Style::Body, "No requests yet.")];
    }
    let mut line = count(asked as usize, "request", "requests");
    if let (Some(i), Some(o)) = (n(stat::TOKENS_IN), n(stat::TOKENS_OUT)) {
        if i > 0 || o > 0 {
            line += &[DOT, &tokens(i), " tokens in, ", &tokens(o), " out"].concat();
        }
    }
    let mut nodes = vec![text(Style::Body, &line)];
    if let Some(f) = n(stat::FAILED).filter(|&f| f > 0) {
        nodes.push(text(Style::Error, &[&f.to_string(), " failed"].concat()));
    }
    if let Some(u) = n(stat::UNCOUNTED).filter(|&u| u > 0) {
        nodes.push(text(Style::Dim, &[&u.to_string(), " had no token count."].concat()));
    }
    nodes
}

/// A list row: `[name, line]` (the line small under the name), `detail` at the right, a chevron
/// when it opens a page.
fn entry(
    id: u32,
    (glyph, hue): (u8, u32),
    [name, line]: [&str; 2],
    detail: String,
    more: bool,
) -> Node {
    Node::Entry { id, glyph, hue, text: [name, "\n", line].concat(), detail, more }
}

fn back() -> Node {
    Node::Button { id: BACK, variant: Variant::Quiet, label: "\u{2039} Activity".into() }
}

/// How much a wrapping count grew from `a` to `b` (none if it seems to have gone back).
pub(crate) fn growth(a: u32, b: u32) -> u32 {
    (b.wrapping_sub(a) as i32).max(0) as u32
}

/// Per mille of a core as people say it: `<1%`, `12%`, at most `100%`; unknown `—`.
pub(crate) fn percent(pm: Option<u32>) -> String {
    match pm {
        None => "\u{2014}".into(),
        Some(0..10) => "<1%".into(),
        Some(pm) => [&(pm.min(1000) / 10).to_string(), "%"].concat(),
    }
}

/// KB of memory as bytes are said; unknown `—`.
fn kb(n: Option<u32>) -> String {
    n.map_or("\u{2014}".into(), |kb| bytes(u64::from(kb) * 1024))
}

/// `n` bytes in decimal units, as phones' storage screens say them: `740 B`, `212 KB`, `1.3 MB`,
/// `18 MB`.
pub(crate) fn bytes(n: u64) -> String {
    let (kb, tenths, mb) = ((n + 500) / 1000, (n + 50_000) / 100_000, (n + 500_000) / 1_000_000);
    match n {
        0..1000 => [&n.to_string(), " B"].concat(),
        _ if kb < 1000 => [&kb.to_string(), " KB"].concat(),
        _ if tenths < 100 => {
            [&(tenths / 10).to_string(), ".", &(tenths % 10).to_string(), " MB"].concat()
        }
        _ => [&mb.to_string(), " MB"].concat(),
    }
}

/// Tokens as the Assistant says them: `940`, `12.4k`, `1.2M`.
pub(crate) fn tokens(n: u32) -> String {
    let (n, unit, scale) = match n {
        0..1000 => return n.to_string(),
        1000..1_000_000 => (n, "k", 1000),
        _ => (n, "M", 1_000_000),
    };
    [&(n / scale).to_string(), ".", &(n % scale / (scale / 10)).to_string(), unit].concat()
}

/// `n` and the noun for it: `1 request`, `2 requests`.
fn count(n: usize, one: &str, more: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { more })
}
