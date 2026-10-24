//! Pure-Rust MP3 decoding through Symphonia.

use std::io::Cursor;

use eden_audio::{AudioClip, AudioError};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Decode a concatenated MP3 byte stream (as stored in an AVI `01wb` track).
pub fn decode_mp3(bytes: &[u8]) -> Result<AudioClip, AudioError> {
    if bytes.is_empty() {
        return Err(AudioError::Unsupported("empty MP3 stream".into()));
    }
    let mss = MediaSourceStream::new(Box::new(Cursor::new(bytes.to_vec())), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mp3");
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &MetadataOptions::default(),
        )
        .map_err(|e| AudioError::Unsupported(format!("mp3 probe: {e}")))?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| AudioError::Unsupported("mp3 has no decodable track".into()))?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| AudioError::Unsupported(format!("mp3 decoder: {e}")))?;

    let mut sample_buf: Option<SampleBuffer<f32>> = None;
    let mut spec = None;
    let mut samples: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break
            }
            Err(SymphoniaError::ResetRequired) => break,
            Err(e) => return Err(AudioError::Unsupported(format!("mp3 packet: {e}"))),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                if sample_buf.is_none() {
                    let s = *decoded.spec();
                    let capacity = decoded.capacity().max(1) as u64;
                    sample_buf = Some(SampleBuffer::<f32>::new(capacity, s));
                    spec = Some(s);
                }
                if let (Some(buf), Some(s)) = (sample_buf.as_mut(), spec) {
                    buf.copy_interleaved_ref(decoded);
                    samples.extend_from_slice(buf.samples());
                    let _ = s;
                }
            }
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(SymphoniaError::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break
            }
            Err(e) => return Err(AudioError::Unsupported(format!("mp3 decode: {e}"))),
        }
    }
    let Some(spec) = spec else {
        return Err(AudioError::Unsupported("mp3 decoded zero frames".into()));
    };
    let channels = spec.channels.count() as u16;
    // Symphonia may output more than two channels for a malformed stream;
    // downmix to stereo instead of rejecting the whole track.
    let samples = if channels <= 2 {
        samples
    } else {
        let ch = channels as usize;
        let mut out = Vec::with_capacity(samples.len() / ch * 2);
        for frame in samples.chunks_exact(ch) {
            out.push(frame[0]);
            out.push(frame[1]);
        }
        out
    };
    let channels = channels.min(2);
    AudioClip::new(spec.rate, channels, samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_stream_is_rejected() {
        assert!(decode_mp3(&[]).is_err());
    }
}
