//! **判据：`SHORTCUTS` 表的 `implemented` 必须与宿主 `action_has_implementation` 一致**。
//!
//! 这条防的是一类具体的谎：`--print-shortcuts` 是用户/CI 核对键位的**官方口径**，
//! 若某行写着「能用」（`implemented: true`）而 `host::apply_action` 对它直接
//! `reject`，用户按下去没反应、表却已许下承诺。
//!
//! 历史：`cli_contract` 的 B11b 原本就对账这三面，后来被弱化成占位断言
//! （只读 `app.slint`/宿主的 `callback accept-ai-proposal`）。本判据把最核心的
//! 那一面（表 `implemented` ↔ 宿主 `action_has_implementation`）恢复成常驻对账。
//!
//! ⚠ 本机编不了 `yeban-app`（Slint 重依赖 ⇒ AGENTS §5），这条**只能靠 CI** 真跑；
//! 逻辑极简（一次遍历 + bool 比对），CI 失败信息会逐行打印不一致的表项。

use yeban_app::cli::shortcut_rows;
use yeban_app::host::action_has_implementation;

#[test]
fn shortcut_table_implemented_matches_host_claim() {
    let mut bad = Vec::new();
    for row in shortcut_rows() {
        let actual = action_has_implementation(row.action);
        if actual != row.implemented {
            bad.push(format!(
                "「{}」表 implemented={} 宿主 action_has_implementation={}",
                row.label, row.implemented, actual
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "--print-shortcuts 主表与宿主实现不一致（{} 处）：\n{}",
        bad.len(),
        bad.join("\n")
    );
}

/// 同一 `(key, modifiers)` 不应在表里出现两次（否则 `--print-shortcuts` 重影、语义歧义）。
#[test]
fn shortcut_table_has_no_duplicate_key_rows() {
    let rows = shortcut_rows();
    let mut dup = Vec::new();
    for (i, a) in rows.iter().enumerate() {
        for b in &rows[i + 1..] {
            if a.key == b.key && a.modifiers == b.modifiers {
                dup.push(format!("「{}」与「{}」同键 {:?}", a.label, b.label, a.key));
            }
        }
    }
    assert!(
        dup.is_empty(),
        "SHORTCUTS 有重复键绑定：\n{}",
        dup.join("\n")
    );
}
