//! iOS video bridge.
//!
//! `platforms/ios/EdenApp/EdenVideo.m` implements these C entry points on top
//! of AVFoundation/VideoToolbox. The Rust side owns timing and scene
//! compositing; the Objective-C side only turns a decrypted AVI into RGBA
//! frames.

use std::ffi::{c_char, c_void, CString};
use std::path::Path;

use eden_paz::RgbaImage;

use crate::avi::{AviError, Result};
use crate::VideoDecoder;

extern "C" {
    fn eden_ios_video_open(
        path: *const c_char,
        width: u32,
        height: u32,
    ) -> *mut c_void;
    fn eden_ios_video_next(handle: *mut c_void, rgba: *mut u8, len: usize) -> i32;
    fn eden_ios_video_seek(handle: *mut c_void, time: f64) -> i32;
    fn eden_ios_video_close(handle: *mut c_void);
    fn eden_ios_video_last_error() -> *const c_char;
}

pub struct IosVideo {
    handle: *mut c_void,
    width: u32,
    height: u32,
    fps: f64,
    frame_index: u64,
    finished: bool,
}

unsafe impl Send for IosVideo {}

impl IosVideo {
    pub fn open(
        path: impl AsRef<Path>,
        width: u32,
        height: u32,
        fps: f64,
    ) -> Result<Self> {
        let path = path.as_ref();
        let c = CString::new(path.to_string_lossy().as_bytes())
            .map_err(|_| AviError::Invalid("video path contains NUL".into()))?;
        let handle = unsafe { eden_ios_video_open(c.as_ptr(), width, height) };
        if handle.is_null() {
            return Err(AviError::Unsupported(ios_error()));
        }
        Ok(Self {
            handle,
            width,
            height,
            fps,
            frame_index: 0,
            finished: false,
        })
    }
}

impl VideoDecoder for IosVideo {
    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn fps(&self) -> f64 {
        self.fps
    }

    fn frame_index(&self) -> u64 {
        self.frame_index
    }

    fn next_frame(&mut self) -> Result<Option<RgbaImage>> {
        if self.finished {
            return Ok(None);
        }
        let mut image = RgbaImage::new(self.width, self.height);
        let rc = unsafe {
            eden_ios_video_next(
                self.handle,
                image.data.as_mut_ptr(),
                image.data.len(),
            )
        };
        match rc {
            1 => {
                self.frame_index += 1;
                Ok(Some(image))
            }
            0 => {
                self.finished = true;
                Ok(None)
            }
            _ => Err(AviError::Unsupported(ios_error())),
        }
    }

    fn seek(&mut self, time: f64) -> Result<()> {
        let rc = unsafe { eden_ios_video_seek(self.handle, time.max(0.0)) };
        if rc != 0 {
            return Err(AviError::Unsupported(ios_error()));
        }
        self.finished = false;
        Ok(())
    }
}

impl Drop for IosVideo {
    fn drop(&mut self) {
        unsafe { eden_ios_video_close(self.handle) };
        self.handle = std::ptr::null_mut();
    }
}

fn ios_error() -> String {
    unsafe {
        let ptr = eden_ios_video_last_error();
        if ptr.is_null() {
            "iOS video decoder error".into()
        } else {
            std::ffi::CStr::from_ptr(ptr)
                .to_string_lossy()
                .into_owned()
        }
    }
}
