//! The 46 `wasi_snapshot_preview1` imports as cpu.wasm's own exports, with WASI's exact core
//! signatures in witx order (u32 is i32, u64 is i64, the errno an i32): wasm-bindgen keeps
//! primitive-only exports at that raw ABI, so the guest calls them wasm to wasm. No docs or
//! JSDoc on the exports: they would ship in cpu.js.

#![allow(clippy::too_many_arguments)]

use wasi::*;
use wasm_bindgen::prelude::*;

// Each entry is `ID name(argument types);`: its arguments, named a, b, c, .., widen to u64 and
// the names left over pad its call to `crate::sys` (nine arguments) with zeros.
macro_rules! sys {
    (0 $unused:ident) => { 0 };
    ($($id:ident $name:ident($($t:ident)*);)+) => {
        /// `(id, name, arity)` of each export but proc_exit.
        #[cfg(test)]
        pub const EXPORTS: [(usize, &str, usize); 45] =
            [$(($id, stringify!($name), <[&str]>::len(&[$(stringify!($t)),*]))),+];
        $(sys!(@ $id $name [$($t)*] [a b c d e f g h i] []);)+
    };
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

sys! {
    ARGS_GET args_get(u32 u32); ARGS_SIZES_GET args_sizes_get(u32 u32);
    ENVIRON_GET environ_get(u32 u32); ENVIRON_SIZES_GET environ_sizes_get(u32 u32);
    CLOCK_RES_GET clock_res_get(u32 u32); CLOCK_TIME_GET clock_time_get(u32 u64 u32);
    FD_ADVISE fd_advise(u32 u64 u64 u32); FD_ALLOCATE fd_allocate(u32 u64 u64);
    FD_CLOSE fd_close(u32); FD_DATASYNC fd_datasync(u32); FD_FDSTAT_GET fd_fdstat_get(u32 u32);
    FD_FDSTAT_SET_FLAGS fd_fdstat_set_flags(u32 u32);
    FD_FDSTAT_SET_RIGHTS fd_fdstat_set_rights(u32 u64 u64);
    FD_FILESTAT_GET fd_filestat_get(u32 u32); FD_FILESTAT_SET_SIZE fd_filestat_set_size(u32 u64);
    FD_FILESTAT_SET_TIMES fd_filestat_set_times(u32 u64 u64 u32);
    FD_PREAD fd_pread(u32 u32 u32 u64 u32); FD_PRESTAT_GET fd_prestat_get(u32 u32);
    FD_PRESTAT_DIR_NAME fd_prestat_dir_name(u32 u32 u32); FD_PWRITE fd_pwrite(u32 u32 u32 u64 u32);
    FD_READ fd_read(u32 u32 u32 u32); FD_READDIR fd_readdir(u32 u32 u32 u64 u32);
    FD_RENUMBER fd_renumber(u32 u32); FD_SEEK fd_seek(u32 u64 u32 u32); FD_SYNC fd_sync(u32);
    FD_TELL fd_tell(u32 u32); FD_WRITE fd_write(u32 u32 u32 u32);
    PATH_CREATE_DIRECTORY path_create_directory(u32 u32 u32);
    PATH_FILESTAT_GET path_filestat_get(u32 u32 u32 u32 u32);
    PATH_FILESTAT_SET_TIMES path_filestat_set_times(u32 u32 u32 u32 u64 u64 u32);
    PATH_LINK path_link(u32 u32 u32 u32 u32 u32 u32);
    PATH_OPEN path_open(u32 u32 u32 u32 u32 u64 u64 u32 u32);
    PATH_READLINK path_readlink(u32 u32 u32 u32 u32 u32);
    PATH_REMOVE_DIRECTORY path_remove_directory(u32 u32 u32);
    PATH_RENAME path_rename(u32 u32 u32 u32 u32 u32);
    PATH_SYMLINK path_symlink(u32 u32 u32 u32 u32); PATH_UNLINK_FILE path_unlink_file(u32 u32 u32);
    POLL_ONEOFF poll_oneoff(u32 u32 u32 u32); PROC_RAISE proc_raise(u32); SCHED_YIELD sched_yield();
    RANDOM_GET random_get(u32 u32); SOCK_ACCEPT sock_accept(u32 u32 u32);
    SOCK_RECV sock_recv(u32 u32 u32 u32 u32 u32); SOCK_SEND sock_send(u32 u32 u32 u32 u32);
    SOCK_SHUTDOWN sock_shutdown(u32 u32);
}

// proc_exit returns nothing.
#[wasm_bindgen(skip_jsdoc)]
pub fn proc_exit(code: u32) {
    crate::sys(PROC_EXIT, code.into(), 0, 0, 0, 0, 0, 0, 0, 0);
}
