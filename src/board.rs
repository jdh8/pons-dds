//! Solving input: boards, tricks-in-progress, targets, and objectives
//!
//! Mirrors `ddss::board` (the FFI reference crate) minus the FFI
//! conversions, so a `pons` migration between the two crates is a
//! near-mechanical swap.

use crate::tricks::TrickCount;
use contract_bridge::deal::PartialDeal;
use contract_bridge::hand::{Card, Hand};
use contract_bridge::seat::Seat;
use contract_bridge::{Strain, Suit};

use arrayvec::ArrayVec;
use thiserror::Error;

/// Target tricks and number of solutions to find
///
/// Corresponds to the `target` and `solutions` arguments of the DDS
/// `SolveBoard` entry point. The associated `Option<TrickCount>` selects
/// between a minimum target (`Some`) and "find the most tricks" (`None`);
/// the `-1` sentinel is produced by [`Target::target`] and is not part of
/// the public payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    /// Find any card that fulfills the target
    ///
    /// - `Some(tc)`: any card scoring at least `tc` tricks
    /// - `None`: any card scoring the most tricks
    Any(Option<TrickCount>),

    /// Find all cards that fulfill the target
    ///
    /// - `Some(tc)`: all cards scoring at least `tc` tricks
    /// - `None`: all cards scoring the most tricks
    All(Option<TrickCount>),

    /// Solve for all legal plays
    ///
    /// Cards are sorted with their scores in descending order.
    Legal,
}

impl Target {
    /// Get the DDS `target` argument: the minimum trick count, or `-1`
    /// for "find the most tricks"
    #[must_use]
    #[inline]
    pub const fn target(self) -> i32 {
        match self {
            Self::Any(Some(tc)) | Self::All(Some(tc)) => tc.get() as i32,
            Self::Any(None) | Self::All(None) | Self::Legal => -1,
        }
    }

    /// Get the DDS `solutions` argument
    #[must_use]
    #[inline]
    pub const fn solutions(self) -> i32 {
        match self {
            Self::Any(_) => 1,
            Self::All(_) => 2,
            Self::Legal => 3,
        }
    }
}

/// Position of the revoking card within the current trick
///
/// The lead (first card) cannot revoke; these variants represent the
/// subsequent seats in playing order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RevokePosition {
    /// Second card of the trick (index 1)
    Second,
    /// Third card of the trick (index 2)
    Third,
    /// Fourth card of the trick (index 3)
    Fourth,
}

impl core::fmt::Display for RevokePosition {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Second => f.write_str("second"),
            Self::Third => f.write_str("third"),
            Self::Fourth => f.write_str("fourth"),
        }
    }
}

/// Error returned when constructing a [`Board`] with invalid invariants
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BoardError {
    /// A card on the table is still present in one of the remaining hands
    #[error("A played card is also present in a remaining hand")]
    PlayedCardInHand,
    /// The remaining hand sizes do not match the number of played cards
    ///
    /// With `k` cards on the table, exactly the `k` seats starting from
    /// `leader` (in playing order) must have one fewer card than the other
    /// seats; all other seats must share a common size.
    #[error(
        "Remaining hand sizes do not match the played-count pattern \
         (the k seats from leader must have size m-1; others m)"
    )]
    InconsistentHandSizes,
    /// A played card does not follow suit though the player held the led suit
    #[error("Played card at {position} position is a revoke — player held the led suit")]
    Revoke {
        /// Position of the revoking card within the current trick
        position: RevokePosition,
    },
}

/// Error returned when pushing cards to a [`CurrentTrick`]
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CurrentTrickError {
    /// More than three cards are on the table
    #[error("A trick can hold at most 3 cards on the table before it completes")]
    TooManyPlayed,
    /// The same card appears twice among the played cards
    #[error("Duplicate card in the played cards on the table")]
    DuplicatePlayedCard,
}

/// Trick-in-progress — 0 to 3 cards played, in playing order
///
/// Cards are played by the seats starting at [`leader`](Self::leader) in playing
/// order: the first card by `leader`, the second by `leader.lho()`, and so on.
///
/// # Invariants
///
/// 1. At most 3 cards are stored (enforced by the backing `ArrayVec<Card, 3>`).
/// 2. The stored cards are pairwise distinct.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CurrentTrick {
    trump: Strain,
    leader: Seat,
    cards: ArrayVec<Card, 3>,
    seen: Hand,
}

impl CurrentTrick {
    /// Empty trick led by `leader` under `trump`
    #[must_use]
    #[inline]
    pub const fn new(trump: Strain, leader: Seat) -> Self {
        Self {
            trump,
            leader,
            cards: ArrayVec::new_const(),
            seen: Hand::EMPTY,
        }
    }

    /// Build from a slice, validating the 0–3-card length and pairwise
    /// disjointness invariants.
    ///
    /// # Errors
    ///
    /// Returns a [`CurrentTrickError`] if the slice has more than 3 entries or
    /// contains a duplicate card.
    pub fn from_slice(
        trump: Strain,
        leader: Seat,
        played: &[Card],
    ) -> Result<Self, CurrentTrickError> {
        let mut trick = Self::new(trump, leader);
        for &card in played {
            trick.try_push(card)?;
        }
        Ok(trick)
    }

    /// Append one card to the trick.
    ///
    /// # Errors
    ///
    /// Returns [`CurrentTrickError::TooManyPlayed`] if the trick already holds
    /// 3 cards, or [`CurrentTrickError::DuplicatePlayedCard`] if `card` is
    /// already in the trick.
    pub fn try_push(&mut self, card: Card) -> Result<(), CurrentTrickError> {
        if self.cards.is_full() {
            return Err(CurrentTrickError::TooManyPlayed);
        }
        if !self.seen.insert(card) {
            return Err(CurrentTrickError::DuplicatePlayedCard);
        }
        self.cards.push(card);
        Ok(())
    }

    /// Strain of the contract governing this trick
    #[must_use]
    #[inline]
    pub const fn trump(&self) -> Strain {
        self.trump
    }

    /// Seat that led this trick
    #[must_use]
    #[inline]
    pub const fn leader(&self) -> Seat {
        self.leader
    }

    /// Cards played so far, in playing order
    #[must_use]
    #[inline]
    pub fn cards(&self) -> &[Card] {
        &self.cards
    }

    /// Number of cards played so far (0 to 3)
    #[must_use]
    #[inline]
    pub const fn len(&self) -> usize {
        self.cards.len()
    }

    /// Whether no cards have been played yet
    #[must_use]
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }

    /// Bitmask union of the cards played so far
    #[must_use]
    #[inline]
    pub const fn seen(&self) -> Hand {
        self.seen
    }

    /// Suit led this trick, or `None` if no card has been played yet
    #[must_use]
    #[inline]
    pub fn led_suit(&self) -> Option<Suit> {
        self.cards.first().map(|c| c.suit)
    }
}

/// A snapshot of a board
///
/// Construct via [`Board::try_new`], which handles both start-of-trick
/// (use [`CurrentTrick::new`]) and mid-trick (0–3 played cards) cases.  The
/// invariants below are enforced by the constructor.
///
/// # Invariants
///
/// 1. `remaining` is a valid [`PartialDeal`] (≤13 cards per hand, pairwise
///    disjoint).
/// 2. Each card in the current trick is absent from every remaining hand (the
///    "already played" invariant).
/// 3. **Uniform-size-after-restoration**: putting the
///    `k = current_trick.len()` table cards back into their players' hands
///    yields a subset where all four hands share a common size `m`.
///    Equivalently, the `k` seats starting at `current_trick.leader()` (in
///    playing order: `leader`, `leader.lho()`, …) have size `m − 1` and the
///    remaining `4 − k` seats have size `m`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Board {
    current_trick: CurrentTrick,
    remaining: PartialDeal,
}

impl Board {
    /// Construct a mid-trick board from a pre-validated [`CurrentTrick`] and
    /// the cards remaining in each hand.
    ///
    /// # Errors
    ///
    /// Returns a [`BoardError`] if the invariants documented on [`Board`] do
    /// not hold.
    pub fn try_new(
        remaining: PartialDeal,
        current_trick: CurrentTrick,
    ) -> Result<Self, BoardError> {
        if !(current_trick.seen() & remaining.collected()).is_empty() {
            return Err(BoardError::PlayedCardInHand);
        }

        let leader = current_trick.leader();
        let seats = [leader, leader.lho(), leader.partner(), leader.rho()];
        let index = current_trick.len();
        // Leader's RHO has not yet played this trick, so its hand length is the
        // common "full" length we expect.
        let full_len = remaining[leader.rho()].len();
        for (j, &seat) in seats.iter().enumerate() {
            if remaining[seat].len() + usize::from(j < index) != full_len {
                return Err(BoardError::InconsistentHandSizes);
            }
        }

        if let Some(led_suit) = current_trick.led_suit() {
            for (j, played_card) in current_trick.cards().iter().enumerate().skip(1) {
                if played_card.suit != led_suit && !remaining[seats[j]][led_suit].is_empty() {
                    return Err(BoardError::Revoke {
                        position: match j {
                            1 => RevokePosition::Second,
                            2 => RevokePosition::Third,
                            _ => RevokePosition::Fourth,
                        },
                    });
                }
            }
        }

        Ok(Self {
            current_trick,
            remaining,
        })
    }

    /// Strain of the contract
    #[must_use]
    #[inline]
    pub const fn trump(&self) -> Strain {
        self.current_trick.trump()
    }

    /// Seat leading the current trick
    #[must_use]
    #[inline]
    pub const fn leader(&self) -> Seat {
        self.current_trick.leader()
    }

    /// Cards already played to the current trick, in playing order
    #[must_use]
    #[inline]
    pub fn current_cards(&self) -> &[Card] {
        self.current_trick.cards()
    }

    /// The current trick — cards played so far plus trump and leader
    #[must_use]
    #[inline]
    pub const fn current_trick(&self) -> &CurrentTrick {
        &self.current_trick
    }

    /// Remaining cards in each hand
    #[must_use]
    #[inline]
    pub const fn remaining(&self) -> &PartialDeal {
        &self.remaining
    }
}

/// A board and its solving target
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Objective {
    /// The board to solve
    pub board: Board,
    /// The target tricks and number of solutions to find
    pub target: Target,
}

#[cfg(test)]
mod tests {
    use super::*;
    use contract_bridge::FullDeal;
    use contract_bridge::hand::Rank;

    fn card(suit: Suit, rank: u8) -> Card {
        Card {
            suit,
            rank: Rank::new(rank),
        }
    }

    /// The straight-flush deal: N spades, E hearts, S diamonds, W clubs.
    fn straight_flush_deal() -> PartialDeal {
        let deal: FullDeal = "N:AKQJT98765432... .AKQJT98765432.. \
                              ..AKQJT98765432. ...AKQJT98765432"
            .parse()
            .expect("fixture parses");
        deal.into()
    }

    /// A start-of-trick board with full hands validates.
    #[test]
    fn board_at_trick_start() {
        let board = Board::try_new(
            straight_flush_deal(),
            CurrentTrick::new(Strain::Notrump, Seat::East),
        )
        .expect("valid start-of-trick board");
        assert_eq!(board.leader(), Seat::East);
        assert!(board.current_cards().is_empty());
    }

    /// Mid-trick: the played cards must be absent from the hands and the
    /// hand sizes must follow the leader pattern.
    #[test]
    fn board_mid_trick_validation() {
        let full = straight_flush_deal();

        // East (hearts) leads the ♥A, which is still in East's hand →
        // PlayedCardInHand.
        let trick =
            CurrentTrick::from_slice(Strain::Notrump, Seat::East, &[card(Suit::Hearts, 14)])
                .expect("one-card trick");
        assert_eq!(
            Board::try_new(full, trick.clone()),
            Err(BoardError::PlayedCardInHand)
        );

        // Remove the ♥A from East: sizes now match (E has 12, others 13).
        let mut builder = contract_bridge::deal::Builder::from(full);
        builder[Seat::East].remove(card(Suit::Hearts, 14));
        let remaining: PartialDeal = builder.build_partial().expect("valid partial deal");
        let board = Board::try_new(remaining, trick).expect("valid mid-trick board");
        assert_eq!(board.current_cards().len(), 1);

        // Same cards but claiming South led → InconsistentHandSizes.
        let wrong_leader =
            CurrentTrick::from_slice(Strain::Notrump, Seat::South, &[card(Suit::Hearts, 14)])
                .expect("one-card trick");
        assert_eq!(
            Board::try_new(remaining, wrong_leader),
            Err(BoardError::InconsistentHandSizes)
        );

        // Revoke check: N still holds a club when failing to follow W's
        // club lead. 2-card ending — N: ♠A ♣2, E: ♥AK, S: ♦AK, W: ♣KQ.
        let mut builder = contract_bridge::deal::Builder::new();
        builder[Seat::North].insert(card(Suit::Spades, 14));
        builder[Seat::North].insert(card(Suit::Clubs, 2));
        builder[Seat::East].insert(card(Suit::Hearts, 14));
        builder[Seat::East].insert(card(Suit::Hearts, 13));
        builder[Seat::South].insert(card(Suit::Diamonds, 14));
        builder[Seat::South].insert(card(Suit::Diamonds, 13));
        builder[Seat::West].insert(card(Suit::Clubs, 13));
        builder[Seat::West].insert(card(Suit::Clubs, 12));

        // W leads ♣K, N plays ♠A while still holding the ♣2 → revoke at
        // the second position. Remove the played cards from the hands.
        builder[Seat::West].remove(card(Suit::Clubs, 13));
        builder[Seat::North].remove(card(Suit::Spades, 14));
        // Hand sizes: W 1 (led), N 1 (played), E 2, S 2 — leader pattern OK.
        let after: PartialDeal = builder.build_partial().expect("valid partial");
        let revoke = CurrentTrick::from_slice(
            Strain::Notrump,
            Seat::West,
            &[card(Suit::Clubs, 13), card(Suit::Spades, 14)],
        )
        .expect("two-card trick");
        assert_eq!(
            Board::try_new(after, revoke),
            Err(BoardError::Revoke {
                position: RevokePosition::Second
            })
        );
    }

    /// `CurrentTrick` rejects a fourth card and duplicates.
    #[test]
    fn current_trick_validation() {
        let mut trick = CurrentTrick::new(Strain::Spades, Seat::North);
        trick.try_push(card(Suit::Clubs, 2)).expect("first card");
        assert_eq!(
            trick.try_push(card(Suit::Clubs, 2)),
            Err(CurrentTrickError::DuplicatePlayedCard)
        );
        trick.try_push(card(Suit::Clubs, 3)).expect("second card");
        trick.try_push(card(Suit::Clubs, 4)).expect("third card");
        assert_eq!(
            trick.try_push(card(Suit::Clubs, 5)),
            Err(CurrentTrickError::TooManyPlayed)
        );
        assert_eq!(trick.led_suit(), Some(Suit::Clubs));
        assert_eq!(trick.len(), 3);
    }

    /// `Target` maps to the DDS target/solutions encoding.
    #[test]
    fn target_encoding() {
        assert_eq!(Target::Any(None).target(), -1);
        assert_eq!(Target::Any(Some(TrickCount::new(5))).target(), 5);
        assert_eq!(Target::Any(None).solutions(), 1);
        assert_eq!(Target::All(None).solutions(), 2);
        assert_eq!(Target::Legal.solutions(), 3);
        assert_eq!(Target::Legal.target(), -1);
    }
}
