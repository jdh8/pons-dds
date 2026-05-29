//! Profiles the two levers that decide double-dummy search speed:
//! transposition-table hit rate and move-ordering quality.
//!
//! Build with the `profiling` feature so the per-node counters are
//! live (they compile to nothing otherwise):
//!
//! ```text
//! cargo run --release --features profiling --example search_stats -- [N]
//! ```
//!
//! Reported metrics:
//!
//! * **Node-0 funnel** — of every lead-node entry, the fraction that
//!   returns early (TT hit, trivial bound, leaf, quick/later tricks) vs
//!   the fraction that has to generate and search moves. Early exits are
//!   free; the move loop is where the cost lives.
//! * **TT hit rate** — `hits / lookups`. A high rate means the table is
//!   pruning whole subtrees.
//! * **Move ordering** — when a node beta-cuts, which move (1-based) in
//!   the ordered list fired the cutoff. Perfect ordering cuts on move 1;
//!   the "all-node" share is nodes searched to exhaustion with no cutoff.

use contract_bridge::Strain;
use contract_bridge::deck::full_deal;
use pons_dds::{Solver, solve_deal_on};
use rand::SeedableRng;
use rand::rngs::SmallRng;
use std::time::Instant;

#[allow(clippy::cast_precision_loss)]
fn pct(num: u64, den: u64) -> f64 {
    if den == 0 {
        0.0
    } else {
        100.0 * num as f64 / den as f64
    }
}

#[allow(clippy::too_many_lines)]
fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);

    let mut rng = SmallRng::seed_from_u64(0);
    let deals: Vec<_> = (0..n).map(|_| full_deal(&mut rng)).collect();

    let mut solver = Solver::new(Strain::Notrump);
    // Warmup pass (not measured): populate any first-touch caches.
    for deal in &deals {
        std::hint::black_box(solve_deal_on(&mut solver, *deal));
    }
    solver.reset_search_stats();

    let start = Instant::now();
    for deal in &deals {
        std::hint::black_box(solve_deal_on(&mut solver, *deal));
    }
    let elapsed = start.elapsed();

    let s = solver.search_stats();

    if s.node0_entries == 0 {
        eprintln!(
            "All counters are zero — rebuild with `--features profiling`:\n  \
             cargo run --release --features profiling --example search_stats -- {n}"
        );
        return;
    }

    #[allow(clippy::cast_precision_loss)]
    let ms_per_deal = elapsed.as_secs_f64() * 1000.0 / n as f64;

    let e = s.node0_entries;
    println!("deals solved:           {n}");
    println!("total time:             {elapsed:?}");
    println!("ms per deal:            {ms_per_deal:.3}");
    println!();

    println!("== Node-0 (lead) early-exit funnel ==");
    println!("node-0 entries:         {e}");
    println!(
        "  TT hit (depth>=20):   {:>12}  ({:5.1}%)",
        s.exit_tt_early,
        pct(s.exit_tt_early, e)
    );
    println!(
        "  trivial bound:        {:>12}  ({:5.1}%)",
        s.exit_trivial,
        pct(s.exit_trivial, e)
    );
    println!(
        "  leaf evaluate:        {:>12}  ({:5.1}%)",
        s.exit_leaf,
        pct(s.exit_leaf, e)
    );
    println!(
        "  quick tricks:         {:>12}  ({:5.1}%)",
        s.exit_quick,
        pct(s.exit_quick, e)
    );
    println!(
        "  later tricks:         {:>12}  ({:5.1}%)",
        s.exit_later,
        pct(s.exit_later, e)
    );
    println!(
        "  TT hit (depth<20):    {:>12}  ({:5.1}%)",
        s.exit_tt_late,
        pct(s.exit_tt_late, e)
    );
    println!(
        "  reached move loop:    {:>12}  ({:5.1}%)",
        s.reached_moveloop0,
        pct(s.reached_moveloop0, e)
    );
    println!();

    println!("== Transposition table ==");
    println!("lookups:                {}", s.tt_lookups);
    println!(
        "hits:                   {}  ({:.1}% hit rate)",
        s.tt_hits,
        pct(s.tt_hits, s.tt_lookups)
    );
    println!("stores:                 {}", s.tt_stores);
    println!();

    println!("== Move ordering (all node types) ==");
    let decided = s.cutoff_nodes + s.allnode_nodes;
    println!("nodes decided by loop:  {decided}");
    println!(
        "  beta/alpha cutoff:    {:>12}  ({:5.1}%)",
        s.cutoff_nodes,
        pct(s.cutoff_nodes, decided)
    );
    println!(
        "  all-node (no cutoff): {:>12}  ({:5.1}%)",
        s.allnode_nodes,
        pct(s.allnode_nodes, decided)
    );
    println!(
        "first-move cutoffs:     {:.1}% of cutoffs",
        pct(s.cutoff_first, s.cutoff_nodes)
    );
    #[allow(clippy::cast_precision_loss)]
    let mean_idx = if s.cutoff_nodes == 0 {
        0.0
    } else {
        s.cutoff_index_sum as f64 / s.cutoff_nodes as f64
    };
    println!("mean cutoff move index: {mean_idx:.3}");
    println!("cutoff index histogram (1-based, last bucket = 8+):");
    for (i, &count) in s.cutoff_hist.iter().enumerate() {
        let label = if i == 7 {
            "8+".to_string()
        } else {
            (i + 1).to_string()
        };
        println!(
            "  move {label:>2}: {count:>12}  ({:5.1}%)",
            pct(count, s.cutoff_nodes)
        );
    }
    println!();

    if pct(s.cutoff_first, s.cutoff_nodes) >= 90.0 {
        println!("→ Move ordering is excellent (>=90% first-move cutoffs).");
    } else if pct(s.cutoff_first, s.cutoff_nodes) >= 75.0 {
        println!(
            "→ Move ordering is decent; the tail past move 1 is where wasted node expansions hide."
        );
    } else {
        println!(
            "→ Move ordering is weak (<75% first-move cutoffs) — the biggest available prize."
        );
    }
}
