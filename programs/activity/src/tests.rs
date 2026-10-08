use uiwire::stat::{LOUD, Proc, Stats};

use super::*;

/// A process as a sample lists it: pid, window, state, argv, DRAWs; then its meters (ms busy,
/// KB), if it has them.
type P = (u32, u32, u8, &'static str, u32, Option<(u32, u32)>);

/// The desktop's sample at `at` ms: `loud` counts by index (the rest 0), the processes, the
/// desktop's µs and frames by the watcher, a timer and other; Activity's own meters (`own` ms,
/// 9,000 KB).
fn sample(at: u32, loud: &[(usize, u32)], procs: &[P], us: u32, own: u32) -> Event {
    let mut counts = vec![0; LOUD];
    loud.iter().for_each(|&(i, n)| counts[i] = n);
    let argv = |s: &str| s.split(' ').map(String::from).collect();
    let proc =
        |r: &P| Proc { pid: r.0, window: r.1, state: r.2, argv: argv(r.3), counts: vec![r.4] };
    let meters = procs.iter().filter_map(|r| r.5.map(|(b, kb)| (r.0, vec![b, kb]))).collect();
    let quiet = vec![1, 2, 3, us, 18_000];
    let s = Stats {
        at,
        loud: counts,
        procs: procs.iter().map(proc).collect(),
        meters,
        quiet,
        own: vec![own, 9_000],
    };
    Event::Stats { data: s.encode() }
}

/// Every node of `nodes` and their children, in pre-order.
fn all(nodes: &[Node]) -> Vec<&Node> {
    nodes.iter().flat_map(|n| [vec![n], all(n.children())].concat()).collect()
}

/// The texts of `nodes`, in order: Texts, Buttons' labels, Entries' names and cells.
fn texts(nodes: &[Node]) -> Vec<String> {
    let text = |n: &&Node| match n {
        Node::Text { text, .. } | Node::Button { label: text, .. } => Some(text.clone()),
        Node::Entry { text, detail, .. } => Some([text, "|", detail].concat()),
        _ => None,
    };
    all(nodes).iter().filter_map(text).collect()
}

/// The values of the Charts of `nodes`, in order.
fn charts(nodes: &[Node]) -> Vec<Vec<u16>> {
    let chart = |n: &&Node| match n {
        Node::Chart { values, .. } => Some(values.clone()),
        _ => None,
    };
    all(nodes).iter().filter_map(chart).collect()
}

/// Activity `w` wide after `events`, and its last frame's nodes.
fn shown(w: u16, events: &[Event]) -> (Activity, Vec<Node>) {
    let mut a = Activity::default();
    a.event(&Event::Resize { w, h: 560 });
    events.iter().for_each(|e| _ = a.event(e));
    let nodes = a.frame().nodes;
    (a, nodes)
}

#[test]
fn it_watches_from_its_first_size_and_frames_only_what_changes() {
    let mut a = Activity::default();
    assert!(a.event(&Event::Resize { w: 720, h: 560 }));
    let f = a.frame();
    assert_eq!(
        (f.title.as_str(), &f.requests[..]),
        ("Activity", &[Request::Watch { on: true }][..])
    );
    assert!(matches!(f.nodes[0], Node::Pages { on: 0, .. }));
    assert!(texts(&f.nodes).contains(&"Asking the desktop\u{2026}".to_string()));
    // The same again changes nothing; a sample does.
    assert!(!a.event(&Event::Resize { w: 720, h: 560 }) && !a.event(&Event::Focus { on: false }));
    assert!(a.event(&sample(1000, &[], &[], 0, 0)));
    // A phone's window watches while it has the focus, and measures anew after a pause.
    let mut a = Activity::default();
    a.event(&Event::Resize { w: 400, h: 800 });
    a.event(&sample(1000, &[], &[], 0, 0));
    a.event(&Event::Focus { on: false });
    a.event(&sample(2000, &[], &[], 0, 0));
    let watched: Vec<_> = a.frame().requests;
    assert_eq!(watched, [Request::Watch { on: true }, Request::Watch { on: false }]);
    a.event(&Event::Focus { on: true });
    a.event(&sample(9000, &[], &[], 0, 0));
    assert!(a.prev.is_none() && a.now.is_none(), "no rate spans the pause");
}

#[test]
fn the_graphs_take_a_point_a_second_the_seconds_between_samples_at_their_average() {
    let spin: P = (2, 5, stat::RUNS, "spin", 0, Some((0, 2_000)));
    let spun = |busy| (2, 5, stat::RUNS, "spin", 0, Some((busy, 2_000)));
    let (a, _) = shown(
        720,
        &[
            sample(1_000, &[], &[spin], 0, 0),
            // A second: the program a whole core, the desktop a tenth, 30 frames for the person.
            sample(2_000, &[(stat::INPUT, 30)], &[spun(1_000)], 100_000, 0),
            // Then three seconds at once: the program half of them, 30 frames more.
            sample(5_000, &[(stat::INPUT, 60)], &[spun(2_500)], 100_000, 0),
        ],
    );
    assert_eq!(a.graphs[0].0, [1_100, 500, 500, 500]);
    assert_eq!(a.graphs[2].0, [30, 10, 10, 10]);
    // Memory is a level: the desktop's, the program's and Activity's own; the seconds before a
    // change as they were.
    assert_eq!(a.graphs[1].0, [29_000; 5]);
    let rows = &a.rows.iter().find(|r| r.0 == 2).unwrap().1;
    assert_eq!(rows.0, [1_000, 500, 500, 500]);
    // A minute is kept; as a chart, the seconds not yet seen unknown, the rest per mille of the top.
    let mut h = History::default();
    h.add(100, 7);
    assert_eq!(h.0.len(), SECONDS);
    let Node::Chart { values, .. } = History(vec![500, 1_000, 2_000]).chart(11, 64, 1_000) else {
        panic!()
    };
    assert_eq!((values.len(), &values[57..]), (SECONDS, &[500, 1_000, 1_000][..]));
    assert!(values[..57].iter().all(|&v| v == uiwire::UNKNOWN), "not yet seen");
    assert_eq!([nice(0), nice(11), nice(36_250), nice(u32::MAX)], [10, 20, 50_000, u32::MAX]);
}

#[test]
fn performance_shows_four_graphs_and_storage_side_by_side_when_wide() {
    let ai = [
        (stat::ASKED, 3),
        (stat::TOKENS_IN, 11_000),
        (stat::TOKENS_OUT, 1_400),
        (stat::MICROUSD, 19_400),
        (stat::FAILED, 1),
    ];
    let home = [(stat::HOME, 4_200_000)];
    let events = [
        sample(1_000, &home, &[], 0, 0),
        sample(2_000, &[&home[..], &ai].concat(), &[], 50_000, 30),
    ];
    let (_, nodes) = shown(720, &events);
    let said = texts(&nodes);
    for line in [
        "CPU",
        "8%",
        "Desktop 5%\u{b7}programs 3%",
        "Memory",
        "28 MB",
        "Frames",
        "0 fps",
        "Nothing is drawing.",
        "12.4k tokens",
        "3 requests\u{b7}about $0.02\u{b7}1 failed",
        "4.2 MB of about 5 MB",
        "Show your files",
    ] {
        assert!(said.iter().any(|s| s.replace(DOT, "\u{b7}").contains(line)), "{line}: {said:?}");
    }
    assert_eq!(charts(&nodes).len(), 4);
    let pairs = nodes
        .iter()
        .filter(|n| matches!(n, Node::Row { children, .. } if children.len() == 2))
        .count();
    assert_eq!(pairs, 2, "two cards a row");
    // Nearly full is yellow; unkept red, saying why. Narrow, the cards stack.
    let meter = |nodes: &[Node]| {
        all(nodes).iter().find_map(|n| match n {
            Node::Meter { hue, value, .. } => Some((*hue, *value)),
            _ => None,
        })
    };
    assert_eq!(meter(&nodes), Some((3, 840)));
    let (_, nodes) = shown(400, &[sample(1_000, &[(stat::UNKEPT, 1)], &[], 0, 0)]);
    assert_eq!(meter(&nodes), Some((1, 0)));
    assert!(texts(&nodes).iter().any(|s| s.starts_with("This browser refused")));
    assert!(!nodes.iter().any(|n| matches!(n, Node::Row { .. })));
}

#[test]
fn processes_is_a_table_sorted_by_the_column_picked_and_a_row_opens_its_page() {
    #[rustfmt::skip]
    let procs: [P; 3] = [
        (2, 5, stat::RUNS, "spin", 0, Some((0, 3_000))), (3, 6, stat::IDLE, "files", 0, Some((0, 8_000))),
        (4, 7, stat::RUNS, "studio run /apps/Snake.app", 0, Some((0, 1_000))),
    ];
    let later = [
        (2, 5, stat::RUNS, "spin", 0, Some((900, 3_000))),
        procs[1],
        (4, 7, stat::RUNS, "studio run /apps/Snake.app", 30, Some((100, 1_000))),
    ];
    let events = [
        sample(1_000, &[], &procs, 0, 0),
        sample(2_000, &[], &later, 20_000, 10),
        Event::Click { id: NAV + 1 },
    ];
    let (mut a, nodes) = shown(720, &events);
    assert!(matches!(nodes[0], Node::Pages { on: 1, .. }));
    assert!(nodes.contains(&Node::Columns {
        id: SORT,
        on: 2,
        labels: "Name\tCPU\tMemory\tFrames".into()
    }));
    let rows: Vec<String> = texts(&nodes).into_iter().filter(|s| s.contains('|')).collect();
    #[rustfmt::skip]
    assert_eq!(rows, [
        "spin\nin Terminal \u{b7} working|90%\t3.1 MB\t0", "Snake.app\ndrawing|10%\t1.0 MB\t30",
        "The desktop\nhome screen and windows|2%\t18 MB\t\u{2014}", "Activity\nthis window|1%\t9.2 MB\t\u{2014}",
        "Files\nwaiting for you|0%\t8.2 MB\t0",
    ]);
    // By name, then by memory.
    a.event(&Event::Click { id: SORT });
    let first =
        |a: &mut Activity| texts(&a.frame().nodes).into_iter().find(|s| s.contains('|')).unwrap();
    assert!(first(&mut a).starts_with("Activity"));
    a.event(&Event::Click { id: SORT + 2 });
    assert!(first(&mut a).starts_with("The desktop"));
    // A program's page: its CPU over the minute and End, which asks the desktop and goes back.
    a.event(&Event::Click { id: ROW + 2 });
    let page = texts(&a.frame().nodes);
    assert!(
        page.contains(&"spin".to_string()) && page.contains(&"End spin".to_string()),
        "{page:?}"
    );
    assert!(page.iter().any(|s| s.contains("It stops at once, as Ctrl+C would.")));
    a.event(&Event::Click { id: END });
    let f = a.frame();
    assert_eq!((a.page, &f.requests[..]), (Page::Processes, &[Request::End { pid: 2 }][..]));
    assert!(!texts(&f.nodes).iter().any(|s| s.starts_with("spin")), "ended, left out");
    // The desktop's page and Activity's have no End; Escape goes back; a page gone, the table.
    for id in [DESKTOP, OWN] {
        a.event(&Event::Click { id });
        assert!(!texts(&a.frame().nodes).iter().any(|s| s.starts_with("End")));
        a.event(&Event::Key { id: 0, key: Key::Escape, mods: 0, ch: '\0' });
        assert_eq!(a.page, Page::Processes);
    }
    a.event(&Event::Click { id: ROW + 4 });
    a.event(&sample(3_000, &[], &procs[..2], 0, 0));
    assert_eq!(a.page, Page::Processes);
    // Narrow, the table drops its frames column.
    let (_, nodes) = shown(400, &events);
    assert!(nodes.contains(&Node::Columns { id: SORT, on: 2, labels: "Name\tCPU\tMemory".into() }));
}

#[test]
fn numbers_read_as_people_say_them() {
    assert_eq!(
        [740, 212_000, 999_999, 1_300_000, 18_400_000].map(bytes),
        ["740 B", "212 KB", "1.0 MB", "1.3 MB", "18 MB"]
    );
    assert_eq!(
        [None, Some(0), Some(5), Some(120), Some(1800)].map(percent),
        ["\u{2014}", "0%", "<1%", "12%", "180%"]
    );
    assert_eq!([940, 12_400, 1_234_567].map(tokens), ["940", "12.4k", "1.2M"]);
    assert_eq!(
        [1, 4_999, 5_000, 19_400, 1_254_999].map(dollars),
        ["under $0.01", "under $0.01", "about $0.01", "about $0.02", "about $1.25"]
    );
    assert_eq!([growth(5, 9), growth(9, 5), growth(u32::MAX, 1)], [4, 0, 2]);
}

#[test]
fn a_pool_snapshot_close_on_the_last_still_brings_each_devices_model_and_speed() {
    use uiwire::pool::{Device, Snap};
    let (mut page, mut asked) = (PoolPage::default(), Vec::new());
    let snap = |at, tok, units| {
        let d = Device { name: "A".into(), model: "m".into(), tok, units, ..Device::default() };
        Event::Pool { data: Snap { at, devices: vec![d], ..Snap::default() }.encode() }
    };
    for (at, tok, units) in [(1000, 455, 0), (2000, 455, 1000), (2250, 439, 5000)] {
        page.event(&snap(at, tok, units), &mut asked);
    }
    let d = &page.cur.as_ref().unwrap().devices[0];
    assert_eq!((d.tok, d.units), (439, 1000), "its speed as it is now, its counter for the rates");
}
