//! AVI RIFF demuxer.
//!
//! The retail `mov.paz` archives contain plain AVI 1.0 files:
//!
//! * `op.avi` — WMV3 (VC-1 Main) video, 16-bit PCM audio.
//! * `ed.avi` — MPEG-4 ASP (XviD) video, MP3 audio.
//! * PLUS+MOSAIC `op.avi` / `ed.avi` — WMV3 video, PCM audio.
//!
//! This module parses the RIFF structure, both stream headers, the `movi`
//! chunk index (`idx1`, with a sequential fallback) and exposes the raw
//! compressed packets plus decoded PCM/MP3 audio. Video packet decoding lives
//! in [`crate::ffmpeg`] (desktop) and the iOS VideoToolbox bridge.

use std::fmt;
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eden_audio::{AudioClip, AudioError};

#[derive(Debug)]
pub enum AviError {
    Io(String),
    Invalid(String),
    Unsupported(String),
    Audio(AudioError),
}

impl fmt::Display for AviError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AviError::Io(e) => write!(f, "avi io: {e}"),
            AviError::Invalid(e) => write!(f, "invalid avi: {e}"),
            AviError::Unsupported(e) => write!(f, "unsupported avi: {e}"),
            AviError::Audio(e) => write!(f, "avi audio: {e}"),
        }
    }
}

impl std::error::Error for AviError {}

impl From<std::io::Error> for AviError {
    fn from(value: std::io::Error) -> Self {
        AviError::Io(value.to_string())
    }
}

impl From<AudioError> for AviError {
    fn from(value: AudioError) -> Self {
        AviError::Audio(value)
    }
}

pub type Result<T> = std::result::Result<T, AviError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoCodec {
    Wmv3,
    Mpeg4,
    H264,
    Other([u8; 4]),
}

impl VideoCodec {
    fn from_tag(tag: [u8; 4]) -> Self {
        match &tag {
            b"WMV3" | b"WVC1" | b"WMVA" => VideoCodec::Wmv3,
            b"XVID" | b"xvid" | b"DIVX" | b"divx" | b"DX50" | b"FMP4" | b"MP4V" | b"mp4v" => {
                VideoCodec::Mpeg4
            }
            b"H264" | b"h264" | b"AVC1" | b"avc1" => VideoCodec::H264,
            _ => VideoCodec::Other(tag),
        }
    }

    pub fn name(self) -> String {
        match self {
            VideoCodec::Wmv3 => "wmv3".into(),
            VideoCodec::Mpeg4 => "mpeg4".into(),
            VideoCodec::H264 => "h264".into(),
            VideoCodec::Other(tag) => String::from_utf8_lossy(&tag).to_string(),
        }
    }

    pub fn is_legacy(self) -> bool {
        matches!(self, VideoCodec::Wmv3 | VideoCodec::Mpeg4)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioCodec {
    Pcm,
    Mp3,
    Other(u16),
}

impl AudioCodec {
    pub fn name(self) -> String {
        match self {
            AudioCodec::Pcm => "pcm_s16le".into(),
            AudioCodec::Mp3 => "mp3".into(),
            AudioCodec::Other(tag) => format!("tag-0x{tag:04x}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AudioFormat {
    pub codec: AudioCodec,
    pub channels: u16,
    pub sample_rate: u32,
    pub bits_per_sample: u16,
    pub block_align: u16,
}

#[derive(Clone, Debug)]
pub struct AviChunk {
    pub id: [u8; 4],
    /// Absolute file offset of the chunk payload.
    pub offset: u64,
    pub size: u32,
    pub keyframe: bool,
}

#[derive(Clone, Debug)]
pub struct VideoStream {
    pub codec: VideoCodec,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub frame_count: u32,
    pub scale: u32,
    pub rate: u32,
    pub extradata: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct AudioStream {
    pub format: AudioFormat,
    pub chunks: Vec<AviChunk>,
}

enum Source {
    Memory(Arc<Vec<u8>>),
    File(File),
}

/// A parsed AVI file. The payload is read lazily through `read_at`, so a
/// 130 MiB movie never has to be resident in memory as a whole.
pub struct AviFile {
    source: Source,
    pub path: Option<PathBuf>,
    pub video: VideoStream,
    pub audio: Option<AudioStream>,
    pub video_chunks: Vec<AviChunk>,
    pub file_len: u64,
}

impl fmt::Debug for AviFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AviFile")
            .field("path", &self.path)
            .field("video", &self.video)
            .field("audio", &self.audio)
            .field("video_chunks", &self.video_chunks.len())
            .field("file_len", &self.file_len)
            .finish()
    }
}

impl AviFile {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let file = File::open(path)?;
        let file_len = file.metadata()?.len();
        let mut avi = Self::parse(Source::File(file), Some(path.to_path_buf()), file_len)?;
        avi.resolve_chunk_offsets()?;
        Ok(avi)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        let len = bytes.len() as u64;
        let mut avi = Self::parse(Source::Memory(Arc::new(bytes)), None, len)?;
        avi.resolve_chunk_offsets()?;
        Ok(avi)
    }

    pub fn read_at(&self, offset: u64, len: usize) -> Result<Vec<u8>> {
        match &self.source {
            Source::Memory(bytes) => {
                let start = offset as usize;
                let end = start
                    .checked_add(len)
                    .ok_or_else(|| AviError::Invalid("range overflow".into()))?;
                if end > bytes.len() {
                    return Err(AviError::Invalid(format!(
                        "range {offset}..{end} past EOF {}",
                        bytes.len()
                    )));
                }
                Ok(bytes[start..end].to_vec())
            }
            Source::File(file) => {
                let mut buf = vec![0u8; len];
                read_exact_at(file, &mut buf, offset)?;
                Ok(buf)
            }
        }
    }

    pub fn read_chunk(&self, chunk: &AviChunk) -> Result<Vec<u8>> {
        self.read_at(chunk.offset, chunk.size as usize)
    }

    pub fn duration(&self) -> f64 {
        if self.video.fps > 0.0 {
            self.video.frame_count as f64 / self.video.fps
        } else {
            0.0
        }
    }

    /// Decode the complete audio track. PCM is converted directly; MP3 is
    /// handed to Symphonia's pure-Rust decoder.
    pub fn decode_audio(&self) -> Result<Option<AudioClip>> {
        let Some(stream) = &self.audio else {
            return Ok(None);
        };
        let mut payload = Vec::new();
        for chunk in &stream.chunks {
            payload.extend_from_slice(&self.read_chunk(chunk)?);
        }
        if payload.is_empty() {
            return Ok(None);
        }
        let clip = match stream.format.codec {
            AudioCodec::Pcm => decode_pcm(&payload, &stream.format)?,
            AudioCodec::Mp3 => crate::mp3::decode_mp3(&payload)?,
            AudioCodec::Other(tag) => {
                return Err(AviError::Unsupported(format!(
                    "audio format tag 0x{tag:04x}"
                )))
            }
        };
        Ok(Some(clip))
    }

    fn parse(source: Source, path: Option<PathBuf>, file_len: u64) -> Result<Self> {
        let reader = SourceReader::new(source);
        let mut header = [0u8; 12];
        reader.read_at(0, &mut header)?;
        if &header[0..4] != b"RIFF" || &header[8..12] != b"AVI " {
            return Err(AviError::Invalid("not a RIFF/AVI file".into()));
        }
        let riff_len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as u64;
        let end = (riff_len + 8).min(file_len);

        let mut parser = Parser {
            reader: &reader,
            file_len,
            avih: None,
            streams: Vec::new(),
            idx1: Vec::new(),
            movi_offsets: Vec::new(),
        };
        parser.walk(12, end)?;
        let avih = parser
            .avih
            .ok_or_else(|| AviError::Invalid("missing avih".into()))?;
        let mut video = None;
        let mut audio = None;
        for stream in parser.streams {
            match stream.kind {
                StreamKind::Video(v) => {
                    if video.is_none() {
                        video = Some(v);
                    }
                }
                StreamKind::Audio(a) => {
                    if audio.is_none() {
                        audio = Some(a);
                    }
                }
            }
        }
        let mut video = video.ok_or_else(|| AviError::Invalid("no video stream".into()))?;
        if video.width == 0 {
            video.width = avih.width;
        }
        if video.height == 0 {
            video.height = avih.height;
        }
        if video.frame_count == 0 {
            video.frame_count = avih.total_frames;
        }
        if video.fps <= 0.0 {
            video.fps = if avih.micro_sec_per_frame > 0 {
                1_000_000.0 / avih.micro_sec_per_frame as f64
            } else {
                60.0
            };
        }
        // The stream header is authoritative, but some encoders leave
        // dwLength at zero while avih has the real count.
        if video.frame_count == 0 && avih.total_frames > 0 {
            video.frame_count = avih.total_frames;
        }
        let mut audio = audio;
        if let Some(audio) = &mut audio {
            audio.chunks = parser
                .idx1
                .iter()
                .filter(|e| e.chunk.id[0..2] == [b'0', b'1'] && &e.chunk.id[2..4] == b"wb")
                .map(|e| e.chunk.clone())
                .collect();
        }
        let video_chunks = parser
            .idx1
            .iter()
            .filter(|e| &e.chunk.id[2..4] == b"db" || &e.chunk.id[2..4] == b"dc")
            .map(|e| e.chunk.clone())
            .collect();
        Ok(Self {
            source: reader.into_source(),
            path,
            video,
            audio,
            video_chunks,
            file_len,
        })
    }

    /// `idx1` offsets are relative to either the `movi` fourcc or its payload;
    /// sniff the fourcc to pick the right base.
    fn resolve_chunk_offsets(&mut self) -> Result<()> {
        let check = |chunk: &AviChunk| -> Result<()> {
            // `chunk.offset` points at the payload; the fourcc lives 8 bytes
            // earlier (the 8-byte RIFF chunk header).
            let head = self
                .read_at(chunk.offset.saturating_sub(8), 4)
                .unwrap_or_default();
            if head == chunk.id {
                return Ok(());
            }
            Err(AviError::Invalid(format!(
                "chunk {:?} at {} has header {:?}",
                String::from_utf8_lossy(&chunk.id),
                chunk.offset,
                String::from_utf8_lossy(&head)
            )))
        };
        for chunk in &self.video_chunks {
            check(chunk)?;
        }
        if let Some(audio) = &self.audio {
            for chunk in &audio.chunks {
                check(chunk)?;
            }
        }
        Ok(())
    }
}

struct SourceReader {
    source: Source,
}

impl SourceReader {
    fn new(source: Source) -> Self {
        Self { source }
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        match &self.source {
            Source::Memory(bytes) => {
                let start = offset as usize;
                let end = start + buf.len();
                if end > bytes.len() {
                    return Err(AviError::Invalid("read past EOF".into()));
                }
                buf.copy_from_slice(&bytes[start..end]);
                Ok(())
            }
            Source::File(file) => read_exact_at(file, buf, offset).map_err(AviError::from),
        }
    }

    fn into_source(self) -> Source {
        self.source
    }
}

#[derive(Default)]
struct Avih {
    micro_sec_per_frame: u32,
    total_frames: u32,
    width: u32,
    height: u32,
}

enum StreamKind {
    Video(VideoStream),
    Audio(AudioStream),
}

struct ParsedStream {
    kind: StreamKind,
}

struct IndexEntry {
    chunk: AviChunk,
}

struct Parser<'a> {
    reader: &'a SourceReader,
    file_len: u64,
    avih: Option<Avih>,
    streams: Vec<ParsedStream>,
    idx1: Vec<IndexEntry>,
    movi_offsets: Vec<u64>,
}

impl<'a> Parser<'a> {
    fn walk(&mut self, start: u64, end: u64) -> Result<()> {
        let mut pos = start;
        while pos + 8 <= end {
            let mut head = [0u8; 8];
            self.reader.read_at(pos, &mut head)?;
            let id = [head[0], head[1], head[2], head[3]];
            let size = u32::from_le_bytes(head[4..8].try_into().unwrap());
            let data = pos + 8;
            let next = data
                .checked_add(size as u64 + (size & 1) as u64)
                .ok_or_else(|| AviError::Invalid("chunk size overflow".into()))?;
            if data + size as u64 > self.file_len {
                return Err(AviError::Invalid(format!(
                    "chunk {} at {pos} runs past EOF",
                    fourcc(&id)
                )));
            }
            match &id {
                b"LIST" => {
                    let mut kind = [0u8; 4];
                    if size >= 4 {
                        self.reader.read_at(data, &mut kind)?;
                    }
                    match &kind {
                        b"hdrl" => self.walk(data + 4, data + size as u64)?,
                        b"strl" => self.parse_strl(data + 4, data + size as u64)?,
                        b"movi" => {
                            self.movi_offsets.push(pos);
                            // The index is authoritative when present. Only
                            // scan when idx1 is missing; we cannot know that
                            // until the whole file has been walked.
                        }
                        _ => {}
                    }
                }
                b"avih" => {
                    let mut buf = vec![0u8; size as usize];
                    self.reader.read_at(data, &mut buf)?;
                    if buf.len() >= 40 {
                        self.avih = Some(Avih {
                            micro_sec_per_frame: u32::from_le_bytes(
                                buf[0..4].try_into().unwrap(),
                            ),
                            total_frames: u32::from_le_bytes(buf[16..20].try_into().unwrap()),
                            width: u32::from_le_bytes(buf[32..36].try_into().unwrap()),
                            height: u32::from_le_bytes(buf[36..40].try_into().unwrap()),
                        });
                    }
                }
                b"idx1" => {
                    let mut buf = vec![0u8; size as usize];
                    self.reader.read_at(data, &mut buf)?;
                    self.parse_idx1(&buf)?;
                }
                _ => {}
            }
            pos = next;
        }
        Ok(())
    }

    fn parse_strl(&mut self, start: u64, end: u64) -> Result<()> {
        let mut strh = None;
        let mut strf = None;
        let mut pos = start;
        while pos + 8 <= end {
            let mut head = [0u8; 8];
            self.reader.read_at(pos, &mut head)?;
            let id = [head[0], head[1], head[2], head[3]];
            let size = u32::from_le_bytes(head[4..8].try_into().unwrap());
            let data = pos + 8;
            let next = data + size as u64 + (size & 1) as u64;
            let mut buf = vec![0u8; size as usize];
            self.reader.read_at(data, &mut buf)?;
            match &id {
                b"strh" => strh = Some(buf),
                b"strf" => strf = Some(buf),
                _ => {}
            }
            pos = next;
        }
        let Some(strh) = strh else {
            return Ok(());
        };
        if strh.len() < 48 {
            return Ok(());
        }
        let fcc_type = [strh[0], strh[1], strh[2], strh[3]];
        let fcc_handler = [strh[4], strh[5], strh[6], strh[7]];
        let scale = u32::from_le_bytes(strh[20..24].try_into().unwrap());
        let rate = u32::from_le_bytes(strh[24..28].try_into().unwrap());
        let length = u32::from_le_bytes(strh[32..36].try_into().unwrap());
        if &fcc_type == b"vids" {
            let strf = strf.unwrap_or_default();
            if strf.len() < 40 {
                return Err(AviError::Invalid("video strf shorter than 40".into()));
            }
            let width = i32::from_le_bytes(strf[4..8].try_into().unwrap()).unsigned_abs();
            let height = i32::from_le_bytes(strf[8..12].try_into().unwrap()).unsigned_abs();
            let tag = [strf[16], strf[17], strf[18], strf[19]];
            let codec = if tag == [0, 0, 0, 0] {
                VideoCodec::from_tag(fcc_handler)
            } else {
                VideoCodec::from_tag(tag)
            };
            let extradata = if strf.len() > 40 {
                strf[40..].to_vec()
            } else {
                Vec::new()
            };
            let fps = if scale > 0 && rate > 0 {
                rate as f64 / scale as f64
            } else {
                0.0
            };
            self.streams.push(ParsedStream {
                kind: StreamKind::Video(VideoStream {
                    codec,
                    width,
                    height,
                    fps,
                    frame_count: length,
                    scale,
                    rate,
                    extradata,
                }),
            });
        } else if &fcc_type == b"auds" {
            let Some(strf) = strf else {
                return Ok(());
            };
            if strf.len() < 16 {
                return Ok(());
            }
            let tag = u16::from_le_bytes(strf[0..2].try_into().unwrap());
            let codec = match tag {
                0x0001 | 0x0003 => AudioCodec::Pcm,
                0x0055 => AudioCodec::Mp3,
                other => AudioCodec::Other(other),
            };
            self.streams.push(ParsedStream {
                kind: StreamKind::Audio(AudioStream {
                    format: AudioFormat {
                        codec,
                        channels: u16::from_le_bytes(strf[2..4].try_into().unwrap()),
                        sample_rate: u32::from_le_bytes(strf[4..8].try_into().unwrap()),
                        bits_per_sample: if strf.len() >= 16 {
                            u16::from_le_bytes(strf[14..16].try_into().unwrap())
                        } else {
                            16
                        },
                        block_align: if strf.len() >= 14 {
                            u16::from_le_bytes(strf[12..14].try_into().unwrap())
                        } else {
                            4
                        },
                    },
                    chunks: Vec::new(),
                }),
            });
        }
        Ok(())
    }

    fn parse_idx1(&mut self, buf: &[u8]) -> Result<()> {
        if self.movi_offsets.is_empty() {
            return Ok(());
        }
        let movi = self.movi_offsets[0];
        for record in buf.chunks_exact(16) {
            let id = [record[0], record[1], record[2], record[3]];
            let flags = u32::from_le_bytes(record[4..8].try_into().unwrap());
            let offset = u32::from_le_bytes(record[8..12].try_into().unwrap()) as u64;
            let size = u32::from_le_bytes(record[12..16].try_into().unwrap());
            if size == 0 {
                continue;
            }
            // The index offset is relative to the `movi` fourcc in most
            // files and to its payload in others. Pick the candidate whose
            // chunk header matches the recorded fourcc.
            let mut chosen = None;
            for base in [movi + 8, movi + 4] {
                let abs = base + offset;
                let mut head = [0u8; 4];
                if self.reader.read_at(abs, &mut head).is_ok() && head == id {
                    chosen = Some(abs);
                    break;
                }
            }
            let Some(abs) = chosen else {
                continue;
            };
            self.idx1.push(IndexEntry {
                chunk: AviChunk {
                    id,
                    offset: abs + 8,
                    size,
                    keyframe: flags & 0x10 != 0,
                },
            });
        }
        Ok(())
    }
}

/// Convert a PCM payload to `AudioClip`.
fn decode_pcm(payload: &[u8], format: &AudioFormat) -> Result<AudioClip> {
    let channels = format.channels.max(1);
    let bytes_per_sample = (format.bits_per_sample / 8).max(1) as usize;
    let frame_bytes = bytes_per_sample * channels as usize;
    if frame_bytes == 0 {
        return Err(AviError::Unsupported("zero PCM frame size".into()));
    }
    let frames = payload.len() / frame_bytes;
    let mut samples = Vec::with_capacity(frames * channels as usize);
    for frame in 0..frames {
        let base = frame * frame_bytes;
        for ch in 0..channels as usize {
            let at = base + ch * bytes_per_sample;
            let value = match bytes_per_sample {
                1 => (payload[at] as f32 - 128.0) / 128.0,
                2 => i16::from_le_bytes([payload[at], payload[at + 1]]) as f32 / 32768.0,
                3 => {
                    let v = i32::from_le_bytes([
                        0,
                        payload[at],
                        payload[at + 1],
                        payload[at + 2],
                    ]) >> 8;
                    v as f32 / 8_388_608.0
                }
                4 => i32::from_le_bytes([
                    payload[at],
                    payload[at + 1],
                    payload[at + 2],
                    payload[at + 3],
                ]) as f32
                    / 2_147_483_648.0,
                _ => return Err(AviError::Unsupported("PCM sample width".into())),
            };
            samples.push(value);
        }
    }
    Ok(AudioClip::new(format.sample_rate, channels, samples)?)
}

fn fourcc(id: &[u8; 4]) -> String {
    String::from_utf8_lossy(id).to_string()
}

#[cfg(unix)]
fn read_exact_at(file: &File, mut buf: &mut [u8], mut offset: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    while !buf.is_empty() {
        match file.read_at(buf, offset) {
            Ok(0) => return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "short pread")),
            Ok(n) => {
                buf = &mut buf[n..];
                offset += n as u64;
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(windows)]
fn read_exact_at(file: &File, mut buf: &mut [u8], mut offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        match file.seek_read(buf, offset) {
            Ok(0) => return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "short seek_read")),
            Ok(n) => {
                buf = &mut buf[n..];
                offset += n as u64;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Sequential fallback used by tests and archives without `idx1`.
pub fn scan_movi<R: Read + Seek>(reader: &mut R, start: u64, end: u64) -> Result<Vec<AviChunk>> {
    reader.seek(SeekFrom::Start(start))?;
    let mut chunks = Vec::new();
    let mut pos = start;
    while pos + 8 <= end {
        let mut head = [0u8; 8];
        reader.read_exact(&mut head)?;
        let id = [head[0], head[1], head[2], head[3]];
        let size = u32::from_le_bytes(head[4..8].try_into().unwrap());
        if &id[2..4] == b"db" || &id[2..4] == b"dc" || &id[2..4] == b"wb" {
            chunks.push(AviChunk {
                id,
                offset: pos + 8,
                size,
                keyframe: false,
            });
        }
        pos += 8 + size as u64 + (size & 1) as u64;
        reader.seek(SeekFrom::Start(pos))?;
    }
    Ok(chunks)
}

/// Decode MP3 bytes with Symphonia (re-exported helper used by tests).
pub fn decode_mp3_bytes(bytes: &[u8]) -> Result<AudioClip> {
    crate::mp3::decode_mp3(bytes).map_err(AviError::from)
}

/// Read a whole AVI into a cursor (used by unit tests with synthetic data).
pub fn cursor(bytes: &[u8]) -> Cursor<Vec<u8>> {
    Cursor::new(bytes.to_vec())
}
