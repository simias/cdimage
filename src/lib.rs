//! Generic interface to various Compact Disc (CD) image formats.
//!
//! The architecture is inspired by BizHawk's CD handling code.

#![warn(missing_docs)]

#[macro_use]
extern crate arrayref;
#[macro_use]
extern crate bitflags;
extern crate thiserror;

use bcd::Bcd;
use msf::Msf;
use sector::Sector;
use std::clone::Clone;
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

/// Abstract read-only interface to an image format
pub trait Image {
    /// Return a string identifying the image format in a
    /// human-readable way. If the backend is daisy-chained it should
    /// mention the underlying image format as well.
    fn image_format(&self) -> String;

    /// Read a single sector at the given MSF
    fn read_sector(&mut self, msf: Msf) -> CdResult<Sector>;

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
}

/// Convenience type alias for a `Result<R, CdError>`
pub type CdResult<R> = std::result::Result<R, CdError>;
