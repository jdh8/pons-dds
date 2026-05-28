//! Pure-Rust double-dummy solver for contract bridge.
//!
//! Reimplementation of the DDS algorithm (see the C++ vendor in
//! `ddss-sys/vendor/src/`) with the same alpha-beta + transposition
//! table + heuristic-ordered search at its core, but without the FFI
//! / `cc`-crate / sanitizer-pain that the existing
//! [`ddss`](https://crates.io/crates/ddss) and
//! [`dds-bridge`](https://crates.io/crates/dds-bridge) wrappers carry.
//!
//! # v0.1 scope
//!
//! This release ships the substrate only — internal types
//! ([`pos::Pos`], [`move_type::MoveType`]) and the precomputed lookup
//! tables in [`lookup`]. The public `Solver` API arrives in a later
//! milestone (Phase 4 of the port).
//!
//! # Algorithm reference
//!
//! For each ported module, the corresponding vendor C++ file is named
//! in the module docs. The canonical reference is
//! [`ddss-sys/vendor/src/ABsearch.cpp`](../../../ddss-sys/vendor/src/ABsearch.cpp)
//! for the search, plus the supporting files documented per-module.

pub(crate) mod later_tricks;
pub(crate) mod lookup;
pub(crate) mod move_type;
pub(crate) mod moves;
pub(crate) mod pos;
pub(crate) mod quick_tricks;
pub(crate) mod tt;
