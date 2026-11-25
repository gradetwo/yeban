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
//! ⭐ ⭐ **追加修订段（T-FIX-281A）—— 被修订的是上面「⛔ 不做」那一支**（⛔ 原文一字未删，
//! 口径同台账第 278 条）：两族的 CC 支**已做（解析层）** —— `*_onccN` / `*ccN` /
//! `*_curveccN` 进 [`crate::modulation::ModTarget`]（[`crate::modulation::ModTarget::Fileg`]／
//! [`crate::modulation::ModTarget::Pitcheg`]，8 参数 × 4 后缀 × 2 族 ＝ 64 种写法，
//! 见 [`crate::modulation`] §9）；[`EgDef`] 携 CC 表（[`EgDef::cc_mods`]，由区域已归约的
//! 调制表按族过滤而来），[`EgDef::value_with_cc`] 叠加 delta，[`EgDef::is_transparent`]
//! 计入 CC 项。⭐ **逐块可行的依据**：CC 调制确实要逐块求值，但求值点不是逐样本循环 ——
//! 引擎逐块重建 `EgRun`（与 `ampeg` 的逐块烘焙同层；`read_cc_mods` 的归约结果烘焙后按目标
//! 过滤即是逐块输入），本片的解析层已把那张表备好；**引擎接线是 281B／281C**
//! （本单不碰引擎三条路）。巡检 S118 里这条的 `NotRecognized` 腿随之翻成 `Recognized`
//! （见 `yeban-engine/tests/sfz_instrument.rs` 的 S118 ②）。
//! ⚠ **仍成立的部分**：`fileg_vel2attack` 一族**仍不进** `ModTarget`（力度支，
//! 由 `read_eg_vel2` 单独归约）；`*_smoothccN`／`*_stepccN` 仍按未知 opcode 忽略。
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

use crate::curve::Curve;
use crate::modulation::{
    AMPEG_MOD_CAP, BakedCcMod, BakedCurve, CcMod, ModTarget, ModUnit, cc_value_with_baked_and_vel2,
};

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

/// 一族 CC 调制表的上限（T-FIX-281A）：本表由区域已归约的调制表按族过滤而来，
/// 而区域表本身以 [`AMPEG_MOD_CAP`] 为上限 ⇒ 一族的项数天然 ≤ 该上限。
/// （⛔ 不是新开一个数：与区域上限**同一个**常数，改一处即改两处。）
pub const EG_CC_CAP: usize = AMPEG_MOD_CAP;

/// ⭐ 一族调制包络的完整定义：基准形状 ＋ 力度调制量 ＋ CC 调制表。
///
/// `vel2` 下标 ＝ [`EgParam`] 声明序；`None` ＝ 该参数的 `*_vel2<param>`
/// **根本没出现**（缺省 ⇒ 无调制，求值期一项算术都不做，与 `ampeg_vel2` 同款）。
/// `cc_mods` 是本族的 CC 调制表（T-FIX-281A）：由区域已归约的调制表按族过滤而来
/// （⛔ 不重读 opcode ⇒ 与 [`crate::instrument::Region::cc_modulations`] 天然一致）；
/// `None` 槽 ＝ 空（⛔ 不靠 `delta == 0.0` 判空 —— 真写了 `delta=0` 的项也占一格）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EgDef {
    /// 八个基准值。
    pub params: EgParams,
    /// `*_vel2<param>` 的调制量（单位随参数，域见 [`EgParam::vel2_range`]）。
    pub vel2: [Option<f32>; EgParam::COUNT],
    /// 本族的 CC 调制项（[`ModTarget::Fileg`]／[`ModTarget::Pitcheg`]，按
    /// `(target, cc)` 升序；`None` ＝ 空槽）。
    pub cc_mods: [Option<CcMod>; EG_CC_CAP],
}

impl EgDef {
    /// 空定义（无该族任何 opcode）。
    pub const EMPTY: Self = Self {
        params: EgParams::TRANSPARENT,
        vel2: [None; EgParam::COUNT],
        cc_mods: [None; EG_CC_CAP],
    };

    /// ⭐ **静态透明**：基准全缺省 **且** 力度项一条都没有 **且** CC 项一条都没有
    /// ⇒ 任何力度、任何 CC 下都透明（摊平期据此不进侧表，与 `PitchEg::is_transparent` 同形）。
    /// ⭐ T-FIX-281A：CC 项进表 ⇒ 不透明（只写 CC 的族也要进侧表，否则逐块求值
    /// 拿不到它 —— 仅含 CC 的族按旧口径会被误判透明）。
    #[must_use]
    pub fn is_transparent(&self) -> bool {
        self.params == EgParams::TRANSPARENT
            && self.vel2.iter().all(Option::is_none)
            && self.cc_mods.iter().all(Option::is_none)
    }

    /// 某参数在给定力度下的**有效值**：`base + vel2 × velocity_scale(velocity)`
    /// （与 `ampeg` 的力度公式**同一份**，连结合序都照
    /// [`crate::modulation::velocity_scale`] 的乘法形状，⛔ 不写成
    /// `(delta × velocity) / 127` —— binary32 下两者末位可不同；
    /// 钳制由消费方按参数种类做，⛔ 不在这里夹 —— 与
    /// `ampeg_value_with_baked_and_vel2` 的分工相同）。
    /// ⭐ T-FIX-281A：CC 叠加不在本函数 —— 本函数是**无 CC**那一支（签名逐字未变，
    /// 引擎 `eg_run_from` 的调用未动）；CC 叠加见 [`Self::value_with_cc`]。
    #[must_use]
    pub fn value(&self, param: EgParam, velocity: u8) -> f32 {
        let base = self.params.base(param);
        match self.vel2[param.index()] {
            Some(delta) => base + delta * crate::modulation::velocity_scale(velocity),
            None => base,
        }
    }

    /// 某参数在给定力度 **与 CC 探针** 下的有效值：[`Self::value`] 的 CC 叠加形
    /// （T-FIX-281A）—— `base + Σ_cc delta × curve(probe(cc)) + vel2 × velocity / 127`。
    ///
    /// * 求和序与 [`cc_value_with_baked_and_vel2`] **同一份**（CC 项在前、力度项在末尾；
    ///   CC 项按本表已有的 `(target, cc)` 升序）—— 本函数是它的**薄包装**，⛔ 没有第二套公式：
    ///   本参数的项先在栈上烘成 [`BakedCcMod`]（曲线下标由 `resolve` 解析，解析不到 ⇒
    ///   [`BakedCurve::UNRESOLVED`]，计进 `unresolved`、⛔ 不贡献），再整段交给它；
    /// * `resolve` 是曲线下标的解析口（与 [`crate::modulation::bake_cc_mods`] 同款签名）：
    ///   调用方传 [`crate::instrument::Instrument::modulation_curve`] 一类的解析器；
    ///   测试里传 [`Curve::built_in`] 即可（曲线 0 是内建的）；
    /// * 本函数只做**解析层**的逐块求值辅助（触发期／测试）：引擎的逐块路用摊平表里
    ///   **预烘焙**的区域调制表（281B／281C），⛔ 不每块重烘。
    ///   无 CC 项时与 [`Self::value`] **逐位相同**（返回的 `unresolved` 也是 0）。
    ///
    /// 实时安全：栈上定长数组 ＋ 纯算术；无分配、无锁、无 I/O。
    #[must_use]
    pub fn value_with_cc(
        &self,
        param: EgParam,
        velocity: u8,
        probe: &dyn Fn(u8) -> u8,
        mut resolve: impl FnMut(u8) -> Option<Curve>,
    ) -> (f32, usize) {
        // 只取本参数的项（本表是单族的 —— `read_eg` 已按族过滤 —— 这里只按参数过滤）。
        // 目标取自第一项（同参数的项必属同一目标；表空时目标无关，传 `Fileg(param)` 占位）。
        let mut baked = [BakedCcMod::EMPTY; EG_CC_CAP];
        let mut count = 0usize;
        let mut target = ModTarget::Fileg(param);
        for entry in self.cc_mods.iter().flatten() {
            let entry_param = match entry.target {
                ModTarget::Fileg(p) | ModTarget::Pitcheg(p) => p,
                _ => continue,
            };
            if entry_param != param {
                continue;
            }
            if count == 0 {
                target = entry.target;
            }
            baked[count] = BakedCcMod {
                target: entry.target,
                cc: entry.cc,
                delta: entry.delta,
                curve: resolve(entry.curve)
                    .map_or(BakedCurve::UNRESOLVED, |curve| BakedCurve::bake(&curve)),
            };
            count += 1;
        }
        cc_value_with_baked_and_vel2(
            self.params.base(param),
            target,
            &baked[..count],
            self.vel2[param.index()],
            velocity,
            Some(probe),
        )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::Curve;

    /// 由 `(目标, CC 号, delta, 曲线下标)` 建一张 CC 表（其余槽为空）。
    fn cc_table(entries: &[(ModTarget, u8, f32, u8)]) -> [Option<CcMod>; EG_CC_CAP] {
        let mut table = [None; EG_CC_CAP];
        for (index, &(target, cc, delta, curve)) in entries.iter().enumerate() {
            table[index] = Some(CcMod {
                target,
                cc,
                delta,
                curve,
            });
        }
        table
    }

    #[test]
    fn empty_is_transparent_and_value_with_cc_matches_value() {
        assert!(EgDef::EMPTY.is_transparent());
        assert_eq!(EgDef::EMPTY.cc_mods.iter().flatten().count(), 0);
        for param in EgParam::ALL {
            for velocity in [0u8, 1, 64, 127] {
                assert_eq!(
                    EgDef::EMPTY.value_with_cc(param, velocity, &|_| 127, Curve::built_in),
                    (EgDef::EMPTY.value(param, velocity), 0),
                    "{param:?} / 力度 {velocity}：无 CC 项时 `value_with_cc` 必须与 `value` 逐位相同"
                );
            }
        }
    }

    #[test]
    fn cc_entries_make_it_non_transparent_and_value_with_cc_adds_the_delta() {
        // 基准 attack 0.5 ＋ vel2 −0.25 ＋ 两条 CC 项（curve 0 ＝ 线性 0 → 1）。
        let def = EgDef {
            params: EgParams {
                attack: 0.5,
                ..EgParams::TRANSPARENT
            },
            vel2: {
                let mut vel2 = [None; EgParam::COUNT];
                vel2[EgParam::Attack.index()] = Some(-0.25);
                vel2
            },
            cc_mods: cc_table(&[
                (ModTarget::Fileg(EgParam::Attack), 100, 0.5, 0),
                (ModTarget::Fileg(EgParam::Attack), 7, -0.25, 0),
            ]),
        };
        assert!(!def.is_transparent(), "CC 项进表 ⇒ 不透明");
        // 探针全 127 ⇒ curve0(127) ＝ 1：0.5 ＋ 0.5 − 0.25 − 0.25 ＝ 0.5。
        assert_eq!(
            def.value_with_cc(EgParam::Attack, 127, &|_| 127, Curve::built_in),
            (0.5, 0)
        );
        // 探针全 0 ⇒ curve0(0) ＝ 0：CC 项一项都不加 ⇒ 与 `value()` 相同（0.25）。
        assert_eq!(
            def.value_with_cc(EgParam::Attack, 127, &|_| 0, Curve::built_in),
            (0.25, 0)
        );
        assert_eq!(def.value(EgParam::Attack, 127), 0.25);
        // 探针只开 cc100 ⇒ 只吃第一项：0.5 ＋ 0.5 − 0.25 ＝ 0.75。
        let probe = |cc: u8| u8::from(cc == 100) * 127;
        assert_eq!(
            def.value_with_cc(EgParam::Attack, 127, &probe, Curve::built_in),
            (0.75, 0)
        );
    }

    #[test]
    fn cc_entries_for_one_param_do_not_move_another() {
        let def = EgDef {
            params: EgParams::TRANSPARENT,
            vel2: [None; EgParam::COUNT],
            cc_mods: cc_table(&[(ModTarget::Pitcheg(EgParam::Attack), 1, 5.0, 0)]),
        };
        assert!(!def.is_transparent());
        assert_eq!(
            def.value_with_cc(EgParam::Attack, 127, &|_| 127, Curve::built_in),
            (5.0, 0)
        );
        // 没被调制的参数一动不动（基准 0 ＋ 无 vel2 ＋ 无 CC 项）。
        for param in [EgParam::Delay, EgParam::Hold, EgParam::Depth] {
            assert_eq!(
                def.value_with_cc(param, 127, &|_| 127, Curve::built_in),
                (0.0, 0),
                "{param:?} 必须一动不动"
            );
        }
    }

    #[test]
    fn unresolved_curve_is_counted_and_contributes_nothing() {
        // 曲线 4 内建未给公式 ⇒ `Curve::built_in(4)` 是 `None`。
        let def = EgDef {
            params: EgParams {
                attack: 0.5,
                ..EgParams::TRANSPARENT
            },
            vel2: [None; EgParam::COUNT],
            cc_mods: cc_table(&[(ModTarget::Fileg(EgParam::Attack), 100, 0.5, 4)]),
        };
        assert_eq!(
            def.value_with_cc(EgParam::Attack, 127, &|_| 127, Curve::built_in),
            (0.5, 1),
            "不可烘焙的曲线不贡献、必须计进未施加数"
        );
        // 反向腿：同一条项换曲线 0 ⇒ 未施加数归 0 且取值真的变。
        let ok = EgDef {
            cc_mods: cc_table(&[(ModTarget::Fileg(EgParam::Attack), 100, 0.5, 0)]),
            ..def
        };
        assert_eq!(
            ok.value_with_cc(EgParam::Attack, 127, &|_| 127, Curve::built_in),
            (1.0, 0)
        );
    }

    #[test]
    fn a_zero_delta_entry_still_counts_as_a_cc_entry() {
        // `delta = 0` 的项也占一格（⛔ 不靠 `delta == 0.0` 判空）。
        let def = EgDef {
            params: EgParams::TRANSPARENT,
            vel2: [None; EgParam::COUNT],
            cc_mods: cc_table(&[(ModTarget::Pitcheg(EgParam::Depth), 3, 0.0, 0)]),
        };
        assert!(!def.is_transparent(), "`delta = 0` 的项也是 CC 项");
        assert_eq!(
            def.value_with_cc(EgParam::Depth, 127, &|_| 127, Curve::built_in),
            (0.0, 0),
            "加 0 ⇒ 取值不变，但未施加数仍是 0（曲线可烘焙）"
        );
    }

    #[test]
    fn red_control_no_cc_is_bit_identical() {
        // ⭐ 红对照：同一份基准 ＋ 力度，有 CC 表与无 CC 表的 `value()` 逐位相同，
        // 且无 CC 表的 `value_with_cc` 与 `value()` 逐位相同。
        let params = EgParams {
            attack: 0.5,
            depth: 100.0,
            ..EgParams::TRANSPARENT
        };
        let mut vel2 = [None; EgParam::COUNT];
        vel2[EgParam::Attack.index()] = Some(-0.25);
        let without_cc = EgDef {
            params,
            vel2,
            cc_mods: [None; EG_CC_CAP],
        };
        let with_cc = EgDef {
            cc_mods: cc_table(&[(ModTarget::Fileg(EgParam::Attack), 100, 0.5, 0)]),
            ..without_cc
        };
        for param in EgParam::ALL {
            for velocity in [0u8, 64, 127] {
                assert_eq!(
                    with_cc.value(param, velocity),
                    without_cc.value(param, velocity),
                    "{param:?} / 力度 {velocity}：`value()` 必须无视 CC 表"
                );
                assert_eq!(
                    without_cc.value_with_cc(param, velocity, &|_| 127, Curve::built_in),
                    (without_cc.value(param, velocity), 0),
                    "{param:?} / 力度 {velocity}：无 CC 项时 `value_with_cc` 必须与 `value()` 逐位相同"
                );
            }
        }
    }
}
