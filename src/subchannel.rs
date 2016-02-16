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

use SessionFormat;

/// Common interface shared by all subchannels
pub trait SubChannel {
    /// Return the raw 12 bytes of subchannel data
    fn raw(&self) -> &[u8; 12];
}

/// This struct contains the Subchannel P data of one sector.
///
/// This subchannel indicates the beginning of an information
/// track. All bits of the p-channel of a Section should be set to the
/// same value (per the standard).
///
/// This channel is generally ignored in favor of the Q subchannel.
///
/// See section 22.2 of ECMA-130 for more informations.
pub struct SubChannelP {
    /// Raw contents
    bytes: [u8; 12],
}

impl SubChannelP {
    /// Create a SubChannelP instance from 12 bytes of subchannel
    /// data.
    pub fn new(raw: [u8; 12]) -> SubChannelP {
        SubChannelP {
            bytes: raw,
        }
    }

    /// Return true if all the bits of the channel are set to the same
    /// value as the standard mandates
    pub fn valid(&self) -> bool {
        if self.bytes[0] != 0 && self.bytes[0] != 0xff {
            return false;
        }

        for i in 1..12 {
            if self.bytes[i] != self.bytes[i - 1] {
                return false;
            }
        }

        true
    }
}

impl SubChannel for SubChannelP {
    fn raw(&self) -> &[u8; 12] {
        &self.bytes
    }
}

/// This struct contains the Subchannel Q data of one sector.
pub struct SubChannelQ {
    /// Raw contents
    bytes: [u8; 12],
}

impl SubChannelQ {
    /// Create a SubChannelQ instance from 12 bytes of subchannel
    /// data.
    pub fn new(raw: [u8; 12]) -> SubChannelQ {
        SubChannelQ {
            bytes: raw,
        }
    }

    /// Return true if this is a data track. For table of content
    /// sectors this flag applies to the target track.
    pub fn data(&self) -> bool {
        self.bytes[0] & 0x40 != 0
    }

    /// Return true if this is an audio track. For table of content
    /// sectors this flag applies to the target track.
    pub fn audio(&self) -> bool {
        !self.data()
    }

    /// Return true if the "digital copy permitted" flag is set. For
    /// table of content sectors this flag applies to the target
    /// track.
    pub fn digital_copy_permitted(&self) -> bool {
        self.bytes[0] & 0x20 != 0
    }

    /// Return true if this is an audio track and pre-emphasis is
    /// enabled.
    ///
    /// For more informations on pre-emphasis check out
    /// http://wiki.hydrogenaud.io/index.php?title=Pre-emphasis
    pub fn pre_emphasis(&self) -> bool {
        self.audio() && (self.bytes[0] & 0x10 != 0)
    }

    /// Return true if this is a 4-channel audio track. The vast
    /// majority of audio CDs are 2-channel (stereo).
    pub fn four_channel_audio(&self) -> bool {
        self.audio() && (self.bytes[0] & 0x80 != 0)
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
        self.bytes[0] & 0xf
    }

    /// Return the 16bit CRC stored at the end of the subchannel data.
    pub fn crc(&self) -> u16 {
        let msb = self.bytes[10] as u16;
        let lsb = self.bytes[11] as u16;

        (msb << 8) | lsb
    }

    /// Parse the contents of this subchannel and return it as a
    /// `QData`. This method does not validate the sector's CRC but
    /// will return `QData::Unsupported` if it encounters a format
    /// error.
    pub fn parse_data(&self) -> QData {
        if self.mode() != 1 {
            // We might want to add Mode2 and Mode3 support here at
            // some point. For the time being only Mode1 is supported.
            return QData::Unsupported
        }

        let track =
            match Bcd::from_bcd(self.bytes[1]) {
                Some(b) => b,
                None => return QData::Unsupported,
            };

        let min =
            match Bcd::from_bcd(self.bytes[3]) {
                Some(b) => b,
                None => return QData::Unsupported,
            };

        let sec =
            match Bcd::from_bcd(self.bytes[4]) {
                Some(b) => b,
                None => return QData::Unsupported,
            };

        let frac =
            match Bcd::from_bcd(self.bytes[5]) {
                Some(b) => b,
                None => return QData::Unsupported,
            };

        let msf =
            match Msf::new(min, sec, frac) {
                Some(m) => m,
                None => return QData::Unsupported,
            };

        let zero = self.bytes[6];
        if zero != 0 {
            return QData::Unsupported;
        }

        let ap_min =
            match Bcd::from_bcd(self.bytes[7]) {
                Some(b) => b,
                None => return QData::Unsupported,
            };

        let ap_sec =
            match Bcd::from_bcd(self.bytes[8]) {
                Some(b) => b,
                None => return QData::Unsupported,
            };

        let ap_frac =
            match Bcd::from_bcd(self.bytes[9]) {
                Some(b) => b,
                None => return QData::Unsupported,
            };

        let ap_msf =
            match Msf::new(ap_min, ap_sec, ap_frac) {
                Some(m) => m,
                None => return QData::Unsupported,
            };

        if track.bcd() == 0 {
            // We're in the lead-in, this is a TOC entry
            let pointer = self.bytes[2];

            match pointer {
                0xa0 => {
                    let format =
                        match ap_sec.bcd() {
                            0x00 => SessionFormat::CddaCdRom,
                            0x10 => SessionFormat::Cdi,
                            0x20 => SessionFormat::Cdxa,
                            _ => return QData::Unsupported,
                        };

                    if ap_frac.bcd() != 0 {
                        return QData::Unsupported;
                    }

                    QData::Mode1TocFirstTrack(ap_min, format, msf)
                }
                0xa1 => {
                    if ap_frac.bcd() != 0 || ap_sec.bcd() != 0 {
                        return QData::Unsupported;
                    }

                    QData::Mode1TocLastTrack(ap_min, msf)
                }
                0xa2 => {
                    QData::Mode1TocLeadOut(ap_msf, msf)
                }
                _ => {
                    let ptrack =
                        match Bcd::from_bcd(pointer) {
                            Some(b) => b,
                            None => return QData::Unsupported,
                        };

                    QData::Mode1Toc(ptrack, ap_msf, msf)
                }
            }
        } else {
            let index = match Bcd::from_bcd(self.bytes[2]) {
                Some(b) => b,
                None => return QData::Unsupported,
            };

            QData::Mode1(track, index, msf, ap_msf)
        }
    }
}

impl SubChannel for SubChannelQ {
    fn raw(&self) -> &[u8; 12] {
        &self.bytes
    }
}

/// Possible contents of the Q subchannel data depending on the
/// mode.
///
/// See section 22.3.2 of ECMA-130 for more details.
pub enum QData {
    /// Mode 1 data in the user data area and the lead-out area:
    ///
    /// * Track number
    /// * Index
    /// * MSF of this sector relative to the beginning of the track
    ///   (index 01). In the prepap (index 00) it decreases until it
    ///   reaches index 01 at 00:00:00.
    /// * MSF of this sector relative to the beginning of the user
    ///   data area. This is *not* an absolute MSF, you have to add 2
    ///   minutes (150 sectors) to get an absolute MSF.
    Mode1(Bcd, Bcd, Msf, Msf),
    /// Mode 1 Table of content entry (in the lead-in):
    ///
    /// * Track number pointer
    /// * Absolute MSF of Index 00 the track designed by the pointer
    /// * MSF of this TOC entry in the lead-in
    Mode1Toc(Bcd, Msf, Msf),
    /// Mode 1 Table of content entry with pointer set to 0xa0:
    ///
    /// * Track number of the first track in the user area
    /// * Session format (stored in the p-sec field)
    /// * MSF of this TOC entry in the lead-in
    Mode1TocFirstTrack(Bcd, SessionFormat, Msf),
    /// Mode 1 Table of content entry with pointer set to 0xa1:
    ///
    /// * Track number of the last track in the user area
    /// * MSF of this TOC entry in the lead-in
    Mode1TocLastTrack(Bcd, Msf),
    /// Mode 1 Table of content entry with pointer set to 0xa2:
    ///
    /// * Absolute MSF of the lead-out track
    /// * MSF of this TOC entry in the lead-in
    Mode1TocLeadOut(Msf, Msf),
    /// Unsupported or corrupted data. Use `Subchannel::raw()` if you
    /// want to access the raw data directly for further processing.
    Unsupported,
}

/// This struct is used for subchannels where no special handling is
/// implemented.
pub struct SubChannelBasic {
    /// Raw contents
    bytes: [u8; 12],
}

impl SubChannelBasic {
    /// Create a SubChannelBasic instance from 12 bytes of subchannel
    /// data.
    pub fn new(raw: [u8; 12]) -> SubChannelQ {
        SubChannelQ {
            bytes: raw,
        }
    }
}

impl SubChannel for SubChannelBasic {
    fn raw(&self) -> &[u8; 12] {
        &self.bytes
    }
}

/// This struct contains the Subchannel R data for one sector.
pub type SubChannelR = SubChannelBasic;
/// This struct contains the Subchannel S data for one sector.
pub type SubChannelS = SubChannelBasic;
/// This struct contains the Subchannel T data for one sector.
pub type SubChannelT = SubChannelBasic;
/// This struct contains the Subchannel U data for one sector.
pub type SubChannelU = SubChannelBasic;
/// This struct contains the Subchannel V data for one sector.
pub type SubChannelV = SubChannelBasic;
/// This struct contains the Subchannel W data for one sector.
pub type SubChannelW = SubChannelBasic;
