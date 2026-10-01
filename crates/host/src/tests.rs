use std::cell::RefCell;
use std::rc::Rc;

use ui::theme::{GAP, PANEL_H, WORKSPACES};
use ui::{App, AppEvent as E};
use wm::Gaps;

use super::*;

const SANS: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
const SYM_A: &[u8] = include_bytes!("../../../assets/fonts/lazy/symbols-a.ttf");
const SYM_B: &[u8] = include_bytes!("../../../assets/fonts/lazy/symbols-b.ttf");
const PORT: u16 = 8123;
/// The names the registry knows; the first three open at the start.
const KNOWN: &str = "welcome terminal /apps/counter.app launcher huge nan";

type Log = Rc<RefCell<Vec<(u32, E)>>>;

/// A scripted app: instance number (1, 2, ... as created, so window `n`
/// while every open succeeds), name and log. It logs every event, and runs
/// the `;`-separated commands of its text.
struct Probe(u32, &'static str, Log);

impl App for Probe {
    fn title(&self) -> String {
        self.1.to_uppercase()
    }

    fn draw(&mut self, _: &mut Ui<'_>) {}

    fn event(&mut self, ev: E, cx: &mut Cx<'_>) -> bool {
        self.2.borrow_mut().push((self.0, ev.clone()));
        let port = cx.pairing.map_or(1, |p| p.port);
        let E::Text(cmds) = ev else {
            return false;
        };
        for cmd in cmds.split(';') {
            let (verb, arg) = cmd.split_once(' ').unwrap_or((cmd, ""));
            let id = SocketId(arg.parse().unwrap_or(0));
            match verb {
                "open" => cx.open(arg),
                "float" => cx.open_floating(arg),
                "close" => cx.close_self(),
                "connect" => drop(cx.connect(port)),
                "send" => cx.send(id, b"hi".to_vec()),
                "drop" => cx.close_socket(id),
                "fonts" => cx.load_fallback_fonts(),
                _ => {}
            }
        }
        true
    }

    fn preferred_size(&self) -> Option<(f32, f32)> {
        let names = ["launcher", "huge", "nan"];
        let i = names.iter().position(|n| *n == self.1)?;
        Some([(400.0, 300.0), (5e3, 5e3), (f32::NAN, 100.0)][i])
    }
}

/// A host as the shell makes one on a 1280 x 800 screen, paired on `PORT`,
/// with the first three apps open, window 1 focused, and its log cleared.
fn host() -> (Host, Log) {
    let (log, n) = (Log::default(), Rc::new(RefCell::new(0)));
    let l = log.clone();
    let registry: Registry = Box::new(move |name| {
        let k = KNOWN.split(' ').find(|k| *k == name)?;
        *n.borrow_mut() += 1;
        Some(Box::new(Probe(*n.borrow(), k, l.clone())) as Box<dyn App>)
    });
    let mut gaps = Gaps::default();
    [gaps.outer, gaps.inner] = [GAP; 2];
    let wm = Wm::new(Rect::new(0, PANEL_H as i32, 1280, 764), gaps, WORKSPACES);
    let text = TextSystem::new(SANS.to_vec()).unwrap();
    let pairing = ui::parse_pairing(&format!("node={PORT}&token={}", "0f".repeat(32)));
    let mut h = Host::new(wm, text, Vfs::new(), registry, pairing);
    let mut out = Response::default();
    for name in KNOWN.split(' ').take(3) {
        h.open(name, false, &mut out);
    }
    h.apply(Cmd::Focus(WinId(1)));
    h.settle(&mut out);
    log.take();
    (h, log)
}

fn ws_open(id: u32, port: u16) -> Effect {
    let url = format!("ws://127.0.0.1:{port}/");
    Effect::WsOpen { id, url }
}

/// Test drivers: each runs one call and returns its response.
impl Host {
    /// Text to window `n`'s app: its commands.
    fn say(&mut self, n: u32, t: &str) -> Response {
        let mut out = Response::default();
        self.deliver(WinId(n), E::Text(t.to_string()), &mut out);
        out
    }
    fn sock(&mut self, id: u32, ev: WsEvent) -> Response {
        let mut out = Response::default();
        self.ws(SocketId(id), ev, &mut out);
        out
    }
    fn fetch(&mut self, id: u32, got: Result<Vec<u8>, String>) -> Response {
        let mut out = Response::default();
        self.fetched(id, got, &mut out);
        out
    }
    fn rect_of(&self, n: u32) -> Option<Rect> {
        let layout = self.wm().layout();
        layout.iter().find(|p| p.win == WinId(n)).map(|p| p.rect)
    }
    fn names(&self) -> Vec<(u32, &str)> {
        self.wins.iter().map(|w| (w.id.0, &*w.name)).collect()
    }
}

#[test]
fn sockets_belong_to_the_window_that_opened_them() {
    let (mut h, log) = host();
    let socket = SocketId(1);
    let got = |ev| (1, E::Ws { socket, ev });
    assert_eq!(h.say(1, "connect").effects, [ws_open(1, PORT)]);
    h.sock(1, WsEvent::Open);
    h.sock(9, WsEvent::Error);
    assert_eq!(log.take()[1..], [got(WsEvent::Open)]);
    // Only the owner sends on or closes a socket.
    let (id, bytes) = (1, b"hi".to_vec());
    let hi = [Effect::WsSend { id, bytes }];
    assert_eq!(h.say(1, "send 1;send 7;drop 7").effects, hi);
    assert!(h.say(2, "send 1;drop 1").effects.is_empty());
    let effects = h.say(1, "connect;drop 2").effects;
    assert_eq!(effects, [ws_open(2, PORT), Effect::WsClose { id: 2 }]);
    assert!(h.sock(2, WsEvent::Open).effects.is_empty());
    // A closed socket is forgotten after its last event.
    log.take();
    let reason = "gone".to_string();
    let closed = WsEvent::Closed { code: 1, reason };
    h.sock(1, closed.clone());
    h.sock(1, WsEvent::Open);
    assert_eq!(log.take(), [got(closed)]);
    assert!(h.say(1, "send 1").effects.is_empty());
    // Closing a window closes its sockets; a closed app asks for nothing more.
    h.say(1, "connect;connect");
    let mut out = Response::default();
    h.close(WinId(1), &mut out);
    assert_eq!(out.effects, [3, 4].map(|id| Effect::WsClose { id }));
    let effects = h.say(2, "connect;close;connect;open terminal").effects;
    assert_eq!(effects, [ws_open(5, PORT), Effect::WsClose { id: 5 }]);
    assert!(h.wins.len() == 1 && h.sockets.is_empty());
}

#[test]
fn fonts_are_fetched_once_and_added_in_order() {
    let (mut h, _log) = host();
    let fetch = |id, f| {
        let url = format!("fonts/symbols-{f}.ttf");
        Effect::Fetch { id, url }
    };
    let effects = h.say(1, "fonts;fonts").effects;
    assert_eq!(effects, [fetch(1, "a"), fetch(2, "b")]);
    assert!(h.say(1, "fonts").effects.is_empty());
    // b waits for a; repeats and unknown ids do nothing.
    assert!(!h.fetch(2, Ok(SYM_B.to_vec())).redraw);
    assert_eq!(h.text_mut().fallback_count(), 0);
    assert!(h.fetch(1, Ok(SYM_A.to_vec())).redraw);
    assert_eq!(h.text_mut().fallback_count(), 2);
    for id in [1, 2, 3] {
        assert_eq!(h.fetch(id, Ok(SYM_A.to_vec())), Response::default());
    }
    // A failed or bad font is skipped.
    for first in [Err("404".to_string()), Ok(vec![1, 2, 3])] {
        let (mut h, _log) = host();
        h.say(1, "fonts");
        assert!(!h.fetch(1, first).redraw);
        assert!(h.fetch(2, Ok(SYM_B.to_vec())).redraw);
        assert_eq!(h.text_mut().fallback_count(), 1);
    }
    // Fonts already there are not fetched again.
    let (mut h, _log) = host();
    h.text_mut().add_fallback(SYM_A.to_vec()).unwrap();
    assert!(h.say(1, "fonts").effects.is_empty());
}

#[test]
fn apps_open_and_close_windows() {
    let (mut h, log) = host();
    h.say(1, "open terminal;open nope;float launcher");
    assert_eq!(h.names()[3..], [(4, "terminal"), (5, "launcher")]);
    // A floating window takes its app's size, centered.
    assert_eq!(h.rect_of(5), Some(Rect::new(439, 252, 402, 331)));
    h.settle(&mut Response::default());
    let sized = (5, E::Resized { w: 400.0, h: 300.0 });
    assert!(log.take().contains(&sized));
    // One launcher at a time: opening it again focuses it.
    let mut out = Response::default();
    h.apply(Cmd::SwitchWorkspace(2));
    h.open("launcher", true, &mut out);
    let at = (h.wm().focused(), h.wm().active_workspace());
    assert_eq!((at, h.wins.len()), ((Some(WinId(5)), 0), 5));
    h.say(5, "close");
    h.open("launcher", true, &mut out);
    assert_eq!(h.names()[3..], [(4, "terminal"), (6, "launcher")]);
    // Oversized apps fit the screen; sizes that are not finite are ignored.
    h.say(6, "float huge;float nan");
    assert_eq!(h.rect_of(7), Some(Rect::new(0, 36, 1280, 764)));
    assert_eq!(h.rect_of(8), Some(Rect::new(213, 163, 853, 509)));
}
