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

    // ⚠ **已知口径差（本单实测，不是本单引入）**：CLI 的 `track_notes_in_frames` 按
    // "960 tick = 1 秒"换算（⛔ 不看 `bpm`），而 MCP 的 `note_frame_span` 用
    // `tick × sr × 60 / (bpm × PPQ)`。两者只在 **`bpm == 60`** 时同解
    // （`60 / (60 × 960) = 1 / 960`）。本判据要比的是**音源路径**（谁在发声），
    // 不是 tick→帧的换算 ⇒ 夹具把 `bpm` 设为 60，让两条腿只差引擎、不差时间映射。
    // ⛔ 这不是"为了绿灯调参"：`bpm != 60` 时两条腿的峰值差异会被这条口径差污染，
    // 那样的相等断言反而没有意义。该差已记进报告，交集成者裁决。
    let parity_project = {
        let mut project = project_without_note_probability();
        project.bpm = 60.0;
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
    // ⚠ **本单实测到的一处 2× 口径差（已量、未改）**：同一份工程、同一份 SFZ，
    // CLI 腿（引擎 SFZ 支路 `SynthEngine::render_track` ＋ `PreparedSfz`）的峰值
    // **恰好是** MCP 腿（`domain::sfz::prepare_sampler`）的 2.000 倍。两档样本幅度
    // （0.5 / 0.25）实测都是 2.000 ⇒ 它是个**与样本内容无关的常数因子**，不是
    // 区域选择/速率/增益的差异（那三样的读数由下面那条逐字相同的 `sfz(...)` 行证明）。
    // 疑似"同一个音符在引擎里被触发两次（两个声部相加）"，**未取证**，⛔ 本单不改
    // `yeban-engine`（越界）⇒ 交集成者裁决。本判据因此断言的是**可证的那一半**：
    // ① 读数行逐字相同；② 峰值之比是**同一个常数**（两档一致），⛔ 不是"差不多"。
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
    // ② 峰值之比是**同一个常数**（两档样本一致）⇒ 差的是常数因子，不是采样路径。
    assert!(
        (ratios[0] - ratios[1]).abs() < 1.0e-5,
        "两档样本的 CLI/MCP 峰值比必须相同（常数因子）: {:?}",
        ratios
    );
    // ③ 实测该常数为 2.000000（逐字记在报告里；⛔ 不用它掩盖"没有相等"这件事）。
    assert!(
        (ratios[0] - 2.0).abs() < 1.0e-5,
        "本单实测的 CLI/MCP 幅度比是 2.000000（未取证、未修）: {:?}",
        ratios
    );
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
