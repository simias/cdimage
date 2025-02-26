//! CD sector interface.

use crate::crc::crc32;
use crate::ecc::compute_ecc;
use crate::msf::Msf;
use crate::subchannel::Q;
use crate::{CdError, CdResult, TrackFormat};

/// Structure containing a single sector. For better peformance it tries to be as lazy as possible
/// and regenerate missing sector data only if it's requested.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone)]
pub struct Sector {
    /// Actual sector data, only the portions set in `ready` are currently valid.
    #[cfg_attr(feature = "serde", serde(with = "serde_big_array::BigArray"))]
    data: [u8; 2352],
    /// Q subchannel data for this sector
    q: Q,
    /// Format of the track this sector is contained in
    format: TrackFormat,
}

impl Sector {
    /// Create a sector containing only zeroes with the given Q subchannel data and track format.
    /// The resulting sector may not be valid since any ECC/EDC data will not be generated. Use
    /// `Sector::empty()` if you want a correctly formatted sector with no payload.
    ///
    /// Returns an error if the format and Q data are not compatible.
    pub fn uninitialized(q: Q, format: TrackFormat) -> CdResult<Sector> {
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

    /// Create an empty sector with the given Q subchannel data and track format. If the format
    /// requires headers or CDC/ECC data, it will be generated, the rest of the payload will be all
    /// zeroes.
    ///
    /// Returns an error if the format and Q data are not compatible.
    pub fn empty(q: Q, format: TrackFormat) -> CdResult<Sector> {
        let mut sector = Sector::uninitialized(q, format)?;

        sector.write_headers();
        sector.write_edc_ecc();

        Ok(sector)
    }

    /// If this is a CD-ROM sector (according to the sector's format), rewrite the header (12-byte SYNC
    /// field, 4-byte header).
    ///
    /// For Mode2Xa and CDi formats the subheader won't be touched (since we can't regenerate it
    /// without additional information) with the exception of the submode byte if *both* copies are
    /// 0 (bytes 18 and 22). In this case we set the submode to 0x8 (Data, Form 1) except in the
    /// pregap (INDEX 00), lead-in and lead-out where it's set to 0x28 (Data, Form 2).
    pub fn write_headers(&mut self) {
        // Add the CD-ROM header if necessary
        if let Some(mode) = self.format.cdrom_mode() {
            // Sync field
            self.data[0] = 0;
            for i in 1..11 {
                self.data[i] = 0xff;
            }
            self.data[12] = 0;

            // Sector Address
            let (m, s, f) = self.q.amsf().into_bcd();

            let m = if self.q.is_lead_in() {
                // According to ECMA-130 this may not be accurate in the lead-in:
                //
                //    If the Lead-in Area contains a Digital Data Track, the Sector Address of the
                //    Headers in this area shall contain the Physical Address of the Sector
                //    expressed in terms of the relative time elapsed since the beginning of the
                //    Lead-in Area.
                //
                // Then it explains that the minute byte should be set to 0xA0 + MIN. In practice
                // we don't know what `q.amsf()` will return in the lead-in, and our own
                // implementation in this crate will count to 99:59:74 at the end of the lead-in,
                // so it's clearly not appropriate here. To keep things simple I just cheat by
                // only keeping the last digit of the minutes and setting the tenths to 0xA, which
                // should look like what the spec mandates even if it's not fully accurate.
                0xa0 | (m.bcd() & 0xf)
            } else {
                m.bcd()
            };

            self.data[12] = m;
            self.data[13] = s.bcd();
            self.data[14] = f.bcd();
            self.data[15] = mode as u8;

            if matches!(self.format, TrackFormat::Mode2Xa | TrackFormat::Mode2CdI)
                && self.data[18] == 0
                && self.data[19] == 0
            {
                // We have invalid XA/CDi subheader submodes, set it to Data, Form 1 unless we're
                // in the lead-in, pregap or lead-out in which case we set it to Data, Form 2.
                //
                // The rationale for this decision is that the CDi spec recommends that if the last track
                // of the disc is a data track, then the lead-out should be Mode 2 Form 2. I
                // actually didn't find any such recommendation for the pregap but I think it makes
                // sense that it would be too.
                //
                // If we're within a track I set it to Mode 2 Form 1 mainly for testing purposes,
                // in practice we probably shouldn't be generated sectors within a track, and if we
                // had to we should probably be told what exactly to generate
                let submode = if self.q.is_lead_in() || self.q.is_lead_out() || self.q.is_pregap() {
                    // Data, Form 2
                    0x28
                } else {
                    // Data, Form 1
                    0x08
                };

                self.data[18] = submode;
                self.data[22] = submode;
            }
        }
    }

    /// If the sector's format includes ECC and/or EDC data, recompute it and write it to the
    /// sector.
    pub fn write_edc_ecc(&mut self) {
        // Calculate and add the ECC/EDC data
        match self.format {
            TrackFormat::Audio => (),
            TrackFormat::Mode1 => {
                let crc = crc32(&self.data[0..2064]).to_le_bytes();
                self.data[2064] = crc[0];
                self.data[2065] = crc[1];
                self.data[2066] = crc[2];
                self.data[2067] = crc[3];

                for i in (2068..).take(8) {
                    self.data[i] = 0;
                }

                compute_ecc(array_mut_ref![self.data, 12, 2340]);
            }
            TrackFormat::Mode2Xa | TrackFormat::Mode2CdI => {
                // Look for the form in the Mode2 XA/CDi subheader
                let form = if self.data[18] & (1 << 5) == 0 {
                    XaForm::Form1
                } else {
                    XaForm::Form2
                };

                match form {
                    XaForm::Form1 => {
                        let crc = crc32(&self.data[16..2072]).to_le_bytes();
                        self.data[2072] = crc[0];
                        self.data[2073] = crc[1];
                        self.data[2074] = crc[2];
                        self.data[2075] = crc[3];

                        // Unlike Mode-1, we must zero the MSF and Mode before computing the ECC
                        let tmp = [self.data[12], self.data[13], self.data[14], self.data[15]];
                        self.data[12] = 0;
                        self.data[13] = 0;
                        self.data[14] = 0;
                        self.data[15] = 0;

                        compute_ecc(array_mut_ref![self.data, 12, 2340]);

                        self.data[12] = tmp[0];
                        self.data[13] = tmp[1];
                        self.data[14] = tmp[2];
                        self.data[15] = tmp[3];
                    }
                    XaForm::Form2 => {
                        // Form 2 has EDC but no ECC. Technically even the EDC is optional and
                        // could be left to all zeroes (the green book doesn't even call it EDC but
                        // rather "reserved for quality control"), but let's write it to make
                        // accidental corruptions easier to identify
                        let crc = crc32(&self.data[16..2348]).to_le_bytes();
                        self.data[2348] = crc[0];
                        self.data[2349] = crc[1];
                        self.data[2350] = crc[2];
                        self.data[2351] = crc[3];
                    }
                }
            }
        }
    }

    /// Returns false if the sector's EDC doesn't match the computed value from its contents.
    /// Returns true if the EDC is valid or if the sector does not contain any EDC (for instance
    /// for a CD-DA audio track)
    pub fn edc_valid(&self) -> bool {
        match self.format {
            TrackFormat::Audio => true,
            TrackFormat::Mode1 => {
                let crc = crc32(&self.data[0..2064]);
                let expected = u32::from_le_bytes([
                    self.data[2064],
                    self.data[2065],
                    self.data[2066],
                    self.data[2067],
                ]);

                expected == crc
            }
            TrackFormat::Mode2Xa | TrackFormat::Mode2CdI => {
                // Look for the form in the Mode2 XA/CDi subheader
                let form = if self.data[18] & (1 << 5) == 0 {
                    XaForm::Form1
                } else {
                    XaForm::Form2
                };

                match form {
                    XaForm::Form1 => {
                        let crc = crc32(&self.data[16..2072]);
                        let expected = u32::from_le_bytes([
                            self.data[2072],
                            self.data[2073],
                            self.data[2074],
                            self.data[2075],
                        ]);

                        expected == crc
                    }
                    XaForm::Form2 => {
                        // Form 2 has EDC but no ECC
                        let crc = crc32(&self.data[16..2348]);
                        let expected = u32::from_le_bytes([
                            self.data[2348],
                            self.data[2349],
                            self.data[2350],
                            self.data[2351],
                        ]);

                        // The CRC is optional and is set to zero if not used
                        expected == crc || expected == 0
                    }
                }
            }
        }
    }

    /// Returns the Q subchannel data for this sector
    pub fn q(&self) -> &Q {
        &self.q
    }

    /// Retrieve the entire sector data (except for the subchannel data).
    pub fn data_2352(&self) -> &[u8; 2352] {
        &self.data
    }

    /// Retrieve a mutable reference to the entire sector data (except for the subchannel data).
    pub fn data_2352_mut(&mut self) -> &mut [u8; 2352] {
        &mut self.data
    }

    /// Return the format of the track this sector belongs to
    pub fn format(&self) -> TrackFormat {
        self.format
    }

    /// Returns the raw 16bit CD-ROM header for this sector. Returns an error if this is not a
    /// CD-ROM track (per sub-Q). If the header wasn't available in the original image format, it
    /// will be created on the fly.
    pub fn cd_rom_header_raw(&self) -> CdResult<&[u8; 16]> {
        if !self.q.is_data() {
            // This is an audio track
            return Err(CdError::BadFormat);
        }

        Ok(array_ref![self.data, 0, 16])
    }

    /// Parse the CD-ROM header and return it. Same failure mode as `Sector::cd_rom_header_raw` but
    /// will also fail if the sync pattern or header format is incorrect.
    pub fn cdrom_header(&self) -> CdResult<CdRomHeader> {
        let header = self.cd_rom_header_raw()?;

        // Validate sync pattern
        if header[0] != 0 || header[11] != 0 {
            return Err(CdError::BadSyncPattern);
        }

        if header.iter().take(11).skip(1).any(|&b| b != 0xff) {
            return Err(CdError::BadSyncPattern);
        }

        let m = if self.q.is_lead_in() {
            header[12] - 0xa0
        } else {
            header[12]
        };
        let s = header[13];
        let f = header[14];

        let msf = match Msf::from_bcd(m, s, f) {
            Some(msf) => msf,
            None => return Err(CdError::BadBcd),
        };

        let mode = match header[15] {
            0 => CdRomMode::Empty,
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Copy, Clone, PartialEq, Eq)]
pub struct CdRomHeader {
    /// Sector MSF (normally should match the one in the metadata, although if the CD is improperly
    /// formatted it could be different)
    pub msf: Msf,
    /// CD-ROM mode for this sector
    pub mode: CdRomMode,
}

/// Mode for a CD-ROM sector
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum CdRomMode {
    /// All bytes in positions 16 to 2351 of the sector are set to 0. No CRC/ECC.
    Empty = 0,
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct XaCodingVideo(pub u8);

/// Audio Coding Information byte from an XA sub-header
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum XaSamplingFreq {
    /// 37.8 kHz
    F37_8 = 37_800,
    /// 18.9 kHz
    F18_9 = 18_900,
}

/// Possible values for the number of bits per sample of an audio XA sector
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum XaBitsPerSample {
    /// 4 bits per sample
    S4Bits = 4,
    /// 8 bits per sample
    S8Bits = 8,
}

/// The Submode byte in a Mode 2 XA sub-header (byte 6)
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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

#[test]
fn empty_mode_1() {
    use bcd::Bcd;
    use subchannel::QData;

    // Empty sector dumped from "Les Chevaliers de Baphomet", disc 1, sector 00:02:14
    let expected: [u8; 0x930] = [
        0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0, 0, 0x02, 0x14, 0x01, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0x9e, 0xdc, 0x20, 0x94, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xf7, 0x18, 0xf5,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xbf, 0x79, 0x60, 0xa1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xf5,
        0x0c, 0xf4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x21, 0xa5, 0x40, 0x35, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0x41, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xb3, 0xdd, 0xda,
        0x20, 0x4d, 0x49, 0x24, 0xb4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x97, 0x65,
        0xc5, 0xc2, 0x52, 0xe6, 0, 0x43, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0x93, 0x49, 0x24, 0x5d, 0xb2, 0x05, 0x05, 0x11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0x8f, 0x90, 0xc9, 0xc1, 0x46, 0x12,
    ];

    let format = TrackFormat::Mode1;

    let qdata = QData::Mode1 {
        track: Bcd::TABLE[1],
        index: Bcd::TABLE[0],
        track_msf: Msf::ZERO,
        disc_msf: Msf::from_bcd(0x00, 0x02, 0x14).unwrap(),
    };

    let q = Q::from_qdata_mode1(qdata, ::subchannel::AdrControl::DATA);
    let sector = Sector::empty(q, format).unwrap();

    assert!(sector.edc_valid());

    let data = sector.data_2352();

    assert_eq!(data, &expected);
}

#[test]
fn empty_mode_2_xa_form_1() {
    use bcd::Bcd;
    use subchannel::QData;

    // Empty sector dumped from "Metal Gear Solid", disc 1, sector 00:02:03
    let expected: [u8; 0x930] = [
        0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0, 0, 0x02, 0x03, 0x02, 0,
        0, 0x08, 0, 0, 0, 0x08, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x0b, 0x88, 0x81, 0x94, 0, 0, 0, 0, 0,
        0, 0xfb, 0, 0, 0, 0xfb, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x1d, 0x85, 0x9e, 0xa1, 0, 0, 0,
        0, 0, 0, 0xf3, 0, 0, 0, 0xf3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x16, 0x0d, 0x1f, 0x35, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x9e, 0xa1, 0x8e, 0x61, 0x72, 0xe3, 0x62, 0x23, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xb9, 0, 0xd2, 0, 0xa5, 0, 0x67, 0, 0xa9, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x1f, 0x35, 0x1b, 0x48, 0x70, 0x53,
        0x74, 0x2e, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x42, 0, 0x21, 0, 0x56, 0,
        0x94, 0, 0xa1, 0, 0, 0, 0, 0,
    ];

    let format = TrackFormat::Mode2Xa;

    let qdata = QData::Mode1 {
        track: Bcd::TABLE[1],
        index: Bcd::TABLE[1],
        track_msf: Msf::ZERO,
        disc_msf: Msf::from_bcd(0x00, 0x02, 0x03).unwrap(),
    };

    let q = Q::from_qdata_mode1(qdata, ::subchannel::AdrControl::DATA);

    let sector = Sector::empty(q, format).unwrap();

    assert!(sector.edc_valid());

    let data = sector.data_2352();

    assert_eq!(data, &expected);
}
