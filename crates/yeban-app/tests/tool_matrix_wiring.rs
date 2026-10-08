//! **`[UI-NOTE-003]` §3.3 工具矩阵的接线诚实性** —— 台账缺口 **R16**。
//!
//! ## 缺陷（本文件钉住的就是它）
//!
//! `crates/yeban-app/ui/console/piano_roll.slint` 的五个工具按钮原先声明了
//! `accessible-role: button` 与 `accessible-checkable` / `accessible-checked`，
//! 但那个节点是纯 `Rectangle` + `UiText`：**没有** `TouchArea`、也**没有**任何回调
//! ⇒ 用户点它**没有任何反应**（`active-tool` 只由 `src/input.rs` 的 `1`..`5` 数字键写）。
//! 也就是说，界面对自动化的承诺（"我是按钮、我会被勾选"）与用户的手是两回事。
//!
//! ## 为什么判据住在 `tests/` 而不是 `src/elements.rs` 的单元测试里
//!
//! 与 `tests/undo_tree_honesty.rs` 同款理由：`src/elements.rs` 的 `mod tests` 尾部是
//! 多线并发下的热点（一条线在改注册表标签），判据放在**新文件**里 ⇒ 两个写者的改动面不重叠。
//!
//! ## 三条路径（各自独立，破坏任何一条都变红）
//!
//! | # | 路径 | 钉住什么 |
//! | :--- | :--- | :--- |
//! | ① | `piano_roll.slint` 的结构 | 五个工具节点各自是**真的** `TouchArea`（就地 `clicked`）＋声明了 `callback tool-selected(int)`；`z: 1;` 保证这五个输入面**在滚动手势面之前**被命中测试看到 |
//! | ② | 组件链的转发 | `ConsoleTabs` 与 `MainWindow` 两级都声明并**转发**这条回调（只声明不转发 = 空壳） |
//! | ③ | `src/**` 的写者计数 | `active-tool` 的写入仍然**只有** `host::apply_action` 的 `Action::SelectTool` 一个臂；界面的点击经 `host::wire_tool_select` 进那**同一个**臂 |
//!
//! ## 本文件**不做**的事
//!
//! 它不判像素（本票明文禁止改基准图），也不判"点下去真的生效" —— 后者由
//! `crates/yeban-app/tests/live_ui_mcp.rs` 的
//! `the_piano_roll_tool_buttons_change_the_active_tool_at_runtime` 用**真实指针注入**见证
//! （按下 → 移动 → 松手 → Slint 命中测试 → `.slint` 的输入面 → 回调 → 宿主写属性）。
//! 两条判据合起来才是"源码里接上了"+"运行时真的动"。

use std::path::PathBuf;

/// `crates/yeban-app/`。
fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// 读 crate 内一个相对路径的文本。
fn read(rel: &str) -> String {
    let path = crate_dir().join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读 {} 失败: {error}", path.display()))
}

/// 去掉 `//` 行注释（含行尾注释）之后的**代码文本**。
///
/// 与 `tests/undo_tree_honesty.rs` 的 `code_only` 同款口径：本票把"修了什么"写进注释留痕，
/// 探针若把注释也算成界面内容，就会逼着作者**不写**留痕 —— 那与本仓库的纪律相反。
fn code_only(source: &str) -> String {
    source
        .lines()
        .map(|line| match line.split_once("//") {
            Some((code, _comment)) => code,
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `source` 里 `from` 与 `to` 之间的那一段（`from` 之后，`to` 之前的内容）。
///
/// 两个锚点都必须存在 —— 缺一个就 panic（"文件被改到看不出来"本身就是要报出来的事，
/// 不能静默返回空串让后面的断言变成"看起来有"）。
fn block<'a>(source: &'a str, from: &str, to: &str) -> &'a str {
    let start = source
        .find(from)
        .unwrap_or_else(|| panic!("源码里找不到起始锚点 `{from}`"));
    let rest = &source[start + from.len()..];
    let end = rest
        .find(to)
        .unwrap_or_else(|| panic!("起始锚点 `{from}` 之后找不到结束锚点 `{to}`"));
    &rest[..end]
}

/// `source` 里从 `from`（含）到与它配对的第一个 `{` 的闭合 `}`（含）之间的那一段。
///
/// 为什么要按括号配对取块：工具按钮的循环体里还有嵌套的元素块与 `TouchArea`。按"下一个
/// `}`"取会在第一个内层块处停住，于是后面的断言只在半个块上成立（"看起来有"的典型来源）。
///
/// `{` / `}` 计数对 `.slint` 源码是安全的：这里**不去注释**，而工具块那一段里的字符串字面量
/// 与注释都不含花括号（`text:` 只有 `"" + (tool_index + 1) + " " + …`）。配对失败时 panic，
/// 不返回残缺块。
fn brace_block<'a>(source: &'a str, from: &str) -> &'a str {
    let start = source
        .find(from)
        .unwrap_or_else(|| panic!("源码里找不到 `{from}`"));
    let rest = &source[start..];
    let mut depth = 0usize;
    let mut opened = false;
    for (offset, byte) in rest.bytes().enumerate() {
        match byte {
            b'{' => {
                depth += 1;
                opened = true;
            }
            b'}' => {
                depth = depth
                    .checked_sub(1)
                    .unwrap_or_else(|| panic!("`{from}` 的块里出现了多余的 `}}`"));
                if opened && depth == 0 {
                    return &rest[..=offset];
                }
            }
            _ => {}
        }
    }
    panic!("`{from}` 的块没有闭合（花括号不配对）");
}

/// `src/**/*.rs` 的全部**代码**文本（去掉 `//` 注释后拼接；单位：一个字符串）。
///
/// 为什么去掉注释：本票在注释里引用旧代码（"`active-tool` 只由键盘写"）。不去注释的话，
/// 那条留痕自己会把写者计数变成 2，探针反而逼作者删掉史实。
fn all_src_rs() -> String {
    fn walk(dir: &std::path::Path, out: &mut String) {
        for entry in std::fs::read_dir(dir).expect("读 src 目录") {
            let path = entry.expect("目录项").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push_str(&code_only(&std::fs::read_to_string(&path).unwrap_or_else(
                    |error| panic!("读 {} 失败: {error}", path.display()),
                )));
                out.push('\n');
            }
        }
    }
    let mut out = String::new();
    walk(&crate_dir().join("src"), &mut out);
    assert!(out.len() > 10_000, "`src/` 的源码拼接异常偏小");
    out
}

/// `needle` 在 `haystack` 里出现的次数（单位：次）。
fn count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

/// 工具按钮的几何 / 无障碍声明里**声明了却没有输入面**的旧形态（逐字）。
///
/// 它们是缺陷的**唯一**承载行：`TouchArea` 版本里不可能同时出现"矩形背景色 + 无 `clicked`"，
/// 因为本文件下面的断言要求每个工具节点的块里都有 `clicked`。
const TOOL_BUTTON_IDS: [&str; 5] = [
    "piano-roll-tool-select-button",
    "piano-roll-tool-pencil-button",
    "piano-roll-tool-knife-button",
    "piano-roll-tool-velocity-button",
    "piano-roll-tool-eraser-button",
];

// =========================================================================
// ① `piano_roll.slint`：五个工具节点各自是**真的**输入面
// =========================================================================

/// 判据（**结构 + 几何 + 回调声明**）。
///
/// 四条子路径：
/// 1. 头里必须有 `callback tool-selected(int);`（没有它，点击无处上报）；
/// 2. 五个工具节点各自所在的块里必须有 `clicked =>`（"点了没反应"就是这个块缺了它）；
/// 3. 每个工具块必须声明 `z: 1;` —— 本组件末尾那条覆盖全高的滚动手势 `TouchArea` 也是
///    `root` 的兄弟且声明更晚；没有这条 `z`，Slint 的 `FrontToBack` 命中测试会把工具行的
///    点击交给滚动手势面（"点了没反应"会以另一种形态回来）；
/// 4. 点击上报的行号必须就是 `tool_index + 1`（与 `active-tool` / `Tool::digit` 同一口径）。
#[test]
fn every_piano_roll_tool_button_is_a_real_click_source() {
    let roll = code_only(&read("ui/console/piano_roll.slint"));

    assert!(
        roll.contains("callback tool-selected(int);"),
        "`piano_roll.slint` 必须声明 `callback tool-selected(int);` —— \
         没有它，工具按钮的点击没有落点（R16 的缺陷形态）"
    );

    // 每个工具节点的无障碍声明必须**逐 ID** 保留（`[UI-TEST-001]` 的 ID 形状不变），
    // 而每个节点块里的输入面由下面按括号配对取出的循环体统一证明。
    let loop_body = brace_block(&roll, "for tool_index in 5 : Rectangle {");
    for (index, id) in TOOL_BUTTON_IDS.iter().enumerate() {
        // ID 由 `root.tool-names[tool_index]` 拼出来 ⇒ 本判据把"名字表 → ID"的对应逐行钉死：
        // 第 index 个名字必须存在于 `scene::TOOL_NAMES`，否则 `piano-roll-tool-<名字>-button`
        // 这个 ID 在任何投影下都不可能出现（注册表与界面用的是同一张表）。
        let name = &id
            .trim_start_matches("piano-roll-tool-")
            .trim_end_matches("-button");
        assert_eq!(
            yeban_app::scene::TOOL_NAMES[index],
            *name,
            "第 {index} 个工具按钮的 ID（`{id}`）必须与 `scene::TOOL_NAMES[{index}]` 同名"
        );
    }
    assert!(
        roll.contains("\"piano-roll-tool-\" + root.tool-names[tool_index] + \"-button\""),
        "工具按钮的语义 ID 必须仍然由 `root.tool-names[tool_index]` 拼出来（`[UI-TEST-001]` 的 ID 形状不变）"
    );
    assert_eq!(
        count(loop_body, "accessible-id:"),
        1,
        "五个工具按钮的 ID 必须由循环体里**一个**模板拼出来（写死五个 ID 就是第二份名单）\n{loop_body}"
    );
    assert_eq!(
        count(loop_body, "TouchArea {"),
        1,
        "工具循环体里必须**恰好一个** `TouchArea` 模板（它被 `for` 展开成五个输入面）—— \
         一个都没有就是 R16 的旧形态（点了没反应）；超过一个就是第二个输入源\n{loop_body}"
    );
    assert_eq!(
        count(loop_body, "clicked =>"),
        1,
        "工具循环体里必须**恰好一个** `clicked`（五个输入面共用这一个模板）\n{loop_body}"
    );
    assert_eq!(
        count(loop_body, "z: 1;"),
        1,
        "工具循环体里必须**恰好一条** `z: 1;`（每个展开出来的按钮都要在最前）\n{loop_body}"
    );
    assert!(
        loop_body.contains("root.tool-selected(tool_index + 1);"),
        "点击上报的行号必须是 `tool_index + 1`（1..5，与 `Tool::digit` / `active-tool` 同一口径）—— \
         `tool_index` 是 0..4，写错一格就选错工具\n{loop_body}"
    );
}

// =========================================================================
// ② 组件链：两级都必须**声明并转发**
// =========================================================================

/// 判据（**转发链 + 单一写者提示**）。
///
/// 三条子路径：
/// 1. `ConsoleTabs` 声明 `callback tool-selected(int);` 并把 `PianoRoll` 的实例回调转发给
///    `root.tool-selected(tool)` —— 只声明不转发是空壳；
/// 2. `MainWindow` 声明同一条回调并把 `ConsoleTabs` 的实例回调转给 `root.tool-selected(tool)`；
/// 3. `src/**` 里**恰好一处** `ui.on_tool_selected(`（宿主的唯一接线点），且它读的行号经
///    `crate::input::Tool::from_digit` 还原成工具、随后调用 `apply_action`。
///    多出来的第二处就是第二个真相源。
#[test]
fn the_tool_selected_callback_is_forwarded_by_every_component_in_the_chain() {
    let tabs = code_only(&read("ui/console/console_tabs.slint"));
    assert!(
        tabs.contains("callback tool-selected(int);"),
        "`console_tabs.slint` 必须声明 `callback tool-selected(int);`"
    );
    assert!(
        tabs.contains("tool-selected(tool) => { root.tool-selected(tool); }"),
        "`console_tabs.slint` 必须把 `PianoRoll` 的 `tool-selected` 转给 `root.tool-selected`"
    );

    let app = code_only(&read("ui/app.slint"));
    assert!(
        app.contains("callback tool-selected(int);"),
        "`app.slint` 的 `MainWindow` 必须声明 `callback tool-selected(int);`"
    );
    assert!(
        app.contains("tool-selected(tool) => { root.tool-selected(tool); }"),
        "`app.slint` 必须把 `ConsoleTabs` 的 `tool-selected` 转给 `root.tool-selected`"
    );

    let src = all_src_rs();
    assert_eq!(
        count(&src, "ui.on_tool_selected("),
        1,
        "`ui.on_tool_selected(` 在 `src/**` 里必须恰好出现一次（唯一接线点 host::wire_tool_select），\
         实际 {} 次",
        count(&src, "ui.on_tool_selected(")
    );
    let host = code_only(&read("src/host.rs"));
    // 结束锚点必须是**代码**行：`code_only` 去掉了全部 `//` 注释（含文档注释），
    // 因此 `/// …` 形状的锚点在它上面永远找不到。
    let wire = block(&host, "pub fn wire_tool_select(", "pub fn pencil_op_for(");
    assert!(
        wire.contains("crate::input::Tool::from_digit"),
        "宿主必须把界面报上来的行号经 `Tool::from_digit`（唯一的行号口径）还原成工具\n{wire}"
    );
    assert!(
        wire.contains("Action::SelectTool(tool)"),
        "宿主必须把还原出的工具送进**同一个** `Action::SelectTool` 臂（与数字键同一条链）\n{wire}"
    );
    assert!(
        wire.contains("apply_action"),
        "宿主必须调用 `apply_action`（`active-tool` 的唯一写者），不得在这里另写一次 setter\n{wire}"
    );
    assert!(
        !wire.contains("set_active_tool"),
        "`wire_tool_select` 里不得出现 `set_active_tool` —— `active-tool` 的写者只有 \
         `apply_action` 一个（复制一份就是第二个真相源）\n{wire}"
    );
}

// =========================================================================
// ③ `active-tool` 的写者计数：键盘与点击落到同一个臂
// =========================================================================

/// 判据（**写者计数 + 与键盘共用同一个臂**）。
///
/// 两条子路径：
/// 1. **生产代码**（`src/host.rs`）里 `ui.set_active_tool(` 恰好出现 **2** 次：
///    `apply_action` 的 `Action::SelectTool` 臂与 `Action::TogglePencilTool` 臂
///    （后者是 `B` 键的快速切换）。第三处就是第三个真相源；
/// 2. `apply_action` 的工具选择臂必须**同时**写界面属性与记动作日志（两条都在那一个臂里）。
///
/// ## 为什么分母是 `src/host.rs` 而不是 `src/**`
///
/// 实测：`src/**` 里 `set_active_tool(` 有 **3** 处 —— 第三处在
/// `src/test_port_adapter.rs` 的 `the_active_tool_is_readable_from_the_host`，那是**判据自己**
/// 直接调 setter 来证明"宿主能读回镜像"。把它算成"写者"是**假阳性**：它不是生产路径上的
/// 第二个真相源，删掉它只会让那条既有判据变瞎。因此本条的分母是生产宿主文件，
/// 并且下面另有一条断言把 `test_port_adapter.rs` 之外的文件排除在写者名单外。
///
/// 为什么计数是 2 而不是 1：`B`（铅笔 ⇄ 箭头）与 `1`..`5` 是规范里两个不同的动作，
/// 它们**共用** `active-tool` 这一个属性。本判据把"恰好这两个"钉死。
#[test]
fn active_tool_still_has_exactly_the_two_host_writers() {
    let src = all_src_rs();
    let host = code_only(&read("src/host.rs"));
    assert_eq!(
        count(&host, "ui.set_active_tool("),
        2,
        "`ui.set_active_tool(` 在 `src/host.rs` 里必须恰好出现 2 次（`Action::SelectTool` 与 \
         `Action::TogglePencilTool` 两个臂），实际 {} 次 —— 多出来的那一次是第二个真相源",
        count(&host, "ui.set_active_tool(")
    );
    // 写者名单只许是生产宿主与那条直读判据：任何**别的**文件新写一次都会在这里变红。
    for file in [
        "src/main.rs",
        "src/live_surface.rs",
        "src/undo.rs",
        "src/bridge.rs",
        "src/elements.rs",
        "src/input.rs",
    ] {
        let text = code_only(&read(file));
        assert!(
            !text.contains("set_active_tool("),
            "`{file}` 不得写 `active-tool` —— 写者只有 `host::apply_action` 的两个臂\
             （界面侧的镜像由它写，调用方不该自己写）"
        );
    }
    assert!(
        count(&src, "ui.on_tool_selected(") == 1,
        "`src/**` 里必须恰好一处 `ui.on_tool_selected(`（新回调的唯一接线点）"
    );

    let tool_arm = block(
        &host,
        "Action::SelectTool(tool) =>",
        "Action::TogglePencilTool",
    );
    assert!(
        tool_arm.contains("ui.set_active_tool(i32::from(tool.digit()))"),
        "工具选择臂必须把 `Tool::digit()`（唯一的数字口径）写进界面属性\n{tool_arm}"
    );
    assert!(
        tool_arm.contains("port.perform(UiAction::SelectTool(tool))"),
        "撤销端口在场时，工具选择臂必须同时记一条 `UiAction::SelectTool` 动作日志\n{tool_arm}"
    );
}
