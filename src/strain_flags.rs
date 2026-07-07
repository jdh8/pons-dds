//! Strain selection flags for batch solving
//!
//! Mirrors `ddss::StrainFlags` / `ddss::NonEmptyStrainFlags` so a `pons`
//! migration between the two crates is a near-mechanical swap.

use contract_bridge::Strain;

bitflags::bitflags! {
    /// Flags for the solver to solve for a strain
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct StrainFlags : u8 {
        /// Solve for clubs ([`Strain::Clubs`])
        const CLUBS = 0x01;
        /// Solve for diamonds ([`Strain::Diamonds`])
        const DIAMONDS = 0x02;
        /// Solve for hearts ([`Strain::Hearts`])
        const HEARTS = 0x04;
        /// Solve for spades ([`Strain::Spades`])
        const SPADES = 0x08;
        /// Solve for notrump ([`Strain::Notrump`])
        const NOTRUMP = 0x10;
    }
}

impl StrainFlags {
    /// The flag for a single strain. Flag bit values align with [`Strain`]'s
    /// enum integers (`Clubs = 0` → `0x01` … `Notrump = 4` → `0x10`).
    #[must_use]
    pub const fn from_strain(strain: Strain) -> Self {
        Self::from_bits_truncate(1 << strain as u8)
    }
}

impl From<Strain> for StrainFlags {
    #[inline]
    fn from(strain: Strain) -> Self {
        Self::from_strain(strain)
    }
}

/// A guaranteed non-empty [`StrainFlags`]
///
/// Analogous to [`NonZero`](core::num::NonZero) — constructable only if the
/// flags are non-empty, ensuring callers cannot accidentally pass an empty set
/// to functions that require at least one strain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NonEmptyStrainFlags(StrainFlags);

impl NonEmptyStrainFlags {
    /// All strains
    pub const ALL: Self = Self(StrainFlags::all());

    /// Wrap `flags` if non-empty, otherwise return `None`
    #[must_use]
    pub const fn new(flags: StrainFlags) -> Option<Self> {
        if flags.is_empty() {
            None
        } else {
            Some(Self(flags))
        }
    }

    /// Extract the inner [`StrainFlags`]
    #[must_use]
    pub const fn get(self) -> StrainFlags {
        self.0
    }
}

impl From<NonEmptyStrainFlags> for StrainFlags {
    #[inline]
    fn from(flags: NonEmptyStrainFlags) -> Self {
        flags.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Flag bits line up with `Strain` enum integers, and the non-empty
    /// wrapper rejects the empty set.
    #[test]
    fn from_strain_bit_alignment() {
        assert_eq!(StrainFlags::from_strain(Strain::Clubs), StrainFlags::CLUBS);
        assert_eq!(
            StrainFlags::from_strain(Strain::Diamonds),
            StrainFlags::DIAMONDS
        );
        assert_eq!(
            StrainFlags::from_strain(Strain::Hearts),
            StrainFlags::HEARTS
        );
        assert_eq!(
            StrainFlags::from_strain(Strain::Spades),
            StrainFlags::SPADES
        );
        assert_eq!(
            StrainFlags::from_strain(Strain::Notrump),
            StrainFlags::NOTRUMP
        );

        assert_eq!(NonEmptyStrainFlags::new(StrainFlags::empty()), None);
        assert_eq!(
            NonEmptyStrainFlags::new(StrainFlags::all()),
            Some(NonEmptyStrainFlags::ALL)
        );
    }
}
