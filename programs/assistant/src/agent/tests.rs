use std::cell::{Cell, RefCell};
use std::rc::Rc;

use chats::{CHAT, COMPACT, NEW};
use host::{Effect, Host, OVERLAY, Registry, Response};
use ui::{App, AppEvent, Cx, TextSystem, Ui};
use uiwire::scene::{Hit, Mark, Run, Win};
use vfs::Vfs;
use wm::{Rect, Wm};

use super::compact::{NOTE, SUM};
use super::*;
use crate::calls::encode;
use crate::look::render;

const SANS: &[u8] = include_bytes!("../../../../assets/fonts/Inter-Regular.ttf");

type Shared<T> = Rc<RefCell<T>>;

/// The overlay on a real desktop: what the agent asks goes out through `cx.agent`; what the host
/// answers is kept.
struct Hand(Shared<Vec<Request>>, Shared<Vec<Event>>);

impl App for Hand {
    fn title(&self) -> String {
        "Assistant".into()
    }
    fn draw(&mut self, _: &mut Ui<'_>) {}
    fn event(&mut self, ev: AppEvent, cx: &mut Cx<'_>) -> bool {
        if let AppEvent::Agent(e) = ev {
            self.1.borrow_mut().push(e);
        }
        self.0.take().into_iter().for_each(|r| cx.agent(r));
        false
    }
}

/// Settings as the desktop runs it, in-process: the system program's frames drawn by uiview, its
/// requests honored as the desktop honors its window's, and what Settings shows told on a change.
#[derive(Default)]
struct Own {
    app: system::Settings,
    nodes: Vec<Node>,
    texts: uiview::Texts,
    view: uiview::View,
    told: Option<[bool; 3]>,
}

/// No files: Settings reaches none.
struct NoDisk;

impl system::Disk for NoDisk {
    fn list(&mut self, _: &str) -> std::io::Result<Vec<system::Entry>> {
        Err(std::io::ErrorKind::NotFound.into())
    }
    fn read(&mut self, _: &str) -> std::io::Result<Vec<u8>> {
        Err(std::io::ErrorKind::NotFound.into())
    }
    fn write(&mut self, _: &str, _: &[u8]) -> std::io::Result<()> {
        Err(std::io::ErrorKind::PermissionDenied.into())
    }
}

impl App for Own {
    fn title(&self) -> String {
        "Settings".into()
    }
    fn draw(&mut self, ui: &mut Ui<'_>) {
        uiview::draw(ui, &self.nodes, &mut self.texts, &mut self.view);
    }
    fn event(&mut self, ev: AppEvent, cx: &mut Cx<'_>) -> bool {
        use system::View;
        let now = [!cx.ai.reports_off, cx.grain, !cx.ai.unkept];
        if self.told.replace(now) != Some(now) {
            let [reports, grain, kept] = now;
            self.app.event(&Event::Prefs { reports, grain, kept }, &mut NoDisk);
        }
        let ev = match ev {
            AppEvent::Resized { w, h } => Event::Resize { w: w as u16, h: h as u16 },
            AppEvent::Click(ui::WidgetId(id)) => Event::Click { id },
            AppEvent::Agent(ev) => ev,
            _ => Event::Focus { on: true },
        };
        self.app.event(&ev, &mut NoDisk);
        let frame = self.app.frame();
        for r in frame.requests {
            match r {
                Request::Pref { key, value } if key == "theme" => cx.set_theme(&value),
                Request::Pref { key, value } => cx.pref(&key, &value),
                Request::Open { name } => cx.open(&name),
                _ => {}
            }
        }
        self.nodes = frame.nodes;
        true
    }
}

/// A 1440 x 900 desktop in Mono with Settings and a terminal to open, its overlay running; what
/// the model was asked, what the desktop did, and each act's flash.
struct Desk {
    host: Host,
    out: Shared<Vec<Request>>,
    heard: Shared<Vec<Event>>,
    bodies: Vec<Json>,
    effects: Vec<Effect>,
    flashes: usize,
}

fn desk() -> Desk {
    let (out, heard) = (Shared::default(), Shared::default());
    let (o, h) = (out.clone(), heard.clone());
    let registry: Registry = Box::new(move |name| match name {
        "assistant" => Some(Box::new(Hand(o.clone(), h.clone())) as Box<dyn App>),
        "settings" => Some(Box::new(Own::default())),
        _ => None,
    });
    let (wm, text) = (Wm::new(Rect::new(0, 44, 1440, 771)), TextSystem::new(SANS.to_vec()));
    let mut host = Host::new(wm, text.unwrap(), Vfs::new(), registry, "Mono");
    host.agent.screen = (1440.0, 900.0);
    host.agent.apps = ["settings", "terminal"].map(String::from).into();
    assert!(host.open_overlay());
    Desk { host, out, heard, bodies: Vec::new(), effects: Vec::new(), flashes: 0 }
}

impl Desk {
    /// Serves `agent` until it asks nothing more: the model answers each request as `model`
    /// says, in 7-byte chunks; acts, status and steps aside go to the host as the overlay's.
    fn run(&mut self, agent: &mut Agent, model: &dyn Fn(&Json) -> String) {
        for _ in 0..300 {
            let mut events = Vec::new();
            for r in agent.frame().requests {
                match r {
                    Request::Ai { id, body } => {
                        let body = Json::parse(&body).expect("a JSON body");
                        let sse = model(&body);
                        self.bodies.push(body);
                        let data =
                            sse.as_bytes().chunks(7).map(|c| Event::AiData { id, data: c.into() });
                        let end = Event::AiEnd { id, status: 200, error: "".into() };
                        events.extend(data.chain([end]));
                    }
                    r if r.overlay() => {
                        self.out.borrow_mut().push(r);
                        self.host.now_ms += 100.0;
                        let (mut out, now) = (Response::default(), self.host.now_ms);
                        self.host.deliver(OVERLAY, AppEvent::Tick { now_ms: now }, &mut out);
                        self.effects.extend(out.effects);
                        self.flashes +=
                            usize::from(self.host.agent.flash.is_some_and(|f| f.1 == now));
                    }
                    _ => {}
                }
            }
            events.extend(self.heard.take());
            if events.is_empty() {
                // Time passes for an act that waits: the platform's timer fires.
                self.host.now_ms += 6000.0;
                self.host.kernel_in(host::KernelIn::Wake, &mut Response::default());
                events.extend(self.heard.take());
            }
            if events.is_empty() {
                return;
            }
            events.iter().for_each(|e| _ = agent.event(e));
        }
        panic!("the agent never settled");
    }
}

/// The SSE event ending a reply out of room.
const LENGTH: &str =
    "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"length\"}]}\n\n";

/// An SSE event carrying `delta`.
fn sse(delta: &str) -> String {
    format!("data: {{\"choices\":[{{\"index\":0,\"delta\":{delta}}}]}}\n\n")
}

/// A reply saying `text`, then calling `call` (its arguments in 5-byte pieces), then its usage.
fn reply(text: &str, call: Option<(&str, &str)>) -> String {
    let mut out = if text.is_empty() {
        String::new()
    } else {
        sse(&format!("{{\"content\":{}}}", quote(text)))
    };
    if let Some((name, args)) = call {
        let head = format!(
            "{{\"index\":0,\"id\":\"call_{name}\",\"type\":\"function\",\"function\":{{\"name\":\"{name}\",\"arguments\":\"\"}}}}"
        );
        out += &sse(&format!("{{\"tool_calls\":[{head}]}}"));
        for p in args.as_bytes().chunks(5).map(|p| std::str::from_utf8(p).unwrap()) {
            out += &sse(&format!(
                "{{\"tool_calls\":[{{\"index\":0,\"function\":{{\"arguments\":{}}}}}]}}",
                quote(p)
            ));
        }
    }
    let finish = if call.is_some() { "tool_calls" } else { "stop" };
    out += &format!(
        "data: {{\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"{finish}\"}}]}}\n\n"
    );
    out + "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":1200,\"completion_tokens\":30}}\n\ndata: [DONE]\n\n"
}

/// A reply calling each of `calls` (name and arguments), whole, by index.
fn many(calls: &[(&str, String)]) -> String {
    let call = |(i, (name, args)): (usize, &(&str, String))| {
        let f = format!("{{\"name\":\"{name}\",\"arguments\":{}}}", quote(args));
        format!("{{\"index\":{i},\"id\":\"c{i}\",\"type\":\"function\",\"function\":{f}}}")
    };
    let all: Vec<String> = calls.iter().enumerate().map(call).collect();
    sse(&format!("{{\"tool_calls\":[{}]}}", all.join(","))) + "data: [DONE]\n\n"
}

/// The messages of a request body as (role, content).
fn messages(body: &Json) -> Vec<(String, String)> {
    let m = |i| body.get("messages")?.at(i);
    let text = |v: Option<&Json>| v.and_then(Json::text).unwrap_or("").to_string();
    (0..).map_while(m).map(|m| (text(m.get("role")), text(m.get("content")))).collect()
}

/// The model that turns error reports off, reading the latest screen: Settings, its Privacy
/// tab, the switch, closing Settings, then the answer.
fn reports_off(body: &Json) -> String {
    let all = messages(body);
    let screen = all.iter().rev().find(|m| m.1.contains("Screen ")).map_or("", |m| m.1.as_str());
    let line = |want: &str| screen.lines().find(|l| l.contains(want)).map(|l| l.trim().to_string());
    let first = |l: String| l.split(' ').next().unwrap_or("").to_string();
    let click = |r: String| reply("", Some(("click", &format!("{{\"ref\":\"{r}\"}}"))));
    if !all.iter().any(|m| m.0 == "tool") {
        return reply("", Some(("open_app", r#"{"name":"settings"}"#)));
    }
    if let Some(l) = line("tab \"Privacy\"").filter(|l| !l.ends_with("selected")) {
        return click(first(l));
    }
    if let Some(l) = line("switch \"Send error reports automatically\" on") {
        return click(first(l));
    }
    if let Some(w) = line("(settings)").map(first) {
        let close = format!("{{\"window\":\"{w}\",\"action\":\"close\"}}");
        return reply("", Some(("window", &close)));
    }
    reply("Error reports are off.", None)
}

fn ask(a: &mut Agent, prompt: &str) {
    a.event(&Event::Change { id: a.input_id(), version: 1, text: prompt.into() });
    a.event(&Event::Submit { id: a.input_id() });
}

/// Every Text's text in a frame, one per line.
fn shown(a: &mut Agent) -> String {
    let texts = a.frame().nodes.into_iter().filter_map(|n| match n {
        Node::Text { text, .. } => Some(text),
        _ => None,
    });
    texts.collect::<Vec<_>>().join("\n")
}

/// The ids of the act and of the request to the model that frame `f` asks, if it does.
fn ids(f: &Frame) -> (Option<u32>, Option<u32>) {
    let id = |ai: bool| {
        f.requests.iter().find_map(|r| match r {
            Request::Act { id, .. } if !ai => Some(*id),
            Request::Ai { id, .. } if ai => Some(*id),
            _ => None,
        })
    };
    (id(false), id(true))
}

/// A task on `scene` as `a` sees it, opened over its focus, waiting on nothing.
#[rustfmt::skip]
fn on(a: &mut Agent, scene: Scene) -> Task {
    let (screen, shown) = render(&scene, &mut a.refs, scene.focus);
    Task { prompt: String::new(), over: scene.focus, steps: Vec::new(), screen, scene, shown,
        wait: Wait::User, calls: 0, acts: 0, fails: 0, repeat: (0, 0), approved: false,
        usage: (0, 0), into: (0, false) }
}

#[test]
fn a_scripted_model_turns_error_reports_off_on_a_real_desktop() {
    let (mut d, mut a) = (desk(), Agent::default());
    a.event(&Event::Resize { w: 560, h: 480 });
    ask(&mut a, "turn off error reports");
    d.run(&mut a, &reports_off);
    // Settings opened, its Privacy tab, the switch flipped, Settings closed: four acts, each
    // flashed, and five model calls with the answer.
    assert_eq!((d.bodies.len(), d.flashes), (5, 4));
    assert!(d.effects.contains(&Effect::Pref { key: "reports".into(), value: "off".into() }));
    let settings = d.host.wins.iter().find(|w| w.name == "settings").map(|w| w.id).unwrap();
    assert!(!d.host.live(settings) && !d.host.agent.working);
    let said = shown(&mut a);
    let lines =
        ["Opening settings", "Clicking \u{201c}Privacy\u{201d}", "Closing \u{201c}Settings"];
    assert!(lines.iter().all(|l| said.contains(l)), "{said}");
    assert!(said.ends_with("Error reports are off.\n4 steps, 6.0k tokens in, 150 out"), "{said}");
    // The first request: the system prompt, the tools, the task and the screen, read as text.
    let b = &d.bodies[0];
    let get = |k| b.get(k).map(encode).unwrap_or_default();
    let got = [get("max_tokens"), get("temperature"), get("tool_choice")];
    assert_eq!(got, ["2048", "0.2", "\"auto\""]);
    assert_eq!(get("tools"), TOOLS);
    let m = messages(b);
    assert_eq!((m.len(), m[0].0.as_str(), &*m[0].1), (2, "system", SYSTEM));
    assert!(m[1].1.starts_with("turn off error reports\n\nScreen 1440x900, theme Mono."));
    assert!(m[1].1.contains("\nApps: settings, terminal\n"));
    // Folded: the first screen left out, older results their first line, the latest the screen.
    let m = messages(&d.bodies[3]);
    assert_eq!(m[1].1, "turn off error reports\n\n(screen omitted)");
    let tools: Vec<&str> = m.iter().filter(|m| m.0 == "tool").map(|m| m.1.as_str()).collect();
    assert_eq!(tools[..2], ["ok: opened settings", "ok: clicked \u{201c}Privacy\u{201d} (e4)"]);
    let flip = "ok: clicked \u{201c}Send error reports automatically\u{201d} (e";
    assert!(tools[2].starts_with(flip));
    assert!(tools[2].contains("switch \"Send error reports automatically\" off"));
    // The next task remembers this one; the same refs name the same widgets.
    ask(&mut a, "now turn it back on");
    d.run(&mut a, &|_| reply("Done.", None));
    let m = messages(d.bodies.last().unwrap());
    let pair = (m[1].1.as_str(), m[2].1.as_str());
    assert_eq!(pair, ("turn off error reports", "Error reports are off."));
}

#[test]
fn tool_calls_are_collected_however_they_are_split() {
    let mut c = Calls::default();
    let body = reply("Sure.", Some(("click", r#"{"ref":"e4"}"#)));
    body.as_bytes().chunks(3).for_each(|p| c.feed(p));
    c.end();
    let args = r#"{"ref":"e4"}"#.into();
    let call = Call { id: "call_click".into(), name: "click".into(), args, cut: false };
    let got = (c.text.as_str(), &c.calls[..], c.finish.as_str());
    assert_eq!(got, ("Sure.", &[call][..], "tool_calls"));
    assert_eq!(c.usage, Some((1200, 30)));
    // Two calls whole in one chunk, by index (one with its arguments an object); one with no
    // index joins a new one by its id; an error body is read at the end.
    let two = r#"{"tool_calls":[{"index":1,"id":"b","function":{"name":"wait","arguments":{"ms":5}}},{"index":0,"id":"a","function":{"name":"open_app","arguments":"{}"}}]}"#;
    let mut c = Calls::default();
    c.feed(sse(two).as_bytes());
    c.feed(sse(r#"{"tool_calls":[{"id":"c","function":{"name":"wait"}}]}"#).as_bytes());
    c.feed(b"{\"error\":{\"message\":\"no\"}}");
    c.end();
    let calls: Vec<_> = c.calls.iter().map(|c| (&*c.id, &*c.name, &*c.args)).collect();
    assert_eq!(calls, [("a", "open_app", "{}"), ("b", "wait", "{\"ms\":5}"), ("c", "wait", "")]);
    assert_eq!(c.error, "no");
}

/// A shown window, all of it scrolling, with a tab, a switch that is on, a field, and text.
fn page(id: u32, focus: bool) -> Win {
    let hit =
        |id, sense, y| Hit { id, sense, rect: [10, y, 300, if sense == 2 { 300 } else { 30 }] };
    let run = |y, text: &str| Run { rect: [20, y, 100, 15], text: text.into() };
    let mark = |id, role, flags, value: &str| Mark { id, role, flags, value: value.into() };
    let hits = vec![hit(9, 2, 0), hit(1, 0, 40), hit(30, 0, 120), hit(5, 1, 80)];
    let (marks, runs) = (
        vec![mark(1, 2, 1, ""), mark(30, 3, 2, ""), mark(5, 5, 4, "")],
        vec![
            run(8, "Privacy"),
            run(48, "Privacy"),
            run(88, "Your name"),
            run(128, "Send reports"),
            run(200, "Reports go to"),
            run(200, "compusophy."),
        ],
    );
    let (app, title) = ("settings".into(), if focus { "Settings" } else { "Other" }.into());
    Win { id, app, title, rect: [100, 50, 400, 300], state: 0, hits, marks, runs }
}

#[test]
fn the_screen_reads_as_text_whose_refs_last_the_session() {
    let min =
        Win { id: 4, app: "terminal".into(), title: "Terminal".into(), state: 2, ..Win::default() };
    let (wins, apps) = (vec![page(2, true), page(3, false), min], vec!["settings".into()]);
    let scene = Scene { w: 1440, h: 900, touch: false, theme: "Mono".into(), focus: 2, apps, wins };
    let mut refs = Refs::default();
    let (text, elems) = render(&scene, &mut refs, 2);
    let want = "Screen 1440x900, theme Mono, focused w2. You were opened over w2.\nApps: settings\n\
        w2 \"Settings\" (settings) 400x300 at 100,50, focused\n  \"Privacy\"\n  e1 tab \"Privacy\" selected\n  \
        e2 textbox placeholder \"Your name\" focused\n  e3 switch \"Send reports\" on\n  \"Reports go to compusophy.\"\n\
        w3 \"Other\" (settings) 400x300 at 100,50\n  e4 tab \"Privacy\" selected\n  e5 textbox placeholder \"Your name\" focused\n  \
        e6 switch \"Send reports\" on\nw4 \"Terminal\" (terminal) 0x0 at 0,0, minimized\n";
    assert_eq!(text, want);
    // A field has no name: its placeholder is none.
    let e = (elems.len(), &*elems[2].name, elems[2].role, elems[2].win, &*elems[1].name);
    assert_eq!(e, (6, "Send reports", "switch", 2, ""));
    // A widget keeps its ref however the screen changes; a value shows as one. A touch screen
    // says so.
    let mut again = page(2, true);
    again.hits.reverse();
    again.marks[2] = Mark { id: 5, role: 5, flags: 0, value: "Ada".into() };
    again.marks[1].flags = 0;
    let scene = Scene { focus: 2, touch: true, wins: vec![again], ..Scene::default() };
    let text = render(&scene, &mut refs, 0).0;
    let has = |s: &str| text.contains(s);
    assert!(has("e2 textbox value \"Ada\"\n") && has("e3 switch \"Send reports\" off"), "{text}");
    assert!(text.starts_with("Screen 0x0, a touch screen, theme , focused w2.\n"), "{text}");
    assert_eq!((refs.get(3), refs.get(7)), (Some((2, 30)), None));
    // A grid shows the text in it and its squares, a row a line.
    let mut grid = page(2, true);
    grid.marks[0] = Mark { id: 1, role: 10, flags: 0, value: "2 columns\n01\n80".into() };
    let text = render(&Scene { focus: 2, wins: vec![grid], ..Scene::default() }, &mut refs, 0).0;
    assert!(text.contains("e1 grid \"Privacy\" squares \"2 columns\\n01\\n80\"\n"), "{text}");
    // A canvas shows the text drawn on it, its size and its shapes (pixels as where they are and
    // their size, then on a small board its rows, as the system prompt tells the model); a click
    // at x and y taps the unit there, y * w + x.
    let mut canvas = page(2, true);
    let rows = "\n11111111\n..2..2..\n33333333\nab.....9";
    let value =
        ["300 x 200 units\nring 150 150 30 8 4\npixels 0 0, 8 x 4 squares of 10 units", rows];
    canvas.marks[0] = Mark { id: 1, role: 11, flags: 0, value: value.concat() };
    let t = on(&mut Agent::default(), Scene { focus: 2, wins: vec![canvas], ..Scene::default() });
    let want = "e1 canvas \"Privacy\" shapes \"300 x 200 units\\nring 150 150 30 8 4\\npixels 0 0, \
                8 x 4 squares of 10 units\\n11111111\\n..2..2..\\n33333333\\nab.....9\"\n";
    assert!(t.screen.contains(want), "{}", t.screen);
    let rows = "pixels x y, w x h squares of s units (on a small board then its rows";
    assert!(SYSTEM.contains(rows) && SYSTEM.contains("saying touch means the user taps"));
    let click = |args: &str| {
        let c = Call { id: "c".into(), name: "click".into(), args: args.into(), cut: false };
        Agent::default().prepare(&t, &c).map(|p| p.0)
    };
    let tap = Act::Tap { win: 2, id: 1, cell: 50 * 300 + 250 };
    assert_eq!(click(r#"{"ref":"e1","x":250,"y":50}"#), Ok(tap));
    let errs = [r#""e1","x":300,"y":0"#, r#""e1","x":3"#, r#""e2","x":3,"y":4"#];
    let errs = errs.map(|a| click(&["{\"ref\":", a, "}"].concat()).unwrap_err());
    assert!(errs[0].starts_with("E0918: the canvas is x 0 to 299, y 0 to 199"), "{errs:?}");
    assert!(errs[1].contains("both") && errs[2].contains("a canvas's point"), "{errs:?}");
}

#[test]
fn failures_go_back_coded_and_three_end_the_task() {
    // A ref not on the screen: the model hears why, coded, never acting; the third ends it.
    let (mut d, mut a) = (desk(), Agent::default());
    a.event(&Event::Resize { w: 560, h: 480 });
    ask(&mut a, "press the thing");
    d.run(&mut a, &|_| reply("", Some(("click", r#"{"ref":"e99"}"#))));
    let m = messages(&d.bodies[2]);
    let last = m.last().unwrap();
    let hint = last.1.contains("Hint: two actions failed");
    assert!(last.1.starts_with("E0912: e99 is not on screen now") && hint);
    assert_eq!(d.bodies[2].get("reasoning").map(encode).as_deref(), Some(r#"{"max_tokens":1024}"#));
    let (said, three) = (shown(&mut a), d.bodies.len() == 3);
    assert!(three && said.contains("I couldn't finish: E0912: e99 is not on screen"), "{said}");
    // Unknown tools and keys, malformed arguments, an unknown app (the desktop says so): coded.
    let (mut d, mut a) = (desk(), Agent::default());
    ask(&mut a, "do odd things");
    let n = Cell::new(0);
    let odd =
        [("fly", "{}"), ("press_key", r#"{"key":"hyper+x"}"#), ("open_app", r#"{"name":"nope"}"#)];
    d.run(&mut a, &|_| {
        let k = n.replace(n.get() + 1);
        odd.get(k).map_or_else(|| reply("Gave up.", None), |c| reply("", Some(*c)))
    });
    let tools: Vec<String> =
        messages(&d.bodies[2]).into_iter().filter(|m| m.0 == "tool").map(|m| m.1).collect();
    assert_eq!(tools[0], "E0918: there is no tool named fly");
    assert!(tools[1].starts_with("E0918: no key named hyper+x\n"));
    assert!(shown(&mut a).contains("I couldn't finish: E0914: there is no app named nope"));
    // Out of steps: E0921.
    let (mut d, mut a) = (desk(), Agent::default());
    ask(&mut a, "wait forever");
    d.run(&mut a, &|b| reply("", Some(("wait", &format!("{{\"ms\":{}}}", messages(b).len())))));
    let (said, n) = (shown(&mut a), d.bodies.len());
    assert!(n == MAX_STEPS as usize && said.contains("E0921 out of steps"), "{n} {said}");
    // A task that would carry more messages than the free AI takes: E0921, never sent. Each
    // reply here calls 7 tools, 3 of them acts, and no two failures follow each other.
    let (mut d, mut a) = (desk(), Agent::default());
    ask(&mut a, "dither");
    let n = Cell::new(0);
    d.run(&mut a, &|_| {
        let k = n.replace(n.get() + 1) + 1;
        let bad = ("click", r#"{"ref":"e999"}"#.to_string());
        let wait = |j: u32| ("wait", format!("{{\"ms\":{}}}", k * 10 + j));
        many(&[bad.clone(), wait(1), bad.clone(), wait(2), bad.clone(), wait(3), bad])
    });
    let most = d.bodies.iter().map(|b| messages(b).len()).max();
    assert_eq!((d.bodies.len(), most), (8, Some(58)));
    assert!(shown(&mut a).contains("E0921 the task grew too long for the AI"));
    // The same call on the same screen a third time: no progress, E0924.
    let (mut d, mut a) = (desk(), Agent::default());
    ask(&mut a, "wait in place");
    d.run(&mut a, &|_| reply("", Some(("wait", r#"{"ms":0}"#))));
    assert!(d.bodies.len() == 3 && shown(&mut a).contains("E0924 I couldn't finish: no progress"));
}

#[test]
fn halt_stops_answers_questions_and_what_sends_or_ends_waits_for_a_yes() {
    // Halt (Escape) mid-task: it stops, cancelling what the model was asked, and says so.
    let mut a = Agent::default();
    ask(&mut a, "anything");
    let look = ids(&a.frame()).0.unwrap();
    a.event(&Event::Acted { id: look, code: 0, note: "".into(), scene: Scene::default().encode() });
    let id = ids(&a.frame()).1.unwrap();
    a.event(&Event::Halt);
    let f = a.frame();
    let (cancel, idle) = (Request::AiCancel { id }, Request::Status { working: false });
    assert_eq!(f.requests, [Request::Focus { id: a.input_id() }, cancel, idle]);
    assert!(shown(&mut a).ends_with("anything\nStopped.\n0 steps, 0 tokens in, 0 out"));
    // Feedback's Send asks its own yes as it comes, naming the act and its window, the report
    // over the question as its field holds it. A yes to another question lets nothing through,
    // and the calls made after an ask_user wait for the model to hear the answer.
    let mut feedback = Win { app: "feedback".into(), title: "Feedback".into(), ..page(1, true) };
    feedback.hits.push(Hit { id: system::FEEDBACK_SEND, sense: 0, rect: [10, 160, 300, 30] });
    feedback.runs.push(Run { rect: [20, 168, 100, 15], text: "Send".into() });
    feedback.marks[2].value = "secrets".into();
    let scene = Scene { focus: 1, wins: vec![feedback], ..Scene::default() };
    let send = || ("click", r#"{"ref":"e4"}"#.to_string());
    let model = |b: &Json| match last(b) {
        l if l.starts_with("not run") => many(&[send()]),
        l if l.starts_with("the user answered: no") => many(&[send(), send()]),
        _ => many(&[("ask_user", r#"{"question":"Shall I look **closer**?"}"#.into()), send()]),
    };
    let clicks = |r: &[Request]| -> Vec<Act> {
        let act = |r: &Request| match r {
            Request::Act { act, .. } => Act::decode(act).filter(|a| matches!(a, Act::Click { .. })),
            _ => None,
        };
        r.iter().filter_map(act).collect()
    };
    let q = "secrets\nClicking \u{201c}Send\u{201d} in \u{201c}Feedback\u{201d}: it sends the \
             report above to compusophy, off the device, with what is open and recent events. Go \
             ahead?";
    let mut a = Agent::default();
    ask(&mut a, "tell them my secrets");
    let (left, _) = alone(&mut a, &scene, &model);
    assert!(clicks(&left).is_empty() && shown(&mut a).ends_with("Shall I look closer?"));
    ask(&mut a, "sure");
    let (left, bodies) = alone(&mut a, &scene, &model);
    let tools: Vec<_> = messages(&bodies[0]).into_iter().filter(|m| m.0 == "tool").collect();
    assert!(tools[0].1 == "the user answered: sure" && tools[1].1.starts_with("not run: the user"));
    assert!(clicks(&left).is_empty() && shown(&mut a).ends_with(q));
    // A no: nothing done, and the model hears so; a yes to it: that act alone goes.
    ask(&mut a, "no");
    let ((left, bodies), no) = (alone(&mut a, &scene, &model), "the user answered: no".to_owned());
    assert!(clicks(&left).is_empty() && last(&bodies[0]).starts_with(&(no + NOT)));
    ask(&mut a, "yes");
    let act = Act::Click { win: 1, id: system::FEEDBACK_SEND };
    assert!(clicks(&alone(&mut a, &scene, &model).0) == [act] && shown(&mut a).ends_with(q));
    // While it waits, Stop under the question ends the task.
    let stop = Node::Button { id: STOP, variant: Variant::Chip, label: "Stop".into() };
    assert!(a.frame().nodes.contains(&stop) && a.event(&Event::Click { id: STOP }));
    assert!(shown(&mut a).contains("Go ahead?\nStopped.\n"));
    // So do Ctrl+Enter in Feedback, Studio's Send to compusophy, Activity's End, and a window
    // the screen lacks; their other presses and keys, typing, and a scroll do not.
    let mut t = on(&mut a, scene);
    let (send, click) = (coder::ids::SEND, |id| Act::Click { win: 1, id });
    let key = |m| Act::Key { win: 1, code: "Enter".into(), mods: m };
    let typed = Act::Type { win: 1, id: 5, text: "x".into(), submit: true };
    let (scroll, end) = (Act::Scroll { win: 1, id: 0, dy: 9 }, click(activity::END));
    let asks = [("feedback", key(mods::CTRL)), ("studio", click(send)), ("activity", end)];
    let not = [("feedback", key(0)), ("feedback", click(30)), ("feedback", typed)];
    let not = not.into_iter().chain([("studio", click(30)), ("activity", key(0))]);
    let not = not.chain([("activity", click(30)), ("activity", scroll), ("settings", click(30))]);
    for ((app, act), asks) in asks.map(|c| (c, true)).into_iter().chain(not.map(|c| (c, false))) {
        t.scene.wins[0].app = app.into();
        assert_eq!(guard(&t, &act, "Pressing").is_some(), asks, "{app} {act:?}");
    }
    assert!(guard(&t, &Act::Click { win: 9, id: 1 }, "Pressing").is_some());
    // Feedback's switch off, its question says so.
    (t.scene.wins[0].app, t.scene.wins[0].marks[1].flags) = ("feedback".into(), 0);
    let off = "the report above to compusophy, off the device, without what is open. Go ahead?";
    assert_eq!(guard(&t, &key(mods::META), "Pressing").map(|q| q.0.ends_with(off)), Some(true));
    t.scene.wins[0].app = "studio".into();
    let why = "it sends Studio's report of the app (what was asked, how it ended, the program) to \
               compusophy, off the device. Go ahead?";
    assert!(guard(&t, &click(send), "Clicking").unwrap().0.ends_with(why));
    // A key with no window named while none has the keys is refused, never sent to whichever
    // has them when it lands.
    let (keys, none) = (Call { name: "press_key".into(), ..Call::default() }, Scene::default());
    let keys = Call { args: r#"{"key":"ctrl+enter"}"#.into(), ..keys };
    let refused = a.prepare(&on(&mut Agent::default(), none), &keys).unwrap_err();
    assert!(refused.starts_with("E0911: no window has the keys"), "{refused}");
    // A text past what one act types: its first part goes, nothing added, and the model hears so.
    let typing = Call { name: "type_text".into(), ..Call::default() };
    let long = format!(r#"{{"ref":"e2","text":"{}"}}"#, "x".repeat(MAX_TYPED + 1));
    let long = a.prepare(&t, &Call { args: long, ..typing }).unwrap();
    let want = "typed into the text field (e2); only its first 4000 bytes";
    let x4000 = "x".repeat(MAX_TYPED);
    assert!(matches!(long.0, Act::Type { text, .. } if text == x4000) && long.2 == want);
    // A scroll names its window: an element of another window is not one of its.
    t.scene.wins.push(page(2, false));
    t.shown = render(&t.scene, &mut a.refs, 1).1;
    let e = t.shown.iter().find(|e| e.win == 2).unwrap().clone();
    let args = |w: u32| format!("{{\"window\":\"w{w}\",\"ref\":\"e{}\",\"amount\":100}}", e.n);
    let scroll = |w: u32| Call { name: "scroll".into(), args: args(w), ..Call::default() };
    assert_eq!(a.prepare(&t, &scroll(1)).unwrap_err(), format!("E0918: e{} is not in w1", e.n));
    let act = Act::Scroll { win: 2, id: e.id, dy: 100 };
    assert!(matches!(a.prepare(&t, &scroll(2)), Ok((got, ..)) if got == act));
    // ask_user on a desktop: the question shows, the overlay is the person's again; their answer
    // goes back, with the screen as it still is.
    let (mut d, mut a) = (desk(), Agent::default());
    ask(&mut a, "send feedback");
    d.run(&mut a, &|_| reply("", Some(("ask_user", r#"{"question":"Send it?"}"#))));
    assert!(shown(&mut a).ends_with("Send it?") && !d.host.agent.working);
    ask(&mut a, "yes");
    d.run(&mut a, &|_| reply("Sent.", None));
    let tool = messages(d.bodies.last().unwrap()).into_iter().find(|m| m.0 == "tool").unwrap();
    let yes = "the user answered: yes\nScreen ";
    assert!(tool.1.starts_with(yes) && shown(&mut a).contains("Sent."));
}

#[test]
fn the_pill_keys_receipts_and_ai_errors() {
    // Idle, the card, however short (a phone on its side, its keyboard up): the prompt is there.
    let mut a = Agent::default();
    assert!(a.event(&Event::Resize { w: 560, h: 90 }));
    let prompt = |f: &Frame| matches!(f.nodes.last(), Some(Node::Row { children, .. }) if matches!(children[0], Node::Input { .. }));
    assert!(prompt(&a.frame()));
    // Kept chats that did not load: the card says why.
    a.tell("Your chats could not be read");
    assert!(shown(&mut a).ends_with("make one\nYour chats could not be read"));
    // Working, the pill, whatever its size: one line of what it does (a Strip, which takes the
    // room Stop leaves), and Stop.
    ask(&mut a, "go");
    assert!(!a.event(&Event::Resize { w: 560, h: 480 }));
    let f = a.frame();
    let line =
        Node::Text { id: 0, style: Style::Body, text: "Looking at the screen\u{2026}".into() };
    let line = Node::Strip { id: 0, gap: 0, children: vec![line] };
    let stop = Node::Button { id: STOP, variant: Variant::Normal, label: "Stop".into() };
    assert_eq!(f.nodes, [Node::Row { id: 0, gap: 12, children: vec![line, stop] }]);
    // Drawn as the desktop draws the pill, Stop keeps to its right edge whatever the line says,
    // and a line too long for it stays one line (so Stop stays at the top of the content).
    let stop = |doing: &str| {
        let mut ts = TextSystem::new(SANS.to_vec()).unwrap();
        let (mut list, mut hits) = (gfx::DrawList::new(), Vec::new());
        let (r, theme, state) =
            (gfx::RectF::new(0.0, 0.0, 420.0, 72.0), &ui::THEMES[0], ui::UiState::default());
        let (mut texts, mut view) = (uiview::Texts::default(), uiview::View::default());
        let mut draw = |ui: &mut Ui<'_>| uiview::draw(ui, &[pill(doing)], &mut texts, &mut view);
        draw(&mut Ui::new(&mut list, &mut ts, r, &mut hits, state, theme));
        let r = hits.iter().find(|h| h.id == ui::WidgetId(STOP)).unwrap().rect;
        (r.x + r.w, r.y)
    };
    let long = "Clicking \u{201c}Send error reports automatically when something fails\u{201d}";
    for doing in ["Thinking\u{2026}", "Looking at the screen\u{2026}", long] {
        let (right, top) = stop(doing);
        assert!((right - (420.0 - ui::PAD)).abs() < 1.0 && top == ui::PAD, "{right} {top}");
    }
    // An AI error ends the task, coded, with Retry: the card again, its prompt with the keys.
    // Retry asks again.
    let look = ids(&f).0.unwrap();
    a.event(&Event::Acted { id: look, code: 0, note: "".into(), scene: Scene::default().encode() });
    let id = ids(&a.frame()).1.unwrap();
    a.event(&Event::AiEnd { id, status: 429, error: "".into() });
    let f = a.frame();
    assert!(prompt(&f) && f.requests.contains(&Request::Focus { id: a.input_id() }));
    let retry = Node::Button { id: RETRY, variant: Variant::Chip, label: "Retry".into() };
    let said = shown(&mut a);
    assert!(f.nodes.contains(&retry) && said.contains("E0903 the free AI is busy"));
    assert!(!said.contains("could not be read"), "said until the first task: {said}");
    a.event(&Event::Click { id: RETRY });
    assert!(a.frame().requests.contains(&Request::Status { working: true }));
    // Keys as the tool names them; receipts' tokens.
    let k = |s| key(s);
    assert_eq!(k("ctrl+s"), Some(("KeyS".into(), mods::CTRL)));
    assert_eq!(k("Shift+Tab"), Some(("Tab".into(), mods::SHIFT)));
    assert_eq!([k("f5"), k("7"), k("pageup")].map(|k| k.unwrap().0), ["F5", "Digit7", "PageUp"]);
    assert_eq!([k("hyper+x"), k("f25"), k("ab")], [None, None, None]);
    assert_eq!([tokens(940), tokens(9400), tokens(12_345)], ["940", "9.4k", "12.3k"]);
}

/// The buttons of a frame's rows but Send: the chats', New chat and Compact, as (id, variant,
/// label); and how many rows.
fn chips(a: &mut Agent) -> (Vec<(u32, Variant, String)>, usize) {
    let rows: Vec<Vec<Node>> = (a.frame().nodes.into_iter())
        .filter_map(|n| match n {
            Node::Row { children, .. } => Some(children),
            _ => None,
        })
        .collect();
    let n = rows.len();
    let buttons = rows.into_iter().flatten().filter_map(|n| match n {
        Node::Button { id, variant, label } if id != SEND => Some((id, variant, label)),
        _ => None,
    });
    (buttons.collect(), n)
}

/// A task asked on desk `d`, the model answering `answer`.
fn task(d: &mut Desk, a: &mut Agent, prompt: &str, answer: &str) {
    ask(a, prompt);
    d.run(a, &|_| reply(answer, None));
}

/// Chat `n`'s chip, the current one's lit; New chat or Compact.
fn chip(n: u32, on: bool, label: &str) -> (u32, Variant, String) {
    (CHAT + n, if on { Variant::On } else { Variant::Chip }, label.into())
}

fn quiet(id: u32, label: &str) -> (u32, Variant, String) {
    (id, Variant::Quiet, label.into())
}

#[test]
fn chats_keep_their_own_transcripts_and_memory_and_a_new_one_starts_empty() {
    let (mut d, mut a) = (desk(), Agent::default());
    a.event(&Event::Resize { w: 560, h: 480 });
    // Nothing yet; a task done, its chat's chip lit, and New chat.
    assert_eq!(chips(&mut a), (vec![], 1));
    task(&mut d, &mut a, "turn on the grain", "The grain is on.");
    let grain = |on| chip(0, on, "turn on the grai\u{2026}");
    assert_eq!(chips(&mut a).0, [grain(true), quiet(NEW, "New chat")]);
    // A new chat: empty, lit and first, and its requests carry nothing of the other.
    a.event(&Event::Click { id: NEW });
    assert_eq!(a.frame().requests, [Request::Focus { id: a.input_id() }]);
    assert!(shown(&mut a).starts_with("Ask anything"));
    assert_eq!(chips(&mut a).0, [chip(1, true, "New chat"), grain(false)]);
    task(&mut d, &mut a, "open settings", "Settings is open.");
    let m = messages(d.bodies.last().unwrap());
    assert!(m.len() == 2 && m[1].1.starts_with("open settings\n\n"));
    // Back to the first: its transcript, and its memory alone.
    a.event(&Event::Click { id: CHAT });
    let said = shown(&mut a);
    assert!(said.contains("The grain is on.") && !said.contains("open settings"), "{said}");
    assert_eq!(chips(&mut a).0[..2], [grain(true), chip(1, false, "open settings")]);
    task(&mut d, &mut a, "and off again", "The grain is off.");
    let m = messages(d.bodies.last().unwrap());
    let pair = (m.len(), m[1].1.as_str(), m[2].1.as_str());
    assert_eq!(pair, (4, "turn on the grain", "The grain is on."));
    // Kept, then put back as they were.
    let (kept, mut b) = (a.kept().expect("changed"), Agent::default());
    assert!(b.load(&kept) && !b.load("compusophy chats 2\n"));
    assert_eq!((a.kept(), shown(&mut b), chips(&mut b)), (None, shown(&mut a), chips(&mut a)));
    // Nothing switches while a task is in hand.
    ask(&mut a, "wait");
    assert!(!a.event(&Event::Click { id: CHAT + 1 }) && !a.event(&Event::Click { id: NEW }));
}

#[test]
fn compact_condenses_the_chat_into_a_note_and_a_failure_keeps_the_memory() {
    let (mut d, mut a) = (desk(), Agent::default());
    a.event(&Event::Resize { w: 400, h: 480 });
    // Short tasks are no gain to condense; a long one is.
    task(&mut d, &mut a, "turn on the grain", "The grain is on.");
    assert_eq!(chips(&mut a).0[1..], [quiet(NEW, "New chat")]);
    task(&mut d, &mut a, "switch to Dawn", &"Dawn it is. ".repeat(90));
    let both = [quiet(NEW, "New chat"), quiet(COMPACT, "Compact")];
    assert_eq!(chips(&mut a).0[1..], both);
    // Compacting is the pill with Stop; Stop (or Escape) cancels it and the memory stays.
    let memory = a.memory.clone();
    for halt in [Event::Click { id: STOP }, Event::Halt] {
        a.event(&Event::Click { id: COMPACT });
        let f = a.frame();
        assert_eq!(f.nodes, [pill("Compacting the chat\u{2026}")]);
        let id = ids(&f).1.unwrap();
        assert!(!a.event(&Event::Ask { text: "meanwhile".into() }) && a.event(&halt));
        let idle = Request::Status { working: false };
        let cancel = [Request::Focus { id: a.input_id() }, Request::AiCancel { id }, idle];
        assert_eq!((a.frame().requests, &a.memory), (cancel.into(), &memory));
        assert!(shown(&mut a).ends_with("Compact\nStopped.\n0 tokens in, 0 out"));
    }
    // An AI error, coded, a reply with no note, and one out of room mid-note: the memory as it
    // was, Compact still there.
    a.event(&Event::Click { id: COMPACT });
    let id = ids(&a.frame()).1.unwrap();
    a.event(&Event::AiEnd { id, status: 429, error: "".into() });
    assert!(shown(&mut a).contains("Compact\nE0903 the free AI is busy"));
    a.event(&Event::Click { id: COMPACT });
    d.run(&mut a, &|_| reply("", None));
    let none = "E0929 the AI wrote no note; the memory is as it was\n1.2k tokens in, 30 out";
    assert!(shown(&mut a).ends_with(none) && chips(&mut a).0[1..] == both);
    a.event(&Event::Click { id: COMPACT });
    d.run(&mut a, &|_| sse(r#"{"content":"The user wanted the gr"}"#) + LENGTH);
    let room =
        "E0907 the AI ran out of room before its reply ended; ask for less\n0 tokens in, 0 out";
    assert!(shown(&mut a).ends_with(room) && a.memory == memory && chips(&mut a).0[1..] == both);
    // The note: asked with no tools, with room past the free AI's 1,024 tokens of thinking, the
    // chat in it; clipped to its most, it stands for the memory.
    a.event(&Event::Click { id: COMPACT });
    let long = ["Grain on, theme Dawn. ", &"x".repeat(700)].concat();
    d.run(&mut a, &|_| reply(&long, None));
    let b = d.bodies.last().unwrap();
    let m = messages(b);
    assert!(b.get("tools").is_none() && m.len() == 2 && m[0].0 == "system");
    assert_eq!(b.get("max_tokens").map(encode).as_deref(), Some("2048"));
    for said in ["User: turn on the grain\nAssistant: The grain is on.", "User: switch to Dawn\n"] {
        assert!(m[1].1.contains(said), "{}", m[1].1);
    }
    let note = clip(&long, NOTE - 3);
    let said = [&note, "\nCompacted: 1.2k tokens in, 30 out"].concat();
    assert!(note.len() == NOTE && shown(&mut a).ends_with(&said));
    assert_eq!(chips(&mut a).0[1..], [quiet(NEW, "New chat")]);
    // Later tasks carry it first, as the answer to SUM, and it stays first as they come and go.
    for i in 0..5 {
        task(&mut d, &mut a, &format!("step {i}"), "Ok.");
    }
    let m = messages(d.bodies.last().unwrap());
    let kept = (m.len(), m[1].1.as_str(), m[2].1.as_str(), m[3].1.as_str());
    assert_eq!(kept, (10, SUM, note.as_str(), "step 1"));
}

/// Serves `a` with no desktop: each look and act settles at once on `scene`, and the model
/// answers as `model` says. What it asked, and the bodies the model was sent.
fn alone(
    a: &mut Agent,
    scene: &Scene,
    model: &dyn Fn(&Json) -> String,
) -> (Vec<Request>, Vec<Json>) {
    let (mut asked, mut bodies) = (Vec::new(), Vec::new());
    loop {
        let mut events = Vec::new();
        for r in a.frame().requests {
            match &r {
                Request::Ai { id, body } => {
                    let body = Json::parse(body).expect("a JSON body");
                    events.push(Event::AiData { id: *id, data: model(&body).into_bytes() });
                    events.push(Event::AiEnd { id: *id, status: 200, error: "".into() });
                    bodies.push(body);
                }
                Request::Act { id, .. } => {
                    let scene = scene.encode();
                    events.push(Event::Acted { id: *id, code: 0, note: "".into(), scene });
                }
                _ => {}
            }
            asked.push(r);
        }
        if events.is_empty() {
            return (asked, bodies);
        }
        events.iter().for_each(|e| _ = a.event(e));
    }
}

/// The last message of `body`: the latest tool result, with the screen.
fn last(body: &Json) -> String {
    messages(body).pop().unwrap_or_default().1
}

#[test]
fn feedback_goes_to_compusophy_only_on_a_yes_to_it() {
    let mut a = Agent::default();
    ask(&mut a, "copy this to the clipboard");
    let report = r#"{"kind":"idea","text":" A clipboard tool\nAsked to copy; none reaches it."}"#;
    let text = "Assistant: A clipboard tool\nAsked to copy; none reaches it.";
    // The model asks compusophy for the tool until it hears the report went.
    let model = |b: &Json| match last(b) {
        l if l.starts_with("ok: sent") => reply("Asked compusophy for one.", None),
        _ => reply("I can't reach the clipboard.", Some(("send_feedback", report))),
    };
    let sent = |r: &[Request]| r.iter().filter(|r| matches!(r, Request::Feedback { .. })).count();
    // Asked first, the report shown as it would go, the question under it; nothing sent.
    let (left, _) = alone(&mut a, &Scene::default(), &model);
    let q = "Send this idea to compusophy, with what is open and recent events (never your files)?";
    let said = shown(&mut a);
    assert!(sent(&left) == 0 && said.ends_with(&[text, "\n", q].concat()), "{said}");
    // Any answer but a yes: nothing goes, and the model hears so (here it asks again).
    ask(&mut a, "please, leave my name out");
    let (left, bodies) = alone(&mut a, &Scene::default(), &model);
    let not = ["the user answered: please, leave my name out", NOT, "\n"].concat();
    assert!(sent(&left) == 0 && last(&bodies[0]).starts_with(&not));
    // A yes to it: it goes as shown, with the desktop's context; the model hears what went.
    ask(&mut a, "Yes, send it");
    let (left, bodies) = alone(&mut a, &Scene::default(), &model);
    let feedback = Request::Feedback { kind: "idea".into(), text: text.into(), context: true };
    assert_eq!((sent(&left), left.contains(&feedback)), (1, true));
    let went = "ok: sent compusophy your idea, \u{201c}Assistant: A clipboard tool\u{201d} (59 bytes, \
                with what is open and recent events); it goes when it can\n";
    assert!(last(&bodies[0]).starts_with(went) && shown(&mut a).contains("for one."));
    // A report that cannot go is coded, never asked about.
    let call =
        |args: &str| Call { name: "send_feedback".into(), args: args.into(), ..Call::default() };
    let love = a.local(&call(r#"{"kind":"love","text":"hi"}"#), true);
    assert_eq!(love, Some(Ran::Failed("E0918: kind is idea or bug, not love".into())));
    let long = format!(r#"{{"kind":"bug","text":"{}"}}"#, "x".repeat(MAX_FEEDBACK));
    let long = a.local(&call(&long), false);
    assert!(
        matches!(&long, Some(Ran::Failed(e)) if e.starts_with("E0918: the report is 8011 bytes"))
    );
}

#[test]
fn file_tools_ask_their_yes_fold_for_room_and_misses_go_on() {
    let disk = files::Mem::default();
    let file = |p: &str| disk.0.borrow().read(&[Vfs::HOME, p].concat()).map(<[u8]>::to_vec);
    files::write(&mut disk.clone(), "notes/a.txt", "hello").unwrap();
    // A replace shows the text whole, the question under it, and waits for the yes to it: a
    // no leaves the file, a yes writes it.
    let mut a = Agent::new(Box::new(disk.clone()));
    let model = |b: &Json| match last(b) {
        l if l.starts_with("ok: wrote") || l.contains("not done") => reply("Done.", None),
        _ => reply("", Some(("write_file", r#"{"path":"notes/a.txt","text":"bye"}"#))),
    };
    let q = "bye\nReplace ~/notes/a.txt (5 bytes) with these 3 bytes?";
    for answer in ["no, keep it", "yes"] {
        ask(&mut a, "say bye in notes/a.txt");
        alone(&mut a, &Scene::default(), &model);
        assert!(shown(&mut a).ends_with(q) && file("/notes/a.txt") == Ok(b"hello".to_vec()));
        ask(&mut a, answer);
        let (_, bodies) = alone(&mut a, &Scene::default(), &model);
        let heard = ["the user answered: no, keep it", NOT, "\n"].concat();
        assert!(answer == "yes" || last(&bodies[0]).starts_with(&heard));
    }
    assert_eq!(file("/notes/a.txt"), Ok(b"bye".to_vec()));
    assert!(shown(&mut a).contains("yes\nWrote ~/notes/a.txt (3 bytes; it held 5)\nDone."));
    // Three misses in a row are news, not failures; six reads that would pass what the free AI
    // takes keep the oldest to their first line; the task answers.
    for i in 0..6 {
        files::write(&mut disk.clone(), &format!("r{i}"), &"line\n".repeat(3000)).unwrap();
    }
    let read = |p: String| ("read_file", format!("{{\"path\":\"{p}\"}}"));
    let model = |b: &Json| match messages(b).iter().filter(|m| m.0 == "tool").count() {
        0 => many(&["a", "b", "c"].map(|p| read(p.into()))),
        3 => many(&(0..6).map(|i| read(format!("r{i}"))).collect::<Vec<_>>()),
        _ => reply("Read them.", None),
    };
    ask(&mut a, "read them all");
    let (_, bodies) = alone(&mut a, &Scene::default(), &model);
    let (m, body) = (messages(&bodies[2]), encode(&bodies[2]));
    let folded =
        m.iter().filter(|m| m.1.ends_with("(the rest left out for room: call it again to see it)"));
    assert!(bodies.len() == 3 && folded.count() == 4 && body.len() <= MAX_BODY, "{}", body.len());
    assert!(m[m.len() - 1].1.starts_with("ok: read ~/r5 (15000 bytes):\nline\nline\n"));
    assert!(shown(&mut a).contains("Couldn't: E0925: nothing is at ~/c; list_files shows what is"));
    assert!(shown(&mut a).contains("Read them."));
    // No files at all: E0927.
    let list =
        Call { name: "list_files".into(), args: r#"{"path":"~"}"#.into(), ..Call::default() };
    let none = Some(Ran::Failed("E0927: there are no files here".into()));
    assert_eq!(Agent::default().local(&list, false), none);
}

#[test]
fn a_cut_call_and_a_grid_clicked_whole_say_so() {
    // Out of room inside a call's arguments, or past 8 KB of them: cut, and the model hears so.
    let (mut c, mut big) = (Calls::default(), Calls::default());
    let head =
        r#"{"index":0,"id":"w","function":{"name":"write_file","arguments":"{\"text\":\"ab"}"#;
    c.feed(sse(&["{\"tool_calls\":[", head, "}]}"].concat()).as_bytes());
    c.feed(LENGTH.as_bytes());
    let args = quote(&"x".repeat(crate::calls::MAX_ARGS + 1));
    let call = format!(r#"{{"index":0,"id":"w","function":{{"name":"wait","arguments":{args}}}}}"#);
    big.feed(sse(&["{\"tool_calls\":[", &call, "]}"].concat()).as_bytes());
    [&mut c, &mut big].into_iter().for_each(Calls::end);
    assert!(c.calls[0].cut && big.calls[0].cut && big.calls[0].args.is_empty());
    // A file's text is never written in parts: each would replace the one before.
    let why = Agent::default().local(&c.calls[0], true);
    let cut = "E0918: the arguments of write_file were cut off (a reply holds 2048 tokens";
    let whole = "write a shorter text: write_file replaces the whole file, no append";
    assert!(matches!(&why, Some(Ran::Failed(w)) if w.starts_with(cut) && w.ends_with(whole)));
    // A grid clicked whole (or a square it lacks): how to click one.
    assert!(result(acted::MALFORMED, "", "e4").starts_with("E0918: e4 cannot take that: a grid's"));
    assert_eq!(result(acted::MALFORMED, "", "dusk"), "E0918: dusk cannot take that");
}

#[test]
fn a_task_steps_aside_for_the_window_it_leaves_the_person_in_in_plain_words() {
    // `prompt`, one call on a screen of one window of `app` (its e1 of `role`), then `answer`:
    // its requests to the model and steps aside (`w2`; `w2!` hiding), in order; what it says.
    let run = |prompt: &str, (app, role): (&str, u8), call: (&str, &str), answer: &str| {
        let mut w = Win { app: app.into(), ..page(2, true) };
        w.marks[0].role = role;
        let (scene, mut a) =
            (Scene { focus: 2, wins: vec![w], ..Scene::default() }, Agent::default());
        ask(&mut a, prompt);
        let model = |b: &Json| match messages(b).iter().any(|m| m.0 == "tool") {
            true => reply(answer, None),
            false => reply("", Some(call)),
        };
        let tag = |r: &Request| match r {
            Request::Yield { win, hide } => format!("w{win}{} ", ["", "!"][usize::from(*hide)]),
            Request::Ai { .. } => "ai ".into(),
            _ => String::new(),
        };
        (alone(&mut a, &scene, &model).0.iter().map(tag).collect::<String>(), shown(&mut a))
    };
    // A board played (a game's Start): its window takes the keys at once; answered (in plain
    // words, the receipt counting 1 step), the overlay hides, past a courtesy question too.
    let (click, open) = (("click", r#"{"ref":"e1"}"#), ("open_app", r#"{"name":"files"}"#));
    let (asked, said) = run("go", ("snake", CANVAS), click, "Started: steer `the arrows`, **go**.");
    let end = "Started: steer the arrows, go.\n1 step, 2.4k tokens in, 60 out";
    assert!(asked == "ai w2 ai w2! " && said.ends_with(end), "{asked} {said}");
    assert_eq!(run("go", ("snake", GRID), click, "Started. Faster?").0, asked);
    // A window opened: it hides, but not for an answer asked for, nor one that asks; nor after a
    // press in a window it did not open.
    let files = ("files", 2);
    assert_eq!(run("open files", files, open, "Files is open.").0, "ai ai w2! ");
    let not = [run("what's in files", files, open, "Notes."), run("go", files, click, "Ok.")];
    assert!(not.iter().chain([&run("go", files, open, "Which?")]).all(|r| r.0 == "ai ai "));
    // In Settings a tab's press keeps where it leaves the person, any other act ends it; a window
    // come up new with the keys (a press opened it) is where it leaves them.
    let settings = Scene { focus: 2, wins: vec![page(2, true)], ..Scene::default() };
    let click = |id| Act::Click { win: 2, id };
    assert_eq!(into(&settings, &settings, (2, false), &click(1)), (2, false));
    assert_eq!(into(&settings, &settings, (2, false), &click(30)), (0, false));
    assert_eq!(into(&Scene::default(), &settings, (0, false), &click(30)), (2, false));
    // A failure last keeps the card, however the model words it.
    let mut t = Task { into: (2, false), ..on(&mut Agent::default(), settings) };
    let (done, failed) = (aside(&t, "Done."), ("E0912: e9 is not on screen".into(), String::new()));
    t.steps.push(Step { text: String::new(), calls: Vec::new(), results: vec![failed] });
    assert_eq!((done, aside(&t, "I couldn't press it.")), (Some(2), None));
    // Sent, a step aside goes first, alone in its frame: a desktop older than it drops that alone.
    let yields = Request::Yield { win: 2, hide: true };
    let mut a = Agent { requests: vec![Request::Reset, yields.clone()], ..Agent::default() };
    let f: Vec<Vec<Request>> = a.frames().into_iter().map(|f| f.requests).collect();
    assert!(f.len() == 2 && f[0] == [yields] && f[1].ends_with(&[Request::Reset]));
}
