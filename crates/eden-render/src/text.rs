//! CJK + Latin text layout/rasterisation via `fontdue`.

use std::path::{Path, PathBuf};

use eden_paz::image::RgbaImage;
use fontdue::{Font, FontSettings};

#[derive(Clone, Debug, PartialEq)]
pub struct TextLayout {
    pub width: f32,
    pub height: f32,
    pub lines: Vec<String>,
}

pub struct FontRenderer {
    font: Font,
    path: PathBuf,
}

impl std::fmt::Debug for FontRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontRenderer")
            .field("path", &self.path)
            .finish()
    }
}

impl FontRenderer {
    pub fn from_bytes(bytes: Vec<u8>, path: impl Into<PathBuf>) -> Result<Self, String> {
        let font = Font::from_bytes(bytes, FontSettings::default())
            .map_err(|e| format!("fontdue: {e}"))?;
        Ok(Self {
            font,
            path: path.into(),
        })
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_bytes(bytes, path)
    }

    /// Load a CJK-capable system font. Returns `None` when no candidate is
    /// installed (the headless tests then skip the text assertions).
    pub fn load_system() -> Option<Self> {
        let mut candidates = Vec::new();
        if let Ok(p) = std::env::var("EDEN_FONT") {
            candidates.push(PathBuf::from(p));
        }
        candidates.extend(
            [
                "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
                "/usr/share/fonts/opentype/noto/NotoSerifCJK-Regular.ttc",
                "/System/Library/Fonts/PingFang.ttc",
                // iOS 12–14 system font locations.
                "/System/Library/Fonts/LanguageSupport/PingFang.ttc",
                "/System/Library/Fonts/Core/PingFang.ttc",
                "/System/Library/Fonts/Supplemental/Songti.ttc",
                "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            ]
            .iter()
            .map(PathBuf::from),
        );
        for p in candidates {
            if p.exists() {
                if let Ok(f) = Self::from_path(&p) {
                    return Some(f);
                }
            }
        }
        None
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn advance(&self, ch: char, size: f32) -> f32 {
        self.font.metrics(ch, size).advance_width
    }

    pub fn measure_line(&self, text: &str, size: f32) -> f32 {
        text.chars().map(|c| self.advance(c, size)).sum()
    }

    /// Wrap at `max_width` by character (works for CJK and Latin). Explicit
    /// `\n` always starts a new line.
    pub fn layout(&self, text: &str, size: f32, max_width: f32) -> TextLayout {
        let mut lines = Vec::new();
        for paragraph in text.split('\n') {
            if paragraph.is_empty() {
                lines.push(String::new());
                continue;
            }
            let mut line = String::new();
            let mut width = 0.0f32;
            for ch in paragraph.chars() {
                let adv = self.advance(ch, size);
                if max_width > 0.0 && width + adv > max_width && !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                    width = 0.0;
                }
                line.push(ch);
                width += adv;
            }
            lines.push(line);
        }
        let width = lines
            .iter()
            .map(|l| self.measure_line(l, size))
            .fold(0.0f32, f32::max);
        TextLayout {
            width,
            height: lines.len() as f32 * size * 1.35,
            lines,
        }
    }

    /// Draw text with the top-left at `(x, y)`. `color[3]` is the global alpha.
    pub fn draw_text(
        &self,
        out: &mut RgbaImage,
        text: &str,
        x: f32,
        y: f32,
        size: f32,
        color: [u8; 4],
        max_width: f32,
    ) -> TextLayout {
        let layout = self.layout(text, size, max_width);
        let line_height = size * 1.35;
        for (li, line) in layout.lines.iter().enumerate() {
            let mut pen_x = x;
            let baseline = y + size + li as f32 * line_height;
            for ch in line.chars() {
                let (metrics, bitmap) = self.font.rasterize(ch, size);
                if metrics.width > 0 && metrics.height > 0 {
                    let gx = pen_x + metrics.xmin as f32;
                    let gy = baseline - metrics.ymin as f32 - metrics.height as f32;
                    for row in 0..metrics.height {
                        for col in 0..metrics.width {
                            let coverage = bitmap[row * metrics.width + col];
                            if coverage == 0 {
                                continue;
                            }
                            let px = gx.round() as i32 + col as i32;
                            let py = gy.round() as i32 + row as i32;
                            if px < 0 || py < 0 {
                                continue;
                            }
                            let a = (u16::from(coverage) * u16::from(color[3]) / 255) as u8;
                            if a == 0 {
                                continue;
                            }
                            out.blend_pixel(
                                px as u32,
                                py as u32,
                                [color[0], color[1], color[2], a],
                                255,
                            );
                        }
                    }
                }
                pen_x += metrics.advance_width;
            }
        }
        layout
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_font_renders_latin_and_cjk() {
        let Some(font) = FontRenderer::load_system() else {
            eprintln!("skip: no system font found");
            return;
        };
        let mut img = RgbaImage::filled(600, 200, [0, 0, 0, 255]);
        let layout = font.draw_text(
            &mut img,
            "eden* 测试 中文",
            10.0,
            10.0,
            40.0,
            [255, 255, 255, 255],
            560.0,
        );
        assert!(layout.width > 0.0);
        assert_eq!(layout.lines.len(), 1);
        let lit = img
            .data
            .chunks_exact(4)
            .filter(|p| p[0] > 0 || p[1] > 0 || p[2] > 0)
            .count();
        assert!(lit > 200, "only {lit} lit pixels");
    }

    #[test]
    fn wraps_long_text() {
        let Some(font) = FontRenderer::load_system() else {
            return;
        };
        let text = "The quick brown fox jumps over the lazy dog. ".repeat(4);
        let layout = font.layout(&text, 24.0, 200.0);
        assert!(layout.lines.len() > 2);
        assert!(layout
            .lines
            .iter()
            .all(|l| font.measure_line(l, 24.0) <= 220.0));
    }

    #[test]
    fn bundled_ios_font_covers_chinese() {
        // The iOS host copies this file to Payload/Eden.app/font.ttf.
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../platforms/ios/EdenApp/font.ttf");
        if !path.exists() {
            eprintln!("skip: bundled font missing");
            return;
        }
        let font = FontRenderer::from_path(&path).expect("bundled font must parse");
        for ch in "中文测试，。！？「」…—ABCDEFG0123".chars() {
            assert!(
                font.advance(ch, 28.0) > 0.0,
                "bundled font has no advance for {ch:?}"
            );
        }
        let mut img = RgbaImage::filled(400, 120, [0, 0, 0, 255]);
        let layout = font.draw_text(&mut img, "中文测试", 8.0, 8.0, 32.0, [255; 4], 380.0);
        assert_eq!(layout.lines, vec!["中文测试"]);
        let lit = img
            .data
            .chunks_exact(4)
            .filter(|p| p[0] > 0 || p[1] > 0 || p[2] > 0)
            .count();
        assert!(lit > 300, "bundled font rendered only {lit} lit pixels");
    }

    #[test]
    fn newline_forces_a_line() {
        let Some(font) = FontRenderer::load_system() else {
            return;
        };
        let layout = font.layout("a\nb\nc", 20.0, 1000.0);
        assert_eq!(layout.lines, vec!["a", "b", "c"]);
    }
}
