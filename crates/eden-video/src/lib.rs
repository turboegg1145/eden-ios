//! `eden-video` — real AVI demuxing and video/audio decoding for `eden*`.
//!
//! * [`avi`] parses the RIFF/AVI container, stream headers, `idx1` and raw
//!   compressed packets without copying the whole movie into memory.
//! * [`mp3`] decodes the MP3 audio track with Symphonia (pure Rust).
//! * [`ffmpeg`] decodes WMV3/VC-1 and MPEG-4 ASP frames through the system
//!   FFmpeg executable on desktop.
//! * [`ios`] forwards frame requests to the Objective-C VideoToolbox /
//!   AVFoundation bridge used by the iOS host.
//!
//! No frame is ever synthesised: a decoder error is propagated to the caller
//! and the runtime reports it instead of pretending the movie played.

pub mod avi;
pub mod ffmpeg;
pub mod mp3;

#[cfg(target_os = "ios")]
pub mod ios;

pub use avi::{
    AviChunk, AviError, AviFile, AudioCodec, AudioFormat, AudioStream, VideoCodec, VideoStream,
};
pub use ffmpeg::FfmpegVideo;

use eden_paz::RgbaImage;

/// Common interface implemented by the desktop FFmpeg bridge and the iOS
/// VideoToolbox bridge.
pub trait VideoDecoder: Send {
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    fn fps(&self) -> f64;
    fn frame_index(&self) -> u64;
    fn next_frame(&mut self) -> avi::Result<Option<RgbaImage>>;
    fn seek(&mut self, time: f64) -> avi::Result<()>;
}

impl VideoDecoder for FfmpegVideo {
    fn width(&self) -> u32 {
        self.width()
    }
    fn height(&self) -> u32 {
        self.height()
    }
    fn fps(&self) -> f64 {
        self.fps()
    }
    fn frame_index(&self) -> u64 {
        self.frame_index()
    }
    fn next_frame(&mut self) -> avi::Result<Option<RgbaImage>> {
        self.next_frame()
    }
    fn seek(&mut self, time: f64) -> avi::Result<()> {
        self.seek(time)
    }
}

/// Opens the best available decoder for `path`.
///
/// On iOS the Objective-C bridge is used. On desktop the FFmpeg executable is
/// required; there is deliberately no silent fallback that would skip frames.
pub fn open_decoder(
    path: impl AsRef<std::path::Path>,
    width: u32,
    height: u32,
    fps: f64,
) -> avi::Result<Box<dyn VideoDecoder>> {
    #[cfg(target_os = "ios")]
    {
        Ok(Box::new(ios::IosVideo::open(path, width, height, fps)?))
    }
    #[cfg(not(target_os = "ios"))]
    {
        Ok(Box::new(FfmpegVideo::open(path, width, height, fps)?))
    }
}
