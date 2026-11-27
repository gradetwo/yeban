//! **菜单命令完整性判据**：每个 `command-execute("X")` 必须有宿主落点。
//!
//! 背景（清单 P1-7 那类「半个功能」）：`.slint` 菜单写出 `command-execute("X")`，
//! 但宿主 `dispatch_aether_command` 里没有 `"X" =>` 臂 ⇒ 菜单项点下去**什么都不发生**，
//! 用户以为坏了。本判据把「菜单承诺的每条命令都在宿主出现过」做成常驻对账。
//!
//! ⚠ 口径说明：这里只断言命令字面量**在 host.rs 出现过**（不是 AST 级的「落在
//! `=>` 左侧」）—— 足以捕获「加了菜单忘加臂」这一类；更细的落点正确性由
//! `cli_contract` / 活窗口判据覆盖。纯 `include_str!`，`rustc --test` 可直跑。

const APP: &str = include_str!("../ui/app.slint");
const HOST: &str = include_str!("../src/host.rs");

/// 从 `.slint` 里提取所有 `command-execute("..."))` 的命令名。
fn menu_commands(src: &str) -> Vec<String> {
    const PAT: &str = "command-execute(\"";
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(i) = rest.find(PAT) {
        let after = &rest[i + PAT.len()..];
        if let Some(end) = after.find('"') {
            out.push(after[..end].to_string());
            rest = &after[end..];
        } else {
            break;
        }
    }
    out
}

/// ① 菜单里出现的每条命令，宿主里都必须有同名落点（否则是空菜单项）。
#[test]
fn every_menu_command_has_a_host_landing() {
    let cmds = menu_commands(APP);
    assert!(
        cmds.len() >= 40,
        "菜单命令只解析出 {} 条，明显少于实际（解析器可能被改坏）",
        cmds.len()
    );
    let mut missing = Vec::new();
    for c in &cmds {
        if !HOST.contains(&format!("\"{c}\"")) {
            missing.push(c.clone());
        }
    }
    assert!(
        missing.is_empty(),
        "菜单命令在 host.rs 里没有落点（点下去什么都不发生）：{missing:?}"
    );
}

/// ② P1-8：预备拍有「走带」菜单入口，且走与 Ctrl+Alt+D 同一条链。
#[test]
fn count_in_has_a_transport_menu_entry_and_host_arm() {
    assert!(
        APP.contains("预备拍开关：关 (Ctrl+Alt+D)")
            && APP.contains("预备拍开关：1 小节 (Ctrl+Alt+D)")
            && APP.contains("预备拍开关：2 小节 (Ctrl+Alt+D)"),
        "走带菜单必须有预备拍入口，且 0/1/2 三档都要能回显（清单 P1-8：藏在快捷键里等于没有）"
    );
    assert!(
        APP.contains("root.count-in-bars <= 0"),
        "菜单标题必须由 count-in-bars 驱动（⛔ 写死成 0/1 两档）"
    );
    assert!(
        APP.contains(r#"command-execute("transport-count-in")"#),
        "菜单项必须指向 transport-count-in"
    );
    assert!(
        HOST.contains(r#""transport-count-in" =>"#),
        "dispatch_aether_command 必须有 transport-count-in 臂"
    );
}
