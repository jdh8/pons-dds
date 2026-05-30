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

/// `solve_deals` must run the deep alpha-beta search on its own large-stack
/// worker pool, never on the caller's stack — so it stays safe when invoked
/// from a thread with a small stack (e.g. Windows' 1 MiB default). Before the
/// dedicated pool, the calling thread joined rayon's work and a small-stack
/// caller overflowed. We verify by solving from a deliberately tiny-stack
/// thread and checking the result matches a normal (large-stack) call — the
/// solver is deterministic, so any divergence (or an overflow) fails here.
#[test]
fn solve_deals_safe_on_small_stack() {
    let mut rng = SmallRng::seed_from_u64(SEED);
    let deals: Vec<FullDeal> = (0..64).map(|_| full_deal(&mut rng)).collect();

    let on_small_stack = {
        let deals = deals.clone();
        std::thread::Builder::new()
            .stack_size(1024 * 1024) // 1 MiB — Windows' default; far below one solve
            .spawn(move || pons_dds::solve_deals(&deals))
            .expect("spawn small-stack thread")
            .join()
            .expect("small-stack solve panicked or overflowed its stack")
    };

    let reference = pons_dds::solve_deals(&deals);
    assert_eq!(on_small_stack.len(), reference.len());
    for (i, (a, b)) in on_small_stack.iter().zip(&reference).enumerate() {
        assert_eq!(
            a.tricks, b.tricks,
            "small-stack vs normal mismatch on deal #{i}"
        );
    }
}
