# Eden for iOS 12.0 – 14.4

Native Objective-C host for the pure-Rust `eden-rs` runtime. The game engine
(PAZ reader, script VM, software rasteriser, UI state machine, Ogg/WAV/MP3
decoding, video demuxing) lives in `libeden_ffi.a`; the host only provides the
window, display link, touch input, audio unit and the video decoder bridge.

```
platforms/ios/
├── Eden.xcodeproj/                 # checked-in project (Xcode 12.5+)
├── build_ios.sh                    # cargo + xcodebuild + optional ldid
├── build_ffmpeg_ios.sh             # minimal arm64 FFmpeg (WMV3/MPEG-4/MP3)
├── Entitlements.plist              # TrollStore / jailbreak pseudo-signing
└── EdenApp/
    ├── main.m / AppDelegate.*
    ├── ViewController.*            # display link (60/120Hz), gestures, save/load
    ├── EdenRenderer.h/.m           # backend factory
    ├── EdenMetalRenderer.h/.m      # Metal 2.0 + MTKView
    ├── EdenGLRenderer.h/.m         # OpenGL ES 3.0 + CAEAGLLayer
    ├── EdenAudio.h/.m              # AVAudioSession + RemoteIO AudioUnit
    ├── EdenVideo.h/.m              # libavcodec / AVAssetReader decoder bridge
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

`mov.paz` is optional but fully supported: the OP/ED movies are decoded by
libavcodec and composited into the stage. The host also accepts the archives
directly in `Documents/` and in `Library/Application Support/eden/`.

## 2. Build

Requirements: macOS with Xcode 12.5+, Rust stable with the
`aarch64-apple-ios` target (the script installs it with rustup).

```bash
cd platforms/ios
./build_ffmpeg_ios.sh          # optional: WMV3/MPEG-4/MP3 static libs
./build_ios.sh                 # release, unsigned, prints Eden.app path
./build_ios.sh --debug         # Debug configuration
./build_ios.sh --cargo-only    # staticlib only (also works on Linux)
```

`build_ios.sh`:

1. builds `eden-ffi` for `aarch64-apple-ios` with
   `IPHONEOS_DEPLOYMENT_TARGET=12.0`;
2. copies `libeden_ffi.a` and `eden_ffi.h` into the app;
3. if `ffmpeg-ios/lib/libavcodec.a` exists, adds the FFmpeg header/library
   search paths and `-lavformat -lavcodec -lswscale -lavutil`;
4. runs `xcodebuild -sdk iphoneos -destination generic/platform=iOS` with
   signing disabled;
5. optionally ad-hoc signs with `ldid -S platforms/ios/Entitlements.plist`.

The same steps run in the cloud in `.github/workflows/ios-ipa.yml`
(`macos-14`), which uploads `Eden-iOS-arm64.ipa` as a build artifact and
attaches it to tag releases. Free accounts can exhaust the Actions artifact
storage quota; the workflow therefore also publishes the IPA plus a generated
`ipa_report.json` to the rolling `ci-latest` prerelease, which is the reliable
download path:

```bash
gh release download ci-latest \
  -p 'Eden-iOS-arm64.ipa' -p 'ipa_report.json' -D artifacts
```

When the artifact quota is available, `gh run download <run-id> --name
Eden-iOS-arm64 -D artifacts` works as well.

For TrollStore, install the resulting `Eden.app` (or its `.ipa`) directly.
For a normal device, open the project in Xcode and select your team; the
bundle id is `cool.codebuddy.eden`.

## 3. Runtime behaviour

- The display link runs at `UIScreen.maximumFramesPerSecond` (60 or 120).
  `eden_engine_tick_display(dt)` advances the fixed 60 Hz VM and computes a
  sub-pixel offset for the current scroll/transition; `eden_engine_render()`
  rasterises at the interpolated position.
- Metal is the default backend. Force OpenGL ES 3.0 with
  `defaults write cool.codebuddy.eden EdenBackend gles` (or change the
  `EdenBackend` key in `NSUserDefaults`). If the preferred backend cannot
  create its device/pipeline, the host automatically tries the other one.
- The app boots into the title screen (`eden_engine_ui_open(engine, 0)`),
  built from the real `sys.paz` art: start main story, PLUS+MOSAIC, load,
  CG/music Extra, config. In-game the same state machine provides the 10-slot
  save/load grid with live thumbnails, backlog with voice replay, CG gallery,
  music room, auto/skip/hide-text controls.
- Gestures: tap = advance / activate, drag = scroll (backlog or gallery),
  swipe up = open backlog, two-finger tap = hide the text box and UI.
- The 1024×640 stage is aspect-fit with black letterboxing. Both backends
  upload the same RGBA8 buffer; there is no GPU-side game logic.
- `AVAudioSessionCategoryPlayback`, 44.1 kHz, 1024-frame IO buffer. The
  RemoteIO callback calls `eden_engine_mix_audio()`; BGM, SE1/SE2 and voice
  are mixed additively, and the voice RMS drives the `.ani` mouth frame.
- Movies: `EdenVideo.m` opens the decrypted AVI through libavcodec (WMV3 /
  MPEG-4 ASP) and converts every frame to RGBA; without FFmpeg it falls back
  to `AVAssetReader`. `eden_engine_tick_display` feeds frames to the stage and
  a tap skips the movie.
- Entering the background pauses the display link and audio, then writes
  `Documents/eden_save.json` through `eden_engine_save()`. Returning to the
  foreground restores it with `eden_engine_load()` and resets the display
  clock so the first frame after resume has no huge delta.

## 4. iOS 14.4 constraints honoured by this host

- Deployment target 12.0, arm64 only, `UIRequiresFullScreen`, landscape.
- Metal 2.0 (`MTKView`, `MTLPixelFormatBGRA8Unorm`) and OpenGL ES 3.0
  (`CAEAGLLayer`, `#version 300 es`) are both available on 14.4.
- No whole-archive `mmap`: `eden-paz` streams entries and uses byte-bounded
  LRU caches (96 MiB texture cache in `Runtime`), so Jetsam limits are
  respected.
- FFmpeg is built with `--disable-everything` plus only the WMV3, MPEG-4 and
  MP3 decoders, the AVI/MP3 demuxers and swscale; no network, no programs.
- The C ABI is documented in `include/eden_ffi.h` (version 3); the Rust side
  has a header-conformance test that fails if an exported symbol is missing.

## 5. Current limitations

- VC-1 (`WVC1`) is not enabled in the minimal FFmpeg build; the retail movies
  use WMV3 and MPEG-4 ASP, which are covered. Add `--enable-decoder=vc1` if a
  future archive needs it.
- The message panel uses the system PingFang font on iOS. To guarantee the
  original look, bundle a licensed CJK TTF/OTF as `GameFont.ttf` in the app
  resources; the host calls `eden_engine_load_font()` automatically.
- Real-device performance (60/120 fps, peak memory, Jetsam) still needs an
  iPhone 8 / iPhone 12 run; the simulator and CI only prove the build.
