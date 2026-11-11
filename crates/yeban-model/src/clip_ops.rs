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
}
