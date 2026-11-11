//! 片段级人性化 / 量化的**纯计算** —— MCP 与 GUI 的唯一实现。
//!
//! `yeban_edit_notes` 的 `humanizeClip` / `quantizeClip` 与钢琴卷帘菜单的
//! 「人性化 / 量化」调的是同一份计算：
//!
//! - MCP 侧：`yeban-mcp` 的 `expand_clip_level_op` 两臂先调这里算值，
//!   再包成 `NoteOp`，由 `compile` 走批投影编译成模型 `Op`；
//! - GUI 侧：宿主（`yeban-app`）直接调这里算值，就地包成模型 `Op` 提交。
//!
//! 为什么计算住在这里（而不是 MCP 里）：默认构建没有 `yeban-mcp` 依赖边
//! （`MUST-GATE-009`：发行物默认关），宿主调不到 `notes::compile`；
//! 而把展开逻辑在宿主侧再写一份是平行实现 —— 同一种子在两处产出不同
//! 抖动是 seed 绑定的直接违背，且引擎 v2（分布替换 / 自回归记忆 /
//! 按轨派生）改展开时两份必然分叉。因此纯计算只有这一份，两侧只做
//! 各自的 `Op` 包装（那一步是机械的字段抄写，各自由自家测试钉死）。
//!
//! ## 命名口径（Wang 2026-11-11 13:00）
//!
//! 这里只标**幅度**（`time_jitter_ticks` 是 tick 数，`velocity_jitter` 是
//! ±n），⛔ 不写死分布假设：真人误差是有偏、有记忆、有结构的随机过程，
//! v2 会按层升级分布与记忆；参数透传保持通用，让 v2 直接接上。
//!
//! ## 种子口径（Wang 2026-11-11 12:44 / 13:01）
//!
//! `seed` 是调用方给的**有效种子**：同一 seed 对同一批音符永远给出
//! 同一份结果；⛔ 不用未播种的随机源。UI 只透传一个**基础种子**（缺省
//! 工程 `rng_seed`，可显式改）；每轨的有效种子派生（基础种子 ⊕ 轨标识）
//! 在引擎侧做 —— v2 线的落点也是这里（调用方在传参前派生，函数本身
//! 只认传进来的有效种子）。

use crate::EntityId;
use crate::music::{MICRO_TIMING_MAX_ABS, MIDI_VELOCITY_MAX, MidiNote};

/// 把 `start` 吸到最近的 `grid` 整数倍；`strength_permille` < 1000 时
/// 只走强度比例的那段距离（线性内插，tick 分辨率取整）。
///
/// `grid <= 1` ⇒ 原值（1 tick 的网格吸了等于没吸）；半格向上取整。
#[must_use]
pub fn quantize_tick(start: u64, grid: u64, strength_permille: u16) -> u64 {
    let snapped = if grid <= 1 {
        start
    } else {
        // 最近整数倍 = round(start / grid) * grid（半格向上）。
        (start + grid / 2) / grid * grid
    };
    let distance = i128::from(snapped) - i128::from(start);
    let scaled = i128::from(start) + distance * i128::from(strength_permille) / 1000;
    scaled.max(0) as u64
}

/// 按轨派生有效种子（Wang 2026-11-11 13:01 定：每轨种子必须不一致）。
///
/// `effective_seed = splitmix64(base_seed ⊕ 轨盐)`，轨盐 = 轨 ULID 高低
/// 64 位的嵌套 splitmix64 折叠（与 `music::trigger_hash` 同型；轨用身份、不用
/// 下标：轨重排不改变抖动）。**唯一实现**：MCP 侧
/// `yeban-mcp::domain::humanize::derive_track_seed` 与宿主都调这里，⛔ 不另写一份。
/// 调用方在传参前派生（见模块头种子口径），`humanize_note` 本身只认有效种子。
#[must_use]
pub fn derive_track_seed(base_seed: u64, track_id: &EntityId) -> u64 {
    let raw: u128 = track_id.as_ulid().0;
    #[allow(clippy::cast_possible_truncation)]
    let low = raw as u64;
    #[allow(clippy::cast_possible_truncation)]
    let high = (raw >> 64) as u64;
    crate::music::splitmix64(
        base_seed ^ crate::music::splitmix64(low ^ crate::music::splitmix64(high)),
    )
}

/// 人性化一个音符，返回 `(新微时值 tick, 新力度)`。
///
/// - `seed`：调用方给的有效种子（见模块头种子口径）。
/// - `index`：该音符在片段音符集合（`BTreeMap` 键序 = 身份序）迭代中的下标
///   —— 与 MCP 展开同一口径，顺序变了抖动就变了。
/// - `time_jitter_ticks`：微时值抖动幅度（tick，±n；结果裁进 ±240）。
///   `0` ⇒ 微时值原样不动。
/// - `velocity_jitter`：力度抖动幅度（±n；结果裁进 1..=127）。
///   `0` ⇒ 力度原样不动。
///
/// 派生是确定性的 splitmix64 式位混合（`seed ⊕ index ⊕ 通道`），
/// 同一输入永远同一输出，跨进程 / 重放稳定。
#[must_use]
pub fn humanize_note(
    note: &MidiNote,
    seed: u64,
    index: usize,
    time_jitter_ticks: u64,
    velocity_jitter: u8,
) -> (i16, u8) {
    // 确定性抖动：splitmix64(seed ⊕ index ⊕ 通道) —— 同一
    // `seed` 永远给出同一份结果，跨进程/重放稳定。
    let base_seed = seed;
    let jitter = |channel: u64| -> i64 {
        let mut x = base_seed
            .wrapping_add(0x9E37_79B9_7F4A_7C15)
            .wrapping_add((index as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9))
            .wrapping_add(channel.wrapping_mul(0x94D0_49BB_1331_11EB));
        x ^= x >> 30;
        x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x ^= x >> 27;
        x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^= x >> 31;
        // 取高 33 位，对 (2n+1) 取模后平移进 [-n, n]。
        (x >> 33) as i64
    };
    let micro = if time_jitter_ticks == 0 {
        note.micro_timing_ticks.unwrap_or(0)
    } else {
        let span = time_jitter_ticks.min(u64::try_from(MICRO_TIMING_MAX_ABS).unwrap_or(240));
        let jitter = (jitter(0) % (2 * span as i64 + 1)) - span as i64;
        let base = i64::from(note.micro_timing_ticks.unwrap_or(0));
        (base + jitter).clamp(
            -i64::from(MICRO_TIMING_MAX_ABS),
            i64::from(MICRO_TIMING_MAX_ABS),
        ) as i16
    };
    let velocity = if velocity_jitter == 0 {
        note.velocity
    } else {
        let span = i64::from(velocity_jitter);
        let jitter = (jitter(1) % (2 * span + 1)) - span;
        (i64::from(note.velocity) + jitter).clamp(1, i64::from(MIDI_VELOCITY_MAX)) as u8
    };
    (micro, velocity)
}

/// 批量力度塑形的规范缺省带（缺口报告 §1a：舒适区 65–95、重音 >100、幽灵音 35–50）。
pub const DEFAULT_COMFORT_MIN: u8 = 65;
/// 舒适区上限（缺省 95）。
pub const DEFAULT_COMFORT_MAX: u8 = 95;
/// 重音下限（缺省 101：重音 >100）。
pub const DEFAULT_ACCENT_MIN: u8 = 101;
/// 幽灵音下限（缺省 35）。
pub const DEFAULT_GHOST_MIN: u8 = 35;
/// 幽灵音上限（缺省 50）。
pub const DEFAULT_GHOST_MAX: u8 = 50;
/// 拍点网格：960 PPQ 下四分音符 = 960 tick（重音判定：旋律音落在拍点上即重音）。
pub const BEAT_TICKS: u64 = 960;
/// 力度塑形抖动的缺省幅度（±3：带内微差，不是重塑）。
pub const DEFAULT_SHAPE_JITTER: u8 = 3;
/// 力度塑形抖动的上限（±20：再大就不是微调、而是重塑，带映射失去意义）。
pub const MAX_SHAPE_JITTER: u8 = 20;
/// 和弦错开步长的上限（tick；与模型 `micro_timing_ticks` 的 ±240 上界同口径）。
pub const MAX_STAGGER_TICKS: u64 = 240;

/// 批量力度塑形的参数（`velocityShapeClip` 的字段；缺省 = 规范带）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VelocityShapeParams {
    /// 舒适区下限（缺省 65）。
    pub comfort_min: u8,
    /// 舒适区上限（缺省 95）。
    pub comfort_max: u8,
    /// 重音下限（缺省 101）。
    pub accent_min: u8,
    /// 幽灵音下限（缺省 35）。
    pub ghost_min: u8,
    /// 幽灵音上限（缺省 50）。
    pub ghost_max: u8,
    /// 施加强度千分比 0..=1000（缺省 1000 = 全额；0 = 不动，与 `quantizeClip` 同口径）。
    pub strength_permille: u16,
    /// 带内抖动幅度 ±n（缺省 3；seed 驱动、沿 tick 序阻尼）。
    pub jitter: u8,
}

impl Default for VelocityShapeParams {
    fn default() -> Self {
        Self {
            comfort_min: DEFAULT_COMFORT_MIN,
            comfort_max: DEFAULT_COMFORT_MAX,
            accent_min: DEFAULT_ACCENT_MIN,
            ghost_min: DEFAULT_GHOST_MIN,
            ghost_max: DEFAULT_GHOST_MAX,
            strength_permille: 1000,
            jitter: DEFAULT_SHAPE_JITTER,
        }
    }
}

impl VelocityShapeParams {
    /// 带顺序校验：`1 ≤ ghost_min ≤ ghost_max ≤ comfort_min ≤ comfort_max
    /// < accent_min ≤ 127`，强度 0..=1000，抖动 0..=[`MAX_SHAPE_JITTER`]。
    /// 调用方（MCP 解析期）须先判，非法 ⇒ `INVALID_PARAMETER_RANGE`。
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.ghost_min < 1
            || self.ghost_min > self.ghost_max
            || self.ghost_max > self.comfort_min
            || self.comfort_min > self.comfort_max
            || self.comfort_max >= self.accent_min
        {
            return Err(
                "力度带必须满足 1 <= ghostMin <= ghostMax <= comfortMin <= comfortMax < accentMin <= 127",
            );
        }
        if self.strength_permille > 1000 {
            return Err("strengthPermille 必须在 0..=1000");
        }
        if self.jitter > MAX_SHAPE_JITTER {
            return Err("jitter 必须在 0..=20");
        }
        Ok(())
    }
}

/// 和弦错开的方向（`chordStaggerClip` 的 `direction` 字段；缺省 `Up`）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StaggerDirection {
    /// 低音先、高音后（上行琶音；缺省）。
    #[default]
    Up,
    /// 高音先、低音后（下行琶音）。
    Down,
}

impl StaggerDirection {
    /// 解析 `direction` 字段值（小写；未知值返回 `None`，由调用方报
    /// `INVALID_PARAMETER_RANGE`）。
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "up" => Some(Self::Up),
            "down" => Some(Self::Down),
            _ => None,
        }
    }
}

/// 和弦错开的参数（`chordStaggerClip` 的字段）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChordStaggerParams {
    /// 每步标称错开量（tick，1..=240）。
    pub stagger_ticks: u64,
    /// 错开方向（缺省 `Up`）。
    pub direction: StaggerDirection,
    /// 每音附加抖动幅度 ±n tick（缺省 0；seed 驱动、组内阻尼、单调钳制）。
    pub jitter_ticks: u64,
}

impl ChordStaggerParams {
    /// 取值校验：步长 1..=240，抖动 0..=240。
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.stagger_ticks < 1 || self.stagger_ticks > MAX_STAGGER_TICKS {
            return Err("staggerTicks 必须在 1..=240");
        }
        if self.jitter_ticks > u64::try_from(MICRO_TIMING_MAX_ABS).unwrap_or(240) {
            return Err("jitterTicks 必须在 0..=240");
        }
        Ok(())
    }
}

/// 双参 splitmix64 混合（与 `yeban-mcp` 的 `humanize::splitmix64_mix` /
/// `yeban-theory` 的 `splitmix64(seed, salt)` 同一公式：终结器
/// `seed + salt·GAMMA + GAMMA`）。
#[must_use]
fn splitmix64_mix(seed: u64, salt: u64) -> u64 {
    crate::music::splitmix64(
        seed.wrapping_add(salt.wrapping_mul(0x9E37_79B9_7F4A_7C15))
            .wrapping_add(0x9E37_79B9_7F4A_7C15),
    )
}

/// seed 驱动的单点偏差：`[-amplitude, +amplitude]` 内的确定性整数。
///
/// `amplitude == 0` ⇒ 0（不耗熵，输出仍确定）。
fn seeded_deviation(seed: u64, salt: u64, amplitude: u64) -> i64 {
    if amplitude == 0 {
        return 0;
    }
    let span = 2 * amplitude + 1;
    ((splitmix64_mix(seed, salt) >> 33) % span) as i64 - amplitude as i64
}

/// 阻尼抖动序列：逐点 seed 偏差，再按相邻步 `≤ max(1, 幅度 / 4)` 限摆
/// （细粒度阻尼，Wang 2026-11-11 定；首点以前一值 0 为参考，不从远处起跳）。
fn damped_deviations(seed: u64, salt_base: u64, count: usize, amplitude: u64) -> Vec<i64> {
    let slew = (amplitude / 4).max(1) as i64;
    let mut prev = 0i64;
    let mut out = Vec::with_capacity(count);
    for index in 0..count {
        #[allow(clippy::cast_possible_wrap)]
        let salt = salt_base.wrapping_add(index as u64);
        let raw = seeded_deviation(seed, salt, amplitude);
        let damped = raw.clamp(prev - slew, prev + slew);
        prev = damped;
        out.push(damped);
    }
    out
}

/// 线性映射：把 `value` 从 `[src_lo, src_hi]` 保序映到 `[dst_lo, dst_hi]`
///（`value` 先钳进源区间；源区间退化 ⇒ `dst_lo`）。
fn linear_map(value: u8, src_lo: u8, src_hi: u8, dst_lo: u8, dst_hi: u8) -> u8 {
    if src_hi <= src_lo {
        return dst_lo;
    }
    let clamped = value.clamp(src_lo, src_hi);
    let numerator = u32::from(clamped - src_lo) * u32::from(dst_hi - dst_lo);
    let mapped = u32::from(dst_lo) + numerator / u32::from(src_hi - src_lo);
    u8::try_from(mapped).unwrap_or(dst_hi)
}

/// 和弦内的声部（按同 `start_tick` 分组内的音高排位定：最低 = bass、
/// 最高 = melody、中间 = accompaniment；单音组 = melody）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Voice {
    Melody,
    Bass,
    Accompaniment,
}

/// 批量力度塑形：把每个音符的力度映进它所属声部/角色的目标带。
///
/// - `notes`：**必须**按 `(start_tick, pitch, id)` 排好序（MCP 展开侧排；
///   同 `start_tick` 的连续段即一个和弦组，组内已按音高升序）。
/// - `seed`：调用方给的**有效种子**（MCP 侧已按轨派生；同（seed，轨）同结果）。
/// - 返回与 `notes` 一一对应的新力度（1..=127）。
///
/// 角色判定（确定性，不耗 seed）：
/// - 幽灵音：原力度 ≤ `ghost_max` ⇒ 映进 `[ghost_min, ghost_max]`；
/// - 重音：原力度 ≥ `accent_min`，或旋律音落在拍点（`start_tick % 960 == 0`）
///   ⇒ 映进 `[accent_min, 127]`；
/// - 其余按声部：melody ⇒ `[comfort_min+10, comfort_max]`、bass ⇒
///   `[comfort_min, comfort_min+15]`、accompaniment ⇒ `[comfort_min,
///   comfort_max-10]`（带端钳进舒适区内，不越界）。
///   带内保序：原力度在源区间 `[ghost_max+1, 127]`（幽灵音 `[1, ghost_max]`、
///   重音 `[accent_min, 127]`）内的相对位置线性映进目标带。
///   之后按 `strength_permille` 在原力度与目标间内插，再加 seed 驱动的带内
///   抖动（±`jitter`，沿 tick 序阻尼，相邻步 ≤ `max(1, jitter / 4)`）。
///
/// # Panics
///
/// 参数未通过 [`VelocityShapeParams::validate`] 时 debug 断言失败
/// （MCP 解析期已判；GUI 调用方同样须先判）。
#[must_use]
pub fn shape_velocities(notes: &[&MidiNote], seed: u64, params: &VelocityShapeParams) -> Vec<u8> {
    debug_assert!(params.validate().is_ok());
    let comfort_min = u16::from(params.comfort_min);
    let comfort_max = u16::from(params.comfort_max);
    let bands = |voice: Voice| -> (u8, u8) {
        match voice {
            Voice::Melody => (
                u8::try_from(comfort_min.saturating_add(10).min(comfort_max))
                    .unwrap_or(params.comfort_max),
                params.comfort_max,
            ),
            Voice::Bass => (
                params.comfort_min,
                u8::try_from(comfort_min.saturating_add(15).min(comfort_max))
                    .unwrap_or(params.comfort_max),
            ),
            Voice::Accompaniment => (
                params.comfort_min,
                u8::try_from(comfort_max.saturating_sub(10).max(comfort_min))
                    .unwrap_or(params.comfort_min),
            ),
        }
    };
    // 抖动盐域 `0x7…`：与错开的盐域（`0x5…` / `0x6…`）互不重叠。
    let jitter = damped_deviations(
        seed,
        0x7000_0000_0000_0000,
        notes.len(),
        u64::from(params.jitter),
    );
    let mut out = Vec::with_capacity(notes.len());
    let mut index = 0usize;
    while index < notes.len() {
        let start = notes[index].start_tick;
        let mut end = index + 1;
        while end < notes.len() && notes[end].start_tick == start {
            end += 1;
        }
        // 组内已按音高升序：首 = bass、尾 = melody、中间 = accompaniment。
        for position in index..end {
            let note = notes[position];
            let voice = if end - index == 1 {
                Voice::Melody
            } else if position == index {
                Voice::Bass
            } else if position + 1 == end {
                Voice::Melody
            } else {
                Voice::Accompaniment
            };
            let is_ghost = note.velocity <= params.ghost_max;
            let is_accent = !is_ghost
                && (note.velocity >= params.accent_min
                    || (voice == Voice::Melody && note.start_tick.is_multiple_of(BEAT_TICKS)));
            let mapped = if is_accent {
                linear_map(
                    note.velocity,
                    params.accent_min,
                    MIDI_VELOCITY_MAX,
                    params.accent_min,
                    MIDI_VELOCITY_MAX,
                )
            } else if is_ghost {
                linear_map(
                    note.velocity,
                    1,
                    params.ghost_max,
                    params.ghost_min,
                    params.ghost_max,
                )
            } else {
                let (lo, hi) = bands(voice);
                linear_map(
                    note.velocity,
                    params.ghost_max.saturating_add(1),
                    MIDI_VELOCITY_MAX,
                    lo,
                    hi,
                )
            };
            let blended = i32::from(note.velocity)
                + (i32::from(mapped) - i32::from(note.velocity))
                    * i32::from(params.strength_permille)
                    / 1000;
            let shaped = (blended + i32::try_from(jitter[position]).unwrap_or(0))
                .clamp(1, i32::from(MIDI_VELOCITY_MAX));
            out.push(u8::try_from(shaped).unwrap_or(params.comfort_min));
        }
        index = end;
    }
    out
}

/// 和弦错开：同 `start_tick` 的和弦组内，各音按方向依次延迟，返回每音的新
/// 微时值（`micro_timing_ticks`，±240 钳制）。
///
/// - `notes`：**必须**按 `(start_tick, pitch, id)` 排好序（与 [`shape_velocities`] 同口径）。
/// - `seed`：调用方给的**有效种子**（MCP 侧已按轨派生）。
/// - 返回与 `notes` 一一对应的新微时值（原微时值 + 延迟；单音组不动）。
///
/// 保证（`humanizeClip` 的偶然错开做不到的两点）：
/// - **单调**：组内总微时值严格递增（首音锚定原微时值，之后每音至少比前一音晚 1 tick）；
/// - **seed 绑定**：每和弦实际步长 = `stagger_ticks` ±25%（seed 决定，不同
///   和弦步长不同，像真人琶音）；附加抖动（`jitter_ticks`，缺省 0）同样
///   seed 驱动、组内阻尼（相邻步 ≤ `max(1, jitter / 4)`），再经单调钳制。
///
/// # Panics
///
/// 参数未通过 [`ChordStaggerParams::validate`] 时 debug 断言失败
/// （MCP 解析期已判；GUI 调用方同样须先判）。
#[must_use]
pub fn stagger_micro_offsets(
    notes: &[&MidiNote],
    seed: u64,
    params: &ChordStaggerParams,
) -> Vec<i16> {
    debug_assert!(params.validate().is_ok());
    let cap = i64::from(MICRO_TIMING_MAX_ABS);
    let mut out: Vec<i16> = notes
        .iter()
        .map(|note| note.micro_timing_ticks.unwrap_or(0))
        .collect();
    let mut index = 0usize;
    let mut chord_ordinal = 0u64;
    while index < notes.len() {
        let start = notes[index].start_tick;
        let mut end = index + 1;
        while end < notes.len() && notes[end].start_tick == start {
            end += 1;
        }
        if end - index >= 2 {
            // 步长盐域 `0x5…`：每和弦实际步长 = 标称 ±25%（seed 决定）。
            let quarter = (params.stagger_ticks / 4) as i64;
            let step_delta = seeded_deviation(
                seed,
                0x5000_0000_0000_0000_u64.wrapping_add(chord_ordinal),
                quarter as u64,
            );
            let step = (params.stagger_ticks as i64 + step_delta).clamp(1, cap);
            // 抖动盐域 `0x6…`：组内逐音，阻尼后仍经单调钳制。
            let group_len = end - index;
            let jitter = damped_deviations(
                seed,
                0x6000_0000_0000_0000_u64.wrapping_add(chord_ordinal.wrapping_mul(1024)),
                group_len,
                params.jitter_ticks,
            );
            // 组内顺序：Up = 音高升序（切片序），Down = 反序。
            let mut order: Vec<usize> = (index..end).collect();
            if params.direction == StaggerDirection::Down {
                order.reverse();
            }
            let mut prev_total = 0i64;
            for (position, note_index) in order.iter().enumerate() {
                let base = i64::from(notes[*note_index].micro_timing_ticks.unwrap_or(0));
                // 单调以**总微时值**计（原微时值 + 错开量）：原微时值已错开的和弦
                // 不会被"延迟严格递增"再反向——首音锚定原值，其余至少比前一音晚 1 tick。
                let total = if position == 0 {
                    base
                } else {
                    let raw = base + position as i64 * step + jitter[*note_index - index];
                    raw.max(prev_total + 1)
                };
                prev_total = total;
                out[*note_index] = total.clamp(-cap, cap) as i16;
            }
        }
        chord_ordinal += 1;
        index = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use super::*;
    use crate::EntityId;
    use crate::music::MidiNote;

    fn note(velocity: u8, micro: Option<i16>) -> MidiNote {
        MidiNote {
            id: EntityId::from_str("01J8Z5Q0R7K3M9X2V4B6N8P1F9").expect("fixture ulid"),
            start_tick: 0,
            duration_ticks: 240,
            pitch: 60,
            velocity,
            channel: None,
            probability: None,
            ratchet: None,
            micro_timing_ticks: micro,
            slide: None,
            pitch_bend_curve: Vec::new(),
            syllable: None,
            phonemes: Vec::new(),
        }
    }

    #[test]
    fn quantize_tick_snaps_to_the_nearest_grid_line() {
        // 半格向上：120 正好半格 ⇒ 240；100 ⇒ 0；130 ⇒ 240。
        assert_eq!(quantize_tick(120, 240, 1000), 240);
        assert_eq!(quantize_tick(100, 240, 1000), 0);
        assert_eq!(quantize_tick(130, 240, 1000), 240);
        assert_eq!(quantize_tick(360, 240, 1000), 480);
        // 已在格上 ⇒ 不动。
        assert_eq!(quantize_tick(480, 240, 1000), 480);
        // 强度 0 ⇒ 一位不动；强度一半 ⇒ 走一半距离。
        assert_eq!(quantize_tick(100, 240, 0), 100);
        assert_eq!(quantize_tick(100, 240, 500), 50);
        // grid <= 1 ⇒ 不吸。
        assert_eq!(quantize_tick(123, 1, 1000), 123);
        assert_eq!(quantize_tick(123, 0, 1000), 123);
    }

    #[test]
    fn humanize_note_is_deterministic_for_the_same_inputs() {
        let n = note(100, None);
        let first = humanize_note(&n, 42, 0, 20, 10);
        let second = humanize_note(&n, 42, 0, 20, 10);
        assert_eq!(first, second, "同一 seed 必须给出同一份结果");
        // 下标不同 ⇒ 派生不同（同一 seed 下各音符不拿同一份抖动）。
        let other_index = humanize_note(&n, 42, 1, 20, 10);
        assert_ne!(first, other_index);
        // seed 不同 ⇒ 派生不同。
        let other_seed = humanize_note(&n, 43, 0, 20, 10);
        assert_ne!(first, other_seed);
    }

    #[test]
    fn humanize_note_zero_jitter_leaves_the_note_unchanged() {
        let n = note(100, Some(-30));
        assert_eq!(humanize_note(&n, 42, 0, 0, 0), (-30, 100));
        // 只开一侧时另一侧不动。
        let (micro, velocity) = humanize_note(&n, 42, 0, 20, 0);
        assert_eq!(velocity, 100);
        let _ = micro;
        let (micro, velocity) = humanize_note(&n, 42, 0, 0, 10);
        assert_eq!(micro, -30);
        let _ = velocity;
    }

    #[test]
    fn humanize_note_results_stay_inside_the_clamps() {
        // 极端幅度 + 极端基值：微时值不出 ±240，力度不出 1..=127。
        let loud = note(127, Some(240));
        let quiet = note(1, Some(-240));
        for seed in [0u64, 1, 42, u64::MAX] {
            for index in [0usize, 1, 7, 100] {
                let (micro, velocity) = humanize_note(&loud, seed, index, 10_000, 127);
                assert!((-240..=240).contains(&micro), "micro {micro} 越界");
                assert!((1..=127).contains(&velocity), "velocity {velocity} 越界");
                let (micro, velocity) = humanize_note(&quiet, seed, index, 10_000, 127);
                assert!((-240..=240).contains(&micro), "micro {micro} 越界");
                assert!((1..=127).contains(&velocity), "velocity {velocity} 越界");
            }
        }
    }

    #[test]
    fn humanize_note_matches_the_independently_computed_vector() {
        // 金向量：由 Python 按同一位运算独立手算（非本函数自循环），
        // seed=42、index=0、时值幅度 20、力度幅度 10、力度 100、无微时值。
        // 变动常量或推导 ⇒ 本测试先红（MCP 侧的确定性测试随后也会红）。
        let n = note(100, None);
        assert_eq!(humanize_note(&n, 42, 0, 20, 10), (6, 92));
    }

    fn shaped_note(
        id_index: u128,
        start_tick: u64,
        pitch: u8,
        velocity: u8,
        micro: Option<i16>,
    ) -> MidiNote {
        MidiNote {
            id: EntityId::from_ulid(ulid::Ulid::from(id_index)),
            start_tick,
            duration_ticks: 240,
            pitch,
            velocity,
            channel: None,
            probability: None,
            ratchet: None,
            micro_timing_ticks: micro,
            slide: None,
            pitch_bend_curve: Vec::new(),
            syllable: None,
            phonemes: Vec::new(),
        }
    }

    #[test]
    fn velocity_shape_params_validate_rejects_bad_bands() {
        assert!(VelocityShapeParams::default().validate().is_ok());
        // 带顺序错乱（ghostMax > comfortMin）。
        let bad = VelocityShapeParams {
            ghost_max: 70,
            ..Default::default()
        };
        assert!(bad.validate().is_err());
        // 重音下限压进舒适区（comfortMax >= accentMin）。
        let bad = VelocityShapeParams {
            accent_min: 90,
            ..Default::default()
        };
        assert!(bad.validate().is_err());
        // 强度与抖动越界。
        let bad = VelocityShapeParams {
            strength_permille: 1001,
            ..Default::default()
        };
        assert!(bad.validate().is_err());
        let bad = VelocityShapeParams {
            jitter: MAX_SHAPE_JITTER + 1,
            ..Default::default()
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn shape_velocities_maps_each_voice_into_its_band() {
        // 和弦（tick 0，力度 100）：bass 48 / 内声部 60 / melody 72（拍点 ⇒ 重音）；
        // 幽灵音（tick 960，力度 40）；已是重音的单音（tick 1920，力度 110）；
        // 拍外的旋律单音（tick 480，力度 80）。jitter 0、强度 1000 ⇒ 纯映射值。
        let notes = [
            shaped_note(1, 0, 48, 100, None),
            shaped_note(2, 0, 60, 100, None),
            shaped_note(3, 0, 72, 100, None),
            shaped_note(4, 480, 67, 80, None),
            shaped_note(5, 960, 64, 40, None),
            shaped_note(6, 1920, 72, 110, None),
        ];
        let refs: Vec<&MidiNote> = notes.iter().collect();
        let params = VelocityShapeParams {
            jitter: 0,
            ..VelocityShapeParams::default()
        };
        let shaped = shape_velocities(&refs, 42, &params);
        // bass：源 [51,127] → [65,80]：65 + 49×15/76 = 74。
        assert_eq!(shaped[0], 74);
        // 内声部：源 [51,127] → [65,85]：65 + 49×20/76 = 77。
        assert_eq!(shaped[1], 77);
        // 拍点旋律音 ⇒ 重音带 [101,127]：力度 100 先钳到源下限 101 ⇒ 101。
        assert_eq!(shaped[2], 101);
        // 拍外旋律单音：源 [51,127] → [75,95]：75 + 29×20/76 = 82。
        assert_eq!(shaped[3], 82);
        // 幽灵音：源 [1,50] → [35,50]：35 + 39×15/49 = 46。
        assert_eq!(shaped[4], 46);
        // 已是重音：源 [101,127] → [101,127]：101 + 9×26/26 = 110。
        assert_eq!(shaped[5], 110);
    }

    #[test]
    fn shape_velocities_strength_zero_leaves_velocities_unchanged() {
        let notes = [
            shaped_note(1, 0, 48, 100, None),
            shaped_note(2, 0, 72, 100, None),
            shaped_note(3, 960, 64, 40, None),
        ];
        let refs: Vec<&MidiNote> = notes.iter().collect();
        let params = VelocityShapeParams {
            strength_permille: 0,
            jitter: 0,
            ..VelocityShapeParams::default()
        };
        assert_eq!(shape_velocities(&refs, 42, &params), vec![100, 100, 40]);
    }

    #[test]
    fn shape_velocities_is_deterministic_and_seed_sensitive() {
        let notes: Vec<MidiNote> = (0..16)
            .map(|index| shaped_note(index, u64::try_from(index).unwrap_or(0) * 480, 60, 90, None))
            .collect();
        let refs: Vec<&MidiNote> = notes.iter().collect();
        let params = VelocityShapeParams::default();
        let first = shape_velocities(&refs, 42, &params);
        let second = shape_velocities(&refs, 42, &params);
        assert_eq!(first, second, "同一 seed 必须给出同一份塑形");
        assert_ne!(
            first,
            shape_velocities(&refs, 43, &params),
            "不同 seed 的抖动序列必须不同"
        );
        assert!(first.iter().all(|velocity| (1..=127).contains(velocity)));
    }

    #[test]
    fn shape_velocities_jitter_is_damped() {
        // 强度 0 ⇒ 塑形只剩抖动：终值 − 64 即逐音抖动，须服从阻尼（±20 ⇒ 步 ≤ 5）。
        let notes: Vec<MidiNote> = (0..32)
            .map(|index| shaped_note(index, u64::try_from(index).unwrap_or(0) * 480, 60, 64, None))
            .collect();
        let refs: Vec<&MidiNote> = notes.iter().collect();
        let params = VelocityShapeParams {
            strength_permille: 0,
            jitter: MAX_SHAPE_JITTER,
            ..VelocityShapeParams::default()
        };
        let shaped = shape_velocities(&refs, 42, &params);
        let jitter: Vec<i64> = shaped.iter().map(|value| i64::from(*value) - 64).collect();
        assert_eq!(jitter.len(), 32);
        assert!(jitter.iter().all(|value| value.abs() <= 20));
        let max_step = jitter
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .max()
            .unwrap_or(0);
        assert!(max_step <= 5, "抖动相邻步 {max_step} 超过 20/4 = 5");
    }

    #[test]
    fn stagger_direction_parse() {
        assert_eq!(StaggerDirection::parse("up"), Some(StaggerDirection::Up));
        assert_eq!(
            StaggerDirection::parse("down"),
            Some(StaggerDirection::Down)
        );
        assert_eq!(StaggerDirection::parse("sideways"), None);
        assert_eq!(StaggerDirection::parse("UP"), None);
    }

    #[test]
    fn chord_stagger_params_validate() {
        let ok = ChordStaggerParams {
            stagger_ticks: 12,
            direction: StaggerDirection::Up,
            jitter_ticks: 0,
        };
        assert!(ok.validate().is_ok());
        assert!(
            ChordStaggerParams {
                stagger_ticks: 0,
                ..ok
            }
            .validate()
            .is_err()
        );
        assert!(
            ChordStaggerParams {
                stagger_ticks: 241,
                ..ok
            }
            .validate()
            .is_err()
        );
        assert!(
            ChordStaggerParams {
                jitter_ticks: 241,
                ..ok
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn stagger_micro_offsets_are_strictly_increasing_within_each_chord() {
        // 四音和弦（tick 0）+ 三音和弦（tick 960）+ 单音（tick 1920，不动）。
        let notes = [
            shaped_note(1, 0, 48, 100, None),
            shaped_note(2, 0, 60, 100, None),
            shaped_note(3, 0, 64, 100, None),
            shaped_note(4, 0, 72, 100, None),
            shaped_note(5, 960, 55, 100, Some(10)),
            shaped_note(6, 960, 67, 100, None),
            shaped_note(7, 960, 79, 100, None),
            shaped_note(8, 1920, 60, 100, Some(-5)),
        ];
        let refs: Vec<&MidiNote> = notes.iter().collect();
        let params = ChordStaggerParams {
            stagger_ticks: 12,
            direction: StaggerDirection::Up,
            jitter_ticks: 0,
        };
        let offsets = stagger_micro_offsets(&refs, 42, &params);
        // 首音锚定 0；步长 = 12 ± 3；严格递增。
        assert_eq!(offsets[0], 0);
        assert!((9..=15).contains(&offsets[1]), "第二音延迟 {}", offsets[1]);
        assert!(offsets[2] > offsets[1] && offsets[3] > offsets[2]);
        assert!(offsets[3] <= 45, "末音延迟 {} 不超过 3×15", offsets[3]);
        // 第二和弦：首音保留原微时值 10，其余严格递增。
        assert_eq!(offsets[4], 10);
        assert!(offsets[5] > offsets[4] && offsets[6] > offsets[5]);
        // 单音不动。
        assert_eq!(offsets[7], -5);
        // 下行：最低音最晚。
        let down = ChordStaggerParams {
            direction: StaggerDirection::Down,
            ..params
        };
        let offsets = stagger_micro_offsets(&refs, 42, &down);
        assert_eq!(offsets[3], 0);
        assert!(offsets[0] > offsets[1] && offsets[1] > offsets[2] && offsets[2] > 0);
    }

    #[test]
    fn stagger_micro_offsets_are_deterministic_and_clamped() {
        let notes: Vec<MidiNote> = (0..8)
            .map(|index| shaped_note(index, 0, 48 + u8::try_from(index).unwrap_or(0), 100, None))
            .collect();
        let refs: Vec<&MidiNote> = notes.iter().collect();
        let params = ChordStaggerParams {
            stagger_ticks: 240,
            direction: StaggerDirection::Up,
            jitter_ticks: 240,
        };
        let first = stagger_micro_offsets(&refs, 42, &params);
        let second = stagger_micro_offsets(&refs, 42, &params);
        assert_eq!(first, second, "同一 seed 必须给出同一份错开");
        assert_eq!(first[0], 0);
        assert!(first.iter().all(|value| (-240..=240).contains(value)));
        // 严格递增（钳制后仍单调：后音至少比前音晚 1 tick，直到顶到 240）。
        for pair in first.windows(2) {
            assert!(
                pair[1] > pair[0] || pair[1] == 240,
                "延迟必须严格递增: {first:?}"
            );
        }
    }
}
