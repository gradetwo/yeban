//! 判据：**SFZ 音名八度覆盖 (c) 的 CLI 可达性 + 记法结论可见性 + 未命中告警**。
//!
//! ## 这条判据要证的句子（问题文件 drum-octave-issue-2）
//!
//! > SSO 式采样库（键位用 Yamaha 音名拼写 `key=c3` 意为 MIDI 60、采样全部
//! > 没有 WAV `smpl` 嵌入根音）挂上工程后：
//! > ① 无覆盖时按作者键位 60/61/62 触发**全静音**，且报告必须**响亮**——
//! >    零证据独立结论（`reason=no_evidence`）+ 未命中音符统计，两样都出；
//! > ② 经 `setTrackSampler` 的对象形写入八度覆盖 +12 后出声，且与「数字
//! >    键位副本」的渲染**逐样本一致**；
//! > ③ 合成顺序 = (a) 文件声明恒叠加、外部层由 (c) 充当（(c) 在时 (b) 停跑）；
//! > ④ 覆盖随工程持久化（serde 往返后仍在且仍生效）。
//!
//! ## 夹具口径
//!
//! 判据夹具按报告实测形状合成：3 个 group 分别 `key=c3/c#3/d3` +
//! `loop_mode=one_shot`、每组一条 region、采样为**无 `smpl` 块**的普通
//! WAV（`write_plain_wav` 只写 fmt+data）。真实文件验收（原版 vs fixed
//! 副本 + Timpani）在盒上单独跑、数字入回执，不在本文件。
//!
//! ## 怎么变红
//!
//! | 改哪里 | 哪一条红 |
//! | :--- | :--- |
//! | 解析/编译任一处再把覆盖写死 `None` | `override_plus_twelve_matches_the_numeric_copy_bit_for_bit`（静音）＋ mcp 单元测试 |
//! | 零证据结论被并回静默多数 | `yamaha_named_keys_without_evidence_are_silent_and_loudly_reported` |
//! | 未命中统计删除/口径漂移 | 同上（`unmatched_note_count`） |
//! | (a) 不再叠加或 (c) 改成替换总移位 | `declared_shift_always_adds_and_override_forms_the_external_layer` |

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use yeban_app::export_wav::WavExportOptions;
use yeban_app::render_core::render_project_f32_with_assets;
use yeban_model::project::ClipContent;
use yeban_model::{EntityId, MidiNote, SamplerBinding, YebanProjectV1};
use yeban_render::dither::{BitDepth, PcmBuffer};
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;
use yeban_sfz::NotationMissReason;

/// 临时目录（形态照 `sfz_model_chain.rs` 的 `scratch`）。
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-sfz-octave-{tag}-{}-{}",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 32f 位深让渲染读数不经量化（与 `sfz_model_chain.rs` 同口径）。
fn float_options() -> WavExportOptions {
    WavExportOptions {
        bit_depth: BitDepth::Float32,
        ..WavExportOptions::default()
    }
}

/// 写一条**常量** PCM 样本（24-bit 单声道、4800 帧；**无 `smpl` 块** ——
/// 这正是 SSO 形态的关键：嵌入根音证据为空）。
fn write_const_sample(path: &Path, value: f32) {
    let scale = (8_388_608.0_f32 * value) as i32;
    let buffer = PcmBuffer::Int24(vec![scale; 4_800]);
    let format = PcmFormat::integer(1, 48_000, 24);
    write_plain_wav(path, &format, &buffer).expect("写样本夹具");
}

/// SSO 形状夹具：三组音名键位 + one_shot；样本互异常量（逐位比对可分）。
fn write_yamaha_fixture(dir: &Path) -> PathBuf {
    write_const_sample(&dir.join("kick.wav"), 0.2);
    write_const_sample(&dir.join("snare.wav"), 0.4);
    write_const_sample(&dir.join("roll.wav"), 0.6);
    let path = dir.join("yamaha.sfz");
    std::fs::write(
        &path,
        "<group> key=c3 loop_mode=one_shot\n\
         <region> sample=kick.wav\n\
         <group> key=c#3 loop_mode=one_shot\n\
         <region> sample=snare.wav\n\
         <group> key=d3 loop_mode=one_shot\n\
         <region> sample=roll.wav\n",
    )
    .expect("写 SFZ 夹具");
    path
}

/// 数字键位副本：同一批样本、键位直接写 60/61/62（报告人的 fixed 副本形态）。
fn write_numeric_fixture(dir: &Path) -> PathBuf {
    let path = dir.join("numeric.sfz");
    std::fs::write(
        &path,
        "<group> key=60 loop_mode=one_shot\n\
         <region> sample=kick.wav\n\
         <group> key=61 loop_mode=one_shot\n\
         <region> sample=snare.wav\n\
         <group> key=62 loop_mode=one_shot\n\
         <region> sample=roll.wav\n",
    )
    .expect("写 SFZ 夹具");
    path
}

/// 带 (a) 声明的音名夹具：`<control> octave_offset=-1`（(a) = −12）。
/// 样本必须自带一份（本夹具用独立目录，⛔ 不能指望别的夹具写过）。
fn write_declared_fixture(dir: &Path) -> PathBuf {
    write_const_sample(&dir.join("kick.wav"), 0.2);
    write_const_sample(&dir.join("snare.wav"), 0.4);
    write_const_sample(&dir.join("roll.wav"), 0.6);
    let path = dir.join("declared.sfz");
    std::fs::write(
        &path,
        "<control> octave_offset=-1\n\
         <group> key=c3 loop_mode=one_shot\n\
         <region> sample=kick.wav\n\
         <group> key=c#3 loop_mode=one_shot\n\
         <region> sample=snare.wav\n\
         <group> key=d3 loop_mode=one_shot\n\
         <region> sample=roll.wav\n",
    )
    .expect("写 SFZ 夹具");
    path
}

/// `filled_project` 里带 MIDI 音符的那条非主总线轨（同 `sfz_model_chain.rs`）。
fn midi_track_of(project: &YebanProjectV1) -> EntityId {
    *project
        .tracks
        .iter()
        .find(|(id, track)| {
            **id != project.master_bus_track_id
                && track.clips.values().any(|placement| {
                    project
                        .clip_pool
                        .get(&placement.clip_id)
                        .is_some_and(|entry| entry.content.notes().is_some())
                })
        })
        .map(|(id, _)| id)
        .expect("filled_project 必须有一条带 MIDI 音符的非主总线轨")
}

/// 搭工程：那条 MIDI 轨的片段音符换成 `pitches`（力度 100、每音 480 tick、
/// 依次错开），绑定 `sfz_path`，覆盖 = `override_step`。
///
/// 隔离处理（让静音/计数断言精确）：目标轨只留**第一条摆放**（未命中统计
/// 按摆放计次，多摆放会成倍），其余非主总线轨全部静音（混音里只剩目标轨
/// 的声音，`peak == 0` 才是「这条轨全静音」的真断言）。
fn project_with_notes(
    sfz_path: &Path,
    pitches: &[u8],
    override_step: Option<i32>,
) -> YebanProjectV1 {
    let mut project = yeban_model::samples::filled_project();
    let track_id = midi_track_of(&project);
    let master_id = project.master_bus_track_id;
    for (id, track) in project.tracks.iter_mut() {
        if *id != track_id && *id != master_id {
            track.mute = true;
        }
    }
    let track = project.tracks.get_mut(&track_id).expect("轨必须在");
    let first_placement = track.clips.keys().next().copied().expect("轨必须有摆放");
    track.clips.retain(|id, _| *id == first_placement);
    let clip_id = track
        .clips
        .values()
        .next()
        .expect("保留的摆放必须在")
        .clip_id;
    track.sampler = Some(SamplerBinding {
        note_octave_override: override_step,
        path: sfz_path.to_string_lossy().into_owned(),
    });
    let mut notes: BTreeMap<EntityId, MidiNote> = BTreeMap::new();
    for (index, pitch) in pitches.iter().enumerate() {
        let id = EntityId::new();
        let mut note = MidiNote::new(id, (index as u64) * 480, *pitch, 480);
        note.velocity = 100;
        notes.insert(id, note);
    }
    project
        .clip_pool
        .get_mut(&clip_id)
        .expect("片段必须在池里")
        .content = ClipContent::Midi { notes };
    project
}

/// 峰值（绝对值最大；单位：线性幅度）。
fn peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .fold(0.0_f32, |acc, sample| acc.max(sample.abs()))
}

/// 逐位指纹的载体（`f32` 位模式全表）。
fn bits(samples: &[f32]) -> Vec<u32> {
    samples.iter().map(|sample| sample.to_bits()).collect()
}

/// ⭐ 判据一（问题本体）：无覆盖时 60/61/62 **全静音**，且两条响亮结论都在：
/// 零证据独立结论 + 未命中统计。在此之前 CLI/报告对这个形态**一个字都没有**。
#[test]
fn yamaha_named_keys_without_evidence_are_silent_and_loudly_reported() {
    let dir = scratch("sso-silent");
    let sfz = write_yamaha_fixture(&dir);
    let project = project_with_notes(&sfz, &[60, 61, 62], None);

    let out =
        render_project_f32_with_assets(&project, float_options(), None, &[]).expect("渲染必须成功");
    assert_eq!(peak(&out.samples), 0.0, "按作者键位触发必须全静音（复现）");
    assert_eq!(out.sfz_decoded.len(), 1, "恰一份解码报告");
    let report = &out.sfz_decoded[0].1;

    // ① 零证据是**独立结论**，不是 TooFewEvidence。
    let detection = report.notation.as_ref().expect("检测结论必须在报告里");
    assert_eq!(
        detection.miss_reason,
        Some(NotationMissReason::NoEvidence),
        "音名键位 + 零证据必须是独立的 NoEvidence 结论"
    );
    assert_eq!(detection.evidence_count, 0);
    assert_eq!(detection.named_key_region_count, 3);
    let lines = report.notation_lines();
    assert!(
        lines
            .iter()
            .any(|line| line.contains("reason=no_evidence") && line.contains("evidence=0/3")),
        "零证据响亮行必须透出（含机器可读字段）: {lines:?}"
    );

    // ② 未命中统计：3 个音符全打空、音高逐个点名。
    assert_eq!(report.unmatched_note_count, 3, "三个音符全部未命中");
    assert_eq!(report.unmatched_note_pitches, vec![60, 61, 62]);
    let line = report
        .unmatched_notes_line()
        .expect("有未命中就必须有告警行");
    assert!(line.contains("notes=3"), "告警行必须带计数: {line}");
}

/// ⭐ 判据二（修复本体）：覆盖 +12 后出声，且与数字键位副本**逐样本一致**。
#[test]
fn override_plus_twelve_matches_the_numeric_copy_bit_for_bit() {
    let dir = scratch("sso-fixed");
    let yamaha = write_yamaha_fixture(&dir);
    let numeric = write_numeric_fixture(&dir);

    let with_override = project_with_notes(&yamaha, &[60, 61, 62], Some(12));
    let auto = render_project_f32_with_assets(&with_override, float_options(), None, &[])
        .expect("带覆盖渲染必须成功");
    assert!(peak(&auto.samples) > 0.0, "设覆盖后必须出声");
    let report = &auto.sfz_decoded[0].1;
    assert_eq!(report.unmatched_note_count, 0, "覆盖后不许再有未命中");
    assert_eq!(
        report.notation_source,
        yeban_engine::sfz_instrument::NotationShiftSource::ManualOverride,
        "外部移位来源必须是工程覆盖 (c)"
    );
    assert!(
        report
            .notation_lines()
            .iter()
            .any(|line| line.contains("手动八度覆盖") && line.contains("+12")),
        "覆盖生效必须响亮报告: {:?}",
        report.notation_lines()
    );

    let plain = project_with_notes(&numeric, &[60, 61, 62], None);
    let reference = render_project_f32_with_assets(&plain, float_options(), None, &[])
        .expect("数字副本渲染必须成功");
    assert!(
        peak(&reference.samples) > 0.0,
        "数字副本必须出声（夹具前提）"
    );
    assert_eq!(
        bits(&auto.samples),
        bits(&reference.samples),
        "覆盖后的渲染必须与数字键位副本逐样本一致"
    );
}

/// ⭐ 判据三（合成顺序）：(a) 恒叠加、外部层 = (c)。
/// (a) = −12 时键位落 36；无覆盖按 48 写音符全打空；加覆盖 +12 后总移位
/// 回到 0、48 出声 —— 证明 (c) 没有把 (a) 顶掉、(a) 也没有被 (c) 吞掉。
#[test]
fn declared_shift_always_adds_and_override_forms_the_external_layer() {
    let dir = scratch("compose");
    let declared = write_declared_fixture(&dir);

    // (a) 单独生效：键位在 36/37/38。
    let at36 = project_with_notes(&declared, &[36, 37, 38], None);
    let out = render_project_f32_with_assets(&at36, float_options(), None, &[])
        .expect("(a) 单独渲染必须成功");
    assert!(peak(&out.samples) > 0.0, "(a) = −12 后 36 必须出声");
    assert_eq!(out.sfz_decoded[0].1.unmatched_note_count, 0);
    assert!(
        out.sfz_decoded[0]
            .1
            .notation_lines()
            .iter()
            .any(|line| line.contains("文件自带移位")),
        "(a) 生效必须响亮报告: {:?}",
        out.sfz_decoded[0].1.notation_lines()
    );

    // (a) 在、按 IPN 原位 48 写 ⇒ 全打空。
    let at48 = project_with_notes(&declared, &[48, 49, 50], None);
    let out =
        render_project_f32_with_assets(&at48, float_options(), None, &[]).expect("渲染必须成功");
    assert_eq!(peak(&out.samples), 0.0);
    assert_eq!(out.sfz_decoded[0].1.unmatched_note_count, 3);

    // (a) + (c)：总移位 = −12 + 12 = 0 ⇒ 48 出声。
    let composed = project_with_notes(&declared, &[48, 49, 50], Some(12));
    let out = render_project_f32_with_assets(&composed, float_options(), None, &[])
        .expect("(a)+(c) 渲染必须成功");
    assert!(peak(&out.samples) > 0.0, "(a) 与 (c) 相加后 48 必须出声");
    assert_eq!(out.sfz_decoded[0].1.unmatched_note_count, 0);
}

/// ⭐ 判据四（持久化）：覆盖写进工程 → serde 往返 → 仍在且仍生效。
#[test]
fn the_override_survives_a_project_serde_round_trip() {
    let dir = scratch("persist");
    let yamaha = write_yamaha_fixture(&dir);
    let project = project_with_notes(&yamaha, &[60, 61, 62], Some(12));

    let text = serde_json::to_string(&project).expect("工程必须可序列化");
    let reloaded: YebanProjectV1 = serde_json::from_str(&text).expect("工程必须可回读");
    let track_id = midi_track_of(&reloaded);
    let binding = reloaded
        .tracks
        .get(&track_id)
        .and_then(|track| track.sampler.as_ref())
        .expect("回读后绑定必须还在");
    assert_eq!(binding.note_octave_override, Some(12), "覆盖必须随工程落盘");

    let out = render_project_f32_with_assets(&reloaded, float_options(), None, &[])
        .expect("回读工程渲染必须成功");
    assert!(peak(&out.samples) > 0.0, "回读后覆盖必须仍生效");
    assert_eq!(out.sfz_decoded[0].1.unmatched_note_count, 0);
}

/// ⭐ 判据五（CLI 可见性，`--sfz` 路）：装载报告行里必须直接出现零证据结论。
#[test]
fn the_cli_sfz_load_lines_carry_the_no_evidence_conclusion() {
    let dir = scratch("cli-sfz");
    let sfz = write_yamaha_fixture(&dir);
    let out = dir.join("probe.wav");
    let argv: Vec<String> = vec![
        "--sfz".to_owned(),
        sfz.display().to_string(),
        "--export-wav".to_owned(),
        out.display().to_string(),
        // T-FIX-360: 保留旧产物口径，显式 8 秒。
        "--wav-seconds".to_owned(),
        "8".to_owned(),
    ];
    let options = yeban_app::cli::parse(&argv).expect("命令行必须解析");
    let lines = yeban_app::cli::run_batch(&options).expect("批处理必须成功");
    assert!(
        lines.iter().any(|line| line.contains("reason=no_evidence")),
        "--sfz 的可见输出必须带零证据结论: {lines:?}"
    );
}

/// ⭐ 判据六（CLI 全链，`--apply-ops` 对象形 ⇒ 保存 ⇒ 导出）：覆盖经 ops
/// 入口写进工程、落盘、导出时响亮报告来源 = 工程覆盖。
#[cfg(feature = "in-process-mcp")]
#[test]
fn apply_ops_object_form_sets_the_override_end_to_end() {
    /// demo 工程第一条轨（鼓）的 ULID（与 `tests/apply_ops.rs` 同一常数）。
    const T1: &str = "01J8Z5Q0R7K3M9X2V4B6N8P0T1";
    let dir = scratch("cli-ops");
    let sfz = write_yamaha_fixture(&dir);
    let ops_path = dir.join("ops.json");
    std::fs::write(
        &ops_path,
        format!(
            "{{\"trackId\":\"{T1}\",\"ops\":[{{\"kind\":\"setTrackSampler\",\"trackId\":\"{T1}\",\"sampler\":{{\"path\":{},\"noteOctaveOverride\":12}}}}]}}",
            serde_json::to_string(&sfz.display().to_string()).expect("路径序列化")
        ),
    )
    .expect("写 ops 夹具");
    let saved = dir.join("with-override.yeban");
    let run = |argv: Vec<String>| {
        let options = yeban_app::cli::parse(&argv).expect("命令行必须解析");
        yeban_app::cli::run_batch(&options).expect("批处理必须成功")
    };
    let lines = run(vec![
        "--project-sample".to_owned(),
        "default".to_owned(),
        "--apply-ops".to_owned(),
        ops_path.display().to_string(),
        "--save-as".to_owned(),
        saved.display().to_string(),
    ]);
    assert!(
        lines.iter().any(|line| line.starts_with("applied-ops:")),
        "ops 必须被应用: {lines:?}"
    );

    // 回读保存的工程：绑定必须带覆盖（持久化在 CLI 链上同样成立）。
    use std::str::FromStr as _;
    let argv = vec![
        "--open".to_owned(),
        saved.display().to_string(),
        "--headless".to_owned(),
    ];
    let options = yeban_app::cli::parse(&argv).expect("解析回读命令");
    let loaded = yeban_app::cli::load_project(&options).expect("回读保存的工程");
    let track_id = EntityId::from_str(T1).expect("T1 是合法 ULID");
    let binding = loaded
        .archive
        .project
        .track(&track_id)
        .expect("T1 在工程里")
        .sampler
        .as_ref()
        .expect("回读后绑定必须在");
    assert_eq!(binding.note_octave_override, Some(12));

    // 导出：解码报告的记法行必须点名工程覆盖。
    let target = dir.join("out.wav");
    let lines = run(vec![
        "--open".to_owned(),
        saved.display().to_string(),
        "--export-wav".to_owned(),
        target.display().to_string(),
        // T-FIX-360: 同上，显式 8 秒。
        "--wav-seconds".to_owned(),
        "8".to_owned(),
    ]);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("手动八度覆盖") && line.contains("+12")),
        "导出可见输出必须报告覆盖生效: {lines:?}"
    );
    assert!(
        !lines.iter().any(|line| line.contains("reason=no_evidence")),
        "覆盖在时不许再报零证据（(c) 在 ⇒ (b) 停跑）: {lines:?}"
    );
}
