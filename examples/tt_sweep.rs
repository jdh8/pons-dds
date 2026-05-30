//! Sweeps the transposition-table memory budget to find the **parallel**
//! throughput sweet spot.
//!
//! The solver is memory bound: a bigger table lifts the TT hit rate (fewer
//! re-searches) but, replicated across every `rayon` worker, a large table
//! also stresses the shared memory bus and last-level cache. The
//! single-threaded optimum (~160 MiB) is therefore *not* the parallel
//! optimum — under contention a smaller per-thread table can win. This
//! sweep measures where that trade nets out on a fixed deal corpus, on
//! however many cores `rayon` is using.
//!
//! ```text
//! cargo run --release --example tt_sweep -- [N]          # all cores (parallel)
//! RAYON_NUM_THREADS=1 cargo run --release --example tt_sweep -- [N]   # serial reference
//! ```
//!
//! Each row reports wall-clock ms/deal (lower is better) for a
//! `(default_mb, max_mb)` budget across the active thread pool; speedup is
//! vs the current default ([`pons_dds`]'s 160 / 256 MiB).
//!
//! Note: [`solve_deals_with_memory`] builds fresh per-worker solvers each
//! call, so per-worker TT warmup is paid inside the timed call. Pick an `N`
//! large enough (the default is sized for that) that warmup amortises to
//! noise — at 32 cores each worker then handles dozens of tasks.

use contract_bridge::FullDeal;
use contract_bridge::deck::full_deal;
use core::hint::black_box;
use pons_dds::solve_deals_with_memory;
use rand::SeedableRng;
use rand::rngs::SmallRng;
use std::time::Instant;

/// `(default_mb, max_mb)` budgets to probe. One page is ≈6.4 MiB, so
/// the smallest few collapse to a handful of pages. Probes well below the
/// single-threaded optimum to expose the parallel sweet spot.
const BUDGETS: &[(u32, u32)] = &[
    (16, 32),
    (32, 48),
    (48, 64),
    (64, 96),
    (95, 160),
    (128, 192),
    (160, 256), // current default
];

/// The crate's current default budget ([`pons_dds::Solver::new`]).
const DEFAULT_BUDGET: (u32, u32) = (160, 256);

struct Row {
    default_mb: u32,
    max_mb: u32,
    ms_per_deal: f64,
}

#[allow(clippy::cast_precision_loss)]
fn measure(default_mb: u32, max_mb: u32, deals: &[FullDeal]) -> Row {
    // Warm up (untimed): build and fault each worker's table at this budget.
    // `solve_deals_with_memory` parks solvers in thread-local storage and
    // rebuilds only on a budget change, so the timed call below reuses these
    // warm tables — steady state, matching how `solve_deals` reuses tables
    // across calls in production.
    black_box(solve_deals_with_memory(deals, default_mb, max_mb));

    let start = Instant::now();
    let tables = solve_deals_with_memory(black_box(deals), default_mb, max_mb);
    let elapsed = start.elapsed();
    black_box(tables);

    let ms_per_deal = elapsed.as_secs_f64() * 1000.0 / deals.len() as f64;
    Row {
        default_mb,
        max_mb,
        ms_per_deal,
    }
}

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(500);

    let mut rng = SmallRng::seed_from_u64(0);
    let deals: Vec<_> = (0..n).map(|_| full_deal(&mut rng)).collect();

    let rows: Vec<Row> = BUDGETS
        .iter()
        .map(|&(d, m)| measure(d, m, &deals))
        .collect();

    let default_ms = rows
        .iter()
        .find(|r| (r.default_mb, r.max_mb) == DEFAULT_BUDGET)
        .map_or(f64::NAN, |r| r.ms_per_deal);

    let threads = rayon::current_num_threads();
    println!("deals: {n}  threads: {threads}  (seed 0)\n");
    println!(
        "{:>9} {:>9} {:>11} {:>9}",
        "default", "max", "ms/deal", "speedup"
    );
    for r in &rows {
        let speedup = default_ms / r.ms_per_deal;
        let tag = if (r.default_mb, r.max_mb) == DEFAULT_BUDGET {
            "  <- default"
        } else {
            ""
        };
        println!(
            "{:>5} MiB {:>5} MiB {:>11.3} {:>8.3}x{tag}",
            r.default_mb, r.max_mb, r.ms_per_deal, speedup
        );
    }
}
