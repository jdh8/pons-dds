//! Dealer-dependent par calculation (the text engine).
//!
//! Ported statement-for-statement from
//! [`DealerPar.cpp`](../../../ddss-sys/vendor/src/DealerPar.cpp).
//!
//! [`dealer_par`] finds the side entitled to a plus score (the *primacy*),
//! surveys its constructively-bid contracts, weighs them against the best
//! opposing sacrifice, and emits the par contracts as text strings such as
//! `4S-NS+1`, `5C*-NS-3`, or `5D-W-1` into the fixed `contracts[10][10]`
//! buffer of [`ParResultsDealer`].
//!
//! Fidelity note: the vendor `strcpy`s C strings into `char contracts[10][10]`
//! and `DealerParBin` (see `src/par.rs`) re-parses those buffers at fixed
//! byte offsets, sometimes reading past the string into the NUL padding
//! (e.g. for single-seat sacrifices like `5D-W-1`, which lack the `*`
//! marker). The buffers are therefore modeled as zero-initialized NUL-padded
//! byte arrays — never `String` — so the downstream parse is byte-for-byte
//! identical, quirks included.

#![allow(dead_code)] // TODO(par): consumed by `src/par.rs` in the next commit.

/// Number of strains (`#define DDS_STRAINS 5`).
const DDS_STRAINS: usize = 5;

/// First index: 0 nonvul, 1 vul. Second index: tricks down.
const DOUBLED_SCORES: [[i32; 14]; 2] = [
    [
        0, 100, 300, 500, 800, 1100, 1400, 1700, 2000, 2300, 2600, 2900, 3200, 3500,
    ],
    [
        0, 200, 500, 800, 1100, 1400, 1700, 2000, 2300, 2600, 2900, 3200, 3500, 3800,
    ],
];

/// First index is contract number, 0 is pass, 1 is 1C, ..., 35 is 7NT.
/// Second index is 0 nonvul, 1 vul.
const SCORES: [[i32; 2]; 36] = [
    [0, 0],
    [70, 70],
    [70, 70],
    [80, 80],
    [80, 80],
    [90, 90],
    [90, 90],
    [90, 90],
    [110, 110],
    [110, 110],
    [120, 120],
    [110, 110],
    [110, 110],
    [140, 140],
    [140, 140],
    [400, 600],
    [130, 130],
    [130, 130],
    [420, 620],
    [420, 620],
    [430, 630],
    [400, 600],
    [400, 600],
    [450, 650],
    [450, 650],
    [460, 660],
    [920, 1370],
    [920, 1370],
    [980, 1430],
    [980, 1430],
    [990, 1440],
    [1440, 2140],
    [1440, 2140],
    [1510, 2210],
    [1510, 2210],
    [1520, 2220],
];

/// First index is contract number, 0 .. 35.
/// Second index is vul: none, only defender, only declarer, both.
/// (The vendor comment names the two indices in the opposite order of the
/// actual `[36][4]` layout; the indexing below matches the vendor's usage.)
const DOWN_TARGET: [[i32; 4]; 36] = [
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [1, 0, 1, 0],
    [1, 0, 1, 0],
    [1, 0, 1, 0],
    [1, 0, 1, 0],
    [1, 0, 1, 0],
    [1, 0, 1, 0],
    [1, 0, 1, 0],
    [2, 1, 3, 2],
    [1, 0, 1, 0],
    [1, 0, 1, 0],
    [2, 1, 3, 2],
    [2, 1, 3, 2],
    [2, 1, 3, 2],
    [2, 1, 3, 2],
    [2, 1, 3, 2],
    [2, 1, 3, 2],
    [2, 1, 3, 2],
    [2, 1, 3, 2],
    [4, 3, 5, 4],
    [4, 3, 5, 4],
    [4, 3, 6, 5],
    [4, 3, 6, 5],
    [4, 3, 6, 5],
    [6, 5, 8, 7],
    [6, 5, 8, 7],
    [6, 5, 8, 7],
    [6, 5, 8, 7],
    [6, 5, 8, 7],
];

/// Lowest contract that a making contract may be reduced to without losing
/// its game or slam bonus, by contract number.
const FLOOR_CONTRACT: [i32; 36] = [
    0, 1, 2, 3, 4, 5, 1, 2, 3, 4, 5, 1, 2, 3, 4, 15, 1, 2, 18, 19, 15, 21, 22, 18, 19, 15, 26, 27,
    28, 29, 30, 31, 32, 33, 34, 35,
];

/// Contract number to text, 0 is pass, 1 is 1C, ..., 35 is 7NT.
const NUMBER_TO_CONTRACT: [&str; 36] = [
    "0", "1C", "1D", "1H", "1S", "1N", "2C", "2D", "2H", "2S", "2N", "3C", "3D", "3H", "3S", "3N",
    "4C", "4D", "4H", "4S", "4N", "5C", "5D", "5H", "5S", "5N", "6C", "6D", "6H", "6S", "6N", "7C",
    "7D", "7H", "7S", "7N",
];

/// Seat number to text, 0 is North, ..., 3 is West.
const NUMBER_TO_PLAYER: [&str; 4] = ["N", "E", "S", "W"];

/// First index is vul: none, both, NS, EW.
/// Second index is vul (0, 1) for NS and then EW.
const VUL_LOOKUP: [[i32; 2]; 4] = [[0, 0], [1, 1], [1, 0], [0, 1]];

/// First vul is declarer (not necessarily NS), second is defender.
const VUL_TO_NO: [[i32; 2]; 2] = [[0, 1], [2, 3]];

/// Maps par order (C, D, H, S, NT) to DDS order (S, H, D, C, NT).
///
/// (The vendor comment states the mapping in the opposite direction; every
/// use is `res_table[DENOM_ORDER[dno]]` with `dno` in par order.)
const DENOM_ORDER: [usize; 5] = [3, 2, 1, 0, 4];

/// Mirror of the vendor's `data_type`.
#[derive(Debug, Default, Clone, Copy)]
struct DataType {
    primacy: i32,
    /// Filled by [`survey_scores`] for vendor parity; never read afterwards
    /// (the vendor reads it only through the local in `survey_scores`).
    highest_making_no: i32,
    /// Filled for vendor parity like `highest_making_no`.
    dearest_making_no: i32,
    /// Filled for vendor parity like `highest_making_no`.
    dearest_score: i32,
    vul_no: i32,
}

/// Mirror of the vendor's `list_type`.
#[derive(Debug, Default, Clone, Copy)]
struct ListType {
    score: i32,
    dno: i32,
    /// Contract number. May be negative (vendor comment)!
    no: i32,
    tricks: i32,
    down: i32,
}

/// `#define BIGNUM 9999`.
const BIGNUM: i32 = 9999;

/// Mirror of `parResultsDealer` in `dll.h`.
///
/// - `number`: Number of contracts yielding the par score.
/// - `score`: Par score for the specified dealer hand.
/// - `contracts`: Par contract text strings as NUL-padded C strings.  The
///   first contract is in `contracts[0]`, the last one in
///   `contracts[number - 1]`.
#[derive(Debug, Default, Clone, Copy)]
pub struct ParResultsDealer {
    pub number: i32,
    pub score: i32,
    pub contracts: [[u8; 10]; 10],
}

/// Mirror of C `strcpy` into one of the fixed 10-byte buffers: copies the
/// bytes and one NUL terminator, leaving the (already zeroed) tail as is.
fn strcpy(dst: &mut [u8; 10], src: &str) {
    dst[..src.len()].copy_from_slice(src.as_bytes());
    dst[src.len()] = 0;
}

/// Mirror of the vendor's `DealerPar`.
///
/// - `res_table`: the `ddTableResults::resTable` layout — strains in DDS
///   order (S, H, D, C, NT) by seats (N, E, S, W).
/// - `dealer` — 0: North, 1: East, 2: South, 3: West.
/// - `vulnerable` — 0: None, 1: Both, 2: NS, 3: EW.
///
/// On a passed-out deal the vendor leaves `score` untouched; the
/// `Default`-zeroed struct pins it to 0, which is also what `DealerParBin`
/// reports for a pass-out.
pub fn dealer_par(res_table: &[[i32; 4]; 5], dealer: i32, vulnerable: i32) -> ParResultsDealer {
    let mut presp = ParResultsDealer::default();
    let vul_by_side = VUL_LOOKUP[vulnerable as usize];
    let mut data = DataType::default();
    let mut list = [[ListType::default(); DDS_STRAINS]; 2];

    /* First we find the side entitled to a plus score (primacy)
    and some statistics for each constructively bid (undoubled)
    contract that might be the par score. */

    let mut num_cand = 0;
    survey_scores(
        res_table,
        dealer,
        vul_by_side,
        &mut data,
        &mut num_cand,
        &mut list,
    );
    let side = data.primacy;

    if side == -1 {
        presp.number = 1;
        strcpy(&mut presp.contracts[0], "pass");
        return presp;
    }

    /* Go through the contracts, starting from the highest one. */
    // The vendor aliases `list_type * lists = list[side]`; the side's list
    // is indexed directly here to satisfy the borrow checker.
    let sd = side as usize;
    let vul_no = data.vul_no;
    let mut best_plus = 0;
    let mut down = 0;
    let mut sac_found = 0;

    let mut type_ = [0i32; DDS_STRAINS];
    let mut sac_gap = [0i32; DDS_STRAINS];
    let mut best_down = 0;
    let mut sacr = [[0i32; DDS_STRAINS]; DDS_STRAINS];

    for n in 0..num_cand as usize {
        let no = list[sd][n].no;
        let dno = list[sd][n].dno;
        let target = DOWN_TARGET[no as usize][vul_no as usize];

        best_sacrifice(
            res_table, side, no, dno, dealer, &list, &mut sacr, &mut down,
        );

        if down <= target {
            if down > best_down {
                best_down = down;
            }
            if sac_found != 0 {
                /* Declarer will never get a higher sacrifice by bidding
                less, so we can stop looking for sacrifices. But it
                can't be a worthwhile contract to bid, either. */
                type_[n] = -1;
            } else {
                sac_found = 1;
                type_[n] = 0;
                list[sd][n].down = down;
            }
        } else {
            if list[sd][n].score > best_plus {
                best_plus = list[sd][n].score;
            }
            type_[n] = 1;
            sac_gap[n] = target - down;
        }
    }

    let mut res_no = 0usize;
    let vul_def = vul_by_side[1 - sd];
    let sac = DOUBLED_SCORES[vul_def as usize][best_down as usize];

    if sac_found == 0 || best_plus > sac {
        /* The primacy side bids. */
        presp.score = if side == 0 { best_plus } else { -best_plus };

        for n in 0..num_cand as usize {
            if type_[n] != 1 || list[sd][n].score != best_plus {
                continue;
            }
            let mut no = list[sd][n].no;
            let mut plus = 0;
            reduce_contract(&mut no, sac_gap[n], &mut plus);

            strcpy(
                &mut presp.contracts[res_no],
                &contract_as_text(res_table, side, no, list[sd][n].dno, plus),
            );
            res_no += 1;
        }
    } else {
        /* The primacy side collects the penalty. */
        let sac_vul = vul_by_side[1 - sd];
        let sac_score = DOUBLED_SCORES[sac_vul as usize][best_down as usize];
        presp.score = if side == 0 { sac_score } else { -sac_score };

        for n in 0..num_cand as usize {
            if type_[n] != 0 || list[sd][n].down != best_down {
                continue;
            }
            sacrifices_as_text(
                res_table,
                side,
                dealer,
                best_down,
                list[sd][n].no,
                list[sd][n].dno,
                &list,
                &sacr,
                &mut presp.contracts,
                &mut res_no,
            );
        }
    }
    presp.number = res_no as i32;
    presp
}

/// Mirror of the vendor's `survey_scores`.
///
/// When this is done, `data` has added the following entries:
/// * `primacy` (0 or 1) is the side entitled to a plus score.
///   If the deal should be passed out, it is -1, and nothing
///   else is set.
/// * `highest_making_no` is a contract number (for that side)
/// * `dearest_making_no` is a contract number (for that side)
/// * `dearest_score` is the best score if there is no sacrifice
/// * `vul_no` is an index for a table, seen from the primacy
///
/// `list[side][dno]` has added the following entries:
/// * `score`
/// * `dno` is the denomination number
/// * `no` is a contract number
/// * `tricks` is the number of tricks embedded in the contract
///
/// For the primacy side, the list is sorted in descending
/// order of the contract number (`no`).
fn survey_scores(
    res_table: &[[i32; 4]; 5],
    dealer: i32,
    vul_by_side: [i32; 2],
    data: &mut DataType,
    num_candidates: &mut i32,
    list: &mut [[ListType; DDS_STRAINS]; 2],
) {
    let mut stats = [DataType::default(); 2];

    for side in 0..=1usize {
        let mut highest_making_no = 0;
        let mut dearest_making_no = 0;
        let mut dearest_score = 0;

        for dno in 0..DDS_STRAINS {
            let slist = &mut list[side][dno];
            let t = &res_table[DENOM_ORDER[dno]];
            let a = t[side];
            let b = t[side + 2];
            let best = if a > b { a } else { b };

            let no = 5 * (best - 7) + dno as i32 + 1;
            slist.no = no; /* May be negative! */

            if best < 7 {
                slist.score = 0;
                continue;
            }

            let score = SCORES[no as usize][vul_by_side[side] as usize];
            slist.score = score;
            slist.dno = dno as i32;
            slist.tricks = best;

            if score > dearest_score {
                dearest_score = score;
                dearest_making_no = no;
            } else if score == dearest_score && no < dearest_making_no {
                /* The lowest such, e.g. 3NT and 5C. */
                dearest_making_no = no;
            }

            if no > highest_making_no {
                highest_making_no = no;
            }
        }
        let sside = &mut stats[side];
        sside.highest_making_no = highest_making_no;
        sside.dearest_making_no = dearest_making_no;
        sside.dearest_score = dearest_score;
    }

    let mut primacy: i32 = 0;
    let s0 = stats[0].highest_making_no;
    let s1 = stats[1].highest_making_no;
    if s0 > s1 {
        primacy = 0;
    } else if s0 < s1 {
        primacy = 1;
    } else if s0 == 0 {
        data.primacy = -1;
        return;
    } else {
        /* Special case, depends who can bid it first. */
        let dno = ((s0 - 1) % 5) as usize;
        let t_max = list[0][dno].tricks;
        let t = &res_table[DENOM_ORDER[dno]];

        for pno in dealer..=dealer + 3 {
            if t[(pno % 4) as usize] != t_max {
                continue;
            }
            primacy = pno % 2;
            break;
        }
    }

    let sside = &stats[primacy as usize];

    let dm_no = sside.dearest_making_no;
    data.primacy = primacy;
    data.highest_making_no = sside.highest_making_no;
    data.dearest_making_no = dm_no;
    data.dearest_score = sside.dearest_score;

    let vul_primacy = vul_by_side[primacy as usize];
    let vul_other = vul_by_side[1 - primacy as usize];
    data.vul_no = VUL_TO_NO[vul_primacy as usize][vul_other as usize];

    /* Sort the scores in descending order of contract number,
    i.e. first by score and second by contract number in case
    the score is the same. Primitive bubble sort... */
    let mut n = DDS_STRAINS;
    loop {
        let mut new_n = 0;
        for i in 1..n {
            if list[primacy as usize][i - 1].no > list[primacy as usize][i].no {
                continue;
            }
            list[primacy as usize].swap(i - 1, i);

            new_n = i;
        }
        n = new_n;
        if n == 0 {
            break;
        }
    }

    *num_candidates = DDS_STRAINS as i32;
    for entry in &list[primacy as usize] {
        if entry.no < dm_no {
            *num_candidates -= 1;
        }
    }
}

/// Mirror of the vendor's `best_sacrifice`.
// The `> 35` overbid check ends both branches in the vendor too; keep the
// duplication for line-by-line correspondence.
#[allow(clippy::too_many_arguments, clippy::branches_sharing_code)]
fn best_sacrifice(
    res_table: &[[i32; 4]; 5],
    side: i32,
    no: i32,
    dno: i32,
    dealer: i32,
    list: &[[ListType; DDS_STRAINS]; 2],
    sacr_table: &mut [[i32; DDS_STRAINS]; DDS_STRAINS],
    best_down: &mut i32,
) {
    let other = (1 - side) as usize;
    let sacr_list = &list[other];
    *best_down = BIGNUM;

    for eno in 0..=4usize {
        let sacr = sacr_list[eno];
        let mut down = BIGNUM;

        if eno as i32 == dno {
            let t_max = (no + 34) / 5;
            let t = &res_table[DENOM_ORDER[dno as usize]];
            let mut incr_flag = 0;
            for pno in dealer..=dealer + 3 {
                let diff = t_max - t[(pno % 4) as usize];
                let s = pno % 2;
                if s == side {
                    if diff == 0 {
                        incr_flag = 1;
                    }
                } else {
                    let local = diff + incr_flag;
                    if local < down {
                        down = local;
                    }
                }
            }
            if sacr.no + 5 * down > 35 {
                down = BIGNUM;
            }
        } else {
            down = (no - sacr.no + 4) / 5;
            if sacr.no + 5 * down > 35 {
                down = BIGNUM;
            }
        }
        sacr_table[dno as usize][eno] = down;
        if down < *best_down {
            *best_down = down;
        }
    }
}

/// Mirror of the vendor's `sacrifices_as_text`.
#[allow(clippy::too_many_arguments)]
fn sacrifices_as_text(
    res_table: &[[i32; 4]; 5],
    side: i32,
    dealer: i32,
    best_down: i32,
    no_decl: i32,
    dno: i32,
    list: &[[ListType; DDS_STRAINS]; 2],
    sacr: &[[i32; DDS_STRAINS]; DDS_STRAINS],
    results: &mut [[u8; 10]; 10],
    res_no: &mut usize,
) {
    let other = (1 - side) as usize;
    let sacr_list = &list[other];

    for eno in 0..=4usize {
        let mut down = sacr[dno as usize][eno];
        if down != best_down {
            continue;
        }

        if eno as i32 != dno {
            let no_sac = sacr_list[eno].no + 5 * best_down;
            strcpy(
                &mut results[*res_no],
                &contract_as_text(res_table, other as i32, no_sac, eno as i32, -best_down),
            );
            *res_no += 1;
            continue;
        }

        let t_max = (no_decl + 34) / 5;
        let t = &res_table[DENOM_ORDER[dno as usize]];
        let mut incr_flag = 0;
        let mut p_hit = 0usize;
        let mut pno_list = [0i32; 2];
        let mut sac_list = [0i32; 2];
        for pno in dealer..=dealer + 3 {
            let pno_mod = pno % 4;
            let diff = t_max - t[pno_mod as usize];
            let s = pno % 2;
            if s == side {
                if diff == 0 {
                    incr_flag = 1;
                }
            } else {
                down = diff + incr_flag;
                if down != best_down {
                    continue;
                }
                pno_list[p_hit] = pno_mod;
                sac_list[p_hit] = no_decl + 5 * incr_flag;
                p_hit += 1;
            }
        }

        let ns0 = sac_list[0];
        if p_hit == 1 {
            strcpy(
                &mut results[*res_no],
                &sacrifice_as_text(ns0, pno_list[0], best_down),
            );
            *res_no += 1;
            continue;
        }

        let ns1 = sac_list[1];
        if ns0 == ns1 {
            /* Both players */
            strcpy(
                &mut results[*res_no],
                &contract_as_text(res_table, other as i32, ns0, eno as i32, -best_down),
            );
            *res_no += 1;
            continue;
        }

        let p = usize::from(ns0 >= ns1);
        strcpy(
            &mut results[*res_no],
            &sacrifice_as_text(sac_list[p], pno_list[p], best_down),
        );
        *res_no += 1;
    }
}

/// Mirror of the vendor's `reduce_contract`.
///
/// Could be that we found 4C just making, but it would be
/// enough to bid 2C +2. But we don't want to bid so low that
/// we lose a game or slam bonus.
const fn reduce_contract(no: &mut i32, sac_gap: i32, plus: &mut i32) {
    if sac_gap >= -1 {
        /* No scope to reduce. */
        *plus = 0;
        return;
    }

    /* This is the lowest contract that we could reduce to. */
    let flr = FLOOR_CONTRACT[*no as usize];

    /* As such, declarer could reduce the contract by down+1 levels
    (where down is negative) and still the opponent's sacrifice
    would not turn profitable. But for non-vulnerable partials,
    this can go wrong: 1M+1 and 2M= both pay +90, but 3m*-2
    is a bad sacrifice against 2M=, while 2m*-1 would be a good
    sacrifice against 1M+1. */
    let no_sac_level = *no + 5 * (sac_gap + 1);
    let new_no = if no_sac_level > flr {
        no_sac_level
    } else {
        flr
    };
    *plus = (*no - new_no) / 5;
    *no = new_no;
}

/// Mirror of the vendor's `contract_as_text`, e.g. `4S-NS+1` or `5C*-NS-3`.
fn contract_as_text(res_table: &[[i32; 4]; 5], side: i32, no: i32, dno: i32, delta: i32) -> String {
    let t = &res_table[DENOM_ORDER[dno as usize]];
    let ta = t[side as usize];
    let tb = t[side as usize + 2];
    let t_max = if ta > tb { ta } else { tb };

    let mut text = String::from(NUMBER_TO_CONTRACT[no as usize]);
    text.push_str(if delta < 0 { "*-" } else { "-" });
    if ta == t_max {
        text.push_str(NUMBER_TO_PLAYER[side as usize]);
    }
    if tb == t_max {
        text.push_str(NUMBER_TO_PLAYER[side as usize + 2]);
    }
    if delta > 0 {
        text.push('+');
    }
    if delta != 0 {
        text.push_str(&delta.to_string());
    }
    text
}

/// Mirror of the vendor's `sacrifice_as_text`, e.g. `5D-W-1`.
///
/// Note that this single-seat form carries no `*` marker; `DealerParBin`
/// depends on that (mis)feature.
fn sacrifice_as_text(no: i32, pno: i32, down: i32) -> String {
    format!(
        "{}-{}-{}",
        NUMBER_TO_CONTRACT[no as usize], NUMBER_TO_PLAYER[pno as usize], down
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use contract_bridge::Strain;
    use contract_bridge::contract::{Contract, Penalty};

    /// Every making entry of `SCORES` matches `contract-bridge`'s independent
    /// duplicate-scoring implementation.  Contract number `no` (1..=35)
    /// encodes level `(no - 1) / 5 + 1` and strain `(no - 1) % 5` in
    /// ascending par order (C, D, H, S, NT); the score is for making exactly.
    #[test]
    fn scores_match_contract_bridge() {
        for (no, row) in SCORES.iter().enumerate().skip(1) {
            let level = (no - 1) / 5 + 1;
            let strain = Strain::ASC[(no - 1) % 5];
            let contract = Contract::new(level as u8, strain, Penalty::Undoubled);
            for (vul, &expected) in row.iter().enumerate() {
                assert_eq!(
                    expected,
                    contract.score(level as u8 + 6, vul == 1),
                    "SCORES[{no}][{vul}]"
                );
            }
        }
    }

    /// Every `DOUBLED_SCORES` entry matches `contract-bridge`'s doubled
    /// undertrick scoring.  The doubled penalty depends only on the
    /// undertrick count and the defenders' vulnerability, so any doubled
    /// contract (here 7NT) going down by that count must agree.  Index 0
    /// (a make) is skipped: the vendor keeps a 0 placeholder there.
    #[test]
    fn doubled_scores_match_contract_bridge() {
        let contract = Contract::new(7, Strain::Notrump, Penalty::Doubled);
        for (vul, row) in DOUBLED_SCORES.iter().enumerate() {
            for (down, &expected) in row.iter().enumerate().skip(1) {
                assert_eq!(
                    expected,
                    -contract.score(13 - down as u8, vul == 1),
                    "DOUBLED_SCORES[{vul}][{down}]"
                );
            }
        }
    }

    /// C-string view of a contract buffer for assertions.
    fn text(buf: &[u8; 10]) -> &str {
        let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        core::str::from_utf8(&buf[..len]).unwrap()
    }

    /// All cells at 6 tricks or fewer: passed out for every dealer and
    /// vulnerability.  The score stays at the `Default` 0 (the vendor leaves
    /// it unset on a pass-out).
    #[test]
    fn pass_out() {
        let res_table = [[6, 5, 6, 5]; 5];
        for dealer in 0..4 {
            for vulnerable in 0..4 {
                let res = dealer_par(&res_table, dealer, vulnerable);
                assert_eq!(res.number, 1);
                assert_eq!(res.score, 0);
                assert_eq!(text(&res.contracts[0]), "pass");
            }
        }
    }

    /// NS makes 4♥ with both seats (10 tricks each); everything else is low.
    /// No profitable sacrifice exists, so the game is the par contract for
    /// every dealer: +420 nonvul, +620 vul.
    #[test]
    fn plain_game() {
        // DDS row order S, H, D, C, NT; seats N, E, S, W.
        let res_table = [
            [6, 3, 6, 3],
            [10, 3, 10, 3],
            [6, 3, 6, 3],
            [6, 3, 6, 3],
            [6, 3, 6, 3],
        ];
        for dealer in 0..4 {
            let res = dealer_par(&res_table, dealer, 0);
            assert_eq!(res.number, 1);
            assert_eq!(res.score, 420);
            assert_eq!(text(&res.contracts[0]), "4H-NS");

            /* Both vulnerable: the same contract, now worth 620. */
            let res = dealer_par(&res_table, dealer, 1);
            assert_eq!(res.score, 620);
            assert_eq!(text(&res.contracts[0]), "4H-NS");
        }
    }

    /// NS makes exactly 4♠; EW takes 10 tricks in clubs and saves in 5♣
    /// doubled, down one — cheaper than −420 even vulnerable (−200).
    #[test]
    fn doubled_sacrifice() {
        let res_table = [
            [10, 3, 10, 3],
            [6, 5, 6, 5],
            [6, 5, 6, 5],
            [3, 10, 3, 10],
            [6, 5, 6, 5],
        ];
        let res = dealer_par(&res_table, 0, 0);
        assert_eq!(res.number, 1);
        assert_eq!(res.score, 100);
        assert_eq!(text(&res.contracts[0]), "5C*-EW-1");

        /* Only EW vulnerable: down one doubled now costs 200. */
        let res = dealer_par(&res_table, 0, 3);
        assert_eq!(res.score, 200);
        assert_eq!(text(&res.contracts[0]), "5C*-EW-1");
    }

    /// EW takes only 9 club tricks, so the save is down two: still a
    /// sacrifice nonvul (−300 beats −420), but vulnerable it would cost 500,
    /// so par flips back to the 4♠ game.
    #[test]
    fn sacrifice_killed_by_vulnerability() {
        let res_table = [
            [10, 3, 10, 3],
            [6, 5, 6, 5],
            [6, 5, 6, 5],
            [3, 9, 3, 9],
            [6, 5, 6, 5],
        ];
        let res = dealer_par(&res_table, 0, 0);
        assert_eq!((res.score, text(&res.contracts[0])), (300, "5C*-EW-2"));

        let res = dealer_par(&res_table, 0, 3);
        assert_eq!((res.score, text(&res.contracts[0])), (420, "4S-NS"));
    }

    /// Only East can afford the same-strain sacrifice over 4♥ (West would go
    /// down six), producing the single-seat text form without the `*` marker.
    /// With East as dealer, East gets to bid hearts before NS shows them, so
    /// the save is a level cheaper.
    #[test]
    fn single_seat_sacrifice() {
        let res_table = [
            [6, 5, 6, 5],
            [10, 9, 10, 5],
            [6, 5, 6, 5],
            [6, 5, 6, 5],
            [6, 5, 6, 5],
        ];
        let res = dealer_par(&res_table, 0, 0);
        assert_eq!(res.number, 1);
        assert_eq!((res.score, text(&res.contracts[0])), (300, "5H-E-2"));

        let res = dealer_par(&res_table, 1, 0);
        assert_eq!((res.score, text(&res.contracts[0])), (100, "4H-E-1"));
    }

    /// Two equally cheap sacrifices against 4♥: 5♣ by either opponent
    /// (starred pair form) and 5♥ by East alone (single-seat form), emitted
    /// in par denomination order (clubs first).
    #[test]
    fn mixed_sacrifices() {
        let res_table = [
            [6, 5, 6, 5],
            [10, 9, 10, 5],
            [6, 5, 6, 5],
            [3, 9, 3, 9],
            [6, 5, 6, 5],
        ];
        let res = dealer_par(&res_table, 0, 0);
        assert_eq!(res.number, 2);
        assert_eq!(res.score, 300);
        assert_eq!(text(&res.contracts[0]), "5C*-EW-2");
        assert_eq!(text(&res.contracts[1]), "5H-E-2");
    }

    /// Both sides make 1NT (7 tricks in every seat): the par goes to
    /// whichever side gets to bid it first — the dealer's.
    #[test]
    fn hot_deal() {
        let res_table = [
            [6, 6, 6, 6],
            [6, 6, 6, 6],
            [6, 6, 6, 6],
            [6, 6, 6, 6],
            [7, 7, 7, 7],
        ];
        for vulnerable in 0..4 {
            let res = dealer_par(&res_table, 0, vulnerable);
            assert_eq!((res.score, text(&res.contracts[0])), (90, "1N-NS"));
            let res = dealer_par(&res_table, 1, vulnerable);
            assert_eq!((res.score, text(&res.contracts[0])), (-90, "1N-EW"));
        }
    }

    /// Each seat holds one entire suit: NS makes 7♠ (and 7♦), EW makes 7♥
    /// (and 7♣).  NS holds the highest grand slam and no sacrifice above it
    /// is affordable, so par is the vulnerability-adjusted 7♠ score.
    #[test]
    fn laydown_grand_slam() {
        let res_table = [
            [13, 0, 13, 0],
            [0, 13, 0, 13],
            [13, 0, 13, 0],
            [0, 13, 0, 13],
            [0, 0, 0, 0],
        ];
        for dealer in 0..4 {
            let res = dealer_par(&res_table, dealer, 0);
            assert_eq!((res.score, text(&res.contracts[0])), (1510, "7S-NS"));
            let res = dealer_par(&res_table, dealer, 1);
            assert_eq!((res.score, text(&res.contracts[0])), (2210, "7S-NS"));
        }
    }
}
