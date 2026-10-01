//! computehub-node: serves real shells on this machine to compusophyOS
//! terminals over an authenticated WebSocket on loopback.
//!
//! [`proto`] is the wire format, [`auth`] the token and the Origin allowlist,
//! and [`session`] the sockets, PTYs and admission limits. The binary
//! (`main.rs`) is only the command line. See README.md for the security
//! model and the pairing flow.

#![forbid(unsafe_code)]

pub mod auth;
pub mod proto;
pub mod session;
