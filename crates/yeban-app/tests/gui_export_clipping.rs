//! 任务单 `#229`：**GUI 的导出也报削波**（`#226` 自己登记的待办）。
//!
//! # 本单补的是哪一格
//!
//! | 层 | `#226` 之后、本单之前 |
//! | :--- | :--- |
//! | 能力（`WavExportReport`） | ✓ `clipped_samples` / `true_peak_dbfs` / `headroom_db` / `suggested_trim_db` / `loudest_source` |
//! | CLI 出口 | ✓ `--export-wav` 削波时**多打一行** |
//! | **GUI 出口** | ⛔ **`on_export_wav` 走 `module_receipt`，回执里没有削波字段** ⇒ 人在窗口里看不到 |
//! | ⭐ **本单** | ⭐ `on_export_wav` 的成功回执在**削波时多一条** `clipping_note="…"`（内容 = CLI 同一条 `clipping_note()`）；⛔ 不削波时回执**逐字不变** |
//!
//! # 判据（两条臂 + 一条字节臂）
//!
//! | 判据 | 钉住的事实 |
//! | :--- | :--- |
//! | ①（削波臂） | 经**真窗口的回调**（`invoke_export_wav`）导出过载工程 ⇒ 回执多一条 `clipping_note`，且它含 `clippedSamples` / `suggestedTrimDb` / 最响的源 |
//! | ②（干净臂） | 同一装配导出不削波工程 ⇒ 回执**逐字等于** `#226` 之前的九字段形状（⛔ 没有 `clipping_note`、⛔ 没有"削波"） |
//! | ③（不改音频） | 两条臂的 WAV `sha256` 都等于 `#226` 之前（`main` @ `95bdfec3`）实测的常量 ⇒ 提示只加文字、一个字节都没动 |
//!
//! ## 判据怎么变红
//!
//! - 回执里删掉 `clipping_note`（或只在调试输出里打）⇒ ① 红；
//! - 把 `clipping_note` 无条件加进回执 ⇒ ② 红（干净臂多了一条）；
//! - 提示路径碰了样本 / 增益 / 量化 / 落盘顺序 ⇒ ③ 红；
//! - GUI 换一条**不是** `export_wav_with_sfz_file` 的导出器 ⇒ ③ 的字节与 CLI 常量不再相同 ⇒ 红。
//!
//! ## 平台约束：本文件只有一个 `#[test]`
//!
//! `slint::platform::set_platform` 是**线程局部**的，`Tier1Window::install` 每线程只能成功
//! 一次 ⇒ 同款用法见 `tests/ui_module_actions.rs` / `tests/export_midi_ui.rs`。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的装配入口 ⇒ 其余项在本测试目标里是 dead_code。
#[allow(dead_code)]
mod live;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::str::FromStr as _;

use live::{LiveWiringOptions, build_live_ui_with};
use yeban_app::export_wav::{WavExportOptions, export_wav};
use yeban_app::host::{ModulePaths, wire_module_actions_with};
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::{
    AssetHash, ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote,
    RoutingEdge, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;

/// 过载夹具（`volume_db = +24`、力度 127）的整文件 SHA-256 —— 测自**加读数之前**
/// （`main` @ `95bdfec3`，与 `tests/export_wav_clipping.rs` 的 `CLIPPING_WAV_SHA256` **同一个**常量）。
const CLIPPING_WAV_SHA256: &str =
    "36918563041549eac27ed22121188d64b1ab98ee533216bc8ed47c85030c1a3c";
/// 同一夹具的母带位级摘要（同上，CLI 侧常量）。
const CLIPPING_WAV_DIGEST: &str =
    "93827504fa8bfb2833b6185712c1a2f6a53accd8ddaad4358c89b60f40e940dd";
/// 不削波夹具（同一条轨、`volume_db = 0`）的整文件 SHA-256（测自 `main`）。
const CLEAN_WAV_SHA256: &str = "fa98b7a3c623b8e6b6d9929eabdc062b4f9179b2e8f7a2c443d3d9bb86eaedf0";

/// 回执字段名闭集（只用于把"值里自带的冒号"排除在切分边界之外；口径照
/// `tests/ui_module_actions.rs` 的 `receipt_field_end`）。⭐ 本单新增 `clipping_note`。
const RECEIPT_KEYS: [&str; 10] = [
    "action",
    "outcome",
    "path",
    "frames",
    "sample_rate",
    "channels",
    "bits",
    "bytes",
    "source",
    "digest",
];

fn fixture_id(tail: &str) -> EntityId {
    EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8P0{tail}")).expect("夹具 ULID 必须合法")
}

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-229-{tag}-{}-{}",
        std::process::id(),
        EntityId::new()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

fn sha_hex(path: &Path) -> String {
    AssetHash::of_bytes(&std::fs::read(path).expect("读产物"))
        .as_str()
        .to_owned()
}

fn receipt_field_end(receipt: &str, at: usize, skip: &str) -> Option<usize> {
    let mut index = at;
    while index < receipt.len() {
        if receipt.as_bytes()[index] == b':' {
            for key in RECEIPT_KEYS {
                if key == skip {
                    continue;
                }
                let rest = &receipt[index + 1..];
                if rest.starts_with(key) && rest[key.len()..].starts_with('=') {
                    return Some(index);
                }
            }
        }
        index += 1;
    }
    None
}

fn field_of(receipt: &str, key: &str) -> String {
    let needle = format!("{key}=");
    let start = receipt
        .find(&needle)
        .unwrap_or_else(|| panic!("回执里没有字段 `{key}`: {receipt}"))
        + needle.len();
    let end = receipt_field_end(receipt, start, key).unwrap_or(receipt.len());
    receipt[start..end].to_owned()
}

/// 与 `tests/export_wav_clipping.rs` **逐字同款**的最小工程：master ＋ 一条 MIDI 轨。
fn project_with_track(volume_db: f32) -> YebanProjectV1 {
    let master = fixture_id("M1");
    let track_id = fixture_id("T0");
    let clip = fixture_id("K0");
    let placement = fixture_id("P0");
    let note = fixture_id("N0");
    let edge = fixture_id("E0");
    let mut project = YebanProjectV1 {
        id: fixture_id("Z1"),
        title: "gui clipping fixture".to_owned(),
        bpm: 120.0,
        rng_seed: 0x5945_4241_4E00_0001,
        master_bus_track_id: master,
        ..YebanProjectV1::default()
    };
    project.tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    project.routing_graph.nodes.push(master);
    let mut track = TrackV3 {
        id: track_id,
        name: "Lead".to_owned(),
        kind: TrackKind::Midi,
        volume_db,
        ..TrackV3::default()
    };
    track.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: 1_920,
            loop_config: LoopConfig::default(),
            muted: false,
        },
    );
    let mut notes = BTreeMap::new();
    notes.insert(
        note,
        MidiNote {
            velocity: 127,
            ..MidiNote::new(note, 0, 69, 1_920)
        },
    );
    project.clip_pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "clip0".to_owned(),
            content: ClipContent::Midi { notes },
        },
    );
    project.tracks.insert(track_id, track);
    project.routing_graph.nodes.push(track_id);
    project.routing_graph.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: track_id,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        },
    );
    project.validate().expect("夹具工程必须合法");
    project
}

#[test]
fn the_gui_export_reports_clipping_only_when_the_master_clips() {
    let dir = scratch_dir("gui");
    let clipping_project = project_with_track(24.0);
    let clean_project = project_with_track(0.0);

    // ---- 装配：真窗口 + Tier-1 执行面 + 撤销端口（GUI 生产路径的同一个函数） ----
    let clipping_port = Rc::new(UndoPort::new(
        UndoSession::open(
            "<判据 #229: 削波>",
            "yeban-app",
            clipping_project.clone(),
            1_760_000_000_000,
        )
        .expect("打开撤销会话"),
    ));
    let options = LiveWiringOptions {
        permission: Permission::Interactive,
        undo: Some(Rc::clone(&clipping_port)),
        ..LiveWiringOptions::default()
    };
    let live_ui = build_live_ui_with(&clipping_project, &options).expect("Tier-1 平台 + 活窗口");
    let window = slint::ComponentHandle::clone_strong(live_ui.ui());

    // =====================================================================
    // ① 削波臂：按下 export-wav ⇒ 回执**多一条** `clipping_note`
    // =====================================================================
    let clip_path = dir.join("clip.wav");
    let mut paths = ModulePaths::from_env();
    paths.export_wav = clip_path.clone();
    wire_module_actions_with(&window, Some(Rc::clone(&clipping_port)), paths.clone());
    assert_eq!(window.get_module_status(), "", "按之前回执必须是空的");
    window.invoke_export_wav();
    let receipt = window.get_module_status().to_string();
    report_line(&format!("[#229] 削波回执（逐字）: {receipt}"));
    assert!(window.get_module_status_succeeded(), "{receipt}");
    assert_eq!(field_of(&receipt, "action"), "export-wav", "{receipt}");
    assert_eq!(field_of(&receipt, "outcome"), "ok", "{receipt}");
    assert_eq!(field_of(&receipt, "frames"), "384000", "{receipt}");
    assert_eq!(field_of(&receipt, "sample_rate"), "48000", "{receipt}");

    // ⭐ 本单的判据：回执里**恰好多一条** `clipping_note="…"`，且三件事都在。
    let note = field_of(&receipt, "clipping_note");
    assert!(
        note.starts_with('"') && note.ends_with('"'),
        "值必须整体带引号: {note:?}"
    );
    assert!(
        note.contains("clippedSamples"),
        "必须点名 clippedSamples: {note}"
    );
    assert!(
        note.contains("suggestedTrimDb"),
        "必须点名 suggestedTrimDb: {note}"
    );
    assert!(note.contains("最响的源"), "必须点名最响的源: {note}");
    assert!(note.contains("Lead"), "必须点名最响源的轨道名: {note}");
    assert!(note.contains("-24"), "建议 trim 必须进文案: {note}");
    assert!(
        receipt.contains(&format!("digest={CLIPPING_WAV_DIGEST}")),
        "回执里的 digest 必须是母带样本摘要: {receipt}"
    );

    // ③ 不改音频：GUI 这条路径的产物与 `#226` 之前 CLI 的常量逐字节相同。
    //    对照导出同时给出**同一份工程**的九字段回执（用来钉"恰好多一条"）。
    let clip_oracle = dir.join("clip-oracle.wav");
    let clip_report = export_wav(
        &clipping_project,
        &clip_oracle,
        WavExportOptions::resolve(&clipping_project, None, None, None),
    )
    .expect("对照导出必须成功");
    let clip_legacy = format!(
        "module:action=export-wav:outcome=ok:path={}:frames={}:sample_rate={}:channels={}:bits={}:bytes={}:source={}:digest={}",
        clip_path.display(),
        clip_report.frames,
        clip_report.sample_rate,
        clip_report.channels,
        clip_report.bits,
        clip_report.bytes,
        clip_report.source,
        clip_report.digest
    );
    let report_note = clip_report.clipping_note().expect("削波报告必须有提示");
    assert_eq!(
        note.trim_matches('"'),
        report_note,
        "回执里的提示必须是 `WavExportReport::clipping_note()` 的**逐字**内容"
    );
    assert_eq!(
        receipt,
        format!("{clip_legacy}:clipping_note=\"{report_note}\""),
        "削波回执必须**恰好**是既有九字段 ＋ 一条 `clipping_note`"
    );
    assert_eq!(
        sha_hex(&clip_path),
        CLIPPING_WAV_SHA256,
        "削波提示不得改动音频字节（GUI 产物必须等于 CLI 基线）"
    );
    assert_eq!(
        sha_hex(&clip_path),
        sha_hex(&clip_oracle),
        "GUI 路径与直接调用必须是同一份产物"
    );
    assert_eq!(
        std::fs::metadata(&clip_path)
            .expect("导出文件")
            .len()
            .to_string(),
        field_of(&receipt, "bytes"),
        "回执报的字节数必须等于磁盘上的字节数"
    );

    // =====================================================================
    // ② 干净臂：同一装配换一份不削波工程 ⇒ 回执**逐字不变**
    // =====================================================================
    let clean_path = dir.join("clean.wav");
    let clean_port = Rc::new(UndoPort::new(
        UndoSession::open(
            "<判据 #229: 干净>",
            "yeban-app",
            clean_project.clone(),
            1_760_000_000_001,
        )
        .expect("打开撤销会话"),
    ));
    let mut clean_paths = paths.clone();
    clean_paths.export_wav = clean_path.clone();
    wire_module_actions_with(&window, Some(Rc::clone(&clean_port)), clean_paths);
    window.invoke_export_wav();
    let clean_receipt = window.get_module_status().to_string();
    report_line(&format!("[#229] 干净回执（逐字）: {clean_receipt}"));
    assert!(window.get_module_status_succeeded(), "{clean_receipt}");
    assert!(
        !clean_receipt.contains("clipping_note") && !clean_receipt.contains("削波"),
        "不削波时回执不得多任何字段: {clean_receipt}"
    );

    // 逐字：用**同一个导出器**在干净工程上的报告重建既有九字段形状（顺序也逐字对上）。
    let oracle_path = dir.join("oracle.wav");
    let report = export_wav(
        &clean_project,
        &oracle_path,
        WavExportOptions::resolve(&clean_project, None, None, None),
    )
    .expect("对照导出必须成功");
    let legacy = format!(
        "module:action=export-wav:outcome=ok:path={}:frames={}:sample_rate={}:channels={}:bits={}:bytes={}:source={}:digest={}",
        clean_path.display(),
        report.frames,
        report.sample_rate,
        report.channels,
        report.bits,
        report.bytes,
        report.source,
        report.digest
    );
    assert_eq!(
        clean_receipt, legacy,
        "不削波时 GUI 回执必须与 `#226` 之前的九字段形状**逐字相同**"
    );
    // 两份导出器（GUI 与直接调用）的音频逐字节相同。
    assert_eq!(
        sha_hex(&clean_path),
        sha_hex(&oracle_path),
        "GUI 路径与直接调用必须是同一份产物"
    );
    assert_eq!(
        sha_hex(&clean_path),
        CLEAN_WAV_SHA256,
        "干净夹具的字节必须与 `main` 的基线相同"
    );

    std::fs::remove_dir_all(&dir).ok();
}
