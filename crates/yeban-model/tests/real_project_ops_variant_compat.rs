//! **真实交付工程 `history.dag` 的 Ops variant 向前兼容判据**。
//!
//! 证据来源（2026-11-15 本机零编译解析，见 `docs/adr/ADR-0073`）：
//! - `夜半霓虹_v6_电影质感版.zip` → `夜半霓虹.yeban`（108 MB）内 `history.dag`
//!   含 163 个 commit、3742 条 op；逐个取 `ops[].op` 的 externally-tagged
//!   variant 名，去重得下列 21 个。
//! - `霞飞路夜雨_交付包.zip` → `霞飞路夜雨.yeban`（209 MB）的 `history.dag`
//!   为 40 字节空 DAG，不引入新 variant。
//!
//! 触发本判据的真实事故：本机 `target/release/yeban-mcp`（2026-11-06 编译）
//! 打开 v6 工程时报错 `unknown variant SetTrackPreset, expected one of [...]` ——
//! 旧构建不认识新 Ops variant ⇒ 整个工程打不开。当前 `ops.rs` 已支持该 variant，
//! 但此前**没有任何判据**把「真实交付件里出现过的 variant 必须被当前代码覆盖」
//! 钉住。本判据把这份兼容性契约固化为可回归的源码断言。
//!
//! 纯 `include_str!` 文本判据：零 GUI / 零音频 / 零重依赖，`rustc --test` 可直跑，
//! CI 的 `yeban-model` 腿亦会真跑（判据名出现在 `Running tests/` 日志里）。

/// 真实交付工程 `history.dag` 里实测出现的 `Op` variant 全集（21 个）。
/// ⛔ 新增 variant 若被真实工程使用，必须同时追加到此处与 `ops.rs` 的
/// `Op` 枚举 / `kind_name` / `apply` 分派，否则本判据变红。
const OBSERVED_VARIANTS: [&str; 21] = [
    "AddClip",
    "AddClipPlacement",
    "AddRoutingNode",
    "AddTrack",
    "Batch",
    "ConnectRouting",
    "DisconnectRouting",
    "InsertDevice",
    "ModifyNoteVelocity",
    "RemoveRoutingNode",
    "RemoveTrack",
    "SetAuthor",
    "SetKeySignature",
    "SetNoteExpression",
    "SetParam",
    "SetRoutingGain",
    "SetSection",
    "SetTrackColor",
    "SetTrackMute",
    "SetTrackPreset",
    "SetTrackSampler",
];

const OPS_SRC: &str = include_str!("../src/ops.rs");

/// 只取 `Op` 枚举声明体，避免把 `McpProposal` 之类其它枚举或注释里的
/// 同名字符串误当成声明。声明体从 `pub enum Op {` 起、到下一个顶层 `}` 止。
fn op_enum_body() -> &'static str {
    let start = OPS_SRC
        .find("pub enum Op {")
        .expect("ops.rs 必须定义 pub enum Op");
    let tail = &OPS_SRC[start..];
    let end = tail.find("\n}").expect("Op 枚举必须有闭合大括号");
    &tail[..end]
}

#[test]
fn every_variant_observed_in_real_projects_is_declared_in_the_op_enum() {
    let body = op_enum_body();
    for v in OBSERVED_VARIANTS {
        let declared = body.contains(&format!("    {v} {{"))
            || body.contains(&format!("    {v}("))
            || body.contains(&format!("    {v},"));
        assert!(
            declared,
            "真实工程出现过的 Op variant `{v}` 未在当前 Op 枚举中声明：\
             旧构建将无法反序列化该工程的 history.dag（见 ADR-0073）"
        );
    }
}

#[test]
fn every_observed_variant_is_dispatched_by_kind_name() {
    // `kind_name` 是每个 variant 的 serde 名称分派；缺一条 = 读得出枚举、
    // 但序列化/反序列化链路会漏掉它。逐字要求 `Self::V { .. } => "V",`。
    for v in OBSERVED_VARIANTS {
        let arm = format!("Self::{v} {{ .. }} => \"{v}\",");
        // Batch 是 tuple variant，形态为 `Self::Batch(..) => "Batch",`
        let tuple_arm = format!("Self::{v}(..) => \"{v}\",");
        assert!(
            OPS_SRC.contains(&arm) || OPS_SRC.contains(&tuple_arm),
            "Op::{v} 缺少 kind_name 分派臂；真实工程的 history.dag 会\
             在 serde 往返时丢失该 variant"
        );
    }
}

#[test]
fn the_set_track_preset_regression_is_pinned() {
    // 事故定点回归：2026-11-06 的 yeban-mcp 构建因缺 `SetTrackPreset`
    // 而拒绝打开 v6 工程。这条单独钉死它，避免"其它 20 个都在、偏偏它又被删"。
    assert!(
        op_enum_body().contains("    SetTrackPreset {"),
        "SetTrackPreset 必须保留在 Op 枚举：v6 交付工程的 history.dag 含 6 条该 op"
    );
    assert!(
        OPS_SRC.contains("Self::SetTrackPreset { .. } => \"SetTrackPreset\","),
        "SetTrackPreset 必须有 kind_name 分派臂"
    );
}
