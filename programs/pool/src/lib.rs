//! The pool, compusophyOS's mesh program: tabs linked tab to tab share one job, every core of
//! every device on it. A tab is a node: nothing to install. Two tabs pair once by a short code
//! ([`hub`]), then talk over a WebRTC data channel the page makes, in [`msg`]s that could cross
//! any network. The [`pool`] spreads a job's chunks over the workers of every linked tab:
//! whoever is idle takes the next, so faster cores take more, and an answer comes back with its
//! fuel and the SHA-256 of its result, checked by replaying some of them. The desktop (the
//! `mesh` crate) does for it what only a page can, in [`uiwire::relay`] frames on its console.

#![forbid(unsafe_code)]

pub mod hub;
pub mod msg;
pub mod pool;
#[cfg(test)]
mod tests;

pub use hub::Hub;
