//
//  EdenAudio.m
//  Eden
//

#import "EdenAudio.h"

#include <string.h>

@interface EdenAudio ()
@property (nonatomic, assign) AudioUnit unit;
@property (nonatomic, assign) EdenEngine *engine;
@property (nonatomic, assign) BOOL running;
@end

static OSStatus EdenAudioRenderCallback(void *inRefCon,
                                        AudioUnitRenderActionFlags *ioActionFlags,
                                        const AudioTimeStamp *inTimeStamp,
                                        UInt32 inBusNumber,
                                        UInt32 inNumberFrames,
                                        AudioBufferList *ioData)
{
    (void)ioActionFlags;
    (void)inTimeStamp;
    (void)inBusNumber;

    EdenAudio *audio = (__bridge EdenAudio *)inRefCon;
    if (ioData == NULL || ioData->mNumberBuffers == 0) {
        return noErr;
    }

    AudioBuffer *buffer = &ioData->mBuffers[0];
    float *out = (float *)buffer->mData;
    const UInt32 channels = 2;
    const UInt32 samples = inNumberFrames * channels;
    if (out == NULL || buffer->mDataByteSize < samples * sizeof(float)) {
        return noErr;
    }
    memset(out, 0, samples * sizeof(float));

    EdenEngine *engine = audio.engine;
    if (engine != NULL) {
        eden_engine_mix_audio(engine, out, inNumberFrames);
    }
    return noErr;
}

@implementation EdenAudio

- (BOOL)startWithEngine:(EdenEngine *)engine error:(NSError **)error
{
    if (self.running) {
        self.engine = engine;
        return YES;
    }
    self.engine = engine;

    AVAudioSession *session = [AVAudioSession sharedInstance];
    NSError *sessionError = nil;
    if (![session setCategory:AVAudioSessionCategoryPlayback error:&sessionError] ||
        ![session setPreferredSampleRate:44100.0 error:&sessionError] ||
        ![session setPreferredIOBufferDuration:(1024.0 / 44100.0) error:&sessionError] ||
        ![session setActive:YES error:&sessionError]) {
        if (error) {
            *error = sessionError ?: [NSError errorWithDomain:@"EdenAudio"
                                                         code:1
                                                     userInfo:@{NSLocalizedDescriptionKey: @"AVAudioSession setup failed"}];
        }
        self.engine = NULL;
        return NO;
    }

    AudioComponentDescription description = {
        .componentType = kAudioUnitType_Output,
        .componentSubType = kAudioUnitSubType_RemoteIO,
        .componentManufacturer = kAudioUnitManufacturer_Apple,
        .componentFlags = 0,
        .componentFlagsMask = 0,
    };
    AudioComponent component = AudioComponentFindNext(NULL, &description);
    if (component == NULL) {
        if (error) {
            *error = [NSError errorWithDomain:@"EdenAudio"
                                         code:2
                                     userInfo:@{NSLocalizedDescriptionKey: @"RemoteIO component missing"}];
        }
        self.engine = NULL;
        return NO;
    }

    OSStatus status = AudioComponentInstanceNew(component, &_unit);
    if (status != noErr) {
        if (error) {
            *error = [NSError errorWithDomain:@"EdenAudio"
                                         code:3
                                     userInfo:@{NSLocalizedDescriptionKey:
                                                    [NSString stringWithFormat:@"AudioComponentInstanceNew: %d", (int)status]}];
        }
        self.engine = NULL;
        return NO;
    }

    UInt32 enableOutput = 1;
    AudioUnitSetProperty(self.unit,
                         kAudioOutputUnitProperty_EnableIO,
                         kAudioUnitScope_Output,
                         1,
                         &enableOutput,
                         sizeof(enableOutput));

    AudioStreamBasicDescription format = {0};
    format.mSampleRate = 44100.0;
    format.mFormatID = kAudioFormatLinearPCM;
    format.mFormatFlags = kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked;
    format.mFramesPerPacket = 1;
    format.mChannelsPerFrame = 2;
    format.mBitsPerChannel = 32;
    format.mBytesPerFrame = sizeof(float) * 2;
    format.mBytesPerPacket = sizeof(float) * 2;
    status = AudioUnitSetProperty(self.unit,
                                  kAudioUnitProperty_StreamFormat,
                                  kAudioUnitScope_Input,
                                  0,
                                  &format,
                                  sizeof(format));
    if (status != noErr) {
        if (error) {
            *error = [NSError errorWithDomain:@"EdenAudio"
                                         code:4
                                     userInfo:@{NSLocalizedDescriptionKey:
                                                    [NSString stringWithFormat:@"stream format: %d", (int)status]}];
        }
        [self stop];
        return NO;
    }

    AURenderCallbackStruct callback = {
        .inputProc = EdenAudioRenderCallback,
        .inputProcRefCon = (__bridge void *)self,
    };
    status = AudioUnitSetProperty(self.unit,
                                  kAudioUnitProperty_SetRenderCallback,
                                  kAudioUnitScope_Input,
                                  0,
                                  &callback,
                                  sizeof(callback));
    if (status != noErr) {
        if (error) {
            *error = [NSError errorWithDomain:@"EdenAudio"
                                         code:5
                                     userInfo:@{NSLocalizedDescriptionKey:
                                                    [NSString stringWithFormat:@"render callback: %d", (int)status]}];
        }
        [self stop];
        return NO;
    }

    status = AudioUnitInitialize(self.unit);
    if (status != noErr) {
        if (error) {
            *error = [NSError errorWithDomain:@"EdenAudio"
                                         code:6
                                     userInfo:@{NSLocalizedDescriptionKey:
                                                    [NSString stringWithFormat:@"AudioUnitInitialize: %d", (int)status]}];
        }
        [self stop];
        return NO;
    }

    status = AudioOutputUnitStart(self.unit);
    if (status != noErr) {
        if (error) {
            *error = [NSError errorWithDomain:@"EdenAudio"
                                         code:7
                                     userInfo:@{NSLocalizedDescriptionKey:
                                                    [NSString stringWithFormat:@"AudioOutputUnitStart: %d", (int)status]}];
        }
        [self stop];
        return NO;
    }

    self.running = YES;

    NSNotificationCenter *center = [NSNotificationCenter defaultCenter];
    [center addObserver:self
               selector:@selector(handleInterruption:)
                   name:AVAudioSessionInterruptionNotification
                 object:nil];
    [center addObserver:self
               selector:@selector(handleRouteChange:)
                   name:AVAudioSessionRouteChangeNotification
                 object:nil];
    return YES;
}

- (void)stop
{
    if (self.unit != 0) {
        AudioOutputUnitStop(self.unit);
        AudioUnitUninitialize(self.unit);
        AudioComponentInstanceDispose(self.unit);
        self.unit = 0;
    }
    if (self.running) {
        [[NSNotificationCenter defaultCenter] removeObserver:self];
        [[AVAudioSession sharedInstance] setActive:NO
                                       withOptions:AVAudioSessionSetActiveOptionNotifyOthersOnDeactivation
                                             error:NULL];
    }
    self.running = NO;
    self.engine = NULL;
}

- (void)setPaused:(BOOL)paused
{
    if (self.unit == 0 || !self.running) {
        return;
    }
    if (paused) {
        AudioOutputUnitStop(self.unit);
    } else {
        AudioOutputUnitStart(self.unit);
    }
}

- (void)handleInterruption:(NSNotification *)notification
{
    NSDictionary *userInfo = notification.userInfo;
    AVAudioSessionInterruptionType type =
        (AVAudioSessionInterruptionType)[userInfo[AVAudioSessionInterruptionTypeKey] unsignedIntegerValue];
    if (type == AVAudioSessionInterruptionTypeBegan) {
        [self setPaused:YES];
    } else {
        AVAudioSessionInterruptionOptions options =
            (AVAudioSessionInterruptionOptions)[userInfo[AVAudioSessionInterruptionOptionKey] unsignedIntegerValue];
        if (options & AVAudioSessionInterruptionOptionShouldResume) {
            NSError *error = nil;
            [[AVAudioSession sharedInstance] setActive:YES error:&error];
            if (error == nil) {
                [self setPaused:NO];
            }
        }
    }
}

- (void)handleRouteChange:(NSNotification *)notification
{
    AVAudioSessionRouteChangeReason reason =
        (AVAudioSessionRouteChangeReason)[notification.userInfo[AVAudioSessionRouteChangeReasonKey] unsignedIntegerValue];
    if (reason == AVAudioSessionRouteChangeReasonOldDeviceUnavailable) {
        // Headphones were unplugged: pause instead of blasting the speaker.
        [self setPaused:YES];
    }
}

- (void)dealloc
{
    [self stop];
}

@end
