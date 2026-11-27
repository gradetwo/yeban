//! T-FIX-802：TUI 的 `:import audio` / `:import midi` / `:export midi`
//! 必须走既有导入/导出后端，失败响亮回执。

use std::path::{Path, PathBuf};

use yeban_model::ids::EntityId;
use yeban_model::project::ClipContent;
use yeban_tui::app::App;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-tui-t802-{tag}-{}-{}",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

fn write_wav(path: &Path) {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&36u32.to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&44_100u32.to_le_bytes());
    bytes.extend_from_slice(&88_200u32.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&2u32.to_le_bytes());
    bytes.extend_from_slice(&0i16.to_le_bytes());
    std::fs::write(path, bytes).expect("写 WAV");
}

fn write_midi(path: &Path) {
    let mut bytes: Vec<u8> = Vec::new();
    bytes.extend_from_slice(b"MThd");
    bytes.extend_from_slice(&6u32.to_be_bytes());
    bytes.extend_from_slice(&0u16.to_be_bytes()); // format 0
    bytes.extend_from_slice(&1u16.to_be_bytes()); // one track
    bytes.extend_from_slice(&960u16.to_be_bytes()); // PPQ
    bytes.extend_from_slice(b"MTrk");
    bytes.extend_from_slice(&13u32.to_be_bytes());
    bytes.extend_from_slice(&[0x00, 0x90, 0x3c, 0x64]);
    bytes.extend_from_slice(&[0x83, 0x60, 0x80, 0x3c, 0x64]);
    bytes.extend_from_slice(&[0x00, 0xff, 0x2f, 0x00]);
    std::fs::write(path, bytes).expect("写 MIDI");
}

fn last(app: &App) -> String {
    app.log.last().cloned().unwrap_or_default()
}

#[test]
fn import_audio_command_adds_a_pool_entry() {
    let dir = scratch_dir("audio");
    let wav = dir.join("take.wav");
    write_wav(&wav);
    let mut app = App::with_demo().expect("demo app");
    let before = app.project.clip_pool.len();
    app.execute_command(&format!("import audio {}", wav.display()));
    let line = last(&app);
    assert!(line.contains("已导入音频"), "回执必须点名成功：{line}");
    assert!(
        line.contains("尚未摆放"),
        "必须如实说只进素材池、没有摆放：{line}"
    );
    assert_eq!(app.project.clip_pool.len(), before + 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_midi_command_writes_notes_into_the_current_clip() {
    let dir = scratch_dir("midi");
    let midi = dir.join("one.mid");
    write_midi(&midi);
    let mut app = App::with_demo().expect("demo app");
    let clip_id = app
        .project
        .clip_pool
        .values()
        .find(|entry| matches!(entry.content, ClipContent::Midi { .. }))
        .map(|entry| entry.id)
        .expect("演示工程应有 MIDI 片段");
    let before = match &app.project.clip_pool[&clip_id].content {
        ClipContent::Midi { notes } => notes.len(),
        ClipContent::Audio { .. } => 0,
    };
    app.selected = app
        .track_ids
        .iter()
        .position(|track_id| {
            app.project.tracks[track_id]
                .clips
                .values()
                .any(|p| p.clip_id == clip_id)
        })
        .expect("应有引用该条目的摆放");
    app.cursor_bar = 0;
    app.execute_command(&format!("import midi {}", midi.display()));
    let line = last(&app);
    assert!(line.contains("已导入 MIDI"), "回执必须点名成功：{line}");
    let after = match &app.project.clip_pool[&clip_id].content {
        ClipContent::Midi { notes } => notes.len(),
        ClipContent::Audio { .. } => 0,
    };
    assert_eq!(after, before + 1, "SMF 里的一颗音符必须进目标片段");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_midi_command_writes_a_real_smf() {
    let dir = scratch_dir("export");
    let target = dir.join("out.mid");
    let mut app = App::with_demo().expect("demo app");
    app.execute_command(&format!("export midi {}", target.display()));
    let line = last(&app);
    assert!(line.contains("已导出 MIDI"), "回执必须点名成功：{line}");
    let bytes = std::fs::read(&target).expect("目标文件必须存在");
    assert_eq!(&bytes[..4], b"MThd", "必须是 SMF 头");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn failures_are_loud_and_never_write_a_file() {
    let dir = scratch_dir("fail");
    let mut app = App::with_demo().expect("demo app");
    let missing = dir.join("missing.wav");
    app.execute_command(&format!("import audio {}", missing.display()));
    assert!(
        last(&app).starts_with("导入音频失败"),
        "缺失音频必须响亮失败：{}",
        last(&app)
    );

    let missing_dir_target = dir.join("no-such-dir").join("out.mid");
    app.execute_command(&format!("export midi {}", missing_dir_target.display()));
    assert!(
        last(&app).starts_with("导出 MIDI 失败"),
        "不可写目标必须响亮失败：{}",
        last(&app)
    );
    assert!(!missing_dir_target.exists(), "失败不得留下目标文件");

    app.execute_command("import flac somewhere.flac");
    assert!(last(&app).contains("未知导入格式"), "未知格式不能静默");
    app.execute_command("export midi");
    assert!(last(&app).contains("用法：:export"), "缺路径必须报用法");
    let _ = std::fs::remove_dir_all(&dir);
}
