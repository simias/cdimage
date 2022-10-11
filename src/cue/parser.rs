use super::{Cue, CueTrackType, Storage, CUE_SHEET_MAX_LENGTH};
use bcd::Bcd;
use internal::{Index, IndexCache};
use msf::Msf;
use std::fs::{metadata, File};
use std::io;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use subchannel::AdrControl;
use zip::ZipArchive;
use CdError;
use CdResult;
use TrackFormat;

pub struct CueParser {
    /// Path to the cue sheet
    ///
    /// This may not be an valid path on the filesystem, for instance referencing archive formats.
    /// It's used for better error reporting. `bin_source` should be used to figure out where the
    /// cue file is located
    cue_path: PathBuf,
    /// The place where bin files should be loaded from
    bin_source: BinSource,
    /// Position within the buffer
    pos: usize,
    /// Current line in the buffer
    line: u32,
    /// Current absolute MSF
    msf: Msf,
    /// List of BIN files
    bin_files: Vec<BinaryBlob>,
    /// Length of the current BIN file in bytes
    bin_len: u64,
    /// Bytes consumed from the current BIN file
    consumed_bytes: u64,
    /// MSF of the last generated index into the file (00:00:00 is the
    /// beginning of the current BIN file, per CUE convention)
    index_msf: Msf,
    /// Type of the last generated index
    index_type: Option<CueTrackType>,
    /// Current Track
    track: Option<(Bcd, CueTrackType, TrackFormat, AdrControl)>,
    /// Indices
    indices: Vec<Index<Storage>>,
}

impl CueParser {
    /// Parse a CUE sheet, open the BIN files and generate the CD
    /// structure
    pub fn build_cue<P: AsRef<Path>>(cue_path: P) -> CdResult<Cue> {
        let cue_path = cue_path.as_ref();
        let cue_sheet = match read_file(cue_path, CUE_SHEET_MAX_LENGTH) {
            Ok(c) => c,
            Err(e) => return Err(CdError::IoError(e)),
        };

        let cue_path = PathBuf::from(cue_path);
        let dir = match cue_path.parent() {
            Some(p) => p.to_path_buf(),
            // Probably shouldn't happen
            None => cue_path.clone(),
        };

        let bin_source = BinSource::Fs(dir);

        CueParser::do_parse(cue_path, bin_source, &cue_sheet)
    }

    fn do_parse(cue_path: PathBuf, bin_source: BinSource, cue_sheet: &[u8]) -> CdResult<Cue> {
        let mut parser = CueParser {
            cue_path,
            bin_source,
            pos: 0,
            line: 0,
            // CUE always skips track 01's pregap (and assumes it's 2
            // seconds long) so we start at index 01.
            msf: Msf::from_sector_index(150).unwrap(),
            bin_files: Vec::new(),
            bin_len: 0,
            consumed_bytes: 0,
            index_type: None,
            index_msf: Msf::ZERO,
            track: None,
            indices: Vec::new(),
        };

        parser.parse(cue_sheet)?;

        let indices = IndexCache::new(parser.cue_path, parser.indices, parser.msf)?;
        let toc = indices.toc()?;

        Ok(Cue {
            indices,
            bin_source: parser.bin_source,
            bin_files: parser.bin_files,
            toc,
        })
    }

    pub fn build_cue_from_zip<P: AsRef<Path>>(zip_path: P) -> CdResult<Cue> {
        let zip_path = zip_path.as_ref();
        let archive = File::open(zip_path)?;
        let mut zip = ZipArchive::new(archive)?;

        for i in 0..zip.len() {
            let mut f = match zip.by_index(i) {
                Ok(f) => f,
                Err(_) => continue,
            };

            // Technically since we're not extracting anything we could use f.name() directly
            // without danger, but we may as well reject wonky archives
            let p = match f.enclosed_name() {
                Some(p) => p,
                None => continue,
            };

            if !matches!(
                p.extension().and_then(|ext| ext.to_str()),
                Some("cue") | Some("CUE")
            ) {
                continue;
            }

            let mut cue_path = zip_path.to_path_buf();
            cue_path.push(p);

            if f.size() > CUE_SHEET_MAX_LENGTH {
                return Err(CdError::BadImage {
                    path: cue_path,
                    desc: "CUE sheet is too big".to_string(),
                });
            }

            let mut cue_sheet = Vec::with_capacity(f.size() as usize);

            f.read_to_end(&mut cue_sheet)?;

            drop(f);

            let bin_source = BinSource::Zip {
                zip,
                path: zip_path.to_path_buf(),
            };

            return CueParser::do_parse(cue_path, bin_source, &cue_sheet);
        }

        Err(CdError::BadImage {
            path: zip_path.to_path_buf(),
            desc: "No CUE file found in archive".to_string(),
        })
    }

    fn error(&self, msg: String) -> CdError {
        CdError::ParseError {
            path: self.cue_path.clone(),
            line: self.line,
            desc: msg,
        }
    }

    fn error_str(&self, msg: &str) -> CdError {
        self.error(msg.to_string())
    }

    fn parse(&mut self, cue_sheet: &[u8]) -> CdResult<()> {
        while let Some((new_pos, buf)) = next_line(cue_sheet, self.pos) {
            self.pos = new_pos;
            self.line += 1;

            let params = self.split(buf)?;

            if params.is_empty() {
                // Empty line
                continue;
            }

            let command = params[0];

            type Callback = fn(&mut CueParser, &[&[u8]]) -> CdResult<()>;

            let handlers: [(&'static [u8], Callback, Option<u32>); 6] = [
                (b"REM", CueParser::command_rem, None),
                (b"FILE", CueParser::command_file, Some(2)),
                (b"TRACK", CueParser::command_track, Some(2)),
                (b"PREGAP", CueParser::command_pregap, Some(1)),
                (b"INDEX", CueParser::command_index, Some(2)),
                (b"FLAGS", CueParser::command_flags, None),
            ];

            let callback = handlers.iter().find(|&&(name, _, _)| name == command);

            match callback {
                Some(&(_, c, nparams)) => {
                    if let Some(nparams) = nparams {
                        if params.len() - 1 != nparams as usize {
                            let command = String::from_utf8_lossy(command);

                            let error = format!(
                                "Wrong number of parameters \
                                 for command {}: expected \
                                 {} got {}",
                                command,
                                nparams,
                                params.len() - 1
                            );

                            return Err(self.error(error));
                        }
                    }

                    c(self, &params)?;
                }
                None => {
                    let command = String::from_utf8_lossy(command);

                    let error = format!("Unexpected command \"{}\"", command);
                    return Err(self.error(error));
                }
            }
        }

        self.finalize_bin()?;

        Ok(())
    }

    /// REM comment
    fn command_rem(&mut self, _: &[&[u8]]) -> CdResult<()> {
        // REM is used for comments, we can ignore this line
        Ok(())
    }

    /// FILE filename filetype
    fn command_file(&mut self, params: &[&[u8]]) -> CdResult<()> {
        let mut bin_name = params[1];
        let bin_type = params[2];

        self.finalize_bin()?;

        if bin_name[0] == b'"' {
            // The name was quoted, move past the quote (the end quote
            // has already been stripped by `split`
            bin_name = &bin_name[1..];
        }

        if bin_type != b"BINARY" {
            let ty = String::from_utf8_lossy(bin_type);

            let error = format!("Unsupported file type \"{}\"", ty);

            return Err(self.error(error));
        }

        // A new binary blob is introduced
        let (blob, size) = match self.bin_source {
            BinSource::Fs(ref root) => {
                // Open the new BIN blob
                BinaryBlob::from_file(root.clone(), bin_name)
            }
            BinSource::Zip { ref mut zip, .. } => BinaryBlob::from_zip_file(zip, bin_name),
        }?;

        self.bin_files.push(blob);
        self.bin_len = size;
        self.consumed_bytes = 0;
        self.index_msf = Msf::ZERO;
        self.index_type = None;

        Ok(())
    }

    /// TRACK bcd track_format
    fn command_track(&mut self, params: &[&[u8]]) -> CdResult<()> {
        if self.bin_files.is_empty() {
            return Err(self.error_str("File-less track"));
        }

        let n = match from_buf(params[1]) {
            Ok(b) => b,
            Err(_) => return Err(self.error_str("Invalid track number")),
        };

        let t = match params[2] {
            b"AUDIO" => CueTrackType::Audio,
            b"CDG" => return Err(self.error_str("Unsupported CDG track format")),
            b"MODE1/2048" => CueTrackType::Mode1Data,
            b"MODE1/2352" => CueTrackType::Mode1Raw,
            b"MODE2/2336" => CueTrackType::Mode2Headerless,
            b"MODE2/2352" => CueTrackType::Mode2Raw,
            b"CDI/2336" => CueTrackType::CdIHeaderless,
            b"CDI/2352" => CueTrackType::CdIRaw,
            _ => return Err(self.error_str("Unsupported track type")),
        };

        // According to the cdrwin docs the Mode2 formats are specifically for CD-ROM XA and never
        // CD-ROM Mode 2
        let f = match t {
            CueTrackType::Audio => TrackFormat::Audio,
            CueTrackType::Mode1Data => TrackFormat::Mode1,
            CueTrackType::Mode1Raw => TrackFormat::Mode1,
            CueTrackType::Mode2Headerless => TrackFormat::Mode2Xa,
            CueTrackType::Mode2Raw => TrackFormat::Mode2Xa,
            CueTrackType::CdIHeaderless => TrackFormat::Mode2CdI,
            CueTrackType::CdIRaw => TrackFormat::Mode2CdI,
        };

        let ctrl = if f.is_audio() {
            AdrControl::AUDIO
        } else {
            AdrControl::DATA
        };

        self.track = Some((n, t, f, ctrl));

        Ok(())
    }

    /// PREGAP mm:ss:ff
    ///
    /// There can be only one PREGAP per track and it must appear before any INDEX
    fn command_pregap(&mut self, params: &[&[u8]]) -> CdResult<()> {
        let (track_number, _track_type, track_format, ctrl) = match self.track {
            Some(t) => t,
            None => return Err(self.error_str("Track-less pregap")),
        };

        let msf = match from_buf(params[1]) {
            Ok(b) => b,
            Err(_) => return Err(self.error_str("Invalid index MSF")),
        };

        self.consume_bin_sectors(self.msf)?;

        let pregap = Index::new(
            Bcd::ZERO,
            self.msf,
            track_number,
            track_format,
            0,
            ctrl,
            Storage::PreGap,
        );
        self.indices.push(pregap);

        self.msf += msf;
        // Since the pregap is not stored in the file we need to advance the index_msf to avoid
        // skipping some of the data
        self.index_msf += msf;

        Ok(())
    }

    /// INDEX bcd mm:ss:ff
    fn command_index(&mut self, params: &[&[u8]]) -> CdResult<()> {
        let (track_number, track_type, track_format, ctrl) = match self.track {
            Some(t) => t,
            None => return Err(self.error_str("Track-less index")),
        };

        let n = match from_buf(params[1]) {
            Ok(b) => b,
            Err(_) => return Err(self.error_str("Invalid index")),
        };

        let msf = match from_buf(params[2]) {
            Ok(b) => b,
            Err(_) => return Err(self.error_str("Invalid index MSF")),
        };

        if track_number.binary() == 1 {
            // CUE always ignores track 1's pregap, let's add it in here
            let pregap = Index::new(
                Bcd::ZERO,
                Msf::ZERO,
                track_number,
                track_format,
                0,
                ctrl,
                Storage::PreGap,
            );

            self.indices.push(pregap);
        }

        self.consume_bin_sectors(msf)?;

        self.msf += msf;

        // Should be validated in `command_track`
        assert!(!self.bin_files.is_empty());

        let bin_index = (self.bin_files.len() - 1) as u32;

        let index = Index::new(
            n,
            self.msf,
            track_number,
            track_format,
            0,
            ctrl,
            Storage::Bin(bin_index, self.consumed_bytes, track_type),
        );

        self.indices.push(index);
        self.index_type = Some(track_type);

        Ok(())
    }

    /// FLAGS flag [flag [...]]
    fn command_flags(&mut self, params: &[&[u8]]) -> CdResult<()> {
        let (_track_number, _track_type, _track_format, ref mut ctrl) = match self.track {
            Some(t) => t,
            None => return Err(self.error_str("Track-less flag")),
        };

        if params.len() < 2 {
            return Err(self.error_str("Empty FLAGS command"));
        }

        for &flag in params.iter().skip(1) {
            match flag {
                b"DCP" => ctrl.set_digital_copy_permited(true),
                b"4CH" => ctrl.set_four_channel_audio(true),
                b"PRE" => ctrl.set_pre_emphasis(true),
                _ => return Err(self.error_str("Unknown flag")),
            }
        }

        Ok(())
    }

    /// Split the buffer into individual words. Handles quoted strings
    /// and treats them as a single word but returns them with the
    /// first quote included (to detect elements that shouldn't be
    /// quoted in the first place).
    pub fn split<'a>(&self, line: &'a [u8]) -> CdResult<Vec<&'a [u8]>> {
        let mut pos = 0;
        let len = line.len();
        let mut in_word = None;
        let mut words = Vec::new();

        let whitespace = b" \t\n\r";

        while pos < len {
            match in_word {
                Some((start, quoted)) => {
                    let delim = if quoted {
                        b"\"" as &[u8]
                    } else {
                        whitespace as &[u8]
                    };

                    if delim.contains(&line[pos]) {
                        words.push(&line[start..pos]);
                        in_word = None;
                    }
                }
                None => {
                    let cur = line[pos];

                    if !whitespace.contains(&cur) {
                        in_word = Some((pos, cur == b'"'));
                    }
                }
            }

            pos += 1;
        }

        if let Some((start, quoted)) = in_word {
            if quoted {
                // we reached the end of the line but didn't find the
                // matching quote, return an error
                return Err(self.error_str("Mismatched quote"));
            }

            words.push(&line[start..pos]);
        }

        Ok(words)
    }

    /// Advance in the current BIN file, updating how many bytes are
    /// left to consume.
    fn consume_bin_sectors(&mut self, offset: Msf) -> CdResult<()> {
        let delta = offset - self.index_msf;

        let delta = delta.sector_index() as u64;

        if delta == 0 {
            return Ok(());
        }

        let ty = match self.index_type {
            Some(t) => t,
            None => return Err(self.error_str("File doesn't start at 00:00:00")),
        };

        let sector_size = ty.sector_size() as u64;

        let index_size = match sector_size.checked_mul(delta) {
            Some(m) => m,
            None => return Err(self.error_str("Overflow: index is too big")),
        };

        if index_size > (self.bin_len - self.consumed_bytes) {
            return Err(self.error_str("Index out of range (past the end of the BIN file)"));
        }

        self.consumed_bytes += index_size;
        self.index_msf = offset;

        Ok(())
    }

    /// We're done with this bin file which means that whatever's left
    /// of it is for the last index.
    fn finalize_bin(&mut self) -> CdResult<()> {
        let ty = match self.index_type {
            Some(t) => t,
            // No previous index, nothing to be done
            None => return Ok(()),
        };

        let sector_size = ty.sector_size() as u64;

        let remaining_bytes = self.bin_len - self.consumed_bytes;

        let sectors = remaining_bytes / sector_size;

        if remaining_bytes % sector_size != 0 {
            return Err(self.error_str("Missaligned sector data while finishing a BIN file"));
        }

        let msf = match Msf::from_sector_index(sectors as u32) {
            Some(m) => m,
            None => return Err(self.error_str("Previous BIN file is too big, MSF overflow")),
        };

        self.msf = match self.msf.checked_add(msf) {
            Some(m) => m,
            None => return Err(self.error_str("Previous BIN file is too big, MSF overflow")),
        };

        Ok(())
    }
}

fn read_file<P: AsRef<Path>>(cue: P, max_len: u64) -> Result<Vec<u8>, io::Error> {
    let cue = cue.as_ref();
    let md = metadata(cue)?;

    let len = md.len();

    if len > max_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Cue sheet is too big",
        ));
    }

    let mut file = File::open(cue)?;
    let mut cue_sheet = Vec::with_capacity(len as usize);

    file.read_to_end(&mut cue_sheet)?;

    Ok(cue_sheet)
}

fn next_line(cue_sheet: &[u8], start: usize) -> Option<(usize, &[u8])> {
    if start >= cue_sheet.len() {
        return None;
    }

    let mut end = start;

    while end < cue_sheet.len() && cue_sheet[end] != b'\n' {
        end += 1;
    }

    end += 1;

    Some((end, &cue_sheet[start..end]))
}

/// Like from_str but from an `u8`. Fails if buffer is not valid utf-8
fn from_buf<T: FromStr>(b: &[u8]) -> Result<T, ()> {
    let s = match ::std::str::from_utf8(b) {
        Ok(s) => s,
        Err(_) => return Err(()),
    };

    match T::from_str(s) {
        Ok(t) => Ok(t),
        Err(_) => Err(()),
    }
}

/// Build a PathBuf from a byte buffer. If the C-string doesn't
/// contain a valid Path encoding return `None`.
#[cfg(unix)]
pub fn build_path(bytes: &[u8]) -> Option<PathBuf> {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    // On unix I assume that the path is an arbitrary null-terminated
    // byte string
    Some(PathBuf::from(OsStr::from_bytes(bytes)))
}

/// Build a PathBuf from a byte buffer. If the C-string doesn't
/// contain a valid Path encoding return `None`.
#[cfg(not(unix))]
pub fn build_path(bytes: &[u8]) -> Option<PathBuf> {
    // On Windows and other non-unices I assume that the path is
    // utf-8 encoded. That might be a bogus assumption, we'll see
    // in practice.
    let s = match ::std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return None,
    };

    Some(PathBuf::from(s))
}

/// Possible sources for BIN files
pub enum BinSource {
    Fs(PathBuf),
    Zip {
        zip: ZipArchive<File>,
        path: PathBuf,
    },
}

impl BinSource {
    pub fn read_exact_from(
        &mut self,
        blob: &mut BinaryBlob,
        seek: SeekFrom,
        buf: &mut [u8],
    ) -> CdResult<()> {
        match (self, blob) {
            (BinSource::Fs(_path), BinaryBlob::File(f)) => {
                f.seek(seek)?;

                f.read_exact(buf)?;
            }
            (BinSource::Zip { zip, .. }, BinaryBlob::ZipFile { zip_index, buffer }) => {
                {
                    let v = buffer.get_mut();

                    if v.is_empty() {
                        // Decompress this file
                        let mut f = zip.by_index(*zip_index)?;

                        f.read_to_end(v)?;
                    }
                }

                buffer.seek(seek)?;
                buffer.read_exact(buf)?;
            }
            _ => unreachable!("Invalid BinarySource/BinaryBlob configuration"),
        }

        Ok(())
    }
}

/// `BinaryBlob` can contain one or several slices interrupted by pre- and post-gaps.
#[derive(Debug)]
pub enum BinaryBlob {
    /// The blob is contained in a File
    File(File),
    /// The blob is contained in a ZIP file, referenced by its index.
    ZipFile {
        /// The index in the ZIP archive
        zip_index: usize,
        /// The contents are decompressed when the blob is first accessed
        buffer: io::Cursor<Vec<u8>>,
    },
}

impl BinaryBlob {
    fn from_file(mut bin_path: PathBuf, bin_name: &[u8]) -> io::Result<(BinaryBlob, u64)> {
        match build_path(bin_name) {
            // If bin_name is an absolute Path it'll replace the
            // parent completely bin_path (see the doc for PathBuf)
            Some(p) => bin_path.push(p),
            None => {
                // XXX Use `InvalidFilename` when stabilized
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "Invalid BIN path in cuesheet: `{}`",
                        String::from_utf8_lossy(bin_name)
                    ),
                ));
            }
        }

        let file = File::open(&bin_path)?;

        let size = metadata(&bin_path)?.len();

        Ok((BinaryBlob::File(file), size))
    }

    fn from_zip_file(zip: &mut ZipArchive<File>, name: &[u8]) -> io::Result<(BinaryBlob, u64)> {
        for i in 0..zip.len() {
            let f = match zip.by_index(i) {
                Ok(f) => f,
                Err(_) => continue,
            };

            if f.name_raw() == name {
                let size = f.size();
                let blob = BinaryBlob::ZipFile {
                    zip_index: i,
                    buffer: io::Cursor::new(Vec::new()),
                };

                return Ok((blob, size));
            }
        }

        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "Couldn't find `{}` in ZIP file",
                String::from_utf8_lossy(name)
            ),
        ))
    }
}
