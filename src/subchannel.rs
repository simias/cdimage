//! Subchannel data interface.
//!
//! The subchannel data (sometimes called subcode or control bytes) is
//! stored alongside each sector on the CD. There are 8 subchannels
//! named P, Q, R, S, T, U, V and W. Each of them contain 12 bytes of
//! data per sector for a total of 96bytes of subchannel data per
//! sector.
//!
//! Subchannels generally contain "metadata" about the current sector
//! such as timing information, track name or even some low resolution
//! graphics in certain standards. It also contains the table of
//! contents of the disc in the lead-in area (in the Q subchannel).
//!
//! The subchannel data is not protected by the error correction code
//! in CD-ROMs so it's more likely to be corrupted than regular data.
//!
//! Some platforms abuse the subchannels for copy-protection since
//! many drives and image formats fail to reproduce the subchannel
//! data correctly.
//!
//! For instance libcrypt on the PlayStation stores a crypto key by
//! purposefully corrupting the SubChannelQ CRC of a few sectors at
//! some known location on the disc. If one attempts to copy the disc
//! the Q subchannel must be preserved exactly, if the software or the
//! hardware fails to copy or corrects the corrupted data the game
//! won't work.
//!
//! For more details see section 22 of [ECMA-130]
//! (http://www.ecma-international.org/publications/files/ECMA-ST/Ecma-130.pdf)
//! and [Wikipedia's article on the subject]
//! (https://en.wikipedia.org/wiki/Compact_Disc_subcode)

use bcd::Bcd;
use msf::Msf;

use {crc, CdError, CdResult, SessionFormat, TrackFormat};

/// Full contents of a Q subchannel frame, parsed. From this structure we should be able to
/// regenerate the raw Subchannel Q data losslessly
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Q {
    /// Decoded payload
    data: QData,
    /// ADR/Control byte
    adr_control: AdrControl,
}

impl Q {
    /// Generate a Q from the given QData and track format
    pub fn from_qdata(data: QData, format: TrackFormat) -> Q {
        let adr_control = if format.is_audio() {
            AdrControl::MODE1_AUDIO
        } else {
            AdrControl::MODE1_DATA
        };

        Q { data, adr_control }
    }

    /// Generate a Q from raw subchannel Q data
    pub fn from_raw(raw: [u8; 12]) -> CdResult<Q> {
        let adr_control = AdrControl(raw[0]);
        let data = QData::from_raw(raw)?;

        Ok(Q { data, adr_control })
    }

    /// Generate a Q from raw interleaved subchannel data (this is what you get from a raw_rw dump
    /// in cdrdao for instance)
    pub fn from_raw_interleaved(raw: [u8; 96]) -> CdResult<Q> {
        let mut subq = [0u8; 12];

        for (bit, &r) in raw.iter().enumerate() {
            // Subchannel Q is in bit 7
            let v = (r & 0x40) != 0;

            if !v {
                continue;
            }

            subq[bit / 8] |= 1 << (7 - (bit & 7));
        }

        Q::from_raw(subq)
    }

    /// Generate the raw representation of this Q subchannel data
    pub fn to_raw(&self) -> [u8; 12] {
        self.data.to_raw(self.adr_control)
    }

    /// Returns true if this is a data sector
    pub fn is_data(&self) -> bool {
        self.adr_control.is_data()
    }

    /// Returns true if this is an audio sector
    pub fn is_audio(&self) -> bool {
        self.adr_control.is_audio()
    }

    /// Returns the parsed `QData`
    pub fn data(&self) -> &QData {
        &self.data
    }
}

/// Possible contents of the Q subchannel data depending on the mode.
///
/// See section 22.3.2 of ECMA-130 for more details.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QData {
    /// Mode 1 data in the user data area and the lead-out area:
    Mode1 {
        /// Current track number
        track: Bcd,
        /// Index within the track
        index: Bcd,
        /// MSF within the track. If we're in the pregap this value decreases until it reaches
        /// INDEX01
        track_msf: Msf,
        /// Absolute MSF
        disc_msf: Msf,
    },
    /// Mode 1 Table of content entry (in the lead-in):
    Mode1Toc {
        /// Which track this ToC entry is for
        track: Bcd,
        /// Address of the track's INDEX01
        index1_msf: Msf,
        /// MSF for this ToC entry in the lead-in. Normally ignored.
        lead_in_msf: Msf,
    },
    /// Mode 1 Table of content entry with pointer set to 0xa0:
    Mode1TocFirstTrack {
        /// Number of the first track (usually 01)
        first_track: Bcd,
        /// Format of the session
        session_format: SessionFormat,
        /// MSF for this ToC entry in the lead-in. Normally ignored.
        lead_in_msf: Msf,
    },
    /// Mode 1 Table of content entry with pointer set to 0xa1:
    Mode1TocLastTrack {
        /// Number of the last track
        last_track: Bcd,
        /// MSF for this ToC entry in the lead-in. Normally ignored.
        lead_in_msf: Msf,
    },
    /// Mode 1 Table of content entry with pointer set to 0xa2:
    Mode1TocLeadOut {
        /// Absolute MSF of the first sector of the lead-out
        lead_out_start: Msf,
        /// MSF for this ToC entry in the lead-in. Normally ignored.
        lead_in_msf: Msf,
    },
}

impl QData {
    /// Create a QData from raw subchannel Q data
    pub fn from_raw(raw: [u8; 12]) -> CdResult<QData> {
        let crc = crc::crc16(&raw[..10]);

        if crc.to_be_bytes() != raw[10..12] {
            return Err(CdError::InvalidSubQCRC);
        }

        let adr_ctrl = AdrControl(raw[0]);

        if adr_ctrl.mode() != 1 {
            // We might want to add Mode2 and Mode3 support here at
            // some point. For the time being only Mode1 is supported.
            return Err(CdError::Unsupported);
        }

        let track = match Bcd::from_bcd(raw[1]) {
            Some(b) => b,
            None => return Err(CdError::Unsupported),
        };

        let min = match Bcd::from_bcd(raw[3]) {
            Some(b) => b,
            None => return Err(CdError::Unsupported),
        };

        let sec = match Bcd::from_bcd(raw[4]) {
            Some(b) => b,
            None => return Err(CdError::Unsupported),
        };

        let frac = match Bcd::from_bcd(raw[5]) {
            Some(b) => b,
            None => return Err(CdError::Unsupported),
        };

        let msf = match Msf::new(min, sec, frac) {
            Some(m) => m,
            None => return Err(CdError::Unsupported),
        };

        let zero = raw[6];
        if zero != 0 {
            return Err(CdError::Unsupported);
        }

        let ap_min = match Bcd::from_bcd(raw[7]) {
            Some(b) => b,
            None => return Err(CdError::Unsupported),
        };

        let ap_sec = match Bcd::from_bcd(raw[8]) {
            Some(b) => b,
            None => return Err(CdError::Unsupported),
        };

        let ap_frac = match Bcd::from_bcd(raw[9]) {
            Some(b) => b,
            None => return Err(CdError::Unsupported),
        };

        let ap_msf = match Msf::new(ap_min, ap_sec, ap_frac) {
            Some(m) => m,
            None => return Err(CdError::Unsupported),
        };

        let d = if track.bcd() == 0 {
            // We're in the lead-in, this is a TOC entry
            let pointer = raw[2];

            match pointer {
                0xa0 => {
                    let format = match ap_sec.bcd() {
                        0x00 => SessionFormat::CdDaCdRom,
                        0x10 => SessionFormat::Cdi,
                        0x20 => SessionFormat::CdXa,
                        _ => return Err(CdError::Unsupported),
                    };

                    if ap_frac.bcd() != 0 {
                        return Err(CdError::Unsupported);
                    }

                    QData::Mode1TocFirstTrack {
                        first_track: ap_min,
                        session_format: format,
                        lead_in_msf: msf,
                    }
                }
                0xa1 => {
                    if ap_sec.bcd() != 0 || ap_frac.bcd() != 0 {
                        return Err(CdError::Unsupported);
                    }

                    QData::Mode1TocLastTrack {
                        last_track: ap_min,
                        lead_in_msf: msf,
                    }
                }
                0xa2 => QData::Mode1TocLeadOut {
                    lead_out_start: ap_msf,
                    lead_in_msf: msf,
                },
                _ => {
                    let ptrack = match Bcd::from_bcd(pointer) {
                        Some(b) => b,
                        None => return Err(CdError::Unsupported),
                    };

                    QData::Mode1Toc {
                        track: ptrack,
                        index1_msf: ap_msf,
                        lead_in_msf: msf,
                    }
                }
            }
        } else {
            let index = match Bcd::from_bcd(raw[2]) {
                Some(b) => b,
                None => return Err(CdError::Unsupported),
            };

            QData::Mode1 {
                track,
                index,
                track_msf: msf,
                disc_msf: ap_msf,
            }
        };

        Ok(d)
    }

    /// Generate the raw representation of this Q subchannel data
    pub fn to_raw(&self, adr_ctrl: AdrControl) -> [u8; 12] {
        let mut subq = [0u8; 12];

        subq[0] = adr_ctrl.0;

        match self {
            QData::Mode1 {
                track,
                index,
                track_msf,
                disc_msf,
            } => {
                subq[1] = track.bcd();
                subq[2] = index.bcd();

                let (m, s, f) = track_msf.into_bcd();

                subq[3] = m.bcd();
                subq[4] = s.bcd();
                subq[5] = f.bcd();

                let (m, s, f) = disc_msf.into_bcd();
                subq[7] = m.bcd();
                subq[8] = s.bcd();
                subq[9] = f.bcd();
            }
            QData::Mode1Toc {
                track,
                index1_msf,
                lead_in_msf,
            } => {
                subq[2] = track.bcd();

                let (m, s, f) = lead_in_msf.into_bcd();

                subq[3] = m.bcd();
                subq[4] = s.bcd();
                subq[5] = f.bcd();

                let (m, s, f) = index1_msf.into_bcd();

                subq[7] = m.bcd();
                subq[8] = s.bcd();
                subq[9] = f.bcd();
            }
            QData::Mode1TocFirstTrack {
                first_track,
                session_format,
                lead_in_msf,
            } => {
                subq[2] = 0xa0;

                let (m, s, f) = lead_in_msf.into_bcd();

                subq[3] = m.bcd();
                subq[4] = s.bcd();
                subq[5] = f.bcd();

                subq[7] = first_track.bcd();
                subq[8] = match session_format {
                    SessionFormat::CdDaCdRom => 0,
                    SessionFormat::Cdi => 0x10,
                    SessionFormat::CdXa => 0x20,
                };
            }
            QData::Mode1TocLastTrack {
                last_track,
                lead_in_msf,
            } => {
                subq[2] = 0xa1;

                let (m, s, f) = lead_in_msf.into_bcd();

                subq[3] = m.bcd();
                subq[4] = s.bcd();
                subq[5] = f.bcd();

                subq[7] = last_track.bcd();
            }
            QData::Mode1TocLeadOut {
                lead_out_start,
                lead_in_msf,
            } => {
                subq[2] = 0xa2;

                let (m, s, f) = lead_in_msf.into_bcd();

                subq[3] = m.bcd();
                subq[4] = s.bcd();
                subq[5] = f.bcd();

                let (m, s, f) = lead_out_start.into_bcd();

                subq[7] = m.bcd();
                subq[8] = s.bcd();
                subq[9] = f.bcd();
            }
        }

        let crc = crc::crc16(&subq[..10]).to_be_bytes();

        subq[10] = crc[0];
        subq[11] = crc[1];

        subq
    }
}

/// The first byte of subchannel Q data, containing the mode and various attributes
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AdrControl(u8);

impl AdrControl {
    /// A Mode1 audio AdrControl with no other attribute set
    pub const MODE1_AUDIO: AdrControl = AdrControl(0x01);

    /// A Mode1 data AdrControl with no other attribute set
    pub const MODE1_DATA: AdrControl = AdrControl(0x41);

    /// Return true if this is a data track. For table of content
    /// sectors this flag applies to the target track.
    pub fn is_data(&self) -> bool {
        self.0 & 0x40 != 0
    }

    /// Return true if this is an audio track. For table of content
    /// sectors this flag applies to the target track.
    pub fn is_audio(&self) -> bool {
        !self.is_data()
    }

    /// Return true if the "digital copy permitted" flag is set. For
    /// table of content sectors this flag applies to the target
    /// track.
    pub fn is_digital_copy_permitted(&self) -> bool {
        self.0 & 0x20 != 0
    }

    /// Return true if this is an audio track and pre-emphasis is
    /// enabled.
    ///
    /// For more informations on pre-emphasis check out
    /// http://wiki.hydrogenaud.io/index.php?title=Pre-emphasis
    pub fn pre_emphasis(&self) -> bool {
        self.is_audio() && (self.0 & 0x10 != 0)
    }

    /// Return true if this is a 4-channel audio track. The vast
    /// majority of audio CDs are 2-channel (stereo).
    pub fn four_channel_audio(&self) -> bool {
        self.is_audio() && (self.0 & 0x80 != 0)
    }

    /// Retrieve the mode of the data specified by this
    /// Q-subchannel.
    ///
    /// The Q subchannel has several modes (see section 5.4.3 of
    /// ECMA-395). Mode 1 is used to store the table of content in the
    /// lead-in and timing information elsewhere.
    ///
    /// This field is specified over 4 bits so theoretically 16
    /// different modes are possible.
    pub fn mode(&self) -> u8 {
        self.0 & 0xf
    }
}

#[test]
fn adr_control_attrs() {
    assert!(AdrControl::MODE1_AUDIO.is_audio());
    assert!(!AdrControl::MODE1_AUDIO.is_data());
    assert_eq!(AdrControl::MODE1_AUDIO.mode(), 1);

    assert!(!AdrControl::MODE1_DATA.is_audio());
    assert!(AdrControl::MODE1_DATA.is_data());
    assert_eq!(AdrControl::MODE1_DATA.mode(), 1);
}

#[test]
fn subq_raw_rw() {
    // Random Metal Gear Solid 1 raw subchannel data dumped with cdrdao
    let raw_rw: [[u8; 96]; 3] = [
        [
            0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x40,
            0x00, 0x40, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40,
            0x00, 0x00, 0x00, 0x40, 0x00, 0x40, 0x00, 0x40, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00,
            0x40, 0x40, 0x40, 0x40, 0x40, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x40,
        ],
        [
            0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x40,
            0x00, 0x40, 0x00, 0x40, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40,
            0x00, 0x00, 0x00, 0x40, 0x00, 0x40, 0x00, 0x40, 0x00, 0x40, 0x00, 0x00, 0x40, 0x40,
            0x00, 0x40, 0x00, 0x40, 0x40, 0x40, 0x40, 0x00, 0x00, 0x00, 0x00, 0x40,
        ],
        [
            0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x40,
            0x00, 0x00, 0x00, 0x00, 0xb3, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x40,
            0x00, 0x40, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40,
            0x00, 0x00, 0x00, 0x40, 0x00, 0x40, 0x00, 0x40, 0x00, 0x00, 0x40, 0x40, 0x40, 0x40,
            0x40, 0x00, 0x40, 0x40, 0x40, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x40,
        ],
    ];

    for raw in raw_rw.iter() {
        // Extract subchannel Q from the raw data
        let mut subq = [0u8; 12];
        for (bit, &r) in raw.iter().enumerate() {
            // Subchannel Q is in bit 7
            let v = (r & 0x40) != 0;

            if !v {
                continue;
            }

            subq[bit / 8] |= 1 << (7 - (bit & 7));
        }

        let q = Q::from_raw_interleaved(*raw).unwrap();
        let qr = Q::from_raw(subq).unwrap();

        assert_eq!(q, qr);

        let subq_generated = q.to_raw();
        assert_eq!(subq, subq_generated)
    }
}
