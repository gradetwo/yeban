//! `compare_song_snapshots` —— Songmap A/B 快照 Diff（设计稿 §8.3，Phase 2）。
//!
//! ## 快照是什么
//!
//! 快照 = `get_song_overview` 的 `data`（[`crate::output_schema::SongOverview`]，
//! schema 1.1）原样。调用方先取修改前的快照 A，改谱后再取当前版快照 B，
//! 把两份 JSON 原样传进本工具，拿回逐小节 Diff。Diff 是逐点相减
//! （Δ = B − A）：曲线列逐小节减、异常与重复按条目集合比对、结构摘要比
//! 高潮小节与动态范围（设计稿 §8.3）。
//!
//! ## 为什么 Diff 住在 `yeban-mcp`（而不是 `yeban-dsp`）
//!
//! `scripts/gates/check_mcp_dependency_direction.py` 的 `FORBIDDEN` 逐字含
//! `"yeban-dsp"`，`yeban-mcp` **禁**直接依赖它（见 `domain::song_overview`
//! 模块头）。Diff 不是分析（STFT / LUFS / 情绪公式都不在这里），只是对两份
//! 已算好的总览做算术 —— 它只能住在 MCP 侧的形状
//! （[`crate::output_schema::SongOverview`]）上。本模块不复制任何检测算法，
//! 异常与重复条目只做集合比对（设计稿 §8.3 的"数值逐点相减"）。
//!
//! ## 本工具的取舍（设计稿没写死的部分，由本轮定）
//!
//! - **只比小节级**：设计稿 §11 定 Diff 返回"小节级 Diff JSON"。任一快照含
//!   拍级行（`beat` 键）⇒ `snapshotMismatch`（`field: "resolution"`），
//!   ⛔ 不静默按小节折叠（折叠会丢拍信息还假装没丢）。
//! - **不要求活跃工程**：Diff 是入参的纯函数，不读工程、不调 provider。
//!   （`get_song_overview` 需要活跃工程是因为它要现场分析；本工具不需要。）
//! - **两版 `schema_version` 必须相等**：不等 ⇒ `snapshotMismatch`
//!   （`field: "schema_version"`）。1.0 快照缺 `repetition` 字段，
//!   解析那一步就以 `invalidSnapshot` 拒绝，不会走到这里。
//! - **`band_definitions_hz` 必须相等**：不等 ⇒ `snapshotMismatch`
//!   （频段定义不同时 8 列的含义不同，差值无意义）。
//! - **行键重复 / 小节号为 0** ⇒ `invalidSnapshot`（`get_song_overview`
//!   永远不产出这种形状，收到就是调用方拼错了）。
//! - **空快照**（`bar_series` 为空）与任一分辨率兼容：它没有行，也就没有
//!   粒度；与非空快照比时，非空那边的行全进 `bars_added` / `bars_removed`。
//! - **`changed` 的定义**：任一差值取整后非 0，或段落改名。不设阈值 ——
//!   入参已按 §8.2 取整（归一化 2 位小数、dB 1 位小数），取整后非 0 的差值
//!   就是报告精度下的真实变化。
//! - **取整**：差值先减后取整，口径与 §8.2 相同（归一化 2 位小数、dB 1 位
//!   小数），复用 `domain::song_overview` 的 `round1` / `round2` ——
//!   取整只有一份事实源（⛔ 不在本模块另写一份看起来一样的取整）。
//! - **时间坐标列不参与差值**：`bar` 与 `time_sec` 是行的坐标，不是量测；
//!   速度变化由 `meta_changes.base_bpm` 覆盖（中途变速致 `time_sec` 平移而
//!   `base_bpm` 不变时，Diff 不显式报这一项 —— 小节级 Diff 的既定范围）。
//!
//! ## 参数与失败语义
//!
//! - `snapshot_a` / `snapshot_b`（必填，`object`）：两份快照。非对象 ⇒
//!   协议层 `-32602`（注册表类型校验先于领域层，与 `expected_levels` 同款）；
//!   缺失 ⇒ 同码（`MissingParam`）。对象但不是合法 `SongOverview` 形状 ⇒
//!   `INVALID_PARAMETER_RANGE`（`reason: "invalidSnapshot"`）。
//! - 上面"取舍"里的三类跨快照不一致 ⇒ `INVALID_PARAMETER_RANGE`
//!   （`reason: "snapshotMismatch"`，`field` 点名哪一项）。
//! - 只读：不改工程、不落盘、不进提交图谱（`Plan::CompareSongSnapshots` 的
//!   `commit_delta` 为 0，`commitModel` 为 `"none"`）。

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use crate::output_schema::{
    BarDiff, BarRow, BestMatch, Change, DetectedAnomaly, DiffSummary, MetaChanges, RepeatedPassage,
    RepetitionChanges, SongOverview, SongOverviewDiff, StructuralChanges,
};
use crate::tools::{ErrorCode, ToolCall};

use super::error::Fault;
use super::{Plan, song_overview};

/// 快照形状非法时的 `data.reason`（缺字段、类型错、行键重复、小节号为 0）。
pub const INVALID_SNAPSHOT_REASON: &str = "invalidSnapshot";

/// 两版快照不可比时的 `data.reason`（版本不等、频段定义不等、拍级快照）。
pub const SNAPSHOT_MISMATCH_REASON: &str = "snapshotMismatch";

/// 读一份快照实参：缺失或形状非法 ⇒ `INVALID_PARAMETER_RANGE`。
fn arg_snapshot(call: &ToolCall, name: &str) -> Result<SongOverview, Fault> {
    let Some(value) = call.arguments.get(name) else {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "`{name}` 是必填参数：须给一份 `get_song_overview` 的 `data`（SongOverview，schema 1.1）"
            ),
            serde_json::json!({
                "reason": INVALID_SNAPSHOT_REASON,
                "snapshot": name,
                "detail": "missing required argument",
            }),
        ));
    };
    serde_json::from_value::<SongOverview>(value.clone()).map_err(|error| {
        Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!("`{name}` 不是一份合法的 SongOverview：{error}"),
            serde_json::json!({
                "reason": INVALID_SNAPSHOT_REASON,
                "snapshot": name,
                "detail": error.to_string(),
            }),
        )
    })
}

/// 单份快照的行键校验：小节号 ≥ 1、行键唯一。
fn validate_snapshot(overview: &SongOverview, name: &str) -> Result<(), Fault> {
    let mut seen = BTreeSet::new();
    for row in &overview.bar_series {
        if row.bar == 0 {
            return Err(Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                "`bar_series` 里小节号必须 ≥ 1",
                serde_json::json!({
                    "reason": INVALID_SNAPSHOT_REASON,
                    "snapshot": name,
                    "detail": "bar must be >= 1, got 0",
                }),
            ));
        }
        if !seen.insert(row.bar) {
            return Err(Fault::domain_with_data(
                ErrorCode::InvalidParameterRange,
                format!("`bar_series` 里小节号 {} 出现了两次", row.bar),
                serde_json::json!({
                    "reason": INVALID_SNAPSHOT_REASON,
                    "snapshot": name,
                    "detail": format!("duplicate bar {}", row.bar),
                }),
            ));
        }
    }
    Ok(())
}

/// 两版快照的可比性检查：版本相等、频段定义相等、均为小节级。
fn check_compatible(a: &SongOverview, b: &SongOverview) -> Result<(), Fault> {
    if a.schema_version != b.schema_version {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            format!(
                "两版快照的 schema_version 必须相等：A 是 {:?}，B 是 {:?}",
                a.schema_version, b.schema_version
            ),
            serde_json::json!({
                "reason": SNAPSHOT_MISMATCH_REASON,
                "field": "schema_version",
                "detail": format!(
                    "snapshot_a schema_version is {:?} but snapshot_b is {:?}",
                    a.schema_version, b.schema_version
                ),
            }),
        ));
    }
    if a.band_definitions_hz != b.band_definitions_hz {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            "两版快照的 band_definitions_hz 必须相等（频段定义不同时 8 列的含义不同）",
            serde_json::json!({
                "reason": SNAPSHOT_MISMATCH_REASON,
                "field": "band_definitions_hz",
                "detail": "band definitions differ between snapshots",
            }),
        ));
    }
    let a_beat = a.bar_series.iter().any(|row| row.beat.is_some());
    let b_beat = b.bar_series.iter().any(|row| row.beat.is_some());
    if a_beat || b_beat {
        return Err(Fault::domain_with_data(
            ErrorCode::InvalidParameterRange,
            "Diff 只比小节级快照（设计稿 §11）：拍级快照请先取小节级总览再比",
            serde_json::json!({
                "reason": SNAPSHOT_MISMATCH_REASON,
                "field": "resolution",
                "detail": "beat-level snapshots are not diffed (design §11: bar-level Diff only)",
            }),
        ));
    }
    Ok(())
}

/// 两值不等时给出一对 `Change`，相等时给 `None`（`meta_changes` 只含变化项）。
fn change_if_diff<T: PartialEq>(from: T, to: T) -> Option<Change<T>> {
    if from == to {
        None
    } else {
        Some(Change { from, to })
    }
}

/// 集合三分：`(只在 B 里的， 只在 A 里的， 两版都有的)`。
///
/// 按 `key` 排序后双指针归并；`key` 须与整值一一对应（键相等 ⟺ 值相等），
/// 否则"改了描述的同一条异常"会被误判为未变。输出三组各自按键升序。
fn partition_by<T: Clone, K: Ord>(
    a: &[T],
    b: &[T],
    key: impl Fn(&T) -> K,
) -> (Vec<T>, Vec<T>, Vec<T>) {
    let mut a_sorted: Vec<&T> = a.iter().collect();
    let mut b_sorted: Vec<&T> = b.iter().collect();
    a_sorted.sort_by_key(|x| key(x));
    b_sorted.sort_by_key(|x| key(x));
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut unchanged = Vec::new();
    let mut i = 0;
    let mut j = 0;
    while i < a_sorted.len() && j < b_sorted.len() {
        match key(a_sorted[i]).cmp(&key(b_sorted[j])) {
            Ordering::Less => {
                removed.push(a_sorted[i].clone());
                i += 1;
            }
            Ordering::Greater => {
                added.push(b_sorted[j].clone());
                j += 1;
            }
            Ordering::Equal => {
                unchanged.push(a_sorted[i].clone());
                i += 1;
                j += 1;
            }
        }
    }
    while i < a_sorted.len() {
        removed.push(a_sorted[i].clone());
        i += 1;
    }
    while j < b_sorted.len() {
        added.push(b_sorted[j].clone());
        j += 1;
    }
    (added, removed, unchanged)
}

/// 两版都有的某一小节的差值（Δ = B − A；取整口径与 §8.2 相同）。
fn diff_row(a: &BarRow, b: &BarRow) -> BarDiff {
    let tension_delta = song_overview::round2(b.tension - a.tension);
    let valence_hue_delta = song_overview::round2(b.valence_hue - a.valence_hue);
    let lufs_short_delta = song_overview::round1(b.lufs_short - a.lufs_short);
    let lufs_momentary_max_delta =
        song_overview::round1(b.lufs_momentary_max - a.lufs_momentary_max);
    let score_dyn_delta = song_overview::round2(b.score_dyn - a.score_dyn);
    let envelope_lo_delta = match (a.envelope_lo, b.envelope_lo) {
        (Some(x), Some(y)) => Some(song_overview::round2(y - x)),
        _ => None,
    };
    let envelope_hi_delta = match (a.envelope_hi, b.envelope_hi) {
        (Some(x), Some(y)) => Some(song_overview::round2(y - x)),
        _ => None,
    };
    let target_deviation_delta = song_overview::round2(b.target_deviation - a.target_deviation);
    let density_pitched_onsets_per_beat_delta = song_overview::round2(
        b.density_pitched_onsets_per_beat - a.density_pitched_onsets_per_beat,
    );
    let density_pitched_duty_delta =
        song_overview::round2(b.density_pitched_duty - a.density_pitched_duty);
    let density_perc_onsets_per_beat_delta =
        song_overview::round2(b.density_perc_onsets_per_beat - a.density_perc_onsets_per_beat);
    let mut bands_tilted_db_delta = [0.0; 8];
    for (index, slot) in bands_tilted_db_delta.iter_mut().enumerate() {
        *slot = song_overview::round1(b.bands_tilted_db[index] - a.bands_tilted_db[index]);
    }
    let section_changed = a.section != b.section;
    let changed = section_changed
        || tension_delta != 0.0
        || valence_hue_delta != 0.0
        || lufs_short_delta != 0.0
        || lufs_momentary_max_delta != 0.0
        || score_dyn_delta != 0.0
        || envelope_lo_delta.is_some_and(|delta| delta != 0.0)
        || envelope_hi_delta.is_some_and(|delta| delta != 0.0)
        || target_deviation_delta != 0.0
        || density_pitched_onsets_per_beat_delta != 0.0
        || density_pitched_duty_delta != 0.0
        || density_perc_onsets_per_beat_delta != 0.0
        || bands_tilted_db_delta.iter().any(|delta| *delta != 0.0);
    BarDiff {
        bar: b.bar,
        beat: b.beat,
        section: b.section.clone(),
        section_from: if section_changed {
            Some(a.section.clone())
        } else {
            None
        },
        tension_delta,
        valence_hue_delta,
        lufs_short_delta,
        lufs_momentary_max_delta,
        score_dyn_delta,
        envelope_lo_delta,
        envelope_hi_delta,
        target_deviation_delta,
        density_pitched_onsets_per_beat_delta,
        density_pitched_duty_delta,
        density_perc_onsets_per_beat_delta,
        bands_tilted_db_delta,
        changed,
    }
}

/// 两版快照的 Diff（纯函数：调用前须先过 `validate_snapshot` +
/// `check_compatible`；直接调本函数时假定入参已合法。行键重复时
/// `BTreeMap` 的收集会静默丢行 —— 调用方必须先校验唯一性，本函数不补查；
/// 分工写在模块头）。
#[must_use]
pub fn diff_overviews(a: &SongOverview, b: &SongOverview) -> SongOverviewDiff {
    let meta_changes = MetaChanges {
        total_bars: change_if_diff(a.song_meta.total_bars, b.song_meta.total_bars),
        duration_sec: change_if_diff(a.song_meta.duration_sec, b.song_meta.duration_sec),
        base_bpm: change_if_diff(a.song_meta.base_bpm, b.song_meta.base_bpm),
        key_signature: change_if_diff(
            a.song_meta.key_signature.clone(),
            b.song_meta.key_signature.clone(),
        ),
        profile: change_if_diff(a.song_meta.profile.clone(), b.song_meta.profile.clone()),
    };
    let map_a: BTreeMap<u64, &BarRow> = a.bar_series.iter().map(|row| (row.bar, row)).collect();
    let map_b: BTreeMap<u64, &BarRow> = b.bar_series.iter().map(|row| (row.bar, row)).collect();
    let mut bar_diffs = Vec::new();
    let mut bars_added = Vec::new();
    let mut bars_removed = Vec::new();
    let mut bars_changed = 0u64;
    let mut max_abs_tension_delta = 0.0f64;
    let mut max_abs_lufs_delta = 0.0f64;
    let mut max_abs_band_delta_db = 0.0f64;
    for (&bar, row_b) in &map_b {
        match map_a.get(&bar) {
            Some(row_a) => {
                let diff = diff_row(row_a, row_b);
                if diff.changed {
                    bars_changed += 1;
                }
                max_abs_tension_delta = max_abs_tension_delta.max(diff.tension_delta.abs());
                max_abs_lufs_delta = max_abs_lufs_delta.max(diff.lufs_short_delta.abs());
                max_abs_band_delta_db = max_abs_band_delta_db.max(
                    diff.bands_tilted_db_delta
                        .iter()
                        .fold(0.0f64, |max, delta| max.max(delta.abs())),
                );
                bar_diffs.push(diff);
            }
            None => bars_added.push(bar),
        }
    }
    for &bar in map_a.keys() {
        if !map_b.contains_key(&bar) {
            bars_removed.push(bar);
        }
    }
    let (anomalies_added, anomalies_removed, anomalies_unchanged) = partition_by(
        &a.structural_summary.detected_anomalies,
        &b.structural_summary.detected_anomalies,
        |anomaly: &DetectedAnomaly| {
            (
                anomaly.bars,
                anomaly.anomaly_type.clone(),
                anomaly.description.clone(),
            )
        },
    );
    let (best_matches_added, best_matches_removed, best_matches_unchanged) = partition_by(
        &a.repetition.best_matches,
        &b.repetition.best_matches,
        |best: &BestMatch| (best.bar_a, best.bar_b, best.similarity.to_bits()),
    );
    let (repeated_passages_added, repeated_passages_removed, repeated_passages_unchanged) =
        partition_by(
            &a.repetition.repeated_passages,
            &b.repetition.repeated_passages,
            |passage: &RepeatedPassage| {
                (
                    passage.bars,
                    passage.source_bars,
                    passage.mean_similarity.to_bits(),
                )
            },
        );
    SongOverviewDiff {
        schema_version: b.schema_version.clone(),
        snapshot_a: a.song_meta.clone(),
        snapshot_b: b.song_meta.clone(),
        meta_changes,
        summary: DiffSummary {
            bars_compared: bar_diffs.len() as u64,
            bars_added: bars_added.len() as u64,
            bars_removed: bars_removed.len() as u64,
            bars_changed,
            max_abs_tension_delta,
            max_abs_lufs_delta,
            max_abs_band_delta_db,
        },
        bar_diffs,
        bars_added,
        bars_removed,
        anomalies_added,
        anomalies_removed,
        anomalies_unchanged,
        repetition_changes: RepetitionChanges {
            mechanical_index_delta: song_overview::round2(
                b.repetition.mechanical_index - a.repetition.mechanical_index,
            ),
            best_matches_added,
            best_matches_removed,
            best_matches_unchanged,
            repeated_passages_added,
            repeated_passages_removed,
            repeated_passages_unchanged,
        },
        structural_changes: StructuralChanges {
            climax_bar: change_if_diff(
                a.structural_summary.climax_bar,
                b.structural_summary.climax_bar,
            ),
            climax_tension_delta: song_overview::round2(
                b.structural_summary.climax_tension - a.structural_summary.climax_tension,
            ),
            dynamic_range_lu_delta: song_overview::round1(
                b.structural_summary.dynamic_range_lu - a.structural_summary.dynamic_range_lu,
            ),
        },
    }
}

/// `compare_song_snapshots` 的只读规划（不要求活跃工程、不调 provider）。
///
/// # Errors
///
/// 快照缺失 / 形状非法（`INVALID_PARAMETER_RANGE` + `invalidSnapshot`）、
/// 两版不可比（同码 + `snapshotMismatch`），或 Diff 序列化失败（`IO_ERROR`，
/// 实现级状况，不应发生）。
pub fn plan_compare_song_snapshots(call: &ToolCall) -> Result<Plan, Fault> {
    let a = arg_snapshot(call, "snapshot_a")?;
    let b = arg_snapshot(call, "snapshot_b")?;
    validate_snapshot(&a, "snapshot_a")?;
    validate_snapshot(&b, "snapshot_b")?;
    check_compatible(&a, &b)?;
    let diff = diff_overviews(&a, &b);
    let data = serde_json::to_value(diff).map_err(|error| {
        Fault::domain(
            ErrorCode::IoError,
            format!("Diff 序列化失败（实现级状况，不应发生）: {error}"),
        )
    })?;
    Ok(Plan::CompareSongSnapshots { data })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use crate::output_schema::{
        BarRow, BestMatch, DetectedAnomaly, FrequencyClashSummary, RepeatedPassage,
        RepetitionSummary, SongMeta, SongOverview, StructuralSummary,
    };
    use crate::tools::ToolCall;

    use super::*;

    fn row(bar: u64, tension: f64, lufs_short: f64, band0: f64) -> BarRow {
        BarRow {
            bar,
            beat: None,
            // 120 BPM、4/4 下每小节 2 秒（只为夹具自洽；Diff 不比时间坐标）。
            time_sec: bar.saturating_sub(1) as f64 * 2.0,
            section: "Verse".to_string(),
            tension,
            valence_hue: 0.5,
            lufs_short,
            lufs_momentary_max: -10.0,
            score_dyn: 0.5,
            envelope_lo: Some(0.2),
            envelope_hi: Some(0.8),
            target_deviation: 0.0,
            density_pitched_onsets_per_beat: 1.0,
            density_pitched_duty: 0.5,
            density_perc_onsets_per_beat: 1.0,
            bands_tilted_db: [band0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        }
    }

    fn overview(bars: Vec<BarRow>) -> SongOverview {
        SongOverview {
            schema_version: "1.1".to_string(),
            song_meta: SongMeta {
                total_bars: bars.len() as u64,
                duration_sec: 100.0,
                base_bpm: 120.0,
                key_signature: "C major".to_string(),
                profile: "pop".to_string(),
            },
            band_definitions_hz: [
                [20, 60],
                [60, 250],
                [250, 500],
                [500, 2000],
                [2000, 4000],
                [4000, 6000],
                [6000, 10000],
                [10000, 20000],
            ],
            structural_summary: StructuralSummary {
                climax_bar: 1,
                climax_tension: 0.9,
                dynamic_range_lu: 8.0,
                detected_anomalies: Vec::new(),
            },
            repetition: RepetitionSummary {
                mechanical_index: 0.3,
                best_matches: Vec::new(),
                repeated_passages: Vec::new(),
            },
            // 夹具只喂小节序列供 Diff 算增量，不算打架检测（检测在 dsp）：
            // 字段只保形状完整，值为空摘要。
            frequency_clash: FrequencyClashSummary {
                total_groups: 0,
                overall_overlap: 0.0,
                bar_overlaps: Vec::new(),
                entries: Vec::new(),
            },
            bar_series: bars,
        }
    }

    fn call_with(a: Value, b: Value) -> ToolCall {
        ToolCall::from_params(Some(&serde_json::json!({
            "name": "compare_song_snapshots",
            "arguments": {
                "snapshot_a": a,
                "snapshot_b": b,
            },
        })))
        .expect("合法的工具调用")
    }

    #[test]
    fn diff_of_two_overviews_matches_hand_computed_deltas() {
        let mut a = overview(vec![row(1, 0.3, -14.0, -2.0), row(2, 0.5, -12.0, 1.0)]);
        a.structural_summary.detected_anomalies = vec![DetectedAnomaly {
            bars: [1, 2],
            anomaly_type: "flat_energy".to_string(),
            description: "flat".to_string(),
        }];
        a.repetition.best_matches = vec![BestMatch {
            bar_a: 1,
            bar_b: 2,
            similarity: 0.97,
        }];
        let mut b = overview(vec![row(1, 0.5, -13.0, -1.0), row(2, 0.5, -12.0, 1.0)]);
        b.structural_summary.climax_bar = 2;
        b.structural_summary.climax_tension = 0.8;
        b.structural_summary.dynamic_range_lu = 9.5;
        b.structural_summary.detected_anomalies = vec![DetectedAnomaly {
            bars: [1, 2],
            anomaly_type: "flat_energy".to_string(),
            description: "flat".to_string(),
        }];
        b.repetition.mechanical_index = 0.45;
        b.repetition.best_matches = vec![BestMatch {
            bar_a: 1,
            bar_b: 2,
            similarity: 0.97,
        }];
        b.repetition.repeated_passages = vec![RepeatedPassage {
            bars: [1, 2],
            source_bars: [3, 4],
            mean_similarity: 0.96,
        }];

        let diff = diff_overviews(&a, &b);
        assert_eq!(diff.schema_version, "1.1");
        assert_eq!(diff.summary.bars_compared, 2);
        assert_eq!(diff.summary.bars_added, 0);
        assert_eq!(diff.summary.bars_removed, 0);
        assert_eq!(diff.summary.bars_changed, 1);
        assert_eq!(diff.summary.max_abs_tension_delta, 0.2);
        assert_eq!(diff.summary.max_abs_lufs_delta, 1.0);
        assert_eq!(diff.summary.max_abs_band_delta_db, 1.0);
        assert!(diff.bars_added.is_empty());
        assert!(diff.bars_removed.is_empty());
        let first = &diff.bar_diffs[0];
        assert_eq!(first.bar, 1);
        assert_eq!(first.tension_delta, 0.2);
        assert_eq!(first.lufs_short_delta, 1.0);
        assert_eq!(first.bands_tilted_db_delta[0], 1.0);
        assert!(first.changed);
        assert!(!diff.bar_diffs[1].changed);
        assert_eq!(diff.anomalies_unchanged.len(), 1);
        assert!(diff.anomalies_added.is_empty());
        assert!(diff.anomalies_removed.is_empty());
        assert_eq!(diff.repetition_changes.mechanical_index_delta, 0.15);
        assert_eq!(diff.repetition_changes.best_matches_unchanged.len(), 1);
        assert_eq!(diff.repetition_changes.repeated_passages_added.len(), 1);
        assert_eq!(
            diff.structural_changes.climax_bar,
            Some(Change { from: 1, to: 2 })
        );
        assert_eq!(diff.structural_changes.climax_tension_delta, -0.1);
        assert_eq!(diff.structural_changes.dynamic_range_lu_delta, 1.5);
        assert!(diff.meta_changes.total_bars.is_none());
    }

    #[test]
    fn added_and_removed_bars_are_listed_not_diffed() {
        let a = overview(vec![
            row(1, 0.3, -14.0, 0.0),
            row(2, 0.5, -12.0, 0.0),
            row(3, 0.4, -13.0, 0.0),
        ]);
        let b = overview(vec![
            row(2, 0.5, -12.0, 0.0),
            row(3, 0.4, -13.0, 0.0),
            row(4, 0.6, -11.0, 0.0),
        ]);
        let diff = diff_overviews(&a, &b);
        assert_eq!(diff.summary.bars_compared, 2);
        assert_eq!(diff.summary.bars_added, 1);
        assert_eq!(diff.summary.bars_removed, 1);
        assert_eq!(diff.summary.bars_changed, 0);
        assert_eq!(diff.bars_added, vec![4]);
        assert_eq!(diff.bars_removed, vec![1]);
        assert_eq!(diff.meta_changes.total_bars, None);
    }

    #[test]
    fn empty_snapshots_diff_to_empty() {
        let a = overview(vec![]);
        let b = overview(vec![]);
        let diff = diff_overviews(&a, &b);
        assert_eq!(diff.summary.bars_compared, 0);
        assert_eq!(diff.summary.bars_added, 0);
        assert_eq!(diff.summary.bars_removed, 0);
        assert_eq!(diff.summary.bars_changed, 0);
        assert_eq!(diff.summary.max_abs_tension_delta, 0.0);
        assert!(diff.bar_diffs.is_empty());
        assert!(diff.bars_added.is_empty());
        assert!(diff.bars_removed.is_empty());
    }

    #[test]
    fn rounding_cleans_float_noise() {
        // 0.3 − 0.1 在二进制里是 0.19999999999999998；取整后必须是 0.2。
        let a = overview(vec![row(1, 0.1, -14.0, 0.0)]);
        let b = overview(vec![row(1, 0.3, -14.0, 0.0)]);
        let diff = diff_overviews(&a, &b);
        assert_eq!(diff.bar_diffs[0].tension_delta, 0.2);
        assert!(diff.bar_diffs[0].changed);
    }

    #[test]
    fn envelope_deltas_present_only_when_both_sides_have_bands() {
        let mut a = overview(vec![row(1, 0.5, -14.0, 0.0)]);
        a.bar_series[0].envelope_lo = None;
        let b = overview(vec![row(1, 0.5, -14.0, 0.0)]);
        let diff = diff_overviews(&a, &b);
        assert_eq!(diff.bar_diffs[0].envelope_lo_delta, None);
        assert_eq!(diff.bar_diffs[0].envelope_hi_delta, Some(0.0));
        assert!(!diff.bar_diffs[0].changed);
    }

    #[test]
    fn section_rename_marks_row_changed() {
        let a = overview(vec![row(1, 0.5, -14.0, 0.0)]);
        let mut b = overview(vec![row(1, 0.5, -14.0, 0.0)]);
        b.bar_series[0].section = "Chorus".to_string();
        let diff = diff_overviews(&a, &b);
        assert!(diff.bar_diffs[0].changed);
        assert_eq!(diff.bar_diffs[0].section, "Chorus");
        assert_eq!(diff.bar_diffs[0].section_from.as_deref(), Some("Verse"));
    }

    #[test]
    fn duplicate_bar_is_invalid_snapshot() {
        let a = overview(vec![row(1, 0.5, -14.0, 0.0), row(1, 0.6, -13.0, 0.0)]);
        let error = validate_snapshot(&a, "snapshot_a").expect_err("重复行键必须拒绝");
        let Fault::Domain { data, .. } = &error else {
            panic!("期望领域失败，得到 {error:?}");
        };
        let data = data.as_ref().expect("fault 必须带 data");
        assert_eq!(data["reason"], "invalidSnapshot");
        assert_eq!(data["snapshot"], "snapshot_a");
        assert_eq!(data["detail"], "duplicate bar 1");
    }

    #[test]
    fn bar_zero_is_invalid_snapshot() {
        let a = overview(vec![row(0, 0.5, -14.0, 0.0)]);
        let error = validate_snapshot(&a, "snapshot_a").expect_err("小节号 0 必须拒绝");
        let Fault::Domain { data, .. } = &error else {
            panic!("期望领域失败，得到 {error:?}");
        };
        let data = data.as_ref().expect("fault 必须带 data");
        assert_eq!(data["reason"], "invalidSnapshot");
        assert_eq!(data["detail"], "bar must be >= 1, got 0");
    }

    #[test]
    fn version_mismatch_is_snapshot_mismatch() {
        let a = overview(vec![row(1, 0.5, -14.0, 0.0)]);
        let mut b = overview(vec![row(1, 0.5, -14.0, 0.0)]);
        b.schema_version = "1.0".to_string();
        let error = check_compatible(&a, &b).expect_err("版本不等必须拒绝");
        let Fault::Domain { data, .. } = &error else {
            panic!("期望领域失败，得到 {error:?}");
        };
        let data = data.as_ref().expect("fault 必须带 data");
        assert_eq!(data["reason"], "snapshotMismatch");
        assert_eq!(data["field"], "schema_version");
    }

    #[test]
    fn beat_rows_are_snapshot_mismatch() {
        let a = overview(vec![row(1, 0.5, -14.0, 0.0)]);
        let mut b = overview(vec![row(1, 0.5, -14.0, 0.0)]);
        b.bar_series[0].beat = Some(1);
        let error = check_compatible(&a, &b).expect_err("拍级快照必须拒绝");
        let Fault::Domain { data, .. } = &error else {
            panic!("期望领域失败，得到 {error:?}");
        };
        let data = data.as_ref().expect("fault 必须带 data");
        assert_eq!(data["reason"], "snapshotMismatch");
        assert_eq!(data["field"], "resolution");
    }

    #[test]
    fn band_definitions_mismatch_is_snapshot_mismatch() {
        let a = overview(vec![row(1, 0.5, -14.0, 0.0)]);
        let mut b = overview(vec![row(1, 0.5, -14.0, 0.0)]);
        b.band_definitions_hz[0] = [20, 80];
        let error = check_compatible(&a, &b).expect_err("频段定义不等必须拒绝");
        let Fault::Domain { data, .. } = &error else {
            panic!("期望领域失败，得到 {error:?}");
        };
        let data = data.as_ref().expect("fault 必须带 data");
        assert_eq!(data["reason"], "snapshotMismatch");
        assert_eq!(data["field"], "band_definitions_hz");
    }

    #[test]
    fn plan_returns_diff_data_for_valid_pair() {
        let a = overview(vec![row(1, 0.3, -14.0, -2.0)]);
        let b = overview(vec![row(1, 0.5, -13.0, -1.0)]);
        let call = call_with(
            serde_json::to_value(a).expect("fixture 序列化"),
            serde_json::to_value(b).expect("fixture 序列化"),
        );
        let plan = plan_compare_song_snapshots(&call).expect("合法快照对必须规划成功");
        match plan {
            Plan::CompareSongSnapshots { data } => {
                assert_eq!(data["summary"]["bars_compared"], 1);
                assert_eq!(data["summary"]["bars_changed"], 1);
                assert_eq!(data["bar_diffs"][0]["tension_delta"], 0.2);
            }
            other => panic!("期望 CompareSongSnapshots，得到 {other:?}"),
        }
    }

    #[test]
    fn plan_rejects_malformed_snapshot() {
        let call = call_with(
            json!({"schema_version": "1.1"}),
            serde_json::to_value(overview(vec![])).expect("fixture 序列化"),
        );
        let error = plan_compare_song_snapshots(&call).expect_err("缺字段的快照必须拒绝");
        let Fault::Domain { data, .. } = &error else {
            panic!("期望领域失败，得到 {error:?}");
        };
        let data = data.as_ref().expect("fault 必须带 data");
        assert_eq!(data["reason"], "invalidSnapshot");
        assert_eq!(data["snapshot"], "snapshot_a");
    }
}
