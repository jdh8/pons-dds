//! Sure-tricks pre-search pruning.
//!
//! Ported field-for-field from
//! [`QuickTricks.cpp`](../../../ddss-sys/vendor/src/QuickTricks.cpp).
//!
//! Called from `ABsearch0` (lead-hand entry) and `ABsearch1`
//! (second-hand entry) before move generation. Detects positions where
//! the MAX side can immediately claim enough tricks to satisfy the
//! search's `target`, short-circuiting the recursion.
//!
//! The four internal helpers
//! ([`qtricks_lead_hand_nt`], [`qtricks_lead_hand_trump`],
//! [`quick_tricks_partner_hand_nt`], [`quick_tricks_partner_hand_trump`])
//! correspond one-to-one with the vendor's static helpers; each carries
//! the same "res = 0/1/2 = continue-same-suit / cutoff / continue-next-suit"
//! convention as the vendor source.

use crate::lookup::{BIT_MAP_RANK, HIGHEST_RANK, LHO, PARTNER, RHO};
use crate::moves::RelRanks;
use crate::pos::Pos;

/// MAXNODE constant from the vendor (`#define MAXNODE 1`).
pub const MAXNODE: i32 = 1;

/// MINNODE constant from the vendor (`#define MINNODE 0`).
#[allow(dead_code)]
pub const MINNODE: i32 = 0;

/// `DDS_NOTRUMP` constant (`#define DDS_NOTRUMP 4`).
pub const DDS_NOTRUMP: i32 = 4;

/// Number of suits (`#define DDS_SUITS 4`).
const DDS_SUITS: usize = 4;

/// Number of hands (`#define DDS_HANDS 4`).
const DDS_HANDS: usize = 4;

/// Find the (rank, hand) of the `k`-th highest card present in `aggr`
/// for `suit` — the vendor's `thrd.rel[aggr].absRank[k][suit]`, one
/// load into the per-deal table.
///
/// Returns `(0, -1)` if no such card exists. `k` is 1-based: `k = 1` is
/// the highest, `k = 2` the second-highest, etc.
#[inline]
fn abs_rank(rel: &[RelRanks], aggr: u16, k: usize, suit: usize) -> (i32, i32) {
    let entry = rel[aggr as usize].abs_rank[k][suit];
    (entry.rank, entry.hand)
}

/// Sure-tricks heuristic. Mirror of the vendor's `QuickTricks`.
///
/// Returns the count of tricks MAX can immediately claim from this
/// position; writes `*success = true` if the count is sufficient to
/// reach `target`, `*success = false` if the heuristic could not
/// guarantee a result and the search must continue.
///
/// `node_type_store[h]` must be `MAXNODE` or `MINNODE` for each hand
/// `h`; this is the per-deal MAX/MIN assignment from the vendor's
/// `thrd.nodeTypeStore`.
#[inline]
#[allow(clippy::too_many_arguments)]
pub fn quick_tricks(
    tpos: &mut Pos,
    hand: i32,
    depth: i32,
    target: i32,
    trump: i32,
    result: &mut bool,
    node_type_store: &[i32; DDS_HANDS],
    rel: &[RelRanks],
) -> i32 {
    let hand_u = hand as usize;
    let depth_u = depth as usize;

    let mut comm_rank: i32 = 0;
    let mut comm_suit: i32 = -1;
    let mut lho_trump_ranks: i32;
    let mut rho_trump_ranks: i32;
    let mut lowest_qtricks = 0;

    *result = true;
    let mut qtricks = 0;

    let cutoff_initial = if node_type_store[hand_u] == MAXNODE {
        target - tpos.tricks_max
    } else {
        tpos.tricks_max - target + (depth >> 2) + 2
    };
    let mut cutoff = cutoff_initial;

    let mut comm_partner = false;

    for s in 0..DDS_SUITS {
        if trump != DDS_NOTRUMP && trump != s as i32 {
            // Trump game, and we lead a non-trump suit.
            let trump_u = trump as usize;
            if tpos.winner[s].hand == PARTNER[hand_u] as i32 {
                // Partner has winning card.
                if tpos.rank_in_suit[hand_u][s] != 0
                    && ((tpos.rank_in_suit[LHO[hand_u]][s] != 0
                        || tpos.rank_in_suit[LHO[hand_u]][trump_u] == 0)
                        && (tpos.rank_in_suit[RHO[hand_u]][s] != 0
                            || tpos.rank_in_suit[RHO[hand_u]][trump_u] == 0))
                {
                    comm_partner = true;
                    comm_suit = s as i32;
                    comm_rank = tpos.winner[s].rank;
                    break;
                }
            } else if tpos.second_best[s].hand == PARTNER[hand_u] as i32
                && tpos.winner[s].hand == hand
                && tpos.length[hand_u][s] >= 2
                && tpos.length[PARTNER[hand_u]][s] >= 2
            {
                // Can cross to partner's card: Type Kx opposite Ax.
                if (tpos.rank_in_suit[LHO[hand_u]][s] != 0
                    || tpos.rank_in_suit[LHO[hand_u]][trump_u] == 0)
                    && (tpos.rank_in_suit[RHO[hand_u]][s] != 0
                        || tpos.rank_in_suit[RHO[hand_u]][trump_u] == 0)
                {
                    comm_partner = true;
                    comm_suit = s as i32;
                    comm_rank = tpos.second_best[s].rank;
                    break;
                }
            }
        } else if trump == DDS_NOTRUMP {
            if tpos.winner[s].hand == PARTNER[hand_u] as i32 {
                // Partner has winning card in NT.
                if tpos.rank_in_suit[hand_u][s] != 0 {
                    comm_partner = true;
                    comm_suit = s as i32;
                    comm_rank = tpos.winner[s].rank;
                    break;
                }
            } else if tpos.second_best[s].hand == PARTNER[hand_u] as i32
                && tpos.winner[s].hand == hand
                && tpos.length[hand_u][s] >= 2
                && tpos.length[PARTNER[hand_u]][s] >= 2
            {
                comm_partner = true;
                comm_suit = s as i32;
                comm_rank = tpos.second_best[s].rank;
                break;
            }
        }
    }

    if trump != DDS_NOTRUMP
        && !comm_partner
        && tpos.rank_in_suit[hand_u][trump as usize] != 0
        && tpos.winner[trump as usize].hand == PARTNER[hand_u] as i32
    {
        // Communication in trump suit.
        comm_partner = true;
        comm_suit = trump;
        comm_rank = tpos.winner[trump as usize].rank;
    }

    let mut suit: i32;
    if trump == DDS_NOTRUMP {
        suit = 0;
        lho_trump_ranks = 0;
        rho_trump_ranks = 0;
    } else {
        suit = trump;
        lho_trump_ranks = i32::from(tpos.length[LHO[hand_u]][trump as usize]);
        rho_trump_ranks = i32::from(tpos.length[RHO[hand_u]][trump as usize]);
    }

    loop {
        let suit_u = suit as usize;
        let count_own = i32::from(tpos.length[hand_u][suit_u]);
        let count_lho = i32::from(tpos.length[LHO[hand_u]][suit_u]);
        let count_rho = i32::from(tpos.length[RHO[hand_u]][suit_u]);
        let count_part = i32::from(tpos.length[PARTNER[hand_u]][suit_u]);
        let opps = count_lho | count_rho;

        if opps == 0 && count_part == 0 {
            if count_own == 0 {
                // Continue with next suit.
                if trump != DDS_NOTRUMP && trump != suit {
                    suit += 1;
                    if trump != DDS_NOTRUMP && suit == trump {
                        suit += 1;
                    }
                } else if trump != DDS_NOTRUMP && trump == suit {
                    suit = i32::from(trump == 0);
                } else {
                    suit += 1;
                    if trump != DDS_NOTRUMP && suit == trump {
                        suit += 1;
                    }
                }
                if suit > 3 {
                    break;
                }
                continue;
            }

            // Long tricks when only leading hand has cards in the suit.
            if trump != DDS_NOTRUMP && trump != suit {
                if lho_trump_ranks == 0 && rho_trump_ranks == 0 {
                    qtricks += count_own;
                    if qtricks >= cutoff {
                        return qtricks;
                    }
                }
                suit += 1;
                if trump != DDS_NOTRUMP && suit == trump {
                    suit += 1;
                }
                if suit > 3 {
                    break;
                }
                continue;
            }
            qtricks += count_own;
            if qtricks >= cutoff {
                return qtricks;
            }

            if trump != DDS_NOTRUMP && suit == trump {
                suit = i32::from(trump == 0);
            } else {
                suit += 1;
                if trump != DDS_NOTRUMP && suit == trump {
                    suit += 1;
                }
            }
            if suit > 3 {
                break;
            }
            continue;
        }
        if opps == 0 && trump != DDS_NOTRUMP && suit == trump {
            // The partner but not the opponents have cards in
            // the trump suit.
            let mut sum = count_own.max(count_part);
            for s in 0..DDS_SUITS {
                if sum > 0
                    && s as i32 != trump
                    && count_own >= count_part
                    && tpos.length[hand_u][s] > 0
                    && tpos.length[PARTNER[hand_u]][s] == 0
                {
                    sum += 1;
                    break;
                }
            }
            // If the additional trick by ruffing causes a cutoff
            // (qtricks not incremented).
            if sum >= cutoff {
                return sum;
            }
        } else if opps == 0 {
            // The partner but not the opponents have cards in the suit.
            let sum = count_own.min(count_part);
            if trump == DDS_NOTRUMP {
                if sum >= cutoff {
                    return sum;
                }
            } else if suit != trump && lho_trump_ranks == 0 && rho_trump_ranks == 0 && sum >= cutoff
            {
                return sum;
            }
        }

        if comm_partner {
            if opps == 0 && count_own == 0 {
                if trump != DDS_NOTRUMP && trump != suit {
                    if lho_trump_ranks == 0 && rho_trump_ranks == 0 {
                        qtricks += count_part;
                        tpos.win_ranks[depth_u][comm_suit as usize] |=
                            BIT_MAP_RANK[comm_rank as usize];

                        if qtricks >= cutoff {
                            return qtricks;
                        }
                    }
                    suit += 1;
                    if trump != DDS_NOTRUMP && suit == trump {
                        suit += 1;
                    }
                    if suit > 3 {
                        break;
                    }
                    continue;
                }
                qtricks += count_part;
                tpos.win_ranks[depth_u][comm_suit as usize] |= BIT_MAP_RANK[comm_rank as usize];

                if qtricks >= cutoff {
                    return qtricks;
                }

                if trump != DDS_NOTRUMP && suit == trump {
                    suit = i32::from(trump == 0);
                } else {
                    suit += 1;
                    if trump != DDS_NOTRUMP && suit == trump {
                        suit += 1;
                    }
                }
                if suit > 3 {
                    break;
                }
                continue;
            } else if opps == 0 && trump != DDS_NOTRUMP && suit == trump {
                let mut sum = count_own.max(count_part);
                for s in 0..DDS_SUITS {
                    if sum > 0
                        && s as i32 != trump
                        && count_own <= count_part
                        && tpos.length[PARTNER[hand_u]][s] > 0
                        && tpos.length[hand_u][s] == 0
                    {
                        sum += 1;
                        break;
                    }
                }
                if sum >= cutoff {
                    tpos.win_ranks[depth_u][comm_suit as usize] |= BIT_MAP_RANK[comm_rank as usize];
                    return sum;
                }
            } else if opps == 0 {
                let sum = count_own.min(count_part);
                if trump == DDS_NOTRUMP {
                    if sum >= cutoff {
                        return sum;
                    }
                } else if suit != trump
                    && lho_trump_ranks == 0
                    && rho_trump_ranks == 0
                    && sum >= cutoff
                {
                    return sum;
                }
            }
        }

        if tpos.winner[suit_u].rank == 0 {
            if trump != DDS_NOTRUMP && suit == trump {
                suit = i32::from(trump == 0);
            } else {
                suit += 1;
                if trump != DDS_NOTRUMP && suit == trump {
                    suit += 1;
                }
            }
            if suit > 3 {
                break;
            }
            continue;
        }

        if tpos.winner[suit_u].hand == hand {
            let mut res = 0;
            if trump != DDS_NOTRUMP && trump != suit {
                qtricks = qtricks_lead_hand_trump(
                    tpos,
                    cutoff,
                    depth,
                    count_lho,
                    count_rho,
                    lho_trump_ranks,
                    rho_trump_ranks,
                    count_own,
                    count_part,
                    suit,
                    qtricks,
                    &mut res,
                    hand,
                );

                if res == 1 {
                    return qtricks;
                } else if res == 2 {
                    suit += 1;
                    if trump != DDS_NOTRUMP && suit == trump {
                        suit += 1;
                    }
                    if suit > 3 {
                        break;
                    }
                    continue;
                }
            } else {
                qtricks = qtricks_lead_hand_nt(
                    tpos,
                    cutoff,
                    depth,
                    count_lho,
                    count_rho,
                    &mut lho_trump_ranks,
                    &mut rho_trump_ranks,
                    comm_partner,
                    comm_suit,
                    count_own,
                    count_part,
                    suit,
                    qtricks,
                    trump,
                    &mut res,
                    hand,
                );

                if res == 1 {
                    return qtricks;
                } else if res == 2 {
                    if trump != DDS_NOTRUMP && trump == suit {
                        suit = i32::from(trump == 0);
                    } else {
                        suit += 1;
                    }
                    if suit > 3 {
                        break;
                    }
                    continue;
                }
            }
        } else {
            // It was not possible to take a quick trick by own winning
            // card in the suit. Partner winning card?
            if tpos.winner[suit_u].hand == PARTNER[hand_u] as i32 {
                // Winner found at partner.
                if comm_partner {
                    // There is communication with the partner.
                    let mut res = 0;
                    if trump != DDS_NOTRUMP && trump != suit {
                        qtricks = quick_tricks_partner_hand_trump(
                            tpos,
                            cutoff,
                            depth,
                            count_lho,
                            count_rho,
                            lho_trump_ranks,
                            rho_trump_ranks,
                            count_own,
                            count_part,
                            suit,
                            qtricks,
                            comm_suit,
                            comm_rank,
                            &mut res,
                            hand,
                            rel,
                        );

                        if res == 1 {
                            return qtricks;
                        } else if res == 2 {
                            suit += 1;
                            if trump != DDS_NOTRUMP && suit == trump {
                                suit += 1;
                            }
                            if suit > 3 {
                                break;
                            }
                            continue;
                        }
                    } else {
                        qtricks = quick_tricks_partner_hand_nt(
                            tpos, cutoff, depth, count_lho, count_rho, count_own, count_part, suit,
                            qtricks, comm_suit, comm_rank, &mut res, hand, rel,
                        );

                        if res == 1 {
                            return qtricks;
                        } else if res == 2 {
                            if trump != DDS_NOTRUMP && trump == suit {
                                suit = i32::from(trump == 0);
                            } else {
                                suit += 1;
                            }
                            if suit > 3 {
                                break;
                            }
                            continue;
                        }
                    }
                }
            }
        }

        if trump != DDS_NOTRUMP
            && suit != trump
            && count_own > 0
            && lowest_qtricks == 0
            && (qtricks == 0
                || (tpos.winner[suit_u].hand != hand
                    && tpos.winner[suit_u].hand != PARTNER[hand_u] as i32
                    && tpos.winner[trump as usize].hand != hand
                    && tpos.winner[trump as usize].hand != PARTNER[hand_u] as i32))
            && count_part == 0
            && tpos.length[PARTNER[hand_u]][trump as usize] > 0
        {
            if (count_rho > 0 || tpos.length[RHO[hand_u]][trump as usize] == 0)
                && (count_lho > 0 || tpos.length[LHO[hand_u]][trump as usize] == 0)
            {
                lowest_qtricks = 1;
                if 1 >= cutoff {
                    return 1;
                }
                suit += 1;
                if trump != DDS_NOTRUMP && suit == trump {
                    suit += 1;
                }
                if suit > 3 {
                    break;
                }
                continue;
            } else if count_rho == 0 && count_lho == 0 {
                if (tpos.rank_in_suit[LHO[hand_u]][trump as usize]
                    | tpos.rank_in_suit[RHO[hand_u]][trump as usize])
                    < tpos.rank_in_suit[PARTNER[hand_u]][trump as usize]
                {
                    lowest_qtricks = 1;

                    let rr = i32::from(
                        HIGHEST_RANK[tpos.rank_in_suit[PARTNER[hand_u]][trump as usize] as usize],
                    );
                    if rr != 0 {
                        tpos.win_ranks[depth_u][trump as usize] |= BIT_MAP_RANK[rr as usize];
                        if 1 >= cutoff {
                            return 1;
                        }
                    }
                }
                suit += 1;
                if trump != DDS_NOTRUMP && suit == trump {
                    suit += 1;
                }
                if suit > 3 {
                    break;
                }
                continue;
            } else if count_lho == 0 {
                if tpos.rank_in_suit[LHO[hand_u]][trump as usize]
                    < tpos.rank_in_suit[PARTNER[hand_u]][trump as usize]
                {
                    lowest_qtricks = 1;
                    for rr in (2..=14).rev() {
                        if tpos.rank_in_suit[PARTNER[hand_u]][trump as usize] & BIT_MAP_RANK[rr]
                            != 0
                        {
                            tpos.win_ranks[depth_u][trump as usize] |= BIT_MAP_RANK[rr];
                            break;
                        }
                    }
                    if 1 >= cutoff {
                        return 1;
                    }
                }
                suit += 1;
                if trump != DDS_NOTRUMP && suit == trump {
                    suit += 1;
                }
                if suit > 3 {
                    break;
                }
                continue;
            } else if count_rho == 0 {
                if tpos.rank_in_suit[RHO[hand_u]][trump as usize]
                    < tpos.rank_in_suit[PARTNER[hand_u]][trump as usize]
                {
                    lowest_qtricks = 1;
                    for rr in (2..=14).rev() {
                        if tpos.rank_in_suit[PARTNER[hand_u]][trump as usize] & BIT_MAP_RANK[rr]
                            != 0
                        {
                            tpos.win_ranks[depth_u][trump as usize] |= BIT_MAP_RANK[rr];
                            break;
                        }
                    }
                    if 1 >= cutoff {
                        return 1;
                    }
                }
                suit += 1;
                if trump != DDS_NOTRUMP && suit == trump {
                    suit += 1;
                }
                if suit > 3 {
                    break;
                }
                continue;
            }
        }

        if qtricks >= cutoff {
            return qtricks;
        }

        if trump != DDS_NOTRUMP && suit == trump {
            suit = i32::from(trump == 0);
        } else {
            suit += 1;
            if trump != DDS_NOTRUMP && suit == trump {
                suit += 1;
            }
        }
        if suit > 3 {
            break;
        }
    }

    if qtricks == 0 && (trump == DDS_NOTRUMP || tpos.winner[trump as usize].hand == -1) {
        for ss in 0..DDS_SUITS {
            if tpos.winner[ss].hand == -1 {
                continue;
            }
            if tpos.length[hand_u][ss] > 0 {
                tpos.win_ranks[depth_u][ss] = BIT_MAP_RANK[tpos.winner[ss].rank as usize];
            }
        }

        // Note: the vendor flips the cutoff calculation here (uses
        // the OTHER node-type formula). Preserve that quirk.
        cutoff = if node_type_store[hand_u] == MAXNODE {
            tpos.tricks_max - target + (depth >> 2) + 2
        } else {
            target - tpos.tricks_max
        };

        if 1 >= cutoff {
            return 0;
        }
    }

    *result = false;
    qtricks
}

#[allow(clippy::too_many_arguments)]
fn qtricks_lead_hand_trump(
    tpos: &mut Pos,
    cutoff: i32,
    depth: i32,
    count_lho: i32,
    count_rho: i32,
    lho_trump_ranks: i32,
    rho_trump_ranks: i32,
    count_own: i32,
    count_part: i32,
    suit: i32,
    qtricks: i32,
    res: &mut i32,
    hand: i32,
) -> i32 {
    // res = 0: continue with same suit.
    // res = 1: cutoff.
    // res = 2: continue with next suit.
    *res = 1;
    let suit_u = suit as usize;
    let depth_u = depth as usize;
    let hand_u = hand as usize;
    let mut qt = qtricks;

    if (count_lho != 0 || lho_trump_ranks == 0) && (count_rho != 0 || rho_trump_ranks == 0) {
        tpos.win_ranks[depth_u][suit_u] |= BIT_MAP_RANK[tpos.winner[suit_u].rank as usize];
        qt += 1;
        if qt >= cutoff {
            return qt;
        }

        if count_lho <= 1
            && count_rho <= 1
            && count_part <= 1
            && lho_trump_ranks == 0
            && rho_trump_ranks == 0
        {
            qt += count_own - 1;
            if qt >= cutoff {
                return qt;
            }
            *res = 2;
            return qt;
        }
    }

    if tpos.second_best[suit_u].hand == hand {
        if lho_trump_ranks == 0 && rho_trump_ranks == 0 {
            tpos.win_ranks[depth_u][suit_u] |= BIT_MAP_RANK[tpos.second_best[suit_u].rank as usize];
            qt += 1;
            if qt >= cutoff {
                return qt;
            }
            if count_lho <= 2 && count_rho <= 2 && count_part <= 2 {
                qt += count_own - 2;
                if qt >= cutoff {
                    return qt;
                }
                *res = 2;
                return qt;
            }
        }
    } else if tpos.second_best[suit_u].hand == PARTNER[hand_u] as i32
        && count_own > 1
        && count_part > 1
    {
        // Second best at partner and suit length of own hand and
        // partner > 1.
        if lho_trump_ranks == 0 && rho_trump_ranks == 0 {
            tpos.win_ranks[depth_u][suit_u] |= BIT_MAP_RANK[tpos.second_best[suit_u].rank as usize];
            qt += 1;
            if qt >= cutoff {
                return qt;
            }
            if count_lho <= 2 && count_rho <= 2 && (count_part <= 2 || count_own <= 2) {
                qt += (count_own - 2).max(count_part - 2);
                if qt >= cutoff {
                    return qt;
                }
                *res = 2;
                return qt;
            }
        }
    }

    *res = 0;
    qt
}

#[allow(clippy::too_many_arguments)]
fn qtricks_lead_hand_nt(
    tpos: &mut Pos,
    cutoff: i32,
    depth: i32,
    count_lho: i32,
    count_rho: i32,
    lho_trump_ranks: &mut i32,
    rho_trump_ranks: &mut i32,
    comm_partner: bool,
    comm_suit: i32,
    count_own: i32,
    count_part: i32,
    suit: i32,
    qtricks: i32,
    trump: i32,
    res: &mut i32,
    hand: i32,
) -> i32 {
    // res = 0: continue with same suit.
    // res = 1: cutoff.
    // res = 2: continue with next suit.
    *res = 1;
    let suit_u = suit as usize;
    let depth_u = depth as usize;
    let hand_u = hand as usize;
    let mut qt = qtricks;

    tpos.win_ranks[depth_u][suit_u] |= BIT_MAP_RANK[tpos.winner[suit_u].rank as usize];
    qt += 1;
    if qt >= cutoff {
        return qt;
    }
    if trump == suit && (!comm_partner || suit != comm_suit) {
        *lho_trump_ranks = (*lho_trump_ranks - 1).max(0);
        *rho_trump_ranks = (*rho_trump_ranks - 1).max(0);
    }

    if count_lho <= 1 && count_rho <= 1 && count_part <= 1 {
        qt += count_own - 1;
        if qt >= cutoff {
            return qt;
        }
        *res = 2;
        return qt;
    }

    if tpos.second_best[suit_u].hand == hand {
        tpos.win_ranks[depth_u][suit_u] |= BIT_MAP_RANK[tpos.second_best[suit_u].rank as usize];
        qt += 1;
        if qt >= cutoff {
            return qt;
        }
        if trump == suit && (!comm_partner || suit != comm_suit) {
            *lho_trump_ranks = (*lho_trump_ranks - 1).max(0);
            *rho_trump_ranks = (*rho_trump_ranks - 1).max(0);
        }
        if count_lho <= 2 && count_rho <= 2 && count_part <= 2 {
            qt += count_own - 2;
            if qt >= cutoff {
                return qt;
            }
            *res = 2;
            return qt;
        }
    } else if tpos.second_best[suit_u].hand == PARTNER[hand_u] as i32
        && count_own > 1
        && count_part > 1
    {
        // Second best at partner and suit length of own hand and
        // partner > 1.
        tpos.win_ranks[depth_u][suit_u] |= BIT_MAP_RANK[tpos.second_best[suit_u].rank as usize];
        qt += 1;
        if qt >= cutoff {
            return qt;
        }
        if trump == suit && (!comm_partner || suit != comm_suit) {
            *lho_trump_ranks = (*lho_trump_ranks - 1).max(0);
            *rho_trump_ranks = (*rho_trump_ranks - 1).max(0);
        }
        if count_lho <= 2 && count_rho <= 2 && (count_part <= 2 || count_own <= 2) {
            qt += (count_own - 2).max(count_part - 2);
            if qt >= cutoff {
                return qt;
            }
            *res = 2;
            return qt;
        }
    }

    *res = 0;
    qt
}

#[allow(clippy::too_many_arguments)]
fn quick_tricks_partner_hand_trump(
    tpos: &mut Pos,
    cutoff: i32,
    depth: i32,
    count_lho: i32,
    count_rho: i32,
    lho_trump_ranks: i32,
    rho_trump_ranks: i32,
    count_own: i32,
    count_part: i32,
    suit: i32,
    qtricks: i32,
    comm_suit: i32,
    comm_rank: i32,
    res: &mut i32,
    hand: i32,
    rel: &[RelRanks],
) -> i32 {
    // res = 0: continue with same suit.
    // res = 1: cutoff.
    // res = 2: continue with next suit.
    *res = 1;
    let suit_u = suit as usize;
    let depth_u = depth as usize;
    let hand_u = hand as usize;
    let mut qt = qtricks;

    if (count_lho != 0 || lho_trump_ranks == 0) && (count_rho != 0 || rho_trump_ranks == 0) {
        tpos.win_ranks[depth_u][suit_u] |= BIT_MAP_RANK[tpos.winner[suit_u].rank as usize];
        tpos.win_ranks[depth_u][comm_suit as usize] |= BIT_MAP_RANK[comm_rank as usize];
        qt += 1; // A trick can be taken.
        if qt >= cutoff {
            return qt;
        }
        if count_lho <= 1
            && count_rho <= 1
            && count_own <= 1
            && lho_trump_ranks == 0
            && rho_trump_ranks == 0
        {
            qt += count_part - 1;
            if qt >= cutoff {
                return qt;
            }
            *res = 2;
            return qt;
        }
    }

    if tpos.second_best[suit_u].hand == PARTNER[hand_u] as i32 {
        // Second best found in partner's hand.
        if lho_trump_ranks == 0 && rho_trump_ranks == 0 {
            // Opponents have no trump.
            tpos.win_ranks[depth_u][suit_u] |= BIT_MAP_RANK[tpos.second_best[suit_u].rank as usize];
            tpos.win_ranks[depth_u][comm_suit as usize] |= BIT_MAP_RANK[comm_rank as usize];
            qt += 1;
            if qt >= cutoff {
                return qt;
            }
            if count_lho <= 2 && count_rho <= 2 && count_own <= 2 {
                qt += count_part - 2;
                if qt >= cutoff {
                    return qt;
                }
                *res = 2;
                return qt;
            }
        }
    } else if tpos.second_best[suit_u].hand == hand && count_part > 1 && count_own > 1 {
        // Second best found in own hand and suit lengths of own hand
        // and partner > 1.
        if lho_trump_ranks == 0 && rho_trump_ranks == 0 {
            // Opponents have no trump.
            tpos.win_ranks[depth_u][suit_u] |= BIT_MAP_RANK[tpos.second_best[suit_u].rank as usize];
            tpos.win_ranks[depth_u][comm_suit as usize] |= BIT_MAP_RANK[comm_rank as usize];
            qt += 1;
            if qt >= cutoff {
                return qt;
            }
            if count_lho <= 2 && count_rho <= 2 && (count_own <= 2 || count_part <= 2) {
                qt += (count_part - 2).max(count_own - 2);
                if qt >= cutoff {
                    return qt;
                }
                *res = 2;
                return qt;
            }
        }
    } else if suit == comm_suit
        && tpos.second_best[suit_u].hand == LHO[hand_u] as i32
        && (count_lho >= 2 || lho_trump_ranks == 0)
        && (count_rho >= 2 || rho_trump_ranks == 0)
    {
        let ranks = tpos.aggr[suit_u];
        let (third_rank, third_hand) = abs_rank(rel, ranks, 3, suit_u);
        if third_hand == PARTNER[hand_u] as i32 {
            tpos.win_ranks[depth_u][suit_u] |= BIT_MAP_RANK[third_rank as usize];
            tpos.win_ranks[depth_u][comm_suit as usize] |= BIT_MAP_RANK[comm_rank as usize];
            qt += 1;
            if qt >= cutoff {
                return qt;
            }
            if count_own <= 2
                && count_lho <= 2
                && count_rho <= 2
                && lho_trump_ranks == 0
                && rho_trump_ranks == 0
            {
                qt += count_part - 2;
                if qt >= cutoff {
                    return qt;
                }
            }
        }
    }

    *res = 0;
    qt
}

#[allow(clippy::too_many_arguments)]
fn quick_tricks_partner_hand_nt(
    tpos: &mut Pos,
    cutoff: i32,
    depth: i32,
    count_lho: i32,
    count_rho: i32,
    count_own: i32,
    count_part: i32,
    suit: i32,
    qtricks: i32,
    comm_suit: i32,
    comm_rank: i32,
    res: &mut i32,
    hand: i32,
    rel: &[RelRanks],
) -> i32 {
    *res = 1;
    let suit_u = suit as usize;
    let depth_u = depth as usize;
    let hand_u = hand as usize;
    let mut qt = qtricks;

    tpos.win_ranks[depth_u][suit_u] |= BIT_MAP_RANK[tpos.winner[suit_u].rank as usize];
    tpos.win_ranks[depth_u][comm_suit as usize] |= BIT_MAP_RANK[comm_rank as usize];
    qt += 1;
    if qt >= cutoff {
        return qt;
    }
    if count_lho <= 1 && count_rho <= 1 && count_own <= 1 {
        qt += count_part - 1;
        if qt >= cutoff {
            return qt;
        }
        *res = 2;
        return qt;
    }

    if tpos.second_best[suit_u].hand == PARTNER[hand_u] as i32 {
        // Second best found in partner's hand.
        tpos.win_ranks[depth_u][suit_u] |= BIT_MAP_RANK[tpos.second_best[suit_u].rank as usize];
        qt += 1;
        if qt >= cutoff {
            return qt;
        }
        if count_lho <= 2 && count_rho <= 2 && count_own <= 2 {
            qt += count_part - 2;
            if qt >= cutoff {
                return qt;
            }
            *res = 2;
            return qt;
        }
    } else if tpos.second_best[suit_u].hand == hand && count_part > 1 && count_own > 1 {
        // Second best found in own hand and own and partner's suit
        // length > 1.
        tpos.win_ranks[depth_u][suit_u] |= BIT_MAP_RANK[tpos.second_best[suit_u].rank as usize];
        qt += 1;
        if qt >= cutoff {
            return qt;
        }
        if count_lho <= 2 && count_rho <= 2 && (count_own <= 2 || count_part <= 2) {
            qt += (count_part - 2).max(count_own - 2);
            if qt >= cutoff {
                return qt;
            }
            *res = 2;
            return qt;
        }
    } else if suit == comm_suit && tpos.second_best[suit_u].hand == LHO[hand_u] as i32 {
        let ranks = tpos.aggr[suit_u];
        let (third_rank, third_hand) = abs_rank(rel, ranks, 3, suit_u);
        if third_hand == PARTNER[hand_u] as i32 {
            tpos.win_ranks[depth_u][suit_u] |= BIT_MAP_RANK[third_rank as usize];
            qt += 1;
            if qt >= cutoff {
                return qt;
            }
            if count_own <= 2 && count_lho <= 2 && count_rho <= 2 {
                // Mirror of vendor's TODO-marked fix.
                qt += count_part - 2;
                if qt >= cutoff {
                    return qt;
                }
            }
        }
    }

    *res = 0;
    qt
}

/// Second-hand sure-tricks heuristic. Mirror of the vendor's
/// `QuickTricksSecondHand`.
///
/// Called from `ABsearch1` after an opponent has played the first card
/// of the trick. Returns `true` if the MAX side can immediately win
/// enough tricks to satisfy the search's `target`.
///
/// `ini_depth` is the search's initial depth (the recursion stops short
/// of the actual initial position).
#[inline]
pub fn quick_tricks_second_hand(
    tpos: &mut Pos,
    hand: i32,
    depth: i32,
    target: i32,
    trump: i32,
    node_type_store: &[i32; DDS_HANDS],
    ini_depth: i32,
) -> bool {
    if depth == ini_depth {
        return false;
    }

    let hand_u = hand as usize;
    let depth_u = depth as usize;
    let ss = tpos.move_history[depth_u + 1].suit;
    let ss_u = ss as usize;
    let ranks: u16 = tpos.rank_in_suit[hand_u][ss_u] | tpos.rank_in_suit[PARTNER[hand_u]][ss_u];

    for s in 0..DDS_SUITS {
        tpos.win_ranks[depth_u][s] = 0;
    }

    if trump != DDS_NOTRUMP
        && ss != trump
        && ((tpos.rank_in_suit[hand_u][ss_u] == 0
            && tpos.rank_in_suit[hand_u][trump as usize] != 0)
            || (tpos.rank_in_suit[PARTNER[hand_u]][ss_u] == 0
                && tpos.rank_in_suit[PARTNER[hand_u]][trump as usize] != 0))
    {
        if tpos.rank_in_suit[LHO[hand_u]][ss_u] == 0
            && tpos.rank_in_suit[LHO[hand_u]][trump as usize] != 0
        {
            return false;
        }
        // Own side can ruff, their side can't.
    } else if ranks
        > (BIT_MAP_RANK[tpos.move_history[depth_u + 1].rank as usize]
            | tpos.rank_in_suit[LHO[hand_u]][ss_u])
    {
        if trump != DDS_NOTRUMP
            && ss != trump
            && tpos.rank_in_suit[LHO[hand_u]][trump as usize] != 0
            && tpos.rank_in_suit[LHO[hand_u]][ss_u] == 0
        {
            return false;
        }

        // Own side has highest card in suit, which LHO can't ruff.
        let rr = i32::from(HIGHEST_RANK[ranks as usize]);
        tpos.win_ranks[depth_u][ss_u] = BIT_MAP_RANK[rr as usize];
    } else {
        // No easy way to win current trick for own side.
        return false;
    }

    let mut qtricks = 1;

    let cutoff = if node_type_store[hand_u] == MAXNODE {
        target - tpos.tricks_max
    } else {
        tpos.tricks_max - target + (depth >> 2) + 3
    };

    if qtricks >= cutoff {
        return true;
    }

    if trump != DDS_NOTRUMP {
        return false;
    }

    // In NT, second winner (by rank) in same suit.
    let hh = if tpos.rank_in_suit[hand_u][ss_u] > tpos.rank_in_suit[PARTNER[hand_u]][ss_u] {
        hand // Hand to lead next trick.
    } else {
        PARTNER[hand_u] as i32
    };
    let hh_u = hh as usize;

    if tpos.winner[ss_u].hand == hh
        && tpos.second_best[ss_u].rank != 0
        && tpos.second_best[ss_u].hand == hh
    {
        qtricks += 1;
        tpos.win_ranks[depth_u][ss_u] |= BIT_MAP_RANK[tpos.second_best[ss_u].rank as usize];

        if qtricks >= cutoff {
            return true;
        }
    }

    for s in 0..DDS_SUITS {
        if s == ss_u || tpos.length[hh_u][s] == 0 {
            continue;
        }

        if tpos.length[LHO[hh_u]][s] == 0
            && tpos.length[RHO[hh_u]][s] == 0
            && tpos.length[PARTNER[hh_u]][s] == 0
        {
            // Long other suit which nobody else holds.
            qtricks += i32::from(crate::lookup::COUNT_TABLE[tpos.rank_in_suit[hh_u][s] as usize]);
            if qtricks >= cutoff {
                return true;
            }
        } else if tpos.winner[s].rank != 0 && tpos.winner[s].hand == hh {
            // Top winners in other suits.
            qtricks += 1;
            tpos.win_ranks[depth_u][s] |= BIT_MAP_RANK[tpos.winner[s].rank as usize];

            if qtricks >= cutoff {
                return true;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::move_type::HighCard;

    /// MAX = North/South (hands 0 and 2), MIN = East/West (hands 1 and 3).
    const NODE_TYPE: [i32; 4] = [MAXNODE, MINNODE, MAXNODE, MINNODE];

    /// Construct a position where MAX (hand 0 = North) holds AKQ of
    /// suit 0 (spades), opponents are void in spades, partner also
    /// void. Expect at least 3 quick tricks in NT.
    #[test]
    fn akq_void_returns_three() {
        let mut pos = Pos::default();
        // North (hand 0) holds AKQ of suit 0.
        // BIT_MAP_RANK: A=14 -> 0x1000, K=13 -> 0x0800, Q=12 -> 0x0400.
        let akq: u16 = 0x1000 | 0x0800 | 0x0400;
        pos.rank_in_suit[0][0] = akq;
        pos.length[0][0] = 3;
        pos.aggr[0] = akq;
        // Winner of suit 0 = A held by hand 0.
        pos.winner[0] = HighCard { rank: 14, hand: 0 };
        // Second-best of suit 0 = K held by hand 0.
        pos.second_best[0] = HighCard { rank: 13, hand: 0 };
        // Partner (hand 2) and opps (1, 3) have nothing — defaults are 0.
        // No other suits — winners.hand defaults to 0 which would be
        // misleading; flag them as "no winner" by setting hand = -1.
        for s in 1..4 {
            pos.winner[s] = HighCard { rank: 0, hand: -1 };
            pos.second_best[s] = HighCard { rank: 0, hand: -1 };
        }

        // Target = 3 (we want 3 tricks); depth doesn't really matter,
        // pick something below MAX_DEPTH and above 0 such that the
        // win_ranks slot exists.
        let mut success = false;
        let rel = crate::search::build_rel_for(&pos.rank_in_suit);
        let qt = quick_tricks(
            &mut pos,
            0, // hand = North
            44,
            3, // target
            DDS_NOTRUMP,
            &mut success,
            &NODE_TYPE,
            rel.as_ref(),
        );

        assert!(
            qt >= 3,
            "expected ≥ 3 quick tricks holding AKQ alone, got {qt}"
        );
        assert!(success, "expected success flag set when target met");
    }

    /// Position where MAX has exactly one easy trick (an A in suit 0)
    /// but needs many more to reach `target`. The heuristic takes the
    /// one trick but can't conclude (target unreachable from sure
    /// tricks alone), so it returns `qt=1` with `success = false`,
    /// signalling the search must continue.
    #[test]
    fn partial_qt_continues_search() {
        let mut pos = Pos::default();
        // North (hand 0) holds the bare A of suit 0.
        pos.rank_in_suit[0][0] = 0x1000;
        pos.length[0][0] = 1;
        pos.aggr[0] = 0x1000;
        pos.winner[0] = HighCard { rank: 14, hand: 0 };
        pos.second_best[0] = HighCard { rank: 0, hand: -1 };
        // Other suits: no winner.
        for s in 1..4 {
            pos.winner[s] = HighCard { rank: 0, hand: -1 };
            pos.second_best[s] = HighCard { rank: 0, hand: -1 };
        }

        let mut success = true; // start true to verify it gets cleared.
        let rel = crate::search::build_rel_for(&pos.rank_in_suit);
        let qt = quick_tricks(
            &mut pos,
            0,
            0,
            13,
            DDS_NOTRUMP,
            &mut success,
            &NODE_TYPE,
            rel.as_ref(),
        );

        // We take the one easy trick but cannot reach target=13.
        assert_eq!(qt, 1, "single A → 1 quick trick");
        assert!(
            !success,
            "expected success = false: heuristic can not determine outcome"
        );
    }

    /// Position where MAX holds nothing useful and the heuristic cannot
    /// determine a result. Expect `qt = 0` with `success = false`.
    ///
    /// Vendor quirk: when `qtricks == 0` at the end of the main loop,
    /// the cleanup block recomputes `cutoff` with the *opposite*
    /// node-type formula and returns 0 (with `success` left true) if
    /// `1 >= cutoff`. To force `success = false`, push the cleanup
    /// cutoff above 1: with `target = 1`, `tricks_max = 0`, MAXNODE,
    /// the flipped cutoff is `0 - 1 + (depth>>2) + 2`. Choose
    /// `depth = 4` → cutoff = 2, so `1 >= 2` is false and we drop
    /// through to `*result = false; return qtricks;`.
    #[test]
    fn no_winners_returns_zero() {
        let mut pos = Pos::default();
        // MIN (hand 1 = East) holds the aces in every suit.
        for s in 0..4 {
            pos.rank_in_suit[1][s] = 0x1000; // A
            pos.length[1][s] = 1;
            pos.aggr[s] = 0x1000;
            pos.winner[s] = HighCard { rank: 14, hand: 1 };
            pos.second_best[s] = HighCard { rank: 0, hand: -1 };
        }
        // North (hand 0) has nothing in any suit.

        let mut success = true;
        let rel = crate::search::build_rel_for(&pos.rank_in_suit);
        let qt = quick_tricks(
            &mut pos,
            0,
            4,
            1,
            DDS_NOTRUMP,
            &mut success,
            &NODE_TYPE,
            rel.as_ref(),
        );

        assert_eq!(qt, 0, "no winners → 0 quick tricks");
        assert!(
            !success,
            "expected success = false: heuristic indeterminate"
        );
    }

    /// Second-hand variant: if `depth == ini_depth`, must return false
    /// without examining position.
    #[test]
    fn second_hand_at_ini_depth_returns_false() {
        let mut pos = Pos::default();
        let result = quick_tricks_second_hand(
            &mut pos,
            0,
            44,
            13,
            DDS_NOTRUMP,
            &NODE_TYPE,
            44, // ini_depth == depth
        );
        assert!(!result);
    }
}
