//! Generic interface to various Compact Disc (CD) image formats.
//!
//! The architecture is inspired by BizHawk's CD handling code.

#![warn(missing_docs)]

#[macro_use]
extern crate bitflags;

use std::path::PathBuf;
use std::io;
use std::fmt;

pub mod bcd;
pub mod msf;
pub mod subchannel;
pub mod internal;
pub mod sector;
pub mod cue;

/// Abstract read-only interface to an image format
pub trait Image {
    /// Return a string identifying the image format in a
    /// human-readable way. If the backend is daisy-chained it should
    /// mention the underlying image format as well.
    fn image_format(&self) -> String;
}

/// Possible session formats.
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
    Mode2CdI
}

/// Error type for disc operations.
#[derive(Debug)]
pub enum CdError {
    /// Unexpected or corrupted image format. Contains the path of the
    /// file and the line where the error occured and a string
    /// describing the problem in a human-readble way.
    ParseError(PathBuf, u32, String),
    /// Disc format error (two tracks with the same number, missing
    /// track, absurd index etc...). Contains the path of the file and
    /// a `String` describing the problem.
    BadImage(PathBuf, String),
    /// Generic I/O error
    IoError(io::Error),
}

impl fmt::Display for CdError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &CdError::ParseError(ref path, line, ref err) =>
                write!(f, "{}:{}: {}", path.display(), line, err),
            e =>
                write!(f, "{:?}", e)
        }
    }
}
