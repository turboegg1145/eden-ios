//
//  EdenGLRenderer.h
//  Eden
//
//  OpenGL ES 3.0 backend using CAEAGLLayer. Kept as a fallback for devices
//  where Metal is unavailable or disabled by the user.
//

#import <UIKit/UIKit.h>

#import "EdenRenderer.h"

NS_ASSUME_NONNULL_BEGIN

@interface EdenGLRenderer : UIView <EdenRenderer>

- (nullable instancetype)initWithFrame:(CGRect)frame NS_DESIGNATED_INITIALIZER;
- (instancetype)initWithCoder:(NSCoder *)coder NS_UNAVAILABLE;

@end

NS_ASSUME_NONNULL_END
