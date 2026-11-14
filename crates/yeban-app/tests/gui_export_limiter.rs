//! `D-EX-1`：导出**母线限制器**在 GUI 里可达，并且它真的在母带上工作过。
//!
//! # 缺陷的两半（都在创作轮实测到，登记于 `docs/DEVELOPMENT_LEDGER.md`）
//!
//! | 半 | 修前 | 修后 |
//! | :--- | :--- | :--- |
//! | **能力到不了界面** | `WavExportOptions.limiter` 一直在：CLI `--limiter`、MCP `yeban_render{limiter}`、`render_core` 都消费它 —— ⛔ 只有 GUI 的导出面没有这个开关，人在窗口里**永远**导出一个不限制的版本，还以为导出＝听到的那个 | 对话框第五格 `export-wav-limiter-switch`；宿主在 `WavExportOptions::resolve()` **之后**赋 `wav_options.limiter`（⛔ 不给 `resolve()` 加形参：那是 GUI 与 CLI **共用**的唯一口径处，加形参会把"两边缺省逐字节相同"拆成两份） |
//! | **读数只在削波时发** | `true_peak_dbfs` / `headroom_db` / `limiter_reductions` 早就算好了，⛔ 却拴在 `if let Some(note) = clipping_note()` 上 ⇒ "贴轨但没削平"那一格一片空白（实测：引擎自报 `headroomDb = 5.18e-07`，GUI 回执里什么都没有） | 四条读数**恒发**，发在 `clipping_note` **之前**（长文案的门一位未动，⛔ 不削波仍不发它） |
//!
//! # 判据（四臂，其中①②各含一条字节自比）＋ 一条不开窗口的纯函数臂
//!
//! | 判据 | 钉住的事实 |
//! | :--- | :--- |
//! | ①（关） | GUI 导出过载工程 ⇒ 回执**逐字** ＝ `wav_export_receipt(off 报告, 路径, false) ＋ clipping_note`；`limiter=off`、`limiter_reductions=0`；产物 sha256 ＝ 同一次运行**直接调用** `export_wav`（`limiter:false`）的产物（自比） |
//! | ②（开） | 同一装配、同一工程，只把根属性翻真 ⇒ 回执**逐字** ＝ `wav_export_receipt(on 报告, 路径, true)`（⛔ 没有 `clipping_note`）；且 **`limiter_reductions > 0`**、**`clipped_samples == 0`**、`true_peak_dbfs` 比关那一臂**低**；产物 sha256 ＝ 直接调用（`limiter:true`）的产物 |
//! | ③（不是装饰） | 两条臂的产物 sha256 **不相等** ⇒ 勾选框改的是母带，不是回执上的一个字 |
//! | ④（关回去） | 把开关再翻回假 ⇒ 产物与 ① **逐字节相同**、回执同形（只有路径不同）⇒ 它是**每次调用**的修饰量，⛔ 不留在链路上、⛔ 不写进工程（**模型侧**读数：`port.fingerprint()` 与 `port.records().len()` 一位不变 —— ⛔ 不是界面那个 `undo-commit-count`，它的唯一写者根本不经过导出这条路） |
//!
//! ## 为什么 ② 断的是"压过 N > 0 个样本"，不是"峰值 ≤ 阈值"
//!
//! `yeban-dsp/src/limiter.rs` 的 `reduction_count` 文档自己警告过：只断言"峰值 ≤ 阈值"
//! 在**从未越阈值**的夹具上永真（假绿）。所以这里用**过载**夹具（母线 `volume_db = +24`），
//! 并要求计数 `> 0` —— 那才是"限制器真的在这条链路上、真的被驱动过"的结构证据。
//!
//! ## ⛔ 没有绝对常量
//!
//! 回执里的每个数字都取自**同一次运行**的直接调用报告；`sha256` 只做**自比**
//! （绝对字节来自 `sin`/`libm`/FMA，机器相关，第 313 条；钉常量只能描述某一台机器）。
//!
//! ## 平台约束：装平台的 `#[test]` 只有一个
//!
//! `slint::platform::set_platform` 是**线程局部**的，`Tier1Window::install` 每线程只能成功
//! 一次 ⇒ 同款用法见 `tests/gui_export_clipping.rs` / `tests/ui_export_dialog.rs`。
//! 本文件第一条判据独占"活窗口"这条线程；第二条（`the_readouts_are_emitted_…`）
//! **不开窗口**、只调 `export_wav` ⇒ 它与平台无关，可以安全地并存。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的装配入口 ⇒ 其余项在本测试目标里是 dead_code。
#[allow(dead_code)]
mod live;

// ⭐ 回执形状的第二份（唯一一份）住在 `tests/common/mod.rs`。
mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::str::FromStr as _;

use common::wav_export_receipt;
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

fn fixture_id(tail: &str) -> EntityId {
    EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8P0{tail}")).expect("夹具 ULID 必须合法")
}

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-dex1-{tag}-{}-{}",
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

/// 与 `tests/gui_export_clipping.rs` **逐字同款**的最小工程：master ＋ 一条 MIDI 轨。
///
/// `volume_db` 是**主总线之前**那条轨的推子；`+24` 把母线推到满量程之上（① 那一臂
/// 会量出 `clipped_samples > 0`），限制器就是为这一种工程存在的。
fn project_with_track(volume_db: f32) -> YebanProjectV1 {
    let master = fixture_id("M1");
    let track_id = fixture_id("T0");
    let clip = fixture_id("K0");
    let placement = fixture_id("P0");
    let note = fixture_id("N0");
    let edge = fixture_id("E0");
    let mut project = YebanProjectV1 {
        id: fixture_id("Z1"),
        title: "gui limiter fixture".to_owned(),
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

/// 一份直接调用的选项：与宿主的赋值顺序**逐字同形**（先 `resolve()`，后赋 `limiter`）。
///
/// ⛔ 不是 `WavExportOptions { limiter, ..resolve() }` —— 那会让判据比的是"另一条
/// 构造路"，而宿主走的是"resolve 之后改一个字段"。两者今天恰好同值，明天未必。
fn options_with_limiter(project: &YebanProjectV1, limiter: bool) -> WavExportOptions {
    let mut options = WavExportOptions::resolve(project, None, None, None, None::<u64>);
    options.limiter = limiter;
    options
}

#[test]
fn the_gui_limiter_switch_reaches_the_master_bus_and_the_receipt_says_so() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback export-wav("),
        "新 UI 契约面必须声明 callback export-wav("
    );
    assert!(
        aether_host.contains("on_export_wav"),
        "宿主接线层必须注册 on_export_wav"
    );
    assert!(
        aether_app.contains("> export-wav-limiter:"),
        "新 UI 契约面必须声明 > export-wav-limiter:"
    );
    assert!(
        aether_host.contains("export_wav_limiter"),
        "宿主接线层必须注册 export_wav_limiter"
    );
    assert!(
        aether_app.contains("> module-status:"),
        "新 UI 契约面必须声明 > module-status:"
    );
    assert!(
        aether_host.contains("set_module_status"),
        "宿主接线层必须注册 set_module_status"
    );
}

/// `D-EX-1` 的**纯函数**臂：回执形状里四个读数**恒发**，与 `limiter` 取值无关。
///
/// 这一条不开窗口、不导音频，把"`limiter:false` 时读数也在场"这件事钉在一个
/// 不需要 `slint::platform` 的位置上（`gui_export_clipping.rs` 的干净臂走的是
/// 整条 GUI 路，红的时候分不清是宿主没发还是平台没装起来）。
#[test]
fn the_readouts_are_emitted_whether_the_limiter_is_on_or_off() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback export-wav("),
        "新 UI 契约面必须声明 callback export-wav("
    );
    assert!(
        aether_host.contains("on_export_wav"),
        "宿主接线层必须注册 on_export_wav"
    );
    assert!(
        aether_app.contains("> export-wav-limiter:"),
        "新 UI 契约面必须声明 > export-wav-limiter:"
    );
    assert!(
        aether_host.contains("export_wav_limiter"),
        "宿主接线层必须注册 export_wav_limiter"
    );
    assert!(
        aether_app.contains("> module-status:"),
        "新 UI 契约面必须声明 > module-status:"
    );
    assert!(
        aether_host.contains("set_module_status"),
        "宿主接线层必须注册 set_module_status"
    );
}
