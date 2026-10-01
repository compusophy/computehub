//! The 46 `wasi_snapshot_preview1` imports as cpu.wasm's own exports, with WASI's exact core
//! signatures in witx order (u32 is i32, u64 is i64, the errno an i32): wasm-bindgen keeps
//! primitive-only exports at that raw ABI, so the guest calls them wasm to wasm. No docs or
//! JSDoc on the exports: they would ship in cpu.js.

#![allow(clippy::too_many_arguments)]

use wasi::*;
use wasm_bindgen::prelude::*;

// Each entry of [`wasi::functions`]: its arguments, named a, b, c, .., widen to u64 and the names
// left over pad its call to `crate::sys` (nine arguments) with zeros. proc_exit is below.
macro_rules! sys {
    (0 $unused:ident) => { 0 };
    ($($id:ident $name:ident($($t:ident)*);)+) => {
        $(sys!(@ $id $name [$($t)*] [a b c d e f g h i] []);)+
    };
    (@ PROC_EXIT $($skip:tt)*) => {};
    (@ $id:ident $name:ident [$t:ident $($ts:ident)*] [$a:ident $($as:ident)*] [$($done:tt)*]) => {
        sys!(@ $id $name [$($ts)*] [$($as)*] [$($done)* $a: $t,]);
    };
    (@ $id:ident $name:ident [] [$($as:ident)*] [$($a:ident: $t:ident,)*]) => {
        #[wasm_bindgen(skip_jsdoc)]
        pub fn $name($($a: $t),*) -> u32 {
            crate::sys($id, $(u64::from($a),)* $(sys!(0 $as)),*)
        }
    };
}

wasi::functions!(sys);

// proc_exit returns nothing.
#[wasm_bindgen(skip_jsdoc)]
pub fn proc_exit(code: u32) {
    crate::sys(PROC_EXIT, code.into(), 0, 0, 0, 0, 0, 0, 0, 0);
}
