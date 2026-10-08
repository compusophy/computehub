use uiwire::{Event, Node, Request};

use crate::*;

#[test]
fn a_tile_answers_its_fuel_hash_and_pixels_the_same_every_time() {
    let line = chunk(&HOME, 4, 6);
    let a = work_tile(&line).unwrap();
    assert_eq!(Some(a.clone()), work_tile(&line));
    let mut w = a.split(' ');
    let (fuel, hash, pixels) = (w.next().unwrap(), w.next().unwrap(), w.next().unwrap());
    assert!(fuel.parse::<u64>().unwrap() > 0 && w.next().is_none());
    assert_eq!(
        (hash, pixels.len()),
        (sha::hex(&sha::sha256(pixels.as_bytes())).as_str(), (TILE * TILE) as usize)
    );
    assert!(pixels.bytes().all(|b| b"012".contains(&b)));
    // A tile inside the set is all 0, and costs the most.
    let inside = work_tile(&chunk(&View { x: -0.2, y: 0.0, span: 0.01 }, 5, 5)).unwrap();
    assert!(inside.ends_with(&"0".repeat((TILE * TILE) as usize)));
    // Lines that are no tile.
    for bad in ["", "1 2", "1 2 0 0 0", "99 0 0 0 1", "0 0 0 0 1 extra", "a b c d e"] {
        assert_eq!(work_tile(bad), None, "{bad}");
    }
}

#[test]
fn the_window_renders_every_tile_shows_them_and_zooms_where_tapped() {
    let mut f = Fractal::default();
    assert!(f.event(&Event::Resize { w: 400, h: 600 }));
    let frame = f.frame();
    let [Request::Job { name, chunks }] = &frame.requests[..] else {
        panic!("{:?}", frame.requests)
    };
    assert_eq!((name.as_str(), chunks.len()), ("fractal", (SIDE * SIDE) as usize));
    // An answer shows its tile in its device's color; a malformed one is dropped.
    let out = work_tile(&chunks[0]).unwrap();
    assert!(f.event(&Event::Done { index: 0, node: 1, out }));
    assert!(f.event(&Event::Done { index: 1, node: 0, out: "1 x 12".into() }));
    assert_eq!(f.tiles.iter().filter(|t| t.is_some()).count(), 1);
    let frame = f.frame();
    let Node::Scroll { children, .. } = &frame.nodes[0] else { panic!() };
    let Node::Col { children, .. } = &children[0] else { panic!() };
    let Node::Canvas { draws, .. } = &children[1] else { panic!("{:?}", children[1]) };
    assert_eq!(draws.len(), 1);
    // A tap zooms in four times there and renders again; Zoom out goes back.
    assert!(f.event(&Event::Tap { id: PICTURE, cell: PX / 2 * PX + PX / 2 }));
    assert_eq!(f.view.span, HOME.span / 4.0);
    assert!(f.frame().requests.len() == 1 && f.tiles.iter().all(Option::is_none));
    assert!(f.event(&Event::Click { id: OUT }));
    assert_eq!(f.view.span, HOME.span);
}
