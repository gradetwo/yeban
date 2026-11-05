//! P1 按需装载（渲染级）：渲染步只解码命中区域。
//!
//! 夹具说明：仓库语料只有 SFZ 文本、无样本文件，故本文件自带样本夹具
//! （临时目录里手写 14 区域 SFZ + 13 个真实 WAV；12 个单音高区域各一文件，
//! 音高 72 的两个力度层共用一个文件）。
//!
//! 本文件只走 `tools/call` 管线（旧代码也能编译），不引用 P1 新增 API。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::samples::filled_project;
use yeban_model::{ClipContent, EntityId, SamplerBinding, YebanProjectV1};

/// 一个临时目录，`Drop` 时删除自己。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-demand-{}-{tag}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// 手写一份常量 16-bit 单声道 48 kHz WAV（⛔ 不用被测写入器自证）。
fn write_constant_wav(path: &Path, value: f32, frames: usize) {
    let scaled = (value * 32767.0).round() as i16;
    let mut data = Vec::with_capacity(frames * 2);
    for _ in 0..frames {
        data.extend_from_slice(&scaled.to_le_bytes());
    }
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&48000_u32.to_le_bytes());
    out.extend_from_slice(&96000_u32.to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    fs::write(path, &out).expect("写样本夹具");
}

/// 夹具乐器：音高 60..=71 各一文件，音高 72 的两个力度层共用一个文件。
/// 共 14 区域、13 文件，每文件 4800 帧。
fn write_fixture_sfz(dir: &Path) -> PathBuf {
    fs::create_dir_all(dir).expect("建 SFZ 目录");
    let mut text = String::new();
    for pitch in 60..=71 {
        let name = format!("tone{pitch}.wav");
        write_constant_wav(&dir.join(&name), 0.1 + (pitch - 60) as f32 * 0.05, 4800);
        text.push_str(&format!("<region> sample={name} key={pitch}\n"));
    }
    write_constant_wav(&dir.join("shared.wav"), 0.9, 4800);
    text.push_str("<region> sample=shared.wav key=72 lovel=0 hivel=63\n");
    text.push_str("<region> sample=shared.wav key=72 lovel=64 hivel=127\n");
    let path = dir.join("demand.sfz");
    fs::write(&path, &text).expect("写 SFZ 夹具");
    path
}

/// `filled_project()` 的 Lead 轨换成给定音符（音高/力度），并绑定采样器。
fn project_with_notes_and_sampler(notes: &[(u8, u8)], sfz_path: &Path) -> YebanProjectV1 {
    let mut project = filled_project();
    let lead = project
        .tracks
        .iter()
        .find(|(_, track)| track.name == "Lead")
        .map(|(id, _)| *id)
        .expect("filled_project 必须有 Lead 轨");
    let mut clip_ids = Vec::new();
    let track = project.tracks.get_mut(&lead).expect("Lead");
    for placement in track.clips.values_mut() {
        placement.start_tick = 0;
        placement.duration_ticks = 960 * notes.len().max(1) as u64;
        // 夹具摆放自带循环（0..3840）；关掉它，让替换后的音符按摆放跨度直排，
        // ⛔ 不让循环窗口截掉后半段音符。
        placement.loop_config = yeban_model::LoopConfig::default();
        clip_ids.push(placement.clip_id);
    }
    for clip_id in clip_ids {
        if let Some(entry) = project.clip_pool.get_mut(&clip_id)
            && let ClipContent::Midi { notes: midi_notes } = &mut entry.content
        {
            midi_notes.clear();
            for (i, &(pitch, velocity)) in notes.iter().enumerate() {
                let id = EntityId::new();
                let mut note = yeban_model::MidiNote::new(id, (i as u64) * 960, pitch, 960);
                note.velocity = velocity;
                midi_notes.insert(id, note);
            }
        }
    }
    project.tracks.get_mut(&lead).expect("Lead").sampler = Some(SamplerBinding {
        path: sfz_path.display().to_string(),
    });
    project.validate().expect("夹具工程必须合法");
    project
}

/// 走真实 `tools/call` 管线渲染一次，返回工具结果。
fn render(project: YebanProjectV1, project_path: &Path, out: &Path, extra: Value) -> Value {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(project_path.to_path_buf(), project, false)
        .expect("注入夹具工程");
    let mut arguments = json!({
        "format": "wav",
        "sampleRate": 48000,
        "path": out.display().to_string(),
    });
    if let Value::Object(map) = &mut arguments
        && let Value::Object(more) = extra
    {
        map.extend(more);
    }
    let line = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": "yeban_render_master", "arguments": arguments},
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(&auth), &line);
    outcome
        .response
        .expect("必须有响应")
        .result
        .expect("工具结果")
}

/// 找到绑定了给定采样器路径的那条源的 `sampler` 读数；找不到时把整条响应打出来。
fn sampler_report<'a>(result: &'a Value, raw: &str) -> &'a Value {
    let sources = result["data"]["sources"]
        .as_array()
        .unwrap_or_else(|| panic!("渲染必须成功并报 sources: {result}"));
    sources
        .iter()
        .find(|source| source["sampler"]["path"].as_str() == Some(raw))
        .map(|source| &source["sampler"])
        .unwrap_or_else(|| panic!("找不到绑定 {raw} 的采样器读数: {result}"))
}

fn full_notes() -> Vec<(u8, u8)> {
    let mut notes: Vec<(u8, u8)> = (60..=71).map(|pitch| (pitch, 100)).collect();
    notes.push((72, 32));
    notes.push((72, 96));
    notes
}

/// 稀疏谱子（1 音符）只读命中区域：13 区域跳过、1 文件、4800 帧、无失败。
#[test]
fn sparse_score_reads_only_hit_region() {
    let scratch = Scratch::new("sparse");
    let sfz = write_fixture_sfz(&scratch.join("inst"));
    let project_path = scratch.join("project").join("song.yeban");
    fs::create_dir_all(project_path.parent().expect("有父目录")).expect("建工程目录");
    let out = scratch.join("sparse.wav");
    let project = project_with_notes_and_sampler(&[(60, 100)], &sfz);
    let raw = sfz.display().to_string();
    let result = render(project, &project_path, &out, Value::Null);
    let sampler = sampler_report(&result, &raw);
    assert_eq!(
        sampler["skippedRegions"], 13,
        "14 区域命中 1 条 ⇒ 跳过 13: {result}"
    );
    assert_eq!(
        sampler["sampleSources"].as_array().expect("sources").len(),
        1,
        "只读命中文件: {result}"
    );
    assert_eq!(
        sampler["sampleFramesLoaded"], 4800,
        "只计命中文件帧数: {result}"
    );
    assert_eq!(sampler["sampleFailures"], json!([]), "无读失败: {result}");
    assert_eq!(
        sampler["sampleReadsTruncated"], false,
        "稀疏谱子不触预算: {result}"
    );
    assert!(
        out.is_file() && fs::metadata(&out).expect("产物").len() > 44,
        "渲染必须落盘非空产物"
    );
}

/// 全量谱子（覆盖全部键/力度层）读全：跳过 0、13 文件、13×4800 帧。
#[test]
fn full_score_reads_everything() {
    let scratch = Scratch::new("full");
    let sfz = write_fixture_sfz(&scratch.join("inst"));
    let project_path = scratch.join("project").join("song.yeban");
    fs::create_dir_all(project_path.parent().expect("有父目录")).expect("建工程目录");
    let out = scratch.join("full.wav");
    let project = project_with_notes_and_sampler(&full_notes(), &sfz);
    let raw = sfz.display().to_string();
    let result = render(project, &project_path, &out, Value::Null);
    let sampler = sampler_report(&result, &raw);
    assert_eq!(sampler["skippedRegions"], 0, "全量谱子无跳过: {result}");
    assert_eq!(
        sampler["sampleSources"].as_array().expect("sources").len(),
        13,
        "全量谱子读全 13 文件: {result}"
    );
    assert_eq!(
        sampler["sampleFramesLoaded"],
        13 * 4800,
        "全量谱子帧数全计: {result}"
    );
    assert_eq!(sampler["sampleFailures"], json!([]), "无读失败: {result}");
}

/// 稀疏谱子在小预算下也不截断：预算只对已用子集记账（整件乐器 13 文件 > 2）。
#[test]
fn sparse_score_under_tiny_budget_is_not_truncated() {
    let scratch = Scratch::new("budget");
    let sfz = write_fixture_sfz(&scratch.join("inst"));
    let project_path = scratch.join("project").join("song.yeban");
    fs::create_dir_all(project_path.parent().expect("有父目录")).expect("建工程目录");
    let out = scratch.join("budget.wav");
    let project = project_with_notes_and_sampler(&[(60, 100)], &sfz);
    let raw = sfz.display().to_string();
    let result = render(project, &project_path, &out, json!({"maxSampleFiles": 2}));
    let sampler = sampler_report(&result, &raw);
    assert_eq!(
        sampler["sampleReadsTruncated"], false,
        "已用 1 文件 < 上限 2 ⇒ 不截断（整件 13 文件不算）: {result}"
    );
    assert_eq!(
        sampler["skippedRegions"], 13,
        "跳过数不受预算影响: {result}"
    );
}
