//! Sweeps the transposition-table memory budget to find the
//! throughput sweet spot.
//!
//! The solver is memory-latency bound (≈27% of cycles lost to
//! cache-misses on the ~95 MiB TT). A smaller table trades hit rate for
//! cache residency; this sweep measures where that trade nets out on a
//! fixed deal corpus.
//!
//! ```text
//! cargo run --release --example tt_sweep -- [N]
//! # add --features profiling to also fill the TT hit-rate column
//! ```
//!
//! Each row reports single-threaded ms/deal (lower is better) for a
//! `(default_mb, max_mb)` budget; speedup is vs the current default
//! (95 / 160 MiB).

use contract_bridge::deck::full_deal;
use contract_bridge::{FullDeal, Strain};
use dds_rs::{Solver, solve_deal_on};
use rand::SeedableRng;
use rand::rngs::SmallRng;
use std::time::Instant;

/// `(default_mb, max_mb)` budgets to probe. One page is ≈6.4 MiB, so
/// the smallest few collapse to 1–2 pages.
const BUDGETS: &[(u32, u32)] = &[
    (16, 32),
    (48, 64),
    (64, 96),
    (95, 160), // current default
    (160, 256),
    (256, 384),
    (512, 768),
];

const DEFAULT_BUDGET: (u32, u32) = (95, 160);

struct Row {
    default_mb: u32,
    max_mb: u32,
    ms_per_deal: f64,
    hit_pct: f64,
}

fn measure(default_mb: u32, max_mb: u32, deals: &[FullDeal]) -> Row {
    let mut solver = Solver::with_memory(Strain::Notrump, default_mb, max_mb);
    // Warmup (not timed): fault in pages, warm the TT for this budget.
    for deal in deals {
        std::hint::black_box(solve_deal_on(&mut solver, *deal));
    }
    solver.reset_search_stats();
    let start = Instant::now();
    for deal in deals {
        std::hint::black_box(solve_deal_on(&mut solver, *deal));
    }
    let elapsed = start.elapsed();

    let s = solver.search_stats();
    #[allow(clippy::cast_precision_loss)]
    let hit_pct = if s.tt_lookups == 0 {
        f64::NAN
    } else {
        100.0 * s.tt_hits as f64 / s.tt_lookups as f64
    };
    #[allow(clippy::cast_precision_loss)]
    let ms_per_deal = elapsed.as_secs_f64() * 1000.0 / deals.len() as f64;
    Row {
        default_mb,
        max_mb,
        ms_per_deal,
        hit_pct,
    }
}

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);

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

    println!("deals: {n}  (single-threaded, seed 0)\n");
    println!(
        "{:>9} {:>9} {:>11} {:>9} {:>10}",
        "default", "max", "ms/deal", "speedup", "TT hit %"
    );
    for r in &rows {
        let speedup = default_ms / r.ms_per_deal;
        let hit = if r.hit_pct.is_nan() {
            "n/a".to_string()
        } else {
            format!("{:.1}", r.hit_pct)
        };
        let tag = if (r.default_mb, r.max_mb) == DEFAULT_BUDGET {
            "  <- default"
        } else {
            ""
        };
        println!(
            "{:>5} MiB {:>5} MiB {:>11.3} {:>8.3}x {:>10}{tag}",
            r.default_mb, r.max_mb, r.ms_per_deal, speedup, hit
        );
    }
}
