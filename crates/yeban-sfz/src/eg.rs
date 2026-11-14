//! ⭐⭐ **调制包络定义**（`fileg_*` / `pitcheg_*`；SFZ 包络族片，审计 C19／C20）。
//!
//! `fileg` 是**滤波包络**（包络电平调制静态有效截止，见 `yeban-engine` 的接线），
//! `pitcheg` 是**音高包络**（包络电平调制播放速率）。两族的 opcode 形状**逐字同构**：
//! 七个段参（`delay` / `start` / `attack` / `hold` / `decay` / `sustain` / `release`）
//! ＋ 一个深度（`depth`，音分），每个段参再带一个 `vel2<param>` 力度调制量。
//! ⇒ 本模块只定义**一份**形状（[`EgParam`] / [`EgParams`] / [`EgDef`]），
//! 两族各持一份（[`crate::instrument::Region::fileg`] / [`crate::instrument::Region::pitcheg`]），
//! ⛔ 不写两套。
//!
//! # 规范值（2026-11-14 逐页亲核 <https://sfzformat.com>）
//!
//! | opcode | Type | Default | Range | Unit |
//! | :--- | :--- | :--- | :--- | :--- |
//! | `fileg_delay` / `pitcheg_delay` | float | 0 | 0..100 | seconds |
//! | `fileg_start` / `pitcheg_start` | float | 0 | 0..100 | % |
//! | `fileg_attack` / `pitcheg_attack` | float | 0 | 0..100 | seconds |
//! | `fileg_hold` / `pitcheg_hold` | float | 0 | 0..100 | seconds |
//! | `fileg_decay` / `pitcheg_decay` | float | 0 | 0..100 | seconds |
//! | `fileg_sustain` / `pitcheg_sustain` | float | **0** | 0..100 | % |
//! | `fileg_release` / `pitcheg_release` | float | 0 | 0..100 | seconds |
//! | `fileg_depth` / `pitcheg_depth` | **integer** | 0 | −12000..12000 | cents |
//!
//! ⚠ `sustain` 的缺省是 **0**（与 `ampeg_sustain` 的 100 **不同**）：包络的静止
//! 电平由 `start` 给出，`sustain` 只是衰减段的落点。⚠ `depth` 是 **integer**
//! （与 `fil_keytrack` 同种）⇒ 解析走整数读，带小数点的值响亮报错。
//! `vel2*` 伴随域：时间类 −100..100（秒）、`start`／`sustain` −100..100（%）、
//! `depth` −12000..12000（音分）—— 与各自主 opcode 的 Modulation 表行同域。
//!
//! # 本片的范围裁决（与滤波静态族的 `cutoff_onccN` 延期同口径）
//!
//! * ✅ **做**：八个静态参 ＋ 八个 `vel2*`（触发期按声部力度折算，与 `ampeg_vel2` 同层）。
//! * ⛔ **不做**：`*_onccN` / `*ccN` / `*_curveccN` —— 本族**不进** [`crate::modulation::ModTarget`]
//!   （`parse_modulation_name("fileg_attackcc100")` 仍是 `None`，巡检 S118 的
//!   `NotRecognized` 一条照旧成立）。理由：CC 调制要逐块求值，而本族的求值点在
//!   逐样本循环（与 ampeg 的逐块烘焙不同层）；留待后续单独立项。
//!
//! # 播放器分歧（裁决写在这里，⛔ 不假装是规范）
//!
//! * **释放目标 ＝ `start` 电平**：释放段从 note-off 时刻的电平线性回到 `start`
//!   （包络的静止位），而不是回到 0 —— `depth` 偏移在触发前就是 `start × depth`，
//!   释放回到同一电平才连续。`ampeg` 的释放归零是**幅度**包络的特例，⛔ 不套到本族。
//! * **形状 ＝ 分段线性**：沿本仓 D7／`ampeg` 的既有口径（ARIA 的乘性曲线⛔ 不追），
//!   三条路（离线／摊平／实时）共用引擎里同一份帧口径归约 ⇒ 逐位一致。
//! * **`fileg` 需要静态截止**：region 没有 `cutoff` ⇒ 没有滤波可调 ⇒ `fileg`
//!   整体透明（引擎侧不产出运行束）。sfizz 在无 `cutoff` 时以缺省全开截止为基准，
//!   本仓⛔ 不取（凭空造一个滤波器会改变无 `cutoff` 区域的音色与逐位基线）。

use crate::modulation::ModUnit;

/// ⭐ 调制包络的一个参数（`fileg_*` / `pitcheg_*` 共用）。
///
/// 声明顺序是**规范的段序**（delay → start → attack → hold → decay → sustain
/// → release → depth）；它同时是 [`EgDef::vel2`] 的下标序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EgParam {
    /// `*_delay`（秒）：触发后先停在 `start` 电平的时长。
    Delay,
    /// `*_start`（百分比）：包络的起始／静止电平。
    Start,
    /// `*_attack`（秒）：自 `start` 线性升到 1 的时长。
    Attack,
    /// `*_hold`（秒）：峰值保持时长。
    Hold,
    /// `*_decay`（秒）：自峰值降到 `sustain` 的时长。
    Decay,
    /// `*_sustain`（百分比）：衰减段落点电平（⚠ 规范缺省 **0**）。
    Sustain,
    /// `*_release`（秒）：note-off 后回到 `start` 的时长。
    Release,
    /// `*_depth`（音分，整数）：电平 1.0 对应的总调制量。
    Depth,
}

impl EgParam {
    /// 参数个数（`[T; EgParam::COUNT]` 的下标域 ＝ `0..COUNT`）。
    pub const COUNT: usize = 8;

    /// 全部八个参数（声明序；判据与遍历用）。
    pub const ALL: [Self; 8] = [
        Self::Delay,
        Self::Start,
        Self::Attack,
        Self::Hold,
        Self::Decay,
        Self::Sustain,
        Self::Release,
        Self::Depth,
    ];

    /// 在 [`Self::ALL`] 里的下标（`0..=7`）。
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Delay => 0,
            Self::Start => 1,
            Self::Attack => 2,
            Self::Hold => 3,
            Self::Decay => 4,
            Self::Sustain => 5,
            Self::Release => 6,
            Self::Depth => 7,
        }
    }

    /// 不带族前缀的参数名（`fileg_attack` 里的 `attack`；诊断与名字识别用）。
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Delay => "delay",
            Self::Start => "start",
            Self::Attack => "attack",
            Self::Hold => "hold",
            Self::Decay => "decay",
            Self::Sustain => "sustain",
            Self::Release => "release",
            Self::Depth => "depth",
        }
    }

    /// 按名取参数（名字识别的唯一实现；认不出 ⇒ `None`）。
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|param| param.name() == name)
    }

    /// 该参数的**单位类别**（后缀政策与之一致，见 [`ModUnit::suffixes`]）。
    #[must_use]
    pub const fn mod_unit(self) -> ModUnit {
        match self {
            Self::Delay | Self::Attack | Self::Hold | Self::Decay | Self::Release => {
                ModUnit::Seconds
            }
            Self::Start | Self::Sustain => ModUnit::Percent,
            Self::Depth => ModUnit::Cents,
        }
    }

    /// `vel2<param>` 调制量的取值域（与主 opcode 的 Modulation 表行同域）。
    #[must_use]
    pub const fn vel2_range(self) -> (f32, f32) {
        match self.mod_unit() {
            ModUnit::Cents => (-12000.0, 12000.0),
            _ => (-100.0, 100.0),
        }
    }
}

/// ⭐ 调制包络的八个**基准值**（解析层的形状；单位见各字段）。
///
/// 全部缺省 ＝ 全 0（[`Self::TRANSPARENT`]）⇒ 该包络在任何力度下都不产生
/// 偏移的**静态**那一半（力度那一半见 [`EgDef::vel2`]）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EgParams {
    /// `*_delay`（秒；Range 0..=100，Default 0）。
    pub delay: f32,
    /// `*_start`（百分比；Range 0..=100，Default 0）。
    pub start: f32,
    /// `*_attack`（秒；Range 0..=100，Default 0）。
    pub attack: f32,
    /// `*_hold`（秒；Range 0..=100，Default 0）。
    pub hold: f32,
    /// `*_decay`（秒；Range 0..=100，Default 0）。
    pub decay: f32,
    /// `*_sustain`（百分比；Range 0..=100，Default **0**）。
    pub sustain: f32,
    /// `*_release`（秒；Range 0..=100，Default 0）。
    pub release: f32,
    /// `*_depth`（音分；Range −12000..=12000，Default 0；规范 Type ＝ integer，
    /// 解析期已验整数，这里存 `f32` 与其余段参同形做帧口径换算）。
    pub depth: f32,
}

impl EgParams {
    /// 全缺省（全 0）：没有任何 `fileg_*` / `pitcheg_*` opcode 时的形状。
    pub const TRANSPARENT: Self = Self {
        delay: 0.0,
        start: 0.0,
        attack: 0.0,
        hold: 0.0,
        decay: 0.0,
        sustain: 0.0,
        release: 0.0,
        depth: 0.0,
    };

    /// 取一个参数的基准值（与 [`EgDef::vel2`] 的下标同一套序）。
    #[must_use]
    pub const fn base(&self, param: EgParam) -> f32 {
        match param {
            EgParam::Delay => self.delay,
            EgParam::Start => self.start,
            EgParam::Attack => self.attack,
            EgParam::Hold => self.hold,
            EgParam::Decay => self.decay,
            EgParam::Sustain => self.sustain,
            EgParam::Release => self.release,
            EgParam::Depth => self.depth,
        }
    }
}

/// ⭐ 一族调制包络的完整定义：基准形状 ＋ 力度调制量。
///
/// `vel2` 下标 ＝ [`EgParam`] 声明序；`None` ＝ 该参数的 `*_vel2<param>`
/// **根本没出现**（缺省 ⇒ 无调制，求值期一项算术都不做，与 `ampeg_vel2` 同款）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EgDef {
    /// 八个基准值。
    pub params: EgParams,
    /// `*_vel2<param>` 的调制量（单位随参数，域见 [`EgParam::vel2_range`]）。
    pub vel2: [Option<f32>; EgParam::COUNT],
}

impl EgDef {
    /// 空定义（无该族任何 opcode）。
    pub const EMPTY: Self = Self {
        params: EgParams::TRANSPARENT,
        vel2: [None; EgParam::COUNT],
    };

    /// ⭐ **静态透明**：基准全缺省 **且** 力度项一条都没有
    /// ⇒ 任何力度下都透明（摊平期据此不进侧表，与 `PitchEg::is_transparent` 同形）。
    #[must_use]
    pub fn is_transparent(&self) -> bool {
        self.params == EgParams::TRANSPARENT && self.vel2.iter().all(Option::is_none)
    }

    /// 某参数在给定力度下的**有效值**：`base + vel2 × velocity_scale(velocity)`
    /// （与 `ampeg` 的力度公式**同一份**，连结合序都照
    /// [`crate::modulation::velocity_scale`] 的乘法形状，⛔ 不写成
    /// `(delta × velocity) / 127` —— binary32 下两者末位可不同；
    /// 钳制由消费方按参数种类做，⛔ 不在这里夹 —— 与
    /// `ampeg_value_with_baked_and_vel2` 的分工相同）。
    #[must_use]
    pub fn value(&self, param: EgParam, velocity: u8) -> f32 {
        let base = self.params.base(param);
        match self.vel2[param.index()] {
            Some(delta) => base + delta * crate::modulation::velocity_scale(velocity),
            None => base,
        }
    }
}

impl Default for EgDef {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl Default for EgParams {
    fn default() -> Self {
        Self::TRANSPARENT
    }
}
