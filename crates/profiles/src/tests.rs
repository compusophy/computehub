use super::*;

/// `localStorage` holding `kv`.
fn store<'a>(kv: &'a [(&str, &str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |k| kv.iter().find(|p| p.0 == k).map(|p| p.1.to_string())
}

/// Guest, and ana (id 2, one dot).
const TWO: &str = "CSPR 1 3\n0 00000000 - guest\n2 00000001 - ana";

#[test]
fn profiles_keep_their_own_keys_and_the_first_keeps_todays() {
    // Profile 0's keys are the ones from before profiles: nothing was moved.
    let today = "compusophy.home compusophy.home.bad compusophy.home.mark compusophy.theme \
        compusophy.dock compusophy.home.order compusophy.grain compusophy.ai.model \
        compusophy.reports compusophy.outbox";
    assert_eq!(PER_PROFILE.map(|k| key(0, k)).join(" "), today);
    assert_eq!(
        [key(7, "home"), key(1000, "theme")],
        ["compusophy.7.home", "compusophy.1000.theme"]
    );
    // Every profile's keys (ids to 1,000) and the device's are apart.
    let mut all: Vec<String> = (0..=1000).flat_map(|id| PER_PROFILE.map(|k| key(id, k))).collect();
    all.extend([LIST, LIST_BAD, LAST, "compusophy.seen", "compusophy.session"].map(String::from));
    let n = all.len();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), n);
    // Before a sign-in, profile 0's; then the signed-in one's, by name or by a first key, with
    // its face. A panic stays unreported if its reports are off (before: any listed one's).
    let off = [(LIST, TWO), ("compusophy.2.reports", "off")];
    assert_eq!(
        (active(), own("compusophy.theme"), quiet(&store(&off)), face()),
        (None, key(0, "theme"), true, 0)
    );
    sign(2, Some(TWO));
    assert_eq!((own("compusophy.outbox"), own("ai.model")), (key(2, "outbox"), key(2, "ai.model")));
    assert!(quiet(&store(&off)) && !quiet(&store(&[])) && face() == 1);
    // Its files go unkept once another tab removed it; the first is listed with no list.
    assert!(listed(Some(TWO)) && !listed(None) && !listed(Some("CSPR 1 3\n0 00000000 - guest")));
    sign(0, None);
    assert!(listed(None) && !quiet(&store(&off)) && face() == 0);
}

#[test]
fn the_list_reads_back_what_it_wrote_and_codes_what_it_cannot() {
    let implied = Profiles::implied();
    assert_eq!((implied.list[0].face, implied.list[0].name.as_str()), (0, "guest"));
    assert_eq!(Profiles::read(None), (implied.clone(), None));
    assert_eq!(implied.format(), "CSPR 1 1\n0 00000000 - guest");
    // Up to eight, names with spaces or past ASCII, each reads back; ids are never reused. Each
    // new one takes the fewest dots no other has: one, two, three...
    let mut p = implied;
    let names = ["kai", "Ana María", "\u{674e}", "x y z", "a-b", "Zo\u{eb}", "seven"];
    for (i, name) in names.into_iter().enumerate() {
        let id = p.apply(Op::Add { name: name.into(), pin: None }, &|_| false);
        assert_eq!((id, p.list[i + 1].face), (Ok(i as u32 + 1), i as u8 + 1));
        assert_eq!(Profiles::read(Some(&p.format())), (p.clone(), None));
    }
    let add = |name: &str| Op::Add { name: name.into(), pin: None };
    assert_eq!(p.apply(add("nine"), &|_| false), Err(FULL));
    assert_eq!(p.apply(Op::Remove(3), &|_| false), Ok(3));
    assert_eq!(p.apply(add("KAI"), &|_| false), Err(TAKEN));
    // A new one skips ids whose keys are still there: it inherits no one's files. It takes the
    // dots the removed one left.
    assert_eq!(p.apply(add("eight"), &|id| id == 8), Ok(9));
    assert_eq!(p.get(9).map(|p| p.face), Some(3));
    assert_eq!(p.apply(Op::Rename(9, "Kai".into()), &|_| false), Err(TAKEN));
    assert_eq!(p.apply(Op::Rename(9, "EIGHT".into()), &|_| false), Ok(9));
    assert_eq!(p.apply(Op::Face(9, 9), &|_| false), Ok(9));
    assert!(p.format().ends_with("\n9 00000009 - EIGHT"));
    assert_eq!(p.apply(Op::Remove(3), &|_| false), Err(GONE));
    assert_eq!(p.apply(Op::Face(3, 1), &|_| false), Err(GONE));
    let mut one = Profiles::implied();
    assert_eq!(one.apply(Op::Remove(0), &|_| false), Err(LAST_ONE));
    // A face from before faces were dots shows its id's, as many as there can be.
    let old = "CSPR 1 12\n0 be5cdbf3 - guest\n2 0c55aa31 - ana\n11 00000004 - bo\n10 ffffffff - cy";
    let faces: Vec<u8> = Profiles::read(Some(old)).0.list.iter().map(|p| p.face).collect();
    assert_eq!(faces, [0, 2, 4, FACES - 1]);
    // Damaged lists offer profile 0 alone; a newer one reads as far as it can, read-only.
    let guest = "0 be5cdbf3 - guest";
    #[rustfmt::skip]
    let bad = ["", "CSPR", "CSPX 1 1\n0 be5cdbf3 - guest", "CSPR 1 1\n0 5be1c0dz - guest",
        "CSPR 1 1\n1 be5cdbf3 - guest", "CSPR 1 2\n0 be5cdbf3 - guest\n1 be5cdbf3 - GUEST",
        "CSPR 1 1\n0 be5cdbf3 - ", "CSPR 1 1\n0 be5cdbf3 -  guest", "CSPR 1 1", "CSPR 0 1\n0",
        "CSPR 1 1\n0 be5cdbf3\t- guest"];
    for s in bad {
        assert_eq!(Profiles::read(Some(s)), (Profiles::implied(), Some(DAMAGED)), "{s:?}");
    }
    let nine: String = (0..9).map(|i| format!("\n{i} 0000000{i} - p{i}")).collect();
    assert_eq!(Profiles::read(Some(&format!("CSPR 1 9{nine}"))).1, Some(DAMAGED));
    let newer = Profiles::read(Some(&format!("CSPR 2 5\n{guest}\n4 00000001 - kai")));
    assert_eq!((newer.0.list.len(), newer.1), (2, Some(NEWER)));
    // Names: control chars go, the ends are trimmed, 24 chars at most, never none.
    let long = "abcdefghijklmnopqrstuvwxyz";
    assert_eq!(clean("  k\u{7}ai \n"), Ok("kai".into()));
    assert_eq!(clean(long), Ok(long[..24].into()));
    assert_eq!(clean(" \t "), Err(NO_NAME));
}

#[test]
fn settings_sets_the_signed_in_profiles_face() {
    // Nobody signed in: nothing to set.
    assert_eq!(set_face(Some(TWO), "4"), None);
    sign(2, Some(TWO));
    let set = set_face(Some(TWO), "4");
    assert_eq!(set.as_deref(), Some("CSPR 1 3\n0 00000000 - guest\n2 00000004 - ana"));
    assert_eq!(face(), 4);
    // Never a face past the last or not a count, nor in a damaged or newer list, nor once it is
    // gone.
    let newer = "CSPR 2 3\n0 00000000 - guest\n2 00000001 - ana";
    let damaged = "CSPR 1 1\n0 x - guest";
    for (stored, face) in [(TWO, "10"), (TWO, "x"), (damaged, "1"), (newer, "1")] {
        assert_eq!(set_face(Some(stored), face), None, "{stored:?} {face}");
    }
    assert_eq!((set_face(None, "1"), face()), (None, 4));
    // Guest's, with no list yet, makes it.
    sign(0, None);
    assert_eq!(set_face(None, "9").as_deref(), Some("CSPR 1 1\n0 00000009 - guest"));
    assert_eq!(face(), 9);
}

#[test]
fn a_pins_record_reads_back_and_only_a_well_formed_one() {
    let pin = Pin { len: 4, iterations: 100_000, salt: [7; 16], hash: [0xab; 32] };
    let rec = pin.format();
    assert_eq!((&rec[..18], Pin::read(&rec)), ("p1:4:100000:070707", Some(pin)));
    let hex = |n: usize| "ab".repeat(n);
    let (salt, hash) = (hex(16), hex(32));
    #[rustfmt::skip]
    let bad = [format!("p1:3:9:{salt}:{hash}"), format!("p1:9:9:{salt}:{hash}"),
        format!("p2:4:9:{salt}:{hash}"), format!("p1:4:0:{salt}:{hash}"),
        format!("p1:4:9:{}:{hash}", hex(15)), format!("p1:4:9:{salt}:{}", hash.to_uppercase()),
        format!("p1:4:9:{salt}:{hash}:"), String::from("-x")];
    assert!(bad.iter().all(|b| Pin::read(b).is_none()));
    // A list keeps a PIN's record and reads it back; a bad one damages it.
    let rec = Pin { len: 4, iterations: 9, salt: [7; 16], hash: [1; 32] }.format();
    let list = format!("CSPR 1 2\n0 00000000 - guest\n1 00000001 {rec} kai");
    assert_eq!(Profiles::read(Some(&list)).0.format(), list);
    let broken = list.replace("p1:4:9", "p1:4:x");
    assert_eq!(Profiles::read(Some(&broken)).1, Some(DAMAGED));
}
