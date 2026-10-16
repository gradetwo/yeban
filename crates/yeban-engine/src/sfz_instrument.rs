//! SFZ 乐器的**适配层**（`yeban-sfz` → 引擎）。[ROAD-M2-005, ROAD-M3-004]
//!
//! `yeban-sfz` 是 SFZ v2 的**唯一实现**（零拷贝解析器、区域/曲线/力度/交叉淡化/声部池）。
//! 它的 API 是**借用式**的：
//!
//! ```text
//! pub fn parse_text<'a>(text: &'a str, limits: &ParseLimits) -> Result<Instrument<'a>, SfzError>
//! pub struct Region<'a> { … }        // 同样带生命周期
//! ```
//!
//! 于是"把 `String` 与解析结果放进**同一个结构体**"会变成**自引用结构** —— 安全 Rust 里
//! 表达不了（必须 `unsafe` 或引入 `ouroboros` 一类新依赖）。本适配层因此采用**闭包式**设计：
//! 结构体只持有**文本与元数据**，解析在**使用时**发生，生命周期被限制在闭包内。
//!
//! # ⚠ 两条必须记住的事
//!
//! ① **本类型⛔ 不在实时音频路径上**：`with_instrument` 会**重新解析**（⇒ **堆分配**），
//!    若在音频回调里调用就**违反红线 7**（实时回调内禁任何堆分配/释放、禁锁等待、禁阻塞 I/O）
//!    [MUST-GATE-001]。回调里只能读**预先摊平**的只读数据。
//! ② **实时路径的正解（下一步）**：在**构造期/离线期**把 `Instrument::regions()` 摊平成
//!    **自有数据**（定长数组/预分配 `Vec`，`f32` 键范围与力度范围直接内联），回调侧只做
//!    整数/浮点比较与索引 ⇒ 零分配、零锁。⛔ 当前**尚未**实现这一步，故本类型只用于
//!    **控制侧**（加载、校验、计数、选择区域）。
//!
//! 本模块**不新增任何所有权机制**：只包一层文本 + 元数据。
//!
//! # ✂️ 摊平表（[`PreparedSfz`]）的**口径**与已知局限
//!
//! 摊平表是实时回调唯一能读的形态。它必须与**离线**路径（[`SfzInstrument::render_note_into`]）
//! 用**同一份**口径，否则同一条 SFZ 在导出与实时两条路上会发出不同的声音。
//!
//! ## 口径（本片修正后的三条）
//!
//! 1. **区域选择**：两侧都用 `yeban-sfz` 的 `Region::matches_query` —— 即
//!    `Instrument::region_for_with` 内部那个谓词（键 ＋ 力度 ＋ 通道 ＋ `trigger` 事件 ＋
//!    keyswitch ＋ CC，严格策略：门控没有外部状态 ⇒ **不匹配**）。
//!    ⛔ 引擎**不再**自拼 `matches_key && matches_velocity`（那会静默绕过 `trigger=release`
//!    一类的门控，把 release 采样在**按下**时打出来）。
//! 2. **多匹配语义 = `all`（叠加）**：一次触发下**所有**匹配区域都发声并相加（SFZ 里每个
//!    `region` 是独立声部），⛔ 不是「只取第一条」。两侧同此。
//! 3. **增益**：按 `PlaybackSpec::total_gain()` 的乘积结构摊平成
//!    `volume × 力度 × 键轴淡化 × 力度轴淡化(in/out 分开)`，相乘顺序与离线路径**逐位相同**
//!    （见 [`PreparedSfz::render_block_into`]）。⛔ 不再冻结在 `(note, velocity) = (60, 100)`。
//!
//! ## 仍未摊平 / 仍未应用（如实登记，⛔ 不假装已有）
//!
//! | # | 局限 | 状态 |
//! | :--- | :--- | :--- |
//! | ① | 力度增益 | **已修**（步 1：每力度一张表，⛔ 不再冻结在 100） |
//! | ② | `trigger` 门控 | **已修**（步 1：走 `matches_query`；门控不过的区域掩码全 0） |
//! | ③ | `offset`/`end`（`SampleSpan`） | ⭐ **步 3 已闭合**：离线 `render_note_into`／摊平 `render_block_into`／实时声部三条路共用 [`reduce_span`] 的**一份**归约 ⇒ `start`（起点）与 `end`（终点）都**真正应用**，两侧逐样本差 ＝ 0 |
//! | ④ | 无 `group`/`off_by`/`note_polyphony`/`seq_` | 未做（语义由 `yeban-sfz` 裁、槽位与窃取由 `SynthEngine` 裁 ⇒ 属步 2） |
//! | ⑤ | 无字节预算、无流式 | 未做（PCM 整段 clone 进摊平表；无预算上限、无流式解码） |
//! | ⑥ | 同一样本引两次 | ⭐ **步 3 已修**：去重键 ＝ `region.sample_path()`、容器 ＝ `BTreeMap`（确定性 [MODEL-AST-003]）⇒ 同一路径只 materialize **一份** [`PreparedSample`]；读数见 [`PreparedSfz::sample_count`] / [`PreparedSfz::pcm_bytes`] |
//! | ⑦ | CC 轴交叉淡化 | 未做（回调侧没有 CC 状态 ⇒ 与离线 `playback_spec` 一致地**跳过**该轴） |
//! | ⑧ | ⭐ **循环**（`loop_mode` / `loop_start` / `loop_end`） | ⭐ **本单已接线**（任务 `#91`：`PreparedRegion` 承载三个字段 ＋ [`mix_span_into`] 回绕；三条路共用唯一归约 [`reduce_loop_window`]）。⚠ 沿革：本行原文是"`loop_window` / `direction` / `pan` / `off_mode` 未做（读数在 `PlaybackSpec` 里，摊平表未携带）" ⇒ 循环已做；`off_mode` 已由任务 `#94` 接线（⭐ 但**只作用于 `off_by` 关断**，见下面"独占组"一节），`direction` / `pan` **仍未做**，拆到⑩ |
//! | ⑨ | ⭐ **多 `sample=` 每区域各播自己** | ⭐ **步 3 已做**：摊平表持 `samples: Vec<PreparedSample>` ＋ [`PreparedRegion::sample_index`] 指路；⚠ 单样本重载 [`PreparedSfz::prepare`] **仍是广播**（键/力度映射对、**音色内容错** ⇒ 读数 `sample_content=broadcast(1)`） |
//! | ⑩ | `direction` / `pan` | 未做（读数在 `PlaybackSpec` 里，摊平表未携带）。⚠ 沿革：本行原文把 `off_mode` 也列在这里 —— ⭐ **任务 `#94` 已把它接线**（`PreparedRegion` 承载 ＋ `off_by` 关断期用；⛔ 作用面**不含** MIDI note-off），故本行只剩 `direction` / `pan` |
//!
//! # 引擎侧声部（任务 `#66` 的步 2：`SfzPlan` / `SfzVoicePool` / 分派次序）
//!
//! 摊平表（[`PreparedSfz`]）此前**只被测试引用**（`engine/lib.rs` 的模块地图里
//! `synth` 一节仍写"没有采样播放/SFZ"）。本片把它接进
//! [`crate::synth::SynthEngine::render_track`] 的**第三条音源支路**：
//!
//! | 件 | 归属 | 形状 |
//! | :--- | :--- | :--- |
//! | [`SfzPlan`] | 快照（控制侧投影，随快照共享） | 一条轨的摊平表（重数据：区域表 ＋ PCM 的 `Vec`） |
//! | [`SfzVoice`] / [`SfzVoicePool`] | [`crate::synth::SynthEngine`] 的轨道槽 | 定长 `[SfzVoice; 16]`，**全 `Copy`** |
//! | `render_track` 的第 3 参 | 快照 → 实时 | `sfz: Option<&PreparedSfz>`（与既有 `Option<&NoteSchedule>` **对称**） |
//!
//! ## 分派次序（**鼓机 ＞ SFZ ＞ 复音**）
//!
//! 一条轨至多一件音源（[`crate::drums`] §4.1 的既有裁决）。三件同时在场时的次序是
//! **鼓机 ＞ SFZ ＞ 复音**：`drum_note_map.is_some()` ⇒ 走鼓机（SFZ 池**被释放**）；
//! 否则 `sfz.is_some()` ⇒ 走 SFZ（⛔ 不叠加复音）；否则走复音。
//! ⚠ "鼓机 ＋ SFZ 同轨"是**引擎的取舍**、⛔ 不是模型语义 ⇒ 登记为 needs（见下表）。
//!
//! ## 终点口径（⭐ 任务 `#92` 起：**有 `ampeg_*` 时由包络接管**）
//!
//! 声部的停止条件是
//! `stop = min(end_sample, span 终点 → 引擎帧, 样本播完)`：
//!
//! * ⭐ **无 `ampeg_*`（透明包络）**：完全就是上面那条式子 —— ⛔ `ampeg_*` 一个都不读，
//!   音符**没有**包络收尾（与任务 `#92` 之前**逐位相同**，判据 `S37`/`S33`）；
//! * ⭐ **有 `ampeg_*`**：note-off（＝ `end_sample`）**不再**是硬停 ⇒ 见下面"幅度包络"一节
//!   （`loop_continuous` 例外：它**照旧**硬停，那正是它与 `loop_sustain` 的唯一分野）；
//! * ⭐ **用样本自然结束**：源位置走到 [`PreparedRegion::source_end`]
//!   （`min(span 终点, PCM 长度)`；线性插值需要 `position` 与 `position + 1` 两个样本）
//!   即退休；
//! * 音符先到 [`crate::synth::ScheduledNote::end_sample`] 也退休 ——
//!   ⭐ **但 `loop_mode=one_shot` 例外**（任务 `#91`：`one_shot` 忽略 note-off ⇒
//!   声部⛔ 不因 `end_sample` 退休）；
//! * ⭐ **循环声部（`loop_continuous` / `loop_sustain`）不由"样本播完"退休**：
//!   回绕把源位置压在窗口内（[`reduce_loop_window`] 保证窗口落在可读范围内）⇒
//!   只剩 `end_sample`（无包络）或**释放段走完**（有包络且会进释放）能终止它。
//!
//! ⚠ `span` 的**起点**（`offset`）与**终点**（`end`）**步 3 已应用**：三条路都从
//! `span.start` 起播、在 `span.end` 停（[`reduce_span`] 的唯一归约）。
//! ⭐ **循环**（任务 `#91`）**已接线**：见下面"循环"一节；
//! ⭐ **幅度包络**（任务 `#92`）**已接线**：见再下面"幅度包络"一节。
//!
//! # ⭐ 循环（任务 `#91` 的**循环那一半**；⛔ ADSR／`ampeg_*` 是下一单）
//!
//! ## ⚠ 本单是**接线**，⛔ 不是新实现
//!
//! 解析层**早就有了**：`yeban-sfz` 的 [`Region::loop_start`](yeban_sfz::instrument::Region::loop_start)／
//! `loop_end`／`loop_mode`（含旧拼写别名 `loopstart`／`loopend`）与四取值白名单，
//! 以及判据 [`Region::loop_window`](yeban_sfz::instrument::Region::loop_window)
//! （`no_loop` / `one_shot` ⇒ `None`；`loop_end <= loop_start` ⇒ `None`）。
//! 缺的只是**播放层装不下**：摊平表（[`PreparedRegion`]）此前⛔ 不携带任何循环字段
//! （旧⑧逐字："读数在 `PlaybackSpec` 里，摊平表未携带"）。本单把那三个字段搬进
//! 摊平表、接到逐样本路径上 ⇒ ⭐ **新增的是接线，⛔ 不是新的解析/判定规则**。
//!
//! ## 落地形状（三条，逐条可判）
//!
//! 1. **每区域承载**：`loop_start` / `loop_end` / `loop_mode` 进 [`PreparedRegion`]，
//!    ⛔ **不进** [`SfzVoice`]。声部只加 `region_index: u16`（落在既有尾部填充里
//!    ⇒ **+0 字节**，`S9` 仍钉 `40 B`）；循环窗口由
//!    [`PreparedRegion::loop_window`] ＋ 唯一归约 [`reduce_loop_window`] 求值。
//!    ⚠ [`SfzVoice::sample_index`] **不够**：它指到**去重后的样本**，多条区域可以共用
//!    一份 PCM 却各有各的循环窗口 ⇒ 必须能回到**区域**。
//! 2. **唯一逐样本实现**：[`mix_span_into`] 在写每个样本之前回绕
//!    （`fmod` 而非"减一次"，理由见该函数文档）⇒ 离线／摊平／实时**三条路同口径**。
//! 3. **`one_shot` 忽略 note-off**：`loop_mode=one_shot` 的区域
//!    （[`PreparedRegion::ignores_note_off`]）⛔ 不因 `end_sample`（＝ note-off）退休，
//!    只由样本自然结束退休（与 `yeban-sfz` 的 `PlaybackSpec::ignores_note_off` 同一份裁决）。
//!
//! ## ⭐ `no_loop` / `loop_sustain` / `loop_continuous` 在 note-off 后的**分野**
//! （⭐ 任务单 `#94` **按规范改写**；⛔ 旧口径已作废）
//!
//! ⚠ **沿革（⛔ 不改写历史）**：本行在循环接线片（任务 `#91`）里曾**逐字**写
//! 「note-off 后两者的差别（走完当前循环再进释放）属 ADSR 片、本片不声称已实现」
//! —— 任务 `#92` 曾按"`loop_sustain` ＝ 走完当前循环再释放；`loop_continuous` ＝ note-off
//! **不进**释放、`end_sample` 硬停"实现；⭐ **任务 `#94` 查了 `sfzformat` 的两份页面后把
//! 那两条口径都撤掉** ⇒ 现在的分野是：
//!
//! | 模式 | note-off 之后 |
//! | :--- | :--- |
//! | `no_loop` | ⭐ 进释放段；释放期**⛔ 不循环**（源位置沿途往后播） |
//! | `loop_sustain` | ⭐ 同 `no_loop`（"During the release phase, there's no looping." ＋ `legacy` 的 "The rest of the sample will play after note release."） |
//! | `loop_continuous` | ⭐ ⭐ **释放期照旧循环（可以重复多次）**（"the loop will repeat if the loop end is reached during the release phase, including repeating multiple times if the release time is longer than the loop length."） |
//!
//! 规范逐字（<https://sfzformat.com/opcodes/loop_mode/>）："Whether no_loop,
//! loop_continuous or loop_sustain is set, the duration of the release phase is set using
//! `ampeg_release`. However, in loop_sustain or no_loop mode, the sound can be cut off
//! before the release phase ends, if the end of sample is reached."
//! ⇒ ⭐ 三档**都在 note-off 就地进释放**；分野只剩"释放期要不要回绕"＋"谁先到谁结束"。
//! ⚠ 没有包络（透明）时**没有释放阶段可言** ⇒ 不在上表里：照旧 `end_sample` 硬停。
//! ⚠ 第 3 档与第 4 档的输出**必须不同**（判据 `S36` 各配一条"它自己在 note-off 后该怎样"
//! 的断言，⛔ 不是只断言"两者不同"—— 那可能两个都错）。
//!
//! ⭐ **实现落点（唯一闸门）**：[`EnvRun::may_wrap`] 是逐样本回绕的**唯一**判据
//! （[`mix_span_into`] 里那一个 `if`）⇒"释放期循环与否"⛔ 不是第二份循环几何；
//! "进释放"本身仍由 [`EnvRun::advance`] 在 `abs_frame >= note-off` 时做（三档同径）。
//!
//! ## ⭐ 退化窗口的**既有**语义（本片只登记，⛔ 不加新机制）
//!
//! `loop_end <= loop_start`（两个值**各自**合法、只有**组合**退化）⇒
//! [`PreparedRegion::loop_window`] 返回 `None` ＝ **按不循环渲染**
//! （零长度循环没有定义，⛔ 不返回 `start == end` 的死循环窗口）。
//! 这是 `yeban-sfz` 的既有裁决，本片**只沿用**。
//!
//! ## 输入政策不变（⭐ 响亮 `Err`，⛔ 不静默降级）
//!
//! 集合外的 `loop_mode` 取值与越界的 `loop_start` / `loop_end` **仍然报错**：
//! 前者走 `yeban-sfz` 的 `as_option` ⇒ `SfzError::InvalidOption`，后者走 `read_u32`
//! ⇒ `SfzError::IntegerOutOfRange`。⚠ 本片⛔ 没有把它们改成"降级 ＋ 记录"，
//! 也没有新增任何降级档（判据 `S31` 在引擎侧钉住这两条 `Err` 仍能透出）。
//!
//! # ⭐ 幅度包络（任务 `#92` 的 **ADSR 那一半**；`sfz-needs-6` 的另一半）
//!
//! ## ⭐ 曲线口径（⭐ 逐字登记，⛔ 不许含糊）
//!
//! ⭐ **本仓取线性段；参考实现的指数／对数选项不在本片范围内**（⛔ 不许假装线性等于
//! 参考实现的曲线）。参考实现的形状见 `sfzformat` 的 `ampeg_attack` 页正文：ARIA 的
//! SFZ1 包络**起音是线性**、衰减/释放**带一条比线性更快的乘性曲线**
//! （`μ = exp(−8/(t·s))`）。本片把**四段都取分段线性**：逐样本可算、确定性、可判
//! （判据 `S35` 的容差口径是「线性段上每一点都在 `|实测 − 理论| ≤ 1e-6` 内」）。
//!
//! ## 段与电平（分段线性；`A`/`H`/`D`/`R` 是**引擎帧**、`S` 是 **0..1**）
//!
//! | 段 | 区间（自触发起的帧数 `t`） | 电平 |
//! | :--- | :--- | :--- |
//! | 起音 `Attack` | `0 ≤ t < A` | `t / A` |
//! | 保持 `Hold` | `A ≤ t < A + H` | `1` |
//! | 衰减 `Decay` | `A + H ≤ t < A + H + D` | `1 − (1 − S) · (t − A − H) / D` |
//! | 保持 `Sustain` | `t ≥ A + H + D` | `S` |
//! | 释放 `Release` | 释放起点后第 `k` 帧（`0 ≤ k < R`） | `L_off · (R − k) / R` |
//!
//! * `A/H/D/R = round(秒 × engine_hz)`（**四舍五入**，⛔ 不截断）；
//! * `S = ampeg_sustain / 100`（⛔ 该 opcode 是**百分比**不是增益）；
//! * `L_off` ＝ 释放**起点那一刻**的电平（⛔ 不是"永远从 `S` 起算"：note-off 落在
//!   起音/衰减段时从**当时**的电平起算 ⇒ 连续、⛔ 不跳变）；
//! * ⭐ **零长段被跳过**（`A == 0` / `D == 0` / `R == 0` 各自合法）⇒ 这是**数学上的必然**
//!   （零长的线性段没有中间点），⛔ 不是"降级政策"。
//!
//! ## ⭐ note-off 语义（⭐ 任务单 `#94` 按规范改写；五档）
//!
//! | # | 情形 | note-off 之后 |
//! | :--- | :--- | :--- |
//! | 1 | ⭐ **透明包络**（五个值都在缺省上 ⇒ 区域**没有** `ampeg_*` 的效果） | ⭐ **一字不改**：照旧 `end_sample` 硬停（⛔ 逐样本一个浮点运算都不加） |
//! | 2 | `loop_mode=no_loop` ＋ 有包络 | ⭐ note-off **就地**进释放段；释放期⛔ 不循环 ⇒ 声部在**"释放走完"与"到样本尾"谁先到**时结束 |
//! | 3 | `loop_mode=loop_sustain` ＋ 有包络 | ⭐ 同第 2 档（note-off **之前**"按住才循环"照旧；释放期⛔ 不循环） |
//! | 4 | `loop_mode=loop_continuous` ＋ 有包络 | ⭐ note-off **就地**进释放段，且⭐ **释放期照旧循环（可以重复多次）** ⇒ 声部到**释放走完**才结束 |
//! | 5 | ⭐ `loop_mode=one_shot` ＋ 有包络 | ⭐ **连释放段也不进**（"忽略 note-off"是一条**完整**裁决：既不改退休、也不起释放）⇒ 声部照旧播到样本尾 |
//!
//! ⚠ `ampeg_release` 换算成 **0 帧**时，第 2/3/4 档都是 note-off 那一刻**立即退休**
//! （零长段跳过，见 [`AmpegRun`]）。
//! ⚠ 第 3 档与第 4 档的输出**必须不同**（判据 `S36`）。
//!
//! # ⭐ 独占组（`group` / `off_by` / `off_mode`；任务单 `#94` 接线）
//!
//! ## ⚠ 本单是**接线**，⛔ 不是新解析
//!
//! 解析层**早就有了**：`yeban-sfz` 的 [`Region::group`](yeban_sfz::instrument::Region::group)／
//! `off_by`／`off_mode`（别名 `polyphony_group`／`offby`；缺省 `0`/`0`/`fast`；
//! 域按 `read_u32` 的 `0..=u32::MAX`，集合外的 `off_mode` 走 `as_option` 报 `Err`）。
//! 缺的只是**播放层装不下**（摊平表以前⛔ 不携带这三个字段）⇒ 本单把它们搬进
//! [`PreparedRegion`] ＋ 在[`SfzVoicePool::trigger_note`] 的**触发期**接上关断。
//!
//! ## 语义（规范逐字，⭐ 三条 ＋ 一条工程裁决）
//!
//! * [`PreparedRegion::group`]（<https://sfzformat.com/opcodes/group/>）：
//!   "Exclusive group number for this region." —— **本区域属于哪个独占组**；
//! * [`PreparedRegion::off_by`]（<https://sfzformat.com/opcodes/off_by/>）：
//!   "Region off group. When a new region with a group number equal to `off_by` plays,
//!   this region will be turned off." ＋ `group` 页的镲片扼音例
//!   （"the crash sounds would be in one group, the choke samples in another"）
//!   ⇒ **本区域触发时**去关掉"当前正在发声、且其区域的 `group == 本区域的 off_by`"的声部；
//!   `off_by == 0` ＝ **一个都不关**（格式页 Practical considerations：各播放器对
//!   `group=0`/`off_by=0` 都做"not muted"的绕过 ⇒ 本仓取同一条）；
//! * [`PreparedRegion::off_mode`]（<https://sfzformat.com/opcodes/off_mode/>）：
//!   "This opcode will determinate how a region is turned off by an `off_by` opcode."
//!   ⇒ ⭐ ⭐ **它的作用面只有 `off_by` 关断**（本片按规范字面裁；⛔ **不管** MIDI note-off
//!   —— 那条路由 `loop_mode` ＋ `ampeg_*` 决定，见上面那张五档表）：
//!   * `fast`（缺省）："The voice will be turned off immediately. Release settings will
//!     not have any effect." ⇒ victim **当场硬停**（[`SfzVoice::silence`]）；
//!   * `normal`："The region will be set into release stage." ⇒ victim 进释放段
//!     （把它自己的 `end_sample` 挪到"现在"，剩下**完全**交给既有 note-off 机）；
//!   * `time`（ARIA）：⚠ **本片按 `normal` 处理**并**逐字登记为书面偏离** ——
//!     差别只剩 `<https://sfzformat.com/opcodes/off_time/>` 的延迟（缺省 6 ms），
//!     而 `off_time` 不在本片范围内（语料分布：`time` 4 次）；⛔ 不是静默降级
//!     （显式取值一律原样解析、原样带出，`S44` 钉住集合外的取值仍报 `Err`）。
//!
//! ## ⭐ 触发者**不自掐**（rgc／sfizz 一侧；⭐ 语料逼出来的正确答案）
//!
//! 登记语料 `assets/samples/karoryfer-272-merry-orks/ork_dialogue.sfz` 的写法是
//! `group=1` ＋ `off_by=1`（规范页说的"同组独占"标准写法）⇒ **若按 ARIA 的自掐解释**，
//! 新声部会被**自己**立刻掐掉 ⇒ 那 20 处会**发不出声**。本仓因此取：
//! **只有 [`SfzVoicePool::trigger_note`] 入口那一刻就 active 的槽位**才是候选 victim
//! （一个 `u16` 快照，⛔ 零分配、⛔ 不进任何结构体）⇒ 本次新建的声部永远不在候选里。
//! 判据 `S43` 把"旧声部被关掉、新声部照常发声"钉在一起（自掐 ⇒ 0 个活声部、
//! 完全不关 ⇒ 2 个，两种都被抓住）。
//!
//! ## ⭐ 其它两条边界（都有判据）
//!
//! * **`one_shot` 的 victim 不关**：`loop_mode=one_shot` 的"忽略 note-off"是一条**完整**
//!   裁决（`#92` 立、`#94` 确认）⇒ `off_by` 是**停止请求**的一种，同样被它忽略
//!   （⛔ 不造"忽略 note-off 但不忽略 `off_by`"的半裁决；判据 `S42`，并配 `no_loop` 正对照）；
//! * **硬停只动 `active` 那一个声部**：同槽的 `pending`（被窃取后等着接管的新声部）
//!   **原样保留** ⇒ 它会照常接管（判据 `S45`）。⚠ 同时把该槽的 `fade` 归
//!   [`FadeRamp::NONE`]：那个淡出属于**已经不存在**的 victim（它被硬停而不是淡出）
//!   ⇒ 留着它会让 `render_slot` 的"提升"条件永远不成立（淡出只在 victim 还 active 时
//!   推进）⇒ 挂起者会**卡死**。
//!
//! ## 分配纪律（红线 7 / [MUST-GATE-001]）
//!
//! 查找与释放触发全部在**触发期**（⛔ 不在逐样本路径）：一次 ≤ [`SFZ_VOICES_PER_TRACK`]
//! 槽的**线性扫描**、一次 `u16` 位运算 ⇒ **零堆分配、零锁等待、零阻塞 I/O**
//! （预计算全在摊平期：`group`/`off_by`/`off_mode` 已是摊平表的定长字段）。
//!
//!
//! ## 状态归属（⛔ 不把状态塞进实时对象以外的地方）
//!
//! * **每区域**的 ADSR **时间与电平**（秒 / 百分比）进 [`PreparedRegion`]
//!   （`ampeg_attack/hold/decay/sustain/release` 五个字段，另见 [`AmpegRun`] 的帧口径归约）；
//! * **每声部**只有**推进状态**：段位（`u8`）＋ 段内计数（`u32`）＝ **5 字节**，
//!   落在 [`SfzVoice`] 既有的**填充字节**里 ⇒ 尺寸 **+0 字节**（`S9` 仍钉 `40 B`）；
//! * ⭐ 释放段的**起点电平不需要另存**：段位里记着"收到 note-off 时在哪一段"，
//!   而计数**冻结**在那一刻 ⇒ `L_off` 是 `(段位, 计数)` 的纯函数；
//! * ⭐ 释放段的**进度**也不需要另存：它是 `引擎绝对帧 − epoch`，而 epoch 借用了
//!   [`SfzVoice::end_sample`] 这个**既有** `u64`（见 [`EnvRun`] 的文档）。
//!
//! ## 输入政策（⭐ 响亮 `Err`，⛔ 不静默降级；第 259 条）
//!
//! 解析层（`crates/yeban-sfz/src/instrument.rs`）对五个 opcode 的口径：
//!
//! | 输入 | 结果 |
//! | :--- | :--- |
//! | opcode **没出现** | ⭐ **缺省值**（`0/0/0/100 %/0`）——"**缺失 ≠ 非法**" |
//! | 单位后缀（`1.5s` / `250ms` / `50%`） | 认（大小写不敏感）；`ms` **优先**于 `s` 匹配 |
//! | 认不出（`abc` / `1x` / 空串 / 时间 opcode 收到 `%`） | [`SfzError::InvalidFloat`]（⛔ 不"认不出就当秒"、⛔ 不落缺省） |
//! | `NaN` / `±Inf` | [`SfzError::NonFiniteFloat`] |
//! | 负时长 | [`SfzError::InvalidDuration`] |
//! | 越界（时间 `> 100` 秒 / `sustain ∉ 0..=100`） | [`SfzError::FloatOutOfRange`] |
//!
//! ⚠ 一处**登记在案**的缺省偏离：`ampeg_release` 的规范缺省是 `0.001` 秒，**本仓取 `0`**
//! —— 依据是任务 `#92` 的裁决第 3 条（"无 `ampeg_release` ⇒ 今天的 `end_sample` 行为不变"）。
//! 完整理由见 `yeban_sfz` 的 `AMPEG_RELEASE_DEFAULT_SECONDS`（⛔ 不是静默降级：显式取值一律原样读入）。
//!
//! ### 包络这一半的分配纪律
//!
//! 包络的逐样本推进**零堆分配、零锁等待、零阻塞 I/O**：
//! [`EnvRun`] 是一个**借用式**入参（⛔ 不分配），[`AmpegRun`] 是 `Copy` 的定长参数，
//! [`AmpegRun::level_at`] 只有「一次整数→浮点转换 ＋ 一次乘加」
//! （倒数在**逐块**时算好 ⇒ 逐样本⛔ 不做除法）。
//!
//! ## 分配纪律（红线 7 / [MUST-GATE-001]）
//!
//! * [`SfzPlan::prepare`]：**允许分配**（控制线程的构造期）；
//! * **快照边界⛔ 不摊平**：`SynthEngine::begin_snapshot` **不**碰 SFZ
//!   （`arm_from_snapshot` 会分配 ⇒ 它只能在**窗口外**被调用）；
//! * 逐样本路径只读 `&PreparedSfz`、原地写 `out`：**零分配、零锁、零 I/O**；
//! * ⭐ **PCM 借用快照**：`render_track` 收的是 `&PreparedSfz`（快照里那一份），
//!   ⛔ 不 clone `Vec`、⛔ 不 `Arc` clone `Vec`；
//! * ⚠ 摊平表带着**构造期的 `engine_hz`**（`rate_per_note`）⇒ **换采样率**必须由
//!   **控制侧在窗口外重新摊平**并发布一份新快照；引擎⛔ 不在快照边界重算，
//!   **在飞的声部**保留触发时抄下的 `rate`（实测见 `tests/rt_zero_alloc.rs` 的 ㉙）；
//!   ⭐ 任务 `#92` 起 [`PreparedSfz::engine_hz`] 也**带回同一个数**（把 `ampeg_*` 的
//!   **秒**换算成帧）⇒ 包络与速率**同一份采样率事实源**，⛔ 不是第二个时钟；
//! * **退休归属不变**：SFZ 声部的退休是**池内状态翻转**（`active = false`），
//!   ⛔ 不碰快照退役队列（旧快照仍由 `crate::snapshot` 的 `RetireProducer`
//!   交回控制线程释放）。
//!
//! ## ⚠ 与摊平表 `render_block_into` 的**口径边界**（如实登记）
//!
//! `PreparedSfz::render_block_into` 是**无状态的一次性调用**（每次从源位置 `span.start` 起播，
//! 立即播完），引擎声部是**有状态**的（跨量子延续 `position`、播完即退休）。
//! ⭐ 步 3 之后**两条路的数值口径完全相同**（`span` 起点/终点、样本、速率、增益都在
//! [`reduce_span`] / `gain_at` / `rate_per_note` 上对齐）⇒ "逐样本最大绝对差 ＝ 0"覆盖
//! **整段**（不再是"只覆盖触发量子"），判据见 `tests/sfz_instrument.rs` 的 S1/S1b/S3。
//! ⚠ 唯一的**额外因子**是引擎层的**轨增益门**（`track_gain`，见 `needs-1`）：
//! 摊平表那条路**没有轨上下文** ⇒ 对账时取**单位门**（`track_gain = 1.0`，`× 1.0` 逐位不变）
//! ⇒ 上式仍逐位成立；⭐ "引擎只多乘轨增益、⛔ 不碰力度"由 S18 逐位钉住。
//! ⭐ 任务 `#92` 起**包络也在三条路上同口径**（同一个 [`ampeg_run_from`]）⇒ 有 `ampeg_*`
//! 的夹具同样满足"逐样本最大绝对差 ＝ 0"（⚠ 前提：实时侧取 `end_sample = u64::MAX`，
//! 否则实时侧会**按语义**多出释放段 —— 那不是口径分叉而是 note-off 的裁决）。
//! 除此之外唯一的差别是**状态归属**（谁记住位置、谁负责退休），⛔ 不是数值口径。
//!
//! # 任务 `#67` 的步 3：**多采样** ＋ ⭐ **绑定注入 API**（⭐ 控制侧；⛔ 不发明模型语义）
//!
//! | 件 | 形状 | 说明 |
//! | :--- | :--- | :--- |
//! | [`PreparedSample`] | 摊平表里的**一份**样本（`path` ＋ `pcm` ＋ `sample_hz`） | 去重键 ＝ `region.sample_path()`；容器 ＝ `BTreeMap` ⇒ 顺序确定 [MODEL-AST-003] |
//! | [`PreparedRegion::sample_index`] | `u16` 指路 | `NO_SAMPLE` 哨兵 ＝ 该区域**不产生采样输出** |
//! | [`PreparedSfz::prepare_with_samples`] | 控制侧构造 | 每区域**播自己**那份样本；同一路径**只 materialize 一次**；⭐ 缺样本 ⇒ **静音 ＋ 计数**（⛔ 不发明替代内容） |
//! | [`PreparedSfz::prepare`] | 单样本**广播**（兼容重载，⛔ 未删） | 所有 `sample_index = 0`；⚠ **音色内容错**（各 region 播同一份）⇒ 读数 `sample_content=broadcast(1)` |
//! | [`SfzPlan::prepare_with_samples`] | 快照投影 | 经 [`crate::snapshot::EngineSnapshot::with_sfz`] 注入 ⇒ **唯一**的公开绑定路径 |
//! | ⭐ [`SfzByteBudget`] ／ [`SfzByteUsage`] ／ [`SfzBudgetOverrun`] | ⭐ **字节预算读数**（⭐ `needs-8` 的"可判"那一半） | 占用拆成**区域表 ／ PCM ／ 路径文本**三项（[`PreparedSfz::byte_usage`]），对**声明形状反推的上界** `N` 判 `bytes ≤ N`（[`SfzByteBudget::for_layout`]）⇒ ⛔ **只报数、不拒绝、不截断、不改摊平结果**（超预算口径与归属理由见 [`PreparedSfz::budget_overrun`]） |
//! | ⭐ [`PendingSfz`] | 快照（⭐ 轻数据：只带路径） | ⭐ **模型 `TrackV3.sampler` → 快照的接线**：`from_project` 抄进 `sfz_pending` ⇒ 控制侧据此读盘／解码（引擎⛔ 不读盘）；`with_sfz` 摊平后**取代**它 |
//!
//! ⛔ **解码在控制侧**：引擎只接**已经解码**的 [`InMemorySample`]（⛔ 不引 `symphonia`/`rubato`）。
//! 调用方从 [`crate::snapshot::EngineSnapshot::sfz_pending`]（⭐ 由 `TrackV3.sampler` 投影而来）
//! 拿到 **SFZ 文件路径**、读盘后用 [`SfzInstrument::load`] 解析，
//! 再用 [`SfzInstrument::referenced_sample_paths`] 拿到"这条 SFZ 到底引用哪些路径"
//! （排序去重），再按 `crates/yeban-app/src/sfz_load.rs` 的形状（真读 PCM WAV、读不到 ⇒
//! 如实回退 ＋ 把原因写进来源文案）解码成 `&[(String, InMemorySample)]`。
//! ⭐ ⭐ **引擎侧对"没给"的路径只做一件事：出静音 ＋ 计数**（[`PreparedSfz::silent_region_count`]
//! ／[`PreparedSfz::missing_sample_paths`]），⛔ **不发明替代内容** ——
//! "用什么替代"是**控制侧**的产品决定（归属裁决见台账 `R711`；旧行为"回退到 `samples[0]`"
//! 是**内容错**，已删除 ⇒ `needs-7` 闭合）。
//!
//! ## needs 汇总（⭐ 本步闭合的标注"已闭合"，未闭合的说明卡在哪）
//!
//! | # | 缺口 | 为什么需要裁决 |
//! | :--- | :--- | :--- |
//! | sfz-needs-1 | **轨推子/静音没有乘到 SFZ 声部上** | ⭐ ⭐ **R711 已裁决、本步已闭合**：**乐器层**拥有乐器内增益（`volume_gain`／`velocity_gain[128]`／`key_crossfade`／`velocity_fade_*`／`amp_veltrack`），**引擎层**拥有轨增益（推子＝音量、静音、独奏）⇒ ⭐ **两者相乘、⛔ 不重复**。落地口径：`snapshot::project_schedules` 把**同一个** `track_gain_scale(...)` 结果写进 [`crate::synth::NoteSchedule::track_gain`]（与鼓机/复音读的 `note.gain` **同源**）⇒ [`SfzVoicePool::trigger_note`] 把它乘在 `region.gain_at(...)` **之外** ⇒ ⭐ SFZ 与鼓机**同径**（判据：`tests/sfz_instrument.rs` 的 S17／S18）|
//! | sfz-needs-2 | **池满（> 16 条区域同时匹配）只丢弃 ＋ 计数，没有窃取口径** | ⭐ ⭐ **本单已闭合**：引擎侧 SFZ 池**采纳引擎既有那一套**（`yeban_dsp::polysynth::PolySynth` 的三段键 `(层级, 触发序号, 槽位下标)`，见 [`SfzVoicePool::select_victim`]）＋ 3 ms 指数淡出（`yeban-sfz` 的 `StealFade` 形状、**同一把** `steal_fade_frames` 舵）。`yeban_sfz::voice_pool::VoicePool::select_victim` 是**它自己那一份**（`(层级, 注入电平, 序号, 下标)` ＋ 立即接管），且那个池**不被引擎使用** ⇒ ⛔ 不构成"两份实现"。落地：池满走 `steal` —— victim 在自己槽里淡出、新声部**挂起在同一槽**（`pending`）⇒ 定容池⛔ 不需要额外 slack 槽；真丢弃只剩"挂起者还没发声就被再一次换掉"（计 `dropped`，⛔ 不静默），窃取计 `stolen`（判据 S8/S19）|
//! | sfz-needs-3 | **鼓机 ＋ SFZ 同轨的取舍**（本片＝鼓机优先，SFZ 池被释放） | ⭐ ⭐ **本单已闭合（信号腿）**：裁决**维持** `鼓机 ＞ SFZ ＞ 复音`（与 [`crate::drums`] §4.1 的既有裁决一致 ⇒ ⛔ **不改成叠加**）；真正缺的是"用户看得出被忽略了" ⇒ 新增**投影读数** [`crate::synth::SynthEngine::armed_sources`] ／ [`crate::synth::SynthEngine::ignored_sources`]（三件同轨 ⇒ 武装 `3`、被忽略 `2`；判据 S20）。⚠ **模型面**那一半（"模型若禁止这种组合，本条删除"）已由任务单 `#96` 闭合：⭐ **模型层不禁止**这个组合（模型里没有"鼓机"这个概念 —— `DeviceKind` 只有四档、无鼓机档；`TrackV3.sampler` 与 `devices` 是**平行**字段）⇒ ⭐ 裁决**留在引擎**（`鼓机 ＞ SFZ ＞ 复音` ＋ `armed_sources`／`ignored_sources` 读数）；常驻判据 `crates/yeban-model/tests/track_multi_source_coexistence.rs` |
//! | sfz-needs-4 | **`span` 终点已在引擎侧应用、离线侧未应用** | ⭐ **步 3 已闭合**：离线／摊平／实时三条路共用 [`reduce_span`]，`start`＋`end` 都应用；Criterion S3 量到**逐样本差 ＝ 0**（详见 `docs/ledger/sfz-core-notes.md` §13.2） |
//! | sfz-needs-5 | **轨↔SFZ 的模型绑定不存在**（⭐ 模型面已由任务单 `#76` 补齐、引擎侧由 `#78` 接线） | ⭐ ⭐ **两侧都已闭合**：模型面 = `TrackV3.sampler: Option<SamplerBinding>`（`SamplerBinding { path }`，只留痕路径、⛔ 零运行时对象，见台账 `R731`）；引擎面 = `EngineSnapshot` 携带**待摊平项** [`PendingSfz`]（`from_project` 从 `track.sampler` 投影）⇒ ⭐ **摊平仍全在控制侧**（引擎⛔ 不读盘、⛔ 不引 `symphonia`/`rubato`）：`sfz_pending()` →（读盘／解析／解码）→ [`SfzPlan::prepare_with_samples`] → [`crate::snapshot::EngineSnapshot::with_sfz`]（后者**取代**前者）。判据：`tests/sfz_instrument.rs` 的 S21／S22／S23 |
//! | sfz-needs-6 | **循环 / ADSR / `off_mode` / `group`/`off_by`** | ⭐ ⭐ **三片已闭合**：① 循环 ＝ 任务 `#91`（`loop_start`/`loop_end`/`loop_mode` 进 [`PreparedRegion`]、[`mix_span_into`] 回绕、`one_shot` 忽略 note-off；判据 S28～S33）；② ADSR ＝ 任务 `#92`（五个 `ampeg_*` 进 [`PreparedRegion`]、逐声部只带 5 字节推进状态、[`AmpegRun`] 分段线性；判据 S34～S38）；⭐ ③ **独占组／`off_mode` ＝ 任务 `#94`**（`group`/`off_by`/`off_mode` 进 [`PreparedRegion`]（＋8 B／区域，摊平期）、触发期 ≤16 槽线性扫描、`fast` ⇒ 硬停／`normal`＋`time` ⇒ 进释放、`one_shot` victim 忽略 `off_by`、触发者不自掐、硬停保留同槽 `pending`；判据 S39～S45）。⭐ **一处书面偏离**：`off_mode=time` 按 `normal` 处理（差别只剩 `off_time` 的延迟，而 `off_time` 不在本片范围；⛔ 不是静默降级）。⛔ **仍未做**：`ampeg_*` 的**调制类**（`ampeg_*ccN` / `ampeg_vel2*` / `*_curveccN`）、`fileg_*` / `pitcheg_*`、`off_time`、`note_polyphony`、`direction`、`pan` |
//! | sfz-needs-7 | ⭐ **缺样本的引擎侧回退是"广播到 `samples[0]`"（内容错）** | ⭐ ⭐ **R711 已裁决、本步已闭合**：**引擎出静音并计数**（[`PreparedSfz::silent_region_count`] ＋ [`PreparedSfz::missing_sample_paths`]）、**合成替代内容是控制侧的决定**（想给替代样本 ⇒ 调用方显式放进提供表，或用广播重载 [`PreparedSfz::prepare`] 明确声明）。落地口径：`prepare_with_samples` 遇缺失路径 ⇒ 该区域 `sample_index = NO_SAMPLE` ⇒ 触发时**不建声部**（与 `span()==None` 同径）⇒ ⛔ **旧"广播到 `samples[0]`"已删除**（判据：`tests/sfz_instrument.rs` 的 S15）|
//! | sfz-needs-8 | ⭐ **摊平表不含样本字节预算 / 流式** | ⭐ ⭐ **部分闭合（任务单 `#84`）——"可判"那一半已闭合，"流式"那一半仍未做**：①**字节预算读数 ＋ 判据**已落地（[`SfzByteBudget`] ／ [`SfzByteUsage`] ／ [`SfzBudgetOverrun`]、[`PreparedSfz::byte_usage`] ／ [`PreparedSfz::budget_overrun`]；判据 `tests/sfz_instrument.rs` 的 S25～S27）；②**超预算口径 ＝ 引擎只报数、控制侧决定**（⛔ 不报错、⛔ 不截断、⛔ 不改摊平结果；理由见 [`PreparedSfz::budget_overrun`]）；③⚠ **仍未闭合**：`prepare_with_samples` 依旧把每个被引用的 `InMemorySample::pcm` **整段 clone** 进摊平表（去重后 N 份）⇒ **无流式、无 `Arc` 共享**，且**峰值内存不受本预算约束**（调用方手里那份已解码 PCM 本就在场） |

use std::collections::BTreeMap;

// ⭐ 窃取淡出的**帧数口径**：引擎里 PolySynth 用的就是这一个（3 ms × 采样率，
// 上限 0.5 s、下限 1 帧）⇒ SFZ 池与复音池共用**同一个**舵，⛔ 不是第二份常数。
use yeban_dsp::polysynth::steal_fade_frames_for;
use yeban_sfz::crossfade::{XfAxis, XfDirection};
use yeban_sfz::error::SfzError;
use yeban_sfz::instrument::{Instrument, LoopMode, OffMode, Region, RegionQuery};
// ⚠ 这两个是 **lib 构建**就需要的（`load` / `with_instrument` 都调用它们）⇒ ⛔ 不能放在 `#[cfg(test)]` 下，
// 否则非测试构建会报 "cannot find type `ParseLimits`"（本批实测过一次：三项门全红）。
use yeban_sfz::parser::{ParseLimits, parse_text};
use yeban_sfz::playback::{LoopWindow, RenderRates, SampleSpan};
// ⭐ 衰减底（−60 dBFS）取自 `yeban-sfz` 的 [`StealFade`](yeban_sfz::voice_pool::StealFade)：
// 引擎侧的 SFZ 窃取淡出与该库的 3 ms 指数淡出**同形状**（`FLOOR^(n/N)`），
// ⛔ 不是引擎自造的一条包络（victim **选择**口径则由引擎自己拥有，见 `SfzVoicePool::select_victim`）。
use yeban_sfz::voice_pool::STEAL_FADE_FLOOR;

/// 已加载的 SFZ 乐器：**文本 + 元数据**（⛔ 不含借用式解析结果）。
#[derive(Debug, Clone)]
pub struct SfzInstrument {
    /// 原始 SFZ 文本（自己持有 ⇒ 无生命周期参数）。
    text: String,
    /// 构造期解析得到的**区域数**（元数据，供控制侧快速查询）。
    region_count: usize,
}

impl SfzInstrument {
    /// 构造期解析一次：**校验**文本并记下元数据。
    ///
    /// ⚠ 本函数会分配（`String` 已由调用方给出；解析器内部有分配）⇒ **只允许在控制侧调用**。
    ///
    /// # Errors
    /// 文本非法（语法、超限等）时原样透出 [`SfzError`]，⛔ 不 panic。
    pub fn load(text: String, limits: &ParseLimits) -> Result<Self, SfzError> {
        let region_count = parse_text(&text, limits)?.regions().len();
        Ok(Self { text, region_count })
    }

    /// 区域数（规范里的 `region` 条目个数）。
    #[must_use]
    pub fn region_count(&self) -> usize {
        self.region_count
    }

    /// 原始文本（调用方可能需要原样回写/哈希）。
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// **用时现解析**：把借用式 [`Instrument`] 限制在闭包内 ⇒ 无需自引用结构。
    ///
    /// ⚠ **控制侧专用**（见模块头文档 ①）：本函数会重新解析 ⇒ 会分配。
    ///
    /// # Errors
    /// 若文本在此期间变得非法（正常路径不会发生），透出 [`SfzError`]。
    pub fn with_instrument<R>(&self, f: impl FnOnce(&Instrument<'_>) -> R) -> Result<R, SfzError> {
        let limits = ParseLimits::default();
        let instrument = parse_text(&self.text, &limits)?;
        Ok(f(&instrument))
    }

    /// 在给定 `note`/`velocity` 下**选中**的区域（控制侧的查询口径）。
    ///
    /// ⭐ 复用 `yeban-sfz` 的**唯一**选择谓词 `Region::matches_query`（即
    /// `Instrument::region_for_with` 内部那个）：键/力度之外还包含通道、`trigger` 事件、
    /// keyswitch 与 CC 门控（严格策略：没有外部门控状态 ⇒ 不匹配）。
    /// ⛔ 不再自拼 `matches_key && matches_velocity` —— 那会绕过门控（`trigger=release`
    /// 的 region 会进 note-on 的命中集）。
    ///
    /// # Errors
    /// 同 [`Self::with_instrument`]。
    pub fn regions_matching(&self, note: u8, velocity: u8) -> Result<Vec<usize>, SfzError> {
        self.with_instrument(|instrument| {
            let query = RegionQuery::new(note, velocity);
            instrument
                .regions()
                .iter()
                .enumerate()
                .filter(|(_, region)| region.matches_query(&query))
                .map(|(index, _)| index)
                .collect::<Vec<usize>>()
        })
    }

    /// ⭐ **这条 SFZ 到底引用哪些样本路径**（排序 ＋ 去重 ⇒ 确定性顺序）。
    ///
    /// 它是**控制侧解码**的入口：调用方拿这张表去真读 PCM WAV
    /// （形状见 `crates/yeban-app/src/sfz_load.rs:238-269`），把结果整理成
    /// `&[(String, InMemorySample)]` 再交给 [`PreparedSfz::prepare_with_samples`]。
    /// ⚠ 路径是 [`yeban_sfz::instrument::Region::sample_path`] 的原样读数（含 `default_path`
    /// 前缀、⛔ 不做任何规范化/存在性检查）。
    ///
    /// ⚠ **会分配**（重新解析 ＋ `BTreeSet`）⇒ 控制侧专用（见模块头文档 ①）。
    ///
    /// # Errors
    /// 同 [`Self::with_instrument`]。
    pub fn referenced_sample_paths(&self) -> Result<Vec<String>, SfzError> {
        self.with_instrument(|instrument| {
            let mut paths = std::collections::BTreeSet::new();
            for region in instrument.regions() {
                paths.insert(region.sample_path().into_owned());
            }
            paths.into_iter().collect::<Vec<String>>()
        })
    }
}

/// **只在内存里的样本源**（本切片的"样本源"）。
///
/// ⚠ 本仓目前**没有**采样文件读取/解码 ⇒ 样本数据必须由**调用方**提供；
/// 本类型是离线/控制侧的最小载体（构造期分配，⛔ 不用于实时回调）。
#[derive(Debug, Clone)]
pub struct InMemorySample {
    /// 单声道 PCM（归一化到 `[-1, 1]`）。
    pub pcm: Vec<f32>,
    /// 该 PCM 的原始采样率（Hz）—— 换算交给 `yeban-sfz` 的 `RenderRates`。
    pub sample_hz: f32,
}

impl InMemorySample {
    /// 造一条**极短正弦**（判据/体验用的最小样本源）：`cycles` 个周期 @ `sample_hz`。
    #[must_use]
    pub fn sine(freq_hz: f32, sample_hz: f32, frames: usize) -> Self {
        let pcm = (0..frames)
            .map(|index| {
                let phase = core::f32::consts::TAU * freq_hz * (index as f32) / sample_hz;
                phase.sin() * 0.5
            })
            .collect();
        Self { pcm, sample_hz }
    }
}

impl SfzInstrument {
    /// **离线**渲染一个音符到 `out`（`+=` 叠加），返回写入的帧数。
    ///
    /// ⭐ 复用 `yeban-sfz` 自己的口径：区域选择用 `Region::matches_query`（含 trigger /
    /// keyswitch / 通道 / CC 门控）、播放速率用 `Region::playback_rate`、总增益用
    /// `PlaybackSpec::total_gain`（⛔ 引擎不重写这些规则）。
    /// 样本数据来自调用方给的 [`InMemorySample`]（本仓尚无采样文件读取）。
    ///
    /// ⭐ **多匹配语义 = `all`**：匹配的**每一条**区域都渲染并叠加（SFZ 里每个 `region` 是
    /// 独立声部），⛔ 不是「只取 `find` 的第一条」。返回值 = 各匹配区域写入帧数的**最大值**
    /// （⛔ 不是各区域之和；所有区域共享同一份 `source.pcm`，故它即最长那条的帧数）。
    /// [`PreparedSfz::render_block_into`] 用同一口径。
    ///
    /// ⭐ **`offset`/`end` 已应用**（步 3）：源位置从 `span.start` 起、在
    /// `min(span.end, PCM 长度)` 停 —— 与摊平表/实时声部共用 [`reduce_span`] 的**一份**归约。
    /// `end=-1`（[`SampleEnd::Silent`](yeban_sfz::instrument::SampleEnd)）⇒ 该区域**不写任何帧**。
    ///
    /// ⚠ **只覆盖离线**：本函数在调用时**解析**（⇒ 分配）⇒ ⛔ 不得在音频回调里调用（红线 7）。
    /// 实时侧用 [`PreparedSfz`]（构造期摊平）。
    ///
    /// # Errors
    /// 文本非法 ⇒ [`SfzError`]；没有匹配区域 ⇒ 返回 `Ok(0)`（合法：该音符不发声）。
    pub fn render_note_into(
        &self,
        source: &InMemorySample,
        note: u8,
        velocity: u8,
        out: &mut [f32],
        engine_hz: f32,
    ) -> Result<usize, SfzError> {
        let pcm_len = source.pcm.len();
        let rates = RenderRates::new(source.sample_hz, engine_hz);
        let query = RegionQuery::new(note, velocity);
        self.with_instrument(|instrument| {
            let mut written = 0usize;
            for region in instrument.regions() {
                if !region.matches_query(&query) {
                    continue;
                }
                // ⭐ 复用 `yeban-sfz` 自己的换算：速率与总增益都来自它的 `PlaybackSpec`
                let rate = region.playback_rate(note, rates);
                let gain = region.playback_spec(note, velocity, rates).total_gain();
                if !(rate.is_finite() && rate > 0.0) || pcm_len == 0 {
                    continue;
                }
                // ⭐ `offset`/`end`：与摊平表、实时声部**同一份**归约（`None` ⇒ 不产生输出）。
                let Some((start, source_end)) = reduce_span(region.playback_span(), pcm_len) else {
                    continue;
                };
                // ⭐ 循环：与摊平表、实时声部**同一份**归约（`None` ⇒ 按不循环渲染）。
                let loop_window = reduce_loop_window(
                    region.effective_loop_mode(),
                    region.loop_start,
                    region.loop_end,
                    source_end,
                    pcm_len,
                );
                let mut position = start;
                // 离线/摊平这条路**不窃取**：淡出状态恒为 [`FadeRamp::NONE`]
                // （`gain == 1.0` ⇒ 那个逐帧乘法逐位不变）。
                let mut fade = FadeRamp::NONE;
                // ⭐ 包络（任务 `#92`）：与摊平/实时两条路**同一份归约**（[`ampeg_run_from`]）。
                // 这条路**没有 note-off** ⇒ 只走 起音→保持→衰减→保持
                // （`frame = u64::MAX`：永远不会进释放段）；
                // ⚠ `abs_base = 0` 只用于释放进度，这条路上用不到。
                // ⚠ `frame = u64::MAX`（这条路没有 note-off ⇒ 永不进释放段）、
                // `abs_base = 0`（释放进度用不到）、`loop_during_release = false`。
                let mut env = ampeg_run_from(
                    region.ampeg_attack,
                    region.ampeg_hold,
                    region.ampeg_decay,
                    region.ampeg_sustain,
                    region.ampeg_release,
                    engine_hz,
                )
                .map(|run| EnvRun::new(run, ENV_STAGE_IDLE, 0, u64::MAX, 0, false));
                let region_written = mix_span_into(
                    &source.pcm,
                    &mut position,
                    rate,
                    gain,
                    source_end,
                    loop_window,
                    &mut fade,
                    &mut env,
                    out,
                );
                written = written.max(region_written);
            }
            written
        })
    }
}

/// ⭐ **`span`（`offset`/`end`）归约的唯一实现**：离线 [`SfzInstrument::render_note_into`]、
/// 摊平 [`PreparedSfz::render_block_into`] 与实时声部（[`SfzVoicePool`]）三条路共用它
/// ⇒ 两侧口径不会分叉（本仓"⛔ 不许两份实现"的机械纪律）。
///
/// 返回**半开区间** `[start, end)`（单位：源采样帧）的起点（`f32`，供逐样本位置使用）
/// 与终点（`u64`）：
///
/// * `span == None`（`end=-1`，或显式区间落在 `offset` 之前）⇒ `None` ＝ **不产生采样输出**；
/// * `span.end == None` ⇒ 终点取 `pcm_len`（"播到采样末尾"）；
/// * 显式 `end` 超出 PCM ⇒ 夹到 `pcm_len`（⛔ 不越界读）；
/// * 归约后**空区间**（`end <= start`，例如 `offset` 落在 PCM 之外）⇒ `None`
///   （⛔ 不是"从 0 播一整份"）。
///
/// ⚠ `start as f32`：逐样本位置本来就是 `f32`（`mix_span_into` 的口径），
/// 超过 2²⁴ 帧的 `offset` 会有位级舍入 —— 如实登记，⛔ 不假装是精确整数。
///
/// ⚠ **零分配、零锁、零 I/O**（纯整数/浮点比较）⇒ 可被实时声部在**触发期**调用 [MUST-GATE-001]。
#[must_use]
pub fn reduce_span(span: Option<SampleSpan>, pcm_len: usize) -> Option<(f32, u64)> {
    let span = span?;
    let pcm_len = pcm_len as u64;
    let end = span.end.unwrap_or(pcm_len).min(pcm_len);
    if end <= span.start {
        return None;
    }
    Some((span.start as f32, end))
}

/// ⭐ **循环窗口归约的唯一实现**（本单新增）：离线 [`SfzInstrument::render_note_into`]、
/// 摊平 [`PreparedSfz::render_block_into`] 与实时声部（[`SfzVoicePool`]）三条路共用它
/// ⇒ 与 [`reduce_span`] 同一形态，循环口径不可能在三条路之间分叉。
///
/// # 判定次序（每条都只回答"能不能按这个窗口循环"）
///
/// 1. **模式**：只有 `loop_continuous` / `loop_sustain` 会循环。`no_loop` / `one_shot`
///    恒 `None`（即使文件写了 `loop_start` / `loop_end`）——与
///    [`Region::loop_window`](yeban_sfz::instrument::Region::loop_window) 同一条口径；
/// 2. **退化窗口**：`loop_end <= loop_start` ⇒ `None` ＝ **按不循环渲染**
///    （零长度循环没有定义，⛔ 不返回 `start == end` 的死循环窗口）。这是 `yeban-sfz`
///    的**既有**裁决，本函数只**沿用**它，⛔ 不新增机制、⛔ 不报错
///    （两个值**各自**合法，只有组合退化）；
/// 3. **越界**：窗口终点超出**源采样区间**（[`reduce_span`] 的归约结果）或超出 PCM
///    可读范围 ⇒ `None`。⚠ 线性插值要读 `index + 1`
///    （见 [`mix_span_into`]）⇒ 窗口终点必须**严格小于** `pcm_len`，否则回绕后会读到
///    PCM 之外的 `pcm[loop_end]`（⛔ 越界读、⛔ panic 都不允许）；
/// 4. **合法窗口**：半开区间 `[loop_start, loop_end)`，⛔ 不夹、⛔ 不改写。
///
/// ⚠ **零分配、零锁、零 I/O**（纯整数比较）⇒ 可被实时声部在**逐块**路径调用 [MUST-GATE-001]。
#[must_use]
pub fn reduce_loop_window(
    loop_mode: LoopMode,
    loop_start: u32,
    loop_end: u32,
    source_end: u64,
    pcm_len: usize,
) -> Option<LoopWindow> {
    if !matches!(loop_mode, LoopMode::LoopContinuous | LoopMode::LoopSustain) {
        return None;
    }
    if loop_end <= loop_start {
        return None;
    }
    if u64::from(loop_end) > source_end {
        return None;
    }
    if loop_end as usize >= pcm_len {
        return None;
    }
    Some(LoopWindow {
        start: loop_start,
        end: loop_end,
    })
}

/// ✂️ **摊平后的一个区域**：回调侧**只读**（⛔ 无 `Vec`／`Box`／`String`，全部定长）。
///
/// ⭐ **幅度包络的段位**（`ampeg_*` 的逐声部推进状态的公开读数；任务单 `#92`）。
///
/// 只有**五个**取值：四段 ＋ 一个"释放态"。⚠ "从哪一段进释放"**不是**本枚举的取值
/// （它是内部编码的低位），因为对调用方而言"释放中"就是释放中。
///
/// ⚠ 沿革（⛔ 不改写历史）：任务单 `#92` 曾有一个第六取值 `ReleasePending`
/// （"等当前循环走完"）。任务单 `#94` 按 `sfzformat` 的两份页面**删掉了它** ——
/// 规范逐字只说"note-off 那一刻进释放段"（`ampeg_release` 决定时长），
/// "走完当前循环再释放"**没有**出处；释放期要不要继续回绕由 `loop_mode` 决定
/// （见 [`EnvRun::may_wrap`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmpegStage {
    /// 起音段：线性上升 `0 → 1`（时长 ＝ `ampeg_attack`）。
    Attack,
    /// 保持段：恒定 `1`（时长 ＝ `ampeg_hold`）。
    Hold,
    /// 衰减段：线性下降 `1 → ampeg_sustain`（时长 ＝ `ampeg_decay`）。
    Decay,
    /// 保持段：恒定 `ampeg_sustain`（直到 note-off）。
    Sustain,
    /// 释放段：从释放起点电平线性降到 `0`（时长 ＝ `ampeg_release`）。
    Release,
}

// ---------------------------------------------------------------------------
// ⭐ 段位编码（`SfzVoice::env_stage` 的取值）：⛔ 别处不许重复这套常量
// ---------------------------------------------------------------------------

/// 未启用（无包络的区域；`SfzVoice::envelope_stage()` 对它返回 `None`）。
const ENV_STAGE_IDLE: u8 = 0;
const ENV_STAGE_ATTACK: u8 = 1;
const ENV_STAGE_HOLD: u8 = 2;
const ENV_STAGE_DECAY: u8 = 3;
const ENV_STAGE_SUSTAIN: u8 = 4;
/// 「释放中」位：`段位 = ENV_RELEASING | (收到 note-off 时所在的段)`。
const ENV_RELEASING: u8 = 0x80;
/// 段位（低 3 位）的掩码。
const ENV_STAGE_MASK: u8 = 0x07;

/// 把 `t`（自触发起的帧数）落在哪一段编码成低位段位。
fn env_held_stage_byte(run: &AmpegRun, t: u32) -> u8 {
    if t < run.attack_frames() {
        ENV_STAGE_ATTACK
    } else if t < run.hold_end {
        ENV_STAGE_HOLD
    } else if t < run.decay_end {
        ENV_STAGE_DECAY
    } else {
        ENV_STAGE_SUSTAIN
    }
}

/// 段位编码 ⇒ 公开枚举（`0` ＝ 未启用 ⇒ `None`）。
const fn env_stage_of(byte: u8) -> Option<AmpegStage> {
    if byte == ENV_STAGE_IDLE {
        return None;
    }
    if byte & ENV_RELEASING != 0 {
        return Some(AmpegStage::Release);
    }
    match byte & ENV_STAGE_MASK {
        ENV_STAGE_ATTACK => Some(AmpegStage::Attack),
        ENV_STAGE_HOLD => Some(AmpegStage::Hold),
        ENV_STAGE_DECAY => Some(AmpegStage::Decay),
        ENV_STAGE_SUSTAIN => Some(AmpegStage::Sustain),
        _ => None,
    }
}

/// ⭐ **引擎帧口径**的分段线性幅度包络（`ampeg_*`；任务单 `#92`）。
///
/// 由 [`PreparedRegion::ampeg_run`] 在**逐块**路径上求得（⛔ 逐样本不求值）：
/// * 四个时间是**帧**（`round(秒 × engine_hz)`）；
/// * [`Self::sustain_level`] 是 **0..1 的线性增益**（规范里的百分比 ÷ 100）。
///
/// ⚠ **逐样本只用一次乘加**（`t × 倒数` / `1 − span × (t × 倒数)`）⇒ 预先存好倒数，
/// 回调里⛔ 不做除法、⛔ 不做整数→浮点的重复转换以外的开销。
///
/// ⚠ 本类型**不带任何逐声部状态**：段位与段内计数在 [`SfzVoice`]（⛔ 不在按区域的参数里）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AmpegRun {
    /// 起音段终点（帧；`0` ＝ 零长段 ⇒ 跳过）。
    attack_end: u32,
    /// 保持段终点（帧；＝ 起音 ＋ 保持）。
    hold_end: u32,
    /// 衰减段终点（帧；＝ 起音 ＋ 保持 ＋ 衰减）。
    decay_end: u32,
    /// `1 / attack_end`（`attack_end == 0` 时不会被读到）。
    attack_inv: f32,
    /// `1 / 衰减段帧数`（零长时不会被读到）。
    decay_inv: f32,
    /// `1 − sustain_level`（衰减段的落差）。
    decay_span: f32,
    /// 保持电平（0..1 的线性增益）。
    sustain: f32,
    /// 释放段帧数（`0` ＝ 零长段 ⇒ 跳到 0）。
    release_frames: u32,
    /// `1 / release_frames`（零长时不会被读到）。
    release_inv: f32,
}

impl AmpegRun {
    /// 起音段帧数（`ampeg_attack × engine_hz`，四舍五入）。
    #[must_use]
    pub const fn attack_frames(&self) -> u32 {
        self.attack_end
    }

    /// 保持段帧数。
    #[must_use]
    pub const fn hold_frames(&self) -> u32 {
        self.hold_end - self.attack_end
    }

    /// 衰减段帧数。
    #[must_use]
    pub const fn decay_frames(&self) -> u32 {
        self.decay_end - self.hold_end
    }

    /// 保持电平（0..1 的线性增益；规范里的百分比 ÷ 100）。
    #[must_use]
    pub const fn sustain_level(&self) -> f32 {
        self.sustain
    }

    /// 释放段帧数。
    #[must_use]
    pub const fn release_frames(&self) -> u32 {
        self.release_frames
    }

    /// ⭐ **未释放阶段的电平**：给定段位与段内计数（`t` ＝ 自触发起的帧数）。
    ///
    /// 这是**唯一**的包络求值处：三种状态（保持 / 等回绕 / 释放）都从这里出发
    /// （释放态把它的结果再乘一个 `(R − k) / R`）。
    ///
    /// ⚠ 零长段在这里**自然跳过**（例：`decay == 0` ⇒ `t < decay_end` 与
    /// `t < hold_end` 同时为假 ⇒ 直接落到 `sustain`）—— 这是**数学上的必然**，
    /// ⛔ 不是"降级政策"（见模块头）。
    fn level_at(&self, stage_byte: u8, t: u32) -> f32 {
        match stage_byte & ENV_STAGE_MASK {
            ENV_STAGE_ATTACK => t as f32 * self.attack_inv,
            ENV_STAGE_HOLD => 1.0,
            ENV_STAGE_DECAY => {
                1.0 - self.decay_span * ((t - self.hold_end) as f32 * self.decay_inv)
            }
            _ => self.sustain,
        }
    }

    /// ⭐ **释放段的缩放**：`(R − k) / R`（`k` ＝ 释放已走的帧数）。
    ///
    /// `k >= R` ⇒ 返回 `None`（释放走完 ⇒ 声部退休）。
    fn release_scale(&self, k: u64) -> Option<f32> {
        if self.release_frames == 0 || k >= u64::from(self.release_frames) {
            return None;
        }
        let remaining = (self.release_frames - k as u32) as f32;
        Some(remaining * self.release_inv)
    }
}

/// ⭐ **包络归约的唯一实现**（任务单 `#92`）：把 `ampeg_*` 的**秒 / 百分比**
/// 换算成引擎帧口径的 [`AmpegRun`]。
///
/// 三条路（离线 [`SfzInstrument::render_note_into`] ／摊平
/// [`PreparedSfz::render_block_into`] ／实时 [`SfzVoicePool`]）**共用它**
/// ⇒ 包络口径不可能在三条路之间分叉（与 [`reduce_span`] / [`reduce_loop_window`] 同形）。
///
/// # 判定次序（逐条可判）
///
/// 1. **透明**：五个值都恰好在透明缺省（各段 `0` 秒、`sustain` 100 %、`release` 0 秒）
///    ⇒ `None` ＝ **逐样本路径一个浮点运算都不加**（"没有 `ampeg_*`" ⇒ 与接线前逐位相同）；
/// 2. **换算**：`frames = round(秒 × engine_hz)`（⛔ 不是截断：`0.01 s @ 48 kHz` 截断会得到
///    479 而不是 480）；
/// 3. **非有限 / 负的 `engine_hz`**：换算结果按 `0` 计（调用方给不出正常采样率时
///    ⛔ 不 panic、⛔ 不产出 `NaN` —— 与 `crate::synth` 的 `sanitise_sample_rate` 同一精神）；
/// 4. 换算后**为 0 帧**的段 ＝ 零长段，由 [`AmpegRun::level_at`] 自然跳过。
#[must_use]
pub fn ampeg_run_from(
    attack_seconds: f32,
    hold_seconds: f32,
    decay_seconds: f32,
    sustain_percent: f32,
    release_seconds: f32,
    engine_hz: f32,
) -> Option<AmpegRun> {
    if attack_seconds == 0.0
        && hold_seconds == 0.0
        && decay_seconds == 0.0
        && sustain_percent == 100.0
        && release_seconds == 0.0
    {
        return None;
    }
    let frames = |seconds: f32| -> u32 {
        let raw = f64::from(seconds) * f64::from(engine_hz);
        if !raw.is_finite() || raw <= 0.0 {
            0
        } else if raw >= f64::from(u32::MAX) {
            u32::MAX
        } else {
            raw.round() as u32
        }
    };
    let attack = frames(attack_seconds);
    let hold = frames(hold_seconds);
    let decay = frames(decay_seconds);
    let release = frames(release_seconds);
    let sustain = (sustain_percent / 100.0).clamp(0.0, 1.0);
    let hold_end = attack.saturating_add(hold);
    Some(AmpegRun {
        attack_end: attack,
        hold_end,
        decay_end: hold_end.saturating_add(decay),
        attack_inv: if attack == 0 {
            0.0
        } else {
            1.0 / attack as f32
        },
        decay_inv: if decay == 0 { 0.0 } else { 1.0 / decay as f32 },
        decay_span: 1.0 - sustain,
        sustain,
        release_frames: release,
        release_inv: if release == 0 {
            0.0
        } else {
            1.0 / release as f32
        },
    })
}

/// 键/力度的可播性、以及**全部静态门控**（通道 / `trigger` 事件 / keyswitch / CC）
/// 在**构造期**由 `yeban-sfz` 的 `Region::matches_query` 求值后**打包成 128 位掩码** ⇒
/// 回调侧只做位测试，⛔ 不在回调里调用任何解析/分配函数。
/// 门控未通过的区域**两张掩码都清 0** ⇒ 它**永远不会**进回调的命中集
/// （含 `trigger=release` 一类的 region）。
///
/// 增益按 `PlaybackSpec::total_gain()` 的乘积结构拆成四份（`volume`／力度／键轴淡化／
/// 力度轴淡化），使回调侧的相乘顺序与离线路径**逐位相同**：
/// `(volume × 力度) × ((键轴 × 力度淡入) × 力度淡出)`。
/// 因此 `(note, velocity)` 的取值**不再冻结**在 `(60, 100)`。
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedRegion {
    /// 键掩码：第 `n` 位为 1 表示该区域接受 `note == n`（门控不过时全 0）。
    key_mask: [u64; 2],
    /// 力度掩码：第 `v` 位为 1 表示该区域接受 `velocity == v`（门控不过时全 0）。
    vel_mask: [u64; 2],
    /// 每个音符的播放速率（构造期由 `Region::playback_rate` 求值）。
    rate_per_note: [f32; 128],
    /// 键轴（`xf*_*key`）交叉淡化的每音符因子；无该轴时恒 `1.0`。
    key_crossfade: [f32; 128],
    /// 力度轴**淡入**（`xfin_*vel`）段的每力度因子；无该段时恒 `1.0`。
    velocity_fade_in: [f32; 128],
    /// 力度轴**淡出**（`xfout_*vel`）段的每力度因子；无该段时恒 `1.0`。
    ///
    /// 与 [`Self::velocity_fade_in`] **分开**存：`Region::crossfade_gain` 是一条
    /// 从左到右的乘法链，`(K * 淡入) * 淡出` 与 `K * (淡入 * 淡出)` 在 binary32 下
    /// 不一定同值 ⇒ 分开存才能逐位还原该链的结合序。
    velocity_fade_out: [f32; 128],
    /// `PlaybackSpec::gain`：`volume` 换算出的线性增益（与键/力度无关）。
    volume_gain: f32,
    /// `PlaybackSpec::velocity_gain` 的**每力度**读数（⛔ 不再冻结在力度 100）。
    velocity_gain: [f32; 128],
    /// ⭐ **该区域用哪一份摊平样本**（[`PreparedSfz::samples`] 的下标）。
    ///
    /// [`Self::NO_SAMPLE`] ＝ 该区域**不产生采样输出**（调用方没给它的 `sample_path`）
    /// ⇒ 渲染与建声部时整条跳过。
    sample_index: u16,
    /// ⭐ `offset` / `end` 归约出的**源采样区间**（半开 `[start, end)`，单位源采样帧）。
    ///
    /// `None` 表示该区域**不产生采样输出**（`end=-1`，或显式区间落在 `offset` 之前）。
    /// ⭐ **步 3 起渲染真正应用它**（[`reduce_span`]）：起点从 `span.start` 开始、
    /// 终点停在 `min(span.end, PCM 长度)` ⇒ 离线/摊平/实时三条路同口径。
    span: Option<SampleSpan>,
    /// ⭐ **循环起点**（`loop_start`，源采样点；原样带出，⛔ 不夹、⛔ 不回填）。
    ///
    /// 本片（循环接线片）把 `yeban-sfz` 已经解析好的三个循环字段搬进摊平表
    /// ⇒ 实时回调**不再**只有 `PlaybackSpec` 里那份读数。生效归约见
    /// [`Self::loop_window`]／[`reduce_loop_window`]。
    loop_start: u32,
    /// ⭐ **循环终点**（`loop_end`，源采样点；半开区间的**不含**端点）。
    loop_end: u32,
    /// ⭐ **生效循环模式**（`Region::effective_loop_mode`：`trigger=release` /
    /// `release_key` 家族已被覆写成 [`LoopMode::OneShot`]）。
    ///
    /// ⚠ 沿革：循环接线片（任务 `#91`）曾在此逐字登记"`loop_sustain` 的取值**原样保留**，
    /// 但它在 note-off 之后的语义与 `loop_continuous` **相同**"。⭐ 任务 `#92`（ADSR 片）
    /// 已把那条差别**变成现实** —— 见模块头的"⭐ 幅度包络"一节与
    /// [`Self::ampeg_release`] 的说明。
    loop_mode: LoopMode,
    /// ⭐ **起音时长**（`ampeg_attack`，**秒**；规范 Range 0..=100）。
    ///
    /// ⚠ 单位与口径：本摊平表**原样承载 opcode 的秒 / 百分比**（⛔ 不预先换算成引擎帧）——
    /// 换算在**逐块**读该区域时用摊平期的采样率做（见 [`Self::ampeg_run`]），
    /// 与 [`Self::rate_per_note`] 同一份"摊平期采样率"事实源。
    ampeg_attack: f32,
    /// ⭐ **保持段时长**（`ampeg_hold`，**秒**；规范 Range 0..=100）。
    ampeg_hold: f32,
    /// ⭐ **衰减段时长**（`ampeg_decay`，**秒**；从峰值降到 [`Self::ampeg_sustain`]）。
    ampeg_decay: f32,
    /// ⭐ **保持电平**（`ampeg_sustain`，**百分比** 0..=100；规范缺省 100）。
    ampeg_sustain: f32,
    /// ⭐ **释放段时长**（`ampeg_release`，**秒**；⚠ 本仓缺省 `0`，见 `yeban_sfz` 的
    /// `AMPEG_RELEASE_DEFAULT_SECONDS`）。
    ///
    /// ⚠ 与 `one_shot` 的分工：`one_shot` ⇒ note-off **根本不触发**释放段（声部继续到样本尾）；
    /// 本字段只在 note-off **真的**到达时才起作用（且 `release == 0` ⇒ 释放段零长 ⇒ 立即退休，
    /// 即"今天的 `end_sample` 硬停"）。
    ampeg_release: f32,
    /// ⭐ **独占组号**（`group`，缺省 `0`；别名 `polyphony_group`）。
    ///
    /// 语义（SFZ v1 逐字，见 <https://sfzformat.com/opcodes/group/>）：
    /// "Exclusive group number for this region." —— 它标的是**本区域属于哪个独占组**，
    /// 被别的区域的 `off_by` 用来**指名关断**（本片：任务单 `#94`）。
    ///
    /// ⚠ 同一个 `group` 还是下一片 `note_polyphony` 的检查键（`yeban-sfz` 的
    /// [`yeban_sfz::instrument::NotePolyphony`] 已经把"无 group 时按 `group=0`"写进文档）
    /// ⇒ 本字段**一片两用**，⛔ 不是两个字段。
    group: u32,
    /// ⭐ **被谁关掉**（`off_by`，缺省 `0` ＝ **不关任何区域**；别名 `offby`）。
    ///
    /// 语义（SFZ v1 逐字，见 <https://sfzformat.com/opcodes/off_by/>）：
    /// "Region off group. When a new region with a group number equal to `off_by` plays,
    /// this region will be turned off." ＋ 格式页的镲片扼音例（`group` 页）把**触发侧**写成
    /// `off_by`、**被关侧**写成 `group` ⇒ 本仓裁决：**本区域触发时**去关掉"当前正在发声、
    /// 且其区域的 `group == 本区域的 off_by`"的声部（见 [`SfzVoicePool::trigger_note`]）。
    ///
    /// ⚠ `0` ＝ 关闭该功能：`read_u32` 的缺省就是 `0`，且格式页的 Practical considerations
    /// 逐字记着各播放器对 `group=0` / `off_by=0` 的绕过（"regions are not muted"）
    /// ⇒ 本仓取同一条：**`off_by == 0` 永不关断别人**。
    off_by: u32,
    /// ⭐ **关断模式**（`off_mode`，缺省 [`OffMode::Fast`]）。
    ///
    /// 格式页逐字（<https://sfzformat.com/opcodes/off_mode/>）：
    /// "This opcode will determinate how a region is turned off by an `off_by` opcode."
    /// * `fast`（缺省）："The voice will be turned off immediately. Release settings will
    ///   not have any effect." ⇒ 被关的声部**当场硬停**；
    /// * `normal`："The region will be set into release stage." ⇒ 被关的声部**进释放段**
    ///   （它自己的 `ampeg_*`／`loop_sustain` 规则照旧生效）；
    /// * `time`（ARIA）：本片**按 `normal` 处理**并逐字登记为书面偏离 —— 差别只剩
    ///   [`yeban_sfz::instrument::Region::off_time`] 的延迟（缺省 6 ms），而 `off_time`
    ///   不在本片范围内；见模块头的"off_by 关断"一节。
    off_mode: OffMode,
}

impl PreparedRegion {
    /// ⭐ 「没有样本」哨兵（[`Self::sample_index`] 的取值）：该区域**不产生采样输出**。
    ///
    /// 出现条件只有一个：调用方给的样本表里**没有**该区域的 `sample_path`
    /// ⇒ 如实标静音 ＋ 计数（⛔ 不静默、⛔ 不 panic、⛔ **不去播别人的采样**）。
    /// ⚠ 归属裁决（台账 R711）：引擎出静音，**合成替代内容是控制侧的决定**。
    pub const NO_SAMPLE: u16 = u16::MAX;

    /// 该区域是否接受 `(note, velocity)`（⭐ 纯位测试：无分配、无解析）。
    #[must_use]
    pub fn accepts(&self, note: u8, velocity: u8) -> bool {
        let n = note as usize;
        let v = velocity as usize;
        self.key_mask[n / 64] & (1u64 << (n % 64)) != 0
            && self.vel_mask[v / 64] & (1u64 << (v % 64)) != 0
    }

    /// 键掩码的**只读副本**（判据用：`trigger=release` 的区域必须为 `[0, 0]`）。
    #[must_use]
    pub fn key_mask(&self) -> [u64; 2] {
        self.key_mask
    }

    /// 力度掩码的**只读副本**（判据用）。
    #[must_use]
    pub fn velocity_mask(&self) -> [u64; 2] {
        self.vel_mask
    }

    /// ⭐ 该区域的**源采样区间**（`offset`/`end` 归约；`None` ＝ 不产生采样输出）。
    #[must_use]
    pub fn span(&self) -> Option<SampleSpan> {
        self.span
    }

    /// ⭐ 该区域**用哪一份摊平样本**（[`PreparedSfz::samples`] 的下标）。
    #[must_use]
    pub const fn sample_index(&self) -> u16 {
        self.sample_index
    }

    /// ⭐ 该区域**有没有**样本可播（`false` ＝ [`Self::NO_SAMPLE`]）。
    #[must_use]
    pub const fn has_sample(&self) -> bool {
        self.sample_index != Self::NO_SAMPLE
    }

    /// ⭐ **`span` 的归约结果**：`(源起点, 源终点)`（半开）；`None` ＝ 不产生采样输出。
    ///
    /// 直接转交 [`reduce_span`]（⭐ **唯一**归约）⇒ 离线/摊平/实时三条路不可能分叉。
    #[must_use]
    pub fn span_bounds(&self, pcm_len: usize) -> Option<(f32, u64)> {
        reduce_span(self.span, pcm_len)
    }

    /// ⭐ **源采样起点**（`offset`，已归约）；`None` ＝ 不产生采样输出。
    #[must_use]
    pub fn source_start(&self, pcm_len: usize) -> Option<f32> {
        self.span_bounds(pcm_len).map(|(start, _)| start)
    }

    /// ⭐ **源采样终点（半开）**：`min(span 终点, 样本播完)`；`None` ＝ 不产生采样输出。
    ///
    /// * [`Self::span`] 为 `None` ⇒ `None`（`end=-1` / 空区间）⇒ 实时侧⛔ 不为它建声部；
    /// * `span.end` 未指定（`offset`/`end` 都没写）⇒ 取 `pcm_len`（"到采样末尾"）；
    /// * 显式 `end` 超出 PCM ⇒ 夹到 `pcm_len`（⛔ 不越界读）；
    /// * 归约后空区间（`end <= start`）⇒ `None`（⛔ 不是"从 0 播"）。
    #[must_use]
    pub fn source_end(&self, pcm_len: usize) -> Option<u64> {
        self.span_bounds(pcm_len).map(|(_, end)| end)
    }

    /// ⭐ 该区域的**循环起点**（`loop_start`，原样）。
    #[must_use]
    pub const fn loop_start(&self) -> u32 {
        self.loop_start
    }

    /// ⭐ 该区域的**循环终点**（`loop_end`，原样）。
    #[must_use]
    pub const fn loop_end(&self) -> u32 {
        self.loop_end
    }

    /// ⭐ 该区域的**生效循环模式**（原样，除 `trigger=release` 家族被覆写成 `one_shot`）。
    #[must_use]
    pub const fn loop_mode(&self) -> LoopMode {
        self.loop_mode
    }

    /// ⭐ **有效循环窗口**（本单的接线点）：半开 `[start, end)`；`None` ＝ 按不循环渲染。
    ///
    /// 直接转交 [`reduce_loop_window`]（⭐ **唯一**归约），把存储的三个字段与
    /// [`Self::span_bounds`] 的终点、PCM 长度折成一个判断 ⇒ 与离线/摊平两条路同口径。
    #[must_use]
    pub fn loop_window(&self, pcm_len: usize) -> Option<LoopWindow> {
        let (_, source_end) = self.span_bounds(pcm_len)?;
        reduce_loop_window(
            self.loop_mode,
            self.loop_start,
            self.loop_end,
            source_end,
            pcm_len,
        )
    }

    /// ⭐ 该区域是否**忽略 note-off**（`loop_mode=one_shot`，鼓组常用）。
    ///
    /// 与 [`yeban_sfz::playback::PlaybackSpec::ignores_note_off`] 的**同一份**裁决
    /// （那边是 `self.loop_mode == LoopMode::OneShot`）：本片把它接进实时声部
    /// ⇒ `end_sample` 对 `one_shot` 声部**不生效**（声部只由样本自然结束退休）。
    #[must_use]
    pub const fn ignores_note_off(&self) -> bool {
        matches!(self.loop_mode, LoopMode::OneShot)
    }

    // -----------------------------------------------------------------
    // ⭐ 独占组（`group` / `off_by` / `off_mode`；任务单 `#94`）的只读读数
    // -----------------------------------------------------------------

    /// ⭐ 本区域**属于哪个独占组**（`group`，原样；缺省 `0`）。
    ///
    /// 用途有两个（⭐ 同一份数据、⛔ 不是两个字段）：① 被别的区域的 `off_by` **指名关断**
    /// （本片）；② 下一片 `note_polyphony` 的检查键。
    #[must_use]
    pub const fn group(&self) -> u32 {
        self.group
    }

    /// ⭐ 本区域**触发时去关掉哪个组**（`off_by`，原样；`0` ＝ **不关任何区域**）。
    #[must_use]
    pub const fn off_by(&self) -> u32 {
        self.off_by
    }

    /// ⭐ 本区域的**关断模式**（`off_mode`，原样；缺省 [`OffMode::Fast`]）。
    #[must_use]
    pub const fn off_mode(&self) -> OffMode {
        self.off_mode
    }

    /// 给定时点的**合并线性增益**（与离线 `PlaybackSpec::total_gain` 逐位相同）。
    #[must_use]
    pub fn gain_at(&self, note: u8, velocity: u8) -> f32 {
        let n = note as usize;
        let v = velocity as usize;
        let base = self.volume_gain * self.velocity_gain[v];
        let crossfade =
            (self.key_crossfade[n] * self.velocity_fade_in[v]) * self.velocity_fade_out[v];
        base * crossfade
    }

    // -----------------------------------------------------------------
    // ⭐ 幅度包络（`ampeg_*`；任务单 `#92`）的**只读读数**与逐块归约
    // -----------------------------------------------------------------

    /// ⭐ `ampeg_attack`（**秒**，原样）。
    #[must_use]
    pub const fn ampeg_attack_seconds(&self) -> f32 {
        self.ampeg_attack
    }

    /// ⭐ `ampeg_hold`（**秒**，原样）。
    #[must_use]
    pub const fn ampeg_hold_seconds(&self) -> f32 {
        self.ampeg_hold
    }

    /// ⭐ `ampeg_decay`（**秒**，原样）。
    #[must_use]
    pub const fn ampeg_decay_seconds(&self) -> f32 {
        self.ampeg_decay
    }

    /// ⭐ `ampeg_sustain`（**百分比** 0..=100，原样 --- ⛔ 不是 0..1 的增益）。
    #[must_use]
    pub const fn ampeg_sustain_percent(&self) -> f32 {
        self.ampeg_sustain
    }

    /// ⭐ `ampeg_release`（**秒**，原样）。
    #[must_use]
    pub const fn ampeg_release_seconds(&self) -> f32 {
        self.ampeg_release
    }

    /// ⭐ **该区域是否"没有包络效应"**（透明包络）：五个值都恰好在各自的**透明缺省**上
    /// （各段零长 ＋ `sustain = 100 %` ＋ `release = 0`）。
    ///
    /// ⚠ 这是**快速通道**的判据，⛔ 不是"合法性"判据：它只看数值，**不看** opcode 是否出现
    /// （`ampeg_attack=0` 与"没写 `ampeg_attack`"在这里**同值** ⇒ 同样走快速通道）。
    ///
    /// 判据的后果（三条路同口径）：透明 ⇒ 逐样本路径**一个浮点运算都不加**
    /// （`env` 传 `None`）⇒ 与接线前的输出**逐位相同**（`S37`／`S33`）。
    #[must_use]
    pub fn ampeg_is_transparent(&self) -> bool {
        self.ampeg_attack == 0.0
            && self.ampeg_hold == 0.0
            && self.ampeg_decay == 0.0
            && self.ampeg_sustain == 100.0
            && self.ampeg_release == 0.0
    }

    /// ⭐ **逐块归约**：把秒 / 百分比换算成**引擎帧**口径的分段线性参数（见 [`AmpegRun`]）。
    ///
    /// 换算口径（逐条可判）：
    /// * `frames = round(seconds × engine_hz)`（**四舍五入**：`0.01 s @ 48 kHz` 若截断会得到
    ///   479 帧而不是 480 ⇒ 判据里"恰好第 `A` 帧到峰"会差一帧）；
    /// * 换算后**为 0 帧**的段 ＝ **零长段 ⇒ 跳过该段**（⛔ 不是"降级政策"，是数学上的必然：
    ///   零长的线性段没有中间点）；
    /// * `engine_hz` 是**摊平期**的那一个（与 `rate_per_note` 同一份）⇒ 换采样率必须由
    ///   控制侧在窗口外**重新摊平**（既有纪律，见模块头）；
    /// * 透明区域 ⇒ `None`（⛔ 不进包络路径）。
    ///
    /// ⚠ **零分配、零锁、零 I/O**（纯浮点/整数换算）⇒ 可被逐块路径调用 [MUST-GATE-001]。
    #[must_use]
    pub fn ampeg_run(&self, engine_hz: f32) -> Option<AmpegRun> {
        ampeg_run_from(
            self.ampeg_attack,
            self.ampeg_hold,
            self.ampeg_decay,
            self.ampeg_sustain,
            self.ampeg_release,
            engine_hz,
        )
    }
}

/// ✂️ **摊平后的乐器**：区域表 ＋ **样本表**，**构造期**建好，回调侧只读。
///
/// # 样本形态（⭐ 任务 `#67` 的步 3）
///
/// * [`Self::prepare_with_samples`]：**每区域播自己**那份样本 —— 去重键 ＝ `region.sample_path()`
///   字符串、容器 ＝ `BTreeMap`（确定性顺序 [MODEL-AST-003]）⇒ 同一路径**只 materialize 一份**
///   [`PreparedSample`]；下标由 [`PreparedRegion::sample_index`] 指路；
/// * [`Self::prepare`]：单样本**广播**（兼容重载，⛔ 未删）⇒ 所有 `sample_index = 0`：
///   键/力度映射**对**、**音色内容错**（各 region 播同一份）⇒ 读数 `sample_content=broadcast(1)`。
///
/// # 分配纪律（红线 7 / [MUST-GATE-001]）
///
/// * [`Self::prepare`] / [`Self::prepare_with_samples`]：**允许分配**（区域表与样本 PCM 的
///   `Vec` 在构造期/控制侧建立）；
/// * [`Self::render_block_into`]：**禁止分配** —— 它只做位测试、数组读取与
///   `out` 的原地写入（⛔ 无 `Vec::push`／`Box`／`String`／锁／I/O）。
///   违反的后果：实时回调内产生堆分配/释放或锁等待 ⇒ 可能造成音频线程不可预测的停顿
///   （红线 7 的一票否决项），由 `tests/rt_zero_alloc.rs` 的**计数型全局分配器**窗口判据检测。
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedSfz {
    regions: Vec<PreparedRegion>,
    /// ⭐ **去重后的样本表**：每个被区域引用的 `sample_path` 至多一份。
    samples: Vec<PreparedSample>,
    /// 建表口径：`true` ⇒ 单样本重载 [`Self::prepare`] 的**广播**路径（⛔ 内容错，已登记）。
    broadcast: bool,
    /// ⭐ 缺样本 ⇒ 标 `NO_SAMPLE`（**不产生采样输出**）的区域数（**计数**，⛔ 不静默）。
    ///
    /// ⚠ 归属裁决（台账 R711）：引擎只出**静音** ＋ 计数，⛔ **不发明替代内容**
    /// （旧行为"回退到 `samples[0]`"是**内容错**，已删除）。
    silent_regions: usize,
    /// ⭐ 缺样本的**路径**（排序 ＋ 去重 ⇒ 确定性顺序；报告/诊断用）。
    missing_samples: Vec<String>,
    /// ⭐ **摊平期采样率**（Hz）：`prepare` / `prepare_with_samples` 的第二参原样抄下。
    ///
    /// 用途只有一个：把区域的 `ampeg_*` **秒**换算成引擎帧（[`PreparedRegion::ampeg_run`]）。
    /// ⚠ 这与 `rate_per_note` 是**同一份事实源**（两者都在摊平期由同一个 `engine_hz` 求得）
    /// ⇒ **换采样率必须由控制侧重新摊平**并发布新快照（既有纪律，见模块头），
    /// ⛔ 实时侧不重算、⛔ 不读时钟。
    engine_hz: f32,
}

/// ✂️ **摊平后的一份样本**：路径（去重键）＋ PCM ＋ 原始采样率。
///
/// ⚠ PCM 是**整段 clone** 进来的（构造期分配）：无字节预算、无流式、无 `Arc` 共享
/// ⇒ 登记为 sfz-needs-8。回调侧只**借用**它（⛔ 不 clone、⛔ 不 `Arc` clone）。
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedSample {
    /// 去重键 ＝ [`yeban_sfz::instrument::Region::sample_path`] 的字符串（⛔ 不规范化、⛔ 不查文件）。
    ///
    /// ⚠ [`PreparedSfz::prepare`] 的**广播**路径没有路径身份 ⇒ 它是**空串**。
    path: String,
    /// 单声道 PCM（归一化到 `[-1, 1]`）。
    pcm: Vec<f32>,
    /// 该 PCM 的原始采样率（Hz；摊平期已在 `Region::playback_rate` 里换算过）。
    sample_hz: f32,
}

impl PreparedSample {
    /// 去重键（`region.sample_path()` 的原样字符串；广播路径为空串）。
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// ⭐ 只读 PCM（**实时侧借用它**，⛔ 不 clone 这个 `Vec`）。
    #[must_use]
    pub fn pcm(&self) -> &[f32] {
        &self.pcm
    }

    /// 原始采样率（Hz）。
    #[must_use]
    pub const fn sample_hz(&self) -> f32 {
        self.sample_hz
    }

    /// 帧数。
    #[must_use]
    pub fn frames(&self) -> usize {
        self.pcm.len()
    }

    /// PCM 字节数（⛔ 不含 `path` 文本）。
    #[must_use]
    pub fn pcm_bytes(&self) -> usize {
        core::mem::size_of::<f32>() * self.pcm.len()
    }

    /// 从调用方给的 [`InMemorySample`] **clone** 一份（构造期；`path` 是去重键）。
    fn from_in_memory(path: String, source: &InMemorySample) -> Self {
        Self {
            path,
            pcm: source.pcm.clone(),
            sample_hz: source.sample_hz,
        }
    }
}

/// ✂️ **摊平表的字节预算**：三条**独立**上界（区域表 ／ PCM ／ 路径文本）。
///
/// ⭐ 本类型**只装上界**，⛔ 不装行为 —— 超过它**不会**拒绝、截断或改变摊平结果
/// （超预算怎么办见 [`PreparedSfz::budget_overrun`] 的归属裁决）。
///
/// # 为什么是三条而不是一个总数
///
/// 摊平表的占用有三条**来源不同**的项（[`PreparedSfz::byte_usage`]）：
///
/// | 项 | 随什么增长 | 单独设界的理由 |
/// | :--- | :--- | :--- |
/// | 区域表 | 区域**条数**（每条 [`PreparedRegion`] 定长） | 它由 SFZ **文本**决定，与样本大小无关 |
/// | PCM | 样本**帧数**（每帧一个 `f32`） | 它是"巨大音色库"的**唯一**放大器（整段 clone，`needs-8`） |
/// | 路径文本 | 路径**字符串长度**（去重后每份一份） | 极小，但它是去重键的半身 ⇒ 单独留读数 |
///
/// 只给一个总数的话，"区域表悄悄长大"与"PCM 爆炸"在读数上**不可区分** ⇒
/// 三条独立上界才能把超预算**指向**是哪一项（[`SfzBudgetOverrun`]）。
///
/// # `N`（上界）的来源：⛔ 不是拍出来的常数
///
/// 本 crate **不自带**任何"默认上限"：上界是**控制侧的输入**。两种给法：
///
/// * [`Self::for_layout`]（本单判据用的形状）：由**声明的形状**（区域数 ／ PCM 帧数 ／
///   路径字节）按**同一份换算**反推 ⇒ "声明内的表一定进得来"是**构造上**成立的，
///   ⛔ 不是靠调数字。口径与 `yeban-decode` 的 `PcmBudget::for_layout` 同一形状
///   （那边的"为什么是这个数"由 `pcm_bytes_for` 一条换算给出 ⇒ 判据可以按同一条公式复算）。
/// * [`Self::new`]：调用方直接给三条上界。⭐ 控制侧若要用**产品要求**推导的上限
///   （如 `yeban-decode` 的 `PcmBudget::max_pcm_bytes`），把那个数**传进来**即可 ——
///   ⛔ 本 crate **不复制**那份常数（一份事实一个来源）。
///
/// ⚠ 判定是**闭区间**：恰好等于上界**通过**（与 `PcmBudget` 同一条口径）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SfzByteBudget {
    /// 区域表上界（字节）。
    max_region_bytes: usize,
    /// 全部样本 PCM 上界（字节；⛔ 不含路径文本）。
    max_pcm_bytes: usize,
    /// 全部样本路径文本上界（字节）。
    max_path_bytes: usize,
}

impl SfzByteBudget {
    /// 逐字段构造（控制侧给的任意上界）。
    #[must_use]
    pub const fn new(max_region_bytes: usize, max_pcm_bytes: usize, max_path_bytes: usize) -> Self {
        Self {
            max_region_bytes,
            max_pcm_bytes,
            max_path_bytes,
        }
    }

    /// ⭐ 由**声明的形状**反推一个预算：`regions` 条区域 ＋ `pcm_frames` 帧单声道 `f32` PCM
    /// ＋ `path_bytes` 字节路径文本。
    ///
    /// 换算就是 [`PreparedSfz::region_bytes`] ／ [`PreparedSfz::pcm_bytes`] ／
    /// [`PreparedSfz::path_bytes`] 那三条**同一份**公式
    /// （`size_of::<PreparedRegion>() × regions` ／ `size_of::<f32>() × frames` ／ 原样）⇒
    /// 返回的预算**恰好**允许该形状；因此判据 "`bytes ≤ N`" 钉住的是
    /// "**读数里没有隐藏的第四份开销**"（⛔ 不是"某个数比某个阈值小"）。
    #[must_use]
    pub const fn for_layout(regions: usize, pcm_frames: usize, path_bytes: usize) -> Self {
        Self {
            max_region_bytes: core::mem::size_of::<PreparedRegion>() * regions,
            max_pcm_bytes: core::mem::size_of::<f32>() * pcm_frames,
            max_path_bytes: path_bytes,
        }
    }

    /// 区域表上界（字节）。
    #[must_use]
    pub const fn max_region_bytes(&self) -> usize {
        self.max_region_bytes
    }

    /// 全部样本 PCM 上界（字节）。
    #[must_use]
    pub const fn max_pcm_bytes(&self) -> usize {
        self.max_pcm_bytes
    }

    /// 全部样本路径文本上界（字节）。
    #[must_use]
    pub const fn max_path_bytes(&self) -> usize {
        self.max_path_bytes
    }
}

/// ✂️ 摊平表的**逐项字节读数**（[`PreparedSfz::byte_usage`]）：区域表 ／ PCM ／ 路径文本。
///
/// ⚠ 三项之和**逐字节等于** [`PreparedSfz::memory_bytes`]（**同一份求和**，
/// ⛔ 不是第二套口径 —— 改这里的求和就等于改 `memory_bytes`，判据会立刻看出来）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SfzByteUsage {
    region_bytes: usize,
    pcm_bytes: usize,
    path_bytes: usize,
}

impl SfzByteUsage {
    /// 逐项构造（判据可用于"不真的分配那份 PCM"的纯算术场景）。
    #[must_use]
    pub const fn new(region_bytes: usize, pcm_bytes: usize, path_bytes: usize) -> Self {
        Self {
            region_bytes,
            pcm_bytes,
            path_bytes,
        }
    }

    /// 区域表占用：`size_of::<PreparedRegion>() × 区域数`。
    #[must_use]
    pub const fn region_bytes(&self) -> usize {
        self.region_bytes
    }

    /// 全部样本 PCM 占用（去重后）：`4 × 总帧数`。
    #[must_use]
    pub const fn pcm_bytes(&self) -> usize {
        self.pcm_bytes
    }

    /// 全部样本**路径文本**占用（去重后）：`Σ path.len()`（⛔ 不含 `String` 容量余量）。
    #[must_use]
    pub const fn path_bytes(&self) -> usize {
        self.path_bytes
    }

    /// 总占用字节数（三项之和；＝ [`PreparedSfz::memory_bytes`]）。
    #[must_use]
    pub const fn total_bytes(&self) -> usize {
        self.region_bytes + self.pcm_bytes + self.path_bytes
    }

    /// ⭐ 对给定预算的**逐项超出量**（全 0 ⇒ 在预算内）。
    ///
    /// ⚠ 纯算术 ⇒ "1 GiB 级音色库会不会超"这件事**可以在不真的持有那 1 GiB 的前提下判定**
    /// （判据 S27 就是这么做的）。
    #[must_use]
    pub const fn overrun(&self, budget: &SfzByteBudget) -> SfzBudgetOverrun {
        SfzBudgetOverrun {
            region_bytes: self.region_bytes.saturating_sub(budget.max_region_bytes),
            pcm_bytes: self.pcm_bytes.saturating_sub(budget.max_pcm_bytes),
            path_bytes: self.path_bytes.saturating_sub(budget.max_path_bytes),
        }
    }
}

/// ✂️ **超预算读数**：逐项**超出上界的字节数**（⭐ 这是引擎对"超预算"唯一给出的东西）。
///
/// 全 0 ⇒ 在预算内（[`Self::is_within`]）。⭐ 本类型只装**事实**，⛔ 不装决定 ——
/// 归属裁决（"引擎只报数、控制侧决定"）见 [`PreparedSfz::budget_overrun`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SfzBudgetOverrun {
    region_bytes: usize,
    pcm_bytes: usize,
    path_bytes: usize,
}

impl SfzBudgetOverrun {
    /// 区域表超出量（字节）。
    #[must_use]
    pub const fn region_bytes(&self) -> usize {
        self.region_bytes
    }

    /// PCM 超出量（字节）。
    #[must_use]
    pub const fn pcm_bytes(&self) -> usize {
        self.pcm_bytes
    }

    /// 路径文本超出量（字节）。
    #[must_use]
    pub const fn path_bytes(&self) -> usize {
        self.path_bytes
    }

    /// 三项超出量之和（字节）。
    #[must_use]
    pub const fn total_bytes(&self) -> usize {
        self.region_bytes + self.pcm_bytes + self.path_bytes
    }

    /// ⭐ 是否**在预算内**（三项超出量全 0）＝ 预算判据 `bytes ≤ N`。
    #[must_use]
    pub const fn is_within(&self) -> bool {
        self.total_bytes() == 0
    }
}

impl PreparedSfz {
    /// **构造期**摊平（**单样本广播**）：把区域谓词、速率、增益与样本一次算好（⭐ 允许分配）。
    ///
    /// ⭐ **区域选择口径 = `Region::matches_query`**（与离线侧、与 `Instrument::region_for_with`
    /// 同一份谓词）。门控（通道 / `trigger` / keyswitch / CC / `held_notes`）在键与力度之外
    /// **独立于** `(note, velocity)`，因此它的结果可以压成一次布尔判断：
    /// 取该区域键/力度两维**都命中**的一个探测点求 `matches_query`，答案就是门控本身；
    /// 门控不过 ⇒ 两张掩码清 0，该区域**永不**进回调的命中集。
    ///
    /// ⭐ 增益按 `total_gain()` 的乘积结构摊平（⛔ 不再冻结在 `(60, 100)`）：见
    /// [`PreparedRegion`] 的字段文档与 [`PreparedRegion::gain_at`]。
    ///
    /// ⚠ ⭐ **这是广播口径**（兼容重载）：**每一**个 region 的 `sample_index` 都是 `0`
    /// —— 键/力度映射**对**，但**音色内容错**（多 `sample=` 的区域会播同一份 PCM）。
    /// 读数 [`Self::sample_content_label`] 为 `broadcast(1)`。要"每区域各播自己"请用
    /// [`Self::prepare_with_samples`]。⛔ 本函数**不**被删：既有调用方与步 2 的逐位读数靠它。
    ///
    /// # Errors
    /// 文本非法 ⇒ [`SfzError`]。
    pub fn prepare(
        instrument: &SfzInstrument,
        source: &InMemorySample,
        engine_hz: f32,
    ) -> Result<Self, SfzError> {
        // 广播：只有**一份**样本（路径身份为空串），所有区域都指它。
        let samples = vec![PreparedSample::from_in_memory(String::new(), source)];
        let sample_hz = source.sample_hz;
        let regions = flatten_regions(instrument, engine_hz, |_region| (0u16, sample_hz))?;
        Ok(Self {
            regions,
            samples,
            broadcast: true,
            silent_regions: 0,
            missing_samples: Vec::new(),
            engine_hz,
        })
    }

    /// ⭐ **构造期**摊平（**多采样**）：每个区域播**它自己的** `sample=`。
    ///
    /// # 口径（三条，都可机械判定）
    ///
    /// 1. **去重键**＝[`yeban_sfz::instrument::Region::sample_path`] 的**字符串**原样
    ///    （⛔ 不规范化、⛔ 不看文件内容）；容器 ＝ `BTreeMap` ⇒ 顺序确定 [MODEL-AST-003]。
    ///    **同一样本引两次只 materialize 一份**（[`Self::sample_count`] ＝ 1、
    ///    [`Self::pcm_bytes`] ＝ 一份的字节数）。提供表里同一路径出现两次时**首个为准**。
    /// 2. **解码在控制侧**：本函数只接**已经解码**的 [`InMemorySample`] ⇒ 引擎⛔ 不引
    ///    `symphonia`/`rubato`、⛔ 不做任何 I/O。调用方先用
    ///    [`SfzInstrument::referenced_sample_paths`] 取路径全集，再按
    ///    `crates/yeban-app/src/sfz_load.rs` 的形状真读 PCM WAV。
    /// 3. ⭐ ⭐ **缺样本 ⇒ 静音 ＋ 计数**（⛔ 不静默、⛔ **不发明内容**）：
    ///    `provided_samples` 里**没有**某个 `sample_path` 时 —— 该区域标
    ///    [`PreparedRegion::NO_SAMPLE`]（不产生任何采样输出，触发时**不建声部**）
    ///    并计入 [`Self::silent_region_count`]，路径进 [`Self::missing_sample_paths`]。
    ///    ⛔ **本函数不再"回退到 `samples[0]`"**：那会让缺样本的区域去播**别人的采样**
    ///    （听得出、读文件看不见的**内容错**）。⭐ 归属裁决（台账 R711）：**引擎出静音并计数**、
    ///    **合成替代内容是控制侧的决定**（想给替代样本 ⇒ 由调用方显式放进 `provided_samples`，
    ///    或先用 [`Self::prepare`] 明确声明"这一份就是全部区域的样本"）。
    ///
    /// ⚠ 样本数上限：`sample_index` 是 `u16` ⇒ 去重后至多 `65534` 份（超出时后续区域被记为
    /// `NO_SAMPLE`，⛔ 不 panic、⛔ 不回绕）。
    ///
    /// # Errors
    /// 文本非法 ⇒ [`SfzError`]。
    pub fn prepare_with_samples(
        instrument: &SfzInstrument,
        provided_samples: &[(String, InMemorySample)],
        engine_hz: f32,
    ) -> Result<Self, SfzError> {
        // ① 提供表的**确定性索引**（BTreeMap）：同一路径出现两次 ⇒ 首个为准。
        let mut provided_index: BTreeMap<&str, usize> = BTreeMap::new();
        for (index, (path, _sample)) in provided_samples.iter().enumerate() {
            provided_index.entry(path.as_str()).or_insert(index);
        }
        // ② 样本表按**区域引用顺序**去重建立（⛔ 不预置"回退目标"：缺样本不回退）。
        let mut samples: Vec<PreparedSample> = Vec::new();
        let mut index_of: BTreeMap<String, u16> = BTreeMap::new();
        let mut missing: Vec<String> = Vec::new();
        let mut silent_regions = 0usize;
        let regions = flatten_regions(instrument, engine_hz, |region| {
            let path = region.sample_path();
            // 已 materialize 过 ⇒ 直接复用（**去重**：同一路径只解码/只 clone 一次）。
            if let Some(index) = index_of.get(path.as_ref()) {
                return (*index, samples[usize::from(*index)].sample_hz);
            }
            // 提供表里有这份路径 ⇒ materialize 一份新的（按**首次引用顺序**编号）。
            if let Some(provided) = provided_index.get(path.as_ref()).copied() {
                let (path, source) = &provided_samples[provided];
                let index = u16::try_from(samples.len()).unwrap_or(PreparedRegion::NO_SAMPLE - 1);
                samples.push(PreparedSample::from_in_memory(path.clone(), source));
                let sample_hz = samples[usize::from(index)].sample_hz;
                index_of.insert(path.clone(), index);
                return (index, sample_hz);
            }
            // ⭐ 缺样本：**静音 ＋ 计数**（⛔ 不静默、⛔ 不去播别人的采样）。
            missing.push(path.clone().into_owned());
            silent_regions += 1;
            // 没有样本 ⇒ 速率表用引擎采样率（该区域**永不渲染**，读数不进结论）。
            (PreparedRegion::NO_SAMPLE, engine_hz)
        })?;
        missing.sort();
        missing.dedup();
        Ok(Self {
            regions,
            samples,
            broadcast: false,
            silent_regions,
            missing_samples: missing,
            engine_hz,
        })
    }

    /// ⭐ **区域表**占用的字节数（`size_of::<PreparedRegion>() × 区域数`）。
    ///
    /// ⚠ 这是**结构本身**的字节数（⛔ 不含 `Vec` 的容量余量、⛔ 不含任何样本数据）：
    /// [`PreparedRegion`] 是定长纯数据（两张掩码 ＋ 五张 `[f32; 128]` 表 ＋ 标量）
    /// ⇒ "区域条数"这一维的开销是**可复算**的。
    #[must_use]
    pub fn region_bytes(&self) -> usize {
        core::mem::size_of::<PreparedRegion>() * self.regions.len()
    }

    /// ⭐ **摊平期采样率**（Hz）—— 把区域的 `ampeg_*` 秒换算成引擎帧的那一份事实源。
    ///
    /// ⚠ 与 `rate_per_note` **同源**（同一次 `prepare(…, engine_hz)`）⇒ 换采样率必须
    /// 由控制侧重新摊平；本读数只是把它带出来（判据 S34 会对着它复算帧数）。
    #[must_use]
    pub const fn engine_hz(&self) -> f32 {
        self.engine_hz
    }

    /// ⭐ **全部样本路径文本**占用的字节数（去重后 `Σ path.len()`）。
    ///
    /// ⚠ 只算 `len()`（⛔ 不含 `String` 的容量余量）；广播路径是空串 ⇒ 0。
    #[must_use]
    pub fn path_bytes(&self) -> usize {
        self.samples
            .iter()
            .map(|sample| sample.path.len())
            .sum::<usize>()
    }

    /// ⭐ **逐项字节读数**（区域表 ／ PCM ／ 路径文本）—— `needs-8` 的"现状拆开"读数。
    ///
    /// ⚠ 三项之和 ＝ [`Self::memory_bytes`]（同一份求和）。
    #[must_use]
    pub fn byte_usage(&self) -> SfzByteUsage {
        SfzByteUsage {
            region_bytes: self.region_bytes(),
            pcm_bytes: self.pcm_bytes(),
            path_bytes: self.path_bytes(),
        }
    }

    /// 摊平占用的**内存字节数**（读数用：区域表 ＋ 样本 PCM ＋ 样本路径文本）。
    ///
    /// ⚠ `path` 只算 `len()`（⛔ 不含 `String` 的容量余量）；广播路径是空串 ⇒ 0。
    /// ⭐ 本读数就是 [`Self::byte_usage`] 三项之和（**同一份求和**：⛔ 不是第二套口径）。
    #[must_use]
    pub fn memory_bytes(&self) -> usize {
        self.byte_usage().total_bytes()
    }

    /// ⭐⭐ **字节预算判据**（`needs-8` 的"可判"落地）：本摊平表对给定预算的**逐项超出读数**。
    ///
    /// 判定量 ＝ `bytes ≤ N`（**闭区间**）：[`SfzBudgetOverrun::is_within`] 为 `true` 当且仅当
    /// 区域表 ／ PCM ／ 路径文本三项**各自**都在 [`SfzByteBudget`] 的上界内。
    /// `N` 的来源见 [`SfzByteBudget`]（判据用 [`SfzByteBudget::for_layout`] 的"声明形状反推"）。
    ///
    /// # ⭐ 超预算怎么办 —— 归属裁决：**引擎只报数，调用方决定**
    ///
    /// 引擎侧对"超预算"的**唯一**动作是给出这份 [`SfzBudgetOverrun`]：
    ///
    /// * ⛔ **不报错**（`prepare` ／ `prepare_with_samples` 仍然返回 `Ok`）；
    /// * ⛔ **不计数继续**（"计数"是 `needs-7` 那条**缺样本**的口径，本项**不新增**计数器）；
    /// * ⛔ **不截断**：不丢样本、不丢区域、不改变任何渲染读数。
    ///
    /// 三条理由：
    ///
    /// 1. **在摊平里报错来得太晚**。调用 [`Self::prepare_with_samples`] 时，
    ///    `provided_samples` 里那份已解码的 [`InMemorySample`] **已经在调用方手里**
    ///    （本函数只是把 `pcm` 整段 clone 一份）⇒ 峰值内存在**调用之前**就已花掉一次。
    ///    在摊平里报错 ⛔ **挡不住那个峰值**，只会把"载入成功但偏重"变成"载入失败"。
    ///    ⇒ 真正能省内存的闸门必须落在**控制侧的解码之前**（那是 app 侧的事，本单⛔ 不动）。
    /// 2. **归属先例**：台账 `R711`（`needs-7`）已裁决"**引擎出事实 ＋ 计数**、
    ///    **产品决定交控制侧**"。字节预算是同一条分界线：引擎拥有"这张表占多少字节"这份
    ///    **事实**，控制侧拥有"这个库值不值得载入 ／ 要不要提示用户 ／ 要不要不发出这一轨"
    ///    这份**产品判断**。
    /// 3. **⛔ 不动既有判据**：在 `prepare*` 里强制预算就必须改它们的形状
    ///    （`Result` 变体 ／ 新参数）⇒ 既有调用点（[`SfzPlan::prepare`] ／
    ///    [`SfzPlan::prepare_with_samples`] ／ `crates/yeban-app/src/sfz_load.rs` ／
    ///    S11～S16）全部要动。本单明确⛔ 不改既有类型与签名 ⇒ 只加**新**读数与新判据。
    ///
    /// ⭐ 因此控制侧的正确用法：拿 [`Self::byte_usage`] ／ 本读数**自行**决定
    /// （例如把 `yeban-decode` 的 `PcmBudget::max_pcm_bytes` 传进 [`SfzByteBudget::new`]，
    /// 在解码之前或之后判）。
    #[must_use]
    pub fn budget_overrun(&self, budget: &SfzByteBudget) -> SfzBudgetOverrun {
        self.byte_usage().overrun(budget)
    }

    /// 区域数（摊平后）。
    #[must_use]
    pub fn region_count(&self) -> usize {
        self.regions.len()
    }

    /// 摊平后的区域表（**只读**；判据与控制侧读数用）。
    #[must_use]
    pub fn regions(&self) -> &[PreparedRegion] {
        &self.regions
    }

    /// ⭐ 摊平后的**样本表**（去重后；**只读借用**：实时侧⛔ 不 clone）。
    #[must_use]
    pub fn samples(&self) -> &[PreparedSample] {
        &self.samples
    }

    /// ⭐ 单份样本（`sample_index` 之外 / `NO_SAMPLE` ⇒ `None`）。
    #[must_use]
    pub fn sample(&self, index: u16) -> Option<&PreparedSample> {
        if index == PreparedRegion::NO_SAMPLE {
            return None;
        }
        self.samples.get(usize::from(index))
    }

    /// ⭐ **样本数（去重后）＝ 摊平期的样本 clone 次数**（"解码次数"的机械读数）：
    /// 同一路径引两次 ⇒ `1`。
    #[must_use]
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    /// ⭐ **全部样本的 PCM 字节数**（去重后；⛔ 不含路径文本）。
    #[must_use]
    pub fn pcm_bytes(&self) -> usize {
        self.samples
            .iter()
            .map(PreparedSample::pcm_bytes)
            .sum::<usize>()
    }

    /// ⭐ 是否走**单样本广播**重载（[`Self::prepare`]）⇒ ⛔ 音色内容错（模块头 ⑨）。
    #[must_use]
    pub const fn is_broadcast(&self) -> bool {
        self.broadcast
    }

    /// ⭐ 缺样本 ⇒ 标 [`PreparedRegion::NO_SAMPLE`]（**不产生采样输出**）的**区域数**。
    ///
    /// ⚠ 这就是"缺样本"的**唯一**计数口径（⛔ 不再有第二个"回退"计数）：
    /// 引擎只出静音 ＋ 计数、⛔ 不发明替代内容（归属裁决见台账 R711）。
    #[must_use]
    pub const fn silent_region_count(&self) -> usize {
        self.silent_regions
    }

    /// ⭐ 缺样本的**路径**（排序 ＋ 去重）。
    #[must_use]
    pub fn missing_sample_paths(&self) -> &[String] {
        &self.missing_samples
    }

    /// ⭐ **样本内容读数**（报告用）：
    /// `broadcast(N)` / `distinct(N)`，缺样本时追加 `+silent(M)`。
    #[must_use]
    pub fn sample_content_label(&self) -> String {
        let mut label = if self.broadcast {
            format!("broadcast({})", self.samples.len())
        } else {
            format!("distinct({})", self.samples.len())
        };
        if self.silent_regions > 0 {
            label.push_str(&format!("+silent({})", self.silent_regions));
        }
        label
    }

    /// ⭐ **回调侧渲染**：把一个音符混进 `out`，返回写入帧数。**零分配**（只读 + 原地写）。
    ///
    /// ⭐ **多匹配语义 = `all`**：匹配的每条区域都叠加（与离线
    /// [`SfzInstrument::render_note_into`] 同一口径）。返回值 = 各匹配区域写入帧数的
    /// **最大值**（⛔ 不是各区域之和）。
    ///
    /// ⭐ **每区域播自己那份样本**（[`PreparedRegion::sample_index`]）；`NO_SAMPLE` 的区域跳过。
    ///
    /// 增益的相乘顺序与离线 `PlaybackSpec::total_gain()` **逐位相同**：
    /// `(volume × 力度) × ((键轴 × 力度淡入) × 力度淡出)`。
    /// ⭐ **`span`（`offset`/`end`）已应用**（步 3）：源位置从 `span.start` 起、
    /// 在 `min(span.end, PCM 长度)` 停（[`PreparedRegion::span_bounds`] ⇒ [`reduce_span`]）
    /// ⇒ 与离线侧、与实时声部的**逐样本最大绝对差为 0**，且覆盖**整段**（含显式 `end`）。
    pub fn render_block_into(&self, note: u8, velocity: u8, out: &mut [f32]) -> usize {
        let n = note as usize;
        let mut written = 0usize;
        for region in &self.regions {
            if !region.accepts(note, velocity) {
                continue;
            }
            let Some(sample) = self.sample(region.sample_index) else {
                continue;
            };
            let pcm = sample.pcm();
            let rate = region.rate_per_note[n];
            if !(rate.is_finite() && rate > 0.0) || pcm.is_empty() {
                continue;
            }
            let Some((start, source_end)) = region.span_bounds(pcm.len()) else {
                continue;
            };
            let gain = region.gain_at(note, velocity);
            // ⭐ 循环窗口（本单）：归约已把模式／退化／越界三条判完（`None` ⇒ 不循环）。
            let loop_window = region.loop_window(pcm.len());
            let mut position = start;
            // 摊平表的一次性调用**不窃取**（与上一条路同款：`× 1.0` 逐位不变）。
            let mut fade = FadeRamp::NONE;
            // ⭐ 包络（任务 `#92`）：与离线/实时两条路**同一份归约**（秒 × 本表的 `engine_hz`）。
            // 这条一次性调用**没有 note-off** ⇒ 只走 起音→保持→衰减→保持（`frame = u64::MAX`）。
            let mut env = region
                .ampeg_run(self.engine_hz)
                .map(|run| EnvRun::new(run, ENV_STAGE_IDLE, 0, u64::MAX, 0, false));
            let region_written = mix_span_into(
                pcm,
                &mut position,
                rate,
                gain,
                source_end,
                loop_window,
                &mut fade,
                &mut env,
                out,
            );
            written = written.max(region_written);
        }
        written
    }
}

/// ⭐ **区域摊平的唯一实现**（构造期）：把一条 SFZ 的每条区域变成 [`PreparedRegion`]。
///
/// `resolve` 决定"这条区域用哪份样本"：
/// * [`PreparedSfz::prepare`] 传常数 `(0, source.sample_hz)`（**广播**）；
/// * [`PreparedSfz::prepare_with_samples`] 传路径查表（**多采样** ＋ 去重；缺样本 ⇒ 静音 ＋ 计数）。
///
/// ⭐ 每个区域的**速率表**用**它自己那份样本**的采样率换算（`RenderRates::new(sample_hz, engine_hz)`）
/// ⇒ 多采样下不同源采样率的样本各自换算正确（⛔ 不是全表一个 rates）。
fn flatten_regions(
    instrument: &SfzInstrument,
    engine_hz: f32,
    mut resolve: impl FnMut(&Region<'_>) -> (u16, f32),
) -> Result<Vec<PreparedRegion>, SfzError> {
    instrument.with_instrument(|parsed| {
        let mut regions = Vec::with_capacity(parsed.regions().len());
        for region in parsed.regions() {
            let (sample_index, sample_hz) = resolve(region);
            let rates = RenderRates::new(sample_hz, engine_hz);
            let mut key_mask = [0u64; 2];
            let mut vel_mask = [0u64; 2];
            let mut rate_per_note = [0.0f32; 128];
            for note in 0u16..128 {
                let note_u8 = note as u8;
                if region.matches_key(note_u8) {
                    key_mask[note as usize / 64] |= 1u64 << (note % 64);
                }
                rate_per_note[note as usize] = region.playback_rate(note_u8, rates);
            }
            for velocity in 0u16..128 {
                if region.matches_velocity(velocity as u8) {
                    vel_mask[velocity as usize / 64] |= 1u64 << (velocity % 64);
                }
            }
            // ⭐ 门控：取"键与力度都命中"的一个探测点 ⇒ `matches_query` 的答案只剩
            // 通道 / trigger / keyswitch / CC 这些与 (note, velocity) 无关的门控。
            // 门控不过 ⇒ 掩码清 0（`trigger=release` 的 region 因此不进 note-on 掩码）。
            let probe = first_set_bit(&key_mask).zip(first_set_bit(&vel_mask));
            let gate_open = probe.is_some_and(|(note, velocity)| {
                region.matches_query(&RegionQuery::new(note, velocity))
            });
            if !gate_open {
                key_mask = [0u64; 2];
                vel_mask = [0u64; 2];
            }

            // ⭐ 增益：`total_gain() = (volume × 力度) × 交叉淡化`，而交叉淡化是一条
            // 从左到右的乘法链：键轴段（in/out）→ 力度轴段（in/out）→ CC 段（**跳过**，
            // 因为回调侧没有 CC 状态，离线 `playback_spec` 也是 `cc = None`）。
            // 分开摊平键轴、力度淡入、力度淡出三张表，回调侧就能**逐位**还原该链。
            let mut key_crossfade = [1.0f32; 128];
            let mut velocity_fade_in = [1.0f32; 128];
            let mut velocity_fade_out = [1.0f32; 128];
            for crossfade in &region.crossfades {
                match crossfade.axis {
                    XfAxis::Key => {
                        for (note, slot) in key_crossfade.iter_mut().enumerate() {
                            *slot *= crossfade.gain_at(note as u8);
                        }
                    }
                    XfAxis::Velocity => {
                        let table = match crossfade.direction {
                            XfDirection::In => &mut velocity_fade_in,
                            XfDirection::Out => &mut velocity_fade_out,
                        };
                        for (velocity, slot) in table.iter_mut().enumerate() {
                            *slot *= crossfade.gain_at(velocity as u8);
                        }
                    }
                    // CC 轴：回调侧没有 CC 状态 ⇒ 与离线 `playback_spec` 一致地跳过
                    // （`Region::crossfade_gain` 对未提供状态的 CC 段就是 `continue`）。
                    XfAxis::Cc(_) => {}
                }
            }
            let mut velocity_gain = [0.0f32; 128];
            for (velocity, slot) in velocity_gain.iter_mut().enumerate() {
                *slot = region.velocity_gain(velocity as u8);
            }

            regions.push(PreparedRegion {
                key_mask,
                vel_mask,
                rate_per_note,
                key_crossfade,
                velocity_fade_in,
                velocity_fade_out,
                volume_gain: region.linear_gain(),
                velocity_gain,
                sample_index,
                span: region.playback_span(),
                // ⭐ 循环接线（本单）：三个**每区域**字段原样搬进摊平表。生效模式取
                // `effective_loop_mode()`（与 `PlaybackSpec::loop_mode` 同一份读数）
                // ⇒ `trigger=release` 家族在这里就已经是 `one_shot`。
                loop_start: region.loop_start,
                loop_end: region.loop_end,
                loop_mode: region.effective_loop_mode(),
                // ⭐ 幅度包络（任务单 `#92`）：五个**每区域**参数原样搬进摊平表
                // （秒 / 百分比 —— ⛔ 不在这里换算成帧：换算在逐块读该区域时用
                // `engine_hz` 做，见 `PreparedRegion::ampeg_run`）。
                ampeg_attack: region.ampeg_attack,
                ampeg_hold: region.ampeg_hold,
                ampeg_decay: region.ampeg_decay,
                ampeg_sustain: region.ampeg_sustain,
                ampeg_release: region.ampeg_release,
                // ⭐ 独占组（任务单 `#94`）：三个**每区域**字段原样搬进摊平表。
                // `group` / `off_by` 的解析与域（`0..=u32::MAX`）在 `yeban-sfz` 里早已有
                // 判据（`instrument.rs` 的 `read_u32` 一族）⇒ 本片只**搬运**，⛔ 不新增解析。
                group: region.group,
                off_by: region.off_by,
                off_mode: region.off_mode,
            });
        }
        regions
    })
}

/// ⭐ **窃取淡出的一段斜坡**（逐帧推进；`Copy`、定长、⛔ 无指针）。
///
/// 形状与 [`StealFade`](yeban_sfz::voice_pool::StealFade) 的 `gain_at` **同一条**
/// （`FLOOR^(n/N)`，`−60 dBFS` 收底）：第 0 帧 `gain == 1.0`，第 `N−1` 帧约 `1.2e-3`，
/// 走完 `N` 帧后归 `false` 并把 `gain` 置 `0.0`（⇒ 与接管者之间不留跃变）。
///
/// ⚠ **非淡出**用 [`FadeRamp::NONE`]（`decay == 1.0`）：逐帧那个乘法是 `× 1.0`，
/// 在 binary32 下**逐位不变** ⇒ 离线/摊平那条路与接线前的读数一字不差。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FadeRamp {
    /// 是否仍在淡出（`false` ⇒ [`Self::NONE`] 语义，逐帧增益恒 `1.0`）。
    fading: bool,
    /// 剩余淡出帧数（`fading == true` 时恒 `>= 1`）。
    remaining: u32,
    /// 当前逐帧增益（非淡出恒 `1.0`）。
    gain: f32,
    /// 每帧乘的衰减因子 `FLOOR^(1/N)`（触发期算好 ⇒ ⛔ 逐样本无 `powf`）。
    decay: f32,
}

impl FadeRamp {
    /// **不淡出**（`decay == 1.0` ⇒ 逐帧乘法是恒等）。
    pub const NONE: Self = Self {
        fading: false,
        remaining: 0,
        gain: 1.0,
        decay: 1.0,
    };

    /// 按**帧数**构造一段 3 ms 指数淡出；`frames == 0` ⇒ [`Self::NONE`]（硬窃取）。
    ///
    /// `decay = STEAL_FADE_FLOOR^(1/frames)` 用 `f64` 算一次（**触发期**，
    /// 与 PolySynth 的 `note_on` 允许 `exp` 同一条纪律）⇒ 逐样本只有一次乘法。
    #[must_use]
    pub fn new(frames: u32) -> Self {
        if frames == 0 {
            return Self::NONE;
        }
        let decay = f64::from(STEAL_FADE_FLOOR).powf(1.0 / f64::from(frames)) as f32;
        Self {
            fading: true,
            remaining: frames,
            gain: 1.0,
            decay,
        }
    }

    /// 是否仍在淡出（`false` ⇒ 未淡出**或**已走完）。
    #[must_use]
    pub const fn is_fading(&self) -> bool {
        self.fading
    }

    /// 剩余淡出帧数（非淡出恒 `0`）。
    #[must_use]
    pub const fn remaining(&self) -> u32 {
        self.remaining
    }

    /// 推进一帧；返回 `false` 表示**淡出已走完**（调用方应交出剩余帧）。
    fn advance(&mut self) -> bool {
        self.remaining = self.remaining.saturating_sub(1);
        self.gain *= self.decay;
        if self.remaining == 0 {
            self.gain = 0.0;
            self.fading = false;
            false
        } else {
            true
        }
    }
}

/// ⭐ **逐样本包络的运行时束**（任务单 `#92`）：把"**每声部**的推进状态"、
/// "**每块**求得的参数（[`AmpegRun`]）"与"note-off / 释放 epoch"打成一个**定长、`Copy`
/// 之外的纯值**入参 —— ⛔ 它不进任何结构体的布局（`SfzVoice` 只多 5 字节，见 `S9`）。
///
/// # ⭐ `frame` 一个 `u64` 承载两件事（**有登记**）
///
/// * **未进释放**：它是 **note-off 帧**（就是 [`SfzVoice::end_sample`] 的原义）；
/// * **已进释放**：它是**释放 epoch**（释放段的第 0 帧）。
///
/// 于是"释放已走的帧数" ＝ `abs_frame − frame`；而 epoch 在**进释放那一帧**被改写成
/// `abs_frame`（⭐ 任务 `#94` 起：三档一律"note-off 就地进释放"，⛔ 没有"等回绕"那种
/// 延迟改写）⇒ ⭐ 这一个 `u64` 就够（⛔ 不需要给
/// `SfzVoice` 加第 9 个字段）。释放一旦开始，note-off 帧就**不再被需要**
/// （退休已由包络接管）⇒ ⛔ 不产生语义冲突。
///
/// # ⭐ 为什么是**自有状态**（⛔ 不是 `&mut` 借用）
///
/// 两个理由，都是机械的：
///
/// 1. `SfzVoice` 的**尺寸**被字面判据钉住（`S9`：`40 B`）：把状态打成一个
///    `#[derive(Copy)] struct { stage: u8, counter: u32 }` 会因**对齐**从 5 字节涨到
///    8 字节 ⇒ 声部涨到 48 B ⇒ 判据红。故声部里是两个**独立字段**。
/// 2. 借用式（`&mut` 字段）会让构造点只能写成 `match opt { Some(r) => Some(EnvRun::new(
///    &mut stage, …)), None => None }` —— 那个 `match` 被 `clippy::manual_map` 判红，
///    而改回 `.map(闭包)` 又会被**借用检查器**拒绝（闭包体里返回借用了捕获变量的值，
///    闭包签名表达不出那个生命周期）。自有状态两边都清爽：`.map(闭包)` 合法
///    （闭包只**读**外部值、返回**自有**值），逐块写回也只是一次结构体赋值。
pub(crate) struct EnvRun {
    /// 段位（[`ENV_STAGE_IDLE`] / 四个段 / ＋释放位 / ＋等回绕位）。
    stage: u8,
    /// 段内计数：**自触发起的帧数**（进入释放/等回绕后**冻结**在 `t_off`）。
    counter: u32,
    /// 每块由区域求得的帧口径参数。
    run: AmpegRun,
    /// 未进释放 ⇒ note-off 帧；已进释放 ⇒ 释放 epoch（见类型文档）。
    frame: u64,
    /// 本块 `out[0]` 的**引擎绝对帧**（释放进度靠它算）。
    ///
    /// ⚠ 离线/摊平两条路给 `0`：它们**没有 note-off**（`frame = u64::MAX`）⇒ 该值不被使用。
    abs_base: u64,
    /// 该区域的 note-off 之后是否**继续循环**（`loop_continuous` **且**窗口有效）。
    ///
    /// 规范逐字（<https://sfzformat.com/opcodes/loop_mode/>）："In loop_continuous mode,
    /// the loop will repeat if the loop end is reached during the release phase, including
    /// repeating multiple times if the release time is longer than the loop length."
    /// ⇒ 三档里**只有它**在释放期照旧回绕；`no_loop` / `loop_sustain` 释放期⛔ 不循环
    /// （"During the release phase, there's no looping." ＋ 沿源位置往后播到样本尾）。
    ///
    /// ⚠ 窗口退化（`loop_end <= loop_start` ⇒ [`PreparedRegion::loop_window`] 为 `None`）
    /// 时必须是 `false`：那时根本没有窗口可回绕。调用方按 `loop_window.is_some()` 求值。
    loop_during_release: bool,
}

impl EnvRun {
    /// ⭐ 构造（逐块一次；⛔ 不在逐样本路径上）。
    ///
    /// `stage` / `counter` 从声部抄来（跨块延续）；`frame` 是 note-off 帧**或**释放 epoch
    /// （由声部的 [`SfzVoice::end_sample`] 承载 —— 语义由 `stage` 决定，见类型文档）；
    /// `loop_during_release` 见字段文档（离线/摊平两条路给 `false`：它们没有 note-off）。
    pub(crate) const fn new(
        run: AmpegRun,
        stage: u8,
        counter: u32,
        frame: u64,
        abs_base: u64,
        loop_during_release: bool,
    ) -> Self {
        EnvRun {
            stage,
            counter,
            run,
            frame,
            abs_base,
            loop_during_release,
        }
    }

    /// ⭐ **本帧允不允许回绕**（[`mix_span_into`] 回绕闸门的**唯一**判据）。
    ///
    /// * `loop_continuous` ⇒ **恒允许**（释放期照旧循环，规范那一条）；
    /// * 其余模式 ⇒ 只在**释放期之前**允许：`abs_frame < self.frame` 把 **note-off 那一帧**
    ///   也算进释放期（⛔ 不在 note-off 帧再回绕一次）；一旦进释放，`frame` 被改写成释放
    ///   epoch（`<= abs_frame`）⇒ 这个比较恒假 ⇒ 源位置沿 `source_end` 往后播。
    ///
    /// ⚠ 没有包络的声部（`env == None`）走**另一支**：回绕不受任何限制（＝ 接线前的行为）。
    fn may_wrap(&self, abs_frame: u64) -> bool {
        if self.loop_during_release {
            return true;
        }
        self.stage & ENV_RELEASING == 0 && abs_frame < self.frame
    }

    /// ⭐ **推进一帧并给出该帧的包络增益**；`None` ＝ 包络**已归零**
    /// （释放段走完 ⇒ 调用方应退休声部）。
    ///
    /// 判定次序（逐条可判）：
    /// 1. **启用**：`IDLE` ⇒ `ATTACK`（区域有包络才会走到这里）；
    /// 2. **note-off**（只在还没进释放时）：记下**当时所在的段**
    ///    （释放段要从那一刻的电平起算，这是"分段线性"的必要信息），把 `frame` 改写成
    ///    释放 epoch ＝ 本帧 ⇒ **三档（`no_loop` / `loop_sustain` / `loop_continuous`）
    ///    一律就地进释放**（规范逐字："the duration of the release phase is set using
    ///    `ampeg_release`"）；释放期**要不要继续循环**由 [`Self::may_wrap`] 决定，
    ///    ⛔ 不在这里分支（也就⛔ 不存在"等回绕"那一档）；
    /// 3. **电平**：释放 ⇒ [`AmpegRun::level_at`] 再乘 `(R − k) / R`；其余 ⇒ `level_at`；
    /// 4. **计数**：只有还没进释放时才推进（释放期冻结在 `t_off`）。
    ///
    /// ⚠ **未进释放时的段位由计数唯一决定**（段边界是常数 `A` / `A+H` / `A+H+D`）⇒
    /// 每帧都按 [`env_held_stage_byte`] 刷新 `stage`，于是 [`SfzVoice::envelope_stage`]
    /// 是**真实段位**（⛔ 不是"永远是 Attack"的假读数）。
    fn advance(&mut self, abs_frame: u64) -> Option<f32> {
        let t = self.counter;
        let mut byte = self.stage;
        if byte == ENV_STAGE_IDLE {
            byte = ENV_STAGE_ATTACK;
        }
        if byte & ENV_RELEASING == 0 {
            byte = env_held_stage_byte(&self.run, t);
            if abs_frame >= self.frame {
                // ⭐ note-off 就地进释放：段位低 3 位记下"收到时在哪一段"（`L_off` 的来源）。
                byte |= ENV_RELEASING;
                self.frame = abs_frame;
            }
        }
        let level = if byte & ENV_RELEASING != 0 {
            match self.run.release_scale(abs_frame.saturating_sub(self.frame)) {
                Some(scale) => self.run.level_at(byte, t) * scale,
                None => 0.0,
            }
        } else {
            self.run.level_at(byte, t)
        };
        if byte & ENV_RELEASING == 0 {
            self.counter = t.saturating_add(1);
        }
        self.stage = byte;
        // ⭐ 释放走完（或零长释放）⇒ `release_scale` 落空 ⇒ 电平 0 ⇒ 退休。
        // ⚠ 用 `level > 0.0 || 未在释放` 而不是 `level > 0.0`：`sustain = 0` 且**未**释放时
        // 电平恒 0 —— 那**不是**"释放走完"，声部应照旧活着（继续写 0 直到 note-off/样本尾）。
        if level > 0.0 || byte & ENV_RELEASING == 0 {
            Some(level)
        } else {
            None
        }
    }

    /// ⭐ 把推进状态写回声部的三个字段（逐块一次）。
    ///
    /// `stage` / `counter` / `frame` 分别回写 [`SfzVoice::env_stage`] /
    /// [`SfzVoice::env_counter`] / [`SfzVoice::end_sample`]（最后那个是 note-off ↔ epoch 的
    /// **同一格**，见类型文档）。
    fn write_back(self, voice: &mut SfzVoice) {
        voice.env_stage = self.stage;
        voice.env_counter = self.counter;
        voice.end_sample = self.frame;
    }
}

/// ⭐ **全仓唯一的逐样本混音循环**（线性插值 ＋ 增益乘加 ＋ 源采样终点 ＋ 幅度包络）。
///
/// `position` 是**可变**的源采样位置 ⇒ 无状态的一次性调用
/// （[`PreparedSfz::render_block_into`]）与跨量子延续的实时声部
/// （[`SfzVoicePool`]）走**同一段代码**（本仓"不许两份实现"的机械纪律；
/// 两份实现在 binary32 下无法维持"逐位相同"）。
///
/// 停止条件（写每个样本**之前**）：`floor(position) + 1 >= source_end` ——
/// 线性插值需要 `position` 与 `position + 1` 两个样本。
/// `source_end` 由调用方给出（⭐ 三条路都是 [`reduce_span`] 的归约结果）：
/// * 摊平表的一次性调用给 [`PreparedRegion::span_bounds`] 的终点（`span` 已应用）；
/// * 实时声部给同一个 [`PreparedRegion::span_bounds`]（触发期抄下）；
/// * 离线 [`SfzInstrument::render_note_into`] 给同一份归约。
///
/// ⚠ **给了循环窗口时这一条不可达**：回绕把位置压在 `[loop_start, loop_end)` 内，
/// 而窗口已由 [`reduce_loop_window`] 保证 `loop_end < pcm_len` 且
/// `loop_end <= source_end` ⇒ 声部由**外部**终止（`end_sample`）或继续循环，
/// ⛔ 不再由"样本播完"退休。
///
/// ⭐ `fade` 是**窃取淡出**：非淡出传 [`FadeRamp::NONE`]（`× 1.0` 逐位不变）；
/// 淡出中**写满 `remaining` 帧就返回**（`written == remaining`）⇒ 调用方可以就地
/// 提升挂起声部、用**同一段**代码继续写剩下的帧。
///
/// ⚠ **零分配、零锁、零 I/O**：只读 `pcm`、原地写 `out` [MUST-GATE-001]。
///
/// # ⭐ 循环（本单新增；`sfz-needs-6` 的"循环"那一半）
///
/// `loop_window` 是[归约后](reduce_loop_window)的**有效**窗口
/// （半开 `[start, end)`）；`None` ＝ 按不循环渲染 ⇒ 逐样本路径**一字不改**
/// （`if let` 落空 ⇒ ⛔ 不加任何浮点运算）⇒ 既有夹具**逐位不变**。
///
/// 写每个样本**之前**回绕：位置一旦到达（或越过）窗口终点，就折回窗口内。
/// ⚠ **用 fmod 而不是"减一次"**：
///
/// * `rate <= 窗口长` 时两者**逐位相同**（`position − start < 窗口长` ⇒
///   `(position − start) % len == position − start`）——即裁决里那条
///   `position -= (loop_end - loop_start)`；
/// * `rate > 窗口长`（极短循环 ＋ 大幅移调）时"减一次"**折不回**窗口内（会一路播到
///   `source_end`，循环退化成长音）；"重复减到窗口内"则有**死循环**风险
///   （`position` 远大于 `len` 时 f32 的 ULP 会把 `-= len` 吸收掉）。
///   `fmod` 的余数**按定义精确**、逐样本**有界**（一次求余）⇒ 两条风险都不在。
// ⚠ 9 个参数是**逐样本热路径**的代价：拆成结构体会在回调里多一次读结构体
// （本仓既有同款豁免见 `SfzVoice::new`）。⭐ `loop_window` 与 `env` 是仅有的**静态**入参
// （触发期/构造期/块首定好），⛔ 不在逐样本路径上构造。
#[allow(clippy::too_many_arguments)]
pub(crate) fn mix_span_into(
    pcm: &[f32],
    position: &mut f32,
    rate: f32,
    gain: f32,
    source_end: u64,
    loop_window: Option<LoopWindow>,
    fade: &mut FadeRamp,
    env: &mut Option<EnvRun>,
    out: &mut [f32],
) -> usize {
    // ⭐ 窗口预换算成 `f32`（逐样本路径⛔ 不做 `u32 → f32` 转换、⛔ 不读结构体）。
    let loop_bounds = loop_window.map(|window| (window.start as f32, window.len() as f32));
    let mut written = 0usize;
    for slot in out.iter_mut() {
        // ⭐ 逐样本的引擎**绝对**帧（包络的释放进度 `k = abs − epoch` 靠它算；
        // ⛔ 不用"本块内的下标"：那会让跨块的释放段重新计时）。
        let abs_frame = env.as_ref().map_or(0, |run| run.abs_base + written as u64);
        if let Some((loop_start, loop_len)) = loop_bounds
            && *position >= loop_start + loop_len
            // ⭐ **回绕闸门**（任务单 `#94` 按规范改）：有包络时，只有
            // [`EnvRun::may_wrap`] 允许才回绕 ⇒ 释放期里 `loop_continuous` 照旧循环、
            // `no_loop` / `loop_sustain` 沿源位置往后播到样本尾。
            // ⚠ 没有包络（`env == None`）⇒ **恒允许**（＝ 接线前的行为，逐位不变）。
            && env.as_ref().is_none_or(|run| run.may_wrap(abs_frame))
        {
            *position = loop_start + (*position - loop_start) % loop_len;
        }
        let index = *position as usize;
        if index as u64 + 1 >= source_end || index + 1 >= pcm.len() {
            break;
        }
        // ⭐ 包络（本片新增）：`None` ⇒ `1.0` ⇒ 下面那个乘法是**恒等**的 `× 1.0`
        // （binary32 下逐位不变 ⇒ 没有 `ampeg_*` 的夹具与接线前一字不差）。
        let env_gain = match env.as_mut() {
            Some(run) => match run.advance(abs_frame) {
                Some(level) => level,
                // 释放段走完 ⇒ 如实停在这里（调用方据 `written < out.len()` 退休声部）。
                None => break,
            },
            None => 1.0,
        };
        let frac = *position - index as f32;
        let sample = pcm[index] * (1.0 - frac) + pcm[index + 1] * frac;
        // 非淡出时 `fade.gain == 1.0` ⇒ `(sample × gain) × 1.0` 与接线前**逐位相同**；
        // 无包络时 `env_gain == 1.0` ⇒ 再乘一次 `1.0` 也**逐位相同**。
        *slot += sample * gain * fade.gain * env_gain;
        *position += rate;
        written += 1;
        if fade.fading && !fade.advance() {
            break;
        }
    }
    written
}

/// 每轨的 **SFZ 声部数上限**（定长池）。
///
/// ⚠ 与 `yeban-sfz` 的**全局 512 声部** `VoicePool` 是**两套形状**：那个池管
/// `group`/`off_by`/`note_polyphony` 一类语义，且它的 `note_on` 走**立即接管**
/// （把 victim 句柄交还调用方、由渲染器对残余输出淡出）；本池只管"本轨有哪几个
/// 采样在响"，victim 选择与淡出都在池**内**（见 [`SfzVoicePool::select_victim`]）。
/// ⚠ 那个池**不被引擎使用**（引擎只从 `yeban-sfz` 借区域选择/速率/增益/`StealFade`）
/// ⇒ 它的 `select_victim` 是**它自己那一份**，⛔ 不构成"引擎里有两份实现"。
pub const SFZ_VOICES_PER_TRACK: usize = 16;

/// ✂️ **实时侧的一个 SFZ 声部**：定长、`Copy`、无指针 [ARCH-RT-001]。
///
/// 一个声部 ＝ **一条匹配区域**（SFZ 里每个 `region` 是一个独立声部 ⇒ 多匹配 ＝ 叠加）。
/// 触发时（[`SfzVoicePool::trigger_note`]）把摊平表里的标量**预计算**进来：
///
/// * `rate` ＝ `region.rate_per_note[note]`（速率预计算 ⇒ ⛔ 逐样本不做除法/`exp`）；
/// * `gain` ＝ `region.gain_at(note, velocity)`（**乐器层**，与摊平口径**逐位相同**）
///   **×** `track_gain`（**引擎层**的轨增益门，归属裁决见台账 `R711`）
///   ⇒ ⭐ 两者**相乘、⛔ 不重复**（引擎⛔ 不碰力度）；
/// * `sample_index` ＝ [`PreparedRegion::sample_index`]（⭐ 步 3：多采样下读**自己**那份 PCM）；
/// * `source_start` / `source_end` ＝ [`PreparedRegion::span_bounds`] 的**同一份**归约
///   （`offset` 起点与 `min(end, 样本播完)` 终点）。
///
/// # 终点口径（⭐ 任务 `#92` 起：**有 `ampeg_*` 时由包络接管**；⭐ `#94` 按规范统一三档）
///
/// * **无 `ampeg_*`（透明包络）**：`stop = min(end_sample, source_end, 样本播完)`
///   —— ⭐ **与任务 `#92` 之前一字不差**（逐位判据 `S37`）；
/// * ⭐ **有 `ampeg_*`**：note-off（＝ `end_sample`）**不再**是硬停，而是**释放段的起点** ——
///   声部继续写（到 `source_end` 或释放走完为止）：
///   * ⭐ **三档（`no_loop` / `loop_sustain` / `loop_continuous`）都就地进释放段**
///     （规范逐字："Whether no_loop, loop_continuous or loop_sustain is set, the duration
///     of the release phase is set using `ampeg_release`."）；
///   * ⭐ 分野只在**释放期要不要继续回绕**（唯一闸门 [`EnvRun::may_wrap`]）：
///     `loop_continuous` ⇒ ⭐ **照旧循环（可以重复多次）**；`no_loop` / `loop_sustain`
///     ⇒ ⛔ 不循环（沿源位置往后播，**"释放走完"与"到样本尾"谁先到谁结束**）；
///   * `ampeg_release` 换算后为 **0 帧** ⇒ 零长释放段 ⇒ 立即退休（＝ 今天的硬停）；
/// * ⭐ **用样本自然结束**：源位置走到 `source_end` 即退休（有包络时这条**仍然**有效：
///   释放段比样本尾巴长 ⇒ 尾巴用完就停）；
/// * ⭐ **`loop_mode=one_shot` 的区域 ⛔ 不退休**（`one_shot` 忽略 note-off，
///   见 [`PreparedRegion::ignores_note_off`]）—— 注意它与 `loop_continuous` 的分别：
///   `one_shot` 连**循环窗口都没有**（[`reduce_loop_window`] 对它恒 `None`）。
/// * ⭐ **`off_by` 关断不是第二条终点口径**：被关掉的声部走的就是上面这几条
///   （`fast` ⇒ 硬停；`normal` / `time` ⇒ 把它自己的 `end_sample` 挪到"现在"），
///   见模块头的"独占组"一节与 [`SfzVoicePool::stop_off_by_victims`]。
///
/// ⭐ **循环由区域（[`PreparedRegion`]）拥有、⛔ 不由声部拥有**：声部只带
/// [`Self::region_index`] 指路 ⇒ `position` ＋ `region_index` 就够（[`Self::sample_index`]
/// **不够**：它指向**去重后的样本**，多条区域可以共用一份样本、却各有各的循环窗口）。
/// ⭐ **包络参数也由区域拥有**（秒 / 百分比）：声部只带**推进状态**
/// （[`Self::envelope_stage`] ＋ [`Self::envelope_counter`] ＝ `u8 ＋ u32` ＝ **5 字节**，
/// 落在既有尾部填充里 ⇒ `SfzVoice` 仍 **40 B**，判据 `S9`）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SfzVoice {
    /// 源采样位置（帧；浮点 ⇒ 分数速率与线性插值）。
    ///
    /// ⭐ 触发时初始化为 [`reduce_span`] 归约出的**源起点**（`offset`，步 3）
    /// ⇒ `offset=N` 的声部从源采样 `N` 起播，⛔ 不是从 0。
    position: f32,
    /// 每样本的源位置增量（触发时从摊平表抄来）。
    rate: f32,
    /// 逐样本增益（触发时算好；见 [`PreparedRegion::gain_at`]）。
    gain: f32,
    /// 触发该声部的音高（诊断与判据用）。
    note: u8,
    /// 触发该声部的力度（诊断与判据用）。
    velocity: u8,
    /// 是否在发声（`false` ⇒ 逐样本循环整段跳过）。
    active: bool,
    /// ⭐ **包络段位**（任务 `#92`；编码见 [`ENV_STAGE_IDLE`] 一族常量，
    /// 公开读数是 [`Self::envelope_stage`]）。
    ///
    /// ⚠ 它落在 `SfzVoice` 既有的**填充字节**里（与 [`Self::env_counter`] 一起 5 字节）
    /// ⇒ 尺寸 **+0 字节**。
    ///
    /// ⚠ `PartialEq` 会**比较它**：段位是声部的可观测状态（同一输入序列下确定），
    /// 与 `position` / `active` 同等对待；没有判据用"两个声部相等"来断言别的东西
    /// （`S9` 用的是 `size_of`，其余判据走 `note()` / `is_active()` 这类读数）。
    env_stage: u8,
    /// ⭐ **包络段内计数**（任务 `#92`）：**自触发起的帧数**（进入释放/等回绕后**冻结**）。
    ///
    /// ⚠ 增量是 `u32`（⛔ 不是 `u16`）：`ampeg_*` 上限 100 秒 ⇒ 48 kHz 上是 4 800 000 帧，
    /// `u16` 装不下（会把长段"回绕"成短段）。落在填充里 ⇒ 尺寸 **+0 字节**。
    env_counter: u32,
    /// ⭐ **该声部播哪一份摊平样本**（[`PreparedSfz::samples`] 的下标；触发时抄自
    /// [`PreparedRegion::sample_index`]）⇒ 多采样下各声部读**自己**那份 PCM。
    sample_index: u16,
    /// ⭐ **该声部来自哪一条摊平区域**（[`PreparedSfz::regions`] 的下标；触发时抄下）。
    ///
    /// 本片（循环接线）新增：循环窗口在**区域**上（[`PreparedRegion::loop_window`]），
    /// 而 [`Self::sample_index`] 只指到**去重后的样本** ⇒ 逐块路径必须能回到区域。
    /// ⚠ 它落在 `SfzVoice` 既有的**尾部填充**里 ⇒ 尺寸 **+0 字节**（`S9` 钉住 40 B）。
    region_index: u16,
    /// **源采样终点（半开）**：见类型文档的"终点口径"。
    source_end: u64,
    /// **引擎侧终点**（绝对样本位置）：来自 [`crate::synth::ScheduledNote::end_sample`]。
    end_sample: u64,
}

impl SfzVoice {
    /// 空闲声部（哨兵；`active == false` ⇒ 不参与渲染）。
    pub const EMPTY: Self = Self {
        position: 0.0,
        rate: 0.0,
        gain: 0.0,
        note: 0,
        velocity: 0,
        active: false,
        env_stage: ENV_STAGE_IDLE,
        env_counter: 0,
        sample_index: PreparedRegion::NO_SAMPLE,
        region_index: 0,
        source_end: 0,
        end_sample: 0,
    };

    /// 组装一个**在发声**的声部（**触发期**调用；⛔ 逐样本路径不调用）。
    ///
    /// `source_start` ＝ [`reduce_span`] 归约出的源起点（`offset`）；`source_end` ＝ 同一份
    /// 归约的终点。两者与摊平表/离线侧**同源** ⇒ 两侧逐样本相同。
    ///
    /// ⚠ 包络状态**不在参数里**：它从 [`ENV_STAGE_IDLE`] ＋ 计数 0 起（区域有包络时由
    /// 逐样本路径第一帧提升为 [`AmpegStage::Attack`]）⇒ ⛔ 本构造器的签名与接线前一致。
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        note: u8,
        velocity: u8,
        rate: f32,
        gain: f32,
        sample_index: u16,
        region_index: u16,
        source_start: f32,
        source_end: u64,
        end_sample: u64,
    ) -> Self {
        Self {
            position: source_start,
            rate,
            gain,
            note,
            velocity,
            active: true,
            env_stage: ENV_STAGE_IDLE,
            env_counter: 0,
            sample_index,
            region_index,
            source_end,
            end_sample,
        }
    }

    /// 是否在发声。
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active
    }

    /// ⭐ **该声部的包络段位**（任务 `#92`）；`None` ＝ 该声部**没有**包络
    /// （区域透明，或还没渲染过第一帧）。
    ///
    /// ⚠ 这是"推进状态真的在动"的**可判读数**（判据 `S35`/`S36` 用它把
    /// "状态推进"与"输出电平"分开钉住）。
    #[must_use]
    pub const fn envelope_stage(&self) -> Option<AmpegStage> {
        env_stage_of(self.env_stage)
    }

    /// ⭐ **该声部的包络段内计数**（自触发起的帧数；释放/等回绕期冻结）。
    #[must_use]
    pub const fn envelope_counter(&self) -> u32 {
        self.env_counter
    }

    /// 触发该声部的音高。
    #[must_use]
    pub const fn note(&self) -> u8 {
        self.note
    }

    /// 触发该声部的力度。
    #[must_use]
    pub const fn velocity(&self) -> u8 {
        self.velocity
    }

    /// ⭐ 该声部播哪一份摊平样本（[`PreparedSfz::samples`] 的下标）。
    #[must_use]
    pub const fn sample_index(&self) -> u16 {
        self.sample_index
    }

    /// ⭐ 该声部来自哪一条摊平区域（[`PreparedSfz::regions`] 的下标）。
    ///
    /// 逐块路径据此取**该区域自己**的循环窗口（[`PreparedRegion::loop_window`]）与
    /// `one_shot` 的"忽略 note-off"（[`PreparedRegion::ignores_note_off`]）。
    #[must_use]
    pub const fn region_index(&self) -> u16 {
        self.region_index
    }

    /// 当前源采样位置（帧）。
    #[must_use]
    pub const fn source_position(&self) -> f32 {
        self.position
    }

    /// 源采样终点（半开）。
    #[must_use]
    pub const fn source_end(&self) -> u64 {
        self.source_end
    }

    /// 引擎侧终点（绝对样本位置）。
    #[must_use]
    pub const fn end_sample(&self) -> u64 {
        self.end_sample
    }

    /// 释放（退休）。⛔ 不是堆释放：只是把状态翻回哨兵 [红线 7]。
    pub(crate) fn silence(&mut self) {
        *self = Self::EMPTY;
    }
}

/// 每轨的 **SFZ 声部池**：定长 `[SfzVoice; 16]`，**全 `Copy`** [ARCH-RT-001]。
///
/// # 纪律（与红线 7 配套）
///
/// * **触发**（[`Self::trigger_note`]）：只在**已有数组**里找最低空闲槽 ⇒ 零分配；
/// * **渲染**（[`Self::render_into`]）：只读 `&PreparedSfz`、原地写 `out` ⇒ 零分配；
/// * **退休**是**池内**状态翻转（`active = false`）⇒ ⛔ 不碰快照退役队列
///   （退役归属不变：旧快照仍由 `crate::snapshot` 的 `RetireProducer` 交回控制线程）；
/// * 池满（> [`SFZ_VOICES_PER_TRACK`] 条区域同时匹配）⇒ ⭐ **窃取**（⛔ 不是丢弃）：
///   按 [`Self::select_victim`] 的三段键选 victim，victim 在自己的槽里走
///   [`FadeRamp`]（3 ms 指数淡出），新声部**挂起**在同一槽、淡出走完即接管
///   （[`Self::pending_count`]）；⛔ 不 panic、⛔ 不扩容。
///   ⭐ **真丢弃**只剩一种情形：某槽的挂起者还没轮到发声就被**再一次**窃取换掉
///   ⇒ 计入 [`Self::dropped`]（累计、单调），⛔ 不静默。
///
/// # ⭐ victim 选择口径（**引擎既有那一套**，与 `yeban_dsp::polysynth::PolySynth` 同形）
///
/// 排序键（字典序，取最小者，见 [`Self::select_victim`]）：
/// `(层级, 触发序号, 槽位下标)`。层级 `0` ＝ **正在淡出**（即 PolySynth 的"已在
/// Release **或**电平 < −60 dBFS"那一档；本池⛔ 无包络、也⛔ 无电平注入 ⇒ 只剩
/// "已在淡出"这一条可判），层级 `1` ＝ 其余在响的声部；同层级取**触发最早**者、
/// 仍并列取**下标最小**者。三级合起来是全序 [ARCH-DET-001]。
///
/// ⚠ `yeban_sfz::voice_pool::VoicePool::select_victim` 是**另一份**（它自己那份：
/// `(层级, 注入电平 dBFS, 触发序号, 下标)`，且走**立即接管**把 victim 句柄交还调用方）
/// —— 那个池**不被引擎使用**（见 [`SFZ_VOICES_PER_TRACK`]）⇒ ⛔ 引擎里没有"两份实现"。
///
/// ⭐ 本条同时裁决了模块头登记的那处"立即接管 vs 额外 slack 槽"：**slack 就在槽内**
/// （`pending` 与 victim 共用同一个槽，与 `PolySynth` 的 `PendingNote` 同款）
/// ⇒ 定容池不需要第 17 个槽，⛔ 也不是"立即接管"。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SfzVoicePool {
    /// 定长声部表（下标稳定 ⇒ 触发分配是确定性的：取最低空闲槽 [ARCH-DET-001]）。
    voices: [SfzVoice; SFZ_VOICES_PER_TRACK],
    /// 每槽**挂起**的待接管声部（淡出窗口里与 victim 共用一个槽 ⇒ 定容池也能同时
    /// 容纳"正在淡出的旧声部"与"刚开始的新声部"，⛔ 不需要额外的 slack 槽）。
    pending: [SfzVoice; SFZ_VOICES_PER_TRACK],
    /// 该槽是否有挂起声部。
    pending_active: [bool; SFZ_VOICES_PER_TRACK],
    /// 每槽的窃取淡出状态（非淡出恒 [`FadeRamp::NONE`]）。
    fade: [FadeRamp; SFZ_VOICES_PER_TRACK],
    /// 每槽最近一次触发的序号（越大越晚；victim 键的第二段）。
    order: [u64; SFZ_VOICES_PER_TRACK],
    /// 下一个触发序号（单调递增、`wrapping_add`）。
    order_counter: u64,
    /// 窃取淡出帧数（默认 3 ms × 采样率；`0` ＝ 硬窃取，判据可注入）。
    steal_fade_frames: u32,
    /// 因池满被**丢弃**的声部数（**累计、单调递增**；与 [`Self::active_count`] 的量规配对）。
    dropped: u64,
    /// 池满时**窃取**（换掉别人）的声部数（**累计、单调递增**）。
    stolen: u64,
}

/// ⭐ **逐块路径的三个区域读数**（[`SfzVoicePool::slot_readings`] 的产物）。
///
/// 全部是**每块一次**求得的静态参数（⛔ 不进逐样本路径、⛔ 不含逐声部状态）。
#[derive(Clone, Copy, Debug, PartialEq)]
struct SlotReadings {
    /// 有效循环窗口（[`PreparedRegion::loop_window`] 的归约结果）。
    loop_window: Option<LoopWindow>,
    /// **生效的引擎终点**：`u64::MAX` ＝「本块不由 `end_sample` 截止」（见 `slot_readings`）。
    limit: u64,
    /// ⭐ **包络眼里的 note-off 帧**：`u64::MAX` ＝「**不理会** note-off」
    /// （`loop_mode=one_shot`，与 [`PreparedRegion::ignores_note_off`] 的既有裁决同源）。
    note_off: u64,
    /// ⭐ note-off 之后是否**继续循环**（`loop_continuous` ＋ 有效窗口；
    /// 见 [`EnvRun::loop_during_release`] 与 [`EnvRun::may_wrap`]）。
    loop_during_release: bool,
    /// 该区域的帧口径包络参数；`None` ＝ 透明包络（⛔ 逐样本不加浮点运算）。
    env: Option<AmpegRun>,
}

impl SfzVoicePool {
    /// 空池（⛔ 无分配：整个池是栈上/结构体内的定长数组）；淡出帧数按 `sample_rate` 定。
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        Self {
            voices: [SfzVoice::EMPTY; SFZ_VOICES_PER_TRACK],
            pending: [SfzVoice::EMPTY; SFZ_VOICES_PER_TRACK],
            pending_active: [false; SFZ_VOICES_PER_TRACK],
            fade: [FadeRamp::NONE; SFZ_VOICES_PER_TRACK],
            order: [0; SFZ_VOICES_PER_TRACK],
            order_counter: 0,
            steal_fade_frames: steal_fade_frames_for(sample_rate),
            dropped: 0,
            stolen: 0,
        }
    }

    /// 空池（48 kHz 的 3 ms 淡出）；引擎侧用 [`Self::new`] 传**真实采样率**。
    #[must_use]
    pub fn empty() -> Self {
        Self::new(48_000.0)
    }

    /// 采样率变化（快照边界）：重算 3 ms 的帧数。
    ///
    /// ⚠ **在响的淡出窗口不重算** —— 它的帧数在窃取那一刻就定下了
    /// （与 `PolySynth` 的 `start_steal_fade` 同一条口径）。
    ///
    /// ⚠ ⭐ **包络的秒→帧不同此理**（如实登记）：`ampeg_*` 的秒是**区域的**参数，
    /// 换算用的是**摊平期**的 `PreparedSfz::engine_hz()`（见
    /// [`PreparedRegion::ampeg_run`]）⇒ 与 `rate_per_note` 同一条纪律
    /// （换采样率必须由控制侧**重新摊平**）。本函数只重算窃取淡出。
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.steal_fade_frames = steal_fade_frames_for(sample_rate);
    }

    /// 覆盖窃取淡出帧数（**判据/注入用**；`0` ＝ 硬窃取）。
    ///
    /// 产物路径由采样率决定；存在的理由是"淡出确实生效"这条判据必须能对着
    /// **硬窃取**变红（与 `PolySynth::set_steal_fade_frames` 同款）。
    pub fn set_steal_fade_frames(&mut self, frames: u32) {
        self.steal_fade_frames = frames;
    }

    /// 当前窃取淡出帧数（默认 3 ms × 采样率）。
    #[must_use]
    pub const fn steal_fade_frames(&self) -> u32 {
        self.steal_fade_frames
    }

    /// ⭐ **为一个音符建声部**：**每一条匹配区域**占一个槽（多匹配 ＝ 叠加）。
    ///
    /// 返回**本次占用空槽的声部数**（0 ＝ 没有区域匹配、匹配的区域不产生采样输出，
    /// 或池满走了窃取 —— 窃取占的是**别人**的槽，⛔ 不计入"新建"）。
    /// 覆盖度：池满时多出来的区域走 [`Self::steal`]（记 [`Self::stolen`]）；
    /// 只有"挂起者被换掉"才是真丢弃（记 [`Self::dropped`]，⛔ 不静默）。
    ///
    /// ⭐ 步 3：每个声部抄下**该区域自己**的 `sample_index`（多采样）与 `span` 归约出的
    /// **源起点**（`offset`）；`NO_SAMPLE` 的区域⛔ 不建声部。
    ///
    /// # ⭐ 独占组／`off_by`（任务单 `#94`；`sfz-needs-6` 的最后一瓣之一）
    ///
    /// 匹配的**每一条**区域在**建自己的声部之前**先跑一次
    /// [`Self::stop_off_by_victims`] ⇒ 语义、来源与边界见模块头的"⭐ `off_by` 关断"一节。
    /// ⚠ 关断**不受**"这条区域是否真的建声部"影响（`end=-1` 的静音区域正是规范给出的
    /// 用法："Marking a region end with -1 can be used to use a silent region to turn off
    /// other regions by using the `group` and `off_by` opcodes."）⇒ 扫描点在
    /// `accepts()` 之后、`sample/span/rate` 那些检查**之前**。
    ///
    /// # ⭐ 轨增益门（`track_gain`；归属裁决见台账 R711）
    ///
    /// 声部增益 ＝ **乐器层**（`region.gain_at`：`volume_gain`／`velocity_gain[128]`／
    /// `key_crossfade`／`velocity_fade_*`／`amp_veltrack`）× **引擎层**（本参数：
    /// 可闻门 × 推子，与鼓机/复音拿到的那一份**同一个** `track_gain_scale` 值）。
    /// ⇒ ⭐ 两者**相乘、⛔ 不重复**：引擎层⛔ 不碰力度（那由乐器层拥有）。
    pub fn trigger_note(
        &mut self,
        plan: &PreparedSfz,
        note: u8,
        velocity: u8,
        end_sample: u64,
        track_gain: f32,
    ) -> usize {
        let n = note as usize;
        let mut created = 0usize;
        // ⭐ **触发前的在场快照**（任务单 `#94` 裁决 5：**触发者不自掐**）：只有本次调用
        // **之前**就 active 的槽位才是 `off_by` 的候选 victim ⇒ 无论本次新建的声部属于
        // 什么 `group`、本次区域的 `off_by` 是多少，都不会被自己这次触发关掉。
        // 语料依据：登记语料 `assets/samples/karoryfer-272-merry-orks/ork_dialogue.sfz`
        // 用的是 `group=1` ＋ `off_by=1`（同组独占的标准写法）；按 ARIA 的自掐解释，
        // 新声部会被自己立刻掐掉 ⇒ 那 20 处会**发不出声**。
        // 机械代价：一个 `u16` 局部量（⛔ 零分配、⛔ 不进任何结构体）。
        let mut active_before: u16 = 0;
        for (index, voice) in self.voices.iter().enumerate() {
            if voice.is_active() {
                active_before |= 1u16 << index;
            }
        }
        for (region_index, region) in plan.regions().iter().enumerate() {
            if !region.accepts(note, velocity) {
                continue;
            }
            // ⭐ `off_by`：在**建自己的声部之前**关掉它点名的那个组（`0` ＝ 不关任何人）。
            if region.off_by != 0 {
                self.stop_off_by_victims(plan, region.off_by, active_before);
            }
            // ⭐ 区域下标进 `u16`（`flatten_regions` 走 `ParseLimits::default()` ⇒
            // 区域数 ≤ `DEFAULT_MAX_REGIONS = 65_536` ⇒ 下标必落在 `u16` 内）。
            let Ok(region_index) = u16::try_from(region_index) else {
                continue;
            };
            // ⭐ 缺样本（`NO_SAMPLE`）⇒ 与 `span_bounds() == None` **同径**：⛔ 不建声部
            // （引擎⛔ 不发明替代内容 —— 那是控制侧的决定；归属裁决见台账 R711 的 `needs-7`）。
            let Some(sample) = plan.sample(region.sample_index) else {
                continue;
            };
            let pcm_len = sample.pcm().len();
            let rate = region.rate_per_note[n];
            if !(rate.is_finite() && rate > 0.0) || pcm_len == 0 {
                continue;
            }
            // ⭐ 起点/终点口径：同一份归约（`span() == None` / 空区间 ⇒ 不建声部）。
            let Some((source_start, source_end)) = region.span_bounds(pcm_len) else {
                continue;
            };
            // ⭐ 引擎层的轨增益门**乘在乐器层增益之外**（两个归属各一份 ⇒ ⛔ 不重复）。
            let gain = region.gain_at(note, velocity) * track_gain;
            let voice = SfzVoice::new(
                note,
                velocity,
                rate,
                gain,
                region.sample_index,
                region_index,
                source_start,
                source_end,
                end_sample,
            );
            match self.voices.iter().position(|voice| !voice.active) {
                Some(index) => {
                    self.order_counter = self.order_counter.wrapping_add(1);
                    self.order[index] = self.order_counter;
                    self.voices[index] = voice;
                    self.pending_active[index] = false;
                    self.fade[index] = FadeRamp::NONE;
                    created += 1;
                }
                // ⭐ 池满 ⇒ **窃取**（sfz-needs-2 的落地点）；⛔ 不再丢弃。
                None => self.steal(voice),
            }
        }
        created
    }

    /// ⭐ **`off_by` 关断**（任务单 `#94`）：把 `active_before` 里那些"其区域的
    /// `group == off_by`"的声部按**它自己的** `off_mode` 关掉。
    ///
    /// # 语义（逐条，全部有出处）
    ///
    /// * 调用方保证 `off_by != 0`；`0` 在格式页的 Practical considerations 里是
    ///   "regions are not muted" ⇒ 本函数对它**一个都不碰**；
    /// * ⭐ **`one_shot` victim 不关**：`loop_mode=one_shot` 的"忽略 note-off"是一条
    ///   **完整**裁决（`#92`／`#94` 两次确认）⇒ `off_by` 是**停止请求**的一种，
    ///   同样被它忽略（⛔ 不造"忽略 note-off 但不忽略 off_by"的半裁决）；
    /// * ⭐ **`fast`（缺省）**："The voice will be turned off immediately. Release settings
    ///   will not have any effect." ⇒ 用**既有**的 [`SfzVoice::silence`] 原语当场硬停
    ///   （⭐ 只翻 `active` 那一个声部：同槽的 `pending`（被窃取后等着接管的新声部）
    ///   与 `fade` **原样保留** ⇒ 它会照常接管，判据 `S41`）；
    /// * ⭐ **`normal` / `time`**："The region will be set into release stage." ⇒ 把 victim 的
    ///   [`SfzVoice::end_sample`] 置 `0` ＝「note-off 就是现在」，剩下**完全**由既有
    ///   note-off 机接手（透明包络 ⇒ 硬停；有包络 ⇒ 释放段；`loop_continuous` ⇒ 释放期
    ///   照旧循环）—— ⛔ 本函数**不新造**任何包络/淡化路径；
    /// * ⭐ **透明包络的 victim 直接走硬停那一支**（即使 `off_mode=normal`）：没有包络就
    ///   没有释放阶段可言（规范那张表只覆盖"进 release"这一种），而走 `end_sample = 0`
    ///   会经 `render_slot` 的 `silence_slot` 分支**连带清掉同槽的 `pending`**。
    /// * ⭐ **已经在释放里的 victim 不重写 `end_sample`**：那会把释放 epoch 篡改成 0
    ///   ⇒ 释放段被当场截断（"set into release stage" 对它已经成立 ⇒ 保持原状）。
    ///
    /// # 分配纪律
    ///
    /// 一次 ≤ [`SFZ_VOICES_PER_TRACK`] 槽的线性扫描（**触发期**一次，⛔ 不进逐样本路径）：
    /// **零分配、零锁、零 I/O**。
    fn stop_off_by_victims(&mut self, plan: &PreparedSfz, off_by: u32, active_before: u16) {
        for index in 0..SFZ_VOICES_PER_TRACK {
            if active_before & (1u16 << index) == 0 || !self.voices[index].is_active() {
                continue;
            }
            let region_index = usize::from(self.voices[index].region_index);
            let Some(victim_region) = plan.regions().get(region_index) else {
                continue;
            };
            if victim_region.group() != off_by || victim_region.ignores_note_off() {
                continue;
            }
            let hard_stop =
                victim_region.off_mode() == OffMode::Fast || victim_region.ampeg_is_transparent();
            if hard_stop {
                // ⭐ 硬停**只翻 `active` 那一个声部**；同槽的 `pending`（被窃取后等着接管的
                // 新声部）**原样保留** ⇒ ⛔ 不丢掉一个合法声部。
                // ⚠ 同时把该槽的 `fade` 归 [`FadeRamp::NONE`]：那个淡出属于**已经不存在**的
                // victim（它被硬停而不是淡出）⇒ 留着它会让 `render_slot` 的"提升"条件
                // （`!fade.is_fading()`）永远不成立（淡出只在 victim 还 active 时才推进）
                // ⇒ 挂起者会**卡死**。归零之后 `render_slot` 第 2 步在**同一块**里就地提升。
                self.voices[index].silence();
                self.fade[index] = FadeRamp::NONE;
            } else if self.voices[index].env_stage & ENV_RELEASING == 0 {
                // 进释放：交给既有 note-off 机（`end_sample = 0` ＝ "note-off 就是现在"）。
                self.voices[index].end_sample = 0;
            }
        }
    }

    /// ⭐ **池满时换掉一个声部**（`trigger_note` 的池满分支）。
    ///
    /// * **软窃取**（`steal_fade_frames > 0`，默认 3 ms）：victim 继续占着自己的槽、
    ///   进入 [`FadeRamp`] 淡出；新声部**挂起**在同一槽的 `pending` 上，淡出走完
    ///   由 [`Self::render_into`] 就地接管 ⇒ "旧声部淡出"与"新声部起音"**永不同时
    ///   发声**（不可能叠加爆音），且定容池不需要额外的槽。
    /// * **硬窃取**（帧数 `0`，仅注入路径）：立即接管，⛔ 不留淡出。
    ///
    /// ⚠ 若选中的 victim **已经有挂起者**（还没轮到发声），那个挂起者被换掉
    /// ⇒ 它是一次**真丢弃**：计入 [`Self::dropped`]（⛔ 不静默）。
    fn steal(&mut self, voice: SfzVoice) {
        let Some(victim) = self.select_victim() else {
            // 不可达：容量 ≥ 1 且"池满"意味至少一个槽 active。
            self.dropped = self.dropped.saturating_add(1);
            return;
        };
        self.order_counter = self.order_counter.wrapping_add(1);
        let order = self.order_counter;
        let fade = FadeRamp::new(self.steal_fade_frames);
        self.stolen = self.stolen.saturating_add(1);
        if !fade.is_fading() {
            self.voices[victim] = voice;
            self.order[victim] = order;
            self.pending_active[victim] = false;
            self.fade[victim] = FadeRamp::NONE;
            return;
        }
        if self.pending_active[victim] {
            // 上一位挂起者还没发声就被换掉 ⇒ 真丢弃（⛔ 不静默）。
            self.dropped = self.dropped.saturating_add(1);
        }
        self.pending[victim] = voice;
        self.pending_active[victim] = true;
        self.order[victim] = order;
        self.fade[victim] = fade;
    }

    /// ⭐ **确定性窃取目标**（同一输入序列永远同一结果 [ARCH-DET-001]）。
    ///
    /// 排序键（字典序，取最小者；见类型文档的"victim 选择口径"）：
    /// `(层级, 触发序号, 槽位下标)`；层级 `0` ＝ 正在淡出，`1` ＝ 其余。
    /// 全程线性扫描、零分配；池空 ⇒ `None`（调用方按"不可达"计数，⛔ 不 panic）。
    #[must_use]
    pub fn select_victim(&self) -> Option<usize> {
        let mut best: Option<(u8, u64, u32)> = None;
        let mut best_index = None;
        for (index, voice) in self.voices.iter().enumerate() {
            if !voice.active {
                continue;
            }
            // `0` = 正在淡出（优先被换）；`1` = 其余在响的声部。
            let key = (
                u8::from(!self.fade[index].is_fading()),
                self.order[index],
                index as u32,
            );
            if best.is_none_or(|current| key < current) {
                best = Some(key);
                best_index = Some(index);
            }
        }
        best_index
    }

    /// 渲染本池的**全部活声部**到 `out`（`position` ＝ 本量子的**引擎绝对**起点）。
    ///
    /// **零分配**：只读 `plan` 的样本 PCM 与已有数组、原地写 `out` [MUST-GATE-001]。
    /// ⭐ 每个声部读**它自己**那份样本（[`SfzVoice::sample_index`]）。
    /// ⭐ 正在淡出的槽在**本块内**走完淡出时就地接管挂起者、用同一段
    /// [`mix_span_into`] 继续写剩下的帧（帧精确、⛔ 不留一个量子的空档）。
    pub fn render_into(&mut self, plan: &PreparedSfz, position: u64, out: &mut [f32]) {
        for index in 0..SFZ_VOICES_PER_TRACK {
            self.render_slot(index, plan, position, out);
        }
    }

    /// 渲染**一个槽**（`render_into` 的逐槽体；拆出来是为了借用清晰）。
    ///
    /// ⭐ **循环接线（任务 `#91`）的两件事在这里**：① 逐块路径按
    /// [`SfzVoice::region_index`] 回到**该声部来源的那条区域**取循环窗口
    /// （[`PreparedRegion::loop_window`]）；② `loop_mode=one_shot` 的声部把
    /// `end_sample` 当成**无穷远**（[`PreparedRegion::ignores_note_off`]）⇒
    /// note-off ⛔ 不再硬切它。
    ///
    /// ⭐ **幅度包络（任务 `#92`）的三件事也在这里**（都是**每块一次**的结构体读取，
    /// ⛔ 逐样本路径不做额外查表）：
    ///
    /// 1. 按区域求 [`AmpegRun`]（秒 × `plan.engine_hz()` ⇒ 帧）；
    /// 2. 有包络且**会进释放段**的区域 ⇒ 本块的**截止帧不再取 `end_sample`**
    ///    （否则释放段会被 note-off 当场切断）—— 见 [`Self::slot_readings`]；
    /// 3. 把声部的推进状态（段位 / 计数 / note-off↔epoch）按需交给 [`EnvRun`]，
    ///    写完再**写回**（跨块延续）。
    fn render_slot(&mut self, index: usize, plan: &PreparedSfz, position: u64, out: &mut [f32]) {
        let mut offset = 0usize;
        let mut victim_stopped = false;
        // --- 1) 淡出段：victim 在自己的窗口里继续发声，逐帧乘衰减增益 ---
        if self.voices[index].active && self.fade[index].is_fading() {
            let voice = self.voices[index];
            let Some(readings) = Self::slot_readings(plan, voice, plan.engine_hz()) else {
                // 摊平表与池不匹配（例如热更了另一份表）⇒ 如实退休。
                self.silence_slot(index);
                return;
            };
            let limit = readings
                .limit
                .saturating_sub(position)
                .min(out.len() as u64) as usize;
            if limit == 0 {
                // 引擎终点先到 ⇒ 本槽（含挂起者）退休，⛔ 不静默地继续跑。
                self.silence_slot(index);
                return;
            }
            let Some(sample) = plan.sample(voice.sample_index) else {
                self.silence_slot(index);
                return;
            };
            let mut local = voice.position;
            let mut fade = self.fade[index];
            let mut env = readings.env.map(|run| {
                EnvRun::new(
                    run,
                    voice.env_stage,
                    voice.env_counter,
                    readings.note_off,
                    position,
                    readings.loop_during_release,
                )
            });
            let written = mix_span_into(
                sample.pcm(),
                &mut local,
                voice.rate,
                voice.gain,
                voice.source_end,
                readings.loop_window,
                &mut fade,
                &mut env,
                &mut out[..limit],
            );
            // 淡出中的 victim ⛔ 不回写包络状态（它马上就被挂起者接管）；位置/淡出照旧回写。
            self.voices[index].position = local;
            self.fade[index] = fade;
            offset = written;
            // 源/引擎终点在淡出走完之前先到 ⇒ victim 已经静音（见下面第 2 步）。
            victim_stopped = written < limit;
        }
        // --- 2) 提升：淡出走完 **或** victim 提前静音 ⇒ 挂起者接管这一槽 ---
        if self.pending_active[index] && (!self.fade[index].is_fading() || victim_stopped) {
            self.voices[index] = self.pending[index];
            self.pending_active[index] = false;
            self.fade[index] = FadeRamp::NONE;
        }
        // --- 3) 正常段：本槽现在的声部写本块剩余帧 ---
        let voice = self.voices[index];
        if !voice.active {
            return;
        }
        let Some(readings) = Self::slot_readings(plan, voice, plan.engine_hz()) else {
            self.silence_slot(index);
            return;
        };
        // 引擎终点（音符先到终点 ⇒ 本量子只渲染到它为止）。
        // ⚠ 有包络的声部这里通常是 `u64::MAX`（note-off 已交给包络 ⇒ ⛔ 不在这里切）。
        if readings.limit <= position.saturating_add(offset as u64) {
            self.silence_slot(index);
            return;
        }
        let remaining = readings
            .limit
            .saturating_sub(position)
            .min(out.len() as u64) as usize;
        let Some(sample) = plan.sample(voice.sample_index) else {
            self.silence_slot(index);
            return;
        };
        let mut local = voice.position;
        let mut fade = self.fade[index];
        let mut env = readings.env.map(|run| {
            EnvRun::new(
                run,
                voice.env_stage,
                voice.env_counter,
                readings.note_off,
                position + offset as u64,
                readings.loop_during_release,
            )
        });
        let written = mix_span_into(
            sample.pcm(),
            &mut local,
            voice.rate,
            voice.gain,
            voice.source_end,
            readings.loop_window,
            &mut fade,
            &mut env,
            &mut out[offset..remaining],
        );
        self.voices[index].position = local;
        self.fade[index] = fade;
        if let Some(run) = env {
            // ⭐ 包络状态**跨块延续**：段位 / 段内计数 / note-off↔释放 epoch。
            run.write_back(&mut self.voices[index]);
        }
        // ⭐ 用样本自然结束：源位置走完 ⇒ 退休。⚠ 有循环窗口时这一条**不可达**
        // （回绕把位置压在窗口内）⇒ 循环声部只由 `end_sample`（无包络）
        // 或**包络的释放段走完**（有包络）终止。
        if written < remaining - offset {
            self.silence_slot(index);
        }
    }

    /// ⭐ **逐块路径的三个区域读数**（任务 `#92` 起）：有效循环窗口 ／ 生效的引擎终点 ／
    /// 包络参数（＋ 释放期要不要继续循环）。
    ///
    /// * 区域查不到（池与摊平表不匹配）⇒ `None` ⇒ 调用方**如实退休**（与
    ///   [`PreparedSfz::sample`] 落空同径）；
    /// * 循环窗口按**该声部自己那份 PCM** 的长度归约（[`PreparedRegion::loop_window`]）；
    /// * ⭐ **终点**（任务单 `#94` 按 `sfzformat` 改）：
    ///   * `one_shot`（[`PreparedRegion::ignores_note_off`]）⇒ `u64::MAX`
    ///     ＝「note-off ⛔ 不切它」（循环接线片既有裁决，⭐ 本片不动）；
    ///   * ⭐ **有包络** ⇒ `u64::MAX` ＝「note-off 由**包络**接管，⛔ 不在这里硬切」——
    ///     规范逐字（<https://sfzformat.com/opcodes/loop_mode/>）："Whether `no_loop`,
    ///     `loop_continuous` or `loop_sustain` is set, the duration of the release phase is
    ///     set using `ampeg_release`." ⇒ **三档都是同一件事**（旧口径把 `loop_continuous`
    ///     排除在这条之外，已按规范撤掉）；
    ///   * 其余（**透明包络**）⇒ 原样带出 `voice.end_sample` ＝ 今天的硬停
    ///     （没有释放阶段可言 ⇒ 不在规范那张表里）。
    /// * ⭐ `loop_during_release`：`loop_continuous` **且**窗口有效 ⇒ `true`（释放期照旧回绕；
    ///   规范逐字："the loop will repeat if the loop end is reached during the release
    ///   phase, including repeating multiple times if the release time is longer than the
    ///   loop length."）；`no_loop` / `loop_sustain` ⇒ `false`（"During the release phase,
    ///   there's no looping." ＋ `legacy` 的 "The rest of the sample will play after note
    ///   release."）。
    /// * ⭐ `note_off`：包络眼里的 note-off 帧（`one_shot` ⇒ `u64::MAX` ＝ 不理会）。
    fn slot_readings(plan: &PreparedSfz, voice: SfzVoice, engine_hz: f32) -> Option<SlotReadings> {
        let region = plan.regions().get(voice.region_index as usize)?;
        let pcm_len = plan.sample(voice.sample_index)?.pcm().len();
        let loop_window = region.loop_window(pcm_len);
        let env = region.ampeg_run(engine_hz);
        let end_sample = if region.ignores_note_off() || env.is_some() {
            u64::MAX
        } else {
            voice.end_sample
        };
        Some(SlotReadings {
            loop_window,
            limit: end_sample,
            // ⭐ `one_shot` 连**包络的释放段**也不许进（"忽略 note-off"是一条完整裁决：
            // 既不改退休，也不起释放）⇒ 给它 `u64::MAX`。其余模式把引擎终点原样交给包络
            // （释放已开始时该字段是**释放 epoch** —— 语义由声部的段位决定，见 [`EnvRun`]）。
            note_off: if region.ignores_note_off() {
                u64::MAX
            } else {
                voice.end_sample
            },
            loop_during_release: matches!(region.loop_mode(), LoopMode::LoopContinuous)
                && loop_window.is_some(),
            env,
        })
    }

    /// 释放**一个**槽：声部与挂起者一起退休（淡出状态归 [`FadeRamp::NONE`]）。
    fn silence_slot(&mut self, index: usize) {
        self.voices[index].silence();
        self.pending_active[index] = false;
        self.fade[index] = FadeRamp::NONE;
    }

    /// 释放**全部**声部（`seek` / 换音源 ⇒ ⛔ 不留上一件音源的尾巴）。
    ///
    /// ⚠ 保留 [`Self::dropped`] / [`Self::stolen`]：它们是**累计**读数（与"活声部数"
    /// 这张量规配对），与 `DrumMachine::reset` 保留 `drum_hits` 同口径。
    pub fn silence(&mut self) {
        for index in 0..SFZ_VOICES_PER_TRACK {
            self.silence_slot(index);
        }
    }

    /// 活声部数（**量规**：可升可降；淡出中的声部**仍计**）。
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.voices.iter().filter(|voice| voice.active).count()
    }

    /// 挂起声部数（**量规**：某个槽正在淡出、新声部等接管时的占用数）。
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.pending_active.iter().filter(|active| **active).count()
    }

    /// 因池满被丢弃的声部数（**累计**，单调递增）。
    ///
    /// ⚠ 窃取**不**计入这里（那是 [`Self::stolen`]）；只有"挂起者被换掉"这类
    /// **真丢弃**才计入（⛔ 不静默）。
    #[must_use]
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    /// 池满时**窃取**（换掉别人）的声部数（**累计**，单调递增）。
    #[must_use]
    pub const fn stolen(&self) -> u64 {
        self.stolen
    }

    /// 某槽剩余的窃取淡出帧数（`0` ＝ 不在淡出；下标越界也返回 `0`）。
    #[must_use]
    pub fn fade_remaining_at(&self, index: usize) -> u32 {
        self.fade.get(index).map_or(0, |fade| {
            if fade.is_fading() {
                fade.remaining()
            } else {
                0
            }
        })
    }

    /// 只读声部表（判据用）。
    #[must_use]
    pub const fn voices(&self) -> &[SfzVoice; SFZ_VOICES_PER_TRACK] {
        &self.voices
    }
}

/// ⭐ **一条轨的"待摊平"SFZ 绑定**：只带路径、**零运行时对象**（任务单 `#78`）。
///
/// # 它补的是哪一段（引擎侧只做接线、⛔ 不读盘）
///
/// 模型层现在能表达"这条轨用哪一份 SFZ 定义文件"（`TrackV3.sampler:
/// Option<SamplerBinding>`，形状见 `yeban-model` 的 `project.rs`，登记在台账 `R731`）
/// —— ⚠ 而它**只留痕一个路径字符串**、⛔ 不带任何已解析乐器或 PCM。
///
/// 摊平（`SfzInstrument` → 区域表 ＋ PCM）需要**读盘 ＋ 解码**，而本仓有两条既有边界
/// 把它钉在控制侧 ⇒ 引擎侧能把 `sampler.path` 变成的**只有这一件事**：
///
/// * ⛔ **引擎不读磁盘**（真读 PCM 的是控制侧，形状见 `crates/yeban-app/src/sfz_load.rs`）；
/// * ⛔ **引擎不引 `symphonia`／`rubato`**（解码器的依赖⛔ 不进实时 crate）。
///
/// # 分工（本类型就是两个阶段之间的那张"待办条"）
///
/// ```text
/// from_project(&project)            ⇒  sfz_pending(): "这条轨有一份 SFZ，路径是它"
///                                       ↓  控制侧（允许分配、允许 I/O，⛔ 不在音频线程）
///                                       SfzInstrument::load(读盘)
///                                       referenced_sample_paths() → 逐个解码
///                                       SfzPlan::prepare_with_samples(...)
/// with_sfz(track, plan)             ⇒  sfz(): 摊平表（并**移除**该轨的待摊平项）
/// ```
///
/// ⚠ **实时侧完全不看本类型**：`SynthEngine` / `EngineRuntime` 只读
/// [`crate::snapshot::EngineSnapshot::sfz`] 那一张摊平表 ⇒ `sampler: None` 与
/// `sampler: Some(…)` 的**渲染输出逐位相同**（引擎没有第二份音源事实源）。
///
/// ⚠ 字段**原样**保留模型给的字节：⛔ 不查存在性、⛔ 不查扩展名、⛔ 不做路径规范化
/// （与 `SamplerBinding::path` 同一口径，"留痕"就是它的全部语义）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingSfz {
    /// 采样器定义文件路径（逐字留痕；见本类型的模块级说明）。
    path: String,
}

impl PendingSfz {
    /// 从一个路径建（控制侧投影用：把 `TrackV3.sampler.path` 原样搬过来）。
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }

    /// 路径（**逐字**，⛔ 未被规范化）。
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }
}

/// ✂️ **一条轨的 SFZ 投影**：控制侧构造期建好、实时侧只读。
///
/// 它存在的理由与 [`crate::insert::ConvolutionPlan`] 同一句：**模型层放不下样本数组**
/// （`ParameterValue` 只有 `name`/`value`/`unit` 三个字段）⇒ "SFZ 文本 → 区域表 ＋ PCM"
/// 的摊平必须发生在**控制线程的构造期**，结果随快照一起送到音频线程。
///
/// # 分配纪律（红线 7 / [MUST-GATE-001]）
///
/// * [`Self::prepare`] / [`Self::prepare_with_samples`]：**允许分配**（建区域表与样本 PCM）；
/// * ⛔ **不在快照边界摊平**：快照边界（`SynthEngine::begin_snapshot` / `arm_from_snapshot`）
///   一个字都不碰它 —— 那两条路径会分配（见 `crate::synth::SynthEngine::arm_from_snapshot`）；
/// * 实时侧只**借用** [`Self::prepared`] 的 `&PreparedSfz`（**PCM 借用快照**），
///   ⛔ 不 clone、⛔ 不 `Arc` clone 那些 `Vec`。
///
/// ⚠ **轨↔SFZ 的绑定不由模型给**（模块头 sfz-needs-5）⇒ 只能由引擎 API 显式注入
/// （[`crate::snapshot::EngineSnapshot::with_sfz`]）。⭐ 步 3 起这条公开路径**可用多采样**
/// 走通：`referenced_sample_paths` →（控制侧解码）→ [`Self::prepare_with_samples`] →
/// `EngineSnapshot::with_sfz`（判据见 `tests/sfz_instrument.rs` 的 S11/S12）。
#[derive(Clone, Debug, PartialEq)]
pub struct SfzPlan {
    /// 重数据：摊平后的区域表 ＋ PCM（控制侧构造、实时侧只读）。
    prepared: PreparedSfz,
}

impl SfzPlan {
    /// 控制侧摊平（**允许分配**）：见 [`PreparedSfz::prepare`] 的口径（**单样本广播**）。
    ///
    /// # Errors
    /// 文本非法 ⇒ [`SfzError`]（与 [`PreparedSfz::prepare`] 同）。
    pub fn prepare(
        instrument: &SfzInstrument,
        source: &InMemorySample,
        engine_hz: f32,
    ) -> Result<Self, SfzError> {
        Ok(Self {
            prepared: PreparedSfz::prepare(instrument, source, engine_hz)?,
        })
    }

    /// ⭐ **控制侧多采样摊平**：把 `[(sample_path, 已解码样本)]` 交给
    /// [`PreparedSfz::prepare_with_samples`]（去重 ＋ 缺样本**静音** ＋ 计数）。
    ///
    /// 这是**绑定注入 API 的公开入口**：调用方建好本 plan 之后，用
    /// [`crate::snapshot::EngineSnapshot::with_sfz`] 把它注入某一条轨的快照 —— 全仓**唯一**
    /// 的"轨↔SFZ"绑定路径（模型层今天仍**没有**这条语义 ⇒ sfz-needs-5）。
    ///
    /// # Errors
    /// 文本非法 ⇒ [`SfzError`]（与 [`PreparedSfz::prepare_with_samples`] 同）。
    pub fn prepare_with_samples(
        instrument: &SfzInstrument,
        samples: &[(String, InMemorySample)],
        engine_hz: f32,
    ) -> Result<Self, SfzError> {
        Ok(Self {
            prepared: PreparedSfz::prepare_with_samples(instrument, samples, engine_hz)?,
        })
    }

    /// 从**已经摊平**的表直接建（控制侧；判据与"摊平与控制侧分离"的场景用）。
    #[must_use]
    pub const fn from_prepared(prepared: PreparedSfz) -> Self {
        Self { prepared }
    }

    /// 实时侧只读的摊平表 —— `SynthEngine::render_track` 的 `sfz` 参数就是它。
    #[must_use]
    pub const fn prepared(&self) -> &PreparedSfz {
        &self.prepared
    }

    /// 区域数（控制侧读数）。
    #[must_use]
    pub fn region_count(&self) -> usize {
        self.prepared.region_count()
    }

    /// 摊平占用的**内存字节数**（读数用：区域表 ＋ **全部**样本 PCM ＋ 样本路径文本）。
    #[must_use]
    pub fn memory_bytes(&self) -> usize {
        self.prepared.memory_bytes()
    }

    /// ⭐ **逐项字节读数**（转交 [`PreparedSfz::byte_usage`]：区域表 ／ PCM ／ 路径文本）。
    ///
    /// ⭐ 这是**控制侧**真正拿得到的读数形状（`SfzPlan` 才是公开的绑定入口）。
    #[must_use]
    pub fn byte_usage(&self) -> SfzByteUsage {
        self.prepared.byte_usage()
    }

    /// ⭐ **字节预算判据**（转交 [`PreparedSfz::budget_overrun`]）。
    ///
    /// ⚠ 超预算**不会**让本 plan 失效：⛔ 不报错、⛔ 不截断、⛔ 不改摊平结果；
    /// 归属裁决（"引擎只报数、控制侧决定"）与三条理由见 [`PreparedSfz::budget_overrun`]。
    #[must_use]
    pub fn budget_overrun(&self, budget: &SfzByteBudget) -> SfzBudgetOverrun {
        self.prepared.budget_overrun(budget)
    }
}

/// 掩码里**第一个置位**的下标（`[u64; 2]` 的低半在前）⇒ `0..=127`；全 0 时 `None`。
///
/// 只在构造期调用（`prepare`），⛔ 不在回调路径上。
fn first_set_bit(mask: &[u64; 2]) -> Option<u8> {
    for (half, word) in mask.iter().enumerate() {
        if *word != 0 {
            let bit = word.trailing_zeros() as usize;
            return u8::try_from(half * 64 + bit).ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 最小 SFZ：两条区域（键范围不同）。
    const MINIMAL: &str = "<region> sample=a.wav key=60\n<region> sample=b.wav key=64\n";

    /// [ROAD-M2-005] 判据 ①：`load` 一段最小文本 ⇒ `region_count` 等于期望值。
    #[test]
    fn load_records_the_region_count() {
        let instrument = SfzInstrument::load(MINIMAL.to_owned(), &ParseLimits::default())
            .expect("最小 SFZ 必须解析成功");
        assert_eq!(instrument.region_count(), 2, "两条 `<region>` ⇒ 区域数 2");
        // 单区域对照（⛔ 防止把"1"写死成常量也能过）
        let one = SfzInstrument::load(
            "<region> sample=a.wav key=60\n".to_owned(),
            &ParseLimits::default(),
        )
        .expect("单区域必须解析成功");
        assert_eq!(one.region_count(), 1, "单区域 ⇒ 1");
    }

    /// [ROAD-M2-005] 判据 ②：`with_instrument` 能在给定 note/velocity 下查到区域。
    #[test]
    fn regions_matching_selects_by_key_and_velocity() {
        let instrument =
            SfzInstrument::load(MINIMAL.to_owned(), &ParseLimits::default()).expect("解析必须成功");
        assert_eq!(
            instrument.regions_matching(60, 100).expect("查询"),
            vec![0],
            "note 60 ⇒ 第 0 条"
        );
        assert_eq!(
            instrument.regions_matching(64, 100).expect("查询"),
            vec![1],
            "note 64 ⇒ 第 1 条"
        );
        assert!(
            instrument
                .regions_matching(72, 100)
                .expect("查询")
                .is_empty(),
            "无匹配键 ⇒ 空（⛔ 不是 panic、也不是默认第一条）"
        );
        // `with_instrument` 本身：闭包内能读到区域表
        let paths = instrument
            .with_instrument(|i| {
                i.regions()
                    .iter()
                    .map(|region| region.sample_path().to_string())
                    .collect::<Vec<_>>()
            })
            .expect("闭包式访问");
        assert_eq!(paths, vec!["a.wav".to_owned(), "b.wav".to_owned()]);
    }

    /// [ROAD-M2-005] 判据 ③：非法/超限 SFZ ⇒ 返回 [`SfzError`]，⛔ **不 panic**。
    ///
    /// ⭐ **实测登记（本节的重要读数）**：空文本、纯空白、以及坏指令 `<region> ???`
    /// 都被 `yeban-sfz` **接受**为 `Ok(0 区域)` —— 库把无法识别的 opcode 当作**忽略**而非错误。
    /// ⛔ 这不是我的缺陷、也不该由我在引擎侧"补一个错误"（那是**发明规范**）；
    /// 因此本判据分两半：
    ///   ① **不 panic**：三类退化输入都必须**返回**（`Ok` 或 `Err` 皆可）；
    ///   ② **必然报错**的机械证据：用**超限**输入（`max_line_bytes` 极小）⇒ 必须 `Err`。
    #[test]
    fn invalid_or_empty_sfz_returns_an_error_without_panicking() {
        // ① 退化输入：只要求"不 panic"（实测三者皆 Ok(0)，如实打印）
        for (label, text) in [
            ("空文本", ""),
            ("纯空白", "   \n\t\n"),
            ("坏指令", "<region> ???\n"),
        ] {
            let result = SfzInstrument::load(text.to_owned(), &ParseLimits::default());
            println!(
                "[sfz] {label}: {:?}",
                result.as_ref().map(SfzInstrument::region_count)
            );
            if let Ok(instrument) = result {
                assert_eq!(instrument.region_count(), 0, "{label} 若成功则区域数应为 0");
            }
        }
        // ② 超限输入 ⇒ **必须 Err**（库自己的判据也用这一口径）
        let tiny = ParseLimits {
            max_line_bytes: 8,
            ..ParseLimits::default()
        };
        let over = SfzInstrument::load("<region> sample=abcdefgh.wav key=60\n".to_owned(), &tiny);
        println!(
            "[sfz] 超限输入: {:?}",
            over.as_ref().map(SfzInstrument::region_count)
        );
        assert!(
            over.is_err(),
            "超限行必须返回 SfzError（这才是'非法 ⇒ 报错'的机械证据）"
        );
        // ③ 正常 limits 下同一条文本必须**成功**（反向对照：证明上面的 Err 来自 limits 而非文本本身）
        let normal = SfzInstrument::load(
            "<region> sample=abcdefgh.wav key=60\n".to_owned(),
            &ParseLimits::default(),
        );
        assert!(
            normal.is_ok(),
            "同一文本在默认 limits 下必须成功（否则上面的 Err 另有原因）"
        );
    }

    /// ⭐ 本任务的核心判据 a：**某音符触发 ⇒ 输出非静音**，并且与"不触发"**可区分**。
    ///
    /// 样本源是**内存里的极短正弦**（本仓尚无采样文件读取 ⇒ 样本由调用方提供）。
    /// 通路里的区域选择、播放速率、总增益**全部来自 `yeban-sfz`**（⛔ 引擎不重写）。
    #[test]
    fn triggering_a_region_produces_non_silence_and_is_distinguishable() {
        const SFZ: &str = "<region> sample=a.wav key=60 pitch_keycenter=60\n";
        let instrument = SfzInstrument::load(SFZ.to_owned(), &ParseLimits::default())
            .expect("最小 SFZ 必须成功");
        let sample = InMemorySample::sine(440.0, 48_000.0, 480);

        // ① 命中的音符（key=60）⇒ 非静音
        let mut hit = vec![0.0f32; 512];
        let written = instrument
            .render_note_into(&sample, 60, 100, &mut hit, 48_000.0)
            .expect("渲染不得报错");
        let energy: f32 = hit.iter().map(|s| s * s).sum();
        println!("[sfz] key=60: 写入 {written} 帧，能量 = {energy:.6}");
        assert!(written > 0, "命中区域必须写入样本帧（实得 {written}）");
        assert!(energy > 1e-6, "命中区域必须**非静音**（能量 {energy}）");

        // ② 不命中的音符（key=72）⇒ 静音 ⇒ ⭐ 与①**可区分**
        let mut miss = vec![0.0f32; 512];
        let written_miss = instrument
            .render_note_into(&sample, 72, 100, &mut miss, 48_000.0)
            .expect("不命中也不得报错");
        let energy_miss: f32 = miss.iter().map(|s| s * s).sum();
        println!("[sfz] key=72: 写入 {written_miss} 帧，能量 = {energy_miss:.6}");
        assert_eq!(written_miss, 0, "不命中的音符不得写入帧");
        assert_eq!(energy_miss, 0.0, "不命中的音符必须是静音");
        assert!(energy > energy_miss, "⭐ 可区分：命中的能量必须大于不命中");
    }

    /// ⭐ 新判据：**`trigger=release` 的区域不进 `note-on` 掩码**。
    ///
    /// 口径：摊平表建表时用 `Region::matches_query`（与 `Instrument::region_for_with`
    /// 同一个谓词）过门控；门控不过 ⇒ **两张掩码都清 0** ⇒ 回调侧的
    /// [`PreparedRegion::accepts`] 永远为假。离线侧同此口径。
    ///
    /// 反向对照：该 release 区域在**键/力度**上确实匹配（用 `matches_key`/`matches_velocity`
    /// 直接读回）⇒ 掩码全 0 只可能来自**门控**，⛔ 不是"它本来就不匹配"。
    #[test]
    fn trigger_release_never_enters_the_note_on_mask() {
        const SFZ: &str = "<region> sample=attack.wav key=60 pitch_keycenter=60\n\
                           <region> sample=release.wav key=60 pitch_keycenter=60 trigger=release\n";
        let instrument =
            SfzInstrument::load(SFZ.to_owned(), &ParseLimits::default()).expect("解析必须成功");
        let sample = InMemorySample::sine(440.0, 48_000.0, 480);
        let prepared = PreparedSfz::prepare(&instrument, &sample, 48_000.0).expect("摊平");

        assert_eq!(
            prepared.region_count(),
            2,
            "两条区域都要摊平进来（⛔ 不是把 release 那条丢弃）"
        );
        let attack = &prepared.regions()[0];
        let release = &prepared.regions()[1];
        // ① 攻击区域：note-on 掩码里有键 60
        assert!(attack.accepts(60, 100), "attack 区域必须在 note-on 掩码里");
        assert_ne!(attack.key_mask(), [0u64; 2], "attack 的键掩码必须非空");
        // ② release 区域：两张掩码**全 0** ⇒ 永不进 note-on 命中集
        assert_eq!(
            release.key_mask(),
            [0u64; 2],
            "⭐ trigger=release 的键掩码必须全 0（不进 note-on 掩码）"
        );
        assert_eq!(
            release.velocity_mask(),
            [0u64; 2],
            "⭐ trigger=release 的力度掩码必须全 0"
        );
        assert!(
            !release.accepts(60, 100),
            "trigger=release 不得被 note-on 接受"
        );

        // ③ 反向对照：它**在键/力度上确实匹配** ⇒ 全 0 只能来自门控
        let raw_match = instrument
            .with_instrument(|parsed| {
                let region = &parsed.regions()[1];
                region.matches_key(60) && region.matches_velocity(100)
            })
            .expect("闭包式访问");
        assert!(
            raw_match,
            "该 release 区域在键/力度上**确实**匹配 ⇒ 掩码清 0 只能来自 trigger 门控"
        );

        // ④ 离线侧同一口径：只剩 release 区域时 note-on 必须**一帧都不写**
        let only_release = SfzInstrument::load(
            "<region> sample=release.wav key=60 pitch_keycenter=60 trigger=release\n".to_owned(),
            &ParseLimits::default(),
        )
        .expect("解析必须成功");
        let mut out = vec![0.0f32; 64];
        let written = only_release
            .render_note_into(&sample, 60, 100, &mut out, 48_000.0)
            .expect("渲染不得报错");
        assert_eq!(written, 0, "只有 release 区域时 note-on 必须写 0 帧");
        assert!(
            out.iter().all(|s| *s == 0.0),
            "只有 release 区域时 note-on 必须静音"
        );
        // 对照臂：同一份样本在 fixture（含 attack）里必须**非静音**，否则上面是空转
        let mut both = vec![0.0f32; 64];
        let written_both = instrument
            .render_note_into(&sample, 60, 100, &mut both, 48_000.0)
            .expect("渲染不得报错");
        assert!(
            written_both > 0 && both.iter().any(|s| *s != 0.0),
            "含 attack 区域的 fixture 必须非静音（否则本判据是空转）"
        );
    }

    /// ⭐ 判据（本片新增）：摊平增益 = `PlaybackSpec::total_gain`，**全 128×128 网格逐位相同**。
    ///
    /// 场景刻意同时给出**键轴**与**力度轴**的交叉淡化（in 与 out 都有），
    /// 用来钉住"键轴 / 力度淡入 / 力度淡出分开摊平、回调侧按同一结合序相乘"这条口径：
    /// 若把力度淡入与淡出并成一张表（`K * (in * out)`），binary32 下会与
    /// `(K * in) * out` 不同值 ⇒ 本判据会红。
    #[test]
    fn flattened_gain_equals_total_gain_over_the_whole_grid() {
        const XF: &str = "<region> sample=a.wav key=60 pitch_keycenter=60 volume=-3 \
             xfin_lokey=48 xfin_hikey=72 xfout_lokey=48 xfout_hikey=72 \
             xfin_lovel=40 xfin_hivel=90 xfout_lovel=60 xfout_hivel=110\n";
        let instrument =
            SfzInstrument::load(XF.to_owned(), &ParseLimits::default()).expect("解析必须成功");
        let sample = InMemorySample::sine(440.0, 48_000.0, 480);
        let prepared = PreparedSfz::prepare(&instrument, &sample, 48_000.0).expect("摊平");
        let flat = &prepared.regions()[0];

        // 鉴别力自检：至少有一个轴在网格上真的改变了增益（⛔ 否则本判据对结合序无话可说）
        assert_ne!(
            flat.gain_at(60, 100),
            flat.gain_at(60, 40),
            "力度轴交叉淡化必须在网格上生效"
        );
        assert_ne!(
            flat.gain_at(60, 100),
            flat.gain_at(72, 100),
            "键轴交叉淡化必须在网格上生效"
        );

        let rates = RenderRates::new(48_000.0, 48_000.0);
        let (mismatches, first) = instrument
            .with_instrument(|parsed| {
                let region = &parsed.regions()[0];
                let mut mismatches = 0usize;
                let mut first: Option<(u8, u8, u32, u32)> = None;
                for n in 0u16..128 {
                    for v in 0u16..128 {
                        let want = region.playback_spec(n as u8, v as u8, rates).total_gain();
                        let got = flat.gain_at(n as u8, v as u8);
                        if want.to_bits() != got.to_bits() {
                            mismatches += 1;
                            if first.is_none() {
                                first = Some((n as u8, v as u8, want.to_bits(), got.to_bits()));
                            }
                        }
                    }
                }
                (mismatches, first)
            })
            .expect("闭包式访问");
        println!(
            "[sfz] 摊平增益 vs total_gain：128×128 网格里位型不同的点数 = {mismatches}（首个不同点 = {first:?}）"
        );
        assert_eq!(
            mismatches, 0,
            "⭐ 摊平增益必须与 total_gain **逐位相同**（首个不同点 = {first:?}）"
        );
    }

    /// ⭐ 判据（本片新增）：`offset`/`end` 归约出的 `SampleSpan` 已进摊平表。
    ///
    /// ⚠ 本判据**只证明字段被摊平登记**，⛔ 不证明渲染应用了它 —— 渲染仍未应用
    /// （见模块头文档的局限表第 ③ 条）。
    #[test]
    fn prepared_region_carries_the_source_span() {
        let instrument = SfzInstrument::load(
            "<region> sample=a.wav key=60 pitch_keycenter=60 offset=10 end=200\n\
             <region> sample=b.wav key=64 pitch_keycenter=60 end=-1\n"
                .to_owned(),
            &ParseLimits::default(),
        )
        .expect("解析必须成功");
        let sample = InMemorySample::sine(440.0, 48_000.0, 480);
        let prepared = PreparedSfz::prepare(&instrument, &sample, 48_000.0).expect("摊平");
        assert_eq!(
            prepared.regions()[0].span(),
            Some(SampleSpan {
                start: 10,
                end: Some(201)
            }),
            "offset=10 end=200（含端点）⇒ 半开区间 [10, 201)"
        );
        assert_eq!(
            prepared.regions()[1].span(),
            None,
            "end=-1 ⇒ 不产生采样输出（`SampleEnd::Silent`）"
        );
    }

    /// [MUST-GATE-001] 判据 ④：**本类型不在实时路径上** —— 机械部分。
    ///
    /// 机械可判的部分：`with_instrument` / `regions_matching` **必然分配**（重新解析 +
    /// `Vec`），因此它们**不可能**出现在实时回调里。这里用`Vec`返回值与"重新解析"两条
    /// 事实把它钉住；⭐ **"回调里只用摊平数据"这一条属于文档约束**，见模块头文档 ②
    /// （⛔ 机器暂时判不了"某函数有没有被回调调用"，故不假装有机械判据）。
    #[test]
    fn the_adapter_is_documented_as_control_side_only() {
        let instrument =
            SfzInstrument::load(MINIMAL.to_owned(), &ParseLimits::default()).expect("解析必须成功");
        // `regions_matching` 返回 `Vec` ⇒ 一定有堆分配 ⇒ 不能进实时回调
        let selected: Vec<usize> = instrument.regions_matching(60, 100).expect("查询");
        assert_eq!(selected.len(), 1, "查询结果本身就是堆分配的 `Vec`");
        // 每次 `with_instrument` 都**重新解析** ⇒ 用"区域数恒定但地址不同"的间接证据说明
        // 每次调用都会重建区域表（控制侧可接受，实时侧不可接受）。
        let first = instrument
            .with_instrument(|i| i.regions().as_ptr() as usize)
            .expect("闭包");
        let second = instrument
            .with_instrument(|i| i.regions().as_ptr() as usize)
            .expect("闭包");
        println!("[sfz] 两次解析的区域表地址: {first} / {second}");
        assert_eq!(instrument.region_count(), 2, "区域数不随解析次数变化");
    }
}
