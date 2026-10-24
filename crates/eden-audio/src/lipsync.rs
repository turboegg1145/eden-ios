//! Real-time voice RMS envelope → `.ani` mouth frame index.
//!
//! The audio thread calls [`LipSyncAnalyzer::process`] with each decoded block;
//! the render thread calls [`LipSyncAnalyzer::frame`] to pick the mouth frame.
//! Both values live in `AtomicU32` (an `f32` bit pattern for the envelope), so
//! there is no mutex and no allocation on either side.

use std::sync::atomic::{AtomicU32, Ordering};

/// Envelope below this level is treated as silence (mouth closed).
pub const DEFAULT_GATE: f32 = 0.01;
/// Envelope at or above this level selects the fully open mouth frame.
pub const DEFAULT_FULL_SCALE: f32 = 0.20;
/// Smoothing coefficient when the level rises (0..1, higher = faster).
pub const DEFAULT_ATTACK: f32 = 0.60;
/// Smoothing coefficient when the level falls (0..1, higher = faster).
pub const DEFAULT_RELEASE: f32 = 0.12;

/// Consistent snapshot of the published envelope and frame index.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LipSyncState {
    pub envelope: f32,
    pub frame: u32,
}

/// Lock-free RMS envelope follower.
#[derive(Debug)]
pub struct LipSyncAnalyzer {
    frame_count: u32,
    gate: f32,
    full_scale: f32,
    attack: f32,
    release: f32,
    envelope: AtomicU32,
    frame: AtomicU32,
}

impl LipSyncAnalyzer {
    /// Creates an analyzer for a mouth animation with `frame_count` frames.
    pub fn new(frame_count: u32) -> Self {
        Self::with_response(
            frame_count,
            DEFAULT_GATE,
            DEFAULT_FULL_SCALE,
            DEFAULT_ATTACK,
            DEFAULT_RELEASE,
        )
    }

    /// Creates an analyzer with custom gate/scale/smoothing.
    ///
    /// `attack` and `release` are clamped to `0.0..=1.0`; `full_scale` is kept
    /// strictly above `gate` so the normalisation cannot divide by zero.
    pub fn with_response(
        frame_count: u32,
        gate: f32,
        full_scale: f32,
        attack: f32,
        release: f32,
    ) -> Self {
        let gate = gate.max(0.0);
        Self {
            frame_count: frame_count.max(1),
            gate,
            full_scale: full_scale.max(gate + 1e-6),
            attack: attack.clamp(0.0, 1.0),
            release: release.clamp(0.0, 1.0),
            envelope: AtomicU32::new(0.0_f32.to_bits()),
            frame: AtomicU32::new(0),
        }
    }

    pub fn frame_count(&self) -> u32 {
        self.frame_count
    }

    /// Current smoothed RMS envelope.
    pub fn envelope(&self) -> f32 {
        f32::from_bits(self.envelope.load(Ordering::Relaxed))
    }

    /// Current mouth frame index, in `0..frame_count`.
    pub fn frame(&self) -> u32 {
        self.frame.load(Ordering::Relaxed)
    }

    pub fn state(&self) -> LipSyncState {
        LipSyncState {
            envelope: self.envelope(),
            frame: self.frame(),
        }
    }

    /// Clears the envelope and returns the mouth to frame 0.
    pub fn reset(&self) {
        self.envelope.store(0.0_f32.to_bits(), Ordering::Relaxed);
        self.frame.store(0, Ordering::Relaxed);
    }

    /// Feeds one block of samples and returns the new mouth frame.
    ///
    /// Empty blocks are ignored (they carry no level information) and return
    /// the current frame.
    pub fn process(&self, samples: &[f32]) -> u32 {
        if samples.is_empty() {
            return self.frame();
        }
        let mean_square: f64 = samples
            .iter()
            .map(|s| f64::from(*s) * f64::from(*s))
            .sum::<f64>()
            / samples.len() as f64;
        let rms = mean_square.sqrt() as f32;

        let previous = self.envelope();
        let coefficient = if rms > previous {
            self.attack
        } else {
            self.release
        };
        let envelope = previous + coefficient * (rms - previous);
        self.envelope.store(envelope.to_bits(), Ordering::Relaxed);

        let frame = self.frame_for(envelope);
        self.frame.store(frame, Ordering::Relaxed);
        frame
    }

    /// Maps an envelope value to a frame index without touching the atomics.
    pub fn frame_for(&self, envelope: f32) -> u32 {
        if self.frame_count <= 1 || envelope <= self.gate {
            return 0;
        }
        let normalized = ((envelope - self.gate) / (self.full_scale - self.gate)).clamp(0.0, 1.0);
        (normalized * (self.frame_count - 1) as f32).round() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn silence_stays_closed() {
        let a = LipSyncAnalyzer::new(4);
        assert_eq!(a.process(&[0.0; 128]), 0);
        assert_eq!(a.frame(), 0);
        assert_eq!(a.envelope(), 0.0);
    }

    #[test]
    fn loud_block_opens_then_decays() {
        let a = LipSyncAnalyzer::with_response(4, 0.01, 0.2, 1.0, 1.0);
        assert_eq!(a.process(&[0.5; 64]), 3);
        assert!((a.envelope() - 0.5).abs() < 1e-6);
        assert_eq!(a.process(&[0.0; 64]), 0);
        assert_eq!(a.envelope(), 0.0);
    }

    #[test]
    fn attack_is_faster_than_release() {
        let a = LipSyncAnalyzer::new(3);
        for _ in 0..10 {
            a.process(&[0.1; 64]);
        }
        let rising = a.envelope();
        assert!(rising > 0.05, "envelope did not rise: {rising}");
        for _ in 0..10 {
            a.process(&[0.0; 64]);
        }
        let falling = a.envelope();
        assert!(falling > 0.01, "release should be gradual: {falling}");
        assert!(falling < rising);
    }

    #[test]
    fn frame_mapping_covers_the_whole_range() {
        let a = LipSyncAnalyzer::new(5);
        assert_eq!(a.frame_for(0.0), 0);
        assert_eq!(a.frame_for(0.005), 0);
        assert_eq!(a.frame_for(0.2), 4);
        assert_eq!(a.frame_for(0.6), 4);
        assert_eq!(a.frame_for(0.105), 2);
        assert_eq!(LipSyncAnalyzer::new(1).frame_for(1.0), 0);
    }

    #[test]
    fn empty_block_keeps_state() {
        let a = LipSyncAnalyzer::new(3);
        a.process(&[0.5; 32]);
        let before = a.state();
        assert_eq!(a.process(&[]), before.frame);
        assert_eq!(a.state(), before);
    }

    #[test]
    fn concurrent_reader_never_sees_out_of_range() {
        let a = Arc::new(LipSyncAnalyzer::new(4));
        let producer = {
            let a = Arc::clone(&a);
            thread::spawn(move || {
                for i in 0..500 {
                    let level = if i % 2 == 0 { 0.4 } else { 0.0 };
                    a.process(&[level; 64]);
                }
            })
        };
        for _ in 0..2000 {
            assert!(a.frame() < 4);
            let s = a.state();
            assert!(s.frame < 4 && s.envelope.is_finite());
        }
        producer.join().unwrap();
    }
}
