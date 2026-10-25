//
//  EdenCGRenderer.m
//  Eden
//

#import "EdenCGRenderer.h"

#import <QuartzCore/QuartzCore.h>

@interface EdenCGRenderer ()
@property (nonatomic, assign) BOOL suspended;
@end

@implementation EdenCGRenderer

- (instancetype)initWithFrame:(CGRect)frame
{
    self = [super initWithFrame:frame];
    if (self == nil) {
        return nil;
    }
    self.opaque = YES;
    self.backgroundColor = [UIColor blackColor];
    self.multipleTouchEnabled = YES;
    self.userInteractionEnabled = YES;

    CALayer *layer = self.layer;
    layer.backgroundColor = [UIColor blackColor].CGColor;
    layer.contentsGravity = kCAGravityResizeAspect;
    layer.magnificationFilter = kCAFilterLinear;
    layer.minificationFilter = kCAFilterLinear;
    layer.contentsScale = [UIScreen mainScreen].scale;
    layer.opaque = YES;
    return self;
}

- (UIView *)view
{
    return self;
}

- (BOOL)prepareWithError:(NSError **)error
{
    // CoreGraphics/UIKit are always available; there is nothing to allocate.
    (void)error;
    return YES;
}

- (void)presentPixels:(const uint8_t *)pixels
                width:(uint32_t)width
               height:(uint32_t)height
{
    if (self.suspended || pixels == NULL || width == 0 || height == 0) {
        return;
    }
    const size_t bytesPerRow = (size_t)width * 4;
    const size_t length = bytesPerRow * (size_t)height;

    @autoreleasepool {
        // Copy out of the engine-owned framebuffer: it is reused next tick and
        // CoreAnimation may read the image after this method returns.
        NSData *data = [NSData dataWithBytes:pixels length:length];
        CGDataProviderRef provider = CGDataProviderCreateWithCFData((__bridge CFDataRef)data);
        if (provider == NULL) {
            return;
        }
        CGColorSpaceRef colorSpace = CGColorSpaceCreateDeviceRGB();
        // The engine always writes alpha = 255, so skip it instead of
        // premultiplying; this also keeps fully transparent pixels visible.
        CGImageRef image = CGImageCreate((size_t)width,
                                         (size_t)height,
                                         8,
                                         32,
                                         bytesPerRow,
                                         colorSpace,
                                         kCGImageAlphaNoneSkipLast,
                                         provider,
                                         NULL,
                                         false,
                                         kCGRenderingIntentDefault);
        CGColorSpaceRelease(colorSpace);
        CGDataProviderRelease(provider);
        if (image == NULL) {
            return;
        }
        // No implicit contents fade between frames.
        [CATransaction begin];
        [CATransaction setDisableActions:YES];
        self.layer.contents = (__bridge_transfer id)image;
        [CATransaction commit];
    }
}

- (void)setPaused:(BOOL)paused
{
    self.suspended = paused;
}

@end
