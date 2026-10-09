//! Activity's Pool page: the mesh, as the rest of Activity shows this tab. The devices linked for
//! good (here or not, Reconnect, Unlink) and linking a new one (show a code, or enter the other
//! device's); the pool's size and how busy it is over the minute, how fast it
//! answers chunks and steps; each device (sortable), its share of the work in its color; each
//! link's round trip and traffic over the minute, both ways, and its throughput when measured;
//! the job, if one runs; the model a device shares, asked from here, and this device's to share. The pool program sends a snapshot ([`Snap`]) at most once a second
//! while something changes, so at rest nothing comes and nothing here draws.

use uiwire::pool::{self, Snap};
use uiwire::{Event, Node, Request, Style, Variant};

use super::{DASH, DOT, History, bytes, card, count, nice, text};

/// Node ids: Test this device; Show a code, the code field, Link, Measure, Fractal, Verify; the
/// server field and
/// Share, Ask; the devices' head (column `k` is `SORT + k`); the question field, `QUESTION` and
/// one more for each question asked (a new field is an empty one, as a chat's is).
pub const TEST: u32 = 38;
/// Measure all memory.
pub const FILL: u32 = 39;
pub const SHOW: u32 = 40;
pub const CODE: u32 = 41;
pub const JOIN: u32 = 42;
pub const MEASURE: u32 = 43;
pub const FRACTAL: u32 = 44;
pub const VERIFY: u32 = 45;
/// The IQ suite's job: the program, and the file its chunks are made of, three tasks each.
pub const IQ_JOB: &str = "iq";
pub const IQ_SUITE: &str = "@suites/iq.jsonl 3";
pub const SERVER: u32 = 46;
pub const SHARE: u32 = 47;
pub const ASK: u32 = 49;
/// The server a device shares unless the person names another.
pub const LOCAL: &str = "http://localhost:8080";
pub const SORT: u32 = 50;
pub const QUESTION: u32 = 1 << 30;
/// A linked device's Reconnect, `BOND` + 2 its place, and Unlink, one more.
pub const BOND: u32 = 1 << 24;
/// An offline device's color in the table: grey.
const AWAY: u32 = 0x6b7280;
/// Each device's color as RGB, by [`pool::TINTS`]'s canvas colors (cyan, yellow, magenta, green,
/// red, blue).
const RGB: [u32; 6] = [0x22d3ee, 0xfacc15, 0xe879f9, 0x4ade80, 0xf87171, 0x60a5fa];
const NOTE: &str = "Linked tabs share one job: every core of every device takes the next chunk \
as it goes idle, so faster devices take more. Answers come back with the steps they took and a \
hash of the result; some are replayed here to check them. Tabs talk directly, encrypted; only \
their descriptions pass through the server, briefly, when they link or meet again.";

/// The Pool page's state: the last two snapshots, the minute of utilization (per mille of the
/// workers busy) and chunks a second, the steps a second last taken, each link's bytes a second
/// heard and sent (by device name), the code typed, the devices' sort column (0 the name), the IQ
/// suite's run if one was asked for, the server and the question typed, and the questions asked.
#[derive(Debug, Default)]
pub struct PoolPage {
    pub prev: Option<Snap>,
    pub cur: Option<Snap>,
    pub graphs: [History; 2],
    pub steps: u64,
    pub links: Vec<(String, History, History)>,
    pub code: String,
    pub sort: u8,
    pub iq: Option<Iq>,
    pub server: String,
    pub question: String,
    pub asked: u32,
}

/// The IQ suite verified on the pool: each chunk's answer (its verifier, then its tasks' lines)
/// by index.
#[derive(Debug, Default)]
pub struct Iq {
    pub answers: std::collections::BTreeMap<u32, String>,
}

impl Iq {
    /// The report as the native runs are compared: `verifier <hash>: <n> tasks`, each task's line
    /// sorted, `<v> verified, <r> refused`, each ending in a newline; and its SHA-256.
    pub fn report(&self) -> (String, String) {
        let (mut verifier, mut lines) = (String::new(), Vec::new());
        for a in self.answers.values() {
            let mut parts = a.split('\t');
            verifier = parts.next().unwrap_or("").to_string();
            lines.extend(parts.map(String::from));
        }
        lines.sort();
        let refused = lines.iter().filter(|l| l.contains(" REFUSED ")).count();
        let n = lines.len();
        let head = ["verifier ", &verifier, ": ", &n.to_string(), " tasks"].concat();
        let foot =
            [&(n - refused).to_string(), " verified, ", &refused.to_string(), " refused"].concat();
        let mut text = String::new();
        for l in std::iter::once(&head).chain(&lines).chain([&foot]) {
            text.push_str(l);
            text.push('\n');
        }
        let hash = sha::hex(&sha::sha256(text.as_bytes()));
        (text, hash)
    }
}

impl PoolPage {
    /// Handles one event; whether it was the page's.
    pub fn event(&mut self, ev: &Event, requests: &mut Vec<Request>) -> bool {
        match ev {
            Event::Pool { data } => match Snap::decode(data) {
                Some(s) => self.take(s),
                None => return false,
            },
            Event::Click { id: SHOW } => requests.push(Request::Pair { code: String::new() }),
            Event::Click { id: TEST } => requests.push(Request::Test),
            Event::Click { id: FILL } => requests.push(Request::Memory),
            Event::Change { id: CODE, text, .. } => self.code.clone_from(text),
            Event::Click { id: JOIN } | Event::Submit { id: CODE } if !self.code.is_empty() => {
                requests.push(Request::Pair { code: self.code.clone() })
            }
            Event::Click { id: MEASURE } => requests.push(Request::Measure),
            Event::Click { id: FRACTAL } => requests.push(Request::Open { name: "fractal".into() }),
            Event::Click { id: VERIFY } => {
                self.iq = Some(Iq::default());
                let chunks = vec![IQ_SUITE.into()];
                requests.push(Request::Job { name: IQ_JOB.into(), chunks });
            }
            // A chunk's answer: `<made> <sha256> <answer>`.
            Event::Done { index, out, .. } if self.iq.is_some() => {
                let answer = out.splitn(3, ' ').nth(2).unwrap_or("");
                if let Some(q) = &mut self.iq {
                    q.answers.insert(*index, answer.into());
                }
            }
            Event::Change { id: SERVER, text, .. } => self.server.clone_from(text),
            Event::Click { id: SHARE } | Event::Submit { id: SERVER } => {
                let sharing = self.cur.as_ref().is_some_and(|s| !s.serve.is_empty());
                let url = match (sharing, self.server.trim()) {
                    (true, _) => String::new(),
                    (false, "") => LOCAL.into(),
                    (false, typed) => typed.into(),
                };
                requests.push(Request::Serve { url });
            }
            Event::Change { id, text, .. } if *id == self.field() => self.question.clone_from(text),
            Event::Click { id } | Event::Submit { id }
                if (*id == ASK || *id == self.field()) && !self.question.trim().is_empty() =>
            {
                // Asked: a new field, empty for the next question.
                requests.push(Request::Ask { text: self.question.trim().into() });
                (self.question, self.asked) = (String::new(), self.asked.wrapping_add(1));
            }
            Event::Click { id } if (SORT..SORT + 5).contains(id) => self.sort = (id - SORT) as u8,
            Event::Click { id } if (BOND..BOND + 1024).contains(id) => {
                let i = ((id - BOND) / 2) as usize;
                let verb = if (id - BOND) % 2 == 0 { "reconnect " } else { "unlink " };
                let bonds = self.cur.as_ref().map(|s| s.bonds.as_slice()).unwrap_or_default();
                let Some(b) = bonds.get(i) else { return false };
                requests.push(Request::Link { what: [verb, &b.key].concat() });
            }
            _ => return false,
        }
        true
    }

    /// A new snapshot: rates from the last, into the graphs.
    fn take(&mut self, s: Snap) {
        // Rates span half a second at least: two snapshots close together (one sent at once, to a
        // new watcher) would make any bytes between them a torrent.
        // The rest is taken as it is now: the devices too (a model's speed, what is busy), their
        // counters kept as they were for the rates. Else the last of a burst would be lost.
        if self.cur.as_ref().is_some_and(|p| s.at.wrapping_sub(p.at) < 500) {
            if let Some(c) = &mut self.cur {
                let mut devices = s.devices;
                for (i, d) in devices.iter_mut().enumerate() {
                    if let Some(o) = c.devices.get(i).filter(|o| o.name == d.name) {
                        (d.chunks, d.units, d.tx, d.rx) = (o.chunks, o.units, o.tx, o.rx);
                    }
                }
                (c.pairing, c.code, c.job, c.answer) = (s.pairing, s.code, s.job, s.answer);
                (c.bonds, c.testing, c.key) = (s.bonds, s.testing, s.key);
                (c.serve, c.serving, c.devices) = (s.serve, s.serving, devices);
            }
            return;
        }
        if let Some(p) = &self.cur {
            let secs = s.at.wrapping_sub(p.at).div_ceil(1000).max(1);
            let ms = u64::from(s.at.wrapping_sub(p.at).max(1));
            let sum =
                |snap: &Snap, f: fn(&pool::Device) -> u64| snap.devices.iter().map(f).sum::<u64>();
            let rate =
                |a: u64, b: u64| (b.saturating_sub(a) * 1000 / ms).min(u64::from(u32::MAX)) as u32;
            let chunks = rate(sum(p, |d| d.chunks.into()), sum(&s, |d| d.chunks.into()));
            self.graphs[1].add(secs, chunks);
            // Steps whole: a fractal takes billions a second, the IQ suite's mutants hundreds.
            let steps = sum(&s, |d| d.units).saturating_sub(sum(p, |d| d.units));
            self.steps = steps.saturating_mul(1000) / ms;
            for d in s.devices.iter().skip(1) {
                let Some(q) = p.devices.iter().find(|q| q.name == d.name) else { continue };
                let i = match self.links.iter().position(|l| l.0 == d.name) {
                    Some(i) => i,
                    None => {
                        self.links.push((d.name.clone(), History::default(), History::default()));
                        self.links.len() - 1
                    }
                };
                self.links[i].1.add(secs, rate(q.rx, d.rx));
                self.links[i].2.add(secs, rate(q.tx, d.tx));
            }
            self.graphs[0].level(secs, utilization(&s));
        } else {
            self.graphs[0].level(1, utilization(&s));
        }
        self.links.retain(|l| s.devices.iter().any(|d| d.name == l.0));
        self.prev = self.cur.replace(s);
    }

    /// The page: what it shows now, `wide` with cards two a row.
    pub fn nodes(&self, wide: bool) -> Vec<Node> {
        let mut nodes = vec![text(Style::Heading, "Pool")];
        let s = self.cur.clone().unwrap_or_default();
        let devices = &s.devices;
        let cores: u32 = devices.iter().map(|d| u32::from(d.cores)).sum();
        let quota: u64 = devices.iter().map(|d| u64::from(d.quota_mb)).sum();
        let mut line = count(devices.len().max(1), "device", "devices");
        if cores > 0 {
            line += &[DOT, &cores.to_string(), " cores"].concat();
        }
        // The pool's real compute and memory: what testing measured, summed.
        let tested: Vec<&pool::Device> = devices.iter().filter(|d| d.tested != 0).collect();
        if !tested.is_empty() {
            let cpu: u32 = tested.iter().map(|d| d.cpun).sum();
            let mem: u32 = tested.iter().map(|d| d.mem).sum();
            let floor = tested.iter().any(|d| d.floor);
            line += &[DOT, &rate(cpu), " CPU"].concat();
            if mem > 0 {
                line += &[DOT, &at_least(mem, floor), " usable RAM"].concat();
            }
            let untested = devices.len() - tested.len();
            if untested > 0 {
                line += &[" (", &untested.to_string(), " untested)"].concat();
            }
        }
        if quota > 0 {
            line += &[DOT, &storage(quota), " storage"].concat();
        }
        nodes.push(text(Style::Dim, &line));
        nodes.extend(bonded(&s));
        nodes.push(self.pairing(&s));
        // Busy and speed over the minute.
        let util = &self.graphs[0];
        let shown = util.now().map_or(DASH.into(), |u| [&(u / 10).to_string(), "%"].concat());
        let busy: u32 = devices.iter().map(|d| u32::from(d.busy)).sum();
        let workers: u32 = devices.iter().map(|d| u32::from(d.workers)).sum();
        let busy = [&busy.to_string(), " of ", &workers.to_string(), " workers busy"].concat();
        let util = card("Utilization", &shown, util.chart(11, 64, 1000), &busy);
        let speed = &self.graphs[1];
        let per = speed.now().map_or(DASH.into(), |c| [&c.to_string(), " chunks/s"].concat());
        let line = match self.prev {
            Some(_) => [&big(self.steps), " steps/s"].concat(),
            None => String::new(),
        };
        let speed = card("Speed", &per, speed.chart(2, 64, nice(speed.peak().max(10))), &line);
        if wide {
            nodes.push(Node::Row { id: 0, gap: 12, children: vec![util, speed] });
        } else {
            nodes.extend([util, speed]);
        }
        nodes.extend(self.devices(&s, wide));
        nodes.push(testing(&s));
        nodes.push(self.asking(&s));
        nodes.push(self.sharing(&s));
        nodes.extend(self.linked(&s));
        nodes.extend(job(&s));
        let fractal = "Render a fractal on the pool".into();
        nodes.push(Node::Button { id: FRACTAL, variant: Variant::Primary, label: fractal });
        nodes.push(self.verifying(&s));
        nodes.push(text(Style::Small, NOTE));
        nodes
    }

    /// The IQ suite on the pool: a button, then its progress, then its report's hash and time.
    fn verifying(&self, s: &Snap) -> Node {
        let mut children = vec![text(Style::Small, "The IQ suite, verified on the pool")];
        let job = s.job.as_ref().filter(|j| j.mine && j.name == IQ_JOB);
        match (&self.iq, job) {
            (Some(q), Some(j)) if j.total > 0 && q.answers.len() as u32 >= j.total => {
                let (report, hash) = q.report();
                let foot = report.lines().last().unwrap_or("");
                let secs =
                    [&(j.ms / 1000).to_string(), ".", &(j.ms % 1000 / 100).to_string(), " s"]
                        .concat();
                children.push(text(Style::Title, &[foot, DOT, &secs].concat()));
                children.push(text(Style::Mono, &["sha256 ", &hash].concat()));
            }
            (Some(q), Some(j)) => {
                let line = [&q.answers.len().to_string(), " of ", &j.total.to_string(), " chunks"]
                    .concat();
                children.push(text(Style::Body, &line));
            }
            (Some(_), None) => children.push(text(Style::Body, "Starting")),
            (None, _) => {}
        }
        let label = "Verify the IQ suite".into();
        children.push(Node::Button { id: VERIFY, variant: Variant::Normal, label });
        Node::Card { id: 0, children: vec![Node::Col { id: 0, gap: 6, children }] }
    }

    /// The question field's id now.
    fn field(&self) -> u32 {
        QUESTION + (self.asked & 0xffff)
    }

    /// The pool's model asked: the question field and Ask, then the answer as it is written,
    /// whose model wrote it and how fast, or why it failed.
    fn asking(&self, s: &Snap) -> Node {
        let mut children = vec![text(Style::Small, "Ask the pool's model")];
        if s.devices.iter().all(|d| d.model.is_empty()) {
            children.push(text(Style::Small, "No device shares a model yet."));
        }
        let field = Node::Input {
            id: self.field(),
            value: self.question.clone(),
            placeholder: "A question".into(),
        };
        let ask = Node::Button { id: ASK, variant: Variant::Primary, label: "Ask".into() };
        children.push(Node::Row { id: 0, gap: 8, children: vec![field, ask] });
        if let Some(a) = &s.answer {
            children.push(text(Style::Small, &a.question));
            if !a.text.is_empty() {
                children.push(text(Style::Body, &a.text));
            }
            let by = if a.by.is_empty() { String::new() } else { ["by ", &a.by].concat() };
            let line = match (a.done, a.tok) {
                (false, _) => [&by, DOT, "writing"].concat(),
                (true, 0) => by,
                (true, t) => [&by, DOT, &speed(t)].concat(),
            };
            children.push(text(Style::Small, line.trim_start_matches(DOT)));
            if !a.why.is_empty() {
                children.push(text(Style::Error, &a.why));
            }
        }
        Node::Card { id: 0, children: vec![Node::Col { id: 0, gap: 6, children }] }
    }

    /// This device's model to share: what sharing says, the server field and Share (or Stop).
    fn sharing(&self, s: &Snap) -> Node {
        let mut children = vec![text(Style::Small, "Share this device's model")];
        let me = s.devices.first();
        let say = match me.map(|d| (d.model.as_str(), d.tok)) {
            _ if !s.serving.is_empty() => s.serving.clone(),
            Some((model, tok)) if !model.is_empty() => {
                let ctx = me.map_or(0, |d| d.ctx);
                let ctx = if ctx == 0 { String::new() } else { [DOT, &context(ctx)].concat() };
                ["Sharing ", model, DOT, &speed(tok), &ctx].concat()
            }
            _ => "Run an OpenAI-compatible server on this device (as llama-server), then share \
                  it: linked devices may ask it, and only they."
                .into(),
        };
        children.push(text(Style::Small, &say));
        let sharing = !s.serve.is_empty();
        let value = if self.server.is_empty() { s.serve.clone() } else { self.server.clone() };
        let field = Node::Input { id: SERVER, value, placeholder: LOCAL.into() };
        let label = if sharing { "Stop sharing" } else { "Share" }.into();
        let share = Node::Button { id: SHARE, variant: Variant::Normal, label };
        children.push(Node::Row { id: 0, gap: 8, children: vec![field, share] });
        Node::Card { id: 0, children: vec![Node::Col { id: 0, gap: 6, children }] }
    }

    /// Linking a new device: the code shown, or what pairing says; Show a code, or enter the
    /// other's.
    fn pairing(&self, s: &Snap) -> Node {
        let mut children = vec![text(Style::Small, "Link a new device")];
        if !s.code.is_empty() {
            children.push(text(Style::Title, &s.code));
        }
        let say = if s.pairing.is_empty() {
            "Open computehub on the other device: show a code here and enter it there, or the other \
             way. Linked once, the two find each other again whenever both are open."
        } else {
            &s.pairing
        };
        children.push(text(Style::Small, say));
        // This device's key, as the other device shows it among its linked devices.
        if !s.key.is_empty() {
            children.push(text(Style::Small, &["This device's key ", short(&s.key)].concat()));
        }
        let show = Node::Button { id: SHOW, variant: Variant::Normal, label: "Show a code".into() };
        let field = Node::Input { id: CODE, value: self.code.clone(), placeholder: "Code".into() };
        let join = Node::Button { id: JOIN, variant: Variant::Primary, label: "Link".into() };
        children.push(Node::Row { id: 0, gap: 8, children: vec![show, field, join] });
        Node::Card { id: 0, children: vec![Node::Col { id: 0, gap: 6, children }] }
    }

    /// The devices, sorted by the column picked, and each one's share of the work.
    fn devices(&self, s: &Snap, wide: bool) -> Vec<Node> {
        let mut rows: Vec<(usize, &pool::Device)> = s.devices.iter().enumerate().collect();
        let total: u64 = s.devices.iter().map(|d| u64::from(d.chunks)).sum();
        let key = |d: &pool::Device| match self.sort {
            1 => u64::from(d.cpun),
            2 => u64::from(d.mem),
            3 => u64::from(d.quota_mb),
            _ => u64::from(d.chunks),
        };
        if self.sort == 0 {
            rows.sort_by(|a, b| a.1.name.cmp(&b.1.name));
        } else {
            rows.sort_by_key(|r| std::cmp::Reverse(key(r.1)));
        }
        let labels = if wide { "Device\tCPU\tRAM\tStorage\tShare" } else { "Device\tCPU\tShare" };
        let on = if wide || matches!(self.sort, 0 | 1 | 4) { self.sort + 1 } else { 0 };
        let mut nodes = vec![Node::Columns { id: SORT, on, labels: labels.into() }];
        for (i, d) in &rows {
            let share = (u64::from(d.chunks) * 1000).checked_div(total).unwrap_or(0);
            let pct = [&(share / 10).to_string(), "%"].concat();
            // Measured, never the browser's guess: a dash until the device is tested.
            let tested = d.tested != 0;
            let cpu = if tested { rate(d.cpun) } else { DASH.into() };
            let ram = if tested && d.mem > 0 { at_least(d.mem, d.floor) } else { DASH.into() };
            let quota = if d.quota_mb == 0 { DASH.into() } else { storage(d.quota_mb.into()) };
            let cells = match wide {
                true => [cpu, ram, quota, pct].join("\t"),
                false => [cpu, pct].join("\t"),
            };
            // What tells devices apart first (a row's second line is cut to fit): this tab, or
            // whether its key was pinned before and how long it is linked; then its GPU and kind.
            let gpu = if d.gpu { "GPU compute OK" } else { "GPU compute off" };
            let pinned = if d.known { "known" } else { "new" };
            let first = match i {
                0 => "this tab".to_string(),
                _ => [pinned, DOT, "linked ", &minutes(d.up_ms)].concat(),
            };
            let model = match d.model.as_str() {
                "" => String::new(),
                m => [DOT, m, " ", &speed(d.tok)].concat(),
            };
            let one = match d.tested {
                0 => String::new(),
                _ => [DOT, "one core ", &rate(d.cpu1)].concat(),
            };
            let cores = [DOT, &d.cores.to_string(), " cores"].concat();
            let about = [&first, &model, &one, &cores, DOT, gpu, DOT, &d.kind].concat();
            let name = if d.name.is_empty() { "A device" } else { &d.name };
            let text = [name, "\n", &about].concat();
            let hue = RGB[i % RGB.len()];
            nodes.push(Node::Entry {
                id: 0,
                glyph: icons::Glyph::Mark as u8,
                hue,
                text,
                detail: cells,
                more: false,
            });
        }
        // The devices linked for good and away: greyed, with what they last said.
        for b in s.bonds.iter().filter(|b| b.state != pool::ONLINE) {
            let tested = b.tested != 0;
            let cpu = if tested { rate(b.cpun) } else { DASH.into() };
            let ram = if tested && b.mem > 0 { at_least(b.mem, b.floor) } else { DASH.into() };
            let cells = match wide {
                true => [cpu, ram, DASH.into(), DASH.into()].join("\t"),
                false => [cpu, DASH.into()].join("\t"),
            };
            let name = if b.name.is_empty() { "A device" } else { &b.name };
            let text = [name, "\n", &away(b)].concat();
            let (glyph, hue) = (icons::Glyph::Mark as u8, AWAY);
            nodes.push(Node::Entry { id: 0, glyph, hue, text, detail: cells, more: false });
        }
        // Each device's share in its color, in the table's order, named.
        nodes.push(text(Style::Small, "Share of the pool's work"));
        for (i, d) in &rows {
            let value = (u64::from(d.chunks) * 1000).checked_div(total).unwrap_or(0) as u16;
            let name = if d.name.is_empty() { "A device" } else { &d.name };
            let pct = [&(value / 10).to_string(), "%"].concat();
            nodes.push(text(Style::Small, &[name, DOT, &pct].concat()));
            nodes.push(Node::Meter { id: 0, hue: pool::TINTS[i % pool::TINTS.len()], value });
        }
        nodes
    }

    /// Each link: its round trip and traffic over the minute both ways, measured throughput.
    fn linked(&self, s: &Snap) -> Vec<Node> {
        if s.devices.len() < 2 {
            return Vec::new();
        }
        let mut nodes = vec![text(Style::Heading, "Links")];
        for d in s.devices.iter().skip(1) {
            let rtt = if d.rtt == pool::UNKNOWN {
                DASH.into()
            } else {
                [&d.rtt.to_string(), " ms"].concat()
            };
            let measured =
                |b: u32| if b == 0 { DASH.into() } else { [&bytes(b.into()), "/s"].concat() };
            let line = [
                "Round trip ",
                &rtt,
                DOT,
                "measured \u{2191} ",
                &measured(d.up),
                " \u{2193} ",
                &measured(d.down),
            ]
            .concat();
            let (down, up) = match self.links.iter().find(|l| l.0 == d.name) {
                Some(l) => (l.1.clone(), l.2.clone()),
                None => Default::default(),
            };
            let full = nice(down.peak().max(up.peak()).max(1000));
            let now =
                |h: &History| h.now().map_or(DASH.into(), |b| [&bytes(b.into()), "/s"].concat());
            let children = vec![
                text(Style::Small, &d.name),
                text(Style::Body, &line),
                text(Style::Small, &["Heard ", &now(&down)].concat()),
                down.chart(6, 40, full),
                text(Style::Small, &["Sent ", &now(&up)].concat()),
                up.chart(3, 40, full),
            ];
            nodes.push(Node::Card { id: 0, children: vec![Node::Col { id: 0, gap: 6, children }] });
        }
        nodes.push(Node::Button {
            id: MEASURE,
            variant: Variant::Normal,
            label: "Measure links".into(),
        });
        nodes
    }
}

/// The share of the pool's workers busy, per mille.
fn utilization(s: &Snap) -> u32 {
    let busy: u32 = s.devices.iter().map(|d| u32::from(d.busy)).sum();
    let workers: u32 = s.devices.iter().map(|d| u32::from(d.workers)).sum();
    (busy * 1000).checked_div(workers).unwrap_or(0)
}

/// The job: chunks done and waiting, each device's, work stealing, checks, time and what is left.
fn job(s: &Snap) -> Vec<Node> {
    let Some(j) = &s.job else { return Vec::new() };
    let mut children = vec![text(Style::Small, &["Job", DOT, &j.name].concat())];
    let secs =
        |ms: u32| [&(ms / 1000).to_string(), ".", &(ms % 1000 / 100).to_string(), " s"].concat();
    if !j.mine {
        children.push(text(Style::Title, &count(j.done as usize, "chunk", "chunks")));
        // Busy until its last answer here; the rest of the job's time, other devices' tails.
        let line = ["answered here for another device: busy ", &secs(j.busy), " of ", &secs(j.ms)];
        let line = line.concat();
        children.push(text(Style::Small, &line));
        // Answers here that came second: their chunks were taken back and answered there first.
        if j.used > 0 || j.ms > 0 {
            let late = j.done.saturating_sub(j.used);
            let used =
                [&j.used.to_string(), " used", DOT, &late.to_string(), " answered first there"];
            children.push(text(Style::Small, &used.concat()));
        }
        return vec![Node::Card { id: 0, children: vec![Node::Col { id: 0, gap: 6, children }] }];
    }
    let left = j.total - j.done.min(j.total);
    children
        .push(text(Style::Title, &[&j.done.to_string(), " of ", &j.total.to_string()].concat()));
    let value = (j.done * 1000).checked_div(j.total).unwrap_or(1000) as u16;
    children.push(Node::Meter { id: 0, hue: 11, value });
    let eta = match (left, j.done) {
        (0, _) => ["done in ", &secs(j.ms)].concat(),
        (_, 0) => [&secs(j.ms), " so far"].concat(),
        _ => {
            let rest = u64::from(j.ms) * u64::from(left) / u64::from(j.done);
            [&secs(j.ms), " so far", DOT, "about ", &secs(rest as u32), " left"].concat()
        }
    };
    children.push(text(Style::Body, &[&left.to_string(), " left", DOT, &eta].concat()));
    let per: Vec<String> = s
        .devices
        .iter()
        .zip(&j.per)
        .map(|(d, n)| [&d.name, " ", &n.to_string()].concat())
        .collect();
    children.push(text(Style::Small, &per.join(DOT)));
    let steal = [
        &count(j.steals as usize, "chunk", "chunks"),
        " taken back from slower devices",
        DOT,
        &j.requeued.to_string(),
        " requeued",
        DOT,
        &j.checked.to_string(),
        " checked by replay, ",
        &j.mismatched.to_string(),
        " differed",
    ]
    .concat();
    children.push(text(if j.mismatched > 0 { Style::Error } else { Style::Small }, &steal));
    vec![Node::Card { id: 0, children: vec![Node::Col { id: 0, gap: 6, children }] }]
}

/// MB of storage in decimal units: `740 MB`, `25.7 GB`, `1.2 TB`.
fn storage(mb: u64) -> String {
    let tenths =
        |n: u64, unit: &str| [&(n / 10).to_string(), ".", &(n % 10).to_string(), unit].concat();
    match mb {
        0..1000 => [&mb.to_string(), " MB"].concat(),
        1000..1_000_000 => tenths(mb / 100, " GB"),
        _ => tenths(mb / 100_000, " TB"),
    }
}

/// This device's test: what it measured and when (the CPU's half kept while the memory's runs),
/// then what the test says (under way, or what stopped it); Test.
fn testing(s: &Snap) -> Node {
    let mut children = vec![text(Style::Small, "Test this device")];
    let me = s.devices.first().filter(|d| d.tested != 0);
    let said = match me {
        None if !s.testing.is_empty() => String::new(),
        Some(d) => [
            // A device measured for its memory alone says so for its CPU.
            &match d.cpu1 {
                0 => "CPU not measured".to_string(),
                c => ["One core ", &rate(c), DOT, "all cores ", &rate(d.cpun)].concat(),
            },
            DOT,
            "memory ",
            &match (d.mem, d.floor) {
                (0, _) => "not measured".into(),
                (m, true) => ["at least ", &memory(m), " usable"].concat(),
                (m, false) => [&memory(m), " usable"].concat(),
            },
            DOT,
            &ago(d.tested),
        ]
        .concat(),
        None => "Not tested yet.".into(),
    };
    for line in [&said, &s.testing] {
        if !line.is_empty() {
            children.push(text(Style::Body, line));
        }
    }
    let how = "Measured, not reported: SHA-256 on one core, then on all at once; then the memory \
               this tab can really hold, never more than 2 GB (a quarter of the device's memory if \
               the browser says it is less), stopping at once at a slow step, where a device short \
               of memory starts to swap. Stopped by the test's own limit, the figure is a floor: \
               at least that much (2.0+ GB).";
    children.push(text(Style::Small, how));
    let all = "Measure all memory fills this tab until the device shows the first sign of \
               swapping (its oldest pages slowing down), then frees it all at once: the memory it \
               can really give. Other apps may slow for a moment while it runs.";
    children.push(text(Style::Small, all));
    let label = if me.is_some() { "Test again" } else { "Test this device" }.into();
    let test = Node::Button { id: TEST, variant: Variant::Normal, label };
    let fill =
        Node::Button { id: FILL, variant: Variant::Normal, label: "Measure all memory".into() };
    children.push(Node::Row { id: 0, gap: 8, children: vec![test, fill] });
    Node::Card { id: 0, children: vec![Node::Col { id: 0, gap: 6, children }] }
}

/// A speed in MiB a second, as people say it: `256 MB/s`, `3.1 GB/s`.
fn rate(mib: u32) -> String {
    match mib {
        0..1024 => [&mib.to_string(), " MB/s"].concat(),
        m => [&(m / 1024).to_string(), ".", &(m % 1024 * 10 / 1024).to_string(), " GB/s"].concat(),
    }
}

/// Memory in MiB, a floor said as one, short: `2.0+ GB` (the test stopped before the device did).
/// In words, not `≥`: the desktop's fonts are subset, and that sign is not in them.
fn at_least(mib: u32, floor: bool) -> String {
    let said = memory(mib);
    match said.rsplit_once(' ') {
        Some((n, unit)) if floor => [n, "+ ", unit].concat(),
        _ => said,
    }
}

/// Memory in MiB, as people say it: `960 MB`, `9.4 GB`.
fn memory(mib: u32) -> String {
    match mib {
        0..1024 => [&mib.to_string(), " MB"].concat(),
        m => [&(m / 1024).to_string(), ".", &(m % 1024 * 10 / 1024).to_string(), " GB"].concat(),
    }
}

/// When testing measured, at Unix time `at`: `measured just now`, `measured 5 min ago`.
fn ago(at: u32) -> String {
    ["measured ", &since(at)].concat()
}

/// How long ago Unix time `at` was, as people say it: `just now`, `5 min ago`, `3 h ago`.
fn since(at: u32) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    let secs = now.map_or(0, |d| d.as_secs()).saturating_sub(u64::from(at));
    match secs {
        0..60 => "just now".into(),
        s @ 60..3600 => [&(s / 60).to_string(), " min ago"].concat(),
        s @ 3600..172_800 => [&(s / 3600).to_string(), " h ago"].concat(),
        s => [&(s / 86_400).to_string(), " days ago"].concat(),
    }
}

/// A linked device not here: `offline · last seen 5 min ago`, or `connecting`.
fn away(b: &pool::Bond) -> String {
    match (b.state, b.seen) {
        (pool::CONNECTING, _) => "connecting".into(),
        (_, 0) => "offline".into(),
        (_, at) => ["offline", DOT, "last seen ", &since(at)].concat(),
    }
}

/// A key said short: its first three bytes, `AB:CD:EF`.
fn short(key: &str) -> &str {
    let hex = key.strip_prefix("sha-256 ").unwrap_or(key);
    hex.get(..8).unwrap_or(hex)
}

/// The devices linked for good: each one here or not (when last seen), what it last said it
/// has, its key, Reconnect while it is away, and Unlink.
fn bonded(s: &Snap) -> Vec<Node> {
    if s.bonds.is_empty() {
        return Vec::new();
    }
    let mut children = vec![text(Style::Small, "Linked devices")];
    for (i, b) in s.bonds.iter().enumerate() {
        let here = b.state == pool::ONLINE;
        let state = if here { "online".into() } else { away(b) };
        let figures = match b.tested {
            0 => String::new(),
            _ => [DOT, "all cores ", &rate(b.cpun)].concat(),
        };
        let line = [&state, &figures, DOT, "key ", short(&b.key)].concat();
        let name = if b.name.is_empty() { "A device" } else { &b.name };
        let about = vec![text(Style::Body, name), text(Style::Small, &line)];
        let mut row = vec![Node::Col { id: 0, gap: 2, children: about }];
        let id = BOND + 2 * i as u32;
        if !here {
            row.push(Node::Button { id, variant: Variant::Normal, label: "Reconnect".into() });
        }
        row.push(Node::Button { id: id + 1, variant: Variant::Normal, label: "Unlink".into() });
        children.push(Node::Row { id: 0, gap: 8, children: row });
    }
    vec![Node::Card { id: 0, children: vec![Node::Col { id: 0, gap: 8, children }] }]
}

/// A model's speed from tenths of a token a second: `9.5 tok/s`; unmeasured, a dash.
fn speed(tenths: u32) -> String {
    match tenths {
        0 => [DASH, " tok/s"].concat(),
        t => [&(t / 10).to_string(), ".", &(t % 10).to_string(), " tok/s"].concat(),
    }
}

/// A model's context in tokens, as people say it: `4k context`, `32k context`, `512 context`.
fn context(tokens: u32) -> String {
    match tokens {
        0..1024 => [&tokens.to_string(), " context"].concat(),
        t => [&(t / 1024).to_string(), "k context"].concat(),
    }
}

/// ms as people say how long: `40 s`, `3 min`, `2 h`.
fn minutes(ms: u32) -> String {
    match ms / 1000 {
        s @ 0..60 => [&s.to_string(), " s"].concat(),
        s @ 60..3600 => [&(s / 60).to_string(), " min"].concat(),
        s => [&(s / 3600).to_string(), " h"].concat(),
    }
}

/// A count as people say it: `412`, `9.4k`, `12.4M`, `3.1G`, `2.0T`.
fn big(n: u64) -> String {
    let (scale, unit) = match n {
        0..1000 => return n.to_string(),
        1000..1_000_000 => (1000, "k"),
        1_000_000..1_000_000_000 => (1_000_000, "M"),
        1_000_000_000..1_000_000_000_000 => (1_000_000_000, "G"),
        _ => (1_000_000_000_000, "T"),
    };
    [&(n / scale).to_string(), ".", &(n % scale / (scale / 10)).to_string(), unit].concat()
}
