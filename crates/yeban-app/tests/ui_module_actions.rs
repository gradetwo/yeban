//! **四个模块入口的 UI 接线** —— 端到端判据（`cargo test -p yeban-app --all-targets` 会跑它）。
//!
//! ## 本文件要证的句子
//!
//! `crates/yeban-app/ui/module_actions.slint` 的四个按钮**不是画上去的控件**：
//! 每一个都有稳定语义 ID、都在活窗口的运行时控件树里可寻址，按下去**真的**走到
//! `export_wav` / `import_midi` / `sfz_load` / `record_wav` 四个**已经存在**的模块实现，
//! 并把**结构化回执**写回界面（成功与失败都写）。
//!
//! ## 三条判据（对应交付要求的 a / b / c）
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（可寻址） | 四个入口按钮 + 小节标题 + 状态行共 6 个语义 ID 都**在运行时控件树里**，角色符合登记；初始回执是**空**（不是一个编造的读数） |
//! | ②（状态按预期变） | 按下入口 ⇒ `MainWindow.module-status` 变成结构化回执，字段（路径 / 帧数 / 采样率 / 区域数 / 来源 / `origin=midi-input`）逐条对上；目标文件真的落盘、字节数与回执一致 |
//! | ③（错误路径不 panic） | SFZ 源不存在 ⇒ 回执是 `outcome=error` + 稳定错误码，进程**不** panic，**不留半成品** |
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
use yeban_model::YebanProjectV1;
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
const RECEIPT_KEYS: [&str; 22] = [
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
    yeban_app::host::wire_module_actions(live_ui.ui(), Some(Rc::clone(&port)));
    let window = slint::ComponentHandle::clone_strong(live_ui.ui());
    let runtime_tree = live_ui.tree_snapshot();
    let plane = live_ui.into_control_plane(Permission::Interactive);
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
    ] {
        assert_addressable(&runtime_tree, id, ElementKind::Button);
    }
    // 把这次寻址的**实测读数**打出来（`--nocapture` / CI 日志里能看到具体是哪几个节点，
    // ⛔ 不是只留一个"断言通过"）：运行时树规模 + 6 个 ID 的角色/标签/包围盒。
    report_line(&format!(
        "[ui-modules] 运行时控件树 {} 条；新入口 6 条如下:",
        runtime_tree.len()
    ));
    for id in [
        "module-actions-title",
        "module-export-wav-button",
        "module-import-midi-button",
        "module-load-sfz-button",
        "module-record-wav-button",
        "module-last-status",
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

    // 初始态 = **空回执**（"本次会话还没按过任何模块入口"），不是编造的读数。
    assert_eq!(
        ui.get_module_status(),
        "",
        "刚装配的状态行必须是空的 —— 任何非空值都意味着宿主在没人按之前就写了回执"
    );
    assert!(!ui.get_module_status_succeeded());

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
        ] {
            std::env::remove_var(key);
        }
    }
}
