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
    DEFAULT_DEMOS_DIR, LOGIC_REGION_LENGTH_MAX, LOGIC_REGION_LENGTH_MIN, LogicInspection,
    LogicTotals, RegionPlacement, import_bundle, inspect_bundle, inspect_demos,
    is_region_length_in_range, placement_events, region_placements,
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

/// 判据（导入读取）：`import_bundle` 与只读探针逐项一致，且可导入区域/音符是实测的。
///
/// 这一条把 `#149` 的**读取侧**钉在 `#148` 的读数上：导入器如果换了记录行走、换了解码器、
/// 或把空区域/内部区域的过滤写错，这里会红。⛔ 它不写任何文件。
#[test]
fn the_import_reader_agrees_with_the_probe_on_the_measured_demo() {
    if !Path::new(DEFAULT_DEMOS_DIR).exists() {
        println!("logic-import-reader: 无 demo（{DEFAULT_DEMOS_DIR} 不存在），跳过（不红）");
        return;
    }
    let bundle = Path::new(DEFAULT_DEMOS_DIR).join("Swing!.logicx");
    let probe = inspect_bundle(&bundle).expect("探针必须能读 Swing!");
    let import = import_bundle(&bundle).expect("导入读取必须能读 Swing!");

    assert_eq!(import.bundle, probe.bundle);
    assert_eq!(import.alternative, probe.alternative);
    assert_eq!(import.sample_rate, probe.sample_rate);
    assert!((import.bpm - probe.bpm).abs() < 1e-9);
    assert_eq!(import.numerator, probe.numerator);
    assert_eq!(import.denominator, probe.denominator);
    assert_eq!(import.tracks, probe.tracks, "轨道数（payload[0] == 1）");
    assert_eq!(import.regions_all, probe.regions_all);
    assert_eq!(import.internal_regions, probe.internal_regions);
    assert_eq!(import.regions, probe.regions);
    assert_eq!(import.notes, probe.notes);
    assert_eq!(import.audio_regions, probe.audio_regions);

    // 实测字面量（`#148` 同一台）。
    assert_eq!(import.tracks, 76);
    assert_eq!(import.regions_all, 545);
    assert_eq!(import.regions, 200);
    assert_eq!(import.notes, 2_903);
    assert_eq!(import.internal_regions, 345);
    assert_eq!(import.automation_regions, 170);
    assert_eq!(import.rba_sequences, 175);
    assert_eq!(import.audio_regions, 402);
    assert_eq!(import.channel_states, 634);
    assert_eq!(import.imported_regions.len(), 174);
    assert_eq!(import.skipped_empty_regions, 24);
    assert_eq!(import.skipped_zero_length_regions, 0);
    assert_eq!(import.out_of_range_regions.len(), 2);
    assert_eq!(import.unpaired_notes, 0);
    assert_eq!(import.notes_outside_region, 168);

    // `#150`：两条垃圾样本被**逐条**留在读数里（⛔ 不是只计数）。
    let rejected: Vec<(&str, u32, u32, usize)> = import
        .out_of_range_regions
        .iter()
        .map(|region| {
            (
                region.name.as_str(),
                region.start_tick,
                region.length_ticks,
                region.notes.len(),
            )
        })
        .collect();
    assert_eq!(
        rejected,
        vec![("", 0, 4, 0), ("TRASH", 0, 1_073_626_623, 0)],
        "Swing! 的两条越界样本（记录流顺序、逐字读数）"
    );

    // 音符没有在过滤里丢掉：可导入区域的音符数 == 探针的音符总数。
    let imported_notes: usize = import
        .imported_regions
        .iter()
        .map(|region| region.notes.len())
        .sum();
    assert_eq!(imported_notes, probe.notes, "可导入区域的音符数");

    // 前 5 条**通过判据**的可导入区域（记录流顺序）与实测字面量。
    let first: Vec<(&str, u32, u32, usize)> = import
        .imported_regions
        .iter()
        .take(5)
        .map(|region| {
            (
                region.name.as_str(),
                region.start_tick,
                region.length_ticks,
                region.notes.len(),
            )
        })
        .collect();
    assert_eq!(
        first,
        vec![
            ("Swing!", 0, 316_800, 0),
            ("Fragile Keys", 163_200, 61_440, 96),
            ("Glass Piano", 53_760, 15_360, 38),
            ("Snaps", 163_200, 7_680, 9),
            ("Glass Piano", 86_400, 138_240, 282),
        ]
    );

    // 每条可导入区域都能在 `#148` 的摆放读数里逐条找到（同起点、同长度）。
    let placements = region_placements(&bundle).expect("读 #148 的摆放");
    for region in &import.imported_regions {
        assert!(
            placements.iter().any(|placement| {
                !placement.internal
                    && placement.start_tick == region.start_tick
                    && placement.length_ticks == region.length_ticks
            }),
            "可导入区域 `{}`（{}+{}）必须能对应到 #148 的摆放读数",
            region.name,
            region.start_tick,
            region.length_ticks
        );
    }
}

/// 判据（`#150` 长度判据）：8 台**逐台**的越界条数、垃圾值分布与边界口径。
///
/// 这一条是 `#149` 暴露的那个洞的判据：单条时长的垃圾**不进任何总数**
/// （`tracks/regions/non_internal/notes` 8/8 台仍然全对），所以只有**逐条**的
/// 读数能判它。⛔ 不许把它写成"合计 17 条"了事 —— 下面逐台钉住。
#[test]
fn the_length_bound_rejects_the_measured_garbage_on_every_demo() {
    if !Path::new(DEFAULT_DEMOS_DIR).exists() {
        println!("logic-import-bounds: 无 demo（{DEFAULT_DEMOS_DIR} 不存在），跳过（0 台，不红）");
        return;
    }

    // (bundle, 可导入条数, 被挡条数) —— `#150` 实测（`#149` 的 881 条减去 17 条）。
    const BOUNDS_EXPECTED: [(&str, usize, usize); 8] = [
        ("Colors", 155, 2),
        ("Manzana", 113, 2),
        ("Manzana - Spatial Audio", 113, 2),
        ("MONTERO", 41, 3),
        ("MONTERO - Spatial Audio", 41, 3),
        ("ocean eyes", 121, 1),
        ("Spatial Audio Demo Grid", 106, 2),
        ("Swing!", 174, 2),
    ];

    let mut rejected_lengths: Vec<u32> = Vec::new();
    let mut total_imported = 0usize;
    for (bundle, expected_imported, expected_rejected) in BOUNDS_EXPECTED {
        let path = Path::new(DEFAULT_DEMOS_DIR).join(format!("{bundle}.logicx"));
        let import =
            import_bundle(&path).unwrap_or_else(|error| panic!("{bundle} 的导入读取失败: {error}"));
        let expected = EXPECTED
            .iter()
            .find(|expected| expected.bundle == bundle)
            .unwrap_or_else(|| panic!("缺少 demo `{bundle}` 的实测表"));
        // `#149` 的口径：计数**不受**长度判据影响（8/8 台逐项相等）。
        assert_eq!(import.tracks, expected.tracks, "{bundle} 的轨道数");
        assert_eq!(
            import.regions_all, expected.regions,
            "{bundle} 的 qeSM 条数"
        );
        assert_eq!(
            import.regions, expected.non_internal,
            "{bundle} 的非内部区域条数"
        );
        assert_eq!(import.notes, expected.notes, "{bundle} 的音符条数");

        assert_eq!(
            import.imported_regions.len(),
            expected_imported,
            "{bundle} 的可导入区域条数"
        );
        assert_eq!(
            import.out_of_range_regions.len(),
            expected_rejected,
            "{bundle} 的被挡条数"
        );
        for region in &import.imported_regions {
            assert!(
                is_region_length_in_range(region.length_ticks),
                "{bundle} 的可导入区域 `{}` 长度 {} 必须在 [{}, {}] 内",
                region.name,
                region.length_ticks,
                LOGIC_REGION_LENGTH_MIN,
                LOGIC_REGION_LENGTH_MAX
            );
        }
        for region in &import.out_of_range_regions {
            assert!(
                !is_region_length_in_range(region.length_ticks),
                "{bundle} 被挡的区域 `{}` 长度 {} 必须真的越界",
                region.name,
                region.length_ticks
            );
            assert_eq!(
                region.notes.len(),
                0,
                "{bundle} 被挡的区域 `{}` 不含音符（挡掉它 ⛔ 不丢音符）",
                region.name
            );
            rejected_lengths.push(region.length_ticks);
        }
        total_imported += import.imported_regions.len();
    }

    assert_eq!(total_imported, 864, "8 台的可导入区域合计（881 − 17）");
    assert_eq!(rejected_lengths.len(), 17, "8 台合计被挡条数");
    // 垃圾值的**分布**：11 条同一个 10.7 亿 tick 字面量 + 6 条 4 tick。
    let huge = rejected_lengths
        .iter()
        .filter(|length| **length == 1_073_626_623)
        .count();
    let tiny = rejected_lengths
        .iter()
        .filter(|length| **length == 4)
        .count();
    assert_eq!(
        (huge, tiny),
        (11, 6),
        "垃圾值分布：11 条 1.07e9 + 6 条 4 tick"
    );

    // 判据自身的边界口径（含两端的**开区间**）与两个常量的依据。
    assert!(is_region_length_in_range(LOGIC_REGION_LENGTH_MIN));
    assert!(!is_region_length_in_range(LOGIC_REGION_LENGTH_MIN - 1));
    assert!(is_region_length_in_range(LOGIC_REGION_LENGTH_MAX));
    assert!(!is_region_length_in_range(LOGIC_REGION_LENGTH_MAX + 1));
    assert!(!is_region_length_in_range(4));
    assert!(!is_region_length_in_range(1_073_626_623));
    // 下界 = 1/64 拍（960 PPQ）；上界 = 4 × 实测最大正常值 549,120。
    assert_eq!(LOGIC_REGION_LENGTH_MIN, 960 / 16);
    assert_eq!(LOGIC_REGION_LENGTH_MAX, 4 * 549_120);
}

/// 判据（`#151`）：轨道号来自**摆放事件** `+0x14`，⛔ 不在 `qeSM` 载荷里。
///
/// 三条读数（8 台实测，逐台钉死）：
///
/// 1. **`qeSM` 里没有轨道号字段**：`rest+0x00..0x117` 的 `u8`/`u16`/`u32` 全扫过，
///    **没有任何一个**偏移同时满足「1 ≤ 值 ≤ 轨数」与「每台 ≥ 2 个不同值」
///    （`rest+0xd8` 实测是 `4` / `0x1d8` 这类小组 id，`rest+0xec` 落在名字长度不同的
///    记录之外）。因此本判据不引用任何"轨道号偏移"常量（它不存在），只判**事件**。
/// 2. **绑定是声的**：每条 `track_1based = Some(t)` 的区域都能找到一条摆放事件，
///    其 `position_tick − 34560` **等于**区域起点、`track_1based` **等于** `t`，且
///    `1 ≤ t ≤ tracks`。位置相等是独立于"槽位相等"的第二条证据（槽位那一条由
///    `import_bundle` 施加）。
/// 3. **覆盖是部分的**：8 台合计 **651/864**；`Spatial Audio Demo Grid` 只有 **5/106**。
///    余下的区域**没有**摆放事件（形态与同一位置的 take／Track Alternatives 一致），
///    判据⛔ 不许把它们写成已绑定 —— 导入器对它们保留 provisional 并逐条计数。
#[test]
fn track_binding_comes_from_placement_events_and_is_partial_on_every_demo() {
    if !Path::new(DEFAULT_DEMOS_DIR).exists() {
        println!("logic-track-binding: 无 demo（{DEFAULT_DEMOS_DIR} 不存在），跳过（0 台，不红）");
        return;
    }

    /// 区域摆放的计数原点（实测字面量，⛔ 不从实现常量抄）。
    const REGION_ORIGIN: u32 = 34_560;

    // (bundle, 可导入区域, 绑定到实测轨道的条数) —— `#151` 实测。
    const BINDING_EXPECTED: [(&str, usize, usize); 8] = [
        ("Colors", 155, 89),
        ("Manzana", 113, 107),
        ("Manzana - Spatial Audio", 113, 107),
        ("MONTERO", 41, 37),
        ("MONTERO - Spatial Audio", 41, 37),
        ("ocean eyes", 121, 105),
        ("Spatial Audio Demo Grid", 106, 5),
        ("Swing!", 174, 164),
    ];

    let mut total_regions = 0usize;
    let mut total_bound = 0usize;
    for (bundle, expected_imported, expected_bound) in BINDING_EXPECTED {
        let path = Path::new(DEFAULT_DEMOS_DIR).join(format!("{bundle}.logicx"));
        let import =
            import_bundle(&path).unwrap_or_else(|error| panic!("{bundle} 的导入读取失败: {error}"));
        let events = placement_events(&path)
            .unwrap_or_else(|error| panic!("{bundle} 的摆放事件读取失败: {error}"));
        assert!(
            !events.is_empty(),
            "{bundle} 必须至少解析出一条 MIDI 摆放事件"
        );

        assert_eq!(
            import.imported_regions.len(),
            expected_imported,
            "{bundle} 的可导入区域条数"
        );
        assert_eq!(
            import.measured_track_regions(),
            expected_bound,
            "{bundle} 绑定到实测轨道的区域条数"
        );
        assert_eq!(
            import.provisional_track_regions(),
            expected_imported - expected_bound,
            "{bundle} 仍 provisional 的区域条数"
        );

        // ② 每条绑定的区域都必须有一条**位置与轨道号都相等**的摆放事件。
        let mut matched = 0usize;
        for region in &import.imported_regions {
            let Some(track) = region.track_1based else {
                continue;
            };
            assert!(
                region.is_bound_to_measured_track(import.tracks),
                "{bundle} 的区域 `{}` 轨道号 {track} 必须在 1..={}",
                region.name,
                import.tracks
            );
            assert!(
                events.iter().any(|event| {
                    event.track_1based == track
                        && event.position_tick.checked_sub(REGION_ORIGIN) == Some(region.start_tick)
                }),
                "{bundle} 的区域 `{}`（起点 {}、轨道 {track}）找不到位置/轨道号都相等的摆放事件",
                region.name,
                region.start_tick
            );
            matched += 1;
        }
        assert_eq!(
            matched, expected_bound,
            "{bundle} 的实测绑定条数必须逐条有事件支持"
        );

        println!(
            "logic-track-binding: bundle={bundle} tracks={} imported={expected_imported} \
             bound={expected_bound} provisional={}",
            import.tracks,
            expected_imported - expected_bound,
        );
        total_regions += expected_imported;
        total_bound += expected_bound;
    }

    assert_eq!(total_regions, 864, "8 台的可导入区域合计");
    assert_eq!(total_bound, 651, "8 台绑定到实测轨道的区域合计");
    assert!(
        total_bound < total_regions,
        "诚实口径：余下 {} 条没有摆放事件，仍是 provisional（判据⛔ 不假装全覆盖）",
        total_regions - total_bound
    );
}

/// `#152`：`AuRg`（音频区域）/ `lFuA`（文件引用）/ 音频摆放事件的逐台读数。
///
/// 三条**可伪证**的约束（全部来自字节，⛔ 不读实现里的表达式）：
/// 1. 每条 `AuRg` 的记录头对象号都能找到**同号**的 `lFuA`（8/8 台，0 例外）；
/// 2. 每条**音频摆放事件**的 `+0x2c` 槽位都能找到 `AuRg`（起点只在事件里）；
/// 3. 区域名非空、take 序号 < 64、长度帧 > 0。
///
/// 逐台期望值是 `#152` 的实测条数（不是从实现里抄的）。
#[test]
fn audio_regions_files_and_placements_match_the_measured_inventory() {
    if !Path::new(DEFAULT_DEMOS_DIR).is_dir() {
        println!("logic-audio: 无 demo（{DEFAULT_DEMOS_DIR} 不存在），跳过（不红）");
        return;
    }
    let demos = inspect_demos(Path::new(DEFAULT_DEMOS_DIR))
        .expect("demo 目录存在 ⇒ 解析不得失败")
        .expect("目录存在时必须有 8 台");
    let mut total_regions = 0usize;
    let mut total_placements = 0usize;
    for inspection in &demos {
        let (au_rg, files, placements, frames_exact, bad_tracks) = match inspection.bundle.as_str()
        {
            "Colors" => (697, 260, 351, 0, 0),
            "MONTERO" => (1_014, 196, 720, 0, 0),
            "MONTERO - Spatial Audio" => (1_013, 196, 715, 0, 0),
            "Manzana" => (154, 26, 148, 14, 0),
            "Manzana - Spatial Audio" => (195, 26, 148, 14, 0),
            "Spatial Audio Demo Grid" => (102, 85, 98, 0, 0),
            "Swing!" => (402, 181, 152, 32, 0),
            // `#152` 实测：ocean eyes 有 4 条音频摆放写在轨道 43..46，
            // 而 `karT` 的实测轨数只有 42 ⇒ 导入器逐条记损失（⛔ 不猜轨道）。
            "ocean eyes" => (527, 144, 449, 0, 4),
            other => panic!("未登记读数的 demo：{other}"),
        };
        let bundle = Path::new(DEFAULT_DEMOS_DIR).join(format!("{}.logicx", inspection.bundle));
        let import = import_bundle(&bundle).expect("import_bundle 必须成功");
        assert_eq!(
            import.audio_regions, au_rg,
            "{} 的 AuRg 条数",
            inspection.bundle
        );
        assert_eq!(
            import.audio_region_records.len(),
            au_rg,
            "{} 的 AuRg 逐条读数",
            inspection.bundle
        );
        assert_eq!(
            import.audio_files.len(),
            files,
            "{} 的 lFuA 条数",
            inspection.bundle
        );
        assert_eq!(
            import.audio_placements.len(),
            placements,
            "{} 的音频摆放事件条数",
            inspection.bundle
        );
        // ① 每个 AuRg 对象号都有同号的 lFuA（8/8 台，0 例外）。
        for region in &import.audio_region_records {
            assert!(
                import.audio_file_for_slot(region.object >> 16).is_some(),
                "{} 的区域 `{}`（对象 {:08x}）没有 lFuA 文件引用",
                inspection.bundle,
                region.name,
                region.object
            );
            assert!(
                !region.name.is_empty(),
                "{} 有空名字的 AuRg",
                inspection.bundle
            );
            assert!(
                region.take_index < 0x1_0000,
                "{} 的 take 序号越界",
                inspection.bundle
            );
            assert!(
                region.length_frames > 0,
                "{} 的 AuRg 长度帧为 0",
                inspection.bundle
            );
        }
        // ② 每个音频摆放事件的槽位都有 AuRg（起点只在事件里）。
        let mut bad = 0usize;
        for placement in &import.audio_placements {
            assert!(
                import
                    .audio_region_for_slot(placement.region_slot)
                    .is_some(),
                "{} 的音频摆放（start={} track={}）槽位 {:x} 没有 AuRg",
                inspection.bundle,
                placement.start_tick,
                placement.track_1based,
                placement.region_slot
            );
            if !(1..=u32::try_from(import.tracks).expect("轨数")).contains(&placement.track_1based)
            {
                bad += 1;
            }
        }
        assert_eq!(
            bad, bad_tracks,
            "{} 的越界轨道号音频摆放条数（实测表）",
            inspection.bundle
        );
        // ③ 长度帧 → tick：采样的**长度**在 Swing!/Manzana 上有 `<frames_exact>` 条
        //    恰好落在 960 tick 的整数倍上（素材率换算；见 `#152` 报告的手算对照）。
        let mut exact = 0usize;
        for region in &import.audio_region_records {
            let Some(file) = import.audio_file_for_slot(region.object >> 16) else {
                continue;
            };
            let path = bundle.join("Media").join("Audio Files").join(&file.name);
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let Some(rate) = aiff_sample_rate(&bytes) else {
                continue;
            };
            let ticks = (f64::from(region.length_frames) * 960.0 * import.bpm
                / (60.0 * f64::from(rate)))
            .round();
            if (ticks / 960.0).fract().abs() < 0.000_1 {
                exact += 1;
            }
        }
        if frames_exact > 0 {
            assert!(
                exact >= frames_exact,
                "{} 的 960 整数倍长度必须至少 {frames_exact} 条（实际 {exact}）",
                inspection.bundle
            );
        }
        println!(
            "logic-audio: bundle={} au_rg={} files={} placements={} exact_bar_lengths={exact}",
            inspection.bundle, au_rg, files, placements
        );
        total_regions += au_rg;
        total_placements += placements;
    }
    assert_eq!(total_regions, 4_104, "8 台 AuRg 合计（#139 实测）");
    assert_eq!(total_placements, 2_781, "8 台音频摆放事件合计");
}

/// AIFF `COMM` 块的采样率（只给判据用；非 AIFF/短文件 ⇒ `None`）。
fn aiff_sample_rate(bytes: &[u8]) -> Option<u32> {
    if bytes.len() < 12 || &bytes[..4] != b"FORM" || &bytes[8..12] != b"AIFF" {
        return None;
    }
    let mut at = 12usize;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_be_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]])
            as usize;
        let body = at + 8;
        if id == b"COMM" && size >= 18 && body + 18 <= bytes.len() {
            let exponent = u16::from_be_bytes([bytes[body + 8], bytes[body + 9]]);
            let mantissa = u64::from_be_bytes([
                bytes[body + 10],
                bytes[body + 11],
                bytes[body + 12],
                bytes[body + 13],
                bytes[body + 14],
                bytes[body + 15],
                bytes[body + 16],
                bytes[body + 17],
            ]);
            let value =
                (mantissa as f64 / (1u64 << 63) as f64) * 2f64.powi(i32::from(exponent) - 16383);
            return u32::try_from(value.round() as i64).ok();
        }
        at = body.checked_add(size)?.checked_add(size & 1)?;
    }
    None
}
