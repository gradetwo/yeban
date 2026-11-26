//! 任务单 `#229`：**GUI 的导出也报削波**（`#226` 自己登记的待办）。
//!
//! # 本单补的是哪一格
//!
//! | 层 | `#226` 之后、本单之前 |
//! | :--- | :--- |
//! | 能力（`WavExportReport`） | ✓ `clipped_samples` / `true_peak_dbfs` / `headroom_db` / `suggested_trim_db` / `loudest_source` |
//! | CLI 出口 | ✓ `--export-wav` 削波时**多打一行** |
//! | **GUI 出口** | ⛔ **`on_export_wav` 走 `module_receipt`，回执里没有削波字段** ⇒ 人在窗口里看不到 |
//! | ⭐ **本单** | ⭐ `on_export_wav` 的成功回执在**削波时多一条** `clipping_note="…"`（内容 = CLI 同一条 `clipping_note()`） |
//! | ⭐ `D-EX-1`（后续） | ⭐ 四条峰值读数（`limiter` / `limiter_reductions` / `true_peak_dbfs` / `headroom_db`）改成**恒发**，发在 `clipping_note` **之前**。<br>于是本文件的"干净臂逐字 ＝ 九字段"变成"九字段 ＋ 四读数"；`clipping_note` 仍**只在削波时**出现（那条长文案的门一位未动） |
//!
//! ## `D-EX-1` 为什么改的是"发不发"，不是"量不量"
//!
//! `#229` 把四个读数拴在"削波"这一个条件上 ⇒ **贴轨但没削平**的那一格永远看不见任何数
//! （创作轮里实测：引擎自报 `headroomDb = 5.18e-07`，GUI 回执里却一个字段都没有）。
//! 四个数本来就在那条路上算好了（`render_core` 的压样本计数 ＋ 量化前浮点母带的真峰值），
//! 本单只是把它们发出去：⛔ 不新增测量、⛔ 不改一个样本 —— 判据 ③ 的自比就是这个意思。
//!
//! # 判据（两条臂 + 一条字节臂）
//!
//! | 判据 | 钉住的事实 |
//! | :--- | :--- |
//! | ①（削波臂） | 经**真窗口的回调**（`invoke_export_wav`）导出过载工程 ⇒ 回执多一条 `clipping_note`，且它含 `clippedSamples` / `suggestedTrimDb` / 最响的源 |
//! | ②（干净臂） | 同一装配导出不削波工程 ⇒ 回执**逐字等于**"九字段 ＋ 四条恒发读数"（⛔ 没有 `clipping_note`、⛔ 没有"削波"），且四个读数**在场**、真峰值**非** `null` |
//! | ③（不改音频） | 两条臂的 GUI 产物与**同一次运行里直接调用** [`export_wav`] 的产物 `sha256` 相同（**自比**）⇒ 提示只加文字、一个字节都没动 |
//!
//! ## 判据怎么变红
//!
//! - 回执里删掉 `clipping_note`（或只在调试输出里打）⇒ ① 红；
//! - 把 `clipping_note` 无条件加进回执 ⇒ ② 红（干净臂多了一条）；
//! - 提示路径碰了样本 / 增益 / 量化 / 落盘顺序 ⇒ ③ 红（GUI 产物与直接调用的产物不再逐字节相同）；
//! - GUI 换一条**不是** `export_wav_with_sfz_file` 的导出器 ⇒ ③ 的自比不再相同 ⇒ 红。
//!
//! ⚠ ③ **不**钉绝对 `sha256`：绝对字节来自 `sin`/`libm`/FMA，**机器相关**（第 313 条），
//! 钉常量只能描述某一台机器（`#232` 的 Linux/Windows 红）。自比证明的是"GUI 路径与
//! 直接调用在这台机器上产出同一串字节"，可跨平台成立。
//!
//! ## 平台约束：本文件只有一个 `#[test]`
//!
//! `slint::platform::set_platform` 是**线程局部**的，`Tier1Window::install` 每线程只能成功
//! 一次 ⇒ 同款用法见 `tests/ui_module_actions.rs` / `tests/export_midi_ui.rs`。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的装配入口 ⇒ 其余项在本测试目标里是 dead_code。
#[allow(dead_code)]
mod live;

// ⭐ `D-EX-1`：回执形状是**共享**的第二份（`tests/common/mod.rs` 里有它的住户说明）。
mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::str::FromStr as _;

use common::wav_export_receipt;
use yeban_app::bridge::ViewState;
use yeban_app::export_wav::{WavExportOptions, export_wav};
use yeban_app::host;
use yeban_app::host::{ModulePaths, wire_module_actions_with};
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::{
    AssetHash, ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote,
    RoutingEdge, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_ui_test_port::inspect::install_testing_backend_with_event_loop;
use yeban_ui_test_port::render::report_line;

/// 把事件循环里已经排队的回写闭包跑掉（先压 `Quit`，FIFO ⇒ 回写先跑、循环后退出）。
fn drain_event_loop() {
    slint::quit_event_loop().expect("带事件循环的后端必须提供 proxy");
    slint::run_event_loop().expect("无头事件循环必须正常返回");
}

/// 经**真窗口**导出一次并返回 `module_status`（结构化回执原文）。
///
/// 用带事件循环的 testing backend（导出回写走 `upgrade_in_event_loop`，Tier-1 平台
/// 无 proxy ⇒ 必须用本后端），生产构造入口 `build_main_window_with_console_tab` +
/// `wire_module_actions_with`（装配参考 `tests/ui_module_actions.rs`）。
fn export_through_gui(
    project: &YebanProjectV1,
    target: &std::path::Path,
    label: &str,
) -> (String, bool) {
    let port = Rc::new(UndoPort::new(
        UndoSession::open(label, "yeban-app", project.clone(), 1).expect("打开撤销会话"),
    ));
    let view = ViewState::from_project(project).expect("投影");
    let scene = DemoScene::from_view(&view);
    let ui = host::build_main_window_with_console_tab(&view, &scene, 0).expect("生产窗口");
    let mut paths = ModulePaths::from_env();
    paths.export_wav = target.to_path_buf();
    let _midi = wire_module_actions_with(&ui, Some(Rc::clone(&port)), paths);

    assert_eq!(ui.get_module_status().as_str(), "", "导出前回执应为空");
    ui.invoke_export_wav();
    // ⭐ 确定性等待：join 工作线程 ⇒ upgrade_in_event_loop 已返回 ⇒ 回写闭包
    // 一定已在事件循环队列 ⇒ drain 让它在 UI 线程执行（⛔ 无 sleep/轮询/超时）。
    assert!(
        host::wait_for_pending_export(),
        "必须有一个刚结束的导出工作线程可 join"
    );
    drain_event_loop();
    (
        ui.get_module_status().to_string(),
        ui.get_module_status_succeeded(),
    )
}

// ⚠ 这里**没有** WAV 的绝对 `sha256`/`digest` 常量：绝对字节是机器相关的（第 313 条），
//   "不改音频"一律用**同一台机器上的自比**（GUI 产物 vs 直接调用同一导出器）。

/// 回执字段名闭集（只用于把"值里自带的冒号"排除在切分边界之外；口径照
/// `tests/ui_module_actions.rs` 的 `receipt_field_end`）。
/// ⭐ 本单新增 `clipping_note`；`D-EX-1` 再加**恒发**的四个峰值读数字段。
const RECEIPT_KEYS: [&str; 19] = [
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
    "audio_clips_rendered",
    "audio_clips_skipped",
    "limiter",
    "limiter_reductions",
    "true_peak_dbfs",
    "headroom_db",
    "integrated_lufs",
    "loudness_range_lu",
    "clipping_note",
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

/// 一条回执的**顶层**字段取值（引号感知，`#321`）；缺席返回 `None`。
///
/// 字段名只认**顶层**位置（`key=` 必须紧跟一个**顶层** `:`；引号内的 `:` 不算起点）：
/// 引号值跳到**第一个闭合引号**（引号内的 `channels=` 等一律不是字段名），普通值仍按
/// [`receipt_field_end`] 的"已知字段名 + `=`"边界取。引号值**保留引号**（本文件的
/// `clipping_note` 断言就依赖这一点）。
///
/// ## 引号里的 `"`（写死的契约）
///
/// 回执没有转义 ⇒ 第一个 `"` 即闭合引号；值里真带 `"` 时其后的字段取不到（`None`）。
fn field_of_opt(receipt: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=");
    let mut in_quotes = false;
    for (index, byte) in receipt.bytes().enumerate() {
        match byte {
            b'"' => in_quotes = !in_quotes,
            b':' if !in_quotes => {
                let start = index + 1;
                if receipt[start..].starts_with(&needle) {
                    let value_start = start + needle.len();
                    if receipt[value_start..].starts_with('"') {
                        let close = receipt[value_start + 1..].find('"').expect("引号必须闭合");
                        return Some(receipt[value_start..=value_start + 1 + close].to_owned());
                    }
                    let end = receipt_field_end(receipt, value_start, key).unwrap_or(receipt.len());
                    return Some(receipt[value_start..end].to_owned());
                }
            }
            _ => {}
        }
    }
    None
}

/// 取一个字段；缺席即判据失败（⛔ 不返回空串）。
fn field_of(receipt: &str, key: &str) -> String {
    field_of_opt(receipt, key).unwrap_or_else(|| panic!("回执里没有字段 `{key}`: {receipt}"))
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
            offset_samples: None,
            fade_in_ticks: None,
            fade_out_ticks: None,
            source_start_samples: None,
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
            source_lane: None,
            dest_lane: None,
        },
    );
    project.validate().expect("夹具工程必须合法");
    project
}

/// ⭐ 运行时判据（恢复自退化前的源码断言）：经**真窗口**回调导出过载/干净工程，
/// 解析结构化回执 —— 削波臂有 `clipping_note`（含 clippedSamples/suggestedTrimDb/最响的源），
/// 干净臂逐字等于 `wav_export_receipt`（无 clipping_note），两臂产物与直接调用
/// `export_wav` 的产物 sha256 自比。
///
/// 红对照：删 host.rs 的 `clipping_note` 字段 ⇒ 削波臂红；把它改成无条件出现 ⇒ 干净臂红；
/// 让 GUI 走另一条导出器 ⇒ sha256 自比红。
#[test]
fn the_gui_export_reports_clipping_only_when_the_master_clips() {
    install_testing_backend_with_event_loop();

    let dir = scratch_dir("clip");
    let clip_project = project_with_track(24.0); // +24dB ⇒ master 削波
    let clean_project = project_with_track(0.0);
    let options = WavExportOptions::resolve(&clip_project, None, None, None, None);

    // ---------- ① 削波臂：真窗口导出过载工程 ----------
    let clip_path = dir.join("clip.wav");
    let (clip_receipt, clip_ok) = export_through_gui(&clip_project, &clip_path, "gui clip");
    assert!(clip_ok, "削波工程导出必须成功: {clip_receipt}");
    let note = field_of_opt(&clip_receipt, "clipping_note")
        .expect("削波工程回执必须含 clipping_note（删该字段 ⇒ 本断言红）");
    let inner = note.trim_matches('"');
    assert!(
        inner.contains("clippedSamples"),
        "clipping_note 必须含 clippedSamples: {note}"
    );
    assert!(
        inner.contains("suggestedTrimDb"),
        "clipping_note 必须含 suggestedTrimDb: {note}"
    );
    assert!(
        inner.contains("Lead"),
        "clipping_note 必须点名最响的源 Lead: {note}"
    );
    assert_ne!(
        field_of(&clip_receipt, "true_peak_dbfs"),
        "null",
        "削波臂真峰值非 null"
    );
    assert_eq!(field_of(&clip_receipt, "limiter"), "off");

    // 与同一工程直接调用导出器对照：clipping_note 逐字相等 + 整条回执逐字相等 + sha256 自比。
    let clip_oracle = dir.join("clip-oracle.wav");
    let direct = export_wav(&clip_project, &clip_oracle, options).expect("直接导出对照");
    let direct_note = direct
        .clipping_note()
        .expect("削波报告必须有 clipping_note");
    assert_eq!(
        inner, direct_note,
        "GUI 与直接调用的 clipping_note 必须逐字相同"
    );
    let legacy = wav_export_receipt(&direct, &clip_path, false);
    assert_eq!(
        clip_receipt,
        format!("{legacy}:clipping_note={note}"),
        "削波臂整条回执必须逐字等于 第二份形状 + clipping_note"
    );
    assert_eq!(
        sha_hex(&clip_path),
        sha_hex(&clip_oracle),
        "GUI 产物与直接调用 sha256 自比"
    );

    // ---------- ② 干净臂：真窗口导出不削波工程 ----------
    let clean_path = dir.join("clean.wav");
    let (clean_receipt, clean_ok) = export_through_gui(&clean_project, &clean_path, "gui clean");
    assert!(clean_ok, "干净工程导出必须成功: {clean_receipt}");
    assert!(
        field_of_opt(&clean_receipt, "clipping_note").is_none(),
        "干净臂不得含 clipping_note（无条件加 ⇒ 本断言红）: {clean_receipt}"
    );
    assert!(
        !clean_receipt.contains("削波"),
        "干净臂回执不得出现削波字样"
    );
    let clean_options = WavExportOptions::resolve(&clean_project, None, None, None, None);
    let clean_oracle = dir.join("clean-oracle.wav");
    let clean_direct = export_wav(&clean_project, &clean_oracle, clean_options).expect("干净对照");
    assert_eq!(
        clean_receipt,
        wav_export_receipt(&clean_direct, &clean_path, false),
        "干净臂整条回执必须逐字等于 第二份形状（无 clipping_note）"
    );
    assert_ne!(
        field_of(&clean_receipt, "true_peak_dbfs"),
        "null",
        "干净臂真峰值非 null"
    );
    assert_eq!(
        sha_hex(&clean_path),
        sha_hex(&clean_oracle),
        "干净臂 GUI 产物 sha256 自比"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// `#321` 负向判据：**引号内的键 ⛔ 可被顶层取到**（本文件的取值器也必须引号感知）。
///
/// `clipping_note` 也是引号值；修后 `field_of("channels")` 只认顶层字段，`None`，
/// 而引号值仍**保留引号**返回（本文件既有约定）。
#[test]
fn a_quoted_value_does_not_expose_its_inner_keys_to_the_top_level() {
    // 纯函数判据（不装平台，可与主测试并行）：引号值里的 `key=` 不得被当成顶层字段。
    let receipt = "module:action=export-wav:outcome=ok:clipping_note=\"clippedSamples = 3, channels=2, suggestedTrimDb = -1\"";
    // 引号值整段（保留引号）可取到。
    let note = field_of_opt(receipt, "clipping_note").expect("顶层 clipping_note 必须可取");
    assert!(
        note.starts_with('"') && note.ends_with('"'),
        "引号值保留引号"
    );
    assert!(note.contains("clippedSamples"), "引号值内容完整");
    // 引号里的 `channels=` / `suggestedTrimDb=` 不是顶层字段 ⇒ 取不到。
    assert!(
        field_of_opt(receipt, "channels").is_none(),
        "引号内的 key 不得暴露为顶层字段"
    );
    assert!(
        field_of_opt(receipt, "suggestedTrimDb").is_none(),
        "引号内的 key 不得暴露为顶层字段"
    );
    // 顶层字段照常可取。
    assert_eq!(field_of(receipt, "outcome"), "ok");
}
