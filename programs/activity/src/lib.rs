//! Activity, compusophyOS's resource monitor: what the desktop and each program use, now and over
//! the last minute. **Performance** graphs CPU (the desktop's and the programs' share of a core),
//! memory, frames a second and the AI's tokens a second, and shows your files against what the
//! browser keeps. **Processes** is a table of what runs, sorted by the column you pick; a row
//! opens its page (its CPU over the minute, and End for a program).
//!
//! It watches the desktop's meters from its first size ([`Request::Watch`]). The desktop sends a
//! sample only when something changed, at most once a second, so at rest nothing comes and
//! nothing here draws: the graphs move when a sample does, the seconds since the last at their
//! average (a rate) or as they were (a level). On a phone (narrower than [`PHONE`] px) it
//! watches only while it has the focus, since another app covers it then. What it cannot know
//! reads `—`.

#![forbid(unsafe_code)]

use std::mem;

use icons::Glyph;
pub use pool::PoolPage;
use uiwire::stat::{self, Proc, Stats};
use uiwire::{Event, Frame, Key, Node, Request, SIGIL, Style, Variant};

/// Node ids: the pages (Performance, Processes), Back, End, the files link, the table's head
/// (column `k` is `SORT + k`), the desktop's row, Activity's own; a program's row is `ROW` plus
/// its pid.
pub const NAV: u32 = 1;
pub const BACK: u32 = 10;
pub const END: u32 = 11;
pub const FILES: u32 = 12;
pub const SORT: u32 = 20;
pub const DESKTOP: u32 = 30;
pub const OWN: u32 = 31;
pub const ROW: u32 = 100;
/// The keys of the desktop's and Activity's own histories among the programs'.
const DESKTOP_KEY: u32 = 0;
const OWN_KEY: u32 = u32::MAX;
/// Narrower than this a window is a phone's; narrower than `NARROW` the cards stack and the
/// table drops its frames column.
pub const PHONE: u16 = 480;
pub const NARROW: u16 = 640;
/// The seconds a graph shows.
pub const SECONDS: usize = 60;
/// What the browser keeps of a page's storage, about.
const KEPT: u64 = 5_000_000;
/// The graphs' heights, on a card and on a page.
const CHART_H: u16 = 64;
/// The canvas colors of the graphs: CPU the accent, memory magenta, frames green, the AI cyan;
/// the storage bar blue, yellow when nearly full, red when unkept.
const HUES: [u8; 4] = [11, 5, 2, 6];
#[rustfmt::skip]
const TILES: [(&str, &str, Glyph, u32); 8] = [
    ("about", "About", Glyph::About, 0xfbbf24), ("feedback", "Feedback", Glyph::Bug, 0x34d399),
    ("files", "Files", Glyph::Folder, 0x60a5fa), ("welcome", "Welcome", Glyph::Mark, 0xf472b6),
    ("settings", "Settings", Glyph::Cog, 0x94a3b8), ("editor", "Editor", Glyph::Editor, 0xfb923c),
    ("assistant", "Assistant", Glyph::Assistant, 0xa78bfa),
    ("studio", "Studio", Glyph::Studio, 0x8b7bff),
];
const TERMINAL: (Glyph, u32) = (Glyph::Terminal, 0x2dd4bf);
const DESKTOP_TILE: (Glyph, u32) = (Glyph::Mark, 0x94a3b8);
const OWN_TILE: (Glyph, u32) = (Glyph::Pulse, 0x22d3ee);
const DESKTOP_DOES: &str = "The desktop runs the home screen, the windows and the Terminal's \
screen, and serves programs\u{2019} files. Reloading the page restarts it.";
const PERFORMANCE_NOTE: &str = "The graphs show the last minute, a point a second. At rest the \
desktop sends nothing, so they hold still until something moves. Percents are of one core; \
memory is wasm memory, which never shrinks.";
const DOT: &str = " \u{b7} ";
const DASH: &str = "\u{2014}";

/// A meter's last minute, a value a second, the newest last.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History(pub Vec<u32>);

impl History {
    /// `secs` more seconds (1 at least) of `v` each; a minute is kept.
    pub fn add(&mut self, secs: u32, v: u32) {
        let n = (secs.max(1) as usize).min(SECONDS);
        self.0.extend(std::iter::repeat_n(v, n));
        let over = self.0.len().saturating_sub(SECONDS);
        self.0.drain(..over);
    }

    /// A level now `v`: the seconds before it as the last one was.
    pub fn level(&mut self, secs: u32, v: u32) {
        let was = self.0.last().copied().unwrap_or(v);
        if secs > 1 {
            self.add(secs - 1, was);
        }
        self.add(1, v);
    }

    pub fn now(&self) -> Option<u32> {
        self.0.last().copied()
    }

    pub fn peak(&self) -> u32 {
        self.0.iter().copied().max().unwrap_or(0)
    }

    /// A graph `h` px tall of the minute, `full` at its top; seconds not yet seen unknown.
    pub fn chart(&self, hue: u8, h: u16, full: u32) -> Node {
        let unseen = SECONDS.saturating_sub(self.0.len());
        let at = |v: u32| (u64::from(v) * 1000 / u64::from(full.max(1))).min(1000) as u16;
        let unseen = std::iter::repeat_n(uiwire::UNKNOWN, unseen);
        let values = unseen.chain(self.0.iter().map(|&v| at(v)));
        Node::Chart { id: 0, hue, h, values: values.collect() }
    }
}

/// What the last two samples say, each a rate a second: the desktop's and Activity's own share
/// of a core (per mille), each program's and its frames, and frames by cause (the person,
/// motion, programs, Activity, timers, other).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Now {
    pub desktop: u32,
    pub own: u32,
    pub procs: Vec<(u32, u32, u32)>,
    pub causes: [u32; 6],
}

/// A column of the table, by its index in the head: 0 the name, then CPU, memory, frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Column(pub u8);

impl Default for Column {
    /// CPU.
    fn default() -> Column {
        Column(1)
    }
}

/// The page shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Performance,
    Processes,
    /// A row's page, by its key: the desktop 0, Activity `u32::MAX`, else a pid.
    Of(u32),
    /// The mesh: linked devices and the job they share.
    Pool,
}

/// Activity: see the crate docs.
#[derive(Debug, Default)]
pub struct Activity {
    /// The last two samples, the newer last, and what they say.
    pub prev: Option<Stats>,
    pub cur: Option<Stats>,
    pub now: Option<Now>,
    /// The minute of CPU (per mille of a core, all of it), memory (KB, all of it), frames a
    /// second and AI tokens a second; each row's CPU, by key.
    pub graphs: [History; 4],
    pub rows: Vec<(u32, History)>,
    /// The Pool page's own.
    pub pool: PoolPage,
    pub page: Page,
    /// The table's column, by its index in the head (0 the name; CPU at first).
    pub sort: Column,
    /// Processes ended from here, left out until the desktop no longer lists them.
    ended: Vec<u32>,
    /// The nodes last framed, the requests since, whether a frame went yet; the width; whether
    /// it watches, and whether the next sample starts anew.
    shown: Vec<Node>,
    requests: Vec<Request>,
    framed: bool,
    width: u16,
    watching: bool,
    anew: bool,
}

/// A row of the table: its node id and key, name, what it does, tile, share of a core, memory
/// and frames a second; whether it runs in a Terminal, its window, its state, its argv.
#[derive(Clone, Debug)]
pub struct Row {
    pub id: u32,
    pub key: u32,
    pub name: String,
    pub doing: String,
    pub tile: (u8, u32),
    pub cpu: Option<u32>,
    pub mem: Option<u32>,
    pub fps: Option<u32>,
    pub terminal: bool,
    pub window: u32,
    pub state: u8,
    pub argv: Vec<String>,
}

impl Activity {
    /// Handles one event; whether the window changed.
    pub fn event(&mut self, ev: &Event) -> bool {
        // The Pool page's own: never a process row too (its linked devices' ids are past ROW).
        let pool = self.pool.event(ev, &mut self.requests);
        match ev {
            // From the first size; and again once wide, if a phone's focus had paused it.
            Event::Resize { w, .. } => {
                self.width = *w;
                if !self.framed || *w >= PHONE {
                    self.watch(true);
                }
            }
            Event::Focus { on } if self.width < PHONE => self.watch(*on),
            Event::Stats { data } => {
                if let Some(s) = Stats::decode(data) {
                    self.take(s);
                }
            }
            Event::Click { id: BACK } | Event::Key { key: Key::Escape, .. } => {
                if let Page::Of(_) = self.page {
                    self.page = Page::Processes;
                }
            }
            Event::Click { id: FILES } => {
                self.requests.push(Request::Open { name: "files".into() })
            }
            Event::Click { id: END } => {
                if let Page::Of(pid) = self.page {
                    if pid != DESKTOP_KEY && pid != OWN_KEY {
                        self.requests.push(Request::End { pid });
                        self.ended.push(pid);
                        self.page = Page::Processes;
                    }
                }
            }
            Event::Click { id: DESKTOP } => self.page = Page::Of(DESKTOP_KEY),
            Event::Click { id: OWN } => self.page = Page::Of(OWN_KEY),
            Event::Click { id } if *id == NAV => self.page = Page::Performance,
            Event::Click { id } if *id == NAV + 1 => self.page = Page::Processes,
            Event::Click { id } if *id == NAV + 2 => self.page = Page::Pool,
            Event::Click { id } if (SORT..SORT + 4).contains(id) => {
                self.sort = Column((id - SORT) as u8)
            }
            Event::Click { id } if *id >= ROW && !pool => self.page = Page::Of(id - ROW),
            _ => {}
        }
        // A page whose process is gone shows the table.
        if let Page::Of(key) = self.page {
            if !self.table().iter().any(|r| r.key == key) {
                self.page = Page::Processes;
            }
        }
        let nodes = self.nodes();
        let changed = !self.framed || !self.requests.is_empty() || nodes != self.shown;
        self.shown = nodes;
        changed
    }

    /// The window now, with the requests since the last frame.
    pub fn frame(&mut self) -> Frame {
        self.framed = true;
        let requests = mem::take(&mut self.requests);
        Frame { seq: 0, title: "Activity".into(), requests, nodes: self.shown.clone() }
    }

    /// Starts or stops watching the meters; paused, it keeps what it showed. Back from a pause it
    /// measures anew: no rate spans the pause.
    fn watch(&mut self, on: bool) {
        if on != self.watching {
            (self.watching, self.anew) = (on, on);
            self.requests.push(Request::Watch { on });
        }
    }

    /// A new sample: what it and the last say, into the graphs.
    fn take(&mut self, s: Stats) {
        self.ended.retain(|&pid| s.procs.iter().any(|p| p.pid == pid));
        let anew = mem::take(&mut self.anew);
        self.prev = self.cur.replace(s).filter(|_| !anew);
        let Some(cur) = &self.cur else { return };
        let (dt, now) = match &self.prev {
            Some(prev) => {
                let dt = cur.at.wrapping_sub(prev.at).max(1);
                (dt, Some(rates(prev, cur, dt)))
            }
            None => (1000, None),
        };
        let secs = (dt + 500) / 1000;
        let memory = memory(cur);
        self.graphs[1].level(secs, memory.0.unwrap_or(0) + memory.1);
        if let Some(n) = &now {
            let cpu = n.desktop + n.own + n.procs.iter().map(|p| p.1).sum::<u32>();
            let per_s = |grew: u32| (u64::from(grew) * 1000 / u64::from(dt)) as u32;
            let tokens = |s: &Stats| {
                let n = |i: usize| s.loud.get(i).copied().unwrap_or(0);
                n(stat::TOKENS_IN).wrapping_add(n(stat::TOKENS_OUT))
            };
            let ai = self.prev.as_ref().map_or(0, |p| per_s(growth(tokens(p), tokens(cur))));
            self.graphs[0].add(secs, cpu);
            self.graphs[2].add(secs, n.causes.iter().sum());
            self.graphs[3].add(secs, ai);
            let mut keyed = vec![(DESKTOP_KEY, n.desktop), (OWN_KEY, n.own)];
            keyed.extend(n.procs.iter().map(|p| (p.0, p.1)));
            self.rows.retain(|r| keyed.iter().any(|k| k.0 == r.0));
            for (key, cpu) in keyed {
                match self.rows.iter_mut().find(|r| r.0 == key) {
                    Some(r) => r.1.add(secs, cpu),
                    None => self.rows.push((key, History(vec![cpu]))),
                }
            }
        }
        self.now = now;
    }

    /// The table's rows: the desktop, each program the desktop lists (but those ended from
    /// here), Activity; sorted by the column picked, the most first (names A to Z).
    pub fn table(&self) -> Vec<Row> {
        let Some(cur) = &self.cur else { return Vec::new() };
        let now = self.now.as_ref();
        let (desk_mem, _) = memory(cur);
        let (glyph, hue) = DESKTOP_TILE;
        let mut rows = vec![Row {
            id: DESKTOP,
            key: DESKTOP_KEY,
            name: "The desktop".into(),
            doing: "home screen and windows".into(),
            tile: (glyph as u8, hue),
            cpu: now.map(|n| n.desktop),
            mem: desk_mem,
            fps: None,
            terminal: false,
            window: 0,
            state: stat::RUNS,
            argv: Vec::new(),
        }];
        let shown = cur.procs.iter().filter(|p| !self.ended.contains(&p.pid));
        rows.extend(shown.map(|p| self.program(p, cur)));
        let (glyph, hue) = OWN_TILE;
        rows.push(Row {
            id: OWN,
            key: OWN_KEY,
            name: "Activity".into(),
            doing: "this window".into(),
            tile: (glyph as u8, hue),
            cpu: now.map(|n| n.own),
            mem: cur.own.get(stat::MEM_KB).copied(),
            fps: None,
            terminal: false,
            window: 0,
            state: stat::RUNS,
            argv: Vec::new(),
        });
        match self.sort.0 {
            0 => rows.sort_by_key(|r| r.name.to_lowercase()),
            k => rows.sort_by_key(|r| {
                std::cmp::Reverse([r.cpu, r.mem, r.fps][usize::from(k - 1).min(2)].unwrap_or(0))
            }),
        }
        rows
    }

    /// Process `p` of sample `cur` as a row: named by its command line, never its window's title
    /// (a program sets that).
    fn program(&self, p: &Proc, cur: &Stats) -> Row {
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
        let now = self.now.as_ref().and_then(|n| n.procs.iter().find(|q| q.0 == p.pid));
        let meters = cur.meters_of(p.pid);
        let cpu = now.filter(|_| !meters.is_empty()).map(|q| q.1);
        let fps = now.map(|q| q.2);
        let doing = match (p.state, fps) {
            (stat::ENDED, _) => "ended",
            (_, Some(1..)) => "drawing",
            (stat::IDLE, _) => "waiting for you",
            (_, _) if cpu == Some(0) => "waiting",
            _ => "working",
        };
        let doing = if terminal { ["in Terminal", DOT, doing].concat() } else { doing.into() };
        let (mem, window, state, argv) =
            (meters.get(stat::MEM_KB).copied(), p.window, p.state, p.argv.clone());
        Row {
            id: ROW + p.pid,
            key: p.pid,
            name,
            doing,
            tile,
            cpu,
            mem,
            fps,
            terminal,
            window,
            state,
            argv,
        }
    }

    fn nodes(&self) -> Vec<Node> {
        let on = match self.page {
            Page::Performance => 0,
            Page::Pool => 2,
            _ => 1,
        };
        let labels = "Performance\nProcesses\nPool".into();
        let mut nodes = vec![Node::Pages { id: NAV, on, labels }];
        nodes.extend(match self.page {
            Page::Performance => self.performance(),
            Page::Processes => self.processes(),
            Page::Pool => self.pool.nodes(self.width >= NARROW),
            Page::Of(key) => self
                .table()
                .into_iter()
                .find(|r| r.key == key)
                .map_or_else(Vec::new, |r| self.page_of(&r)),
        });
        nodes
    }

    /// The cards: CPU, memory, frames, the AI; your files; a note on the numbers.
    fn performance(&self) -> Vec<Node> {
        let mut nodes = vec![text(Style::Heading, "Performance")];
        let Some(cur) = &self.cur else {
            nodes.push(text(Style::Dim, "Asking the desktop\u{2026}"));
            return nodes;
        };
        let now = self.now.as_ref();
        let n = |i: usize| cur.loud.get(i).copied();
        // CPU: of one core, or of as many as the minute needed.
        let cpu = &self.graphs[0];
        let line = now.map_or_else(String::new, |n| {
            let programs = n.own + n.procs.iter().map(|p| p.1).sum::<u32>();
            ["Desktop ", &percent(Some(n.desktop)), DOT, "programs ", &percent(Some(programs))]
                .concat()
        });
        // A core at the top, or the minute's most past it.
        let chart = cpu.chart(HUES[0], CHART_H, cpu.peak().max(1000));
        let cpu = card("CPU", &percent(now.and(cpu.now())), chart, &line);
        // Memory: all of it, the graph's top a round number above the minute's most.
        let mem = &self.graphs[1];
        let (desk, programs) = memory(cur);
        let line = ["Desktop ", &kb(desk), DOT, "programs ", &kb(Some(programs))].concat();
        let full = nice(mem.peak() + mem.peak() / 4);
        let memory = card("Memory", &kb(mem.now()), mem.chart(HUES[1], CHART_H, full), &line);
        // Frames: by cause.
        let fps = &self.graphs[2];
        const CAUSES: [&str; 6] = ["you", "motion", "programs", "this window", "timers", "other"];
        let causes = now.map(|n| {
            let said: Vec<String> = n
                .causes
                .iter()
                .zip(CAUSES)
                .filter(|c| *c.0 > 0)
                .map(|(f, c)| format!("{c} {f}"))
                .collect();
            if said.is_empty() { "Nothing is drawing.".to_string() } else { said.join(DOT) }
        });
        let full = fps.peak().div_ceil(60).max(1) * 60;
        let shown = now.and(fps.now()).map_or(DASH.into(), |f| [&f.to_string(), " fps"].concat());
        let frames =
            card("Frames", &shown, fps.chart(HUES[2], CHART_H, full), &causes.unwrap_or_default());
        // The AI: since this tab opened, tokens a second over the minute.
        let ai = &self.graphs[3];
        let used = n(stat::TOKENS_IN).zip(n(stat::TOKENS_OUT)).map(|(i, o)| i.saturating_add(o));
        let asked = n(stat::ASKED).unwrap_or(0);
        let mut line = count(asked as usize, "request", "requests");
        if let Some(usd) = n(stat::MICROUSD).filter(|&u| u > 0) {
            line += &[DOT, &dollars(usd)].concat();
        }
        if let Some(f) = n(stat::FAILED).filter(|&f| f > 0) {
            line += &[DOT, &f.to_string(), " failed"].concat();
        }
        let head = used.map_or(DASH.into(), |t| [&tokens(t), " tokens"].concat());
        let full = nice(ai.peak().max(10));
        let ai = card("AI since this tab opened", &head, ai.chart(HUES[3], CHART_H, full), &line);
        if self.width >= NARROW {
            let pair = |a, b| Node::Row { id: 0, gap: 12, children: vec![a, b] };
            nodes.extend([pair(cpu, memory), pair(frames, ai)]);
        } else {
            nodes.extend([cpu, memory, frames, ai]);
        }
        nodes.push(storage(cur));
        nodes.push(text(Style::Small, PERFORMANCE_NOTE));
        nodes
    }

    /// The table: its head (the column picked lit), a row a process.
    fn processes(&self) -> Vec<Node> {
        let rows = self.table();
        let wide = self.width >= NARROW;
        let labels = if wide { "Name\tCPU\tMemory\tFrames" } else { "Name\tCPU\tMemory" };
        let on = if wide || self.sort.0 < 3 { self.sort.0 + 1 } else { 0 };
        let mut nodes = vec![
            text(Style::Heading, "Processes"),
            text(Style::Dim, &count(rows.len(), "running", "running")),
            Node::Columns { id: SORT, on, labels: labels.into() },
        ];
        for r in &rows {
            let fps = r.fps.map_or(DASH.into(), |f| f.to_string());
            let mut cells = [percent(r.cpu), kb(r.mem)].join("\t");
            if wide {
                cells = [&cells, "\t", &fps].concat();
            }
            let (glyph, hue) = r.tile;
            let text = [&r.name, "\n", &r.doing].concat();
            nodes.push(Node::Entry { id: r.id, glyph, hue, text, detail: cells, more: true });
        }
        nodes.push(text(
            Style::Small,
            "Percents are of one core. A row shows what it is doing; its page can end it.",
        ));
        nodes
    }

    /// A row's page: its name and command line, what it does, its CPU over the minute, its
    /// memory and frames; End for a program, with what ending it does.
    fn page_of(&self, r: &Row) -> Vec<Node> {
        let back =
            Node::Button { id: BACK, variant: Variant::Quiet, label: "\u{2039} Processes".into() };
        let mut nodes = vec![back, text(Style::Title, &r.name)];
        let cmd = r.argv.join(" ");
        if !cmd.is_empty() && !cmd.eq_ignore_ascii_case(&r.name) {
            nodes.push(text(Style::Mono, &cmd));
        }
        if r.key == DESKTOP_KEY {
            nodes.push(text(Style::Body, DESKTOP_DOES));
        }
        let history =
            self.rows.iter().find(|h| h.0 == r.key).map(|h| h.1.clone()).unwrap_or_default();
        let mut facts = vec![["Memory ", &kb(r.mem)].concat()];
        if let Some(f) = r.fps {
            facts.push(count(f as usize, "frame a second", "frames a second"));
        }
        if r.key != DESKTOP_KEY && r.key != OWN_KEY {
            facts.push(["process ", &r.key.to_string()].concat());
        }
        let doing = [&capital(&r.doing), DOT, &facts.join(DOT)].concat();
        nodes.push(card(
            "CPU, the last minute",
            &percent(r.cpu),
            history.chart(HUES[0], 96, history.peak().max(1000)),
            &doing,
        ));
        if r.key != DESKTOP_KEY && r.key != OWN_KEY && r.state != stat::ENDED {
            let what = match (r.terminal, r.window) {
                (true, _) => "It stops at once, as Ctrl+C would. What it had not saved is lost.",
                (_, 0) => "Ends the Assistant now. It starts again when you call it.",
                _ => "Ends it and closes its window. What it had not saved is lost.",
            };
            let label = ["End ", &r.name].concat();
            nodes.extend([
                Node::Button { id: END, variant: Variant::Danger, label },
                text(Style::Small, what),
            ]);
        }
        nodes
    }
}

/// What the samples `prev` and `cur`, `dt` ms apart, say a second.
fn rates(prev: &Stats, cur: &Stats, dt: u32) -> Now {
    let per_mille = |grew: u32| ((u64::from(grew) * 1000 / u64::from(dt)) as u32).min(1000);
    let per_s = |grew: u32| (u64::from(grew) * 1000 / u64::from(dt)) as u32;
    let grew = |of: &dyn Fn(&Stats) -> Option<u32>| match (of(prev), of(cur)) {
        (Some(a), Some(b)) => growth(a, b),
        _ => 0,
    };
    // µs over ms is per mille.
    let desktop = (grew(&|s| s.quiet.get(stat::DESKTOP_US).copied()) / dt).min(1000);
    let own = per_mille(grew(&|s| s.own.get(stat::BUSY_MS).copied()));
    let procs = cur.procs.iter().map(|p| {
        let busy = grew(&|s| s.meters_of(p.pid).get(stat::BUSY_MS).copied());
        let draws = |s: &Stats| {
            s.procs.iter().find(|q| q.pid == p.pid).and_then(|q| q.counts.get(stat::DRAWS).copied())
        };
        (p.pid, per_mille(busy), per_s(grew(&draws)))
    });
    let loud = [stat::INPUT, stat::MOTION, stat::PROGRAMS]
        .map(|i| per_s(grew(&|s| s.loud.get(i).copied())));
    let quiet =
        [stat::SELF, stat::TIMER, stat::OTHER].map(|i| per_s(grew(&|s| s.quiet.get(i).copied())));
    let causes = [loud[0], loud[1], loud[2], quiet[0], quiet[1], quiet[2]];
    Now { desktop, own, procs: procs.collect(), causes }
}

/// The desktop's memory and the programs' (Activity's own among them), in KB.
fn memory(s: &Stats) -> (Option<u32>, u32) {
    let programs = s.meters.iter().filter_map(|m| m.1.get(stat::MEM_KB)).sum::<u32>();
    let own = s.own.get(stat::MEM_KB).copied().unwrap_or(0);
    (s.quiet.get(stat::DESKTOP_KB).copied(), programs + own)
}

/// A card: its name small, its value large, a graph, a line under it.
fn card(name: &str, value: &str, chart: Node, line: &str) -> Node {
    let mut children = vec![text(Style::Small, name), text(Style::Title, value), chart];
    if !line.is_empty() {
        children.push(text(Style::Small, line));
    }
    Node::Card { id: 0, children: vec![Node::Col { id: 0, gap: 6, children }] }
}

/// Your files against what the browser keeps: a bar, blue, yellow nearly full, red unkept.
fn storage(cur: &Stats) -> Node {
    let (home, unkept) =
        (cur.loud.get(stat::HOME).copied(), cur.loud.get(stat::UNKEPT) == Some(&1));
    let n = u64::from(home.unwrap_or(0));
    let value = (n * 1000 / KEPT).min(1000) as u16;
    let hue = if unkept {
        1
    } else if value >= 800 {
        3
    } else {
        4
    };
    let size = home.map_or(DASH.into(), |n| bytes(u64::from(n)));
    let note = if unkept {
        text(
            Style::Error,
            "This browser refused to keep your files. Changes since then are lost at reload.",
        )
    } else {
        text(Style::Small, "Kept in this browser, on this device.")
    };
    let link = Node::Button { id: FILES, variant: Variant::Link, label: "Show your files".into() };
    let children = vec![
        text(Style::Small, "Storage"),
        text(Style::Title, &[&size, " of about 5 MB"].concat()),
        Node::Meter { id: 0, hue, value },
        note,
        link,
    ];
    Node::Card { id: 0, children: vec![Node::Col { id: 0, gap: 6, children }] }
}

fn text(style: Style, text: &str) -> Node {
    Node::Text { id: 0, style, text: text.into() }
}

/// `s` with its first letter a capital.
fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

/// The smallest of 1, 2 and 5 times a power of ten at least `v` (10 at least).
pub fn nice(v: u32) -> u32 {
    let mut ten = 1u32;
    loop {
        for m in [1, 2, 5] {
            match ten.checked_mul(m) {
                Some(n) if n >= v.max(10) => return n,
                None => return u32::MAX,
                _ => {}
            }
        }
        ten = match ten.checked_mul(10) {
            Some(t) => t,
            None => return u32::MAX,
        };
    }
}

/// µ$ as people say a small cost: `about $0.02`, `under $0.01`, `about $1.25`.
pub fn dollars(usd: u32) -> String {
    let cents = usd.saturating_add(5_000) / 10_000;
    match cents {
        0 => "under $0.01".into(),
        _ => format!("about ${}.{:02}", cents / 100, cents % 100),
    }
}

/// How much a wrapping count grew from `a` to `b` (none if it seems to have gone back).
pub fn growth(a: u32, b: u32) -> u32 {
    (b.wrapping_sub(a) as i32).max(0) as u32
}

/// Per mille of a core as people say it: `<1%`, `12%`, `180%`; unknown `—`.
pub fn percent(pm: Option<u32>) -> String {
    match pm {
        None => DASH.into(),
        Some(0) => "0%".into(),
        Some(1..10) => "<1%".into(),
        Some(pm) => [&(pm / 10).to_string(), "%"].concat(),
    }
}

/// KB of memory as bytes are said; unknown `—`.
pub fn kb(n: Option<u32>) -> String {
    n.map_or(DASH.into(), |kb| bytes(u64::from(kb) * 1024))
}

/// `n` bytes in decimal units, as phones' storage screens say them: `740 B`, `212 KB`, `1.3 MB`,
/// `18 MB`.
pub fn bytes(n: u64) -> String {
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
pub fn tokens(n: u32) -> String {
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

mod pool;
#[cfg(test)]
mod tests;
