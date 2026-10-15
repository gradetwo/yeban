//! 判据（任务单 `#80`）：**路径 ⇒ 已摊平 SFZ 的缓存** ＋ `EngineHost::reload`／
//! `publish_project` 真的把待摊平表 arm 进快照（补 `#79` 登记的"实时／GUI 半边"）。
//!
//! ## 本单补的是哪一环
//!
//! | 层 | 现状 |
//! | :--- | :--- |
//! | 模型面 | `TrackV3.sampler: Option<SamplerBinding>`（`#76`／`R731`）✓ |
//! | 引擎面 | `EngineSnapshot.sfz_pending`（`#78`）⇒ 只带路径、⛔ 不读盘 ✓ |
//! | 导出／录音 | `render_core` 构造期走 `arm_pending_sfz`（`#79`）✓ |
//! | **实时／GUI** | ⭐ **本单**：`EngineHost::reload`／`publish_project` 走**同一个** `arm_pending_sfz`，前面加一层 `SfzPlanCache`（同路径同采样率 ⇒ ⛔ 不重复读盘） |
//!
//! ## 判定量（每个数字都有单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | `entries` / `tracks` | 缓存此刻的摊平表份数 / 有绑定记录的轨数 | 份 / 条 |
//! | `hits` / `misses` / `evicted` | 缓存复用次数 / 重建次数 / 回收条目数 | 次 / 次 / 条 |
//! | `sfz_text_reads` / `sample_decodes` | **本线程**发起的 SFZ 文本读盘 / 样本解码尝试 | 次 |
//! | `plan_bits` | 摊平表的逐位指纹（样本路径 ＋ PCM 位型 ＋ 原始采样率位型） | 无符号 64 位 |
//! | 整份快照相等 | `EngineSnapshot` 的 `PartialEq`（全部字段） | 布尔（逐位） |
//!
//! ⚠ 读盘计数是**按线程**的（`sfz_load::read_probe`）：libtest 每个判据一条线程 ⇒
//! 并行的判据**不会**互相污染读数（与 `yeban-engine` 的 `release_probe` 同一条纪律）。
//!
//! ## 注入（每一条都由本文件的判据打红；读数贴在本单交付报告里）
//!
//! * **① `reload` 不 arm**：删掉 `arm_pending_sfz_into_snapshot` 的调用 ⇒
//!   [`the_publish_points_arm_the_flattened_table`] 的"摊平表非空 / 待摊平表为空"两条腿红；
//! * **② 缓存永不失效**：`SfzPlanCache::get_or_prepare` 不再比对路径 ⇒
//!   [`a_changed_path_invalidates_and_rebuilds`] 红（旧摊平表被复用）；
//! * **②' 去掉缓存**：`arm_pending_sfz_into_snapshot` 直接调 `decode_sfz_plan` ⇒
//!   [`repeated_publishing_does_not_read_the_disk_again`] 的"读盘次数不增加"红；
//! * **应绿腿**：[`an_absent_sampler_is_bit_identical_to_the_direct_projection`] 在①②'
//!   三种注入下都必须**照旧绿**（`sampler = None` 那条路一个字都没碰）。

use std::path::{Path, PathBuf};

use yeban_app::engine_host::{EditMark, EngineHost, EngineHostError};
use yeban_app::sfz_load::{
    decode_sfz_plan, load_sfz_with_instrument, read_probe, read_sample_file,
};
use yeban_engine::sfz_instrument::{InMemorySample, SfzPlan};
use yeban_engine::snapshot::EngineSnapshot;
use yeban_model::{EntityId, SampleRate, SamplerBinding, YebanProjectV1};
use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;

/// `24-bit` 满量程（夹具写入常量样本用；与 `read_plain_wav_mono_f32` 同口径）。
const I24_SCALE: f32 = 8_388_608.0;

/// 临时目录（形态照 `sfz_model_chain.rs` 的 `scratch`）。
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-sfz-realtime-{tag}-{}-{}",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 写一条**常量** PCM 样本（24-bit 单声道；值 = 满量程的 `value` 倍）。
fn write_const_sample(path: &Path, value: f32) {
    let scale = (I24_SCALE * value) as i32;
    let buffer = PcmBuffer::Int24(vec![scale; 4_800]);
    let format = PcmFormat::integer(1, 48_000, 24);
    write_plain_wav(path, &format, &buffer).expect("写样本夹具");
}

/// 写一条**阶梯斜坡** PCM 样本（24-bit 单声道；⛔ 不是常量）。
///
/// 为什么要它：摊平表对采样率的依赖落在**播放速率**上（`sample_hz / engine_hz`），而常量
/// 样本按任何速率读出来都是同一个值 ⇒ 常量夹具**看不见**采样率。（`PreparedRegion` 的速率
/// 没有公开读口，引擎侧本单⛔ 不改 ⇒ 用**渲染输出**当速率敏感的观测量。）
fn write_ramp_sample(path: &Path) {
    let pcm: Vec<i32> = (0..4_800)
        .map(|index| ((index % 2_400) - 1_200) * 2_000)
        .collect();
    let buffer = PcmBuffer::Int24(pcm);
    let format = PcmFormat::integer(1, 48_000, 24);
    write_plain_wav(path, &format, &buffer).expect("写斜坡样本夹具");
}

/// 用摊平表渲染一个音符，返回**逐位**输出（判据用；⛔ 不比较"接近"）。
fn rendered_bits(plan: &SfzPlan, note: u8, velocity: u8, frames: usize) -> Vec<u32> {
    let mut out = vec![0.0_f32; frames];
    let written = plan.prepared().render_block_into(note, velocity, &mut out);
    out.truncate(written.max(1));
    out.iter().map(|sample| sample.to_bits()).collect()
}

/// 工程里那条**带 MIDI 音符**的非主总线轨。
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

/// 那条轨上**出现过的全部音高**（排序 ＋ 去重）。
fn pitches_of(project: &YebanProjectV1, track_id: EntityId) -> Vec<u8> {
    let mut pitches: Vec<u8> = project
        .tracks
        .get(&track_id)
        .expect("轨必须在")
        .clips
        .values()
        .filter_map(|placement| project.clip_pool.get(&placement.clip_id))
        .filter_map(|entry| entry.content.notes())
        .flat_map(|notes| notes.values().map(|note| note.pitch))
        .collect();
    pitches.sort_unstable();
    pitches.dedup();
    assert!(!pitches.is_empty(), "夹具必须真的带音符");
    pitches
}

/// 给那条 MIDI 轨写一个 `sampler = Some(路径)`（或 `None`）。
fn bind_sampler(project: &mut YebanProjectV1, track_id: EntityId, path: Option<&Path>) {
    project.tracks.get_mut(&track_id).expect("轨必须在").sampler =
        path.map(|path| SamplerBinding {
            path: path.to_string_lossy().into_owned(),
        });
}

/// 写一份"每个工程音高一条区域"的 SFZ（`sample=` 用相对路径 ⇒ 按 SFZ 目录解析）。
fn write_sfz(dir: &Path, name: &str, pitches: &[u8], sample: &str) -> PathBuf {
    let mut text = String::new();
    for pitch in pitches {
        text.push_str(&format!(
            "<region> sample={sample} key={pitch} pitch_keycenter={pitch}\n"
        ));
    }
    let path = dir.join(name);
    std::fs::write(&path, text).expect("写 SFZ 夹具");
    path
}

/// ⭐ **摊平表的逐位指纹**：`(区域数, 内存字节, 样本表位型哈希, 缺样本路径)`。
///
/// 哈希卷进：每条样本的**路径字节**、**PCM 全部 `f32` 的 `to_bits()`（小端）**、
/// **原始采样率的位型**。⛔ 不哈希 `f32` 的十进制形态（那会掩盖尾数差异）。
fn plan_fingerprint(plan: &SfzPlan) -> (usize, usize, u64, Vec<String>) {
    let prepared = plan.prepared();
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for sample in prepared.samples() {
        for byte in sample.path().as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        for value in sample.pcm() {
            for byte in value.to_bits().to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        for byte in sample.sample_hz().to_bits().to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    (
        prepared.region_count(),
        prepared.memory_bytes(),
        hash,
        prepared.missing_sample_paths().to_vec(),
    )
}

/// 测试内**显式**走一遍六步，独立搭出摊平表（与 `#79` 判据同一份"手搭参照"）。
///
/// ⚠ 它**故意不调用** `decode_sfz_plan`：两条路只要有一个字节不同，逐位判据立刻红。
fn hand_built_plan(sfz_path: &Path, engine_hz: f32) -> SfzPlan {
    let (_load, instrument) =
        load_sfz_with_instrument(sfz_path, None, engine_hz).expect("手搭：装载必须成功");
    let referenced = instrument
        .referenced_sample_paths()
        .expect("手搭：路径全集");
    let parent = sfz_path.parent().expect("SFZ 必须有父目录");
    let mut provided: Vec<(String, InMemorySample)> = Vec::new();
    for raw in &referenced {
        let raw_path = Path::new(raw);
        let resolved = if raw_path.is_absolute() {
            raw_path.to_path_buf()
        } else {
            parent.join(raw_path)
        };
        if let Ok((sample, _frames, _describe)) = read_sample_file(&resolved) {
            provided.push((raw.clone(), sample));
        }
    }
    SfzPlan::prepare_with_samples(&instrument, &provided, engine_hz).expect("手搭：摊平必须成功")
}

/// ⭐ 判据一（正向）：`reload`／`publish_project` 后 ⇒ 快照该轨**摊平表非空**、
/// **待摊平表为空**，且那份 `PreparedSfz` 与 `#79` 的 `decode_sfz_plan` 产物
/// **以及与测试内手搭的同一份**逐位相同。
///
/// # 判定量
///
/// * `snapshot.sfz_plan(lead)` 为 `Some`、`snapshot.pending_sfz(lead)` 为 `None`、
///   `snapshot.sfz_pending()` 为空；
/// * `plan_fingerprint(snapshot 的) == plan_fingerprint(decode_sfz_plan 的)`（逐位）；
/// * `snapshot 的 PreparedSfz == 手搭的 PreparedSfz`（**结构级** `PartialEq`）；
/// * `publish_project` 换一份 SFZ ⇒ 快照换到**新**那份（与旧那份指纹不同）。
///
/// # 怎么变红
///
/// * 删掉 [`EngineHost::reload`] 里 `arm_pending_sfz_into_snapshot` 的调用 ⇒
///   摊平表为空、待摊平表非空 ⇒ 本判据红（注入 ①）。
#[test]
fn the_publish_points_arm_the_flattened_table() {
    let dir = scratch("arm");
    write_const_sample(&dir.join("const.wav"), 0.25);
    write_const_sample(&dir.join("const2.wav"), 0.5);

    let mut project = yeban_model::samples::filled_project();
    let lead = midi_track_of(&project);
    let pitches = pitches_of(&project, lead);
    let sfz_path = write_sfz(&dir, "keys.sfz", &pitches, "const.wav");
    bind_sampler(&mut project, lead, Some(&sfz_path));

    let mut host = EngineHost::new();
    host.reload(&project, 0).expect("reload 必须成功");

    let snapshot = host.current_snapshot().expect("reload 后必须有快照");
    let engine_hz = snapshot.sample_rate() as f32;
    assert_eq!(engine_hz, 48_000.0, "filled_project 的采样率");
    assert!(
        snapshot.sfz_pending().is_empty(),
        "⭐ reload 后**待摊平表必须为空**（摊平表取代了它）：{:?}",
        snapshot.sfz_pending()
    );
    assert!(
        snapshot.pending_sfz(&lead).is_none(),
        "⭐ 那条轨⛔ 不得留下待摊平项"
    );
    let armed = snapshot
        .sfz_plan(&lead)
        .expect("⭐ reload 后该轨必须有摊平表（#79 登记的实时半边）");

    // #79 的产物（同一条管线）＋ 测试内手搭的独立参照。
    let (expected, report) = decode_sfz_plan(&sfz_path, engine_hz).expect("#79 管线解码");
    let hand = hand_built_plan(&sfz_path, engine_hz);
    println!(
        "[app-sfz-realtime/arm] reload: armed={:?} pipeline={:?} hand={:?} report: {}",
        plan_fingerprint(armed),
        plan_fingerprint(&expected),
        plan_fingerprint(&hand),
        report.summary(),
    );
    assert_eq!(
        plan_fingerprint(armed),
        plan_fingerprint(&expected),
        "⭐ 快照里的摊平表必须与 #79 的 `decode_sfz_plan` 产物**逐位相同**"
    );
    assert_eq!(
        plan_fingerprint(armed),
        plan_fingerprint(&hand),
        "⭐ 也必须与测试内**手搭**的同一份逐位相同（第 244 条：与手搭对照）"
    );
    assert_eq!(
        armed.prepared(),
        hand.prepared(),
        "⭐ 结构级 `PartialEq` 也必须相等（PreparedSfz 全部字段逐位）"
    );
    assert_eq!(report.regions, pitches.len(), "区域数 = 工程音高数");

    // ---- publish 半边：换一份 SFZ（不同的样本内容）⇒ 快照必须换到新那份 ----
    let sfz_two = write_sfz(&dir, "keys2.sfz", &pitches, "const2.wav");
    let mut changed = project.clone();
    bind_sampler(&mut changed, lead, Some(&sfz_two));
    let revision = host
        .publish_project(&changed, EditMark::new(None, 1))
        .expect("publish 必须成功");
    let published = host.current_snapshot().expect("发布后必须有快照");
    assert!(
        published.sfz_pending().is_empty(),
        "⭐ publish 后待摊平表也必须为空"
    );
    let armed_two = published
        .sfz_plan(&lead)
        .expect("⭐ publish 后该轨必须有摊平表");
    let expected_two = decode_sfz_plan(&sfz_two, engine_hz)
        .expect("第二份 #79 产物")
        .0;
    println!(
        "[app-sfz-realtime/arm] publish: revision={revision} armed={:?} expected={:?}",
        plan_fingerprint(armed_two),
        plan_fingerprint(&expected_two),
    );
    assert_eq!(
        plan_fingerprint(armed_two),
        plan_fingerprint(&expected_two),
        "⭐ publish 后快照里必须是**新**那份 SFZ 的摊平表（逐位）"
    );
    assert_ne!(
        plan_fingerprint(armed_two),
        plan_fingerprint(armed),
        "⭐ 两份 SFZ 的样本内容不同 ⇒ 指纹必须不同（否则'换到了新那份'是空话）"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// ⭐ 判据二（反向对照，也是注入的**应绿腿**）：`sampler = None` ⇒ 快照与
/// `EngineSnapshot::from_project` 的**直接投影**逐位相同（整份结构 `PartialEq`），
/// 且这条路**一次盘都不读**。
///
/// # 判定量
///
/// * `*snapshot == EngineSnapshot::from_project(project, revision)`（**全部字段**）；
/// * `read_probe::counts()` 在 `reload` ＋ `publish` 前后**一位不动**；
/// * `sfz_cache_readings().entries == 0`（没有绑定 ⇒ 缓存空）。
#[test]
fn an_absent_sampler_is_bit_identical_to_the_direct_projection() {
    let project = yeban_model::samples::filled_project();
    let lead = midi_track_of(&project);
    assert!(
        project.tracks[&lead].sampler.is_none(),
        "filled_project 的 sampler 默认必须是 None"
    );

    let mut host = EngineHost::new();
    let before = read_probe::counts();
    host.reload(&project, 0).expect("reload 必须成功");
    let direct = EngineSnapshot::from_project(&project, 1).expect("直接投影");
    let snapshot = host.current_snapshot().expect("有快照");
    println!(
        "[app-sfz-realtime/none] reload: reads_before={before:?} reads_after={:?} \
         cache={:?} revision={}",
        read_probe::counts(),
        host.sfz_cache_readings(),
        snapshot.revision(),
    );
    assert_eq!(
        *snapshot, direct,
        "⭐ `sampler = None` ⇒ 整份快照必须与直接投影**逐位相同**（接线前就是这条投影）"
    );
    assert_eq!(
        read_probe::counts(),
        before,
        "⭐ `sampler = None` ⇒ ⛔ 一次盘都不读（缓存/解码整段跳过）"
    );

    host.publish_project(&project, EditMark::new(None, 1))
        .expect("publish 必须成功");
    let direct_after = EngineSnapshot::from_project(&project, 2).expect("直接投影（第二次）");
    let snapshot_after = host.current_snapshot().expect("有快照");
    assert_eq!(
        *snapshot_after, direct_after,
        "⭐ 增量发布也必须与直接投影逐位相同"
    );
    assert_eq!(
        read_probe::counts(),
        before,
        "⭐ 发布也不得读盘（应绿腿：三种注入下都必须照旧绿）"
    );
    assert_eq!(
        host.sfz_cache_readings().entries,
        0,
        "没有 SFZ 绑定 ⇒ 缓存必须是空的"
    );
    assert_eq!(
        host.sfz_cache_readings().misses,
        0,
        "没有 SFZ 绑定 ⇒ ⛔ 不得发生任何重建"
    );
}

/// ⭐ 判据三（缓存判据，本单特有）：同一工程连续 `reload`／`publish` N 次 ⇒
/// **读盘次数不增加**（缓存命中）。
///
/// # 判定量
///
/// * 首次 `reload`：`sfz_text_reads == 1`、`sample_decodes == 1`（一份 SFZ ＋ 一份样本）；
/// * 之后 8×`reload` ＋ 8×`publish`：`read_probe::counts()` **与首次之后相同**；
/// * 缓存读数：`misses == 1`、`hits == 16`、`entries == 1`；
/// * **见证（防"计数器根本没动"）**：再手动调一次 `decode_sfz_plan` ⇒
///   计数器必须**当场 +2**（+1 文本，+1 样本）⇒ 说明"没增加"是**跑出来的**，不是量程外。
///
/// # 怎么变红
///
/// * 去掉缓存（`arm_pending_sfz_into_snapshot` 直接 `decode_sfz_plan`）⇒ 每跳 +2 ⇒ 红；
/// * 缓存键丢掉采样率 ⇒ 本判据仍绿（那是判据五的事），但注入 ② 会让判据四红。
#[test]
fn repeated_publishing_does_not_read_the_disk_again() {
    let dir = scratch("cache");
    write_const_sample(&dir.join("const.wav"), 0.25);
    let mut project = yeban_model::samples::filled_project();
    let lead = midi_track_of(&project);
    let pitches = pitches_of(&project, lead);
    let sfz_path = write_sfz(&dir, "keys.sfz", &pitches, "const.wav");
    bind_sampler(&mut project, lead, Some(&sfz_path));

    let mut host = EngineHost::new();
    let before = read_probe::counts();
    host.reload(&project, 0).expect("首次 reload");
    let after_first = read_probe::counts();
    let first = after_first.since(before);
    println!(
        "[app-sfz-realtime/cache] 首次 reload 的读盘增量: text=+{} sample=+{}；cache={:?}",
        first.sfz_text_reads,
        first.sample_decodes,
        host.sfz_cache_readings(),
    );
    assert_eq!(first.sfz_text_reads, 1, "首次必须读一次 SFZ 文本");
    assert_eq!(first.sample_decodes, 1, "首次必须解码一次样本");

    for jump in 0..8 {
        host.reload(&project, 0).expect("再次 reload");
        host.publish_project(&project, EditMark::new(None, jump))
            .expect("增量发布");
    }
    let after_many = read_probe::counts();
    let readings = host.sfz_cache_readings();
    println!(
        "[app-sfz-realtime/cache] 8×reload ＋ 8×publish 之后: text=+{} sample=+{}；cache={readings:?}",
        after_many.since(after_first).sfz_text_reads,
        after_many.since(after_first).sample_decodes,
    );
    assert_eq!(
        after_many, after_first,
        "⭐ 同一工程连续 reload／publish ⇒ **读盘次数不得增加**（缓存必须命中）"
    );
    assert_eq!(readings.misses, 1, "整个窗口只允许一次重建");
    assert_eq!(
        readings.hits, 16,
        "8×reload ＋ 8×publish = 16 次命中（首次 reload 是那次唯一的未命中）"
    );
    assert_eq!(readings.entries, 1, "同一路径 ⇒ 缓存里只有一份摊平表");

    // ⭐ 见证：计数器是**活的**（否则"没增加"可能只是它没在数）。
    let witness_before = read_probe::counts();
    let _ = decode_sfz_plan(&sfz_path, 48_000.0).expect("手动解码");
    let witness = read_probe::counts().since(witness_before);
    println!(
        "[app-sfz-realtime/cache] 见证（手动 decode_sfz_plan 一次）: text=+{} sample=+{}",
        witness.sfz_text_reads, witness.sample_decodes,
    );
    assert_eq!(
        witness.total(),
        2,
        "计数器必须当场 +2 ⇒ '没增加'是量程内跑出来的"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// ⭐ 判据四（失效条件 ①：**路径变**）：换 `sampler` 路径 ⇒ 缓存**失效重建**（读盘 +2），
/// 旧条目被回收，快照里换到**新路径**的摊平表。
///
/// # 怎么变红
///
/// * 注入 ②（`get_or_prepare` 不再比对路径 ⇒ 旧条目"永不失效"）⇒ 本判据红：
///   读盘**不增加**、快照里仍是**旧**那份摊平表。
#[test]
fn a_changed_path_invalidates_and_rebuilds() {
    let dir = scratch("path-change");
    write_const_sample(&dir.join("const.wav"), 0.25);
    write_const_sample(&dir.join("const2.wav"), 0.5);
    let mut project = yeban_model::samples::filled_project();
    let lead = midi_track_of(&project);
    let pitches = pitches_of(&project, lead);
    let first_sfz = write_sfz(&dir, "keys.sfz", &pitches, "const.wav");
    let second_sfz = write_sfz(&dir, "keys2.sfz", &pitches, "const2.wav");

    let mut host = EngineHost::new();
    bind_sampler(&mut project, lead, Some(&first_sfz));
    host.reload(&project, 0).expect("首次 reload");
    let after_first = read_probe::counts();
    let readings_first = host.sfz_cache_readings();
    assert_eq!(readings_first.misses, 1, "首次必须重建一次");
    assert_eq!(readings_first.entries, 1, "首次后缓存里一份");

    let mut changed = project.clone();
    bind_sampler(&mut changed, lead, Some(&second_sfz));
    host.reload(&changed, 0).expect("换路径后 reload");
    let delta = read_probe::counts().since(after_first);
    let readings = host.sfz_cache_readings();
    let snapshot = host.current_snapshot().expect("有快照");
    let armed = snapshot.sfz_plan(&lead).expect("换路径后必须有摊平表");
    let (expected_new, _report) = decode_sfz_plan(&second_sfz, 48_000.0).expect("#79 产物");
    let (expected_old, _report) = decode_sfz_plan(&first_sfz, 48_000.0).expect("#79 产物");
    println!(
        "[app-sfz-realtime/path-change] 换路径的读盘增量: text=+{} sample=+{}；cache={readings:?}；\
         armed={:?} new={:?} old={:?}",
        delta.sfz_text_reads,
        delta.sample_decodes,
        plan_fingerprint(armed),
        plan_fingerprint(&expected_new),
        plan_fingerprint(&expected_old),
    );
    assert_eq!(delta.sfz_text_reads, 1, "路径变 ⇒ 必须重读 SFZ 文本");
    assert_eq!(delta.sample_decodes, 1, "路径变 ⇒ 必须重新解码样本");
    assert_eq!(readings.misses, 2, "路径变 ⇒ 第二次重建");
    assert_eq!(readings.entries, 1, "旧条目必须被回收（⛔ 不是只涨不落）");
    assert_eq!(readings.evicted, 1, "回收计数 +1");
    assert_eq!(
        plan_fingerprint(armed),
        plan_fingerprint(&expected_new),
        "⭐ 快照里必须换成**新路径**的摊平表（逐位）"
    );
    assert_ne!(
        plan_fingerprint(armed),
        plan_fingerprint(&expected_old),
        "⭐ ⛔ 不得复用旧路径的摊平表（注入 ② 这一条会红）"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// ⭐ 判据五（失效条件 ②：**采样率变**）：同一个路径、工程采样率 48k ⇒ 96k
/// ⇒ 缓存**失效重建**，快照里是**按新采样率**摊平的那一份。
///
/// # 判定量
///
/// * 换采样率那一跳的读盘增量：`sfz_text_reads == 1`、`sample_decodes == 1`（重建）；
/// * `snapshot.sample_rate() == 96_000`、`snapshot.sfz_plan(lead)` 与
///   `decode_sfz_plan(path, 96_000.0)` 的产物**逐位相同**；
/// * ⭐ **速率敏感的内容见证**：用斜坡样本（⛔ 常量样本看不见速率）渲染同一个音符，
///   `armed` 的输出必须与 **48k 那份**逐位**不同**、与 **96k 那份**逐位**相同**
///   ⇒ "没有复用旧那份"不只是读盘计数，**内容**也换了。
///
/// ⚠ 摊平表的 `plan_fingerprint` 对采样率**不敏感**：速率住在 `PreparedRegion` 里，而它
/// 没有公开读口（引擎侧本单⛔ 不改）⇒ 本判据用**渲染输出**补上速率这一维。
///
/// # 怎么变红
///
/// * `get_or_prepare` 的命中判定不再比采样率（"缓存永不失效"的另一种写法）⇒
///   读盘**不增加**、且渲染输出仍是 48k 那份 ⇒ 本判据红。
#[test]
fn a_changed_sample_rate_invalidates_and_rebuilds() {
    let dir = scratch("rate-change");
    write_ramp_sample(&dir.join("ramp.wav"));
    let mut project = yeban_model::samples::filled_project();
    let lead = midi_track_of(&project);
    let pitches = pitches_of(&project, lead);
    let sfz_path = write_sfz(&dir, "keys.sfz", &pitches, "ramp.wav");
    bind_sampler(&mut project, lead, Some(&sfz_path));

    let mut host = EngineHost::new();
    host.reload(&project, 0).expect("48k reload");
    // ⚠ 基线必须在**手动 decode 之后**取：那次手动解码自己也会读盘（否则把它的读数
    //    算进"换采样率的增量"里 ⇒ 本判据数错对象）。
    let (plan_48k, _report) = decode_sfz_plan(&sfz_path, 48_000.0).expect("#79 产物 48k");
    let after_48k = read_probe::counts();

    let mut faster = project.clone();
    faster.audio_config.sample_rate = SampleRate::Hz96000;
    host.reload(&faster, 0).expect("96k reload");
    let delta = read_probe::counts().since(after_48k);
    let readings = host.sfz_cache_readings();
    let snapshot = host.current_snapshot().expect("有快照");
    assert_eq!(snapshot.sample_rate(), 96_000, "快照必须换到新采样率");
    let armed = snapshot.sfz_plan(&lead).expect("96k 下必须有摊平表");
    let (plan_96k, _report) = decode_sfz_plan(&sfz_path, 96_000.0).expect("#79 产物 96k");
    let note = pitches[0];
    let rendered_armed = rendered_bits(armed, note, 100, 4_800);
    let rendered_48k = rendered_bits(&plan_48k, note, 100, 4_800);
    let rendered_96k = rendered_bits(&plan_96k, note, 100, 4_800);
    println!(
        "[app-sfz-realtime/rate-change] 换采样率的读盘增量: text=+{} sample=+{}；cache={readings:?}；\
         armed={:?} plan96k={:?} plan48k={:?}；渲染帧 armed={} plan48k={} plan96k={}；\
         armed≠48k={} armed=96k={}",
        delta.sfz_text_reads,
        delta.sample_decodes,
        plan_fingerprint(armed),
        plan_fingerprint(&plan_96k),
        plan_fingerprint(&plan_48k),
        rendered_armed.len(),
        rendered_48k.len(),
        rendered_96k.len(),
        rendered_armed != rendered_48k,
        rendered_armed == rendered_96k,
    );
    assert_eq!(delta.sfz_text_reads, 1, "采样率变 ⇒ 必须重读 SFZ 文本");
    assert_eq!(delta.sample_decodes, 1, "采样率变 ⇒ 必须重新解码样本");
    assert_eq!(readings.misses, 2, "采样率变 ⇒ 第二次重建");
    assert_eq!(readings.hits, 0, "两次都是未命中（同一个路径但采样率不同）");
    assert_eq!(
        plan_fingerprint(armed),
        plan_fingerprint(&plan_96k),
        "⭐ 快照里必须是**按 96k 摊平**的那一份（逐位）"
    );
    assert_eq!(
        rendered_armed, rendered_96k,
        "⭐ 速率敏感的内容见证：armed 的渲染输出必须与 96k 那份**逐位相同**"
    );
    assert_ne!(
        rendered_armed, rendered_48k,
        "⭐ ⛔ 不得复用 48k 那份摊平表（渲染输出必须与它逐位不同）"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// ⭐ 判据六（诚实性）：SFZ 文件读不到 ⇒ `reload`／`publish` **明确失败**
/// （[`EngineHostError::Sfz`]），⛔ 不静默回退到复音合成器，且**既有的一代一位不动**。
#[test]
fn a_missing_sfz_fails_the_publish_point_and_keeps_the_old_generation() {
    let dir = scratch("missing");
    let missing = dir.join("nope.sfz");
    let mut project = yeban_model::samples::filled_project();
    let lead = midi_track_of(&project);
    bind_sampler(&mut project, lead, Some(&missing));

    let mut host = EngineHost::new();
    let error = host.reload(&project, 0).expect_err("文件不存在必须失败");
    println!("[app-sfz-realtime/missing] reload: {error}");
    assert!(
        matches!(error, EngineHostError::Sfz(_)),
        "必须是 SFZ 错误（实际 {error:?}）"
    );
    assert!(!host.has_engine(), "失败的 reload ⛔ 不得装上一代引擎");

    // 先装一代好的（sampler = None），再发布一份坏的 ⇒ 旧快照一位不动。
    let good = yeban_model::samples::filled_project();
    host.reload(&good, 0).expect("好的工程");
    let before = host.current_snapshot().expect("有快照");
    let error = host
        .publish_project(&project, EditMark::new(None, 1))
        .expect_err("发布坏工程必须失败");
    assert!(
        matches!(error, EngineHostError::Sfz(_)),
        "必须是 SFZ 错误（实际 {error:?}）"
    );
    let after = host.current_snapshot().expect("有快照");
    assert_eq!(
        *after, *before,
        "⭐ 发布失败 ⇒ 槽里当前快照必须**一位不动**"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// ⭐ 判据七（失效条件 ③）：`sampler` 清回 `None` ⇒ 缓存条目被**回收**
/// （⛔ 不是只涨不落）。
#[test]
fn clearing_the_sampler_reclaims_the_cached_plan() {
    let dir = scratch("reclaim");
    write_const_sample(&dir.join("const.wav"), 0.25);
    let mut project = yeban_model::samples::filled_project();
    let lead = midi_track_of(&project);
    let pitches = pitches_of(&project, lead);
    let sfz_path = write_sfz(&dir, "keys.sfz", &pitches, "const.wav");
    bind_sampler(&mut project, lead, Some(&sfz_path));

    let mut host = EngineHost::new();
    host.reload(&project, 0).expect("有 sampler 的 reload");
    assert_eq!(host.sfz_cache_readings().entries, 1, "绑定在 ⇒ 一份条目");

    bind_sampler(&mut project, lead, None);
    host.reload(&project, 0).expect("清空 sampler 的 reload");
    let readings = host.sfz_cache_readings();
    let snapshot = host.current_snapshot().expect("有快照");
    println!("[app-sfz-realtime/reclaim] 清空 sampler 后: cache={readings:?}");
    assert_eq!(readings.entries, 0, "⛔ 旧摊平表必须被回收");
    assert_eq!(readings.tracks, 0, "绑定记录也必须清掉");
    assert_eq!(readings.evicted, 1, "回收计数 +1");
    assert!(
        snapshot.sfz_plan(&lead).is_none(),
        "⛔ 清空 sampler 后该轨不得再有摊平表"
    );
    assert!(
        snapshot.sfz_pending().is_empty(),
        "⛔ 也不得有残留的待摊平项"
    );
    std::fs::remove_dir_all(&dir).ok();
}
