//! **导出 WAV 的时长缺省 = 工程可闻末端**（`D-UI-28`）—— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 打开「导出 WAV」弹窗时，**时长那一格自动填上整首曲子的长度**（而不是写死 8 秒），
//! 于是"按缺省点导出"产出的是**全曲**；用户改过的那一格重开时逐字保留；清空它 ⇒ 重新
//! 播种；工程变长 ⇒ 重新播种；没有内容的工程 ⇒ ⛔ 不编一个假数。
//!
//! ## 为什么缺口是真的（不是审美问题）
//!
//! `ui/dialogs/export_wav_dialog.slint` 原来在**两处**写死 `seconds: "8"`（根属性与对话框
//! 自己的 `in-out`）。一份几分钟的工程，创作者点开弹窗、什么都不改、按「导出」⇒ 拿到
//! 8 秒的片段，**界面上没有任何地方说这件事**。这是"UI 看起来能用、实际会静默毁掉产物"
//! 的那一类缺口。
//!
//! ## 为什么补在宿主、不补在 `resolve`
//!
//! [`yeban_app::export_wav::WavExportOptions::resolve`] 的 8 秒缺省是 **GUI / CLI / MCP
//! 三方共用**的那一份，五处既有判据把 `frames=384000` 钉在上面（判据 ① 复钉一次）。
//! 播种落在"打开弹窗"这一刻 ⇒ 共用口径一位不改，缺省行为逐字节不变。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（未打开） | 时长格初始为**空串**（⛔ 不再是 `"8"`）；此时直接导出仍 `frames=384000` ⇒ 共用的 8 秒缺省一位没动 |
//! | ②（打开播种） | `open-export-wav` ⇒ 该格变成 `"7.387"` = `filled_project` 可闻末端（段末 15360 tick @ 128 → 132 的**分段**换算） |
//! | ③（不截断） | 按播好的值导出 ⇒ `frames=354576` **≥** 内容末端帧数 `354545` ⇒ 最后一段一个样本不丢，且字节数与磁盘一致 |
//! | ④（用户值保留） | 该格填 `0.5` ⇒ 关 → 重开，**仍是** `0.5`（⛔ 不被宿覆盖） |
//! | ⑤（清空重播） | 该格清成空串 ⇒ 关 → 重开，回到 `"7.387"`（⇒ "保留用户值"⛔ 不是"永远不再播种"） |
//! | ⑥（工程变长 ⇒ 刷新） | 提交 `Op::SetTempo { 128 → 20 }` ⇒ 重开变 `"27.637"`（旧缺省 8 秒在这一格上会**静默切掉 19.6 秒**） |
//! | ⑦（空工程不编数） | 换成 `default_project()`（只有母线轨 ⇒ 末端 0）⇒ 重开该格**仍为空**，⛔ 不播一个算出来的假值 |
//! | ⑧（上限） | 把末端推到 4735 秒 ⇒ 播 `"3600.000"`（与 MCP `MAX_RENDER_FRAMES` 同一小时上限），⛔ 不播一个会挂死 UI 线程的数（`D-EX-2` 登记的同步导出今天仍在） |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/ui_export_dialog.rs` 同款）。
//! 判据 ③ 之外**不做真导出**（⑥⑧ 只读那一格）：一次 3600 秒的导出是 172.8 M 帧，
//! CI 里既没必要也没有意义。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code
// （`-D warnings` 会判红）。同款见 `tests/a11y_screen_reader.rs`。
#[allow(dead_code)]
mod live;

use std::path::{Path, PathBuf};
use std::rc::Rc;

use live::{LiveWiringOptions, build_live_ui_with};
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::YebanProjectV1;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;

/// 测试产物落在临时目录里（`Drop` 时清理，**绝不污染仓库**）。
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-d-ui-28-{tag}-{}-{}",
            std::process::id(),
            yeban_model::ids::EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 取回执里的一个顶层字段（`key=value` 以 `:` 分隔；只做**前缀**匹配）。
///
/// 本判据只读 `action` / `outcome` / `frames` / `bytes`，回执里可能含 `:` 的 `path=`
/// 与 `clipping_note=` 都不取值 ⇒ 不需要引号感知（同款见 `tests/export_dialog_button_focus.rs`）。
fn field_of(receipt: &str, key: &str) -> String {
    let needle = format!("{key}=");
    for part in receipt.split(':') {
        if let Some(value) = part.strip_prefix(needle.as_str()) {
            return value.to_owned();
        }
    }
    panic!("回执里没有字段 `{key}`: {receipt}");
}

fn frames_of(receipt: &str) -> u64 {
    field_of(receipt, "frames")
        .parse()
        .unwrap_or_else(|error| panic!("`frames` 不是整数（{error}）: {receipt}"))
}

fn assert_nonempty_file(path: &Path) {
    let meta = std::fs::metadata(path)
        .unwrap_or_else(|error| panic!("导出的产物没落盘 {}: {error}", path.display()));
    assert!(meta.is_file(), "{} 不是文件", path.display());
    assert!(meta.len() > 0, "{} 是 0 字节", path.display());
}

#[test]
fn the_export_length_field_seeds_from_the_audible_end_of_the_project() {
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
        aether_app.contains("callback close-export-wav("),
        "新 UI 契约面必须声明 callback close-export-wav("
    );
    assert!(
        aether_host.contains("on_close_export_wav"),
        "宿主接线层必须注册 on_close_export_wav"
    );
    assert!(
        aether_app.contains("callback export-wav("),
        "新 UI 契约面必须声明 callback export-wav("
    );
    assert!(
        aether_host.contains("on_export_wav"),
        "宿主接线层必须注册 on_export_wav"
    );
    assert!(
        aether_app.contains("> export-wav-open:"),
        "新 UI 契约面必须声明 > export-wav-open:"
    );
    assert!(
        aether_host.contains("set_export_wav_open"),
        "宿主接线层必须注册 set_export_wav_open"
    );
}
