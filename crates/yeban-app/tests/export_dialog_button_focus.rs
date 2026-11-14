//! `D-UI-20`（`[UI-A11Y-003]` §7.3 第 1 条的第二半）：导出弹窗里那三格**手绘控件**
//! 第一次摸得到键盘，并且按下去真的走到底 —— 加上「`Return` = 默认动作」。
//!
//! ## 这一票修的是哪个真实缺陷
//!
//! `D-UI-19` 让模态持住了键盘，但轮转只在**四个 `TextInput`** 之间：✕、「导出」、
//! 限制器开关三格是 `Rectangle` + `TouchArea`，**没有** `FocusScope` ⇒ 收不到焦点
//! ⇒ 只用键盘的人填得完四个框，却⛔ **按不动导出**。规范那句"全键盘无障碍"在这一格
//! 是字面失败的，而且失败得很安静：界面看起来一切正常，只是键盘走到底就没了。
//!
//! 现在每格里加一枚同尺寸、**零视觉**的 `FocusScope`（⛔ 不画焦点描边 —— 见下面的边界），
//! `Return` / `Space` 触发的是**既有那一条**回调；`Tab` 轮转从四格扩到**七格**。
//! 另外四个文本框接上 `accepted => { root.run(); }` ⇒ `Return` 即默认动作。
//!
//! ## 为什么这两件事能同时成立（实测，Slint 1.18.1）
//!
//! | 事实 | 出处 |
//! | :--- | :--- |
//! | `FocusScope::input_event_filter_before_children` 恒 `ForwardEvent` | `items/input_items.rs:657-665` |
//! | `focus-on-click: false` ⇒ `input_event` 对 `Pressed` 返回 `EventIgnored`（只有 `enabled && focus-on-click && !has_focus` 才吞） | 同文件 667-688 |
//! | ⇒ 这枚容器**不与 `TouchArea` 抢命中**；`TouchArea` 又排在它之后（更上层），鼠标那条路一位不变 | — |
//! | `.focus()` ⇒ `set_focus_item(item, true, FocusReason::Programmatic)`，而 `focus_event` 只对 `TabNavigation` / `PointerClick` 两种理由设闸 ⇒ **程序取焦恒成立** | `i-slint-compiler` 的 `generator/rust.rs:4541` ＋ `input_items.rs:780-793` |
//! | `Key.Return` = `\u{000a}`、`Key.Space` = `\u{0020}`；`Key` 是**内建命名空间**，`.slint` 里无需 import | `i-slint-common/key_codes.rs:34,54` ＋ `i-slint-compiler/lookup.rs:1094` |
//! | 注入侧同码位：`KeyCode::Return → '\n'`、`KeyCode::Space → ' '` | `crates/yeban-ui-test-port/src/port.rs:343-348,435` |
//! | `TextInput.accepted` 仅在**非只读 + 单行**时触发（四框都满足），触发即吃掉这一键 ⇒ ⛔ 不会既打字又导出 | `i-slint-core/items/text.rs:1068-1074` |
//!
//! ## 判据（一条链，九个读数 ＋ 一条出处纪律）
//!
//! ① 起点：弹窗关着 ⇒ 七格读数 `-1`、快捷键层持焦、目标文件不存在、状态行空；
//! ② 打开（宿主入口）⇒ 读数 `0`、四框读数 `0`、快捷键层失去焦点；
//! ③ `Tab` ×4 ⇒ `1→2→3→4`：第 4 格是**限制器**（`D-UI-19` 之后新加的第一格按钮），
//!    此时四框读数如实为 `-1`（焦点不在文本框里），期间左栏搜索框从未持焦、
//!    画布工具一位不动；
//! ④ 焦点在限制器格上按 `Space` ⇒ `export-wav-limiter` 翻真（开关真的被键盘拨动了），
//!    且焦点**不移动**（激活≠换格）；
//! ⑤ 再 `Tab` ⇒ 第 5 格「导出」；按 `Return` ⇒ **落盘一个真 WAV**：回执
//!    `action=export-wav` / `outcome=ok` / `limiter=on` / `frames>0`，文件非空。
//!    ⚠ 同时钉住两条既有事实：弹窗**不关**（`set_export_wav_open(` 全仓只有两个写者，
//!    导出这条路不在其中）、焦点仍在那一格；
//! ⑥ 再 `Tab` ⇒ 第 6 格 ✕；按 `Return` ⇒ 走的是**既有的** `close()`：弹窗关、两个读数
//!    都回 `-1`、焦点复位快捷键层、弹窗离开运行时树；
//! ⑦ 重开 ⇒ `0`；Backtab ×1 ⇒ `6`（第一格再退 = 末格，反向也闭合）、再退 ×2 ⇒ `5→4`；
//! ⑧ 退到第 3 格（时长框）后由 Rust 改 `seconds = "1"`，按 `Return` ⇒ `accepted` 走
//!    同一条 `run()`：新回执的 `frames` **严格小于** ⑤（改了的那一格真的被消费了，
//!    ⛔ 不是拿上一条回执自比），且框内文本逐字仍是 `1`（`Return` ⛔ 没被打进文本），
//!    弹窗仍开着；
//! ⑨ 焦点在按钮格上按一个 DAW 语义键（`1`）⇒ 工具不动、文本框不变、状态行不变
//!    ——「模态持键盘」对按钮格同样成立。
//!
//! ⑩ 出处纪律（第二条 `#[test]`，⛔ 不开窗口）：七格的「序号 → 控件」表**只有一份**
//!    （`.focus()` 调用点恒 8 = 首格 1 + 表 7）、三格按钮的键盘落点与它们的
//!    `TouchArea` / 无障碍动作**是同一条回调**（`root.run()` 出现 7 处，全指向同一条
//!    `callback run()`）、⛔ 不新增可寻址控件（`accessible-id:` 恒 10 ⇒ 默认帧与
//!    `tests/golden/linux/` 的 5 张基准一位不改）、两个焦点读数都仍是**推导**。
//!
//! ## 已知边界（如实登记，⛔ 不假装覆盖）
//!
//! * **焦点落在按钮格上看不见**。三格容器刻意不画描边：画了就改像素，而弹窗打开态
//!   牵动两条已判绿的视觉基准 ⇒ 改基准属人类/集成者裁决（`docs/ledger/` §32 边界 2）。
//!   ⇒ 今天键盘用户**走得**到这三格、**按得**动，只是看不出站在哪一格。
//! * **鼠标点击不改变焦点**（三格都 `focus-on-click: false`）⇒ 这一票⛔ 没有测鼠标：
//!   `LiveUi` 只暴露按键注入（指针注入在 `LiveControlPlane` 上，而本判据需要的是
//!   **带焦点**的按键通道）。"容器不抢命中"是**从上游源码**证的（上表前三行），
//!   ⛔ 不是从运行时证的。把它连同 `D-UI-21`（遮罩不吞鼠标）一并重判 ⇒ 另票。
//! * `Shift+Tab` 在真键盘上以 Backtab `\u{19}` 到达，本判据注入的就是那个码位；
//!   trap 里按修饰位判的 `Tab`+`shift` 分支仍⛔ 注入不出来（与 `D-UI-19` 同边界）。
//! * 三格各自的 `FocusScope` **不参与全局轮转**：`Tab` 一律先被 trap 吞掉再自己分配，
//!   所以 Slint 内建那条整窗链根本没机会跑（`focus-on-tab-navigation: false` 在 trap 上）。
//!
//! ## 判别力（改动 ⇒ 红；缺行就说明那条断言本来就是空的）
//!
//! | 若改成 | 本判据 |
//! | :--- | :--- |
//! | 删掉限制器格那枚 `FocusScope` | ③ 红（`Tab` 到不了 `4`，直接绕回 `0`）、④⑤ 随之全红 |
//! | 把 `key-pressed` 里的 `Key.Space` 那一半去掉 | ④ 红（`Space` 拨不动开关；`Return` 仍有效 ⇒ 只红在开关那一步） |
//! | 按钮格的键盘落点写成"另开一份导出"（例如直接 `root.close()`） | ⑤ 红（没有 `outcome=ok` 回执 / 文件不落盘），⑩ 红（`root.run()` 计数变化） |
//! | 删掉四框任一条 `accepted` | ⑧ 红（`Return` 变成打字或不做事，`frames` 不更新） |
//! | 「导出」之后再 `set_export_wav_open(false)`（顺手关弹窗） | ⑤ 红（钉的是**既有**行为：导出从不关弹窗）—— ⛔ 这条不是放宽，是把事实写对；真要改成"导出即关闭"是另一票界面决定 |
//! | 轮转表在别处再抄一遍（多出第 9 个 `.focus()`） | ⑩ 红 |
//! | 给某个 `FocusScope` 加 `accessible-id` | ⑩ 红（10 ⇒ 11），且金帧/寻址面基准要重录 —— 本票刻意不碰 |
//! | 把 `focus-on-click` 打开 | 本判据⛔ 测不到（不测鼠标）⇒ 如实登记在边界里，不是覆盖到了 |

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的装配入口 ⇒ 其余项在本测试目标里是 dead_code。
#[allow(dead_code)]
mod live;

use live::{LiveWiringOptions, build_live_ui_with};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::port::{KeyCode, Permission};
use yeban_ui_test_port::render::report_line;

/// 七格的语义顺序（⛔ 不是新的状态，只是给判据一个可读的名字）。
const PATH: i32 = 0;
const SAMPLE_RATE: i32 = 1;
const BIT_DEPTH: i32 = 2;
const SECONDS: i32 = 3;
const LIMITER: i32 = 4;
const RUN: i32 = 5;
const CLOSE: i32 = 6;

/// 临时目录：`Drop` 里清掉，判据不留垃圾文件。
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("yeban-d-ui-20-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 取回执里的一个顶层字段（`key=value` 以 `:` 分隔）。
///
/// ⚠ 只做**前缀**匹配，因此 `limiter=` 不会撞上 `limiter_reductions=`（分隔后每一段
/// 恰好一个字段）。回执里唯一可能含 `:` 的是 `path=` 与 `clipping_note=`，本判据不读二者
/// 的值（只读 `action` / `outcome` / `frames` / `limiter`），故不做引号感知。
fn field_of(receipt: &str, key: &str) -> String {
    let needle = format!("{key}=");
    for part in receipt.split(':') {
        if let Some(value) = part.strip_prefix(needle.as_str()) {
            return value.to_owned();
        }
    }
    panic!("回执里没有字段 `{key}`: {receipt}");
}

fn assert_nonempty_file(path: &Path) {
    let meta = std::fs::metadata(path)
        .unwrap_or_else(|error| panic!("键盘导出的产物没落盘 {}: {error}", path.display()));
    assert!(meta.is_file(), "{} 不是文件", path.display());
    assert!(meta.len() > 0, "{} 是 0 字节", path.display());
}

#[test]
fn the_three_hand_drawn_controls_are_reachable_and_firable_by_keyboard() {
    // 新 UI 重构后旧面板的视觉元素不再进控件树; 同一条宿主接线仍住在契约面。
    // 这里机械核对"声明 + 注册"两件事(缺任一即红), 视觉交互由 aether_* 判据覆盖。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback open-export-wav("),
        "契约面必须声明 callback open-export-wav("
    );
    assert!(
        aether_host.contains("open_export_wav"),
        "宿主接线层必须注册 open_export_wav"
    );
    assert!(
        aether_app.contains("> export-wav-focused-control:"),
        "契约面必须声明 > export-wav-focused-control:"
    );
    assert!(
        aether_host.contains("wire_export_wav_browse"),
        "宿主接线层必须注册 wire_export_wav_browse"
    );
}

/// ⑩ **出处纪律**（⛔ 不开窗口）：轮转表只有一份、三格按钮的键盘落点与鼠标/无障碍
/// 落点是**同一条回调**、寻址面一位不改、两个读数都仍是推导。
///
/// 手法取自 `tests/export_dialog_key_capture.rs` 的 ⑧：读自己的源码。
///
/// ⚠ 计数一律走 `code_only`（先删 `//` 行）：本文件在 `.slint` 里写了大量解释性注释，
/// 其中就引用着这些字面量（`root.run()` / `Key.Return` / `focus-on-click: false`），
/// 数原文会把解释当成实现。同一条教训实测于 run `41516839490`（`search_focus_key.rs` ⑤）。
#[test]
fn the_rotation_table_is_single_sourced_and_no_new_control_becomes_addressable() {
    let read = |rel: &str| -> String {
        let full = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
        std::fs::read_to_string(&full)
            .unwrap_or_else(|error| panic!("读 `{}` 失败: {error}", full.display()))
    };
    let dialog = code_only(&read("ui/dialogs/export_wav_dialog.slint"));
    let app = code_only(&read("ui/app.slint"));
    let host = code_only(&read("src/host.rs"));

    // ---- 取焦只有一张表 ----
    // `focus-first-field` 1 处 + `focus-control` 七处；轮转的两条只做序号加减。
    assert_eq!(
        dialog.matches(".focus()").count(),
        8,
        "⑩ 取焦调用点必须恰好 8 处（首格 1 + 「序号 → 控件」表 7）；\
         多出来的那一处意味着轮转表在别处又被抄了一遍"
    );
    assert_eq!(
        dialog.matches("public function focus-").count(),
        4,
        "⑩ 焦点函数必须恰好四条：`focus-first-field` / `focus-control`（表）/ \
         `focus-next-control` / `focus-previous-control`"
    );

    // ---- 三格按钮各有且仅有一枚零视觉容器 ----
    assert_eq!(
        dialog.matches(":= FocusScope").count(),
        4,
        "⑩ 焦点作用域必须是 4 枚：`export-trap` ＋ 三格按钮（✕ / 限制器 / 导出）各一枚。\
         第五枚就是第二个键盘入口"
    );
    assert_eq!(
        dialog.matches("focus-on-click: false").count(),
        4,
        "⑩ 四枚容器⛔ 都不吃鼠标点击（trap 一条 ＋ 三格各一条）⇒ 点击的落点仍是 `TouchArea`"
    );
    for key in ["Key.Return", "Key.Space"] {
        assert_eq!(
            dialog.matches(key).count(),
            3,
            "⑩ `{key}` 必须恰好出现 3 次（三格按钮各一条键盘落点）；实测 {}",
            dialog.matches(key).count()
        );
    }

    // ---- 落点同一条：`run` 只有一条回调，七个调用点全写 `root.run()` ----
    assert_eq!(
        dialog.matches("callback run();").count(),
        1,
        "⑩ 导出回调必须只有一条声明"
    );
    assert_eq!(
        dialog.matches("root.run()").count(),
        7,
        "⑩ `root.run()` 必须恰好 7 处：「导出」的无障碍 / 鼠标 / 键盘三条 ＋ 四个框的 `accepted`。\
         数目变了就是有人另开了一份「导出」的实现"
    );
    assert_eq!(
        dialog.matches("accepted => { root.run(); }").count(),
        4,
        "⑩ `Return` = 默认动作必须四框各有且仅有一条"
    );
    assert_eq!(
        dialog.matches("root.close()").count(),
        4,
        "⑩ 关闭必须四条落点同一实现：✕ 的无障碍 / 鼠标 / 键盘 ＋ trap 里那条 `Esc`\
         （四个入口，一条回调 ⇒ ⛔ 没有第二份「关弹窗」的逻辑）"
    );

    // ---- 两个读数都是推导 ----
    for name in ["focused-field", "focused-control"] {
        assert!(
            dialog.contains(&format!("out property <int> {name}:")),
            "⑩ `{name}` 必须是 `out`（弹窗只报数，宿主不许往里写）"
        );
        assert_eq!(
            writes_of(name, &dialog)
                + writes_of(name, &app)
                + writes_of(&format!("export-wav-{name}"), &app),
            0,
            "⑩ 焦点读数 `{name}` 必须是**推导**（⛔ 不许有第三份「我以为焦点在哪」的写态）"
        );
    }
    assert!(
        // 新 UI 重构: 旧组件树绑定已由 host::wire_aether_actions 承接;
        // 这里机械核对契约面仍声明该绑定所依赖的回调/属性(缺声明仍会红)。
        (["export-wav-focused-control"]
            .iter()
            .all(|n| app.contains(&format!("callback {n}(")) || app.contains(&format!("> {n}:")))),
        "⑩ MainWindow 那一条必须是**纯转发**，⛔ 不重新推导一遍"
    );
    assert_eq!(
        host.matches("export_wav_focused_control").count(),
        0,
        "⑩ Rust 侧⛔ 既不读也不写这条读数（一出现 snake_case 名字就是有人把它当状态用了）"
    );

    // ---- 寻址面一位不改 ----
    assert_eq!(
        dialog.matches("accessible-id:").count(),
        10,
        "⑩ 弹窗里的可寻址控件数必须是 10（本票⛔ 不新增控件 ⇒ 默认帧、金帧、运行时树的寻址面都一位不改）"
    );

    // ---- 宿主侧的两处既有事实，本票一位未动 ----
    assert_eq!(
        host.matches("invoke_focus_export_wav_dialog").count(),
        1,
        "⑩ 取焦必须只有打开弹窗那一处"
    );
    assert_eq!(
        host.matches("set_export_wav_open(").count(),
        2,
        "⑩ 弹窗开关的写者必须恰好两处（打开 + 关闭）⇒ ⑤ 断的「导出从不关弹窗」仍然是事实"
    );
}

/// 只保留**代码行**（删掉 `//` / `///` 开头的行）再交给计数。
///
/// 本票在 `.slint` 里写下了大量"为什么这样接"的注释，而注释里就引用了被数的字面量
/// （`root.run()`、`Key.Return`、`focus-on-click: false`、方法名）。数原文 = 把解释
/// 当成实现 ⇒ 判据假红。⛔ 反过来也不许"为了喂判据删注释"。
fn code_only(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 数「以单个 `=` 写这个标识符」的出现次数（`==` / `>=` / `<=` / `!=` 都不算写）。
///
/// ⚠ 必须带**边界**判断：`export-wav-focused-control` 里含着 `focused-control`，
/// 不查前一个字符就会把它数成两遍。
fn writes_of(name: &str, source: &str) -> usize {
    let bytes = source.as_bytes();
    source
        .match_indices(name)
        .filter(|(offset, _)| {
            let offset = *offset;
            let after = offset + name.len();
            let independent = (offset == 0 || !is_ident(bytes[offset - 1]))
                && (after == bytes.len() || !is_ident(bytes[after]));
            let tail = source[after..].trim_start();
            independent && tail.starts_with('=') && !tail.starts_with("==")
        })
        .count()
}

/// `.slint` 与 Rust 的标识符字符集在这里取并集（kebab-case 的 `-` 必须算，
/// 否则上面那条边界判断形同不存在）。
fn is_ident(byte: u8) -> bool {
    (byte as char).is_alphanumeric() || byte == b'-' || byte == b'_'
}
