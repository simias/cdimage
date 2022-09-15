//! Generic interface to various Compact Disc (CD) image formats.
//!
//! The architecture is inspired by BizHawk's CD handling code.

#![warn(missing_docs)]

#[macro_use]
extern crate arrayref;
extern crate thiserror;

pub use bcd::Bcd;
pub use msf::Msf;
pub use sector::Sector;
use subchannel::{QData, Q};

use std::clone::Clone;
use std::fmt;
use std::io;
use std::path::PathBuf;
use thiserror::Error;

pub mod bcd;
pub mod crc;
pub mod cue;
pub mod internal;
pub mod msf;
pub mod sector;
pub mod subchannel;

#[cfg(test)]
mod tests;

/// An offset within the lead-in (counting away from the pregap of Track 01)
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct LeadInIndex(u16);

/// An enum that can describe any position on the disc, be it in the lead-in, program data or
/// lead-out
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum DiscPosition {
    /// Position within the lead-in. The integer is the number of sector until we reach the program
    /// area, so it will count down to zero and then switch to the program area.
    LeadIn(LeadInIndex),
    /// Position within the program area, containing an absolute MSF
    Program(Msf),
}

impl DiscPosition {
    /// Return a position that corresponds to a reasonable estimation of the innermost position
    /// within the lead-in. In practice the real value will depend depending on the disc *and* the
    /// drive.
    pub fn innermost() -> DiscPosition {
        // A few values taken with my PlayStation drive:
        //
        // - Ridge Racer revolution: ~4_800 sectors to the program area
        // - MGS1 disc 1: ~4_900 sectors to the program area
        // - Tame Impala (CD-DA): ~4_500 sectors to the program area
        //
        // Judging by the lead-in MSF
        DiscPosition::LeadIn(LeadInIndex(4_500))
    }

    /// Returns true if this position is within the lead-in area
    pub fn in_lead_in(self) -> bool {
        matches!(self, DiscPosition::LeadIn(_))
    }

    /// Returns the position of the sector after `self` or `None` if we've reached 99:59:74.
    pub fn next(self) -> Option<DiscPosition> {
        let n = match self {
            DiscPosition::LeadIn(LeadInIndex(0)) => DiscPosition::Program(Msf::ZERO),
            DiscPosition::LeadIn(LeadInIndex(n)) => DiscPosition::LeadIn(LeadInIndex(n - 1)),
            DiscPosition::Program(msf) => match msf.next() {
                Some(msf) => DiscPosition::Program(msf),
                None => return None,
            },
        };

        Some(n)
    }
}

/// Abstract read-only interface to an image format
pub trait Image {
    /// Return a string identifying the image format in a
    /// human-readable way. If the backend is daisy-chained it should
    /// mention the underlying image format as well.
    fn image_format(&self) -> String;

    /// Read a single sector at the given absolute MSF
    fn read_sector(&mut self, position: DiscPosition) -> CdResult<Sector>;

    /// Get the table of contents
    fn toc(&self) -> &Toc;
}

/// Struct representing a track's attributes
#[derive(Clone)]
pub struct Track {
    /// Track number
    pub track: Bcd,
    /// Track format
    pub format: TrackFormat,
    /// Absolute MSF for the first sector of the track
    pub start: Msf,
    /// Length of the track
    pub length: Msf,
}

impl Track {
    /// Return the absolute Msf for the position `track_msf` in `track`. Will return an error if
    /// the `track_msf` is outside of the track.
    pub fn absolute_msf(&self, track_msf: Msf) -> CdResult<Msf> {
        if track_msf < self.length {
            Ok(self.start + track_msf)
        } else {
            Err(CdError::EndOfTrack)
        }
    }
}

/// Table of contents
#[derive(Clone)]
pub struct Toc {
    /// Track list
    tracks: Vec<Track>,
}

impl Toc {
    fn new(tracks: Vec<Track>) -> CdResult<Toc> {
        if tracks.is_empty() {
            Err(CdError::EmptyToc)
        } else {
            Ok(Toc { tracks })
        }
    }

    /// Return the Track description for the given `track_no`. Returns an error if `track_no` is 0
    /// or greater than the total number of tracks.
    pub fn track(&self, track_no: Bcd) -> CdResult<&Track> {
        let t = track_no.binary();

        if t < 1 {
            return Err(CdError::BadTrack);
        }

        self.tracks.get((t - 1) as usize).ok_or(CdError::BadTrack)
    }

    /// Return the full track list
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// Generate a lead-in Toc sector for the given `index`.
    pub fn build_toc_sector(&self, LeadInIndex(index): LeadInIndex) -> CdResult<Sector> {
        let index = u32::from(index);

        // The absolute MSF for the lead-in is arbitrary but must be incrementing.
        //
        // Some discs appear to have small values (maybe starting at 00:00:00 at the very start of
        // the lead-in? The PlayStation generally a few minutes in, but it could be a mechanical
        // constraint).
        //
        // Other discs appear to do the opposite: they make the last sector of the lead-in 99:59:74
        // and then count back from there into the lead-in. I arbitrary selected this 2nd approach
        // here
        let lead_in_msf = {
            let end_idx = Msf::MAX.sector_index();

            // By design it should be impossible for the operations below to fail since the lead-in
            // index is an u16 and Msf::MAX is well beyond that range.
            assert!(end_idx >= index, "Invalid lead-in index");
            Msf::from_sector_index(end_idx - index).expect("Invalid lead-in MSF")
        };

        // Number of entries in the raw ToC: one per track + first track + last track + lead-in
        let ntracks = self.tracks.len() as u32;
        let nentries = ntracks + 3;

        // We divide by 3 because each entry is usually repeated 3 times in a row
        let entry_off = nentries - ((index / 3) % nentries) - 1;

        let (q, fmt) = match entry_off {
            0 => {
                let t = &self.tracks[0];

                let format = t.format;

                let qdata = QData::Mode1TocFirstTrack {
                    first_track: t.track,
                    session_format: self.session_format(),
                    lead_in_msf,
                };

                (Q::from_qdata(qdata, format), format)
            }
            1 => {
                let t = self.tracks.last().unwrap();

                let format = t.format;

                let qdata = QData::Mode1TocLastTrack {
                    last_track: t.track,
                    lead_in_msf,
                };

                (Q::from_qdata(qdata, format), format)
            }
            2 => {
                // I'm not sure what the format of these sectors should be but in practice it seems
                // to be the same type as the last sector.
                let t = self.tracks.last().unwrap();
                let format = t.format;

                let qdata = QData::Mode1TocLeadOut {
                    lead_out_start: self.lead_out_start(),
                    lead_in_msf,
                };

                (Q::from_qdata(qdata, format), format)
            }
            n => {
                let t = &self.tracks[(n - 3) as usize];
                let format = t.format;

                let qdata = QData::Mode1Toc {
                    track: t.track,
                    index1_msf: t.start,
                    lead_in_msf,
                };

                (Q::from_qdata(qdata, format), format)
            }
        };

        Sector::empty(q, fmt)
    }

    /// Returns the MSF of the first sector in the lead-out
    pub fn lead_out_start(&self) -> Msf {
        let t = self.tracks.last().unwrap();

        t.start + t.length
    }

    /// Return the session format for this ToC based on the format of its tracks
    pub fn session_format(&self) -> SessionFormat {
        for t in self.tracks.iter() {
            match t.format {
                TrackFormat::Audio => (),
                // XXX Not sure about this one
                TrackFormat::CdG => (),
                TrackFormat::Mode1 => (),
                TrackFormat::Mode2Xa => return SessionFormat::CdXa,
                TrackFormat::Mode2CdI => return SessionFormat::Cdi,
            }
        }

        // No "special" track found, it's probably a conventional CD
        SessionFormat::CdDaCdRom
    }
}

impl fmt::Debug for Toc {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        writeln!(
            fmt,
            "ToC: {} track{}, total length {}",
            self.tracks.len(),
            if self.tracks.len() == 1 { "" } else { "s" },
            self.lead_out_start()
        )?;

        for t in self.tracks.iter() {
            writeln!(
                fmt,
                " - Track {}: start {} length {} {:?}",
                t.track, t.start, t.length, t.format,
            )?;
        }

        Ok(())
    }
}

/// Possible session formats.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum SessionFormat {
    /// CD-DA (audio CD, "red book" specification) or CD-ROM ("yellow
    /// book" specification) session
    CdDaCdRom,
    /// CD-i (compact disc interactive, "green book"
    /// specification). Used on Philips' CD-i console.
    Cdi,
    /// CD-ROM XA (extended architecture). Used on Sony's PlayStation
    /// console.
    CdXa,
}

/// Possible track types
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum TrackFormat {
    /// CD-DA audio track (red book audio)
    Audio,
    /// CD-G track (CD-Graphics)
    CdG,
    /// CD-ROM Mode1 data
    Mode1,
    /// CD-ROM XA Mode 2 data
    Mode2Xa,
    /// CD-i Mode 2 data
    Mode2CdI,
}

impl TrackFormat {
    /// Return the CD-ROM mode for this track format, or `None` if this is not a CD-ROM format
    pub fn cdrom_mode(self) -> Option<sector::CdRomMode> {
        let m = match self {
            TrackFormat::Mode1 => sector::CdRomMode::Mode1,
            TrackFormat::Mode2Xa => sector::CdRomMode::Mode2,
            TrackFormat::Mode2CdI => sector::CdRomMode::Mode2,
            _ => return None,
        };

        Some(m)
    }

    /// Return true if this is a CD-ROM track
    pub fn is_cdrom(self) -> bool {
        self.cdrom_mode().is_some()
    }

    /// Returns true if this is an audio track
    pub fn is_audio(self) -> bool {
        self == TrackFormat::Audio
    }
}

/// Error type for disc operations.
#[allow(missing_docs)]
#[derive(Error, Debug)]
pub enum CdError {
    #[error("Generic I/O error")]
    IoError(#[from] io::Error),
    #[error("Format missmatch. For instance when one attempts to retrieve CD-ROM payloads on an audio track.")]
    BadFormat,
    #[error("Attempted to access a sector past the end of the CD")]
    LeadOut,
    #[error("Unexpected or corrupted image format `{path}`|{line}: {desc}")]
    ParseError {
        path: PathBuf,
        line: u32,
        desc: String,
    },
    #[error("Disc format error in file `{path}`: {desc}")]
    BadImage { path: PathBuf, desc: String },
    #[error("Attempted to access an invalid track number")]
    BadTrack,
    #[error("Attempted to access a track past its end")]
    EndOfTrack,
    #[error(
        "The sync pattern at the start of a CD-ROM sector (0x00, 0xff * 10, 0x00) was invalid"
    )]
    BadSyncPattern,
    #[error("Attempted to parse invalid BCD data")]
    BadBcd,
    #[error("Invalid Q subchannel CRC")]
    InvalidSubQCRC,
    #[error("Unsupported format")]
    Unsupported,
    #[error("Empty table of contents")]
    EmptyToc,
}

/// Convenience type alias for a `Result<R, CdError>`
pub type CdResult<R> = std::result::Result<R, CdError>;

#[test]
fn cderror_display() {
    // Make sure that CdError implements Display. This should be true if we set an
    // `#[error("...")]` for every variant
    println!("{}", CdError::BadTrack);
}
