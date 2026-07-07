//! Oracle tests for `solve_board` against the FFI DDS 2.9 reference.
//!
//! Boards are random legal mid-play positions replayed from seeded random
//! deals. The oracle is the raw `ddss_sys::SolveBoard` with **mode = 1**:
//! the safe `ddss` wrapper hardcodes mode = 0, whose forced-single-card
//! `-2` score sentinel its own decoder cannot represent — pons-dds
//! implements mode-1 semantics instead (a documented divergence). Cards,
//! scores, equals masks, and output order must match the vendor exactly;
//! only the `nodes` counter is exempt (documented as approximate).

use contract_bridge::deck::full_deal;
use contract_bridge::hand::{Card, Holding, Rank};
use contract_bridge::{FullDeal, Seat, Strain, Suit};
use pons_dds::{Board, CurrentTrick, Objective, Target, TrickCount};
use rand::rngs::SmallRng;
use rand::{RngExt, SeedableRng};

/// Fixed RNG seed so the same corpus is exercised on every run.
const SEED: u64 = 0;

/// A play snapshot: remaining hands + trick in progress, evolved by
/// random legal plays.
struct Replay {
    hands: [contract_bridge::hand::Hand; 4],
    trump: Strain,
    leader: Seat,
    table: Vec<Card>,
}

impl Replay {
    fn new(deal: FullDeal, trump: Strain, leader: Seat) -> Self {
        Self {
            hands: core::array::from_fn(|i| deal[Seat::ALL[i]]),
            trump,
            leader,
            table: Vec::new(),
        }
    }

    const fn seat_to_play(&self) -> Seat {
        Seat::ALL[(self.leader as usize + self.table.len()) & 3]
    }

    fn cards_of(hand: contract_bridge::hand::Hand, suit_filter: Option<Suit>) -> Vec<Card> {
        let mut cards = Vec::new();
        for suit in Suit::ASC {
            if suit_filter.is_some_and(|f| f != suit) {
                continue;
            }
            for r in 2..=14u8 {
                if hand[suit].to_bits() & (1 << r) != 0 {
                    cards.push(Card {
                        suit,
                        rank: Rank::new(r),
                    });
                }
            }
        }
        cards
    }

    /// Legal cards for the seat on move: follow suit when possible.
    fn legal_moves(&self) -> Vec<Card> {
        let hand = self.hands[self.seat_to_play() as usize];
        if let Some(led) = self.table.first().map(|c| c.suit)
            && !hand[led].is_empty()
        {
            return Self::cards_of(hand, Some(led));
        }
        Self::cards_of(hand, None)
    }

    /// Play one uniformly random legal card, resolving the trick winner
    /// when the fourth card lands.
    fn play_random(&mut self, rng: &mut SmallRng) {
        let moves = self.legal_moves();
        let card = moves[rng.random_range(0..moves.len())];
        let seat = self.seat_to_play();
        self.hands[seat as usize].remove(card);
        self.table.push(card);

        if self.table.len() == 4 {
            let led = self.table[0].suit;
            let trump_suit = self.trump.suit();
            let winner_index = (0..4)
                .max_by_key(|&i| {
                    let c = self.table[i];
                    if Some(c.suit) == trump_suit {
                        (2, c.rank.get())
                    } else if c.suit == led {
                        (1, c.rank.get())
                    } else {
                        (0, 0)
                    }
                })
                .expect("four cards on the table");
            self.leader = Seat::ALL[(self.leader as usize + winner_index) & 3];
            self.table.clear();
        }
    }

    /// Snapshot the current position as a validated pons [`Board`] and
    /// the twin `ddss::Board` for the oracle.
    fn boards(&self) -> (Board, ddss::Board) {
        let mut builder = contract_bridge::deal::Builder::new();
        for (i, &hand) in self.hands.iter().enumerate() {
            builder[Seat::ALL[i]] = hand;
        }
        let remaining = builder.build_partial().expect("valid partial deal");

        let pons_trick = CurrentTrick::from_slice(self.trump, self.leader, &self.table)
            .expect("valid current trick");
        let pons_board = Board::try_new(remaining, pons_trick).expect("valid pons board");

        let ddss_trick = ddss::CurrentTrick::from_slice(self.trump, self.leader, &self.table)
            .expect("valid current trick");
        let ddss_board = ddss::Board::try_new(remaining, ddss_trick).expect("valid ddss board");

        (pons_board, ddss_board)
    }

    fn cards_left(&self) -> usize {
        self.hands.iter().map(|h| h.len()).sum()
    }
}

/// One decoded oracle/pons play for comparison.
type Line = (Card, Holding, u8);

/// Call the raw DDS `SolveBoard` with mode = 1 and decode the result.
fn oracle_solve(board: &ddss::Board, target: i32, solutions: i32) -> Vec<Line> {
    let dl = ddss_sys::deal::from(board.clone());
    let mut fut = ddss_sys::futureTricks::default();

    // Hold the ddss lock to serialize FFI access and to ensure the DDS
    // thread pool is initialized before the raw call on thread 0.
    let solver = ddss::Solver::lock();
    // SAFETY: `dl` and `fut` are valid; the lock serializes DDS access.
    let status = unsafe { ddss_sys::SolveBoard(dl, target, solutions, 1, &raw mut fut, 0) };
    core::mem::drop(solver);
    assert!(status >= 0, "ddss SolveBoard failed with status {status}");

    (0..usize::try_from(fut.cards).expect("non-negative card count"))
        .map(|i| {
            let suit = match fut.suit[i] {
                0 => Suit::Spades,
                1 => Suit::Hearts,
                2 => Suit::Diamonds,
                _ => Suit::Clubs,
            };
            (
                Card {
                    suit,
                    rank: Rank::new(u8::try_from(fut.rank[i]).expect("rank in range")),
                },
                Holding::from_bits_truncate(u16::try_from(fut.equals[i]).expect("equals fit")),
                u8::try_from(fut.score[i]).expect("score in range"),
            )
        })
        .collect()
}

fn pons_solve(solver: &mut pons_dds::Solver, board: &Board, target: Target) -> Vec<Line> {
    solver
        .solve_board(&Objective {
            board: board.clone(),
            target,
        })
        .plays
        .iter()
        .map(|p| (p.card, p.equals, p.score.get()))
        .collect()
}

/// How to compare a pons result against the oracle's.
#[derive(Clone, Copy)]
enum Mode {
    /// Cards, equals, scores, and order must all match.
    Exact,
    /// Same multiset of (card, equals, score); order exempt. Used where
    /// DDS's output *order* is history-dependent (stale best-move
    /// weights on its thread) but the set is not.
    Set,
}

/// Compare pons and oracle for one target.
fn check(
    solver: &mut pons_dds::Solver,
    pons_board: &Board,
    ddss_board: &ddss::Board,
    target: Target,
    mode: Mode,
    context: &str,
) {
    let mut pons = pons_solve(solver, pons_board, target);
    let mut oracle = oracle_solve(ddss_board, target.target(), target.solutions());
    if matches!(mode, Mode::Set) {
        pons.sort_by_key(|&(c, ..)| (c.suit, c.rank));
        oracle.sort_by_key(|&(c, ..)| (c.suit, c.rank));
    }
    assert_eq!(
        pons, oracle,
        "solve_board disagrees with ddss (mode 1) for {target:?} on {context}"
    );
}

/// The target matrix exercised per board.
///
/// The `Some(t)` paths probe without the per-probe best-move reset in
/// DDS, so which satisfying card DDS reports (`Any`) and in what order
/// (`All`) depends on what its thread solved before. The *sets* are
/// history-independent, so `All(Some(t))` compares as a set, and
/// `Any(Some(t))` is validated semantically: same reachability and
/// score, and pons's card must appear in the (already exactly-verified)
/// `Legal` listing with a true score of at least `t`.
fn check_matrix(
    solver: &mut pons_dds::Solver,
    pons_board: &Board,
    ddss_board: &ddss::Board,
    total_tricks: u8,
    context: &str,
) {
    check(
        solver,
        pons_board,
        ddss_board,
        Target::Any(None),
        Mode::Exact,
        context,
    );
    check(
        solver,
        pons_board,
        ddss_board,
        Target::All(None),
        Mode::Exact,
        context,
    );
    check(
        solver,
        pons_board,
        ddss_board,
        Target::Legal,
        Mode::Exact,
        context,
    );

    if total_tricks == 1 {
        // The last trick short-circuits through LastTrickWinner in both
        // engines — no search, no history dependence, and the score is
        // "does the lead side win" regardless of any Some(t) target.
        // Everything compares exactly.
        for target in [
            Target::Any(Some(TrickCount::new(0))),
            Target::All(Some(TrickCount::new(0))),
            Target::Any(Some(TrickCount::new(1))),
            Target::All(Some(TrickCount::new(1))),
        ] {
            check(solver, pons_board, ddss_board, target, Mode::Exact, context);
        }
        return;
    }

    // Trusted per-card truth: Legal was just verified exactly.
    let legal = pons_solve(solver, pons_board, Target::Legal);

    let max = pons_solve(solver, pons_board, Target::Any(None))
        .first()
        .map_or(0, |&(.., score)| score);
    let mut targets = vec![1, max, max + 1];
    targets.retain(|&t| (1..=total_tricks).contains(&t));
    targets.dedup();
    for t in targets {
        let tc = Some(TrickCount::new(t));

        let pons_any = pons_solve(solver, pons_board, Target::Any(tc));
        let oracle_any = oracle_solve(ddss_board, i32::from(t), 1);
        assert_eq!(
            pons_any.len(),
            oracle_any.len(),
            "Any(Some({t})) reachability disagrees on {context}"
        );
        if let [(card, equals, score)] = pons_any[..] {
            assert_eq!(score, t, "Any(Some({t})) must report the target itself");
            assert_eq!(oracle_any[0].2, t);
            assert!(
                legal
                    .iter()
                    .any(|&(c, e, s)| c == card && e == equals && s >= t),
                "Any(Some({t})) card {card:?} does not score {t}+ per Legal on {context}"
            );
        }

        check(
            solver,
            pons_board,
            ddss_board,
            Target::All(tc),
            Mode::Set,
            context,
        );
    }

    // `Some(0)` never searches: DDS lists moves in its history-dependent
    // stale-weight order. `All` compares as a set; `Any` returns a single
    // arbitrary legal group, so validate it against the Legal listing.
    let zero = Some(TrickCount::new(0));
    let pons_any0 = pons_solve(solver, pons_board, Target::Any(zero));
    let oracle_any0 = oracle_solve(ddss_board, 0, 1);
    assert_eq!(pons_any0.len(), 1, "Any(Some(0)) returns one card");
    assert_eq!(oracle_any0.len(), 1);
    let (card, equals, score) = pons_any0[0];
    assert_eq!(score, 0, "Any(Some(0)) does not score");
    assert!(
        legal.iter().any(|&(c, e, _)| c == card && e == equals),
        "Any(Some(0)) card {card:?} is not a legal group per Legal on {context}"
    );
    check(
        solver,
        pons_board,
        ddss_board,
        Target::All(zero),
        Mode::Set,
        context,
    );
}

/// Play a seeded random deal to several stop points and compare every
/// target variant against the oracle at each.
fn cross_check_boards(deals: usize) {
    let mut rng = SmallRng::seed_from_u64(SEED);
    let mut solver = pons_dds::Solver::default();

    for deal_no in 0..deals {
        let deal = full_deal(&mut rng);
        let trump = Strain::ASC[rng.random_range(0..5)];
        let leader = Seat::ALL[rng.random_range(0..4)];
        let mut replay = Replay::new(deal, trump, leader);

        // Stop points: (completed tricks, cards on table). The final
        // pair lands in the LastTrickWinner shortcut.
        let stops = [
            (0, 0),
            (rng.random_range(1..=5), 0),
            (rng.random_range(1..=9), rng.random_range(1..=3)),
            (11, rng.random_range(0..=3)),
            (12, rng.random_range(0..=3)),
        ];

        let mut played = 0;
        for (tricks, on_table) in stops {
            let want = tricks * 4 + on_table;
            while played < want {
                replay.play_random(&mut rng);
                played += 1;
            }

            let (pons_board, ddss_board) = replay.boards();
            let total_tricks = u8::try_from(replay.cards_left().div_ceil(4)).expect("≤ 13");
            let context = format!(
                "deal #{deal_no} {trump} led by {leader:?}, \
                 {tricks} tricks + {on_table} cards played\ndeal: {deal}"
            );
            if played == 0 {
                // Full deals are the slowest; the walk targets are
                // covered here and the full matrix on mid-game boards.
                check(
                    &mut solver,
                    &pons_board,
                    &ddss_board,
                    Target::Any(None),
                    Mode::Exact,
                    &context,
                );
                check(
                    &mut solver,
                    &pons_board,
                    &ddss_board,
                    Target::All(None),
                    Mode::Exact,
                    &context,
                );
            } else {
                check_matrix(
                    &mut solver,
                    &pons_board,
                    &ddss_board,
                    total_tricks,
                    &context,
                );
            }
        }
    }
}

/// Fast cross-check kept small enough to run by default on `cargo test`.
#[test]
fn boards_match_ddss() {
    cross_check_boards(8);
}

/// Heavy soak. Run during optimization with
/// `cargo test --release --test compare_ddss_boards -- --ignored`.
#[test]
#[ignore = "large board soak; run explicitly in release"]
fn boards_match_ddss_soak() {
    cross_check_boards(200);
}

/// The parallel batch entry must agree with sequential single solves.
#[test]
fn solve_boards_matches_singles() {
    let mut rng = SmallRng::seed_from_u64(SEED);
    let mut objectives = Vec::new();
    for _ in 0..6 {
        let deal = full_deal(&mut rng);
        let trump = Strain::ASC[rng.random_range(0..5)];
        let leader = Seat::ALL[rng.random_range(0..4)];
        let mut replay = Replay::new(deal, trump, leader);
        for _ in 0..rng.random_range(9..=27) {
            replay.play_random(&mut rng);
        }
        let (board, _) = replay.boards();
        for target in [Target::Any(None), Target::All(None), Target::Legal] {
            objectives.push(Objective {
                board: board.clone(),
                target,
            });
        }
    }

    let batch = pons_dds::solve_boards(&objectives);
    let mut solver = pons_dds::Solver::default();
    for (i, objective) in objectives.iter().enumerate() {
        assert_eq!(
            batch[i],
            solver.solve_board(objective),
            "batch vs single mismatch on objective #{i}"
        );
    }
}
