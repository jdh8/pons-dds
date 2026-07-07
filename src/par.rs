//! Par score and par contract calculation.
//!
//! Ported statement-for-statement from
//! [`Par.cpp`](../../../ddss-sys/vendor/src/Par.cpp): `rawscore`,
//! `SideSeats`, `CalcOverTricks`, `VulnerDefSide`, `SidesParBin` (the
//! Matthew Kidd-derived algorithm), and `DealerParBin` (which runs the
//! `crate::dealer_par` text engine and re-parses its strings at fixed
//! byte offsets).  The binary structs `ContractType` and
//! `ParResultsMaster` mirror `contractType` and `parResultsMaster` in
//! [`dll.h`](../../../ddss-sys/vendor/include/dll.h).
//!
//! The public surface — [`Par`], [`ParContract`], [`calculate_par`], and
//! [`calculate_pars`] — mirrors the `ddss` FFI reference crate, so a
//! migration between the two crates is a near-mechanical swap.

use crate::dealer_par::dealer_par;
use crate::tricks::{TrickCountRow, TrickCountTable};
use crate::vulnerability::Vulnerability;

use contract_bridge::Strain;
use contract_bridge::contract::{Bid, Contract, Level, Penalty};
use contract_bridge::seat::Seat;

use core::ops::BitOr as _;

/// Par contract
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParContract {
    /// The contract
    pub contract: Contract,

    /// The declarer of the contract
    pub declarer: Seat,

    /// The number of overtricks (negative for undertricks)
    pub overtricks: i8,
}

/// Par score and contracts
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Par {
    /// The par score
    pub score: i32,

    /// The contracts that achieve the par score
    pub contracts: Vec<ParContract>,
}

impl Par {
    /// Check if two pars are equivalent
    ///
    /// Two pars are equivalent if they have the same par score and the same
    /// set of (strain, declarer) pairs.  Overtricks and duplicate entries are
    /// ignored.
    ///
    /// This is intentionally looser than [`PartialEq`], which compares every
    /// field exactly.  `equivalent` exists because DDS may report the same
    /// par result with different overtrick, doubling, or ordering details
    /// depending on the code path (e.g. `DealerParBin` vs `SidesParBin`).
    /// Use `==` when you need exact structural equality; use `equivalent`
    /// when you only care about the strategic meaning of the par result.
    #[must_use]
    pub fn equivalent(&self, other: &Self) -> bool {
        // Since every contract scores the same, we can compare only the set of
        // (`Strain`, `Seat`).  #`Strain` * #`Seat` = 5 * 4 = 20, which fits
        // in a `u32` as a bitset.
        fn key(contracts: &[ParContract]) -> u32 {
            contracts
                .iter()
                .map(|p| 1 << ((p.contract.bid.strain as u8) << 2 | p.declarer as u8))
                .fold(0, u32::bitor)
        }
        self.score == other.score && key(&self.contracts) == key(&other.contracts)
    }
}

/// Mirror of `contractType` in `dll.h`.
#[derive(Debug, Default, Clone, Copy)]
struct ContractType {
    /// 0 = make, 1-13 = sacrifice.  [`dealer_par_bin`]'s fixed-offset parse
    /// can also leave `'\0' - '0'` = -48 here (see the quirk note there), so
    /// this must stay a signed `i32`.
    under_tricks: i32,
    /// 0-3, e.g. 1 for 4S + 1.
    over_tricks: i32,
    /// 1-7.
    level: i32,
    /// 0 = No Trumps, 1 = trump Spades, 2 = trump Hearts,
    /// 3 = trump Diamonds, 4 = trump Clubs.
    denom: i32,
    /// One of the cases N, E, W, S, NS, EW;
    /// 0 = N, 1 = E, 2 = S, 3 = W, 4 = NS, 5 = EW.
    seats: i32,
}

/// Mirror of `parResultsMaster` in `dll.h`.
#[derive(Debug, Default, Clone, Copy)]
struct ParResultsMaster {
    /// Sign according to the NS view.
    score: i32,
    /// Number of contracts giving the par score.
    number: i32,
    /// Par contracts.
    contracts: [ContractType; 10],
}

impl From<ParResultsMaster> for Par {
    fn from(par: ParResultsMaster) -> Self {
        let number = usize::try_from(par.number)
            .ok()
            .filter(|&n| n <= par.contracts.len())
            .unwrap_or_else(|| unreachable!("the number of par contracts is 0..=10"));

        // The vendor returns a zero contract for par-zero deals, but we want
        // to filter it out for consistency.
        let len = number * usize::from(par.contracts[0].level != 0);

        let contracts = par.contracts[..len]
            .iter()
            .flat_map(|contract| {
                let strain = match contract.denom {
                    0 => Strain::Notrump,
                    1 => Strain::Spades,
                    2 => Strain::Hearts,
                    3 => Strain::Diamonds,
                    4 => Strain::Clubs,
                    _ => unreachable!("the par denomination is 0..=4"),
                };

                let (penalty, overtricks) = if contract.under_tricks > 0 {
                    (Penalty::Doubled, -contract.under_tricks as i8)
                } else {
                    (Penalty::Undoubled, contract.over_tricks as i8)
                };

                let seat = match contract.seats & 3 {
                    0 => Seat::North,
                    1 => Seat::East,
                    2 => Seat::South,
                    3 => Seat::West,
                    _ => unreachable!("The bitmask ensures this is always in 0..=3"),
                };
                let is_pair = contract.seats >= 4;

                let contract = Contract {
                    bid: Bid {
                        level: Level::try_new(contract.level as u8)
                            .unwrap_or_else(|_| unreachable!("the par contract level is 1..=7")),
                        strain,
                    },
                    penalty,
                };

                core::iter::once(ParContract {
                    contract,
                    declarer: seat,
                    overtricks,
                })
                .chain(is_pair.then_some(ParContract {
                    contract,
                    declarer: seat.partner(),
                    overtricks,
                }))
            })
            .collect();

        Self {
            score: par.score,
            contracts,
        }
    }
}

/// Mirror of the vendor's `par_suits_type`.
#[derive(Debug, Default, Clone, Copy)]
struct ParSuitsType {
    suit: i32,
    tricks: i32,
    score: i32,
}

/// Mirror of the vendor's `best_par_type`.
#[derive(Debug, Default, Clone, Copy)]
struct BestParType {
    par_denom: i32,
    par_tricks: i32,
}

/// Mirror of the vendor's `parContr2Type`.
#[derive(Debug, Default, Clone, Copy)]
struct ParContr2Type {
    contracts: [u8; 10],
    denom: i32,
}

/// Maps par denomination order (NT, S, H, D, C) to DDS order (S, H, D, C,
/// NT); the vendor re-declares `denom_conv` locally in each function.
const DENOM_CONV: [usize; 5] = [4, 0, 1, 2, 3];

/// Maximal contract lowering that keeps the game or slam bonus.
/// index 1: 0=NT, 1=Major, 2=Minor; index 2: contract level 1-7.
const MAX_LOW: [[i32; 8]; 3] = [
    [0, 0, 1, 0, 1, 2, 0, 0],
    [0, 0, 1, 2, 0, 1, 0, 0],
    [0, 0, 1, 2, 3, 0, 0, 0],
];

/// Mirror of the vendor's `rawscore` (Par.cpp).
///
/// Computes the score for an undoubled making contract or for a doubled
/// contract with a given number of undertricks.  These are the only
/// possibilities for a par contract (aside from a passed out hand).
///
/// - `denom` — 0 = NT, 1 = Spades, 2 = Hearts, 3 = Diamonds, 4 = Clubs
///   (same order as results from the double dummy solver); -1 undertricks.
/// - `tricks` — for making contracts (7-13); otherwise, number of
///   undertricks.
/// - `isvul` — true (nonzero) if vulnerable.
const fn rawscore(denom: i32, tricks: i32, isvul: i32) -> i32 {
    if denom == -1 {
        if isvul != 0 {
            return -300 * tricks + 100;
        }
        if tricks <= 3 {
            return -200 * tricks + 100;
        }
        return -300 * tricks + 400;
    }

    let level = tricks - 6;
    let mut game_bonus = 0;
    let mut score;
    if denom == 0 {
        score = 10 + 30 * level;
        if level >= 3 {
            game_bonus = 1;
        }
    } else if denom == 1 || denom == 2 {
        score = 30 * level;
        if level >= 4 {
            game_bonus = 1;
        }
    } else {
        score = 20 * level;
        if level >= 5 {
            game_bonus = 1;
        }
    }
    if game_bonus != 0 {
        score += if isvul != 0 { 500 } else { 300 };
    } else {
        score += 50;
    }

    if level == 6 {
        score += if isvul != 0 { 750 } else { 500 };
    } else if level == 7 {
        score += if isvul != 0 { 1500 } else { 1000 };
    }

    score
}

/// Mirror of the vendor's `SideSeats`.
// The vendor's if/else chain is kept over a `match` on `Ordering` for
// line-by-line correspondence.
#[allow(clippy::comparison_chain)]
const fn side_seats(
    dr: i32,
    i: usize,
    t1: i32,
    t2: i32,
    order: usize,
    sides_res: &mut [ParResultsMaster; 2],
) {
    if (dr + i as i32) % 2 != 0 {
        if t1 == t2 {
            sides_res[i].contracts[order].seats = 4;
        } else if t1 > t2 {
            sides_res[i].contracts[order].seats = 0;
        } else {
            sides_res[i].contracts[order].seats = 2;
        }
    } else if t1 == t2 {
        sides_res[i].contracts[order].seats = 5;
    } else if t1 > t2 {
        sides_res[i].contracts[order].seats = 1;
    } else {
        sides_res[i].contracts[order].seats = 3;
    }
}

/// Mirror of the vendor's `CalcOverTricks`.
const fn calc_over_tricks(
    i: usize,
    max_lower: i32,
    tricks: i32,
    order: usize,
    sides_res: &mut [ParResultsMaster; 2],
) {
    match tricks - 6 {
        5 | 4 => {
            if max_lower == 3 {
                sides_res[i].contracts[order].over_tricks = 3;
            } else if max_lower == 2 {
                sides_res[i].contracts[order].over_tricks = 2;
            } else if max_lower == 1 {
                sides_res[i].contracts[order].over_tricks = 1;
            } else {
                sides_res[i].contracts[order].over_tricks = 0;
            }
        }
        3 => {
            if max_lower == 2 {
                sides_res[i].contracts[order].over_tricks = 2;
            } else if max_lower == 1 {
                sides_res[i].contracts[order].over_tricks = 1;
            } else {
                sides_res[i].contracts[order].over_tricks = 0;
            }
        }
        2 => {
            if max_lower == 1 {
                sides_res[i].contracts[order].over_tricks = 1;
            } else {
                sides_res[i].contracts[order].over_tricks = 0;
            }
        }
        _ => sides_res[i].contracts[order].over_tricks = 0,
    }
}

/// Mirror of the vendor's `VulnerDefSide`: vulnerability (0/1) of the
/// defending side, where `side` is nonzero when N/S makes the par contract.
const fn vulner_def_side(side: i32, vulnerable: i32) -> i32 {
    if vulnerable == 0 {
        0
    } else if vulnerable == 1 {
        1
    } else if side != 0 {
        /* N/S makes par contract. */
        if vulnerable == 2 { 0 } else { 1 }
    } else if vulnerable == 3 {
        0
    } else {
        1
    }
}

/// Mirror of the vendor's `SidesParBin` (Par.cpp).
///
/// The code for calculation of par score / contracts is based upon the
/// perl code written by Matthew Kidd for `ACBLmerge`.  He has kindly given
/// permission to include a C++ adaptation in DDS.
///
/// - `res_table`: the `ddTableResults::resTable` layout — strains in DDS
///   order (S, H, D, C, NT) by seats (N, E, S, W).
/// - `vulnerable` — 0: None, 1: Both, 2: NS, 3: EW.
///
/// Returns the par results for N-S (index 0) and E-W (index 1) starting
/// the bidding.  These will nearly always be the same, but when we have a
/// "hot" situation they will not be.
fn sides_par_bin(res_table: &[[i32; 4]; 5], vulnerable: i32) -> [ParResultsMaster; 2] {
    /* vulnerable 0: None 1: Both 2: NS 3: EW */

    let mut sides_res = [ParResultsMaster::default(); 2];

    let mut denom_max = 0usize;
    let mut denom_filter = [0i32; 5];
    let mut no_of_denom = [0usize; 2];
    let mut best_par_score = [0i32; 2];
    let mut best_par_sacut = [0i32; 2];
    let mut best_par = [[BestParType::default(); 2]; 5]; /* 1st index order number. */

    let mut ut = 0;
    let mut t3 = [0i32; 5];
    let mut t4 = [0i32; 5];
    let mut par_suits = [ParSuitsType::default(); 5];

    let mut par_denom = [-1i32; 2]; /* 0-4 = NT,S,H,D,C */
    let mut par_tricks = [6i32; 2]; /* Initial "contract" beats 0 NT */
    let mut par_score = [0i32; 2];
    let mut par_sacut = [0i32; 2]; /* Undertricks for sacrifice (0 if not sac) */

    /* Find best par result for N-S (i==0) or E-W (i==1). These will
    nearly always be the same, but when we have a "hot" situation
    they will not be. */

    for i in 0..=1usize {
        /* Start with the with the offensive side (current_side = 0) and
        alternate between sides seeking the to improve the result for the
        current side. */

        let mut no_filtered = 0;
        denom_filter.fill(0);

        let mut current_side = 0usize;
        let mut both_sides_once_flag = 0;
        loop {
            /* Find best contract for current side that beats current contract.
            Choose highest contract if results are equal. */

            let k = (i + current_side) % 2;

            let isvul = i32::from(
                vulnerable == 1
                    || if k != 0 {
                        vulnerable == 3
                    } else {
                        vulnerable == 2
                    },
            );

            let mut new_score_flag = 0;
            let prev_par_denom = par_denom[i];
            let prev_par_tricks = par_tricks[i];

            /* Calculate tricks and score values and
            store them for each denomination in structure par_suits[5]. */

            let mut n = 0usize;
            for j in 0..=4usize {
                if denom_filter[j] == 0 {
                    /* Current denomination is not filtered out. */
                    let t1 = if k != 0 {
                        res_table[DENOM_CONV[j]][1]
                    } else {
                        res_table[DENOM_CONV[j]][0]
                    };
                    let t2 = if k != 0 {
                        res_table[DENOM_CONV[j]][3]
                    } else {
                        res_table[DENOM_CONV[j]][2]
                    };
                    let tt = t1.max(t2);
                    /* tt is the maximum number of tricks current side can take in
                    denomination. */

                    par_suits[n].suit = j as i32;
                    par_suits[n].tricks = tt;

                    par_suits[n].score = if tt > par_tricks[i]
                        || (tt == par_tricks[i] && (j as i32) < par_denom[i])
                    {
                        rawscore(j as i32, tt, isvul)
                    } else {
                        rawscore(-1, prev_par_tricks - tt, isvul)
                    };
                    n += 1;
                }
            }

            /* Sort the items in the par_suits structure with decreasing order
            of the values on the scores. */

            for s in 1..n {
                let tmp = par_suits[s];
                let mut r = s;
                while r != 0 && tmp.score > par_suits[r - 1].score {
                    par_suits[r] = par_suits[r - 1];
                    r -= 1;
                }
                par_suits[r] = tmp;
            }

            /* Do the iteration as before but now in the order of the sorted
            denominations. */

            for par_suit in par_suits.iter().take(n) {
                let j = par_suit.suit;
                let tt = par_suit.tricks;

                let mut score = if tt > par_tricks[i] || (tt == par_tricks[i] && j < par_denom[i]) {
                    /* Can bid higher and make contract. */
                    rawscore(j, tt, isvul)
                } else {
                    /* Bidding higher in this denomination will not beat previous
                    denomination and may be a sacrifice. */
                    ut = prev_par_tricks - tt;
                    if j >= prev_par_denom {
                        /* Sacrifices higher than 7N are not permitted (but long ago
                        the official rules did not prohibit bidding higher than 7N!) */
                        if prev_par_tricks == 13 {
                            continue;
                        }
                        /* It will be necessary to bid one level higher, resulting in
                        one more undertrick. */
                        ut += 1;
                    }
                    /* Not a sacrifice (due to par_tricks > prev_par_tricks) */
                    if ut <= 0 {
                        continue;
                    }
                    /* Compute sacrifice. */
                    rawscore(-1, ut, isvul)
                };

                if current_side == 1 {
                    score = -score;
                }

                if (current_side == 0 && score > par_score[i])
                    || (current_side == 1 && score < par_score[i])
                {
                    new_score_flag = 1;
                    par_score[i] = score;
                    par_denom[i] = j;

                    if (current_side == 0 && score > 0) || (current_side == 1 && score < 0) {
                        /* New par score from a making contract.
                        Can immediately update since score at same level in higher
                        ranking suit is always >= score in lower ranking suit and
                        better than any sacrifice. */

                        par_tricks[i] = tt;
                        par_sacut[i] = 0;
                    } else {
                        par_tricks[i] = tt + ut;
                        par_sacut[i] = ut;
                    }
                }
            }

            if new_score_flag == 0 && both_sides_once_flag != 0 {
                if no_filtered == 0 {
                    best_par_score[i] = par_score[i];
                    if best_par_score[i] == 0 {
                        break;
                    }
                    best_par_sacut[i] = par_sacut[i];
                    no_of_denom[i] = 0;
                } else if best_par_score[i] != par_score[i] {
                    break;
                }
                if no_filtered >= 5 {
                    break;
                }
                denom_filter[par_denom[i] as usize] = 1;
                no_filtered += 1;
                best_par[no_of_denom[i]][i].par_denom = par_denom[i];
                best_par[no_of_denom[i]][i].par_tricks = par_tricks[i];
                no_of_denom[i] += 1;
                both_sides_once_flag = 0;
                current_side = 0;
                par_denom[i] = -1;
                par_tricks[i] = 6;
                par_score[i] = 0;
                par_sacut[i] = 0;
            } else {
                both_sides_once_flag = 1;
                current_side = 1 - current_side;
            }
        }
    }

    /* Output: "best par score" */
    sides_res[0].score = best_par_score[0];
    sides_res[1].score = best_par_score[1];

    if best_par_score[0] == 0 {
        /* Neither side can make anything. */
        sides_res[0].contracts[0].denom = 0;
        sides_res[0].contracts[0].level = 0;
        sides_res[0].contracts[0].over_tricks = 0;
        sides_res[0].contracts[0].under_tricks = 0;
        sides_res[0].contracts[0].seats = 0;
        sides_res[0].number = 1;
        sides_res[1].contracts[0].denom = 0;
        sides_res[1].contracts[0].level = 0;
        sides_res[1].contracts[0].over_tricks = 0;
        sides_res[1].contracts[0].under_tricks = 0;
        sides_res[1].contracts[0].seats = 0;
        sides_res[1].number = 1;
        return sides_res;
    }

    for i in 0..=1usize {
        sides_res[i].number = no_of_denom[i] as i32;
        sides_res[i].score = best_par_score[i];

        if best_par_sacut[i] > 0 {
            /* Sacrifice */
            // The vendor's `dr = (best_par_score[i] > 0) ? 0 : 1`.
            let dr = i32::from(best_par_score[i] <= 0);
            /* Sort the items in the best_par structure with increasing order
            of the values on denom. */

            for s in 1..no_of_denom[i] {
                let tmp = best_par[s][i];
                let mut r = s;
                while r != 0 && tmp.par_denom < best_par[r - 1][i].par_denom {
                    best_par[r][i] = best_par[r - 1][i];
                    r -= 1;
                }
                best_par[r][i] = tmp;
            }

            for (m, best) in best_par.iter().enumerate().take(no_of_denom[i]) {
                let j = best[i].par_denom;
                let ju = j as usize;

                let t1 = if (dr + i as i32) % 2 != 0 {
                    res_table[DENOM_CONV[ju]][0]
                } else {
                    res_table[DENOM_CONV[ju]][1]
                };
                let t2 = if (dr + i as i32) % 2 != 0 {
                    res_table[DENOM_CONV[ju]][2]
                } else {
                    res_table[DENOM_CONV[ju]][3]
                };
                // The vendor also computes `tt = max(t1, t2)` here, unused
                // in this branch.

                side_seats(dr, i, t1, t2, m, &mut sides_res);
                sides_res[i].contracts[m].denom = j;
                sides_res[i].contracts[m].level = best[i].par_tricks - 6;
                sides_res[i].contracts[m].over_tricks = 0;
                sides_res[i].contracts[m].under_tricks = best_par_sacut[i];
            }
        } else {
            /* Par contract is a makeable contract. */

            // The vendor's `dr = (best_par_score[i] < 0) ? 0 : 1`.
            let dr = i32::from(best_par_score[i] >= 0);

            let mut tu_max = 0;
            for m in 0..=4usize {
                t3[m] = if (dr + i as i32) % 2 == 0 {
                    res_table[DENOM_CONV[m]][0]
                } else {
                    res_table[DENOM_CONV[m]][1]
                };
                t4[m] = if (dr + i as i32) % 2 == 0 {
                    res_table[DENOM_CONV[m]][2]
                } else {
                    res_table[DENOM_CONV[m]][3]
                };
                let tu = if t3[m] > t4[m] { t3[m] } else { t4[m] };
                if tu > tu_max {
                    tu_max = tu;
                    denom_max = m;
                    /* Lowest if several denominations have max tricks. */
                }
            }

            for m in 0..no_of_denom[i] {
                let j = best_par[m][i].par_denom;
                let ju = j as usize;

                let t1 = if (dr + i as i32) % 2 != 0 {
                    res_table[DENOM_CONV[ju]][0]
                } else {
                    res_table[DENOM_CONV[ju]][1]
                };
                let t2 = if (dr + i as i32) % 2 != 0 {
                    res_table[DENOM_CONV[ju]][2]
                } else {
                    res_table[DENOM_CONV[ju]][3]
                };
                // The vendor also computes `tt = max(t1, t2)` here, unused
                // in this branch.

                side_seats(dr, i, t1, t2, m, &mut sides_res);

                let mut max_lower = if (denom_max as i32) < j {
                    best_par[m][i].par_tricks - tu_max - 1
                } else {
                    best_par[m][i].par_tricks - tu_max
                };

                /* max_lower is the maximal contract lowering, otherwise
                opponent contract is higher. It is already known that par_score
                is high enough to make opponent sacrifices futile.
                To find the actual contract lowering allowed, it must be
                checked that the lowered contract still gets the score bonus
                points that is present in par score. */

                // The vendor's `(best_par_score[i] >= 0 ? ... : -...)`.
                let sc2 = best_par_score[i].abs();
                /* Score for making the tentative lower par contract. */
                while max_lower > 0 {
                    let sc1 = if (denom_max as i32) < j {
                        -rawscore(
                            -1,
                            best_par[m][i].par_tricks - max_lower - tu_max,
                            vulner_def_side(i32::from(best_par_score[0] > 0), vulnerable),
                        )
                    } else {
                        -rawscore(
                            -1,
                            best_par[m][i].par_tricks - max_lower - tu_max + 1,
                            vulner_def_side(i32::from(best_par_score[0] > 0), vulnerable),
                        )
                    };
                    /* Score for undertricks needed to beat the tentative
                    lower par contract. */

                    if sc2 < sc1 {
                        break;
                    }
                    max_lower -= 1;

                    /* Tentative lower par contract must be 1 trick higher,
                    since the cost for the sacrifice is too small. */
                }

                let opp_tricks = t3[ju].max(t4[ju]);

                while max_lower > 0 {
                    let sc3 = -rawscore(
                        -1,
                        best_par[m][i].par_tricks - max_lower - opp_tricks,
                        vulner_def_side(i32::from(best_par_score[0] > 0), vulnerable),
                    );

                    /* If opponents to side with par score start the bidding
                    and has a sacrifice in the par denom on the same trick level
                    as implied by current max_lower, then max_lower must be
                    decremented. */

                    if sc2 > sc3 && best_par_score[i] < 0 {
                        /* Opposite side with best par score starts the bidding. */
                        max_lower -= 1;
                    } else {
                        break;
                    }
                }

                let k = match j {
                    0 => 0usize,
                    1 | 2 => 1,
                    3 | 4 => 2,
                    _ => unreachable!("the par denomination is 0..=4"),
                };

                max_lower = MAX_LOW[k][(best_par[m][i].par_tricks - 6) as usize].min(max_lower);

                sides_res[i].contracts[m].denom = j;
                sides_res[i].contracts[m].under_tricks = 0;

                calc_over_tricks(i, max_lower, best_par[m][i].par_tricks, m, &mut sides_res);

                sides_res[i].contracts[m].level =
                    best_par[m][i].par_tricks - 6 - sides_res[i].contracts[m].over_tricks;
            }
        }
    }

    /* Filter out par contracts where the other side has a higher par
    contract.  This can happen when par scores differ for the two sides. */

    let mut opp_side = [0usize; 2];

    let mut denom_to_remove = [[0i32; 5]; 2];

    let mut dom_denom = [-1i32; 2]; /* Dominating denom */

    let mut dom_level = [-1i32; 2]; /* Dominating level */

    for i in 0..2usize {
        let mut k = 0usize;
        opp_side[i] = usize::from(i == 0);

        while k < sides_res[opp_side[i]].number as usize {
            let j = sides_res[opp_side[i]].contracts[k].denom;
            let ss = sides_res[opp_side[i]].contracts[k].level
                + sides_res[opp_side[i]].contracts[k].over_tricks;

            if (ss > dom_level[opp_side[i]]
                || (ss == dom_level[opp_side[i]] && j < dom_denom[opp_side[i]]))
                && ((i == 0 && sides_res[opp_side[i]].contracts[k].seats % 2 != 0)
                    || (i == 1 && sides_res[opp_side[i]].contracts[k].seats % 2 == 0))
            {
                dom_denom[opp_side[i]] = j;
                dom_level[opp_side[i]] = sides_res[opp_side[i]].contracts[k].level
                    + sides_res[opp_side[i]].contracts[k].over_tricks;
            }
            k += 1;
        }
    }

    if dom_denom[0] != -1 && dom_denom[1] != -1 {
        /* Remove par contracts that can be dominated by the other side. */

        for i in 0..2usize {
            opp_side[i] = usize::from(i == 0);

            for k in 0..sides_res[i].number as usize {
                let j = sides_res[i].contracts[k].denom;

                if (sides_res[i].contracts[k].level + sides_res[i].contracts[k].over_tricks)
                    < dom_level[opp_side[i]]
                    || ((sides_res[i].contracts[k].level + sides_res[i].contracts[k].over_tricks)
                        == dom_level[opp_side[i]]
                        && dom_denom[opp_side[i]] < sides_res[i].contracts[k].denom)
                {
                    denom_to_remove[i][j as usize] = 1;
                }
            }

            let mut mm = 0usize;

            for k in 0..sides_res[i].number as usize {
                let j = sides_res[i].contracts[k].denom;
                if denom_to_remove[i][j as usize] != 1 {
                    sides_res[i].contracts[mm] = sides_res[i].contracts[k];
                    mm += 1;
                }
            }
            sides_res[i].number = mm as i32;
        }
    }

    sides_res
}

/// The C-string prefix of a NUL-padded buffer (the bytes before the first
/// NUL).
fn c_str(buf: &[u8; 10]) -> &[u8] {
    let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    &buf[..len]
}

/// Mirror of C `strstr` as a predicate: does the C string in `buf` contain
/// `needle`?
fn strstr(buf: &[u8; 10], needle: &[u8]) -> bool {
    c_str(buf)
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Mirror of C `strchr` as a predicate: does the C string in `buf` contain
/// the byte `ch`?
fn strchr(buf: &[u8; 10], ch: u8) -> bool {
    c_str(buf).contains(&ch)
}

/// Mirror of the vendor's `DealerParBin` (Par.cpp): runs the
/// [`dealer_par`] text engine and re-parses its strings at fixed character
/// offsets into binary form.
///
/// Quirk note (faithfully preserved): whether a contract is parsed as a
/// sacrifice depends on the *first* emitted string carrying a `*` marker at
/// byte 2.  Single-seat sacrifice strings like `5D-W-1` have no marker, so
/// on their own they parse as undoubled makes; when they follow a starred
/// pair sacrifice, the fixed-offset read lands on the NUL padding and
/// produces `underTricks = '\0' - '0'` = -48, which the [`Par`] conversion
/// then classifies as a make.  The FFI reference (`ddss`) exhibits exactly
/// the same behavior.
fn dealer_par_bin(res_table: &[[i32; 4]; 5], dealer: i32, vulnerable: i32) -> ParResultsMaster {
    /* dealer 0: North 1: East 2: South 3: West */
    /* vulnerable 0: None 1: Both 2: NS 3: EW */

    let mut presp = ParResultsMaster::default();
    let mut par_contr2 = [ParContr2Type::default(); 10];

    let par_res_dealer = dealer_par(res_table, dealer, vulnerable);

    if par_res_dealer.contracts[0][0] == b'p' {
        /* Passed out, i.e. no par contract can be found. */
        presp.number = 1;
        presp.score = 0;
        return presp;
    }

    for (contr2, text) in par_contr2
        .iter_mut()
        .zip(&par_res_dealer.contracts)
        .take(par_res_dealer.number as usize)
    {
        // The vendor copies the 10 chars one by one.
        contr2.contracts = *text;

        if text[1] == b'N' {
            contr2.denom = 0;
        } else if text[1] == b'S' {
            contr2.denom = 1;
        } else if text[1] == b'H' {
            contr2.denom = 2;
        } else if text[1] == b'D' {
            contr2.denom = 3;
        } else if text[1] == b'C' {
            contr2.denom = 4;
        }
    }

    for s in 1..par_res_dealer.number as usize {
        let tmp = par_contr2[s];
        let mut r = s;
        while r != 0 && tmp.denom < par_contr2[r - 1].denom {
            par_contr2[r] = par_contr2[r - 1];
            r -= 1;
        }
        par_contr2[r] = tmp;
    }

    presp.score = par_res_dealer.score;
    presp.number = par_res_dealer.number;

    for (contract, contr2) in presp
        .contracts
        .iter_mut()
        .zip(&par_contr2)
        .take(par_res_dealer.number as usize)
    {
        let mut delta = 1usize;

        contract.level = i32::from(contr2.contracts[0]) - i32::from(b'0');

        match contr2.contracts[1] {
            b'N' => contract.denom = 0,
            b'S' => contract.denom = 1,
            b'H' => contract.denom = 2,
            b'D' => contract.denom = 3,
            b'C' => contract.denom = 4,
            // The vendor returns RETURN_UNKNOWN_FAULT here.
            _ => unreachable!("denomination not in (NSHDC)"),
        }

        if strstr(&contr2.contracts, b"NS") {
            contract.seats = 4;
        } else if strstr(&contr2.contracts, b"EW") {
            contract.seats = 5;
        } else if strstr(&contr2.contracts, b"-N") {
            contract.seats = 0;
            delta = 0;
        } else if strstr(&contr2.contracts, b"-E") {
            contract.seats = 1;
            delta = 0;
        } else if strstr(&contr2.contracts, b"-S") {
            contract.seats = 2;
            delta = 0;
        } else if strstr(&contr2.contracts, b"-W") {
            contract.seats = 3;
            delta = 0;
        }

        if par_res_dealer.contracts[0][2] == b'*' {
            /* Sacrifice */
            contract.under_tricks = i32::from(contr2.contracts[6 + delta]) - i32::from(b'0');
            contract.over_tricks = 0;
        } else {
            /* Make */
            if strchr(&contr2.contracts, b'+') {
                contract.over_tricks = i32::from(contr2.contracts[5 + delta]) - i32::from(b'0');
            } else {
                contract.over_tricks = 0;
            }
            contract.under_tricks = 0;
        }
    }
    presp
}

/// Adapt a [`TrickCountTable`] (rows in ascending [`Strain`] order) to the
/// vendor's `ddTableResults::resTable` layout: strains in DDS order
/// (S, H, D, C, NT) by seats (N, E, S, W).
fn res_table_from(table: TrickCountTable) -> [[i32; 4]; 5] {
    const fn row(row: TrickCountRow) -> [i32; 4] {
        [
            row.get(Seat::North).get() as i32,
            row.get(Seat::East).get() as i32,
            row.get(Seat::South).get() as i32,
            row.get(Seat::West).get() as i32,
        ]
    }

    [
        row(table[Strain::Spades]),
        row(table[Strain::Hearts]),
        row(table[Strain::Diamonds]),
        row(table[Strain::Clubs]),
        row(table[Strain::Notrump]),
    ]
}

/// Calculate par score and contracts for a deal
///
/// - `tricks`: The number of tricks each seat can take as declarer for each strain
/// - `vul`: The vulnerability of pairs
/// - `dealer`: The dealer of the deal
///
/// The score is signed from North-South's point of view.  A pure, infallible
/// function — a faithful port of DDS's `DealerParBin` (via the `DealerPar`
/// text engine), matching the `ddss` FFI reference bit for bit.
#[must_use]
pub fn calculate_par(tricks: TrickCountTable, vul: Vulnerability, dealer: Seat) -> Par {
    dealer_par_bin(&res_table_from(tricks), dealer as i32, vul.to_dds()).into()
}

/// Calculate par scores for both pairs
///
/// - `tricks`: The number of tricks each seat can take as declarer for each strain
/// - `vul`: The vulnerability of pairs
///
/// Returns the par result with North-South (index 0) and East-West (index 1)
/// starting the bidding.  The two nearly always agree, but differ on "hot"
/// deals where both sides can make something.  A pure, infallible function —
/// a faithful port of DDS's `SidesParBin`, matching the `ddss` FFI reference
/// bit for bit.
#[must_use]
pub fn calculate_pars(tricks: TrickCountTable, vul: Vulnerability) -> [Par; 2] {
    sides_par_bin(&res_table_from(tricks), vul.to_dds()).map(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VULS: [Vulnerability; 4] = [
        Vulnerability::NONE,
        Vulnerability::NS,
        Vulnerability::EW,
        Vulnerability::ALL,
    ];

    /// Build a table from rows in ascending [`Strain`] order (C, D, H, S,
    /// NT), each row holding the tricks for seats N, E, S, W.
    fn table(rows: [[u8; 4]; 5]) -> TrickCountTable {
        TrickCountTable(rows.map(|[n, e, s, w]| TrickCountRow::new(n, e, s, w)))
    }

    /// Shorthand for the expected [`ParContract`]s.
    fn contract(text: &str, declarer: Seat, overtricks: i8) -> ParContract {
        ParContract {
            contract: text.parse().unwrap(),
            declarer,
            overtricks,
        }
    }

    /// No seat takes more than 6 tricks anywhere: passed out.  Both engines
    /// report score 0 with no contracts for every dealer and vulnerability.
    #[test]
    fn pass_out() {
        let tricks = table([[6, 5, 6, 5]; 5]);
        let expected = Par {
            score: 0,
            contracts: Vec::new(),
        };
        for vul in VULS {
            for dealer in Seat::ALL {
                assert_eq!(calculate_par(tricks, vul, dealer), expected);
            }
            assert_eq!(
                calculate_pars(tricks, vul),
                [expected.clone(), expected.clone()]
            );
        }
    }

    /// NS makes 4♥ from either seat and EW has no save: +420 nonvul, +620
    /// vul, with the pair expanded to both declarers.
    #[test]
    fn plain_game() {
        let tricks = table([
            [6, 3, 6, 3],
            [6, 3, 6, 3],
            [10, 3, 10, 3],
            [6, 3, 6, 3],
            [6, 3, 6, 3],
        ]);
        let expected = vec![
            contract("4H", Seat::North, 0),
            contract("4H", Seat::South, 0),
        ];
        for dealer in Seat::ALL {
            let par = calculate_par(tricks, Vulnerability::NONE, dealer);
            assert_eq!(par.score, 420);
            assert_eq!(par.contracts, expected);

            let par = calculate_par(tricks, Vulnerability::NS, dealer);
            assert_eq!(par.score, 620);
            assert_eq!(par.contracts, expected);
        }

        let pars = calculate_pars(tricks, Vulnerability::NONE);
        assert_eq!(pars[0].score, 420);
        assert_eq!(pars[0].contracts, expected);
        assert_eq!(pars[1].score, -420);
        assert_eq!(pars[1].contracts, expected);
        assert!(pars[0].equivalent(&calculate_par(tricks, Vulnerability::NONE, Seat::North)));
    }

    /// Only North takes 10 tricks in hearts (South takes 9), so the pair is
    /// not expanded: 4♥ by North alone.
    #[test]
    fn single_seat_game() {
        let tricks = table([
            [6, 3, 6, 3],
            [6, 3, 6, 3],
            [10, 3, 9, 3],
            [6, 3, 6, 3],
            [6, 3, 6, 3],
        ]);
        let expected = vec![contract("4H", Seat::North, 0)];
        let par = calculate_par(tricks, Vulnerability::NONE, Seat::North);
        assert_eq!(par.score, 420);
        assert_eq!(par.contracts, expected);

        let pars = calculate_pars(tricks, Vulnerability::NONE);
        assert_eq!(pars[0].score, 420);
        assert_eq!(pars[0].contracts, expected);
    }

    /// NS makes exactly 4♠, but EW takes 10 tricks in clubs: 5♣ doubled
    /// down one is a good save even vulnerable (−200 beats −420).
    #[test]
    fn doubled_sacrifice() {
        let tricks = table([
            [3, 10, 3, 10],
            [6, 5, 6, 5],
            [6, 5, 6, 5],
            [10, 3, 10, 3],
            [6, 5, 6, 5],
        ]);
        let expected = vec![
            contract("5Cx", Seat::East, -1),
            contract("5Cx", Seat::West, -1),
        ];
        for dealer in Seat::ALL {
            let par = calculate_par(tricks, Vulnerability::NONE, dealer);
            assert_eq!(par.score, 100);
            assert_eq!(par.contracts, expected);

            let par = calculate_par(tricks, Vulnerability::EW, dealer);
            assert_eq!(par.score, 200);
            assert_eq!(par.contracts, expected);
        }

        let pars = calculate_pars(tricks, Vulnerability::NONE);
        assert_eq!(pars[0].score, 100);
        assert_eq!(pars[0].contracts, expected);
        assert_eq!(pars[1].score, -100);
    }

    /// EW takes only 9 club tricks, so the save is down two: fine nonvul
    /// (−300 beats −420) but too dear vulnerable (−500), where par flips
    /// back to the 4♠ game.
    #[test]
    fn sacrifice_killed_by_vulnerability() {
        let tricks = table([
            [3, 9, 3, 9],
            [6, 5, 6, 5],
            [6, 5, 6, 5],
            [10, 3, 10, 3],
            [6, 5, 6, 5],
        ]);
        let par = calculate_par(tricks, Vulnerability::NONE, Seat::North);
        assert_eq!(par.score, 300);
        assert_eq!(
            par.contracts,
            vec![
                contract("5Cx", Seat::East, -2),
                contract("5Cx", Seat::West, -2),
            ]
        );

        let par = calculate_par(tricks, Vulnerability::EW, Seat::North);
        assert_eq!(par.score, 420);
        assert_eq!(
            par.contracts,
            vec![
                contract("4S", Seat::North, 0),
                contract("4S", Seat::South, 0),
            ]
        );
    }

    /// Every seat takes 7 tricks in notrump: whichever side opens the
    /// bidding claims 1NT, so the par score's sign follows the dealer.
    #[test]
    fn hot_deal() {
        let tricks = table([
            [6, 6, 6, 6],
            [6, 6, 6, 6],
            [6, 6, 6, 6],
            [6, 6, 6, 6],
            [7, 7, 7, 7],
        ]);
        for vul in VULS {
            let north = calculate_par(tricks, vul, Seat::North);
            assert_eq!(north.score, 90);
            assert_eq!(
                north.contracts,
                vec![
                    contract("1N", Seat::North, 0),
                    contract("1N", Seat::South, 0),
                ]
            );

            let east = calculate_par(tricks, vul, Seat::East);
            assert_eq!(east.score, -90);
            assert_eq!(
                east.contracts,
                vec![contract("1N", Seat::East, 0), contract("1N", Seat::West, 0)]
            );

            assert_eq!(north.score, -east.score);
        }

        // Each element of `calculate_pars` is scored for the side that
        // starts the bidding, so a hot deal is +90 from both views.
        let pars = calculate_pars(tricks, Vulnerability::NONE);
        assert_eq!(pars[0].score, 90);
        assert_eq!(pars[1].score, 90);
        assert_eq!(
            pars[1].contracts,
            vec![contract("1N", Seat::East, 0), contract("1N", Seat::West, 0)]
        );
    }

    /// North holds all spades, East all hearts, South all diamonds, West
    /// all clubs: NS bids the highest laydown grand slam, 7♠, and no
    /// sacrifice above it is affordable.
    #[test]
    fn laydown_grand_slam() {
        let tricks = table([
            [0, 13, 0, 13],
            [13, 0, 13, 0],
            [0, 13, 0, 13],
            [13, 0, 13, 0],
            [0, 0, 0, 0],
        ]);
        let expected = vec![
            contract("7S", Seat::North, 0),
            contract("7S", Seat::South, 0),
        ];
        for dealer in Seat::ALL {
            let par = calculate_par(tricks, Vulnerability::NONE, dealer);
            assert_eq!(par.score, 1510);
            assert_eq!(par.contracts, expected);
        }
        assert_eq!(
            calculate_par(tricks, Vulnerability::ALL, Seat::North).score,
            2210
        );

        let pars = calculate_pars(tricks, Vulnerability::NONE);
        assert_eq!(pars[0].score, 1510);
        assert_eq!(pars[0].contracts, expected);
        assert_eq!(pars[1].score, -1510);
        assert_eq!(pars[1].contracts, expected);
    }

    /// Only East can afford the same-strain save over 4♥ (5♥ down two).
    /// The emitted text `5H-E-2` carries no `*` marker, so `DealerParBin`
    /// misparses it as an undoubled make — while `SidesParBin` classifies
    /// the same save correctly as 5♥ doubled down two.  Both agree under
    /// [`Par::equivalent`], which exists for exactly this disagreement.
    #[test]
    fn single_seat_sacrifice_parse_quirk() {
        let tricks = table([
            [6, 5, 6, 5],
            [6, 5, 6, 5],
            [10, 9, 10, 5],
            [6, 5, 6, 5],
            [6, 5, 6, 5],
        ]);
        let par = calculate_par(tricks, Vulnerability::NONE, Seat::North);
        assert_eq!(par.score, 300);
        assert_eq!(par.contracts, vec![contract("5H", Seat::East, 0)]);

        let pars = calculate_pars(tricks, Vulnerability::NONE);
        assert_eq!(pars[0].score, 300);
        assert_eq!(pars[0].contracts, vec![contract("5Hx", Seat::East, -2)]);

        assert!(par.equivalent(&pars[0]));
        assert_ne!(par, pars[0]);

        // With East as dealer, East gets to bid hearts before NS shows
        // them, so the save is one level cheaper: 4♥ down one for +100.
        let par = calculate_par(tricks, Vulnerability::NONE, Seat::East);
        assert_eq!(par.score, 100);
        assert_eq!(par.contracts, vec![contract("4H", Seat::East, 0)]);
    }

    /// Two equally cheap saves against 4♥: 5♣ by either opponent (starred
    /// pair text) and 5♥ by East alone (unstarred single-seat text).  The
    /// first emitted string is starred, so `DealerParBin` parses *all*
    /// contracts as sacrifices; for `5H-E-2` the fixed-offset read lands on
    /// the NUL padding, yielding `underTricks` = -48, which the conversion
    /// classifies back as an undoubled make.  Byte-for-byte vendor behavior.
    #[test]
    fn nul_padding_parse_quirk() {
        let tricks = table([
            [3, 9, 3, 9],
            [6, 5, 6, 5],
            [10, 9, 10, 5],
            [6, 5, 6, 5],
            [6, 5, 6, 5],
        ]);
        let par = calculate_par(tricks, Vulnerability::NONE, Seat::North);
        assert_eq!(par.score, 300);
        assert_eq!(
            par.contracts,
            vec![
                contract("5H", Seat::East, 0),
                contract("5Cx", Seat::East, -2),
                contract("5Cx", Seat::West, -2),
            ]
        );
    }
}
