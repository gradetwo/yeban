//! ⭐ 任务单 `#221`：**内嵌装配**的 `mcp_mount::session_dispatcher` 真的走完
//! "绑采样器 → 渲染母带 → 真的是那个音色"（验收 `#220` 的缺陷 ① ②），
//! 并把它与 **app 的 CLI 导出腿（`render_core` → `arm_pending_sfz`）** 并排比读数。
//!
//! ## 这一节量什么（对象 ＋ 单位）
//!
//! | 量 | 来源 | 单位 |
//! | :--- | :--- | :--- |
//! | `data.sha256` | `yeban_render_master` | 64 位十六进制（母带文件字节） |
//! | `data.peak.before` | 同上（抖动/量化**之前**的浮点母带峰值） | 满刻度线性幅度 |
//! | `data.sources[].sampler.label` | 同上（本单新增的 per-track 读数） | 与 `--export-wav` 同形的 `sfz(...)` 行 |
//! | CLI 腿峰值 | `render_core::render_project_f32_with_assets_under` 返回的 `samples` 上独立求 `max |x|` | 同上 |
//! | CLI 腿 `sfz(...)` | 同一个返回值的 `source_label` | 同上 |
//!
//! ## 装配（这两条为什么都要）
//!
//! | 腿 | `Dispatcher` 从哪来 | 它证什么 |
//! | :--- | :--- | :--- |
//! | 内嵌 | [`super::session_dispatcher`]（生产路径 `InProcessMcp::start_for_project` 的**同一个符号**） | app 进程内托管的控制面能绑采样器并渲染 |
//! | stdio 形 | [`super::embedded_sfz_tools::stdio_shaped`]（`yeban-mcp` 二进制的三行 ＋ 会话注入） | 两条装配在**同一批读数**上一致（差异只有时钟与 `read_only`） |
//!
//! ⚠ 两条腿的 `Dispatcher` **都不是**在测试里 `Dispatcher::new` 手搭的：一旦复制装配，
//! "结构共享"就又回来了，而本单的存在理由正是让**生产装配**真的驱动新能力。
//!
//! ## 它**不**证什么
//!
//! ⛔ 没人耳听过（"真的是那个音色"在这里的定义是：**文件字节与峰值随绑定改变**、
//! 且与 CLI 同一条音源给出**同一个峰值**）。⛔ 不证用户机器上的真 SFZ 库
//! （夹具全部写在 `std::env::temp_dir()` 下，`Drop` 删除）。

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::Channel;

use super::embedded_sfz_tools::{
    Scratch, bearer, call_tool, embedded, project_without_note_probability, write_constant_wav,
};

/// 与 `embedded_sfz_tools::stdio_shaped` 同形，但**工程由调用方给**
/// （并列比读数时两条腿必须拿到同一份工程）。
fn stdio_shaped_with(
    project: &yeban_model::YebanProjectV1,
    scratch: &Scratch,
    now_ms: u64,
    read_only: bool,
) -> Dispatcher {
    use yeban_mcp::security::{BearerToken, RunMode, Scope, ScopeSet};
    let mut dispatcher = Dispatcher::new(
        BearerToken::generate().token,
        ScopeSet::from_scopes([Scope::AppAdmin]),
        RunMode::Production,
    );
    dispatcher.domain_mut().set_now_ms(now_ms);
    dispatcher
        .domain_mut()
        .open_in_memory(
            scratch.join("stdio-session.yeban"),
            project.clone(),
            read_only,
        )
        .expect("注入工程");
    dispatcher
}

/// 一条**全覆盖**区域：`lokey=0 hikey=127` ⇒ 样本工程里任何一个音符都能落到它上面。
///
/// 为什么不用 `key=<某个音高>`：`filled_project` 的音符集不是本判据的被测对象，
/// 判据要的是"绑了采样器 ⇒ 发声体换成这份 SFZ"，不是"某一条 key 映射对不对"。
const FULL_RANGE_SFZ: &str = "<region> sample=sampler.wav lokey=0 hikey=127 pitch_keycenter=60\n";

/// 注入的墙钟（与 `#219` 同值；只进报文字段，不进文件字节）。
const INJECTED_NOW_MS: u64 = 1_760_000_000_000;

/// 一次 `tools/call` 的 `result`（⛔ 不许有 JSON-RPC 层错误）。
fn call(dispatcher: &mut Dispatcher, channel: Channel, name: &str, arguments: Value) -> Value {
    call_tool(dispatcher, channel, name, arguments)
}

/// 从 `yeban_query_project` 的结果里挑一条**有 MIDI 片段**的音轨身份。
fn midi_track_of(dispatcher: &mut Dispatcher, channel: Channel) -> String {
    let queried = call(
        dispatcher,
        channel,
        "yeban_query_project",
        json!({"fields": ["tracks"]}),
    );
    assert_eq!(queried["status"], "success", "{queried}");
    let tracks = queried["data"]["project"]["tracks"]
        .as_object()
        .expect("tracks 对象");
    for (id, track) in tracks {
        let has_notes = track["clips"]
            .as_object()
            .is_some_and(|clips| !clips.is_empty())
            && track["kind"] == "Midi";
        if has_notes {
            return id.clone();
        }
    }
    panic!("样本工程里必须有一条带摆放的 MIDI 轨: {queried}");
}

/// 经 MCP 把 `sampler` 绑到一条轨上（`edit_notes` 建提案 ＋ `merge_proposal` 施加）。
fn bind_sampler(
    dispatcher: &mut Dispatcher,
    channel: Channel,
    track: &str,
    sampler: &str,
) -> Value {
    let edited = call(
        dispatcher,
        channel,
        "yeban_edit_notes",
        json!({
            "trackId": track,
            "ops": [{"kind": "setTrackSampler", "trackId": track, "sampler": sampler}],
        }),
    );
    assert_eq!(edited["status"], "success", "绑定必须成功: {edited}");
    let proposal = edited["data"]["proposal"]["proposalId"]
        .as_str()
        .expect("proposalId")
        .to_owned();
    let merged = call(
        dispatcher,
        channel,
        "yeban_merge_proposal",
        json!({"proposalId": proposal, "commitMessage": "绑定采样器"}),
    );
    assert_eq!(merged["data"]["merged"], true, "提案必须被施加: {merged}");
    merged
}

/// 渲染一次母带，返回 `(sha256, peakBefore, 响应)`。
fn render(
    dispatcher: &mut Dispatcher,
    channel: Channel,
    scratch: &Scratch,
    tag: &str,
) -> (String, f64, Value) {
    let out = scratch.join(&format!("{tag}.wav"));
    let result = call(
        dispatcher,
        channel,
        "yeban_render_master",
        json!({"format": "wav", "sampleRate": 48000, "path": out.display().to_string()}),
    );
    assert_eq!(result["status"], "success", "{tag}: 渲染必须成功: {result}");
    let sha = result["data"]["sha256"]
        .as_str()
        .expect("sha256")
        .to_owned();
    let peak = result["data"]["peak"]["before"]
        .as_f64()
        .expect("peak.before");
    (sha, peak, result)
}

/// 一份把采样器绑在 MIDI 轨上的工程（给 CLI 腿用；**与 MCP 会话里那份逐字段相同**）。
fn bound_project_of(
    base: &yeban_model::YebanProjectV1,
    sampler: &str,
) -> yeban_model::YebanProjectV1 {
    let mut project = base.clone();
    let track_id = project
        .tracks
        .values()
        .find(|track| {
            track.kind == yeban_model::TrackKind::Midi
                && track.clips.values().any(|placement| {
                    project
                        .clip_pool
                        .get(&placement.clip_id)
                        .is_some_and(|entry| entry.content.notes().is_some())
                })
        })
        .expect("样本工程必须有一条带音符的 MIDI 轨")
        .id;
    project.tracks.get_mut(&track_id).expect("轨").sampler = Some(yeban_model::SamplerBinding {
        path: sampler.to_owned(),
    });
    project.validate().expect("夹具工程必须合法");
    project
}

/// ⭐ 判据 5（内嵌腿）：`session_dispatcher` 造出来的会话真的能**绑采样器并把它渲染出来**。
///
/// 承重断言是**不相等**：绑之前与绑之后的 `sha256` 必须不同、峰值必须不同，
/// 且 per-track 读数点名了采样器（`#220` 缺陷 ② 修复前这三条全都不成立）。
#[test]
fn the_embedded_dispatcher_binds_a_sampler_and_the_export_really_changes() {
    let scratch = Scratch::new("sampler-bind");
    // 0.5 常量样本 ⇒ 峰值可独立估算（区域增益会缩小它，但比例是确定的）。
    write_constant_wav(&scratch.join("sampler.wav"), 0.5, 48_000, 48_000);
    let sfz = scratch.write_sfz("sampler.sfz", FULL_RANGE_SFZ);
    let sfz_path = sfz.display().to_string();

    // ---- 腿 0：不绑采样器的同一条会话形状 ----
    let mut plain = embedded(&scratch, false);
    let (sha_plain, peak_plain, _) = render(&mut plain, Channel::Http, &scratch, "plain");

    // ---- 腿 1：经 MCP 绑定，再渲染 ----
    let mut bound = embedded(&scratch, false);
    let track = midi_track_of(&mut bound, Channel::Http);
    bind_sampler(&mut bound, Channel::Http, &track, &sfz_path);
    let (sha_bound, peak_bound, response) = render(&mut bound, Channel::Http, &scratch, "bound");

    let sampler = &response["data"]["sources"]
        .as_array()
        .expect("sources")
        .iter()
        .find(|source| !source["sampler"].is_null())
        .unwrap_or_else(|| panic!("必须有一条源的 per-track 采样器读数: {response}"))["sampler"]
        .clone();
    let label = sampler["label"].as_str().expect("label");
    println!("### 内嵌腿 sha256（逐字）: 未绑={sha_plain} 已绑={sha_bound}");
    println!("### 内嵌腿峰值: 未绑={peak_plain:.9} 已绑={peak_bound:.9}");
    println!("### 内嵌腿 per-track 读数（逐字）: {label}");

    assert_ne!(
        sha_plain, sha_bound,
        "绑了采样器的导出必须与未绑不同（`#220` 缺陷 ② 的正面证据）"
    );
    assert_ne!(
        peak_plain.to_bits(),
        peak_bound.to_bits(),
        "峰值必须随绑定改变: {peak_plain} vs {peak_bound}"
    );
    assert!(
        label.starts_with("sfz(tracks=1 regions=1 samples=1 silent=0"),
        "per-track 读数必须以 sfz(tracks=1 regions=1 samples=1 silent=0 开头: {label}"
    );
    assert!(label.contains("missing=[]"), "不得有缺失样本: {label}");
    assert_eq!(
        sampler["notesUsingFileSample"].as_u64().unwrap_or(0),
        sampler["notes"].as_u64().unwrap_or(u64::MAX),
        "**每一个**排程音符都必须落在文件样本上（⛔ 不许回退正弦）: {sampler}"
    );
    assert_eq!(
        response["data"]["sources"]
            .as_array()
            .expect("sources")
            .iter()
            .filter(|source| source["kind"] == "sfz-sampler")
            .count(),
        1,
        "必须恰好一条源被换成采样器音源"
    );
}

/// ⭐ 判据 4 ＋ 5（两条装配）：同一份工程、同一份 SFZ ⇒
/// **MCP 导出**与 **CLI 导出腿**（`render_core` → `arm_pending_sfz`）给出同一个峰值、
/// 同一条 `sfz(...)` 读数；两条 MCP 装配（内嵌 / stdio 形）彼此逐位一致。
///
/// 三条承重读数：
/// - `peak` **相等**（同一音源路径的判据；不是"差不多"）；
/// - CLI 的 `sfz(...)` 行与 MCP 的 per-track 行**逐字相同**；
/// - 内嵌腿与 stdio 形腿的 `sha256` **逐字节相同**。
#[test]
fn the_mcp_export_and_the_cli_leg_agree_about_the_sampler() {
    let scratch = Scratch::new("sampler-parity");
    write_constant_wav(&scratch.join("sampler.wav"), 0.5, 48_000, 48_000);
    let sfz = scratch.write_sfz("sampler.sfz", FULL_RANGE_SFZ);
    let sfz_path = sfz.display().to_string();

    // ⭐ `#222`：本判据以前把 `bpm` 钉成 60 来回避两条口径差（CLI 的
    // `track_notes_in_frames` 不看 `bpm`；引擎 SFZ 支路把块内起音提前到块首 ⇒
    // 两个相邻脉冲叠加成 2.000000×）。两条都已修（见 `render_core`／`synth.rs`
    // 的 `#222` 段），因此现在直接用夹具自己的 **120 BPM** 跑，⛔ 不再调参回避。
    let parity_project = {
        let mut project = project_without_note_probability();
        project.bpm = 120.0;
        project.validate().expect("夹具合法");
        project
    };

    // ---- 内嵌腿 ----
    let mut embedded_dispatcher = super::session_dispatcher(
        &parity_project,
        &scratch.join("parity-embedded.yeban"),
        false,
    )
    .expect("内嵌会话装配");
    let track = midi_track_of(&mut embedded_dispatcher, Channel::Http);
    bind_sampler(&mut embedded_dispatcher, Channel::Http, &track, &sfz_path);
    let (sha_embedded, peak_embedded, _) = render(
        &mut embedded_dispatcher,
        Channel::Http,
        &scratch,
        "embedded",
    );

    // ---- stdio 形腿（同一个项目、同一条绑定；差异只有时钟与 read_only） ----
    let mut stdio_dispatcher = stdio_shaped_with(&parity_project, &scratch, INJECTED_NOW_MS, false);
    let stdio_track = midi_track_of(&mut stdio_dispatcher, Channel::Stdio);
    assert_eq!(stdio_track, track, "两条装配必须看到同一条轨");
    bind_sampler(
        &mut stdio_dispatcher,
        Channel::Stdio,
        &stdio_track,
        &sfz_path,
    );
    let (sha_stdio, peak_stdio, _) =
        render(&mut stdio_dispatcher, Channel::Stdio, &scratch, "stdio");

    // ---- CLI 腿：`render_core` →（`TrackV3.sampler` ⇒ `arm_pending_sfz`） ----
    //
    // ⭐ `#222` 的取证与修法（本判据现在钉的是**修后**的 1.000000×）：
    // 旧实现实测 `block=468 first=59904 voices=2 starts=[0,48000,60000,95400,144000]`
    // —— 同一 MIDI 音符（`ratchet=2`）拆出的两个脉冲里，起点 60000 的那个被
    // `render_track` 在**块首**触发，而 `SfzVoicePool` 从块首写 `out` ⇒ 它提前 96 帧
    // 与前一脉冲叠加，峰值 = 单声部 `0.219463661` 的**恰好 2 倍**
    // （`0.438927323`）。修法见 `synth.rs` 的 `#222` 段：按音符起始帧**分段**触发/渲染
    // ⇒ 每个声部从自己的帧起振，块内无中途起音时输出逐位不变。
    let mut ratios: Vec<f64> = Vec::new();
    let mut labels: Vec<(String, String)> = Vec::new();
    for (value, tag) in [(0.5_f64, "half"), (0.25_f64, "quarter")] {
        write_constant_wav(&scratch.join("sampler.wav"), value, 48_000, 48_000);
        let cli_project = bound_project_of(&parity_project, &sfz_path);
        let options = crate::export_wav::WavExportOptions {
            frames: 48_000 * 8,
            sample_rate: 48_000,
            channels: 2,
            use_engine_source: true,
            ..crate::export_wav::WavExportOptions::default()
        };
        let rendered = crate::render_core::render_project_f32_with_assets_under(
            &cli_project,
            options,
            None,
            &[],
            Some(scratch.dir()),
        )
        .expect("CLI 出口必须能渲染这份工程");
        let cli_peak = rendered
            .samples
            .iter()
            .fold(0.0f32, |acc, sample| acc.max(sample.abs()));
        // MCP 腿用**同一份**样本重渲一次（两档各一次）。
        let mut dispatcher = super::session_dispatcher(
            &parity_project,
            &scratch.join(&format!("parity-{tag}.yeban")),
            false,
        )
        .expect("内嵌会话装配");
        let track = midi_track_of(&mut dispatcher, Channel::Http);
        bind_sampler(&mut dispatcher, Channel::Http, &track, &sfz_path);
        let (_, mcp_peak, mcp_response) = render(&mut dispatcher, Channel::Http, &scratch, tag);
        let mcp_label = mcp_response["data"]["sources"]
            .as_array()
            .expect("sources")
            .iter()
            .find(|source| !source["sampler"].is_null())
            .expect("采样器读数")["sampler"]["label"]
            .as_str()
            .expect("label")
            .to_owned();
        println!(
            "### 样本 {value}: MCP={mcp_peak:.9} CLI={cli_peak:.9} 比值={:.6}",
            f64::from(cli_peak) / mcp_peak
        );
        println!("### 样本 {value}: CLI label={}", rendered.source_label);
        ratios.push(f64::from(cli_peak) / mcp_peak);
        labels.push((rendered.source_label.clone(), mcp_label));
    }

    // ① 两条腿的 `sfz(...)` 读数**逐字相同**（音源路径同一份的机械证据）。
    for (cli_label, mcp_label) in &labels {
        assert!(
            cli_label.contains(mcp_label),
            "CLI 的 source_label 必须包含与 MCP 逐字相同的 sfz(...) 行: {cli_label} vs {mcp_label}"
        );
    }
    // ② 两档样本的 CLI/MCP 峰值比必须**相等**（同一个比值，不是"差不多"）。
    assert!(
        (ratios[0] - ratios[1]).abs() < 1.0e-5,
        "两档样本的 CLI/MCP 峰值比必须相同: {:?}",
        ratios
    );
    // ③ ⭐ `#222` 修后的目标值：**1.000000**（两档都是）。旧值 2.000000 的取证见
    //    `synth.rs` 的 `#222` 段与报告：`voices=2`、第二个脉冲提前 96 帧叠加。
    for (index, ratio) in ratios.iter().enumerate() {
        assert!(
            (ratio - 1.0).abs() < 1.0e-5,
            "⭐ CLI/MCP 峰值比必须是 1.000000（第 {index} 档实测 {ratio:.9}）: {:?}",
            ratios
        );
    }
    // ④ 两条 MCP 装配（内嵌 / stdio 形）逐字节一致 —— 时钟与 read_only 不进音频。
    assert_eq!(
        sha_embedded, sha_stdio,
        "两条装配的母带字节必须相同（时钟/read_only 不进音频路径）"
    );
    assert_eq!(
        peak_embedded.to_bits(),
        peak_stdio.to_bits(),
        "两条装配的峰值必须相同"
    );
    let _ = bearer(&embedded_dispatcher);
}

/// ⭐ 判据 4（任务单 `#222`）：**CLI 的 tick→帧换算必须看 `bpm`**，且与 MCP 的
/// `render_math::note_frame_span` **逐值同解**。
///
/// # 量什么
///
/// 一条只有一个音符（tick 960，时值 480 tick）的工程，在 `bpm = 60 / 90 / 120 / 174`
/// 四个值下：
/// * `track_notes_in_frames` 给出的 `(起始帧, 时值帧)`；
/// * `note_frame_span(960, 480, 0, PPQ, bpm, 48000)` 给出的 `(start, end)`。
///
/// 两者必须**逐值相等**（单位：帧 @ 48 kHz）。
///
/// # 怎么变红
///
/// 把 `track_notes_in_frames` 里的模型调用改回 `tick * sample_rate / 960`（旧口径）⇒
/// 四个值里只有 `bpm = 60` 仍然相等，其余三个的起始帧全部偏小
/// （例如 `bpm = 174`：旧 48000 vs 新 16552）。
#[test]
fn the_cli_note_frames_follow_tempo_like_the_mcp_converter() {
    let mut project = project_without_note_probability();
    let track_id = project
        .tracks
        .values()
        .find(|track| {
            track.kind == yeban_model::TrackKind::Midi
                && track.clips.values().any(|placement| {
                    project
                        .clip_pool
                        .get(&placement.clip_id)
                        .is_some_and(|entry| entry.content.notes().is_some())
                })
        })
        .expect("夹具必须有一条带 MIDI 片段的轨")
        .id;
    let clip_id = project.tracks[&track_id]
        .clips
        .values()
        .next()
        .expect("轨必须有摆放")
        .clip_id;
    let notes = project
        .clip_pool
        .get_mut(&clip_id)
        .expect("夹具片段必须存在")
        .content
        .notes_mut()
        .expect("夹具片段必须是 MIDI");
    notes.clear();
    let note_id = yeban_model::EntityId::new();
    notes.insert(note_id, yeban_model::MidiNote::new(note_id, 960, 60, 480));

    let mut readings: Vec<(f64, u64, u64, u64, u64)> = Vec::new();
    for bpm in [60.0_f64, 90.0, 120.0, 174.0] {
        project.bpm = bpm;
        project.validate().expect("夹具必须合法");
        let cli = crate::export_wav::track_notes_in_frames(&project, track_id, 48_000);
        assert_eq!(cli.len(), 1, "夹具必须只有一个音符: {cli:?}");
        let (start, duration, _, _) = cli[0];
        let (mcp_start, mcp_end) = yeban_mcp::domain::render_math::note_frame_span(
            960,
            480,
            0,
            yeban_model::PPQ,
            bpm,
            48_000,
        );
        let mcp_start = u64::try_from(mcp_start).expect("非负起始帧");
        let mcp_duration =
            u64::try_from(mcp_end - i64::try_from(mcp_start).expect("非负")).expect("非负时值");
        println!("### bpm={bpm}: CLI=({start},{duration}) MCP=({mcp_start},{mcp_duration})");
        assert_eq!(
            (start, duration),
            (mcp_start, mcp_duration),
            "⭐ bpm={bpm} 时 CLI 的 (起始帧, 时值帧) 必须与 MCP 的 note_frame_span 相等"
        );
        readings.push((bpm, start, duration, mcp_start, mcp_duration));
    }
    // 目标值之一（`bpm = 60`）：**旧口径**在这里恰好同解 ⇒ 修 bpm 不许破坏它。
    let (_, start, duration, _, _) = readings[0];
    assert_eq!(
        (start, duration),
        (48_000, 24_000),
        "⛔ bpm=60 时旧口径 `tick × 48000 / 960` 与 MCP 同解 ⇒ 必须逐值不变"
    );
    // 反向对照：非 60 BPM 的读数必须**真的变**（否则上面的四个相等可能是空转）。
    assert_ne!(
        readings[1].1, start,
        "bpm=90 的起始帧必须与 bpm=60 不同（旧口径会相同）"
    );
}

/// ⭐ 判据 5（任务单 `#222`）：`bpm == 60` 时新换算必须与**旧算式逐值相同**。
///
/// 旧算式（`#222` 之前）：`start = tick × sr / 960`、
/// `duration = duration_ticks × sr / 960`（整数除法）。
/// 新算式在 `bpm = 60` 时 `60 / (960 × 60) = 1 / 960` ⇒ 数学上同解；本判据把
/// **整个夹具的每一个音符**逐值对一遍，证明"bpm=60 的导出字节不变"不是抽样结论。
#[test]
fn the_tempo_fix_is_a_no_op_at_sixty_bpm() {
    let mut project = project_without_note_probability();
    project.bpm = 60.0;
    let track_id = project
        .tracks
        .values()
        .find(|track| {
            track.kind == yeban_model::TrackKind::Midi
                && track.clips.values().any(|placement| {
                    project
                        .clip_pool
                        .get(&placement.clip_id)
                        .is_some_and(|entry| entry.content.notes().is_some())
                })
        })
        .expect("夹具必须有一条带 MIDI 片段的轨")
        .id;
    let new = crate::export_wav::track_notes_in_frames(&project, track_id, 48_000);
    let mut old: Vec<(u64, u64, u8, u8)> = Vec::new();
    for placement in project.tracks[&track_id].clips.values() {
        let Some(entry) = project.clip_pool.get(&placement.clip_id) else {
            continue;
        };
        let Some(notes) = entry.content.notes() else {
            continue;
        };
        for note in notes.values() {
            let start_tick = placement.start_tick + note.start_tick;
            let start = start_tick.saturating_mul(48_000) / 960;
            let duration = (note.duration_ticks.max(1)).saturating_mul(48_000) / 960;
            old.push((start, duration.max(1), note.pitch, note.velocity));
        }
    }
    old.sort_unstable();
    println!(
        "### bpm=60 旧算式 vs 新算式：音符数 = {}（new={} old={}）",
        new.len(),
        new.len(),
        old.len()
    );
    assert_eq!(new, old, "⛔ bpm=60 时新换算必须与旧算式逐值相同");
    assert!(!new.is_empty(), "夹具必须真的带音符（否则相等是空转）");
}

// ---------------------------------------------------------------------------
// ⭐ 任务单 `#225`：**内嵌装配**也真的把削波预警带出来（⛔ 不自动改增益）
// ---------------------------------------------------------------------------
//
// ## 这一条量什么（对象 ＋ 单位）
//
// | 量 | 来源 | 单位 |
// | :--- | :--- | :--- |
// | `data.warnings` 的条数 | 内嵌会话的 `yeban_render_master` 响应 | 条 |
// | `data.warnings[].suggestedTrimDb` | 同上（建议的母带 trim） | dB（负值 = 减） |
// | `data.sources[].peakDbfs` | 同上（逐源峰值） | dBFS |
//
// ## 它**不**证什么
//
// ⛔ 没人耳听；⛔ 不证"减完就好听"（预警只说明"会削波"与"减多少能不过载"）。
// ⚠ 本判据在 `yeban-app`（含 slint / cpal 重依赖）里 ⇒ **本机不编译**，判决由 CI 给。
// stdio 那一半在 `crates/yeban-mcp/tests/stdio_e2e.rs` 的
// `the_stdio_binary_warns_when_the_export_clips`（本机可跑）。

/// ⭐ `#225` 判据 5（内嵌腿）：`session_dispatcher` 造出来的会话在**会削波**的工程上
/// 返回恰好一条 `exportClipping` 预警，带建议 trim 与逐源峰值。
///
/// 夹具：`filled_project` 的**每一条轨**（含母带轨）推子 +24 dB ⇒ 必然过载。
/// 承重断言：`clippedSamples > 0` ＋ `warnings.len() == 1` ＋ `suggestedTrimDb < 0`。
#[test]
fn the_embedded_dispatcher_surfaces_the_clipping_warning() {
    let scratch = Scratch::new("clip-warning");
    let mut project = project_without_note_probability();
    // 母带轨也一起推 ⇒ "母带增益在求和之后"那条路径也被推到过载。
    for track in project.tracks.values_mut() {
        track.volume_db = 24.0;
    }
    project.validate().expect("过载夹具工程必须合法");

    let mut dispatcher =
        super::session_dispatcher(&project, &scratch.join("clip-warning.yeban"), false)
            .expect("内嵌会话装配");

    let (sha, peak, response) = render(&mut dispatcher, Channel::Http, &scratch, "clip-warning");
    let data = &response["data"];
    let warnings = data["warnings"]
        .as_array()
        .unwrap_or_else(|| panic!("warnings 必须是数组: {response}"));
    println!(
        "### 内嵌腿削波读数（逐字）: sha256={sha} peak.before={peak:.9} clippedSamples={} truePeakDbfs={} suggestedTrimDb={} warnings={}",
        data["clippedSamples"],
        data["truePeakDbfs"],
        warnings
            .first()
            .map_or(Value::Null, |warning| warning["suggestedTrimDb"].clone()),
        serde_json::to_string(warnings).expect("warnings")
    );
    assert!(
        data["clippedSamples"].as_u64().unwrap_or(0) > 0,
        "夹具必须真的削波（否则本条没有前提）: {response}"
    );
    assert_eq!(warnings.len(), 1, "削波工程必须恰好一条预警: {response}");
    let warning = &warnings[0];
    assert_eq!(warning["code"], "exportClipping", "{warning}");
    assert_eq!(
        warning["suggestedTrimDbFormula"], "suggestedTrimDb = -(ceil(truePeakDbfs * 2) / 2)",
        "{warning}"
    );
    assert!(
        warning["suggestedTrimDb"]
            .as_f64()
            .is_some_and(|value| value < 0.0),
        "必须给出负的 trim 建议: {warning}"
    );
    assert!(
        warning["loudestSource"]["track"].is_string(),
        "必须点名最响的源: {warning}"
    );
    // ⛔ 不自动改增益：响应里只有读数与建议；音频字节由同一次响应的 sha256 报出，
    // 这里断言它是 64 位十六进制（真落盘了）而不是"改过增益后的另一份"。
    assert_eq!(sha.len(), 64, "sha256 必须是 64 位十六进制: {sha}");
    let sources = data["sources"].as_array().expect("sources");
    assert!(!sources.is_empty(), "{response}");
    for source in sources {
        let peak = source["peak"].as_f64().expect("sources[].peak 必须是数");
        if peak > 0.0 {
            assert!(
                source["peakDbfs"].is_number(),
                "有信号的源必须有 peakDbfs: {source}"
            );
        } else {
            assert_eq!(
                source["peakDbfs"],
                Value::Null,
                "静音源的 peakDbfs 必须是 null（⛔ 不写 -inf / 0）: {source}"
            );
        }
    }
}
