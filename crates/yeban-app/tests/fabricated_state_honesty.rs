//! 编造状态的**诚实性**看守 —— 2026-10-30 批次。
//!
//! 本票删掉两处"看起来有、实际没有"的界面状态。删掉之后，**没有任何东西**
//! 阻止它们在后续提交里被静默画回来（双向注册表契约只管 `accessible-id` 是否登记，
//! 管不了一个编出来的三态图案）。本文件把"删掉的理由"钉在**源码层**：
//!
//! | 判据 | 钉住的事 |
//! | :--- | :--- |
//! | ① | `note-suggestion-overlay-rect` 不在 `.slint`（代码行）也不在注册表（代码行） |
//! | ② | Session 插槽格不编占用态 / 选中态：代码行里没有"已装载|空插槽"提示、没有 `.mod(3)` 图案、`accessible-item-selectable` / `selected` 都是字面 `false` |
//!
//! ## 为什么扫"代码行"而不是整份文件
//!
//! 两处删除都在原地留了注释（写清"为什么删、什么条件才允许画回来"），注释里**必然**
//! 出现被删的标识符与文案。整份文件做 `contains` 会把注释判成违规 ⇒ 这里先滤掉
//! **整行注释**（`trim_start().starts_with("//")`）再扫。行尾注释不滤：本文件看守的
//! 两处删除都没有行尾注释，滤不滤不影响读数，少一层解析就少一处能手滑的地方。
//!
//! ## 这不是像素判据
//!
//! 像素那一半由金标（`real_ui_tier1`）与运行时树（`live_ui_mcp.rs`）守；本文件
//! **不**编译 Slint，因此在本机也能跑（`AGENTS.md` §5.1 的"无重依赖 crate 的 test"）。

/// 卷帘源码（`include_str!`：文件被删 / 改名 ⇒ 编译期就红，判据不会"读不到就跳过"）。
const PIANO_ROLL_SLINT: &str = include_str!("../ui/console/piano_roll.slint");
/// Session 视图源码。
const SESSION_VIEW_SLINT: &str = include_str!("../ui/workspace/session_view.slint");
/// 元素注册表的唯一实现。
const ELEMENTS_RS: &str = include_str!("../src/elements.rs");

/// 滤掉**整行注释**之后剩下的源码行（理由见文件头）。
fn code_lines(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// **判据 ①**：AI 建议叠加层不再存在于代码里。
///
/// 删除理由（`.slint` 与注册表两处注释都写了同一条）：界面侧没有"当前待采纳的提案
/// 身份"这一表示（提案抽屉的四个数据面属性默认全空、没有宿主写者；证据见
/// `src/host.rs` 的 `action_has_implementation` 文档里 `AcceptAiSuggestion` 那一段）。
/// 那块 45% 半透明的"待采纳音符"因此是**编出来的状态**。
///
/// 两侧都要钉：只删 `.slint` 会被双向契约判红（注册表多一条），只删注册表也会被判红
/// （`.slint` 的 `accessible-id` 没登记）—— 但那条契约判红的是"不一致"，本判据钉的是
/// "**两边都没有**"，两者不重复。
#[test]
fn ai_suggestion_overlay_is_not_drawn_anymore() {
    let roll = code_lines(PIANO_ROLL_SLINT);
    assert!(
        !roll.contains("note-suggestion-overlay-rect"),
        "AI 建议叠加层被画回来了：卷帘源码的代码行里重新出现了 `note-suggestion-overlay-rect`。\
         界面侧仍然没有提案数据源 ⇒ 画它就是在谎报「有待采纳音符」"
    );
    assert!(
        !roll.contains("AI 建议音符 (待采纳)"),
        "AI 建议叠加层的假标签被写回来了：卷帘源码的代码行里重新出现了「AI 建议音符 (待采纳)」"
    );
    let registry = code_lines(ELEMENTS_RS);
    assert!(
        !registry.contains("note-suggestion-overlay-rect"),
        "注册表重新登记了 `note-suggestion-overlay-rect` —— 界面不画的东西不该留在注册表里"
    );
}

/// **判据 ②**：Session 插槽格不编"占用 / 选中"三态。
///
/// 删除理由：模型里没有 track×scene 插槽实体（`SceneV3` 没有槽位，`ClipPlacement`
/// 只有时间轴起点），宿主也没有任何插槽回调。原来 `(track_index + scene_index).mod(3)`
/// 同时决定了底色、"已装载 / 空插槽"提示文字与 `accessible-item-selected` —— 三处
/// 一起谎报。用户看得见，屏读器也读得到。
///
/// 真落地时的顺序（`.slint` 注释里也写着）：先有槽位来源（投影 → 注入平行数组），
/// 再把底色 / 文字 / 选中**分别**绑到真实读数上；在那之前三处一律不编。
#[test]
fn session_slots_do_not_fabricate_occupancy_or_selection() {
    let view = code_lines(SESSION_VIEW_SLINT);
    assert!(
        !view.contains("已装载") && !view.contains("空插槽"),
        "Session 插槽格又画了「已装载 / 空插槽」提示 —— 那是编出来的占用态"
    );
    assert!(
        !view.contains(".mod(3)"),
        "Session 插槽格又用了 `(track_index + scene_index).mod(3)` 之类的图案 —— \
         三态图案不是任何工程读数"
    );
    assert!(
        view.contains("accessible-item-selectable: false;"),
        "Session 插槽格必须如实声明 `accessible-item-selectable: false`（没有点击落点 ⇒ 不可选）"
    );
    assert!(
        view.contains("accessible-item-selected: false;"),
        "Session 插槽格必须如实声明 `accessible-item-selected: false`（没有选中状态 ⇒ 不声称已选中）"
    );
    assert!(
        view.contains("background: Tokens.bg-panel;"),
        "Session 插槽格的底色必须是一个固定令牌（每一格同一个样子），不是图案"
    );
}
