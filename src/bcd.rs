//! The CD format uses binary-coded decimal (BCD) extensively in its
//! internal format (track numbers, seek positions etc...) probably in
//! order to make it easier to display those informations on the first
//! CD players.

use std::str::FromStr;
use std::fmt;

/// A single packed BCD value in the range 0-99 (2 digits, 4bits per
/// digit).
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord,
         RustcDecodable, RustcEncodable)]
pub struct Bcd(u8);

impl Bcd {
    /// Build a `Bcd` from an `u8` in BCD format. Returns `None` if
    /// the value provided is not valid BCD.
    pub fn from_bcd(b: u8) -> Option<Bcd> {
        if b <= 0x99 && (b & 0xf) <= 0x9 {
            Some(Bcd(b))
        } else {
            None
        }
    }

    /// Build a `Bcd` from a binary `u8`. Returns `None` if the value
    /// is greater than 0x99.
    pub fn from_binary(b: u8) -> Option<Bcd> {
        if b > 99 {
            None
        } else {
            Some(Bcd(((b / 10) << 4) | (b % 10)))
        }
    }

    /// Return a BCD equal to 0
    pub fn zero() -> Bcd {
        Bcd(0)
    }

    /// Return a BCD equal to 1
    pub fn one() -> Bcd {
        Bcd(1)
    }

    /// Returns the BCD as an u8
    pub fn bcd(self) -> u8 {
        self.0
    }

    /// Convert the BCD as a binary byte
    pub fn binary(self) -> u8 {
        let b = self.0;

        (b >> 4) * 10 + (b & 0xf)
    }

    /// Returns the BCD value plus one. Wrap to 0 if `self` is equal
    /// to 99.
    pub fn wrapping_next(self) -> Bcd {
        let b = self.bcd();

        if b & 0xf < 9 {
            Bcd(b + 1)
        } else if b < 0x99 {
            Bcd((b & 0xf0) + 0x10)
        } else {
            Bcd(0)
        }
    }
}

impl FromStr for Bcd {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let b =
            match u8::from_str(s) {
                Ok(b) => b,
                Err(_) => return Err(()),
            };

        Bcd::from_binary(b).ok_or(())
    }
}

impl fmt::Display for Bcd {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:02x}", self.0)
    }
}

#[test]
fn conversions() {
    assert!(Bcd::from_bcd(0) == Some(Bcd(0)));
    assert!(Bcd::from_bcd(1) == Some(Bcd(1)));
    assert!(Bcd::from_bcd(0x42) == Some(Bcd(0x42)));
    assert!(Bcd::from_bcd(0x1a) == None);
    assert!(Bcd::from_bcd(0xf2) == None);

    assert!(Bcd::from_binary(0) == Some(Bcd(0)));
    assert!(Bcd::from_binary(1) == Some(Bcd(1)));
    assert!(Bcd::from_binary(42) == Some(Bcd(0x42)));
    assert!(Bcd::from_binary(100) == None);
    assert!(Bcd::from_binary(0xff) == None);

}

#[test]
fn from_str() {
    assert!(Bcd::from_str("00") == Ok(Bcd(0)));
    assert!(Bcd::from_str("0") == Ok(Bcd(0)));
    assert!(Bcd::from_str("4") == Ok(Bcd(4)));
    assert!(Bcd::from_str("04") == Ok(Bcd(4)));
    assert!(Bcd::from_str("99") == Ok(Bcd(0x99)));
    assert!(Bcd::from_str("099") == Ok(Bcd(0x99)));
    assert!(Bcd::from_str("42") == Ok(Bcd(0x42)));

    assert!(Bcd::from_str("0x00") == Err(()));
    assert!(Bcd::from_str("0xab") == Err(()));
    assert!(Bcd::from_str("ab") == Err(()));
    assert!(Bcd::from_str("100") == Err(()));
    assert!(Bcd::from_str("0100") == Err(()));
    assert!(Bcd::from_str("-2") == Err(()));
}
