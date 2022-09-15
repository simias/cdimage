//! CD sector interface.

use subchannel::Q;
use {CdError, CdResult, TrackFormat};

use msf::Msf;

/// Structure containing a single sector. For better peformance it tries to be as lazy as possible
/// and regenerate missing sector data only if it's requested.
#[derive(Clone)]
pub struct Sector {
    /// Actual sector data, only the portions set in `ready` are currently valid.
    data: [u8; 2352],
    /// Q subchannel data for this sector
    q: Q,
    /// Format of the track this sector is contained in
    format: TrackFormat,
}

impl Sector {
    /// Create a sector containing only zeroes with the given Q subchannel data and track format.
    /// Returns an error if the format and Q data are not compatible.
    pub fn empty(q: Q, format: TrackFormat) -> CdResult<Sector> {
        let fmt_ok = match format {
            TrackFormat::Audio => q.is_audio(),
            _ => q.is_data(),
        };

        if !fmt_ok {
            return Err(CdError::BadFormat);
        }

        Ok(Sector {
            data: [0; 2352],
            q,
            format,
        })
    }

    /// Returns the Q subchannel data for this sector
    pub fn q(&self) -> &Q {
        &self.q
    }

    /// Retreive the entire sector data (except for the subchannel data).
    pub fn data_2352(&self) -> &[u8; 2352] {
        &self.data
    }

    /// Return the format of the track this sector belongs to
    pub fn format(&self) -> TrackFormat {
        self.format
    }

    /// Load up the full 2352 bytes of sector data. The `loader` function will be called with a
    /// mutable reference to the sector data. If the `loader` callback returns an error the sector
    /// data won't be tagged as valid.
    pub fn set_data_2352<F, E>(&mut self, loader: F) -> Result<(), E>
    where
        F: FnOnce(&mut [u8; 2352]) -> Result<(), E>,
    {
        // The reason we do it this way instead of just returning a mutable reference to the data
        // is to keep track of what parts of the sector data have been initialized, this way we'll
        // be able to add support for partial sector data later if we want (for instance to store
        // data sectors without ECC).
        loader(&mut self.data)?;

        Ok(())
    }

    /// Returns the raw 16bit CD-ROM header for this sector. Returns an error if this is not a
    /// CD-ROM track. If the header wasn't available in the original image format, it will be
    /// created on the fly.
    pub fn cd_rom_header_raw(&self) -> CdResult<&[u8; 16]> {
        if !self.q.is_data() {
            // This is an audio track
            return Err(CdError::BadFormat);
        }

        Ok(array_ref![self.data, 0, 16])
    }

    /// Parse the CD-ROM header and return it. Same failure mode as `Sector::cd_rom_header_raw` but
    /// will also fail if the format is incorrect.
    pub fn cdrom_header(&self) -> CdResult<CdRomHeader> {
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

    /// Retrieve the CD-ROM XA Mode2 subheader. Returns `CdError::BadFormat` if this is not a
    /// CD-ROM XA Mode 2 sector.
    pub fn mode2_xa_subheader(&self) -> CdResult<XaSubHeader> {
        let mode = self.cdrom_header()?.mode;

        if self.format != TrackFormat::Mode2Xa || mode != CdRomMode::Mode2 {
            return Err(CdError::BadFormat);
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

/// Decoded CD-ROM sector header
#[derive(Copy, Clone, PartialEq, Eq)]
pub struct CdRomHeader {
    /// Sector MSF (normally should match the one in the metadata, although if the CD is improperly
    /// formatted it could be different)
    pub msf: Msf,
    /// CD-ROM mode for this sector
    pub mode: CdRomMode,
}

/// Mode for a CD-ROM sector
#[derive(Copy, Clone, PartialEq, Eq)]
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
