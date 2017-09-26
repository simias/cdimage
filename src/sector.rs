//! CD sector interface.

use rustc_serialize::{Decodable, Encodable, Decoder, Encoder};

use CdError;
use TrackFormat;

use msf::Msf;
use bcd::Bcd;

/// Sector metadata, contains informations about the position and
/// format of a given sector.
#[derive(RustcDecodable, RustcEncodable)]
pub struct Metadata {
    /// Absolute MSF of the sector
    pub msf: Msf,
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
    /// Create a new empty sector
    pub fn empty() -> Sector {
        Sector {
            ready: DataReady::empty(),
            data: [0; 2352],
            metadata: Metadata {
                msf: Msf::zero(),
                track_msf: Msf::zero(),
                index: Bcd::zero(),
                track: Bcd::zero(),
                format: TrackFormat::Audio,
                session: 0,
            },
        }
    }

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

    /// Retrieve the CD-ROM XA Mode2 subheader. Returns
    /// `CdError::BadFormat` if this is not a CD-ROM XA sector.
    pub fn mode2_xa_subheader(&self) -> Result<XaSubHeader, CdError> {
        // Should we allow CDi tracks as well? Probably, but it's not
        // like I have a CDi image to test at the moment...
        if self.metadata.format != TrackFormat::Mode2Xa {
            return Err(CdError::BadFormat);
        }

        // The subheader is at the beginning of the payload
        if !self.ready.contains(PAYLOAD) {
            // Should we really support this case? Which image format
            // could leave us in this state?
            panic!("Missing payload for a track!");
        }

        Ok(XaSubHeader::new(array_ref![self.data, 16, 8]))
    }

    /// Retrieve a CD-ROM XA Mode 2 payload. Returns
    /// `CdError::BadFormat` if this is not a Mode 2 sector.
    ///
    /// For Form 1 tracks the slice returned will be either be 2048 or
    /// 2324 bytes long depending on whether the sector is form 1 or
    /// form 2 respectively.
    pub fn mode2_xa_payload(&self) -> Result<&[u8], CdError> {
        let subheader = try!(self.mode2_xa_subheader());

        let payload =
            match subheader.form() {
                XaForm::Form1 => &self.data[24..2072],
                XaForm::Form2 => &self.data[24..2348],
            };

        Ok(payload)
    }
}

impl Encodable for Sector {
    fn encode<S: Encoder>(&self, s: &mut S) -> Result<(), S::Error> {

        s.emit_struct("Sector", 3, |s| {
            try!(s.emit_struct_field("ready", 0,
                                     |s| self.ready.encode(s)));

            try!(s.emit_struct_field(
                "data", 1,
                |s| s.emit_seq(
                    2352,
                    |s| {
                        for (i, &b) in self.data.iter().enumerate() {
                            try!(s.emit_seq_elt(i, |s| b.encode(s)))
                        }

                        Ok(())
                    })));

            try!(s.emit_struct_field("metadata", 2,
                                     |s| self.metadata.encode(s)));


            Ok(())
        })
    }
}

impl Decodable for Sector {
    fn decode<D: Decoder>(d: &mut D) -> Result<Sector, D::Error> {
        d.read_struct("Sector", 3, |d| {
            let mut sector = Sector::empty();

            sector.ready =
                try!(d.read_struct_field("ready", 0,
                                         Decodable::decode));

            try!(d.read_struct_field(
                    "data", 1,
                    |d| {
                        d.read_seq(|d, len| {
                            if len != 2352 {
                                return Err(
                                    d.error("wrong sector data length"));
                            }

                            for i in 0..len {
                                sector.data[i] =
                                    try!(d.read_seq_elt(i, Decodable::decode));
                            }

                            Ok(len)
                        })
                    }));

            sector.metadata =
                try!(d.read_struct_field("metadata", 2,
                                         Decodable::decode));

            Ok(sector)
        })
    }
}

bitflags! {
    /// Bitflag holding the data ready to be read from the sector.
    #[derive(RustcDecodable, RustcEncodable)]
    flags DataReady: u8 {
        /// 16byte sector header for CD-ROM and CDi data
        /// tracks. Contains the sync pattern, MSF and mode of the
        /// sector. Some image formats don't store this information
        /// since it can be reconstructed easily.
        const HEADER    = 0b00000001,
        /// Sector data without the header and error
        /// detection/correction bits. The actual portion of the
        /// sector this represents varies depends on the sector
        /// format. For CD-ROM XA and CD-i Mode 2 the subheader is
        /// considered to be part of the payload.
        ///
        /// Maybe this flag is useless, could there be any situation
        /// where this won't be set? Arguably we won't be able to get
        /// the payload in certain image formats (pregap in CUEs for
        /// instance) but then we can just fill them with zeroes?
        const PAYLOAD   = 0b00000010,
        /// Error detection and correction bits (if applicable, always
        /// set for audio tracks).
        const ECM       = 0b00000100,
        /// The entire 2352 bytes of sector data (everything except
        /// for the subchannel data)
        const DATA_2352 = HEADER.bits | PAYLOAD.bits | ECM.bits,
        /// Set when the metadata is valid
        const METADATA  = 0b00001000,
    }
}

/// Mode 2 XA sub-header (from the CDi "green book"):
///
///   byte 0: File number
///   byte 1: Channel number
///   byte 2: Submode
///   byte 3: Coding information
///   byte 4: File number
///   byte 5: Channel number
///   byte 6: Submode
///   byte 7: Coding information
///
/// The subheader starts at byte 16 of CD-ROM XA sectors, just after
/// the CD-ROM header.
pub struct XaSubHeader {
    subheader: [u8; 8],
}

impl XaSubHeader {
    /// Create a new XaSubHeader instance from the 8 bytes of
    /// `subheader` data.
    pub fn new(subheader: &[u8; 8]) -> XaSubHeader {
        XaSubHeader {
            subheader: *subheader,
        }
    }

    /// Return "form" of this sector
    pub fn form(&self) -> XaForm {
        match self.subheader[2] & 0x20 != 0 {
            false => XaForm::Form1,
            true  => XaForm::Form2,
        }
    }
}

/// CD-ROM XA Mode 2 sectors have two possible forms (advertised in
/// the subheader)
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum XaForm {
    /// Mode 2 Form 1: 2048 bytes of data, 4 bytes of error detection
    /// and 276 bytes of error correction
    Form1,
    /// Mode 2 Form 2: 2324 bytes of data, 4 bytes of "quality
    /// control".
    ///
    /// The CDi spec says that those bytes are reserved and ignored by
    /// the system and *recommends* to use the same algorithm as for
    /// the Form 1 error detection code. It's also possible to leave
    /// it to zero if unused...
    Form2,
}

/// Interface used to build a new sector "in place" to avoid copying
/// sector data around.
pub struct SectorBuilder<'a> {
    sector: &'a mut Sector,
}

impl<'a> SectorBuilder<'a> {
    /// Create a new SectorBuilder using `sector` for storage. The
    /// contents of `sector` will be reset.
    pub fn new(sector: &mut Sector) -> SectorBuilder {
        sector.ready = DataReady::empty();

        SectorBuilder {
            sector: sector,
        }
    }

    /// Load up the full 2352 bytes of sector data. The `loader`
    /// function will be called with a mutable reference to the sector
    /// data. If the `loader` callback returns an error the sector
    /// data won't be tagged as valid.
    pub fn set_data_2352<F, E>(&mut self, loader: F) -> Result<(), E>
        where F: FnOnce(&mut [u8; 2352]) -> Result<(), E> {

        try!(loader(&mut self.sector.data));

        self.sector.ready.insert(DATA_2352);

        Ok(())
    }

    /// Set the metadata for the sector
    pub fn set_metadata(&mut self, metadata: Metadata) {
        self.sector.metadata = metadata;
        self.sector.ready.insert(METADATA);
    }
}
