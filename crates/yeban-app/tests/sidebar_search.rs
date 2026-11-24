//! ⭐ T-FIX-218 退役注：本文件头注的活窗口判据 ①–⑨（运行时树里的
//! `sidebar-search-field` / `sidebar-item-{i}` / `sidebar-empty`）已在新 UI 重构
//! （`21f70fd1`）时退役为契约面字符串判据；`ui/sidebar.slint` 本身也在
//! T-FIX-218 删除（旧 `Sidebar` 组件无人 import、不参与编译）。下面头注是
//! **历史记录**（判据曾经证什么），不是现行断言 —— 现行断言只有文件末尾那一个
//! `#[test]`（`callback sidebar-search-changed(` / `> sidebar-search:` /
//! 宿主 `sidebar_search_changed` / `sidebar_search` 四条字符串核对）。数据面仍活：
//! 投影经 `host::inject_sidebar` 注入 `MainWindow.sidebar-items`，搜索串经
//! `sidebar-search` 双向绑定。活窗口辅助里命名已删语义 ID 的两处（`FIELD` /
//! `present_items`）随本次删除一并移除；其余通用辅助（`live_window` /
//! `bounds_of` / `click` / `oracle` / `pick_fragment` / `pick_mismatch_suffix` /
//! `code_only`）是迁移留下的死代码，不引用已删面板，已由 T-FIX-395 删除（见收据）。
//!
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

// ⭐ T-FIX-218：`present_items` 已删 —— 它在运行时树里找已删的
// `sidebar-item-{i}`（`ui/sidebar.slint` 已删，无调用者）。

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
