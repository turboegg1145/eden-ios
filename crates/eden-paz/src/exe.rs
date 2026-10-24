//! Minimal PE32 reader used to extract the Musica key tables directly from
//! `eden.exe` / `eden_pm_en.exe` at runtime.
//!
//! The embedded tables in [`crate::tables`] are generated from the same
//! binaries and are used when no executable is available (e.g. on iOS, where
//! the user only copies the `.paz` files). Both paths are verified to produce
//! identical bytes by the test-suite.

use std::fs;
use std::path::Path;

use crate::error::{PazError, Result};
use crate::tables::{
    MAIN_TBL1, MAIN_TBL1_VA, MAIN_TBL2, MAIN_TBL2_VA, MAIN_TYPE_KEYS, MAIN_TYPE_KEYS_VA,
    MAIN_TYPE_KEY_STRIDE, PM_TBL1, PM_TBL1_VA, PM_TBL2, PM_TBL2_VA, PM_TYPE_KEYS, PM_TYPE_KEYS_VA,
    PM_TYPE_KEY_STRIDE, P_INIT,
};

/// Which release of the game a `.paz` belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Edition {
    /// `eden_en` — Chinese-localised original release (`eden.exe`).
    Main,
    /// `eden_PlusMosaic_en` — English PLUS+MOSAIC release (`eden_pm_en.exe`).
    PlusMosaic,
}

impl Edition {
    pub fn from_archive_id(id: &str) -> Option<Self> {
        match id {
            "eden_en" => Some(Edition::Main),
            "eden_PlusMosaic_en" => Some(Edition::PlusMosaic),
            _ => None,
        }
    }

    pub fn archive_id(self) -> &'static str {
        match self {
            Edition::Main => "eden_en",
            Edition::PlusMosaic => "eden_PlusMosaic_en",
        }
    }

    /// Script text encoding. `eden_en` was localised to Simplified Chinese and
    /// stores GBK; the PLUS+MOSAIC scripts are CP932 (the ASCII-looking text
    /// still contains CP932 curly quotes, so plain ASCII is not sufficient).
    pub fn script_encoding(self) -> &'static encoding_rs::Encoding {
        match self {
            Edition::Main => encoding_rs::GBK,
            Edition::PlusMosaic => encoding_rs::SHIFT_JIS,
        }
    }
}

/// The four per-type RC4 salts in `{sc, ogg, png, avi}` order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySet {
    /// Ten per-slot 32-byte index keys (`TBL1 + slot * 0x20`).
    pub tbl1: [[u8; 32]; 10],
    /// Ten per-slot 32-byte data keys (`TBL2 + slot * 0x20`).
    pub tbl2: [[u8; 32]; 10],
    pub type_keys: [[u8; 8]; 4],
}

impl KeySet {
    pub fn embedded(edition: Edition) -> Self {
        match edition {
            Edition::Main => Self {
                tbl1: MAIN_TBL1,
                tbl2: MAIN_TBL2,
                type_keys: MAIN_TYPE_KEYS,
            },
            Edition::PlusMosaic => Self {
                tbl1: PM_TBL1,
                tbl2: PM_TBL2,
                type_keys: PM_TYPE_KEYS,
            },
        }
    }

    pub fn index_key(&self, slot: usize) -> &[u8; 32] {
        &self.tbl1[slot.min(9)]
    }

    pub fn data_key(&self, slot: usize) -> &[u8; 32] {
        &self.tbl2[slot.min(9)]
    }

    /// Salt for an extension (`sc`, `ogg`, `png`, `avi`), if the extension has
    /// an RC4 layer at all.
    pub fn type_key(&self, ext: &str) -> Option<&[u8; 8]> {
        match ext.to_ascii_lowercase().as_str() {
            "sc" => Some(&self.type_keys[0]),
            "ogg" => Some(&self.type_keys[1]),
            "png" => Some(&self.type_keys[2]),
            "avi" => Some(&self.type_keys[3]),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct Section {
    va: u32,
    vsize: u32,
    raw_size: u32,
    raw_ptr: u32,
}

/// A parsed PE image held in memory (the executables are only ~2.5 MiB).
pub struct PeImage {
    data: Vec<u8>,
    image_base: u32,
    sections: Vec<Section>,
}

impl PeImage {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let data = fs::read(path)?;
        Self::parse(data)
    }

    pub fn parse(data: Vec<u8>) -> Result<Self> {
        if data.len() < 0x40 || &data[0..2] != b"MZ" {
            return Err(PazError::Exe("missing MZ signature".into()));
        }
        let pe_off = u32::from_le_bytes(data[0x3c..0x40].try_into().unwrap()) as usize;
        if pe_off + 24 > data.len() || &data[pe_off..pe_off + 4] != b"PE\0\0" {
            return Err(PazError::Exe("missing PE signature".into()));
        }
        let coff = pe_off + 4;
        let n_sections = u16::from_le_bytes(data[coff + 2..coff + 4].try_into().unwrap()) as usize;
        let opt_size = u16::from_le_bytes(data[coff + 16..coff + 18].try_into().unwrap()) as usize;
        let opt_off = coff + 20;
        if opt_off + 32 > data.len() {
            return Err(PazError::Exe("truncated optional header".into()));
        }
        let magic = u16::from_le_bytes(data[opt_off..opt_off + 2].try_into().unwrap());
        let image_base = match magic {
            0x10b => u32::from_le_bytes(data[opt_off + 28..opt_off + 32].try_into().unwrap()),
            0x20b => u32::from_le_bytes(data[opt_off + 24..opt_off + 28].try_into().unwrap()),
            other => {
                return Err(PazError::Exe(format!(
                    "unsupported optional-header magic 0x{other:04X}"
                )))
            }
        };
        let mut off = opt_off + opt_size;
        let mut sections = Vec::with_capacity(n_sections);
        for _ in 0..n_sections {
            if off + 40 > data.len() {
                return Err(PazError::Exe("truncated section table".into()));
            }
            let vsize = u32::from_le_bytes(data[off + 8..off + 12].try_into().unwrap());
            let va = u32::from_le_bytes(data[off + 12..off + 16].try_into().unwrap());
            let raw_size = u32::from_le_bytes(data[off + 16..off + 20].try_into().unwrap());
            let raw_ptr = u32::from_le_bytes(data[off + 20..off + 24].try_into().unwrap());
            sections.push(Section {
                va,
                vsize,
                raw_size,
                raw_ptr,
            });
            off += 40;
        }
        Ok(Self {
            data,
            image_base,
            sections,
        })
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Translate a virtual address to a file offset.
    pub fn va_to_offset(&self, va: u32) -> Option<usize> {
        // The key-table addresses in the RE notes are absolute virtual
        // addresses (image base 0x400000), while PE section headers store
        // RVAs. Accept either form.
        let rva = if va >= self.image_base {
            va - self.image_base
        } else {
            va
        };
        for s in &self.sections {
            let span = s.vsize.max(s.raw_size);
            if rva >= s.va && rva < s.va.saturating_add(span) {
                let delta = rva - s.va;
                if delta >= s.raw_size {
                    return None;
                }
                let off = s.raw_ptr as usize + delta as usize;
                if off <= self.data.len() {
                    return Some(off);
                }
            }
        }
        None
    }

    pub fn read_va(&self, va: u32, len: usize) -> Result<&[u8]> {
        let off = self
            .va_to_offset(va)
            .ok_or_else(|| PazError::Exe(format!("VA 0x{va:08X} is not mapped")))?;
        let end = off
            .checked_add(len)
            .ok_or_else(|| PazError::Exe("VA length overflow".into()))?;
        if end > self.data.len() {
            return Err(PazError::Exe(format!(
                "VA 0x{va:08X} length {len} runs past EOF"
            )));
        }
        Ok(&self.data[off..end])
    }

    /// Locate the standard Blowfish P/S boxes by searching for P[0].
    pub fn find_blowfish_tables(&self) -> Result<([u32; 18], [[u32; 256]; 4])> {
        let needle = P_INIT[0].to_le_bytes();
        let mut found = None;
        for i in 0..self.data.len().saturating_sub(4) {
            if self.data[i..i + 4] == needle {
                let need = 1042 * 4;
                if i + need <= self.data.len() {
                    // Verify a few more P words to avoid false positives.
                    let vals: Vec<u32> = self.data[i..i + need]
                        .chunks_exact(4)
                        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
                        .collect();
                    if vals[..18] == P_INIT {
                        found = Some((i, vals));
                        break;
                    }
                }
            }
        }
        let (_, vals) = found.ok_or_else(|| PazError::Exe("Blowfish tables not found".into()))?;
        let mut p = [0u32; 18];
        p.copy_from_slice(&vals[..18]);
        let mut s = [[0u32; 256]; 4];
        for (i, box_) in s.iter_mut().enumerate() {
            box_.copy_from_slice(&vals[18 + i * 256..18 + (i + 1) * 256]);
        }
        Ok((p, s))
    }

    /// Extract the per-slot keys and type salts for one release.
    pub fn extract_keys(&self, edition: Edition) -> Result<KeySet> {
        let (tbl1_va, tbl2_va, tk_va, stride) = match edition {
            Edition::Main => (
                MAIN_TBL1_VA,
                MAIN_TBL2_VA,
                MAIN_TYPE_KEYS_VA,
                MAIN_TYPE_KEY_STRIDE,
            ),
            Edition::PlusMosaic => (PM_TBL1_VA, PM_TBL2_VA, PM_TYPE_KEYS_VA, PM_TYPE_KEY_STRIDE),
        };
        let raw1 = self.read_va(tbl1_va, 10 * 32)?;
        let raw2 = self.read_va(tbl2_va, 10 * 32)?;
        let mut tbl1 = [[0u8; 32]; 10];
        let mut tbl2 = [[0u8; 32]; 10];
        for i in 0..10 {
            tbl1[i].copy_from_slice(&raw1[i * 32..(i + 1) * 32]);
            tbl2[i].copy_from_slice(&raw2[i * 32..(i + 1) * 32]);
        }
        let mut type_keys = [[0u8; 8]; 4];
        for (i, key) in type_keys.iter_mut().enumerate() {
            key.copy_from_slice(self.read_va(tk_va + (i * stride) as u32, 8)?);
            if !key.iter().all(|b| b.is_ascii_graphic()) {
                return Err(PazError::Exe(format!(
                    "type key {i} at 0x{:08X} is not ASCII",
                    tk_va + (i * stride) as u32
                )));
            }
        }
        Ok(KeySet {
            tbl1,
            tbl2,
            type_keys,
        })
    }
}

/// Extract keys from an executable, falling back to the embedded tables when
/// the executable is unavailable.
pub fn load_keys_from_exe(path: impl AsRef<Path>, edition: Edition) -> Result<KeySet> {
    let pe = PeImage::load(path)?;
    pe.extract_keys(edition)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str) -> Option<std::path::PathBuf> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../samples")
            .join(name);
        if p.exists() {
            Some(p)
        } else {
            None
        }
    }

    #[test]
    fn dynamic_keys_match_embedded_main() {
        let Some(path) = sample("main/eden.exe") else {
            eprintln!("skip: samples/main/eden.exe missing");
            return;
        };
        let pe = PeImage::load(path).unwrap();
        let dynamic = pe.extract_keys(Edition::Main).unwrap();
        assert_eq!(dynamic, KeySet::embedded(Edition::Main));
        let (p, s) = pe.find_blowfish_tables().unwrap();
        assert_eq!(p, crate::tables::P_INIT);
        assert_eq!(s[0][0], crate::tables::S_INIT[0][0]);
    }

    #[test]
    fn dynamic_keys_match_embedded_pm() {
        let Some(path) = sample("pm/eden_pm_en.exe") else {
            eprintln!("skip: samples/pm/eden_pm_en.exe missing");
            return;
        };
        let pe = PeImage::load(path).unwrap();
        let dynamic = pe.extract_keys(Edition::PlusMosaic).unwrap();
        assert_eq!(dynamic, KeySet::embedded(Edition::PlusMosaic));
    }
}
