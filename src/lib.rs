//! Generic interface to various Compact Disc (CD) image formats.
//!
//! The architecture is inspired by BizHawk's CD handling code.

#![warn(missing_docs)]

#[macro_use]
extern crate arrayref;
#[cfg(feature = "serde")]
extern crate serde;
#[cfg(feature = "serde")]
extern crate serde_big_array;
extern crate thiserror;
extern crate zip;

pub mod bcd;
mod crc;
pub mod cue;
pub mod disc_position;
mod ecc;
pub mod internal;
pub mod msf;
pub mod sector;
pub mod subchannel;
mod toc;

pub use bcd::Bcd;
pub use disc_position::DiscPosition;
pub use msf::Msf;
pub use sector::Sector;
use std::clone::Clone;
use std::io;
use std::path::PathBuf;
use thiserror::Error;
pub use toc::Toc;

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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, PartialEq, Eq)]
pub struct Track {
    /// Track number
    pub track: Bcd,
    /// Track format
    pub format: TrackFormat,
    /// Absolute MSF for the first sector of the track
    pub start: Msf,
    /// Length of the track
    pub length: Msf,
    /// Value of the control bits for this track (upper 4 bits of the first byte of SUBQ data,
    /// containing pre-emphasis, audio/data flag, digital copy flag and 4-channel audio flag)
    pub control: subchannel::AdrControl,
}

impl Track {
    /// Return the absolute Msf for the position `track_msf` in `track`. Will return an error if
    /// the `track_msf` is outside of the track.
    pub fn absolute_msf(&self, track_msf: Msf) -> CdResult<Msf> {
        if track_msf < self.length {
            // If the image format is not bogus that shouldn't happen, since it would mean that a
            // track has data past the max MSF value
            self.start.checked_add(track_msf).ok_or(CdError::InvalidMsf)
        } else {
            Err(CdError::EndOfTrack)
        }
    }

    /// Return the disc position for the position `track_msf` in `track`. Will return an error if
    /// the `track_msf` is outside of the track.
    ///
    /// This is just a thin convenience function that wraps `Track::absolute_msf` in a DiscPosition
    pub fn disc_position(&self, track_msf: Msf) -> CdResult<DiscPosition> {
        self.absolute_msf(track_msf).map(DiscPosition::Program)
    }
}

/// Possible session formats.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum TrackFormat {
    /// CD-DA audio track (red book audio)
    Audio,
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
    #[error("I/O error: {0}")]
    IoError(#[from] io::Error),
    #[error(
        "Format missmatch. \
            For instance when one attempts to retrieve CD-ROM payloads on an audio track."
    )]
    BadFormat,
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
    #[error("Invalid or unexpected MSF format")]
    InvalidMsf,
    #[error("Invalid or unexpected disc position format")]
    InvalidDiscPosition,
    #[error("Attempted to build a lead-out sector before lead-out start")]
    InvalidLeadOutPosition,
    #[error("Couldn't handle disc position that's before the lead-in")]
    PreLeadInPosition,
    #[error("Couldn't handle disc position that's outside of the disc")]
    OutOfDiscPosition,
    #[error("ZIP format error: {0}")]
    ZipError(#[from] zip::result::ZipError),
}

/// Convenience type alias for a `Result<R, CdError>`
pub type CdResult<R> = std::result::Result<R, CdError>;

#[test]
fn cderror_display() {
    // Make sure that CdError implements Display. This should be true if we set an
    // `#[error("...")]` for every variant
    println!("{}", CdError::BadTrack);
}
