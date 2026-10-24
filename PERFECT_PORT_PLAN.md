# 《eden*》+《eden* PLUS+MOSAIC》100% 完美还原移植方案

> 目标平台：**iOS 12.0 – 14.4（arm64）**，同时保留桌面/Android 可构建性。
> 本方案的所有格式与算法结论均来自对 `eden.exe` / `eden_pm_en.exe` 的静态反汇编
> 与 Unicorn 单步执行，并在真实 `.paz` 上解包验证。证据汇总见
> `reports/RE_FINDINGS.md`，解包器见 `tools/paz_extract.py`。

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
> 计划用 `pl_mpeg` 解码。实测是 **WMV3/VC-1 + XviD**，`pl_mpeg` 无法解码。
> 必须改用 **FFmpeg（LGPL，启用 `vc1` + `mpeg4` + `mp3` 解码器）**，
> 或自带 VC-1 解码器。iOS 的 VideoToolbox 也不支持 VC-1/Xvid。

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

## 四、100% 移植架构方案

### 4.1 方案对比

| | 方案 A：Native Musica Runtime | 方案 B：素材全转换 + 现代引擎 |
|---|---|---|
| 资源 | 直接读原版 `.paz`，零转换 | 全量解包 + 转 PNG/OGG/MP4 |
| 包体 | 与原版相同（~5.3GB） | 转码后更大（PNG 无损）或更小（有损） |
| 保真 | 逐字节一致 | 转码有损，颜色/alpha 可能变 |
| 工作量 | 写 VFS + VM + 渲染 + 音频 + 视频 | 仍要写 VM/演出，只是资源层简单 |
| iOS 沙盒 | 用户直接放 `.paz` 即可 | 需要导入转换后的资源 |
| 结论 | **推荐** | 仅作为"资源预览/调试"辅助 |

### 4.2 技术栈（对齐 iOS 12–14.4 约束）

```
语言      C++17（核心） + Objective-C（iOS 平台层）
构建      CMake + clang；iOS 用 cctools-port + iPhoneOS14.4.sdk + ldid
窗口/输入  SDL2（桌面/Android）；UIKit + MetalKit（iOS）
图形      Metal 2.0（iOS）/ OpenGL ES 3.0（备选）/ D3D11 或 Vulkan（桌面，可选）
音频      miniaudio + stb_vorbis（iOS 走 RemoteIO AudioUnit）
视频      FFmpeg（LGPL，vc1 + mpeg4 + mp3 + pcm_s16le）
文本      FreeType + 自带位图图集；GBK/CP932 → UTF-8
存档      JSON + zlib + 缩略图 PNG
```

> 不用 Rust/wgpu：iOS 12–14.4 上 wgpu 需要较新的 Metal 特性与工具链，
> 且会引入额外运行时；C++17 + Metal/GLES3 更稳、包体更小、交叉编译更简单。

### 4.3 总体架构

```
┌─────────────────────────────────────────────────────────────┐
│                        App / Platform                        │
│  iOS: UIKit view + Metal layer + AudioUnit + touch           │
│  Desktop: SDL2 window + GL/Metal + keyboard/mouse            │
├─────────────────────────────────────────────────────────────┤
│                      Musica Runtime (C++)                    │
│                                                              │
│  ┌──────────┐  ┌──────────┐  ┌───────────┐  ┌────────────┐  │
│  │  VFS     │  │ ScriptVM │  │  Scene    │  │  Audio     │  │
│  │ PazReader│←→│ 24 cmds  │→ │  Graph    │  │ BGM/SE/Voice│ │
│  │ mmap/pread│  │ 60Hz     │  │ layers    │  │ lip sync   │  │
│  └──────────┘  └──────────┘  └───────────┘  └────────────┘  │
│        │            │              │              │          │
│  ┌──────────┐  ┌──────────┐  ┌───────────┐  ┌────────────┐  │
│  │ Decoder  │  │ Save/Load│  │ Renderer  │  │  Video     │  │
│  │ PNG/OGG  │  │  JSON    │  │ batching  │  │  FFmpeg    │  │
│  │ ANI      │  │          │  │ shaders   │  │  AVI demux │  │
│  └──────────┘  └──────────┘  └───────────┘  └────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

### 4.4 VFS：PAZ 直读

**iOS 内存红线**：禁止整文件 `mmap` 3GB。索引只有几百 KB，常驻内存；
数据按需 `pread` 分块解密。

```cpp
struct PazEntry { std::string name; uint64_t off; uint32_t usize, size, aligned; bool packed; };

class PazReader {
  int fd_;
  std::vector<PazEntry> entries_;
  std::unordered_map<std::string,int> index_;
  Blowfish bf_data_;            // TBL2[slot]
  std::array<uint8_t,32> tbl1_, tbl2_;
  bool is_video_, is_audio_;
  std::array<uint8_t,256> video_key_;

public:
  // 打开时只做：读 0x24 头 → 解密索引 → 解析条目
  bool Open(const char* path, int slot, const uint8_t* tbl1, const uint8_t* tbl2);

  // 按需解密；小文件一次读完，大文件流式
  std::unique_ptr<InputStream> OpenEntry(const PazEntry& e);
};
```

流式解密（大文件）：

```
block_size = 1 MiB（必须是 Blowfish 8 字节块的整数倍）
for each block:
    read(off + i, block)
    blowfish_decrypt_inplace(block, tbl2_key)
    if has_rc4: xor_rc4(block, rc4_state)   // 维护跨块的 RC4 状态
```

> Blowfish ECB 可以按 8 字节块独立解密，天然支持随机访问；
> RC4 必须维护 `i/j/S[256]` 状态跨块连续。mov 的 64K 块 XOR 也按块边界处理。

多卷：打开 `<name>.paz` 后依次探测 `<name>.paz0`..`<name>.paz9`，
把各卷的 `[base, base+size)` 映射到统一虚拟地址空间。

### 4.5 Script VM

**解析**：

```cpp
struct Token { std::string_view text; };
// 按行读取（CRLF），先按档案编码解码为 UTF-8，再识别 ';' 注释
// 命令名以 '.' 开头；'.message' 保留自由文本尾部
```

**命令对象**：24 个 `Command` 子类，统一接口：

```cpp
struct Command {
  virtual bool Parse(const std::vector<Token>&) = 0;
  virtual ExecResult Execute(ScriptContext&) = 0;  // Done / Wait / Jump
};
```

**调度器**：固定 60Hz。

```cpp
void ScriptVM::Tick() {
  while (state_ == Running) {
    if (wait_frames_ > 0) { --wait_frames_; return; }
    if (wait_bgm_ || wait_anim_ || wait_timer_) return;
    Command* c = Fetch();
    auto r = c->Execute(ctx_);
    if (r == Wait) return;
    if (r == Jump) pc_ = label_pc_[target_];
  }
}
```

**等待状态**（与引擎一致）：

| 状态 | 触发 | 结束条件 |
|---|---|---|
| `WAIT` (5) | `.wait` | 帧计数到 0 |
| `WAIT_BGM` | `.waitBGM` | BGM 播放结束 |
| `WAIT_ANIM` | `.waitAnimation` | `.ani` 播放完 |
| `WAIT_TIMER` | `.waitTimer` | 计时器到点 |
| `WAIT_CLICK` | `.message` 的 `\a` | 玩家点击/触摸 |

**变量**：`std::unordered_map<std::string, Value>`，`Value = int | bool | string`。
`.set` 写局部，`.setGlobal` 写全局；`.if` 只比较整数。

**脚本切换**：`.chain` 压栈新脚本；`.end` 弹栈；`.goto/.label` 在文件内跳转。

**存档快照**：

```json
{
  "script": "B003.sc",
  "pc": 1234,
  "vars": {"eden_D001": 1, "EYE_SEQ_DEFAULT": 2},
  "stage": {"image": "ev007a01.png", "x": 0, "y": 884, "mask": null},
  "view": {"x": 0, "y": 884},
  "bgm": {"file": "edenBGM016.ogg", "pos_ms": 32100},
  "voice": null,
  "panel": 1,
  "timestamp": 1700000000,
  "thumbnail": "base64..."
}
```

### 4.6 场景图与渲染

**虚拟画布**：1024×640，整数倍缩放 + letterbox（iPhone 刘海屏安全区）。

**图层**：

```cpp
struct Layer {
  Texture* tex;
  int x, y;                 // stage space
  int z;
  BlendMode blend;          // Normal / Add / Multiply / Screen
  float alpha;
  MaskRef mask;             // 可选灰度规则图
  AnimOverlay* ani;         // 可选 .ani 差分
};
```

**批处理**：所有 quad 合并为一次 draw call；纹理按 LRU 池化，硬上限 256MB。
`.stage` 切换时立即回收旧层纹理；长图按 1024×640 切片上传，最多保留
可见的 3 片。

**转场 shader**（对齐约束文档的公式）：

```glsl
// mask transition
float rule = texture(ruleMap, uv).r;
float a = clamp((rule - (1.0 - progress)) * softness + 0.5, 0.0, 1.0);
color = mix(oldColor, newColor, a);
```

**其它 shader**：

* 交叉淡入：`mix(old, new, progress)`。
* HSV 滤镜：RGB→HSV→调整→RGB。
* 震屏：在顶点着色器加 `offset = amplitude * noise(time)`。
* 萤火虫/雪/雨：GPU 粒子（点精灵 + 预计算轨迹）。
* 视频：YUV420P → RGB（`yuv2rgb` 矩阵），叠加到舞台。

**口型合成**：

```cpp
// .ani 每帧是 BGRA 小块，按 origin 贴到同名 PNG 上
for (frame : ani.frames) {
  dst = base_png.subrect(frame.origin_x, frame.origin_y, frame.w, frame.h);
  blend_bgra(dst, frame.pixels);   // 直接覆盖（差分帧自带 alpha）
}
```

口型帧选择：`LipAnimater` 根据语音 RMS 包络在 `frame_count` 中选索引；
静音时回到闭口帧。

### 4.7 音频

* 解码：`stb_vorbis` 流式解码 OGG → 48kHz float32 立体声。
* 混音：`miniaudio` 回调，BGM/SE/Voice 三条总线，支持音量/声像/循环。
* 语音同步：在混音回调里计算 voice 总线的 RMS：

```cpp
float rms = sqrt(sum(sample^2) / n);
lip_level = smooth(rms, 0.85f);
```

* `.playSE` 的 `[n]` 是百分比音量；`pan` 映射到等功率声像。
* 循环点：OGG 的 `LoopStart/LoopLength` 注释（若有）；否则整段循环。

### 4.8 视频

* AVI demux：自己解析 RIFF（`hdrl`/`movi`/`idx1`），或直接用 FFmpeg 的 AVI demuxer。
* 解码：FFmpeg `vc1`（WMV3）+ `mpeg4`（XviD）+ `mp3`/`pcm_s16le`。
* 输出：YUV420P → Metal/GLES 双纹理，GPU 做 YUV→RGB。
* 同步：以音频时钟为主，视频帧按 PTS 丢弃/重复。
* `.movie` 的 `width/height` 是目标尺寸，解码后缩放到该尺寸再叠加。

### 4.9 文本与字体

* 编码：main 用 GBK，PM 用 ASCII；统一转 UTF-8。
* 字体：拉丁字符用 Merriweather（随包），中文用系统字体（iOS: PingFang SC；
  Android: Noto Sans CJK）；或内置一份子集字体保证跨端一致。
* 排版：按 1024×640 的逻辑坐标排，字号/行高从 `MessagePanel` 的常量表读取
  （需在实现时用截图校准）。
* 打字机效果：每帧推进 N 个字符，`\n` 立即换行，`\v` 等待语音，`\a` 等待点击。

### 4.10 UI / 存档 / 回想

* 主菜单：开始、继续、CG 鉴赏、音乐鉴赏、设置、退出。
* 存档：9 页 × 20 槽 + 快速存档；缩略图 256×160。
* 回想：按 `.message` 的 id 建索引，支持语音重播。
* CG 图鉴：扫描 `bg.paz` 里的 `ev*.png` 与 `H*.png`。
* 音乐鉴赏：扫描 `bgm.paz` 的 OGG 标签。
* 设置：文字速度、自动播放间隔、BGM/SE/Voice 音量、全屏/窗口、语言（中/英）。

### 4.11 iOS 专项

| 问题 | 方案 |
|---|---|
| 内存 1.2GB 上限 | 纹理 LRU 256MB；长图切片；`.ani` 按需解码；不整包 mmap |
| 沙盒资源 | `UIFileSharingEnabled` + `LSSupportsOpeningDocumentsInPlace`；用户放 `.paz` 到 `Documents/eden/` |
| 高刷 | 逻辑固定 60Hz，渲染 120Hz 用 `presentationTime` 插值（可选关闭） |
| 触摸 | 点击=推进；滑动=隐藏 UI；双指=菜单；安全区 letterbox |
| 后台 | 暂停音频、保存自动存档、恢复时重建音频设备 |
| 签名 | TrollStore / ldid 自签；不依赖 App Store 审核 |
| 视频 | FFmpeg 静态链接（LGPL，动态链接或提供对象文件以合规） |

---

## 五、分阶段路线图与验收标准

### 5.1 阶段划分

| 阶段 | 内容 | 产出 |
|---|---|---|
| **P0 资源管线** | `.paz` 解包器、密钥表、`.ani` 解析、`.sc` 统计 | `tools/paz_extract.py`、`reports/RE_FINDINGS.md`（**已完成**） |
| **P1 VFS + 解码** | C++ PazReader（流式/mmap 双后端）、PNG/OGG 解码、纹理池 | 能在桌面窗口显示任意 `bg.paz` 图片 |
| **P2 Script VM** | 24 条命令、60Hz 调度、变量/标签/chain、存档 | 能从头跑到尾，日志与引擎 trace 对齐 |
| **P3 演出还原** | `.stage` 长图/滚动、`.transition` 遮罩、`.panel`、`.movie` | 关键场景逐帧对比通过 |
| **P4 音频/口型** | BGM/SE/Voice、`.ani` 口型、RMS 同步 | 语音与口型同步误差 < 1 帧 |
| **P5 UI/存档** | 主菜单、存读档、回想、CG/音乐鉴赏、设置 | 完整可通关 |
| **P6 iOS** | UIKit/Metal 后端、AudioUnit、触摸、TrollStore 打包 | 真机 60fps，内存 < 1.2GB |
| **P7 校准** | 差分截图、音频对齐、边界场景 | 验收报告 |

### 5.2 验收标准（硬指标）

1. **脚本覆盖**：58 个 `.sc` 全部可解析；24/24 命令实现；
   未知命令 0 次（当前数据里本来就没有）。
2. **执行轨迹**：对每个脚本，记录 `(pc, cmd, args)` 序列，与引擎
   （Unicorn 插桩或人工标注）逐条比对，差异 = 0。
3. **像素对比**：选取 200 个关键帧（转场中、滚动中、口型中、视频中），
   与原版 1024×640 截图做 diff：
   * 平均绝对误差 < 1/255；
   * 99.5% 像素误差 ≤ 2/255；
   * 转场 alpha 误差 ≤ 1/255。
4. **时序**：`.wait 180` 在 60Hz 下误差 ≤ 1 帧；`.transition 30` 时长误差 ≤ 1 帧。
5. **音频**：BGM/SE 触发点与引擎日志误差 ≤ 5ms；语音时长一致；
   口型帧与语音 RMS 包络相关系数 > 0.9。
6. **视频**：4 个 AVI 全部可播；首帧/末帧 PTS 误差 ≤ 1 帧；音画同步 ≤ 20ms。
7. **存档**：存/读 100 次往返，状态完全一致；跨版本（main/PM）不串档。
8. **iOS**：iPhone 8 / iPhone 12 真机 60fps；峰值内存 < 1.2GB；
   冷启动 < 3s；后台恢复 < 500ms。

### 5.3 差分验证工具链

```
tools/
  paz_extract.py          # 解包（已完成）
  sc_stats.py             # 命令普查（已完成）
  ani_dump.py             # .ani → PNG 序列（待写）
  trace_engine.py         # Unicorn 插桩，输出 (pc, cmd, args)（待写）
  frame_diff.py           # 截图 diff（待写）
```

`frame_diff.py` 的核心：

```python
def diff(a, b):
    d = np.abs(a.astype(np.int16) - b.astype(np.int16))
    return d.mean(), (d <= 2).mean(), d.max()
```

### 5.4 需要"截图校准"的项（诚实清单）

以下项的反汇编只能确定**数据流**，具体曲线/常量需要在原版上采样后拟合：

1. `.transition type 0` 的 alpha 曲线（线性？smoothstep？）——用 30 帧转场
   逐帧截图拟合。
2. `.transition type 1` 的 mask softness 常量。
3. `.shakeScreen` 的噪声函数与衰减曲线。
4. `.effect Firefly` 的粒子生命周期/速度分布。
5. 文字排版的行高、字距、标点挤压规则。
6. `.ani` 口型帧的切换阈值（RMS → frame index）。

这些不影响架构，但决定"100%"的最后一公里。计划在 P7 用差分工具逐项锁定。

### 5.5 风险与对策

| 风险 | 影响 | 对策 |
|---|---|---|
| WMV3/VC-1 解码 | 视频无法播放 | FFmpeg 静态链接；提前在 iOS 上验证 |
| 3GB `bg.paz` 内存 | Jetsam 闪杀 | 流式 pread + 纹理 LRU；禁止整包 mmap |
| GBK/CP932 混排 | 文本乱码 | 按档案 id 选编码；先解码再解析控制码 |
| 120Hz 高刷 | 时序漂移 | 逻辑固定 60Hz，渲染插值可关 |
| 字体缺失 | 排版差异 | 内置字体子集 + 系统字体 fallback |
| LGPL 合规 | 上架风险 | 动态链接 FFmpeg 或提供对象文件；TrollStore 分发不受影响 |

---

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
# 解包脚本
.venv/bin/python tools/paz_extract.py samples/main/eden.exe samples/main/scr.paz extracted/main_scr
.venv/bin/python tools/paz_extract.py samples/pm/eden_pm_en.exe samples/pm/scr.paz extracted/pm_scr

# 命令统计
.venv/bin/python tools/sc_stats.py

# 全档案验证（示例）
.venv/bin/python - <<'PY'
import sys; sys.path.insert(0,'tools')
import paz_extract as P
a=P.Archive('samples/pm/eden_pm_en.exe','/mnt/f/gal/【PC硬盘】【官中】eden/eden PLUS MOSAIC/sys.paz')
from collections import Counter
c=Counter(P.sniff(a.decrypt(e)) for e in a.entries)
print(len(a.entries), c)
PY
```

---

**一句话总结**：eden* 的"演出"= 1024×640 虚拟舞台 + 纵向长图视口滚动 + 遮罩
转场 + `.ani` 口型差分 + 固定 60Hz 帧语义 + WMV3/Xvid 视频。ONS 表达不了这些，
所以正确路线是 **Native Musica Runtime 直读原版 `.paz`**；格式与密钥已全部攻破，
解包器已实测零失败，剩下的工作是把 24 条命令、4 层图层、3 类转场和口型同步
按 60Hz 逐帧复刻，并用差分截图把最后 1% 的曲线校准到位。
