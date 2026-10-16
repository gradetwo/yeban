//! SFZ 的 **MIDI CC 调制**：`ccN`（SFZ1 旧形）／`_ccN`／`_onccN`（SFZ2）／`_curveccN`（曲线）。
//!
//! 本模块只做三件事：**名字识别**（[`parse_modulation_name`]）、**归约**（[`CcMod`]）、
//! **求值**（[`cc_value_with_baked`]）。解析（读作用域链、查取值域）在
//! [`crate::instrument`]，本模块不碰任何 `OpcodeMap`。
//!
//! ⚠ **沿革（任务单 `#115`）**：`#104` 只服务五个 `ampeg_*` 参数；`#115` 把**目标**
//! 推广成 [`ModTarget`]（增益族的第一个目标是 [`ModTarget::Amplitude`]）。
//! ⛔ 上面两句的语义未变，只是"目标"从一个写死的类型变成了枚举；
//! `#104`／`#106` 的旧名（`AmpegCcMod`／`BakedAmpegCcMod`／`bake_ampeg_mods`／
//! `ampeg_value_with_baked*`）保留为**薄包装/别名**（口径同台账第 278 条）。
//! 推广的规范逐字与裁定见下面 §5。
//!
//! # 规范逐字（先说清四形态的语义差别，再写代码）
//!
//! ## 1. 四种形态
//!
//! | 形态 | 例 | 规范出处 | 语义 |
//! | :--- | :--- | :--- | :--- |
//! | **基准** | `ampeg_attack=0.5` | <https://sfzformat.com/opcodes/ampeg_attack/> 表格行（Type = float，Default = 0，Range = 0 to 100，Unit = seconds） | 被调制的那一项本身 |
//! | **旧形 `ccN`** | `ampeg_attackcc100=1.5` | 同页 Modulation 表：`ampeg_attackccN`，Type = float，Default = 0，**Range = -100 to 100**，Unit = seconds | **调制量 `delta`**（不是取值） |
//! | **SFZ2 `_onccN`** | `ampeg_attack_oncc100=1.5` | <https://sfzformat.com/modulations/midi_ccs/> 正文：SFZ2 用 `_onccN`；"although the `ccN` and `_ccN` versions **may be aliases**, depending on implementation" | **与旧形同义**（下表登记为同一条归约） |
//! | **曲线 `_curveccN`** | `ampeg_attack_curvecc100=2` | 同页表：`ampeg_attack_curveccN`，Version = **ARIA**，Type = **integer**，Default = **0**，**Range = 0 to 255**，Unit = N/A；<https://sfzformat.com/modulations/curveccN/>："designates a ‹curve› which shapes the **controller input**" | **曲线下标**（⛔ 不是 `delta`） |
//!
//! `_curveccN` 的**取值域**在同一张表里是 `0 to 255`（integer）。⚠ 与
//! <https://sfzformat.com/headers/curve/> 的 "Curve_index in ARIA can be any integer from
//! **0 to 254**" 差一位；两页冲突，本 crate **不发明**：按**本 opcode 自己那一页**的表取
//! `0..=255`（255 永远解析不到一条曲线 ⇒ 见 [`BakedCurve::UNRESOLVED`]）。
//!
//! ⛔ **不属本族**：`ampeg_*smoothccN` / `ampeg_*stepccN` —— `ampeg_attack` 页的 Modulation
//! 表**只有** `ccN` / `_curveccN` / `vel2*` 三行（没有 `smoothccN` / `stepccN`），
//! 且登记语料 1398 个 `.sfz` 里 `ampeg_*smoothccN` / `stepccN` 出现 **0 次**
//! ⇒ 本模块**不识别**这两个名字，它们按未知 opcode 忽略（既有口径）。
//!
//! ## 2. 公式：**代数相加** `base + delta × curve(cc_value)`
//!
//! ⛔ 不是从 `vel2` 外推，而是 SFZ1 CC 教程**逐字**给出的同一条形状：
//! <https://sfzformat.com/tutorials/sfz-1-midiccs/>：
//!
//! > the value of the MIDI CC adjusting the setting applied by the opcode
//! > (**0 to 127 mapped to the maximum set in the modulation**)
//!
//! 同页的 `offset` 例（`offset=500` ＋ `offset_cc100=500` ⇒ "giving an offset of 1000
//! at maximum"）就是 `base + delta × cc/127`。**曲线**那一层由 `curveccN` 页补上：
//!
//! > If absent, the default curve is used, **a straight line which runs from 0 to 1**.
//!
//! 而 `‹curve›` 页的内建表把 **curve 0 = Default，0 to 1，Linear** 定死 ⇒ 缺省时
//! `curve(cc) == cc/127`，与教程的 `0 to 127 mapped to the maximum` 逐字吻合。
//! 第二个独立见证是 <https://sfzformat.com/modulations/midi_ccs/> 的 `<control>` 例
//! （`width=50` ＋ `width_cc12=50` ＋ curve 1 = 双极 `-1 to 1`）："Pushing CC12 to maximum
//! will **add 50** … pulling CC12 to minimum will **subtract 50**" ⇒ `base + delta × curve`。
//!
//! ## 3. `set_ccN` 的初值语义（与本模块的关系）
//!
//! `set_ccN` 是**加载时**的 CC 初值（[`crate::control`]），**⛔ 不是**调制量：
//!
//! > "Sets a default initial value for MIDI CC number N, when the instrument is initially
//! > loaded."（<https://sfzformat.com/opcodes/set_ccN/>）
//!
//! 求值只读**调用方的 CC 探针**（[`crate::RegionQuery::cc`] 的同一形状）。"探针没给"
//! ⇒ **无调制**（＝ 与接线前逐位相同，见 [`ampeg_value_with_baked`] 的调用方约定）；
//! 「把探针按 `set_ccN` 播种」是**调用方**的事（[`crate::Instrument::cc_defaults`] 就是
//! 那个交接点），本模块 ⛔ 不替调用方应用初值，也 ⛔ 不读文件级状态。
//!
//! ⭐ ⭐ **追加修订段（任务单 `#110`）—— 被修订的是上面这段的"下游那一支"**
//! （⛔ 原文一字未删，口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋ 点名被修订的支）：
//! "**播种归调用方**"这条**契约本身不变**（本模块仍然不持有 CC 状态、仍然不读文件级状态、
//! 仍然只读调用方给的探针）；被修订的是**那位调用方现在真的播了** ——
//! `yeban-engine` 在**快照边界**按摊平表自带的 `CcSeedTable`
//! （`PreparedSfz::cc_seeds`，摊平期从 `Instrument::cc_defaults` 抄下）调
//! `ControlChangeState::seed`，规则是"**只填从未收到过的槽**"⇒ 真 MIDI CC 永远赢、
//! 没有 `set_ccN` 的文件一个字节都不动。⇒ 本模块下游拿到的探针**从此按文件声明的
//! 初值起步**（这正是本节开头引的那句规范语义要的效果）。
//!
//! # 工程裁决（登记，⛔ 不静默）
//!
//! 1. **CC 号域 = `0..=255`**（[`MAX_MODULATION_CC`]）。依据是**既有容器**：
//!    [`crate::RegionQuery::cc`] 的探针类型是 `&dyn Fn(u8) -> u8` ⇒ 能表达的最大下标是
//!    `u8::MAX`。规范侧：标准 CC 是 `0..=127`，SFZ2 的扩展 CC 是 `128..=137`
//!    （<https://sfzformat.com/extensions/midi_ccs/>），ARIA 再加 `140..=155`。
//!    登记语料里 `ampeg_*` 族用到的最大下标是 **131**（`ampeg_attack_oncc131` /
//!    `ampeg_attack_curvecc131`，MTG sax 族 16 处）⇒ `0..=255` **覆盖全部登记语料**。
//!    ⚠ 超过 255 是明确 `Err`（⛔ 不钳位）。**扩展 CC 的取值域不是 0..127**
//!    （135 = unipolar random 0..1、136 = bipolar random -1..1、140/141 = 半音数）——
//!    `u8` 探针表达不了负值/小数，**本片不声称支持它们的取值语义**（登记为缺口）。
//! 2. **`delta` 域 = `-100..=100`**（[`MIN_MODULATION_DELTA`] / [`MAX_MODULATION_DELTA`]），
//!    与目标同单位（时间 = 秒、`sustain` = 百分比）。出处是各 opcode 页 Modulation 表的
//!    Range 列（`ampeg_attackccN` / `ampeg_sustainccN` 都是 `-100 to 100`）。**允许负值**
//!    —— 登记语料里负 `delta` 有 **760 处**（`ampeg_decay_oncc71=-2.4` 一类）。
//! 3. **结果不钳位**：规范只给了 `delta` 的域，**没有**任何一句说"调制后的结果要夹回目标
//!    opcode 的 Range"⇒ 本 crate ⛔ 不发明夹取。负的时长由消费方按零长段处理
//!    （`yeban-engine` 的 `ampeg_run_from`：非正数 ⇒ `0` 帧），`sustain` 也由消费方
//!    夹到 `0..=1` 增益（那里是既有口径）。
//! 4. **只有 `_curveccN`、没有 `ccN`/`_onccN` 的 CC ⇒ 不产生调制项**：曲线只"shapes the
//!    controller input"，没有 `delta` 就没有任何量被调制。这是**退化输入**，⛔ 不是错误。
//! 5. **求和顺序 = `(param, cc)` 升序**（[`AmpegParam`] 的声明序 × CC 号）：二进制浮点的
//!    加法不结合 ⇒ 顺序必须唯一，才能保证同一文件两次求值逐位相同 [ARCH-DET-001]。
//! 6. ⭐ **力度项在求和顺序里的位置 = 最后一项**（任务单 `#106`）：每个参数按
//!    `base + Σ_cc delta×curve(cc) + delta_vel2 × velocity/127` 求值 —— CC 项按 CC 号
//!    升序（裁决 5 不变），**力度项永远加在末尾**。规范只给了公式的形状、**没有**规定
//!    两支输入的相加次序 ⇒ 这是**本仓**为 [ARCH-DET-001] 定的唯一顺序（⛔ 不声称规范如此）。
//! 7. ⭐ **力度项的域 = `-100..=100`**，与 `delta` **共用**同一对常数
//!    （[`MIN_MODULATION_DELTA`] / [`MAX_MODULATION_DELTA`]）：五个 `ampeg_vel2*` 页的
//!    Range 列与 `ccN` 那几行**逐字相同**。单位随目标（时间 = 秒、`sustain` = %）。
//!    ⛔ **缺失（`None`）与"写了 0"是两回事**：`None` ⇒ **一项算术都不做**（⇒ 与接线前
//!    逐位相同）；`Some(0.0)` ⇒ 真的做一次 `+ 0.0 × v/127`（对 `-0.0` 基准可与 `+0.0`
//!    差一位）⇒ 归约层用 `Option` 如实区分，⛔ 不用哨兵值。
//!
//! ## 4. ⭐ 力度调制（`vel2`；任务单 `#106`）—— **第二支独立输入**
//!
//! 五个 opcode 的**逐字**表行（<https://sfzformat.com/opcodes/ampeg_attack/> 一族的
//! Modulation 表，五行**同形**）：
//!
//! | Name | Version | Type | Default | Range | Unit |
//! | :--- | :--- | :--- | :--- | :--- | :--- |
//! | `ampeg_vel2attack` | （空） | float | 0 | **-100 to 100** | **seconds** |
//! | `ampeg_vel2hold` | （空） | float | 0 | **-100 to 100** | **seconds** |
//! | `ampeg_vel2decay` | （空） | float | 0 | **-100 to 100** | **seconds** |
//! | `ampeg_vel2sustain` | （空） | float | 0 | **-100 to 100** | **%** |
//! | `ampeg_vel2release` | （空） | float | 0 | **-100 to 100** | **seconds** |
//!
//! 公式的**唯一**逐字出处是 <https://sfzformat.com/modulations/vel2/>：
//!
//! > `(param) = (eg type)_(param) + ( (eg type)_vel2(param) * velocity / 127 )`
//!
//! 同页例（`ampeg_attack=0.5` ＋ `ampeg_vel2attack=-0.4` ⇒ "attack of 0.5 seconds at
//! 0 velocity and 0.1 seconds at 127 velocity"）就是这条公式的两个端点：
//! `0.5 + (−0.4 × 0/127) = 0.5`、`0.5 + (−0.4 × 127/127) = 0.1`。
//!
//! ⛔ **没有曲线 opcode**：`ampeg_attack` 页的 Modulation 表**只有** `ccN` ／
//! `_curveccN` ／ `vel2*` 三行 —— `_curveccN` 修的是 **CC** 的输入，力度那一支的
//! 映射在公式里写死为 `velocity / 127`（[`velocity_scale`] 是它的唯一实现）。
//! ⛔ **负值合法**（Range 的下界就是 `-100`）。
//! ⛔ **与 `ccN` 是两支独立输入**：一支来自 CC 状态、一支来自**音符力度**
//! （每个声部自带、触发后不变）⇒ 两者**相加**，⛔ 不互相顶替。
//!
//! ## 5. ⭐ 增益族目标推广（任务单 `#115`）—— 首个目标 `amplitude`
//!
//! ⭐ ⭐ **追加段（任务单 `#115`）**：本节**推广**上面四节的目标类型，⛔ 不修订它们的
//! 任何一条（五个 `ampeg_*` 的语义、公式、域与求和序**逐字不变**）。
//!
//! ### 5.1 规范逐字（`amplitude` 页，<https://sfzformat.com/opcodes/amplitude/>）
//!
//! 表行（**逐字**）：
//!
//! | Name | Version | Type | Default | Range | Unit |
//! | :--- | :--- | :--- | :--- | :--- | :--- |
//! | `amplitude` | ARIA | float | **100** | 0 to 100 | **%** |
//! | `amplitude_onccN` | （空） | float | N/A | **-100 to 100** | **%** |
//! | `amplitude_curveccN` | （空） | integer | **0** | **0 to 255** | N/A |
//! | `amplitude_smoothccN` | （空） | N/A | N/A | （空） | N/A |
//!
//! 同页正文逐字：「**Amplitude for the specified region in percentage of full
//! amplitude. 100% amplitude meaning no volume change is applied to the sample when
//! played. Can be modulated with MIDI CC.**」以及「Note: Range in the table below is
//! 0 to 100, but some players such as Aria/Sforazando do not clamp the range. Negative
//! values invert the signal.」
//!
//! ⇒ `amplitude_onccN` 的 **Range ＝ `-100 to 100`、Unit ＝ `%`** 与五个 `ampeg_*ccN`
//! 行**同形**（⇒ [`MIN_MODULATION_DELTA`] / [`MAX_MODULATION_DELTA`] 原样复用、
//! ⛔ 不开第二对常数）；`amplitude_curveccN` 的 Type/Default/Range 与
//! `ampeg_*_curveccN` **逐字相同** ⇒ [`MAX_MODULATION_CURVE_INDEX`] 原样复用。
//! ⛔ `amplitude_smoothccN` 的表行 Type ＝ N/A（未实现）⇒ 本模块**不识别**它
//! （按未知 opcode 忽略），与 `ampeg_*smoothccN` 同一条口径。
//!
//! ### 5.2 `amplitude` 与 `volume` 的关系（**逐字**依据）
//!
//! 唯一一处把两者放在一起写的规范页是 <https://sfzformat.com/opcodes/volume/> 的
//! 收尾段：
//!
//! > For the final output level, you can expect the sample level to be adjusted by a
//! > combination of the following:
//! > - volume (as adjusted by, e.g. rt_decay, amp_keytrack and other direct modifiers)
//! >   **in decibels**
//! > - **linear amplitude adjustment by ampeg... opcodes, along with adjustments to that
//! >   by amplitude opcodes**
//! > - pan
//!
//! ⇒ 三条**逐字**事实：① `volume` 的单位是 **dB**；② `ampeg…` 与 `amplitude` 一起构成
//! **linear amplitude adjustment**（线性振幅调整）；③ `amplitude` 的调整**叠加在
//! ampeg 那台包络之上**（"along with adjustments **to that** by amplitude opcodes"），
//! 与 `volume`（dB 段）是**两个不同的段**。⇒ **裁定：`amplitude` 是一条线性因子，
//! 与 `volume` 的 dB→线性换算**相乘**，⛔ 不是 `volume` 的别名、⛔ 不是再一串 dB。**
//!
//! ### 5.3 施加点与公式（本仓的**唯一**形状）
//!
//! 复用 §2 的同一条代数相加公式（SFZ1 教程的 `offset=500` ＋ `offset_cc100=500`
//! ⇒ "an offset of 1000 at maximum" 就是 `base + delta × curve`；`amplitude` 只是把
//! **单位换成 %**）：
//!
//! ```text
//! amplitude_value = amplitude + Σ_{cc} delta × curve(probe(cc))      // 单位 %
//! amplitude_gain  = amplitude_value / 100                            // 线性因子，100 ⇒ 1.0
//! final_gain      = PlaybackSpec::total_gain() × amplitude_gain      // ⭐ 最外层乘子
//! ```
//!
//! * `amplitude` 缺省 ⇒ **100**（表行 Default 列逐字）⇒ `amplitude_gain == 1.0`
//!   ⇒ 乘它**逐位不变**（IEEE：任何有限值 × `1.0` 恒等）⇒ **接线前后逐位相同**；
//! * `probe == None` ⇒ 一项 CC 都不加（与 §3 的探针约定一致）；
//! * ⛔ **不钳位**：规范自己记了"some players … do not clamp"，且 §2 的公式**只给
//!   `delta` 的域**⇒ 本 crate 沿用裁决 3（⛔ 不发明夹取）。负值 ⇒ 反相
//!   （规范逐字 "Negative values invert the signal"）。
//! * ⚠ **算子次序**：`amplitude_gain` 是**最外层**乘子（`total_gain × amplitude_gain`），
//!   三条路（离线 ／ 摊平表 ／ 实时）**都**用这一个次序与**同一份**求值本体
//!   （[`cc_value_with_baked`]）⇒ 逐位一致（判据 `S103`）。
//!
//! ⚠ **base 的取值歧义（如实登记，⛔ 不发明）**：`amplitude` 页的表行把 Default 写成
//! **100**，而同一页正文与 `‹curve›` 页的例子（`amplitude_oncc110=100` ＋
//! `amplitude_curvecc110=9`，曲线 9 为 `0→0→1`）读起来像"CC 从 0 到最大 ⇒ 音量从
//! 静音到满"（那要求 base ＝ 0）。两处**互相冲突** ⇒ 本 crate 取**表行的 Default
//! ＝ 100**（它才是唯一带数值的声明），并把"要 0 → 满"的写法留给作者显式写
//! `amplitude=0`（登记语料里 `cithara-barbarica` 的静音区正是这么写的）。⛔ 不把
//! 冲突的一侧当成另一侧。
//!
//! ## 6. ⭐⭐ `tune` 目标（任务单 `#116`）—— 第二个被推广的目标
//!
//! ⭐ ⭐ **追加段（任务单 `#116`）**：本节**再推广一个目标**（[`ModTarget::Tune`]），
//! ⛔ **不修订** §1～§5 的任何一条（五个 `ampeg_*` 与 `amplitude` 的语义、公式、域与
//! 求和序**逐字不变**）。
//!
//! ### 6.1 规范逐字（<https://sfzformat.com/opcodes/tune/>，2026-10-16 现读）
//!
//! 该页**只有一行** `tune` 自己的表行（**逐字**）：
//!
//! | Name | Version | Type | Default | Range | Unit |
//! | :--- | :--- | :--- | :--- | :--- | :--- |
//! | `tune` | SFZ v1 | integer | 0 | **-100 to 100** | **cents** |
//!
//! 同页正文逐字：「The fine tuning for the sample, **in cents**.」＋
//! 「Range of `tune` in the SFZ1 spec is ±1 semitone, from -100 to 100, though at least
//! in ARIA, it seems a broader range is supported, at least -2400 to 2400 cents.」
//!
//! ⚠ ⚠ **本单最重要的一条事实裁决（派单方写错的那一条）**：该页的 **Modulation 表里
//! ⛔ 没有 `tune_onccN` / `tune_ccN` / `tune_curveccN` 行**。它列的是 **`pitch` 族**：
//!
//! | Name | Version | Type | Default | Range | Unit |
//! | :--- | :--- | :--- | :--- | :--- | :--- |
//! | `pitch_onccN` | SFZ v2 | N/A | N/A | **-9600 to 9600** | **cents** |
//! | `pitch_curveccN` | SFZ v2 | integer | 0 | **0 to 255** | N/A |
//! | `pitch_smoothccN` | SFZ v2 | float | 0 | 0 to ? | ms |
//! | `pitch_stepccN` | SFZ v2 | N/A | 0 | 0 to ? | N/A |
//!
//! 而 `tune` 族与被别名者的关系**只有一句**（同页 Practical Considerations，逐字）：
//!
//! > In ARIA, `tune_*ccN` can also be used as an alias for `pitch_*ccN`.
//!
//! ### 6.2 `tune` 与 `pitch_keycenter` / `transpose` / `playback_rate` 的关系
//!
//! ⛔ 规范**没有**给"音分 → 速率"的公式。本仓的换算**不是本单发明的**，它比 `#104`
//! 还早：`crates/yeban-sfz/src/playback.rs` 从 `b5ca1528` 起就把三个 pitch opcode
//! 折进**同一个指数** —— 该文件模块头逐字写着
//! `pitch_ratio = 2 ^ ( ((note - pitch_keycenter + transpose) * 100 + tune) / 1200 )`，
//! 实现是 [`crate::Region::pitch_ratio`]（`cents = semitones × 100 + tune`，
//! 再 `exp2(cents / 1200)`）。⭐ 同理 `pitch_keycenter` 页只把 `pitch_keycenter` 定义成
//! "Root key for the sample"（Range `0 to 127`），它与 `transpose`（半音）一起决定
//! **指数里的整数半音项**，`tune` 决定**同一指数里的小数音分项** ⇒ 三者**加法共存**，
//! ⛔ 不互相覆盖。`PlaybackSpec::playback_rate` 是**速率**（`f32` ＝
//! `pitch_ratio × sample_hz / engine_hz`），而 `tune` 是**音分**（整数）⇒
//! 本单要挂的桥就是"音分增量 → 乘在速率上的线性因子"：
//!
//! ```text
//! rate_with_cc = rate_base × 2 ^ (Σ_{tune 项} delta × curve(probe(cc)) / 1200)
//! ```
//!
//! ⚠ 依据链：`tune` 的 Unit ＝ **cents**（表行逐字）＋ 一个八度 ＝ 1200 音分
//! （[`TUNE_CENTS_PER_OCTAVE`]，与 [`crate::Region::pitch_ratio`] 同源同值）
//! ⇒ `2^(Δcents/1200)`。⛔ 本单**不发明**第二套换算，
//! 唯一实现是 [`tune_ratio_with_baked`]。
//!
//! ### 6.3 `delta` 的域（**逐目标不同**，裁定 ＋ 依据）
//!
//! ⭐ **裁定：`tune` 族的 `delta` 域 ＝ `-9600..=9600` 音分**
//! （[`MIN_TUNE_MODULATION_DELTA`] / [`MAX_TUNE_MODULATION_DELTA`]），
//! ⛔ **不是** `tune` 基准自己的 `-100..=100`。三条依据：
//!
//! 1. `tune_*ccN` 的表行在规范里**不存在**，唯一可援引的是那句别名声明
//!    「`tune_*ccN` … alias for `pitch_*ccN`」⇒ 域随 `pitch_onccN` ＝ `-9600 to 9600`；
//! 2. `tune` 页**自己的例子**逐字是 `tune=-100` ＋ `pitch_oncc27=200` —— 那个 `200`
//!    **超出** `tune` 基准的 `-100..=100` ⇒ 规范文本自己证明"调制量的域 ≠ 基准的域"；
//! 3. 语料普查（分母 **1398** 份 `.sfz`）：`tune_*ccN` 的 `|delta| > 100` 有 **599** 处、
//!    最大 **2400**（`tune_cc98=2400`）⇒ 沿用 `±100` 会把今天**能解析**的 599 处
//!    变成不可解析（与 `#109` 的"⛔ 不改既有文件可解析性"同一条纪律）。
//!
//! ⚠ 这条与本模块裁决 2（"`delta` 域 ＝ `-100..=100`"）**并存**：裁决 2 管的是
//! 各 opcode 页 Modulation 表 Range 列**真写 `-100 to 100`** 的那些目标
//! （五个 `ampeg_*` ＋ `amplitude`）；`tune` 的不在其列。⭐ 唯一读它的地方是
//! [`ModTarget::delta_range`] ⇒ ⛔ 没有第二套域检查。
//! ⚠ `_curveccN` 的下标域**没有**分叉：`pitch_curveccN` 的 Range 也是 `0 to 255`
//! ⇒ [`MAX_MODULATION_CURVE_INDEX`] 原样复用。
//!
//! ### 6.4 施加点（**唯一形状**）
//!
//! `tune` 只**加音分、不改方向、不改区间** ⇒ 它是**速率上的一个线性乘子**
//! （[`tune_ratio_with_baked`]），三条路（离线 ／ 摊平表 ／ 实时）都把它乘在
//! **已经含基准 `tune` 的**速率上：
//!
//! ```text
//! 离线      rate = Region::playback_rate(note, rates) × tune_ratio        // yeban-engine
//! 摊平表    rate = PreparedRegion::rate_per_note[n]   × tune_ratio
//! 实时      rate = SfzVoice::rate()                   × tune_ratio        // 逐块现算
//! ```
//!
//! ⚠ `probe == None` ⇒ `tune_ratio` **恰** `1.0` ⇒ 三个乘法都是 `× 1.0`
//! （binary32 恒等）⇒ 与接线前**逐位相同**。
//! ⚠ 单位政策：`tune` 的 `delta` 认 `%` 后缀（[`crate::instrument::read_modulation_delta`]
//! 的非时间支）—— 这是**既有分支的副作用**（`Unit` 列的真实值是 `cents`）；
//! 如实登记为"过度宽松"，⛔ 不声称规范允许 `tune_onccN=50%`。
//!
//! ⭐ ⭐ **追加修订段（任务单 `#117`）—— 被修订的是上面这段的"认 `%` 后缀"那一支**
//! （⛔ 原文一字未删，口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋ 点名被修订的支）：
//! `#117` 起 `tune` 的 `delta` **⛔ 不接受 `%`**（也⛔ 不接受 `s` / `ms`）——
//! 上面那句描述的"非时间支一律剥 `%`"**只对 `ampeg_sustain` 与 `amplitude` 继续成立**
//! （它们的表行 Unit 逐字是 `%`）。判据与理由：
//!
//! 1. **规范依据**（§6.1 逐字）：`tune` 页的表行 Unit ＝ **`cents`**，同页
//!    Modulation 表**只有 `pitch` 族**（`pitch_onccN` 的 Unit 也是 `cents`），
//!    别名句只说 `tune_*ccN` 是 `pitch_*ccN` 的别名 ⇒ **`cents` 一族在规范里
//!    没有任何 `%` 写法**；
//! 2. **处置 ＝ 响亮 `Err`**（⛔ 不是"登记为未定义而悄悄接受"）：`tune_oncc1=50%`
//!    在解析期报 [`crate::error::SfzError::InvalidFloat`]（`opcode = "tune_oncc1"`、
//!    `value = "50%"`）。理由：本 crate 对"后缀与单位不匹配"的**既有口径**就是
//!    响亮 `Err`（第 259 条：⛔ 不静默降级、⛔ 不钳位）—— `tune_oncc1=50s` /
//!    `tune_oncc1=50ms` **在 `#116` 就已经**响亮 `Err`；`%` 与它们是同一类输入
//!    ⇒ 让它走同一条出口才不会在"哪种错后缀被吞掉"上产生第二个口径；
//! 3. **语料安全**：登记语料 **1398** 份 `.sfz` 里 `tune_*ccN` 的取值**全部**是
//!    纯数字（`%` **0** 处、任何后缀 **0** 处）⇒ 今天能解析的文件**一个都不会**
//!    变成不可解析；
//! 4. ⛔ **不收紧** `ampeg_*` / `amplitude`：实现是**逐单位分派**
//!    （[`ModUnit`]／[`ModTarget::mod_unit`]），`Seconds` 支与 `Percent` 支**逐字未动**。
//!
//! ## 7. ⭐⭐ `pitch` 族（任务单 `#118`）—— 与 `tune` 是**同一个目标**的两个拼写
//!
//! ### 7.1 规范逐字（<https://sfzformat.com/opcodes/tune/>，2026-10-17 现读）
//!
//! 同一页（`…/opcodes/pitch/` 落到的也是**这一页**）的 Modulation 表**逐字**四行：
//!
//! | Name | Version | Type | Default | Range | Unit |
//! | :--- | :--- | :--- | :--- | :--- | :--- |
//! | `pitch_onccN` | SFZ v2 | N/A | N/A | **-9600 to 9600** | **cents** |
//! | `pitch_curveccN` | SFZ v2 | integer | **0** | **0 to 255** | N/A |
//! | `pitch_smoothccN` | SFZ v2 | float | **0** | 0 to ? | **ms** |
//! | `pitch_stepccN` | SFZ v2 | N/A | **0** | 0 to ? | N/A |
//!
//! 同页 Practical Considerations 逐字（§6.1 已引一次；这里是**裁定的直接依据**）：
//!
//! > In ARIA, `tune_*ccN` can also be used as an alias for `pitch_*ccN`.
//!
//! ⚠ 该页的**静态**表**只有一行** `| tune | SFZ v1 | integer | 0 | -100 to 100 | cents |`
//! ⇒ ⛔ **规范里没有 `pitch` 这个静态 opcode**（`pitch_keycenter` 在**另一页**，
//! 且它定义的是 "Root key for the sample"，⛔ 不是音分偏移）。
//!
//! ### 7.2 裁定（本单要求自裁的那一条）：**同一个调制目标的两个拼写**
//!
//! ⭐ **裁定：`pitch_*ccN` 归 [`ModTarget::Tune`]** —— 与 `tune_*ccN` 落**同一个键**
//! （`BTreeMap<(ModTarget, u8), …>`），⛔ **不是**两个独立目标。三条理由：
//!
//! 1. **规范的用词就是 alias**：那句逐字写 "can also be used as an **alias** for"；
//!    而 Modulation 表里⛔ **没有** `tune_*ccN` 行 ⇒ 规范里只有**一个**实体
//!    （`pitch_*ccN`），`tune_*ccN` 是它的拼写变体 ⇒ 两个拼写调制的是**同一件事**；
//! 2. **仓内既有先例（同一形状）**：`ccN`（SFZ1）与 `_onccN`（SFZ2）规范说 "may be
//!    aliases" ⇒ [`crate::instrument::read_cc_mods`] 让它们落**同一个键**、后者胜
//!    （该函数文档逐字写着这条）⇒ "别名归一键"是本仓**已立**的形状，⛔ 非本单发明；
//! 3. **"两个独立目标"的代价算不出来**：那要新开一个枚举变体（`ModTarget::COUNT`
//!    7 → 8），而它与 `Tune` 的**物理效果完全相同**（都只喂
//!    [`tune_ratio_with_baked`] 的那**一个**指数）⇒ 求值层必须把两个目标**再合起来
//!    求和**，等于用两个目标实现一个目标；并且 `#116` 逐字登记的
//!    `COUNT == 7`／`ALL[6] == Tune`（判据 `S106` 的 `contains`）会被迫改写
//!    —— 与任务单"⛔ 不改既有 `tune_*ccN` 的读数"直接冲突。
//!
//! ⚠ **代价（如实登记，⛔ 不静默）**：同一作用域里**同一 CC** 上两个拼写同时出现
//! ⇒ 归约到**一个键**、**不求和**（`OpcodeMap` 是 `BTreeMap<String, _>` ⇒ 字典序
//! `pitch_…` < `tune_…` ⇒ `tune_…` 那一行胜）。这与 `ccN`／`_onccN` 的既有别名坍塌
//! 是**同一类**行为。⭐ 语料安全：登记语料 **1398** 份 `.sfz` 里 `pitch_*ccN` 只有
//! **39** 处（**1** 个文件 `assets/samples/karoryfer-bear-sax/Programs/5-bearcussion.sfz`），
//! 而该文件里 `tune_*ccN` **0** 处 ⇒ 今天**没有**可观察的碰撞。
//! ⚠ `ModTarget::Tune::name()` 仍是 `"tune"`（矩阵/诊断的标签）；而**报错文本**用的是
//! **原文 opcode 名**（[`crate::instrument::read_modulation_delta`] 的 `name` 参数）
//! ⇒ `pitch_oncc1=9601` 报的是 `pitch_oncc1`，⛔ 不冒充 `tune`。
//!
//! ### 7.3 落地形状（⛔ 零新字段、零新变体、零引擎改动）
//!
//! * **识别**：[`parse_modulation_name`] 里加**一支** `pitch`，与 `tune` 支**同款**的
//!   四种后缀（`ccN` / `_ccN` / `_onccN` / `_curveccN`）⇒ `pitch_onccN`、
//!   `pitch_ccN`、`pitch_curveccN` 三类写法读得进；
//! * **域与后缀**：⛔ **一行都不用改** —— 目标既然就是 [`ModTarget::Tune`]，
//!   它的 [`ModTarget::mod_unit`]（`Cents`）、[`ModTarget::delta_range`]
//!   （`±9600` 音分）与 [`ModUnit::suffixes`]（只认 `""`）**自动**生效
//!   （`#117` 的逐单位分派）；
//! * **归约**：落回**同一张** `BTreeMap` ⇒ 求和序（[`ModTarget::index`] × CC 升序）、
//!   [`AMPEG_MOD_CAP`] 与摊平表形状**全不变**；
//! * **求值**：三条路（离线／摊平表／实时）**早已**读 [`ModTarget::Tune`]
//!   （[`tune_ratio_with_baked`] 是唯一实现）⇒ `yeban-engine` **一个字都不改**；
//! * **尺寸**：`PreparedRegion` / `SfzVoice` / `SfzVoicePool` / `EngineRuntime`
//!   **四个全 ＋0**（判据 `S113` 由 `size_of` 现读）。
//!
//! ### 7.4 ⛔ 本单**不做**的两行（登记，⛔ 不静默忽略）
//!
//! 表里有 `pitch_smoothccN`（float，Default `0`，Unit **ms**）与 `pitch_stepccN`
//! （N/A，Default `0`）两行，⛔ **本单不做**：前者是**平滑时间常数**（需要逐块/逐样本
//! 状态），后者是**步进量化**（另一套曲线语义）—— 两者都**不在**"复用 `#116`／`#117`
//! 已立机制"的范围内（本单的边界是 **CC 调制量**，⛔ 不含平滑与步进）。
//! ⭐ **今天的行为（⛔ 一字不改）**：两者的名字落不进 [`parse_modulation_name`] 的
//! 四后缀 ⇒ 按**未知 opcode 忽略**（与 `tune_smoothccN`／`tune_stepccN` 同款）；
//! 判据 `S113` 把这一格**逐字**钉住（⛔ 不许哪天变成"识别到了但静默丢弃"）。
//!
//! ## 8. ⭐⭐ `volume` 族（任务单 `#119`）—— **第三个单位类别**（dB）
//!
//! ### 8.1 规范逐字（<https://sfzformat.com/opcodes/volume/>，2026-10-17 现读）
//!
//! 静态表**只有一行**：
//!
//! | Name | Version | Type | Default | Range | Unit |
//! | :--- | :--- | :--- | :--- | :--- | :--- |
//! | `volume` | SFZ v1 | float | **0** | \-144 to 6 | **dB** |
//!
//! 同页 **Modulation 表逐字四行**（⚠ 这是本单最承重的一条事实）：
//!
//! | Name | Version | Type | Default | Range | Unit |
//! | :--- | :--- | :--- | :--- | :--- | :--- |
//! | `gain_ccN` |  | float | **0** | **-144 to 48** | **dB** |
//! | `volume_curveccN` | SFZ v2 | integer | **0** | **0 to 255** | N/A |
//! | `volume_smoothccN` | SFZ v2 | N/A | N/A |  | N/A |
//! | `volume_stepccN` | SFZ v2 | N/A | N/A |  | N/A |
//!
//! ⚠ ⚠ **逐字事实 ①：Modulation 表里⛔ 没有 `volume_onccN` 这一行**。表里的
//! **delta 行是 `gain_ccN`**（`-144 to 48` dB）。`volume_onccN`（以及 `volume_ccN`）
//! 是**别名拼写**，出处是同页正文两句逐字：
//!
//! > Extended opcodes, such as `volume_curveccN` shown above, only apply to `volume`,
//! > although the specification originally stated `gain_ccN` for adjusting volume using CCs.
//! > `volume_ccN` is a valid alias for `gain_ccN` and is used for consistency in the example.
//!
//! ⚠ **逐字事实 ②**：`volume_curveccN` 的行 **Range ＝ `0 to 255`、Unit ＝ `N/A`**
//! —— 与 `amplitude_curveccN`／`pitch_curveccN` 同形（**曲线下标**，⛔ 不是调制量）。
//! ⚠ **逐字事实 ③**：`volume_smoothccN` 与 `volume_stepccN` 两行 **Range 空、Unit ＝ `N/A`**
//! ⇒ ⛔ **不发明域**（它们⛔ 不在本单范围）。
//! ⚠ **逐字事实 ④**（正文与表行**不一致**，如实登记）：正文写
//! "Whilst the range is **-144.6 to 6** in the specification"，表行写 **-144 to 6**
//! ⇒ 本单以**表行**为准（表行是结构化的那一个）。
//!
//! ### 8.2 单位裁定：⭐ **新开第四类 `ModUnit::Decibels`**（⛔ 不复用既有三类）
//!
//! ⭐ **裁定：`volume` 族的 `delta` 单位类别是**新变体 [`ModUnit::Decibels`]**。
//! 四条理由：
//!
//! 1. **物理类别不同**：既有三类是 秒（时间）／百分比（线性比）／音分（对数音高比）；
//!    dB 是**对数幅度比** —— 它的换算是 `10 ^ (dB / 20)`，与三者**都不是**同一个函数。
//!    ⛔ 复用 [`ModUnit::Percent`] 会让 `volume_oncc1=50%` 被**静默**接受（dB 没有
//!    `%` 写法）；⛔ 复用 [`ModUnit::Cents`] 会既接受 `%` 语义错、又把 `dB` 当音分；
//! 2. **`#117` 的分派就是为这件事建的**：`#117` 把 `#104` 的布尔 `is_time()`
//!    显式化成单位类别，其**存在理由**逐字是"非时间那一支把百分比与音分混在一起"
//!    ⇒ 遇到**第三个**类别时正解是**加一行**，⛔ 不是把 dB 塞进某一支；
//! 3. ⭐ **它⛔ 不动 `#104` 的 `is_time()` 契约**（这正是 `#116` 的警示）：
//!    [`ModTarget::is_time`] 现在是 `matches!(self.mod_unit(), ModUnit::Seconds)`
//!    ⇒ 新变体 `Decibels` 对它返回 `false`，而**七个既有目标**的 `is_time()` 返回值
//!    **一个都没变**（`Seconds ⇒ true`、`Percent`／`Cents ⇒ false`）；
//! 4. **代价可控、且已核实**：`ModUnit` 是 `#[derive(Copy)]` 的无载荷枚举，
//!    3 → 4 变体仍落在 **1 B**（判据 `S111` 现读 `size_of::<ModUnit>() == 1`）；
//!    ⛔ 不进任何定长容器（`BakedCcMod` 存的是 [`ModTarget`] 而⛔ 不是 `ModUnit`）
//!    ⇒ `PreparedRegion`／`SfzVoice`／`SfzVoicePool`／`EngineRuntime` **四个全 ＋0**。
//!
//! ⭐ **域裁定**：`volume` 族的 `delta` 域取 **`-144.0 ..= 48.0` dB** ——
//! 出处是 Modulation 表里**被别名的那一行** `gain_ccN` 的 Range `-144 to 48`
//! （与 `#116` 让 `tune` 随 `pitch_onccN` 的 `±9600` 是**同一条口径**）。
//! ⚠ 语料见证：登记语料 `volume_onccN` 只有 **2** 处（值均为 **7**）⇒ 域上下界
//! 都**没有**语料约束力，因此本题只能由规范表行回答（⛔ 不发明 ±100）。
//!
//! ⭐ ⭐ **追加修订段（任务单 `#125`）—— 被修订的是上面那句里的「只有 **2** 处」**（⛔ 原文
//! 一字未删，口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋ 点名被修订的支）：按 `#121` 裁的
//! 唯一口径（`assets/samples/**/*.sfz` 逐行**剥 `//` 注释** ⇒ 取**行首或空白边界**的 opcode
//! token ⇒ token 里 `volume_oncc` 之后的尾巴是**数字**或 **`$` 宏名** ⇒ ⭐ **收 `$` 宏名**）
//! ⭐ **实测 3 处 ／ 3 文件**：`volume_oncc24` **2** 处（值均为 **7**）＋ `volume_oncc$F2_DIS`
//! **1** 处（值 **-6**，`assets/samples/ixox-flute/Ixox Flute.sfz:152`）。⚠ 上面「**2** 处」
//! 恰好是**不收 `$` 宏名**那个口径的读数（`#121` 已复现）⇒ 域结论（两个端点都**没有**语料
//! 约束力）**逐字不变**。依据命令 ＝
//! `cargo test -p yeban-engine --test sfz_instrument s115 -- --nocapture`（判据 `S115` 用同一
//! 口径就地重算并逐名打印；判据 `S118` 把这三处就地修订本身钉住）。
//!
//! ⭐ **别名处置**（口径同 §7.2 与台账第 285 条）：`volumeccN`（裸）／`volume_ccN`
//! （SFZ1）／`volume_onccN`（SFZ2）／`volume_curveccN` 三个 delta 拼写 ＋ 一个 curve
//! 拼写**全部落同一个 [`ModTarget::Volume`] 键**。
//! ⚠ **代价（如实登记）**：同一作用域同一 CC 上两个拼写同时出现 ⇒ 归约到**一个键**、
//! **不求和**（`OpcodeMap` 是 `BTreeMap` ⇒ 字典序后者胜）—— 与 `ccN`／`_onccN` 的
//! 既有别名坍塌同形。
//!
//! ### 8.3 落地形状：**dB 增量 → 线性乘子**（⛔ 不重新读基准）
//!
//! 基准 `volume`（dB）**早就烘进**两条既有读数：`Region::linear_gain()`（摊平表的
//! `volume_gain`）与 `PlaybackSpec::volume_db`（离线 `total_gain()`）⇒ 本族只贡献
//! **一个线性乘子**（与 `#115` 的 `amplitude_gain`、`#116` 的 `tune_ratio` 同形）：
//!
//! ```text
//! volume_delta_db = 0 + Σ_{cc} delta × curve(probe(cc))      // 单位：dB
//! volume_ratio    = 2 ^ (volume_delta_db × log2(10) / 20)    // ＝ 10 ^ (dB/20)
//! 结果            = (… × amplitude_gain) × volume_ratio       // 最外层线性乘子
//! ```
//!
//! * `probe == None` ⇒ **一项 CC 都不读**、直接返回 `1.0`（`× 1.0` 在 binary32 下
//!   **逐位不变**）；
//! * `probe == Some` 且表里没有 `volume` 项 ⇒ `0.0 × log2(10) / 20 == 0.0`
//!   ⇒ `0.0f32.exp2()` 恰 `1.0` ⇒ 同样逐位不变；
//! * ⭐ **为什么是乘子而不是"重算 `10^((volume+delta)/20)`"**：`PreparedRegion` 里
//!   只有**烘好的** `volume_gain`、⛔ **没有** `volume_db` ⇒ 走"重新求指数"那条路
//!   需要给 `PreparedRegion` **加一个 `f32` 字段**（＝尺寸必须长大 ⇒ 任务单止损条件 ⑤）；
//!   乘子形状则**一个字段都不用加**；
//! * ⚠ ⭐ **如实登记的偏离**：上式是 `linear(volume) × linear(delta)`，⛔ 不是
//!   `linear(volume + delta)`。两者在 binary32 下**一般相差若干个 ulp**
//!   （判据 `S114` 量出实数差并把**两条**读数都打印）。⭐ 采用乘子形状的代价就是这条
//!   偏离；收益是"⛔ 零新字段 ＋ 无调制时逐位不变"两条硬约束同时成立。
//!   ⚠ 三条路用的是**同一个**乘子与**同一个**结合序 ⇒ 三路仍**逐位相同**（判据 `S114`）；
//! * ⛔ **不钳位**：域只由 [`ModTarget::delta_range`] 在**读取**层检查（第 259 条）；
//!   多项代数相加后 `2^x` 仍可能上溢成 `Inf`（`x > 128`）或下溢成 `0` ——
//!   消费方按**既有**守卫如实退休，⛔ 不静默改成 `1.0`（判据 `S114` 的病态臂）。
//!
//! ### 8.4 ⛔ 本单**不做**的行（登记，⛔ 不静默忽略）
//!
//! * ⭐ **`gain_ccN`**（规范 Modulation 表里**逐字的 delta 行名**，`-144 to 48` dB）
//!   与它的 ARIA 衍生拼写 `gain_onccN`：⛔ **本单不识别**。依据：① 任务单的
//!   accept 集逐字只列 `volume` 拼写；② 登记语料里**裸** `gain_ccN`＝**0** 处、
//!   `gain_onccN`＝**0** 处（那 14 处 `gain_onccN` 全是 `lfo*_eq1gain_onccN`，
//!   ⛔ 不是本族）；③ `gain` 在本仓是**静态** `volume` 的别名 opcode
//!   （[`crate::instrument`] 的 `scopes.get("volume").or_else(\|\| scopes.get("gain"))`）
//!   ⇒ 是否把 `gain_*ccN` 也接上属**另一张单**。⭐ 今天的可观察行为由判据 `S114` 钉住
//!   （⛔ 不是"哪天变成识别到了但静默丢弃"）；
//! * **`volume_smoothccN`** ／ **`volume_stepccN`**：表行 **Range 空、Unit ＝ `N/A`**
//!   ⇒ 需逐块平滑／步进语义，⛔ 不在"复用 `#104`～`#118` 已立机制"的范围内。
//!   ⭐ 今天：名字落不进四后缀 ⇒ 按**未知 opcode 忽略**（`S114` 钉住）。
//! * ⛔ **`global_volume`／`master_volume`／`group_volume`** 的 CC 调制
//!   （语料 `global_volume_onccN` 一族 **47** 处）：它们是**另外三个**静态 opcode
//!   （[`crate::instrument`] 今天⛔ 不读它们）⇒ 另开单。
//!
//! ### 8.4.1 ⭐ ⭐ 追加修订段（任务单 `#121`）—— 被修订的是 §8.4 第三条里**那 47 处的归属**
//! 以及同口径下四处「`volume_onccN` 只有 **2** 处」的同类登记
//!
//! ⛔ 上面**原文一字未删**（口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋ 点名被修订的支）。
//!
//! **口径（本修订段唯一口径，台账第 274／287 条）**：`assets/samples/**/*.sfz` 逐行剥 `//` 注释
//! ⇒ 取**行首或空白边界**的 opcode token ⇒ token 里 `volume_oncc` 之后的尾巴是**数字**
//! 或 **`$` 宏名** ⇒ ⭐ **收 `$` 宏名**（解析器把 `volume_oncc$F2_DIS` 当本族的一个键读）。
//! **命令** ＝ `cargo test -p yeban-engine --test sfz_instrument s115 -- --nocapture`
//! （判据 `S115` 用同一口径就地重算并逐名打印）；分母
//! `find assets/samples -name '*.sfz' | wc -l` ⇒ **1398**。
//!
//! * ① 原登记「语料 `global_volume_onccN` 一族 **47** 处」 ⇒ ⭐ **实测**：
//!   `global_volume_onccN` ／ `master_volume_onccN` ／ `group_volume_onccN` **各 0 次 ／ 0 文件**；
//!   那 **47** 处**全是 `lfo*_volume_onccN`**（**47** 次 ／ **30** 文件）—— ⛔ 与那三个静态 opcode 无关；
//! * ② 原登记「**47** 次 ／ **32** 文件」里的**文件数** ⇒ ⭐ **实测 30 文件**
//!   （该族在**任何**测过的口径下都是 **30**，⛔ 复现不出 **32**）；
//! * ③ 原登记「`volume_onccN` 只有 **2** 处（值均为 **7**）」（§8.2 的语料见证句，
//!   以及同模块另两处 `///` 文档里的同类句子） ⇒ ⭐ **实测 3 处**：
//!   `volume_oncc24` **2** 处（值均为 **7**）＋ `volume_oncc$F2_DIS` **1** 处（值 **-6**，
//!   `assets/samples/ixox-flute/Ixox Flute.sfz:152`）⇒ 第三处**就是**宏名形式，按本口径**算本族**；
//! * ④ 原登记「家族子串 **49** 次 ／ **32** 文件」 ⇒ ⭐ **实测 50 次 ／ 33 文件**
//!   （＝裸 **3** ＋ 前缀 **47**）。⚠ **②／③／④ 的原登记恰好是"不收 `$` 宏名"口径的读数**
//!   （本轮实测复现：不收宏名 ⇒ **2** 处 ／ 子串 **49** 次 ／ **32** 文件）。
//!
//! ⭐ **被修订的支点名**：§8.4 第三条的**家族名**（`global_volume_onccN` 一族 —— 它其实 **0** 处）、
//! §8.2 的**语料见证句**、`volume_ratio_with_cc` 上方与"非目标写法反向腿"上方那**两条代码注释**
//! （它们写的「语料 47 处」同属错误归属）、以及 `crates/yeban-engine/src/sfz_instrument.rs`
//! `#119` 语料普查表里的**三格**。⭐ **完整逐名展开（11 个名字 ＋ 各自次数／文件数）的唯一事实源**
//! 是那三格上面的 `#121` 修订段。
//!
//! ### 8.5 ⭐ 作用域音量三 opcode（任务单 `#120`）—— ⭐ **规范无 Modulation 表**
//!
//! ⭐ **依据（事实面）**：`global_volume` ／ `master_volume` ／ `group_volume` **三页各只有一张表**
//! —— **静态**表各只有一行（逐字分别是
//! `| global_volume | ARIA | float | 0 | -144 to 6 | dB |`、
//! `| master_volume | ARIA | float | 0 | -144 to 6 | dB |`、
//! `| group_volume | ARIA | float | 0 | -144 to 6 | dB |`），⭐ **无任何 `_onccN` 行**
//! （三页都没有 `Modulation` 表）⇒ `global_volume_onccN` ／ `master_volume_onccN` ／
//! `group_volume_onccN` **三个 opcode 规范无** —— 没有域、没有单位、没有别名句、没有默认值
//! ⇒ **⛔ 没有可实现的契约**（不是"规范里写了而本线漏做"）。现读日期 2026-10-17，三页 URL：
//! <https://sfzformat.com/opcodes/global_volume/> ／ <https://sfzformat.com/opcodes/master_volume/> ／
//! <https://sfzformat.com/opcodes/group_volume/>。⭐ 语料侧同向：三个家族**各 0 次 ／ 0 文件**
//! （口径与命令见上面 §8.4.1）。
//!
//! ⭐ **结构面（另一条独立的依据，与规范无关）**：三页正文逐字写着那是**按 header 生效**的
//! （"affecting everything when set under the <`global`> header"，`master`／`group` 两页同形）
//! —— 而**今天的查询面表达不了"只在某个 header 下有效"**：[`crate::instrument`] 的
//! `Scopes::get`（`crates/yeban-sfz/src/instrument.rs:1550-1555`）是**扁平 first-match**
//! （在 `[region, group, master, global]` 上 `find_map`，**名字里没有 header 维度**）
//! ⇒ 同一个 opcode 名在四个作用域里是**同一个键**，⛔ 无法表达"只在 `<global>` 有效"。
//! ⇒ 这件事的**前置件**是一个**新的按 header 限定的查询 API**：登记为 ⭐ **需要新面**
//! —— ⛔ 这不是"本线漏了一个实现"（规范根本没有那三个 opcode 的定义；即便按 ARIA 语义实现，
//! 也得先有这个 header 维度的查询面）。
//!
//! ### 8.6 ⭐⭐ 追加修订段（任务单 `#124`）—— 被修订的是 §8.4 **第一条**（`gain_ccN` 那一支）
//!
//! ⛔ §8.4 第一条的原文**一字未删**（口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋
//! **点名被修订的是哪一支**）。**被修订的支** ＝ §8.4 第一条里
//! 「**`gain_ccN`** … ⛔ **本单不识别**」那一句 ＋ `parse_modulation_name` 里
//! 「⚠ 规范 Modulation 表逐字的 delta 行名 `gain_ccN` ⛔ 本单不识别（§8.4）。」那条代码注释
//! ＋ 单元测试反向腿表里的 `"gain_cc1"` 那一项 ＋ 判据 `S114` ⑧ 反向腿表里的 `"gain_cc1"` 那一项。
//!
//! #### 8.6.1 ⭐ 归属裁定：`gain_ccN` 归 **同一个** [`ModTarget::Volume`]（⛔ 不新开目标）
//!
//! ⭐ **裁定：`gain_ccN` 是 `volume` 族的**逐字 delta 行名**，归 [`ModTarget::Volume`]
//! —— ⛔ **不新开 `ModTarget`、⛔ 不新开 `ModUnit`**。四条理由：
//!
//! 1. **规范自己说两者是同一个 opcode**：`volume` 页正文逐字
//!    「``volume_ccN`` is a valid alias for ``gain_ccN``」＋「although the specification
//!    originally stated ``gain_ccN`` for adjusting volume using CCs」⇒ `gain_ccN` 与
//!    `volume_ccN` 是**同一行**的两种拼写（口径同台账第 285 条、同 §7.2 的 `pitch`≡`tune`）；
//! 2. **静态基准已经这么判了**：[`crate::instrument`] 读基准音量时逐字是
//!    `scopes.get("volume").or_else(|| scopes.get("gain"))`
//!    （`crates/yeban-sfz/src/instrument.rs:1730`）⇒ **`gain` 与 `volume` 本来就是同一个
//!    静态 opcode 的两个名字** ⇒ 它们的 `*ccN` 派生名落同一个目标，是那条既有事实的机械后果；
//! 3. **域与单位**：δ 的 Unit ＝ **dB**（＝既有 [`ModUnit::Decibels`]）、Range ＝
//!    **`-144 to 48`**（＝既有 [`ModTarget::delta_range`]）⇒ 新支**一个数都不用新定**；
//! 4. **代价最小的形状**：同一目标 ⇒ ⛔ 不加变体、⛔ 不加每声部／每区域字段
//!    ⇒ `ModTarget::COUNT` 仍 **8** ／ `SfzVoice` ／ `SfzVoicePool` ／ `EngineRuntime` ／
//!    `PreparedRegion` **四个全 ＋0**（判据 `S117` 现读）。
//!
//! ⚠ **代价（如实登记）**：`gain_ccN` 与 `volume_ccN` 在同一作用域同一 CC 上同时出现时归
//! **一个键**、**不求和**（与 §8.2 末尾登记的别名坍塌**同形**，⛔ 不是新增代价）。
//! ⚠ **⛔ 不许改变既有 `volume_*ccN` 的读数**：新支只接**以 `gain` 开头的名字**，与
//! `volume` 前缀不相交 ⇒ 既有四拼写的识别集、域、单位与位模式**逐字未动**（判据 `S117` 的
//! 别名见证腿：同一 CC 下 `gain_cc1` 与 `volume_oncc1` 的乘子**逐位相同**）。
//!
//! #### 8.6.2 ⛔ 本单**只**收 `gain_ccN` 一个拼写（同族申请被驳回的三条）
//!
//! ⭐ **裁定：只识别 `gain_ccN`**（`gain` ＋ `_cc` ＋ 数字）。⛔ **不识别** `gain_onccN`、
//! ⛔ `gain_curveccN`、⛔ `gainccN`（无下划线形）、⛔ `gain_smoothccN`／`gain_stepccN`。
//! 依据（**规范侧**，8 档实测见 §8.6.3）：
//!
//! * `gain_ccN` 页 Modulation 表**只有这一行**以 `gain` 开头：
//!   `| \`gain_ccN\` |  | float | **0** | **-144 to 48** | **dB** |`
//!   —— ⛔ 表里**没有** `gain_onccN`、⛔ **没有** `gain_curveccN`、⛔ **没有** `gain_smoothccN`、
//!   ⛔ **没有** `gain_stepccN`（⛔ `#119`／`#120` 两次"我以为有那一行、实际没有"的教训）；
//! * 正文那句把扩展拼写**限定在 `volume` 上**：逐字
//!   「Extended opcodes, such as ``volume_curveccN`` shown above, only apply to ``volume``」
//!   ⇒ `_curvecc`／`_smoothcc`／`_stepcc` 的 `gain` 拼写**规范里不存在**；
//! * `gain_onccN` 是**外推**（ARIA 的通用 `_oncc` 形）：规范**一个字都没写**它
//!   ⇒ 接它就＝**发明语义**（`AGENTS.md` §5.7）；⭐ 语料也没有约束力（**0** 次，见 §8.6.3）；
//! * `gainccN`（无下划线）：⛔ 规范里的 delta 行名**逐字带下划线**；`volumeccN` 的既有识别是
//!   `volume` 拼写的**既成事实**，⛔ **不构成**给 `gain` 发明新拼写的理由（第 288 条的反面）。
//!
//! ⚠ **代价（如实登记）**：`gain_onccN`／`gain_curveccN`／`gainccN` 今天仍按**未知 opcode
//! 忽略**（与接线前**逐字相同**、⛔ 不报错、⛔ 不静默吃进表）—— 判据 `S117` 把这三格
//! **逐字钉住**（⛔ 不是"哪天变成识别到了但静默丢弃"）；其中前两者**若**将来规范补行，
//! 追加它们只需在**新支**里加一个后缀分支（位置与形状见 §8.6.1 的落地）。
//!
//! #### 8.6.3 ⭐ 语料实测（分母 **1398** 份 `assets/samples/**/*.sfz`；第 287／291 条的三元组口径）
//!
//! ⭐ **三元组口径** ＝ **剥不剥 `//` 注释** × **左边界**（任意位置 ／ **行首或空白**）
//! × **收不收 `$` 宏名**。⭐ **8 档全部同值**（下表每一档都逐字复现），故只列一个数：
//!
//! | 档 | 剥注释 | 左边界 | 收 `$` 宏名 | 裸 `gain_ccN` | 裸 `gain_onccN` | 裸 `gain_curveccN` | 本族之外总数 |
//! | :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
//! | ① | ⛔ 不剥 | 任意 | 收 | **0** | **0** | **0** | **43** |
//! | ② | ⛔ 不剥 | 任意 | ⛔ 不收 | **0** | **0** | **0** | **43** |
//! | ③ | ⛔ 不剥 | 行首/空白 | 收 | **0** | **0** | **0** | **43** |
//! | ④ | ⛔ 不剥 | 行首/空白 | ⛔ 不收 | **0** | **0** | **0** | **43** |
//! | ⑤ | 剥 `//` | 任意 | 收 | **0** | **0** | **0** | **43** |
//! | ⑥ | 剥 `//` | 任意 | ⛔ 不收 | **0** | **0** | **0** | **43** |
//! | ⑦ | 剥 `//` | 行首/空白 | 收 | **0** | **0** | **0** | **43** |
//! | ⑧ | 剥 `//` | 行首/空白 | ⛔ 不收 | **0** | **0** | **0** | **43** |
//!
//! ⭐ **匹配 `gain_cc`／`gain_oncc`／`gain_curvecc` 的完整 opcode 名逐名展开**（8 档同表；
//! 口径 ＝ ⑦）—— ⛔ **裸档三项各 0**，那 **43** 次全在**别的目标**上：
//!
//! | 完整 opcode 名 | 次数 | 文件数 |
//! | :--- | :--- | :--- |
//! | `eq1_gain_oncc77` | **23** | **21** |
//! | `lfo01_eq1gain_oncc114` | **7** | **7** |
//! | `lfo02_eq1gain_oncc115` | **4** | **4** |
//! | `lfo01_eq1gain_oncc21` | **3** | **3** |
//! | `eq1_gain_oncc1` | **2** | **2** |
//! | `eq2_gain_oncc1` | **2** | **2** |
//! | `eq3_gain_oncc1` | **2** | **2** |
//! | （合计 ＝ 本族之外） | **43** | **35** |
//!
//! ⚠ **与 `#119` 登记的异同（第 287／289 条）**：⭐ **同** ＝ 裸 `gain_ccN` **0** 处、
//! 裸 `gain_onccN` **0** 处（§8.4 的登记**复核成立**）；⭐ **异** ＝ §8.4 写「那 **14** 处
//! `gain_onccN` 全是 `lfo*_eq1gain_onccN`」—— **14** 只是**其中一族**：
//! `lfo*_eq1gain_onccN` 确为 **14** 次 ／ **14** 文件 ✓，但另有 `eq*_gain_onccN` 一族
//! **29** 次 ／ **23** 文件（`eq1_gain_oncc77` 23 ＋ `eq1_gain_oncc1` 2 ＋
//! `eq2_gain_oncc1` 2 ＋ `eq3_gain_oncc1` 2，逐名见上表）⇒ `gain_oncc` **合计 43 次 ／
//! 35 文件** ⇒ §8.4 的「14」是**漏计**（它只数了带 `lfo*` 前缀的那些，
//! `eq*_` 前缀那些的左边界是 `_`）。
//! ⭐ **复现不出的如实写（第 289 条）**：**`43`** 这个总数在 `#119` 的登记里**没有**，
//! 本线也**复现不出**任何"`gain_oncc` 只有 14 处"的口径（左边界取"行首或空白"时是 **0**、
//! 取"任意位置"时是 **43**）⇒ 本条只登记 **0**（裸档）与 **43**（本族之外）**两个**读数。
//!
//! # 实时安全
//!
//! [`ampeg_value_with_baked`] ／ [`ampeg_value_with_baked_and_vel2`] 与
//! [`BakedCurve::value_at`] 是**纯算术 ＋ 切片索引**：
//! 无分配、无锁、无 I/O、无日志，可在实时路径调用。逐位可复现：只有 `+ - * /` 与比较
//! （插值实现是 [`crate::curve::curve_value_at`] **同一份**，⛔ 没有第二套）。

use crate::curve::{Curve, CurvePoint, curve_value_at};

/// `ampeg_*` 参数里可被 CC 调制的那一个。
///
/// 声明顺序就是**求和顺序**（见模块文档第 5 条裁决）：`Attack < Hold < Decay < Sustain
/// < Release`。与 [`crate::instrument::Region`] 上五个字段一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AmpegParam {
    /// `ampeg_attack`（秒）。
    Attack,
    /// `ampeg_hold`（秒）。
    Hold,
    /// `ampeg_decay`（秒）。
    Decay,
    /// `ampeg_sustain`（百分比）。
    Sustain,
    /// `ampeg_release`（秒）。
    Release,
}

impl AmpegParam {
    /// 参数个数（`[T; AmpegParam::COUNT]` 的下标域 ＝ `0..COUNT`）。
    pub const COUNT: usize = 5;

    /// 全部五个参数（声明序；判据与遍历用）。
    pub const ALL: [Self; 5] = [
        Self::Attack,
        Self::Hold,
        Self::Decay,
        Self::Sustain,
        Self::Release,
    ];

    /// 在 [`Self::ALL`] 里的下标（`0..=4`）。
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Attack => 0,
            Self::Hold => 1,
            Self::Decay => 2,
            Self::Sustain => 3,
            Self::Release => 4,
        }
    }

    /// 不带 `ampeg_` 前缀的参数名（诊断文案用）。
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Attack => "attack",
            Self::Hold => "hold",
            Self::Decay => "decay",
            Self::Sustain => "sustain",
            Self::Release => "release",
        }
    }

    /// 该参数是不是**时间**（秒）；`false` ⇒ 百分比（只有 `sustain`）。
    #[must_use]
    pub const fn is_time(self) -> bool {
        !matches!(self, Self::Sustain)
    }

    /// 按 [`Self::ALL`] 的顺序取第 `index` 个（越界 ⇒ `None`）。
    #[must_use]
    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Attack),
            1 => Some(Self::Hold),
            2 => Some(Self::Decay),
            3 => Some(Self::Sustain),
            4 => Some(Self::Release),
            _ => None,
        }
    }
}

/// ⭐⭐ `delta` 的**单位类别**（任务单 `#117`）：把 `#104` 的 `is_time()` **布尔**
/// 显式化成一个枚举。
///
/// ⚠ **为什么要有它**：`#104` 只区分"时间／非时间"，而非时间那一支把
/// **百分比**（`ampeg_sustain` ／ `amplitude`）与**音分**（`tune`）混在一起
/// ⇒ 后缀政策只能靠"不是时间就剥 `%`"实现，于是 `tune_oncc1=50%` 被**静默**
/// 读成 `50` 音分（`#116` 如实登记的"过度宽松"）。本枚举把三者分开，
/// 让**后缀接受集**成为单位类别的一个函数。
///
/// ⛔ **它不改任何既有接受集**：`Seconds` 仍只认 `s` / `ms`、`Percent` 仍只认 `%`
/// —— 两支的行为**逐字不变**；被收窄的只有**新增的** `Cents` 一支。
///
/// ⚠ `ampeg_sustain` 是 `Percent`（表行 Unit ＝ `%`）、`amplitude` 是 `Percent`
/// （表行 Unit ＝ `%`）、`tune` 是 `Cents`（表行 Unit ＝ `cents`）—— 逐字出处见
/// 本模块 §2／§5.1／§6.1。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ModUnit {
    /// **秒**（`ampeg_attack` / `hold` / `decay` / `release`）：认 `s` / `ms` 后缀。
    Seconds,
    /// **百分比**（`ampeg_sustain` ／ `amplitude`）：认 `%` 后缀。
    Percent,
    /// ⭐⭐ **音分**（`tune`）：⛔ **不认任何后缀** —— 规范里 `cents` 没有后缀写法，
    /// 尤其 ⛔ 没有 `%`（见本模块 §6.1 的逐字表行与别名句）。
    Cents,
    /// ⭐⭐ **分贝**（`volume`；任务单 `#119`）：⛔ **不认任何后缀** —— 规范里 `dB`
    /// 一族只写纯数字（`volume_oncc24=7`），⛔ 没有 `%`／`s`／`ms`／`dB` 写法。
    ///
    /// ⚠ **这是第四个**单位类别，`#117` 的三支**逐字未动**（见本模块 §8.2 的裁定、
    /// 理由与代价）。⚠ 它对 [`ModTarget::is_time`] 返回 `false` ⇒ `#104` 的
    /// `is_time()` 契约对**七个既有目标**逐字不变。
    Decibels,
}

impl ModUnit {
    /// 该单位类别**接受的后缀**（诊断与判据文案用；空串 ＝ 无后缀）。
    ///
    /// ⚠ 它是**唯一**一处把"单位类别 → 后缀集"写下来的地方
    /// （[`crate::instrument::read_modulation_delta`] 的分支就是它的机械后果）。
    #[must_use]
    pub const fn suffixes(self) -> &'static [&'static str] {
        match self {
            Self::Seconds => &["", "s", "ms"],
            Self::Percent => &["", "%"],
            Self::Cents => &[""],
            // ⭐⭐ `#119`：`dB` 一族与 `cents` 同款 —— ⛔ 一个后缀都不认
            // （规范里没有 `volume_oncc1=6dB` 这种写法）。
            Self::Decibels => &[""],
        }
    }
}

/// ⭐ **调制目标**（任务单 `#115`：把 `#104` 写死的 `AmpegParam` 推广成"目标"）。
///
/// `#104` 的机制（名字识别 → 归约 → 求值）本来就是"**对某个目标**施加
/// `base + Σ delta × curve(cc)`"；唯一写死的地方是**目标的类型**是 [`AmpegParam`]。
/// 本条把那一层抽象出来：目标可以是五个 `ampeg_*` 参数之一，也可以是 ⭐ **增益族**的
/// 第一个目标 [`Self::Amplitude`]（`amplitude` ／ `amplitude_ccN` ／ `amplitude_onccN` ／
/// `amplitude_curveccN`）。
///
/// ⚠ **形状裁决与代价**：用**一个带载荷的枚举**，而不是"给每一族各开一张表、各写一套公式"——
/// ① 归约仍是**一张** `BTreeMap<(目标, cc), …>` ⇒ 求和顺序唯一
/// （[`Self::index`] 的声明序 × CC 升序，[ARCH-DET-001]）；
/// ② 摊平表仍是**一个** `[BakedCcMod;` [`AMPEG_MOD_CAP`]`]` ⇒ `PreparedRegion`
/// **不因推广而长大**（`AmpegParam`(1 B) 与 `ModTarget`(≤2 B) 落在既有对齐空隙里，
/// 由判据 `S100` 现读钉死）；
/// ③ ⛔ 没有并行机制、⛔ 没有第二份公式。
///
/// 声明顺序**就是求和顺序**：五个 `ampeg_*` 在前（`#104`／`#106` 的顺序**逐位不变**），
/// [`Self::Amplitude`] 在**末尾**（新增目标永远加在最后 ⇒ 既有目标的位模式不受影响）。
///
/// ⭐ ⭐ **追加修订段（任务单 `#116`）—— 被修订的是上面这句里"[`Self::Amplitude`] 在
/// **末尾**"那一支**（⛔ 原文一字未删，口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋
/// 点名被修订的支）：`#116` 把**第二个**目标 [`Self::Tune`]（`tune` ／ `tune_ccN` ／
/// `tune_onccN` ／ `tune_curveccN`）加在 [`Self::Amplitude`] **之后** ⇒ "末尾"现在是
/// `Tune`。⭐ **被修订的只是"末尾"这个位置**：五个 `ampeg_*`（下标 `0..=4`）与
/// [`Self::Amplitude`]（下标 `5`）的下标**一个都没动** ⇒ 既有六个目标的求和序与
/// 位模式**逐位不变**（判据 `S106`／`S110` 现读）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ModTarget {
    /// 五个 `ampeg_*` 参数之一（`#104` 的原始目标、`#106` 的力度支）。
    Ampeg(AmpegParam),
    /// ⭐ `amplitude`（**线性**百分比；任务单 `#115` 的第一个增益族目标）。
    Amplitude,
    /// ⭐⭐ `tune`（**音分**；任务单 `#116` 的第二个被推广目标）。
    ///
    /// ⚠ ⭐ **沿革（任务单 `#116`）—— 被修订的是上面那句"新增目标永远加在最后"
    /// 里"最后"的位置**（⛔ 原文一字未删，口径同台账第 278 条：保留原文 ＋ 追加修订段
    /// ＋ 点名被修订的支）：`#115` 时"最后"是 [`Self::Amplitude`]（下标 5），`#116` 起
    /// "最后"是**本变体**（下标 6）⇒ **既有六个目标的下标一个都没动**（求和序逐位不变，
    /// 判据 `S106` 现读 `All[5] == Amplitude` 与 `All[6] == Tune`）。
    ///
    /// ⚠ **它承载的是"调制量 `delta`"，单位 ＝ 音分**（⛔ 不是基准 `tune`）：
    /// 基准 `tune` 早就烘进 `Region::pitch_ratio` / 摊平表的 `rate_per_note`，
    /// 本目标只贡献**逐块可变的增量**（见 [`tune_ratio_with_baked`]）。
    ///
    /// ⭐ ⭐ **追加修订段（任务单 `#118`）—— 被修订的是上面那句里"`tune`"这个目标名
    /// 所涵盖的**拼写集**那一支**（⛔ 原文一字未删，口径同台账第 278 条：保留原文 ＋
    /// 追加修订段 ＋ 点名被修订的支）：`#116` 时本目标只由 **`tune<suffix>`** 四族识别
    /// 到达；`#118` 起**`pitch<suffix>`**（`pitch_onccN` / `pitch_ccN` / `pitch_curveccN`）
    /// 也归到**本变体**（规范那句 "In ARIA, `tune_*ccN` can also be used as an alias
    /// for `pitch_*ccN`." 的机械后果，裁定见模块文档 §7.2）。
    /// ⭐ **被修订的只是"谁能到达本变体"**：本变体的**下标（6）**、`name()`（`"tune"`）、
    /// `mod_unit()`（`Cents`）与 `delta_range()`（`±9600`）**逐字未变**
    /// ⇒ `S106`／`S107`／`S111` 的既有读数与位模式**逐位不变**。
    Tune,
    /// ⭐⭐ `volume`（**分贝**；任务单 `#119` 的第三个被推广目标，也是**第三个**
    /// 单位类别 [`ModUnit::Decibels`]）。
    ///
    /// ⚠ ⭐ **沿革（任务单 `#119`）—— 被修订的是 [`Self::Tune`] 上面那段里
    /// 「⛔ 本单**不发明** `pitch=`」所在的**"末尾"位置**那一支**
    /// （⛔ 原文一字未删，口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋
    /// 点名被修订的支）：`#118` 起"末尾"是 [`Self::Tune`]（下标 6），`#119` 起
    /// "末尾"是**本变体**（下标 7）⇒ **既有七个目标的下标一个都没动**
    /// （求和序逐位不变，判据 `S106` 现读 `ALL[5] == Amplitude`、
    /// `ALL[6] == Tune` 与 `S114` 现读 `ALL[7] == Volume`）。
    ///
    /// ⚠ **它承载的是"调制量 `delta`"，单位 ＝ dB**（⛔ 不是基准 `volume`）：
    /// 基准 `volume`（dB）早就烘进 [`crate::Region::linear_gain`]（摊平表的
    /// `volume_gain`）与 [`crate::playback::PlaybackSpec::volume_db`]（离线
    /// `total_gain()`），本目标只贡献**逐块可变的增量**（见
    /// [`volume_ratio_with_baked`]）。
    ///
    /// ⚠ 本变体**只由 `volume<suffix>` 四族**到达（`volumeccN` / `volume_ccN` /
    /// `volume_onccN` / `volume_curveccN`）；⛔ **`gain_ccN` 那一支本单不识别**
    /// （规范逐字的 delta 行名是 `gain_ccN`，裁定与理由见本模块 §8.4）。
    ///
    /// ⚠ ⭐ **追加修订段（任务单 `#124`）—— 被修订的是上一段末尾「⛔ `gain_ccN` 那一支本单
    /// 不识别」那一支**（⛔ 原文一字未删，口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋
    /// 点名被修订的支）：`#124` 起本变体**还由 `gain_ccN` 一个拼写到达**（第**五**个到达者，
    /// 与那四族落**同一个键**；⛔ 不新开目标、⛔ 不新开单位类别 ⇒ `COUNT` 仍 **8**、
    /// 四个尺寸读数**全 ＋0**）。⭐ **被修订的只有"谁能到达本变体"**：本变体的**下标（7）**、
    /// `name()`（`"volume"`）、`mod_unit()`（`Decibels`）与 `delta_range()`（`-144..=48`）
    /// **逐字未变** ⇒ `S114`／`S115`／`S116` 的既有读数与位模式**逐位不变**
    /// （`S114` ⑧ 里 `"gain_cc1"` 那一项按第 278 条改由 `S117` 用**正例**钉住，
    /// `gain_oncc1`／`gain_curvecc1` 两格**逐字仍钉在"未知 opcode 忽略"**；
    /// 裁定、代价与 8 档语料口径见 §8.6）。
    Volume,
}

impl ModTarget {
    /// 目标个数（[`Self::ALL`] 的长度）。
    ///
    /// ⭐ **沿革（任务单 `#119`）**：`#115` 登记 `6`、`#116`／`#118` 登记 `7`
    /// （后两者⛔ 不加变体）。`#119` 加上 [`Self::Volume`] ⇒ **`8`**。
    /// ⚠ 被修订的只是**这个数**：既有七个目标的**下标与位模式逐位不变**。
    pub const COUNT: usize = 8;

    /// 全部八个目标（声明序；判据与遍历用）。
    pub const ALL: [Self; 8] = [
        Self::Ampeg(AmpegParam::Attack),
        Self::Ampeg(AmpegParam::Hold),
        Self::Ampeg(AmpegParam::Decay),
        Self::Ampeg(AmpegParam::Sustain),
        Self::Ampeg(AmpegParam::Release),
        Self::Amplitude,
        Self::Tune,
        // ⭐⭐ `#119` 的**新增目标**（⛔ 加在末尾 ⇒ 既有七个下标一个都不动）。
        Self::Volume,
    ];

    /// 在 [`Self::ALL`] 里的下标（`0..=7`）；**这个序就是求和序**。
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Ampeg(param) => param.index(),
            Self::Amplitude => AmpegParam::COUNT,
            Self::Tune => AmpegParam::COUNT + 1,
            Self::Volume => AmpegParam::COUNT + 2,
        }
    }

    /// 规范里的目标名（诊断与判据文案用）。
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Ampeg(AmpegParam::Attack) => "ampeg_attack",
            Self::Ampeg(AmpegParam::Hold) => "ampeg_hold",
            Self::Ampeg(AmpegParam::Decay) => "ampeg_decay",
            Self::Ampeg(AmpegParam::Sustain) => "ampeg_sustain",
            Self::Ampeg(AmpegParam::Release) => "ampeg_release",
            Self::Amplitude => "amplitude",
            Self::Tune => "tune",
            Self::Volume => "volume",
        }
    }

    /// 目标是不是 `ampeg_*` 那一族；`Some(param)` ⇒ 该参数（力度支只服务这一族）。
    #[must_use]
    pub const fn ampeg(self) -> Option<AmpegParam> {
        match self {
            Self::Ampeg(param) => Some(param),
            Self::Amplitude | Self::Tune | Self::Volume => None,
        }
    }

    /// ⭐⭐ `delta` 的**单位类别**（任务单 `#117`）—— **单位政策的唯一来源**。
    ///
    /// ⚠ **`is_time()` 与 `unit()` 都从它派生**（两者对既有目标逐字返回同一结果）
    /// ⇒ ⛔ 没有第二张表、⛔ 没有第二个真相；后缀接受集
    /// （[`ModUnit::suffixes`]）是它的机械后果。
    ///
    /// ⭐⭐ **追加修订段（任务单 `#117`）—— 被修订的是上面 `unit()` 文档里
    /// 「换算由各目标的消费方做」那一支里**"消费方"的名单**（⛔ 原文一字未删，
    /// 口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋ 点名被修订的支）：
    /// `#104`～`#116` 的消费方只有**域检查**与**数值求值**；`#117` 起
    /// [`crate::instrument::read_modulation_delta`] 的**后缀分支**也读它
    /// ⇒ `tune`（`Cents`）与 `amplitude`／`ampeg_sustain`（`Percent`）在**读取**层
    /// 就分了道（⛔ 不再是"都是非时间 ⇒ 都剥 `%`"）。
    #[must_use]
    pub const fn mod_unit(self) -> ModUnit {
        match self {
            Self::Ampeg(param) => {
                if param.is_time() {
                    ModUnit::Seconds
                } else {
                    ModUnit::Percent
                }
            }
            Self::Amplitude => ModUnit::Percent,
            Self::Tune => ModUnit::Cents,
            // ⭐⭐ `#119`：**第四个**单位类别（`volume` 的表行 Unit ＝ dB）。
            // ⚠ `#117` 的三支**逐字未动**（见本模块 §8.2）。
            Self::Volume => ModUnit::Decibels,
        }
    }

    /// `delta` 的单位是不是**时间**（秒）；`false` ⇒ 百分比或音分。
    ///
    /// 五个 `ampeg_*` 里只有 `sustain` 是百分比；⭐ [`Self::Amplitude`] 的
    /// `amplitude_onccN` 表行 Unit ＝ **%**（逐字表行见模块文档 §5）⇒ `false`；
    /// ⭐⭐ [`Self::Tune`] 的 `tune_*ccN` Unit ＝ **cents**（逐字表行与裁定见模块文档 §6）
    /// ⇒ `false`（⛔ 不是"默认百分比"—— 调用方读 [`Self::mod_unit`] 取真单位）。
    ///
    /// ⭐⭐ **追加修订段（任务单 `#117`）—— 被修订的是上面那句里**
    /// 「调用方读 [`Self::unit`] 取真单位」**那一支**（⛔ 原文一字未删，口径同台账
    /// 第 278 条：保留原文 ＋ 追加修订段 ＋ 点名被修订的支）：`#104`～`#116` 的
    /// **后缀政策**读的是本函数（布尔）；`#117` 起改读 [`Self::mod_unit`]
    /// ⭐ **被修订的只是"后缀政策读谁"这件事** —— 本函数的**返回值对七个目标
    /// 一个都没变**（`Seconds` ⇒ `true`、`Percent`／`Cents` ⇒ `false`）
    /// ⇒ 既有判据（`S100`／`S106`／`S101`／`S107`）逐字仍绿。
    #[must_use]
    pub const fn is_time(self) -> bool {
        matches!(self.mod_unit(), ModUnit::Seconds)
    }

    /// ⭐⭐ `delta` 的**单位名**（规范表行 Unit 列的字面；诊断与判据文案用）。
    ///
    /// ⚠ 它⛔ 不参与任何换算 —— 换算由各目标的消费方做
    /// （`ampeg_*`／`amplitude` 见模块文档 §2／§5，`tune` 见 §6）。
    /// ⭐⭐ `#117` 起它由 [`Self::mod_unit`] **派生**（⛔ 不是第二张表）。
    #[must_use]
    pub const fn unit(self) -> &'static str {
        match self.mod_unit() {
            ModUnit::Seconds => "seconds",
            ModUnit::Percent => "%",
            ModUnit::Cents => "cents",
            // ⭐⭐ `#119`：`volume` 页表行 Unit 列的字面就是 `dB`。
            ModUnit::Decibels => "dB",
        }
    }

    /// ⭐⭐ `delta` 的**取值域**（下界，上界），**逐目标不同**。
    ///
    /// * [`Self::Ampeg`] ／ [`Self::Amplitude`] ⇒ `-100..=100`
    ///   （各 opcode 页 Modulation 表 Range 列，见模块文档裁决 2 与 §5.1）；
    /// * ⭐⭐ [`Self::Tune`] ⇒ **`-9600..=9600` 音分**（模块文档 §6 的裁定：
    ///   `tune_*ccN` 在规范里是 `pitch_onccN` 的 ARIA 别名，而 `pitch_onccN` 表行
    ///   Range ＝ `-9600 to 9600`、Unit ＝ cents；`tune` 页自己的例子
    ///   `pitch_oncc27=200` 也**超出** `tune` 自身的 `±100`）；
    /// * ⭐⭐ [`Self::Volume`] ⇒ **`-144..=48` dB**（模块文档 §8.2 的裁定：
    ///   `volume` 页的 Modulation 表**没有 `volume_onccN` 行**，delta 行是
    ///   `gain_ccN`（Range ＝ `-144 to 48`、Unit ＝ dB），而正文写
    ///   "`volume_ccN` is a valid alias for `gain_ccN`" ⇒ 域随被别名的那一行 ——
    ///   与 `Tune` 走的是**同一条口径**）。
    ///
    /// ⚠ 调用方 [`crate::instrument::read_modulation_delta`] 是**唯一**读它的地方
    /// ⇒ ⛔ 没有第二套域检查、⛔ 没有第二对常数被绕过。
    #[must_use]
    pub const fn delta_range(self) -> (f32, f32) {
        match self {
            Self::Ampeg(_) | Self::Amplitude => (MIN_MODULATION_DELTA, MAX_MODULATION_DELTA),
            Self::Tune => (MIN_TUNE_MODULATION_DELTA, MAX_TUNE_MODULATION_DELTA),
            Self::Volume => (MIN_VOLUME_MODULATION_DELTA, MAX_VOLUME_MODULATION_DELTA),
        }
    }
}

/// CC 号上界（**容器界**：既有探针是 `&dyn Fn(u8) -> u8`）。见模块文档裁决 1。
pub const MAX_MODULATION_CC: u16 = u8::MAX as u16;

/// `_curveccN` 取值的上界（`ampeg_attackccN` 页 Modulation 表 Range = `0 to 255`）。
pub const MAX_MODULATION_CURVE_INDEX: u16 = u8::MAX as u16;

/// `delta` 的下界（各 opcode 页 Modulation 表 Range = `-100 to 100`）。
pub const MIN_MODULATION_DELTA: f32 = -100.0;

/// `delta` 的上界（同上）。
pub const MAX_MODULATION_DELTA: f32 = 100.0;

/// ⭐⭐ `tune_*ccN` 的 `delta` 下界（**音分**；任务单 `#116`）。
///
/// 出处（逐字，模块文档 §6）：`tune` 页的 Modulation 表**没有** `tune_onccN` 行，
/// 它列的是 `pitch_onccN`（Range ＝ **`-9600 to 9600`**、Unit ＝ **cents**），
/// 而同页正文写「In ARIA, `tune_*ccN` can also be used as an alias for `pitch_*ccN`.」
/// ⇒ `tune` 族的 `delta` 域随被别名的那一行取 **`±9600` 音分**。
/// ⚠ 独立见证：`tune` 页自己的例子 `tune=-100` ＋ `pitch_oncc27=200` 里的 `200`
/// **超出** `tune` 自身的 `-100 to 100` ⇒ `tune` 的基准域**不是**调制量的域。
/// ⚠ 语料见证：登记语料里 `tune_*ccN` 的 `|delta| > 100` 有 **599** 处、最大 **2400** 音分
/// ⇒ 若沿用 `±100` 会把今天**能解析**的 599 处变成不可解析。
pub const MIN_TUNE_MODULATION_DELTA: f32 = -9600.0;

/// ⭐⭐ `tune_*ccN` 的 `delta` 上界（`+9600` 音分；依据同 [`MIN_TUNE_MODULATION_DELTA`]）。
pub const MAX_TUNE_MODULATION_DELTA: f32 = 9600.0;

/// ⭐⭐ `volume_*ccN` 的 `delta` 下界（**dB**；任务单 `#119`）。
///
/// 出处（逐字，模块文档 §8.1／§8.2）：`volume` 页的 Modulation 表**没有**
/// `volume_onccN` 行，delta 行逐字是
/// `| gain_ccN |  | float | 0 | -144 to 48 | dB |`，而同页正文写
/// 「**`volume_ccN` is a valid alias for `gain_ccN`**」⇒ `volume` 族的 `delta` 域
/// 随被别名的那一行取 **`-144` dB**（与 `tune` 随 `pitch_onccN` 的 `±9600`
/// 是**同一条口径**）。
/// ⚠ 语料见证：登记语料 `volume_onccN` 只有 **2** 处（值均 **7**）、`volume_ccN`
/// **0** 处 ⇒ 域两个端点都**没有**语料约束力，本题只能由规范表行回答（⛔ 不发明 `±100`）。
///
/// ⭐ ⭐ **追加修订段（任务单 `#125`）—— 被修订的是上面那句里的「只有 **2** 处」**（⛔ 原文
/// 一字未删，口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋ 点名被修订的支）：同 `#121` 裁的
/// 唯一口径（剥 `//` 注释 ＋ 行首/空白边界 opcode token ＋ **收 `$` 宏名**）⭐ **实测 3 处**：
/// `volume_oncc24` **2** 处（值均 **7**）＋ `volume_oncc$F2_DIS` **1** 处（值 **-6**，
/// `assets/samples/ixox-flute/Ixox Flute.sfz:152`）⇒ 「域两个端点都**没有**语料约束力」**仍成立**
/// （第三处是**宏名**形式，它的值 **-6** 落在 `-144..=48` 内、⛔ 不贴任一端点）。
/// 依据命令 ＝ `cargo test -p yeban-engine --test sfz_instrument s115 -- --nocapture`。
pub const MIN_VOLUME_MODULATION_DELTA: f32 = -144.0;

/// ⭐⭐ `volume_*ccN` 的 `delta` 上界（`+48` dB；依据同
/// [`MIN_VOLUME_MODULATION_DELTA`]）。
pub const MAX_VOLUME_MODULATION_DELTA: f32 = 48.0;

/// ⭐⭐ `volume` 调制的**唯一换算**（任务单 `#119`）：dB 增量 → 线性增益乘子。
///
/// ```text
/// volume_delta_db = 0 + Σ_{cc} delta × curve(probe(cc))      // 单位：dB
/// volume_ratio    = 2 ^ (volume_delta_db × log2(10) / 20)    // ＝ 10 ^ (dB / 20)
/// gain            = gain_base × volume_ratio                 // gain_base 已含基准 volume
/// ```
///
/// ⚠ **它用的是 `2^x` 而不是 `10^x`**：`log2(10) / 20` 是**同一个**换算的等价写法，
/// 而 `exp2` 是本 crate 已经在用的超越函数（[`tune_ratio_with_baked`]）——
/// ⛔ 没有第二个超越函数入口。⚠ `Region::linear_gain()` 用的是**同一个**代数式
/// （`(volume × LOG2_10 / 20).exp2()`，f64 中间量 + `as f32`）；本函数是它的
/// `f32` 形态（调制量在 `f32` 上求和 ⇒ 换算走 `f32`，与 [`tune_ratio_with_baked`] 同款）。
///
/// * **基准 `volume` 不在本函数里**：它早就烘进 [`crate::Region::linear_gain`]
///   （摊平表的 `volume_gain`）／[`crate::playback::PlaybackSpec::volume_db`]
///   （离线 `total_gain()`）⇒ 本函数只贡献**增量**（`base = 0.0`）；
/// * `probe == None` ⇒ **一项 CC 都不读**、直接返回 `1.0`（⇒ `gain × 1.0` 在 binary32
///   下**逐位不变**，与接线前逐位相同）；
/// * `probe == Some` 且**没有** `volume` 项 ⇒ `cc_value_with_baked` 返回 `0.0`
///   ⇒ `0.0f32.exp2()` 恰为 `1.0`（IEEE：`exp2(±0)` ＝ `1`）⇒ 同样逐位不变；
/// * 曲线不可烘焙的项**不贡献** dB（⛔ 不当成曲线 0），与 [`cc_value_with_baked`] 同口径；
/// * ⛔ **不钳位**：规范只给了 `delta` 的域（模块文档 §8.2）⇒ 本函数沿用。
///   ⚠ 如实登记的边界：多项代数相加后 `2^x` 仍可能上溢成 `Inf`
///   （`x > 128` 即 `|Σ delta| > 1605` dB）或下溢成 `0`（`x < −149`）——
///   消费方按**既有**守卫如实退休，⛔ 不静默改成 `1.0`（判据 `S114` 的病态臂）。
///
/// ⚠ ⭐ **如实登记的偏离（模块文档 §8.3）**：本乘子给出
/// `linear(volume) × linear(delta)`，⛔ 不是 `linear(volume + delta)`；
/// 两者在 binary32 下一般相差若干 ulp。采用乘子形状的代价就是这条偏离，
/// 收益是"⛔ 零新每区域字段（`PreparedRegion` ＋0）＋ 无调制时逐位不变"两条硬约束
/// 同时成立 —— 因为摊平表里只有**烘好的** `volume_gain`、⛔ 没有 `volume_db`。
///
/// 实时安全：纯算术 ＋ 定长切片 ＋ 一次 `exp2`；无分配、无锁、无 I/O。
/// ⚠ `exp2` 属 ADR-0001 的超越函数类 ⇒ 跨架构用 ulp 预算判定，⛔ 不声称位型相等；
/// **同一二进制内**三条路调用的是本函数**同一份**实现 ⇒ 逐位一致（判据 `S114`）。
#[must_use]
pub fn volume_ratio_with_baked(mods: &[BakedCcMod], probe: Option<&dyn Fn(u8) -> u8>) -> f32 {
    let Some(probe) = probe else {
        return 1.0;
    };
    let (decibels, _unresolved) = cc_value_with_baked(0.0, ModTarget::Volume, mods, probe);
    (decibels * std::f32::consts::LOG2_10 / 20.0).exp2()
}

/// ⭐⭐ 一个八度的音分数（**音分 → 音高比** 换算的唯一分母；任务单 `#116`）。
///
/// ⚠ ⛔ 本常量**不是**本单发明的：`crates/yeban-sfz/src/playback.rs` 的
/// `CENTS_PER_OCTAVE`（`= 1200.0`，`f64`）从 `b5ca1528`
/// （"add the region -> renderable sample descriptor bridge"，2026-10-08）起就在这里，
/// [`crate::Region::pitch_ratio`] 一直用它把 `transpose`（半音）与 `tune`（音分）
/// 折成音高比：`2 ^ (((note − pitch_keycenter + transpose) × 100 + tune) / 1200)`。
/// 本常量是那**同一个数**的 `f32` 形态（`tune` 的调制量在 `f32` 上求和 ⇒ 换算走 `f32`）。
pub const TUNE_CENTS_PER_OCTAVE: f32 = 1200.0;

/// ⭐⭐ `tune` 调制的**唯一换算**（任务单 `#116`）：音分增量 → 音高比乘子。
///
/// ```text
/// tune_delta_cents = 0 + Σ_{cc} delta × curve(probe(cc))     // 单位：音分
/// tune_ratio       = 2 ^ (tune_delta_cents / 1200)           // 线性乘子（无调制 ⇒ 恰 1.0）
/// rate             = rate_base × tune_ratio                  // rate_base 已含基准 tune
/// ```
///
/// * **基准 `tune` 不在本函数里**：它早就烘进
///   [`crate::Region::pitch_ratio`]（离线）／摊平表的 `rate_per_note`（引擎）
///   ⇒ 本函数只贡献**增量**（`base = 0.0`）；
/// * `probe == None` ⇒ **一项 CC 都不读**、直接返回 `1.0`（⇒ `rate × 1.0` 在 binary32
///   下**逐位不变**，与接线前逐位相同）；
/// * `probe == Some` 且**没有** `tune` 项 ⇒ `cc_value_with_baked` 返回 `0.0`
///   ⇒ `0.0f32.exp2()` 恰为 `1.0`（IEEE：`exp2(±0)` ＝ `1`）⇒ 同样逐位不变；
/// * 曲线不可烘焙的项**不贡献**音分（⛔ 不当成曲线 0），与 [`cc_value_with_baked`] 同口径；
/// * ⛔ **不钳位**：规范只给了 `delta` 的域（模块文档裁决 3）⇒ 本函数沿用。
///   ⚠ 如实登记的边界：多个 `tune` 项代数相加后**可能**让 `2^x` 上溢成 `Inf`
///   （`x > 128` 即 `|Σ delta| > 153600` 音分）或下溢成 `0`（`x < −149`）——
///   消费方（`yeban-engine`）按**摊平期 `rate_per_note` 的既有守卫**（"速率必须有限正数，
///   否则该区域不产生采样输出"）如实退休，⛔ 不静默改成 `1.0`。
///
/// 实时安全：纯算术 ＋ 定长切片 ＋ 一次 `exp2`；无分配、无锁、无 I/O。
/// ⚠ `exp2` 属 ADR-0001 的超越函数类（与 [`crate::Region::pitch_ratio`] 同一个函数）
/// ⇒ 跨架构用 ulp 预算判定，⛔ 不声称位型相等；**同一二进制内**三条路调用的是本函数
/// **同一份**实现 ⇒ 逐位一致（判据 `S109`）。
#[must_use]
pub fn tune_ratio_with_baked(mods: &[BakedCcMod], probe: Option<&dyn Fn(u8) -> u8>) -> f32 {
    let Some(probe) = probe else {
        return 1.0;
    };
    let (cents, _unresolved) = cc_value_with_baked(0.0, ModTarget::Tune, mods, probe);
    (cents / TUNE_CENTS_PER_OCTAVE).exp2()
}

/// 摊平表里**烘进**一条曲线的点数上限（[`BakedCurve`]）。
///
/// 依据是登记语料普查：1398 个 `.sfz` 里 `_curveccN` 真正引用到的 `<curve>` 段最多
/// **8 个点**（含规范缺省补上的 `v000` / `v127`；`assets/samples/*/curves.sfz`）。
/// 超过上限 ⇒ 该曲线**不可烘焙**（[`BakedCurve::UNRESOLVED`]），⛔ 不截断、⛔ 不丢点。
pub const MAX_BAKED_CURVE_POINTS: usize = 8;

/// **每区域**的调制项数上限（摊平表的定长容器）。
///
/// 依据是登记语料普查（1398 个 `.sfz`，**`#include` 已展开**的原文尺：把每份根文件
/// 及其 include 树里出现的 `(参数, CC)` 取**并集**，因此它是任何单条 region 生效链的
/// **上界**）：并集最大 **15** 条
/// （`assets/samples/karoryfer-big-rusty-drums/Programs/01-full.sfz`）。
/// 取 `20` 留约 33 % 余量。超过上限 ⇒ 解析期明确 `Err`
/// （[`crate::error::SfzError::TooManyAmpegModulations`]），⛔ 不截断、⛔ 不静默丢弃。
///
/// ⚠ **这是一条新引入的容器界**（规范对调制项**条数**没有上限）：它的代价是
/// 摊平表的定长数组；若将来语料越界，应**抬高这个数**而不是改成截断。
/// ⚠ 该普查是**原文尺**（⛔ 不模拟 `#define`，glob include 由 `glob` 展开）
/// ⇒ 与解析器的读数可能有微小差别（登记为"不能验"项）。
///
/// ⭐ ⭐ **追加修订段（任务单 `#115`）—— 被修订的是上面那个分**母**那一支**
/// （⛔ 原文一字未删，口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋ 点名被修订的支）：
/// `#104` 的分母是**仅 `ampeg_*` 一族**的 `(参数, CC)` 并集（最大 15）；`#115` 之后
/// 这张表**同时**承载增益族目标（首个是 `amplitude`）⇒ 正确的分母是
/// **两族合计**的 `(目标, CC)` 并集。⭐ `#115` 现读数：最大 **11** 条
/// （剥 `//` 注释后按 `(目标, cc)` 去重；分母 1398 份 `.sfz`）⇒ **11 ≤ 20**
/// ⇒ ⛔ **本常量不抬高、⛔ 不引入新 `Err`**（今天能解析的文件一个都不会变成不可解析）。
///
/// ⭐ ⭐ **追加修订段（任务单 `#116`）—— 被修订的是上面那个分**母**那一支里的
/// "**两族**合计"**（⛔ 原文一字未删，口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋
/// 点名被修订的支）：`#116` 之后这张表**同时**承载**三族**目标（`ampeg_*` ＋ `amplitude`
/// ＋ `tune`）⇒ 正确的分母是**三族合计**的 `(目标, CC)` 并集。⭐ `#116` 现读数：
/// 与 `#104` **同一口径**（含 `_curveccN` 键的并集）最大 **15** 条
/// （`assets/samples/karoryfer-big-rusty-drums/Programs/01-full.sfz` —— 与 `#104` 登记
/// 的 15 是**同一个文件同一个数**）；仅按"真的产生 `delta` 的项"去重则最大 **13** 条。
/// ⇒ **15 ≤ 20** ⇒ ⛔ **本常量仍然不抬高、⛔ 仍然不引入新 `Err`**
/// （今天能解析的文件一个都不会变成不可解析）。
///
/// ⭐ ⭐ **追加修订段（任务单 `#119`）—— 被修订的是上面那个分**母**那一支里的
/// "**三族**合计"**（⛔ 原文一字未删，口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋
/// 点名被修订的支）：`#119` 之后这张表**同时**承载**四族**目标（`ampeg_*` ＋
/// `amplitude` ＋ `tune`／`pitch` ＋ `volume`）⇒ 正确的分母是**四族合计**的
/// `(目标, CC)` 并集。⭐ `#119` 现读数（`#116` 的**同一口径**：剥 `//` 注释后按
/// `(目标, cc)` 去重、**含 `_curveccN` 键**；分母 1398 份 `.sfz`）：最大 **15** 条
/// （`assets/samples/karoryfer-big-rusty-drums/Programs/01-full.sfz` —— 与 `#104`／
/// `#116` 登记的是**同一个文件同一个数**）；仅按"真的产生 `delta` 的项"去重则最大
/// **11** 条。⚠ 语料里 `volume_*ccN` 只有 **2** 处（`karoryfer-black-and-blue-basses`
/// 的 `Programs/controls/common.sfz` 与 `common_warm.sfz`），它**没有**把任何文件的
/// 并集推高。⇒ **15 ≤ 20** ⇒ ⛔ **本常量仍然不抬高、⛔ 仍然不引入新 `Err`**。
///
/// ⭐ ⭐ **追加修订段（任务单 `#125`）—— 被修订的是上面那句里的「只有 **2** 处」**（⛔ 原文
/// 一字未删，口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋ 点名被修订的支）：同 `#121` 裁的
/// 唯一口径（剥 `//` 注释 ＋ 行首/空白边界 opcode token ＋ **收 `$` 宏名**）⇒ ⭐ **实测 3 处
/// ／ 3 文件** —— 上面点名的两个 `volume_oncc24=7`（`common.sfz` ／ `common_warm.sfz`）
/// **＋** `assets/samples/ixox-flute/Ixox Flute.sfz:152` 的 `volume_oncc$F2_DIS=-6`
/// （**宏名**形式，按本口径**必须**算本族）。⚠ 本修订段**只**修订计数那一支：
/// 「并集最大 **15**」与「**15 ≤ 20**」是 `#116`／`#119` 的**并集**读数（⛔ 本单不复算、
/// 原样保留）；本单复核的是**裸出现计数**这一支。依据命令 ＝
/// `cargo test -p yeban-engine --test sfz_instrument s115 -- --nocapture`。
pub const AMPEG_MOD_CAP: usize = 20;

/// 一个调制项的**形态**（名字后缀）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModulationForm {
    /// `ccN`（SFZ1 旧形）或 `_onccN`（SFZ2）——载**调制量 `delta`**。
    Delta,
    /// `_curveccN`（ARIA）——载**曲线下标**。
    Curve,
    /// ⭐ `ampeg_vel2<param>`（任务单 `#106`）——载**力度调制量**（`-100..=100`，
    /// 与目标同单位）。名字里**没有下标**（⇒ [`parse_modulation_name`] 返回的
    /// 数字串是**空串**），也**没有**对应的曲线 opcode（映射写死在公式里：
    /// `delta × velocity / 127`，见 [`velocity_scale`]）。
    Vel2,
}

/// 识别 `ampeg_<param><suffix>` ／ ⭐ `ampeg_vel2<param>` ／ ⭐ **`amplitude<suffix>`**
/// 三族，返回 `(⭐ 目标, 形态, 下标数字串)`。
///
/// ⭐ ⭐ **追加修订段（任务单 `#116`）—— 被修订的是上面那句里的族数"三族"**
/// （⛔ 原文一字未删，口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋ 点名被修订的支）：
/// `#116` 加上第四族 **`tune<suffix>`** ⇒ 现在是**四族**。⚠ 被修订的只是**族数**：
/// `#104`／`#106`／`#115` 那三族的识别文本、后缀政策与返回形状**一字未变**。
///
/// ⭐ ⭐ **追加修订段（任务单 `#118`）—— 被修订的是上一段末尾"现在是**四族**"里的
/// 族数**（⛔ 原文一字未删，口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋
/// 点名被修订的支）：`#118` 加上第五族 **`pitch<suffix>`** ⇒ 现在是**五族**。
/// ⚠ **被修订的只有族数**：`#104`／`#106`／`#115`／`#116` 那四族的识别文本、
/// 后缀政策与返回形状**一字未变**；⭐ 而且第五族归的是**同一个目标**
/// [`ModTarget::Tune`]（裁定、理由与代价见模块文档 §7.2）—— 它**没有**新增枚举变体、
/// **没有**新增域、**没有**新增单位类别。
///
/// ⭐ ⭐ **追加修订段（任务单 `#119`）—— 被修订的是上一段末尾"现在是**五族**"里的
/// 族数**（⛔ 原文一字未删，口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋
/// 点名被修订的支）：`#119` 加上第六族 **`volume<suffix>`** ⇒ 现在是**六族**。
/// ⚠ **被修订的只有族数**：`#104`／`#106`／`#115`／`#116`／`#118` 那五族的识别文本、
/// 后缀政策与返回形状**一字未变**；⭐ 第六族归的是**新目标** [`ModTarget::Volume`]
/// —— 与第五族**不同**，它**是**新枚举变体、**是**新域
/// （`-144..=48` dB）、**也是**新单位类别（[`ModUnit::Decibels`]）：
/// 规范逐字表行、域裁定、理由与代价见模块文档 §8。
///
/// 四种后缀（大小写敏感，与其它 opcode 同口径）：
/// * `ccN` ⇒ [`ModulationForm::Delta`]（SFZ1 旧形）；
/// * `_ccN` ⇒ [`ModulationForm::Delta`]（SFZ1 的另一种写法；`amplitude_ccN` 用它，
///   语料 854 处 —— 五个 `ampeg_*` **没有**这个写法，见本函数末尾的反向腿）；
/// * `_onccN` ⇒ [`ModulationForm::Delta`]（SFZ2；规范说它与上一条**是别名**）；
/// * `_curveccN` ⇒ [`ModulationForm::Curve`]；
/// * ⭐ `vel2<param>` ⇒ [`ModulationForm::Vel2`]，数字串恒为**空串 `""`**
///   （该形态的名字里没有下标；调用方 ⛔ 不得把空串当成畸形下标 —— 它由本函数
///   在**返回 `Some` 之前**就确定了，⛔ 不到达下面那条"非空且全数字"的检查）。
///   ⚠ `vel2` 在**参数名之前**（`ampeg_vel2attack`，⛔ 不是 `ampeg_attack_vel2`）；
///   规范只列这五个名字 ⇒ `ampeg_vel2attack` 之外的任何尾巴（`…cc1` / `…_curvecc1` /
///   未知参数名）都**不是**本族。⭐ `amplitude` 页**没有** `vel2` 行 ⇒ 它没有力度形态。
///
/// ⭐ **目标**（任务单 `#115`）：`ampeg_…` 一族 ⇒ [`ModTarget::Ampeg`]；
/// `amplitude<suffix>` ⇒ [`ModTarget::Amplitude`]（逐字表行见模块文档 §5）。
/// ⭐⭐ **目标**（任务单 `#116`）：`tune<suffix>` ⇒ [`ModTarget::Tune`]
/// （逐字表行与裁定见模块文档 §6）。
/// `amplitude_smoothccN`（表行 Type ＝ N/A）与裸 `amplitude`（没有下标）都**不是**
/// 本函数的目标（前者按未知 opcode 忽略、后者是**基准** opcode、由
/// [`crate::instrument`] 单独读）。⭐ 同理 `tune_smoothccN` / `tune_stepccN`
/// （`tune` 页的 Modulation 表没有这两行）与裸 `tune=` 也都**不是**本函数的目标。
///
/// 数字串必须非空且全是 ASCII 数字；否则返回 [`None`]（按未知 opcode 忽略，与
/// [`crate::instrument`] 里 `amp_velcurve_N` / `loccN` 两族同一条口径）。
///
/// 返回**数字串**而不是解析结果，是为了让调用方把"数字但越界"和"名字不像本族"
/// 分开报（前者明确 `Err`，后者忽略）。
#[must_use]
pub fn parse_modulation_name(name: &str) -> Option<(ModTarget, ModulationForm, &str)> {
    // ⭐ 增益族（任务单 `#115`）：`amplitude` ＋ 四种后缀之一。
    // ⚠ 检查顺序：`_curvecc` 必须在 `_cc` **之前**（否则 `…_curvecc4` 会被剥成
    // `urvecc4` ⇒ 非数字 ⇒ 静默漏掉）；`_oncc` 也必须在 `cc` 之前。
    if let Some(tail) = name.strip_prefix("amplitude") {
        let (form, digits) = if let Some(digits) = tail.strip_prefix("_curvecc") {
            (ModulationForm::Curve, digits)
        } else if let Some(digits) = tail.strip_prefix("_oncc") {
            (ModulationForm::Delta, digits)
        } else if let Some(digits) = tail.strip_prefix("_cc") {
            (ModulationForm::Delta, digits)
        } else {
            (ModulationForm::Delta, tail.strip_prefix("cc")?)
        };
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        return Some((ModTarget::Amplitude, form, digits));
    }
    // ⭐⭐ `pitch` 族（任务单 `#118`）：规范表行逐字是 `pitch_onccN` / `pitch_curveccN`
    // （模块文档 §7.1）⇒ 与 `tune` 族**同款**的四种后缀识别（⛔ 不另开一套），
    // 并归**同一个目标** [`ModTarget::Tune`] —— 规范那句 "alias" 的机械后果
    // （裁定、理由与代价见模块文档 §7.2）。⚠ 检查顺序同上面：`_curvecc` 在 `_cc`
    // 之前、`_oncc` 也在 `cc` 之前。
    // ⚠ `pitch_keycenter`（静态 opcode）在这里被 `strip_prefix("cc")?` 挡掉
    // ⇒ 它**不是**本族的目标（⛔ 本单不发明 `pitch=` 这个静态 opcode）。
    // ⚠ `pitch_smoothccN` / `pitch_stepccN`（表里有、⛔ 本单不做）同样落不进四后缀
    // ⇒ 按未知 opcode 忽略（登记见模块文档 §7.4）。
    if let Some(tail) = name.strip_prefix("pitch") {
        let (form, digits) = if let Some(digits) = tail.strip_prefix("_curvecc") {
            (ModulationForm::Curve, digits)
        } else if let Some(digits) = tail.strip_prefix("_oncc") {
            (ModulationForm::Delta, digits)
        } else if let Some(digits) = tail.strip_prefix("_cc") {
            (ModulationForm::Delta, digits)
        } else {
            (ModulationForm::Delta, tail.strip_prefix("cc")?)
        };
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        return Some((ModTarget::Tune, form, digits));
    }
    // ⭐⭐ `tune` 族（任务单 `#116`）：与 [`ModTarget::Amplitude`] **同款**的四种后缀
    // （⛔ 不另开一套识别）。⚠ 检查顺序同上面：`_curvecc` 在 `_cc` 之前、`_oncc` 也在
    // `cc` 之前。⚠ 裸 `tune=`（静态基准）在这里被 `strip_prefix("cc")?` 挡掉
    // ⇒ 它**不是**本族的目标（由 [`crate::instrument`] 单独读，见模块文档 §6）。
    if let Some(tail) = name.strip_prefix("tune") {
        let (form, digits) = if let Some(digits) = tail.strip_prefix("_curvecc") {
            (ModulationForm::Curve, digits)
        } else if let Some(digits) = tail.strip_prefix("_oncc") {
            (ModulationForm::Delta, digits)
        } else if let Some(digits) = tail.strip_prefix("_cc") {
            (ModulationForm::Delta, digits)
        } else {
            (ModulationForm::Delta, tail.strip_prefix("cc")?)
        };
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        return Some((ModTarget::Tune, form, digits));
    }
    // ⭐⭐ `volume` 族（任务单 `#119`）：与上面三族**同款**的四种后缀识别
    // （⛔ 不另开一套）。⚠ 检查顺序同上面：`_curvecc` 在 `_cc` 之前、`_oncc` 也在
    // `cc` 之前。
    // ⚠ 裸 `volume=`（**静态基准**）在这里被 `strip_prefix("cc")?` 挡掉
    // ⇒ 它**不是**本族的目标（由 [`crate::instrument`] 单独读）。
    // ⚠ `volume_smoothccN` / `volume_stepccN`（表里有、⛔ 本单不做，见 §8.4）同样
    // 落不进四后缀 ⇒ 按未知 opcode 忽略。
    // ⚠ `global_volume_onccN` / `master_volume_onccN` / `group_volume_onccN`
    // （**另外三个**静态 opcode，语料 47 处）⛔ 不以 `volume` 开头 ⇒ 同样不落本族。
    // ⭐⭐ 追加修订（任务单 `#121`，台账第 278 条）：上面那句里的「语料 **47** 处」**归属错了** ——
    // 实测那 47 处**全是 `lfo*_volume_onccN`（47 次 ／ 30 文件）**，而这**三个** `_oncc` 家族
    // **各 0 次 ／ 0 文件**（口径 ＝ 剥 `//` 注释 ＋ 行首/空白边界 token ＋ **收 `$` 宏名**；
    // 命令 ＝ `cargo test -p yeban-engine --test sfz_instrument s115 -- --nocapture`；
    // 完整依据与逐名展开见上面 §8.4.1）。
    // ⚠ 规范 Modulation 表逐字的 delta 行名 `gain_ccN` ⛔ 本单不识别（§8.4）。
    // ⭐⭐ 追加修订段（任务单 `#124`，台账第 278 条）：上面那句里「`gain_ccN` ⛔ 本单不识别」
    // 的**那一支**已被修订 —— `#124` 起 `gain_ccN` **是识别的**（归**同一个**
    // [`ModTarget::Volume`]，见 §8.6 的裁定）；⛔ 原文一字未删，修订落在下面的新支。
    //   ⚠ 新支**逐字只认一个后缀** `_cc`（＝ `gain_ccN`）：规范表里 `gain` 拼写**只有这一行**，
    //   ⛔ 没有 `gain_onccN`／`gain_curveccN`（8 档实测，见 §8.6）⇒ `_oncc`／`_curvecc` 与
    //   无下划线的 `gainccN` 都**落不进**新支 ⇒ 仍是"未知 opcode 忽略"。
    //   ⚠ 顺序无关：`gain` 与其他任何族的前缀**不相交**（`amplitude`／`pitch`／`tune`／
    //   `volume`／`ampeg_` 都不以 `gain` 开头）⇒ 既有族的可识别集**逐字未动**。
    if let Some(tail) = name.strip_prefix("gain") {
        let digits = tail.strip_prefix("_cc")?;
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        return Some((ModTarget::Volume, ModulationForm::Delta, digits));
    }
    if let Some(tail) = name.strip_prefix("volume") {
        let (form, digits) = if let Some(digits) = tail.strip_prefix("_curvecc") {
            (ModulationForm::Curve, digits)
        } else if let Some(digits) = tail.strip_prefix("_oncc") {
            (ModulationForm::Delta, digits)
        } else if let Some(digits) = tail.strip_prefix("_cc") {
            (ModulationForm::Delta, digits)
        } else {
            (ModulationForm::Delta, tail.strip_prefix("cc")?)
        };
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        return Some((ModTarget::Volume, form, digits));
    }
    let rest = name.strip_prefix("ampeg_")?;
    // ⭐ 力度形态（任务单 `#106`）：`ampeg_vel2<param>` —— 整名到此为止。
    if let Some(param_name) = rest.strip_prefix("vel2") {
        let param = AmpegParam::ALL
            .into_iter()
            .find(|param| param.name() == param_name)?;
        return Some((ModTarget::Ampeg(param), ModulationForm::Vel2, ""));
    }
    let (param, tail) = AmpegParam::ALL
        .into_iter()
        .find_map(|param| rest.strip_prefix(param.name()).map(|tail| (param, tail)))?;
    let (form, digits) = if let Some(digits) = tail.strip_prefix("_curvecc") {
        (ModulationForm::Curve, digits)
    } else if let Some(digits) = tail.strip_prefix("_oncc") {
        (ModulationForm::Delta, digits)
    } else {
        // ⚠ `ampeg_*` 族**没有** `_ccN` 写法（SFZ1 教程把 EG 一族列为"名字复杂 ⇒
        // 只追加 `ccN`"）⇒ 这里**只**认裸 `cc`，`ampeg_attack_cc100` 仍是 `None`。
        (ModulationForm::Delta, tail.strip_prefix("cc")?)
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some((ModTarget::Ampeg(param), form, digits))
}

/// 一条**解析层**的 CC 调制项（[`crate::instrument::Region::cc_modulations`] 的元素）。
///
/// `curve` 是**下标**（`0..=255`），**不是**曲线本体：`Region` 在解析期**看不到**
/// `<curve>` 段（段头可以出现在 region 之后），所以这里只留下标，由
/// [`crate::instrument::Instrument::modulation_curve`] 在求值期解析。
///
/// ⚠ **沿革（任务单 `#115`）**：`#104` 把它登记为 `AmpegCcMod`（字段 `param: AmpegParam`）；
/// `#115` 把目标推广成 [`ModTarget`] ⇒ 本结构改名 [`CcMod`]、字段改名 `target`。
/// 旧名保留为[类型别名](AmpegCcMod)（⛔ 原文一字未删，口径同台账第 278 条）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CcMod {
    /// 被调制的**目标**（五个 `ampeg_*` 之一，或 `#115` 的 `amplitude`）。
    pub target: ModTarget,
    /// MIDI CC 号（`0..=255`；标准 `0..=127`，扩展 `128..=155`）。
    pub cc: u8,
    /// 调制量 `delta`（与目标同单位；规范 Range `-100..=100`）。
    pub delta: f32,
    /// `_curveccN` 的曲线下标（缺省 `0` = 线性 `0 → 1`）。
    pub curve: u8,
}

/// ⚠ **沿革别名**（台账第 278 条）：`#104` 的登记名 `AmpegCcMod`；`#115` 推广后
/// 正名是 [`CcMod`]。旧名保留只为**既有登记文本的链接不断**（⛔ 语义未变）。
pub type AmpegCcMod = CcMod;

/// 一条**烘好曲线**的 CC 调制项（摊平表 [`crate::modulation`] 的 `Copy` 形状）。
///
/// 与 [`CcMod`] 的唯一差别是 `curve` 已经从"下标"变成"曲线本体"的首地址无关表示
/// （[`BakedCurve`]）⇒ 求值不再需要 `Instrument`。
///
/// ⚠ **沿革（任务单 `#115`）**：`#104` 的登记名是 `BakedAmpegCcMod`（字段 `param`）；
/// `#115` 改名并把字段推广成 `target`。旧名保留为[类型别名](BakedAmpegCcMod)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BakedCcMod {
    /// 被调制的**目标**。
    pub target: ModTarget,
    /// MIDI CC 号。
    pub cc: u8,
    /// 调制量。
    pub delta: f32,
    /// 已烘焙的曲线。
    pub curve: BakedCurve,
}

/// ⚠ **沿革别名**（台账第 278 条）：`#104` 的登记名 `BakedAmpegCcMod`；正名是
/// [`BakedCcMod`]。旧名保留只为**既有登记文本的链接不断**。
pub type BakedAmpegCcMod = BakedCcMod;

impl BakedCcMod {
    /// 定长容器的**空槽**填充值。
    ///
    /// ⚠ 它不是一条调制项：调用方**必须**用容器外的条数（`cc_mod_count`）限定有效范围，
    /// ⛔ 不得靠 `delta == 0.0` 之类的值判断（那会把"真的写了 `delta=0`"混淆掉）。
    pub const EMPTY: Self = Self {
        target: ModTarget::Ampeg(AmpegParam::Attack),
        cc: 0,
        delta: 0.0,
        curve: BakedCurve::UNRESOLVED,
    };
}

/// 占位点（`BakedCurve::UNRESOLVED` 用；值恒被 `len == 0` 挡掉）。
const NO_POINT: CurvePoint = CurvePoint { at: 0, value: 0.0 };

/// **定长、`Copy` 的曲线**：下标 ＋ 至多 [`MAX_BAKED_CURVE_POINTS`] 个显式点。
///
/// `len == 0` ＝ [`Self::UNRESOLVED`]：该曲线**不可烘焙**（内建 4..=6 规范未给公式、
/// `<curve>` 未定义、或点数超过上限）⇒ 求值方必须**如实报告**（[`ampeg_value_with_baked`]
/// 把它计进 `unresolved`），⛔ 不得当成"曲线 0"。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BakedCurve {
    /// 曲线下标（原样保留，便于诊断）。
    index: u8,
    /// 有效点数（`0` ＝ 不可烘焙）。
    len: u8,
    /// 点表（`..len` 有效；首点 `at == 0`、末点 `at == 127` 由 [`Curve`] 契约保证）。
    points: [CurvePoint; MAX_BAKED_CURVE_POINTS],
}

impl BakedCurve {
    /// **不可烘焙**的哨兵（`len == 0`）。
    pub const UNRESOLVED: Self = Self {
        index: 0,
        len: 0,
        points: [NO_POINT; MAX_BAKED_CURVE_POINTS],
    };

    /// 把一条**已解析**的曲线烘成定长形状；点数超过 [`MAX_BAKED_CURVE_POINTS`]
    /// ⇒ [`Self::UNRESOLVED`]（⛔ 不截断）。
    #[must_use]
    pub fn bake(curve: &Curve) -> Self {
        let points = curve.points();
        if points.len() > MAX_BAKED_CURVE_POINTS {
            return Self::UNRESOLVED;
        }
        // `Curve` 的构造方保证至少两个点；空表按"不可烘焙"处理（⛔ 不 panic）。
        if points.is_empty() {
            return Self::UNRESOLVED;
        }
        let count = points.len() as u8;
        let mut baked = Self {
            index: curve.index(),
            len: count,
            points: [NO_POINT; MAX_BAKED_CURVE_POINTS],
        };
        baked.points[..points.len()].copy_from_slice(points);
        baked
    }

    /// 曲线下标（原样）。
    #[must_use]
    pub const fn index(&self) -> u8 {
        self.index
    }

    /// 是否烘焙成功（`len > 0`）。
    #[must_use]
    pub const fn is_resolved(&self) -> bool {
        self.len > 0
    }

    /// 求 `x`（＝ CC 值，`0..=127`）处的曲线值。
    ///
    /// 未烘焙 ⇒ `0.0`（＝ 该项**不贡献**任何量）；**调用方必须先用
    /// [`Self::is_resolved`] 报告**，⛔ 不得把 `0.0` 当成"曲线 0 在 `x = 0` 处"。
    /// 求值本体是 [`curve_value_at`]（与 [`Curve::value_at`] **同一份**实现）。
    #[must_use]
    pub fn value_at(&self, x: f32) -> f32 {
        if self.len == 0 {
            return 0.0;
        }
        curve_value_at(&self.points[..usize::from(self.len)], x)
    }
}

/// ⭐ **调制求值的唯一实现**（任务单 `#115` 起按 [`ModTarget`] 过滤）：
/// `base + Σ delta × curve(probe(cc))`。
///
/// `target` 过滤 [`BakedCcMod::target`]；`mods` 的求和顺序**就是**传入顺序
/// （调用方按 `(target, cc)` 升序给出，见模块文档裁决 5）。
///
/// 返回 `(取值, 未施加的调制项数)`：曲线不可烘焙的项**计进第二个数**并且**不贡献**
/// 任何量（⛔ 不静默当成曲线 0）——调用方负责把那个数报出去。
///
/// ⚠ **本形态没有力度输入** ⇒ `ampeg_vel2*` **不参与**（⛔ 不是"按力度 0 求值"）。
/// 有力度的那一支用 [`cc_value_with_baked_and_vel2`]；两支部走**同一段** CC 求和
/// （本函数是它的薄包装，⛔ 没有第二套公式）。
///
/// 实时安全：纯算术 ＋ 切片读；无分配、无锁、无 I/O。
#[must_use]
pub fn cc_value_with_baked(
    base: f32,
    target: ModTarget,
    mods: &[BakedCcMod],
    probe: &dyn Fn(u8) -> u8,
) -> (f32, usize) {
    cc_value_with_baked_and_vel2(base, target, mods, None, 0, Some(probe))
}

/// ⚠ **沿革薄包装**（台账第 278 条）：`#104` 的 `ampeg_value_with_baked(base, param, …)`
/// ＝ [`cc_value_with_baked`] 传 `ModTarget::Ampeg(param)`。⛔ 没有第二套公式。
#[must_use]
pub fn ampeg_value_with_baked(
    base: f32,
    param: AmpegParam,
    mods: &[BakedCcMod],
    probe: &dyn Fn(u8) -> u8,
) -> (f32, usize) {
    cc_value_with_baked(base, ModTarget::Ampeg(param), mods, probe)
}

/// ⭐ **力度调制的唯一换算**：规范的 `velocity / 127`（`velocity` ∈ `0..=127`）。
///
/// 出处逐字：<https://sfzformat.com/modulations/vel2/> 的
/// `(param) = (eg type)_(param) + ( (eg type)_vel2(param) * velocity / 127 )`。
/// ⛔ 没有第二个分母、⛔ 没有曲线下标、⛔ 不查表。
///
/// ⚠ 本函数对 `velocity` 的**全 `u8` 域**都有定义（`128..=255` 会算出 `> 1.0`）——
/// 调用方负责只喂 `0..=127`（MIDI 力度域；本仓的三条路喂的都是 `SfzVoice::velocity`／
/// 渲染入口的 `velocity: u8`，其域由上层保证）。⛔ 这里**不钳位**（第 259 条同形）。
#[must_use]
pub fn velocity_scale(velocity: u8) -> f32 {
    f32::from(velocity) / 127.0
}

/// ⭐ **两支输入的合一求值**（任务单 `#106`）：
/// `base + Σ_cc delta × curve(probe(cc)) + delta_vel2 × velocity / 127`。
///
/// * `probe == None` ⇒ **不读任何 CC**（CC 项一项都不加，且不产生"未施加"计数 ——
///   与 [`cc_value_with_baked`] 的调用方约定一致：探针没给 ⇒ 无 CC 调制）；
/// * `vel2 == None` ⇒ **力度项一项算术都不做**（⇒ 与接线前逐位相同，见模块文档裁决 7）；
///   `vel2 == Some(delta)` ⇒ 末尾加上 `delta × velocity / 127`（[`velocity_scale`]）；
///   ⭐ 力度项只服务 `ampeg_*` 一族 ⇒ ⛔ 对 [`ModTarget::Amplitude`] 恒传 `None`
///   （`amplitude` 页没有 `vel2` 行）；
/// * 求和顺序唯一：CC 项（传入顺序 ＝ `(target, cc)` 升序）在前、力度项在**末尾**
///   （模块文档裁决 5／6，[ARCH-DET-001]）；
/// * 返回 `(取值, 未施加的调制项数)` —— 计数的定义与 [`cc_value_with_baked`] 一致。
///
/// 实时安全：纯算术 ＋ 定长切片；无分配、无锁、无 I/O。
#[must_use]
pub fn cc_value_with_baked_and_vel2(
    base: f32,
    target: ModTarget,
    mods: &[BakedCcMod],
    vel2: Option<f32>,
    velocity: u8,
    probe: Option<&dyn Fn(u8) -> u8>,
) -> (f32, usize) {
    let mut value = base;
    let mut unresolved = 0usize;
    if let Some(probe) = probe {
        for item in mods {
            if item.target != target {
                continue;
            }
            if !item.curve.is_resolved() {
                unresolved += 1;
                continue;
            }
            value += item.delta * item.curve.value_at(f32::from(probe(item.cc)));
        }
    }
    if let Some(delta) = vel2 {
        value += delta * velocity_scale(velocity);
    }
    (value, unresolved)
}

/// ⚠ **沿革薄包装**（台账第 278 条）：`#106` 的
/// `ampeg_value_with_baked_and_vel2(base, param, …)` ＝ [`cc_value_with_baked_and_vel2`]
/// 传 `ModTarget::Ampeg(param)`。⛔ 没有第二套公式。
#[must_use]
pub fn ampeg_value_with_baked_and_vel2(
    base: f32,
    param: AmpegParam,
    mods: &[BakedCcMod],
    vel2: Option<f32>,
    velocity: u8,
    probe: Option<&dyn Fn(u8) -> u8>,
) -> (f32, usize) {
    cc_value_with_baked_and_vel2(base, ModTarget::Ampeg(param), mods, vel2, velocity, probe)
}

/// 把解析层的调制项烘成摊平层的定长形状（目标**原样**带过，任务单 `#115`）。
///
/// `resolve` 把曲线下标变成一条曲线（`None` ＝ **无据可依**：内建 4..=6 无公式、
/// `<curve>` 未定义）⇒ 那一项烘成 [`BakedCurve::UNRESOLVED`]（**保留在表里**，
/// 由 [`cc_value_with_baked`] 计数，⛔ 不删除、⛔ 不替换成曲线 0）。
///
/// 顺序**原样保留**（调用方给的是 `(target, cc)` 升序，见模块文档裁决 5）。
/// 构造期调用（`Vec` ⇒ 允许分配）。
#[must_use]
pub fn bake_cc_mods(
    mods: &[CcMod],
    mut resolve: impl FnMut(u8) -> Option<Curve>,
) -> Vec<BakedCcMod> {
    mods.iter()
        .map(|item| BakedCcMod {
            target: item.target,
            cc: item.cc,
            delta: item.delta,
            curve: resolve(item.curve)
                .map_or(BakedCurve::UNRESOLVED, |curve| BakedCurve::bake(&curve)),
        })
        .collect()
}

/// ⚠ **沿革薄包装**（台账第 278 条）：`#104` 的 `bake_ampeg_mods` ＝ [`bake_cc_mods`]。
/// 名字保留只为既有调用点与登记链接不断（⛔ 语义、顺序、曲线烘焙口径全同）。
#[must_use]
pub fn bake_ampeg_mods(
    mods: &[CcMod],
    resolve: impl FnMut(u8) -> Option<Curve>,
) -> Vec<BakedCcMod> {
    bake_cc_mods(mods, resolve)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(entries: &[(AmpegParam, u8, f32)]) -> Vec<BakedCcMod> {
        entries
            .iter()
            .map(|&(param, cc, delta)| BakedCcMod {
                target: ModTarget::Ampeg(param),
                cc,
                delta,
                curve: BakedCurve::bake(&Curve::built_in(0).expect("curve 0 is built in")),
            })
            .collect()
    }

    #[test]
    fn the_three_suffixes_are_recognised_and_nothing_else_is() {
        // ⭐ 四形态里"名字"这一层：旧形 `ccN` / SFZ2 `_onccN` 同归 Delta，`_curveccN` 归 Curve。
        // ⚠ ⭐ **追加修订段（任务单 `#119`）—— 被修订的是本数组的行数 `17` 那一支**
        // （⛔ `#104`～`#118` 的十七条**原文一字未删**，口径同台账第 278 条：保留原文 ＋
        // 追加修订段 ＋ 点名被修订的支）：`#119` 在**末尾**追加四条 `volume` 族的
        // 合法写法 ⇒ 行数 `17 → 21`。
        // ⭐⭐ 追加修订段（任务单 `#124`，台账第 278 条）：**被修订的支 ＝ 上一段末尾的行数
        // `21`**（以及本数组末尾那一项）—— `#124` 在**末尾**追加第 22 项 `gain_cc9`
        // （`gain_ccN` 与那四个 `volume` 拼写落**同一个目标**）⇒ 行数 `21 → 22`。
        // ⚠ **被修订的只有行数**：`#104`～`#119` 的**前 21 项逐字未动**、归属一个都没改。
        let cases: [(&str, ModTarget, ModulationForm, &str); 22] = [
            (
                "ampeg_attackcc100",
                ModTarget::Ampeg(AmpegParam::Attack),
                ModulationForm::Delta,
                "100",
            ),
            (
                "ampeg_attack_oncc100",
                ModTarget::Ampeg(AmpegParam::Attack),
                ModulationForm::Delta,
                "100",
            ),
            (
                "ampeg_attack_curvecc100",
                ModTarget::Ampeg(AmpegParam::Attack),
                ModulationForm::Curve,
                "100",
            ),
            (
                "ampeg_sustain_oncc71",
                ModTarget::Ampeg(AmpegParam::Sustain),
                ModulationForm::Delta,
                "71",
            ),
            (
                "ampeg_release_curvecc49",
                ModTarget::Ampeg(AmpegParam::Release),
                ModulationForm::Curve,
                "49",
            ),
            (
                "ampeg_holdcc0",
                ModTarget::Ampeg(AmpegParam::Hold),
                ModulationForm::Delta,
                "0",
            ),
            // ⭐ 任务单 `#115`：增益族第一目标 `amplitude` 的四种合法写法。
            (
                "amplitude_oncc108",
                ModTarget::Amplitude,
                ModulationForm::Delta,
                "108",
            ),
            (
                "amplitude_cc50",
                ModTarget::Amplitude,
                ModulationForm::Delta,
                "50",
            ),
            (
                "amplitude_curvecc110",
                ModTarget::Amplitude,
                ModulationForm::Curve,
                "110",
            ),
            // ⭐⭐ 任务单 `#116`：`tune` 族的四种后缀（与 `amplitude` **同款**）。
            ("tune_oncc34", ModTarget::Tune, ModulationForm::Delta, "34"),
            ("tune_cc41", ModTarget::Tune, ModulationForm::Delta, "41"),
            ("tune_curvecc1", ModTarget::Tune, ModulationForm::Curve, "1"),
            ("tunecc1", ModTarget::Tune, ModulationForm::Delta, "1"),
            // ⭐⭐ 任务单 `#118`：`pitch` 族 —— 规范表行逐字就是这四个名字里的
            // `pitch_onccN` / `pitch_curveccN`，而它归的是**同一个目标** `Tune`
            // （裁定见模块文档 §7.2）。
            ("pitch_oncc27", ModTarget::Tune, ModulationForm::Delta, "27"),
            ("pitch_cc94", ModTarget::Tune, ModulationForm::Delta, "94"),
            (
                "pitch_curvecc3",
                ModTarget::Tune,
                ModulationForm::Curve,
                "3",
            ),
            ("pitchcc12", ModTarget::Tune, ModulationForm::Delta, "12"),
            // ⭐⭐ 任务单 `#119`：`volume` 族的四种后缀 —— 与上面三族**同款**
            // （⛔ 不另开一套识别）。⚠ 规范 Modulation 表里逐字的 delta 行名是
            // `gain_ccN`（⛔ 本单不识别，见 §8.4），而正文写
            // "`volume_ccN` is a valid alias for `gain_ccN`" ⇒ 这四个拼写落同一个目标。
            // ⭐⭐ 追加修订段（任务单 `#125`，台账第 278 条）—— **被修订的是上面那句里的
            // 「`gain_ccN`（⛔ 本单不识别，见 §8.4）」这一支**（⛔ 原文一字未删）：
            // `#124` 起 `gain_ccN` **是识别的**（归**同一个** `ModTarget::Volume`，
            // 单位 `Decibels`；正例表里就是下面那一项 `"gain_cc9"`，裁定见 §8.6）。
            // ⭐ 本单**点名**它，是因为 `#124` 的 §8.6「被修订的支」清单只点了
            // `parse_modulation_name` 里那条同款注释与本函数**反向腿表**里的 `"gain_cc1"`，
            // **漏了**这一处（本单只补登记，⛔ 不改行为）。
            (
                "volume_oncc24",
                ModTarget::Volume,
                ModulationForm::Delta,
                "24",
            ),
            ("volume_cc7", ModTarget::Volume, ModulationForm::Delta, "7"),
            (
                "volume_curvecc11",
                ModTarget::Volume,
                ModulationForm::Curve,
                "11",
            ),
            ("volumecc3", ModTarget::Volume, ModulationForm::Delta, "3"),
            // ⭐⭐ 任务单 `#124`：`gain` 拼写 —— `volume` 页 Modulation 表里**逐字的 delta
            // 行名**就是 `gain_ccN`，而正文写 "`volume_ccN` is a valid alias for `gain_ccN`"
            // ⇒ 它与上面四个 `volume` 拼写落**同一个目标**（裁定与代价见模块文档 §8.6）。
            // ⚠ 本单**只**收这一个拼写：⛔ `gain_onccN`／⛔ `gain_curveccN`／⛔ `gainccN`
            // （规范 Modulation 表里 `gain` 开头的**只有 `gain_ccN` 这一行**）。
            ("gain_cc9", ModTarget::Volume, ModulationForm::Delta, "9"),
        ];
        for (name, target, form, digits) in cases {
            assert_eq!(
                parse_modulation_name(name),
                Some((target, form, digits)),
                "{name}"
            );
        }
        // ⛔ 基准形态、⛔ `_ccN`（eg 族没有这个写法）、⛔ smoothcc/stepcc（不属本族）、
        // ⛔ 别的参数族、⛔ 空/非数字下标 —— 一律 None（按未知 opcode 忽略）。
        // ⭐ 注意 `amplitude_oncc100` **不再**在这张表里：`#115` 起它是**合法**目标。
        for bad in [
            "ampeg_attack",
            "ampeg_attack_cc100",
            "ampeg_attack_smoothcc100",
            "ampeg_attack_stepcc100",
            "ampeg_attack_oncc",
            "ampeg_attackcc",
            "ampeg_attackcc1x",
            "ampeg_attack_oncc-1",
            "fileg_attackcc100",
            "ampeg_attackhdcc100",
            "ampeg_",
            "ampeg_attackcc 100",
            "ampeg_attack_curvecc",
            // ⭐ `#115` 的反向腿：`amplitude` 族的**非目标**写法一律 None。
            "amplitude",
            "amplitude_smoothcc150",
            "amplitude_stepcc150",
            "amplitude_hdcc12",
            "amplitude_oncc",
            "amplitude_cc",
            "amplitude_curvecc",
            "amplitude_oncc1x",
            "amplitudeoncc1",
            "amplitude_cc-1",
            // ⭐⭐ `#116` 的反向腿：`tune` 族的**非目标**写法一律 None。
            // ⚠ 裸 `tune`（基准 opcode）与 `tune_smoothccN`／`tune_stepccN`
            // （`tune` 页的 Modulation 表没有这两行）都在这里 —— ⛔ 不是本族。
            "tune",
            "tune_smoothcc1",
            "tune_stepcc1",
            "tune_hdcc12",
            "tune_oncc",
            "tune_cc",
            "tune_curvecc",
            "tune_oncc1x",
            "tuneoncc1",
            "tune_cc-1",
            "tuned",
            // ⭐⭐ `#118` 的反向腿：`pitch` 族的**非目标**写法一律 None。
            // ⚠⭐ 最承重的一条是 `pitch_keycenter` —— 它是**真的静态 opcode**，
            // ⛔ 绝不能被本族识别吞掉；裸 `pitch`（⛔ 规范里没有这个静态 opcode）
            // 与 `pitch_smoothccN`／`pitch_stepccN`（表里有、⛔ 本单不做，见 §7.4）
            // 同样必须 None。
            "pitch",
            "pitch_keycenter",
            "pitch_smoothcc1",
            "pitch_stepcc1",
            "pitch_hdcc12",
            "pitch_oncc",
            "pitch_cc",
            "pitch_curvecc",
            "pitch_oncc1x",
            "pitchoncc1",
            "pitch_cc-1",
            "pitched",
            // ⭐⭐ `#119` 的反向腿：`volume` 族的**非目标**写法一律 None。
            // ⚠⭐ 最承重的三条：裸 `volume`（**静态基准** opcode）、
            // `volume_smoothccN`／`volume_stepccN`（表里有、Range 空、Unit `N/A`
            // ⇒ ⛔ 本单不做，见 §8.4）必须 None；
            // ⚠ `global_volume_oncc1`（**另外三个**静态 opcode 之一，语料 47 处）
            // 同样必须 None —— 它⛔ 不以 `volume` 开头。
            // ⭐⭐ 追加修订（任务单 `#121`）：上面「语料 **47** 处」的**归属错了** ——
            // 实测那 47 处全是 `lfo*_volume_onccN`（47 次 ／ 30 文件）；`global_volume_oncc*`
            // 家族本身 **0 次 ／ 0 文件**（口径 ＝ 剥 `//` 注释 ＋ 行首/空白边界 token
            // ＋ **收 `$` 宏名**；命令 ＝ `cargo test -p yeban-engine --test sfz_instrument s115 --
            // --nocapture`；依据见 §8.4.1）。
            // ⚠ 规范 Modulation 表逐字的 delta 行名 `gain_ccN`／`gain_onccN`
            // ⛔ 本单不识别（§8.4 的裁定）。
            // ⭐⭐ 追加修订段（任务单 `#124`，台账第 278 条）：上面那句里的 `gain_ccN`
            // **那一支**已被修订 —— `#124` 起 `gain_ccN` **是识别的**（归 `ModTarget::Volume`，
            // 单位 `Decibels`）⇒ 它的那一项（原文逐字是 `"gain_cc1"`）已从**下面这张反向腿表**
            // **移到上面那张正例表**（现读 `"gain_cc9"`，⛔ 移动而非删除）。
            // ⚠ **`gain_onccN` 的"不识别"逐字仍有效**（表里没有那一行），故下面三格里的
            // `"gain_oncc1"`／`"gain_curvecc1"` 仍在，并**新增** `"gaincc1"`（无下划线形）。
            // 裁定、代价与 8 档语料口径见 §8.6。
            "volume",
            "volume_smoothcc1",
            "volume_stepcc1",
            "volume_hdcc12",
            "volume_oncc",
            "volume_cc",
            "volume_curvecc",
            "volume_oncc1x",
            "volumeoncc1",
            "volume_cc-1",
            "volumed",
            "global_volume_oncc1",
            "master_volume_oncc1",
            "group_volume_oncc1",
            "gain_oncc1",
            "gain_curvecc1",
            "gaincc1",
        ] {
            assert_eq!(parse_modulation_name(bad), None, "{bad:?} must not match");
        }
    }

    #[test]
    fn the_amplitude_target_is_really_the_gain_family_first_target() {
        // ⭐ 任务单 `#115`：目标枚举的**声明序就是求和序**，且 `amplitude` 在**末尾**
        // （⇒ `#104`／`#106` 的五个 `ampeg_*` 顺序**逐位不变**）。
        for (position, target) in ModTarget::ALL.into_iter().enumerate() {
            assert_eq!(target.index(), position, "声明序与 index 必须一致");
        }
        assert_eq!(ModTarget::COUNT, ModTarget::ALL.len());
        assert_eq!(ModTarget::ALL[5], ModTarget::Amplitude, "新目标必须在末尾");
        // ⭐⭐ 任务单 `#116`：`#115` 那句"新目标必须在末尾"里的 `ALL[5]` 断言
        // **原样保留**（它现在钉的是"既有目标的下标一个都没动"）；新目标 `Tune`
        // 加在**它之后**（下标 6 ＝ 当时的 `COUNT − 1`）。
        //
        // ⭐ ⭐ **追加修订段（任务单 `#119`）—— 被修订的是本段原判据里
        // 「`Tune` 在 `COUNT − 1`（＝**末尾**）」那一支**（⛔ 原句一字未删，
        // 口径同台账第 278 条：保留原文 ＋ 追加修订段 ＋ 点名被修订的支）：
        // `#119` 把 [`ModTarget::Volume`] 加在 `Tune` **之后** ⇒ "末尾"现在是
        // `Volume`（`COUNT − 1`），而 `Tune` 退到 `COUNT − 2`。
        // ⚠ **被修订的只是"末尾"这个位置**：既有七个目标的下标**一个都没动**
        // ⇒ 上面那条 `ALL[5] == Amplitude` 逐位不变。
        // ⛔ **原句（历史，⛔ 不再作为断言）**：
        // `assert_eq!(ModTarget::ALL[ModTarget::COUNT - 1], ModTarget::Tune);`
        assert_eq!(ModTarget::ALL[ModTarget::COUNT - 2], ModTarget::Tune);
        assert_eq!(ModTarget::ALL[ModTarget::COUNT - 1], ModTarget::Volume);
        assert_eq!(ModTarget::Tune.index(), AmpegParam::COUNT + 1);
        assert_eq!(ModTarget::Volume.index(), AmpegParam::COUNT + 2);
        assert_eq!(ModTarget::Tune.name(), "tune");
        assert_eq!(ModTarget::Tune.ampeg(), None);
        assert!(!ModTarget::Tune.is_time(), "`tune_onccN` 的单位是音分");
        assert_eq!(ModTarget::Tune.unit(), "cents");
        assert_eq!(
            ModTarget::Tune.delta_range(),
            (-9600.0, 9600.0),
            "⭐ `tune_*ccN` 是 `pitch_onccN` 的别名 ⇒ 域随那一行（±9600 音分），\
             ⛔ 不是 `tune` 基准自己的 ±100"
        );
        assert_eq!(
            ModTarget::Amplitude.delta_range(),
            (MIN_MODULATION_DELTA, MAX_MODULATION_DELTA)
        );
        assert_eq!(ModTarget::Amplitude.unit(), "%");
        assert_eq!(ModTarget::Ampeg(AmpegParam::Attack).unit(), "seconds");
        assert_eq!(ModTarget::Ampeg(AmpegParam::Sustain).unit(), "%");
        // ⭐⭐ 任务单 `#119`：第三个被推广目标 `Volume` —— **第三个**单位类别。
        assert_eq!(ModTarget::Volume.name(), "volume");
        assert_eq!(ModTarget::Volume.ampeg(), None);
        assert!(
            !ModTarget::Volume.is_time(),
            "`volume_onccN` 的单位是 dB（⛔ 不是秒）"
        );
        assert_eq!(ModTarget::Volume.mod_unit(), ModUnit::Decibels);
        assert_eq!(ModTarget::Volume.unit(), "dB");
        assert_eq!(
            ModTarget::Volume.delta_range(),
            (-144.0, 48.0),
            "⭐ `volume_*ccN` 的域随被别名的那一行 `gain_ccN`（表行 Range `-144 to 48`、\
             Unit dB）—— 与 `Tune` 随 `pitch_onccN` 是**同一条口径**"
        );
        assert_eq!(ModUnit::Decibels.suffixes(), &[""]);
        // ⭐ `#119` 的换算：`6 dB` 在 CC 满量程 ⇒ `2 ^ (6 × log2(10) / 20)`。
        let volume_item = vec![BakedCcMod {
            target: ModTarget::Volume,
            cc: 1,
            delta: 6.0,
            curve: BakedCurve::bake(&Curve::built_in(0).expect("curve 0")),
        }];
        assert_eq!(volume_ratio_with_baked(&volume_item, None), 1.0);
        assert_eq!(
            volume_ratio_with_baked(&[], Some(&|_| 127)),
            1.0,
            "无 `volume` 项 ⇒ `0.0.exp2()` 恰 1.0（逐位不变）"
        );
        assert_eq!(
            volume_ratio_with_baked(&volume_item, Some(&|_| 127)),
            (6.0f32 * std::f32::consts::LOG2_10 / 20.0).exp2()
        );
        // ⭐ 目标过滤真的按 `target` 走：`Volume` 的项 ⛔ 不影响任何既有目标。
        for param in AmpegParam::ALL {
            let (other, _) = ampeg_value_with_baked(1.0, param, &volume_item, &|_| 127);
            assert_eq!(other, 1.0, "{param:?} 的基准值必须一动不动");
        }
        let (amp, _) = cc_value_with_baked(100.0, ModTarget::Amplitude, &volume_item, &|_| 127);
        assert_eq!(amp, 100.0, "`amplitude` 也必须一动不动");
        assert_eq!(
            tune_ratio_with_baked(&volume_item, Some(&|_| 127)),
            1.0,
            "⭐ `volume` 的项 ⛔ 不动 `tune` 的速率乘子（不同目标 ⇒ 不求和）"
        );
        // ⭐ 目标过滤真的按 `target` 走：`Tune` 的项 ⛔ 不影响任何既有目标。
        let tune_item = vec![BakedCcMod {
            target: ModTarget::Tune,
            cc: 1,
            delta: 1200.0,
            curve: BakedCurve::bake(&Curve::built_in(0).expect("curve 0")),
        }];
        for param in AmpegParam::ALL {
            let (other, _) = ampeg_value_with_baked(1.0, param, &tune_item, &|_| 127);
            assert_eq!(other, 1.0, "{param:?} 的基准值必须一动不动");
        }
        let (amp, _) = cc_value_with_baked(100.0, ModTarget::Amplitude, &tune_item, &|_| 127);
        assert_eq!(amp, 100.0, "`amplitude` 也必须一动不动");
        // ⭐ `tune` 的换算：+1200 音分在 CC 满量程 ⇒ 恰一个八度（×2）。
        assert_eq!(tune_ratio_with_baked(&tune_item, Some(&|_| 127)), 2.0);
        // ⭐ 无探针 ⇒ 恰 `1.0`（⛔ 一项 CC 都不读）。
        assert_eq!(tune_ratio_with_baked(&tune_item, None), 1.0);
        // ⭐ 有探针但表里没有 `tune` 项 ⇒ `0.0.exp2()` 恰 `1.0`（逐位不变）。
        assert_eq!(tune_ratio_with_baked(&[], Some(&|_| 127)), 1.0);
        assert_eq!(ModTarget::Amplitude.name(), "amplitude");
        assert_eq!(ModTarget::Amplitude.ampeg(), None);
        assert!(
            !ModTarget::Amplitude.is_time(),
            "`amplitude_onccN` 的单位是 %（不是秒）"
        );
        for param in AmpegParam::ALL {
            assert_eq!(ModTarget::Ampeg(param).ampeg(), Some(param));
        }
        // ⭐ 目标过滤真的按 `target` 走：`amplitude` 的项 ⛔ 不影响任何 `ampeg_*` 目标。
        let mixed = vec![BakedCcMod {
            target: ModTarget::Amplitude,
            cc: 1,
            delta: 40.0,
            curve: BakedCurve::bake(&Curve::built_in(0).expect("curve 0")),
        }];
        let (amp, unresolved) = cc_value_with_baked(100.0, ModTarget::Amplitude, &mixed, &|_| 127);
        assert_eq!((amp, unresolved), (140.0, 0));
        for param in AmpegParam::ALL {
            let (other, _) = ampeg_value_with_baked(1.0, param, &mixed, &|_| 127);
            assert_eq!(other, 1.0, "{param:?} 的基准值必须一动不动");
        }
    }

    #[test]
    fn the_unipolar_default_curve_reproduces_the_tutorial_formula() {
        // 规范（`sfz-1-midiccs`）：`offset=500` ＋ `offset_cc100=500` ⇒ 最大 1000。
        let baked = bake_ampeg_mods(
            &[CcMod {
                target: ModTarget::Ampeg(AmpegParam::Attack),
                cc: 100,
                delta: 0.5,
                curve: 0,
            }],
            Curve::built_in,
        );
        let value_at = |cc: u8| ampeg_value_with_baked(0.5, AmpegParam::Attack, &baked, &|_| cc);
        assert_eq!(
            value_at(0).0,
            0.5,
            "CC=0 ⇒ 只留基准（＝教程里的 offset=500）"
        );
        assert_eq!(
            value_at(127).0,
            1.0,
            "CC=127 ⇒ base + delta（＝教程里的 1000）"
        );
        assert_eq!(value_at(0).1, 0, "曲线 0 是可烘焙的，未施加数必须是 0");
        // 中点：curve 0 在 63.5 处恰为 0.5 ⇒ 0.5 + 0.5*0.5 = 0.75（纯算术，可逐位断言）。
        assert_eq!(value_at(63).0, 0.5 + 0.5 * (63.0 / 127.0));
    }

    #[test]
    fn the_bipolar_built_in_curve_subtracts_below_the_midpoint() {
        // 规范（`modulations/midi_ccs` 的 `<control>` 例）：curve 1 = -1..1 ⇒
        // 最大 `add 50`、最小 `subtract 50`。这里 delta = 50、base = 50 → 0 / 100。
        let bipolar = BakedCurve::bake(&Curve::built_in(1).expect("built in"));
        let baked = vec![BakedCcMod {
            target: ModTarget::Ampeg(AmpegParam::Sustain),
            cc: 12,
            delta: 50.0,
            curve: bipolar,
        }];
        let value_at = |cc: u8| ampeg_value_with_baked(50.0, AmpegParam::Sustain, &baked, &|_| cc);
        assert_eq!(value_at(0).0, 0.0, "CC 最小 ⇒ base - delta");
        assert_eq!(value_at(127).0, 100.0, "CC 最大 ⇒ base + delta");
        // 双极曲线在 0..127 上是 `-1 + (cc/127)*2`；整数 CC 到不了**恰好** 0，
        // 因此这里按插值本体逐位复算（⛔ 不写"约等于中点"这种空判据）。
        let bipolar_at = |cc: u8| -1.0f32 + (f32::from(cc) / 127.0) * 2.0;
        assert_eq!(value_at(63).0, 50.0 + 50.0 * bipolar_at(63));
        assert_eq!(value_at(64).0, 50.0 + 50.0 * bipolar_at(64));
        assert!(
            value_at(63).0 < 50.0 && value_at(64).0 > 50.0,
            "中点两侧符号必须真的反过来"
        );
    }

    #[test]
    fn an_unresolvable_curve_is_counted_and_contributes_nothing() {
        // 内建 4..=6 规范未给公式（`curve.rs` 已登记）⇒ 不可烘焙。
        let baked = bake_ampeg_mods(
            &[CcMod {
                target: ModTarget::Ampeg(AmpegParam::Attack),
                cc: 100,
                delta: 0.5,
                curve: 4,
            }],
            Curve::built_in,
        );
        assert_eq!(baked.len(), 1, "不可烘焙的项仍在表里（可报告）");
        assert!(!baked[0].curve.is_resolved());
        let (value, unresolved) = ampeg_value_with_baked(0.5, AmpegParam::Attack, &baked, &|_| 127);
        assert_eq!(value, 0.5, "不贡献任何量（⛔ 不当成曲线 0）");
        assert_eq!(unresolved, 1, "必须如实计数");
        // 反向腿：同一条项换个可烘焙的曲线 ⇒ 未施加数归 0 且取值真的变。
        let ok = bake_ampeg_mods(
            &[CcMod {
                target: ModTarget::Ampeg(AmpegParam::Attack),
                cc: 100,
                delta: 0.5,
                curve: 0,
            }],
            Curve::built_in,
        );
        let (value, unresolved) = ampeg_value_with_baked(0.5, AmpegParam::Attack, &ok, &|_| 127);
        assert_eq!((value, unresolved), (1.0, 0));
    }

    #[test]
    fn only_the_requested_param_is_modulated_and_the_order_is_the_input_order() {
        let baked = mods(&[
            (AmpegParam::Attack, 100, 0.5),
            (AmpegParam::Decay, 100, -0.25),
        ]);
        let (attack, _) = ampeg_value_with_baked(0.0, AmpegParam::Attack, &baked, &|_| 127);
        let (decay, _) = ampeg_value_with_baked(0.0, AmpegParam::Decay, &baked, &|_| 127);
        let (hold, _) = ampeg_value_with_baked(0.0, AmpegParam::Hold, &baked, &|_| 127);
        assert_eq!(attack, 0.5);
        assert_eq!(decay, -0.25, "负 delta 原样相加（⛔ 不钳位）");
        assert_eq!(hold, 0.0, "没被调制的参数一动不动");
    }

    #[test]
    fn a_baked_curve_evaluates_bit_identically_to_its_source_curve() {
        // ⭐ 非空证明：烘焙（拷贝点表）不改变求值 —— 逐位比较三个完全不同形状的点表。
        let instrument = crate::parser::parse_text(
            "<curve>curve_index=9\nv000=-0.0\nv040=0.5\nv064=1.0\nv127=0.25",
            &Default::default(),
        )
        .expect("parses");
        let source = instrument.curve(9).expect("curve 9 is defined");
        let baked = BakedCurve::bake(source);
        assert!(baked.is_resolved());
        assert_eq!(baked.index(), 9);
        for x in [0.0f32, 1.0, 39.5, 40.0, 63.0, 64.0, 95.25, 127.0] {
            assert_eq!(
                baked.value_at(x).to_bits(),
                source.value_at(x).to_bits(),
                "x = {x}"
            );
        }
        assert_eq!(
            baked.value_at(40.0).to_bits(),
            0.5f32.to_bits(),
            "已定义点逐位命中"
        );
    }

    #[test]
    fn a_curve_with_too_many_points_is_unresolved_not_truncated() {
        // 9 个点 > `MAX_BAKED_CURVE_POINTS`(8) ⇒ 不可烘焙（⛔ 不截断成前 8 个）。
        let instrument = crate::parser::parse_text(
            "<curve>curve_index=9\nv000=0\nv010=1\nv020=0\nv030=1\nv040=0\nv050=1\nv060=0\nv100=1\nv127=0",
            &Default::default(),
        )
        .expect("parses");
        let source = instrument.curve(9).expect("curve 9 is defined");
        assert_eq!(source.points().len(), 9);
        assert_eq!(BakedCurve::bake(source), BakedCurve::UNRESOLVED);
        // 反向腿：恰好 8 个点 ⇒ 烘焙成功（证明上限读的是点数，不是别的）。
        let eight = crate::parser::parse_text(
            "<curve>curve_index=9\nv000=0\nv010=1\nv020=0\nv030=1\nv040=0\nv050=1\nv060=0\nv127=1",
            &Default::default(),
        )
        .expect("parses");
        assert!(BakedCurve::bake(eight.curve(9).expect("defined")).is_resolved());
    }

    #[test]
    fn a_nan_cc_value_still_propagates_and_never_panics() {
        // `curve_value_at` 的既有契约：NaN 原样传出（⛔ 不"修成 0"）。
        let baked = BakedCurve::bake(&Curve::built_in(0).expect("built in"));
        assert!(baked.value_at(f32::NAN).is_nan());
        // 分辨率边界：没有点 ⇒ 不 panic 且恒 0.0。
        assert_eq!(BakedCurve::UNRESOLVED.value_at(64.0), 0.0);
        assert_eq!(BakedCurve::UNRESOLVED.index(), 0);
    }

    // -----------------------------------------------------------------------
    // ⭐ 力度调制（`ampeg_vel2*`；任务单 `#106`）
    // -----------------------------------------------------------------------

    #[test]
    fn the_five_vel2_names_are_recognised_and_nothing_else_is() {
        // 规范表逐字的五个 opcode（本模块文档 §4）—— 每个参数恰好一个名字。
        let cases: [(&str, AmpegParam); 5] = [
            ("ampeg_vel2attack", AmpegParam::Attack),
            ("ampeg_vel2hold", AmpegParam::Hold),
            ("ampeg_vel2decay", AmpegParam::Decay),
            ("ampeg_vel2sustain", AmpegParam::Sustain),
            ("ampeg_vel2release", AmpegParam::Release),
        ];
        for (name, param) in cases {
            assert_eq!(
                parse_modulation_name(name),
                Some((ModTarget::Ampeg(param), ModulationForm::Vel2, "")),
                "{name}：力度形态，且数字串必须是**空串**（名字里没有下标）"
            );
        }
        // 参数序与 `index()` 一致（归约层用 `index()` 当下标）。
        for (position, param) in AmpegParam::ALL.into_iter().enumerate() {
            assert_eq!(param.index(), position, "声明序与 index 必须一致");
        }
        assert_eq!(AmpegParam::COUNT, AmpegParam::ALL.len());
        // ⛔ `vel2` 在前 ⛔ 整名到此为止 ⛔ 大小写敏感 ⛔ 别名 opcode 没有 `vel2` 形态
        // ⛔ 别的 EG 族不属本模块 —— 一律 None（按未知 opcode 忽略）。
        for bad in [
            "ampeg_vel2",
            "ampeg_vel2Attack",
            "ampeg_vel2ATTACK",
            "ampeg_vel2attackcc1",
            "ampeg_vel2attack_oncc1",
            "ampeg_vel2attack_curvecc1",
            "ampeg_vel2attackx",
            "ampeg_vel2attack1",
            "ampeg_attack_vel2",
            "ampeg_attackvel2",
            "fileg_vel2attack",
            "pitcheg_vel2attack",
            "ampeg_vel2sustain%",
            "ampeg_",
            "vel2attack",
        ] {
            assert_eq!(
                parse_modulation_name(bad),
                None,
                "{bad:?} 必须不匹配（⛔ 不从兄弟形态外推）"
            );
        }
        // ⭐ 同批数据里的合法样例**单独**出现必须被接受（负判据的正对照）。
        assert_eq!(
            parse_modulation_name("ampeg_vel2decay"),
            Some((
                ModTarget::Ampeg(AmpegParam::Decay),
                ModulationForm::Vel2,
                ""
            ))
        );
    }

    #[test]
    fn the_velocity_scale_is_the_specification_division_by_127() {
        // 出处逐字：`( (eg type)_vel2(param) * velocity / 127 )`。
        assert_eq!(velocity_scale(0), 0.0);
        assert_eq!(velocity_scale(127), 1.0);
        assert_eq!(velocity_scale(63), 63.0f32 / 127.0);
        // 端点可逐位断言：64/127 与 63/127 都不是"恰好 0.5"。
        assert_eq!(velocity_scale(63).to_bits(), (63.0f32 / 127.0).to_bits());
        assert!(velocity_scale(63) < 0.5 && velocity_scale(64) > 0.5);
    }

    #[test]
    fn the_vel2_term_reproduces_the_specification_example() {
        // ⭐ 规范例逐字：`ampeg_attack=0.5` ＋ `ampeg_vel2attack=-0.4` ⇒
        // "attack of 0.5 seconds at 0 velocity and 0.1 seconds at 127 velocity"。
        let value_at = |velocity: u8| {
            ampeg_value_with_baked_and_vel2(
                0.5,
                AmpegParam::Attack,
                &[],
                Some(-0.4),
                velocity,
                None,
            )
            .0
        };
        // 端点逐位 = `base + delta × velocity/127` 的**同一表达式**（⛔ 不手抄十进制近似）。
        let expected = |velocity: u8| 0.5f32 + (-0.4f32) * (f32::from(velocity) / 127.0);
        assert_eq!(value_at(0).to_bits(), expected(0).to_bits());
        assert_eq!(value_at(127).to_bits(), expected(127).to_bits());
        assert_eq!(value_at(0), 0.5, "力度 0 ⇒ 基准值（规范原文 0.5 seconds）");
        // 127 处规范原文写 `0.1`：binary32 下是 `0.5 + (-0.4)`，与字面 `0.1f32`
        // **差一位**（`0.4f32` 不是精确的 0.4）⇒ 这里对**公式**逐位断言，对**十进制原文**
        // 只断言 1e-7 容差（⛔ 不假装两者逐位相等）。
        let at_127 = value_at(127);
        assert_eq!(at_127, 0.5f32 + (-0.4f32));
        assert!(
            (at_127 - 0.1f32).abs() < 1e-7,
            "规范原文的十进制 0.1：实测 {at_127} 偏差 {:e}",
            (at_127 - 0.1f32).abs()
        );
        // 力度项的**符号必须真的随负 delta 反向**（⛔ 不是"两个值碰巧不同"）。
        assert!(value_at(127) < value_at(0), "负 delta ⇒ 力度越大越短");
        // 反向腿：把 delta 取正 ⇒ 单调方向翻转（证明上面那条不是常数）。
        let positive = |velocity: u8| {
            ampeg_value_with_baked_and_vel2(0.5, AmpegParam::Attack, &[], Some(0.4), velocity, None)
                .0
        };
        assert!(positive(127) > positive(0));
    }

    #[test]
    fn the_vel2_term_is_added_last_in_one_fixed_order() {
        // ⭐ 求和顺序（模块文档裁决 5／6）：CC 项按 cc 升序在前、力度项在**末尾**
        // —— `((base + Σ_cc) + vel2)`。这里用**搜索**找一组让两种结合序真的分道扬镳的
        // 三元组（⛔ 不手抄"魔法常数"），再断言本实现走的是登记的那一种。
        let mut witness = None;
        'search: for base in [1.0f32, 0.1, 1.0e8, 3.0, 0.5] {
            for delta in [6.0e-8f32, 1.2e-7, 0.1, 1.0e-8, 4.0] {
                for vel2 in [6.0e-8f32, 0.1, 4.0, 1.0e-8, 2.5e-7] {
                    // 探针恒 127 ⇒ 曲线 0 的权重恰为 1.0 ⇒ cc 项 = delta。
                    let documented = (base + delta) + vel2;
                    let alternative = (base + vel2) + delta;
                    if documented.to_bits() != alternative.to_bits() {
                        witness = Some((base, delta, vel2, documented, alternative));
                        break 'search;
                    }
                }
            }
        }
        let (base, delta, vel2, documented, alternative) =
            witness.expect("必须存在一组非结合的三元组（否则下面的判据是空的）");
        let baked = [BakedCcMod {
            target: ModTarget::Ampeg(AmpegParam::Attack),
            cc: 7,
            delta,
            curve: BakedCurve::bake(&Curve::built_in(0).expect("curve 0")),
        }];
        let (value, unresolved) = ampeg_value_with_baked_and_vel2(
            base,
            AmpegParam::Attack,
            &baked,
            Some(vel2),
            127,
            Some(&|_| 127),
        );
        assert_eq!(unresolved, 0, "曲线 0 可烘焙 ⇒ 未施加数 0");
        assert_eq!(
            value.to_bits(),
            documented.to_bits(),
            "本实现必须走 `((base + cc) + vel2)`（base={base} delta={delta} vel2={vel2}）"
        );
        assert_ne!(
            value.to_bits(),
            alternative.to_bits(),
            "⭐ 有牙：`((base + vel2) + cc)` 那条**不同**的结合序必须给出不同的位模式"
        );
    }

    #[test]
    fn an_absent_vel2_opcode_does_no_arithmetic_at_all() {
        // ⭐ 模块文档裁决 7：`None`（opcode 没出现）与 `Some(0.0)`（写了 0）是两回事。
        // 取 `base = -0.0`：加 `+0.0` 会把符号位翻成正 ⇒ 位模式直接暴露"有没有做那次加法"。
        let negative_zero = -0.0f32;
        let (absent, _) = ampeg_value_with_baked_and_vel2(
            negative_zero,
            AmpegParam::Attack,
            &[],
            None,
            127,
            None,
        );
        assert_eq!(
            absent.to_bits(),
            negative_zero.to_bits(),
            "⭐ 缺失 ⇒ **一项算术都不做**（⇒ 与接线前逐位相同）"
        );
        let (present, _) = ampeg_value_with_baked_and_vel2(
            negative_zero,
            AmpegParam::Attack,
            &[],
            Some(0.0),
            127,
            None,
        );
        assert_eq!(present.to_bits(), 0.0f32.to_bits());
        assert_ne!(
            absent.to_bits(),
            present.to_bits(),
            "⭐ 有牙：`None` 与 `Some(0.0)` 必须**可区分**（⛔ 不是同一个哨兵值）"
        );
        // 反向腿：`Some(0.0)` 在**任何**力度上都不改变一个正常基准值。
        for velocity in [0u8, 1, 63, 126, 127] {
            let (value, _) = ampeg_value_with_baked_and_vel2(
                0.75,
                AmpegParam::Attack,
                &[],
                Some(0.0),
                velocity,
                None,
            );
            assert_eq!(value, 0.75, "velocity = {velocity}");
        }
    }

    #[test]
    fn the_two_inputs_add_and_never_replace_each_other() {
        // ⭐ 两支独立输入：CC 项与力度项**相加**（⛔ 不互相顶替）。
        let baked = vec![BakedCcMod {
            target: ModTarget::Ampeg(AmpegParam::Sustain),
            cc: 74,
            delta: 25.0,
            curve: BakedCurve::bake(&Curve::built_in(0).expect("curve 0")),
        }];
        // cc = 127 ⇒ +25；力度 127 ⇒ +（-10）× 1.0 = -10 ⇒ 50 + 25 - 10 = 65。
        let (value, unresolved) = ampeg_value_with_baked_and_vel2(
            50.0,
            AmpegParam::Sustain,
            &baked,
            Some(-10.0),
            127,
            Some(&|_| 127),
        );
        assert_eq!((value, unresolved), (65.0, 0));
        // 只给 CC ⇒ 75；只给力度 ⇒ 40（两个单臂都必须自洽）。
        assert_eq!(
            ampeg_value_with_baked(50.0, AmpegParam::Sustain, &baked, &|_| 127).0,
            75.0,
            "⭐ `ampeg_value_with_baked` 是 `vel2 = None` 的薄包装（⛔ 没有第二套公式）"
        );
        assert_eq!(
            ampeg_value_with_baked_and_vel2(
                50.0,
                AmpegParam::Sustain,
                &baked,
                Some(-10.0),
                127,
                None
            )
            .0,
            40.0,
            "无 CC 探针 ⇒ 只剩力度项"
        );
        // ⛔ 两支输入**各自**按参数过滤：`Sustain` 的 CC 调制项 ⛔ 不影响 `Hold`，
        // 而 `Hold` 的力度项照算（`1.0 + 5.0 × 1.0 = 6.0`）。
        let (hold, _) = ampeg_value_with_baked_and_vel2(
            1.0,
            AmpegParam::Hold,
            &baked,
            Some(5.0),
            127,
            Some(&|_| 127),
        );
        assert_eq!(hold, 6.0, "别的参数的 CC 项必须一动不动");
        // ⛔ 两支都没有的参数 ⇒ 原样基准值。
        let (release, _) = ampeg_value_with_baked_and_vel2(
            1.0,
            AmpegParam::Release,
            &baked,
            None,
            127,
            Some(&|_| 127),
        );
        assert_eq!(release, 1.0);
    }
}
