//! Musica scene graph: 1024x640 stage, viewport, layers, masks, `.ani`
//! overlays, transitions, shake and fireflies.

use std::sync::Arc;

use eden_paz::ani::AniFile;
use eden_paz::image::RgbaImage;

use crate::particles::FireflySystem;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlendMode {
    Normal,
    Add,
    Multiply,
    Screen,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageSpec {
    pub image_name: String,
    pub mask_name: Option<String>,
    pub x: i32,
    pub y: i32,
}

impl Default for StageSpec {
    fn default() -> Self {
        Self {
            image_name: "BLACK.png".into(),
            mask_name: None,
            x: 0,
            y: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AniOverlay {
    pub ani: Arc<AniFile>,
    pub frame: usize,
    pub opacity: u8,
}

#[derive(Clone, Debug)]
pub struct Layer {
    pub name: String,
    pub image: Arc<RgbaImage>,
    pub x: i32,
    pub y: i32,
    pub opacity: u8,
    pub blend: BlendMode,
    pub z: i32,
    pub mask: Option<Arc<RgbaImage>>,
    pub ani: Option<AniOverlay>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MessageOverlay {
    pub text: String,
    pub shown_chars: usize,
    pub font_size: f32,
}

impl MessageOverlay {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let n = text.chars().count();
        Self {
            text,
            shown_chars: n,
            font_size: 24.0,
        }
    }

    pub fn visible(&self) -> String {
        self.text.chars().take(self.shown_chars).collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PanelOverlay {
    pub visible: bool,
    pub message: Option<MessageOverlay>,
    pub opacity: u8,
    /// Real `msgPanel.png` / `fullPanel.png` background, when available.
    pub image: Option<Arc<RgbaImage>>,
}

impl Default for PanelOverlay {
    fn default() -> Self {
        Self {
            visible: false,
            message: None,
            opacity: 220,
            image: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShakeKind {
    Random,
    Sine,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShakeState {
    pub kind: ShakeKind,
    pub amplitude: i32,
    pub frames: u32,
    pub elapsed: u32,
}

impl ShakeState {
    pub fn offset(&self) -> (i32, i32) {
        if self.frames == 0 || self.elapsed >= self.frames {
            return (0, 0);
        }
        let t = self.elapsed as f32 / self.frames as f32;
        let decay = (1.0 - t).max(0.0);
        let amp = self.amplitude as f32 * decay;
        match self.kind {
            ShakeKind::Random => {
                let mut x = self
                    .elapsed
                    .wrapping_mul(747_796_405)
                    .wrapping_add(2_891_336_453);
                x ^= x >> 15;
                let mut y = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                y ^= y >> 13;
                let dx = ((x % 2001) as f32 / 1000.0 - 1.0) * amp;
                let dy = ((y % 2001) as f32 / 1000.0 - 1.0) * amp * 0.6;
                (dx.round() as i32, dy.round() as i32)
            }
            ShakeKind::Sine => {
                let dx = (self.elapsed as f32 * 0.9).sin() * amp;
                let dy = (self.elapsed as f32 * 1.37 + 1.7).sin() * amp * 0.5;
                (dx.round() as i32, dy.round() as i32)
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Transition {
    /// 0 = cross-fade, 1 = rule mask.
    pub kind: i32,
    pub mask: Option<Arc<RgbaImage>>,
    pub total_frames: u32,
    pub elapsed: u32,
    pub from: Option<Arc<RgbaImage>>,
}

impl Transition {
    pub fn progress(&self) -> f32 {
        self.progress_with_sub(0.0)
    }

    /// Progress including a sub-frame fraction in `0..1`, used by the
    /// 120 Hz display link to interpolate between two 60 Hz VM ticks.
    pub fn progress_with_sub(&self, sub: f32) -> f32 {
        if self.total_frames == 0 {
            1.0
        } else {
            ((self.elapsed as f32 + sub.clamp(0.0, 1.0)) / self.total_frames as f32)
                .clamp(0.0, 1.0)
        }
    }
}

pub struct Scene {
    pub width: u32,
    pub height: u32,
    pub view_x: i32,
    pub view_y: i32,
    /// Sub-pixel viewport offset (0..1) used at 120 Hz. The stage artwork is
    /// sampled with bilinear filtering so ProMotion scrolls stay smooth
    /// between two 60 Hz logical frames.
    pub view_sub_x: f32,
    pub view_sub_y: f32,
    /// Sub-frame transition progress (0..1) for the same reason.
    pub transition_sub: f32,
    pub stage_spec: StageSpec,
    pub layers: Vec<Layer>,
    pub panel: PanelOverlay,
    pub shake: Option<ShakeState>,
    pub fireflies: Option<FireflySystem>,
    pub transition: Option<Transition>,
}

impl Default for Scene {
    fn default() -> Self {
        Self::new(1024, 640)
    }
}

impl Scene {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            view_x: 0,
            view_y: 0,
            view_sub_x: 0.0,
            view_sub_y: 0.0,
            transition_sub: 0.0,
            stage_spec: StageSpec::default(),
            layers: Vec::new(),
            panel: PanelOverlay::default(),
            shake: None,
            fireflies: None,
            transition: None,
        }
    }

    pub fn set_view(&mut self, x: i32, y: i32) {
        self.view_x = x;
        self.view_y = y;
    }

    /// Sub-pixel viewport offset applied on top of [`Self::set_view`].
    /// Values are clamped to one frame of the fastest retail scroll speed: a
    /// scroll moving towards a lower offset runs slightly ahead of the last
    /// logical frame, then the next 60 Hz tick catches up exactly.
    pub fn set_view_subpixel(&mut self, x: f32, y: f32) {
        self.view_sub_x = x.clamp(-64.0, 64.0);
        self.view_sub_y = y.clamp(-64.0, 64.0);
    }

    pub fn set_transition_subpixel(&mut self, sub: f32) {
        self.transition_sub = sub.clamp(0.0, 1.0);
    }

    pub fn clear_subpixel(&mut self) {
        self.view_sub_x = 0.0;
        self.view_sub_y = 0.0;
        self.transition_sub = 0.0;
    }

    pub fn set_stage_spec(
        &mut self,
        image_name: impl Into<String>,
        mask_name: Option<String>,
        x: i32,
        y: i32,
    ) {
        self.stage_spec = StageSpec {
            image_name: image_name.into(),
            mask_name,
            x,
            y,
        };
    }

    pub fn set_stage(
        &mut self,
        image: Arc<RgbaImage>,
        x: i32,
        y: i32,
        mask: Option<Arc<RgbaImage>>,
    ) {
        self.stage_spec.x = x;
        self.stage_spec.y = y;
        self.layers.retain(|l| l.name != "stage");
        // `.stage` x/y are the initial *viewport offset* into the artwork
        // (the engine uses them for oversized CGs and for the starting point
        // of `.vscroll`). The layer itself always starts at the origin and
        // the scene's view is subtracted from it while blitting.
        self.layers.push(Layer {
            name: "stage".into(),
            image,
            x: 0,
            y: 0,
            opacity: 255,
            blend: BlendMode::Normal,
            z: 0,
            mask,
            ani: None,
        });
    }

    pub fn set_stage_ani(&mut self, ani: Arc<AniFile>, frame: usize, opacity: u8) {
        if let Some(layer) = self.layers.iter_mut().find(|l| l.name == "stage") {
            layer.ani = Some(AniOverlay {
                ani,
                frame,
                opacity,
            });
        }
    }

    pub fn set_stage_ani_frame(&mut self, frame: usize) {
        if let Some(layer) = self.layers.iter_mut().find(|l| l.name == "stage") {
            if let Some(ani) = &mut layer.ani {
                ani.frame = frame;
            }
        }
    }

    pub fn set_movie(&mut self, image: Option<Arc<RgbaImage>>) {
        self.layers.retain(|l| l.name != "movie");
        if let Some(image) = image {
            self.layers.push(Layer {
                name: "movie".into(),
                image,
                x: 0,
                y: 0,
                opacity: 255,
                blend: BlendMode::Normal,
                z: 3,
                mask: None,
                ani: None,
            });
        }
    }

    pub fn set_panel(&mut self, visible: bool, message: Option<MessageOverlay>) {
        self.panel.visible = visible;
        self.panel.message = message;
    }

    /// Sets the message-box background (`msgPanel.png`, `fullPanel.png`, ...).
    pub fn set_panel_image(&mut self, image: Option<Arc<RgbaImage>>) {
        self.panel.image = image;
    }

    pub fn start_shake(&mut self, kind: ShakeKind, amplitude: i32, frames: u32) {
        self.shake = Some(ShakeState {
            kind,
            amplitude,
            frames,
            elapsed: 0,
        });
    }

    pub fn start_firefly(&mut self, count: u32, frames: u32) {
        self.fireflies = Some(FireflySystem::with_bounds(
            count,
            frames,
            0xEDE_1234,
            self.width,
            self.height,
        ));
    }

    pub fn stop_fireflies(&mut self) {
        self.fireflies = None;
    }

    pub fn start_transition(
        &mut self,
        kind: i32,
        mask: Option<Arc<RgbaImage>>,
        frames: u32,
        from: RgbaImage,
    ) {
        self.transition = Some(Transition {
            kind,
            mask,
            total_frames: frames,
            elapsed: 0,
            from: Some(Arc::new(from)),
        });
    }

    pub fn tick(&mut self) {
        if let Some(shake) = &mut self.shake {
            shake.elapsed = shake.elapsed.saturating_add(1);
            if shake.elapsed >= shake.frames {
                self.shake = None;
            }
        }
        if let Some(f) = &mut self.fireflies {
            f.tick();
            if !f.is_active() {
                self.fireflies = None;
            }
        }
        if let Some(t) = &mut self.transition {
            t.elapsed = t.elapsed.saturating_add(1);
            if t.elapsed > t.total_frames {
                self.transition = None;
            }
        }
    }

    pub fn shake_offset(&self) -> (i32, i32) {
        self.shake.as_ref().map(|s| s.offset()).unwrap_or((0, 0))
    }

    /// Draw the stage/movie/firefly layers. The panel and transition are
    /// handled by the renderer.
    pub fn render_into(&self, out: &mut RgbaImage) {
        for px in out.data.chunks_exact_mut(4) {
            px.copy_from_slice(&[0, 0, 0, 255]);
        }
        let mut order: Vec<&Layer> = self.layers.iter().collect();
        order.sort_by_key(|l| l.z);
        for layer in order {
            let is_stage = layer.name == "stage";
            let sx_f = if is_stage {
                -(self.view_x as f32 + self.view_sub_x)
            } else {
                (layer.x - self.view_x) as f32
            };
            let sy_f = if is_stage {
                -(self.view_y as f32 + self.view_sub_y)
            } else {
                (layer.y - self.view_y) as f32
            };
            let sx = sx_f.floor() as i32;
            let sy = sy_f.floor() as i32;
            let subpixel = sx_f.fract() != 0.0 || sy_f.fract() != 0.0;
            if subpixel {
                blit_stage_bilinear(
                    out,
                    &layer.image,
                    sx_f,
                    sy_f,
                    layer.mask.as_deref(),
                    layer.opacity,
                );
            } else if let Some(mask) = &layer.mask {
                blit_masked(out, &layer.image, sx, sy, mask, layer.opacity);
            } else {
                blit_blend(out, &layer.image, sx, sy, layer.opacity, layer.blend);
            }
            if let Some(ani) = &layer.ani {
                if let Some(frame) = ani.ani.frames.get(ani.frame) {
                    let ax = sx + i32::from(frame.origin_x);
                    let ay = sy + i32::from(frame.origin_y);
                    if let Some(mask) = &layer.mask {
                        blit_bgra_masked(
                            out,
                            &frame.pixels,
                            u32::from(frame.width),
                            u32::from(frame.height),
                            ax,
                            ay,
                            mask,
                            layer.opacity,
                        );
                    } else {
                        out.blit_bgra(
                            &frame.pixels,
                            u32::from(frame.width),
                            u32::from(frame.height),
                            ax,
                            ay,
                            ani.opacity,
                        );
                    }
                }
            }
        }
        if let Some(f) = &self.fireflies {
            f.draw(out);
        }
    }
}

fn blit_blend(dst: &mut RgbaImage, src: &RgbaImage, x: i32, y: i32, opacity: u8, blend: BlendMode) {
    let x0 = x.max(0) as u32;
    let y0 = y.max(0) as u32;
    let x1 = (x + src.width as i32).min(dst.width as i32).max(0) as u32;
    let y1 = (y + src.height as i32).min(dst.height as i32).max(0) as u32;
    for sy in y0..y1 {
        let src_y = (sy as i32 - y) as u32;
        for sx in x0..x1 {
            let src_x = (sx as i32 - x) as u32;
            let sp = src.pixel(src_x, src_y);
            let a = (u16::from(sp[3]) * u16::from(opacity) / 255) as u8;
            if a == 0 {
                continue;
            }
            if blend == BlendMode::Normal {
                dst.blend_pixel(sx, sy, sp, opacity);
                continue;
            }
            let i = dst.index(sx, sy);
            for c in 0..3 {
                let s = (u16::from(sp[c]) * u16::from(a) / 255) as u8;
                let d = dst.data[i + c];
                let v = match blend {
                    BlendMode::Add => d.saturating_add(s),
                    BlendMode::Multiply => ((u16::from(d) * u16::from(s) + 127) / 255) as u8,
                    BlendMode::Screen => {
                        255 - (((255 - u16::from(d)) * (255 - u16::from(s)) + 127) / 255) as u8
                    }
                    BlendMode::Normal => unreachable!(),
                };
                dst.data[i + c] = v;
            }
        }
    }
}

fn blit_masked(
    dst: &mut RgbaImage,
    src: &RgbaImage,
    x: i32,
    y: i32,
    mask: &RgbaImage,
    opacity: u8,
) {
    let x0 = x.max(0) as u32;
    let y0 = y.max(0) as u32;
    let x1 = (x + src.width as i32).min(dst.width as i32).max(0) as u32;
    let y1 = (y + src.height as i32).min(dst.height as i32).max(0) as u32;
    for sy in y0..y1 {
        let src_y = (sy as i32 - y) as u32;
        for sx in x0..x1 {
            let src_x = (sx as i32 - x) as u32;
            let u = (src_x as f32 + 0.5) / src.width as f32;
            let v = (src_y as f32 + 0.5) / src.height as f32;
            let m = mask.luma_bilinear(u, v);
            let sp = src.pixel(src_x, src_y);
            let a = (u16::from(sp[3]) * u16::from(m) / 255 * u16::from(opacity) / 255) as u8;
            dst.blend_pixel(sx, sy, [sp[0], sp[1], sp[2], a], 255);
        }
    }
}

/// Bilinear stage blit used for sub-pixel scroll offsets at 120 Hz.
///
/// `x`/`y` are the (possibly fractional) screen position of the artwork's
/// top-left corner. Pixels outside the source are transparent.
fn blit_stage_bilinear(
    dst: &mut RgbaImage,
    src: &RgbaImage,
    x: f32,
    y: f32,
    mask: Option<&RgbaImage>,
    opacity: u8,
) {
    let sw = src.width as i32;
    let sh = src.height as i32;
    for sy in 0..dst.height as i32 {
        let v = sy as f32 - y;
        for sx in 0..dst.width as i32 {
            let u = sx as f32 - x;
            let sp = sample_bilinear(src, u, v);
            if sp[3] == 0 {
                continue;
            }
            let mut a = u16::from(sp[3]);
            if let Some(mask) = mask {
                let mu = ((u + 0.5) / sw as f32).clamp(0.0, 1.0);
                let mv = ((v + 0.5) / sh as f32).clamp(0.0, 1.0);
                a = a * u16::from(mask.luma_bilinear(mu, mv)) / 255;
            }
            let a = (a * u16::from(opacity) / 255) as u8;
            if a == 0 {
                continue;
            }
            dst.blend_pixel(sx as u32, sy as u32, [sp[0], sp[1], sp[2], a], 255);
        }
    }
}

fn sample_bilinear(src: &RgbaImage, x: f32, y: f32) -> [u8; 4] {
    let xf = x.floor();
    let yf = y.floor();
    let fx = x - xf;
    let fy = y - yf;
    let xi = xf as i32;
    let yi = yf as i32;
    let texel = |dx: i32, dy: i32| -> [u8; 4] {
        let sx = xi + dx;
        let sy = yi + dy;
        if sx < 0 || sy < 0 || sx >= src.width as i32 || sy >= src.height as i32 {
            [0, 0, 0, 0]
        } else {
            src.pixel(sx as u32, sy as u32)
        }
    };
    let p00 = texel(0, 0);
    let p10 = texel(1, 0);
    let p01 = texel(0, 1);
    let p11 = texel(1, 1);
    let mut out = [0u8; 4];
    for c in 0..4 {
        let top = p00[c] as f32 * (1.0 - fx) + p10[c] as f32 * fx;
        let bottom = p01[c] as f32 * (1.0 - fx) + p11[c] as f32 * fx;
        out[c] = (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8;
    }
    out
}

fn blit_bgra_masked(
    dst: &mut RgbaImage,
    bgra: &[u8],
    w: u32,
    h: u32,
    x: i32,
    y: i32,
    mask: &RgbaImage,
    opacity: u8,
) {
    let x0 = x.max(0) as u32;
    let y0 = y.max(0) as u32;
    let x1 = (x + w as i32).min(dst.width as i32).max(0) as u32;
    let y1 = (y + h as i32).min(dst.height as i32).max(0) as u32;
    for sy in y0..y1 {
        let src_y = (sy as i32 - y) as u32;
        for sx in x0..x1 {
            let src_x = (sx as i32 - x) as u32;
            let i = (src_y as usize * w as usize + src_x as usize) * 4;
            let a_src = bgra[i + 3];
            if a_src == 0 {
                continue;
            }
            let u = (src_x as f32 + 0.5) / w as f32;
            let v = (src_y as f32 + 0.5) / h as f32;
            let m = mask.luma_bilinear(u, v);
            let a = (u16::from(a_src) * u16::from(m) / 255 * u16::from(opacity) / 255) as u8;
            dst.blend_pixel(sx, sy, [bgra[i + 2], bgra[i + 1], bgra[i], a], 255);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: u32, h: u32) -> Arc<RgbaImage> {
        let mut img = RgbaImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                img.set_pixel(x, y, [(x % 256) as u8, (y % 256) as u8, 77, 255]);
            }
        }
        Arc::new(img)
    }

    #[test]
    fn viewport_offsets_stage() {
        let img = gradient(1024, 2007);
        let mut scene = Scene::new(1024, 640);
        scene.set_stage(img.clone(), 0, 1280, None);
        // The stage's y is the initial scroll offset into the artwork.
        scene.set_view(0, 1280);
        let mut out = RgbaImage::new(1024, 640);
        scene.render_into(&mut out);
        assert_eq!(out.pixel(0, 0), img.pixel(0, 1280));
        // Scrolling up reveals earlier rows, without any empty band: the
        // viewport is a window into the image, not a world-space camera.
        scene.set_view(0, 970);
        scene.render_into(&mut out);
        assert_eq!(out.pixel(0, 0), img.pixel(0, 970));
        assert_eq!(out.pixel(0, 639), img.pixel(0, 1609));
        // Scrolling down clips the top of the image.
        scene.set_view(0, 1590);
        scene.render_into(&mut out);
        assert_eq!(out.pixel(0, 0), img.pixel(0, 1590));
        // Past the bottom edge the framebuffer stays black.
        scene.set_view(0, 2007);
        scene.render_into(&mut out);
        assert_eq!(out.pixel(0, 0), [0, 0, 0, 255]);
    }

    #[test]
    fn subpixel_view_interpolates_rows() {
        let img = gradient(1024, 2007);
        let mut scene = Scene::new(1024, 640);
        scene.set_stage(img.clone(), 0, 0, None);
        scene.set_view(0, 100);
        let mut out = RgbaImage::new(1024, 640);
        scene.render_into(&mut out);
        let integer = out.pixel(10, 0);
        scene.set_view_subpixel(0.0, 0.5);
        scene.render_into(&mut out);
        let half = out.pixel(10, 0);
        let a = img.pixel(10, 100)[1] as i32;
        let b = img.pixel(10, 101)[1] as i32;
        assert!((half[1] as i32 - (a + b) / 2).abs() <= 1, "{half:?}");
        assert_ne!(half, integer);
        scene.clear_subpixel();
        scene.render_into(&mut out);
        assert_eq!(out.pixel(10, 0), integer);
    }

    #[test]
    fn transition_subpixel_adds_a_fraction_of_a_frame() {
        let mut t = Transition {
            kind: 0,
            mask: None,
            total_frames: 30,
            elapsed: 15,
            from: None,
        };
        assert!((t.progress() - 0.5).abs() < 1e-6);
        assert!((t.progress_with_sub(0.5) - 15.5 / 30.0).abs() < 1e-6);
        t.elapsed = 30;
        assert!((t.progress_with_sub(0.5) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn mask_controls_visibility() {
        let img = Arc::new(RgbaImage::filled(4, 4, [255, 0, 0, 255]));
        let white = Arc::new(RgbaImage::filled(4, 4, [255, 255, 255, 255]));
        let black = Arc::new(RgbaImage::filled(4, 4, [0, 0, 0, 255]));
        let mut scene = Scene::new(4, 4);
        scene.set_stage(img.clone(), 0, 0, Some(white));
        let mut out = RgbaImage::new(4, 4);
        scene.render_into(&mut out);
        assert_eq!(out.pixel(1, 1), [255, 0, 0, 255]);
        scene.set_stage(img, 0, 0, Some(black));
        scene.render_into(&mut out);
        assert_eq!(out.pixel(1, 1), [0, 0, 0, 255]);
    }

    #[test]
    fn shake_decays_to_zero() {
        let mut scene = Scene::new(1024, 640);
        scene.start_shake(ShakeKind::Sine, 50, 20);
        let (x0, y0) = scene.shake_offset();
        assert!(x0 != 0 || y0 != 0);
        for _ in 0..20 {
            scene.tick();
        }
        assert_eq!(scene.shake_offset(), (0, 0));
    }
}
