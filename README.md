# eden-rs

Pure-Rust port of the minori *Musica* engine for **eden\*** and **eden\* PLUS+MOSAIC**.
The workspace reads the original `.paz` archives directly — no conversion, no
C/C++ runtime. The only native code is the Objective-C iOS host and a minimal
statically linked FFmpeg for the retail movies.

## Status

| Check | Result |
| --- | --- |
| `cargo test --workspace` | 146 passed, 0 failed |
| `eden verify` | 12 archives / 9927 entries / 0 failures |
| `eden walkthrough` | 58/58 scripts, 0 errors |
| CI (`ci.yml`, `ios-ipa.yml`) | green on `main` |
| iOS package | arm64, min iOS 12.0, ldid-signed with entitlements |

Stage 3 renders the retail `sys.paz` UI **1:1**: title art with per-row hover
sprites, save/load cards and native thumbnails, backlog, CG gallery, 36-track
OST list, config checkboxes, the in-game radial menu, message-panel HUD
(`icons.png` / `iconson.png`) and the animated click-wait cursor. All of it is
rasterised by the Rust renderer and shared verbatim by the CLI, the browser
player and the iOS host.

## Workspace

```
crates/eden-paz      .paz VFS (Blowfish/RC4), PNG, .ani, LRU caches
crates/eden-script   GBK/CP932 .sc parser, 60 Hz VM, Serde JSON/bincode saves
crates/eden-render   1024x640 software rasteriser, UI state machine, text, transitions
crates/eden-audio    OGG/WAV/MP3 mixer, RMS lip-sync
crates/eden-video    AVI demux, WMV3/MPEG-4 ASP via FFmpeg, AVAssetReader fallback
crates/eden-ffi      safe Runtime + C ABI (v3), staticlib for iOS
crates/eden-app      CLI: verify, trace, shot, movie, ui, walkthrough, display, serve
platforms/ios        Xcode project, Metal 2.0 / GLES 3.0 host, RemoteIO audio
```

## Quick start

```bash
cargo build --release

# Point at the retail install (or pass --game DIR to every command)
export EDEN_GAME_DIR="/path/to/eden"

cargo run --release -p eden-app -- verify
cargo run --release -p eden-app -- trace --script A001.sc --frames 4000
cargo run --release -p eden-app -- shot        # reports/screenshots/
cargo run --release -p eden-app -- ui          # 10 sys.paz UI captures
cargo run --release -p eden-app -- movie       # real mov.paz frames + audio
cargo run --release -p eden-app -- walkthrough # all 58 scripts
cargo run --release -p eden-app -- display     # 60/120 Hz interpolation check
```

`--pm` selects the nested `eden PLUS MOSAIC` data for `trace`, `serve` and
`walkthrough`.

### Interactive browser player

```bash
cargo run --release -p eden-app -- serve --open
# http://127.0.0.1:8787
```

`serve` streams PNG frames over `multipart/x-mixed-replace`, maps pointer,
wheel and keyboard input onto the same hit regions as the iOS host, and serves
BGM/SE/voice clips straight out of the archives. `--fps N`, `--bind ADDR`,
`--save-dir DIR` and `--pm` are supported.

### Asset catalogue

```bash
cargo run --release -p eden-app -- catalog --out reports/sys_paz_catalog.json
cargo run --release -p eden-app -- catalog --extract reports/sys_paz
```

## iOS build

Requires macOS with Xcode 12.5+ and the `aarch64-apple-ios` Rust target.
See [`platforms/ios/README.md`](platforms/ios/README.md) for host details.

```bash
cd platforms/ios
./build_ffmpeg_ios.sh   # optional WMV3/MPEG-4/MP3 static libs
./build_ios.sh          # unsigned Eden.app
```

`build_ios.sh --cargo-only` builds just `libeden_ffi.a` and also works on Linux.

## Install the prebuilt IPA

CI publishes every green `main` build to the rolling `ci-latest` prerelease:

```bash
gh release download ci-latest \
  -p 'Eden-iOS-arm64.ipa' -p 'ipa_report.json' -D artifacts --clobber
```

The package is arm64-only, deployment target 12.0, and pseudo-signed with the
sandbox-safe `get-task-allow` entitlement only. It intentionally omits
`platform-application` / `com.apple.private.security.no-container`: those strip
GPU driver access and break Metal/OpenGL ES. For a normal device, open
`platforms/ios/Eden.xcodeproj`, select your team and run from Xcode.

Copy the retail archives into the app's Documents folder after installing:

```
Documents/eden/
  scr.paz  bg.paz  sys.paz  st.paz  bgm.paz  se.paz  voice.paz  mov.paz
```

`ipa_report.json` records the source commit, IPA size and SHA-256, Mach-O
architecture, minimum OS, signature presence and the exported engine/FFmpeg
symbols, so a download can be verified without unpacking it.

## Tests

```bash
cargo test --workspace -- --test-threads=2
```

Tests that need the retail corpus (`samples/`, `extracted/`, `EDEN_GAME_DIR`)
skip automatically on clean CI runners.
