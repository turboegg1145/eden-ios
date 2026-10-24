//! Real video-frame decoding.
//!
//! Desktop builds use the system FFmpeg binary as a decoder bridge. FFmpeg
//! demuxes the AVI and decodes WMV3/VC-1 and MPEG-4 ASP; this crate reads the
//! resulting RGBA frames from a pipe. Nothing is pre-converted or cached: the
//! pixels handed to the scene come straight from the retail `mov.paz` entry.
//!
//! iOS builds route through [`crate::ios`], which calls the Objective-C
//! VideoToolbox/AVFoundation bridge supplied by `platforms/ios`.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use eden_paz::RgbaImage;

use crate::avi::AviError;

pub type Result<T> = std::result::Result<T, AviError>;

/// Returns true when an FFmpeg executable is usable.
pub fn is_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub struct FfmpegVideo {
    child: Child,
    width: u32,
    height: u32,
    frame_bytes: usize,
    path: PathBuf,
    fps: f64,
    start_time: f64,
    frame_index: u64,
    finished: bool,
}

impl std::fmt::Debug for FfmpegVideo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FfmpegVideo")
            .field("path", &self.path)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("fps", &self.fps)
            .field("frame_index", &self.frame_index)
            .finish()
    }
}

impl FfmpegVideo {
    /// Spawn a decoder for `path`, scaling every frame to `width`×`height`.
    pub fn open(path: impl AsRef<Path>, width: u32, height: u32, fps: f64) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(AviError::Unsupported("video size is zero".into()));
        }
        if !is_available() {
            return Err(AviError::Unsupported(
                "ffmpeg executable not found (desktop decoder bridge)".into(),
            ));
        }
        let path = path.as_ref().to_path_buf();
        let child = spawn(&path, width, height, 0.0)?;
        Ok(Self {
            child,
            width,
            height,
            frame_bytes: width as usize * height as usize * 4,
            path,
            fps,
            start_time: 0.0,
            frame_index: 0,
            finished: false,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn frame_index(&self) -> u64 {
        self.frame_index
    }

    pub fn fps(&self) -> f64 {
        self.fps
    }

    pub fn start_time(&self) -> f64 {
        self.start_time
    }

    /// Decode the next frame. `None` means end of stream.
    pub fn next_frame(&mut self) -> Result<Option<RgbaImage>> {
        if self.finished {
            return Ok(None);
        }
        let mut buf = vec![0u8; self.frame_bytes];
        match read_full(&mut self.child, &mut buf) {
            Ok(true) => {
                self.frame_index += 1;
                let mut image = RgbaImage::new(self.width, self.height);
                image.data.copy_from_slice(&buf);
                Ok(Some(image))
            }
            Ok(false) => {
                self.finished = true;
                Ok(None)
            }
            Err(e) => Err(AviError::Io(e.to_string())),
        }
    }

    /// Restart decoding at `time` seconds (fast input seek).
    pub fn seek(&mut self, time: f64) -> Result<()> {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.child = spawn(&self.path, self.width, self.height, time.max(0.0))?;
        self.start_time = time.max(0.0);
        self.frame_index = 0;
        self.finished = false;
        Ok(())
    }
}

impl Drop for FfmpegVideo {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn(path: &Path, width: u32, height: u32, seek: f64) -> Result<Child> {
    let mut command = Command::new("ffmpeg");
    command
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-nostdin");
    if seek > 0.0 {
        command.arg("-ss").arg(format!("{seek:.6}"));
    }
    command
        .arg("-i")
        .arg(path)
        .arg("-map")
        .arg("0:v:0")
        .arg("-an")
        .arg("-sn")
        .arg("-vf")
        .arg(format!(
            "scale={width}:{height}:flags=bilinear"
        ))
        .arg("-fps_mode")
        .arg("passthrough")
        .arg("-f")
        .arg("rawvideo")
        .arg("-pix_fmt")
        .arg("rgba")
        .arg("pipe:1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command
        .spawn()
        .map_err(|e| AviError::Io(format!("cannot spawn ffmpeg: {e}")))
}

fn read_full(child: &mut Child, buf: &mut [u8]) -> std::io::Result<bool> {
    let stdout = child
        .stdout
        .as_mut()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "ffmpeg stdout"))?;
    let mut filled = 0;
    while filled < buf.len() {
        match stdout.read(&mut buf[filled..]) {
            Ok(0) => {
                if filled == 0 {
                    return Ok(false);
                }
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    format!("short ffmpeg frame ({filled}/{} bytes)", buf.len()),
                ));
            }
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(true)
}

/// Decode one frame at `time` seconds. Used by the screenshot command.
pub fn decode_frame_at(
    path: impl AsRef<Path>,
    time: f64,
    width: u32,
    height: u32,
    fps: f64,
) -> Result<RgbaImage> {
    let mut video = FfmpegVideo::open(path, width, height, fps)?;
    if time > 0.0 {
        video.seek(time)?;
    }
    video
        .next_frame()?
        .ok_or_else(|| AviError::Invalid("ffmpeg returned no frame".into()))
}
