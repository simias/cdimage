//! Generic interface to various Compact Disc (CD) image formats.
//!
//! The architecture is inspired by BizHawk's CD handling code.

#![warn(missing_docs)]

use formats::Backend;

pub mod formats;
pub mod bcd;
pub mod msf;

/// Generic interface for manipulating a CD image. It caches the table
/// of contents for faster access. It provides a higher level
/// interface than the raw `Backend`.
pub struct Disc {
    /// format-specific backend used to access
    backend: Box<Backend>,
}

impl Disc {
    /// Create a new `Disc` with the provided `backend`
    pub fn new(backend: Box<Backend>) -> Disc {
        Disc {
            backend: backend,
        }
    }

    /// Return a reference to the underlying backend
    pub fn backend(&self) -> &Backend {
        &*self.backend
    }
}
