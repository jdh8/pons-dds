# Changelog

All notable changes to this project will be documented in this file.

The format is based on Keep a Changelog.

## [Unreleased]

### Changed

- Web UI: factored the outlined "Edit →" / Copy PBN / Clear button styling into
  a reusable `button.secondary` class (parallel to `button.primary`). No visual
  change.

## [0.2.0] - 2026-07-08

### Added

API parity with the FFI `ddss` reference crate — every solving feature it
exposes now has a pure-Rust equivalent, oracle-tested against DDS 2.9:

- **`solve_board` / `solve_boards`** (+ `solve_boards_with_memory`,
  `Solver::solve_board`): per-card double-dummy solving from arbitrary,
  possibly mid-trick positions. New types `Board`, `CurrentTrick`,
  `Target` (any/all/legal-plays), `Objective`, `Play` (with
  sequence-equals), and `FoundPlays`. Scores, cards, equals masks, and
  output order match `SolveBoard` bit-for-bit (mode 1; see divergences).
- **`analyse_play` / `analyse_plays`** (+ `Solver::analyse_play`,
  `Solver::try_analyse_play`): double-dummy trick counts before and after
  each card of a play trace, with new types `PlayTrace`, `PlayAnalysis`,
  and `PlayFaultError`. The batch entry fans traces across the solver
  pool (the FFI reference analyses serially).
- **`calculate_par` / `calculate_pars`**: par scores and contracts from a
  DD table, vulnerability, and (for the dealer-relative variant) dealer.
  New types `Par`, `ParContract`, and `Vulnerability`; statement-for-
  statement ports of the vendor's `DealerParBin`/`SidesParBin` paths,
  text-parse quirks included, matching `ddss` byte-for-byte.
- **Strain-filtered batch solving**: `solve_deals` /
  `solve_deals_with_memory` take `NonEmptyStrainFlags` (new, with
  `StrainFlags`) restricting which strains are solved; rows of
  unrequested strains are zero-filled, matching the FFI crate's
  observable behavior (now documented).
- Hex and GIB hand-record views on the trick-count types.

### Changed

- **Breaking:** `TrickCountTable` is now the validated newtype stack from
  the `ddss` crate — `TrickCount` (0..=13), bit-packed `TrickCountRow`,
  and `TrickCountTable` indexed by `Strain` with per-seat access by
  `Seat` — replacing the plain `pub tricks: [[u8; 4]; 5]` struct.
  `Solver::solve` returns a `TrickCountRow` instead of `[u8; 4]`.
- **Breaking:** `solve_deals` and `solve_deals_with_memory` take a
  `NonEmptyStrainFlags` argument; pass `NonEmptyStrainFlags::ALL` for the
  previous behavior.

### Divergences from the FFI `ddss` crate

All deliberate, all on the safe side, all oracle-tested around:

- `solve_board` implements DDS **mode-1** semantics: a forced single card
  gets a real score, never the unevaluated `-2` sentinel that `ddss`'s
  own decoder cannot represent (its `solve_board` panics on forced-card
  boards).
- `analyse_play` returns the documented full `cards.len() + 1` trick
  counts; DDS never analyses the final trick and mis-counts entries on
  mid-trick snapshots (to the point that `ddss` can panic on short
  traces there).
- `analyse_play` detects revokes as errors; DDS silently scores the
  off-suit card as a discard and produces a wrong analysis.
- Empty boards and targets above the remaining tricks yield empty
  results instead of error-code panics.
- `Target::Any(Some(0))`/`All(Some(0))` list moves in a deterministic
  fresh-weight order; DDS's order depends on stale best-move state from
  earlier solves on its thread.
- The `nodes` counter of `FoundPlays` is approximate (probe schedules
  differ).
- `system_info` and `Solver::lock`/`try_lock` have no equivalent — the
  pure-Rust solver needs no global lock; construct `Solver`s freely.

### Performance

A solver-wide optimization pass closing most of the gap to the DDS 2.9 C++
reference (`ddss`). On an 8c/16t Ryzen 7 8700F, same-run criterion
head-to-head: `solve_deal` 91.7 → 78.8 ms (C++ ratio 1.25× → 1.12×),
`solve_deals/32` 24.4 → 20.0 ms/deal (1.46× → 1.21×), `solve_deals/200`
25.4 → 22.8 ms/deal (1.43× → 1.29×). Results are bit-for-bit unchanged
(verified against `ddss` over a 10 000-deal soak).

- The per-deal tables (the 8192-entry `rel` table and the transposition
  table's aggregator) are now built once per deal instead of once per
  declarer, and reused across strains via a deal fingerprint on `Solver` —
  they depend only on which hand holds which card. 20 rebuilds per deal
  become 1, mirroring the vendor's `SolveSameBoard` setup reuse.
- `search_target` replaces its midpoint bisection over the trick target
  with the vendor's hint-anchored ±1 stepping walk (`SolveSameBoard` /
  `CalcSingleCommon`): the partner of a solved declarer is seeded with that
  score, an opponent with its complement. Probes per declarer drop from
  3.86 to 2.55, and successive probes land next to targets the warm
  transposition table has already seen.
- The six 8192-entry lookup tables (`HIGHEST_RANK`, `LOWEST_RANK`,
  `COUNT_TABLE`, `REL_RANK`, `WIN_RANKS`, `GROUP_DATA`) and the two
  transposition-table constants (`TT_LOWEST_RANK`, `MASK_BYTES`) are built
  at compile time by `const fn`s instead of `LazyLock`s — a hot-path read
  is now a direct `.rodata` load with no atomic init-check or pointer
  indirection. This was the single largest win (−23% on the parallel
  batch).
- `AbsRank` is packed to the vendor's 2-byte layout (`i8` rank/hand),
  shrinking the randomly-probed per-deal `rel` table from 3.93 MB to
  960 KiB — it now fits a Zen 4 core's L2 instead of 16 threads fighting
  over L3.
- The quick-tricks/later-tricks `abs_rank` helpers read the `rel` table
  (one load) instead of scanning 13 ranks × 4 hands, matching
  `QuickTricks.cpp`.
- Assorted hot-path trims: the `[u32; 4]` aggr widening for the
  transposition table is computed once per lead node; make/undo use plain
  arithmetic instead of saturating ops; the per-probe `Instant::now`
  timing in the driver is now gated behind the `profiling` feature (the
  `bisection_timing()` diagnostics read zero without it).
- The transposition table now **retains** its page pool across the
  per-strain `reset()` instead of freeing it down to a single page, so a
  worker re-uses the slabs it already allocated rather than re-`malloc`ing
  and zeroing fresh 6.2 MiB pages every strain (mirroring the vendor's
  `ResetMemory`). Over a 200-deal sequential corpus this cuts page
  allocations from ~4200 to ~50 (≈26 GB → ≈0.3 GB of `malloc`+memset
  traffic); the single-thread `bisection_stats` corpus runs ~3% faster,
  and a 1000-deal parallel batch — where the freed bandwidth is contended
  across all 16 threads — gains ~10% (p < 0.05), narrowing the same-run
  gap to `ddss` from 1.26× to 1.13× (`ddss` itself flat, p = 0.33).
  Bit-for-bit unchanged (10 000-deal soak).

## [0.1.2] - 2026-07-05

### Fixed

- The solver no longer aborts on `wasm32-unknown-unknown`: the bisection
  loop's always-on iteration timing called `std::time::Instant::now()`, which
  panics on wasm (no clock). The timing diagnostics
  (`SearchStats::iter1_nanos` / `later_nanos`) are now native-only and stay 0
  on wasm; native behavior is unchanged. Callers targeting wasm should drive
  the single-threaded paths (`Solver`, `solve_deal_on`) and raise the shadow
  stack (e.g. `-Clink-arg=-zstack-size=16777216`) — the deep search overflows
  wasm's 1 MiB default.

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
