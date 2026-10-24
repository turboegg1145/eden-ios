//
//  EdenVideo.h
//  Eden
//
//  C bridge between the Rust `eden-video` crate and the iOS decoder.
//  The implementation prefers libavcodec (WMV3/VC-1, MPEG-4 ASP, ...) when
//  FFmpeg headers/libraries are linked, and falls back to AVFoundation's
//  AVAssetReader for the formats VideoToolbox supports.
//

#ifndef EDEN_VIDEO_H
#define EDEN_VIDEO_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/// Opens `path` and prepares RGBA output at `width` x `height`.
/// Returns NULL on failure (see eden_ios_video_last_error()).
void *eden_ios_video_open(const char *path, uint32_t width, uint32_t height);

/// Decodes the next frame into `rgba` (`len` must be width*height*4).
/// Returns 1 on success, 0 at end of stream, -1 on error.
int eden_ios_video_next(void *handle, uint8_t *rgba, size_t len);

/// Seeks to `time` seconds. Returns 0 on success.
int eden_ios_video_seek(void *handle, double time);

/// Releases the decoder. Passing NULL is a no-op.
void eden_ios_video_close(void *handle);

/// Last error message for the current thread; never NULL.
const char *eden_ios_video_last_error(void);

#ifdef __cplusplus
}
#endif

#endif /* EDEN_VIDEO_H */
