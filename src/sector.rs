//! CD sector interface.

use CdError;
use TrackFormat;

use msf::Msf;
use bcd::Bcd;

/// Sector metadata, contains informations about the position and
/// format of a given sector.
pub struct Metadata {
    /// Absolute position of the sector. Stored as a sector index
    /// since it's often used by comparisons and it's faster that way.
    pub msf: u32,
    /// Relative MSF within the current track (decrements in the
    /// pregap/index0)
    pub track_msf: Msf,
    /// Index containing this sector
    pub index: Bcd,
    /// Track containing this sector
    pub track: Bcd,
    /// Track format (and therefore format of this particular sector)
    pub format: TrackFormat,
    /// Number of the session containing this sector
    pub session: u8,
}

/// Structure containing a single sector. For better peformance it
/// tries to be as lazy as possible and regenerate missing sector data
/// only if it's requested.
pub struct Sector {
    /// Which portions of `data` are currently valid
    ready: DataReady,
    /// Actual sector data, only the portions set in `ready` are
    /// currently valid.
    data: [u8; 2352],
    /// Sector metadata
    metadata: Metadata,
}

impl Sector {

    /// Retreive the entire sector data (except for the subchannel
    /// data).
    pub fn data_2352(&mut self) -> Result<&[u8; 2352], CdError> {
        if self.ready.contains(DATA_2352) {
            Ok(&self.data)
        } else {
            unimplemented!()
        }
    }

    /// Retreive the sector's metadata. This is *not* the subchannel
    /// data, the sector position is where it's expected to be based
    /// on the CD's table of contents, not based on subchannel Q data.
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }
}

bitflags! {
    /// Bitflag holding the data ready to be read from the sector.
    flags DataReady: u8 {
        /// Sector header for CD-ROM and CDi data tracks.
        const HEADER    = 0b00000001,
        /// Sector data without the header and error
        /// detection/correction bits.
        const PAYLOAD   = 0b00000010,
        /// Error detection and correction bits (if applicable, always
        /// set for audio tracks).
        const ECM       = 0b00000100,
        /// The entire 2352 bytes of sector data (everything except
        /// for the subchannel data)
        const DATA_2352 = HEADER.bits | PAYLOAD.bits | ECM.bits,
    }
}
