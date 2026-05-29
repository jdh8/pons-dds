# dds-rs

[![CI](https://github.com/jdh8/dds-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/jdh8/dds-rs/actions/workflows/ci.yml)

Pure-Rust double dummy solver for contract bridge.
No C++ compiler required — compiles anywhere Rust runs.

## Usage

```toml
[dependencies]
dds-rs = "0.1"
```

```rust
use contract_bridge::{FullDeal, Seat, Strain};
use dds_rs::{solve_deal, solve_deals};

// Solve one deal — fans the 5 strains across rayon workers.
let deal: FullDeal = "N:AKQJT98765432... .AKQJT98765432.. \
                      ..AKQJT98765432. ...AKQJT98765432".parse()?;
let table = solve_deal(deal);
assert_eq!(table.get(Strain::Spades, Seat::North), 13);

// Solve many deals in parallel — preferred for batch workloads.
let tables = solve_deals(&deals);
```

For sequential or diagnostic use, drive `Solver` directly:

```rust
use contract_bridge::Strain;
use dds_rs::{Solver, solve_deal_on};

let mut solver = Solver::new(Strain::Notrump);
let table = solve_deal_on(&mut solver, deal);
```

## Performance

Benchmarked with `cargo bench` (seed 0, 200 random deals, 32-core machine).

| Engine                            | Serial (1 thr)      | Parallel (32 cores) |
|-----------------------------------|---------------------|---------------------|
| ddss 0.1 (DDS 2.9, C++ FFI)       | 131.5 ms/deal       | 9.9 ms/deal         |
| **dds-rs (DDS 2.9, pure Rust)**   | **149.3 ms/deal**   | **13.2 ms/deal**    |
| dds-bridge 0.19 (DDS 3.0, C++ FFI) | 193.7 ms/deal     | — †                 |

† dds-bridge 0.19 ships single-threaded; 0.20+ adds a parallel path.

Head-to-head benchmarks against each C++ crate live in their respective
repositories: `ddss/benches/compare_dds_rs.rs` and
`dds-bridge/benches/compare_dds_rs.rs`. They are kept separate because
ddss-sys and dds-bridge-sys both vendor the DDS C++ symbols and cannot
link into the same binary.

## License

[Apache-2.0](LICENSE)
