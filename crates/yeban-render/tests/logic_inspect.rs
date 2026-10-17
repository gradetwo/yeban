//! #143 的**只读探针判据**：把 `#139` 勘察到的 8 个 Apple Logic demo 变成可复跑的仓库判据。
//!
//! 三条纪律：
//!
//! 1. **路径门控**：`DEFAULT_DEMOS_DIR` 不存在 ⇒ 打印一行"无 demo、跳过"并 `return`
//!    （**不红**）—— CI/其它机器上没有 Apple 演示工程，本判据不得因此变红。
//! 2. **demo 在但解析失败 ⇒ 必须红**：⛔ 不静默跳过（那会把"解析坏了"伪装成"demo 不在"）。
//! 3. **只读**：demo 目录一个字都不改；本判据不写任何文件，也不把 demo 或解析结果入库
//!    （Apple 演示工程有版权）。
//!
//! 期望值全部是 `#139` 的**实测**读数（不是从实现里抄的表达式），逐项相等才算绿。

#![cfg(feature = "experimental-logic-import")]

use std::path::Path;

use yeban_render::logic_read::{
    DEFAULT_DEMOS_DIR, LogicInspection, LogicTotals, RegionPlacement, inspect_demos,
    region_placements,
};

/// 一台 demo 的期望读数（`#139` 实测）。
struct Expected {
    /// `.logicx` 目录名（不含后缀）。
    bundle: &'static str,
    /// `ProjectInformation.plist` 的 `ActiveVariant` 三位目录名。
    alternative: &'static str,
    /// `MetaData.plist` 的 `SampleRate`。
    sample_rate: u32,
    /// `MetaData.plist` 的 `BeatsPerMinute`。
    bpm: f64,
    /// `payload[0] == 1` 的 `karT` 条数。
    tracks: usize,
    /// 所有 `qeSM` 条数（过滤前）。
    regions: usize,
    /// 非内部区域条数。
    non_internal: usize,
    /// 音符条数。
    notes: usize,
}

/// `#139` 的 8/8 实测表。
const EXPECTED: [Expected; 8] = [
    Expected {
        bundle: "Colors",
        alternative: "000",
        sample_rate: 44_100,
        bpm: 120.0,
        tracks: 135,
        regions: 774,
        non_internal: 232,
        notes: 2_007,
    },
    Expected {
        bundle: "Manzana",
        alternative: "003",
        sample_rate: 44_100,
        bpm: 146.0,
        tracks: 81,
        regions: 464,
        non_internal: 139,
        notes: 1_519,
    },
    Expected {
        bundle: "Manzana - Spatial Audio",
        alternative: "003",
        sample_rate: 48_000,
        bpm: 146.0,
        tracks: 81,
        regions: 465,
        non_internal: 139,
        notes: 1_519,
    },
    Expected {
        bundle: "MONTERO",
        alternative: "002",
        sample_rate: 44_100,
        bpm: 179.0,
        tracks: 139,
        regions: 706,
        non_internal: 56,
        notes: 7,
    },
    Expected {
        bundle: "MONTERO - Spatial Audio",
        alternative: "001",
        sample_rate: 48_000,
        bpm: 179.0,
        tracks: 140,
        regions: 705,
        non_internal: 56,
        notes: 7,
    },
    Expected {
        bundle: "ocean eyes",
        alternative: "001",
        sample_rate: 44_100,
        bpm: 145.0,
        tracks: 42,
        regions: 245,
        non_internal: 136,
        notes: 1_823,
    },
    Expected {
        bundle: "Spatial Audio Demo Grid",
        alternative: "000",
        sample_rate: 44_100,
        bpm: 120.0,
        tracks: 20,
        regions: 240,
        non_internal: 116,
        notes: 40,
    },
    Expected {
        bundle: "Swing!",
        alternative: "004",
        sample_rate: 48_000,
        bpm: 115.0,
        tracks: 76,
        regions: 545,
        non_internal: 200,
        notes: 2_903,
    },
];

/// 合计期望（`#139` 实测）。
const EXPECTED_TOTALS: LogicTotals = LogicTotals {
    demos: 8,
    tracks: 714,
    regions_all: 4_144,
    regions: 1_074,
    notes: 9_825,
    audio_regions: 4_104,
};

/// `#148` 的区域摆放期望（8 台 demo 的**实测字面量**，不是从实现反算）。
///
/// 口径：
/// * `note_regions` = 其后 `qSvE` 里 `decode_note_lines` 非空的 `qeSM` 条数；
/// * `start_sum` / `length_sum` = 这些区域的 `rest+0xe0` / `rest+0x3c` 之和（tick）；
/// * `beat_aligned` = 起点能被 960（一拍）整除的条数；
/// * `start_max` = **所有** `qeSM`（含空区域/内部区域）起点的最大值（tick）。
struct PlacementExpected {
    /// `.logicx` 目录名（不含后缀）。
    bundle: &'static str,
    /// 所有 `qeSM` 条数（= `region_placements().len()`）。
    regions_all: usize,
    /// 有音符的区域条数。
    note_regions: usize,
    /// 有音符区域的起点之和。
    start_sum: u64,
    /// 有音符区域的长度之和。
    length_sum: u64,
    /// 起点是整拍的条数。
    beat_aligned: usize,
    /// 所有区域起点的最大值。
    start_max: u32,
}

/// `#148` 实测表（8/8）。
const PLACEMENT_EXPECTED: [PlacementExpected; 8] = [
    PlacementExpected {
        bundle: "Colors",
        regions_all: 774,
        note_regions: 122,
        start_sum: 26_989_680,
        length_sum: 2_568_720,
        beat_aligned: 117,
        start_max: 489_600,
    },
    PlacementExpected {
        bundle: "Manzana",
        regions_all: 464,
        note_regions: 54,
        start_sum: 9_968_640,
        length_sum: 977_760,
        beat_aligned: 54,
        start_max: 422_400,
    },
    PlacementExpected {
        bundle: "Manzana - Spatial Audio",
        regions_all: 465,
        note_regions: 54,
        start_sum: 9_968_640,
        length_sum: 977_760,
        beat_aligned: 54,
        start_max: 422_400,
    },
    PlacementExpected {
        bundle: "MONTERO",
        regions_all: 706,
        note_regions: 1,
        start_sum: 199_680,
        length_sum: 24_960,
        beat_aligned: 1,
        start_max: 460_320,
    },
    PlacementExpected {
        bundle: "MONTERO - Spatial Audio",
        regions_all: 705,
        note_regions: 1,
        start_sum: 199_680,
        length_sum: 24_960,
        beat_aligned: 1,
        start_max: 460_320,
    },
    PlacementExpected {
        bundle: "ocean eyes",
        regions_all: 245,
        note_regions: 35,
        start_sum: 6_481_920,
        length_sum: 2_726_400,
        beat_aligned: 35,
        start_max: 424_720,
    },
    PlacementExpected {
        bundle: "Spatial Audio Demo Grid",
        regions_all: 240,
        note_regions: 5,
        start_sum: 0,
        length_sum: 76_800,
        beat_aligned: 5,
        start_max: 92_160,
    },
    PlacementExpected {
        bundle: "Swing!",
        regions_all: 545,
        note_regions: 154,
        start_sum: 26_388_472,
        length_sum: 1_592_610,
        beat_aligned: 146,
        start_max: 303_359,
    },
];

/// 读 demo；目录不存在 ⇒ 打印"跳过"并返回 `None`（**不红**）。
fn load_demos() -> Option<Vec<LogicInspection>> {
    // demo 在但解析失败 ⇒ `expect` 变红：这正是"不静默跳过"的要求。
    let inspections = inspect_demos(Path::new(DEFAULT_DEMOS_DIR))
        .expect("demo 目录存在时，解析失败必须红（⛔ 不静默跳过）");
    match inspections {
        Some(inspections) => Some(inspections),
        None => {
            println!("logic-inspect: 无 demo（{DEFAULT_DEMOS_DIR} 不存在），跳过（0 台，不红）");
            None
        }
    }
}

/// 判据 1 + 2：8 台逐项对照 + 合计相等（逐项相等才算绿）。
#[test]
fn eight_apple_demos_match_the_measured_table_and_totals() {
    let Some(inspections) = load_demos() else {
        return;
    };

    assert_eq!(
        inspections.len(),
        EXPECTED.len(),
        "读到的 demo 台数与实测表不同（下面逐台对照会指出缺哪台/多哪台）"
    );

    for expected in &EXPECTED {
        let actual = inspections
            .iter()
            .find(|inspection| inspection.bundle == expected.bundle)
            .unwrap_or_else(|| panic!("缺少 demo `{}`", expected.bundle));

        // 逐台打印读数 —— 报告与判据看到的是同一组数字。
        println!("{}", actual.report_line());

        assert_eq!(
            actual.alternative, expected.alternative,
            "{} 的 ActiveVariant 目录（⛔ 不硬编码 000）",
            expected.bundle
        );
        assert_eq!(
            actual.sample_rate, expected.sample_rate,
            "{} 的 SampleRate",
            expected.bundle
        );
        assert!(
            (actual.bpm - expected.bpm).abs() < 1e-9,
            "{} 的 BeatsPerMinute: 期望 {}，实际 {}",
            expected.bundle,
            expected.bpm,
            actual.bpm
        );
        assert_eq!(
            actual.tracks, expected.tracks,
            "{} 的轨道数（payload[0] == 1）",
            expected.bundle
        );
        assert_eq!(
            actual.declared_tracks, actual.tracks,
            "{} 的 NumberOfTracks 必须等于 payload[0] == 1 的条数",
            expected.bundle
        );
        assert_eq!(
            actual.master_tracks, 1,
            "{} 的 payload[0] == 3 恰好一条 master",
            expected.bundle
        );
        assert_eq!(
            actual.regions_all, expected.regions,
            "{} 的 qeSM 条数（过滤前）",
            expected.bundle
        );
        assert_eq!(
            actual.regions, expected.non_internal,
            "{} 的非内部区域数",
            expected.bundle
        );
        assert_eq!(
            actual.regions + actual.internal_regions,
            actual.regions_all,
            "{} 的区域分账（非内部 + 内部 == 全部）",
            expected.bundle
        );
        assert_eq!(
            actual.notes, expected.notes,
            "{} 的音符数（decode_note_lines）",
            expected.bundle
        );
        assert!(
            actual.audio_regions > 0,
            "{} 的 AuRg 计数不得为空",
            expected.bundle
        );
        assert_eq!(
            (actual.numerator, actual.denominator),
            (4, 4),
            "{} 的拍号（8/8 实测都是 4/4）",
            expected.bundle
        );
    }

    let totals = LogicTotals::of(&inspections);
    println!("{}", totals.report_line());
    assert_eq!(totals, EXPECTED_TOTALS, "合计必须逐项相等");
    assert_eq!(
        totals.regions_all, 4_144,
        "qeSM 合计（过滤前，`#139` 实测）"
    );
    assert_eq!(totals.regions, 1_074, "非内部区域合计");
}

/// 判据 3：过滤有牙 —— 两个方向都给读数。
///
/// * 不过滤 `payload[0] == 0` 的非轨道条 ⇒ Colors 得 **138 ≠ 135**；
/// * 不过滤 `*Automation` / `RBA Sequence` ⇒ 区域 **4,144 ≠ 1,074**。
#[test]
fn filters_have_teeth_on_the_measured_examples() {
    let Some(inspections) = load_demos() else {
        return;
    };

    let colors = inspections
        .iter()
        .find(|inspection| inspection.bundle == "Colors")
        .expect("Colors demo");
    println!(
        "filter-teeth: Colors tracks={} tracks_including_zero_rows={}",
        colors.tracks, colors.tracks_including_zero_rows
    );
    assert_eq!(
        colors.tracks, 135,
        "过滤 payload[0] == 0 之后的 Colors 轨道数"
    );
    assert_eq!(
        colors.tracks_including_zero_rows, 138,
        "不过滤 payload[0] == 0 时的 Colors 轨道数 —— 必须与 135 不同"
    );
    assert_ne!(colors.tracks, colors.tracks_including_zero_rows);

    let total_all: usize = inspections
        .iter()
        .map(|inspection| inspection.regions_all)
        .sum();
    let total_non_internal: usize = inspections
        .iter()
        .map(|inspection| inspection.regions)
        .sum();
    let total_internal: usize = inspections
        .iter()
        .map(|inspection| inspection.internal_regions)
        .sum();
    println!(
        "filter-teeth: regions_all={total_all} internal_regions={total_internal} \
         non_internal={total_non_internal}"
    );
    assert_eq!(total_all, 4_144, "不过滤自动化/RBA 时的 qeSM 合计");
    assert_eq!(total_non_internal, 1_074, "过滤之后的区域合计");
    assert_ne!(total_all, total_non_internal);
    assert_eq!(
        total_all,
        total_non_internal + total_internal,
        "内部 + 非内部 == 全部"
    );
}

/// 判据 4：**路径门控** —— demo 目录不存在 ⇒ skip，且**不红**。
#[test]
fn an_absent_demos_directory_is_skipped_not_failed() {
    let absent = Path::new("/nonexistent/yeban-143/no-logic-demos-here");
    assert!(
        !absent.exists(),
        "夹具路径不得意外存在: {}",
        absent.display()
    );
    let report = inspect_demos(absent).expect("不存在的 demo 目录必须 skip，⛔ 不得报错");
    assert!(report.is_none(), "不存在的目录必须返回 None");
    println!(
        "logic-inspect: 无 demo（{} 不存在），跳过（0 台，不红）",
        absent.display()
    );
}

/// 判据 5（`#148`）：**区域摆放字段**的路径门控判据。
///
/// 这一条量的是「`qeSM` 的起点/长度能不能读出来」，三条约束各自有读数：
///
/// 1. **字段存在且 8/8 一致**：每台 demo 的 `region_placements()` 条数必须等于
///    `qeSM` 条数，且起点字段在同一偏移（`rest+0xe0`）上 —— 用**钉住的起点之和**
///    与**起点最大值**判，错偏移会立刻变红；
/// 2. **单位是 tick**：有音符区域里绝大多数起点是 960（一拍）的整数倍 ——
///    钉住逐台的整拍条数（8 台合计 413/426 = 96.9%）；如果是「样本」或「拍」，不会这样；
/// 3. **上界**：每台 `max(起点+长度)` 对 `max(起点+最后音符结束)` 的比值必须落在
///    `[0.5, 1.10]`（实测 0.5517…1.0571）。空区域的哨兵长度 7,680,000 ⛔ 不参与。
///
/// **单调性读数**（不是断言）：记录流是**对象创建顺序**，不是时间顺序 ⇒ 全局
/// 「起点非递减」不成立；本判据打印逐台下降数，避免把不成立的性质写成判据。
#[test]
fn region_placement_fields_are_measured_and_bounded() {
    let Some(inspections) = load_demos() else {
        return;
    };

    for expected in &PLACEMENT_EXPECTED {
        let inspection = inspections
            .iter()
            .find(|inspection| inspection.bundle == expected.bundle)
            .unwrap_or_else(|| panic!("缺少 demo `{}`", expected.bundle));
        let bundle = Path::new(DEFAULT_DEMOS_DIR).join(format!("{}.logicx", expected.bundle));
        let placements = region_placements(&bundle)
            .unwrap_or_else(|error| panic!("{} 的区域摆放读取失败: {error}", expected.bundle));

        // ① 字段存在 + 8/8 同一偏移：条数与起点汇总都必须等于实测字面量。
        assert_eq!(
            placements.len(),
            expected.regions_all,
            "{} 的摆放条数必须等于 qeSM 条数",
            expected.bundle
        );
        assert_eq!(
            inspection.region_placements,
            placements.len(),
            "{} 的 report_line 摆放条数必须与 region_placements() 一致",
            expected.bundle
        );
        assert_eq!(
            inspection.region_start_max, expected.start_max,
            "{} 的所有区域起点最大值（rest+0xe0）",
            expected.bundle
        );
        assert!(
            inspection.report_line().contains("region_start_max=")
                && inspection.report_line().contains("region_end_max=")
                && inspection.report_line().contains("note_end_max="),
            "{} 的 --inspect-logic 行必须打印 #148 的摆放读数",
            expected.bundle
        );

        let note_regions: Vec<&RegionPlacement> = placements
            .iter()
            .filter(|region| region.notes > 0)
            .collect();
        assert_eq!(
            note_regions.len(),
            expected.note_regions,
            "{} 的有音符区域条数",
            expected.bundle
        );
        assert!(
            !note_regions.is_empty(),
            "{} 必须至少有一条有音符的区域（8/8）",
            expected.bundle
        );
        assert!(
            placements.iter().any(|region| region.start_tick > 0),
            "{} 必须至少有一条非零起点的区域（8/8）",
            expected.bundle
        );

        let start_sum: u64 = note_regions
            .iter()
            .map(|region| u64::from(region.start_tick))
            .sum();
        let length_sum: u64 = note_regions
            .iter()
            .map(|region| u64::from(region.length_ticks))
            .sum();
        assert_eq!(
            start_sum, expected.start_sum,
            "{} 的有音符区域起点之和（rest+0xe0）",
            expected.bundle
        );
        assert_eq!(
            length_sum, expected.length_sum,
            "{} 的有音符区域长度之和（rest+0x3c）",
            expected.bundle
        );

        // ② 单位：起点几乎都是整拍（960 tick）。
        let beat_aligned = note_regions
            .iter()
            .filter(|region| region.start_tick % 960 == 0)
            .count();
        assert_eq!(
            beat_aligned, expected.beat_aligned,
            "{} 的整拍起点条数（960 tick）",
            expected.bundle
        );

        // ③ 上界：有音符区域的 max(起点+长度) vs max(起点+最后音符结束)。
        let max_end = note_regions
            .iter()
            .filter(|region| !region.is_empty())
            .map(|region| region.end_tick())
            .max()
            .expect("至少一条有音符区域");
        let max_note_end = note_regions
            .iter()
            .map(|region| region.note_end_tick())
            .max()
            .expect("至少一条有音符区域");
        assert_eq!(
            inspection.region_end_max, max_end,
            "{} 的 report_line max(start+length)",
            expected.bundle
        );
        assert_eq!(
            inspection.note_end_max, max_note_end,
            "{} 的 report_line max(start+note_end)",
            expected.bundle
        );
        let ratio = max_end as f64 / max_note_end as f64;
        assert!(
            (0.5..=1.10).contains(&ratio),
            "{} 的上界比 {ratio:.4} 必须落在 [0.5, 1.10]（max_end={max_end}, max_note_end={max_note_end}）",
            expected.bundle
        );

        // 单调性：**只打印读数**（流的顺序不是时间顺序 —— 见模块文档）。
        let violations = placements
            .windows(2)
            .filter(|pair| pair[1].start_tick < pair[0].start_tick)
            .count();
        println!(
            "logic-placement: bundle={} placements={} note_regions={} start_sum={} length_sum={} \
             beat_aligned={}/{} max_end={} max_note_end={} ratio={ratio:.4} stream_order_drops={violations}",
            expected.bundle,
            placements.len(),
            note_regions.len(),
            start_sum,
            length_sum,
            beat_aligned,
            note_regions.len(),
            max_end,
            max_note_end,
        );
    }
}
