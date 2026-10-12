//! **四个模块入口的 UI 接线** —— 端到端判据（`cargo test -p yeban-app --all-targets` 会跑它）。
//!
//! ## 本文件要证的句子
//!
//! `crates/yeban-app/ui/module_actions.slint` 的四个按钮**不是画上去的控件**：
//! 每一个都有稳定语义 ID、都在活窗口的运行时控件树里可寻址，按下去**真的**走到
//! `export_wav` / `import_midi` / `sfz_load` / `record_wav` 四个**已经存在**的模块实现，
//! 并把**结构化回执**写回界面（成功与失败都写）。
//!
//! ## 判据（对应交付要求的 a / b / c）
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（可寻址） | 四个入口按钮 + 小节标题 + 状态行共 6 个语义 ID 都**在运行时控件树里**，角色符合登记；初始回执是**空**（不是一个编造的读数） |
//! | ②（状态按预期变） | 按下入口 ⇒ `MainWindow.module-status` 变成结构化回执，字段（路径 / 帧数 / 采样率 / 区域数 / 来源 / `origin=midi-input`）逐条对上；目标文件真的落盘、字节数与回执一致 |
//! | ③（错误路径不 panic） | SFZ 源不存在 ⇒ 回执是 `outcome=error` + 稳定错误码，进程**不** panic，**不留半成品** |
//! | ④（**硬件 MIDI 输入**，第五条入口） | 默认档 ⇒ 回执点名 `feature-disabled` 且措辞里带 `cargo build … --features midi-input`；`midi-input` 档 ⇒ 回执**结构化**（`ok` 或非 `feature-disabled` 的错误码）、进程**不** panic。⭐ 同一份判据在**两条 CI 腿**上各跑一次（`test` 与 `test (midi-input 特性)`）|
//! | ⑤（跨线程那一跳） | 注入替身把"采集在飞"窗口拉住 ⇒ 中间态可观察、第二次触发被拒、替身读数逐字段进界面 |
//! | ⑥（**采到的音符真的进谱面**） | `the_gui_midi_in_entry_writes_the_captured_notes_into_the_score`：注入一对 note-on / note-off ⇒ 工程音符数 +1、key = 60、`start_tick`/时值按 960PPQ 对得上、`OpOrigin::MidiInput`、状态行报出写入条数；**负臂**：零条消息 ⇒ 计数不变且不产生提交 |
//!
//! ## 判据 ② 的核心断言：装载过的 SFZ 真的改变了导出的音源
//!
//! 先 `load-sfz`（成功 ⇒ `wired_to_export=yes`），再 `export-wav`：第二次导出的回执里
//! `source=` 必须带 `sfz`。这是"四个模块**接起来**了"（而不是四个独立按钮）的机械证据。
//!
//! ## 这条判据怎么变红
//!
//! - `.slint` 改了 / 删了任何一个 `accessible-id`，或 `elements.rs` 没跟上 ⇒ 判据 ① 红
//!   （`elements.rs` 的 `slint_accessible_ids_and_registry_cover_each_other` 会在
//!   **纯 Rust** 侧先红一次，两侧独立）；
//! - `host::wire_module_actions` 不装回调 / 装错 arm ⇒ 回执仍为空 ⇒ 判据 ② 红；
//! - 目标路径不读 `ModulePaths`（界面写死）⇒ 判据 ② 断言的是**那个**路径 ⇒ 红；
//! - 把 `load-sfz` 的乐器丢掉（不交给导出）⇒ 判据 ② 的 `source=…sfz…` 断言红；
//! - 错误分支 panic（`.unwrap()` / `.expect()`）⇒ 判据 ③ 在**进程级**红。
//!
//! ## 依赖从哪来（红线 6 没有被削弱）
//!
//! `yeban-ui-test-port` 是 `[dev-dependencies]`：**不进入** release 构建。
//! 核验命令（零编译）：`cargo tree -p yeban-app -e normal --locked`。
//!
//! ## 平台约束：本文件只有一个 `#[test]`，而且**只装一次** Tier-1 平台
//!
//! `slint::platform::set_platform` 是**线程局部**的，而 `Tier1Window::install` 每线程只能
//! 成功一次（第二次 `AlreadySet`）。同款用法见 `tests/production_save_ui.rs`。
//! 判据 ③ 的另一支（没有撤销会话 ⇒ `no-undo-port` / `no-project`）住在
//! `tests/ui_module_actions_no_port.rs`（另一个测试二进制 = 另一个进程）。
//!
//! ## 墙钟读数（可复跑，见 `docs/ledger/ui-modules-notes.md`）
//!
//! ```bash
//! YEBAN_UI_MODULE_TRACE=1 RUST_MIN_STACK=16777216 \
//!   target/debug/deps/ui_module_actions-<hash> --nocapture
//! ```
//!
//! 每一步打一行 `[yeban-app] ui-module-trace: action=… elapsed_ms=…`（默认关闭 ⇒
//! 产品与既有判据的确定性不受影响）。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code
// （`-D warnings` 会判红）。同款见 `tests/a11y_screen_reader.rs`。
#[allow(dead_code)]
mod live;

use std::path::{Path, PathBuf};
use std::rc::Rc;

use live::{LiveWiringOptions, build_live_ui_with};
use yeban_app::elements::{ElementKind, ElementRegistry};
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::{OpOrigin, YebanProjectV1};
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;
use yeban_ui_test_port::tree::ControlTree;

/// 测试产物落在这个临时目录里（`Drop` 时清理，**绝不污染仓库**）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        // 目录名里带进程 id 与一个 ULID：并行跑多份测试时不会互相踩。
        let dir = std::env::temp_dir().join(format!(
            "yeban-ui-module-actions-{}-{}",
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

/// 一份最小但**合法**的 SMF：一个 `note_on`（delta 0）+ 一个 `note_off`（delta 480），480 PPQ。
///
/// 为什么自己造字节而不是读 `crates/yeban-midi/tests/fixtures/*.mid`：那是**另一个 crate**
/// 的判据夹具（改它不该让本 crate 变红），而本判据要的是"音符条数 = 1"这个可精确断言的读数。
fn minimal_smf() -> Vec<u8> {
    // delta=0 note_on ch0 pitch=60 vel=100; delta=480 note_off; EndOfTrack。
    let events: &[u8] = &[
        0x00, 0x90, 60, 100, // note_on
        0x83, 0x60, 0x80, 60, 0x00, // delta 480, note_off
        0x00, 0xFF, 0x2F, 0x00, // end of track
    ];
    let mut track = Vec::new();
    track.extend_from_slice(b"MTrk");
    track.extend_from_slice(&u32::try_from(events.len()).expect("小片段").to_be_bytes());
    track.extend_from_slice(events);

    let mut file = Vec::new();
    file.extend_from_slice(b"MThd");
    file.extend_from_slice(&6u32.to_be_bytes());
    file.extend_from_slice(&0u16.to_be_bytes()); // format 0
    file.extend_from_slice(&1u16.to_be_bytes()); // 1 track
    file.extend_from_slice(&480u16.to_be_bytes()); // 480 PPQ
    file.extend_from_slice(&track);
    file
}

/// 一份最小但**合法**的 SFZ（一个区域，覆盖全部音高）。
///
/// ⚠ **不**指向真实 wav：`sfz_load` 的诚实登记写明"样本数据是内存正弦"。
/// 本判据只证"装载成功并且那份乐器被交给导出"，不证"样本被解码"
/// （后者的判据在 `src/sfz_load.rs` 自己的单元测试里）。
const MINIMAL_SFZ: &str = "<region> sample=tone.wav lokey=0 hikey=127 pitch_keycenter=60\n";

/// 回执的字段名闭集（唯一的一份：名字在这里，切分就按这里）。
///
/// 加一个字段就加一行 —— 漏加不会静默：那个字段取不到值，判据会失败并打印整条回执。
const RECEIPT_KEYS: [&str; 31] = [
    "action",
    "outcome",
    "error",
    "message",
    "path",
    "notes",
    "track",
    "clip",
    "commit",
    "origin",
    "frames",
    "pushed",
    "dropped",
    "sample_rate",
    "channels",
    "bits",
    "bytes",
    "regions",
    "sample_source",
    "source",
    "wired_to_export",
    "probe",
    // ⭐ 硬件 MIDI 输入那一条自己的字段（`midi-in` 回执）。
    "port",
    "seconds",
    "events",
    "unsupported",
    "dropped-events",
    "jitter",
    "first",
    // ⭐ 判据 ⑥ 起：落地（把采到的音符写进谱面）那一步的读数。
    // ⚠ 它们**必须**在这里：`receipt_field_end` 只认"已知名 + `=`"作边界 ⇒ 漏一个，
    // `field_of(&receipt, "first")` 这类取值会**静默多带一截**（与本注释记录的两种错法同族）。
    "landing",
    "unclosed",
];

/// 一条回执里，从 `at` 起**第一个** `:<已知字段名>=` 的冒号位置。
///
/// ## 为什么不能按"下一个冒号"切
///
/// 值里**自己带冒号**：`probe` 的值是 `note:vel:frames:energy`（实测回执：
/// `probe=60:100:23999:1153.1989`）⇒ 按 `:` 切会把 `probe` 截成 `60`。
///
/// ## 为什么也不能按"下一个**期望**字段名"切
///
/// 那是同一个陷阱的第二形态：把 `path` 的结束位置写成"我期望的下一个字段名出现处"，
/// 一旦字段顺序与期望不一致（本判据第一次跑就是：`bytes` 排在 `regions` **之前**），
/// 结果会**静默多带一截**（实测把 `path` 读成 `…/ui-instrument.sfz:bytes=62`）。
///
/// ## 实际口径
///
/// 冒号后面必须是"已知字段名紧跟 `=`"。`probe` 值里的 `:100:` / `:23999:` 后面不是
/// 这个形状 ⇒ 不切；`path` 值里就算出现 `foo=`，也只有命中**已知名 + `=`** 才算边界。
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

/// 取一个字段；缺席即判据失败（⛔ 不返回空串 —— "没找到"与"值是空的"必须可分）。
fn field_of(receipt: &str, key: &str) -> String {
    let needle = format!("{key}=");
    let start = receipt
        .find(&needle)
        .unwrap_or_else(|| panic!("回执里没有字段 `{key}`: {receipt}"))
        + needle.len();
    let end = receipt_field_end(receipt, start, key).unwrap_or(receipt.len());
    receipt[start..end].to_owned()
}

/// 判据 ① 的一半：一个语义 ID 必须**在这个活窗口的运行时控件树里**（不是只在静态注册表里）。
fn assert_addressable(tree: &ControlTree, id: &str, kind: ElementKind) {
    let node = tree.find_by_id(id).unwrap_or_else(|| {
        panic!(
            "`{id}` 不在运行时控件树里 —— 被裁掉 / 改了 accessible-id / 没接进 app.slint。\
             树里 {} 条节点",
            tree.len()
        )
    });
    assert_eq!(
        node.role.as_str(),
        kind.accessible_role(),
        "`{id}` 的角色必须是 `{}`（它在 elements.rs 里登记成 {kind:?}）",
        kind.accessible_role()
    );
}

/// 大小非零的文件（证明"真的落盘了"，⛔ 不看文件名）。
fn assert_nonempty_file(path: &Path) {
    let meta = std::fs::metadata(path)
        .unwrap_or_else(|error| panic!("目标文件没落盘 {}: {error}", path.display()));
    assert!(meta.is_file(), "{} 不是文件", path.display());
    assert!(
        meta.len() > 0,
        "{} 是 0 字节 —— 落盘失败但回执说成功",
        path.display()
    );
}

#[test]
fn the_four_module_entries_are_addressable_and_their_actions_change_the_ui_state() {
    let scratch = Scratch::new();
    let export = scratch.join("ui-export.wav");
    let import = scratch.join("ui-import.mid");
    let sfz = scratch.join("ui-instrument.sfz");
    let record = scratch.join("ui-record.wav");
    std::fs::write(&import, minimal_smf()).expect("写最小 SMF");
    std::fs::write(&sfz, MINIMAL_SFZ).expect("写最小 SFZ");

    let project: YebanProjectV1 = yeban_model::samples::filled_project();
    // 撤销端口：与产品进程同款（导入要一个**真的**会话，来源标记才会是 MidiInput）。
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据: 模块入口>", "yeban-app", project.clone(), 1)
            .expect("打开撤销会话"),
    ));

    // `YEBAN_UI_MODULE_*` 环境变量是**产品**路径上唯一的路径解析点（`ModulePaths::from_env`），
    // 因此判据驱动的是同一条解析 —— 而不是给判据开一条私有入口。
    //
    // SAFETY: 本测试二进制**只有一个** `#[test]`（见文件头的"平台约束"）⇒ 进程里不存在
    // 第二个线程读这些变量；设置发生在本函数内部、早于任何装配与任何后台线程。
    // Rust 2024 起 `set_var` 是 `unsafe fn`（"改环境变量会和 `getenv` 竞态"）。
    unsafe {
        std::env::set_var("YEBAN_UI_EXPORT_WAV_PATH", &export);
        std::env::set_var("YEBAN_UI_IMPORT_MIDI_PATH", &import);
        std::env::set_var("YEBAN_UI_LOAD_SFZ_PATH", &sfz);
        std::env::set_var("YEBAN_UI_RECORD_WAV_PATH", &record);
        std::env::set_var("YEBAN_UI_RECORD_SECONDS", "1.0");
        // ⭐ 硬件 MIDI 输入：端口 `0`；⭐ 秒数取一个**很小**的值 —— 有端口的机器上
        // 采集也立刻返回（CI 无端口 ⇒ 根本进不了采集循环）。⚠ 上下限仍由
        // `midi_in::capture_messages` 判（⛔ 判据不绕过那条规则）。
        std::env::set_var("YEBAN_UI_MIDI_IN_PORT", "0");
        std::env::set_var("YEBAN_UI_MIDI_IN_SECONDS", "0.05");
    }

    let options = LiveWiringOptions {
        permission: Permission::Interactive,
        undo: Some(Rc::clone(&port)),
        ..LiveWiringOptions::default()
    };
    // 装配顺序（刻意的）：
    //   ① `build_live_ui_with` 装 Tier-1 平台、建活窗口、**并**用产品函数
    //      `host::wire_module_actions` 接线（`ModulePaths::from_env()`）；
    //   ② 紧接着再显式接一次**同一组**路径（同一份撤销端口）—— 于是本判据的期望路径
    //      不依赖 `LiveWiringOptions` 是否将来学会传路径；两次接线输入相同，
    //      行为逐字相同（Slint 生成的 `on_*` 是 `set_handler` 可覆盖；生成代码
    //      `target/debug/build/yeban-app-*/out/app.rs` 的 `pub fn on_*` 可核）；
    //   ③ `clone_strong()` 拿一个**共享**的窗口句柄（`Rc` 语义，不是第二份窗口状态），
    //      再 `into_control_plane`（它按值消费 `LiveUi`）。这样"控制面的注册表"与
    //      "生成组件上的属性"两侧都读得到 —— 与 `tests/a11y_screen_reader.rs` 同款。
    let live_ui = build_live_ui_with(&project, &options).expect("Tier-1 平台 + 活窗口 + 四条接线");
    let midi_state = yeban_app::host::wire_module_actions(live_ui.ui(), Some(Rc::clone(&port)));
    let window = slint::ComponentHandle::clone_strong(live_ui.ui());
    let runtime_tree = live_ui.tree_snapshot();
    let mut plane = live_ui.into_control_plane(Permission::Interactive);
    let ui = &window;

    // =====================================================================
    // 判据 ①：每个新入口的 Element ID 在无头控件树里可寻址
    // =====================================================================
    // 方向 1（强）：运行时有 ⇒ 注册表有 —— 新入口**没有**未登记的 ID。
    let coverage = runtime_tree.coverage_against(plane.registry());
    assert!(
        coverage.unknown_at_runtime.is_empty(),
        "运行时树里出现了注册表没登记的语义 ID（双向契约被破坏）: {:?}",
        coverage.unknown_at_runtime
    );
    for id in [
        "module-actions-title",
        "module-export-wav-button",
        "module-import-midi-button",
        "module-load-sfz-button",
        "module-record-wav-button",
        "module-last-status",
        "module-midi-in-button",
        "module-midi-in-status",
    ] {
        // 静态注册表（`elements.rs`）与运行时树**两侧**都要有：前者 = "登记了"，
        // 后者 = "真的画出来了"。只查一侧都会漏掉一半缺陷。
        assert!(
            ElementRegistry::from_view(plane.view()).contains(id),
            "`{id}` 不在静态语义注册表里 —— UI 加了 ID 却忘了登记（[UI-TEST-001]）"
        );
    }
    for id in [
        "module-export-wav-button",
        "module-import-midi-button",
        "module-load-sfz-button",
        "module-record-wav-button",
        "module-midi-in-button",
    ] {
        assert_addressable(&runtime_tree, id, ElementKind::Button);
    }
    // 把这次寻址的**实测读数**打出来（`--nocapture` / CI 日志里能看到具体是哪几个节点，
    // ⛔ 不是只留一个"断言通过"）：运行时树规模 + 6 个 ID 的角色/标签/包围盒。
    report_line(&format!(
        "[ui-modules] 运行时控件树 {} 条；新入口 8 条如下:",
        runtime_tree.len()
    ));
    for id in [
        "module-actions-title",
        "module-export-wav-button",
        "module-import-midi-button",
        "module-load-sfz-button",
        "module-record-wav-button",
        "module-last-status",
        "module-midi-in-button",
        "module-midi-in-status",
    ] {
        let node = runtime_tree.find_by_id(id).expect("上面刚断言过");
        report_line(&format!(
            "[ui-modules]   {id} role={} label={:?} bounds={:?}",
            node.role.as_str(),
            node.label,
            node.bounds
        ));
    }
    assert_addressable(&runtime_tree, "module-actions-title", ElementKind::Text);
    assert_addressable(&runtime_tree, "module-last-status", ElementKind::Text);
    assert_addressable(&runtime_tree, "module-midi-in-status", ElementKind::Text);

    // 初始态 = **空回执**（"本次会话还没按过任何模块入口"），不是编造的读数。
    assert_eq!(
        ui.get_module_status(),
        "",
        "刚装配的状态行必须是空的 —— 任何非空值都意味着宿主在没人按之前就写了回执"
    );
    assert!(!ui.get_module_status_succeeded());
    assert_eq!(
        ui.get_module_midi_in_status(),
        "",
        "硬件 MIDI 输入的专属结果行初始也必须是空的"
    );
    assert!(!ui.get_module_midi_in_status_succeeded());

    // =====================================================================
    // 判据 ②：触发入口 ⇒ UI 状态/输出文本按预期变化
    // =====================================================================

    // ---- ②-0 导出 WAV（还没装载 SFZ ⇒ 音源是引擎乐器）----
    ui.invoke_export_wav();
    let receipt = ui.get_module_status().to_string();
    assert!(
        ui.get_module_status_succeeded(),
        "导出 WAV 应当成功: {receipt}"
    );
    assert_eq!(field_of(&receipt, "action"), "export-wav", "{receipt}");
    assert_eq!(field_of(&receipt, "outcome"), "ok", "{receipt}");
    assert_eq!(
        field_of(&receipt, "path"),
        export.display().to_string(),
        "{receipt}"
    );
    assert_eq!(field_of(&receipt, "frames"), "384000", "默认 8 秒 @48kHz");
    assert_eq!(field_of(&receipt, "sample_rate"), "48000", "{receipt}");
    assert_eq!(field_of(&receipt, "channels"), "2", "{receipt}");
    assert_eq!(field_of(&receipt, "bits"), "24", "{receipt}");
    assert!(
        !field_of(&receipt, "source").contains("sfz"),
        "还没装载 SFZ ⇒ 音源不该声称是 SFZ: {receipt}"
    );
    assert_nonempty_file(&export);
    assert_eq!(
        std::fs::metadata(&export)
            .expect("导出文件")
            .len()
            .to_string(),
        field_of(&receipt, "bytes"),
        "回执报的字节数必须等于磁盘上的字节数"
    );

    // ---- ②-1 装载 SFZ：区域数 + 一次音符探测 + `wired_to_export=yes` ----
    ui.invoke_load_sfz();
    let receipt = ui.get_module_status().to_string();
    assert!(
        ui.get_module_status_succeeded(),
        "装载 SFZ 应当成功: {receipt}"
    );
    assert_eq!(field_of(&receipt, "action"), "load-sfz", "{receipt}");
    assert_eq!(field_of(&receipt, "outcome"), "ok", "{receipt}");
    assert_eq!(
        field_of(&receipt, "path"),
        sfz.display().to_string(),
        "{receipt}"
    );
    assert_eq!(
        field_of(&receipt, "bytes"),
        std::fs::metadata(&sfz).expect("SFZ 文件").len().to_string(),
        "回执报的字节数必须等于磁盘上的字节数: {receipt}"
    );
    assert_eq!(field_of(&receipt, "regions"), "1", "最小 SFZ 只有一个区域");
    assert_eq!(
        field_of(&receipt, "wired_to_export"),
        "yes",
        "装载过的乐器必须被交给之后的导出: {receipt}"
    );
    // 探测读数形如 `note:velocity:frames:energy`，只断言形状（值本身随仪器而异）。
    let probe = field_of(&receipt, "probe");
    assert_eq!(
        probe.split(':').count(),
        4,
        "probe 形状应为 note:vel:frames:energy: {probe}"
    );

    // ---- ②-2 再导出一次：音源必须变成那份 SFZ（四个模块真的接起来了）----
    ui.invoke_export_wav();
    let receipt = ui.get_module_status().to_string();
    assert!(
        ui.get_module_status_succeeded(),
        "装载 SFZ 之后的导出应当成功: {receipt}"
    );
    assert!(
        field_of(&receipt, "source").contains("sfz"),
        "装载过 SFZ 之后，导出的音源必须是那份 SFZ（否则四个入口只是四个独立按钮）: {receipt}"
    );

    // ---- ②-3 导入 MIDI：音符进了工程、来源是 midi-input、可撤销 ----
    let commits_before = port.display().undoable;
    let project_before = port.project();
    ui.invoke_import_midi();
    let receipt = ui.get_module_status().to_string();
    assert!(
        ui.get_module_status_succeeded(),
        "导入 MIDI 应当成功: {receipt}"
    );
    assert_eq!(field_of(&receipt, "action"), "import-midi", "{receipt}");
    assert_eq!(field_of(&receipt, "outcome"), "ok", "{receipt}");
    assert_eq!(
        field_of(&receipt, "path"),
        import.display().to_string(),
        "{receipt}"
    );
    assert_eq!(field_of(&receipt, "notes"), "1", "最小 SMF 只有一个音符");
    assert_eq!(
        field_of(&receipt, "origin"),
        "midi-input",
        "导入的来源标记必须是 MidiInput（⛔ 不是 UserUi）: {receipt}"
    );
    let project_after = port.project();
    let note_count = |project: &YebanProjectV1| -> usize {
        project
            .clip_pool
            .values()
            .filter_map(|entry| entry.content.notes())
            .map(std::collections::BTreeMap::len)
            .sum()
    };
    assert_eq!(
        note_count(&project_after),
        note_count(&project_before) + 1,
        "这一次导入必须真的加了一个音符"
    );
    assert!(
        port.display().undoable > commits_before,
        "导入必须进撤销栈（可撤销是构造上的，⛔ 不是承诺）"
    );

    // ---- ②-4 录音：1 秒 @48kHz ⇒ 48000 帧，落成可回放 WAV ----
    ui.invoke_record_wav();
    let receipt = ui.get_module_status().to_string();
    assert!(ui.get_module_status_succeeded(), "录音应当成功: {receipt}");
    assert_eq!(field_of(&receipt, "action"), "record-wav", "{receipt}");
    assert_eq!(field_of(&receipt, "outcome"), "ok", "{receipt}");
    assert_eq!(
        field_of(&receipt, "path"),
        record.display().to_string(),
        "{receipt}"
    );
    assert_eq!(
        field_of(&receipt, "frames"),
        "48000",
        "`YEBAN_UI_RECORD_SECONDS=1.0`"
    );
    assert_eq!(field_of(&receipt, "dropped"), "0", "环够大 ⇒ 一帧都不该丢");
    assert_eq!(field_of(&receipt, "bits"), "24", "{receipt}");
    assert_eq!(
        field_of(&receipt, "source"),
        "in-memory-sine(440Hz)",
        "⛔ 不是声卡: {receipt}"
    );
    assert_nonempty_file(&record);
    assert_eq!(
        std::fs::metadata(&record)
            .expect("录音文件")
            .len()
            .to_string(),
        field_of(&receipt, "bytes"),
        "回执报的字节数必须等于磁盘上的字节数"
    );

    // =====================================================================
    // 判据 ④：**硬件 MIDI 输入**（第五条入口）
    // =====================================================================
    //
    // ⭐ 这条判据在**两条 CI 腿**上各跑一次（`test` 与 `test (midi-input 特性)`），
    // 用 `cfg!` 分别要求各自的形态 ⇒ 两个 feature 档都有**自己的**判据，
    // ⛔ 不会出现"特征门把判据藏起来"（默认档也必须给出**结构化**说法）。
    ui.invoke_midi_in();
    // ---- ④-a **中间态**（`outcome=pending`）：⭐ 确定性，⛔ 不靠竞态 ----
    //
    // 为什么确定：`pending` 是在 `thread::spawn` **之前**由 UI 线程同步写下的；
    // 而"落终态"只有两条路 —— ① 生产路径的 `upgrade_in_event_loop`（**Tier-1 平台没有
    // event loop proxy ⇒ 必然返回 Err**）② 调用方显式 [`apply_pending_midi_in`]。
    // 于是在本判据里，触发之后、排空之前，状态**不可能**被别人改掉。
    let receipt = ui.get_module_status().to_string();
    let midi_receipt = ui.get_module_midi_in_status().to_string();
    assert_eq!(
        midi_receipt, receipt,
        "两条状态行必须是**同一串字节**（同一个写者写一次、写两处）"
    );
    assert_eq!(field_of(&receipt, "action"), "midi-in", "{receipt}");
    assert_eq!(
        field_of(&receipt, "outcome"),
        "pending",
        "触发后必须**立刻**进入已声明的中间态: {receipt}"
    );
    assert_eq!(field_of(&receipt, "port"), "0", "{receipt}");
    assert!(
        !ui.get_module_midi_in_status_succeeded(),
        "中间态不是成功态: {receipt}"
    );
    assert_eq!(
        ui.get_module_midi_in_status_succeeded(),
        ui.get_module_status_succeeded(),
        "两条状态行的成功位也必须一致"
    );
    // ---- ④-b **不阻塞**的结构证据：中间态窗口内控件树仍可查询 ----
    //
    // ⚠ 这一条证的是"采集进行中时 UI 线程仍能响应查询"，⛔ 不是"事件循环没被占住"
    // 的直接证据（后者在无头环境里不可确定性判定，见文件头的登记）。
    {
        let (fresh, _json) = plane.plane().tree().expect("采集中控件树必须仍可读");
        for id in ["module-midi-in-button", "module-midi-in-status"] {
            assert!(
                fresh.find(id).is_some(),
                "采集中 `{id}` 必须仍在运行时树里（共 {} 条节点）",
                fresh.count
            );
        }
    }
    assert_eq!(
        ui.get_module_midi_in_status(),
        pending_snapshot(&receipt),
        "没人排空时状态行不得自己变（⇒ 中间态是一个**稳定**的窗口）"
    );
    // ---- ④-c 终态：**确定性等待**（join）＋显式排空 ----
    assert!(
        yeban_app::host::wait_for_pending_midi_in(&midi_state),
        "必须真的 join 到一个工作线程（否则这条判据的前提不成立）"
    );
    assert!(
        yeban_app::host::apply_pending_midi_in(ui, &midi_state, Some(&*port)),
        "join 之后槽里必须有一个结果可落界面"
    );
    let receipt = ui.get_module_status().to_string();
    let midi_receipt = ui.get_module_midi_in_status().to_string();
    assert_eq!(midi_receipt, receipt, "排空后两条状态行仍是同一串字节");
    assert_eq!(field_of(&receipt, "action"), "midi-in", "{receipt}");
    assert_ne!(
        field_of(&receipt, "outcome"),
        "pending",
        "排空后必须不再是中间态: {receipt}"
    );
    if cfg!(feature = "midi-input") {
        // ⭐ 开档：⭐ 端口解析 / 宿主可用性由 `midi_in` 决定 ⇒ 这里只要求
        // "结构化 + 不是那个 feature 的说法 + 码在闭集里"。
        // ⚠ 有端口的机器上允许 `ok`（真采集到事件）⇒ 判据不对硬件做假设。
        let outcome = field_of(&receipt, "outcome");
        assert!(
            outcome == "ok" || outcome == "error",
            "开档也必须给出结构化结果: {receipt}"
        );
        if outcome == "error" {
            let code = field_of(&receipt, "error");
            assert_ne!(
                code, "feature-disabled",
                "⭐ `midi-input` 档**不该**报 feature 未启用（那说明 cfg 接线反了）: {receipt}"
            );
            assert!(
                [
                    "midi-invalid-seconds",
                    "midi-port-not-found",
                    "midi-ambiguous-port",
                    "midi-host-unavailable",
                    "midi-connect-failed",
                ]
                .contains(&code.as_str()),
                "错误码必须在闭集里（⛔ 不是「未知操作」）: {receipt}"
            );
            // ⭐ 无端口时**必须列出端口清单**（空清单也要说出来）—— CI 就是这一形态。
            if code == "midi-port-not-found" {
                let message = field_of(&receipt, "message");
                assert!(
                    message.contains("没有匹配") && message.contains("MIDI 输入端口"),
                    "无端口时必须给出「无端口 + 端口清单」的结构化说法: {receipt}"
                );
            }
        }
    } else {
        // ⭐ 默认档（`midi-input` 关）：⭐ 必须点名 feature ＋ **一句怎么开**，
        // ⛔ 不许退化成"未知操作"。
        assert!(
            !ui.get_module_status_succeeded(),
            "默认档下这个入口**不该**报成功: {receipt}"
        );
        assert_eq!(field_of(&receipt, "outcome"), "error", "{receipt}");
        assert_eq!(
            field_of(&receipt, "error"),
            "feature-disabled",
            "默认档必须给出**结构化**的 feature 说法: {receipt}"
        );
        let message = field_of(&receipt, "message");
        assert!(
            message.contains("midi-input") && message.contains("--features midi-input"),
            "默认档的措辞必须点名 feature 并给出**怎么开**: {message}"
        );
    }
    // =====================================================================
    // 判据 ⑤：**注入替身**把"采集进行中"窗口**拉住** ⇒ 在飞期间的三件事都可确定判定
    // =====================================================================
    //
    // 为什么必须用替身：真入口在**无端口**的机器（CI 就是）上会**立刻**返回
    // （`PortNotFound` 在进入采集循环之前发生）⇒ "在飞"只有微秒级。
    // ⭐ 替身把窗口拉住（判据放行它才返回），于是"第二次触发被拒"这一条**不靠竞态**。
    // ⚠ 替身证的是**接线**，⛔ 不证"硬件 MIDI 能收事件"（边界写在 `MidiInCapture` 的文档里）。
    let (entered_tx, entered_rx) = std::sync::mpsc::channel::<()>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    DOUBLE
        .set(DoubleGate {
            entered: std::sync::Mutex::new(entered_tx),
            release: std::sync::Mutex::new(release_rx),
        })
        .expect("本判据只装一次门");
    let double_paths = yeban_app::host::ModulePaths {
        export_wav: scratch.join("double-never-written.wav"),
        import_midi: import.clone(),
        load_sfz: scratch.join("double-never-loaded.sfz"),
        record_wav: scratch.join("double-never-recorded.wav"),
        record_seconds: 1.0,
        midi_in_port: "double".to_owned(),
        midi_in_seconds: 0.05,
    };
    let double_state = yeban_app::host::wire_module_actions_with_capture(
        ui,
        Some(Rc::clone(&port)),
        double_paths,
        Some(double_capture),
    );

    ui.invoke_midi_in();
    // ⭐ 阻塞式等待"工作线程已进入替身"——⛔ 无 sleep、⛔ 无轮询、⛔ 无超时。
    entered_rx
        .recv()
        .expect("注入替身之后，工作线程**一定**会进入它");
    assert!(double_state.is_in_flight(), "替身还没放行 ⇒ 必须处于在飞态");
    // ① 在飞期间：第二次触发被**明确拒绝**（⛔ 不排队、⛔ 不并发）
    ui.invoke_midi_in();
    let rejected = ui.get_module_status().to_string();
    assert_eq!(field_of(&rejected, "action"), "midi-in", "{rejected}");
    assert_eq!(field_of(&rejected, "outcome"), "error", "{rejected}");
    assert_eq!(
        field_of(&rejected, "error"),
        "capture-in-progress",
        "在飞期间的第二次触发必须被明确拒绝: {rejected}"
    );
    // ② 在飞期间：控件树仍可查询（这一条与 ④-b 一起构成"UI 线程没被占死"的结构证据）
    {
        let (fresh, _json) = plane
            .plane()
            .tree()
            .expect("在飞期间控件树必须仍可读（UI 线程没被采集占死）");
        assert!(
            fresh.find("module-midi-in-button").is_some(),
            "在飞期间那个按钮必须仍在运行时树里（共 {} 条节点）",
            fresh.count
        );
    }
    // ③ 放行替身 ⇒ 结果落槽 ⇒ 确定性 join ⇒ 排空 ⇒ 断言**成功分支**的字段
    release_tx.send(()).expect("放行替身");
    assert!(
        yeban_app::host::wait_for_pending_midi_in(&double_state),
        "放行后必须能 join 到工作线程"
    );
    assert!(yeban_app::host::apply_pending_midi_in(
        ui,
        &double_state,
        Some(&*port)
    ));
    let done = ui.get_module_status().to_string();
    assert_eq!(field_of(&done, "action"), "midi-in", "{done}");
    assert_eq!(
        field_of(&done, "outcome"),
        "ok",
        "替身返回 Ok ⇒ 排空后必须是成功态: {done}"
    );
    assert_eq!(field_of(&done, "events"), "3", "替身报的事件数: {done}");
    assert_eq!(field_of(&done, "unsupported"), "1", "{done}");
    assert_eq!(
        field_of(&done, "dropped-events"),
        "4",
        "丢弃事件数必须**逐字段**来自替身读数（单位 = 事件条数、累计）: {done}"
    );
    assert_eq!(
        field_of(&done, "jitter"),
        "n=2:p50=0:p99=1:max=1",
        "抖动读数必须逐字段来自替身的**时间戳**（`CapturedMessages::report()` 是唯一装配点）: {done}"
    );
    assert_eq!(field_of(&done, "first"), "3", "{done}");
    // ⭐ **界面侧**（判据要证的那句话）：`module_actions.slint` 的
    // `module-midi-in-status` 那一行的 `text` 绑定的就是 `root.module-midi-in-status`
    // ⇒ 下面断言的是"丢弃事件数**真的**出现在界面状态行要画的字节里"，
    // ⛔ 不是只把读数留在 Rust 结构体 / `MidiInReport` 字段上。
    let midi_line = ui.get_module_midi_in_status().to_string();
    assert_eq!(
        midi_line, done,
        "两条状态行（通用回执与 MIDI 输入专属行）必须是同一串字节"
    );
    assert!(
        midi_line.contains("dropped-events=4"),
        "界面状态行必须渲染出丢弃事件数: {midi_line}"
    );
    assert!(ui.get_module_midi_in_status_succeeded(), "{done}");
    // ⭐ 成功分支的**可见**终态也写进 CI 日志（与 ④ 的那一行同款理由）。
    report_line(&format!("[ui-modules] midi-in double-path receipt={done}"));

    // ⭐ 把这一档**实际**给出的说法写进 CI 日志（`report_line` 直写 `/dev/stderr` ⇒
    // 绕过 libtest 的捕获 ⇒ 两档各自在日志里留下**可复核**的那一串字节）。
    // 为什么需要它：回执原文是**环境相关**的（开档在无端口机器上是
    // `midi-port-not-found`，在有端口的机器上会是 `ok`）⇒ 只报"断言通过"不够，
    // 读日志的人要能看见**这一档到底说了什么**。
    report_line(&format!(
        "[ui-modules] midi-in feature={} receipt={receipt}",
        if cfg!(feature = "midi-input") {
            "on"
        } else {
            "off"
        }
    ));

    // =====================================================================
    // 判据 ⑥：**硬件 MIDI 输入把采到的音符写进谱面**
    //         `the_gui_midi_in_entry_writes_the_captured_notes_into_the_score`
    // =====================================================================
    //
    // ⚠ 本文件只有一个 `#[test]`（Tier-1 平台每进程只能装一次）⇒ 判据就是**这一段**，
    // 名字经 `report_line` 与断言消息带出去（与 ④ / ⑤ 同款）。
    //
    // ⭐ 设计映射（本票的裁决；同时写进 `host::wire_module_actions` 的字段文档与提交信息）：
    //   **界面那个按钮 = CLI 的 `--midi-in` ＋ `--midi-in-write-notes`。**
    //   ⛔ 界面不加任何开关；⛔ CLI 自己的默认值一位不动（`--midi-in` 单给仍是只读诊断）。
    //
    // 采集入口用**注入替身**（与 ⑤ 同款）：无硬件也能判，且"采到什么"由判据说了算。
    // 目标音轨规则与 CLI 的 `--midi-in-write-notes` **同一条**（第一条非 master 音轨）：
    // 它住在 `crate::import_midi::destination`（`midi_notes` 的写入原语转调它）。
    let landing_paths = yeban_app::host::ModulePaths {
        export_wav: scratch.join("landing-never-written.wav"),
        import_midi: import.clone(),
        load_sfz: scratch.join("landing-never-loaded.sfz"),
        record_wav: scratch.join("landing-never-recorded.wav"),
        record_seconds: 1.0,
        midi_in_port: "landing".to_owned(),
        midi_in_seconds: 0.05,
    };
    let landing_state = yeban_app::host::wire_module_actions_with_capture(
        ui,
        Some(Rc::clone(&port)),
        landing_paths.clone(),
        Some(landing_capture),
    );
    let count_of_pitch = |project: &YebanProjectV1, pitch: u8| -> usize {
        project
            .clip_pool
            .values()
            .filter_map(|entry| entry.content.notes())
            .flat_map(|notes| notes.values())
            .filter(|note| note.pitch == pitch)
            .count()
    };
    let notes_60 = |project: &YebanProjectV1| -> Vec<(u64, u64)> {
        project
            .clip_pool
            .values()
            .filter_map(|entry| entry.content.notes())
            .flat_map(|notes| notes.values())
            .filter(|note| note.pitch == 60)
            .map(|note| (note.start_tick, note.duration_ticks))
            .collect()
    };
    let before_notes = note_count(&port.project());
    let before_60 = count_of_pitch(&port.project(), 60);
    ui.invoke_midi_in();
    assert!(
        yeban_app::host::wait_for_pending_midi_in(&landing_state),
        "必须真的 join 到采集替身（否则这一条的前提不成立）"
    );
    assert!(
        yeban_app::host::apply_pending_midi_in(ui, &landing_state, Some(&*port)),
        "join 之后槽里必须有一个结果可落界面"
    );
    let after_landing = port.project();
    assert_eq!(
        note_count(&after_landing),
        before_notes + 1,
        "判据 the_gui_midi_in_entry_writes_the_captured_notes_into_the_score: \
         按一下界面上的硬件 MIDI 输入，采到的 note-on / note-off 必须**真的**变成谱面上的一条音符"
    );
    assert_eq!(
        count_of_pitch(&after_landing, 60),
        before_60 + 1,
        "新增的那一条必须是 key 60"
    );
    assert!(
        notes_60(&after_landing).contains(&(0, 480)),
        "note-on(t=0) + note-off(t=500ms) ⇒ start_tick=0、duration=480 tick\
         （500ms @960PPQ = 480）；实得 {:?}",
        notes_60(&after_landing)
    );
    assert_eq!(
        port.last_commit_origin(),
        Some(OpOrigin::MidiInput),
        "这一次写入的来源标记必须是 MidiInput（⛔ 不是 UserUi）"
    );
    let landing_receipt = ui.get_module_status().to_string();
    assert!(
        landing_receipt.contains("notes=1"),
        "状态行必须报出**写进去的条数**: {landing_receipt}"
    );
    assert_eq!(
        ui.get_module_midi_in_status(),
        slint::SharedString::from(landing_receipt.clone()),
        "两条状态行仍是同一串字节"
    );
    report_line(&format!(
        "[ui-modules] criterion=the_gui_midi_in_entry_writes_the_captured_notes_into_the_score \
         receipt={landing_receipt}"
    ));

    // ---- 负臂：注入**零条**消息 ⇒ 音符条数一位不变（也⛔ 不产生提交）----
    let zero_state = yeban_app::host::wire_module_actions_with_capture(
        ui,
        Some(Rc::clone(&port)),
        landing_paths,
        Some(empty_capture),
    );
    let before_zero = note_count(&port.project());
    let commits_before_zero = port.display().undoable;
    ui.invoke_midi_in();
    assert!(
        yeban_app::host::wait_for_pending_midi_in(&zero_state),
        "负臂也必须有工作线程可 join"
    );
    assert!(
        yeban_app::host::apply_pending_midi_in(ui, &zero_state, Some(&*port)),
        "负臂的结果同样必须落到界面"
    );
    assert_eq!(
        note_count(&port.project()),
        before_zero,
        "零条消息 ⇒ 一个音符都不得写（负臂）"
    );
    assert_eq!(
        port.display().undoable,
        commits_before_zero,
        "零条消息 ⇒ 不得产生撤销提交（负臂）"
    );
    let zero_receipt = ui.get_module_status().to_string();
    assert!(
        zero_receipt.contains("notes=0"),
        "负臂的状态行必须如实报 0 条: {zero_receipt}"
    );
    report_line(&format!(
        "[ui-modules] criterion=the_gui_midi_in_entry_writes_the_captured_notes_into_the_score \
         negative-arm receipt={zero_receipt}"
    ));

    // =====================================================================
    // 判据 ③：错误路径 ⇒ UI 显示结构化错误而 **不 panic**
    // =====================================================================
    // SFZ 源**不存在**：走**同一个**活窗口，换一条不存在的来源
    // （`wire_module_actions_with` 是 `wire_module_actions` 的显式参数版，同一条实现）。
    let missing = scratch.join("does-not-exist.sfz");
    yeban_app::host::wire_module_actions_with(
        ui,
        Some(Rc::clone(&port)),
        yeban_app::host::ModulePaths {
            export_wav: scratch.join("never-written.wav"),
            import_midi: import.clone(),
            load_sfz: missing.clone(),
            record_wav: scratch.join("never-recorded.wav"),
            record_seconds: 1.0,
            midi_in_port: "0".to_owned(),
            midi_in_seconds: 0.05,
        },
    );
    ui.invoke_load_sfz();
    let receipt = ui.get_module_status().to_string();
    assert!(
        !ui.get_module_status_succeeded(),
        "SFZ 不存在时**不得**报成功: {receipt}"
    );
    assert_eq!(field_of(&receipt, "action"), "load-sfz", "{receipt}");
    assert_eq!(field_of(&receipt, "outcome"), "error", "{receipt}");
    assert_eq!(
        field_of(&receipt, "error"),
        "sfz-load",
        "稳定错误码: {receipt}"
    );
    assert!(
        receipt.contains(&missing.display().to_string()),
        "错误回执必须点名是哪一条路径: {receipt}"
    );
    assert!(
        !scratch.join("never-written.wav").exists(),
        "装载失败不得碰导出目标"
    );

    // 错误臂之后**只读**的那条仍然可用：接线没被前一条错误弄坏。
    ui.invoke_record_wav();
    let receipt = ui.get_module_status().to_string();
    assert!(
        ui.get_module_status_succeeded(),
        "错误臂之后只读动作仍必须可用: {receipt}"
    );
    assert_eq!(field_of(&receipt, "action"), "record-wav", "{receipt}");
    assert_eq!(field_of(&receipt, "outcome"), "ok", "{receipt}");
    assert_nonempty_file(&scratch.join("never-recorded.wav"));

    // SAFETY: 同上 —— 单 `#[test]` 的进程，且此刻没有任何后台线程（Tier-1 平台不建线程）。
    // 收尾：还原环境变量（显式比依赖进程退出干净）。
    unsafe {
        for key in [
            "YEBAN_UI_EXPORT_WAV_PATH",
            "YEBAN_UI_IMPORT_MIDI_PATH",
            "YEBAN_UI_LOAD_SFZ_PATH",
            "YEBAN_UI_RECORD_WAV_PATH",
            "YEBAN_UI_RECORD_SECONDS",
            "YEBAN_UI_MIDI_IN_PORT",
            "YEBAN_UI_MIDI_IN_SECONDS",
        ] {
            std::env::remove_var(key);
        }
    }
}

/// ⭐ 判据用**替身**的两道门（通道同步 ⇒ ⛔ 无 sleep、⛔ 无轮询、⛔ 无超时）。
#[derive(Debug)]
struct DoubleGate {
    /// 工作线程进入替身时发一条。
    entered: std::sync::Mutex<std::sync::mpsc::Sender<()>>,
    /// 判据放行时收一条（替身阻塞在这里）。
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

/// ⭐ 判据只装一次门（本文件只有一个 `#[test]`）。
static DOUBLE: std::sync::OnceLock<DoubleGate> = std::sync::OnceLock::new();

/// ⭐ **受判据控制**的采集入口：进来先报到，再等放行，然后返回一份**已知**的读数。
///
/// ⚠ 它是 [`yeban_app::host::MidiInCapture`] 的一个实现 —— 与真入口**同一个签名**，
/// ⛔ 不是第二份采集实现（它一行采集逻辑都没有）。
fn double_capture(
    _port: &str,
    _seconds: f32,
) -> Result<yeban_app::midi_in::CapturedMessages, yeban_app::midi_in::MidiInError> {
    let gate = DOUBLE.get().expect("判据必须先装门");
    gate.entered
        .lock()
        .expect("entered 锁")
        .send(())
        .expect("报到");
    gate.release
        .lock()
        .expect("release 锁")
        .recv()
        .expect("等放行");
    // ⚠ 夹具的形状从"报告"换成了**全量消息**（`apply_pending_midi_in` 现在经
    // `CapturedMessages::report()` 装配那一行状态文本）：⭐ `events` / `first` /
    // `jitter` **全部**由这一批消息推出 ⇒ 时间戳在这里是**承重**的（选 [0,3000,8000]
    // 微秒 ⇒ 间隔 [3000,5000] ⇒ 中位 5000 ⇒ 偏差 [2000→1 tick, 0] ⇒ `n=2:p50=0:p99=1:max=1`）。
    // ⭐ 3 条消息里有**一对音符** ⇒ 这一次排空**也**会真的写 2 条音符进工程
    // （落地路径与判据 ⑥ 同一条；⑤ 只断言读数与接线，落地的结构证据在 ⑥）。
    Ok(yeban_app::midi_in::CapturedMessages {
        origin_us: 0,
        seconds: 0.05,
        messages: vec![
            (0, 0x90, 60, 100),
            (3_000, 0x90, 62, 100),
            (8_000, 0x80, 60, 0),
        ],
        unsupported: 1,
        // ⭐ 本字面量是**夹具**：`dropped_events` 的读数必须**真的进 UI 回执**
        // （`src/host.rs` 的 `apply_pending_midi_in` 把它写成 `dropped-events=`，
        // 界面状态行的 `text` 直接画那一串）⇒ 这里给一个**非零、且与
        // `events`(3) / `unsupported`(1) / `first`(3) 都不同**的值：
        // 一旦把它误接到别的读数上，上面的 `dropped-events` 断言立刻变红。
        dropped_events: 4,
    })
}

/// ⭐ 判据 ⑥ 的注入替身：一次**已知**的采集 —— 一个 C4 按下（t=0）＋ 500ms 后松开。
///
/// ⚠ 它与 [`double_capture`] 同款：是 `MidiInCapture` 的一个实现（⛔ 不是第二份采集逻辑），
/// 只是**不拉门**（这一条不要"在飞"窗口，要的是采到的东西真的落进谱面）。
fn landing_capture(
    _port: &str,
    _seconds: f32,
) -> Result<yeban_app::midi_in::CapturedMessages, yeban_app::midi_in::MidiInError> {
    Ok(yeban_app::midi_in::CapturedMessages {
        origin_us: 0,
        seconds: 0.5,
        messages: vec![(0, 0x90, 60, 100), (500_000, 0x80, 60, 0)],
        unsupported: 0,
        dropped_events: 0,
    })
}

/// ⭐ 判据 ⑥ **负臂**的注入替身：采集成功，但**一条消息都没有**。
fn empty_capture(
    _port: &str,
    _seconds: f32,
) -> Result<yeban_app::midi_in::CapturedMessages, yeban_app::midi_in::MidiInError> {
    Ok(yeban_app::midi_in::CapturedMessages {
        origin_us: 0,
        seconds: 0.05,
        messages: Vec::new(),
        unsupported: 0,
        dropped_events: 0,
    })
}

/// 把已经写下的那一串回执**原样**取回来（用于断言"没人排空时它不得自己变"）。
fn pending_snapshot(receipt: &str) -> slint::SharedString {
    slint::SharedString::from(receipt.to_owned())
}
