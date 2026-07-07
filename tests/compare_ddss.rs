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
                pons[strain].get(seat).get(),
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
    let pons = pons_dds::solve_deals(&deals, pons_dds::NonEmptyStrainFlags::ALL);

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

/// Strain-filtered batches must match ddss cell-for-cell **including the
/// zero-filled rows of unrequested strains** — both crates document filtered
/// rows as meaningless zeros, and this pins that parity.
#[test]
fn filtered_solve_deals_matches_ddss() {
    let deals = deals(20);

    let cases = [
        ddss::StrainFlags::NOTRUMP,
        ddss::StrainFlags::SPADES,
        ddss::StrainFlags::HEARTS | ddss::StrainFlags::CLUBS,
    ];
    for flags in cases {
        let pons_flags = pons_dds::NonEmptyStrainFlags::new(
            pons_dds::StrainFlags::from_bits_truncate(flags.bits()),
        )
        .expect("non-empty");
        let pons = pons_dds::solve_deals(&deals, pons_flags);

        let solver = Solver::lock();
        let reference =
            solver.solve_deals(&deals, NonEmptyStrainFlags::new(flags).expect("non-empty"));
        core::mem::drop(solver);

        for ((&p, &r), &deal) in pons.iter().zip(&reference).zip(&deals) {
            assert_tables_match(p, r, deal);
        }
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

// ---------------------------------------------------------------------
// Par calculation
// ---------------------------------------------------------------------

/// Rebuild a ddss table as a pons table (identical strain/seat layout).
fn pons_table_from_ddss(table: ddss::TrickCountTable) -> pons_dds::TrickCountTable {
    pons_dds::TrickCountTable(Strain::ASC.map(|strain| {
        let row = table[strain];
        pons_dds::TrickCountRow::new(
            row.get(Seat::North).into(),
            row.get(Seat::East).into(),
            row.get(Seat::South).into(),
            row.get(Seat::West).into(),
        )
    }))
}

/// Exact structural equality across the crate boundary: same score and
/// the same ordered contract list, field for field. Both crates share
/// `contract_bridge::{Contract, Seat}`, so only the wrappers differ.
/// The exact bar holds because pons ports the very same vendor path per
/// entry point (`DealerParBin` / `SidesParBin`), text-parse quirks
/// included.
fn assert_par_matches(pons: &pons_dds::Par, oracle: &ddss::Par, context: &str) {
    let same = pons.score == oracle.score
        && pons.contracts.len() == oracle.contracts.len()
        && pons.contracts.iter().zip(&oracle.contracts).all(|(p, d)| {
            p.contract == d.contract && p.declarer == d.declarer && p.overtricks == d.overtricks
        });
    assert!(
        same,
        "par disagrees with ddss on {context}:\npons: {pons:?}\nddss: {oracle:?}"
    );
}

/// Compare `calculate_par` and `calculate_pars` against ddss over the DD
/// tables of `n` seeded deals × every vulnerability × every dealer. The
/// tables are computed once with ddss — par correctness is independent
/// of which engine produced the table.
fn cross_check_par(n: usize) {
    let deals = deals(n);
    let solver = Solver::lock();
    let tables = solver.solve_deals(&deals, NonEmptyStrainFlags::ALL);
    core::mem::drop(solver);

    let vulnerabilities = [
        (pons_dds::Vulnerability::NONE, ddss::Vulnerability::NONE),
        (pons_dds::Vulnerability::NS, ddss::Vulnerability::NS),
        (pons_dds::Vulnerability::EW, ddss::Vulnerability::EW),
        (pons_dds::Vulnerability::ALL, ddss::Vulnerability::ALL),
    ];

    for (table, deal) in tables.iter().zip(&deals) {
        let pons_table = pons_table_from_ddss(*table);
        for &(pons_vul, ddss_vul) in &vulnerabilities {
            for dealer in Seat::ALL {
                let pons = pons_dds::calculate_par(pons_table, pons_vul, dealer);
                let oracle = ddss::calculate_par(*table, ddss_vul, dealer);
                assert_par_matches(
                    &pons,
                    &oracle,
                    &format!("dealer {dealer:?}, vul {pons_vul}\ndeal: {deal}"),
                );
            }

            let pons_sides = pons_dds::calculate_pars(pons_table, pons_vul);
            let oracle_sides = ddss::calculate_pars(*table, ddss_vul);
            for (side, (pons, oracle)) in pons_sides.iter().zip(&oracle_sides).enumerate() {
                assert_par_matches(
                    pons,
                    oracle,
                    &format!("side {side}, vul {pons_vul}\ndeal: {deal}"),
                );
            }
        }
    }
}

/// Fast par cross-check run by default on every `cargo test`.
#[test]
fn par_matches_ddss() {
    cross_check_par(40);
}

/// Par soak over many more tables; run explicitly in release.
#[test]
#[ignore = "2k-deal par soak; run explicitly in release"]
fn par_matches_ddss_soak() {
    cross_check_par(2_000);
}
