//! Play-trace input and solved-play output types
//!
//! Mirrors `ddss::play` (the FFI reference crate) minus the FFI
//! conversions, so a `pons` migration between the two crates is a
//! near-mechanical swap.

use crate::board::Board;
use crate::tricks::TrickCount;
use contract_bridge::hand::{Card, Holding};
use contract_bridge::seat::Seat;

use arrayvec::ArrayVec;
use thiserror::Error;

/// A play and its consequences
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Play {
    /// The card to play, the highest in a sequence
    ///
    /// For example, if the solution is to play a card from ♥KQJ, this field
    /// would be ♥K.
    pub card: Card,

    /// Lower equals in the sequence
    ///
    /// Playing any card in a sequence is equal in bridge and many trick-taking
    /// games.  This field contains lower cards in the same sequence as `card`.
    /// For example, if the solution is to play KQJ, this field would contain
    /// QJ.  Ranks removed in **earlier** tricks merge the cards around them
    /// into one sequence; cards on the table of the current trick do not.
    pub equals: Holding,

    /// Tricks this play would score
    pub score: TrickCount,
}

/// Solved plays for a board
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FoundPlays {
    /// The plays and their consequences
    pub plays: ArrayVec<Play, 13>,
    /// The number of nodes searched by the solver
    ///
    /// Approximate: pons-dds counts completed-trick nodes like DDS, but
    /// its probe schedule differs, so this does not match the FFI
    /// crate's count bit-for-bit.
    pub nodes: u32,
}

/// A starting board and a sequence of cards played from it
///
/// Input to [`Solver::analyse_play`](crate::Solver::analyse_play).  The two
/// fields split the position and the play-trace cleanly:
///
/// - [`board`](Self::board) is the snapshot from which analysis begins.  It
///   encodes the state at the start of a trick — possibly with up to three
///   cards already on the table in [`Board::current_cards`] — and
///   [`Board::remaining`] holds only the cards still in each hand.  Cards
///   from **previously completed tricks are not represented individually**;
///   they are simply absent from `remaining`.
/// - [`cards`](Self::cards) is the play trace to replay from that snapshot,
///   in chronological order.  The first card in `cards` is whichever card
///   comes *after* any already in `board.current_cards` — it does **not**
///   restart the trick or repeat prior history.  Each card must be legal
///   (follow suit when possible and be held by the player on turn).
///
/// `cards` may span trick boundaries; the analyser tracks trick completion
/// and whose lead follows internally.  The trace length may be any value
/// from `0` to `52`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlayTrace {
    /// Snapshot at the start of analysis: state at the start of the current
    /// trick, plus any 0–3 cards already played to it via
    /// [`Board::current_cards`]
    pub board: Board,
    /// Cards played after `board`, in chronological order; may cross tricks
    pub cards: ArrayVec<Card, 52>,
}

/// Double-dummy trick counts before and after each played card in a trace
///
/// Returned by [`Solver::analyse_play`](crate::Solver::analyse_play).  Trick
/// counts are from the declarer's viewpoint: declarer is the right-hand
/// opponent of the opening leader (the side to lead the very first trick in
/// the starting [`Board`]).
///
/// `tricks[0]` is the DD value before any card in the trace is played.
/// `tricks[i]` for `i > 0` is the DD value after the i-th card.  A drop from
/// `tricks[i - 1]` to `tricks[i]` means that card was a double-dummy mistake
/// by the side to move at the time.
///
/// pons-dds always returns the full `cards.len() + 1` entries.  (The FFI
/// reference truncates the final trick and mis-counts entries on mid-trick
/// snapshots — its own documentation promises the full length.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayAnalysis {
    /// Trick counts — `cards.len() + 1` entries, starting with the position
    /// before any card is played
    pub tricks: ArrayVec<TrickCount, 53>,
}

/// The way a trace card is illegal
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PlayFaultKind {
    /// The player on turn does not hold the card
    NotHeld,
    /// The player failed to follow suit while holding the led suit
    ///
    /// The FFI reference does not detect revokes — it silently scores the
    /// off-suit card as a discard and produces a wrong analysis.
    Revoke,
}

/// Error returned when a [`PlayTrace`] card is illegal
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq, Hash)]
#[error("trace card #{index} ({card} by {seat}) is illegal: {kind:?}")]
pub struct PlayFaultError {
    /// Index of the offending card within [`PlayTrace::cards`]
    pub index: usize,
    /// The seat that was on turn
    pub seat: Seat,
    /// The offending card
    pub card: Card,
    /// How the card is illegal
    pub kind: PlayFaultKind,
}
