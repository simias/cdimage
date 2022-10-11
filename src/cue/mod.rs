//! BIN/CUE image format implementation
//!
//! The CUE sheet format was created for the CDRWIN burning software.
//!
//! The original format was described in the CDRWIN user guide but
//! many extensions and variations exist.
//!
//! The CUE file format does not support multi-session discs

use std::io::SeekFrom;
use std::path::Path;

use internal::IndexCache;
use sector::Sector;
use subchannel::{QData, Q};
use {CdResult, DiscPosition, Image, Toc};

use self::parser::{BinSource, BinaryBlob, CueParser};

mod parser;

/// CUE parser state.
pub struct Cue {
    /// Cache of all the indices in the CD image
    indices: IndexCache<Storage>,
    bin_source: BinSource,
    /// List of all the BIN files referenced in the cue sheet
    bin_files: Vec<BinaryBlob>,
    /// Table of contents
    toc: Toc,
}

impl Cue {
    /// Parse a CUE sheet, open the BIN files and build a `Cue`
    /// instance.
    pub fn new<P: AsRef<Path>>(cue_path: P) -> CdResult<Cue> {
        CueParser::build_cue(cue_path)
    }

    /// Attempt to load a disc image from a ZIP file.
    ///
    /// The parser will look for a file with a `.cue` extension in the archive and load it. All the
    /// referenced `.bin` files will be loaded from the archive lazily.
    ///
    /// If the archive contains several `.cue` files, only the first one will be loaded.
    pub fn new_from_zip<P: AsRef<Path>>(zip_path: P) -> CdResult<Cue> {
        CueParser::build_cue_from_zip(zip_path)
    }
}

impl Image for Cue {
    fn image_format(&self) -> String {
        "CUE".to_string()
    }

    fn read_sector(&mut self, position: DiscPosition) -> CdResult<Sector> {
        let msf = match position {
            DiscPosition::LeadIn(index) => return self.toc.build_toc_sector(index),
            DiscPosition::Program(msf) => msf,
        };

        let (pos, index) = match self.indices.find_index_for_msf(msf) {
            Some(i) => i,
            None => return self.toc.build_lead_out_sector(msf),
        };

        // First we compute the relative track MSF
        let track_msf = if index.is_pregap() {
            // In the pregap the track MSF decreases until index1 is reached
            let index1 = match self.indices.get(pos + 1) {
                Some(i) => i,
                None => panic!("Pregap without index 1!"),
            };

            index1.msf() - msf
        } else {
            // The track MSF is relative to index1
            let index1 = if index.index().bcd() == 0x01 {
                index
            } else {
                match self.indices.find_index01_for_track(index.track()) {
                    Ok((_, i)) => i,
                    // Shouldn't be reached, should be
                    // caught by IndexCache's constructor
                    Err(_e) => panic!("Missing index 1 for track {}", index.track()),
                }
            };

            msf - index1.msf()
        };

        let qdata = QData::Mode1 {
            track: index.track(),
            index: index.index(),
            track_msf,
            disc_msf: msf,
        };

        let ctrl = index.control();
        let format = index.format();

        let q = Q::from_qdata_mode1(qdata, ctrl);

        // First let's read the sector data
        let sector = match index.private() {
            Storage::Bin(bin, offset, ty) => {
                let bin = &mut self.bin_files[*bin as usize];

                // For now we only support "simple sector" format
                if ty.sector_size() != 2352 {
                    panic!("Unimplemented CUE track type: {:?}", ty);
                }

                let index_offset =
                    ty.sector_size() as u64 * (msf.sector_index() - index.sector_index()) as u64;

                let offset = offset + index_offset;

                let mut sector = Sector::uninitialized(q, format)?;

                self.bin_source.read_exact_from(
                    bin,
                    SeekFrom::Start(offset),
                    sector.data_2352_mut(),
                )?;

                sector
            }
            Storage::PreGap => {
                // We don't have data for this track, leave it empty
                Sector::empty(q, format)?
            }
        };

        Ok(sector)
    }

    fn toc(&self) -> &Toc {
        &self.toc
    }
}

/// Possible types for a CUE track.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum CueTrackType {
    /// CD-DA audio track (red book audio)
    Audio,
    /// CD-ROM Mode1/2048 (only data, no header or ECC/EDC)
    Mode1Data,
    /// CD-ROM Mode1/2352
    Mode1Raw,
    /// CD-ROM XA Mode2/2336 (without the 16byte header)
    Mode2Headerless,
    /// CD-ROM XA Mode2/2352
    Mode2Raw,
    /// CD-I Mode2/2336 (without the 16byte header)
    CdIHeaderless,
    /// CD-I Mode2/2352
    CdIRaw,
}

impl CueTrackType {
    fn sector_size(self) -> u16 {
        match self {
            CueTrackType::Audio => 2352,
            CueTrackType::Mode1Data => 2048,
            CueTrackType::Mode1Raw => 2336,
            CueTrackType::Mode2Headerless => 2336,
            CueTrackType::Mode2Raw => 2352,
            CueTrackType::CdIHeaderless => 2336,
            CueTrackType::CdIRaw => 2352,
        }
    }
}

/// Storage for a slice
enum Storage {
    /// The slice is stored in a portion of a BIN file. Contains the
    /// index of the BIN file and the offset in the file.
    Bin(u32, u64, CueTrackType),
    /// The slice is a pre-gap, it's not stored in the BIN file and
    /// must be regererated.
    PreGap,
}

/// Max size for a cue sheet, used to detect bogus input early without attempting to load a huge
/// file to RAM. Cue sheets bigger than this will be rejected.
pub const CUE_SHEET_MAX_LENGTH: u64 = 1024 * 1024;
