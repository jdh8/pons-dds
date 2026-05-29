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
//! This release ships the [`Solver`] API: a per-strain solver that
//! produces one strain's row of a [`TrickCountTable`] for a [`FullDeal`],
//! plus rayon-parallel [`solve_deal`] (single-deal) and [`solve_deals`]
//! (batch) helpers that assemble the full 5 × 4 table, and the sequential
//! single-thread [`solve_deal_on`] for deterministic profiling. The
//! internal substrate (position state, move generator, search engine,
//! transposition table, and friends) remains crate-private.
//!
//! # Algorithm reference
//!
//! For each ported module, the corresponding vendor C++ file is named
//! in the module docs. The canonical reference is
//! [`ddss-sys/vendor/src/ABsearch.cpp`](../../../ddss-sys/vendor/src/ABsearch.cpp)
//! for the search, plus the supporting files documented per-module.

// The crate is a line-by-line port of a C++ codebase: it mixes `i32`,
// `usize`, `u8`, and `u16` throughout its indexing and bit-twiddling
// math, uses bridge-shorthand bindings (lho/rho, lh/rh), keeps long
// translated functions and large fixed-size lookup tables, and mirrors
// vendor instance methods even when they do not touch `self`. Fighting
// these lints would obscure the correspondence with the vendor source
// without buying real safety — invariants are guaranteed by the
// surrounding bridge-specific context.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::large_stack_arrays,
    clippy::large_stack_frames,
    clippy::needless_pass_by_ref_mut,
    clippy::similar_names,
    clippy::too_many_lines,
    clippy::unused_self
)]

pub(crate) mod convert;
pub(crate) mod later_tricks;
pub(crate) mod lookup;
pub(crate) mod move_type;
pub(crate) mod moves;
pub(crate) mod pos;
pub(crate) mod quick_tricks;
pub(crate) mod search;
pub mod solver;
pub(crate) mod tt;

pub use contract_bridge::FullDeal;
pub use search::SearchStats;
pub use solver::{Solver, TrickCountTable, solve_deal, solve_deal_on, solve_deals};
