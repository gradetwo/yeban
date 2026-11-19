//! 离屏取证（只出图）：把"扁平/拟物 × 编曲页/混音台"四张主界面拍下来，
//! 用来核对用户反复点名的三件事：
//!
//! 1. 混音台各通道条（含 Master / Bus）**底色是否同源**（同一 token，不各写一份）；
//! 2. 扁平主题按钮后面**有没有方块**（按钮背板阴影/描边留下的硬边）；
//! 3. 两套主题下信息层级是否"一眼抓重点"（网格线不抢、文字不糊）。
//!
//! 产物：`target/ui-test-port/shot-ui-*.png`。

use yeban_app::bridge;
use yeban_app::bridge::ViewState;
use yeban_app::cli::Texture;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, write_artifact};

const W: u32 = 1400;
const H: u32 = 900;

fn shot(name: &str, texture: Texture, console_tab: i32) {
    let project = bridge::demo_project();
    let view = ViewState::from_project(&project).expect("投影");
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(Size::new(W, H), Permission::Interactive, None, || {
        host::build_main_window_with_console_tab(&view, &scene, console_tab)
    })
    .expect("Tier-1 平台");
    host::apply_texture(live.ui(), texture);
    let frame = live.window().capture().expect("抓帧");
    write_artifact(name, &frame).expect("写截图工件");
}

#[test]
fn shot_arrangement_flat() {
    shot("shot-ui-arrangement-flat", Texture::Flat, 0);
}

#[test]
fn shot_mixer_flat() {
    shot("shot-ui-mixer-flat", Texture::Flat, 1);
}

#[test]
fn shot_arrangement_physical() {
    shot("shot-ui-arrangement-physical", Texture::Physical, 0);
}

#[test]
fn shot_mixer_physical() {
    shot("shot-ui-mixer-physical", Texture::Physical, 1);
}
