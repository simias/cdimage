//! The CD format uses binary-coded decimal (BCD) extensively in its
//! internal format (track numbers, seek positions etc...) probably in
//! order to make it easier to display those informations on the first
//! CD players.

use std::fmt;
use std::str::FromStr;

use CdError;

/// A single packed BCD value in the range 0-99 (2 digits, 4bits per digit).
#[derive(Copy, Hash, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Bcd(u8);

impl Bcd {
    /// A Bcd instance with value 0
    pub const ZERO: Bcd = Self::TABLE[0];
    /// A Bcd instance with value 1
    pub const ONE: Bcd = Self::TABLE[1];
    /// A Bcd instance with value 99
    pub const MAX: Bcd = Self::TABLE[99];

    /// Build a `Bcd` from an `u8` in BCD format. Returns `None` if the value provided is not valid
    /// BCD.
    pub const fn from_bcd(b: u8) -> Option<Bcd> {
        if b <= 0x99 && (b & 0xf) <= 0x9 {
            Some(Bcd(b))
        } else {
            None
        }
    }

    /// Build a `Bcd` from a binary `u8`. Returns `None` if the value is greater than 0x99.
    pub const fn from_binary(b: u8) -> Option<Bcd> {
        let pos = b as usize;

        if pos < Self::TABLE.len() {
            Some(Self::TABLE[pos])
        } else {
            None
        }
    }

    /// Returns the BCD as an u8
    pub const fn bcd(self) -> u8 {
        self.0
    }

    /// Convert the BCD as a binary byte
    pub const fn binary(self) -> u8 {
        let b = self.0;

        (b >> 4) * 10 + (b & 0xf)
    }

    /// Returns the BCD value plus one. Wrap to 0 if `self` is equal to 99.
    pub const fn wrapping_next(self) -> Bcd {
        let b = self.bcd();

        if b & 0xf < 9 {
            Bcd(b + 1)
        } else if b < 0x99 {
            Bcd((b & 0xf0) + 0x10)
        } else {
            Bcd(0)
        }
    }

    /// BCD lookup table.
    ///
    /// May help a bit with performance but is mainly here to avoid sprinkling `unwraps` every time one
    /// needs a literal BCD value in the code. So instead of doing `Bcd::from_binary(59).unwrap()` you can do
    /// `BCD_TABLE[59]` and you'll get a compilation error if it's out of range.
    pub const TABLE: [Bcd; 100] = [
        Bcd(0x00),
        Bcd(0x01),
        Bcd(0x02),
        Bcd(0x03),
        Bcd(0x04),
        Bcd(0x05),
        Bcd(0x06),
        Bcd(0x07),
        Bcd(0x08),
        Bcd(0x09),
        Bcd(0x10),
        Bcd(0x11),
        Bcd(0x12),
        Bcd(0x13),
        Bcd(0x14),
        Bcd(0x15),
        Bcd(0x16),
        Bcd(0x17),
        Bcd(0x18),
        Bcd(0x19),
        Bcd(0x20),
        Bcd(0x21),
        Bcd(0x22),
        Bcd(0x23),
        Bcd(0x24),
        Bcd(0x25),
        Bcd(0x26),
        Bcd(0x27),
        Bcd(0x28),
        Bcd(0x29),
        Bcd(0x30),
        Bcd(0x31),
        Bcd(0x32),
        Bcd(0x33),
        Bcd(0x34),
        Bcd(0x35),
        Bcd(0x36),
        Bcd(0x37),
        Bcd(0x38),
        Bcd(0x39),
        Bcd(0x40),
        Bcd(0x41),
        Bcd(0x42),
        Bcd(0x43),
        Bcd(0x44),
        Bcd(0x45),
        Bcd(0x46),
        Bcd(0x47),
        Bcd(0x48),
        Bcd(0x49),
        Bcd(0x50),
        Bcd(0x51),
        Bcd(0x52),
        Bcd(0x53),
        Bcd(0x54),
        Bcd(0x55),
        Bcd(0x56),
        Bcd(0x57),
        Bcd(0x58),
        Bcd(0x59),
        Bcd(0x60),
        Bcd(0x61),
        Bcd(0x62),
        Bcd(0x63),
        Bcd(0x64),
        Bcd(0x65),
        Bcd(0x66),
        Bcd(0x67),
        Bcd(0x68),
        Bcd(0x69),
        Bcd(0x70),
        Bcd(0x71),
        Bcd(0x72),
        Bcd(0x73),
        Bcd(0x74),
        Bcd(0x75),
        Bcd(0x76),
        Bcd(0x77),
        Bcd(0x78),
        Bcd(0x79),
        Bcd(0x80),
        Bcd(0x81),
        Bcd(0x82),
        Bcd(0x83),
        Bcd(0x84),
        Bcd(0x85),
        Bcd(0x86),
        Bcd(0x87),
        Bcd(0x88),
        Bcd(0x89),
        Bcd(0x90),
        Bcd(0x91),
        Bcd(0x92),
        Bcd(0x93),
        Bcd(0x94),
        Bcd(0x95),
        Bcd(0x96),
        Bcd(0x97),
        Bcd(0x98),
        Bcd(0x99),
    ];
}

impl FromStr for Bcd {
    type Err = CdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let b = match u8::from_str(s) {
            Ok(b) => b,
            Err(_) => return Err(CdError::BadBcd),
        };

        Bcd::from_binary(b).ok_or(CdError::BadBcd)
    }
}

impl fmt::Display for Bcd {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:02x}", self.0)
    }
}

impl fmt::Debug for Bcd {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self)
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
    assert!(Bcd::from_str("00").unwrap() == Bcd(0));
    assert!(Bcd::from_str("0").unwrap() == Bcd(0));
    assert!(Bcd::from_str("4").unwrap() == Bcd(4));
    assert!(Bcd::from_str("04").unwrap() == Bcd(4));
    assert!(Bcd::from_str("99").unwrap() == Bcd(0x99));
    assert!(Bcd::from_str("099").unwrap() == Bcd(0x99));
    assert!(Bcd::from_str("42").unwrap() == Bcd(0x42));

    assert!(Bcd::from_str("0x00").is_err());
    assert!(Bcd::from_str("0xab").is_err());
    assert!(Bcd::from_str("ab").is_err());
    assert!(Bcd::from_str("100").is_err());
    assert!(Bcd::from_str("0100").is_err());
    assert!(Bcd::from_str("-2").is_err());
}

#[test]
fn bcd_table() {
    for v in 0..=99 {
        let bcd = Bcd(((v / 10) << 4) | (v % 10));

        assert_eq!(bcd, Bcd::TABLE[v as usize]);
        assert_eq!(bcd, Bcd::from_binary(v).unwrap());
    }
}
