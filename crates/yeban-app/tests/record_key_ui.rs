//! 判据：裸 `R` 从**真事件源**落到走带「录音」按钮**同一个** `toggle-record` 回调。
//!
//! 本切片补的是"录音只能靠鼠标点走带按钮"这一缺口（**界面决定**，⛔ 不是规范点名；
//! `[ARCH-REC-001]` 点名的是录音**能力**本身，已由 `host::wire_record_at` 落地）。
//!
//! ## 两层（不可互相代替）
//!
//! | 层 | 位置 | 证的事 |
//! | :--- | :--- | :--- |
//! | A 纯解析 | `src/input.rs` 的 `bare_r_toggles_record_and_does_not_disturb_its_neighbours` | 裸 `R` 解出 `ToggleRecord`；带修饰的邻居仍放行；文本框放行 |
//! | B 活窗口（本文件） | `build_live_ui_with` 装**生产同一个** `host::wire_keys`，再在根回调 `toggle-record` 上装计数器 | 按键 ⇒ 回调**恰好**一次；带修饰 ⇒ 不消费且计数不动 |
//!
//! ⚠ 本文件**不**开真设备：`toggle-record` 的产品实现（起录 / 停录 / 落盘）在
//! `host::wire_record_at`，判据在录音那几份文件里。本文件只钉"键盘按到了**那一个**回调" ——
//! 与 `src/elements.rs` 钉住"按钮按到了 `root.toggle-record()`"合起来就是完整链路
//! （`player` 侧的"切了状态"另有 `record_device_wiring.rs` 等文件）。
//!
//! ## 变异表（每条都写明它应当让谁变红）
//!
//! | 注入 | 预期 |
//! | :--- | :--- |
//! | 臂里改成 `ui.invoke_toggle_play()`（错回调） | 本文件红（计数不动） |
//! | 臂里返回 `false`（不消费） | 本文件红（第 ① 条） |
//! | 删掉 `resolve_logical` 第 6 步的 `Key::Character('r')` | 本文件红（键不消费）＋ `src/input.rs` 那条红 |
//! | 删掉 `apply_action` 的 `ToggleRecord` 臂 | 编译红（穷尽匹配，⛔ 没有 `_` 兜底） |
//! | 把 `cli.rs` 那一行的 `key` 写成 `KeyD` | `tests/cli_contract.rs` 的 B11b 红 |

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code，
// 而 `-D warnings` 会判红（与 `mixer_keyboard_switch.rs` 同一处理）。
#[allow(dead_code)]
mod live;

use std::cell::RefCell;
use std::rc::Rc;

use yeban_model::samples::filled_project;
use yeban_ui_test_port::port::Permission;

/// 裸键的文本形态 —— `logical()` 把它映射成 `Character('r')`。
const R: &str = "r";

#[test]
fn bare_r_is_a_second_entry_to_the_record_button_callback() {
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
    // 组件实例；`ui` 必须留在判据作用域里把组件撑着（与 `mixer_keyboard_switch.rs` 同款）。
    let window = slint::ComponentHandle::clone_strong(ui.ui());

    // 走带「录音」按钮调用的就是根上的这一个回调 ⇒ 计数器数的是"键盘按到它几次"。
    let hits = Rc::new(RefCell::new(0usize));
    let hits_for_callback = Rc::clone(&hits);
    window.on_toggle_record(move || {
        *hits_for_callback.borrow_mut() += 1;
    });

    // ① 裸 `R` ⇒ 消费 + 回调**恰好**一次。
    assert!(
        window.invoke_key_action(R.into(), false, false, false, false),
        "裸 `R` 必须被消费（录制开/关不冒泡给焦点系统）"
    );
    assert_eq!(
        *hits.borrow(),
        1,
        "一次按键只能让录音回调执行**恰好一次**（多一次 = 键被处理了两次；零次 = 按到了别处）"
    );

    // ② 再按一次 = 另一次拨动（开/关语义在回调里；键盘只负责"按到它"）。
    assert!(
        window.invoke_key_action(R.into(), false, false, false, false),
        "裸 `R` 第二次按也必须被消费"
    );
    assert_eq!(*hits.borrow(), 2);

    // ③ 邻居组合仍放行、且不惊动录音：`Ctrl+R` / `Shift+R` / `Alt+R`。
    for (shift, ctrl, alt) in [
        (false, true, false),
        (true, false, false),
        (false, false, true),
    ] {
        assert!(
            !window.invoke_key_action(R.into(), shift, ctrl, alt, false),
            "带修饰的 `R`（shift={shift} ctrl={ctrl} alt={alt}）必须放行"
        );
    }
    assert_eq!(*hits.borrow(), 2, "放行的组合不得触发录音回调");
}
