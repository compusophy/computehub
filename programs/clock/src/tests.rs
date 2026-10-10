use super::*;

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| w.to_string()).collect()
}

/// The Line draws of `d`, as (x2, y2, width).
fn hands(d: &[Draw]) -> Vec<(i16, i16, i16)> {
    let lines = d.iter().filter(|d| d.shape == Shape::Line).map(|d| (d.at[2], d.at[3], d.at[4]));
    lines.collect::<Vec<_>>()[60..].to_vec()
}

#[test]
fn its_name_says_what_it_shows_and_tz_sets_its_zone() {
    assert_eq!(Clock::new(&argv(&["/bin/grandfather", "tz=-360"])).kind, Kind::Grandfather);
    assert_eq!(Clock::new(&argv(&["shop"])).kind, Kind::Shop);
    assert_eq!(Clock::new(&argv(&["clock"])).kind, Kind::Face);
    assert_eq!(Clock::new(&argv(&["clock", "tz=-360"])).tz, -360);
    assert_eq!(Clock::new(&argv(&["clock", "tz=99999"])).tz, 14 * 60);
    assert_eq!(Clock::new(&argv(&["clock", "tz=x"])).tz, 0);
}

#[test]
fn the_hands_point_where_the_time_says() {
    // 3:00:00: the hour hand at 3 (right), the minute hand and the second hand at 12 (up).
    let h = hands(&face(3 * 3_600_000));
    let c = (FACE / 2) as i16;
    assert_eq!(h[0], (c + 48, c, 7));
    assert_eq!(h[1], (c, c - 72, 4));
    assert_eq!((h[2].0, h[2].1), (c, c - 80));
    // 6:30: the hour hand halfway between 6 and 7, the minute hand at 6 (down).
    let h = hands(&face(6 * 3_600_000 + 30 * 60_000));
    assert!(h[0].0 < c && h[0].1 > c, "{:?}", h[0]);
    assert_eq!((h[1].0, h[1].1), (c, c + 72));
}

#[test]
fn the_zone_moves_the_hands_and_a_day_wraps() {
    let mut c = Clock::new(&argv(&["clock", "tz=-360"]));
    c.now_ms = 3 * 3_600_000; // 03:00 UTC is 21:00 six hours west: the hour hand at 9.
    assert_eq!(c.day_ms(), 21 * 3_600_000);
}

#[test]
fn every_kind_draws_a_frame_that_decodes_and_asks_its_ticks_once() {
    for (name, ticks) in [("clock", Some(1000)), ("grandfather", Some(SWING_MS)), ("shop", None)] {
        let mut c = Clock::new(&argv(&[name, "tz=120"]));
        assert!(c.event(&Event::Resize { w: 400, h: 600 }, 1_000_000));
        let first = c.frame();
        let bytes = first.encode_checked().expect(name);
        assert_eq!(Frame::decode(&bytes).as_ref(), Some(&first));
        let asked = first.requests.iter().filter_map(|r| match r {
            Request::Timer { ms } => Some(*ms),
            _ => None,
        });
        assert_eq!(asked.collect::<Vec<_>>(), ticks.into_iter().collect::<Vec<_>>());
        assert!(c.frame().requests.is_empty());
    }
}

#[test]
fn a_holder_holds_what_it_says_and_passes_its_zone_on() {
    let held = |name| {
        let mut out = Vec::new();
        fn walk(n: &[Node], out: &mut Vec<(u32, String, String)>) {
            for n in n {
                match n {
                    Node::Embed { id, program, args, .. } => {
                        out.push((*id, program.clone(), args.clone()))
                    }
                    _ => walk(n.children(), out),
                }
            }
        }
        walk(&Clock::new(&argv(&[name, "tz=-360"])).frame().nodes, &mut out);
        out
    };
    let z = || "tz=-360".to_string();
    assert_eq!(held("grandfather"), vec![(HELD_FACE, "clock".into(), z())]);
    let shop = held("shop");
    assert_eq!(
        shop,
        vec![
            (WALL, "clock".into(), z()),
            (LEFT, "grandfather".into(), z()),
            (RIGHT, "grandfather".into(), z())
        ]
    );
    assert!(held("clock").is_empty());
}

#[test]
fn the_pendulum_swings_and_stays_in_its_window() {
    let bob = |ms| match pendulum(ms) {
        Node::Canvas { draws, .. } => (draws[3].at[0], draws[3].at[1]),
        _ => unreachable!(),
    };
    let (rest, left, right) = (bob(0), bob(PERIOD_MS / 4), bob(3 * PERIOD_MS / 4));
    assert_eq!(rest.0, (PENDULUM.0 / 2) as i16);
    assert!(left.0 > rest.0 && right.0 < rest.0);
    for ms in (0..PERIOD_MS).step_by(50) {
        let (x, y) = bob(ms);
        assert!(x - 14 >= 0 && x + 14 < PENDULUM.0 as i16 && y + 14 < PENDULUM.1 as i16, "{ms}");
    }
}
