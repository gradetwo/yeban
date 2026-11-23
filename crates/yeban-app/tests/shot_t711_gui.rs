//! Hub #711 离屏取证：批 3 五项 GUI 面（OTCP 提交/状态/音符表、Songmap A/B、
//! 相位对齐、导出实测行）。
//!
//! 只出图不断言像素；判据在 `elements.rs` 的语义 ID 对账与各域单测。
//! 产物：`target/ui-test-port/shot-t711-*`（输出目录由 `write_artifact` 决定）。
//!
//! 1:1 是整窗原帧；3x 是同一帧上该面板区的最近邻放大（像素一位不动，只放大看）。

use slint::{ModelRc, SharedString, VecModel};
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, write_artifact};
use yeban_ui_test_port::{Rgb8Image, Size};

fn open_otcp(live: &LivePort<MainWindow>) {
    let ui = live.ui();
    ui.set_otcp_open(true);
    ui.set_otcp_clip_label("lead / 片段 01G7Y8".into());
    ui.set_otcp_job_id("01H2OTCPJOB0000000000000".into());
    ui.set_otcp_key_root("2".into());
    ui.set_otcp_scale("major".into());
    ui.set_otcp_mode("snapNotes".into());
    ui.set_otcp_tuning_hz("440".into());
    ui.set_otcp_status("succeeded · 音符 6 before→after · delivery pending/noActiveProject".into());
    let rows: Vec<SharedString> = vec![
        "#0   0.00– 1.20s  MIDI  57→ 57   +0.0¢  保持".into(),
        "#1   1.20– 2.40s  MIDI  59→ 60  -12.4¢  校正".into(),
        "#2   2.40– 3.60s  MIDI  62→ 62   +0.8¢  保持".into(),
        "#3   3.60– 4.80s  MIDI  64→ 65   +7.1¢  校正".into(),
        "#4   4.80– 6.00s  MIDI  67→ 67   -0.3¢  保持".into(),
        "#5   6.00– 7.20s  MIDI  69→ 70   +3.6¢  校正".into(),
    ];
    ui.set_otcp_notes_rows(ModelRc::new(VecModel::from(rows)));
}

fn open_phase(live: &LivePort<MainWindow>) {
    let ui = live.ui();
    ui.set_phase_align_open(true);
    ui.set_phase_align_reference("Lead Synth".into());
    ui.set_phase_align_target("Pad Bed".into());
    ui.set_phase_align_max_lag_ms("50".into());
    ui.set_phase_align_apply(true);
    ui.set_phase_align_status(
        "已测量并补偿 · lag 37 采样 / 2.31 ms · ρ=0.84 · confidence high · 延时 0→37 采样 · 极性翻转=false"
            .into(),
    );
}

fn open_song_ab(live: &LivePort<MainWindow>) {
    let ui = live.ui();
    ui.set_song_overview_collapsed(false);
    ui.set_song_overview_total_bars(64);
    ui.set_song_overview_ab_status(
        "A→B · 比 64 小节 · 变 5 · 增 0 删 0 · Δ张力 0.42 · ΔLUFS 2.4 LU · Δ频段 3.1 dB".into(),
    );
}

fn open_export(live: &LivePort<MainWindow>) {
    let ui = live.ui();
    ui.set_export_wav_open(true);
    ui.set_export_wav_path("/tmp/yeban-master.wav".into());
    ui.set_export_wav_measurement(
        "实测 · 真峰值 -0.84 dBFS · 积分响度 -14.2 LUFS · LRA 6.8 LU".into(),
    );
}

fn crop(frame: &Rgb8Image, x: u32, y: u32, w: u32, h: u32) -> Rgb8Image {
    let mut bytes = Vec::with_capacity((w * h * 3) as usize);
    for row in y..y + h {
        let start = ((row * frame.width() + x) * 3) as usize;
        let end = start + (w * 3) as usize;
        bytes.extend_from_slice(&frame.pixels()[start..end]);
    }
    Rgb8Image::from_raw(Size::new(w, h), bytes).expect("裁剪尺寸必须自洽")
}

fn upscale_3x(image: &Rgb8Image) -> Rgb8Image {
    let (w, h) = (image.width(), image.height());
    let mut bytes = Vec::with_capacity((w * h * 27) as usize);
    for row in 0..h {
        let start = (row * w * 3) as usize;
        let end = start + (w * 3) as usize;
        let line = &image.pixels()[start..end];
        for _ in 0..3 {
            for px in line.as_chunks::<3>().0 {
                for _ in 0..3 {
                    bytes.extend_from_slice(px);
                }
            }
        }
    }
    Rgb8Image::from_raw(Size::new(w * 3, h * 3), bytes).expect("放大尺寸必须自洽")
}

/// 写一张整窗 1:1 与一块面板区 3x；`rect` 是窗口里的面板包围盒。
fn write_pair(name: &str, frame: &Rgb8Image, rect: (u32, u32, u32, u32)) {
    write_artifact(&format!("shot-t711-{name}-1x"), frame).expect("写 1:1 截图");
    let panel = crop(frame, rect.0, rect.1, rect.2, rect.3);
    write_artifact(&format!("shot-t711-{name}-3x"), &upscale_3x(&panel)).expect("写 3x 截图");
}

fn shot_group(prefix: &str, width: u32, height: u32, scale: f32) {
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("规范样本必须能投影");
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(
        Size::new(width, height),
        Permission::Interactive,
        None,
        || host::build_main_window_with_console_tab(&view, &scene, 0),
    )
    .expect("Tier-1 平台");
    host::apply_view(live.ui(), &view, width as f32, 0.0);

    let px = |v: u32| (v as f32 * scale) as u32;

    open_otcp(&live);
    let frame = live.window().capture().expect("抓帧 OTCP");
    write_pair(
        &format!("{prefix}-otcp"),
        &frame,
        (px(455), px(185), px(690), px(500)),
    );
    live.ui().set_otcp_open(false);

    open_phase(&live);
    let frame = live.window().capture().expect("抓帧 相位");
    write_pair(
        &format!("{prefix}-phase"),
        &frame,
        (px(510), px(275), px(580), px(350)),
    );
    live.ui().set_phase_align_open(false);

    open_song_ab(&live);
    let frame = live.window().capture().expect("抓帧 Song A/B");
    write_pair(
        &format!("{prefix}-song-ab"),
        &frame,
        (px(0), px(518), px(860), px(34)),
    );

    open_export(&live);
    let frame = live.window().capture().expect("抓帧 导出测量");
    write_pair(
        &format!("{prefix}-export"),
        &frame,
        (px(510), px(270), px(580), px(360)),
    );
}

#[test]
fn shot_t711_gui_1x_and_3x() {
    shot_group("1600x900", 1600, 900, 1.0);
}
