//
//  EdenRenderer.m
//  Eden
//

#import "EdenRenderer.h"

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
    } else {
        EdenMetalRenderer *renderer = [[EdenMetalRenderer alloc] initWithFrame:frame];
        if (renderer != nil) {
            return renderer;
        }
    }
    if (error) {
        *error = [NSError errorWithDomain:@"EdenRenderer"
                                     code:1
                                 userInfo:@{NSLocalizedDescriptionKey:
                                                [NSString stringWithFormat:@"renderer '%@' unavailable", backend]}];
    }
    return nil;
}
