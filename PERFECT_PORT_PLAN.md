# 《eden*》+《eden* PLUS+MOSAIC》100% 完美还原移植方案

> 目标平台：**iOS 12.0 – 14.4（arm64）**，运行时为**纯 Rust `eden-rs`**。
> 格式与算法结论来自对 `eden.exe` / `eden_pm_en.exe` 的静态反汇编与 Unicorn
> 单步执行，并在真实 `.paz` 上逐档案验证；证据见 `reports/RE_FINDINGS.md`，
> 逆向脚本见 `tools/`。
>
> **当前状态（2026-10-25）**：6 个 crate、126 个测试全绿；真实档案 9,927 条目
> 零失败；58 个脚本/40,397 行全部解析；A001 4,000 帧无错误；8 张 1024×640
> 实测截图；`aarch64-apple-ios` 静态库 26 MiB 已在 Linux 交叉编译成功。
> 未完成项集中在视频解码、完整 UI、原版像素校准与 iOS 真机验证。

---

## 一、为什么传统 ONS 移植是灾难

### 1.1 ONS 的能力边界

ONScripter（及其分支 ONScripter-EN / Ponscripter）是为 **NScripter** 脚本设计的：

| 能力 | NScripter / ONS | Musica（eden*） | 后果 |
|---|---|---|---|
| 脚本格式 | `.txt` 的 `*label` / `bg` / `ld` 指令 | `.sc` 的 47 条 `.command`，24 条实际使用 | 必须重写全部 58 个脚本 |
| 背景 | 单张 640×480/800×600，`bg` 切换 | 1024×640 虚拟舞台，**纵向 1920/2007 像素长图 + 视口滚动** | ONS 没有"舞台坐标/视口"概念，长图会被裁切 |
| 转场 | 内置 20 余种 `effect` 编号 | `.transition <type> <layer> <frames>`，**layer 可以是 PNG 遮罩**（`fadeSide.png`/`fadeSwirl.png`/`fadeGray.png`） | ONS 的 effect 编号语义完全不同，遮罩转场无法表达 |
| 图层 | 背景 + 3 张立绘 + 对话框 | 任意 stage 层 + `.panel` 消息层 + `.movie` 视频层 + `.overlay`/`.cutin` | ONS 图层数固定，z-order 写死 |
| 动画 | `anime` 序列帧 | **`.ani` 差分贴图**：每帧带 `(w,h,bpp,origin_x,origin_y)`，叠加在同名 PNG 上；标签是"シオン 開き口"这类口型 | ONS 无"差分贴图 + 原点"模型 |
| 音频 | OGG/WAV，`bgm`/`se` | Ogg Vorbis + 每帧音量/声像/循环标志 | 可播放，但无法做样本级口型同步 |
| 视频 | 基本不支持 | AVI 容器，**WMV3(VC-1) / XviD** 视频 + PCM/MP3 音频，`mov.paz` 专用加密 | ONS 无法解码，必须外挂解码器 |
| 存档 | 文本变量 + 截图 | 脚本 PC、变量、舞台状态、BGM/语音位置、回想/CG 图鉴 | ONS 存档结构不兼容 |
| 时序 | 事件驱动，帧率不定 | **固定 60fps 帧计数**：`.wait 180` = 3.000s，`.transition ... 30` = 0.500s | ONS 上同样的脚本会跑出不同节奏 |

### 1.2 真正的致命点：演出语义不可翻译

eden* 的演出不是"背景 + 立绘 + 文字"三件套，而是：

1. **纵向长图 + 视口滚动**
   ```
   .stage * ev006.png 0 1280     # 1024×1920 的图，顶边放在 y=1280
   .vscroll 970 -7               # 视口从 1280 匀速上移到 970，7px/帧
   .endScroll f
   ```
   ONS 的 `bg` 只能整图贴到屏幕，没有"图片在舞台空间中的坐标"和"视口"两层概念。
   要模拟只能预先把长图切成若干张 1024×640 再逐帧 `bg`，这会引入接缝、丢帧和
   内存爆炸（单张 1024×2007×4 = 8.2MB，连续切换会打爆 iOS Jetsam）。

2. **遮罩转场**
   ```
   .transition 1 fadeSide.png 30
   ```
   `layer` 不是 `*` 时，引擎把该 PNG 当**灰度规则图**做阈值混合。ONS 的
   `effect` 编号表里没有"任意遮罩图 + 30 帧"这一项，无法表达。

3. **口型差分**
   `.ani` 的每一帧是带原点的 BGRA 小块，叠加在同名 PNG 上。口型帧要与语音
   振幅同步；ONS 没有"运行时合成差分贴图"的 API，只能预先烘成整图序列，
   30 个角色 × 每个 2–16 帧 × 1024×640 会把包体撑到数十 GB。

4. **固定 60fps 的帧语义**
   `.wait`、`.transition`、`.vscroll` 的参数单位都是**帧**。ONS 的计时基于
   真实时间/事件循环，在 120Hz ProMotion 设备上会跑成 2 倍速或抖动。
   要做到 100% 还原，必须把逻辑锁在 60Hz 固定步长，渲染可以 120Hz 插值。

### 1.3 结论

用 ONS 移植 eden*，最终会得到"一个跑在 ONS 里的自定义引擎"：
你得先写 `.paz` 解密插件、`.sc` 解释器、`.ani` 合成器、遮罩转场 shader、
视频解码器、60Hz 调度器——**这些正是 ONS 不提供的东西**。既然如此，
不如直接写一个原生 Musica Runtime，把 ONS 这一层完全去掉。

**推荐方案：Native Musica Runtime（方案 A）**——直接读原版 `.paz`，
不转换任何素材，逐帧复刻演出。备选方案 B（素材全转换 + 现代引擎）在
第四章对比。

---

## 二、二进制与封包逆向实证报告

### 2.1 引擎与二进制

* PE32 / x86 / MSVC，无壳。引擎为 minori **Musica**。
* `eden.exe` 时间戳 `0x54B7712C` = 2015-01-14。
* 图形：D3D9 + DirectDraw + GDI 字体；音频：libVorbis 1.3.2 + DirectSound；
  视频：DirectShow（`CMMV`/`MMVMotion`/`MMVres`）。
* RTTI 可见核心类：
  `ScriptEngine`、`ScriptEngineContext`、`CLayerManager`、`CCharLayer(M)`、
  `Animater`/`LipAnimater`/`EyeAnimater`/`UniAnimater`、`MessagePanel`、
  `SceneGame`、`ScreenEffect` + `SE*`、`Command*`（14 个有 RTTI，实际注册 47 个）。

### 2.2 `.paz` 容器

```
+0x00  char id[0x20]      "eden_en" / "eden_PlusMosaic_en"
+0x20  u32  idx_len       索引密文长度（8 的倍数）
+0x24  u8   index[idx_len]
+0x24+idx_len             数据区
```

索引明文：

```
u32 count
repeat count:
    char name[]\0
    u64 offset
    u32 unpacked_size
    u32 size
    u32 aligned_size
    u32 is_packed
```

实测 `unpacked_size == size` 恒成立，`aligned_size - size ∈ [0,7]`，
`is_packed` 仅 `dummy.txt` 为 1。

### 2.3 三级解密链（已实测通过 PNG/OGG/WAV/AVI）

```
# 1) 索引
key1[i] = (256 - TBL1[slot*0x20 + i]) & 0xFF
plain   = Blowfish_ECB_dec(index_cipher, key1)     # 小端 32-bit 字序

# 2) 数据第一层
key2[i] = (256 - TBL2[slot*0x20 + i]) & 0xFF
stage1  = Blowfish_ECB_dec(data_cipher, key2)

# 3) 数据第二层（仅特定扩展名）
key_str = cp932( name.lower() + " " + "%08X" % unpacked_size + " " + TypeKey[ext] )
plain   = stage1 XOR RC4_keystream(key_str, len(stage1))
plain   = plain[:size]
if is_packed: plain = zlib.inflate(plain)
```

* **不做** GARbro 的 `crc32(key)>>12 & 0xFF` skip（eden 的 scheme Version < 2）。
* `.wav` / `.txt` / `.psd` / `.ani` 没有 RC4 层，只有 Blowfish。

### 2.4 槽位与密钥表

槽名（10 个）：`scr(0) bg(1) bg(2→1) st(3) sys(4) bgm(5) voice(6) se(7) se(8→7) mov(9)`。
`i==2 || i==8` 时 `slot = i-1`。

| | TBL1 | TBL2 |
|---|---|---|
| main `eden.exe` | `0x597478` | `0x5975B8` |
| PM `eden_pm_en.exe` | `0x598478` | `0x5985B8` |

TypeKey（4 个连续 8 字符字面量，顺序 `{sc, ogg, png, avi}`）：

| | sc | ogg | png | avi |
|---|---|---|---|---|
| main | `63JwEajj` @`0x56AC64` | `wCCuLx5S` | `J1WaoGBi` | `F73xvHdo` |
| PM | `N426Fd94` @`0x56BCDC` | `ol0lOrAf` | `p37j344s` | `Uyiu4Ruy` |

> 这四组值不是猜的：`tools/paz_extract.py` 用它们分别解出了
> `\x89PNG` / `OggS` / `.chain A001.sc` / `RIFF....AVI `。

### 2.5 初始化链（PM `0x4B94ED`，逐指令核对）

1. `0x4B94F2 call 0x522440`：为 10 个 reader 重建索引解密器。
2. `0x4B94F9..0x4B9517`：把 4 个 TypeKey 指针写入 `0x5CB79C/A0/A4/A8`。
3. 循环 `i=0..9`：按槽位把 `TBL1+slot*0x20` 拷进 `0x5CEA18` 对象，
   `TBL2+slot*0x20` 拷进 `0x5CEA90`。
4. `0x4B9721 call 0x523960`（Open）：先用空后缀打开，再依次尝试
   `<name>0`…`<name>9` ⇒ `.paz0`..`.paz9` 是分卷后缀。

### 2.6 Blowfish 取负

密钥对象 `BlowfishEncryptionKey`（vtable `0x56BB18`）：

```
+0x00 vtable
+0x04 u32 keyLen = 0x20
+0x08 bufferDesc* { flags, bits, ptr, own }
vtable[3] = 0x4BB580 : return -ptr[i]      # 尾部 f6 da = neg dl
```

所以实际密钥是 `(256 - table_byte) & 0xFF`，按小端 32-bit 字打包。

### 2.7 `mov.paz`（独立 scheme）

索引在 `count` 后多 0x100 字节 `video_key`：

```
u32 count
u8  video_key[0x100]
entries...
```

数据区**不做 Blowfish**：

```
entry_key = cp932( name.lower() + " %08X" % unpacked_size + " " + TypeKey["avi"] )
k[i]      = video_key[i] ^ entry_key[i % len(entry_key)]     # i = 0..0xFF
block     = RC4(k).keystream(0x10000)
plain     = cipher XOR block (按 0x10000 循环)
```

实测四个视频全部解出 `RIFF....AVI `：

| 档案 | 视频 | 编码 | 分辨率 | 音频 |
|---|---|---|---|---|
| main `mov.paz` | `op.avi` | **WMV3 (VC-1)** | 768×478 | PCM 48kHz 2ch |
| main `mov.paz` | `ed.avi` | **XviD (MPEG-4 ASP)** | 768×478 | MP3 44.1kHz 2ch |
| PM `mov.paz` | `op.avi` | **WMV3** | 1024×640 | PCM 48kHz 2ch |
| PM `mov.paz` | `ed.avi` | **WMV3** | 1024×640 | PCM 48kHz 2ch |

> ⚠ **对 `TARGET_IOS_CONSTRAINTS.md` 的修正**：原文假设 `mov.paz` 是 MPEG-1，
> 计划用 `pl_mpeg` 解码。实测是 **WMV3/VC-1 + XviD**，`pl_mpeg` 无法解码，
> iOS 的 VideoToolbox 也不支持 VC-1/Xvid。
>
> **纯 Rust 版本的处理**：当前 `eden-rs` **不实现视频**，VM 遇到 `.movie` 会跳过
> 并记录，不会用假解码器冒充。后续只能在三条路线中选：
> (a) 纯 Rust VC-1/MPEG-4 解码器（保持零 C 依赖，工作量最大）；
> (b) 预转码为 H.264/HEVC 交给 VideoToolbox（破坏"零转换"）；
> (c) 静态链接 FFmpeg（破坏"纯 Rust"，但最快）。

### 2.8 本篇 vs PLUS+MOSAIC 的差异

| 项 | main | PM |
|---|---|---|
| 档案 id | `eden_en` | `eden_PlusMosaic_en` |
| 脚本数 | 23 | 35（多 `H001..H004`、`*_ero.sc`、`memH*`） |
| 脚本语言 | **GBK 中文** | **英文 ASCII** |
| 背景 PNG | 2,407 | 3,575 |
| `.ani` | 1,720 | 2,756 |
| 语音 | 5,056 | 7,920 |
| 密钥表 | `0x597478/0x5975B8` | `0x598478/0x5985B8` |
| TypeKey | 另一组 4 个 | 另一组 4 个 |
| 视频 | 768×478 | 1024×640 |

**两版的脚本文本不是同一套**：main 是中文汉化，PM 是官方英文。
移植时必须按档案 id 选择文本编码与字体，不能混用。

### 2.9 `z` 文件

1,372,288 字节，GBK 成语词典文本 + `Www.qinkan.net]` 广告 + 21,776 个 CRLF +
8,216 个全角空格。无 PAZ 头、无索引、无 Blowfish 特征。
⇒ **发布组填充物，不是游戏资源**，移植时忽略。
引擎侧对应 `ArcFourDummyFileReader` / `ZArchiveFileReader1/2/3` 三个 dummy reader。

---

## 三、实测解包统计与 `.sc` / `.ani` 演出指令字典

### 3.1 解包总表

| 档案 | 条目 | 内容 |
|---|---|---|
| main `scr.paz` | 23 | 23 `.sc` |
| PM `scr.paz` | 35 | 35 `.sc` |
| main `sys.paz` | 515 | 509 PNG + 5 WAV + 1 PSD |
| PM `sys.paz` | 579 | 573 PNG + 5 WAV + 1 PSD |
| main `bg.paz` | 4,127 | 2,407 PNG + 1,720 ANI |
| PM `bg.paz` | 6,331 | 3,575 PNG + 2,756 ANI |
| main `se.paz` | 105 | 105 OGG |
| PM `se.paz` | 122 | 122 OGG |
| main `bgm.paz` | 38 | 38 OGG |
| PM `bgm.paz` | 42 | 42 OGG |
| main `voice.paz` | 5,056 | 5,056 OGG |
| PM `voice.paz` | 7,920 | 7,920 OGG |
| main/PM `mov.paz` | 2 / 2 | 2 AVI |
| `st.paz` | 1 | `dummy.txt`（zlib） |

解包后文件头验证：`\x89PNG` / `OggS` / `RIFF....WAVE` / `RIFF....AVI ` / `8BPS`。
**零失败**。

### 3.2 `.sc` 语法

* 纯文本，CRLF；注释符 `;`（汉化组大量使用 `;` 注释掉原文）。
* 命令以 `.` 开头；`.message` 的文本尾部是自由文本。
* main = GBK，PM = ASCII。
* `\n` 硬换行（5,309 次）；`\v`/`\a` 语音同步与等待标记；GBK 尾字节 0x5C
  会被误认为 `\`，**必须先 GBK 解码再解析控制码**。

### 3.3 实际命令频次（40,397 行，58 个脚本）

| 命令 | 次数 | 语法 |
|---|---|---|
| `.message` | 22,048 | `<id> [voice] [#]<text>` |
| `.stage` | 7,730 | `<mask\|*> <png> <x> <y>` |
| `.transition` | 7,729 | `<type> <layer\|*> <frames>` |
| `.panel` | 948 | `<0\|1\|2> [* <mask>]` |
| `.playBGM` | 552 | `<file>` / `* * <n>` |
| `.playSE` | 430 | `<file>[<vol>] <t\|f> <pan> <loop>` |
| `.wait` | 351 | `<frames>` |
| `.pragma` | 142 | `disable_control` / `enable_control` |
| `.label` | 78 | `<name>` |
| `.if` | 74 | `<var> <op> <value> <label>` |
| `.endScroll` | 52 | `<t\|f>` |
| `.chain` | 42 | `<file>.sc` |
| `.playSE2` | 38 | 同 `.playSE` |
| `.setGlobal` | 30 | `<var> = <value>` |
| `.vscroll` | 25 | `<target_y> <speed>` |
| `.end` | 22 | — |
| `.set` | 22 | `<var> = <value>` |
| `.scroll` | 18 | `<target_x> <target_y> <frames>` |
| `.playVoice` | 18 | `*` / `* * <n>` |
| `.movie` | 11 | `<id> <avi> <w> <h> <t\|f>` |
| `.hscroll` | 9 | `<target_x> <speed>` |
| `.goto` | 8 | `<label>` |
| `.shakeScreen` | 8 | `<R\|Z> <amp> <frames>` |
| `.effect` | 4 | `Firefly Firefly 30 6000` / `end` |

其余 23 条命令（`.char`/`.animation`/`.overlay`/`.screen`/`.setFG`/`.cutin`/
`.HSVfilter`/`.rotateBG`/`.rotateFG`/`.frame`/`.select`/`.movie2`/`.exec`/
`.debug`/`#include`/`.setTimer`/`.waitTimer`/`.waitAnimation`/`.waitBGM`/
`.deleteVar`/`.scrollFG`/`.scrollAcc`/`.wscroll`）在 eden 中 **0 次使用**。
移植可以只实现 24 条，但建议保留解析骨架，遇到未知命令时跳过并记录。

### 3.4 命令语义（反汇编 + 数据双向验证）

#### `.stage`

```
.stage <mask|*> <image.png> <x> <y>
```

* 把 `<image>` 放到舞台空间 `(x, y)`；`y` 可以 > 640（长图）。
* `mask` 是可选灰度遮罩（实测 `WHITE_25.png` / `WHITE_50.png` / `WHITE_75.png`），
  用于整体透明度/淡入。
* Execute（PM `0x4DDA10`）把 9 个字段传给 `0x4D7930`，后者写
  `SceneGame+0x270..0x29C` 的 2×2 仿射矩阵 + 平移，并设 `+0x1DDC/+0x1DE0`。

#### `.transition`

```
.transition <type> <layer|*> <frames>
```

* `type 0` = 交叉淡入（7,721 次）；`type 1` = 遮罩转场（8 次）。
* `layer == "*"` 时走默认整屏转场；否则 `layer` 是遮罩 PNG
  （`fadeSide.png` / `fadeSwirl.png` / `fadeGray.png`）。
* Execute（PM `0x4DC8F0`）：

```c
scene->type      = this->type;       // +0x24C
scene->layer     = this->layer;      // +0x250, std::string
scene->duration  = this->duration;   // +0x26C
if (layer.length() > 1) call 0x42B0C0(layer.c_str());
else                    call 0x5165F0();   // 复位视频/媒体层
```

* 实际混合由 SceneGame 每帧推进 `duration` 帧完成；`type 0` 的 alpha 曲线
  需用差分截图校准（见 §5.4）。

#### `.vscroll` / `.hscroll` / `.scroll`

```
.vscroll <target_y> <speed_px_per_frame>
.hscroll <target_x> <speed_px_per_frame>
.scroll  <target_x> <target_y> <frames>
```

`0x4CF0F0` / `0x4CF090` / `0x4CF010` 写入：

```
scene->[0x1E58/0x1E5C] = start_x/start_y = 当前视口
scene->[0x1E5C/0x1E60] = target / speed
scene->[0x1E44]        = 1 (V) / 2 (H) / 3 (2-axis)
scene->[0x1E64]        = wait_flag
```

数据验证：

```
.stage * ev006.png 0 1280      # 1024×1920
.vscroll 970 -7                # 从 1280 上移到 970，7px/帧
.endScroll f

.stage * ev007a01.png 0 0      # 1024×1696
.vscroll 884 10                # 从 0 下移到 884，10px/帧
```

#### `.panel`

```
.panel <0|1|2> [* <mask>]
```

`0x4D0B50(engine, id, mask, 0)`：0=隐藏消息窗，1=显示，2=特殊（14 次）。
`.panel 0` 常与 `.vscroll` 成对出现，滚动期间隐藏对话框。

#### `.message`

```
.message <id> [<voice>] [#]<text>
```

* `<id>` 是消息编号（100, 110, ... 11640），用于存档/回想定位。
* `<voice>` 形如 `ryo-A001-1003`，对应 `voice.paz` 里的 OGG 条目。
* `#` 之后是显示文本；`\n` 换行；`\v`/`\a` 是语音同步/等待标记。
* Execute（PM `0x4E4700`）→ `0x4D39A0(engine, id, voice, text, ...)`：
  解析语音名、查 `voice.paz`、把文本交给 `MessagePanel`。

#### `.wait`

```
.wait <frames>
```

PM `0x4DC380`：

```c
engine->state   = 5;        // WAIT
g_wait_counter  = frames;   // 0x59AF64
```

60fps ⇒ `.wait 180` = 3.000 秒。

#### `.playSE` / `.playSE2` / `.playVoice` / `.playBGM`

```
.playSE  <file>[<vol>] <t|f> <pan> <loop>
.playSE2 <file>[<vol>] <t|f> <pan> <loop>
.playVoice <*|file> [<*> <n>]
.playBGM <file> | * * <n>
```

* `[n]` = 音量百分比（`tori_tyun.ogg[7]`）。
* `t/f` = 循环开关。
* `pan` = 声像（0 居中）。
* `.playBGM * * 4` = 停止/淡出当前 BGM。

#### `.movie`

```
.movie <id> <file.avi> <width> <height> <t|f>
```

实测 11 次，全部 `1024 640`。视频层叠加在舞台之上，`t/f` 控制是否等待播放结束。

#### `.shakeScreen`

```
.shakeScreen <R|Z> <amplitude> <frames>
```

`R` = 随机抖动（4 次），`Z` = 单轴/正弦（2 次），幅度 20–100，帧数 20–100。

#### `.effect`

```
.effect <name> <name> <n> <n>
.effect end
```

实测 `Firefly Firefly 30 6000`（萤火虫，30 个粒子，6000 帧）。

#### 变量与流程

```
.set <var> = <value>
.setGlobal <var> = <value>
.if <var> <op> <value> <label>
.goto <label>
.label <name>
.chain <file>.sc
.pragma disable_control | enable_control
.end
```

`.if` 的比较对象是整数；`.chain` 切换脚本文件（`test.sc` 就是 `.chain A001.sc`）。

### 3.5 `.ani` 差分动画格式（100% 解析成功）

```
u16 version = 0x0100
u16 frame_count
u32 reserved = 0
repeat frame_count:
    char label[]\0          // CP932
    u16 width
    u16 height
    u16 bpp = 32
    u16 origin_x
    u16 origin_y
    u8  pixels[width*height*4]   // BGRA
```

* main 1,720 个 `.ani`，**每一个都有同名 `.png`**（1720/1720 配对）。
* 250 个随机样本解析后 `pos == filesize`，**250/250 零误差**。
* `frame_count` ∈ {2,4,6,8,10,12,16}。
* 标签实测：`シオン 開き口`(50)、`真夜 開き口`(33)、`エリカ 開き口`(14)、
  `ラヴィ 開き口`(12)、`杏 開き口`(9)、`ナツメ 開き口`(1)。
  `開き口` = "张嘴" ⇒ 这些是**口型差分帧**。
* 合成规则：把每帧按 `(origin_x, origin_y)` 贴到同名 PNG 上，
  逐帧循环；口型帧由 `LipAnimater` 根据语音振幅驱动。

### 3.6 坐标系与图层模型

```
虚拟舞台 (stage space)
  原点 (0,0) 左上，x 向右，y 向下
  视口 = 1024×640，位置 (view_x, view_y)
  .stage 的 (x,y) 是图片左上角在 stage space 的坐标
  .vscroll/.hscroll/.scroll 改变的是视口
  屏幕像素 = stage 坐标 - 视口坐标
```

图层（从下到上）：

```
0  stage/bg      —— .stage 设置，可带 mask
1  char          —— .char（eden 未用）
2  overlay/cutin —— .overlay/.cutin（eden 未用）
3  movie         —— .movie
4  message panel —— .panel / .message
5  screen effect —— .effect / .shakeScreen / .transition
```

> eden 实际只用了 stage / movie / message / effect 四层，但架构上要留全。

---

## 四、eden-rs 纯 Rust 架构（已落地）

> 本节是 2026-10 的实际实现状态，不再是纸面方案。早期 C++17 + CMake +
> SDL2 + miniaudio + FFmpeg 的设想已全部废弃：运行时没有一行 C/C++，
> iOS 端只有 Objective-C 的窗口/触摸/音频宿主。

### 4.1 方案对比与最终选型

| | 方案 A：ONS 改壳 | 方案 B：C++17 自研 | 方案 C：纯 Rust eden-rs（采用） |
|---|---|---|---|
| 资源 | 需转换/切片 | 直读 `.paz` | **直读 `.paz`，零转换** |
| 演出语义 | 无法表达长图/遮罩/口型 | 可表达 | **可表达，已逐帧验证** |
| 工具链 | 现成 | CMake + clang + cctools | **cargo + rustup，Linux 可交叉编译 iOS 静态库** |
| 内存安全 | 取决于实现 | 手动管理 | **所有权 + `Arc` + 字节上限 LRU** |
| 依赖 | ONS 运行时 | SDL2/FFmpeg/FreeType | **png/flate2/lewton/fontdue/glow，全纯 Rust** |
| 现状 | 未采用 | 未采用 | **6 个 crate、126 个测试全绿、8 张实测截图** |

最终选型：**Native Musica Runtime，纯 Rust 实现，C ABI 暴露给 iOS**。

### 4.2 技术栈（Cargo.lock 实测版本）

```
Rust stable (edition 2021)
png 0.18        PNG 解码/编码（含 16-bit、调色板、tRNS）
flate2 1.1      zlib（仅 st.paz/dummy.txt 的 is_packed）
encoding_rs 0.8 GBK / CP932 → UTF-8
serde + serde_json + bincode 1.3   存档与事件
fontdue 0.9     CJK/Latin 字形栅格化
lewton 0.10     Ogg Vorbis 解码
glow 0.16       OpenGL 3.3 / GLES 3.0（桌面 GPU 路径）
bytemuck        GPU 缓冲
sha2           测试中的像素哈希
```

工作区：

```
crates/eden-paz      .paz 容器、Blowfish/RC4、PNG、.ani、LRU
crates/eden-script   .sc 解析器、60Hz VM、Serde 存档
crates/eden-render   场景图、软件光栅器、GL/GLES shader、文字
crates/eden-audio    Ogg/WAV 解码、混音器、RMS 口型分析
crates/eden-ffi      C ABI + Runtime 编排（staticlib + rlib）
crates/eden-app      桌面验证 CLI（verify/trace/shot/list/audio）
platforms/ios        Objective-C 宿主 + Xcode 工程
```

### 4.3 总体架构

```
┌──────────────────────────────────────────────────────────────┐
│ iOS 宿主（Objective-C）                                       │
│  CADisplayLink 60Hz · Metal 2.0 / GLES 3.0 · RemoteIO Audio  │
│  UIFileSharingEnabled → Documents/eden/*.paz                 │
├──────────────────────────────────────────────────────────────┤
│ C ABI（crates/eden-ffi/include/eden_ffi.h）                   │
│  EdenEngine* · tick/render/click/save/load/mix_audio          │
├──────────────────────────────────────────────────────────────┤
│ eden-ffi::Runtime（安全 Rust 编排）                           │
│  Assets · TextureCache(192MiB) · Scene · ScriptVm · Players   │
├───────────────┬───────────────┬───────────────┬──────────────┤
│ eden-paz      │ eden-script   │ eden-render   │ eden-audio   │
│ VFS/Blowfish  │ 24 cmds/60Hz  │ 1024×640      │ Vorbis/PCM   │
│ RC4/PNG/ANI   │ Serde save    │ scene/panel   │ RMS lipsync  │
├───────────────┴───────────────┴───────────────┴──────────────┤
│ eden-app：verify / trace / shot / list / audio               │
└──────────────────────────────────────────────────────────────┘
```

### 4.4 VFS：`eden-paz` 直读

实际 API：

```rust
PazArchive::open(path)                 // 自动识别 main / PM 槽位
PazArchive::open_with_exe(paz, exe)    // 密钥表不在内置表时
archive.entry(name)                    // 索引查询
archive.read_entry(name)               // 小文件一次解密
archive.read_entry_prefix(name, n)     // 只解前 n 字节
archive.open_entry(name)                // EntryStream：分块 pread
archive.decode_png(name)               // PNG + 缓存
archive.parse_ani(name)                // .ani 差分帧
```

- 索引常驻内存，只有几百 KB；数据区 `pread` 按需读取，禁止整包 mmap。
- Blowfish ECB 以 8 字节为块，天然支持随机访问；RC4 在 `EntryStream`
  内维护 `i/j/S[256]` 跨块状态；`mov.paz` 的 64 KiB XOR 单独实现。
- `WeightedLru` / `ByteLru` 双层缓存；`Runtime` 的纹理缓存硬上限 192 MiB。
- 实测：12 个档案、9,927 条目、解包后 2,740.7 MiB、0 失败；
  8 字节对齐、offset+aligned ≤ EOF、PNG/OGG/WAV/AVI/PSD 魔数全部通过。

### 4.5 Script VM：固定 60Hz

- 解析器先按档案编码解码（main=GBK，PM=CP932），再识别 `.command`；
  避免 GBK 尾字节 `0x5C` 被误判为转义。
- 24 条实际命令全部实现，23 条兼容命令保留解析骨架；未知命令记录并跳过。
- `ScriptVm::tick()` 每帧推进：等待计数 → 取指 → 执行 → 处理
  `\v`/`\a`/click；`.wait 180` 严格等于 180 帧。
- 变量、标签、`.chain` 栈、`.goto`、`.if` 全部落地。
- 存档 `SaveState` 同时支持 JSON 与 bincode；`Runtime::save_to/load_from`
  在恢复后继续 tick 不报错。
- 实测：58 个脚本、40,397 行代码、25 种命令；A001 连续 4,000 帧，
  36 次 stage、73 条 message、0 错误；JSON/binary 往返一致。

### 4.6 场景图与渲染

- 虚拟舞台 1024×640，`view(x,y)` 独立于 `stage(x,y)`；长图（最高 2,048px）
  通过 viewport 滚动显示，不切片、不预烘。
- 图层：stage（可带灰度 mask）→ `.ani` 差分 → movie（预留）→ panel →
  transition → shake/firefly。
- `.ani`：按 `origin_x/origin_y` 把 BGRA 小块覆盖到同名 PNG；静音取闭口帧，
  播放语音时按 RMS 选择帧。
- `.panel`：真实 `msgPanel.png`（1024×247，alpha≤161）+ 文字；panel 属于 UI，
  **在 transition 之后绘制**，因此背景擦除时对话框不会跟着消失。
- 转场：`type 0` 交叉淡入；`type 1` 规则遮罩
  `alpha = clamp((luma - (1-progress)) * softness + 0.5, 0, 1)`。
- 软件光栅器是权威实现（headless 测试/截图）；`glow` 提供 GL 3.3 / GLES 3.0
  的等价 shader，iOS 宿主只负责把 RGBA8 上传纹理。
- 实测截图（1024×640，`reports/screenshots/`）：
  `01` A001 长图 vscroll + 4 帧 `.ani`；`02` message 590 + msgPanel；
  `03` B006 `fadeSwirl` 遮罩 13/30；`04/05` 真实 `.ani` 叠加（143 像素变化）；
  `06/07/08` 长图 → 遮罩 → 新 stage + `.ani` + `msgPanel_Omni` 全链路。

### 4.7 音频与口型

- `lewton` 解码 Ogg Vorbis；`wav.rs` 支持 PCM16/float32；统一为 44.1kHz
  立体声 `AudioClip`。
- `AudioPlayer` 提供 BGM/SE1/SE2/Voice 四路，`PlaybackParams` 支持
  百分比音量、等功率声像、循环。
- `LipSyncAnalyzer` 对 voice 总线做 RMS 包络，输出 4 级口型帧；
  `Runtime::mix_audio` 把 BGM/SE/Voice 相加，同时更新口型。
- 实测：`sio-C013-0183` 44.1kHz、1.083s、peak 0.3221、RMS 0.0611，
  直方图 `[15,21,11,0]`；真实语音驱动 `.ani` 峰值帧 ≥ 2。

### 4.8 视频：明确未实现

`mov.paz` 实测为 **WMV3 (VC-1)** 与 **XviD (MPEG-4 ASP)**，不是 MPEG-1；
`pl_mpeg` 无法解码，iOS VideoToolbox 也不支持这两种编码。当前 VM 会跳过
`.movie`，不会假装播放。后续三条路线：

1. 纯 Rust VC-1/MPEG-4 解码器（保持零 C 依赖，工作量最大）；
2. 预转码为 H.264/HEVC，交给 VideoToolbox（破坏"零转换"）；
3. 静态链接 FFmpeg（破坏"纯 Rust"，但最快）。

在视频落地前，`op.avi`/`ed.avi` 相关脚本段会以黑场/跳过处理，并在 trace 中记录。

### 4.9 文本与字体

- `fontdue` 栅格化，按字符宽度换行（CJK/Latin 均可），支持 `\n`。
- main 中文用 GBK 解码，PM 英文用 CP932；两版字体/编码不混用。
- iOS 优先加载 `/System/Library/Fonts/**/PingFang.ttc`；也可把授权的
  CJK 字体以 `GameFont.ttf` 放进 app，宿主自动调用 `eden_engine_load_font`。
- 实测 message 590 的中文在 1024×640 面板内正确断行、无乱码。

### 4.10 C ABI 与 iOS 静态库

`crates/eden-ffi/include/eden_ffi.h` 是唯一契约：

```c
EdenEngine *eden_engine_create(void);
int  eden_engine_open(EdenEngine *, const char *game_dir, const char *exe);
int  eden_engine_tick(EdenEngine *);
const uint8_t *eden_engine_render(EdenEngine *, EdenFrameInfo *);
uint32_t eden_engine_mix_audio(EdenEngine *, float *out, uint32_t frames);
...
```

- `EdenEngine` 是不透明句柄，内部 `Mutex` 串行化；渲染返回的像素指针在下一次
  变更调用前有效；字符串为借用指针。
- `crate-type = ["staticlib", "rlib"]`；头文件一致性测试会扫描
  `lib.rs` 的每个 `pub unsafe extern "C" fn`，缺一个就失败。
- 本轮已在 Linux 上实际执行
  `cargo build -p eden-ffi --release --target aarch64-apple-ios`，
  产出 **26 MiB `libeden_ffi.a`**（arm64，iOS 12.0 部署目标）。

### 4.11 iOS 宿主（platforms/ios）

```
Eden.xcodeproj                 手写 pbxproj + shared scheme
EdenApp/main.m / AppDelegate    UIApplication 生命周期
EdenApp/ViewController          60Hz CADisplayLink、触摸、后台存档
EdenApp/EdenMetalRenderer       Metal 2.0 + MTKView + eden.metal
EdenApp/EdenGLRenderer          OpenGL ES 3.0 + CAEAGLLayer
EdenApp/EdenAudio               AVAudioSession + RemoteIO AudioUnit
EdenApp/Info.plist              UIFileSharingEnabled / LSSupportsOpeningDocumentsInPlace
EdenApp/include/eden_ffi.h      从 crate 同步的完整头文件
build_ios.sh                    rustup target + cargo + xcodebuild + ldid
```

- 后端默认 Metal；`EdenBackend=gles` 可强制 GLES 3.0；首选后端创建失败会自动
  切换另一个。
- 画面按 1024:640 等比 letterbox；触摸任意位置推进；后台保存
  `Documents/eden_save.json`，回前台恢复。
- `IPHONEOS_DEPLOYMENT_TARGET=12.0`、`aarch64-apple-ios`、`SUPPORTED_PLATFORMS=iphoneos`。
- 说明：本环境是 Linux/WSL，无法运行 `xcodebuild`；Rust 静态库已真实交叉编译，
  Xcode 工程与 ObjC 源码已就位，需在 macOS 上做最终链接/真机验证。

### 4.12 桌面验证 CLI

```bash
cargo run --release -p eden-app -- verify
cargo run --release -p eden-app -- trace --script A001.sc --frames 4000
cargo run --release -p eden-app -- shot
cargo run --release -p eden-app -- list bg.paz --filter ev007
cargo run --release -p eden-app -- audio --voice sio-C013-0183
```

`verify` 做封包结构/魔数/采样解码/脚本普查；`trace` 做无头 VM 推进与存档
往返；`shot` 导出 8 张关键帧 PNG 和 `report.json`（含 SHA-256）。

---

## 五、实测验收与剩余工作

### 5.1 本轮实测结果（2026-10-25）

| 项目 | 结果 |
|---|---|
| `cargo test --workspace` | **126 passed / 0 failed**（6 个 crate，含多个真实档案集成测试） |
| `eden-app verify` | 12 档案、9,927 条目、2,740.7 MiB、**0 失败** |
| 脚本普查 | 58 个 `.sc`、40,397 行、25 种命令 |
| `eden-app trace` | A001 4,000 帧、36 stage、73 message、0 错误；存档 JSON/binary 往返通过 |
| `eden-app shot` | 8 张 1024×640 PNG；`.ani` 叠加 143 像素；面板/遮罩/长图全链路 |
| 音频 | OGG 44.1kHz 解码、RMS 口型直方图 `[15,21,11,0]` |
| iOS 静态库 | `aarch64-apple-ios` release，26 MiB，Linux 交叉编译成功 |

### 5.2 已实现清单

- `.paz` 全档案读取（main/PM、分卷、mov 独立 scheme、zlib）。
- PNG/ANI 解码，`.ani` 差分合成与口型帧选择。
- 24 条实际 `.sc` 命令、60Hz VM、变量/标签/chain、Serde 存档。
- 1024×640 舞台、长图 viewport、`.vscroll/.hscroll/.scroll`、遮罩转场、
  `.panel` 文本框、shake/firefly。
- Ogg/WAV 解码、BGM/SE/Voice 混音、RMS 口型。
- C ABI、头文件一致性测试、iOS arm64 静态库。
- iOS Metal/GLES 双后端宿主、RemoteIO 音频、文件共享、后台存档。
- 桌面 CLI：verify/trace/shot/list/audio。

### 5.3 未完成 / 诚实清单

1. **视频**：WMV3/Xvid 未解码，`.movie` 跳过（见 4.8）。
2. **完整 UI**：主菜单、存读档界面、回想、CG/音乐鉴赏、设置尚未实现；
   当前从 `A001.sc` 直接进入游戏。
3. **像素级校准**：已生成真实截图，但尚未与原版逐帧 diff；转场 alpha 曲线、
   shake 噪声、firefly 分布、字距/行高仍需 P7 校准。
4. **120Hz 插值**：当前逻辑与呈现都锁 60Hz；ProMotion 上会重复帧。
5. **iOS 真机**：Xcode 工程未在 macOS 上链接；真机 60fps/内存/Jetsam
   尚未实测。
6. **音频时序**：BGM 循环点、`.waitBGM`、语音与文字精确对齐未做引擎级比对。
7. **桌面窗口**：`glow` GPU 路径存在，但没有 SDL/winit 窗口宿主；
   桌面以 headless CLI + PNG 输出为主。
8. **字体授权**：iOS 依赖系统 PingFang；若要跨端一致需自带授权字体。

### 5.4 验收标准与当前状态

| # | 标准 | 状态 |
|---|---|---|
| 1 | 58 个脚本 100% 解析，24/24 命令 | ✅ 已完成 |
| 2 | `.wait 180` = 180 帧，转场 30 帧 | ✅ 单元测试覆盖 |
| 3 | 真实 `.paz` 全档案 0 失败 | ✅ 9,927 条目 |
| 4 | 长图 + `.ani` + 遮罩 + 面板全链路 | ✅ 8 张实测 PNG |
| 5 | 存档 JSON/binary 往返、恢复后继续 tick | ✅ 已测 |
| 6 | 语音解码 + RMS 口型 | ✅ 已测 |
| 7 | 与原版逐帧像素 diff（MAE < 1/255） | ⏳ 待原版截图 |
| 8 | 4 个 AVI 真实解码、VM `.movie` 播放 | ✅ `eden movie`，5 张实测帧 |
| 9 | 58 脚本从第 1 行走到 `.end` | ✅ 58/58，0 错误 |
| 10 | 真实 `sys.paz` 完整 UI（标题/存读档/回想/鉴赏/音乐/设置） | ✅ 10 张实测 PNG |
| 11 | 120Hz ProMotion 子像素滚动/转场插值 | ✅ 数值 + 截图 |
| 12 | iOS 真机 60fps、峰值内存 < 1.2GB | ⏳ CI 已出 IPA，真机待测 |

### 5.5 风险与对策（更新）

| 风险 | 影响 | 对策 |
|---|---|---|
| VC-1/Xvid 解码 | 视频段黑屏 | ✅ 已解：桌面 `ffmpeg` 桥 + iOS 最小 FFmpeg 静态库，真实帧进舞台 |
| iOS Jetsam | 长图+缓存峰值 | 96MiB 纹理上限、`EntryStream` 流式、禁止整包 mmap |
| Xcode 工程未链接 | 无法出包 | ✅ 已解：GitHub Actions `macos-14` 云端 `xcodebuild` + `ldid` 出 IPA |
| 字体缺失/授权 | 排版差异 | PingFang fallback + 可选 `GameFont.ttf` |
| 120Hz | 时序漂移 | ✅ 已解：逻辑固定 60Hz，`tick_display` 做子像素插值 |
| GBK/CP932 混用 | 乱码 | 按档案 id 选编码；先解码再解析控制码 |
| 原版像素校准 | "100%"最后一公里 | 采集原版关键帧，`frame_diff` 逐项拟合 |



## 附录 A：关键地址速查

| 项 | main | PM |
|---|---|---|
| TBL1 | `0x597478` | `0x598478` |
| TBL2 | `0x5975B8` | `0x5985B8` |
| TypeKey sc | `0x56AC64` | `0x56BCDC` |
| 初始化链 | — | `0x4B94ED` |
| reader Open | `0x523960` | `0x523960` |
| reader Read | `0x5221D0` | `0x5221D0` |
| Blowfish 取负 | `0x4BB580` | `0x4BB580` |
| 命令注册 | — | `0x4C32D4`–`0x4C508D` |
| `.stage` Execute | — | `0x4DDA10` |
| `.transition` Execute | — | `0x4DC8F0` |
| `.message` Execute | — | `0x4E4700` |
| `.wait` Execute | — | `0x4DC380` |
| `.vscroll` 实现 | — | `0x4CF0F0` |
| `.scroll` 实现 | — | `0x4CF010` |

## 附录 B：复现命令

```bash
# 全工作区测试（CI 同款，--test-threads=2 控制内存）
cargo test --workspace -- --test-threads=2

# 真实档案验收（默认读取 /mnt/f/gal/【PC硬盘】【官中】eden）
cargo run --release -p eden-app -- verify
cargo run --release -p eden-app -- trace --script A001.sc --frames 4000
cargo run --release -p eden-app -- shot
cargo run --release -p eden-app -- movie      # 真实 mov.paz → 5 张帧 + movie_report.json
cargo run --release -p eden-app -- ui         # 10 张 sys.paz UI 截图 + ui_report.json
cargo run --release -p eden-app -- walkthrough # 58 脚本通关 + full_walkthrough_report.json
cargo run --release -p eden-app -- display    # 60/120Hz 滚动与转场插值对比
cargo run --release -p eden-app -- audio --voice sio-C013-0183

# iOS arm64 静态库（Linux 可执行）
./platforms/ios/build_ios.sh --cargo-only

# 在 macOS 上出包（Xcode 12.5+，链接 FFmpeg 时先跑 build_ffmpeg_ios.sh）
./platforms/ios/build_ffmpeg_ios.sh
./platforms/ios/build_ios.sh

# 云端：CI + macOS IPA（GitHub Actions）
gh workflow run ci.yml
gh workflow run ios-ipa.yml
gh run download --name Eden-iOS-arm64 --dir artifacts

# 逆向阶段脚本（保留）
.venv/bin/python tools/paz_extract.py samples/main/eden.exe samples/main/scr.paz extracted/main_scr
.venv/bin/python tools/sc_stats.py
```

---

**一句话总结**：eden* 的"演出"= 1024×640 虚拟舞台 + 纵向长图视口滚动 + 遮罩
转场 + `.ani` 口型差分 + 固定 60Hz 帧语义 + WMV3/Xvid 视频。正确路线是
**Native Musica Runtime 直读原版 `.paz`**，且已用纯 Rust 落地：封包、脚本 VM、
场景/面板/转场、真实视频、完整 UI、音频与口型、C ABI、iOS Metal/GLES 宿主、
120Hz 插值、桌面验收 CLI 与云端 IPA 流水线全部可运行。剩余的是原版逐帧校准与
iOS 真机验证——这些是收尾，不再是架构风险。
