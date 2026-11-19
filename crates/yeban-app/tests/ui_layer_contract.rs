//! **层与出口的守卫**（2026-11-19 用户截图点破的问题 → 变成机械检查）。
//!
//! 背景：四条现场缺陷（救援面板压住音源管理、下载看不见进度、换好音源后红字不散、
//! 横幅中英文叠字）当时**没有任何判据**能拦住 —— 因为它们是"两层同时在场""窗口一矮"
//! 这类状态里的问题，而既有判据都是一层一层单独开。本文件把其中**能从源码钉住的**
//! 三类钉死，剩下的用像素截图（1200×700 最小窗口）人工过一遍。
//!
//! | 不变量 | 为什么它能防回退 |
//! | :--- | :--- |
//! | ① 音源管理的 `z` **必须**大于救援面板的 `z` | 写反了就是用户截图里那张"音源管理在后面" |
//! | ② 两个弹层的面板尺寸都必须带 `Math.min(... , root.width/height - …)` 上限 | 上限一没，窗口一矮就把底部出口裁掉（"看不见先不管"） |
//! | ③ 打开音源管理之前必须先收起救援面板 | 否则两层叠着，下载进度被盖住 |
//! | ④ 引擎横幅的消息文本必须 `clip` + `elide` | 否则中文提醒与英文按钮抢同一行、叠字 |
//!
//! ## 反向腿
//! - 把 `app.slint` 里两处 `z:` 互换 ⇒ ① 红；
//! - 把 `missing_sounds_dialog.slint` 的 `panel-h` 改回"只按行数算" ⇒ ② 红；
//! - 删掉 `host::open_library_for` 里那句 `set_missing_sounds_open(false)` ⇒ ③ 红；
//! - 把横幅里那层 `clip: true` / `overflow: elide` 去掉 ⇒ ④ 红。

use std::path::Path;

fn read(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("读 {} 失败：{err}", path.display()))
}

/// ① 两层弹层的层级：音源管理在救援面板**之上**。
#[test]
fn the_sound_library_sits_above_the_rescue_panel() {
    let app = read("ui/app.slint");
    let rescue_z = z_after(&app, "if root.missing-sounds-open : MissingSoundsDialog {");
    let library_z = z_after(&app, "if root.sound-library-open : SoundLibraryDialog {");
    assert!(
        library_z > rescue_z,
        "音源管理（z={library_z}）必须盖在救援面板（z={rescue_z}）上面 —— \
         写反了就是用户截图里'点击下载声音管理在后面、看不见进度'"
    );
}

fn z_after(source: &str, anchor: &str) -> i64 {
    let start = source
        .find(anchor)
        .unwrap_or_else(|| panic!("app.slint 里找不到挂载点：{anchor}"));
    let tail = &source[start..];
    for line in tail.lines().take(12) {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("z:") {
            let number: String = rest
                .trim()
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect();
            return number.parse().unwrap_or_else(|_| panic!("z: 后面不是整数：{line}"));
        }
    }
    panic!("{anchor} 之后 12 行内没有 `z:`")
}

/// ② 两个弹层的面板尺寸都必须对窗口取上限（小屏 / 分屏也够得着底部出口）。
#[test]
fn both_dialog_panels_are_bounded_by_the_window() {
    let rescue = read("ui/dialogs/missing_sounds_dialog.slint");
    assert!(
        rescue.contains("panel-w: Math.min("),
        "救援面板宽度必须 `Math.min(…, root.width - …)`"
    );
    assert!(
        rescue.contains("panel-h: Math.min(Math.min(520px, root.height - 24px)")
            || (rescue.contains("panel-h:") && rescue.contains("root.height -")),
        "救援面板高度必须对 `root.height` 取上限：\n{}",
        rescue
            .lines()
            .find(|line| line.trim_start().starts_with("property <length> panel-h"))
            .unwrap_or("<找不到 panel-h>")
    );
    let library = read("ui/dialogs/sound_library_dialog.slint");
    assert!(
        library.contains("Math.min(820px, root.width - 32px)")
            && library.contains("Math.min(600px, root.height - 32px)"),
        "音源管理面板的宽/高都必须对窗口取上限"
    );
}

/// ③ 去下载时救援面板必须先让位（否则"下载进度"被它盖住）。
#[test]
fn opening_the_library_first_makes_the_rescue_panel_step_aside() {
    let host = read("src/host.rs");
    let anchor = "fn open_library_for";
    let start = host
        .find(anchor)
        .unwrap_or_else(|| panic!("host.rs 里找不到 {anchor}"));
    let body: Vec<&str> = host[start..].lines().take(40).collect();
    let body = body.join("\n");
    assert!(
        body.contains("set_missing_sounds_open(false)"),
        "open_library_for 必须在打开音源管理之前收起救援面板：\n{body}"
    );
}

/// ④ 引擎横幅：消息进裁剪块 + 省略号，动作按钮同样自缩。
#[test]
fn the_error_banner_lets_its_message_shrink_instead_of_overlapping() {
    let app = read("ui/app.slint");
    let start = app
        .find("accessible-id: \"engine-error-banner\";")
        .expect("app.slint 里必须有引擎横幅");
    let block: Vec<&str> = app[start..].lines().take(70).collect();
    let block = block.join("\n");
    assert!(
        block.contains("overflow: elide"),
        "横幅消息必须 `overflow: elide`（否则与右侧按钮叠字）：\n{block}"
    );
    assert!(
        block.contains("clip: true"),
        "横幅消息必须住在一个 `clip: true` 的弹性块里：\n{block}"
    );
}
