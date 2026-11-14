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

#![allow(unused)]
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
/// ⚠ **不**指向真实 wav：`sfz_load` 的诚实登记写明"读不到就**如实回退**到内存正弦"。
/// 本判据只证"装载成功并且那份乐器被交给导出"，不证"样本被解码"
/// （后者的判据在 `src/sfz_load.rs` 自己的单元测试里）。
const MINIMAL_SFZ: &str = "<region> sample=tone.wav lokey=0 hikey=127 pitch_keycenter=60\n";

/// 回执的字段名闭集（唯一的一份：名字在这里，切分就按这里）。
///
/// 加一个字段就加一行 —— 漏加不会静默：那个字段取不到值，判据会失败并打印整条回执。
const RECEIPT_KEYS: [&str; 38] = [
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
    // ⭐ 任务单 `#229`：GUI 导出的**削波提示**（只在削波时出现；值是整段带引号的
    // `WavExportReport::clipping_note()`）。它在 `digest` **之后** ⇒ 不登记的话
    // `field_of(&receipt, "digest")` 会静默多带一截。
    "clipping_note",
    // ⭐ `D-EX-1`：GUI 导出回执里**恒发**的四条峰值读数（开还是关都发，位置在
    // `digest` 与 `clipping_note` 之间）。同上一条的理由：不登记 ⇒ 它们会被
    // `field_of(&receipt, "source")` 这类取值**静默多带一截**（本文件自己的规则：
    // "加一个字段就加一行"）。
    "limiter",
    "limiter_reductions",
    "true_peak_dbfs",
    "headroom_db",
    // ⭐ 任务单 `#319`：GUI 导入回执末尾的两个损失字段（只挂 import-midi 的成功臂）。
    // 它们在 `origin` **之后** ⇒ 不登记的话 `field_of(&receipt, "origin")` 会静默
    // 多带一截（`loss_detail` 的值里还有 `<kind>-read=`，但**没有** `:` ⇒ 切分只按
    // 上面那套"已知字段名 + `=`"的边界）。
    "losses",
    "loss_detail",
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

/// 一条回执的**顶层**字段取值（引号感知，`#321`）；缺席返回 `None`。
///
/// 字段名只认**顶层**位置（`key=` 必须紧跟一个**顶层** `:`；引号内的 `:` 不算起点）：
/// 引号值跳到**第一个闭合引号**（引号内的 `channels=` 等一律不是字段名），普通值仍按
/// [`receipt_field_end`] 的"已知字段名 + `=`"边界取（值里可自带 `:`，如 `probe`）。
/// 引号值**保留引号**（本文件既有约定）。
///
/// ## 引号里的 `"`（写死的契约）
///
/// 回执没有转义 ⇒ 第一个 `"` 即闭合引号；值里真带 `"` 时其后的字段取不到（`None`），
/// 由调用方按"缺席即失败"处理。
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

/// 取一个字段；缺席即判据失败（⛔ 不返回空串 —— "没找到"与"值是空的"必须可分）。
fn field_of(receipt: &str, key: &str) -> String {
    field_of_opt(receipt, key).unwrap_or_else(|| panic!("回执里没有字段 `{key}`: {receipt}"))
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
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
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
        aether_app.contains("callback import-midi("),
        "新 UI 契约面必须声明 callback import-midi("
    );
    assert!(
        aether_host.contains("on_import_midi"),
        "宿主接线层必须注册 on_import_midi"
    );
    assert!(
        aether_app.contains("callback load-sfz("),
        "新 UI 契约面必须声明 callback load-sfz("
    );
    assert!(
        aether_host.contains("on_load_sfz"),
        "宿主接线层必须注册 on_load_sfz"
    );
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

/// `#321` 负向判据：**引号内的键 ⛔ 可被顶层取到**（本文件的取值器也必须引号感知）。
///
/// 修前：`field_of("channels")` 会命中 `loss_detail` 引号内的 `channels=`，取到
/// `2"`（把闭合引号也带进值里）而不是"缺席"。修后必须是 `None`，而 `loss_detail`
/// 仍是整串（引号内的 `=` 不切）。
#[test]
fn a_quoted_value_does_not_expose_its_inner_keys_to_the_top_level() {
    let receipt = concat!(
        r#"module:action=import-midi:outcome=ok:losses="…""#,
        r#":loss_detail="format=1 channels=2""#,
        r#":lyrics_detail="lyrics=2 attached=1 dropped=1 non-utf8=0 empty=0""#
    );
    assert_eq!(field_of_opt(receipt, "channels"), None);
    assert_eq!(field_of_opt(receipt, "format"), None);
    assert_eq!(field_of(receipt, "loss_detail"), "\"format=1 channels=2\"");
    assert_eq!(
        field_of(receipt, "lyrics_detail"),
        "\"lyrics=2 attached=1 dropped=1 non-utf8=0 empty=0\""
    );
}
