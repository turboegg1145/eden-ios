//! `eden-audio` — audio decoding, playback parameters and lip-sync.
//!
//! The Musica engine stores background music, sound effects and voices as
//! Ogg Vorbis streams inside `.paz` archives. Voice entries have no file
//! extension (`sio-C013-0186`), so containers are identified by magic bytes
//! rather than by name. `sys.paz` additionally ships a handful of PCM WAV
//! files used by the system menu.
//!
//! The crate is deliberately free of platform audio APIs: it produces
//! interleaved `f32` samples which the desktop/iOS shells hand to CoreAudio
//! (or any other output backend). [`LipSyncAnalyzer`] publishes the voice RMS
//! envelope and mouth frame through `AtomicU32`, so the audio thread and the
//! render thread never need a lock.

pub mod lipsync;
pub mod ogg;
pub mod player;
pub mod wav;

use std::fmt;

pub use lipsync::{LipSyncAnalyzer, LipSyncState};
pub use ogg::decode_ogg;
pub use player::{AudioPlayer, PlaybackParams};
pub use wav::decode_wav;

/// Errors produced while decoding audio.
#[derive(Debug)]
pub enum AudioError {
    /// Malformed or unsupported Ogg/Vorbis data.
    Vorbis(lewton::VorbisError),
    /// Malformed or unsupported RIFF/WAVE data.
    Wav(&'static str),
    /// The container is neither Ogg nor RIFF/WAVE.
    UnknownContainer,
    /// Structurally valid audio that this crate cannot represent.
    Unsupported(String),
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AudioError::Vorbis(e) => write!(f, "vorbis decode error: {e}"),
            AudioError::Wav(e) => write!(f, "wav decode error: {e}"),
            AudioError::UnknownContainer => {
                write!(f, "unknown audio container (expected OggS or RIFF/WAVE)")
            }
            AudioError::Unsupported(e) => write!(f, "unsupported audio: {e}"),
        }
    }
}

impl std::error::Error for AudioError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AudioError::Vorbis(e) => Some(e),
            _ => None,
        }
    }
}

impl From<lewton::VorbisError> for AudioError {
    fn from(value: lewton::VorbisError) -> Self {
        AudioError::Vorbis(value)
    }
}

/// Decoded PCM audio, interleaved by frame.
///
/// Samples are normalised to `[-1.0, 1.0]`. Only mono and stereo are accepted:
/// every asset shipped with `eden*` / `eden* PLUS+MOSAIC` is one of the two,
/// and the mixer ([`AudioPlayer`]) is defined for those layouts.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioClip {
    pub sample_rate: u32,
    pub channels: u16,
    /// Interleaved samples: `channels` values per frame.
    pub samples: Vec<f32>,
}

impl AudioClip {
    /// Builds a clip after validating the layout.
    pub fn new(sample_rate: u32, channels: u16, samples: Vec<f32>) -> Result<Self, AudioError> {
        if sample_rate == 0 {
            return Err(AudioError::Unsupported("sample rate is zero".into()));
        }
        if !(1..=2).contains(&channels) {
            return Err(AudioError::Unsupported(format!(
                "{channels} channels (only mono/stereo are supported)"
            )));
        }
        if samples.len() % usize::from(channels) != 0 {
            return Err(AudioError::Unsupported(format!(
                "{} samples is not a whole number of {channels}-channel frames",
                samples.len()
            )));
        }
        Ok(Self {
            sample_rate,
            channels,
            samples,
        })
    }

    /// Number of frames (samples per channel).
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels)
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Duration in seconds.
    pub fn duration(&self) -> f64 {
        self.frames() as f64 / f64::from(self.sample_rate)
    }

    /// One interleaved frame, or `None` when out of range.
    pub fn frame(&self, index: usize) -> Option<&[f32]> {
        let ch = usize::from(self.channels);
        let start = index.checked_mul(ch)?;
        self.samples.get(start..start + ch)
    }

    /// Sample of one channel, or `0.0` when out of range.
    pub fn sample(&self, frame: usize, channel: u16) -> f32 {
        match self.frame(frame) {
            Some(f) if usize::from(channel) < f.len() => f[usize::from(channel)],
            _ => 0.0,
        }
    }

    /// Largest absolute sample value.
    pub fn peak(&self) -> f32 {
        self.samples.iter().fold(0.0_f32, |m, s| m.max(s.abs()))
    }

    /// Root-mean-square level of the whole clip.
    pub fn rms(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let sum: f64 = self
            .samples
            .iter()
            .map(|s| f64::from(*s) * f64::from(*s))
            .sum();
        (sum / self.samples.len() as f64).sqrt() as f32
    }
}

/// Decodes an audio container by sniffing its magic bytes.
///
/// * `OggS` → Ogg Vorbis ([`decode_ogg`]).
/// * `RIFF....WAVE` → RIFF/WAVE ([`decode_wav`]).
pub fn decode_audio(bytes: &[u8]) -> Result<AudioClip, AudioError> {
    if bytes.starts_with(b"OggS") {
        decode_ogg(bytes)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        decode_wav(bytes)
    } else {
        Err(AudioError::UnknownContainer)
    }
}

/// Convenience wrapper: reads a `.paz` entry and decodes it.
pub fn decode_paz_entry(
    archive: &eden_paz::PazArchive,
    name: &str,
) -> Result<AudioClip, AudioError> {
    let entry = archive
        .entry(name)
        .ok_or_else(|| AudioError::Unsupported(format!("entry {name:?} not found")))?;
    let bytes = archive
        .read_entry_obj(entry)
        .map_err(|e| AudioError::Unsupported(format!("cannot read {name:?}: {e}")))?;
    decode_audio(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_layout_validation() {
        assert!(AudioClip::new(44100, 0, vec![]).is_err());
        assert!(AudioClip::new(44100, 3, vec![0.0; 3]).is_err());
        assert!(AudioClip::new(44100, 2, vec![0.0; 3]).is_err());
        let clip = AudioClip::new(44100, 2, vec![0.0; 8]).unwrap();
        assert_eq!(clip.frames(), 4);
        assert!((clip.duration() - 4.0 / 44100.0).abs() < 1e-12);
    }

    #[test]
    fn magic_sniffing_rejects_garbage() {
        assert!(matches!(
            decode_audio(b"not audio"),
            Err(AudioError::UnknownContainer)
        ));
        assert!(matches!(
            decode_audio(b"RIFF\0\0\0\0AVI "),
            Err(AudioError::UnknownContainer)
        ));
    }
}
