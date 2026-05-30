# Changelog

All notable changes to this project will be documented in this file.

The format is based on Keep a Changelog.

## [Unreleased]

## [0.1.0] - 2026-05-30

Initial release of pons-dds, a pure-Rust double dummy solver for contract
bridge. The engine — alpha-beta search with a transposition table and heuristic
move ordering — needs no C++ compiler or FFI and compiles anywhere Rust runs.

### Added

- Solving API: `solve_deal` (one deal, strains fanned across Rayon workers) and
  `solve_deals` (parallel batch over many deals).
- Reusable `Solver` plus `solve_deal_on` for sequential and diagnostic use.
- `TrickCountTable` result type indexed by `(Strain, Seat)`; `FullDeal` is
  re-exported from `contract-bridge`.
- Optional `profiling` feature exposing per-node search instrumentation (TT hit
  rate, move-ordering cutoffs, node-0 funnel) with zero overhead when disabled.
- Criterion benchmark suite in `benches/solver.rs` covering `solve_deal` and
  `solve_deals/{32,200}`, with history published continuously from `main` to the
  GitHub Pages dashboard (`dev/bench`) via `github-action-benchmark` for
  trend/regression tracking.
- README badges for CI, Crates.io, Docs.rs, and published benchmarks.
