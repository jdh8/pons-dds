//! Play analysis: double-dummy trick counts along a play trace.
//!
//! Ports the semantics of `AnalysePlayBin`
//! ([`PlayAnalyser.cpp`](../../../ddss-sys/vendor/src/PlayAnalyser.cpp))
//! and the per-card `AnalyseLaterBoard` re-solve (`SolverIF.cpp`) onto the
//! native engine. The driver replays the trace card by card, keeps the
//! trick bookkeeping itself like the vendor does, and re-solves each
//! successive position with a one-sided seeded stepping walk: a played
//! card can only hurt the mover's own side, so the previous value is a
//! proven bound in the new position and the no-error case converges in a
//! single probe. The transposition table persists across the whole trace
//! (every position is a subposition of the snapshot, and TT bounds are
//! node-relative), which is the vendor's `AnalyseLater` reuse.
//!
//! Deliberate divergences from the FFI reference, all on the safe side:
//! the full `cards.len() + 1` trick counts are returned (DDS never
//! analyses the final trick and mis-counts entries on mid-trick
//! snapshots — to the point that its own wrapper can panic on short
//! traces); and revokes are detected as errors instead of being silently
//! mis-analysed.

use crate::board::Board;
use crate::convert::dds_suit_from_cb;
use crate::lookup::BIT_MAP_RANK;
use crate::move_type::MoveType;
use crate::moves::DDS_NOTRUMP;
use crate::play::{PlayAnalysis, PlayFaultError, PlayFaultKind, PlayTrace};
use crate::pos::Pos;
use crate::quick_tricks::{MAXNODE, MINNODE};
use crate::search::Engine;
use crate::tricks::TrickCount;
use crate::tt::TransTable;
use contract_bridge::{Seat, Suit};

const DDS_SUITS: usize = 4;

/// Replay state in DDS coordinates: remaining hand bitmaps, the trick in
/// progress, and the declarer-side bookkeeping.
struct Replay {
    rank_in_suit: [[u16; 4]; 4],
    /// `(suit, rank)` cards on the table, in playing order.
    table: Vec<(i32, i32)>,
    /// Hand index leading the current trick.
    leader: i32,
    /// Hand-index parity (0 = N/S, 1 = E/W) of the declarer side: the
    /// side *not* on lead at the snapshot (vendor PlayAnalyser.cpp:56).
    declarer_parity: i32,
    /// Completed tricks won by the declarer side during the replay.
    banked: u8,
    trump: i32,
}

impl Replay {
    fn cards_left(&self) -> i32 {
        self.rank_in_suit
            .iter()
            .flatten()
            .map(|bits| bits.count_ones() as i32)
            .sum()
    }

    /// Play one validated card: remove it from the hand, resolve the
    /// trick winner when the fourth card lands, and bank declarer
    /// tricks.
    fn play(&mut self, suit: i32, rank: i32) {
        let seat = ((self.leader + self.table.len() as i32) & 3) as usize;
        self.rank_in_suit[seat][suit as usize] &= !BIT_MAP_RANK[rank as usize];
        self.table.push((suit, rank));

        if self.table.len() == 4 {
            let winner_rel = self.trick_winner_rel();
            self.leader = (self.leader + winner_rel) & 3;
            if self.leader & 1 == self.declarer_parity {
                self.banked += 1;
            }
            self.table.clear();
        }
    }

    /// Relative position (0-3 from the leader) of the current table's
    /// winning card. Requires a full table of 4 cards.
    fn trick_winner_rel(&self) -> i32 {
        let led = self.table[0].0;
        (0..4)
            .max_by_key(|&rel| {
                let (s, r) = self.table[rel as usize];
                if self.trump != DDS_NOTRUMP && s == self.trump {
                    (2, r)
                } else if s == led {
                    (1, r)
                } else {
                    (0, 0)
                }
            })
            .expect("four cards on the table")
    }

    /// Declarer-side tricks in the forced endgame: with at most one card
    /// per hand, every play is forced, so complete the final trick and
    /// score it. `banked` plus this is the total for every remaining
    /// position of the trace.
    fn forced_final_value(&self) -> u8 {
        if self.cards_left() == 0 && self.table.is_empty() {
            return 0;
        }
        let mut probe = Self {
            rank_in_suit: self.rank_in_suit,
            table: self.table.clone(),
            leader: self.leader,
            declarer_parity: self.declarer_parity,
            banked: 0,
            trump: self.trump,
        };
        while probe.table.len() < 4 {
            let seat = ((probe.leader + probe.table.len() as i32) & 3) as usize;
            let (suit, rank) = (0..DDS_SUITS)
                .find_map(|s| {
                    let bits = probe.rank_in_suit[seat][s];
                    (bits != 0).then(|| (s as i32, bits.ilog2() as i32 + 2))
                })
                .expect("forced endgame hand has a card");
            probe.play(suit, rank);
        }
        probe.banked
    }
}

/// Exact declarer-side future tricks of the replay's current position,
/// probed with a seeded one-sided stepping walk. `lower`/`upper` seed
/// the walk's bounds and `guess` its first probe; any sound seeds give
/// the same value, only the probe count varies.
fn solve_value(
    engine: &mut Engine,
    tt: &mut TransTable,
    replay: &Replay,
    guess: i32,
    lower_seed: i32,
    upper_seed: i32,
) -> u8 {
    let card_count = replay.cards_left();
    if card_count == 0 {
        return 0;
    }
    if card_count <= 4 {
        return replay.forced_final_value();
    }

    let ini_depth = card_count - 4;
    let trick = (ini_depth + 3) >> 2;
    let hand_rel_first = (48 - ini_depth) % 4;
    debug_assert_eq!(hand_rel_first as usize, replay.table.len());

    let mut pos = Pos {
        rank_in_suit: replay.rank_in_suit,
        ..Pos::default()
    };
    pos.first[ini_depth as usize] = replay.leader;

    let mut table_aggr = [0u16; DDS_SUITS];
    for &(s, r) in &replay.table {
        table_aggr[s as usize] |= BIT_MAP_RANK[r as usize];
    }
    engine.init_pos_with_table(&mut pos, table_aggr);

    engine.ini_depth = ini_depth;
    engine.moves.reinit(trick, replay.leader);
    for (k, &(s, r)) in replay.table.iter().enumerate() {
        let mv = MoveType {
            suit: s,
            rank: r,
            ..MoveType::default()
        };
        pos.move_history[(ini_depth + hand_rel_first - k as i32) as usize] = mv;
        engine.moves.make_specific(&mv, trick, k as i32);
    }
    engine
        .moves
        .init_removed_ranks_with_table(trick, &pos, &replay.table);

    let total_tricks = (card_count + 3) / 4;
    let mut lowerbound = lower_seed.clamp(0, total_tricks);
    let mut upperbound = upper_seed.clamp(lowerbound, total_tricks);
    let mut guess = guess.clamp(lowerbound, upperbound);
    if lowerbound >= upperbound {
        return lowerbound as u8;
    }
    // Probe at 0 is vacuous (any target ≤ 0 holds); start at 1.
    guess = guess.max(1);

    loop {
        engine.reset_best_moves();
        let val = engine.root_probe(&mut pos, tt, guess, ini_depth, hand_rel_first);
        if val {
            lowerbound = guess;
            guess += 1;
        } else {
            guess -= 1;
            upperbound = guess;
        }
        if lowerbound >= upperbound {
            break;
        }
    }
    lowerbound as u8
}

/// Analyse one trace on an engine + transposition table pair whose
/// strain is already set to the board's trump. See the module docs.
///
/// # Errors
///
/// Returns a [`PlayFaultError`] naming the first illegal trace card —
/// not held by the player on turn, or a revoke.
pub fn analyse_play_on(
    engine: &mut Engine,
    tt: &mut TransTable,
    trace: &PlayTrace,
) -> Result<PlayAnalysis, PlayFaultError> {
    let board = &trace.board;

    let mut replay = Replay {
        rank_in_suit: pos_bitmaps(board),
        table: board
            .current_cards()
            .iter()
            .map(|c| (dds_suit_from_cb(c.suit) as i32, i32::from(c.rank.get())))
            .collect(),
        leader: board.leader() as i32,
        declarer_parity: (board.leader() as i32 & 1) ^ 1,
        banked: 0,
        trump: engine.trump,
    };

    // Per-trace engine setup: tables from the snapshot's remaining cards
    // serve every subposition of the trace, and the TT persists across
    // all of them (bounds are node-relative and side-0-framed).
    tt.reset();
    engine.set_deal_tables(
        &Pos {
            rank_in_suit: replay.rank_in_suit,
            ..Pos::default()
        },
        tt,
    );
    // MAX = the declarer side, fixed for the whole trace, so every walk
    // value is declarer-side future tricks directly.
    engine.set_node_types(if replay.declarer_parity == 0 {
        [MAXNODE, MINNODE, MAXNODE, MINNODE]
    } else {
        [MINNODE, MAXNODE, MINNODE, MAXNODE]
    });
    engine.clear_forbidden_moves();

    let mut tricks = arrayvec::ArrayVec::new();
    let total = (replay.cards_left() + 3) / 4;
    let value = solve_value(
        engine,
        tt,
        &replay,
        7 - (replay.declarer_parity & 1),
        0,
        total,
    );
    let mut prev_total = replay.banked + value;
    tricks.push(TrickCount::new(prev_total));

    for (index, card) in trace.cards.iter().enumerate() {
        let suit = dds_suit_from_cb(card.suit) as i32;
        let rank = i32::from(card.rank.get());
        let seat_index = (replay.leader + replay.table.len() as i32) & 3;
        let seat = Seat::ALL[seat_index as usize];

        // Validate before solving: the seeded bounds below are proven
        // only for legal plays.
        if replay.rank_in_suit[seat_index as usize][suit as usize] & BIT_MAP_RANK[rank as usize]
            == 0
        {
            return Err(PlayFaultError {
                index,
                seat,
                card: *card,
                kind: PlayFaultKind::NotHeld,
            });
        }
        if let Some(&(led, _)) = replay.table.first()
            && suit != led
            && replay.rank_in_suit[seat_index as usize][led as usize] != 0
        {
            return Err(PlayFaultError {
                index,
                seat,
                card: *card,
                kind: PlayFaultKind::Revoke,
            });
        }

        let mover_on_declarer_side = seat_index & 1 == replay.declarer_parity;
        replay.play(suit, rank);

        // Monotonicity: a card can only hurt the mover's own side, so
        // the previous total bounds the new value from one side.
        let remaining = (replay.cards_left() + 3) / 4;
        let hint = i32::from(prev_total) - i32::from(replay.banked);
        let value = if mover_on_declarer_side {
            solve_value(engine, tt, &replay, hint, 0, hint.min(remaining))
        } else {
            solve_value(engine, tt, &replay, hint + 1, hint, remaining)
        };

        prev_total = replay.banked + value;
        tricks.push(TrickCount::new(prev_total));
    }

    Ok(PlayAnalysis { tricks })
}

/// The board's remaining cards as DDS bitmaps.
fn pos_bitmaps(board: &Board) -> [[u16; 4]; 4] {
    let mut rank_in_suit = [[0u16; 4]; 4];
    for (h, seat) in Seat::ALL.iter().enumerate() {
        let hand = board.remaining()[*seat];
        for cb_suit in Suit::ASC {
            rank_in_suit[h][dds_suit_from_cb(cb_suit)] = hand[cb_suit].to_bits() >> 2;
        }
    }
    rank_in_suit
}

#[cfg(test)]
mod tests {
    use crate::board::{Board, CurrentTrick};
    use crate::play::{PlayFaultKind, PlayTrace};
    use crate::solver::Solver;
    use arrayvec::ArrayVec;
    use contract_bridge::hand::{Card, Rank};
    use contract_bridge::{FullDeal, Seat, Strain, Suit};

    fn straight_flush_deal() -> FullDeal {
        "N:AKQJT98765432... .AKQJT98765432.. \
         ..AKQJT98765432. ...AKQJT98765432"
            .parse()
            .expect("fixture parses")
    }

    fn card(suit: Suit, rank: u8) -> Card {
        Card {
            suit,
            rank: Rank::new(rank),
        }
    }

    fn trace_of(deal: FullDeal, trump: Strain, leader: Seat, cards: &[Card]) -> PlayTrace {
        PlayTrace {
            board: Board::try_new(deal.into(), CurrentTrick::new(trump, leader))
                .expect("valid board"),
            cards: ArrayVec::try_from(cards).expect("≤ 52 cards"),
        }
    }

    /// Spades trump, East on lead → declarer side is N/S, and North's
    /// 13 trumps take everything no matter what is played. The count
    /// stays 13 across a full first trick.
    #[test]
    fn straight_flush_all_thirteen() {
        let trace = trace_of(
            straight_flush_deal(),
            Strain::Spades,
            Seat::East,
            &[
                card(Suit::Hearts, 14),  // E leads ♥A
                card(Suit::Diamonds, 2), // S discards ♦2
                card(Suit::Clubs, 2),    // W discards ♣2
                card(Suit::Spades, 2),   // N ruffs ♠2
            ],
        );
        let analysis = Solver::default().analyse_play(&trace);
        assert_eq!(analysis.tricks.len(), 5);
        assert!(
            analysis.tricks.iter().all(|tc| tc.get() == 13),
            "NS ruff and run trumps whatever happens: {:?}",
            analysis.tricks
        );
    }

    /// An empty trace still reports the snapshot value.
    #[test]
    fn empty_trace() {
        let trace = trace_of(straight_flush_deal(), Strain::Notrump, Seat::West, &[]);
        let analysis = Solver::default().analyse_play(&trace);
        // At notrump the opening leader runs their suit: declarer (NS,
        // opposite West) takes nothing.
        assert_eq!(analysis.tricks.len(), 1);
        assert_eq!(analysis.tricks[0].get(), 0);
    }

    /// Illegal traces surface descriptive faults instead of wrong
    /// analyses: a card the player does not hold, and a revoke.
    #[test]
    fn trace_validation() {
        // East does not hold the ♠A.
        let not_held = trace_of(
            straight_flush_deal(),
            Strain::Notrump,
            Seat::East,
            &[card(Suit::Spades, 14)],
        );
        let fault = Solver::default()
            .try_analyse_play(&not_held)
            .expect_err("East does not hold the ace of spades");
        assert_eq!(fault.index, 0);
        assert_eq!(fault.seat, Seat::East);
        assert_eq!(fault.kind, PlayFaultKind::NotHeld);

        // Give North a heart so failing to follow East's heart lead is
        // a revoke: swap ♥2 (E) with ♠2 (N).
        let mut builder = contract_bridge::deal::Builder::from(straight_flush_deal());
        builder[Seat::East].remove(card(Suit::Hearts, 2));
        builder[Seat::East].insert(card(Suit::Spades, 2));
        builder[Seat::North].remove(card(Suit::Spades, 2));
        builder[Seat::North].insert(card(Suit::Hearts, 2));
        let deal = builder.build_full().expect("valid full deal");

        let revoke = trace_of(
            deal,
            Strain::Notrump,
            Seat::East,
            &[
                card(Suit::Hearts, 14),  // E leads ♥A
                card(Suit::Diamonds, 2), // S is void in hearts — legal
                card(Suit::Clubs, 2),    // W is void in hearts — legal
                card(Suit::Spades, 3),   // N holds ♥2 → revoke
            ],
        );
        let fault = Solver::default()
            .try_analyse_play(&revoke)
            .expect_err("North must follow hearts");
        assert_eq!(fault.index, 3);
        assert_eq!(fault.seat, Seat::North);
        assert_eq!(fault.kind, PlayFaultKind::Revoke);
    }
}
