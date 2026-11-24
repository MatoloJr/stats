//! Platform-independent core of Stats for Windows.
//!
//! Everything here is plain Rust with no Win32 calls, so it is unit-tested on
//! any machine with `cargo test --lib`. The Win32 side (`src/win/`) only
//! samples counters, draws, and calls into these modules for decisions.

pub mod advice;
pub mod cleanup;
pub mod config;
pub mod counters;
pub mod fit;
pub mod history;
pub mod junk;
pub mod numbers;
pub mod pagefile;
pub mod pressure;
