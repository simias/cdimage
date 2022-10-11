//! This module provides generic reusable structures to easily store and lookup a CD's structure in
//! an image format implementation.
//!
//! Those are only useful for implementing a new image format backend.

use std::cmp;
use std::fmt;
use std::path::PathBuf;
use subchannel::AdrControl;
use {Bcd, CdError, CdResult, Msf, Toc, Track, TrackFormat};

/// A generic CD index implementation. Each image format can specialize it by adding its own
/// `private` implementation.
pub struct Index<T> {
    /// Sector pointed at by this index. Stored as an absolute sector index.
    sector_index: u32,
    /// Index number
    index: Bcd,
    /// Track number this index belongs to
    track: Bcd,
    /// Track format this index belongs to
    format: TrackFormat,
    /// Session number this index belongs to
    session: u8,
    /// Control bits for the current track
    control: AdrControl,
    /// Generic private data associated with this index
    private: T,
}

impl<T> Index<T> {
    /// Create a new index
    pub fn new(
        index: Bcd,
        start: Msf,
        track: Bcd,
        format: TrackFormat,
        session: u8,
        control: AdrControl,
        private: T,
    ) -> Index<T> {
        Index {
            sector_index: start.sector_index(),
            index,
            track,
            format,
            session,
            control,
            private,
        }
    }

    /// Retrieve the absolute `sector_index` of the sector referenced
    /// by this index
    pub fn sector_index(&self) -> u32 {
        self.sector_index
    }

    /// Retrieve the MSF of the sector referenced by this index
    pub fn msf(&self) -> Msf {
        Msf::from_sector_index(self.sector_index).unwrap()
    }

    /// Retrieve a reference to the `private` data
    pub fn private(&self) -> &T {
        &self.private
    }

    /// Retrieve a mutable reference to the `private` data
    pub fn private_mut(&mut self) -> &mut T {
        &mut self.private
    }

    /// Retrieve the value of the control bits
    pub fn control(&self) -> AdrControl {
        self.control
    }

    /// Retrieve the index number in BCD
    pub fn index(&self) -> Bcd {
        self.index
    }

    /// Retrieve the track number in BCD
    pub fn track(&self) -> Bcd {
        self.track
    }

    /// Retrieve the format of the track containing this index
    pub fn format(&self) -> TrackFormat {
        self.format
    }

    /// Retrieve the session number
    pub fn session(&self) -> u8 {
        self.session
    }

    /// Return `true` if the index number is 0
    pub fn is_pregap(&self) -> bool {
        self.index.bcd() == 0
    }
}

impl<T> cmp::PartialEq for Index<T> {
    fn eq(&self, other: &Index<T>) -> bool {
        self.sector_index == other.sector_index
    }
}

impl<T> cmp::Eq for Index<T> {}

impl<T> cmp::PartialOrd for Index<T> {
    fn partial_cmp(&self, other: &Index<T>) -> Option<cmp::Ordering> {
        self.sector_index.partial_cmp(&other.sector_index)
    }
}

impl<T> Ord for Index<T> {
    fn cmp(&self, other: &Index<T>) -> cmp::Ordering {
        self.sector_index.cmp(&other.sector_index)
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
    pub fn new(
        file: PathBuf,
        mut indices: Vec<Index<T>>,
        lead_out: Msf,
    ) -> CdResult<IndexCache<T>> {
        if indices.is_empty() {
            return Err(CdError::BadImage {
                path: file,
                desc: "Empty disc".to_string(),
            });
        }

        // Make sure the list is sorted
        indices.sort();

        {
            let index0 = &indices[0];

            if index0.sector_index != 0 {
                let error = format!("Track 01's pregap starts at {}", index0.msf());

                return Err(CdError::BadImage {
                    path: file,
                    desc: error,
                });
            }
        }

        // TODO: Add more validation here.

        Ok(IndexCache {
            indices,
            lead_out: lead_out.sector_index(),
        })
    }

    /// Return the MSF of the first sector in the lead out.
    pub fn lead_out(&self) -> Msf {
        Msf::from_sector_index(self.lead_out).unwrap()
    }

    /// Return a reference to the index at position `pos` or `None` if
    /// it's out of bounds
    pub fn get(&self, pos: usize) -> Option<&Index<T>> {
        self.indices.get(pos)
    }

    /// Locate the index directly before `msf` and return its position along with a reference to
    /// the `Index` struct. Returns `None` if the index is in the lead-out.
    pub fn find_index_for_msf(&self, msf: Msf) -> Option<(usize, &Index<T>)> {
        let sector = msf.sector_index();

        if sector >= self.lead_out {
            return None;
        }

        let pos = match self
            .indices
            .binary_search_by(|index| index.sector_index.cmp(&sector))
        {
            // The MSF matched an index exactly
            Ok(i) => i,
            // No exact match, the function returns the index of
            // the first element greater than `sector` (on one
            // past the end if no greater element is found).
            Err(i) => i - 1,
        };

        Some((pos, &self.indices[pos]))
    }

    /// Locate `index` for `track` and return its position along with
    /// a reference to the `Index` struct.
    pub fn find_index_for_track(&self, track: Bcd, index: Bcd) -> CdResult<(usize, &Index<T>)> {
        match self
            .indices
            .binary_search_by(|idx| match idx.track().cmp(&track) {
                cmp::Ordering::Equal => idx.index().cmp(&index),
                o => o,
            }) {
            Ok(i) => Ok((i, &self.indices[i])),
            Err(_) => Err(CdError::BadTrack),
        }
    }

    /// Locate index1 for `track` and return its position along with a
    /// reference to the `Index` struct.
    pub fn find_index01_for_track(&self, track: Bcd) -> CdResult<(usize, &Index<T>)> {
        self.find_index_for_track(track, Bcd::ONE)
    }

    /// Return the length of the given track starting at INDEX 01, not
    /// counting the pregap. Also returns the position and a reference
    /// to the INDEX 01 for this track.
    pub fn track_length(&self, track: Bcd) -> CdResult<(Msf, usize, &Index<T>)> {
        let (pos01, index01) = self.find_index01_for_track(track)?;

        // Iterate over the remaining indices to find the beginning of
        // the next track

        let next_track = self.indices[pos01 + 1..]
            .iter()
            .find(|i| i.track() != track);

        let end = match next_track {
            // We found the next track, the previous sector is the
            // last one in our track.
            Some(next) => next.sector_index(),
            // Seems like we got the last track
            None => self.lead_out,
        };

        let len = Msf::from_sector_index(end - index01.sector_index()).unwrap();

        Ok((len, pos01, index01))
    }

    /// Return the absolute Msf for the position `track_msf` in
    /// `track`. Will return an error if the `track_msf` is outside of
    /// the track or if `track` doesn't exist.
    pub fn track_msf(&self, track: Bcd, track_msf: Msf) -> CdResult<Msf> {
        // We need to make sure that `track_msf` is not bigger than
        // this tracks' length.
        let (len, _, index01) = self.track_length(track)?;

        if track_msf < len {
            Ok(index01.msf() + track_msf)
        } else {
            Err(CdError::EndOfTrack)
        }
    }

    /// Build a table of contents with the current cache's contents
    pub fn toc(&self) -> CdResult<Toc> {
        let track_count = self.indices.last().map(|i| i.track).unwrap_or(Bcd::ZERO);

        let mut tracks = Vec::with_capacity(track_count.binary() as usize);
        for b in 1..=99 {
            let track_no = Bcd::from_binary(b).unwrap();

            match self.track_length(track_no) {
                Ok((len, _, idx)) => {
                    let track = Track {
                        track: track_no,
                        format: idx.format,
                        start: idx.msf(),
                        length: len,
                        control: idx.control,
                    };

                    tracks.push(track);
                }
                // Last track
                Err(_) => break,
            }
        }

        Toc::new(tracks)
    }
}

impl<T> fmt::Debug for IndexCache<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut force_display = true;
        let mut session = 0;
        let mut track = Bcd::ZERO;

        for i in &self.indices {
            if i.session != session || force_display {
                writeln!(f, "Session {}:", i.session)?;
                session = i.session;
                force_display = true;
            }

            if i.track != track || force_display {
                writeln!(f, "  Track {} {:?}:", i.track, i.format)?;
                track = i.track;
                force_display = false;
            }

            writeln!(f, "    Index {}: {}", i.index, i.msf())?;
        }

        writeln!(f, "Lead-out: {}", self.lead_out())
    }
}
