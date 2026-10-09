//! `yeban_render_master` 的实现：参数校验 **+ 真渲染** [MCP-TOOL-008] [ROAD-M4-005]。
//!
//! ## 这一半现在是真的
//!
//! 上一线（`line/tools-domain`）只接了参数校验这一半：好参数会在校验通过之后返回
//! JSON-RPC `-32005 NOT_IMPLEMENTED`。现在这一半**接线到 `yeban-render`**：
//!
//! ```text
//! 工程 + 路由图 + 会话 CAS 资产池 (assets/{sha256})
//!   ├─ 音频片段: 资产字节 → (哈希复核) → yeban-decode 解码 → 编码器延迟裁剪
//!   │            → 必要时 rubato sinc 重采样 → AudioClipSource   [ARCH-DSP-002 / D26]
//!   ├─ track_latencies(DeviceDefinition::latency_samples)   [ARCH-PDC-001] **唯一**延迟来源
//!   ├─ RenderPlan::compile_with_latencies(...)              拓扑分层 + PDC 关键路径
//!   ├─ 每个源节点注入一个 AudioSource                       MIDI 合成源 ⊕ 音频片段源
//!   ├─ RenderPlan::execute(...)                             Rayon 并行 + 固定顺序归约 [ARCH-DET-002]
//!   ├─ 母带增益 + (可选) 峰值归一化
//!   ├─ TPDF 抖动 → 24-bit PCM                              [ARCH-FMT-001]
//!   └─ RIFF / RF64 / BW64 容器 + bext 元数据               [ARCH-FMT-001]
//!        └─ 原子落盘: 同目录 .tmp + sync_all + rename      [ARCH-SEC-004]
//! ```
//!
//! 渲染发生在 [`build`]（**只读计算**，`dryRun` 也走它，因此预览里的字节数、
//! SHA-256、帧数都是**实测值**而不是估算），落盘发生在 `domain::apply`。
//! 这个切分保住了"`dryRun` 不改状态"的类型系统级保证：[`super::plan`] 只拿 `&Domain`。
//!
//! ## 解码发生在哪里（[ARCH-TOP-002] / [ARCH-RT-001]）
//!
//! **全部解码在 [`build`] 里完成，一次都不在 `AudioSource::render_block` 里。**
//! 这是刻意的边界：`render_block` 会被 Rayon 工作线程按块反复调用，
//! 一个"按需现解码"的实现会在每一块上重开一次容器。
//! 每个资产只解码一次（`BTreeMap<AssetHash, …>` 缓存），必要时重采样一次，
//! 渲染期只有"按帧拷贝 + 乘增益"。
//!
//! ## 真的渲染了什么 / 明确没渲染什么
//!
//! **真做**：**音频片段**（`ClipContent::Audio`——CAS 资产字节 → 解码 → 采样率不一致时
//! `rubato` sinc 重采样 → 按 placement 的帧区间落位、按 placement 的 `muted` 与片段的
//! `gain_db` 门控、参与 [ARCH-PDC-001] 的延迟对齐）、MIDI 音符（起止 tick、音高、力度、
//! 微时值、**概率触发**、**连击**——见下）、placement 的 `start_tick`（缺省 0）、`muted`、音轨 `volume_db` /
//! `pan`（等功率 −3 dB）/ `mute` / `solo`（见下）、**未旁通设备的 `latency_samples`**
//! （PDC 对齐）、边增益 `gain_db`、Master 轨的 `volume_db`、确定性 TPDF 抖动与
//! 24-bit 量化、RIFF/RF64/BW64 容器与 `bext` 元数据、峰值归一化。
//!
//! **明确没做**（只要工程里真的出现，就会同时出现在响应的 `unsupported` 与
//! `unsupportedCounts` 里，**绝不静默**）：
//!
//! | 键 | 含义 |
//! | :--- | :--- |
//! | `audioClips` | **收窄后的口径**：仅当工程在 `assets` 索引里**声明**了这个资产、而会话 CAS 池里**没有它的字节**时才登记（`Domain::open_in_memory` 注入的会话就是这种形态）。此时该片段当静音，且响应 `data.audio.assets[].bytesPresent = false` 如实说明。读 `.yeban` 容器时索引与字节由 `store` 双向校验必然同时存在，因此这一项在真实容器工程上**不会**出现 |
//! | `deviceChainDsp` | 设备链的**参数**（滤波器/音色）没有求值；只有 `latency_samples` 进了 PDC |
//! | `externalPlugins` | `DeviceKind::ExternalInstrument/ExternalEffect` 没有宿主 |
//! | `automationLanes` | 自动化曲线没有求值（静态值也不代偿） |
//! | `clipLoopRepetition` | `loop_config` 的**重复**没有展开（只渲染摆放的第一遍）。登记口径 = "启用中**且**区间不是恰好 `0..duration_ticks`"（覆盖才静默；见下面的 `loop_requests_repetition`） |
//! | `noteSlide` | 滑音没有实现 |
//! | `notePitchBend` | 弯音曲线没有求值 |
//! | `noteLyrics` | 歌词/音素没有歌声合成 |
//! | `busTrackClips` | 非源节点（总线/主轨）上的片段没有渲染（只有源节点有音源） |
//! | `sidechainRouting` | 侧链边按普通音频边处理（`yeban-render` 的既有边界） |
//! | `masterPan` | Master 轨的 `pan` 没有应用（母带输出的声相由总线求和决定） |
//! | `sfzSampler` | 本模型版本（`DeviceKind`）里**没有** SFZ 设备变体；该能力不在本切片内 |
//!
//! "工程里有 MIDI 音符或音频片段"这一条是渲染的**前提**：完全没有可渲染内容（0 帧）
//! 时返回 `RENDER_FAILED`，而不是写一个 0 帧的文件冒充成功。
//!
//! ## 概率触发：判定的**唯一**入口在模型层（本线接线）
//!
//! `MidiNote::probability` 的语义是"这个音符有多大概率被触发"，规范
//! （`MODEL-AST-005`）要求它按 `rng_seed + note.id` **确定性**计算。模型层因此提供了
//! [`yeban_model::MidiNote::triggers`]（splitmix64 稳定哈希，无熵源、无 `unsafe`），
//! 实时引擎用它（`yeban-engine/src/snapshot.rs` 的 `project_schedules`）。
//!
//! 本渲染器此前**忽略**它：`probability < 1.0` 的音符被当作**必然触发**照常发声，
//! 同时把 `noteProbability` 登记进 `unsupported`。后果是同一份工程
//! **监听时听到的**与**导出的母带**不是同一个作品（引擎丢掉的音符在母带里响了）。
//!
//! 接线之后：
//!
//! - 判定只调用 [`yeban_model::MidiNote::triggers`]（本层不掷骰子、不写第二份哈希）；
//! - 未触发的音符**一位都不贡献**：不进排程，也不延长母带长度（它不发声，就没有时值）；
//! - `noteProbability` **不再**出现在 `unsupported` 里 —— 报一个已经做到的键就是假话；
//! - 判为不触发的音符数进 `data.sources[].notesSilenced` 与
//!   `data.probability.notesSilenced`，`: rng_seed` 也一并报出（复现判定的唯一熵输入）。
//!
//! 边界（如实登记）：概率触发**不产生**多个变体、不做多次渲染平均 —— 它只是
//! "这个音符这一遍响不响"的确定性裁决，与引擎同一口径。
//!
//! ## 连击（`ratchet`）：格点与实时引擎**同一裁决**（本线接线）
//!
//! `MidiNote::ratchet` 的语义是"把这一个音符的时值细分成几次触发"（`1..=16`，
//! `MODEL-AST-005`）。实时引擎真的展开它：`project_schedules` 用**整数除法**把时值
//! 等分成 `ratchet` 个脉冲（余数不补），逐脉冲排程（`yeban-engine/src/snapshot.rs`；
//! 该裁决登记在 `docs/ledger/engine-sound-notes.md`）。
//!
//! 本渲染器此前**忽略**它：`ratchet = 4` 的音符在母带里是**一个**长音，
//! 同时把 `noteRatchet` 登记进 `unsupported`。后果与概率触发那一条同类 ——
//! 同一份工程**监听时听到的**与**导出的母带**不是同一个作品（引擎的连击在母带里消失）。
//!
//! 接线之后：
//!
//! - 格点用引擎的那条公式：`step = (duration_ticks / ratchet).max(1)`，
//!   第 `p` 个脉冲起于音符起点 `+ p * step`，长为 `step`（余数不补）；
//! - 本层**不**另立一份"怎么分"的规则，也不改 `yeban_render` 的任何东西；
//! - `ratchet` 缺省或 `= 1` 时 `step == duration_ticks` ⇒ 排程结果与加这个旋钮之前
//!   **逐位相同**（判据钉住：默认夹具的摘要常量不变）；
//! - `noteRatchet` **不再**出现在 `unsupported` 里 —— 报一个已经做到的键就是假话；
//! - 展开读数进 `data.sources[].notesRatcheted` / `.ratchetPulses` 与
//!   `data.ratchet`，否则"连击没展开"与"这个音符本来就只有一响"在响应里长得一样。
//!
//! 边界（如实登记）：`ratchet` 只做**时值细分**，不引入多次渲染平均、不改音高/力度；
//! 脉冲之间没有静音间隔（整数除法把时值**铺满**，余数不补）—— 分离感只来自
//! 每个脉冲自带的起音/释音包络。对齐的**只是格点**：每个脉冲的帧区间仍走本渲染器
//! 既有的 `note_frame_span`（它只看音符自身的时值与微时值，**没有**摆放区间参数），
//! "音符是否被裁剪到摆放区间"这一条与引擎口径的差异**不在本切片内**，本线不改它。
//!
//! ## 采样率：请求率 ≠ 工程率**不再**是错误
//!
//! 上一版在"请求采样率 ≠ 工程采样率"时直接返回 `RENDER_FAILED`
//! （`data.unwired = "resampler"`）。本线把 `yeban-decode` 的 `rubato` sinc 接进来之后，
//! 这条拒绝**退役**了：
//!
//! - 渲染速率 = **请求的** `sampleRate`（母带就是那个率）；
//! - 每个音频资产的采样率如果与之不同，就先用
//!   `yeban_decode::resample_interleaved` 转过去（[`clip_math::RESAMPLER_SUMMARY`]）；
//! - MIDI 合成本来就是按渲染率算相位的，因此换率对音高没有影响。
//!
//! **绝不**用"改个采样率标签"或"丢帧"代替重采样：前者改时长、后者改音高，
//! 两者都是静默的错误音频。响应 `data.audio.assets[]` 逐条给出
//! `sourceSampleRate` / `targetSampleRate` / `resampled`。
//!
//! ## 输出路径规则（确定性，`dryRun` 与真调用共用同一份实现）
//!
//! ```text
//! arguments.path 存在        → 原样使用（拒绝空串）
//! 否则                       → <工程文件所在目录>/<工程文件 stem>.master.<format>
//!                              例: /x/demo.yeban + format=wav ⇒ /x/demo.master.wav
//!                              没有 stem 时用 "master": /x/.master.wav
//! ```
//!
//! 两条**安全护栏**（都返回契约内的 `INVALID_PARAMETER_RANGE`）：
//! 输出路径不得等于工程文件本身、也不得等于 `.yeban.lock`。
//!
//! ## `normalize` 的口径（含"全零信号"边界）
//!
//! **峰值归一化**，作用域是**抖动/量化之前的浮点母带**（也就是 `RenderOutput.samples`
//! 乘以母带增益之后的那一份）：
//!
//! ```text
//! peak = max |s[i]|          (非有限样本不计入)
//! gain = 1.0 / peak          (peak > 0)
//! 目标 = 1.0 (满量程), 因此归一化后峰值 == 满刻度
//! ```
//!
//! - **全零信号**（`peak == 0`）：`gain = 1.0`，即**不改动任何样本**，响应里
//!   `normalize.applied = false` + `normalize.note` 说明原因。把 0 放大到任何目标都是
//!   在制造一个不存在的信号，因此这里刻意不"成功"。
//! - 容差：f32 的除法与乘法各一次舍入，实测 `|peak_after − 1.0| < 1e-6`
//!   （相对误差上界 ~2⁻²⁴）。判据在 `tests/render_master.rs`。
//! - 归一化发生在抖动**之前**：抖动是"降位深"这一步的伴生 [ARCH-FMT-001]，
//!   对还没定标的信号抖动等于提前污染母带。
//!
//! ## `targetLufs` 的口径（本切片新增；"能测的那一半"）
//!
//! **响度目标不是一个新工具，是 `yeban_render_master` 的一个可选参数**，因为
//! 交付形态是"一次渲染 + 一份实测-对-目标的报告"，而渲染入口已经存在
//! （`AGENTS.md` §6.3：先 grep 交付形态，再下结论）。
//!
//! ```text
//! 不给 targetLufs  → 只报实测读数; verdict = "noTarget"   ← **默认路径, 逐字节不变**
//! 给 targetLufs    → 报读数 + 判定; |实测 − 目标| ≤ 0.5 LU ⇒ "pass"
//!                                            否则           ⇒ "fail" + RENDER_FAILED
//! 测不出（静音 / 采样率无核验系数）          ⇒ "unmeasurable" + RENDER_FAILED
//! ```
//!
//! 五件事写死在这里，都是**刻意的**：
//!
//! 1. **不做响度归一化**（不像 `normalize` 那样去改样本）：规范只要求"显示 LUFS"，
//!    没有任何文字要求按 LUFS 目标改变增益（见下方"规范沉默"）。做归一化会改变
//!    母带字节，而**默认路径必须逐字节不变**（`MUST-GATE-002` 的 L1 位级渲染）。
//! 2. **测量对象是抖动/量化之前的浮点母带**（与 `normalize` 同一作用域）：响度计量的
//!    对象是母带信号本身，抖动是"降位深"的伴生步骤。
//! 3. **测量在 [`RenderArtifact::build`] 里做**（`plan` 阶段，只读）；`apply` 只落盘 ⇒
//!    `dryRun` 与真调用拿到的是**同一份**判定。
//! 4. **未达标不写文件**：`RENDER_FAILED` 从 `build` 里返回，`apply` 根本不会被调用。
//! 5. **判定是只读的**：`bytes` / `sha256` / `masterDigest` 与不给目标时逐位相同，
//!    判据 `the_default_rendering_path_stays_bit_identical_with_a_loudness_target_attached`。
//!
//! ### 规范沉默处（**这是工程选择，必须登记**）
//!
//! - `docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md` §5.2 只要求
//!   "主母带总线提供标准的 **LUFS (Momentary / Short-term / Integrated)** 与
//!   响度范围（LRA）数值显示" —— 是**显示**要求，不是**目标**要求
//!   （原引 `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:1016`：该行号在
//!   架构文档的内嵌副本里，副本已由 `HD-57` 去重删除，正文唯一副本即上面那份）；
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` 通篇没有响度目标条目
//!   （`grep -n '响度\|loudness\|LUFS'` 只命中 `:107` 的"符合 ITU-R BS.1770-4 的
//!   真峰值多相插值砖墙限制器"）；
//! - `docs/ledger/mcp-tools-expansion-notes.md:224` 把"响度目标"挂在 `BASELINE-006`
//!   名下，而 `BASELINE-006` 的实测定义是
//!   "**AI 交互效率**：序列化 JSON 载荷 ≤ 4 KB，Token 开销中位数 ≤ 600"
//!   （`YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:361`）—— **它是 tokenizer 指标**，
//!   与 LUFS 无关。该注解见本文件所在工作线的台账（annotate，不改写）。
//!
//! 因此**容差**（[`TARGET_TOLERANCE_LU`] = 0.5 LU）与**判定规则**（不达标 ⇒
//! `RENDER_FAILED`）都是本切片的选择。要改它们得有人先裁决"谁来定容差"。
//!
//! ### 哪些采样率上可判
//!
//! K 加权系数只对 44.1 / 48 / 88.2 / 96 kHz 四档推导并核验过
//! （`crates/yeban-dsp/src/loudness.rs` 的 `K_WEIGHTING_SAMPLE_RATES_HZ`）。
//! `SampleRate` 枚举还有 192 kHz：那时**不硬套 48 kHz 系数**（那会给出错 0.8 dB 的
//! "说谎"读数），读数写 `null` + `measurementNote`，给了目标则判 `unmeasurable`。
//!
//! ## 可观测性（响应 `data` 里的数字全部是实测值）
//!
//! `frames` / `channels` / `sampleRate` / `bytes`（容器总字节）/ `headerBytes` /
//! `payloadBytes` / `sha256`（整份文件）/ `masterDigest`（母带样本的位级 SHA-256）/
//! `blocks` / `longestPathFrames` / `peak.before` / `peak.after` / `sourceNodes` /
//! `audio`（音频片段的**实测**事实：容器嗅探、声道布局、源/目标采样率、裁剪帧数、
//! 重采样口径）/ `loudness`（门限积分读数 + 对目标的判定）/ `unsupported`。
//! `dryRun` 返回同一批数字 + `wouldWrite`，**不落盘**。
//!
//! ## 错误映射（不发明新码，`ADR-0001 D25`）
//!
//! | 状况 | 出口 |
//! | :--- | :--- |
//! | `format` 不在白名单 / `sampleRate` 不在模型集合 / `normalize` 非布尔 / `path` 非字符串 / **`targetLufs` 非数字或不在 `[-70, 0]`** / 输出路径为工程或锁文件 | `INVALID_PARAMETER_RANGE`（带内） |
//! | 没有活跃工程 | `NO_ACTIVE_PROJECT`（带内） |
//! | 0 帧、超出帧数上限、路由图非法、缺音轨/片段 | `RENDER_FAILED`（带内，`data` 带原因与规范 ID） |
//! | **母带响度未达标或测不出** | `RENDER_FAILED`（`data.reason = "loudnessTargetMissed"`，`data` 带实测/目标/差值/容差） |
//! | 片段引用的资产既不在会话 CAS 池里、工程 `assets` 索引里也没有 | `RENDER_FAILED`（`data.reason = "assetMissing"`） |
//! | 会话 CAS 池里的字节与它声明的 SHA-256 不符（完整性破坏） | `RENDER_FAILED`（`data.reason = "assetHashMismatch"`） |
//! | 资产解码失败（坏/截断/不支持的容器） | `RENDER_FAILED`（`data.reason = "assetDecodeFailed"` + 分类）；解码器报的是 I/O 错时走 `IO_ERROR` |
//! | 素材声道数与母线声道数无法映射（>2 声道素材进立体声母线） | `RENDER_FAILED`（`data.reason = "assetChannelLayout"`） |
//! | 输出目录不可写 / 磁盘满 / 目标父目录不存在 | `IO_ERROR` / `DISK_FULL`（带内，来自 `store` 的既有映射） |
//!
//! **缺资产为什么不一律报错**：`assets` 索引（工程文档里的声明）与会话 CAS 池
//! （容器里的 `assets/{sha256}` 字节）在读 `.yeban` 容器时必然同时存在
//! （`store` 读写双向校验），而在**内存注入的会话**
//! （`Domain::open_in_memory` + `yeban_model::samples::filled_project()`）里，
//! 索引在、字节不在 —— 那是"这条会话种子路径本来就不携带资产载荷"，不是工程损坏。
//! 因此口径是：**索引也没有 ⇒ 明确的 `assetMissing` 错误**；索引有而字节不在 ⇒
//! 登记 `audioClips` + 静音（并在 `data.audio.assets[]` 里写 `bytesPresent = false`）。
//! 两条路都**不静默**。
//!
//! `ADR-0001 D43` 删掉裸 JSON 兼容读路径之后，**唯一**还能造出"有声明无载荷"的路径
//! 就是 `open_in_memory`（内存注入的判据夹具）。只要它还在，`audioClips` 就**不是**
//! 可删的死键 —— 删掉它会把一个"渲染必然失败"的工程交出去。触发条件与归属登记在
//! `docs/ledger/audio-render-notes.md` §needs-7。
//!
//! `BUSY` 在当前架构下**仍然不可达**：领域状态单线程同步，一次 `tools/call` 完整跑完
//! 才返回，不存在"已经有一个渲染在跑"的窗口。这是登记，不是遗漏。
//!
//! ## 幂等
//!
//! 不在这里实现第二套：[`crate::dispatch`] 的幂等缓存查询在工具执行**之前**，
//! 相同 `idempotencyKey` 的第二次调用根本到不了 `domain::execute` ⇒ 不重复渲染。
//! 判据用**输出文件的 inode** 与"删掉文件后重放同键不会重建"两个可观测量证明。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Map, Value};

use yeban_decode::{DecodeError, DecodeOptions, resample_interleaved};
use yeban_model::music::{RATCHET_MAX, RATCHET_MIN};
use yeban_model::{
    AssetHash, ClipContent, DeviceKind, EntityId, PPQ, RoutingKind, SampleRate, TrackV3,
    YebanProjectV1,
};
use yeban_render::dither::{BitDepth, quantize};
use yeban_render::render::{
    AudioSource, BlockContext, L1_BLOCK_SIZE, RenderError, RenderOptions, RenderPlan, db_to_linear,
    track_latencies,
};
// **响度计量的唯一实现**在 `yeban-dsp::loudness`（ITU-R BS.1770-4 的 K 加权 +
// 门限积分）。本模块**不复制**它一个表达式：走 `yeban-render` 的再导出，因为
// `scripts/gates/check_mcp_dependency_direction.py` 禁止 MCP 直接依赖 `yeban-dsp`
// （"不拖音频栈进 MCP"），而 `yeban-mcp -> yeban-render` 这条边早已存在。
use yeban_render::loudness::GatedLoudness;
use yeban_render::rf64::{Bext, ContainerKind, ContainerPlan, PcmFormat, write_container};
use yeban_render::rng::dither_rng_for;

use super::error::Fault;
use super::render_clip_math as clip_math;
use super::render_math as math;
use super::store;
use crate::tools::ErrorCode;

/// 允许的输出容器/编码。
pub const FORMATS: [&str; 3] = ["wav", "rf64", "bw64"];

/// 母带声道数（规范把 Master 定义为立体声总线；工程模型里没有声道数字段）。
pub const MASTER_CHANNELS: usize = 2;

/// 导出位深：**恒为 24-bit 整数 PCM**（[ARCH-FMT-001] 的内置高质量 TPDF 抖动路径）。
pub const OUTPUT_BIT_DEPTH: u16 = 24;

/// 归一化目标（满量程）。
pub const NORMALIZE_TARGET: f32 = 1.0;

// ---------------------------------------------------------------------------
// 响度目标（D46 的"能测的另一半"）：容差、词表与判据常量
// ---------------------------------------------------------------------------

/// 判定"母带达到响度目标"的**容差**（LU, 即 LUFS 上的差）。
///
/// ## 为什么是 ±0.5 LU（这是**规范沉默处的选择**，必须登记）
///
/// `docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md` §5.2 只要求"提供 LUFS
/// (Momentary / Short-term / Integrated) 与 LRA 数值显示"（原引
/// `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:1016`；该行号在架构文档的**内嵌副本**里，
/// 该副本已由 `HD-57` 去重删除，正文唯一副本即上面那份），
/// `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` 的通篇没有"响度目标"条目
/// （`grep -n '响度\|loudness\|LUFS' …ROADMAP.md` 只命中 `:107` 的限制器那句）。
/// 也就是说：**没有任何规范文字给出容差**。本常量因此是**工程选择**，不是规范读数。
///
/// 口径与理由：
/// - 单位是 **LU**（相对响度差），与 BS.1770 的读数差同量纲；
/// - 数值取 **0.5 LU**：它比该实现自身的数值噪声大三个数量级
///   （四档采样率上 997 Hz 标定点的残余偏差 ≤0.025 dB，见
///   `crates/yeban-dsp/src/loudness.rs:1091-1127`），又比广播实务里"±1 LU 以内算合规"
///   更严 —— 于是它**不会**因为实现误差而误判，也不会宽到让"差 0.9 LU"过关；
/// - 需要更宽/更严的调用方此刻**没有**参数可调：本切片刻意只加**一个**参数
///   （`targetLufs`），因为"容差也做成参数"会把一次判定变成二维旋钮，
///   而规范对判定规则本身沉默（见 `docs/ledger/mcp-tools-expansion-notes.md` 的注解）。
///   要放宽必须先有人裁决"谁来定容差"。
pub const TARGET_TOLERANCE_LU: f32 = 0.5;

/// `targetLufs` 的**可接受区间**（LUFS，闭区间）。
///
/// 下界 = BS.1770 的绝对门限 `Γa = −70 LUFS`（`crates/yeban-dsp/src/loudness.rs:94`
/// 的 `ABSOLUTE_GATE_LUFS`）：比它还低的目标在门限积分的定义里**不可达**
/// （所有块都会被绝对门限丢掉 ⇒ 读数恒为负无穷），因此拒绝而不是假装能判。
/// 上界 = `0.0 LUFS`（数字满量程；正值在 PCM 里不可达）。区间**端点是接受的**。
///
/// 这是**实现侧护栏**，来自两个已公开的常量（`ABSOLUTE_GATE_LUFS` 与满量程），
/// 不是从规范里抄来的数字。
pub const TARGET_LUFS_RANGE: [f32; 2] = [-70.0, 0.0];

/// 判定词表：达标。
pub const VERDICT_PASS: &str = "pass";
/// 判定词表：**未**达标（差值的绝对值超出容差）。
pub const VERDICT_FAIL: &str = "fail";
/// 判定词表：无法测量（纯静音母带，或该采样率上本实现没有核验过的 K 加权系数）。
pub const VERDICT_UNMEASURABLE: &str = "unmeasurable";
/// 判定词表：调用方没有给 `targetLufs`（**默认路径**；只报读数，不做判定）。
pub const VERDICT_NO_TARGET: &str = "noTarget";

/// 响度读数或判定**不可用**时的原因词（进响应 `loudness.measurementNote`）。
pub const NOTE_SILENT_MASTER: &str = "纯静音母带: 门限积分的读数为负无穷, 不是 0 LUFS";
/// 见 [`NOTE_SILENT_MASTER`]；这一条是采样率不在四档核验表里。
pub const NOTE_UNVERIFIED_SAMPLE_RATE: &str =
    "该采样率没有核验过的 K 加权系数 (44.1/48/88.2/96 kHz 四档), 拒绝硬套 48 kHz 系数";

// ---------------------------------------------------------------------------
/// 起音时长（毫秒）。
pub const ATTACK_MS: u32 = 5;

/// 释音时长（毫秒）。
pub const RELEASE_MS: u32 = 10;

/// 单次渲染的帧数上限：1 小时 @ 48 kHz。
///
/// 不是性能目标，而是一道**防呆闸门**：工程里一个错误的 `duration_ticks`
/// （或极慢的 BPM）可以在几秒内申请几十 GB 缓冲。超过即 `RENDER_FAILED`
/// 并如实报出请求帧数与上限。
pub const MAX_RENDER_FRAMES: u64 = 48_000 * 3_600;

/// 源节点没有可识别名字时的文件名 stem。
pub const DEFAULT_STEM: &str = "master";

/// 音频源种类（响应 `sources[].kind`）：只有 MIDI 片段的源。
pub const SOURCE_KIND: &str = "midi-synth-osc";

/// 音频源种类：只有音频片段的源。
pub const AUDIO_SOURCE_KIND: &str = "audio-clip";

/// 音频源种类：MIDI 片段与音频片段**都有**的源。
pub const MIXED_SOURCE_KIND: &str = "midi-synth-osc+audio-clip";

/// 音频源种类：既没有音符也没有可渲染的音频片段（静音源）。
pub const SILENT_SOURCE_KIND: &str = "silent";

/// 资产字节的只读提供者（`ARCH-SEC-003` 的 `assets/{sha256}` 会话 CAS 池）。
///
/// 抽成 trait 而不是直接吃 `&BTreeMap`：`yeban-mcp` 的会话池住在
/// [`super::Domain`] 的私有字段里，而判据需要能注入一份**手工构造**的池
/// （例如"池里的字节与它声明的哈希不符"这种完整性破坏，走
/// [`super::Domain::put_asset`] 是造不出来的——它总是用字节算出键）。
pub trait AssetStore {
    /// 取一份资产的原始字节；池里没有它时返回 `None`。
    fn asset(&self, hash: &AssetHash) -> Option<&[u8]>;
}

impl AssetStore for BTreeMap<AssetHash, Vec<u8>> {
    fn asset(&self, hash: &AssetHash) -> Option<&[u8]> {
        self.get(hash).map(Vec::as_slice)
    }
}

/// 一次已校验的渲染请求（**还没有渲染**）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderRequest {
    /// 输出格式下标（[`FORMATS`]）。
    pub format_index: usize,
    /// 采样率（模型层枚举）。
    pub sample_rate: SampleRate,
    /// 是否做峰值归一化。
    pub normalize: bool,
    /// 响度目标（LUFS）；`None` = **只报读数、不做判定**（也是默认路径）。
    ///
    /// 用「`f32` 的位型」参与 `Eq`（`#[derive(Eq)]` 对 `f32` 不成立），因此这里存
    /// 判定所需的**位型**，读出时用 [`target_lufs`](Self::target_lufs) 还原。
    pub target_lufs_bits: Option<u32>,
    /// 显式输出路径（`None` = 按缺省规则从工程路径派生）。
    pub path: Option<PathBuf>,
}

impl RenderRequest {
    /// 输出格式字符串。
    #[must_use]
    pub fn format(&self) -> &'static str {
        FORMATS.get(self.format_index).copied().unwrap_or("wav")
    }

    /// 响度目标（LUFS）；`None` = 未请求判定。
    #[must_use]
    pub fn target_lufs(&self) -> Option<f32> {
        self.target_lufs_bits.map(f32::from_bits)
    }

    /// 诚实描述这次请求（`dryRun` 与响应共用的 `request` 字段）。
    #[must_use]
    pub fn to_value(&self) -> Value {
        serde_json::json!({
            "format": self.format(),
            "sampleRate": self.sample_rate.hz(),
            "normalize": self.normalize,
            "targetLufs": self.target_lufs(),
            "targetToleranceLu": if self.target_lufs_bits.is_some() {
                Some(TARGET_TOLERANCE_LU)
            } else {
                None
            },
            "path": self.path.as_ref().map(|path| path.display().to_string()),
            "pathRule": if self.path.is_some() {
                "显式 path 参数"
            } else {
                "<工程文件 stem>.master.<format>, 与工程同目录"
            },
            "bitDepth": OUTPUT_BIT_DEPTH,
            "channels": MASTER_CHANNELS,
        })
    }

    /// 解析输出路径（确定性；`dryRun` 与真调用共用）。
    ///
    /// # Errors
    ///
    /// 显式 `path` 为空（`validate` 已拦，这里是第二道）。
    pub fn output_path(&self, project_path: &Path) -> Result<PathBuf, Fault> {
        if let Some(path) = &self.path {
            if path.as_os_str().is_empty() {
                return Err(Fault::domain(
                    ErrorCode::InvalidParameterRange,
                    "`path` 不能是空字符串",
                ));
            }
            return Ok(path.clone());
        }
        let stem = project_path
            .file_stem()
            .and_then(std::ffi::OsStr::to_str)
            .filter(|stem| !stem.is_empty())
            .unwrap_or(DEFAULT_STEM);
        Ok(store::parent_dir(project_path)
            .join(math::default_output_file_name(stem, self.format())))
    }
}

/// 母带的**实测**响度 + 对目标的判定（进响应 `data.loudness`）。
///
/// ## 它为什么不是"第二份测量标准"
///
/// 读数**只**来自 `yeban-dsp::loudness` 的 [`GatedLoudness`]（ITU-R BS.1770-4 的
/// K 加权 + `Γa = −70 LUFS` 绝对门限 + `Γr = −10 LU` 相对门限的**门限积分**），
/// 也就是 `yeban_query_engine_state` 的 `integratedLufs` 字段背后**同一个**计量器。
/// 本类型只加两件计量器没有的东西：**目标**与**比较**。
///
/// ## 它为什么**不**改渲染
///
/// 判定发生在抖动/量化**之前的浮点母带**上，且**只读**：它不改一个样本、不参与
/// 容器编码。[`RenderArtifact::bytes`] 与 `sha256`／`masterDigest` 因此与不给
/// `targetLufs` 时**逐字节相同**（判据
/// `the_default_rendering_path_stays_bit_identical_with_a_loudness_target_attached`）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoudnessTarget {
    /// 调用方给的目标（LUFS）；`None` = 默认路径（只报读数）。
    pub target_lufs: Option<f32>,
    /// 容差（LU）。
    pub tolerance_lu: f32,
    /// 实测的**门限积分**响度（LUFS）；测不出时 `None`。
    pub measured_lufs: Option<f32>,
    /// `measured − target`（LU）；两者有一缺就 `None`。
    pub delta_lu: Option<f32>,
    /// 判定（[`VERDICT_PASS`] / [`VERDICT_FAIL`] / [`VERDICT_UNMEASURABLE`] /
    /// [`VERDICT_NO_TARGET`]）。
    pub verdict: &'static str,
    /// 测不出读数时的原因（[`NOTE_SILENT_MASTER`] / [`NOTE_UNVERIFIED_SAMPLE_RATE`]）。
    pub measurement_note: Option<&'static str>,
}

impl LoudnessTarget {
    /// 测一段**抖动/量化之前**的浮点母带，并对目标判定。
    ///
    /// `samples` 是 `channels` 声道交织的 `f32`（本工具恒为立体声）。
    #[must_use]
    pub fn measure(
        sample_rate: u32,
        channels: usize,
        samples: &[f32],
        target: Option<f32>,
    ) -> Self {
        let (measured_lufs, measurement_note) = if channels != MASTER_CHANNELS {
            // 本工具只产出立体声；真走到这里说明调用点错了，如实报"测不出"而不是瞎量。
            (None, Some(NOTE_UNVERIFIED_SAMPLE_RATE))
        } else {
            let frames = samples.len() / channels;
            // 交错 → 两个声道的切片（计量器的口径是"每声道一段"，不是交错流）。
            let mut left = Vec::with_capacity(frames);
            let mut right = Vec::with_capacity(frames);
            for index in 0..frames {
                left.push(samples[index * channels]);
                right.push(samples[index * channels + 1]);
            }
            match GatedLoudness::integrated_stereo_at(sample_rate as f32, &left, &right) {
                Some(value) if value.is_finite() => (Some(value), None),
                // `Some(−∞)` = 没有任何 400 ms 块过绝对门限（纯静音母带）。
                Some(_) => (None, Some(NOTE_SILENT_MASTER)),
                // `None` = 该采样率没有核验过的 K 加权系数（本实现只内置四档）。
                None => (None, Some(NOTE_UNVERIFIED_SAMPLE_RATE)),
            }
        };
        let (delta_lu, verdict) = match (target, measured_lufs) {
            (None, _) => (None, VERDICT_NO_TARGET),
            (Some(_), None) => (None, VERDICT_UNMEASURABLE),
            (Some(want), Some(got)) => {
                let delta = got - want;
                let verdict = if delta.abs() <= TARGET_TOLERANCE_LU {
                    VERDICT_PASS
                } else {
                    VERDICT_FAIL
                };
                (Some(delta), verdict)
            }
        };
        Self {
            target_lufs: target,
            tolerance_lu: TARGET_TOLERANCE_LU,
            measured_lufs,
            delta_lu,
            verdict,
            measurement_note,
        }
    }

    /// 进响应的 JSON 形状（`null` 而不是 `Infinity`／`NaN` —— 两者都不是合法 JSON）。
    #[must_use]
    pub fn to_value(&self) -> Value {
        serde_json::json!({
            "targetLufs": self.target_lufs,
            "toleranceLu": self.tolerance_lu,
            "measuredIntegratedLufs": json_f32(self.measured_lufs),
            "deltaLu": json_f32(self.delta_lu),
            "verdict": self.verdict,
            "measured": self.measured_lufs.is_some(),
            "measurementNote": self.measurement_note,
            "algorithm": "ITU-R BS.1770-4 (K-weighted, gated integrated: Γa = −70 LUFS, Γr = −10 LU)",
            "implementation": "yeban-dsp::loudness::GatedLoudness (与 yeban_query_engine_state 的 integratedLufs 同一计量器)",
            "specId": "ARCH-UI-002",
            "specSilentOnTolerance": true,
        })
    }

    /// **判定未通过**时的带内错误（`RENDER_FAILED`，`ADR-0001` D25 里既有的 20 值之一）。
    ///
    /// 只在 `verdict` 是 [`VERDICT_FAIL`] 或 [`VERDICT_UNMEASURABLE`] 时调用。
    /// `RENDER_FAILED` 的语义是"渲染没有产出可交付的母带"，两句都字面成立：
    /// 母带**渲染成功了，但它不达标**（或根本测不出响度），调用方拿到的不是能发布的
    /// 东西。报文里逐字给出实测值、目标、差值与容差 —— 它不是一个只说"失败了"的码。
    #[must_use]
    pub fn failure(&self) -> Fault {
        let target = self.target_lufs.unwrap_or(f32::NAN);
        let message = match self.measured_lufs {
            Some(measured) => {
                let delta = self.delta_lu.unwrap_or(f32::NAN);
                format!(
                    "母带响度未达标: 实测 {measured} LUFS, 目标 {target} LUFS, 差 {delta} LU \
                     (容差 ±{} LU)",
                    self.tolerance_lu
                )
            }
            // 测不出 ⇒ 无法判定 ⇒ 也不能算"通过"。原因句**原样**给出, 不编一个数字。
            None => format!(
                "母带响度无法测量, 因此无法判定目标 {target} LUFS 是否达成: {}",
                self.measurement_note.unwrap_or("(未给出原因)")
            ),
        };
        Fault::domain_with_data(
            ErrorCode::RenderFailed,
            message,
            serde_json::json!({
                "reason": "loudnessTargetMissed",
                "measuredIntegratedLufs": json_f32(self.measured_lufs),
                "targetLufs": target,
                "deltaLu": json_f32(self.delta_lu),
                "toleranceLu": self.tolerance_lu,
                "measurementNote": self.measurement_note,
                "algorithm": "ITU-R BS.1770-4",
            }),
        )
    }
}

/// `f32` → JSON：非有限值写 `null`（`serde_json` 会把 `Infinity`/`NaN` 静默写成 `null`
/// 并留下一个"看起来是数字、其实是空"的字段；这里显式做，口径写在一处）。
#[must_use]
fn json_f32(value: Option<f32>) -> Value {
    match value {
        Some(number) if number.is_finite() => Value::from(number),
        _ => Value::Null,
    }
}

/// 校验渲染参数。
///
/// # Errors
///
/// - 未知 `format` → `INVALID_PARAMETER_RANGE`（带白名单）；
/// - `sampleRate` 不在模型层允许集合 → `INVALID_PARAMETER_RANGE`；
/// - `normalize` 不是布尔 / `path` 不是非空字符串 → `INVALID_PARAMETER_RANGE`；
/// - `targetLufs` 不是有限数、或不在 [`TARGET_LUFS_RANGE`] 内 →
///   `INVALID_PARAMETER_RANGE`（带区间）。
pub fn validate(arguments: &Map<String, Value>) -> Result<RenderRequest, Fault> {
    let format = arguments
        .get("format")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let format_index = FORMATS
        .iter()
        .position(|known| *known == format)
        .ok_or_else(|| {
            Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!("不支持的输出格式 `{format}`"),
                serde_json::json!({ "supportedFormats": FORMATS }),
            )
        })?;
    let sample_rate_hz = arguments
        .get("sampleRate")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| {
            Fault::domain(
                ErrorCode::InvalidParameterRange,
                "`sampleRate` 必须是 32 位无符号整数",
            )
        })?;
    let sample_rate = SampleRate::from_hz(sample_rate_hz)
        .map_err(|error| super::error::from_model("采样率校验", &error))?;
    let normalize = match arguments.get("normalize") {
        None => false,
        Some(Value::Bool(flag)) => *flag,
        Some(other) => {
            return Err(Fault::domain(
                ErrorCode::InvalidParameterRange,
                format!("`normalize` 必须是布尔值, 实际收到 {other}"),
            ));
        }
    };
    let target_lufs_bits = match arguments.get("targetLufs") {
        None | Some(Value::Null) => None,
        Some(Value::Number(number)) => {
            // `as_f64` 覆盖整数与浮点两种 JSON 形状；`f32` 是有意收窄
            // （响应里报出的也是 `f32`，判定与读数用同一个宽度）。
            // `as_f64` 对 `serde_json::Number` 恒有值；万一没有就按非法处理。
            let value = number.as_f64().map_or(f32::NAN, |raw| raw as f32);
            let [low, high] = TARGET_LUFS_RANGE;
            if !value.is_finite() || value < low || value > high {
                return Err(Fault::domain_with_data(
                    ErrorCode::InvalidParameterRange,
                    format!("`targetLufs` 必须在 {low}..={high} LUFS 之间且有限, 实际收到 {value}"),
                    serde_json::json!({
                        "minLufs": low,
                        "maxLufs": high,
                        "reason": "低于 BS.1770 的绝对门限 Γa 的读数不可达; 高于 0 LUFS 在 PCM 里不可达",
                    }),
                ));
            }
            Some(value.to_bits())
        }
        Some(other) => {
            return Err(Fault::domain(
                ErrorCode::InvalidParameterRange,
                format!("`targetLufs` 必须是数字, 实际收到 {other}"),
            ));
        }
    };
    let path = match arguments.get("path") {
        None => None,
        Some(Value::String(text)) if !text.trim().is_empty() => Some(PathBuf::from(text)),
        Some(other) => {
            return Err(Fault::domain(
                ErrorCode::InvalidParameterRange,
                format!("`path` 必须是非空字符串, 实际收到 {other}"),
            ));
        }
    };
    Ok(RenderRequest {
        format_index,
        sample_rate,
        normalize,
        target_lufs_bits,
        path,
    })
}

/// 渲染请求的"工程前提"检查（无活跃工程 → `NO_ACTIVE_PROJECT`）。
///
/// 单独成函数是为了让"参数校验"与"会话前提"两条判据可以分别变红。
///
/// # Errors
///
/// 没有活跃工程。
pub fn require_project(project: Option<&YebanProjectV1>) -> Result<(), Fault> {
    if project.is_none() {
        return Err(Fault::domain(
            ErrorCode::NoActiveProject,
            "没有活跃工程, 无法渲染母带",
        ));
    }
    Ok(())
}

/// 一个源节点的渲染报告（进响应 `data.sources`）。
#[derive(Clone, Debug)]
pub struct SourceReport {
    /// 路由图节点身份（= 音轨身份）。
    pub node: EntityId,
    /// 音轨名（人眼可读）。
    pub track: String,
    /// 音频源种类（见 [`SOURCE_KIND`] 一族）。
    pub kind: &'static str,
    /// 是否可闻（`mute` / `solo` 判定之后的结论）。
    pub audible: bool,
    /// 排程的音符数。
    pub notes: u64,
    /// **真的参与渲染**的音频片段摆放数（不含被静音或被登记的）。
    pub audio_clips: u64,
    /// 因为资产字节不在会话池里而**没有**渲染的音频片段摆放数。
    pub audio_clips_unrendered: u64,
    /// 因为所在音轨被 `mute` / `solo` 门控而没有渲染的音频片段摆放数
    /// （这类摆放**不做解码** —— 静音轨的资产坏了不该让整份母带导出失败，
    /// 但它必须被数出来，而不是看起来"这段工程里没有音频片段"）。
    pub audio_clips_gated: u64,
    /// 被 [`yeban_model::MidiNote::triggers`] 的**确定性**概率判定判为"不触发"、
    /// 因而不发声的音符数 [MODEL-AST-005, ARCH-DET-001]。
    ///
    /// 这是"母带里少了的音符"的**可见**读数：没有它，一个 `probability: 0.0`
    /// 的音符与"这个音符根本不存在"在响应里长得一模一样。
    pub notes_silenced: u64,
    /// 被**连击**细分的音符数（`ratchet > 1` 的、且真的进了排程的音符）。
    pub notes_ratcheted: u64,
    /// 这些连击音符一共产生了多少个脉冲（每个 ≥ 2）。
    ///
    /// 没有它，"连击没展开"（一个脉冲）与"这个音符本来就只有一响"在响应里
    /// 长得一模一样 —— 与 `notes_silenced` 同一条理由。
    pub ratchet_pulses: u64,
    /// 该源贡献的时间轴末端（tick）。
    pub end_tick: u64,
}

impl SourceReport {
    /// 进响应的 JSON 形状。
    #[must_use]
    pub fn to_value(&self) -> Value {
        serde_json::json!({
            "node": self.node.to_canonical_string(),
            "track": self.track.clone(),
            "kind": self.kind,
            "audible": self.audible,
            "notes": self.notes,
            "audioClips": self.audio_clips,
            "audioClipsUnrendered": self.audio_clips_unrendered,
            "audioClipsGated": self.audio_clips_gated,
            "notesSilenced": self.notes_silenced,
            "notesRatcheted": self.notes_ratcheted,
            "ratchetPulses": self.ratchet_pulses,
            "endTick": self.end_tick,
        })
    }
}

/// 一个音频资产的**实测**事实（进响应 `data.audio.assets`）。
///
/// 全部字段都是"解出来/量出来"的值，没有一个是按工程声明抄的
/// （唯一的例外是 `declared_bytes`，它是索引里的声明，刻意与实测并列，好让
/// "声明与实际不符"这件事**可见**）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioAssetReport {
    /// CAS 键（`assets/{sha256}`）。
    pub hash: String,
    /// 会话池里有没有它的字节。
    pub bytes_present: bool,
    /// 工程 `assets` 索引里有没有它的声明。
    pub declared_in_index: bool,
    /// 魔数嗅探出的容器形态（**报告用**，不是裁决）。
    pub container: &'static str,
    /// 解码器实际产出的样本格式（`S16`/`F32`/…；未解码时为 `"unknown"`）。
    pub pcm_format: &'static str,
    /// 素材声道数。
    pub source_channels: u16,
    /// 素材采样率 (Hz)。
    pub source_sample_rate: u32,
    /// 素材帧数。
    pub source_frames: u64,
    /// 渲染管线采样率 (Hz)。
    pub target_sample_rate: u32,
    /// 是否真的做了重采样。
    pub resampled: bool,
    /// 声道布局（[`clip_math::ChannelLayout::name`]）。
    pub channel_layout: &'static str,
    /// 编码器前置延迟（容器上报表，帧）。
    pub encoder_delay_frames: Option<u32>,
    /// 编码器尾部填充（容器上报表，帧）。
    pub encoder_padding_frames: Option<u32>,
    /// 实际**丢掉**的帧数（`delay + padding` 中真正落在素材内的部分）。
    pub trimmed_frames: u64,
    /// 进入渲染的帧数（裁剪之后）。
    pub rendered_frames: u64,
    /// 原始资产字节数。
    pub bytes: usize,
    /// 引用这个资产的**参与渲染**的片段摆放数。
    pub clips: u64,
}

impl AudioAssetReport {
    /// 进响应的 JSON 形状。
    #[must_use]
    pub fn to_value(&self) -> Value {
        serde_json::json!({
            "hash": self.hash.clone(),
            "bytesPresent": self.bytes_present,
            "declaredInIndex": self.declared_in_index,
            "container": self.container,
            "pcmFormat": self.pcm_format,
            "sourceChannels": self.source_channels,
            "sourceSampleRate": self.source_sample_rate,
            "sourceFrames": self.source_frames,
            "targetSampleRate": self.target_sample_rate,
            "resampled": self.resampled,
            "channelLayout": self.channel_layout,
            "encoderDelayFrames": self.encoder_delay_frames,
            "encoderPaddingFrames": self.encoder_padding_frames,
            "trimmedFrames": self.trimmed_frames,
            "renderedFrames": self.rendered_frames,
            "bytes": self.bytes,
            "clips": self.clips,
        })
    }
}

/// 渲染产物：**已经编码好的容器字节** + 全部实测数字。
///
/// 它由 [`build`] 产出（只读计算），由 `domain::apply` 落盘。
/// 刻意**手动实现 `Debug`**：`bytes` 有几十万字节，`#[derive(Debug)]` 会在任何
/// `{:?}` 里把日志淹掉。
pub struct RenderArtifact {
    /// 已校验的请求（响应里原样回显）。
    pub request: RenderRequest,
    /// 产物路径（原子落盘的目标）。
    pub path: PathBuf,
    /// 实际容器种类（`wav` 在超过 4 GiB 时会被 `yeban-render` 升级为 RF64）。
    pub container: ContainerKind,
    /// 采样率 (Hz)。
    pub sample_rate: u32,
    /// 声道数。
    pub channels: usize,
    /// 位深。
    pub bit_depth: u16,
    /// 帧数。
    pub frames: u64,
    /// 处理块数。
    pub blocks: u64,
    /// 处理块大小。
    pub block_size: usize,
    /// 头部字节数（含容器前缀与全部 chunk 头）。
    pub header_bytes: usize,
    /// 音频负载字节数（`frames * channels * bit_depth / 8`）。
    pub payload_bytes: usize,
    /// 整份文件字节（头部 + 负载 + 偶数补位）。
    pub bytes: Vec<u8>,
    /// 整份文件的 SHA-256（十六进制）。
    pub sha256: String,
    /// 母带**样本**的位级 SHA-256（对每个 `f32` 的 IEEE-754 位型）。
    pub master_digest: String,
    /// 最长延迟关键路径（帧）[ARCH-PDC-001]。
    pub longest_path_frames: u32,
    /// 渲染计划里的节点数（剪枝后）。
    pub node_count: usize,
    /// 源节点。
    pub source_nodes: Vec<EntityId>,
    /// 每个源节点的报告。
    pub sources: Vec<SourceReport>,
    /// **真的参与渲染**的音频片段摆放数（全部源节点合计）。
    pub audio_clips: u64,
    /// 被**确定性**概率判定判为"不触发"、因而不发声的 MIDI 音符数
    /// （全部源节点合计）[MODEL-AST-005, ARCH-DET-001]。
    ///
    /// 判定的**唯一**入口是 [`yeban_model::MidiNote::triggers`]；实时引擎
    /// (`yeban-engine/src/snapshot.rs`) 用的是同一个入口，因此"监听时听到的"
    /// 与"导出的母带"在这一点上一致。
    pub notes_silenced: u64,
    /// 被**连击**细分的 MIDI 音符数（全部源节点合计，`ratchet > 1` 的排程音符）。
    ///
    /// 格点与实时引擎同一裁决（整数除法等分时值，余数不补）；
    /// `ratchet` 的合法范围 `1..=16` 由模型层 `MidiNote::validate` 把关。
    pub notes_ratcheted: u64,
    /// 连击一共产生了多少个脉冲（全部源节点合计）。
    pub ratchet_pulses: u64,
    /// 这次渲染**实际使用**的工程随机种子（`YebanProjectV1::rng_seed`）。
    ///
    /// 它是概率触发判定的唯一熵输入 ⇒ 响应必须报出来，否则"为什么这个音符
    /// 没发声"无法从响应本身复现。
    pub rng_seed: u64,
    /// 每个被引用的音频资产的实测事实（键序 = 哈希字典序）。
    pub audio_assets: Vec<AudioAssetReport>,
    /// 归一化前的峰值。
    pub peak_before: f32,
    /// 归一化后的峰值（未请求归一化时等于 `peak_before`）。
    pub peak_after: f32,
    /// 归一化是否真的改变了样本。
    pub normalize_applied: bool,
    /// 归一化的补充说明（全零信号等边界）。
    pub normalize_note: Option<&'static str>,
    /// **实测**的门限积分响度 + 对 `targetLufs` 的判定。
    ///
    /// 它在**抖动/量化之前**的浮点母带上测量（与响度计量"作用在母带信号上"的口径
    /// 一致），且只读 ⇒ 不改变 [`Self::bytes`]。默认路径（不给 `targetLufs`）也报读数，
    /// 只是 [`VERDICT_NO_TARGET`]。
    pub loudness: LoudnessTarget,
    /// `bext` 的 `OriginatorReference`（工程 ULID，`ARCH-FMT-001` 要求的映射）。
    pub originator_reference: String,
    /// `bext` 的 `OriginationDate`。
    pub origination_date: String,
    /// `bext` 的 `OriginationTime`。
    pub origination_time: String,
    /// `bext` 的 `CodingHistory`（写进文件的那一份）。
    pub coding_history: String,
    /// 工程内容摘要（渲染输入的指纹，用于溯源）。
    pub project_digest: String,
    /// 渲染时刻（注入时钟的 Unix 毫秒）。
    pub rendered_at_unix_ms: u64,
    /// 明确没渲染的部分（键序字典序，来自 `BTreeSet`）。
    pub unsupported: BTreeSet<&'static str>,
    /// 每个"没渲染"部分的出现次数。
    pub unsupported_counts: BTreeMap<&'static str, u64>,
}

impl core::fmt::Debug for RenderArtifact {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RenderArtifact")
            .field("path", &self.path)
            .field("container", &self.container)
            .field("sample_rate", &self.sample_rate)
            .field("channels", &self.channels)
            .field("frames", &self.frames)
            .field("blocks", &self.blocks)
            .field("header_bytes", &self.header_bytes)
            .field("payload_bytes", &self.payload_bytes)
            .field("file_bytes", &self.bytes.len())
            .field("sha256", &self.sha256)
            .field("unsupported", &self.unsupported)
            .finish_non_exhaustive()
    }
}

impl RenderArtifact {
    /// 容器种类的规范字符串。
    #[must_use]
    pub const fn container_name(&self) -> &'static str {
        match self.container {
            ContainerKind::Riff => "RIFF",
            ContainerKind::Rf64 => "RF64",
            ContainerKind::Bw64 => "BW64",
        }
    }

    /// 实测数字（`dryRun` 预览与真调用响应共用的主体）。
    #[must_use]
    pub fn details(&self) -> Map<String, Value> {
        let mut map = Map::new();
        map.insert(
            "path".to_owned(),
            Value::from(self.path.display().to_string()),
        );
        map.insert("format".to_owned(), Value::from(self.request.format()));
        map.insert("container".to_owned(), Value::from(self.container_name()));
        map.insert("sampleRate".to_owned(), Value::from(self.sample_rate));
        map.insert("channels".to_owned(), Value::from(self.channels));
        map.insert("bitDepth".to_owned(), Value::from(self.bit_depth));
        map.insert("frames".to_owned(), Value::from(self.frames));
        map.insert(
            "durationSeconds".to_owned(),
            Value::from(self.frames as f64 / f64::from(self.sample_rate)),
        );
        map.insert("blocks".to_owned(), Value::from(self.blocks));
        map.insert("blockSize".to_owned(), Value::from(self.block_size));
        map.insert("bytes".to_owned(), Value::from(self.bytes.len()));
        map.insert("headerBytes".to_owned(), Value::from(self.header_bytes));
        map.insert("payloadBytes".to_owned(), Value::from(self.payload_bytes));
        map.insert("sha256".to_owned(), Value::from(self.sha256.clone()));
        map.insert(
            "masterDigest".to_owned(),
            Value::from(self.master_digest.clone()),
        );
        map.insert(
            "longestPathFrames".to_owned(),
            Value::from(self.longest_path_frames),
        );
        map.insert(
            "latencySource".to_owned(),
            Value::from("DeviceDefinition::latency_samples (track_latencies)"),
        );
        map.insert("nodeCount".to_owned(), Value::from(self.node_count));
        map.insert(
            "sourceNodes".to_owned(),
            Value::Array(
                self.source_nodes
                    .iter()
                    .map(|node| Value::from(node.to_canonical_string()))
                    .collect(),
            ),
        );
        map.insert(
            "sources".to_owned(),
            Value::Array(self.sources.iter().map(SourceReport::to_value).collect()),
        );
        // 概率触发的**实测**读数：`probability < 1.0` 的音符里有多少被判为不触发。
        // 这是"母带里少了的音符"的可见证据 —— 判定走模型层唯一入口
        // (`MidiNote::triggers`)，与实时引擎同一份实现 [MODEL-AST-005]。
        map.insert(
            "probability".to_owned(),
            serde_json::json!({
                "decision": "MidiNote::triggers(project.rng_seed)",
                "specId": "MODEL-AST-005",
                "adr": "ADR-0001 (ARCH-DET-001)",
                "notesSilenced": self.notes_silenced,
                "rngSeed": self.rng_seed,
            }),
        );
        map.insert(
            "ratchet".to_owned(),
            serde_json::json!({
                // 格点公式与实时引擎 `project_schedules` 是同一条裁决：
                // 整数除法等分时值（余数不补），脉冲 p 起于 `p * step`。
                "grid": "step = (duration_ticks / ratchet).max(1); pulse p starts at p * step",
                "specId": "MODEL-AST-005",
                "engineParity": "yeban-engine/src/snapshot.rs 的 project_schedules",
                // 规范只给了 `ratchet: 1..=16`；"怎么分"是**登记在本线台账里的本地裁决**
                // （不是 ADR），因此如实叫 ruling 而不是 adr。
                "ruling": "docs/ledger/engine-sound-notes.md 的 ratchet 语义 (整数除法等分时值, 余数不补)",
                "minPerNote": RATCHET_MIN,
                "notesExpanded": self.notes_ratcheted,
                "pulses": self.ratchet_pulses,
                "maxPerNote": RATCHET_MAX,
            }),
        );
        map.insert(
            "audio".to_owned(),
            serde_json::json!({
                "wired": true,
                "clipsRendered": self.audio_clips,
                "assetsReferenced": self.audio_assets.len(),
                "assets": self
                    .audio_assets
                    .iter()
                    .map(AudioAssetReport::to_value)
                    .collect::<Vec<_>>(),
                "supported": clip_math::SUPPORTED_ASSET_SUMMARY,
                "unsupported": clip_math::UNSUPPORTED_ASSET_SUMMARY,
                // 重采样口径 (D26): 写清**用的是哪一种**, 而不是只说"已重采样"。
                "resampler": {
                    "method": clip_math::RESAMPLER_SUMMARY,
                    "specId": "ARCH-DSP-002",
                    "adr": "ADR-0001 D26",
                },
                "bitExactness": {
                    // D32: 重采样含超越函数, 因此跨架构只承诺数值预算, 不承诺逐位。
                    "identityRate": "bit-exact (无滤波, 逐位透传)",
                    "resampled": "同架构同工具链逐位; 跨架构按 ADR-0001 D32 给数值预算 (不承诺位级)",
                },
            }),
        );
        map.insert(
            "peak".to_owned(),
            serde_json::json!({
                "before": self.peak_before,
                "after": self.peak_after,
                "target": NORMALIZE_TARGET,
                "unit": "full-scale",
            }),
        );
        map.insert(
            "normalize".to_owned(),
            serde_json::json!({
                "requested": self.request.normalize,
                "applied": self.normalize_applied,
                "target": NORMALIZE_TARGET,
                "domain": "抖动/量化之前的浮点母带",
                "note": self.normalize_note,
            }),
        );
        // 响度：**永远**给出读数（默认路径也一样），判定只在给了 `targetLufs` 时有意义。
        map.insert("loudness".to_owned(), self.loudness.to_value());
        map.insert("atomic".to_owned(), Value::from(true));
        map.insert(
            "strategy".to_owned(),
            Value::from("同目录 .tmp-{ulid} + File::sync_all + rename (ARCH-SEC-004)"),
        );
        map.insert("dither".to_owned(), Value::from("TPDF (ARCH-FMT-001)"));
        map.insert(
            "bwf".to_owned(),
            serde_json::json!({
                "originator": "Yeban DAW",
                "originatorReference": self.originator_reference.clone(),
                "originationDate": self.origination_date.clone(),
                "originationTime": self.origination_time.clone(),
                "codingHistory": self.coding_history.clone(),
                // EBU R128 响度没有测量 ⇒ 写哨兵 `UNKNOWN`, 绝不写一个假的 LUFS 读数。
                "loudness": "unknown (bext v2 哨兵)",
            }),
        );
        map.insert(
            "unsupported".to_owned(),
            Value::Array(
                self.unsupported
                    .iter()
                    .map(|key| Value::from(*key))
                    .collect(),
            ),
        );
        map.insert(
            "unsupportedCounts".to_owned(),
            Value::Object(
                self.unsupported_counts
                    .iter()
                    .map(|(key, count)| ((*key).to_owned(), Value::from(*count)))
                    .collect(),
            ),
        );
        map.insert(
            "projectDigest".to_owned(),
            Value::from(self.project_digest.clone()),
        );
        map.insert(
            "renderedAtUnixMs".to_owned(),
            Value::from(self.rendered_at_unix_ms),
        );
        map
    }

    /// `dryRun` 预览：同一批实测数字 + `wouldWrite`（**不落盘**）。
    #[must_use]
    pub fn preview(&self) -> Value {
        let mut map = self.details();
        map.insert("renderer".to_owned(), Value::from("yeban-render"));
        map.insert("wired".to_owned(), Value::from(true));
        map.insert("writesFile".to_owned(), Value::from(true));
        map.insert("request".to_owned(), self.request.to_value());
        map.insert(
            "wouldWrite".to_owned(),
            serde_json::json!({
                "path": self.path.display().to_string(),
                "format": self.request.format(),
                "container": self.container_name(),
                "frames": self.frames,
                "bytes": self.bytes.len(),
                "sha256": self.sha256.clone(),
            }),
        );
        Value::Object(map)
    }

    /// 真调用响应的 `data`。
    #[must_use]
    pub fn response_data(&self) -> Value {
        let mut map = self.details();
        map.insert("rendered".to_owned(), Value::from(true));
        map.insert("request".to_owned(), self.request.to_value());
        Value::Object(map)
    }
}

/// 从工程构造一份**已编码**的母带产物（只读、不落盘）。
///
/// 这是本模块唯一的入口：`plan_render_master` 调它拿 [`RenderArtifact`]，
/// `domain::apply` 只负责把 `artifact.bytes` 原子写到 `artifact.path`。
///
/// `assets` 是会话 CAS 池的只读视图（`assets/{sha256}`）；音频片段的字节只从这里来。
///
/// # Errors
///
/// - `RENDER_FAILED`：0 帧、超出帧数上限、路由图非法、源节点没有对应音轨、
///   摆放引用了不存在的片段、容器编码失败、**资产缺失/哈希不符/解码失败/声道布局不支持**；
/// - `IO_ERROR`：资产解码时报的是 I/O 错；
/// - `INVALID_PARAMETER_RANGE`：输出路径是工程文件或锁文件。
pub fn build(
    project: &YebanProjectV1,
    project_path: &Path,
    request: &RenderRequest,
    now_ms: u64,
    assets: &dyn AssetStore,
) -> Result<RenderArtifact, Fault> {
    let sample_rate = request.sample_rate.hz();
    // 1. 输出路径 + 两条安全护栏。
    let path = request.output_path(project_path)?;
    guard_output_path(&path, project_path)?;
    // 2. 延迟表: 唯一来源是 DeviceDefinition::latency_samples [ARCH-PDC-001]。
    //    音频片段**不自建第二来源** —— 模型里没有 per-asset 延迟字段,
    //    片段所在音轨的设备链延迟就是它的全部延迟贡献。
    let latencies = track_latencies(&project.tracks);
    // 3. 探针编译: 用 frames=1 取得"剪枝后的可达子图 + 源节点集合"。
    //    刻意复用渲染器的剪枝/分层逻辑, 而不是在这里复制一份图算法(两份必然漂移)。
    let probe = RenderOptions::l1(1, MASTER_CHANNELS, sample_rate, project.rng_seed);
    let probe_plan = RenderPlan::compile_with_latencies(
        &project.routing_graph,
        project.master_bus_track_id,
        probe,
        &latencies,
    )
    .map_err(|error| render_error("渲染计划探针编译", &error))?;
    let nodes = probe_plan.nodes();
    let source_nodes: Vec<EntityId> = nodes
        .iter()
        .copied()
        .filter(|node| is_source_node(&probe_plan, *node))
        .collect();
    // 4. 组装音源 + 采集"明确没渲染"的部分 + 得到时间轴末端。
    let mut unsupported: BTreeSet<&'static str> = BTreeSet::new();
    let mut counts: BTreeMap<&'static str, u64> = BTreeMap::new();
    let any_solo = project.tracks.values().any(|track| track.solo);
    if project
        .routing_graph
        .edges
        .values()
        .any(|edge| edge.kind == RoutingKind::Sidechain)
    {
        note_unsupported(&mut unsupported, &mut counts, "sidechainRouting");
    }
    // 资产上下文: 每个资产**解码/重采样一次**, 供所有音轨共享（`Arc<[f32]>` 只读）。
    let mut asset_cache: BTreeMap<AssetHash, Option<Arc<PreparedAsset>>> = BTreeMap::new();
    let mut asset_reports: BTreeMap<AssetHash, AudioAssetReport> = BTreeMap::new();
    let mut registry: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
    let mut pending: Vec<(EntityId, TrackSource)> = Vec::new();
    let mut reports: Vec<SourceReport> = Vec::new();
    let mut end_tick = 0u64;
    for &node in &source_nodes {
        let track = project.tracks.get(&node).ok_or_else(|| {
            render_failed(
                "路由图里的源节点没有对应的音轨, 无法知道该渲染什么",
                serde_json::json!({ "node": node.to_canonical_string() }),
            )
        })?;
        let (source, report) = build_source(
            project,
            track,
            sample_rate,
            any_solo,
            assets,
            &mut asset_cache,
            &mut asset_reports,
            &mut unsupported,
            &mut counts,
        )?;
        end_tick = end_tick.max(report.end_tick);
        reports.push(report);
        pending.push((node, source));
    }
    // 5. 非源节点(总线/主轨)上的片段: 不渲染, 但**明确登记**, 不静默丢。
    for &node in &nodes {
        if source_nodes.contains(&node) {
            continue;
        }
        if let Some(track) = project.tracks.get(&node)
            && audible_end_tick(track).is_some()
        {
            note_unsupported(&mut unsupported, &mut counts, "busTrackClips");
        }
    }
    // 7. 帧数: 由时间轴末端与工程量纲唯一决定。
    let frames = math::ticks_to_frames(end_tick, PPQ, project.bpm, sample_rate);
    if frames == 0 {
        return Err(render_failed(
            "工程里没有可渲染的内容 (0 帧)",
            serde_json::json!({
                "endTick": end_tick,
                "sourceNodes": source_nodes
                    .iter()
                    .map(|node| node.to_canonical_string())
                    .collect::<Vec<_>>(),
                "hint": "需要至少一个未静音的 MIDI 音符或可渲染的音频片段摆放",
            }),
        ));
    }
    if frames > MAX_RENDER_FRAMES {
        return Err(render_failed(
            "渲染帧数超出单次上限 (1 小时 @ 48 kHz)",
            serde_json::json!({ "frames": frames, "maxFrames": MAX_RENDER_FRAMES }),
        ));
    }
    // 7b. 音频片段的帧区间在**知道总帧数之后**才能夹住（避免区间越过母带末端）。
    //     这一步只改内存里的 `TrackSource`，不重新解码。
    for (node, mut source) in pending {
        source.clamp_to(frames);
        // 显式标注类型: 让 `Box<TrackSource> -> Box<dyn AudioSource>` 的强制转换
        // 出现在插槽类型已知的位置, 而不是依赖 `insert` 的参数推断。
        let boxed: Box<dyn AudioSource> = Box::new(source);
        registry.insert(node, boxed);
    }
    // 8. 真编译 + 真执行。
    let options = RenderOptions::l1(frames, MASTER_CHANNELS, sample_rate, project.rng_seed);
    let mut plan = RenderPlan::compile_with_latencies(
        &project.routing_graph,
        project.master_bus_track_id,
        options,
        &latencies,
    )
    .map_err(|error| render_error("渲染计划编译", &error))?;
    let mut output = plan
        .execute(registry)
        .map_err(|error| render_error("渲染执行", &error))?;
    // 9. Master 轨增益 (在源侧无法表达: 它是总线节点)。pan 无法应用 ⇒ 登记。
    if let Some(master) = project.tracks.get(&project.master_bus_track_id) {
        let master_gain = if master.mute {
            0.0
        } else {
            db_to_linear(master.volume_db)
        };
        if master_gain != 1.0 {
            math::scale_in_place(&mut output.samples, master_gain);
        }
        if master.pan != 0.0 {
            note_unsupported(&mut unsupported, &mut counts, "masterPan");
        }
    }
    // 10. 峰值归一化(抖动之前)。
    let peak_before = math::peak_of(&output.samples);
    let mut normalize_applied = false;
    let mut normalize_note = None;
    if request.normalize {
        if peak_before > 0.0 {
            let gain = math::normalize_gain(peak_before, NORMALIZE_TARGET);
            math::scale_in_place(&mut output.samples, gain);
            normalize_applied = true;
        } else {
            normalize_note = Some("全零信号: 峰值为 0, 归一化不改动任何样本");
        }
    }
    let peak_after = math::peak_of(&output.samples);
    // 10b. 响度读数 + 对 `targetLufs` 的判定（**只读**：在抖动之前的浮点母带上测）。
    //      刻意**在**抖动/量化之前：响度计量的对象是母带信号本身, 而抖动是
    //      "降位深"的伴生步骤（与 `normalize` 同一条理由, 见模块文档）。
    //      `with_master_gain` 之后、`normalize` 之后的这一份就是响度口径下的母带。
    let loudness = LoudnessTarget::measure(
        sample_rate,
        MASTER_CHANNELS,
        &output.samples,
        request.target_lufs(),
    );
    // 判定的两个否定出口都**不落盘**：`fail`（不达标）与 `unmeasurable`（测不出 ⇒
    // 无法判定 ⇒ 不能算通过）。判据用"失败时目录里没有产品"钉住这一点。
    if matches!(loudness.verdict, VERDICT_FAIL | VERDICT_UNMEASURABLE) {
        return Err(loudness.failure());
    }
    // 11. TPDF 抖动 → 24-bit PCM → 容器。
    let mut rng = dither_rng_for(project.rng_seed, project.master_bus_track_id);
    let pcm = quantize(&output.samples, BitDepth::Int24, &mut rng);
    let payload = pcm.to_le_bytes();
    let (origination_date, origination_time) = math::bext_stamp(now_ms);
    let originator_reference = project.id.to_canonical_string();
    let coding_history = format!("A=PCM,F={sample_rate},W={OUTPUT_BIT_DEPTH},M=stereo,T=Yeban-MCP");
    let mut bext = Bext::for_project(&originator_reference, &origination_date, &origination_time);
    // `Bext::for_project` 的 coding history 是模板字符串(带 `<sample_rate>` 字面量);
    // 母带导出必须写**真实**参数, 否则那是文件里的假元数据。
    bext.coding_history.clone_from(&coding_history);
    let preferred = match request.format() {
        "rf64" => ContainerKind::Rf64,
        "bw64" => ContainerKind::Bw64,
        _ => ContainerKind::Riff,
    };
    let container_plan = ContainerPlan::for_payload(
        preferred,
        PcmFormat::integer(
            u16::try_from(MASTER_CHANNELS).unwrap_or(u16::MAX),
            sample_rate,
            OUTPUT_BIT_DEPTH,
        ),
        payload.len() as u64,
        frames,
        Some(bext),
    );
    let header_bytes = container_plan.header_bytes().len();
    let mut bytes = Vec::with_capacity(header_bytes + payload.len() + 1);
    write_container(&mut bytes, &container_plan, &payload).map_err(|error| {
        render_failed(
            "容器编码失败",
            serde_json::json!({ "rf64Error": format!("{error:?}") }),
        )
    })?;
    let project_digest = store::digest_of(store::serialize_project(project)?.as_bytes());
    let audio_clips: u64 = reports.iter().map(|report| report.audio_clips).sum();
    let notes_silenced: u64 = reports.iter().map(|report| report.notes_silenced).sum();
    let notes_ratcheted: u64 = reports.iter().map(|report| report.notes_ratcheted).sum();
    let ratchet_pulses: u64 = reports.iter().map(|report| report.ratchet_pulses).sum();
    Ok(RenderArtifact {
        request: request.clone(),
        path,
        container: container_plan.kind,
        sample_rate,
        channels: MASTER_CHANNELS,
        bit_depth: OUTPUT_BIT_DEPTH,
        frames: output.frames,
        blocks: output.blocks,
        block_size: L1_BLOCK_SIZE,
        header_bytes,
        payload_bytes: payload.len(),
        sha256: AssetHash::of_bytes(&bytes).as_str().to_owned(),
        master_digest: hex(&output.digest),
        longest_path_frames: output.longest_path_frames,
        bytes,
        node_count: nodes.len(),
        source_nodes,
        sources: reports,
        audio_clips,
        notes_silenced,
        notes_ratcheted,
        ratchet_pulses,
        rng_seed: project.rng_seed,
        audio_assets: asset_reports.into_values().collect(),
        peak_before,
        peak_after,
        normalize_applied,
        normalize_note,
        loudness,
        originator_reference,
        origination_date,
        origination_time,
        coding_history,
        project_digest,
        rendered_at_unix_ms: now_ms,
        unsupported,
        unsupported_counts: counts,
    })
}

/// 一个保留节点是不是**源节点**（没有入边）。
///
/// `bus_reduction_order` 对源节点返回 `Some(vec![])`（空入边列表），对总线返回非空列表。
fn is_source_node(plan: &RenderPlan, node: EntityId) -> bool {
    plan.bus_reduction_order(node)
        .is_none_or(|inputs| inputs.is_empty())
}

/// 一个摆放的循环配置是否**请求了重复播放**（= 本渲染器没有展开的东西）。
///
/// 本层只渲染摆放的**第一遍**：时间轴区间 `0..duration_ticks`，内容一位都不回卷。
/// 一个启用中的循环区间，只有当它**恰好覆盖**整个摆放跨度（`start_tick == 0` 且
/// `end_tick == duration_ticks`）时，回卷点才与摆放末端重合 ⇒ 播放结果与"不循环"
/// 逐位相同，可以**静默**；其余任何形态都可能在摆放结束前回卷 ⇒ 必须登记。
///
/// 为什么是"覆盖才静默"而不是"越过末端才登记"：`loop_config` 的 tick 坐标语义
/// （相对摆放还是相对片段内容）在规范里未裁决（`docs/ledger/mcp-render-notes.md`
/// 的 pending 与 `yeban-engine` 的 `loop_config` 边界都如实登记了这一点）。本层因此
/// 取**保守**的一侧：只有"两种解释下都不可能回卷"的形态才不报，绝不拿未裁决的语义
/// 去少报。判据与 `yeban-render` 的两个导出器（`logic.rs` 与 `als.rs` 的
/// `loop_covers`）同一条口径，本层不另立第二种"什么算覆盖"。
fn loop_requests_repetition(placement: &yeban_model::ClipPlacement) -> bool {
    let loop_config = placement.loop_config;
    let covers = loop_config.start_tick == 0 && loop_config.end_tick == placement.duration_ticks;
    loop_config.enabled && !covers
}

/// 音轨上**可闻内容**的时间轴末端（没有内容时为 `None`）。
///
/// 只做"有没有/到哪"的判定，不做排程 —— 它服务于"总线轨上的片段必须被登记为
/// 不支持"这一条，以及防止"没有内容却渲染出 0 帧"。
fn audible_end_tick(track: &TrackV3) -> Option<u64> {
    let mut end: Option<u64> = None;
    for placement in track.clips.values() {
        if placement.muted {
            continue;
        }
        let candidate = placement
            .start_tick
            .saturating_add(placement.duration_ticks);
        end = Some(end.map_or(candidate, |current| current.max(candidate)));
    }
    end
}

/// 一个已排程的音符（发音所需的全部信息，与模型层解耦）。
///
/// 声相/音轨增益**不在**这里：一个源就是一条音轨，因此这两个量在整个源上恒定，
/// 放在 [`MidiSynthSource`] 上（每音符复制一遍只会制造漂移面）。
#[derive(Clone, Copy, Debug)]
struct ScheduledNote {
    /// 起始帧（含）。
    start: i64,
    /// 结束帧（不含）。
    end: i64,
    /// 频率 (Hz)。
    frequency: f32,
    /// 幅度（力度 / 127）。
    amplitude: f32,
}

/// 确定性 MIDI 合成源：**纯函数式**振荡器（相位由绝对帧号算出，不做累加）。
///
/// 相位 `= (global_frame - note_start) * frequency / sample_rate`，
/// 因此输出只依赖**绝对帧号**，与块边界、块大小、线程数完全无关 ——
/// 这比"在块之间累加相位"更强：累加只要错一次（丢块/重复块）就会漂移。
///
/// **累加而不是覆写**：调用方（[`TrackSource`] 与 `yeban-render` 的 `render_node`）
/// 负责先把缓冲清零，本源的语义是"把这一块的贡献**加上去**"。这样 MIDI 与音频片段
/// 可以按固定顺序叠加到同一个源节点上，而不需要一块额外的中转缓冲。
struct MidiSynthSource {
    notes: Vec<ScheduledNote>,
    /// 左声道增益（音轨音量 × 等功率声相）。
    gain_l: f32,
    /// 右声道增益。
    gain_r: f32,
    attack_frames: i64,
    release_frames: i64,
}

impl AudioSource for MidiSynthSource {
    fn render_block(&mut self, context: BlockContext, out: &mut [f32]) -> Result<(), RenderError> {
        let channels = context.channels;
        if channels == 0 || self.notes.is_empty() {
            return Ok(());
        }
        let sample_rate = f64::from(context.sample_rate);
        if sample_rate <= 0.0 {
            return Ok(());
        }
        for index in 0..context.frames {
            let global = i64::try_from(context.first_frame + index as u64).unwrap_or(i64::MAX);
            let mut value = 0.0f32;
            for note in &self.notes {
                if global < note.start || global >= note.end {
                    continue;
                }
                let offset = global - note.start;
                let envelope = math::envelope(
                    offset,
                    note.end - note.start,
                    self.attack_frames,
                    self.release_frames,
                );
                if envelope <= 0.0 {
                    continue;
                }
                let phase = (offset as f64) * f64::from(note.frequency) / sample_rate;
                let fraction = (phase - phase.floor()) as f32;
                value += note.amplitude * envelope * libm::sinf(fraction * core::f32::consts::TAU);
            }
            if value == 0.0 {
                continue;
            }
            let base = index * channels;
            for (channel, slot) in out[base..base + channels].iter_mut().enumerate() {
                let gain = match channel {
                    0 => self.gain_l,
                    1 => self.gain_r,
                    _ => 0.0,
                };
                *slot += value * gain;
            }
        }
        Ok(())
    }
}

/// 一个音频片段摆放的**帧区间 + 左右增益**（tick→帧的换算已在构造时做完）。
#[derive(Clone, Copy, Debug, PartialEq)]
struct ClipSpan {
    /// 起始帧（含）。
    start: i64,
    /// 结束帧（不含）。
    end: i64,
    /// 左声道增益（片段 `gain_db` + 音轨 `volume_db` → 线性，× 等功率声相）。
    gain_l: f32,
    /// 右声道增益。
    gain_r: f32,
}

/// 一个**已经解码（必要时已重采样）**的音频资产的共享载体。
///
/// 用 `Arc<[f32]>` 而不是 `Vec<f32>`：同一份素材可以被多条音轨的片段引用，
/// 只读共享让它只解码一次、只驻留一份（[ARCH-TOP-002] 的"不可变资产"语义）。
#[derive(Debug)]
struct PreparedAsset {
    /// 交织 `f32` 样本（已经过编码器延迟裁剪与重采样）。
    samples: Arc<[f32]>,
    /// 素材原始声道数（决定声道布局映射）。
    source_channels: u16,
    /// 裁剪后进入渲染的帧数。
    frames: i64,
}

/// 音频片段源：把一份**已经解码好**的 PCM 按 placement 的帧区间搬进母带。
///
/// 语义（全部是刻意的、可判据化的）：
///
/// - **累加**（不是覆写）：与 [`MidiSynthSource`] 一样，调用方负责清零；
/// - **块无关**：某帧的取值只由 `帧号 - 片段起点` 决定，与块边界、块大小、
///   线程数、块的到达顺序都无关 ⇒ 母带逐位可复现；
/// - **硬切**：`placement.duration_ticks` 换算出的区间之外一律不加，
///   素材比区间长就截断、比区间短就留静音（没有淡入淡出 —— 本模型版本
///   没有交叉淡化字段，凭空造一个是发明规范）；
/// - **重叠相加**：同一素材的多个摆放重叠时按 `(start, end, gain 位型)` 全序
///   依次累加（[ARCH-DET-002]：浮点加法不满足结合律，顺序必须与输入顺序无关）。
struct AudioClipSource {
    asset: Arc<PreparedAsset>,
    /// 声道布局（素材声道数 → 母线声道数）。
    layout: clip_math::ChannelLayout,
    /// 摆放区间，**已按 `(start, end, gain)` 全序排好**。
    spans: Vec<ClipSpan>,
}

impl AudioClipSource {
    /// 把每个区间夹进 `[0, frames)`（母带长度在构造之后才知道）。
    fn clamp_to(&mut self, frames: u64) {
        let limit = i64::try_from(frames).unwrap_or(i64::MAX);
        for span in &mut self.spans {
            span.start = span.start.clamp(0, limit);
            span.end = span.end.clamp(0, limit);
        }
        self.spans.retain(|span| span.start < span.end);
    }

    /// 按 `[ARCH-DET-002]` 的固定全序排好区间。
    fn sort_spans(&mut self) {
        self.spans.sort_by(|a, b| {
            a.start
                .cmp(&b.start)
                .then(a.end.cmp(&b.end))
                .then(a.gain_l.to_bits().cmp(&b.gain_l.to_bits()))
                .then(a.gain_r.to_bits().cmp(&b.gain_r.to_bits()))
        });
    }

    /// 本块内某一帧在**素材**里的下标（超出素材则 `None`）。
    fn asset_frame(&self, span: &ClipSpan, global: i64) -> Option<usize> {
        let offset = global - span.start;
        if offset < 0 || offset >= self.asset.frames {
            return None;
        }
        usize::try_from(offset).ok()
    }
}

impl AudioSource for AudioClipSource {
    fn render_block(&mut self, context: BlockContext, out: &mut [f32]) -> Result<(), RenderError> {
        let channels = context.channels;
        if channels == 0 || self.spans.is_empty() || self.asset.frames <= 0 {
            return Ok(());
        }
        let source_channels = usize::from(self.asset.source_channels);
        let block_start = i64::try_from(context.first_frame).unwrap_or(i64::MAX);
        let block_end =
            block_start.saturating_add(i64::try_from(context.frames).unwrap_or(i64::MAX));
        for span in &self.spans {
            // spans 按 start 升序: 一旦起点已经在块之后, 后面的也不可能落进来。
            if span.start >= block_end {
                break;
            }
            if span.end <= block_start {
                continue;
            }
            let from = span.start.max(block_start);
            let to = span.end.min(block_end);
            for global in from..to {
                let Some(asset_frame) = self.asset_frame(span, global) else {
                    continue;
                };
                let source_base = asset_frame * source_channels;
                let index = usize::try_from(global - block_start).unwrap_or(0);
                let out_base = index * channels;
                match self.layout {
                    clip_math::ChannelLayout::Identity => {
                        for channel in 0..channels {
                            let gain = channel_gain(span, channel);
                            out[out_base + channel] +=
                                self.asset.samples[source_base + channel] * gain;
                        }
                    }
                    clip_math::ChannelLayout::MonoToAll => {
                        let value = self.asset.samples[source_base];
                        for channel in 0..channels {
                            let gain = channel_gain(span, channel);
                            out[out_base + channel] += value * gain;
                        }
                    }
                    clip_math::ChannelLayout::StereoToMono => {
                        let mixed = (self.asset.samples[source_base]
                            + self.asset.samples[source_base + 1])
                            * 0.5;
                        out[out_base] += mixed * span.gain_l;
                    }
                }
            }
        }
        Ok(())
    }
}

/// 某一输出声道的增益（第 0 声道左、第 1 声道右，其余不参与）。
fn channel_gain(span: &ClipSpan, channel: usize) -> f32 {
    match channel {
        0 => span.gain_l,
        1 => span.gain_r,
        _ => 0.0,
    }
}

/// 一条音轨的复合源：**MIDI 合成 ⊕ 音频片段**（固定顺序相加）。
///
/// 一个 `AudioSource` 对应一个源节点（= 一条音轨），而一条音轨上可以同时有
/// MIDI 片段与音频片段 —— 因此需要一个把两者按**固定顺序**叠起来的容器。
/// 顺序是"先 MIDI、后音频片段"，与块边界无关，因此逐位可复现。
struct TrackSource {
    midi: MidiSynthSource,
    clips: Vec<AudioClipSource>,
}

impl TrackSource {
    /// 把音频片段的区间夹进母带长度（母带长度在构造之后才知道）。
    fn clamp_to(&mut self, frames: u64) {
        for clips in &mut self.clips {
            clips.clamp_to(frames);
        }
    }

    /// 源种类（响应 `sources[].kind`）。
    fn kind(&self) -> &'static str {
        let has_midi = !self.midi.notes.is_empty();
        let has_audio = self.clips.iter().any(|clips| !clips.spans.is_empty());
        match (has_midi, has_audio) {
            (true, true) => MIXED_SOURCE_KIND,
            (true, false) => SOURCE_KIND,
            (false, true) => AUDIO_SOURCE_KIND,
            (false, false) => SILENT_SOURCE_KIND,
        }
    }
}

impl AudioSource for TrackSource {
    fn render_block(&mut self, context: BlockContext, out: &mut [f32]) -> Result<(), RenderError> {
        // 调用方（`yeban-render` 的 `render_node`）已经把缓冲清零；这里只累加。
        self.midi.render_block(context, out)?;
        for clips in &mut self.clips {
            clips.render_block(context, out)?;
        }
        Ok(())
    }
}

/// 解析（必要时解码 + 重采样）一个音频资产；结果按哈希缓存。
///
/// 返回 `Ok(None)` 的**唯一**情形是"工程声明了它、但会话 CAS 池里没有它的字节"
/// —— 调用方据此登记 `audioClips` 并当静音。其余一切异常都是明确的 `Fault`。
fn resolve_asset(
    hash: &AssetHash,
    project: &YebanProjectV1,
    assets: &dyn AssetStore,
    target_rate: u32,
    cache: &mut BTreeMap<AssetHash, Option<Arc<PreparedAsset>>>,
    reports: &mut BTreeMap<AssetHash, AudioAssetReport>,
) -> Result<Option<Arc<PreparedAsset>>, Fault> {
    if let Some(cached) = cache.get(hash) {
        if let Some(entry) = reports.get_mut(hash) {
            entry.clips = entry.clips.saturating_add(1);
        }
        return Ok(cached.clone());
    }
    let declared_in_index = project.assets.contains_key(hash);
    let sniffed = assets.asset(hash).map_or(
        clip_math::ContainerSniff::Unknown,
        clip_math::sniff_container,
    );
    // 池里没有字节：工程也没声明 ⇒ 悬空引用（明确错误）；声明了 ⇒ 登记 + 静音。
    let Some(bytes) = assets.asset(hash) else {
        if !declared_in_index {
            return Err(render_failed(
                "音频片段引用了一个既不在会话资产池里、工程资产索引里也不存在的资产 \
                 (悬空的 CAS 引用)",
                serde_json::json!({
                    "reason": "assetMissing",
                    "asset": hash.as_str(),
                    "declaredInIndex": false,
                    "hint": "容器形态下 assets/{sha256} 与工程 assets 索引必须同时存在; \
                             单独存在的哈希说明工程被改坏了",
                }),
            ));
        }
        reports.insert(
            hash.clone(),
            AudioAssetReport {
                hash: hash.as_str().to_owned(),
                bytes_present: false,
                declared_in_index: true,
                container: sniffed.name(),
                pcm_format: "unknown",
                source_channels: 0,
                source_sample_rate: 0,
                source_frames: 0,
                target_sample_rate: target_rate,
                resampled: false,
                channel_layout: "none",
                encoder_delay_frames: None,
                encoder_padding_frames: None,
                trimmed_frames: 0,
                rendered_frames: 0,
                bytes: 0,
                clips: 1,
            },
        );
        cache.insert(hash.clone(), None);
        return Ok(None);
    };
    // 完整性: 池里的字节必须**真的是**这个哈希的内容。
    // 容器读写两侧都校验 SHA-256，这里是第三道 —— 内存注入的池（判据用的那种）
    // 不经过容器，因此这一道不是多余的。
    let actual = AssetHash::of_bytes(bytes);
    if actual != *hash {
        return Err(render_failed(
            "会话资产池里的字节与它声明的 SHA-256 不符 (资产被篡改或串位)",
            serde_json::json!({
                "reason": "assetHashMismatch",
                "asset": hash.as_str(),
                "actual": actual.as_str(),
                "bytes": bytes.len(),
            }),
        ));
    }
    let decoded = yeban_decode::decode_bytes(bytes, &DecodeOptions::default())
        .map_err(|error| decode_fault(hash, &error))?;
    let source_channels = decoded.channels();
    let source_rate = decoded.sample_rate();
    let source_frames = decoded.frame_count();
    let facts = decoded.facts();
    let delay = facts.encoder_delay_frames;
    let padding = facts.encoder_padding_frames;
    let (from, to) =
        clip_math::encoder_trim_span(source_frames, delay, padding).ok_or_else(|| {
            render_failed(
                "音频资产的编码器延迟/填充把整段都裁掉了 (没有可渲染的样本)",
                serde_json::json!({
                    "reason": "assetDecodeFailed",
                    "asset": hash.as_str(),
                    "sourceFrames": source_frames,
                    "encoderDelayFrames": delay,
                    "encoderPaddingFrames": padding,
                }),
            )
        })?;
    let retained_frames = to - from;
    let source_channels_usize = usize::from(source_channels);
    let trimmed = if from == 0 && to == source_frames {
        decoded.samples().to_vec()
    } else {
        let lo = usize::try_from(from)
            .unwrap_or(usize::MAX)
            .saturating_mul(source_channels_usize);
        let hi = usize::try_from(to)
            .unwrap_or(usize::MAX)
            .saturating_mul(source_channels_usize);
        decoded
            .samples()
            .get(lo..hi)
            .map_or_else(Vec::new, <[f32]>::to_vec)
    };
    // 采样率不一致 ⇒ 必须**真的重采样**（[ARCH-DSP-002] / D26），
    // 绝不用"改标签"（改时长）或"丢帧"（改音高）代替。
    let resampled = clip_math::needs_resample(source_rate, target_rate);
    let converted = resample_interleaved(&trimmed, source_channels, source_rate, target_rate)
        .map_err(|error| decode_fault(hash, &error))?;
    let frames = i64::try_from(converted.len() / source_channels_usize.max(1)).unwrap_or(i64::MAX);
    // 报告里的样本格式是**解码器实际吐出的**那一种（不是容器声明的），
    // 用穷举映射成 `&'static str`（不泄漏字符串、也不随 `Debug` 措辞漂移）。
    let pcm_format_name = pcm_format_name(decoded.pcm_format());
    let channel_layout = clip_math::channel_layout(source_channels, MASTER_CHANNELS);
    let layout_name = channel_layout.map_or("unsupported", clip_math::ChannelLayout::name);
    let prepared = Arc::new(PreparedAsset {
        samples: Arc::from(converted),
        source_channels,
        frames,
    });
    reports.insert(
        hash.clone(),
        AudioAssetReport {
            hash: hash.as_str().to_owned(),
            bytes_present: true,
            declared_in_index,
            container: sniffed.name(),
            pcm_format: pcm_format_name,
            source_channels,
            source_sample_rate: source_rate,
            source_frames,
            target_sample_rate: target_rate,
            resampled,
            channel_layout: layout_name,
            encoder_delay_frames: delay,
            encoder_padding_frames: padding,
            // `trimmed_frames` 是**丢掉**的帧数, `rendered_frames` 是**重采样之后**
            // 真正进渲染的帧数（`retained_frames` 是重采样前的帧数, 两者只在
            // 采样率一致时相等 —— 这一点由 `a_44k1_asset_...` 判据钉住）。
            trimmed_frames: source_frames.saturating_sub(retained_frames),
            rendered_frames: u64::try_from(frames.max(0)).unwrap_or(0),
            bytes: bytes.len(),
            clips: 1,
        },
    );
    cache.insert(hash.clone(), Some(Arc::clone(&prepared)));
    Ok(Some(prepared))
}

/// `PcmFormat` → 稳定的短名（响应用）。
///
/// `pub(crate)`：`yeban_import_audio` 也用它，**不**复制第二份格式名表
/// （名字表一多就必然有一份会漂移）。
pub(crate) fn pcm_format_name(format: yeban_decode::PcmFormat) -> &'static str {
    use yeban_decode::PcmFormat as P;
    match format {
        P::U8 => "u8",
        P::U16 => "u16",
        P::U24 => "u24",
        P::U32 => "u32",
        P::S8 => "s8",
        P::S16 => "s16",
        P::S24 => "s24",
        P::S32 => "s32",
        P::F32 => "f32",
        P::F64 => "f64",
    }
}

/// `DecodeError` → 契约内的错误码 + 可自纠的载荷。
///
/// 只有一个变体走 `IO_ERROR`（`DecodeError::Io`）—— 它**确实**是 I/O 失败；
/// 其余（畸形流、不支持的容器、声道数非法、空流…）走 `RENDER_FAILED`，
/// 并在 `data.decodeError` 里带上分类名，绝不吞成"静音成功"。
fn decode_fault(hash: &AssetHash, error: &DecodeError) -> Fault {
    let classification = decode_error_class(error);
    let data = serde_json::json!({
        "reason": "assetDecodeFailed",
        "asset": hash.as_str(),
        "decodeError": classification,
        "detail": error.to_string(),
        "specId": "MODEL-AST-007",
    });
    match error {
        DecodeError::Io(_) => Fault::domain_with_data(
            ErrorCode::IoError,
            format!("音频资产 {} 读取失败: {error}", hash.as_str()),
            data,
        ),
        _ => render_failed(
            format!("音频资产 {} 解码失败: {error}", hash.as_str()),
            data,
        ),
    }
}

/// `DecodeError` 的稳定分类名（响应用；不随 `Display` 的措辞漂移）。
///
/// `pub(crate)`：`yeban_import_audio` 的错误分类复用这一份（判据
/// `import_and_render_agree_on_the_decode_error_vocabulary` 钉住两侧同词）。
pub(crate) fn decode_error_class(error: &DecodeError) -> &'static str {
    match error {
        DecodeError::Io(_) => "io",
        DecodeError::UnsupportedFormat => "unsupportedFormat",
        DecodeError::NoAudioTrack => "noAudioTrack",
        DecodeError::MissingCodecParameters => "missingCodecParameters",
        DecodeError::UnsupportedCodec { .. } => "unsupportedCodec",
        DecodeError::MissingSampleRate => "missingSampleRate",
        DecodeError::ResetRequired => "resetRequired",
        DecodeError::Malformed { .. } => "malformed",
        DecodeError::Budget(_) => "budget",
        DecodeError::InconsistentLayout { .. } => "inconsistentLayout",
        DecodeError::EmptyStream => "emptyStream",
        DecodeError::DurationMismatch(_) => "durationMismatch",
        DecodeError::ResamplerConfiguration { .. } => "resamplerConfiguration",
        DecodeError::Resampling { .. } => "resampling",
        DecodeError::LengthContract(_) => "lengthContract",
    }
}

/// 由一条音轨构造音频源 + 报告，并把"没渲染的部分"登记进 `unsupported`。
#[allow(clippy::too_many_arguments)]
fn build_source(
    project: &YebanProjectV1,
    track: &TrackV3,
    sample_rate: u32,
    any_solo: bool,
    assets: &dyn AssetStore,
    cache: &mut BTreeMap<AssetHash, Option<Arc<PreparedAsset>>>,
    asset_reports: &mut BTreeMap<AssetHash, AudioAssetReport>,
    unsupported: &mut BTreeSet<&'static str>,
    counts: &mut BTreeMap<&'static str, u64>,
) -> Result<(TrackSource, SourceReport), Fault> {
    // solo 语义（只作用于源轨；辅助返回总线不参与判定 —— 登记在 notes 的边界里）:
    // 只要工程里有任一音轨 solo, 未 solo 且非 solo-safe 的源轨就不发声。
    let audible = !track.mute && (!any_solo || track.solo || track.solo_safe);
    let track_gain = if audible {
        db_to_linear(track.volume_db)
    } else {
        0.0
    };
    // 设备链: 只有 latency_samples 进了 PDC; 参数求值没有实现 —— 出现即登记。
    if track.devices.iter().any(|device| !device.bypassed) {
        note_unsupported(unsupported, counts, "deviceChainDsp");
    }
    if track.devices.iter().any(|device| {
        !device.bypassed
            && matches!(
                device.kind,
                DeviceKind::ExternalInstrument | DeviceKind::ExternalEffect
            )
    }) {
        note_unsupported(unsupported, counts, "externalPlugins");
    }
    if !track.automation_lanes.is_empty() {
        note_unsupported(unsupported, counts, "automationLanes");
    }
    let attack_frames = math::ms_to_frames(ATTACK_MS, sample_rate);
    let release_frames = math::ms_to_frames(RELEASE_MS, sample_rate);
    let mut scheduled: Vec<ScheduledNote> = Vec::new();
    // 每个资产 → 它在这条音轨上的摆放区间（同一资产的多个摆放共用一个源对象）。
    let mut per_asset: BTreeMap<AssetHash, (Arc<PreparedAsset>, Vec<ClipSpan>)> = BTreeMap::new();
    let mut clips_unrendered = 0u64;
    let mut clips_gated = 0u64;
    let mut notes_silenced = 0u64;
    let mut notes_ratcheted = 0u64;
    let mut ratchet_pulses = 0u64;
    let mut end_tick = 0u64;
    for placement in track.clips.values() {
        if placement.muted {
            continue;
        }
        let clip = project.clip_pool.get(&placement.clip_id).ok_or_else(|| {
            render_failed(
                "摆放引用了片段池里不存在的片段 (工程结构已失效)",
                serde_json::json!({
                    "placementId": placement.id.to_canonical_string(),
                    "clipId": placement.clip_id.to_canonical_string(),
                }),
            )
        })?;
        end_tick = end_tick.max(
            placement
                .start_tick
                .saturating_add(placement.duration_ticks),
        );
        if loop_requests_repetition(placement) {
            note_unsupported(unsupported, counts, "clipLoopRepetition");
        }
        match &clip.content {
            ClipContent::Midi { notes } => {
                for note in notes.values() {
                    // 概率触发：**唯一**判定入口是模型层的 `MidiNote::triggers`
                    // [MODEL-AST-005, ARCH-DET-001]。本层**不**自己掷骰子、不引入熵源，
                    // 也不再看 `probability` 的字面值 —— 否则这里就会出现第二份
                    // "什么算触发"的实现，与实时引擎
                    // (`yeban-engine/src/snapshot.rs` 的 `project_schedules`) 漂移，
                    // 于是"监听时听到的"与"导出的母带"不再是同一个作品。
                    //
                    // 未触发的音符**一位都不贡献**：不进排程、不延长 `end_tick`
                    // （它不发声，因此没有时值可占）。
                    if !note.triggers(project.rng_seed) {
                        notes_silenced = notes_silenced.saturating_add(1);
                        continue;
                    }
                    if note.slide.is_some() {
                        note_unsupported(unsupported, counts, "noteSlide");
                    }
                    if !note.pitch_bend_curve.is_empty() {
                        note_unsupported(unsupported, counts, "notePitchBend");
                    }
                    if note.syllable.is_some() || !note.phonemes.is_empty() {
                        note_unsupported(unsupported, counts, "noteLyrics");
                    }
                    let micro = i64::from(note.micro_timing_ticks.unwrap_or(0));
                    // 连击：**格点与实时引擎同一裁决** —— 时值用整数除法等分成
                    // `ratchet` 个脉冲，余数不补（`docs/ledger/engine-sound-notes.md`；
                    // 引擎侧见 `yeban-engine/src/snapshot.rs` 的 `project_schedules`）。
                    // 本层不另立一份"怎么分"的规则。`ratchet` 缺省或 `= 1` 时
                    // `step == duration_ticks` ⇒ 正好是加这个旋钮之前的那一个区间，
                    // 因此默认路径逐位不变（判据：默认夹具的母带摘要常量）。
                    // `ratchet` 的合法范围由模型层唯一定义（`MidiNote::validate` 用同两个常量）。
                    let ratchet = u64::from(
                        note.ratchet
                            .unwrap_or(RATCHET_MIN)
                            .clamp(RATCHET_MIN, RATCHET_MAX),
                    );
                    let step = (note.duration_ticks / ratchet).max(1);
                    if ratchet > 1 {
                        notes_ratcheted = notes_ratcheted.saturating_add(1);
                        ratchet_pulses = ratchet_pulses.saturating_add(ratchet);
                    }
                    let base_tick = placement.start_tick.saturating_add(note.start_tick);
                    for pulse in 0..ratchet {
                        let (start, end) = math::note_frame_span(
                            base_tick.saturating_add(pulse.saturating_mul(step)),
                            step,
                            micro,
                            PPQ,
                            project.bpm,
                            sample_rate,
                        );
                        scheduled.push(ScheduledNote {
                            start,
                            end,
                            frequency: pitch_to_hz(note.pitch),
                            amplitude: f32::from(note.velocity) / 127.0,
                        });
                    }
                    let shifted_micro = u64::try_from(micro.max(0)).unwrap_or(0);
                    end_tick = end_tick.max(
                        placement
                            .start_tick
                            .saturating_add(note.start_tick)
                            .saturating_add(shifted_micro)
                            .saturating_add(note.duration_ticks),
                    );
                }
            }
            ClipContent::Audio { asset, gain_db } => {
                // 片段增益与音轨音量都是 dB ⇒ 先求和（一次超越函数求值）再转线性。
                let Some(combined_db) =
                    clip_math::combined_gain_db(*gain_db, track.volume_db, audible)
                else {
                    // 音轨被 mute/solo 门控掉: 不发声、**不解码**, 但仍计入时间轴末端。
                    clips_gated = clips_gated.saturating_add(1);
                    continue;
                };
                let gain = db_to_linear(combined_db);
                let (gain_l, gain_r) = if track.pan.is_finite() && track.pan != 0.0 {
                    let (left, right) = pan_gains(track.pan);
                    (gain * left, gain * right)
                } else {
                    (gain, gain)
                };
                let Some(prepared) =
                    resolve_asset(asset, project, assets, sample_rate, cache, asset_reports)?
                else {
                    note_unsupported(unsupported, counts, "audioClips");
                    clips_unrendered = clips_unrendered.saturating_add(1);
                    continue;
                };
                // 声道布局不支持 ⇒ **拒绝**，而不是悄悄丢声道。
                let Some(_layout) =
                    clip_math::channel_layout(prepared.source_channels, MASTER_CHANNELS)
                else {
                    return Err(render_failed(
                        "音频素材的声道数无法映射到立体声母线 (不做丢声道的静默降混)",
                        serde_json::json!({
                            "reason": "assetChannelLayout",
                            "asset": asset.as_str(),
                            "sourceChannels": prepared.source_channels,
                            "targetChannels": MASTER_CHANNELS,
                            "supported": [1, MASTER_CHANNELS],
                        }),
                    ));
                };
                let (start, end) = clip_math::clip_frame_span(
                    placement.start_tick,
                    placement.duration_ticks,
                    PPQ,
                    project.bpm,
                    sample_rate,
                );
                let entry = per_asset
                    .entry(asset.clone())
                    .or_insert_with(|| (Arc::clone(&prepared), Vec::new()));
                entry.1.push(ClipSpan {
                    start,
                    end,
                    gain_l,
                    gain_r,
                });
            }
        }
    }
    // 浮点求和不满足结合律 [ARCH-DET-002]: 排程顺序必须是**全序**且与输入顺序无关。
    // 键取到"两个音符完全相同则顺序无影响"的深度即可。
    scheduled.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then(a.end.cmp(&b.end))
            .then(a.frequency.to_bits().cmp(&b.frequency.to_bits()))
            .then(a.amplitude.to_bits().cmp(&b.amplitude.to_bits()))
    });
    let audio_clips: u64 = per_asset
        .values()
        .map(|(_, spans)| u64::try_from(spans.len()).unwrap_or(u64::MAX))
        .sum();
    // 一条音轨可能引用多个资产（本模型允许）: 为每个资产建一个源，
    // 由 `TrackSource` 按**哈希字典序**（`BTreeMap` 的键序）叠加 ——
    // 顺序确定且与输入顺序无关，因此逐位可复现。
    let clips: Vec<AudioClipSource> = per_asset
        .into_iter()
        .map(|(_, (asset, spans))| {
            // 声道布局在 resolve 阶段已经校验过；这里复用同一张表（不另立一份）。
            let layout = clip_math::channel_layout(asset.source_channels, MASTER_CHANNELS)
                .unwrap_or(clip_math::ChannelLayout::MonoToAll);
            let mut source = AudioClipSource {
                asset,
                layout,
                spans,
            };
            source.sort_spans();
            source
        })
        .collect();
    let midi = MidiSynthSource {
        notes: scheduled,
        gain_l: if track.pan.is_finite() && track.pan != 0.0 {
            track_gain * pan_gains(track.pan).0
        } else {
            track_gain
        },
        gain_r: if track.pan.is_finite() && track.pan != 0.0 {
            track_gain * pan_gains(track.pan).1
        } else {
            track_gain
        },
        attack_frames,
        release_frames,
    };
    let source = TrackSource { midi, clips };
    let report = SourceReport {
        node: track.id,
        track: track.name.clone(),
        kind: source.kind(),
        audible,
        notes: u64::try_from(source.midi.notes.len()).unwrap_or(u64::MAX),
        audio_clips,
        audio_clips_unrendered: clips_unrendered,
        audio_clips_gated: clips_gated,
        notes_silenced,
        notes_ratcheted,
        ratchet_pulses,
        end_tick,
    };
    Ok((source, report))
}

/// 等程律频率：`440 * 2^((pitch - 69) / 12)`，走 `libm::powf` [ARCH-DET-001]。
fn pitch_to_hz(pitch: u8) -> f32 {
    440.0 * libm::powf(2.0, (f32::from(pitch) - 69.0) / 12.0)
}

/// 等功率（−3 dB）声相增益：`θ = (pan + 1) * π/4`，`(cos θ, sin θ)`。
///
/// 与模型层 `PanLaw::ConstantPowerMinus3dB` 的口径一致：居中的 `pan = 0`
/// 给出 `(√2/2, √2/2)`（即每个声道 −3 dB）。
fn pan_gains(pan: f32) -> (f32, f32) {
    let clamped = pan.clamp(-1.0, 1.0);
    let angle = (clamped + 1.0) * core::f32::consts::FRAC_PI_4;
    (libm::cosf(angle), libm::sinf(angle))
}

/// 登记一条"明确没渲染"的部分（键 + 出现次数）。
fn note_unsupported(
    unsupported: &mut BTreeSet<&'static str>,
    counts: &mut BTreeMap<&'static str, u64>,
    key: &'static str,
) {
    unsupported.insert(key);
    let counter = counts.entry(key).or_insert(0);
    *counter = counter.saturating_add(1);
}

/// `[u8; 32]` → 64 位小写十六进制。
fn hex(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// `RENDER_FAILED` 的简写（带结构化原因，便于调用方自纠）。
fn render_failed(message: impl Into<String>, data: Value) -> Fault {
    Fault::domain_with_data(ErrorCode::RenderFailed, message, data)
}

/// [`RenderError`] → `RENDER_FAILED`。
fn render_error(context: &str, error: &RenderError) -> Fault {
    render_failed(
        format!("{context}: {error}"),
        serde_json::json!({
            "renderError": format!("{error:?}"),
            "context": context,
            "specId": "ROAD-M4-004",
        }),
    )
}

/// 输出路径的两条安全护栏：不得是工程文件本身、不得是 `.yeban.lock`。
fn guard_output_path(output: &Path, project_path: &Path) -> Result<(), Fault> {
    if same_path(output, project_path) {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            "输出路径不能是工程文件本身 (会毁掉工程)",
            serde_json::json!({
                "outputPath": output.display().to_string(),
                "projectPath": project_path.display().to_string(),
            }),
        ));
    }
    let lock = store::lock_path(project_path);
    if same_path(output, &lock) {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            "输出路径不能是工程的锁文件 (.yeban.lock)",
            serde_json::json!({
                "outputPath": output.display().to_string(),
                "lockFile": lock.display().to_string(),
            }),
        ));
    }
    Ok(())
}

/// 两个路径是否指向同一个文件（存在时用 `canonicalize`，否则比字面量）。
fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn unknown_format_and_sample_rate_are_invalid_parameter_range() {
        let fault = validate(&args(
            serde_json::json!({"format": "mp3", "sampleRate": 48000}),
        ))
        .expect_err("未知格式");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["code"], "INVALID_PARAMETER_RANGE");
        assert!(value["error"]["data"]["supportedFormats"].is_array());

        let fault = validate(&args(
            serde_json::json!({"format": "wav", "sampleRate": 12345}),
        ))
        .expect_err("采样率集合外");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));

        let fault = validate(&args(
            serde_json::json!({"format": "wav", "sampleRate": -1}),
        ))
        .expect_err("负数采样率");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
    }

    #[test]
    fn valid_arguments_produce_a_checked_request() {
        let request = validate(&args(
            serde_json::json!({"format": "rf64", "sampleRate": 96000, "normalize": true}),
        ))
        .expect("合法");
        assert_eq!(request.format(), "rf64");
        assert_eq!(request.sample_rate.hz(), 96000);
        assert!(request.normalize);
        assert_eq!(request.path, None);
        // normalize 缺省为 false。
        let default = validate(&args(
            serde_json::json!({"format": "wav", "sampleRate": 44100}),
        ))
        .expect("合法");
        assert!(!default.normalize);
        assert_eq!(default.to_value()["format"], "wav");
    }

    #[test]
    fn explicit_path_is_validated_and_used_verbatim() {
        let request = validate(&args(
            serde_json::json!({"format": "wav", "sampleRate": 48000, "path": "/tmp/x.wav"}),
        ))
        .expect("合法");
        assert_eq!(request.path, Some(PathBuf::from("/tmp/x.wav")));
        assert_eq!(
            request
                .output_path(Path::new("/tmp/demo.yeban"))
                .expect("路径"),
            PathBuf::from("/tmp/x.wav")
        );
        for bad in [
            serde_json::json!({"format": "wav", "sampleRate": 48000, "path": ""}),
            serde_json::json!({"format": "wav", "sampleRate": 48000, "path": "   "}),
            serde_json::json!({"format": "wav", "sampleRate": 48000, "path": 7}),
        ] {
            let fault = validate(&args(bad.clone())).expect_err("坏 path");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{bad}"
            );
        }
    }

    #[test]
    fn default_output_path_follows_the_documented_rule() {
        let request = validate(&args(
            serde_json::json!({"format": "rf64", "sampleRate": 48000}),
        ))
        .expect("合法");
        assert_eq!(
            request
                .output_path(Path::new("/music/demo.yeban"))
                .expect("路径"),
            PathBuf::from("/music/demo.master.rf64")
        );
        // 没有扩展名时 stem 就是整个文件名。
        assert_eq!(
            request
                .output_path(Path::new("/music/no-extension"))
                .expect("路径"),
            PathBuf::from("/music/no-extension.master.rf64")
        );
        // 连 stem 都没有（根目录）时回退到 DEFAULT_STEM；此时 `parent_dir` 也在 `"."`。
        assert_eq!(
            request.output_path(Path::new("/")).expect("路径"),
            PathBuf::from("./master.master.rf64")
        );
    }

    #[test]
    fn output_path_may_not_be_the_project_or_the_lock_file() {
        let project = Path::new("/music/demo.yeban");
        let fault = guard_output_path(project, project).expect_err("不得覆盖工程");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let lock = PathBuf::from(format!("{}.lock", project.display()));
        let fault = guard_output_path(&lock, project).expect_err("不得覆盖锁文件");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        guard_output_path(Path::new("/music/demo.master.wav"), project).expect("正常路径");
    }

    #[test]
    fn pitch_and_pan_follow_the_model_conventions() {
        assert!((pitch_to_hz(69) - 440.0).abs() < 1.0e-3, "A4 = 440 Hz");
        assert!((pitch_to_hz(81) - 880.0).abs() < 1.0e-2, "A5 = 880 Hz");
        assert!((pitch_to_hz(57) - 220.0).abs() < 1.0e-2, "A3 = 220 Hz");
        let (left, right) = pan_gains(0.0);
        assert!((left - right).abs() < 1.0e-6, "居中必须对称");
        assert!((left - core::f32::consts::FRAC_1_SQRT_2).abs() < 1.0e-6);
        let (left, right) = pan_gains(-1.0);
        assert!((left - 1.0).abs() < 1.0e-6 && right.abs() < 1.0e-6, "全左");
        let (left, right) = pan_gains(1.0);
        assert!((right - 1.0).abs() < 1.0e-6 && left.abs() < 1.0e-6, "全右");
    }

    #[test]
    fn unsupported_keys_are_counted_and_ordered() {
        let mut set = BTreeSet::new();
        let mut counts = BTreeMap::new();
        note_unsupported(&mut set, &mut counts, "audioClips");
        note_unsupported(&mut set, &mut counts, "audioClips");
        note_unsupported(&mut set, &mut counts, "deviceChainDsp");
        assert_eq!(
            set.iter().copied().collect::<Vec<_>>(),
            vec!["audioClips", "deviceChainDsp"],
            "键序必须是字典序 (BTreeSet)"
        );
        assert_eq!(counts.get("audioClips"), Some(&2));
        assert_eq!(counts.get("deviceChainDsp"), Some(&1));
    }

    #[test]
    fn missing_project_is_no_active_project() {
        let fault = require_project(None).expect_err("没有工程");
        assert_eq!(fault.domain_code(), Some(ErrorCode::NoActiveProject));
        let project = YebanProjectV1::default();
        require_project(Some(&project)).expect("有工程");
    }

    #[test]
    fn hex_is_lowercase_and_fixed_width() {
        let mut digest = [0u8; 32];
        digest[0] = 0xAB;
        digest[31] = 0x01;
        let text = hex(&digest);
        assert_eq!(text.len(), 64);
        assert!(text.starts_with("ab"));
        assert!(text.ends_with("01"));
        assert!(
            text.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }

    #[test]
    fn audible_end_tick_ignores_muted_placements() {
        let mut track = TrackV3::default();
        assert_eq!(audible_end_tick(&track), None);
        let id = EntityId::new();
        track.clips.insert(
            id,
            yeban_model::ClipPlacement {
                id,
                clip_id: EntityId::new(),
                start_tick: 960,
                duration_ticks: 480,
                muted: true,
                ..yeban_model::ClipPlacement::default()
            },
        );
        assert_eq!(audible_end_tick(&track), None, "静音摆放不算内容");
        let live = EntityId::new();
        track.clips.insert(
            live,
            yeban_model::ClipPlacement {
                id: live,
                clip_id: EntityId::new(),
                start_tick: 1_920,
                duration_ticks: 960,
                ..yeban_model::ClipPlacement::default()
            },
        );
        assert_eq!(audible_end_tick(&track), Some(2_880));
    }

    /// `loop_requests_repetition` 的**穷尽边界**（纯函数, 不需要渲染）。
    ///
    /// 摆放跨度固定为 `0..1920`；逐个形态钉住"登记 / 静默"的判定。
    #[test]
    fn loop_repetition_is_claimed_unless_the_region_covers_the_placement() {
        fn placed(enabled: bool, start_tick: u64, end_tick: u64) -> yeban_model::ClipPlacement {
            yeban_model::ClipPlacement {
                duration_ticks: 1_920,
                loop_config: yeban_model::LoopConfig {
                    enabled,
                    start_tick,
                    end_tick,
                },
                ..yeban_model::ClipPlacement::default()
            }
        }
        // 关掉的循环从来不登记（哪怕区间形态怪异）。
        assert!(!loop_requests_repetition(&placed(false, 0, 0)));
        assert!(!loop_requests_repetition(&placed(false, 480, 960)));
        // 恰好覆盖 `0..duration_ticks` ⇒ 回卷点与末端重合 ⇒ 静默。
        assert!(!loop_requests_repetition(&placed(true, 0, 1_920)));
        // 区间短于摆放 ⇒ 摆放内一定回卷 ⇒ 登记（这正是旧实现的静默口）。
        assert!(loop_requests_repetition(&placed(true, 0, 960)));
        // 起点不为 0 ⇒ 不是覆盖 ⇒ 保守登记。
        assert!(loop_requests_repetition(&placed(true, 480, 1_920)));
        // 终点越过摆放末端 ⇒ 同样不是覆盖 ⇒ 保守登记（既有的判据 12 钉住这一形态）。
        assert!(loop_requests_repetition(&placed(true, 0, 3_840)));
    }
}
