//! Script text decoding.
//!
//! The original engine stores scripts as raw 8-bit text. `eden_en` was
//! localised into Simplified Chinese and uses GBK; the PLUS+MOSAIC scripts use
//! CP932 (not plain ASCII — the English text contains CP932 curly quotes).
//!
//! Decoding **before** tokenising is essential: GBK characters such as `乗`
//! (`81 5C`) end in `0x5C`, which a byte-oriented parser would mistake for the
//! escape character `\`.

use crate::error::{ParseError, ScriptError};
use encoding_rs::{Encoding, GBK, SHIFT_JIS};

/// Which encoding a script archive uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptEncoding {
    /// `eden_en` — GBK/GB2312.
    Gbk,
    /// `eden_PlusMosaic_en` — CP932 / Shift_JIS.
    Cp932,
    /// UTF-8 (used by extracted/modern tooling and tests).
    Utf8,
}

impl ScriptEncoding {
    pub fn for_archive_id(id: &str) -> Option<Self> {
        match id {
            "eden_en" => Some(ScriptEncoding::Gbk),
            "eden_PlusMosaic_en" => Some(ScriptEncoding::Cp932),
            _ => None,
        }
    }

    pub fn encoding(self) -> Option<&'static Encoding> {
        match self {
            ScriptEncoding::Gbk => Some(GBK),
            ScriptEncoding::Cp932 => Some(SHIFT_JIS),
            ScriptEncoding::Utf8 => None,
        }
    }
}

/// Decode a script, rejecting malformed input instead of silently inserting
/// U+FFFD. A UTF-8 BOM is accepted and stripped.
pub fn decode_script(bytes: &[u8], encoding: ScriptEncoding) -> Result<String, ScriptError> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let text = match encoding {
        ScriptEncoding::Utf8 => std::str::from_utf8(bytes)
            .map_err(|e| ScriptError::InvalidSave(format!("script is not UTF-8: {e}")))?
            .to_string(),
        ScriptEncoding::Gbk => {
            let (text, _, had_errors) = GBK.decode(bytes);
            if had_errors {
                return Err(ScriptError::InvalidSave(
                    "script contains bytes that are not valid GBK".into(),
                ));
            }
            text.into_owned()
        }
        ScriptEncoding::Cp932 => {
            let (text, _, had_errors) = SHIFT_JIS.decode(bytes);
            if had_errors {
                return Err(ScriptError::InvalidSave(
                    "script contains bytes that are not valid CP932".into(),
                ));
            }
            text.into_owned()
        }
    };
    Ok(text)
}

/// Convenience wrapper that turns a decoding failure into a line-1 parse error.
pub fn decode_script_or_error(
    bytes: &[u8],
    encoding: ScriptEncoding,
) -> Result<String, Vec<ParseError>> {
    decode_script(bytes, encoding).map_err(|e| {
        vec![ParseError {
            line: 1,
            source: String::new(),
            message: e.to_string(),
        }]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gbk_trailing_5c_is_decoded_as_one_character() {
        // U+4E57 "乗" is 81 5C in GBK. The second byte must not be treated as
        // a backslash by the tokenizer.
        let bytes = b".message 1 #\x81\x5c\r\n.wait 1\r\n";
        let text = decode_script(bytes, ScriptEncoding::Gbk).unwrap();
        assert_eq!(text, ".message 1 #乗\r\n.wait 1\r\n");
    }

    #[test]
    fn cp932_curly_quote_decodes() {
        let (bytes, _, _) = SHIFT_JIS.encode(".message 1 #I mustn’t stop.");
        let text = decode_script(&bytes, ScriptEncoding::Cp932).unwrap();
        assert!(text.contains('’'));
    }

    #[test]
    fn utf8_bom_is_stripped() {
        let bytes = b"\xef\xbb\xbf.message 1 #hello";
        assert_eq!(
            decode_script(bytes, ScriptEncoding::Utf8).unwrap(),
            ".message 1 #hello"
        );
    }

    #[test]
    fn invalid_gbk_is_rejected() {
        // 0x81 alone is an incomplete GBK lead byte.
        assert!(decode_script(b".message 1 #\x81", ScriptEncoding::Gbk).is_err());
    }
}
