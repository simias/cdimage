//! This module provides generic reusable structures to easily store
//! and lookup a CD's structure in an image format implementation.
//!
//! Those are only useful for implementing a new image format backend.

use std::fmt;
use std::cmp;
use std::path::PathBuf;

use bcd::Bcd;
use msf::Msf;

use TrackFormat;
use CdError;

/// A generic CD index implementation. Each image format can
/// specialize it by adding its own `private` implementation.
pub struct Index<T> {
    /// Sector pointed at by this index. Stored as an absolute sector
    /// index.
    start: u32,
    /// Index number
    index: Bcd,
    /// Track number this index belongs to
    track: Bcd,
    /// Track format this index belongs to
    format: TrackFormat,
    /// Session number this index belongs to
    session: u8,
    /// Generic private data associated with this index
    private: T,
}

impl<T> Index<T> {
    /// Create a new index
    pub fn new(index: Bcd,
               start: Msf,
               track: Bcd,
               format: TrackFormat,
               session: u8,
               private: T) -> Index<T> {
        Index {
            start: start.sector_index(),
            index: index,
            track: track,
            format: format,
            session: session,
            private: private,
        }
    }

    /// Retrieve a reference to the `private` data
    pub fn private(&self) -> &T {
        &self.private
    }

    /// Retrieve a mutable reference to the `private` data
    pub fn private_mut(&mut self) -> &mut T {
        &mut self.private
    }
}

impl<T> cmp::PartialEq for Index<T> {
    fn eq(&self, other: &Index<T>) -> bool {
        self.start == other.start
    }
}

impl<T> cmp::Eq for Index<T> {
}

impl<T> cmp::PartialOrd for Index<T> {
    fn partial_cmp(&self, other: &Index<T>) -> Option<cmp::Ordering> {
        self.start.partial_cmp(&other.start)
    }
}

impl<T> Ord for Index<T> {
    fn cmp(&self, other: &Index<T>) -> cmp::Ordering {
        self.start.cmp(&other.start)
    }
}

/// A simple cache structure used to quickly look up where an
/// arbitrary MSF lives on the disc.
pub struct IndexCache<T> {
    /// Ordered vector containing all the indices in the CD
    indices: Vec<Index<T>>,
    /// First sector in the lead-out, given as a sector index instead
    /// of an MSF to avoid converting back and forth all the time.
    lead_out: u32,
}

impl<T> IndexCache<T> {

    /// Create a new `IndexCache` from a vector of indices and the MSF
    /// of the first sector in the lead-out. This method will return
    /// an error if the disc structure makes no sense (duplicate
    /// tracks, indices in the wrong order etc...).
    pub fn new(file: PathBuf,
               mut indices: Vec<Index<T>>,
               lead_out: Msf) -> Result<IndexCache<T>, CdError> {
        if indices.is_empty() {
            return Err(CdError::BadImage(file, "Empty disc".to_string()));
        }

        // Make sure the list is sorted
        indices.sort();

        // TODO: Add more validation here.

        Ok(IndexCache {
            indices: indices,
            lead_out: lead_out.sector_index(),
        })
    }

    /// Return the MSF of the first sector in the lead out.
    pub fn lead_out(&self) -> Msf {
        Msf::from_sector_index(self.lead_out).unwrap()
    }
}

impl<T> fmt::Debug for IndexCache<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut force_display = true;
        let mut session = 0;
        let mut track = Bcd::zero();

        for i in &self.indices {
            if i.session != session || force_display {
                try!(writeln!(f, "Session {}:", i.session));
                session = i.session;
                force_display = true;
            }

            if i.track != track || force_display {
                try!(writeln!(f, "  Track {} {:?}:", i.track, i.format));
                track = i.track;
                force_display = false;
            }

            try!(writeln!(f, "    Index {}: {}",
                          i.index,
                          Msf::from_sector_index(i.start).unwrap()));
        }

        writeln!(f, "Lead-out: {}", self.lead_out())
    }
}
