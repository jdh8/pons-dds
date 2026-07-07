//! Oracle and property tests for `analyse_play`.
//!
//! The oracle is `ddss::Solver::analyse_play` (DDS 2.9 `AnalysePlayBin`).
//! DDS truncates its output — it never analyses the final trick, so a
//! trace of `n` cards from a trick-boundary snapshot yields
//! `min(n + 1, 4·numTricks − 3)` entries — while pons-dds returns the
//! documented full `n + 1`. The tests encode DDS's length exactly and
//! compare element-wise over the common prefix, so a genuine divergence
//! still fails loudly. Mid-trick snapshots are kept away from the oracle
//! (DDS silently drops trailing cards there and its wrapper can panic on
//! short traces) and are covered instead by properties anchored on the
//! separately oracle-verified `solve_board`.

use arrayvec::ArrayVec;
use contract_bridge::deck::full_deal;
use contract_bridge::hand::{Card, Rank};
use contract_bridge::{FullDeal, Seat, Strain, Suit};
use pons_dds::{Board, CurrentTrick, Objective, PlayTrace, Target};
use rand::rngs::SmallRng;
use rand::{RngExt, SeedableRng};

/// Fixed RNG seed so the same corpus is exercised on every run.
const SEED: u64 = 0;

/// Replay helper: remaining hands + trick in progress.
struct Replay {
    hands: [contract_bridge::hand::Hand; 4],
    trump: Strain,
    leader: Seat,
    table: Vec<Card>,
    /// Completed tricks won by the side opposite the original leader.
    declarer_tricks: u8,
    declarer_parity: usize,
}

impl Replay {
    fn new(deal: FullDeal, trump: Strain, leader: Seat) -> Self {
        Self {
            hands: core::array::from_fn(|i| deal[Seat::ALL[i]]),
            trump,
            leader,
            table: Vec::new(),
            declarer_tricks: 0,
            declarer_parity: (leader as usize & 1) ^ 1,
        }
    }

    const fn seat_to_play(&self) -> Seat {
        Seat::ALL[(self.leader as usize + self.table.len()) & 3]
    }

    /// Legal cards for the seat on move: follow suit when possible.
    fn legal_moves(&self) -> Vec<Card> {
        let hand = self.hands[self.seat_to_play() as usize];
        let follow = self
            .table
            .first()
            .map(|c| c.suit)
            .filter(|&led| !hand[led].is_empty());
        let mut cards = Vec::new();
        for suit in Suit::ASC {
            if follow.is_some_and(|f| f != suit) {
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

    /// Play one uniformly random legal card and return it.
    fn play_random(&mut self, rng: &mut SmallRng) -> Card {
        let moves = self.legal_moves();
        let card = moves[rng.random_range(0..moves.len())];
        let seat = self.seat_to_play();
        self.hands[seat as usize].remove(card);
        self.table.push(card);

        if self.table.len() == 4 {
            let led = self.table[0].suit;
            let trump_suit = self.trump.suit();
            let winner = (0..4)
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
            self.leader = Seat::ALL[(self.leader as usize + winner) & 3];
            if self.leader as usize & 1 == self.declarer_parity {
                self.declarer_tricks += 1;
            }
            self.table.clear();
        }
        card
    }

    /// Snapshot the position as a validated board.
    fn board(&self) -> Board {
        let mut builder = contract_bridge::deal::Builder::new();
        for (i, &hand) in self.hands.iter().enumerate() {
            builder[Seat::ALL[i]] = hand;
        }
        let remaining = builder.build_partial().expect("valid partial deal");
        let trick = CurrentTrick::from_slice(self.trump, self.leader, &self.table)
            .expect("valid current trick");
        Board::try_new(remaining, trick).expect("valid board")
    }
}

/// A random legal trace of `len` cards from a fresh deal at trick start.
fn random_trace(
    deal: FullDeal,
    trump: Strain,
    leader: Seat,
    len: usize,
    rng: &mut SmallRng,
) -> PlayTrace {
    let mut replay = Replay::new(deal, trump, leader);
    let board = replay.board();
    let cards: ArrayVec<Card, 52> = (0..len).map(|_| replay.play_random(rng)).collect();
    PlayTrace { board, cards }
}

/// Compare a trace against the ddss oracle: pons returns the full
/// `n + 1` counts, ddss its truncated prefix; the prefix must agree
/// element-wise.
fn check_against_oracle(trace: &PlayTrace, context: &str) {
    let pons = pons_dds::analyse_play(trace);
    let n = trace.cards.len();
    assert_eq!(pons.tricks.len(), n + 1, "pons returns full length");

    let ddss_trace = ddss::PlayTrace {
        board: ddss::Board::try_new(
            *trace.board.remaining(),
            ddss::CurrentTrick::from_slice(
                trace.board.trump(),
                trace.board.leader(),
                trace.board.current_cards(),
            )
            .expect("valid ddss trick"),
        )
        .expect("valid ddss board"),
        cards: trace.cards.clone(),
    };
    let solver = ddss::Solver::lock();
    let oracle = solver.analyse_play(&ddss_trace);
    core::mem::drop(solver);

    // DDS never analyses the final trick (PlayAnalyser.cpp:64-73): from
    // a trick-boundary snapshot it emits min(n + 1, 4·numTricks − 3)
    // entries.
    let num_tricks = trace.board.remaining().collected().len().div_ceil(4);
    let expected = (n + 1).min(4 * num_tricks - 3);
    assert_eq!(
        oracle.tricks.len(),
        expected,
        "unexpected ddss truncation on {context}"
    );

    for (i, (p, d)) in pons.tricks.iter().zip(&oracle.tricks).enumerate() {
        assert_eq!(
            p.get(),
            u8::from(*d),
            "analysis disagrees with ddss at step {i} on {context}"
        );
    }
}

/// Oracle comparison over random legal traces of assorted lengths,
/// including full 52-card traces (exercising DDS's final-trick
/// truncation) and empty ones.
#[test]
fn plays_match_ddss() {
    let mut rng = SmallRng::seed_from_u64(SEED);
    for deal_no in 0..6 {
        let deal = full_deal(&mut rng);
        let trump = Strain::ASC[rng.random_range(0..5)];
        let leader = Seat::ALL[rng.random_range(0..4)];
        for len in [0, 1, rng.random_range(2..=30), 52] {
            let trace = random_trace(deal, trump, leader, len, &mut rng);
            let context =
                format!("deal #{deal_no} {trump} led by {leader:?}, {len} cards\ndeal: {deal}");
            check_against_oracle(&trace, &context);
        }
    }
}

/// Heavy soak. Run explicitly with
/// `cargo test --release --test compare_ddss_plays -- --ignored`.
#[test]
#[ignore = "play-analysis soak; run explicitly in release"]
fn plays_match_ddss_soak() {
    let mut rng = SmallRng::seed_from_u64(SEED);
    for _ in 0..100 {
        let deal = full_deal(&mut rng);
        let trump = Strain::ASC[rng.random_range(0..5)];
        let leader = Seat::ALL[rng.random_range(0..4)];
        for len in [0, 1, rng.random_range(2..=51), 52] {
            let trace = random_trace(deal, trump, leader, len, &mut rng);
            check_against_oracle(
                &trace,
                &format!("soak {trump} {leader:?} len {len}\ndeal: {deal}"),
            );
        }
    }
}

/// Properties that need no oracle, over mid-trick snapshots too (which
/// DDS cannot analyse correctly):
///
/// 1. A declarer-side card never increases the count; a defender's card
///    never decreases it.
/// 2. Analysing a prefix yields the leading entries of the full run.
/// 3. The final entry of a full trace equals the replay-counted
///    declarer tricks.
/// 4. `tricks[0]` agrees with the separately oracle-verified
///    `solve_board` value of the snapshot.
#[test]
fn analysis_properties() {
    let mut rng = SmallRng::seed_from_u64(SEED);
    for _ in 0..4 {
        let deal = full_deal(&mut rng);
        let trump = Strain::ASC[rng.random_range(0..5)];
        let leader = Seat::ALL[rng.random_range(0..4)];

        // Build a mid-trick snapshot by replaying a random prefix, then
        // trace the whole rest of the play from there.
        let mut replay = Replay::new(deal, trump, leader);
        for _ in 0..rng.random_range(0..=9) {
            replay.play_random(&mut rng);
        }
        let board = replay.board();
        let snapshot_leader = board.leader();
        let cards_left = board.remaining().collected().len();
        let mut cards = ArrayVec::<Card, 52>::new();
        for _ in 0..cards_left {
            cards.push(replay.play_random(&mut rng));
        }
        let declarer_parity = (snapshot_leader as usize & 1) ^ 1;

        let trace = PlayTrace {
            board: board.clone(),
            cards,
        };
        let analysis = pons_dds::analyse_play(&trace);
        assert_eq!(analysis.tricks.len(), trace.cards.len() + 1);

        // Property 1: per-mover monotonicity.
        for (i, w) in analysis.tricks.windows(2).enumerate() {
            let card = trace.cards[i];
            let mover = next_mover(&trace, i);
            if mover & 1 == declarer_parity {
                assert!(
                    w[1] <= w[0],
                    "declarer-side card {card} raised the count at step {i}"
                );
            } else {
                assert!(
                    w[1] >= w[0],
                    "defender card {card} lowered the count at step {i}"
                );
            }
        }

        // Property 2: prefix invariance.
        let cut = trace.cards.len() / 2;
        let prefix = PlayTrace {
            board: board.clone(),
            cards: trace.cards[..cut].iter().copied().collect(),
        };
        let prefix_analysis = pons_dds::analyse_play(&prefix);
        assert_eq!(
            prefix_analysis.tricks[..],
            analysis.tricks[..=cut],
            "prefix analysis must match the leading entries"
        );

        // Property 3: final entry equals the replayed declarer count —
        // counted from the snapshot onward, in the snapshot's frame
        // (declarer = the side opposite the snapshot's leader).
        assert_eq!(
            analysis.tricks.last().expect("non-empty").get(),
            declarer_tricks_of(&trace),
            "final entry must equal the replayed declarer tricks"
        );

        // Property 4: tricks[0] agrees with solve_board on the snapshot.
        let found = pons_dds::solve_board(&Objective {
            board: board.clone(),
            target: Target::Any(None),
        });
        let mover_side_score = found.plays.first().map_or(0, |p| p.score.get());
        let total = u8::try_from(cards_left.div_ceil(4)).expect("≤ 13 tricks");
        let mover_parity = (snapshot_leader as usize + board.current_cards().len()) & 1;
        let expected = if mover_parity == declarer_parity {
            mover_side_score
        } else {
            total - mover_side_score
        };
        assert_eq!(
            analysis.tricks[0].get(),
            expected,
            "tricks[0] must agree with solve_board's snapshot value"
        );
    }
}

/// Tricks won during the trace by the snapshot's declarer side (the
/// side opposite the snapshot leader), by independent replay.
fn declarer_tricks_of(trace: &PlayTrace) -> u8 {
    let board = &trace.board;
    let declarer_parity = (board.leader() as usize & 1) ^ 1;
    let mut leader = board.leader() as usize;
    let mut table: Vec<Card> = board.current_cards().to_vec();
    let trump = board.trump().suit();
    let mut tricks = 0;
    for card in &trace.cards {
        table.push(*card);
        if table.len() == 4 {
            let led = table[0].suit;
            let winner = (0..4)
                .max_by_key(|&i| {
                    let c = table[i];
                    if Some(c.suit) == trump {
                        (2, c.rank.get())
                    } else if c.suit == led {
                        (1, c.rank.get())
                    } else {
                        (0, 0)
                    }
                })
                .expect("full trick");
            leader = (leader + winner) & 3;
            if leader & 1 == declarer_parity {
                tricks += 1;
            }
            table.clear();
        }
    }
    tricks
}

/// The hand index on move after `played` trace cards.
fn next_mover(trace: &PlayTrace, played: usize) -> usize {
    // Recompute by replaying — simple and independent of the driver.
    let board = &trace.board;
    let mut hands: [contract_bridge::hand::Hand; 4] =
        core::array::from_fn(|i| board.remaining()[Seat::ALL[i]]);
    let mut leader = board.leader() as usize;
    let mut table: Vec<Card> = board.current_cards().to_vec();
    let trump = board.trump().suit();
    for card in &trace.cards[..played] {
        let seat = (leader + table.len()) & 3;
        hands[seat].remove(*card);
        table.push(*card);
        if table.len() == 4 {
            let led = table[0].suit;
            let winner = (0..4)
                .max_by_key(|&i| {
                    let c = table[i];
                    if Some(c.suit) == trump {
                        (2, c.rank.get())
                    } else if c.suit == led {
                        (1, c.rank.get())
                    } else {
                        (0, 0)
                    }
                })
                .expect("full trick");
            leader = (leader + winner) & 3;
            table.clear();
        }
    }
    (leader + table.len()) & 3
}

/// The parallel batch entry must agree with sequential single analyses.
#[test]
fn analyse_plays_matches_singles() {
    let mut rng = SmallRng::seed_from_u64(SEED);
    let traces: Vec<PlayTrace> = (0..8)
        .map(|_| {
            let deal = full_deal(&mut rng);
            let trump = Strain::ASC[rng.random_range(0..5)];
            let leader = Seat::ALL[rng.random_range(0..4)];
            let len = rng.random_range(0..=52);
            random_trace(deal, trump, leader, len, &mut rng)
        })
        .collect();

    let batch = pons_dds::analyse_plays(&traces);
    let mut solver = pons_dds::Solver::default();
    for (i, trace) in traces.iter().enumerate() {
        assert_eq!(
            batch[i],
            solver.analyse_play(trace),
            "batch vs single mismatch on trace #{i}"
        );
    }
}
