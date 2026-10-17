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

use yeban_render::logic_read::{DEFAULT_DEMOS_DIR, LogicInspection, LogicTotals, inspect_demos};

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
