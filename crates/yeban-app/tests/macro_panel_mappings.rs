//! 宏面板的**级联呈现**判据（审计 `T-FIX-88` 项 1）。
//!
//! 台账 `docs/ledger/feature-alignment.md` 第 116 行当时记 GUI 列「无」——
//! 实测已过期：宏面板（`ui/dialogs/macro_panel.slint`）与滑块写入面
//! （`host::wire_macro_panel`）都在，缺的是**入口不通**（面板从未被 `MainWindow`
//! 实例化、下舱也没有按钮）与**级联去向不呈现**（投影只带 name/value）。
//!
//! 本文件钉四件事：
//! 1. 投影把 `MacroParameter::mappings` 逐条带出来（目标显示名 + 带符号 depth，
//!    顺序 = 模型顺序），与 name/value 同索引集；
//! 2. 空映射 ⇒ 摘要为空串（界面画「无映射」，⛔ 不画假箭头）；
//! 3. 面板**可达**：`app.slint` 里有 `if root.macro-panel-open` 的实例化、
//!    下舱有 `macro-open-button` 与 `open-macro-panel` 回调；
//! 4. 写通路**已存在**（T-FIX-96 落地 `Op::SetMacroMappings`）：判据断言 `Op` 枚举体
//!    含 `SetMacroMappings` 且带 `old`/`new` 陈旧载荷校验 ⇒ 宏映射的建立 / 改绑 /
//!    删除有了唯一写入载体；面板入口接线前，⛔ 界面不能绕过它直改内存模型。

use std::path::PathBuf;

use yeban_app::bridge::{
    MACRO_MAPPING_PANEL_NOT_WIRED, MacroRowView, ViewState, automation_target_display,
    macro_depth_display,
};
use yeban_model::YebanProjectV1;
use yeban_model::project::AutomationTarget;

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(relative: &str) -> String {
    let path = crate_root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("读不到 {relative}: {error}"))
}

#[test]
fn the_projection_carries_every_mapping_in_model_order() {
    let project: YebanProjectV1 = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("规范样本必须能投影");

    let names = view.macro_names();
    let values = view.macro_values();
    let summaries = view.macro_mapping_summaries();
    assert_eq!(
        names.len(),
        summaries.len(),
        "级联摘要必须与宏名**同索引集**（少一格就会串行）"
    );
    assert_eq!(values.len(), summaries.len());

    // 规范样本的 lead 轨：1 个宏 "Brightness"，1 条到 DeviceParam{slot 0, param 0}
    // 的映射，depth = 0.8。
    let brightness = names
        .iter()
        .position(|name| name == "Brightness")
        .expect("规范样本必须有 Brightness 宏");
    assert_eq!(
        summaries[brightness], "设备 0 参数 0 +0.8",
        "级联摘要必须逐字写出目标与带符号深度（模型顺序 = 投影顺序）"
    );
}

#[test]
fn an_empty_mapping_list_renders_as_an_empty_summary() {
    // 直接构造一行：没有映射 ⇒ 摘要空串（界面据此画「无映射」），
    // 且无障碍标签**不**多出一个空箭头。
    let row = MacroRowView {
        macro_index: 0,
        name: "Empty".to_owned(),
        value: 0.25,
        value_display: "25%".to_owned(),
        mappings: Vec::new(),
    };
    assert_eq!(row.mapping_summary(), "");
    assert_eq!(row.accessible_label(), "Empty · 25%");
    assert_eq!(row.label(), "Empty · 25%");
}

#[test]
fn every_automation_target_has_a_stable_display_name() {
    use yeban_model::EntityId;
    let track = EntityId::new();
    let cases = [
        (AutomationTarget::TrackVolume { track_id: track }, "音量"),
        (AutomationTarget::TrackPan { track_id: track }, "声相"),
        (
            AutomationTarget::SendGain {
                track_id: track,
                edge_id: EntityId::new(),
            },
            "发送增益",
        ),
        (
            AutomationTarget::DeviceParam {
                track_id: track,
                slot_index: 2,
                param_index: 7,
            },
            "设备 2 参数 7",
        ),
        (
            AutomationTarget::Macro {
                track_id: track,
                macro_index: 3,
            },
            "宏 3",
        ),
        (
            AutomationTarget::MidiCc {
                track_id: track,
                controller: 74,
            },
            "CC 74",
        ),
    ];
    for (target, expected) in cases {
        assert_eq!(automation_target_display(&target), expected);
    }
    // 深度恒带符号：正数 +、负数 −、零 +0。
    assert_eq!(macro_depth_display(6.0), "+6");
    assert_eq!(macro_depth_display(-0.5), "-0.5");
    assert_eq!(macro_depth_display(0.0), "+0");
}

#[test]
fn the_panel_is_reachable_from_the_live_window() {
    let app = read("ui/app.slint");
    assert!(
        app.contains("if root.macro-panel-open : macro_panel := MacroPanel {"),
        "宏面板必须在 MainWindow 里**实例化**（否则那三个数组与回调全是死接线）"
    );
    assert!(
        app.contains("macro-mapping-summaries: root.macro-mapping-summaries;"),
        "面板必须拿到级联摘要数组"
    );
    assert!(
        app.contains("open-macro-panel => { root.macro-panel-open = true; }"),
        "下舱按钮的上抛必须落到打开状态"
    );

    let dock = read("ui/lower_dock.slint");
    assert!(
        dock.contains("accessible-id: \"macro-open-button\""),
        "下舱必须有可命中的「宏 (N)」按钮（判据按无障碍 ID 找它）"
    );
    assert!(
        dock.contains("callback open-macro-panel();"),
        "下舱必须把打开手势上抛给宿主"
    );

    let panel = read("ui/dialogs/macro_panel.slint");
    assert!(
        panel.contains("in property <[string]> macro-mapping-summaries: [];"),
        "面板必须声明级联摘要的数据面"
    );
    assert!(
        panel.contains("\"-mappings\""),
        "每个宏行必须有一个 `macro-N-mappings` 的无障碍节点（判据与读屏都按它找）"
    );
}

/// `Op` 枚举体**有** `SetMacroMappings` 变体 ⇒ 宏映射的写入通路已存在。
///
/// 历史：T-FIX-88 发现缺口（本判据旧版断言 `!enum_body.contains("mappings")`）；
/// T-FIX-96（2026-11-24）落地 `Op::SetMacroMappings`（陈旧载荷校验 =
/// `old_mappings` 逐项相等），MCP 工具面经 `yeban_set_macro` 的可选 `mappings`
/// 参数接线。**仍然有效的纪律**：GUI 改映射必须走 `Op` 这条唯一写入载体
/// （`ADR-0031` D1），不许绕过它直接改内存模型 —— 面板的建立 / 改绑 / 删除
/// 入口接线即本判据的下一步（待后续线承接）。
#[test]
fn the_op_enum_has_a_macro_mapping_write_path() {
    let ops = read("../yeban-model/src/ops.rs");
    // ⚠ 锚点必须带花括号：`pub enum Op` 是 `pub enum OpOrigin` 的前缀，
    // 只按前缀找会截到 `OpOrigin` 的体内（那里的第一个断言会"恰好"通过）。
    let start = ops
        .find("\npub enum Op {")
        .expect("ops.rs 必须有 `pub enum Op {`");
    let body = &ops[start..];
    // 枚举体在本文件里以 `\n}\n` 结束（rustfmt 下顶层 `}` 行首）。
    let end = body.find("\n}\n").expect("枚举体必须以行首 } 结束");
    let enum_body = &body[..end];

    assert!(
        enum_body.contains("SetMacroMappings"),
        "`Op` 里没有 SetMacroMappings ⇒ 宏映射写路径被移除了，请同步面板与台账"
    );
    assert!(
        enum_body.contains("old_mappings") && enum_body.contains("new_mappings"),
        "SetMacroMappings 必须带陈旧载荷校验的 old/new 载荷"
    );
    assert!(
        enum_body.contains("SetMacro"),
        "前提判据自身失效：SetMacro 变体都找不到，说明切片的锚点不对"
    );
    assert!(
        MACRO_MAPPING_PANEL_NOT_WIRED.contains("写路径已存在")
            && MACRO_MAPPING_PANEL_NOT_WIRED.contains("T-FIX-96")
            && !MACRO_MAPPING_PANEL_NOT_WIRED.contains("写路径不存在"),
        "常量必须给用户一句话事实：写路径已存在（T-FIX-96），仅面板入口未接线；实际：{MACRO_MAPPING_PANEL_NOT_WIRED}"
    );
}
