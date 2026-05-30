//! Diagnoses parallel load balance and per-strain cost for the
//! `solve_deals` work decomposition.
//!
//! `solve_deals` fans out one task per (deal, strain) across `rayon`. This
//! example replays that exact decomposition while timing each task, then
//! reports:
//!
//!   * the **tail ratio** = wall-clock makespan ÷ ideal (Σ task time ÷
//!     threads). 1.00 is perfect balance; higher means cores idled at the
//!     tail, so a hardest-first task ordering has headroom to recover it.
//!   * the **per-strain cost** distribution, to check *empirically* whether
//!     notrump is the slowest strain for this solver. It usually is: trump
//!     strains often reach a forced claimable ending (e.g. a cross-ruff)
//!     that the quick-/later-tricks heuristics resolve without expanding the
//!     subtree, whereas notrump has no ruffs and prunes less. This is the
//!     measured input for any per-strain cost model — never assume it.
//!
//! ```text
//! cargo run --release --example par_balance -- [N]
//! RAYON_NUM_THREADS=8 cargo run --release --example par_balance -- [N]
//! ```
//!
//! Uses the crate-default TT budget (via `Solver::new`). Per-worker solvers
//! are built fresh, so the first task on each worker pays TT warmup; pick a
//! large enough `N` that this is noise.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use contract_bridge::Strain;
use contract_bridge::deck::full_deal;
use core::hint::black_box;
use pons_dds::Solver;
use rand::SeedableRng;
use rand::rngs::SmallRng;
use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// Strain labels in `Strain::ASC` order (Clubs, Diamonds, Hearts, Spades,
/// Notrump) — the row order `solve_deals` uses.
const STRAIN_LABEL: [&str; 5] = ["C", "D", "H", "S", "NT"];

/// `p`-th percentile of an already-sorted slice (nearest-rank).
fn percentile(sorted: &[u128], p: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let last = sorted.len() - 1;
    let idx = ((p / 100.0) * last as f64).round() as usize;
    sorted[idx.min(last)]
}

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(500);

    let mut rng = SmallRng::seed_from_u64(0);
    let deals: Vec<_> = (0..n).map(|_| full_deal(&mut rng)).collect();

    let strains = Strain::ASC;
    // One task per (deal, strain): exactly what `solve_deals` hands rayon.
    let tasks: Vec<(usize, usize)> = (0..deals.len())
        .flat_map(|d| (0..strains.len()).map(move |s| (d, s)))
        .collect();

    // Mirror the production `solve_deals` executor: a dedicated pool with
    // large worker stacks (the deep search overflows the ~2 MiB default) and
    // a shared atomic cursor that hands tasks to workers one at a time. The
    // solver is parked in thread-local storage, off the deep search stack.
    thread_local! {
        static SOLVER: RefCell<Option<Solver>> = const { RefCell::new(None) };
    }
    let pool = rayon::ThreadPoolBuilder::new()
        .stack_size(16 * 1024 * 1024)
        .build()
        .expect("failed to build thread pool");
    let threads = pool.current_num_threads();
    let cursor = AtomicUsize::new(0);

    let start = Instant::now();
    let timings: Vec<(usize, u128)> = pool
        .broadcast(|_| {
            SOLVER.with(|cell| {
                let mut slot = cell.borrow_mut();
                let solver = slot.get_or_insert_with(|| Solver::new(Strain::Notrump));
                let mut local = Vec::new();
                loop {
                    let i = cursor.fetch_add(1, Ordering::Relaxed);
                    let Some(&(d, s)) = tasks.get(i) else { break };
                    solver.set_strain(strains[s]);
                    let t0 = Instant::now();
                    black_box(solver.solve(deals[d]));
                    local.push((s, t0.elapsed().as_nanos()));
                }
                local
            })
        })
        .into_iter()
        .flatten()
        .collect();
    let makespan = start.elapsed();
    let sum_nanos: u128 = timings.iter().map(|&(_, ns)| ns).sum();
    let ideal_nanos = sum_nanos as f64 / threads as f64;
    let tail_ratio = makespan.as_nanos() as f64 / ideal_nanos;

    let ms = |nanos: f64| nanos / 1.0e6;

    println!(
        "deals: {n}  tasks: {}  threads: {threads}  (seed 0)\n",
        tasks.len()
    );
    println!(
        "makespan         {:>9.2} ms",
        ms(makespan.as_nanos() as f64)
    );
    println!(
        "ms/deal          {:>9.3} ms",
        ms(makespan.as_nanos() as f64) / n as f64
    );
    println!(
        "Σ task time      {:>9.2} ms  (total compute)",
        ms(sum_nanos as f64)
    );
    println!("ideal (Σ/thr)    {:>9.2} ms", ms(ideal_nanos));
    println!("tail ratio       {tail_ratio:>9.2}x  (1.00 = perfect balance)\n");

    println!("per-strain task cost (ms):");
    println!(
        "{:>6} {:>8} {:>8} {:>8} {:>8}",
        "strain", "mean", "median", "p95", "share%"
    );
    for (s, label) in STRAIN_LABEL.iter().enumerate() {
        let mut v: Vec<u128> = timings
            .iter()
            .filter(|&&(ts, _)| ts == s)
            .map(|&(_, ns)| ns)
            .collect();
        v.sort_unstable();
        let strain_sum: u128 = v.iter().sum();
        let mean = strain_sum as f64 / v.len() as f64;
        let median = percentile(&v, 50.0) as f64;
        let p95 = percentile(&v, 95.0) as f64;
        let share = 100.0 * strain_sum as f64 / sum_nanos as f64;
        println!(
            "{label:>6} {:>8.2} {:>8.2} {:>8.2} {:>7.1}%",
            ms(mean),
            ms(median),
            ms(p95),
            share
        );
    }
}
