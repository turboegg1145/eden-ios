//! `.message` text and control-code handling.
//!
//! The scripts use three backslash controls:
//!
//! * `\n` — hard line break
//! * `\v` — voice synchronisation marker
//! * `\a` — wait for click / advance
//!
//! Because the whole script is decoded to UTF-8 *before* this parser runs, a
//! GBK trailing `0x5C` byte can never be confused with a control backslash.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextSegment {
    Text(String),
    LineBreak,
    VoiceSync,
    WaitClick,
}

/// A `.message` payload split into renderable text and control markers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageText {
    /// The exact text after the optional `#`, with escapes intact.
    pub raw: String,
    pub segments: Vec<TextSegment>,
}

impl MessageText {
    pub fn parse(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        let mut segments = Vec::new();
        let mut buf = String::new();
        let mut chars = raw.chars().peekable();
        while let Some(c) = chars.next() {
            // PLUS+MOSAIC uses inline `#4` / `#5` layout controls (soft break,
            // spacing). They are not visible text.
            if c == '#' && chars.peek().is_some_and(|d| d.is_ascii_digit()) {
                chars.next();
                continue;
            }
            if c != '\\' {
                buf.push(c);
                continue;
            }
            match chars.peek().copied() {
                Some('n') => {
                    chars.next();
                    if !buf.is_empty() {
                        segments.push(TextSegment::Text(std::mem::take(&mut buf)));
                    }
                    segments.push(TextSegment::LineBreak);
                }
                Some('v') => {
                    chars.next();
                    if !buf.is_empty() {
                        segments.push(TextSegment::Text(std::mem::take(&mut buf)));
                    }
                    segments.push(TextSegment::VoiceSync);
                }
                Some('a') => {
                    chars.next();
                    if !buf.is_empty() {
                        segments.push(TextSegment::Text(std::mem::take(&mut buf)));
                    }
                    segments.push(TextSegment::WaitClick);
                }
                Some('\\') => {
                    chars.next();
                    buf.push('\\');
                }
                _ => buf.push('\\'),
            }
        }
        if !buf.is_empty() {
            segments.push(TextSegment::Text(buf));
        }
        Self { raw, segments }
    }

    pub fn has_wait_click(&self) -> bool {
        self.segments
            .iter()
            .any(|s| matches!(s, TextSegment::WaitClick))
    }

    pub fn has_voice_sync(&self) -> bool {
        self.segments
            .iter()
            .any(|s| matches!(s, TextSegment::VoiceSync))
    }

    /// Text with control markers removed; `\n` becomes a real newline.
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        for seg in &self.segments {
            match seg {
                TextSegment::Text(t) => out.push_str(t),
                TextSegment::LineBreak => out.push('\n'),
                TextSegment::VoiceSync | TextSegment::WaitClick => {}
            }
        }
        out
    }

    /// Number of visible characters (line breaks count as one).
    pub fn visible_len(&self) -> usize {
        self.segments
            .iter()
            .map(|s| match s {
                TextSegment::Text(t) => t.chars().count(),
                TextSegment::LineBreak => 1,
                _ => 0,
            })
            .sum()
    }

    /// Prefix of the visible text for a typewriter effect.
    pub fn visible_prefix(&self, n: usize) -> String {
        let mut left = n;
        let mut out = String::new();
        for seg in &self.segments {
            if left == 0 {
                break;
            }
            match seg {
                TextSegment::Text(t) => {
                    for ch in t.chars() {
                        if left == 0 {
                            break;
                        }
                        out.push(ch);
                        left -= 1;
                    }
                }
                TextSegment::LineBreak => {
                    out.push('\n');
                    left -= 1;
                }
                _ => {}
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_controls() {
        let t = MessageText::parse("a\\nb\\vc\\ad");
        assert_eq!(
            t.segments,
            vec![
                TextSegment::Text("a".into()),
                TextSegment::LineBreak,
                TextSegment::Text("b".into()),
                TextSegment::VoiceSync,
                TextSegment::Text("c".into()),
                TextSegment::WaitClick,
                TextSegment::Text("d".into()),
            ]
        );
        assert_eq!(t.plain_text(), "a\nbcd");
        assert!(t.has_wait_click());
        assert!(t.has_voice_sync());
        assert_eq!(t.visible_len(), 5);
    }

    #[test]
    fn keeps_unknown_escapes() {
        let t = MessageText::parse("C:\\path\\x");
        assert_eq!(t.plain_text(), "C:\\path\\x");
        assert!(t.segments.iter().all(|s| matches!(s, TextSegment::Text(_))));
    }

    #[test]
    fn typewriter_prefix() {
        let t = MessageText::parse("abc\\ndef");
        assert_eq!(t.visible_prefix(3), "abc");
        assert_eq!(t.visible_prefix(4), "abc\n");
        assert_eq!(t.visible_prefix(100), "abc\ndef");
    }
}
