//
//  EdenCGRenderer.h
//  Eden
//
//  Level 3 fallback presentation backend. It needs no GPU API at all: the
//  finished 1024x640 RGBA8 frame from the Rust engine is wrapped in a CGImage
//  and assigned to the view's layer.contents. If Metal and OpenGL ES are both
//  unavailable or blocked, the game still appears.
//

#import <UIKit/UIKit.h>

#import "EdenRenderer.h"

NS_ASSUME_NONNULL_BEGIN

@interface EdenCGRenderer : UIView <EdenRenderer>

- (instancetype)initWithFrame:(CGRect)frame NS_DESIGNATED_INITIALIZER;
- (instancetype)initWithCoder:(NSCoder *)coder NS_UNAVAILABLE;

@end

NS_ASSUME_NONNULL_END
