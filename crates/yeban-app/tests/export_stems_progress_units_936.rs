//! T-FIX-936：分轨导出在途进度与终局回执 —— 不需要事件循环的判据。
//!
//! - 进度回调本身（后端在每条分轨落盘后报一次采样；总数在循环起时已定）；
//! - 进度写点的直接写入（`report_stems_progress` 把结构化在途回执＋人话读数
//!   写进常驻状态行，不走 toast）；
//! - 九语文案覆盖（起始文案／计数读数／建目录失败三条 Rust 侧表）。
//!
//! GUI 端到端臂（点菜单 → 同步写态 → 飞行中计数 → 常驻成功回执／建目录失败）
//! 在 `export_stems_progress_receipt_936.rs`（事件循环后端，单判据）。

use yeban_app::bridge::ViewState;
use yeban_app::cli::Language;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_model::ids::EntityId;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

fn build_ui() -> LivePort<yeban_app::ui::MainWindow> {
    let view = ViewState::demo();
    let scene = DemoScene::from_view(&view);
    LivePort::new(Size::new(1280, 800), Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("LivePort 必须装配 MainWindow")
}

#[test]
fn stems_progress_callback_reports_each_stem() {
    let scratch = std::env::temp_dir().join(format!(
        "yeban-936-progress-{}-{}",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    let out_dir = scratch.join("stems");
    std::fs::create_dir_all(&out_dir).expect("建分轨输出目录");
    let project = yeban_model::samples::filled_project();
    let project_path = scratch.join("Song.yeban");

    let mut samples: Vec<(usize, usize, Option<String>)> = Vec::new();
    let report = yeban_app::export_stems::export_stems_with_progress(
        &project,
        &project_path,
        &out_dir,
        None,
        &[],
        &mut |sample| {
            samples.push((sample.done, sample.total, sample.current.clone()));
        },
    )
    .expect("filled_project 分轨导出必须成功");
    assert!(!samples.is_empty(), "进度回调至少要报一次起始采样");
    let total = samples[0].1;
    assert!(total > 0, "起始采样必须带出总数：{samples:?}");
    assert_eq!(report.stems.len(), total, "回调总数必须等于实际落盘分轨数");
    assert_eq!(samples[0].0, 0, "第一条采样是起始（done == 0）");
    assert_eq!(samples[0].2, None, "起始采样没有刚完成的那条：{samples:?}");
    for (index, (done, sample_total, current)) in samples.iter().enumerate() {
        assert_eq!(*sample_total, total, "总数全程不变：{samples:?}");
        assert_eq!(*done, index, "done 必须逐条 +1：{samples:?}");
        if index == 0 {
            assert_eq!(*current, None);
        } else {
            assert!(
                current.as_ref().is_some_and(|name| !name.is_empty()),
                "完成采样必须带轨名：{samples:?}"
            );
        }
    }
    let (last_done, _, last_current) = samples.last().expect("至少一条采样");
    assert_eq!(*last_done, total, "最后一条采样必须报完：{samples:?}");
    assert!(
        last_current.is_some(),
        "最后一条采样必须带最后那条轨名：{samples:?}"
    );
    let manifest = out_dir.join("stems-manifest.json");
    assert!(manifest.exists(), "分轨 manifest 必须落盘");
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn stems_progress_write_lands_in_persistent_status_not_toast() {
    let port = build_ui();
    let ui = port.ui();

    // 先造一个旧的错误回执＋一条中性提示条，证明在途写点只改回执面、
    // 不碰提示条（提示条另有 `report_export_success` / `report_module_failure`
    // 管，不由在途写点越俎代庖；也不得把一条中性提示条翻成错误档）。
    ui.set_module_status_succeeded(false);
    ui.set_toast_is_error(false);
    ui.set_toast_message("旧提示条".into());
    ui.set_module_status("module:action=export-wav:outcome=error:error=io".into());
    ui.set_module_status_text("上一步没做成".into());

    let text = host::stems_progress_text(Language::ZhCn, 2, 5, Some("Lead"));
    host::report_stems_progress(ui, &text, 2, 5);
    assert_eq!(ui.get_module_status_text().as_str(), text);
    assert!(
        ui.get_module_status_succeeded(),
        "在途不是失败档：状态行必须中性显示"
    );
    let receipt = ui.get_module_status().to_string();
    assert!(
        receipt.contains("action=export-stems"),
        "在途回执点名动作：{receipt}"
    );
    assert!(
        receipt.contains("outcome=in-progress"),
        "在途回执必须标 in-progress：{receipt}"
    );
    assert!(receipt.contains("done=2"), "在途回执带 done：{receipt}");
    assert!(receipt.contains("total=5"), "在途回执带 total：{receipt}");
    assert_eq!(
        ui.get_toast_message().as_str(),
        "旧提示条",
        "在途写点⛔ 不得碰提示条（toast 4 秒就收走，撑不起长操作）"
    );
    assert!(!ui.get_toast_is_error(), "在途写点不得把提示条留在错误档");
}

#[test]
fn stems_progress_copy_covers_all_nine_languages() {
    assert_eq!(Language::ALL.len(), 9, "源语言＋八个目标语言");
    for lang in Language::ALL {
        let start = host::stems_progress_start_message(lang);
        let mkdir = host::stems_mkdir_failure_message(lang);
        assert!(!start.trim().is_empty(), "{lang:?} 缺在途起始文案");
        assert!(!mkdir.trim().is_empty(), "{lang:?} 缺建目录失败文案");
        let counted = host::stems_progress_text(lang, 1, 3, Some("Lead"));
        assert!(
            counted.contains('1') && counted.contains('3'),
            "{lang:?} 计数读数必须带 done/total：{counted}"
        );
        assert!(counted.contains("Lead"), "{lang:?} 计数读数必须带轨名");
    }
    let zh_start = host::stems_progress_start_message(Language::ZhCn);
    assert!(
        zh_start.contains("正在导出分轨"),
        "起始文案必须说清在导出分轨：{zh_start}"
    );
    let zh_mkdir = host::stems_mkdir_failure_message(Language::ZhCn);
    assert!(
        zh_mkdir.contains("无法创建") && zh_mkdir.contains("没有开始"),
        "建目录失败必须说清没开始＋原因：{zh_mkdir}"
    );
    assert!(
        zh_mkdir.contains("可写") || zh_mkdir.contains("换一个位置"),
        "建目录失败必须给下一步：{zh_mkdir}"
    );
    let zh_counted = host::stems_progress_text(Language::ZhCn, 2, 5, Some("Lead"));
    assert_eq!(zh_counted, "正在导出分轨 2/5 · Lead");
    let zh_start_counted = host::stems_progress_text(Language::ZhCn, 0, 5, None);
    assert_eq!(zh_start_counted, "正在导出分轨 0/5");
}
