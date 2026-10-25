//! `setClipPlacementLoop` 的**工具面**判据（任务单 `#336`）。
//!
//! 模型侧的参数形状判据在 `yeban-model/tests/placement_loop.rs`；本文件补的是
//! "经**真实** `tools/call` 管线（`yeban_edit_notes` → `yeban_merge_proposal`）之后，
//! 文档与**母带音频**真的变了"。
//!
//! | # | 判据 | 单位 |
//! | :-- | :--- | :--- |
//! | 1 | [`the_document_carries_exactly_the_requested_loop_config`] | `LoopConfig` 三个字段（bool / tick / tick） |
//! | 3 | [`each_loop_field_changes_independently_and_is_audible`] | 字段差异集合（个）＋ `sources[].notes`（排程音符数，个） |
//! | 4 | [`a_single_pass_placement_has_one_bar_of_energy_and_the_loop_op_fills_four`] | 逐小节 RMS（dBFS，24-bit 母带，两个声道一起算） |
//! | 5 | [`degenerate_loop_configs_render_identically_through_add_and_through_the_op`] | 逐样本差（f32 样本，必须全 0） |
//!
//! 另有一条**线上形状**判据 [`the_wire_carries_new_values_only`]：
//! 旧值键必须响亮拒绝（`unknownSetClipPlacementLoopField` + 支持集合），
//! 缺键与 `null` 是**两种**不同的形状错误。
//!
//! ## 量什么、阈值为什么这么定、会漏掉什么
//!
//! - **逐小节 RMS**：把 24-bit 母带解码成 `f32` 交错样本，按 `BAR_TICKS × 25` 帧
//!   （= 96_000 帧 = 2.000 秒）切窗，窗内两个声道的平方均值开方取 dBFS
//!   （满刻度 1.0 = 0 dBFS）。
//!   - "有声"地板 `−45 dBFS`：比 `#335` 实测的四遍 RMS（`−23.0016 dBFS`）低 22 dB，
//!     足以容纳音色/响度差异；它**会漏掉**"只有 −50 dBFS 的极轻串音"——
//!     那种能量在 24-bit 母带里仍可测，但已低于本判据的关心范围。
//!   - "静音"天花板 `−100 dBFS`：24-bit 的 1 LSB 约 `−138.5 dBFS`，单次非零样本
//!     在这 96_000 帧窗里也远低于 `−100`；反过来，任何可听串音都远高于它。
//!     它会**漏掉** `−100 dBFS` 以下的算术噪声（若有）；那是量化底噪量级。
//! - **逐样本相等**：两轨母带解码后 `f32` 逐项 `==`。这不是"差不多响"，是"同一个
//!   二进制样本序列"；它会漏掉"只在浮点舍入之外、但听感相同"的差异（不存在于此路径：
//!   两边输入逐字段相同，渲染是确定的）。
//!
//! 复跑读数：`cargo test -p yeban-mcp --test set_clip_placement_loop -- --nocapture`

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::domain::notes::{SET_CLIP_PLACEMENT_LOOP_FIELDS, SET_CLIP_PLACEMENT_LOOP_KIND};
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::LoopConfig;

// ---------------------------------------------------------------------------
// 蓝图的算术（编译期常量；不从响应里反推）
// ---------------------------------------------------------------------------

/// 960 PPQ。
const PPQ: u64 = 960;

/// 4/4 ⇒ 一小节 3_840 tick。
const BAR_TICKS: u64 = 4 * PPQ;

/// 4 小节。
const BARS: u64 = 4;

/// 摆放跨度 = 4 小节 = 15_360 tick。
const TOTAL_TICKS: u64 = BARS * BAR_TICKS;

/// 母带采样率（Hz）。
const SAMPLE_RATE: u64 = 48_000;

/// 120 BPM ⇒ 1 tick = `48_000×60 / (120×960)` = 25 帧。
const FRAMES_PER_TICK: u64 = SAMPLE_RATE * 60 / (120 * PPQ);

/// 请求的母带帧数 = 15_360 × 25 = 384_000（= 8.000 秒）。
const EXPECTED_FRAMES: u64 = TOTAL_TICKS * FRAMES_PER_TICK;

/// 一小节的帧数 = 3_840 × 25 = 96_000（= 2.000 秒）。
const BAR_FRAMES: usize = (BAR_TICKS * FRAMES_PER_TICK) as usize;

/// 音符起点（tick）：第 1 小节第 2 拍 —— **在** `0..BAR_TICKS` 的循环区间内。
const NOTE_START_TICK: u64 = 960;

/// 音符时值（tick）。
const NOTE_DURATION_TICKS: u64 = 960;

/// 音符音高：MIDI 45 = A2 = 110 Hz。
const NOTE_PITCH: u8 = 45;

/// 音符力度。
const NOTE_VELOCITY: u8 = 110;

/// "这一小节有能量"的地板（dBFS）。
const AUDIBLE_FLOOR_DBFS: f64 = -45.0;

/// "这一小节是静音"的天花板（dBFS）。
const SILENT_CEILING_DBFS: f64 = -100.0;

const NOTE_ID: u32 = 5_000;
const PLACEMENT_ID: u32 = 6_000;

// ---------------------------------------------------------------------------
// 临时目录
// ---------------------------------------------------------------------------

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |delta| delta.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-loop-{}-{tag}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
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

// ---------------------------------------------------------------------------
// 工具面管线（与 `creation_acceptance.rs` / `render_master.rs` 同一形状）
// ---------------------------------------------------------------------------

fn fresh_dispatcher() -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    (dispatcher, auth)
}

fn call(dispatcher: &mut Dispatcher, auth: &str, name: &str, arguments: Value) -> Value {
    let line = json!({
        "jsonrpc": "2.0",
        "id": "set-clip-placement-loop",
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments}
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(auth), &line);
    let response = outcome.response.expect("必须有响应");
    assert!(
        response.error.is_none(),
        "{name} 不该有 JSON-RPC 层错误: {:?}",
        response.error
    );
    response.result.expect("必须有 result")
}

/// `yeban_edit_notes` → `yeban_merge_proposal`，返回 merge 响应。
fn edit_and_merge(dispatcher: &mut Dispatcher, auth: &str, arguments: Value, label: &str) -> Value {
    let proposed = call(dispatcher, auth, "yeban_edit_notes", arguments);
    assert_eq!(proposed["status"], "success", "{label}: {proposed}");
    assert_eq!(
        proposed["data"]["pendingMerge"], true,
        "{label}: 编辑必须先进隔离提案: {proposed}"
    );
    let proposal = proposed["data"]["proposal"]["proposalId"]
        .as_str()
        .unwrap_or_else(|| panic!("{label}: 提案必须带 proposalId: {proposed}"))
        .to_owned();
    let merged = call(
        dispatcher,
        auth,
        "yeban_merge_proposal",
        json!({"proposalId": proposal, "commitMessage": format!("#336 {label}")}),
    );
    assert_eq!(merged["status"], "success", "{label} 合并: {merged}");
    merged
}

/// `yeban_edit_notes`（可能带内失败）→ 若成功则合并；返回**最后一次**响应。
///
/// 用于"两条通路都必须报同一个错"的判据：失败可能发生在编辑期（`parse_placement`
/// 的模型校验）或提案模拟期（`Op::precondition`），两条路都要能读到带内错误。
fn try_edit_and_merge(dispatcher: &mut Dispatcher, auth: &str, arguments: Value) -> Value {
    let proposed = call(dispatcher, auth, "yeban_edit_notes", arguments);
    if proposed["status"] != "success" {
        return proposed;
    }
    let proposal = proposed["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("成功提案必须带 proposalId")
        .to_owned();
    call(
        dispatcher,
        auth,
        "yeban_merge_proposal",
        json!({"proposalId": proposal, "commitMessage": "#336 parity rejection"}),
    )
}

fn query_project(dispatcher: &mut Dispatcher, auth: &str) -> Value {
    let response = call(dispatcher, auth, "yeban_query_project", json!({}));
    assert_eq!(response["status"], "success", "{response}");
    response["data"]["project"].clone()
}

/// `yeban_render_master` → 落盘 WAV；返回响应。
fn render_master(dispatcher: &mut Dispatcher, auth: &str, path: &Path, label: &str) -> Value {
    let response = call(
        dispatcher,
        auth,
        "yeban_render_master",
        json!({
            "format": "wav",
            "sampleRate": SAMPLE_RATE,
            "path": path.display().to_string(),
        }),
    );
    assert_eq!(response["status"], "success", "{label}: {response}");
    assert_eq!(
        response["data"]["frames"], EXPECTED_FRAMES,
        "{label}: 母带帧数必须等于 4 小节 × 25 帧/tick"
    );
    response
}

/// 读 24-bit PCM WAV → `(交织样本, 声道数)`（独立解码器，⛔ 不复用生产写入器）。
fn read_master(path: &Path) -> (Vec<f32>, usize) {
    let bytes = fs::read(path).expect("读母带");
    let parsed = yeban_render::rf64::parse_container(&bytes).expect("母带必须是合法容器");
    let samples: Vec<f32> = bytes[parsed.data.clone()]
        .as_chunks::<3>()
        .0
        .iter()
        .map(|chunk| {
            let raw =
                i32::from(chunk[0]) | (i32::from(chunk[1]) << 8) | (i32::from(chunk[2]) << 16);
            let signed = (raw << 8) >> 8;
            #[allow(clippy::cast_precision_loss)]
            let value = signed as f32 / 8_388_608.0;
            value
        })
        .collect();
    (samples, usize::from(parsed.format.channels))
}

/// `[start_frame, start_frame + frames)` 的 RMS（dBFS，两个声道一起算）。
fn window_rms_dbfs(samples: &[f32], channels: usize, start_frame: usize, frames: usize) -> f64 {
    let available = samples.len() / channels;
    let mut sum = 0.0f64;
    let mut count = 0usize;
    for frame in start_frame..(start_frame + frames).min(available) {
        for channel in 0..channels {
            let value = f64::from(samples[frame * channels + channel]);
            sum += value * value;
            count += 1;
        }
    }
    if count == 0 {
        return f64::NEG_INFINITY;
    }
    let mean = sum / count as f64;
    if mean <= 0.0 {
        return f64::NEG_INFINITY;
    }
    20.0 * mean.sqrt().log10()
}

/// 四个小节窗口的 RMS（dBFS）。
fn bar_rms(samples: &[f32], channels: usize) -> Vec<f64> {
    (0..BARS as usize)
        .map(|bar| window_rms_dbfs(samples, channels, bar * BAR_FRAMES, BAR_FRAMES))
        .collect()
}

/// 一条轨在母带响应里的**排程音符数**（`sources[].notes`）。
fn scheduled_notes(render: &Value, track_id: &str) -> u64 {
    render["data"]["sources"]
        .as_array()
        .unwrap_or_else(|| panic!("母带响应必须有 sources: {render}"))
        .iter()
        .find(|source| source["node"] == track_id)
        .unwrap_or_else(|| panic!("找不到音轨 {track_id} 的源报告: {render}"))
        .get("notes")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("源报告必须带 notes: {render}"))
}

// ---------------------------------------------------------------------------
// 工程夹具（全部经工具面）
// ---------------------------------------------------------------------------

/// 一个合法的 26 字符 ULID 文本（末 4 位由 `seed` 决定）。
fn eid(seed: u32) -> String {
    let text = format!("01J8ZR{:0>16}{seed:04}", 0);
    assert_eq!(text.len(), 26, "ULID 必须 26 字符");
    text
}

/// 建 1 轨工程、在种子片段里写一个音符；返回 `(trackId, placementId, clipId)`。
///
/// 音符**不由** `seed.notes` 给（那一栏把第 i 个音符放在 `i × PPQ`，放不到 960），
/// 而是建完工程后经 `add` op 写进种子片段。
fn create_and_fill(
    dispatcher: &mut Dispatcher,
    auth: &str,
    path: &Path,
) -> (String, String, String) {
    let created = call(
        dispatcher,
        auth,
        "yeban_open_project",
        json!({
            "path": path.display().to_string(),
            "create": true,
            "title": "loop parity",
            "bpm": 120.0,
            "seed": {"trackCount": 1, "clipName": "Loop Material", "notes": []},
        }),
    );
    assert_eq!(created["status"], "success", "建工程必须成功: {created}");
    assert_eq!(created["data"]["created"], true, "{created}");

    let project = query_project(dispatcher, auth);
    let tracks = project["tracks"].as_object().expect("tracks");
    let master = project["master_bus_track_id"].as_str().expect("主总线");
    let track = tracks
        .iter()
        .find(|(id, _)| id.as_str() != master)
        .map(|(id, _)| id.clone())
        .expect("必须有一条非主总线音轨");
    let placements = project["tracks"][&track]["clips"]
        .as_object()
        .expect("音轨必须有 clips");
    let (placement, placement_value) = placements.iter().next().expect("种子摆放");
    let clip = placement_value["clip_id"]
        .as_str()
        .expect("摆放必须带 clip_id")
        .to_owned();

    edit_and_merge(
        dispatcher,
        auth,
        json!({
            "trackId": track,
            "clipId": clip,
            "ops": [{
                "kind": "add",
                "note": {
                    "id": eid(NOTE_ID),
                    "startTick": NOTE_START_TICK,
                    "pitch": NOTE_PITCH,
                    "durationTicks": NOTE_DURATION_TICKS,
                    "velocity": NOTE_VELOCITY,
                },
            }],
        }),
        "seed note",
    );
    (track, placement.clone(), clip)
}

/// 把种子摆放拉长到 4 小节（仍是**单遍**：循环配置保持缺省关闭）。
fn stretch_to_four_bars(dispatcher: &mut Dispatcher, auth: &str, track: &str, placement: &str) {
    edit_and_merge(
        dispatcher,
        auth,
        json!({
            "trackId": track,
            "ops": [{
                "kind": "setClipPlacementDuration",
                "trackId": track,
                "placementId": placement,
                "durationTicks": TOTAL_TICKS,
            }],
        }),
        "stretch placement to 4 bars",
    );
}

/// 经本票的新 op 写入一份循环配置。
fn set_loop(
    dispatcher: &mut Dispatcher,
    auth: &str,
    track: &str,
    placement: &str,
    config: LoopConfig,
    label: &str,
) {
    edit_and_merge(
        dispatcher,
        auth,
        json!({
            "trackId": track,
            "ops": [{
                "kind": SET_CLIP_PLACEMENT_LOOP_KIND,
                "trackId": track,
                "placementId": placement,
                "loopEnabled": config.enabled,
                "loopStartTick": config.start_tick,
                "loopEndTick": config.end_tick,
            }],
        }),
        label,
    );
}

/// 当前文档里那条摆放的循环配置（`query_project` 的全量视图）。
fn placement_loop(project: &Value, track: &str, placement: &str) -> LoopConfig {
    let value = &project["tracks"][track]["clips"][placement]["loop_config"];
    LoopConfig {
        enabled: value["enabled"]
            .as_bool()
            .unwrap_or_else(|| panic!("loop_config.enabled 必须是布尔: {value}")),
        start_tick: value["start_tick"]
            .as_u64()
            .unwrap_or_else(|| panic!("loop_config.start_tick 必须是非负整数: {value}")),
        end_tick: value["end_tick"]
            .as_u64()
            .unwrap_or_else(|| panic!("loop_config.end_tick 必须是非负整数: {value}")),
    }
}

/// 渲染并解码一条音轨的母带（把 `track` 之外的音轨静音，留一个干净的单轨母带）。
fn render_track(
    dispatcher: &mut Dispatcher,
    auth: &str,
    _track: &str,
    path: &Path,
    label: &str,
) -> (Vec<f32>, usize, Value) {
    let response = render_master(dispatcher, auth, path, label);
    let (samples, channels) = read_master(path);
    (samples, channels, response)
}

// ---------------------------------------------------------------------------
// 判据 1：文档携带的 LoopConfig 逐字段等于请求值
// ---------------------------------------------------------------------------

/// 判据 1：经工具面写进文档的 `LoopConfig` **逐字段**等于请求值。
///
/// 量什么：`yeban_query_project` 读回的 `loop_config.{enabled,start_tick,end_tick}`
/// 与请求的三个值逐项相等（bool / tick / tick）。
///
/// 注入（实测红）：把 `compile` 里的 `old_loop_config: placement.loop_config`
/// 改成 `LoopConfig::default()`（旧值不再从文档读）⇒ 第一次之后的第二次写入会因
/// 陈旧载荷被模型拒绝 ⇒ 本判据在第二步就红。
#[test]
fn the_document_carries_exactly_the_requested_loop_config() {
    let scratch = Scratch::new("roundtrip");
    let (mut dispatcher, auth) = fresh_dispatcher();
    let (track, placement, _clip) =
        create_and_fill(&mut dispatcher, &auth, &scratch.join("roundtrip.yeban"));
    stretch_to_four_bars(&mut dispatcher, &auth, &track, &placement);

    let requested = LoopConfig {
        enabled: true,
        start_tick: 0,
        end_tick: BAR_TICKS,
    };
    set_loop(
        &mut dispatcher,
        &auth,
        &track,
        &placement,
        requested,
        "set loop 0..bar",
    );
    assert_eq!(
        placement_loop(&query_project(&mut dispatcher, &auth), &track, &placement),
        requested,
        "文档必须逐字段携带请求的 LoopConfig"
    );

    // 第二次整体替换（同时改三项）：旧值必须从文档读，否则模型会报陈旧载荷。
    let second = LoopConfig {
        enabled: false,
        start_tick: 480,
        end_tick: 1_440,
    };
    set_loop(
        &mut dispatcher,
        &auth,
        &track,
        &placement,
        second,
        "replace loop wholly",
    );
    assert_eq!(
        placement_loop(&query_project(&mut dispatcher, &auth), &track, &placement),
        second,
        "整体替换必须写入第二份配置的**全部**三项（⛔ 不是合并）"
    );
}

// ---------------------------------------------------------------------------
// 判据 3：三个字段各自生效（字段 + 可听行为都可区分）
// ---------------------------------------------------------------------------

/// 判据 3：`start_tick` / `end_tick` / `enabled` **各自**生效。
///
/// 量什么（每步两组读数）：
/// - 字段差异集合（相对上一步，恰好一个字段）；
/// - `sources[].notes`：渲染响应里该轨**真的排程**的音符数（个）。
///
/// 序列（`D = 15_360`，音符在 tick 960）：
/// 1. `{true, 0, 3840}` ⇒ `N = 4` ⇒ notes = 4；
/// 2. 只改 `start_tick` → `{true, 960, 3840}` ⇒ `L = 2880`，
///    `N = ceil((15360−960)/2880) = 5` ⇒ notes = 5；
/// 3. 只改 `end_tick` → `{true, 960, 1920}` ⇒ `L = 960`，
///    `N = ceil((15360−960)/960) = 15` ⇒ notes = 15；
/// 4. 只翻 `enabled` → `{false, 960, 1920}` ⇒ 单遍 ⇒ notes = 1。
///
/// 四步的 notes 读数两两不同 ⇒ 每一处改动都**可区分于"没变"**（若某一字段被静默
/// 忽略，那一步的字段差异集合与 notes 读数都会停在上一值）。
///
/// 注入（实测红）：`commit` 里只写 `enabled`（丢掉 `start_tick`/`end_tick`）⇒
/// 第 2 步就红（字段差异是 0 个、notes 仍 4）。
#[test]
fn each_loop_field_changes_independently_and_is_audible() {
    let scratch = Scratch::new("fields");
    let (mut dispatcher, auth) = fresh_dispatcher();
    let (track, placement, _clip) =
        create_and_fill(&mut dispatcher, &auth, &scratch.join("fields.yeban"));
    stretch_to_four_bars(&mut dispatcher, &auth, &track, &placement);

    let base = LoopConfig {
        enabled: true,
        start_tick: 0,
        end_tick: BAR_TICKS,
    };
    set_loop(
        &mut dispatcher,
        &auth,
        &track,
        &placement,
        base,
        "base loop",
    );

    let steps = [
        (
            "start_tick only",
            LoopConfig {
                enabled: true,
                start_tick: 960,
                end_tick: BAR_TICKS,
            },
            "start_tick",
            5_u64,
        ),
        (
            "end_tick only",
            LoopConfig {
                enabled: true,
                start_tick: 960,
                end_tick: 2 * PPQ,
            },
            "end_tick",
            15_u64,
        ),
        (
            "enabled only",
            LoopConfig {
                enabled: false,
                start_tick: 960,
                end_tick: 2 * PPQ,
            },
            "enabled",
            1_u64,
        ),
    ];

    let mut previous = base;
    let mut previous_notes = {
        let path = scratch.join("base.wav");
        let (_samples, _channels, render) =
            render_track(&mut dispatcher, &auth, &track, &path, "base loop render");
        scheduled_notes(&render, &track)
    };
    assert_eq!(previous_notes, 4, "base `0..3840` ⇒ N = 4");

    for (label, next, changed_field, expected_notes) in steps {
        set_loop(&mut dispatcher, &auth, &track, &placement, next, label);
        let project = query_project(&mut dispatcher, &auth);
        let got = placement_loop(&project, &track, &placement);
        assert_eq!(got, next, "{label}: 三项必须整体等于新配置");
        assert_ne!(got, previous, "{label}: 改动必须可区分于'没变'");

        let mut differing = Vec::new();
        if got.enabled != previous.enabled {
            differing.push("enabled");
        }
        if got.start_tick != previous.start_tick {
            differing.push("start_tick");
        }
        if got.end_tick != previous.end_tick {
            differing.push("end_tick");
        }
        assert_eq!(
            differing,
            vec![changed_field],
            "{label}: 只有 `{changed_field}` 该变"
        );

        let path = scratch.join(&format!("{}.wav", label.replace(' ', "-")));
        let (_samples, _channels, render) =
            render_track(&mut dispatcher, &auth, &track, &path, label);
        assert_eq!(
            scheduled_notes(&render, &track),
            expected_notes,
            "{label}: 排程音符数必须按新配置变化"
        );
        previous = next;
        previous_notes = scheduled_notes(&render, &track);
    }
    assert_eq!(previous_notes, 1, "最后一步是关闭循环（单遍）");
}

// ---------------------------------------------------------------------------
// 判据 4：单遍只有第 1 小节有声，设循环后四小节都有声
// ---------------------------------------------------------------------------

/// 判据 4（音频端到端）：单遍摆放只有第 1 小节有能量；设 `loopEnabled` 且
/// `loopEndTick` = 1 小节之后，同一条摆放的四小节都有能量。
///
/// 量什么：24-bit 母带的**逐小节 RMS**（dBFS）。单遍：第 1 小节 > −45，第 2/3/4
/// 小节 < −100；循环：四小节全部 > −45。
///
/// 另外把"恰好覆盖"（`end == duration`）经本 op 写回，断言母带**逐字节**等于
/// 单遍那一次（`sha256` 相同）—— 即单遍母带的**钉住字节**没有被本变体改变。
///
/// 注入（实测红）：把 `commit` 里的 `placement.loop_config = *new_loop_config` 删掉
/// ⇒ 循环那次渲染的 2/3/4 小节 < −100 ⇒ 红；把模型的 `note_copies` 展开条件写坏
/// 同理。
#[test]
fn a_single_pass_placement_has_one_bar_of_energy_and_the_loop_op_fills_four() {
    let scratch = Scratch::new("audio");
    let (mut dispatcher, auth) = fresh_dispatcher();
    let (track, placement, _clip) =
        create_and_fill(&mut dispatcher, &auth, &scratch.join("audio.yeban"));
    stretch_to_four_bars(&mut dispatcher, &auth, &track, &placement);

    // ---- 单遍 ----
    let single_path = scratch.join("single.wav");
    let (single_samples, single_channels, single) =
        render_track(&mut dispatcher, &auth, &track, &single_path, "single pass");
    let single_rms = bar_rms(&single_samples, single_channels);
    println!("### [336] 单遍逐小节 RMS = {single_rms:?} dBFS");
    assert!(
        single_rms[0] > AUDIBLE_FLOOR_DBFS,
        "单遍时第 1 小节必须有能量：{single_rms:?}"
    );
    for (bar, value) in single_rms.iter().enumerate().skip(1) {
        assert!(
            *value < SILENT_CEILING_DBFS,
            "单遍时第 {} 小节必须静音（循环关闭）：{single_rms:?}",
            bar + 1
        );
    }
    assert_eq!(
        scheduled_notes(&single, &track),
        1,
        "单遍：一个音符只排一遍"
    );
    let single_sha = single["data"]["sha256"]
        .as_str()
        .expect("sha256")
        .to_owned();

    // ---- 设循环：区间 = 1 小节 ⇒ 4 遍铺满 4 小节 ----
    let looped_config = LoopConfig {
        enabled: true,
        start_tick: 0,
        end_tick: BAR_TICKS,
    };
    set_loop(
        &mut dispatcher,
        &auth,
        &track,
        &placement,
        looped_config,
        "loop one bar over four",
    );
    let looped_path = scratch.join("looped.wav");
    let (looped_samples, looped_channels, looped) =
        render_track(&mut dispatcher, &auth, &track, &looped_path, "looped");
    let looped_rms = bar_rms(&looped_samples, looped_channels);
    println!("### [336] 循环逐小节 RMS = {looped_rms:?} dBFS");
    for (bar, value) in looped_rms.iter().enumerate() {
        assert!(
            *value > AUDIBLE_FLOOR_DBFS,
            "循环必须铺满 4 小节：第 {} 小节 RMS 必须 > {AUDIBLE_FLOOR_DBFS} dBFS，实测 {value:.3}",
            bar + 1
        );
    }
    assert_eq!(
        scheduled_notes(&looped, &track),
        4,
        "N = ceil(15360 / 3840) = 4：一个音符必须排四遍"
    );
    assert_ne!(
        looped["data"]["sha256"], single["data"]["sha256"],
        "循环必须真的改变母带字节"
    );

    // ---- 恰好覆盖（end == D）经本 op 写回 ⇒ 单遍母带逐字节复原 ----
    let covered = LoopConfig {
        enabled: true,
        start_tick: 0,
        end_tick: TOTAL_TICKS,
    };
    set_loop(
        &mut dispatcher,
        &auth,
        &track,
        &placement,
        covered,
        "exact coverage is one pass",
    );
    let covered_path = scratch.join("covered.wav");
    let (covered_samples, covered_channels, covered_render) =
        render_track(&mut dispatcher, &auth, &track, &covered_path, "covered");
    assert_eq!(
        covered_render["data"]["sha256"], single_sha,
        "`end == duration_ticks` ⇒ 单遍，母带必须与单遍那一次逐字节相同"
    );
    assert_eq!(
        covered_samples, single_samples,
        "逐样本也必须相同（不只是文件摘要）"
    );
    assert_eq!(covered_channels, single_channels);
    assert_eq!(
        scheduled_notes(&covered_render, &track),
        1,
        "恰好覆盖 ⇒ 单遍"
    );
}

// ---------------------------------------------------------------------------
// 判据 5：退化配置经 `add` 与经本 op 的音频相同
// ---------------------------------------------------------------------------

/// 构建通路的两种拼法。
#[derive(Clone, Copy)]
enum LoopPath {
    /// `placement` 的 `add` 形态带 `loopEnabled` / `loopStartTick` / `loopEndTick`。
    Add,
    /// 先 `add`（缺省关闭）再 `setClipPlacementLoop` 整体替换。
    LoopOp,
}

/// 用给定通路把工程搭成"4 小节摆放 + 一份循环配置"。
fn build_with_path(
    dispatcher: &mut Dispatcher,
    auth: &str,
    path: &Path,
    loop_path: LoopPath,
    config: LoopConfig,
) {
    let (track, placement, clip) = create_and_fill(dispatcher, auth, path);
    match loop_path {
        LoopPath::Add => {
            // 取走种子摆放，再用带循环配置的 `add` 摆一条 4 小节的。
            edit_and_merge(
                dispatcher,
                auth,
                json!({
                    "trackId": track,
                    "clipId": clip,
                    "ops": [],
                    "placement": {"kind": "remove", "placementId": placement},
                }),
                "remove seed placement",
            );
            let mut payload = json!({
                "startTick": 0,
                "durationTicks": TOTAL_TICKS,
                "placementId": eid(PLACEMENT_ID),
            });
            if config.enabled {
                payload["loopEnabled"] = Value::from(true);
                payload["loopStartTick"] = Value::from(config.start_tick);
                payload["loopEndTick"] = Value::from(config.end_tick);
            }
            edit_and_merge(
                dispatcher,
                auth,
                json!({
                    "trackId": track,
                    "clipId": clip,
                    "ops": [],
                    "placement": payload,
                }),
                "add placement with loop config",
            );
        }
        LoopPath::LoopOp => {
            stretch_to_four_bars(dispatcher, auth, &track, &placement);
            set_loop(
                dispatcher,
                auth,
                &track,
                &placement,
                config,
                "set loop through the new op",
            );
        }
    }
}

/// 判据 5（退化配置的**音频等价**）：同一份退化配置，经 `add` 的循环字段与经本 op
/// 写进文档后，母带**逐样本相同**。
///
/// 覆盖的退化形态（都是 `add` **真的接受**的形态）：
/// - `enabled: true, start: 0, end == D`（恰好覆盖 ⇒ 单遍）；
/// - `enabled: true, start: 0, end > D`（越过末端 ⇒ 单遍）；
/// - `enabled: false, start: 0, end: 0`（关闭 ⇒ 单遍）。
///
/// 外加 `end <= start` 的两半：
/// - `enabled: true, start == end`：两条通路都必须**同样拒绝**（`OUT_OF_RANGE`）；
/// - `enabled: false, start: 960, end: 480`（`end < start`）：⭐ **`add` 表达不出来**
///   （关闭循环时给区间会被 `placementFieldNotApplicable` 拒绝），所以只能证明
///   "它经本 op 的音频 == `add` 的缺省关闭音频" —— 两者都是单遍。
///   这一条**如实**登记了"同一份配置"在 `add` 上不可表达；本 op 的接受集合仍与
///   `add` 的**共享校验**一致（见模型判据）。
///
/// 量什么：两份母带的解码 `f32` 样本向量**逐项相等**（不是 dBFS 近似）。
///
/// 注入（实测红）：在 `commit` 里把 `new_loop_config` 换成立即 `validate` 后再
/// 规范化（例如 `end = min(end, duration)`）⇒ "越过末端"那份会变成不同音频 / 文件；
/// 把校验改成拒绝 `end <= start` 的关闭形态 ⇒ 最后一条构建失败。
#[test]
fn degenerate_loop_configs_render_identically_through_add_and_through_the_op() {
    let scratch = Scratch::new("parity");

    let parity: [(&str, LoopConfig); 3] = [
        (
            "exact-coverage",
            LoopConfig {
                enabled: true,
                start_tick: 0,
                end_tick: TOTAL_TICKS,
            },
        ),
        (
            "past-the-end",
            LoopConfig {
                enabled: true,
                start_tick: 0,
                end_tick: TOTAL_TICKS + 960,
            },
        ),
        (
            "disabled-zero-interval",
            LoopConfig {
                enabled: false,
                start_tick: 0,
                end_tick: 0,
            },
        ),
    ];

    for (tag, config) in parity {
        let (add_samples, add_channels) =
            render_path(&scratch, &format!("add-{tag}"), LoopPath::Add, config, tag);
        let (op_samples, op_channels) = render_path(
            &scratch,
            &format!("op-{tag}"),
            LoopPath::LoopOp,
            config,
            tag,
        );
        assert_eq!(add_channels, op_channels);
        assert_eq!(
            add_samples, op_samples,
            "{tag}: `add` 与 `setClipPlacementLoop` 的母带必须逐样本相同"
        );
    }

    // ---- `enabled && end == start`：两条通路必须同样拒绝 ----
    let (mut add_dispatcher, add_auth) = fresh_dispatcher();
    let (track, placement, clip) = create_and_fill(
        &mut add_dispatcher,
        &add_auth,
        &scratch.join("reject-add.yeban"),
    );
    let add_response = try_edit_and_merge(
        &mut add_dispatcher,
        &add_auth,
        json!({
            "trackId": track,
            "clipId": clip,
            "ops": [],
            "placement": {
                "startTick": 0,
                "durationTicks": TOTAL_TICKS,
                "placementId": eid(PLACEMENT_ID),
                "loopEnabled": true,
                "loopStartTick": 960,
                "loopEndTick": 960,
            },
        }),
    );
    assert_eq!(
        add_response["status"], "error",
        "`add` 必须拒绝 enabled && end == start: {add_response}"
    );
    let _ = placement;

    let (mut op_dispatcher, op_auth) = fresh_dispatcher();
    let (track, placement, _clip) = create_and_fill(
        &mut op_dispatcher,
        &op_auth,
        &scratch.join("reject-op.yeban"),
    );
    stretch_to_four_bars(&mut op_dispatcher, &op_auth, &track, &placement);
    let op_response = try_edit_and_merge(
        &mut op_dispatcher,
        &op_auth,
        json!({
            "trackId": track,
            "ops": [{
                "kind": SET_CLIP_PLACEMENT_LOOP_KIND,
                "trackId": track,
                "placementId": placement,
                "loopEnabled": true,
                "loopStartTick": 960,
                "loopEndTick": 960,
            }],
        }),
    );
    assert_eq!(
        op_response["status"], "error",
        "本 op 必须拒绝同一份配置: {op_response}"
    );
    assert_eq!(
        add_response["error"]["code"], op_response["error"]["code"],
        "两条通路必须报**同一个**契约码"
    );

    // ---- `disabled && end < start`：`add` 不可表达，只能与它的缺省关闭音频对账 ----
    let (add_samples, add_channels) = render_path(
        &scratch,
        "add-disabled-reversed",
        LoopPath::Add,
        LoopConfig {
            enabled: false,
            start_tick: 0,
            end_tick: 0,
        },
        "add disabled default",
    );
    let (op_samples, op_channels) = render_path(
        &scratch,
        "op-disabled-reversed",
        LoopPath::LoopOp,
        LoopConfig {
            enabled: false,
            start_tick: 960,
            end_tick: 480,
        },
        "op disabled end < start",
    );
    assert_eq!(add_channels, op_channels);
    assert_eq!(
        add_samples, op_samples,
        "`enabled: false, end < start` 经本 op 必须与 `add` 的关闭循环音频逐样本相同"
    );
}

/// 用给定通路搭工程、渲染并解码母带。
fn render_path(
    scratch: &Scratch,
    tag: &str,
    path_kind: LoopPath,
    config: LoopConfig,
    label: &str,
) -> (Vec<f32>, usize) {
    let (mut dispatcher, auth) = fresh_dispatcher();
    let project_path = scratch.join(&format!("{tag}.yeban"));
    build_with_path(&mut dispatcher, &auth, &project_path, path_kind, config);
    let project = query_project(&mut dispatcher, &auth);
    let tracks = project["tracks"].as_object().expect("tracks");
    let master = project["master_bus_track_id"].as_str().expect("主总线");
    let track = tracks
        .iter()
        .find(|(id, _)| id.as_str() != master)
        .map(|(id, _)| id.clone())
        .expect("非主总线音轨");
    let wav = scratch.join(&format!("{tag}.wav"));
    let (samples, channels, _render) = render_track(&mut dispatcher, &auth, &track, &wav, label);
    (samples, channels)
}

// ---------------------------------------------------------------------------
// 线上形状：只带新值；未知键 / 缺键 / null 三种失败各有各的样子
// ---------------------------------------------------------------------------

/// `setClipPlacementLoop` 的线上形状判据。
///
/// 1. **旧值键必须响亮拒绝**：`oldLoopConfig` / `oldEnabled` / `oldStartTick` /
///    `oldEndTick` 全部是未知键 ⇒ `INVALID_PARAMETER_RANGE` +
///    `reason = unknownSetClipPlacementLoopField`，`supportedFields` **逐字**等于
///    [`SET_CLIP_PLACEMENT_LOOP_FIELDS`]（5 个新值键 + `kind`）。
/// 2. **缺键 ≠ null**：缺一个键报"缺少 `X`"（`data` 为空）；`loopStartTick: null`
///    是**形状错误**（`reason = valueMustBeUnsignedInteger`），`loopEnabled: null`
///    是 `valueMustBeBoolean` —— 两者都不是"缺省"。
/// 3. 三个新值键**全部必填**：逐个删掉都必须红。
///
/// 注入（实测红）：把 `reject_set_clip_placement_loop_fields` 的调用删掉 ⇒ 第 1 条
/// 的四条载荷全部变成 `success`（旧值键被静默忽略）⇒ 红。
#[test]
fn the_wire_carries_new_values_only() {
    let scratch = Scratch::new("wire");
    let (mut dispatcher, auth) = fresh_dispatcher();
    let (track, placement, _clip) =
        create_and_fill(&mut dispatcher, &auth, &scratch.join("wire.yeban"));

    let base = json!({
        "kind": SET_CLIP_PLACEMENT_LOOP_KIND,
        "trackId": track,
        "placementId": placement,
        "loopEnabled": true,
        "loopStartTick": 0,
        "loopEndTick": BAR_TICKS,
    });

    // ---- 1. 旧值键：四种都响亮拒绝 ----
    for old_key in ["oldLoopConfig", "oldEnabled", "oldStartTick", "oldEndTick"] {
        let mut payload = base.clone();
        payload[old_key] = match old_key {
            "oldLoopConfig" => json!({"enabled": true, "start_tick": 0, "end_tick": BAR_TICKS}),
            "oldEnabled" => json!(true),
            _ => json!(0),
        };
        let response = call(
            &mut dispatcher,
            &auth,
            "yeban_edit_notes",
            json!({"trackId": track, "ops": [payload]}),
        );
        assert_eq!(
            response["status"], "error",
            "`{old_key}` 是未知键，必须被拒: {response}"
        );
        assert_eq!(
            response["error"]["code"], "INVALID_PARAMETER_RANGE",
            "`{old_key}` 的契约码: {response}"
        );
        assert_eq!(
            response["error"]["data"]["reason"], "unknownSetClipPlacementLoopField",
            "`{old_key}` 的 reason: {response}"
        );
        assert_eq!(
            response["error"]["data"]["supportedFields"],
            json!(SET_CLIP_PLACEMENT_LOOP_FIELDS),
            "`{old_key}` 的拒绝报文必须列出**同一份**支持集合: {response}"
        );
    }

    // ---- 2. 缺键：三个新值键逐个删 ----
    for missing in ["loopEnabled", "loopStartTick", "loopEndTick"] {
        let mut payload = base.clone();
        payload.as_object_mut().expect("对象").remove(missing);
        let response = call(
            &mut dispatcher,
            &auth,
            "yeban_edit_notes",
            json!({"trackId": track, "ops": [payload]}),
        );
        assert_eq!(
            response["status"], "error",
            "缺 `{missing}` 必须响亮失败: {response}"
        );
        assert_eq!(
            response["error"]["code"], "INVALID_PARAMETER_RANGE",
            "缺 `{missing}` 的契约码: {response}"
        );
        assert_eq!(
            response["error"]["data"],
            Value::Null,
            "缺键是'缺少字段'（不带 data），与 `null` 的形状错误**不同**: {response}"
        );
        assert!(
            response["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains(missing)),
            "缺键报文必须点名 `{missing}`: {response}"
        );
    }

    // ---- 3. `null` 是形状错误，不是缺省 ----
    for (field, reason) in [
        ("loopEnabled", "valueMustBeBoolean"),
        ("loopStartTick", "valueMustBeUnsignedInteger"),
        ("loopEndTick", "valueMustBeUnsignedInteger"),
    ] {
        let mut payload = base.clone();
        payload[field] = Value::Null;
        let response = call(
            &mut dispatcher,
            &auth,
            "yeban_edit_notes",
            json!({"trackId": track, "ops": [payload]}),
        );
        assert_eq!(
            response["status"], "error",
            "`{field}: null` 必须被拒: {response}"
        );
        assert_eq!(
            response["error"]["data"]["reason"], reason,
            "`{field}: null` 的 reason: {response}"
        );
    }
}
