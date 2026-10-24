//
//  EdenMetalRenderer.h
//  Eden
//
//  Metal 2.0 backend. The stage is uploaded into a single 1024x640 RGBA8
//  texture and drawn as a fullscreen triangle, letterboxed to the drawable.
//

#import <MetalKit/MetalKit.h>

#import "EdenRenderer.h"

NS_ASSUME_NONNULL_BEGIN

@interface EdenMetalRenderer : MTKView <MTKViewDelegate, EdenRenderer>

- (nullable instancetype)initWithFrame:(CGRect)frame;
- (instancetype)initWithCoder:(NSCoder *)coder NS_UNAVAILABLE;

@end

NS_ASSUME_NONNULL_END
