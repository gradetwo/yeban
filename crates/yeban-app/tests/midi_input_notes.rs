//! `--midi-in` 的**两半**在**无声卡**条件下的端到端判据（谱面 / 声音）。
//!
//! ## 为什么这个文件不需要 MIDI 硬件
//!
//! "OS 真的把字节送进来"那层在本机与 CI **都不可覆盖**（没有 MIDI 端口）。可判的那层是
//! **消息 ⇒ 音乐**：本文件喂一批**注入**的消息（与 `yeban-midi-io` 的模拟回调体
//! `MidiEventSender::handle_message` 同一手法），走 [`cli::apply_captured_midi`] ——
//! **生产路径调用的就是它**（`run_batch` 里 `capture_messages()` 之后立刻是它）。
//!
//! ⛔ 本文件**不**打开任何端口、⛔ 不依赖 `midi-input` 特性 ⇒ **默认档**（CI 的默认腿）就跑到。
//!
//! ## 两半各自的证据
//!
//! | 半 | 断言 |
//! | :-- | :--- |
//! | ① 谱面有反应 | 从**工程里读回**音符：条数 0 → 2、音高 `{60, 64}`、力度、起止 tick 逐项对得上；`midi-notes:` 行带 `origin=midi-input` |
//! | ② 声音有反应 | ⭐ **事件进环**后离线渲染：原始样本**非静音**（非零样本 > 0）＋ **合成器器件侧计数增长**（`poly-notes-triggered > 0`，即"事件真的到了合成器"）＋ 与"**不推任何事件**"档的 WAV **逐字节不同** |
//!
//! ⚠ 反向对照（"不推事件"档）用**同一个工程**、**同一次装配**渲染 ⇒ 两档唯一的差别是
//! 那批事件，因此"字节不同"只能归因于它们（⛔ 不是"反正都有声音"）。
//!
//! ⚠ ② 走的是 **`EngineEvent` 环**（`EngineHost::send_engine_events` ⇒ `process_quantum`，
//! 与设备回调同一条渲染路径）：引擎自 `eca718ab` 起**真的消费** `NoteOn`/`NoteOff`
//! （`SynthEngine::note_on/note_off`）。这条路径的登记判据在
//! `yeban_app::midi_notes::tests::the_engine_event_note_path_is_wired_to_the_synth`。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use yeban_app::cli::{self, Options};
use yeban_app::engine_host::EngineHost;
use yeban_app::export_wav::{WavExportOptions, write_interleaved_wav};
use yeban_app::midi_in::CapturedMessages;
use yeban_app::midi_notes;
use yeban_model::container::ProjectArchive;
use yeban_model::project::{ClipContent, YebanProjectV1};

/// 渲染窗口（2 秒）与声道数 —— 两条判据共用，⛔ 不各写一份。
const FRAMES: u64 = 2 * 48_000;
const CHANNELS: u16 = 2;
const SAMPLE_RATE: u32 = 48_000;

/// 一次性的临时目录（**不用外部 crate**）。
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yeban-midi-notes-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 注入的**原始消息**（与真回调推给环里的东西同形态：`(timestamp_us, status, data1, data2)`）。
///
/// * `0.0 s` C4 按下（力度 100）→ `0.5 s` 松开；
/// * `1.0 s` E4 按下（力度 90）→ `1.5 s` 松开；
/// * `1.2 s` 一条 CC7（**非音符** ⇒ 被计数忽略，不参与配对）。
fn injected_messages() -> Vec<(u64, u8, u8, u8)> {
    vec![
        (0, 0x90, 60, 100),
        (500_000, 0x80, 60, 0),
        (1_000_000, 0x90, 64, 90),
        (1_200_000, 0xB0, 7, 100),
        (1_500_000, 0x80, 64, 0),
    ]
}

/// 注入的一批消息（`origin_us = 0` ⇒ tick 直接由时间戳算出）。
fn injected() -> CapturedMessages {
    CapturedMessages {
        origin_us: 0,
        seconds: 2.0,
        messages: injected_messages(),
        unsupported: 0,
        // ⭐ 注入夹具**不经过环**（没有端口、没有回调）⇒ 丢弃数就是 0；
        // ⭐ "环满 ⇒ 报告里的值 = 接收侧读数 且 > 0" 由 `midi_in.rs` 的成对判据覆盖
        // （最小容量环 + 注入足量事件, 与 io 侧同一手法）。
        dropped_events: 0,
    }
}

/// ⭐ 一个**没有音符**的工程：音轨 / 乐器 / 摆放都在，只是片段里**一个音符都没有**。
///
/// 为什么要它：反向对照要的是"没有这批音符 ⇒ 真的没有声音"。若工程自带音符，
/// 引擎会按快照里的排程自己出声，"这批音符造成了声音"这句话就**不再可判**。
///
/// ⚠ 只清**音符**、⛔ 不清摆放：`Op::AddNote` 需要一个目标摆放（`import_midi::destination`
/// 的判定），清掉摆放会让写音符因为"没有目标"而失败（本机实测踩到）。
fn without_notes() -> YebanProjectV1 {
    let mut project = yeban_app::bridge::demo_project();
    for entry in project.clip_pool.values_mut() {
        if let ClipContent::Midi { notes } = &mut entry.content {
            notes.clear();
        }
    }
    project
}

/// 把工程写成 `.yeban` 容器并返回路径（`--open` 用）。
fn write_container(dir: &Path, project: YebanProjectV1) -> PathBuf {
    let path = dir.join("project.yeban");
    yeban_app::save::save_archive_file(
        &ProjectArchive {
            project,
            history_dag: Vec::new(),
            assets: Vec::new(),
        },
        &path,
    )
    .expect("写出容器");
    path
}

/// 从工程里读回**目标音轨**上的音符 `(pitch, velocity, start_tick, duration_ticks)`。
fn notes_of(project: &YebanProjectV1, track_id: yeban_model::EntityId) -> Vec<(u8, u8, u64, u64)> {
    let mut out: Vec<(u8, u8, u64, u64)> = Vec::new();
    let Some(track) = project.tracks.get(&track_id) else {
        return out;
    };
    for placement in track.clips.values() {
        let Some(entry) = project.clip_pool.get(&placement.clip_id) else {
            continue;
        };
        let Some(map) = entry.content.notes() else {
            continue;
        };
        for note in map.values() {
            out.push((
                note.pitch,
                note.velocity,
                note.start_tick,
                note.duration_ticks,
            ));
        }
    }
    out.sort_unstable();
    out
}

/// 打开一个容器（`--open` 的公开入口）。
fn open_container(path: &Path) -> cli::Loaded {
    cli::load_project(&Options {
        open: Some(path.to_path_buf()),
        ..Options::default()
    })
    .expect("打开容器")
}

/// 判据 ①（**谱面有反应**）：⭐ 注入 5 条消息（其中 2 条音符）⇒ 工程里**真的**多出 2 条音符，
/// 音高 / 力度 / 起止 tick 逐项对得上，且提交的来源是 `midi-input`。
#[test]
fn writing_notes_puts_the_injected_notes_into_the_project() {
    let dir = scratch_dir("write");
    let container = write_container(&dir, without_notes());
    let mut loaded = open_container(&container);
    assert_eq!(loaded.archive.project.tracks.len(), 7, "6 条音轨 + 主总线");

    let options = Options {
        midi_in: Some("0".to_owned()),
        midi_in_write_notes: true,
        ..Options::default()
    };
    let mut lines: Vec<String> = Vec::new();
    cli::apply_captured_midi(&mut loaded, &options, &injected(), &mut lines)
        .expect("写音符必须成功");
    let text = lines.join("\n");

    // ⭐ 报告必须说清配对结果与来源。
    assert!(
        text.contains("midi-notes-pairing: notes=2"),
        "配对摘要必须说清条数:\n{text}"
    );
    assert!(text.contains("ignored=1"), "CC7 必须被计数忽略:\n{text}");
    assert!(
        text.contains("origin=midi-input"),
        "提交来源必须是 midi-input:\n{text}"
    );
    assert!(text.contains("midi-notes: notes=2"), "{text}");

    // ⭐ 从工程里**读回**：这就是"谱面有反应"。0.5 s 在 960 PPQ 下 = 480 tick。
    let track_id = midi_notes::default_track(&loaded.archive.project).expect("有音轨");
    let notes = notes_of(&loaded.archive.project, track_id);
    assert_eq!(notes.len(), 2, "工程里必须多出两条音符: {notes:?}");
    assert_eq!(
        notes,
        vec![(60, 100, 0, 480), (64, 90, 960, 480)],
        "音高 / 力度 / 起点 / 时长必须逐项对得上"
    );
    assert_ne!(
        loaded.archive.project,
        without_notes(),
        "工程必须真的被改了（不是只打印了一行）"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 ②（**声音有反应 ＋ 反向对照**）：⭐ **事件进环** ⇒ 渲染非静音 ＋
/// **合成器器件侧计数增长**（事件真的到了合成器）＋ 与"不推事件"档的 WAV **逐字节不同**；
/// 反向对照档必须**全零**（那条对照的牙）。
#[test]
fn the_ring_events_make_the_engine_sound_and_differ_from_the_no_event_render() {
    let dir = scratch_dir("render");
    let baseline_project = without_notes();

    // ---- 反向对照：同一个工程、同一次装配、**不推任何事件** ----
    let mut baseline_host = EngineHost::new();
    baseline_host
        .reload(&baseline_project, 0)
        .expect("空调度也要能建引擎");
    let block = EngineHost::quantum_frames();
    let channels = CHANNELS;
    let quanta = FRAMES.div_ceil(u64::try_from(block).expect("块长"));
    let mut baseline = vec![0.0_f32; usize::try_from(quanta).expect("量子数") * block * 2];
    let driven = baseline_host.render_into(&mut baseline, channels, quanta);
    assert_eq!(driven, quanta, "必须真的推进了全部量子");
    let nonzero = |samples: &[f32]| samples.iter().filter(|s| **s != 0.0).count();

    // ---- 事件档：注入消息 ⇒ 成对音符 ⇒ **推进环** ⇒ 离线渲染 ----
    let pairing = midi_notes::pair_notes(&injected().messages, 0, midi_notes::UNCLOSED_TAIL_TICKS);
    assert_eq!(pairing.notes.len(), 2, "{}", pairing.summary());
    let track_id = midi_notes::default_track(&baseline_project).expect("有音轨");
    let mut host = EngineHost::new();
    host.reload(&baseline_project, 0).expect("引擎必须能建起来");
    let wav = dir.join("ring.wav");
    let report = midi_notes::render_notes_to_wav(
        &mut host,
        &pairing.notes,
        track_id,
        &wav,
        FRAMES,
        channels,
        SAMPLE_RATE,
        midi_notes::bpm_millis(baseline_project.bpm),
        1,
    )
    .expect("环路径渲染必须成功");
    let (samples, _, _, _) = yeban_app::render_core::render_project_f32(
        &baseline_project,
        WavExportOptions::default(),
        None,
    )
    .expect("对照档（同一次装配的独立读数）");

    if let Some(stats) = host.engine_stats() {
        println!(
            "[midi-notes] 引擎读数: events_applied={} notes_triggered={} poly={} drum_hits={} \
             drum_triggers={} note_steals={} voice_steals={} orphan_note_offs={}",
            stats.events_applied,
            stats.notes_triggered,
            stats.poly_notes_triggered,
            stats.drum_hits,
            stats.drum_triggers,
            stats.note_steals,
            stats.voice_steals,
            stats.orphan_note_offs,
        );
    }
    println!(
        "[midi-notes] 环路径: live-notes={} events={}/{} poly-notes-triggered={} \
         nonzero-samples={} peak={:.6} | 原始样本对照 nonzero={}",
        report.notes,
        report.events_sent,
        report.events_scheduled,
        report.poly_notes_triggered,
        report.nonzero_samples,
        report.peak,
        nonzero(&baseline),
    );
    let _ = samples;

    assert_eq!(report.notes, 2, "报告必须说清这次声音里有几条**现场**音符");
    assert_eq!(report.events_scheduled, 4, "两条音符 ⇒ 4 条事件");
    assert_eq!(
        report.events_sent, report.events_scheduled,
        "全部事件都必须真的进环（容量远大于 4）"
    );
    assert!(
        report.nonzero_samples > 0 && report.peak > 0.0,
        "⭐ 弹一下必须**有声音**（非零样本 {} / 峰值 {}）",
        report.nonzero_samples,
        report.peak
    );
    assert_eq!(
        report.poly_notes_triggered, 2,
        "⭐ 见证：**两条**音符必须各触发一次合成器器件（`poly_notes_triggered` 增量 = {}）\
         —— ⚠ 若这里是 1, 说明有一条音符没到器件（本机踩过: tick 0 的环音符在**没有预热量子**时\
         会因为引擎'先出队事件、再武装快照'的步骤顺序被丢掉）",
        report.poly_notes_triggered
    );
    assert_eq!(
        nonzero(&baseline),
        0,
        "⭐ 反向对照必须**全零**（不推事件就没有声音）—— 否则'事件造成了声音'不可判"
    );
    assert!(
        report.summary(Some(&wav)).contains("live-notes=2"),
        "报告行必须说清现场音符数: {}",
        report.summary(Some(&wav))
    );

    // ---- 产物层：两档 WAV 逐字节不同 ----
    let baseline_wav = dir.join("baseline.wav");
    let (baseline_bytes, _) = write_interleaved_wav(
        &baseline_wav,
        &baseline,
        channels,
        SAMPLE_RATE,
        1,
        yeban_render::dither::BitDepth::Int24,
    )
    .expect("写对照档");
    let ring_file = std::fs::read(&wav).expect("读环档");
    let baseline_file = std::fs::read(&baseline_wav).expect("读对照档");
    println!(
        "[midi-notes] WAV: 环档 {} 字节 / 对照档 {baseline_bytes} 字节",
        report.bytes.unwrap_or(0)
    );
    assert_ne!(
        ring_file, baseline_file,
        "⭐ 环档与不推事件档的 WAV 必须**逐字节不同**"
    );
    assert!(report.bytes.unwrap_or(0) > 44, "必须超过一个 WAV 头");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 ③（**接线**）：`--midi-in-write-notes` ＋ `--midi-in-render` 一次给 ⇒
/// **谱面**（工程里有音符, `midi-notes:` 行）与**声音**（磁盘上有 WAV, `midi-render:` 行）
/// **两件事各自有读数**（⛔ 不是一句话糊在一起）。
#[test]
fn both_landing_switches_together_write_notes_and_a_wav() {
    let dir = scratch_dir("both");
    let container = write_container(&dir, without_notes());
    let mut loaded = open_container(&container);
    let wav = dir.join("midi-in.wav");
    let options = Options {
        midi_in: Some("0".to_owned()),
        midi_in_write_notes: true,
        midi_in_render: Some(wav.clone()),
        ..Options::default()
    };

    let mut lines: Vec<String> = Vec::new();
    cli::apply_captured_midi(&mut loaded, &options, &injected(), &mut lines).expect("两条路径");
    let text = lines.join("\n");
    assert!(
        text.contains("midi-notes: notes=2"),
        "谱面侧必须有读数:\n{text}"
    );
    let render_line = lines
        .iter()
        .find(|line| line.starts_with("midi-render:"))
        .unwrap_or_else(|| panic!("必须有 midi-render: 行:\n{text}"));
    assert!(render_line.contains("live-notes=2"), "{render_line}");
    assert!(
        render_line.contains("source=engine(EngineEvent 环"),
        "渲染行必须明说声音来自**环**: {render_line}"
    );
    assert!(
        !render_line.contains("poly-notes-triggered=0"),
        "⭐ 见证: 事件必须到了合成器: {render_line}"
    );

    let bytes = std::fs::metadata(&wav).expect("WAV 必须真的落盘").len();
    assert!(bytes > 44, "WAV 必须超过一个头（{bytes} 字节）");
    assert!(
        render_line.contains(&format!("bytes={bytes}")),
        "报告里的字节数必须是实际落盘字节数: {render_line}"
    );

    // ⭐ 顺序（谱面 → 声音）：工程里也要真的有音符。
    let track_id = midi_notes::default_track(&loaded.archive.project).expect("有音轨");
    assert_eq!(notes_of(&loaded.archive.project, track_id).len(), 2);

    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 ④（**用法与组合**）：⭐ 三条落地开关**必须**与 `--midi-in` 一起给；
/// ⭐ `--midi-in-render` **必须**与 `--midi-in-write-notes` 一起给；
/// ⭐ `--midi-in-track` 选不中音轨时精确报错（列出可用音轨）。
#[test]
fn the_landing_switches_require_midi_in_and_the_track_selector_is_precise() {
    // (a) 单独给 ⇒ 用法错误（退出码 2 的那一档），且点名该与谁一起给。
    for args in [
        vec!["--midi-in-write-notes"],
        vec!["--midi-in-render", "/tmp/x.wav"],
        vec!["--midi-in-track", "0"],
        vec!["--midi-in-seconds", "1"],
    ] {
        let owned: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        let options = cli::parse(&owned).expect("解析本身是通的（组合性在 run_batch 里判）");
        let error = cli::run_batch(&options).expect_err("必须是用法错误");
        assert_eq!(
            error.exit_code(),
            cli::EXIT_USAGE,
            "{args:?} 必须是用法错误: {error}"
        );
        assert!(
            error.to_string().contains("--midi-in"),
            "{args:?} 必须点名该与谁一起给: {error}"
        );
    }

    // (b) ⭐ `--midi-in-render` **不再**要求与 `--midi-in-write-notes` 一起给：
    // 声音来自**环**、谱面来自**写工程**，两者可以一有一无（约束已被批准取消）。
    // ⇒ 这里只断言"解析能过、组合性检查不再拦它"，⛔ 不假装它必须成对。
    let options = cli::parse(&[
        "--midi-in".to_owned(),
        "0".to_owned(),
        "--midi-in-render".to_owned(),
        "/tmp/x.wav".to_owned(),
    ])
    .expect("解析必须通过");
    assert!(options.midi_in_render.is_some() && !options.midi_in_write_notes);

    // (c) 音轨选择器：选不中 ⇒ 精确错误（列出可用音轨），⛔ 不是"随便写一条"。
    let dir = scratch_dir("selector");
    let container = write_container(&dir, without_notes());
    let mut loaded = open_container(&container);
    let options = Options {
        midi_in: Some("0".to_owned()),
        midi_in_write_notes: true,
        midi_in_track: Some("\u{1F6AB}不存在\u{1F6AB}".to_owned()),
        ..Options::default()
    };
    let mut lines: Vec<String> = Vec::new();
    let error = cli::apply_captured_midi(&mut loaded, &options, &injected(), &mut lines)
        .expect_err("选不中音轨必须失败");
    let text = error.to_string();
    assert!(text.contains("命中 0 条音轨"), "{text}");
    assert!(text.contains("可用音轨"), "必须列出可用音轨: {text}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 判据 ⑤（**只读路径不变**）：⭐ 只给 `--midi-in`（不给任何落地开关）时，
/// [`cli::apply_captured_midi`] **什么都不做** —— ⛔ 不建会话、⛔ 不改工程、⛔ 不写盘。
#[test]
fn without_a_landing_switch_the_capture_stays_read_only() {
    let dir = scratch_dir("readonly");
    let container = write_container(&dir, without_notes());
    let mut loaded = open_container(&container);
    let before = loaded.archive.project.clone();
    let options = Options {
        midi_in: Some("0".to_owned()),
        ..Options::default()
    };
    let mut lines: Vec<String> = Vec::new();
    cli::apply_captured_midi(&mut loaded, &options, &injected(), &mut lines).expect("只读必须成功");
    assert!(lines.is_empty(), "只读路径不该追加任何行: {lines:?}");
    assert_eq!(loaded.archive.project, before, "只读路径不许改工程");
    let files: BTreeMap<String, u64> = std::fs::read_dir(&dir)
        .expect("列目录")
        .filter_map(Result::ok)
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                entry.metadata().map(|meta| meta.len()).unwrap_or(0),
            )
        })
        .collect();
    assert_eq!(files.len(), 1, "只读路径不许写新文件: {files:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// ⭐ 判据 ⑥–⑦（**连续性**：`midi_live` 泵）—— ⚠ 只在 `midi-input` 特性下存在
// ---------------------------------------------------------------------------
// ⚠ **为什么有 `cfg` 门**：`yeban-midi-io` 是 app 的**可选依赖**（`midi-input` 才拉进来）
// ⇒ ⛔ 默认档里连这个 crate 都不在依赖图上 ⇒ 这些判据在默认档里**不存在**（⛔ 不是"被跳过"）。
// ⇒ 必须显式 `cargo test -p yeban-app --features midi-input` 才跑到（见回报里的命令）。
//
// ⭐ 判据的**形状**：⭐ 真接收侧（`yeban_midi_io::event_channel` 建的环）＋ ⭐ **模拟回调体**
// `MidiEventSender::handle_message`（＝真 `midir` 回调的**内核**）＋ ⭐ 真 `EngineHost`
// ⇒ ⛔ 不需要 MIDI 端口、⛔ 不需要硬件。
#[cfg(feature = "midi-input")]
mod live {
    use yeban_app::engine_host::EngineHost;
    use yeban_app::midi_live;
    use yeban_app::midi_notes;
    use yeban_model::EntityId;

    use super::{CHANNELS, FRAMES, SAMPLE_RATE, without_notes};

    /// ⭐ **与 `injected_messages()` 同一批 ＋ 三处刻意的追加**的**字节形态**。
    ///
    /// * `0.0 s` C4 按下（力度 100）→ `0.5 s` 松开（`0x80`）；
    /// * `0.7 s` **G4 与 B4** 同时按下（⭐ **同一个块**里的和弦 ⇒ 一次发布搬 2 条）
    ///   → `0.9 s` G4 用 **`0x90` 力度 0** 松开（⭐ 经典 bug 点：这**必须**是 NoteOff）；
    /// * `1.0 s` E4 按下（力度 90）→ `1.5 s` 松开；
    /// * `1.2 s` 一条 CC7（**非音符** ⇒ 计数忽略）；
    /// * `1.5 s` B4 松开（与 E4 松开同块）。
    ///
    /// ⚠ 为什么与批次判据的夹具**不完全相同**：端到端判据要**自己**钉住"力度 0 ⇒ NoteOff"
    /// 与"一个块里的两条事件只发布一次"这两条 —— ⛔ 不能只把它们钉在纯函数判据上。
    fn live_messages() -> Vec<(u64, [u8; 3])> {
        vec![
            (0, [0x90, 60, 100]),
            (500_000, [0x80, 60, 0]),
            (700_000, [0x90, 67, 110]),
            (700_000, [0x90, 71, 100]),
            (900_000, [0x90, 67, 0]),
            (1_000_000, [0x90, 64, 90]),
            (1_200_000, [0xB0, 7, 100]),
            (1_500_000, [0x80, 64, 0]),
            (1_500_000, [0x80, 71, 0]),
        ]
    }

    /// 非零样本个数（交错样本计**个**，⛔ 不是帧、⛔ 不是字节）。
    fn nonzero(samples: &[f32]) -> usize {
        samples.iter().filter(|sample| **sample != 0.0).count()
    }

    /// 峰值绝对值。
    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0_f32, |max, s| max.max(s.abs()))
    }

    /// ⭐ **实时形状的驱动**：每渲染**一个量子**之前 —— 先把到点的消息喂进环（模拟回调体），
    /// 再泵**一次**（控制侧），然后渲染这**一个**量子。
    ///
    /// ⚠ 调用方必须**先预热一个量子**（见下面的判据）—— 那是
    /// [`midi_notes::render_engine_offline`] 记的**同一条**引擎行为（"先出队事件、后武装快照"
    /// ⇒ 冷引擎第一个块里的音符会被丢掉）。
    ///
    /// 返回（泵读数合计，渲染出的交错样本）。
    fn drive_live(
        host: &mut EngineHost,
        track: EntityId,
        quanta: u64,
    ) -> (midi_live::LivePumpReport, Vec<f32>) {
        let block = EngineHost::quantum_frames();
        let channels = usize::from(CHANNELS);
        let mut out = vec![0.0_f32; usize::try_from(quanta).expect("量子数") * block * channels];
        let (mut sender, mut receiver) = midi_live::event_channel(64);
        let messages = live_messages();
        let mut cursor = 0_usize;
        let mut total = midi_live::LivePumpReport::default();

        for index in 0..quanta {
            let block_end_frames = (index + 1).saturating_mul(u64::try_from(block).expect("块长"));
            let block_end_us = block_end_frames.saturating_mul(1_000_000) / u64::from(SAMPLE_RATE);
            while cursor < messages.len() && messages[cursor].0 <= block_end_us {
                sender.handle_message(messages[cursor].0, &messages[cursor].1);
                cursor += 1;
            }
            let step = midi_live::pump_once(&mut receiver, track, host);
            total.merge(&step);

            let start = usize::try_from(index).expect("下标") * block * channels;
            let end = start + block * channels;
            assert_eq!(
                host.render_into(&mut out[start..end], CHANNELS, 1),
                1,
                "每个量子都必须真的推进"
            );
        }
        assert_eq!(cursor, messages.len(), "全部消息都必须被喂进环");
        (total, out)
    }

    /// 判据 ⑥（**连续性：真的到了合成器并发出声音，⛔ 不是等采集窗口结束**）：
    ///
    /// * ⭐ **控制臂先滚**：同一个工程、同一次装配、**一个事件都不喂** ⇒ 渲染**必须全零**
    ///   （否则"泵进去的事件造成了声音"不可判 ⇒ 判据会 vacuous）；
    /// * ⭐ 事件臂：**实时形状**（每量子：喂到点的消息 ⇒ 泵一次 ⇒ 渲染一个量子）⇒ 真引擎：
    ///   `poly_notes_triggered` 增长 **4**（四条音符各触发一次器件）＋ 输出**非静音**，
    ///   且音频侧 `event_bulk_pops` 的增量**恰好等于量子数**（pop-only）。
    ///
    /// ⚠ 判据**走的是 `EngineHost::reload` ＋ `send_engine_events`（经 `midi_live::pump_once`）
    /// ＋ `render_into` ＋ `engine_stats`** —— 与 [`midi_notes::render_engine_offline`] 用的是
    /// **同一套**底层；⛔ 本判据**不**走那个**批次**入口（它吃一张预先算好的调度表，
    /// 正是本判据要区分的东西）。
    #[test]
    fn the_pumped_events_reach_the_synth_and_the_no_event_arm_is_silent() {
        let project = without_notes();
        let track = midi_notes::default_track(&project).expect("有音轨");
        let block = EngineHost::quantum_frames();
        let channels = usize::from(CHANNELS);
        let quanta = FRAMES.div_ceil(u64::try_from(block).expect("块长"));

        // ---- ① 先滚的控制臂：⛔ 一个事件都不喂 ----
        let mut control = EngineHost::new();
        control.reload(&project, 0).expect("对照档也要能建引擎");
        let mut control_warmup = vec![0.0_f32; block * channels];
        control.render_into(&mut control_warmup, CHANNELS, 1);
        let control_frames = usize::try_from(quanta).expect("量子数") * block * channels;
        let mut control_out = vec![0.0_f32; control_frames];
        assert_eq!(
            control.render_into(&mut control_out, CHANNELS, quanta),
            quanta,
            "对照档必须真的推进了全部量子"
        );
        assert_eq!(
            nonzero(&control_out),
            0,
            "⭐ 控制臂必须**全零** —— 不喂事件就没有声音，否则'事件造成了声音'不可判"
        );

        // ---- ② 事件臂：同一批消息（＋ 和弦 / 力度 0 松开）⇒ 每量子泵一次 ----
        let mut host = EngineHost::new();
        host.reload(&project, 0).expect("引擎必须能建起来");
        let mut warmup = vec![0.0_f32; block * channels];
        host.render_into(&mut warmup, CHANNELS, 1); // ⭐ 预热量子（与批次路同一条引擎行为）
        let stats = host.engine_stats().expect("有引擎");
        let (poly_before, bulk_before) = (stats.poly_notes_triggered, stats.event_bulk_pops);

        let (report, out) = drive_live(&mut host, track, quanta);
        let stats = host.engine_stats().expect("有引擎");

        println!(
            "[midi-live] {} | poly-notes-triggered {}→{} | event-bulk-pops {}→{} | \
             nonzero={} peak={:.6} | 控制臂 nonzero={} ({control_frames} 样本)",
            report.summary(),
            poly_before,
            stats.poly_notes_triggered,
            bulk_before,
            stats.event_bulk_pops,
            nonzero(&out),
            peak(&out),
            nonzero(&control_out),
        );

        // ---- (a) 映射：⭐ 力度 0 与 `0x80` 都是 NoteOff；CC7 **计数** ----
        assert_eq!(report.drained, 9, "{}", report.summary());
        assert_eq!(
            report.note_ons,
            4,
            "C4 / G4 / B4 / E4 四条按下: {}",
            report.summary()
        );
        assert_eq!(
            report.note_offs,
            4,
            "⭐ 四条松开，其中一条是 `0x90` **力度 0**（经典 bug 点 ⇒ 它必须是 NoteOff，\
             ⛔ 不是第四条 NoteOn）: {}",
            report.summary()
        );
        assert_eq!(
            report.ignored,
            1,
            "⭐ CC7 必须被**计数**，⛔ 不静默丢: {}",
            report.summary()
        );
        // ---- (d) 批量：8 条事件分布在 6 个块（其中一块是**和弦**）⇒ 6 次批量发布 ----
        assert_eq!(
            report.publishes,
            6,
            "⭐ 每个有音符的块**恰好一次**批量发布（⛔ 不是每条事件一次，⛔ 空块不发布）: {}",
            report.summary()
        );
        assert!(
            report.publishes < report.mapped(),
            "⭐ 和弦那一块必须**一次发布搬 2 条**（publishes={} < mapped={}）",
            report.publishes,
            report.mapped()
        );
        assert_eq!(report.published, 8, "环装得下 8 条: {}", report.summary());
        assert_eq!(report.refused, 0, "环装得下: {}", report.summary());
        // ---- (c) 真的到了合成器 ＋ 真的发出声音 ----
        assert_eq!(
            stats.poly_notes_triggered.saturating_sub(poly_before),
            4,
            "⭐ 见证：**四条**音符必须各触发一次合成器器件（⚠ 若为 5 ⇒ G4 的力度 0 松开被当成了\
             又一次 NoteOn；若为 3 ⇒ 有一条没到器件）"
        );
        assert!(
            nonzero(&out) > 0 && peak(&out) > 0.0,
            "⭐ 弹一下必须**有声音**（非零样本 {} / 峰值 {}）",
            nonzero(&out),
            peak(&out)
        );
        // ---- (d) 音频侧 pop-only：每个量子**恰好一次**批量出队（⛔ 与事件条数无关）----
        assert_eq!(
            stats.event_bulk_pops.saturating_sub(bulk_before),
            quanta,
            "⭐ 音频侧结构性读数：{quanta} 个量子 ⇒ 恰好 {quanta} 次批量出队（pop-only）"
        );
    }
}
