use std::fmt;
use std::ops;
use std::str::FromStr;

use {CdError, Msf};

/// An enum that can describe any position on the disc, be it in the lead-in, program data or
/// lead-out
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub enum DiscPosition {
    /// Position within the lead-in. When the MSF reaches 99:59:74 we continue in the program area.
    ///
    /// Some discs appear to have small values (maybe starting at 00:00:00 at the very start of
    /// the lead-in? The PlayStation starts generally a few minutes in, but it could be a
    /// mechanical constraint that doesn't let the sled reach completely inwards). For these discs
    /// the last sector of the lead-in has an arbitrary value.
    ///
    /// Other discs appear to do the opposite: they make the last sector of the lead-in 99:59:74
    /// and then count back from there into the lead-in. I arbitrary selected this approach here
    /// because I thought that it made the code a bit cleaner.
    LeadIn(Msf),
    /// Position within the program area, containing an absolute MSF
    Program(Msf),
}

impl DiscPosition {
    /// A position that corresponds to a reasonable estimation of the innermost position within the
    /// lead-in. In practice the real value will vary depending on the disc *and* the drive.
    ///
    /// The value chosen here shouldn't matter too much, however if ToC emulation is used there
    /// should *at least* be `3 * (99 + 3) = 306` (~4seconds) lead-in sectors to accommodate the
    /// biggest ToC for 99 tracks.
    ///
    /// A few values taken with my PlayStation drive:
    ///
    /// - Ridge Racer revolution: ~01:04:00 to the program area
    /// - MGS1 disc 1: ~01:05:25 to the program area
    /// - Tame Impala (CD-DA): ~01:00:00 to the program area
    ///
    /// I settled on one full minute of lead-in for simplicity.
    pub const INNERMOST: DiscPosition = DiscPosition::LeadIn(Msf::T_99_00_00);

    /// Position at the start of the progam area (MSF 00:00:00)
    pub const ZERO: DiscPosition = DiscPosition::Program(Msf::ZERO);

    /// Returns true if this position is within the lead-in area
    pub fn in_lead_in(self) -> bool {
        matches!(self, DiscPosition::LeadIn(_))
    }

    /// Returns the position of the sector after `self` or `None` if we've reached 99:59:74.
    pub fn next(self) -> Option<DiscPosition> {
        let n = match self {
            DiscPosition::LeadIn(msf) => match msf.next() {
                Some(msf) => DiscPosition::LeadIn(msf),
                None => DiscPosition::Program(Msf::ZERO),
            },
            DiscPosition::Program(msf) => match msf.next() {
                Some(msf) => DiscPosition::Program(msf),
                None => return None,
            },
        };

        Some(n)
    }

    /// Computes `self - rhs`, returning `None` if overflow occurred
    pub fn checked_sub(self, rhs: Msf) -> Option<DiscPosition> {
        match self {
            DiscPosition::LeadIn(msf) => msf.checked_sub(rhs).map(DiscPosition::LeadIn),
            DiscPosition::Program(msf) => match msf.checked_sub(rhs) {
                Some(msf) => Some(DiscPosition::Program(msf)),
                None => rhs
                    .checked_sub(msf)
                    .and_then(|off| Msf::MAX.checked_sub(off))
                    .and_then(|msf| msf.next())
                    .map(DiscPosition::LeadIn),
            },
        }
    }

    /// Computes `self + rhs`, returning `None` if overflow occurred
    pub fn checked_add(self, rhs: Msf) -> Option<DiscPosition> {
        match self {
            DiscPosition::Program(msf) => msf.checked_add(rhs).map(DiscPosition::Program),
            DiscPosition::LeadIn(msf) => match msf.checked_add(rhs) {
                Some(msf) => Some(DiscPosition::LeadIn(msf)),
                None => Msf::MAX
                    .checked_sub(msf)
                    .and_then(|msf| msf.next())
                    .and_then(|off| rhs.checked_sub(off))
                    .map(DiscPosition::Program),
            },
        }
    }
}

impl fmt::Display for DiscPosition {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match self {
            DiscPosition::LeadIn(msf) => write!(fmt, "<{}", msf),
            DiscPosition::Program(msf) => write!(fmt, "+{}", msf),
        }
    }
}

impl fmt::Debug for DiscPosition {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        write!(fmt, "{}", self)
    }
}

impl ops::Sub<Msf> for DiscPosition {
    type Output = DiscPosition;

    fn sub(self, rhs: Msf) -> DiscPosition {
        self.checked_sub(rhs)
            .unwrap_or_else(|| panic!("DiscPosition subtraction overflow {} - {}", self, rhs))
    }
}

impl ops::SubAssign<Msf> for DiscPosition {
    fn sub_assign(&mut self, other: Msf) {
        *self = *self - other;
    }
}

impl ops::Add<Msf> for DiscPosition {
    type Output = DiscPosition;

    fn add(self, rhs: Msf) -> DiscPosition {
        self.checked_add(rhs)
            .unwrap_or_else(|| panic!("DiscPosition add overflow {} + {}", self, rhs))
    }
}

impl ops::AddAssign<Msf> for DiscPosition {
    fn add_assign(&mut self, other: Msf) {
        *self = *self + other;
    }
}

impl FromStr for DiscPosition {
    type Err = CdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut chars = s.chars();

        let dir = chars.next();

        let msf = chars.as_str().parse()?;

        match dir {
            Some('<') => Ok(DiscPosition::LeadIn(msf)),
            Some('+') => Ok(DiscPosition::Program(msf)),
            _ => Err(CdError::InvalidDiscPosition),
        }
    }
}

#[test]
fn disc_position_sub() {
    let to_test = &[
        ("+00:00:00", "00:00:01", Some("<99:59:74")),
        ("+00:00:00", "00:00:02", Some("<99:59:73")),
        ("+00:00:01", "00:00:01", Some("+00:00:00")),
        ("+99:59:74", "00:00:01", Some("+99:59:73")),
        ("+99:00:00", "99:59:74", Some("<99:00:01")),
        ("+00:00:00", "99:59:74", Some("<00:00:01")),
        ("<99:59:74", "99:59:74", Some("<00:00:00")),
        ("<00:00:00", "00:00:00", Some("<00:00:00")),
        ("+00:00:00", "00:00:00", Some("+00:00:00")),
        ("<99:59:73", "99:59:74", None),
        ("<00:00:00", "00:00:01", None),
    ];

    for (dp, msf, exp) in to_test.iter() {
        let dp: DiscPosition = dp.parse().unwrap();
        let msf: Msf = msf.parse().unwrap();

        println!("{} - {}", dp, msf);

        let r = dp.checked_sub(msf);

        match exp {
            Some(e) => {
                let e: DiscPosition = e.parse().unwrap();
                let r = r.unwrap();

                assert_eq!(e, r);

                // Check that the addition gets us back where we started.
                println!("{} + {}", r, msf);
                assert_eq!(r + msf, dp);
            }
            None => assert!(r.is_none()),
        }

        // Check that adding one is the same as calling next
        assert_eq!(dp.next(), dp.checked_add("00:00:01".parse().unwrap()));
    }
}

#[test]
fn disc_position_add() {
    let to_test = &[
        ("+00:00:00", "00:00:01", Some("+00:00:01")),
        ("+00:00:00", "00:00:02", Some("+00:00:02")),
        ("<99:59:74", "00:00:01", Some("+00:00:00")),
        ("<99:59:74", "00:00:02", Some("+00:00:01")),
        ("<99:59:73", "00:00:01", Some("<99:59:74")),
        ("<00:00:00", "12:34:56", Some("<12:34:56")),
        ("+00:00:00", "99:59:74", Some("+99:59:74")),
        ("<00:00:00", "99:59:74", Some("<99:59:74")),
        ("+99:59:74", "00:00:00", Some("+99:59:74")),
        ("+99:59:74", "00:00:01", None),
        ("+00:00:01", "99:59:74", None),
    ];

    for (dp, msf, exp) in to_test.iter() {
        let dp: DiscPosition = dp.parse().unwrap();
        let msf: Msf = msf.parse().unwrap();

        println!("{} + {}", dp, msf);
        let r = dp.checked_add(msf);

        match exp {
            Some(e) => {
                let e: DiscPosition = e.parse().unwrap();
                let r = r.unwrap();

                assert_eq!(e, r);

                // Check that the subtraction gets us back where we started.
                println!("{} - {}", r, msf);
                assert_eq!(r - msf, dp);
            }
            None => assert!(r.is_none()),
        }

        // Check that adding one is the same as calling next
        assert_eq!(dp.next(), dp.checked_add("00:00:01".parse().unwrap()));
    }
}

#[test]
fn disc_position_format() {
    let to_test = &[
        (DiscPosition::ZERO, "+00:00:00"),
        (DiscPosition::LeadIn(Msf::T_99_00_00), "<99:00:00"),
        (
            DiscPosition::LeadIn(Msf::T_99_00_00).next().unwrap(),
            "<99:00:01",
        ),
        (
            DiscPosition::LeadIn(Msf::from_bcd(0x99, 0x59, 0x74).unwrap()),
            "<99:59:74",
        ),
        (
            DiscPosition::LeadIn(Msf::from_bcd(0x99, 0x59, 0x74).unwrap())
                .next()
                .unwrap(),
            "+00:00:00",
        ),
        (
            DiscPosition::Program(Msf::from_bcd(0x99, 0x59, 0x74).unwrap()),
            "+99:59:74",
        ),
        (
            DiscPosition::Program(Msf::from_bcd(0x12, 0x34, 0x56).unwrap()),
            "+12:34:56",
        ),
        (DiscPosition::ZERO.next().unwrap(), "+00:00:01"),
    ];

    for (dp, s) in to_test.iter() {
        assert_eq!(dp.to_string().as_str(), *s);
    }
}

#[test]
fn disc_position_parse() {
    let to_test = &[
        "+00:00:00",
        "<00:00:00",
        "+01:00:00",
        "<01:00:00",
        "+99:59:74",
        "<99:59:74",
        "+12:34:56",
        "<12:34:56",
    ];
    for &s in to_test.iter() {
        let dp: DiscPosition = s.parse().unwrap();

        let f = dp.to_string();

        assert_eq!(s, f.as_str())
    }
}

#[test]
fn disc_position_parse_bad() {
    let to_test = &[
        "00:00:00",
        "?00:00:00",
        "+01:00",
        "+99:60:74",
        "<99:59:75",
        "<12:34:56?",
    ];
    for &s in to_test.iter() {
        println!("{}", s);
        assert!(s.parse::<DiscPosition>().is_err())
    }
}
