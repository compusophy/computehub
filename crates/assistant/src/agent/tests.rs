use std::cell::RefCell;
use std::rc::Rc;

use host::{Effect, Host, OVERLAY, Registry, Response};
use ui::{App, AppEvent, Cx, TextSystem, Ui};
use uiwire::scene::{Hit, Mark, Run, Win};
use vfs::Vfs;
use wm::{Rect, Wm};

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
        name => apps::open(name),
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
    /// says, in 7-byte chunks; acts and status go to the host as the overlay's.
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
                        events.extend(data.chain([Event::AiEnd {
                            id,
                            status: 200,
                            error: "".into(),
                        }]));
                    }
                    r @ (Request::Act { .. } | Request::Status { .. }) => {
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
        return reply(
            "",
            Some(("window", &format!("{{\"window\":\"{w}\",\"action\":\"close\"}}"))),
        );
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
    for line in
        ["Opening settings", "Clicking \u{201c}Privacy\u{201d}", "Closing \u{201c}Settings\u{201d}"]
    {
        assert!(said.contains(line), "{line}: {said}");
    }
    assert!(said.ends_with("Error reports are off.\n4 steps, 6.0k tokens in, 150 out"), "{said}");
    // The first request: the system prompt, the tools, the task and the screen, read as text.
    let b = &d.bodies[0];
    let get = |k| b.get(k).map(encode).unwrap_or_default();
    assert_eq!(
        [get("max_tokens"), get("temperature"), get("tool_choice")],
        ["2048", "0.2", "\"auto\""]
    );
    assert_eq!(get("tools"), TOOLS);
    let m = messages(b);
    assert_eq!((m.len(), m[0].0.as_str(), &*m[0].1), (2, "system", SYSTEM));
    assert!(m[1].1.starts_with("turn off error reports\n\nScreen 1440x900, theme Mono."));
    assert!(m[1].1.contains("\nApps: settings, terminal\n"));
    // Folded: the first screen left out, older results their first line, the latest the screen.
    let m = messages(&d.bodies[3]);
    assert_eq!(m[1].1, "turn off error reports\n\n(screen omitted)");
    let tools: Vec<&str> = m.iter().filter(|m| m.0 == "tool").map(|m| m.1.as_str()).collect();
    assert_eq!(tools[..2], ["ok: opened settings", "ok: clicked \u{201c}Privacy\u{201d} (e3)"]);
    assert!(
        tools[2].starts_with("ok: clicked \u{201c}Send error reports automatically\u{201d} (e")
    );
    assert!(tools[2].contains("switch \"Send error reports automatically\" off"));
    // The next task remembers this one; the same refs name the same widgets.
    ask(&mut a, "now turn it back on");
    d.run(&mut a, &|_| reply("Done.", None));
    let m = messages(d.bodies.last().unwrap());
    assert_eq!(
        (m[1].1.as_str(), m[2].1.as_str()),
        ("turn off error reports", "Error reports are off.")
    );
}

#[test]
fn tool_calls_are_collected_however_they_are_split() {
    let mut c = Calls::default();
    let body = reply("Sure.", Some(("click", r#"{"ref":"e4"}"#)));
    body.as_bytes().chunks(3).for_each(|p| c.feed(p));
    c.end();
    let call =
        Call { id: "call_click".into(), name: "click".into(), args: r#"{"ref":"e4"}"#.into() };
    assert_eq!(
        (c.text.as_str(), &c.calls[..], c.finish.as_str()),
        ("Sure.", &[call][..], "tool_calls")
    );
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
    let title = if focus { "Settings" } else { "Other" };
    Win {
        id,
        app: "settings".into(),
        title: title.into(),
        rect: [100, 50, 400, 300],
        state: 0,
        hits,
        marks,
        runs,
    }
}

#[test]
fn the_screen_reads_as_text_whose_refs_last_the_session() {
    let min =
        Win { id: 4, app: "terminal".into(), title: "Terminal".into(), state: 2, ..Win::default() };
    let wins = vec![page(2, true), page(3, false), min];
    let scene = Scene {
        w: 1440,
        h: 900,
        theme: "Mono".into(),
        focus: 2,
        apps: vec!["settings".into()],
        wins,
    };
    let mut refs = Refs::default();
    let (text, elems) = render(&scene, &mut refs, 2);
    let want = "Screen 1440x900, theme Mono, focused w2. You were opened over w2.\nApps: settings\n\
        w2 \"Settings\" (settings) 400x300 at 100,50, focused\n  \"Privacy\"\n  e1 tab \"Privacy\" selected\n  \
        e2 textbox placeholder \"Your name\" focused\n  e3 switch \"Send reports\" on\n  \"Reports go to compusophy.\"\n\
        w3 \"Other\" (settings) 400x300 at 100,50\n  e4 tab \"Privacy\" selected\n  e5 textbox placeholder \"Your name\" focused\n  \
        e6 switch \"Send reports\" on\nw4 \"Terminal\" (terminal) 0x0 at 0,0, minimized\n";
    assert_eq!(text, want);
    assert_eq!(
        (elems.len(), &*elems[2].name, elems[2].role, elems[2].win),
        (6, "Send reports", "switch", 2)
    );
    // A widget keeps its ref however the screen changes; a value shows as one.
    let mut again = page(2, true);
    again.hits.reverse();
    again.marks[2] = Mark { id: 5, role: 5, flags: 0, value: "Ada".into() };
    again.marks[1].flags = 0;
    let scene = Scene { focus: 2, wins: vec![again], ..Scene::default() };
    let text = render(&scene, &mut refs, 0).0;
    assert!(
        text.contains("e2 textbox value \"Ada\"\n")
            && text.contains("e3 switch \"Send reports\" off"),
        "{text}"
    );
    assert_eq!((refs.get(3), refs.get(7)), (Some((2, 30)), None));
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
    assert!(
        last.1.starts_with("E0912: e99 is not on screen now")
            && last.1.contains("Hint: two actions failed")
    );
    assert_eq!(d.bodies[2].get("reasoning").map(encode).as_deref(), Some(r#"{"effort":"low"}"#));
    let said = shown(&mut a);
    assert!(
        d.bodies.len() == 3 && said.contains("I couldn't finish: E0912: e99 is not on screen"),
        "{said}"
    );
    // Unknown tools and keys, malformed arguments, an unknown app (the desktop says so): coded.
    let (mut d, mut a) = (desk(), Agent::default());
    ask(&mut a, "do odd things");
    let n = Rc::new(RefCell::new(0));
    let odd =
        [("fly", "{}"), ("press_key", r#"{"key":"hyper+x"}"#), ("open_app", r#"{"name":"nope"}"#)];
    d.run(&mut a, &|_| {
        *n.borrow_mut() += 1;
        let k = *n.borrow() - 1;
        odd.get(k).map_or_else(|| reply("Gave up.", None), |c| reply("", Some(*c)))
    });
    let tools: Vec<String> =
        messages(&d.bodies[2]).into_iter().filter(|m| m.0 == "tool").map(|m| m.1).collect();
    assert!(
        tools[0] == "E0918: there is no tool named fly"
            && tools[1].starts_with("E0918: no key named hyper+x\n")
    );
    assert!(shown(&mut a).contains("I couldn't finish: E0914: there is no app named nope"));
    // Out of steps: E0921.
    let (mut d, mut a) = (desk(), Agent::default());
    ask(&mut a, "wait forever");
    d.run(&mut a, &|b| reply("", Some(("wait", &format!("{{\"ms\":{}}}", messages(b).len())))));
    let said = shown(&mut a);
    assert!(
        d.bodies.len() == MAX_STEPS as usize && said.contains("E0921 out of steps"),
        "{} {said}",
        d.bodies.len()
    );
    // The same call on the same screen a third time: no progress, E0924.
    let (mut d, mut a) = (desk(), Agent::default());
    ask(&mut a, "wait in place");
    d.run(&mut a, &|_| reply("", Some(("wait", r#"{"ms":0}"#))));
    assert!(d.bodies.len() == 3 && shown(&mut a).contains("E0924 I couldn't finish: no progress"));
}

#[test]
fn the_person_takes_over_answers_questions_and_feedback_waits_for_a_yes() {
    // Halt mid-task: it stops, cancelling what the model was asked, and says so.
    let mut a = Agent::default();
    ask(&mut a, "anything");
    let look = a
        .frame()
        .requests
        .iter()
        .find_map(|r| match r {
            Request::Act { id, .. } => Some(*id),
            _ => None,
        })
        .unwrap();
    a.event(&Event::Acted { id: look, code: 0, note: "".into(), scene: Scene::default().encode() });
    let id = a
        .frame()
        .requests
        .iter()
        .find_map(|r| match r {
            Request::Ai { id, .. } => Some(*id),
            _ => None,
        })
        .unwrap();
    a.event(&Event::Halt);
    let f = a.frame();
    assert_eq!(f.requests, [Request::AiCancel { id }, Request::Status { working: false }]);
    assert!(shown(&mut a).contains("Stopped: you took over."));
    // Typing into Feedback waits for the person's yes, asked; then it goes.
    let (mut d, mut a) = (desk(), Agent::default());
    d.host.open("terminal", None, &mut Response::default());
    let feedback = Win { id: 1, app: "feedback".into(), title: "Feedback".into(), ..page(1, true) };
    let mut t = Task {
        prompt: "x".into(),
        over: 1,
        screen: "".into(),
        steps: Vec::new(),
        scene: Scene { focus: 1, wins: vec![feedback], ..Scene::default() },
        shown: Vec::new(),
        wait: Wait::User,
        calls: 0,
        acts: 0,
        fails: 0,
        repeat: (0, 0),
        approved: false,
        usage: (0, 0),
    };
    t.shown = render(&t.scene, &mut a.refs, 1).1;
    let typed = Call {
        id: "c".into(),
        name: "type_text".into(),
        args: r#"{"ref":"e2","text":"hi"}"#.into(),
    };
    assert!(a.prepare(&t, &typed).unwrap_err().starts_with("E0916: refused: Feedback"));
    t.approved = true;
    assert!(matches!(a.prepare(&t, &typed), Ok((Act::Type { win: 1, .. }, ..))));
    // ask_user: the question shows, the overlay is the person's again; their answer goes back.
    ask(&mut a, "send feedback");
    let calls = Rc::new(RefCell::new(0));
    d.run(&mut a, &|_| {
        *calls.borrow_mut() += 1;
        match *calls.borrow() {
            1 => reply("", Some(("ask_user", r#"{"question":"Send it?"}"#))),
            _ => reply("Sent.", None),
        }
    });
    assert!(shown(&mut a).ends_with("Send it?") && !d.host.agent.working);
    ask(&mut a, "yes");
    d.run(&mut a, &|_| reply("Sent.", None));
    let tool = messages(d.bodies.last().unwrap()).into_iter().find(|m| m.0 == "tool").unwrap();
    // With the screen, as it still is.
    assert!(
        tool.1.starts_with("the user answered: yes\nScreen ") && shown(&mut a).contains("Sent.")
    );
}

#[test]
fn the_pill_keys_receipts_and_ai_errors() {
    // In the pill: one line of what it does, and Stop.
    let mut a = Agent::default();
    assert!(a.event(&Event::Resize { w: 420, h: 72 }));
    ask(&mut a, "go");
    let f = a.frame();
    let line =
        Node::Text { id: 0, style: Style::Body, text: "Looking at the screen\u{2026}".into() };
    let stop = Node::Button { id: STOP, variant: Variant::Normal, label: "Stop".into() };
    assert_eq!(f.nodes, [Node::Row { id: 0, gap: 12, children: vec![line, stop] }]);
    // The card again: the prompt has the keys. An AI error ends the task, coded, with Retry;
    // Retry asks again.
    a.event(&Event::Resize { w: 560, h: 480 });
    assert_eq!(a.frame().requests, [Request::Focus { id: a.input_id() }]);
    let look = f
        .requests
        .iter()
        .find_map(|r| match r {
            Request::Act { id, .. } => Some(*id),
            _ => None,
        })
        .unwrap();
    a.event(&Event::Acted { id: look, code: 0, note: "".into(), scene: Scene::default().encode() });
    let id = a
        .frame()
        .requests
        .iter()
        .find_map(|r| match r {
            Request::Ai { id, .. } => Some(*id),
            _ => None,
        })
        .unwrap();
    a.event(&Event::AiEnd { id, status: 429, error: "".into() });
    let f = a.frame();
    assert!(f.nodes.contains(&Node::Button {
        id: RETRY,
        variant: Variant::Chip,
        label: "Retry".into()
    }));
    assert!(shown(&mut a).contains("E0903 the free AI is busy"));
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
