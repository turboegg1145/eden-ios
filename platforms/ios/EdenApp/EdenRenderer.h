//
//  EdenRenderer.h
//  Eden
//
//  Common interface for the iOS 14.4-era presentation backends:
//  Metal 2.0 (MTKView), OpenGL ES 3.0/2.0 (CAEAGLLayer) and CoreGraphics
//  (CALayer.contents) as a never-fail last resort.
//

#import <UIKit/UIKit.h>

#import "eden_ffi.h"

NS_ASSUME_NONNULL_BEGIN

@protocol EdenRenderer <NSObject>

/// The view that must be inserted into the view hierarchy.
- (UIView *)view;

/// Allocates GPU resources. Must be called once before the first present.
- (BOOL)prepareWithError:(NSError **)error;

/// Uploads a 1024x640 RGBA8 frame (stride = width * 4) and presents it.
- (void)presentPixels:(const uint8_t *)pixels
                width:(uint32_t)width
               height:(uint32_t)height;

/// Suspends presentation (the render loop keeps ticking the VM).
- (void)setPaused:(BOOL)paused;

@end

/// Creates exactly the requested backend: "metal", "gles" (OpenGL ES 3.0 with
/// an ES 2.0 fallback) or "cg" (CoreGraphics/CALayer). Returns nil and fills
/// `error` when that backend cannot be created. Callers own the fallback chain.
FOUNDATION_EXPORT id<EdenRenderer> EdenCreateRenderer(NSString *_Nullable name,
                                                      CGRect frame,
                                                      NSError **error);

NS_ASSUME_NONNULL_END
