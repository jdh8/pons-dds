//! Per-search position state.
//!
//! Ported field-for-field from the vendor's `struct pos` in
//! [`dds.h`](../../../ddss-sys/vendor/src/dds.h). This struct is read
//! and mutated by every recursive call into the alpha-beta search, so
//! its layout is performance-critical — both for cache locality and
//! for keeping the per-frame stack working set small.
//!
//! Fields are `pub(crate)` so the `moves`, `tt`, `quick_tricks`,
//! `later_tricks`, and `search` modules can write to them directly,
//! mirroring the vendor's flat C-style access without forcing accessor
//! methods.

use crate::move_type::{HighCard, MoveType};

/// Maximum search depth (52 cards / 4 hands + slack for indexing).
/// The vendor uses 50 as the upper bound for depth-indexed arrays.
pub(crate) const MAX_DEPTH: usize = 50;

/// Per-search position state. One copy per [`crate::solver::Solver`]
/// instance; mutated in place during the recursive search.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Pos {
    // ---- Card state ------------------------------------------------
    /// `rank_in_suit[hand][suit]` — bitmap of which ranks (2..=14, bits
    /// 2..=14 of a u16) `hand` holds in `suit`.
    pub rank_in_suit: [[u16; 4]; 4],
    /// `aggr[suit]` — union of `rank_in_suit[h][suit]` across all
    /// hands. Used as an index into the precomputed 8192-entry tables.
    pub aggr: [u16; 4],
    /// `length[hand][suit]` — popcount of `rank_in_suit[hand][suit]`,
    /// in 0..=13.
    pub length: [[u8; 4]; 4],
    /// `hand_dist[hand]` — packed distribution hash used as a TT key.
    pub hand_dist: [i32; 4],

    // ---- Depth-indexed history ------------------------------------
    /// `win_ranks[depth][suit]` — bitmap of cards that have already
    /// won tricks by being the highest card played at `depth`. Used by
    /// `LaterTricks` for late-game forced-result detection.
    pub win_ranks: [[u16; 4]; MAX_DEPTH],
    /// `first[depth]` — hand that leads the trick whose top ply is
    /// `depth`.
    pub first: [i32; MAX_DEPTH],
    /// `move_history[depth]` — the currently-winning move at `depth`.
    /// Named `move` in the vendor but renamed here to avoid the
    /// Rust keyword.
    pub move_history: [MoveType; MAX_DEPTH],

    // ---- Current ply state ----------------------------------------
    /// Position of the current hand relative to the trick's leader,
    /// in 0..=3. 0 means the current hand IS the leader.
    pub hand_rel_first: i32,
    /// Tricks won so far by the MAX player on this branch of the
    /// search.
    pub tricks_max: i32,

    // ---- Per-suit running state -----------------------------------
    /// `winner[suit]` — the highest card globally in `suit`.
    pub winner: [HighCard; 4],
    /// `second_best[suit]` — the second-highest card globally in
    /// `suit`. Together with `winner` this drives quick-tricks
    /// heuristics.
    pub second_best: [HighCard; 4],
}

impl Default for Pos {
    fn default() -> Self {
        Self {
            rank_in_suit: [[0; 4]; 4],
            aggr: [0; 4],
            length: [[0; 4]; 4],
            hand_dist: [0; 4],
            win_ranks: [[0; 4]; MAX_DEPTH],
            first: [0; MAX_DEPTH],
            move_history: [MoveType::default(); MAX_DEPTH],
            hand_rel_first: 0,
            tricks_max: 0,
            winner: [HighCard::default(); 4],
            second_best: [HighCard::default(); 4],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_zero() {
        let p = Pos::default();
        assert_eq!(p.rank_in_suit, [[0; 4]; 4]);
        assert_eq!(p.aggr, [0; 4]);
        assert_eq!(p.tricks_max, 0);
        assert_eq!(p.hand_rel_first, 0);
    }
}
