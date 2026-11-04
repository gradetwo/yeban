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
    let dir = scratch_dir("gui");
    let project = project_with_track(24.0);

    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            "<判据 D-EX-1: 母线限制器>",
            "yeban-app",
            project.clone(),
            1_760_000_000_000,
        )
        .expect("打开撤销会话"),
    ));
    let options = LiveWiringOptions {
        permission: Permission::Interactive,
        undo: Some(Rc::clone(&port)),
        ..LiveWiringOptions::default()
    };
    let live_ui = build_live_ui_with(&project, &options).expect("Tier-1 平台 + 活窗口");
    let window = slint::ComponentHandle::clone_strong(live_ui.ui());

    // 开关的**缺省**必须是关：逐字等于 CLI 不给 `--limiter`（`WavExportOptions::default()`）。
    assert!(
        !window.get_export_wav_limiter(),
        "装配后限制器必须默认关着（默认产物因此与修前逐字节相同）"
    );
    // ④ 的参照量：**模型**的指纹与动作日志长度。用模型读数而不是界面的撤销读数 ——
    // 后者的唯一写者是 `host::apply_undo`（`undo-commit-count`），导出这条动作根本不经过它，
    // 拿它当"没改工程"的证据是**自证**（⛔ 防伪第 4 条：读数没被更新 ≠ 工程没被改）。
    let fingerprint_before = port
        .fingerprint()
        .expect("装配后必须读得出工程指纹（模型容器字节的 SHA-256）");
    let records_before = port.records().len();

    // =====================================================================
    // ① 关：过载工程的默认导出（削波、有 `clipping_note`、限制器一次都没压）
    // =====================================================================
    let off_path = dir.join("limiter-off.wav");
    let off_oracle = dir.join("limiter-off-oracle.wav");
    let mut paths = ModulePaths::from_env();
    paths.export_wav = off_path.clone();
    wire_module_actions_with(&window, Some(Rc::clone(&port)), paths.clone());
    window.invoke_export_wav();
    let off_receipt = window.get_module_status().to_string();
    report_line(&format!("[D-EX-1] 关着的回执（逐字）: {off_receipt}"));
    assert!(window.get_module_status_succeeded(), "{off_receipt}");

    let off_report = export_wav(&project, &off_oracle, options_with_limiter(&project, false))
        .expect("对照导出（关）必须成功");
    // 夹具自检：这一臂**必须**真的过载，否则 ② 的"压过 N > 0"就退化成永真（假绿）。
    assert!(
        off_report.clipped_samples > 0,
        "夹具必须真的削波（否则本判据没有判别力）: {:?}",
        off_report.clipped_samples
    );
    assert_eq!(
        off_report.limiter_reductions, 0,
        "`limiter:false` ⇒ `render_core` 整段不跑，压样本计数必须恒 0"
    );
    assert_eq!(
        off_receipt,
        format!(
            "{}:clipping_note=\"{}\"",
            wav_export_receipt(&off_report, &off_path, false),
            off_report
                .clipping_note()
                .expect("过载工程必须给出削波提示")
        ),
        "关着的回执必须是**九字段 ＋ 四条恒发读数 ＋ 一条 `clipping_note`**，逐字"
    );
    assert_eq!(
        sha_hex(&off_path),
        sha_hex(&off_oracle),
        "① 自比：GUI 路径与直接调用（limiter:false）逐字节相同"
    );

    // =====================================================================
    // ② 开：同一装配、同一工程，只翻根属性
    // =====================================================================
    let on_path = dir.join("limiter-on.wav");
    let on_oracle = dir.join("limiter-on-oracle.wav");
    paths.export_wav = on_path.clone();
    wire_module_actions_with(&window, Some(Rc::clone(&port)), paths.clone());
    window.set_export_wav_limiter(true);
    assert!(window.get_export_wav_limiter(), "根属性必须翻得动");
    window.invoke_export_wav();
    let on_receipt = window.get_module_status().to_string();
    report_line(&format!("[D-EX-1] 开着的回执（逐字）: {on_receipt}"));
    assert!(window.get_module_status_succeeded(), "{on_receipt}");

    let on_report = export_wav(&project, &on_oracle, options_with_limiter(&project, true))
        .expect("对照导出（开）必须成功");
    // ⭐ 结构证据（文件头解释为什么不是"峰值 ≤ 阈值"）：限制器**真的被驱动过**。
    assert!(
        on_report.limiter_reductions > 0,
        "过载母带上限制器必须压过样本（`0` ＝ 勾选框是装饰 / 接线漏了）"
    );
    assert_eq!(
        on_report.clipped_samples, 0,
        "限制器输出严格不超过 `LIMITER_CEILING` ⇒ 这一臂不该再有削波"
    );
    let (off_peak, on_peak) = (
        // 测不出（`None`）时这一臂无从比较 ⇒ 如实红，⛔ 不猜一个数。
        off_report
            .true_peak_dbfs
            .expect("关着的过载导出必须量得出真峰值"),
        on_report
            .true_peak_dbfs
            .expect("开限制器后同样必须量得出真峰值"),
    );
    assert!(
        on_peak < off_peak,
        "真峰值必须被压下去：关 {off_peak} dBTP → 开 {on_peak} dBTP"
    );
    assert!(
        !on_receipt.contains("clipping_note"),
        "不削波 ⇒ 那条长文案不发（`D-EX-1` 改的是读数的门，⛔ 不是长文案的门）: {on_receipt}"
    );
    assert_eq!(
        on_receipt,
        wav_export_receipt(&on_report, &on_path, true),
        "开着的回执必须是**同一形状**、`limiter=on`、`limiter_reductions` 取报告里的真实计数"
    );
    assert_eq!(
        sha_hex(&on_path),
        sha_hex(&on_oracle),
        "② 自比：GUI 路径与直接调用（limiter:true）逐字节相同 —— 这就是\"开关接到的是真限制器\""
    );

    // =====================================================================
    // ③ 不是装饰：两条臂的母带**必须**不同
    // =====================================================================
    assert_ne!(
        sha_hex(&off_path),
        sha_hex(&on_path),
        "勾选框若只改回执不改音频，这里会相等 ⇒ 红"
    );
    assert_eq!(
        on_report.frames, off_report.frames,
        "限制器不改请求帧数（33 帧前瞻延迟由 `render_core` 补掉：多渲 33 帧、随后丢掉前 33 帧）"
    );

    // =====================================================================
    // ④ 关回去：默认产物逐字节回到 ①，且撤销读数一位不变
    // =====================================================================
    let back_path = dir.join("limiter-back.wav");
    paths.export_wav = back_path.clone();
    wire_module_actions_with(&window, Some(Rc::clone(&port)), paths);
    window.set_export_wav_limiter(false);
    window.invoke_export_wav();
    let back_receipt = window.get_module_status().to_string();
    assert!(window.get_module_status_succeeded(), "{back_receipt}");
    assert_eq!(
        sha_hex(&back_path),
        sha_hex(&off_path),
        "关回去必须与 ① 逐字节相同 ⇒ 开关是**每次调用**的修饰量，不留在链路上"
    );
    assert_eq!(
        back_receipt,
        off_receipt.replace(
            &off_path.display().to_string(),
            &back_path.display().to_string()
        ),
        "关回去的回执必须与 ① 同形（只有路径这一处不同）"
    );
    assert_eq!(
        port.fingerprint().expect("收尾同样必须读得出工程指纹"),
        fingerprint_before,
        "开／关限制器与三次导出都不许改**工程**一个字节（`#302` 的硬边界：导出修饰量不进工程）"
    );
    assert_eq!(
        port.records().len(),
        records_before,
        "导出⛔ 不是编辑动作：动作日志一条都不许多（多了就是给它记了 `Op`）"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// `D-EX-1` 的**纯函数**臂：回执形状里四个读数**恒发**，与 `limiter` 取值无关。
///
/// 这一条不开窗口、不导音频，把"`limiter:false` 时读数也在场"这件事钉在一个
/// 不需要 `slint::platform` 的位置上（`gui_export_clipping.rs` 的干净臂走的是
/// 整条 GUI 路，红的时候分不清是宿主没发还是平台没装起来）。
#[test]
fn the_readouts_are_emitted_whether_the_limiter_is_on_or_off() {
    let project = project_with_track(0.0);
    let dir = scratch_dir("shape");
    let mut reports = Vec::new();
    for limiter in [false, true] {
        let path = dir.join(format!("shape-{}.wav", if limiter { "on" } else { "off" }));
        let report = export_wav(&project, &path, options_with_limiter(&project, limiter))
            .expect("形状臂必须导出成功");
        let receipt = wav_export_receipt(&report, &path, limiter);
        for key in [
            "limiter=",
            "limiter_reductions=",
            "true_peak_dbfs=",
            "headroom_db=",
        ] {
            assert!(
                receipt.contains(key),
                "`{key}` 必须**恒发**（开还是关都发）: {receipt}"
            );
        }
        reports.push(report);
    }
    // 安静夹具（`volume_db = 0`）：不削波那一臂也**必须**看得见数 —— 这正是 `D-EX-1`
    // 要修的那一格（`#229` 的旧门只在削波时发）。
    for report in &reports {
        assert_eq!(report.clipped_samples, 0, "安静夹具不该削波");
        assert!(
            report.true_peak_dbfs.is_some() && report.headroom_db.is_some(),
            "不削波也必须报得出峰值（测不出才是 `None`，⛔ 这里不是）"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}
