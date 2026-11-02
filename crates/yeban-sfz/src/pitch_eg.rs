//! SFZ v2 的 `egN_*`（flex envelope）里**只服务 portamento 的那一支**。
//!
//! # 为什么需要这个模块（规范事实，逐条带出处）
//!
//! SFZ **没有** portamento / glide opcode。对 `sfzformat` 的 opcode 总表逐项检索
//! （<https://sfzformat.com/opcodes/>，854 行表格）：
//!
//! > 含 `portamento` 的 opcode 名：**0 个**；含 `glide` 的 opcode 名：**0 个**。
//!
//! 生态里的做法是用**扩展 CC 140/141（key delta）＋ 一条 pitch 目标的 flex envelope**
//! 拼出来。规范扩展页原文（`docs/extensions/midi_ccs.md`，逐字）：
//!
//! > **CC 140 and 141 are measured in half-steps.** So, the below will result in an
//! > envelope with a depth of **100 cents per half-step**. This is typically what would
//! > be used for portamento glides.
//! >
//! > `eg07_pitch_oncc140=100`
//!
//! 也就是说：`egN_pitch_oncc140=<d>` 的 `<d>` 单位是 **cents / 半音**，而 CC140 的取值是
//! **当前音与上一个音的半音差**（有符号），CC141 是它的绝对值
//! （同页："keydelta: 140 / keydelta absolute: 141"）。
//!
//! # 本模块建模的子集（**其余部分明确登记为未建模**）
//!
//! | opcode | 规范 | 本模块 | 说明 |
//! | :--- | :--- | :--- | :--- |
//! | `egN_level0` | float，`-1 to 1`，默认 `0`（SFZ v2） | ✅ 读 | 包络起点电平 |
//! | `egN_level1` | 同上 | ✅ 读 | 第一段终点电平 |
//! | `egN_time1` | float，SFZ v2（规范表**无描述、无 Range**） | ✅ 读 | `level0 → level1` 的时长（秒） |
//! | `egN_pitch_oncc140` | Envelope Generators 类（规范表无描述） | ✅ 读 | 深度单位见上 |
//! | `egN_pitch_oncc141` | 同上 | ✅ 读 | 用**绝对值**半音差 |
//! | `egN_time0` | float | ⛔ 忽略 | delay 段；portamento 的实例里恒 `0` |
//! | `egN_level2` / `egN_time2` | float | ⛔ 忽略 | 第二段；到 `level1` 即保持 |
//! | `egN_sustain` | 规范表无描述 | ⛔ 忽略 | 见下面的"已知偏差" |
//! | `egN_timeX_onccY` | float | ⛔ 忽略 | 用 CC 改变滑音时长；本模块只认**静态** `time1` |
//! | `egN_*_curveccY` | integer | ⛔ 忽略 | 段曲线；本模块只做线性 |
//! | `egN_pitch_oncc<其它 CC>` | — | ⛔ 忽略 | 非 140/141 的 CC 不是半音语义 |
//!
//! ## `egN_time1` 的语义凭什么这么读
//!
//! 规范表对 `egN_timeX` **一个字都没写**。本模块的读法是从生态里两处可判读的实例
//! **反推**的，并且反过来能自洽地解释它们的每一个数值：
//!
//! ```text
//! // Meatbass（DSmolken）/ Squidpipes 的 legato 写法
//! eg06_level0=-1
//! eg06_time1=0
//! eg06_level1=0
//! eg06_pitch_oncc140=100
//! // legato 区域额外写：
//! eg06_time1_oncc109=0.3      // CC109 给滑音时长
//! ```
//!
//! * 若 `time1` 是 `level0 → level1` 的时长：默认 `0` ⇒ 立即到 0（无滑音，正确 ——
//!   首音本来就不该滑）；legato 区域拉到 `0.3` ⇒ **0.3 秒滑音**。✅ 自洽。
//! * 若 `time1` 是 `level1 → level2` 的时长：`level1` 与 `level2` 都缺省 `0`
//!   ⇒ 无论时长多长，**电平差恒为 0** ⇒ 永远滑不动。❌ 与作者的意图矛盾。
//!
//! 因此本模块采用前者。这条推理是**可判读**的（不是"凭感觉"）：判据
//! [`tests::the_ramp_reaches_level1_exactly_at_time1`] 把终点钉死，
//! [`tests::a_zero_length_ramp_is_already_at_level1`] 把 `time1 = 0` 的退化钉死。
//!
//! ## 已知偏差（登记，不静默）
//!
//! `egN_sustain` 在规范表里**没有描述**，语料里的取值（`eg06_sustain=1`）与本模块
//! "到 `level1` 即保持"的口径**结果相同**：那些实例的 `level1` 就是 `0`（滑到目标音高），
//! `sustain=1 %` 让包络停在 `0.01`，与 `0` 相差 `0.01 × depth × Δ半音`
//! （标准写法下 7 半音 × 100 cents ⇒ **7 cents**）。本模块停在 `0`
//! ⇒ 偏差上限 7 cents，且方向是"滑得更准"。若将来有库用 `sustain` 让滑音**停在半路**，
//! 本模块会滑到底 —— 那时应补 `sustain` 而不是改这里的读法。

use crate::error::SfzError;

/// portamento 用哪一路 key-delta 输入。
///
/// 两路的差别只在**符号**：CC140 保留"上行/下行"（可负），CC141 恒为正
/// （扩展页原文："keydelta: 140 / keydelta absolute: 141"）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PitchEgSource {
    /// `egN_pitch_oncc140`：CC140 ＝ 当前音 − 上一个音（**有符号**半音差）。
    KeyDelta,
    /// `egN_pitch_oncc141`：CC141 ＝ |当前音 − 上一个音|（上行与下行同向）。
    KeyDeltaAbsolute,
}

/// 一条 **pitch 专用**的 flex envelope（`egN_*` 的最小子集）。
///
/// 求值只有两步：先把时间换成包络电平（[`Self::level_at`]），再乘以
/// "深度 × 半音差"（[`Self::cents_at`]）。两步都是纯算术，实时安全。
///
/// ⚠ 电平是**归一化值**（`-1..1`），不是百分比 —— 依据是规范表的 Range
/// （`egN_levelX`：`-1 to 1`）与生态实例（`eg06_level0=-1`）双重一致。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PitchEg {
    /// `egN_level0`：包络**起点**电平（`-1..1`）。
    pub level0: f32,
    /// `egN_level1`：第一段**终点**电平（`-1..1`）。
    pub level1: f32,
    /// `egN_time1`：`level0 → level1` 的时长（**秒**，`0` ＝ 立即到达）。
    pub time1: f32,
    /// `egN_pitch_oncc140|141` 的深度（**cents / 半音**；扩展页原文的 `100`）。
    pub depth: f32,
    /// 用哪一路 key-delta。
    pub source: PitchEgSource,
}

impl PitchEg {
    /// ARIA 的 flex envelope 编号上界（`eg1_*` … `eg6_*`）。
    ///
    /// ⚠ 这是**容器界**，不是规范给的：规范只写 `egN`，没写 `N` 的域。取 `6`
    /// 是因为 ARIA 的 `<control>` 面板列出的 pitch/filter/amp 三族各两条已用满
    /// `eg1..=eg6`；越界的编号按未建模 opcode 忽略（⛔ 不猜第 7 条）。
    pub const MAX_INDEX: u8 = 6;

    /// `egN_timeX` 的取值上界（秒）。与 `ampeg_*` 各段的 `0 to 100` 同域。
    pub const MAX_TIME_SECONDS: f32 = 100.0;

    /// 组装一条 pitch envelope（**不做**任何"是否透明"的判断）。
    #[must_use]
    pub const fn new(
        level0: f32,
        level1: f32,
        time1: f32,
        depth: f32,
        source: PitchEgSource,
    ) -> Self {
        Self {
            level0,
            level1,
            time1,
            depth,
            source,
        }
    }

    /// **包络电平**：自触发起 `seconds` 秒时的输出（`-1..1`）。
    ///
    /// * `time1 <= 0` ⇒ 恒 `level1`（零长段，与 `ampeg_*` 的"零长段自然跳过"同一条口径）；
    /// * `seconds >= time1` ⇒ `level1`（到达后**保持** —— 本模块不建模 `sustain`/`level2`）；
    /// * 其余线性插值；`seconds <= 0` ⇒ `level0`（⛔ 不外推成负电平）。
    #[must_use]
    pub fn level_at(&self, seconds: f32) -> f32 {
        if self.time1 <= 0.0 {
            return self.level1;
        }
        // NaN 与 ≤ 0 同走 level0（与原 !(> 0.0) 同义；clippy 禁止偏序上的否定比较）。
        if seconds.partial_cmp(&0.0) != Some(core::cmp::Ordering::Greater) {
            return self.level0;
        }
        if seconds >= self.time1 {
            return self.level1;
        }
        self.level0 + (self.level1 - self.level0) * (seconds / self.time1)
    }

    /// **音高偏移**（音分）：`level_at(seconds) × depth × key_delta`。
    ///
    /// `key_delta` 是当前音与上一个音的**半音差**（可负）；`depth` 的单位是
    /// **cents / 半音** ⇒ 三者相乘得到音分。标准写法（`level0=-1`、`level1=0`、
    /// `depth=100`、`Δ=+7`）在 `t=0` 给出 `-700` 音分 ⇒ 从上一个音高起滑。
    #[must_use]
    pub fn cents_at(&self, seconds: f32, key_delta: f32) -> f32 {
        self.level_at(seconds) * self.depth * key_delta
    }

    /// 该包络是否**恒不改变音高**（深度 0 或两段电平都为 0）。
    ///
    /// 用于让消费方走"没有包络"的老路径 —— 这是**逐位不变**承诺的入口判据。
    #[must_use]
    pub fn is_transparent(&self) -> bool {
        self.depth == 0.0 || (self.level0 == 0.0 && self.level1 == 0.0)
    }
}

/// 解析 `egN_*` 里**带 `pitch_oncc140|141`** 的那一条；返回 `None` ＝ 该区域没有 pitch 包络。
///
/// 编号**从小到大**取第一个带 `pitch_oncc140|141` 的 `N`（同一条 region 上写了多条
/// pitch envelope 时，`eg1` 优先）—— ARIA 的语义是"多条包络各管一个目标"，
/// 两条都指 pitch 属于语料里没出现过的写法，此处按确定性选最小号，⛔ 不叠加。
///
/// # 错误
///
/// * 取值不是浮点 ⇒ [`SfzError::InvalidFloat`]；
/// * 取值越界（`level` 不在 `-1..=1`、`time` 不在 `0..=100`、`depth` 不在 `±9600`）
///   ⇒ [`SfzError::FloatOutOfRange`]。⛔ 不钳位、⛔ 不静默丢弃。
pub(crate) fn read_pitch_eg<'a>(
    scopes: &crate::instrument::Scopes<'_, 'a>,
) -> Result<Option<PitchEg>, SfzError> {
    for index in 1..=PitchEg::MAX_INDEX {
        let name140 = format!("eg{index}_pitch_oncc140");
        let name141 = format!("eg{index}_pitch_oncc141");
        // `140` 与 `141` 同时写 ⇒ 取 `140`（有符号），`141` 忽略。⛔ 不叠加、⛔ 不报错：
        // 两路是**同一件事的两种符号口径**，叠加会得到两倍深度。
        let (source, opcode) = match (scope_value(scopes, &name140), scope_value(scopes, &name141))
        {
            (Some(_), _) => (PitchEgSource::KeyDelta, name140),
            (None, Some(_)) => (PitchEgSource::KeyDeltaAbsolute, name141),
            (None, None) => continue,
        };
        let depth = read_f32(scopes, &opcode, -9600.0, 9600.0)?.unwrap_or(0.0);
        let level0 = read_f32(scopes, &format!("eg{index}_level0"), -1.0, 1.0)?.unwrap_or(0.0);
        let level1 = read_f32(scopes, &format!("eg{index}_level1"), -1.0, 1.0)?.unwrap_or(0.0);
        let time1 = read_f32(
            scopes,
            &format!("eg{index}_time1"),
            0.0,
            PitchEg::MAX_TIME_SECONDS,
        )?
        .unwrap_or(0.0);
        return Ok(Some(PitchEg::new(level0, level1, time1, depth, source)));
    }
    Ok(None)
}

/// 在 `region → group → master → global` 里查一个**动态名** opcode 的原始取值。
///
/// ⚠ 与 [`crate::instrument::Scopes::get`] 的唯一差别是名字不必是 `&'static str`
/// （`egN_*` 的 `N` 是变量）⇒ 只借原始 `&Cow<str>`，不构造 `OpcodeValue`。
fn scope_value<'m, 'a>(
    scopes: &'m crate::instrument::Scopes<'_, 'a>,
    name: &str,
) -> Option<&'m std::borrow::Cow<'a, str>> {
    [scopes.region, scopes.group, scopes.master, scopes.global]
        .into_iter()
        .find_map(|map| map.get(name))
}

/// 读一个浮点 opcode（`None` ＝ 四个作用域都没写）。
fn read_f32(
    scopes: &crate::instrument::Scopes<'_, '_>,
    name: &str,
    min: f32,
    max: f32,
) -> Result<Option<f32>, SfzError> {
    let Some(value) = scope_value(scopes, name) else {
        return Ok(None);
    };
    let parsed = value.parse::<f32>().map_err(|_| SfzError::InvalidFloat {
        line: scopes.line,
        opcode: name.to_string(),
        value: value.to_string(),
    })?;
    if !parsed.is_finite() || parsed < min || parsed > max {
        return Err(SfzError::FloatOutOfRange {
            line: scopes.line,
            opcode: name.to_string(),
            value: parsed,
            min,
            max,
        });
    }
    Ok(Some(parsed))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 标准 portamento 写法（`level0=-1`、`level1=0`、`depth=100`、`Δ=+7`）。
    fn standard() -> PitchEg {
        PitchEg::new(-1.0, 0.0, 0.3, 100.0, PitchEgSource::KeyDelta)
    }

    /// 判据：`time1` 那一刻**恰好**到 `level1`（不是"接近"）。
    ///
    /// 怎么变红：把 `level_at` 的 `seconds / self.time1` 换成 `seconds / (self.time1 * 2)`
    /// ⇒ 0.3 s 时读到 `-0.5`。
    #[test]
    fn the_ramp_reaches_level1_exactly_at_time1() {
        let eg = standard();
        assert_eq!(eg.level_at(0.0), -1.0);
        assert_eq!(eg.level_at(0.3), 0.0);
        assert_eq!(eg.level_at(0.15), -0.5);
        // 越过 `time1` 后**保持** `level1`（本模块不建模 sustain）。
        assert_eq!(eg.level_at(10.0), 0.0);
    }

    /// 判据：`time1 = 0` 是**零长段** —— 起点就已经在 `level1`（⛔ 不是"从头慢慢滑"）。
    ///
    /// 怎么变红：把 `if self.time1 <= 0.0 { return self.level1 }` 删掉 ⇒ 0/0 得 `NaN`。
    #[test]
    fn a_zero_length_ramp_is_already_at_level1() {
        let eg = PitchEg::new(-1.0, 0.0, 0.0, 100.0, PitchEgSource::KeyDelta);
        assert_eq!(eg.level_at(0.0), 0.0);
        assert!(eg.cents_at(0.0, 7.0) == 0.0);
    }

    /// 判据：音分 ＝ 电平 × 深度 × 半音差；标准写法在 `t=0` 给出 **−700 音分**。
    ///
    /// 怎么变红：把 `depth` 当成"百分比"除以 100 ⇒ 读到 `-7`。
    #[test]
    fn the_cents_are_the_level_times_depth_times_half_steps() {
        let eg = standard();
        assert_eq!(eg.cents_at(0.0, 7.0), -700.0);
        assert_eq!(eg.cents_at(0.3, 7.0), 0.0);
        // 下行 5 半音：起点比目标**高** 500 音分。
        assert_eq!(eg.cents_at(0.0, -5.0), 500.0);
        // 无前音（Δ = 0）⇒ 恒 0 音分 —— 这就是"首音不滑"的机械来源。
        assert_eq!(eg.cents_at(0.0, 0.0), -0.0);
    }

    /// 判据：`depth = 0` 或两段电平都为 0 ⇒ **透明**（消费方走老路径）。
    #[test]
    fn a_flat_envelope_is_transparent() {
        assert!(PitchEg::new(-1.0, 0.0, 0.3, 0.0, PitchEgSource::KeyDelta).is_transparent());
        assert!(PitchEg::new(0.0, 0.0, 0.3, 100.0, PitchEgSource::KeyDelta).is_transparent());
        assert!(!standard().is_transparent());
    }

    /// 判据：负时间/`NaN` 输入不让电平跑出 `[-1, 1]`。
    #[test]
    fn the_level_never_leaves_the_normalised_domain() {
        let eg = standard();
        for t in [-5.0f32, -0.0, 0.0, 0.1, 0.29, 0.3, 1.0e9] {
            let level = eg.level_at(t);
            assert!((-1.0..=0.0).contains(&level), "t={t} level={level}");
        }
    }
}
