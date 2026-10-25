//
//  EdenMetalRenderer.m
//  Eden
//

#import "EdenMetalRenderer.h"

#import <QuartzCore/CAMetalLayer.h>

/// Mirrors EdenApp/Shaders/eden.metal. Compiled at runtime when the bundled
/// default.metallib is missing, stale or does not contain the expected
/// functions, so an Xcode/SDK packaging mismatch can never leave the app
/// without a Metal pipeline.
static NSString *const kEdenMetalInlineSource =
    @"#include <metal_stdlib>\n"
    @"using namespace metal;\n"
    @"struct EdenVertexOut {\n"
    @"    float4 position [[position]];\n"
    @"    float2 uv;\n"
    @"};\n"
    @"vertex EdenVertexOut eden_vertex(uint vertexID [[vertex_id]]) {\n"
    @"    const float2 positions[3] = {\n"
    @"        float2(-1.0, -1.0),\n"
    @"        float2( 3.0, -1.0),\n"
    @"        float2(-1.0,  3.0),\n"
    @"    };\n"
    @"    const float2 uvs[3] = {\n"
    @"        float2(0.0, 1.0),\n"
    @"        float2(2.0, 1.0),\n"
    @"        float2(0.0, -1.0),\n"
    @"    };\n"
    @"    EdenVertexOut out;\n"
    @"    out.position = float4(positions[vertexID], 0.0, 1.0);\n"
    @"    out.uv = uvs[vertexID];\n"
    @"    return out;\n"
    @"}\n"
    @"fragment float4 eden_fragment(EdenVertexOut in [[stage_in]],\n"
    @"                              texture2d<float> stage [[texture(0)]],\n"
    @"                              sampler linearSampler [[sampler(0)]]) {\n"
    @"    return stage.sample(linearSampler, in.uv);\n"
    @"}\n";

@interface EdenMetalRenderer ()
@property (nonatomic, strong) id<MTLCommandQueue> queue;
@property (nonatomic, strong) id<MTLRenderPipelineState> pipeline;
@property (nonatomic, strong) id<MTLSamplerState> sampler;
@property (nonatomic, strong) id<MTLTexture> texture;
@property (nonatomic, assign) BOOL prepared;
/// Mirrors EdenGLRenderer/EdenCGRenderer. MTKView's own `paused` must stay YES
/// so its internal timer never runs; this flag is the real pause switch.
@property (nonatomic, assign) BOOL suspended;
@end

@implementation EdenMetalRenderer

- (instancetype)initWithFrame:(CGRect)frame
{
    id<MTLDevice> device = MTLCreateSystemDefaultDevice();
    if (device == nil) {
        return nil;
    }
    self = [super initWithFrame:frame device:device];
    if (self == nil) {
        return nil;
    }
    self.device = device;
    self.delegate = self;
    // Drive every frame from the shared CADisplayLink: MTKView's internal timer
    // stays off, and `suspended` is what actually gates drawing.
    self.paused = YES;
    self.enableSetNeedsDisplay = NO;
    self.suspended = NO;
    self.framebufferOnly = YES;
    self.autoResizeDrawable = YES;
    self.colorPixelFormat = MTLPixelFormatBGRA8Unorm;
    self.clearColor = MTLClearColorMake(0.0, 0.0, 0.0, 1.0);
    self.multipleTouchEnabled = YES;
    self.userInteractionEnabled = YES;
    return self;
}

- (UIView *)view
{
    return self;
}

- (BOOL)prepareWithError:(NSError **)error
{
    if (self.prepared) {
        return YES;
    }
    id<MTLDevice> device = self.device;
    if (device == nil) {
        if (error) {
            *error = [NSError errorWithDomain:@"EdenMetal"
                                         code:1
                                     userInfo:@{NSLocalizedDescriptionKey: @"Metal is unavailable"}];
        }
        return NO;
    }

    self.queue = [device newCommandQueue];
    if (self.queue == nil) {
        if (error) {
            *error = [NSError errorWithDomain:@"EdenMetal"
                                         code:2
                                     userInfo:@{NSLocalizedDescriptionKey: @"Cannot create MTLCommandQueue"}];
        }
        return NO;
    }

    id<MTLLibrary> library = [device newDefaultLibrary];
    id<MTLFunction> vertex = nil;
    id<MTLFunction> fragment = nil;
    NSError *libraryError = nil;

    if (library != nil) {
        vertex = [library newFunctionWithName:@"eden_vertex"];
        fragment = [library newFunctionWithName:@"eden_fragment"];
        if (vertex == nil || fragment == nil) {
            libraryError = [NSError errorWithDomain:@"EdenMetal"
                                               code:3
                                           userInfo:@{NSLocalizedDescriptionKey:
                                                          @"default.metallib is missing eden_vertex/eden_fragment"}];
        }
    } else {
        libraryError = [NSError errorWithDomain:@"EdenMetal"
                                           code:3
                                       userInfo:@{NSLocalizedDescriptionKey: @"default.metallib missing"}];
    }

    if (vertex == nil || fragment == nil) {
        // Level 2: compile the inline copy of eden.metal at runtime. This is
        // independent of how (or whether) Xcode emitted default.metallib.
        NSError *compileError = nil;
        id<MTLLibrary> inlineLibrary = [device newLibraryWithSource:kEdenMetalInlineSource
                                                            options:nil
                                                              error:&compileError];
        if (inlineLibrary != nil) {
            id<MTLFunction> inlineVertex = [inlineLibrary newFunctionWithName:@"eden_vertex"];
            id<MTLFunction> inlineFragment = [inlineLibrary newFunctionWithName:@"eden_fragment"];
            if (inlineVertex != nil && inlineFragment != nil) {
                vertex = inlineVertex;
                fragment = inlineFragment;
                NSLog(@"Eden: Metal default library unusable (%@); compiled inline eden.metal",
                      libraryError.localizedDescription);
            } else {
                libraryError = [NSError errorWithDomain:@"EdenMetal"
                                                   code:4
                                               userInfo:@{NSLocalizedDescriptionKey:
                                                              @"inline eden.metal is missing eden_vertex/eden_fragment"}];
            }
        } else {
            libraryError = compileError ?: [NSError errorWithDomain:@"EdenMetal"
                                                               code:4
                                                           userInfo:@{NSLocalizedDescriptionKey:
                                                                          @"inline eden.metal compilation failed"}];
        }
    }

    if (vertex == nil || fragment == nil) {
        if (error) {
            *error = libraryError;
        }
        return NO;
    }

    MTLRenderPipelineDescriptor *descriptor = [[MTLRenderPipelineDescriptor alloc] init];
    descriptor.label = @"eden.stage";
    descriptor.vertexFunction = vertex;
    descriptor.fragmentFunction = fragment;
    descriptor.colorAttachments[0].pixelFormat = self.colorPixelFormat;
    NSError *pipelineError = nil;
    self.pipeline = [device newRenderPipelineStateWithDescriptor:descriptor error:&pipelineError];
    if (self.pipeline == nil) {
        if (error) {
            *error = pipelineError ?: [NSError errorWithDomain:@"EdenMetal"
                                                          code:5
                                                      userInfo:@{NSLocalizedDescriptionKey: @"pipeline creation failed"}];
        }
        return NO;
    }

    MTLSamplerDescriptor *samplerDescriptor = [[MTLSamplerDescriptor alloc] init];
    samplerDescriptor.minFilter = MTLSamplerMinMagFilterLinear;
    samplerDescriptor.magFilter = MTLSamplerMinMagFilterLinear;
    samplerDescriptor.sAddressMode = MTLSamplerAddressModeClampToEdge;
    samplerDescriptor.tAddressMode = MTLSamplerAddressModeClampToEdge;
    self.sampler = [device newSamplerStateWithDescriptor:samplerDescriptor];

    MTLTextureDescriptor *textureDescriptor =
        [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:MTLPixelFormatRGBA8Unorm
                                                           width:EDEN_STAGE_WIDTH
                                                          height:EDEN_STAGE_HEIGHT
                                                       mipmapped:NO];
    textureDescriptor.usage = MTLTextureUsageShaderRead;
    textureDescriptor.storageMode = MTLStorageModeShared;
    self.texture = [device newTextureWithDescriptor:textureDescriptor];
    if (self.texture == nil) {
        if (error) {
            *error = [NSError errorWithDomain:@"EdenMetal"
                                         code:6
                                     userInfo:@{NSLocalizedDescriptionKey: @"stage texture allocation failed"}];
        }
        return NO;
    }

    self.prepared = YES;
    return YES;
}

- (void)presentPixels:(const uint8_t *)pixels
                width:(uint32_t)width
               height:(uint32_t)height
{
    if (!self.prepared || self.suspended || pixels == NULL) {
        return;
    }
    if (width != EDEN_STAGE_WIDTH || height != EDEN_STAGE_HEIGHT) {
        return;
    }
    [self.texture replaceRegion:MTLRegionMake2D(0, 0, width, height)
                    mipmapLevel:0
                      withBytes:pixels
                    bytesPerRow:(NSUInteger)width * 4];
    // Keep MTKView's own timer off; the frame is drawn explicitly below.
    self.paused = YES;
    [self draw];
}

- (void)drawInMTKView:(MTKView *)view
{
    if (!self.prepared || self.suspended) {
        return;
    }
    if (view.drawableSize.width <= 0.0 || view.drawableSize.height <= 0.0) {
        // The first frame can arrive before the layer has a size; a zero
        // drawable would give a degenerate viewport and no visible output.
        CGFloat scale = view.contentScaleFactor;
        if (!(scale > 0.0)) {
            scale = [UIScreen mainScreen].scale;
        }
        CGSize bounds = view.bounds.size;
        view.drawableSize = CGSizeMake(bounds.width * scale, bounds.height * scale);
    }
    MTLRenderPassDescriptor *pass = view.currentRenderPassDescriptor;
    id<CAMetalDrawable> drawable = view.currentDrawable;
    if (pass == nil || drawable == nil) {
        return;
    }
    pass.colorAttachments[0].loadAction = MTLLoadActionClear;
    pass.colorAttachments[0].clearColor = MTLClearColorMake(0.0, 0.0, 0.0, 1.0);

    id<MTLCommandBuffer> commandBuffer = [self.queue commandBuffer];
    id<MTLRenderCommandEncoder> encoder =
        [commandBuffer renderCommandEncoderWithDescriptor:pass];

    // Aspect-fit the 1024x640 stage into the drawable.
    CGSize size = view.drawableSize;
    const CGFloat stageAspect = (CGFloat)EDEN_STAGE_WIDTH / (CGFloat)EDEN_STAGE_HEIGHT;
    CGFloat viewportWidth = size.width;
    CGFloat viewportHeight = viewportWidth / stageAspect;
    if (viewportHeight > size.height) {
        viewportHeight = size.height;
        viewportWidth = viewportHeight * stageAspect;
    }
    MTLViewport viewport = {
        .originX = (size.width - viewportWidth) * 0.5,
        .originY = (size.height - viewportHeight) * 0.5,
        .width = viewportWidth,
        .height = viewportHeight,
        .znear = 0.0,
        .zfar = 1.0,
    };
    [encoder setViewport:viewport];
    [encoder setRenderPipelineState:self.pipeline];
    [encoder setFragmentTexture:self.texture atIndex:0];
    [encoder setFragmentSamplerState:self.sampler atIndex:0];
    [encoder drawPrimitives:MTLPrimitiveTypeTriangle vertexStart:0 vertexCount:3];
    [encoder endEncoding];
    [commandBuffer presentDrawable:drawable];
    [commandBuffer commit];
}

- (void)mtkView:(MTKView *)view drawableSizeWillChange:(CGSize)size
{
    // The viewport is recomputed every frame from drawableSize.
    (void)view;
    (void)size;
}

@end
