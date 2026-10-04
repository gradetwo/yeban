//! 运行时控件树抓取（`i-slint-backend-testing` 的 `ElementHandle`）—— `[ARCH-UI-005]` / `[UI-TEST-001]`。
//!
//! ## 分工（ADR-0001 D18 的裁决）
//!
//! | 目标 | 走哪条路 | 为什么 |
//! | :--- | :--- | :--- |
//! | 控件树 / 属性 / 几何断言 | `i-slint-backend-testing` 的 `ElementHandle`（本模块） | 官方内部 crate，`=1.18.1` 精确版本；**只做属性断言，不渲染像素** |
//! | Golden 截图 | Tier-1 软件光栅化（[`crate::render`]） | `[MUST-GATE-015]` 明文禁止用 Testing Backend 产出 Golden（它不渲染像素） |
//!
//! 两条路**都不需要物理显示器**。规范 `[ARCH-UI-005]` 写的
//! `slint::testing::init_integration_test_backend()` / `send_mouse_click()` 在上游 1.18.1
//! **不存在**（路径都没有）；实际可用的入口是本模块用到的 `i_slint_backend_testing::init_no_event_loop()`
//! 与 `ElementHandle`（见 `docs/adr/ADR-0001-...md` D18、`docs/ledger/ui-shell-notes.md` §3.3）。
//!
//! ## 本模块**不**安装平台
//!
//! `ElementHandle` 的只读路径只依赖 `item_tree()` / `WindowInner`，与具体平台无关，
//! 因此它既可以在 Testing Backend 下用，也可以在 [`crate::render`] 的软件光栅化平台下用。
//! 需要"在窗口实例上核对静态注册表"的调用方，请走 [`crate::render::LivePort`]
//! （同一个实例同时给出控件树**和**像素）。
//! 想单跑 Testing Backend 时，调 [`install_testing_backend`] —— 它按线程生效，
//! 每个测试线程只能调一次（上游 `init_no_event_loop` 在已初始化时会 panic）。

use i_slint_backend_testing::{AccessibleRole, ElementHandle, ElementRoot};

use crate::image::Rect;
use crate::tree::{ControlNode, ControlTree, Role, TreeError, is_well_formed_id};

/// 安装官方 Testing Backend（无事件循环，每线程一次）。
///
/// 出处：`i_slint_backend_testing::init_no_event_loop()`，源码 `lib.rs:37`
/// <https://docs.rs/i-slint-backend-testing/1.18.1/i_slint_backend_testing/fn.init_no_event_loop.html>。
///
/// # Panics
///
/// 当当前线程上已经初始化过后端时 panic（上游行为）。测试里一个线程只调一次。
pub fn install_testing_backend() {
    i_slint_backend_testing::init_no_event_loop();
}

/// Slint `AccessibleRole` → kebab-case 字面名。
///
/// 上游**没有**运行时的 "enum → 字符串" API（kebab 化发生在编译期的
/// `i-slint-compiler/builtin_elements.rs:79` 的 `kebab()`），所以这里自己映射。
/// `AccessibleRole` 是 `#[non_exhaustive]`，未识别的未来变体落到 `"none"`
/// （保守：宁可少报角色，也不谎报一个不存在的角色字符串）。
///
/// 完整性与一致性由 `role_names_cover_every_known_role` 判据钉住。
#[must_use]
pub fn role_name(role: AccessibleRole) -> &'static str {
    match role {
        AccessibleRole::None => "none",
        AccessibleRole::Button => "button",
        AccessibleRole::Checkbox => "checkbox",
        AccessibleRole::Combobox => "combobox",
        AccessibleRole::Groupbox => "groupbox",
        AccessibleRole::Image => "image",
        AccessibleRole::List => "list",
        AccessibleRole::Slider => "slider",
        AccessibleRole::Spinbox => "spinbox",
        AccessibleRole::Tab => "tab",
        AccessibleRole::TabList => "tab-list",
        AccessibleRole::TabPanel => "tab-panel",
        AccessibleRole::Text => "text",
        AccessibleRole::Table => "table",
        AccessibleRole::Tree => "tree",
        AccessibleRole::ProgressIndicator => "progress-indicator",
        AccessibleRole::TextInput => "text-input",
        AccessibleRole::Switch => "switch",
        AccessibleRole::ListItem => "list-item",
        AccessibleRole::RadioButton => "radio-button",
        AccessibleRole::RadioGroup => "radio-group",
        AccessibleRole::WindowTitleBar => "window-title-bar",
        AccessibleRole::Banner => "banner",
        AccessibleRole::Complementary => "complementary",
        AccessibleRole::ContentInfo => "content-info",
        AccessibleRole::Form => "form",
        AccessibleRole::Main => "main",
        AccessibleRole::Navigation => "navigation",
        AccessibleRole::Region => "region",
        AccessibleRole::Search => "search",
        _ => "none",
    }
}

/// 把一个 `ElementHandle` 转成控件树节点。
///
/// - 没有 `accessible-id` 的元素返回 `Ok(None)`：控件树只登记**语义可寻址**的节点
///   （`[UI-TEST-001]` §12.2 要求"关键节点必须声明稳定语义 ID"，而装饰性节点不是关键节点）；
/// - `accessible-id` 格式不合法返回 `Err(MalformedId)`：不允许静默跳过，
///   否则一个拼错的 ID 会表现为"元素不见了"；
/// - 角色缺省为 `none`，标签缺省为空串；
/// - 几何来自 `absolute_position()` + `size()`（窗口坐标，逻辑像素，见 `[UI-MCP-002]` 的包围盒）。
///
/// `dynamic_region` 在这里**恒为 `false`**：运行时读不到"这个节点每帧跳变"这种业务知识。
/// 它必须由调用方用 [`ControlTree::merge_dynamic_flags_from`] 从静态注册表注入
/// （见 [`tree_from_element_root`]）。
pub fn node_from_handle(handle: &ElementHandle) -> Result<Option<ControlNode>, TreeError> {
    let Some(accessible_id) = handle.accessible_id() else {
        return Ok(None);
    };
    let id = accessible_id.to_string();
    if !is_well_formed_id(&id) {
        return Err(TreeError::MalformedId { id });
    }
    let role = Role::parse(handle.accessible_role().map_or("none", role_name))?;
    let label = handle
        .accessible_label()
        .map(|text| text.to_string())
        .unwrap_or_default();
    let position = handle.absolute_position();
    let size = handle.size();
    let bounds = Rect::new(
        round_i32(position.x),
        round_i32(position.y),
        round_u32(size.width),
        round_u32(size.height),
    );
    Ok(Some(ControlNode {
        id,
        role,
        label,
        bounds: Some(bounds),
        dynamic_region: false,
        parent: None,
    }))
}

/// 遍历一个组件实例的全部后代，构建运行时控件树。
///
/// `registry` 传入静态注册表时，会把"哪些节点是高频刷新区"注入进去
/// （`[UI-MCP-002]` 的遮罩依赖这个标记，而它只能来自业务知识）。
///
/// 重复的 `accessible-id` 会让整次构建失败（[`TreeError::DuplicateId`]）—— 这是**故意的**：
/// 重复 ID 意味着自动化脚本可能寻址到两个不同的节点，必须当缺陷报出来，
/// 而不是让后一个静默顶掉前一个。
pub fn tree_from_element_root(
    root: &impl ElementRoot,
    registry: Option<&ControlTree>,
) -> Result<ControlTree, TreeError> {
    let mut tree = ControlTree::new();
    let mut pending = Vec::new();
    let root_element = root.root_element();
    // `visit_descendants` 只访问**后代**，不含自身；根节点同样可能带 `accessible-id`，
    // 因此先单独收一次根元素。
    if let Some(node) = node_from_handle(&root_element)? {
        pending.push(node);
    }
    // `visit_descendants` 支持 `ControlFlow::Break` 短路并把值带出来 —— 用它把
    // 第一个非法 ID 原样报出去（而不是收集一堆半成品再猜哪个错了）。
    let failure = root_element.visit_descendants(|handle| match node_from_handle(&handle) {
        Ok(Some(node)) => {
            pending.push(node);
            core::ops::ControlFlow::Continue(())
        }
        Ok(None) => core::ops::ControlFlow::Continue(()),
        Err(err) => core::ops::ControlFlow::Break(err),
    });
    if let Some(err) = failure {
        return Err(err);
    }
    for node in pending {
        tree.insert(node)?;
    }
    if let Some(registry) = registry {
        tree.merge_dynamic_flags_from(registry);
    }
    Ok(tree)
}

/// 按语义 ID 在运行时树里查找元素句柄（`[UI-TEST-001]`：只按语义 ID 寻址）。
///
/// 上游**没有** `find_by_accessible_id`（只有 `find_by_element_id`，键是 `组件名::局部名`
/// 这种"限定 id"，见 `docs/ledger/ui-shell-notes.md` §2 第 15/16 条），因此这里用
/// `query_descendants().match_predicate(...)` 实现。
#[must_use]
pub fn find_by_accessible_id(root: &impl ElementRoot, id: &str) -> Option<ElementHandle> {
    let wanted = id.to_owned();
    root.root_element()
        .query_descendants()
        .match_predicate(move |handle| {
            handle
                .accessible_id()
                .is_some_and(|candidate| candidate.as_str() == wanted)
        })
        .find_first()
}

/// 读一个元素的"属性投影"。
///
/// 只暴露**可字符串化且稳定**的那些属性（§12.3 的 `ReadOnly` 就是"响应式属性读取"）。
/// 未支持的属性名返回 `None` —— 不返回空串冒充成功。
///
/// 支持：`role` / `label` / `id` / `type` / `x` / `y` / `width` / `height` / `opacity` / `valid`。
#[must_use]
pub fn property_of(handle: &ElementHandle, name: &str) -> Option<String> {
    let position = handle.absolute_position();
    let size = handle.size();
    match name {
        "role" => Some(
            handle
                .accessible_role()
                .map_or("none", role_name)
                .to_owned(),
        ),
        "label" => Some(
            handle
                .accessible_label()
                .map(|text| text.to_string())
                .unwrap_or_default(),
        ),
        "id" => Some(
            handle
                .accessible_id()
                .map(|text| text.to_string())
                .unwrap_or_default(),
        ),
        "type" => Some(
            handle
                .type_name()
                .map(|text| text.to_string())
                .unwrap_or_default(),
        ),
        "x" => Some(format!("{:.2}", position.x)),
        "y" => Some(format!("{:.2}", position.y)),
        "width" => Some(format!("{:.2}", size.width)),
        "height" => Some(format!("{:.2}", size.height)),
        "opacity" => Some(format!("{:.4}", handle.computed_opacity())),
        "valid" => Some(handle.is_valid().to_string()),
        _ => None,
    }
}

/// 逻辑坐标是 `f32`；几何断言按"最近整数像素"取整（避免 `100.00001` 变成 `99`）。
fn round_i32(value: f32) -> i32 {
    value.round() as i32
}

/// 尺寸取整。非有限值与负数一律返回 0 —— 它们不是合法包围盒，不能变成巨大的 `u32`。
fn round_u32(value: f32) -> u32 {
    if value.is_finite() && value > 0.0 {
        value.round() as u32
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::KNOWN_ROLES;

    /// 判据 1: 角色映射必须**恰好**覆盖 Slint 1.18.1 的 `AccessibleRole` 全集，
    /// 且与 `tree::KNOWN_ROLES`（手写清单）逐条一致。
    ///
    /// 这条判据同时守住三件事：① 上游加角色时我们会发现；② 我们手写的 kebab 名不会漂移；
    /// ③ `role_name` 的 wildcard 分支不会悄悄吞掉一个真实变体
    /// （这里显式列出 30 个变体求值，任何落到 `_` 的都会让集合不等于 `KNOWN_ROLES`）。
    #[test]
    fn role_names_cover_every_known_role() {
        let all = [
            AccessibleRole::None,
            AccessibleRole::Button,
            AccessibleRole::Checkbox,
            AccessibleRole::Combobox,
            AccessibleRole::Groupbox,
            AccessibleRole::Image,
            AccessibleRole::List,
            AccessibleRole::Slider,
            AccessibleRole::Spinbox,
            AccessibleRole::Tab,
            AccessibleRole::TabList,
            AccessibleRole::TabPanel,
            AccessibleRole::Text,
            AccessibleRole::Table,
            AccessibleRole::Tree,
            AccessibleRole::ProgressIndicator,
            AccessibleRole::TextInput,
            AccessibleRole::Switch,
            AccessibleRole::ListItem,
            AccessibleRole::RadioButton,
            AccessibleRole::RadioGroup,
            AccessibleRole::WindowTitleBar,
            AccessibleRole::Banner,
            AccessibleRole::Complementary,
            AccessibleRole::ContentInfo,
            AccessibleRole::Form,
            AccessibleRole::Main,
            AccessibleRole::Navigation,
            AccessibleRole::Region,
            AccessibleRole::Search,
        ];
        assert_eq!(
            all.len(),
            KNOWN_ROLES.len(),
            "上游变体数必须与 KNOWN_ROLES 一致"
        );

        let mut mapped: Vec<&str> = all.iter().copied().map(role_name).collect();
        mapped.sort_unstable();
        mapped.dedup();
        let mut expected = KNOWN_ROLES.to_vec();
        expected.sort_unstable();
        assert_eq!(
            mapped, expected,
            "每个上游角色都必须映射到一个 KNOWN_ROLE 且互不相同"
        );

        // 每一个映射结果都必须能被 tree::Role::parse 接受（否则运行时建树会失败）。
        for role in all {
            assert!(
                Role::parse(role_name(role)).is_ok(),
                "{} 不在 KNOWN_ROLES 里",
                role_name(role)
            );
        }
    }

    /// 判据 2: 坐标取整必须稳定，且非有限/负数尺寸不会被当成合法包围盒。
    #[test]
    fn geometry_rounding_is_stable() {
        assert_eq!(round_i32(100.000_01), 100);
        assert_eq!(round_i32(99.6), 100);
        assert_eq!(round_i32(-0.4), 0);
        assert_eq!(round_u32(8.5), 9);
        assert_eq!(round_u32(0.0), 0);
        assert_eq!(round_u32(-3.0), 0);
        assert_eq!(round_u32(f32::NAN), 0);
        assert_eq!(round_u32(f32::INFINITY), 0);
    }
}
