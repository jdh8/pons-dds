//! Move generation and heuristic ordering.
//!
//! Ported from [`Moves.cpp`](../../../ddss-sys/vendor/src/Moves.cpp) and
//! its header [`Moves.h`](../../../ddss-sys/vendor/src/Moves.h). The
//! [`Moves`] struct manages per-trick state during the alpha-beta search:
//! generating candidate plays, assigning each move a heuristic weight,
//! and yielding them in priority order via [`Moves::make_next`].
//!
//! The vendor classes `Moves`, `trickDataType`, and `trackType` collapse
//! into the types below. Statistics/printing helpers from the vendor are
//! omitted — they don't affect search behaviour, only debug output.
//!
//! Naming differs from the vendor: `numMoves` → `num_moves`,
//! `trackp` → `track_index` (a `usize`), etc. Field semantics and the
//! magic-number weights in the `weight_alloc_*` helpers are preserved
//! verbatim.
//!
//! # Trump handling
//!
//! The vendor's `Moves::Init` takes `trump` as a parameter. Since the
//! caller-driven `Solver` doesn't exist yet (Phase 4), [`Moves::new`]
//! defaults trump to `DDS_NOTRUMP` (4) and a separate
//! [`Moves::set_trump`] setter lets the eventual `Solver` configure it.
//! [`Moves::init_removed_ranks`] does the same job as the
//! `removedRanks` initialization in `Init`.

use crate::lookup::{
    BIT_MAP_RANK, COUNT_TABLE, GROUP_DATA, HIGHEST_RANK, LHO, LOWEST_RANK, PARTNER, REL_RANK, RHO,
};
use crate::move_type::{MovePly, MoveType};
use crate::pos::{MAX_DEPTH, Pos};

/// The vendor's `DDS_NOTRUMP` magic value (4).
pub const DDS_NOTRUMP: i32 = 4;
/// Number of hands, four.
const DDS_HANDS: usize = 4;
/// Number of suits, four.
const DDS_SUITS: usize = 4;

// ---------------------------------------------------------------------
// Auxiliary types
// ---------------------------------------------------------------------

/// Vendor's `absRankType`: which seat holds the rank at this offset.
///
/// Packed to the vendor's 2-byte layout (`char rank; signed char
/// hand;`, dds.h) — the 8192-entry [`RelRanks`] table is randomly
/// probed on the search's hot path, so its footprint (960 KiB at this
/// size, 4x that with `i32` fields) decides whether it lives in cache.
/// `rank` is 0..=14 and `hand` is -1..=3, so `i8` is lossless.
#[derive(Clone, Copy, Debug, Default)]
pub struct AbsRank {
    pub rank: i8,
    pub hand: i8,
}

/// Vendor's `relRanksType`: for each (`rank_index`, suit) entry, the seat
/// (and absolute rank) holding the `rank_index`-th highest card in
/// `suit`, *given* the current aggr bitmap.
///
/// Indexed in the caller by `aggr[suit]`. The whole table has 8192
/// entries when precomputed; the move generator only ever reads
/// `absRank[3][suit].hand`.
#[derive(Clone, Copy, Debug, Default)]
pub struct RelRanks {
    pub abs_rank: [[AbsRank; DDS_SUITS]; 15],
}

/// The whole point of the `i8` packing: one entry per cache line pair,
/// vendor parity (`relRanksType` is 120 bytes).
const _: () = assert!(size_of::<RelRanks>() == 120);

/// `trackType` from the vendor — per-trick scratchpad.
#[derive(Clone, Copy, Debug)]
pub struct Track {
    pub lead_hand: i32,
    pub lead_suit: i32,
    pub play_suits: [i32; DDS_HANDS],
    pub play_ranks: [i32; DDS_HANDS],
    pub trick_data: TrickData,
    pub move_played: [ExtCard; DDS_HANDS],
    pub high: [i32; DDS_HANDS],
    pub lowest_win: [[i32; DDS_SUITS]; DDS_HANDS],
    /// Bitmap of cards no longer in play, per suit. Initialized from
    /// the `Pos` and updated as the search advances tricks.
    pub removed_ranks: [i32; DDS_SUITS],
}

impl Default for Track {
    fn default() -> Self {
        Self {
            lead_hand: 0,
            lead_suit: 0,
            play_suits: [0; DDS_HANDS],
            play_ranks: [0; DDS_HANDS],
            trick_data: TrickData::default(),
            move_played: [ExtCard::default(); DDS_HANDS],
            high: [0; DDS_HANDS],
            lowest_win: [[0; DDS_SUITS]; DDS_HANDS],
            removed_ranks: [0; DDS_SUITS],
        }
    }
}

/// Vendor's `trickDataType` — populated by `GetTrickData`.
#[derive(Clone, Copy, Debug, Default)]
pub struct TrickData {
    pub play_count: [i32; DDS_SUITS],
    pub best_rank: i32,
    pub best_suit: i32,
    pub best_sequence: i32,
    pub rel_winner: i32,
}

/// Vendor's `extCard` — like [`MoveType`] but without a weight field.
#[derive(Clone, Copy, Debug, Default)]
pub struct ExtCard {
    pub suit: i32,
    pub rank: i32,
    pub sequence: i32,
}

// ---------------------------------------------------------------------
// Moves
// ---------------------------------------------------------------------

/// Per-trick move generation and heuristic ordering.
///
/// Mirrors the vendor's `Moves` class. One instance lives inside the
/// eventual `Solver` and is reused across recursive search frames.
#[allow(clippy::struct_field_names)]
pub struct Moves {
    // ---- Current ply scratch (matches vendor's member ints) ----
    lead_hand: i32,
    lead_suit: i32,
    curr_hand: i32,
    /// Suit currently being processed by a `weight_alloc_*` helper.
    suit: i32,
    curr_trick: i32,
    trump: i32,
    num_moves: i32,
    last_num_moves: i32,
    /// Index into `track`; analogue of the vendor's `trackp`.
    track_index: usize,

    /// Per-trick state. `Box`-allocated to keep the [`Moves`] struct
    /// small on the search stack.
    track: Box<[Track; MAX_DEPTH]>,

    /// Move lists, indexed by `[trick][hand_rel]`. Each entry stores up
    /// to 14 candidate moves plus a `current` cursor consumed by
    /// [`Moves::make_next`].
    move_list: Box<[[MovePly; DDS_HANDS]; MAX_DEPTH]>,
}

impl Moves {
    /// Allocate a fresh move generator. Trump defaults to notrump;
    /// callers should follow up with [`Moves::set_trump`] for trump
    /// contracts.
    pub(crate) fn new() -> Self {
        Self {
            lead_hand: 0,
            lead_suit: 0,
            curr_hand: 0,
            suit: 0,
            curr_trick: 0,
            trump: DDS_NOTRUMP,
            num_moves: 0,
            last_num_moves: 0,
            track_index: 0,
            track: Box::new([Track::default(); MAX_DEPTH]),
            move_list: Box::new([[MovePly::default(); DDS_HANDS]; MAX_DEPTH]),
        }
    }

    /// Configure the trump strain. Use [`DDS_NOTRUMP`] for notrump.
    pub(crate) const fn set_trump(&mut self, trump: i32) {
        self.trump = trump;
    }

    /// Returns the trump strain currently in effect.
    #[allow(dead_code)]
    pub(crate) const fn trump(&self) -> i32 {
        self.trump
    }

    /// Initialize `removed_ranks[tricks]` from the hand bitmaps in
    /// `tpos`. Analogue of the loop in vendor's `Init` that turns the
    /// initial position into the played-cards bitmap.
    pub(crate) fn init_removed_ranks(&mut self, tricks: i32, tpos: &Pos) {
        let idx = tricks as usize;
        for s in 0..DDS_SUITS {
            // 0xffff is what the vendor starts with; we XOR off
            // whatever each hand holds, leaving the set of cards
            // not in any hand (i.e. already played).
            let mut removed: i32 = 0xffff;
            for h in 0..DDS_HANDS {
                removed ^= i32::from(tpos.rank_in_suit[h][s]);
            }
            self.track[idx].removed_ranks[s] = removed;
        }
    }

    /// Set the lead hand for `tricks` (used at the start of a new trick
    /// during the search). The vendor's `Reinit`.
    pub(crate) fn reinit(&mut self, tricks: i32, lead_hand: i32) {
        self.track[tricks as usize].lead_hand = lead_hand;
    }

    /// Length of the candidate list for `(trick, rel_hand)`.
    #[allow(dead_code)]
    pub(crate) fn get_length(&self, trick: i32, rel_hand: i32) -> i32 {
        self.move_list[trick as usize][rel_hand as usize].last + 1
    }

    /// Step the `current` cursor of `(tricks, rel_hand)` forward.
    #[allow(dead_code)]
    pub(crate) fn step(&mut self, tricks: i32, rel_hand: i32) {
        self.move_list[tricks as usize][rel_hand as usize].current += 1;
    }

    /// Reset the `current` cursor of `(tricks, rel_hand)` to zero.
    #[allow(dead_code)]
    pub(crate) fn rewind(&mut self, tricks: i32, rel_hand: i32) {
        self.move_list[tricks as usize][rel_hand as usize].current = 0;
    }

    /// Reward the last move returned from `(tricks, rel_hand)` by
    /// bumping its weight by 100. Mirrors `Moves::Reward`.
    #[allow(dead_code)]
    pub(crate) fn reward(&mut self, tricks: i32, rel_hand: i32) {
        let mp = &mut self.move_list[tricks as usize][rel_hand as usize];
        let idx = (mp.current - 1) as usize;
        mp.moves[idx].weight += 100;
    }

    /// Populate `trick_data` for the trick at `tricks` and return it.
    pub(crate) fn get_trick_data(&mut self, tricks: i32) -> TrickData {
        let track = &mut self.track[self.track_index];
        let mut data = TrickData::default();
        for relh in 0..DDS_HANDS {
            data.play_count[track.play_suits[relh] as usize] += 1;
        }
        let sum: i32 = data.play_count.iter().sum();
        debug_assert_eq!(sum, 4, "play_count sum must be 4 at end of trick");
        data.best_rank = track.move_played[3].rank;
        data.best_suit = track.move_played[3].suit;
        data.best_sequence = track.move_played[3].sequence;
        data.rel_winner = track.high[3];
        let _ = tricks; // mirrors vendor; arg only there for parity
        track.trick_data = data;
        data
    }

    // -----------------------------------------------------------------
    // Move generation — leader (hand 0)
    // -----------------------------------------------------------------

    /// Generate candidate moves for the trick's leader.
    ///
    /// Returns the number of moves added to `moveList[tricks][0]`.
    #[inline]
    pub(crate) fn move_gen_0(
        &mut self,
        tricks: i32,
        tpos: &Pos,
        best_move: &MoveType,
        best_move_tt: &MoveType,
        thrp_rel: &[RelRanks],
    ) -> i32 {
        self.track_index = tricks as usize;
        self.lead_hand = self.track[self.track_index].lead_hand;
        self.curr_hand = self.lead_hand;
        self.curr_trick = tricks;

        let tidx = self.track_index;
        for s in 0..DDS_SUITS {
            self.track[tidx].lowest_win[0][s] = 0;
        }
        self.num_moves = 0;

        let ftest = self.trump != DDS_NOTRUMP && tpos.winner[self.trump as usize].rank != 0;

        let trick_usize = tricks as usize;
        // Borrow the move list slot for the leader.
        // We'll re-borrow inside the loop because the WeightAlloc
        // helpers below need to call &mut self for unrelated state.
        for suit in 0..DDS_SUITS {
            self.suit = suit as i32;
            let ris = tpos.rank_in_suit[self.lead_hand as usize][suit];
            if ris == 0 {
                continue;
            }
            self.last_num_moves = self.num_moves;

            let mp = GROUP_DATA[ris as usize];
            let mut g = i32::from(mp.last_group);
            let removed = self.track[tidx].removed_ranks[suit];

            let list = &mut self.move_list[trick_usize][0];
            while g >= 0 {
                let mut seq = i32::from(mp.sequence[g as usize]);
                let rank = i32::from(mp.rank[g as usize]);
                while g >= 1
                    && (i32::from(mp.gap[g as usize]) & removed) == i32::from(mp.gap[g as usize])
                {
                    g -= 1;
                    seq |= i32::from(mp.fullseq[g as usize]);
                }
                let k = self.num_moves as usize;
                list.moves[k].sequence = seq;
                list.moves[k].suit = suit as i32;
                list.moves[k].rank = rank;
                self.num_moves += 1;
                g -= 1;
            }

            if ftest {
                self.weight_alloc_trump0(tpos, best_move, best_move_tt, thrp_rel);
            } else {
                self.weight_alloc_nt0(tpos, best_move, best_move_tt, thrp_rel);
            }
        }

        let list = &mut self.move_list[trick_usize][0];
        list.current = 0;
        list.last = self.num_moves - 1;
        if self.num_moves != 1 {
            self.sort_active_list(trick_usize, 0);
        }
        self.num_moves
    }

    // -----------------------------------------------------------------
    // Move generation — followers (hands 1, 2, 3)
    // -----------------------------------------------------------------

    /// Generate candidate moves for follower hands (1, 2 or 3 in
    /// trick-order). Returns the number of moves added.
    #[inline]
    pub(crate) fn move_gen_123(&mut self, tricks: i32, hand_rel: i32, tpos: &Pos) -> i32 {
        self.track_index = tricks as usize;
        let tidx = self.track_index;
        self.lead_hand = self.track[tidx].lead_hand;
        self.curr_hand = (self.lead_hand + hand_rel) & 3;
        self.curr_trick = tricks;
        self.lead_suit = self.track[tidx].lead_suit;

        for s in 0..DDS_SUITS {
            self.track[tidx].lowest_win[hand_rel as usize][s] = 0;
        }
        self.num_moves = 0;

        let ftest =
            i32::from(self.trump != DDS_NOTRUMP && tpos.winner[self.trump as usize].rank != 0);

        let trick_usize = tricks as usize;
        let hand_usize = hand_rel as usize;

        let ris_lead = tpos.rank_in_suit[self.curr_hand as usize][self.lead_suit as usize];

        if ris_lead != 0 {
            // Follower can follow suit.
            let mp = GROUP_DATA[ris_lead as usize];
            let mut g = i32::from(mp.last_group);
            let removed = self.track[tidx].removed_ranks[self.lead_suit as usize];

            let list = &mut self.move_list[trick_usize][hand_usize];
            while g >= 0 {
                let mut seq = i32::from(mp.sequence[g as usize]);
                let rank = i32::from(mp.rank[g as usize]);
                while g >= 1
                    && (i32::from(mp.gap[g as usize]) & removed) == i32::from(mp.gap[g as usize])
                {
                    g -= 1;
                    seq |= i32::from(mp.fullseq[g as usize]);
                }
                let k = self.num_moves as usize;
                list.moves[k].sequence = seq;
                list.moves[k].suit = self.lead_suit;
                list.moves[k].rank = rank;
                self.num_moves += 1;
                g -= 1;
            }

            let findex = 4 * hand_rel + ftest;

            let list = &mut self.move_list[trick_usize][hand_usize];
            list.current = 0;
            list.last = self.num_moves - 1;
            if self.num_moves == 1 {
                return self.num_moves;
            }

            self.dispatch_weight_alloc(findex, tpos);
            self.sort_active_list(trick_usize, hand_usize);
            return self.num_moves;
        }

        // Follower is void in the led suit — generate discards across
        // all suits.
        let findex = 4 * hand_rel + ftest + 2;

        for suit in 0..DDS_SUITS {
            self.suit = suit as i32;
            let ris = tpos.rank_in_suit[self.curr_hand as usize][suit];
            if ris == 0 {
                continue;
            }
            self.last_num_moves = self.num_moves;

            let mp = GROUP_DATA[ris as usize];
            let mut g = i32::from(mp.last_group);
            let removed = self.track[tidx].removed_ranks[suit];

            let list = &mut self.move_list[trick_usize][hand_usize];
            while g >= 0 {
                let mut seq = i32::from(mp.sequence[g as usize]);
                let rank = i32::from(mp.rank[g as usize]);
                while g >= 1
                    && (i32::from(mp.gap[g as usize]) & removed) == i32::from(mp.gap[g as usize])
                {
                    g -= 1;
                    seq |= i32::from(mp.fullseq[g as usize]);
                }
                let k = self.num_moves as usize;
                list.moves[k].sequence = seq;
                list.moves[k].suit = suit as i32;
                list.moves[k].rank = rank;
                self.num_moves += 1;
                g -= 1;
            }

            self.dispatch_weight_alloc(findex, tpos);
        }

        let list = &mut self.move_list[trick_usize][hand_usize];
        list.current = 0;
        list.last = self.num_moves - 1;
        if self.num_moves != 1 {
            self.sort_active_list(trick_usize, hand_usize);
        }
        self.num_moves
    }

    /// Vendor's `WeightList[findex]` dispatch table.
    #[inline]
    fn dispatch_weight_alloc(&mut self, findex: i32, tpos: &Pos) {
        match findex {
            4 => self.weight_alloc_nt_notvoid1(tpos),
            5 => self.weight_alloc_trump_notvoid1(tpos),
            6 => self.weight_alloc_nt_void1(tpos),
            7 => self.weight_alloc_trump_void1(tpos),
            8 => self.weight_alloc_nt_notvoid2(tpos),
            9 => self.weight_alloc_trump_notvoid2(tpos),
            10 => self.weight_alloc_nt_void2(tpos),
            11 => self.weight_alloc_trump_void2(tpos),
            12 | 13 => self.weight_alloc_combined_notvoid3(tpos),
            14 => self.weight_alloc_nt_void3(tpos),
            15 => self.weight_alloc_trump_void3(tpos),
            _ => panic!("invalid weight-function index {findex}"),
        }
    }

    // -----------------------------------------------------------------
    // make_next, make_specific, purge, sort, etc.
    // -----------------------------------------------------------------

    /// Fetch the next candidate move from `(trick, rel_hand)` in
    /// heuristic order, applying the "lowest winning rank" filter from
    /// `win_ranks`. Returns `None` once the list is exhausted.
    ///
    /// Updates `track[trick]` to record the chosen move and, when
    /// `rel_hand == 3`, propagates `removed_ranks` into the next trick.
    #[inline]
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub(crate) fn make_next(
        &mut self,
        trick: i32,
        rel_hand: i32,
        win_ranks: &[u16; DDS_SUITS],
    ) -> Option<MoveType> {
        let trick_usize = trick as usize;
        let hand_usize = rel_hand as usize;
        let list = &mut self.move_list[trick_usize][hand_usize];

        if list.last == -1 {
            return None;
        }

        self.track_index = trick_usize;
        let mut current_move: MoveType;

        let mut found = list.current == 0;
        if found {
            current_move = list.moves[0];
        } else {
            let prev_idx = (list.current - 1) as usize;
            let prev_suit = list.moves[prev_idx].suit as usize;
            let prev_rank = list.moves[prev_idx].rank;
            let lwp = &mut self.track[trick_usize].lowest_win[hand_usize];

            if lwp[prev_suit] == 0 {
                let mut low = i32::from(LOWEST_RANK[win_ranks[prev_suit] as usize]);
                if low == 0 {
                    low = 15;
                }
                if prev_rank < low {
                    lwp[prev_suit] = low;
                }
            }

            current_move = MoveType::default();
            let list = &mut self.move_list[trick_usize][hand_usize];
            while list.current <= list.last && !found {
                let cur_idx = list.current as usize;
                let curr = list.moves[cur_idx];
                let lw = self.track[trick_usize].lowest_win[hand_usize][curr.suit as usize];
                if curr.rank >= lw {
                    found = true;
                    current_move = curr;
                } else {
                    list.current += 1;
                }
            }

            if !found {
                return None;
            }
        }

        // Record the chosen move on the per-trick track.
        self.record_chosen_move(trick, rel_hand, &current_move);

        // Bump the cursor.
        self.move_list[trick_usize][hand_usize].current += 1;
        Some(current_move)
    }

    /// `MakeNextSimple` from the vendor — like [`Moves::make_next`] but
    /// without the `winRanks` low-rank filtering.
    #[allow(dead_code)]
    pub(crate) fn make_next_simple(&mut self, trick: i32, rel_hand: i32) -> Option<MoveType> {
        let trick_usize = trick as usize;
        let hand_usize = rel_hand as usize;
        let list = &self.move_list[trick_usize][hand_usize];
        if list.current > list.last {
            return None;
        }
        let current_move = list.moves[list.current as usize];

        self.track_index = trick_usize;
        self.record_chosen_move(trick, rel_hand, &current_move);

        if rel_hand == 3 {
            // The simple variant only updates leadHand, not removedRanks.
            let prev_lead = self.track[trick_usize].lead_hand;
            let high3 = self.track[trick_usize].high[3];
            if trick_usize > 0 {
                self.track[trick_usize - 1].lead_hand = (prev_lead + high3) & 3;
            }
        }

        self.move_list[trick_usize][hand_usize].current += 1;
        Some(current_move)
    }

    /// Update `track[trick]` to record `mv` being played by `rel_hand`.
    /// Handles `move_played`, `high`, `lead_suit`, `play_suits`,
    /// `play_ranks`, plus the trick-completion propagation.
    fn record_chosen_move(&mut self, trick: i32, rel_hand: i32, mv: &MoveType) {
        let trick_usize = trick as usize;
        let track = &mut self.track[trick_usize];
        let r = rel_hand as usize;

        if rel_hand == 0 {
            track.move_played[0].suit = mv.suit;
            track.move_played[0].rank = mv.rank;
            track.move_played[0].sequence = mv.sequence;
            track.high[0] = 0;
            track.lead_suit = mv.suit;
        } else if mv.suit == track.move_played[r - 1].suit {
            if mv.rank > track.move_played[r - 1].rank {
                track.move_played[r].suit = mv.suit;
                track.move_played[r].rank = mv.rank;
                track.move_played[r].sequence = mv.sequence;
                track.high[r] = rel_hand;
            } else {
                track.move_played[r] = track.move_played[r - 1];
                track.high[r] = track.high[r - 1];
            }
        } else if mv.suit == self.trump {
            track.move_played[r].suit = mv.suit;
            track.move_played[r].rank = mv.rank;
            track.move_played[r].sequence = mv.sequence;
            track.high[r] = rel_hand;
        } else {
            track.move_played[r] = track.move_played[r - 1];
            track.high[r] = track.high[r - 1];
        }

        track.play_suits[r] = mv.suit;
        track.play_ranks[r] = mv.rank;

        if rel_hand == 3 && trick_usize > 0 {
            let cur = self.track[trick_usize];
            let new_lead = (cur.lead_hand + cur.high[3]) & 3;
            let next = &mut self.track[trick_usize - 1];
            next.lead_hand = new_lead;
            for s in 0..DDS_SUITS {
                next.removed_ranks[s] = cur.removed_ranks[s];
            }
            for h in 0..DDS_HANDS {
                let rk = cur.play_ranks[h];
                let su = cur.play_suits[h];
                next.removed_ranks[su as usize] |= i32::from(BIT_MAP_RANK[rk as usize]);
            }
        }
    }

    /// Apply a specific predetermined move (no heuristic search). Used
    /// by the search to follow a user-specified card.
    #[allow(dead_code)]
    pub(crate) fn make_specific(&mut self, mv: &MoveType, trick: i32, rel_hand: i32) {
        self.track_index = trick as usize;
        self.record_chosen_move(trick, rel_hand, mv);
    }

    /// Drop any move in `(tricks, rel_hand)` whose `(suit, rank)`
    /// appears in `forbidden_moves`. The vendor iterates over a fixed
    /// 14-slot list starting at index 1; we accept a slice and skip
    /// entries whose rank is zero (sentinel for "no move").
    #[inline]
    pub(crate) fn purge(&mut self, tricks: i32, rel_hand: i32, forbidden_moves: &[MoveType]) {
        let list = &mut self.move_list[tricks as usize][rel_hand as usize];
        // Match the vendor's loop bound: it iterates k=1..=13, so the
        // first slot is skipped. We replicate that exactly here.
        for mv in &forbidden_moves[1..forbidden_moves.len().min(14)] {
            let s = mv.suit;
            let rank = mv.rank;
            if rank == 0 {
                continue;
            }
            // Walk the live moves; on a match, shift everything down.
            let mut r = 0i32;
            while r <= list.last {
                if list.moves[r as usize].suit == s && list.moves[r as usize].rank == rank {
                    for n in (r as usize)..(list.last as usize) {
                        list.moves[n] = list.moves[n + 1];
                    }
                    list.last -= 1;
                    // Don't bump r — the slot we just overwrote may
                    // also be a duplicate forbidden move.
                } else {
                    r += 1;
                }
            }
        }
    }

    /// Sort the *active* list (`numMoves` entries) in-place by descending weight.
    ///
    /// `move_gen_0` / `move_gen_123` set up `num_moves` and `mply` (here:
    /// `(trick, hand_rel)`) before calling this. Public callers should
    /// use [`Moves::sort`] instead, which sets up the state for them.
    fn sort_active_list(&mut self, trick: usize, hand_rel: usize) {
        let list = &mut self.move_list[trick][hand_rel];
        merge_sort(&mut list.moves, self.num_moves as usize);
    }

    /// Sort `(tricks, rel_hand)`'s candidate list in descending weight
    /// order — the vendor's `Sort`.
    #[allow(dead_code)]
    pub(crate) fn sort(&mut self, tricks: i32, rel_hand: i32) {
        let trick_usize = tricks as usize;
        let hand_usize = rel_hand as usize;
        let list = &mut self.move_list[trick_usize][hand_usize];
        self.num_moves = list.last + 1;
        merge_sort(&mut list.moves, self.num_moves as usize);
    }

    /// Public `MergeSort` wrapper as required by the spec.
    #[allow(dead_code)]
    pub(crate) fn merge_sort(&mut self) {
        let trick_usize = self.curr_trick as usize;
        // Without a tracked "current rel-hand", we sort the list that
        // was last populated. The vendor uses the implicit `mply`
        // pointer; in our port `move_gen_*` sorts immediately after
        // generating, so this method exists only to satisfy the spec
        // surface for now. Pick rel-hand 0 conservatively.
        if (self.curr_trick as usize) < self.move_list.len() {
            let list = &mut self.move_list[trick_usize][0];
            merge_sort(&mut list.moves, self.num_moves as usize);
        }
    }

    /// Returns true if the winning side of `mvp1` beats `mvp2` under
    /// `our_trump`. Pure helper; mirrors vendor's `WinningMove`.
    #[allow(dead_code)]
    pub(crate) const fn winning_move(mvp1: &MoveType, mvp2: &ExtCard, our_trump: i32) -> bool {
        if mvp1.suit == mvp2.suit {
            mvp1.rank > mvp2.rank
        } else {
            mvp1.suit == our_trump
        }
    }

    // -----------------------------------------------------------------
    // WeightAlloc — leader, trump
    // -----------------------------------------------------------------

    /// Vendor `WeightAllocTrump0` — leader weighting in a trump
    /// contract. Magic numbers preserved verbatim.
    fn weight_alloc_trump0(
        &mut self,
        tpos: &Pos,
        best_move: &MoveType,
        best_move_tt: &MoveType,
        thrp_rel: &[RelRanks],
    ) {
        let lead = self.lead_hand as usize;
        let lho_i = LHO[lead];
        let rho_i = RHO[lead];
        let partner_i = PARTNER[lead];
        let trump = self.trump as usize;
        let suit = self.suit as usize;

        let suit_count = i32::from(tpos.length[lead][suit]);
        let suit_count_lh = i32::from(tpos.length[lho_i][suit]);
        let suit_count_rh = i32::from(tpos.length[rho_i][suit]);
        let aggr = tpos.aggr[suit] as usize;

        let count_lh = if suit_count_lh == 0 {
            self.curr_trick + 1
        } else {
            suit_count_lh
        } << 2;
        let count_rh = if suit_count_rh == 0 {
            self.curr_trick + 1
        } else {
            suit_count_rh
        } << 2;
        let suit_weight_d = -(((count_lh + count_rh) << 5) / 13);

        let trick_usize = self.curr_trick as usize;
        for k in self.last_num_moves..self.num_moves {
            let k_usize = k as usize;
            let mut suit_bonus = 0i32;
            let mut win_move = false;

            let mply = &self.move_list[trick_usize][0].moves;
            let cur_rank = mply[k_usize].rank;
            let cur_sequence = mply[k_usize].sequence;
            let r_rank = i32::from(REL_RANK[aggr][cur_rank as usize]);

            // Discourage suit if LHO or RHO can ruff.
            if suit != trump
                && ((tpos.rank_in_suit[lho_i][suit] == 0 && tpos.rank_in_suit[lho_i][trump] != 0)
                    || (tpos.rank_in_suit[rho_i][suit] == 0
                        && tpos.rank_in_suit[rho_i][trump] != 0))
            {
                suit_bonus = -12;
            }

            // Encourage suit if partner can ruff.
            if suit != trump
                && tpos.length[partner_i][suit] == 0
                && tpos.length[partner_i][trump] > 0
                && suit_count_rh > 0
            {
                suit_bonus += 17;
            }

            // Discourage suit if RHO has high card.
            if tpos.winner[suit].hand == rho_i as i32 || tpos.second_best[suit].hand == rho_i as i32
            {
                if suit_count_rh != 1 {
                    suit_bonus += -12;
                }
            } else if tpos.winner[suit].hand == lho_i as i32
                && tpos.second_best[suit].hand == partner_i as i32
            {
                // Joël Bradmetz case.
                if tpos.length[partner_i][suit] != 1 {
                    suit_bonus += 27;
                }
            }

            // Partner wins and returns for a ruff.
            if suit != trump
                && suit_count == 1
                && tpos.length[lead][trump] > 0
                && tpos.length[partner_i][suit] > 1
                && tpos.winner[suit].hand == partner_i as i32
            {
                suit_bonus += 19;
            }

            let mut suit_weight_delta = suit_bonus + suit_weight_d;

            // Determine win_move.
            if tpos.winner[suit].rank == cur_rank {
                if suit == trump {
                    win_move = true;
                } else if tpos.length[partner_i][suit] != 0 || tpos.length[partner_i][trump] == 0 {
                    if (tpos.length[lho_i][suit] != 0 || tpos.length[lho_i][trump] == 0)
                        && (tpos.length[rho_i][suit] != 0 || tpos.length[rho_i][trump] == 0)
                    {
                        win_move = true;
                    }
                } else if (tpos.length[lho_i][suit] != 0
                    || tpos.rank_in_suit[partner_i][trump] > tpos.rank_in_suit[lho_i][trump])
                    && (tpos.length[rho_i][suit] != 0
                        || tpos.rank_in_suit[partner_i][trump] > tpos.rank_in_suit[rho_i][trump])
                {
                    win_move = true;
                }
            } else if tpos.rank_in_suit[partner_i][suit]
                > (tpos.rank_in_suit[lho_i][suit] | tpos.rank_in_suit[rho_i][suit])
            {
                if suit == trump
                    || ((tpos.length[lho_i][suit] != 0 || tpos.length[lho_i][trump] == 0)
                        && (tpos.length[rho_i][suit] != 0 || tpos.length[rho_i][trump] == 0))
                {
                    win_move = true;
                }
            } else if suit != trump
                && tpos.length[partner_i][suit] == 0
                && tpos.length[partner_i][trump] != 0
            {
                if tpos.length[lho_i][suit] == 0
                    && tpos.length[lho_i][trump] != 0
                    && tpos.length[rho_i][suit] == 0
                    && tpos.length[rho_i][trump] != 0
                {
                    if tpos.rank_in_suit[partner_i][trump]
                        > (tpos.rank_in_suit[lho_i][trump] | tpos.rank_in_suit[rho_i][trump])
                    {
                        win_move = true;
                    }
                } else if tpos.length[lho_i][suit] == 0 && tpos.length[lho_i][trump] != 0 {
                    if tpos.rank_in_suit[partner_i][trump] > tpos.rank_in_suit[lho_i][trump] {
                        win_move = true;
                    }
                } else if tpos.length[rho_i][suit] == 0 && tpos.length[rho_i][trump] != 0 {
                    if tpos.rank_in_suit[partner_i][trump] > tpos.rank_in_suit[rho_i][trump] {
                        win_move = true;
                    }
                } else {
                    win_move = true;
                }
            }

            let mply = &mut self.move_list[trick_usize][0].moves;
            let cur = mply[k_usize];
            let new_weight: i32;
            if win_move {
                // Ruff opponent's singleton highest card.
                if (suit_count_lh == 1 && tpos.winner[suit].hand == lho_i as i32)
                    || (suit_count_rh == 1 && tpos.winner[suit].hand == rho_i as i32)
                {
                    new_weight = suit_weight_delta + 35 + r_rank;
                } else if tpos.winner[suit].hand == lead as i32 {
                    if tpos.second_best[suit].hand == partner_i as i32 {
                        new_weight = suit_weight_delta + 48 + r_rank;
                    } else if tpos.winner[suit].rank == cur_rank {
                        new_weight = suit_weight_delta + 31;
                    } else {
                        new_weight = suit_weight_delta - 3 + r_rank;
                    }
                } else if tpos.winner[suit].hand == partner_i as i32 {
                    if tpos.second_best[suit].hand == lead as i32 {
                        new_weight = suit_weight_delta + 42 + r_rank;
                    } else {
                        new_weight = suit_weight_delta + 28 + r_rank;
                    }
                } else if cur_sequence != 0 && cur_rank == tpos.second_best[suit].rank {
                    new_weight = suit_weight_delta + 40;
                } else if cur_sequence != 0 {
                    new_weight = suit_weight_delta + 22 + r_rank;
                } else {
                    new_weight = suit_weight_delta + 11 + r_rank;
                }
                mply[k_usize].weight = new_weight;
                if best_move.suit == suit as i32 && best_move.rank == cur.rank {
                    mply[k_usize].weight += 55;
                } else if best_move_tt.suit == suit as i32 && best_move_tt.rank == cur.rank {
                    mply[k_usize].weight += 18;
                }
            } else {
                let third_best_hand =
                    Self::third_best_hand_or_zero(thrp_rel, aggr, suit, partner_i, lead);

                if tpos.second_best[suit].hand == partner_i as i32
                    && partner_i as i32 == third_best_hand
                {
                    suit_weight_delta += 20;
                } else if (tpos.second_best[suit].hand == lead as i32
                    && partner_i as i32 == third_best_hand
                    && tpos.length[partner_i][suit] > 1)
                    || (tpos.second_best[suit].hand == partner_i as i32
                        && lead as i32 == third_best_hand
                        && tpos.length[partner_i][suit] > 1)
                {
                    suit_weight_delta += 13;
                }

                if (suit_count_lh == 1 && tpos.winner[suit].hand == lho_i as i32)
                    || (suit_count_rh == 1 && tpos.winner[suit].hand == rho_i as i32)
                {
                    mply[k_usize].weight = suit_weight_delta + r_rank + 2;
                } else if tpos.winner[suit].hand == lead as i32 {
                    if tpos.second_best[suit].hand == partner_i as i32 {
                        mply[k_usize].weight = suit_weight_delta + 33 + r_rank;
                    } else if tpos.winner[suit].rank == cur_rank {
                        mply[k_usize].weight = suit_weight_delta + 38;
                    } else {
                        mply[k_usize].weight = suit_weight_delta - 14 + r_rank;
                    }
                } else if tpos.winner[suit].hand == partner_i as i32 {
                    mply[k_usize].weight = suit_weight_delta + 34 + r_rank;
                } else if cur_sequence != 0 && cur_rank == tpos.second_best[suit].rank {
                    mply[k_usize].weight = suit_weight_delta + 35;
                } else {
                    mply[k_usize].weight = suit_weight_delta + 17 - cur_rank;
                }

                if best_move.suit == suit as i32 && best_move.rank == cur.rank {
                    mply[k_usize].weight += 18;
                }
            }
        }
    }

    /// Pull `thrp_rel[aggr].absRank[3][suit].hand` defensively. Vendor
    /// indexes blindly; we treat an empty `thrp_rel` slice as "0".
    fn third_best_hand_or_zero(
        thrp_rel: &[RelRanks],
        aggr: usize,
        suit: usize,
        _partner: usize,
        _lead: usize,
    ) -> i32 {
        if aggr < thrp_rel.len() {
            i32::from(thrp_rel[aggr].abs_rank[3][suit].hand)
        } else {
            0
        }
    }

    // -----------------------------------------------------------------
    // WeightAlloc — leader, notrump
    // -----------------------------------------------------------------

    fn weight_alloc_nt0(
        &mut self,
        tpos: &Pos,
        best_move: &MoveType,
        best_move_tt: &MoveType,
        thrp_rel: &[RelRanks],
    ) {
        let lead = self.lead_hand as usize;
        let lho_i = LHO[lead];
        let rho_i = RHO[lead];
        let partner_i = PARTNER[lead];
        let suit = self.suit as usize;

        let aggr = tpos.aggr[suit] as usize;
        let suit_count_lh = i32::from(tpos.length[lho_i][suit]);
        let suit_count_rh = i32::from(tpos.length[rho_i][suit]);

        let count_lh = if suit_count_lh == 0 {
            self.curr_trick + 1
        } else {
            suit_count_lh
        } << 2;
        let count_rh = if suit_count_rh == 0 {
            self.curr_trick + 1
        } else {
            suit_count_rh
        } << 2;
        let mut suit_weight_d = -(((count_lh + count_rh) << 5) / 19);
        if tpos.length[partner_i][suit] == 0 {
            suit_weight_d += -9;
        }

        let trick_usize = self.curr_trick as usize;
        for k in self.last_num_moves..self.num_moves {
            let k_usize = k as usize;
            let mut suit_weight_delta = suit_weight_d;
            let mply = &self.move_list[trick_usize][0].moves;
            let cur_rank = mply[k_usize].rank;
            let cur_sequence = mply[k_usize].sequence;
            let r_rank = i32::from(REL_RANK[aggr][cur_rank as usize]);

            if tpos.winner[suit].rank == cur_rank
                || tpos.rank_in_suit[partner_i][suit]
                    > (tpos.rank_in_suit[lho_i][suit] | tpos.rank_in_suit[rho_i][suit])
            {
                // Can win trick.
                if tpos.second_best[suit].hand == rho_i as i32 {
                    if suit_count_rh != 1 {
                        suit_weight_delta += -1;
                    }
                } else if tpos.second_best[suit].hand == lho_i as i32 {
                    if suit_count_lh == 1 {
                        suit_weight_delta += 16;
                    } else {
                        suit_weight_delta += 22;
                    }
                }

                let mply = &mut self.move_list[trick_usize][0].moves;
                let cond_l = tpos.second_best[suit].hand != lho_i as i32 || suit_count_lh == 1;
                let cond_r = tpos.second_best[suit].hand != rho_i as i32 || suit_count_rh == 1;
                if cond_l && cond_r {
                    mply[k_usize].weight = suit_weight_delta + 45 + r_rank;
                } else {
                    mply[k_usize].weight = suit_weight_delta + 18 + r_rank;
                }

                if best_move.suit == suit as i32 && best_move.rank == cur_rank {
                    mply[k_usize].weight += 126;
                } else if best_move_tt.suit == suit as i32 && best_move_tt.rank == cur_rank {
                    mply[k_usize].weight += 32;
                }
            } else {
                if tpos.winner[suit].hand == rho_i as i32
                    || tpos.second_best[suit].hand == rho_i as i32
                {
                    if suit_count_rh != 1 {
                        suit_weight_delta += -10;
                    }
                } else if tpos.winner[suit].hand == lho_i as i32
                    && tpos.second_best[suit].hand == partner_i as i32
                    && tpos.length[partner_i][suit] != 1
                {
                    suit_weight_delta += 31;
                }

                let third_best_hand =
                    Self::third_best_hand_or_zero(thrp_rel, aggr, suit, partner_i, lead);

                if tpos.second_best[suit].hand == partner_i as i32
                    && partner_i as i32 == third_best_hand
                {
                    suit_weight_delta += 35;
                } else if (tpos.second_best[suit].hand == lead as i32
                    && partner_i as i32 == third_best_hand
                    && tpos.length[partner_i][suit] > 1)
                    || (tpos.second_best[suit].hand == partner_i as i32
                        && lead as i32 == third_best_hand
                        && tpos.length[partner_i][suit] > 1)
                {
                    suit_weight_delta += 25;
                }

                let mply = &mut self.move_list[trick_usize][0].moves;
                if (suit_count_lh == 1 && tpos.winner[suit].hand == lho_i as i32)
                    || (suit_count_rh == 1 && tpos.winner[suit].hand == rho_i as i32)
                {
                    mply[k_usize].weight = suit_weight_delta + 28 + r_rank;
                } else if tpos.winner[suit].hand == lead as i32 {
                    mply[k_usize].weight = suit_weight_delta - 17 + r_rank;
                } else if cur_sequence == 0 {
                    mply[k_usize].weight = suit_weight_delta + 12 + r_rank;
                } else if cur_rank == tpos.second_best[suit].rank {
                    mply[k_usize].weight = suit_weight_delta + 48;
                } else {
                    mply[k_usize].weight = suit_weight_delta + 29 - r_rank;
                }

                if best_move.suit == suit as i32 && best_move.rank == cur_rank {
                    mply[k_usize].weight += 47;
                } else if best_move_tt.suit == suit as i32 && best_move_tt.rank == cur_rank {
                    mply[k_usize].weight += 19;
                }
            }
        }
    }

    // -----------------------------------------------------------------
    // WeightAlloc — hand 1 (second to play)
    // -----------------------------------------------------------------

    fn weight_alloc_trump_notvoid1(&mut self, tpos: &Pos) {
        let lead = self.lead_hand as usize;
        let rho_i = RHO[lead];
        let partner_i = PARTNER[lead];
        let trump = self.trump;
        let lead_suit = self.lead_suit as usize;
        let trick_usize = self.curr_trick as usize;

        let max3rd = i32::from(HIGHEST_RANK[tpos.rank_in_suit[partner_i][lead_suit] as usize]);
        let maxpd = i32::from(HIGHEST_RANK[tpos.rank_in_suit[rho_i][lead_suit] as usize]);
        let min3rd = i32::from(LOWEST_RANK[tpos.rank_in_suit[partner_i][lead_suit] as usize]);
        let minpd = i32::from(LOWEST_RANK[tpos.rank_in_suit[rho_i][lead_suit] as usize]);

        let move0_rank = self.track[self.track_index].move_played[0].rank;
        let aggr_lead = tpos.aggr[lead_suit] as usize;
        let hand_rel_usize = 1; // hand 1
        let _ = hand_rel_usize;

        let n = self.num_moves as usize;
        for mv in &mut self.move_list[trick_usize][1].moves[..n] {
            let cur_rank = mv.rank;
            let cur_sequence = mv.sequence;
            let r_rank = i32::from(REL_RANK[aggr_lead][cur_rank as usize]);
            let mut win_move = false;

            if self.lead_suit == trump {
                if (maxpd > move0_rank && maxpd > max3rd)
                    || (cur_rank > move0_rank && cur_rank > max3rd)
                {
                    win_move = true;
                }
            } else if cur_rank > move0_rank && cur_rank > max3rd {
                if max3rd != 0
                    || tpos.length[partner_i][trump as usize] == 0
                    || (maxpd == 0
                        && tpos.length[rho_i][trump as usize] != 0
                        && tpos.rank_in_suit[rho_i][trump as usize]
                            > tpos.rank_in_suit[partner_i][trump as usize])
                {
                    win_move = true;
                }
            } else if maxpd > move0_rank && maxpd > max3rd {
                if max3rd != 0 || tpos.length[partner_i][trump as usize] == 0 {
                    win_move = true;
                }
            } else if move0_rank > maxpd && move0_rank > max3rd && move0_rank > cur_rank {
                if maxpd == 0
                    && tpos.length[rho_i][trump as usize] != 0
                    && (max3rd != 0
                        || tpos.length[partner_i][trump as usize] == 0
                        || tpos.rank_in_suit[rho_i][trump as usize]
                            > tpos.rank_in_suit[partner_i][trump as usize])
                {
                    win_move = true;
                }
            } else if maxpd == 0 && tpos.length[rho_i][trump as usize] != 0 {
                win_move = true;
            }

            if win_move {
                if min3rd > cur_rank {
                    mv.weight = 40 + r_rank;
                } else if maxpd > move0_rank
                    && tpos.rank_in_suit[lead][lead_suit] > tpos.rank_in_suit[rho_i][lead_suit]
                {
                    mv.weight = 41 + r_rank;
                } else if cur_rank > move0_rank {
                    if cur_rank < maxpd {
                        mv.weight = 78 - cur_rank;
                    } else if cur_rank > max3rd {
                        mv.weight = 73 - cur_rank;
                    } else if cur_sequence != 0 {
                        mv.weight = 62 - cur_rank;
                    } else {
                        mv.weight = 49 - cur_rank;
                    }
                } else if maxpd > 0 {
                    mv.weight = 47 - cur_rank;
                } else {
                    mv.weight = 40 - cur_rank;
                }
            } else if cur_rank < min3rd || cur_rank < minpd {
                mv.weight = -9 + r_rank;
            } else if cur_rank < move0_rank {
                mv.weight = -16 + r_rank;
            } else if cur_sequence != 0 {
                mv.weight = 22 - cur_rank;
            } else {
                mv.weight = 10 - cur_rank;
            }
        }
    }

    fn weight_alloc_nt_notvoid1(&mut self, tpos: &Pos) {
        let lead = self.lead_hand as usize;
        let rho_i = RHO[lead];
        let partner_i = PARTNER[lead];
        let lead_suit = self.lead_suit as usize;
        let trick_usize = self.curr_trick as usize;

        let max3rd = i32::from(HIGHEST_RANK[tpos.rank_in_suit[partner_i][lead_suit] as usize]);
        let maxpd = i32::from(HIGHEST_RANK[tpos.rank_in_suit[rho_i][lead_suit] as usize]);
        let move0_rank = self.track[self.track_index].move_played[0].rank;

        let n = self.num_moves as usize;
        if maxpd > move0_rank && maxpd > max3rd {
            let mply = &mut self.move_list[trick_usize][1].moves;
            for mv in &mut mply[..n] {
                mv.weight = -mv.rank;
            }
        } else {
            let min3rd = i32::from(LOWEST_RANK[tpos.rank_in_suit[partner_i][lead_suit] as usize]);
            let minpd = i32::from(LOWEST_RANK[tpos.rank_in_suit[rho_i][lead_suit] as usize]);
            let aggr_lead = tpos.aggr[lead_suit] as usize;

            let mply = &mut self.move_list[trick_usize][1].moves;
            for mv in &mut mply[..n] {
                let cur_rank = mv.rank;
                let cur_sequence = mv.sequence;
                let r_rank = i32::from(REL_RANK[aggr_lead][cur_rank as usize]);
                if cur_rank > move0_rank && cur_rank > max3rd {
                    mv.weight = 81 - cur_rank;
                } else if min3rd > cur_rank || minpd > cur_rank {
                    mv.weight = -3 + r_rank;
                } else if cur_rank < move0_rank {
                    mv.weight = -11 + r_rank;
                } else if cur_sequence != 0 {
                    mv.weight = 10 + r_rank;
                } else {
                    mv.weight = 13 - cur_rank;
                }
            }
        }
    }

    fn weight_alloc_trump_void1(&mut self, tpos: &Pos) {
        let lead = self.lead_hand as usize;
        let curr_hand = self.curr_hand as usize;
        let rho_i = RHO[lead];
        let partner_i = PARTNER[lead];
        let trump = self.trump as usize;
        let lead_suit = self.lead_suit as usize;
        let suit = self.suit as usize;
        let trick_usize = self.curr_trick as usize;
        let hand_rel = 1usize;

        let suit_count = i32::from(tpos.length[curr_hand][suit]);
        let suit_count_lo = suit_count << 6;
        let move0_rank = self.track[self.track_index].move_played[0].rank;

        let lo = self.last_num_moves as usize;
        let n = self.num_moves as usize;
        if self.lead_suit == trump as i32 {
            // We pitch.
            let suit_add = if tpos.rank_in_suit[rho_i][lead_suit]
                > (tpos.rank_in_suit[partner_i][lead_suit] | BIT_MAP_RANK[move0_rank as usize])
            {
                suit_count_lo / 44
            } else {
                let mut add = suit_count_lo / 36;
                if suit_count == 2 && tpos.second_best[suit].hand == curr_hand as i32 {
                    add += -4;
                }
                add
            };
            let mply = &mut self.move_list[trick_usize][hand_rel].moves;
            for mv in &mut mply[lo..n] {
                mv.weight = -mv.rank + suit_add;
            }
        } else if suit != trump {
            // We discard on a side suit.
            let suit_add = if tpos.length[partner_i][lead_suit] != 0 {
                if tpos.rank_in_suit[rho_i][lead_suit]
                    > (tpos.rank_in_suit[partner_i][lead_suit] | BIT_MAP_RANK[move0_rank as usize])
                    || (tpos.length[rho_i][lead_suit] == 0 && tpos.length[rho_i][trump] != 0)
                {
                    60 + suit_count_lo / 44
                } else {
                    let mut add = -2 + suit_count_lo / 36;
                    if suit_count == 2 && tpos.second_best[suit].hand == curr_hand as i32 {
                        add += -4;
                    }
                    add
                }
            } else if (tpos.length[rho_i][lead_suit] == 0
                && tpos.rank_in_suit[rho_i][trump] > tpos.rank_in_suit[partner_i][trump])
                || (tpos.length[partner_i][trump] == 0
                    && tpos.rank_in_suit[rho_i][lead_suit] > BIT_MAP_RANK[move0_rank as usize])
            {
                60 + suit_count_lo / 44
            } else {
                let mut add = -2 + suit_count_lo / 36;
                if suit_count == 2 && tpos.second_best[suit].hand == curr_hand as i32 {
                    add += -4;
                }
                add
            };
            let mply = &mut self.move_list[trick_usize][hand_rel].moves;
            for mv in &mut mply[lo..n] {
                mv.weight = -mv.rank + suit_add;
            }
        } else if tpos.length[partner_i][lead_suit] != 0 {
            // 3rd hand follows suit while we ruff.
            let suit_add = suit_count_lo / 44;
            let mply = &mut self.move_list[trick_usize][hand_rel].moves;
            for mv in &mut mply[lo..n] {
                mv.weight = 24 - mv.rank + suit_add;
            }
        } else if tpos.length[rho_i][lead_suit] == 0
            && tpos.length[rho_i][trump] != 0
            && tpos.rank_in_suit[rho_i][trump] > tpos.rank_in_suit[partner_i][trump]
        {
            let suit_add = suit_count_lo / 44;
            let mply = &mut self.move_list[trick_usize][hand_rel].moves;
            for mv in &mut mply[lo..n] {
                mv.weight = 24 - mv.rank + suit_add;
            }
        } else {
            let mply = &mut self.move_list[trick_usize][hand_rel].moves;
            for mv in &mut mply[lo..n] {
                let cur_rank = mv.rank;
                if BIT_MAP_RANK[cur_rank as usize] > tpos.rank_in_suit[partner_i][trump] {
                    let suit_add = suit_count_lo / 44;
                    mv.weight = 24 - cur_rank + suit_add;
                } else {
                    let mut suit_add = suit_count_lo / 36;
                    if suit_count == 2 && tpos.second_best[suit].hand == curr_hand as i32 {
                        suit_add += -4;
                    }
                    mv.weight = 15 - cur_rank + suit_add;
                }
            }
        }
    }

    #[allow(clippy::branches_sharing_code)]
    fn weight_alloc_nt_void1(&mut self, tpos: &Pos) {
        let lead = self.lead_hand as usize;
        let curr_hand = self.curr_hand as usize;
        let rho_i = RHO[lead];
        let partner_i = PARTNER[lead];
        let lead_suit = self.lead_suit as usize;
        let suit = self.suit as usize;
        let trick_usize = self.curr_trick as usize;
        let hand_rel = 1usize;

        let move0_rank = self.track[self.track_index].move_played[0].rank;

        if tpos.rank_in_suit[rho_i][lead_suit]
            > (tpos.rank_in_suit[partner_i][lead_suit] | BIT_MAP_RANK[move0_rank as usize])
        {
            // Partner can win.
            let suit_count = i32::from(tpos.length[curr_hand][suit]);
            let mut suit_add = (suit_count << 6) / 23;
            if suit_count == 2 && tpos.second_best[suit].hand == curr_hand as i32 {
                suit_add += -2;
            } else if suit_count == 1 && tpos.winner[suit].hand == curr_hand as i32 {
                suit_add += -3;
            }
            let lo = self.last_num_moves as usize;
            let n = self.num_moves as usize;
            let mply = &mut self.move_list[trick_usize][hand_rel].moves;
            for mv in &mut mply[lo..n] {
                mv.weight = -mv.rank + suit_add;
            }
        } else {
            let suit_count = i32::from(tpos.length[curr_hand][suit]);
            let mut suit_add = (suit_count << 6) / 33;
            if suit_count == 2 && tpos.second_best[suit].hand == curr_hand as i32 {
                suit_add += -6;
            } else if suit_count == 1 && tpos.winner[suit].hand == curr_hand as i32 {
                suit_add += -8;
            }
            let lo = self.last_num_moves as usize;
            let n = self.num_moves as usize;
            let mply = &mut self.move_list[trick_usize][hand_rel].moves;
            for mv in &mut mply[lo..n] {
                mv.weight = -mv.rank + suit_add;
            }
        }
    }

    // -----------------------------------------------------------------
    // WeightAlloc — hand 2 (third to play)
    // -----------------------------------------------------------------

    fn weight_alloc_trump_notvoid2(&mut self, tpos: &Pos) {
        let lead = self.lead_hand as usize;
        let rho_i = RHO[lead];
        let lead_suit = self.lead_suit as usize;
        let trump = self.trump;
        let trick_usize = self.curr_trick as usize;
        let hand_rel = 2usize;

        let cards4th = tpos.rank_in_suit[rho_i][lead_suit];
        let max4th = i32::from(HIGHEST_RANK[cards4th as usize]);
        let min4th = i32::from(LOWEST_RANK[cards4th as usize]);
        let max3rd = self.move_list[trick_usize][hand_rel].moves[0].rank;
        let track = self.track[self.track_index];

        let n = self.num_moves as usize;
        if self.lead_suit == trump {
            if (track.high[1] == 0 && track.move_played[0].rank > max4th)
                || max3rd < min4th
                || max3rd < track.move_played[1].rank
            {
                let mply = &mut self.move_list[trick_usize][hand_rel].moves;
                for mv in &mut mply[..n] {
                    mv.weight = -mv.rank;
                }
            } else if max3rd > max4th {
                let mply = &mut self.move_list[trick_usize][hand_rel].moves;
                let move1_rank = track.move_played[1].rank;
                for mv in &mut mply[..n] {
                    if mv.rank > max4th && mv.rank > move1_rank {
                        mv.weight = 58 - mv.rank;
                    } else {
                        mv.weight = -mv.rank;
                    }
                }
            } else {
                let k_bonus = self.rank_forces_ace(i32::from(cards4th));
                let mply = &mut self.move_list[trick_usize][hand_rel].moves;
                for mv in &mut mply[..n] {
                    mv.weight = -mv.rank;
                }
                if k_bonus != -1 {
                    mply[k_bonus as usize].weight += 20;
                }
            }
        } else if track.move_played[1].suit == trump {
            // 2nd hand ruffs, and we must follow suit.
            let mply = &mut self.move_list[trick_usize][hand_rel].moves;
            for mv in &mut mply[..n] {
                mv.weight = -mv.rank;
            }
        } else if track.high[1] == 0 {
            // Partner is winning so far.
            if max4th == 0
                || track.move_played[0].rank > max4th
                || max3rd < min4th
                || max3rd < track.move_played[1].rank
            {
                let mply = &mut self.move_list[trick_usize][hand_rel].moves;
                for mv in &mut mply[..n] {
                    mv.weight = -mv.rank;
                }
            } else if max3rd > max4th {
                let mply = &mut self.move_list[trick_usize][hand_rel].moves;
                for mv in &mut mply[..n] {
                    if mv.rank > max4th {
                        mv.weight = 58 - mv.rank;
                    } else {
                        mv.weight = -mv.rank;
                    }
                }
            } else {
                let k_bonus = self.rank_forces_ace(i32::from(cards4th));
                let mply = &mut self.move_list[trick_usize][hand_rel].moves;
                let move1_rank = track.move_played[1].rank;
                for mv in &mut mply[..n] {
                    if mv.rank > move1_rank && mv.rank > max4th {
                        mv.weight = 60 - mv.rank;
                    } else {
                        mv.weight = -mv.rank;
                    }
                }
                if k_bonus != -1 {
                    mply[k_bonus as usize].weight += 20;
                }
            }
        } else {
            // 2nd hand is winning so far.
            if max4th == 0 {
                let mply = &mut self.move_list[trick_usize][hand_rel].moves;
                let move1_rank = track.move_played[1].rank;
                for mv in &mut mply[..n] {
                    if mv.rank > move1_rank {
                        mv.weight = 20 - mv.rank;
                    } else {
                        mv.weight = -mv.rank;
                    }
                }
                return;
            } else if max3rd < min4th || max3rd < track.move_played[1].rank {
                let mply = &mut self.move_list[trick_usize][hand_rel].moves;
                for mv in &mut mply[..n] {
                    mv.weight = -mv.rank;
                }
                return;
            } else if max3rd > max4th {
                let mply = &mut self.move_list[trick_usize][hand_rel].moves;
                let move1_rank = track.move_played[1].rank;
                for mv in &mut mply[..n] {
                    if mv.rank > move1_rank && mv.rank > max4th {
                        mv.weight = 58 - mv.rank;
                    } else {
                        mv.weight = -mv.rank;
                    }
                }
                return;
            }
            let k_bonus = self.rank_forces_ace(i32::from(cards4th));
            let mply = &mut self.move_list[trick_usize][hand_rel].moves;
            let move1_rank = track.move_played[1].rank;
            for mv in &mut mply[..n] {
                if mv.rank > move1_rank && mv.rank > max4th {
                    mv.weight = 60 - mv.rank;
                } else {
                    mv.weight = -mv.rank;
                }
            }
            if k_bonus != -1 {
                mply[k_bonus as usize].weight += 20;
            }
        }
    }

    /// Vendor's `RankForcesAce`. Returns the move index (within the
    /// current `numMoves`) to bonus, or -1 if none.
    fn rank_forces_ace(&self, cards4th: i32) -> i32 {
        let mp = GROUP_DATA[cards4th as usize];
        let mut g = i32::from(mp.last_group);
        let removed = self.track[self.track_index].removed_ranks[self.lead_suit as usize];

        while g >= 1 && (i32::from(mp.gap[g as usize]) & removed) == i32::from(mp.gap[g as usize]) {
            g -= 1;
        }
        if g == 0 {
            // The vendor checks `if (!g)` here, which is true for g == 0.
            // Returning -1 means "no bonus".
            return -1;
        }

        let second_rho = if g == 0 {
            0
        } else {
            i32::from(mp.rank[(g - 1) as usize])
        };
        let hand_rel = 2usize;
        let trick_usize = self.curr_trick as usize;
        let mply = &self.move_list[trick_usize][hand_rel].moves;
        let track = &self.track[self.track_index];

        if second_rho > track.move_played[1].rank {
            let mut k = 0;
            while k < self.num_moves && mply[k as usize].rank > second_rho {
                k += 1;
            }
            if k != 0 {
                return k - 1;
            }
        } else if track.high[1] == 1 {
            let mut k = 0;
            while k < self.num_moves && mply[k as usize].rank > track.move_played[1].rank {
                k += 1;
            }
            if k != 0 {
                return k - 1;
            }
        }
        -1
    }

    /// Vendor's `GetTopNumber`. Sets `top_number` and `mno`.
    fn get_top_number(&self, ris: i32, prank: i32) -> (i32, i32) {
        let mut mno = 0i32;
        let trick_usize = self.curr_trick as usize;
        let hand_rel = 2usize;
        let mply = &self.move_list[trick_usize][hand_rel].moves;
        while mno < self.num_moves - 1 && mply[(1 + mno) as usize].rank > prank {
            mno += 1;
        }

        let mp = GROUP_DATA[ris as usize];
        let mut g = i32::from(mp.last_group);
        let removed = self.track[self.track_index].removed_ranks[self.lead_suit as usize]
            | i32::from(BIT_MAP_RANK[prank as usize]);

        let mut fullseq = i32::from(mp.fullseq[g as usize]);
        while g >= 1 && (i32::from(mp.gap[g as usize]) & removed) == i32::from(mp.gap[g as usize]) {
            g -= 1;
            fullseq |= i32::from(mp.fullseq[g as usize]);
        }
        let top_number = i32::from(COUNT_TABLE[fullseq as usize]) - 1;
        (top_number, mno)
    }

    fn weight_alloc_nt_notvoid2(&mut self, tpos: &Pos) {
        let lead = self.lead_hand as usize;
        let curr_hand = self.curr_hand as usize;
        let lho_i = LHO[lead];
        let rho_i = RHO[lead];
        let partner_i = PARTNER[lead];
        let lead_suit = self.lead_suit as usize;
        let trick_usize = self.curr_trick as usize;
        let hand_rel = 2usize;

        let cards4th = tpos.rank_in_suit[rho_i][lead_suit];
        let max4th = i32::from(HIGHEST_RANK[cards4th as usize]);
        let min4th = i32::from(LOWEST_RANK[cards4th as usize]);
        let max3rd = self.move_list[trick_usize][hand_rel].moves[0].rank;
        let track = self.track[self.track_index];

        let n = self.num_moves as usize;
        if track.high[1] == 0 && track.move_played[0].rank > max4th {
            let mply = &mut self.move_list[trick_usize][hand_rel].moves;
            for mv in &mut mply[..n] {
                mv.weight = -mv.rank;
            }
            if tpos.length[lead][lead_suit] == 0 && tpos.winner[lead_suit].hand == curr_hand as i32
            {
                let mut opp_len = i32::from(tpos.length[rho_i][lead_suit]) - 1;
                let lho_len = i32::from(tpos.length[lho_i][lead_suit]);
                if lho_len > opp_len {
                    opp_len = lho_len;
                }
                let (top_number, mno) = self.get_top_number(
                    i32::from(tpos.rank_in_suit[partner_i][lead_suit]),
                    track.move_played[0].rank,
                );
                if opp_len <= top_number {
                    let mply = &mut self.move_list[trick_usize][hand_rel].moves;
                    mply[mno as usize].weight += 20;
                }
            }
            return;
        } else if max3rd < min4th || max3rd < track.move_played[1].rank {
            let mply = &mut self.move_list[trick_usize][hand_rel].moves;
            for mv in &mut mply[..n] {
                mv.weight = -mv.rank;
            }
            return;
        }

        let k_bonus = if max4th > max3rd && max4th > track.move_played[1].rank {
            self.rank_forces_ace(i32::from(cards4th))
        } else {
            -1
        };
        let mply = &mut self.move_list[trick_usize][hand_rel].moves;
        let move1_rank = track.move_played[1].rank;
        for mv in &mut mply[..n] {
            if mv.rank > move1_rank && mv.rank > max4th {
                mv.weight = 60 - mv.rank;
            } else {
                mv.weight = -mv.rank;
            }
        }
        if k_bonus != -1 {
            mply[k_bonus as usize].weight += 20;
        }
    }

    #[allow(clippy::branches_sharing_code)]
    fn weight_alloc_trump_void2(&mut self, tpos: &Pos) {
        let lead = self.lead_hand as usize;
        let curr_hand = self.curr_hand as usize;
        let rho_i = RHO[lead];
        let trump = self.trump as usize;
        let lead_suit = self.lead_suit as usize;
        let suit = self.suit as usize;
        let trick_usize = self.curr_trick as usize;
        let hand_rel = 2usize;

        let suit_count = i32::from(tpos.length[curr_hand][suit]);
        let max4th = i32::from(HIGHEST_RANK[tpos.rank_in_suit[rho_i][lead_suit] as usize]);
        let track = self.track[self.track_index];

        let lo = self.last_num_moves as usize;
        let n = self.num_moves as usize;
        if self.lead_suit == trump as i32 || suit != trump {
            // Discard small from a long suit.
            let suit_add = (suit_count << 6) / 40;
            let mply = &mut self.move_list[trick_usize][hand_rel].moves;
            for mv in &mut mply[lo..n] {
                mv.weight = -mv.rank + suit_add;
            }
            return;
        } else if track.high[1] == 0
            && track.move_played[0].rank > max4th
            && (max4th != 0 || tpos.length[rho_i][trump] == 0)
        {
            // Partner already beat 2nd and 4th hands.
            let mply = &mut self.move_list[trick_usize][hand_rel].moves;
            for mv in &mut mply[lo..n] {
                mv.weight = -mv.rank - 50;
            }
            return;
        }

        let mply = &mut self.move_list[trick_usize][hand_rel].moves;
        for mv in &mut mply[lo..n] {
            let cur_rank = mv.rank;
            if track.move_played[1].suit == trump as i32 && cur_rank < track.move_played[1].rank {
                let r_rank = i32::from(REL_RANK[tpos.aggr[suit] as usize][cur_rank as usize]);
                let suit_add = (suit_count << 6) / 40;
                mv.weight = -32 + r_rank + suit_add;
            } else if track.high[1] == 0 {
                if max4th != 0 {
                    if tpos.second_best[lead_suit].hand == lead as i32 {
                        let suit_add = (suit_count << 6) / 50;
                        mv.weight = 36 - cur_rank + suit_add;
                    } else {
                        let suit_add = (suit_count << 6) / 50;
                        mv.weight = 48 - cur_rank + suit_add;
                    }
                } else if BIT_MAP_RANK[cur_rank as usize] > tpos.rank_in_suit[rho_i][trump] {
                    let suit_add = (suit_count << 6) / 50;
                    mv.weight = 48 - cur_rank + suit_add;
                } else {
                    let suit_add = (suit_count << 6) / 50;
                    mv.weight = -12 - cur_rank + suit_add;
                }
            } else if max4th != 0 {
                let suit_add = (suit_count << 6) / 50;
                mv.weight = 72 - cur_rank + suit_add;
            } else if BIT_MAP_RANK[cur_rank as usize] > tpos.rank_in_suit[rho_i][trump] {
                let suit_add = (suit_count << 6) / 50;
                mv.weight = 48 - cur_rank + suit_add;
            } else {
                let suit_add = (suit_count << 6) / 50;
                mv.weight = 36 - cur_rank + suit_add;
            }
        }
    }

    fn weight_alloc_nt_void2(&mut self, tpos: &Pos) {
        let curr_hand = self.curr_hand as usize;
        let suit = self.suit as usize;
        let trick_usize = self.curr_trick as usize;
        let hand_rel = 2usize;

        let suit_count = i32::from(tpos.length[curr_hand][suit]);
        let mut suit_add = (suit_count << 6) / 24;
        if (suit_count == 2 && tpos.second_best[suit].hand == curr_hand as i32)
            || (suit_count == 1 && tpos.winner[suit].hand == curr_hand as i32)
        {
            suit_add -= 4;
        }

        let lo = self.last_num_moves as usize;
        let n = self.num_moves as usize;
        let mply = &mut self.move_list[trick_usize][hand_rel].moves;
        for mv in &mut mply[lo..n] {
            mv.weight = -mv.rank + suit_add;
        }
    }

    // -----------------------------------------------------------------
    // WeightAlloc — hand 3 (fourth to play)
    // -----------------------------------------------------------------

    #[allow(clippy::branches_sharing_code)]
    fn weight_alloc_combined_notvoid3(&mut self, _tpos: &Pos) {
        let trump = self.trump;
        let lead_suit = self.lead_suit;
        let trick_usize = self.curr_trick as usize;
        let hand_rel = 3usize;
        let track = self.track[self.track_index];

        let n = self.num_moves as usize;
        if track.high[2] == 1 || (lead_suit != trump && track.move_played[2].suit == trump) {
            let mply = &mut self.move_list[trick_usize][hand_rel].moves;
            for mv in &mut mply[..n] {
                mv.weight = -mv.rank;
            }
        } else {
            let mply = &mut self.move_list[trick_usize][hand_rel].moves;
            let move2_rank = track.move_played[2].rank;
            for mv in &mut mply[..n] {
                if mv.rank > move2_rank {
                    mv.weight = 30 - mv.rank;
                } else {
                    mv.weight = -mv.rank;
                }
            }
        }
    }

    #[allow(clippy::branches_sharing_code)]
    fn weight_alloc_trump_void3(&mut self, tpos: &Pos) {
        let curr_hand = self.curr_hand as usize;
        let suit = self.suit as usize;
        let trump = self.trump;
        let lead_suit = self.lead_suit;
        let trick_usize = self.curr_trick as usize;
        let hand_rel = 3usize;
        let track = self.track[self.track_index];

        let mylen = i32::from(tpos.length[curr_hand][suit]);
        let mut val = (mylen << 6) / 24;
        if mylen == 2 && tpos.second_best[suit].hand == curr_hand as i32 {
            val -= 2;
        }

        let lo = self.last_num_moves as usize;
        let n = self.num_moves as usize;
        if lead_suit == trump {
            let mply = &mut self.move_list[trick_usize][hand_rel].moves;
            for mv in &mut mply[lo..n] {
                mv.weight = -mv.rank + val;
            }
        } else if track.high[2] == 1 {
            if suit == trump as usize {
                let mply = &mut self.move_list[trick_usize][hand_rel].moves;
                for mv in &mut mply[lo..n] {
                    mv.weight = 2 - mv.rank + val;
                }
            } else {
                let mply = &mut self.move_list[trick_usize][hand_rel].moves;
                for mv in &mut mply[lo..n] {
                    mv.weight = 25 - mv.rank + val;
                }
            }
        } else if track.move_played[2].suit == trump {
            if suit == trump as usize {
                let mply = &mut self.move_list[trick_usize][hand_rel].moves;
                let move2_rank = track.move_played[2].rank;
                for mv in &mut mply[lo..n] {
                    let r_rank = i32::from(REL_RANK[tpos.aggr[suit] as usize][mv.rank as usize]);
                    if mv.rank > move2_rank {
                        mv.weight = 33 + r_rank;
                    } else {
                        mv.weight = -13 + r_rank;
                    }
                }
            } else {
                let mply = &mut self.move_list[trick_usize][hand_rel].moves;
                for mv in &mut mply[lo..n] {
                    mv.weight = 14 - mv.rank + val;
                }
            }
        } else if suit == trump as usize {
            let mply = &mut self.move_list[trick_usize][hand_rel].moves;
            for mv in &mut mply[lo..n] {
                let r_rank = i32::from(REL_RANK[tpos.aggr[suit] as usize][mv.rank as usize]);
                mv.weight = 33 + r_rank;
            }
        } else {
            let mply = &mut self.move_list[trick_usize][hand_rel].moves;
            for mv in &mut mply[lo..n] {
                mv.weight = 14 - mv.rank + val;
            }
        }
    }

    fn weight_alloc_nt_void3(&mut self, tpos: &Pos) {
        let curr_hand = self.curr_hand as usize;
        let suit = self.suit as usize;
        let trick_usize = self.curr_trick as usize;
        let hand_rel = 3usize;

        let mylen = i32::from(tpos.length[curr_hand][suit]);
        let mut val = (mylen << 6) / 27;
        if mylen == 2 && tpos.second_best[suit].hand == curr_hand as i32 {
            val -= 6;
        } else if mylen == 1 && tpos.winner[suit].hand == curr_hand as i32 {
            val -= 8;
        }

        let lo = self.last_num_moves as usize;
        let n = self.num_moves as usize;
        let mply = &mut self.move_list[trick_usize][hand_rel].moves;
        for mv in &mut mply[lo..n] {
            mv.weight = -mv.rank + val;
        }
    }
}

impl Default for Moves {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------
// MergeSort — fixed-size sorting networks for 2..=12 elements, with
// insertion-sort fallback for larger lists. Direct port of the vendor's
// `MergeSort` switch statement.
// ---------------------------------------------------------------------

/// Compare-and-swap `moves[i]` against `moves[j]` so the higher-weight
/// move ends up at the lower index.
#[allow(clippy::inline_always)]
#[inline(always)]
fn cmp_swap(moves: &mut [MoveType], i: usize, j: usize) {
    if moves[i].weight < moves[j].weight {
        moves.swap(i, j);
    }
}

/// Sort the first `n` entries of `moves` in descending weight order.
fn merge_sort(moves: &mut [MoveType], n: usize) {
    match n {
        0 | 1 => {}
        2 => {
            cmp_swap(moves, 0, 1);
        }
        3 => {
            cmp_swap(moves, 0, 1);
            cmp_swap(moves, 0, 2);
            cmp_swap(moves, 1, 2);
        }
        4 => {
            cmp_swap(moves, 0, 1);
            cmp_swap(moves, 2, 3);
            cmp_swap(moves, 0, 2);
            cmp_swap(moves, 1, 3);
            cmp_swap(moves, 1, 2);
        }
        5 => {
            cmp_swap(moves, 0, 1);
            cmp_swap(moves, 2, 3);
            cmp_swap(moves, 0, 2);
            cmp_swap(moves, 1, 3);
            cmp_swap(moves, 1, 2);
            cmp_swap(moves, 0, 4);
            cmp_swap(moves, 2, 4);
            cmp_swap(moves, 1, 2);
            cmp_swap(moves, 3, 4);
        }
        6 => {
            cmp_swap(moves, 0, 1);
            cmp_swap(moves, 2, 3);
            cmp_swap(moves, 4, 5);
            cmp_swap(moves, 0, 2);
            cmp_swap(moves, 1, 3);
            cmp_swap(moves, 1, 2);
            cmp_swap(moves, 0, 4);
            cmp_swap(moves, 1, 5);
            cmp_swap(moves, 2, 4);
            cmp_swap(moves, 3, 5);
            cmp_swap(moves, 1, 2);
            cmp_swap(moves, 3, 4);
        }
        7 => {
            cmp_swap(moves, 0, 1);
            cmp_swap(moves, 2, 3);
            cmp_swap(moves, 4, 5);
            cmp_swap(moves, 0, 2);
            cmp_swap(moves, 4, 6);
            cmp_swap(moves, 1, 3);
            cmp_swap(moves, 1, 2);
            cmp_swap(moves, 5, 6);
            cmp_swap(moves, 0, 4);
            cmp_swap(moves, 1, 5);
            cmp_swap(moves, 2, 6);
            cmp_swap(moves, 2, 4);
            cmp_swap(moves, 3, 5);
            cmp_swap(moves, 1, 2);
            cmp_swap(moves, 3, 4);
            cmp_swap(moves, 5, 6);
        }
        8 => {
            cmp_swap(moves, 0, 1);
            cmp_swap(moves, 2, 3);
            cmp_swap(moves, 4, 5);
            cmp_swap(moves, 6, 7);
            cmp_swap(moves, 0, 2);
            cmp_swap(moves, 4, 6);
            cmp_swap(moves, 1, 3);
            cmp_swap(moves, 5, 7);
            cmp_swap(moves, 1, 2);
            cmp_swap(moves, 5, 6);
            cmp_swap(moves, 0, 4);
            cmp_swap(moves, 1, 5);
            cmp_swap(moves, 2, 6);
            cmp_swap(moves, 3, 7);
            cmp_swap(moves, 2, 4);
            cmp_swap(moves, 3, 5);
            cmp_swap(moves, 1, 2);
            cmp_swap(moves, 3, 4);
            cmp_swap(moves, 5, 6);
        }
        9 => {
            cmp_swap(moves, 0, 1);
            cmp_swap(moves, 3, 4);
            cmp_swap(moves, 6, 7);
            cmp_swap(moves, 1, 2);
            cmp_swap(moves, 4, 5);
            cmp_swap(moves, 7, 8);
            cmp_swap(moves, 0, 1);
            cmp_swap(moves, 3, 4);
            cmp_swap(moves, 6, 7);
            cmp_swap(moves, 0, 3);
            cmp_swap(moves, 3, 6);
            cmp_swap(moves, 0, 3);
            cmp_swap(moves, 1, 4);
            cmp_swap(moves, 4, 7);
            cmp_swap(moves, 1, 4);
            cmp_swap(moves, 2, 5);
            cmp_swap(moves, 5, 8);
            cmp_swap(moves, 2, 5);
            cmp_swap(moves, 1, 3);
            cmp_swap(moves, 5, 7);
            cmp_swap(moves, 2, 6);
            cmp_swap(moves, 4, 6);
            cmp_swap(moves, 2, 4);
            cmp_swap(moves, 2, 3);
            cmp_swap(moves, 5, 6);
        }
        10 => {
            cmp_swap(moves, 1, 8);
            cmp_swap(moves, 0, 4);
            cmp_swap(moves, 5, 9);
            cmp_swap(moves, 2, 6);
            cmp_swap(moves, 3, 7);
            cmp_swap(moves, 0, 3);
            cmp_swap(moves, 6, 9);
            cmp_swap(moves, 2, 5);
            cmp_swap(moves, 0, 1);
            cmp_swap(moves, 3, 6);
            cmp_swap(moves, 8, 9);
            cmp_swap(moves, 4, 7);
            cmp_swap(moves, 0, 2);
            cmp_swap(moves, 4, 8);
            cmp_swap(moves, 1, 5);
            cmp_swap(moves, 7, 9);
            cmp_swap(moves, 1, 2);
            cmp_swap(moves, 3, 4);
            cmp_swap(moves, 5, 6);
            cmp_swap(moves, 7, 8);
            cmp_swap(moves, 1, 3);
            cmp_swap(moves, 6, 8);
            cmp_swap(moves, 2, 4);
            cmp_swap(moves, 5, 7);
            cmp_swap(moves, 2, 3);
            cmp_swap(moves, 6, 7);
            cmp_swap(moves, 3, 5);
            cmp_swap(moves, 4, 6);
            cmp_swap(moves, 4, 5);
        }
        11 => {
            cmp_swap(moves, 0, 1);
            cmp_swap(moves, 2, 3);
            cmp_swap(moves, 4, 5);
            cmp_swap(moves, 6, 7);
            cmp_swap(moves, 8, 9);
            cmp_swap(moves, 1, 3);
            cmp_swap(moves, 5, 7);
            cmp_swap(moves, 0, 2);
            cmp_swap(moves, 4, 6);
            cmp_swap(moves, 8, 10);
            cmp_swap(moves, 1, 2);
            cmp_swap(moves, 5, 6);
            cmp_swap(moves, 9, 10);
            cmp_swap(moves, 1, 5);
            cmp_swap(moves, 6, 10);
            cmp_swap(moves, 5, 9);
            cmp_swap(moves, 2, 6);
            cmp_swap(moves, 1, 5);
            cmp_swap(moves, 6, 10);
            cmp_swap(moves, 0, 4);
            cmp_swap(moves, 3, 7);
            cmp_swap(moves, 4, 8);
            cmp_swap(moves, 0, 4);
            cmp_swap(moves, 1, 4);
            cmp_swap(moves, 7, 10);
            cmp_swap(moves, 3, 8);
            cmp_swap(moves, 2, 3);
            cmp_swap(moves, 8, 9);
            cmp_swap(moves, 2, 4);
            cmp_swap(moves, 7, 9);
            cmp_swap(moves, 3, 5);
            cmp_swap(moves, 6, 8);
            cmp_swap(moves, 3, 4);
            cmp_swap(moves, 5, 6);
            cmp_swap(moves, 7, 8);
        }
        12 => {
            cmp_swap(moves, 0, 1);
            cmp_swap(moves, 2, 3);
            cmp_swap(moves, 4, 5);
            cmp_swap(moves, 6, 7);
            cmp_swap(moves, 8, 9);
            cmp_swap(moves, 10, 11);
            cmp_swap(moves, 1, 3);
            cmp_swap(moves, 5, 7);
            cmp_swap(moves, 9, 11);
            cmp_swap(moves, 0, 2);
            cmp_swap(moves, 4, 6);
            cmp_swap(moves, 8, 10);
            cmp_swap(moves, 1, 2);
            cmp_swap(moves, 5, 6);
            cmp_swap(moves, 9, 10);
            cmp_swap(moves, 1, 5);
            cmp_swap(moves, 6, 10);
            cmp_swap(moves, 5, 9);
            cmp_swap(moves, 2, 6);
            cmp_swap(moves, 1, 5);
            cmp_swap(moves, 6, 10);
            cmp_swap(moves, 0, 4);
            cmp_swap(moves, 7, 11);
            cmp_swap(moves, 3, 7);
            cmp_swap(moves, 4, 8);
            cmp_swap(moves, 0, 4);
            cmp_swap(moves, 7, 11);
            cmp_swap(moves, 1, 4);
            cmp_swap(moves, 7, 10);
            cmp_swap(moves, 3, 8);
            cmp_swap(moves, 2, 3);
            cmp_swap(moves, 8, 9);
            cmp_swap(moves, 2, 4);
            cmp_swap(moves, 7, 9);
            cmp_swap(moves, 3, 5);
            cmp_swap(moves, 6, 8);
            cmp_swap(moves, 3, 4);
            cmp_swap(moves, 5, 6);
            cmp_swap(moves, 7, 8);
        }
        _ => {
            // Insertion sort fallback for n > 12.
            for i in 1..n {
                let tmp = moves[i];
                let mut j = i;
                while j > 0 && tmp.weight > moves[j - 1].weight {
                    moves[j] = moves[j - 1];
                    j -= 1;
                }
                moves[j] = tmp;
            }
        }
    }
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::move_type::HighCard;

    fn make_pos_singleton_in_each_suit() -> Pos {
        // North = AS, East = AH, South = AD, West = AC.
        let mut p = Pos::default();
        // North gets A in suit 0.
        p.rank_in_suit[0][0] = BIT_MAP_RANK[14];
        // East gets A in suit 1.
        p.rank_in_suit[1][1] = BIT_MAP_RANK[14];
        // South gets A in suit 2.
        p.rank_in_suit[2][2] = BIT_MAP_RANK[14];
        // West gets A in suit 3.
        p.rank_in_suit[3][3] = BIT_MAP_RANK[14];

        for s in 0..4 {
            p.aggr[s] = (0..4).fold(0u16, |a, h| a | p.rank_in_suit[h][s]);
        }
        for h in 0..4 {
            for s in 0..4 {
                p.length[h][s] = p.rank_in_suit[h][s].count_ones() as u8;
            }
        }
        for s in 0..4 {
            let hand = (0..4)
                .find(|&h| p.rank_in_suit[h][s] & BIT_MAP_RANK[14] != 0)
                .unwrap_or(0) as i32;
            p.winner[s] = HighCard { rank: 14, hand };
            p.second_best[s] = HighCard { rank: 0, hand: 0 };
        }
        p
    }

    fn make_pos_akqj_per_suit() -> Pos {
        // North: spades AKQJ; East: hearts AKQJ; South: diamonds AKQJ;
        // West: clubs AKQJ. Each hand has exactly four cards, all four
        // top honours in one suit.
        let mut p = Pos::default();
        let akqj: u16 = BIT_MAP_RANK[14] | BIT_MAP_RANK[13] | BIT_MAP_RANK[12] | BIT_MAP_RANK[11];
        p.rank_in_suit[0][0] = akqj; // N spades
        p.rank_in_suit[1][1] = akqj; // E hearts
        p.rank_in_suit[2][2] = akqj; // S diamonds
        p.rank_in_suit[3][3] = akqj; // W clubs
        for s in 0..4 {
            p.aggr[s] = (0..4).fold(0u16, |a, h| a | p.rank_in_suit[h][s]);
        }
        for h in 0..4 {
            for s in 0..4 {
                p.length[h][s] = p.rank_in_suit[h][s].count_ones() as u8;
            }
        }
        for s in 0..4 {
            let hand = (0..4).find(|&h| p.rank_in_suit[h][s] != 0).unwrap_or(0) as i32;
            p.winner[s] = HighCard { rank: 14, hand };
            p.second_best[s] = HighCard { rank: 13, hand };
        }
        p
    }

    #[test]
    fn new_initializes_clean_state() {
        let m = Moves::new();
        assert_eq!(m.trump(), DDS_NOTRUMP);
        // All move lists are default (last == -1).
        for t in 0..MAX_DEPTH {
            for h in 0..DDS_HANDS {
                assert_eq!(m.move_list[t][h].last, -1);
                assert_eq!(m.move_list[t][h].current, 0);
            }
        }
        for t in 0..MAX_DEPTH {
            assert_eq!(m.track[t].lead_hand, 0);
            assert_eq!(m.track[t].removed_ranks, [0; 4]);
        }
    }

    #[test]
    fn reinit_sets_lead_hand() {
        let mut m = Moves::new();
        m.reinit(7, 2);
        assert_eq!(m.track[7].lead_hand, 2);
        // Other tricks untouched.
        assert_eq!(m.track[8].lead_hand, 0);
    }

    #[test]
    fn init_removed_ranks_against_full_deck() {
        // 13 ranks per suit spread across 4 hands → removed_ranks
        // should be 0 (no cards missing).
        let mut p = Pos::default();
        // Give NORTH ranks 2..7, EAST 8..10, SOUTH J..K, WEST A.
        for bits in &BIT_MAP_RANK[2..=7] {
            p.rank_in_suit[0][0] |= bits;
        }
        for bits in &BIT_MAP_RANK[8..=10] {
            p.rank_in_suit[1][0] |= bits;
        }
        for bits in &BIT_MAP_RANK[11..=13] {
            p.rank_in_suit[2][0] |= bits;
        }
        p.rank_in_suit[3][0] |= BIT_MAP_RANK[14];

        let mut m = Moves::new();
        m.init_removed_ranks(12, &p);
        // The 13 cards span bits 0..=12 (ranks 2..=14 in BIT_MAP_RANK),
        // i.e. 0x1fff. Starting from 0xffff and XOR-ing off the deck
        // gives 0xffff ^ 0x1fff = 0xe000.
        assert_eq!(m.track[12].removed_ranks[0], 0xffff ^ 0x1fff);
    }

    #[test]
    fn move_gen_0_aces_only_one_per_suit() {
        // Each hand has just an ace in its own suit. Leader (north)
        // can only lead the spade ace.
        let p = make_pos_singleton_in_each_suit();
        let mut m = Moves::new();
        m.init_removed_ranks(0, &p);
        m.reinit(0, 0);
        m.set_trump(DDS_NOTRUMP);
        let bm = MoveType::default();
        let n = m.move_gen_0(0, &p, &bm, &bm, &[]);
        assert_eq!(n, 1, "leader has exactly one card to play");
        // The single move is the spade ace.
        let list = &m.move_list[0][0];
        assert_eq!(list.moves[0].suit, 0);
        assert_eq!(list.moves[0].rank, 14);
        assert_eq!(list.last, 0);
        assert_eq!(list.current, 0);
    }

    #[test]
    fn move_gen_0_akqj_collapses_to_one_per_suit() {
        // North holds AKQJ of spades — one sequence → one move.
        let p = make_pos_akqj_per_suit();
        let mut m = Moves::new();
        m.init_removed_ranks(0, &p);
        m.reinit(0, 0);
        m.set_trump(DDS_NOTRUMP);
        let bm = MoveType::default();
        let n = m.move_gen_0(0, &p, &bm, &bm, &[]);
        assert_eq!(n, 1, "AKQJ collapses to one candidate (top of run)");
        let list = &m.move_list[0][0];
        assert_eq!(list.moves[0].suit, 0);
        assert_eq!(list.moves[0].rank, 14);
    }

    #[test]
    fn make_next_returns_none_when_exhausted() {
        let p = make_pos_singleton_in_each_suit();
        let mut m = Moves::new();
        m.init_removed_ranks(0, &p);
        m.reinit(0, 0);
        let bm = MoveType::default();
        let _ = m.move_gen_0(0, &p, &bm, &bm, &[]);
        let win = [0u16; 4];
        let first = m.make_next(0, 0, &win);
        assert!(first.is_some(), "first call yields the ace");
        let second = m.make_next(0, 0, &win);
        assert!(second.is_none(), "list is exhausted after one move");
    }

    #[test]
    fn make_next_records_lead_suit_on_track() {
        let p = make_pos_akqj_per_suit();
        let mut m = Moves::new();
        m.init_removed_ranks(0, &p);
        m.reinit(0, 0);
        let bm = MoveType::default();
        m.move_gen_0(0, &p, &bm, &bm, &[]);
        let win = [0u16; 4];
        let mv = m.make_next(0, 0, &win).unwrap();
        assert_eq!(mv.suit, 0);
        assert_eq!(mv.rank, 14);
        assert_eq!(m.track[0].lead_suit, 0);
        assert_eq!(m.track[0].move_played[0].rank, 14);
        assert_eq!(m.track[0].high[0], 0);
    }

    #[test]
    fn merge_sort_empty_and_small() {
        // Empty / single shouldn't crash.
        let mut buf = [MoveType::default(); 14];
        merge_sort(&mut buf, 0);
        merge_sort(&mut buf, 1);

        // Two elements — higher weight should win.
        buf[0].weight = 5;
        buf[1].weight = 10;
        merge_sort(&mut buf, 2);
        assert_eq!(buf[0].weight, 10);
        assert_eq!(buf[1].weight, 5);
    }

    #[test]
    fn merge_sort_seven_elements_descending() {
        let mut buf = [MoveType::default(); 14];
        let weights = [3, 1, 7, 4, 6, 2, 5];
        for (i, &w) in weights.iter().enumerate() {
            buf[i].weight = w;
        }
        merge_sort(&mut buf, 7);
        let got: Vec<i32> = buf.iter().take(7).map(|m| m.weight).collect();
        assert_eq!(got, vec![7, 6, 5, 4, 3, 2, 1]);
    }

    #[test]
    fn merge_sort_thirteen_falls_back_to_insertion() {
        let mut buf = [MoveType::default(); 14];
        let weights = [1, 13, 4, 8, 2, 11, 5, 9, 3, 12, 6, 10, 7];
        for (i, &w) in weights.iter().enumerate() {
            buf[i].weight = w;
        }
        merge_sort(&mut buf, 13);
        let got: Vec<i32> = buf.iter().take(13).map(|m| m.weight).collect();
        assert_eq!(got, vec![13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1]);
    }

    #[test]
    fn winning_move_basic_cases() {
        let trump = 0;
        let mv = MoveType {
            suit: 0,
            rank: 5,
            sequence: 0,
            weight: 0,
        };
        let opp = ExtCard {
            suit: 0,
            rank: 3,
            sequence: 0,
        };
        assert!(Moves::winning_move(&mv, &opp, trump));
        // Higher rank of same suit wins.
        let opp_hi = ExtCard {
            suit: 0,
            rank: 7,
            sequence: 0,
        };
        assert!(!Moves::winning_move(&mv, &opp_hi, trump));
        // Different suit: trump wins.
        let mv_trump = MoveType {
            suit: 0,
            rank: 2,
            sequence: 0,
            weight: 0,
        };
        let opp_other = ExtCard {
            suit: 1,
            rank: 13,
            sequence: 0,
        };
        assert!(Moves::winning_move(&mv_trump, &opp_other, trump));
        // Non-trump on a different suit loses.
        let mv_nt = MoveType {
            suit: 1,
            rank: 12,
            sequence: 0,
            weight: 0,
        };
        let opp_lead = ExtCard {
            suit: 0,
            rank: 2,
            sequence: 0,
        };
        assert!(!Moves::winning_move(&mv_nt, &opp_lead, trump));
    }

    #[test]
    fn purge_drops_forbidden_moves() {
        let p = make_pos_akqj_per_suit();
        let mut m = Moves::new();
        m.init_removed_ranks(0, &p);
        m.reinit(0, 0);
        let bm = MoveType::default();
        let n_before = m.move_gen_0(0, &p, &bm, &bm, &[]);
        assert_eq!(n_before, 1);
        // Forbid the spade ace, starting at index 1 (per vendor loop).
        let mut forbidden = [MoveType::default(); 14];
        forbidden[1] = MoveType {
            suit: 0,
            rank: 14,
            sequence: 0,
            weight: 0,
        };
        m.purge(0, 0, &forbidden);
        assert_eq!(m.move_list[0][0].last, -1);
    }
}
