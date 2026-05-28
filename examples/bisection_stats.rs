//! Measures whether `Engine::search_target`'s bisection is wasteful.
//!
//! Each `solve_deal` call invokes `search_target` 20 times (5 strains ×
//! 4 declarers). Each `search_target` runs a binary search on the trick
//! target, calling `ab_search_0` once per bisection iteration. If the
//! transposition table carries bounds between successive probes, that
//! iteration count is near 1; if the tree is re-traversed from scratch
//! each time, it approaches `log2(13) + 1 ≈ 4`.
//!
//! Run with `cargo run --release --example bisection_stats -- [N]`.

use contract_bridge::deck::full_deal;
use dds_rs::Solver;
use rand::SeedableRng;
use rand::rngs::SmallRng;
use std::time::Instant;

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);

    // Pre-materialize the deal corpus so RNG cost doesn't pollute timing.
    let mut rng = SmallRng::seed_from_u64(0);
    let deals: Vec<_> = (0..n).map(|_| full_deal(&mut rng)).collect();

    let mut solver = Solver::new();
    // One warmup pass to populate caches; not timed.
    for deal in &deals {
        std::hint::black_box(solver.solve_deal(*deal));
    }
    solver.reset_bisection_stats();

    let start = Instant::now();
    for deal in &deals {
        std::hint::black_box(solver.solve_deal(*deal));
    }
    let elapsed = start.elapsed();

    let (calls, iters) = solver.bisection_stats();
    #[allow(clippy::cast_precision_loss)]
    let avg = iters as f64 / calls as f64;
    #[allow(clippy::cast_precision_loss)]
    let ms_per_deal = elapsed.as_secs_f64() * 1000.0 / n as f64;

    println!("deals solved:           {n}");
    println!("total time:             {elapsed:?}");
    println!("ms per deal:            {ms_per_deal:.3}");
    println!(
        "search_target calls:    {calls}  (= {} per deal)",
        calls / n as u64
    );
    println!("bisection iterations:   {iters}");
    println!("avg iters per call:     {avg:.3}");
}
