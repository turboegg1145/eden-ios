//! RGBA framebuffer helpers used by the reference renderer.

use eden_paz::image::{encode_png, RgbaImage};
use eden_paz::Result;

#[derive(Clone, Debug)]
pub struct Framebuffer {
    pub image: RgbaImage,
}

impl Framebuffer {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            image: RgbaImage::new(width, height),
        }
    }

    pub fn clear(&mut self, rgba: [u8; 4]) {
        for px in self.image.data.chunks_exact_mut(4) {
            px.copy_from_slice(&rgba);
        }
    }

    pub fn width(&self) -> u32 {
        self.image.width
    }

    pub fn height(&self) -> u32 {
        self.image.height
    }

    pub fn to_png(&self) -> Result<Vec<u8>> {
        encode_png(&self.image)
    }

    /// Shift the image by `(dx, dy)` with edge clamping. Used for
    /// `.shakeScreen`, which shakes the final composited screen.
    pub fn shifted(&self, dx: i32, dy: i32) -> RgbaImage {
        let w = self.image.width as i32;
        let h = self.image.height as i32;
        let mut out = RgbaImage::new(self.image.width, self.image.height);
        for y in 0..h {
            let sy = (y - dy).clamp(0, h - 1) as u32;
            for x in 0..w {
                let sx = (x - dx).clamp(0, w - 1) as u32;
                let px = self.image.pixel(sx, sy);
                out.set_pixel(x as u32, y as u32, px);
            }
        }
        out
    }

    /// Blend `src` over this framebuffer with a global opacity.
    pub fn blend_over(&mut self, src: &RgbaImage, opacity: u8) {
        self.image.blit(src, 0, 0, opacity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_clamps_edges() {
        let mut fb = Framebuffer::new(2, 2);
        fb.image.set_pixel(0, 0, [255, 0, 0, 255]);
        let out = fb.shifted(1, 1);
        assert_eq!(out.pixel(1, 1), [255, 0, 0, 255]);
        assert_eq!(out.pixel(0, 0), [255, 0, 0, 255]);
    }

    #[test]
    fn clear_and_png() {
        let mut fb = Framebuffer::new(8, 8);
        fb.clear([1, 2, 3, 255]);
        let png = fb.to_png().unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    }
}
