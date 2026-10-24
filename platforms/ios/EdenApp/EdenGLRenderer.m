//
//  EdenGLRenderer.m
//  Eden
//

#import "EdenGLRenderer.h"

#import <OpenGLES/ES3/gl.h>
#import <OpenGLES/ES3/glext.h>
#import <QuartzCore/QuartzCore.h>

static const GLchar *kEdenVertexShader =
    "#version 300 es\n"
    "const vec2 kPos[3] = vec2[3](vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));\n"
    "const vec2 kUV[3]  = vec2[3](vec2(0.0, 1.0),  vec2(2.0, 1.0),  vec2(0.0, -1.0));\n"
    "out vec2 v_uv;\n"
    "void main() {\n"
    "    gl_Position = vec4(kPos[gl_VertexID], 0.0, 1.0);\n"
    "    v_uv = kUV[gl_VertexID];\n"
    "}\n";

static const GLchar *kEdenFragmentShader =
    "#version 300 es\n"
    "precision mediump float;\n"
    "in vec2 v_uv;\n"
    "uniform sampler2D u_texture;\n"
    "out vec4 fragColor;\n"
    "void main() {\n"
    "    fragColor = texture(u_texture, v_uv);\n"
    "}\n";

@interface EdenGLRenderer ()
@property (nonatomic, strong) EAGLContext *context;
@property (nonatomic, assign) GLuint framebuffer;
@property (nonatomic, assign) GLuint renderbuffer;
@property (nonatomic, assign) GLuint program;
@property (nonatomic, assign) GLuint vao;
@property (nonatomic, assign) GLuint texture;
@property (nonatomic, assign) GLint textureUniform;
@property (nonatomic, assign) GLsizei backingWidth;
@property (nonatomic, assign) GLsizei backingHeight;
@property (nonatomic, assign) BOOL prepared;
@property (nonatomic, assign) BOOL suspended;
@end

@implementation EdenGLRenderer

+ (Class)layerClass
{
    return [CAEAGLLayer class];
}

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

    CAEAGLLayer *layer = (CAEAGLLayer *)self.layer;
    layer.opaque = YES;
    layer.contentsScale = [UIScreen mainScreen].scale;
    layer.drawableProperties = @{
        kEAGLDrawablePropertyRetainedBacking : @NO,
        kEAGLDrawablePropertyColorFormat : kEAGLColorFormatRGBA8,
    };
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
    EAGLContext *context = [[EAGLContext alloc] initWithAPI:kEAGLRenderingAPIOpenGLES3];
    if (context == nil) {
        if (error) {
            *error = [NSError errorWithDomain:@"EdenGL"
                                         code:1
                                     userInfo:@{NSLocalizedDescriptionKey: @"OpenGL ES 3.0 is unavailable"}];
        }
        return NO;
    }
    self.context = context;
    if (![EAGLContext setCurrentContext:context]) {
        if (error) {
            *error = [NSError errorWithDomain:@"EdenGL"
                                         code:2
                                     userInfo:@{NSLocalizedDescriptionKey: @"Cannot make EAGLContext current"}];
        }
        return NO;
    }

    if (![self compileShaders:error]) {
        return NO;
    }
    if (![self createTexture:error]) {
        return NO;
    }
    if (![self createFramebuffer:error]) {
        return NO;
    }

    glGenVertexArrays(1, &_vao);
    glBindVertexArray(_vao);

    glDisable(GL_DEPTH_TEST);
    glDisable(GL_BLEND);
    glPixelStorei(GL_UNPACK_ALIGNMENT, 1);

    self.prepared = YES;
    return YES;
}

- (BOOL)compileShaders:(NSError **)error
{
    GLuint vertex = glCreateShader(GL_VERTEX_SHADER);
    glShaderSource(vertex, 1, &kEdenVertexShader, NULL);
    glCompileShader(vertex);
    GLint ok = GL_FALSE;
    glGetShaderiv(vertex, GL_COMPILE_STATUS, &ok);
    if (ok != GL_TRUE) {
        GLchar log[1024] = {0};
        glGetShaderInfoLog(vertex, sizeof(log), NULL, log);
        glDeleteShader(vertex);
        if (error) {
            *error = [NSError errorWithDomain:@"EdenGL"
                                         code:3
                                     userInfo:@{NSLocalizedDescriptionKey:
                                                    [NSString stringWithFormat:@"vertex shader: %s", log]}];
        }
        return NO;
    }

    GLuint fragment = glCreateShader(GL_FRAGMENT_SHADER);
    glShaderSource(fragment, 1, &kEdenFragmentShader, NULL);
    glCompileShader(fragment);
    glGetShaderiv(fragment, GL_COMPILE_STATUS, &ok);
    if (ok != GL_TRUE) {
        GLchar log[1024] = {0};
        glGetShaderInfoLog(fragment, sizeof(log), NULL, log);
        glDeleteShader(vertex);
        glDeleteShader(fragment);
        if (error) {
            *error = [NSError errorWithDomain:@"EdenGL"
                                         code:4
                                     userInfo:@{NSLocalizedDescriptionKey:
                                                    [NSString stringWithFormat:@"fragment shader: %s", log]}];
        }
        return NO;
    }

    GLuint program = glCreateProgram();
    glAttachShader(program, vertex);
    glAttachShader(program, fragment);
    glLinkProgram(program);
    glDeleteShader(vertex);
    glDeleteShader(fragment);
    glGetProgramiv(program, GL_LINK_STATUS, &ok);
    if (ok != GL_TRUE) {
        GLchar log[1024] = {0};
        glGetProgramInfoLog(program, sizeof(log), NULL, log);
        glDeleteProgram(program);
        if (error) {
            *error = [NSError errorWithDomain:@"EdenGL"
                                         code:5
                                     userInfo:@{NSLocalizedDescriptionKey:
                                                    [NSString stringWithFormat:@"link: %s", log]}];
        }
        return NO;
    }

    self.program = program;
    self.textureUniform = glGetUniformLocation(program, "u_texture");
    return YES;
}

- (BOOL)createTexture:(NSError **)error
{
    glGenTextures(1, &_texture);
    glBindTexture(GL_TEXTURE_2D, _texture);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
    glTexImage2D(GL_TEXTURE_2D,
                 0,
                 GL_RGBA8,
                 (GLsizei)EDEN_STAGE_WIDTH,
                 (GLsizei)EDEN_STAGE_HEIGHT,
                 0,
                 GL_RGBA,
                 GL_UNSIGNED_BYTE,
                 NULL);
    if (glGetError() != GL_NO_ERROR) {
        if (error) {
            *error = [NSError errorWithDomain:@"EdenGL"
                                         code:6
                                     userInfo:@{NSLocalizedDescriptionKey: @"stage texture allocation failed"}];
        }
        return NO;
    }
    return YES;
}

- (BOOL)createFramebuffer:(NSError **)error
{
    if (_framebuffer != 0) {
        glDeleteFramebuffers(1, &_framebuffer);
        _framebuffer = 0;
    }
    if (_renderbuffer != 0) {
        glDeleteRenderbuffers(1, &_renderbuffer);
        _renderbuffer = 0;
    }

    glGenFramebuffers(1, &_framebuffer);
    glBindFramebuffer(GL_FRAMEBUFFER, _framebuffer);

    glGenRenderbuffers(1, &_renderbuffer);
    glBindRenderbuffer(GL_RENDERBUFFER, _renderbuffer);
    if (![self.context renderbufferStorage:GL_RENDERBUFFER fromDrawable:(CAEAGLLayer *)self.layer]) {
        if (error) {
            *error = [NSError errorWithDomain:@"EdenGL"
                                         code:7
                                     userInfo:@{NSLocalizedDescriptionKey: @"renderbufferStorage failed"}];
        }
        return NO;
    }
    glFramebufferRenderbuffer(GL_FRAMEBUFFER,
                              GL_COLOR_ATTACHMENT0,
                              GL_RENDERBUFFER,
                              _renderbuffer);

    GLenum status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
    if (status != GL_FRAMEBUFFER_COMPLETE) {
        if (error) {
            *error = [NSError errorWithDomain:@"EdenGL"
                                         code:8
                                     userInfo:@{NSLocalizedDescriptionKey:
                                                    [NSString stringWithFormat:@"framebuffer status 0x%x", status]}];
        }
        return NO;
    }

    glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_WIDTH, &_backingWidth);
    glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_HEIGHT, &_backingHeight);
    return YES;
}

- (void)layoutSubviews
{
    [super layoutSubviews];
    if (!self.prepared) {
        return;
    }
    [EAGLContext setCurrentContext:self.context];
    [self createFramebuffer:NULL];
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
    [EAGLContext setCurrentContext:self.context];

    glBindTexture(GL_TEXTURE_2D, self.texture);
    glTexSubImage2D(GL_TEXTURE_2D,
                    0,
                    0,
                    0,
                    (GLsizei)width,
                    (GLsizei)height,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    pixels);

    glBindFramebuffer(GL_FRAMEBUFFER, self.framebuffer);
    glViewport(0, 0, self.backingWidth, self.backingHeight);
    glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
    glClear(GL_COLOR_BUFFER_BIT);

    const GLfloat stageAspect = (GLfloat)EDEN_STAGE_WIDTH / (GLfloat)EDEN_STAGE_HEIGHT;
    GLfloat viewportWidth = (GLfloat)self.backingWidth;
    GLfloat viewportHeight = viewportWidth / stageAspect;
    if (viewportHeight > (GLfloat)self.backingHeight) {
        viewportHeight = (GLfloat)self.backingHeight;
        viewportWidth = viewportHeight * stageAspect;
    }
    const GLint vx = (GLint)(((GLfloat)self.backingWidth - viewportWidth) * 0.5f);
    const GLint vy = (GLint)(((GLfloat)self.backingHeight - viewportHeight) * 0.5f);
    glViewport(vx, vy, (GLsizei)viewportWidth, (GLsizei)viewportHeight);

    glUseProgram(self.program);
    glActiveTexture(GL_TEXTURE0);
    glBindTexture(GL_TEXTURE_2D, self.texture);
    glUniform1i(self.textureUniform, 0);
    glBindVertexArray(self.vao);
    glDrawArrays(GL_TRIANGLES, 0, 3);

    glBindRenderbuffer(GL_RENDERBUFFER, self.renderbuffer);
    [self.context presentRenderbuffer:GL_RENDERBUFFER];
}

- (void)setPaused:(BOOL)paused
{
    self.suspended = paused;
}

- (void)dealloc
{
    if (!self.prepared) {
        return;
    }
    [EAGLContext setCurrentContext:self.context];
    if (_texture != 0) {
        glDeleteTextures(1, &_texture);
    }
    if (_vao != 0) {
        glDeleteVertexArrays(1, &_vao);
    }
    if (_program != 0) {
        glDeleteProgram(_program);
    }
    if (_framebuffer != 0) {
        glDeleteFramebuffers(1, &_framebuffer);
    }
    if (_renderbuffer != 0) {
        glDeleteRenderbuffers(1, &_renderbuffer);
    }
    [EAGLContext setCurrentContext:nil];
}

@end
