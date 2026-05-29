//! Cross-check `dds-rs` against the reference C++-backed wrappers.
//!
//! Uses a fixed seed so test results are reproducible; the same `N`
//! deals are submitted to all three solvers and the resulting DD
//! tables must match cell-by-cell. Fails loudly with a printout of
//! the first divergent cell.

use contract_bridge::deck::full_deal;
use contract_bridge::{FullDeal, Seat, Strain};
use rand::SeedableRng;
use rand::rngs::SmallRng;

/// Fixed RNG seed so the same corpus is exercised on every run.
const SEED: u64 = 0;

/// Corpus size — 32 is enough for first-pass coverage and keeps the
/// suite well under a minute even in release builds.
const N: usize = 32;

/// Generate the shared corpus of [`FullDeal`]s from the fixed seed.
fn deals() -> Vec<FullDeal> {
    let mut rng = SmallRng::seed_from_u64(SEED);
    (0..N).map(|_| full_deal(&mut rng)).collect()
}

/// Solve a full deal on a fresh per-strain [`dds_rs::Solver`], returning
/// the raw `[strain][seat]` matrix. The deterministic single-thread
/// reference for the parallel free functions.
fn solve_deal_sequential(deal: FullDeal) -> [[u8; 4]; 5] {
    dds_rs::solve_deal_on(&mut dds_rs::Solver::new(Strain::Notrump), deal).tricks
}

/// Lower a [`ddss::TrickCountTable`] into the raw `[strain][seat]`
/// `u8` matrix used by [`dds_rs::TrickCountTable`].
fn extract_ddss(t: &ddss::TrickCountTable) -> [[u8; 4]; 5] {
    let mut out = [[0u8; 4]; 5];
    for (i, strain) in Strain::ASC.iter().enumerate() {
        let row = t[*strain];
        for (j, seat) in Seat::ALL.iter().enumerate() {
            out[i][j] = row.get(*seat).get();
        }
    }
    out
}

/// Strain enum from [`dds_bridge`]'s vendored bridge types, in
/// `Strain::ASC` order (Clubs, Diamonds, Hearts, Spades, Notrump).
const DDS_BRIDGE_STRAINS: [dds_bridge::Strain; 5] = [
    dds_bridge::Strain::Clubs,
    dds_bridge::Strain::Diamonds,
    dds_bridge::Strain::Hearts,
    dds_bridge::Strain::Spades,
    dds_bridge::Strain::Notrump,
];

/// Seat enum from [`dds_bridge`]'s vendored bridge types, in
/// `Seat::ALL` order (North, East, South, West).
const DDS_BRIDGE_SEATS: [dds_bridge::Seat; 4] = [
    dds_bridge::Seat::North,
    dds_bridge::Seat::East,
    dds_bridge::Seat::South,
    dds_bridge::Seat::West,
];

/// Lower a [`dds_bridge::solver::TrickCountTable`] into the raw
/// `[strain][seat]` `u8` matrix used by [`dds_rs::TrickCountTable`].
fn extract_dds_bridge(t: &dds_bridge::solver::TrickCountTable) -> [[u8; 4]; 5] {
    let mut out = [[0u8; 4]; 5];
    for (i, strain) in DDS_BRIDGE_STRAINS.iter().enumerate() {
        let row = t[*strain];
        for (j, seat) in DDS_BRIDGE_SEATS.iter().enumerate() {
            out[i][j] = row.get(*seat).get();
        }
    }
    out
}

/// Convert a [`contract_bridge::FullDeal`] into [`dds_bridge`]'s own
/// vendored `FullDeal` type by round-tripping through PBN notation —
/// the only conversion path the two distinct crates share.
fn to_dds_bridge_deal(deal: FullDeal) -> dds_bridge::FullDeal {
    deal.to_string()
        .parse()
        .expect("PBN round-trip from contract-bridge to dds-bridge")
}

#[test]
#[allow(clippy::significant_drop_tightening)] // hold the lock across the loop
fn dds_rs_matches_ddss() {
    let deals = deals();
    let theirs = ddss::Solver::lock();
    for (i, &d) in deals.iter().enumerate() {
        let our_t = dds_rs::solve_deal(d).tricks;
        let their_t = extract_ddss(&theirs.solve_deal(d));
        assert_eq!(
            our_t, their_t,
            "divergence on deal #{i}: {d}\n  dds-rs:    {our_t:?}\n  ddss:      {their_t:?}",
        );
    }
}

#[test]
#[allow(clippy::significant_drop_tightening)] // hold the lock across the loop
fn dds_rs_matches_dds_bridge() {
    let deals = deals();
    let theirs = dds_bridge::Solver::lock();
    for (i, &d) in deals.iter().enumerate() {
        let our_t = dds_rs::solve_deal(d).tricks;
        let their_t = extract_dds_bridge(&theirs.solve_deal(to_dds_bridge_deal(d)));
        assert_eq!(
            our_t, their_t,
            "divergence on deal #{i}: {d}\n  dds-rs:     {our_t:?}\n  dds-bridge: {their_t:?}",
        );
    }
}

/// The rayon batch must produce the same answers as repeated
/// single-deal calls — proves the per-worker `Solver` reuse and
/// TT reset logic are safe.
#[test]
fn dds_rs_solve_deals_matches_single() {
    let deals = deals();
    let batch = dds_rs::solve_deals(&deals);
    for (i, &d) in deals.iter().enumerate() {
        assert_eq!(
            batch[i].tricks,
            solve_deal_sequential(d),
            "batch vs single mismatch on deal #{i}: {d}",
        );
    }
}
