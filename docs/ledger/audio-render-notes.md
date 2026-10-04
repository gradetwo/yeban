# audio-render 工作线台账：**音频片段真的进母带**了

- **台账类型**：交付映射 / **能力矩阵前后对照** / 判据与注入证据 / 未决项（**不是规范**）
- **工作线**：`line/audio-render`（worktree `yeban/.worktrees/audio-render`，基线 `44a071a`）
- **所有者目录**：`crates/yeban-mcp/**`、`crates/yeban-render/**`、`crates/yeban-decode/**`
- **规范来源**：
  - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §7.2（`MCP-TOOL-008`）、§4（离线渲染管线）、
    §5.3（`ARCH-SEC-003` 容器与资产）、§10.2（`ARCH-DSP-002` 重采样）、`ARCH-PDC-001`（延迟对齐）、
    `ARCH-TOP-002` / `ARCH-RT-001`（解码不属于实时回调）、`ARCH-DET-001/002`（确定性）
  - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` `ROAD-M4-004/005/006`、`ROAD-M-1-004`
  - `docs/adr/ADR-0001-workspace-topology-and-version-pinning.md`
    **D19**（render 不拖 cpal）、**D25**（错误码联集 20 值）、**D26**（`symphonia`/`rubato` 的真实 API）、
    **D30**（`.yeban` 容器：资产字节住在容器里）、**D32**（跨架构位精确**按运算类别分策**）、
    **D43**（1.0.0 之前没有历史包袱；自查见 §3.4）
  - `schemas/mcp-tools.schema.json`（承重契约：工具名 / `dryRun` / `idempotencyKey` / `error.code` 联集）
  - 前置台账：`docs/ledger/mcp-render-notes.md`（本线接手它的 `needs-1` / `needs-2`）、
    `docs/ledger/decode-core-notes.md`（`yeban-decode` 的能力与边界）、
    `docs/ledger/store-container-notes.md`（资产字节的存放位置与"裸 JSON 兼容路径"）

> 本文件回答：**`ClipContent::Audio` 现在到底渲染成什么**、**哪些组合仍不支持**、
> **实测数字是多少**、**哪些读数来自本机、哪些只能来自 CI**、**需要谁裁决什么**。

---

## 1. 落地清单（文件 ↔ 规范 ID）

| 文件（相对仓库根） | 内容 | 规范 ID |
| :--- | :--- | :--- |
| [`crates/yeban-mcp/src/domain/render_clip_math.rs`](../../crates/yeban-mcp/src/domain/render_clip_math.rs) **(新增)** | **零第三方依赖**纯逻辑：片段帧落位、两级 dB 增益合成、声道布局矩阵、重采样判定、编码器延迟裁剪区间、容器魔数嗅探 | `ARCH-DSP-002`、`ARCH-DET-001` |
| [`crates/yeban-mcp/src/domain/render.rs`](../../crates/yeban-mcp/src/domain/render.rs) | 资产解析（**哈希复核 → 解码 → 延迟裁剪 → 重采样**）、`AudioClipSource` / `TrackSource`、响应 `data.audio` 段、错误映射 | `MCP-TOOL-008`、`ARCH-DSP-002`、`ARCH-PDC-001`、`ARCH-TOP-002` |
| [`crates/yeban-mcp/src/domain/mod.rs`](../../crates/yeban-mcp/src/domain/mod.rs) | `pub mod render_clip_math`；`impl render::AssetStore for Domain`（会话 CAS 池的只读投影，**唯一来源**）；`plan_render_master` 把资产池交给渲染层 | `ARCH-SEC-003` |
| [`crates/yeban-mcp/Cargo.toml`](../../crates/yeban-mcp/Cargo.toml) | 新增依赖边 `yeban-decode.workspace = true`（**复用根清单已登记的条目**，未改根 `Cargo.toml`） | `ADR-0001` D21 |
| [`crates/yeban-mcp/tests/render_audio_clips.rs`](../../crates/yeban-mcp/tests/render_audio_clips.rs) **(新增)** | **13 条端到端判据**（样本对应、落位、门控、重采样、确定性+钉死哈希、缺资产、哈希不符、声明但无字节、坏资产不 panic、`dryRun`、幂等、PDC） | `MCP-TOOL-008`、`MUST-GATE-002` |
| [`crates/yeban-mcp/verify/render_pure.rs`](../../crates/yeban-mcp/verify/render_pure.rs) | 本机零依赖脚手架：新增 [`render_clip_math`] 的 6 条**独立参考实现对账**（整数有理数版落位、显式布局表、显式裁剪参考、贪心嗅探充要性…） | — |
| [`crates/yeban-mcp/verify/audio_pins.py`](../../crates/yeban-mcp/verify/audio_pins.py) **(新增，非 cargo 目标)** | **独立 Python 复算**三个钉死摘要（SHA-256 over 位型 / xorshift32+TPDF / RIFF 头布局），证明判据里的常量是**被推导出来的** | `ARCH-DET-001` |
| [`crates/yeban-mcp/tests/render_master.rs`](../../crates/yeban-mcp/tests/render_master.rs) | 判据 7 改写：采样率不一致从"`RENDER_FAILED` + `unwired`"改成"成功 + 报告重采样口径" | `MCP-TOOL-008` |
| [`crates/yeban-mcp/src/transport/http.rs`](../../crates/yeban-mcp/src/transport/http.rs) | 同名判据改写（HTTP 腿不再把 44.1 kHz 请求当失败） | `MCP-TOOL-008` |
| [`crates/yeban-mcp/src/tools.rs`](../../crates/yeban-mcp/src/tools.rs) | `sampleRate` 的参数说明更正为"输出采样率；不一致时重采样" | `MCP-TOOL-008` |
| [`docs/ledger/mcp-render-notes.md`](mcp-render-notes.md) | 能力矩阵里 `audioClips` 那一格从 `unsupported` 改写；`needs-1`/`needs-2` 关闭 | — |
| [`docs/ledger/tools-domain-notes.md`](tools-domain-notes.md) | `MCP-TOOL-008` 那一行的能力边界同步 | — |
| [`Cargo.lock`](../../Cargo.lock) / [`docs/ledger/dependency-licenses.md`](dependency-licenses.md) | 依赖图变化（+1 条边，**0 个新包**）后重新生成 | `ADR-0001` D5 |

**未触碰**：根 `Cargo.toml`、`.github/**`、`scripts/**`、`deny.toml`、`docs/DEVELOPMENT_LEDGER.md`、
`docs/adr/**`、`docs/YEBAN_*.md`、`schemas/**`、其它 `crates/**`、`spikes/**`、法务文件、`README*.md`。
`crates/yeban-render/**` 与 `crates/yeban-decode/**` 本次**零改动**（见 §2 的"边界选择"）。

---

## 2. 装配图：资产字节 → 解码 → 重采样 → 片段源 → 母线

```text
  .yeban 容器 assets/{sha256}                 工程文档
        │                                          │
        │  open_project 解出                        │  ClipContent::Audio { asset, gain_db }
        ▼                                          ▼
  Domain 会话 CAS 池 ──(render::AssetStore)──► render::build
   (BTreeMap<AssetHash, Vec<u8>>)                    │
                                                     │ 每个被引用的哈希**只处理一次**
                                                     ▼
                              ┌──────────────────────────────────────────────┐
                              │ 1. 池里有字节吗?                              │
                              │    ├ 没有 + 索引里也没有 → RENDER_FAILED      │
                              │    │        (reason = "assetMissing")        │
                              │    └ 没有 + 索引里有     → 登记 audioClips    │
                              │              + 当静音（裸 JSON / 内存夹具）    │
                              │ 2. SHA-256(字节) == 声明的哈希?               │
                              │    └ 不等 → RENDER_FAILED                     │
                              │             (reason = "assetHashMismatch")   │
                              │ 3. yeban_decode::decode_bytes（symphonia）    │
                              │    └ 失败 → RENDER_FAILED / IO_ERROR          │
                              │ 4. encoder_delay/padding → 裁剪区间           │
                              │ 5. 采样率 ≠ 渲染率?                           │
                              │    └ 是 → yeban_decode::resample_interleaved  │
                              │             (rubato Async::new_sinc)         │
                              │ 6. 声道布局: 1→复制 / 2→恒等 / 其它 → 拒绝     │
                              └──────────────────────────────────────────────┘
                                                     │  Arc<[f32]>（只读共享，解码一次）
                                                     ▼
                          AudioClipSource（每个资产一个）
                             ├ 若干 ClipSpan { start, end, gain_l, gain_r }
                             └ render_block: 只做"按帧拷贝 + 乘增益"，**累加**
                                                     │
  MidiSynthSource ───────────────────────────────┐   │
                                                 ▼   ▼
                                     TrackSource（先 MIDI、再按哈希字典序的音频栈）
                                                     │  一个源节点 = 一条音轨
                                                     ▼
                        RenderPlan::execute（Rayon 并行 + 固定顺序归约 + PDC 延迟线）
                                                     │
                                       Master 增益 → (归一化) → TPDF → 24-bit → 容器
```

### 2.1 边界选择（为什么改 `yeban-mcp` 而不是 `yeban-render`）

- `yeban-render` 的模块头明确写着"**样本源是注入的, 不是内建的**"，把资产解码塞进去
  会推翻它的一条承重契约；`AudioSource` 的两个实现（MIDI 合成与音频片段）现在住在
  同一处（`domain/render.rs`），正是这条契约的自然结果。
- 解码**只**发生在 `render::build` 里：每个资产一次，绝不进 `AudioSource::render_block`
  （那个函数会被 Rayon 工作线程按块反复调用）。[ARCH-TOP-002] / [ARCH-RT-001]
- **最终二进制体积与依赖集合不因这个选择而不同**（`yeban-mcp → yeban-render → yeban-decode`
  与 `yeban-mcp → yeban-decode` 链接的是同一批包）；区别只在**依赖边的声明位置与耦合方向**。

### 2.2 延迟：**唯一来源**仍是 `DeviceDefinition::latency_samples`

本线**没有**新增任何延迟字段，也**没有**第二张延迟表：

- 模型层没有 per-asset 延迟字段（`AssetMetadata` 只有 `hash`/`original_path`/`byte_len`/
  `media_kind`/`license`），因此"资产自带 `latency_samples`"这一情形在当前模型里**不存在**；
- 音频片段的全部延迟贡献 = **它所在音轨的设备链累加**，即
  [`yeban_render::render::track_latencies`] 那一张表 —— 与 MIDI 片段走的是同一次调用、
  同一张 `BTreeMap`，响应里的 `latencySource` 仍是
  `"DeviceDefinition::latency_samples (track_latencies)"`；
- 资产**解码器**上报的 `encoder_delay_frames` / `encoder_padding_frames` 是另一回事
  （编码器前置延迟，不是设备延迟）。本线按"**裁剪并如实报告**"处置：裁剪掉的帧数进
  `data.audio.assets[].trimmedFrames`、留下的进 `renderedFrames`。
  `yeban-decode` 的边界是"只记录不裁剪"（见其台账 §9 的 needs），渲染侧要的是时间轴上对齐的
  音频，因此这一步落在渲染侧并**必须报告**。

---

## 3. 能力矩阵：**前后对照**

### 3.1 从 `unsupported` 变成真做

| 能力 | 之前（`line/mcp-render`） | 现在（本线） | 判据 |
| :--- | :--- | :--- | :--- |
| `audioClips` | `ClipContent::Audio` 的 CAS 资产字节**没有被消费**，当静音 + 登记 `unsupported` | **真做**：解码 → 延迟裁剪 → 重采样 → 按 placement 落位 → 增益/静音/独奏门控 → 参与 PDC | `tests/render_audio_clips.rs` 判据 1–13 |
| 重采样（`sampleRate` ≠ 工程采样率） | **一票拒绝**：`RENDER_FAILED` + `data.unwired = "resampler"` | **真做**：`yeban_decode::resample_interleaved`（rubato sinc）；响应逐资产给 `sourceSampleRate`/`targetSampleRate`/`resampled`/`renderedFrames` | 判据 4、5 |
| 音频片段的**时长语义** | 摆放的时序只被算进母带长度 | 素材**不被"改标签"**：44.1 kHz 的 1 秒素材进 48 kHz 工程后仍然是 1 秒（判据用"47000 帧之后仍有 >800 帧信号"证明，改标签的实现只剩 0.919 秒） | 判据 4 |
| 工程帧数上限的来源 | 只有 MIDI 内容能产生帧 | 音频片段的 placement 时值同样产生帧；完全静音的摆放**不构成内容** ⇒ 0 帧 ⇒ `RENDER_FAILED` | 判据 3(c) |

### 3.2 收窄后的表述（**不是**"部分支持"，是"支持的边界写清楚了"）

| 维度 | 支持 | 明确不支持（遇到即 `RENDER_FAILED` 或登记） |
| :--- | :--- | :--- |
| 容器/编解码 | WAV（PCM 8/16/24/32-bit 与 f32、ADPCM）、FLAC、Ogg-Vorbis —— 即 `yeban-decode` 启用的全部 feature | Matroska / AIFF / CAF / ISO-MP4 / MP3 / AAC / ALAC（feature 未启用）；任何解码器认领不了的字节流 |
| 声道 | 1（复制到立体声母线）、2（恒等）、`asset == 母线`（恒等，理论上） | **>2 声道**（不做静默丢声道的降混，直接拒绝：`reason = "assetChannelLayout"`） |
| 采样率 | `SampleRate::ALL` 的 5 个率（44.1/48/88.2/96/192 kHz）互转，以及素材的任意率 → 渲染率 | 素材率或目标率 = 0、超出 `limits::MAX_SAMPLE_RATE`（768 kHz）—— 这两种在 `yeban-decode` 层就被拒 |
| 素材字节 | 会话 CAS 池里的字节，**且** SHA-256 与工程声明的哈希一致 | 池里没有字节但索引里有 ⇒ 登记 `audioClips` + 静音（裸 JSON 兼容路径的形态）；索引里也没有 ⇒ `assetMissing` |
| 摆放语义 | `start_tick`（tick→帧）、`duration_ticks`（**硬切**边界）、`muted`、片段 `gain_db`、音轨 `volume_db`/`pan`/`mute`/`solo`/`solo_safe` | `loop_config` 的重复（登记 `clipLoopRepetition`，与 MIDI 同一条）；交叉淡化（模型里没有这个字段，不发明） |
| 延迟 | 音轨设备链的 `latency_samples` → PDC（与 MIDI 同表） | 资产自带的延迟字段（模型里不存在）；编码器延迟裁剪由本线做，`yeban-decode` 不做 |

### 3.4 与 `ADR-0001` **D43**（1.0.0 之前没有历史包袱）的关系

集成者通报 D43 之后，本线逐条自查（**结论：本线没有为"兼容"保留任何东西**）：

| D43 的问题 | 本线的回答 |
| :--- | :--- |
| 你在改 `store.rs` 的**读取分派**吗？ | **没有。** 本线一个字节都没动 `store.rs`（裸 JSON 兼容路径仍原样保留），因此 `mcp-no-compat` 那条线不会被两条写者撞车 |
| `unsupported` 矩阵里有"为以后兼容而留"的键吗？ | **没有。** 13 个既有键一个都没动；本线唯一涉及的是 `audioClips`，它是**当前会话真的缺资产载荷**时的诚实信号（见下），不是"以后再说" |
| 新增了 `#[serde(default)]` 吗？ | **没有。** 本线不改模型层，也不新增任何 serde 属性 |
| 有没有"为了旧文件能读"的分支？ | **没有。** 唯一的向后兼容形状是 §3.2 最后两行的 `audioClips`，它的触发条件是**会话里没有字节**，而不是"文档版本旧" |

**`audioClips` 为什么不是"为兼容而留"**：它描述的是一个**运行时事实** —— 工程声明了这个
资产、而本次会话的 CAS 池里没有它的字节，于是这些摆放**没有进母带**。删掉这个键就会
把"这一段没渲染"变成"看不出来" —— 那违反 D43 保留的那条红线（**诚实性**：真不支持的
必须在响应里如实说）。它会在下面两件事都发生之后**自然消失**（届时应当由
`mcp-no-compat` 线连同分支一起删除）：

1. `store.rs` 的裸 JSON 读取分派被 D43 删掉（容器成为唯一格式）；**且**
2. `Domain::open_in_memory` 这条"只注入工程、不注入资产载荷"的会话种子路径也被收掉
   （否则内存夹具 / 内存会话仍然会有"声明了但没有字节"的形态）。

在那之前删掉它，只会把 `yeban_model::samples::filled_project()`（仓库自己的规范样本工程）
变成**渲染必然失败**的工程 —— 那是拿诚实换整洁。这一条已登记为 §9 的 **needs-7**。

### 3.3 仍然 `unsupported` 的键（**没有任何一条被本线悄悄去掉**）

`deviceChainDsp`、`externalPlugins`、`automationLanes`、`clipLoopRepetition`、
`noteProbability`、`noteRatchet`、`noteSlide`、`notePitchBend`、`noteLyrics`、
`busTrackClips`、`sidechainRouting`、`masterPan`、`sfzSampler` —— 与 `mcp-render-notes.md`
§2 完全一致。**唯一的变化**是 `audioClips` 的语义被收窄：它现在**只**在
"工程声明了资产、会话池里没有字节"时出现（见 §3.2 最后两行）。

---

## 4. `D26` / `D32` 的口径（这一线必须写清的两条）

### 4.1 `D26`：重采样**用的是哪一种**（写进响应，不许含糊）

`yeban-decode` 按 `ADR-0001` **D26** 以**实际版本**实现（`rubato 5.0.1` 里没有
`SincFixedIn`/`FftFixedIn`）。本线**复用**它的封装，不自建第二套：

| 项 | 值 | 出处 |
| :--- | :--- | :--- |
| 类型 | `rubato::Async::<f32>::new_sinc` | `rubato-5.0.1/src/asynchro.rs:297` |
| 窗 | `WindowFunction::BlackmanHarris2` | `resample.rs` 的 `pub const` |
| `sinc_len` | 256 | `resample::SINC_LEN` |
| 分块 | 1024 帧 | `resample::CHUNK_FRAMES` |
| 最大相对比例 | 1.0（**不可调**，一次导入一个固定比例） | `resample::MAX_RELATIVE_RATIO` |
| 前置延迟 | **0**（上游 `process_all_into_buffer` 内部裁掉启动静音） | `resample::PREFILL_FRAMES` |
| 同率 | **逐位透传**，零滤波（因此本线的钉死哈希夹具不受重采样影响） | `resample_interleaved` 的恒等分支 |

响应里 `data.audio.resampler.method` 就是上表那句话（与 `render_clip_math::RESAMPLER_SUMMARY`
**同一份字符串**，不复制两份会漂移的文案）。

### 4.2 `D32`：本线的位级承诺**按运算类别分策**

`ADR-0001` **D32** 的裁决是：IEEE 精确类（加减乘除、比较、`round`、`sqrt`）跨架构**零容差**；
超越函数类（`log`/`sin`/窗函数…）给数值预算、只在**冻结架构**上要求逐位。

- **本线的钉死哈希夹具是 48 kHz → 48 kHz**（同率透传、零滤波、零超越函数），
  链路上只有"拷贝 + 乘增益 + `round` + 整数运算" ⇒ 常量是**跨架构可比**的（L1 形式）。
  这一点由 `verify/audio_pins.py` 用**另一个语言**复算验证：它不调用任何 Rust 代码，
  算出的三个摘要与判据里的常量**逐字节相同**。
- **重采样路径不作位级承诺**：rubato 的窗系数来自 `windowfunctions`，属"超越函数类"。
  判据 4 因此断言的是**长度契约 + 时长不缩水 + 前置静音为 0**（数值/结构性），
  **不**断言字节；响应里 `data.audio.bitExactness` 也如实写着这一点。
- 跨架构的实测读数（`MUST-GATE-003` / 手动档 `arm`）**不在本线**：那需要
  `HD-36` 的 ARM 腿，本线不假装通过。

---

## 5. 实测数字

### 5.1 夹具（`tests/render_audio_clips.rs` 判据 6）

固定的 48 kHz 立体声夹具：Master + 一条音轨 → Master；素材 = 480 帧 32-bit float WAV
（两个声道各一个 1/64 阶梯，逐位可复现）；工程 `rng_seed = 0x594542414E000001`、
固定注入时钟 `1_760_000_000_000` ms、固定 ULID（无熵源 ⇒ 完全确定）。

| 量 | 值 |
| :--- | ---: |
| `frames` / `channels` / `bitDepth` | 48 000 / 2 / 24 |
| `payloadBytes` / `headerBytes` / `bytes` | 288 000 / 694 / 288 694 |
| `masterDigest`（浮点母带位型 SHA-256） | `c600d4f011f6e223d41363cfc8763b13f1c55ba426f0a95b895c0461c8f3a213` |
| 24-bit **负载** SHA-256 | `1c6747217c02b1c93c3d24673568be2dc5ea58c5644552fb4fd5ca7a92775f31` |
| 整份 RIFF 文件 SHA-256 | `6d290a90a091128c1842c6a7ce10f044773dafdcfc919b534f441f42d7522c37` |
| 片段落位误差 | **0 帧**（判据 2：起点恰好第 24 000 帧、终点恰好第 24 239 帧） |
| 样本对应最大偏差 | ≤ 2 LSB（`2/2²³ ≈ 2.38e-7`；容差理由见判据 1 的文档注释） |
| PDC 实测样本偏移 | **48 帧**（`L_max = 48`，判据 13 逐帧读三个电平台阶） |
| 重采样长度 | 44 100 帧 @44.1k → 48 000 帧 @48k（理想值 ±48 帧内；47000 帧之后仍有 >800 帧信号） |

三个常量都是**独立推导**的：`verify/audio_pins.py` 用 Python 按源码口径重写了一遍
（xorshift32 + TPDF + 24-bit 取整 + RIFF/`bext` 布局），跑出来的值与判据里的常量逐字节相同。

### 5.2 端到端夹具的其它读数

| 场景 | 读数 |
| :--- | :--- |
| placement 静音 | `RENDER_FAILED`（`endTick = 0`）—— 静音的摆放不构成内容，与 MIDI 摆放同一语义 |
| 音轨 mute | 母带峰值 ≤ 2 LSB；`sources[].audible = false`、`audioClipsGated = 1` |
| 片段 `gain_db = −6` | 峰值 0.5 → 0.2506（`0.5 × 10^(−6/20)`，±1%） |
| 缺资产（索引与池都没有） | `RENDER_FAILED` + `data.reason = "assetMissing"` |
| 池里的字节与哈希不符 | `RENDER_FAILED` + `data.reason = "assetHashMismatch"` + `data.actual` |
| 声明了但池里没有字节 | 成功 + `unsupported = ["audioClips"]` + `data.audio.assets[0].bytesPresent = false` |
| 垃圾字节 / 截断 / 12-bit PCM | `RENDER_FAILED` + `data.reason = "assetDecodeFailed"`（**不 panic**） |
| `dryRun` | 目录条目数不变；预览给出 `frames`/`payloadBytes`/`wouldWrite.sha256` |
| 同幂等键重放 | `replayed = true`；产物 `mtime` 不变；删掉产物后重放**不重建** |

---

## 6. 判据清单

### 6.1 端到端（[`crates/yeban-mcp/tests/render_audio_clips.rs`](../../crates/yeban-mcp/tests/render_audio_clips.rs)，**13 条**）

| # | 判据 | 钉住的可观测量 |
| :--- | :--- | :--- |
| 1 | `an_audio_clip_renders_into_the_master_matching_the_asset_samples` | 逐样本对账（≤2 LSB）+ 静音段 ≤1 LSB + `audio.*` 实测字段 + `unsupported == []` |
| 2 | `the_clip_lands_exactly_on_the_placement_start_frame` | 起止帧**恰好** 24000 / 24239（≤1 帧，实测 0） |
| 3 | `mute_solo_and_clip_gain_measurably_gate_the_audio_clip` | 基线 0.5 / −6 dB 减半 / 静音摆放 0 帧 ⇒ `RENDER_FAILED` / 音轨 mute 静音 + `audioClipsGated` / solo 仍发声 |
| 4 | `a_44k1_asset_in_a_48k_project_is_resampled_and_keeps_its_duration` | `resampled = true`、长度契约 ±48 帧、47000 帧后仍有 >800 帧信号（判别"改标签"） |
| 5 | `a_sample_rate_mismatch_is_no_longer_an_error` | 96 kHz 请求成功 + `unwired` 消失 + 口径字符串含 `rubato`/`BlackmanHarris2` |
| 6 | `two_renders_of_the_same_project_are_byte_identical_and_hit_the_pinned_hashes` | 逐字节相同 + **三个**钉死常量 |
| 7 | `an_asset_that_exists_nowhere_is_a_named_error` | `RENDER_FAILED` + `assetMissing` + 资产哈希 + `declaredInIndex=false` |
| 8 | `bytes_that_do_not_match_the_declared_hash_are_rejected` | `assetHashMismatch` + `actual`（走手工构造的池，绕开 `put_asset`） |
| 9 | `a_declared_asset_without_payload_is_registered_rather_than_faked` | `unsupported = ["audioClips"]` + `bytesPresent=false` + 无信号 |
| 10 | `broken_assets_are_contract_errors_not_panics` | 三类坏资产 ⇒ 契约内码 + `assetDecodeFailed` + 无残留 |
| 11 | `dry_run_renders_the_audio_clip_without_writing_anything` | 目录条目不变 + 预览的实测数字 |
| 12 | `the_same_idempotency_key_does_not_render_the_audio_clip_twice` | `replayed` + `mtime` + 删产物不重建 |
| 13 | `pdc_shifts_the_audio_clip_branch_by_exactly_the_device_latency` | 三个电平台阶 ⇒ 偏移**恰好 48 帧** + `latencySource` |

### 6.2 纯逻辑（本机 `rustc --test`，`render_clip_math.rs` 的 7 条 + 脚手架的 6 条对账）

`render_clip_math.rs` 自带 7 条（落位精确、病态输入、静音是 `None`、布局矩阵、
重采样判定、延迟裁剪、魔数嗅探严格性）；[`verify/render_pure.rs`](../../crates/yeban-mcp/verify/render_pure.rs)
另外用**独立参考实现**对账 6 条（整数有理数版落位、显式布局表、显式裁剪参考、
增益精确和、嗅探的充要性、重采样判定对称性）。

### 6.3 改写的旧判据

| 文件 | 旧 | 新 |
| :--- | :--- | :--- |
| `tests/render_master.rs` | 判据 7 把"采样率不一致 ⇒ `RENDER_FAILED` + `unwired`"当成正确行为 | 同一处断言它**成功**并给出 44.1 kHz 母带 + 重采样口径；`unwired` 必须消失 |
| `src/transport/http.rs` | 同名判据（HTTP 腿） | 同上（`dryRun` 腿，读 `data.preview`） |
| `src/domain/render.rs` 的文档 | `audioClips` 在"明确没做"表里 | 移入"真做"清单，并新增采样率一节 |

---

## 7. 本机真跑 vs CI（**严格区分**）

### 7.1 本机**真的跑了**什么

| 命令 | 读数 |
| :--- | :--- |
| `rustc --edition 2024 --test -D warnings -W missing_docs crates/yeban-mcp/verify/render_pure.rs` | **30 通过 / 0 失败**（真实 `render_math.rs` + `render_clip_math.rs`） |
| `clippy-driver --edition 2024 --test -D warnings -D clippy::all -D rust_2018_idioms …verify/render_pure.rs` | **0 告警** |
| `python3 crates/yeban-mcp/verify/audio_pins.py` | 三个摘要与判据常量逐字节相同 |
| `bash scripts/dev/cargo-local.sh fmt --all --check` | 通过 |
| `bash scripts/gates/run-gates.sh light` | 通过（fmt + 13 条红线守卫 + 文档链接 + 许可清单漂移） |
| `python3 scripts/gates/license_inventory.py` + `--check` | 重新生成后通过（只有 `Cargo.lock` 摘要变化，**依赖清单本身未变**） |

### 7.2 本机**替代性验证**（必须如实说明，别当成"CI 已绿"）

`yeban-mcp` 现在传递依赖 `rayon` / `hound` / `midly` / `symphonia` / `rubato`。
按本机纪律（`AGENTS.md` §5.2、`docs/DEV_WORKFLOW.md`）我**没有**在本机编译它们，
也就**没有**跑 `run-gates.sh crate yeban-mcp` / `cargo test -p yeban-mcp`。

替代方案（**仓库外的临时脚手架**，`/tmp/tcroot/pkg/mcp-audio-typecheck`，不入库）：

- 一个 scratch 包用**绝对路径**把本 worktree 的 `src/lib.rs` 当作 crate root
  （因此 `crate::…` 路径、内嵌 `#[cfg(test)]`、6 个集成测试目标全都是**真实源文件**）；
- 它把重依赖换成两个替身：
  - `yeban-render`：**`#[path]` 引入真实源文件** `dither.rs` / `pdc.rs` / `render.rs` /
    `rf64.rs` / `rng.rs` / `sum.rs` / `vlq.rs`，唯一被替换的是 `rayon`
    （一个**顺序执行**的影子 crate：`install` 直接调用、`par_iter_mut().try_for_each` 顺序跑）。
    因此**调度、PDC 延迟线、固定顺序归约、抖动、容器布局全是真实现**；
  - `yeban-decode`：手写影子，镜像被用到的那一小部分 API，自带一个最小 WAV 解析器
    （f32 WAV 是**逐位拷贝**，与 symphonia 的行为一致 —— 见 `decode-core-notes.md` §3.2）
    与一个线性插值重采样器。
- `[lints]` 复制工作区的 `clippy::all`/`rust_2018_idioms` 设置，因此 `clippy -D warnings`
  与 CI 同源。

| 命令（在 scratch 包里） | 读数 |
| :--- | :--- |
| `cargo test --lib` | **181 通过 / 0 失败** |
| `cargo test --test render_audio_clips` | **13 通过 / 0 失败** |
| `cargo test --test render_master` | **12 通过 / 0 失败** |
| `cargo test --test tools_e2e / contract / container_store / lock_advisory` | **25 / 15 / 16 / 14 通过** |
| 合计 | **276 通过 / 0 失败** |
| `cargo clippy --all-targets` | **0 告警** |

**替身的边界（不许当成真 CI 的证据）**：

1. `rayon` 被顺序化 ⇒ "任意线程数 ⇒ 逐位相同"这条**在本机没有被真正检验**
   （CI 上由 `yeban-render` 自己的判据检验）；
2. 解码替身**不是** symphonia：它对 f32 WAV 逐位一致、对 16-bit PCM 用 `/32768`，
   但 Ogg/FLAC/ADPCM 的路径**完全没跑过**；CI 上跑的是真解码器；
3. 重采样替身是线性插值 ⇒ 判据 4 的**数值**读数（长度契约、时长）可以在 CI 上复现，
   但"rubato 的相位/幅频响应"这一层本机没有证据；
4. 这些读数**替代不了** CI：`run-gates.sh crate yeban-mcp` 与
   `cargo test -p yeban-mcp --all-targets` 是唯一的判决。

### 7.3 注入 → 变红 → 还原（**9 条，全部真做过**）

方法：把 `render.rs` / `render_clip_math.rs` 备份到 `/tmp/inj-backup-audio/`，
用 Python 精确替换并**断言锚点恰好命中 1 次**（否则属于"注入无效"，账本 L3），
跑判据、记录红掉的判据名，再从备份还原并复跑全绿。

**纯逻辑层（`rustc --test`，全部在本机真跑）**：

| # | 注入 | 变红的判据（实测） | 还原 |
| :--- | :--- | :--- | :--- |
| **J1** | `clip_frame_span` 的起点 **+1 tick** | `clip_span_matches_the_rational_reference`、`clip_span_maps_one_second_of_ticks_to_the_sample_rate`（28 passed; 2 failed） | ✅ 30/30 |
| **J2** | `needs_resample` 恒 `false`（**丢弃重采样**） | `resample_decision_is_symmetric_and_size_independent`、`resample_decision_is_about_inequality_only`（28; 2 failed） | ✅ 30/30 |
| **J3** | `encoder_trim_span` 的 `from` 恒 0（**不裁剪编码器延迟**） | `encoder_trim_matches_the_explicit_reference`、`encoder_trim_span_handles_delay_padding_and_overclaims`（28; 2 failed） | ✅ 30/30 |
| **J4** | 单声道素材不再复制到所有母线声道 | `channel_layout_agrees_with_the_explicit_table`、`channel_layout_matrix_is_explicit`（28; 2 failed） | ✅ 30/30 |

**端到端层（scratch 脚手架里跑**真实** `render.rs`**）：

| # | 注入 | 变红的判据（实测） | 还原 |
| :--- | :--- | :--- | :--- |
| **E1** | `AudioClipSource::render_block` 直接返回（**片段源换成静音**） | 6 条（样本对应、落位、门控、重采样、确定性+钉死哈希、PDC）（7 passed; 6 failed） | ✅ 13/13 |
| **E2** | 片段落位 **+1 帧** | 5 条（样本对应、落位、重采样、确定性+钉死哈希、PDC）（8; 5 failed） | ✅ 13/13 |
| **E3** | **关掉资产哈希校验**（`if false && actual != *hash`） | `bytes_that_do_not_match_the_declared_hash_are_rejected`（12; 1 failed） | ✅ 13/13 |
| **E4** | **缺资产静默当静音**（`if false && !declared_in_index`） | `an_asset_that_exists_nowhere_is_a_named_error`（12; 1 failed） | ✅ 13/13 |
| **E5** | `needs_resample` 恒 `false`（**改标签式假重采样**） | `a_44k1_asset_in_a_48k_project_is_resampled_and_keeps_its_duration`、`a_sample_rate_mismatch_is_no_longer_an_error`（11; 2 failed） | ✅ 13/13 |

**这 9 条证明的事**：① "片段一定静音"会被抓；② "落位差一帧"会被抓；
③ "完整性校验被关掉"会被抓；④ "缺资产被吞成静音"会被抓；⑤ "重采样被跳过"会被抓；
⑥ 重采样/落位/裁剪/声道的纯逻辑错误会在**本机**就变红。

**诚实边界**：E1–E5 是在**替身环境**（顺序 rayon + 解码影子）里跑的，虽然用的是真实
`render.rs` / `render.rs` 之外的 mcp 源码，但**不是** CI 环境；CI 上会再用真
rayon/symphonia/rubato 重跑同一批判据。

---

## 8. `BASELINE-001` / `MUST-GATE-002/003` 的关系

**不变，如实说**：本线没有产出任何性能读数（没有 `criterion`，不做吞吐声明）。
`MUST-GATE-002`（同平台 bit-exact）仍是**部分**：本线新增的三个**钉死常量**是
"同平台 + 同一实现"的位级判据，且由独立 Python 实现复算过；但"跨机器/跨架构 SHA-256 全同"
这条门禁仍不存在（`MUST-GATE-003` 依然 PENDING，`HD-36` 的 ARM 腿未接）。
按 `D32`，**含重采样的链路本来就不该承诺跨架构逐位** —— 本线的承诺边界写在 §4.2。

---

## 9. needs / pending / TODO(hoist)

### needs（需要裁决或别的所有者接线）

| # | 项 | 性质 | 建议 |
| :--- | :--- | :--- | :--- |
| **needs-1** | **多声道素材的降混规则**（>2 声道素材进立体声母线） | 需要产品裁决 | 现状是**拒绝**（`assetChannelLayout`）。若要支持，需要一条规范化的降混矩阵（ITU-R BS.775？还是按 `MediaKind`？）—— 凭空发明一个会破坏 [ARCH-DET-001] 的可解释性 |
| **needs-2** | **片段内偏移 / 交叉淡化** | 模型缺口 | `ClipPlacement` 没有"素材内起点"与淡入淡出字段。若产品需要，先扩模型（别的所有者），渲染侧再接 |
| **needs-3** | **`Ogg-Vorbis` / `ADPCM` 的字节级夹具** | 承接 `decode-core` 的 pending | 本线的判据只喂 WAV（f32/PCM）。FLAC/Ogg 的路径在 CI 上会经过真解码器，但**没有**音频片段的端到端夹具；建议随 `decode-core` 的 pending 一起补 |
| **needs-4** | **编码器延迟裁剪的规范落点** | 需要裁决 | `yeban-decode` 的边界是"只记录不裁剪"，本线在渲染侧裁剪并报告。若要统一（gapless 语义属于导入还是渲染），需要一条 ADR |
| **needs-5** | **`run-gates.sh crate yeban-mcp` 在本机会真编译重依赖** | 门禁口径 | 承接 `mcp-render-notes` 的 needs-7（`HD-39`）。本线新增的 `yeban-decode` 边让本机编译更贵（symphonia+rubato），这条更值得修 |
| **needs-6** | **音频片段的性能读数**（解码/重采样/逐块拷贝的吞吐） | 未打点 | 没有 `criterion`；`BASELINE-*` 仍与本线无关 |
| **needs-7** | **`audioClips` 键在 D43 之后的归属** | 跨线 + 需要裁决 | 当 `mcp-no-compat` 删掉裸 JSON 读取分派、且 `Domain::open_in_memory` 也不再产生"有声明无载荷"的会话时，§3.4 那条分支与 `audioClips` 键应当**一起删除**（并把它改成 `assetMissing` 硬错误）。本线**不在**别人的文件里做这件事，也不在触发条件成立之前提前删 |

### pending

| # | 项 | 状态 |
| :--- | :--- | :--- |
| P | `mcp-render-notes.md` 的 `needs-1`（音频片段渲染） | **本轮关闭** |
| P | `mcp-render-notes.md` 的 `needs-2`（重采样接线） | **本轮关闭**（`RENDER_FAILED` 换成真重采样） |
| P | `HD-36` 的 ARM 腿（跨架构读数） | 仍 pending（本线不作跨架构位级承诺，见 §4.2） |
| P | 音频片段的 `loop_config` 重复展开 | 与 MIDI 同一条 `clipLoopRepetition`，仍 pending |
| P | 音频片段的自动化（`automationLanes` 对片段增益的驱动） | 仍 pending（自动化曲线整体未求值） |

### TODO(hoist)

**没有。** 本线只新增一条**已在根 `[workspace.dependencies]` 登记**的依赖边
（`yeban-mcp → yeban-decode`）：`Cargo.lock` 的包集合**不变**（`git diff Cargo.lock`
只有 1 行），因此 `deny` / 许可清单**不漂移**（`license_inventory.py` 只有
`Cargo.lock` 摘要那一行变化）。

---

## 10. CI 判决（读到什么写什么）

读取方式：`bash scripts/dev/ci-verdict.sh line/audio-render`

| 轮次 | run id | 头部 | 结论 |
| :--- | ---: | :--- | :--- |
| 第 1 轮（音频片段真渲染 + 13 条判据 + 9 条注入） | 见 `ci-verdict.sh` 的读数 | 本提交 | 见下（提交后回填） |

> 本文件自身是**文档改动**：记录判决的那一次提交会再前进一格，且只改 `docs/ledger/**`
> 与 `crates/yeban-mcp/verify/**`。按纪律仍然读回判决（§10 的表格在判决回来后追加一行）。

---

## 11. 修改文件的绝对路径清单

```text
/Users/crow/work/music/yeban/.worktrees/audio-render/Cargo.lock
/Users/crow/work/music/yeban/.worktrees/audio-render/crates/yeban-mcp/Cargo.toml
/Users/crow/work/music/yeban/.worktrees/audio-render/crates/yeban-mcp/src/domain/mod.rs
/Users/crow/work/music/yeban/.worktrees/audio-render/crates/yeban-mcp/src/domain/render.rs
/Users/crow/work/music/yeban/.worktrees/audio-render/crates/yeban-mcp/src/domain/render_clip_math.rs   (新增)
/Users/crow/work/music/yeban/.worktrees/audio-render/crates/yeban-mcp/src/tools.rs
/Users/crow/work/music/yeban/.worktrees/audio-render/crates/yeban-mcp/src/transport/http.rs
/Users/crow/work/music/yeban/.worktrees/audio-render/crates/yeban-mcp/tests/render_master.rs
/Users/crow/work/music/yeban/.worktrees/audio-render/crates/yeban-mcp/tests/render_audio_clips.rs   (新增)
/Users/crow/work/music/yeban/.worktrees/audio-render/crates/yeban-mcp/verify/render_pure.rs
/Users/crow/work/music/yeban/.worktrees/audio-render/crates/yeban-mcp/verify/audio_pins.py        (新增)
/Users/crow/work/music/yeban/.worktrees/audio-render/docs/ledger/dependency-licenses.md           (机器重新生成)
/Users/crow/work/music/yeban/.worktrees/audio-render/docs/ledger/mcp-render-notes.md              (矩阵那一格 + needs)
/Users/crow/work/music/yeban/.worktrees/audio-render/docs/ledger/tools-domain-notes.md            (008 那一行)
/Users/crow/work/music/yeban/.worktrees/audio-render/docs/ledger/audio-render-notes.md            (新增, 本文件)
```

**未触碰**：根 `Cargo.toml`、`.github/**`、`scripts/**`、`deny.toml`、
`docs/DEVELOPMENT_LEDGER.md`、`docs/adr/**`、`docs/YEBAN_*.md`、`schemas/**`、
`crates/yeban-render/**`（零改动）、`crates/yeban-decode/**`（零改动）、其它 `crates/**`、
`spikes/**`、法务文件、`README*.md`。
