//! 保存侧内嵌（`embedded_samples`）的端到端判据：只走 `tools/call` 管线。
//!
//! 夹具布局（模拟真实工程）：工程在 `<scratch>/proj/song.yeban`，绑定是渲染根相对路径
//! `assets/samples/kit/001.sfz`，SFZ 与样本都在 `<scratch>/proj/assets/samples/kit/` 下
//!（`resolve_sampler_path` 候选 ① 工程目录命中）。
//!
//! 四条判据：
//! 1. 保存只收触发集（逐项相等，无多收无漏收；多力度层只收触发到的层）；重存幂等（跳过）。
//! 2. 跨机证明：删掉盘上采样库后，用全新会话从容器打开再渲染，摘要与有库渲染逐位相同。
//! 3. 反向对照：同一工程无映射表时，空库渲染响亮失败（证明 2 的摘要相同不是静音）。
//! 4. 无采样轨的工程保存后不写 `embedded_samples` 键。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::domain::store::load_project_with_limits;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::container::ContainerLimits;
use yeban_model::samples::filled_project;
use yeban_model::{AssetHash, ClipContent, EntityId, SamplerBinding, YebanProjectV1};

/// 一个临时目录，`Drop` 时删除自己。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-embed-{tag}-{}-{}",
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

/// 夹具乐器：tone60（key 60）、tone61（key 61，未命中）、low/high（key 72 的两个力度层，
/// 各一文件）、unhit（key 80，未命中）。每文件 4800 帧，值各不相同。
fn write_fixture_kit(kit: &Path) -> String {
    fs::create_dir_all(kit).expect("建样本目录");
    write_constant_wav(&kit.join("tone60.wav"), 0.10, 4800);
    write_constant_wav(&kit.join("tone61.wav"), 0.20, 4800);
    write_constant_wav(&kit.join("low.wav"), 0.30, 4800);
    write_constant_wav(&kit.join("high.wav"), 0.40, 4800);
    write_constant_wav(&kit.join("unhit.wav"), 0.50, 4800);
    let text = "<region> sample=tone60.wav key=60\n\
                <region> sample=tone61.wav key=61\n\
                <region> sample=low.wav key=72 lovel=0 hivel=63\n\
                <region> sample=high.wav key=72 lovel=64 hivel=127\n\
                <region> sample=unhit.wav key=80\n";
    fs::write(kit.join("001.sfz"), text).expect("写 SFZ 夹具");
    text.to_owned()
}

/// `filled_project()` 的 Lead 轨换成给定音符，并绑定相对路径采样器。
fn project_with_notes_and_sampler(notes: &[(u8, u8)], binding: &str) -> YebanProjectV1 {
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
    project.tracks.get_mut(&lead).expect("Lead").sampler = Some(SamplerBinding {
        path: binding.to_owned(),
    });
    project.validate().expect("夹具工程必须合法");
    project
}

/// 新会话（每条判据独立会话，⛔ 不跨判据复用状态）。
fn fresh_dispatcher() -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    (dispatcher, auth)
}

/// 走 `tools/call` 调一个工具，返回 `result`（失败时把整条响应打出来）。
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

/// 走 `tools/call` 调一个工具，返回原始响应（成功/失败都保留，供反向对照断言）。
fn call_tool_raw(
    dispatcher: &mut Dispatcher,
    auth: &str,
    name: &str,
    arguments: Value,
) -> yeban_mcp::jsonrpc::Response {
    let line = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments},
    })
    .to_string();
    dispatcher
        .handle_line(Channel::Http, Some(auth), &line)
        .response
        .expect("必须有响应")
}

/// 渲染一次（全新会话注入工程 ＋ 可选 CAS 池），返回工具结果。
fn render(
    project: YebanProjectV1,
    project_path: &Path,
    out: &Path,
    assets: Vec<(AssetHash, Vec<u8>)>,
) -> Value {
    let (mut dispatcher, auth) = fresh_dispatcher();
    dispatcher
        .domain_mut()
        .open_in_memory_with_assets(project_path.to_path_buf(), project, false, assets)
        .expect("注入夹具工程");
    call_tool(
        &mut dispatcher,
        &auth,
        "yeban_render_master",
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "path": out.display().to_string(),
        }),
    )
}

/// 找到绑定了给定采样器路径的那条源的 `sampler` 读数。
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

/// 文件字节的 SHA-256（复用模型层的 CAS 哈希，⛔ 不引新依赖）。
fn sha_of(path: &Path) -> String {
    let bytes = fs::read(path).expect("读产物");
    AssetHash::of_bytes(&bytes).as_str().to_owned()
}

const BINDING: &str = "assets/samples/kit/001.sfz";

/// 搭一份夹具：返回（工程，SFZ 文本，工程路径）。
fn fixture(scratch: &Scratch, notes: &[(u8, u8)]) -> (YebanProjectV1, String, PathBuf) {
    let kit = scratch
        .join("proj")
        .join("assets")
        .join("samples")
        .join("kit");
    let sfz_text = write_fixture_kit(&kit);
    let project_path = scratch.join("proj").join("song.yeban");
    fs::create_dir_all(project_path.parent().expect("有父目录")).expect("建工程目录");
    let project = project_with_notes_and_sampler(notes, BINDING);
    (project, sfz_text, project_path)
}

/// 保存只收触发集：SFZ 本身 ＋ tone60 ＋ low（72 低力度层）；tone61、high（72 高力度层）、
/// unhit 一律不收；重存幂等（第二次保存跳过）。
#[test]
fn save_embeds_only_triggered_samples() {
    let scratch = Scratch::new("save");
    let (project, sfz_text, project_path) = fixture(&scratch, &[(60, 100), (72, 32)]);
    let (mut dispatcher, auth) = fresh_dispatcher();
    dispatcher
        .domain_mut()
        .open_in_memory(project_path.clone(), project, false)
        .expect("注入夹具工程");
    let saved = call_tool(&mut dispatcher, &auth, "yeban_save_project", json!({}));
    assert_eq!(saved["data"]["saved"], true, "第一次保存必须落盘: {saved}");
    assert!(project_path.is_file(), "容器必须落盘");

    let loaded =
        load_project_with_limits(&project_path, &ContainerLimits::default()).expect("读回容器");
    let kit_prefix = "assets/samples/kit/";
    let expected: BTreeMap<String, String> = [
        (
            BINDING.to_owned(),
            AssetHash::of_bytes(sfz_text.as_bytes()).as_str().to_owned(),
        ),
        (
            format!("{kit_prefix}tone60.wav"),
            AssetHash::of_bytes(
                &fs::read(scratch.join("proj").join(format!("{kit_prefix}tone60.wav")))
                    .expect("读夹具"),
            )
            .as_str()
            .to_owned(),
        ),
        (
            format!("{kit_prefix}low.wav"),
            AssetHash::of_bytes(
                &fs::read(scratch.join("proj").join(format!("{kit_prefix}low.wav")))
                    .expect("读夹具"),
            )
            .as_str()
            .to_owned(),
        ),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        loaded.project.embedded_samples, expected,
        "内嵌表必须逐项等于触发集（SFZ ＋ tone60 ＋ low），无多收无漏收"
    );
    let expected_hashes: BTreeSet<AssetHash> = expected
        .values()
        .map(|hash| AssetHash::parse(hash).expect("合法哈希"))
        .collect();
    assert_eq!(
        loaded.assets.keys().cloned().collect::<BTreeSet<_>>(),
        expected_hashes,
        "容器资产池必须恰好是内嵌表的三条哈希"
    );

    let saved_again = call_tool(&mut dispatcher, &auth, "yeban_save_project", json!({}));
    assert_eq!(
        saved_again["data"]["saved"], false,
        "重存必须跳过（幂等）: {saved_again}"
    );
    assert_eq!(
        saved_again["data"]["skipped"], true,
        "重存必须报 skipped: {saved_again}"
    );
}

/// 跨机证明：删掉盘上采样库后，用全新会话从容器打开再渲染，摘要与有库渲染逐位相同，
/// 且两条触发样本都实解码（不是静音）。
#[test]
fn embedded_project_renders_bit_identically_without_the_library() {
    let scratch = Scratch::new("xmachine");
    let (project, _sfz_text, project_path) = fixture(&scratch, &[(60, 100), (72, 32)]);
    let (mut dispatcher, auth) = fresh_dispatcher();
    dispatcher
        .domain_mut()
        .open_in_memory(project_path.clone(), project, false)
        .expect("注入夹具工程");
    let saved = call_tool(&mut dispatcher, &auth, "yeban_save_project", json!({}));
    assert_eq!(saved["data"]["saved"], true, "保存必须落盘: {saved}");

    // 有库渲染（同一会话，库还在盘上）—— 对照摘要。
    let out_a = scratch.join("a.wav");
    let result_a = render(
        load_project_with_limits(&project_path, &ContainerLimits::default())
            .expect("读回容器")
            .project,
        &project_path,
        &out_a,
        load_project_with_limits(&project_path, &ContainerLimits::default())
            .expect("读回容器")
            .assets
            .into_iter()
            .collect(),
    );
    let sha_a = sha_of(&out_a);
    let sampler_a = sampler_report(&result_a, BINDING);
    assert_eq!(
        sampler_a["sampleFailures"],
        json!([]),
        "有库渲染无读失败: {result_a}"
    );

    // 删掉盘上采样库（SFZ ＋ 全部样本），模拟换一台没有采样库的机器。
    fs::remove_dir_all(scratch.join("proj").join("assets")).expect("删采样库");

    // 全新会话从容器打开再渲染（⛔ 不复用旧会话的池）。
    let loaded =
        load_project_with_limits(&project_path, &ContainerLimits::default()).expect("读回容器");
    let out_b = scratch.join("b.wav");
    let result_b = render(
        loaded.project,
        &project_path,
        &out_b,
        loaded.assets.into_iter().collect(),
    );
    let sha_b = sha_of(&out_b);
    assert_eq!(
        sha_b, sha_a,
        "空库渲染摘要必须与有库渲染逐位相同（a={sha_a} b={sha_b}）"
    );
    let sampler_b = sampler_report(&result_b, BINDING);
    assert_eq!(
        sampler_b["sampleSources"]
            .as_array()
            .expect("sources")
            .len(),
        2,
        "空库渲染必须实解码两条触发样本: {result_b}"
    );
    assert_eq!(
        sampler_b["sampleFailures"],
        json!([]),
        "空库渲染无读失败: {result_b}"
    );
}

/// 反向对照：同一工程无映射表时，空库渲染响亮失败（证明跨机摘要相同不是静音）。
#[test]
fn render_without_embedded_map_fails_loudly_without_the_library() {
    let scratch = Scratch::new("control");
    let (project, _sfz_text, project_path) = fixture(&scratch, &[(60, 100), (72, 32)]);
    fs::remove_dir_all(scratch.join("proj").join("assets")).expect("删采样库");
    let (mut dispatcher, auth) = fresh_dispatcher();
    dispatcher
        .domain_mut()
        .open_in_memory(project_path.clone(), project, false)
        .expect("注入夹具工程");
    let response = call_tool_raw(
        &mut dispatcher,
        &auth,
        "yeban_render_master",
        json!({
            "format": "wav",
            "sampleRate": 48000,
            "path": scratch.join("control.wav").display().to_string(),
        }),
    );
    // 领域失败以带内形态返回：`result.error`（不是 JSON-RPC 层的 `error`）。
    let result = response.result.expect("失败以带内 error 形态返回");
    assert_eq!(
        result["status"], "error",
        "空库＋无映射表的渲染必须失败（⛔ 不能静默出静音）: {result}"
    );
    assert_eq!(
        result["error"]["data"]["reason"], "samplerUnreadable",
        "失败必须复用 samplerUnreadable 原因: {result}"
    );
    assert!(
        result["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("assets/samples/kit/001.sfz"),
        "失败消息必须点名缺失的 SFZ: {result}"
    );
}

/// 无采样轨的工程保存后不写 `embedded_samples` 键。
#[test]
fn save_without_sampler_tracks_writes_no_map() {
    let scratch = Scratch::new("nosampler");
    let project_path = scratch.join("proj").join("song.yeban");
    fs::create_dir_all(project_path.parent().expect("有父目录")).expect("建工程目录");
    let project = filled_project();
    let (mut dispatcher, auth) = fresh_dispatcher();
    dispatcher
        .domain_mut()
        .open_in_memory(project_path.clone(), project, false)
        .expect("注入夹具工程");
    // 无采样轨 ⇒ 内嵌表为空 ⇒ 摘要与打开时一致 ⇒ 首次保存会跳过；用 force 写盘再验容器。
    let saved = call_tool(
        &mut dispatcher,
        &auth,
        "yeban_save_project",
        json!({"force": true}),
    );
    assert_eq!(saved["data"]["saved"], true, "保存必须落盘: {saved}");
    let loaded =
        load_project_with_limits(&project_path, &ContainerLimits::default()).expect("读回容器");
    assert!(
        loaded.project.embedded_samples.is_empty(),
        "无采样轨 ⇒ 表必须为空"
    );
    assert!(loaded.assets.is_empty(), "无采样轨 ⇒ 容器不得多出资产");
}
