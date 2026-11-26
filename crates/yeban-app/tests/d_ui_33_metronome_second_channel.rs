//! T-FIX-556（`D-UI-33`）：节拍器开/关必须有颜色之外的第二可见通道。
//!
//! ## 问题（ISS-MWX2AC6X-91D7EC，T-FIX-550 审计上报）
//!
//! 节拍器按钮的两态此前只靠 ♩ 字形颜色区分（`transport.slint`
//! `color: root.metronome-on ? Tokens.playing : Tokens.ink-2`），背景只区分
//! pressed/hover ⇒ 开/关状态不能一眼可辨，色觉缺陷 / 低对比环境不可辨。
//!
//! ## 为什么是「源码不变量」而不是像素比对
//!
//! 与 `transport_play_glyph.rs` 同口径：两态差异是**纯 `.slint` 绑定**
//! （描边 `border-width`、指示灯 `visible` 都直接读 `root.metronome-on`），
//! 没有宿主侧 Rust 状态投影可断言；且 UI 金帧基准在本仓处于禁用状态
//! （T-FIX-94 裁决），像素比对当前不是可用证据。所以本判据钉**源码结构**。
//!
//! 本机等价红对照（python，同断言）在交付回执里记录：改前 3 红（①②③）、
//! 改后 10 绿；`git stash` 后复红、`stash pop` 后复绿，判据有效。
//!
//! ## 判据（每条都能被一次最小改动弄红）
//!
//! | # | 断言 | 弄红它的改动 |
//! | :--- | :--- | :--- |
//! | ① | 按钮声明 `border-width: root.metronome-on ? … : …`（外框通道） | 删描边 ⇒ 红 |
//! | ② | `border-color: Tokens.playing`（描边走功能色，非中性装饰色） | 换成恒定灰 ⇒ 红 |
//! | ③ | 存在 `visible: root.metronome-on` 的指示灯 `Rectangle` | 删指示灯 ⇒ 红 |
//! | ④ | 指示灯 `background: Tokens.playing` 且圆角非零（圆点形态） | 改透明/方形 ⇒ 红 |
//! | ⑤ | 颜色通道保留：`color: root.metronome-on ? Tokens.playing : Tokens.ink-2` | 改掉颜色门控 ⇒ 红 |
//! | ⑥ | `accessible-checkable: true` 且 `accessible-checked: root.metronome-on`（屏读不回退） | 删任一 ⇒ 红 |
//! | ⑦ | `metronome-on` 仍是 `in property`（宿主唯一写者，`D-UI-29` 口径） | 界面自造状态 ⇒ 红 |
//! | ⑧ | `clicked` 只发 `toggle-metronome` 回调、不赋值 `metronome-on` | 本地翻转 ⇒ 红 |

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

/// 节拍器按钮附近的源码窗口：`accessible-id` 锚点前 800 / 后 2200 字符。
/// 窗口必须盖住：按钮块声明（描边）、字形（颜色通道）、指示灯、TouchArea。
fn metronome_window(source: &str) -> String {
    let block = component_block(source, "Transport");
    let anchor = block
        .find("\"transport-metronome-button\"")
        .unwrap_or_else(|| panic!("找不到节拍器按钮锚点 transport-metronome-button"));
    let begin = anchor.saturating_sub(800);
    let end = (anchor + 2200).min(block.len());
    block[begin..end].to_owned()
}

#[test]
fn d_ui_33_border_is_state_gated() {
    let win = metronome_window(&read("transport.slint"));
    assert!(
        win.contains("border-width: root.metronome-on ?"),
        "判据①：外框通道缺失 —— border-width 必须由 root.metronome-on 门控（T-FIX-556 / D-UI-33）"
    );
}

#[test]
fn d_ui_33_border_uses_playing_color() {
    let win = metronome_window(&read("transport.slint"));
    assert!(
        win.contains("border-color: Tokens.playing"),
        "判据②：描边必须取 Tokens.playing 功能色（T-FIX-556 / D-UI-33）"
    );
}

#[test]
fn d_ui_33_indicator_dot_exists_and_gated() {
    let win = metronome_window(&read("transport.slint"));
    // 用花括号配平取指示灯块（⛔ 不按字节切片截窗：源码含中文注释，按字节会切进
    // 多字节字符内部 panic，同口径见 component_block）。
    let dot_anchor = win
        .find("visible: root.metronome-on")
        .and_then(|gate| win[..gate].rfind("Rectangle {"))
        .unwrap_or_else(|| {
            panic!(
                "判据③：找不到 visible: root.metronome-on 门控的指示灯 Rectangle（T-FIX-556 / D-UI-33）"
            )
        });
    let body = &win[dot_anchor..];
    let mut depth = 0usize;
    let mut opened = false;
    let mut end = body.len();
    for (offset, ch) in body.char_indices() {
        match ch {
            '{' => {
                depth += 1;
                opened = true;
            }
            '}' => {
                depth -= 1;
                if opened && depth == 0 {
                    end = offset + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    let body = &body[..end];
    assert!(
        body.contains("background: Tokens.playing"),
        "判据④a：指示灯必须用 Tokens.playing 填充"
    );
    let radius = body
        .find("border-radius:")
        .unwrap_or_else(|| panic!("判据④b：指示灯必须有非零 border-radius（圆点形态）"));
    let radius_expr = body[radius..].trim_start_matches("border-radius:").trim();
    assert!(
        radius_expr.starts_with("self.width / 2")
            || radius_expr.starts_with(|c: char| c.is_ascii_digit()),
        "判据④b：border-radius 必须是圆点形态（self.width / 2 或像素值），实际：{radius_expr}"
    );
}

#[test]
fn d_ui_33_color_channel_kept() {
    let win = metronome_window(&read("transport.slint"));
    assert!(
        win.contains("color: root.metronome-on ? Tokens.playing : Tokens.ink-2"),
        "判据⑤：颜色通道不得移除（第二通道是叠加，不是替换）"
    );
}

#[test]
fn d_ui_33_screen_reader_not_regressed() {
    let win = metronome_window(&read("transport.slint"));
    assert!(
        win.contains("accessible-checkable: true"),
        "判据⑥a：accessible-checkable 不得回退"
    );
    assert!(
        win.contains("accessible-checked: root.metronome-on"),
        "判据⑥b：accessible-checked 不得回退"
    );
}

#[test]
fn d_ui_33_single_writer_and_callback_only() {
    let block = component_block(&read("transport.slint"), "Transport");
    assert!(
        block.contains("in property <bool> metronome-on"),
        "判据⑦：metronome-on 必须保持 in property（宿主唯一写者，D-UI-29 口径）"
    );
    let win = metronome_window(&read("transport.slint"));
    assert!(
        win.contains("clicked => { root.toggle-metronome(); }"),
        "判据⑧a：clicked 必须只发 toggle-metronome 回调"
    );
    // TouchArea 处理器内不得给 metronome-on 赋值（本地翻转）。
    let area_start = win
        .find("metronome_area := TouchArea")
        .unwrap_or_else(|| panic!("判据⑧：找不到 metronome_area"));
    let area = &win[area_start..(area_start + 300).min(win.len())];
    assert!(
        !area.contains("metronome-on =") && !area.contains("metronome-on ="),
        "判据⑧b：TouchArea 不得本地翻转 metronome-on（宿主唯一写者）"
    );
}
