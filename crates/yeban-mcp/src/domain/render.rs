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
//! （PDC 对齐）、边增益 `gain_db`、Master 轨的 `volume_db` 与 **`pan`**（`#296`）、
//! 总线轨的 `volume_db`（`#296`，输出级推子）、确定性 TPDF 抖动与
//! 24-bit 量化、RIFF/RF64/BW64 容器与 `bext` 元数据、峰值归一化。
//!
//! **明确没做**（只要工程里真的出现，就会同时出现在响应的 `unsupported` 与
//! `unsupportedCounts` 里，**绝不静默**）：
//!
//! | 键 | 含义 |
//! | :--- | :--- |
//! | `audioClips` | **收窄后的口径**：仅当工程在 `assets` 索引里**声明**了这个资产、而会话 CAS 池里**没有它的字节**时才登记（`Domain::open_in_memory` 注入的会话就是这种形态）。此时该片段当静音，且响应 `data.audio.assets[].bytesPresent = false` 如实说明。读 `.yeban` 容器时索引与字节由 `store` 双向校验必然同时存在，因此这一项在真实容器工程上**不会**出现 |
//! | `deviceChainDsp` | 设备链里**没有任何离线路径消费**的参数（未知旋钮 / `filter_cutoff_hz` / `reverb_mix` / `reverb_width`）没有求值；增益、EQ、低通、**压缩**、Freeverb、卷积已求值（压缩自 `#303`；`#303` 之前它**一整级不执行**）；⭐ G5：内置乐器（音源）的**引擎认下**的参数（音色 `osc1_wave`/`osc2_wave`、电平/失谐、滤波、鼓机键位与旋钮）由 `yeban-engine` 的音源投影**真的求值**（自 `#268`；判据 `every_consumed_instrument_param_reaches_the_engine_projection`），**不再**因此键登记 —— 只有音源**不认**的名字仍登记 |
//! | `externalPlugins` | `DeviceKind::ExternalInstrument/ExternalEffect` 没有宿主 |
//! | `automationLanes` | ⭐ `#243` / `#256` / `#257` / `#266` / `#267` / `#293` / `#297` / `#298` / `#304`：**残差桶** —— **仅**指仍有可求值点、但**这次渲染没有落点**的泳道（空泳道与 `readEnabled:false` 的泳道**不**登记）。具体是：`DeviceParam` 的名字不是插入链落点（**卷积 IR 形状** `conv_ir_decay*` / `conv_ir_seed` / 未知旋钮），或指向旁通/非对应器件族的来源设备；`TrackVolume` / `TrackPan` 落在**不可达轨**（不在保留子图）；`SendGain` 落在**被剪枝的边**；`Macro` 泳道没有落点（宏不存在 / 空 mappings / 某条 mapping 的目标本身无落点 / 目标是嵌套 `Macro`）。**已经有落点的都真的被求值、不在此列**：源节点 / 主总线 / **可达总线轨**的 `TrackVolume` 与 `TrackPan`（`#296` / `#298` 的输出级）；保留子图入边的 `SendGain`（`#256`）；`DeviceParam` 的通道条（**含压缩**）/ Freeverb / 卷积标量字段（`data.appliedDeviceParamNames` 逐条列出；`#266` / `#267` / `#293` / `#304`）；**有落点的 `Macro` 泳道按目标单位级联**（`#297`）。逐泳道的支持/应用读数见 `data.automation` |
//! | ~~`noteSlide`~~ | ⭐ **已退役（`#311b`）**：滑音由引擎**真的求值**（`SlideSpan` 进 `NoteSchedule`，末频精确落在目标音高），因此**不再**登记 —— 报一个已经做到的键就是假话。判据见 `yeban-engine/tests/pitch_bend.rs` |
//! | ~~`notePitchBend`~~ | ⭐ **已退役（`#311b`）**：弯音曲线由引擎**真的求值**（音符局部 64 样本网格重算 `phase_inc`），因此**不再**登记。判据见 `yeban-engine/tests/pitch_bend.rs` 与 `render_master.rs` 的 `the_master_bends...` |
//! | `noteLyrics` | **母带不会唱**：没有歌声合成（SVS 不存在），`syllable` / `phonemes` 不出声。⚠ 歌词**不是**"没有载体"：`syllable`（非空）已能写成 SMF `FF 05` 歌词元事件（`#312`），导入侧也按"唯一才归属"读回（`#317`）；`phonemes` 仍无 SMF 载体（SMF 只有一种文本槽，写进去等于把 SVS 对齐数据伪装成歌词 ⇒ 导出侧另报键 `phonemes`）。触发条件不变：任一音符带 `syllable` 或非空 `phonemes` 就登记 |
//! | `busTrackClips` | 非源节点（总线/主轨）上的片段没有渲染（只有源节点有音源） |
//! | `sidechainRouting` | 侧链边是**键输入**的数据形态，本渲染器**没有键输入消费面**（压缩器的检波器只读它自己正在压缩的那一路，`Compressor::process_gain` 不接受第二个输入）。⚠ `#315`：**这个键的判据是"图里有没有这种数据"**（一条两端都到不了 master 的悬挂边也照报），**说不出"发生了什么事"**。`#337` 起侧链边**不再**被当成普通音频边求和（修前平行时会 +6.0206 dB），因此"实际发生了什么"要看三个读数：`data.sidechainEdgesSummedAsOrdinaryAudio`（**真的被当成普通音频边求和进母带了几条**，修后恒 `0`）、`data.sidechainEdgesInRoutingGraph`（声明了几条）、以及 `data.sidechainEdgesDeclaredWithoutKeyInput`（**声明了、但没有键输入路径**的条数）。`0 / 0 / 0` = 没有侧链；`0 / N / N` = N 条侧链声明了，但一条都没有被求值（本切片没有检波器可喂）。修前 `#315` 的两个数**都不存在** ⇒ "报不支持却悄悄 +6.0206 dB"在响应里连一个字都没有（`#314` 的缺陷） |
//! | `sfzAmplitude` | ⭐ `#274`：命中区域的 **`amplitude_oncc*`（CC 调制那一半）** 没有求值 —— 静态 `amplitude` 已由引擎的 SFZ 支路应用；CC 增量需要 CC 状态，本层没有。**只**在真的声明了 CC 调制时出现（静态 `amplitude=50` ⇒ ⛔ 不报） |
//! | `sfzTuneCcModulation` | ⭐ `#223`：命中区域的 **`tune` 的 CC 调制**（`tune*ccN`，`pitch*ccN` 亦归此族）没有求值 —— 基准 `tune` 已烘进 `pitch_ratio`，CC 增量需要 CC 状态，本层没有。只在真的声明时出现 |
//! | `sfzVolumeCcModulation` | ⭐ `#223`：命中区域的 **`volume` 的 CC 调制**（`volume*ccN` / `gain_ccN`）没有求值 —— 基准 `volume` 已折进 `total_gain()`，CC 增量（dB）需要 CC 状态，本层没有。只在真的声明时出现 |
//!
//! ⚠ **`sfzSampler` 这个键已经退役**（`#221`）：`#220` 的第 324 条指出，模块文档
//! 曾承诺"会把 `sfzSampler` 放进 `unsupported`"，而代码里**没有任何地方读 `sampler`**
//! ⇒ 缺失连 `unsupported` 都不报。现在的处置是**真的支持**（`TrackV3.sampler` 由引擎的
//! SFZ 支路发声，读数进 `data.sources[].sampler`），因此那个键**不能**再出现在
//! `unsupported` 里 —— 报一个已经做到的键就是假话。
//!
//! ⭐ **`#274`：循环 / 播放区间 / 幅度包络 / 双层也是同一条道理** —— 采样器轨改由
//! 引擎的 SFZ 支路发声之后，引擎真的应用这四项（`PreparedRegion` 的
//! `loop_mode`/`span`/`ampeg_*`、以及"每条命中区域一个声部"）⇒ 那四个键退役
//! （17 → 13）。⚠ **声部池是 16 槽**：> 16 个同时声部会**窃取**，读数进
//! `data.sources[].sampler.voiceSteals`（第 324 条：⛔ 不许静默少做）。
//!
//! ⚠ **`sfzTrackPan` 这个键也已经退役**（`#255`）：修前 MCP 的 [`SamplerTrackSource`]
//! 与 CLI 的 SFZ 导出分支同口径地**不应用**声相，因此 `pan != 0` 时登记它；
//! `#255` 把**两侧一起**接上同一条等功率曲线（MCP 在采样器源上乘 `pan_gains`，
//! CLI 在 `--sfz` 旁路与引擎轨上乘 `audio_clip::track_pan_gains`）⇒ 采样器轨的 `pan`
//! **真的生效**，再报它就是说假话。`pan == 0` 仍走恒等快路径（⛔ 不乘 `√2/2`）。
//!
//! 已知不支持项的**全集**由 [`KNOWN_UNSUPPORTED_KEYS`] 唯一列出，并由
//! `tests/render_master.rs::the_known_unsupported_list_is_not_decoration` 逐条逼出读数
//! （清单里的每一项都必须在一个**具体输入**下真的出现在 `unsupported` 里）。
//! ⚠ 这里曾指向一个**不存在**的判据名（`every_known_unsupported_key_is_actually_reported`；
//! `grep 'fn every_known'` 为空）⇒ 照名字去核对的人什么也找不到。
//! 由 `tests/doc_criterion_references_resolve.rs` 常驻钉住"散文里点名的判据必须存在"。
//!
//! "工程里有 MIDI 音符或音频片段"这一条是渲染的**前提**：完全没有可渲染内容（0 帧）
//! 时返回 `RENDER_FAILED`，而不是写一个 0 帧的文件冒充成功。
//!
//! ## 循环重复：在**渲染层**展开（`#241`，`clipLoopRepetition` 已退役）
//!
//! `ClipPlacement::loop_config` 的 tick 坐标按**摆放相对**解释（与
//! `note.start_tick` / `placement.duration_ticks` 同一坐标系；旧的
//! `loop_requests_repetition` 本来就是这样读的，`yeban-render` 的两个导出器也
//! 用同一句"是否恰好覆盖摆放"）。展开规则**只有这一份**：
//!
//! - 启用且 `start_tick < end_tick < duration_ticks` ⇒ 循环区间
//!   `[start_tick, end_tick)` 以 `L = end_tick - start_tick` 为周期重复，第 `k`
//!   遍的相位是 `start_tick + k*L`，直到铺满 `duration_ticks`（`N = ceil((duration - start) / L)`）；
//! - **每一遍以循环区间末端为硬切边界**：区间内起音的音符尾巴裁到 `end_tick`，
//!   音频片段只搬 `[start, end)` 对应的那一段素材 ⇒ 每一遍逐位相同；最后一遍
//!   再裁到摆放末端（`start_tick + duration_ticks`）；
//! - `start_tick > 0` 时区间**之前**的内容播放**一遍**（不复制）；区间之后
//!   （起音 `>= end_tick`）的内容不参与循环演奏 —— 循环区间就是被演奏的内容；
//! - 区间终点 `>= duration_ticks`（含"恰好覆盖 `0..duration_ticks`"）⇒ 摆放内
//!   不会回卷 ⇒ 单遍播放，与"不循环"**逐位相同**；`enabled == false` 同理。
//!
//! ⇒ 任务单 `#241` 之后 [`KNOWN_UNSUPPORTED_KEYS`] 里**没有** `clipLoopRepetition`：
//! 报一个已经做到的键就是假话。MIDI 音符与音频片段走**同一个** `yeban_model::ClipPlacement::loop_repeat`，
//! 音频的每一遍靠 `ClipSpan::source_offset` 指向素材里循环区间的那一段。
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
//! ## 削波可见（#175；#174 实测坑⑧）
//!
//! 实测：响应里 `peak.before = 1.5748`，而写出的文件里 **6134 个样本**被钳在 `±1.0`
//! —— 调用方**无法**从峰值推出"有多少个样本真的被削平了"，于是不知道自己导出的
//! 是一份削波文件。现在响应有**三个**新读数（都在抖动/量化**之前**的浮点母带上量）：
//!
//! | 字段 | 单位 | 口径 |
//! | :--- | :--- | :--- |
//! | `clippedSamples` | **样本个数** | `|sample| > 1.0`（严格大于；`±1.0` 是 PCM 端点，不钳位） |
//! | `truePeakDbfs` | dBFS（0 = 满量程，正值 = 过载） | 16× 多相过采样真峰值（母带口径） |
//! | `headroomDb` | dB | `-truePeakDbfs`（负值 = 已过载） |
//!
//! 三者都**只读**：不改样本、不改 `bytes`、不改 `sha256`／`masterDigest`
//! （判据 `tests/render_master.rs` 的"默认路径逐字节不变"仍然钉住）。
//! 真峰值的实现走 `yeban-render::mastering::measure_master`（`yeban-dsp` 的
//! `TruePeakDetector`，唯一实现）；本层不复制第二份过采样器。
//!
//! ⚠ **`peak.before` / `peak.after` 的语义**：它们分别是**归一化前 / 归一化后**的
//! **样本峰值**（`max |sample|`），⛔ 不是写盘文件（TPDF 抖动 + 24-bit 量化之后）的
//! 峰值，也⛔ 不是真峰值。这句话同时写在响应的 `peak.semantics` 与
//! `tools/list` 的 `yeban_render_master` 描述里，不让调用方去猜。
//!
//! ## MIDI 音源 = 引擎音色（⭐ `#268` / `ADR-0011`；`#251` 的文案已改）
//!
//! `#251` 时 MIDI 音源是**内置正弦振荡器**，于是 `data.midiSource` 逐字写着
//! `kind = "builtin-sine"`、`engineTimbre = false`。`#268` 按负责人逐字裁决「统一」
//! 把音源换成 `yeban-engine` 的 `SynthEngine`（与界面 / CLI 的 `--export-wav`
//! **同一个**音色、同一条快照投影）⇒ 旧文案今天就是假话（第 324 条），本单改成：
//! `kind = "engine-instrument"`、`engineTimbre = true`，`note` 里逐字写
//! "与界面/CLI 同一个音色、⛔ 不再使用内置正弦振荡器"。
//!
//! ⚠ **这不是只改文案**：`MidiSynthSource` 已退役，母带样本因此真的变了
//! （`masterDigest` / `sha256` / `clippedSamples` 的旧→新读数见任务单 `#268` 的报告）。
//! ⭐ `#274`：绑定了 SFZ 采样器的轨**也走这条引擎音源**（引擎的 SFZ 支路）——
//! `SamplerTrackSource` / `sfz::prepare_sampler` 已退休，`sources[].sampler` 里另有读数。
//!
//! ## 削波**预警 + 建议 trim**（⭐ `#225`；⛔ **不自动改增益**）
//!
//! `#224` 的第三轮验收实测：**不加任何推子**、agent 编完直接导出，单轨峰值就达到
//! `1.874512`、`clippedSamples = 21634` —— 工具面当时**没有任何预警**，用户拿到一份
//! 削波的 WAV 还以为是正常的。`#175` 已经把**测量**补齐了（上表三个读数），缺的
//! 不是测量，是**提示 + 可执行建议**。
//!
//! 裁决（`#224`，本单照做）：给"削波预警 + 建议 trim"，⛔ **不是自动改增益**
//! —— 自动改就是**静默改用户意图**（`#224` 的实测参照正是靠手工
//! `track −6 dB` / `aux −12 dB` / `send −12 dB` 才把 `clippedSamples` 降到 0）。
//!
//! 交付形态：
//!
//! - `data.warnings[]`（**结果质量提示**通道，与提案响应的 `#175`/`#178` 同名同义）：
//!   两条来源 —— ① `clippedSamples > 0` 时**恰好一条** `code:"exportClipping"`；
//!   ② 可闻、排了内容、却**一个非零样本都没出**的源 ⇒ **一条** `code:"silentSources"`
//!   逐源点名（见 [`silent_sources_warning`]）。两条都不出现时是**空数组**。
//!   为什么**不**进 `unsupported` / [`KNOWN_UNSUPPORTED_KEYS`]：那份清单的口径是
//!   "我们会静默少做什么"（第 324 条：清单里的每一项都必须在一个具体输入下真的
//!   出现）。削波不是"少做了什么"—— 渲染把工程算对了，**结果本身**过载。塞进去会
//!   让清单含义漂移，也会让"每项可触发"的判据去构造一个不是功能缺口的输入。
//! - 每条预警可寻址：`clippedSamples` / `truePeakDbfs` / `headroomDb` /
//!   `suggestedTrimDb` / `suggestedTrimDbFormula` / `basis` / `loudestSource` / `hint`。
//! - **建议值公式（逐字）**：`suggestedTrimDb = -(ceil(truePeakDbfs * 2) / 2)`
//!   （向上取整到 0.5 dB ⇒ 减完保证 `truePeakDbfs <= 0`，最多多减 0.5 dB）。
//! - **最响的源**：`data.sources[]` 每条新增 `peak`（源节点输出的样本峰值，线性幅度）
//!   与 `peakDbfs`；预警的 `loudestSource` 点名其中最大的一条（并列取 `node`
//!   字典序最小 ⇒ 每次点同一个名字）。
//! - ⛔ **不动音频**：整文件 `sha256` / `masterDigest` 与加预警之前**逐字节相同**
//!   （判据 `tests/render_master.rs::the_clipping_warning_does_not_change_a_single_byte`
//!   用**加预警前**测到的常量钉住）。
//!
//! ⚠ 边界：逐源峰值的测点是**源节点输出**（含轨音量 / 声相 / 采样器与片段增益），
//! ⛔ 不含该轨插入链（EQ / 混响 / 卷积）与路由边增益 ⇒ 它与 `peak.after` 不是同一个数。
//! `suggestedTrimDb` 是**母带级**建议（减母带增益保证不削波）；只减最响的一条源
//! ⛔ **不保证**母带不削波（其余源仍在求和）—— 这句话逐字进 `warnings[].hint`。
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
//! ## 显式帧数（⭐ `#252`；`frames` / `durationSeconds`，⛔ 缺省不变）
//!
//! 缺陷（`#250` 勘察）：CLI 的 `--export-wav` 缺省导出**8 秒固定**
//! （`crates/yeban-app/src/export_wav.rs` 的 `frames: 48_000 * 8`），而本工具按
//! **工程末端**推导（[`math::ticks_to_frames`]）⇒ 同一工程两条导出的缓冲长度不同，
//! `masterDigest` 因此**永远比不了**。帧数是 `#250` 的 `C1` 判据（"两侧同
//! `masterDigest`"）的六个前置条件之一；其余五个里声相／采样率／归一化／无音频片段
//! 是**可对齐**的，抖动与容器是**刻意差异**（判据只比 `masterDigest`，⛔ 不比整文件
//! `sha256`）。本切片补的正是帧数这一条：让调用方把两侧的长度对齐。
//!
//! | 实参 | 类型 | 语义 |
//! | :--- | :--- | :--- |
//! | `frames` | 整数 | 显式总帧数（`1..=` 单次渲染帧数上限，1 小时 @ 48 kHz） |
//! | `durationSeconds` | 数字（秒） | 帧数 = `round(秒 × sampleRate)`（请求的采样率，⛔ 不是工程采样率） |
//!
//! 规则（[`validate`] 是唯一解析处；[`build`] 只读解析结果）：
//!
//! - 两个都不给 ⇒ `None` ⇒ **仍按工程末端推导**（＝修改前逐字节相同的默认路径）；
//! - 只给一个 ⇒ 用它；两个都给且换算结果**不同** ⇒ `INVALID_PARAMETER_RANGE`
//!   （报文带两个原值与换算出的帧数）；
//! - `frames <= 0`／`durationSeconds` 非有限或 `<= 0`／换算后 `0` 帧 ⇒
//!   `INVALID_PARAMETER_RANGE`（⛔ 不写哨兵、⛔ 不落盘）；
//! - 显式帧数超过单次渲染帧数上限 ⇒ 与工程末端推导**同一个**闸门
//!   （[`frame_ceiling`]）、同一条报文（`RENDER_FAILED` + `maxFrames`）。
//!
//! ⚠ `frames` 是**输出**采样率下的帧数（请求的 `sampleRate`）。显式帧数**大于**
//! 工程末端 ⇒ 末尾补静音；**小于** ⇒ 裁掉末端（音频片段的帧区间也被同一处夹住）。
//! ⛔ 本切片**不**改任何缺省：既有钉死常量一位不动（判据
//! `the_default_rendering_path_stays_bit_identical_with_an_explicit_frame_count`）。
//!
//! ## 逐轨贡献（⭐ `ADR-0020`；`contributions`，⛔ 缺省不变）
//!
//! `sources[].peak` 只给**一次渲染**里的逐源峰值；"拿掉这条轨后母带变了多少"此前
//! 只能靠调用方自己做 N 次静音渲染 + 本地逐样本比较（`creation_acceptance.rs` 的
//! WALL ⑦）。`contributions: true` 让工具自己做这件事：
//!
//! - **基线** = 本请求关掉归一化 / 目标响度 / 显式路径，帧数**钉成**母带的 `frames`
//!   （静音"最晚结束"的轨不许改变时间轴长度）；
//! - 对每条**非 master** 轨各渲染一次"只静音该轨"，与基线**逐样本**比，报逐样本差的
//!   RMS / 峰值（dBFS）。逐样本全等 ⇒ `null`；渲染前已静音 ⇒ `alreadyMuted: true`
//!   且两个差值为 `null`（⛔ 不假装量过）；
//! - 成本 = `1 + 未静音的非 master 轨数` 次额外渲染，如实写在
//!   `data.contributionsBasis.renders`；
//! - ⛔ **不改**母带字节：`sha256` / `masterDigest` 与 `contributions: false` 逐字节相同；
//! - 缺省 `false` ⇒ 响应里**不出现** `contributions` / `contributionsBasis` 两键。
//!
//! ⚠ 差值取自最终 24-bit 负载 ⇒ 它证明"这条轨留下多少能量"，
//! ⛔ 不证明"这条轨在音乐上该不该被听见"。
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
//! | `format` 不在白名单 / `sampleRate` 不在模型集合 / `normalize` 非布尔 / `path` 非字符串 / **`targetLufs` 非数字或不在 `[-70, 0]`** / **⭐ `frames` 非正整数或 `durationSeconds` 非有限正数或两者冲突** / **`contributions` 非布尔** / 输出路径为工程或锁文件 | `INVALID_PARAMETER_RANGE`（带内） |
//! | 没有活跃工程 | `NO_ACTIVE_PROJECT`（带内） |
//! | 0 帧、**显式帧数或工程末端推导**超出帧数上限、路由图非法、缺音轨/片段 | `RENDER_FAILED`（带内，`data` 带原因与规范 ID；超限带 `maxFrames`） |
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
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use serde_json::{Map, Value};

use yeban_decode::{DecodeError, DecodeOptions, resample_interleaved};
use yeban_model::music::{RATCHET_MAX, RATCHET_MIN};
use yeban_model::{
    AssetHash, AutomationTarget, AutomationWriteMode, ClipContent, DeviceKind, EntityId, PPQ,
    RoutingKind, SampleRate, TrackV3, YebanProjectV1,
};
use yeban_render::dither::{BitDepth, quantize};
// ⭐ `ADR-0011`（`#268`）：引擎音色源适配器（CSV：CLI 与 MCP 共用同一份实现）。
// 本 crate ⛔ 不直接依赖 `yeban-engine`（`scripts/gates/check_mcp_dependency_direction.py`）。
use yeban_render::engine_source::{ArmedSfzInput, ArmedSfzSample, EngineTimbre, EngineTrackSource};
// 插入链的投影与"是否还有未消费参数"的判据都住在 `yeban-render`：MCP 不复制
// 第二份白名单（`yeban-engine` 的投影是同一份规则的来源，但 MCP 不能依赖它）。
use yeban_render::automation::{
    APPLIED_DEVICE_PARAM_NAMES, LaneResidual, TrackGainAutomation, TrackPanAutomation,
    centered_pan_gains, compressing_nodes_with_lanes, device_param_automations,
    forced_insert_instances, macro_cascade_reaches, send_gain_automations,
};
use yeban_render::inserts::{
    project_inserts_with_forced_instances, resolve_device_param, track_has_unhandled_dsp,
};
use yeban_render::mastering;
use yeban_render::render::{
    AudioSource, BlockContext, L1_BLOCK_SIZE, OutputStage, RenderError, RenderOptions,
    RenderOutput, RenderPlan, db_to_linear, track_latencies,
};
// **响度计量的唯一实现**在 `yeban-dsp::loudness`（ITU-R BS.1770-4 的 K 加权 +
// 门限积分）。本模块**不复制**它一个表达式：走 `yeban-render` 的再导出，因为
// `scripts/gates/check_mcp_dependency_direction.py` 禁止 MCP 直接依赖 `yeban-dsp`
// （"不拖音频栈进 MCP"），而 `yeban-mcp -> yeban-render` 这条边早已存在。
use yeban_render::loudness::GatedLoudness;
use yeban_render::rf64::{Bext, ContainerKind, ContainerPlan, PcmFormat, write_container};
use yeban_render::rng::{DeterministicDitherRng, dither_rng_for};

use super::error::Fault;
use super::render_clip_math as clip_math;
use super::render_math as math;
// ⭐ `#221`：采样器音源**复用** `domain::sfz` 的那一份实现（区域选择 / 播放速率 /
// 总增益 / 样本来源）—— ⛔ 这里不写第二套"一个音符怎么响"。
use super::sfz;
use super::store;
use crate::tools::ErrorCode;

/// 允许的输出容器/编码。
pub const FORMATS: [&str; 3] = ["wav", "rf64", "bw64"];

/// 母带声道数（规范把 Master 定义为立体声总线；工程模型里没有声道数字段）。
pub const MASTER_CHANNELS: usize = 2;

/// 导出位深：**恒为 24-bit 整数 PCM**（[ARCH-FMT-001] 的内置高质量 TPDF 抖动路径）。
pub const OUTPUT_BIT_DEPTH: u16 = 24;

/// ⭐ `ADR-0030` D2：**分轨（stem）**容器位深 —— 32-bit IEEE 浮点、无抖动。
///
/// 口径来自规范 `ARCH-DSP-003`（冻结/分轨要求 32-bit Float），⛔ 不是选型自由。
/// 它**不**是 `yeban_render_master` 的位深（那条恒为 [`OUTPUT_BIT_DEPTH`]）。
pub const STEM_BIT_DEPTH: u16 = 32;

/// ⭐ `ADR-0030` D2：容器里的**样本格式**（写进 `fmt ` 的 `audioFormat` 与位深）。
///
/// ⚠ 这不是用户可给的参数：[`validate`] 恒填 [`Self::Pcm24`]（母带口径），只有
/// `yeban_export_stems` 在**内部**构造 [`Self::Float32`]。因此 `yeban_render_master`
/// 的默认路径逐字节不变（同值、同键、同容器实现）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleFormat {
    /// 24-bit 整数 PCM ＋ TPDF 抖动 —— `yeban_render_master` / SFZ / 录音的唯一路径。
    Pcm24,
    /// 32-bit IEEE 浮点、**不抖动**（`quantize(_, BitDepth::Float32, _)` 是直通分支，
    /// 非有限样本归 `0.0`）—— 分轨导出 [ARCH-DSP-003]。
    ///
    /// ⚠ 浮点容器同时改变 `fmt ` 的 `audioFormat`（`WAVE_FORMAT_IEEE_FLOAT`）与
    /// **`fact` chunk 的有无**（非 PCM 容器必须带 `fact`）—— 这两处都由
    /// `yeban-render` 的 `ContainerPlan` 按 `PcmFormat::is_float` 自动补，
    /// ⛔ 本层不自己拼头，因此这里**不**再复制一个 `is_float` 判断。
    Float32,
}

impl SampleFormat {
    /// 写进响应 `bitDepth` 与 `fmt ` chunk 的位深。
    #[must_use]
    pub const fn bit_depth(self) -> u16 {
        match self {
            Self::Pcm24 => OUTPUT_BIT_DEPTH,
            Self::Float32 => STEM_BIT_DEPTH,
        }
    }

    /// 抖动/量化目标（`yeban-render` 的 [`BitDepth`]，唯一入口）。
    const fn quantize_to(self) -> BitDepth {
        match self {
            Self::Pcm24 => BitDepth::Int24,
            Self::Float32 => BitDepth::Float32,
        }
    }
}

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
/// 单次渲染的帧数上限：1 小时 @ 48 kHz。
///
/// 不是性能目标，而是一道**防呆闸门**：工程里一个错误的 `duration_ticks`
/// （或极慢的 BPM）可以在几秒内申请几十 GB 缓冲。超过即 `RENDER_FAILED`
/// 并如实报出请求帧数与上限。
pub const MAX_RENDER_FRAMES: u64 = 48_000 * 3_600;

/// 源节点没有可识别名字时的文件名 stem。
pub const DEFAULT_STEM: &str = "master";

/// 音频源种类（响应 `sources[].kind`）：只有 MIDI 片段的源。
///
/// ⭐ `ADR-0011`（`#268`）：MIDI 音源已换成 `yeban-engine` 的 `SynthEngine`
/// （与界面 / CLI **同一个**音色）⇒ 标签从 `midi-synth-osc` 改成 `engine-instrument`。
/// 旧标签逐字描述的是**正弦振荡器**，留着就是假话（第 324 条）。
pub const SOURCE_KIND: &str = "engine-instrument";

/// 音频源种类：只有音频片段的源。
pub const AUDIO_SOURCE_KIND: &str = "audio-clip";

/// 音频源种类：MIDI 片段与音频片段**都有**的源。
pub const MIXED_SOURCE_KIND: &str = "engine-instrument+audio-clip";

/// 音频源种类：既没有音符也没有可渲染的音频片段（静音源）。
pub const SILENT_SOURCE_KIND: &str = "silent";

/// 音频源种类：⭐ `TrackV3.sampler` 绑定的**采样器音源**（`#221`）。
///
/// ⭐ `#274`：它现在是**引擎的 SFZ 支路**（`render_track` 的第 3 参 = 快照里的摊平表）
/// —— 与 CLI 的 `arm_pending_sfz` 同一条路径。本常量只负责**命名**
/// （`has_sampler && has_midi`），⛔ 不再对应第二种音源实现。
pub const SAMPLER_SOURCE_KIND: &str = "sfz-sampler";

/// 音频源种类：采样器音符与音频片段**都有**的源。
pub const SAMPLER_MIXED_SOURCE_KIND: &str = "sfz-sampler+audio-clip";

/// ⭐ `#243`：**音量自动化的读数块**（`data.automation`）与上面的 `unsupported`
/// 是**互补**的。`unsupported` 只说“少了什么”，`data.automation` 把每条泳道的
/// `points` / `readEnabled` / `writeMode` / `supported` / `evaluable` / `applied`
/// 与首末点逐条列出 —— 因此空泳道（`points: 0`）、关掉的泳道、master-only 与
/// bus-only 的泳道都**可见**，而不是靠一条布尔键去猜。
///
/// ⭐ **已知不支持项的全集**（第 324 条：`unsupported` 必须能列出全部已知不支持项）。
///
/// 这是本工具**唯一**的"我们会静默少做什么"的清单：
///
/// - [`note_unsupported`] 在 debug 断言里要求每一次登记都用**这里面的**键
///   ⇒ 加一个新键却忘了登记 ⇒ 判据当场红（而不是悄悄多一个没人知道的口径）；
/// - `tests/render_master.rs::the_known_unsupported_list_is_not_decoration` 反过来逼：
///   清单里的**每一项**都必须在一个**具体输入**下真的出现在 `unsupported` 里
///   ⇒ 清单不会腐烂成一张"反正也不会被检查"的装饰表。
///
/// ⚠ `#221` 之后 `sfzSampler` **不在**这里：采样器绑定现在真的被渲染（`#274` 起由
/// 引擎的 SFZ 支路发声），报它就是在说假话。
///
/// ⚠ `#255` 之后 `sfzTrackPan` 也**不在**这里：采样器轨的 `pan` 现在真的生效
/// （MCP 与 CLI 同时接上同一条等功率曲线），报它同样是在说假话。
///
/// ⭐ `#274`：**17 → 13** —— 采样器轨改由引擎的 SFZ 支路发声之后，这四项
/// **真的做到了**，报它们就是在说假话：
///
/// | 退役的键 | 为什么不再报 |
/// | :--- | :--- |
/// | `sfzLoopExpansion` | 引擎的 `PreparedRegion` 真的展开 `loop_mode`（`#91`） |
/// | `sfzSampleSpan` | 引擎真的按 `offset`/`end` 切片（`span`，步 3） |
/// | `sfzRegionEnvelope` | 引擎真的应用 `ampeg_*`（`#92`） |
/// | `sfzRegionLayering` | 引擎把**每一条**命中区域都建成声部并叠加（⛔ 不再只取第一条） |
///
/// ⭐ `#296`：**13 → 12** —— `masterPan` 退役。理由不是"我们决定不报它"，而是
/// 主总线的 `pan` **真的被乘了**：静态 `pan` 走 `RenderPlan` 的新增输出级
/// （`OutputStage::with_pan` + `centered_pan_gains`，`pan == 0` ⇒ `(1,1)`），
/// `TrackPan` 自动化在执行之后按块乘绝对左右增益（用户可见的功能缺失补上了）。
///
/// | 退役的键 | 为什么不再报 |
/// | :--- | :--- |
/// | `masterPan` | 主总线 `pan != 0` 现在真的改变母带左右声道（`#296`） |
///
/// ⚠ `sfzAmplitude` **收窄不退役**：静态 `amplitude` 已由引擎应用，但
/// `amplitude_oncc*` 需要 CC 状态（本层没有）⇒ 只有 **CC 那一半**仍登记。
/// `sfzTuneCcModulation` / `sfzVolumeCcModulation` 同理（模型没有 CC 状态）。
pub const KNOWN_UNSUPPORTED_KEYS: [&str; 10] = [
    "audioClips",
    "automationLanes",
    "busTrackClips",
    "deviceChainDsp",
    "externalPlugins",
    "noteLyrics",
    "sidechainRouting",
    "sfzAmplitude",
    "sfzTuneCcModulation",
    "sfzVolumeCcModulation",
];

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
    /// ⭐ `#252`：显式总帧数（`None` = 按工程末端推导 ＝ **默认路径**）。
    ///
    /// 给了 `durationSeconds` 时，这里存的是**换算结果**
    /// （`round(秒 × sampleRate)`）⇒ 渲染路径只需读这一个字段；原始秒数另存
    /// [`Self::duration_seconds_bits`] 供回显（⛔ 不用它参与渲染，避免两处换算）。
    pub frames: Option<u64>,
    /// ⭐ `#252`：调用方给的显式时长（秒）的 `f64` 位型。
    ///
    /// 为什么存位型：`#[derive(Eq)]` 对 `f64` 不成立，而本类型要能整体比较。
    /// 读出时用 [`Self::duration_seconds`]。
    pub duration_seconds_bits: Option<u64>,
    /// 响度目标（LUFS）；`None` = **只报读数、不做判定**（也是默认路径）。
    ///
    /// 用「`f32` 的位型」参与 `Eq`（`#[derive(Eq)]` 对 `f32` 不成立），因此这里存
    /// 判定所需的**位型**，读出时用 [`target_lufs`](Self::target_lufs) 还原。
    pub target_lufs_bits: Option<u32>,
    /// 显式输出路径（`None` = 按缺省规则从工程路径派生）。
    pub path: Option<PathBuf>,
    /// ⭐ `ADR-0020`：是否额外报**逐轨贡献**（默认 `false`）。
    ///
    /// `true` 时 [`track_contributions`] 会额外做 `1 + N` 次渲染（`N` = **未静音**的
    /// 非 master 音轨数）：一次**未归一化基线**，然后每条轨各一次"只静音该轨"的渲染。
    /// 它**不**改写盘字节（`sha256` / `masterDigest` 与 `false` 时逐字节相同），
    /// 只往响应里加 `data.contributions` / `data.contributionsBasis`。
    /// ⚠ 缺省时响应里**没有**这两个键，但 `data.request` 的回显多一个 `contributions: false`。
    pub contributions: bool,
    /// ⭐ `ADR-0030` D2：容器**样本格式**（母带 = [`SampleFormat::Pcm24`]，分轨 = `Float32`）。
    ///
    /// ⛔ **不是**用户可给的实参：[`validate`] 恒产出 `Pcm24`，所以 `yeban_render_master`
    /// 的默认路径（含 `data.request` 回显）逐字节不变。分轨路径在内部构造 `Float32`
    /// 的副本请求（`normalize:false` + 钉死帧数），因此母带那条 24-bit/TPDF 路径
    /// 一位都不经过这条分支。
    pub sample_format: SampleFormat,
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

    /// ⭐ `#252`：调用方给的显式时长（秒）；`None` = 没给 `durationSeconds`。
    ///
    /// 只用于回显。渲染用的帧数是 [`Self::frames`]（已在校验期换算好）。
    #[must_use]
    pub fn duration_seconds(&self) -> Option<f64> {
        self.duration_seconds_bits.map(f64::from_bits)
    }

    /// ⭐ `#252`：这次请求的帧数从哪来（回显用：`"frames"` / `"durationSeconds"` /
    /// `"projectEnd"`）。
    ///
    /// 默认路径（两个实参都没给）是 `"projectEnd"` —— 与修改前**同一条**推导。
    #[must_use]
    pub fn frames_source(&self) -> &'static str {
        match (self.frames.is_some(), self.duration_seconds_bits.is_some()) {
            (false, false) => "projectEnd",
            (true, true) => "durationSeconds",
            (true, false) => "frames",
            // 校验期不会造出「只给了秒数却没换算成帧」的请求；万一有，如实说没有显式帧数。
            (false, true) => "projectEnd",
        }
    }

    /// 诚实描述这次请求（`dryRun` 与响应共用的 `request` 字段）。
    #[must_use]
    pub fn to_value(&self) -> Value {
        serde_json::json!({
            "format": self.format(),
            "sampleRate": self.sample_rate.hz(),
            "normalize": self.normalize,
            // ⭐ `#252`：`frames` = **显式**帧数（`null` = 按工程末端推导）；与响应
            //   顶层的 `data.frames`（实际长度）不是同一个含义 ⇒ 两者都报。
            "frames": self.frames,
            "durationSeconds": self.duration_seconds(),
            "framesSource": self.frames_source(),
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
            "bitDepth": self.sample_format.bit_depth(),
            "channels": MASTER_CHANNELS,
            "contributions": self.contributions,
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

/// ⭐ `#225`：削波时**唯一**的那条预警（`data.warnings[]` 的一个元素，可寻址）。
///
/// ## 为什么走 `warnings[]` 而不是 `unsupported`
///
/// `unsupported` / `KNOWN_UNSUPPORTED_KEYS`（`#221`/`#223`，现 12 项）的口径是
/// "**我们会静默少做什么**"（第 324 条：清单必须逐项可触发）。削波不是"少做了什么"
/// —— 渲染把工程里的一切都算对了，**结果本身**就是过载的。把它塞进
/// `KNOWN_UNSUPPORTED_KEYS` 会让那份清单的含义漂移（"不支持削波"？说不通），
/// 也会让"每项可触发"的判据去构造一个**不是功能缺口**的输入。
/// `warnings[]` 是**结果质量提示**的既有通道（`#175`/`#178` 用在提案响应上），
/// 语义正好：**非阻断、可寻址、不影响落盘**。
///
/// ## ⛔ 它不改增益
///
/// 本函数只读 `clipped_samples` / `true_peak_dbfs` / `peak_after` / `reports`，
/// 返回一个 JSON 值。一个样本都不碰 —— 整文件 `sha256` 与加预警之前**逐字节相同**
/// （判据 `tests/render_master.rs::the_clipping_warning_does_not_change_a_single_byte`）。
///
/// ## 建议值
///
/// `suggestedTrimDb = -(ceil(truePeakDbfs * 2) / 2)`（公式逐字进响应，见
/// [`math::suggested_trim_db`]）。`clippedSamples == 0` ⇒ `None`（**没有这条预警**，
/// 判据钉住"不是恒报"）。
fn clipping_warning(
    clipped_samples: u64,
    true_peak_dbfs: Option<f32>,
    headroom_db: Option<f32>,
    peak_after: f32,
    reports: &[SourceReport],
) -> Option<Value> {
    if clipped_samples == 0 {
        return None;
    }
    // 建议值必须**算得出来**：真峰值可测就用它；测不出（本渲染链只在"采样率没有
    // 核验系数"或"全零"时发生）就退到样本峰值，并在 `basis` 里逐字说明用的是哪个。
    let (basis, basis_dbfs) = match true_peak_dbfs {
        Some(peak) if peak.is_finite() => ("truePeakDbfs", peak),
        _ => (
            "samplePeakDbfs",
            math::amplitude_dbfs(peak_after).unwrap_or(0.0),
        ),
    };
    let suggested_trim_db = math::suggested_trim_db(basis_dbfs);
    // 公式逐字进响应；`basis` 缺省是 `truePeakDbfs`，只有在真峰值测不出时
    // 才如实换成 `samplePeakDbfs`（⛔ 不把两种口径混成一句）。
    let formula = format!("suggestedTrimDb = -(ceil({basis} * 2) / 2)");
    // 最响的源：逐源峰值里最大的那条；并列时取 `node` 字典序**最小**的
    // （`max_by` 的"更大"⇒ 反向比较 node），保证同一工程每次点同一个名字。
    let loudest = reports
        .iter()
        .filter(|report| report.peak > 0.0)
        .max_by(|left, right| {
            left.peak.total_cmp(&right.peak).then_with(|| {
                right
                    .node
                    .to_canonical_string()
                    .cmp(&left.node.to_canonical_string())
            })
        });
    let loudest_value = loudest.map_or(Value::Null, |report| {
        let source_dbfs = math::amplitude_dbfs(report.peak);
        serde_json::json!({
            "node": report.node.to_canonical_string(),
            "track": report.track.clone(),
            "kind": report.kind,
            "peak": report.peak,
            "peakDbfs": json_f32(source_dbfs),
            // 只把**这一条源**压到 0 dBFS 以下需要的量；⛔ 不是母带保证（见 hint）。
            "suggestedSourceTrimDb": json_f32(source_dbfs.map(math::suggested_trim_db)),
        })
    });
    let loudest_hint = loudest.map_or_else(
        || "本次没有任何源产生非零样本（母带过载来自别处？）".to_owned(),
        |report| {
            let source_dbfs = math::amplitude_dbfs(report.peak);
            let source_dbfs_text = source_dbfs
                .map_or_else(|| "null".to_owned(), |value| format!("{value:.6}"));
            // ⚠ 实测：`#224` 那种"多源求和"的工程里，最响的**单条源**自己可能
            // 根本没有过满量程（`peakDbfs <= 0`）⇒ 单独减它**不会**解决。
            // 这一句必须说出来，否则"点名最响的源"会被误读成"减它就对了"。
            let summing = source_dbfs.is_none_or(|value| math::suggested_trim_db(value) == 0.0);
            let summing_note = if summing {
                "；⚠ 它**自己**没有过满量程 ⇒ 削波来自**多源求和**，⛔ 单独减这一条不会解决，\
                 请按 suggestedTrimDb 减母带（或逐条配平）"
            } else {
                ""
            };
            format!(
                "最响的源是 `{}`（node `{}`, kind `{}`, peakDbfs = {source_dbfs_text}{summing_note}）",
                report.track,
                report.node.to_canonical_string(),
                report.kind
            )
        },
    );
    let hint = format!(
        "这次导出的母带会削波: clippedSamples = {clipped_samples}（这些样本会被 \
         24-bit 量化器钳在 ±1.0）({basis} = {basis_dbfs:.6} dBFS, headroomDb = {} dB)。\
         建议把母带增益减 {:.1} dB（{formula}）。\
         {loudest_hint}；想从源上减就改那条轨的 volumeDb \
         (yeban_edit_notes 的 `{{\"kind\":\"setParam\",\"lane\":\"TrackVolume\",\"value\":<新 dB>}}`)。\
         ⚠ 只减最响的一条源 ⛔ 不保证母带因此不削波（其余源仍在求和）；\
         ⛔ 本工具**不自动改增益** —— 这里只是读数 + 建议，音频一个字节都没动。\
         逐源峰值见 data.sources[].peak / .peakDbfs。",
        headroom_db.map_or_else(|| "null".to_owned(), |value| format!("{value:.6}")),
        suggested_trim_db.abs()
    );
    Some(serde_json::json!({
        "code": "exportClipping",
        "severity": "warning",
        "blocking": false,
        "clippedSamples": clipped_samples,
        "truePeakDbfs": json_f32(true_peak_dbfs),
        "headroomDb": json_f32(headroom_db),
        "suggestedTrimDb": suggested_trim_db,
        "suggestedTrimDbFormula": formula,
        "basis": basis,
        "loudestSource": loudest_value,
        "hint": hint,
    }))
}

/// **静音源预警**（`data.warnings[]` 的第二条来源）：可闻、排了内容，却一个非零样本都没出。
///
/// ## 为什么有它（实测缺陷，G1）
///
/// 28 轨工程里 12 条轨 `peak = 0.0` 地"渲染成功"了：离线源每台引擎只渲染自己那条轨，
/// 却把整份快照塞进 16 个槽 ⇒ 第 17 条起拿不到槽位、整轨数字静音（根因见
/// `yeban_engine::synth::SynthEngine::arm_track_from_snapshot`）。回执里没有任何提示，
/// 只有逐条看 `sources[].peak` 才能发现 —— 这个函数把那件事**点名**。
///
/// 三条同时成立才点名：
/// 1. `audible` —— `mute` / `solo` 门控下本来就应当静音的源**不**报（那不是缺陷）；
/// 2. `peak == 0.0` —— 源节点输出一个非零样本都没有；
/// 3. `notes > 0 || audio_clips > 0` —— 它**确实排了**会发声的内容
///    （空轨不点名；被概率判为不触发的音符不计入 `notes`）。
///
/// ⛔ 只生成 JSON：不改样本 / 不改 `bytes` / 不改 `sha256`。
#[must_use]
fn silent_sources_warning(reports: &[SourceReport]) -> Option<Value> {
    let silent: Vec<&SourceReport> = reports
        .iter()
        .filter(|report| {
            report.audible && report.peak == 0.0 && (report.notes > 0 || report.audio_clips > 0)
        })
        .collect();
    if silent.is_empty() {
        return None;
    }
    let named: Vec<Value> = silent
        .iter()
        .map(|report| {
            serde_json::json!({
                "node": report.node.to_canonical_string(),
                "track": report.track.clone(),
                "kind": report.kind,
                "notes": report.notes,
                "audioClips": report.audio_clips,
                "notesSilenced": report.notes_silenced,
                // ⚠ 不在这里复制 `sampler` 子对象：`sources[].sampler` 是它的**唯一**形状，
                //    同名键在别处长成另一副样子会让"同名同义"失效（见模块头的读数约定）。
            })
        })
        .collect();
    let hint = format!(
        "本次渲染里有 {} 条源被判定为**可闻**、也确实排了内容（音符或音频片段），\
         但源节点输出**一个非零样本都没有** —— 它们没有出现在母带里。\
         请逐条检查：轨是否被别的 solo 门控（`audible` 已按 mute/solo 判过）、\
         轨推子/声相是否为 0、路由边是否断开、采样器是否缺样本\
         （`sources[].sampler.missingSamplePaths`）。\
         ⚠ 若这些轨本来就不该出声，请删掉它们的内容，而不是让「渲染成功」掩盖静音。\
         ⛔ 本工具**不自动修**，音频一个字节都没动。",
        silent.len()
    );
    Some(serde_json::json!({
        "code": "silentSources",
        "severity": "warning",
        "blocking": false,
        "count": silent.len(),
        "sources": named,
        "hint": hint,
    }))
}

/// 校验渲染参数。
///
/// # Errors
///
/// - 未知 `format` → `INVALID_PARAMETER_RANGE`（带白名单）；
/// - `sampleRate` 不在模型层允许集合 → `INVALID_PARAMETER_RANGE`；
/// - `normalize` 不是布尔 / `path` 不是非空字符串 → `INVALID_PARAMETER_RANGE`；
/// - `targetLufs` 不是有限数、或不在 [`TARGET_LUFS_RANGE`] 内 →
///   `INVALID_PARAMETER_RANGE`（带区间）；
/// - ⭐ `#252`：`frames` 不是 `> 0` 的整数 / `durationSeconds` 不是有限正数 /
///   两者换算到**不同**帧数 → `INVALID_PARAMETER_RANGE`；显式帧数超过
///   单次渲染帧数上限 → 与工程末端推导同一个 `RENDER_FAILED`（带 `maxFrames`）。
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
    // ⭐ `#252`：显式帧数 / 时长。两者都不给 ⇒ `None` ⇒ 仍按工程末端推导（默认路径）。
    let requested_frames = match arguments.get("frames") {
        None => None,
        Some(Value::Number(number)) => Some(
            number
                .as_u64()
                .filter(|frames| *frames > 0)
                .ok_or_else(|| {
                    Fault::domain(
                        ErrorCode::InvalidParameterRange,
                        format!("`frames` 必须是 > 0 的整数, 实际收到 {number}"),
                    )
                })?,
        ),
        Some(other) => {
            return Err(Fault::domain(
                ErrorCode::InvalidParameterRange,
                format!("`frames` 必须是 > 0 的整数, 实际收到 {other}"),
            ));
        }
    };
    let duration_seconds_bits = match arguments.get("durationSeconds") {
        None => None,
        Some(Value::Number(number)) => {
            // `as_f64` 覆盖整数与浮点两种 JSON 形状（与 `targetLufs` 同一收窄口径）。
            let seconds = number.as_f64().unwrap_or(f64::NAN);
            if !seconds.is_finite() || seconds <= 0.0 {
                return Err(Fault::domain_with_data(
                    ErrorCode::InvalidParameterRange,
                    format!("`durationSeconds` 必须是 > 0 的有限秒数, 实际收到 {seconds}"),
                    serde_json::json!({
                        "field": "durationSeconds",
                        "received": seconds,
                    }),
                ));
            }
            Some(seconds.to_bits())
        }
        Some(other) => {
            return Err(Fault::domain(
                ErrorCode::InvalidParameterRange,
                format!("`durationSeconds` 必须是 > 0 的有限秒数, 实际收到 {other}"),
            ));
        }
    };
    // 秒数 ⇒ 帧数用**请求的**采样率（⛔ 不是工程采样率）：渲染路径按它重采样。
    let duration_frames = duration_seconds_bits
        .map(|bits| (f64::from_bits(bits) * f64::from(sample_rate.hz())).round());
    let frames = match (requested_frames, duration_frames) {
        (None, None) => None,
        (Some(frames), None) => Some(frames),
        (None, Some(derived)) => Some(derived as u64),
        // 两个都给 ⇒ 必须指向**同一个**帧数；否则拒绝（⛔ 不悄悄选一个）。
        (Some(frames), Some(derived)) => {
            if derived < 1.0 || derived as u64 != frames {
                let seconds = duration_seconds_bits.map_or(0.0, f64::from_bits);
                return Err(Fault::domain_with_data(
                    ErrorCode::InvalidParameterRange,
                    format!(
                        "`frames` = {frames} 与 `durationSeconds` = {seconds} 冲突: \
                         按 sampleRate = {} Hz 换算得到 {derived} 帧; 只给一个, \
                         或让两者指向同一个帧数",
                        sample_rate.hz()
                    ),
                    serde_json::json!({
                        "frames": frames,
                        "durationSeconds": seconds,
                        "durationFrames": derived,
                        "sampleRate": sample_rate.hz(),
                    }),
                ));
            }
            Some(frames)
        }
    };
    if let Some(frames) = frames {
        // 0 帧（秒数太小被 round 成 0）在参数层就是**非法**，⛔ 不交给渲染去"渲染 0 帧"。
        if frames == 0 {
            return Err(Fault::domain(
                ErrorCode::InvalidParameterRange,
                format!(
                    "`durationSeconds` = {} 按 {} Hz 换算后是 0 帧; 需要至少 1 帧",
                    duration_seconds_bits.map_or(0.0, f64::from_bits),
                    sample_rate.hz()
                ),
            ));
        }
        // 上限与工程末端推导**同一处**闸门（`RENDER_FAILED` + `maxFrames`）。
        frame_ceiling(frames)?;
    }
    // ⭐ `ADR-0020`：逐轨贡献开关。缺省 `false` ⇒ 渲染路径与响应逐字节不变。
    let contributions = match arguments.get("contributions") {
        None => false,
        Some(Value::Bool(flag)) => *flag,
        Some(other) => {
            return Err(Fault::domain(
                ErrorCode::InvalidParameterRange,
                format!("`contributions` 必须是布尔值, 实际收到 {other}"),
            ));
        }
    };
    // ⭐ `ADR-0030` D2：样本格式**不进参数面** —— `validate` 恒产出母带口径（24-bit 整数
    // PCM ＋ TPDF 抖动）。分轨导出在 `domain::stems` 里构造 `Float32` 的副本请求。
    Ok(RenderRequest {
        format_index,
        sample_rate,
        normalize,
        frames,
        duration_seconds_bits,
        target_lufs_bits,
        path,
        contributions,
        sample_format: SampleFormat::Pcm24,
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

/// 一条**采样器轨**的实测读数（进响应 `data.sources[].sampler`，⭐ `#221`）。
///
/// ⚠ 这些字段的**存在理由**是让"绑了采样器"与"绑了但没生效"在响应里长得不一样：
/// `#220` 的缺陷 ② 正是这两者**逐字节相同**（同一工程经 MCP 渲染与"去掉 sampler"
/// 相同，且 `unsupported=[]`）。
///
/// ⚠ ⭐ `#275`：**采样器轨没有正弦回退** —— `TrackV3.sampler` 走引擎的 SFZ 支路
/// （`#274`），缺样本 ⇒ 静音 ＋ [`Self::notes_without_sample`]，⛔ 不发明替代音源。
/// 因此本报告**没有** `notesUsingSineFallback` 键：那个键只在 `yeban_render_sfz_wav`
/// 这条**旁路**上存在（会话乐器 ＋ 可缺文件的试听，见 `domain/sfz.rs`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SamplerReport {
    /// 绑定的路径（模型里的原样字符串）。
    pub path: String,
    /// 实际读的那份 SFZ 文件（路径解析后的绝对/相对形态）。
    pub resolved_path: String,
    /// SFZ 区域总数。
    pub regions: usize,
    /// ⭐ **引擎摊平口径**的静音区域数（缺样本 ＋ 内建静音；决定该轨是否发声）。
    ///
    /// 与 `yeban_load_sfz` / `yeban_list_instruments` 的 `silentRegionCount` **同一个
    /// 含义**（报告缺口 `G4` 起对齐；`builtinSilentRegionCount` 才是内建静音子集）。
    pub silent_regions: usize,
    /// 排程进采样器的音符数。
    pub notes: u64,
    /// 没匹配到任何区域的音符数（合法：该音高在这份 SFZ 里不发声）。
    pub notes_unmatched: u64,
    /// 用文件样本发声的音符数。
    pub notes_using_file_sample: u64,
    /// ⭐ `#274`：有命中区域、但**没有任何已解码样本**的音符数（引擎静音 ＋ 计数）。
    pub notes_without_sample: u64,
    /// 落在内建静音区域上的音符数。
    pub notes_using_builtin_silence: u64,
    /// 落在要求循环的区域上的音符数（⭐ `#274` 起引擎**真的**展开循环；这仍是读数）。
    pub notes_looping: u64,
    /// 真的读到的样本文件来源（升序去重）。
    pub sample_sources: Vec<String>,
    /// 读失败的样本文件（升序去重）。
    pub sample_failures: Vec<String>,
    /// 累计解码帧数。
    pub sample_frames_loaded: u64,
    /// 本次装载是否命中样本读盘上界（MCP 侧的三条上限；⛔ 不是"静默截断"）。
    pub sample_reads_truncated: bool,
    /// 命中的区域里声明了 `ampeg_*` 的个数（⭐ `#274` 起引擎**应用**包络；这仍是读数，
    /// ⛔ 不再登记 `sfzRegionEnvelope`）。
    pub regions_with_envelope: u64,
    /// ⭐ `#274`：**真的解码成功**的去重样本数（＝ 提供表长度；引擎摊平口径）。
    pub decoded_samples: usize,
    /// ⭐ `#274`：引擎摊平表报出的**缺样本路径**（升序去重；`silent` 里除内建静音
    /// 以外的那一半）。
    pub missing_sample_paths: Vec<String>,
    /// ⭐ `#274`：**声部窃取**数（引擎 SFZ 声部池 16 槽；池满 ⇒ 窃取最老的声部）。
    ///
    /// 由渲染期的 [`EngineTrackSource::steal_counter`] 填回（构造期恒 `0`）——
    /// 第 324 条：MCP 现在**会**窃取（> 16 个同时发声的声部），这件事必须能看见，
    /// ⛔ 不许静默少做。
    pub voice_steals: u64,
}

impl SamplerReport {
    /// ⭐ 与 app 的 `--export-wav` **同形**的 `sfz(...)` 读数行（`#221` 判据 3/4 直接比它）。
    ///
    /// 形态逐字照 `yeban-app` 的 `render_core::sfz_decode_label`：
    /// `sfz(tracks=1 regions=R samples=S silent=N missing=[…] unreadable(not_found=A,unsupported=B,other=C))`。
    ///
    /// ⚠ 两条读数**口径差**如实登记：
    /// - `samples` 在这里数的是"**真的读到**的不同样本文件数"（`sampleSources` 的长度）；
    /// - `missing` 数的是"**声明了 `sample=` 却读不到**的不同文件路径"（从
    ///   `sampleFailures` 里取路径，不是整条原因串）；
    /// - `unreadable(...)` 的三档分类 MCP 不做（它没有 app 侧的 `SampleReadFailure`
    ///   分档）：三档恒为 `0`，**原因串**逐条在 `sampleFailures` 里。
    #[must_use]
    pub fn label(&self) -> String {
        // ⭐ `#274`：`samples` / `missing` 与 `silent` 现在是**引擎摊平表**的口径
        // （`PreparedSfz` 的 `sample_count` / `missing_sample_paths` /
        // `silent_region_count`）—— 与 CLI 的 `sfz_decode_label` **同一个来源**，
        // 因此两侧的 `sfz(...)` 行逐字相同。
        let missing: Vec<&str> = self
            .missing_sample_paths
            .iter()
            .map(String::as_str)
            .collect();
        format!(
            "sfz(tracks=1 regions={} samples={} silent={} missing={missing:?} \
             unreadable(not_found=0,unsupported=0,other=0))",
            self.regions, self.decoded_samples, self.silent_regions,
        )
    }

    /// 进响应的 JSON 形状。
    #[must_use]
    pub fn to_value(&self) -> Value {
        serde_json::json!({
            "path": self.path,
            "resolvedPath": self.resolved_path,
            "regions": self.regions,
            "silentRegionCount": self.silent_regions,
            "notes": self.notes,
            "notesUnmatched": self.notes_unmatched,
            "notesUsingFileSample": self.notes_using_file_sample,
            "notesWithoutSample": self.notes_without_sample,
            "notesUsingBuiltinSilence": self.notes_using_builtin_silence,
            "notesLooping": self.notes_looping,
            "sampleSources": self.sample_sources,
            "sampleFailures": self.sample_failures,
            "sampleFramesLoaded": self.sample_frames_loaded,
            "sampleReadsTruncated": self.sample_reads_truncated,
            "regionsWithEnvelope": self.regions_with_envelope,
            "decodedSamples": self.decoded_samples,
            "missingSamplePaths": self.missing_sample_paths,
            "voiceSteals": self.voice_steals,
            "label": self.label(),
        })
    }
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
    /// ⭐ `#225`：这个源节点输出的**样本峰值**（`max |sample|`，线性幅度，两声道合计）。
    ///
    /// 测点是 `yeban-render` 的源节点输出（[`PeakTap`] 在 `render_block` 返回后扫一遍），
    /// 因此**含**轨音量 / 声相 / 采样器与音频片段增益，⛔ **不含**该轨的插入链
    /// （EQ / 混响 / 卷积在 `render_node` 里、探针之后才处理）与路由边增益。
    /// 单位与 `peak.after` 相同（满刻度线性幅度，⛔ 不是 dB、⛔ 不是文件峰值）。
    pub peak: f32,
    /// ⭐ 采样器绑定（`TrackV3.sampler`）的实测读数；`None` = 这条轨没有绑采样器。
    pub sampler: Option<SamplerReport>,
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
            // ⭐ #225：逐源峰值 —— 削波预警要能点名"该减谁"。
            "peak": self.peak,
            "peakDbfs": json_f32(math::amplitude_dbfs(self.peak)),
            "sampler": self.sampler.as_ref().map(SamplerReport::to_value),
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

/// 一条自动化泳道的**只读**读数（`data.automation.tracks[].lanes[]`）。
///
/// 口径（与求值入口逐条对齐）：
///
/// - `evaluable` = 泳道有点 **且** `readEnabled == true`（`automation_value_at` 的
///   三个“无值”出口里的两个）；空泳道与关掉的泳道因此**不可求值**；
/// - `supported` = 本渲染器真的消费这个目标（唯一判定是
///   `yeban_render::automation::unsupported_lane_reason`）：`TrackVolume` / `TrackPan`
///   在主总线与保留子图里的**任何节点**（源轨、可达总线轨）上都有落点（`#296` /
///   `#298` 的输出级）；`SendGain` 的落点是那条**入边**在 `RenderPlan` 里的归约
///   （`#256`）；`DeviceParam` 的落点是该节点的插入链（通道条 — 含压缩 — /
///   Freeverb / 卷积标量，`#266` / `#267` / `#293` / `#304`）；`Macro` 的落点是
///   "宏存在 ∧ 至少一条 mapping ∧ 每条 mapping 的目标都落点"（`#297`）。不可达轨、
///   被剪枝的边、没有落点的名字与宏仍登记 `automationLanes`；
/// - `applied` = `evaluable && supported`；
/// - `unsupported` = `evaluable && !supported`（调用方据此登记 `automationLanes`）。
///
/// ⚠ 空泳道会出现在读数里（`points: 0`, `evaluable: false`），但**不**进
/// `unsupported` —— 一条没有点的曲线没有“少做了什么”。
#[derive(Clone, Debug, PartialEq)]
pub struct AutomationLaneReport {
    /// 目标变体名（`TrackVolume` / `TrackPan` / `SendGain` / `DeviceParam` / `Macro`）。
    pub target: &'static str,
    /// 采样点数。
    pub points: usize,
    /// `readEnabled` 开关。
    pub read_enabled: bool,
    /// 写模式（`Off` / `Write` / `Touch` / `Latch`）。
    pub write_mode: &'static str,
    /// 目标的静态值（dB / 双极 / 归一化；不可对账时为 `None`）。
    pub static_value: Option<f32>,
    /// 本渲染器是否消费该目标。
    pub supported: bool,
    /// 是否有可求值点（有点且读开关打开）。
    pub evaluable: bool,
    /// 是否真的被应用到这次渲染。
    pub applied: bool,
    /// 首点 tick。
    pub first_tick: Option<u64>,
    /// 末点 tick。
    pub last_tick: Option<u64>,
    /// 首点值。
    pub first_value: Option<f32>,
    /// 末点值。
    pub last_value: Option<f32>,
    /// ⭐ `#263`：`DeviceParam` 的**序数地址**此刻解析到的设备身份（其它目标为 `None`）。
    ///
    /// 有了它，"插一台设备就静默改指"从不可见变成**读数**：同一个泳道在插设备前后
    /// 报出**两个不同的** `resolved_device_id`，而 `supported` / `evaluable` 一位不变。
    pub resolved_device_id: Option<EntityId>,
    /// ⭐ `#263`：解析到的参数名**原文**（解析失败 / 非设备参数为 `None`）。
    ///
    /// ⚠ 名字解析到了、但 DSP 不认时本字段**仍然报名字**，原因写进 [`Self::unresolved`]。
    pub resolved_param_name: Option<String>,
    /// ⭐ `#263`：未解析（或名字不在白名单）的**原因串**；`None` = 完全解析且支持。
    ///
    /// 词表只有一个来源：`yeban_render::inserts::UnresolvedReason::as_str` /
    /// `ResolvedDeviceParam::unresolved_reason`（本模块⛔ 不拼第二份字面量）。
    pub unresolved: Option<&'static str>,
}

impl AutomationLaneReport {
    /// 进响应的 JSON 形状。
    #[must_use]
    pub fn to_value(&self) -> Value {
        serde_json::json!({
            "target": self.target,
            "points": self.points,
            "readEnabled": self.read_enabled,
            "writeMode": self.write_mode,
            "staticValue": json_f32(self.static_value),
            "supported": self.supported,
            "evaluable": self.evaluable,
            "applied": self.applied,
            "firstTick": self.first_tick,
            "lastTick": self.last_tick,
            "firstValue": json_f32(self.first_value),
            "lastValue": json_f32(self.last_value),
            // ⭐ `#263`：序数地址的可观测解析（非设备参数一律 `null`，⛔ 不静默省略键）。
            "resolvedDeviceId": self.resolved_device_id.map(|id| id.to_canonical_string()),
            "resolvedParamName": self.resolved_param_name,
            "unresolved": self.unresolved,
        })
    }
}

/// 一条音轨的自动化读数（`data.automation.tracks[]`）。
#[derive(Clone, Debug, PartialEq)]
pub struct AutomationTrackReport {
    /// 音轨身份。
    pub node: EntityId,
    /// 音轨名。
    pub track: String,
    /// 泳道（按目标序）。
    pub lanes: Vec<AutomationLaneReport>,
}

impl AutomationTrackReport {
    /// 进响应的 JSON 形状。
    #[must_use]
    pub fn to_value(&self) -> Value {
        serde_json::json!({
            "node": self.node.to_canonical_string(),
            "track": self.track.clone(),
            "lanes": self.lanes.iter().map(AutomationLaneReport::to_value).collect::<Vec<_>>(),
        })
    }
}

/// `data.automation`：**音量自动化**在这次渲染里的全部读数。
///
/// 为什么需要它：`unsupported` 只说“少了什么”，说不清“自动化到底看没看、看了哪几条”。
/// 这份读数把每条泳道的**可求值性**、**支持性**与**是否真的应用**逐条列出 ——
/// 于是“空泳道不报 / master-only 与 bus-only 泳道必须可见”这两件事都有同一份证据。
#[derive(Clone, Debug, PartialEq)]
pub struct AutomationReport {
    /// 有泳道的音轨（按 `EntityId` 升序；没有泳道的轨不出现）。
    pub tracks: Vec<AutomationTrackReport>,
    /// 泳道总数（含空泳道与关掉的泳道）。
    pub lanes: usize,
    /// 有可求值点的泳道数。
    pub evaluable: usize,
    /// 真的被应用的泳道数。
    pub applied: usize,
    /// 有可求值点、但本渲染器不消费的泳道数（⇒ 调用方登记 `automationLanes`）。
    pub unsupported: usize,
}

impl AutomationReport {
    /// 扫描工程，生成读数。
    ///
    /// - `source_nodes` 用来判定非主总线的 `TrackPan` 是否有源侧落点（`#257`）；
    ///   有入边的**总线轨**仍因此拿不到 `TrackPan` 落点（`#296` B2）；
    /// - `rendered_edges` 是**保留子图里的入边身份**（`RenderPlan::edge_ids()`）——
    ///   `SendGain` 只有落在这张表里才算“会被累加”（任务单 `#256`）；
    /// - `rendered_nodes` 是**保留子图里的全部节点**（`RenderPlan::nodes()`）——
    ///   `TrackVolume` 在计划里的**任何**节点上都有落点（源轨在源侧、总线轨在
    ///   `#296` 的输出级、主总线在执行之后；只有不可达轨仍 `busTrack`）；
    ///   `DeviceParam` 的落点是该节点的插入链（通道条 / Freeverb / 卷积标量），因此
    ///   只有节点在计划里、且参数名有落点（`device_param_lane_is_applicable`）才算
    ///   “会被应用”（`#266`/`#267`/`#293`）。
    #[must_use]
    pub fn from_project(
        project: &YebanProjectV1,
        source_nodes: &[EntityId],
        rendered_edges: &[EntityId],
        rendered_nodes: &[EntityId],
    ) -> Self {
        let mut tracks = Vec::with_capacity(project.tracks.len());
        let mut lanes = 0usize;
        let mut evaluable = 0usize;
        let mut applied = 0usize;
        let mut unsupported = 0usize;
        for track in project.tracks.values() {
            let mut lane_reports = Vec::with_capacity(track.automation_lanes.len());
            for (target, lane) in &track.automation_lanes {
                lanes += 1;
                let is_evaluable = lane.read_enabled && !lane.points.is_empty();
                if is_evaluable {
                    evaluable += 1;
                }
                // 支持的目标与落点判定**只有一份**：
                // `yeban_render::automation::unsupported_lane_reason`（本层⛔ 不写第二份
                // match —— `yeban_edit_automation` 的读侧也调同一个函数）。
                let supported = yeban_render::automation::unsupported_lane_reason(
                    project,
                    target,
                    source_nodes,
                    rendered_edges,
                    rendered_nodes,
                )
                .is_none();
                let is_applied = is_evaluable && supported;
                if is_applied {
                    applied += 1;
                }
                if is_evaluable && !supported {
                    unsupported += 1;
                }
                let ordered = lane.points_in_tick_order();
                let first = ordered.first().copied();
                let last = ordered.last().copied();
                // ⭐ `#263`：`DeviceParam` 的两个下标都是序数 ⇒ 逐泳道把"此刻指到谁"
                // 报出来。解析的唯一实现在 `yeban_render::inserts`（本模块⛔ 不复制）；
                // 非设备目标三个字段一律 `null`，键仍然在（⛔ 不静默省略）。
                let (resolved_device_id, resolved_param_name, unresolved) = match target {
                    AutomationTarget::DeviceParam {
                        track_id,
                        slot_index,
                        param_index,
                    } => {
                        match resolve_device_param(project, *track_id, *slot_index, *param_index) {
                            Ok(resolved) => {
                                let reason = resolved.unresolved_reason();
                                (Some(resolved.device_id), Some(resolved.param_name), reason)
                            }
                            Err(reason) => (None, None, Some(reason.as_str())),
                        }
                    }
                    AutomationTarget::TrackVolume { .. }
                    | AutomationTarget::TrackPan { .. }
                    | AutomationTarget::SendGain { .. }
                    | AutomationTarget::Macro { .. } => (None, None, None),
                };
                lane_reports.push(AutomationLaneReport {
                    target: automation_target_name(*target),
                    points: lane.points.len(),
                    read_enabled: lane.read_enabled,
                    write_mode: automation_write_mode_name(lane.write_mode),
                    static_value: target.static_value(project).ok(),
                    supported,
                    evaluable: is_evaluable,
                    applied: is_applied,
                    first_tick: first.map(|point| point.tick),
                    last_tick: last.map(|point| point.tick),
                    first_value: first.map(|point| point.value),
                    last_value: last.map(|point| point.value),
                    resolved_device_id,
                    resolved_param_name,
                    unresolved,
                });
            }
            if !lane_reports.is_empty() {
                tracks.push(AutomationTrackReport {
                    node: track.id,
                    track: track.name.clone(),
                    lanes: lane_reports,
                });
            }
        }
        Self {
            tracks,
            lanes,
            evaluable,
            applied,
            unsupported,
        }
    }

    /// 进响应的 JSON 形状。
    #[must_use]
    pub fn to_value(&self) -> Value {
        serde_json::json!({
            "wired": true,
            "evaluator": "YebanProjectV1::automation_value_at",
            "targetsSupported": ["TrackVolume", "TrackPan", "SendGain", "DeviceParam"],
            "targetsUnsupported": ["Macro"],
            // ⭐ `#267`/`#293`/`#304`：`DeviceParam` 这一族接了**插入链能改的标量字段**
            // （55 个拼写 = 41 通道条（含压缩 14）+ 6 混响 + 8 卷积，含别名）；哪些名字
            // 有落点必须可读，且**只有一处来源**
            // （`yeban_render::automation::APPLIED_DEVICE_PARAM_NAMES`）——
            // ⛔ 不让 "DeviceParam 支持了" 这句话被读成 `is_supported_param` 全通。
            "appliedDeviceParamNames": APPLIED_DEVICE_PARAM_NAMES,
            "granularity": {
                "blockSize": L1_BLOCK_SIZE,
                "curve": "每块起点取一次曲线值（tick = floor(frame * PPQ * bpm / (sample_rate * 60))）",
                "smoothing": "TrackVolume / SendGain / TrackPan：块内逐样本一阶低通（时间常数 5 ms）；DeviceParam：只在块边界 set_params（⛔ 不逐样本插值，⛔ 没有平滑器）",
            },
            "domainTransfer": {
                "ratio": "db_to_linear(自动化 dB) / db_to_linear(静态值 dB)（TrackVolume 取 track.volume_db；SendGain 取 edge.gain_db.unwrap_or(0.0)）；TrackPan 不换域：平滑的是位置（-1..=1），乘的是等功率曲线的绝对左右增益；DeviceParam 不换域：原生绝对值（dB）直接交给器件参数",
                "adr": "R55",
                "denominator": "静态值（⛔ 不是 mute/solo 之后的增益；TrackVolume 静音时为 0.0 会除零）",
            },
            "smoother": "yeban_dsp::smoothing::ParamSmoother",
            "timeConstantSeconds": 0.005,
            "lanes": self.lanes,
            "evaluableLanes": self.evaluable,
            "appliedLanes": self.applied,
            "unsupportedLanes": self.unsupported,
            "tracks": self.tracks.iter().map(AutomationTrackReport::to_value).collect::<Vec<_>>(),
        })
    }
}

/// 目标变体的稳定名字（读数用；与 `yeban_edit_automation` 的词表同字）。
#[must_use]
fn automation_target_name(target: AutomationTarget) -> &'static str {
    match target {
        AutomationTarget::TrackVolume { .. } => "TrackVolume",
        AutomationTarget::TrackPan { .. } => "TrackPan",
        AutomationTarget::SendGain { .. } => "SendGain",
        AutomationTarget::DeviceParam { .. } => "DeviceParam",
        AutomationTarget::Macro { .. } => "Macro",
    }
}

/// 写模式的稳定名字。
#[must_use]
fn automation_write_mode_name(mode: AutomationWriteMode) -> &'static str {
    match mode {
        AutomationWriteMode::Off => "Off",
        AutomationWriteMode::Write => "Write",
        AutomationWriteMode::Touch => "Touch",
        AutomationWriteMode::Latch => "Latch",
    }
}

/// ⭐ `ADR-0020`：一条轨对母带的**贡献读数**（"拿掉它，母带变了多少"）。
///
/// ## 量的是什么（对象 ＋ 单位）
///
/// `delta_rms_dbfs` / `delta_peak_dbfs` = 两次渲染**母带样本之差**的 RMS / 峰值，
/// 单位 dBFS（`20·log10(x)`，满量程 = 0 dBFS）。两次渲染除"该轨 `mute`"外全部相同：
/// **同一**采样率 / 帧数 / 格式，且都**不归一化**（归一化会把差值按不同增益缩放，
/// 于是"变了多少"变成"归一化算法变了多少"）。
///
/// ## 它诚实地说不了什么
///
/// - 差值来自**最终** 24-bit PCM 负载 ⇒ 逐样本差恒为 0 时报 `null`（⛔ 不报 `-inf`）。
/// - 它证明"这条轨在母带里留下多少能量"，⛔ 不证明"这条轨在音乐上该不该被听见"。
/// - 工程里有别的轨处于 solo 时，静音一条**非 solo** 轨不会改变母带 ⇒ 该轨报 `null`。
pub struct TrackContribution {
    /// 被静音的那条轨。
    pub track_id: EntityId,
    /// 轨名（响应里便于人读；⛔ 不参与寻址）。
    pub name: String,
    /// 渲染**之前**这条轨已经是 `mute = true`（贡献无法判定 ⇒ 两个差值为 `null`）。
    pub already_muted: bool,
    /// 母带样本差值的 RMS（dBFS）；`null` = 一个 24-bit 样本都没变。
    pub delta_rms_dbfs: Option<f64>,
    /// 母带样本差值的峰值（dBFS）；`null` = 一个 24-bit 样本都没变。
    pub delta_peak_dbfs: Option<f64>,
}

impl TrackContribution {
    /// 响应里的形状。
    #[must_use]
    pub fn to_value(&self) -> Value {
        serde_json::json!({
            "trackId": self.track_id.to_canonical_string(),
            "name": self.name,
            "alreadyMuted": self.already_muted,
            "deltaRmsDbfs": self.delta_rms_dbfs,
            "deltaPeakDbfs": self.delta_peak_dbfs,
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
    /// ⭐ `ADR-0020`：逐轨贡献读数；`None` = 调用方没要（`contributions` 缺省）。
    ///
    /// 由 `plan_render_master` 在 [`build`] 之后调 [`track_contributions`] 填 ——
    /// ⛔ 不在 `build` 里做，因为 `build` 是"一次渲染"，而这里要 `N + 1` 次。
    pub contributions: Option<Vec<TrackContribution>>,
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
    /// ⭐ #175（#174 坑⑧）：会被 24-bit 量化器**钳住**的样本数。
    ///
    /// 对象是抖动/量化**之前**的浮点母带；单位是**样本个数**（两侧声道合计）。
    /// 判据是 `|sample| > 1.0`（严格大于：`±1.0` 恰好是 PCM 满量程端点，不需要钳位）。
    /// 计算方法见 [`math::clipped_sample_count`]。
    pub clipped_samples: u64,
    /// ⭐ #175：**真峰值**（dBFS；`0.0` = 满量程，正值 = 超过满量程 ⇒ 可能削波）。
    ///
    /// 由 `yeban-render::mastering::measure_master` 按**母带口径 16×** 多相过采样测得
    /// （唯一实现：`yeban-dsp::meter::TruePeakDetector`，本 crate 不复制 DSP）。
    /// 全零母带 / 测不出 ⇒ `None`（⛔ 不写一个假的 `0.0`）。
    ///
    /// ⚠ 它与 `peak.after` **不是同一个数**：后者是样本峰值（`max |sample|`），
    /// 真峰值还看样本之间的过冲，因此 `truePeakDbfs >= 20·log10(peak_after)`。
    pub true_peak_dbfs: Option<f32>,
    /// ⭐ #175：**余量**（dB）= `-truePeakDbfs`。
    ///
    /// 正值 = 真峰值在 0 dBFS 之下（还有余量）；负值 = 已过载（该削波了）。
    /// 真峰值测不出 ⇒ `None`。
    pub headroom_db: Option<f32>,
    /// ⭐ `#225`：**结果质量**提示（`data.warnings[]`，非阻断）。
    ///
    /// 两条来源（两条都不出现 ⇒ **空数组**，⛔ 不是恒报）：
    /// - `code:"exportClipping"`（`clippedSamples > 0`，含 `clippedSamples` /
    ///   `truePeakDbfs` / `suggestedTrimDb` / `loudestSource`）—— 见 [`clipping_warning`]；
    /// - `code:"silentSources"`（可闻、排了内容、却 `peak == 0.0` 的源逐条点名）——
    ///   见 [`silent_sources_warning`]。
    ///
    /// ⚠ 它们**不是** `unsupported`：那份清单的口径是"我们会静默少做什么"，
    /// 削波是"算对了、但结果过载"，静音源是"排了内容却没进母带"。判据与消费者都不同。
    pub warnings: Vec<Value>,
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
    /// `bext` 的 `OriginationDate` —— 取自**工程创建时刻**
    /// （`metadata.created_at_unix_ms`），⛔ 不是导出时刻（`#214`）。
    pub origination_date: String,
    /// `bext` 的 `OriginationTime` —— 同 [`Self::origination_date`]。
    pub origination_time: String,
    /// `bext` 的 `CodingHistory`（写进文件的那一份）。
    pub coding_history: String,
    /// 工程内容摘要（渲染输入的指纹，用于溯源）。
    pub project_digest: String,
    /// 渲染时刻（注入时钟的 Unix 毫秒）。
    pub rendered_at_unix_ms: u64,
    /// ⭐ 音量自动化（`TrackVolume`）的逐泳道读数。
    ///
    /// 这份读数与 `unsupported` 是**互补**的：`automationLanes` 只说“有没消费的泳道”，
    /// 这里把每条泳道的 `points` / `readEnabled` / `supported` / `applied` 逐条列出。
    pub automation: AutomationReport,
    /// ⭐ `#315`：**真的被当成普通音频边求和进母带**的侧链边条数（读数，⛔ 不改音频）。
    ///
    /// ⭐ `#337`：这个读数现在**恒为 `0`** —— `yeban-render` 把 `Sidechain` 边从音频
    /// 子图里拿掉了（可达性 / PDC / 分层 / `incoming` 全都不收它）。保留该字段是为了
    /// 兼容既有消费者，并让"修前求和了几条"这个事实仍可被对照读出。
    ///
    /// ⚠ 它**单独读会误导**：`0` 既可能是"工程里没有侧链"，也可能是"有侧链、但一条都
    /// 没有被求值"。所以必须与 [`Self::sidechain_edges_declared_without_key_input`]
    /// （和 [`Self::sidechain_edges_in_routing_graph`]）一起读。
    ///
    /// 口径逐字：⭐ **这条边走了普通音频求和路径**、⛔ **不是"+6 dB 了几路"**。
    /// 反例正是"从路由图数"：D case（悬挂侧链边，两端都到不了 master）图上 1 条
    /// ⇒ 报 1 是假话 ⇒ 所以必须走 `RenderPlan::edge_ids()`：`NodeState.incoming` 里的
    /// **每一条**都在 `render_node` 里被无条件累加，而 `edge_ids()` 列出的正是这一张表。
    pub sidechain_edges_summed_as_ordinary_audio: usize,
    /// ⭐ `#315`：路由图里**声明**了多少条侧链边（含被剪枝 / 悬挂、到不了 master 的）。
    ///
    /// 与另外两个读数对照读：声明了 N 条里，真的求和了 M 条（修后恒 0）、
    /// 没有键输入路径的有 K 条。差额是"连图上的音频子图都没进"的悬挂边。
    pub sidechain_edges_in_routing_graph: usize,
    /// ⭐ `#337`：**声明了、但没有键输入路径**的侧链边条数。
    ///
    /// 这是"侧链到底做没做事"的读数：本切片没有任何检波器/键输入消费面
    /// （`yeban-dsp` 的压缩器只从**正在压缩的那一路**取检波输入，`process_gain(left,
    /// right)` 没有第二个输入；`yeban-render` 也没有把任何信号送进第二个输入），
    /// 因此**每一条**声明的侧链边都"没有键输入路径"。
    ///
    /// ⇒ 与 [`Self::sidechain_edges_summed_as_ordinary_audio`] 一起读：
    /// `(0, 0)` = 工程里没有侧链；`(0, N)` = N 条侧链声明了、但一条都没有被求值
    /// （"你的侧链现在什么都不做"）—— 这正是"问题看起来消失了"与"问题仍然存在"
    /// 的分界。⛔ 不要把这个数读成"求和了"。
    pub sidechain_edges_declared_without_key_input: usize,
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
            .field(
                "contributions",
                &self
                    .contributions
                    .as_ref()
                    .map_or("None".to_owned(), |rows| format!("{} rows", rows.len())),
            )
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
        // ⭐ `ADR-0020`：逐轨贡献。**只有**调用方给了 `contributions: true` 才出现；
        // 缺省时整块不出现 ⇒ 响应与加这个开关之前逐字节相同（`request` 回显除外）。
        if let Some(rows) = &self.contributions {
            map.insert(
                "contributions".to_owned(),
                Value::Array(rows.iter().map(TrackContribution::to_value).collect()),
            );
            map.insert(
                "contributionsBasis".to_owned(),
                serde_json::json!({
                    "method": "对每条非 master 轨各渲染一次\"只静音该轨\", 与同一份基线逐样本比",
                    "metric": "两次母带样本逐样本差的 RMS / 峰值, 单位 dBFS (20·log10, 满量程 = 0 dBFS)",
                    "domain": "解码后的 24-bit PCM 负载",
                    "normalize": false,
                    "frames": self.frames,
                    "renders": 1 + rows.iter().filter(|row| !row.already_muted).count(),
                    "nullMeans": "逐样本差恒为 0（这条轨没有留下任何 24-bit 级变化）, \
                                  或渲染前这条轨已经是 mute",
                    "doesNotProve": "这条轨在音乐上该不该被听见",
                }),
            );
        }
        // ⭐ `#251`（S0）：MIDI 音源**显式**说明 —— 不再只靠 `sources[].kind` 的间接标签。
        // ⭐ `#268`（`ADR-0011`，第 324 条）：正弦振荡器**已退休** ⇒ 这段文案必须
        //    逐字改成实话。`kind` 与 `engineTimbre` 也跟着变：旧值（`builtin-sine` /
        //    `false`）今天就是假话，留着会让 agent 以为听到的不是引擎音色。
        map.insert(
            "midiSource".to_owned(),
            serde_json::json!({
                "kind": "engine-instrument",
                "label": "yeban-engine SynthEngine（引擎音色）",
                "engineTimbre": true,
                "adr": "ADR-0011",
                "engine": "yeban_engine::synth::SynthEngine（HOLLOW 加法波表 + ADSR + 双振荡器）",
                "note": "MCP 的 MIDI 音符音源是 yeban-engine 的 SynthEngine，与界面/CLI 的\
                         `--export-wav` **同一个音色、同一条快照投影**（ADR-0011）；\
                         本工具⛔ 不再使用内置正弦振荡器。绑定了 SFZ 采样器的轨由采样器发声\
                         （与 CLI 的 `--sfz` 旁路同形），见 sources[].sampler。",
                "appliesTo": "sources[].kind == \"engine-instrument\"",
            }),
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
                "unit": "full-scale (线性幅度; 1.0 = 0 dBFS, 不是 dB)",
                // ⭐ #175（#174 坑⑧）：这两个数的语义曾在响应里**没有**任何一句话说清。
                "semantics": "before = 归一化**前**的样本峰值 (max |sample|), after = 归一化**后**的样本峰值; \
                              两者都作用在 TPDF 抖动 / 24-bit 量化**之前**的浮点母带上。\
                              ⛔ 不是文件峰值 (写盘的那份经过抖动与量化; 见 top-level `clippedSamples`), \
                              ⛔ 也不是真峰值 (样本间的过冲见 top-level `truePeakDbfs`)。",
                "domain": "TPDF 抖动 / 24-bit 量化之前的浮点母带",
                "truePeakDbfs": json_f32(self.true_peak_dbfs),
                "truePeakOversampling": 16,
            }),
        );
        // ⭐ #175（#174 坑⑧）：削波必须是**可见**的读数，⛔ 不许只给一个峰值让人自己推。
        map.insert(
            "clippedSamples".to_owned(),
            Value::from(self.clipped_samples),
        );
        map.insert("truePeakDbfs".to_owned(), json_f32(self.true_peak_dbfs));
        map.insert("headroomDb".to_owned(), json_f32(self.headroom_db));
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
        // ⭐ 音量自动化的逐泳道读数。空泳道会出现在这里（`evaluable:false`），但
        // **不**进 `unsupported`；master-only / bus-only 的泳道也在这份读数里可见。
        map.insert("automation".to_owned(), self.automation.to_value());
        // ⭐ #225：结果质量预警（非阻断、可寻址）。`clippedSamples == 0` 时是空数组。
        // 它与 `unsupported` **不是一回事**：那条口径是"我们会静默少做什么"。
        map.insert("warnings".to_owned(), Value::Array(self.warnings.clone()));
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
        // ⭐ `#315`：侧链边的**两个读数** —— "声明了 N 条、求和了 M 条"。
        //
        // ⚠ 它们**不是** `unsupported` 的替代品，也**不是**它的重复：
        //   - `unsupported` 里的 `sidechainRouting` 的判据是"**图里有没有这种数据**"
        //     （D case 的悬挂边照样报）—— 它说不出"那条边已经 +6 dB 了"；
        //   - 这两个数说的是"**实际发生了什么事**"：`M` 条侧链边真的走进了某个节点的
        //     入边求和（⇒ 与同形 `TrackToBus` 边逐位等价）。
        //   ⇒ 修前响应**只有**前者的"不支持"，于是"报不支持却悄悄 +6.0206 dB"是一个
        //     没有任何一个键能揭穿的组合。两条一起读才是实话。
        map.insert(
            "sidechainEdgesSummedAsOrdinaryAudio".to_owned(),
            Value::from(self.sidechain_edges_summed_as_ordinary_audio),
        );
        map.insert(
            "sidechainEdgesInRoutingGraph".to_owned(),
            Value::from(self.sidechain_edges_in_routing_graph),
        );
        // ⭐ `#337`：与"求和了几条"互补的第三个读数。修后求和恒 0，单看它会以为
        // "问题消失了"；这个数说"声明了 N 条、**一条都没有键输入路径**"。
        map.insert(
            "sidechainEdgesDeclaredWithoutKeyInput".to_owned(),
            Value::from(self.sidechain_edges_declared_without_key_input),
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
    //    ⭐ 同一个探针也服务 `yeban_edit_automation` 的读侧 ⇒ 写侧读数与导出读数
    //    看到**同一张**保留子图（否则"写时说会应用、导出说不支持"会各判一套）。
    let probe = probe_subgraph(project, sample_rate)
        .map_err(|error| render_error("渲染计划探针编译", &error))?;
    let nodes = probe.nodes;
    let source_nodes = probe.source_nodes;
    let rendered_edges = probe.rendered_edges;
    // ⭐ `#315` / `#337`：三个侧链读数（"声明了 N 条、求和了 M 条、没有键输入路径 K 条"）。
    // **纯读**，⛔ 不碰一个样本。
    //
    // ⚠ **求和条数必须从 `edge_ids()` 数，⛔ 不能从路由图数** —— 反例正是"从路由图数"：
    //    D case（悬挂侧链边，两端都到不了 master）图上 1 条 ⇒ 报 1 是假话
    //    （音频逐字节等于基线）⇒ 所以必须走 `edge_ids()`（剪枝过的边不在里面）。
    //    ⭐ 一个**看起来更简单**的算法会在这一条上**重复它正在修的错**：
    //    "读数该从哪取"决定了"读到的数是不是真的"。
    //
    // `rendered_edges` 就是 `RenderPlan::edge_ids()`（见 `probe_subgraph`），
    // 而 `render_node` 对每一个 `incoming` 条目无条件累加 ⇒ 它就是"真的被加进某个
    // 节点输入缓冲的边"的全集；再按 `kind` 过滤即得侧链边条数。
    //
    // ⭐ `#337` 起 `yeban-render` 不再把 `Sidechain` 放进 `edge_ids()` ⇒
    // `sidechain_edges_summed_as_ordinary_audio` **恒为 0**。它单独读会造成
    // "问题消失了"的误判，因此必须同时报"N 条声明了、但没有键输入路径"。
    let sidechain_edges_in_routing_graph = project
        .routing_graph
        .edges
        .values()
        .filter(|edge| edge.kind == RoutingKind::Sidechain)
        .count();
    // ⭐ `#337`：本渲染器**没有键输入消费面**（`yeban-dsp` 的压缩器检波器只读自己
    // 正在压缩的那一路；`yeban-render` 没有把任何第二路信号送进检波器）。⇒ 每一条
    // **声明**的侧链边都没有键输入路径。这个等式是当前事实，⛔ 不是省事的复制：
    // 将来真有键输入时，这里必须改成"声明条数 − 真的接上键输入的条数"，并同步改
    // `yeban-render` 的 `edge_ids()` 语义与本字段的文档。
    let sidechain_edges_declared_without_key_input = sidechain_edges_in_routing_graph;
    let sidechain_edges_summed_as_ordinary_audio = rendered_edges
        .iter()
        .filter(|edge_id| {
            project
                .routing_graph
                .edges
                .get(edge_id)
                .is_some_and(|edge| edge.kind == RoutingKind::Sidechain)
        })
        .count();
    // ⭐ 任务单 `#303` / `#309`：**会执行压缩**的轨（pre-pan tap 的唯一判据）。
    // 与 CLI 的 `render_core` 共用**同一份**
    // `yeban_render::automation::compressing_nodes_with_lanes`：
    // 静态投影（`statically_compressing_nodes`）∪ **非恒等 `compressor_enabled` 泳道**。
    // ⭐ `#308` 的缺陷：只看静态位 ⇒ 静态关的压缩器被泳道 `0 → 1` 打开时，检波器
    // 仍落在声相之后（`GR` 随 pan 变，Δ = `+0.515761 dB`）。
    // ⛔ 本层不复制"哪些参数算压缩"的判定。必须在**构造音源之前**算出来：
    // 这些轨的声相要搬到输出级（插入链之后），源侧取 `(1, 1)`。
    // 设备参数扫描器必须**先**算（并集要看非恒等泳道）；下面第 8 步复用它。
    let device_automations = device_param_automations(project, sample_rate);
    let forced = forced_insert_instances(&device_automations);
    let compressing = compressing_nodes_with_lanes(&project.tracks, &device_automations);
    // 4b. 组装音源 + 采集"明确没渲染"的部分 + 得到时间轴末端。
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
    // 4. ⭐ 音量自动化读数：扫描**全部**音轨（含主总线与总线轨），而不是只扫源节点
    //    —— 旧实现只在 `build_source` 里看 `automation_lanes`，于是 master / bus 上
    //    唯一的泳道连报都不报；空泳道又会被一律报成 `automationLanes`。现在：
    //    - 空泳道 / `readEnabled:false` / 所有点都等于静态推子 ⇒ 可求值但无效果，
    //      读数里可见，**不**进 `unsupported`；
    //    - 有可求值点、但这次渲染**没有落点**（`DeviceParam` 名字不是插入链落点 /
    //      `TrackVolume`、`TrackPan` 落在不可达轨 / `SendGain` 落在被剪枝的边 /
    //      `Macro` 没有可落点）⇒ 进 `unsupported`（`automationLanes`）；
    //    - 主总线与**可达总线轨**上的 `TrackVolume` / `TrackPan`（`#296` / `#298`）、
    //      保留子图入边上的 `SendGain`（`#256`）、计划节点上的 `DeviceParam`
    //      （通道条/混响/卷积字段，`#266`/`#267`/`#293`/`#304`）、以及有落点的
    //      `Macro` 泳道（`#297`）真的被应用（见 `TrackSource`、第 9 步、
    //      `RenderPlan::attach_send_automation` 与
    //      `RenderPlan::attach_device_param_automation`）。
    let automation =
        AutomationReport::from_project(project, &source_nodes, &rendered_edges, &nodes);
    if automation.unsupported > 0 {
        note_unsupported(&mut unsupported, &mut counts, "automationLanes");
    }
    // 资产上下文: 每个资产**解码/重采样一次**, 供所有音轨共享（`Arc<[f32]>` 只读）。
    let mut asset_cache: BTreeMap<AssetHash, Option<Arc<PreparedAsset>>> = BTreeMap::new();
    let mut asset_reports: BTreeMap<AssetHash, AudioAssetReport> = BTreeMap::new();
    // ⭐ `#268`（`ADR-0011`）：**一次**快照投影，供全部 CPU 轨共用
    // （与 CLI 的 `render_core` 同一条装配；`Arc::clone` 只是指针复制）。
    // 失败 ⇒ 在**落盘之前**如实失败（不写一份"没有声部"的母带）。
    //
    // ⭐ `#274`：这里改成**武装入口** —— 快照投影之后立刻把 `sfz_pending()` 里每条轨
    // 的样本摊平进快照（`SfzPlan::prepare_with_samples` ＋ `with_sfz`）。解码仍在本层
    // 当 provider（⛔ `yeban-decode` 不下移进 `yeban-render`），引擎只接已解码 PCM。
    //
    // ⭐ `#276`：样本池（[`sfz::SampleStore`]）的生命周期＝**这一次渲染** ——
    // 同一份样本被多条轨引用时只解码一次；每条轨的装载读数仍按自己的预算记账不变。
    //
    // ⚠ 失败的**结构化**报文由 provider 捕获（`sampler_failure`）并在武装之后**原样**
    // 抛回 ⇒ `samplerUnreadable` 的 `tried` 列表与 `#271` 的形状一字不变。
    let mut sampler_loads: BTreeMap<EntityId, ArmedSamplerTrack> = BTreeMap::new();
    let mut sampler_failure: Option<Fault> = None;
    let mut sampler_store = sfz::SampleStore::new();
    let timbre = EngineTimbre::from_project_armed(project, sample_rate as f32, |track, raw| {
        match prepare_sampler_track(track, raw, project, project_path, &mut sampler_store) {
            Ok((armed, samples)) => {
                // ⭐ `#279`：解码表**直接**交给引擎（`(sample= 原样字符串, (PCM, 原始采样率))`），
                // ⛔ 不再经 `ArmedSamplerTrack` 中转一次深拷贝（旧 `armed.samples.clone()`）。
                let resolved = armed.load.path.clone();
                sampler_loads.insert(track, armed);
                ArmedSfzInput::new(resolved, samples)
            }
            Err(fault) => {
                if sampler_failure.is_none() {
                    sampler_failure = Some(fault);
                }
                ArmedSfzInput::empty(raw)
            }
        }
    });
    if let Some(fault) = sampler_failure {
        return Err(fault);
    }
    let timbre = timbre.map_err(|error| {
        render_failed(
            format!("引擎快照投影/武装失败: {error}"),
            serde_json::json!({
                "reason": "engineSource",
                "cause": error.message(),
                "hint": "工程必须有可用的主总线节点；路由图必须自洽",
            }),
        )
    })?;
    let mut registry: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
    let mut pending: Vec<(EntityId, TrackSource)> = Vec::new();
    let mut reports: Vec<SourceReport> = Vec::new();
    // ⭐ `#274`：逐轨的**声部窃取**句柄（渲染之后才有读数，见 `9a` 的同一条时机）。
    let mut sampler_steals: BTreeMap<EntityId, Arc<AtomicU64>> = BTreeMap::new();
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
            &timbre,
            track,
            sample_rate,
            any_solo,
            assets,
            &mut asset_cache,
            &mut asset_reports,
            &sampler_loads,
            &mut sampler_steals,
            &mut unsupported,
            &mut counts,
            compressing.contains(&node),
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
    // 7. 帧数: ⭐ `#252` 显式 `frames`／`durationSeconds` 优先；缺省仍由时间轴末端与
    //    工程量纲唯一决定（＝修改前的同一条公式，逐字节不变）。
    let frames = request
        .frames
        .unwrap_or_else(|| math::frames_at_tick(end_tick, PPQ, &project.tempo(), sample_rate));
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
    //    上限对**两条**来源（显式帧数 / 工程末端推导）是同一处闸门、同一条报文。
    frame_ceiling(frames)?;
    // 7b. 音频片段的帧区间在**知道总帧数之后**才能夹住（避免区间越过母带末端）。
    //     这一步只改内存里的 `TrackSource`，不重新解码。
    //
    // ⭐ #225：每个源外面套一个只读的 [`PeakTap`]（逐源峰值），句柄留在
    // `peak_taps` 里，渲染结束后把读数填回 `reports[]`。探针**只读** `out`。
    let mut peak_taps: BTreeMap<EntityId, Arc<AtomicU32>> = BTreeMap::new();
    for (node, mut source) in pending {
        source.clamp_to(frames);
        let peak_bits = Arc::new(AtomicU32::new(0));
        peak_taps.insert(node, Arc::clone(&peak_bits));
        let boxed: Box<dyn AudioSource> = Box::new(PeakTap {
            inner: source,
            peak_bits,
        });
        registry.insert(node, boxed);
    }
    // 8. 真编译 + 真执行。
    let options = RenderOptions::l1(frames, MASTER_CHANNELS, sample_rate, project.rng_seed);
    // 每轨插入链：从 `TrackV3.devices` 投影（通道条增益/EQ/低通/**压缩** + Freeverb
    // + 卷积混响）。⭐ 压缩自 `#303` 起**执行**（见 `yeban_render::inserts` 模块头 §1.1；
    // 会压缩的源轨由 `compressing` 把声相搬到输出级 ⇒ 检波器在声相之前）。
    // ⚠ **主总线也挂**（`#173` 起 `project_inserts` 不过滤 master）——旧注释写
    // "主总线上的设备链不挂"已陈（`#244` 实测点出，此处更正）。实时引擎的逐轨循环
    // 跳过 master 是**另一件事**，不要拿它推这条行为的结论。
    //
    // ⭐ 任务单 `#266`/`#293`：有**非恒等** `DeviceParam` 泳道的轨，对应器件被**强制
    // 实例化**（通道条：`effective_strip` 在全恒等参数下也返回 `Some`；Freeverb/卷积：
    // 静态 `wet = 0` 时 `is_active()` 为假，但块边界仍需有器件可改）。扫描器与 app 的
    // `--export-wav` 共用同一份（`yeban-render::automation`），已在构造音源之前算好
    // （`#309` 起 pre-pan 判据也要读它）。
    let inserts = project_inserts_with_forced_instances(&project.tracks, sample_rate, &forced);
    let mut plan = RenderPlan::compile_with_latencies_and_inserts(
        &project.routing_graph,
        project.master_bus_track_id,
        options,
        &latencies,
        &inserts,
    )
    .map_err(|error| render_error("渲染计划编译", &error))?;
    // ⭐ 任务单 `#256`：把每条 `SendGain` 泳道接到计划里的**那条入边**上。
    // 求值/平滑/恒等判定全部在 `yeban-render::automation`（MCP ⛔ 不写第二份平滑，
    // 它不能依赖 `yeban-dsp`）；这里只负责“哪条边”。没有泳道的工程一个槽都不改。
    for automation in send_gain_automations(project, sample_rate).into_values() {
        plan.attach_send_automation(automation);
    }
    // ⭐ 任务单 `#266`/`#293`：把每条 `DeviceParam` 泳道接到**该节点的插入链**上。
    // 求值（模型唯一入口）、恒等判定与块级落点都在 `yeban-render`；这里只负责“哪些轨”。
    // 恒等泳道也挂（`value_at_block` 返回 `None`，一个参数都不改）。
    for automation in device_automations.into_values().flatten() {
        plan.attach_device_param_automation(automation);
    }
    // ⭐ 任务单 `#296` ＋ B2：总线的**输出级**（`render_node` 的插入链之后）——
    // 推子 / `mute` / `solo` / 静态声相 / `TrackPan` 自动化。
    //
    // 挂"计划里、非源节点、非主总线"的节点 —— 源节点的音量与声相仍在源侧（引擎的
    // `track_gain` / 片段增益 / `TrackSource` 的 R55 与 `pan_automation`），主总线的
    // 推子与声相自动化仍在第 9 步（执行之后）；本层⛔ 不搬家 ⇒ 普通 MIDI/音频轨的
    // 行为与 `#296` 之前**逐位相同**。
    //
    // ⭐ `#303` 的**例外**：`compressing` 里的**源节点**也挂一个**只含声相**的输出级
    // （pre-pan tap，见 `inserts` 模块头 §1.1）。它们的推子仍在源侧（`is_source` 分支
    // ⛔ 不乘 bus_gain），源侧声相已在 `build_source` 关掉 ⇒ 只乘一次、且在插入之后。
    //
    // ⭐ B2b/B2c：总线的 `mute`/`solo`（含 `solo_safe`）是**静态字段**，折进输出级的
    // 静态 `gain`（静音 ⇒ `0.0`，整条总线 ×0）。`any_solo` 与源轨同一条扫描（含 master）。
    // ⭐ B2a：静态 `pan` 与 `TrackPan` 泳道在**同一处**落点；泳道非恒等时静态声相关掉。
    // 没有落点（不在保留子图）时 `attach_output_stage` 返回 `false`，本层不谎称应用。
    for &node in &nodes {
        if node == project.master_bus_track_id {
            continue;
        }
        let is_source = source_nodes.contains(&node);
        // 源节点只在"会执行压缩"时进这里（`#303` 的 pre-pan tap）；否则一律不挂。
        if is_source && !compressing.contains(&node) {
            continue;
        }
        let Some(track) = project.tracks.get(&node) else {
            continue;
        };
        let mut stage = OutputStage::new();
        if !is_source {
            let bus_gain = if track_is_audible(track, any_solo) {
                db_to_linear(track.volume_db)
            } else {
                0.0
            };
            stage = stage.with_gain(bus_gain);
            if track
                .automation_lanes
                .keys()
                .any(|target| matches!(target, AutomationTarget::TrackVolume { .. }))
                || macro_cascade_reaches(project, &AutomationTarget::TrackVolume { track_id: node })
            {
                // 恒等泳道也挂：`apply_gain_block` 的恒等快路径一个样本都不多碰。
                stage = stage.with_gain_automation(TrackGainAutomation::new(
                    project,
                    node,
                    sample_rate,
                ));
            }
        }
        let pan_automation = TrackPanAutomation::new(project, node, sample_rate);
        let static_pan = if pan_automation.is_identity() {
            track.pan
        } else {
            0.0
        };
        stage = stage.with_pan(static_pan, pan_gains);
        if !pan_automation.is_identity() {
            stage = stage.with_pan_automation(pan_automation, pan_gains);
        }
        plan.attach_output_stage(node, stage);
    }
    // ⭐ 任务单 `#296`：主总线的**声相**（裁决 A / B3 / B4）。
    //
    // 静态声相走计划的输出级，曲线是**既有**的 [`pan_gains`] 且 `pan == 0` 由
    // `centered_pan_gains` 统一成 `(1, 1)`（⛔ 0.707）。`TrackPan` 自动化非恒等时
    // **关掉静态声相**，改由执行之后乘绝对左右增益 —— 与 `#257` 的源侧纪律同一条
    // （否则同一位置会乘两次）。恒等路径：`master.pan == 0` ⇒ 输出级是恒等，一个
    // 样本都不碰。
    let mut master_pan_automation: Option<TrackPanAutomation> = None;
    if let Some(master) = project.tracks.get(&project.master_bus_track_id) {
        let automation = TrackPanAutomation::new(project, master.id, sample_rate);
        let static_pan = if automation.is_identity() {
            master.pan
        } else {
            0.0
        };
        plan.attach_output_stage(
            master.id,
            OutputStage::new().with_pan(static_pan, pan_gains),
        );
        if !automation.is_identity() {
            master_pan_automation = Some(automation);
        }
    }
    let mut output = plan
        .execute(registry)
        .map_err(|error| render_error("渲染执行", &error))?;
    // 9. Master 轨增益 (在源侧无法表达: 它是总线节点)。
    //
    //    ⭐ 这是音量自动化的**第二个站点**：主总线没有源，`TrackVolume` 泳道不能在
    //    `TrackSource` 里乘。这里在整段母带上按 128 帧块求值 + 逐样本平滑，乘
    //    `静态主推子 × R55 比率`（`mute` 时静态主推子是 0.0）。
    //    恒等快路径仍走**原来那一句** `scale_in_place`，因此没有自动化的工程逐字节不变。
    //    ⭐ `#296`：主总线的**声相**不再登记为不支持 —— 静态 `pan` 在计划的输出级里
    //    乘，`TrackPan` 自动化在下面（推子之后）按块乘绝对左右增益（裁决 B4：
    //    推子 → 声相）。
    if let Some(master) = project.tracks.get(&project.master_bus_track_id) {
        let master_gain = if master.mute {
            0.0
        } else {
            db_to_linear(master.volume_db)
        };
        let mut automation = TrackGainAutomation::new(project, master.id, sample_rate);
        if automation.is_identity() {
            if master_gain != 1.0 {
                math::scale_in_place(&mut output.samples, master_gain);
            }
        } else {
            let frames = output.samples.len() / MASTER_CHANNELS;
            automation.apply_gain_block(
                &mut output.samples,
                0,
                frames,
                MASTER_CHANNELS,
                master_gain,
            );
        }
        // ⭐ `#296`：非恒等的 `TrackPan` 泳道（静态声相已在计划里被关掉 ⇒ 这里乘的
        // 是**绝对**左右增益，不是比率）。恒等 ⇒ `None`，一个样本都不碰。
        if let Some(automation) = master_pan_automation.as_mut() {
            let frames = output.samples.len() / MASTER_CHANNELS;
            automation.apply_pan_block(&mut output.samples, 0, frames, MASTER_CHANNELS, pan_gains);
        }
    }
    // 9-master-digest. 主总线增益/自动化是**执行之后**才乘的，而 `RenderOutput::digest`
    // 在 `execute` 里就按当时（未乘主推子）的样本算好了。`masterDigest` 的文档是
    // “母带样本的位级 SHA-256” ⇒ 必须对**最终**样本重算，否则主推子/主自动化改变
    // 了音频而摘要不变，读数与产物分叉。
    output.digest = RenderOutput::digest_of(&output.samples);
    // 9a. ⭐ #225：把逐源峰值从探针里取回来（**只读**：渲染已经结束，一个样本都不再改）。
    //     它们进 `sources[].peak` / `.peakDbfs`，也是削波预警点名"最响的源"的依据。
    for report in &mut reports {
        if let Some(bits) = peak_taps.get(&report.node) {
            report.peak = f32::from_bits(bits.load(Ordering::Relaxed));
        }
        // ⭐ `#274`：采样器轨的**声部窃取**读数（与峰值同一个时机：渲染已结束，
        //    只读原子；第 324 条 —— MCP 现在会窃取，这件事必须在响应里能看见）。
        if let Some(counter) = sampler_steals.get(&report.node)
            && let Some(sampler) = &mut report.sampler
        {
            sampler.voice_steals = counter.load(Ordering::Relaxed);
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
    // 10a. ⭐ #175（#174 坑⑧）：削波与真峰值读数（**只读**：不改一个样本、不进 `bytes`/摘要）。
    //      - `clipped_samples` 数的是"会被 24-bit 量化器钳住"的样本（`|s| > 1.0`）；
    //      - 真峰值走 `yeban-render::mastering`（母带口径 16× 过采样，唯一实现在
    //        `yeban-dsp::meter`）。⚠ 它内部会把整段信号多扫几遍（离线路径可接受）；
    //        本层不复制 DSP，也不在 MCP 里另写一份过采样器。
    let clipped_samples = math::clipped_sample_count(&output.samples);
    let (true_peak_dbfs, headroom_db) = {
        let frames = output.samples.len() / MASTER_CHANNELS;
        let mut left = Vec::with_capacity(frames);
        let mut right = Vec::with_capacity(frames);
        for index in 0..frames {
            left.push(output.samples[index * MASTER_CHANNELS]);
            right.push(output.samples[index * MASTER_CHANNELS + 1]);
        }
        match mastering::measure_master(sample_rate, &left, &right) {
            Some(master) if master.true_peak_dbtp.is_finite() => {
                let peak = master.true_peak_dbtp;
                (Some(peak), Some(-peak))
            }
            // 静音（真峰值 = −∞）或采样率没有核验系数 ⇒ 如实报 `null`，
            // ⛔ 绝不把"测不出"写成 `0 dBFS`（那是满量程，语义正好相反）。
            _ => (None, None),
        }
    };
    // 10a-bis. ⭐ #225：**削波预警**（只在 `clippedSamples > 0` 时出现一条）。
    //           ⛔ 只生成 JSON：不改样本 / 不改 `bytes` / 不改 `sha256`。
    let warnings: Vec<Value> = clipping_warning(
        clipped_samples,
        true_peak_dbfs,
        headroom_db,
        peak_after,
        &reports,
    )
    .into_iter()
    // ⭐ G1：可闻、有内容、却一个非零样本都没出的源必须被点名（见 `silent_sources_warning`）。
    .chain(silent_sources_warning(&reports))
    .collect();
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
    // 11. 抖动/量化 → 容器（母带 24-bit 整数 PCM 与分轨 32-bit 浮点**共用同一个**容器写入器，
    //     见 [`encode_container_with_format`] —— 头构造只有一份实现，不可能漂移）。
    let (
        bytes,
        container_kind,
        header_bytes,
        payload_bytes,
        origination_date,
        origination_time,
        originator_reference,
        coding_history,
    ) = match request.sample_format {
        SampleFormat::Pcm24 => write_master_container(
            &output.samples,
            sample_rate,
            project,
            request.format(),
            // 母带导出写 BWF 元数据（与 SFZ 导出刻意不同的**唯一**一处，见该函数的表）。
            true,
        )?,
        // ⭐ `ADR-0030` D2：分轨 = 32-bit IEEE 浮点、无抖动、**不写 `bext`**
        // （`coding history` 里的 W 随之如实写成 32）。
        SampleFormat::Float32 => {
            write_stem_container(&output.samples, sample_rate, project, request.format())?
        }
    };
    let project_digest = store::digest_of(store::serialize_project(project)?.as_bytes());
    let audio_clips: u64 = reports.iter().map(|report| report.audio_clips).sum();
    let notes_silenced: u64 = reports.iter().map(|report| report.notes_silenced).sum();
    let notes_ratcheted: u64 = reports.iter().map(|report| report.notes_ratcheted).sum();
    let ratchet_pulses: u64 = reports.iter().map(|report| report.ratchet_pulses).sum();
    Ok(RenderArtifact {
        request: request.clone(),
        path,
        container: container_kind,
        sample_rate,
        channels: MASTER_CHANNELS,
        bit_depth: request.sample_format.bit_depth(),
        frames: output.frames,
        blocks: output.blocks,
        block_size: L1_BLOCK_SIZE,
        header_bytes,
        payload_bytes,
        sha256: AssetHash::of_bytes(&bytes).as_str().to_owned(),
        master_digest: hex(&output.digest),
        longest_path_frames: output.longest_path_frames,
        bytes,
        node_count: nodes.len(),
        source_nodes,
        sources: reports,
        // ⭐ `ADR-0020`：一次渲染给不出"拿掉某轨差多少"；由调用方（`plan_render_master`）
        // 在 `build` 之后按需填。这样 `build` 仍然只做**一次**渲染。
        contributions: None,
        audio_clips,
        notes_silenced,
        notes_ratcheted,
        ratchet_pulses,
        rng_seed: project.rng_seed,
        audio_assets: asset_reports.into_values().collect(),
        peak_before,
        peak_after,
        clipped_samples,
        true_peak_dbfs,
        headroom_db,
        warnings,
        normalize_applied,
        normalize_note,
        loudness,
        originator_reference,
        origination_date,
        origination_time,
        coding_history,
        project_digest,
        rendered_at_unix_ms: now_ms,
        automation,
        sidechain_edges_summed_as_ordinary_audio,
        sidechain_edges_in_routing_graph,
        sidechain_edges_declared_without_key_input,
        unsupported,
        unsupported_counts: counts,
    })
}

/// ⭐ `ADR-0020`：**逐轨贡献**读数 —— "把这一轨静音，母带变了多少"。
///
/// 对每条**非 master** 轨各渲染一次"只静音该轨"，与**同一份基线**逐样本比。
/// 基线 = 本请求关掉归一化 / 目标响度 / 显式路径，并把帧数**钉成** `master.frames`
/// —— 静音"最晚结束"的那条轨不能让时间轴变短（否则两次渲染长度不同，差值无意义）。
///
/// 逐条渲染而不是在 `build` 里做：`build` 是"**一次**渲染"，这里要 `1 + N` 次。
/// ⛔ 一个字节都不改 `master`；成本如实进 `data.contributionsBasis.renders`。
///
/// # Errors
///
/// `RENDER_FAILED`：额外渲染失败，或两次渲染的位深 / 负载长度不一致。
pub fn track_contributions(
    project: &YebanProjectV1,
    project_path: &Path,
    master: &RenderArtifact,
    request: &RenderRequest,
    now_ms: u64,
    assets: &dyn AssetStore,
) -> Result<Vec<TrackContribution>, Fault> {
    let basis = RenderRequest {
        format_index: request.format_index,
        sample_rate: request.sample_rate,
        normalize: false,
        frames: Some(master.frames),
        duration_seconds_bits: None,
        target_lufs_bits: None,
        path: None,
        contributions: false,
        sample_format: request.sample_format,
    };
    let baseline = build(project, project_path, &basis, now_ms, assets)?;
    let reference = decode_master_payload(&baseline)?;
    let mut rows = Vec::with_capacity(project.tracks.len());
    // `tracks` 是 `BTreeMap` ⇒ 行序是身份的确定序（跨进程/跨重启一致）[MODEL-AST-003]。
    for (&track_id, track) in &project.tracks {
        if track_id == project.master_bus_track_id {
            continue;
        }
        if track.mute {
            // 渲染**之前**就已经静音 ⇒ "拿掉它"没有可测的变化，⛔ 不假装量过。
            rows.push(TrackContribution {
                track_id,
                name: track.name.clone(),
                already_muted: true,
                delta_rms_dbfs: None,
                delta_peak_dbfs: None,
            });
            continue;
        }
        let mut muted_project = project.clone();
        muted_project
            .tracks
            .get_mut(&track_id)
            .expect("轨来自同一份 tracks 遍历")
            .mute = true;
        let muted = build(&muted_project, project_path, &basis, now_ms, assets)?;
        let muted_samples = decode_master_payload(&muted)?;
        let (delta_rms_dbfs, delta_peak_dbfs) = delta_levels(&reference, &muted_samples)?;
        rows.push(TrackContribution {
            track_id,
            name: track.name.clone(),
            already_muted: false,
            delta_rms_dbfs,
            delta_peak_dbfs,
        });
    }
    Ok(rows)
}

/// 从母带容器字节里解出 24-bit PCM 负载（符号扩展后除以 2^23）。
fn decode_master_payload(artifact: &RenderArtifact) -> Result<Vec<f32>, Fault> {
    if artifact.bit_depth != OUTPUT_BIT_DEPTH {
        return Err(render_failed(
            format!("逐轨贡献只解码 {OUTPUT_BIT_DEPTH}-bit 负载"),
            serde_json::json!({ "bitDepth": artifact.bit_depth }),
        ));
    }
    let end = artifact
        .header_bytes
        .checked_add(artifact.payload_bytes)
        .ok_or_else(|| {
            render_failed(
                "母带负载区间溢出",
                serde_json::json!({
                    "headerBytes": artifact.header_bytes,
                    "payloadBytes": artifact.payload_bytes,
                }),
            )
        })?;
    let payload = artifact
        .bytes
        .get(artifact.header_bytes..end)
        .ok_or_else(|| {
            render_failed(
                "母带负载区间超出文件字节",
                serde_json::json!({
                    "bytes": artifact.bytes.len(),
                    "headerBytes": artifact.header_bytes,
                    "payloadBytes": artifact.payload_bytes,
                }),
            )
        })?;
    if payload.len() % 3 != 0 {
        return Err(render_failed(
            "母带负载字节数不是 3 的倍数",
            serde_json::json!({ "payloadBytes": payload.len() }),
        ));
    }
    let mut samples = Vec::with_capacity(payload.len() / 3);
    for sample in payload.as_chunks::<3>().0 {
        // 24-bit 小端 ⇒ 先零扩展到 i32，再左移 8 位把符号位送到 bit 31，
        // 算术右移 8 位完成符号扩展（与本文件测试的 `decode_i24` 同一口径）。
        let raw = i32::from_le_bytes([sample[0], sample[1], sample[2], 0]) << 8 >> 8;
        samples.push(raw as f32 / 8_388_608.0);
    }
    Ok(samples)
}

/// 两次母带样本的逐样本差（RMS / 峰值，dBFS）；逐样本全等 ⇒ `None`。
fn delta_levels(reference: &[f32], muted: &[f32]) -> Result<(Option<f64>, Option<f64>), Fault> {
    if reference.len() != muted.len() {
        return Err(render_failed(
            "逐轨贡献的两次渲染负载长度不同",
            serde_json::json!({
                "referenceSamples": reference.len(),
                "mutedSamples": muted.len(),
            }),
        ));
    }
    if reference.is_empty() {
        return Ok((None, None));
    }
    let mut sum_squares = 0.0_f64;
    let mut peak = 0.0_f64;
    for (baseline, muted) in reference.iter().zip(muted) {
        let delta = f64::from(*baseline) - f64::from(*muted);
        sum_squares += delta * delta;
        peak = peak.max(delta.abs());
    }
    let rms = (sum_squares / reference.len() as f64).sqrt();
    Ok((dbfs(rms), dbfs(peak)))
}

/// 线性幅度 → dBFS；0 ⇒ `None`（⛔ 不报 `-inf`）。
///
/// ⚠ 与 [`crate::domain::render_math::amplitude_dbfs`] 同口径：用 `libm`（纯 Rust）而不是
/// 宿主 `log10`，否则逐轨贡献读数会变成跨平台假设 [ARCH-DET-001]；本 crate 的生产区
/// 也因此在 `probe_authenticity` 的超越函数守卫下保持为空。
fn dbfs(amplitude: f64) -> Option<f64> {
    (amplitude > 0.0).then(|| 20.0 * libm::log10(amplitude))
}

/// `yeban_render_master` / `yeban_render_sfz_wav` 的容器入口：**24-bit 整数 PCM**。
///
/// 它只是 [`encode_container_with_format`] 的一层薄封装（把格式钉成
/// [`SampleFormat::Pcm24`]）—— ⛔ 头构造没有第二份实现。
#[allow(clippy::type_complexity)]
pub(super) fn write_master_container(
    samples: &[f32],
    sample_rate: u32,
    project: &YebanProjectV1,
    format: &str,
    with_bext: bool,
) -> Result<
    (
        Vec<u8>,
        ContainerKind,
        usize,
        usize,
        String,
        String,
        String,
        String,
    ),
    Fault,
> {
    encode_container_with_format(
        samples,
        sample_rate,
        project,
        format,
        with_bext,
        SampleFormat::Pcm24,
    )
}

/// ⭐ `ADR-0030` D2：**分轨（stem）**容器入口 —— 32-bit IEEE 浮点、无抖动、不写 `bext`。
///
/// 与母带共用 [`encode_container_with_format`]（同一份头构造），差别只有三条，
/// 全部由 [`SampleFormat::Float32`] 决定：
/// 1. `fmt ` 的 `audioFormat` = `WAVE_FORMAT_IEEE_FLOAT`、`bitsPerSample` = 32；
/// 2. **带 `fact` chunk**（非 PCM 容器必须带，`yeban-render` 的 `ContainerPlan`
///    按 `PcmFormat::is_float` 自动补，⛔ 本层不自己拼头）；
/// 3. `quantize(_, BitDepth::Float32, _)` 是**直通**分支 ⇒ 不加 TPDF 抖动
///    （抖动是"降位深"的伴生步骤，32-bit 浮点不降位深）。
///
/// 不写 `bext` 的理由与 `yeban_render_sfz_wav` 同一条：stem 是要**混回**的分轨，
/// 不是广播母带；`bext` 的 `OriginatorReference` / 日期戳描述的是"这份文件是谁"，
/// 逐轨重复一份只会让 N 个文件声称自己是同一个 originator。
#[allow(clippy::type_complexity)]
pub(super) fn write_stem_container(
    samples: &[f32],
    sample_rate: u32,
    project: &YebanProjectV1,
    format: &str,
) -> Result<
    (
        Vec<u8>,
        ContainerKind,
        usize,
        usize,
        String,
        String,
        String,
        String,
    ),
    Fault,
> {
    encode_container_with_format(
        samples,
        sample_rate,
        project,
        format,
        false,
        SampleFormat::Float32,
    )
}

/// **落盘字节的唯一编码器**：抖动/量化 → RIFF/RF64/BW64 容器（+ 可选 `bext`）。
///
/// 三条导出路径**共用**本函数（经 [`write_master_container`] / [`write_stem_container`] /
/// [`encode_container_without_bext`] 三个薄封装）—— 位深、抖动与容器选择因此不可能漂移
/// （"两份编码器必然漂移"是本仓的既有教训）。
///
/// `with_bext` 与 `sample_format` 是它们**刻意**的差别：
///
/// | 调用方 | `with_bext` | `sample_format` | 为什么 |
/// | :--- | :--- | :--- | :--- |
/// | `yeban_render_master` | `true` | `Pcm24` | 广播级母带要 BWF 元数据（`bext`：originatorReference / 日期 / coding history） |
/// | `yeban_render_sfz_wav` / `yeban_record_wav` | `false` | `Pcm24` | 它们**不写** `bext` ⇒ 输出不含时间戳 ⇒ 同一个请求**逐字节可复现**（判据 `tests/sfz_tools.rs::dry_run_writes_nothing_and_matches_the_real_call` 用同一份 SHA-256 钉住这一点） |
/// | ⭐ `yeban_export_stems` (`ADR-0030`) | `false` | `Float32` | 分轨是**内部精度**交付（`ARCH-DSP-003` 的 32-bit Float）⇒ 不降位深也就不抖动；`fact` chunk 由 `ContainerPlan` 补 |
///
/// 返回 `(容器字节, 容器种类, 头部字节数, 载荷字节数, bext 日期, bext 时间,
/// originatorReference, codingHistory)`；`with_bext == false` 时后四项为空串。
///
/// # BWF 的两条 ASCII 戳**不取墙钟**（`#214`）
///
/// `OriginationDate` / `OriginationTime` 的唯一来源是
/// [`YebanProjectV1::metadata`]`.created_at_unix_ms` —— 工程创建时刻**是输入的一部分**，
/// 因此"同一份工程 ⇒ 同一串字节"。墙钟（`Domain::now_ms`）只进响应的
/// `renderedAtUnixMs`，⛔ 一个字节都不进文件。
///
/// 之前的实现把导出时刻（墙钟）写进这两条戳：`masterDigest` 稳定而整文件
/// `sha256` 每次不同（`#209` 实测：`58ec4f71…` vs `9b5d79e6…`），缺陷表现为
/// "同一命令跑两次得到两份不同的 WAV"，而 PCM 一位没差。本函数的签名**不收**
/// 墙钟参数，就是为了让那条路径在类型系统上不可能被重新接回来。
///
/// `created_at_unix_ms == 0`（未知/未记录）⇒ 写 `bext` 的默认纪元
/// `1970-01-01` / `00:00:00`：那是**确定**的"未知"，而不是每次不同的"现在"。
///
/// # Errors
///
/// 容器编码失败 ⇒ `RENDER_FAILED`（与调用方原来的出口同一个码）。
#[allow(clippy::type_complexity)]
fn encode_container_with_format(
    samples: &[f32],
    sample_rate: u32,
    project: &YebanProjectV1,
    format: &str,
    with_bext: bool,
    sample_format: SampleFormat,
) -> Result<
    (
        Vec<u8>,
        ContainerKind,
        usize,
        usize,
        String,
        String,
        String,
        String,
    ),
    Fault,
> {
    let mut rng = dither_rng_for(project.rng_seed, project.master_bus_track_id);
    let pcm = quantize(samples, sample_format.quantize_to(), &mut rng);
    let payload = pcm.to_le_bytes();
    let (origination_date, origination_time) = if with_bext {
        // 墙钟**不**在这里：见函数文档"BWF 的两条 ASCII 戳不取墙钟"。
        math::bext_stamp(project.metadata.created_at_unix_ms)
    } else {
        (String::new(), String::new())
    };
    let originator_reference = if with_bext {
        project.id.to_canonical_string()
    } else {
        String::new()
    };
    let coding_history = if with_bext {
        format!(
            "A=PCM,F={sample_rate},W={},M=stereo,T=Yeban-MCP",
            sample_format.bit_depth()
        )
    } else {
        String::new()
    };
    let bext = with_bext.then(|| {
        let mut bext =
            Bext::for_project(&originator_reference, &origination_date, &origination_time);
        // `Bext::for_project` 的 coding history 是模板字符串(带 `<sample_rate>` 字面量);
        // 母带导出必须写**真实**参数, 否则那是文件里的假元数据。
        bext.coding_history.clone_from(&coding_history);
        bext
    });
    let (bytes, kind, header_bytes, payload_bytes) =
        encode_interleaved_container(samples, sample_rate, format, &payload, bext, sample_format)?;
    Ok((
        bytes,
        kind,
        header_bytes,
        payload_bytes,
        origination_date,
        origination_time,
        originator_reference,
        coding_history,
    ))
}

/// **容器写入器本体**（唯一）：交织 `f32` ⇒ 容器字节（位深/浮点由 [`SampleFormat`] 决定）。
///
/// 三个调用方共用它：`yeban_render_master`（带 `bext`）、`yeban_render_sfz_wav`
/// 与 `yeban_record_wav`（都 `None`）、⭐ `yeban_export_stems`（`Float32`，也 `None`）。
/// ⛔ 全仓只有这一个函数写 RIFF/RF64/BW64 头 —— 母带与分轨的差别**只有**
/// `PcmFormat::integer` / `PcmFormat::float` 这一个构造选择，`fact` chunk 由
/// `yeban-render` 的 `ContainerPlan` 按 `is_float` 自动补。
///
/// # Errors
///
/// 容器编码失败 ⇒ `RENDER_FAILED`。
fn encode_interleaved_container(
    samples: &[f32],
    sample_rate: u32,
    format: &str,
    payload: &[u8],
    bext: Option<Bext>,
    sample_format: SampleFormat,
) -> Result<(Vec<u8>, ContainerKind, usize, usize), Fault> {
    let preferred = match format {
        "rf64" => ContainerKind::Rf64,
        "bw64" => ContainerKind::Bw64,
        _ => ContainerKind::Riff,
    };
    let frames = u64::try_from(samples.len() / MASTER_CHANNELS).unwrap_or(0);
    // ⭐ `ADR-0030` D2：浮点与整数的**唯一**差别就是这一个构造选择（`is_float` ⇒
    // `WAVE_FORMAT_IEEE_FLOAT` + `fact` chunk，都由 `ContainerPlan` 负责）。
    let channels = u16::try_from(MASTER_CHANNELS).unwrap_or(u16::MAX);
    let container_format = match sample_format {
        SampleFormat::Pcm24 => PcmFormat::integer(channels, sample_rate, sample_format.bit_depth()),
        SampleFormat::Float32 => PcmFormat::float(channels, sample_rate, sample_format.bit_depth()),
    };
    let container_plan = ContainerPlan::for_payload(
        preferred,
        container_format,
        payload.len() as u64,
        frames,
        bext,
    );
    let header_bytes = container_plan.header_bytes().len();
    let payload_bytes = payload.len();
    let mut bytes = Vec::with_capacity(header_bytes + payload_bytes + 1);
    write_container(&mut bytes, &container_plan, payload).map_err(|error| {
        render_failed(
            "容器编码失败",
            serde_json::json!({ "rf64Error": format!("{error:?}") }),
        )
    })?;
    Ok((bytes, container_plan.kind, header_bytes, payload_bytes))
}

/// **无 `bext`** 的交织 `f32` ⇒ 24-bit 容器字节（录音工具用）。
///
/// 与 [`write_master_container`] 共用 [`encode_interleaved_container`] ⇒ 容器头不可能
/// 有两份实现。抖动种子由调用方给（录音侧用固定 `1`：录音没有"工程种子"，
/// 与 `crates/yeban-app/src/record_wav.rs` 的 `DeterministicDitherRng::new(1)` 同一口径）。
///
/// # Errors
///
/// 容器编码失败 ⇒ `RENDER_FAILED`。
pub(super) fn encode_container_without_bext(
    samples: &[f32],
    sample_rate: u32,
    dither_seed: u32,
) -> Result<(Vec<u8>, usize, usize), Fault> {
    let mut rng = DeterministicDitherRng::new(dither_seed);
    let pcm = quantize(samples, BitDepth::Int24, &mut rng);
    let payload = pcm.to_le_bytes();
    let (bytes, _kind, header_bytes, payload_bytes) = encode_interleaved_container(
        samples,
        sample_rate,
        "wav",
        &payload,
        None,
        SampleFormat::Pcm24,
    )?;
    Ok((bytes, header_bytes, payload_bytes))
}

/// 一个保留节点是不是**源节点**（没有入边）。
///
/// `bus_reduction_order` 对源节点返回 `Some(vec![])`（空入边列表），对总线返回非空列表。
pub(super) fn is_source_node(plan: &RenderPlan, node: EntityId) -> bool {
    plan.bus_reduction_order(node)
        .is_none_or(|inputs| inputs.is_empty())
}

/// 一次**探针**渲染计划的三个落点视图（不渲染一个样本）。
///
/// 这是 `build` 与 `yeban_edit_automation` 的读侧**共用**的探针：两边必须看到
/// 同一张保留子图，否则"写的时候说会应用"与"导出时说不支持"会各判一套。
pub(super) struct ProbeSubgraph {
    /// 保留子图里的全部节点（`RenderPlan::nodes()`）。
    pub(super) nodes: Vec<EntityId>,
    /// 其中的源节点（没有入边的节点；主总线也算）。
    pub(super) source_nodes: Vec<EntityId>,
    /// 保留子图里的全部边（`RenderPlan::edge_ids()`）。
    pub(super) rendered_edges: Vec<EntityId>,
}

/// 编译一次探针计划并取出三个落点视图。
///
/// 与 `build` 的探针**同一条**：`track_latencies` + `RenderOptions::l1(1, …)` +
/// `RenderPlan::compile_with_latencies` ⇒ 剪枝/分层逻辑不复制第二份。
///
/// # Errors
///
/// 路由图非法 / 主总线不在图里 / PDC 分析失败 —— 与真实导出在**同一处**失败。
pub(super) fn probe_subgraph(
    project: &YebanProjectV1,
    sample_rate: u32,
) -> Result<ProbeSubgraph, RenderError> {
    let latencies = track_latencies(&project.tracks);
    let probe = RenderOptions::l1(1, MASTER_CHANNELS, sample_rate, project.rng_seed);
    let plan = RenderPlan::compile_with_latencies(
        &project.routing_graph,
        project.master_bus_track_id,
        probe,
        &latencies,
    )?;
    let nodes = plan.nodes();
    let source_nodes = nodes
        .iter()
        .copied()
        .filter(|node| is_source_node(&plan, *node))
        .collect();
    let rendered_edges = plan.edge_ids();
    Ok(ProbeSubgraph {
        nodes,
        source_nodes,
        rendered_edges,
    })
}

/// `yeban_edit_automation` 读侧的**离线落点**读数。
///
/// 它复用 `yeban_render::automation::unsupported_lane_reason`（与 `AutomationReport`
/// 同一份判定）与 [`probe_subgraph`]（与 `build` 同一份探针），⛔ 本层不维护清单。
pub(super) struct OfflineLaneStatus {
    /// 渲染器是否消费这个目标（与 `data.automation[].supported` 同义）。
    pub(super) supported: bool,
    /// 没有落点时的原因（`None` = 有落点）。
    pub(super) residual: Option<LaneResidual>,
    /// 探针编译失败的原因（`None` = 探针正常）。
    ///
    /// 失败时 `supported = false`，但 `residual` 也是 `None` —— 这不是"某条泳道的
    /// 残差"，而是"这份工程根本编译不出渲染计划"（导出会以同一个原因 `RENDER_FAILED`）。
    pub(super) error: Option<String>,
}

/// 判定一条目标在离线渲染里会不会被应用（写侧读数的唯一入口）。
pub(super) fn offline_lane_status(
    project: &YebanProjectV1,
    target: &AutomationTarget,
) -> OfflineLaneStatus {
    let sample_rate = project.audio_config.sample_rate.hz();
    match probe_subgraph(project, sample_rate) {
        Ok(probe) => {
            let residual = yeban_render::automation::unsupported_lane_reason(
                project,
                target,
                &probe.source_nodes,
                &probe.rendered_edges,
                &probe.nodes,
            );
            OfflineLaneStatus {
                supported: residual.is_none(),
                residual,
                error: None,
            }
        }
        Err(error) => OfflineLaneStatus {
            supported: false,
            residual: None,
            error: Some(error.to_string()),
        },
    }
}

/// `u64` 帧数 → `i64` 的饱和转换（`ClipSpan` 用 `i64`；上限远小于 `i64::MAX`）。
fn frames_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
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

/// 一个音频片段摆放的**帧区间 + 左右增益**（tick→帧的换算已在构造时做完）。
#[derive(Clone, Copy, Debug, PartialEq)]
struct ClipSpan {
    /// 起始帧（含）。
    start: i64,
    /// 结束帧（不含）。
    end: i64,
    /// 素材里的起始帧（`#241`）：循环重复时每一遍只搬循环区间那一段素材。
    /// 非循环摆放恒为 `0` ⇒ 与加循环展开之前逐位相同。
    source_offset: i64,
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
/// - **累加**（不是覆写）：调用方负责先把缓冲清零（引擎源自己 `fill(0.0)`，
///   本类型只往上加）；
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
                .then(a.source_offset.cmp(&b.source_offset))
                .then(a.gain_l.to_bits().cmp(&b.gain_l.to_bits()))
                .then(a.gain_r.to_bits().cmp(&b.gain_r.to_bits()))
        });
    }

    /// 本块内某一帧在**素材**里的下标（超出素材则 `None`）。
    fn asset_frame(&self, span: &ClipSpan, global: i64) -> Option<usize> {
        let offset = global - span.start + span.source_offset;
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
///
/// ⭐ `#268`（`ADR-0011`）：`midi` 是 **`yeban-engine` 的引擎音源**
/// （[`EngineTrackSource`]，与 CLI / 界面**同一份**实现）—— 本 crate 不再有正弦振荡器。
/// `None` 的唯一情形：该轨在快照里没有可发声音符（纯音频轨，不白建一台 109 KB 的引擎）。
///
/// ⭐ `#274`：**采样器轨也走这一支**（`TrackV3.sampler` ⇒ 引擎的 SFZ 支路）；
/// `sampler_bound` 只用于 `sources[].kind` 的命名 —— 它⛔ 不参与发声
/// （旧的 `SamplerTrackSource` 已退休，⛔ 不再有第二份采样音源）。
struct TrackSource {
    midi: Option<EngineTrackSource>,
    /// 本轨是否绑了 `TrackV3.sampler`（`kind` 命名用）。
    sampler_bound: bool,
    clips: Vec<AudioClipSource>,
    /// ⭐ 本轨的 `TrackVolume` 自动化；`None` = 恒等快路径（没有可求值点 / 曲线处处
    /// 等于静态推子）。它在源输出**之后**乘 R55 比率，因此包含 MIDI、采样器与音频片段
    /// 三路的全部贡献。
    automation: Option<TrackGainAutomation>,
    /// ⭐ `#257`：本轨的 `TrackPan` 自动化；`None` = 恒等快路径（没有可求值点 /
    /// 曲线处处等于静态 `pan`）。非恒等时源侧静态声相已被关掉（`track_pan_gains`
    /// 收到 `pan = 0`），这里在源输出之后乘**绝对**左右增益 —— 顺序在音量之前，
    /// 与静态路径（源里的声相 → 音量）同序。
    pan_automation: Option<TrackPanAutomation>,
}

/// ⭐ `#225`：**只读**的逐源峰值探针（包装一个 [`TrackSource`]）。
///
/// ## 为什么需要它
///
/// `sources[]` 此前只有"响了几个音符 / 几个片段"，没有"这条轨多响" ⇒
/// 削波预警**点不出名字**（只能让调用方自己去猜该减谁）。本探针在每块渲染之后扫一遍
/// 已经写进 `out` 的样本，取 `max |sample|`。
///
/// ## 它为什么**不改音频**
///
/// - 只调一次 `inner.render_block`，写进去的字节一个不改；
/// - 只读 `out`，不写；
/// - 峰值存进 `AtomicU32`（`fetch_max`），⛔ 不参与增益、⛔ 不参与归一化。
///
/// 判据：`tests/render_master.rs` 的
/// `the_clipping_warning_does_not_change_a_single_byte` 用**加预警前**整文件
/// `sha256` 的常量钉住这一点。
///
/// ## 测点口径（写进响应，⛔ 不让人猜）
///
/// `out` 是**源节点**的输出：含轨音量 / 声相 / 采样器与片段增益；
/// ⛔ **不含**该轨插入链（`render_node` 在探针之后才 `inserts.process`）
/// 与路由边增益。因此 `sources[].peak` 与 `peak.after` 不是同一个数：
/// 前者是单条轨的输出，后者是母带求和之后的样本峰值。
///
/// ⭐ `#303` 的边界（实测读数在 `tests/render_master.rs` 的
/// `a_compressed_track_source_peak_is_taken_before_the_insert_chain`）：**会执行压缩的
/// 源轨**把声相搬到了输出级 ⇒ 这类轨的 `sources[].peak` 是**声相之前**的读数
/// （`pan == 0` 时与搬前逐位相同）。插入链在探针之后 ⇒ **压缩本身**永远不改这个数。
struct PeakTap {
    inner: TrackSource,
    /// 峰值位型（非负有限 `f32` 的位型与数值同序 ⇒ `fetch_max` 就是"取更大"）。
    peak_bits: Arc<AtomicU32>,
}

impl AudioSource for PeakTap {
    fn render_block(&mut self, context: BlockContext, out: &mut [f32]) -> Result<(), RenderError> {
        self.inner.render_block(context, out)?;
        let peak = math::peak_of(out);
        if peak > 0.0 {
            self.peak_bits.fetch_max(peak.to_bits(), Ordering::Relaxed);
        }
        Ok(())
    }
}

impl TrackSource {
    /// 把音频片段的区间夹进母带长度（母带长度在构造之后才知道）。
    fn clamp_to(&mut self, frames: u64) {
        for clips in &mut self.clips {
            clips.clamp_to(frames);
        }
    }

    /// 源种类（响应 `sources[].kind`）。
    ///
    /// ⭐ `#274`：`has_sampler` 现在看 `sampler_bound`（不再有 `SamplerTrackSource`）——
    /// 只影响**命名**，发声仍由 `midi` 那一支（引擎 SFZ 支路）负责。
    fn kind(&self) -> &'static str {
        let has_midi = self
            .midi
            .as_ref()
            .is_some_and(|midi| midi.scheduled_notes() > 0);
        let has_sampler = self.sampler_bound && has_midi;
        let has_audio = self.clips.iter().any(|clips| !clips.spans.is_empty());
        match (has_sampler, has_midi, has_audio) {
            (true, _, true) => SAMPLER_MIXED_SOURCE_KIND,
            (true, _, false) => SAMPLER_SOURCE_KIND,
            (false, true, true) => MIXED_SOURCE_KIND,
            (false, true, false) => SOURCE_KIND,
            (false, false, true) => AUDIO_SOURCE_KIND,
            (false, false, false) => SILENT_SOURCE_KIND,
        }
    }
}

impl AudioSource for TrackSource {
    fn render_block(&mut self, context: BlockContext, out: &mut [f32]) -> Result<(), RenderError> {
        // 调用方（`yeban-render` 的 `render_node`）已经把缓冲清零；这里只累加。
        // ⚠ 引擎源是**有状态**的（内部播放头）：它自己 `out.fill(0.0)` 再写单声道，
        //    且强制"块必须从第 0 帧起连续升序"（见 `EngineTrackSource::render_block`）。
        //    它是本容器的**第一路**，因此覆写不会抹掉别的贡献。
        if let Some(midi) = &mut self.midi {
            midi.render_block(context, out)?;
        }
        for clips in &mut self.clips {
            clips.render_block(context, out)?;
        }
        // ⭐ 声相自动化乘在**源输出之外**（含上面三路），且在音量之前 ——
        // 与静态路径同序（源里的声相 → 音量）。`None` 时整段不碰。
        if let Some(automation) = &mut self.pan_automation {
            automation.apply_pan_block(
                out,
                context.first_frame,
                context.frames,
                context.channels,
                pan_gains,
            );
        }
        // ⭐ 音量自动化乘在**源输出之外**（含上面三路）；`None` 时整段不碰。
        if let Some(automation) = &mut self.automation {
            automation.apply_ratio_block(
                out,
                context.first_frame,
                context.frames,
                context.channels,
            );
        }
        Ok(())
    }
}

/// ⭐ `#274`：一条采样器轨的**武装材料**（provider 交回；报告与读数用）。
///
/// 为什么把 `SfzLoad` 留在这里：`data.sources[].sampler` 的读数（区域摘要 /
/// 样本来源 / 读失败 / 累计帧数）本来就在装载期算好 —— 再算一遍会变成第二份口径。
/// ⭐ `#279`：解码表（`ArmedSfzSample`）**不再**复制进本类型 —— 它由 provider 直接交给
/// 引擎（`ArmedSfzInput`），本类型只剩**读数**（`load` ＋ `decoded`）。旧字段
/// `samples: Vec<ArmedSfzSample>` 在整条渲染里**零读者**（只有一处 `clone()` 中转），
/// 却按轨深拷一份 PCM ⇒ 删除。
struct ArmedSamplerTrack {
    /// 本次装载的读数（与 `yeban_load_sfz` 工具同一份 `SfzLoad`）。
    load: sfz::SfzLoad,
    /// 解码成功的 `sample=` 原样字符串（读数用；`survey_sampler` 的 `decoded` 集合）。
    decoded: BTreeSet<String>,
}

/// ⭐ `#274` 的 **provider 实现**：解析路径 → 装载 SFZ（**一次**解析）→ 交出引擎需要的
/// 解码表（⭐ `#279`：返回值 ＝ `(读数 ArmedSamplerTrack, 解码表 Vec<ArmedSfzSample>)`
/// 两件；解码表直接从池里的 `Arc` 克隆，⛔ 不复制 PCM 元素）。
///
/// ## 为什么解码留在这一侧（⛔ 不下移 `yeban-decode`）
///
/// `yeban-render` 是**离线渲染栈**：把 `symphonia` / `rubato` 放进去会让每个
/// `cargo test -p yeban-render` 都编解码器栈，也违背 `yeban-mcp` 的"轻量 crate"边界
/// （`#270` 的禁区）。因此本层把**已经解码**的 PCM 交出去，引擎只做摊平。
///
/// ## 与 CLI 的口径
///
/// 解码走 [`sfz::read_sample_file`]（WAV 走 `read_plain_wav_mono_f32`、其余走
/// `yeban-decode`）—— 与 `yeban_app::sfz_load::read_sample_file` **同一条腿**；
/// 去重键是 `region.sample_path()` 的**原样字符串**（与引擎摊平的去重键同一句）。
///
/// ⭐ `#276`：这条腿不再**自己**解析、也不再**自己**解码 ——
/// [`sfz::load_with`] 在同一次装载里既产出 `SfzLoad` 读数，又产出提供表；
/// 样本文件由 [`sfz::SampleStore`]（一次渲染共享）负责"同一文件只解码一次"。
///
/// 读不到 ⇒ **不放进提供表**（引擎按缺样本 ⇒ 静音 ＋ 计数，⛔ 不发明替代内容）。
///
/// # Errors
///
/// 路径解析不到 / SFZ 文件读不到 / 文本非法 ⇒ `RENDER_FAILED`（报文与原形状逐字相同）。
fn prepare_sampler_track(
    _track: EntityId,
    raw: &str,
    project: &YebanProjectV1,
    project_path: &Path,
    store: &mut sfz::SampleStore,
) -> Result<(ArmedSamplerTrack, Vec<ArmedSfzSample>), Fault> {
    let resolved = resolve_sampler_path(raw, project_path)?;
    // ⭐ `#276`：装载与提供表**同一次解析**；解码由 `store` 去重（同一文件只解一次）。
    let mut cache = sfz::TrackSampleCache::new(store);
    let (load, table) = sfz::load_with(
        &resolved,
        project.audio_config.sample_rate.hz() as f32,
        0,
        false,
        &mut cache,
    )
    .map_err(|fault| {
        let cause = fault.into_result().map_or_else(
            |error| serde_json::json!({ "code": error.code, "message": error.message }),
            |value| value["error"].clone(),
        );
        render_failed(
            format!(
                "采样器定义文件读不到或解析不了: {} (工程里登记的是 `{raw}`)",
                resolved.display()
            ),
            serde_json::json!({
                "reason": "samplerUnreadable",
                "path": raw,
                "resolvedPath": resolved.display().to_string(),
                "cause": cause,
            }),
        )
    })?;
    // 读不到 ⇒ 不进提供表（引擎按缺样本处理），原因已在 `SfzLoad::sample_failures`
    // 里如实登记。表的顺序/键与修前逐条相同：区域首见顺序、`sample_path()` 原样字符串。
    // ⭐ `#279`：池里那份已解码 PCM 是 `Arc<Vec<f32>>` ⇒ 这里只 `Arc::clone`
    // （⛔ 不再 `as_ref().clone()` 复制元素）；引擎摊平表随后也 `Arc::clone` 同一份
    // ⇒ 整条渲染只有**一份** PCM 字节。
    let samples: Vec<ArmedSfzSample> = table
        .into_iter()
        .filter_map(|(raw_path, outcome)| {
            outcome
                .ok()
                .map(|reading| (raw_path, (Arc::clone(&reading.pcm), reading.sample_hz)))
        })
        .collect();
    let decoded: BTreeSet<String> = samples.iter().map(|(path, _)| path.clone()).collect();
    Ok((ArmedSamplerTrack { load, decoded }, samples))
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

/// 解析 `TrackV3.sampler.path` 到一份**真的存在**的文件（⭐ `#221`，⭐ `#271` 补两档）。
///
/// 候选次序与 app 侧 `sfz_load::resolve_sampler_path` 的**四档**同口径：
///
/// 1. **绝对路径** ⇒ 只试它自己；
/// 2. **相对路径** ⇒ 依次试
///    ① 工程文件所在目录；
///    ② **采样库根**（[`super::sample_roots::library_root`]：`YEBAN_SAMPLE_LIBRARY` 非空覆盖，否则按平台惯例；`assets/samples/` 前缀先剥掉 ⇒ 库内布局是 `<库根>/<乐器>/…`）；
///    ③ **仓库根**（[`super::sample_roots::repo_root`]：`CARGO_MANIFEST_DIR/../..`，内置清单相对路径的契约根）；
///    ④ 进程 cwd（"原样"路径 —— 与 `#221` 修前**同一形状**，按 cwd 解析）。
///
/// 候选按 `PathBuf` 去重（工程目录可能就是仓库根），先命中先返回。顺序的理由逐条照
/// app 的文档：工程目录最具体；库根用户可写、内置样本字节住在那里（所以它优先于仓库根
/// 里那份只有 SFZ 文本的空目录）；仓库根是内置清单的契约；cwd 只是 `#79` 之前的历史行为。
///
/// ⭐ `#271`：修前只有 ① 和 ④ —— 装在采样库里的内置 SFZ 在 MCP 导出上会报
/// `samplerUnreadable`，而 CLI 正常加载。本函数现在与 CLI 找同一个文件。
///
/// ## 与 app 的两条**如实登记的**口径差
///
/// - **绝对路径**：app 原样返回、⛔ 不查存在性（缺文件由后续 `Read` 错误兜住）；本层仍
///   先 `is_file` 判一次、失败即报 `samplerUnreadable` —— `#221` 既有行为，且理由更准
///   （`#324`：`samplerUnreadable` 只在真的不可读时出现）；
/// - **cwd 档**：app 的候选是 `cwd.join(raw)`（绝对），本层仍把**原样**相对路径作为候选
///   与返回值 —— 二者解析到同一个文件，但本层保留 `#221` 的 `resolvedPath` 形状
///   （`#271` 判据 4 的"正对照"：本来就能解析的路径不改行为）。
///
/// # Errors
///
/// 四档都不是文件 ⇒ `RENDER_FAILED` + `data.reason = "samplerUnreadable"`
/// （带 `tried` 列表，逐条按尝试顺序）。⛔ **绝不**静默回退成正弦 —— 那正是
/// `#220` 缺陷 ② 的形状。
fn resolve_sampler_path(raw: &str, project_path: &Path) -> Result<PathBuf, Fault> {
    use super::sample_roots::{library_root, repo_root};

    let candidate = Path::new(raw);
    let mut tried: Vec<PathBuf> = Vec::new();
    if candidate.is_absolute() {
        tried.push(candidate.to_path_buf());
    } else {
        if let Some(dir) = project_path.parent() {
            push_unique_candidate(&mut tried, dir.join(candidate));
        }
        // 库内布局剥掉 `assets/samples/` 前缀（与 app 及下载脚本 `--dest` 同一条规则）。
        let library_relative = candidate
            .strip_prefix("assets/samples")
            .unwrap_or(candidate);
        push_unique_candidate(&mut tried, library_root().join(library_relative));
        push_unique_candidate(&mut tried, repo_root().join(candidate));
        // 最后一档：**原样**（相对进程 cwd）—— 保留 `#221` 既有形状，见上面的口径差。
        push_unique_candidate(&mut tried, candidate.to_path_buf());
    }
    for path in &tried {
        if path.is_file() {
            return Ok(path.clone());
        }
    }
    Err(render_failed(
        format!(
            "音轨绑定的采样器定义文件不存在: `{raw}`（试过 {}）",
            tried
                .iter()
                .map(|path| format!("{}", path.display()))
                .collect::<Vec<_>>()
                .join(" / ")
        ),
        serde_json::json!({
            "reason": "samplerUnreadable",
            "path": raw,
            "tried": tried
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>(),
            "hint": "`setTrackSampler` 只把路径当留痕字符串（不查存在性）; \
                     相对路径按 工程目录 ⇒ 采样库根 ⇒ 仓库根 ⇒ cwd 解析",
        }),
    ))
}

/// 按 `PathBuf` 逐字去重地追加一个候选（顺序 = 尝试顺序；与 app 的
/// `push_unique_candidate` 同一条，⭐ `#271`）。
fn push_unique_candidate(tried: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !tried.contains(&candidate) {
        tried.push(candidate);
    }
}

/// 音轨可闻门：静音，或"有轨道独奏而自己既不独奏也不独奏安全"时为 `false`。
///
/// 与 `yeban-engine::synth::track_is_audible` **同式**；⛔ MCP 不依赖 `yeban-engine`
/// （依赖方向门禁），所以这里是本 crate 的**唯一**一份（源轨与总线轨共用）。
fn track_is_audible(track: &TrackV3, any_solo: bool) -> bool {
    !track.mute && (!any_solo || track.solo || track.solo_safe)
}

/// 由一条音轨构造音频源 + 报告，并把"没渲染的部分"登记进 `unsupported`。
#[allow(clippy::too_many_arguments)]
fn build_source(
    project: &YebanProjectV1,
    timbre: &EngineTimbre,
    track: &TrackV3,
    sample_rate: u32,
    any_solo: bool,
    assets: &dyn AssetStore,
    cache: &mut BTreeMap<AssetHash, Option<Arc<PreparedAsset>>>,
    asset_reports: &mut BTreeMap<AssetHash, AudioAssetReport>,
    sampler_loads: &BTreeMap<EntityId, ArmedSamplerTrack>,
    sampler_steals: &mut BTreeMap<EntityId, Arc<AtomicU64>>,
    unsupported: &mut BTreeSet<&'static str>,
    counts: &mut BTreeMap<&'static str, u64>,
    // ⭐ `#303`：本轨会执行压缩 ⇒ 源侧**不带声相**（pre-pan tap；输出级负责声相）。
    pre_pan: bool,
) -> Result<(TrackSource, SourceReport), Fault> {
    // solo 语义（B2 起也作用于辅助返回总线 —— 见 `build` 的输出级）；判据与源轨
    // 同一条 `track_is_audible`：只要工程里有任一音轨 solo, 未 solo 且非 solo-safe
    // 的轨就不发声。`any_solo` 扫全部轨（含主总线）。
    let audible = track_is_audible(track, any_solo);
    // `#339`：本轨的 tick → 帧换算读工程的速度点（空表 = 恒定 BPM，逐位不变）。
    let tempo = project.tempo();
    // ⭐ `#257`：本轨的 `TrackPan` 自动化。**先**构造它，再决定源侧静态声相 ——
    // 非恒等时把静态声相关掉（`pan = 0` ⇒ `(track_gain, track_gain)`），
    // 改由源输出之后的门乘自动化位置的绝对左右增益（同一份 `pan_gains` 曲线，
    // 且只乘一次）。恒等 ⇒ 源侧静态声相原样保留 ⇒ 无泳道工程的字节逐位不变。
    //
    // ⭐ `#303`：会执行压缩的轨（`pre_pan`）把**静态与自动化声相都**交给输出级
    // （`pre_pan` 来自 `compressing_nodes_with_lanes`，见 `inserts` 模块头 §1.1；
    // ⛔ 不是静态 `compresses_statically`）：源侧 `pan = 0`
    // 且不挂 `pan_automation` ⇒ 压缩器的检波器看到**声相之前**的电平（与实时同序）。
    let pan_automation = TrackPanAutomation::new(project, track.id, sample_rate);
    let pan_identity = pan_automation.is_identity() || pre_pan;
    let source_pan = if pan_identity {
        if pre_pan { 0.0 } else { track.pan }
    } else {
        0.0
    };
    // ⚠ `#274`：采样器轨与 CPU 轨**共用**同一个引擎源 ⇒ 轨增益由引擎的
    //   `NoteSchedule::track_gain` 乘，本层只给等功率那一半（见下面 `midi` 的装配）。
    //   旧的 `SamplerTrackSource`（自带 `gain_l/gain_r`）已退休。
    // ⭐ `#221`：绑了采样器 ⇒ 这条轨的音符走**采样器音源**（⛔ 不走正弦振荡器）。
    //    这条判据只看 `TrackV3.sampler` 是否为 `Some`，与 app 的
    //    `sfz_pending()` → `arm_pending_sfz` 那条链的入口条件**同一个字段**。
    let sampler_bound = track.sampler.is_some();
    // 设备链: 通道条（增益 / EQ / 低通 / **压缩**）、Freeverb、卷积已经由
    // `yeban-render` 消费（压缩自 `#303` 起）；音源参数（音色 `osc*_wave` /
    // 电平 / 失谐 / 滤波 / 鼓机）由 `yeban-engine` 的音源投影消费（⭐ G5，
    // 判据在 `yeban-engine` 的 `every_consumed_instrument_param_reaches_the_engine_projection`）；
    // 只有**没有任何离线路径消费**的参数（未知旋钮 / `filter_cutoff_hz` /
    // `reverb_mix` / `reverb_width` / 外部器件）仍要如实登记。
    // ⚠ 采样器轨是例外：音源是 SFZ ⇒ **正弦/鼓机音源的参数不会被求值**，
    // 这类轨上挂着带参数的内置乐器设备仍登记（报"已消费"就是假话）。
    let sampler_drops_instrument_params = sampler_bound
        && track.devices.iter().any(|device| {
            !device.bypassed
                && device.kind == yeban_model::DeviceKind::InternalInstrument
                && !device.params.is_empty()
        });
    if track_has_unhandled_dsp(track) || sampler_drops_instrument_params {
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
    // ⭐ `#221`：采样器轨的音符（`(起始帧, 时值帧, 音高, 力度)`）。
    // ⭐ `#268`（`ADR-0011`）：**非采样器轨**的音符不再在本层排成"正弦参数"——
    //    它们由 `yeban-engine` 的 `EngineSnapshot::from_project` 投影成 `NoteSchedule`，
    //    本层只保留**读数**（概率触发 / 连击 / 时间轴末端）与 SFZ 采样器需要的那一份。
    let mut sampler_notes: Vec<(u64, u64, u8, u8)> = Vec::new();
    // 每个资产 → 它在这条音轨上的摆放区间（同一资产的多个摆放共用一个源对象）。
    let mut per_asset: BTreeMap<AssetHash, (Arc<PreparedAsset>, Vec<ClipSpan>)> = BTreeMap::new();
    let mut clips_unrendered = 0u64;
    let mut clips_gated = 0u64;
    let mut notes_silenced = 0u64;
    let mut notes_ratcheted = 0u64;
    let mut ratchet_pulses = 0u64;
    let mut audio_placements = 0u64;
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
        // ⭐ `#246` / [ADR-0010 D5]：循环展开**只**由 `yeban-model` 决定
        // （`ClipPlacement::loop_repeat` / `note_copies`）。没有循环计划时
        // `note_copies` 返回原样一遍 ⇒ 与展开之前逐位相同。
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
                    // ⭐ `#311b`：`noteSlide` / `notePitchBend` **不再是**"没做"——
                    // 两个字段都由引擎的 `NoteSchedule` 真的求值（弯音曲线 ＋ 滑音，
                    // 相加），母带里听得到。因此这里**一次也不登记**：报一个已经
                    // 做到的键就是假话（与 `clipLoopRepetition` 退役同一条纪律）。
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
                    // ⭐ `#241`：循环区间内的音符按周期复制 N 遍；每遍以循环区间
                    // 末端为硬切边界（尾巴裁到 `end_tick`），最后一遍再裁到摆放末端。
                    // 区间之前的内容播放一遍；区间之后的内容不参与循环演奏。
                    for (copy_start, copy_duration) in
                        placement.note_copies(note.start_tick, note.duration_ticks)
                    {
                        let step = (copy_duration / ratchet).max(1);
                        if ratchet > 1 {
                            notes_ratcheted = notes_ratcheted.saturating_add(1);
                            ratchet_pulses = ratchet_pulses.saturating_add(ratchet);
                        }
                        let base_tick = placement.start_tick.saturating_add(copy_start);
                        for pulse in 0..ratchet {
                            // ⭐ `#268`：`note_frame_span` 现在**只服务采样器音源**。
                            // CPU 轨的音符由引擎快照投影（同一个 `yeban-model` 的
                            // tick→sample 换算），本层⛔ 不再排一份自己的音源参数。
                            if sampler_bound {
                                // 采样器音源要的是"起在哪、多长、多高、多重"。
                                let (start, end) = math::note_frame_span_with_tempo(
                                    base_tick.saturating_add(pulse.saturating_mul(step)),
                                    step,
                                    micro,
                                    PPQ,
                                    &tempo,
                                    sample_rate,
                                );
                                let origin = u64::try_from(start.max(0)).unwrap_or(0);
                                let span = u64::try_from((end - start).max(1)).unwrap_or(1);
                                sampler_notes.push((origin, span, note.pitch, note.velocity));
                            }
                        }
                        let shifted_micro = u64::try_from(micro.max(0)).unwrap_or(0);
                        end_tick = end_tick.max(
                            placement
                                .start_tick
                                .saturating_add(copy_start)
                                .saturating_add(shifted_micro)
                                .saturating_add(copy_duration),
                        );
                    }
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
                // ⭐ `#257`：`source_pan` 是“静态声相被自动化关掉后的位置”（恒等时就是
                // `track.pan`）⇒ 片段与 MIDI/采样器走**同一个**声相决定，⛔ 不是两条。
                let (gain_l, gain_r) = if source_pan.is_finite() && source_pan != 0.0 {
                    let (left, right) = pan_gains(source_pan);
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
                let (base_start, base_end) = clip_math::clip_frame_span_with_tempo(
                    placement.start_tick,
                    placement.duration_ticks,
                    PPQ,
                    &tempo,
                    sample_rate,
                );
                let entry = per_asset
                    .entry(asset.clone())
                    .or_insert_with(|| (Arc::clone(&prepared), Vec::new()));
                if let Some(repeat) = placement.loop_repeat() {
                    // ⭐ `#246`：音频片段走**同一份** `ClipPlacement::loop_repeat`。
                    // 每一遍只搬
                    // 循环区间 `[start, end)` 对应的素材段（`source_offset`），
                    // 第一遍之前的内容（若有）原样播一遍。
                    let base_start_frames =
                        math::frames_at_tick(placement.start_tick, PPQ, &tempo, sample_rate);
                    let region_start_frames = math::frames_at_tick(
                        placement.start_tick.saturating_add(repeat.start_tick),
                        PPQ,
                        &tempo,
                        sample_rate,
                    );
                    let region_end_frames = math::frames_at_tick(
                        placement.start_tick.saturating_add(repeat.end_tick),
                        PPQ,
                        &tempo,
                        sample_rate,
                    );
                    let period_frames = region_end_frames.saturating_sub(region_start_frames);
                    let base_end_frames = math::frames_at_tick(
                        placement
                            .start_tick
                            .saturating_add(placement.duration_ticks),
                        PPQ,
                        &tempo,
                        sample_rate,
                    );
                    if period_frames == 0 {
                        // tick 太短、帧换算后不足一帧 ⇒ 没有可重复的区间，
                        // 按单遍播放（不静默丢内容）。
                        entry.1.push(ClipSpan {
                            start: base_start,
                            end: base_end,
                            source_offset: 0,
                            gain_l,
                            gain_r,
                        });
                    } else {
                        // 区间之前的内容：播放一遍（素材从 0 开始）。
                        if region_start_frames > base_start_frames {
                            entry.1.push(ClipSpan {
                                start: frames_i64(base_start_frames),
                                end: frames_i64(region_start_frames),
                                source_offset: 0,
                                gain_l,
                                gain_r,
                            });
                        }
                        let source_offset =
                            frames_i64(region_start_frames.saturating_sub(base_start_frames));
                        let mut iteration = 0u64;
                        loop {
                            let start = region_start_frames
                                .saturating_add(iteration.saturating_mul(period_frames));
                            if start >= base_end_frames || iteration >= repeat.iterations {
                                break;
                            }
                            let end = start.saturating_add(period_frames).min(base_end_frames);
                            entry.1.push(ClipSpan {
                                start: frames_i64(start),
                                end: frames_i64(end),
                                source_offset,
                                gain_l,
                                gain_r,
                            });
                            iteration = iteration.saturating_add(1);
                        }
                    }
                } else {
                    entry.1.push(ClipSpan {
                        start: base_start,
                        end: base_end,
                        source_offset: 0,
                        gain_l,
                        gain_r,
                    });
                }
                audio_placements = audio_placements.saturating_add(1);
            }
        }
    }
    // ⭐ `#268`：本层不再有"正弦音符表"可排序 —— 音符的顺序由引擎快照投影的
    // `project_schedules` 唯一决定（`start_sample, pitch, end_sample` 全序）。
    // `#241`：一个摆放可能产生多个 `ClipSpan`（循环的每一遍），但响应里的
    // `audioClips` 口径是**摆放数** ⇒ 用逐摆放计数器，不用 span 数。
    let audio_clips = audio_placements;
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
    // ⭐ `#274`（`ADR-0011`）：采样器轨**改由引擎的 SFZ 支路发声**。
    //
    // 这一段是 `#221`／`#220` 缺陷 ② 的**最终形态**：在 `#274` 之前，本层用
    // `sfz::prepare_sampler` 逐音符烘 PCM（`SamplerTrackSource`），于是与引擎的
    // SFZ 支路成了**两份音源** —— `#272` 实测 `loop` / `offset,end` / `ampeg` /
    // 双层四项在两侧字节不同。现在：
    //
    // 1. 路径**读不到** ⇒ `RENDER_FAILED`（`reason: "samplerUnreadable"`，带 `tried`），
    //    发生在 `build` 的 provider 里（⛔ 绝不静默回退）；
    // 2. 读到 ⇒ 解码表交给 `EngineTimbre::from_project_armed` 摊平进快照 ⇒
    //    `render_track` 的第 3 参走引擎的 SFZ 支路（与 CLI 的 `arm_pending_sfz`
    //    **同一条**路径、**同一份** `SfzPlan`）；
    // 3. 读数仍进 `data.sources[].sampler`（含与 CLI 同形的 `sfz(...)` 行）：
    //    `regions`／`silent`／`missing` 来自**引擎的摊平表**（⛔ 不是另算一遍），
    //    逐音符计数来自**同一份已解析区域表**（`sfz::survey_sampler`，⛔ 不渲染）。
    //
    // ⚠ 因此本轨**要建引擎源**（与 CPU 轨同一支）—— 声相 / 轨增益也走同一条：
    //    引擎内部已含轨增益门（`NoteSchedule::track_gain`），本层只传等功率那一半。
    let sampler_report = match &track.sampler {
        None => None,
        Some(binding) => {
            let armed = sampler_loads.get(&track.id).ok_or_else(|| {
                render_failed(
                    format!(
                        "采样器轨没有武装记录: {} —— provider 必须为每条 `sfz_pending` 返回输入",
                        track.id.to_canonical_string()
                    ),
                    serde_json::json!({ "reason": "engineSource" }),
                )
            })?;
            let summary = timbre.armed_sfz(track.id).ok_or_else(|| {
                render_failed(
                    format!(
                        "采样器轨没有摊平表: {} —— `with_sfz` 没有注入该轨",
                        track.id.to_canonical_string()
                    ),
                    serde_json::json!({ "reason": "engineSource" }),
                )
            })?;
            // ⭐ 读数在同一份已解析区域表上投影（包络 / CC 调制 / 命中数）。
            //    `with_instrument` 只读重放：失败 ⇒ 空读数（⛔ 不影响渲染）。
            let survey = armed
                .load
                .with_instrument(|instrument| {
                    sfz::survey_sampler(instrument, &sampler_notes, &armed.decoded)
                })
                .unwrap_or_default();
            // ⭐ 第 324 条：**只登记还没做到的**。`#274` 之后循环 / 播放区间 /
            //    幅度包络 / 双层都由引擎应用 ⇒ 那四个键退役（== `KNOWN_UNSUPPORTED_KEYS`
            //    17 → 13）；静态 `amplitude` 也已由引擎应用 ⇒ 只剩 **CC 那一半**。
            if survey.notes_with_amplitude_cc > 0 {
                note_unsupported(unsupported, counts, "sfzAmplitude");
            }
            if survey.notes_with_tune_cc > 0 {
                note_unsupported(unsupported, counts, "sfzTuneCcModulation");
            }
            if survey.notes_with_volume_cc > 0 {
                note_unsupported(unsupported, counts, "sfzVolumeCcModulation");
            }
            Some(SamplerReport {
                path: binding.path.clone(),
                resolved_path: summary.resolved_path.display().to_string(),
                regions: summary.regions,
                silent_regions: summary.silent_regions,
                notes: u64::try_from(timbre.scheduled_notes(track.id)).unwrap_or(u64::MAX),
                notes_unmatched: survey.notes_unmatched,
                notes_using_file_sample: survey.notes_using_file_sample,
                // ⚠ ⭐ `#275`：引擎的 SFZ 支路**没有**正弦回退（缺样本 ⇒ 静音 ＋ 计数）
                //    ⇒ 本报告**不再有** `notesUsingSineFallback` 键（它曾恒 0、只占响应
                //    形状）；诚实的"没有样本"读数是 `notes_without_sample`。⛔ 回退只
                //    存在于 `yeban_render_sfz_wav` 旁路（见 `domain/sfz.rs`）。
                notes_without_sample: survey.notes_without_sample,
                notes_using_builtin_silence: survey.notes_using_builtin_silence,
                notes_looping: survey.notes_looping,
                sample_sources: armed.load.sample_sources.clone(),
                sample_failures: armed.load.sample_failures.clone(),
                sample_frames_loaded: armed.load.sample_frames_loaded,
                sample_reads_truncated: armed.load.sample_reads_truncated,
                regions_with_envelope: survey.regions_with_envelope,
                decoded_samples: summary.samples,
                missing_sample_paths: summary.missing_samples.clone(),
                voice_steals: 0,
            })
        }
    };
    // ⭐ `#268`（`ADR-0011` D2）：CPU 轨走**引擎音色** —— 适配器住在
    // `yeban-render::engine_source`，快照只投影一次（[`EngineTimbre`]），
    // 引擎内部自带轨增益门（`project_schedules` 的 `track_gain_scale`）。
    //
    // 声相因此**只**传等功率那一半（`centered_pan_gains`）：
    // 轨增益已经在引擎的 `note.gain` 里 ⇒ 这一层⛔ 不再乘第二次。
    // `pan == 0` / 声相自动化接管时它逐位等于 `(1.0, 1.0)`（与 CLI 同一条快路径）。
    //
    // ⚠ 采样器轨**不建引擎源**（`#221` 的 `sampler` 分支是整体替换，与 CLI 的
    //   `--sfz` 旁路同形）⇒ 否则采样与复音会叠成两份声音。
    // ⚠ 没有可发声音符的纯音频轨也不建（一台引擎约 109 KB，白建没有收益）。
    let midi = if timbre.scheduled_notes(track.id) == 0 {
        None
    } else {
        let (engine_l, engine_r) = centered_pan_gains(source_pan, pan_gains);
        Some(
            timbre
                .track_source(track.id)
                .map_err(|error| {
                    render_failed(
                        format!("引擎音源装配失败: {error}"),
                        serde_json::json!({
                            "reason": "engineSource",
                            "cause": error.message(),
                            "hint": format!(
                                "轨道 {} 的引擎源装配失败（快照已投影成功时这一条不应出现）",
                                track.id.to_canonical_string()
                            ),
                        }),
                    )
                })?
                .with_pan((engine_l, engine_r)),
        )
    };
    // ⭐ `#274`：把采样器轨的**声部窃取**句柄留到渲染之后（⛔ 不猜、⛔ 不静默）。
    if sampler_bound && let Some(counter) = midi.as_ref().map(EngineTrackSource::steal_counter) {
        sampler_steals.insert(track.id, counter);
    }
    // ⭐ 本轨的 `TrackVolume` 自动化：构造期判定恒等快路径，非恒等才挂到源上。
    let automation = TrackGainAutomation::new(project, track.id, sample_rate);
    let source = TrackSource {
        midi,
        sampler_bound,
        clips,
        automation: (!automation.is_identity()).then_some(automation),
        // ⭐ `#257`：声相自动化（恒等时 `None`，源侧静态声相保持原样）。
        pan_automation: (!pan_identity).then_some(pan_automation),
    };
    let report = SourceReport {
        node: track.id,
        track: track.name.clone(),
        kind: source.kind(),
        audible,
        notes: source.midi.as_ref().map_or(0, |midi| {
            u64::try_from(midi.scheduled_notes()).unwrap_or(u64::MAX)
        }),
        audio_clips,
        audio_clips_unrendered: clips_unrendered,
        audio_clips_gated: clips_gated,
        notes_silenced,
        notes_ratcheted,
        ratchet_pulses,
        end_tick,
        // ⭐ #225：真实值由渲染期的 `PeakTap` 填回（见 `build` 的 9a）。
        peak: 0.0,
        sampler: sampler_report,
    };
    Ok((source, report))
}

/// 等功率（−3 dB）声相增益：`θ = (pan + 1) * π/4`，`(cos θ, sin θ)`。
///
/// 与模型层 `PanLaw::ConstantPowerMinus3dB` 的口径一致：居中的 `pan = 0`
/// 给出 `(√2/2, √2/2)`（即每个声道 −3 dB）。
pub(super) fn pan_gains(pan: f32) -> (f32, f32) {
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
    // ⭐ 第 324 条：登记用的键必须来自 [`KNOWN_UNSUPPORTED_KEYS`] —— 否则"已知不支持项
    // 的全集"就不再是全集（清单漏一条，判据也数不到它）。
    debug_assert!(
        KNOWN_UNSUPPORTED_KEYS.contains(&key),
        "`{key}` 不在 KNOWN_UNSUPPORTED_KEYS 里: 先登记清单, 再登记这条读数"
    );
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

/// 单次渲染的帧数上限闸门（⭐ `#252`：**显式帧数**与**工程末端推导**共用这一处）。
///
/// 为什么抽成一个函数：`#252` 让 `validate` 也能拿到显式 `frames` ⇒ 参数层就要拒绝
/// 超限。两处各写一份 `if frames > …` 会让"上限"有两个来源（迟早漂移），而源码级判据
/// `the_frame_ceiling_gate_uses_the_constant_itself` 钉住"闸门紧贴常量"与"常量只出现
/// 三次（定义 + 比较 + 报文）" ⇒ 抽成一处后两条仍然成立，且**两处出口逐字相同**。
///
/// 出口故意是 `RENDER_FAILED`（不是 `INVALID_PARAMETER_RANGE`）：它与"工程自己的末端
/// 就超过上限"是**同一个**条件，调用方拿到的报文（`frames` + `maxFrames`）因此不因
/// 帧数从哪来而变。⛔ 不新增 `reason` 词：那会把 `fault_vocabulary` 的冻结摘要也拖进来。
fn frame_ceiling(frames: u64) -> Result<(), Fault> {
    if frames > MAX_RENDER_FRAMES {
        return Err(render_failed(
            "渲染帧数超出单次上限 (1 小时 @ 48 kHz)",
            serde_json::json!({ "frames": frames, "maxFrames": MAX_RENDER_FRAMES }),
        ));
    }
    Ok(())
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
pub(super) fn guard_output_path(output: &Path, project_path: &Path) -> Result<(), Fault> {
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
    fn an_absent_frame_override_keeps_the_project_end_default() {
        let request = validate(&args(
            serde_json::json!({"format": "wav", "sampleRate": 48000}),
        ))
        .expect("合法");
        assert_eq!(request.frames, None, "缺省不得有任何显式帧数");
        assert_eq!(request.duration_seconds(), None);
        assert_eq!(request.frames_source(), "projectEnd");
        let echoed = request.to_value();
        assert_eq!(echoed["frames"], Value::Null);
        assert_eq!(echoed["durationSeconds"], Value::Null);
        assert_eq!(echoed["framesSource"], "projectEnd");
    }

    #[test]
    fn frames_and_duration_seconds_resolve_to_the_same_output_length() {
        let by_frames = validate(&args(serde_json::json!({
            "format": "wav", "sampleRate": 48000, "frames": 8_400,
        })))
        .expect("合法");
        assert_eq!(by_frames.frames, Some(8_400));
        assert_eq!(by_frames.frames_source(), "frames");
        assert_eq!(by_frames.to_value()["frames"], 8_400);
        // ⛔ 秒数不是"只回显"：换算用的是**请求的**采样率。
        let by_seconds = validate(&args(serde_json::json!({
            "format": "wav", "sampleRate": 48000, "durationSeconds": 0.175,
        })))
        .expect("合法");
        assert_eq!(by_seconds.frames, Some(8_400));
        assert_eq!(by_seconds.duration_seconds(), Some(0.175));
        assert_eq!(by_seconds.frames_source(), "durationSeconds");
        assert_eq!(by_seconds.to_value()["frames"], 8_400);
        // 同给且一致 ⇒ 接受（只保留一个帧数，⛔ 不造第二个真相）。
        let both = validate(&args(serde_json::json!({
            "format": "wav",
            "sampleRate": 48000,
            "frames": 8_400,
            "durationSeconds": 0.175,
        })))
        .expect("一致");
        assert_eq!(both.frames, Some(8_400));
        assert_eq!(both.frames_source(), "durationSeconds");
        // 另一个采样率下同一段秒数换算到另一个帧数（证明用的是请求率，不是工程率）。
        let at_96k = validate(&args(serde_json::json!({
            "format": "wav", "sampleRate": 96000, "durationSeconds": 0.175,
        })))
        .expect("合法");
        assert_eq!(at_96k.frames, Some(16_800));
    }

    #[test]
    fn illegal_frame_overrides_are_refused_explicitly() {
        // `frames <= 0` / 非整数 / 非数字。
        for bad in [
            serde_json::json!({"format": "wav", "sampleRate": 48000, "frames": 0}),
            serde_json::json!({"format": "wav", "sampleRate": 48000, "frames": -1}),
            serde_json::json!({"format": "wav", "sampleRate": 48000, "frames": 1.5}),
            serde_json::json!({"format": "wav", "sampleRate": 48000, "frames": "48000"}),
        ] {
            let fault = validate(&args(bad.clone())).expect_err("坏 frames");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{bad}"
            );
            assert!(
                !format!("{fault:?}").is_empty(),
                "拒绝必须带报文（⛔ 不写哨兵）"
            );
        }
        // `durationSeconds` 非正 / 换算成 0 帧 / 非数字。
        for bad in [
            serde_json::json!({"format": "wav", "sampleRate": 48000, "durationSeconds": 0}),
            serde_json::json!({"format": "wav", "sampleRate": 48000, "durationSeconds": -0.5}),
            // 1e-9 s × 48 kHz = 4.8e-5 帧 ⇒ round 到 0 帧。
            serde_json::json!({"format": "wav", "sampleRate": 48000, "durationSeconds": 1e-9}),
            serde_json::json!({"format": "wav", "sampleRate": 48000, "durationSeconds": "1"}),
        ] {
            let fault = validate(&args(bad.clone())).expect_err("坏时长");
            assert_eq!(
                fault.domain_code(),
                Some(ErrorCode::InvalidParameterRange),
                "{bad}"
            );
        }
        // 两个都给且冲突 ⇒ 拒绝，报文带两个原值与换算值。
        let fault = validate(&args(serde_json::json!({
            "format": "wav",
            "sampleRate": 48000,
            "frames": 8_400,
            "durationSeconds": 0.5,
        })))
        .expect_err("冲突");
        assert_eq!(fault.domain_code(), Some(ErrorCode::InvalidParameterRange));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["data"]["frames"], 8_400);
        assert_eq!(value["error"]["data"]["durationSeconds"], 0.5);
        assert_eq!(value["error"]["data"]["durationFrames"], 24_000.0);
        // 上限：与工程末端推导**同一个**出口（`RENDER_FAILED` + `maxFrames`）。
        // ⚠ 这里用**字面量**而不是常量：源码级判据 `the_frame_ceiling_gate_uses_the_
        // constant_itself` 要求常量在 render.rs 里只出现三次（定义 + 比较 + 报文）。
        let fault = validate(&args(serde_json::json!({
            "format": "wav",
            "sampleRate": 48000,
            "frames": 172_800_001u64,
        })))
        .expect_err("超限");
        assert_eq!(fault.domain_code(), Some(ErrorCode::RenderFailed));
        let value = fault.into_result().expect("带内");
        assert_eq!(value["error"]["data"]["maxFrames"], 172_800_000);
        assert_eq!(value["error"]["data"]["frames"], 172_800_001);
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
    fn pan_follows_the_model_conventions() {
        // ⭐ `#268`：等程律 `pitch → Hz` 的实现已随音源一起退休（引擎的
        // `ScheduledNote::new` 在快照投影里做这一步）⇒ 本判据只保留声相。
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

    /// `#246` S6（MCP 端）：循环展开**只**调 `yeban-model` 的纯函数。
    ///
    /// 夹具与引擎侧 `crates/yeban-engine/tests/clip_loop_expansion.rs` 的 S2
    /// **逐字相同**（`D=1920 / 循环 0..480 / 音符 0..240 / 120 BPM / 48 kHz`），
    /// 因此两端断言的是同一张 tick 表与同一张帧表。
    #[test]
    fn loop_expansion_is_delegated_to_the_model() {
        let placement = yeban_model::ClipPlacement {
            duration_ticks: 1_920,
            loop_config: yeban_model::LoopConfig {
                enabled: true,
                start_tick: 0,
                end_tick: 480,
            },
            ..yeban_model::ClipPlacement::default()
        };
        assert_eq!(
            placement.loop_repeat(),
            Some(yeban_model::LoopRepeat {
                start_tick: 0,
                end_tick: 480,
                period_ticks: 480,
                iterations: 4,
            }),
            "计划由模型给（本层不再有本地 loop_plan）"
        );
        let table = placement.note_copies(0, 240);
        assert_eq!(
            table,
            vec![(0, 240), (480, 240), (960, 240), (1_440, 240)],
            "MCP 端的 tick 表"
        );
        let frames: Vec<(i64, i64)> = table
            .iter()
            .map(|(start_tick, duration)| {
                math::note_frame_span(*start_tick, *duration, 0, PPQ, 120.0, 48_000)
            })
            .collect();
        assert_eq!(
            frames,
            vec![
                (0, 6_000),
                (12_000, 18_000),
                (24_000, 30_000),
                (36_000, 42_000)
            ],
            "48 kHz / 120 BPM ⇒ 1 tick = 25 样本；与引擎侧同一张帧表"
        );
        // `(start_tick, end_tick, pitch)`：与引擎侧 `s2` 的 tick 表逐项相同。
        let ticks: Vec<(u64, u64, u8)> = table
            .iter()
            .map(|(start_tick, duration)| (*start_tick, start_tick + duration, 60))
            .collect();
        assert_eq!(
            ticks,
            vec![
                (0, 240, 60),
                (480, 720, 60),
                (960, 1_200, 60),
                (1_440, 1_680, 60)
            ],
            "MCP 与引擎同一张 (start_tick, end_tick, pitch) 表"
        );
    }

    // -----------------------------------------------------------------------
    // ⭐ `#263`：`DeviceParam` 的**序数地址**解析成稳定名字
    //
    // 交付物就是 `data.automation.tracks[].lanes[]` 的三个字段，因此判据全部走
    // `AutomationReport::from_project` + `to_value`（无头、不渲染音频）。
    // -----------------------------------------------------------------------

    use yeban_model::{
        AutomationLane, AutomationPoint, CurveType, DeviceDefinition, ParameterValue,
    };

    fn node(seed: u32) -> EntityId {
        let mut text = [b'0'; 26];
        const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
        for position in 0..4 {
            text[25 - position] = ALPHABET[((seed >> (5 * position)) & 0x1F) as usize];
        }
        core::str::from_utf8(&text)
            .expect("ASCII")
            .parse()
            .expect("合法 ULID")
    }

    fn param(name: &str, value: f32) -> ParameterValue {
        ParameterValue {
            name: name.to_owned(),
            value,
            unit: None,
        }
    }

    fn device(id: EntityId, params: &[(&str, f32)]) -> DeviceDefinition {
        DeviceDefinition {
            id,
            name: "dev".to_owned(),
            kind: DeviceKind::InternalEffect,
            bypassed: false,
            params: params
                .iter()
                .map(|(name, value)| param(name, *value))
                .collect(),
            latency_samples: 0,
        }
    }

    fn device_param(track_id: EntityId, slot_index: usize, param_index: usize) -> AutomationTarget {
        AutomationTarget::DeviceParam {
            track_id,
            slot_index,
            param_index,
        }
    }

    /// 一条轨 + 给定的设备链 + 给定的泳道（每条泳道一个点，`readEnabled: true`）。
    fn project_with(
        devices: Vec<DeviceDefinition>,
        targets: &[AutomationTarget],
    ) -> (YebanProjectV1, EntityId) {
        let track_id = node(1);
        let mut track = TrackV3 {
            id: track_id,
            name: "Lead".to_owned(),
            devices,
            ..TrackV3::default()
        };
        for (offset, target) in targets.iter().enumerate() {
            let point = AutomationPoint {
                id: node(300 + offset as u32),
                tick: 0,
                value: 0.25,
                curve: CurveType::Linear,
            };
            track.automation_lanes.insert(
                *target,
                AutomationLane {
                    target: *target,
                    points: BTreeMap::from([(point.id, point)]),
                    read_enabled: true,
                    write_mode: AutomationWriteMode::Off,
                    domain: None,
                },
            );
        }
        let mut project = YebanProjectV1 {
            master_bus_track_id: node(0xFF),
            ..YebanProjectV1::default()
        };
        project.tracks.insert(track_id, track);
        (project, track_id)
    }

    /// 报告里某条泳道的 JSON（按目标名找；同名的多条用 [`lanes_of`]）。
    fn lane_of(report: &AutomationReport, target: &str) -> Value {
        let value = report.to_value();
        let lanes = value["tracks"][0]["lanes"].as_array().expect("lanes");
        lanes
            .iter()
            .find(|lane| lane["target"] == target)
            .cloned()
            .unwrap_or_else(|| panic!("缺少 {target} 泳道: {value}"))
    }

    fn lanes_of(report: &AutomationReport) -> Vec<Value> {
        report.to_value()["tracks"][0]["lanes"]
            .as_array()
            .expect("lanes")
            .clone()
    }

    /// 判据 1：`DeviceParam { 0, 0 }` 且设备有参数 ⇒
    /// `resolvedDeviceId`（设备 `id`）＋ `resolvedParamName`（参数名）＋ `unresolved: null`。
    /// 非设备目标的同名字段**键仍在、值为 `null`**（⛔ 不静默省略）。
    #[test]
    fn a_device_param_lane_reports_the_resolved_device_and_param_name() {
        let device_id = node(101);
        let track_id = node(1);
        let (project, _) = project_with(
            vec![device(device_id, &[("input_gain_db", -3.0), ("mix", 0.5)])],
            &[
                AutomationTarget::TrackVolume { track_id },
                device_param(track_id, 0, 0),
            ],
        );
        let report = AutomationReport::from_project(&project, &[], &[], &[]);
        let lane = lane_of(&report, "DeviceParam");
        println!(
            "判据 1 读数: {}",
            serde_json::to_string(&lane).expect("JSON")
        );
        assert_eq!(
            lane["resolvedDeviceId"],
            device_id.to_canonical_string(),
            "必须是设备身份，不是下标"
        );
        assert_eq!(lane["resolvedParamName"], "input_gain_db");
        assert_eq!(lane["unresolved"], Value::Null);

        let structural = report.tracks[0]
            .lanes
            .iter()
            .find(|lane| lane.target == "DeviceParam")
            .expect("结构层");
        assert_eq!(structural.resolved_device_id, Some(device_id));
        assert_eq!(
            structural.resolved_param_name.as_deref(),
            Some("input_gain_db")
        );
        assert_eq!(structural.unresolved, None);

        let volume = lane_of(&report, "TrackVolume");
        for key in ["resolvedDeviceId", "resolvedParamName", "unresolved"] {
            assert!(
                volume.get(key).is_some(),
                "非设备目标也必须带 `{key}` 键（值 null）: {volume}"
            );
            assert_eq!(volume[key], Value::Null, "{key} 必须是 null: {volume}");
        }
    }

    /// 判据 2：`slot_index = 99` ⇒ `"slotOutOfRange"`；`param_index = 99` ⇒
    /// `"paramIndexOutOfRange"`；两者都不报设备/名字（不是"猜一个"）。
    #[test]
    fn out_of_range_device_param_lanes_report_the_reason_verbatim() {
        let track_id = node(1);
        let (project, _) = project_with(
            vec![device(node(101), &[("input_gain_db", -3.0)])],
            &[device_param(track_id, 99, 0), device_param(track_id, 0, 99)],
        );
        let report = AutomationReport::from_project(&project, &[], &[], &[]);
        let lanes = lanes_of(&report);
        println!(
            "判据 2 读数: {}",
            serde_json::to_string(&lanes).expect("JSON")
        );
        assert_eq!(lanes.len(), 2);
        let by_reason = |reason: &str| {
            lanes
                .iter()
                .find(|lane| lane["unresolved"] == reason)
                .unwrap_or_else(|| panic!("缺少 unresolved={reason}: {lanes:?}"))
                .clone()
        };
        let slot = by_reason("slotOutOfRange");
        assert_eq!(slot["resolvedDeviceId"], Value::Null);
        assert_eq!(slot["resolvedParamName"], Value::Null);
        let index = by_reason("paramIndexOutOfRange");
        assert_eq!(index["resolvedDeviceId"], Value::Null);
        assert_eq!(index["resolvedParamName"], Value::Null);
    }

    /// 判据 3：名字解析到了、但 DSP 不认 ⇒ `unresolved: "paramNotSupported"`，
    /// **名字仍然报**（⛔ 不能因为不支持就把名字也吞掉）。
    #[test]
    fn an_unsupported_param_name_is_still_reported_alongside_the_reason() {
        let device_id = node(101);
        let track_id = node(1);
        let (project, _) = project_with(
            vec![device(device_id, &[("mix", 0.5)])],
            &[device_param(track_id, 0, 0)],
        );
        let report = AutomationReport::from_project(&project, &[], &[], &[]);
        let lane = lane_of(&report, "DeviceParam");
        println!(
            "判据 3 读数: {}",
            serde_json::to_string(&lane).expect("JSON")
        );
        assert_eq!(lane["resolvedDeviceId"], device_id.to_canonical_string());
        assert_eq!(lane["resolvedParamName"], "mix", "名字必须仍然报");
        assert_eq!(lane["unresolved"], "paramNotSupported");
        // ⚠ 解析成功不等于被应用：既有 `supported` / `applied` 口径一位不变。
        assert_eq!(lane["supported"], false);
        assert_eq!(lane["applied"], false);
    }

    /// 判据 4（**本单的核心价值**）：槽位 0 插入一台新设备 ⇒ 同一个泳道的
    /// `resolvedDeviceId` **变成新设备的 id**，而 `validate_against` 仍是 `Ok(())`。
    /// 这就是"插一台设备就静默改指"的机械证据。
    #[test]
    fn inserting_a_device_at_slot_zero_silently_retargets_the_lane() {
        let old_device = node(101);
        let new_device = node(202);
        let target = device_param(node(1), 0, 0);
        let (mut project, track_id) = project_with(
            vec![device(old_device, &[("input_gain_db", -3.0)])],
            &[target],
        );
        target.validate_against(&project).expect("插设备前目标合法");
        let before = lane_of(
            &AutomationReport::from_project(&project, &[], &[], &[]),
            "DeviceParam",
        );

        // 模型自己的插入操作（不是测试里手搓 Vec 下标）。
        yeban_model::Op::InsertDevice {
            track_id,
            slot_index: 0,
            device: device(new_device, &[("input_gain_db", 0.0)]),
        }
        .apply(&mut project)
        .expect("插入设备");

        // ⚠ 静默的那一半：新下标仍在范围 ⇒ 对账入口什么都不报。
        target
            .validate_against(&project)
            .expect("插设备后 validate_against 仍然 Ok（这正是隐患）");
        let after = lane_of(
            &AutomationReport::from_project(&project, &[], &[], &[]),
            "DeviceParam",
        );
        println!(
            "判据 4 读数: before={} after={}（两次 unresolved 都是 null）",
            before["resolvedDeviceId"], after["resolvedDeviceId"]
        );
        assert_eq!(before["resolvedDeviceId"], old_device.to_canonical_string());
        assert_eq!(after["resolvedDeviceId"], new_device.to_canonical_string());
        assert_ne!(
            before["resolvedDeviceId"], after["resolvedDeviceId"],
            "插设备必须让同一序数改指 —— 否则本判据测的不是这件事"
        );
        assert_eq!(before["unresolved"], Value::Null);
        assert_eq!(after["unresolved"], Value::Null);
    }
}
