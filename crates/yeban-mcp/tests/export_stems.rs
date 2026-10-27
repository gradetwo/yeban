//! ⭐ `ADR-0030`：**`yeban_export_stems` 的端到端判据** —— 一次调用出 N 条 32-bit Float 分轨。
//!
//! 缺口的形状（审计 `G9b`）：工具面只有母带，"每条轨一个 WAV"必须由调用方自己
//! 完成 `N × (克隆工程 → 只留一条轨 → yeban_render_master → 换文件名)`。
//! 本文件钉的就是"一次调用"这件事，以及它必须如实交代的代价与状态。
//!
//! ## 判据地图（ADR-0030 末尾登记的五条 + 参数面四条）
//!
//! | # | 判据 | 可被什么注入破坏 |
//! | :--- | :--- | :--- |
//! | 1 | **一次** `tools/call` 出 N 个文件，逐文件 `sha256` 从**盘上字节**读回 == 响应 | 又变成"只出母带"、响应抄内存字节而非盘上文件 |
//! | 2 | RIFF `fmt ` 的 `audioFormat == 3` 且 `bitsPerSample == 32`，`fact` chunk 存在，`bext` **不存在** | 拿 24-bit 整数母带冒充分轨 / 浮点漏 `fact`（坏文件） |
//! | 3 | 同一工程 `solo` / `mute` 前后：**文件名集合与逐文件 sha256 完全不变**（D4） | 照搬 `track_is_audible` 的门 ⇒ 批次随独听键漂移 |
//! | 4 | 第 2 条写失败：`IO_ERROR` + `completedStems` 长度 1 且**仍在盘上**、`failedTrackId` 如实、`orphanedFiles` 实测（D5） | "要么全有要么全无"的假话、静默删掉已写文件 |
//! | 5 | `stemsBasis.renders == N + 1` 且每条分轨 `frames` 相等（D3a 的基线那一次） | 报 `N` 制造免费的错觉 / 各条长度不齐 |
//! | 6 | `dryRun` 不落盘任何文件，预览给全部实测字节 | dryRun 真写盘 |
//! | 7 | `outDir` 缺失/不存在/不是目录 ⇒ `INVALID_PARAMETER_RANGE`（⛔ 不 `create_dir_all`） | 擅自建目录 |
//! | 8 | `trackIds` 筛选子集；陌生身份 ⇒ `TRACK_NOT_FOUND`（⛔ 不静默忽略） | 拼错的 ULID 被吞掉 |
//! | 9 | 文件名确定：同一工程两次导出**同名**；轨名里的 `/`、`..`、全角被清洗成安全字符，无目录穿越 | 直接把轨名当文件名 |
//!
//! ## 临时目录纪律
//!
//! 全部落在 `std::env::temp_dir()/<唯一子目录>`，[`Scratch`] 的 `Drop` 负责恢复权限并删除
//! —— **绝不污染仓库**。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote, RoutingEdge,
    RoutingKind, SampleRate, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_render::rf64::parse_container;

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// 一个临时目录，`Drop` 时恢复权限并删除。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |delta| delta.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-stems-{}-{tag}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    /// 工程文件落点（在 `scratch/project/`，与分轨目录 `scratch/stems/` 分开）。
    fn project_path(&self) -> PathBuf {
        let dir = self.dir.join("project");
        fs::create_dir_all(&dir).expect("建工程目录");
        dir.join("demo.yebanproject")
    }

    /// 分轨输出目录（**必须已存在** ⇒ 判据 7 的另一半）。
    fn stems_dir(&self) -> PathBuf {
        let dir = self.dir.join("stems");
        fs::create_dir_all(&dir).expect("建分轨目录");
        dir
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = fs::set_permissions(&self.dir, fs::Permissions::from_mode(0o755));
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// 确定性 ULID（末 4 位是种子）。
fn id(seed: u32) -> EntityId {
    let text = format!("01J8ZR{:0>16}{seed:04}", 0);
    assert_eq!(text.len(), 26, "ULID 必须 26 字符");
    EntityId::from_str(&text).expect("合法 Crockford Base32 ULID")
}

/// 三条 MIDI 源轨（`Lead` = `id(2)` / `Pad` = `id(3)` / `Bass` = `id(4)`）汇入 Master。
///
/// 每条轨各一个 1 秒（1920 tick @ 120 BPM / 960 PPQ）的音符，音高不同 ⇒
/// **逐轨信号互不相同**：这是判据 3 的前提（若各条字节相同，"solo 不影响批次"
/// 会被"渲染的根本是同一份东西"冒充）。
fn three_track_project() -> YebanProjectV1 {
    let master = id(1);
    let mut project = YebanProjectV1 {
        id: id(999),
        title: "stem fixture".to_owned(),
        bpm: 120.0,
        master_bus_track_id: master,
        ..YebanProjectV1::default()
    };
    project.audio_config.sample_rate = SampleRate::Hz48000;
    project.metadata.created_at_unix_ms = 1_700_000_000_000;
    project.metadata.modified_at_unix_ms = 1_700_000_000_000;
    project.tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    for (index, name) in ["Lead", "Pad", "Bass"].iter().enumerate() {
        let seed = u32::try_from(index).expect("夹具规模放得进 u32");
        let track = id(2 + seed);
        let clip = id(10 + seed);
        let placement = id(20 + seed);
        let note = id(30 + seed);
        let edge = id(40 + seed);
        let mut params = TrackV3 {
            id: track,
            name: (*name).to_owned(),
            kind: TrackKind::Midi,
            ..TrackV3::default()
        };
        params.clips.insert(
            placement,
            ClipPlacement {
                id: placement,
                clip_id: clip,
                start_tick: 0,
                duration_ticks: 1_920,
                loop_config: LoopConfig::default(),
                muted: false,
            },
        );
        let mut notes = BTreeMap::new();
        notes.insert(
            note,
            MidiNote::new(note, 0, 48 + u8::try_from(index * 12).expect("取模"), 1_920),
        );
        project.clip_pool.insert(
            clip,
            ClipPoolEntry {
                id: clip,
                name: format!("{name}Clip"),
                content: ClipContent::Midi { notes },
            },
        );
        project.tracks.insert(track, params);
        project.routing_graph.edges.insert(
            edge,
            RoutingEdge {
                id: edge,
                source_node: track,
                destination_node: master,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        );
    }
    project.routing_graph.nodes = vec![id(2), id(3), id(4), master];
    project
        .validate()
        .unwrap_or_else(|error| panic!("分轨夹具必须合法: {error}"));
    project
}

/// 注入夹具工程的分发器（`open_in_memory` 不碰文件系统）。
fn dispatcher_with(project: &YebanProjectV1, path: &Path) -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(path.to_path_buf(), project.clone(), false)
        .expect("注入夹具工程");
    (dispatcher, auth)
}

/// 走真实 `tools/call` 管线。
fn call(dispatcher: &mut Dispatcher, auth: &str, arguments: Value) -> Value {
    let line = json!({
        "jsonrpc": "2.0",
        "id": "t",
        "method": "tools/call",
        "params": {"name": "yeban_export_stems", "arguments": arguments}
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(auth), &line);
    let response = outcome.response.expect("必须有响应");
    match (response.result, response.error) {
        (Some(result), None) => result,
        (None, Some(error)) => panic!("工具路径上不该有 JSON-RPC 错误: {error:?}"),
        other => panic!("result 与 error 必须恰好一个: {other:?}"),
    }
}

/// 一次**成功**导出的便捷封装：返回响应的 `data`（失败直接把响应拍在断言里）。
fn export(dispatcher: &mut Dispatcher, auth: &str, out_dir: &Path, extra: Value) -> Value {
    let mut arguments = json!({"outDir": out_dir.display().to_string()});
    if let Value::Object(map) = &extra {
        let target = arguments.as_object_mut().expect("arguments 是对象");
        for (key, value) in map {
            target.insert(key.clone(), value.clone());
        }
    }
    let response = call(dispatcher, auth, arguments);
    assert_eq!(response["status"], "success", "导出必须成功: {response}");
    response["data"].clone()
}

/// 磁盘上这份文件的 SHA-256（**从字节读**，不是回抄响应）。
fn file_sha256(path: &Path) -> String {
    let bytes = fs::read(path).expect("读产物");
    yeban_model::AssetHash::of_bytes(&bytes).as_str().to_owned()
}

/// 目录里的全部条目名（排序）。
fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("列目录")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// 响应里 `data.stems` / `data.completedStems` 这类数组的条目数（单位：条）。
fn array_len(value: &Value, key: &str) -> usize {
    value[key]
        .as_array()
        .unwrap_or_else(|| panic!("`{key}` 必须是数组: {value}"))
        .len()
}

/// `fmt ` chunk 里的 `audioFormat`（u16 小端 = `fmt ` 负载的头两个字节）。
fn audio_format_tag(bytes: &[u8]) -> u16 {
    let parsed = parse_container(bytes).expect("容器必须可解析");
    for chunk in &parsed.chunks {
        if chunk.fourcc == *b"fmt " {
            let offset = chunk.payload_offset;
            return u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
        }
    }
    panic!("分轨容器必须有 `fmt ` chunk");
}

// ---------------------------------------------------------------------------
// 判据
// ---------------------------------------------------------------------------

/// 判据 1 + 2 + 5：**一次**调用出 3 个文件，逐文件哈希对得上盘上字节，容器是 IEEE 浮点 32-bit
/// 且带 `fact`、不带 `bext`，成本读数如实报 `N + 1` 次渲染。
#[test]
fn one_call_writes_every_track_as_a_32_bit_float_stem() {
    let scratch = Scratch::new("one-call");
    let out_dir = scratch.stems_dir();
    let (mut dispatcher, auth) = dispatcher_with(&three_track_project(), &scratch.project_path());

    let data = export(&mut dispatcher, &auth, &out_dir, Value::Null);

    // 一次调用 ⇒ 目录里恰好 3 个 wav（母带都不写）。
    let names = entries(&out_dir);
    assert_eq!(
        names,
        vec!["0001.Lead.wav", "0002.Pad.wav", "0003.Bass.wav"],
        "一次调用必须出全部三条分轨"
    );
    let stems = data["stems"].as_array().expect("stems 是数组");
    assert_eq!(stems.len(), 3);
    assert_eq!(data["partial"], json!(false));
    assert_eq!(data["auditionStateIgnored"], json!(true));
    assert_eq!(data["stemCount"], json!(3));

    // 逐文件 sha256 **从盘上读回**（响应抄内存字节而没写对文件，这里就红）。
    let mut bytes_total = 0_usize;
    for row in stems {
        let path = PathBuf::from(row["path"].as_str().expect("path 是字符串"));
        assert_eq!(
            file_sha256(&path),
            row["sha256"].as_str().expect("sha256 是字符串"),
            "回执的 sha256 必须等于盘上字节: {}",
            path.display()
        );
        let size = fs::metadata(&path).expect("stat").len() as usize;
        assert_eq!(
            size,
            usize::try_from(row["bytes"].as_u64().expect("bytes")).expect("usize"),
            "bytes 必须是磁盘上这份文件的真实字节数"
        );
        bytes_total += size;
        assert_eq!(row["bitDepth"], json!(32), "分轨位深是 32 [ARCH-DSP-003]");

        let bytes = fs::read(&path).expect("读产物");
        let parsed = parse_container(&bytes).expect("分轨必须是可解析的 RIFF/WAVE");
        assert!(parsed.format.is_float, "分轨容器必须是 IEEE 浮点");
        assert_eq!(parsed.format.bits_per_sample, 32);
        assert_eq!(parsed.format.channels, 2);
        assert_eq!(
            audio_format_tag(&bytes),
            3,
            "audioFormat 必须是 WAVE_FORMAT_IEEE_FLOAT"
        );
        // ⚠ 非 PCM 容器**必须**带 `fact`（`rf64.rs` 明确写着的容器规则）：漏了就是坏文件。
        assert!(
            parsed.chunks.iter().any(|chunk| chunk.fourcc == *b"fact"),
            "浮点容器缺 `fact` chunk"
        );
        // 分轨是素材，不是母带档案 ⇒ 不写 BWF 扩展块。
        assert!(parsed.bext.is_none(), "分轨不该带 bext");
    }
    assert_eq!(
        usize::try_from(data["bytesTotal"].as_u64().expect("bytesTotal")).expect("usize"),
        bytes_total,
        "bytesTotal 必须等于逐文件字节合计（单位：字节）"
    );

    // 成本读数：N + 1 次渲染（D3a），且每条分轨的帧数**齐平**（批次对齐的唯一含义）。
    assert_eq!(
        data["stemsBasis"]["renders"],
        json!(4),
        "3 条分轨 + 1 次基线渲染"
    );
    assert_eq!(data["stemsBasis"]["basisRender"], json!("projectWide"));
    let frames: Vec<u64> = stems
        .iter()
        .map(|row| row["frames"].as_u64().expect("frames"))
        .collect();
    assert!(
        frames.iter().all(|value| *value == frames[0]),
        "批次帧数必须齐平: {frames:?}"
    );
    assert_eq!(
        data["stemsBasis"]["audioFramesPerRender"].as_u64(),
        Some(frames[0]),
        "stemsBasis 报的帧数就是每条分轨实际用的那一个数"
    );
    assert_eq!(
        data["stemsBasis"]["sampleRate"].as_u64(),
        Some(48_000),
        "分轨采样率 = 工程采样率"
    );
    // 批次共用同一个 `creationTimestamp` ⇒ 同一批的分轨逐字节可互认（除轨道内容外）。
    assert_eq!(array_len(&data, "stems"), 3);
}

/// 判据 3：`solo` / `mute` **不影响批次**（D4）—— 文件名集合与逐文件 sha256 完全不变。
///
/// 这是本工具最容易被做坏的一条：照搬 `track_is_audible` 的门，批次内容就会随
/// 用户上一次按了哪个独听键而漂移，而分轨是给下游用的素材、不是试听快照。
#[test]
fn audition_state_does_not_change_the_batch() {
    let clean = Scratch::new("audition-clean");
    let soloed = Scratch::new("audition-solo");
    let clean_dir = clean.stems_dir();
    let solo_dir = soloed.stems_dir();

    let (mut dispatcher, auth) = dispatcher_with(&three_track_project(), &clean.project_path());
    let before = export(&mut dispatcher, &auth, &clean_dir, Value::Null);

    // 同一份工程，但把 `Pad` 静音 + 给 `Bass` 独听。
    let mut project = three_track_project();
    project.tracks.get_mut(&id(3)).expect("Pad 在夹具里").mute = true;
    project.tracks.get_mut(&id(4)).expect("Bass 在夹具里").solo = true;
    let (mut dispatcher, auth) = dispatcher_with(&project, &soloed.project_path());
    let after = export(&mut dispatcher, &auth, &solo_dir, Value::Null);

    assert_eq!(
        entries(&clean_dir),
        entries(&solo_dir),
        "批次文件集合不得随独听键漂移"
    );
    let before_rows = before["stems"].as_array().expect("数组");
    let after_rows = after["stems"].as_array().expect("数组");
    assert_eq!(before_rows.len(), after_rows.len());
    for (left, right) in before_rows.iter().zip(after_rows) {
        assert_eq!(left["fileName"], right["fileName"]);
        assert_eq!(
            left["sha256"].as_str(),
            right["sha256"].as_str(),
            "同一条轨的分轨字节必须与试听状态无关: {}",
            left["fileName"]
        );
        assert_eq!(
            left["bytes"], right["bytes"],
            "批次字节数不得随 solo 前后变化"
        );
    }
    // 被静音的那条**照样出文件**，且如实回报它本来在工程里是静音的。
    let pad = after_rows
        .iter()
        .find(|row| row["name"] == json!("Pad"))
        .expect("Pad 必须有分轨");
    assert_eq!(pad["sourceMute"], json!(true), "sourceMute 要如实回读工程");
    assert!(
        pad["bytes"].as_u64().expect("bytes") > 0,
        "静音的轨也要有素材字节"
    );
    assert!(
        pad["sha256"].as_str() == before_rows[1]["sha256"].as_str(),
        "静音不得改变这一条的字节"
    );
    let bass = after_rows
        .iter()
        .find(|row| row["name"] == json!("Bass"))
        .expect("Bass 必须有分轨");
    assert_eq!(bass["sourceSolo"], json!(true));
    assert_eq!(after["auditionStateIgnored"], json!(true));
}

/// 判据 4：第 2 条落盘失败 ⇒ `IO_ERROR` + 如实的部分成功读数（D5）。
///
/// 注入方式：把中间那条轨命名成一个**超出文件名长度上限**的名字（300 个 `A`）。
/// 清洗只换字符、不截断 ⇒ 同目录临时文件的名字在 `create_new` 那一步就撞
/// `ENAMETOOLONG`，失败点**恰好**落在第二条的写盘上，而第 1 条已经原子落盘。
/// ⛔ 这不是"编一个错误码"，是一条真的写失败。
#[test]
fn the_second_stem_failing_reports_what_is_already_on_disk() {
    let scratch = Scratch::new("partial");
    let out_dir = scratch.stems_dir();
    let mut project = three_track_project();
    project.tracks.get_mut(&id(3)).expect("Pad 在夹具里").name = "A".repeat(300);

    let (mut dispatcher, auth) = dispatcher_with(&project, &scratch.project_path());
    let response = call(
        &mut dispatcher,
        &auth,
        json!({"outDir": out_dir.display().to_string()}),
    );
    assert_eq!(response["status"], "error", "{response}");
    assert_eq!(
        response["error"]["code"],
        json!("IO_ERROR"),
        "部分成功必须落在闭合联集内的码"
    );
    // ⛔ 不发明错误码：靠 `data.errorKind` 区分子类。
    let data = &response["error"]["data"];
    assert_eq!(data["errorKind"], json!("stemWriteFailed"));
    assert_eq!(data["failedTrackId"], json!(id(3).to_canonical_string()));
    assert_eq!(
        array_len(data, "completedStems"),
        1,
        "第 2 条失败 ⇒ 恰好 1 条已落盘"
    );
    assert_eq!(data["completedStems"][0]["name"], json!("Lead"));
    // 已写的那条**保留不删**（删用户盘上的文件比留一个如实的清单危险）。
    assert_eq!(
        entries(&out_dir),
        vec!["0001.Lead.wav".to_owned()],
        "只许留下已成功的第 1 条, 不许有临时残留"
    );
    assert_eq!(
        file_sha256(&out_dir.join("0001.Lead.wav")),
        data["completedStems"][0]["sha256"]
            .as_str()
            .expect("sha256"),
        "保留下来的文件要能按回执校验回读"
    );
    // `orphanedFiles` 是**实测**：rename 从未成功 ⇒ 第 2 条没有留下文件，因此为空。
    assert_eq!(
        array_len(data, "orphanedFiles"),
        0,
        "没有残留就不许虚报: {data}"
    );
    // `resumeFrom` 给出"下一次只补剩下的"。
    let resume = &data["resumeFrom"];
    assert_eq!(
        resume["outDir"].as_str(),
        Some(out_dir.display().to_string().as_str())
    );
    let remaining = resume["trackIds"].as_array().expect("数组");
    assert_eq!(
        remaining.len(),
        2,
        "剩下 Pad + Bass 两条要能被点名补跑: {resume}"
    );
    assert_eq!(resume["trackIds"][0], json!(id(3).to_canonical_string()));
    assert_eq!(
        data["auditionStateIgnored"],
        json!(true),
        "部分失败的回执也必须说清批次口径"
    );
    assert_eq!(data["partial"], json!(true));
    assert_eq!(
        data["stemsBasis"]["renders"],
        json!(4),
        "失败的批次的成本读数照样如实"
    );
}

/// 判据 6：`dryRun` 一个字节都不写盘，预览里的字节数与真跑一致。
#[test]
fn dry_run_previews_measured_bytes_without_writing_any_file() {
    let scratch = Scratch::new("dry");
    let out_dir = scratch.stems_dir();
    let (mut dispatcher, auth) = dispatcher_with(&three_track_project(), &scratch.project_path());

    let preview = call(
        &mut dispatcher,
        &auth,
        json!({"outDir": out_dir.display().to_string(), "dryRun": true}),
    );
    assert_eq!(preview["status"], "success", "{preview}");
    assert_eq!(
        entries(&out_dir),
        Vec::<String>::new(),
        "dryRun 不得写任何文件"
    );
    assert_eq!(preview["data"]["willWrite"], json!(true));
    assert_eq!(preview["data"]["written"], json!(false));
    assert_eq!(array_len(&preview["data"], "stems"), 3);
    assert_eq!(preview["data"]["stemsBasis"]["renders"], json!(4));

    // 预览的字节必须是**实测**：真跑之后逐文件哈希与预览一致。
    let rows = preview["data"]["stems"].as_array().expect("数组");
    export(&mut dispatcher, &auth, &out_dir, Value::Null);
    for row in rows {
        let path = out_dir.join(row["fileName"].as_str().expect("fileName"));
        assert_eq!(
            Some(file_sha256(&path)),
            row["sha256"].as_str(),
            "预览的 sha256 必须就是真落盘的那份字节: {}",
            path.display()
        );
    }
}

/// 判据 7：`outDir` 缺失 / 不存在 / 是文件 ⇒ `INVALID_PARAMETER_RANGE`，且**不**建目录。
#[test]
fn out_dir_must_be_an_existing_directory_and_is_never_created() {
    let scratch = Scratch::new("outdir");
    let a_file = scratch.dir.join("a-file");
    fs::write(&a_file, b"x").expect("写一个文件");
    let (mut dispatcher, auth) = dispatcher_with(&three_track_project(), &scratch.project_path());

    // 缺失。
    let response = call(&mut dispatcher, &auth, json!({}));
    assert_eq!(response["status"], "error", "{response}");
    assert_eq!(
        response["error"]["code"],
        json!("INVALID_PARAMETER_RANGE"),
        "缺 outDir 是参数问题, 不是领域失败"
    );

    // 不存在 ⇒ ⛔ 不 `create_dir_all`。
    let would_be = scratch.dir.join("deep").join("nested");
    let response = call(
        &mut dispatcher,
        &auth,
        json!({"outDir": would_be.display().to_string()}),
    );
    assert_eq!(response["status"], "error", "{response}");
    assert_eq!(response["error"]["code"], json!("INVALID_PARAMETER_RANGE"));
    assert!(!would_be.exists(), "分轨导出不得替调用方创建目录");

    // 是文件而不是目录。
    let response = call(
        &mut dispatcher,
        &auth,
        json!({"outDir": a_file.display().to_string()}),
    );
    assert_eq!(response["status"], "error", "{response}");
    assert_eq!(response["error"]["code"], json!("INVALID_PARAMETER_RANGE"));
}

/// 判据 8：`trackIds` 只导出点名的子集；陌生身份 ⇒ `TRACK_NOT_FOUND`（⛔ 不静默忽略）。
#[test]
fn track_ids_filters_the_batch_and_unknown_ids_fail_loudly() {
    let scratch = Scratch::new("filter");
    let out_dir = scratch.stems_dir();
    let (mut dispatcher, auth) = dispatcher_with(&three_track_project(), &scratch.project_path());

    let data = export(
        &mut dispatcher,
        &auth,
        &out_dir,
        json!({"trackIds": [id(4).to_canonical_string()]}),
    );
    assert_eq!(
        entries(&out_dir),
        vec!["0003.Bass.wav"],
        "筛选只改**导出哪些**, 不改序号与命名（与全量导出同名才可互认）"
    );
    assert_eq!(array_len(&data, "stems"), 1);
    // 1 条分轨 + 1 次基线 = 2 次渲染。
    assert_eq!(data["stemsBasis"]["renders"], json!(2));

    let response = call(
        &mut dispatcher,
        &auth,
        json!({"outDir": out_dir.display().to_string(), "trackIds": [id(77).to_canonical_string()]}),
    );
    assert_eq!(response["status"], "error", "{response}");
    assert_eq!(response["error"]["code"], json!("TRACK_NOT_FOUND"));
    assert_eq!(
        array_len(&response["error"]["data"], "unknownTrackIds"),
        1,
        "陌生身份要点名回来: {}",
        response["error"]["data"]
    );
    assert_eq!(
        entries(&out_dir),
        vec!["0003.Bass.wav"],
        "被拒绝的调用不得改变已有产物"
    );
}

/// 判据 9：文件名确定且不越出目录（清洗规则 D7）。
#[test]
fn file_names_are_deterministic_and_carry_no_path_from_the_track_name() {
    let scratch = Scratch::new("names");
    let out_dir = scratch.stems_dir();
    let mut project = three_track_project();
    project.tracks.get_mut(&id(2)).expect("Lead").name = "../../etc/passwd".to_owned();
    project.tracks.get_mut(&id(3)).expect("Pad").name = "鼓 组 / 主".to_owned();
    project.tracks.get_mut(&id(4)).expect("Bass").name = "...".to_owned();

    let (mut dispatcher, auth) = dispatcher_with(&project, &scratch.project_path());
    let data = export(&mut dispatcher, &auth, &out_dir, Value::Null);
    let names = entries(&out_dir);
    assert_eq!(names.len(), 3, "三条都要出文件: {names:?}");
    // `/` 全被换成 `_` ⇒ 名字里不存在任何目录分隔；首尾的点号/下划线被剥掉。
    assert_eq!(
        data["stems"][0]["fileName"],
        json!("0001.etc_passwd.wav"),
        "相对路径前缀必须被清洗干净"
    );
    assert_eq!(data["stems"][1]["fileName"], json!("0002.track.wav"));
    assert_eq!(
        data["stems"][2]["fileName"],
        json!("0003.track.wav"),
        "只剩点号的名字确定回退成 `track`（序号消歧, ⛔ 不做隐式 ` (1)`）"
    );
    assert!(
        !scratch.dir.join("etc").exists() && !Path::new("/etc/passwd.wav").exists(),
        "分轨不得写到 outDir 之外"
    );
    let reported = usize::try_from(data["maxFileNameBytes"].as_u64().expect("maxFileNameBytes"))
        .expect("usize");
    assert!(
        reported
            >= names
                .iter()
                .map(|name| name.as_bytes().len())
                .max()
                .expect("有名字"),
        "maxFileNameBytes 必须覆盖最长的文件名"
    );

    // 同一工程再导一次（换一个目录）⇒ 逐字节同名同哈希。
    let again = scratch.dir.join("again");
    fs::create_dir_all(&again).expect("建第二个目录");
    let second = export(&mut dispatcher, &auth, &again, Value::Null);
    assert_eq!(entries(&again), names, "同一工程两次导出必须同名");
    for (left, right) in data["stems"]
        .as_array()
        .expect("数组")
        .iter()
        .zip(second["stems"].as_array().expect("数组"))
    {
        assert_eq!(left["fileName"], right["fileName"]);
        assert_eq!(left["sha256"], right["sha256"], "分轨必须确定性可重现");
    }
}

/// 主总线与**非源节点**的轨：⛔ 不静默丢，在 `skippedTracks` 里点名（D4）。
#[test]
fn master_and_bus_tracks_are_named_in_skips_instead_of_vanishing() {
    let scratch = Scratch::new("bus");
    let out_dir = scratch.stems_dir();
    let (mut dispatcher, auth) = dispatcher_with(&three_track_project(), &scratch.project_path());

    let data = export(&mut dispatcher, &auth, &out_dir, Value::Null);
    let skipped = data["skippedTracks"].as_array().expect("数组");
    assert_eq!(
        skipped.len(),
        1,
        "夹具里只有主总线一条不出文件: {skipped:?}"
    );
    assert_eq!(skipped[0]["trackId"], json!(id(1).to_canonical_string()));
    assert_eq!(skipped[0]["name"], json!("Master"));
    assert_eq!(skipped[0]["reason"], json!("masterBusTrack"));
    assert_eq!(
        data["stemCount"],
        json!(3),
        "点名不等于丢轨：出文件的还是 3 条"
    );
}

/// 没有活跃工程 ⇒ `NO_ACTIVE_PROJECT`（与母带同一条前提闸门）。
#[test]
fn export_without_an_active_project_fails_as_no_active_project() {
    let scratch = Scratch::new("no-project");
    let out_dir = scratch.stems_dir();
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    let response = call(
        &mut dispatcher,
        &auth,
        json!({"outDir": out_dir.display().to_string()}),
    );
    assert_eq!(response["status"], "error", "{response}");
    assert_eq!(response["error"]["code"], json!("NO_ACTIVE_PROJECT"));
    assert_eq!(entries(&out_dir), Vec::<String>::new());
}
