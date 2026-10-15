# `mod-app` 工作线台账 —— 任务单 `#83`：SFZ 的**非 PCM-WAV 样本**（FLAC／OGG）

- **台账类型**：勘察结论 / 判据清单 / 实测证据 / **需要集成者批的事项**（**不是规范**）
- **工作线**：`line/mod-app`（worktree `yeban/.worktrees/mod-app`，基线 main `9a939318`）
- **所有者目录**：`crates/yeban-app/**`（本台账是唯一新增的共享区文档）
- **规范来源**：`AGENTS.md` §5.6（根级共享文件由集成者独占）、§6（测量纪律）；
  `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §0.1 / §5.3（`[ARCH-TOP-002]` 控制侧 vs 实时侧）；
  上游接缝：`docs/ledger/app-cli-notes.md`、`docs/ledger/decode-core-notes.md`（§3 支持的格式清单）
- **上游交接**：`#79` 的边界第 ③ 条逐字 —— "非 PCM-WAV 样本（FLAC／OGG）会被 `read_plain_wav`
  判为读不到 ⇒ 按'缺样本'静音 ＋ 计数（如实、不假装支持）"。

> 本文件回答四个问题：**本仓有哪些现成的解码入口**、**接进 app 要付什么代价**、
> **本单做到哪一步（为什么不是全部）**、**每个读数与每次注入怎么来的**。

---

## 0. 一句话

改之前：`sample=` 指向 **FLAC／OGG** ⇒ 读不到 ⇒ 区域静音 ＋ 计数，但**报告里只有一句混在一起的
错误文案** —— "缺文件"与"格式不支持"在读数上**分不开**，而这正是"听起来像缺陷"的那一处
（真实音色库大量用 FLAC／OGG）。

改之后：**仍然不支持 FLAC／OGG**（⛔ 没有假装支持），但每一条读失败都带**可判别理由**：
`NotFound`（文件不存在）／`UnsupportedFormat`（**FLAC／OGG 落这一档**）／`Unreadable`
（文件在但读不动），并计入报告与 CLI 报告行的 `unreadable(not_found=…,unsupported=…,other=…)`。

---

## 1. 勘察结论：本仓现成的解码入口（`文件:行号`）

| 入口 | 位置 | 能读什么 | 产物 |
| :--- | :--- | :--- | :--- |
| `yeban_decode::decode_path` | `crates/yeban-decode/src/decode.rs:91` | **WAV ＋ FLAC ＋ OGG/Vorbis ＋ ADPCM ＋ APE** | `DecodedAsset` |
| `yeban_decode::decode_bytes` / `decode_reader` / `decode_source` | 同文件 `:107` / `:122` / `:152` | 同上（内存 / `Read+Seek` / 不可回退源） | `DecodedAsset` |
| `DecodedAsset::samples()` | `crates/yeban-decode/src/asset.rs:200` | — | `&[f32]`（**交织**） |
| `DecodedAsset::channels()` / `sample_rate()` | 同文件 `:182` / `:188` | — | `u16` / `u32` |
| `DecodeOptions`（含 `PcmBudget` 六道闸门） | `crates/yeban-decode/src/decode.rs:58` | — | 配置 |
| `yeban_render::wav::read_plain_wav` | `crates/yeban-render/src/wav.rs:239` | **只** PCM／float WAV（`hound`） | `(PcmFormat, PcmBuffer)` |
| `read_plain_wav_mono_f32` | 同文件 `:294` | 同上，折单声道 ＋ 归一化 | `(Vec<f32>, u32, String)` |

⚠ `yeban-decode` 的 `symphonia` feature 集（`crates/yeban-decode/Cargo.toml`）**显式**含
`wav` / `ogg` / `flac` / `pcm` / `adpcm` / `vorbis` / `ape` / `id3v1` / `id3v2`
⇒ **FLAC 与 OGG/Vorbis 都在**。⛔ 刻意不含 `mp3` / `aac` / `alac` / `isomp4` / `mkv`。

### 1.1 "用它解一份 FLAC／OGG 成 `Vec<f32>`"需要几步？

**一步调用 ＋ 一次拷贝**（**已有现成函数**，⛔ 不需要自己写解析）：

```rust
let asset = yeban_decode::decode_path(path, &DecodeOptions::default())?; // ① 容器解封装 ＋ 解码
let pcm: Vec<f32> = asset.samples().to_vec();                            // ② 交织 f32
let (channels, rate) = (asset.channels(), asset.sample_rate());           // ③ 形状事实
```

⚠ 与 app 现有 `read_plain_wav_mono_f32` 的**口径差**只有一处：后者按 `step_by(channels)` 取
**声道 0**，`DecodedAsset::samples()` 是**交织**多声道 ⇒ 接进 `read_sample_file` 时要显式折单声道
（⛔ 不能直接 `.to_vec()` 当成单声道，否则音高/时长都会错）。

---

## 2. 勘察结论二：`yeban-app` 能不能直接用？——**不能，需要集成者批**

- `yeban-app` 的 `-e normal` 依赖：`yeban-midi` / `slint` / `yeban-model` / `yeban-sfz` /
  `yeban-render` / `yeban-engine` / `yeban-diagnostics` / `serde_json`
  （可选：`yeban-midi-io` / `yeban-ui-test-port` / `yeban-mcp` / `yeban-ui-mcp`，**默认全关**）。
  ⇒ **没有** `yeban-decode`。
- 逐 crate 读清单：**全仓只有 `yeban-mcp` 依赖 `yeban-decode`**（台账记的那一条**成立**）。
- 根 `Cargo.toml:94` **已有** `yeban-decode = { path = "crates/yeban-decode" }`
  ⇒ 接线**不必动根 `Cargo.toml`**，但必须在 `crates/yeban-app/Cargo.toml` 加一条边
  ⇒ **`Cargo.lock` 必然变化**（`yeban-app` 的依赖列表）⇒ 那是集成者独占面（`AGENTS.md` §5.6）。

### 2.1 影响面（实测：零编译的 `cargo tree` 集合差）

**指标**：`cargo tree -p <crate> -e normal --locked --prefix none` 里**不同的包名**个数。

| 读数 | 值 |
| :--- | :--- |
| `yeban-app` 现有包数 | **288** |
| `yeban-decode` 现有包数 | **57** |
| 接线后**新增**的包数 | ⭐ **+22** |

新增的 22 个（逐包）：`yeban-decode`、`symphonia`、`symphonia-bundle-flac`、
`symphonia-codec-adpcm`、`symphonia-codec-pcm`、`symphonia-codec-vorbis`、`symphonia-common`、
`symphonia-core`、`symphonia-format-ogg`、`symphonia-format-riff`、`symphonia-metadata`、
`rubato`、`audioadapter`、`audioadapter-buffers`、`audioadapter-sample`、
`audio-codec-algorithms`、`num-complex`、`num-integer`、`regex-lite`、`windowfunctions`、
`extended`、`visibility`。

⚠ 其中 `symphonia` 属 `AGENTS.md` §5.2 点名的**重依赖** ⇒ 本机与 CI 的构建代价都会上升。

---

## 3. 做到哪一步：**乙**（停手报批），⛔ 不是甲

任务单的三选项：甲＝直接接线；乙＝需新依赖 ⇒ 停手报批 ＋ 只做"可判别化"；丙＝统一走 decode ⇒ 先报批。

**裁决：乙。** 理由：`yeban-app` 现在**没有** `yeban-decode` 这条边，接上它要改
`crates/yeban-app/Cargo.toml` ＋ `Cargo.lock`（集成者独占）＋ 往 `-e normal` 图里塞 **+22 个包**
（含 `symphonia` 重依赖）⇒ 落在"须报批"那一档。**乙**被任务单明确预授权，且它已经消除了
"听起来像缺陷"的**读数面**（用户能看出是"格式不支持"而不是"文件没了"）。

⛔ **丙不做**：没有理由动既有 WAV 路径（`BASELINE_WAV_READ` 那条逐位判据就是它的护栏）。

---

## 4. 判据清单（全部落在 `crates/yeban-app/tests/sfz_sample_formats.rs`）

| # | 判据 | 作用 | 状态 |
| :--- | :--- | :--- | :--- |
| ① | `the_wav_sample_path_is_bit_identical_to_the_pre_change_fingerprint` | **应绿的反向腿**：WAV 路径逐位不变 | ok |
| ② | `the_unreadable_reason_separates_missing_files_from_unsupported_formats` | **乙核心**：`.xyz`／FLAC 与"文件不存在"分得开 | ok |
| ③ | `the_export_report_line_carries_the_reason_counts` | 用户可见：CLI 报告行带可判别计数 | ok |

## 5. 实测读数

### 5.1 判据 ①：WAV 路径**逐位**指纹（反向对照）

**指标**：`RenderProjectOutput::samples`（交错 `f32`）的 FNV-1a 64 指纹。

| 时点 | 指纹 | `nonzero` | `peak` |
| :--- | :--- | :--- | :--- |
| **接线前**（HEAD `9a939318`，工作树干净） | `0x0df478fa15264e79` | 9598 | 0.109731831 |
| **本单改完** | `0x0df478fa15264e79` | 9598 | 0.109731831 |

⇒ **逐位相同**（该字面值已写进 `BASELINE_WAV_READ`，仅在测量平台 `aarch64/macos` 上断言）。

⚠ 为什么既有判据不够：`sfz_model_chain::the_model_chain_is_bit_identical_to_the_hand_built_plan`
的两条腿**都要经过**被本单改动的 `read_sample_file` ⇒ 它对"WAV 路径被改坏"**不独立**。
① 是独立见证。

### 5.2 判据 ②：五条区域的可判别读数（`decode_sfz_plan`）

`summary()` 逐字：

```text
path=<TMP>/reasons.sfz bytes=264 regions=5 samples=1 silent=4
missing=["blob.xyz", "broken.wav", "fake.flac", "nothere.wav"] unreadable=4
unreadable_kind(not_found=1,unsupported=2,other=1) plan_bytes=32331
```

逐条（`=>` 左边是解析后路径，右边是**理由**）：

| `sample=` | 盘上是什么 | 实得理由 |
| :--- | :--- | :--- |
| `present.wav` | 合法 24-bit PCM WAV | （**读成功**，`samples=1`） |
| `nothere.wav` | 不存在 | `文件不存在` |
| `blob.xyz` | 存在；任意文本 | `格式不受支持（本入口只读 PCM／float WAV；FLAC／OGG 等需 yeban-decode，app 未接线）` |
| `fake.flac` | 存在；`fLaC` 魔数 ＋ 垃圾体 | 同上 |
| `broken.wav` | 存在；`RIFF…WAVE` 头 ＋ 垃圾体 | `文件在、但读取/解析失败（截断／字段非法／样本越界／I/O）` |

底层 `hound` 原文（逐字保留在 `UnreadableSample::detail`）：
`blob.xyz` / `fake.flac` ⇒ `hound 失败: Ill-formed WAVE file: no RIFF tag found`；
`broken.wav` ⇒ `hound 失败: Failed to read enough bytes.`；
`nothere.wav` ⇒ `hound 失败: No such file or directory (os error 2)`。

⚠ **这张表就是"为什么必须分档"的证据**：后两条的 `hound` 原文**都**是"Ill-formed WAVE"，
只靠文案字面**分不开**"格式不对"与"文件没了"（`os error 2` 那条只是恰好露了出来）。

### 5.3 判据 ③：CLI 报告行（用户可见）

```text
source=… + sfz(tracks=1 regions=4 samples=1 silent=3 missing=["fake.flac"]
unreadable(not_found=0,unsupported=1,other=0)) + audio-clips(rendered=0 skipped=1)
```

⇒ FLAC 那一份落 **`unsupported=1`**（⛔ 不是 `not_found`），且 `nonzero=9598` ⇒ 有样本的那条区域**照常发声**。
⚠ 计数段用**逗号分隔无空格**：报告行按空格切词（`cli_contract::field`），⛔ 不引入第二种切法。

---

## 6. 注入双向（含应绿腿）

**还原自证**：注入前对 4 个文件取 `shasum -a 256`；每次注入后**逐行还原**再 `shasum -c`
⇒ 4 个文件全部 `OK`，且 `grep -rn "INJECTION" crates/yeban-app/` = **0 行**。

| 注入 | 改什么 | 期望 | 实得 |
| :--- | :--- | :--- | :--- |
| **A**（新分类路径 ⇒ 红） | `classify_sample_failure` 开头 `return SampleReadFailure::NotFound;` | ②③红、①绿 | ②③ **FAILED**、① **ok** |
| **B**（应绿腿 ⇒ 红） | `decode_sample_classified` 成功臂把第 0 帧清零（模拟"为支持别的格式而动了 WAV"） | ①红 | ① **FAILED** |

**注入 A 的红读数**（逐字）：

```text
unreadable_kind(not_found=4,unsupported=0,other=0)          ← 四档全塌成一档
assertion `left == right` failed: ⭐ `.xyz` 后缀 ⇒ 必须是 `UnsupportedFormat`（任务单 `#83` 逐字点名的那条）
  left: Some(NotFound)
 right: Some(UnsupportedFormat)
⭐ 导出报告行必须带可判别计数（FLAC ⇒ unsupported=1，⛔ 不是 not_found）：
  … unreadable(not_found=1,unsupported=0,other=0) …
test result: FAILED. 1 passed; 2 failed; 0 ignored
```

**注入 B 的红读数**（逐字）：

```text
[app-sfz-formats/wav-fingerprint] fnv1a_64=0xc0ec3f3fa2b8840d nonzero=9596 peak=0.109731831
assertion `left == right` failed: ⭐ 同平台（本机 == aarch64/macos）⇒ WAV 样本路径的输出必须逐位等于**接线前**实测指纹
  left: 13901555692331369485      （0xc0ec3f3fa2b8840d）
 right: 1005561632302452345      （0x0df478fa15264e79）
test result: FAILED. 2 passed; 1 failed; 0 ignored
```

⚠ 注入 A **没有**让 ① 变红（1 passed）—— 那正是"两条腿互不冒充"的证据：关掉新分类不碰 WAV 路径。

---

## 7. 状态差集（第 241 条）

**指标**：`(测试二进制, 判据名, 状态)` 三元组，来自 `cargo test -p yeban-app --locked` 的逐条输出。

| 读数 | 值 |
| :--- | :--- |
| 改动前 | **557** 条（全部 `ok`） |
| 改动后 | **560** 条（全部 `ok`） |
| 差集 | ⭐ **恰好 ＋3 条，全为 `sfz_sample_formats.rs` 新增且 `ok`** |
| 既有判据状态变化 | **0 条** |

⇒ 状态差集**恰好等于本单应变的那些**（⛔ 无一条既有判据翻红或翻绿）。

---

## 8. 边界复述（⛔ 不假装）

1. **FLAC／OGG 仍然不能发声** —— 本单**只**让"读不到"可判别，⛔ 没有接入解码。
   真实音色库里的 FLAC／OGG 样本 ⇒ 依旧按**缺样本**静音 ＋ 计数（`#79` 的边界第 ③ 条**原样成立**）。
2. **`fake.flac` 是魔数夹具，⛔ 不是真 FLAC**：本机没有真 FLAC／OGG 测试样本
   ⇒ 判据 ② 造的是 `fLaC` 魔数 ＋ 垃圾体。它证明的是"**理由**分得开"，⛔ **不**证明"能解 FLAC"。
   （既然本单不接线，也就**没有**"WAV vs FLAC 逐样本差 ≤ 量化误差"这条端到端读数可给 ——
   甲才会产生它。）
3. **不改模型侧／引擎侧**：全部改动在 `crates/yeban-app/**`（控制侧）；
   引擎的"缺样本 ⇒ `NO_SAMPLE` ＋ 计数"口径**一字未动**。
4. ⛔ 未动 `Cargo.toml` / `Cargo.lock` / `audio_clip.rs`；未推远端。
5. ⚠ `SfzDecodeReport::summary()` 的文案变了（多一段 `unreadable_kind(...)`）——
   它不是任何判据的断言对象（只被 `println!`），故状态差集不含它。

## 9. 需要集成者裁的事（`SCHEMA_REQUEST`）

**要不要在 `crates/yeban-app/Cargo.toml` 加这条工作区内部依赖边？**

```toml
# 非 PCM-WAV 样本（FLAC／OGG）的解码入口（symphonia ＋ rubato）。
# 为什么 app 需要它: SFZ 的 `sample=` 在真实音色库里大量是 FLAC／OGG，
#   而 app 现有的 `read_plain_wav_mono_f32` 只吃 PCM／float WAV（`#83` 的边界第 ③ 条）。
# ⛔ 不是新的外部 crate（`Cargo.lock` 里本来就有 `yeban-decode`；根 `Cargo.toml:94` 已登记）
#   ⇒ `cargo deny` 的许可面不变；但依赖图变了 ⇒ 按纪律重生成
#   `docs/ledger/dependency-licenses.md` 并一起提交。
yeban-decode = { workspace = true }
```

- **为何必要**：不加这条边，app **无法**在源码里命名 `yeban_decode::*`（Rust 要求显式依赖）
  ⇒ 甲不可能在不批的前提下完成。
- **影响面**：`Cargo.lock` 的 `yeban-app` 依赖列表 ＋ `-e normal` 图 **+22 个包**（逐包见 §2.1），
  其中 `symphonia` 属 `AGENTS.md` §5.2 点名的重依赖；`cargo deny` 许可面**不变**。
- **不批则维持现状**：本单交付即终态（FLAC／OGG 不支持、但理由可判别）。
- **若批**：接法已勘明 —— `read_plain_wav_mono_f32` 见 §1.1 的**一次调用＋折单声道**，
  WAV 仍走既有 `hound` 路径（保 `BASELINE_WAV_READ` 逐位），其余后缀走 `decode_path`。
  ⚠ 折单声道口径必须显式（`step_by(channels)` ⇒ 声道 0），⛔ 不可把交织当单声道。
