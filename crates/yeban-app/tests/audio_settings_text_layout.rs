//! ⭐ **T-FIX-153**：音频设置面板的**长文本几何**判据。
//!
//! # 本文件要证的句子
//!
//! 「输入错误状态 / 长设备名 / 长说明」这三种长文本进面板后，**不裁、不溢出、
//! 不重叠**——用真实活窗口的包围盒逐条钉住，不是靠截图猜。
//!
//! # 修前的三个缺陷（实测 bounds，1280×800，面板打开）
//!
//! | 缺陷 | 修前 | 根因 |
//! | :--- | :--- | :--- |
//! | ① 长状态被裁 | `audio-device-status (376,344,512,36)` | 状态行只 36px（2 行 10px 字），宿主拼出的长设备名/错误超 2 行 ⇒ `clip` 吃掉 |
//! | ② 输入错误文字重叠 | 标签与 `audio-input-level-text` 同在 y=392 | 读数整宽右对齐，长错误向左压到左对齐的标签上 |
//! | ③ 说明行首行被裁 | 说明行 30px（2 行） | 约 100 字 `word-wrap` 要 4–5 行，30px 放不下 |
//!
//! # 修后（本判据钉住）
//!
//! - ① 状态行加高到 **60px**（≥4 行）：`(376,344,512,60)`；
//! - ② 标签与读数**分两行**：标签 `(…,410,512,16)`、读数 `(…,428,512,16)`，竖向不相交；
//! - ③ 说明行加高到 **84px**（≥5 行）且整体在面板内：`y=516..600 < 面板底 636`。
//!
//! 修法只动 `.slint` 表达式级几何（加高 + 下移 + 分行），控件数不变，按钮也整体下移
//! 到 y=470（仍在面板内）—— 与 `pane_no_overlap` 的"不靠删控件换绿"纪律同款。
//!
//! # 平台约束
//!
//! `slint::platform::set_platform` 线程局部、每线程一次 ⇒ 本文件只有一个 `#[test]`。

#![allow(unused)]
use slint::ComponentHandle as _;
use yeban_app::bridge::{ViewState, demo_project};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, Size, UiTestPort as _};

type Bounds = (i32, i32, u32, u32);

/// 两矩形是否相交（半开区间：边贴边不算相交，与 `pane_no_overlap` 同口径）。
fn intersect(a: Bounds, b: Bounds) -> bool {
    let (ax, ay, aw, ah) = a;
    let (bx, by, bw, bh) = b;
    ax < bx + bw as i32 && bx < ax + aw as i32 && ay < by + bh as i32 && by < ay + ah as i32
}

#[test]
fn long_text_does_not_clip_overflow_or_overlap() {
    let project = demo_project();
    let view = ViewState::from_project(&project).expect("演示工程必须能投影");
    let scene = DemoScene::from_view(&view);
    let mut port = LivePort::new(Size::new(1280, 800), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 主窗口");

    let ui = port.ui().clone_strong();
    ui.set_audio_settings_open(true);
    // 注入会触发三缺陷的长文本（设备名 + 采样率 + 声道 + 缓冲 + 名义值 + 重采样），
    // 以及一条很长的"输入不可用：<错误>"读数。
    ui.set_audio_device_status(
        "当前输出：USB Audio Interface  pianosample-macbook 12 · 48000 Hz · 2 声道 · 缓冲 512 帧（约 10.7 ms，名义值） · 工程 44100 Hz 实时转换中".into(),
    );
    ui.set_audio_input_level_text(
        "输入不可用：无法打开输入设备 Super Long Device Name That Used To Overflow The Readout"
            .into(),
    );
    port.refresh_tree(None).expect("必须能抓控件树");

    let rect_of = |id: &str| -> Bounds {
        let node = port
            .tree()
            .find_by_id(id)
            .unwrap_or_else(|| panic!("控件树里找不到 `{id}`"));
        let b = node.bounds.expect("运行时节点必须有包围盒");
        (b.x, b.y, b.width, b.height)
    };

    let panel = rect_of("audio-settings-panel");
    let status = rect_of("audio-device-status");
    let label = rect_of("audio-input-level-label");
    let readout = rect_of("audio-input-level-text");
    let explainer = rect_of("audio-settings-explainer");
    let tone = rect_of("audio-test-tone-button");

    // ① 状态行加高：≥ 60px（修前 36）。
    assert!(
        status.3 >= 60,
        "状态行必须 ≥60px（约 4 行 10px 字）以容纳长设备名/错误，实际 {}",
        status.3
    );

    // ② 标签与读数分行：竖向包围盒不相交（修前两者 y 相同 ⇒ 相交）。
    assert!(
        !intersect(label, readout),
        "输入电平标签与读数不得相交（长错误会压到标签上）：label={label:?} readout={readout:?}"
    );
    assert!(
        readout.1 > label.1,
        "读数必须在标签下方另起一行：label.y={} readout.y={}",
        label.1,
        readout.1
    );

    // ③ 说明行加高：≥ 84px（修前 30），且完整落在面板内。
    assert!(
        explainer.3 >= 84,
        "说明行必须 ≥84px（约 5 行）以容纳长文案，实际 {}",
        explainer.3
    );
    let panel_bottom = panel.1 + panel.3 as i32;
    assert!(
        explainer.1 + explainer.3 as i32 <= panel_bottom,
        "说明行必须完整落在面板内：bottom={} panel_bottom={}",
        explainer.1 + explainer.3 as i32,
        panel_bottom
    );

    // 边界：按钮整组下移后仍在面板内（没靠删控件/移出面板换绿）。
    assert!(
        tone.1 + tone.3 as i32 <= panel_bottom,
        "测试音按钮必须仍在面板内"
    );
    yeban_ui_test_port::render::report_line(
        "T-FIX-153: status=60px label/readout split explainer=84px all-in-panel",
    );
}
