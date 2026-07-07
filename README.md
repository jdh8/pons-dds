# pons-dds

[![Crates.io](https://img.shields.io/crates/v/pons-dds.svg)](https://crates.io/crates/pons-dds)
[![Docs.rs](https://docs.rs/pons-dds/badge.svg)](https://docs.rs/pons-dds)
[![CI](https://github.com/jdh8/pons-dds/actions/workflows/rust.yml/badge.svg)](https://github.com/jdh8/pons-dds/actions/workflows/rust.yml)
[![Benchmarks](https://img.shields.io/badge/benchmarks-published-blue?logo=github)](https://jdh8.github.io/pons-dds/dev/bench/)

Pure-Rust double dummy solver for contract bridge.
No C++ compiler required — compiles anywhere Rust runs.

## Usage

```toml
[dependencies]
pons-dds = "0.2"
```

```rust
use contract_bridge::{FullDeal, Seat, Strain};
use pons_dds::{NonEmptyStrainFlags, solve_deal, solve_deals};

// Solve one deal — fans the 5 strains across rayon workers.
let deal: FullDeal = "N:AKQJT98765432... .AKQJT98765432.. \
                      ..AKQJT98765432. ...AKQJT98765432".parse().unwrap();
let table = solve_deal(deal);
assert_eq!(table[Strain::Spades].get(Seat::North).get(), 13);

// Solve many deals in parallel — preferred for batch workloads.
// The strain flags restrict which rows are computed.
let deals = [deal, deal];
let tables = solve_deals(&deals, NonEmptyStrainFlags::ALL);
assert_eq!(tables.len(), 2);
```

Beyond DD tables, the full DDS feature set is available:

```rust
use contract_bridge::{Seat, Strain};
use pons_dds::{
    Board, CurrentTrick, Objective, PlayTrace, Target,
    Vulnerability, analyse_play, calculate_par, solve_board, solve_deal,
};
# let deal: contract_bridge::FullDeal = "N:AKQJT98765432... .AKQJT98765432.. ..AKQJT98765432. ...AKQJT98765432".parse().unwrap();

// Per-card solving from any (even mid-trick) position.
let board = Board::try_new(deal.into(), CurrentTrick::new(Strain::Notrump, Seat::East))?;
let plays = solve_board(&Objective { board: board.clone(), target: Target::Legal });

// Double-dummy trick counts along a play trace.
let analysis = analyse_play(&PlayTrace { board, cards: Default::default() });

// Par score and contracts from a DD table.
let par = calculate_par(solve_deal(deal), Vulnerability::NONE, Seat::North);
assert_eq!(par.score, 1510); // NS bid and make 7♠ — nonvul grand slam
# Ok::<(), pons_dds::BoardError>(())
```

For sequential or diagnostic use, drive `Solver` directly:

```rust
use contract_bridge::Strain;
use pons_dds::{Solver, solve_deal_on};
# let deal: contract_bridge::FullDeal = "N:AKQJT98765432... .AKQJT98765432.. ..AKQJT98765432. ...AKQJT98765432".parse().unwrap();

let mut solver = Solver::new(Strain::Notrump);
let table = solve_deal_on(&mut solver, deal);
```

## Scope

Full API parity with the FFI [`ddss`][ddss-rs] reference crate, pure Rust:

| `ddss` (FFI)                        | pons-dds                                    |
|-------------------------------------|---------------------------------------------|
| `Solver::solve_deal`                | `solve_deal` / `solve_deal_on`              |
| `Solver::solve_deals(deals, flags)` | `solve_deals(deals, flags)`                 |
| `Solver::solve_board(s)`            | `solve_board` / `solve_boards`              |
| `Solver::analyse_play(s)`           | `analyse_play` / `analyse_plays`            |
| `calculate_par` / `calculate_pars`  | `calculate_par` / `calculate_pars`          |
| `Solver::lock` / `try_lock`         | not needed — construct `Solver`s freely     |
| `system_info`                       | not applicable (no FFI library underneath)  |

Every solver output is oracle-tested against DDS 2.9 (see
`tests/compare_ddss*.rs`); the few deliberate divergences — all on the safe
side, like scoring forced cards instead of DDS's `-2` sentinel and
detecting revokes in play traces — are listed in the CHANGELOG. The
internal substrate (position state, move generator, search engine,
transposition table, and friends) remains crate-private.

## Performance

Benchmarked with `cargo bench` (seed 0, 200 random deals, 16c/32t machine).

| Engine                            | Serial (1 thr)      | Parallel (32 thr)   |
|-----------------------------------|---------------------|---------------------|
| ddss 0.1 (DDS 2.9, C++ FFI)       | 131.5 ms/deal       | 9.9 ms/deal         |
| **pons-dds (DDS 2.9, pure Rust)** | **149.3 ms/deal**   | **13.2 ms/deal**    |
| dds-bridge 0.19 (DDS 3.0, C++ FFI) | 193.7 ms/deal     | — †                 |

† dds-bridge 0.19 ships single-threaded; 0.20+ adds a parallel path.

Head-to-head benchmarks against each C++ crate live in their respective
repositories: `ddss/benches/compare_pons_dds.rs` and
`dds-bridge/benches/compare_pons_dds.rs`. They are kept separate because
ddss-sys and dds-bridge-sys both vendor the DDS C++ symbols and cannot
link into the same binary.

## Acknowledgements

pons-dds is a line-by-line pure-Rust port of the [DDS][dds] double dummy
solver by Bo Haglund and Soren Hein — specifically the DDS 2.9.0 engine as
carried by [Robert Salita's `ddss` fork][ddss-c], whose C++ sources are
vendored under `ddss-sys/vendor/src/` and cited per-module throughout this
crate's source: each ported module names its corresponding vendor file in its
docs, with `ddss-sys/vendor/src/ABsearch.cpp` the canonical reference for the
search. The alpha-beta search, transposition table, and move-ordering
heuristics all follow that reference; only the language and memory-safety
scaffolding are new. The same C++ engine is reachable from Rust through the
[`ddss`][ddss-rs] / [`ddss-sys`](https://crates.io/crates/ddss-sys) FFI crates,
which pons-dds benchmarks against above. Like DDS and ddss, pons-dds is licensed
under Apache-2.0.

[dds]: https://github.com/dds-bridge/dds
[ddss-c]: https://github.com/bsalita/ddss
[ddss-rs]: https://github.com/jdh8/ddss

## License

[Apache-2.0](https://github.com/jdh8/pons-dds/blob/main/LICENSE)
