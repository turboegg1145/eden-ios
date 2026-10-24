//
//  EdenMetalRenderer.m
//  Eden
//

#import "EdenMetalRenderer.h"

#import <QuartzCore/CAMetalLayer.h>

@interface EdenMetalRenderer ()
@property (nonatomic, strong) id<MTLCommandQueue> queue;
@property (nonatomic, strong) id<MTLRenderPipelineState> pipeline;
@property (nonatomic, strong) id<MTLSamplerState> sampler;
@property (nonatomic, strong) id<MTLTexture> texture;
@property (nonatomic, assign) BOOL prepared;
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
    self.paused = YES;                 // driven by the shared CADisplayLink
    self.enableSetNeedsDisplay = NO;
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
    if (library == nil) {
        if (error) {
            *error = [NSError errorWithDomain:@"EdenMetal"
                                         code:3
                                     userInfo:@{NSLocalizedDescriptionKey: @"default.metallib missing"}];
        }
        return NO;
    }

    id<MTLFunction> vertex = [library newFunctionWithName:@"eden_vertex"];
    id<MTLFunction> fragment = [library newFunctionWithName:@"eden_fragment"];
    if (vertex == nil || fragment == nil) {
        if (error) {
            *error = [NSError errorWithDomain:@"EdenMetal"
                                         code:4
                                     userInfo:@{NSLocalizedDescriptionKey: @"eden_vertex/eden_fragment missing"}];
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
    if (!self.prepared || self.paused || pixels == NULL) {
        return;
    }
    if (width != EDEN_STAGE_WIDTH || height != EDEN_STAGE_HEIGHT) {
        return;
    }
    [self.texture replaceRegion:MTLRegionMake2D(0, 0, width, height)
                    mipmapLevel:0
                      withBytes:pixels
                    bytesPerRow:(NSUInteger)width * 4];
    [self draw];
}

- (void)setPaused:(BOOL)paused
{
    // MTKView already exposes `paused`; forward to its setter instead of
    // recursing into this override.
    [super setPaused:paused];
}

- (void)drawInMTKView:(MTKView *)view
{
    if (!self.prepared || self.paused) {
        return;
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
