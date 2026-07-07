//! Solved-play output types
//!
//! Mirrors `ddss::play` (the FFI reference crate) minus the FFI
//! conversions, so a `pons` migration between the two crates is a
//! near-mechanical swap.

use crate::tricks::TrickCount;
use contract_bridge::hand::{Card, Holding};

use arrayvec::ArrayVec;

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
