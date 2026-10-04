use super::*;

/// A chat's transcript and memory, as the Assistant holds the current one's.
type Talk = (Vec<Turn>, Memory);

/// One task's transcript and memory: `prompt`, answered `answer`.
fn talk(prompt: &str, answer: &str) -> Talk {
    let turn = Turn { prompt: prompt.into(), lines: vec![(Style::Body, answer.into())] };
    (vec![turn], vec![(prompt.into(), answer.into())])
}

/// Eight chats of a task each, named `name(0)` to `name(7)`, the last the current one.
fn eight(name: &dyn Fn(usize) -> String) -> (Chats, Talk) {
    let (mut c, mut t) = (Chats::default(), Talk::default());
    for i in 0..8 {
        if i > 0 {
            assert_eq!(click(&mut c, NEW, &mut t), Some(Clicked::New));
        }
        t = talk(&name(i), "Done.");
        c.name(&name(i));
    }
    (c, t)
}

fn click(c: &mut Chats, id: u32, t: &mut Talk) -> Option<Clicked> {
    c.click(id, &mut t.0, &mut t.1)
}

/// Each chat's name, the current one first.
fn names(c: &Chats) -> Vec<String> {
    c.list.iter().map(|c| c.name.clone()).collect()
}

/// The row's buttons as (id, variant, label), their width as estimated, and the list over it.
fn shown(c: &Chats, w: u16, t: &Talk) -> (Vec<(u32, Variant, String)>, u32, Vec<Node>) {
    let mut nodes = c.row(w, &t.0, &t.1);
    let Some(Node::Row { children, .. }) = nodes.pop() else { return (Vec::new(), 0, nodes) };
    let width = children.iter().map(wide).sum::<u32>() - GAP;
    let buttons = children.into_iter().filter_map(|b| match b {
        Node::Button { id, variant, label } => Some((id, variant, label)),
        _ => None,
    });
    (buttons.collect(), width, nodes)
}

fn on(n: u32, label: &str) -> (u32, Variant, String) {
    (CHAT + n, Variant::On, label.into())
}

fn quiet(id: u32, label: &str) -> (u32, Variant, String) {
    (id, Variant::Quiet, label.into())
}

#[test]
fn the_chat_left_longest_ago_goes_never_the_one_just_left() {
    let (mut c, mut t) = eight(&|i| format!("task {i}"));
    assert_eq!(names(&c), (0..8).rev().map(|i| format!("task {i}")).collect::<Vec<_>>());
    // Back to the first: current, with its own transcript and memory; the one left next to it.
    let first = CHAT + c.list[7].n;
    assert_eq!(click(&mut c, first, &mut t), Some(Clicked::Switched));
    assert_eq!(t, talk("task 0", "Done."));
    assert_eq!(names(&c)[..2], ["task 0", "task 7"]);
    // A ninth: the one left longest ago goes, never the one just left; it starts empty.
    assert_eq!(click(&mut c, NEW, &mut t), Some(Clicked::New));
    assert_eq!(t, Talk::default());
    let all = ["", "task 0", "task 7", "task 6", "task 5", "task 4", "task 3", "task 2"];
    assert_eq!(names(&c), all);
    assert_eq!(c.list[1].memory, talk("task 0", "Done.").1);
    // None starts from an empty one, and an empty one switched away from goes.
    assert_eq!(click(&mut c, NEW, &mut t), None);
    let seven = CHAT + c.list[2].n;
    assert_eq!(click(&mut c, seven, &mut t), Some(Clicked::Switched));
    assert_eq!((c.list.len(), names(&c)[..2] == ["task 7", "task 0"]), (7, true));
    assert_eq!(t, talk("task 7", "Done."));
    // The current one's chip shows the list, and its item there hides it; another id is not
    // the chats'.
    let (chip, item) = (CHAT + c.list[0].n, ITEM + c.list[0].n);
    assert_eq!(click(&mut c, chip, &mut t), Some(Clicked::Shown));
    assert!(c.listing);
    assert_eq!(click(&mut c, item, &mut t), Some(Clicked::Shown));
    assert!(!c.listing);
    assert_eq!(click(&mut c, 1, &mut t), None);
}

#[test]
fn one_row_holds_what_fits_and_the_list_deletes_asked_again() {
    // Nothing while the one chat holds nothing; then its chip, lit, and New chat.
    let (mut c, mut t) = (Chats::default(), Talk::default());
    assert!(c.row(411, &t.0, &t.1).is_empty());
    t = talk("turn on the grain", "The grain is on.");
    c.name(&t.0[0].prompt.clone());
    assert_eq!(shown(&c, 0, &t).0, [on(0, "turn on the grai\u{2026}"), quiet(NEW, "New chat")]);
    // Eight, long named: one row within the card however narrow, the current one first, those
    // that do not fit behind "N more" (on a small phone with Compact, behind the lit chip
    // alone); Compact once the memory is past LEAN.
    let (mut c, mut t) = eight(&|i| format!("turn the grain on and off, {i}"));
    let more = |n: usize| (MORE, Variant::Chip, format!("{n} more"));
    for w in [340, 360, 411, 560, 1200] {
        for long in [false, true] {
            t.1[0].1 = if long { "x".repeat(LEAN) } else { "Done.".into() };
            let (b, width, list) = shown(&c, w, &t);
            assert!(width <= u32::from(w) - 40 && list.is_empty(), "{w}: {b:?}");
            assert_eq!((b[0].1, b[b.len() - 1].0), (Variant::On, if long { COMPACT } else { NEW }));
            let chips = b.iter().filter(|b| b.0 >= CHAT).count();
            let hidden = b.iter().any(|b| *b == more(8 - chips));
            assert!(chips == 8 || hidden || (long && w < 400), "{w}: {b:?}");
        }
    }
    t.1[0].1 = "Done.".into();
    assert_eq!(
        shown(&c, 411, &t).0,
        [on(7, "turn the g\u{2026}"), more(7), quiet(NEW, "New chat")]
    );
    let chip = (CHAT + 6, Variant::Chip, "turn the grain o\u{2026}".into());
    let desk = [on(7, "turn the grain o\u{2026}"), chip, more(6), quiet(NEW, "New chat")];
    assert_eq!(shown(&c, 560, &t).0, desk);
    // "7 more" shows them all over the row, the current one last, nearest it, with Delete chat.
    assert_eq!(click(&mut c, MORE, &mut t), Some(Clicked::Shown));
    let (b, _, list) = shown(&c, 411, &t);
    assert_eq!(b, [on(7, "turn the g\u{2026}"), quiet(DELETE, "Delete chat")]);
    let item = |n: u32| Node::Item {
        id: ITEM + n,
        text: format!("turn the grain on and off, {n}"),
        detail: "1 task".into(),
        selected: n == 7,
    };
    assert_eq!(list, (0..8).map(item).collect::<Vec<_>>());
    // Asked again; anything else between asks again.
    assert_eq!(click(&mut c, DELETE, &mut t), Some(Clicked::Shown));
    assert_eq!(shown(&c, 411, &t).0[1], (DELETE, Variant::Danger, "Delete for good".into()));
    assert_eq!(click(&mut c, 3, &mut t), None);
    assert_eq!(shown(&c, 411, &t).0[1], quiet(DELETE, "Delete chat"));
    click(&mut c, DELETE, &mut t);
    // Deleted: the one left last is current, the list hidden.
    assert_eq!(click(&mut c, DELETE, &mut t), Some(Clicked::Switched));
    assert_eq!((c.list.len(), c.listing), (7, false));
    assert_eq!(t, talk("turn the grain on and off, 6", "Done."));
    // An item switches, and hides the list.
    click(&mut c, MORE, &mut t);
    assert_eq!(click(&mut c, ITEM + 2, &mut t), Some(Clicked::Switched));
    assert_eq!((c.listing, &t.0[0].prompt[..]), (false, "turn the grain on and off, 2"));
    // The last one deleted leaves an empty one.
    let (mut c, mut t) = (Chats::default(), talk("a", "b"));
    for id in [CHAT, DELETE, DELETE] {
        click(&mut c, id, &mut t);
    }
    assert!(
        t == Talk::default() && c.list.len() == 1 && c.row(411, &[], &Memory::new()).is_empty()
    );
}

#[test]
fn chats_are_kept_and_read_back_defensively() {
    // A note, escapes and styles in the current one, another's turns: as they were.
    let (mut c, mut t) = eight(&|i| format!("task {i}"));
    let three = CHAT + c.list[4].n;
    click(&mut c, three, &mut t);
    t.1.insert(0, (String::new(), "a note:\n\\ \t \r".into()));
    t.0[0].lines.extend([(Style::Error, "E0903 busy".into()), (Style::Small, "1 step".into())]);
    let kept = c.encode(&t.0, &t.1);
    let (b, turns, memory) = Chats::load(&kept, (16, 4)).unwrap();
    assert_eq!((b.encode(&turns, &memory), turns, memory), (kept, t.0, t.1));
    assert_eq!((names(&b)[0].as_str(), b.next), ("task 3", 8));
    assert_eq!(names(&b), names(&c));
    // A chat's newest turns within 6 KiB, each its last 16 lines, clipped.
    let lines = vec![(Style::Body, "y".repeat(400)); 20];
    let big: Vec<_> =
        (0..16).map(|i| Turn { prompt: format!("p{i}"), lines: lines.clone() }).collect();
    let (_, kept, _) =
        Chats::load(&Chats::default().encode(&big, &Memory::new()), (16, 4)).unwrap();
    assert_eq!((kept.len(), &kept[0].prompt[..], kept[0].lines.len()), (1, "p15", 16));
    assert_eq!(kept[0].lines[15].1.len(), LINE);
    // A newest turn too big alone (its escapes) keeps its last lines that fit, and the older
    // turns that fit after it stay.
    let heavy = vec![(Style::Body, "a\n".repeat(150)); 16];
    let two = [
        Turn { prompt: "earlier".into(), lines: Vec::new() },
        Turn { prompt: "latest".into(), lines: heavy },
    ];
    let (_, kept, _) =
        Chats::load(&Chats::default().encode(&two, &Memory::new()), (16, 4)).unwrap();
    assert_eq!((kept.len(), &kept[1].prompt[..], kept[1].lines.len()), (2, "latest", 13));
    // Texts past their bounds (a file not of its making) come back clipped as it keeps them.
    let long = format!("compusophy chats 1\nc {0}\nm {0}\t{0}\nt {0}\nl 0 {0}\n", "m".repeat(5000));
    let (c, t, m) = Chats::load(&long, (2, 4)).unwrap();
    let sizes = [c.list[0].name.len(), m[0].0.len(), m[0].1.len(), t[0].prompt.len()];
    assert_eq!((sizes, t[0].lines[0].1.len()), ([NAME, TASK, TASK, LINE], LINE));
    // Escapes come back; an unknown line, a style or a memory line without its tab, turns past
    // the most (with their lines) and a ninth chat are skipped.
    let mut odd =
        "compusophy chats 1\nc a\\\\b\\nc\nm p\\tq\tok\nm no tab\nt x\nl 99 no\nl 4 yes\nzz what\nl\n"
            .to_string();
    (0..9).for_each(|i| odd += &format!("c n{i}\nt one\nt two\nl 0 kept\nt three\nl 0 not\n"));
    let (c, t, m) = Chats::load(&odd, (2, 4)).unwrap();
    assert_eq!(
        (&c.list[0].name[..], &t[0].lines[..], &m[0].0[..]),
        ("a\\b\nc", &[(Style::Small, "yes".into())][..], "p\tq")
    );
    let n: String = (0..7).map(|i| format!("c n{i}\nt one\nt two\nl 0 kept\n")).collect();
    let want = ["compusophy chats 1\nc a\\\\b\\nc\nm p\\tq\tok\nt x\nl 4 yes\n", &n].concat();
    assert_eq!(c.encode(&t, &m), want);
    // Not the file, no chat in it, or past its most bytes: nothing.
    let long = ["compusophy chats 1\nc a\n", &" ".repeat(MAX_FILE)].concat();
    for text in ["compusophy chats 2\nc lost\n", "compusophy chats 1\n", long.as_str()] {
        assert!(Chats::load(text, (2, 4)).is_none());
    }
    assert_eq!(Chats::default().encode(&[], &Memory::new()), "compusophy chats 1\nc \n");
}

#[test]
fn a_note_stays_first_until_the_next_and_a_task_like_it_is_none() {
    let task = |i: u32| (format!("step {i}"), "Ok.".to_string());
    let mut m: Memory = vec![(String::new(), "note".into())];
    for i in 0..5 {
        m.push(task(i));
        forget(&mut m, 4);
    }
    assert_eq!(m, [(String::new(), "note".into()), task(2), task(3), task(4)]);
    // A task whose prompt reads as a note's is a task: it goes in its turn.
    let mut m: Memory = vec![("Sum up our conversation so far.".into(), "x".into())];
    for i in 0..4 {
        m.push(task(i));
        forget(&mut m, 4);
    }
    assert_eq!(m, (0..4).map(task).collect::<Vec<_>>());
    // Worth compacting past LEAN bytes, more than a note takes.
    assert!(!compactable(&m) && compactable(&vec![("a".into(), "x".repeat(LEAN))]));
    // A task is remembered as the file keeps it, each text within TASK bytes, so the memory never
    // outgrows a request: an ellipsis where cut.
    remember(&mut m, &"p".repeat(16 << 10), "Ok.", 4);
    let (prompt, answer) = &m[3];
    assert!(
        prompt.len() == TASK && prompt.ends_with("p\u{2026}") && answer == "Ok." && m.len() == 4
    );
}
