//
//  EdenGLRenderer.h
//  Eden
//
//  OpenGL ES backend using CAEAGLLayer. Tries an ES 3.0 context first and
//  falls back to ES 2.0 with GLSL ES 1.00 shaders. Kept as a fallback for
//  devices where Metal is unavailable or disabled by the user.
//

#import <UIKit/UIKit.h>

#import "EdenRenderer.h"

NS_ASSUME_NONNULL_BEGIN

@interface EdenGLRenderer : UIView <EdenRenderer>

- (nullable instancetype)initWithFrame:(CGRect)frame NS_DESIGNATED_INITIALIZER;
- (instancetype)initWithCoder:(NSCoder *)coder NS_UNAVAILABLE;

@end

NS_ASSUME_NONNULL_END
