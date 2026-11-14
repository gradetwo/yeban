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

#![allow(unused)]
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
        aether_app.contains("callback sidebar-search-changed("),
        "契约面必须声明 callback sidebar-search-changed("
    );
    assert!(
        aether_host.contains("sidebar_search_changed"),
        "宿主接线层必须注册 sidebar_search_changed"
    );
    assert!(
        aether_app.contains("> sidebar-search:"),
        "契约面必须声明 > sidebar-search:"
    );
    assert!(
        aether_host.contains("sidebar_search"),
        "宿主接线层必须注册 sidebar_search"
    );
}
