# Eden for iOS 12.0 – 14.4

Native Objective-C host for the pure-Rust `eden-rs` runtime. No C++ engine,
no SDL, no FFmpeg: `libeden_ffi.a` contains the PAZ reader, script VM,
software rasteriser, Ogg/WAV decoder and lip-sync analyser.

```
platforms/ios/
├── Eden.xcodeproj/                 # checked-in project (Xcode 12.5+)
├── build_ios.sh                    # cargo + xcodebuild + optional ldid
└── EdenApp/
    ├── main.m / AppDelegate.*
    ├── ViewController.*            # 60 Hz CADisplayLink, touch, save/load
    ├── EdenRenderer.h/.m           # backend factory
    ├── EdenMetalRenderer.h/.m      # Metal 2.0 + MTKView
    ├── EdenGLRenderer.h/.m         # OpenGL ES 3.0 + CAEAGLLayer
    ├── EdenAudio.h/.m              # AVAudioSession + RemoteIO AudioUnit
    ├── Shaders/eden.metal          # fullscreen textured triangle
    ├── include/eden_ffi.h          # synced from crates/eden-ffi/include
    ├── lib/libeden_ffi.a           # produced by build_ios.sh
    ├── Info.plist                  # file sharing enabled
    └── LaunchScreen.storyboard
```

## 1. Put the retail data on the device

`Info.plist` sets `UIFileSharingEnabled` and
`LSSupportsOpeningDocumentsInPlace`, so the app's Documents folder appears in
Finder (iTunes file sharing), the Files app, Filza and TrollStore.

Create `Documents/eden/` and copy the original archives into it:

```
scr.paz  bg.paz  sys.paz  st.paz  bgm.paz  se.paz  voice.paz  mov.paz
```

`mov.paz` is optional (the OP/ED movies are not played yet). The host also
accepts the archives directly in `Documents/` and in
`Library/Application Support/eden/`.

## 2. Build

Requirements: macOS with Xcode 12.5+, Rust stable with the
`aarch64-apple-ios` target (the script installs it with rustup).

```bash
cd platforms/ios
./build_ios.sh                 # release, unsigned, prints Eden.app path
./build_ios.sh --debug         # Debug configuration
./build_ios.sh --cargo-only    # staticlib only (also works on Linux)
```

The script:

1. builds `eden-ffi` for `aarch64-apple-ios` with
   `IPHONEOS_DEPLOYMENT_TARGET=12.0`;
2. copies `libeden_ffi.a` and `eden_ffi.h` into the app;
3. runs `xcodebuild -sdk iphoneos -destination generic/platform=iOS` with
   signing disabled;
4. optionally ad-hoc signs with `ldid -S` when available.

For TrollStore, install the resulting `Eden.app` (or its `.ipa`) directly.
For a normal device, open the project in Xcode and select your team; the
bundle id is `cool.codebuddy.eden`.

## 3. Runtime behaviour

- The VM advances exactly one 60 Hz frame per `CADisplayLink` callback;
  `eden_engine_tick()` never rasterises, `eden_engine_render()` does.
- Metal is the default backend. Force OpenGL ES 3.0 with
  `defaults write cool.codebuddy.eden EdenBackend gles` (or change the
  `EdenBackend` key in `NSUserDefaults`). If the preferred backend cannot
  create its device/pipeline, the host automatically tries the other one.
- The 1024×640 stage is aspect-fit with black letterboxing. Both backends
  upload the same RGBA8 buffer; there is no GPU-side game logic.
- Tapping anywhere calls `eden_engine_click()`.
  `EdenAutoAdvance` (default `YES`) advances through `\a` markers.
- `AVAudioSessionCategoryPlayback`, 44.1 kHz, 1024-frame IO buffer. The
  RemoteIO callback calls `eden_engine_mix_audio()`; BGM, SE1/SE2 and voice
  are mixed additively, and the voice RMS drives the `.ani` mouth frame.
- Entering the background pauses the display link and audio, then writes
  `Documents/eden_save.json` through `eden_engine_save()`. Returning to the
  foreground restores it with `eden_engine_load()`.

## 4. iOS 14.4 constraints honoured by this host

- Deployment target 12.0, arm64 only, `UIRequiresFullScreen`, landscape.
- Metal 2.0 (`MTKView`, `MTLPixelFormatBGRA8Unorm`) and OpenGL ES 3.0
  (`CAEAGLLayer`, `#version 300 es`) are both available on 14.4.
- No whole-archive `mmap`: `eden-paz` streams entries and uses byte-bounded
  LRU caches (192 MiB in `Runtime`), so Jetsam limits are respected.
- The C ABI is documented in `include/eden_ffi.h`; the Rust side has a
  header-conformance test that fails if an exported symbol is missing.

## 5. Current limitations

- Movies (`op.avi` / `ed.avi`, WMV3/Xvid in `mov.paz`) are not decoded; the
  game scripts that reference them are skipped by the VM.
- The message panel uses the system PingFang font on iOS. To guarantee the
  original look, bundle a licensed CJK TTF/OTF as `GameFont.ttf` in the app
  resources; the host calls `eden_engine_load_font()` automatically.
- Save data is a single slot in `Documents/eden_save.json`.
