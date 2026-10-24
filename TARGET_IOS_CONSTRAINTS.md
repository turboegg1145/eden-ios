# 关键约束：iOS ≤ 14.4（iOS 12.0 ~ 14.4）+ 纯 Rust `eden-rs`

用户已明确指定：**本次《eden*》完美移植的目标平台为 iOS 14.4 及以下
（iOS 12 / 13 / 14.0 ~ 14.4）**。本文件是架构与实现的硬约束；与
`PERFECT_PORT_PLAN.md` 冲突时，以本文件为准。

当前实现状态（2026-10-25）：运行时是**纯 Rust**（无 C/C++ 引擎），iOS 端只有
Objective-C 的窗口/触摸/音频宿主；`aarch64-apple-ios` 静态库已在 Linux 上
实际交叉编译成功（26 MiB）。

---

## 1. 部署目标与工具链红线

- `IPHONEOS_DEPLOYMENT_TARGET = 12.0`，`ARCHS = arm64`，
  `SUPPORTED_PLATFORMS = iphoneos`。
- 允许的最低 Xcode：**12.5**（工程 `objectVersion = 51`，不依赖 Xcode 15/16）。
- Rust 目标：`aarch64-apple-ios`；产物
  `target/aarch64-apple-ios/release/libeden_ffi.a`，由
  `platforms/ios/build_ios.sh` 复制进 app。
- **禁用**：
  - SwiftUI、Swift 5.5+ 并发运行时、Flutter/Impeller；
  - 任何 iOS 15+ / Metal 2.4 / Metal 3 专属 API；
  - 动态链接第三方运行时（音频/视频/字体全部静态进 `libeden_ffi.a`）。
- UI 使用 UIKit + Objective-C；渲染使用 Metal 2.0（`MTKView`）或
  OpenGL ES 3.0（`CAEAGLLayer`）双后端。
- 只支持横屏（`UIInterfaceOrientationLandscapeLeft/Right`），
  `UIRequiresFullScreen = YES`，隐藏状态栏。

## 2. 内存红线（Jetsam）

iOS ≤ 14.4 的主力设备物理内存多为 2–4 GB，单进程超过约 1–1.4 GB 会被
Jetsam 强杀。`bg.paz` 为 1.96 GB，PM 版 `bg.paz` 为 2.93 GB，**绝不能整包
mmap 或整包解密**。

硬性要求与当前实现：

| 要求 | 实现 |
|---|---|
| 索引常驻、数据按需 | `PazArchive` 只常驻几百 KB 索引；`EntryStream` 用 `pread` 分块 |
| 不整包 mmap | 无 `mmap` 调用；Blowfish ECB 8 字节块随机访问 |
| RC4 跨块连续 | `EntryStream` 维护 `i/j/S[256]` 状态 |
| 纹理/图片缓存上限 | `Runtime` 的 `TextureCache` 硬上限 **192 MiB**；`ByteLru` 按字节淘汰 |
| 长图不切片预烘 | 1024×640 viewport 直接采样 1024×2048 长图 |
| `.ani` 按需 | 仅解析当前 stage 的同名 `.ani`，`Arc` 共享 |
| 音频流式 | 逐条 OGG 解码为 `AudioClip`；voice 播放时按需加载 |

新增缓存必须使用 `eden-paz::cache::{WeightedLru, ByteLru}` 或
`eden-render::TextureCache`，禁止无上限 `HashMap<String, RgbaImage>`。

## 3. 图形与着色器标准（Metal 2.0 / GLES 3.0）

- Metal 着色器目标 `-std=ios-metal2.0`；GLES 着色器 `#version 300 es`。
- 舞台固定 **1024×640**，等比 letterbox；逻辑帧率固定 **60 Hz**。
- 核心演出必须在两条后端上语义一致：
  - 长图 viewport 滚动：屏幕像素 = stage 坐标 − view 坐标；
  - `.ani` 差分：按 `origin_x/origin_y` 覆盖 BGRA 小块；
  - 规则遮罩转场：
    `alpha = clamp((luma - (1.0 - progress)) * softness + 0.5, 0.0, 1.0)`；
  - `.panel` 消息框是 UI 层，**在 transition 之后绘制**，不随背景擦除消失；
  - 震屏/萤火虫只影响舞台层，不影响文字排版。
- 后端选择：默认 Metal；`NSUserDefaults` 的 `EdenBackend=gles` 强制
  OpenGL ES 3.0；首选后端创建失败时自动回退另一个。
- 禁止在 GPU 侧做游戏逻辑；GPU 只接收 `eden_engine_render()` 返回的
  RGBA8 1024×640 缓冲并显示。

## 4. 音频管线（AVAudioSession + RemoteIO）

- `AVAudioSessionCategoryPlayback`，44.1 kHz，1024 帧 IO 缓冲；
  `AVAudioSession` 中断/拔出耳机时暂停，恢复时重新激活。
- `RemoteIO` AudioUnit 的 render callback 调用
  `eden_engine_mix_audio(engine, out, frames)`；BGM/SE1/SE2/Voice 在 Rust
  侧加法混音，回调内不做内存分配。
- Ogg Vorbis 由 `lewton` 解码，WAV 由内置 PCM 解码器处理；**不再使用
  stb_vorbis / miniaudio / libvorbis**。
- 口型：voice 总线 RMS → `LipSyncAnalyzer` 4 级包络 → `.ani` 帧索引；
  静音回闭口帧。目标误差 < 1 帧。
- 音频对象必须先 `stop`（`AudioOutputUnitStop` 会等待回调返回）再销毁
  `EdenEngine`，禁止在回调执行期间 `eden_engine_destroy`。

## 5. 视频（明确未实现，禁止假实现）

`mov.paz` 实测为：

| 档案 | 视频 | 编码 | 分辨率 | 音频 |
|---|---|---|---|---|
| main | `op.avi` | **WMV3 (VC-1)** | 768×478 | PCM 48kHz 2ch |
| main | `ed.avi` | **XviD (MPEG-4 ASP)** | 768×478 | MP3 44.1kHz 2ch |
| PM | `op.avi` | **WMV3** | 1024×640 | PCM 48kHz 2ch |
| PM | `ed.avi` | **WMV3** | 1024×640 | PCM 48kHz 2ch |

- **不是 MPEG-1**；`pl_mpeg` 无法解码，iOS VideoToolbox 也不支持
  VC-1/Xvid。
- 当前 `eden-rs` 在 VM 层跳过 `.movie` 并记录事件，不伪造视频帧。
- 后续三选一：
  1. 纯 Rust VC-1/MPEG-4 解码器（保持零 C 依赖，工作量最大）；
  2. 预转码为 H.264/HEVC，交给 VideoToolbox（破坏"零转换"）；
  3. 静态链接 FFmpeg（破坏"纯 Rust"，最快但包体/合规成本最高）。
- 在选定路线前，含 `.movie` 的脚本段以黑场/跳过处理，验收报告必须如实标注。

## 6. 签名、沙盒与资源挂载

- `Info.plist` 必须包含：
  - `UIFileSharingEnabled = true`
  - `LSSupportsOpeningDocumentsInPlace = true`
  - `UIRequiresFullScreen = true`
  - `UISupportedInterfaceOrientations` 仅横屏
- 资源目录优先级：
  `Documents/eden/` → `Documents/` → `Library/Application Support/eden/`
  → app bundle 的 `eden/`。检测 `scr.paz` 是否存在。
- 用户通过 Finder（iTunes 文件共享）、系统「文件」App、Filza 或 TrollStore
  直接放入原版 `.paz`，**无需转码**。
- 签名：`build_ios.sh` 默认 `CODE_SIGNING_ALLOWED=NO`；有 `ldid` 时自动
  `ldid -S`，可直接用于 TrollStore；正常设备在 Xcode 里选团队签名。
- Bundle id：`cool.codebuddy.eden`；无网络权限、无后台模式、无第三方 SDK。

## 7. 构建与验证

```bash
# Linux / WSL：只交叉编译 Rust 静态库
./platforms/ios/build_ios.sh --cargo-only

# macOS：静态库 + xcodebuild（可选 ldid）
./platforms/ios/build_ios.sh
```

- `build_ios.sh` 会：
  1. 检查/安装 `aarch64-apple-ios`；
  2. `cargo build -p eden-ffi --release --target aarch64-apple-ios`；
  3. 复制 `libeden_ffi.a` 和 `crates/eden-ffi/include/eden_ffi.h`；
  4. 在 macOS 上执行 `xcodebuild -sdk iphoneos -destination generic/platform=iOS`。
- C ABI 契约：`platforms/ios/EdenApp/include/eden_ffi.h` 与
  `crates/eden-ffi/include/eden_ffi.h` 必须一致；Rust 侧
  `header_declares_every_exported_function` 测试会逐个校验导出符号。
- 本轮实测：Linux 上静态库 26 MiB 构建成功；`cargo test --workspace`
  126 个测试全绿；`eden-app verify` 9,927 条目零失败。
- **未在真实 macOS/Xcode 上链接**：本环境为 Linux/WSL，无法运行
  `xcodebuild`。Xcode 工程、ObjC 源码、Info.plist、Metal shader 已就位，
  最终链接与真机 60fps/内存测试需在 macOS 上完成。

## 8. 明确禁止

1. 引入 C++17 引擎、CMake、SDL2、miniaudio、stb_vorbis、FreeType、
   D3D11/Vulkan 桌面后端作为 iOS 运行时依赖。
2. 整包 `mmap`、整包解压、无上限纹理缓存。
3. 用假解码器/占位帧冒充视频播放。
4. 依赖 iOS 15+ API、SwiftUI、Flutter/Impeller。
5. 把 `.paz` 转成自定义格式后再要求用户导入（必须支持原版直读）。

## 9. iOS 验收清单

- [ ] macOS 上 `build_ios.sh` 成功产出 `Eden.app`。
- [ ] iPhone 8（iOS 12/13/14）与 iPhone 12（iOS 14.4）真机启动。
- [ ] `Documents/eden/` 放入原版档案后能进入 A001。
- [ ] 长图 vscroll、`.ani` 口型、遮罩转场、消息框均正确显示。
- [ ] 触摸推进、自动播放、后台存档/恢复。
- [ ] BGM/SE/Voice 可听，口型随语音变化。
- [ ] 峰值内存 < 1.2 GB，连续运行 30 分钟不被 Jetsam。
- [ ] 逻辑 60 Hz 稳定；ProMotion 设备允许重复帧，但不加速。
- [ ] `.movie` 段明确显示为跳过/黑场，不崩溃、不伪装。
