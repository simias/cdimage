//! BIN/CUE image format implementation
//!
//! The CUE sheet format was created for the CDRWIN burning software.
//!
//! The original format was described in the CDRWIN user guide but
//! many extensions and variations exist.
//!
//! The CUE file format does not support multi-session discs

use std::path::Path;
use std::fs::File;

use CdError;
use Image;
use internal::IndexCache;

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
    Bin(u32, u64),
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
    /// Current position within the file, used to avoid needless
    /// seeks.
    pos: u64,
}

impl BinaryBlob {
    fn new(path: &Path) -> Result<BinaryBlob, CdError> {

        let file =
            match File::open(path) {
                Ok(f) => f,
                Err(e) => return Err(CdError::IoError(e)),
            };

        Ok(BinaryBlob {
            file: file,
            pos: 0
        })
    }
}

/// Max size for a cue sheet, used to detect bogus input early without
/// attempting to load a huge file to RAM. Cue sheets bigger than this
/// will be rejected.
pub const CUE_SHEET_MAX_LENGTH: u64 = 1024 * 1024;
