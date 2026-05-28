//! Move representation used during search.
//!
//! Ported from `moveType` / `movePlyType` / `highCardType` in
//! [`dds.h`](../../../ddss-sys/vendor/src/dds.h).

/// A candidate move under consideration by the search.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MoveType {
    pub suit: i32,
    pub rank: i32,
    /// Non-zero if this move is the first in a sequence of consecutive
    /// ranks (e.g. the K when the played hand holds KQJ). The
    /// move-generator collapses equivalent moves so that only one card
    /// from each run reaches the search.
    pub sequence: i32,
    /// Heuristic priority used by [`crate::moves`]'s merge-sort. Higher
    /// is searched first.
    pub weight: i32,
}

/// Fixed-capacity move buffer for a single ply of the search.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MovePly {
    pub moves: [MoveType; 14],
    /// Index of the next move to return from `MakeNext`. Bumped on
    /// every successful candidate fetch.
    pub current: i32,
    /// Index of the last valid move in `moves`.
    pub last: i32,
}

impl Default for MovePly {
    fn default() -> Self {
        Self {
            moves: [MoveType::default(); 14],
            current: 0,
            last: -1,
        }
    }
}

/// Vendor's `highCardType` — a (rank, hand) pair identifying a card.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HighCard {
    pub rank: i32,
    pub hand: i32,
}
