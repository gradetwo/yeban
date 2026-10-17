//! 判据（任务单 `#129`，源码腿）：**「新建工程」按钮真的存在、可无障碍访问、且只上抛**。
//!
//! ## 为什么这条判据要从**源码**派生期望值（台账第 293 条）
//!
//! ⛔ **不从本文件读**被判的字符串：ID 与无障碍名都从
//! `src/elements.rs` 的 `registry.add(…)` 条目**解析**出来，回调名又从 ID 按
//! `module-{动作}-button` 的族规则**算**出来；`.slint` 三处（混音台 / 控制台 / 主窗口）
//! 与宿主 `src/host.rs` 的接线逐条对账。本文件只提供**解析与重算**，不提供文本
//! （与 `tests/roll_injection_registration.rs` 同一口径）。
//!
//! ## 判定量（每个数字都有单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | 注册表条目 | `src/elements.rs` 里 `"module-new-project-button"` 的 `registry.add` | 条 |
//! | 上抛点 | `.slint` 里 `root.<动作>();` 的出现次数（点击源 + 默认无障碍动作） | 处 |
//! | 转发点 | `console_tabs.slint` / `app.slint` 里 `callback` 与 `=> { root.…; }` | 处 |
//! | 宿主接线 | `host.rs::wire_new_project` 里三件复用的调用点 | 处 |
//!
//! ## ⭐ 平台无关：**行尾不是被判对象**（任务单 `#137`）
//!
//! CI 的 `windows` 腿在 `core.autocrlf=true` 下把源码检出成 **CRLF**。本判据有两条
//! needle 是**跨行**的 —— `"… => {\n            root.…();"` 与 `"\n}\n"` —— 它们的 `\n`
//! 紧跟在 `{` / `}` 之后 ⇒ 在 CRLF 检出里 `{` 与 `\n` 之间夹着 `\r` ⇒ `contains` / `find`
//! 恒为假（任务单 `#137` 的复现读数见交付报告）。行终止符**不是**被判对象：判的是
//! 缩进与转发的形状。因此入口先把源码归一到 LF（[`lf`]）；⛔ 归一只动 `\r\n`，
//! 删掉转发行、改标签、拆接线**仍然红**。
//!
//! ## 这条判据怎么变红（本单注入读数见交付报告）
//!
//! - 删掉 `.slint` 的新按钮（**去掉本片改动**）⇒ 「注册表 → `.slint`」那一段红；
//! - 把 `accessible-label` 改一个字 ⇒ 与 `elements.rs` 的条目不再逐字相等 ⇒ 红；
//! - 只保留 `.slint` 的 `TouchArea.clicked` 而摘掉 `accessible-action-default` ⇒ 红；
//! - 宿主 `wire_new_project` 不再调 `create_new_project_file` / `open_runtime` ⇒ 红。

/// 注册表源码（`registry.add(…)` 的唯一登记面）—— ⛔ 不是本文件（台账第 293 条）。
const ELEMENTS_RS: &str = include_str!("../src/elements.rs");
/// 混音台源码（按钮的 accessible-id / accessible-label / 两个上抛点）。
const MIXER_CONSOLE_SLINT: &str = include_str!("../ui/console/mixer_console.slint");
/// 控制台容器源码（控制台级回调的声明与转发）。
const CONSOLE_TABS_SLINT: &str = include_str!("../ui/console/console_tabs.slint");
/// 主窗口源码（`MainWindow` 回调的声明与转发）。
const APP_SLINT: &str = include_str!("../ui/app.slint");
/// 宿主源码（`wire_new_project` 的唯一实现）。
const HOST_RS: &str = include_str!("../src/host.rs");

/// 把 **Windows 检出**（`core.autocrlf=true` ⇒ 每行 CRLF）的行尾归一到 LF。
///
/// ⛔ 只动 `\r\n`：判据钉的是结构（缩进 / 转发形状 / 接线），不是行终止符。
/// ⛔ 裸 `\r` 不归一（那是文件损坏，不该被归一掩盖）。
/// 与 `yeban-mcp/tests/op_reachability.rs` 的 R51 纪律同一条：**入口先归一**。
fn lf(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// 反过来：把 LF 源码换成 **Windows 检出**的字节（模拟腿用，幂等）。
fn as_crlf(text: &str) -> String {
    lf(text).replace('\n', "\r\n")
}

/// 从 `src/elements.rs` 的 `registry.add` 条目里解析出 `(所属 .slint, 无障碍名)`。
///
/// 断言的形状是**固定的**（`registry.add(` = ID / 种类 / 文件 / 标签 / 动态位），
/// 因此解析器只按行推进；⛔ 不读本文件里的任何字面量。
fn registered_button(elements: &str, id: &str) -> (String, String) {
    let needle = format!("\"{id}\"");
    let at = elements
        .find(&needle)
        .unwrap_or_else(|| panic!("`src/elements.rs` 里没有 `{id}` 的 registry.add 条目"));
    let mut lines = elements[at..].lines().skip(1);
    let kind = lines.next().expect("种类行").trim();
    assert!(
        kind.starts_with("ElementKind::Button"),
        "`{id}` 必须登记成 Button（实得 `{kind}`）"
    );
    let file_line = lines.next().expect("文件行").trim();
    let file = file_line
        .trim_end_matches(',')
        .trim()
        .trim_matches('"')
        .to_owned();
    // 文件行之后可能夹注释行，第一条非注释行就是无障碍名的字面量。
    let label_line = lines
        .find(|line| !line.trim().starts_with("//"))
        .expect("无障碍名行")
        .trim();
    let label = label_line
        .trim_end_matches(',')
        .trim()
        .trim_matches('"')
        .to_owned();
    assert!(!label.is_empty(), "`{id}` 的无障碍名不得为空");
    (file, label)
}

/// 判据本体：五种源码文本进、结论出。**参数化**是为了让模拟腿复用同一段断言
/// （⛔ 不复制断言 —— 副本会与真判据漂移）。
fn check_new_project_registration(
    elements_src: &str,
    mixer_src: &str,
    console_src: &str,
    app_src: &str,
    host_src: &str,
) {
    // ⭐ 入口先归一换行（任务单 `#137`）：Windows 检出是 CRLF，而下面有两条跨行 needle。
    // ⛔ 归一只去掉行终止符，不改任何被判的字符。
    let elements = lf(elements_src);
    let mixer = lf(mixer_src);
    let console = lf(console_src);
    let app = lf(app_src);
    let host = lf(host_src);

    const ID: &str = "module-new-project-button";
    let (file, label) = registered_button(&elements, ID);
    // 注册表声明的宿主文件必须就是混音台（元件住在哪，登记就得写哪）。
    assert_eq!(
        file, "console/mixer_console.slint",
        "注册表给 `{ID}` 声明的文件必须与按钮的真实位置一致"
    );

    // 族规则：`module-{动作}-button` ⇒ 回调名 `{动作}`（与「打开工程」同一条规则）。
    let action = ID
        .strip_prefix("module-")
        .and_then(|rest| rest.strip_suffix("-button"))
        .expect("`module-{动作}-button` 的族规则");
    let raise = format!("root.{action}();");
    let callback = format!("callback {action}();");

    // ---- ① 混音台：ID / 标签 / role / 两个上抛点（点击源 + 默认无障碍动作）----
    for (what, needle) in [
        ("accessible-id", format!("accessible-id: \"{ID}\";")),
        (
            "accessible-label",
            format!("accessible-label: \"{label}\";"),
        ),
        ("accessible-role", "accessible-role: button;".to_owned()),
        ("callback", callback.clone()),
        (
            "accessible-action-default",
            format!("accessible-action-default => {{ {raise} }}"),
        ),
    ] {
        assert!(
            mixer.contains(&needle),
            "`ui/console/mixer_console.slint` 缺少 {what}：`{needle}`"
        );
    }
    // 点击源与默认无障碍动作必须是**同一段**落点（`[UI-A11Y-003]` §7.3）：
    // `clicked => { root.<动作>(); }` 在按钮块里必须出现。
    assert!(
        mixer.contains(&format!("clicked => {{ {raise} }}")),
        "`ui/console/mixer_console.slint` 缺少点击源：`clicked => {{ {raise} }}`"
    );

    // ---- ② 控制台容器：声明 + 转发（只上抛，⛔ 没有第二条实现）----
    assert!(
        console.contains(&callback),
        "`ui/console/console_tabs.slint` 缺少 `{callback}`"
    );
    assert!(
        console.contains(&format!("{action} => {{\n            root.{action}();")),
        "`ui/console/console_tabs.slint` 缺少 `{action}` 的转发"
    );

    // ---- ③ 主窗口：声明 + 转发（事件源的**唯一**落点 = 宿主 `wire_new_project`）----
    assert!(app.contains(&callback), "`ui/app.slint` 缺少 `{callback}`");
    assert!(
        app.contains(&format!("{action} => {{\n                root.{action}();")),
        "`ui/app.slint` 缺少 `{action}` 的转发"
    );

    // ---- ④ 宿主：`wire_new_project` 真的接了那三件事（对话框 / 落盘 / 换工程）----
    let at = host
        .find("pub fn wire_new_project(")
        .expect("`host.rs` 里必须有 `wire_new_project`");
    let window = &host[at..];
    let end = window.find("\n}\n").expect("函数体终点");
    let body = &window[..end];
    for needle in [
        "picker.can_prompt()",
        "picker.choose_save(",
        "create_new_project_file(&path)",
        "runtime.open_runtime(&ui, &path)",
        "\"new-project\"",
    ] {
        assert!(
            body.contains(needle),
            "`host::wire_new_project` 缺少接线：`{needle}`"
        );
    }
    // ⛔ 它**不**自己写容器：落盘只有 `create_new_project_file`（唯一入口）那一条。
    assert!(
        !body.contains("save_archive_file("),
        "⛔ `wire_new_project` 不得绕过 `create_new_project_file` 直接调 `save_archive_file`"
    );
}

/// 真判据（LF 工作树 ＝ macOS / Linux 的读数）。
#[test]
fn the_new_project_button_is_registered_accessible_and_upward_only() {
    check_new_project_registration(
        ELEMENTS_RS,
        MIXER_CONSOLE_SLINT,
        CONSOLE_TABS_SLINT,
        APP_SLINT,
        HOST_RS,
    );
}

/// 模拟腿（任务单 `#137`）：**Windows 检出**（每行 CRLF）下同一条判据必须绿。
///
/// 把五份被判源码逐字换成 CRLF、喂给**同一个** [`check_new_project_registration`]。
/// ⚠ 这条腿守的是「判据不依赖行尾」：若 [`lf`] 被删或漏接某一份源码，它立刻变红
/// （macOS 上就能跑出 Windows 的字节形态）。
#[test]
fn the_criterion_holds_on_a_windows_crlf_checkout() {
    check_new_project_registration(
        &as_crlf(ELEMENTS_RS),
        &as_crlf(MIXER_CONSOLE_SLINT),
        &as_crlf(CONSOLE_TABS_SLINT),
        &as_crlf(APP_SLINT),
        &as_crlf(HOST_RS),
    );
}
