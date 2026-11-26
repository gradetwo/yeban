//! T-FIX-48 后续：绑定当刻的**缺样本预检**（`samplerWarnings[]`）与保存回执里的
//! `missingSamples`（工具面 ＋ 宿主 `HostSaveOutcome`）。
//!
//! 两条判据：
//! 1. 绑到一个**有缺样本**的 SFZ：提案仍然成功（`created: true`，⛔ 不阻断），
//!    但响应里的 `samplerWarnings[]` 点名那个样本（轨道 / 绑定 / 样本键 / 原因 / 细节）。
//! 2. `yeban_save_project` 的**两条分支**与 `host_save_project` 的读数都带
//!    `missingSamples`（形状与 `plan` 回执同一份），且保存本身不被缺失阻断。
//!
//! 夹具：工程在 `<scratch>/proj/song.yeban`，采样库在 `<scratch>/proj/assets/samples/kit`，
//! 绑定写成工程目录相对路径（`resolve_sampler_path` 候选 ① 命中）。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::domain::sfz::{SampleDisposition, SampleLimits};
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
            "yeban-mcp-sampler-warn-{tag}-{}-{}",
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

/// `filled_project()` 的 Lead 轨换成给定音符，采样器绑定留空（由用例自己绑）。
fn project_with_notes(notes: &[(u8, u8)]) -> (YebanProjectV1, EntityId) {
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
    project.tracks.get_mut(&lead).expect("Lead").sampler = None;
    project.validate().expect("夹具工程必须合法");
    (project, lead)
}

fn fresh_dispatcher() -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    (dispatcher, auth)
}

fn call_tool(dispatcher: &mut Dispatcher, auth: &str, name: &str, arguments: Value) -> Value {
    let line = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments},
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(auth), &line);
    let response = outcome.response.expect("必须有响应");
    response.result.unwrap_or_else(|| {
        panic!(
            "工具 {name} 必须成功: {}",
            serde_json::to_string(&response.error).unwrap_or_default()
        )
    })
}

/// 搭工程目录：`<scratch>/proj/song.yeban` ＋ 采样库目录，返回（工程路径，库目录）。
fn scaffold(scratch: &Scratch) -> (PathBuf, PathBuf) {
    let kit = scratch.join("proj/assets/samples/kit");
    fs::create_dir_all(&kit).expect("建采样库目录");
    let project_path = scratch.join("proj/song.yeban");
    fs::create_dir_all(project_path.parent().expect("有父目录")).expect("建工程目录");
    (project_path, kit)
}

/// ① 绑定当刻：缺样本进 `samplerWarnings[]`，提案照建（⛔ 不阻断）。
#[test]
fn a_missing_sample_at_binding_time_is_reported_without_blocking() {
    const BINDING: &str = "assets/samples/kit/002.sfz";
    let scratch = Scratch::new("binding");
    let (project_path, kit) = scaffold(&scratch);
    write_constant_wav(&kit.join("tone60.wav"), 0.10, 4800);
    // `gone.wav` **故意不写**：这是"缺样本"的唯一来源。
    fs::write(
        kit.join("002.sfz"),
        "<region> sample=tone60.wav key=59\n<region> sample=gone.wav key=60\n",
    )
    .expect("写 SFZ 夹具");

    let (project, lead) = project_with_notes(&[(60, 100)]);
    let (mut dispatcher, auth) = fresh_dispatcher();
    dispatcher
        .domain_mut()
        .open_in_memory(project_path, project, false)
        .expect("注入夹具工程");

    let result = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": lead,
            "ops": [{"kind": "setTrackSampler", "trackId": lead, "sampler": BINDING}],
        }),
    );
    let data = &result["data"];
    assert_eq!(
        data["created"], true,
        "缺样本 ⛔ 不阻断提案（绑定本身合法）: {result}"
    );
    let warnings = data["samplerWarnings"]
        .as_array()
        .unwrap_or_else(|| panic!("响应必须带 samplerWarnings 数组: {result}"));
    assert_eq!(warnings.len(), 1, "恰一条缺样本警告: {result}");
    let warning = &warnings[0];
    assert_eq!(
        warning["sample"], "assets/samples/kit/gone.wav",
        "样本键与保存侧同一个拼法: {warning}"
    );
    assert_eq!(warning["reason"], "sample_missing", "{warning}");
    assert_eq!(warning["binding"], BINDING, "{warning}");
    assert_eq!(
        warning["trackId"],
        lead.to_canonical_string(),
        "警告要点名轨道: {warning}"
    );
    assert!(
        warning["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("gone.wav")),
        "细节要能指认是哪个文件: {warning}"
    );

    // 同一份读数在 dryRun 预览里（与真正创建同一份，⛔ 不各说一套）。
    let preview = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": lead,
            "ops": [{"kind": "setTrackSampler", "trackId": lead, "sampler": BINDING}],
            "dryRun": true,
        }),
    );
    let preview_warnings = preview["data"]["preview"]["samplerWarnings"]
        .as_array()
        .unwrap_or_else(|| panic!("dryRun 预览也要带 samplerWarnings: {preview}"));
    assert_eq!(
        preview_warnings.len(),
        1,
        "预览与响应用同一份读数: {preview}"
    );
    assert_eq!(
        preview_warnings[0]["sample"], "assets/samples/kit/gone.wav",
        "{preview}"
    );
}

/// ② 保存回执（两条分支 ＋ 宿主读数）都带 `missingSamples`，且保存不被阻断。
#[test]
fn apply_save_reports_missing_samples_in_the_receipt() {
    const BINDING: &str = "assets/samples/kit/001.sfz";
    let scratch = Scratch::new("save");
    let (project_path, kit) = scaffold(&scratch);
    write_constant_wav(&kit.join("tone60.wav"), 0.10, 4800);
    // key 60 命中夹具音符，`gone.wav` 因此进"触发集"的缺失清单。
    fs::write(
        kit.join("001.sfz"),
        "<region> sample=tone60.wav key=59\n<region> sample=gone.wav key=60\n",
    )
    .expect("写 SFZ 夹具");

    let (mut project, lead) = project_with_notes(&[(60, 100)]);
    project.tracks.get_mut(&lead).expect("Lead").sampler = Some(SamplerBinding {
        note_octave_override: None,
        path: BINDING.to_owned(),
    });
    project.validate().expect("夹具工程必须合法");

    let (mut dispatcher, auth) = fresh_dispatcher();
    dispatcher
        .domain_mut()
        .open_in_memory_with_assets(
            project_path.clone(),
            project,
            false,
            yeban_model::samples::filled_project_assets(),
        )
        .expect("注入夹具工程");

    // 第一条分支：真落盘。
    let saved = call_tool(&mut dispatcher, &auth, "yeban_save_project", json!({}));
    let data = &saved["data"];
    assert_eq!(data["saved"], true, "缺样本 ⛔ 不再阻断保存: {saved}");
    assert_eq!(data["missingSamples"]["count"], 1, "{saved}");
    assert_eq!(
        data["missingSamples"]["items"][0]["sample"], "assets/samples/kit/gone.wav",
        "{saved}"
    );
    assert_eq!(
        data["missingSamples"]["items"][0]["reason"], "sample_missing",
        "{saved}"
    );
    assert_eq!(
        data["missingSamples"]["items"][0]["trackId"],
        lead.to_canonical_string(),
        "{saved}"
    );

    // 第二条分支：跳过落盘（内存与磁盘一致）也如实带同一份读数。
    let skipped = call_tool(&mut dispatcher, &auth, "yeban_save_project", json!({}));
    let data = &skipped["data"];
    assert_eq!(data["skipped"], true, "第二次保存必须跳过: {skipped}");
    assert_eq!(
        data["missingSamples"]["count"], 1,
        "跳过分支同样要报缺样本: {skipped}"
    );

    // 宿主读数（GUI 侧走这条，不解析工具 JSON）。
    let outcome =
        yeban_mcp::domain::host_save_project(dispatcher.domain_mut(), &project_path, true)
            .expect("宿主保存必须成功");
    assert_eq!(outcome.missing_samples.len(), 1, "{outcome:?}");
    assert_eq!(
        outcome.missing_samples[0].sample.as_deref(),
        Some("assets/samples/kit/gone.wav"),
        "{outcome:?}"
    );
    assert_eq!(outcome.missing_samples[0].reason, "sample_missing");
}

/// 只有 RIFF/WAVE `fmt ` 头、没有帧数据的 WAV：`hound` 能读出 spec，
/// 但真正读样本必然报错。绑定预检若进入帧解码，就不可能对它保持安静。
fn write_header_only_wav(path: &Path, declared_frames: u32) {
    let data_bytes = declared_frames * 2;
    let mut out = Vec::with_capacity(44);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&48000_u32.to_le_bytes());
    out.extend_from_slice(&96000_u32.to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
    fs::write(path, out).expect("写只有头的 WAV 夹具");
}

/// T-FIX-648：绑定预检只读文件头，不解码整库样本帧；三类既有警告仍响亮。
///
/// 红对照：把 `sampler_binding_warnings` 里的 `sfz::probe_samples` 换回
/// `sfz::load(... SampleLimits::default())`，本判据会多出 `header_only.wav`
/// 的 `sample_unreadable` 并转红。
#[test]
fn binding_precheck_probes_headers_without_decoding_frames() {
    const BINDING: &str = "assets/samples/kit/002.sfz";
    let scratch = Scratch::new("header-probe");
    let (project_path, kit) = scaffold(&scratch);
    write_header_only_wav(&kit.join("header_only.wav"), 1_000_000);
    // 大型多样本腿：32 个各声明 8,000,000 帧的文件合计 256,000,000 帧，
    // 已超过默认 134,217,728 帧预算；但预检只读文件头，不得出现截断/缺失警告。
    let mut sfz = String::from("<region> sample=header_only.wav key=59\n");
    for index in 0..32 {
        let name = format!("bulk_{index}.wav");
        write_header_only_wav(&kit.join(&name), 8_000_000);
        sfz.push_str(&format!(
            "<region> sample={name} key={key}\n",
            key = 62 + index
        ));
    }
    // 故意不写 `gone.wav`；`bad.xyz` 是格式不认的字节。
    fs::write(kit.join("bad.xyz"), b"not an audio container").expect("写不支持格式夹具");
    sfz.push_str("<region> sample=gone.wav key=60\n<region> sample=bad.xyz key=61\n");
    fs::write(kit.join("002.sfz"), sfz).expect("写 SFZ 夹具");

    let (project, lead) = project_with_notes(&[(60, 100)]);
    let (mut dispatcher, auth) = fresh_dispatcher();
    dispatcher
        .domain_mut()
        .open_in_memory(project_path, project, false)
        .expect("注入夹具工程");

    let result = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": lead,
            "ops": [{"kind": "setTrackSampler", "trackId": lead, "sampler": BINDING}],
        }),
    );
    let warnings = result["data"]["samplerWarnings"]
        .as_array()
        .unwrap_or_else(|| panic!("响应必须带 samplerWarnings: {result}"))
        .iter()
        .collect::<Vec<_>>();
    assert_eq!(
        warnings.len(),
        2,
        "只应报缺样本与不支持格式，不得为仅头可读的 WAV 报错: {result}"
    );
    assert!(
        warnings
            .iter()
            .any(|item| item["reason"] == "sample_missing"),
        "缺样本必须仍报 sample_missing: {result}"
    );
    assert!(
        warnings
            .iter()
            .any(|item| item["reason"] == "sample_unsupported"),
        "不支持格式必须仍报 sample_unsupported: {result}"
    );
    assert!(
        !warnings
            .iter()
            .any(|item| item["reason"] == "sample_unreadable"),
        "仅头可读的 WAV 不得被当成不可读: {result}"
    );

    // 同一份 SFZ 走显式全量装载：header_only.wav 必须真的读不动 —— 证明上面的
    // 「安静」来自轻量探针，而不是夹具本身能解码。
    let load = yeban_mcp::domain::sfz::load(
        &kit.join("002.sfz"),
        48_000.0,
        0,
        false,
        SampleLimits::default(),
        &BTreeMap::new(),
    )
    .expect("SFZ 本身可装载");
    let header_disposition = load
        .sample_dispositions
        .iter()
        .find(|(sample, _)| sample.ends_with("header_only.wav"))
        .map(|(_, disposition)| disposition)
        .expect("显式装载必须处置 header_only.wav");
    assert_eq!(
        *header_disposition,
        SampleDisposition::Other,
        "显式全量装载必须把截断帧数据判成不可读: {load:?}"
    );
    let gone_disposition = load
        .sample_dispositions
        .iter()
        .find(|(sample, _)| sample.ends_with("gone.wav"))
        .map(|(_, disposition)| disposition)
        .expect("显式装载必须处置 gone.wav");
    assert_eq!(*gone_disposition, SampleDisposition::NotFound);
    let bad_disposition = load
        .sample_dispositions
        .iter()
        .find(|(sample, _)| sample.ends_with("bad.xyz"))
        .map(|(_, disposition)| disposition)
        .expect("显式装载必须处置 bad.xyz");
    assert_eq!(*bad_disposition, SampleDisposition::Unsupported);

    // ---- SFZ 本身缺失：仍是 sfz_missing，不因轻量探针变成静默 ----
    let missing_sfz = "assets/samples/kit/gone.sfz";
    let result = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_edit_notes",
        json!({
            "trackId": lead,
            "ops": [{"kind": "setTrackSampler", "trackId": lead, "sampler": missing_sfz}],
        }),
    );
    let warnings = result["data"]["samplerWarnings"]
        .as_array()
        .unwrap_or_else(|| panic!("响应必须带 samplerWarnings: {result}"))
        .iter()
        .collect::<Vec<_>>();
    assert_eq!(warnings.len(), 1, "缺 SFZ 必须恰好一条: {result}");
    assert_eq!(warnings[0]["reason"], "sfz_missing", "{result}");
}
