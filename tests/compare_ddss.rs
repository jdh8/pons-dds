//! Cross-check the pure-Rust solver against the FFI-backed `ddss` (DDS 2.9 C++)
//! reference. A double dummy deal has exactly one solution, so every cell of
//! every trick-count table must match bit-for-bit. This is the live oracle for
//! optimizing pons-dds: any divergence prints the offending deal as PBN so it
//! can be replayed in a focused debug run.

use contract_bridge::deck::full_deal;
use contract_bridge::{FullDeal, Seat, Strain};
use ddss::{NonEmptyStrainFlags, Solver};
use rand::SeedableRng;
use rand::rngs::SmallRng;

/// Fixed RNG seed so the same corpus is exercised on every run.
const SEED: u64 = 0;

fn deals(n: usize) -> Vec<FullDeal> {
    let mut rng = SmallRng::seed_from_u64(SEED);
    (0..n).map(|_| full_deal(&mut rng)).collect()
}

/// Assert every strain × seat cell of the pons-dds table equals the ddss one,
/// naming the deal, strain, and seat on the first mismatch.
fn assert_tables_match(
    pons: pons_dds::TrickCountTable,
    ddss: ddss::TrickCountTable,
    deal: FullDeal,
) {
    for strain in Strain::ASC {
        for seat in Seat::ALL {
            assert_eq!(
                pons.get(strain, seat),
                u8::from(ddss[strain].get(seat)),
                "pons-dds disagrees with ddss at {strain:?} declared by {seat:?}\ndeal: {deal}",
            );
        }
    }
}

/// Solve `n` seeded deals with both engines and compare every table. Both
/// batch entry points are parallel, so this stays fast even in a debug build.
fn cross_check(n: usize) {
    let deals = deals(n);
    let pons = pons_dds::solve_deals(&deals);

    // ddss::Solver holds a global reentrant lock and is `!Send`; acquire and
    // drop it on this thread. `solve_deals` parallelizes internally.
    let solver = Solver::lock();
    let reference = solver.solve_deals(&deals, NonEmptyStrainFlags::ALL);
    core::mem::drop(solver);

    assert_eq!(pons.len(), reference.len());
    for ((&p, &r), &deal) in pons.iter().zip(&reference).zip(&deals) {
        assert_tables_match(p, r, deal);
    }
}

/// Fast cross-check kept small enough to run by default on every `cargo test`.
#[test]
fn matches_ddss() {
    cross_check(100);
}

/// Heavy soak — the real "always matches ddss" proof. Ignored by default;
/// run during optimization with
/// `cargo test --release --test compare_ddss -- --ignored`.
#[test]
#[ignore = "10k-deal soak; run explicitly in release"]
fn matches_ddss_soak() {
    cross_check(10_000);
}
