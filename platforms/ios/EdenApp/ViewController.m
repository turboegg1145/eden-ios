//
//  ViewController.m
//  Eden
//
//  Hosts the pure-Rust Musica runtime (libeden_ffi.a) on iOS 12–14.4.
//  The VM is ticked from a CADisplayLink at 60 Hz; the selected backend only
//  receives the finished 1024x640 RGBA8 framebuffer.
//

#import "ViewController.h"

#import <QuartzCore/QuartzCore.h>

#import "EdenAudio.h"
#import "EdenRenderer.h"
#import "eden_ffi.h"

/// CADisplayLink retains its target. A weak proxy breaks the
/// ViewController <-> CADisplayLink retain cycle so -dealloc can stop the
/// audio unit and destroy the engine.
@interface EdenDisplayLinkProxy : NSObject
@property (nonatomic, weak) id target;
@property (nonatomic, assign) SEL selector;
@end

@implementation EdenDisplayLinkProxy

- (void)tick:(CADisplayLink *)link
{
    id target = self.target;
    if (target == nil) {
        return;
    }
    IMP implementation = [target methodForSelector:self.selector];
    void (*function)(id, SEL, CADisplayLink *) = (void *)implementation;
    function(target, self.selector, link);
}

@end

@interface ViewController ()

@property (nonatomic, assign) EdenEngine *engine;
@property (nonatomic, strong) id<EdenRenderer> renderer;
@property (nonatomic, strong) EdenAudio *audio;
@property (nonatomic, strong) CADisplayLink *displayLink;
@property (nonatomic, strong) EdenDisplayLinkProxy *displayLinkProxy;
@property (nonatomic, strong) UILabel *messageLabel;
@property (nonatomic, assign) BOOL engineReady;
@property (nonatomic, assign) BOOL backgrounded;
@property (nonatomic, assign) BOOL reportedRuntimeError;

@end

@implementation ViewController

- (void)dealloc
{
    [[NSNotificationCenter defaultCenter] removeObserver:self];
    [self.displayLink invalidate];
    self.displayLink = nil;
    [self.audio stop];
    if (self.engine != NULL) {
        eden_engine_destroy(self.engine);
        self.engine = NULL;
    }
}

- (void)loadView
{
    UIView *view = [[UIView alloc] initWithFrame:[UIScreen mainScreen].bounds];
    view.backgroundColor = [UIColor blackColor];
    view.autoresizingMask = UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight;
    self.view = view;
}

- (void)viewDidLoad
{
    [super viewDidLoad];

    self.audio = [[EdenAudio alloc] init];

    NSNotificationCenter *center = [NSNotificationCenter defaultCenter];
    [center addObserver:self
               selector:@selector(applicationDidEnterBackground:)
                   name:UIApplicationDidEnterBackgroundNotification
                 object:nil];
    [center addObserver:self
               selector:@selector(applicationWillEnterForeground:)
                   name:UIApplicationWillEnterForegroundNotification
                 object:nil];

    NSString *gameDirectory = [self resolveGameDirectory];
    if (gameDirectory == nil) {
        [self showMessage:
                  @"未找到游戏数据。\n"
                  @"请用 Finder / 文件 App / Filza 将原版 eden* 的\n"
                  @"scr.paz、bg.paz、sys.paz、st.paz、bgm.paz、se.paz、voice.paz\n"
                  @"复制到“我的 iPhone/Eden/eden/”后重新启动。\n\n"
                  @"Game data not found. Copy the retail .paz files to\n"
                  @"Documents/eden/ via iTunes file sharing, Files or Filza."];
        return;
    }
    NSLog(@"Eden: game directory = %@", gameDirectory);

    if (![self createRenderer]) {
        return;
    }

    self.engine = eden_engine_create();
    if (self.engine == NULL) {
        [self showMessage:@"eden_engine_create() failed (out of memory)."];
        return;
    }

    NSString *exePath = nil;
    for (NSString *name in @[ @"eden.exe", @"eden_pm_en.exe" ]) {
        NSString *candidate = [gameDirectory stringByAppendingPathComponent:name];
        if ([[NSFileManager defaultManager] fileExistsAtPath:candidate]) {
            exePath = candidate;
            break;
        }
    }
    if (eden_engine_open(self.engine,
                         gameDirectory.fileSystemRepresentation,
                         exePath.fileSystemRepresentation) != 0) {
        [self showMessage:[NSString stringWithFormat:@"无法打开游戏数据：\n%s",
                           eden_engine_last_error(self.engine)]];
        [self destroyEngine];
        return;
    }

    NSString *fontPath = [self bundledFontPath];
    if (fontPath != nil) {
        if (eden_engine_load_font(self.engine, fontPath.fileSystemRepresentation) != 0) {
            NSLog(@"Eden: font load failed: %s", eden_engine_last_error(self.engine));
        }
    }

    BOOL autoAdvance = [[NSUserDefaults standardUserDefaults] objectForKey:@"EdenAutoAdvance"] == nil
        ? YES
        : [[NSUserDefaults standardUserDefaults] boolForKey:@"EdenAutoAdvance"];
    eden_engine_set_auto_advance(self.engine, autoAdvance ? 1 : 0);

    if (eden_engine_start_script(self.engine, "A001.sc") != 0) {
        [self showMessage:[NSString stringWithFormat:@"无法启动 A001.sc：\n%s",
                           eden_engine_last_error(self.engine)]];
        [self destroyEngine];
        return;
    }

    NSError *audioError = nil;
    if (![self.audio startWithEngine:self.engine error:&audioError]) {
        NSLog(@"Eden: audio unavailable: %@", audioError.localizedDescription);
    }

    self.engineReady = YES;
    self.reportedRuntimeError = NO;
    [self startDisplayLink];
}

- (void)viewDidAppear:(BOOL)animated
{
    [super viewDidAppear:animated];
    if ([self respondsToSelector:@selector(setNeedsUpdateOfHomeIndicatorAutoHidden)]) {
        [self setNeedsUpdateOfHomeIndicatorAutoHidden];
    }
}

#pragma mark - Renderer

- (BOOL)createRenderer
{
    NSString *preferred = [[NSUserDefaults standardUserDefaults] stringForKey:@"EdenBackend"] ?: @"metal";
    NSError *error = nil;
    id<EdenRenderer> renderer = EdenCreateRenderer(preferred, self.view.bounds, &error);
    if (renderer != nil) {
        UIView *view = renderer.view;
        view.frame = self.view.bounds;
        view.autoresizingMask = UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight;
        [self.view addSubview:view];
        if ([renderer prepareWithError:&error]) {
            self.renderer = renderer;
            NSLog(@"Eden: renderer = %@", NSStringFromClass([renderer class]));
            return YES;
        }
        NSLog(@"Eden: %@ prepare failed: %@", NSStringFromClass([renderer class]), error.localizedDescription);
        [view removeFromSuperview];
    }

    // Fall back to the other backend.
    NSString *fallback = [preferred.lowercaseString isEqualToString:@"gles"] ? @"metal" : @"gles";
    renderer = EdenCreateRenderer(fallback, self.view.bounds, &error);
    if (renderer != nil) {
        UIView *view = renderer.view;
        view.frame = self.view.bounds;
        view.autoresizingMask = UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight;
        [self.view addSubview:view];
        if ([renderer prepareWithError:&error]) {
            self.renderer = renderer;
            NSLog(@"Eden: renderer fallback = %@", NSStringFromClass([renderer class]));
            return YES;
        }
        [view removeFromSuperview];
    }

    [self showMessage:[NSString stringWithFormat:@"没有可用的渲染后端：\n%@",
                       error.localizedDescription ?: @"Metal / OpenGL ES unavailable"]];
    return NO;
}

#pragma mark - Frame loop

- (void)startDisplayLink
{
    if (self.displayLink != nil) {
        return;
    }
    EdenDisplayLinkProxy *proxy = [[EdenDisplayLinkProxy alloc] init];
    proxy.target = self;
    proxy.selector = @selector(step:);
    self.displayLinkProxy = proxy;

    CADisplayLink *link = [CADisplayLink displayLinkWithTarget:proxy
                                                      selector:@selector(tick:)];
    link.preferredFramesPerSecond = 60;
    [link addToRunLoop:[NSRunLoop mainRunLoop] forMode:NSRunLoopCommonModes];
    self.displayLink = link;
}

- (void)step:(CADisplayLink *)link
{
    (void)link;
    if (!self.engineReady || self.backgrounded) {
        return;
    }
    if (eden_engine_tick(self.engine) != 0) {
        if (!self.reportedRuntimeError) {
            self.reportedRuntimeError = YES;
            NSLog(@"Eden: tick failed: %s", eden_engine_last_error(self.engine));
        }
        return;
    }
    EdenFrameInfo info = {0};
    const uint8_t *pixels = eden_engine_render(self.engine, &info);
    if (pixels != NULL) {
        [self.renderer presentPixels:pixels width:info.width height:info.height];
    }
}

#pragma mark - Input

- (void)touchesBegan:(NSSet<UITouch *> *)touches withEvent:(UIEvent *)event
{
    (void)touches;
    (void)event;
    if (self.engineReady) {
        eden_engine_click(self.engine);
    }
}

#pragma mark - Lifecycle / persistence

- (void)applicationDidEnterBackground:(NSNotification *)notification
{
    (void)notification;
    self.backgrounded = YES;
    [self.displayLink setPaused:YES];
    [self.audio setPaused:YES];
    if (self.engineReady) {
        NSString *path = [self savePath];
        if (eden_engine_save(self.engine, path.fileSystemRepresentation) != 0) {
            NSLog(@"Eden: save failed: %s", eden_engine_last_error(self.engine));
        }
    }
}

- (void)applicationWillEnterForeground:(NSNotification *)notification
{
    (void)notification;
    self.backgrounded = NO;
    if (self.engineReady) {
        NSString *path = [self savePath];
        if ([[NSFileManager defaultManager] fileExistsAtPath:path]) {
            if (eden_engine_load(self.engine, path.fileSystemRepresentation) != 0) {
                NSLog(@"Eden: load failed: %s", eden_engine_last_error(self.engine));
            }
        }
    }
    [self.audio setPaused:NO];
    [self.displayLink setPaused:NO];
}

- (NSString *)savePath
{
    NSString *documents = NSSearchPathForDirectoriesInDomains(NSDocumentDirectory,
                                                              NSUserDomainMask,
                                                              YES).firstObject;
    return [documents stringByAppendingPathComponent:@"eden_save.json"];
}

#pragma mark - Game data

- (NSString *)resolveGameDirectory
{
    NSFileManager *fileManager = [NSFileManager defaultManager];
    NSMutableArray<NSString *> *candidates = [NSMutableArray array];
    NSString *documents = NSSearchPathForDirectoriesInDomains(NSDocumentDirectory,
                                                              NSUserDomainMask,
                                                              YES).firstObject;
    if (documents != nil) {
        [candidates addObject:[documents stringByAppendingPathComponent:@"eden"]];
        [candidates addObject:documents];
    }
    NSString *support = NSSearchPathForDirectoriesInDomains(NSApplicationSupportDirectory,
                                                            NSUserDomainMask,
                                                            YES).firstObject;
    if (support != nil) {
        [candidates addObject:[support stringByAppendingPathComponent:@"eden"]];
    }
    NSString *bundled = [[NSBundle mainBundle] pathForResource:@"eden" ofType:nil];
    if (bundled != nil) {
        [candidates addObject:bundled];
    }

    for (NSString *candidate in candidates) {
        NSString *scr = [candidate stringByAppendingPathComponent:@"scr.paz"];
        if ([fileManager fileExistsAtPath:scr]) {
            return candidate;
        }
    }
    return nil;
}

- (NSString *)bundledFontPath
{
    NSArray<NSString *> *names = @[ @"GameFont.ttf", @"GameFont.otf",
                                    @"NotoSansCJK-Regular.ttc", @"SourceHanSansSC-Regular.otf" ];
    for (NSString *name in names) {
        NSString *stem = [name stringByDeletingPathExtension];
        NSString *extension = [name pathExtension];
        NSString *path = [[NSBundle mainBundle] pathForResource:stem ofType:extension];
        if (path != nil) {
            return path;
        }
    }
    return nil;
}

#pragma mark - Helpers

- (void)showMessage:(NSString *)text
{
    if (self.messageLabel == nil) {
        UILabel *label = [[UILabel alloc] initWithFrame:CGRectInset(self.view.bounds, 40.0, 40.0)];
        label.numberOfLines = 0;
        label.textAlignment = NSTextAlignmentCenter;
        label.textColor = [UIColor whiteColor];
        label.backgroundColor = [UIColor colorWithWhite:0.0 alpha:0.65];
        label.font = [UIFont systemFontOfSize:15.0];
        label.autoresizingMask = UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight;
        [self.view addSubview:label];
        self.messageLabel = label;
    }
    self.messageLabel.text = text;
}

- (void)destroyEngine
{
    [self.audio stop];
    if (self.engine != NULL) {
        eden_engine_destroy(self.engine);
        self.engine = NULL;
    }
    self.engineReady = NO;
}

#pragma mark - UIKit presentation

- (BOOL)prefersStatusBarHidden
{
    return YES;
}

- (BOOL)prefersHomeIndicatorAutoHidden
{
    return YES;
}

- (UIInterfaceOrientationMask)supportedInterfaceOrientations
{
    return UIInterfaceOrientationMaskLandscape;
}

- (BOOL)shouldAutorotate
{
    return NO;
}

@end
