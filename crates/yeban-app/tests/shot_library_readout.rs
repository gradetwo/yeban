//! 离屏取证（只出图，不带断言逻辑）：1200×700 下把音源管理打开到
//! "正在下载 42%" 的状态，验证三点视觉事实：
//!
//! 1. 读数（`42% · 12.1 / 66.8 MiB · 3.2 MiB/s`）落在**状态行**，够宽不被 elide；
//! 2. 失败态下「再试一次」出现在状态行右侧，且不与文字叠在一起；
//! 3. 面板在最小窗口里四个边都不出界（底部出口还在）。
//!
//! 产物：`target/ui-test-port/shot-library-readout.png`。

use std::rc::Rc;

use yeban_app::bridge;
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, write_artifact};

const NOW_MS: u64 = 1_760_000_000_000;

#[test]
fn shot_library_readout_and_retry() {
    let project = bridge::demo_project();
    let view = ViewState::from_project(&project).expect("投影");
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<截图>", "yeban-app", project.clone(), NOW_MS).expect("会话"),
    ));
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(Size::new(1200, 700), Permission::Interactive, None, || {
        host::build_main_window_with_console_tab(&view, &scene, 2)
    })
    .expect("Tier-1 平台");
    let ui = live.ui();
    host::wire_sound_library_dialog_with(ui, Some(Rc::clone(&port)));
    // ⚠ 必须走**打开回调**（`on_open_sound_library` 才是真正扫描已安装列表的地方）；
    // 直接 `set_sound_library_open(true)` 只会把面板挂上，列表永远是空的 —— 上一版
    // 截图里"已安装"一片空白就是这么来的（不是产品缺陷，是取景方式错了）。
    ui.invoke_open_sound_library();
    // 正在下载：进度条 + 一条真实形状的读数（与 `format_download_readout` 同格式）。
    ui.set_sound_downloading_id("karoryfer-emilyguitar".into());
    ui.set_sound_download_progress(42);
    ui.set_sound_library_status(
        yeban_app::sound_library::format_download_readout(
            42,
            12_700_000,
            Some(70_000_000),
            Some(3_400_000),
        )
        .into(),
    );
    ui.set_sound_library_status_is_error(false);
    let frame = live.window().capture().expect("抓帧（下载中）");
    write_artifact("shot-library-readout", &frame).expect("写截图工件（下载中）");

    // 失败态：同一行右侧出现「再试一次」，读数让位给按钮（不叠字）。
    ui.set_sound_library_status("下载失败：连接被重置（第 3 次重试仍失败）".into());
    ui.set_sound_library_status_is_error(true);
    ui.set_sound_library_retry_id("karoryfer-emilyguitar".into());
    let frame = live.window().capture().expect("抓帧（失败 + 重试）");
    write_artifact("shot-library-retry", &frame).expect("写截图工件（失败 + 重试）");
}
