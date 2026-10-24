//! Musica `.paz` virtual file system.
//!
//! # Container
//!
//! ```text
//! +0x00  char id[0x20]      "eden_en" / "eden_PlusMosaic_en"
//! +0x20  u32  idx_len       encrypted index length (multiple of 8)
//! +0x24  u8   index[idx_len]
//! +0x24+idx_len             data area
//! ```
//!
//! The index is Blowfish-ECB encrypted with `negate(tbl1[slot])` and uses
//! little-endian 32-bit words. Each record is `name\0 u64 offset u32 usize
//! u32 size u32 aligned_size u32 is_packed`.
//!
//! Data entries are `Blowfish-ECB(negate(tbl2[slot]))`, then an optional RC4
//! keystream keyed by `cp932(name.lower() + " %08X" % usize + " " + type_salt)`,
//! then truncated to `size`, then optionally zlib-inflated. `.ani`, `.wav`,
//! `.psd` and `.txt` have no RC4 layer. `mov.paz` uses a separate scheme: the
//! index carries a 256-byte `video_key`, and data is XORed with a 64 KiB RC4
//! block repeated cyclically.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::cache::ByteLru;
use crate::crypto::{rc4_keystream, Blowfish, Rc4};
use crate::error::{PazError, Result};
use crate::exe::{Edition, KeySet};

/// Default decoded-byte cache budget (256 MiB), matching the iOS VRAM/byte
/// cache ceiling in `TARGET_IOS_CONSTRAINTS.md`.
pub const DEFAULT_CACHE_BYTES: usize = 256 * 1024 * 1024;

/// One index record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PazEntry {
    pub name: String,
    /// Absolute file offset of the encrypted payload.
    pub offset: u64,
    /// Size after decryption and zlib inflation (the logical file size).
    pub unpacked_size: u32,
    /// Size after the RC4 layer and truncation, before zlib inflation.
    pub size: u32,
    /// Blowfish ciphertext length (multiple of 8).
    pub aligned_size: u32,
    pub is_packed: bool,
}

impl PazEntry {
    pub fn is_video(&self) -> bool {
        self.name.to_ascii_lowercase().ends_with(".avi")
    }

    pub fn extension(&self) -> String {
        Path::new(&self.name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
    }
}

/// A single-file `.paz` archive with a lazily-read, 64-bit-offset data area.
pub struct PazArchive {
    path: PathBuf,
    file: File,
    ident: String,
    edition: Edition,
    slot: usize,
    keys: KeySet,
    entries: Vec<PazEntry>,
    by_name: HashMap<String, usize>,
    by_lower: HashMap<String, usize>,
    video_key: Option<[u8; 256]>,
    is_audio: bool,
    is_video: bool,
    file_len: u64,
}

impl std::fmt::Debug for PazArchive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PazArchive")
            .field("path", &self.path)
            .field("ident", &self.ident)
            .field("edition", &self.edition)
            .field("slot", &self.slot)
            .field("entries", &self.entries.len())
            .field("is_audio", &self.is_audio)
            .field("is_video", &self.is_video)
            .finish()
    }
}

impl PazArchive {
    /// Open an archive using the embedded key tables.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_impl(path.as_ref(), None)
    }

    /// Open an archive and extract the key tables from the matching executable.
    /// This is the "dynamic extraction" path and must produce the same result
    /// as [`PazArchive::open`].
    pub fn open_with_exe(path: impl AsRef<Path>, exe: impl AsRef<Path>) -> Result<Self> {
        Self::open_impl(path.as_ref(), Some(exe.as_ref()))
    }

    fn open_impl(path: &Path, exe: Option<&Path>) -> Result<Self> {
        let file = File::open(path)?;
        let file_len = file.metadata()?.len();
        if file_len < 0x28 {
            return Err(PazError::InvalidHeader(format!(
                "{} is only {file_len} bytes",
                path.display()
            )));
        }
        let mut header = [0u8; 0x24];
        read_exact_at(&file, &mut header, 0)?;
        // The id is a NUL-terminated string at offset 0. It is *not* padded
        // to 0x20 bytes: bytes after the terminator are already part of the
        // container's reserved area. Stop at the first NUL.
        let id_bytes = header[..0x20].split(|&b| b == 0).next().unwrap_or(&[]);
        let ident = String::from_utf8_lossy(id_bytes).to_string();
        let idx_len = u32::from_le_bytes(header[0x20..0x24].try_into().unwrap()) as usize;
        if idx_len == 0 || idx_len % 8 != 0 {
            return Err(PazError::InvalidHeader(format!(
                "index length {idx_len} is not a non-zero multiple of 8"
            )));
        }
        if 0x24 + idx_len as u64 > file_len {
            return Err(PazError::InvalidHeader("index runs past EOF".into()));
        }

        let slot = slot_for_path(path)?;
        let is_audio = is_audio_archive(path);
        let is_video = is_video_archive(path);

        let edition = match Edition::from_archive_id(&ident) {
            Some(edition) => edition,
            None => {
                // Unknown id: try both editions and keep the one whose index
                // parses into plausible entries.
                let mut last_err = None;
                let mut chosen = None;
                for edition in [Edition::Main, Edition::PlusMosaic] {
                    let keys = match exe {
                        Some(exe) => crate::exe::load_keys_from_exe(exe, edition)?,
                        None => KeySet::embedded(edition),
                    };
                    let mut cipher = vec![0u8; idx_len];
                    read_exact_at(&file, &mut cipher, 0x24)?;
                    let bf = Blowfish::from_paz_table(keys.index_key(slot));
                    if bf.decrypt_ecb_inplace(&mut cipher).is_err() {
                        continue;
                    }
                    match parse_index(&cipher, is_video, file_len) {
                        Ok(_) => {
                            chosen = Some(edition);
                            break;
                        }
                        Err(e) => last_err = Some(e),
                    }
                }
                chosen.ok_or_else(|| {
                    last_err.unwrap_or_else(|| PazError::InvalidHeader("unknown archive id".into()))
                })?
            }
        };
        let keys = match exe {
            Some(exe) => crate::exe::load_keys_from_exe(exe, edition)?,
            None => KeySet::embedded(edition),
        };

        let mut cipher = vec![0u8; idx_len];
        read_exact_at(&file, &mut cipher, 0x24)?;
        Blowfish::from_paz_table(keys.index_key(slot)).decrypt_ecb_inplace(&mut cipher)?;
        let (video_key, entries) = parse_index(&cipher, is_video, file_len)?;

        let mut by_name = HashMap::with_capacity(entries.len());
        let mut by_lower = HashMap::with_capacity(entries.len());
        for (i, e) in entries.iter().enumerate() {
            by_name.insert(e.name.clone(), i);
            by_lower.insert(e.name.to_ascii_lowercase(), i);
        }

        Ok(Self {
            path: path.to_path_buf(),
            file,
            ident,
            edition,
            slot,
            keys,
            entries,
            by_name,
            by_lower,
            video_key,
            is_audio,
            is_video,
            file_len,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn ident(&self) -> &str {
        &self.ident
    }

    pub fn edition(&self) -> Edition {
        self.edition
    }

    pub fn slot(&self) -> usize {
        self.slot
    }

    pub fn entries(&self) -> &[PazEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn is_audio_archive(&self) -> bool {
        self.is_audio
    }

    pub fn is_video_archive(&self) -> bool {
        self.is_video
    }

    pub fn entry(&self, name: &str) -> Option<&PazEntry> {
        self.by_name
            .get(name)
            .or_else(|| self.by_lower.get(&name.to_ascii_lowercase()))
            .map(|&i| &self.entries[i])
    }

    pub fn entry_index(&self, name: &str) -> Option<usize> {
        self.by_name
            .get(name)
            .or_else(|| self.by_lower.get(&name.to_ascii_lowercase()))
            .copied()
    }

    pub fn entry_at(&self, index: usize) -> Option<&PazEntry> {
        self.entries.get(index)
    }

    /// Read and decrypt an entire entry into memory.
    pub fn read_entry(&self, name: &str) -> Result<Vec<u8>> {
        let entry = self
            .entry(name)
            .ok_or_else(|| PazError::EntryNotFound(name.to_string()))?;
        self.read_entry_obj(entry)
    }

    /// Read and decrypt an entire entry into memory.
    pub fn read_entry_obj(&self, entry: &PazEntry) -> Result<Vec<u8>> {
        let mut stream = self.open_entry(entry)?;
        // `unpacked_size` may legitimately exceed the archive size for a
        // zlib-packed entry (st.paz/dummy.txt is 100 bytes on disk and
        // inflates to >1 MiB), so only pre-size from the on-disk size.
        let mut out = Vec::with_capacity(entry.size as usize);
        stream.read_to_end(&mut out)?;
        // Packed entries in the retail data can inflate to a slightly
        // different length than the index's `unpacked_size` (st.paz's
        // dummy.txt is 264 bytes while the index says 256), so only enforce
        // the exact size for uncompressed entries.
        if !entry.is_packed && out.len() != entry.unpacked_size as usize {
            return Err(PazError::InvalidIndex(format!(
                "{}: decoded {} bytes, index says {}",
                entry.name,
                out.len(),
                entry.unpacked_size
            )));
        }
        Ok(out)
    }

    /// Read only the first `n` decrypted bytes (used for magic sniffing and
    /// video probes without pulling a 100 MiB AVI into memory).
    pub fn read_entry_prefix(&self, entry: &PazEntry, n: usize) -> Result<Vec<u8>> {
        let mut stream = self.open_entry(entry)?;
        let mut out = vec![0u8; n.min(entry.unpacked_size as usize)];
        stream.read_exact(&mut out)?;
        Ok(out)
    }

    /// Open a streaming, block-decrypting reader. The archive file itself is
    /// never mmap'd; reads use 64-bit offsets (`pread` on Unix).
    pub fn open_entry(&self, entry: &PazEntry) -> Result<EntryReader> {
        if entry.offset.saturating_add(u64::from(entry.aligned_size)) > self.file_len {
            return Err(PazError::InvalidIndex(format!(
                "{}: data range {}..{} is past EOF {}",
                entry.name,
                entry.offset,
                entry.offset + u64::from(entry.aligned_size),
                self.file_len
            )));
        }
        let file = self.file.try_clone()?;
        let bf = if self.is_video {
            None
        } else {
            Some(Blowfish::from_paz_table(self.keys.data_key(self.slot)))
        };
        let rc4_key = if self.is_video {
            None
        } else {
            self.rc4_salt(entry)
                .map(|salt| entry_rc4_key(&entry.name, entry.unpacked_size, salt))
        };
        let rc4 = rc4_key.as_ref().map(|k| Rc4::new(k));
        let video_block = if self.is_video {
            let vk = self
                .video_key
                .as_ref()
                .ok_or_else(|| PazError::Crypto("mov.paz index has no video_key".into()))?;
            let key = entry_rc4_key(&entry.name, entry.unpacked_size, &self.keys.type_keys[3]);
            let mut k = [0u8; 256];
            for i in 0..256 {
                k[i] = vk[i] ^ key[i % key.len()];
            }
            Some(rc4_keystream(&k, 0x10000))
        } else {
            None
        };
        let plain = EntryStream::new(
            file,
            entry.clone(),
            bf,
            rc4,
            rc4_key,
            video_block,
            self.is_video,
        );
        if entry.is_packed {
            Ok(EntryReader::Zlib(flate2::read::ZlibDecoder::new(plain)))
        } else {
            Ok(EntryReader::Plain(plain))
        }
    }

    fn rc4_salt(&self, entry: &PazEntry) -> Option<&[u8; 8]> {
        if self.is_video {
            return None;
        }
        if self.is_audio {
            return Some(&self.keys.type_keys[1]); // ogg
        }
        self.keys.type_key(&entry.extension())
    }

    /// Decode a PNG entry straight to RGBA.
    pub fn decode_png(&self, name: &str) -> Result<crate::image::RgbaImage> {
        let bytes = self.read_entry(name)?;
        crate::image::decode_png(&bytes)
    }

    /// Parse a `.ani` entry.
    pub fn parse_ani(&self, name: &str) -> Result<crate::ani::AniFile> {
        let bytes = self.read_entry(name)?;
        crate::ani::parse_ani(&bytes)
    }

    /// Cached raw bytes. The cache key is `archive-id/entry-name`.
    pub fn read_entry_cached(&self, name: &str, cache: &mut ByteLru) -> Result<Vec<u8>> {
        let key = format!("{}/{}", self.ident, name);
        if let Some(bytes) = cache.get(&key) {
            return Ok(bytes.clone());
        }
        let bytes = self.read_entry(name)?;
        cache.insert_bytes(key, bytes.clone());
        Ok(bytes)
    }

    /// Cached PNG decode.
    pub fn decode_png_cached(
        &self,
        name: &str,
        cache: &mut ByteLru,
    ) -> Result<crate::image::RgbaImage> {
        let bytes = self.read_entry_cached(name, cache)?;
        crate::image::decode_png(&bytes)
    }
}

fn slot_for_path(path: &Path) -> Result<usize> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let slot = match stem.as_str() {
        "scr" => 0,
        "bg" => 1,
        "st" => 3,
        "sys" => 4,
        "bgm" => 5,
        "voice" => 6,
        "se" => 7,
        "mov" => 9,
        _ => {
            return Err(PazError::InvalidHeader(format!(
                "cannot infer archive slot from filename {}",
                path.display()
            )))
        }
    };
    Ok(slot)
}

fn is_audio_archive(path: &Path) -> bool {
    matches!(
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "bgm" | "se" | "voice" | "pmbgm" | "pmse" | "pmvoice"
    )
}

fn is_video_archive(path: &Path) -> bool {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .eq_ignore_ascii_case("mov")
}

/// `cp932(name.lower() + " %08X" % unpacked_size + " " + salt)`.
///
/// The name must be re-encoded as CP932, not UTF-8: `sys.paz` contains names
/// such as `TRANS1024x75 - コピー.png` whose key bytes are the original CP932
/// byte sequence. Using the Rust string's UTF-8 bytes silently breaks exactly
/// those entries while leaving ASCII names working.
pub fn entry_rc4_key(name: &str, unpacked_size: u32, salt: &[u8]) -> Vec<u8> {
    let lowered = name.to_lowercase();
    let (encoded, _, had_errors) = encoding_rs::SHIFT_JIS.encode(&lowered);
    let mut key = Vec::with_capacity(encoded.len() + 1 + 8 + 1 + salt.len());
    if had_errors {
        // Should never happen for a name that came out of a CP932 index, but
        // keep a deterministic fallback instead of panicking.
        key.extend_from_slice(lowered.as_bytes());
    } else {
        key.extend_from_slice(&encoded);
    }
    key.push(b' ');
    key.extend_from_slice(format!("{unpacked_size:08X}").as_bytes());
    key.push(b' ');
    key.extend_from_slice(salt);
    key
}

fn parse_index(
    buf: &[u8],
    is_video: bool,
    file_len: u64,
) -> Result<(Option<[u8; 256]>, Vec<PazEntry>)> {
    if buf.len() < 4 {
        return Err(PazError::InvalidIndex("index shorter than count".into()));
    }
    let count = u32::from_le_bytes(buf[0..4].try_into().unwrap()) as usize;
    if count > 1_000_000 {
        return Err(PazError::InvalidIndex(format!(
            "implausible entry count {count}"
        )));
    }
    let mut p = 4usize;
    let video_key = if is_video {
        if buf.len() < 4 + 0x100 {
            return Err(PazError::InvalidIndex("mov index missing video_key".into()));
        }
        let mut k = [0u8; 256];
        k.copy_from_slice(&buf[4..4 + 0x100]);
        p += 0x100;
        Some(k)
    } else {
        None
    };
    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        let z = buf[p..]
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| PazError::InvalidIndex(format!("entry {i}: unterminated name")))?;
        let name_bytes = &buf[p..p + z];
        if name_bytes.is_empty() {
            return Err(PazError::InvalidIndex(format!("entry {i}: empty name")));
        }
        let (name, _, had_errors) = encoding_rs::SHIFT_JIS.decode(name_bytes);
        if had_errors {
            return Err(PazError::InvalidIndex(format!(
                "entry {i}: name is not CP932"
            )));
        }
        p += z + 1;
        if p + 24 > buf.len() {
            return Err(PazError::InvalidIndex(format!(
                "entry {i}: truncated record"
            )));
        }
        let offset = u64::from_le_bytes(buf[p..p + 8].try_into().unwrap());
        let unpacked_size = u32::from_le_bytes(buf[p + 8..p + 12].try_into().unwrap());
        let size = u32::from_le_bytes(buf[p + 12..p + 16].try_into().unwrap());
        let aligned_size = u32::from_le_bytes(buf[p + 16..p + 20].try_into().unwrap());
        let is_packed = u32::from_le_bytes(buf[p + 20..p + 24].try_into().unwrap()) != 0;
        p += 24;
        if aligned_size % 8 != 0 {
            return Err(PazError::InvalidIndex(format!(
                "{name}: aligned size {aligned_size} is not a multiple of 8"
            )));
        }
        if size > aligned_size {
            return Err(PazError::InvalidIndex(format!(
                "{name}: size {size} > aligned size {aligned_size}"
            )));
        }
        if offset.saturating_add(u64::from(aligned_size)) > file_len {
            return Err(PazError::InvalidIndex(format!(
                "{name}: data {}..{} is past EOF {file_len}",
                offset,
                offset + u64::from(aligned_size)
            )));
        }
        entries.push(PazEntry {
            name: name.into_owned(),
            offset,
            unpacked_size,
            size,
            aligned_size,
            is_packed,
        });
    }
    // Index ciphertext is padded to 8 bytes; allow zero padding only.
    if buf[p..].iter().any(|&b| b != 0) {
        return Err(PazError::InvalidIndex(format!(
            "{} non-zero trailing index bytes",
            buf.len() - p
        )));
    }
    Ok((video_key, entries))
}

/// Streaming reader over one entry.
pub enum EntryReader {
    Plain(EntryStream),
    Zlib(flate2::read::ZlibDecoder<EntryStream>),
}

impl Read for EntryReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            EntryReader::Plain(s) => s.read(buf),
            EntryReader::Zlib(z) => z.read(buf),
        }
    }
}

/// Sequential block-decrypting stream.
///
/// Blowfish is block-oriented and can be decrypted at any 8-byte boundary, but
/// RC4 is stateful, so this reader advances the RC4 state from byte 0 of the
/// entry. It still reads the `.paz` with 64-bit `pread` calls and never maps the
/// whole archive.
pub struct EntryStream {
    file: File,
    entry: PazEntry,
    bf: Option<Blowfish>,
    rc4: Option<Rc4>,
    rc4_key: Option<Vec<u8>>,
    video_block: Option<Vec<u8>>,
    is_video: bool,
    /// Number of ciphertext bytes consumed (multiple of 8 except for video).
    consumed: u64,
    /// Number of plaintext bytes handed to the caller.
    delivered: u64,
    pending: Vec<u8>,
    pending_pos: usize,
    chunk_size: usize,
}

impl EntryStream {
    fn new(
        file: File,
        entry: PazEntry,
        bf: Option<Blowfish>,
        rc4: Option<Rc4>,
        rc4_key: Option<Vec<u8>>,
        video_block: Option<Vec<u8>>,
        is_video: bool,
    ) -> Self {
        Self {
            file,
            entry,
            bf,
            rc4,
            rc4_key,
            video_block,
            is_video,
            consumed: 0,
            delivered: 0,
            pending: Vec::new(),
            pending_pos: 0,
            chunk_size: 64 * 1024,
        }
    }

    fn fill(&mut self) -> io::Result<()> {
        self.pending.clear();
        self.pending_pos = 0;
        if self.delivered >= u64::from(self.entry.size) {
            return Ok(());
        }
        let total = u64::from(self.entry.aligned_size);
        if self.consumed >= total {
            return Ok(());
        }
        let take = ((total - self.consumed) as usize).min(self.chunk_size);
        let take = if self.is_video {
            take
        } else {
            // Blowfish needs whole 8-byte blocks. `aligned_size` is a multiple
            // of 8 and `chunk_size` is too, so this only trims the last chunk.
            take - (take % 8)
        };
        if take == 0 {
            return Ok(());
        }
        let mut buf = vec![0u8; take];
        read_exact_at(&self.file, &mut buf, self.entry.offset + self.consumed)?;
        if self.is_video {
            let block = self.video_block.as_ref().expect("video block");
            let start = (self.consumed as usize) % block.len();
            for (i, b) in buf.iter_mut().enumerate() {
                *b ^= block[(start + i) % block.len()];
            }
        } else {
            if let Some(bf) = &self.bf {
                bf.decrypt_ecb_inplace(&mut buf)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            }
            if let Some(rc4) = &mut self.rc4 {
                rc4.apply_keystream(&mut buf);
            }
        }
        self.consumed += take as u64;
        // Only expose bytes that belong to the logical file (RC4 padding and
        // Blowfish padding are discarded).
        let remaining = u64::from(self.entry.size).saturating_sub(self.delivered) as usize;
        let visible = remaining.min(buf.len());
        buf.truncate(visible);
        self.pending = buf;
        Ok(())
    }
}

impl Read for EntryStream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            if self.pending_pos < self.pending.len() {
                let n = (self.pending.len() - self.pending_pos).min(out.len());
                out[..n].copy_from_slice(&self.pending[self.pending_pos..self.pending_pos + n]);
                self.pending_pos += n;
                self.delivered += n as u64;
                return Ok(n);
            }
            if self.delivered >= u64::from(self.entry.size) {
                return Ok(0);
            }
            self.fill()?;
            if self.pending.is_empty() {
                return Ok(0);
            }
        }
    }
}

impl Seek for EntryStream {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::Current(n) => self.delivered as i64 + n,
            SeekFrom::End(n) => i64::from(self.entry.size) + n,
        };
        if target < 0 || target > i64::from(self.entry.size) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek outside entry",
            ));
        }
        let target = target as u64;
        if target < self.delivered {
            // RC4 is stateful; restart from the entry key.
            self.consumed = 0;
            self.delivered = 0;
            self.pending.clear();
            self.pending_pos = 0;
            self.rc4 = self.rc4_key.as_ref().map(|k| Rc4::new(k));
        }
        let mut skip = target - self.delivered;
        let mut scratch = [0u8; 8192];
        while skip > 0 {
            let n = (skip as usize).min(scratch.len());
            let got = self.read(&mut scratch[..n])?;
            if got == 0 {
                break;
            }
            skip -= got as u64;
        }
        Ok(self.delivered)
    }
}

#[cfg(unix)]
fn read_exact_at(file: &File, mut buf: &mut [u8], mut offset: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    while !buf.is_empty() {
        match file.read_at(buf, offset) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "short pread")),
            Ok(n) => {
                buf = &mut buf[n..];
                offset += n as u64;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(windows)]
fn read_exact_at(file: &File, mut buf: &mut [u8], mut offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        match file.seek_read(buf, offset) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "short seek_read",
                ))
            }
            Ok(n) => {
                buf = &mut buf[n..];
                offset += n as u64;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    use std::sync::Mutex;
    // Fallback for exotic targets: serialize seek+read on the handle.
    let mut f = file.try_clone()?;
    f.seek(SeekFrom::Start(offset))?;
    f.read_exact(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::decode_png;

    fn game_dir() -> Option<PathBuf> {
        let p = PathBuf::from("/mnt/f/gal/【PC硬盘】【官中】eden");
        if p.exists() {
            Some(p)
        } else {
            None
        }
    }

    fn samples(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../samples")
            .join(name)
    }

    #[test]
    fn opens_main_scr_and_decrypts_script() {
        let path = samples("main/scr.paz");
        if !path.exists() {
            eprintln!("skip: {}", path.display());
            return;
        }
        let a = PazArchive::open(&path).unwrap();
        assert_eq!(a.ident(), "eden_en");
        assert_eq!(a.edition(), Edition::Main);
        assert_eq!(a.slot(), 0);
        assert_eq!(a.len(), 23);
        let names: Vec<_> = a.entries().iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"A001.sc"));
        let bytes = a.read_entry("A001.sc").unwrap();
        let (text, _, had) = encoding_rs::GBK.decode(&bytes);
        assert!(!had);
        assert!(text.starts_with(".setGlobal EYE_SEQ_DEFAULT = 2"));
        assert!(text.contains("我不能停下双手"));
    }

    #[test]
    fn opens_pm_scr_and_decrypts_script() {
        let path = samples("pm/scr.paz");
        if !path.exists() {
            eprintln!("skip: {}", path.display());
            return;
        }
        let a = PazArchive::open(&path).unwrap();
        assert_eq!(a.ident(), "eden_PlusMosaic_en");
        assert_eq!(a.edition(), Edition::PlusMosaic);
        assert_eq!(a.len(), 35);
        let bytes = a.read_entry("A001.sc").unwrap();
        let (text, _, had) = encoding_rs::SHIFT_JIS.decode(&bytes);
        assert!(!had);
        assert!(text.contains("I mustn’t stop."));
    }

    #[test]
    fn dynamic_exe_keys_open_same_archive() {
        let paz = samples("main/scr.paz");
        let exe = samples("main/eden.exe");
        if !paz.exists() || !exe.exists() {
            eprintln!("skip: samples missing");
            return;
        }
        let a = PazArchive::open(&paz).unwrap();
        let b = PazArchive::open_with_exe(&paz, &exe).unwrap();
        assert_eq!(a.ident(), b.ident());
        assert_eq!(a.entries(), b.entries());
        assert_eq!(
            a.read_entry("A001.sc").unwrap(),
            b.read_entry("A001.sc").unwrap()
        );
    }

    #[test]
    fn packed_dummy_txt_inflates() {
        let path = samples("main/st.paz");
        if !path.exists() {
            eprintln!("skip: {}", path.display());
            return;
        }
        let a = PazArchive::open(&path).unwrap();
        assert_eq!(a.len(), 1);
        let e = &a.entries()[0];
        assert!(e.is_packed);
        let bytes = a.read_entry_obj(e).unwrap();
        // The retail index says 256 but the zlib stream inflates to 264.
        assert_eq!(bytes.len(), 264);
        assert!(bytes.starts_with(b"aaaa"));
        assert!(bytes.contains(&b'\r') && bytes.contains(&b'\n'));
    }

    #[test]
    fn streaming_matches_bulk_decryption() {
        let Some(dir) = game_dir() else {
            eprintln!("skip: game dir missing");
            return;
        };
        let a = PazArchive::open(dir.join("sys.paz")).unwrap();
        let png = a
            .entries()
            .iter()
            .find(|e| e.name.to_ascii_lowercase().ends_with(".png") && e.unpacked_size > 100_000)
            .unwrap();
        let bulk = a.read_entry_obj(png).unwrap();
        let mut stream = a.open_entry(png).unwrap();
        let mut chunked = Vec::new();
        let mut buf = [0u8; 7];
        loop {
            let n = stream.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            chunked.extend_from_slice(&buf[..n]);
        }
        assert_eq!(bulk, chunked);
        let img = decode_png(&bulk).unwrap();
        assert!(img.width >= 640 && img.height >= 480);
    }

    #[test]
    fn reads_sys_png_color_types() {
        let Some(dir) = game_dir() else {
            eprintln!("skip: game dir missing");
            return;
        };
        let a = PazArchive::open(dir.join("sys.paz")).unwrap();
        let mut decoded = 0;
        let mut seen = [false; 4];
        for e in a
            .entries()
            .iter()
            .filter(|e| e.name.to_ascii_lowercase().ends_with(".png"))
            .take(80)
        {
            let bytes = a.read_entry_obj(e).unwrap();
            assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
            // Color type is at byte 25 of IHDR.
            let ct = bytes[25];
            if ct == 0 {
                seen[0] = true;
            } else if ct == 2 {
                seen[1] = true;
            } else if ct == 4 {
                seen[2] = true;
            } else if ct == 6 {
                seen[3] = true;
            }
            let img = decode_png(&bytes).unwrap();
            assert!(img.width > 0 && img.height > 0);
            decoded += 1;
        }
        assert!(decoded > 10);
        assert!(seen.iter().any(|x| *x));
    }

    #[test]
    fn parses_every_ani_in_bg_archive() {
        let Some(dir) = game_dir() else {
            eprintln!("skip: game dir missing");
            return;
        };
        let a = PazArchive::open(dir.join("bg.paz")).unwrap();
        let anis: Vec<_> = a
            .entries()
            .iter()
            .filter(|e| e.name.to_ascii_lowercase().ends_with(".ani"))
            .collect();
        assert!(anis.len() > 1000);
        let mut frames = 0usize;
        let mut paired = 0usize;
        for e in &anis {
            let ani = a.parse_ani(&e.name).unwrap();
            frames += ani.frame_count();
            let png = e.name[..e.name.len() - 4].to_string() + ".png";
            if a.entry(&png).is_some() {
                paired += 1;
            }
        }
        assert_eq!(paired, anis.len(), "every .ani must have a same-named PNG");
        assert!(frames > 2000);
        eprintln!("parsed {} .ani files / {} frames", anis.len(), frames);
    }

    #[test]
    fn ani_composite_matches_pillow_reference_hash() {
        let Some(dir) = game_dir() else {
            eprintln!("skip: game dir missing");
            return;
        };
        let a = PazArchive::open(dir.join("bg.paz")).unwrap();
        let name = "ev_add02_up06a.ani";
        let Some(_) = a.entry(name) else {
            eprintln!("skip: sample ani missing");
            return;
        };
        let base = a.decode_png("ev_add02_up06a.png").unwrap();
        let ani = a.parse_ani(name).unwrap();
        let composed = ani.composed(&base, 0, 255).unwrap();
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(&composed.data);
        let digest = format!("{:x}", h.finalize());
        // Reference hash generated by Pillow 13.0 with
        // Image.alpha_composite(base, frame) at (477, 270).
        assert_eq!(
            digest, "e63d64f7b0248270612d11d7f163beea287d9f1126ead582b9a5be781e791ffe",
            "composite hash changed; regenerate the Pillow reference"
        );
    }

    #[test]
    fn mov_paz_first_avi_has_riff_header() {
        let Some(dir) = game_dir() else {
            eprintln!("skip: game dir missing");
            return;
        };
        let path = dir.join("mov.paz");
        if !path.exists() {
            eprintln!("skip: mov.paz missing");
            return;
        }
        let a = PazArchive::open(&path).unwrap();
        assert!(a.is_video_archive());
        let e = &a.entries()[0];
        let prefix = a.read_entry_prefix(e, 16).unwrap();
        assert_eq!(&prefix[..4], b"RIFF");
        assert_eq!(&prefix[8..12], b"AVI ");
    }

    #[test]
    fn lru_cache_roundtrip() {
        let path = samples("main/scr.paz");
        if !path.exists() {
            return;
        }
        let a = PazArchive::open(&path).unwrap();
        let mut cache = ByteLru::new(DEFAULT_CACHE_BYTES);
        let first = a.read_entry_cached("A001.sc", &mut cache).unwrap();
        let second = a.read_entry_cached("A001.sc", &mut cache).unwrap();
        assert_eq!(first, second);
        assert!(!cache.is_empty());
    }

    #[test]
    fn rc4_key_format() {
        let key = entry_rc4_key("Foo.PNG", 0x1234, b"J1WaoGBi");
        assert_eq!(key, b"foo.png 00001234 J1WaoGBi");
    }

    #[test]
    fn rc4_key_uses_cp932_for_non_ascii_names() {
        let key = entry_rc4_key("TRANS1024x75 - コピー.png", 16934, b"J1WaoGBi");
        // The suffix must be the CP932 bytes of "コピー" (83 52 83 73 81 5b),
        // not UTF-8.
        let (cp932, _, _) = encoding_rs::SHIFT_JIS.encode("trans1024x75 - コピー.png");
        assert_eq!(&key[..cp932.len()], &cp932[..]);
        assert_eq!(&key[cp932.len()..cp932.len() + 1], b" ");
        assert_eq!(&key[cp932.len() + 1..cp932.len() + 9], b"00004226");
    }
}
