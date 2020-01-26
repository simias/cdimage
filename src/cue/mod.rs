//! BIN/CUE image format implementation
//!
//! The CUE sheet format was created for the CDRWIN burning software.
//!
//! The original format was described in the CDRWIN user guide but
//! many extensions and variations exist.
//!
//! The CUE file format does not support multi-session discs

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use bcd::Bcd;
use internal::IndexCache;
use msf::Msf;
use sector::{Metadata, Sector, SectorBuilder};
use CdError;
use Image;

use self::parser::CueParser;

mod parser;

/// CUE parser state.
#[derive(Debug)]
pub struct Cue {
    /// Cache of all the indices in the CD image
    indices: IndexCache<Storage>,
    /// List of all the BIN files referenced in the cue sheet
    bin_files: Vec<BinaryBlob>,
}

impl Cue {
    /// Parse a CUE sheet, open the BIN files and build a `Cue`
    /// instance.
    pub fn new(cue_path: &Path) -> Result<Cue, CdError> {
        CueParser::build_cue(cue_path)
    }
}

impl Image for Cue {
    fn image_format(&self) -> String {
        "CUE".to_string()
    }

    fn read_sector(&mut self, sector: &mut Sector, msf: Msf) -> Result<(), CdError> {
        let (pos, index) = match self.indices.find_index_for_msf(msf) {
            Some(i) => i,
            None => return Err(CdError::LeadOut),
        };

        // First we compute the relative track MSF
        let track_msf = if index.is_pregap() {
            // In the pregap the track MSF decreases until index1
            // is reached
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
                    Err(_) => panic!("Missing index 1 for track {}", index.track()),
                }
            };

            msf - index1.msf()
        };

        let mut builder = SectorBuilder::new(sector);

        // First let's read the sector data
        match index.private() {
            &Storage::Bin(bin, offset, ty) => {
                let bin = &mut self.bin_files[bin as usize];

                // For now we only support "simple sector" format
                if ty.sector_size() != 2352 {
                    panic!("Unimplemented CUE track type: {:?}", ty);
                }

                let index_offset =
                    ty.sector_size() as u64 * (msf.sector_index() - index.sector_index()) as u64;

                let offset = offset + index_offset;

                let res = builder.set_data_2352(|data| {
                    bin.file.seek(SeekFrom::Start(offset))?;

                    bin.file.read_exact(data)
                });

                if let Err(e) = res {
                    return Err(CdError::IoError(e));
                }
            }
            &Storage::PreGap => panic!("Unhandled CUE pregap read"),
        }

        // Now let's fill up the metadata
        builder.set_metadata(Metadata {
            msf: msf,
            track_msf: track_msf,
            index: index.index(),
            track: index.track(),
            format: index.format(),
            session: index.session(),
        });

        Ok(())
    }

    fn track_msf(&self, track: Bcd, track_msf: Msf) -> Result<Msf, CdError> {
        self.indices.track_msf(track, track_msf)
    }
}

/// Possible types for a CUE track.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum CueTrackType {
    /// CD-DA audio track (red book audio)
    Audio,
    /// CD+G (CD+Graphics) track (with subchannel data)
    CdG,
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
            CueTrackType::CdG => 2448,
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

/// `BinaryBlob` can contain one or several slices interrupted by pre-
/// and post-gaps.
#[derive(Debug)]
struct BinaryBlob {
    /// BIN file
    file: File,
}

impl BinaryBlob {
    fn new(path: &Path) -> Result<BinaryBlob, CdError> {
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) => return Err(CdError::IoError(e)),
        };

        Ok(BinaryBlob { file: file })
    }
}

/// Max size for a cue sheet, used to detect bogus input early without
/// attempting to load a huge file to RAM. Cue sheets bigger than this
/// will be rejected.
pub const CUE_SHEET_MAX_LENGTH: u64 = 1024 * 1024;
