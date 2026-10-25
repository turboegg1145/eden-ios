//
//  EdenRenderer.m
//  Eden
//

#import "EdenRenderer.h"

#import "EdenCGRenderer.h"
#import "EdenGLRenderer.h"
#import "EdenMetalRenderer.h"

id<EdenRenderer> EdenCreateRenderer(NSString *name, CGRect frame, NSError **error)
{
    NSString *backend = name.lowercaseString ?: @"metal";

    if ([backend isEqualToString:@"gles"] || [backend isEqualToString:@"gl"]) {
        EdenGLRenderer *renderer = [[EdenGLRenderer alloc] initWithFrame:frame];
        if (renderer != nil) {
            return renderer;
        }
        if (error) {
            *error = [NSError errorWithDomain:@"EdenRenderer"
                                         code:1
                                     userInfo:@{NSLocalizedDescriptionKey:
                                                    @"OpenGL ES renderer unavailable"}];
        }
        return nil;
    }

    if ([backend isEqualToString:@"cg"] || [backend isEqualToString:@"coregraphics"]) {
        EdenCGRenderer *renderer = [[EdenCGRenderer alloc] initWithFrame:frame];
        if (renderer != nil) {
            return renderer;
        }
        if (error) {
            *error = [NSError errorWithDomain:@"EdenRenderer"
                                         code:2
                                     userInfo:@{NSLocalizedDescriptionKey:
                                                    @"CoreGraphics renderer unavailable"}];
        }
        return nil;
    }

    if ([backend isEqualToString:@"metal"]) {
        EdenMetalRenderer *renderer = [[EdenMetalRenderer alloc] initWithFrame:frame];
        if (renderer != nil) {
            return renderer;
        }
        if (error) {
            *error = [NSError errorWithDomain:@"EdenRenderer"
                                         code:3
                                     userInfo:@{NSLocalizedDescriptionKey:
                                                    @"Metal is unavailable (MTLCreateSystemDefaultDevice returned nil)"}];
        }
        return nil;
    }

    if (error) {
        *error = [NSError errorWithDomain:@"EdenRenderer"
                                     code:4
                                 userInfo:@{NSLocalizedDescriptionKey:
                                                [NSString stringWithFormat:@"unknown renderer '%@'", backend]}];
    }
    return nil;
}
