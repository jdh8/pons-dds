//! Late-game forced-result detection.
//!
//! Ported field-for-field from
//! [`LaterTricks.cpp`](../../../ddss-sys/vendor/src/LaterTricks.cpp).
//!
//! Called from `ABsearch0` after [`crate::quick_tricks::quick_tricks`]
//! fails to short-circuit. Both `LaterTricksMIN` and `LaterTricksMAX`
//! return a boolean; the convention is:
//!
//! - [`later_tricks_min`] returns `false` to signal that MIN cannot
//!   prevent the search target from being reached (a forced concede),
//!   `true` to continue the search.
//! - [`later_tricks_max`] returns `true` to signal that MAX can
//!   guarantee reaching the target (a forced claim), `false` to
//!   continue.
//!
//! See the vendor source for the precise meaning of each branch — this
//! is one of the most position-sensitive heuristics in the solver.

use crate::lookup::{BIT_MAP_RANK, LHO, PARTNER, RHO};
use crate::pos::Pos;
use crate::quick_tricks::{DDS_NOTRUMP, MAXNODE, MINNODE};

const DDS_SUITS: usize = 4;
const DDS_HANDS: usize = 4;

/// Find the (rank, hand) of the `k`-th highest card in `aggr` for
/// `suit`. Mirrors the vendor's `thrd.rel[aggr].absRank[k][suit]` table
/// lookup. `k` is 1-based. Returns `(0, -1)` if there's no such card.
fn abs_rank(rank_in_suit: &[[u16; 4]; 4], aggr: u16, k: usize, suit: usize) -> (i32, i32) {
    let mut count = 0usize;
    for r in (2i32..=14).rev() {
        let bit = BIT_MAP_RANK[r as usize];
        if aggr & bit != 0 {
            count += 1;
            if count == k {
                for (h, hand_ranks) in rank_in_suit.iter().enumerate() {
                    if hand_ranks[suit] & bit != 0 {
                        return (r, h as i32);
                    }
                }
                return (r, -1);
            }
        }
    }
    (0, -1)
}

/// MIN-side end-game pruning. Mirrors the vendor's `LaterTricksMIN`.
///
/// Returns `false` to indicate that MIN can NOT reach `target` from
/// this position (so the search can stop and report failure), `true` to
/// indicate the search must continue.
#[inline]
pub fn later_tricks_min(
    tpos: &mut Pos,
    hand: i32,
    depth: i32,
    target: i32,
    trump: i32,
    node_type_store: &[i32; DDS_HANDS],
) -> bool {
    let hand_u = hand as usize;
    let depth_u = depth as usize;

    if trump == DDS_NOTRUMP || tpos.winner[trump as usize].rank == 0 {
        let mut sum = 0;
        for ss in 0..DDS_SUITS {
            let hh = tpos.winner[ss].hand;
            if hh != -1 && node_type_store[hh as usize] == MAXNODE {
                sum += i32::from(tpos.length[hh as usize][ss])
                    .max(i32::from(tpos.length[PARTNER[hh as usize]][ss]));
            }
        }

        if tpos.tricks_max + sum < target && sum > 0 {
            if tpos.tricks_max + (depth >> 2) >= target {
                return true;
            }

            for ss in 0..DDS_SUITS {
                let win_hand = tpos.winner[ss].hand;
                if win_hand == -1 {
                    tpos.win_ranks[depth_u][ss] = 0;
                } else if node_type_store[win_hand as usize] == MINNODE {
                    if tpos.rank_in_suit[PARTNER[win_hand as usize]][ss] == 0
                        && tpos.rank_in_suit[LHO[win_hand as usize]][ss] == 0
                        && tpos.rank_in_suit[RHO[win_hand as usize]][ss] == 0
                    {
                        tpos.win_ranks[depth_u][ss] = 0;
                    } else {
                        tpos.win_ranks[depth_u][ss] = BIT_MAP_RANK[tpos.winner[ss].rank as usize];
                    }
                } else {
                    tpos.win_ranks[depth_u][ss] = 0;
                }
            }
            return false;
        }
    } else if node_type_store[tpos.winner[trump as usize].hand as usize] == MINNODE {
        if tpos.length[hand_u][trump as usize] == 0
            && tpos.length[PARTNER[hand_u]][trump as usize] == 0
        {
            if tpos.tricks_max + (depth >> 2) + 1
                - i32::from(tpos.length[LHO[hand_u]][trump as usize])
                    .max(i32::from(tpos.length[RHO[hand_u]][trump as usize]))
                < target
            {
                for ss in 0..DDS_SUITS {
                    tpos.win_ranks[depth_u][ss] = 0;
                }
                return false;
            }
        } else if tpos.tricks_max + (depth >> 2) < target {
            for ss in 0..DDS_SUITS {
                tpos.win_ranks[depth_u][ss] = 0;
            }
            tpos.win_ranks[depth_u][trump as usize] =
                BIT_MAP_RANK[tpos.winner[trump as usize].rank as usize];
            return false;
        } else if tpos.tricks_max + (depth >> 2) == target {
            let hh = tpos.second_best[trump as usize].hand;
            if hh == -1 {
                return true;
            }

            let r2 = tpos.second_best[trump as usize].rank;
            if node_type_store[hh as usize] == MINNODE && r2 != 0
                && (tpos.length[hh as usize][trump as usize] > 1
                    || tpos.length[PARTNER[hh as usize]][trump as usize] > 1)
                {
                    for ss in 0..DDS_SUITS {
                        tpos.win_ranks[depth_u][ss] = 0;
                    }
                    tpos.win_ranks[depth_u][trump as usize] = BIT_MAP_RANK[r2 as usize];
                    return false;
                }
        }
    } else {
        // Not NT.
        let hh = tpos.second_best[trump as usize].hand;
        if hh == -1 {
            return true;
        }

        if node_type_store[hh as usize] != MINNODE || tpos.length[hh as usize][trump as usize] <= 1
        {
            return true;
        }

        if tpos.winner[trump as usize].hand == RHO[hh as usize] as i32 {
            if tpos.tricks_max + (depth >> 2) < target {
                for ss in 0..DDS_SUITS {
                    tpos.win_ranks[depth_u][ss] = 0;
                }
                tpos.win_ranks[depth_u][trump as usize] =
                    BIT_MAP_RANK[tpos.second_best[trump as usize].rank as usize];
                return false;
            }
        } else {
            let aggr = tpos.aggr[trump as usize];
            let (third_rank, h) = abs_rank(&tpos.rank_in_suit, aggr, 3, trump as usize);
            if h == -1 {
                return true;
            }

            if node_type_store[h as usize] == MINNODE && tpos.tricks_max + (depth >> 2) < target {
                for ss in 0..DDS_SUITS {
                    tpos.win_ranks[depth_u][ss] = 0;
                }
                tpos.win_ranks[depth_u][trump as usize] = BIT_MAP_RANK[third_rank as usize];
                return false;
            }
        }
    }
    true
}

/// MAX-side end-game pruning. Mirrors the vendor's `LaterTricksMAX`.
///
/// Returns `true` to indicate that MAX can guarantee reaching `target`
/// from this position (so the search can stop and report success),
/// `false` to indicate the search must continue.
#[inline]
pub fn later_tricks_max(
    tpos: &mut Pos,
    hand: i32,
    depth: i32,
    target: i32,
    trump: i32,
    node_type_store: &[i32; DDS_HANDS],
) -> bool {
    let hand_u = hand as usize;
    let depth_u = depth as usize;

    if trump == DDS_NOTRUMP || tpos.winner[trump as usize].rank == 0 {
        let mut sum = 0;
        for ss in 0..DDS_SUITS {
            let hh = tpos.winner[ss].hand;
            if hh != -1 && node_type_store[hh as usize] == MINNODE {
                sum += i32::from(tpos.length[hh as usize][ss])
                    .max(i32::from(tpos.length[PARTNER[hh as usize]][ss]));
            }
        }

        if tpos.tricks_max + (depth >> 2) + 1 - sum >= target && sum > 0 {
            if tpos.tricks_max + 1 < target {
                return false;
            }

            for ss in 0..DDS_SUITS {
                let win_hand = tpos.winner[ss].hand;
                if win_hand == -1 {
                    tpos.win_ranks[depth_u][ss] = 0;
                } else if node_type_store[win_hand as usize] == MAXNODE {
                    if tpos.rank_in_suit[PARTNER[win_hand as usize]][ss] == 0
                        && tpos.rank_in_suit[LHO[win_hand as usize]][ss] == 0
                        && tpos.rank_in_suit[RHO[win_hand as usize]][ss] == 0
                    {
                        tpos.win_ranks[depth_u][ss] = 0;
                    } else {
                        tpos.win_ranks[depth_u][ss] = BIT_MAP_RANK[tpos.winner[ss].rank as usize];
                    }
                } else {
                    tpos.win_ranks[depth_u][ss] = 0;
                }
            }
            return true;
        }
    } else if node_type_store[tpos.winner[trump as usize].hand as usize] == MAXNODE {
        if tpos.length[hand_u][trump as usize] == 0
            && tpos.length[PARTNER[hand_u]][trump as usize] == 0
        {
            let maxlen = i32::from(tpos.length[LHO[hand_u]][trump as usize])
                .max(i32::from(tpos.length[RHO[hand_u]][trump as usize]));

            if tpos.tricks_max + maxlen >= target {
                for ss in 0..DDS_SUITS {
                    tpos.win_ranks[depth_u][ss] = 0;
                }
                return true;
            }
        } else if tpos.tricks_max + 1 >= target {
            for ss in 0..DDS_SUITS {
                tpos.win_ranks[depth_u][ss] = 0;
            }
            tpos.win_ranks[depth_u][trump as usize] =
                BIT_MAP_RANK[tpos.winner[trump as usize].rank as usize];
            return true;
        } else {
            let hh = tpos.second_best[trump as usize].hand;
            if hh == -1 {
                return false;
            }

            if node_type_store[hh as usize] == MAXNODE && tpos.second_best[trump as usize].rank != 0
                && (tpos.length[hh as usize][trump as usize] > 1
                    || tpos.length[PARTNER[hh as usize]][trump as usize] > 1)
                    && tpos.tricks_max + 2 >= target
                {
                    for ss in 0..DDS_SUITS {
                        tpos.win_ranks[depth_u][ss] = 0;
                    }
                    tpos.win_ranks[depth_u][trump as usize] =
                        BIT_MAP_RANK[tpos.second_best[trump as usize].rank as usize];
                    return true;
                }
        }
    } else {
        // trump != DDS_NOTRUMP.
        let hh = tpos.second_best[trump as usize].hand;
        if hh == -1 {
            return false;
        }

        if node_type_store[hh as usize] != MAXNODE || tpos.length[hh as usize][trump as usize] <= 1
        {
            return false;
        }

        if tpos.winner[trump as usize].hand == RHO[hh as usize] as i32 {
            if tpos.tricks_max + 1 >= target {
                for ss in 0..DDS_SUITS {
                    tpos.win_ranks[depth_u][ss] = 0;
                }
                tpos.win_ranks[depth_u][trump as usize] =
                    BIT_MAP_RANK[tpos.second_best[trump as usize].rank as usize];
                return true;
            }
        } else {
            let aggr = tpos.aggr[trump as usize];
            let (third_rank, h) = abs_rank(&tpos.rank_in_suit, aggr, 3, trump as usize);
            if h == -1 {
                return false;
            }

            if node_type_store[h as usize] == MAXNODE && tpos.tricks_max + 1 >= target {
                for ss in 0..DDS_SUITS {
                    tpos.win_ranks[depth_u][ss] = 0;
                }
                tpos.win_ranks[depth_u][trump as usize] = BIT_MAP_RANK[third_rank as usize];
                return true;
            }
        }
    }
    false
}

/// Wrapper around [`later_tricks_min`] / [`later_tricks_max`] returning
/// an [`Option`]. `Some(tricks_max)` means the result is forced (the
/// search can stop and read off the trick count); `None` means the
/// search must continue.
///
/// The helper consults `node_type_store[hand]` to pick the right
/// underlying function. For the MAX side, a forced result is a claim;
/// for the MIN side, a forced result is a concede.
#[allow(dead_code)]
pub fn later_tricks(
    tpos: &mut Pos,
    hand: i32,
    depth: i32,
    target: i32,
    trump: i32,
    node_type_store: &[i32; DDS_HANDS],
) -> Option<i32> {
    if node_type_store[hand as usize] == MAXNODE {
        // For MAX, LaterTricksMIN returning false indicates MIN cannot
        // prevent the target (a forced concede from MIN's side).
        if !later_tricks_min(tpos, hand, depth, target, trump, node_type_store) {
            // The exact value isn't important here for v0.1 — the
            // vendor's caller treats this as "result determined" and
            // returns the success-flag accordingly.
            return Some(tpos.tricks_max);
        }
        if later_tricks_max(tpos, hand, depth, target, trump, node_type_store) {
            return Some(tpos.tricks_max);
        }
    } else {
        if later_tricks_max(tpos, hand, depth, target, trump, node_type_store) {
            return Some(tpos.tricks_max);
        }
        if !later_tricks_min(tpos, hand, depth, target, trump, node_type_store) {
            return Some(tpos.tricks_max);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::move_type::HighCard;

    const NODE_TYPE: [i32; 4] = [MAXNODE, MINNODE, MAXNODE, MINNODE];

    /// A trivial "no winners" position. With every winner having
    /// `hand = -1`, neither LaterTricksMIN nor LaterTricksMAX should
    /// fire — the `Option` wrapper should return `None` and the search
    /// must continue.
    #[test]
    fn no_winners_continues_search() {
        let mut pos = Pos::default();
        // All four suits: no winner.
        for s in 0..4 {
            pos.winner[s] = HighCard { rank: 0, hand: -1 };
            pos.second_best[s] = HighCard { rank: 0, hand: -1 };
        }

        let result = later_tricks(
            &mut pos,
            0,
            44,
            13,
            DDS_NOTRUMP, // NT, no trump winner
            &NODE_TYPE,
        );

        assert_eq!(result, None, "no winners → search must continue");
    }

    /// LaterTricksMAX in NT: MIN owns the only suit winner (a single
    /// stopper) and MAX can ride out the rest. Verify the heuristic
    /// announces a forced claim.
    ///
    /// Construct:
    ///  - Suit 0: winner is MIN (hand 1, length 1). MIN's partner
    ///    (hand 3) also has length 1. So sum = max(1, 1) = 1.
    ///  - tricks_max = 12, target = 13. With depth = 4 → depth>>2 = 1.
    ///    Check `tricks_max + 1 + 1 - 1 >= target` → 13 >= 13. Passes.
    ///    Also `tricks_max + 1 >= target` (the early-exit guard) — 13 >= 13.
    #[test]
    fn max_claim_in_nt() {
        let mut pos = Pos::default();
        // MIN holds the suit-0 winner.
        pos.winner[0] = HighCard { rank: 14, hand: 1 };
        pos.length[1][0] = 1; // MIN length
        pos.length[3][0] = 1; // MIN's partner length
        for s in 1..4 {
            pos.winner[s] = HighCard { rank: 0, hand: -1 };
            pos.second_best[s] = HighCard { rank: 0, hand: -1 };
        }
        pos.second_best[0] = HighCard { rank: 0, hand: -1 };

        // Set rank_in_suit so the secondary "no other holders" check
        // does NOT zero the win_ranks (we want a defined behavior).
        pos.rank_in_suit[1][0] = 0x1000; // A in MIN's hand.

        pos.tricks_max = 12;

        // hand = 0 (MAX side), depth = 4 so depth>>2 = 1.
        let result = later_tricks(&mut pos, 0, 4, 13, DDS_NOTRUMP, &NODE_TYPE);

        assert!(
            result.is_some(),
            "MAX claim should be detected: result was {result:?}"
        );
    }

    /// One-trick-left forced result. Trump (suit 0) is in play; MAX
    /// owns the trump winner with `length[hand][trump] != 0`. With
    /// `tricks_max + 1 >= target`, [`later_tricks_max`] takes the
    /// "trump winner at MAX" early-exit and returns `true`.
    #[test]
    fn one_trick_left_forced_claim() {
        let mut pos = Pos::default();
        // Trump = suit 0. MAX (hand 0) holds the A of trump.
        pos.rank_in_suit[0][0] = 0x1000;
        pos.length[0][0] = 1;
        pos.aggr[0] = 0x1000;
        pos.winner[0] = HighCard { rank: 14, hand: 0 };
        pos.second_best[0] = HighCard { rank: 0, hand: -1 };
        // Other suits: empty / unused.
        for s in 1..4 {
            pos.winner[s] = HighCard { rank: 0, hand: -1 };
            pos.second_best[s] = HighCard { rank: 0, hand: -1 };
        }

        // Only one more trick to win.
        pos.tricks_max = 12;

        // Trump = 0 (not NT). Depth doesn't matter much; pick 0.
        let result = later_tricks(&mut pos, 0, 0, 13, 0, &NODE_TYPE);

        assert!(
            result.is_some(),
            "forced one-trick claim should be detected: result was {result:?}"
        );
    }
}
