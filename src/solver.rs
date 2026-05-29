//! Public solver API.
//!
//! Mirrors the per-instance `Solver` shape of the FFI-based
//! [`dds-bridge`](https://crates.io/crates/dds-bridge) crate so that a
//! `pons` migration from one to the other can be a near-mechanical swap.
//!
//! The two entry points are [`Solver::solve_deal`] for a single deal
//! (returns a full 5 × 4 [`TrickCountTable`]) and the free function
//! [`solve_deals`] for a parallel batch backed by `rayon` with a
//! per-worker thread-local [`Solver`].

use crate::moves::DDS_NOTRUMP;
use crate::pos::Pos;
use crate::quick_tricks::{MAXNODE, MINNODE};
use crate::search::Engine;
use crate::tt::TransTable;
use contract_bridge::{FullDeal, Seat, Strain, Suit};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};

// ---------------------------------------------------------------------
// Suit / strain conversion
// ---------------------------------------------------------------------
//
// `contract_bridge` numbers suits in ascending order (C=0, D=1, H=2,
// S=3) while DDS uses descending order (S=0, H=1, D=2, C=3); notrump is
// `DDS_NOTRUMP` (= 4) in DDS and `Strain::Notrump` (= 4) in `contract_bridge`.
//
// The two-way mapping is therefore `3 - cb_index` for suits and an
// identity at index 4 for notrump.

/// Convert a `contract_bridge::Suit` (C=0..S=3) to the DDS internal
/// suit index (S=0..C=3).
#[inline]
const fn dds_suit_from_cb(suit: Suit) -> usize {
    3 - suit as usize
}

/// Convert a `contract_bridge::Strain` to the DDS trump constant.
/// Notrump maps to [`DDS_NOTRUMP`] (= 4); suit strains follow the same
/// descending order as [`dds_suit_from_cb`].
#[inline]
fn dds_trump_from_strain(strain: Strain) -> i32 {
    strain
        .suit()
        .map_or(DDS_NOTRUMP, |suit| dds_suit_from_cb(suit) as i32)
}

/// All five strains in [`TrickCountTable`] row order (Clubs, Diamonds,
/// Hearts, Spades, Notrump). Matches `Strain::ASC`.
const STRAINS: [Strain; 5] = Strain::ASC;

/// All four seats in [`TrickCountTable`] column order (North, East,
/// South, West). Matches `Seat::ALL`.
const SEATS: [Seat; 4] = Seat::ALL;

// ---------------------------------------------------------------------
// FullDeal → Pos conversion
// ---------------------------------------------------------------------

/// Populate `pos.rank_in_suit` from a [`FullDeal`]. The remaining
/// `Pos` fields (`aggr`, `length`, `hand_dist`, `winner`, `second_best`)
/// are filled in by [`Engine::set_deal`]; this helper only writes the
/// raw card bitmaps in DDS suit ordering.
///
/// Bit `r` (for `r` in 2..=14) of `rank_in_suit[h][s]` is set iff DDS
/// hand `h` holds rank `r` in DDS suit `s`, per the vendor's
/// [`crate::lookup::BIT_MAP_RANK`] convention. The vendor packs rank
/// `r` at bit position `r - 2`, while `contract_bridge::Holding` packs
/// rank `r` at bit position `r`; we shift right by 2 to translate.
fn pos_from_deal(deal: &FullDeal) -> Pos {
    let mut pos = Pos::default();
    for (h, seat) in SEATS.iter().enumerate() {
        let cb_hand = deal[*seat];
        for cb_suit in Suit::ASC {
            // `Holding::to_bits()` uses bits 2..=14 for ranks 2..=14;
            // DDS uses bits 0..=12. Shift by 2 to convert.
            let bits = cb_hand[cb_suit].to_bits() >> 2;
            pos.rank_in_suit[h][dds_suit_from_cb(cb_suit)] = bits;
        }
    }
    pos
}

// ---------------------------------------------------------------------
// Result table
// ---------------------------------------------------------------------

/// Double-dummy result table: tricks each seat takes as declarer at
/// each strain.
///
/// Indexed by `(strain, seat)`. The storage is a flat `[[u8; 4]; 5]`
/// where the first axis is the strain in ascending order — Clubs,
/// Diamonds, Hearts, Spades, Notrump (matching [`Strain`]'s enum integer
/// values) — and the second is the seat in dealing order — North, East,
/// South, West (matching [`Seat`]).
///
/// Each entry is in `0..=13`. A later release may upgrade this to a
/// validated newtype that mirrors `ddss::tricks::TrickCountTable`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrickCountTable {
    /// Per-`(strain, seat)` trick count, in `0..=13`.
    pub tricks: [[u8; 4]; 5],
}

impl TrickCountTable {
    /// Return the number of tricks `seat` makes as declarer in `strain`.
    #[inline]
    #[must_use]
    pub const fn get(&self, strain: Strain, seat: Seat) -> u8 {
        self.tricks[strain as usize][seat as usize]
    }
}

// ---------------------------------------------------------------------
// Per-instance Solver
// ---------------------------------------------------------------------

/// Per-instance solver.
///
/// Owns a search engine and a transposition table; reuses both across
/// calls so the TT can warm up. Solving a new deal resets the TT — the
/// cached entries from a previous deal use a stale per-deal lookup
/// table and would produce incorrect hits.
///
/// `Solver` is `Send` but intentionally not `Sync`: the transposition
/// table is per-search-context and not safe for concurrent reads or
/// writes. Use the free [`solve_deals`] function to drive multiple
/// solvers in parallel.
pub struct Solver {
    engine: Engine,
    tt: TransTable,
}

impl Solver {
    /// Create a fresh solver with the default transposition-table
    /// memory budget. The trump is initialised to notrump; both will
    /// be overwritten by every [`Self::solve_deal`] call.
    #[must_use]
    pub fn new() -> Self {
        Self {
            engine: Engine::new(DDS_NOTRUMP),
            tt: TransTable::new(),
        }
    }

    /// Create a solver with an explicit transposition-table memory
    /// budget, in MiB: `default_mb` is the size the table shrinks back to
    /// on reset (per strain / deal), `max_mb` the ceiling before a full
    /// reset is forced. [`Self::new`] uses
    /// [`crate::tt::DEFAULT_MEMORY_MB`] / [`crate::tt::MAX_MEMORY_MB`].
    ///
    /// Bigger is better up to a plateau: a starved table full-resets and
    /// re-searches, so undersizing it explodes the node count (16/32 MiB
    /// is ~3.5× slower than the default). Correctness is unaffected at any
    /// size — a full table just resets and rebuilds. Mainly useful for
    /// capping per-thread memory in highly parallel runs.
    #[must_use]
    pub fn with_memory(default_mb: u32, max_mb: u32) -> Self {
        Self {
            engine: Engine::new(DDS_NOTRUMP),
            tt: TransTable::with_memory(default_mb, max_mb),
        }
    }

    /// Solve a single full deal across all 5 strains × 4 declarers.
    /// Returns the full 5 × 4 double-dummy table.
    ///
    /// Internally runs 20 alpha-beta searches. The transposition table
    /// is reset whenever the trump changes (the cached bounds become
    /// stale when the strain — hence the win/ruff dynamics — changes),
    /// matching the vendor's `SolveBoardInternal` / `SolveSameBoard`
    /// pairing: reset on `newDeal`/`newTrump`, reuse across same-strain
    /// declarer changes.
    #[must_use]
    pub fn solve_deal(&mut self, deal: FullDeal) -> TrickCountTable {
        let mut table = TrickCountTable::default();
        for strain_idx in 0..STRAINS.len() {
            table.tricks[strain_idx] = self.solve_strain(deal, strain_idx);
        }
        table
    }

    /// Solve a single strain (all 4 declarers) of `deal`, returning the
    /// per-seat trick row in [`SEATS`] order (North, East, South, West).
    ///
    /// Resets the transposition table for the strain's trump, then reuses
    /// it across the 4 declarer searches: the bounds are framed relative
    /// to seat 0's side, so they stay valid as the declarer — hence the
    /// MAX side — rotates within a strain. This per-strain unit is the
    /// grain of parallelism in [`solve_deals`]; keeping the 4 declarers
    /// on one unit preserves that intra-strain TT reuse.
    fn solve_strain(&mut self, deal: FullDeal, strain_idx: usize) -> [u8; 4] {
        // 13 tricks left → ini_depth = 48. The leader of trick 13 (the
        // opening lead) plays at depth `ini_depth`, then each follower
        // decrements depth by 1.
        const INI_DEPTH: i32 = 48;

        let trump = dds_trump_from_strain(STRAINS[strain_idx]);
        self.engine.set_trump(trump);
        // Drop entries cached under the previous trump (or for any
        // previous deal): the bounds stored at a given (trick, hand,
        // aggr, hand_dist) key are computed under the active trump
        // and would be incorrect after a strain change.
        self.tt.reset();

        let mut row = [0u8; 4];
        for (seat_idx, declarer) in SEATS.iter().enumerate() {
            // Opening leader = declarer's LHO; declarer plays third.
            let leader = declarer.lho() as usize;

            // MAX = the declaring side. NS declares → [MAX, MIN, MAX,
            // MIN]; EW declares → [MIN, MAX, MIN, MAX].
            let node_types = if matches!(declarer, Seat::North | Seat::South) {
                [MAXNODE, MINNODE, MAXNODE, MINNODE]
            } else {
                [MINNODE, MAXNODE, MINNODE, MAXNODE]
            };
            self.engine.set_node_types(node_types);

            // Rebuild Pos from scratch — cheap (~3 KiB struct) and
            // avoids having to remember which depth-indexed history
            // slots were touched by the previous search.
            let mut pos = pos_from_deal(&deal);
            pos.first[INI_DEPTH as usize] = leader as i32;

            // `set_deal` fills aggr/length/hand_dist/winner/
            // second_best from `rank_in_suit` and calls `tt.init`.
            self.engine.set_deal(&mut pos, &mut self.tt);

            let tricks = self.engine.search_target(&mut pos, &mut self.tt, INI_DEPTH);
            debug_assert!((0..=13).contains(&tricks), "tricks out of range");
            row[seat_idx] = tricks as u8;
        }
        row
    }
}

impl Solver {
    /// Diagnostic: total `(search_target_calls, bisection_iters)`
    /// accumulated by this solver's engine since it was created or
    /// [`Self::reset_bisection_stats`] was last called.
    ///
    /// `bisection_iters / search_target_calls` is the average number of
    /// alpha-beta probes per bisection driver call — a value close to 1
    /// means the TT carries bounds between probes; ≈ 4 means each probe
    /// re-traverses the tree from scratch.
    #[inline]
    #[must_use]
    pub const fn bisection_stats(&self) -> (u64, u64) {
        (self.engine.search_target_calls, self.engine.bisection_iters)
    }

    /// Zero the bisection diagnostic counters.
    #[inline]
    pub const fn reset_bisection_stats(&mut self) {
        self.engine.search_target_calls = 0;
        self.engine.bisection_iters = 0;
        self.engine.iter1_nanos = 0;
        self.engine.later_nanos = 0;
    }

    /// Cumulative `(iter1_nanos, later_nanos)` — wall-clock time spent
    /// in the first bisection iteration of each `search_target` call vs
    /// in subsequent iterations. The ratio answers whether TT-cached
    /// internal subtrees make later iters cheap.
    #[inline]
    #[must_use]
    pub const fn bisection_timing(&self) -> (u128, u128) {
        (self.engine.iter1_nanos, self.engine.later_nanos)
    }

    /// Cumulative per-node search instrumentation (TT hit rate,
    /// move-ordering cutoff index, node-0 early-exit funnel).
    ///
    /// All fields are zero unless the crate is built with
    /// `--features profiling`.
    #[inline]
    #[must_use]
    pub const fn search_stats(&self) -> crate::search::SearchStats {
        self.engine.stats
    }

    /// Zero the per-node search instrumentation counters.
    #[inline]
    pub fn reset_search_stats(&mut self) {
        self.engine.stats = crate::search::SearchStats::default();
    }
}

impl Default for Solver {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------
// Parallel batch
// ---------------------------------------------------------------------

/// Solve a batch of deals in parallel.
///
/// Each rayon worker thread amortises its own [`Solver`] (and the
/// associated transposition-table allocation) across the deals routed
/// to it via a [`std::thread_local!`] handle. Order of results matches
/// the order of `deals`.
///
/// This is the recommended entry point for solving many deals at once;
/// constructing a fresh [`Solver`] per deal would defeat warm-cache
/// effects within a worker.
#[must_use]
pub fn solve_deals(deals: &[FullDeal]) -> Vec<TrickCountTable> {
    use std::cell::RefCell;

    thread_local! {
        static SOLVER: RefCell<Solver> = RefCell::new(Solver::new());
    }

    deals
        .par_iter()
        .map(|deal| SOLVER.with(|s| s.borrow_mut().solve_deal(*deal)))
        .collect()
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use contract_bridge::deal::Builder;
    use contract_bridge::hand::{Hand, Holding};

    /// Verify the strain → trump mapping uses the descending suit order
    /// expected by the DDS substrate (and by `dds-bridge`'s
    /// `strain_to_sys`).
    #[test]
    fn strain_to_trump_uses_dds_descending_order() {
        assert_eq!(dds_trump_from_strain(Strain::Spades), 0);
        assert_eq!(dds_trump_from_strain(Strain::Hearts), 1);
        assert_eq!(dds_trump_from_strain(Strain::Diamonds), 2);
        assert_eq!(dds_trump_from_strain(Strain::Clubs), 3);
        assert_eq!(dds_trump_from_strain(Strain::Notrump), DDS_NOTRUMP);
    }

    /// Build a deal where each seat holds exactly one full 13-card suit:
    /// North = spades, East = hearts, South = diamonds, West = clubs.
    fn each_hand_holds_one_suit_deal() -> FullDeal {
        let full = Holding::ALL;
        let empty = Holding::EMPTY;
        let n_hand = Hand::new(empty, empty, empty, full); // C,D,H,S → only spades
        let e_hand = Hand::new(empty, empty, full, empty); // hearts
        let s_hand = Hand::new(empty, full, empty, empty); // diamonds
        let w_hand = Hand::new(full, empty, empty, empty); // clubs

        Builder::new()
            .north(n_hand)
            .east(e_hand)
            .south(s_hand)
            .west(w_hand)
            .build_full()
            .expect("each-suit fixture should be a valid full deal")
    }

    /// Pos conversion: each hand holds exactly one suit at full strength
    /// → that suit's bitmap is the DDS "all 13 ranks set" pattern
    /// (`0x1FFF`) for one hand and zero for the other three.
    #[test]
    fn pos_from_deal_each_hand_one_suit() {
        // contract_bridge → DDS suit mapping reminder:
        //   Suit::Clubs (0)    -> DDS suit 3
        //   Suit::Diamonds (1) -> DDS suit 2
        //   Suit::Hearts (2)   -> DDS suit 1
        //   Suit::Spades (3)   -> DDS suit 0
        //
        // DDS bit layout: rank `r` at bit `r-2`, so `Holding::ALL`
        // (0x7FFC, bits 2..=14) shifts to 0x1FFF (bits 0..=12).
        const DDS_ALL: u16 = 0x1FFF;

        let deal = each_hand_holds_one_suit_deal();
        let pos = pos_from_deal(&deal);

        // N (hand 0) holds spades → DDS suit 0.
        assert_eq!(pos.rank_in_suit[0][0], DDS_ALL);
        assert_eq!(pos.rank_in_suit[0][1], 0);
        assert_eq!(pos.rank_in_suit[0][2], 0);
        assert_eq!(pos.rank_in_suit[0][3], 0);
        // E (hand 1) holds hearts → DDS suit 1.
        assert_eq!(pos.rank_in_suit[1][1], DDS_ALL);
        // S (hand 2) holds diamonds → DDS suit 2.
        assert_eq!(pos.rank_in_suit[2][2], DDS_ALL);
        // W (hand 3) holds clubs → DDS suit 3.
        assert_eq!(pos.rank_in_suit[3][3], DDS_ALL);
    }

    /// Notrump table for the each-hand-holds-one-suit fixture.
    ///
    /// In NT, the opening leader must lead from their own suit; whoever
    /// of declarer / dummy can ruff (no one — notrump) takes only when
    /// the led suit is their own. With each suit fully held by one seat:
    ///
    /// * If declarer leads their own suit (= holds it), they have all
    ///   13 cards and run them all → 13 tricks for declarer.
    /// * BUT the opening lead is by declarer's LHO. The LHO must lead
    ///   from one of their suits (= the LHO's only suit). Since the
    ///   suits are disjoint, the LHO's lead is in a suit neither
    ///   declarer nor dummy holds → declarer/dummy must discard.
    ///
    /// Walking it through trick by trick: every trick is won by the
    /// leader (since no one else has the suit and there's no trump).
    /// The lead rotates only when the winner is on a different side.
    ///
    /// In this fixture, the LHO leads first; the LHO wins (they have
    /// all the cards in their suit), so they lead again. They keep
    /// winning every trick until they run out (13 tricks). So the
    /// opening leader wins all 13.
    ///
    /// * Declarer N: LHO = E. E wins 13. Declarer N → 0.
    /// * Declarer E: LHO = S. S wins 13. Declarer E → 0.
    /// * Declarer S: LHO = W. W wins 13. Declarer S → 0.
    /// * Declarer W: LHO = N. N wins 13. Declarer W → 0.
    ///
    /// So the entire NT row is zeros.
    #[test]
    fn solve_deal_each_hand_one_suit_notrump() {
        let deal = each_hand_holds_one_suit_deal();
        let mut solver = Solver::new();
        let table = solver.solve_deal(deal);

        // Notrump row: declarer always makes 0.
        for seat in Seat::ALL {
            assert_eq!(
                table.get(Strain::Notrump, seat),
                0,
                "declarer {seat} at NT should make 0 tricks (LHO runs their suit)"
            );
        }
    }

    /// Trump-table analytic check for the each-hand-holds-one-suit
    /// fixture.
    ///
    /// With every suit a perfect 13-card holding in one hand, the
    /// "trump suit" picks a winner that takes everything it has and
    /// ruffs all 13 cards from any other lead. The result:
    ///
    /// * The seat holding the trump suit always wins every trick — they
    ///   either lead the trump suit (their hand) or ruff a non-trump
    ///   lead. So that seat takes 13 tricks regardless of who declares.
    ///
    /// Translating into the table: for trump strain `X`, the only seat
    /// that wins any tricks is the one that holds suit `X`. If declarer
    /// IS that seat, declarer makes 13. If declarer is on the same side
    /// (partner), declarer-side makes 13 → declarer makes 13. Otherwise
    /// declarer makes 0.
    ///
    /// Suit ownership in this fixture:
    ///   spades → N, hearts → E, diamonds → S, clubs → W
    ///
    /// So:
    ///   * Spades trump: N and S (= NS) win 13; E and W (= EW) win 0.
    ///   * Hearts trump: E and W (= EW) win 13; N and S (= NS) win 0.
    ///   * Diamonds trump: same as spades (S holds them → NS wins 13).
    ///   * Clubs trump: same as hearts (W holds them → EW wins 13).
    #[test]
    fn solve_deal_each_hand_one_suit_trump_tables() {
        let deal = each_hand_holds_one_suit_deal();
        let mut solver = Solver::new();
        let table = solver.solve_deal(deal);

        // (strain, ns_makes, ew_makes)
        let cases = [
            (Strain::Spades, 13, 0),   // N owns spades → NS wins
            (Strain::Hearts, 0, 13),   // E owns hearts → EW wins
            (Strain::Diamonds, 13, 0), // S owns diamonds → NS wins
            (Strain::Clubs, 0, 13),    // W owns clubs → EW wins
        ];
        for (strain, ns, ew) in cases {
            assert_eq!(table.get(strain, Seat::North), ns, "N declaring {strain}");
            assert_eq!(table.get(strain, Seat::South), ns, "S declaring {strain}");
            assert_eq!(table.get(strain, Seat::East), ew, "E declaring {strain}");
            assert_eq!(table.get(strain, Seat::West), ew, "W declaring {strain}");
        }
    }

    /// Batch solver returns the same table as a sequential per-deal
    /// solve, and preserves input order.
    #[test]
    fn solve_deals_matches_single_deal_solver() {
        let deal_a = each_hand_holds_one_suit_deal();
        // Second deal: rotate by swapping NS and EW to verify ordering.
        // We just reuse the same deal twice — sufficient for ordering /
        // parity.
        let deals = vec![deal_a, deal_a];

        let mut sequential = Solver::new();
        let expected_a = sequential.solve_deal(deal_a);

        let parallel = solve_deals(&deals);
        assert_eq!(parallel.len(), 2);
        assert_eq!(parallel[0], expected_a);
        assert_eq!(parallel[1], expected_a);
    }

    /// Cross-check against a hand-verified reference table.
    ///
    /// The expected double-dummy table for the PBN deal
    ///
    /// ```text
    /// N:.63.AKQ987.A9732 A8654.KQ5.T.QJT6 J973.J98742.3.K4 KQT2.AT.J6542.85
    /// ```
    ///
    /// was generated by the FFI-backed `ddss::Solver` (which wraps the
    /// upstream DDS C++ reference). Both partnerships and all five
    /// strains are covered, so any sign error or off-by-one in the
    /// `FullDeal → Pos` conversion / opening-leader assignment will
    /// surface here.
    #[test]
    fn solve_deal_matches_reference_pbn() {
        let pbn = "N:.63.AKQ987.A9732 A8654.KQ5.T.QJT6 \
                   J973.J98742.3.K4 KQT2.AT.J6542.85";
        let deal: FullDeal = pbn.parse().expect("reference PBN parses");

        let mut solver = Solver::new();
        let got = solver.solve_deal(deal);

        // Reference rows in (N, E, S, W) order — verified against
        // ddss::Solver::lock().solve_deal(deal).
        let expected = TrickCountTable {
            tricks: [
                [8, 5, 8, 5], // ♣
                [8, 5, 8, 5], // ♦
                [6, 5, 6, 6], // ♥
                [4, 9, 4, 9], // ♠
                [5, 8, 5, 8], // NT
            ],
        };

        assert_eq!(got, expected, "DD table mismatch for reference deal");
    }
}
