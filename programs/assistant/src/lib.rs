//! The Assistant: the AI that uses the computer. A wasm32-wasip1 GUI program
//! (`dist/bin/assistant.wasm`) on the [`uiwire`] protocol, which the desktop runs as its overlay
//! over the windows ([`agent`]): it reads the screen ([`look`]), asks the model the desktop names
//! ([`uiwire::Event::Config`]) through compusophy's free AI, which needs no key
//! ([`uiwire::Request::Ai`], the body streamed back as [`uiwire::Event::AiData`] until
//! [`uiwire::Event::AiEnd`] and read by [`calls`]), and does what the model calls for as a person
//! would ([`uiwire::Request::Act`]). To make an app it opens Studio, as a person would. It reads
//! and writes the person's files itself (the [`files`] crate), and what no tool of its own can
//! do it tells compusophy, who builds the OS ([`uiwire::Request::Feedback`]).
//!
//! [`ai`] (the free AI's requests and failures) and [`json`] are the coding agent's
//! ([`coder`]), which Studio runs.

#![forbid(unsafe_code)]

pub mod agent;
pub mod calls;
pub mod look;

pub use coder::ai::DEFAULT_MODEL;
pub use coder::{ai, json};
