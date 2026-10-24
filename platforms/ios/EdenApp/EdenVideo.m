//
//  EdenVideo.m
//  Eden
//
//  Real movie decoding for the iOS host. `eden-video` hands us the decrypted
//  AVI path and asks for RGBA frames at the movie's target size.
//
//  When FFmpeg is linked (`EDEN_HAVE_FFMPEG`) libavformat/libavcodec decode
//  the retail WMV3/VC-1 and MPEG-4 ASP streams. Without FFmpeg the same API
//  is implemented with AVAssetReader, which covers the MPEG-4/H.264 formats
//  VideoToolbox supports. Errors are reported verbatim to Rust; no frame is
//  ever synthesised.
//

#import <Foundation/Foundation.h>
#import <AVFoundation/AVFoundation.h>
#import <CoreVideo/CoreVideo.h>

#import "EdenVideo.h"

#if __has_include(<libavformat/avformat.h>)
#define EDEN_HAVE_FFMPEG 1
#include <libavformat/avformat.h>
#include <libavcodec/avcodec.h>
#include <libswscale/swscale.h>
#else
#define EDEN_HAVE_FFMPEG 0
#endif

static char g_eden_video_error[512] = "no error";

static void EdenVideoSetError(NSString *message)
{
    if (message.length == 0) {
        message = @"unknown video decoder error";
    }
    const char *utf8 = message.UTF8String;
    if (utf8 == NULL) {
        utf8 = "unknown video decoder error";
    }
    strncpy(g_eden_video_error, utf8, sizeof(g_eden_video_error) - 1);
    g_eden_video_error[sizeof(g_eden_video_error) - 1] = '\0';
}

@interface EdenVideoDecoder : NSObject
@property (nonatomic, assign) uint32_t outWidth;
@property (nonatomic, assign) uint32_t outHeight;

// AVFoundation fallback.
@property (nonatomic, strong) AVURLAsset *asset;
@property (nonatomic, strong) AVAssetTrack *track;
@property (nonatomic, strong) AVAssetReader *reader;
@property (nonatomic, strong) AVAssetReaderTrackOutput *output;

#if EDEN_HAVE_FFMPEG
@property (nonatomic, assign) AVFormatContext *format;
@property (nonatomic, assign) AVCodecContext *codec;
@property (nonatomic, assign) struct SwsContext *sws;
@property (nonatomic, assign) AVPacket *packet;
@property (nonatomic, assign) AVFrame *frame;
@property (nonatomic, assign) int streamIndex;
#endif
@end

@implementation EdenVideoDecoder

- (void)dealloc
{
    [self.reader cancelReading];
#if EDEN_HAVE_FFMPEG
    if (_packet != NULL) {
        av_packet_free(&_packet);
    }
    if (_frame != NULL) {
        av_frame_free(&_frame);
    }
    if (_sws != NULL) {
        sws_freeContext(_sws);
        _sws = NULL;
    }
    if (_codec != NULL) {
        avcodec_free_context(&_codec);
    }
    if (_format != NULL) {
        avformat_close_input(&_format);
    }
#endif
}

#if EDEN_HAVE_FFMPEG
- (BOOL)openFFmpegAtPath:(const char *)path width:(uint32_t)width height:(uint32_t)height
{
    AVFormatContext *format = NULL;
    int rc = avformat_open_input(&format, path, NULL, NULL);
    if (rc < 0) {
        EdenVideoSetError([NSString stringWithFormat:@"avformat_open_input: %d", rc]);
        return NO;
    }
    rc = avformat_find_stream_info(format, NULL);
    if (rc < 0) {
        EdenVideoSetError([NSString stringWithFormat:@"avformat_find_stream_info: %d", rc]);
        avformat_close_input(&format);
        return NO;
    }
    const AVCodec *decoder = NULL;
    int stream = av_find_best_stream(format, AVMEDIA_TYPE_VIDEO, -1, -1, &decoder, 0);
    if (stream < 0 || decoder == NULL) {
        EdenVideoSetError(@"no video stream / decoder in AVI");
        avformat_close_input(&format);
        return NO;
    }
    AVCodecContext *codec = avcodec_alloc_context3(decoder);
    if (codec == NULL) {
        EdenVideoSetError(@"avcodec_alloc_context3 failed");
        avformat_close_input(&format);
        return NO;
    }
    rc = avcodec_parameters_to_context(codec, format->streams[stream]->codecpar);
    if (rc < 0) {
        EdenVideoSetError([NSString stringWithFormat:@"avcodec_parameters_to_context: %d", rc]);
        avcodec_free_context(&codec);
        avformat_close_input(&format);
        return NO;
    }
    codec->thread_count = 0;
    rc = avcodec_open2(codec, decoder, NULL);
    if (rc < 0) {
        EdenVideoSetError([NSString stringWithFormat:@"avcodec_open2(%s): %d",
                           decoder->name ?: "?", rc]);
        avcodec_free_context(&codec);
        avformat_close_input(&format);
        return NO;
    }
    struct SwsContext *sws = sws_getContext(codec->width, codec->height, codec->pix_fmt,
                                            (int)width, (int)height, AV_PIX_FMT_RGBA,
                                            SWS_BILINEAR, NULL, NULL, NULL);
    if (sws == NULL) {
        EdenVideoSetError(@"sws_getContext failed");
        avcodec_free_context(&codec);
        avformat_close_input(&format);
        return NO;
    }
    AVPacket *packet = av_packet_alloc();
    AVFrame *frame = av_frame_alloc();
    if (packet == NULL || frame == NULL) {
        EdenVideoSetError(@"av_packet_alloc/av_frame_alloc failed");
        if (packet != NULL) av_packet_free(&packet);
        if (frame != NULL) av_frame_free(&frame);
        sws_freeContext(sws);
        avcodec_free_context(&codec);
        avformat_close_input(&format);
        return NO;
    }
    self.format = format;
    self.codec = codec;
    self.sws = sws;
    self.packet = packet;
    self.frame = frame;
    self.streamIndex = stream;
    NSLog(@"Eden: libavcodec %s %dx%d -> %ux%u", decoder->name,
          codec->width, codec->height, width, height);
    return YES;
}

- (int)nextFFmpegFrame:(uint8_t *)rgba
{
    while (1) {
        int rc = avcodec_receive_frame(self.codec, self.frame);
        if (rc == 0) {
            uint8_t *dst[4] = { rgba, NULL, NULL, NULL };
            int dstStride[4] = { (int)(self.outWidth * 4), 0, 0, 0 };
            sws_scale(self.sws, (const uint8_t *const *)self.frame->data,
                      self.frame->linesize, 0, self.codec->height, dst, dstStride);
            return 1;
        }
        if (rc == AVERROR_EOF) {
            return 0;
        }
        if (rc != AVERROR(EAGAIN)) {
            EdenVideoSetError([NSString stringWithFormat:@"avcodec_receive_frame: %d", rc]);
            return -1;
        }
        rc = av_read_frame(self.format, self.packet);
        if (rc == AVERROR_EOF) {
            avcodec_send_packet(self.codec, NULL);
            continue;
        }
        if (rc < 0) {
            EdenVideoSetError([NSString stringWithFormat:@"av_read_frame: %d", rc]);
            return -1;
        }
        if (self.packet->stream_index == self.streamIndex) {
            rc = avcodec_send_packet(self.codec, self.packet);
            if (rc < 0 && rc != AVERROR(EAGAIN)) {
                EdenVideoSetError([NSString stringWithFormat:@"avcodec_send_packet: %d", rc]);
                av_packet_unref(self.packet);
                return -1;
            }
        }
        av_packet_unref(self.packet);
    }
}
#endif

- (BOOL)restartReaderAt:(double)time
{
    [self.reader cancelReading];
    self.reader = nil;
    self.output = nil;
    NSError *error = nil;
    AVAssetReader *reader = [AVAssetReader assetReaderWithAsset:self.asset error:&error];
    if (reader == nil) {
        EdenVideoSetError(error.localizedDescription);
        return NO;
    }
    if (time > 0.0) {
        reader.timeRange = CMTimeRangeMake(CMTimeMakeWithSeconds(time, 600),
                                           kCMTimePositiveInfinity);
    }
    NSDictionary *settings = @{
        (id)kCVPixelBufferPixelFormatTypeKey : @(kCVPixelFormatType_32BGRA),
    };
    AVAssetReaderTrackOutput *output =
        [AVAssetReaderTrackOutput assetReaderTrackOutputWithTrack:self.track
                                                  outputSettings:settings];
    output.alwaysCopiesSampleData = NO;
    if (![reader canAddOutput:output]) {
        EdenVideoSetError(@"AVAssetReader cannot add video output");
        return NO;
    }
    [reader addOutput:output];
    if (![reader startReading]) {
        EdenVideoSetError(reader.error.localizedDescription ?: @"AVAssetReader startReading failed");
        return NO;
    }
    self.reader = reader;
    self.output = output;
    return YES;
}

- (int)nextAssetReaderFrame:(uint8_t *)rgba
{
    CMSampleBufferRef sample = [self.output copyNextSampleBuffer];
    if (sample == NULL) {
        if (self.reader.status == AVAssetReaderStatusFailed) {
            EdenVideoSetError(self.reader.error.localizedDescription ?: @"AVAssetReader failed");
            return -1;
        }
        return 0;
    }
    CVImageBufferRef buffer = CMSampleBufferGetImageBuffer(sample);
    if (buffer == NULL) {
        CFRelease(sample);
        EdenVideoSetError(@"sample has no CVPixelBuffer");
        return -1;
    }
    if (CVPixelBufferLockBaseAddress(buffer, kCVPixelBufferLock_ReadOnly) != kCVReturnSuccess) {
        CFRelease(sample);
        EdenVideoSetError(@"CVPixelBufferLockBaseAddress failed");
        return -1;
    }
    const uint8_t *base = CVPixelBufferGetBaseAddress(buffer);
    size_t sourceWidth = CVPixelBufferGetWidth(buffer);
    size_t sourceHeight = CVPixelBufferGetHeight(buffer);
    size_t sourceStride = CVPixelBufferGetBytesPerRow(buffer);
    if (base == NULL || sourceWidth == 0 || sourceHeight == 0) {
        CVPixelBufferUnlockBaseAddress(buffer, kCVPixelBufferLock_ReadOnly);
        CFRelease(sample);
        EdenVideoSetError(@"empty CVPixelBuffer");
        return -1;
    }
    for (uint32_t y = 0; y < self.outHeight; y++) {
        size_t sy = (size_t)y * sourceHeight / self.outHeight;
        const uint8_t *row = base + sy * sourceStride;
        uint8_t *dst = rgba + (size_t)y * self.outWidth * 4;
        for (uint32_t x = 0; x < self.outWidth; x++) {
            size_t sx = (size_t)x * sourceWidth / self.outWidth;
            const uint8_t *p = row + sx * 4;
            dst[x * 4 + 0] = p[2];
            dst[x * 4 + 1] = p[1];
            dst[x * 4 + 2] = p[0];
            dst[x * 4 + 3] = 255;
        }
    }
    CVPixelBufferUnlockBaseAddress(buffer, kCVPixelBufferLock_ReadOnly);
    CFRelease(sample);
    return 1;
}

@end

void *eden_ios_video_open(const char *path, uint32_t width, uint32_t height)
{
    if (path == NULL || width == 0 || height == 0) {
        EdenVideoSetError(@"invalid video open parameters");
        return NULL;
    }
    EdenVideoDecoder *decoder = [[EdenVideoDecoder alloc] init];
    decoder.outWidth = width;
    decoder.outHeight = height;

#if EDEN_HAVE_FFMPEG
    if ([decoder openFFmpegAtPath:path width:width height:height]) {
        return (__bridge_retained void *)decoder;
    }
    // Fall through to AVFoundation for formats libavcodec was not built with.
    NSLog(@"Eden: libavcodec failed (%s), trying AVFoundation", g_eden_video_error);
#endif

    NSString *file = [NSString stringWithUTF8String:path];
    NSURL *url = [NSURL fileURLWithPath:file];
    AVURLAsset *asset = [AVURLAsset URLAssetWithURL:url options:nil];
    AVAssetTrack *track = [[asset tracksWithMediaType:AVMediaTypeVideo] firstObject];
    if (track == nil) {
        EdenVideoSetError([NSString stringWithFormat:@"AVFoundation found no video track in %@", file]);
        return NULL;
    }
    decoder.asset = asset;
    decoder.track = track;
    if (![decoder restartReaderAt:0.0]) {
        return NULL;
    }
    return (__bridge_retained void *)decoder;
}

int eden_ios_video_next(void *handle, uint8_t *rgba, size_t len)
{
    if (handle == NULL || rgba == NULL) {
        EdenVideoSetError(@"null decoder or buffer");
        return -1;
    }
    EdenVideoDecoder *decoder = (__bridge EdenVideoDecoder *)handle;
    size_t needed = (size_t)decoder.outWidth * decoder.outHeight * 4;
    if (len < needed) {
        EdenVideoSetError([NSString stringWithFormat:@"RGBA buffer too small (%zu < %zu)", len, needed]);
        return -1;
    }
#if EDEN_HAVE_FFMPEG
    if (decoder.codec != NULL) {
        return [decoder nextFFmpegFrame:rgba];
    }
#endif
    return [decoder nextAssetReaderFrame:rgba];
}

int eden_ios_video_seek(void *handle, double time)
{
    if (handle == NULL) {
        EdenVideoSetError(@"null decoder");
        return -1;
    }
    EdenVideoDecoder *decoder = (__bridge EdenVideoDecoder *)handle;
#if EDEN_HAVE_FFMPEG
    if (decoder.format != NULL) {
        AVStream *stream = decoder.format->streams[decoder.streamIndex];
        int64_t timestamp = (int64_t)(time / av_q2d(stream->time_base));
        int rc = av_seek_frame(decoder.format, decoder.streamIndex, timestamp,
                               AVSEEK_FLAG_BACKWARD);
        if (rc < 0) {
            EdenVideoSetError([NSString stringWithFormat:@"av_seek_frame: %d", rc]);
            return -1;
        }
        avcodec_flush_buffers(decoder.codec);
        return 0;
    }
#endif
    return [decoder restartReaderAt:time] ? 0 : -1;
}

void eden_ios_video_close(void *handle)
{
    if (handle == NULL) {
        return;
    }
    EdenVideoDecoder *decoder = (__bridge_transfer EdenVideoDecoder *)handle;
    [decoder.reader cancelReading];
    decoder = nil;
}

const char *eden_ios_video_last_error(void)
{
    return g_eden_video_error;
}
