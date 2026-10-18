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
//! ⭐ ⭐ **追加修订段（任务单 `#125`）—— 被修订的是上面 ② 那句「⛔ 当前**尚未**实现这一步」**
//! （⛔ 原文一字未删，口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋ 点名被修订的支）：
//! ⭐ **这一步已经实现了**，它就是下面的 [`PreparedSfz`]「摊平表」一节 ——
//! `Instrument::regions()` 在**构造期/离线期**被摊平成自有数据（`Vec` ＋ 定长数组 ＋ 内联的
//! 键/力度范围），实时回调只读它、只做整数/浮点比较与索引；⭐ 任务 `#66` 的步 2 起，它由
//! `render_track` 的**第 3 参** `Option<&PreparedSfz>` 接进实时路径（见下面「引擎侧声部」一节）。
//! ⚠ **被修订的只是"尚未实现"这一支**：① 同一句里「**本类型**（[`SfzInstrument`]）只用于
//! **控制侧**（加载、校验、计数、选择区域）」**逐字仍成立** —— [`SfzInstrument`] 自己仍会
//! **重新解析**（⇒ 堆分配）、⛔ 不在实时路径上；② ① 里那条红线 7 的引用与 `MUST-GATE-001`
//! 也都**仍成立**。
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
//! | ④ | 无 `group`/`off_by`/`note_polyphony`/`seq_` | ⭐ **`group`/`off_by` 已由任务 `#94` 接线、`note_polyphony` ＋ `note_selfmask` 已由任务 `#100` 接线**（语义由 `yeban-sfz` 裁、槽位与窃取由 `SynthEngine` 裁；见下面"独占组"与"同音复音限制"两节）；⛔ **仍未做：`seq_`**（round-robin 序列） |
//! | ⑤ | 无字节预算、无流式 | 未做（PCM 整段 clone 进摊平表；无预算上限、无流式解码）　⭐ ⭐ **追加修订段（任务单 `#125`）—— 被修订的是本行短标签里的「无字节预算」那一支**（⛔ 原文一字未删，口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋ 点名被修订的支）：⭐ 任务单 `#84` 起**字节预算的读数已经落地** —— [`SfzByteBudget`] ／ [`SfzByteUsage`] ／ [`SfzBudgetOverrun`]、[`PreparedSfz::byte_usage`] ／ [`PreparedSfz::budget_overrun`]（判据 S25～S27）⇒ 今天**有**预算读数、**有**超预算报告 ＋ ⭐ **口径是"引擎只报数、控制侧决定"**（⛔ 不报错、⛔ 不截断、⛔ 不改摊平结果；见下面 `sfz-needs-8` 那一行的 `#84` 登记）。⚠ **本行括注里那两条仍逐字成立**：⛔ **不强制上限**（＝ 括注的「无预算上限」）＋ ⛔ **无流式**（PCM 仍整段 clone 进摊平表、无流式解码）。 |
//! | ⑥ | 同一样本引两次 | ⭐ **步 3 已修**：去重键 ＝ `region.sample_path()`、容器 ＝ `BTreeMap`（确定性 [MODEL-AST-003]）⇒ 同一路径只 materialize **一份** [`PreparedSample`]；读数见 [`PreparedSfz::sample_count`] / [`PreparedSfz::pcm_bytes`] |
//! | ⑦ | CC 轴交叉淡化 | 未做（回调侧没有 CC 状态 ⇒ 与离线 `playback_spec` 一致地**跳过**该轴）。⭐ ⭐ **任务单 `#108` 已接线**：上面这句是**原文保留**（第 278 条，⛔ 一字未删）；`#107` 之后"回调侧没有 CC 状态"这条**理由已不成立**，三条路现在都读探针 —— 见下面「⭐ CC 轴交叉淡化（任务单 `#108`）」一节 |
//! | ⑧ | ⭐ **循环**（`loop_mode` / `loop_start` / `loop_end`） | ⭐ **本单已接线**（任务 `#91`：`PreparedRegion` 承载三个字段 ＋ [`mix_span_into`] 回绕；三条路共用唯一归约 [`reduce_loop_window`]）。⚠ 沿革：本行原文是"`loop_window` / `direction` / `pan` / `off_mode` 未做（读数在 `PlaybackSpec` 里，摊平表未携带）" ⇒ 循环已做；⭐ **任务 `#97` 把 `loop_end == pcm_len` 那一档从"不循环"改成"循环 ＋ 插值钳位"**（见下面"`loop_end == pcm_len`"一节）；`off_mode` 已由任务 `#94` 接线（⭐ 但**只作用于 `off_by` 关断**，见下面"独占组"一节），`direction` / `pan` **仍未做**，拆到⑩　⭐ ⭐ **追加修订段（任务单 `#113`）—— 被修订的是本行尾句里 `direction` 那一支**（⛔ 原文一字未删）：`direction` **已接线**、`pan` **仍未做**（见下面「⭐ 播放方向（`direction`，任务单 `#113`）」一节） |
//! | ⑨ | ⭐ **多 `sample=` 每区域各播自己** | ⭐ **步 3 已做**：摊平表持 `samples: Vec<PreparedSample>` ＋ [`PreparedRegion::sample_index`] 指路；⚠ 单样本重载 [`PreparedSfz::prepare`] **仍是广播**（键/力度映射对、**音色内容错** ⇒ 读数 `sample_content=broadcast(1)`） |
//! | ⑩ | `direction` / `pan` | 未做（读数在 `PlaybackSpec` 里，摊平表未携带）。⚠ 沿革：本行原文把 `off_mode` 也列在这里 —— ⭐ **任务 `#94` 已把它接线**（`PreparedRegion` 承载 ＋ `off_by` 关断期用；⛔ 作用面**不含** MIDI note-off），故本行只剩 `direction` / `pan`　⭐ ⭐ **追加修订段（任务单 `#113`）—— 被修订的是本行里 `direction` 那一支**（⛔ 原文一字未删，含上面那句沿革）：`direction` **已接线**、本行只剩 `pan`（见下面「⭐ 播放方向（`direction`，任务单 `#113`）」一节） |
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
//!   回绕把源位置压在窗口内（[`reduce_loop_window`] 保证窗口落在可读范围内；
//!   ⭐ 终点**恰好等于** `pcm_len` 时靠 [`mix_span_into`] 的**插值钳位**可读，
//!   见下面"`loop_end == pcm_len`"一节）⇒
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
//! ## ⭐ `loop_end == pcm_len`（`#91` 登记、任务单 `#97` 裁决；⛔ 不是新机制）
//!
//! ⚠ **沿革（⛔ 不改写历史）**：循环接线片（任务 `#91`）的归约要求 `loop_end`
//! **严格小于** `pcm_len`（理由是归约层无法保证 `index + 1` 可读）⇒ 那一档被
//! **判成"不循环"**。`#91` 把它作为一条**边界**主动上报，集成者登记 ＋ 开单。
//!
//! ⭐ **本单（`#97`）的裁决**：**在循环边界处把插值钳位**，⛔ 不是把窗口判成不循环 ——
//! 即 `loop_end == pcm_len` **也循环**，回绕点上读 `index + 1` 时**钳到最后一个样本**
//! （[`mix_span_into`] 的"钳位"一节；理由：它是 `loop_end < pcm_len` 那条既有路
//! 唯一同形的最小延伸 —— 那条路读的是窗口外**紧邻**的那一格，而 `== pcm_len` 时那一格
//! **不存在**）。⚠ **`loop_end == pcm_len - 1` 的行为⛔ 一字未动**（判据 `S46`／`S47`
//! 两条腿并列量周期；`S28`／`S30`／`S37` 逐位指纹原样）。
//!
//! ⚠ 归约层**仍**拒绝 `loop_end > pcm_len`（⛔ 越界窗口不发明"夹到末尾"语义）；
//! 本单只放宽**恰好等于**这一档。
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
//!     规范（SFZ v2 / ARIA，<https://sfzformat.com/opcodes/off_time/>）说的是
//!     **一段长度 ＝ `off_time` 的淡出**、**independent of the sample release time**
//!     （缺省 6 ms）；⭐ 本仓**不读 per-region `off_time`**、`off_mode=time`
//!     **按 `normal` 处理**（登记在案的偏离，见 `docs/adr/ADR-0008-sfz-off-time-versus-arch-rt-004.md`
//!     裁决 **C**）；而 `off_time` 不在本片范围内（语料分布：`time` 4 次）；
//!     ⛔ 不是静默降级（显式取值一律原样解析、原样带出，`S44` 钉住集合外的取值仍报 `Err`）。
//!
//!     ⚠ **与 `ARCH-RT-004` 的 3 ms 不冲突**：那条管的是**窃取路**（池满抢占），
//!     与 `off_time`（**`off_by` 独占组路／`off_mode=time` 路**）**是两条不同的路**。
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
//! # ⭐ 同音复音限制（`note_polyphony` / `note_selfmask`；任务单 `#100` 接线）
//!
//! ## ⚠ 本单是**接线**，⛔ 不是新解析、⛔ 不是新裁决
//!
//! 解析层**早就有了**：`crates/yeban-sfz/src/instrument.rs:1987` 的 `read_note_polyphony`
//! 读 `note_polyphony`（`read_u32`：缺省 `0`、域 `0..=u32::MAX`、越界／非数值 ⇒ 响亮 `Err`）
//! 与 `note_selfmask`（白名单 `on`/`off`，大小写不敏感，集合外 ⇒ `Err`），归约成
//! [`NotePolyphony`]（一个 `Copy`：`limit: u32` ＋ `self_mask: bool`）—— 取值域／缺省／
//! 语义的逐字出处与"`limit = 0` 读作不限制"的工程裁决**全写在 [`NotePolyphony`] 的文档里**。
//! `yeban-sfz` 的 `VoicePool` 甚至早有一份完整策略 `VoicePool::apply_note_polyphony`
//! （`voice_pool.rs:418`）。⛔ 缺的**只是播放层**：那个池**不被引擎使用**
//! （见 [`SFZ_VOICES_PER_TRACK`]），而引擎的 [`PreparedRegion`]／[`SfzVoicePool`]
//! 从来不消费这份读数 ⇒ 本片把这一份 `Copy` 读数搬进摊平表，并在
//! [`SfzVoicePool::trigger_note`] 的**触发期**接上限：**⛔ 不新增解析、
//! ⛔ 不改 `yeban-sfz` 的既有判定规则**（第 259 条：集合外／非法一律响亮 `Err`，
//! "缺失 ≠ 非法"，这些行为本片一字未动）。
//!
//! ## 语义（规范逐字）
//!
//! * <https://sfzformat.com/opcodes/note_polyphony/>：表格行是 Type = integer、
//!   Default = N/A、Range = **空**；正文 "Polyphony limit for playing the same note
//!   repeatedly."，键是 polyphony group —— "The note polyphony is checked within a
//!   polyphony group, set by the `group` or `polyphony_group` opcodes. If no group is
//!   specified on the region (or its group, master or globally) the note polyphony applies
//!   to the default group as if `group=0` was specified."；
//! * <https://sfzformat.com/opcodes/note_selfmask/>：表格行是 Type = string、Default = `on`、
//!   Options = `on, off`；缺省的力度规则与 `off` 的严格化**逐字**引在
//!   [`SfzVoicePool::enforce_note_polyphony`] 的文档里；
//! * ⭐ **上限是 hint、⛔ 不是硬上限**："The `note_polyphony` opcode is thus not a strict
//!   polyphony limit but more of a hint for the instrument behavior." ＋
//!   "A new note will always play." ⇒ 没有有资格的在场声部时**停手**，新音符照常发声。
//!
//! ## 落地形状（⭐ 三条，逐条可判）
//!
//! * [`PreparedRegion::note_polyphony`]：**每区域**一份 [`NotePolyphony`]（原样搬运，
//!   摊平期 ⇒ ⛔ 不进实时对象；`SfzVoice` **＋0 字节**，`S9` 仍钉 `40 B`）；
//! * [`SfzVoicePool::enforce_note_polyphony`]：**触发期**一次 ≤ [`SFZ_VOICES_PER_TRACK`]
//!   槽的线性扫描（⛔ 不进逐样本路径）⇒ **零分配、零锁等待、零阻塞 I/O**；
//!   victim 的 `group` 由它**自己**的 [`SfzVoice::region_index`] 回查摊平表
//!   ⇒ ⛔ **不新增任何逐声部字节**；
//! * ⛔ **不借用 `off_by` 的关断**、⛔ **不借用池满的窃取**：见下面两条。
//!
//! ## ⚠ "引擎里是不是有**两份** `apply_note_polyphony`？"（⭐ 如实登记）
//!
//! 是**两份扫描**，⛔ 不是两份**语义**：`yeban-sfz` 的 `VoicePool` 与引擎的
//! [`SfzVoicePool`] 是**两套形状**（前者：全局 512 声部、立即接管 ＋ 把 victim 句柄交还
//! 调用方；后者：每轨 16 槽、victim 与挂起者**共用同一槽**、`fade`／`pending` 全在池内）
//! —— 这与 [`SfzVoicePool::select_victim`] 的处境**逐字相同**，而 `#94` 已就那一处境裁决
//! "那个池**不被引擎使用** ⇒ ⛔ 不构成引擎里有两份实现"（见 [`SFZ_VOICES_PER_TRACK`] 的
//! 文档）。本片照**同一口径**：**资格判定直接调用 `yeban-sfz` 的 [`NotePolyphony::masks`]**、
//! **让位顺序照抄 [`NotePolyphony`] 文档里那条既有裁决**（⛔ 不另立一份排序），
//! 引擎侧只多一次 ≤ [`SFZ_VOICES_PER_TRACK`] 槽的**触发期**扫描。
//! ⛔ 本片**没有**把 `yeban_sfz::VoicePool` 拉进引擎：那会引入第二个全局声部池与第二条
//! 声部生命周期，与 `#94` 的形状裁决（定容池、槽内 slack）直接冲突。
//!
//! ## ⭐ 与 `off_by` ／ 池满窃取的分工（⛔ 三件事，⛔ 不互相借用）
//!
//! | 机制 | 触发条件 | victim 选择 | 怎么让位 |
//! | :--- | :--- | :--- | :--- |
//! | `off_by` 关断（`#94`） | **另一个区域**指名一个组 | victim 自己区域的 `group == off_by` | 看 **victim 自己的 `off_mode`**；`one_shot` victim 免疫 |
//! | 池满窃取（`needs-2`） | 池里 **16 槽全 active** | `(层级, 触发序号, 槽位下标)` | 3 ms 淡出 ＋ **同槽挂起者**（计 `stolen`／`dropped`） |
//! | `note_polyphony`（`#100`） | **同一音符连发**超过上限 | `group` × 音高 ＋ [`NotePolyphony::masks`] 的资格；键 `(力度, 触发序号, 槽位下标)` | **立即硬停**（既有 `silence()` 原语）；⛔ 不动 `pending`／[`FadeRamp`]，⛔ 不计数 |
//!
//! ⚠ `off_mode` **不在**本片的路径上：规范把它的作用面限定在 `off_by`
//! （"This opcode will determinate how a region is turned off by an `off_by` opcode."）。
//!
//! ## ⭐ `one_shot` 的 victim **照掐**（语料逼出来的正确答案，与 `off_by` 那条**不同**）
//!
//! `#94` 裁决过"`one_shot` 的 victim 忽略 `off_by`"（那是"**另一个区域**来掐我"）。
//! ⛔ 那条**不**外推到本片：登记语料里 `note_polyphony` 的用法**几乎全长在 `one_shot` 上**
//! —— `virtuosity-drums` 的映射文件是 `<global> loop_mode=one_shot … note_polyphony=3
//! group=5NN`（每麦克风一个组，正是规范下一句说的用法），`cithara-barbarica` 的 Lyre 模式
//! 是 `loop_mode=one_shot`／`note_polyphony=1`／`note_selfmask=off`／`group=1` **四行紧挨着**
//! （`assets/samples/cithara-barbarica/Programs/01-cithara_barbarica_keyswitch.sfz:43-46`）。
//! 若把 `off_by` 那条外推过来，`note_polyphony` 会在**用到它的每一个文件里**变成空操作
//! ⇒ 那不是规范的意思（本片管的是"同音连发的并发数"，⛔ 不是"note-off 到不到"）。
//!
//! ## ⭐ "多个有资格者掐谁"的出处（⭐ 复用既有裁决，⛔ 不发明）
//!
//! 规范只对 `note_polyphony=1` 给出逐步规则，⛔ 不规定"多个同力度者时掐谁"。本仓对这一点的
//! **既有**工程裁决写在 [`NotePolyphony`] 的文档里（`yeban-sfz`，随 `main` 一起在案）：
//! "选择顺序由 `VoicePool::apply_note_polyphony` 规定（力度最低 → 触发最早 → 槽位下标最小），
//! 保证 [ARCH-DET-001] 的确定性" ⇒ 本片**照抄这条键**，⛔ 不是本单新立的裁决。
//! 规范**明确有据**的两条是：**资格**（`note_selfmask` 的力度规则）与
//! **新音符永远发声**（"A new note will always play."）。
//!
//! ## 判据
//!
//! `tests/sfz_instrument.rs` 的 S49～S53（上限真的生效 ／ 键 ＝ `group` × 音高 ／
//! `note_selfmask` 的力度规则 ／ 既有夹具指纹逐位不变 ／ ⛔ 不是窃取）。
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
//! | sfz-needs-6 | **循环 / ADSR / `off_mode` / `group`/`off_by` / `note_polyphony`** | ⭐ ⭐ **四片已闭合**：① 循环 ＝ 任务 `#91`（`loop_start`/`loop_end`/`loop_mode` 进 [`PreparedRegion`]、[`mix_span_into`] 回绕、`one_shot` 忽略 note-off；判据 S28～S33）；② ADSR ＝ 任务 `#92`（五个 `ampeg_*` 进 [`PreparedRegion`]、逐声部只带 5 字节推进状态、[`AmpegRun`] 分段线性；判据 S34～S38）；⭐ ③ **独占组／`off_mode` ＝ 任务 `#94`**（`group`/`off_by`/`off_mode` 进 [`PreparedRegion`]（＋8 B／区域，摊平期）、触发期 ≤16 槽线性扫描、`fast` ⇒ 硬停／`normal`＋`time` ⇒ 进释放、`one_shot` victim 忽略 `off_by`、触发者不自掐、硬停保留同槽 `pending`；判据 S39～S45）；⭐ ④ **同音复音限制 ＝ 任务 `#100`**（`note_polyphony` ＋ `note_selfmask` 早已由 `yeban-sfz` 解析并裁决，本片把这一份 `Copy` 读数搬进 [`PreparedRegion`]（＋8 B／区域，摊平期）并在 [`SfzVoicePool::trigger_note`] 的**触发期**接上限：键 ＝ `group` × 音高、掐只掐 [`NotePolyphony::masks`] 允许的在场声部、让位顺序复用 `yeban-sfz` 的 `(力度, 触发序号, 槽位下标)`、新音符永远发声、上限是 hint ⛔ 不是硬上限；⛔ **不是** `off_by`、⛔ **不是**池满窃取；判据 S49～S53）。⭐ **一处书面偏离**：`off_mode=time` 按 `normal` 处理（规范说的是**一段长度 ＝ `off_time` 的淡出**、**independent of the sample release time**；⭐ 本仓**不读 per-region `off_time`**；`off_time` 不在本片范围；⛔ 不是静默降级；见 `ADR-0008` 裁决 **C**）。⭐ `ampeg_*` 的**调制类**：**CC 支**已由任务单 `#104` 接上（`ampeg_*ccN` / `_onccN` / `_curveccN` 的**解析 ＋ 归约 ＋ 离线求值**；实时通路⛔ 仍不读 CC 状态，登记见本模块头的"CC 调制"一节）、**力度支**已由任务单 `#106` 接上（`ampeg_vel2*` 的**解析 ＋ 归约 ＋ 三条路求值**，登记见下面的"力度调制"一节）。⛔ **仍未做**：`fileg_*` / `pitcheg_*`、`off_time`、`direction`、`pan` ⭐ ⭐ **追加修订段（任务单 `#113`）—— 被修订的是这份清单里的 `direction` 那一项**（⛔ 原文一字未删）：`direction` **已接线**；`fileg_*` / `pitcheg_*` / `off_time` / `pan` **仍未做**（见下面「⭐ 播放方向（`direction`，任务单 `#113`）」一节） |
//! | sfz-needs-7 | ⭐ **缺样本的引擎侧回退是"广播到 `samples[0]`"（内容错）** | ⭐ ⭐ **R711 已裁决、本步已闭合**：**引擎出静音并计数**（[`PreparedSfz::silent_region_count`] ＋ [`PreparedSfz::missing_sample_paths`]）、**合成替代内容是控制侧的决定**（想给替代样本 ⇒ 调用方显式放进提供表，或用广播重载 [`PreparedSfz::prepare`] 明确声明）。落地口径：`prepare_with_samples` 遇缺失路径 ⇒ 该区域 `sample_index = NO_SAMPLE` ⇒ 触发时**不建声部**（与 `span()==None` 同径）⇒ ⛔ **旧"广播到 `samples[0]`"已删除**（判据：`tests/sfz_instrument.rs` 的 S15）|
//! | sfz-needs-8 | ⭐ **摊平表不含样本字节预算 / 流式** | ⭐ ⭐ **部分闭合（任务单 `#84`）——"可判"那一半已闭合，"流式"那一半仍未做**：①**字节预算读数 ＋ 判据**已落地（[`SfzByteBudget`] ／ [`SfzByteUsage`] ／ [`SfzBudgetOverrun`]、[`PreparedSfz::byte_usage`] ／ [`PreparedSfz::budget_overrun`]；判据 `tests/sfz_instrument.rs` 的 S25～S27）；②**超预算口径 ＝ 引擎只报数、控制侧决定**（⛔ 不报错、⛔ 不截断、⛔ 不改摊平结果；理由见 [`PreparedSfz::budget_overrun`]）；③⚠ **仍未闭合**：`prepare_with_samples` 依旧把每个被引用的 `InMemorySample::pcm` **整段 clone** 进摊平表（去重后 N 份）⇒ **无流式、无 `Arc` 共享**，且**峰值内存不受本预算约束**（调用方手里那份已解码 PCM 本就在场） |
//!
//! # ⭐ `ampeg_*` 的 CC 调制（任务单 `#104`）
//!
//! 规范语义、四种形态（`ccN` ／ `_onccN` ／ `_curveccN`，外加被调制的基准 opcode）的
//! **逐字出处**、公式（`base + Σ delta × curve(cc)`）、取值域与全部工程裁决写在
//! **`yeban-sfz`** 的 `modulation` 模块文档里 —— 本片 ⛔ 不复制那份事实源。
//!
//! 本片在**引擎侧**做了三件事：
//!
//! 1. **摊平期承载**：归约结果（含**烘好的**曲线点表）搬进 [`PreparedRegion`]
//!    （`ampeg_mods[AMPEG_MOD_CAP]` ＋ 条数；`PreparedRegion` 2672 → **4192 B**，＋1 520 B／区域，
//!    摊平期、⛔ 不进实时对象；`SfzVoice` 40 B ／ `SfzVoicePool` 1712 B ／ `EngineRuntime` 180000 B 全部 ＋0；⭐ ⚠ **`#107` 后 `EngineRuntime` 为 `180144 B`**——见本模块头的 `#107` 追加修订段）。
//!    曲线在这里烘进**区域表**而不是另开一张表，是为了让
//!    [`SfzByteUsage`] 的"区域表 ＝ `size_of × N`"读数**仍然没有隐藏的第四份开销**；
//! 2. **离线两条路求值**：[`SfzInstrument::render_note_into_with_cc`] 与
//!    [`PreparedSfz::render_block_into_with_cc`]（各有一个 `cc = None` 的旧入口包装，
//!    ⇒ 无探针时**逐位不变**）。求值本体是 `yeban-sfz` 的
//!    `ampeg_value_with_baked`（**同一份**实现，⛔ 没有第二套公式）；
//! 3. **`slot_readings` 侧可见**：[`PreparedRegion::ampeg_value_with_cc`] ／
//!    [`PreparedRegion::ampeg_run_with_cc`] 是纯算术 ＋ 定长切片（零分配、零锁、零 I/O），
//!    [`SfzVoicePool`] 那条路今天传 `None`（见下面的登记）。
//!
//! ⭐ 双口径登记（任务单 `#104`）：**包络的 CC 调制只在离线的两条路
//! `render_note_into_with_cc` ／ `render_block_into_with_cc`（以及它们的摊平表读数）上生效
//! —— 实时通路（[`SfzVoicePool`] 的触发与渲染）今天**不读任何 CC 状态**，
//! 因此同一份 SFZ 在"给了探针的离线渲染"与"实时回调"下包络**不同**：
//! 前者按 `base + Σ delta × curve(cc)`，后者恒为基准值。**这是本片有意的范围边界
//! （⛔ 不是静默降级）：把 CC 状态接进实时回调属于"架构单"，需人类批准。**
//!
//! ⚠ ⭐ 任务单 `#106` 对上面这条登记的修订（⭐ ⛔ 只针对 **CC 那一支**，力度那一支见下节）：
//! 本条的"后者恒为基准值"今天**仍然成立**（实时通路照样不读 CC 状态），但实时通路的包络
//! 参数**不再是**纯粹的基准字段 —— 它会按**逐声部力度**加上 `ampeg_vel2*` 那一项
//! ⇒ 双口径的差别只剩 **CC 支**这一项。
//!
//! ⭐ 摊平期门控登记（任务单 `#104`）：**摊平表不随探针改变区域选择** ——
//! CC 门控（`loccN`/`hiccN`）与 keyswitch / `trigger` 一样在**摊平期**用
//! `RegionQuery::new(note, velocity)`（＝ `cc = None`）求值一次并烧进两张掩码
//! （见 [`flatten_regions`]）⇒ `render_block_into_with_cc` 的探针只影响 `ampeg_*` 的**取值**，
//! ⛔ 不会让一条被 CC 门控挡掉的区域发声。离线的
//! [`SfzInstrument::render_note_into_with_cc`] 则把同一个探针**同时**喂给
//! [`RegionQuery::with_cc`]（门控在那条路上是活的）⇒ 这条差别**登记在案**，⛔ 不分叉成两份选择口径。
//!
//! ⭐ ⭐ **追加修订段（任务单 `#107`）—— 被修订的是上面两条登记里"实时那一侧"的那一支**
//! （⛔ 原文一字未删，见台账第 278 条的既有口径：保留原文 ＋ 追加修订段 ＋ 点名被修订的支）：
//!
//! 1. **双口径登记的"实时恒为基准值"这一半作废**：实时通路（[`SfzVoicePool`] 的逐块现算）
//!    ⛔ **不再**恒为基准值 —— 它经 [`SfzVoicePool::render_into_with_cc`] →
//!    [`PreparedRegion::ampeg_run_with_cc_and_velocity`] 读**会话运行态**的 CC 探针
//!    （[`crate::cc_state::ControlChangeState`]，由 [`crate::rt::EngineRuntime`] 持有、
//!    由 [`crate::ring::EngineEvent::ControlChange`] 在量子边界推进）⇒
//!    **离线 ＝ 摊平 ＝ 实时**在"同一 CC 值 ＋ 同一力度"下**逐位相同**
//!    （判据：`tests/sfz_instrument.rs` 的 S64／S65；门控那一半见第 2 条）。
//!    ⚠ 差别的**残留**只剩两条，都登记在案：① 门控（第 2 条）；② 探针的
//!    **armed 口径** —— 会话运行态里**一条 CC 都没有**时引擎传 `None`
//!    （⇒ 与接线前逐位相同），**armed 之后**未收到过的控制器读 `0`
//!    （[`crate::cc_state`] 模块文档 §2.2）。
//! 2. **摊平期门控登记维持不变（显式选路 ③）**：本单**没有**改区域选择口径 ——
//!    CC 门控（`loccN`/`hiccN`）仍在**摊平期**用 `RegionQuery::new(note, velocity)`
//!    （`cc = None`）求值一次并烧进两张掩码（[`flatten_regions`]）⇒
//!    [`SfzVoicePool::render_into_with_cc`] 的探针**只**影响 `ampeg_*` 的**取值**。
//!    ⭐ **已知边界的明文后果**：一条被 `loccN`/`hiccN` 门控的区域在引擎的**摊平表**路径上
//!    **永远不发声**（不论 CC 状态是什么、也不论是离线摊平还是实时）；而离线的
//!    [`SfzInstrument::render_note_into_with_cc`] 那条路上门控是**活的**。
//!    ⛔ **为什么不选另外两条**：①"控制侧在 CC 变化时重摊平"要把整张区域表 ＋ PCM
//!    重发一遍（每次 CC 一条新快照）——那是**架构单**，且会把 CC 变成快照的一部分；
//!    ②"实时每块现查门控"要让 `PreparedRegion` 带上逐条 `CcGate` 并在 `accepts` 之外
//!    再判一次 —— 它会**改变区域选择**（＝ 修订第 2 条的登记，⛔ 本单的范围不允许静默改）。
//!    语料分母（本机实测，1398 个 `.sfz`）：`loccN`/`hiccN` 门 **7950** 处、
//!    `xfin_loccN`/`xfout_*ccN` **2275** 处 ⇒ 这条边界**有真实体量**，⛔ 不是理论缺口。
//! 3. **本单未接线（登记，⛔ 不静默）**：CC 轴**交叉淡化**（`pan_oncc*` 9796 处 ／
//!    `xfin_*ccN`）与 `set_ccN=`（1137 处）的**播种**都仍归调用方/后续单 ——
//!    `flatten_regions` 的摊平期就跳过了 CC 轴那一段（既有注释），
//!    而 `set_ccN` 的初值口径见 §2.2 与 `yeban-sfz` 的 `Instrument::cc_defaults`。
//!
//! ⭐ ⭐ **追加修订段（任务单 `#108`）—— 被修订的是上面第 3 条登记里
//! "`xfin_*ccN` / `xfout_*ccN` 交叉淡化未接线"这一支**（⛔ 上面三条的**原文一字未删**，
//! 口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋ 点名被修订的支）：
//!
//! 1. **CC 轴交叉淡化已接线（三条路）**：`flatten_regions` 的 CC 分支**不再跳过** ——
//!    几何（`XfAxis::Cc(cc)` 的 [`Crossfade`]）原样烘进
//!    [`PreparedRegion::cc_crossfades`]（定长 `[Crossfade; CC_CROSSFADE_CAP]` ＋ 条数，
//!    `Copy`、⛔ 无 `Vec`），取值在**逐块**由
//!    [`PreparedRegion::gain_at_with_cc`] 按探针做；离线那条路改用
//!    `yeban-sfz` 的 `Region::playback_spec_with`（它本来就**读得到** `RegionQuery::cc`）。
//!    ⇒ **离线 ＝ 摊平 ＝ 实时**在"同一 CC 值 ＋ 同一力度"下**逐位相同**
//!    （判据：`tests/sfz_instrument.rs` 的 S67／S68）。写在这里避免"改了却没登记"。
//! 2. **⛔ 未修订的两支（仍在，逐条点名）**：
//!    * `set_ccN=` 的**播种**（初值）仍归调用方 —— 本节第 3 条的原文**整条有效**；
//!    * ⛔ **不属本单**的"任意 opcode 的 `_onccN` 调制"（`pan_oncc*` 9796 处 ／
//!      `volume_oncc*` 等）是 `#104` 那套 `ampeg_*` 机制的**推广**（语料 9796 处），
//!      另开单；⛔ 与本单的交叉淡化 CC 轴**不是同一件事**；
//!    * `loccN`/`hiccN` **区域门控**的实时化仍**未接线**（`#107` 已登记显式选路 ③，
//!      见上面第 2 条）—— ⛔ 本单**不改**它。
//! 3. **上界（`CC_CROSSFADE_CAP` ＝ 256）是结构性的**，⛔ 不是语料最大值 ＋ 余量：
//!    由来、语料普查读数与代价逐条写在 [`CC_CROSSFADE_CAP`] 的文档里
//!    （分母 1398 份 `.sfz`／可解析根文件 1205 份／region 64894 条；单区域 CC 段最大 **2**）。
//!    ⭐ 代价：`PreparedRegion` `4232 → 5768 B`（＋1536 ＝ 256 × `size_of::<Crossfade>()`）；
//!    ⛔ `SfzVoice` 仍 `40 B`（＋0）、⛔ `SfzVoicePool` 仍 `1712 B`（＋0）、
//!    ⛔ `EngineRuntime` 仍 `180144 B`（＋0）。
//! 4. **顺带修订的措辞**（⛔ 原文保留，只点名读法）：上面 `#104` 的「探针只影响
//!    `ampeg_*` 的**取值**」（第 522 行）与 `#107` 第 2 条同款的那句，在 `#108` 之后
//!    应读作「探针只影响**取值类**的量（`ampeg_*` **与 CC 轴交叉淡化的增益**），
//!    ⛔ 不影响区域选择」—— 两句的原文一字未删，本条就是它们的修订见证。
//!
//! ⭐ ⭐ **追加修订段（任务单 `#109`）—— 被修订的是上面 `#107` 第 2 条登记里
//! "摊平期门控登记维持不变（显式选路 ③）"**那一支**（⛔ 上面 `#104`／`#107`／`#108`
//! 的**原文一字未删**，口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋ 点名被修订的支）：
//!
//! 1. **`#107` 第 2 条的"区域选择口径不改"作废**（它整条就是显式选路 ③ 的登记）：
//!    CC 门控**不再是**区域选择的唯一依据，也**不再**被烧进掩码 ⇒ 一条被
//!    `loccN`/`hiccN` 门控的区域**不再**在摊平表路径上"永远不发声"。
//!    新的口径：掩码只承载**静态**门控（通道 / `trigger` / keyswitch / `first`／
//!    `legato`），CC 门控由**触发期**的 [`PreparedRegion::cc_gates_ok`] 按会话运行态的
//!    探针裁决（`flatten_regions` 的静态门控改走 [`non_cc_gates_open`]，它把 CC 那一段
//!    整个摘出去）。
//!    ⚠ **对没有 CC 门控的区域逐位不变**（[`non_cc_gates_open`] 与 `region_matches` **同序、
//!    同缺省值**，只把 CC 那一段换成 `true`）⇒ `#107` 第 1 条（"实时恒为基准值"已作废、
//!    三条路一致）与 `#108` 的两条**都仍然成立**（判据 `S64`／`S65`／`S68` 复跑）。
//! 2. **`#107` 第 2 条里那两句"明文后果"作废**：原文写"一条被 `loccN`/`hiccN` 门控的
//!    区域在引擎的**摊平表**路径上**永远不发声**（不论 CC 状态是什么、也不论是离线摊平
//!    还是实时）"—— 本单之后**不成立**：CC 门控在摊平表路径上**按探针生效**（判据
//!    `S81`／`S82` 两个 CC 值的发声读数 ＋ `S81` 三条路一致）。⭐ 原文保留，本段是它的
//!    修订见证 ＋ 点名。
//! 3. **`#108` 第 2 条的第三个圆点"`loccN`/`hiccN` 区域门控的实时化仍**未接线**"作废**
//!    （同一件事的另一处登记，⛔ 一并点名）：它现在**已接线**。`#108` 第 2 条另外两个
//!    圆点（`set_ccN=` 播种、`pan_oncc*` 一族）**整条有效**，本单⛔ 不碰。
//! 4. **`#108` 第 4 条"顺带修订的措辞"扩写**（⛔ 原文保留，只点名读法）：上面 `#104`
//!    的「探针只影响 `ampeg_*` 的**取值**」（第 522 行）、`#107` 第 2 条同款的那句、
//!    以及 `#108` 第 4 条自己写的「探针只影响**取值类**的量 …… ⛔ 不影响区域选择」，
//!    在 `#109` 之后应读作「探针影响**取值类**的量（`ampeg_*`、CC 轴交叉淡化的增益）
//!    **与区域选择里唯一那一项——CC 门控**」—— 三句的原文一字未删，本条是修订见证。
//! 5. **`#104` 双口径登记（`cc = None` ⇒ 与 `playback_spec` 一致地跳过该轴）那一支**
//!    **⛔ 未被本单修订**：它说的是**取值**（`cc = None` 时 CC 轴增益恒 `1.0`），
//!    仍然成立。⭐ 被修订的**只有**"门控在掩码里"这一支。
//! 6. **顺带修订的第三处措辞**（⛔ 原文保留）：本节 `#108` 的第 3 条里
//!    "`PreparedRegion` `4232 → 5768 B`"是**`#108` 当时**的读数；`#109` 之后它是
//!    **6152 B**（＋384 ＝ `128 × size_of::<CcGate>()` ＋ 条数 `u8`；余下 6 B 落在结构体尾部对齐）。
//!
//! ### ⭐ 门控上界（`CC_GATE_CAP` ＝ 128）是结构性的（⛔ 不是语料最大值 ＋ 余量）
//!
//! 由来、语料普查读数（分母 **1398** 份 `.sfz`／可解析根文件 **1205** 份／region
//! **64894** 条／单区域门控**最大 3**／直方图 `{0: 43831, 1: 16872, 2: 797, 3: 3394}`）
//! 与两条路的代价逐条写在 [`CC_GATE_CAP`] 的文档里。⭐ 代价：
//! `PreparedRegion` `5768 → 6152 B`（＋384 ＝ `128 × 3`；条数 `u8` 与尾部对齐共 6 B）；
//! ⛔ `SfzVoice` 仍 `40 B`、⛔ `SfzVoicePool` 仍 `1712 B`、⛔ `EngineRuntime` 仍 `180144 B`。
//!
//! # ⭐ 播放方向（`direction`，任务单 `#113`）
//!
//! ⭐ ⭐ **追加修订段（任务单 `#113`）—— 被修订的是三处"`direction` 仍未做"的登记**：
//! 局限表第 ⑧ 行的尾句「`direction` / `pan` **仍未做**，拆到⑩」、局限表第 ⑩ 行
//! 「`direction` / `pan` | 未做」、以及 `sfz-needs-6` 的「⛔ **仍未做**：… `direction` …」
//! （⛔ **三处原文一字未删**，口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋ 点名被修订的支）。
//! **被修订的只有 `direction` 那一支**：它**已由本单接线**（`pan` 那一支**整条有效**、
//! ⛔ 本单不碰 —— 与 `#110` 修订 `set_ccN` 而把 `pan_oncc*` 留在未接线里同款）。
//!
//! ## 规范逐字（<https://sfzformat.com/opcodes/direction/>，2026-10-16 现读）
//!
//! > The direction in which the sample is to be played.
//! >
//! > This is similar to [loop\_type](../loop_type/) but affects the entire sample, not just the
//! > defined loop regions.
//!
//! | Name | Version | Type | Default | Options | Unit |
//! | --- | --- | --- | --- | --- | --- |
//! | direction | SFZ v2 | string | forward | forward, reverse | |
//!
//! ⭐ **取值域＝ `forward` / `reverse` 两项**（⛔ 没有 `alternate`）：`yeban-sfz` 的白名单
//! [`PlayDirection::OPTIONS`] 逐字就是这两项、缺省 `forward`、大小写不敏感匹配
//! （`ScopedValue::as_option`），集合外取值 ⇒ **响亮 `SfzError::InvalidOption`**
//! （⛔ 不降级、⛔ 不猜）。本单只**搬运**那份 `Copy` 读数，⛔ 一个字节的解析都没改。
//!
//! ⭐ **`alternate` 不在 `direction` 里 —— 它在 `loop_type` 里**（同一站点
//! <https://sfzformat.com/opcodes/loop_type/>，2026-10-16 现读）：
//!
//! | Name | Version | Type | Default | Options | Unit |
//! | --- | --- | --- | --- | --- | --- |
//! | loop\_type | SFZ v2 | string | forward | forward, backward, alternate | |
//!
//! > This only affects the loop portion of the sample. To play a sample backwards with no
//! > looping, use [direction](../direction/) instead.
//!
//! ### ⛔ 未做（登记，⛔ 不静默）：`direction` 的第三个取值与 `loop_type`
//!
//! ⭐ **`direction=alternate` 今天的行为 ＝ 响亮报错**：它**不在**
//! `direction` 的 `Options` 里（逐字见上表）⇒ `yeban-sfz` 的 `as_option` 判
//! `InvalidOption`（判据 `S99` 把这个读数钉住，⛔ 不是"We 静默当成 forward"）。
//! ⭐ **`loop_type` 整条**（含 `alternate` / `backward`）在本仓**一个字节都没解析**
//! —— 语料普查 **0 处**、源码 grep **0 处** ⇒ ⛔ 本单不发明它的语义、⛔ 不为
//! "`direction` 的 `alternate`"造一个规范里不存在的取值。⭐ 若将来要实现
//! 「循环段来回弹」，那是 `loop_type` 单（需要**逐声部**的方向状态 ⇒
//! 与 `#113` 的"零新逐声部字段"口径**不相容**，⛔ 不在本单）。
//!
//! ## 实现口径（三条路共用一份归约）
//!
//! ⭐ 唯一归约是 [`reduce_travel`]（与 [`reduce_span`] / [`reduce_loop_window`] 同形态）：
//! 它把方向折成**三件已有读数** —— **行进起点** / **带符号步进** / **行进边界**
//! （`forward`：`start` / `+rate` / 上界 `end`；`reverse`：`end − 1` / `−rate` / 下界 `start`）
//! ⇒ ⛔ **零新逐声部字段**（`SfzVoice` 仍 40 B：三件读数落在既有的
//! `position` / `rate` / `source_end` 三格上）。
//!
//! ⭐ 反向的**逐样本**几何同样只有一处（[`mix_span_into`] 里那个 `rate < 0` 判别）：
//! 回绕镜向（越过窗口**下**侧折回）、停止闸门换成**下界**（浮点比较）、
//! 插值"下一格"**恒钳**到最后可读样本（反向从区间高端起步 ⇒ `index + 1` 可以越界）。
//! `rate > 0` 那一侧的三条表达式**逐字未动** ⇒ 既有夹具逐位不变（判据 `S33`/`S37`/`S48`）。
//!
//! ⭐ **反向的语义 ＝ "把区间样本倒序之后按 `forward` 渲染"**（逐帧逐位，判据 `S95` 用
//! **倒序样本 ＋ 不带 `direction` 的同一条路**当独立参照）：对任意 `t = i + f`，
//! `F(end − 1 − t) = pcm[end−1−i]·(1−f) + pcm[end−2−i]·f` 与倒序数组
//! `R[j] = pcm[end−1−j]` 的 `G(t) = R[i]·(1−f) + R[i+1]·f` 是同一个表达式
//! （两项操作数相同、加法在 binary32 下可交换）⇒ ⛔ 不是"另一套插值口径"。
//!
//! ## ⚠ `direction=reverse` × `offset` / `end`：规范**没有**明文
//!
//! ⭐ 三份相关页面（`direction` / `offset` / `end`）在 2026-10-16 现读时**都没有**
//! 一句说"反向时 `offset` / `end` 的语义如何变"（`direction` 页只有上面那两句 ＋
//! ARIA 的 Practical Considerations；`offset` 页只说"从第 `offset` 个采样起播"；
//! `end` 页只说"`end` 含端点、缺省整份、`-1` 不发声"）⇒ ⛔ **本单不发明**。
//! **工程口径（登记为裁决、⛔ 不声称是规范）**：`offset` / `end` 定义的是**同一个**
//! 半开区间 `[offset, min(end, pcm_len))`（[`reduce_span`] 是三条路共用的那一份归约），
//! 方向**只**改变这个区间**被走过的方向** —— 反向从区间**最后一格**起、走到下界止
//! （判据 `S96` 把"起点 ＝ `end − 1`、终点 ＝ `offset`、`offset` 那一格**读不到**"
//! 逐帧钉住）。⚠ 这条口径的唯一依据是"区间归约在三条路之间**不可能分叉**"这条机械纪律
//! ＋ `direction` 页的 "affects the **entire sample**"；⛔ 不是从规范推出来的。
//!
//! ## 语料普查（2026-10-16 现读；口径：`assets/samples/**/*.sfz` **剥掉 `//` 注释**后
//! 数 `name=` 形式的 opcode token）
//!
//! | 读数 | 值 |
//! | :--- | :--- |
//! | 文件分母 | **1398** 份 `.sfz` |
//! | opcode token 总数 / 不同 opcode | 383049 / 866 |
//! | ⭐ `direction` | ⭐ **0** 处（三种口径一致：token 0 ／ 不剥注释子串 0 ／ `name\s*=` 0） |
//! | 已知非零对照 `ampeg_attack` | ⭐ **835** 处（⭐ 同一份数据的**不剥注释**口径是 **844** —— 差的 9 处落在 `//` 注释里；两个口径都如实登记） |
//! | 对照 `loop_type` | ⭐ **0** 处（⇒ `alternate` 的"家"在本仓同样一个字节都没有） |
//! | 对照 `loop_mode` / `offset` / `end` / `pan` / `off_time` | 294 / 2637 / 56 / 1221 / 16 处 |
//!
//! ⚠ ⭐ **0 处 ⛔ 不构成"不做"的理由**（第 271／274 条）：语料是**夹具**，不是用户 SFZ 的
//! 全集 —— `direction` 是 SFZ v2 的**规范取值**，本单按规范实现并由**构造夹具**判据
//! （S94～S99）。
//!
//! ## 尺寸（本单机械读数）
//!
//! `PreparedRegion` `6152 → ` **6160 B**（＋**8** ＝ `direction: PlayDirection`
//! （`size_of` ＝ 1）＋ `#[repr(Rust)]` 重排后的尾部对齐；判据 `S98` 由 `size_of` 现读）；
//! ⛔ `SfzVoice` 仍 **40 B**、⛔ `SfzVoicePool` 仍 **1712 B**、
//! ⛔ `EngineRuntime` 仍 **180144 B**（门槛 192 KiB ＝ `196608`，余量 `16464`）——
//! 方向进的是**摊平表**（控制侧构造、随快照按值走），⛔ 不进任何实时对象。
//! ⚠ ⭐ **被修订的只是"当前值"那一档**：`#108` / `#109` / `#110` 各节里
//! `PreparedRegion` 的**历史读数**（`4232 → 5768` / `5768 → 6152` / `仍 6152`）
//! ⛔ 一字未删，它们各自是**当时**的实测值。
//!
//! # ⭐⭐ `ampeg_attack` 三档计数的**口径**（任务单 `#123`）
//!
//! ⭐ ⭐ **追加修订段（任务单 `#123`）—— 被修订的支（四处，⛔ 上面所有登记的原文一字未删，
//! 口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋ 点名被修订的支）**：
//! ① `#113` 节语料表里「已知非零对照 `ampeg_attack` | ⭐ **835** 处（⭐ 同一份数据的
//! **不剥注释**口径是 **844** —— 差的 9 处落在 `//` 注释里；两个口径都如实登记）」那一**行**；
//! ② `#115` 节语料表里「三档对照（`ampeg_attack`） | 家族子串 **2828** ／ 不剥注释 token
//! **844** ／ **剥注释 token 835**」那一**行**；③ `#116` 节语料表里同名的「三档对照
//! （`ampeg_attack`）」那一**行**；④ `#112` 节语料表里「已知非零对照（同一次扫描） …
//! `ampeg_attack` **2828** 处 / **368** 文件」那一**格**（⭐ 本模块下方那份**同文本的 `///`
//! 副本**（`CC_SEED_CAP` 一族文档里）同属被修订的支）。
//! ⭐ 这四处**只写了数**（① 例外地写了"剥／不剥注释"，台账 `:11461` 那句把 844 写成
//! "**精确 token**"），⛔ **没有一处写出左边界规则** —— ⭐ 而**左边界正是** `844` 与 `835`
//! 的唯一区别（见下）。
//!
//! ⭐ ⭐ **被修订的结论**：`#122` 的登记写「**844 与 2828 复现不出**、差 **+9** ⇒ 按第 289 条
//! 如实登记"复现不出"」 ⇒ ⭐ **本单两个都复现出来了** —— `#122` 的两把尺子各自**换错了一档**：
//! ① 它量的是**剥注释**子串 **2819**（⛔ 不是登记口径 2828）；② 它用的是 `(^|\s)` 左边界
//! （那个口径下"剥／不剥注释"**都得 835**）。⚠ ⭐ 教训：**"不剥注释 token"这句话没有唯一读数** ——
//! 同一个"不剥注释"在两种左边界下差 **9** ⇒ 口径必须**点名左边界**（台账第 274／289 条）。
//!
//! **分母 / 文件集合**（三档同一个母体）＝ `assets/samples/**/*.sfz`，**1398** 份
//! （`find assets/samples -name '*.sfz' | wc -l` ⇒ **1398**；⭐ 另一把尺子
//! `git ls-files '*.sfz'` 也 ⇒ **1398** ⇒ 本仓 ⛔ 没有母体之外的 `.sfz`、⛔ 没有 `.sfzh`、
//! ⛔ 没有大写扩展名 ⇒ "扩展名集合"这一档在本仓**不产生差别**；⭐ `crates/**` 与
//! `spikes/**` 下 **0** 个 `.sfz`，`assets/` 下的 `.sfz` 全部落在 `assets/samples/`）。
//!
//! | 档 | ⭐ 口径（逐字） | 对象 ＋ 单位 | 计数 |
//! | :--- | :--- | :--- | ---: |
//! | ① 家族**子串** | 裸子串 `ampeg_attack`，**任意位置**、⭐ **⛔ 不剥注释**（`//` 之后也数）、大小写敏感（语料无其它拼写 ⇒ 不敏感同值）；`ampeg_attack_oncc1=` 也是命中 | `grep -o` 的**匹配次数** ／ 含 ≥1 匹配的**文件数** | **2828** 次 ／ **368** 文件 |
//! | ② **不剥注释** token | `\bampeg_attack\b\s*=`（左边界 ＝ 行首或**任一非 `[A-Za-z0-9_]` 字节**，⭐ 含 `/`），⭐ **⛔ 不剥注释** | 同上 | **844** 次 ／ **318** 文件 |
//! | ③ **剥注释** token | 同 ②，但每行先截掉 `//` 到行尾 | 同上 | **835** 次 ／ **310** 文件 |
//!
//! ⭐ ⭐ **② 与 ③ 的唯一差别 ＝ 9 行**：`//ampeg_attack=0.001` ——
//! `assets/samples/virtuosity-drums/Programs/mappings/` 下 **9** 个文件各 **1** 行
//! （`kickmic_all` :205 ／ `kickmic_basic` :62 ／ `lofi_all` :193 ／ `mid_all` :190 ／
//! `oh_all` :190 ／ `oh_basic` :60 ／ `room_all` :191 ／ `snaremic_all` :195 ／
//! `snaremic_basic` :63）。
//! ⭐ 这 9 行的前一个字节是 `/`：`\b` 型左边界**收**它（⇒ ② **844**）、
//! `(^|\s)` 型左边界**不收**它（⇒ 那个口径下"剥／不剥注释"**都得 835**）、剥注释后
//! **谁也收不到**（⇒ ③ **835**）。⭐ 同理 ① −（**剥注释**子串）＝ **2828 − 2819 ＝ 9**，同一个 9。
//! ⇒ ⭐⭐ **2828 与 844 都复现得出**；差额 **+9** 的成因 ＝ **同一批 9 行注释里的 opcode 文本**，
//! ⛔ **不是** `#include` 展开（本仓解析器**会**展开 `#include`，见下面第 ⑥⑦ 档：
//! 展开后差一个**量级**）。
//!
//! ## ⭐ 口径矩阵（⭐ 每一格都可就地重算）
//!
//! | # | 口径（边界 × 注释 × 大小写 × 文件集合） | 次数 ／ 文件数 |
//! | :--- | :--- | ---: |
//! | ① | 子串，**任意位置**，**⛔ 不剥注释**，大小写敏感（＝**登记档**） | **2828** ／ **368** |
//! | ② | token，左边界 `\b` ／ `(?<![\w$])`，**⛔ 不剥注释**（＝**登记档**） | **844** ／ **318** |
//! | ③ | token，左边界 `\b`，**剥** `//` 注释（＝**登记档**） | **835** ／ **310** |
//! | ④ | 子串，**剥** `//` 注释（⭐ `#122` 勘察到的 **2819** 就是这一档） | **2819** ／ **360** |
//! | ⑤ | token，左边界 `(^|\s)`（行首或**空白**），**⛔ 不剥注释** ／ **剥注释**（⭐ 两档**同值**） | **835** ／ **310** |
//! | ⑥ | ⭐ **`#include` 展开后**的子串（解析器尺，**⛔ 不剥注释**） | **5144** ／ **353** |
//! | ⑦ | ⭐ **`#include` 展开后**的 token（解析器尺，`\b` 左边界，**⛔ 不剥注释**） | **1000** ／ **315** |
//! | ⑧ | 宏名 opcode 位 `ampeg_attack$VAR=` ／ 跨行（`ampeg_attack` 换行后再 `=`） ／ 大小写不敏感 ／ ⚠ 也剥 `/* */`（**口径陷阱**） | **0** ／ **0** ／ 与 ① 同值 ／ ⚠ **2786** ／ **359** |
//!
//! ⚠ ⭐ **第 ⑧ 档那个 2786 是"陷阱读数"，⛔ 不是三档的候选**：SFZ **没有**块注释语法
//! （解析器 [`crate::parser`] 的 `strip_comment` **只**认 `//`，且引号内的 `//` 不截断）——
//! 语料里那 **18** 处 `/*` 字节**全部**落在 `//****…` 分隔横幅里；⭐ 一个**不容错**
//! （找不到 `*/` 就一直剥到文末）的天真剥法会在 **9** 个文件里把整份文件吞掉
//! （`ixox-flute` ＋ `mtg-solo-sax` 的 8 份，共丢 **42** 处 ＝ 2828 − 2786）。
//! ⭐ 登记它的用途只是**点名一个会静默改读数的坑**。
//!
//! ⚠ **单位警告（台账第 271／274 条）**：本仓语料上 `ampeg_attack` 的**匹配次数**与
//! **含匹配的行数**恰好都是 **2828**（每命中行恰 **1** 处）⇒ 两个单位在本仓**偶然同值**，
//! ⛔ 不可互换；⭐ 登记与判据**一律以"次数 ／ 文件数"为单位**。
//!
//! ⚠ **第 ⑥⑦ 档的尺子（解析器尺，⛔ 未入库的 `/tmp` 探针）**：对 **1398** 个登记 `.sfz`
//! 逐个把 **它自己所在目录**当基准目录（`IncludeResolver::new(父目录, ParseLimits::default())`）
//! 再 `resolve(文件名)`，把 `SfzSource::text` **按顺序拼接**后数 —— 即**解析器真正看到的东西**。
//! 实测：`resolve` 成功 **1291** ／ include 解析失败 **107**（⭐ 与 `yeban-sfz/src/control.rs`
//! 登记的 **107** 逐字吻合；失败的主因是 `ixox-flute` 那 **108** 处 `#include "Data/*.txt"`
//! 越出 `.sfz`/`.sfzh` 允许集）／ 其中 `parse_sources` 报错 **26**。
//! ⚠ 显式登记的**禁读**：同一次展开**不是**三档的母体（⭐ 展开会把被 include 的文件
//! 在每个引用它的根文件里**再数一遍** ⇒ 读数**更大**且**依赖基准目录的选择**：
//! 若把基准目录取成 `assets/samples` 根，则是 **2596** ／ **296** 文件）——
//! ⭐ 三档**一律是"逐文件原文尺"**，⛔ 与展开无关。
//!
//! ## ⭐ 可重跑命令（分母：`find assets/samples -name '*.sfz' | wc -l` ⇒ **1398**）
//!
//! ```text
//! ① grep -roE 'ampeg_attack' assets/samples --include=*.sfz | wc -l
//!      ⇒ 2828（文件数：grep -rlE 'ampeg_attack' assets/samples --include=*.sfz | wc -l ⇒ 368）
//! ② grep -roE '[[:<:]]ampeg_attack[[:>:]][[:space:]]*=' assets/samples --include=*.sfz | wc -l
//!      ⇒ 844
//! ③ 逐文件先 sed 's|//.*||' 再按 ② 的式子 ⇒ 835
//! ④ 逐文件先 sed 's|//.*||' 再按 ① 的式子 ⇒ 2819
//! ⑤ grep -roE '(^|[[:space:]])ampeg_attack[[:space:]]*=' assets/samples --include=*.sfz | wc -l
//!      ⇒ 835
//! ⑥⑦ 见上「第 ⑥⑦ 档的尺子」
//! ```
//!
//! ⭐⭐ **三档 ＋ 六档矩阵 ＋ `#include` 展开档**都由判据 `S116` 就地重算并打印
//! （`cargo test -p yeban-engine --test sfz_instrument s116 -- --nocapture`）。
//!
//! ## ⭐ 跨单可比性：可直接引用的口径声明
//!
//! ⭐ ⭐ **本台账的 `ampeg_attack` 三档口径 ＝ ① 裸子串（⛔ 不剥注释、任意位置、大小写敏感）
//! 2828 ／ ② `\b…\b\s*=` token（⛔ 不剥注释）**844** ／ ③ 同 ② 但剥 `//` 注释 **835**；
//! 分母 ＝ `assets/samples/**/*.sfz` **1398** 份；对象 ＋ 单位 ＝ 匹配**次数** ／ 含匹配的
//! 文件数（⛔ 不是行数）。其他口径见上面矩阵的 ④～⑧。**
//! ⭐ 引用这三档时**必须**同时带上"剥／不剥注释"**与**"左边界"：二者缺一就复现不出
//! （⭐ `#122` 那次勘察两处都缺）。
//!
//! # ⭐ 增益族目标推广（任务单 `#115`）—— 首个目标 `amplitude`
//!
//! ⭐ ⭐ **追加修订段（任务单 `#115`）—— 被修订的是 `#108` 第 2 条里
//! 「`pan_oncc*` ／ `volume_oncc*` 一族 … 是 `#104` 那套机制**的推广** … **另开单**」
//! 那一支**（⛔ 上面所有登记的**原文一字未删**，口径同台账第 278 条：
//! 保留原文 ＋ 追加修订段 ＋ 点名被修订的支）。**被修订的只是"`amplitude` 这一族"**：
//! 它**不再是"另开单"** —— 本单把 `#104` 的机制**推广到增益族目标**，第一个目标就是
//! `amplitude`（`amplitude` ／ `amplitude_ccN` ／ `amplitude_onccN` ／ `amplitude_curveccN`）。
//! ⛔ **`pan_oncc*` ／ `volume_oncc*` ／ `tune_oncc*` ／ `cutoff_oncc*` ／ `pitch_oncc*`
//! 那一支整条有效**（仍未做，另开单）—— 与本单是**同一推广**的**不同目标**。
//!
//! ## 规范逐字（<https://sfzformat.com/opcodes/amplitude/>，2026-10-16 现读）
//!
//! > Amplitude for the specified region in percentage of full amplitude.
//! > 100% amplitude meaning no volume change is applied to the sample when played.
//! > **Can be modulated with MIDI CC.**
//!
//! > Note: Range in the table below is 0 to 100, but some players such as
//! > Aria/Sforazando do not clamp the range. **Negative values invert the signal.**
//!
//! | Name | Version | Type | Default | Range | Unit |
//! | :--- | :--- | :--- | :--- | :--- | :--- |
//! | `amplitude` | ARIA | float | **100** | 0 to 100 | **%** |
//! | `amplitude_onccN` | （空） | float | N/A | **-100 to 100** | **%** |
//! | `amplitude_curveccN` | （空） | integer | **0** | **0 to 255** | N/A |
//! | `amplitude_smoothccN` | （空） | N/A | N/A | （空） | N/A |
//!
//! ⭐ **`amplitude` 与 `volume` 的关系**（唯一一处把两者写在一起的规范页
//! <https://sfzformat.com/opcodes/volume/> 的收尾段，逐字）：
//!
//! > For the final output level, you can expect the sample level to be adjusted by a
//! > combination of the following:
//! > - volume (as adjusted by, e.g. rt_decay, amp_keytrack and other direct modifiers)
//! >   **in decibels**
//! > - **linear amplitude adjustment by ampeg... opcodes, along with adjustments to that
//! >   by amplitude opcodes**
//! > - pan
//!
//! ⇒ `volume` 是 **dB**、`amplitude` 是**线性百分比**，两者**相乘**（⛔ 不是别名、
//! ⛔ 不是又一段 dB）。公式与逐条裁决的**唯一事实源**是
//! [`yeban_sfz::modulation`] 的模块文档 §5（⛔ 本节不复制）。
//!
//! ## 落地形状（三条，逐条可判）
//!
//! 1. **目标推广 ⟹ 复用，⛔ 不另起系统**：`yeban-sfz` 的 `ModTarget`（五个 `ampeg_*`
//!    在前、`Amplitude` 在末尾）把「目标」从写死的 `AmpegParam` 抽象出来；
//!    归约仍是**一张** `BTreeMap<(目标, cc), …>`、摊平表仍是**一个**
//!    `[BakedCcMod; AMPEG_MOD_CAP]`、求值仍是**同一份**
//!    `cc_value_with_baked_and_vel2`（`#104`／`#106` 的薄包装原样保留）
//!    ⇒ ⛔ **没有第二套公式、没有第二张表**。⭐ 五个 `ampeg_*` 的声明序**逐位不变**。
//! 2. **三条路同一处施加点**：`amplitude_gain` ＝
//!    `(amplitude + Σ delta × curve(cc)) / 100` 是**最外层**线性乘子 ——
//!    离线 [`SfzInstrument::render_note_into_with_cc`] 走
//!    `playback_spec_with(..).total_gain() * amplitude_gain`；摊平／实时走
//!    [`PreparedRegion::gain_at_with_cc`]（`(base * crossfade) * amplitude_gain`）
//!    ⇒ **离线 ＝ 摊平 ＝ 实时**逐位相同（判据 `S101`／`S102`／`S103`）。
//! 3. **零新逐声部字段**：`amplitude` 与调制项都是**区域参数**（CC 逐块从探针读）
//!    ⇒ ⛔ `SfzVoice` 尺寸不变（40 B）。
//!
//! ## 语料普查（2026-10-16 现读；口径同 `#104`：`assets/samples/**/*.sfz` 剥 `//` 注释）
//!
//! 分母 **1398** 份 `.sfz`：
//!
//! | 读数（对象＋单位：opcode **出现次数**／**文件数**） | 值 |
//! | :--- | :--- |
//! | `amplitude_onccN`（**带数字下标**） | **420** 次 ／ **66** 文件 |
//! | `amplitude_ccN`（SFZ1 别名） | **854** 次 ／ **83** 文件 |
//! | `amplitude_curveccN` | **30** 次 |
//! | 静态 `amplitude=` | **1** 次（`cithara-barbarica` 的静音区 `amplitude=0`） |
//! | 三档对照（`ampeg_attack`） | 家族子串 **2828** ／ 不剥注释 token **844** ／ **剥注释 token 835** |
//! | 单文件 `(目标, cc)` **并集**最大（两族合计） | **11** ≤ `AMPEG_MOD_CAP`（20） |
//!
//! ⚠ **条数上界沿用 [`AMPEG_MOD_CAP`] ＝ 20**：`#115` 的并集读数 **11**（含 `amplitude`
//! 族）＜ `#104` 登记的 **15** ⇒ ⛔ **不抬高上限、⛔ 不引入新 `Err`**
//! （⇒ 今天能解析的文件一个都不会变成不可解析）。
//!
//! ## 尺寸（本单机械读数）
//!
//! `PreparedRegion` **仍 6160 B**（`#115` 的 `amplitude: f32` 落进既有尾部对齐空隙
//! —— **＋0**；判据 `S100` 由 `size_of` 现读）；⛔ `SfzVoice` 仍 **40 B**、
//! ⛔ `SfzVoicePool` 仍 **1712 B**、⛔ `EngineRuntime` 仍 **180144 B**
//! （门槛 192 KiB ＝ `196608`，余量 `16464`）。
//!
//! # ⭐⭐ `tune` 目标（任务单 `#116`）—— 第二个被推广的目标
//!
//! ⭐ ⭐ **追加修订段（任务单 `#116`）—— 被修订的是 `#115` 那一节里
//! 「⛔ **`pan_oncc*` ／ `volume_oncc*` ／ `tune_oncc*` ／ `cutoff_oncc*` ／
//! `pitch_oncc*` 那一支整条有效**（仍未做，另开单）」里 **`tune_oncc*` 这一项**
//! **（⛔ 上面所有登记的**原文一字未删**，口径同台账第 278 条：保留原文 ＋
//! 追加修订段 ＋ 点名被修订的支）。**被修订的只是"`tune_oncc*` 仍未做"这一支**：
//! 它**不再是"另开单"** —— 本单把 `#104`／`#115` 的机制**推广到 `tune`**
//! （`tune` ／ `tune_ccN` ／ `tune_onccN` ／ `tune_curveccN`）。
//! ⛔ **`pan_oncc*` ／ `volume_oncc*` ／ `cutoff_oncc*` ／ `pitch_oncc*` 那四项
//! 整条有效**（仍未做，另开单）—— 与本单是**同一推广**的**不同目标**
//! （判据 `S106` 把这条修订逐字钉住）。
//!
//! ## 规范逐字（<https://sfzformat.com/opcodes/tune/>，2026-10-16 现读）
//!
//! ⚠ ⚠ **本单最重要的一条事实裁决**：该页的 **Modulation 表里 ⛔ 没有
//! `tune_onccN` / `tune_ccN` / `tune_curveccN` 行**，它列的是 `pitch` 族
//! （`pitch_onccN` Range ＝ **`-9600 to 9600`**、Unit ＝ **cents**；
//! `pitch_curveccN` Range ＝ `0 to 255`）。`tune` 族与被别名者的关系**只有一句**：
//!
//! > In ARIA, `tune_*ccN` can also be used as an alias for `pitch_*ccN`.
//!
//! `tune` 自己的表行是 **`| tune | SFZ v1 | integer | 0 | -100 to 100 | cents |`**，
//! 正文逐字「The fine tuning for the sample, **in cents**.」。⭐ 同页例子
//! `tune=-100` ＋ `pitch_oncc27=200` 里的 `200` **超出** `tune` 自身的 `±100`
//! ⇒ 规范文本自己证明"**调制量的域 ≠ 基准的域**"。完整依据链、域裁定与语料普查
//! 的**唯一事实源**是 [`yeban_sfz::modulation`] 的模块文档 §6（⛔ 本节不复制）。
//!
//! ## 落地形状（三条，逐条可判）
//!
//! 1. **目标枚举 ＋1**：`yeban_sfz::modulation::ModTarget::Tune` 加在 [`ModTarget::Amplitude`]
//!    **之后**（下标 6）⇒ 既有六个目标的下标**一个都没动**（求和序逐位不变）；
//! 2. **三条路同一处施加点**：`tune` 只加**音分**、不改方向、不改区间 ⇒ 它是
//!    **速率上的一个线性乘子** `2 ^ (Σ delta × curve(cc) / 1200)`
//!    （[`tune_ratio_with_baked`] 是唯一实现）——
//!    离线 [`SfzInstrument::render_note_into_with_cc`] 乘在 `Region::playback_rate` 上、
//!    摊平 [`PreparedSfz::render_block_into_with_cc`] 乘在 `rate_per_note[n]` 上、
//!    实时 [`SfzVoicePool::slot_readings`] 乘在 `SfzVoice::rate()` 上
//!    ⇒ **离线 ＝ 摊平 ＝ 实时**逐位相同（判据 `S108`／`S109`）；
//! 3. **零新逐声部字段**：乘子与基准速率都是**区域参数 ＋ 触发期副本**
//!    （CC 逐块从探针读）⇒ 逐块算出的 `rate` 活在**栈上**的 `SlotReadings` 里，
//!    ⛔ 不回写 [`SfzVoice`]（⛔ 不能回写：那会让乘子**逐块复合**）⇒
//!    `SfzVoice` 仍 **40 B**（判据 `S9`／`S100`／`S106`）。
//!
//! ## 语料普查（2026-10-16 现读；口径同 `#104`／`#115`：`assets/samples/**/*.sfz`
//! 剥 `//` 注释后数 `(^|\s)name[0-9]*\s*=` 形式的 opcode token）
//!
//! 分母 **1398** 份 `.sfz`：
//!
//! | 读数（对象＋单位：opcode **出现次数**／**文件数**） | 值 |
//! | :--- | :--- |
//! | `tune_onccN`（**带数字下标**） | **9442** 次 ／ **83** 文件 |
//! | `tune_ccN`（SFZ1 别名） | **608** 次 ／ **87** 文件 |
//! | `tune_curveccN` | **218** 次 ／ **45** 文件 |
//! | 静态 `tune=` | **4120** 次 ／ **154** 文件 |
//! | 裸 `tuneccN`（⛔ 规范无此行、语料 **0** 处） | **0** 次 ／ **0** 文件 |
//! | `tune_smoothccN` ／ `tune_stepccN`（⛔ 不属本族） | **0** ／ **1** 次 |
//! | 对照 `pitch_onccN`（⛔ 本单**不做**的那一族） | **39** 次 ／ **1** 文件 |
//! | 三档对照（`ampeg_attack`） | 家族子串 **2828** ／ 不剥注释 token **844** ／ **剥注释 token 835** |
//! | `tune_*ccN` 的 `|delta| > 100` 处数 ／ 最大 `|delta|` | **599** ／ **2400** 音分 |
//! | 单文件 `(目标, cc)` **并集**最大（**三族**合计） | **15** ≤ `AMPEG_MOD_CAP`（20） |
//!
//! ⚠ **条数上界沿用 [`AMPEG_MOD_CAP`] ＝ 20**：`#116` 的三族并集读数 **15**
//! （与 `#104` 登记 `ampeg_*` 一族时的 15 是**同一个文件同一个数**）
//! ≤ **20** ⇒ ⛔ **不抬高上限、⛔ 不引入新 `Err`**
//! （⇒ 今天能解析的文件一个都不会变成不可解析）。
//!
//! ## 尺寸（本单机械读数）
//!
//! `PreparedRegion` **仍 6160 B**（⛔ **＋0**：本单**没有**新增任何每区域字段 ——
//! 基准 `tune` 早已烘进 `rate_per_note`、调制项早已在 `cc_mods` 里；判据 `S106`
//! 由 `size_of` 现读）；⛔ `SfzVoice` 仍 **40 B**、⛔ `SfzVoicePool` 仍 **1712 B**、
//! ⛔ `EngineRuntime` 仍 **180144 B**（门槛 192 KiB ＝ `196608`，余量 `16464`）。
//!
//! # ⭐⭐ `pitch` 族（任务单 `#118`）—— `tune` 的**别名拼写**，⛔ 零引擎改动
//!
//! ⭐ ⭐ **追加修订段（任务单 `#118`）—— 被修订的是 `#116` 那一节里
//! 「⛔ **`pan_oncc*` ／ `volume_oncc*` ／ `cutoff_oncc*` ／ `pitch_oncc*` 那四项
//! 整条有效**（仍未做，另开单）」里 **`pitch_oncc*` 这一项**
//! **（⛔ 上面所有登记的**原文一字未删**，口径同台账第 278 条：保留原文 ＋
//! 追加修订段 ＋ 点名被修订的支）。**被修订的只是"`pitch_oncc*` 仍未做"这一支**：
//! ⛔ **`pan_oncc*` ／ `volume_oncc*` ／ `cutoff_oncc*` 那三项仍然有效**（仍未做，另开单）。
//!
//! ## 规范逐字（<https://sfzformat.com/opcodes/tune/>，2026-10-17 现读）
//!
//! 该页（`…/opcodes/pitch/` 落到的也是这一页）Modulation 表**逐字四行**：
//! `pitch_onccN`（Range ＝ **`-9600 to 9600`**、Unit ＝ **cents**）、
//! `pitch_curveccN`（Default `0`、Range ＝ `0 to 255`）、
//! `pitch_smoothccN`（float，Default `0`、Unit ＝ **ms**）、
//! `pitch_stepccN`（Default `0`）。⭐ 裁定依据**只有一句**（同页逐字）：
//!
//! > In ARIA, `tune_*ccN` can also be used as an alias for `pitch_*ccN`.
//!
//! ⚠ 该页的**静态**表只有 `tune` 一行 ⇒ ⛔ **规范里没有 `pitch` 这个静态 opcode**
//! ⇒ `pitch_*ccN` 的基准就是 **0 音分**（只加调制量）；⛔ 本单**不发明** `pitch=`。
//! 完整裁定、理由与代价的**唯一事实源**是 [`yeban_sfz::modulation`] 模块文档 §7
//! （⛔ 本节不复制）。
//!
//! ## 落地形状（三条，逐条可判）
//!
//! 1. **同一个目标的两个拼写**：`pitch_onccN` / `pitch_ccN` / `pitch_curveccN` 归
//!    **同一个** `ModTarget::Tune`（⛔ 不新增枚举变体、⛔ 不改域、⛔ 不改单位类别）
//!    ⇒ 归约表、求和序与 `AMPEG_MOD_CAP` 全不变；
//! 2. ⭐ ⭐ **引擎侧零改动**：三条路（离线 [`SfzInstrument::render_note_into_with_cc`] ／
//!    摊平 [`PreparedSfz::render_block_into_with_cc`] ／ 实时
//!    [`SfzVoicePool::render_into_with_cc`]）**早已**把 `ModTarget::Tune` 喂给
//!    [`tune_ratio_with_baked`] ⇒ `pitch_*ccN` **自动**获得与 `tune_*ccN` **逐位相同**
//!    的作用点与公式（判据 `S113` 的"三条路逐位相同"腿）；
//! 3. **四个尺寸全 ＋0**：`PreparedRegion` 仍 **6160 B**、`SfzVoice` 仍 **40 B**、
//!    `SfzVoicePool` 仍 **1712 B**、`EngineRuntime` 仍 **180144 B**（`S113` 现读）。
//!
//! ## 语料普查（2026-10-17 现读；分母 **1398** 份 `.sfz`）
//!
//! ⭐ ⭐ **口径决定读数**（台账第 274 条）：`pitch_onccN=` 的**家族子串**是
//! **143** 处 ／ **33** 文件，而**opcode token**（词边界 ＋ `=`）是 **39** 处 ／
//! **1** 文件 —— 那 **104** 处差额**全是别的族**：`lfoNN_pitch_onccNN=`（LFO 目标）
//! 与 `egNN_pitch_onccNN=`（包络目标），⛔ 不是 `<region>` 的调制 opcode。
//! 唯一真正用到本族的是 `assets/samples/karoryfer-bear-sax/Programs/5-bearcussion.sfz`
//! （同一文件里 `pitch=-1200` **一处** —— ⛔ 那是个**未知 opcode**，今天被忽略，
//! 本单⛔ 不给它实现）。⭐ `pitch_curveccN` ／ `pitch_ccN` ／ `pitch_smoothccN` ／
//! `pitch_stepccN` 全部 **0** 处。⚠ 本单⛔ 不抬高 `AMPEG_MOD_CAP`（单区域最多
//! 2 条 ⇒ 离 20 远）。
//!
//! # ⭐⭐ `volume` 族（任务单 `#119`）—— **第三个单位类别**（dB），三条路同一处施加点
//!
//! ⭐ ⭐ **追加修订段（任务单 `#119`）—— 被修订的是 `#104`／`#115`／`#116`／`#118`
//! 那几条登记里**「`pan_oncc*` ／ `volume_oncc*` ／ `cutoff_oncc*` 那一族仍未做、
//! 另开单」**这一支里 **`volume_oncc*` 这一项**（⛔ 上面所有登记的**原文一字未删**，
//! 口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋ 点名被修订的支）。
//! **被修订的只是"`volume_oncc*` 仍未做"这一支**：它**不再是"另开单"** —— 本单把
//! `#115` 的增益族机制**推广到 `volume`**（`volumeccN` ／ `volume_ccN` ／
//! `volume_onccN` ／ `volume_curveccN` 四种拼写）。
//! ⛔ **`pan_oncc*` ／ `cutoff_oncc*` 那两项整条有效**（仍未做，另开单）；
//! ⛔ **规范 Modulation 表里逐字的 delta 行名 `gain_ccN`／`gain_onccN` 也不在本单**
//! （裁定与语料读数见 `yeban-sfz/src/modulation.rs` §8.4；判据 `S114` 把今天的
//! 可观察行为逐字钉住）。⭐ 判据 `S114` 同时 `contains` 上面这几条**被修订的原文**
//! 与**本修订段**（台账第 280 条：让"有修订"本身成为可判事实）。
//!
//! ## 规范逐字（<https://sfzformat.com/opcodes/volume/>，2026-10-17 现读）
//!
//! ⚠ ⚠ **本单最承重的一条事实**：该页 **Modulation 表里⛔ 没有 `volume_onccN` 行**，
//! 它的 delta 行逐字是 **`| gain_ccN |  | float | 0 | -144 to 48 | dB |`**，
//! 而同页正文写「**`volume_ccN` is a valid alias for `gain_ccN`**」
//! ⇒ `volume` 族的 delta 域随被别名的那一行（`-144 to 48` dB）。
//! ⭐ 静态表行是 `| volume | SFZ v1 | float | 0 | -144 to 6 | dB |`；
//! `volume_curveccN` 行是 `0 to 255`／`N/A`（**曲线下标**），
//! `volume_smoothccN`／`volume_stepccN` 两行 **Range 空、Unit `N/A`**（⛔ 本单不做）。
//! 完整依据链、单位裁定（**新开** `ModUnit::Decibels` 的四条理由与代价）与语料普查的
//! **唯一事实源**是 [`yeban_sfz::modulation`] 的模块文档 §8（⛔ 本节不复制）。
//!
//! ## 落地形状（三条，逐条可判）
//!
//! 1. **目标枚举 ＋1 ＋ 单位类别 ＋1**：`ModTarget::Volume` 加在 [`ModTarget::Tune`]
//!    **之后**（下标 7）；它的 `mod_unit()` 是新变体 `ModUnit::Decibels`
//!    ⇒ 既有七个目标的下标与 `is_time()` 读数**一个都没动**（`S106` 仍绿）；
//! 2. **三条路同一处施加点**：基准 `volume`（dB）早就烘进摊平表的 `volume_gain`
//!    （[`PreparedRegion::gain_at_with_cc`] 的 `base`）与离线
//!    [`yeban_sfz::playback::PlaybackSpec::total_gain`] ⇒ 本族只贡献**一个新的最外层
//!    线性乘子** `volume_ratio`（`yeban_sfz::modulation::volume_ratio_with_baked`
//!    是唯一实现，＝ `2 ^ (dB × log2(10) / 20)`）：
//!    离线 [`SfzInstrument::render_note_into_with_cc`] ＝
//!    `(total_gain() × amplitude_gain) × volume_ratio`；
//!    摊平 [`PreparedSfz::render_block_into_with_cc`] ／ 实时
//!    [`SfzVoicePool::slot_readings`] ＝ `((base × crossfade) × amplitude_gain) × volume_ratio`
//!    （[`PreparedRegion::volume_ratio_with_cc`]）⇒ **离线 ＝ 摊平 ＝ 实时**逐位相同
//!    （判据 `S114`，含五个病态臂）；
//! 3. **零新逐声部字段**：乘子是**区域参数**（`cc_mods` 早就在摊平表里）、
//!    CC 逐块从探针读、算出的是**栈上**的值 ⇒ ⛔ 不回写 [`SfzVoice`]
//!    ⇒ `SfzVoice` 仍 **40 B**（判据 `S9`／`S114`）。
//!
//! ## 语料普查（2026-10-17 现读；分母 **1398** 份 `.sfz`；剥 `//` 注释 ＋ 词边界 token）
//!
//! | 读数（对象＋单位：opcode **出现次数**／**文件数**） | 值 |
//! | :--- | :--- |
//! | `volume_onccN`（**裸**；⛔ 规范 Modulation 表无此行） | **2** 次 ／ **2** 文件 |
//! | `volume_curveccN` | **2** 次 ／ **2** 文件 |
//! | `volume_ccN`（正文说的 SFZ1 别名） | **0** 次 ／ **0** 文件 |
//! | 裸 `volumeccN` | **0** 次 ／ **0** 文件 |
//! | 静态 `volume=` | **8504** 次 ／ **284** 文件 |
//! | ⛔ `volume_smoothccN` ／ `volume_stepccN`（本单不做） | **0** 次 ／ **0** 文件 |
//! | ⛔ `gain_ccN` ／ `gain_onccN`（规范逐字的 delta 行名；本单不识别） | **0** 次 ／ **0** 文件 |
//! | ⛔ `global_volume_onccN` 一族（**另外三个** opcode） | **47** 次 ／ **32** 文件 |
//!
//! ⭐ **口径警告（台账第 271／274 条）**：`volume_oncc` 的**家族子串**是
//! **49** 次 ／ **32** 文件 —— 那 **47** 处差额**全是别的 opcode**
//! （`global_volume_onccN` 等），⛔ 不是本族的 `<region>` 调制 opcode。
//!
//! ⭐ ⭐ **追加修订段（任务单 `#121`）—— 被修订的是上面这张表里的三格**：
//! ① `global_volume_onccN` 一族 **47** 次 ／ **32** 文件、② 家族子串 **49** 次 ／ **32** 文件、
//! ③ 裸 `volume_onccN` **2** 次 ／ **2** 文件（⛔ 上表**原文一字未删**，口径同台账第 278 条：
//! 保留原文 ＋ 追加修订段 ＋ 点名被修订的支）。⭐ 每一条都按「**原登记 ⇒ 实测 ＋ 口径 ＋ 命令**」写。
//!
//! **本修订段唯一的计数口径**（台账第 274／287 条）：`assets/samples/**/*.sfz` 逐行**剥 `//` 注释**
//! ⇒ 取**行首或空白边界**的 opcode token ⇒ token 里 `volume_oncc` 之后的尾巴必须是**数字**
//! 或 **`$` 宏名**（`$F2_DIS`）⇒ ⭐ **收 `$` 宏名**（解析器就是把 `volume_oncc$F2_DIS`
//! 当本族的一个键读的）。依据命令：分母 `find assets/samples -name '*.sfz' | wc -l` ⇒ **1398**；
//! 逐名重算 `cargo test -p yeban-engine --test sfz_instrument s115 -- --nocapture`
//! （判据 `S115` 用**同一口径**就地重算并**逐名打印** `名字 = 次数 ／ 文件数`）。
//!
//! * ① 原登记「`global_volume_onccN` 一族 **47** 次 ／ **32** 文件」 ⇒ ⭐ **实测**：
//!   那三个 `_oncc` 家族（`global_volume_onccN` ／ `master_volume_onccN` ／ `group_volume_onccN`）
//!   **各 0 次 ／ 0 文件**；那 **47** 处的**真身是 `lfo*_volume_onccN`（47 次 ／ 30 文件）**。
//!   ⚠ 原登记的两个字段里**只有"次数"对**，**文件数是错的** —— 该族在**任何**测过的口径下
//!   都是 **30** 文件（⛔ 复现不出 **32**）。⚠ 对照静态 opcode：`global_volume=` **0**／**0**、
//!   `master_volume=` **0**／**0**、`group_volume=` **1**／**1**（后者在
//!   `assets/samples/ixox-flute/Ixox Flute.sfz:123`，⛔ 与本族不相干）。
//!   口径 ＝ 剥 `//` 注释 ＋ 行首/空白边界 token ＋ token 里 `volume_oncc` 前**有前缀**
//!   ＋ **收 `$` 宏名**；命令 ＝ `cargo test -p yeban-engine --test sfz_instrument s115 -- --nocapture`。
//! * ② 原登记「家族子串 **49** 次 ／ **32** 文件」 ⇒ ⭐ **实测 50 次 ／ 33 文件**
//!   （＝裸 **3** ＋ 前缀 **47**）。⚠ 原登记的 **49／32** 恰好是**不收 `$` 宏名**那个口径下的读数
//!   （本轮实测复现了它）⇒ 差的那 **1 次 1 文件**就是宏名那一处（见 ③）。
//!   口径 ＝ 剥 `//` 注释 ＋ `volume_oncc` 子串（**任意位置**，⛔ 不要求词边界）＋ **收 `$` 宏名**；
//!   命令 ＝ 同上（判据 `S115` 的 ② 档）。
//! * ③ 原登记「裸 `volume_onccN` **2** 次 ／ **2** 文件」 ⇒ ⭐ **实测 3 次 ／ 3 文件**：
//!   `volume_oncc24` **2** 次 ／ **2** 文件 ＋ `volume_oncc$F2_DIS` **1** 次 ／ **1** 文件
//!   （`assets/samples/ixox-flute/Ixox Flute.sfz:152`）—— 第三处**就是**宏名形式，
//!   按本修订段的口径**必须算本族**（解析器就是这么读的）。
//!   口径 ＝ 剥 `//` 注释 ＋ 行首/空白边界 token ＋ token 以 `volume_oncc` **开头**
//!   ＋ **收 `$` 宏名**；命令 ＝ 同上（判据 `S115` 的 ③ 档）。
//!
//! ⭐ **台账第 287 条：`volume_oncc` 家族子串命中的完整 opcode 名逐个列出**（`名字 = 次数／文件数`，
//! 顺序 ＝ 次数降序、同次数名字升序；共 **11** 个名字 ＝ **9** 个 `lfo*` ＋ **2** 个裸）：
//! `lfo01_volume_oncc114` = 13／6、`lfo01_volume_oncc113` = 7／7、`lfo02_volume_oncc113` = 6／6、`lfo01_volume_oncc101` = 5／4、`lfo03_volume_oncc118` = 5／4、`lfo01_volume_oncc100` = 4／4、`lfo01_volume_oncc21` = 3／3、`lfo02_volume_oncc110` = 2／2、`lfo03_volume_oncc122` = 2／2、`volume_oncc24` = 2／2、`volume_oncc$F2_DIS` = 1／1
//! ⚠ 那 9 个 `lfo*_volume_onccNNN` **一个都不是**本族的 `<region>` 调制 opcode
//! （它们也⛔ 不属于 `global_volume_onccN` 一族）—— 谁把它们算进**任何**一档，判据 `S115` 会红。
//!
//! ⚠ **口径敏感**（本修订段的读数**只**在上面那个口径下成立；换口径就换读数）：
//! **不收 `$` 宏名** ⇒ 裸 **2** ／ **2**、子串 **49** ／ **32**（⭐ 正是原登记的两个数
//! ⇒ `#119` 的原口径是"**不收宏名**"）；**不剥 `//` 注释** ⇒ 与本修订段**逐位相同**
//! （11 个名字**没有一个**出现在 `.sfz` 的注释里）；**词边界取严（仅行首）** ⇒ 也**逐位相同**
//! （11 个名字**没有一个**出现在行中）。
//! ⚠ 唯一真正用到本族的是 `assets/samples/karoryfer-black-and-blue-basses/Programs/controls/`
//! 的 `common.sfz` 与 `common_warm.sfz`（各 `volume_oncc24=7` ＋ `volume_curvecc24=11`）。
//! ⚠ 本单⛔ 不抬高 `AMPEG_MOD_CAP`（**四族**并集最大仍 **15** ＝ 与 `#104`／`#116`
//! 同一个文件同一个数 ⇒ 今天能解析的文件**一个都不会**变成不可解析）。
//!
//! # ⭐⭐ `gain_ccN`（任务单 `#124`）—— `volume` 的**逐字 delta 行名**，⛔ 零引擎改动
//!
//! ⭐ ⭐ **追加修订段（任务单 `#124`）—— 被修订的是上面 `#119` 那张语料表里的
//! 「⛔ `gain_ccN` ／ `gain_onccN`（规范逐字的 delta 行名；本单不识别）」**这一行**，
//! 以及 `#119` 修订段里「⛔ **规范 Modulation 表里逐字的 delta 行名 `gain_ccN`／`gain_onccN`
//! 也不在本单**」那一句**（⛔ 上面所有登记的**原文一字未删**，口径同台账第 278 条：
//! 保留原文 ＋ 追加修订段 ＋ 点名被修订的支）。⭐ **被修订的只是"`gain_ccN` 不识别"这一支**：
//! `#124` 起 `gain_ccN` **是识别的**（⛔ 不新开目标、⛔ 不新开单位 ⇒ 引擎侧**一个字节都没改**，
//! 它落在 [`yeban_sfz::modulation::parse_modulation_name`] 的识别层 ＋ 既有的
//! `volume_ratio` 施加点上）。
//!
//! ## 规范逐字（<https://sfzformat.com/opcodes/volume/>，2026-10-17 现读）
//!
//! `volume` 页 Modulation 表里**只有一行**以 `gain` 开头，逐字是
//! **`| gain_ccN |  | float | 0 | -144 to 48 | dB |`**（⛔ **没有** `gain_onccN`、
//! ⛔ **没有** `gain_curveccN`、⛔ **没有** `gain_smoothccN`、⛔ **没有** `gain_stepccN`）；
//! 同页正文两句逐字：「Extended opcodes, such as `volume_curveccN` shown above,
//! **only apply to `volume`**」＋「**`volume_ccN` is a valid alias for `gain_ccN`**」。
//! ⭐ **裁定：`gain_ccN` 归 `ModTarget::Volume`**（同一个目标、同一个
//! `ModUnit::Decibels`、同一个域 `-144.0 ..= 48.0`）—— 四条理由、代价与"只收一个拼写"
//! 的依据，**唯一事实源**是 [`yeban_sfz::modulation`] 的模块文档 §8.6（⛔ 本节不复制）。
//!
//! ## 落地形状（⛔ 零行为改动面 —— 只多一个**名字**）
//!
//! 1. **识别层**：`gain` ＋ `_cc` ＋ 数字 ⇒ `(ModTarget::Volume, ModulationForm::Delta, N)`；
//! 2. **求值层**：⛔ 一行都没改 —— 复用 `#119` 的
//!    [`PreparedRegion::volume_ratio_with_cc`]（＝ `yeban_sfz::modulation::volume_ratio_with_baked`，
//!    `2 ^ (dB × log2(10) / 20)`）⇒ 离线 ＝ 摊平 ＝ 实时**逐位相同**（判据 `S117`）；
//! 3. **零新字段**：`gain` 与 `volume` 是**同一个静态 opcode** 的两个名字
//!    （`scopes.get("volume").or_else(|| scopes.get("gain"))`）⇒ 目标相同 ⇒ ⛔ 不加变体、
//!    ⛔ 不加字段 ⇒ `SfzVoice` 仍 **40 B** ／ `SfzVoicePool` 仍 **1712 B** ／
//!    `EngineRuntime` 仍 **180144 B** ／ `PreparedRegion` 仍 **6160 B**（**四个全 ＋0**）。
//!
//! ## 语料普查（2026-10-17 现读；分母 **1398** 份 `assets/samples/**/*.sfz`）
//!
//! ⭐ **三元组口径** ＝ **剥／不剥 `//` 注释** × **左边界**（任意位置 ／ **行首或空白**）
//! × **收／不收 `$` 宏名** ⇒ ⭐ **8 档全部同值**（判据 `S117` 逐档现算并打印）：
//!
//! | 读数（对象＋单位：opcode **出现次数**／**文件数**） | 值 |
//! | :--- | :--- |
//! | 裸 `gain_ccN`（⭐ 规范逐字的 delta 行名） | **0** 次 ／ **0** 文件 |
//! | 裸 `gain_onccN`（⛔ 规范无此行） | **0** 次 ／ **0** 文件 |
//! | 裸 `gain_curveccN`（⛔ 规范无此行） | **0** 次 ／ **0** 文件 |
//! | 本族**之外**（`gain_cc`／`gain_oncc`／`gain_curvecc` 子串命中） | **43** 次 ／ **35** 文件 ／ **7** 个不同名字 |
//!
//! ⭐ **完整 opcode 名逐个列出**（第 287 条；顺序 ＝ 次数降序、同次数名字升序）：
//! `eq1_gain_oncc77` = 23／21、`lfo01_eq1gain_oncc114` = 7／7、`lfo02_eq1gain_oncc115` = 4／4、
//! `lfo01_eq1gain_oncc21` = 3／3、`eq1_gain_oncc1` = 2／2、`eq2_gain_oncc1` = 2／2、
//! `eq3_gain_oncc1` = 2／2。
//! ⚠ 那 **7** 个名字**一个都不是**本族：`eq*_gain_onccN` 是 **EQ 段增益**、
//! `lfo*_eq*gain_onccN` 是 **LFO 调制 EQ 增益**（⛔ 都不是 `volume` 目标）。
//! ⚠ **左边界必须写成"行首或空白"**：用 `[^a-zA-Z]` 当边界会把 `lfo02_eq1gain_oncc115`
//! 里的 `gain_oncc115` 数成**裸**用法（`#119` 的教训）⇒ 本表按**行首或空白**口径给 **0**。
//!
//! ⚠ **与 `#119` 登记的异同（第 287／289 条）**：⭐ **同** ＝ 裸 `gain_ccN` **0** ／
//! 裸 `gain_onccN` **0** —— `#119` 上面那张表的**两个 0 复核成立**；
//! ⭐ **异** ＝ `yeban-sfz` §8.4 写「那 **14** 处 `gain_onccN` 全是 `lfo*_eq1gain_onccN`」——
//! **14** 只是 **`lfo*` 那一族**（14 次 ／ 14 文件 ✓），另有 `eq*_gain_onccN` 一族
//! **29** 次 ／ **23** 文件 ⇒ `gain_oncc` **合计 43 次 ／ 35 文件**（⭐ 那 **14** 是**漏计**）。
//! ⭐ **复现不出的如实写（第 289 条）**：本线**复现不出**任何"`gain_oncc` 只有 14 处"的口径
//! （左边界取行首/空白 ⇒ **0**；取任意位置 ⇒ **43**）⇒ 只登记 **0**（裸档）与 **43**（本族之外）。
//!
//! ## ⛔ 本单**只**收 `gain_ccN`（同族申请被驳回的三条）
//!
//! ⛔ `gain_onccN` ／ ⛔ `gain_curveccN` ／ ⛔ `gainccN`（无下划线形）／
//! ⛔ `gain_smoothccN` ／ ⛔ `gain_stepccN` —— 依据：规范 Modulation 表里 `gain` 开头的
//! **只有 `gain_ccN` 这一行**，且正文把扩展拼写**限定在 `volume` 上**；`gain_onccN` 是
//! ARIA 通用形的**外推**（规范一个字都没写）⇒ 接它就＝**发明语义**。
//! ⭐ 今天的可观察行为由判据 `S117` **逐字**钉住（⛔ 不是"哪天变成识别到了但静默丢弃"）；
//! ⚠ `gain_smoothccN`／`gain_stepccN` 连**表行都没有**（⛔ 与 `volume_smoothccN` 不同）。
//!
//! # ⭐ `set_ccN` 播种（任务单 `#110`）
//!
//! ⭐ ⭐ **追加修订段（任务单 `#110`）—— 被修订的是下面三处登记里 `set_ccN=` 的
//! "**播种仍归调用方**"那一支**（⛔ 上面 `#104`／`#107`／`#108`／`#109` 的**原文一字未删**，
//! 口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋ 点名被修订的支）：
//!
//! 1. `#107` 第 3 条（「本单未接线（登记，⛔ 不静默）」）：那一整条里有**两个**题材
//!    —— `pan_oncc*` 一族与 `set_ccN=` 播种。**被修订的只有后者**：`set_ccN=` 的播种
//!    **已由本单落地**（机制见本节）；`pan_oncc*`／`volume_oncc*` 一族**整条有效**
//!    （⛔ 仍在"未接线"里，本单不碰）。
//! 2. `#108` 第 2 条的第二个圆点（「`set_ccN=` 的**播种**（初值）仍归调用方 —— 本节第 3 条
//!    的原文**整条有效**」）：被修订的**同样是** `set_ccN` 那一支。
//! 3. `#109`「⛔ 本单仍未接线（登记，⛔ 不静默）」的第二个圆点（「`<control> set_ccN=` 的
//!    **播种**（初值；语料 1137 处）仍归调用方」）：同上。⚠ 该节第三个圆点
//!    （「`set_ccN` 的初值与"会话表未 armed ⇒ 探针 `None`"是**两件事**」）**仍然成立**：
//!    本单正是靠"空表 ⇒ 一次都不写"把这两件事继续分开的。
//!
//! ## 规范逐字（<https://sfzformat.com/opcodes/set_ccN/>）
//!
//! > Sets a default initial value for MIDI CC number N, when the instrument is
//! > initially loaded.
//! >
//! > Used under the ‹`control`› header.
//!
//! 表格行：Type `integer`、Default `N/A`、**Range `0 to 127`**、Category
//! ＝ "Instrument Settings"；同页例子 `set_cc40=127` / `set_cc100=30`。解析层事实源
//! ＝ `yeban_sfz::instrument::Instrument::cc_defaults`（`<control>` 作用域、
//! 同名后者覆盖前者、取值越界是响亮 `Err`、取值非整数是**丢弃 ＋ 告警**）。
//!
//! ## 播种**在哪里**发生（自裁 ＋ 依据 ＋ 代价）
//!
//! **位置 ＝ 引擎的快照边界**：`rt.rs::render_block` 的重新武装分支（`2a-cc` 段）逐轨读
//! [`SfzPlan::prepared`] 的 [`CcSeedTable`]（摊平期从**同一次解析**抄下）并调
//! [`crate::cc_state::ControlChangeState::seed`]。⭐ 三条依据：
//!
//! 1. **"加载时"就是快照边界**：摊平表是控制侧"装载＋解码＋摊平"的产物
//!    （[`SfzPlan::prepare`]），它随快照按值到达音频线程 —— 表到达的那一刻正是
//!    "乐器被装载"那一刻；
//! 2. ⛔ **不新增实时状态**：播种规则是"**只填从未收到过的槽**"
//!    （[`crate::cc_state::ControlChangeState::seed`]）⇒ 对"每次修订重新武装"**幂等**
//!    ⇒ 不需要在 `EngineRuntime` 里记"这条轨播过没有" ⇒ `EngineRuntime` **＋0 B**；
//!    ⚠ 若改成"每次武装都写"，演奏中的真 CC 会被**每次编辑**（每次新快照）覆盖；
//! 3. ⛔ **不走事件面**：把初值做成一批 `EngineEvent::ControlChange` 会让播种受
//!    SPSC 环容量影响（环满 ⇒ 悄悄少播一条），而摊平表是**按值随快照到达**的
//!    ⇒ **无丢件可能**；⚠ 这也让"文件里真的写了 `set_ccN`"与"表非空"是同一个事实。
//!
//! **与真实 MIDI CC 的区别（逐条）**：
//!
//! * **时机**：真 CC 走**事件边界**（`EngineEvent::ControlChange` → `apply`），
//!   播种走**快照边界**（`seed`）；
//! * **先后**：事件面在同一个量子**先**跑 ⇒ **真 CC 永远赢**（`seed` 对已收到的槽返回
//!   `false`）；播种⛔ 不覆盖任何一个真值；
//! * **表状态**：播种**故意**把该控制器标成"收到过"（否则 [`crate::cc_state`] 的探针
//!   读 `None` ⇒ 播了等于没播）—— ⚠ 这是"armed 会因播种变真"的**唯一**来路，而它
//!   **只在表非空**（文件真写了 `set_ccN`）时发生；
//! * **不可撤销**：播种与真 CC 一样**永不**把控制器退回"未收到"（会话运行态是单调的）。
//!
//! ## ⭐ 保命不变量（机械开关，判据 S87）
//!
//! **表为空 ⇒ 播种机制一个字节都不动**：`render_block` 的 `2a-cc` 段对每张摊平表先问
//! [`CcSeedTable::is_empty`]，为空就 `continue` ⇒ 一次 `seed` 都不调
//! ⇒ `ControlChangeState::is_armed()` 保持 `false` ⇒ `render_branch` 继续传 `cc = None`
//! ⇒ **没有 `set_ccN` 的文件与接线前逐位相同**（端到端指纹 ＋ 三个承重指纹）。
//!
//! ## 域（⛔ 不静默、⛔ 不别名）
//!
//! 会话 CC 表的域是 `0..=127`（[`crate::cc_state::CONTROL_CHANGE_COUNT`]）；解析层接受
//! `N` 到 `u16::MAX`（语料里真有 `set_cc400=63` / `set_cc401=63`）。`400 & 0x7F = 16`
//! ⇒ 直接喂表会**冒充 CC16**，因此 [`CcSeedTable`] ⛔ **不做**那次掩码：
//! 域外声明**只计数**（[`CcSeedTable::out_of_domain`]）、不进表（判据 S92）。
//!
//! ## 语料普查（本机实测，2026-10-16，第 271／274 条）
//!
//! ⚠ **尺子先写清楚**：词边界口径 `(?<![A-Za-z0-9_])set_cc[0-9]+=`，
//! 分母 ＝ `assets/samples/**/*.sfz` 的 **1398** 份（`find … -name '*.sfz' | wc -l`）。
//! ⚠ ⛔ **不用纯子串尺**：`offset_cc131=5500` 的尾巴长得像 `set_cc131=`，
//! 会把它算成第 1138 处（实测：纯子串 **1138**／词边界 **1137**）。
//!
//! | 读数 | 值 |
//! | :--- | ---: |
//! | `set_ccN=` 出现次数 / 文件数 | **1137** / **105** |
//! | 其中落在 `<control>` 段的（解析层口径） | **1137**（全部；见 `yeban-sfz/src/control.rs`） |
//! | 取值字面不是十进制整数的次数 / 文件数（解析层丢弃 ＋ 告警） | 51 / 19（`63.5` 42 处、`$ht_lo_hi_init` 9 处） |
//! | 取值是整数但 `> 127`（**响亮 `Err`**）的次数 | **0** |
//! | **`N > 127`** 的声明次数 / 文件数（本表**计数、不进表**） | **38** / 19（全是 `set_cc400=63`／`set_cc401=63`） |
//! | 不同 `N` 的个数 / `N` 的范围 | 82 / `1..=401` |
//! | 单文件可进表条数（`N ≤ 127`、`(文件,N)` 末者胜）**最大** | **62** |
//! | 单文件条数直方图 | `{1: 18, 2: 1, 3: 6, 4: 7, 6: 9, 7: 9, 8: 1, 9: 9, 10: 11, 11: 8, 12: 3, 13: 7, 14: 1, 16: 3, 17: 2, 19: 2, 22: 2, 30: 1, 31: 2, 57: 1, 62: 2}` |
//! | 有 ≥1 条可进表初值的文件数 | 105 |
//! | 已知非零对照（同一次扫描） | `label_cc` 1767 处 / 102 文件；`ampeg_release` 2831 处 / 373 文件；`ampeg_attack` 2828 处 / 368 文件；`note_polyphony` 87 处 / 32 文件 |
//!
//! ⭐ **上界是结构性的**：容量 ＝ 会话表的 **128** 个槽（`BTreeMap` 去重 ⇒ 每个 CC 至多一条）
//! ⇒ **零截断可能**（⛔ 不是"语料最大 **62** ＋ 余量"；62 只说明今天用不满）。
//!
//! ## 尺寸（本单机械读数）
//!
//! `PreparedSfz` **88 → 240 B**（＋152 ＝ `size_of::<CcSeedTable>()`：值表 `[u8; 128]` ＋
//! 掩码 `[u64; 2]` ＋ 域外计数 `usize`）；⭐ `SfzPlan` **88 → 240 B**（⭐ 任务单 `#112`
//! **追加修订段**，被修订的是本行原先的「`SfzPlan` 仍 **88 B**（它只是包装）」那一支：
//! `SfzPlan` 的定义是 `struct SfzPlan { prepared: PreparedSfz }` —— 它**按值内联**那份摊平表
//! ⇒ 内层 `88 + 152 = 240` 时包装**必然同长**；「它只是包装」这条**理由**成立，被修订的只是
//! 由它推出的那个**数**（88）：「只是包装」⛔ **不蕴含**「尺寸不变」—— 增量的**来源在内层**）、
//! `PreparedRegion` 仍 **6152 B**、`SfzVoice` 仍 **40 B**、`SfzVoicePool` 仍 **1712 B**、
//! `EngineRuntime` 仍 **180144 B**（门槛 192 KiB ＝ `196608`，余量 `16464`）——
//! 播种表进的是**摊平表**（控制侧构造、随快照按值走），⛔ 不进任何实时对象。
//!
//! ⭐ ⭐ **追加修订段（任务单 `#113`）—— 被修订的是上一段里 `PreparedRegion` 仍 **6152 B**
//! 那一支**（⛔ 原文一字未删，口径同台账第 278 条）：`PreparedRegion` 现为 **6160 B**
//! （`#113` 加了**一个**每区域字段 `direction: PlayDirection`）；⛔ `SfzVoice` 仍 **40 B**、
//! ⛔ `SfzVoicePool` 仍 **1712 B**、⛔ `EngineRuntime` 仍 **180144 B**（余量 `16464`）。
//! ⭐ `SfzPlan` **88 → 240 B** 与 `PreparedSfz` **88 → 240 B** 两支**整条有效**（与本单无关）。
//!
//! ## ⚠ 已知边界（如实登记，⛔ 不静默）
//!
//! 1. **会话表是"一张"**（[`crate::cc_state`]）：多条轨各带 `set_ccN` 时，
//!    播种按 `EngineSnapshot::sfz()` 的 `BTreeMap` 顺序（[MODEL-AST-003]）**先到先得**
//!    （后到者要填的槽已经"收到过"）⇒ 冲突时**键序最小的那条轨赢**。这是 `#107`
//!    既有的"CC 探针没有轨道维度"（[`crate::ring::EngineEvent::ControlChange`] 的载荷形状）
//!    的**同一个**限制，⛔ 本单没有发明轨道路由；
//! 2. **离线导出那条路仍传 `cc = None`**：`SynthEngine::render_track`（旧入口）**不变**
//!    ⇒ `yeban-app` 的导出/录音路径**还没有**吃到 `set_ccN`；
//!    摊平表已经把表带出来了（[`PreparedSfz::cc_seeds`]），接线点是控制侧的
//!    一行探针（登记在交付报告的"给集成者的待办"）。
//!
//! ⭐ ⭐ **追加修订段（任务单 `#112`，`#110` 的收尾单）—— 被修订的是上面第 1 条的
//! "只登记了『**哪条轨的初值赢**』那一半"这一支**（⛔ 上面第 1／2 条的**原文一字未删**，
//! 口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋ 点名被修订的支）：
//!
//! ⭐ **补上的另一半 ＝ 跨轨外溢，且它是"有意边界"**（机械可判，判据 S93）：会话表是
//! **一张** ⇒ 只要有**任何一条**轨的 `set_ccN` 把表武装起来，**别的**、自己**没有**
//! `set_ccN` 的轨在逐轨渲染时也会拿到 `Some(probe)`（`rt.rs` 的 `render_branch` 每块
//! 只算**一次** `cc`，见那里的 `let cc: Option<&dyn Fn(u8) -> u8>`，再交给**所有**轨）
//! ⇒ 它的 `loccN` / `hiccN` 门控会读**那条轨**的初值。
//!
//! 实测（判据 `S93`；16 量子 ＝ 2048 帧；A 轨区域 `key=127` ⇒ 本夹具的音符 60 **恒不匹配**
//! ⇒ A 轨自身逐位静音（对照腿实测非零样本 `0`）⇒ 下表"非零样本"**全部**来自 B 轨）：
//!
//! | A 轨的 `<control>` | 会话表 `armed` | `control_change(1)` | B 轨（`locc1=64 hicc1=127`）非零样本 | B 轨端到端指纹 |
//! | :--- | :--- | ---: | ---: | :--- |
//! | 无 | `false` | `None` | **0** | `0x9c1bda7f8c872325` |
//! | `set_cc1=64` | `true` | `Some(64)` | **4030** | `0xec4ca6ac0fe2b1e9` |
//! | `set_cc2=64`（**判别腿**） | `true` | `None` | **0** | `0x9c1bda7f8c872325` |
//!
//! ⚠ 第三行是**判别腿**：仅仅"表被武装"**不足以**让 B 轨门控成立 —— 它真的读到了 A 轨
//! **CC1 的值 64**（对照行武装了表、但 CC1 仍 `None` ⇒ B 轨静音）。
//!
//! ⭐ **为什么登记而不是修**（代价逐条）：① 会话级单一 CC 表是 `#107` 的既有形状
//! （`EngineEvent::ControlChange` 的**载荷里没有轨道维度**）⇒ 一条**真** MIDI CC 本来
//! 就**全会话**可见；只把**播种**做成按轨，会让模型变成"一半按轨、一半按会话"，
//! 比现状更难推理；② 要**真的**按轨，就得给 CC 事件加轨道维度（新载荷/新变体）＋
//! 让探针按轨取值（新逐轨状态或新快照面）⇒ 那是**架构面**（本单 §4 的止损条件 ①②③），
//! ⛔ 不是收尾；③ 只把探针换成"读本轨那份摊平表的 `cc_seeds`"是**半修**：真 MIDI CC
//! 仍全会话可见，且会让离线路径与实时路径用**两套**播种机制。⇒ 本单**保留**这个边界，
//! 并把它钉成**可判事实**（`S93`：去掉 `#110` 的快照边界播种段 ⇒ 该判据变红）。
//!
//! ⭐ ⭐ **追加修订段（任务单 `#112`）—— 被修订的是上面第 2 条的"接线点在控制侧"这一支的
//! 确认状态**（⛔ 原文一字未删）：本单**决定维持**这条登记（⛔ 不是接线），并把它从"散文"
//! 钉成**可判事实**（判据 `S93` 的离线腿）：`SynthEngine::render_track` 对带 `set_ccN`
//! 的摊平表**仍**传 `cc = None` ⇒ 逐位等于 `render_track_with_cc(…, None)`、
//! **不等**于把探针喂成"本表 `cc_seeds`"的那一臂。⭐ **为什么维持**：离线/导出路的探针
//! **来源**（只吃 `set_ccN`？还是也吃模型里的 CC 自动化、录音进来的 CC）是**控制侧**的
//! 一次裁决，⛔ 不是引擎可以替它选的；而且只有在引擎里**再实现一套**"按表播种"的探针
//! 才能接上（实时侧那套长在 `rt.rs` 的**会话表**上）⇒ 两套机制比"一套 ＋ 登记来源缺口"更差。
//! ⚠ 这是**已知行为差异**：同一份带 `set_ccN` 的文件，实时路径吃初值、离线导出⛔ 还不吃
//! （给集成者的待办）。
//!
//! # ⭐ CC 区域门控（`loccN` / `hiccN`，任务单 `#109`）

//!
//! ## 规范逐字（<https://sfzformat.com/opcodes/loccN/>，`hiccN` 是同一页的另一行）
//!
//! > Defines the range of the last MIDI controller N required for the region to play.
//! >
//! > `locc74=30 hicc74=100` … The region will play only if last MIDI controller 74
//! > received was in the 30 to 100 range.
//! >
//! > Allowed range is 0 to 127. The defaults are `loccN=0` and `hiccN=127`.
//! >
//! > This is a "hard" switch - if a region is within the locc to hicc range it plays,
//! > if it's outside that range it does not play. For smooth fades controlled by CC
//! > (such as crossfaded dynamic layers or crossfaded vibrato layers on sustained
//! > instruments), other opcodes such as `xfin_loccN` / `xfin_hiccN` and
//! > `xfout_loccN` / `xfout_hiccN` should be used, or perhaps the `amplitude_onccN`
//! > ARIA extension.
//!
//! 表格行（同页）：`loccN` … Type `integer`、Default `0`、Range `0 to 127`；
//! `hiccN` … Type `integer`、Default `127`、Range `0 to 127`。Category
//! ＝ "Region Logic, MIDI Conditions"。
//!
//! ## 判定时机 ＝ **触发期**（裁定 ＋ 依据，⛔ 不是猜的）
//!
//! 1. 规范把它归在 **Region Logic / MIDI Conditions**，措辞是"**required for the
//!    region to play**"（"这条区域**要不要**发声"）—— 那是**区域选择**，与 keyswitch
//!    ／`trigger` 同类，⛔ 不是逐块（per-block）的调制。
//! 2. 同页把它与 `xfin_*`／`xfout_*`（交叉淡化）**显式对立**："This is a 'hard' switch …
//!    For smooth fades … `xfin_loccN` / `xfin_hiccN` … should be used"。⭐ 而渐变的
//!    那两族**恰恰是逐块**的量（本仓 `#108` 已按逐块接线，见上一节）⇒ 门控两族是
//!    **另一类**：一旦判定，**整条音符**（整个声部生命周期）要么有这条区域、要么没有。
//! 3. 因此本仓裁定：**门控在触发期求值一次**，结果就是"建不建这个声部"
//!    ⇒ ⛔ **没有**"一块开、下一块关"的语义（那会把 `hicc1=63`／`locc1=64` 的
//!    硬切换成**块级**断续，与"hard switch"直接矛盾）。
//!
//! ## 三条路的求值点（⛔ 不是三份公式）
//!
//! | 路 | 求值点 | 几何来源 |
//! | :--- | :--- | :--- |
//! | 离线 [`SfzInstrument::render_note_into_with_cc`] | `Region::matches_query(&query)` 里的 `region_matches` → `Region::cc_gates_ok` | `Region::cc_gates`（解析层，**本来就在跑**） |
//! | 摊平 [`PreparedSfz::render_block_into_with_cc`] | [`PreparedRegion::cc_gates_ok`]（掩码之上） | 烘好的 `cc_gates` |
//! | 实时 [`SfzVoicePool::trigger_note`] | 同上（**触发期一次**） | 同摊平 |
//!
//! ## ⭐ 为什么没有新的逐声部字段（`SfzVoice` ＋0）
//!
//! 门控的**几何**（`[lo, hi]` ＋ CC 号）是**区域参数**（静态），它烘进
//! [`PreparedRegion::cc_gates`]；门控的**裁决结果**是"这个声部**存不存在**"——
//! 它**不是**声部状态，因此 ⛔ 不需要在 [`SfzVoice`] 里存任何一位。声部尺寸
//! 仍 `40 B`（判据 `S9`）。
//!
//! ## ⭐ 为什么不破坏既有掩码行为
//!
//! 掩码（`key_mask` / `vel_mask`）承载的是**静态**门控；本单⛔ **不改**它们的口径，
//! 只把"CC 门控用 `cc = None` 求值一次并清 0"这一条**从掩码里摘出来**：
//! `flatten_regions` 的静态门控改用 [`non_cc_gates_open`]（与 `region_matches` **同序、
//! 同缺省值**，只把 CC 那一段换成 `true` ⇒ 门控**不再**清 0）。
//! ⚠ ⛔ 不是「喂一个假探针」：门控是**逐区域**的区间，任何单一常量探针都只能满足
//! 一部分区域（本单实测：探针 `0` 满足不了 `locc1=64`）。⇒ 通道 / `trigger` / keyswitch /
//! `force` / `first`／`legato` / 键轴 / 力度轴 / `seq_*` 的判定**一个字节都没碰**（判据
//! `S65` 的三个承重指纹 ＋ `S9`／`S66`／`S71` 复跑）。
//!
//! ⚠ **本单的明文后果（如实登记，⛔ 不静默）**：一条**只有** CC 门控的区域，
//! 在会话**一条 CC 都没收到过**（探针 `None`）时**仍不发声** —— 这是 `yeban-sfz`
//! 从严策略的**同一份**口径（`region_matches` 的 `else { return false }`），
//! 三条路**一致**；⭐ 本单⛔ **不**替调用方发明规范缺省 `loccN=0`/`hiccN=127`
//! 的播种（那是 `set_ccN` 那一族的事，见本节末）。
//!
//! ## 尺寸（本单机械读数）
//!
//! `CC_GATE_CAP` ＝ **128**（结构性上界，由来见 [`CC_GATE_CAP`] 的文档）⇒
//! `PreparedRegion` `5768 → ` **6152 B**（＋384，摊平期，⛔ 不进实时对象）；
//! ⛔ `SfzVoice` 仍 `40 B`（＋0）、⛔ `SfzVoicePool` 仍 `1712 B`（＋0）、
//! ⛔ `EngineRuntime` 仍 `180144 B`（＋0，门槛 192 KiB、余量 16464）。
//!
//! ## ⛔ 本单仍未接线（登记，⛔ 不静默）
//!
//! * `pan_oncc*` ／ `volume_oncc*` 这一族"**任意 opcode 的 `_onccN` 调制**"（语料
//!   `pan_oncc*` 9796 处）—— 另开单，⛔ 与本单的区域门控不是同一件事；
//! * `<control> set_ccN=` 的**播种**（初值；语料 1137 处）仍归调用方；
//! * `set_ccN` 的初值与"会话表未 armed ⇒ 探针 `None`"是**两件事**：本单⛔ 不播种。
//!
//! # ⭐ CC 轴交叉淡化（任务单 `#108`）
//!
//! 规范逐字（<https://sfzformat.com/opcodes/xfin_loccN/>）：
//!
//! > `xfin_loccN` and `xfin_hiccN` set the range of values in the MIDI continuous
//! > controller N which will perform a fade-in in the region.
//! >
//! > The volume of the region will be zero for values of the MIDI continuous controller N
//! > lower than or equal to `xfin_loccN`, and maximum (as defined by the volume opcode)
//! > for values greater than or equal to `xfin_hiccN`.
//!
//! 与淡出方向（<https://sfzformat.com/opcodes/xfout_loccN/>）：
//!
//! > The volume of the region will be maximum (as defined by the volume opcode) for values
//! > of the MIDI continuous controller N lower than or equal to `xfout_loccN`, and zero
//! > for values greater than or equal to `xfout_hiccN`.
//!
//! 曲线（`xf_cccurve`，缺省 `power`；<https://sfzformat.com/opcodes/xf_cccurve/>：
//! "**gain**: Linear gain crossfade. … **power**: Equal-power RMS crossfade. …"）与
//! 端点缺省、`power ⇒ sqrt` 的取舍**全在** `yeban-sfz` 的 `crossfade` 模块文档里
//! —— ⛔ 本片不复制那份事实源，只**烘几何 ＋ 喂探针**。
//!
//! **求值点（三条路各一个，⛔ 不是三份公式）**：
//!
//! | 路 | 求值点 | 几何来源 |
//! | :--- | :--- | :--- |
//! | 离线 [`SfzInstrument::render_note_into_with_cc`] | `Region::playback_spec_with(&query, rates).total_gain()` | `Region::crossfades`（解析层） |
//! | 摊平 [`PreparedSfz::render_block_into_with_cc`] | [`PreparedRegion::gain_at_with_cc`] | 烘好的 `cc_crossfades` |
//! | 实时 [`SfzVoicePool::slot_readings`] | 同上（每块现算）× [`SfzVoice::track_gain`] | 同摊平 |
//!
//! ⭐ **为什么没有新的逐声部字段**：CC 增益**逐块**可变（CC 随时变）⇒ 触发期烘死的
//! 乘积不够。本片取"**每块按 `region + note + velocity + probe` 重算**"这一支：
//! `note` / `velocity` / `region_index` **早就在** [`SfzVoice`] 里 ⇒ 声部
//! **＋0 字节**（`40 B`）。代价是 `SfzVoice::gain` 改名 [`SfzVoice::track_gain`] ——
//! 它现在只存**引擎层**那一份（可闻门 × 推子），乐器层由逐块路径现算
//! （两条腿仍相乘、⛔ 不重复；归属裁决 `R711` 不变）。
//!
//! # ⭐ `ampeg_*` 的力度调制（任务单 `#106`）
//!
//! 规范语义（五个 `ampeg_vel2*` opcode 的表行）、公式
//! `base + Σ delta_cc × curve(cc) + delta_vel2 × velocity / 127`、取值域（`-100..=100`，
//! 时间 = 秒 ／ `sustain` = %）与全部工程裁决写在 **`yeban-sfz`** 的 `modulation`
//! 模块文档 §4 —— 本片 ⛔ 不复制那份事实源。
//!
//! ⭐ **三条路全部接线**（⛔ 不是只接离线）：
//!
//! 1. **离线** [`SfzInstrument::render_note_into_with_cc`]：输入 ＝ 它的 `velocity` 参数
//!    ＋ CC 探针；求值本体 ＝ `yeban_sfz::modulation::ampeg_value_with_baked_and_vel2`；
//! 2. **摊平表** [`PreparedSfz::render_block_into_with_cc`] → [`PreparedRegion::ampeg_run_with_cc_and_velocity`]：
//!    输入 ＝ 它的 `velocity` 参数（原来就有，⛔ 没有新参数面）；
//! 3. **实时** [`SfzVoicePool::slot_readings`] → 同一个 `ampeg_run_with_cc_and_velocity`：
//!    输入 ＝ **逐声部力度** [`SfzVoice::velocity`]（⭐ 触发期就已抄下，
//!    `SfzVoice` 因此 **＋0 字节**、仍 `40 B`）。
//!    ⇒ ⛔ **没有引入任何新的逐声部字段**：本片要的力度**早就在**声部里。
//!
//! ⭐ 力度支 vs CC 支的分工（登记）：
//!
//! * **力度支**（本片）：**三条路都生效**（力度是声部自带的，实时侧读得到）；
//! * **CC 支**（`#104`）：**只有离线两条路**生效（实时回调不读 CC 状态，见上面的登记）。
//!
//! ⭐ 摊平表承载：`[Option<f32>; AmpegParam::COUNT]`（每参数**至多一条**，规范只列
//! 这五个名字）搬进 [`PreparedRegion`]：`4192 → ` **4232 B**（＋**40 B**／区域，摊平期）、
//! ⛔ 不进实时对象（`SfzVoice` 40 ／ `SfzVoicePool` 1712 ／ `EngineRuntime` 180000 全部 ＋0 —— ⭐ ⚠ **`#107` 后 `EngineRuntime` 为 `180144 B`**；
//! `SynthEngine` 109152 ＋0、`EngineSnapshot` 4496 ＋0）。
//! `None` ＝ 该 opcode **根本没出现** ⇒ 求值期**一项算术都不做**（⇒ 与接线前逐位相同）。
//!
//! ⚠ **仍未做**（⛔ 不发明）：扩展 CC 的**取值语义**（135 = unipolar random `0..1`、
//! 136 = bipolar random `-1..1`、140/141 = 半音数…… `u8` 探针表达不了负值/小数）、
//! `fileg_*` / `pitcheg_*`、`smoothccN` / `stepccN`（`ampeg_*` 的规范表里
//! **没有**这两行，登记语料出现 0 次 ⇒ 本片不识别，按未知 opcode 忽略）。

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

// ⭐ 窃取淡出的**帧数口径**：引擎里 PolySynth 用的就是这一个（3 ms × 采样率，
// 上限 0.5 s、下限 1 帧）⇒ SFZ 池与复音池共用**同一个**舵，⛔ 不是第二份常数。
use yeban_dsp::polysynth::steal_fade_frames_for;
use yeban_sfz::crossfade::{Crossfade, XfAxis, XfCurve, XfDirection, XfRange};
use yeban_sfz::error::SfzError;
use yeban_sfz::instrument::{
    CcGate, Instrument, LoopMode, NotePolyphony, OffMode, PlayDirection, Region, RegionQuery,
};
// ⭐ 任务单 `#104`：CC 调制（`ccN` / `_ccN` / `_onccN` / `_curveccN`）；
// ⭐ 任务单 `#115`：目标从五个 `ampeg_*` 推广成 [`ModTarget`]（首个增益族目标
// `amplitude`）。摊平表按 [`AMPEG_MOD_CAP`] 条**定长**承载（`Copy`），求值本体是
// [`cc_value_with_baked`]（与 `yeban-sfz` 的离线路径**同一份**实现）。
// ⭐⭐ 任务单 `#116`：第二个被推广的目标 `tune`（音分 → 速率乘子），换算本体是
// [`tune_ratio_with_baked`]（同样是 `yeban-sfz` 的**同一份**实现，⛔ 不是引擎里第二套）。
// ⭐⭐ 任务单 `#119`：第三个被推广的目标 `volume`（**dB** → 线性增益乘子），换算本体是
// [`volume_ratio_with_baked`]（同样是 `yeban-sfz` 的**同一份**实现）。
use yeban_sfz::modulation::{
    AMPEG_MOD_CAP, AmpegParam, BakedCcMod, ModTarget, bake_cc_mods, cc_value_with_baked,
    cc_value_with_baked_and_vel2, tune_ratio_with_baked, volume_ratio_with_baked,
};
// ⚠ 这两个是 **lib 构建**就需要的（`load` / `with_instrument` 都调用它们）⇒ ⛔ 不能放在 `#[cfg(test)]` 下，
// 否则非测试构建会报 "cannot find type `ParseLimits`"（本批实测过一次：三项门全红）。
use yeban_sfz::parser::{IncludeIssue, ParseLimits, SfzSource, parse_sources, parse_text};
use yeban_sfz::playback::{LoopWindow, RenderRates, SampleSpan};
// ⭐ 衰减底（−60 dBFS）取自 `yeban-sfz` 的 [`StealFade`](yeban_sfz::voice_pool::StealFade)：
// 引擎侧的 SFZ 窃取淡出与该库的 3 ms 指数淡出**同形状**（`FLOOR^(n/N)`），
// ⛔ 不是引擎自造的一条包络（victim **选择**口径则由引擎自己拥有，见 `SfzVoicePool::select_victim`）。
use yeban_sfz::voice_pool::STEAL_FADE_FLOOR;

/// ⭐ **`amplitude` 的百分比刻度**（任务单 `#115`）：`amplitude` 的单位是 **%**，
/// 「100% ＝ 不做任何音量变化」（<https://sfzformat.com/opcodes/amplitude/> 正文逐字）
/// ⇒ 线性因子 ＝ `amplitude_value / 100`。
///
/// ⚠ 它与 [`yeban_sfz::instrument::AMPLITUDE_DEFAULT_PERCENT`] 的关系：那个是**缺省值**
/// （也是 100），这个是**除数**。两者分开命名，⛔ 不用"缺省值"当刻度。
pub const AMPLITUDE_PERCENT_SCALE: f32 = 100.0;

/// 已加载的 SFZ 乐器：**文本 + 元数据**（⛔ 不含借用式解析结果）。
///
/// ## ⭐ `#include`（任务单 `#206`）
///
/// [`Self::load`] 只接收文本 ⇒ 走 `parse_text`（**纯文本模式**，`#include` 被忽略，
/// 与既有行为逐字一致）。需要展开 `#include` 的调用方走
/// [`Self::load_with_includes`]：它把**根文件文本**＋基准目录交给
/// `yeban_sfz::IncludeResolver`，把展开后的片段序列保存在 [`Self::sources`] 里，
/// 之后每次 [`Self::with_instrument`] 都用 `parse_sources` 重放同一份片段
/// （行号、跨片段作用域、`default_path` 全部与一次性解析逐位一致）。
///
/// ⚠ 为什么不保存"展开后的拼接文本"：`SfzSource` 各自带**原文件行号**，错误/告警
/// 才能指回真正的位置；拼接会把这些信息压成一份假的单文件。
#[derive(Debug, Clone)]
pub struct SfzInstrument {
    /// 根 SFZ 文件原文（自己持有 ⇒ 无生命周期参数）。
    text: String,
    /// `Some` ⇒ 已展开 `#include` 的片段（`parse_sources` 的输入）；
    /// `None` ⇒ 纯文本（`parse_text` 的输入，既有 `load` 路径）。
    sources: Option<Arc<[SfzSource]>>,
    /// 容错展开时被跳过的 `#include`（每条都带行号与路径）。
    include_issues: Vec<IncludeIssue>,
    /// 构造期解析得到的**区域数**（元数据，供控制侧快速查询）。
    region_count: usize,
}

impl SfzInstrument {
    /// 构造期解析一次：**校验**文本并记下元数据。
    ///
    /// ⚠ 本函数会分配（`String` 已由调用方给出；解析器内部有分配）⇒ **只允许在控制侧调用**。
    ///
    /// ⚠ 纯文本模式**不展开** `#include`（`parse_text` 的既有契约：不做 I/O）。
    /// 需要 include 的调用方用 [`Self::load_with_includes`]。
    ///
    /// # Errors
    /// 文本非法（语法、超限等）时原样透出 [`SfzError`]，⛔ 不 panic。
    pub fn load(text: String, limits: &ParseLimits) -> Result<Self, SfzError> {
        let region_count = parse_text(&text, limits)?.regions().len();
        Ok(Self {
            text,
            sources: None,
            include_issues: Vec::new(),
            region_count,
        })
    }

    /// ⭐ 构造期**展开 `#include`** 再解析（任务单 `#206`）。
    ///
    /// * `text`：根文件原文（调用方已读盘；⛔ 本函数不读根文件第二遍）；
    /// * `base_dir`：**沙箱基准目录**＝根 SFZ 所在目录（规范与 `IncludeResolver` 的
    ///   既有口径：include 路径**相对基准目录**，⛔ 不相对被包含文件）；
    /// * `entry`：根文件相对 `base_dir` 的文件名（用于沙箱检查与环检测）。
    ///
    /// ## 展开语义（逐条）
    ///
    /// | 情形 | 行为 |
    /// | :--- | :--- |
    /// | 嵌套 include（A→B→C） | 递归展开，按**粘贴顺序**进入片段序列 |
    /// | include 夹在一行中间 | 在**出现位置**切片段，其余文本照常解析 |
    /// | 重复 include 同一文件 | **每次都展开**（粘贴语义；⛔ 不去重） |
    /// | 循环（A→B→A） | `IncludeCycle` 错误（`stack` 已访问集），⛔ 不递归 |
    /// | 超深 / 超文件数 | `IncludeDepthExceeded` / `IncludeCountExceeded` |
    /// | 目标不存在 / glob 无匹配 | **记一条** [`Self::include_issues`] 并继续 |
    ///
    /// `default_path`：`<control>` 在根/被包含片段里都写进**同一个**解析器状态 ⇒
    /// 跨片段继承与单文件解析**同一条**规则（`Region::sample_path()` 仍先拼前缀）。
    ///
    /// # Errors
    ///
    /// 沙箱逃逸 / 非 UTF-8 / 循环 / 超上限 / 语法错误 ⇒ [`SfzError`]。
    /// **缺失的 include 不是错误**（见上表）。
    pub fn load_with_includes(
        text: String,
        base_dir: &Path,
        entry: &str,
        limits: &ParseLimits,
    ) -> Result<Self, SfzError> {
        let resolver = yeban_sfz::IncludeResolver::new(base_dir, *limits)?;
        let resolution = resolver.resolve_text_lossy(entry, &text)?;
        let region_count = parse_sources(&resolution.sources, limits)?.regions().len();
        Ok(Self {
            text,
            sources: Some(Arc::from(resolution.sources)),
            include_issues: resolution.issues,
            region_count,
        })
    }

    /// 容错展开时被跳过的 `#include`（空 ⇒ 全部目标都读到了）。
    #[must_use]
    pub fn include_issues(&self) -> &[IncludeIssue] {
        &self.include_issues
    }

    /// 区域数（规范里的 `region` 条目个数）。
    #[must_use]
    pub fn region_count(&self) -> usize {
        self.region_count
    }

    /// 原始文本（根文件；调用方可能需要原样回写/哈希）。
    ///
    /// ⚠ 它**不含**被 include 的文件内容 —— 展开后的片段见内部 `sources`
    /// （`#include` 的展开结果对调用方不可见，只通过区域/样本读数体现）。
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
        // ⭐ `#206`：include 展开过的乐器按**同一份片段序列**重放（⛔ 不是拿根文本
        //    再 `parse_text` —— 那会把 include 悄悄丢掉）。
        let instrument = match self.sources.as_deref() {
            Some(sources) => parse_sources(sources, &limits)?,
            None => parse_text(&self.text, &limits)?,
        };
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
        self.render_note_into_with_cc(source, note, velocity, None, out, engine_hz)
    }

    /// ⭐ **离线渲染 ＋ CC 探针**（任务单 `#104`）：与 [`Self::render_note_into`]
    /// **同一条路**，只多一个 CC 状态来源。
    ///
    /// * `cc == None` ⇒ 与 [`Self::render_note_into`] **逐位相同**
    ///   （既有的 `loccN`/`hiccN` 门控走"未提供状态 ⇒ 不匹配"的严格策略，
    ///   `ampeg_*` 走基准值）；
    /// * `cc == Some(probe)` ⇒ ① 探针同时喂给**既有**的
    ///   [`RegionQuery::with_cc`]（CC 门控生效）；② `ampeg_*` 按
    ///   `base + Σ delta × curve(cc)` 求值（公式与逐字出处见
    ///   [`yeban_sfz::modulation`]）；③ ⭐ 任务单 `#108`：增益改走
    ///   `Region::playback_spec_with(&query, ..)` ⇒ **CC 轴交叉淡化**
    ///   （`xfin_loccN` / `xfin_hiccN` / `xfout_loccN` / `xfout_hiccN`）也按探针生效
    ///   （`cc == None` 时与 `playback_spec(..)` **逐位相同**，见下面的注释）。
    ///
    /// ⚠ **`probe` 的值域是 `u8`**：标准 CC（`0..=127`）与 SFZ2 扩展 CC
    /// （`128..=137`）都能表达；**扩展 CC 的取值语义**（135 = unipolar random `0..1`、
    /// 136 = bipolar random `-1..1`、140/141 = 半音数）**本片不声称支持**（登记为缺口）。
    ///
    /// ⚠ **只覆盖离线**：本函数在调用时**解析**（⇒ 分配）⇒ ⛔ 不得在音频回调里调用（红线 7）。
    /// 摊平侧的同一能力见 [`PreparedSfz::render_block_into_with_cc`]。
    ///
    /// # Errors
    /// 同 [`Self::render_note_into`]。
    pub fn render_note_into_with_cc(
        &self,
        source: &InMemorySample,
        note: u8,
        velocity: u8,
        cc: Option<&dyn Fn(u8) -> u8>,
        out: &mut [f32],
        engine_hz: f32,
    ) -> Result<usize, SfzError> {
        let pcm_len = source.pcm.len();
        let rates = RenderRates::new(source.sample_hz, engine_hz);
        let query = RegionQuery::new(note, velocity);
        // ⭐ 探针同时注入**既有**的 CC 门控（`loccN`/`hiccN`）；`None` ⇒ 严格策略不变。
        let query = match cc {
            Some(probe) => query.with_cc(probe),
            None => query,
        };
        self.with_instrument(|instrument| {
            let mut written = 0usize;
            for region in instrument.regions() {
                if !region.matches_query(&query) {
                    continue;
                }
                // ⭐ 复用 `yeban-sfz` 自己的换算：速率与总增益都来自它的 `PlaybackSpec`。
                // ⭐⭐ `tune` 调制（任务单 `#116`）：`playback_rate` 已含**静态** `tune`，
                // 再乘上 `yeban-sfz` 的 [`tune_ratio_with_baked`]（**同一份**换算，
                // 与摊平表 `render_block_into_with_cc`、实时 `slot_readings` 共用）。
                // ⚠ `cc == None` ⇒ 乘子恰 `1.0` ⇒ 与接线前**逐位相同**；
                // ⚠ 非法速率沿用**下面那条既有守卫**（⛔ 不加第二条谓词）。
                // ⭐ 任务单 `#108`：这里从 `playback_spec(..)`（`cc = None` 的那条重载）
                // 换成 `playback_spec_with(&query, ..)` —— 后者读 `query.cc`，于是
                // **CC 轴交叉淡化**也在离线这条路上生效（`query` 上面已经按探针构造过）。
                // ⚠ `cc = None` 时两者**逐位相同**：`playback_spec` 的实现就是
                // `playback_spec_with(&RegionQuery::new(note, velocity), rates)`
                // （见 `yeban-sfz/src/playback.rs`）⇒ 既有读数不变。
                //
                // ⭐ 振幅（任务单 `#115`）：**最外层线性乘子** `amplitude_gain` 与
                // 摊平／实时两条路**同一份**（`cc_value_with_baked_and_vel2`，
                // `target = Amplitude`）⇒ 三条路逐位一致（判据 `S103`）。
                // 缺省 `amplitude = 100` ⇒ 因子恰 `1.0` ⇒ 乘它**逐位不变**。
                // ⚠ `#116` 起 `baked` 同时服务 `tune` ⇒ 它必须**先**算出来。
                let baked = cc.map(|_| {
                    bake_cc_mods(region.cc_modulations(), |index| {
                        instrument.modulation_curve(index)
                    })
                });
                // ⭐⭐ `tune` 调制（任务单 `#116`）：`playback_rate` 已含**静态** `tune`，
                // 再乘上 `yeban-sfz` 的 [`tune_ratio_with_baked`]（**同一份**换算，
                // 与摊平表 `render_block_into_with_cc`、实时 `slot_readings` 共用）。
                // ⚠ `cc == None` ⇒ 乘子恰 `1.0` ⇒ `rate × 1.0` 与接线前**逐位相同**。
                let rate = region.playback_rate(note, rates)
                    * tune_ratio_with_baked(baked.as_deref().unwrap_or(&[]), cc);
                let amplitude_gain = cc_value_with_baked_and_vel2(
                    region.amplitude,
                    ModTarget::Amplitude,
                    baked.as_deref().unwrap_or(&[]),
                    None,
                    0,
                    cc,
                )
                .0 / AMPLITUDE_PERCENT_SCALE;
                // ⭐⭐ 音量调制（任务单 `#119`）：**最外层线性乘子** `volume_ratio`
                // 与摊平／实时两条路**同一份**（`yeban-sfz` 的
                // [`volume_ratio_with_baked`]）⇒ 三条路逐位一致（判据 `S114`）。
                // 缺省（无 `volume_*ccN`）⇒ 乘子恰 `1.0` ⇒ 乘它**逐位不变**。
                let volume_ratio = volume_ratio_with_baked(baked.as_deref().unwrap_or(&[]), cc);
                let gain = region.playback_spec_with(&query, rates).total_gain()
                    * amplitude_gain
                    * volume_ratio;
                // ⭐⭐ `#117`：速率守卫的唯一谓词（三条路同一量、同一谓词）。
                if !rate_is_usable(rate) || pcm_len == 0 {
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
                // ⭐ **播放方向**（任务单 `#113`）：摊平/实时两条路**同一份**归约
                // （[`reduce_travel`]）⇒ `direction=reverse` 时起点 ＝ 区间末格、
                // 步进取负、边界换成下界。⚠ `direction=forward` 时三项**逐位不变**。
                let (mut position, rate, source_end) =
                    reduce_travel(region.direction, rate, start, source_end);
                // 离线/摊平这条路**不窃取**：淡出状态恒为 [`FadeRamp::NONE`]
                // （`gain == 1.0` ⇒ 那个逐帧乘法逐位不变）。
                let mut fade = FadeRamp::NONE;
                // ⭐ 包络（任务 `#92`）：与摊平/实时两条路**同一份归约**（[`ampeg_run_from`]）。
                // 这条路**没有 note-off** ⇒ 只走 起音→保持→衰减→保持
                // （`frame = u64::MAX`：永远不会进释放段）；
                // ⚠ `abs_base = 0` 只用于释放进度，这条路上用不到。
                // ⚠ `frame = u64::MAX`（这条路没有 note-off ⇒ 永不进释放段）、
                // `abs_base = 0`（释放进度用不到）、`loop_during_release = false`。
                // ⭐ CC 调制（任务单 `#104`）：五个参数按探针求值，走
                // `yeban-sfz` 的**同一份**求值本体（`cc_value_with_baked`；
                // `#115` 起按 `ModTarget` 过滤，⛔ 公式未变）；
                // `cc == None` ⇒ 五个读数就是基准字段 ⇒ 与接线前逐位相同。
                // ⭐ 力度调制（任务单 `#106`）：**第二支独立输入**（音符力度）——
                // 五个参数各自 `base + Σ delta_cc × curve(cc) + delta_vel2 × v/127`
                // （公式、逐字出处与求和顺序见 `yeban_sfz::modulation`）。
                // `cc == None` **且**没有 `ampeg_vel2*` ⇒ 五个读数就是基准字段
                // ⇒ 与接线前**逐位相同**（求值本体一项算术都不做）。
                // ⚠ `baked` 在上面已经算好（`#115` 起它同时服务 `amplitude`）。
                let value = |param: AmpegParam| {
                    cc_value_with_baked_and_vel2(
                        region.ampeg_base(param),
                        ModTarget::Ampeg(param),
                        baked.as_deref().unwrap_or(&[]),
                        region.ampeg_vel2(param),
                        velocity,
                        cc,
                    )
                    .0
                };
                let (attack, hold, decay, sustain, release) = (
                    value(AmpegParam::Attack),
                    value(AmpegParam::Hold),
                    value(AmpegParam::Decay),
                    value(AmpegParam::Sustain),
                    value(AmpegParam::Release),
                );
                let mut env = ampeg_run_from(attack, hold, decay, sustain, release, engine_hz)
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

/// ⭐ **播放方向归约的唯一实现**（任务单 `#113`）：离线
/// [`SfzInstrument::render_note_into_with_cc`]、摊平 [`PreparedSfz::render_block_into_with_cc`]
/// 与实时声部（[`SfzVoicePool::trigger_note`]）三条路共用它 ⇒ 方向口径不可能在三条路之间分叉
/// （与 [`reduce_span`] / [`reduce_loop_window`] 同一形态）。
///
/// # 三件读数（⭐ 全部落在**已有**字段上 ⇒ ⛔ 零新逐声部字段）
///
/// 输入是 [`reduce_span`] 归约出的**半开区间** `[start, end)`（源采样帧）与
/// **已校验为正**的步进 `rate`（`yeban-sfz` 的 `Region::playback_rate`）。
///
/// | 方向 | 起点 `position` | 步进 `rate` | 行进边界 `bound` |
/// | :--- | :--- | :--- | :--- |
/// | [`PlayDirection::Forward`] | `start` | `+rate` | `end`（**上**界，半开） |
/// | [`PlayDirection::Reverse`] | `(end − 1) as f32`（区间**最后**一格） | `−rate` | `start`（**下**界，闭） |
///
/// # 为什么这样就够（⛔ 不需要"反向"这个字段）
///
/// ① **起点**：反向必须从区间的高端起步 —— `end − 1` 就是半开区间 `[start, end)` 里
/// **最后**一格（`reduce_span` 保证 `end > start ≥ 0` ⇒ `end ≥ 1`，⛔ 不会下溢）。
/// ② **步进**：取负 ⇒ [`mix_span_into`] 逐样本**递减** —— 那条路的除法/插值算术一个字不改。
/// ③ **边界**：`bound` 是声部**行进方向上的那一端** ⇒ 实时声部把它存进**既有的**
/// [`SfzVoice::source_end`] 格子里（⛔ 不新增字段）；`bound` 是**上界还是下界**由
/// **`rate` 的符号**唯一决定（[`mix_span_into`] 里唯一的那个 `reverse` 判别）。
///
/// ⭐ 等价性（可判）：反向那条路的输出**逐帧逐位等于**"把区间 `[start, end)` 的样本
/// **倒序**之后按 `forward` 渲染"—— 因为对任意 `t = i + f`（`i` 整数、`f ∈ [0,1)`），
/// `F(end − 1 − t) = pcm[end−1−i]·(1−f) + pcm[end−2−i]·f`
/// 与倒序数组 `R[j] = pcm[end−1−j]` 的 `G(t) = R[i]·(1−f) + R[i+1]·f` 是**同一个表达式**
/// （乘法两项各自操作数相同、加法在 binary32 下可交换）⇒ 判据可以拿"倒序样本 ＋
/// 不带 `direction` 的同一条路"当**独立参照**（判据 `S95`）。
///
/// ⚠ `end − 1` 经 `u64 → f32`：与 [`reduce_span`] 的 `start as f32` 同一条登记
/// （超过 2²⁴ 帧时会有位级舍入，⛔ 不假装精确）。
///
/// ⚠ **零分配、零锁、零 I/O**（纯整数/浮点比较与一次取负）⇒ 可被实时声部在**触发期**调用
/// [MUST-GATE-001]。
#[must_use]
pub fn reduce_travel(direction: PlayDirection, rate: f32, start: f32, end: u64) -> (f32, f32, u64) {
    match direction {
        PlayDirection::Forward => (start, rate, end),
        // `end >= 1` 由 `reduce_span` 保证（`end > start >= 0`）⇒ `end - 1` 不下溢。
        PlayDirection::Reverse => ((end - 1) as f32, -rate, start as u64),
    }
}

/// ⭐⭐ **速率可用性谓词**（任务单 `#117`）：三条渲染路**唯一**的速率守卫。
///
/// `rate` 可用 ⇔ **有限且非零**。⚠ **符号不参与判定**：`direction=reverse`
/// 的摊平／实时声部带**负** `rate`（[`reduce_travel`] 把"起点 ＝ 区间末格 ＋
/// 步进取负"折进去），那是**合法**输入 ⇒ 谓词用 `!= 0.0` 而不是 `> 0.0`。
///
/// # 为什么要有它（`#117` 实测）
///
/// `#104`～`#116` 的三条路各写各的守卫：
///
/// | 路 | 守卫的对象 | 谓词 |
/// | :--- | :--- | :--- |
/// | 离线 [`SfzInstrument::render_note_into_with_cc`] | **速率**（基率 × 乘子） | `is_finite && > 0.0` |
/// | 摊平 [`PreparedSfz::render_block_into_with_cc`] | **速率**（同上） | `is_finite && > 0.0` |
/// | ⭐ 实时 [`SfzVoicePool::slot_readings`] | ⭐ **乘子**（`tune_ratio`） | `is_finite && > 0.0` |
///
/// ⇒ 存在一档病态输入（有限乘子 `2^x` 与基率相乘**溢出成 `Inf`**）让前两路
/// **退休**、实时路**放行**（实测：前两路写 0 帧、实时拿 `Inf` 冲进逐样本循环）。
/// 本谓词把三处收口成**同一条**：判定的量都是"本块真正用的那个速率"。
///
/// ⚠ **与既有行为的关系**：两条一次性渲染路的速率恒**非负**（基率 > 0、
/// 乘子 ＝ `exp2(·) ≥ 0`）⇒ 对它们 `rate_is_usable` 与原来的 `> 0.0` 写法
/// **逐位等价**（既有判据 `S109` 等逐字仍绿）；实时路的 `voice.rate()` 在
/// `reverse` 时为负 ⇒ `!= 0.0` 才能保住反向播放（判据 `S97`）。
///
/// ⚠ 它**不是**幅值上界：`rate` 可以**有限但极大**（`2^x` 的有限上界 ≈ `f32::MAX`）
/// ⇒ 那种位置由 [`mix_span_into`] 的"越界即退休"闸门（`saturating_add(1)` 索引算术）
/// 如实收掉，⛔ 本谓词**不发明**第二个阈值（口径同台账第 259 条）。
#[must_use]
pub fn rate_is_usable(rate: f32) -> bool {
    rate.is_finite() && rate != 0.0
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
/// 3. **越界**：窗口终点超出**源采样区间**（[`reduce_span`] 的归约结果）或**超过** PCM
///    长度 ⇒ `None`。⚠ 线性插值要读 `index + 1`：窗口终点**恰好等于** `pcm_len`
///    （＝ 文件用 `loop_end = 总帧数` 表达"循环到结尾"）时，回绕点上那一格落在 PCM 之外
///    ⇒ [`mix_span_into`] 把下一格**钳到最后一个样本**（⛔ 越界读、⛔ panic 都不允许；
///    见该函数的"钳位"一节）。⭐ 任务单 `#97` **把这一档从"不循环"改成"循环 ＋ 钳位"**：
///    旧口径是 `loop_end >= pcm_len ⇒ None`，会把那种文件**静默变成不循环**；
///    ⚠ `loop_end == pcm_len - 1` 的行为⛔ 一字未动（窗口终点仍**不允许超过** `pcm_len`）；
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
    // ⭐ 任务单 `#97`：**超过** `pcm_len` 才拒绝（旧口径是 `>=`）。
    // 恰好等于 `pcm_len` 的窗口**照常循环** ⇒ 回绕点上的插值由 `mix_span_into` 钳位
    // （⛔ 不是把窗口判成"不循环"）。
    if loop_end as usize > pcm_len {
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

/// ⭐ **每区域的 CC 轴交叉淡化段数上界**（任务单 `#108`；摊平表的**定长**容器）。
///
/// # 这个数从哪来（⛔ 不是拍的）
///
/// 它是 `yeban-sfz` 的**结构性上界**，⛔ 不是语料统计的近似值：`read_crossfades` 把
/// CC 端点收在两张 `BTreeMap<u8, XfEndpoints>` 里（`cc_in` / `cc_out`），而
/// `parse_cc_crossfade_name` 只接受 `cc ≤ 127` ⇒ 每张表 ≤ **128** 个键、每个键至多
/// 产生 **1** 段（`XfEndpoints::resolve` 返回 `Some` 时）
/// ⇒ 任意长的一条 scope 链（`global` → `master` → `group` → `region`，含 `#include`
/// 展开的全部内容）归约出的 CC 段 **≤ 128 + 128 = 256**。
///
/// ⚠ **键是去重键**：`cc_in` / `cc_out` 是**按 CC 号索引的映射**，同一个 CC 在四级作用域
/// 里重复声明只会**覆盖端点**，⛔ 不会各留一段 —— 这正是上界只有 256（而不是
/// "声明处的条数"）的原因。
///
/// 登记语料普查（本机实测；分母 ＝ `assets/samples/**/*.sfz` 的 **1398** 份，其中
/// `#include` 能解析全的根文件 **1205** 份、归约出的 region 共 **64894** 条）：
/// **单区域 CC 轴段数最大 ＝ 2**
/// （`karoryfer-bear-sax/Programs/duo_legato/dynfade_legato_first_map.sfz`），
/// 直方图 `{0: 62436, 1: 1816, 2: 642}`；已知非零对照 ＝ 同批 region 的**全部**交叉淡化
/// 段数直方图 `{0: 62281, 1: 1920, 2: 693}`（最大同为 **2**）；CC 下标只用到 `{1, 85, 121}`。
/// ⇒ 取**结构性上界 256**（⛔ 不是"语料最大值 ＋ 余量"）的代价是每区域
/// `256 × size_of::<Crossfade>()`，换来的是**零截断可能**：⛔ 不需要给 `yeban-sfz`
/// 新增 `Err`、⛔ 不改任何既有文件的可解析性（越界在结构上不可达 ⇒
/// `flatten_regions` 的 `debug_assert` 是"绝不可达"的见证，与 `AMPEG_MOD_CAP` 同款）。
pub const CC_CROSSFADE_CAP: usize = 256;

/// ⭐ **每区域的 CC 门控（`loccN`/`hiccN`）条数上界**（任务单 `#109`；摊平表的**定长**容器）。
///
/// # 这个数从哪来（⛔ 不是拍的，也⛔ 不是语料最大值 ＋ 余量）
///
/// 它是 `yeban-sfz` 的**结构性上界**：`instrument.rs` 把四个作用域
/// （`global` → `master` → `group` → `region`）的 `loccN`/`hiccN` 归约进
/// 一张 `BTreeMap<u8, (Option<u8>, Option<u8>)>`（键 ＝ **CC 号**），再由
/// `parse_cc_gate_name` 把 CC 号夹在 `cc > 127 ⇒ None`（⛔ 不接受 128 及以上）
/// ⇒ 任意长的一条 scope 链（含 `#include` 展开的全部内容）归约出的门控
/// **≤ 128**（`u8` 的 `0..=127` 每一个至多 1 条）。
///
/// ⚠ **键是去重键**：同一个 CC 在四级作用域里重复声明只会**覆盖** `lo`/`hi`，
/// ⛔ 不会各留一条 —— 这正是上界只有 128（而不是"声明处的条数"）的原因。
///
/// 登记语料普查（本机实测，2026-10-16；分母 ＝ `assets/samples/**/*.sfz` 的
/// **1398** 份，其中 `#include` 能解析全的根文件 **1205** 份、归约出的 region
/// 共 **64894** 条 —— 与 `#108` 的同一读数**逐字相同**，证明两把尺子的 region
/// 归约口径一致）：**单区域 CC 门控条数最大 ＝ 3**
/// （`karoryfer-bear-sax/Programs/bearborg/medium_sides.sfz`），
/// 直方图 `{0: 43831, 1: 16872, 2: 797, 3: 3394}`；已知非零对照 ＝ 同批 region 的
/// **全部**交叉淡化段数直方图 `{0: 62281, 1: 1920, 2: 693}`（与 `#108` 登记逐字相同）；
/// 有门控的 region **21063** 条、门控条数**总计 28648**、用到的不同 CC 下标 **13** 个
/// （`{1, 80, 81, 83, 85, 86, 89, 92, 94, 107, 109, 119, 121}`）。
/// ⇒ 取**结构性上界 128**（⛔ 不是"语料最大值 3 ＋ 余量"）的代价是每区域
/// `128 × size_of::<CcGate>()`，换来的是**零截断可能**：⛔ 不需要给 `yeban-sfz`
/// 新增 `Err`、⛔ 不改任何既有文件的可解析性（越界在结构上不可达 ⇒
/// `flatten_regions` 的直接下标是"绝不会越界"的见证，与 `CC_CROSSFADE_CAP`／
/// `AMPEG_MOD_CAP` 同款）。⚠ 另一条路的代价（`#108` 的自裁口径）：取语料最大值 ＋
/// 新 `Err` 会把今天**能解析**的文件变成**不可解析** —— 本单不选。
pub const CC_GATE_CAP: usize = 128;

/// ⭐ **`set_ccN` 的播种表**（任务单 `#110`）：一条 SFZ 在**加载时**声明的 CC 初值。
///
/// # 规范逐字（<https://sfzformat.com/opcodes/set_ccN/>）
///
/// > Sets a default initial value for MIDI CC number N, when the instrument is
/// > initially loaded. Used under the ‹`control`› header.
///
/// 表格行：Type `integer`、Default `N/A`、**Range `0 to 127`**、Category
/// ＝ "Instrument Settings"；同页例子 `set_cc40=127` / `set_cc100=30`。
/// 解析层的唯一事实源是 `yeban_sfz::instrument::Instrument::cc_defaults`
/// （`<control>` 作用域、同名后者覆盖前者、取值越界是响亮 `Err`）。
///
/// # 域与形状（⛔ 不是"语料最大值 ＋ 余量"）
///
/// 域 ＝ **会话 CC 表**（[`crate::cc_state::CONTROL_CHANGE_COUNT`] ＝ `128`，即 `0..=127`）：
/// 值表 `[u8; 128]` ＋ 128 位"这个控制器有初值"掩码（与
/// [`crate::cc_state::ControlChangeState`] 同形）。⭐ 域与表**同界** ⇒ **零截断可能**
/// （`cc_defaults` 是 `BTreeMap` ⇒ 每个 CC 号至多一条），⛔ 不需要新增 `Err`、
/// ⛔ 不改任何既有文件的可解析性。
///
/// ⚠ ⭐ **域外声明被计数**、⛔ **不被掩码别名**：解析层接受 `N` 到 `u16::MAX`
/// （登记语料里真有 `set_cc400=63` / `set_cc401=63`，19 个文件 38 处 —— 见
/// [`Self::out_of_domain`] 的普查读数）。`400 & 0x7F = 16` ⇒ 直接喂给
/// [`crate::cc_state::ControlChangeState::seed`] 会**冒充 CC16**。本表⛔ **不做**那次掩码：
/// 域外的声明**只计数、不进表**（"表达不了"必须可见，⛔ 不是静默别名）。
///
/// # 语料普查（本机实测，2026-10-16；分母 ＝ `assets/samples/**/*.sfz` 的 **1398** 份）
///
/// | 读数（词边界口径 `(?<![A-Za-z0-9_])set_cc[0-9]+=`） | 值 |
/// | :--- | ---: |
/// | `set_ccN=` 出现次数 / 文件数 | **1137** / **105** |
/// | 取值字面不是十进制整数的次数 / 文件数（解析层丢弃 ＋ 告警） | 51 / 19（其中 `63.5` 42 处） |
/// | 取值是整数但 `> 127`（**响亮 `Err`**）的次数 | **0** |
/// | `N > 127` 的声明次数 / 文件数（**本表计数、不进表**） | 38 / 19（`set_cc400=63`／`set_cc401=63`） |
/// | 不同 `N` 的个数 / `N` 的范围 | 82 / `1..=401` |
/// | 单文件可进表的条数（`N ≤ 127`、`(文件,N)` 末者胜）最大 / 直方图 | **62** / `{1: 18, 2: 1, 3: 6, 4: 7, 6: 9, 7: 9, 8: 1, 9: 9, 10: 11, 11: 8, 12: 3, 13: 7, 14: 1, 16: 3, 17: 2, 19: 2, 22: 2, 30: 1, 31: 2, 57: 1, 62: 2}` |
/// | 有 ≥1 条可进表初值的文件数 | 105 |
/// | 已知非零对照（同一次扫描） | `label_cc` 1767 处 / 102 文件；`ampeg_release` 2831 处 / 373 文件；`ampeg_attack` 2828 处 / 368 文件；`note_polyphony` 87 处 / 32 文件 |
///
/// ⚠ **上界的来历是结构性的**：容量就是会话表的 128 个槽（⛔ 不是"语料最大 **62** ＋
/// 余量"）。语料最大值只说明"今天用不满"，它⛔ **不是**本表的界。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CcSeedTable {
    /// CC 号 → 初值（`0..=127`；只有掩码置位的位置有意义）。
    values: [u8; crate::cc_state::CONTROL_CHANGE_COUNT],
    /// "这个 CC 有 `set_ccN` 初值"的位图（字 0 = `0..=63`，字 1 = `64..=127`）。
    present: [u64; 2],
    /// **域外**（`N > 127`）的声明条数：⛔ 不进表、⛔ 不别名，只当读数。
    out_of_domain: usize,
}

impl CcSeedTable {
    /// 空表（没有任何初值）。`const` ⇒ 可在常量/构造期用。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            values: [0; crate::cc_state::CONTROL_CHANGE_COUNT],
            present: [0; 2],
            out_of_domain: 0,
        }
    }

    /// 从解析层的 `cc_defaults()` **建表**（控制侧；`BTreeMap` 的键序确定 [MODEL-AST-003]）。
    ///
    /// `N ≤ 127` ⇒ 进表；`N > 127` ⇒ 只进 [`Self::out_of_domain`] 计数。
    #[must_use]
    pub fn from_cc_defaults(defaults: &BTreeMap<u16, u8>) -> Self {
        let mut table = Self::new();
        for (cc, value) in defaults {
            table.insert(*cc, *value);
        }
        table
    }

    /// 写入一条声明：`cc ≤ 127` ⇒ 进表（同一 CC 后者覆盖前者）；否则只计数。
    pub fn insert(&mut self, cc: u16, value: u8) {
        match u8::try_from(cc) {
            Ok(cc) if usize::from(cc) < crate::cc_state::CONTROL_CHANGE_COUNT => {
                let index = usize::from(cc);
                self.values[index] = value & 0x7F;
                self.present[index / 64] |= 1u64 << (index % 64);
            }
            _ => self.out_of_domain = self.out_of_domain.saturating_add(1),
        }
    }

    /// 这个 CC 有没有声明初值（`false` ⇒ ⛔ 不播种：空表 ⇒ 一个字节都不动）。
    #[must_use]
    pub const fn has(&self, cc: u8) -> bool {
        let index = (cc & 0x7F) as usize;
        self.present[index / 64] & (1u64 << (index % 64)) != 0
    }

    /// 这个 CC 的初值（没声明 ⇒ `None`）。
    #[must_use]
    pub const fn value(&self, cc: u8) -> Option<u8> {
        if self.has(cc) {
            Some(self.values[(cc & 0x7F) as usize])
        } else {
            None
        }
    }

    /// **整张表是否为空**（＝ 这份文件一个 `set_ccN` 都没声明）。
    ///
    /// ⭐ 这是"**没有 `set_ccN` 的文件 ⇒ 播种机制一个字节都不动**"那条不变量的机械开关：
    /// 空表 ⇒ 快照边界⛔ 一次 `seed` 都不调 ⇒ `armed` 保持 `false`。
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.present[0] == 0 && self.present[1] == 0
    }

    /// 表里的初值条数（分母 128）。
    #[must_use]
    pub const fn len(&self) -> u32 {
        self.present[0].count_ones() + self.present[1].count_ones()
    }

    /// **域外**（`N > 127`）被计数、⛔ 未进表的声明条数。
    #[must_use]
    pub const fn out_of_domain(&self) -> usize {
        self.out_of_domain
    }

    /// 升序遍历全部初值（`cc` 升序 ⇒ 确定性；`BTreeMap` 来源顺序的机械再现）。
    ///
    /// 零分配、零锁、零 I/O（128 次位测试于**快照边界**，⛔ 不在逐样本路径上）。
    pub fn for_each(&self, mut f: impl FnMut(u8, u8)) {
        for index in 0..crate::cc_state::CONTROL_CHANGE_COUNT {
            if self.present[index / 64] & (1u64 << (index % 64)) != 0 {
                // `index < 128` ⇒ `as u8` 精确；掩码来自同一域。
                #[allow(clippy::cast_possible_truncation)]
                f(index as u8, self.values[index]);
            }
        }
    }
}

impl Default for CcSeedTable {
    fn default() -> Self {
        Self::new()
    }
}

/// 摊平表里 CC 段数组的**惰性填充**（只有 `..cc_crossfade_count` 会被读 ⇒ 取值无关）。
///
/// ⚠ 用**字面构造**（⛔ 不调 `Crossfade::new`：那不是 `const fn`）；`Crossfade` 的字段
/// 全是 `pub`。
const INERT_CC_CROSSFADE: Crossfade = Crossfade {
    axis: XfAxis::Cc(0),
    direction: XfDirection::In,
    range: XfRange::FADE_IN_DEFAULT,
    curve: XfCurve::Gain,
};

/// 摊平表里 CC 门控数组的**惰性填充**（只有 `..cc_gate_count` 会被读 ⇒ 取值无关）。
///
/// ⚠ 用**字面构造**（⛔ 不调任何构造函数）；[`CcGate`] 的字段全是 `pub`。
const INERT_CC_GATE: CcGate = CcGate {
    cc: 0,
    lo: 0,
    hi: 0,
};

/// 键/力度的可播性、以及**全部静态门控**（通道 / `trigger` 事件 / keyswitch / CC）
/// 在**构造期**由 `yeban-sfz` 的 `Region::matches_query` 求值后**打包成 128 位掩码** ⇒
/// 回调侧只做位测试，⛔ 不在回调里调用任何解析/分配函数。
/// 门控未通过的区域**两张掩码都清 0** ⇒ 它**永远不会**进回调的命中集
/// （含 `trigger=release` 一类的 region）。
///
/// 增益按 `PlaybackSpec::total_gain()` 的乘积结构拆成五份（`volume`／力度／键轴淡化／
/// 力度轴淡化／**CC 轴淡化**），使回调侧的相乘顺序与离线路径**逐位相同**：
/// `(volume × 力度) × ((((键轴 × 力度淡入) × 力度淡出) × cc₀) × cc₁ …)`。
/// 因此 `(note, velocity)` 的取值**不再冻结**在 `(60, 100)`，
/// 而 CC 轴（任务单 `#108`）由 [`Self::cc_crossfades`] 承载、**逐块**按探针求值。
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
    /// ⭐ **CC 调制项**（任务单 `#104`；⭐ `#115` 起目标由 [`ModTarget`] 区分）：
    /// `ccN` / `_ccN` / `_onccN` / `_curveccN` 的归约结果，**曲线已烘进本结构**
    /// （[`BakedCcMod`] / [`BakedCurve`]）⇒ 求值不再需要 `Instrument`，
    /// 也⛔ 不额外分配。
    ///
    /// 有效范围是 `..cc_mod_count`（定长槽的空槽是 [`BakedCcMod::EMPTY`]，
    /// ⛔ 不能靠值判断）。顺序由 `yeban-sfz` 的归约唯一确定
    /// （[`ModTarget`] 声明序 × CC 升序）。
    ///
    /// ⚠ **它是"区域参数"、不是"声部状态"**：逐声部的推进状态仍在 [`SfzVoice`]，
    /// ⇒ 本字段⛔ 不进实时对象（`SfzVoice` 尺寸不变）。
    ///
    /// ⚠ **沿革（任务单 `#115`）**：`#104` 登记为 `ampeg_mods: [BakedAmpegCcMod; …]`
    /// ＋ `ampeg_mod_count`；`#115` 改名并把目标推广成 [`ModTarget`]
    /// （⛔ 定长槽的形状与"⛔ 不靠值判断"的口径一字未改，且本条**不改变尺寸**）。
    ///
    /// [BakedCurve]: yeban_sfz::modulation::BakedCurve
    cc_mods: [BakedCcMod; AMPEG_MOD_CAP],
    /// ⭐ 上面那张表里**有效**的条数（`0..=AMPEG_MOD_CAP`）。
    cc_mod_count: u8,
    /// ⭐ **`amplitude` 基准值**（**线性百分比**；任务单 `#115`）。
    ///
    /// 规范表行 Default ＝ **100**（＝ 无音量变化）；`amplitude_onccN` 的 `delta` 与它
    /// **代数相加**（公式与逐字出处见 [`yeban_sfz::modulation`] 模块文档 §5），
    /// 再除以 100 得到**最外层线性增益因子**。
    ///
    /// ⚠ **它是"区域参数"、不是"声部状态"**（CC 逐块从探针读）⇒ ⛔ 不进实时对象。
    /// ⚠ **沿革（任务单 `#115`）**：本字段是 `#115` 新增的**唯一**每区域开销
    /// （`PreparedRegion` 6160 → 6168，判据 `S100` 现读）。
    amplitude: f32,
    /// ⭐ **`ampeg_*` 的力度调制量**（`ampeg_vel2attack` 一族；任务单 `#106`）。
    ///
    /// 下标 ＝ [`AmpegParam`] 的声明序；`None` ＝ 该参数的 `ampeg_vel2…` opcode
    /// **根本没出现** ⇒ 求值期**一项算术都不做**（与接线前逐位相同）；
    /// `Some(delta)` ⇒ 末尾加 `delta × velocity / 127`。
    ///
    /// ⚠ **与 [`Self::cc_mods`] 是两支独立输入**：这支来自**音符力度**
    /// （[`SfzVoice::velocity`]，触发后不变），那支来自 **CC 状态**（探针）
    /// ⇒ 两者**相加**，⛔ 不互相顶替。
    ///
    /// ⚠ **它是"区域参数"、不是"声部状态"**：力度本身在 [`SfzVoice`] 里**早就有**
    /// （触发期抄下）⇒ 本字段⛔ 不进实时对象（`SfzVoice` 尺寸不变）。
    ampeg_vel2: [Option<f32>; AmpegParam::COUNT],
    /// ⭐ **CC 轴交叉淡化的几何**（任务单 `#108`）：`XfAxis::Cc(cc)` 的段**原样**
    /// （`Crossfade` 是 `Copy` 的值类型）搬进摊平表，顺序 ＝ `yeban-sfz` 的
    /// `read_crossfades` 归约序（CC 淡入按 CC 号升序，再 CC 淡出按 CC 号升序）。
    ///
    /// 只有 `..cc_crossfade_count` 有效（定长槽的其余部分是 [`INERT_CC_CROSSFADE`]，
    /// ⛔ 不能靠值判断）；容量 [`CC_CROSSFADE_CAP`] **＝ 结构性上界**（它的文档里有
    /// 由来、语料普查读数与代价）⇒ **没有截断这回事**。
    ///
    /// ⚠ **为什么烘**：实时回调拿不到 `Instrument`（摊平之后只剩本结构）—— 求值期只需
    /// [`Crossfade::gain_at`]（`yeban-sfz` 的**同一份**实现：整数减法、一次 `f32` 除法、
    /// `power` 曲线时一次 `sqrt`），⛔ 不是第二份换算。
    ///
    /// ⚠ **为什么它是"区域参数"、⛔ 不是"声部状态"**：CC 值**逐块**变（探针由调用方沿
    /// `with_cc` 那条链交下来），几何本身却是静态的 ⇒ 段数组不进 [`SfzVoice`]，
    /// 声部尺寸 ⛔ 不变（判据 `S9` 钉住 40 B）。
    cc_crossfades: [Crossfade; CC_CROSSFADE_CAP],
    /// ⭐ 上面那张表里**有效**的条数（`0..=CC_CROSSFADE_CAP`）。
    ///
    /// ⚠ `u16`（⛔ 不是 `u8`）：结构性上界是 **256**，`u8` 装不下 256。
    cc_crossfade_count: u16,
    /// ⭐ **CC 区域门控的几何**（任务单 `#109`）：`Region::cc_gates` 的 `loccN`/`hiccN`
    /// 闭区间**原样**（[`CcGate`] 是 `Copy` 的值类型）搬进摊平表，顺序 ＝ `yeban-sfz`
    /// 的归约序（按 CC 号升序）。
    ///
    /// 只有 `..cc_gate_count` 有效（定长槽的其余部分是 [`INERT_CC_GATE`]，
    /// ⛔ 不能靠值判断）；容量 [`CC_GATE_CAP`] **＝ 结构性上界**（它的文档里有由来、
    /// 语料普查读数与代价）⇒ **没有截断这回事**。
    ///
    /// ⚠ **为什么烘**：实时回调拿不到 `Instrument`（摊平之后只剩本结构），而门控是
    /// **区域选择**的判据 ⇒ 它必须在**触发期**按会话运行态的探针求值
    /// （[`Self::cc_gates_ok`]）。规范逐字与判定时机的依据见模块头的
    /// 「⭐ ⭐ **追加修订段（任务单 `#109`）**」那一节。
    ///
    /// ⚠ **为什么它是"区域参数"、⛔ 不是"声部状态"**：CC 值**逐块**变（探针由调用方
    /// 沿 `with_cc` 那条链交下来），几何本身却是静态的 ⇒ 数组不进 [`SfzVoice`]，
    /// 声部尺寸 ⛔ 不变（判据 `S9` 钉住 40 B）。
    cc_gates: [CcGate; CC_GATE_CAP],
    /// ⭐ 上面那张表里**有效**的条数（`0..=CC_GATE_CAP`）。
    ///
    /// ⚠ `u8`（⛔ 不是 `u16`）：结构性上界是 **128**，`u8` 恰好装得下（`128` 落
    /// `u8` 内）；`usize` 会白占 7 字节填充。
    cc_gate_count: u8,
    /// ⭐ **独占组号**（`group`，缺省 `0`；别名 `polyphony_group`）。
    ///
    /// 语义（SFZ v1 逐字，见 <https://sfzformat.com/opcodes/group/>）：
    /// "Exclusive group number for this region." —— 它标的是**本区域属于哪个独占组**，
    /// 被别的区域的 `off_by` 用来**指名关断**（本片：任务单 `#94`）。
    ///
    /// ⚠ 同一个 `group` 还是 [`Self::note_polyphony`] 的检查键（`yeban-sfz` 的
    /// [`NotePolyphony`] 已经把"无 group 时按 `group=0`"写进文档）⇒ 本字段**一片两用**，
    /// ⛔ 不是两个字段。⚠ 沿革：任务单 `#94` 落地时本行写的是"**下一片** `note_polyphony`
    /// 的检查键" ⇒ 那一瓣已由任务单 `#100` 接线（判据 S50 把"键 ＝ `group` × 音高"钉住）。
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
    /// * `time`（ARIA）：本片**按 `normal` 处理**并逐字登记为书面偏离 —— 规范说的是
    ///   **一段长度 ＝ [`yeban_sfz::instrument::Region::off_time`] 的淡出**、
    ///   **independent of the sample release time**（缺省 6 ms）；⭐ 本仓**不读
    ///   per-region `off_time`**、`time` 按 `normal` 处理（`ADR-0008` 裁决 **C**），
    ///   而 `off_time` 不在本片范围内；与 `ARCH-RT-004` 的 3 ms（**窃取路**）**不冲突**
    ///   —— 两条不同的路；见模块头的"off_by 关断"一节。
    off_mode: OffMode,
    /// ⭐ **播放方向**（`direction`；任务单 `#113`）。
    ///
    /// 解析与取值域在 `yeban-sfz` 里**早已落地**（[`PlayDirection`]：`forward` 缺省 /
    /// `reverse`；白名单拒绝其余取值）⇒ 本字段只把那份 `Copy` 读数搬进摊平表，
    /// ⛔ 不新增解析、⛔ 不改取值域。
    ///
    /// ⚠ **它是"区域参数"、⛔ 不是"声部状态"**：本单的落地口径是
    /// **触发／求值期把方向折进「起点 ＋ 带符号步进 ＋ 行进边界」三件已有读数**
    /// （[`reduce_travel`]）⇒ ⛔ `SfzVoice` 一个字段都不加（判据 `S9` 仍钉 40 B）。
    /// 生效口径、规范逐字与"为什么 `alternate` 未做"见模块头的
    /// 「⭐ 播放方向（`direction`，任务单 `#113`）」一节。
    direction: PlayDirection,
    /// ⭐ **同音同时发声数上限**（`note_polyphony` ＋ `note_selfmask`；任务单 `#100`）。
    ///
    /// 解析与改写（含 `limit = 0` ⇒ 不限制、`note_selfmask` 白名单）在 `yeban-sfz` 里
    /// **早已落地**（[`yeban_sfz::instrument::NotePolyphony`]，判据在
    /// `yeban-sfz` 的 `instrument.rs` / `malformed_inputs.rs`）⇒ 本片只把这份**已经裁决好**
    /// 的读数搬进摊平表，⛔ 不新增解析、⛔ 不改既有判定规则。
    ///
    /// 语义（规范逐字见 [`NotePolyphony`] 的文档）：检查的键是
    /// **polyphony group × 音高**（group 由本区域的 [`Self::group`] 给出，缺省 0）
    /// —— "The note polyphony is checked within a polyphony group, set by the `group` or
    /// `polyphony_group` opcodes."。
    note_polyphony: NotePolyphony,
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

    /// ⭐ **CC 区域门控**（`loccN`/`hiccN`，任务单 `#109`）是否成立。
    ///
    /// 语义 ＝ `yeban-sfz` 的 `Region::cc_gates_ok` **逐字**再实现一遍的必要部分：
    /// 每一条门控要求 `lo ≤ cc(cc号) ≤ hi`（**闭区间**，两端都含）；空集合恒 `true`
    /// （⛔ 不查探针 ⇒ 没有 CC 门控的区域与接线前**逐位相同**）。
    ///
    /// 规范逐字（<https://sfzformat.com/opcodes/loccN/>）：
    /// "The region will play only if last MIDI controller 74 received was in the
    /// 30 to 100 range." ＋ "This is a 'hard' switch - if a region is within the locc to
    /// hicc range it plays, if it's outside that range it does not play."
    /// ⇒ 它是**区域选择**（"要不要这条区域"），⛔ 不是逐块调制 ⇒ 判定时机 ＝ **触发期**。
    ///
    /// ⚠ **严格策略与 `yeban-sfz` 逐字同款**：`cc == None`（＝ **整张会话表一条 CC
    /// 都没收到过**，见 [`crate::cc_state::ControlChangeState::is_armed`]）时，**有**门控的
    /// 区域**不成立**（`region_matches` 的 `let Some(probe) = query.cc else { return false }`
    /// 同一条）—— ⛔ 本仓不去替调用方发明规范缺省（那是 `set_ccN` 播种那一族的事，
    /// 登记见模块头）。没有门控的区域⛔ 不看探针 ⇒ `None` 下逐位不变。
    ///
    /// **零分配、零锁、零 I/O**：≤ [`CC_GATE_CAP`] 条定长数组读 ＋ 每条一次整数区间测试。
    #[must_use]
    pub fn cc_gates_ok(&self, cc: Option<&dyn Fn(u8) -> u8>) -> bool {
        if self.cc_gate_count == 0 {
            return true;
        }
        let Some(probe) = cc else {
            return false;
        };
        let active = &self.cc_gates[..usize::from(self.cc_gate_count)];
        active
            .iter()
            .all(|gate| (gate.lo..=gate.hi).contains(&probe(gate.cc)))
    }

    /// ⭐ 摊平表里承载的 CC 门控**只读副本**（判据用：必须与 `Region::cc_gates` 逐字相同）。
    #[must_use]
    pub fn cc_gates(&self) -> &[CcGate] {
        &self.cc_gates[..usize::from(self.cc_gate_count)]
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

    /// ⭐ 本区域的**同音同时发声数上限**（`note_polyphony` ＋ `note_selfmask`，原样）。
    ///
    /// 判定函数是 [`SfzVoicePool::enforce_note_polyphony`]（触发期一次 ≤ 16 槽的线性扫描）。
    /// 只读副本 ⇒ 判据可以直接回读"摊平表**真的**带上了这份读数"，⛔ 不是只看解析层。
    #[must_use]
    pub const fn note_polyphony(&self) -> NotePolyphony {
        self.note_polyphony
    }

    /// ⭐ 本区域的**播放方向**（`direction`，原样；缺省 [`PlayDirection::Forward`]）。
    ///
    /// 只读副本 ⇒ 判据可以直接回读"摊平表**真的**带上了这份读数"，⛔ 不是只看解析层。
    /// 求值口径见 [`reduce_travel`]（⭐ 三条路共用它）。
    #[must_use]
    pub const fn direction(&self) -> PlayDirection {
        self.direction
    }

    /// 给定时点的**合并线性增益**（与离线 `PlaybackSpec::total_gain` 逐位相同）。
    ///
    /// ⚠ 本方法是 [`Self::gain_at_with_cc`] 传 `cc == None` 的**薄包装**
    /// ⇒ "没有 CC 状态"与接线前（`#108` 之前）的读数**逐位相同**
    /// （`XfAxis::Cc` 段按 `yeban-sfz` 的"未提供状态 ⇒ 不贡献"处理）。
    #[must_use]
    pub fn gain_at(&self, note: u8, velocity: u8) -> f32 {
        self.gain_at_with_cc(note, velocity, None)
    }

    /// ⭐ **合并线性增益 ＋ CC 探针**（任务单 `#108`）：与离线
    /// [`yeban_sfz::playback::PlaybackSpec::total_gain`] **逐位相同**的求值点，
    /// 只是 CC 轴按**当前**探针取值（CC 逐块可变 ⇒ 本方法每块都可能被调）。
    ///
    /// 乘积结构与结合序（**唯一**，[ARCH-DET-001]）：
    ///
    /// ```text
    /// base      = volume_gain * velocity_gain[v]
    /// crossfade = (key_crossfade[n] * velocity_fade_in[v]) * velocity_fade_out[v]
    /// 逐段：crossfade *= cc_crossfades[i].gain_at(probe(cc_i))   // i 递增
    /// 结果       = (base * crossfade) * amplitude_gain           // ⭐ #115 最外层
    /// ```
    ///
    /// ⭐ **振幅（任务单 `#115`）**：`amplitude_gain` ＝
    /// [`Self::amplitude_gain_with_cc`]（`(amplitude + Σ delta × curve(cc)) / 100`）。
    /// 它是**最外层**乘子（`total_gain × amplitude_gain`），与离线
    /// [`SfzInstrument::render_note_into_with_cc`] **同一份**次序 ⇒ 三条路逐位一致。
    /// ⚠ 缺省 `amplitude == 100` ⇒ 因子恰 `1.0` ⇒ 乘它**逐位不变**（判据 `S100`/`S101`）。
    ///
    /// ⚠ 三段摊平表的结合序与 `yeban-sfz` 的 `crossfade_gain` 累加器**逐位相同**
    /// （`1.0` 乘是精确运算）⇒ 上式 ＝ `(gain * velocity_gain) * crossfade_gain(query)`，
    /// 即离线那条路的 `total_gain()`（既有判据 `S…` 的 128×128 网格对账覆盖
    /// `cc = None`；`#108` 的判据把同一份对账扩到 CC 轴）。
    ///
    /// * `cc == None` ⇒ CC 段**一段都不乘**（与 `Region::crossfade_gain` 对
    ///   "未提供状态"的 `continue` 同口径）；`cc_crossfade_count == 0` 的区域
    ///   ⇒ 循环体⛔ 不执行 ⇒ **一个浮点运算都不加**。
    ///
    /// **零分配、无锁、无 I/O、无日志**：只读定长数组与探针 [MUST-GATE-001]。
    #[must_use]
    pub fn gain_at_with_cc(&self, note: u8, velocity: u8, cc: Option<&dyn Fn(u8) -> u8>) -> f32 {
        let n = note as usize;
        let v = velocity as usize;
        let base = self.volume_gain * self.velocity_gain[v];
        let mut crossfade =
            (self.key_crossfade[n] * self.velocity_fade_in[v]) * self.velocity_fade_out[v];
        if let Some(probe) = cc {
            for baked in &self.cc_crossfades[..usize::from(self.cc_crossfade_count)] {
                // 摊平期只装 `XfAxis::Cc(cc)`（见 `flatten_regions` 的 CC 分支）。
                if let XfAxis::Cc(controller) = baked.axis {
                    crossfade *= baked.gain_at(probe(controller));
                }
            }
        }
        // ⭐ 振幅（任务单 `#115`）：最外层线性乘子；缺省 `amplitude = 100` ⇒ 因子 `1.0`
        // ⇒ 这一句对没有 `amplitude` 的区域**逐位不变**（IEEE：× `1.0` 恒等）。
        // ⭐⭐ 音量调制（任务单 `#119`）：**新的最外层**线性乘子；缺省（无 `volume_*ccN`）
        // ⇒ 因子恰 `1.0` ⇒ 同样**逐位不变**。结合序与离线那条路**逐字相同**
        // （`(… × amplitude_gain) × volume_ratio`，判据 `S114`）。
        ((base * crossfade) * self.amplitude_gain_with_cc(cc)) * self.volume_ratio_with_cc(cc)
    }

    /// ⭐ 本区域承载的 CC 轴交叉淡化段数（判据用：`0` ＝ 这条区域没有该轴）。
    #[must_use]
    pub const fn cc_crossfade_count(&self) -> usize {
        self.cc_crossfade_count as usize
    }

    /// ⭐ 本区域承载的 CC 轴交叉淡化段的**只读副本**（判据用：逐段回读几何，
    /// 证明摊平表**真的**带上了这份读数，⛔ 不是只看解析层）。
    #[must_use]
    pub fn cc_crossfades(&self) -> &[Crossfade] {
        &self.cc_crossfades[..usize::from(self.cc_crossfade_count)]
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
    ///
    /// ⭐ 本方法是 [`Self::ampeg_run_with_cc`] 传 `probe == None` 的**薄包装**
    /// ⇒ "没有 CC 状态"与接线前的读数**逐位相同**（⛔ 不是第二份换算）。
    #[must_use]
    pub fn ampeg_run(&self, engine_hz: f32) -> Option<AmpegRun> {
        self.ampeg_run_with_cc(engine_hz, None)
    }

    /// ⭐ **该区域某 `ampeg_*` 参数的基准值**（不含 CC 调制）。
    #[must_use]
    pub const fn ampeg_base(&self, param: AmpegParam) -> f32 {
        match param {
            AmpegParam::Attack => self.ampeg_attack,
            AmpegParam::Hold => self.ampeg_hold,
            AmpegParam::Decay => self.ampeg_decay,
            AmpegParam::Sustain => self.ampeg_sustain,
            AmpegParam::Release => self.ampeg_release,
        }
    }

    /// ⭐ **该区域承载的 CC 调制项**（任务单 `#104`；⭐ `#115` 起带 [`ModTarget`]；
    /// 曲线已烘焙）。
    ///
    /// 空切片 ＝ 该区域没有该族 opcode（⇒ `probe` 给不给都一样）。零分配。
    #[must_use]
    pub fn cc_modulations(&self) -> &[BakedCcMod] {
        &self.cc_mods[..usize::from(self.cc_mod_count)]
    }

    /// ⚠ **沿革别名**（台账第 278 条）：`#104` 的登记名 `ampeg_modulations`；
    /// `#115` 推广后的正名是 [`Self::cc_modulations`]（⛔ 语义未变、⛔ 不是第二份数据）。
    #[must_use]
    pub fn ampeg_modulations(&self) -> &[BakedCcMod] {
        self.cc_modulations()
    }

    /// ⭐ **`amplitude` 的基准值**（线性百分比；任务单 `#115`）。
    ///
    /// 规范表行 Default ＝ **100** ⇒ 没有 `amplitude` opcode 的区域本读数恰 `100.0`。
    #[must_use]
    pub const fn amplitude(&self) -> f32 {
        self.amplitude
    }

    /// ⭐ **振幅的线性因子**（任务单 `#115`）：
    /// `(amplitude + Σ delta × curve(cc)) / 100`。
    ///
    /// * `cc == None` ⇒ 一项 CC 都不加 ⇒ `amplitude / 100`（缺省 ⇒ 恰 `1.0`）；
    /// * 求值本体与离线、摊平两条路**同一份**（[`cc_value_with_baked_and_vel2`]，
    ///   `vel2 = None`、`target = Amplitude`）；
    /// * 曲线解析不到的项**不贡献**量（⛔ 不当成曲线 0；可由
    ///   [`Self::ampeg_unresolved_mods`] 读出）。
    ///
    /// ⚠ **零分配、零锁、零 I/O**（纯算术 ＋ 定长切片）⇒ 可在逐块路径调用。
    #[must_use]
    pub fn amplitude_gain_with_cc(&self, cc: Option<&dyn Fn(u8) -> u8>) -> f32 {
        let (value, _unresolved) = cc_value_with_baked_and_vel2(
            self.amplitude,
            ModTarget::Amplitude,
            self.cc_modulations(),
            None,
            0,
            cc,
        );
        value / AMPLITUDE_PERCENT_SCALE
    }

    /// ⭐ 表里**曲线无法解析**的调制项数（内建 `4..=6` 规范未给公式、`<curve>` 未定义、
    /// 或点数超过 [`MAX_BAKED_CURVE_POINTS`]）。
    ///
    /// 这些项**不贡献**任何量（⛔ 不当成曲线 0），本读数是它们的**响亮**出口
    /// （登记见本模块头的"CC 调制"一节）。
    ///
    /// [MAX_BAKED_CURVE_POINTS]: yeban_sfz::modulation::MAX_BAKED_CURVE_POINTS
    #[must_use]
    pub fn ampeg_unresolved_mods(&self) -> usize {
        self.ampeg_modulations()
            .iter()
            .filter(|item| !item.curve.is_resolved())
            .count()
    }

    /// ⭐⭐ **`tune` 调制的速率乘子**（任务单 `#116`）：
    /// `2 ^ (Σ delta × curve(cc) / 1200)`。
    ///
    /// * `cc == None` ⇒ 一项 CC 都不读 ⇒ **恰 `1.0`**（`rate × 1.0` 在 binary32 下
    ///   **逐位不变** ⇒ 与接线前逐位相同）；
    /// * `cc == Some(probe)` 而表里没有 `tune` 项 ⇒ `0.0f32.exp2()` **恰 `1.0`** ⇒ 同样逐位不变；
    /// * 求值本体是 `yeban-sfz` 的 [`tune_ratio_with_baked`]（离线 ／ 摊平 ／ 实时三条路
    ///   **同一份**实现，⛔ 没有第二套换算）；
    /// * 曲线解析不到的项**不贡献**音分（⛔ 不当成曲线 0；可由
    ///   [`Self::ampeg_unresolved_mods`] 读出）；
    /// * ⛔ **不钳位**：域的裁定（`tune` ⇒ ±9600 音分）见 [`ModTarget::delta_range`]；
    ///   `2^x` 的上溢／下溢边界由调用方按"速率必须有限非零"的既有守卫如实退休
    ///   （见 [`Self::tune_ratio_with_cc`] 的调用点 [`Self::slot_readings`]）。
    ///
    /// ⚠ **零分配、零锁、零 I/O**（纯算术 ＋ 定长切片 ＋ 一次 `exp2`）⇒ 可逐块调用。
    #[must_use]
    pub fn tune_ratio_with_cc(&self, cc: Option<&dyn Fn(u8) -> u8>) -> f32 {
        tune_ratio_with_baked(self.cc_modulations(), cc)
    }

    /// ⭐⭐ **`volume` 调制的线性增益乘子**（任务单 `#119`）：
    /// `2 ^ (Σ delta × curve(cc) × log2(10) / 20)`（＝ `10 ^ (dB / 20)`）。
    ///
    /// * `cc == None` ⇒ 一项 CC 都不读 ⇒ **恰 `1.0`**（`gain × 1.0` 在 binary32 下
    ///   **逐位不变** ⇒ 与接线前逐位相同）；
    /// * `cc == Some(probe)` 而表里没有 `volume` 项 ⇒ `0.0f32.exp2()` **恰 `1.0`**
    ///   ⇒ 同样逐位不变；
    /// * 求值本体是 `yeban-sfz` 的 [`volume_ratio_with_baked`]（离线 ／ 摊平 ／ 实时
    ///   三条路**同一份**实现，⛔ 没有第二套换算）；
    /// * 曲线解析不到的项**不贡献** dB（⛔ 不当成曲线 0；可由
    ///   [`Self::ampeg_unresolved_mods`] 读出）；
    /// * ⛔ **不钳位**：域的裁定（`volume` ⇒ `-144..=48` dB）见
    ///   [`ModTarget::delta_range`]；`2^x` 的上溢（`Inf`）／下溢（`0`）由**既有**增益
    ///   通路如实通过（增益**没有**"必须有限"的守卫 —— 与 `tune` 的速率守卫不同）。
    ///
    /// ⚠ **零分配、零锁、零 I/O**（纯算术 ＋ 定长切片 ＋ 一次 `exp2`）⇒ 可逐块调用。
    #[must_use]
    pub fn volume_ratio_with_cc(&self, cc: Option<&dyn Fn(u8) -> u8>) -> f32 {
        volume_ratio_with_baked(self.cc_modulations(), cc)
    }

    /// ⭐ **该区域某参数上的力度调制量**（`ampeg_vel2…`；任务单 `#106`）。
    ///
    /// `None` ＝ 该参数的 `ampeg_vel2…` opcode **根本没出现**
    /// （⇒ 求值期一项算术都不做）；`Some(delta)` ＝ 末尾加 `delta × velocity / 127`。
    /// 语义与公式逐字出处见 `yeban_sfz::modulation`。零分配、`const`。
    #[must_use]
    pub const fn ampeg_vel2(&self, param: AmpegParam) -> Option<f32> {
        self.ampeg_vel2[param.index()]
    }

    /// ⭐ **求某参数在给定 CC 探针下的取值**：`base + Σ delta × curve(cc)`。
    ///
    /// * `probe == None` ⇒ 基准值 ＋ 未施加数 `0`（＝ [`Self::ampeg_base`]）；
    /// * `probe == Some` ⇒ 逐项相加；曲线解析不到的项**计进第二个返回值**且不贡献量。
    ///
    /// 求值本体是 `yeban-sfz` 的 [`cc_value_with_baked`]（与离线
    /// [`SfzInstrument::render_note_into_with_cc`] **同一份**实现）。
    ///
    /// ⚠ **零分配、零锁、零 I/O**（纯算术 ＋ 定长切片）⇒ 可在逐块路径调用。
    #[must_use]
    pub fn ampeg_value_with_cc(
        &self,
        param: AmpegParam,
        probe: Option<&dyn Fn(u8) -> u8>,
    ) -> (f32, usize) {
        let base = self.ampeg_base(param);
        let Some(probe) = probe else {
            return (base, 0);
        };
        cc_value_with_baked(base, ModTarget::Ampeg(param), self.cc_modulations(), probe)
    }

    /// ⭐ **逐块归约 ＋ CC 调制**（任务单 `#104`）：先按 CC 探针求五个参数，再走
    /// [`ampeg_run_from`] 的**同一份**帧换算。
    ///
    /// * `probe == None` ⇒ 与 [`Self::ampeg_run`] **逐位相同**（⛔ 没有第二条换算路径）；
    /// * `probe == Some` ⇒ 五个参数各自 `base + Σ delta × curve(cc)`；
    /// * 曲线解析不到的项**不贡献**量（可由 [`Self::ampeg_unresolved_mods`] 读出）。
    ///
    /// ⚠ **零分配、零锁、零 I/O**。
    #[must_use]
    pub fn ampeg_run_with_cc(
        &self,
        engine_hz: f32,
        probe: Option<&dyn Fn(u8) -> u8>,
    ) -> Option<AmpegRun> {
        if probe.is_none() {
            return ampeg_run_from(
                self.ampeg_attack,
                self.ampeg_hold,
                self.ampeg_decay,
                self.ampeg_sustain,
                self.ampeg_release,
                engine_hz,
            );
        }
        let probe = probe?;
        // `AmpegParam::ALL` 是声明序 ⇒ 五个读数各自独立求值，顺序确定。
        let mut values = [0.0f32; 5];
        for param in AmpegParam::ALL {
            values[param.index()] = self.ampeg_value_with_cc(param, Some(probe)).0;
        }
        ampeg_run_from(
            values[0], values[1], values[2], values[3], values[4], engine_hz,
        )
    }

    /// ⭐ **该区域某 `ampeg_*` 参数在 CC 探针 ＋ 音符力度下的取值**（任务单 `#106`）：
    /// `base + Σ delta_cc × curve(cc) + delta_vel2 × velocity / 127`
    /// （公式、逐字出处与工程裁决见 `yeban_sfz::modulation` 的模块文档 §4）。
    ///
    /// * `probe == None` ⇒ 不读任何 CC（CC 项一项都不加）；
    /// * 力度项照算（`ampeg_vel2(param) == None` ⇒ 一项算术都不做）；
    /// * 曲线解析不到的项**计进第二个返回值**且不贡献量
    ///   （可由 [`Self::ampeg_unresolved_mods`] 读出）。
    ///
    /// 求值本体是 `yeban-sfz` 的 [`cc_value_with_baked_and_vel2`]（与离线
    /// [`SfzInstrument::render_note_into_with_cc`] **同一份**实现）。
    ///
    /// ⚠ **零分配、零锁、零 I/O**（纯算术 ＋ 定长切片）⇒ 可在逐块路径调用。
    #[must_use]
    pub fn ampeg_value_with_cc_and_velocity(
        &self,
        param: AmpegParam,
        velocity: u8,
        probe: Option<&dyn Fn(u8) -> u8>,
    ) -> (f32, usize) {
        cc_value_with_baked_and_vel2(
            self.ampeg_base(param),
            ModTarget::Ampeg(param),
            self.cc_modulations(),
            self.ampeg_vel2(param),
            velocity,
            probe,
        )
    }

    /// ⭐ **逐块归约 ＋ 力度调制 ＋ CC 调制**（任务单 `#106`）：先按**音符力度**与 CC
    /// 探针求五个参数，再走 [`ampeg_run_from`] 的**同一份**帧换算。
    ///
    /// ⚠ 与 [`Self::ampeg_run_with_cc`] 的分工：那一支**没有力度输入**
    /// ⇒ `ampeg_vel2*` **不参与**（⛔ 不是"按力度 0 求值"）；本方法把力度取进来，
    /// 是**实时**与**摊平**两条路的入口（[`SfzVoicePool::slot_readings`] 用
    /// [`SfzVoice::velocity`]，[`PreparedSfz::render_block_into_with_cc`] 用调用方的参数）。
    ///
    /// * `probe == None` **且**该区域没有任何 `ampeg_vel2*` ⇒ 与 [`Self::ampeg_run`]
    ///   **逐位相同**（⛔ 没有第二条换算路径）；
    /// * 曲线解析不到的项**不贡献**量（可由 [`Self::ampeg_unresolved_mods`] 读出）。
    ///
    /// ⚠ **零分配、零锁、零 I/O**。
    #[must_use]
    pub fn ampeg_run_with_cc_and_velocity(
        &self,
        engine_hz: f32,
        velocity: u8,
        probe: Option<&dyn Fn(u8) -> u8>,
    ) -> Option<AmpegRun> {
        if probe.is_none() && self.ampeg_vel2.iter().all(Option::is_none) {
            return ampeg_run_from(
                self.ampeg_attack,
                self.ampeg_hold,
                self.ampeg_decay,
                self.ampeg_sustain,
                self.ampeg_release,
                engine_hz,
            );
        }
        // `AmpegParam::ALL` 是声明序 ⇒ 五个读数各自独立求值，顺序确定。
        let mut values = [0.0f32; 5];
        for param in AmpegParam::ALL {
            values[param.index()] = self
                .ampeg_value_with_cc_and_velocity(param, velocity, probe)
                .0;
        }
        ampeg_run_from(
            values[0], values[1], values[2], values[3], values[4], engine_hz,
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
    /// ⭐ **`set_ccN` 的播种表**（任务单 `#110`）：摊平期从**同一次解析**的
    /// [`yeban_sfz::instrument::Instrument::cc_defaults`] 抄下（见 [`CcSeedTable`]）。
    ///
    /// ⚠ 形状是**定长内联数组**（⛔ 无 `Vec`、⛔ 零额外堆）：它随摊平表按值走，
    /// 快照边界只**读**它；⭐ **空表 ⇒ 播种机制一个字节都不动**（`#110` 的保命不变量）。
    cc_seeds: CcSeedTable,
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
        let (regions, cc_seeds) =
            flatten_regions(instrument, engine_hz, |_region| (0u16, sample_hz))?;
        Ok(Self {
            regions,
            samples,
            broadcast: true,
            silent_regions: 0,
            missing_samples: Vec::new(),
            engine_hz,
            cc_seeds,
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
        let (regions, cc_seeds) = flatten_regions(instrument, engine_hz, |region| {
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
            cc_seeds,
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

    /// ⭐ **`set_ccN` 的播种表**（任务单 `#110`）。
    ///
    /// 它的读者有两个：
    ///
    /// 1. **实时侧的快照边界**（`rt.rs::render_block` 的重新武装分支）：逐条
    ///    [`CcSeedTable::for_each`] ⇒ [`crate::cc_state::ControlChangeState::seed`]
    ///    ⇒ 会话 CC 表 armed ⇒ 探针拿得到初值（零分配、零锁、零 I/O）；
    /// 2. **控制侧**（判据 / 报告）：`cc_seeds().len()` / `out_of_domain()` 是
    ///    "这份文件声明了多少初值、多少条表达不了"的读数。
    ///
    /// ⭐ **空表 ⇒ 引擎一个字节都不播种**（`is_empty()`）：没有 `set_ccN` 的文件
    /// 因此保持 `armed == false`、输出与接线前逐位相同（`#110` 的保命不变量）。
    #[must_use]
    pub const fn cc_seeds(&self) -> &CcSeedTable {
        &self.cc_seeds
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
        self.render_block_into_with_cc(note, velocity, None, out)
    }

    /// ⭐ **回调侧渲染 ＋ CC 探针**（任务单 `#104`）：与 [`Self::render_block_into`]
    /// **同一条路**，只多一个 `ampeg_*` 调制的 CC 状态来源。
    ///
    /// * `cc == None` ⇒ 与 [`Self::render_block_into`] **逐位相同**；
    /// * `cc == Some(probe)` ⇒ 每条区域的 `ampeg_*` 按 `base + Σ delta × curve(cc)`
    ///   求值（[`PreparedRegion::ampeg_run_with_cc`]）。
    ///
    /// ⚠ **与离线 [`SfzInstrument::render_note_into_with_cc`] 的两处登记差别**
    /// （本模块头的"CC 调制"一节有逐字登记）：
    ///
    /// 1. **区域选择不随探针变**：门控（含 `loccN`/`hiccN`）在**摊平期**就用
    ///    `cc = None` 求值过（[`flatten_regions`]）⇒ 探针⛔ 不会让一条被 CC 门控挡掉的
    ///    区域发声；
    /// 2. **曲线点表已在摊平期烘焙** ⇒ 探针不再需要 `Instrument`。
    ///
    /// ⭐ ⭐ **追加修订段（任务单 `#109`）—— 被修订的是上面第 1 条**（⛔ 原文一字未删）：
    /// `loccN`/`hiccN` 的门控**不再**在摊平期按 `cc = None` 求值 —— 掩码只留**静态**门控
    /// （通道 / `trigger` / keyswitch），CC 门控在**每条区域**上由
    /// [`PreparedRegion::cc_gates_ok`] 按**本函数的 `cc` 参数**裁决（见下面第 3 条）。
    /// 第 2 条（曲线烘焙）**整条有效**。
    ///
    /// ⚠ **探针影响的面**（任务单 `#108` 起）：**取值类**的量 —— `ampeg_*` 的取值
    /// **与** CC 轴交叉淡化的增益（[`PreparedRegion::gain_at_with_cc`]）——
    /// ⛔ **不**影响区域选择（那是第 1 条）。
    ///
    /// ⭐ ⭐ **追加修订段（任务单 `#109`）—— 被修订的是上面这一段里的"⛔ 不影响区域选择"**
    /// （⛔ 原文一字未删）：探针现在还影响**区域选择里的唯一一项** —— CC 门控
    /// （`loccN`/`hiccN`）⇒ `cc == Some(probe)` 时，门控不成立的区域**不再**发声
    /// （新增的一步在位掩码 `accepts()` **之后**；⛔ 键 / 力度 / `trigger` / keyswitch
    /// 那几张掩码的口径一个字节没碰）。`cc == None` 时**有**门控的区域仍不发声
    /// （与 `yeban-sfz` 的严格策略逐字同款）⇒ 既有夹具逐位不变。
    ///
    /// ⚠ **零分配、零锁、零 I/O**（只做位测试、定长数组读与 `out` 的原地写入）
    /// ⇒ 该函数仍可被回调路径调用 [MUST-GATE-001]。
    pub fn render_block_into_with_cc(
        &self,
        note: u8,
        velocity: u8,
        cc: Option<&dyn Fn(u8) -> u8>,
        out: &mut [f32],
    ) -> usize {
        let n = note as usize;
        let mut written = 0usize;
        for region in &self.regions {
            if !region.accepts(note, velocity) {
                continue;
            }
            // ⭐ CC 区域门控（任务单 `#109`）：掩码之上**再**判一次 `loccN`/`hiccN`。
            // 掩码里那一条走 [`non_cc_gates_open`]（⛔ 不再按"CC 未提供"清 0）
            // ⇒ 门控的**唯一**求值点在这里（一次性渲染）与触发期（实时）。
            // ⚠ `cc == None` ⇒ 有门控的区域不过（与 `yeban-sfz` 的严格策略逐字同款，
            // 见 [`PreparedRegion::cc_gates_ok`]）⇒ 既有夹具（无门控）逐位不变。
            if !region.cc_gates_ok(cc) {
                continue;
            }
            let Some(sample) = self.sample(region.sample_index) else {
                continue;
            };
            let pcm = sample.pcm();
            // ⭐⭐ `tune` 调制（任务单 `#116`）：本函数的 `cc` 就是这一支输入 ——
            // `rate_per_note[n]` 已含**静态** `tune`，再乘 [`PreparedRegion::tune_ratio_with_cc`]
            // （与实时 `slot_readings`、离线 `render_note_into_with_cc` **同一份**换算）。
            // ⚠ `cc == None` ⇒ 乘子恰 `1.0` ⇒ `rate` 与接线前**逐位相同**。
            // ⚠ 非法速率（乘子上溢/下溢）**沿用下面那条既有守卫**（⛔ 不加第二条谓词）。
            let rate = region.rate_per_note[n] * region.tune_ratio_with_cc(cc);
            // ⭐⭐ `#117`：速率守卫的唯一谓词（三条路同一量、同一谓词）。
            if !rate_is_usable(rate) || pcm.is_empty() {
                continue;
            }
            let Some((start, source_end)) = region.span_bounds(pcm.len()) else {
                continue;
            };
            // ⭐ 增益（任务单 `#108`）：CC 轴交叉淡化按**同一**探针求值
            // （`cc == None` ⇒ `gain_at` 的老口径，逐位不变）。
            let gain = region.gain_at_with_cc(note, velocity, cc);
            // ⭐ 循环窗口（本单）：归约已把模式／退化／越界三条判完（`None` ⇒ 不循环）。
            let loop_window = region.loop_window(pcm.len());
            // ⭐ **播放方向**（任务单 `#113`）：与离线/实时两条路**同一份**归约
            // （[`reduce_travel`]）。⚠ `forward` ⇒ 三项逐位不变（判据 `S33`/`S37`/`S65`）。
            let (mut position, rate, source_end) =
                reduce_travel(region.direction(), rate, start, source_end);
            // 摊平表的一次性调用**不窃取**（与上一条路同款：`× 1.0` 逐位不变）。
            let mut fade = FadeRamp::NONE;
            // ⭐ 包络（任务 `#92`）：与离线/实时两条路**同一份归约**（秒 × 本表的 `engine_hz`）。
            // 这条一次性调用**没有 note-off** ⇒ 只走 起音→保持→衰减→保持（`frame = u64::MAX`）。
            // ⭐ 力度调制（任务单 `#106`）：本函数的 `velocity` 参数就是**这一支输入**
            // （与 CC 探针**并列**，两支取值的相加顺序见 `yeban_sfz::modulation`）。
            let mut env = region
                .ampeg_run_with_cc_and_velocity(self.engine_hz, velocity, cc)
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
///
/// ⭐ **返回的第二项是 `set_ccN` 的播种表**（任务单 `#110`）：它与区域表来自**同一次解析**
/// （`with_instrument` 会重新解析 ⇒ ⛔ 不在这里开第二次），因此"文件里真的写了 `set_ccN`"
/// 与"摊平表里真的带了初值"是同一个事实（判据 S86／S87／S92）。
fn flatten_regions(
    instrument: &SfzInstrument,
    engine_hz: f32,
    mut resolve: impl FnMut(&Region<'_>) -> (u16, f32),
) -> Result<(Vec<PreparedRegion>, CcSeedTable), SfzError> {
    instrument.with_instrument(|parsed| {
        // ⭐ `set_ccN` 的初值：`<control>` 作用域、同名后者覆盖前者（解析层已归约）
        // ⇒ 这里只做"抄进会话 CC 域"与"域外计数"（见 [`CcSeedTable`]）。
        let cc_seeds = CcSeedTable::from_cc_defaults(parsed.cc_defaults());
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
            //
            // ⭐ ⭐ **追加修订段（任务单 `#109`）—— 被修订的是上面这段在 `#104`／`#107`
            // 口径下的"`cc = None`"那一支**（⛔ 上面两句的**原文一字未删**，口径同台账第 278 条：
            // 保留原文 ＋ 追加修订段 ＋ 点名被修订的支）。规范逐字与裁定见模块头的
            // 「⭐ ⭐ **追加修订段（任务单 `#109`）**」一节；这里只说**机械**改了什么：
            //
            // * 掩码的静态门控改走 [`non_cc_gates_open`]（**同序、同缺省值**，
            //   只把 CC 那一段换成 `true`）⇒ 对**没有** CC 门控的 region（语料 64894 条里
            //   43831 条，`#109` 普查）**逐位不变**；对**有** CC 门控的 region，掩码不再被
            //   `loccN`/`hiccN` 清 0（它们从"永不发声"变成"**触发期**按探针裁决"）。
            //   ⚠ ⛔ 不能用"喂一个假探针"绕过：门控是**逐区域**的区间，任何单一常量
            //   探针都只能满足一部分区域（本单实测过：探针 `0` 满足不了 `locc1=64`）。
            // * 门控几何**原样**烘进 [`PreparedRegion::cc_gates`]（定长 `[CcGate; CC_GATE_CAP]`
            //   ＋ 条数）⇒ 触发／渲染那一刻才用 `cc_gates_ok(probe)` 判。
            let gates = &region.cc_gates;
            let (baked_cc_gates, baked_cc_gate_count) = cc_gate_slots(gates);
            let probe = first_set_bit(&key_mask).zip(first_set_bit(&vel_mask));
            let gate_open =
                probe.is_some_and(|(note, velocity)| non_cc_gates_open(region, note, velocity));
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
            // ⭐ 本片（任务单 `#108`）新增：CC 轴段**原样**烘成定长数组（顺序 ＝
            // `read_crossfades` 的归约序）。容量是**结构性上界**（见 `CC_CROSSFADE_CAP`）
            // ⇒ 条数不可能越界，⛔ 不需要 `Err`、⛔ 没有截断分支。
            // ⚠ 下面用**直接下标**（⛔ 不用 `get_mut` 那种"满了就悄悄丢"的写法）：
            // 万一约束被改坏，那是响亮的 panic，⛔ 不是静默丢段。
            let mut cc_crossfades = [INERT_CC_CROSSFADE; CC_CROSSFADE_CAP];
            let mut cc_crossfade_count = 0usize;
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
                    //
                    // ⭐ ⭐ **追加修订段（任务单 `#108`）** —— ⛔ 上面两句的**原文保留**，
                    // 它们**仍然成立**：它们说的是离线那条 `cc = None` 的重载
                    // （`Region::playback_spec`）。⭐ 被修订的是**另一支**：摊平期
                    // **不再把 CC 轴整个丢掉**。理由逐条：
                    //   ① `yeban-sfz` 本来就另有一条**读得到** CC 状态的重载
                    //      （`Region::playback_spec_with` → `Region::crossfade_gain` 读
                    //      `RegionQuery::cc`）—— 语义在规范页上逐字可读，⛔ 不是发明；
                    //   ② 任务单 `#107` 之后，**实时侧已经有 CC 状态**
                    //      （`EngineEvent::ControlChange` → `ControlChangeState` → 探针）
                    //      ⇒ "回调侧没有 CC 状态"这条**理由已经不成立**；
                    //   ③ 于是三条路统一：几何烘进本数组，取值在**逐块**求值时按探针做
                    //      （[`PreparedRegion::gain_at_with_cc`]），离线那条路改用
                    //      `playback_spec_with`。
                    // ⛔ 不修订的：`cc = None` 的一切读数（`gain_at` 传 `None` ⇒
                    // CC 段一段都不乘 ⇒ 与接线前逐位相同，判据 `S33`/`S37`/`S65`）。
                    XfAxis::Cc(_) => {
                        cc_crossfades[cc_crossfade_count] = *crossfade;
                        cc_crossfade_count += 1;
                    }
                }
            }
            debug_assert!(
                cc_crossfade_count <= CC_CROSSFADE_CAP,
                "yeban-sfz 的 cc_in/cc_out 是按 `u8` CC 号索引的映射、且只收 cc ≤ 127 \
                 ⇒ 单区域的 CC 段 ≤ 2 × 128 = {CC_CROSSFADE_CAP}"
            );
            let mut velocity_gain = [0.0f32; 128];
            for (velocity, slot) in velocity_gain.iter_mut().enumerate() {
                *slot = region.velocity_gain(velocity as u8);
            }

            // ⭐ CC 调制（任务单 `#104`）：把解析层的 `(cc, delta, 曲线下标)` 烘成
            // **定长 `Copy`** 形状。曲线点表只能在这里解析 —— 摊平之后就拿不到
            // `<curve>` 段了（这是本片把曲线**烘进**区域表、而不是另开一张表的原因：
            // `SfzByteUsage` 的"区域表 ＝ `size_of × N`"读数因此仍然**没有隐藏的第四份开销**）。
            // 条数超过 `AMPEG_MOD_CAP` 在 `yeban-sfz` 的**解析期**就已经响亮 `Err`
            //（`SfzError::TooManyAmpegModulations`）⇒ 这里⛔ 不会截断。
            let baked = bake_cc_mods(region.cc_modulations(), |index| {
                parsed.modulation_curve(index)
            });
            debug_assert!(
                baked.len() <= AMPEG_MOD_CAP,
                "yeban-sfz caps at AMPEG_MOD_CAP"
            );
            let cc_mod_count = baked.len() as u8;
            let mut cc_mods = [BakedCcMod::EMPTY; AMPEG_MOD_CAP];
            for (slot, item) in cc_mods.iter_mut().zip(baked) {
                *slot = item;
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
                // ⭐ CC 调制（任务单 `#104`；⭐ `#115` 起目标由 `ModTarget` 区分）：
                // 归约结果（含烘好的曲线）搬进摊平表。
                // 它是**区域参数**、不是声部状态 ⇒ `SfzVoice` 尺寸不变，⛔ 不进实时对象。
                cc_mods,
                cc_mod_count,
                // ⭐ 振幅基准（任务单 `#115`）：`amplitude` 的**线性百分比**原样搬进
                // 摊平表（缺省 100）。它是**区域参数**（CC 逐块从探针读）⇒ `SfzVoice`
                // 尺寸不变，⛔ 不进实时对象。求值点见 `amplitude_gain_with_cc`。
                amplitude: region.amplitude,
                // ⭐ 力度调制（任务单 `#106`）：五个**每参数**的力度调制量原样搬进
                // 摊平表（`[Option<f32>; 5]`：`None` ＝ 该 opcode 没出现 ⇒ 求值期
                // 一项算术都不做）。同样是**区域参数**、⛔ 不是声部状态
                // （力度在 `SfzVoice` 里早就有）⇒ ⛔ 不进实时对象。
                ampeg_vel2: region.ampeg_vel2,
                // ⭐ CC 轴交叉淡化（任务单 `#108`）：归约序的几何 ＋ 有效条数。
                // 它是**区域参数**、不是声部状态 ⇒ `SfzVoice` 尺寸不变，⛔ 不进实时对象。
                cc_crossfades,
                cc_crossfade_count: cc_crossfade_count as u16,
                // ⭐ CC 区域门控（任务单 `#109`）：`loccN`/`hiccN` 的闭区间**原样**搬进
                // 摊平表（定长 `[CcGate; CC_GATE_CAP]` ＋ 条数）。它是**区域参数**、
                // ⛔ 不是声部状态（CC 值逐块变、几何是静态的）⇒ `SfzVoice` 尺寸不变，
                // ⛔ 不进实时对象。求值点在**触发期**（`cc_gates_ok(probe)`）。
                cc_gates: baked_cc_gates,
                cc_gate_count: baked_cc_gate_count,
                // ⭐ 独占组（任务单 `#94`）：三个**每区域**字段原样搬进摊平表。
                // `group` / `off_by` 的解析与域（`0..=u32::MAX`）在 `yeban-sfz` 里早已有
                // 判据（`instrument.rs` 的 `read_u32` 一族）⇒ 本片只**搬运**，⛔ 不新增解析。
                group: region.group,
                off_by: region.off_by,
                off_mode: region.off_mode,
                // ⭐ 同音复音限制（任务单 `#100`）：`note_polyphony` ＋ `note_selfmask`
                // 归约出的**一份** `Copy` 读数原样搬进摊平表（`yeban-sfz` 早已解析并裁决：
                // `limit = 0` ⇒ 不限制、`self_mask` 缺省 `on`）⇒ 本片只**搬运**，
                // ⛔ 不新增解析、⛔ 不改 `yeban-sfz` 的既有判定规则。
                note_polyphony: region.note_polyphony,
                // ⭐ 播放方向（任务单 `#113`）：解析层早已裁决好的 `Copy` 读数**原样**
                // 搬进摊平表（`Region::direction`：`forward` 缺省 / `reverse`；
                // 白名单在 `yeban-sfz` 的 `PlayDirection::OPTIONS`）。
                // 它是**区域参数**、不是声部状态 ⇒ `SfzVoice` 尺寸不变，⛔ 不进实时对象。
                direction: region.direction,
            });
        }
        (regions, cc_seeds)
    })
}

/// ⭐ **摊平期掩码用的"非 CC 门控"谓词**（任务单 `#109`）：它只判**静态**门控
/// （通道 / `trigger` 事件 ＋ `held_notes` / keyswitch），**⛔ 完全不看 CC 门控**。
///
/// # 为什么必须这样（而不是"把 CC 从 `matches_query` 里删掉"）
///
/// 摊平期的掩码承载的是**静态**门控 —— 那些与 `(note, velocity)` 无关、也与会话
/// 运行态无关，因此可以在构造期烧死。CC 门控**不是**静态的：它要等**触发期**拿到
/// 会话运行态的 CC 探针才能裁决（裁定见模块头的「⭐ ⭐ **追加修订段（任务单 `#109`）**」）。
///
/// ⚠ **⛔ 不能靠"喂一个假探针"绕过**（本单试过、实测红）：CC 门控是一条**逐区域**的
/// 区间 `[lo, hi]`，**任何单一常量探针**都只能满足一部分区域 —— 喂 `0` 满足不了
/// `locc1=64`（⇒ 掩码照旧被清 0），喂 `127` 满足不了 `hicc1=63`。所以这里**不碰**
/// `RegionQuery::cc`，而是把非 CC 的那几项**照 `region_matches` 的次序**判一遍。
///
/// # 与 `yeban-sfz::region_matches` 的关系（⛔ 不是第二份口径）
///
/// 逐项与 `region_matches`（`crates/yeban-sfz/src/instrument.rs`）**同源**：
/// `matches_key` / `matches_velocity` / `matches_channel` 是解析层的公开谓词；
/// `trigger.responds_to(NoteOn)` / `trigger.held_notes_ok(None)` 与
/// `keyswitch_ok(None, down)` 也是 —— ⭐ **本函数只把 CC 那一段换成 `true`**，
/// 其余部分的调用序、缺省值（`channel = 1`、`event = NoteOn`、`held_notes` /
/// `last_keyswitch` / `keys_down` 全 `None`）与 `RegionQuery::new(note, velocity)`
/// **逐字段相同** ⇒ 对**没有** CC 门控的区域，掩码**逐位不变**。
///
/// ⚠ 它**只**用于构造期的掩码（摊平期一次）；"这条区域此刻该不该响"的**运行时**判据
/// 是 [`PreparedRegion::cc_gates_ok`]，⛔ 不是本函数。
#[inline]
fn non_cc_gates_open(region: &Region<'_>, note: u8, velocity: u8) -> bool {
    region.matches_key(note)
        && region.matches_velocity(velocity)
        && region.matches_channel(1)
        && region
            .trigger
            .responds_to(yeban_sfz::instrument::TriggerEvent::NoteOn)
        && region.trigger.held_notes_ok(None)
        && {
            // keyswitch：与 `region_matches` 的**同一串**判定
            // （`sw_last` 无 `sw_default` ⇒ 严格策略不过；`sw_down`/`sw_up` 无探针 ⇒ 不过）。
            // ⚠ 两支的结论**同值**（都不过）⇒ 合成一条 `||`（⛔ 不是两条 `if`：clippy 的
            // `if_same_then_else` 正确地点出了那条冗余；语义一个字没变）。
            if !region.has_keyswitch_gate() {
                true
            } else if (region.sw_last.is_some() && region.sw_default.is_none())
                || region.sw_down.is_some()
                || region.sw_up.is_some()
            {
                false
            } else {
                region.keyswitch_ok(None, |_| false)
            }
        }
}

/// ⭐ 把区域归约好的 `loccN`/`hiccN` 门控搬进摊平表的**定长**槽位（任务单 `#109`）。
///
/// 返回 `([CcGate; CC_GATE_CAP], u8)`：前者是惰性填充的定长数组（只有前 `n` 个有效），
/// 后者是有效条数。**零分配**（⛔ 无 `Vec`、⛔ 无堆）。
///
/// ⚠ 条数上界是**结构性**的（[`CC_GATE_CAP`]）：`yeban-sfz` 把门控归约进
/// `BTreeMap<u8, _>` 且 `parse_cc_gate_name` 拒绝 `cc > 127` ⇒ `n ≤ 128`。
/// 越界在结构上不可达，因此这里是**直接下标**（⛔ 不截断、⛔ 不静默丢门控）——
/// 万一约束被改坏，那是响亮的 panic。
fn cc_gate_slots(gates: &[CcGate]) -> ([CcGate; CC_GATE_CAP], u8) {
    let mut slots = [INERT_CC_GATE; CC_GATE_CAP];
    for (slot, gate) in slots.iter_mut().zip(gates) {
        *slot = *gate;
    }
    let count = gates.len();
    debug_assert!(
        count <= CC_GATE_CAP,
        "yeban-sfz 的门控按 `u8` CC 号去重、且只收 cc ≤ 127 \
         ⇒ 单区域门控 ≤ {CC_GATE_CAP}"
    );
    (slots, count as u8)
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
/// ⭐ **钳位腿例外**（任务单 `#97`）：窗口终点恰好等于 `pcm_len` 时"下一格"由钳位给定
/// ⇒ 条件放宽成 `floor(position) >= source_end`（见下面"钳位"一节）。
/// `source_end` 由调用方给出（⭐ 三条路都是 [`reduce_span`] 的归约结果）：
/// * 摊平表的一次性调用给 [`PreparedRegion::span_bounds`] 的终点（`span` 已应用）；
/// * 实时声部给同一个 [`PreparedRegion::span_bounds`]（触发期抄下）；
/// * 离线 [`SfzInstrument::render_note_into`] 给同一份归约。
///
/// ⭐ ⭐ **追加修订段（任务单 `#113`）—— 被修订的是上面这一段"停止条件"里
/// `source_end` 的**语义**那一支**（⛔ 原文一字未删）：
/// `rate < 0.0`（反向，[`reduce_travel`] 的产物）时，同一个参数位承载的是**下界**
/// 而不是上界，闸门也随之换成"位置是否已经走到下界之下"：
///
/// | `rate` 符号 | `source_end` 的含义 | 逐样本闸门（写之前） |
/// | :--- | :--- | :--- |
/// | `> 0`（`forward`） | 上界（半开） | `floor(position) + 1 >= source_end`（钳位腿：`floor(position) >= source_end`） |
/// | `< 0`（`reverse`） | **下界**（闭） | ⭐ `position <= source_end`（**浮点**比较：整数闸门会漏掉 `low + 0.5` 那一帧） |
///
/// ⭐ **反向为什么是 `<=` 而不是 `<`**：参照系是"把区间样本倒序后按 `forward` 渲染"
/// （[`reduce_travel`] 的等价性证明）—— 倒序数组长 `L = end − start`，`forward` 在
/// `floor(t) + 1 >= L` 时停 ⇒ 换成源位置就是 `position <= start`（判据 `S95` 逐帧对账）。
/// ⭐ **反向的回绕也是镜向的**：`forward` 在 `position >= loop_start + loop_len` 时折回；
/// 反向在 `position < loop_start` 时折回 `loop_start + loop_len − (loop_start − position) % loop_len`
/// （同一个 `fmod` 形态的两侧，⛔ 不是"减一次"那种会留下死循环风险的第二套几何）。
/// ⭐ **反向的钳位恒开**：反向读的最高一格可以恰好是 PCM 最后一格
/// （区间取到样本尾时 `position = pcm_len − 1` ⇒ `index + 1` 越界）⇒
/// `next` 恒钳到最后一个可读样本（`forward` 那一侧的触发条件**一字未动**）。
/// ⚠ 三条既有性质在 `rate > 0` 时**逐位不变**（判据 `S33`/`S37`/`S48`/`S52`）：
/// `reverse` 判别是一次 `f32` 比较，`bound_f` 是循环**外**的一次转换，
/// `tail_clamp` 的表达式对 `forward` 与接线前**同一份**。
///
/// ⚠ **给了循环窗口时这一条不可达**：回绕把位置压在 `[loop_start, loop_end)` 内，
/// 而窗口已由 [`reduce_loop_window`] 保证 `loop_end <= pcm_len` 且
/// `loop_end <= source_end`（⭐ `== pcm_len` 那一档靠上面的钳位可读）⇒ 声部由**外部**
/// 终止（`end_sample`）或继续循环，⛔ 不再由"样本播完"退休。
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
///
/// ## ⭐ 钳位（任务单 `#97`：`loop_end == pcm_len` 的窗口**也循环**）
///
/// 窗口终点**恰好等于** `pcm_len` 时（文件用 `loop_end = 总帧数` 表达"循环到结尾"）
/// 位置天然被封在 `[loop_start, pcm_len)`，而线性插值要读 `index + 1` ——
/// 最后一格上那一格**落在 PCM 之外**。本函数的处置是**钳到最后一个样本**
/// （`next = min(index + 1, pcm_len - 1)`），⛔ **不是**把窗口判成"不循环"：
///
/// * **钳到最后一格**（而不是 `0`、也不是回绕到 `loop_start`）：它是**唯一**与
///   `loop_end < pcm_len` 那条既有路同形的最小延伸 —— 那条路在窗口末帧读的是
///   `pcm[loop_end]`（窗口**外**紧邻的那一格，⭐ 本单⛔ 不改），而 `loop_end == pcm_len`
///   时"紧邻的那一格"**根本不存在** ⇒ 只能停在最后一个可读样本上（"保持"语义）。
///   ⚠ 回绕到 `loop_start` 是**另一条**语义（把插值也卷进循环缝），那会与
///   `loop_end == pcm_len - 1` 的既有读数分叉 ⇒ ⛔ 本单不取；
/// * **有界**：`next` 恒在 `[0, pcm_len)`，样本是 `pcm[index]` 与 `pcm[next]` 的凸组合
///   ⇒ 幅度不超过窗口内两个样本的包络（⛔ 不产生爆音），且**一次越界读都没有**；
/// * **逐位不变**：`rate == 1.0` 且窗口为整数时 `frac == 0` ⇒ 读的是 `pcm[index] * 1.0`
///   ＋ `pcm[index] * 0.0` ＝ `pcm[index]`，⭐ 与"多读一格"无关；
/// * ⭐ **代价**：钳位腿的**逐样本**增量只有一次 `.min()` 与一次"是不是钳位腿"的
///   `Option` 判别（`pcm.len()` 那一侧的判断在**函数开头**算一次）
///   ⇒ 纯算术、⛔ 零分配、⛔ 零锁、⛔ 零 I/O [MUST-GATE-001]。
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
    // ⭐ **方向判别**（任务单 `#113`）：唯一的判据就是 `rate` 的符号
    // （[`reduce_travel`] 把 `direction=reverse` 折成"起点 = 区间末尾 ＋ 步进取负"）。
    // ⚠ 这是**一次** `f32` 比较（循环外）⇒ `rate > 0` 那条既有的路一个字节不变。
    let reverse = rate < 0.0;
    // ⭐ 反向的闸门要用**浮点**比（整数下标会漏掉 `low + 0.5` 那一帧）⇒ 循环外转一次。
    // ⚠ `source_end` 在 `reverse` 时是**下界**（见本函数文档的追加修订段）。
    let travel_bound = source_end as f32;
    // ⭐ **钳位腿**（任务单 `#97`）：窗口终点**恰好等于** PCM 长度时，回绕点上要读的
    // `index + 1` 落在 PCM 之外 ⇒ 把"插值要读的下一格"钳到**最后一格**
    // （`Some(last)` ⇒ 本声部走钳位腿；`pcm` 为空时 `checked_sub` ⇒ `None`）。
    // ⚠ **循环外**预算：逐样本路径只做一次 `.min()`，⛔ 不做 `pcm.len()` 比较。
    // ⭐ ⭐ **追加修订段（任务单 `#113`）**：反向**恒**走钳位腿 —— 反向从区间高端起步，
    // 最高一格可以恰好是 `pcm_len − 1` ⇒ `index + 1` 越界；`forward` 那一侧的触发条件
    // 逐字未变（表达式仍是同一个 `loop_window.filter(...)`）。
    let tail_clamp = if reverse {
        pcm.len().checked_sub(1)
    } else {
        loop_window
            .filter(|window| window.end as usize == pcm.len())
            .and_then(|_| pcm.len().checked_sub(1))
    };
    let mut written = 0usize;
    for slot in out.iter_mut() {
        // ⭐ 逐样本的引擎**绝对**帧（包络的释放进度 `k = abs − epoch` 靠它算；
        // ⛔ 不用"本块内的下标"：那会让跨块的释放段重新计时）。
        let abs_frame = env.as_ref().map_or(0, |run| run.abs_base + written as u64);
        // ⭐ **回绕闸门**（任务单 `#94` 按规范改）：有包络时，只有
        // [`EnvRun::may_wrap`] 允许才回绕 ⇒ 释放期里 `loop_continuous` 照旧循环、
        // `no_loop` / `loop_sustain` 沿源位置往后播到样本尾。
        // ⚠ 没有包络（`env == None`）⇒ **恒允许**（＝ 接线前的行为，逐位不变）。
        // ⭐ 任务单 `#113`：闸门**同一处**、几何**镜向** —— 正向越过窗口上侧折回、
        // 反向越过窗口下侧折回（两者都是同一个 `%` 余数，⛔ 不是第二套循环几何）；
        // ⚠ 正向那一支的**短路次序逐字未变**（⛔ 不在每个样本上多求一次 `may_wrap`）。
        if let Some((loop_start, loop_len)) = loop_bounds {
            if reverse {
                if *position < loop_start && env.as_ref().is_none_or(|run| run.may_wrap(abs_frame))
                {
                    *position = loop_start + loop_len - (loop_start - *position) % loop_len;
                }
            } else if *position >= loop_start + loop_len
                && env.as_ref().is_none_or(|run| run.may_wrap(abs_frame))
            {
                *position = loop_start + (*position - loop_start) % loop_len;
            }
        }
        let index = *position as usize;
        // ⭐ **钳位**（任务单 `#97`）：钳位腿上 `index + 1` 那一格已经不在 PCM 里
        // ⇒ 换成最后一格（⛔ 越界读、⛔ panic）；其余两条路**逐字**是原来的 `index + 1`。
        // ⭐ ⭐ **追加修订段（任务单 `#117`）—— 被修订的是上面这句里
        // 「其余两条路**逐字**是原来的 `index + 1`」那一支**（⛔ 原文一字未删，
        // 口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋ 点名被修订的支）：
        // `#116` 起 `rate` 可以收到一个**有限但极大**的 `tune` 乘子（`2^x` 的
        // 有限上界 ≈ `f32::MAX`），`position` 于是可以超过 `usize::MAX`；Rust 的
        // `f32 as usize` 是**饱和**转换 ⇒ `index == usize::MAX`，而 `index + 1`
        // 在 debug 下**溢出 panic**、在 release 下回绕成 `0` 并绕过下面那条
        // "越界即退休"的检查。改成 `saturating_add(1)`：正常值下与 `+ 1`
        // **逐位相同**，只有"位置已在 PCM 之外"这一档从 panic 变成**如实退休**
        // （`next >= pcm.len()` 那条既有闸门接住它，⛔ 不加第二条语义）。
        let next = match tail_clamp {
            Some(last) => index.saturating_add(1).min(last),
            None => index.saturating_add(1),
        };
        // 停止条件（写每个样本**之前**）：默认 `floor(position) + 1 >= source_end`
        // —— 线性插值需要 `position` 与 `position + 1` 两个样本。
        // ⭐ 钳位腿上"下一格"已被钳到最后一格 ⇒ 只要求 `index` 本身仍在源区间内
        // （⭐ 于是 `loop_end == pcm_len` 的窗口能在最后一格上照常回绕，⛔ 不再被当退休）。
        // ⭐ 任务单 `#113`：反向的闸门是**下界**（`position <= bound`）——
        // 理由与等价性证明见本函数文档的追加修订段。
        let past_source_end = if reverse {
            *position <= travel_bound
        } else if tail_clamp.is_some() {
            index as u64 >= source_end
        } else {
            // ⭐ ⭐ `#117`（同上一条修订段）：`index as u64 + 1` 在
            // `index == usize::MAX` 时同样溢出 ⇒ `saturating_add(1)`
            // （正常值下与 `+ 1` 逐位相同）。
            (index as u64).saturating_add(1) >= source_end
        };
        if past_source_end || next >= pcm.len() {
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
        // ⚠ `next`（⭐ 钳位腿上是最后一格）⛔ 不是 `index + 1`：越界读一次都不允许。
        let sample = pcm[index] * (1.0 - frac) + pcm[next] * frac;
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
    /// ⭐ **引擎层的轨增益门**（`track_gain` ＝ 可闻门 × 推子；触发期抄下，此后不变）。
    ///
    /// ⚠ **沿革（任务单 `#108`）**：本字段**原来**存的是"**乐器层** `region.gain_at(..)`
    /// × 引擎层 `track_gain`"的乘积（触发期算好、此后不变）。CC 轴交叉淡化
    /// （`xfin_*ccN` / `xfout_*ccN`）**逐块**可变 ⇒ 那份乘积**不够**（CC 一动就得重算）。
    /// 本片改成只存**引擎层**那一份，乐器层由 [`SfzVoicePool::slot_readings`] 逐块用
    /// [`PreparedRegion::gain_at_with_cc`] 现算（`note` / `velocity` / `region_index`
    /// 早就在声部里）⇒ 两条腿仍**相乘、⛔ 不重复**（归属裁决不变，见台账 `R711`）。
    /// ⭐ 尺寸 **＋0 字节**：仍是**一个** `f32`（判据 `S9` 钉住 40 B）。
    track_gain: f32,
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
    /// **行进边界**：见类型文档的"终点口径"。
    ///
    /// ⭐ ⭐ **追加修订段（任务单 `#113`）**：本格承载的是**行进方向上的那一端**
    /// —— `rate > 0`（`forward`）时是**源采样终点**（半开上界，＝ 接线前的语义，⛔ 未变）；
    /// `rate < 0`（`reverse`）时是**源采样下界**（闭）。哪一端由 [`Self::rate`] 的符号
    /// 唯一决定（[`reduce_travel`] 是三条路共用的唯一归约）⇒ ⛔ 不新增字段。
    source_end: u64,
    /// **引擎侧终点**（绝对样本位置）：来自 [`crate::synth::ScheduledNote::end_sample`]。
    end_sample: u64,
}

impl SfzVoice {
    /// 空闲声部（哨兵；`active == false` ⇒ 不参与渲染）。
    pub const EMPTY: Self = Self {
        position: 0.0,
        rate: 0.0,
        track_gain: 0.0,
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
    /// ⭐ ⭐ **追加修订段（任务单 `#113`）—— 被修订的是上面那句里"`source_start` / `source_end`
    /// 就是归约出的起点 / 终点"那一支**（⛔ 原文一字未删）：调用方现在**先**过一遍
    /// [`reduce_travel`] ⇒ 三个入参的语义变成"**行进**起点 / **带符号**步进 / **行进**边界"：
    ///
    /// | 方向 | `source_start`（→ [`Self::position`]） | `rate` | `source_end`（→ [`Self::source_end`]） |
    /// | :--- | :--- | :--- | :--- |
    /// | `forward` | 区间起点 | `> 0` | 区间**上**界（半开） |
    /// | `reverse` | 区间**末**格 `end − 1` | `< 0` | 区间**下**界（闭） |
    ///
    /// ⚠ **零新字段**：三件读数**全部**落在本构造器**既有**的三个参数/字段上
    /// （`position` / `rate` / `source_end`）⇒ `SfzVoice` 仍 **40 B**（判据 `S9`）。
    /// ⚠ `direction=forward` 时三个入参与接线前**逐字相同** ⇒ 既有夹具逐位不变。
    ///
    /// ⚠ 包络状态**不在参数里**：它从 [`ENV_STAGE_IDLE`] ＋ 计数 0 起（区域有包络时由
    /// 逐样本路径第一帧提升为 [`AmpegStage::Attack`]）⇒ ⛔ 本构造器的签名与接线前一致。
    ///
    /// ⚠ `track_gain`（任务单 `#108` 的改名）：它只是**引擎层**那一份增益门，
    /// ⛔ **不再**是"乐器层 × 引擎层"的乘积（见 [`Self::track_gain`] 的沿革段）。
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        note: u8,
        velocity: u8,
        rate: f32,
        track_gain: f32,
        sample_index: u16,
        region_index: u16,
        source_start: f32,
        source_end: u64,
        end_sample: u64,
    ) -> Self {
        Self {
            position: source_start,
            rate,
            track_gain,
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

    /// ⭐ **该声部的引擎层轨增益门**（`track_gain`；任务单 `#108` 起 ⛔ 不含乐器层增益）。
    ///
    /// ⚠ 它是**只读副本**：乐器层那一份由逐块路径现算（[`PreparedRegion::gain_at_with_cc`]）
    /// ⇒ 判据可以断言"声部里存的确实是引擎层那一份"（⛔ 不是另一个乘积）。
    #[must_use]
    pub const fn track_gain(&self) -> f32 {
        self.track_gain
    }

    /// 当前源采样位置（帧）。
    #[must_use]
    pub const fn source_position(&self) -> f32 {
        self.position
    }

    /// ⭐⭐ **该声部的基准步进**（`region.rate_per_note[note]` 的触发期副本；反向时为**负**）。
    ///
    /// ⚠ `tune` 的**调制量**（任务单 `#116`）⛔ 不在本读数里：基准 `tune` 早在摊平期
    /// 就烘进 `rate_per_note`，而调制量**逐块**可变 ⇒ 逐块路径用
    /// [`PreparedRegion::tune_ratio_with_cc`] 现算乘子再乘到本读数上
    /// （⇒ ⛔ 没有第二个逐声部字段，`SfzVoice` 仍 **40 B**）。
    /// ⚠ 符号是**方向**（`< 0` ⇒ `reverse`，见 [`Self::source_end`] 的追加修订段）。
    #[must_use]
    pub const fn rate(&self) -> f32 {
        self.rate
    }

    /// 源采样终点（半开）。
    ///
    /// ⭐ **追加修订段（任务单 `#113`）**：`rate < 0`（反向）的声部里它承载的是**下界**
    /// （见 `source_end` 字段的追加修订段）⇒ 读它之前先看 `source_position()` 在往哪边走。
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

/// ⭐ **逐块路径的四个区域读数**（[`SfzVoicePool::slot_readings`] 的产物）。
///
/// 全部是**每块一次**求得的量（⛔ 不进逐样本路径、⛔ 不含逐声部状态）。
#[derive(Clone, Copy, Debug, PartialEq)]
struct SlotReadings {
    /// 有效循环窗口（[`PreparedRegion::loop_window`] 的归约结果）。
    loop_window: Option<LoopWindow>,
    /// ⭐ **本块的逐样本增益**（任务单 `#108`）＝ **乐器层**
    /// （[`PreparedRegion::gain_at_with_cc`]：`volume`／力度／键轴／力度轴／**CC 轴**）
    /// **×** **引擎层**（[`SfzVoice::track_gain`]，触发期抄下）。
    ///
    /// ⚠ **为什么每块现算**：CC 轴交叉淡化按**当前** CC 取值（CC 逐块可变）⇒
    /// 触发期的乘积**不够**。⚠ `cc == None` 时它与接线前的 `voice.gain`
    /// **逐位相同**（同一条 `gain_at` 求值、同一个 `track_gain` 因子）。
    gain: f32,
    /// ⭐⭐ **本块的逐样本步进**（任务单 `#116`）＝ `voice.rate()`（触发期抄下的**基准**
    /// 速率，含静态 `tune`；反向时为负）**×** [`PreparedRegion::tune_ratio_with_cc`]。
    ///
    /// ⚠ **为什么每块现算**：`tune_*ccN` 的音分增量按**当前** CC 取值（CC 逐块可变）
    /// ⇒ 触发期抄下的 `voice.rate()` **不够**。
    /// ⚠ `cc == None`（或表里没有 `tune` 项）⇒ 乘子**恰 `1.0`** ⇒ 本读数与接线前的
    /// `voice.rate` **逐位相同**（同一个乘法、同一个 `× 1.0` 恒等）。
    /// ⚠ **符号**：乘子恒 `> 0` ⇒ 方向（`rate < 0` ⇒ `reverse`）仍由 `voice.rate()`
    /// 唯一决定，⛔ 本字段不改变 [`mix_span_into`] 的方向判别。
    /// ⚠ **没有新的逐声部字段**：率值活在**栈上**的本结构里，声部里仍只有
    /// `SfzVoice::rate`（基准）⇒ `SfzVoice` 仍 **40 B**（判据 `S9`／`S100`／`S106`）。
    rate: f32,
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
    /// 声部增益 ＝ **乐器层**（`region.gain_at_with_cc`：`volume_gain`／
    /// `velocity_gain[128]`／`key_crossfade`／`velocity_fade_*`／**CC 轴交叉淡化**／
    /// `amp_veltrack`）× **引擎层**（本参数：可闻门 × 推子，与鼓机/复音拿到的那一份
    /// **同一个** `track_gain_scale` 值）。
    /// ⇒ ⭐ 两者**相乘、⛔ 不重复**：引擎层⛔ 不碰力度（那由乐器层拥有）。
    ///
    /// ⚠ 任务单 `#108` 起，**触发期只存引擎层那一份**（[`SfzVoice::track_gain`]）：
    /// 乐器层由 [`Self::slot_readings`] 逐块现算（CC 逐块可变 ⇒ 触发期烘死不成立）。
    /// ⇒ 乘积的两条腿**没有变**（⛔ 不是第二份口径），变的只是求值时机。
    ///
    /// # ⭐ CC 区域门控（`loccN`/`hiccN`；任务单 `#109`）
    ///
    /// `cc` 是**会话运行态**的 CC 探针（与 [`Self::render_into_with_cc`] 的**同一个**
    /// 形状：`crate::rt` 在量子边界把 [`crate::cc_state::ControlChangeState`] 借出来）。
    /// 每一条区域在 `accepts(note, velocity)` **之后**还要过 [`PreparedRegion::cc_gates_ok`]：
    ///
    /// * `cc == None`（整张表一条 CC 都**没收到过**）⇒ **有**门控的区域不建声部
    ///   （与 `yeban-sfz` 的严格策略逐字同款；⛔ 不替调用方发明规范缺省）；
    /// * `cc == Some(probe)` ⇒ 每条门控要求 `lo ≤ probe(cc号) ≤ hi`（闭区间）。
    ///
    /// ⚠ **为什么求值点在触发期**（⛔ 不是逐块）：规范逐字（<https://sfzformat.com/opcodes/loccN/>）
    /// "The region will play only if last MIDI controller 74 received was in the 30 to 100
    /// range." ＋ "This is a 'hard' switch - if a region is within the locc to hicc range it
    /// plays, if it's outside that range it does not play." ⇒ 它决定的是
    /// **"这个音符要不要这条区域"**（与 keyswitch／`trigger` 同类）⛔ 不是逐块调制
    /// （那是 `xfin_*`／`xfout_*` 与 `_onccN` 家族）。裁定与依据见模块头。
    ///
    /// **零分配、零锁、零 I/O**：每条区域 ≤ [`CC_GATE_CAP`] 条定长数组读 ＋ 每条一次
    /// 整数区间测试（触发期一次，⛔ 不进逐样本路径）。
    pub fn trigger_note(
        &mut self,
        plan: &PreparedSfz,
        note: u8,
        velocity: u8,
        end_sample: u64,
        track_gain: f32,
        cc: Option<&dyn Fn(u8) -> u8>,
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
            // ⭐ CC 区域门控（任务单 `#109`）：掩码之上的**第二道**、也是**唯一按探针**的
            // 区域选择判据。位置在 `accepts()` 之后、`off_by` 关断**之前** ——
            // 理由：被 CC 门控挡掉的区域**不发声**，因此它⛔ 不该去关掉别人
            // （与"`end=-1` 的静音区域**照旧**关断别人"那条**不冲突**：那条区域是
            // **CC 门控通过**的，只是没有采样输出 ⇒ 它仍然走到下面的关断扫描）。
            if !region.cc_gates_ok(cc) {
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
            // ⭐⭐ `#117`：速率守卫的唯一谓词（三条路同一量、同一谓词）。
            if !rate_is_usable(rate) || pcm_len == 0 {
                continue;
            }
            // ⭐ 起点/终点口径：同一份归约（`span() == None` / 空区间 ⇒ 不建声部）。
            let Some((source_start, source_end)) = region.span_bounds(pcm_len) else {
                continue;
            };
            // ⭐ **播放方向**（任务单 `#113`）：与离线/摊平两条路**同一份**归约
            // （[`reduce_travel`]）⇒ 三件读数（起点 / 带符号步进 / 行进边界）**全部**
            // 落在 [`SfzVoice::new`] 已有的三个参数位上 ⇒ ⛔ 零新逐声部字段（`S9` 仍 40 B）。
            // ⚠ `direction=forward` ⇒ 三项**逐位不变**（判据 `S37`/`S52`/`S65`）。
            let (source_start, rate, source_end) =
                reduce_travel(region.direction(), rate, source_start, source_end);
            // ⭐ 引擎层的轨增益门**乘在乐器层增益之外**（两个归属各一份 ⇒ ⛔ 不重复）。
            // ⚠ 任务单 `#108`：这里**只**存引擎层那一份（`track_gain`）—— 乐器层
            // （含 CC 轴交叉淡化）由逐块路径现算（CC 逐块可变，触发期烘死不成立）。
            let voice = SfzVoice::new(
                note,
                velocity,
                rate,
                track_gain,
                region.sample_index,
                region_index,
                source_start,
                source_end,
                end_sample,
            );
            // ⭐ `note_polyphony`（任务单 `#100`）：在建**自己**的声部之前，把同键
            // （`group` × 音高）的在场声部数降到上限以下 —— 语义、规范逐字、工程裁决
            // 与"⛔ 不是 off_by、⛔ 不是窃取"的分工见 [`Self::enforce_note_polyphony`]。
            // ⚠ 次序：它在**新声部入池之前** ⇒ 新声部天然不可能被自己这次触发掐掉
            // （规范："A new note will always play."）。
            self.enforce_note_polyphony(
                plan,
                note,
                velocity,
                region.group(),
                region.note_polyphony(),
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

    /// ⭐ **`note_polyphony` 上限**（任务单 `#100`）：把**同键**（`group` × 音高）的
    /// 在场声部数降到 `policy.limit` 以下，为**即将**建的那个新声部腾位。
    ///
    /// # 语义（规范逐字，⭐ 全部有出处）
    ///
    /// * **键 ＝ polyphony group × 音高**（<https://sfzformat.com/opcodes/note_polyphony/>）：
    ///   "Polyphony limit for playing the same note repeatedly." ＋
    ///   "The note polyphony is checked within a polyphony group, set by the `group` or
    ///   `polyphony_group` opcodes. If no group is specified on the region (or its group,
    ///   master or globally) the note polyphony applies to the default group as if
    ///   `group=0` was specified." ⇒ 只有"`group` 相同（取区域**自己**的
    ///   [`PreparedRegion::group`]，缺省 `0`）**且**音高相同"的在场声部才互相计入。
    ///   语料佐证：`virtuosity-drums` 正是规范下一句说的用法
    ///   （"drums with separate microphone samples, will usually need to set a separate
    ///   group number for each microphone position, so the note polyphony limit is tracked
    ///   separately for each mic."）—— 每个麦克风一个 `group=501`／`502`／…
    /// * **掐谁** ＝ `note_selfmask` 的力度规则
    ///   （<https://sfzformat.com/opcodes/note_selfmask/>）：缺省（`on`）是
    ///   "higher-or-equal-velocity notes turn off lower-velocity notes, but lower-velocity
    ///   notes do not turn off higher-velocity notes."；显式 `off` 时
    ///   "notes turn off notes with the same pitch regardless of velocity"。
    ///   资格判定**复用** `yeban-sfz` 既有的 [`NotePolyphony::masks`]（⛔ 不新写一份）。
    /// * ⭐ **新音符永远发声**：规范原句 "A new note will always play." ⇒ 本函数**不**决定
    ///   新音符的去留（新声部由调用方在扫描**之后**才入池），⛔ 也不因"没掐成"而拒绝它。
    /// * ⭐ **上限是 hint、⛔ 不是硬上限**：规范原句 "The `note_polyphony` opcode is thus
    ///   not a strict polyphony limit but more of a hint for the instrument behavior." ⇒
    ///   没有**有资格**的在场声部时就停下，在场数可以**超过** `limit`。
    /// * `limit == 0`（[`NotePolyphony::is_unlimited`]）⇒ ⛔ 一个都不掐（与"未给出"同义，
    ///   登记语料里 `note_polyphony=0` 出现 26 次、用法是复位）。
    ///
    /// # ⭐ 与 `off_by` ／ 池满窃取的分工（⛔ 三件事，⛔ 不互相借用）
    ///
    /// * 本函数**不是** [`Self::stop_off_by_victims`]：那条是"**另一个区域**指名关断一个组"，
    ///   看 victim 自己的 `off_mode`；本函数是"**同一音符连发**的并发数上限"，
    ///   ⛔ **不看** `off_mode` —— 规范把 `off_mode` 的作用面限定在 `off_by`：
    ///   "This opcode will determinate how a region is turned off by an `off_by` opcode."
    /// * 本函数**不是** [`Self::steal`]（**池满**的处置）：⛔ 不记 [`Self::stolen`]／
    ///   [`Self::dropped`]、⛔ 不动 `pending`／[`FadeRamp`]、⛔ 不做"挂起者接管"。
    ///   池容量（[`SFZ_VOICES_PER_TRACK`]）与每音符上限是**两件事**。
    ///
    /// # ⭐ `one_shot` 的 victim **照掐**（语料逼出来的正确答案，与 `off_by` 那条**不同**）
    ///
    /// `#94` 裁决过"`one_shot` 的 victim 忽略 `off_by`"（那是"**另一个区域**来掐我"）。
    /// ⛔ 那条**不**外推到本函数：登记语料里 `note_polyphony` 的用法**几乎全长在
    /// `one_shot` 上** —— `virtuosity-drums` 的映射文件是
    /// `<global> loop_mode=one_shot … note_polyphony=3 group=5NN`，
    /// `cithara-barbarica` 的 Lyre 模式是 `loop_mode=one_shot`／`note_polyphony=1`／
    /// `note_selfmask=off`／`group=1` **四行紧挨着**
    /// （`assets/samples/cithara-barbarica/Programs/01-cithara_barbarica_keyswitch.sfz:43-46`）。
    /// 若把 `off_by` 那条外推过来，`note_polyphony` 会在**用到它的每一个文件里**
    /// 变成空操作 ⇒ 那不是规范的意思（`note_polyphony` 管的是"同音连发的并发数"，
    /// ⛔ 不是"note-off 到不到"）。
    ///
    /// # 让位顺序（确定性，[ARCH-DET-001]）
    ///
    /// 取排序键 `(力度, 触发序号, 槽位下标)` 的**最小者** —— 与 `yeban-sfz` 的
    /// `VoicePool::apply_note_polyphony` **同一条**键。那是本仓对"多个有资格者掐谁"的
    /// **既有**工程裁决（[`NotePolyphony`] 的文档逐字写着"选择顺序由
    /// `VoicePool::apply_note_polyphony` 规定（力度最低 → 触发最早 → 槽位下标最小），
    /// 保证 [ARCH-DET-001] 的确定性"）⇒ 本片**复用**它，⛔ 不另立一份排序。
    ///
    /// # 分配纪律
    ///
    /// 一次 ≤ [`SFZ_VOICES_PER_TRACK`] 槽的线性扫描（**触发期**一次，⛔ 不进逐样本路径）：
    /// **零分配、零锁等待、零阻塞 I/O**。victim 的 `group` 由它**自己**的
    /// [`SfzVoice::region_index`] 回查摊平表 ⇒ **⛔ 不新增任何逐声部字节**（`S9` 仍钉 40 B）。
    fn enforce_note_polyphony(
        &mut self,
        plan: &PreparedSfz,
        note: u8,
        velocity: u8,
        group: u32,
        policy: NotePolyphony,
    ) {
        if policy.is_unlimited() {
            return;
        }
        let limit = policy.limit as usize;
        // 在场数（**只数 `active`**，与 [`Self::active_count`] 同一量规 —— 被掐掉的声部
        // 当场 `silence()` ⇒ 下一次触发时它不再计入）。
        let mut standing = self.standing_same_key(plan, note, group);
        while standing >= limit {
            let Some(victim) = self.note_polyphony_victim(plan, note, velocity, group, policy)
            else {
                // 规范："A new note will always play." ⇒ 没有有资格者就停下（在场数可超上限）。
                break;
            };
            // ⭐ 掐掉 ＝ **既有**的硬停原语（与 `off_by` 的 `fast` 分支同一句）：
            // ⛔ 不新造包络/淡化路径、⛔ 不记窃取/丢弃、⛔ 不动同槽的 `pending`
            // （挂起者照常接管；`fade` 归零的理由与 `stop_off_by_victims` 逐字相同）。
            self.voices[victim].silence();
            self.fade[victim] = FadeRamp::NONE;
            standing -= 1;
        }
    }

    /// **同键在场声部数**（`note` ＋ `group`；只数 `active`）。零分配、零 I/O。
    fn standing_same_key(&self, plan: &PreparedSfz, note: u8, group: u32) -> usize {
        (0..SFZ_VOICES_PER_TRACK)
            .filter(|index| {
                let voice = self.voices[*index];
                voice.is_active()
                    && voice.note() == note
                    && self.voice_group(plan, *index) == Some(group)
            })
            .count()
    }

    /// **第 `index` 槽的声部来自哪条区域的 `group`**（`region_index` 回查；
    /// 查不到 ⇒ `None` ⇒ 调用方**不**把它计入任何键，与 `stop_off_by_victims` 同径）。
    fn voice_group(&self, plan: &PreparedSfz, index: usize) -> Option<u32> {
        let region_index = usize::from(self.voices[index].region_index());
        plan.regions().get(region_index).map(PreparedRegion::group)
    }

    /// `note_polyphony` 的下一个让位对象：**同键**（`note` ＋ `group`）且**有资格**
    /// （[`NotePolyphony::masks`]）的在场声部里，排序键 `(力度, 触发序号, 槽位下标)`
    /// 的**最小者**；没有则 `None`（新声部照常发声）。零分配、零 I/O。
    fn note_polyphony_victim(
        &self,
        plan: &PreparedSfz,
        note: u8,
        velocity: u8,
        group: u32,
        policy: NotePolyphony,
    ) -> Option<usize> {
        let mut best: Option<(u8, u64, usize)> = None;
        for index in 0..SFZ_VOICES_PER_TRACK {
            let voice = self.voices[index];
            if !voice.is_active()
                || voice.note() != note
                || self.voice_group(plan, index) != Some(group)
                || !policy.masks(voice.velocity(), velocity)
            {
                continue;
            }
            let key = (voice.velocity(), self.order[index], index);
            if best.is_none_or(|current| key < current) {
                best = Some(key);
            }
        }
        best.map(|(_, _, index)| index)
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
    ///
    /// ⭐ 任务单 `#107`：本方法是 [`Self::render_into_with_cc`] 传 `cc = None` 的**薄包装**
    /// ⇒ 既有调用点（含所有既有判据）一个字都不用改，读数与接线前**逐位相同**。
    pub fn render_into(&mut self, plan: &PreparedSfz, position: u64, out: &mut [f32]) {
        self.render_into_with_cc(plan, position, out, None);
    }

    /// ⭐ **带 MIDI CC 探针的逐块渲染**（任务单 `#107`）：语义、退休口径与窃取淡出
    /// **逐字**沿用 [`Self::render_into`]（见那份文档 —— 本函数只多**一个**参数）。
    ///
    /// * `cc == None` ⇒ 与 [`Self::render_into`] **逐位相同**（该包装就传这个值）；
    /// * `cc == Some(probe)` ⇒ [`Self::slot_readings`] 把它交给
    ///   ① [`PreparedRegion::ampeg_run_with_cc_and_velocity`] ⇒ 区域的
    ///   `ampeg_*ccN`（含 `_onccN` ／ `_curveccN`）按 `base + Σ delta × curve(cc)`
    ///   求值，以及 ② ⭐ 任务单 `#108`：[`PreparedRegion::gain_at_with_cc`] ⇒
    ///   **CC 轴交叉淡化**按探针求值（两者都是**与离线两条路同一份实现**）。
    ///
    /// ⚠ **不改区域选择**：`cc` 只到 `slot_readings`，而"这条区域该不该响"由
    /// **摊平期**烧进掩码（[`flatten_regions`]）的键/力度/门控决定
    /// ⇒ CC 门控（`loccN`/`hiccN`）在实时通路上**仍在摊平期**判定（登记见模块头）。
    ///
    /// ⭐ ⭐ **追加修订段（任务单 `#109`）—— 被修订的是上面这两句**（⛔ 原文一字未删）：
    /// 区域选择里**还有一项不在摊平期** —— CC 门控（`loccN`/`hiccN`）在**触发期**由
    /// [`SfzVoicePool::trigger_note`] 按探针裁决（本函数的探针只影响**已建声部**的
    /// 逐块取值）。⚠ 本函数自己**不再**重复判门控：声部已经按触发期的判定建好了
    /// ⇒ 逐块再判会让"一个音符中途换 CC"把它拦腰截断（与规范的 "hard switch" 矛盾）。
    ///
    /// **零分配、零锁、零 I/O**：探针是只读借用，逐块每槽只多一次整数读（见模块头 CC 一节）。
    pub fn render_into_with_cc(
        &mut self,
        plan: &PreparedSfz,
        position: u64,
        out: &mut [f32],
        cc: Option<&dyn Fn(u8) -> u8>,
    ) {
        for index in 0..SFZ_VOICES_PER_TRACK {
            self.render_slot(index, plan, position, out, cc);
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
    fn render_slot(
        &mut self,
        index: usize,
        plan: &PreparedSfz,
        position: u64,
        out: &mut [f32],
        cc: Option<&dyn Fn(u8) -> u8>,
    ) {
        let mut offset = 0usize;
        let mut victim_stopped = false;
        // --- 1) 淡出段：victim 在自己的窗口里继续发声，逐帧乘衰减增益 ---
        if self.voices[index].active && self.fade[index].is_fading() {
            let voice = self.voices[index];
            let Some(readings) = Self::slot_readings(plan, voice, plan.engine_hz(), cc) else {
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
                readings.rate,
                readings.gain,
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
        let Some(readings) = Self::slot_readings(plan, voice, plan.engine_hz(), cc) else {
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
            readings.rate,
            readings.gain,
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
    /// * ⭐ `cc`（任务单 `#107`）：**会话运行态**的 CC 探针（`Fn(u8) -> u8`）；
    ///   `None` ⇒ `ampeg_*ccN` 一项都不加（与接线前逐位相同）。⭐ 任务单 `#108` 起它
    ///   还进 [`PreparedRegion::gain_at_with_cc`]（**CC 轴交叉淡化**的增益）。
    ///   两处都**只**影响取值，⛔ 不参与 `region.accepts(...)`
    ///   （区域选择仍在摊平期，见模块头的门控登记）。
    fn slot_readings(
        plan: &PreparedSfz,
        voice: SfzVoice,
        engine_hz: f32,
        cc: Option<&dyn Fn(u8) -> u8>,
    ) -> Option<SlotReadings> {
        let region = plan.regions().get(voice.region_index as usize)?;
        let pcm_len = plan.sample(voice.sample_index)?.pcm().len();
        let loop_window = region.loop_window(pcm_len);
        // ⭐ ⭐ CC 轴交叉淡化（任务单 `#108`）：**逐块现算**本声部的逐样本增益 ——
        // 乐器层（含 CC 轴）由 `region.gain_at_with_cc` 求值、再乘引擎层的
        // `voice.track_gain`（触发期抄下）。`note` / `velocity` / `region_index` 早就在
        // `SfzVoice` 里 ⇒ ⛔ **没有新的逐声部字段**（`SfzVoice` 仍 40 B，判据 `S9`）。
        // `cc == None` ⇒ 与接线前的 `voice.gain` **逐位相同**（同一个求值点同一个因子）。
        let gain = region.gain_at_with_cc(voice.note(), voice.velocity(), cc) * voice.track_gain();
        // ⭐⭐ `tune` 调制（任务单 `#116`）：**逐块现算**本声部的步进 ——
        // 基准速率 `voice.rate()`（触发期抄下，含静态 `tune`）乘上
        // [`PreparedRegion::tune_ratio_with_cc`]（音分增量 → 线性乘子）。
        // `note` / `velocity` / `region_index` 早就在 `SfzVoice` 里 ⇒ ⛔ **没有新的
        // 逐声部字段**（`SfzVoice` 仍 40 B，判据 `S9`／`S100`／`S106`）。
        // ⚠ 乘子的**唯一**实现是 `yeban-sfz` 的 [`tune_ratio_with_baked`]（离线／摊平
        // 两条路调的是**同一份**）⇒ 三条路逐位一致（判据 `S109`）。
        // `cc == None` ⇒ 乘子恰 `1.0` ⇒ `voice.rate() × 1.0` 与接线前**逐位相同**。
        let tune_ratio = region.tune_ratio_with_cc(cc);
        // ⭐⭐ **退化输入的守卫**（任务单 `#116`）：多个 `tune` 项代数相加后可让
        // `2^x` 上溢成 `Inf`（`|Σ delta| > 153600` 音分）或下溢成 `0`
        // ⇒ 本块速率会变成非有限／零。⭐ 口径**不是新发明的**：摊平期
        // （[`flatten_regions`] 的 `rate_per_note`）与两条一次性渲染路
        // （[`PreparedSfz::render_block_into_with_cc`] ／
        // [`SfzInstrument::render_note_into_with_cc`]）**早就**用同一条谓词
        // （"速率必须有限正数，否则该区域不产生采样输出"）拦住非法速率
        // ⇒ 这里对**乘子**照同一条口径如实退休（`None` ⇒ 调用方 `silence_slot`），
        // ⛔ 不静默改成 `1.0`、⛔ 不让 `Inf` 进逐样本循环（那会在 `index + 1` 上溢出）。
        // ⚠ `cc == None` 时乘子恰 `1.0` ⇒ 本守卫**永不成立** ⇒ 既有夹具逐位不变。
        //
        // ⭐ ⭐ **追加修订段（任务单 `#117`）—— 被修订的是上面这段最后两句里
        // 「这里对**乘子**……如实退休」与「（那会在 `index + 1` 上溢出）」那两支**
        // （⛔ 原文一字未删，口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋
        // 点名被修订的支）。`#117` 的病态夹具实测把两件事钉死了：
        //
        // 1. **守卫的对象必须与另两条路同量**：离线／摊平两条路拦的是
        //    **速率本身**（`rate = 基率 × 乘子`），而这里原来拦的是**乘子**
        //    ⇒ 存在一档输入（有限乘子 `1.69e38` × 基率 `4.0` ＝ `Inf`）让它们
        //    **退休**而本路**放行**（实测：另两路写 0 帧、本路拿 `rate = Inf`
        //    冲进逐样本循环）。⇒ 本片把守卫**下移到乘积之后**，谓词改成
        //    [`rate_is_usable`]（有限且非零）—— 与另两条路**同一量、同一谓词**
        //    （反向播放的 `voice.rate()` 为**负**，所以用 `!= 0.0` 而不是 `> 0.0`；
        //    对那两条路，速率恒非负 ⇒ 两者等价）。
        // 2. **`index + 1` 那一句不再是唯一出口**：有限乘子上界 ≈ `f32::MAX`
        //    ⇒ `rate` 本身可以**有限但极大**（实测 `1.69e38`）而通过守卫，
        //    此时 `position` 超过 `usize::MAX`、`f32 as usize` **饱和**成 `usize::MAX`
        //    ⇒ `index + 1` 在 debug 下 panic、release 下回绕并绕过"越界即退休"
        //    ⇒ [`mix_span_into`] 的索引算术已同片改成 `saturating_add(1)`
        //    （见那里的 `#117` 追加修订段）。
        let rate = voice.rate() * tune_ratio;
        if !rate_is_usable(rate) {
            return None;
        }
        // ⭐ 力度调制（任务单 `#106`）：**逐声部力度**就在 [`SfzVoice`] 里
        // （[`SfzVoice::velocity`]，触发期抄下、此后不变）⇒ 本路**不需要任何新字段**
        // （`SfzVoice` 仍 40 B）。
        // ⭐ ⭐ CC 调制（任务单 `#107`）：探针由调用方沿 `render_into_with_cc` 交下来
        // （`EngineRuntime` 的会话运行态表；`None` ⇒ 没有 CC 事件 ⇒ 逐位同接线前）。
        // 两者是**两支独立输入**，相加顺序由 `yeban_sfz::modulation` 定义（⛔ 不改）。
        let env = region.ampeg_run_with_cc_and_velocity(engine_hz, voice.velocity(), cc);
        let end_sample = if region.ignores_note_off() || env.is_some() {
            u64::MAX
        } else {
            voice.end_sample
        };
        Some(SlotReadings {
            loop_window,
            gain,
            rate,
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

    /// ⭐ 判据（任务单 `#108`）：**CC 轴也逐位相同** —— 带 CC 交叉淡化的区域，在同一个
    /// CC 探针下，摊平增益 ＝ `PlaybackSpec::total_gain`，**全 128×128 网格逐位相同**。
    ///
    /// 两臂（⛔ 各自是一个独立口径）：
    /// * `cc = None` ⇒ 与**接线前**逐位相同（对照臂：`playback_spec(..)`）——它证明
    ///   本片没有扰动键轴 / 力度淡入 / 力度淡出那三张表；
    /// * `cc = Some(probe)` ⇒ 与 `playback_spec_with(&query, ..)` 逐位相同（新口径）——
    ///   它证明烘进摊平表的 CC 几何与解析层是**同一条乘法链、同一个结合序**。
    #[test]
    fn flattened_cc_crossfade_gain_equals_total_gain_over_the_whole_grid() {
        // 键轴 ＋ 力度淡出 ＋ **两条 CC 段**（淡入 cc1、淡出 cc2），⛔ 不带任何 `ampeg_*`
        // ⇒ 这条判据量的**只有增益**。
        const XF: &str = "<region> sample=a.wav key=60 pitch_keycenter=60 volume=-3 \
             xfin_lokey=48 xfin_hikey=72 xfout_lovel=60 xfout_hivel=110 \
             xfin_locc1=0 xfin_hicc1=100 xfout_locc2=20 xfout_hicc2=90\n";
        let instrument =
            SfzInstrument::load(XF.to_owned(), &ParseLimits::default()).expect("解析必须成功");
        let sample = InMemorySample::sine(440.0, 48_000.0, 480);
        let prepared = PreparedSfz::prepare(&instrument, &sample, 48_000.0).expect("摊平");
        let flat = &prepared.regions()[0];
        assert_eq!(
            flat.cc_crossfade_count(),
            2,
            "摊平表必须真的烘进两条 CC 段（⛔ 不是只看解析层）"
        );
        assert_eq!(flat.cc_crossfades()[0].axis, XfAxis::Cc(1));
        assert_eq!(flat.cc_crossfades()[1].axis, XfAxis::Cc(2));

        // 探针：cc1 读 64、cc2 读 40、其余读 0（与 `ControlChangeState::value` 同口径）。
        let probe = |controller: u8| -> u8 {
            match controller {
                1 => 64,
                2 => 40,
                _ => 0,
            }
        };
        // 鉴别力自检：探针必须真的改变增益（⛔ 否则本判据对结合序无话可说）。
        assert_ne!(
            flat.gain_at(60, 100),
            flat.gain_at_with_cc(60, 100, Some(&probe)),
            "⭐ CC 段必须在增益上生效（否则下面是空转）"
        );

        let rates = RenderRates::new(48_000.0, 48_000.0);
        // 臂 ①：`cc = None`（接线前沿用的那条：`playback_spec(..)`）。
        let (mismatches_none, first_none) = instrument
            .with_instrument(|parsed| {
                let region = &parsed.regions()[0];
                let mut mismatches = 0usize;
                let mut first: Option<(u8, u8, u32, u32)> = None;
                for n in 0u16..128 {
                    for v in 0u16..128 {
                        let want = region.playback_spec(n as u8, v as u8, rates).total_gain();
                        let got = flat.gain_at_with_cc(n as u8, v as u8, None);
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
        // 臂 ②：`cc = Some(probe)`（本片新接的那条：`playback_spec_with(&query, ..)`）。
        let probe_ref: &dyn Fn(u8) -> u8 = &probe;
        let (mismatches_cc, first_cc) = instrument
            .with_instrument(|parsed| {
                let region = &parsed.regions()[0];
                let mut mismatches = 0usize;
                let mut first: Option<(u8, u8, u32, u32)> = None;
                for n in 0u16..128 {
                    for v in 0u16..128 {
                        let query = RegionQuery::new(n as u8, v as u8).with_cc(probe_ref);
                        let want = region.playback_spec_with(&query, rates).total_gain();
                        let got = flat.gain_at_with_cc(n as u8, v as u8, Some(probe_ref));
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
            "[sfz] CC 轴摊平增益 vs total_gain：`cc = None` 位型不同的点数 = {mismatches_none}\
             （首个 = {first_none:?}）；`cc = Some(probe)` 位型不同的点数 = {mismatches_cc}\
             （首个 = {first_cc:?}）"
        );
        assert_eq!(
            mismatches_none, 0,
            "⭐ `cc = None` 必须与 `playback_spec(..)` **逐位相同**（键/力度两轴未被扰动；\
             首个不同点 = {first_none:?}）"
        );
        assert_eq!(
            mismatches_cc, 0,
            "⭐ `cc = Some(probe)` 必须与 `playback_spec_with(..)` **逐位相同**\
             （首个不同点 = {first_cc:?}）"
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
