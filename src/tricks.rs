//! Validated trick-count newtypes and their formatting views.
//!
//! Mirrors `ddss::tricks` (the FFI reference crate) minus the FFI
//! conversions, so a `pons` migration between the two crates is a
//! near-mechanical swap: [`TrickCount`] (a number of tricks in `0..=13`),
//! [`TrickCountRow`] (one strain's per-seat counts, bit-packed), and
//! [`TrickCountTable`] (all five strains), plus hexadecimal and GIB
//! hand-record views.

use contract_bridge::Strain;
use contract_bridge::seat::Seat;

use thiserror::Error;

use core::fmt;

/// Error returned when a trick count is outside `0..=13`
///
/// Produced by both [`TrickCount::try_new`] and [`TrickCountRow::try_new`].
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq, Hash)]
#[error("trick count must be in 0..=13")]
pub struct InvalidTrickCount;

/// A number of tricks in `0..=13`
///
/// A validated newtype over `u8`, analogous to
/// [`Level`](contract_bridge::contract::Level) (1..=7) and
/// [`Rank`](contract_bridge::hand::Rank) (2..=14). Appears as the per-seat
/// value returned by [`TrickCountRow::get`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct TrickCount(u8);

impl TrickCount {
    /// Create a new trick count
    ///
    /// # Panics
    ///
    /// When `n` is outside `0..=13`. In const contexts, this is a compile-time
    /// error.
    #[must_use]
    #[inline]
    pub const fn new(n: u8) -> Self {
        match Self::try_new(n) {
            Ok(tc) => tc,
            Err(_) => panic!("trick count must be in 0..=13"),
        }
    }

    /// Try to create a new trick count
    ///
    /// # Errors
    ///
    /// When `n` is outside `0..=13`.
    #[inline]
    pub const fn try_new(n: u8) -> Result<Self, InvalidTrickCount> {
        if n > 13 {
            return Err(InvalidTrickCount);
        }
        Ok(Self(n))
    }

    /// Get the underlying `u8`
    #[must_use]
    #[inline]
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl From<TrickCount> for u8 {
    #[inline]
    fn from(tc: TrickCount) -> Self {
        tc.0
    }
}

impl From<TrickCount> for usize {
    #[inline]
    fn from(tc: TrickCount) -> Self {
        tc.0 as Self
    }
}

impl fmt::Display for TrickCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Tricks that each seat can take as declarer for a strain
///
/// Bit-packed: 4 bits per seat, indexed by [`Seat`]'s enum integer.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct TrickCountRow(u16);

impl TrickCountRow {
    /// Create a new row from the number of tricks each seat can take
    ///
    /// # Panics
    ///
    /// When any value is outside `0..=13`.  In const contexts, this is a
    /// compile-time error.
    #[must_use]
    #[inline]
    pub const fn new(n: u8, e: u8, s: u8, w: u8) -> Self {
        match Self::try_new(n, e, s, w) {
            Ok(row) => row,
            Err(_) => panic!("trick count must be in 0..=13"),
        }
    }

    /// Try to create a new row from the number of tricks each seat can take
    ///
    /// # Errors
    ///
    /// When any value is outside `0..=13`.
    #[inline]
    pub const fn try_new(n: u8, e: u8, s: u8, w: u8) -> Result<Self, InvalidTrickCount> {
        if n > 13 || e > 13 || s > 13 || w > 13 {
            return Err(InvalidTrickCount);
        }
        Ok(Self(
            (n as u16) << (4 * Seat::North as u8)
                | (e as u16) << (4 * Seat::East as u8)
                | (s as u16) << (4 * Seat::South as u8)
                | (w as u16) << (4 * Seat::West as u8),
        ))
    }

    /// Get the number of tricks a seat can take as declarer
    #[must_use]
    pub const fn get(self, seat: Seat) -> TrickCount {
        TrickCount((self.0 >> (4 * seat as u8) & 0xF) as u8)
    }

    /// Hexadecimal representation from a seat's perspective
    #[must_use]
    pub const fn hex(self, seat: Seat) -> TrickCountRowHex {
        TrickCountRowHex { row: self, seat }
    }
}

/// Hexadecimal view of a [`TrickCountRow`] from a seat's perspective
///
/// Returned by [`TrickCountRow::hex`]. Formats as four hex digits — the tricks
/// taken by the seat, its LHO, its partner, and its RHO — via the
/// [`UpperHex`](fmt::UpperHex) impl.
#[derive(Debug, Clone, Copy)]
pub struct TrickCountRowHex {
    row: TrickCountRow,
    seat: Seat,
}

impl fmt::UpperHex for TrickCountRowHex {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{:X}{:X}{:X}{:X}",
            self.row.get(self.seat).get(),
            self.row.get(self.seat.lho()).get(),
            self.row.get(self.seat.partner()).get(),
            self.row.get(self.seat.rho()).get(),
        )
    }
}

/// Tricks that each seat can take as declarer for all strains
///
/// Indexed by [`Strain`]: the row order is ascending — Clubs, Diamonds,
/// Hearts, Spades, Notrump — matching [`Strain`]'s enum integer values.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct TrickCountTable(pub [TrickCountRow; 5]);

impl core::ops::Index<Strain> for TrickCountTable {
    type Output = TrickCountRow;

    fn index(&self, strain: Strain) -> &TrickCountRow {
        &self.0[strain as usize]
    }
}

impl TrickCountTable {
    /// Hexadecimal representation from a seat's perspective
    #[must_use]
    pub const fn hex<T: AsRef<[Strain]>>(self, seat: Seat, strains: T) -> TrickCountTableHex<T> {
        TrickCountTableHex {
            table: self,
            seat,
            strains,
        }
    }

    /// GIB hand-record view of the whole table.
    ///
    /// The 20-digit double-dummy tail used by GIB deal databases (e.g.
    /// `sol100000.txt`): strains in the fixed order `NT, S, H, D, C`, declarers
    /// `E, N, W, S`, with the East/West cells stored as `13 − tricks`. Formats
    /// to 20 uppercase hex digits via the [`UpperHex`](fmt::UpperHex) impl —
    /// the inverse of [`from_gib`](Self::from_gib).
    #[must_use]
    pub const fn gib(self) -> TrickCountTableGib {
        TrickCountTableGib { table: self }
    }

    /// Parse a GIB hand-record tail (the 20 hex digits after the deal) back
    /// into a table. Inverse of [`gib`](Self::gib).
    ///
    /// # Panics
    ///
    /// If `hex` has fewer than 20 bytes, or any of the first 20 is not an
    /// ASCII hex digit.
    #[must_use]
    pub fn from_gib(hex: &[u8]) -> Self {
        let digit = |i: usize| {
            (hex[i] as char)
                .to_digit(16)
                .expect("GIB tail digit must be hex") as u8
        };
        let mut rows = [TrickCountRow::new(0, 0, 0, 0); 5];
        for (s, &strain) in GIB_STRAINS.iter().enumerate() {
            // GIB declarer order within a strain is E, N, W, S; E/W store 13−tricks.
            let undo = |seat: Seat, raw: u8| match seat {
                Seat::East | Seat::West => 13 - raw,
                _ => raw,
            };
            let e = undo(Seat::East, digit(4 * s));
            let n = undo(Seat::North, digit(4 * s + 1));
            let w = undo(Seat::West, digit(4 * s + 2));
            let south = undo(Seat::South, digit(4 * s + 3));
            // `Index<Strain>` reads `self.0[strain as usize]`; assign to match.
            rows[strain as usize] = TrickCountRow::new(n, e, south, w);
        }
        Self(rows)
    }
}

/// Strains in GIB hand-record order.
const GIB_STRAINS: [Strain; 5] = [
    Strain::Notrump,
    Strain::Spades,
    Strain::Hearts,
    Strain::Diamonds,
    Strain::Clubs,
];

/// Declarers in GIB hand-record order (within each strain).
const GIB_SEATS: [Seat; 4] = [Seat::East, Seat::North, Seat::West, Seat::South];

/// GIB hand-record view of a [`TrickCountTable`]
///
/// Returned by [`TrickCountTable::gib`]. See that method for the layout.
#[derive(Debug, Clone, Copy)]
pub struct TrickCountTableGib {
    table: TrickCountTable,
}

impl fmt::UpperHex for TrickCountTableGib {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for &strain in &GIB_STRAINS {
            let row = self.table[strain];
            for &seat in &GIB_SEATS {
                let tricks = row.get(seat).get();
                let stored = match seat {
                    Seat::East | Seat::West => 13 - tricks,
                    _ => tricks,
                };
                write!(f, "{stored:X}")?;
            }
        }
        Ok(())
    }
}

/// Hexadecimal view of a [`TrickCountTable`] from a seat's perspective
///
/// Returned by [`TrickCountTable::hex`]. Formats as one [`TrickCountRowHex`]
/// per strain in the supplied slice, concatenated, via the
/// [`UpperHex`](fmt::UpperHex) impl.
#[derive(Debug, Clone, Copy)]
pub struct TrickCountTableHex<T: AsRef<[Strain]>> {
    table: TrickCountTable,
    seat: Seat,
    strains: T,
}

impl<T: AsRef<[Strain]>> fmt::UpperHex for TrickCountTableHex<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for &strain in self.strains.as_ref() {
            self.table[strain].hex(self.seat).fmt(f)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `from_gib` and `gib` are exact inverses, and `gib` reproduces a stored
    /// `sol100000.txt` tail (the first DB line) byte-for-byte. The E/W `13−tricks`
    /// complement and the `NT,S,H,D,C` × `E,N,W,S` order are both exercised.
    #[test]
    fn gib_round_trip() {
        const TAIL: &str = "65658888888843433232";
        let table = TrickCountTable::from_gib(TAIL.as_bytes());
        assert_eq!(format!("{:X}", table.gib()), TAIL);

        // Spot-check decoded values: E/W make 10 clubs (their 10-card fit minus
        // K-Q), N/S make 8 spades; the E/W club cell survives the complement.
        assert_eq!(table[Strain::Clubs].get(Seat::East).get(), 10);
        assert_eq!(table[Strain::Spades].get(Seat::North).get(), 8);
    }

    /// Row packing: `new` stores per-seat nibbles retrievable by `get`, and
    /// the `hex` view walks seat → LHO → partner → RHO from the viewpoint.
    #[test]
    fn row_packing_and_hex() {
        let row = TrickCountRow::new(13, 0, 12, 1);
        assert_eq!(row.get(Seat::North).get(), 13);
        assert_eq!(row.get(Seat::East).get(), 0);
        assert_eq!(row.get(Seat::South).get(), 12);
        assert_eq!(row.get(Seat::West).get(), 1);
        // From North: N, E (LHO), S (partner), W (RHO).
        assert_eq!(format!("{:X}", row.hex(Seat::North)), "D0C1");
        // From East: E, S, W, N.
        assert_eq!(format!("{:X}", row.hex(Seat::East)), "0C1D");
    }

    /// Constructors reject out-of-range counts.
    #[test]
    fn validation() {
        assert_eq!(TrickCount::try_new(14), Err(InvalidTrickCount));
        assert_eq!(TrickCount::try_new(13).map(TrickCount::get), Ok(13));
        assert!(TrickCountRow::try_new(0, 14, 0, 0).is_err());
    }
}
