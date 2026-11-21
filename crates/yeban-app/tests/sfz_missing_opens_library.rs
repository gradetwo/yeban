//! 审计 `#21`：**缺 SFZ 文件时打开音源管理，必须走真正的打开路径**。
//!
//! ## 病灶
//!
//! `host::on_load_sfz` 在源文件缺失时原来是这么收尾的：
//!
//! ```ignore
//! ui.set_sound_library_open(true);
//! ```
//!
//! 面板确实弹出来了，但 `on_open_sound_library` 那一串准备动作**一步都没跑**：
//! 扫描已安装 → 状态行清零 → 发布"当前轨" → 目录状态归零 → **触发目录拉取**。
//! 用户看到的是一个空列表 —— "已安装"没有、可用音源也没有，于是不知道下一步点哪里。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（未打开 ⇒ 完整路径） | 缺文件 ⇒ `sound-library-open` 为真，且 `sound-catalog-state` 被**归零**（只有真跑过准备序列才会发生），回执仍是 `sfz-missing` |
//! | ②（已打开 ⇒ 不动状态） | 库已经开着时再触发 ⇒ 目录状态**保持原值**（不重置、不重拉），回执与提示照旧 |
//! | ③（源码形状） | 宿主里 `set_sound_library_open(true)` 只剩**一处**（打开回调自己），且它后面紧跟 `start_catalog_fetch()` |
//!
//! ## 为什么用哨兵值而不是等目录拉完
//!
//! 目录拉取在**工作线程**里跑，落地要经事件循环 —— 判据里跑事件循环就是引入
//! 时序（sleep／超时），而这里要证的是"**准备序列跑没跑**"，不是"拉取多快"。
//! 预置一个 `sound-catalog-state = 9` 的哨兵：只有 `on_open_sound_library` 会把
//! 它写成 `0`，老代码（只翻布尔量）碰都不会碰它。确定性因此来自代码路径本身。
//!
//! ⚠ 本文件只有**一个** `#[test]`：`slint::platform::set_platform` 是线程局部的，
//! `LivePort` 每线程只能装一次（同款约束见 `tests/shot_library_readout.rs`）。

use yeban_app::host::{self, ModulePaths};
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

const APP_SLINT: &str = include_str!("../ui/app.slint");
const HOST_RS: &str = include_str!("../src/host.rs");

/// 一个**一定不存在**的 SFZ 路径（带进程 id，避免并行/重跑时撞上残留文件）。
fn missing_sfz_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("yeban-audit21-absent-{}.sfz", std::process::id()))
}

#[test]
fn missing_sfz_opens_the_library_through_the_same_path_as_the_menu() {
    let path = missing_sfz_path();
    assert!(
        !path.is_file(),
        "夹具前提被破坏：{} 居然存在",
        path.display()
    );

    let project = yeban_app::bridge::demo_project();
    let view = yeban_app::bridge::ViewState::from_project(&project).expect("投影");
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(Size::new(1200, 700), Permission::Interactive, None, || {
        host::build_main_window_with_console_tab(&view, &scene, 0)
    })
    .expect("Tier-1 平台");
    let ui = live.ui();

    // 与生产同款的两次接线：音源管理对话框 + 四个模块入口（只用到 `load-sfz`）。
    host::wire_sound_library_dialog(ui);
    host::wire_module_actions_with(
        ui,
        None,
        ModulePaths {
            load_sfz: path.clone(),
            ..ModulePaths::default()
        },
    );

    // ── 判据 ①：库还没开过 ────────────────────────────────────────────────
    assert!(!ui.get_sound_library_open(), "前置：库默认是关着的");
    ui.set_sound_catalog_state(9); // 哨兵：只有真跑打开路径才会被写成 0
    ui.invoke_load_sfz();

    assert!(
        ui.get_sound_library_open(),
        "缺文件 ⇒ 必须把音源管理打开（用户要能在列表里挑一个）"
    );
    assert_eq!(
        ui.get_sound_catalog_state(),
        0,
        "缺文件分支必须走 `on_open_sound_library` 的准备序列（目录状态归零）；\
         只 `set_sound_library_open(true)` 会让哨兵留在 9"
    );
    // 既有回执与提示都还在：
    // - `module-status` 最终是提示条那句人话（`report_notice` 会覆盖它，这是既有顺序）；
    // - `module-status-text` 是 `humanize_module_receipt` 对**结构化回执**的翻译，
    //   只有 `apply_module_receipt` 写它 ⇒ 它不空就说明回执真的落过；
    // - `module-status-succeeded = false` 是回执的 `ok` 位（唯一写点同上）。
    let notice = ui.get_module_status().to_string();
    assert!(
        notice.contains("音源管理"),
        "原有的引导文案必须保留，实际：{notice}"
    );
    assert!(
        !ui.get_module_status_text().is_empty(),
        "结构化回执必须仍然落过（`sfz-missing` 的翻译行不该为空）"
    );
    assert!(
        !ui.get_module_status_succeeded(),
        "缺文件是失败路径 ⇒ 回执的 ok 位必须是 false"
    );

    // ── 判据 ②：库已经开着 ────────────────────────────────────────────────
    // 模拟"目录已经拉完"的稳态（`publish_sound_catalog` 之后是 2）。
    ui.set_sound_catalog_state(2);
    let before = ui.get_sound_library_open();
    ui.invoke_load_sfz();

    assert_eq!(
        ui.get_sound_library_open(),
        before,
        "已经开着 ⇒ 打开态一位不变"
    );
    assert_eq!(
        ui.get_sound_catalog_state(),
        2,
        "已经开着 ⇒ 不得把目录状态打回 0（否则列表被清空重拉，用户正看着的内容消失）"
    );
    let notice = ui.get_module_status().to_string();
    assert!(
        notice.contains("音源管理"),
        "第二次触发仍要如实给出引导文案，实际：{notice}"
    );

    // ── 判据 ③：源码形状（防回流） ─────────────────────────────────────────
    assert!(
        HOST_RS.contains("open_sound_library_for_missing_sfz(&ui);"),
        "缺文件分支必须调共享入口，而不是自己翻布尔量"
    );
    assert_eq!(
        HOST_RS.matches("ui.set_sound_library_open(true);").count(),
        1,
        "`set_sound_library_open(true)` 只允许留在打开回调自己那一处（两处就是两套打开路径）"
    );
    let open_arm = HOST_RS
        .split("ui.on_open_sound_library(move || {")
        .nth(1)
        .expect("宿主必须注册 on_open_sound_library");
    let open_arm = &open_arm[..open_arm.find("});").expect("闭包必须有收尾")];
    assert!(
        open_arm.contains("start_catalog_fetch();"),
        "打开回调必须触发目录拉取 —— 否则\"走完整路径\"只是换个说法"
    );

    // 面板与宿主的语义 id 仍是同一份（打开的是**那个**对话框，不是另画一个）。
    assert!(
        APP_SLINT.contains("if root.sound-library-open : SoundLibraryDialog"),
        "音源管理仍由 `sound-library-open` 挂载"
    );
}
