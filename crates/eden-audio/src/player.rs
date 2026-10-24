//! Playback parameters and a frame-accurate mixer source.
//!
//! The script VM produces `.playSE file.ogg[50] true -0.5` style commands.
//! [`PlaybackParams`] converts those values into linear gains; [`AudioPlayer`]
//! turns a decoded [`AudioClip`] into stereo frames honouring volume, pan and
//! the loop flag.

use std::sync::Arc;

use crate::AudioClip;

/// Volume / pan / loop state of one playing sound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlaybackParams {
    /// Linear volume, `0.0..=1.0`. `[50]` in a script becomes `0.5`.
    pub volume: f32,
    /// `-1.0` hard left, `0.0` centre, `+1.0` hard right.
    pub pan: f32,
    pub looping: bool,
}

impl Default for PlaybackParams {
    fn default() -> Self {
        Self {
            volume: 1.0,
            pan: 0.0,
            looping: false,
        }
    }
}

impl PlaybackParams {
    pub fn new(volume: f32, pan: f32, looping: bool) -> Self {
        Self {
            volume: volume.clamp(0.0, 1.0),
            pan: pan.clamp(-1.0, 1.0),
            looping,
        }
    }

    /// Builds parameters from a `.playSE` volume given in percent (`[50]`).
    pub fn from_percent(volume_percent: u32, pan: f32, looping: bool) -> Self {
        Self::new(volume_percent as f32 / 100.0, pan, looping)
    }

    /// Builds parameters from the VM's already-normalised volume fraction.
    pub fn from_script(volume: Option<f32>, pan: f32, looping: bool) -> Self {
        Self::new(volume.unwrap_or(1.0), pan, looping)
    }

    /// Left/right gains.
    ///
    /// This is a linear balance law: centre leaves both channels at unity,
    /// hard left silences the right channel and vice versa. A mono clip is
    /// duplicated into both channels before the gains are applied.
    pub fn gains(&self) -> (f32, f32) {
        let pan = self.pan.clamp(-1.0, 1.0);
        let volume = self.volume.clamp(0.0, 1.0);
        let left = if pan > 0.0 { 1.0 - pan } else { 1.0 };
        let right = if pan < 0.0 { 1.0 + pan } else { 1.0 };
        (left * volume, right * volume)
    }
}

/// A cursor over an [`AudioClip`] producing stereo frames.
///
/// The player never allocates while rendering and is `Send`, so it can live on
/// the audio callback thread.
#[derive(Clone, Debug)]
pub struct AudioPlayer {
    clip: Arc<AudioClip>,
    params: PlaybackParams,
    position: usize,
    finished: bool,
}

impl AudioPlayer {
    pub fn new(clip: Arc<AudioClip>, params: PlaybackParams) -> Self {
        let finished = clip.frames() == 0;
        Self {
            clip,
            params,
            position: 0,
            finished,
        }
    }

    pub fn clip(&self) -> &AudioClip {
        &self.clip
    }

    pub fn clip_arc(&self) -> Arc<AudioClip> {
        self.clip.clone()
    }

    pub fn params(&self) -> PlaybackParams {
        self.params
    }

    /// Updates volume/pan/loop without disturbing the play position.
    pub fn set_params(&mut self, params: PlaybackParams) {
        self.params = params;
    }

    /// Frame index that the next [`Self::next_frame`] call will read.
    pub fn position(&self) -> usize {
        self.position
    }

    /// True once a non-looping clip has been fully consumed.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Seeks to a frame. Seeking past the end finishes a non-looping clip.
    pub fn seek(&mut self, frame: usize) {
        let frames = self.clip.frames();
        if frame >= frames {
            self.position = if self.params.looping { 0 } else { frames };
            self.finished = !self.params.looping;
        } else {
            self.position = frame;
            self.finished = false;
        }
    }

    /// Produces the next stereo frame, applying volume and pan.
    pub fn next_frame(&mut self) -> Option<[f32; 2]> {
        if self.finished {
            return None;
        }
        let (left_gain, right_gain) = self.params.gains();
        let frame = self.clip.frame(self.position)?;
        let out = if self.clip.channels == 1 {
            [frame[0] * left_gain, frame[0] * right_gain]
        } else {
            [frame[0] * left_gain, frame[1] * right_gain]
        };

        self.position += 1;
        if self.position >= self.clip.frames() {
            if self.params.looping {
                self.position = 0;
            } else {
                self.finished = true;
            }
        }
        Some(out)
    }

    /// Mixes up to `out.len()` frames into `out` (additively).
    ///
    /// Returns the number of frames written; fewer than `out.len()` means the
    /// source finished.
    pub fn mix_into(&mut self, out: &mut [[f32; 2]]) -> usize {
        let mut written = 0;
        for slot in out.iter_mut() {
            let Some(frame) = self.next_frame() else {
                break;
            };
            slot[0] += frame[0];
            slot[1] += frame[1];
            written += 1;
        }
        written
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mono(samples: Vec<f32>) -> Arc<AudioClip> {
        Arc::new(AudioClip::new(1000, 1, samples).unwrap())
    }

    #[test]
    fn volume_percent_and_pan_gains() {
        let p = PlaybackParams::from_percent(50, 0.0, false);
        assert_eq!(p.volume, 0.5);
        assert_eq!(p.gains(), (0.5, 0.5));
        assert_eq!(
            PlaybackParams::from_percent(100, -1.0, false).gains(),
            (1.0, 0.0)
        );
        assert_eq!(
            PlaybackParams::from_percent(100, 1.0, false).gains(),
            (0.0, 1.0)
        );
        assert_eq!(
            PlaybackParams::from_percent(100, 0.5, false).gains(),
            (0.5, 1.0)
        );
        // Values outside the documented range are clamped, not wrapped.
        assert_eq!(PlaybackParams::new(2.0, -3.0, false).gains(), (1.0, 0.0));
    }

    #[test]
    fn mono_is_duplicated_and_panned() {
        let clip = mono(vec![0.5, -0.5]);
        let mut p = AudioPlayer::new(clip, PlaybackParams::from_percent(100, -0.5, false));
        assert_eq!(p.next_frame(), Some([0.5, 0.25]));
        assert_eq!(p.next_frame(), Some([-0.5, -0.25]));
        assert_eq!(p.next_frame(), None);
        assert!(p.is_finished());
    }

    #[test]
    fn stereo_balance_scales_channels() {
        let clip = Arc::new(AudioClip::new(1000, 2, vec![1.0, 1.0, 0.5, -0.5]).unwrap());
        let mut p = AudioPlayer::new(clip, PlaybackParams::new(0.5, 1.0, false));
        assert_eq!(p.next_frame(), Some([0.0, 0.5]));
        assert_eq!(p.next_frame(), Some([0.0, -0.25]));
    }

    #[test]
    fn looping_wraps_and_never_finishes() {
        let clip = mono(vec![1.0, 2.0, 3.0]);
        let mut p = AudioPlayer::new(clip, PlaybackParams::new(1.0, 0.0, true));
        let got: Vec<_> = (0..7)
            .filter_map(|_| p.next_frame())
            .map(|f| f[0])
            .collect();
        assert_eq!(got, vec![1.0, 2.0, 3.0, 1.0, 2.0, 3.0, 1.0]);
        assert!(!p.is_finished());
        assert_eq!(p.position(), 1);
    }

    #[test]
    fn mix_into_is_additive_and_stops_at_end() {
        let clip = mono(vec![0.25, 0.25, 0.25]);
        let mut p = AudioPlayer::new(clip, PlaybackParams::default());
        let mut out = [[0.5, 0.5], [0.0, 0.0], [0.0, 0.0], [0.0, 0.0]];
        assert_eq!(p.mix_into(&mut out), 3);
        assert_eq!(out[0], [0.75, 0.75]);
        assert_eq!(out[1], [0.25, 0.25]);
        assert_eq!(out[3], [0.0, 0.0]);
    }

    #[test]
    fn empty_clip_finishes_immediately() {
        let clip = Arc::new(AudioClip::new(1000, 1, vec![]).unwrap());
        let mut p = AudioPlayer::new(clip, PlaybackParams::default());
        assert!(p.is_finished());
        assert_eq!(p.next_frame(), None);
    }

    #[test]
    fn seek_and_loop_interaction() {
        let clip = mono(vec![1.0, 2.0, 3.0, 4.0]);
        let mut p = AudioPlayer::new(clip, PlaybackParams::new(1.0, 0.0, true));
        p.seek(2);
        assert_eq!(p.next_frame(), Some([3.0, 3.0]));
        p.seek(99);
        assert_eq!(p.position(), 0);
        assert_eq!(p.next_frame(), Some([1.0, 1.0]));
    }
}
