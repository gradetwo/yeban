//! **左侧资源栏的搜索框：真输入框 ＋ 真过滤**（任务单 `D-UI-14`）—— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 这一格**从前是假的**：`ui/sidebar.slint` 里放着一个 `accessible-role: text-input` 的
//! 占位矩形，里面一句静态 `UiText` 写着"搜索乐器 / 采样 / 预置"。敲进去什么都不动 ——
//! 一个能寻址、能被屏读器读出、看起来完全正常的**空壳**（与 `ui/transport.slint:239`
//! 记录的"上一版只有占位矩形……合成事件根本没有源"同型）。
//!
//! 现在它是一个真 `TextInput`，而"搜索"这件事有一条**结构**约束：Slint 1.18.1 的字符串
//! 内置集里没有 `contains`（实测清单抄在 `ui/sidebar.slint` 的 `search-text` 处），
//! 所以匹配⛔ 不可能住在界面里。字只有一条出路：`changed text` → `MainWindow.
//! sidebar-search-changed` → 宿主 `bridge::filter_sidebar_content` → **重注入**条目数组。
//! 于是本票真正钉的是那一整条链路，而不是"框里能显示字"。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（起点） | 搜索串为空 ⇒ 画面逐字等于**未过滤**的全量投影、`sidebar-empty` 不在场、节点的 `value` 是**空串**（⛔ 不是缺席） |
//! | ②（字真的进得去） | 真实点击那个框 + 逐字敲进去 ⇒ `MainWindow.sidebar-search == fragment`，且运行时节点的 `value` 同步 —— 单向 `text:` 绑定会在第一次按键后就停在旧值，这里必须红 |
//! | ③（过滤的口径只有一处） | 树里的行**名称与顺序**逐字等于 `std` 的 `contains` 小写 oracle（与 `bridge::filter_sidebar_content` 同一算法，⛔ 不是把它的输出当预期）；且它是全量的**真子集** |
//! | ④（不重排布局） | 行 0 的几何一位不动：过滤只是**少画几行**，自上而下重排，⛔ 不是把行推来推去 |
//! | ⑤（两档空态分开） | 一个都不匹配 ⇒ 0 行 ＋ `sidebar-empty` 写着「没有匹配「query」的条目（本类共 N 条…）」；措辞与条数都来自 [`bridge::sidebar_no_match_reason`]（本类有 N 条），⛔ 不是"素材池为空"那句 |
//! | ⑥（不许幻影选中） | 钳位的**分母是过滤后的条数**：过滤剩 1 行时 `select-item(99)` ⇒ 0（⛔ 不是全量的 `len-1`）；0 行时任何请求 ⇒ 0；已选中的行被过滤掉 ⇒ 选中态自己落回 0 |
//! | ⑦（清空 = 还原） | 退格清空 ⇒ 全量逐字回来、`sidebar-empty` 离场 |
//! | ⑧（换分类不清串） | 搜索串是**用户敲的**，⛔ 不是分类的一部分：切到分类 2 ⇒ 串保留、空态用的是**那一类自己的**理由（模型缺口，⛔ 不是"没匹配上"）；切回分类 1 ⇒ 同一个串重新过滤 |
//! | ⑨（文本层接线） | `sidebar.slint`：语义节点挂在 `TextInput` 上、全文件**恰好一个** `sidebar-search-field`、`text <=> root.search-text`（⛔ 单向）；`app.slint`：两向绑定到 `MainWindow.sidebar-search` 并转发回调；宿主：注册 `on_sidebar_search_changed` 恰好一次，且⛔ 从不 `set_sidebar_search`（写回会与绑定成环） |
//!
//! ## 为什么夹具用分类 1（素材池）而不是分类 0（乐器）
//!
//! 分类 0 的内容取决于**这台机器下载了哪些采样库**（`assets/manifest.json` × 库根扫描）。
//! 把断言建在它上面 = 一条随机器漂移的判据（`AGENTS.md` §6 明令禁止）。分类 1 是
//! `filled_project()` 素材池的投影 —— 夹具决定，逐台一致。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒ 装**窗口**的
//! `#[test]` 本文件只有一个；另一个 `#[test]` 只做纯函数对账（⛔ 不碰 Slint）。

use yeban_app::bridge::{self, SidebarCategoryContent, ViewState};
use yeban_app::elements::{ElementKind, ElementRegistry};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::{KeyCode, Permission, PointerButton, Rect, Size, UiTestPort as _};

/// 与其余 Tier-1 判据同款（1440 宽 ⇒ `compact` 断点之外，资源栏是展开态）。
const WINDOW: Size = Size::new(1440, 900);

/// 搜索框的语义 id（注册表与运行时共用这一份措辞）。
const FIELD: &str = "sidebar-search-field";

/// 去掉 `//` 行注释与行尾注释后的**代码文本**（⑨ 的每一条读都先过这里）。
///
/// 与 `tests/sidebar_item.rs` 同一口径：注释里的"曾经长什么样"（留痕）不许被当成
/// "现在接上了"，也不许被当成"还有那份假数据"。
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

fn live_window() -> (LivePort<yeban_app::ui::MainWindow>, ViewState) {
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("夹具必须能投影");
    let scene = DemoScene::from_view(&view);
    let port = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    // 生产接线（`main.rs` / 无头装配 / 本判据调的是**同一个**函数）。
    host::wire_view_callbacks(port.ui());
    host::wire_sidebar_contents(
        port.ui(),
        std::rc::Rc::new(std::cell::RefCell::new(view.clone())),
    );
    (port, view)
}

fn bounds_of(port: &mut LivePort<yeban_app::ui::MainWindow>, id: &str) -> Rect {
    port.refresh_tree(None)
        .expect("必须能重抓控件树")
        .find_by_id(id)
        .unwrap_or_else(|| panic!("运行时控件树里找不到 `{id}`"))
        .bounds
        .unwrap_or_else(|| panic!("`{id}` 必须有几何包围盒"))
}

fn click(port: &mut LivePort<yeban_app::ui::MainWindow>, id: &str, bounds: Rect) {
    let x = f64::from(bounds.width) / 2.0;
    let y = f64::from(bounds.height) / 2.0;
    port.dispatch_pointer_down(id, x, y, PointerButton::Left)
        .expect("指针按下必须落到活窗口");
    port.dispatch_pointer_up(PointerButton::Left)
        .expect("指针释放必须落到活窗口");
}

/// 树里**在场**的条目（`(id, 无障碍名)`，按下标升序且连续）。与 `tests/sidebar_item.rs` 同款。
fn present_items(port: &mut LivePort<yeban_app::ui::MainWindow>) -> Vec<String> {
    let tree = port.refresh_tree(None).expect("重抓");
    let mut found: Vec<(usize, String)> = Vec::new();
    for node in tree.iter() {
        let Some(rest) = node.id.strip_prefix("sidebar-item-") else {
            continue;
        };
        let Ok(index) = rest.parse::<usize>() else {
            continue;
        };
        found.push((index, node.label.clone()));
    }
    found.sort_by_key(|(index, _)| *index);
    found
        .into_iter()
        .enumerate()
        .map(|(position, (index, label))| {
            assert_eq!(
                position, index,
                "条目 id 必须是**连续**下标（0..条数）：第 {position} 位实际是第 {index} 条"
            );
            label
        })
        .collect()
}

/// 独立 oracle：小写子串 `contains`，保序。⚠ 它**不是**把
/// `bridge::filter_sidebar_content` 的输出抄一遍（那是自我确认），而是用标准库
/// 把"子串过滤"这句话本身表达出来。
fn oracle(items: &[String], query: &str) -> Vec<String> {
    if query.is_empty() {
        return items.to_vec();
    }
    let needle = query.to_lowercase();
    items
        .iter()
        .filter(|name| name.to_lowercase().contains(&needle))
        .cloned()
        .collect()
}

/// 从夹具名字里挑一个**只命中一部分**条目的 ASCII 片段。
///
/// ⛔ 不写死 `"9"` 之类的字面量：素材池的内容由 `filled_project()` 决定，写死的片段
/// 会在上游换夹具时**静默失效**（片段命中全部条目 ⇒ 过滤"看起来"没起作用，而断言
/// `rows == oracle` 依然绿）。这里扫遍每条名字的 ASCII 字母数字串的所有窗口，
/// 取第一个"命中数在 `(0, 条数)` 开区间内"的片段；找不到就**响亮地**失败。
fn pick_fragment(items: &[String]) -> String {
    let lower: Vec<String> = items.iter().map(|name| name.to_lowercase()).collect();
    let mut candidates: Vec<String> = Vec::new();
    for name in &lower {
        let mut run: Vec<char> = Vec::new();
        let mut runs: Vec<Vec<char>> = Vec::new();
        for ch in name.chars() {
            if ch.is_ascii_alphanumeric() {
                run.push(ch);
            } else if !run.is_empty() {
                runs.push(std::mem::take(&mut run));
            }
        }
        if !run.is_empty() {
            runs.push(run);
        }
        for letters in runs {
            for len in 1..=letters.len() {
                for start in 0..=(letters.len() - len) {
                    let window: String = letters[start..start + len].iter().collect();
                    if !candidates.contains(&window) {
                        candidates.push(window);
                    }
                }
            }
        }
    }
    for candidate in &candidates {
        let matched = lower
            .iter()
            .filter(|name| name.contains(candidate.as_str()))
            .count();
        if matched > 0 && matched < items.len() {
            return candidate.clone();
        }
    }
    panic!(
        "夹具的 {} 条素材池名字里找不到「只命中一部分」的 ASCII 片段：{items:?} \
         ⇒ 本判据需要至少两条、且至少一条含 ASCII 字母数字的资源池投影（改夹具请连本判据一起看）",
        items.len()
    );
}

/// 挑一个字符 `ch`，使 `fragment + ch` **一条都不命中**（⑤ 的输入，同样不写死）。
fn pick_mismatch_suffix(items: &[String], fragment: &str) -> char {
    let lower: Vec<String> = items.iter().map(|name| name.to_lowercase()).collect();
    for ch in 'a'..='z' {
        let probe = format!("{fragment}{ch}");
        if !lower.iter().any(|name| name.contains(probe.as_str())) {
            return ch;
        }
    }
    panic!("26 个字母全都能与 `{fragment}` 拼成一次命中 ⇒ 夹具形态与本判据的前提不符：{items:?}");
}

#[test]
fn typing_in_the_sidebar_search_field_filters_the_injected_projection() {
    let (mut port, view) = live_window();

    // ---- 起点：切到分类 1（采样 = 素材池投影，逐台一致） ----
    port.ui().invoke_select_sidebar_category(1);
    assert_eq!(port.ui().get_sidebar_category(), 1);
    let full = bridge::sidebar_category_content(&view, 1);
    assert!(
        full.items.len() >= 2,
        "② 之后的「真子集」断言需要至少两条素材，实际 {:?}",
        full.items
    );
    assert_eq!(full.empty_reason, "", "① 非空分类不许带空态文案");

    // ---- ① 空串 = 不过滤：装配态逐字等于全量 ----
    assert_eq!(
        port.ui().get_sidebar_search().to_string(),
        "",
        "① 搜索串的默认值必须是空串"
    );
    let present = present_items(&mut port);
    assert_eq!(
        present, full.items,
        "① 空搜索串下画面的名称与顺序必须逐字等于全量投影"
    );
    assert_eq!(
        bridge::filter_sidebar_content(&full, ""),
        full,
        "① 过滤函数对空串必须是**恒等**（默认帧与过滤存在前逐字相同的那条依据）"
    );
    let tree = port.refresh_tree(None).expect("重抓");
    assert!(
        tree.find_by_id("sidebar-empty").is_none(),
        "① 有条目时不许画空态"
    );
    let field = tree.find_by_id(FIELD).unwrap_or_else(|| {
        panic!("① `{FIELD}` 必须在运行时树里（它从前是一个假矩形，现在得是真输入框）")
    });
    assert_eq!(
        field.role.as_str(),
        ElementKind::TextInput.accessible_role(),
        "① `{FIELD}` 的角色必须是 `text-input`"
    );
    assert_eq!(field.label, "搜索资源", "① 无障碍名必须与注册表逐字一致");
    let registry = ElementRegistry::from_view(&view);
    let meta = registry
        .get(FIELD)
        .expect("① `{FIELD}` 必须在装配时的注册表里（[UI-TEST-001]）");
    assert_eq!(meta.kind, ElementKind::TextInput);
    assert_eq!(
        field.value.as_deref(),
        Some(""),
        "① `value` 必须是**空串**而不是缺席（`None` = 这个节点根本没有值语义）"
    );
    let first_row = bounds_of(&mut port, "sidebar-item-0");

    // ---- ② 真实点击 + 逐字敲入 ----
    let fragment = pick_fragment(&full.items);
    let field_bounds = bounds_of(&mut port, FIELD);
    click(&mut port, FIELD, field_bounds);
    let tool_before = port.ui().get_active_tool();
    for ch in fragment.chars() {
        port.dispatch_key_press(KeyCode::Character(ch))
            .unwrap_or_else(|error| panic!("② 输入 `{ch}` 必须送达活窗口: {error:?}"));
    }
    assert_eq!(
        port.ui().get_sidebar_search().to_string(),
        fragment,
        "② 敲进去的字必须落到 `MainWindow.sidebar-search`（单向 `text:` 绑定在这里就会红：\
         用户第一次按键打断绑定，此后根属性停在旧值 —— `[PATCH export-wav-dialog-binding]`）"
    );
    let tree = port.refresh_tree(None).expect("重抓");
    assert_eq!(
        tree.find_by_id(FIELD)
            .and_then(|node| node.value.clone())
            .as_deref(),
        Some(fragment.as_str()),
        "② 屏读器读到的值（`accessible-value`）必须同步 = 框里那份字"
    );
    assert_eq!(
        port.ui().get_active_tool(),
        tool_before,
        "② 焦点被搜索框拿走时，单键快捷键**不许**同时被 DAW 吃掉（`ui/app.slint:722` 记的规范语义）"
    );

    // ---- ③ 过滤口径 ＋ ④ 布局 ----
    let shown = oracle(&full.items, &fragment);
    assert!(
        !shown.is_empty() && shown.len() < full.items.len(),
        "③ 片段必须命中**一部分**而非全部: {fragment:?} → {shown:?} / 全量 {:?}",
        full.items
    );
    let present = present_items(&mut port);
    assert_eq!(
        present, shown,
        "③ 画面的名称与顺序必须等于独立 oracle（std 的小写 `contains`），\
         而不是「少了几条但顺序乱掉」"
    );
    assert_eq!(
        bounds_of(&mut port, "sidebar-item-0"),
        first_row,
        "④ 过滤只减少行数：行 0 的几何必须一位不动"
    );
    let tree = port.refresh_tree(None).expect("重抓");
    assert!(
        tree.find_by_id("sidebar-empty").is_none(),
        "③ 还有命中就不许画空态"
    );

    // ---- ⑥a 钳位的分母 = 过滤后的条数 ----
    port.ui().invoke_select_sidebar_item(99);
    let last_shown = shown.len() - 1;
    assert_eq!(
        port.ui().get_sidebar_item(),
        last_shown as i32,
        "⑥ 越界请求必须钳到**过滤后**的最后一条（{} 条 ⇒ {last_shown}）。\
         ⚠ 全量是 {} 条 ⇒ 分母若是全量，这里会得到 {}",
        shown.len(),
        full.items.len(),
        full.items.len() - 1
    );
    assert_ne!(
        port.ui().get_sidebar_item(),
        (full.items.len() - 1) as i32,
        "⑥ 这条断言之所以有意义，正因为过滤后的下标与全量的下标**不同**"
    );

    // ---- ⑤ 一档：本类有 N 条、但一条都不匹配 ----
    let suffix = pick_mismatch_suffix(&full.items, &fragment);
    port.dispatch_key_press(KeyCode::Character(suffix))
        .expect("⑤ 追加一个字母必须送达");
    let query = format!("{fragment}{suffix}");
    assert_eq!(port.ui().get_sidebar_search().to_string(), query);
    assert!(
        present_items(&mut port).is_empty(),
        "⑤ `{query}` 不命中任何条目 ⇒ 条目族必须**全部**离场"
    );
    let tree = port.refresh_tree(None).expect("重抓");
    let empty = tree
        .find_by_id("sidebar-empty")
        .expect("⑤ 「都不匹配」也必须画一句理由（⛔ 不是空白列表）");
    let expected = bridge::sidebar_no_match_reason(&query, full.items.len());
    assert_eq!(
        empty.label, expected,
        "⑤ 理由必须逐字取自唯一的构造点（措辞与**条数**同源，⛔ 不在测试里手抄）"
    );
    let total = full.items.len().to_string();
    assert!(
        empty.label.contains(query.as_str()) && empty.label.contains(total.as_str()),
        "⑤ 理由里必须看得见用户敲的那份字与本类条数: {}",
        empty.label
    );
    assert_ne!(
        empty.label,
        bridge::SIDEBAR_EMPTY_ASSETS_REASON,
        "⑤ 素材池**有** {} 条，把「都不匹配」说成「尚无素材」是假话",
        full.items.len()
    );

    // ---- ⑥b 零行 ⇒ 一位不选 ----
    port.ui().invoke_select_sidebar_item(5);
    assert_eq!(
        port.ui().get_sidebar_item(),
        0,
        "⑥ 没有行可选时不许写出一个指向空气的选中值"
    );

    // ---- ⑦ 清空 = 全量逐字回来 ----
    for _ in 0..query.chars().count() {
        port.dispatch_key_press(KeyCode::Backspace)
            .expect("⑦ 退格必须送达");
    }
    assert_eq!(
        port.ui().get_sidebar_search().to_string(),
        "",
        "⑦ 清空后根属性必须是空串"
    );
    assert_eq!(
        present_items(&mut port),
        full.items,
        "⑦ 清空必须把全量投影原样放回来（名称与顺序都逐字）"
    );
    let tree = port.refresh_tree(None).expect("重抓");
    assert!(
        tree.find_by_id("sidebar-empty").is_none(),
        "⑦ 还原之后空态必须离场"
    );
    assert_eq!(
        tree.find_by_id(FIELD)
            .and_then(|node| node.value.clone())
            .as_deref(),
        Some(""),
        "⑦ 空框的 `value` 回到空串（提示文案那一格由 `visible` 驱动）"
    );

    // ---- ⑥c 已选中的行被过滤掉 ⇒ 选中态自己落回 ----
    let last_full = full.items.len() - 1;
    port.ui().invoke_select_sidebar_item(last_full as i32);
    assert_eq!(
        port.ui().get_sidebar_item(),
        last_full as i32,
        "⑥ 前提：全量 {last_full} 是合法下标（此刻没有过滤）"
    );
    for ch in fragment.chars() {
        port.dispatch_key_press(KeyCode::Character(ch))
            .expect("⑥ 重新敲入过滤串必须送达");
    }
    assert_eq!(
        port.ui().get_sidebar_item(),
        last_shown as i32,
        "⑥ 第 {last_full} 条被过滤器藏起来之后，选中态必须跟着落回过滤后的合法区间\
         （幻影选中：属性说选着第 {last_full} 行、画面没有那一行、屏读器报的是空气）"
    );

    // ---- ⑧ 换分类**不**清空那份字 ----
    port.ui().invoke_select_sidebar_category(2);
    assert_eq!(
        port.ui().get_sidebar_search().to_string(),
        fragment,
        "⑧ 换分类不许把用户敲的串一起擦掉"
    );
    let presets = bridge::sidebar_category_content(&view, 2);
    assert!(
        presets.is_empty(),
        "⑧ 前提：预置分类恒空（模型没有载体），实际 {:?}",
        presets.items
    );
    let tree = port.refresh_tree(None).expect("重抓");
    let empty = tree
        .find_by_id("sidebar-empty")
        .expect("⑧ 空分类必须画出理由");
    assert_eq!(
        empty.label, presets.empty_reason,
        "⑧ 空分类沿用**那一类自己的**理由（本类 0 条），⛔ 不是「都不匹配」那一档"
    );
    assert_eq!(
        empty.label,
        bridge::SIDEBAR_EMPTY_PRESETS_REASON,
        "⑧ 理由逐字 = 登记的模型缺口文案"
    );
    assert_eq!(
        port.ui().get_sidebar_item(),
        0,
        "⑧ 空分类的选中态落回哨兵 0"
    );

    port.ui().invoke_select_sidebar_category(1);
    assert_eq!(
        present_items(&mut port),
        shown,
        "⑧ 切回分类 1 ⇒ 同一份字重新过滤出同一批条目"
    );

    // ---- ⑨ 文本层接线（本机不编译 Slint 的那一半，读源码取证） ----
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let read = |relative: &str| -> String {
        std::fs::read_to_string(manifest_dir.join(relative))
            .unwrap_or_else(|error| panic!("读 {relative}: {error}"))
    };
    let sidebar = code_only(&read("ui/sidebar.slint"));
    assert_eq!(
        sidebar
            .matches("accessible-id: \"sidebar-search-field\";")
            .count(),
        1,
        "⑨ `{FIELD}` 在源码里必须**恰好声明一次**（语义节点挂在 `TextInput` 上，\
         外层矩形只做皮肤；两处都挂就是两个同名节点）"
    );
    assert!(
        sidebar.contains("text <=> root.search-text;"),
        "⑨ `text` 必须是**双向**绑定"
    );
    assert!(
        !sidebar.contains("text: root.search-text;"),
        "⑨ 单向 `text:` 会静默钉住根属性（用户第一次按键后各说各话）"
    );
    assert!(
        sidebar.contains("changed text => {")
            && sidebar.contains("root.search-changed(self.text);"),
        "⑨ 每敲一个字必须把整串报给宿主那唯一入口"
    );
    assert!(
        sidebar.contains("in-out property <string> search-text: \"\";"),
        "⑨ 承载字的属性必须是 `in-out`（`in` 的那一半由两向绑定写，界面也要能被宿主读到）"
    );
    assert!(
        sidebar.contains("visible: !root.compact;")
            && sidebar.contains("accessible-role: text-input;"),
        "⑨ 折叠断点下这个输入节点必须**离场**，且自带一次角色声明（⛔ 不依赖父矩形替我挡）"
    );
    let app = code_only(&read("ui/app.slint"));
    assert!(
        app.contains("search-text <=> root.sidebar-search;")
            && app.contains("in-out property <string> sidebar-search: \"\";"),
        "⑨ 主窗口必须把框里的字**两向**接到 `MainWindow.sidebar-search`"
    );
    assert!(
        app.contains("callback sidebar-search-changed(string);")
            && app.contains("root.sidebar-search-changed(query);"),
        "⑨ 主窗口必须把「敲了一个字」原样转给宿主"
    );
    let host_src = code_only(&read("src/host.rs"));
    assert_eq!(
        host_src.matches("ui.on_sidebar_search_changed(").count(),
        1,
        "⑨ 宿主必须是这个回调的**唯一**注册者（Slint 的 `on_*` 是覆盖语义，两份就会互吞）"
    );
    assert!(
        !host_src.contains("set_sidebar_search("),
        "⑨ 宿主⛔ 不写回那份字：字的唯一写者是输入框那条两向绑定，写回即成环"
    );
    assert!(
        host_src.contains("crate::bridge::filter_sidebar_content("),
        "⑨ 宿主必须调那个唯一的过滤口（⛔ 不在宿主里再写一份匹配逻辑）"
    );

    report_line(&format!(
        "[sidebar-search] 分类 1 全量 {} 条 {:?}；片段 {fragment:?} ⇒ {} 条 {:?}；\
         追加 {suffix:?}（`{query}`）⇒ 0 条 + 理由「{expected}」；清空还原；\
         换分类串保留（{}）",
        full.items.len(),
        full.items,
        shown.len(),
        shown,
        port.ui().get_sidebar_search()
    ));
}

/// 纯函数臂（⛔ 不装窗口）：把三条口径与两档空态在**手工夹具**上对账。
///
/// 为什么窗口里那份不够：真实素材池只有两条、且以中文为主，"大小写不敏感"这种性质
/// 拿它证不出来；而把措辞写进测试又会在改动文案时红。这里自建 ASCII 夹具，
/// 断言的输入输出都由夹具本身给出。
#[test]
fn filter_sidebar_content_is_a_pure_case_insensitive_projection() {
    let content = SidebarCategoryContent {
        items: vec![
            "Kick Drum".to_owned(),
            "snare".to_owned(),
            "hi-hat".to_owned(),
        ],
        empty_reason: String::new(),
    };

    // ① 空串 = 恒等。
    assert_eq!(bridge::filter_sidebar_content(&content, ""), content);
    // ③ 大小写不敏感（查询侧与名字侧都归一化）。
    assert_eq!(
        bridge::filter_sidebar_content(&content, "KICK").items,
        vec!["Kick Drum".to_owned()],
        "③ 大写片段必须命中首字母大写的名字"
    );
    assert_eq!(
        bridge::filter_sidebar_content(&content, "drum").items,
        vec!["Kick Drum".to_owned()],
        "③ 小写片段也必须命中名字里大写的后半段"
    );
    // ③ 保序（命中**非相邻**的两条才有意义：相邻时"排序"与"原序"看不出区别）。
    let picked = bridge::filter_sidebar_content(&content, "i");
    assert_eq!(
        picked.items,
        vec!["Kick Drum".to_owned(), "hi-hat".to_owned()],
        "③ `i` 命中第 1、3 条（跳过第 2 条）⇒ 顺序必须是原顺序（⛔ 不是命中强度排序）"
    );
    assert_eq!(picked.empty_reason, "", "③ 有命中时空态文案必须是空串");

    // ⑤ 一档：本类本来就 0 条 ⇒ 沿用原那份理由（⛔ 不许说成「没匹配上」）。
    let library = SidebarCategoryContent {
        items: Vec::new(),
        empty_reason: bridge::SIDEBAR_EMPTY_INSTRUMENTS_REASON.to_owned(),
    };
    let filtered = bridge::filter_sidebar_content(&library, "anything");
    assert!(filtered.items.is_empty());
    assert_eq!(
        filtered.empty_reason, library.empty_reason,
        "⑤ 库没下载时被用户搜了一个词 ⇒ 理由仍然是「库没下载」那句；两句混成一句就是假话"
    );

    // ⑤ 另一档：有 N 条、全都不匹配 ⇒ 新理由，带 query 与条数。
    let none = bridge::filter_sidebar_content(&content, "z");
    assert!(none.items.is_empty());
    assert_eq!(
        none.empty_reason,
        bridge::sidebar_no_match_reason("z", content.items.len()),
        "⑤ 「都不匹配」那一档的措辞与条数只有一处来源"
    );
    assert_ne!(none.empty_reason, library.empty_reason);
}
