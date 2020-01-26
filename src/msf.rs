//! Compact discs were originally meant for storing music so positions
//! on the disc are stored in "minute:second:frame" format, where
//! frame means sector.
//!
//! There are 75 frames/sectors in a second, 60 seconds in a
//! minute. All three components are stored as BCD.

use std::str::FromStr;
use std::{cmp, fmt, ops};

use super::bcd::Bcd;

/// CD "minute:second:frame" timestamp, given as triplet of *BCD*
/// encoded bytes. In this context "frame" is synonymous with
/// "sector".
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Msf(Bcd, Bcd, Bcd);

impl Msf {
    /// Create a 00:00:00 MSF
    pub fn zero() -> Msf {
        Msf(Bcd::zero(), Bcd::zero(), Bcd::zero())
    }

    /// Build an MSF from a BCD triplet. Returns `None` if `s` is
    /// greater than 0x59 or if `f` is greater than 0x74.
    pub fn new(m: Bcd, s: Bcd, f: Bcd) -> Option<Msf> {
        // Make sure the frame and seconds makes sense (there are only
        // 75 frames per second and obviously 60 seconds per minute)
        if s.bcd() < 0x60 || f.bcd() < 0x75 {
            Some(Msf(m, s, f))
        } else {
            None
        }
    }

    /// Convenience function to build an MSF from BCD values stored in
    /// an `u8`. Returns none if one of the values is not valid BCD of
    /// if it's not a valid Msf
    pub fn from_bcd(m: u8, s: u8, f: u8) -> Option<Msf> {
        let m = match Bcd::from_bcd(m) {
            Some(b) => b,
            None => return None,
        };

        let s = match Bcd::from_bcd(s) {
            Some(b) => b,
            None => return None,
        };

        let f = match Bcd::from_bcd(f) {
            Some(b) => b,
            None => return None,
        };

        Msf::new(m, s, f)
    }

    /// Return the internal BCD triplet
    pub fn into_bcd(self) -> (Bcd, Bcd, Bcd) {
        (self.0, self.1, self.2)
    }

    /// Convert an MSF into a sector index. In this convention sector
    /// index 0 is MSF 00:00:00
    pub fn sector_index(self) -> u32 {
        let Msf(m, s, f) = self;

        let m = m.binary() as u32;
        let s = s.binary() as u32;
        let f = f.binary() as u32;

        // 60 seconds in a minute, 75 sectors(frames) in a second
        (60 * 75 * m) + (75 * s) + f
    }

    /// Build an MSF from a sector index. Returns None if the index is
    /// out of range.
    pub fn from_sector_index(si: u32) -> Option<Msf> {
        let m = si / (60 * 75);

        if m > 99 {
            return None;
        }

        let si = si % (60 * 75);

        let s = si / 75;
        let f = si % 75;

        let m = Bcd::from_binary(m as u8).unwrap();
        let s = Bcd::from_binary(s as u8).unwrap();
        let f = Bcd::from_binary(f as u8).unwrap();

        Some(Msf::new(m, s, f).unwrap())
    }

    /// Return the MSF timestamp of the next sector. Returns `None` if
    /// the MSF is 99:59:74.
    pub fn next(self) -> Option<Msf> {
        let Msf(m, s, f) = self;

        if f.bcd() < 0x74 {
            return Some(Msf(m, s, f.wrapping_next()));
        }

        if s.bcd() < 0x59 {
            return Some(Msf(m, s.wrapping_next(), Bcd::zero()));
        }

        if m.bcd() < 0x99 {
            return Some(Msf(m.wrapping_next(), Bcd::zero(), Bcd::zero()));
        }

        None
    }

    /// Checked MSF addition. Computes `self + other`, returning
    /// `None` if overflow occurred.
    pub fn checked_add(self, other: Msf) -> Option<Msf> {
        let a = self.sector_index();
        let b = other.sector_index();

        Msf::from_sector_index(a + b)
    }

    /// Pack the Msf in a single BCD u32, makes it easier to do
    /// comparisons without having to do a full decimal conversion
    /// like `sector_index`.
    fn as_u32_bcd(self) -> u32 {
        let Msf(m, s, f) = self;

        ((m.bcd() as u32) << 16) | ((s.bcd() as u32) << 8) | (f.bcd() as u32)
    }
}

impl fmt::Display for Msf {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        let Msf(m, s, f) = *self;

        write!(fmt, "{}:{}:{}", m, s, f)
    }
}

impl cmp::PartialOrd for Msf {
    fn partial_cmp(&self, other: &Msf) -> Option<cmp::Ordering> {
        let a = self.as_u32_bcd();
        let b = other.as_u32_bcd();

        a.partial_cmp(&b)
    }
}

impl cmp::Ord for Msf {
    fn cmp(&self, other: &Msf) -> cmp::Ordering {
        let a = self.as_u32_bcd();
        let b = other.as_u32_bcd();

        a.cmp(&b)
    }
}

impl ops::Sub for Msf {
    type Output = Msf;

    fn sub(self, rhs: Msf) -> Msf {
        let a = self.sector_index();
        let b = rhs.sector_index();

        if b > a {
            panic!("MSF substraction overflow: {} - {}", self, rhs);
        }

        Msf::from_sector_index(a - b).unwrap()
    }
}

impl ops::Add for Msf {
    type Output = Msf;

    fn add(self, rhs: Msf) -> Msf {
        match self.checked_add(rhs) {
            Some(m) => m,
            None => panic!("MSF addition overflow: {} + {}", self, rhs),
        }
    }
}

impl FromStr for Msf {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut msf = [Bcd::zero(); 3];
        let mut count = 0;

        for (i, s) in s.split(':').enumerate() {
            if i >= 3 {
                return Err(());
            }

            count += 1;
            msf[i] = Bcd::from_str(s)?;
        }

        if count != 3 {
            return Err(());
        }

        Msf::new(msf[0], msf[1], msf[2]).ok_or(())
    }
}

#[cfg(test)]
mod test {
    use super::Msf;
    use bcd::Bcd;
    use std::str::FromStr;

    #[test]
    fn conversions() {
        for &(m, s, f) in &[
            (0x00, 0x00, 0x00),
            (0x01, 0x00, 0x00),
            (0x00, 0x01, 0x00),
            (0x00, 0x00, 0x01),
            (0x12, 0x34, 0x56),
            (0x99, 0x59, 0x74),
        ] {
            let m = msf(m, s, f);

            assert!(m == Msf::from_sector_index(m.sector_index()).unwrap());
        }
    }

    #[test]
    fn substractions() {
        let m = msf(0x12, 0x34, 0x56);
        let n = msf(0x00, 0x00, 0x02);

        assert!(m - n == msf(0x12, 0x34, 0x54));

        let m = msf(0x12, 0x34, 0x02);
        let n = msf(0x00, 0x00, 0x02);

        assert!(m - n == msf(0x12, 0x34, 0x00));

        let m = msf(0x12, 0x34, 0x01);
        let n = msf(0x00, 0x00, 0x02);

        assert!(m - n == msf(0x12, 0x33, 0x74));

        let m = msf(0x12, 0x34, 0x01);
        let n = msf(0x00, 0x52, 0x10);

        assert!(m - n == msf(0x11, 0x41, 0x66));
    }

    #[test]
    fn from_str() {
        assert!(Msf::from_str("00:00:00") == Ok(msf(0x00, 0x00, 0x00)));
        assert!(Msf::from_str("01:02:03") == Ok(msf(0x01, 0x02, 0x03)));
        assert!(Msf::from_str("11:12:13") == Ok(msf(0x11, 0x12, 0x13)));
        assert!(Msf::from_str("99:59:74") == Ok(msf(0x99, 0x59, 0x74)));

        assert!(Msf::from_str("00") == Err(()));
        assert!(Msf::from_str("00:00") == Err(()));
        assert!(Msf::from_str("00:00:00:00") == Err(()));

        assert!(Msf::from_str("99:99:99") == Err(()));
    }

    fn msf(m: u8, s: u8, f: u8) -> Msf {
        Msf::new(
            Bcd::from_bcd(m).unwrap(),
            Bcd::from_bcd(s).unwrap(),
            Bcd::from_bcd(f).unwrap(),
        )
        .unwrap()
    }
}
