//! 任务单 `#235`：**1024 档抽屉实例也要转发模块回执**（`#230` 登记的既有缺口）。
//!
//! # 缺口（`#230` 逐字登记）
//!
//! `AiRail` 在 `ui/app.slint` 里有**两个实例**：常驻栏（宽档）与 compact 抽屉
//! （`visible: root.compact && root.ai-drawer-open`）。`#230` 只把
//! `module-clipping-warning` 接到了抽屉实例上，**没有**转发 `module-status` /
//! `module-midi-in-status` 及其成功位 ⇒ 1024 档在抽屉里按「导出 WAV」时，
//! 宿主把回执写进了 `MainWindow.module-status`，但抽屉里的回执行读的是
//! `AiRail.module-status` 的**默认空串** ⇒ 人眼看到空行。
//!
//! # 本单补的是哪一格
//!
//! | 层 | `#230` 之后、本单之前 |
//! | :--- | :--- |
//! | 常驻栏 | ✓ 四条回执属性已转发 |
//! | 抽屉 | ⛔ `module-status` / `module-status-succeeded` / `module-midi-in-status` / `module-midi-in-status-succeeded` 未转发 |
//! | 抽屉警示行 | ✓ `module-clipping-warning` 已转发（`#230`） |
//! | ⭐ 本单 | ⭐ 抽屉也转发四条 ⇒ 抽屉里看到完整回执；⭐ 常驻栏一位不动 |
//!
//! # 判据
//!
//! | # | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① | 1024×800 打开抽屉（生产装配）⇒ 按「导出 WAV」⇒ `MainWindow.module-status` 非空；抽屉的 `module-last-status` 行内像素差异 > 0、行外 `0` px ⇒ **回执行真的画出了那一串** | 证"抽屉回执行不再空"；⛔ 不证人眼 |
//! | ② | 同一份夹具在 1920×1080 常驻栏按「导出 WAV」⇒ 回执行行内像素差异 > 0、行外 `0` px；整串回执逐字打印，注入前 / 注入后**逐字相同** | 证"常驻栏行为一位不变" |
//! | ③ | 抽屉里削波导出 ⇒ `module-clipping-warning` 节点在树里、`value` = 导出报告两个字段拼出的**逐字**短句；与相邻模块行两两不相交 | 证 `#230` 不退化 |
//! | ④ | 抽屉开关前后两帧差分：空回执的抽屉帧与导出后的抽屉帧，差异**全部**落在回执行包围盒内（区域外 `0` px）；默认（抽屉关）帧开关前后 `sha256` 相同 | 证"看得见"不是一句断言；⛔ 不替代 Linux Golden |
//! | ⑤ | 1920 默认帧 / 1920 compact 帧（抽屉关）/ 1024 默认帧（抽屉关）逐字节 `sha256` 在注入前后相同 | 第 315 条：先算两档基准帧 |
//!
//! ⚠ 读法口径：`module-last-status` **没有** `accessible-value`（`.slint` 只把可见 `text`
//! 绑到 `AiRail.status`，无障碍名是静态的），因此回执行"非空"的机械读数是**像素**，
//! ⛔ 不是控件树的 `value`（那条恒为空串，对**两个实例**都一样）。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是**线程局部**的，`Tier1Window::install` 每个线程只能成功
//! 一次 ⇒ 两档几何各在自己的 `std::thread::spawn` 里装配（与 `tests/gui_clipping_warning.rs`
//! / `tests/pane_no_overlap.rs` 同款）。本文件只有一个 `#[test]`。
//!
//! ## 边界
//!
//! ⛔ 没有声卡、没有人耳；⛔ 不证 Linux Golden（本机没有 `tests/golden/macos/**`，
//! 平台无基准时判据只打印「未被判定」）。⭐ 第 315 条的两档基准帧由**同一判据**用
//! Tier-1 软件光栅化逐字节量出（`sha256` 打印在 stdout），⛔ 不是"应该不变"的推断。

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::str::FromStr as _;

use yeban_app::bridge::ViewState;
use yeban_app::export_wav::{WavExportOptions, export_wav};
use yeban_app::host;
use yeban_app::host::{ModulePaths, wire_module_actions_with};
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
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
use yeban_ui_test_port::tree::{ControlNode, ControlTree};

/// 抽屉档（与 `tests/pane_no_overlap.rs` 的第三档同尺寸）。
const DRAWER_SIZE: (u32, u32) = (1024, 800);
/// 常驻栏档（Linux Golden 的尺寸）。
const RESIDENT_SIZE: (u32, u32) = (1920, 1080);

/// 回执行的语义 ID（`ui/module_actions.slint` ↔ `src/elements.rs` 两处逐字相同）。
const RECEIPT_ID: &str = "module-last-status";
/// 硬件 MIDI 输入专属结果行。
const MIDI_ID: &str = "module-midi-in-status";
/// `#230` 的削波警示行（本单不许它退化）。
const WARNING_ID: &str = "module-clipping-warning";

/// 削波警示行相邻的模块行（`#230` 判据同款清单）。
const NEIGHBOURS: &[&str] = &[
    "module-actions-title",
    "module-midi-in-button",
    "module-midi-in-status",
    "module-last-status",
];

fn fixture_id(tail: &str) -> EntityId {
    EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8P0{tail}")).expect("夹具 ULID 必须合法")
}

/// 固定路径：两次运行（注入前 / 注入后）的整串回执必须可逐字比对（路径也是回执字段）。
fn fixed_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yeban-235-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建固定临时目录");
    dir
}

/// 与 `tests/gui_clipping_warning.rs` / `tests/gui_export_clipping.rs` **逐字同款**的最小工程。
fn project_with_track(volume_db: f32) -> YebanProjectV1 {
    let master = fixture_id("M1");
    let track_id = fixture_id("T0");
    let clip = fixture_id("K0");
    let placement = fixture_id("P0");
    let note = fixture_id("N0");
    let edge = fixture_id("E0");
    let mut project = YebanProjectV1 {
        id: fixture_id("Z1"),
        title: "drawer receipt fixture".to_owned(),
        bpm: 120.0,
        rng_seed: 0x5945_4241_4E00_0235,
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

/// 一次**真实**的导出（警示行期望值的对照读数；与 GUI 走同一个导出器）。
fn report_for(project: &YebanProjectV1, path: &Path) -> (u64, f32) {
    let report = export_wav(
        project,
        path,
        WavExportOptions::resolve(project, None, None, None),
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

/// 生产构造入口装配的一档窗口 + 已接线的四模块入口。
struct Live {
    port: LivePort<MainWindow>,
    undo: Rc<UndoPort>,
    midi: host::MidiInState,
}

fn open_live(project: &YebanProjectV1, size: (u32, u32), export_wav: PathBuf) -> Live {
    let view = ViewState::from_project(project).expect("夹具工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let port = LivePort::new(
        Size::new(size.0, size.1),
        Permission::Interactive,
        None,
        || host::build_main_window_with_console_tab(&view, &scene, 0),
    )
    .expect("Tier-1 平台 + 生产窗口构造入口");
    let undo = Rc::new(UndoPort::new(
        UndoSession::open(
            "<判据 #235>",
            "yeban-app",
            project.clone(),
            1_760_000_000_235,
        )
        .expect("打开撤销会话"),
    ));
    let mut paths = ModulePaths::from_env();
    paths.export_wav = export_wav;
    paths.midi_in_port = "0".to_owned();
    paths.midi_in_seconds = 0.05;
    let midi = wire_module_actions_with(port.ui(), Some(Rc::clone(&undo)), paths);
    Live { port, undo, midi }
}

fn tree_of(window: &MainWindow) -> ControlTree {
    tree_from_element_root(window, None).expect("运行时控件树")
}

fn node_of<'a>(tree: &'a ControlTree, id: &str) -> &'a ControlNode {
    tree.find_by_id(id)
        .unwrap_or_else(|| panic!("运行时树里必须有 `{id}`（当前 {} 个节点）", tree.len()))
}

fn bounds_of(tree: &ControlTree, id: &str) -> Rect {
    node_of(tree, id)
        .bounds
        .unwrap_or_else(|| panic!("`{id}` 必须有几何包围盒"))
}

fn value_of(tree: &ControlTree, id: &str) -> String {
    node_of(tree, id).value.clone().unwrap_or_default()
}

fn intersects(left: Rect, right: Rect) -> bool {
    let left_right = left.x + i32::try_from(left.width).expect("宽");
    let left_bottom = left.y + i32::try_from(left.height).expect("高");
    let right_right = right.x + i32::try_from(right.width).expect("宽");
    let right_bottom = right.y + i32::try_from(right.height).expect("高");
    left.x < right_right && right.x < left_right && left.y < right_bottom && right.y < left_bottom
}

/// 一帧的逐字节指纹（RGB8 原始像素；与 Golden 的判据同一口径的原始读数）。
fn frame_sha(image: &Rgb8Image) -> String {
    AssetHash::of_bytes(image.pixels()).as_str().to_owned()
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

fn inside_rect(inner: Rect, outer: Rect) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && inner.x + i32::try_from(inner.width).expect("宽")
            <= outer.x + i32::try_from(outer.width).expect("宽")
        && inner.y + i32::try_from(inner.height).expect("高")
            <= outer.y + i32::try_from(outer.height).expect("高")
}

// ===========================================================================
// 常驻栏臂（1920×1080）：改前 / 改后必须逐字相同
// ===========================================================================
fn resident_arm() -> Result<(), String> {
    let dir = fixed_dir("resident");
    let project = project_with_track(0.0);
    let export = dir.join("resident.wav");
    let live = open_live(&project, RESIDENT_SIZE, export);

    let window = live.port.ui();
    let compact = window.get_compact();
    report_line(&format!(
        "[#235 常驻栏 1920x1080] compact={compact}（必须 false，常驻栏在场）"
    ));

    // ---- 第 315 条：1920 默认帧（Arrangement / 全展开） ----
    let default_frame = live
        .port
        .window()
        .capture()
        .map_err(|err| err.to_string())?;
    report_line(&format!(
        "[#235 第315条 1920x1080 默认帧] {}x{} sha256={} 字节={}",
        default_frame.width(),
        default_frame.height(),
        frame_sha(&default_frame),
        default_frame.pixels().len()
    ));

    // ---- 第 315 条：1920 compact 帧（= compact Golden 的场景；抽屉默认关） ----
    window.set_compact(true);
    let compact_frame = live
        .port
        .window()
        .capture()
        .map_err(|err| err.to_string())?;
    report_line(&format!(
        "[#235 第315条 1920x1080 compact 帧（抽屉关）] sha256={}",
        frame_sha(&compact_frame)
    ));
    window.set_compact(false);

    let tree = tree_of(window);
    let rails = tree.ids().filter(|id| *id == "ai-rail").count();
    report_line(&format!(
        "[#235 常驻栏 1920x1080] 运行时树里 `ai-rail` 实例数={rails}（常驻栏一个、抽屉不在场）"
    ));

    // ---- 按「导出 WAV」：回执行必须画出宿主写的那一串 ----
    //
    // 回执行没有 `accessible-value`（`module_actions.slint` 只把可见 `text` 绑到
    // `AiRail.status`，无障碍名是静态的）⇒ 判据读**像素**：导出前后同一实例的
    // 帧差必须**只**落在回执行那一行里，且行内差异 > 0（= 真的画出字了）。
    let pre_export = live
        .port
        .window()
        .capture()
        .map_err(|err| err.to_string())?;
    window.invoke_export_wav();
    let succeeded = window.get_module_status_succeeded();
    let receipt = window.get_module_status().to_string();
    let post_export = live
        .port
        .window()
        .capture()
        .map_err(|err| err.to_string())?;
    let tree = tree_of(window);
    let row_box = bounds_of(&tree, RECEIPT_ID);
    let (inside, rightmost) = diff_inside(&pre_export, &post_export, row_box);
    let outside = diff_outside(&pre_export, &post_export, row_box);
    let right_edge = row_box.x + i32::try_from(row_box.width).expect("宽");
    report_line(&format!(
        "[#235 常驻栏 1920x1080] 导出成功={succeeded} 回执行={row_box:?} 回执长度={}",
        receipt.len()
    ));
    report_line(&format!(
        "[#235 常驻栏帧差] 导出前 → 导出后：行内差异 {inside} px（右沿 x={} / 行右沿 {right_edge}）；行外差异 {outside} px",
        rightmost.map_or_else(|| "无".to_owned(), |x| x.to_string())
    ));
    report_line(&format!("[#235 常驻栏回执逐字] {receipt}"));

    if !succeeded {
        return Err(format!("常驻栏导出必须成功: {receipt}"));
    }
    if receipt.is_empty() {
        return Err("常驻栏导出后 `MainWindow.module-status` 不得为空".to_owned());
    }
    if inside == 0 || outside != 0 {
        return Err(format!(
            "常驻栏回执行必须在导出后画出文本且只改自己那一行：行内 {inside} px / 行外 {outside} px"
        ));
    }
    if rails != 1 {
        return Err(format!("1920 档常驻 `ai-rail` 必须恰好 1 个，实测 {rails}"));
    }
    Ok(())
}

// ===========================================================================
// 抽屉臂（1024×800）：本单修的那一格
// ===========================================================================
fn drawer_arm() -> Result<(), String> {
    let dir = fixed_dir("drawer");
    let project = project_with_track(0.0);
    let export = dir.join("drawer.wav");
    let live = open_live(&project, DRAWER_SIZE, export);

    let window = live.port.ui();
    let compact = window.get_compact();
    report_line(&format!(
        "[#235 抽屉 1024x800] 生产装配读数 compact={compact}（1024 < 1180 ⇒ 必须 true）"
    ));

    // ---- 第 315 条：1024 默认帧（抽屉默认关 ⇒ 不应画任何抽屉像素） ----
    let closed_frame = live
        .port
        .window()
        .capture()
        .map_err(|err| err.to_string())?;
    report_line(&format!(
        "[#235 第315条 1024x800 默认帧（抽屉关）] sha256={} 字节={}",
        frame_sha(&closed_frame),
        closed_frame.pixels().len()
    ));

    // ---- 打开抽屉：空回执 ----
    window.set_ai_drawer_open(true);
    let open_frame = live
        .port
        .window()
        .capture()
        .map_err(|err| err.to_string())?;
    let tree = tree_of(window);
    let rail = bounds_of(&tree, "ai-rail");
    let empty_row_box = bounds_of(&tree, RECEIPT_ID);
    let empty_row_value = value_of(&tree, RECEIPT_ID);
    report_line(&format!(
        "[#235 抽屉 1024x800] 打开抽屉（空回执）：ai-rail={rail:?} 回执行={empty_row_box:?} \
         行值={empty_row_value:?} 帧 sha256={}",
        frame_sha(&open_frame)
    ));

    // ---- 判据 ①：抽屉里按「导出 WAV」⇒ 回执行画出非空回执 ----
    window.invoke_export_wav();
    let succeeded = window.get_module_status_succeeded();
    let receipt = window.get_module_status().to_string();
    let tree = tree_of(window);
    let row_box = bounds_of(&tree, RECEIPT_ID);
    let row_inside_rail = inside_rect(row_box, rail);
    let after_frame = live
        .port
        .window()
        .capture()
        .map_err(|err| err.to_string())?;
    let (inside, rightmost) = diff_inside(&open_frame, &after_frame, row_box);
    let outside = diff_outside(&open_frame, &after_frame, row_box);
    let right_edge = row_box.x + i32::try_from(row_box.width).expect("宽");
    report_line(&format!(
        "[#235 抽屉导出] 成功={succeeded} 回执长度={} 回执行={row_box:?} 行在抽屉内={row_inside_rail}",
        receipt.len()
    ));
    report_line(&format!("[#235 抽屉回执逐字] {receipt}"));
    report_line(&format!(
        "[#235 抽屉导出帧差] 空抽屉帧 → 导出后帧：区域内差异 {inside} px（右沿 x={} / 行右沿 {right_edge}）；区域外差异 {outside} px",
        rightmost.map_or_else(|| "无".to_owned(), |x| x.to_string())
    ));

    // ---- 判据 ① 之硬件 MIDI 专属行：中间态即可判（pending 是同步写下的稳定窗口） ----
    window.invoke_midi_in();
    let midi_receipt = window.get_module_midi_in_status().to_string();
    let midi_frame = live
        .port
        .window()
        .capture()
        .map_err(|err| err.to_string())?;
    let tree = tree_of(window);
    let midi_box = bounds_of(&tree, MIDI_ID);
    // 「MIDI 输入」同时改写通用回执（同一串字节）⇒ 两行都会变；两行之外必须 0 差异。
    let (midi_inside, _) = diff_inside(&after_frame, &midi_frame, midi_box);
    let (receipt_inside, _) = diff_inside(&after_frame, &midi_frame, row_box);
    let mut midi_outside = 0_u64;
    for y in 0..midi_frame.height() {
        for x in 0..midi_frame.width() {
            let in_box = |area: Rect| {
                (x as i32) >= area.x
                    && (x as i32) < area.x + i32::try_from(area.width).expect("宽")
                    && (y as i32) >= area.y
                    && (y as i32) < area.y + i32::try_from(area.height).expect("高")
            };
            if !in_box(midi_box)
                && !in_box(row_box)
                && after_frame.pixel(x, y) != midi_frame.pixel(x, y)
            {
                midi_outside += 1;
            }
        }
    }
    report_line(&format!(
        "[#235 抽屉 MIDI 输入] 专属行={midi_box:?} 根属性长度={}；专属行内差异 {midi_inside} px / 通用行内差异 {receipt_inside} px / 两行外差异 {midi_outside} px",
        midi_receipt.len()
    ));
    report_line(&format!("[#235 抽屉 MIDI 回执逐字] {midi_receipt}"));
    // 排空（本判据只用到中间态，但仍把工作线程 join 掉，⛔ 不给下一个臂留后台线程）。
    let joined = host::wait_for_pending_midi_in(&live.midi);
    let applied = host::apply_pending_midi_in(window, &live.midi, Some(&*live.undo));
    report_line(&format!(
        "[#235 抽屉 MIDI 输入] join 到工作线程={joined} 落界面={applied}"
    ));

    // ---- 判据 ③：抽屉里削波警示行仍可见（`#230` 不退化） ----
    let clip_project = project_with_track(24.0);
    let clip_path = dir.join("drawer-clip.wav");
    let clip_oracle = dir.join("drawer-clip-oracle.wav");
    let clip_port = Rc::new(UndoPort::new(
        UndoSession::open(
            "<判据 #235: 抽屉削波>",
            "yeban-app",
            clip_project.clone(),
            1_760_000_000_236,
        )
        .expect("打开撤销会话"),
    ));
    let mut clip_paths = ModulePaths::from_env();
    clip_paths.export_wav = clip_path.clone();
    clip_paths.midi_in_port = "0".to_owned();
    clip_paths.midi_in_seconds = 0.05;
    wire_module_actions_with(window, Some(Rc::clone(&clip_port)), clip_paths);
    window.invoke_export_wav();
    let clip_succeeded = window.get_module_status_succeeded();
    let clip_receipt = window.get_module_status().to_string();
    let (clipped, trim) = report_for(&clip_project, &clip_oracle);
    let expected = expected_warning(clipped, trim);
    let tree = tree_of(window);
    let warning_box = bounds_of(&tree, WARNING_ID);
    let warning_value = value_of(&tree, WARNING_ID);
    report_line(&format!(
        "[#235 抽屉削波警示] 导出成功={clip_succeeded} 警示行={warning_box:?} 行值={warning_value:?} 期望={expected:?} 逐字相等={}",
        warning_value == expected
    ));
    report_line(&format!(
        "[#235 抽屉削波] 回执含 clipping_note={} 长度={}",
        clip_receipt.contains("clipping_note=\""),
        clip_receipt.len()
    ));
    let mut overlaps = Vec::new();
    for id in NEIGHBOURS {
        if let Some(node) = tree.find_by_id(id) {
            let other = node.bounds.expect("相邻模块行必须有几何");
            if intersects(warning_box, other) {
                overlaps.push(format!("{id} {other:?}"));
            }
            report_line(&format!(
                "[#235 抽屉削波警示] {WARNING_ID} {warning_box:?} ⟂ {id} {other:?}: {}",
                if intersects(warning_box, other) {
                    "相交"
                } else {
                    "0 相交"
                }
            ));
        }
    }

    // ---- 断言集中在这里：先把所有读数打出来，再判红 ----
    if !compact {
        return Err("1024×800 的生产装配必须 compact=true".to_owned());
    }
    if !empty_row_value.is_empty() {
        return Err(format!("空回执时抽屉回执行必须为空：{empty_row_value:?}"));
    }
    if !succeeded {
        return Err(format!("抽屉导出必须成功: {receipt}"));
    }
    if receipt.is_empty() {
        return Err("抽屉导出后 `MainWindow.module-status` 不得为空".to_owned());
    }
    if !row_inside_rail {
        return Err(format!(
            "抽屉回执行必须完整落在抽屉内：行 {row_box:?} / 抽屉 {rail:?}"
        ));
    }
    if inside == 0 || outside != 0 {
        return Err(format!(
            "抽屉回执只许改它自己那一行且必须画出文本：区域内 {inside} px / 区域外 {outside} px"
        ));
    }
    if midi_receipt.is_empty() || !midi_receipt.contains("action=midi-in") {
        return Err(format!(
            "抽屉 MIDI 专属行必须收到结构化回执: {midi_receipt:?}"
        ));
    }
    if midi_inside == 0 || receipt_inside == 0 || midi_outside != 0 {
        return Err(format!(
            "抽屉 MIDI 回执必须画在两条状态行里：专属行 {midi_inside} px / 通用行 {receipt_inside} px / 两行外 {midi_outside} px"
        ));
    }
    if warning_value != expected {
        return Err(format!(
            "抽屉削波警示行必须可见：行={warning_value:?} 期望={expected:?}"
        ));
    }
    if !clip_succeeded {
        return Err(format!("抽屉削波导出必须成功: {clip_receipt}"));
    }
    if !clip_receipt.contains("clipping_note=\"") {
        return Err(format!("完整削波读数必须仍在回执里: {clip_receipt}"));
    }
    if !overlaps.is_empty() {
        return Err(format!("削波警示行与相邻模块行相交: {overlaps:?}"));
    }
    Ok(())
}

/// 一次失败的可读形状（线程 panic 也收敛成字符串，⛔ 不让第一个线程的 panic 吞掉第二个的读数）。
fn failure_of(label: &str, joined: std::thread::Result<Result<(), String>>) -> Option<String> {
    match joined {
        Ok(Ok(())) => None,
        Ok(Err(message)) => Some(format!("{label}: {message}")),
        Err(payload) => Some(format!(
            "{label}: panic = {}",
            payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<非字符串 panic>".to_owned())
        )),
    }
}

#[test]
fn the_compact_drawer_shows_the_module_receipt_like_the_resident_rail() {
    // 两个线程各自装一次 Tier-1 平台（线程局部）⇒ 先把两档都跑完，再统一判红。
    let resident = std::thread::spawn(resident_arm);
    let drawer = std::thread::spawn(drawer_arm);

    let resident = resident.join();
    let drawer = drawer.join();

    let mut failures = Vec::new();
    failures.extend(failure_of("常驻栏", resident));
    failures.extend(failure_of("抽屉", drawer));
    assert!(
        failures.is_empty(),
        "[#235] 抽屉实例必须转发模块回执、常驻栏一位不变：\n{}",
        failures.join("\n")
    );
}
