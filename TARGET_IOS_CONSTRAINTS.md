# 关键补充约束：目标移植平台为 iOS ≤ 14.4 (iOS 12.0 ~ 14.4)

用户已明确指定：**本次《eden*》完美移植的目标平台为 iOS 14.4 及以下版本（例如 iOS 12 / 13 / 14.0 ~ 14.4）**。

这一版本区间对架构选型、系统 API、内存管理与构建工具链有决定性影响，必须在方案中严格遵守：

## 1. 部署目标与框架选型红线 (`IPHONEOS_DEPLOYMENT_TARGET = 12.0`)
- **彻底摒弃高版本依赖框架**：
  - **禁用** 新版 Flutter（新版 Flutter 及 Impeller 强依赖高版本 iOS SDK 与 Metal 2.4+ 特性，在 iOS ≤ 14.4 上兼容性极差）。
  - **禁用** SwiftUI（iOS 13/14 初代 SwiftUI 存在严重生命周期与渲染 Bug）及 Swift 5.5+ 现代并发运行时高版本符号。
- **最佳黄金技术栈**：**C/C++17 核心引擎 + Objective-C (`UIKit` + `MetalKit` / `OpenGL ES 3.0` 双后端，或 `SDL2` + `sokol_gfx`)**：
  - 零外部臃肿运行时依赖，最低可向下兼容至 **iOS 11 / 12.0 ~ 14.4**（`arm64`）。
  - **支持 WSL/Linux 直接交叉编译出包**：采用純 C/C++ + Objective-C 架构，完全可以在当前 WSL 环境下通过 `clang` + `cctools-port` + `iPhoneOS14.4.sdk` + `ldid` 直接交叉编译打包出 `.ipa`（无需依赖高版本 macOS + Xcode 15/16）！

## 2. 针对 iOS ≤ 14.4 设备的内存红线 (Jetsam 防杀机制)
- iOS ≤ 14.4 的主力设备（如 iPhone 6s/7/8/X/XS/11/12、iPad Air 2/Pro 早期款）物理内存多为 **2GB ~ 4GB**，系统 `Jetsam` 进程内存上限极为严格（通常单进程超过 1GB~1.4GB 就会被系统直接强杀闪退）。
- **严禁全量 `mmap` 超大封包**：
  - `bg.paz` 高达 1.96GB，`eden PLUS MOSAIC/bg.paz` 高达 2.93GB。在 iOS 14.4 以下切勿一次性映射整个 3GB 文件或缓存过多未压缩 RGBA 纹理。
  - **必须采用 64 位文件偏移流式读取 (`pread` / `fseeko`) + 分块流式解密**，僅在内存常驻 `.paz` 的轻量级文件索引表（几百 KB）。
  - **显存/纹理严格 LRU 池化**：GPU 纹理缓存池（VRAM Cache）硬性限制在 **128MB ~ 256MB** 以内，切换场景（`.stage`）时立即回收非活跃图层纹理。

## 3. 图形与着色器标准 (Metal 2.0 / OpenGL ES 3.0)
- 着色器目标锁定为 **`-std=ios-metal2.0`**（并可提供 OpenGL ES 3.0 备选后端），不调用任何 iOS 15+ / Metal 2.4 / Metal 3 专属 API。
- 自定义 Fragment Shader 实现 Musica 引擎的全部核心演出：
  - **Rule 遮罩渐变转场 (`mask`)**：采样灰度规则图进行平滑阈值混合：`alpha = clamp((rule_luma - (1.0 - progress)) * softness + 0.5, 0.0, 1.0)`。
  - **图层混合模式**：Normal、Additive（加色发光）、Multiply（正片叠底）、Screen（滤色）。
  - **动态后处理滤镜**：双 Pass 高斯模糊（Gaussian Blur）、径向模糊（Radial Blur）、色调分离/怀旧（Sepia/Monochrome）、全屏泛光（Bloom）。
  - **2.5D 摄像机矩阵**：支持父子图层层级绑定、多平面视差位移（Parallax）、中心缩放、旋转与震屏（Quake）。

## 4. 音视频管线 (iOS 14.4 原生兼容)
- **音频与口型同步 (CoreAudio `AudioUnit` / `AVAudioSession`)**：
  - 使用 `stb_vorbis` / `libvorbis` + `miniaudio`（对接 iOS `RemoteIO AudioUnit`），在音频混音实时回调中计算当前 `voice` 通道的实时 RMS 包络振幅，驱动角色立绘 `.ani` 的口型帧（闭口 / 半张 / 全张）实时跳动。
- **视频演出 (`mov.paz` MPEG-1 / AVI 解码)**：
  - `mov.paz` 内为 MPEG-1 视频流，iOS 原生 `AVPlayer` 不直接支持裸 MPEG-1/AVI 容器；在引擎内直接静态链接轻量级 **`pl_mpeg`**（单头文件 MPEG-1 视频/MP2 音频解码器，零外部依赖）或轻量 `FFmpeg`，解码出 YUV420P 帧直接上传至 Metal 双通道（Y + CbCr）纹理，在 GPU 端完成 YUV->RGB 转换并参与图层合成！

## 5. 签名安装与沙盒资源挂载 (TrollStore / 越狱 / Filza 友好)
- iOS 14.0 ~ 14.4 完美支持 **TrollStore 巨魔永久签名** 与 **Unc0ver / Taurine 越狱环境**。
- `Info.plist` 开启 `UIFileSharingEnabled` 与 `LSSupportsOpeningDocumentsInPlace`，用户可通过爱思助手、iTunes、系统「文件」App 或 Filza 直接将 PC 原版 `.paz` 放入沙盒 `Documents/eden/`（或直接软链接外部路径），即放即玩，无需转码！
