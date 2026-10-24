//! Deterministic framebuffer reference renderer.

use eden_paz::image::{encode_png, RgbaImage};
use eden_paz::Result;

use crate::scene::Scene;
use crate::text::FontRenderer;
use crate::texture::TextureCache;

/// Rule-mask transition alpha.
///
/// `alpha = clamp((luma - (1 - progress)) * softness + 0.5, 0, 1)`
///
/// `luma` and `progress` are in 0..=1.
pub fn rule_alpha(luma: f32, progress: f32, softness: f32) -> f32 {
    ((luma - (1.0 - progress)) * softness + 0.5).clamp(0.0, 1.0)
}

pub struct SoftwareRenderer {
    pub width: u32,
    pub height: u32,
    pub font: Option<FontRenderer>,
    pub cache: TextureCache,
    pub panel_visible_default: bool,
}

impl SoftwareRenderer {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            font: None,
            cache: TextureCache::new(256 * 1024 * 1024),
            panel_visible_default: true,
        }
    }

    pub fn with_system_font(mut self) -> Self {
        self.font = FontRenderer::load_system();
        self
    }

    pub fn set_font(&mut self, font: FontRenderer) {
        self.font = Some(font);
    }

    /// Render the current scene without the transition pass (used as the
    /// "from" image when a transition starts).
    pub fn capture(&self, scene: &Scene) -> RgbaImage {
        self.render_base(scene)
    }

    /// Start a transition, capturing the current frame first.
    pub fn begin_transition(
        &mut self,
        scene: &mut Scene,
        kind: i32,
        mask: Option<std::sync::Arc<RgbaImage>>,
        frames: u32,
    ) {
        let from = self.render_base(scene);
        scene.start_transition(kind, mask, frames, from);
    }

    pub fn render(&mut self, scene: &Scene) -> RgbaImage {
        // Transitions belong to the stage layer: the message panel is UI and
        // stays on top of the wipe instead of fading out with the background.
        let mut out = self.render_base(scene);
        if let Some(t) = &scene.transition {
            if let Some(from) = &t.from {
                out = blend_transition(from, &out, t, self.width, self.height);
            }
        }
        if scene.panel.visible {
            self.draw_panel(&mut out, scene);
        }
        let (dx, dy) = scene.shake_offset();
        if dx != 0 || dy != 0 {
            out = crate::framebuffer::Framebuffer { image: out }.shifted(dx, dy);
        }
        out
    }

    pub fn render_to_png(&mut self, scene: &Scene) -> Result<Vec<u8>> {
        encode_png(&self.render(scene))
    }

    /// Stage-only raster used for transition capture. The panel is composited
    /// later by [`Self::render`] so it survives background wipes.
    fn render_base(&self, scene: &Scene) -> RgbaImage {
        let mut out = RgbaImage::new(self.width, self.height);
        scene.render_into(&mut out);
        out
    }

    fn draw_panel(&self, out: &mut RgbaImage, scene: &Scene) {
        let panel = &scene.panel;
        let (x0, y0, x1, _y1) = if let Some(image) = &panel.image {
            let y = self.height as i32 - image.height as i32;
            out.blit(image, 0, y, panel.opacity);
            (56, y + 30, self.width as i32 - 56, self.height as i32 - 16)
        } else {
            let x0 = 40i32;
            let y0 = 460i32;
            let x1 = self.width as i32 - 40;
            let y1 = self.height as i32 - 24;
            let radius = 10i32;
            let base = [12u8, 16, 26];
            let border = [96u8, 120, 156];
            for y in y0..y1 {
                for x in x0..x1 {
                    // Rounded corners.
                    let cx = if x < x0 + radius {
                        x0 + radius
                    } else if x >= x1 - radius {
                        x1 - radius - 1
                    } else {
                        x
                    };
                    let cy = if y < y0 + radius {
                        y0 + radius
                    } else if y >= y1 - radius {
                        y1 - radius - 1
                    } else {
                        y
                    };
                    let d = (x - cx) * (x - cx) + (y - cy) * (y - cy);
                    if d > radius * radius {
                        continue;
                    }
                    let edge = x <= x0 + 1 || x >= x1 - 2 || y <= y0 + 1 || y >= y1 - 2;
                    let color = if edge { border } else { base };
                    out.blend_pixel(
                        x as u32,
                        y as u32,
                        [color[0], color[1], color[2], panel.opacity],
                        255,
                    );
                }
            }
            (x0, y0, x1, y1)
        };
        if let Some(msg) = &panel.message {
            if let Some(font) = &self.font {
                let visible = msg.visible();
                font.draw_text(
                    out,
                    &visible,
                    (x0 + 24) as f32,
                    (y0 + 20) as f32,
                    msg.font_size,
                    [238, 240, 245, 255],
                    (x1 - x0 - 48) as f32,
                );
            }
        }
    }
}

fn blend_transition(
    from: &RgbaImage,
    to: &RgbaImage,
    t: &crate::scene::Transition,
    w: u32,
    h: u32,
) -> RgbaImage {
    let p = t.progress();
    let mut out = RgbaImage::new(w, h);
    if t.kind == 0 || t.mask.is_none() {
        // Cross-fade.
        for y in 0..h {
            for x in 0..w {
                let a = from.pixel(x, y);
                let b = to.pixel(x, y);
                let mix =
                    |i: usize| -> u8 { (a[i] as f32 * (1.0 - p) + b[i] as f32 * p).round() as u8 };
                out.set_pixel(x, y, [mix(0), mix(1), mix(2), 255]);
            }
        }
        return out;
    }
    let mask = t.mask.as_ref().unwrap();
    for y in 0..h {
        for x in 0..w {
            let u = (x as f32 + 0.5) / w as f32;
            let v = (y as f32 + 0.5) / h as f32;
            let luma = f32::from(mask.luma_bilinear(u, v)) / 255.0;
            let a = rule_alpha(luma, p, 8.0);
            let f = from.pixel(x, y);
            let b = to.pixel(x, y);
            let mix = |i: usize| -> u8 {
                (f[i] as f32 * (1.0 - a) + b[i] as f32 * a)
                    .round()
                    .clamp(0.0, 255.0) as u8
            };
            out.set_pixel(x, y, [mix(0), mix(1), mix(2), 255]);
        }
    }
    out
}

/// Blend two images for the GPU path's CPU-side tests.
pub fn crossfade(a: &RgbaImage, b: &RgbaImage, progress: f32) -> RgbaImage {
    let p = progress.clamp(0.0, 1.0);
    let mut out = RgbaImage::new(a.width.min(b.width), a.height.min(b.height));
    for y in 0..out.height {
        for x in 0..out.width {
            let pa = a.pixel(x, y);
            let pb = b.pixel(x, y);
            let mix =
                |i: usize| -> u8 { (pa[i] as f32 * (1.0 - p) + pb[i] as f32 * p).round() as u8 };
            out.set_pixel(x, y, [mix(0), mix(1), mix(2), 255]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{BlendMode, MessageOverlay, Scene};
    use eden_paz::PazArchive;
    use std::sync::Arc;

    fn game_dir() -> Option<std::path::PathBuf> {
        let p = std::path::PathBuf::from("/mnt/f/gal/【PC硬盘】【官中】eden");
        if p.exists() {
            Some(p)
        } else {
            None
        }
    }

    #[test]
    fn rule_alpha_formula() {
        assert_eq!(rule_alpha(0.0, 0.0, 8.0), 0.0);
        assert_eq!(rule_alpha(1.0, 1.0, 8.0), 1.0);
        assert!((rule_alpha(0.5, 0.5, 8.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn crossfade_midpoint() {
        let a = RgbaImage::filled(2, 2, [0, 0, 0, 255]);
        let b = RgbaImage::filled(2, 2, [200, 100, 50, 255]);
        let m = crossfade(&a, &b, 0.5);
        assert_eq!(m.pixel(0, 0), [100, 50, 25, 255]);
    }

    #[test]
    fn real_ani_overlay_matches_pillow_hash() {
        let Some(dir) = game_dir() else {
            eprintln!("skip: game dir missing");
            return;
        };
        let a = PazArchive::open(dir.join("bg.paz")).unwrap();
        let base = a.decode_png("ev_add02_up06a.png").unwrap();
        let ani = a.parse_ani("ev_add02_up06a.ani").unwrap();
        let mut scene = Scene::new(1024, 640);
        scene.set_stage(Arc::new(base), 0, 0, None);
        scene.set_stage_ani(Arc::new(ani), 0, 255);
        let mut r = SoftwareRenderer::new(1024, 640);
        let out = r.render(&scene);
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(&out.data);
        assert_eq!(
            format!("{:x}", h.finalize()),
            "e63d64f7b0248270612d11d7f163beea287d9f1126ead582b9a5be781e791ffe"
        );
    }

    #[test]
    fn transition_crossfade_renders() {
        let mut scene = Scene::new(8, 8);
        let from = RgbaImage::filled(8, 8, [0, 0, 0, 255]);
        scene.start_transition(0, None, 2, from);
        let mut r = SoftwareRenderer::new(8, 8);
        scene.tick();
        let mid = r.render(&scene);
        assert_eq!(mid.pixel(0, 0), [0, 0, 0, 255]);
        scene.tick();
        let end = r.render(&scene);
        assert_eq!(end.pixel(0, 0), [0, 0, 0, 255]);
    }

    #[test]
    fn rule_transition_uses_mask() {
        let mut mask = RgbaImage::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                let v = if x < 2 { 0 } else { 255 };
                mask.set_pixel(x, y, [v, v, v, 255]);
            }
        }
        let from = RgbaImage::filled(4, 4, [0, 0, 0, 255]);
        let to = RgbaImage::filled(4, 4, [255, 255, 255, 255]);
        let t = crate::scene::Transition {
            kind: 1,
            mask: Some(Arc::new(mask)),
            total_frames: 2,
            elapsed: 1,
            from: Some(from.clone()),
        };
        let out = blend_transition(&from, &to, &t, 4, 4);
        assert_eq!(out.pixel(0, 0), [0, 0, 0, 255]);
        assert_eq!(out.pixel(3, 0), [255, 255, 255, 255]);
    }

    #[test]
    fn panel_text_renders() {
        let Some(font) = FontRenderer::load_system() else {
            eprintln!("skip: no system font");
            return;
        };
        let mut scene = Scene::new(1024, 640);
        scene.set_panel(true, Some(MessageOverlay::new("Hello eden* 中文测试")));
        let mut r = SoftwareRenderer::new(1024, 640);
        r.set_font(font);
        let out = r.render(&scene);
        let lit = (460..616)
            .flat_map(|y| (40..984).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let p = out.pixel(x, y);
                p[0] > 150 && p[1] > 150 && p[2] > 150
            })
            .count();
        assert!(lit > 100, "lit text pixels: {lit}");
    }

    #[test]
    fn screenshot_png_roundtrip() {
        let scene = Scene::new(32, 16);
        let mut r = SoftwareRenderer::new(32, 16);
        let png = r.render_to_png(&scene).unwrap();
        let img = eden_paz::decode_png(&png).unwrap();
        assert_eq!(img.width, 32);
        assert_eq!(img.height, 16);
    }

    #[test]
    fn panel_image_is_anchored_to_the_bottom() {
        let mut scene = Scene::new(16, 10);
        scene.set_panel(true, None);
        scene.panel.opacity = 255;
        scene.set_panel_image(Some(Arc::new(RgbaImage::filled(16, 4, [10, 20, 30, 255]))));
        let mut r = SoftwareRenderer::new(16, 10);
        let out = r.render(&scene);
        assert_eq!(out.pixel(0, 0), [0, 0, 0, 255]);
        assert_eq!(out.pixel(0, 6), [10, 20, 30, 255]);
        assert_eq!(out.pixel(15, 9), [10, 20, 30, 255]);
    }

    #[test]
    fn blend_modes() {
        // `render_into` always clears the target first, so exercise blending
        // through two scene layers: an opaque base plus an additive top.
        let base = Arc::new(RgbaImage::filled(1, 1, [100, 100, 100, 255]));
        let add = Arc::new(RgbaImage::filled(1, 1, [50, 50, 50, 255]));
        let mut s = Scene::new(1, 1);
        s.layers.push(crate::scene::Layer {
            name: "base".into(),
            image: base,
            x: 0,
            y: 0,
            opacity: 255,
            blend: BlendMode::Normal,
            z: 0,
            mask: None,
            ani: None,
        });
        s.layers.push(crate::scene::Layer {
            name: "add".into(),
            image: add,
            x: 0,
            y: 0,
            opacity: 255,
            blend: BlendMode::Add,
            z: 1,
            mask: None,
            ani: None,
        });
        let mut dst = RgbaImage::new(1, 1);
        s.render_into(&mut dst);
        assert_eq!(dst.pixel(0, 0), [150, 150, 150, 255]);
    }
}
