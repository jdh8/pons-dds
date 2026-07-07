//! Benchmarks for the pure-Rust solver entry points.
//!
//! Mirrors the [`dds-bridge`] crate's bench layout so the numbers can
//! be compared apples-to-apples on the published gh-pages dashboard.
//!
//! [`dds-bridge`]: https://crates.io/crates/dds-bridge

use arrayvec::ArrayVec;
use contract_bridge::deal::PartialDeal;
use contract_bridge::deck::full_deal;
use contract_bridge::{FullDeal, Seat, Strain};
use core::hint::black_box;
use core::time::Duration;
use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use pons_dds::{
    Board, CurrentTrick, NonEmptyStrainFlags, Objective, PlayTrace, Target, analyse_plays,
    solve_boards, solve_deal, solve_deals,
};
use rand::SeedableRng;
use rand::rngs::SmallRng;

/// `n` deterministic random deals from a seeded RNG.
fn deals(seed: u64, n: usize) -> Vec<FullDeal> {
    let mut rng = SmallRng::seed_from_u64(seed);
    (0..n).map(|_| full_deal(&mut rng)).collect()
}

/// A start-of-trick notrump board with North on lead — the shape the
/// FFI crates' `solve_boards` / `analyse_plays` benches use.
fn board_from(deal: FullDeal) -> Board {
    let remaining = PartialDeal::from(deal);
    Board::try_new(remaining, CurrentTrick::new(Strain::Notrump, Seat::North))
        .expect("start-of-trick NT board")
}

fn bench_solve_deal(c: &mut Criterion) {
    let mut rng = SmallRng::seed_from_u64(0);
    c.bench_function("solve_deal", |b| {
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
            b.iter(|| black_box(solve_deals(black_box(&ds), NonEmptyStrainFlags::ALL)));
        });
    }
    group.finish();
}

fn bench_solve_boards(c: &mut Criterion) {
    let mut group = c.benchmark_group("solve_boards");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));
    for &n in SIZES {
        let objectives: Vec<Objective> = deals(1, n)
            .into_iter()
            .map(|d| Objective {
                board: board_from(d),
                target: Target::Any(None),
            })
            .collect();
        group.throughput(Throughput::Elements(n as u64));
        group.bench_function(n.to_string(), |b| {
            b.iter(|| black_box(solve_boards(black_box(&objectives))));
        });
    }
    group.finish();
}

fn bench_analyse_plays(c: &mut Criterion) {
    let traces: Vec<PlayTrace> = deals(2, 32)
        .into_iter()
        .map(|d| PlayTrace {
            board: board_from(d),
            cards: ArrayVec::new(),
        })
        .collect();
    c.bench_function("analyse_plays_32", |b| {
        b.iter(|| black_box(analyse_plays(black_box(&traces))));
    });
}

criterion_group!(
    benches,
    bench_solve_deal,
    bench_solve_deals,
    bench_solve_boards,
    bench_analyse_plays,
);
criterion_main!(benches);
