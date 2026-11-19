//! **鼓机生成层**（一期）：风格预置 + 三旋钮 + 种子 ⇒ 确定性音符列。
//!
//! 生成 = 模板库（[`crate::drum_preset::DrumStylePreset`]）+ 规则变换：
//!
//! - **复杂度**（0..=100）选 tier 层：`<34` 只取核心层（tier 0），
//!   `34..=66` 取核心 + 标准层，`>=67` 全层，且军鼓道按种子哈希补
//!   **鬼音**（邻格有主击的空格、低力度），密度随复杂度增长。
//! - **强度**（0..=100）缩放力度：系数 `0.55 + 0.45 * 强度/100`，
//!   强度 100 = 预置原力度，强度 0 = 压到 55%。
//! - **摇摆**（千分比 500..=1000，缺省取预置定值）按 [`crate::swing`]
//!   的对切分口径推迟每个奇数 16 分格。
//! - **ratchet**（含 hat roll）在事件层展开：一格等分为 ratchet 个
//!   子音，子音力度系数 `0.85 + 0.15 * k / ratchet`（见预置格式
//!   `drum_styles/FORMAT.md` 的约定）。
//! - **分层音色**（P6）：一道可带叠层键，与主道同格同刻触发；逐层
//!   带电平（dB，振幅口径缩放写力度）与力度窗（对**写力度**判定）。
//!   叠层跟随该格的 tier 门与 ratchet 展开；鬼音只作用于主道。
//!   无叠层的道生成结果与引入本功能前逐位一致。
//! - **fill**：请求且预置带 fill 且总小节 ≥ 2 时，最后一小节整段换成
//!   fill 道表（fill 里没有的道在该小节休止）。
//!
//! 确定性契约：同预置 + 同参数 ⇒ 输出逐项一致（逐字节可复现的判据
//! 在 MCP 层以片段字节对账）。唯一的随机源是鬼音判定，它是
//! `(seed, gm_key, 全局步)` 的纯哈希，不依赖迭代顺序与全局状态。
//! 本模块不读时钟、不做 I/O。

use thiserror::Error;

use crate::TheoryError;
use crate::drum_preset::{DrumLanePreset, DrumStylePreset, STEPS_PER_BAR};
use crate::swing::swung_onset_offset;

/// 一小节的 tick 数（PPQ 960 × 4 拍）。
pub const TICKS_PER_BAR: u64 = crate::progression::PPQ * 4;

/// 一个 16 分步的 tick 数。
pub const STEP_TICKS: u64 = TICKS_PER_BAR / STEPS_PER_BAR as u64;

/// 复杂度旋钮的合法上限（含）。
pub const KNOB_MAX: u8 = 100;

/// 生成参数（三旋钮 + 种子 + 小节数 + fill 开关）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrumGenParams {
    /// 复杂度 0..=100（密度/鬼音）。
    pub complexity: u8,
    /// 强度 0..=100（力度层级）。
    pub intensity: u8,
    /// 摇摆千分比 500..=1000；`None` = 用预置定值。
    pub swing_permille: Option<u16>,
    /// 种子（鬼音哈希的盐；同种子同参数逐字节可复现）。
    pub seed: u64,
    /// 生成小节数 1..=16（预置 pattern 循环取用）。
    pub bars: u32,
    /// 是否在最后一小节落 fill（预置无 fill 时无效果）。
    pub fill: bool,
}

impl Default for DrumGenParams {
    fn default() -> Self {
        Self {
            complexity: 50,
            intensity: 80,
            swing_permille: None,
            seed: 0,
            bars: 4,
            fill: true,
        }
    }
}

/// 生成出的一个鼓音符（绝对 tick，工程内时间轴）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeneratedDrumNote {
    /// 起始 tick（自片段/工程起点计，由调用方再加片段偏移）。
    pub tick: u64,
    /// GM 打击乐键号。
    pub gm_key: u8,
    /// 力度 1..=127。
    pub velocity: u8,
    /// 时值（tick）。
    pub duration_ticks: u64,
}

/// 生成失败。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DrumGenError {
    /// 旋钮越界（响亮报错，不静默钳制，与 `swing` 模块同一口径）。
    #[error("drum gen knob `{field}` value {value} out of range")]
    KnobOutOfRange {
        /// 出错字段名。
        field: &'static str,
        /// 实际收到的值。
        value: u32,
    },
    /// 摇摆越界（来自 [`crate::swing`] 的校验）。
    #[error(transparent)]
    Swing(#[from] TheoryError),
}

/// 音符时值口径：短促件 120 tick；开镲 480；crash 960。
fn duration_for(gm_key: u8) -> u64 {
    match gm_key {
        46 => 480,
        49 => 960,
        _ => 120,
    }
}

/// 复杂度允许的最高 tier。
fn max_tier(complexity: u8) -> u8 {
    match complexity {
        0..=33 => 0,
        34..=66 => 1,
        _ => 2,
    }
}

/// 鬼音判定哈希：SplitMix64 终混（与 `lib.rs` 的种子口径同族）。
fn ghost_hash(seed: u64, gm_key: u8, global_step: u64) -> u64 {
    let mut z = seed ^ (u64::from(gm_key) << 32) ^ global_step.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// 生成音符列。输出按 `(tick, gm_key)` 升序、同 tick 同键至多一项
/// （ratchet 子音 tick 互不相同；跨道同键在预置层已合并，叠层键与
/// 全表发声键的唯一性在预置解析期钉死）。
///
/// # Errors
///
/// 旋钮越界返回 [`DrumGenError::KnobOutOfRange`]；摇摆越界返回
/// [`DrumGenError::Swing`]。
pub fn generate_drum_pattern(
    preset: &DrumStylePreset,
    params: &DrumGenParams,
) -> Result<Vec<GeneratedDrumNote>, DrumGenError> {
    if params.complexity > KNOB_MAX {
        return Err(DrumGenError::KnobOutOfRange {
            field: "complexity",
            value: u32::from(params.complexity),
        });
    }
    if params.intensity > KNOB_MAX {
        return Err(DrumGenError::KnobOutOfRange {
            field: "intensity",
            value: u32::from(params.intensity),
        });
    }
    if params.bars == 0 || params.bars > 16 {
        return Err(DrumGenError::KnobOutOfRange {
            field: "bars",
            value: params.bars,
        });
    }
    let swing = params.swing_permille.unwrap_or(preset.swing_permille);
    crate::swing::validate_swing_permille(swing)?;
    // 一对 16 分（480 tick）内后半的推迟量；平直时为 0。
    let odd_delay = swung_onset_offset(STEP_TICKS * 2, swing)?.max(0) as u64;

    let tier_cap = max_tier(params.complexity);
    let velocity_factor = 0.55 + 0.45 * (f64::from(params.intensity) / 100.0);
    let total_steps = params.bars as usize * STEPS_PER_BAR;
    let source_steps = preset.bars as usize * STEPS_PER_BAR;
    let use_fill = params.fill && preset.fill_lanes.is_some() && params.bars >= 2;

    let mut notes: Vec<GeneratedDrumNote> = Vec::new();
    for lane in &preset.lanes {
        // fill 小节的取源：fill 道表里同键的道；无同键道 ⇒ 该小节休止。
        let fill_lane: Option<&DrumLanePreset> = if use_fill {
            preset
                .fill_lanes
                .as_ref()
                .and_then(|lanes| lanes.iter().find(|l| l.gm_key == lane.gm_key))
        } else {
            None
        };
        let in_fill_bar =
            |global_step: usize| -> bool { use_fill && global_step >= total_steps - STEPS_PER_BAR };
        for global_step in 0..total_steps {
            let step_in_bar = global_step % STEPS_PER_BAR;
            // 取源道：fill 小节用 fill 道（步进与叠层都跟源道走，
            // fill 道按主键与主道同键，其叠层是 fill 自己的定值）。
            let (src_lane, step, from_fill) = if in_fill_bar(global_step) {
                match fill_lane {
                    Some(fl) => (fl, fl.steps[step_in_bar], true),
                    None => continue,
                }
            } else {
                (lane, lane.steps[global_step % source_steps], false)
            };
            let base_tick = global_step as u64 * STEP_TICKS;
            let swing_delay = if step_in_bar % 2 == 1 { odd_delay } else { 0 };
            if step.is_hit() {
                if step.tier > tier_cap {
                    continue;
                }
                emit_step(
                    &mut notes,
                    base_tick + swing_delay,
                    src_lane.gm_key,
                    f64::from(step.velocity),
                    step.ratchet,
                    velocity_factor,
                );
                // 叠层（P6）：同格同刻触发；力度窗对写力度判定，
                // 电平按 dB 缩放写力度后走同一条 emit 变换（含
                // ratchet 展开与时值口径，时值按叠层键取）。
                for layer in &src_lane.layers {
                    if layer.fires_on(step.velocity) {
                        let gain = 10f64.powf(f64::from(layer.level_db) / 20.0);
                        emit_step(
                            &mut notes,
                            base_tick + swing_delay,
                            layer.gm_key,
                            f64::from(step.velocity) * gain,
                            step.ratchet,
                            velocity_factor,
                        );
                    }
                }
            } else if !from_fill
                && params.complexity >= 67
                && lane.gm_key == 38
                && has_hit_neighbor(lane, global_step, source_steps)
            {
                // 鬼音：军鼓空格、邻格有主击时按哈希补一击。
                let h = ghost_hash(params.seed, lane.gm_key, global_step as u64);
                if h % 100 < u64::from(params.complexity - 66) {
                    let velocity = 24 + (h / 100 % 16) as u8;
                    emit_step(
                        &mut notes,
                        base_tick + swing_delay,
                        lane.gm_key,
                        f64::from(velocity),
                        1,
                        velocity_factor,
                    );
                }
            }
        }
    }
    notes.sort_by_key(|n| (n.tick, n.gm_key));
    Ok(notes)
}

/// 邻格（同道前后一格，跨 pattern 循环边界）是否有主击。
fn has_hit_neighbor(lane: &DrumLanePreset, global_step: usize, source_steps: usize) -> bool {
    let src = global_step % source_steps;
    let prev = lane.steps[(src + source_steps - 1) % source_steps];
    let next = lane.steps[(src + 1) % source_steps];
    prev.is_hit() || next.is_hit()
}

/// 把一格展开为 1..=ratchet 个子音并套力度变换。`velocity` 是变换
/// 前的力度（主道 = 写力度；叠层 = 写力度 × 电平增益，仍是浮点，
/// 只在最终落点舍入一次——主键路径与整数力度时代逐位一致）。
fn emit_step(
    notes: &mut Vec<GeneratedDrumNote>,
    tick: u64,
    gm_key: u8,
    velocity: f64,
    ratchet: u8,
    velocity_factor: f64,
) {
    let r = u64::from(ratchet.max(1));
    let sub_ticks = STEP_TICKS / r;
    for k in 0..r {
        let ratchet_factor = 0.85 + 0.15 * (k as f64 / r as f64);
        let scaled = (velocity * velocity_factor * ratchet_factor).round();
        let v = scaled.clamp(1.0, 127.0) as u8;
        notes.push(GeneratedDrumNote {
            tick: tick + k * sub_ticks,
            gm_key,
            velocity: v,
            duration_ticks: duration_for(gm_key),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drum_preset::factory_drum_style;

    fn trap() -> DrumStylePreset {
        factory_drum_style("trap")
            .expect("factory presets parse")
            .expect("trap exists")
    }

    #[test]
    fn same_seed_same_output() {
        let preset = trap();
        let params = DrumGenParams {
            complexity: 90,
            seed: 42,
            ..DrumGenParams::default()
        };
        let a = generate_drum_pattern(&preset, &params).expect("gen a");
        let b = generate_drum_pattern(&preset, &params).expect("gen b");
        assert_eq!(a, b);
        assert!(!a.is_empty());
    }

    #[test]
    fn different_seed_changes_ghost_notes() {
        let preset = trap();
        let mk = |seed| {
            generate_drum_pattern(
                &preset,
                &DrumGenParams {
                    complexity: 100,
                    seed,
                    ..DrumGenParams::default()
                },
            )
            .expect("gen")
        };
        // 鬼音层由种子驱动：至少两个种子产出不同列（概率上恒真，
        // 哈希无碰撞退化时才会假——取 4 个种子两两比）。
        let outs: Vec<_> = [1_u64, 2, 3, 4].iter().map(|&s| mk(s)).collect();
        assert!(outs.windows(2).any(|w| w[0] != w[1]));
    }

    #[test]
    fn complexity_gates_density() {
        let preset = trap();
        let count = |complexity| {
            generate_drum_pattern(
                &preset,
                &DrumGenParams {
                    complexity,
                    fill: false,
                    ..DrumGenParams::default()
                },
            )
            .expect("gen")
            .len()
        };
        assert!(count(10) < count(50));
        assert!(count(50) <= count(100));
    }

    #[test]
    fn straight_swing_keeps_grid() {
        let preset = trap();
        let notes = generate_drum_pattern(
            &preset,
            &DrumGenParams {
                swing_permille: Some(500),
                complexity: 0,
                fill: false,
                bars: 1,
                ..DrumGenParams::default()
            },
        )
        .expect("gen");
        // R115：`.all(` 在空集上恒真 ⇒ 先钉住 `notes` 非空，否则该性质断言什么都没测。
        assert!(!notes.is_empty(), "trap preset must emit at least one note");
        assert!(notes.iter().all(|n| n.tick % STEP_TICKS == 0));
    }

    #[test]
    fn fill_replaces_last_bar() {
        let preset = trap();
        assert!(preset.fill_lanes.is_some());
        let with = generate_drum_pattern(
            &preset,
            &DrumGenParams {
                bars: 2,
                fill: true,
                complexity: 100,
                ..DrumGenParams::default()
            },
        )
        .expect("gen with fill");
        let without = generate_drum_pattern(
            &preset,
            &DrumGenParams {
                bars: 2,
                fill: false,
                complexity: 100,
                ..DrumGenParams::default()
            },
        )
        .expect("gen without fill");
        let last_bar = |ns: &[GeneratedDrumNote]| {
            ns.iter()
                .filter(|n| n.tick >= TICKS_PER_BAR)
                .copied()
                .collect::<Vec<_>>()
        };
        assert_ne!(last_bar(&with), last_bar(&without));
    }

    #[test]
    fn out_of_range_knobs_are_loud() {
        let preset = trap();
        assert!(
            generate_drum_pattern(
                &preset,
                &DrumGenParams {
                    complexity: 101,
                    ..DrumGenParams::default()
                }
            )
            .is_err()
        );
        assert!(
            generate_drum_pattern(
                &preset,
                &DrumGenParams {
                    swing_permille: Some(499),
                    ..DrumGenParams::default()
                }
            )
            .is_err()
        );
    }

    /// 一条带叠层的军鼓道：step 0 写力度 120（窗内）、step 4 写
    /// 力度 60（窗外）；叠层 = clap 39、−20 dB（×0.1）、窗 100..=127。
    fn layered_snare_preset() -> DrumStylePreset {
        let text = r#"{
            "format": "yeban-drum-style", "version": 1,
            "id": "layered", "name": "Layered", "name_zh": "分层",
            "source": {"kind": "test", "groove_genre_id": "x",
                       "repo": "t", "license": "MIT",
                       "converted": "2026-11-19", "tuning": []},
            "kit_family": "electronic",
            "bpm": {"default": 90, "min": 80, "max": 100},
            "swing_permille": 500, "bars": 1,
            "lanes": [{"role": "snare", "gm_key": 38,
                       "layers": [{"gm_key": 39, "level_db": -20.0,
                                   "vel_min": 100, "vel_max": 127}],
                       "steps": [
                [120,1,0],[0,1,0],[0,1,0],[0,1,0],[60,1,0],[0,1,0],[0,1,0],[0,1,0],
                [0,1,0],[0,1,0],[0,1,0],[0,1,0],[0,1,0],[0,1,0],[0,1,0],[0,1,0]
            ]}]
        }"#;
        DrumStylePreset::from_json(text).expect("valid layered preset")
    }

    #[test]
    fn layered_lane_expands_to_multiple_notes_with_level_and_window() {
        let preset = layered_snare_preset();
        let notes = generate_drum_pattern(
            &preset,
            &DrumGenParams {
                complexity: 0,
                intensity: 100,
                swing_permille: Some(500),
                bars: 1,
                fill: false,
                ..DrumGenParams::default()
            },
        )
        .expect("gen");
        // 强度 100 ⇒ 力度系数 1.0；ratchet 1 ⇒ 子音系数 0.85。
        // step 0：主道 round(120×0.85)=102；叠层窗内，
        // round(120×0.1×0.85)=10。step 4（tick 960）：主道
        // round(60×0.85)=51；叠层窗外（60 < 100），无叠层音。
        assert_eq!(
            notes,
            vec![
                GeneratedDrumNote {
                    tick: 0,
                    gm_key: 38,
                    velocity: 102,
                    duration_ticks: 120,
                },
                GeneratedDrumNote {
                    tick: 0,
                    gm_key: 39,
                    velocity: 10,
                    duration_ticks: 120,
                },
                GeneratedDrumNote {
                    tick: 960,
                    gm_key: 38,
                    velocity: 51,
                    duration_ticks: 120,
                },
            ]
        );
    }

    #[test]
    fn hiphop_clap_layer_tracks_the_snare_backbeat() {
        let preset = factory_drum_style("hiphop")
            .expect("factory presets parse")
            .expect("hiphop exists");
        let notes = generate_drum_pattern(
            &preset,
            &DrumGenParams {
                complexity: 0,
                intensity: 100,
                bars: 2,
                fill: false,
                ..DrumGenParams::default()
            },
        )
        .expect("gen");
        let ticks_of = |key: u8| {
            notes
                .iter()
                .filter(|n| n.gm_key == key)
                .map(|n| n.tick)
                .collect::<Vec<_>>()
        };
        // clap 只经军鼓叠层发声：与军鼓击点同 tick（反拍 4/12/20/28
        // 格），且每击都比军鼓轻（−3.2 dB）。
        assert_eq!(ticks_of(39), ticks_of(38));
        assert_eq!(ticks_of(38).len(), 4);
        for note in notes.iter().filter(|n| n.gm_key == 39) {
            let snare = notes
                .iter()
                .find(|n| n.gm_key == 38 && n.tick == note.tick)
                .expect("layer note has a primary at the same tick");
            assert!(note.velocity < snare.velocity);
        }
        // 全输出无同 tick 同键重复（生成不变量）。
        let mut seen = std::collections::BTreeSet::new();
        for note in &notes {
            assert!(seen.insert((note.tick, note.gm_key)));
        }
    }

    #[test]
    fn hiphop_fill_clap_layer_fires_only_on_the_loudest_fill_hit() {
        let preset = factory_drum_style("hiphop")
            .expect("factory presets parse")
            .expect("hiphop exists");
        let notes = generate_drum_pattern(
            &preset,
            &DrumGenParams {
                complexity: 0,
                intensity: 100,
                bars: 2,
                fill: true,
                ..DrumGenParams::default()
            },
        )
        .expect("gen");
        // fill 小节（tick ≥ 3840）：fill 军鼓五击里只有写力度 112 的
        // 最后一击（fill 第 12 格 ⇒ 全局 tick 28×240=6720）带 clap
        // 叠层；力度 = round(112 × 10^(−2.5/20) × 0.85) = 71。
        let fill_claps: Vec<&GeneratedDrumNote> = notes
            .iter()
            .filter(|n| n.gm_key == 39 && n.tick >= TICKS_PER_BAR)
            .collect();
        assert_eq!(fill_claps.len(), 1);
        assert_eq!(fill_claps[0].tick, 28 * STEP_TICKS);
        assert_eq!(fill_claps[0].velocity, 71);
    }

    #[test]
    fn factory_outputs_never_repeat_a_tick_key_pair() {
        // 生成不变量全风格钉：同 tick 同键至多一项（含鬼音、fill、
        // ratchet 与叠层全开的组合）。
        for style in crate::drum_preset::factory_drum_styles().expect("factory presets parse") {
            let notes = generate_drum_pattern(
                &style,
                &DrumGenParams {
                    complexity: 100,
                    seed: 7,
                    bars: 4,
                    fill: true,
                    ..DrumGenParams::default()
                },
            )
            .expect("gen");
            let mut seen = std::collections::BTreeSet::new();
            for note in &notes {
                assert!(
                    seen.insert((note.tick, note.gm_key)),
                    "{} repeats tick {} key {}",
                    style.id,
                    note.tick,
                    note.gm_key
                );
            }
        }
    }
}
