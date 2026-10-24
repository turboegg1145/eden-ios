//! RGBA8 image container, PNG codec and compositing helpers.

use std::io::Cursor;

use crate::error::{PazError, Result};

/// A tightly packed 8-bit RGBA image (row-major, 4 bytes per pixel).
#[derive(Clone, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl std::fmt::Debug for RgbaImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RgbaImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.data.len())
            .finish()
    }
}

impl RgbaImage {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0; width as usize * height as usize * 4],
        }
    }

    pub fn filled(width: u32, height: u32, rgba: [u8; 4]) -> Self {
        let mut img = Self::new(width, height);
        for px in img.data.chunks_exact_mut(4) {
            px.copy_from_slice(&rgba);
        }
        img
    }

    pub fn from_rgba(width: u32, height: u32, data: Vec<u8>) -> Result<Self> {
        let expected = width as usize * height as usize * 4;
        if data.len() != expected {
            return Err(PazError::Image(format!(
                "RGBA buffer is {} bytes, expected {expected} for {width}x{height}",
                data.len()
            )));
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }

    #[inline]
    pub fn index(&self, x: u32, y: u32) -> usize {
        (y as usize * self.width as usize + x as usize) * 4
    }

    #[inline]
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = self.index(x, y);
        [
            self.data[i],
            self.data[i + 1],
            self.data[i + 2],
            self.data[i + 3],
        ]
    }

    #[inline]
    pub fn set_pixel(&mut self, x: u32, y: u32, px: [u8; 4]) {
        let i = self.index(x, y);
        self.data[i..i + 4].copy_from_slice(&px);
    }

    /// Source-over alpha blend of one pixel.
    #[inline]
    pub fn blend_pixel(&mut self, x: u32, y: u32, src: [u8; 4], opacity: u8) {
        if x >= self.width || y >= self.height {
            return;
        }
        let a = ((u16::from(src[3]) * u16::from(opacity) + 127) / 255) as u8;
        if a == 0 {
            return;
        }
        if a == 255 {
            self.set_pixel(x, y, src);
            return;
        }
        let i = self.index(x, y);
        let inv = 255 - a;
        for c in 0..3 {
            let dst = u16::from(self.data[i + c]);
            let src_c = u16::from(src[c]);
            self.data[i + c] = ((src_c * u16::from(a) + dst * u16::from(inv) + 127) / 255) as u8;
        }
        let dst_a = u16::from(self.data[i + 3]);
        self.data[i + 3] = ((u16::from(a) * 255 + dst_a * u16::from(inv) + 127) / 255) as u8;
    }

    /// Draw `src` at `(x, y)`, clipping to this image's bounds.
    pub fn blit(&mut self, src: &RgbaImage, x: i32, y: i32, opacity: u8) {
        if opacity == 0 {
            return;
        }
        let x0 = x.max(0) as u32;
        let y0 = y.max(0) as u32;
        let x1 = (x + src.width as i32).min(self.width as i32).max(0) as u32;
        let y1 = (y + src.height as i32).min(self.height as i32).max(0) as u32;
        for sy in y0..y1 {
            let src_y = (sy as i32 - y) as u32;
            for sx in x0..x1 {
                let src_x = (sx as i32 - x) as u32;
                let px = src.pixel(src_x, src_y);
                self.blend_pixel(sx, sy, px, opacity);
            }
        }
    }

    /// Draw a `BGRA` patch (`.ani` frame layout) with source-over alpha.
    pub fn blit_bgra(&mut self, bgra: &[u8], w: u32, h: u32, x: i32, y: i32, opacity: u8) {
        if bgra.len() < w as usize * h as usize * 4 {
            return;
        }
        let x0 = x.max(0) as u32;
        let y0 = y.max(0) as u32;
        let x1 = (x + w as i32).min(self.width as i32).max(0) as u32;
        let y1 = (y + h as i32).min(self.height as i32).max(0) as u32;
        for sy in y0..y1 {
            let src_y = (sy as i32 - y) as u32;
            for sx in x0..x1 {
                let src_x = (sx as i32 - x) as u32;
                let i = (src_y as usize * w as usize + src_x as usize) * 4;
                let px = [bgra[i + 2], bgra[i + 1], bgra[i], bgra[i + 3]];
                self.blend_pixel(sx, sy, px, opacity);
            }
        }
    }

    /// Crop a rectangle, clamped to the image bounds.
    pub fn crop(&self, x: u32, y: u32, w: u32, h: u32) -> RgbaImage {
        let w = w.min(self.width.saturating_sub(x));
        let h = h.min(self.height.saturating_sub(y));
        let mut out = RgbaImage::new(w, h);
        for row in 0..h {
            let src = self.index(x, y + row);
            let dst = out.index(0, row);
            out.data[dst..dst + w as usize * 4]
                .copy_from_slice(&self.data[src..src + w as usize * 4]);
        }
        out
    }

    /// Luma in 0..=255 (Rec. 601 integer approximation) with bilinear sampling.
    pub fn luma_bilinear(&self, u: f32, v: f32) -> u8 {
        if self.width == 0 || self.height == 0 {
            return 0;
        }
        let x = (u * self.width as f32 - 0.5).clamp(0.0, self.width as f32 - 1.0);
        let y = (v * self.height as f32 - 0.5).clamp(0.0, self.height as f32 - 1.0);
        let x0 = x.floor() as u32;
        let y0 = y.floor() as u32;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let fx = x - x0 as f32;
        let fy = y - y0 as f32;
        let p00 = self.pixel(x0, y0);
        let p10 = self.pixel(x1, y0);
        let p01 = self.pixel(x0, y1);
        let p11 = self.pixel(x1, y1);
        let l = |p: [u8; 4]| -> f32 {
            (299.0 * f32::from(p[0]) + 587.0 * f32::from(p[1]) + 114.0 * f32::from(p[2])) / 1000.0
        };
        let top = l(p00) * (1.0 - fx) + l(p10) * fx;
        let bot = l(p01) * (1.0 - fx) + l(p11) * fx;
        (top * (1.0 - fy) + bot * fy).round().clamp(0.0, 255.0) as u8
    }
}

/// Decode a PNG file (all color types and bit depths supported by `png`).
pub fn decode_png(bytes: &[u8]) -> Result<RgbaImage> {
    if bytes.len() < 8 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err(PazError::Image("missing PNG signature".into()));
    }
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|e| PazError::Image(e.to_string()))?;
    let buf_size = reader
        .output_buffer_size()
        .ok_or_else(|| PazError::Image("PNG output buffer size overflow".into()))?;
    let mut buf = vec![0u8; buf_size];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| PazError::Image(e.to_string()))?;
    let w = info.width;
    let h = info.height;
    let src = &buf[..info.buffer_size()];
    let mut out = RgbaImage::new(w, h);
    match info.color_type {
        png::ColorType::Grayscale => {
            for (i, px) in out.data.chunks_exact_mut(4).enumerate() {
                let g = src[i];
                px.copy_from_slice(&[g, g, g, 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for (i, px) in out.data.chunks_exact_mut(4).enumerate() {
                let g = src[i * 2];
                let a = src[i * 2 + 1];
                px.copy_from_slice(&[g, g, g, a]);
            }
        }
        png::ColorType::Rgb => {
            for (i, px) in out.data.chunks_exact_mut(4).enumerate() {
                px.copy_from_slice(&[src[i * 3], src[i * 3 + 1], src[i * 3 + 2], 255]);
            }
        }
        png::ColorType::Rgba => {
            out.data.copy_from_slice(src);
        }
        other => {
            return Err(PazError::Image(format!(
                "unsupported PNG color type {other:?} after expansion"
            )))
        }
    }
    Ok(out)
}

/// Encode an RGBA8 image as PNG.
pub fn encode_png(img: &RgbaImage) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, img.width, img.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| PazError::Image(e.to_string()))?;
        writer
            .write_image_data(&img.data)
            .map_err(|e| PazError::Image(e.to_string()))?;
        writer
            .finish()
            .map_err(|e| PazError::Image(e.to_string()))?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_roundtrip() {
        let mut img = RgbaImage::new(17, 9);
        for y in 0..9 {
            for x in 0..17 {
                img.set_pixel(x, y, [x as u8 * 7, y as u8 * 11, 42, 200]);
            }
        }
        let bytes = encode_png(&img).unwrap();
        let back = decode_png(&bytes).unwrap();
        assert_eq!(back, img);
    }

    #[test]
    fn alpha_blend_matches_reference() {
        let mut img = RgbaImage::filled(1, 1, [100, 100, 100, 255]);
        img.blend_pixel(0, 0, [200, 0, 0, 128], 255);
        let p = img.pixel(0, 0);
        // (200*128 + 100*127 + 127)/255 = 150
        assert_eq!(p[0], 150);
        assert_eq!(p[1], 50);
        assert_eq!(p[2], 50);
    }

    #[test]
    fn blit_clips() {
        let mut dst = RgbaImage::filled(4, 4, [0, 0, 0, 255]);
        let src = RgbaImage::filled(3, 3, [255, 0, 0, 255]);
        dst.blit(&src, -1, -1, 255);
        assert_eq!(dst.pixel(0, 0), [255, 0, 0, 255]);
        assert_eq!(dst.pixel(3, 3), [0, 0, 0, 255]);
    }
}
