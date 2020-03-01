//! Generic interface to various Compact Disc (CD) image formats.
//!
//! The architecture is inspired by BizHawk's CD handling code.

#![warn(missing_docs)]

#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate arrayref;

use bcd::Bcd;
use msf::Msf;
use sector::Sector;
use std::fmt;
use std::io;
use std::path::PathBuf;
use std::clone::Clone;

pub mod bcd;
pub mod crc;
pub mod cue;
pub mod internal;
pub mod msf;
pub mod sector;
pub mod subchannel;

/// Abstract read-only interface to an image format
pub trait Image {
    /// Return a string identifying the image format in a
    /// human-readable way. If the backend is daisy-chained it should
    /// mention the underlying image format as well.
    fn image_format(&self) -> String;

    /// Read a single sector at the given MSF
    fn read_sector(&mut self, Msf) -> CdResult<Sector>;

    /// Get the table of contents
    fn toc(&self) -> &Toc;
}

/// Struct representing a track's attributes
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
pub struct Toc {
    /// Track list
    tracks: Vec<Track>,
}

impl Toc {
    /// Return the Track description for the given `track_no`. Returns `None` if `track_no` is 0 or
    /// greater than the total number of tracks.
    pub fn get_track(&self, track_no: Bcd) -> Option<&Track> {
        let t = track_no.binary();

        if t < 1 {
            return None;
        }

        self.tracks.get((t - 1) as usize)
    }
}

/// Possible session formats.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum SessionFormat {
    /// CD-DA (audio CD, "red book" specification) or CD-ROM ("yellow
    /// book" specification) session
    CddaCdRom,
    /// CD-i (compact disc interactive, "green book"
    /// specification). Used on Philips' CD-i console.
    Cdi,
    /// CD-ROM XA (extended architecture). Used on Sony's PlayStation
    /// console.
    Cdxa,
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

/// Error type for disc operations.
#[derive(Debug)]
pub enum CdError {
    /// Generic I/O error
    IoError(io::Error),
    /// Format missmatch. For instance when one attempts to retrieve
    /// CD-ROM payloads on an audio track.
    BadFormat,
    /// Attempted to access a sector past the end of the CD
    LeadOut,
    /// Unexpected or corrupted image format. Contains the path of the
    /// file and the line where the error occured and a string
    /// describing the problem in a human-readble way.
    ParseError(PathBuf, u32, String),
    /// Disc format error (two tracks with the same number, missing
    /// track, absurd index etc...). Contains the path of the file and
    /// a `String` describing the problem.
    BadImage(PathBuf, String),
    /// Attempted to access an invalid track number
    BadTrack,
    /// Attempted to access a track past its end
    EndOfTrack,
}

/// We want CdError to be clone-able in order to allow caching easily.
impl Clone for CdError {
    fn clone(&self) -> Self {
        match self {
            // IoError can't be cloned, attempt a best-effort workaround
            CdError::IoError(ref e) => {
                let new =
                    match e.raw_os_error() {
                        Some(c) => io::Error::from_raw_os_error(c),
                        None => io::Error::new(e.kind(), "Unknown"),
                    };

                CdError::IoError(new)
            }
            e => e.clone(),
        }
    }
}

/// Convenience type alias for a `Result<R, CdError>`
pub type CdResult<R> = std::result::Result<R, CdError>;

impl fmt::Display for CdError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &CdError::ParseError(ref path, line, ref err) => {
                write!(f, "{}:{}: {}", path.display(), line, err)
            }
            e => write!(f, "{:?}", e),
        }
    }
}
