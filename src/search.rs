//! Alpha-beta search.
//!
//! Ports
//! [`ABsearch.cpp`](../../../ddss-sys/vendor/src/ABsearch.cpp). The
//! [`Engine`] struct holds the per-search state (the vendor's
//! `ThreadData` minus TT — the TT is owned separately for lifetime
//! reasons) and exposes the bisection driver
//! [`Engine::search_target`].
//!
//! ## Layout
//!
//! Each `Engine` owns:
//! * [`crate::moves::Moves`] — the move generator and per-trick state.
//! * `node_type_store[4]` — MAX/MIN classification per seat.
//! * `rel: Box<[RelRanks; 8192]>` — the per-deal absolute-rank lookup
//!   (computed in [`Engine::set_deal`]; see vendor `SetDealTables`).
//! * Scratch arrays for per-depth `bestMove`, `bestMoveTT`, `lowestWin`,
//!   `forbiddenMoves`, and per-trick `winners` (used by `Make3` /
//!   `Undo0`).
//!
//! The position [`crate::pos::Pos`] and the [`crate::tt::TransTable`]
//! are passed in as `&mut` borrows; the engine never owns them.
//!
//! ## Make/Undo
//!
//! [`Engine::make0`] / [`Engine::make1`] / [`Engine::make2`] /
//! [`Engine::make3`] mirror the vendor's `Make0`..`Make3`, and
//! [`Engine::undo0`] / [`Engine::undo1`] / [`Engine::undo2`] /
//! [`Engine::undo3`] mirror the corresponding `Undo*`. The functions
//! update `Pos` in place; `make3` also snapshots `winner` /
//! `second_best` into a per-trick `winners` slot so `undo0` can restore
//! them exactly.

use crate::later_tricks::{later_tricks_max, later_tricks_min};
use crate::lookup::{BIT_MAP_RANK, WIN_RANKS};
use crate::move_type::{HighCard, MoveType};
use crate::moves::{DDS_NOTRUMP, Moves, RelRanks};
use crate::pos::{MAX_DEPTH, Pos};
use crate::quick_tricks::{MAXNODE, MINNODE, quick_tricks, quick_tricks_second_hand};
use crate::tt::{NodeCards, TransTable};

const DDS_SUITS: usize = 4;
const DDS_HANDS: usize = 4;

/// Vendor's `handDelta` constant. Used to update `pos.hand_dist[h]`
/// whenever a card is played or unplayed in suit `s`.
const HAND_DELTA: [i32; DDS_SUITS] = [256, 16, 1, 0];

/// Snapshot of `pos.winner[suit]` / `pos.second_best[suit]` for the
/// suits that were played during a trick. Used by [`Engine::make3`] to
/// stash the values so [`Engine::undo0`] can restore them.
///
/// Mirrors the vendor's `WinnerEntryType`.
#[derive(Clone, Copy, Debug, Default)]
struct WinnerEntry {
    suit: i32,
    winner_rank: i32,
    winner_hand: i32,
    second_rank: i32,
    second_hand: i32,
}

/// Per-trick container for [`WinnerEntry`] snapshots. Mirrors
/// `WinnersType`. Capacity 4 covers the worst case (one entry per suit
/// played, max 4 distinct suits).
#[derive(Clone, Copy, Debug, Default)]
struct Winners {
    number: i32,
    winner: [WinnerEntry; 4],
}

/// Per-search engine state. Owns [`Moves`], holds the per-deal `rel`
/// table and the per-depth scratch arrays.
pub(crate) struct Engine {
    pub moves: Moves,
    pub node_type_store: [i32; DDS_HANDS],
    pub ini_depth: i32,
    pub trump: i32,

    // Per-depth best-move arrays — indexed by `depth`.
    best_move: [MoveType; MAX_DEPTH],
    best_move_tt: [MoveType; MAX_DEPTH],

    // Per-depth lowest-win cache (vendor's `lowestWin[depth][suit]`).
    lowest_win: [[u16; DDS_SUITS]; MAX_DEPTH],

    /// User-forbidden moves; consulted by `purge()` at the top level.
    forbidden_moves: [MoveType; 14],

    /// Per-trick winner snapshot stack (vendor's `winners[13]`).
    winners: [Winners; 13],

    /// Per-deal relative-rank table. Computed by [`Engine::set_deal`].
    /// `Box`-allocated to keep the [`Engine`] struct small on the search
    /// stack.
    rel: Box<[RelRanks; 8192]>,
}

impl Engine {
    /// Create a fresh engine with the given trump. Use
    /// [`Engine::set_deal`] to compute the `rel` table for a specific
    /// deal before running [`Engine::search_target`].
    pub(crate) fn new(trump: i32) -> Self {
        let mut moves = Moves::new();
        moves.set_trump(trump);

        // MAX = N+S by default (matches SolverIF when handToPlay = N/S).
        let node_type_store = [MAXNODE, MINNODE, MAXNODE, MINNODE];

        Self {
            moves,
            node_type_store,
            ini_depth: 0,
            trump,
            best_move: [MoveType::default(); MAX_DEPTH],
            best_move_tt: [MoveType::default(); MAX_DEPTH],
            lowest_win: [[0u16; DDS_SUITS]; MAX_DEPTH],
            forbidden_moves: [MoveType::default(); 14],
            winners: [Winners::default(); 13],
            rel: vec![RelRanks::default(); 8192]
                .into_boxed_slice()
                .try_into()
                .unwrap_or_else(|_| unreachable!()),
        }
    }

    /// Set the MAX/MIN node-type assignment per seat. Pass `MAXNODE` for
    /// hands that are on the MAX side, `MINNODE` for the others. The
    /// usual conventions:
    /// * Declarer is N or S → `[MAX, MIN, MAX, MIN]`.
    /// * Declarer is E or W → `[MIN, MAX, MIN, MAX]`.
    pub(crate) fn set_node_types(&mut self, node_type_store: [i32; DDS_HANDS]) {
        self.node_type_store = node_type_store;
    }

    /// Configure the [`Moves`] trump strain and the engine's cached
    /// copy. Use [`DDS_NOTRUMP`] for notrump.
    pub(crate) fn set_trump(&mut self, trump: i32) {
        self.trump = trump;
        self.moves.set_trump(trump);
    }

    /// Reset the per-depth `bestMove` / `bestMoveTT` arrays. Mirrors
    /// vendor `ResetBestMoves`.
    pub(crate) fn reset_best_moves(&mut self) {
        for d in 0..MAX_DEPTH {
            self.best_move[d] = MoveType::default();
            self.best_move_tt[d] = MoveType::default();
        }
    }

    // ------------------------------------------------------------------
    // Per-deal setup: rel table, TT init, winner / second_best, hand_dist
    // ------------------------------------------------------------------

    /// Populate the per-deal `rel` table from `pos.rank_in_suit`,
    /// initialize `pos.winner` / `pos.second_best` / `pos.length` /
    /// `pos.aggr` / `pos.hand_dist`, and call `tt.init(handLookup)`.
    ///
    /// Mirrors the joint effect of `SetDeal` + `SetDealTables` +
    /// `InitWinners` in the vendor's `Init.cpp`.
    pub(crate) fn set_deal(&mut self, pos: &mut Pos, tt: &mut TransTable) {
        // --- SetDeal: aggr, length, hand_dist ---
        for s in 0..DDS_SUITS {
            pos.aggr[s] = 0;
            for h in 0..DDS_HANDS {
                pos.aggr[s] |= pos.rank_in_suit[h][s];
            }
        }
        for h in 0..DDS_HANDS {
            for s in 0..DDS_SUITS {
                pos.length[h][s] = pos.rank_in_suit[h][s].count_ones() as u8;
            }
        }
        for h in 0..DDS_HANDS {
            pos.hand_dist[h] = ((pos.length[h][0] as i32) << 8)
                | ((pos.length[h][1] as i32) << 4)
                | (pos.length[h][2] as i32);
        }

        // --- SetDealTables: handLookup + rel + tt.init ---
        let mut hand_lookup = [[0i32; 15]; DDS_SUITS];
        for (s, suit_lookup) in hand_lookup.iter_mut().enumerate() {
            for r in (2..=14).rev() {
                suit_lookup[r] = 0;
                for h in 0..DDS_HANDS {
                    if pos.rank_in_suit[h][s] & BIT_MAP_RANK[r] != 0 {
                        suit_lookup[r] = h as i32;
                        break;
                    }
                }
            }
        }

        tt.init(&hand_lookup);

        // rel[0]: every rank has (hand=-1, rank=0). Default is hand=0,
        // rank=0 — fix the hand field.
        for ord in 1..=13usize {
            for s in 0..DDS_SUITS {
                self.rel[0].abs_rank[ord][s].hand = -1;
                self.rel[0].abs_rank[ord][s].rank = 0;
            }
        }

        let mut top_bit_rank: u32 = 1;
        let mut top_bit_no: usize = 2;
        for aggr in 1u32..8192 {
            if aggr >= (top_bit_rank << 1) {
                top_bit_rank <<= 1;
                top_bit_no += 1;
            }
            self.rel[aggr as usize] = self.rel[(aggr ^ top_bit_rank) as usize];

            let weight = aggr.count_ones() as usize;
            for c in (2..=weight).rev() {
                for s in 0..DDS_SUITS {
                    let prev_hand = self.rel[aggr as usize].abs_rank[c - 1][s].hand;
                    let prev_rank = self.rel[aggr as usize].abs_rank[c - 1][s].rank;
                    self.rel[aggr as usize].abs_rank[c][s].hand = prev_hand;
                    self.rel[aggr as usize].abs_rank[c][s].rank = prev_rank;
                }
            }
            for (s, suit_lookup) in hand_lookup.iter().enumerate() {
                self.rel[aggr as usize].abs_rank[1][s].hand = suit_lookup[top_bit_no];
                self.rel[aggr as usize].abs_rank[1][s].rank = top_bit_no as i32;
            }
        }

        // --- InitWinners ---
        // For a position where no cards have been pre-played, we use
        // pos.aggr directly (matches the vendor's InitWinners with
        // posPoint.handRelFirst == 0).
        for s in 0..DDS_SUITS {
            let a = pos.aggr[s] as usize;
            pos.winner[s] = HighCard {
                rank: self.rel[a].abs_rank[1][s].rank,
                hand: self.rel[a].abs_rank[1][s].hand,
            };
            pos.second_best[s] = HighCard {
                rank: self.rel[a].abs_rank[2][s].rank,
                hand: self.rel[a].abs_rank[2][s].hand,
            };
        }
    }

    // ------------------------------------------------------------------
    // Make / Undo
    // ------------------------------------------------------------------

    /// Apply the leader's card to `pos`. Mirrors vendor `Make0`.
    #[inline]
    fn make0(&mut self, pos: &mut Pos, depth: i32, mply: &MoveType) {
        let depth_u = depth as usize;
        let h = pos.first[depth_u] as usize;
        let s = mply.suit as usize;
        let r = mply.rank as usize;

        pos.first[depth_u - 1] = pos.first[depth_u];
        pos.move_history[depth_u] = *mply;

        pos.rank_in_suit[h][s] &= !BIT_MAP_RANK[r];
        pos.aggr[s] ^= BIT_MAP_RANK[r];
        pos.hand_dist[h] -= HAND_DELTA[s];
        pos.length[h][s] = pos.length[h][s].saturating_sub(1);
    }

    /// Apply hand 1's card. Mirrors vendor `Make1`.
    #[inline]
    fn make1(&mut self, pos: &mut Pos, depth: i32, mply: &MoveType) {
        let depth_u = depth as usize;
        let first_hand = pos.first[depth_u];
        pos.first[depth_u - 1] = first_hand;

        let h = ((first_hand + 1) & 3) as usize;
        let s = mply.suit as usize;
        let r = mply.rank as usize;

        pos.rank_in_suit[h][s] &= !BIT_MAP_RANK[r];
        pos.aggr[s] ^= BIT_MAP_RANK[r];
        pos.hand_dist[h] -= HAND_DELTA[s];
        pos.length[h][s] = pos.length[h][s].saturating_sub(1);
    }

    /// Apply hand 2's card. Mirrors vendor `Make2`.
    #[inline]
    fn make2(&mut self, pos: &mut Pos, depth: i32, mply: &MoveType) {
        let depth_u = depth as usize;
        let first_hand = pos.first[depth_u];
        pos.first[depth_u - 1] = first_hand;

        let h = ((first_hand + 2) & 3) as usize;
        let s = mply.suit as usize;
        let r = mply.rank as usize;

        pos.rank_in_suit[h][s] &= !BIT_MAP_RANK[r];
        pos.aggr[s] ^= BIT_MAP_RANK[r];
        pos.hand_dist[h] -= HAND_DELTA[s];
        pos.length[h][s] = pos.length[h][s].saturating_sub(1);
    }

    /// Apply hand 3's card, finishing the trick. Mirrors vendor
    /// `Make3`. Writes `trick_cards[suit]` with the rank-bit(s) that
    /// just won (used by the caller to propagate `win_ranks`).
    #[inline]
    fn make3(
        &mut self,
        pos: &mut Pos,
        trick_cards: &mut [u16; DDS_SUITS],
        depth: i32,
        mply: &MoveType,
    ) {
        let depth_u = depth as usize;
        let first_hand = pos.first[depth_u];
        let trick = ((depth + 3) >> 2) as usize;

        let data = self.moves.get_trick_data((depth + 3) >> 2);

        pos.first[depth_u - 1] = (first_hand + data.rel_winner) & 3;

        let h = ((first_hand + 3) & 3) as usize;

        trick_cards.fill(0);
        let ss = data.best_suit as usize;
        if data.play_count[ss] >= 2 {
            let rr = data.best_rank as usize;
            trick_cards[ss] = BIT_MAP_RANK[rr] | (data.best_sequence as u16);
        }

        let r = mply.rank as usize;
        let s = mply.suit as usize;
        pos.rank_in_suit[h][s] &= !BIT_MAP_RANK[r];
        pos.aggr[s] ^= BIT_MAP_RANK[r];
        pos.hand_dist[h] -= HAND_DELTA[s];
        pos.length[h][s] = pos.length[h][s].saturating_sub(1);

        // Snapshot the winners that will change.
        let wp = &mut self.winners[trick];
        wp.number = 0;
        for st in 0..DDS_SUITS {
            if data.play_count[st] != 0 {
                let n = wp.number as usize;
                wp.winner[n].suit = st as i32;
                wp.winner[n].winner_rank = pos.winner[st].rank;
                wp.winner[n].winner_hand = pos.winner[st].hand;
                wp.winner[n].second_rank = pos.second_best[st].rank;
                wp.winner[n].second_hand = pos.second_best[st].hand;
                wp.number += 1;

                let aggr = pos.aggr[st] as usize;
                pos.winner[st].rank = self.rel[aggr].abs_rank[1][st].rank;
                pos.winner[st].hand = self.rel[aggr].abs_rank[1][st].hand;
                pos.second_best[st].rank = self.rel[aggr].abs_rank[2][st].rank;
                pos.second_best[st].hand = self.rel[aggr].abs_rank[2][st].hand;
            }
        }
    }

    /// Undo a [`Engine::make3`] (i.e. restore the just-played leader's
    /// card and roll back `winner` / `second_best`). Mirrors vendor
    /// `Undo0`.
    #[inline]
    fn undo0(&self, pos: &mut Pos, depth: i32, mply: &MoveType) {
        let depth_u = depth as usize;
        let trick = ((depth + 3) >> 2) as usize;
        let h = ((pos.first[depth_u] + 3) & 3) as usize;
        let s = mply.suit as usize;
        let r = mply.rank as usize;

        pos.rank_in_suit[h][s] |= BIT_MAP_RANK[r];
        pos.aggr[s] |= BIT_MAP_RANK[r];
        pos.hand_dist[h] += HAND_DELTA[s];
        pos.length[h][s] = pos.length[h][s].saturating_add(1);

        let wp = &self.winners[trick];
        for n in 0..(wp.number as usize) {
            let st = wp.winner[n].suit as usize;
            pos.winner[st].rank = wp.winner[n].winner_rank;
            pos.winner[st].hand = wp.winner[n].winner_hand;
            pos.second_best[st].rank = wp.winner[n].second_rank;
            pos.second_best[st].hand = wp.winner[n].second_hand;
        }
    }

    /// Undo a [`Engine::make0`] (leader's card). Mirrors vendor `Undo1`.
    #[inline]
    fn undo1(&self, pos: &mut Pos, depth: i32, mply: &MoveType) {
        let depth_u = depth as usize;
        let h = pos.first[depth_u] as usize;
        let s = mply.suit as usize;
        let r = mply.rank as usize;

        pos.rank_in_suit[h][s] |= BIT_MAP_RANK[r];
        pos.aggr[s] |= BIT_MAP_RANK[r];
        pos.hand_dist[h] += HAND_DELTA[s];
        pos.length[h][s] = pos.length[h][s].saturating_add(1);
    }

    /// Undo a [`Engine::make1`]. Mirrors vendor `Undo2`.
    #[inline]
    fn undo2(&self, pos: &mut Pos, depth: i32, mply: &MoveType) {
        let depth_u = depth as usize;
        let h = ((pos.first[depth_u] + 1) & 3) as usize;
        let s = mply.suit as usize;
        let r = mply.rank as usize;

        pos.rank_in_suit[h][s] |= BIT_MAP_RANK[r];
        pos.aggr[s] |= BIT_MAP_RANK[r];
        pos.hand_dist[h] += HAND_DELTA[s];
        pos.length[h][s] = pos.length[h][s].saturating_add(1);
    }

    /// Undo a [`Engine::make2`]. Mirrors vendor `Undo3`.
    #[inline]
    fn undo3(&self, pos: &mut Pos, depth: i32, mply: &MoveType) {
        let depth_u = depth as usize;
        let h = ((pos.first[depth_u] + 2) & 3) as usize;
        let s = mply.suit as usize;
        let r = mply.rank as usize;

        pos.rank_in_suit[h][s] |= BIT_MAP_RANK[r];
        pos.aggr[s] |= BIT_MAP_RANK[r];
        pos.hand_dist[h] += HAND_DELTA[s];
        pos.length[h][s] = pos.length[h][s].saturating_add(1);
    }

    // ------------------------------------------------------------------
    // Evaluate (depth == 0 leaf)
    // ------------------------------------------------------------------

    /// Last-trick evaluation. Mirrors vendor `Evaluate`. Writes the
    /// resulting win-ranks into `eval_win_ranks` and returns the final
    /// trick count for the MAX side.
    #[inline]
    fn evaluate(&self, pos: &Pos, eval_win_ranks: &mut [u16; DDS_SUITS]) -> i32 {
        let trump = self.trump;
        let first_hand = pos.first[0] as usize;

        eval_win_ranks.fill(0);

        let mut hmax: usize = 0;
        let mut rmax: u16 = 0;
        let mut count = 0;

        if trump != DDS_NOTRUMP {
            for h in 0..DDS_HANDS {
                if pos.rank_in_suit[h][trump as usize] != 0 {
                    count += 1;
                }
                if pos.rank_in_suit[h][trump as usize] > rmax {
                    hmax = h;
                    rmax = pos.rank_in_suit[h][trump as usize];
                }
            }

            if rmax > 0 {
                if count >= 2 {
                    eval_win_ranks[trump as usize] = rmax;
                }
                if self.node_type_store[hmax] == MAXNODE {
                    return pos.tricks_max + 1;
                } else {
                    return pos.tricks_max;
                }
            }
        }

        // Highest card in the suit played by the 1st hand.
        let mut k = 0usize;
        while k < DDS_SUITS {
            if pos.rank_in_suit[first_hand][k] != 0 {
                break;
            }
            k += 1;
        }
        debug_assert!(k < DDS_SUITS, "Evaluate: 1st hand has no cards");

        for h in 0..DDS_HANDS {
            if pos.rank_in_suit[h][k] != 0 {
                count += 1;
            }
            if pos.rank_in_suit[h][k] > rmax {
                hmax = h;
                rmax = pos.rank_in_suit[h][k];
            }
        }

        if count >= 2 {
            eval_win_ranks[k] = rmax;
        }

        if self.node_type_store[hmax] == MAXNODE {
            pos.tricks_max + 1
        } else {
            pos.tricks_max
        }
    }

    // ------------------------------------------------------------------
    // ABsearch — recursive entry points
    // ------------------------------------------------------------------

    /// Lead-hand AB search. Mirrors vendor `ABsearch0`.
    #[inline]
    pub(crate) fn ab_search_0(
        &mut self,
        pos: &mut Pos,
        tt: &mut TransTable,
        target: i32,
        depth: i32,
    ) -> bool {
        let trump = self.trump;
        let depth_u = depth as usize;
        let hand = pos.first[depth_u];
        let tricks = depth >> 2;

        for ss in 0..DDS_SUITS {
            pos.win_ranks[depth_u][ss] = 0;
        }

        if depth >= 20 && (tricks as usize) < 12 {
            if let Some(value) = self.tt_lookup(pos, tt, target, depth, tricks, hand) {
                return value;
            }
        }

        if pos.tricks_max >= target {
            return true;
        }
        if pos.tricks_max + tricks + 1 < target {
            return false;
        }
        if depth == 0 {
            let mut ev_win = [0u16; DDS_SUITS];
            let value_tricks = self.evaluate(pos, &mut ev_win);
            let value = value_tricks >= target;
            pos.win_ranks[depth_u].copy_from_slice(&ev_win);
            return value;
        }

        // ----- QuickTricks ---------------------------------------------------
        let mut res = false;
        let qtricks = quick_tricks(
            pos,
            hand,
            depth,
            target,
            trump,
            &mut res,
            &self.node_type_store,
        );
        let hand_u = hand as usize;

        if self.node_type_store[hand_u] == MAXNODE {
            if res {
                return qtricks != 0;
            }
            if !later_tricks_min(pos, hand, depth, target, trump, &self.node_type_store) {
                return false;
            }
        } else {
            if res {
                return qtricks == 0;
            }
            if later_tricks_max(pos, hand, depth, target, trump, &self.node_type_store) {
                return true;
            }
        }

        // ----- TT lookup (depth < 20 path) -----------------------------------
        if depth < 20 && (tricks as usize) < 12 {
            if let Some(value) = self.tt_lookup(pos, tt, target, depth, tricks, hand) {
                return value;
            }
        }

        // ----- Movegen + loop ------------------------------------------------
        let success = self.node_type_store[hand_u] == MAXNODE;
        let mut value = !success;

        for ss in 0..DDS_SUITS {
            self.lowest_win[depth_u][ss] = 0;
        }
        let bm = self.best_move[depth_u];
        let bmtt = self.best_move_tt[depth_u];
        self.moves
            .move_gen_0(tricks, pos, &bm, &bmtt, self.rel.as_ref());

        for ss in 0..DDS_SUITS {
            pos.win_ranks[depth_u][ss] = 0;
        }

        let mut chosen_move = MoveType::default();
        let mut cutoff = false;
        loop {
            let win_arr = pos.win_ranks[depth_u];
            let mply = self.moves.make_next(tricks, 0, &win_arr);
            let mply = match mply {
                Some(m) => m,
                None => break,
            };

            self.make0(pos, depth, &mply);
            value = self.ab_search_1(pos, tt, target, depth - 1);
            self.undo1(pos, depth, &mply);

            if value == success {
                for ss in 0..DDS_SUITS {
                    pos.win_ranks[depth_u][ss] = pos.win_ranks[depth_u - 1][ss];
                }
                chosen_move = mply;
                cutoff = true;
                break;
            }
            for ss in 0..DDS_SUITS {
                pos.win_ranks[depth_u][ss] |= pos.win_ranks[depth_u - 1][ss];
            }
        }

        if cutoff {
            self.best_move[depth_u] = chosen_move;
        }

        // ----- TT store ------------------------------------------------------
        if (tricks as usize) < 12 {
            let mut first = NodeCards::default();
            if value {
                if self.node_type_store[0] == MAXNODE {
                    first.ubound = (tricks + 1) as i8;
                    first.lbound = (target - pos.tricks_max) as i8;
                } else {
                    first.ubound = (tricks + 1 - target + pos.tricks_max) as i8;
                    first.lbound = 0;
                }
            } else if self.node_type_store[0] == MAXNODE {
                first.ubound = (target - pos.tricks_max - 1) as i8;
                first.lbound = 0;
            } else {
                first.ubound = (tricks + 1) as i8;
                first.lbound = (tricks + 1 - target + pos.tricks_max + 1) as i8;
            }
            first.best_move_suit = self.best_move[depth_u].suit as u8;
            first.best_move_rank = self.best_move[depth_u].rank as u8;

            let flag = (self.node_type_store[hand_u] == MAXNODE && value)
                || (self.node_type_store[hand_u] == MINNODE && !value);

            // The TT API wants `aggr` as u32 and `win_ranks` as u16.
            let aggr_u32 = [
                pos.aggr[0] as u32,
                pos.aggr[1] as u32,
                pos.aggr[2] as u32,
                pos.aggr[3] as u32,
            ];
            tt.add(
                tricks,
                hand,
                &aggr_u32,
                &pos.win_ranks[depth_u],
                first,
                flag,
            );
        }

        value
    }

    /// Helper: do a TT lookup and, if successful, return the cached
    /// boolean outcome (and update `pos.win_ranks[depth]` /
    /// `best_move_tt[depth]`).
    #[inline]
    fn tt_lookup(
        &mut self,
        pos: &mut Pos,
        tt: &mut TransTable,
        target: i32,
        depth: i32,
        tricks: i32,
        hand: i32,
    ) -> Option<bool> {
        let depth_u = depth as usize;
        let limit = if self.node_type_store[0] == MAXNODE {
            target - pos.tricks_max - 1
        } else {
            tricks - (target - pos.tricks_max - 1)
        };
        let aggr_u32 = [
            pos.aggr[0] as u32,
            pos.aggr[1] as u32,
            pos.aggr[2] as u32,
            pos.aggr[3] as u32,
        ];
        let mut lower_flag = false;
        let cards = tt.lookup(
            tricks,
            hand,
            &aggr_u32,
            &pos.hand_dist,
            limit,
            &mut lower_flag,
        )?;
        let lw = cards.least_win;
        let bm_suit = cards.best_move_suit as i32;
        let bm_rank = cards.best_move_rank as i32;

        for ss in 0..DDS_SUITS {
            pos.win_ranks[depth_u][ss] = WIN_RANKS[pos.aggr[ss] as usize][lw[ss] as usize];
        }
        if bm_rank != 0 {
            self.best_move_tt[depth_u].suit = bm_suit;
            self.best_move_tt[depth_u].rank = bm_rank;
        }
        let score = if self.node_type_store[0] == MAXNODE {
            lower_flag
        } else {
            !lower_flag
        };
        Some(score)
    }

    /// Second-hand AB search. Mirrors vendor `ABsearch1`.
    #[inline]
    pub(crate) fn ab_search_1(
        &mut self,
        pos: &mut Pos,
        tt: &mut TransTable,
        target: i32,
        depth: i32,
    ) -> bool {
        let trump = self.trump;
        let depth_u = depth as usize;
        let hand = (pos.first[depth_u] + 1) & 3;
        let success = self.node_type_store[hand as usize] == MAXNODE;
        let mut value = !success;
        let tricks = (depth + 3) >> 2;

        if quick_tricks_second_hand(
            pos,
            hand,
            depth,
            target,
            trump,
            &self.node_type_store,
            self.ini_depth,
        ) {
            return success;
        }

        for ss in 0..DDS_SUITS {
            self.lowest_win[depth_u][ss] = 0;
        }
        self.moves.move_gen_123(tricks, 1, pos);
        if depth == self.ini_depth {
            let fm = self.forbidden_moves;
            self.moves.purge(tricks, 1, &fm);
        }

        for ss in 0..DDS_SUITS {
            pos.win_ranks[depth_u][ss] = 0;
        }

        let mut chosen_move = MoveType::default();
        let mut cutoff = false;
        loop {
            let win_arr = pos.win_ranks[depth_u];
            let mply = self.moves.make_next(tricks, 1, &win_arr);
            let mply = match mply {
                Some(m) => m,
                None => break,
            };

            self.make1(pos, depth, &mply);
            value = self.ab_search_2(pos, tt, target, depth - 1);
            self.undo2(pos, depth, &mply);

            if value == success {
                for ss in 0..DDS_SUITS {
                    pos.win_ranks[depth_u][ss] = pos.win_ranks[depth_u - 1][ss];
                }
                chosen_move = mply;
                cutoff = true;
                break;
            }
            for ss in 0..DDS_SUITS {
                pos.win_ranks[depth_u][ss] |= pos.win_ranks[depth_u - 1][ss];
            }
        }
        if cutoff {
            self.best_move[depth_u] = chosen_move;
        }
        value
    }

    /// Third-hand AB search. Mirrors vendor `ABsearch2`.
    #[inline]
    pub(crate) fn ab_search_2(
        &mut self,
        pos: &mut Pos,
        tt: &mut TransTable,
        target: i32,
        depth: i32,
    ) -> bool {
        let depth_u = depth as usize;
        let hand = (pos.first[depth_u] + 2) & 3;
        let success = self.node_type_store[hand as usize] == MAXNODE;
        let mut value = !success;
        let tricks = (depth + 3) >> 2;

        for ss in 0..DDS_SUITS {
            self.lowest_win[depth_u][ss] = 0;
        }
        self.moves.move_gen_123(tricks, 2, pos);
        if depth == self.ini_depth {
            let fm = self.forbidden_moves;
            self.moves.purge(tricks, 2, &fm);
        }

        for ss in 0..DDS_SUITS {
            pos.win_ranks[depth_u][ss] = 0;
        }

        let mut chosen_move = MoveType::default();
        let mut cutoff = false;
        loop {
            let win_arr = pos.win_ranks[depth_u];
            let mply = self.moves.make_next(tricks, 2, &win_arr);
            let mply = match mply {
                Some(m) => m,
                None => break,
            };

            self.make2(pos, depth, &mply);
            value = self.ab_search_3(pos, tt, target, depth - 1);
            self.undo3(pos, depth, &mply);

            if value == success {
                for ss in 0..DDS_SUITS {
                    pos.win_ranks[depth_u][ss] = pos.win_ranks[depth_u - 1][ss];
                }
                chosen_move = mply;
                cutoff = true;
                break;
            }
            for ss in 0..DDS_SUITS {
                pos.win_ranks[depth_u][ss] |= pos.win_ranks[depth_u - 1][ss];
            }
        }
        if cutoff {
            self.best_move[depth_u] = chosen_move;
        }
        value
    }

    /// Fourth-hand AB search. Mirrors vendor `ABsearch3`. Resolves the
    /// trick winner, increments `tricks_max` accordingly, and recurses
    /// into [`Engine::ab_search_0`] with the trick-winner as the new
    /// leader.
    #[inline]
    pub(crate) fn ab_search_3(
        &mut self,
        pos: &mut Pos,
        tt: &mut TransTable,
        target: i32,
        depth: i32,
    ) -> bool {
        let depth_u = depth as usize;
        let hand = (pos.first[depth_u] + 3) & 3;
        let success = self.node_type_store[hand as usize] == MAXNODE;
        let mut value = !success;
        let tricks = (depth + 3) >> 2;

        for ss in 0..DDS_SUITS {
            self.lowest_win[depth_u][ss] = 0;
        }
        self.moves.move_gen_123(tricks, 3, pos);
        if depth == self.ini_depth {
            let fm = self.forbidden_moves;
            self.moves.purge(tricks, 3, &fm);
        }

        for ss in 0..DDS_SUITS {
            pos.win_ranks[depth_u][ss] = 0;
        }

        let mut make_win_rank = [0u16; DDS_SUITS];
        let mut chosen_move = MoveType::default();
        let mut cutoff = false;

        loop {
            let win_arr = pos.win_ranks[depth_u];
            let mply = self.moves.make_next(tricks, 3, &win_arr);
            let mply = match mply {
                Some(m) => m,
                None => break,
            };

            self.make3(pos, &mut make_win_rank, depth, &mply);

            let new_lead = pos.first[depth_u - 1] as usize;
            let incremented = self.node_type_store[new_lead] == MAXNODE;
            if incremented {
                pos.tricks_max += 1;
            }

            value = self.ab_search_0(pos, tt, target, depth - 1);

            self.undo0(pos, depth, &mply);

            if incremented {
                pos.tricks_max -= 1;
            }

            if value == success {
                let (prev_rows, cur_rows) = pos.win_ranks.split_at_mut(depth_u);
                for ((&prev, dst), &mwr) in prev_rows[depth_u - 1]
                    .iter()
                    .zip(cur_rows[0].iter_mut())
                    .zip(make_win_rank.iter())
                {
                    *dst = prev | mwr;
                }
                chosen_move = mply;
                cutoff = true;
                break;
            }
            let (prev_rows, cur_rows) = pos.win_ranks.split_at_mut(depth_u);
            for ((&prev, dst), &mwr) in prev_rows[depth_u - 1]
                .iter()
                .zip(cur_rows[0].iter_mut())
                .zip(make_win_rank.iter())
            {
                *dst |= prev | mwr;
            }
        }
        if cutoff {
            self.best_move[depth_u] = chosen_move;
        }
        value
    }

    // ------------------------------------------------------------------
    // Bisection driver
    // ------------------------------------------------------------------

    /// Drive the bisection loop to find the exact tricks MAX achieves
    /// from the position. Mirrors the inner bisection of
    /// `SolveSameBoard` in `SolverIF.cpp`.
    ///
    /// `pos.tricks_max` should be 0 at entry. The engine's `ini_depth`
    /// field is set to `ini_depth` before the loop runs; callers should
    /// set the deal via [`Engine::set_deal`] beforehand.
    pub(crate) fn search_target(
        &mut self,
        pos: &mut Pos,
        tt: &mut TransTable,
        ini_depth: i32,
    ) -> i32 {
        self.ini_depth = ini_depth;

        // Edge case: literally no cards in play. The vendor's
        // `SolveBoardInternal` short-circuits this case via
        // `LastTrickWinner` before ever calling `ABsearch`; do the same
        // here so `Evaluate` isn't asked to inspect an empty hand.
        let total_cards: i32 = (0..DDS_HANDS)
            .map(|h| (0..DDS_SUITS).map(|s| pos.length[h][s] as i32).sum::<i32>())
            .sum();
        if total_cards == 0 {
            return 0;
        }

        // Initialize the move generator's per-trick removed-ranks at
        // the trick index where the search starts.
        let start_trick = (ini_depth + 3) >> 2;
        self.moves.init_removed_ranks(start_trick, pos);

        let lead_hand = pos.first[ini_depth as usize];
        self.moves.reinit(start_trick, lead_hand);

        let mut lowerbound = 0i32;
        let mut upperbound = (ini_depth + 4) >> 2;
        if upperbound <= 0 {
            return 0;
        }

        while lowerbound < upperbound {
            let target = (lowerbound + upperbound + 1) / 2;
            self.reset_best_moves();
            let val = self.ab_search_0(pos, tt, target, ini_depth);
            if val {
                lowerbound = target;
            } else {
                upperbound = target - 1;
            }
        }

        lowerbound
    }
}

// ----------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lookup::BIT_MAP_RANK;

    /// Build a partial `Pos` from per-hand suit bitmaps; the helper
    /// populates `aggr` and `length` automatically.
    fn build_pos(rank_in_suit: [[u16; 4]; 4]) -> Pos {
        let mut p = Pos::default();
        p.rank_in_suit = rank_in_suit;
        for s in 0..4 {
            p.aggr[s] = (0..4).fold(0u16, |a, h| a | rank_in_suit[h][s]);
        }
        for h in 0..4 {
            for s in 0..4 {
                p.length[h][s] = rank_in_suit[h][s].count_ones() as u8;
            }
        }
        p
    }

    /// Total number of cards held (across all hands and suits).
    fn card_count(p: &Pos) -> i32 {
        let mut c = 0;
        for h in 0..4 {
            for s in 0..4 {
                c += p.length[h][s] as i32;
            }
        }
        c
    }

    #[test]
    fn empty_position_returns_zero() {
        let mut pos = Pos::default();
        let mut tt = TransTable::new();
        let mut eng = Engine::new(DDS_NOTRUMP);
        eng.set_deal(&mut pos, &mut tt);

        // ini_depth = 0 — no tricks remaining.
        let tricks = eng.search_target(&mut pos, &mut tt, 0);
        assert_eq!(tricks, 0);
    }

    #[test]
    fn one_winner_returns_one() {
        // MAX (North) holds the A of suit 0. Each other hand has a
        // single low card in a distinct suit. North leads → A wins.
        let mut rank = [[0u16; 4]; 4];
        rank[0][0] = BIT_MAP_RANK[14]; // N: AS
        rank[1][1] = BIT_MAP_RANK[2]; // E: 2H
        rank[2][2] = BIT_MAP_RANK[2]; // S: 2D
        rank[3][3] = BIT_MAP_RANK[2]; // W: 2C
        let mut pos = build_pos(rank);

        let cc = card_count(&pos);
        assert_eq!(cc, 4, "should be exactly 4 cards");
        let ini_depth = cc - 4; // 0

        // North leads.
        pos.first[ini_depth as usize] = 0;

        let mut tt = TransTable::new();
        let mut eng = Engine::new(DDS_NOTRUMP);
        // NS = MAX, EW = MIN.
        eng.set_node_types([MAXNODE, MINNODE, MAXNODE, MINNODE]);
        eng.set_deal(&mut pos, &mut tt);

        let tricks = eng.search_target(&mut pos, &mut tt, ini_depth);
        assert_eq!(tricks, 1, "MAX should win 1 trick with the A");
    }

    #[test]
    fn one_loser_returns_zero() {
        // MIN (East) holds the AH; everyone else has a single small
        // card. East leads → AH wins. MAX gets 0 tricks.
        let mut rank = [[0u16; 4]; 4];
        rank[0][0] = BIT_MAP_RANK[2]; // N: 2S
        rank[1][1] = BIT_MAP_RANK[14]; // E: AH
        rank[2][2] = BIT_MAP_RANK[2]; // S: 2D
        rank[3][3] = BIT_MAP_RANK[2]; // W: 2C
        let mut pos = build_pos(rank);

        let cc = card_count(&pos);
        assert_eq!(cc, 4);
        let ini_depth = cc - 4; // 0

        // East leads.
        pos.first[ini_depth as usize] = 1;

        let mut tt = TransTable::new();
        let mut eng = Engine::new(DDS_NOTRUMP);
        // East leads — by SolverIF convention, when handToPlay is E or W,
        // NS are still MAX (declarer's side). But to make this test
        // simpler we leave NS = MAX so East's AH win = MIN win.
        eng.set_node_types([MAXNODE, MINNODE, MAXNODE, MINNODE]);
        eng.set_deal(&mut pos, &mut tt);

        let tricks = eng.search_target(&mut pos, &mut tt, ini_depth);
        assert_eq!(tricks, 0, "MAX should win 0 tricks (MIN owns the A)");
    }

    #[test]
    fn small_real_position() {
        // 4 tricks per hand, 16 cards total. Each hand holds the A of
        // one suit plus 3 small cards in the other suits. With perfect
        // play, both sides claim their 2 As: MAX (NS) wins 2 tricks.
        //
        // Layout (suit indices 0=S, 1=H, 2=D, 3=C):
        //   N: AS, 2H, 2D, 2C
        //   E: 3S, AH, 3D, 3C
        //   S: 4S, 4H, AD, 4C
        //   W: 5S, 5H, 5D, AC
        let mut rank = [[0u16; 4]; 4];
        // N
        rank[0][0] = BIT_MAP_RANK[14];
        rank[0][1] = BIT_MAP_RANK[2];
        rank[0][2] = BIT_MAP_RANK[2];
        rank[0][3] = BIT_MAP_RANK[2];
        // E
        rank[1][0] = BIT_MAP_RANK[3];
        rank[1][1] = BIT_MAP_RANK[14];
        rank[1][2] = BIT_MAP_RANK[3];
        rank[1][3] = BIT_MAP_RANK[3];
        // S
        rank[2][0] = BIT_MAP_RANK[4];
        rank[2][1] = BIT_MAP_RANK[4];
        rank[2][2] = BIT_MAP_RANK[14];
        rank[2][3] = BIT_MAP_RANK[4];
        // W
        rank[3][0] = BIT_MAP_RANK[5];
        rank[3][1] = BIT_MAP_RANK[5];
        rank[3][2] = BIT_MAP_RANK[5];
        rank[3][3] = BIT_MAP_RANK[14];

        let mut pos = build_pos(rank);
        let cc = card_count(&pos);
        assert_eq!(cc, 16);
        let ini_depth = cc - 4; // 12

        // North leads.
        pos.first[ini_depth as usize] = 0;

        let mut tt = TransTable::new();
        let mut eng = Engine::new(DDS_NOTRUMP);
        eng.set_node_types([MAXNODE, MINNODE, MAXNODE, MINNODE]);
        eng.set_deal(&mut pos, &mut tt);

        let tricks = eng.search_target(&mut pos, &mut tt, ini_depth);
        assert_eq!(
            tricks, 2,
            "NS should win exactly 2 tricks (the two aces they hold)"
        );
    }
}
