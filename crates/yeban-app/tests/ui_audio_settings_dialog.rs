//! **音频设置面板**（`ui/dialogs/audio_settings_dialog.slint`，T-FIX-71）—— 声明面对账判据。
//!
//! ## 本文件要证的句子
//!
//! 「设置 → 音频」这条线是**接通的**，不是画了一张图：
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（控件与注册表一致） | 面板 9 个静态语义 ID 全部在 `.slint` 里以声明的角色出现，且全部登记进元素注册表、类型一致、来源文件一致 |
//! | ②（回调链完整） | 面板 5 条回调在组件里声明、在 `app.slint` 的实例化处接上 `MainWindow` 的同名回调、`MainWindow` 声明了它们、宿主 `wire_audio_settings` 注册了全部 `on_*` 落点 |
//! | ③（入口真的存在） | 「设置」菜单有「音频...」项 → 命令 `open-audio-settings` → 宿主命令分发表有该臂 → `invoke_open_audio_settings`；`main.rs` 真的调用了 `wire_audio_settings` |
//! | ④（焦点闭环五格） | 面板有 `focused-control` 推导（五格）、`focus-control` 表覆盖 0..=4、trap 里 `Tab` 轮转与 `Esc` 关闭都在 |
//! | ⑤（持久化键） | 设置文件读写两侧都认 `audio_output_device` / `audio_input_device` 两个键（往返语义由 `settings.rs` 的单测钉住，这里钉"两边都在"） |
//!
//! ## 为什么不开窗
//!
//! 本文件走**声明面对账**（与 `export_dialog_button_focus.rs` 的出处纪律
//! 那条同款）：证的是"声明、注册、接线三份东西互相一致"。面板打开后的
//! 运行时行为（枚举供数、改选重开、电平跳动）需要真声卡环境才有意义，
//! 无声卡 CI 上只能验到"响亮失败"那一半 —— 那一半已由引擎侧判据
//! （`device::tests::open_output_named_with_an_absent_device_*` 等）与
//! 宿主纯函数判据（`audio_device_choices` / `audio_device_status_line`）
//! 钉住。本文件不重复它们。

use std::path::PathBuf;

use yeban_app::elements::{ElementKind, ElementRegistry};

fn source(relative: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读 {} 失败: {error}", path.display()))
}

const DIALOG_IDS: [(&str, ElementKind); 9] = [
    ("audio-settings-dialog", ElementKind::Region),
    ("audio-settings-panel", ElementKind::Region),
    ("audio-output-selector", ElementKind::Combobox),
    ("audio-input-selector", ElementKind::Combobox),
    ("audio-device-status", ElementKind::Text),
    ("audio-input-level-text", ElementKind::Text),
    ("audio-test-tone-button", ElementKind::Button),
    ("audio-refresh-devices-button", ElementKind::Button),
    ("audio-settings-close-button", ElementKind::Button),
];

/// ① 控件与注册表一致：ID 在 .slint 里出现、注册表里有、类型与来源对得上。
#[test]
fn dialog_ids_are_declared_and_registered_with_matching_kinds() {
    let dialog = source("ui/dialogs/audio_settings_dialog.slint");
    let registry = ElementRegistry::demo();
    for (id, kind) in DIALOG_IDS {
        assert!(
            dialog.contains(&format!("accessible-id: \"{id}\"")),
            ".slint 里必须声明 accessible-id {id}"
        );
        let meta = registry
            .get(id)
            .unwrap_or_else(|| panic!("元素注册表必须登记 {id}"));
        assert_eq!(meta.kind, kind, "{id} 的注册类型必须与声明一致");
        assert_eq!(
            meta.component, "dialogs/audio_settings_dialog.slint",
            "{id} 的注册来源必须是音频设置面板文件"
        );
    }
}

/// ② 回调链完整：组件声明 → 实例化接线 → MainWindow 声明 → 宿主落点，四处都在。
#[test]
fn panel_callbacks_are_wired_end_to_end() {
    let dialog = source("ui/dialogs/audio_settings_dialog.slint");
    let app = source("ui/app.slint");
    let host = source("src/host.rs");
    // (组件回调名, MainWindow 回调名, 宿主注册名)
    let callbacks = [
        ("close", "close-audio-settings", "on_close_audio_settings"),
        (
            "output-picked",
            "audio-output-picked",
            "on_audio_output_picked",
        ),
        (
            "input-picked",
            "audio-input-picked",
            "on_audio_input_picked",
        ),
        (
            "refresh-devices",
            "audio-refresh-devices",
            "on_audio_refresh_devices",
        ),
        (
            "play-test-tone",
            "audio-play-test-tone",
            "on_audio_play_test_tone",
        ),
    ];
    for (component_side, window_side, host_side) in callbacks {
        assert!(
            dialog.contains(&format!("callback {component_side}")),
            "面板组件必须声明 callback {component_side}"
        );
        assert!(
            app.contains(&format!("callback {window_side}")),
            "MainWindow 必须声明 callback {window_side}"
        );
        // 实例化接线有两种 Slint 语法：无参写 `name => { ... }`，带 int
        // 参数的写 `name(i) => { ... }`（具名参数）—— 两种都算接上。
        assert!(
            app.contains(&format!("{component_side} =>"))
                || app.contains(&format!("{component_side}(i) =>")),
            "app.slint 实例化处必须接上 {component_side}"
        );
        assert!(
            host.contains(&format!("ui.{host_side}(")),
            "宿主 wire_audio_settings 必须注册 {host_side}"
        );
    }
    // 打开回调同款（它不在面板组件里，入口在菜单/命令）。
    assert!(app.contains("callback open-audio-settings()"));
    assert!(host.contains("ui.on_open_audio_settings("));
    // 供数属性在 MainWindow 与实例化处都在。
    for property in [
        "audio-settings-open",
        "audio-output-device-labels",
        "audio-input-device-labels",
        "audio-device-status",
        "audio-input-level",
    ] {
        assert!(app.contains(property), "MainWindow 必须声明 {property}");
    }
    // T-FIX-131：`if` 门控改为 `visible:` 门控（`if` 内声明的 id 在组件作用域不可见，
    // `focus-audio-settings-dialog` 无法引用）。意图不变：默认关着 ⇒ 不可见 ⇒
    // 不入运行树、不画像素 ⇒ 默认帧不变（行为由 `tests/modal_focus_entry.rs`
    // 的入口/关闭断言与默认帧判据钉住，这里只钉接线语法）。
    assert!(
        app.contains("visible: root.audio-settings-open;"),
        "面板必须只在 audio-settings-open 时可见（默认关着 ⇒ 默认帧不变）"
    );
}

/// ③ 入口真的存在：菜单项 → 命令 → 分发表 → 回调，四环都在。
#[test]
fn the_settings_menu_entry_reaches_the_panel() {
    let app = source("ui/app.slint");
    let host = source("src/host.rs");
    let main = source("src/main.rs");
    assert!(
        app.contains("command-execute(\"open-audio-settings\")"),
        "「设置」菜单必须有走 open-audio-settings 命令的「音频...」项"
    );
    assert!(
        host.contains("\"open-audio-settings\" => ui.invoke_open_audio_settings()"),
        "宿主命令分发表必须有 open-audio-settings 臂"
    );
    assert!(
        main.contains("host::wire_audio_settings("),
        "main.rs 必须真的调用 wire_audio_settings（⛔ 不是只有声明）"
    );
}

/// ④ 焦点闭环五格：推导、序号表、trap 轮转与 Esc 都在面板文件里。
#[test]
fn the_dialog_has_a_five_cell_focus_loop() {
    let dialog = source("ui/dialogs/audio_settings_dialog.slint");
    for focus_id in [
        "audio-output-selector-focus",
        "audio-input-selector-focus",
        "audio-test-tone-focus",
        "audio-refresh-focus",
        "audio-settings-close-focus",
    ] {
        assert!(
            dialog.contains(&format!("{focus_id} := FocusScope")),
            "五格各自必须有取焦身份 {focus_id}"
        );
    }
    assert!(dialog.contains("out property <int> focused-control"));
    assert!(dialog.contains("public function focus-control(index: int)"));
    for cell in 0..=4 {
        assert!(
            dialog.contains(&format!("root.focus-control({cell})")),
            "轮转必须能到达第 {cell} 格"
        );
    }
    assert!(
        dialog.contains("audio-trap := FocusScope"),
        "模态必须有焦点陷阱（D-UI-19 口径）"
    );
    assert!(
        dialog.contains("root.close();"),
        "Esc / ✕ 必须走既有 close 回调"
    );
}

/// ⑤ 持久化键：settings.rs 的读、写两侧都认两个音频设备键。
#[test]
fn settings_persist_both_audio_device_keys() {
    let settings = source("src/settings.rs");
    for key in ["audio_output_device", "audio_input_device"] {
        assert!(
            settings.matches(key).count() >= 4,
            "{key} 必须在 Settings 字段/解析/写盘/存取函数多处出现（实际 {} 处）",
            settings.matches(key).count()
        );
    }
    assert!(
        settings.contains("pub fn save_audio_devices("),
        "必须有 save_audio_devices 存取函数（面板写回的唯一落点）"
    );
}
