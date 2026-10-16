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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ModTarget {
    /// 五个 `ampeg_*` 参数之一（`#104` 的原始目标、`#106` 的力度支）。
    Ampeg(AmpegParam),
    /// ⭐ `amplitude`（**线性**百分比；任务单 `#115` 的第一个增益族目标）。
    Amplitude,
}

impl ModTarget {
    /// 目标个数（[`Self::ALL`] 的长度）。
    pub const COUNT: usize = 6;

    /// 全部六个目标（声明序；判据与遍历用）。
    pub const ALL: [Self; 6] = [
        Self::Ampeg(AmpegParam::Attack),
        Self::Ampeg(AmpegParam::Hold),
        Self::Ampeg(AmpegParam::Decay),
        Self::Ampeg(AmpegParam::Sustain),
        Self::Ampeg(AmpegParam::Release),
        Self::Amplitude,
    ];

    /// 在 [`Self::ALL`] 里的下标（`0..=5`）；**这个序就是求和序**。
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Ampeg(param) => param.index(),
            Self::Amplitude => AmpegParam::COUNT,
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
        }
    }

    /// 目标是不是 `ampeg_*` 那一族；`Some(param)` ⇒ 该参数（力度支只服务这一族）。
    #[must_use]
    pub const fn ampeg(self) -> Option<AmpegParam> {
        match self {
            Self::Ampeg(param) => Some(param),
            Self::Amplitude => None,
        }
    }

    /// `delta` 的单位是不是**时间**（秒）；`false` ⇒ 百分比。
    ///
    /// 五个 `ampeg_*` 里只有 `sustain` 是百分比；⭐ [`Self::Amplitude`] 的
    /// `amplitude_onccN` 表行 Unit ＝ **%**（逐字表行见模块文档 §5）⇒ `false`。
    #[must_use]
    pub const fn is_time(self) -> bool {
        match self {
            Self::Ampeg(param) => param.is_time(),
            Self::Amplitude => false,
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
/// `amplitude_smoothccN`（表行 Type ＝ N/A）与裸 `amplitude`（没有下标）都**不是**
/// 本函数的目标（前者按未知 opcode 忽略、后者是**基准** opcode、由
/// [`crate::instrument`] 单独读）。
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
        let cases: [(&str, ModTarget, ModulationForm, &str); 9] = [
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
