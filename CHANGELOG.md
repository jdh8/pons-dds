# Changelog

All notable changes to this project will be documented in this file.

The format is based on Keep a Changelog.

## [Unreleased]

## [0.1.1] - 2026-05-31

### Added

- `solve_deals_with_memory`: a parallel batch solve taking an explicit
  per-thread transposition-table budget (`default_mb` / `max_mb`), for capping
  per-worker memory in highly parallel runs or sweeping the budget when tuning.
- `examples/par_balance`: reports parallel load balance (the makespan "tail
  ratio") and the per-strain solve-time distribution, to guide task dispatch
  tuning on a given machine.

### Changed

- Batch solving (`solve_deals` / `solve_deals_with_memory`) now runs on a
  dedicated, persistent thread pool with large worker stacks, replacing the plain
  Rayon parallel iterator over the global pool. Work is split into a bounded
  number of work-stealing chunks — bounding the chunk count caps Rayon's
  split-recursion depth, so the deep search stays off a deep stack regardless of
  batch size — and dispatched tail-risky-first (notrump leads, since with no
  trump to force a claimable ending its worst-case searches blow up hardest) to
  trim the makespan tail, most visibly on small batches. Results are unchanged.
- `examples/tt_sweep` now sweeps the transposition-table budget warm and across
  the whole thread pool rather than single-threaded, so it reflects the
  per-thread vs shared-cache trade-off of real parallel solving.

### Fixed

- Stack overflow in parallel batch solving. The deep alpha-beta search ran on
  Rayon's global workers and the calling thread, whose ~2 MiB default stacks it
  could overflow on larger batches — and would overflow readily on Windows'
  1 MiB default. The search now runs only on the solver pool's large-stack
  workers, so `solve_deals` is safe to call from any thread regardless of its
  stack size (regression test `solve_deals_safe_on_small_stack`).

### Documentation

- Add an Acknowledgements section to the README crediting the ported lineage:
  [DDS](https://github.com/dds-bridge/dds) (Bo Haglund and Soren Hein),
  [Robert Salita's ddss fork](https://github.com/bsalita/ddss) that supplies the
  vendored DDS 2.9.0 sources, and the [`ddss`](https://github.com/jdh8/ddss) /
  `ddss-sys` FFI crates.

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
