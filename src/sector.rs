//! CD sector interface.

use CdError;
use CdResult;
use TrackFormat;

use bcd::Bcd;
use msf::Msf;

/// Sector metadata, contains informations about the position and format of a given sector.
#[derive(Clone)]
pub struct Metadata {
    /// Absolute MSF of the sector
    pub msf: Msf,
    /// Relative MSF within the current track (decrements in the pregap/index0)
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

/// Structure containing a single sector. For better peformance it tries to be as lazy as possible
/// and regenerate missing sector data only if it's requested.
#[derive(Clone)]
pub struct Sector {
    /// Which portions of `data` are currently valid
    ready: DataReady,
    /// Actual sector data, only the portions set in `ready` are currently valid.
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

    /// Retreive the entire sector data (except for the subchannel data).
    pub fn data_2352(&mut self) -> CdResult<&[u8; 2352]> {
        if self.ready.contains(DataReady::DATA_2352) {
            Ok(&self.data)
        } else {
            unimplemented!()
        }
    }

    /// Returns the raw 16bit CD-ROM header for this sector. Returns an error if this is not a
    /// CD-ROM track. If the header wasn't available in the original image format, it will be
    /// created on the fly.
    pub fn cd_rom_header_raw(&mut self) -> CdResult<&[u8; 16]> {
        if !self.ready.contains(DataReady::HEADER) {
            self.build_cd_rom_header()?;
        }

        Ok(array_ref![self.data, 0, 16])
    }

    /// Parse the CD-ROM header and return it. Uses `cd_rom_header_raw` internally.
    pub fn cd_rom_header(&mut self) -> CdResult<CdRomHeader> {
        let header = self.cd_rom_header_raw()?;

        // Validate sync pattern
        if header[0] != 0 || header[11] != 0 {
            return Err(CdError::BadSyncPattern);
        }

        if header.iter().take(11).skip(1).any(|&b| b != 0xff) {
            return Err(CdError::BadSyncPattern);
        }

        let m = header[12];
        let s = header[13];
        let f = header[14];

        let msf = match Msf::from_bcd(m, s, f) {
            Some(msf) => msf,
            None => return Err(CdError::BadBcd),
        };

        let mode = match header[15] {
            1 => CdRomMode::Mode1,
            2 => CdRomMode::Mode2,
            _ => return Err(CdError::BadFormat),
        };

        Ok(CdRomHeader { msf, mode })
    }

    fn build_cd_rom_header(&mut self) -> CdResult<()> {
        let mode = match self.metadata.format {
            TrackFormat::Mode1 => CdRomMode::Mode1,
            TrackFormat::Mode2Xa => CdRomMode::Mode2,
            TrackFormat::Mode2CdI => CdRomMode::Mode2,
            _ => return Err(CdError::BadFormat),
        };

        let msf = self.metadata.msf.into_bcd();

        // CD-ROM Sync pattern
        self.data[0] = 0;
        for i in 1..11 {
            self.data[i] = 0xff;
        }
        self.data[11] = 0;

        // Sector MSF
        self.data[12] = msf.0.bcd();
        self.data[13] = msf.1.bcd();
        self.data[14] = msf.2.bcd();

        // Sector mode
        self.data[15] = mode as u8;

        self.ready.insert(DataReady::HEADER);

        Ok(())
    }

    /// Retreive the sector's metadata. This is *not* the subchannel data, the sector position is
    /// where it's expected to be based on the CD's table of contents, not based on subchannel Q
    /// data.
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// Retrieve the CD-ROM XA Mode2 subheader. Returns `CdError::BadFormat` if this is not a
    /// CD-ROM XA sector.
    pub fn mode2_xa_subheader(&self) -> CdResult<XaSubHeader> {
        // Should we allow CDi tracks as well? Probably, but it's not
        // like I have a CDi image to test at the moment...
        if self.metadata.format != TrackFormat::Mode2Xa {
            return Err(CdError::BadFormat);
        }

        // The subheader is at the beginning of the payload
        if !self.ready.contains(DataReady::PAYLOAD) {
            // Should we really support this case? Which image format could leave us in this state?
            unimplemented!("Missing payload for a track!");
        }

        Ok(XaSubHeader(*array_ref![self.data, 16, 8]))
    }

    /// Retrieve a CD-ROM XA Mode 2 payload. Returns `CdError::BadFormat` if this is not a Mode 2
    /// sector.
    ///
    /// For Form 1 tracks the slice returned will be either be 2048 or 2324 bytes long depending on
    /// whether the sector is form 1 or form 2 respectively.
    pub fn mode2_xa_payload(&self) -> CdResult<&[u8]> {
        let subheader = self.mode2_xa_subheader()?;

        let payload = match subheader.submode().form() {
            XaForm::Form1 => &self.data[24..2072],
            XaForm::Form2 => &self.data[24..2348],
        };

        Ok(payload)
    }
}

bitflags! {
    /// Bitflag holding the data ready to be read from the sector.
    struct DataReady: u8 {
        /// 16byte sector header for CD-ROM and CDi data tracks. Contains the sync pattern, MSF and
        /// mode of the sector. Some image formats don't store this information since it can be
        /// reconstructed easily.
        const HEADER    = 0b0000_0001;
        /// Sector data without the header and error detection/correction bits. The actual portion
        /// of the sector this represents varies depends on the sector format. For CD-ROM XA and
        /// CD-i Mode 2 the subheader is considered to be part of the payload.
        ///
        /// Maybe this flag is useless, could there be any situation where this won't be set?
        /// Arguably we won't be able to get the payload in certain image formats (pregap in CUEs
        /// for instance) but then we can just fill them with zeroes?
        const PAYLOAD   = 0b0000_0010;
        /// Error detection and correction bits (if applicable, always set for audio tracks).
        const ECM       = 0b0000_0100;
        /// The entire 2352 bytes of sector data (everything except for the subchannel data)
        const DATA_2352 = Self::HEADER.bits | Self::PAYLOAD.bits | Self::ECM.bits;
        /// Set when the metadata is valid
        const METADATA  = 0b0000_1000;
    }
}

/// Decoded CD-ROM sector header
pub struct CdRomHeader {
    /// Sector MSF (normally should match the one in the metadata, although if the CD is improperly
    /// formatted it could be different)
    pub msf: Msf,
    /// CD-ROM mode for this sector
    pub mode: CdRomMode,
}

/// Mode for a CD-ROM sector
pub enum CdRomMode {
    /// Mode1 ("Regular" CD-ROM)
    Mode1 = 1,
    /// Mode2 (Used for various other sub-formats, such as CD-ROM XA)
    Mode2 = 2,
}

/// Mode 2 XA sub-header (from the CDi "green book"):
///
///   byte 0: File Number
///   byte 1: Channel Number
///   byte 2: Submode
///   byte 3: Coding Information
///   byte 4: File Number
///   byte 5: Channel Number
///   byte 6: Submode
///   byte 7: Coding Information
///
/// The subheader starts at byte 16 of CD-ROM XA sectors, just after the CD-ROM header.
/// The data is copied twice for data integrity but both copies should be identical
pub struct XaSubHeader([u8; 8]);

impl XaSubHeader {
    /// Return the first File Number
    pub fn file_number(&self) -> u8 {
        self.0[0]
    }

    /// Return the first Channel Number
    pub fn channel_number(&self) -> u8 {
        self.0[1]
    }

    /// Return the first Submode
    pub fn submode(&self) -> XaSubmode {
        XaSubmode(self.0[2])
    }

    /// Returns the first coding info based on the sector type
    pub fn coding_info(&self) -> XaCodingInfo {
        let submode = self.submode();
        let coding = self.0[3];

        if submode.video() {
            XaCodingInfo::Video(XaCodingVideo(coding))
        } else if submode.audio() {
            XaCodingInfo::Audio(XaCodingAudio(coding))
        } else {
            XaCodingInfo::Unknown(coding)
        }
    }
}

/// Possible interpretations of the XA sub-header Coding Information
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum XaCodingInfo {
    /// Video coding info
    Video(XaCodingVideo),
    /// Audio coding info
    Audio(XaCodingAudio),
    /// Unknown or unsupported Coding Information
    Unknown(u8),
}

/// Video Coding Information byte from an XA sub-header
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct XaCodingVideo(pub u8);

/// Audio Coding Information byte from an XA sub-header
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct XaCodingAudio(pub u8);

impl XaCodingAudio {
    /// Returns `true` if the `stereo` bit is set.
    ///
    /// Warning: according to the Green Book the field is actualy 2bits (bits 0 and 1) but values
    /// with bit 1 set are "reserved" so this implementation completely ignores that high bit.
    pub fn stereo(self) -> bool {
        self.0 & 1 != 0
    }

    /// Returns the sampling frequency for this sector.
    ///
    /// Warning: according to the Green Book the field is actualy 2bits (bits 2 and 3) but values
    /// with bit 1 set are "reserved" so this implementation completely ignores that high bit.
    pub fn sampling_frequency(self) -> XaSamplingFreq {
        if self.0 & (1 << 2) != 0 {
            XaSamplingFreq::F18_9
        } else {
            XaSamplingFreq::F37_8
        }
    }

    /// Returns the number of bits per sample
    ///
    /// Warning: according to the Green Book the field is actualy 2bits (bits 4 and 5) but values
    /// with bit 1 set are "reserved" so this implementation completely ignores that high bit.
    pub fn bits_per_sample(self) -> XaBitsPerSample {
        if self.0 & (1 << 4) != 0 {
            XaBitsPerSample::S8Bits
        } else {
            XaBitsPerSample::S4Bits
        }
    }

    /// Returns true if emphasis is on for this sector
    pub fn emphasis(self) -> bool {
        self.0 & (1 << 6) != 0
    }
}

/// Possible values for the sampling frequency of an audio XA sector
pub enum XaSamplingFreq {
    /// 37.8 kHz
    F37_8 = 37_800,
    /// 18.9 kHz
    F18_9 = 18_900,
}

/// Possible values for the number of bits per sample of an audio XA sector
pub enum XaBitsPerSample {
    /// 4 bits per sample
    S4Bits = 4,
    /// 8 bits per sample
    S8Bits = 8,
}

/// The Submode byte in a Mode 2 XA sub-header (byte 6)
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct XaSubmode(pub u8);

impl XaSubmode {
    /// True if the End Of Record (EOR) bit is set.
    pub fn end_of_record(self) -> bool {
        self.0 & 1 != 0
    }

    /// True if the Video (V) bit is set
    pub fn video(self) -> bool {
        self.0 & (1 << 1) != 0
    }

    /// True if the Audio (A) bit is set.
    pub fn audio(self) -> bool {
        self.0 & (1 << 2) != 0
    }

    /// True if the Data (D) bit is set.
    pub fn data(self) -> bool {
        self.0 & (1 << 3) != 0
    }

    /// True if the Trigger (T) bit is set.
    pub fn trigger(self) -> bool {
        self.0 & (1 << 4) != 0
    }

    /// Return the sector form
    pub fn form(self) -> XaForm {
        let form2 = self.0 & (1 << 5) != 0;

        if form2 {
            XaForm::Form2
        } else {
            XaForm::Form1
        }
    }

    /// True if the Real-Time Sector (RT) bit is set
    pub fn real_time(self) -> bool {
        self.0 & (1 << 6) != 0
    }

    /// True if the End Of File (EOF) bit is set
    pub fn end_of_file(self) -> bool {
        self.0 & (1 << 7) != 0
    }
}

/// CD-ROM XA Mode 2 sectors have two possible forms (advertised in the subheader)
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum XaForm {
    /// Mode 2 Form 1: 2048 bytes of data, 4 bytes of error detection and 276 bytes of error
    /// correction
    Form1 = 0,
    /// Mode 2 Form 2: 2324 bytes of data, 4 bytes of "quality control".
    ///
    /// The CDi spec says that those bytes are reserved and ignored by the system and *recommends*
    /// to use the same algorithm as for the Form 1 error detection code. It's also possible to
    /// leave it to zero if unused...
    Form2 = 1,
}

/// Interface used to build a new sector "in place" to avoid copying sector data around.
pub struct SectorBuilder {
    sector: Sector,
}

impl SectorBuilder {
    /// Create a new SectorBuilder using `sector` for storage. The contents of `sector` will be
    /// reset.
    pub fn new() -> SectorBuilder {
        SectorBuilder {
            sector: Sector::empty(),
        }
    }

    /// Load up the full 2352 bytes of sector data. The `loader` function will be called with a
    /// mutable reference to the sector data. If the `loader` callback returns an error the sector
    /// data won't be tagged as valid.
    pub fn set_data_2352<F, E>(&mut self, loader: F) -> Result<(), E>
    where
        F: FnOnce(&mut [u8; 2352]) -> Result<(), E>,
    {
        loader(&mut self.sector.data)?;

        self.sector.ready.insert(DataReady::DATA_2352);

        Ok(())
    }

    /// Set the metadata for the sector.
    pub fn set_metadata(&mut self, metadata: Metadata) {
        self.sector.metadata = metadata;
        self.sector.ready.insert(DataReady::METADATA);
    }

    /// Returns the underlying sector. Panics if the sector's metadata hasn't been set.
    pub fn unwrap(self) -> Sector {
        assert!(self.sector.ready.contains(DataReady::METADATA));

        self.sector
    }
}

impl Default for SectorBuilder {
    fn default() -> Self {
        Self::new()
    }
}
