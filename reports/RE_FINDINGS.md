# eden* / eden* PLUS+MOSAIC — 逆向实证报告

所有结论均由本仓库 `tools/` 下的脚本在真实样本上跑通，或由 `objdump -d` 反汇编 +
Unicorn 2.1.4 单步执行验证。禁止把本文件当成"推测"引用——每条都标了验证方式。

样本：

| 文件 | 大小 | 说明 |
|---|---|---|
| `samples/main/eden.exe` | 2,553,688 | 本篇（官中，脚本为 GBK 中文） |
| `samples/main/scr.paz` | — | 23 个 `.sc` |
| `samples/pm/eden_pm_en.exe` | 2,267,136 | PLUS+MOSAIC（官方英文，脚本为 ASCII 英文） |
| `samples/pm/scr.paz` | — | 35 个 `.sc` |
| `samples/pm/z` | 1,372,288 | 见 §7 |

---

## 1. 二进制身份

* PE32 / x86 / MSVC（RTTI + `VCRUNTIME140` + EH cookie），无壳、无 VMProtect。
* `eden.exe` 链接时间戳 `0x54B7712C` = 2015-01-14。
* 引擎：minori **Musica**（RTTI 中可见 `ScriptEngine`、`CLayerManager`、`Command*`、
  `CMMV`、`CVorbisPlayer`、`BlowfishEncryptionKey`）。
* 图形：`d3d9.dll` + `DDRAW`（D3D9 设备 + DirectDraw 表面混用），字体走 GDI。
* 视频：DirectShow BaseClasses（`CMMV` / `MMVMotion` / `MMVres`），MPEG/AVI。
* 音频：`Xiph.Org libVorbis 1.3.2`（`CVorbisPlayer`）+ DirectSound。

### PE 节表（实测）

| 节 | VA | VSIZE | RAW OFF | RAW SIZE |
|---|---|---|---|---|
| main `.text` | 0x401000 | — | 0x1000 | 0x146200 |
| main `.rdata` | 0x548000 | 0x4b84c | 0x148000 | 0x4ba00 |
| main `.data` | 0x594000 | 0x3c9f0 | 0x194000 | 0x5200 |
| main `.rsrc` | 0x5d1000 | — | 0x1d1000 | 0x90400 |
| main `.module` | 0x662000 | 0xb000 | 0x262000 | 0xb000 |
| PM `.text` | 0x401000 | 0x147922 | 0x400 | 0x147a00 |
| PM `.rdata` | 0x549000 | 0x4baae | 0x147e00 | 0x4bc00 |
| PM `.data` | 0x595000 | 0x3ceb0 | 0x193a00 | 0x5400 |
| PM `.rsrc` | 0x5d2000 | — | 0x198e00 | 0x90a00 |

* Blowfish 常量 `0x243F6A88` 全 exe 仅一处：main 文件偏移 `0x1642B8`，PM `0x1640C8`。
  ⇒ 只有一套标准 S/P-Box，没有按游戏改表。

---

## 2. `.paz` 容器格式

```
+0x00  char id[0x20]     "eden_en" / "eden_PlusMosaic_en"（后面是填充垃圾）
+0x20  u32  idx_len      索引密文长度，恒为 8 的倍数
+0x24  u8   index[idx_len]
+0x24+idx_len          数据区
```

索引明文（解密后）：

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

实测：`unpacked_size == size` 恒成立（全部档案 579/579 等），
`aligned_size - size ∈ [0,7]`，`is_packed` 仅在 `dummy.txt` 上为 1。

### 2.1 索引解密（已验证）

```
key[i] = (256 - TBL1[slot*0x20 + i]) & 0xFF      i = 0..31
plain  = Blowfish_ECB_decrypt(cipher, key)       // 小端 32-bit 字序
```

* `TBL1` 是 10 个槽 × 32 字节的表。
* `slot` 由档案名决定，见 §2.3。

### 2.2 数据解密（已验证）

```
stage1 = Blowfish_ECB_decrypt(raw[offset : offset+aligned_size],
                              key = (256 - TBL2[slot*0x20 + i]) & 0xFF)
if ext ∈ {png, ogg, sc, avi} or is_audio:
    key_str = cp932( name.lower() + " " + "%08X" % unpacked_size + " " + TypeKey[ext] )
    plain   = stage1 XOR RC4_keystream(key_str, len(stage1))
else:
    plain   = stage1                      // .wav/.txt/.psd/.ani 没有 RC4 层
plain = plain[:size]
if is_packed: plain = zlib.inflate(plain)
```

**关键点**：RC4 从 offset 0 开始整段 XOR，不是按 64K 分块、不做 CRC32 skip
（eden 的 scheme Version < 2，GARbro 的 `crc>>12` skip 逻辑不适用）。

### 2.3 槽位与 TypeKey（两版不同，已实测）

槽名表 PM `0x5986F8` / main `0x597xxx`，10 项：
`scr(0) bg(1) bg(2→1) st(3) sys(4) bgm(5) voice(6) se(7) se(8→7) mov(9)`
（`i==2 || i==8` 时 `slot = i-1`）。

TypeKey 表：4 个连续 8 字符字面量，按 `{sc, ogg, png, avi}` 顺序
（与 GARbro `GetTypePassword` 的分派顺序一致）。

| 类型 | PM 地址/值 | main 地址/值 |
|---|---|---|
| `.sc`  | `0x56BCDC` `N426Fd94` | `0x56AC64` `63JwEajj` |
| `.ogg` | `0x56BCE4` `ol0lOrAf` | `0x56AC6C` `wCCuLx5S` |
| `.png` | `0x56BCEC` `p37j344s` | `0x56AC74` `J1WaoGBi` |
| `.avi` | `0x56BCF4` `Uyiu4Ruy` | `0x56AC7C` `F73xvHdo` |

`TBL1/TBL2` 基址：

| | TBL1 | TBL2 |
|---|---|---|
| main | `0x597478` | `0x5975B8` |
| PM | `0x598478` | `0x5985B8` |

### 2.4 初始化链（PM，`0x4B94ED`，逐指令核对）

1. `0x4B94F2 call 0x522440`：为 10 个 reader3 重建索引解密器。
2. `0x4B94F9..0x4B9517`：把 4 个 TypeKey 指针写进 `0x5CB79C/A0/A4/A8`。
3. 循环 `i=0..9`：`slot = i-1 if i in (2,8) else i`；
   `TBL1 + slot*0x20` → `0x5CEA18` 对象，`TBL2 + slot*0x20` → `0x5CEA90`。
4. `0x4B9721 call 0x523960`（Open）：先用空后缀打开，再依次尝试 `<name>0` … `<name>9`
   ⇒ `.paz0`..`.paz9` 是分卷后缀。

### 2.5 Blowfish 密钥对象（`0x4BB580` 取负）

```
BlowfishEncryptionKey  vtable = 0x56BB18 (PM) / 0x56BB18 (main)
  +0x00 vtable
  +0x04 u32 keyLen = 0x20
  +0x08 bufferDesc* { flags, bits, ptr, own }
vtable[3] = 0x4BB580 : return -ptr[i]     // 尾部 f6 da = neg dl
```

### 2.6 `mov.paz`（完全不同的 scheme，已实测出 RIFF/AVI）

索引明文在 `count` 之后多 0x100 字节 `video_key`：

```
count u32
u8 video_key[0x100]
entries...
```

数据区**不做 Blowfish**：

```
entry_key = cp932( name.lower() + " %08X" % unpacked_size + " " + TypeKey["avi"] )
k[i]      = video_key[i] ^ entry_key[i % len(entry_key)]      i = 0..0xFF
block     = RC4(k).keystream(0x10000)
plain     = cipher XOR block (按 0x10000 循环)
```

实测结果：

| 档案 | 条目 | 解密后前 12 字节 |
|---|---|---|
| main mov.paz | op.avi | `52 49 46 46 .. .. .. .. 41 56 49 20` (`RIFF....AVI `) |
| main mov.paz | ed.avi | `RIFF....AVI ` |
| PM mov.paz | op.avi | `RIFF....AVI ` |
| PM mov.paz | ed.avi | `RIFF....AVI ` |

---

## 3. 实测解包结果

`tools/paz_extract.py`（完整三级链 + mov 分支 + mmap）实测：

| 档案 | 条目数 | 解密结果 |
|---|---|---|
| main `scr.paz` | 23 | 23 × `.sc` 全部可读 |
| PM `scr.paz` | 35 | 35 × `.sc` 全部可读 |
| main `sys.paz` | 515 | 509 PNG + 5 WAV + 1 PSD（8BPS），0 失败 |
| PM `sys.paz` | 579 | 573 PNG + 5 WAV + 1 PSD，0 失败 |
| main `bg.paz` | 4,127 | 2,407 PNG + 1,720 ANI |
| PM `bg.paz` | 6,331 | 3,575 PNG + 2,756 ANI |
| main `se.paz` | 105 | 105 OGG |
| PM `se.paz` | 122 | 122 OGG |
| main `bgm.paz` | 38 | 38 OGG |
| PM `bgm.paz` | 42 | 42 OGG |
| main `voice.paz` | 5,056 | 5,056 OGG（无扩展名） |
| PM `voice.paz` | 7,920 | 7,920 OGG |
| main `mov.paz` | 2 | 2 AVI |
| PM `mov.paz` | 2 | 2 AVI |
| `st.paz` | 1 | `dummy.txt`（zlib，256B → 264B 的 'a'） |

文件头验证样例：

```
PNG  89 50 4e 47 0d 0a 1a 0a   IHDR 1024x640 / 8-bit
OGG  4f 67 67 53 00 02 ...     vorbis
WAV  52 49 46 46 ... 57 41 56 45  fmt PCM 1ch 44100
AVI  52 49 46 46 ... 41 56 49 20  LIST
PSD  38 42 50 53 00 01 ...     8BPS
```

---

## 4. `.sc` 脚本格式

* 纯文本，CRLF 分隔；**注释符是 `;`**（不是 `#`）。
* 命令以 `.` 开头，参数空白分隔；`.message` 的文本尾部是自由文本（可含空格）。
* 编码：main = **GBK**（官中），PM = **ASCII/英文**（官方英文版）。
* 文本里的 `\` 既可能是控制码，也可能是 **GBK 尾字节 0x5C**，必须先按 GBK 解码再解析控制码。

### 4.1 命令表（PM 注册函数 `0x4C32D4`–`0x4C508D`，47 条）

字符串表位于 PM `.rdata` `0x56B448` 起。每个命令 = `new` + 构造 + `map<string,Command*>::insert`。

| 命令 | vtable | Execute | Parse | 类（RTTI） |
|---|---|---|---|---|
| `;` | 0x569164 | 0x4E7950 | 0x4E7B70 | Comment |
| `.label` | 0x56984C | 0x4E4760 | 0x4E49A0 | — |
| `.select` | 0x56A3E4 | 0x4DF820 | 0x4DF510 | CommandSelect |
| `.panel` | 0x569B2C | 0x4E3010 | 0x4E3330 | CommandPanel |
| `.movie` | 0x56994C | 0x4E4270 | 0x4E4230 | CommandMovie |
| `.movie2` | 0x5699EC | 0x4E3D20 | 0x4E3CE0 | — |
| `.transition` | 0x56A87C | 0x4DC8F0 | 0x4DC880 | CommandTransition |
| `.stage` | 0x56A7C4 | 0x4DDA10 | 0x4DD9A0 | CommandStage |
| `.setFG` | 0x56A4E4 | 0x4DED20 | 0x4DEB30 | — |
| `.char` | 0x5690EC | 0x4E87A0 | 0x4E8760 | CommandChar |
| `.animation` | 0x568F14 | 0x4E97D0 | 0x4E9750 | CommandAnimation |
| `.effect` | 0x569364 | 0x4E6E10 | 0x4E6D90 | CommandEffect |
| `.message` | 0x5698C4 | 0x4E4700 | 0x4E46C0 | CommandMessage |
| `.HSVfilter` | 0x5696BC | 0x4E5480 | 0x4E58C0 | CommandHSV |
| `.overlay` | 0x569AA4 | 0x4E3710 | 0x4E3680 | CommandOverlay |
| `.screen` | 0x56A034 | 0x4E0D10 | 0x4E0C80 | — |
| `.vscroll` | 0x56A0BC | 0x4DF920 | 0x4E0940 | — |
| `.hscroll` | 0x56A13C | 0x4DF8F0 | 0x4E08E0 | — |
| `.scroll` | 0x56A1BC | 0x4DF8C0 | 0x4E0880 | CommandScroll |
| `.scrollFG` | 0x56A2CC | 0x4DF880 | 0x4E07C0 | — |
| `.scrollAcc` | 0x56A354 | 0x4DF850 | 0x4E0760 | — |
| `.rotateBG` | 0x569EF4 | 0x4E0E60 | 0x4E1BD0 | CommandRotateBG |
| `.rotateFG` | 0x569F94 | 0x4E0D50 | 0x4E1B90 | — |
| `.frame` | 0x5695CC | 0x4E5EC0 | 0x4E5E50 | — |
| `.cutin` | 0x5691F4 | 0x4E78A0 | 0x4E7830 | CommandCutin |
| `.wscroll` | 0x56A24C | 0x4DF940 | 0x4E0820 | — |
| `.endScroll` | 0x569474 | 0x4E6560 | 0x4E6750 | — |
| `.shakeScreen` | 0x56A6BC | 0x4DE050 | 0x4DE220 | CommandShakeScreen |
| `.set` | 0x56A474 | 0x4DF020 | 0x4DF2F0 | CommandSet |
| `.setGlobal` | 0x56A55C | 0x4DE750 | 0x4DE8C0 | — |
| `.deleteVar` | 0x5692EC | 0x4E6F10 | 0x4E7110 | — |
| `.goto` | 0x569644 | 0x4E5940 | 0x4E5BC0 | — |
| `.if` | 0x56975C | 0x4E4E30 | 0x4E5420 | CommandIf |
| `.playVoice` | 0x569D64 | 0x4E2150 | 0x4E2350 | — |
| `.playBGM` | 0x569BB4 | 0x4E2DB0 | 0x4E2F90 | — |
| `.playSE` | 0x569C44 | 0x4E2970 | 0x4E2B50 | — |
| `.playSE2` | 0x569CD4 | 0x4E2570 | 0x4E2750 | — |
| `.wait` | 0x56A904 | 0x4DC380 | 0x4DC550 | CommandWait |
| `.setTimer` | 0x56A5CC | 0x4DE2D0 | 0x4DE6E0 | — |
| `.waitTimer` | 0x56A644 | 0x4DE2B0 | 0x4DE680 | — |
| `.waitAnimation` | 0x56A97C | 0x4DC090 | 0x4DC300 | — |
| `.waitBGM` | 0x56A9FC | 0x4DBE40 | 0x4DC010 | — |
| `.pragma` | 0x569E64 | 0x4E1C20 | 0x4E2070 | — |
| `.exec` | 0x5694EC | 0x4E6510 | 0x4E6490 | — |
| `.chain` | 0x56900C | 0x4E8D70 | 0x4E8D00 | — |
| `.end` | 0x569404 | 0x4E6910 | 0x4E68B0 | — |
| `.debug` | 0x569274 | 0x4E7190 | 0x4E73B0 | — |
| `#include` | 0x5697D4 | 0x4E4A30 | 0x4E4C50 | — |

> vtable 槽位固定：`[0]` scalar deleting dtor、`[1]` dtor、`[2]` **Execute**、
> `[4]` **Parse**（`0x4DD9A0` 等）。

### 4.2 实际使用频次（main + PM，40,397 行）

| 命令 | 次数 | 参数形态（实测） |
|---|---|---|
| `.message` | 22,048 | `<id> [voice] [#]<text>` |
| `.stage` | 7,730 | `<mask\|*> <png> <x> <y>`（7,727 次 mask=`*`） |
| `.transition` | 7,729 | `<type> <layer\|*> <frames>`（type 0 = 7,721 次） |
| `.panel` | 948 | `<0\|1\|2>`（485/449/14） |
| `.playBGM` | 552 | `<file>` / `* * <n>` |
| `.playSE` | 430 | `<file>[<vol>] <t\|f> <pan> <loop>` |
| `.wait` | 351 | `<frames>`（60fps） |
| `.pragma` | 142 | `disable_control` / `enable_control` |
| `.label` | 78 | `<name>` |
| `.if` | 74 | `<var> <op> <value> <label>` |
| `.endScroll` | 52 | `<t\|f>` |
| `.chain` | 42 | `<file>.sc` |
| `.playSE2` | 38 | 同 `.playSE` |
| `.setGlobal` | 30 | `<var> = <value>` |
| `.vscroll` | 25 | `<target_y> <speed_px/frame>` |
| `.end` | 22 | — |
| `.set` | 22 | `<var> = <value>` |
| `.scroll` | 18 | `<target_x> <target_y> <frames>` |
| `.playVoice` | 18 | `*` / `* * <n>` |
| `.movie` | 11 | `<id> <avi> <w> <h> <t\|f>`（1024 640） |
| `.hscroll` | 9 | `<target_x> <speed>` |
| `.goto` | 8 | `<label>` |
| `.shakeScreen` | 8 | `<R\|Z> <amp> <frames>` |
| `.effect` | 4 | `Firefly Firefly 30 6000` / `end` |

**结论：eden 实际只用了 24 条命令**，`.char/.animation/.overlay/.screen/.setFG/.cutin/
.HSVfilter/.rotateBG/.frame/.select/.movie2/.exec/.debug/#include/.setTimer/.waitTimer/
.waitAnimation/.waitBGM/.deleteVar` 全部为 0 次。

### 4.3 文本控制码

* `\n` 5,309 次：硬换行。
* `\v` 132 次、`\a` 138 次：绝大多数出现在句尾 `\v\a`（语音同步点 + 等待点击）。
* GBK 尾字节 0x5C 会被误认成 `\`（97 次）⇒ 必须先 GBK 解码。

### 4.4 分辨率 / 帧率 / 舞台

* 画面 **1024×640**（`.movie ... 1024 640`，所有 PNG IHDR 均为 1024×640）。
* 帧率 **60fps**：`.wait 180` = 3 秒；`.transition` 30 帧 = 0.5 秒。
* 舞台是 **1024 宽、可纵向滚动**的虚拟画布：
  * `ev006.png` = 1024×1920，`.stage * ev006.png 0 1280` + `.vscroll 970 -7`
  * `ev007a03.png` = 1024×1696（带 alpha，ctype 6），`.stage ... 0 0` + `.vscroll 884 10`
  * `ev162a01.png` = 1024×2007
* `.stage` 的 x/y 是图片左上角在舞台空间中的位置；`.vscroll` 移动的是**视口**。

### 4.5 `.ani`（差分动画层，1720/2756 个）

**每一个 `.ani` 都有同名 `.png`**（main 1720/1720 配对成功）。格式：

```
u16 version = 0x0100
u16 frame_count
u32 reserved = 0
repeat frame_count:
    char label[]\0          // CP932，例如 "シオン 開き口"（希昂 张嘴）
    u16 width
    u16 height
    u16 bpp = 32
    u16 origin_x
    u16 origin_y
    u8  pixels[width*height*4]   // BGRA
```

* 250 个随机样本，解析后 `pos == filesize` **250/250**，零误差。
* frame_count 分布：2/4/6/8/10/12/16 帧。
* 标签实测：`シオン 開き口`(50)、`真夜 開き口`(33)、`エリカ 開き口`(14)、
  `ラヴィ 開き口`(12)、`杏 開き口`(9)、`ナツメ 開き口`(1)。
* ⇒ `.ani` = **口型/眨眼差分贴图**，按 origin 叠加到同名 PNG 上。
* `.ani` 不在 TypeKey 表里 ⇒ 只做 Blowfish，**没有 RC4 层**（实测直接得到上述结构）。

### 4.6 滚动/转场的实际数学

`CommandTransition::Execute`（PM `0x4DC8F0`）：

```
scene->[0x24C] = type          // this+0x44
scene->[0x250] = layer_name    // this+0x48, std::string
scene->[0x26C] = duration      // this+0x64
if (layer_name.length() > 1)  call 0x42B0C0(layer_name)
else                          call 0x5165F0()   // 启动转场
```

`.vscroll` → `0x4CF0F0(engine, a, b)`：

```
scene->[0x1E58] = scene->[0x1DD8]   // 起始 y = 当前舞台 y
scene->[0x1E5C] = a                 // 目标 y
scene->[0x1E60] = b                 // 每帧速度（带符号）
scene->[0x1E44] = 1                 // mode = VSCROLL
```

`.hscroll` → mode 2；`.scroll` → `0x4CF010`，`mode = 3`，
`[0x1E48/0x1E4C]` 起始 x/y，`[0x1E50/0x1E54]` 目标 x/y，`[0x1E60]` 帧数。

`CommandStage::Execute`（PM `0x4DDA10`）把 9 个字段（`+0x60` 字符串、
`+0x7C` 字符串、`+0x98..0xCC` 整数/向量）传给 `0x4D7930`，后者写入
`SceneGame+0x270..0x29C` 的 2×2 仿射矩阵 + 平移，并设置 `+0x1DDC/+0x1DE0`。

`CommandWait::Execute`（PM `0x4DC380`）：

```
engine->[0x1DA8] = 5        // state = WAIT
*(0x59AF64) = this->[0x44]  // 倒计时帧数
```

---

## 5. 汉化机制

* **不是**内嵌脚本替换：脚本本身就在 `scr.paz` 里被替换成 GBK 中文。
* 外挂：`eden_en.chm`(1.42MB) / `eden_pm_en.chm`(1.42MB)、`eden_en.chw`、
  `names.txt`、`MerriweatherMG_FontInstaller.exe`、`fontinstall.vdf`。
* 字体：Merriweather（衬线英文字体）+ 系统 GDI 字体渲染中文。
* PM 的脚本是**英文**，main 的脚本是**中文 GBK** —— 两版不是同一套文本。

---

## 6. 音频/视频

* BGM/SE/Voice：Ogg Vorbis（`CVorbisPlayer`，libVorbis 1.3.2）。
* `voice.paz` 条目无扩展名（`sio-C013-0186` 等），仍按 ogg 处理（is_audio 规则）。
* 视频：2 个 AVI（op/ed），mov.paz 专用加密；播放走 DirectShow。
* `.playSE <file>[<vol>] <t|f> <pan> <loop>`：`[n]` 是音量百分比，`t/f` 是循环，
  第三个是声像（0 居中），第四个是保留位（`*`）。

---

## 7. `z` 文件的真身

`z`（1,372,288 字节）：

* 开头 GBK：`顶点\r\n\r\n...zhongdian\r\n...自购\r\n...顶点\r\n`
* 全文是 **GBK 成语词典文本**（"开诚布公"、"开源节流"…），夹杂
  `Www.qinkan.net]` 广告串。
* 21,776 个 CRLF、8,216 个 `A1A1`（全角空格）填充。
* 无 PAZ 头、无索引、无 Blowfish 特征。

⇒ **不是游戏资源**，是发布组塞的填充文本。引擎侧对应
`ArcFourDummyFileReader` / `ZArchiveFileReader1/2/3` 三个 dummy reader 类，
不会把它当封包解析。移植时可直接忽略。

---

## 8. 可复现命令

```bash
# 解包（全部档案）
.venv/bin/python tools/paz_extract.py samples/main/eden.exe samples/main/scr.paz extracted/main_scr
.venv/bin/python tools/paz_extract.py samples/pm/eden_pm_en.exe samples/pm/scr.paz extracted/pm_scr

# 命令统计
.venv/bin/python tools/sc_stats.py

# 任意档案（自动 mmap，支持 2.9GB）
.venv/bin/python tools/paz_extract.py samples/pm/eden_pm_en.exe \
    "/mnt/f/gal/【PC硬盘】【官中】eden/eden PLUS MOSAIC/bg.paz" extracted/pm_bg
```

`tools/` 下关键文件：

| 文件 | 作用 |
|---|---|
| `paz_extract.py` | 生产级解包器（索引/数据/mov/zlib/mmap） |
| `sc_stats.py` | `.sc` 命令与参数普查 |
| `emu.py` / `emu_rc4.py` / `emu_key.py` | Unicorn 单步验证引擎密钥构造 |
| `paz.py` / `blowfish.py` | 早期原型（已被 `paz_extract.py` 取代） |
