//
//  ViewController.m
//  Eden
//
//  Hosts the pure-Rust Musica runtime (libeden_ffi.a) on iOS 12–14.4.
//  A CADisplayLink runs at the panel's native rate (60/120 Hz) and calls
//  eden_engine_tick_display(); the runtime advances the VM at exactly 60 Hz
//  and interpolates scroll/transition state for ProMotion. The selected
//  backend only receives the finished 1024x640 RGBA8 framebuffer.
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
@property (nonatomic, assign) CGPoint touchStart;
@property (nonatomic, assign) CGPoint lastPan;
@property (nonatomic, assign) CFTimeInterval touchStartTime;
@property (nonatomic, assign) NSInteger maxTouchCount;
@property (nonatomic, assign) CGFloat panAccum;
@property (nonatomic, assign) BOOL gestureHandled;
@property (nonatomic, assign) BOOL didScrollInTouch;

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
    view.multipleTouchEnabled = YES;
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
                  @"此安装包未内嵌 gamedata，请用 Finder / 文件 App / Filza 将\n"
                  @"scr.paz、bg.paz、sys.paz、st.paz、bgm.paz、se.paz、voice.paz\n"
                  @"复制到“我的 iPhone/Eden/eden/”后重新启动。\n\n"
                  @"Game data not found. Use the full IPA, or copy the retail .paz\n"
                  @"files to Documents/eden/ via iTunes file sharing, Files or Filza."];
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

    // The archives may live in the read-only app bundle (self-contained IPA),
    // so config/unlocks/save slots always go to Documents/saves.
    NSString *savesDirectory = [self savesDirectory];
    NSLog(@"Eden: save directory = %@", savesDirectory);

    NSString *exePath = nil;
    for (NSString *name in @[ @"eden.exe", @"eden_pm_en.exe" ]) {
        NSString *candidate = [gameDirectory stringByAppendingPathComponent:name];
        if ([[NSFileManager defaultManager] fileExistsAtPath:candidate]) {
            exePath = candidate;
            break;
        }
    }
    if (eden_engine_open_with_save_dir(self.engine,
                                       gameDirectory.fileSystemRepresentation,
                                       exePath.fileSystemRepresentation,
                                       savesDirectory.fileSystemRepresentation) != 0) {
        [self showMessage:[NSString stringWithFormat:@"无法打开游戏数据：\n%s",
                           eden_engine_last_error(self.engine)]];
        [self destroyEngine];
        return;
    }

    NSString *fontPath = [self bundledFontPath];
    if (fontPath != nil) {
        if (eden_engine_load_font(self.engine, fontPath.fileSystemRepresentation) != 0) {
            NSLog(@"Eden: font load failed (%@): %s",
                  fontPath.lastPathComponent, eden_engine_last_error(self.engine));
        } else {
            NSLog(@"Eden: font = %@", fontPath);
        }
    }

    // Never auto-advance on a real device by default: the in-game AUTO button
    // is the only way to enable the timed auto-read mode. `auto_advance` in the
    // runtime is the headless per-frame skip used by CLI smoke tests.
    eden_engine_set_auto_advance(self.engine, 0);

    // Boot into the retail title screen; its menu starts A001.sc (main) or
    // the PLUS+MOSAIC edition through the same UI state machine as desktop.
    eden_engine_ui_open(self.engine, 0);

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
    NSString *preferred = ([[NSUserDefaults standardUserDefaults] stringForKey:@"EdenBackend"]
                           ?: @"metal").lowercaseString;
    NSMutableArray<NSString *> *order = [NSMutableArray array];
    if ([preferred isEqualToString:@"gles"] || [preferred isEqualToString:@"gl"]) {
        [order addObjectsFromArray:@[ @"gles", @"metal", @"cg" ]];
    } else if ([preferred isEqualToString:@"cg"] || [preferred isEqualToString:@"coregraphics"]) {
        [order addObjectsFromArray:@[ @"cg", @"metal", @"gles" ]];
    } else {
        // Default chain: Metal -> OpenGL ES (3.0 then 2.0) -> CoreGraphics.
        [order addObjectsFromArray:@[ @"metal", @"gles", @"cg" ]];
    }

    // Keep one error per backend; never let a later failure hide an earlier one.
    NSMutableArray<NSString *> *failures = [NSMutableArray array];
    for (NSString *backend in order) {
        NSError *createError = nil;
        id<EdenRenderer> renderer = EdenCreateRenderer(backend, self.view.bounds, &createError);
        if (renderer == nil) {
            NSString *reason = createError.localizedDescription ?: @"renderer creation failed";
            NSLog(@"Eden: %@ unavailable: %@", backend, reason);
            [failures addObject:[NSString stringWithFormat:@"%@: %@", backend, reason]];
            continue;
        }

        UIView *view = renderer.view;
        view.frame = self.view.bounds;
        view.autoresizingMask = UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight;
        // Keep every touch (single and multi) on ViewController.view; MTKView
        // must never swallow them.
        view.userInteractionEnabled = NO;
        [self.view addSubview:view];

        NSError *prepareError = nil;
        if ([renderer prepareWithError:&prepareError]) {
            self.renderer = renderer;
            NSLog(@"Eden: renderer = %@", NSStringFromClass([renderer class]));
            return YES;
        }
        NSString *reason = prepareError.localizedDescription ?: @"prepare failed";
        NSLog(@"Eden: %@ prepare failed: %@", backend, reason);
        [failures addObject:[NSString stringWithFormat:@"%@: %@", backend, reason]];
        [view removeFromSuperview];
    }

    [self showMessage:[NSString stringWithFormat:@"没有可用的渲染后端：\n%@",
                       [failures componentsJoinedByString:@"\n"]]];
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
    NSInteger fps = 60;
    if (@available(iOS 10.0, *)) {
        fps = (NSInteger)[UIScreen mainScreen].maximumFramesPerSecond;
    }
    if (fps < 60) {
        fps = 60;
    }
    link.preferredFramesPerSecond = fps;
    NSLog(@"Eden: display link = %ld Hz", (long)fps);
    [link addToRunLoop:[NSRunLoop mainRunLoop] forMode:NSRunLoopCommonModes];
    self.displayLink = link;
}

- (void)step:(CADisplayLink *)link
{
    if (!self.engineReady || self.backgrounded) {
        return;
    }
    double dt = 1.0 / 60.0;
    if (@available(iOS 10.0, *)) {
        dt = link.targetTimestamp - link.timestamp;
    }
    if (!(dt > 0.0) || dt > 0.25) {
        dt = 1.0 / 60.0;
    }
    int ticks = eden_engine_tick_display(self.engine, dt);
    if (ticks < 0) {
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

- (CGPoint)stagePoint:(CGPoint)point
{
    CGSize bounds = self.view.bounds.size;
    if (bounds.width <= 0.0 || bounds.height <= 0.0) {
        return point;
    }
    CGFloat scale = MIN(bounds.width / 1024.0, bounds.height / 640.0);
    CGFloat ox = (bounds.width - 1024.0 * scale) * 0.5;
    CGFloat oy = (bounds.height - 640.0 * scale) * 0.5;
    return CGPointMake((point.x - ox) / scale, (point.y - oy) / scale);
}

- (void)touchesBegan:(NSSet<UITouch *> *)touches withEvent:(UIEvent *)event
{
    UITouch *touch = touches.anyObject;
    if (touch == nil) {
        return;
    }
    self.touchStart = [touch locationInView:self.view];
    self.lastPan = self.touchStart;
    self.touchStartTime = CACurrentMediaTime();
    self.maxTouchCount = (NSInteger)event.allTouches.count;
    self.panAccum = 0.0;
    self.gestureHandled = NO;
    self.didScrollInTouch = NO;
    if (self.engineReady) {
        CGPoint stage = [self stagePoint:self.touchStart];
        eden_engine_ui_move(self.engine, (int)lround(stage.x), (int)lround(stage.y));
    }
}

- (void)touchesMoved:(NSSet<UITouch *> *)touches withEvent:(UIEvent *)event
{
    self.maxTouchCount = MAX(self.maxTouchCount, (NSInteger)event.allTouches.count);
    if (!self.engineReady || event.allTouches.count != 1) {
        return;
    }
    UITouch *touch = touches.anyObject;
    if (touch == nil) {
        return;
    }
    CGPoint point = [touch locationInView:self.view];
    self.panAccum += point.y - self.lastPan.y;
    self.lastPan = point;
    CGPoint stage = [self stagePoint:point];
    eden_engine_ui_move(self.engine, (int)lround(stage.x), (int)lround(stage.y));
    const CGFloat step = 26.0;
    if (fabs(self.panAccum) >= step) {
        int steps = (int)(-self.panAccum / step);
        self.panAccum -= steps * step;
        if (steps != 0) {
            eden_engine_ui_scroll(self.engine, steps);
            self.didScrollInTouch = YES;
        }
    }
}

- (void)touchesEnded:(NSSet<UITouch *> *)touches withEvent:(UIEvent *)event
{
    if (!self.engineReady || self.gestureHandled) {
        return;
    }
    UITouch *touch = touches.anyObject;
    if (touch == nil) {
        return;
    }
    CGPoint end = [touch locationInView:self.view];
    CGFloat dx = end.x - self.touchStart.x;
    CGFloat dy = end.y - self.touchStart.y;
    CFTimeInterval elapsed = CACurrentMediaTime() - self.touchStartTime;

    if (self.maxTouchCount >= 2) {
        if (elapsed < 0.45 && hypot(dx, dy) < 28.0) {
            self.gestureHandled = YES;
            eden_engine_ui_two_finger_tap(self.engine);
        }
        return;
    }
    if (dy < -70.0 && fabs(dx) < 80.0) {
        self.gestureHandled = YES;
        eden_engine_ui_swipe_up(self.engine);
        return;
    }
    if (!self.didScrollInTouch && hypot(dx, dy) < 32.0 && elapsed < 1.5) {
        CGPoint stage = [self stagePoint:end];
        self.gestureHandled = YES;
        eden_engine_ui_tap(self.engine, (int)lround(stage.x), (int)lround(stage.y));
    }
}

- (void)touchesCancelled:(NSSet<UITouch *> *)touches withEvent:(UIEvent *)event
{
    (void)touches;
    (void)event;
    self.gestureHandled = YES;
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
    eden_engine_reset_display_clock(self.engine);
    [self.audio setPaused:NO];
    [self.displayLink setPaused:NO];
}

- (NSString *)savePath
{
    return [[self savesDirectory] stringByAppendingPathComponent:@"eden_save.json"];
}

/// Writable location for config, unlocks, slots and the resume snapshot.
/// Never inside the app bundle: that is read-only on a signed install.
- (NSString *)savesDirectory
{
    NSString *documents = NSSearchPathForDirectoriesInDomains(NSDocumentDirectory,
                                                              NSUserDomainMask,
                                                              YES).firstObject;
    if (documents == nil) {
        documents = NSTemporaryDirectory();
    }
    NSString *saves = [documents stringByAppendingPathComponent:@"saves"];
    [[NSFileManager defaultManager] createDirectoryAtPath:saves
                              withIntermediateDirectories:YES
                                               attributes:nil
                                                    error:NULL];
    return saves;
}

#pragma mark - Game data

- (NSString *)resolveGameDirectory
{
    NSFileManager *fileManager = [NSFileManager defaultManager];
    NSMutableArray<NSString *> *candidates = [NSMutableArray array];

    // 1. Archives embedded in the app bundle (self-contained IPA). This is
    //    read-only, which is fine: the engine only reads .paz from here.
    NSString *resources = [NSBundle mainBundle].resourcePath;
    if (resources != nil) {
        NSString *gamedata = [resources stringByAppendingPathComponent:@"gamedata"];
        [candidates addObject:gamedata];
        [candidates addObject:[gamedata stringByAppendingPathComponent:@"eden PLUS MOSAIC"]];
        [candidates addObject:resources];
        [candidates addObject:[resources stringByAppendingPathComponent:@"eden"]];
    }

    // 2. Writable sandbox, for sideloaded data.
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
    NSArray<NSString *> *names = @[ @"font.ttf", @"font.otf",
                                    @"GameFont.ttf", @"GameFont.otf",
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
