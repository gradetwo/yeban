//! `#149`（刀 2）的**真导入判据**：Logic Pro `.logicx` ⇒ `.yeban` =⇒ `--headless --open`。
//!
//! 三条纪律（与 `yeban-render/tests/logic_inspect.rs` 同源）：
//!
//! 1. **路径门控**：`DEFAULT_DEMOS_DIR` 不存在 ⇒ 每条判据打印一行"无 demo、跳过"并
//!    `return`（**不红**）—— CI / 其它机器没有 Apple 演示工程，本判据不得因此变红。
//! 2. **demo 在但解析失败 ⇒ 必须红**：⛔ 不静默跳过。
//! 3. **只读 demo**：导入前后对 importer 真正读过的文件做 `sha256` 对照；⛔ demo 一个
//!    字节都不许变。产物只写到进程临时目录（⛔ 不入库）。
//!
//! 期望值全部是 `#139` / `#148` 的**实测**读数（不是从实现里抄的表达式）。

#![cfg(feature = "experimental-logic-import")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use yeban_app::cli::{self, Options};
use yeban_app::import_logic::{ImportLogicError, PLACEMENT_REPORT_LIMIT, run};
use yeban_model::ids::EntityId;
use yeban_model::project::YebanProjectV1;
use yeban_render::logic_read::{
    DEFAULT_DEMOS_DIR, LOGIC_REGION_EMPTY_LENGTH, LOGIC_REGION_LENGTH_MAX, LOGIC_REGION_LENGTH_MIN,
    is_region_length_in_range, region_placements,
};

/// `#148` 的 Swing! 实测读数（与 `yeban-render/tests/logic_inspect.rs` 同一张表）。
const SWING_TRACKS: usize = 76;
const SWING_REGIONS: usize = 545;
const SWING_NON_INTERNAL: usize = 200;
const SWING_NOTES: usize = 2_903;
const SWING_AU_RG: usize = 402;
const SWING_IMPORTED_REGIONS: usize = 174;
/// `#150`：Swing! 被长度判据挡掉的 2 条。
const SWING_OUT_OF_RANGE: usize = 2;
const SWING_SKIPPED_EMPTY: usize = 24;
const SWING_INTERNAL: usize = 345;
const SWING_NOTES_OUTSIDE_REGION: usize = 168;

/// `#150` 实测的**前 8 条**可导入摆放 `(start_tick, duration_ticks)`（记录流顺序）。
///
/// 口径 = 非内部、长度 ≠ 7,680,000、长度 ≠ 0、长度 ∈ [60, 2,196,480] 的 `qeSM`，
/// 按记录流顺序。⚠ `#149` 的前两条 `(0, 4)` 与 `(0, 1_073_626_623)` 是**垃圾**，
/// `#150` 起被挡在工程之外（逐条打印 `kind=length-out-of-range`）。
const SWING_FIRST_PLACEMENTS: [(u64, u64); 8] = [
    (0, 316_800),
    (163_200, 61_440),
    (53_760, 15_360),
    (163_200, 7_680),
    (86_400, 138_240),
    (7_680, 15_360),
    (170_880, 5_760),
    (23_040, 15_360),
];

/// demo 根目录（本机才有）。
fn swing_bundle() -> PathBuf {
    Path::new(DEFAULT_DEMOS_DIR).join("Swing!.logicx")
}

/// 路径门控：demo 不在 ⇒ 打印跳过并返回 `None`（不红）。
fn demo_present() -> bool {
    if !Path::new(DEFAULT_DEMOS_DIR).is_dir() {
        println!("logic-import: 无 demo（{DEFAULT_DEMOS_DIR} 不存在），跳过（不红）");
        return false;
    }
    true
}

/// 进程临时目录里的一枚唯一工作目录（判据结束即删）。
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "yeban-{tag}-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&root).expect("建临时目录");
        Self { root }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// `run` 的输出行里找一条以 `prefix` 开头的行。
fn line_with<'a>(lines: &'a [String], prefix: &str) -> &'a str {
    lines
        .iter()
        .find(|line| line.starts_with(prefix))
        .unwrap_or_else(|| panic!("输出里缺少 `{prefix}` 行：\n{}", lines.join("\n")))
}

/// 读出 importer 真正读过的文件字节（`bundle` 相对路径 ⇒ 字节），用于"demo 只读"对照。
fn read_files_digest(bundle: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut digests = BTreeMap::new();
    for relative in [
        "Resources/ProjectInformation.plist",
        "Alternatives/004/MetaData.plist",
        "Alternatives/004/ProjectData",
    ] {
        let path = bundle.join(relative);
        let bytes = std::fs::read(&path).expect("读 demo 文件");
        digests.insert(relative.to_owned(), bytes);
    }
    digests
}

/// 判据 1 + 3 + 7：真导入一台 ⇒ 读数与 `#148` 逐项相等、损失表四条逐字出现、
/// 前 8 条摆放与实测字面量相等。
#[test]
fn swing_import_readings_and_loss_table_match_the_measured_table() {
    if !demo_present() {
        return;
    }
    let scratch = Scratch::new("import-readings");
    let output = scratch.path("swing.yeban");
    let before = read_files_digest(&swing_bundle());

    let lines = run(&swing_bundle(), &output).expect("Swing! 必须能导入");

    // demo 只读：importer 读过的文件逐字节未变。
    assert_eq!(
        read_files_digest(&swing_bundle()),
        before,
        "导入器⛔ 不得改动 demo 的一个字节（只读契约）"
    );

    let line = line_with(&lines, "logic-import:");
    for expected in [
        format!("tracks={SWING_TRACKS}"),
        format!("regions={SWING_REGIONS}"),
        format!("non_internal={SWING_NON_INTERNAL}"),
        format!("notes={SWING_NOTES}"),
        format!("au_rg={SWING_AU_RG}"),
        "bpm=115".to_owned(),
        "sample_rate=48000".to_owned(),
        "alternative=004".to_owned(),
    ] {
        assert!(
            line.contains(&expected),
            "`logic-import:` 行必须包含 `{expected}`，实际：{line}"
        );
    }
    let counts = line_with(&lines, "logic-import-counts:");
    for expected in [
        format!("imported-regions={SWING_IMPORTED_REGIONS}"),
        format!("skipped-empty={SWING_SKIPPED_EMPTY}"),
        format!("skipped-out-of-range={SWING_OUT_OF_RANGE}"),
        format!("internal={SWING_INTERNAL}"),
        format!("notes-outside-region={SWING_NOTES_OUTSIDE_REGION}"),
        "gen_m=634".to_owned(),
        format!("clips-pool={SWING_IMPORTED_REGIONS}"),
        "project-notes=2903".to_owned(),
        "measured-track-regions=164".to_owned(),
        "provisional-track-regions=10".to_owned(),
    ] {
        assert!(
            counts.contains(&expected),
            "`logic-import-counts:` 行必须包含 `{expected}`，实际：{counts}"
        );
    }

    // 前 8 条摆放（CLI 逐条打印；报告与判据看到同一组数字）。
    for (index, (start, length)) in SWING_FIRST_PLACEMENTS
        .iter()
        .take(PLACEMENT_REPORT_LIMIT)
        .enumerate()
    {
        let prefix = format!("logic-import-placement: index={index} ");
        let placement = line_with(&lines, &prefix);
        assert!(
            placement.contains(&format!("start_tick={start}"))
                && placement.contains(&format!("duration_ticks={length}")),
            "第 {index} 条摆放必须是 ({start}, {length})，实际：{placement}"
        );
    }

    // 损失表：四条必需条目逐字出现（`#151` 起"轨道归属"一行写明实测绑定数与余下条数）。
    let text = lines.join("\n");
    for required in ["AuRg", "GenM", "自动化", "轨道归属"] {
        assert!(
            text.contains(required),
            "损失表必须逐字包含 `{required}`：\n{text}"
        );
    }
    // `#151` 实测：Swing! 的可导入区域 174 条里 **164** 条有摆放事件、**10** 条没有。
    for required in [
        "164/174 条可导入区域按摆放事件",
        "余下 10 条区域仍 provisional",
    ] {
        assert!(
            text.contains(required),
            "损失表必须逐字包含 `{required}`：\n{text}"
        );
    }
    // `#150`：两条垃圾样本**逐条**打印（⛔ 不是只报总数）。
    for rejected in [
        "level=boundary kind=length-out-of-range start=0 length=4",
        "level=boundary kind=length-out-of-range start=0 length=1073626623",
    ] {
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("logic-import-loss: ") && line.contains(rejected)),
            "损失表必须逐条打印 `{rejected}`：\n{text}"
        );
    }
    assert!(output.is_file(), "必须真的写出 `.yeban` 容器");
}

/// 判据 2：产物可 `--headless --open`（rc 0 由 `Ok` 表示）＋ `project-counts:` 行。
#[test]
fn the_imported_project_opens_through_the_headless_path() {
    if !demo_present() {
        return;
    }
    let scratch = Scratch::new("import-open");
    let output = scratch.path("swing.yeban");
    run(&swing_bundle(), &output).expect("先导入");

    let options = Options {
        headless: true,
        open: Some(output.clone()),
        ..Options::default()
    };
    let lines = cli::run_batch(&options).expect("`--headless --open` 必须成功");
    assert!(
        lines.iter().any(|line| line == cli::HEADLESS_HANDSHAKE),
        "无窗口路径必须打印握手行：\n{}",
        lines.join("\n")
    );
    let counts = line_with(&lines, "project-counts:");
    for expected in [
        "tracks-all=77",
        "master-track=1",
        "clips-pool=174",
        "midi-notes=2903",
    ] {
        assert!(
            counts.contains(expected),
            "`project-counts:` 必须包含 `{expected}`，实际：{counts}"
        );
    }
}

/// 判据 3：摆放真的写进工程；逐条与 `#148` 的 `region_placements` 读数对照。
#[test]
fn placements_are_written_and_match_the_148_readings() {
    if !demo_present() {
        return;
    }
    let scratch = Scratch::new("import-placements");
    let output = scratch.path("swing.yeban");
    run(&swing_bundle(), &output).expect("先导入");
    let project: YebanProjectV1 =
        yeban_app::open::open_project_file(&output).expect("必须能打开导入产物");

    // 从工程读回摆放：placement id 随区域序号递增 ⇒ 按 id 升序 = 记录流顺序。
    let mut written: Vec<(EntityId, u64, u64, usize, EntityId)> = Vec::new();
    for (track_id, track) in &project.tracks {
        for placement in track.clips.values() {
            let notes = project
                .clip_pool
                .get(&placement.clip_id)
                .and_then(|entry| entry.content.notes())
                .map_or(0, BTreeMap::len);
            written.push((
                placement.id,
                placement.start_tick,
                placement.duration_ticks,
                notes,
                *track_id,
            ));
        }
    }
    written.sort_by_key(|(id, ..)| *id);
    assert_eq!(written.len(), SWING_IMPORTED_REGIONS, "写进去的摆放条数");
    assert_eq!(
        written.first().map(|row| (row.1, row.2)),
        Some(SWING_FIRST_PLACEMENTS[0]),
        "第 1 条摆放"
    );

    // 与 `#148` 的读路径（同一个 `region_placements`）逐条对照：
    // 口径 = 非内部、长度 ≠ 哨兵、长度 ≠ 0，按记录流顺序。
    let expected: Vec<(u64, u64)> = region_placements(&swing_bundle())
        .expect("读 #148 的摆放")
        .into_iter()
        .filter(|placement| !placement.internal)
        .filter(|placement| placement.length_ticks != LOGIC_REGION_EMPTY_LENGTH)
        .filter(|placement| placement.length_ticks != 0)
        // `#150`：长度越界的垃圾区域 ⛔ 不进工程（与导入器同一个判据函数）。
        .filter(|placement| is_region_length_in_range(placement.length_ticks))
        .map(|placement| {
            (
                u64::from(placement.start_tick),
                u64::from(placement.length_ticks),
            )
        })
        .collect();
    let actual: Vec<(u64, u64)> = written.iter().map(|row| (row.1, row.2)).collect();
    assert_eq!(
        actual, expected,
        "工程里的 (start_tick, duration_ticks) 必须逐条等于 #148 的读数"
    );

    // 对齐裁决：区域长度**原值**（⛔ 不扩展到音符）；音符内容原样保留（总数 2903）。
    let total_notes: usize = written.iter().map(|row| row.3).sum();
    assert_eq!(total_notes, SWING_NOTES, "工程里的音符总数");
    assert!(
        written.iter().any(|row| row.2 == 61_440),
        "必须保留非整拍的实测长度（不做取整）"
    );
    // `#150`：写进工程的每一条摆放时长都必须在判据的范围内（垃圾 ⛔ 不落盘）。
    for row in &written {
        assert!(
            (u64::from(LOGIC_REGION_LENGTH_MIN)..=u64::from(LOGIC_REGION_LENGTH_MAX))
                .contains(&row.2),
            "写进工程的摆放时长 {} 必须落在 [{}, {}] tick 内",
            row.2,
            LOGIC_REGION_LENGTH_MIN,
            LOGIC_REGION_LENGTH_MAX
        );
    }
}

/// 判据 4：同一台导入两次 ⇒ 容器 `sha256` 相同。
#[test]
fn importing_the_same_project_twice_is_byte_identical() {
    if !demo_present() {
        return;
    }
    let scratch = Scratch::new("import-determinism");
    let first = scratch.path("first.yeban");
    let second = scratch.path("second.yeban");
    run(&swing_bundle(), &first).expect("第一次导入");
    run(&swing_bundle(), &second).expect("第二次导入");
    let first_bytes = std::fs::read(&first).expect("读第一次产物");
    let second_bytes = std::fs::read(&second).expect("读第二次产物");
    assert_eq!(
        first_bytes.len(),
        second_bytes.len(),
        "同一台导入两次必须得到同字节容器（EntityId 来自序号分配器）"
    );
    assert_eq!(
        first_bytes, second_bytes,
        "容器字节必须逐字节相同（不是只比大小）"
    );
}

/// 判据 6：路径门控 —— demo 路径不存在 ⇒ skip 不红，且**不写**输出文件。
#[test]
fn a_missing_demo_path_is_a_non_failing_skip() {
    let scratch = Scratch::new("import-gate");
    let missing = scratch.path("no-such-demos");
    let output = scratch.path("out.yeban");
    let lines = run(&missing, &output).expect("路径不存在 ⇒ 跳过，⛔ 不是错误");
    assert_eq!(lines.len(), 1, "只返回一行跳过说明：{lines:?}");
    assert!(lines[0].contains("跳过") && lines[0].contains("不红"));
    assert!(!output.exists(), "skip ⛔ 不许留下输出文件");
}

/// 判据（负向）：路径存在但不是 bundle / 空目录 ⇒ **红**（⛔ 不伪装成 skip）。
#[test]
fn an_existing_path_that_is_not_a_bundle_is_an_error() {
    let scratch = Scratch::new("import-not-a-bundle");
    let empty = scratch.path("empty");
    std::fs::create_dir_all(&empty).expect("建空目录");
    let error = run(&empty, &scratch.path("out.yeban")).expect_err("空目录必须红");
    assert!(
        matches!(error, ImportLogicError::NoBundles { .. }),
        "实际：{error:?}"
    );
}
