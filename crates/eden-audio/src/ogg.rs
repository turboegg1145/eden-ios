//! Ogg Vorbis decoding via `lewton`.

use std::io::Cursor;

use lewton::inside_ogg::OggStreamReader;
use lewton::samples::InterleavedSamples;

use crate::{AudioClip, AudioError};

/// Decodes a complete Ogg Vorbis stream from memory.
///
/// `.paz` voice entries have no extension, so callers should use
/// [`crate::decode_audio`] when the container is not known in advance.
pub fn decode_ogg(bytes: &[u8]) -> Result<AudioClip, AudioError> {
    if !bytes.starts_with(b"OggS") {
        return Err(AudioError::Unsupported(
            "missing OggS capture pattern".into(),
        ));
    }
    let mut reader = OggStreamReader::new(Cursor::new(bytes))?;
    let channels = reader.ident_hdr.audio_channels;
    let sample_rate = reader.ident_hdr.audio_sample_rate;
    let mut samples: Vec<f32> = Vec::new();
    while let Some(packet) = reader.read_dec_packet_generic::<InterleavedSamples<f32>>()? {
        samples.extend_from_slice(&packet.samples);
    }
    if samples.is_empty() {
        return Err(AudioError::Unsupported("Ogg stream has no audio".into()));
    }
    AudioClip::new(sample_rate, u16::from(channels), samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_ogg() {
        assert!(matches!(
            decode_ogg(b"RIFF\0\0\0\0WAVE"),
            Err(AudioError::Unsupported(_))
        ));
    }

    #[test]
    fn rejects_truncated_stream() {
        // A valid capture pattern with no pages behind it must not panic.
        let err = decode_ogg(b"OggS\0\0\0\0\0\0\0\0\0\0\0\0").unwrap_err();
        assert!(matches!(err, AudioError::Vorbis(_)));
    }
}
