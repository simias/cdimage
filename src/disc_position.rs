//! This module deals with absolute positioning anywhere within the Lead-In or Program area
//! (including lead-out) of the dics

use std::cmp;
use std::fmt;
use std::ops;
use std::str::FromStr;

use crate::{CdError, CdResult, Msf};

/// An enum that can describe any position on the disc, be it in the lead-in, program data or
/// lead-out
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
    /// Originally I used 99:00:00 because during my tests on the PlayStation I seemed to always
    /// start roughly 1 minute before the start of track 01 but then I decided to base myself on
    /// the CD standard which states that the lead-in must start at a maximum radius of 23mm and
    /// the program area must start at a maximum radius of 25mm. With a pitch of 1.6µm that gives
    /// us a little over 2 and a half minutes of lead-in.
    pub const INNERMOST: DiscPosition = DiscPosition::LeadIn(DiscPosition::INNERMOST_MSF);

    /// See DiscPosition::INNERMOST's docs
    pub const INNERMOST_MSF: Msf = Msf::T_97_30_00;

    /// Position at the start of the program area (MSF 00:00:00)
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

    /// Returns the approximate track length in mm from the start of the lead-in to this position,
    /// assuming a standard 16mm per sector
    pub fn track_length_mm(self) -> CdResult<u32> {
        if self < DiscPosition::INNERMOST {
            return Err(CdError::PreLeadInPosition);
        }

        let nsectors = match self {
            DiscPosition::LeadIn(msf) => {
                if msf < DiscPosition::INNERMOST_MSF {
                    return Err(CdError::PreLeadInPosition);
                }

                msf.sector_index() - DiscPosition::INNERMOST_MSF.sector_index()
            }
            DiscPosition::Program(msf) => {
                // Lead-in sectors
                let lio = Msf::MAX.sector_index() + 1 - DiscPosition::INNERMOST_MSF.sector_index();

                lio + msf.sector_index()
            }
        };

        Ok(nsectors * CD_FRAME_LENGTH_MM)
    }

    /// Returns the approximate disc position for the given radius from the center of the disc.
    pub fn from_radius(r: Radius) -> CdResult<DiscPosition> {
        let r0 = CD_LEAD_IN_RADIUS.to_millis();
        let r1 = r.to_millis();
        let thickness = CD_PITCH_MM;

        if r > CD_PROGRAM_RADIUS_MAX {
            return Err(CdError::OutOfDiscPosition);
        }

        if r0 > r1 {
            return Err(CdError::PreLeadInPosition);
        }

        let turns = (r1 - r0) / thickness;

        DiscPosition::from_turns(turns)
    }

    /// Returns the approximate disc position for the given amount of turns from the start of the
    /// lead-in
    pub fn from_turns(turns: f32) -> CdResult<DiscPosition> {
        use std::f32::consts::PI;
        let r0 = CD_LEAD_IN_RADIUS.to_millis();
        let thickness = CD_PITCH_MM;

        // Where does this come from? We approximate the spiral as a series of circles and we sum
        // the radiuses from r0 to r1, increasing by thickness every time. If you reduce the
        // equation, you end up with the following:
        let l = PI * turns * (r0 * 2. + thickness * (turns - 1.));

        let nsectors = l / (CD_FRAME_LENGTH_MM as f32);

        let msf = match Msf::from_sector_index(nsectors.round() as u32) {
            Some(msf) => msf,
            None => return Err(CdError::OutOfDiscPosition),
        };

        DiscPosition::INNERMOST
            .checked_add(msf)
            .ok_or(CdError::OutOfDiscPosition)
    }

    /// Approximate number of rotations required to go from the beginning of the lead-in to the current
    /// position, assuming a standard CD pitch of 1.6µm
    pub fn disc_turns(self) -> CdResult<f32> {
        // I use an approximative formula where the spiral is considered to be a succession of
        // circles since it makes the maths simpler. I suspect (although I haven't checked) that
        // whatever imprecision this introduces is dwarfed by the mechanical tolerances of typical
        // CDs.
        //
        // We basically start with the equation from `from_turns`:
        //
        //   l = PI * turns * (r0 * 2. + thickness * (turns - 1.))
        //
        // Then we solve for `turns` which gives us the quadratic equation:
        //
        //   PI * thickness * turn * turn + 2. * PI * (r0 - thickness / 2) * turn - l = 0
        //
        // Solving this equation gives us the formula below
        use std::f32::consts::PI;

        let thickness = CD_PITCH_MM;
        let r0 = CD_LEAD_IN_RADIUS.to_millis();
        let l = self.track_length_mm()? as f32;

        let b = r0 - thickness / 2.;
        let b2 = b * b;

        let turns = ((thickness / 2. - r0) + (b2 + l * (thickness / PI)).sqrt()) / thickness;

        Ok(turns)
    }

    /// Offset the current position by the given number of `turns` of the spiral. Returns an error
    /// if the resulting position is out of range
    pub fn offset_turns(self, turn_offset: i32) -> CdResult<DiscPosition> {
        let turns = self.disc_turns()? + (turn_offset as f32);

        DiscPosition::from_turns(turns)
    }

    /// Returns an approximate radius from the center of the disc to the current position, assuming
    /// a standard CD pitch of 1.6µm
    pub fn disc_radius(self) -> CdResult<Radius> {
        self.disc_turns().map(|t| {
            let r0 = CD_LEAD_IN_RADIUS.to_millis();
            let thickness = CD_PITCH_MM;

            Radius::from_millis(r0 + t * thickness)
        })
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

impl cmp::Ord for DiscPosition {
    fn cmp(&self, other: &DiscPosition) -> cmp::Ordering {
        use DiscPosition::*;

        match (self, other) {
            (LeadIn(_), Program(_)) => cmp::Ordering::Less,
            (Program(_), LeadIn(_)) => cmp::Ordering::Greater,
            (LeadIn(a), LeadIn(b)) => a.cmp(b),
            (Program(a), Program(b)) => a.cmp(b),
        }
    }
}
impl cmp::PartialOrd for DiscPosition {
    fn partial_cmp(&self, other: &DiscPosition) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
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

/// A radius from the center of the CD, micrometer precision
#[derive(PartialEq, Eq, Copy, Clone, PartialOrd, Ord)]
pub struct Radius(u16);

impl Radius {
    /// Create a Radius for the given distance in micrometers
    pub const fn from_micros(micros: u16) -> Radius {
        Radius(micros)
    }

    /// Returns the radius in millimeters
    pub fn to_millis(self) -> f32 {
        f32::from(self.0) / 1000.
    }

    /// Create a Radius for the given distance in millimeters
    pub fn from_millis(millis: f32) -> Radius {
        Radius((millis * 1000.).round() as u16)
    }
}

impl fmt::Display for Radius {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        write!(fmt, "{}.{:03}mm", self.0 / 1000, self.0 % 1000)
    }
}

impl fmt::Debug for Radius {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        write!(fmt, "{}", self)
    }
}

/// Standard CD track pitch in millimeters
pub const CD_PITCH_MM: f32 = 0.0016;

/// Standard CD inner lead-in radius (maximum radius for the start of the lead-in)
pub const CD_LEAD_IN_RADIUS: Radius = Radius::from_micros(23_000);

/// Standard CD maximum radius of the program area
pub const CD_PROGRAM_RADIUS_MAX: Radius = Radius::from_micros(59_000);

/// Length of a frame in mm. 16mm Assuming a standard scanning speed of 1.2m/s
pub const CD_FRAME_LENGTH_MM: u32 = 16;

#[test]
fn test_disc_turns() {
    use std::f32::consts::PI;

    // Sectors per turn at the start of the lead-in, approximated as a circle
    let sectors_per_turn = (2. * PI * CD_LEAD_IN_RADIUS.to_millis()) / (CD_FRAME_LENGTH_MM as f32);

    let sectors_per_turn = sectors_per_turn.round() as u32;

    let p = DiscPosition::INNERMOST;
    // If we don't make too many turns the radius increase will be negligible, so the linear
    // distance for every round should be roughly the same and we can check that it increases
    // linearly
    for i in 0..30 {
        let p = p + Msf::from_sector_index(sectors_per_turn * i).unwrap();

        assert_eq!(p.disc_turns().unwrap().round() as u32, i);
    }
}

#[test]
fn test_disc_radius() {
    // The standard states that the lead-in must start at a maximum radius of 23mm
    let dp = DiscPosition::INNERMOST;
    let r = dp.disc_radius().unwrap();
    assert_eq!(r, Radius::from_micros(23_000));

    // The standard states that the program must start at a maximum radius of 25mm so our
    // approximation here seems reasonable.
    let dp: DiscPosition = "+00:00:00".parse().unwrap();
    let r = dp.disc_radius().unwrap();
    assert_eq!(r, Radius::from_micros(24_913));

    let dp: DiscPosition = "+26:42:29".parse().unwrap();
    let r = dp.disc_radius().unwrap();
    assert_eq!(r, Radius::from_micros(40_000));

    // A very rough measurement using Legend of Legaia in the lead-out gave me about 50.5mm for the
    // sector at 51:40:20. The estimated value of 50.1 millimeters seems slightly too low but
    // probably reasonable given the amount of approximation in these calculations and the
    // variation between discs.
    let dp: DiscPosition = "+51:40:20".parse().unwrap();
    let r = dp.disc_radius().unwrap();
    assert_eq!(r, Radius::from_micros(50_154));

    // A standard CD can store roughly 74 minutes of content, and the maximum radius for the
    // program area is 59mm
    let dp: DiscPosition = "+74:00:00".parse().unwrap();
    let r = dp.disc_radius().unwrap();
    assert_eq!(r, Radius::from_micros(57_743));
}

#[test]
fn disc_position_from_radius() {
    let to_test = &[
        (CD_LEAD_IN_RADIUS, "<97:30:00"),
        (Radius::from_micros(24_916), "+00:00:16"),
        (Radius::from_micros(25_000), "+00:07:06"),
        (Radius::from_micros(40_000), "+26:42:28"),
        (Radius::from_micros(59_000), "+78:00:08"),
    ];

    for &(r, dp) in to_test {
        let expected: DiscPosition = dp.parse().unwrap();
        assert_eq!(DiscPosition::from_radius(r).unwrap(), expected);

        // Make sure the backward conversion takes us back where we started, with some rounding to
        // account for floating point precision issues.
        assert_eq!(
            (expected.disc_radius().unwrap().to_millis() * 10.).round(),
            (r.to_millis() * 10.).round()
        );
    }
}

#[test]
fn radius_to_string() {
    assert_eq!(Radius(0).to_string().as_str(), "0.000mm");
    assert_eq!(Radius(50_000).to_string().as_str(), "50.000mm");
    assert_eq!(Radius(12_345).to_string().as_str(), "12.345mm");
    assert_eq!(Radius(10_001).to_string().as_str(), "10.001mm");
    assert_eq!(Radius(1).to_string().as_str(), "0.001mm");
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
        (DiscPosition::LeadIn(Msf::T_97_30_00), "<97:30:00"),
        (
            DiscPosition::LeadIn(Msf::T_97_30_00).next().unwrap(),
            "<97:30:01",
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
