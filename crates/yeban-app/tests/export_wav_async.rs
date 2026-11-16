//! **导出 WAV 不得阻塞 UI 线程**（性能实测 P1-A）。
//!
//! 背景：on_export_wav 原本在 Slint 事件循环线程上同步跑 export_wav_with_sfz_file；
//! 实测 60s≈9.7s，3 分钟曲约 25–30s，期间窗口不重绘、点不动、取消按不了
//! （用户以为程序崩了）。
//!
//! 修法：std::thread::spawn 跑离线渲染，upgrade_in_event_loop 回 UI 线程组装/写入回执
//! （与仓库既有 instrument-probe 工作线程同一模式）。跨线程前提已核：
//! YebanProjectV1 纯数据（无 Rc/RefCell），sfz 是 Arc<SfzInstrument>。
//!
//! ⚠ 纯 include_str! 判据（本机编不了 Slint）；线程/回执行为由 CI 真跑。

const HOST: &str = include_str!("../src/host.rs");

/// 截取 on_export_wav 回调区间（到 on_export_midi 之前）。
fn export_region(src: &str) -> &str {
    let start = src.find("ui.on_export_wav(").expect("必须有 on_export_wav");
    let end = src[start..]
        .find("ui.on_export_midi(")
        .map(|i| start + i)
        .unwrap_or(src.len());
    &src[start..end]
}

#[test]
fn export_wav_runs_off_the_ui_thread() {
    let region = export_region(HOST);
    let spawn = region
        .find("std::thread::spawn")
        .expect("导出必须搬到工作线程（P1-A）");
    let export = region
        .find("export_wav_with_sfz_file")
        .expect("必须调 export_wav_with_sfz_file");
    assert!(
        export > spawn,
        "export_wav_with_sfz_file 必须在 thread::spawn 闭包内，⛔ 不能留在 UI 线程"
    );
    assert!(
        region.contains("upgrade_in_event_loop"),
        "工作线程完成后必须回 UI 线程（upgrade_in_event_loop）写回执"
    );
}

#[test]
fn export_wav_receipt_is_written_on_the_ui_thread() {
    let region = export_region(HOST);
    let up = region.find("upgrade_in_event_loop").expect("必须有回跳");
    let apply = region
        .find("apply_module_receipt_with_clipping_warning(")
        .expect("必须写回执");
    assert!(
        apply > up,
        "回执写入必须在 upgrade_in_event_loop 闭包内（UI 线程），⛔ 不能在导出线程里碰 UI"
    );
}
