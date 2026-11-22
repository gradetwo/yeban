//! T-FIX-109：走带播放键在播放期间必须换成暂停符号。
//!
//! ## 为什么用「源码不变量」而不是像素比对
//!
//! 字形选择是**纯 `.slint` 绑定**（`IconTransportPlay.playing ← root.is-playing`），
//! 不在宿主侧计算，因此没有可断言的 Rust 状态投影；而 UI 金帧基准在本仓处于禁用状态
//! （基准待重录，见 T-FIX-94 的裁决），像素比对当前不是可用证据。所以本票钉的是
//! **源码结构**：字形面板（三角 / 双竖条）各自被哪个状态门控、按钮又把哪个属性喂给它。
//!
//! ## 判据（每条都能被一次最小改动弄红）
//!
//! | # | 断言 | 弄红它的改动 |
//! | :--- | :--- | :--- |
//! | ① | `IconTransportPlay` 声明 `in property <bool> playing` | 删掉新加属性 ⇒ 红 |
//! | ② | 按钮把 `playing: root.is-playing` 喂给它 | 删掉这一行 ⇒ 红（回到「永远三角」） |
//! | ③ | 三角（`M 0 0 L 10 5.5 L 0 11 Z`）**只在** `!root.playing` 时可见 | 去掉 `visible` ⇒ 红 |
//! | ④ | 两条竖条都只在 `root.playing` 时可见，且各自有实心填充 | 少一条 / 改门控 ⇒ 红 |
//! | ⑤ | `is-playing` 的唯一写者仍是宿主投影（`host::apply_transport` 同时写 `playing`） | 界面自造状态 ⇒ 红 |

use std::path::{Path, PathBuf};

fn ui_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("ui")
}

fn read(relative: &str) -> String {
    let path = ui_dir().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读 {} 失败：{error}", path.display()))
}

/// 取出 `component <name> ... { ... }` 的整块源码（按花括号配平，⛔ 不靠缩进猜）。
fn component_block(source: &str, name: &str) -> String {
    let header = format!("component {name} ");
    let start = source
        .find(&header)
        .unwrap_or_else(|| panic!("找不到组件 {name}"));
    let mut depth = 0usize;
    let mut opened = false;
    for (offset, ch) in source[start..].char_indices() {
        match ch {
            '{' => {
                depth += 1;
                opened = true;
            }
            '}' => {
                depth -= 1;
                if opened && depth == 0 {
                    return source[start..start + offset + 1].to_owned();
                }
            }
            _ => {}
        }
    }
    panic!("组件 {name} 的花括号没有配平");
}

#[test]
fn play_button_glyph_switches_to_pause_while_transport_runs() {
    let bar = read("transport_bar.slint");
    let icon = component_block(&bar, "IconTransportPlay");

    // ① 字形组件必须知道"正在播放"这件事。
    assert!(
        icon.contains("in property <bool> playing"),
        "IconTransportPlay 必须声明 playing 属性：\n{icon}"
    );

    // ② 按钮必须把走带状态喂进去（`is-playing` 是引擎读数的镜面）。
    assert!(
        bar.contains("playing: root.is-playing;"),
        "播放按钮必须把 root.is-playing 传给 IconTransportPlay"
    );

    // ③ 播放三角只在未播放时出现。
    let triangle_lines: Vec<&str> = icon
        .lines()
        .filter(|line| line.contains("M 0 0 L 10 5.5 L 0 11 Z"))
        .collect();
    assert_eq!(triangle_lines.len(), 1, "必须恰好一条播放三角路径");
    let triangle_index = icon
        .find("M 0 0 L 10 5.5 L 0 11 Z")
        .expect("三角路径必须存在");
    let triangle_scope = &icon[..triangle_index];
    let last_visible = triangle_scope
        .rfind("visible:")
        .map(|index| &triangle_scope[index..])
        .expect("三角路径必须带 visible 门控");
    assert!(
        last_visible.contains("!root.playing"),
        "三角必须由 !root.playing 门控，实得：{last_visible}"
    );

    // ④ 暂停字形 = 两条竖条，都由 root.playing 门控，都有实心 background。
    let pause_gated = icon
        .lines()
        .filter(|line| line.trim() == "visible: root.playing;")
        .count();
    assert_eq!(pause_gated, 2, "暂停字形必须恰好两条竖条各带一道门控");
    let filled = icon
        .lines()
        .filter(|line| line.trim() == "background: root.icon-color;")
        .count();
    assert_eq!(
        filled, 2,
        "两条竖条都必须用 icon-color 实心填充（实得 {filled} 处）"
    );

    // ⑤ 不许出现界面自造状态：字形只读 playing，不写。
    assert!(
        !icon.contains("playing ="),
        "字形组件不许回写 playing（唯一写者是 host::apply_transport）"
    );

    let host = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"))
        .expect("读 host.rs");
    assert!(
        host.contains("ui.set_is_playing(running);"),
        "is-playing 的唯一写者必须仍是 host::apply_transport 的投影"
    );
}
