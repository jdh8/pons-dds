//! Self-consistency tests: the parallel batch entry must return the same
//! answers as repeated sequential single-deal solves.

use contract_bridge::deck::full_deal;
use contract_bridge::{FullDeal, Strain};
use rand::SeedableRng;
use rand::rngs::SmallRng;

/// Fixed RNG seed so the same corpus is exercised on every run.
const SEED: u64 = 0;

/// Corpus size — 32 is enough for first-pass coverage and keeps the
/// suite well under a minute even in debug builds.
const N: usize = 32;

fn deals() -> Vec<FullDeal> {
    let mut rng = SmallRng::seed_from_u64(SEED);
    (0..N).map(|_| full_deal(&mut rng)).collect()
}

fn solve_deal_sequential(deal: FullDeal) -> [[u8; 4]; 5] {
    pons_dds::solve_deal_on(&mut pons_dds::Solver::new(Strain::Notrump), deal).tricks
}

/// The rayon batch must produce the same answers as repeated
/// single-deal calls — proves the per-worker `Solver` reuse and
/// TT reset logic are safe.
#[test]
fn solve_deals_matches_single() {
    let deals = deals();
    let batch = pons_dds::solve_deals(&deals);
    for (i, &d) in deals.iter().enumerate() {
        assert_eq!(
            batch[i].tricks,
            solve_deal_sequential(d),
            "batch vs single mismatch on deal #{i}: {d}",
        );
    }
}
