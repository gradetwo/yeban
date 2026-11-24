//! 判据（T-FIX-304）：`app.slint` 里每个 `*-open` 开关都必须至少有一处渲染引用。
//!
//! 背景：`ai-drawer-open` 曾经只有声明行，快捷键 / 宿主回调 / `Esc` 三处入口都指向它，
//! 界面上什么都不出现，`Esc` 还会白吃一次。这条判据把「声明了却没人渲染」的开关直接判红，
//! 同一族也覆盖 T-FIX-143 那类零引用属性。
//!
//! 口径：声明形如 `in-out property <bool> <name>-open: ...;`；引用 = 文件里出现
//! `root.<name>-open`（`visible:` / 实例化条件 / 读写都算），声明行本身不计。

const APP_SLINT: &str = include_str!("../../../crates/yeban-app/ui/app.slint");

fn declared_open_flags(src: &str) -> Vec<String> {
    src.lines()
        .filter_map(|line| {
            let marker = "property <bool> ";
            let idx = line.find(marker)?;
            let rest = &line[idx + marker.len()..];
            let name = rest.split(':').next()?.trim();
            if name.ends_with("-open") {
                Some(name.to_string())
            } else {
                None
            }
        })
        .collect()
}

fn references(src: &str, flag: &str) -> usize {
    src.match_indices(&format!("root.{flag}")).count()
}

#[test]
fn every_open_flag_is_rendered_somewhere() {
    let flags = declared_open_flags(APP_SLINT);
    assert!(!flags.is_empty(), "没解析到任何 *-open 开关，判据本身失效");
    let mut offenders = Vec::new();
    for flag in &flags {
        if references(APP_SLINT, flag) == 0 {
            offenders.push(flag.clone());
        }
    }
    assert!(
        offenders.is_empty(),
        "以下 *-open 开关只有声明、没有任何 `root.<name>` 渲染引用（快捷键/回调会指向空气）：{offenders:?}"
    );
}

#[test]
fn ai_drawer_overlay_is_actually_instantiated() {
    // 本单的具体回归：`ai-drawer-open` 必须真的有一层覆盖层。
    assert!(
        APP_SLINT.contains("if root.ai-drawer-open : Rectangle"),
        "`ai-drawer-open` 的覆盖层没有实例化回 app.slint"
    );
    assert!(
        APP_SLINT.contains("root.musical-pr-open = true"),
        "AI 抽屉里的「打开提案审核」没有落点（应打开 Musical PR 抽屉）"
    );
}
