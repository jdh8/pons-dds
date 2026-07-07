//! `SolveBoard` driver: per-card double-dummy solving.
//!
//! Ports the target/solutions dispatch of `SolveBoardInternal`
//! ([`SolverIF.cpp`](../../../ddss-sys/vendor/src/SolverIF.cpp)) onto the
//! native [`Engine::root_probe`] loop. One deliberate divergence from the
//! vendor: pons-dds implements **mode-1 semantics** — a forced single move
//! group is scored like any other card — where the FFI crate's `mode = 0`
//! call returns the unevaluated `-2` sentinel that its own decoder cannot
//! represent. Everything else (cards, scores, equals, ordering) matches
//! the vendor bit-for-bit and is oracle-tested against `ddss-sys` with
//! `mode = 1`.

use crate::board::{Board, Objective, Target};
use crate::convert::{cb_suit_from_dds, dds_suit_from_cb};
use crate::lookup::BIT_MAP_RANK;
use crate::move_type::MoveType;
use crate::play::{FoundPlays, Play};
use crate::pos::Pos;
use crate::quick_tricks::{MAXNODE, MINNODE};
use crate::search::Engine;
use crate::tricks::TrickCount;
use crate::tt::TransTable;
use contract_bridge::hand::{Card, Holding, Rank};
use contract_bridge::{Seat, Suit};

const DDS_SUITS: usize = 4;
const DDS_HANDS: usize = 4;

/// Populate `Pos::rank_in_suit` from the remaining cards of a [`Board`].
/// The partial-deal sibling of `solver::pos_from_deal`.
fn pos_from_board(board: &Board) -> Pos {
    let mut pos = Pos::default();
    for (h, seat) in Seat::ALL.iter().enumerate() {
        let hand = board.remaining()[*seat];
        for cb_suit in Suit::ASC {
            let bits = hand[cb_suit].to_bits() >> 2;
            pos.rank_in_suit[h][dds_suit_from_cb(cb_suit)] = bits;
        }
    }
    pos
}

/// The current trick's table cards as DDS `(suit, rank)` pairs, in
/// playing order.
fn table_of(board: &Board) -> Vec<(i32, i32)> {
    board
        .current_cards()
        .iter()
        .map(|c| (dds_suit_from_cb(c.suit) as i32, i32::from(c.rank.get())))
        .collect()
}

/// Convert a root move and its score into a [`Play`].
const fn play_of(mv: &MoveType, score: i32) -> Play {
    Play {
        card: Card {
            suit: cb_suit_from_dds(mv.suit),
            rank: Rank::new(mv.rank as u8),
        },
        equals: Holding::from_bits_truncate((mv.sequence << 2) as u16),
        score: TrickCount::new(score as u8),
    }
}

/// The `cardCount <= 4` shortcut (vendor `LastTrickWinner`,
/// SolverIF.cpp:1121-1198): every play is forced, so just work out who
/// wins the final trick. Always returns exactly one card — the hand to
/// play's card — with `equals = 0` and `nodes = 0`.
fn last_trick_winner(board: &Board, target: Target, pos: &Pos, trump: i32) -> FoundPlays {
    let first = board.leader() as usize;
    let hand_rel_first = board.current_cards().len();
    let hand_to_play = (first + hand_rel_first) & 3;

    // (suit, rank) each hand plays: table cards for those who played,
    // the highest card of the first non-void DDS suit for the rest.
    let mut last_trick = [(0i32, 0i32); DDS_HANDS];
    for (k, &(s, r)) in table_of(board).iter().enumerate() {
        last_trick[(first + k) & 3] = (s, r);
    }
    for rel in hand_rel_first..DDS_HANDS {
        let h = (first + rel) & 3;
        for s in 0..DDS_SUITS {
            let bits = pos.rank_in_suit[h][s];
            if bits != 0 {
                last_trick[h] = (s as i32, bits.ilog2() as i32 + 2);
                break;
            }
        }
    }

    // Highest trump, else highest card in the led suit.
    let mut max_hand = usize::MAX;
    let mut max_rank = 0;
    if trump != crate::moves::DDS_NOTRUMP {
        for (h, &(s, r)) in last_trick.iter().enumerate() {
            if s == trump && r > max_rank {
                max_rank = r;
                max_hand = h;
            }
        }
    }
    if max_rank == 0 {
        let (lead_suit, lead_rank) = last_trick[first];
        max_rank = lead_rank;
        max_hand = first;
        for (h, &(s, r)) in last_trick.iter().enumerate() {
            if s == lead_suit && r > max_rank {
                max_rank = r;
                max_hand = h;
            }
        }
    }

    let lead_side_wins = i32::from(max_hand & 1 == hand_to_play & 1);
    let (suit, rank) = last_trick[hand_to_play];
    let score = if target.target() == 0 && target.solutions() < 3 {
        0
    } else {
        lead_side_wins
    };

    let mut result = FoundPlays::default();
    result.plays.push(play_of(
        &MoveType {
            suit,
            rank,
            ..MoveType::default()
        },
        score,
    ));
    result
}

/// Hint-anchored stepping walk over the trick target at the root
/// (vendor SolverIF.cpp:349-374 / :446-471). Returns the exact MAX
/// score and the cutoff move of the last successful probe. `guess`,
/// `lowerbound`, and `upperbound` follow the vendor's carried-over
/// bounds between Legal iterations.
struct Walk {
    guess: i32,
    lowerbound: i32,
    upperbound: i32,
}

impl Walk {
    fn run(
        &mut self,
        engine: &mut Engine,
        tt: &mut TransTable,
        pos: &mut Pos,
        ini_depth: i32,
        hand_rel_first: i32,
    ) -> (i32, MoveType) {
        let mut mv = MoveType::default();
        loop {
            engine.reset_best_moves();
            let val = engine.root_probe(pos, tt, self.guess, ini_depth, hand_rel_first);
            if val {
                mv = engine.best_move_at(ini_depth);
                self.lowerbound = self.guess;
                self.guess += 1;
            } else {
                self.guess -= 1;
                self.upperbound = self.guess;
            }
            if self.lowerbound >= self.upperbound {
                break;
            }
        }
        (self.lowerbound, mv)
    }
}

/// Solve one board on an engine + transposition table pair whose strain
/// is already set to the board's trump. The TT and per-deal tables are
/// rebuilt here for the board's remaining cards.
///
/// This is the whole `SolveBoardInternal` dispatch; see the module docs
/// for the deliberate divergences.
pub fn solve_board_on(
    engine: &mut Engine,
    tt: &mut TransTable,
    objective: &Objective,
) -> FoundPlays {
    let board = &objective.board;
    let base_pos = pos_from_board(board);

    let card_count: i32 = (0..DDS_HANDS)
        .map(|h| {
            (0..DDS_SUITS)
                .map(|s| base_pos.rank_in_suit[h][s].count_ones() as i32)
                .sum::<i32>()
        })
        .sum();

    // Documented divergences from the vendor's error codes: an empty
    // board and an unreachable target both yield an empty result.
    if card_count == 0 {
        return FoundPlays::default();
    }
    let total_tricks = (card_count + 3) / 4;
    let target = objective.target.target();
    let solutions = objective.target.solutions();
    if target > total_tricks {
        return FoundPlays::default();
    }

    let trump = engine.trump;
    if card_count <= 4 {
        return last_trick_winner(board, objective.target, &base_pos, trump);
    }

    let ini_depth = card_count - 4;
    let trick = (ini_depth + 3) >> 2;
    let hand_rel_first = (48 - ini_depth) % 4;
    let first = board.leader() as i32;
    let hand_to_play = (first + hand_rel_first) & 3;

    // ----- Per-board engine setup (vendor SolverIF.cpp:204-314) -----
    tt.reset();
    engine.set_deal_tables(&base_pos, tt);
    engine.set_node_types(if hand_to_play & 1 == 0 {
        [MAXNODE, MINNODE, MAXNODE, MINNODE]
    } else {
        [MINNODE, MAXNODE, MINNODE, MAXNODE]
    });

    let table = table_of(board);
    let mut table_aggr = [0u16; DDS_SUITS];
    for &(s, r) in &table {
        table_aggr[s as usize] |= BIT_MAP_RANK[r as usize];
    }

    let mut pos = base_pos;
    pos.first[ini_depth as usize] = first;
    engine.init_pos_with_table(&mut pos, table_aggr);

    engine.ini_depth = ini_depth;
    engine.nodes = 0;
    engine.clear_forbidden_moves();
    // Fresh-weight ordering: the vendor's setup movegen reads stale
    // best-move hints from earlier solves on the thread (its `Some(0)`
    // listing order is thread-history-dependent); pons-dds zeroes them
    // for a deterministic order.
    engine.reset_best_moves();

    // Replay the current trick into the moves track (vendor :249-279).
    // The vendor also regenerates each slot's move list along the way;
    // that has no observable effect in this port (every probe and
    // listing regenerates the slot it uses), so only the track state is
    // replayed here.
    engine.moves.reinit(trick, first);
    for (k, &(s, r)) in table.iter().enumerate() {
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
        .init_removed_ranks_with_table(trick, &pos, &table);
    engine.gen_root_moves(&pos, trick, hand_rel_first);
    let no_moves = engine.moves.get_length(trick, hand_rel_first);

    let mut result = FoundPlays::default();
    let static_guess = 7 - (hand_to_play & 1);

    match (solutions, target) {
        // ----- solutions == 3: all legal plays, scored ---------------
        (3, _) => {
            let mut forbidden = [MoveType::default(); 14];
            let mut walk = Walk {
                guess: static_guess,
                lowerbound: 0,
                upperbound: 13,
            };
            for mno in 0..no_moves {
                let (score, mv) = walk.run(engine, tt, &mut pos, ini_depth, hand_rel_first);
                if score != 0 {
                    result.plays.push(play_of(&mv, score));
                    forbidden[(mno + 1) as usize] = MoveType {
                        suit: mv.suit,
                        rank: mv.rank,
                        ..MoveType::default()
                    };
                    engine.set_forbidden_moves(&forbidden);
                    walk.guess = score;
                    walk.lowerbound = 0;
                } else {
                    // Every remaining group scores 0; list them in move
                    // order (vendor :391-408).
                    let no_left = engine.moves.get_length(trick, hand_rel_first);
                    engine.moves.rewind(trick, hand_rel_first);
                    for _ in 0..no_left {
                        let mp = engine
                            .moves
                            .make_next_simple(trick, hand_rel_first)
                            .expect("listed moves exist");
                        result.plays.push(play_of(&mp, 0));
                    }
                    break;
                }
            }
        }

        // ----- target == 0: only cards required, no scoring ----------
        (_, 0) => {
            let count = if solutions == 1 { 1 } else { no_moves };
            for _ in 0..count {
                let mp = engine
                    .moves
                    .make_next_simple(trick, hand_rel_first)
                    .expect("generated moves exist");
                result.plays.push(play_of(&mp, 0));
            }
        }

        // ----- target == -1: find the optimum score ------------------
        (_, -1) => {
            let mut walk = Walk {
                guess: static_guess,
                lowerbound: 0,
                upperbound: 13,
            };
            let (score, mv) = walk.run(engine, tt, &mut pos, ini_depth, hand_rel_first);
            if score == 0 {
                // Every move pays off 0 (vendor :474-496).
                let count = if solutions == 1 { 1 } else { no_moves };
                engine.moves.rewind(trick, hand_rel_first);
                for _ in 0..count {
                    let mp = engine
                        .moves
                        .make_next_simple(trick, hand_rel_first)
                        .expect("generated moves exist");
                    result.plays.push(play_of(&mp, 0));
                }
            } else {
                result.plays.push(play_of(&mv, score));
                if solutions == 2 {
                    all_continuation(
                        engine,
                        tt,
                        &mut pos,
                        &mut result,
                        Continuation {
                            ini_depth,
                            hand_rel_first,
                            trick,
                            no_moves,
                            score,
                            last_best: mv,
                        },
                    );
                }
            }
        }

        // ----- target >= 1: one probe at the user's target -----------
        _ => {
            engine.reset_best_moves();
            let val = engine.root_probe(&mut pos, tt, target, ini_depth, hand_rel_first);
            if val {
                let mv = engine.best_move_at(ini_depth);
                // The reported score is exactly the target, never the
                // true maximum (vendor :546).
                result.plays.push(play_of(&mv, target));
                if solutions == 2 {
                    all_continuation(
                        engine,
                        tt,
                        &mut pos,
                        &mut result,
                        Continuation {
                            ini_depth,
                            hand_rel_first,
                            trick,
                            no_moves,
                            score: target,
                            last_best: mv,
                        },
                    );
                }
            }
        }
    }

    engine.clear_forbidden_moves();
    result.nodes = engine.nodes;
    result
}

/// Parameters of the `solutions == 2` continuation loop.
#[derive(Clone, Copy)]
struct Continuation {
    ini_depth: i32,
    hand_rel_first: i32,
    trick: i32,
    no_moves: i32,
    score: i32,
    last_best: MoveType,
}

/// Find the other cards achieving `score` (vendor SolverIF.cpp:558-604):
/// repeatedly forbid every group up to and including the last best move
/// (in move-list order) and re-probe at the same target until a probe
/// fails.
fn all_continuation(
    engine: &mut Engine,
    tt: &mut TransTable,
    pos: &mut Pos,
    result: &mut FoundPlays,
    params: Continuation,
) {
    let Continuation {
        ini_depth,
        hand_rel_first,
        trick,
        no_moves,
        score,
        mut last_best,
    } = params;

    let mut forbidden = [MoveType::default(); 14];
    let mut forb = 1usize;
    let mut ind = 1;

    while ind < no_moves {
        engine.moves.rewind(trick, hand_rel_first);
        let num = engine.moves.get_length(trick, hand_rel_first);
        for _ in 0..num {
            let mp = engine
                .moves
                .make_next_simple(trick, hand_rel_first)
                .expect("listed moves exist");
            forbidden[forb] = MoveType {
                suit: mp.suit,
                rank: mp.rank,
                ..MoveType::default()
            };
            forb += 1;
            if last_best.suit == mp.suit && last_best.rank == mp.rank {
                break;
            }
        }
        engine.set_forbidden_moves(&forbidden);

        engine.reset_best_moves();
        let val = engine.root_probe(pos, tt, score, ini_depth, hand_rel_first);
        if !val {
            break;
        }
        last_best = engine.best_move_at(ini_depth);
        result.plays.push(play_of(&last_best, score));
        ind += 1;
    }
}

#[cfg(test)]
mod tests {
    use crate::board::{Board, CurrentTrick, Objective, Target};
    use crate::solver::Solver;
    use contract_bridge::hand::{Card, Holding, Rank};
    use contract_bridge::{FullDeal, Seat, Strain, Suit};

    /// Each seat holds one full 13-card suit: N spades, E hearts,
    /// S diamonds, W clubs.
    fn straight_flush_deal() -> FullDeal {
        "N:AKQJT98765432... .AKQJT98765432.. \
         ..AKQJT98765432. ...AKQJT98765432"
            .parse()
            .expect("fixture parses")
    }

    fn objective_at_trick_start(
        deal: FullDeal,
        trump: Strain,
        leader: Seat,
        target: Target,
    ) -> Objective {
        Objective {
            board: Board::try_new(deal.into(), CurrentTrick::new(trump, leader))
                .expect("valid board"),
            target,
        }
    }

    /// With spades trump and North (who holds all 13 spades) on lead,
    /// the entire suit is one sequence: exactly one candidate group —
    /// the ♠A with every lower spade as an equal — scoring 13.
    #[test]
    fn straight_flush_lead_wins_everything() {
        let obj = objective_at_trick_start(
            straight_flush_deal(),
            Strain::Spades,
            Seat::North,
            Target::Any(None),
        );
        let found = Solver::new(Strain::Spades).solve_board(&obj);
        assert_eq!(found.plays.len(), 1);
        let play = found.plays[0];
        assert_eq!(play.card.suit, Suit::Spades);
        assert_eq!(play.card.rank, Rank::new(14));
        assert_eq!(play.score.get(), 13);
        // Equals: K down to 2 — the full suit minus the ace.
        let below_ace = Holding::ALL - Holding::from_bits_truncate(1 << 14);
        assert_eq!(play.equals, below_ace);
    }

    /// Mid-trick: North leads the ♠A under a hearts trump; East (all
    /// hearts) is on move and ruffs everything — 13 tricks for EW,
    /// counting the current one.
    #[test]
    fn mid_trick_ruff_takes_all() {
        let deal = straight_flush_deal();
        let mut builder = contract_bridge::deal::Builder::from(deal);
        let ace = Card {
            suit: Suit::Spades,
            rank: Rank::new(14),
        };
        builder[Seat::North].remove(ace);
        let remaining = builder.build_partial().expect("valid partial");
        let trick =
            CurrentTrick::from_slice(Strain::Hearts, Seat::North, &[ace]).expect("one-card trick");
        let board = Board::try_new(remaining, trick).expect("valid board");

        let found = Solver::default().solve_board(&Objective {
            board,
            target: Target::Any(None),
        });
        assert_eq!(found.plays.len(), 1);
        let play = found.plays[0];
        assert_eq!(play.card.suit, Suit::Hearts);
        assert_eq!(play.score.get(), 13, "East ruffs and runs trumps");
    }

    /// Trick-start smoke against the hand-verified reference table
    /// (`solver::tests::solve_deal_matches_reference_pbn`): at notrump
    /// with West on lead, the defending side EW takes 8 tricks
    /// (13 − the NS declarer's 5).
    #[test]
    fn reference_pbn_lead_score() {
        let deal: FullDeal = "N:.63.AKQ987.A9732 A8654.KQ5.T.QJT6 \
                              J973.J98742.3.K4 KQT2.AT.J6542.85"
            .parse()
            .expect("reference PBN parses");
        let obj = objective_at_trick_start(deal, Strain::Notrump, Seat::West, Target::Any(None));
        let found = Solver::default().solve_board(&obj);
        assert_eq!(found.plays.len(), 1);
        assert_eq!(found.plays[0].score.get(), 8);

        // Legal must agree on the maximum and score every legal group.
        let legal = Solver::default().solve_board(&Objective {
            target: Target::Legal,
            ..obj
        });
        assert_eq!(
            legal.plays.iter().map(|p| p.score.get()).max(),
            Some(8),
            "Legal's best score must match Any(None)"
        );
        assert!(legal.plays.len() >= 8, "West has many distinct groups");
        // Descending scores with move-order ties.
        assert!(
            legal.plays.windows(2).all(|w| w[0].score >= w[1].score),
            "Legal output must be sorted by descending score"
        );
    }
}
