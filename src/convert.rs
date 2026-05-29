//! Strain / suit conversion between `contract_bridge` and the DDS
//! substrate.
//!
//! `contract_bridge` numbers suits in ascending order (C=0, D=1, H=2,
//! S=3) while DDS uses descending order (S=0, H=1, D=2, C=3); notrump is
//! [`DDS_NOTRUMP`] (= 4) in DDS and `Strain::Notrump` (= 4) in
//! `contract_bridge`.
//!
//! The two-way mapping is therefore `3 - cb_index` for suits and an
//! identity at index 4 for notrump.

use crate::moves::DDS_NOTRUMP;
use contract_bridge::{Strain, Suit};

/// Convert a `contract_bridge::Suit` (C=0..S=3) to the DDS internal
/// suit index (S=0..C=3).
#[inline]
pub const fn dds_suit_from_cb(suit: Suit) -> usize {
    3 - suit as usize
}

/// Convert a `contract_bridge::Strain` to the DDS trump constant.
/// Notrump maps to [`DDS_NOTRUMP`] (= 4); suit strains follow the same
/// descending order as [`dds_suit_from_cb`].
#[inline]
pub fn dds_trump_from_strain(strain: Strain) -> i32 {
    strain
        .suit()
        .map_or(DDS_NOTRUMP, |suit| dds_suit_from_cb(suit) as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify the strain → trump mapping uses the descending suit order
    /// expected by the DDS substrate (and by `dds-bridge`'s
    /// `strain_to_sys`).
    #[test]
    fn strain_to_trump_uses_dds_descending_order() {
        assert_eq!(dds_trump_from_strain(Strain::Spades), 0);
        assert_eq!(dds_trump_from_strain(Strain::Hearts), 1);
        assert_eq!(dds_trump_from_strain(Strain::Diamonds), 2);
        assert_eq!(dds_trump_from_strain(Strain::Clubs), 3);
        assert_eq!(dds_trump_from_strain(Strain::Notrump), DDS_NOTRUMP);
    }
}
