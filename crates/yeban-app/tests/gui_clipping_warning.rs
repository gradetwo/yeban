//! 任务单 `#230`：**把削波提示做成窗口里看得见的一行**。
//!
//! # 本单补的是哪一格
//!
//! | 层 | `#229` 之后、本单之前 |
//! | :--- | :--- |
//! | 数据通道 | ✓ `module-status` 里多了 `clipping_note="…"`（完整读数） |
//! | 窗口可见性 | ⛔ 回执行宽 264px，前缀 `module:action=export-wav:outcome=ok:` 占 216px，`clipping_note` 是最后一个字段 ⇒ 被 `elide` |
//! | ⭐ **本单** | ⭐ 右栏 `module-clipping-warning`（**独立一行**，`y = 448`、宽 264、默认空串 ⇒ `visible: false`）：削波时显示 `⚠ 母带削波：N 个样本，建议减 X dB`；不削波 / 其它模块动作 ⇒ 清空 |
//!
//! # 判据
//!
//! | # | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① | 削波导出 ⇒ `MainWindow.module-clipping-warning` **逐字**等于 `⚠ 母带削波：{clipped_samples} 个样本，建议减 {suggestedTrimDb.abs():.1} dB`，且 `module-status` 仍含完整 `clipping_note` | 证"读数进了可见文本"；⛔ 不证人眼 |
//! | ② | 不削波导出 ⇒ 该属性回到 `""`，运行时树里没有 `module-clipping-warning` | 证"不削波时不可见" |
//! | ③ | 默认帧：属性 `""` ⇒ 节点不在树里；同一窗口里把它设成非空再设回空，帧**逐字节回到默认帧** | 证"空串 = 零像素"；⛔ 不替代 Linux Golden |
//! | ④ | 削波时节点在树里：`role == "text"`、`label` 逐字、`value` == 属性、`bounds` 在右栏内、宽高非零 | `[UI-TEST-001]` + a11y 的可寻址落点 |
//! | ⑤ | 三档 `1920×1080 / 1440×900 / 1024×800`：削波时警示行的包围盒与相邻模块行**两两不相交**（1024 档打开右栏抽屉） | 证"多一行没有把 1024 档挤重叠" |
//! | ⑥ | 同一窗口里两帧差分：差异像素**全部**落在警示行包围盒内；清空后与默认帧逐字节相同 | 证"看得见"不是一句断言 |
//! | ⑦ | 两条臂的 WAV `sha256` 与**同一台机器上直接调用** [`export_wav`] 的产物相同（**自比**） | 证"提示不改音频一个字节"；⛔ 不钉绝对常量（机器相关，第 313 条） |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是**线程局部**的，`Tier1Window::install` 每个线程只能成功
//! 一次 ⇒ 本文件只有一个 `#[test]`，三档几何各在自己的 `std::thread::spawn` 里装配
//! （与 `tests/pane_no_overlap.rs` 同款）。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的装配入口 ⇒ 其余项在本测试目标里是 dead_code。
#[allow(dead_code)]
mod live;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::str::FromStr as _;

use live::{LiveWiringOptions, build_live_ui_with};
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
use yeban_ui_test_port::Rgb8Image;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::image::Rect;
use yeban_ui_test_port::inspect::tree_from_element_root;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::tree::ControlTree;

/// 警示行的语义 ID（`ui/module_actions.slint` ↔ `src/elements.rs` 两处逐字相同）。
const WARNING_ID: &str = "module-clipping-warning";

// ⚠ 这里**没有** WAV 的绝对 `sha256` 常量：绝对字节是机器相关的（第 313 条），
//   两条臂都用**同一台机器上的自比**（GUI 产物 vs 直接调用同一导出器）。

/// 默认帧对照用的**非空**文本（与宿主真实写出的那一句同一形状；仅用于"空 ⇒ 零像素"的
/// 两帧差分，不参与削波臂的逐字断言）。
const SAMPLE_WARNING: &str = "⚠ 母带削波：384000 个样本，建议减 24.0 dB";

/// 三档窗口（与 `tests/pane_no_overlap.rs` 同一组）。
const TIERS: &[(u32, u32)] = &[(1920, 1080), (1440, 900), (1024, 800)];

/// 相邻的模块行（警示行在 `y = 448`，它下面没有第二个新控件；这里取同族的 5 个近邻）。
const NEIGHBOURS: &[&str] = &[
    "module-actions-title",
    "module-midi-in-button",
    "module-midi-in-status",
    "module-last-status",
];

fn fixture_id(tail: &str) -> EntityId {
    EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8P0{tail}")).expect("夹具 ULID 必须合法")
}

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-230-{tag}-{}-{}",
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

/// 与 `tests/gui_export_clipping.rs` / `tests/export_wav_clipping.rs` **逐字同款**的最小工程。
fn project_with_track(volume_db: f32) -> YebanProjectV1 {
    let master = fixture_id("M1");
    let track_id = fixture_id("T0");
    let clip = fixture_id("K0");
    let placement = fixture_id("P0");
    let note = fixture_id("N0");
    let edge = fixture_id("E0");
    let mut project = YebanProjectV1 {
        id: fixture_id("Z1"),
        title: "gui warning fixture".to_owned(),
        bpm: 120.0,
        rng_seed: 0x5945_4241_4E00_0002,
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
    let mut notes = std::collections::BTreeMap::new();
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

/// 一次**真实**的导出（判据的对照读数；与 GUI 走同一个导出器）。
fn report_for(project: &YebanProjectV1, path: &Path) -> (u64, f32) {
    let report = export_wav(
        project,
        path,
        WavExportOptions::resolve(project, None, None, None, None),
    )
    .expect("对照导出必须成功");
    (report.clipped_samples, report.suggested_trim_db)
}

/// 期望的可见短句（读的是导出报告的两个字段，⛔ 不硬编码 384000 / 24.0）。
fn expected_warning(clipped_samples: u64, suggested_trim_db: f32) -> String {
    format!(
        "⚠ 母带削波：{clipped_samples} 个样本，建议减 {:.1} dB",
        suggested_trim_db.abs()
    )
}

fn node_of<'a>(tree: &'a ControlTree, id: &str) -> &'a yeban_ui_test_port::tree::ControlNode {
    tree.find_by_id(id)
        .unwrap_or_else(|| panic!("运行时树里必须有 `{id}`（当前 {} 个节点）", tree.len()))
}

fn bounds_of(tree: &ControlTree, id: &str) -> Rect {
    node_of(tree, id)
        .bounds
        .unwrap_or_else(|| panic!("`{id}` 必须有几何包围盒"))
}

fn tree_of(window: &yeban_app::ui::MainWindow) -> ControlTree {
    tree_from_element_root(window, None).expect("运行时控件树")
}

fn intersects(left: Rect, right: Rect) -> bool {
    let left_right = left.x + i32::try_from(left.width).expect("宽");
    let left_bottom = left.y + i32::try_from(left.height).expect("高");
    let right_right = right.x + i32::try_from(right.width).expect("宽");
    let right_bottom = right.y + i32::try_from(right.height).expect("高");
    left.x < right_right && right.x < left_right && left.y < right_bottom && right.y < left_bottom
}

/// 两帧在 `area` **之外**的不同像素数（`0` = 期望区域外逐字节相同）。
fn diff_outside(before: &Rgb8Image, after: &Rgb8Image, area: Rect) -> u64 {
    assert_eq!(
        (before.width(), before.height()),
        (after.width(), after.height()),
        "两帧必须同尺寸"
    );
    let mut outside = 0_u64;
    for y in 0..before.height() {
        for x in 0..before.width() {
            let inside = (x as i32) >= area.x
                && (x as i32) < area.x + i32::try_from(area.width).expect("宽")
                && (y as i32) >= area.y
                && (y as i32) < area.y + i32::try_from(area.height).expect("高");
            if !inside && before.pixel(x, y) != after.pixel(x, y) {
                outside += 1;
            }
        }
    }
    outside
}

/// 两帧在 `area` **之内**的不同像素数 + 这些像素的右沿（`None` = 区域内零差异）。
///
/// 右沿用于"文本没有被裁在右边界上"的读数：左对齐文本若真的被裁，最后一个字形会
/// 顶到 `area` 的右沿；正常放得下时最右墨迹应当离右沿还有几个像素。
fn diff_inside(before: &Rgb8Image, after: &Rgb8Image, area: Rect) -> (u64, Option<u32>) {
    let mut inside = 0_u64;
    let mut rightmost = None;
    for y in 0..before.height() {
        for x in 0..before.width() {
            let within = (x as i32) >= area.x
                && (x as i32) < area.x + i32::try_from(area.width).expect("宽")
                && (y as i32) >= area.y
                && (y as i32) < area.y + i32::try_from(area.height).expect("高");
            if within && before.pixel(x, y) != after.pixel(x, y) {
                inside += 1;
                rightmost = Some(rightmost.map_or(x, |current: u32| current.max(x)));
            }
        }
    }
    (inside, rightmost)
}

/// `MainWindow` 上的削波警示属性（唯一写者 = `host::wire_module_actions`）。
fn warning_of(window: &yeban_app::ui::MainWindow) -> String {
    window.get_module_clipping_warning().to_string()
}

/// 找一条窗口里**看得见**的警示行：属性非空、节点在树里、几何在 `ai-rail` 内。
fn visible_warning(tree: &ControlTree) -> (Rect, String) {
    let rail = bounds_of(tree, "ai-rail");
    let warning = bounds_of(tree, WARNING_ID);
    assert!(
        warning.x >= rail.x
            && warning.y >= rail.y
            && warning.x + i32::try_from(warning.width).expect("宽")
                <= rail.x + i32::try_from(rail.width).expect("宽")
            && warning.y + i32::try_from(warning.height).expect("高")
                <= rail.y + i32::try_from(rail.height).expect("高"),
        "警示行必须完整落在右栏内（warning {warning:?} / rail {rail:?}）"
    );
    let node = node_of(tree, WARNING_ID);
    (warning, node.value.clone().unwrap_or_default())
}

/// 三档里"削波时警示行不与他人抢同一格"（1024 档先打开右栏抽屉）。
fn check_warning_geometry(width: u32, height: u32) {
    let project = yeban_model::samples::demo_project();
    let view = ViewState::from_project(&project).expect("演示工程必须能投影");
    let scene = DemoScene::from_view(&view);
    // 与 `tests/pane_no_overlap.rs` 的 `live_window` 同款：窗口尺寸由 `LivePort` 给，
    // 生产装配入口仍是 `host::build_main_window_with_console_tab`。
    let port = LivePort::new(Size::new(width, height), Permission::ReadOnly, None, || {
        host::build_main_window_with_console_tab(&view, &scene, 0)
    })
    .expect("Tier-1 平台 + 活窗口");
    if port.ui().get_compact() {
        port.ui().set_ai_drawer_open(true);
    }
    port.ui().set_module_clipping_warning(SAMPLE_WARNING.into());
    let tree = tree_of(port.ui());
    let (warning, value) = visible_warning(&tree);
    report_line(&format!(
        "[#230 几何 {width}x{height}] module-clipping-warning bounds={warning:?} value={value:?}；右栏={:?}",
        bounds_of(&tree, "ai-rail")
    ));
    assert_eq!(
        value, SAMPLE_WARNING,
        "运行时 `accessible-value` 必须逐字等于属性"
    );
    for id in NEIGHBOURS {
        if let Some(node) = tree.find_by_id(id) {
            let other = node.bounds.expect("相邻模块行必须有几何");
            assert!(
                !intersects(warning, other),
                "[#230 {width}x{height}] `{WARNING_ID}` {warning:?} 与 `{id}` {other:?} 相交"
            );
            report_line(&format!(
                "[#230 几何 {width}x{height}] {WARNING_ID} ⟂ {id}: 0 相交（{other:?}）"
            ));
        }
    }
}

#[test]
fn the_clipping_warning_is_a_visible_row_only_when_the_master_clips() {
    // =====================================================================
    // 三档几何：削波时警示行与相邻模块行两两不相交（1024 档打开抽屉）
    // =====================================================================
    let geometry: Vec<()> = TIERS
        .iter()
        .map(|&(width, height)| {
            std::thread::spawn(move || check_warning_geometry(width, height))
                .join()
                .expect("几何子线程不得 panic")
        })
        .collect();
    assert_eq!(geometry.len(), TIERS.len());

    // =====================================================================
    // 主臂：真窗口 + 撤销端口 + 两条臂（削波 / 干净）
    // =====================================================================
    std::thread::spawn(move || {
        let dir = scratch_dir("main");
        let clipping_project = project_with_track(24.0);
        let clean_project = project_with_track(0.0);

        let clip_path = dir.join("clip.wav");
        let clean_path = dir.join("clean.wav");
        let clip_oracle = dir.join("clip-oracle.wav");
        let clean_oracle = dir.join("clean-oracle.wav");

        let port = Rc::new(UndoPort::new(
            UndoSession::open(
                "<判据 #230: 削波>",
                "yeban-app",
                clipping_project.clone(),
                1_760_000_000_000,
            )
            .expect("打开撤销会话"),
        ));
        let live_ui = build_live_ui_with(
            &clipping_project,
            &LiveWiringOptions {
                permission: Permission::Interactive,
                undo: Some(Rc::clone(&port)),
                ..LiveWiringOptions::default()
            },
        )
        .expect("Tier-1 平台 + 活窗口");
        let window = slint::ComponentHandle::clone_strong(live_ui.ui());
        let mut paths = ModulePaths::from_env();
        paths.export_wav = clip_path.clone();
        wire_module_actions_with(&window, Some(Rc::clone(&port)), paths.clone());

        // ---- ① 默认：属性空 + 节点不在树里 + 默认帧 ----
        assert_eq!(warning_of(&window), "", "按之前警示行必须是空的");
        let default_tree = tree_of(&window);
        assert!(
            default_tree.find_by_id(WARNING_ID).is_none(),
            "默认帧（空串）里 `{WARNING_ID}` 不该在运行时树里"
        );
        let default_frame = live_ui.capture().expect("默认帧");

        // ---- ② 同一窗口里"空 ⇒ 零像素"的两帧差分 ----
        window.set_module_clipping_warning(SAMPLE_WARNING.into());
        let shown_tree = tree_of(&window);
        let (warning_box, shown_value) = visible_warning(&shown_tree);
        let node = node_of(&shown_tree, WARNING_ID);
        assert_eq!(node.role.as_str(), "text", "警示行的 `accessible-role`");        assert_eq!(
            node.label, "母带削波警示（削波样本数 / 建议减多少 dB）",
            "警示行的 `accessible-label` 必须与 `src/elements.rs` 的注册表逐字相同"
        );
        assert_eq!(shown_value, SAMPLE_WARNING, "`accessible-value` = 可见文本");
        assert!(
            warning_box.width > 0 && warning_box.height > 0,
            "可见的警示行必须有实几何: {warning_box:?}"
        );
        let shown_frame = live_ui.capture().expect("可见帧");
        let (inside, rightmost) = diff_inside(&default_frame, &shown_frame, warning_box);
        let outside = diff_outside(&default_frame, &shown_frame, warning_box);
        let right_edge = warning_box.x + i32::try_from(warning_box.width).expect("宽");
        let rightmost = rightmost.expect("可见帧里警示行必须有差异像素");
        report_line(&format!(
            "[#230 可见] 空 ⇒ 非空: 行 {warning_box:?}；区域内差异 {inside} px、右沿 x={rightmost} / 行右沿 {right_edge}；区域外差异 {outside} px"
        ));
        assert!(inside > 0, "非空文本必须在它的行里产生像素");
        assert_eq!(outside, 0, "非空文本只许改它自己那一行");
        assert!(
            i32::try_from(rightmost).expect("x") < right_edge - 2,
            "文本右沿 {rightmost} 必须离行右沿 {right_edge} 至少 3px（否则有被裁掉的风险）"
        );

        window.set_module_clipping_warning("".into());
        let cleared_tree = tree_of(&window);
        assert!(
            cleared_tree.find_by_id(WARNING_ID).is_none(),
            "清空之后警示行必须离开运行时树"
        );
        let cleared_frame = live_ui.capture().expect("清空帧");
        assert_eq!(
            cleared_frame.pixels(),
            default_frame.pixels(),
            "空串 ⇒ 逐字节回到默认帧（警示行不占布局、不画像素）"
        );
        report_line(&format!(
            "[#230 默认帧] 1920x1080 默认帧 {} 字节；设成非空再设回空之后逐字节相同",
            default_frame.pixels().len()
        ));

        // ---- ③ 削波臂：真导出 ⇒ 属性逐字 + 树里可寻址 + 完整字段仍在 ----
        window.invoke_export_wav();
        let clipping_warning = warning_of(&window);
        let receipt = window.get_module_status().to_string();
        assert!(
            window.get_module_status_succeeded(),
            "削波导出必须成功: {receipt}"
        );
        let (clipped, trim) = report_for(&clipping_project, &clip_oracle);
        let expected = expected_warning(clipped, trim);
        assert_eq!(
            clipping_warning, expected,
            "警示行必须是导出报告两个字段拼出的**逐字**短句"
        );
        assert!(
            receipt.contains("clipping_note=\""),
            "完整读数必须仍在回执里（#229 的数据通道不动）: {receipt}"
        );
        assert!(
            receipt.contains("clippedSamples") && receipt.contains("suggestedTrimDb"),
            "回执必须仍携带完整削波字段: {receipt}"
        );
        report_line(&format!(
            "[#230 削波] warning={clipping_warning:?}；回执长度 {} 字节；回执含 clipping_note={}",
            receipt.len(),
            receipt.contains("clipping_note=\"")
        ));
        let clip_tree = tree_of(&window);
        let (clip_box, clip_value) = visible_warning(&clip_tree);
        assert_eq!(clip_value, expected, "削波时运行时 value = 属性");
        let receipt_box = bounds_of(&clip_tree, "module-last-status");
        let clip_frame = live_ui.capture().expect("削波导出帧");
        let outside = diff_outside(&default_frame, &clip_frame, clip_box)
            .saturating_sub(diff_inside(&default_frame, &clip_frame, receipt_box).0);
        report_line(&format!(
            "[#230 削波] 警示行 {clip_box:?} / 回执行 {receipt_box:?}；默认帧 → 削波帧在警示行外、回执行外的差异 {outside} px"
        ));
        assert!(
            diff_inside(&default_frame, &clip_frame, clip_box).0 > 0,
            "削波之后警示行必须真的画出像素"
        );
        assert_eq!(
            outside, 0,
            "默认帧 → 削波帧只许改警示行与通用回执行两块"
        );
        // 自比（⛔ 不钉绝对常量）：GUI 产物必须与同一次运行里直接调用的产物逐字节相同。
        assert_eq!(
            sha_hex(&clip_path),
            sha_hex(&clip_oracle),
            "提示不得改动音频字节：GUI 路径与直接调用必须逐字节相同"
        );

        // ---- ④ 干净臂：同一窗口换不削波工程 ⇒ 属性清空 + 节点消失 ----
        let clean_port = Rc::new(UndoPort::new(
            UndoSession::open(
                "<判据 #230: 干净>",
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
        assert!(
            window.get_module_status_succeeded(),
            "干净导出必须成功: {clean_receipt}"
        );
        assert_eq!(
            warning_of(&window),
            "",
            "不削波 ⇒ 警示行必须清空（上一条工程的读数不得留着）"
        );
        let clean_tree = tree_of(&window);
        assert!(
            clean_tree.find_by_id(WARNING_ID).is_none(),
            "不削波 ⇒ 警示行必须离开运行时树"
        );
        assert!(
            !clean_receipt.contains("clipping_note") && !clean_receipt.contains("削波"),
            "干净回执不得多任何字段: {clean_receipt}"
        );
        let (clean_clipped, _) = report_for(&clean_project, &clean_oracle);
        assert_eq!(clean_clipped, 0, "干净夹具必须真的不削波");
        // 自比（⛔ 不钉绝对常量）：干净臂同样与直接调用逐字节相同。
        assert_eq!(
            sha_hex(&clean_path),
            sha_hex(&clean_oracle),
            "干净臂的音频字节必须与直接调用逐字节相同"
        );
        report_line(&format!(
            "[#230 干净] warning={:?}；树里 {} 节点；sha256 两条臂 = GUI 与直接调用相同（自比）",
            warning_of(&window),
            clean_tree.len()
        ));

        // 临时目录：判据不把产物留在仓库里。
        std::fs::remove_dir_all(&dir).ok();
    })
    .join()
    .expect("主臂子线程不得 panic");
}
