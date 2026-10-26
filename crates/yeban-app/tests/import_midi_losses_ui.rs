//! ⭐ 任务单 `#319`／`#320`：**GUI 导入回执里的损失与歌词读数**（`#318`／`#317`
//! 的第二个出口）。
//!
//! ## 本文件要证的三句话
//!
//! 1. **修前红**（`/tmp/319-red.log`）：`host::wire_module_actions` 的 `import-midi`
//!    成功回执只写 `outcome/path/notes/track/clip/commit/origin`，**损失一个字不提**；
//! 2. **修后**：同一份 316 B 夹具（24 类 aux、29 条事件里 **28 条 loss**、5 颗音符）⇒
//!    - 回执行末尾 `losses="…"` 与 `loss_detail="…"`；
//!    - 右栏 `module-import-losses` 一行**可见摘要**（与削波警示同一槽位、互斥）；
//! 3. **与 CLI 同源**：GUI 的 `loss_detail` 必须**逐字等于** CLI `imported-midi:` 行
//!    `origin=midi-input ` 之后的尾部 —— 同一份 `MidiImportLosses`、同一个
//!    `MidiImportLosses::loss_detail()`／`aux_fields()`，⛔ 不是第二份格式化；
//!    `lyrics_detail` 必须**逐字等于** CLI 行里 `lyrics=` 起、`track=` 前的五字段
//!    —— 同一份 `MidiLyricCounts`、同一个 `Display`（`#320`）。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是**线程局部**的，`Tier1Window::install` 每线程
//! 只能成功一次 ⇒ 本二进制只有一个 **GUI** `#[test]`，内部按"行为臂 / 三档几何臂"各起线程
//! （与 `tests/gui_clipping_warning.rs` 同款）。`#321` 的取值器判据是纯函数，另有三个
//! `#[test]`（不碰 slint，可与 GUI 臂并行跑）。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code
// （`-D warnings` 会判红）。同款见 `tests/ui_module_actions.rs`。
#[allow(dead_code)]
mod live;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;

use live::{LiveWiringOptions, build_live_ui_with};
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_midi::midi::MidiAuxEventKind;
use yeban_model::YebanProjectV1;
use yeban_ui_test_port::Rgb8Image;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::image::Rect;
use yeban_ui_test_port::inspect::tree_from_element_root;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, fnv1a64, report_line, write_artifact};
use yeban_ui_test_port::tree::{ControlNode, ControlTree};

/// 真二进制（与 `tests/midi_round_trip.rs` 同一个入口）。
const BIN: &str = env!("CARGO_BIN_EXE_yeban-app");

/// 可见摘要行的语义 ID（`ui/module_actions.slint` ↔ `src/elements.rs` 两处逐字相同）。
const LOSS_ID: &str = "module-import-losses";

/// 316 B 夹具的**期望摘要**（24 类 aux、29 条事件里 **28 条 loss** —— ⭐ `#342` 起
/// 唯一的 `Verse` marker 落成一个段落 ⇒ 5 颗音符保留通道值、丢释放力度，
/// 2 条歌词候选里 1 条未归属）。
///
/// ⚠ 这是**夹具的期望值**（与 `tests/midi_round_trip.rs` 的逐类期望表同源），
/// ⛔ 不是从 GUI 回执反推的"第二份判定"。
const EXPECTED_SUMMARY: &str = "⚠ 导入丢失：24 类/28 条；歌词未归属 1；通道/力度0/5";

/// 三档窗口（与 `tests/pane_no_overlap.rs` / `tests/gui_clipping_warning.rs` 同一组）。
const TIERS: &[(u32, u32)] = &[(1920, 1080), (1440, 900), (1024, 800)];

/// 测试产物落在这个临时目录里（`Drop` 时清理，**绝不污染仓库**）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-319-losses-{}-{}",
            std::process::id(),
            yeban_model::ids::EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn run_cli(args: &[&str]) -> String {
    let output = Command::new(BIN)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|error| panic!("无法执行 {BIN}: {error}"));
    assert!(
        output.status.success(),
        "`yeban-app {args:?}` 退出码 {:?}; stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn line_with<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.lines().find(|line| line.starts_with(prefix))
}

/// 一条回执的**顶层**字段取值（引号感知）；缺席返回 `None`。
///
/// ## 口径（`#321`）
///
/// 回执语法是 `module:action=…:key=value:…`（`host.rs::module_receipt`）。字段名只认
/// **顶层**位置：`key=` 必须紧跟一个**顶层** `:`（引号内的 `:` 不算起点）；值是引号时
/// 跳到**第一个闭合引号**（引号内的 `channels=` / `format=` 一律**不是**字段名），
/// 普通值取到下一个 `:`。
///
/// ## 引号里的 `"`（写死的契约）
///
/// 回执**没有转义**（`host.rs` 只做 `format!("message=\"{message}\"")`）⇒ 本取值器把
/// 第一个 `"` 当闭合引号。值里真带 `"` 时，闭合引号后的那个字节不是 `:` ⇒ 其后的字段
/// **一律取不到**（`None`），由调用方按"缺席即失败"处理。⛔ 本判据不改 producer 去加
/// 转义（那会动逐字断言与 CLI 同源）；需要转义时另立单。
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
                    let rest = &receipt[value_start..];
                    if let Some(quoted) = rest.strip_prefix('"') {
                        let end = quoted.find('"').expect("引号必须闭合");
                        return Some(quoted[..end].to_owned());
                    }
                    let end = rest.find(':').unwrap_or(rest.len());
                    return Some(rest[..end].to_owned());
                }
            }
            _ => {}
        }
    }
    None
}

/// 取一个字段；缺席即判据失败（⛔ 不返回空串 —— "没找到"与"值是空的"必须可分）。
///
/// `#321` 起走 [`field_of_opt`]：只认顶层字段；引号内的同名键**不会**再被误取
/// （`field_of("channels")` 对 `loss_detail="… channels=2 …"` 会 panic，而不是返回 `2"`）。
fn field_of(receipt: &str, key: &str) -> String {
    field_of_opt(receipt, key).unwrap_or_else(|| panic!("回执里没有字段 `{key}`: {receipt}"))
}

/// 取 CLI 行里的一个 `key=value`（值到下一个空白为止）。
fn cli_field(line: &str, key: &str) -> String {
    let needle = format!("{key}=");
    let start = line
        .find(&needle)
        .unwrap_or_else(|| panic!("CLI 行里没有 `{key}`: {line}"))
        + needle.len();
    let rest = &line[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    rest[..end].to_owned()
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

/// 手工拼一份**含全部 24 类 aux 事件**的 SMF（与 `tests/midi_round_trip.rs` 的
/// `all_aux_events_smf` 逐字同款：format 1、3 轨、960 PPQ、5 音符、2 歌词）。
///
/// ⛔ 借导出器构造：导出器只写 `FF 03` / `FF 51` / `FF 58` / `FF 05` 四类，
/// 用它会让"没写"与"没读"混在一起。
fn all_aux_events_smf() -> Vec<u8> {
    fn vlq(mut value: u32) -> Vec<u8> {
        let mut out = vec![u8::try_from(value & 0x7F).expect("7 bit")];
        value >>= 7;
        while value > 0 {
            out.push(0x80 | u8::try_from(value & 0x7F).expect("7 bit"));
            value >>= 7;
        }
        out.reverse();
        out
    }
    fn meta(kind: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0xFF, kind, u8::try_from(payload.len()).expect("短负载")];
        out.extend_from_slice(payload);
        out
    }
    fn track(events: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let mut body = Vec::new();
        for (delta, event) in events {
            body.extend(vlq(*delta));
            body.extend_from_slice(event);
        }
        let mut out = b"MTrk".to_vec();
        out.extend_from_slice(&u32::try_from(body.len()).expect("短轨").to_be_bytes());
        out.extend(body);
        out
    }

    let conductor = track(&[
        (0, meta(0x00, &[0x00, 0x01])),
        (0, meta(0x03, b"Conductor")),
        (0, meta(0x51, &[0x03, 0xD0, 0x90])),
        (0, meta(0x58, &[6, 3, 24, 8])),
        (0, meta(0x59, &[2, 0])),
        (0, meta(0x06, b"Verse")),
        (0, meta(0x05, b"la")),
        (0, meta(0x01, b"a text meta")),
        (0, meta(0x02, b"(c) 2026")),
        (0, meta(0x04, b"Piano")),
        (0, meta(0x07, b"Cue")),
        (0, meta(0x08, b"P1")),
        (0, meta(0x09, b"Dev")),
        (0, meta(0x20, &[1])),
        (0, meta(0x21, &[0])),
        (0, meta(0x54, &[0; 5])),
        (0, meta(0x7F, &[1, 2, 3])),
        (0, meta(0x60, &[0xAB])),
        (0, vec![0xF0, 0x03, 0x01, 0x02, 0xF7]),
        (0, vec![0xF7, 0x02, 0x03, 0x04]),
        (480, vec![0xE0, 0x00, 0x40]),
        (0, vec![0xB0, 7, 100]),
        (0, vec![0xD0, 40]),
        (0, vec![0xA0, 60, 33]),
        (0, vec![0xC0, 33]),
        (480, meta(0x51, &[0x07, 0xA1, 0x20])),
        (1440, meta(0x06, b"Chorus")),
        (0, meta(0x58, &[3, 2, 24, 8])),
        (0, meta(0x2F, &[])),
    ]);
    let lead = track(&[
        (0, meta(0x03, b"Lead")),
        (0, vec![0x90, 60, 100]),
        (480, vec![0x80, 60, 64]),
        (0, vec![0x90, 64, 90]),
        (0, meta(0x05, b"re")),
        (480, vec![0x80, 64, 32]),
        (0, vec![0x90, 67, 80]),
        (480, vec![0x80, 67, 16]),
        (0, meta(0x2F, &[])),
    ]);
    let bass = track(&[
        (0, meta(0x03, b"Bass")),
        (0, vec![0x91, 48, 110]),
        (960, vec![0x81, 48, 8]),
        (0, vec![0x91, 52, 100]),
        (960, vec![0x81, 52, 4]),
        (0, meta(0x2F, &[])),
    ]);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"MThd");
    bytes.extend_from_slice(&6u32.to_be_bytes());
    bytes.extend_from_slice(&1u16.to_be_bytes());
    bytes.extend_from_slice(&3u16.to_be_bytes());
    bytes.extend_from_slice(&960u16.to_be_bytes());
    bytes.extend_from_slice(&conductor);
    bytes.extend_from_slice(&lead);
    bytes.extend_from_slice(&bass);
    bytes
}

fn write_fixture(dir: &Path) -> PathBuf {
    let bytes = all_aux_events_smf();
    let path = dir.join("all-aux-316b.mid");
    std::fs::write(&path, &bytes).expect("写 316 B 夹具");
    println!(
        "[#319 fixture] path={} bytes={}",
        path.display(),
        bytes.len()
    );
    path
}

// ===========================================================================
// 行为臂：真装配 + 真导入 + 回执/属性/树/帧 + 与 CLI 同一份
// ===========================================================================
fn behavior_arm() {
    let scratch = Scratch::new();
    let mid = write_fixture(&scratch.dir);
    let export = scratch.join("ui-export.mid");

    // ⚠ 路径唯一解析点 = `ModulePaths::from_env()`（产品同款）。
    // SAFETY: 本臂独占一个线程；设置早于 `build_live_ui_with`（它会起后台线程），
    // 且几何臂在本臂 join 之后才启动。
    unsafe {
        std::env::set_var("YEBAN_UI_IMPORT_MIDI_PATH", &mid);
        std::env::set_var("YEBAN_UI_EXPORT_MIDI_PATH", &export);
    }

    let project: YebanProjectV1 = yeban_model::samples::filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据: #319 导入损失>", "yeban-app", project.clone(), 1)
            .expect("打开撤销会话"),
    ));
    let options = LiveWiringOptions {
        permission: Permission::Interactive,
        undo: Some(Rc::clone(&port)),
        ..LiveWiringOptions::default()
    };
    let live_ui = build_live_ui_with(&project, &options).expect("Tier-1 + 活窗口 + 模块接线");
    let window = live_ui.ui();
    let default_frame = live_ui.capture().expect("默认帧");
    let default_fingerprint = fnv1a64(default_frame.pixels());
    let default_tree = tree_of(window);
    report_line(&format!(
        "[#319 默认帧] 行为臂 1920x1080 fingerprint={default_fingerprint:016x} bytes={}",
        default_frame.pixels().len()
    ));
    assert!(
        default_tree.find_by_id(LOSS_ID).is_none(),
        "默认帧里 `module-import-losses` 必须在树外（空串 ⇒ `visible:false`）"
    );

    // ---- ① 导入 316 B 夹具：回执逐字 ----
    window.invoke_import_midi();
    let succeeded = window.get_module_status_succeeded();
    let receipt = window.get_module_status().to_string();
    println!("[#319 gui-receipt] succeeded={succeeded} module-status={receipt}");
    assert!(succeeded, "GUI 导入必须成功: {receipt}");
    assert_eq!(field_of(&receipt, "action"), "import-midi", "{receipt}");
    assert_eq!(field_of(&receipt, "outcome"), "ok", "{receipt}");
    assert_eq!(field_of(&receipt, "notes"), "5", "{receipt}");
    assert_eq!(field_of(&receipt, "origin"), "midi-input", "{receipt}");
    assert_eq!(
        field_of(&receipt, "losses"),
        EXPECTED_SUMMARY,
        "回执里的损失摘要必须逐字: {receipt}"
    );

    // ---- ② 同一份 `MidiImportLosses`：GUI 明细 == CLI 尾部 ----
    let cli_out = run_cli(&[
        "--project-sample",
        "default",
        "--import-midi",
        mid.to_str().expect("utf8"),
        "--save-as",
        scratch.join("cli.yeban").to_str().expect("utf8"),
    ]);
    let cli_line = line_with(&cli_out, "imported-midi:").expect("CLI 必须有 imported-midi 行");
    println!("[#319 cli-line] {cli_line}");
    let cli_tail = cli_line
        .split_once("origin=midi-input ")
        .map(|(_, tail)| tail)
        .expect("CLI 行必须有 `origin=midi-input ` 之后的部分");
    let detail = field_of(&receipt, "loss_detail");
    assert_eq!(
        detail, cli_tail,
        "GUI `loss_detail` 必须逐字等于 CLI 行 `origin=midi-input ` 之后的尾部（同一个格式化函数）"
    );
    // ---- ②b 歌词五字段：GUI `lyrics_detail` 逐字等于 CLI 行那五个字段（`#320`）----
    let cli_lyrics_start = cli_line.find("lyrics=").expect("CLI 行必须有 `lyrics=`");
    let cli_lyrics_rest = &cli_line[cli_lyrics_start..];
    let cli_lyrics_end = cli_lyrics_rest
        .find(" track=")
        .expect("CLI 行的歌词字段必须在 `track=` 前结束");
    let cli_lyrics = &cli_lyrics_rest[..cli_lyrics_end];
    let lyrics_detail = field_of(&receipt, "lyrics_detail");
    assert_eq!(
        lyrics_detail, cli_lyrics,
        "GUI `lyrics_detail` 必须逐字等于 CLI 行 lyrics= 起、track= 前的五字段（同一个 Display）"
    );
    assert_eq!(
        lyrics_detail, "lyrics=2 attached=1 dropped=1 non-utf8=0 empty=0",
        "316 B 夹具的歌词五字段必须逐项"
    );
    report_line(&format!(
        "[#320 同源] GUI lyrics_detail={lyrics_detail:?} == CLI 五字段（{} 字节）",
        lyrics_detail.len()
    ));

    // ⭐ `#321`：`channels` 在 `loss_detail` 的**引号内**、**不是顶层字段** ⇒
    // 顶层取值器取不到它（`field_of_opt("channels")` 必须为 `None`；`field_of("channels")`
    // 会 panic —— 缺席即判据失败）。数字仍从 `detail` 用 `cli_field` 取 ⇒
    // **这是语义正确解，不是绕过**。
    assert_eq!(field_of_opt(&receipt, "channels"), None, "{receipt}");
    assert_eq!(field_of_opt(&receipt, "lyrics"), None, "{receipt}");
    assert!(
        detail.contains(&format!("channels={}", cli_field(cli_line, "channels"))),
        "两个出口的上下文读数必须同源: {detail}"
    );

    // ---- ③ 明细：24 类一个不缺 + 4 项上下文 + 按音符 4 项 ----
    for (key, want) in [
        ("format", "1"),
        ("file-tracks", "3"),
        ("track-partitions-dropped", "0"),
        ("channels", "2"),
        ("note-channel-read", "5"),
        ("note-channel-dropped", "0"),
        ("note-off-velocity-read", "5"),
        ("note-off-velocity-dropped", "5"),
    ] {
        assert_eq!(
            cli_field(&detail.replace(',', " "), key),
            want,
            "loss_detail 的 `{key}`: {detail}"
        );
    }
    let mut aux_kinds = 0_usize;
    for kind in MidiAuxEventKind::ALL {
        let key = kind.key();
        let read_key = format!("{key}-read=");
        let dropped_key = format!("{key}-dropped=");
        assert!(
            detail.contains(&read_key),
            "loss_detail 必须逐类含 `{read_key}`（24 类一个不缺）: {detail}"
        );
        assert!(
            detail.contains(&dropped_key),
            "loss_detail 必须逐类含 `{dropped_key}`: {detail}"
        );
        aux_kinds += 1;
    }
    assert_eq!(aux_kinds, MidiAuxEventKind::COUNT, "逐类数必须 = 24");
    assert_eq!(
        aux_kinds,
        MidiAuxEventKind::ALL.len(),
        "`ALL` 与 `COUNT` 必须一致"
    );
    report_line(&format!(
        "[#319 同源] GUI losses={:?}；loss_detail {} 字节 == CLI 尾部；逐类 {} 类",
        field_of(&receipt, "losses"),
        detail.len(),
        aux_kinds
    ));

    // ---- ④ 可见摘要行：属性 == 树 value == 文本；几何在右栏内 ----
    let loss_line = window.get_module_import_losses().to_string();
    assert_eq!(loss_line, EXPECTED_SUMMARY, "可见摘要属性必须逐字");
    let tree = tree_of(window);
    let node = node_of(&tree, LOSS_ID);
    assert_eq!(node.role.as_str(), "text", "损失行的角色");
    assert_eq!(
        node.label,
        "最近一次 MIDI 文件导入的损耗摘要（类数 / 事件数 / 歌词丢失 / 音符通道 / 释放力度）",
        "损失行的 `accessible-label` 必须与注册表逐字相同"
    );
    assert_eq!(
        node.value.as_deref(),
        Some(EXPECTED_SUMMARY),
        "`accessible-value` = 可见文本"
    );
    let loss_box = bounds_of(&tree, LOSS_ID);
    let rail = bounds_of(&tree, "ai-rail");
    assert!(
        loss_box.x >= rail.x
            && loss_box.y >= rail.y
            && loss_box.x + i32::try_from(loss_box.width).expect("宽")
                <= rail.x + i32::try_from(rail.width).expect("宽")
            && loss_box.y + i32::try_from(loss_box.height).expect("高")
                <= rail.y + i32::try_from(rail.height).expect("高"),
        "损失行必须完整落在右栏内（loss {loss_box:?} / rail {rail:?}）"
    );
    assert_eq!(
        (loss_box.width, loss_box.height),
        (264, 16),
        "损失行几何必须是 264×16（与削波行同槽位）"
    );
    assert!(
        !intersects(loss_box, bounds_of(&tree, "module-last-status")),
        "损失行不得与回执行相交"
    );
    assert!(
        tree.find_by_id("module-clipping-warning").is_none(),
        "导入之后削波警示必须为空（两条专用行互斥）"
    );
    assert_eq!(
        window.get_module_clipping_warning(),
        "",
        "削波属性必须被清空"
    );

    // ---- ⑤ 帧：只改「损失行 + 回执行」两块 ----
    let shown_frame = live_ui.capture().expect("导入后帧");
    let shot = write_artifact("319-import-losses-1920x1080", &shown_frame)
        .expect("把导入后的帧写成 PNG（过程产物，落在 target/ui-test-port/）");
    report_line(&format!("[#319 截图] {}", shot.display()));
    let receipt_box = bounds_of(&tree, "module-last-status");
    let (inside, rightmost) = diff_inside(&default_frame, &shown_frame, loss_box);
    let outside = diff_outside(&default_frame, &shown_frame, loss_box)
        .saturating_sub(diff_inside(&default_frame, &shown_frame, receipt_box).0);
    let right_edge = loss_box.x + i32::try_from(loss_box.width).expect("宽");
    let rightmost = rightmost.expect("可见帧里损失行必须有差异像素");
    report_line(&format!(
        "[#319 可见] 行 {loss_box:?}；区域内差异 {inside} px、右沿 x={rightmost} / 行右沿 {right_edge}；行外（除回执行）差异 {outside} px"
    ));
    assert!(inside > 0, "摘要文本必须真的画出像素");
    assert_eq!(outside, 0, "默认帧 → 导入帧只许改损失行与通用回执行");
    assert!(
        i32::try_from(rightmost).expect("x") < right_edge - 2,
        "文本右沿 {rightmost} 必须离行右沿 {right_edge} 至少 3px（否则有被裁掉的风险）"
    );

    // ---- ⑥ 互斥（两臂）：导入后紧跟导出 / 陈旧削波后紧跟导入 ----
    window.invoke_export_midi();
    let export_receipt = window.get_module_status().to_string();
    assert!(
        window.get_module_status_succeeded(),
        "导出 MIDI 必须成功: {export_receipt}"
    );
    assert_eq!(
        window.get_module_import_losses(),
        "",
        "导出动作必须把上一条导入损失行清空"
    );
    assert_eq!(window.get_module_clipping_warning(), "");
    let after_export = tree_of(window);
    assert!(
        after_export.find_by_id(LOSS_ID).is_none(),
        "导出之后损失行必须离开运行时树"
    );
    report_line(
        "[#319 互斥] 导入 → 导出：losses=\"\"、clipping=\"\"，树里无 `module-import-losses`",
    );

    window.set_module_clipping_warning("⚠ 陈旧削波读数".into());
    window.invoke_import_midi();
    assert!(
        window.get_module_status_succeeded(),
        "第二次导入必须成功: {}",
        window.get_module_status()
    );
    assert_eq!(
        window.get_module_clipping_warning(),
        "",
        "导入动作必须把陈旧的削波警示清空（两行互斥）"
    );
    assert_eq!(window.get_module_import_losses(), EXPECTED_SUMMARY);
    report_line("[#319 互斥] 陈旧削波 → 导入：clipping=\"\"、losses=摘要（不会两行同时非空）");

    let _ = std::fs::remove_dir_all(&scratch.dir);
}

// ===========================================================================
// 几何臂：三档（1920 / 1440 / 1024），空串 ⇒ 零像素；非空 ⇒ 在栏内、不与他人相交
// ===========================================================================
fn check_geometry(width: u32, height: u32) {
    let project = yeban_model::samples::demo_project();
    let view = ViewState::from_project(&project).expect("演示工程必须能投影");
    let scene = DemoScene::from_view(&view);
    // 与 `tests/gui_clipping_warning.rs` 同款：窗口尺寸由 `LivePort` 给，
    // 装配入口仍是生产的 `host::build_main_window_with_console_tab`。
    let port = LivePort::new(Size::new(width, height), Permission::ReadOnly, None, || {
        host::build_main_window_with_console_tab(&view, &scene, 0)
    })
    .expect("Tier-1 平台 + 活窗口");
    if port.ui().get_compact() {
        port.ui().set_ai_drawer_open(true);
    }
    let default_frame = port.window().capture().expect("默认帧");
    let default_fingerprint = fnv1a64(default_frame.pixels());
    let default_tree = tree_of(port.ui());
    report_line(&format!(
        "[#319 默认帧] {width}x{height} fingerprint={default_fingerprint:016x} bytes={}",
        default_frame.pixels().len()
    ));
    assert!(
        default_tree.find_by_id(LOSS_ID).is_none(),
        "[#319 几何 {width}x{height}] 默认帧里损失行必须在树外"
    );

    port.ui().set_module_import_losses(EXPECTED_SUMMARY.into());
    let tree = tree_of(port.ui());
    let loss = bounds_of(&tree, LOSS_ID);
    let rail = bounds_of(&tree, "ai-rail");
    report_line(&format!(
        "[#319 几何 {width}x{height}] module-import-losses bounds={loss:?} value={:?}；右栏={rail:?}",
        node_of(&tree, LOSS_ID).value
    ));
    assert!(
        loss.x >= rail.x
            && loss.y >= rail.y
            && loss.x + i32::try_from(loss.width).expect("宽")
                <= rail.x + i32::try_from(rail.width).expect("宽")
            && loss.y + i32::try_from(loss.height).expect("高")
                <= rail.y + i32::try_from(rail.height).expect("高"),
        "[#319 几何 {width}x{height}] 损失行必须完整落在右栏内（loss {loss:?} / rail {rail:?}）"
    );
    for id in [
        "module-last-status",
        "module-midi-in-status",
        "module-clipping-warning",
    ] {
        if let Some(other) = tree.find_by_id(id) {
            let other = other.bounds.expect("相邻模块行必须有几何");
            assert!(
                !intersects(loss, other),
                "[#319 几何 {width}x{height}] `{LOSS_ID}` {loss:?} 与 `{id}` {other:?} 相交"
            );
            report_line(&format!(
                "[#319 几何 {width}x{height}] {LOSS_ID} ⟂ {id}: 0 相交（{other:?}）"
            ));
        }
    }
    let shown_frame = port.window().capture().expect("可见帧");
    let (inside, _) = diff_inside(&default_frame, &shown_frame, loss);
    let outside = diff_outside(&default_frame, &shown_frame, loss);
    report_line(&format!(
        "[#319 几何 {width}x{height}] 空 ⇒ 非空：行内差异 {inside} px、行外 {outside} px"
    ));
    assert!(
        inside > 0,
        "[#319 几何 {width}x{height}] 非空文本必须画像素"
    );
    assert_eq!(
        outside, 0,
        "[#319 几何 {width}x{height}] 只许改损失行自己那一格"
    );

    port.ui().set_module_import_losses("".into());
    let cleared_tree = tree_of(port.ui());
    assert!(
        cleared_tree.find_by_id(LOSS_ID).is_none(),
        "[#319 几何 {width}x{height}] 清空之后必须离开运行时树"
    );
    let cleared_frame = port.window().capture().expect("清空帧");
    assert_eq!(
        cleared_frame.pixels(),
        default_frame.pixels(),
        "[#319 几何 {width}x{height}] 空串 ⇒ 逐字节回到默认帧（不占布局、不画像素）"
    );
    report_line(&format!(
        "[#319 默认帧] {width}x{height}：默认帧 {} 字节；设成非空再设回空之后逐字节相同",
        default_frame.pixels().len()
    ));
}

#[test]
fn the_gui_import_receipt_reports_the_same_losses_as_the_cli() {
    // 行为臂先跑（它会 `set_var`；几何臂不读这些环境变量，join 之后再起线程）。
    std::thread::spawn(behavior_arm)
        .join()
        .expect("行为臂不得 panic");

    let geometry: Vec<()> = TIERS
        .iter()
        .map(|&(width, height)| {
            std::thread::spawn(move || check_geometry(width, height))
                .join()
                .unwrap_or_else(|_| panic!("[#319 几何 {width}x{height}] 子线程 panic"))
        })
        .collect();
    assert_eq!(geometry.len(), TIERS.len(), "三档几何都必须跑过");
}

/// `#321` 负向判据：**引号内的键 ⛔ 可被顶层取到**。
///
/// 这条回执是判据 ① 的最小形状：`loss_detail` 的整串里含 `channels=` / `format=`。
/// 修前实测（`/tmp/mod-fq-repro.log`）：`field_of("channels") == "2\""`、
/// `field_of("format") == "1 channels=2\""`（各自把闭合引号也带进值里）。
/// 修后两者都必须是 `None`，而 `loss_detail` 仍是整串（引号内的 `=` 不切）。
#[test]
fn quoted_values_do_not_expose_their_inner_keys_to_the_top_level() {
    let receipt = concat!(
        r#"module:action=import-midi:outcome=ok:losses="⚠ 导入丢失：…""#,
        r#":loss_detail="format=1 channels=2""#,
        r#":lyrics_detail="lyrics=2 attached=1 dropped=1 non-utf8=0 empty=0""#
    );
    assert_eq!(field_of_opt(receipt, "channels"), None);
    assert_eq!(field_of_opt(receipt, "format"), None);
    assert_eq!(field_of_opt(receipt, "lyrics"), None);
    assert_eq!(
        field_of(receipt, "loss_detail"),
        "format=1 channels=2",
        "引号内的 `=` 不切；引号值整体去掉引号"
    );
    assert_eq!(
        field_of(receipt, "lyrics_detail"),
        "lyrics=2 attached=1 dropped=1 non-utf8=0 empty=0"
    );
}

/// `#321` 负向判据：缺席 ⇒ `field_of` 仍 panic（⛔ 不返回空串，⛔ 不返回 `None`）。
#[test]
#[should_panic(expected = "回执里没有字段 `channels`")]
fn a_key_inside_quotes_is_still_absent_for_the_panicking_getter() {
    let receipt = r#"module:action=import-midi:outcome=ok:loss_detail="format=1 channels=2""#;
    let _ = field_of(receipt, "channels");
}

/// `#321` 契约：回执**没有转义** ⇒ 第一个 `"` 就是闭合引号；值里带 `"` 会让其后的
/// 字段不可达（`None`）。这条把取舍钉死（⛔ silently 猜第二个引号）。
#[test]
fn the_first_closing_quote_is_the_contract_and_an_inner_quote_hides_the_rest() {
    let receipt = r#"module:action=x:message="他说"停":outcome=ok"#;
    assert_eq!(field_of_opt(receipt, "message").as_deref(), Some("他说"));
    assert_eq!(field_of_opt(receipt, "outcome"), None);
}
