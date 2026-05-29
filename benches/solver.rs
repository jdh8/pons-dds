//! Benchmarks for the pure-Rust solver entry points.
//!
//! Mirrors the [`dds-bridge`] crate's bench layout so the numbers can
//! be compared apples-to-apples on the published gh-pages dashboard.
//! v0.1 only exposes `solve_deal` and `solve_deals`, so the
//! `solve_boards` / `analyse_plays` cases from the FFI crate are
//! omitted here.
//!
//! [`dds-bridge`]: https://crates.io/crates/dds-bridge

use contract_bridge::deck::full_deal;
use contract_bridge::{FullDeal, Strain};
use core::hint::black_box;
use core::time::Duration;
use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use dds_rs::{Solver, solve_deal, solve_deal_on, solve_deals};
use rand::SeedableRng;
use rand::rngs::SmallRng;

/// `n` deterministic random deals from a seeded RNG.
fn deals(seed: u64, n: usize) -> Vec<FullDeal> {
    let mut rng = SmallRng::seed_from_u64(seed);
    (0..n).map(|_| full_deal(&mut rng)).collect()
}

fn bench_solve_deal_single(c: &mut Criterion) {
    let mut rng = SmallRng::seed_from_u64(0);
    let mut solver = Solver::new(Strain::Notrump);
    c.bench_function("solve_deal_single", |b| {
        b.iter_batched(
            || full_deal(&mut rng),
            |deal| black_box(solve_deal_on(&mut solver, black_box(deal))),
            BatchSize::SmallInput,
        );
    });
}

/// Single-deal latency via the strain-parallel entry point. Compare
/// against `solve_deal_single` to see the within-deal speedup from
/// spreading the 5 strains across rayon workers.
fn bench_solve_deal_parallel_single(c: &mut Criterion) {
    let mut rng = SmallRng::seed_from_u64(0);
    c.bench_function("solve_deal_parallel_single", |b| {
        b.iter_batched(
            || full_deal(&mut rng),
            |deal| black_box(solve_deal(black_box(deal))),
            BatchSize::SmallInput,
        );
    });
}

/// Batch sizes exercised by [`bench_solve_deals`].  Mirrors the
/// sibling crates `ddss` and `dds-bridge`: N=32 for the per-core
/// saturation case, N=200 for the amortization-friendly case where
/// each rayon worker's transposition table sees many deals.
const SIZES: &[usize] = &[32, 200];

fn bench_solve_deals(c: &mut Criterion) {
    let mut group = c.benchmark_group("solve_deals");
    group.sample_size(10);
    // 10 samples + 30 s budget keeps criterion from warning that the
    // default 5 s budget is too small for the slower N=200 iterations.
    group.measurement_time(Duration::from_secs(30));
    for &n in SIZES {
        let ds = deals(0, n);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_function(n.to_string(), |b| {
            b.iter(|| black_box(solve_deals(black_box(&ds))));
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_solve_deal_single,
    bench_solve_deal_parallel_single,
    bench_solve_deals
);
criterion_main!(benches);
