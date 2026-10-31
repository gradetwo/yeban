//! 判据：`Cmd/Ctrl+Alt+A` 从**真事件源**落到走带 AI 抽屉按钮**同一个** `toggle-ai-drawer` 回调。
//!
//! 本切片补的是"AI 抽屉只能点走带 `transport-ai-drawer-button`"这一缺口（**界面决定**，
//! ⛔ 不是规范点名；`[UI-NOTE-005]` 只讲音符）。抽屉是纯视图态（`MODEL-ISO-001` 的挥发性层）
//! ⇒ 不改工程、不进撤销栈。
//!
//! ## 两层（不可互相代替）
//!
//! | 层 | 位置 | 证的事 |
//! | :--- | :--- | :--- |
//! | A 纯解析 | `src/input.rs` 的 `ctrl_alt_a_is_the_ai_drawer_entry_and_does_not_disturb_its_neighbours` | `Cmd/Ctrl+Alt+A` 解出 `ToggleAiDrawer`；`Cmd` 侧同解；邻居仍放行；文本框放行 |
//! | B 活窗口（本文件） | `build_live_ui_with` 装**生产同一个** `host::wire_keys` | ① 按键 ⇒ `ai-drawer-open` **真的翻转**（端到端）；② 换计数器后 ⇒ 回调**恰好**一次；③ 邻居⇒ 不消费且计数不动；④ `Escape` ⇒ 收起开着的抽屉（`U-11`），关上之后如实放行 |
//!
//! ## 变异表（每条都写明它应当让谁变红）
//!
//! | 注入 | 预期 |
//! | :--- | :--- |
//! | 臂里改成 `ui.invoke_toggle_sidebar()`（错回调） | 本文件红（属性不翻） |
//! | 臂里返回 `false`（不消费） | 本文件红（第 ① 条） |
//! | 删掉 `resolve_logical` 第 3 步的 `Key::Character('a')` | 本文件红（键不消费）＋ `src/input.rs` 那条红 |
//! | 删掉 `apply_action` 的 `ToggleAiDrawer` 臂 | 编译红（穷尽匹配，⛔ 没有 `_` 兜底） |
//! | 把 `cli.rs` 那一行的 `modifiers` 写成 `Modifiers::ctrl()` | `tests/cli_contract.rs` 的 B11b 红 |

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code，
// 而 `-D warnings` 会判红（与 `record_key_ui.rs` 同一处理）。
#[allow(dead_code)]
mod live;

use std::cell::RefCell;
use std::rc::Rc;

use yeban_model::samples::filled_project;
use yeban_ui_test_port::port::Permission;

/// 裸键的文本形态 —— `logical()` 把它映射成 `Character('a')`。
const A: &str = "a";

#[test]
fn ctrl_alt_a_is_a_second_entry_to_the_ai_drawer_button_callback() {
    let project = filled_project();
    let ui = live::build_live_ui_with(
        &project,
        &live::LiveWiringOptions {
            permission: Permission::Interactive,
            ..live::LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + Tier-1 端口");
    // `clone_strong` 是 Slint 句柄的**共享**（`Rc` 语义）⇒ 与 `ui` 里那个活窗口是同一个
    // 组件实例；`ui` 必须留在判据作用域里把组件撑着（与 `record_key_ui.rs` 同款）。
    let window = slint::ComponentHandle::clone_strong(ui.ui());

    // ① 生产接线：按键 ⇒ `ai-drawer-open` 真的翻转（证明键打到了 `wire_view_callbacks`
    //    注册的那个回调，而不是某个只改属性、与按钮无关的旁路）。
    assert!(
        !window.get_ai_drawer_open(),
        "默认帧里 AI 抽屉应当是收起的（`.slint` 的初值）"
    );
    assert!(
        window.invoke_key_action(A.into(), false, true, true, false),
        "`Cmd/Ctrl+Alt+A` 必须被消费（抽屉开关不冒泡给焦点系统）"
    );
    assert!(window.get_ai_drawer_open(), "第一按 = 展开");
    assert!(
        window.invoke_key_action(A.into(), false, true, true, false),
        "第二按也必须被消费"
    );
    assert!(!window.get_ai_drawer_open(), "第二按 = 收起");

    // ② 落点身份：把回调换成计数器 ⇒ 一次按键让它执行**恰好一次**。
    let hits = Rc::new(RefCell::new(0usize));
    let hits_for_callback = Rc::clone(&hits);
    window.on_toggle_ai_drawer(move || {
        *hits_for_callback.borrow_mut() += 1;
    });
    assert!(window.invoke_key_action(A.into(), false, true, true, false));
    assert_eq!(*hits.borrow(), 1);

    // ③ 邻居组合仍放行、且不惊动抽屉：裸 `A` / `Alt+A` / `Shift+A` /
    //    `Cmd/Ctrl+Alt+Shift+A`。
    for (shift, ctrl, alt, meta) in [
        (false, false, false, false),
        (false, false, true, false),
        (true, false, false, false),
        (true, true, true, false),
    ] {
        assert!(
            !window.invoke_key_action(A.into(), shift, ctrl, alt, meta),
            "未绑定的 `A` 组合（shift={shift} ctrl={ctrl} alt={alt} meta={meta}）必须放行"
        );
    }
    assert_eq!(*hits.borrow(), 1, "放行的组合不得触发抽屉回调");
}

/// `U-11`（`~/zimmer-ui-mode-report.md`）：抽屉开着时 `Escape` 收起它。
///
/// 这是**面板收尾**（与时光机 / 导出弹窗同一条 `Action::Cancel` 链），⛔ 不是"放弃 AI
/// 提案"—— 拒绝仍只有 `musical-pr-reject-button`。判据落在**生产同一个** `wire_keys`
/// 上：抽屉关着时 `Escape` 必须如实放行（`tests/live_ui_mcp.rs` 对这条放行也有断言）。
#[test]
fn escape_closes_the_open_ai_drawer_and_then_passes_through() {
    let project = filled_project();
    let ui = live::build_live_ui_with(
        &project,
        &live::LiveWiringOptions {
            permission: Permission::Interactive,
            ..live::LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + Tier-1 端口");
    let window = slint::ComponentHandle::clone_strong(ui.ui());

    // 打开抽屉走**唯一写者**（与按钮 / 快捷键同一条回调），⛔ 不直接 set 属性 ——
    // 否则判据测到的只是"属性可写"，与真实打开路径无关。
    window.invoke_toggle_ai_drawer();
    assert!(window.get_ai_drawer_open(), "前置：抽屉应当已展开");

    assert!(
        window.invoke_key_action("\u{1b}".into(), false, false, false, false),
        "抽屉开着时 `Escape` 必须被消费（面板收尾不冒泡）"
    );
    assert!(
        !window.get_ai_drawer_open(),
        "`Escape` 必须收起抽屉（纯视图态）"
    );

    assert!(
        !window.invoke_key_action("\u{1b}".into(), false, false, false, false),
        "没有可收尾的对象时 `Escape` 如实放行（与接线前逐位相同）"
    );
}
