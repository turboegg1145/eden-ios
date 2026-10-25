//
//  EdenGLRenderer.m
//  Eden
//

#import "EdenGLRenderer.h"

#import <OpenGLES/ES3/gl.h>
#import <OpenGLES/ES3/glext.h>
#import <QuartzCore/QuartzCore.h>

static const GLchar *kEdenVertexShaderES3 =
    "#version 300 es\n"
    "const vec2 kPos[3] = vec2[3](vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));\n"
    "const vec2 kUV[3]  = vec2[3](vec2(0.0, 1.0),  vec2(2.0, 1.0),  vec2(0.0, -1.0));\n"
    "out vec2 v_uv;\n"
    "void main() {\n"
    "    gl_Position = vec4(kPos[gl_VertexID], 0.0, 1.0);\n"
    "    v_uv = kUV[gl_VertexID];\n"
    "}\n";

static const GLchar *kEdenFragmentShaderES3 =
    "#version 300 es\n"
    "precision mediump float;\n"
    "in vec2 v_uv;\n"
    "uniform sampler2D u_texture;\n"
    "out vec4 fragColor;\n"
    "void main() {\n"
    "    fragColor = texture(u_texture, v_uv);\n"
    "}\n";

// GLSL ES 1.00 equivalents for devices/drivers where ES 3.0 context creation
// fails. No gl_VertexID and no VAOs: the host feeds a_position/a_uv through
// client-side vertex arrays on every draw.
static const GLchar *kEdenVertexShaderES2 =
    "#version 100\n"
    "attribute vec2 a_position;\n"
    "attribute vec2 a_uv;\n"
    "varying vec2 v_uv;\n"
    "void main() {\n"
    "    gl_Position = vec4(a_position, 0.0, 1.0);\n"
    "    v_uv = a_uv;\n"
    "}\n";

static const GLchar *kEdenFragmentShaderES2 =
    "#version 100\n"
    "precision mediump float;\n"
    "varying vec2 v_uv;\n"
    "uniform sampler2D u_texture;\n"
    "void main() {\n"
    "    gl_FragColor = texture2D(u_texture, v_uv);\n"
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
@property (nonatomic, assign) BOOL legacyES2;
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
    NSError *es3Error = nil;
    EAGLContext *context = [[EAGLContext alloc] initWithAPI:kEAGLRenderingAPIOpenGLES3];
    if (context == nil) {
        es3Error = [NSError errorWithDomain:@"EdenGL"
                                       code:1
                                   userInfo:@{NSLocalizedDescriptionKey: @"OpenGL ES 3.0 is unavailable"}];
        context = [[EAGLContext alloc] initWithAPI:kEAGLRenderingAPIOpenGLES2];
        if (context != nil) {
            self.legacyES2 = YES;
            NSLog(@"Eden: OpenGL ES 3.0 unavailable (%@); using OpenGL ES 2.0",
                  es3Error.localizedDescription);
        }
    }
    if (context == nil) {
        if (error) {
            *error = [NSError errorWithDomain:@"EdenGL"
                                         code:1
                                     userInfo:@{NSLocalizedDescriptionKey:
                                                    [NSString stringWithFormat:
                                                         @"OpenGL ES 3.0 is unavailable (%@); OpenGL ES 2.0 is unavailable",
                                                         es3Error.localizedDescription ?: @"no EAGL context"]}];
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

    if (!self.legacyES2) {
        glGenVertexArrays(1, &_vao);
        glBindVertexArray(_vao);
    }

    glDisable(GL_DEPTH_TEST);
    glDisable(GL_BLEND);
    glPixelStorei(GL_UNPACK_ALIGNMENT, 1);

    self.prepared = YES;
    return YES;
}

- (BOOL)compileShaders:(NSError **)error
{
    const GLchar *vertexSource = self.legacyES2 ? kEdenVertexShaderES2 : kEdenVertexShaderES3;
    const GLchar *fragmentSource = self.legacyES2 ? kEdenFragmentShaderES2 : kEdenFragmentShaderES3;

    GLuint vertex = glCreateShader(GL_VERTEX_SHADER);
    glShaderSource(vertex, 1, &vertexSource, NULL);
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
    glShaderSource(fragment, 1, &fragmentSource, NULL);
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
    if (self.legacyES2) {
        glBindAttribLocation(program, 0, "a_position");
        glBindAttribLocation(program, 1, "a_uv");
    }
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
                 self.legacyES2 ? GL_RGBA : GL_RGBA8,
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
    if (self.legacyES2) {
        // GLSL ES 1.00 has no gl_VertexID/VAO; feed the fullscreen triangle
        // through client-side arrays bound to a_position/a_uv (locations 0/1).
        static const GLfloat positions[6] = {
            -1.0f, -1.0f,  3.0f, -1.0f,  -1.0f, 3.0f,
        };
        static const GLfloat uvs[6] = {
            0.0f, 1.0f,  2.0f, 1.0f,  0.0f, -1.0f,
        };
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 0, positions);
        glEnableVertexAttribArray(1);
        glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, 0, uvs);
    } else {
        glBindVertexArray(self.vao);
    }
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
