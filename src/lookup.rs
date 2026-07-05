//! Precomputed lookup tables used throughout the search.
//!
//! Ported from the vendor's `Init.cpp::InitConstants()`. The five
//! 8192-entry arrays consume ~800 KB of `.rodata`; they're built at
//! compile time by `const fn`s, so a hot-path read is a direct load
//! from a static — no lazy-init check, no pointer indirection (the
//! C++ reads plain globals; `LazyLock` here cost an atomic check per
//! access).

// ---- Hand-rotation constants ---------------------------------------
//
// All four hands are numbered 0=N, 1=E, 2=S, 3=W. The three rotation
// tables encode left-hand-opponent, right-hand-opponent, and partner
// for each starting hand.

/// Left-hand opponent of each seat.
pub const LHO: [usize; 4] = [1, 2, 3, 0];

/// Right-hand opponent of each seat.
pub const RHO: [usize; 4] = [3, 0, 1, 2];

/// Partner of each seat.
pub const PARTNER: [usize; 4] = [2, 3, 0, 1];

// ---- Rank bitmap ---------------------------------------------------
//
// `BIT_MAP_RANK[r]` is the single-bit suit-set representing rank `r`,
// for `r` in 2..=14. Indices 0/1 are zero. Index 15 mirrors index 14
// (the vendor explains this is "useful for some reason" — kept for
// porting fidelity).

pub const BIT_MAP_RANK: [u16; 16] = [
    0x0000, 0x0000, 0x0001, 0x0002, 0x0004, 0x0008, 0x0010, 0x0020, 0x0040, 0x0080, 0x0100, 0x0200,
    0x0400, 0x0800, 0x1000, 0x2000,
];

// ---- 8192-entry precomputed tables --------------------------------
//
// The five large tables. Each is indexed by an "aggr" — a 13-bit
// bitmap of which ranks (2..=14) are present in a suit.

/// `HIGHEST_RANK[aggr]` is the absolute rank (2..=14) of the highest
/// bit set in `aggr`, or 0 if `aggr == 0`.
pub static HIGHEST_RANK: [u8; 8192] = build_highest_rank();

const fn build_highest_rank() -> [u8; 8192] {
    let mut t = [0u8; 8192];
    let mut aggr = 1usize;
    while aggr < 8192 {
        let mut r = 14usize;
        while r >= 2 {
            if (aggr as u16) & BIT_MAP_RANK[r] != 0 {
                t[aggr] = r as u8;
                break;
            }
            r -= 1;
        }
        aggr += 1;
    }
    t
}

/// `LOWEST_RANK[aggr]` — symmetric to `HIGHEST_RANK`, finding the
/// lowest bit set.
pub static LOWEST_RANK: [u8; 8192] = build_lowest_rank();

const fn build_lowest_rank() -> [u8; 8192] {
    let mut t = [0u8; 8192];
    let mut aggr = 1usize;
    while aggr < 8192 {
        let mut r = 2usize;
        while r <= 14 {
            if (aggr as u16) & BIT_MAP_RANK[r] != 0 {
                t[aggr] = r as u8;
                break;
            }
            r += 1;
        }
        aggr += 1;
    }
    t
}

/// `COUNT_TABLE[aggr]` is `popcount(aggr)` for the low 13 bits.
/// Could be replaced with `aggr.count_ones()` at call sites; kept as a
/// table here for porting fidelity (the vendor reads this in a hot
/// loop and the table form preserves identical access patterns).
pub static COUNT_TABLE: [u8; 8192] = build_count_table();

const fn build_count_table() -> [u8; 8192] {
    let mut t = [0u8; 8192];
    let mut aggr = 0usize;
    while aggr < 8192 {
        t[aggr] = (aggr as u32).count_ones() as u8;
        aggr += 1;
    }
    t
}

/// `REL_RANK[aggr][abs_rank]` is the relative rank (1..=13) of
/// `abs_rank` (2..=14) in the suit represented by `aggr`. 1 is the
/// highest card present, 2 the second-highest, etc. Zero if the bit
/// for that absolute rank isn't set in `aggr`.
pub static REL_RANK: [[i8; 15]; 8192] = build_rel_rank();

const fn build_rel_rank() -> [[i8; 15]; 8192] {
    let mut t = [[0i8; 15]; 8192];
    let mut aggr = 1usize;
    while aggr < 8192 {
        let mut ord: i8 = 0;
        let mut r = 14usize;
        while r >= 2 {
            if (aggr as u16) & BIT_MAP_RANK[r] != 0 {
                ord += 1;
                t[aggr][r] = ord;
            }
            r -= 1;
        }
        aggr += 1;
    }
    t
}

/// `WIN_RANKS[aggr][least_win]` is the suit bitmap of the `least_win`
/// highest cards present in `aggr`. `least_win == 0` is always zero;
/// asking for more cards than are present saturates to all of `aggr`.
pub static WIN_RANKS: [[u16; 14]; 8192] = build_win_ranks();

const fn build_win_ranks() -> [[u16; 14]; 8192] {
    // Strip-top-bit recurrence: the top `lw` cards of `aggr` are its
    // top bit plus the top `lw - 1` cards of the rest.
    let mut t = [[0u16; 14]; 8192];
    let mut top_bit: u16 = 1;
    let mut aggr = 1usize;
    while aggr < 8192 {
        if aggr >= (top_bit << 1) as usize {
            top_bit <<= 1;
        }
        let rest = aggr ^ top_bit as usize;
        let mut lw = 1usize;
        while lw < 14 {
            t[aggr][lw] = top_bit | t[rest][lw - 1];
            lw += 1;
        }
        aggr += 1;
    }
    t
}

// ---- Move-group table ---------------------------------------------
//
// `MoveGroup` decomposes a suit's rank set into runs of adjacent bits.
// E.g. AKQ-J-987-2 → 4 groups. Used during move generation to merge
// equivalent moves (within a sequence, only the top card matters for
// the search; the rest are accumulated as `fullseq`).

/// A run-decomposition of one suit's rank bitmap.
#[derive(Clone, Copy, Debug)]
pub struct MoveGroup {
    /// Last valid index into `rank`/`sequence`/`fullseq`/`gap`. -1 if
    /// the source bitmap was empty.
    pub last_group: i8,
    /// Top rank of each group.
    pub rank: [u8; 7],
    /// Bits below the top of the group that are part of the sequence.
    pub sequence: [u16; 7],
    /// Top bit + sequence bits (the union, useful for the heuristic).
    pub fullseq: [u16; 7],
    /// Bitmap of the gap *below* this group (ranks not present between
    /// this group's bottom and the previous group's top).
    pub gap: [u16; 7],
}

impl MoveGroup {
    const fn empty() -> Self {
        Self {
            last_group: -1,
            rank: [0; 7],
            sequence: [0; 7],
            fullseq: [0; 7],
            gap: [0; 7],
        }
    }
}

/// `GROUP_DATA[ris]` decomposes the suit bitmap `ris` into groups.
pub static GROUP_DATA: [MoveGroup; 8192] = build_group_data();

const fn build_group_data() -> [MoveGroup; 8192] {
    // Topside[r] = bits for ranks strictly above r (in the 13-bit
    // rank-2..14 space). Botside[r] = bits strictly below.
    const TOPSIDE: [u16; 15] = [
        0x0000, 0x0000, 0x0000, 0x0001, 0x0003, 0x0007, 0x000f, 0x001f, 0x003f, 0x007f, 0x00ff,
        0x01ff, 0x03ff, 0x07ff, 0x0fff,
    ];
    const BOTSIDE: [u16; 15] = [
        0xffff, 0xffff, 0x1ffe, 0x1ffc, 0x1ff8, 0x1ff0, 0x1fe0, 0x1fc0, 0x1f80, 0x1f00, 0x1e00,
        0x1c00, 0x1800, 0x1000, 0x0000,
    ];

    let mut t = [MoveGroup::empty(); 8192];
    // Seed the singleton-bit case.
    t[1].last_group = 0;
    t[1].rank[0] = 2;
    t[1].sequence[0] = 0;
    t[1].fullseq[0] = 1;
    t[1].gap[0] = 0;

    let mut top_bit_rank: u16 = 1;
    let mut next_bit_rank: u16 = 0;
    let mut top_bit_no: usize = 2;

    let mut ris = 2usize;
    while ris < 8192 {
        if ris as u16 >= (top_bit_rank << 1) {
            next_bit_rank = top_bit_rank;
            top_bit_rank <<= 1;
            top_bit_no += 1;
        }

        // Start with the decomposition of (ris ^ top_bit_rank), then
        // either extend the last group or start a new one.
        t[ris] = t[ris ^ top_bit_rank as usize];

        if (ris as u16) & next_bit_rank != 0 {
            // Extend the existing topmost group.
            let g = t[ris].last_group as usize;
            t[ris].rank[g] += 1;
            t[ris].sequence[g] |= next_bit_rank;
            t[ris].fullseq[g] |= top_bit_rank;
        } else {
            // New group on top. When g == 0 there is no previous
            // group; the vendor's C++ reads `rank[-1]` here and relies
            // on adjacent zero-init struct memory yielding 0
            // (BOTSIDE[0] == 0xffff). Reproduce that explicitly.
            t[ris].last_group += 1;
            let g = t[ris].last_group as usize;
            t[ris].rank[g] = top_bit_no as u8;
            t[ris].sequence[g] = 0;
            t[ris].fullseq[g] = top_bit_rank;
            let prev_rank = if g == 0 { 0 } else { t[ris].rank[g - 1] };
            t[ris].gap[g] = TOPSIDE[top_bit_no] & BOTSIDE[prev_rank as usize];
        }
        ris += 1;
    }

    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highest_rank_known_values() {
        // AKQJ = bits 14,13,12,11 = 0x1e00
        assert_eq!(HIGHEST_RANK[0x1e00], 14);
        // singleton 2 = bit 2 = 0x0001
        assert_eq!(HIGHEST_RANK[0x0001], 2);
        // empty
        assert_eq!(HIGHEST_RANK[0], 0);
    }

    #[test]
    fn lowest_rank_known_values() {
        // AKQJ: lowest is J = 11
        assert_eq!(LOWEST_RANK[0x1e00], 11);
        // singleton A = 0x1000, lowest = 14
        assert_eq!(LOWEST_RANK[0x1000], 14);
        assert_eq!(LOWEST_RANK[0], 0);
    }

    #[test]
    fn count_table_matches_popcount() {
        for aggr in [0_usize, 1, 0x1fff, 0x1e00, 0xaaa] {
            assert_eq!(u32::from(COUNT_TABLE[aggr]), (aggr as u32).count_ones());
        }
    }

    #[test]
    fn rel_rank_simple_case() {
        // aggr = AKQ = 0x1c00 (bits 14, 13, 12). 14 is rel-1, 13 rel-2, 12 rel-3.
        let aggr = 0x1c00;
        assert_eq!(REL_RANK[aggr][14], 1);
        assert_eq!(REL_RANK[aggr][13], 2);
        assert_eq!(REL_RANK[aggr][12], 3);
        // Rank not present yields 0.
        assert_eq!(REL_RANK[aggr][11], 0);
    }

    #[test]
    fn win_ranks_take_top_n() {
        // AKQJT9 = bits 14,13,12,11,10,9 = 0x1f80 | 0x0080? Let's be
        // explicit: bits 14,13,12,11,10,9 → 0x1e00 | 0x0180 = 0x1f80.
        let aggr: usize = 0x1f80;
        // top 0 = none
        assert_eq!(WIN_RANKS[aggr][0], 0);
        // top 1 = just A (bit 14 = 0x1000)
        assert_eq!(WIN_RANKS[aggr][1], 0x1000);
        // top 3 = A,K,Q
        assert_eq!(WIN_RANKS[aggr][3], 0x1c00);
        // requesting more than present saturates at all bits
        assert_eq!(WIN_RANKS[aggr][13], aggr as u16);
    }

    #[test]
    fn group_data_single_bit() {
        // ris = 0x0001 (singleton 2) — one group, rank 2.
        let g = GROUP_DATA[0x0001];
        assert_eq!(g.last_group, 0);
        assert_eq!(g.rank[0], 2);
    }

    #[test]
    fn group_data_two_runs() {
        // ris = AK + 32 = 0x1c00 + 0x0008 (5 of clubs at bit 5? no
        // wait, our scheme is 2 at bit 0, 3 at bit 1, ..., 5 at bit 3).
        // AK = bits 13,14 → 0x1800 + 0x1000... let's just verify A+5.
        // A = bit 14 = 0x1000, 5 = bit 3 = 0x0008.
        let ris = 0x1008;
        let g = GROUP_DATA[ris];
        // Two groups: {A} and {5}.
        assert_eq!(g.last_group, 1);
        assert_eq!(g.rank[0], 5); // lower group first
        assert_eq!(g.rank[1], 14); // higher group second
    }
}
