//! T-FIX-82：四方对齐 TUI 补齐批 3 的判据。
//!
//! 四项各一组"红→绿"，每项都钉住"走的是共享生产路径、不是 TUI 另造一份"：
//!
//! | 项 | 判据 | 钉住的事实 |
//! | :--- | :--- | :--- |
//! | ① 鼓机生成 | `drum_command_lands_a_midi_clip_through_the_shared_recipe` | `:drum` 落 MIDI 片段，音符与 `yeban_theory::generate_drum_pattern` 逐条一致 |
//! | ① 复现性 | `same_seed_and_knobs_reproduce_the_same_note_content` | 同种子同参数两次生成，音符内容逐字段相同（MCP 契约的 TUI 面） |
//! | ① 响亮拒绝 | `drum_refuses_bad_style_and_knobs_without_writing` | 风格不存在 / 小节越界 / 旋钮越界都拒绝且工程一位不动 |
//! | ② 分轨导出 | `stems_export_writes_one_float_wav_per_source_track` | `:stems` 走 `yeban_app::export_stems`：N+1 次渲染、32-bit float、序号命名、manifest |
//! | ③ 诊断包 | `diagnostics_export_matches_the_gui_bundle_items` | 与 GUI 同一采集器、同一组条目（env/git/config/MANIFEST） |
//! | ④ 设备状态行 | `device_line_reports_both_rates_when_converting` | 转换中明示「工程 X Hz → 设备 Y Hz」，无设备时如实写「未开设备腿」 |
//! | ④ 设备腿 | `device_status_line_reads_the_live_host_and_refuses_a_fake_close` | 状态行读宿主实时读数；没开就不能假装关；开设备失败必须响亮 |

use std::path::{Path, PathBuf};

use yeban_model::music::MidiNote;
use yeban_model::project::{ClipContent, ClipPoolEntry, TrackKind};
use yeban_tui::app::{App, DeviceStatus};

fn demo_app() -> App {
    App::with_demo().expect("演示工程")
}

fn last_log(app: &App) -> String {
    app.log.last().cloned().unwrap_or_default()
}

fn temp_dir(tag: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("yeban-tui-p6-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("建临时目录");
    path
}

/// 选中指定轨（`App::selected` 是显示轨序下标）。
fn select_track(app: &mut App, id: yeban_model::ids::EntityId) {
    let index = app
        .track_ids
        .iter()
        .position(|candidate| *candidate == id)
        .unwrap_or_else(|| panic!("轨 {id} 不在显示轨序里"));
    app.selected = index;
}

/// 片段里的音符读数，按 `(tick, 音高, 力度, 时值)` 排序（跨进程稳定）。
fn note_rows(clip: &ClipPoolEntry) -> Vec<(u64, u8, u8, u64)> {
    let ClipContent::Midi { notes } = &clip.content else {
        panic!("鼓机片段必须是 MIDI 内容");
    };
    let mut rows: Vec<(u64, u8, u8, u64)> = notes
        .values()
        .map(|note: &MidiNote| {
            (
                note.start_tick,
                note.pitch,
                note.velocity,
                note.duration_ticks,
            )
        })
        .collect();
    rows.sort_unstable();
    rows
}

/// 取演示工程里第一条 MIDI 轨（避开主总线）。
fn first_midi_track(app: &App) -> yeban_model::ids::EntityId {
    app.project
        .tracks
        .values()
        .find(|track| track.kind == TrackKind::Midi && track.id != app.project.master_bus_track_id)
        .map(|track| track.id)
        .expect("演示工程至少有一条 MIDI 轨")
}

#[test]
fn drum_command_lands_a_midi_clip_through_the_shared_recipe() {
    let mut app = demo_app();
    let track = first_midi_track(&app);
    select_track(&mut app, track);
    let clips_before: Vec<_> = app.project.clip_pool.keys().copied().collect();
    let tracks_before = app.track_ids.len();

    app.execute_command("drum house 2 60 90 600 7");

    // 片段落库（且没有新建轨：选中的是 MIDI 轨 ⇒ 沿用该轨）。
    let new_clips: Vec<_> = app
        .project
        .clip_pool
        .keys()
        .filter(|id| !clips_before.contains(id))
        .copied()
        .collect();
    assert_eq!(
        new_clips.len(),
        1,
        "鼓机生成必须只落一个片段；日志：{}",
        last_log(&app)
    );
    assert_eq!(app.track_ids.len(), tracks_before, "选中 MIDI 轨不得新建轨");
    let clip = &app.project.clip_pool[&new_clips[0]];

    // 音符内容与 theory 生成层逐条一致（TUI 不另造生成算法）。
    let preset = yeban_theory::factory_drum_style("house")
        .expect("风格表可读")
        .expect("出厂风格 house 存在");
    let expected = yeban_theory::generate_drum_pattern(
        &preset,
        &yeban_theory::DrumGenParams {
            complexity: 60,
            intensity: 90,
            swing_permille: Some(600),
            seed: 7,
            bars: 2,
            fill: true,
        },
    )
    .expect("同参数生成");
    let mut expected_rows: Vec<(u64, u8, u8, u64)> = expected
        .iter()
        .map(|note| (note.tick, note.gm_key, note.velocity, note.duration_ticks))
        .collect();
    expected_rows.sort_unstable();
    assert!(!expected_rows.is_empty(), "生成结果不该为空");
    assert_eq!(
        note_rows(clip),
        expected_rows,
        "片段音符必须与 `generate_drum_pattern` 逐条相同"
    );

    // 摆放：落在选中轨、起点 0、时长 = 2 小节。
    let placement = app.project.tracks[&track]
        .clips
        .values()
        .find(|placement| placement.clip_id == new_clips[0])
        .expect("摆放必须在选中轨上");
    assert_eq!(placement.start_tick, 0, "与 GUI 同口径：鼓机从 tick 0 起");
    assert_eq!(
        placement.duration_ticks,
        2 * yeban_theory::TICKS_PER_BAR,
        "摆放时长 = 小节数 × 每小节 tick"
    );
    assert!(
        app.log.iter().any(|line| line.contains("鼓机生成")),
        "日志必须报出结果"
    );
}

#[test]
fn same_seed_and_knobs_reproduce_the_same_note_content() {
    let mut app = demo_app();
    let track = first_midi_track(&app);
    select_track(&mut app, track);
    let initial: Vec<_> = app.project.clip_pool.keys().copied().collect();

    app.execute_command("drum techno 2 70 60 550 42 nofill");
    let after_first: Vec<_> = app
        .project
        .clip_pool
        .keys()
        .copied()
        .filter(|id| !initial.contains(id))
        .collect();
    assert_eq!(
        after_first.len(),
        1,
        "第一次生成应落一个片段；日志：{}",
        last_log(&app)
    );

    app.execute_command("drum techno 2 70 60 550 42 nofill");
    let after_second: Vec<_> = app
        .project
        .clip_pool
        .keys()
        .copied()
        .filter(|id| !initial.contains(id) && !after_first.contains(id))
        .collect();
    assert_eq!(
        after_second.len(),
        1,
        "第二次生成应再落一个新片段；日志：{}",
        last_log(&app)
    );

    let first_clip = &app.project.clip_pool[&after_first[0]];
    let second_clip = &app.project.clip_pool[&after_second[0]];
    assert_eq!(
        note_rows(first_clip),
        note_rows(second_clip),
        "同种子同参数 ⇒ 音符内容必须逐字段相同"
    );
    assert_ne!(
        first_clip.id, second_clip.id,
        "两次生成是两条独立片段（身份不同、内容相同）"
    );
}

#[test]
fn drum_refuses_bad_style_and_knobs_without_writing() {
    let mut app = demo_app();
    let track = first_midi_track(&app);
    select_track(&mut app, track);
    let clips_before = app.project.clip_pool.len();

    app.execute_command("drum not-a-style");
    assert!(
        last_log(&app).contains("没有名为 `not-a-style` 的鼓机风格预置"),
        "未知风格必须响亮拒绝：{}",
        last_log(&app)
    );
    app.execute_command("drum house 17");
    assert!(
        last_log(&app).contains("1..=16"),
        "小节越界必须响亮拒绝：{}",
        last_log(&app)
    );
    app.execute_command("drum house 2 101");
    assert!(
        last_log(&app).contains("0..=100"),
        "复杂度越界必须响亮拒绝：{}",
        last_log(&app)
    );
    app.execute_command("drum house 2 50 50 400");
    assert!(
        last_log(&app).contains("500..=1000"),
        "摇摆越界必须响亮拒绝：{}",
        last_log(&app)
    );
    app.execute_command("drum house 2 fifty");
    assert!(
        last_log(&app).contains("不是数字"),
        "非数字旋钮必须响亮拒绝：{}",
        last_log(&app)
    );
    assert_eq!(
        app.project.clip_pool.len(),
        clips_before,
        "被拒绝的生成不许落任何片段"
    );
    app.execute_command("drum");
    assert!(
        last_log(&app).contains("用法：:drum"),
        "空参数必须给用法与可用风格：{}",
        last_log(&app)
    );
}

#[test]
fn stems_export_writes_one_float_wav_per_source_track() {
    let mut app = demo_app();
    let dir = temp_dir("stems");
    let report = app.export_stems(&dir).expect("分轨导出");

    assert!(!report.stems.is_empty(), "演示工程必须有可导出的源轨");
    assert_eq!(
        report.renders,
        report.stems.len() + 1,
        "N 条分轨 + 1 次全工程基线渲染"
    );
    assert_eq!(report.sample_rate, 48_000, "工程采样率");
    for stem in &report.stems {
        assert!(stem.path.is_file(), "分轨必须落盘：{}", stem.path.display());
        assert_eq!(stem.bit_depth, 32, "分轨恒为 32-bit float");
        let (index, _) = stem
            .file_name
            .split_once('.')
            .unwrap_or_else(|| panic!("命名口径 {{序号}}.{{轨名}}.wav：{}", stem.file_name));
        assert!(
            index.len() >= 4 && index.chars().all(|ch| ch.is_ascii_digit()),
            "序号至少 4 位：{}",
            stem.file_name
        );
        assert!(stem.file_name.ends_with(".wav"));
        let bytes = std::fs::read(&stem.path).expect("读分轨");
        assert_eq!(&bytes[0..4], b"RIFF");
        // WAV fmt 块：audio_format = 3（IEEE float）、bits = 32。
        assert_eq!(u16::from_le_bytes([bytes[20], bytes[21]]), 3);
        assert_eq!(u16::from_le_bytes([bytes[34], bytes[35]]), 32);
        assert_eq!(stem.bytes, bytes.len());
    }
    assert!(
        report
            .skipped
            .iter()
            .any(|skipped| skipped.reason == "masterBusTrack"),
        "主总线必须点名跳过而不是静默丢轨：{:?}",
        report
            .skipped
            .iter()
            .map(|skipped| skipped.reason)
            .collect::<Vec<_>>()
    );
    assert!(
        dir.join("stems-manifest.json").is_file(),
        "批次索引必须在 {} 落盘",
        dir.display()
    );
    // 回执行与 GUI / CLI 同一份（`ExportStemsReport::lines`）。
    assert!(
        app.log
            .iter()
            .any(|line| line.starts_with("exported-stems:"))
    );
    assert!(app.log.iter().any(|line| line.starts_with("stem:")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn diagnostics_export_matches_the_gui_bundle_items() {
    let mut app = demo_app();
    let dir = temp_dir("diag");
    let receipt = app.export_diagnostics(Some(&dir)).expect("诊断包");

    assert!(receipt.path.is_file(), "诊断包必须落盘");
    assert_eq!(
        receipt.bytes,
        std::fs::metadata(&receipt.path).expect("元数据").len(),
        "回执字节数必须等于盘上字节数"
    );
    assert_eq!(receipt.sha256.len(), 64, "sha256 必须是 64 位十六进制");
    assert!(receipt.sha256.chars().all(|ch| ch.is_ascii_hexdigit()));
    // 与 GUI 的 `UiAction::ExportDiagnostics` 同一组输入 ⇒ 同一组条目。
    for expected in ["env.txt", "git.txt", "config.json", "MANIFEST.txt"] {
        assert!(
            receipt.entries.iter().any(|name| name == expected),
            "缺条目 `{expected}`：{:?}",
            receipt.entries
        );
    }
    assert!(
        app.log.iter().any(|line| line.contains("诊断包已写出")),
        "日志必须报出落点"
    );

    // 目录不存在 ⇒ 响亮拒绝（不偷偷换成临时目录）。
    let missing = dir.join("nope");
    let error = app
        .export_diagnostics(Some(Path::new(&missing)))
        .expect_err("不存在的目录必须拒绝");
    assert!(error.contains("目录不存在"), "{error}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn device_line_reports_both_rates_when_converting() {
    let converting = DeviceStatus {
        device_name: Some("Test Device".to_owned()),
        source_rate: Some(48_000),
        device_sample_rate: Some(44_100),
        project_sample_rate: 48_000,
    };
    let line = converting.line();
    assert!(line.contains("Test Device"), "{line}");
    assert!(line.contains("设备采样率 44100 Hz"), "{line}");
    assert!(line.contains("工程采样率 48000 Hz"), "{line}");
    // T-FIX-72 的原话：转换中必须把两侧数值都写出来。
    assert!(
        line.contains("采样率转换: 工程 48000 Hz → 设备 44100 Hz"),
        "转换状态必须明示两侧数值：{line}"
    );

    let equal = DeviceStatus {
        source_rate: Some(48_000),
        device_sample_rate: Some(48_000),
        ..converting.clone()
    };
    assert!(equal.line().contains("无采样率转换"), "{}", equal.line());

    let no_device = DeviceStatus {
        device_name: None,
        source_rate: None,
        device_sample_rate: None,
        project_sample_rate: 48_000,
    };
    let honest = no_device.line();
    assert!(honest.contains("未开设备腿"), "{honest}");
    assert!(honest.contains("设备采样率 —"), "{honest}");
    assert!(honest.contains("工程采样率 48000 Hz"), "{honest}");
    assert!(honest.contains("无（未开设备腿）"), "{honest}");
}

#[test]
fn device_status_line_reads_the_live_host_and_refuses_a_fake_close() {
    let mut app = demo_app();
    let line = app.device_status_line();
    assert!(line.contains("未开设备腿"), "TUI 默认形态：{line}");
    assert_eq!(app.engine_status().device_sample_rate, None);
    assert_eq!(app.device_status().project_sample_rate, 48_000);

    // 没开就不能假装关（响亮拒绝）。
    let error = app.close_device().expect_err("没开设备腿");
    assert!(error.contains("本来就没开"), "{error}");
    app.execute_command("device off");
    assert!(last_log(&app).contains("本来就没开"), "{}", last_log(&app));

    // 状态命令读的是同一份实时读数。
    app.execute_command("device status");
    assert!(
        last_log(&app).contains("工程采样率 48000 Hz"),
        "{}",
        last_log(&app)
    );

    // 非法子命令给用法。
    app.execute_command("device maybe");
    assert!(
        last_log(&app).contains("用法：:device"),
        "{}",
        last_log(&app)
    );

    // 真开设备腿：有声卡 ⇒ 四个读数齐全；无声卡 ⇒ 必须响亮报错（⛔ 不假装开了）。
    app.execute_command("device on");
    let status = app.engine_status();
    if status.device_active {
        assert!(status.device_name.is_some(), "开着的设备腿必须有名字");
        assert!(
            status.device_sample_rate.is_some(),
            "开着的设备腿必须有设备率"
        );
        let line = app.device_status_line();
        assert!(line.contains("设备采样率"), "{line}");
        app.close_device().expect("关设备腿");
        assert!(
            !app.engine_status().device_active,
            "关掉之后必须回到墙钟泵送"
        );
    } else {
        assert!(
            last_log(&app).contains("打开音频设备失败"),
            "无设备时开设备腿必须响亮失败：{}",
            last_log(&app)
        );
        assert_eq!(
            status.device_sample_rate, None,
            "失败时不许留下编造的设备率"
        );
    }
}
