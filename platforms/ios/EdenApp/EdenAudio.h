//
//  EdenAudio.h
//  Eden
//
//  AVAudioSession + RemoteIO AudioUnit. The render callback pulls interleaved
//  stereo float frames from eden_engine_mix_audio().
//

#import <AVFoundation/AVFoundation.h>
#import <AudioToolbox/AudioToolbox.h>
#import <Foundation/Foundation.h>

#import "eden_ffi.h"

NS_ASSUME_NONNULL_BEGIN

@interface EdenAudio : NSObject

/// Configures AVAudioSession (Playback, 44.1 kHz, 1024-frame buffer) and
/// starts the RemoteIO unit. The engine must outlive the audio object.
- (BOOL)startWithEngine:(EdenEngine *)engine error:(NSError **)error;

/// Stops the unit (waits for the render callback to return) and deactivates
/// the session. Safe to call multiple times.
- (void)stop;

/// Pauses/resumes the unit without tearing down the session.
- (void)setPaused:(BOOL)paused;

@property (nonatomic, readonly, getter=isRunning) BOOL running;

@end

NS_ASSUME_NONNULL_END
