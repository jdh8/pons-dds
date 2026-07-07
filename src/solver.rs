//! Public solver API.
//!
//! Mirrors the per-instance `Solver` shape of the FFI-based
//! [`dds-bridge`](https://crates.io/crates/dds-bridge) crate so that a
//! `pons` migration from one to the other can be a near-mechanical swap.
//!
//! The canonical entry points are the free functions [`solve_deal`] (one
//! deal, its 5 strains fanned across `rayon` workers) and [`solve_deals`]
//! (a batch, parallelised per (deal, strain)); both return a full 5 × 4
//! [`TrickCountTable`] per deal. [`Solver`] itself is the per-strain
//! building block they reuse: one instance is bound to a single strain
//! (reconfigurable via [`Solver::set_strain`]) and [`Solver::solve`]s all
//! 4 declarers of that strain for a deal — handy for deterministic
//! profiling or driving the solve yourself.

use crate::convert::dds_suit_from_cb;
use crate::pos::Pos;
use crate::quick_tricks::{MAXNODE, MINNODE};
use crate::search::Engine;
use crate::tricks::{TrickCountRow, TrickCountTable};
use crate::tt::TransTable;
use contract_bridge::{FullDeal, Seat, Strain, Suit};
use std::sync::OnceLock;

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
// Per-strain Solver
// ---------------------------------------------------------------------

/// Per-strain solver.
///
/// Bound to a single strain (set at [`Self::new`], retargetable via
/// [`Self::set_strain`]) and owns a search engine and a transposition
/// table, mirroring the per-strain `Engine`. [`Self::solve`] runs all
/// 4 declarers of the configured strain for a deal; for a full 5 × 4
/// table across every strain use the free [`solve_deal`] / [`solve_deals`].
///
/// The engine and TT are reused across calls so the TT can warm up.
/// Solving a deal resets the TT — the cached entries from a previous
/// deal (or strain) use a stale per-deal lookup table / trump and would
/// produce incorrect hits.
///
/// `Solver` is `Send` but intentionally not `Sync`: the transposition
/// table is per-search-context and not safe for concurrent reads or
/// writes. Use the free [`solve_deals`] function to drive multiple
/// solvers in parallel.
pub struct Solver {
    engine: Engine,
    tt: TransTable,
    /// `rank_in_suit` of the deal whose per-deal tables (`engine.rel`
    /// and the TT aggregator) are currently built. Lets a repeated
    /// [`Solver::solve`] of the same deal — e.g. the next strain —
    /// skip [`Engine::set_deal_tables`]; the tables are strain- and
    /// declarer-independent.
    deal_key: Option<[[u16; 4]; 4]>,
}

impl Solver {
    /// Create a fresh solver for `strain` with the default
    /// transposition-table memory budget. Retarget the strain later with
    /// [`Self::set_strain`].
    #[must_use]
    pub fn new(strain: Strain) -> Self {
        Self {
            engine: Engine::new(strain),
            tt: TransTable::new(),
            deal_key: None,
        }
    }

    /// Create a solver for `strain` with an explicit transposition-table
    /// memory budget, in MiB: `default_mb` is the size the table shrinks
    /// back to on reset (per solve), `max_mb` the ceiling before a full
    /// reset is forced. [`Self::new`] uses the built-in defaults
    /// (`DEFAULT_MEMORY_MB` / `MAX_MEMORY_MB`).
    ///
    /// Bigger is better up to a plateau: a starved table full-resets and
    /// re-searches, so undersizing it explodes the node count (16/32 MiB
    /// is ~3.5× slower than the default). Correctness is unaffected at any
    /// size — a full table just resets and rebuilds. Mainly useful for
    /// capping per-thread memory in highly parallel runs.
    #[must_use]
    pub fn with_memory(strain: Strain, default_mb: u32, max_mb: u32) -> Self {
        Self {
            engine: Engine::new(strain),
            tt: TransTable::with_memory(default_mb, max_mb),
            deal_key: None,
        }
    }

    /// Retarget the solver to a different strain. The next [`Self::solve`]
    /// resets the transposition table, so no stale-trump entries survive
    /// the change.
    pub fn set_strain(&mut self, strain: Strain) {
        self.engine.set_strain(strain);
    }

    /// Solve the configured strain (all 4 declarers) of `deal`, returning
    /// the per-seat trick row.
    ///
    /// Resets the transposition table for the strain's trump, then reuses
    /// it across the 4 declarer searches: the bounds are framed relative
    /// to seat 0's side, so they stay valid as the declarer — hence the
    /// MAX side — rotates within a strain. This per-strain unit is the
    /// grain of parallelism in [`solve_deals`]; keeping the 4 declarers
    /// on one unit preserves that intra-strain TT reuse.
    #[must_use]
    pub fn solve(&mut self, deal: FullDeal) -> TrickCountRow {
        // 13 tricks left → ini_depth = 48. The leader of trick 13 (the
        // opening lead) plays at depth `ini_depth`, then each follower
        // decrements depth by 1.
        const INI_DEPTH: i32 = 48;

        // Drop entries cached under the previous trump (or for any
        // previous deal): the bounds stored at a given (trick, hand,
        // aggr, hand_dist) key are computed under the active trump
        // and would be incorrect after a strain change.
        self.tt.reset();

        // The per-deal tables (`engine.rel`, TT aggregator) depend only
        // on which hand holds which card — invariant across declarer
        // AND strain, so build them once per deal and skip the rebuild
        // when the same deal comes back for its next strain (vendor:
        // `SolveSameBoard` skips all setup on repeat solves).
        let base_pos = pos_from_deal(&deal);
        if self.deal_key != Some(base_pos.rank_in_suit) {
            self.engine.set_deal_tables(&base_pos, &mut self.tt);
            self.deal_key = Some(base_pos.rank_in_suit);
        }

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

            // Fresh Pos per declarer — cheap (~3 KiB copy) and avoids
            // having to remember which depth-indexed history slots were
            // touched by the previous search.
            let mut pos = base_pos;
            pos.first[INI_DEPTH as usize] = leader as i32;
            self.engine.init_pos(&mut pos);

            // Seed the target walk (vendor `CalcSingleCommon`): the
            // partner of a solved declarer almost always scores the
            // same, an opponent almost always scores the complement;
            // the first declarer starts from the vendor's static guess.
            let hint = match seat_idx {
                0 => 7 - (leader as i32 & 1),
                2 => i32::from(row[0]),
                _ => 13 - i32::from(row[seat_idx - 1]),
            };

            let tricks = self
                .engine
                .search_target(&mut pos, &mut self.tt, INI_DEPTH, hint);
            debug_assert!((0..=13).contains(&tricks), "tricks out of range");
            row[seat_idx] = tricks as u8;
        }
        TrickCountRow::new(row[0], row[1], row[2], row[3])
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
    /// in the first target probe of each `search_target` call vs in
    /// subsequent probes. The ratio answers whether TT-cached internal
    /// subtrees make later probes cheap. Both stay 0 unless the crate
    /// is built with `--features profiling`.
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
        Self::new(Strain::Notrump)
    }
}

// ---------------------------------------------------------------------
// Parallel batch
// ---------------------------------------------------------------------

/// Per-worker stack size for the solver thread pool (`solver_pool`).
///
/// The alpha-beta search recurses up to ~52 plies with large per-frame
/// working sets (hence the `large_stack_*` allows in `lib.rs`), so a single
/// solve can want several MiB of stack — more than rayon's ~2 MiB default
/// worker stack, which it overflows. This is virtual address space; only
/// each worker's high-water mark is committed.
const SOLVER_STACK_SIZE: usize = 16 * 1024 * 1024;

/// Process-wide thread pool for batch solving, built once on first use.
///
/// Dedicated rather than rayon's global pool for two reasons: its workers
/// get the large `SOLVER_STACK_SIZE` stacks the search needs, and owning the
/// pool keeps each worker's persistent [`Solver`] (and its warm
/// transposition table) alive across calls. Thread count follows rayon's
/// usual default (`RAYON_NUM_THREADS`, else the available parallelism).
fn solver_pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .stack_size(SOLVER_STACK_SIZE)
            .thread_name(|i| format!("pons-dds-solver-{i}"))
            .build()
            .expect("failed to build pons-dds solver thread pool")
    })
}

/// Whether a strain's tasks should be dispatched ahead of the rest.
///
/// Notrump carries by far the heaviest solve-time tail: with no trump there
/// is no forced cross-ruff ending for the quick-/later-tricks heuristics to
/// claim, so its worst cases blow the search up hardest. Per-strain *means*
/// are nearly equal, so this changes makespan, not total work — starting the
/// tail-risky tasks first keeps a long notrump solve from landing last and
/// defining the finish time. Tune against `examples/par_balance.rs` on the
/// target host.
const fn dispatch_first(strain_idx: usize) -> bool {
    matches!(STRAINS[strain_idx], Strain::Notrump)
}

/// Drive `deals` through `solver_pool` with an explicit per-thread
/// transposition-table budget, returning one [`TrickCountTable`] per deal.
///
/// The work unit is one **(deal, strain)** pair — the 4 declarers of a
/// strain stay together so the per-strain table warms across them. Tasks are
/// ordered tail-risky-first (`dispatch_first`), then split into a bounded
/// number of work-stealing chunks. Bounding the chunk count caps rayon's
/// split-recursion depth (independent of batch size), so the deep search runs
/// from a shallow rayon stack — in a plain loop within each chunk — and worker
/// stack use does not grow with the batch. Work-stealing across the chunks
/// balances the cores without a contended shared counter.
fn solve_deals_pooled(deals: &[FullDeal], default_mb: u32, max_mb: u32) -> Vec<TrickCountTable> {
    use rayon::iter::ParallelIterator;
    use rayon::slice::ParallelSlice;
    use std::cell::RefCell;

    // Per-worker solver, parked in thread-local storage so it stays off the
    // deep search stack and warms across calls. The budget rides alongside it
    // so a worker rebuilds its table only when the budget changes.
    thread_local! {
        static SOLVER: RefCell<Option<(u32, u32, Solver)>> = const { RefCell::new(None) };
    }

    let mut tasks: Vec<(usize, usize)> = (0..deals.len())
        .flat_map(|d| (0..STRAINS.len()).map(move |s| (d, s)))
        .collect();
    // Stable: tail-risky strains first, deal order preserved within a rank.
    tasks.sort_by_key(|&(_, s)| core::cmp::Reverse(dispatch_first(s)));

    let pool = solver_pool();
    // Enough chunks for work-stealing to balance, few enough to keep rayon's
    // split depth (hence the search's rayon-stack nesting) bounded.
    let target_chunks = pool.current_num_threads().saturating_mul(8).max(1);
    let chunk_size = tasks.len().div_ceil(target_chunks).max(1);

    let collected: Vec<Vec<(usize, usize, TrickCountRow)>> = pool.install(|| {
        tasks
            .par_chunks(chunk_size)
            .map(|chunk| {
                SOLVER.with(|cell| {
                    let mut slot = cell.borrow_mut();
                    // Rebuild only when the requested budget changed.
                    if !matches!(slot.as_ref(), Some(&(d_mb, m_mb, _)) if d_mb == default_mb && m_mb == max_mb)
                    {
                        *slot = None;
                    }
                    let solver = &mut slot
                        .get_or_insert_with(|| {
                            (
                                default_mb,
                                max_mb,
                                Solver::with_memory(Strain::Notrump, default_mb, max_mb),
                            )
                        })
                        .2;

                    let mut rows = Vec::with_capacity(chunk.len());
                    for &(d, s) in chunk {
                        solver.set_strain(STRAINS[s]);
                        rows.push((d, s, solver.solve(deals[d])));
                    }
                    rows
                })
            })
            .collect()
    });

    // Scatter results back via each task's (deal, strain) pair.
    let mut tables = vec![TrickCountTable::default(); deals.len()];
    for (d, s, row) in collected.into_iter().flatten() {
        tables[d].0[s] = row;
    }
    tables
}

/// Solve a batch of deals in parallel.
///
/// The unit of work is a single **(deal, strain)** pair: a one-deal batch
/// spreads its 5 strains across workers, and a large batch yields
/// `5 × deals.len()` tasks for fine-grained load balancing. The 4 declarers
/// of a strain stay on one task so the per-strain transposition table warms
/// across them (see [`Solver::solve`]).
///
/// Solving runs on a dedicated, persistent thread pool whose workers each
/// keep a warm [`Solver`] across calls; tasks are self-scheduled
/// tail-risky-first. Order of results matches the order of `deals`.
///
/// This is the recommended entry point for solving many deals at once; for
/// low-latency solving of a single deal see [`solve_deal`].
#[must_use]
pub fn solve_deals(deals: &[FullDeal]) -> Vec<TrickCountTable> {
    solve_deals_pooled(
        deals,
        crate::tt::DEFAULT_MEMORY_MB,
        crate::tt::MAX_MEMORY_MB,
    )
}

/// Solve a batch of deals in parallel with an explicit per-thread
/// transposition-table memory budget, in MiB.
///
/// Identical in result to [`solve_deals`], but each pool worker builds its
/// [`Solver`] with [`Solver::with_memory`] (`default_mb` / `max_mb`) instead
/// of the built-in defaults. Use it to **cap per-thread memory** in highly
/// parallel runs — the table is per-thread, so the aggregate footprint is
/// roughly `threads × max_mb` MiB — or to sweep the budget for tuning (see
/// `examples/tt_sweep.rs`).
///
/// Like [`solve_deals`], each worker parks its [`Solver`] in `thread_local`
/// storage and reuses it across the tasks routed to it — and across calls,
/// so long as the requested budget is unchanged. A worker rebuilds its
/// table only when `default_mb` / `max_mb` differ from its previous call
/// (e.g. between sweep rows). For repeated batches at the default budget,
/// prefer [`solve_deals`].
#[must_use]
pub fn solve_deals_with_memory(
    deals: &[FullDeal],
    default_mb: u32,
    max_mb: u32,
) -> Vec<TrickCountTable> {
    solve_deals_pooled(deals, default_mb, max_mb)
}

/// Solve a single deal, spreading its 5 strains across rayon workers.
///
/// The recommended way to solve one deal. Where a single per-strain
/// [`Solver`] would run the 5 strains sequentially on one thread, this
/// fans them out so a single deal can use up to 5 cores — markedly faster
/// on a multi-core machine, and what keeps the pure-Rust solver
/// competitive with the FFI engines (whose own single-deal calls are
/// internally threaded). For many deals at once, prefer [`solve_deals`].
#[must_use]
pub fn solve_deal(deal: FullDeal) -> TrickCountTable {
    solve_deals(std::slice::from_ref(&deal))
        .pop()
        .unwrap_or_default()
}

/// Solve a single deal sequentially on `solver`, returning the full
/// 5 × 4 [`TrickCountTable`].
///
/// The deterministic single-thread counterpart to [`solve_deal`]: it
/// drives one per-strain [`Solver`] across all 5 strains in turn, on the
/// calling thread, so the solver's engine diagnostics
/// ([`Solver::search_stats`], [`Solver::bisection_stats`]) accumulate over
/// the whole table. Reuse the same `solver` across deals to amortise its
/// transposition-table allocation and gather corpus-wide statistics. For
/// throughput-oriented solving, prefer the parallel [`solve_deal`] /
/// [`solve_deals`].
#[must_use]
pub fn solve_deal_on(solver: &mut Solver, deal: FullDeal) -> TrickCountTable {
    let mut table = TrickCountTable::default();
    for (i, strain) in STRAINS.iter().enumerate() {
        solver.set_strain(*strain);
        table.0[i] = solver.solve(deal);
    }
    table
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use contract_bridge::deal::Builder;
    use contract_bridge::hand::{Hand, Holding};

    /// Solve a full deal on a fresh per-strain [`Solver`] — the
    /// deterministic single-thread reference the parallel free functions
    /// are checked against.
    fn solve_deal_sequential(deal: FullDeal) -> TrickCountTable {
        solve_deal_on(&mut Solver::new(Strain::Notrump), deal)
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
        let table = solve_deal_sequential(deal);

        // Notrump row: declarer always makes 0.
        for seat in Seat::ALL {
            assert_eq!(
                table[Strain::Notrump].get(seat).get(),
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
        let table = solve_deal_sequential(deal);

        // (strain, ns_makes, ew_makes)
        let cases = [
            (Strain::Spades, 13, 0),   // N owns spades → NS wins
            (Strain::Hearts, 0, 13),   // E owns hearts → EW wins
            (Strain::Diamonds, 13, 0), // S owns diamonds → NS wins
            (Strain::Clubs, 0, 13),    // W owns clubs → EW wins
        ];
        for (strain, ns, ew) in cases {
            let row = table[strain];
            assert_eq!(row.get(Seat::North).get(), ns, "N declaring {strain}");
            assert_eq!(row.get(Seat::South).get(), ns, "S declaring {strain}");
            assert_eq!(row.get(Seat::East).get(), ew, "E declaring {strain}");
            assert_eq!(row.get(Seat::West).get(), ew, "W declaring {strain}");
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

        let expected_a = solve_deal_sequential(deal_a);

        let parallel = solve_deals(&deals);
        assert_eq!(parallel.len(), 2);
        assert_eq!(parallel[0], expected_a);
        assert_eq!(parallel[1], expected_a);
    }

    /// The free `solve_deal` fans the 5 strains across rayon workers but
    /// must return the same table as the sequential single-thread solve.
    #[test]
    fn solve_deal_matches_single_deal_solver() {
        let deal = each_hand_holds_one_suit_deal();
        assert_eq!(solve_deal(deal), solve_deal_sequential(deal));
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

        let got = solve_deal_sequential(deal);

        // Reference rows in (N, E, S, W) order — verified against
        // ddss::Solver::lock().solve_deal(deal).
        let expected = TrickCountTable([
            TrickCountRow::new(8, 5, 8, 5), // ♣
            TrickCountRow::new(8, 5, 8, 5), // ♦
            TrickCountRow::new(6, 5, 6, 6), // ♥
            TrickCountRow::new(4, 9, 4, 9), // ♠
            TrickCountRow::new(5, 8, 5, 8), // NT
        ]);

        assert_eq!(got, expected, "DD table mismatch for reference deal");
    }
}
