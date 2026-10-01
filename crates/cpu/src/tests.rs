use crate::{js::span, link::trim, link::wasi, sys::*};
use wasi::{ARITY, FD_WRITE};

#[test]
fn the_exports_pad_their_calls_and_link_by_wasi_name_and_trap_text_is_trimmed() {
    // Each pads its call to `crate::sys` to nine; before Start every call is ENOSYS.
    assert_eq!(ARITY.iter().max(), Some(&9));
    let calls = [fd_write(1, 8, 1, 16), path_open(3, 0, 64, 5, 1, 9, 0, 0, 72), sched_yield()];
    assert_eq!(calls, [kernel::wire::ENOSYS.into(); 3]);
    let p1 = "wasi_snapshot_preview1";
    assert_eq!(wasi(p1, "fd_write"), Some(FD_WRITE));
    let others = [(p1, "fd_write2"), (p1, "__wbindgen_malloc"), ("wasi_unstable", "fd_write")];
    assert!(others.iter().all(|(m, n)| wasi(m, n).is_none()));
    // Engine messages lose what JavaScriptCore appends.
    let t = |s: &str| trim(s.into());
    assert_eq!(t("x is not a function (evaluating 'x()')"), "x is not a function");
    assert_eq!([t("ok"), t(" (evaluating '"), t("\u{e9} (evaluating '")], ["ok", "", "\u{e9}"]);
}

#[test]
fn ring_spans_wrap_and_stop_when_full() {
    let r = kernel::wire::RING_BYTES;
    assert_eq!((span(0, 0, 10), span(0, 0, 70_000)), ((0, 10, 10), (0, 65_536, 65_536)));
    assert_eq!((span(r, 0, 1), span(0, 1, 9)), ((0, 0, 0), (0, 0, 0)));
    assert_eq!((span(r - 6, r - 6, 10), span(r + 3, 5, 100)), ((r - 6, 6, 10), (3, 2, 2)));
    // HEAD and TAIL wrap at 2^32, a multiple of the ring.
    assert_eq!(span(u32::MAX - 1, u32::MAX - 101, 300), (r - 2, 2, 300));
    assert_eq!(span(5, u32::MAX - 10, 9), (5, 9, 9));
}
