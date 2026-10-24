//! Deterministic firefly particle system for `.effect Firefly Firefly 30 6000`.

use eden_paz::image::RgbaImage;

#[derive(Clone, Debug, PartialEq)]
pub struct Firefly {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub phase: f32,
    pub size: f32,
}

/// A bounded particle system. The RNG is a xorshift32 so screenshots are
/// reproducible across runs and platforms.
#[derive(Clone, Debug)]
pub struct FireflySystem {
    pub particles: Vec<Firefly>,
    pub frames_left: u32,
    pub width: u32,
    pub height: u32,
    rng: u32,
    tick: u32,
}

impl FireflySystem {
    pub fn new(count: u32, frames: u32, seed: u32) -> Self {
        Self::with_bounds(count, frames, seed, 1024, 640)
    }

    pub fn with_bounds(count: u32, frames: u32, seed: u32, width: u32, height: u32) -> Self {
        let mut sys = Self {
            particles: Vec::with_capacity(count as usize),
            frames_left: frames,
            width,
            height,
            rng: seed.max(1),
            tick: 0,
        };
        for _ in 0..count {
            let x = sys.rand() * width as f32;
            let y = sys.rand() * height as f32;
            let vx = (sys.rand() - 0.5) * 0.35;
            let vy = -0.12 - sys.rand() * 0.35;
            let phase = sys.rand() * std::f32::consts::TAU;
            let size = 6.0 + sys.rand() * 10.0;
            sys.particles.push(Firefly {
                x,
                y,
                vx,
                vy,
                phase,
                size,
            });
        }
        sys
    }

    fn rand(&mut self) -> f32 {
        // xorshift32
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x & 0x00FF_FFFF) as f32 / 0x0100_0000 as f32
    }

    pub fn is_active(&self) -> bool {
        self.frames_left > 0
    }

    pub fn tick(&mut self) {
        if self.frames_left == 0 {
            return;
        }
        self.frames_left -= 1;
        self.tick = self.tick.wrapping_add(1);
        let w = self.width as f32;
        let h = self.height as f32;
        for p in &mut self.particles {
            p.x += p.vx + (p.phase * 0.7).sin() * 0.12;
            p.y += p.vy;
            p.phase += 0.035 + (p.vx.abs() * 0.01);
            if p.y < -20.0 {
                p.y = h + 20.0;
                p.x = (p.x + 137.0) % w;
            }
            if p.x < -20.0 {
                p.x = w + 20.0;
            } else if p.x > w + 20.0 {
                p.x = -20.0;
            }
        }
    }

    /// Additive glow pass. Fireflies are drawn over the stage and movie but
    /// below the message panel.
    pub fn draw(&self, out: &mut RgbaImage) {
        if self.frames_left == 0 {
            return;
        }
        for p in &self.particles {
            let pulse = 0.55 + 0.45 * (p.phase * 1.7).sin();
            let radius = p.size * (0.75 + 0.25 * (p.phase * 0.9).cos());
            let cx = p.x;
            let cy = p.y;
            let x0 = (cx - radius).floor().max(0.0) as u32;
            let y0 = (cy - radius).floor().max(0.0) as u32;
            let x1 = (cx + radius).ceil().min(out.width as f32) as u32;
            let y1 = (cy + radius).ceil().min(out.height as f32) as u32;
            let r2 = radius * radius;
            for y in y0..y1 {
                for x in x0..x1 {
                    let dx = x as f32 + 0.5 - cx;
                    let dy = y as f32 + 0.5 - cy;
                    let d2 = dx * dx + dy * dy;
                    if d2 >= r2 {
                        continue;
                    }
                    let falloff = 1.0 - (d2 / r2).sqrt();
                    let a = falloff * falloff * pulse;
                    // Warm yellow-green, with a white-hot core.
                    let core = if d2 < (radius * 0.18) * (radius * 0.18) {
                        0.9
                    } else {
                        0.0
                    };
                    let intensity = (a * 0.85 + core).min(1.0);
                    let add = [
                        (255.0 * intensity) as u8,
                        (232.0 * intensity) as u8,
                        (120.0 * intensity) as u8,
                    ];
                    let i = out.index(x, y);
                    for c in 0..3 {
                        out.data[i + c] = out.data[i + c].saturating_add(add[c]);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_bounded() {
        let mut a = FireflySystem::new(30, 6000, 0xEDE);
        let mut b = FireflySystem::new(30, 6000, 0xEDE);
        for _ in 0..500 {
            a.tick();
            b.tick();
        }
        assert_eq!(a.particles, b.particles);
        for p in &a.particles {
            assert!(p.x > -30.0 && p.x < 1054.0, "x={}", p.x);
            assert!(p.y > -30.0 && p.y < 670.0, "y={}", p.y);
        }
    }

    #[test]
    fn draws_additive_glow() {
        let sys = FireflySystem::new(30, 100, 1);
        let mut img = RgbaImage::filled(1024, 640, [0, 0, 0, 255]);
        sys.draw(&mut img);
        let bright = img
            .data
            .chunks_exact(4)
            .filter(|p| p[0] > 0 || p[1] > 0)
            .count();
        assert!(bright > 100, "bright pixels: {bright}");
    }

    #[test]
    fn expires_after_frame_budget() {
        let mut sys = FireflySystem::new(1, 3, 1);
        for _ in 0..3 {
            sys.tick();
        }
        assert!(!sys.is_active());
        let mut img = RgbaImage::filled(16, 16, [0, 0, 0, 255]);
        sys.draw(&mut img);
        assert!(img.data.chunks_exact(4).all(|p| p[0] == 0));
    }
}
