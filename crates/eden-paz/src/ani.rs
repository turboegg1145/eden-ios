//! Parser and compositor for Musica `.ani` differential animation files.
//!
//! Layout (little-endian):
//!
//! ```text
//! u16 version = 0x0100
//! u16 frame_count
//! u32 reserved
//! repeat frame_count:
//!     char label[]\0          // CP932, e.g. "シオン 開き口"
//!     u16 width
//!     u16 height
//!     u16 bpp = 32
//!     u16 origin_x
//!     u16 origin_y
//!     u8  pixels[width * height * 4]   // BGRA
//! ```
//!
//! Frames are patches: they are alpha-blended over the same-named base PNG at
//! `(origin_x, origin_y)`. Alpha is typically binary (0 or 255), but the
//! compositor uses general source-over blending so anti-aliased frames work.

use encoding_rs::SHIFT_JIS;

use crate::error::{PazError, Result};
use crate::image::RgbaImage;

/// One differential frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AniFrame {
    pub label: String,
    pub width: u16,
    pub height: u16,
    pub bpp: u16,
    pub origin_x: u16,
    pub origin_y: u16,
    /// BGRA8, row-major, `width * height * 4` bytes.
    pub pixels: Vec<u8>,
}

impl AniFrame {
    pub fn byte_len(&self) -> usize {
        self.pixels.len()
    }
}

/// A parsed `.ani` file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AniFile {
    pub version: u16,
    pub frames: Vec<AniFrame>,
    pub reserved: u32,
}

impl AniFile {
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Find the first frame whose label contains `needle`.
    pub fn frame_index_by_label(&self, needle: &str) -> Option<usize> {
        self.frames.iter().position(|f| f.label.contains(needle))
    }

    /// Return the closed-mouth frame if present, otherwise frame 0.
    pub fn closed_frame(&self) -> usize {
        self.frame_index_by_label("閉じ口")
            .or_else(|| self.frame_index_by_label("中口"))
            .unwrap_or(0)
    }

    /// Alpha-composite one frame onto `base` at its origin.
    pub fn compose_frame(&self, base: &mut RgbaImage, index: usize, opacity: u8) -> Result<()> {
        let frame = self
            .frames
            .get(index)
            .ok_or_else(|| PazError::Ani(format!("frame index {index} out of range")))?;
        base.blit_bgra(
            &frame.pixels,
            u32::from(frame.width),
            u32::from(frame.height),
            i32::from(frame.origin_x),
            i32::from(frame.origin_y),
            opacity,
        );
        Ok(())
    }

    /// Compose a frame and return a new image, leaving the base untouched.
    pub fn composed(&self, base: &RgbaImage, index: usize, opacity: u8) -> Result<RgbaImage> {
        let mut out = base.clone();
        self.compose_frame(&mut out, index, opacity)?;
        Ok(out)
    }
}

/// Parse a complete `.ani` file. The parser is strict: every byte must be
/// accounted for, which is how the 1,720-file corpus is validated.
pub fn parse_ani(bytes: &[u8]) -> Result<AniFile> {
    if bytes.len() < 8 {
        return Err(PazError::Ani("file shorter than 8-byte header".into()));
    }
    let version = u16::from_le_bytes([bytes[0], bytes[1]]);
    let frame_count = u16::from_le_bytes([bytes[2], bytes[3]]);
    let reserved = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    if version != 0x0100 {
        return Err(PazError::Ani(format!(
            "unsupported version 0x{version:04X}"
        )));
    }
    if frame_count == 0 {
        return Err(PazError::Ani("zero frames".into()));
    }
    let mut frames = Vec::with_capacity(usize::from(frame_count));
    let mut p = 8usize;
    for i in 0..frame_count {
        let z = bytes[p..]
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| PazError::Ani(format!("frame {i}: unterminated label")))?;
        let label_bytes = &bytes[p..p + z];
        let (label, _, had_errors) = SHIFT_JIS.decode(label_bytes);
        if had_errors {
            return Err(PazError::Ani(format!(
                "frame {i}: label is not valid CP932: {label_bytes:02X?}"
            )));
        }
        p += z + 1;
        if p + 10 > bytes.len() {
            return Err(PazError::Ani(format!("frame {i}: truncated dimensions")));
        }
        let width = u16::from_le_bytes([bytes[p], bytes[p + 1]]);
        let height = u16::from_le_bytes([bytes[p + 2], bytes[p + 3]]);
        let bpp = u16::from_le_bytes([bytes[p + 4], bytes[p + 5]]);
        let origin_x = u16::from_le_bytes([bytes[p + 6], bytes[p + 7]]);
        let origin_y = u16::from_le_bytes([bytes[p + 8], bytes[p + 9]]);
        p += 10;
        if bpp != 32 {
            return Err(PazError::Ani(format!("frame {i}: unsupported bpp {bpp}")));
        }
        if width == 0 || height == 0 {
            return Err(PazError::Ani(format!("frame {i}: zero dimensions")));
        }
        let n = usize::from(width) * usize::from(height) * 4;
        if p + n > bytes.len() {
            return Err(PazError::Ani(format!(
                "frame {i}: pixel data runs past EOF (need {n} at {p}, file {})",
                bytes.len()
            )));
        }
        let pixels = bytes[p..p + n].to_vec();
        p += n;
        frames.push(AniFrame {
            label: label.into_owned(),
            width,
            height,
            bpp,
            origin_x,
            origin_y,
            pixels,
        });
    }
    if p != bytes.len() {
        return Err(PazError::Ani(format!(
            "trailing {} bytes after {frame_count} frames",
            bytes.len() - p
        )));
    }
    Ok(AniFile {
        version,
        frames,
        reserved,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(version: u16, frames: &[(&str, u16, u16, u16, u16, [u8; 4])]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&version.to_le_bytes());
        out.extend_from_slice(&(frames.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for (label, w, h, ox, oy, px) in frames {
            let (encoded, _, had) = SHIFT_JIS.encode(label);
            assert!(!had, "test label must be CP932-encodable");
            out.extend_from_slice(&encoded);
            out.push(0);
            out.extend_from_slice(&w.to_le_bytes());
            out.extend_from_slice(&h.to_le_bytes());
            out.extend_from_slice(&32u16.to_le_bytes());
            out.extend_from_slice(&ox.to_le_bytes());
            out.extend_from_slice(&oy.to_le_bytes());
            for _ in 0..(*w as usize * *h as usize) {
                out.extend_from_slice(px);
            }
        }
        out
    }

    #[test]
    fn parses_synthetic_file() {
        let data = build(
            0x100,
            &[
                ("シオン 開き口", 2, 1, 5, 7, [1, 2, 3, 255]),
                ("シオン 閉じ口", 1, 1, 0, 0, [4, 5, 6, 0]),
            ],
        );
        let ani = parse_ani(&data).unwrap();
        assert_eq!(ani.version, 0x100);
        assert_eq!(ani.frame_count(), 2);
        assert_eq!(ani.frames[0].label, "シオン 開き口");
        assert_eq!(ani.frames[0].origin_x, 5);
        assert_eq!(ani.frames[1].pixels, vec![4, 5, 6, 0]);
    }

    #[test]
    fn rejects_trailing_bytes() {
        let mut data = build(0x100, &[("x", 1, 1, 0, 0, [0, 0, 0, 255])]);
        data.push(0);
        assert!(parse_ani(&data).is_err());
    }

    #[test]
    fn composes_bgra_with_alpha() {
        let data = build(
            0x100,
            &[
                ("開き口", 1, 1, 1, 1, [10, 20, 30, 255]),
                ("閉じ口", 1, 1, 1, 1, [10, 20, 30, 0]),
            ],
        );
        let ani = parse_ani(&data).unwrap();
        let base = RgbaImage::filled(3, 3, [100, 100, 100, 255]);
        let composed = ani.composed(&base, 0, 255).unwrap();
        assert_eq!(composed.pixel(1, 1), [30, 20, 10, 255]);
        let transparent = ani.composed(&base, 1, 255).unwrap();
        assert_eq!(transparent.pixel(1, 1), [100, 100, 100, 255]);
    }

    #[test]
    fn closed_frame_prefers_closed_label() {
        let data = build(
            0x100,
            &[
                ("シオン 開き口", 1, 1, 0, 0, [0, 0, 0, 255]),
                ("シオン 中口", 1, 1, 0, 0, [0, 0, 0, 255]),
                ("シオン 閉じ口", 1, 1, 0, 0, [0, 0, 0, 255]),
            ],
        );
        let ani = parse_ani(&data).unwrap();
        assert_eq!(ani.closed_frame(), 2);
    }
}
