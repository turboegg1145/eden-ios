//! RIFF/WAVE decoding.
//!
//! Musica only ships uncompressed WAVs (`sys.paz`: `click.wav`,
//! `SEtest.wav`, `VOICEtest.wav`, `BGMtest.wav`), but the parser accepts all
//! integer widths and IEEE float so that user-supplied replacements work too.

use crate::{AudioClip, AudioError};

const FORMAT_PCM: u16 = 1;
const FORMAT_FLOAT: u16 = 3;
const FORMAT_EXTENSIBLE: u16 = 0xFFFE;

#[derive(Clone, Copy, Debug)]
struct Fmt {
    format: u16,
    channels: u16,
    sample_rate: u32,
    bits: u16,
}

impl Fmt {
    fn bytes_per_sample(self) -> usize {
        usize::from(self.bits).div_ceil(8)
    }

    fn frame_size(self) -> usize {
        usize::from(self.channels) * self.bytes_per_sample()
    }
}

/// Decodes a complete RIFF/WAVE file from memory.
pub fn decode_wav(bytes: &[u8]) -> Result<AudioClip, AudioError> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(AudioError::Wav("missing RIFF/WAVE header"));
    }

    let mut fmt: Option<Fmt> = None;
    let mut data: Option<&[u8]> = None;
    let mut pos = 12usize;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes([
            bytes[pos + 4],
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
        ]) as usize;
        let body_start = pos + 8;
        let body_end = body_start.saturating_add(size).min(bytes.len());
        let body = &bytes[body_start..body_end];
        match id {
            b"fmt " => fmt = Some(parse_fmt(body)?),
            b"data" if data.is_none() => data = Some(body),
            _ => {}
        }
        // Chunks are word-aligned: an odd-sized body is followed by a pad byte.
        pos = body_start.saturating_add(size).saturating_add(size & 1);
    }

    let fmt = fmt.ok_or(AudioError::Wav("missing fmt chunk"))?;
    let data = data.ok_or(AudioError::Wav("missing data chunk"))?;
    decode_pcm(fmt, data)
}

fn parse_fmt(body: &[u8]) -> Result<Fmt, AudioError> {
    if body.len() < 16 {
        return Err(AudioError::Wav("fmt chunk is shorter than 16 bytes"));
    }
    let mut format = u16::from_le_bytes([body[0], body[1]]);
    let channels = u16::from_le_bytes([body[2], body[3]]);
    let sample_rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
    let block_align = u16::from_le_bytes([body[12], body[13]]);
    let bits = u16::from_le_bytes([body[14], body[15]]);

    if format == FORMAT_EXTENSIBLE {
        // WAVEFORMATEXTENSIBLE: the real format code is the first two bytes of
        // the SubFormat GUID at offset 24.
        if body.len() < 40 {
            return Err(AudioError::Wav("extensible fmt chunk is truncated"));
        }
        format = u16::from_le_bytes([body[24], body[25]]);
    }

    if !(1..=2).contains(&channels) {
        return Err(AudioError::Unsupported(format!(
            "{channels} WAV channels (only mono/stereo are supported)"
        )));
    }
    if sample_rate == 0 {
        return Err(AudioError::Wav("sample rate is zero"));
    }
    let fmt = Fmt {
        format,
        channels,
        sample_rate,
        bits,
    };
    match (format, bits) {
        (FORMAT_PCM, 8 | 16 | 24 | 32) => {}
        (FORMAT_FLOAT, 32) => {}
        (FORMAT_FLOAT, _) => {
            return Err(AudioError::Unsupported(format!(
                "{bits}-bit IEEE float WAV"
            )))
        }
        (FORMAT_PCM, _) => return Err(AudioError::Unsupported(format!("{bits}-bit PCM WAV"))),
        (other, _) => {
            return Err(AudioError::Unsupported(format!(
                "WAV format tag {other} (only PCM and IEEE float are supported)"
            )))
        }
    }
    let expected = fmt.frame_size();
    if usize::from(block_align) != expected {
        return Err(AudioError::Wav(
            "block align does not match channel/bits layout",
        ));
    }
    Ok(fmt)
}

fn decode_pcm(fmt: Fmt, data: &[u8]) -> Result<AudioClip, AudioError> {
    let frame_size = fmt.frame_size();
    let bytes_per_sample = fmt.bytes_per_sample();
    let frames = data.len() / frame_size;
    let mut samples = Vec::with_capacity(frames * usize::from(fmt.channels));
    for frame in data.chunks_exact(frame_size) {
        for ch in 0..usize::from(fmt.channels) {
            let b = &frame[ch * bytes_per_sample..(ch + 1) * bytes_per_sample];
            let v = match (fmt.format, fmt.bits) {
                (FORMAT_PCM, 8) => (f32::from(b[0]) - 128.0) / 128.0,
                (FORMAT_PCM, 16) => f32::from(i16::from_le_bytes([b[0], b[1]])) / 32_768.0,
                (FORMAT_PCM, 24) => {
                    let raw = i32::from(b[0]) | (i32::from(b[1]) << 8) | (i32::from(b[2]) << 16);
                    // Sign-extend 24 bits.
                    ((raw << 8) >> 8) as f32 / 8_388_608.0
                }
                (FORMAT_PCM, 32) => {
                    f64::from(i32::from_le_bytes([b[0], b[1], b[2], b[3]])) as f32 / 2_147_483_648.0
                }
                (FORMAT_FLOAT, 32) => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                _ => unreachable!("parse_fmt validated the layout"),
            };
            samples.push(v);
        }
    }
    AudioClip::new(fmt.sample_rate, fmt.channels, samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a canonical little-endian RIFF/WAVE file.
    fn wav(format: u16, channels: u16, rate: u32, bits: u16, data: &[u8]) -> Vec<u8> {
        let block_align = channels * bits.div_ceil(8);
        let byte_rate = rate * u32::from(block_align);
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36u32 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&format.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&byte_rate.to_le_bytes());
        out.extend_from_slice(&block_align.to_le_bytes());
        out.extend_from_slice(&bits.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
        if data.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    #[test]
    fn pcm16_stereo_samples_are_exact() {
        let data = [
            0x00, 0x80, 0xff, 0x7f, // -32768, +32767
            0x00, 0x00, 0x00, 0xc0, // 0, -16384
        ];
        let clip = decode_wav(&wav(FORMAT_PCM, 2, 44_100, 16, &data)).unwrap();
        assert_eq!(clip.sample_rate, 44_100);
        assert_eq!(clip.channels, 2);
        assert_eq!(clip.frames(), 2);
        assert_eq!(clip.sample(0, 0), -1.0);
        assert!((clip.sample(0, 1) - 32_767.0 / 32_768.0).abs() < 1e-6);
        assert_eq!(clip.sample(1, 0), 0.0);
        assert_eq!(clip.sample(1, 1), -0.5);
    }

    #[test]
    fn pcm8_is_unsigned() {
        let data = [0u8, 128, 255];
        let clip = decode_wav(&wav(FORMAT_PCM, 1, 22_050, 8, &data)).unwrap();
        assert_eq!(clip.sample(0, 0), -1.0);
        assert_eq!(clip.sample(1, 0), 0.0);
        assert!((clip.sample(2, 0) - 127.0 / 128.0).abs() < 1e-6);
    }

    #[test]
    fn pcm24_sign_extends() {
        let data = [
            0x00, 0x00, 0x80, // -8388608
            0xff, 0xff, 0x7f, // +8388607
            0x00, 0x00, 0x40, // +4194304
        ];
        let clip = decode_wav(&wav(FORMAT_PCM, 1, 48_000, 24, &data)).unwrap();
        assert_eq!(clip.sample(0, 0), -1.0);
        assert!((clip.sample(1, 0) - 8_388_607.0 / 8_388_608.0).abs() < 1e-7);
        assert_eq!(clip.sample(2, 0), 0.5);
    }

    #[test]
    fn float32_is_passthrough() {
        let data = [0.0f32, 0.25, -0.75]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>();
        let clip = decode_wav(&wav(FORMAT_FLOAT, 1, 48_000, 32, &data)).unwrap();
        assert_eq!(clip.samples, vec![0.0, 0.25, -0.75]);
    }

    #[test]
    fn odd_data_chunk_is_padded() {
        // 3 bytes of 8-bit mono + 1 pad byte, followed by a LIST chunk that
        // must still be skipped correctly.
        let mut file = wav(FORMAT_PCM, 1, 8_000, 8, &[128, 129, 130]);
        file.extend_from_slice(b"LIST");
        file.extend_from_slice(&4u32.to_le_bytes());
        file.extend_from_slice(b"INFO");
        let clip = decode_wav(&file).unwrap();
        assert_eq!(clip.frames(), 3);
    }

    #[test]
    fn rejects_compressed_and_broken_files() {
        let adpcm = wav(2, 1, 44_100, 4, &[0; 4]);
        assert!(matches!(
            decode_wav(&adpcm),
            Err(AudioError::Unsupported(_))
        ));
        assert!(matches!(
            decode_wav(b"RIFF\0\0\0\0WAVEfmt "),
            Err(AudioError::Wav(_))
        ));
        // Block align disagrees with channels * bytes.
        let mut bad = wav(FORMAT_PCM, 2, 44_100, 16, &[0; 4]);
        bad[32] = 2; // block align low byte -> 2 instead of 4
        assert!(matches!(decode_wav(&bad), Err(AudioError::Wav(_))));
    }
}
