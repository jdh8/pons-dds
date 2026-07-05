//! Head-to-head benchmark: pure-Rust pons-dds vs. the FFI-backed `ddss`
//! (DDS 2.9 C++) reference, on identical seeded deals. Mirrors the layout of
//! `benches/solver.rs` and `../ddss/benches/solver.rs` (seed 0, N=32/200) so
//! the numbers line up with the published dashboards — but here both engines
//! run in one process, so criterion prints them side by side.

use contract_bridge::FullDeal;
use contract_bridge::deck::full_deal;
use core::hint::black_box;
use core::time::Duration;
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use ddss::{NonEmptyStrainFlags, Solver};
use rand::SeedableRng;
use rand::rngs::SmallRng;

/// `n` deterministic random deals from a seeded RNG.
fn deals(seed: u64, n: usize) -> Vec<FullDeal> {
    let mut rng = SmallRng::seed_from_u64(seed);
    (0..n).map(|_| full_deal(&mut rng)).collect()
}

/// Per-core saturation (32) and TT-amortization (200) cases; matches the
/// sibling `solver` benches so batch numbers are comparable.
const SIZES: &[usize] = &[32, 200];

fn bench_solve_deal(c: &mut Criterion) {
    let mut group = c.benchmark_group("solve_deal");

    let mut rng = SmallRng::seed_from_u64(0);
    group.bench_function("pons_dds", |b| {
        b.iter_batched(
            || full_deal(&mut rng),
            |deal| black_box(pons_dds::solve_deal(black_box(deal))),
            BatchSize::SmallInput,
        );
    });

    // Same seed → same deal sequence, so the two engines are timed on identical
    // inputs. `Solver` is `!Send`; lock it once on this thread for the run.
    let mut rng = SmallRng::seed_from_u64(0);
    let solver = Solver::lock();
    group.bench_function("ddss", |b| {
        b.iter_batched(
            || full_deal(&mut rng),
            |deal| black_box(solver.solve_deal(black_box(deal))),
            BatchSize::SmallInput,
        );
    });
    core::mem::drop(solver);

    group.finish();
}

fn bench_solve_deals(c: &mut Criterion) {
    let mut group = c.benchmark_group("solve_deals");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));

    let solver = Solver::lock();
    for &n in SIZES {
        let ds = deals(0, n);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::new("pons_dds", n), &ds, |b, ds| {
            b.iter(|| black_box(pons_dds::solve_deals(black_box(ds))));
        });
        group.bench_with_input(BenchmarkId::new("ddss", n), &ds, |b, ds| {
            b.iter(|| black_box(solver.solve_deals(black_box(ds), NonEmptyStrainFlags::ALL)));
        });
    }
    core::mem::drop(solver);

    group.finish();
}

criterion_group!(benches, bench_solve_deal, bench_solve_deals);
criterion_main!(benches);
