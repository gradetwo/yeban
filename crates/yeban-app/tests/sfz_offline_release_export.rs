//! ⭐ **`--sfz` 导出的 `ampeg_release`**（任务 `#310`）：一条**端到端**护栏。
//!
//! # 这条判据在修什么
//!
//! `crates/yeban-engine/tests/sfz_offline_note_off.rs` 钉的是**引擎函数**的语义；
//! 本文件钉的是**产品路径**：`--sfz <音色> --export-wav` 出来的字节里**能不能听见释放段**。
//!
//! 两者都必须有 —— 引擎改对了但 `SfzTrackSource` 仍按时值截断，用户听到的还是一样。
//! （这不是假设：第一版修完引擎后 `render_block` 的窗口上限仍钉在 `target_frames` 上，
//! `tests/sfz_pan.rs` 当场红了，因为 `--sfz` 那条腿一个音符都不出声。）
//!
//! # 判据
//!
//! | # | 判据 | 判定量 |
//! | :--- | :--- | :--- |
//! | E1 | 两份只差 `ampeg_release` 的 SFZ ⇒ 导出**必须不同** | 位级摘要（`digest`）不等 |
//! | E2 | **有** `ampeg_release` 的那份：note-off 之后仍有信号 | `t = 0.65 s`（释放段中点）的左声道绝对值 > 0 |
//! | E3 | **没有** `ampeg_release` 的那份：note-off 之后必须是静音 | `t = 0.65 s` 逐位 0 |
//! | E4 | 两份在 note-off **之前**的值相同（释放段不影响按住的段） | `t = 0.25 s` 的差 < 1e-6 |
//!
//! ⚠ 不钉 WAV 的绝对 `sha256`（机器相关；台账第 313 条）：只做**同一台机器**的 A/B 自比。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;
use std::sync::Arc;

use yeban_app::export_wav::WavExportOptions;
use yeban_app::render_core::render_project_f32_with_assets;
use yeban_app::sfz_load::load_sfz_with_instrument;
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote, RoutingEdge,
    RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_render::dither::{BitDepth, PcmBuffer};
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;

/// 音符时长（tick）：120 BPM ＋ PPQ 960 ⇒ **960 tick = 0.5 s**。
const NOTE_TICKS: u64 = 960;

/// 导出帧数：1 s（音符 0.5 s ＋ 释放 0.3 s 之后还剩 0.2 s 静音，足够判"尾巴停在哪"）。
const FRAMES: u64 = 48_000;

fn id(tail: &str) -> EntityId {
    EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8P0{tail}")).expect("夹具 ULID 必须合法")
}

/// 临时夹具目录（`Drop` 删掉自己）。
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-310-{name}-{}-{}",
            std::process::id(),
            EntityId::new()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建夹具目录");
        // ⭐ **220 Hz 正弦**（1 s，幅度 0.5）而不是常量：渲染链里有 DC blocker，
        //    常量会被它转成 **Nyquist 振荡**（±0.268 逐样本交替）⇒ 任何"取某一个
        //    采样点"的判据都会**随机落在过零点**上（第一版就是这么红的：`t = 0.65 s`
        //    读到 4.8e-6，而同一时刻的包络其实是 0.25）。正弦没有这个问题，
        //    但判据仍然按**窗口峰值**取（见 `window_peak`）—— 双保险。
        let samples: Vec<i16> = (0..48_000)
            .map(|i| {
                let phase = std::f64::consts::TAU * 220.0 * f64::from(i) / 48_000.0;
                (0.5 * phase.sin() * 32_767.0) as i16
            })
            .collect();
        write_plain_wav(
            dir.join("const.wav"),
            &PcmFormat::integer(1, 48_000, 16),
            &PcmBuffer::Int16(samples),
        )
        .expect("写夹具 WAV");
        Self { dir }
    }

    /// 一份只差 `ampeg_release` 的 SFZ。
    fn write_sfz(&self, name: &str, release: &str) -> PathBuf {
        let path = self.dir.join(format!("{name}.sfz"));
        std::fs::write(
            &path,
            format!(
                "<region> sample=const.wav lokey=0 hikey=127 pitch_keycenter=60 \
                 ampeg_release={release}\n"
            ),
        )
        .expect("写夹具 SFZ");
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 一条 MIDI 轨（1 个音符）→ Master 的工程。
fn project() -> YebanProjectV1 {
    let master = id("M1");
    let track_id = id("T1");
    let clip = id("K1");
    let placement = id("P1");
    let note = id("N1");
    let edge = id("E1");

    let mut project = YebanProjectV1 {
        id: id("Z1"),
        title: "sfz offline release fixture".to_owned(),
        bpm: 120.0,
        rng_seed: 0x5945_4241_4E00_0310,
        master_bus_track_id: master,
        ..YebanProjectV1::default()
    };
    project.tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    project.routing_graph.nodes.push(master);

    let mut track = TrackV3 {
        id: track_id,
        name: "Lead".to_owned(),
        kind: TrackKind::Midi,
        ..TrackV3::default()
    };
    track.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: NOTE_TICKS,
            loop_config: LoopConfig::default(),
            muted: false,
        },
    );
    let mut notes = BTreeMap::new();
    notes.insert(note, MidiNote::new(note, 0, 60, NOTE_TICKS));
    project.clip_pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "clip".to_owned(),
            content: ClipContent::Midi { notes },
        },
    );
    project.tracks.insert(track_id, track);
    project.routing_graph.nodes.push(track_id);
    project.routing_graph.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: track_id,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        },
    );
    project.routing_graph.nodes.sort_unstable();
    project.validate().expect("夹具工程必须合法");
    project
}

fn options() -> WavExportOptions {
    WavExportOptions {
        frames: FRAMES,
        sample_rate: 48_000,
        channels: 2,
        bit_depth: BitDepth::Float32,
        use_engine_source: true,
        ..WavExportOptions::default()
    }
}

/// 用 `--sfz` 旁路渲一份，返回（左声道样本, 位级摘要）。
fn render_via_sfz(fixture: &Fixture, sfz_path: &Path) -> (Vec<f32>, String) {
    let (_, instrument) = load_sfz_with_instrument(sfz_path, None, 48_000.0).expect("装载夹具 SFZ");
    let out = render_project_f32_with_assets(
        &project(),
        options(),
        Some((Arc::new(instrument), Some(fixture.dir.clone()))),
        &[],
    )
    .expect("--sfz 旁路必须能渲染");
    let left: Vec<f32> = out
        .samples
        .as_chunks::<2>()
        .0
        .iter()
        .map(|f| f[0])
        .collect();
    (left, out.digest)
}

/** 判据窗口的半宽（帧）：±240 帧 ＝ **±5 ms**。 */
const HALF_WINDOW: usize = 240;
/** note-off 之后的判据中心（0.65 s ＝ 释放段中点）。 */
const AFTER_NOTE_OFF: usize = 31_200;
/** 按住段的判据中心（0.25 s），两份必须同值。 */
const HELD: usize = 12_000;

/// 窗口内**峰值**（绝对值最大）。
///
/// ⚠ 为什么不是单点（第一版就栽在这里）：样本是周期信号，单点可能正好落在过零点；
/// 而且渲染链的 DC blocker 会把直流样本变成 Nyquist 振荡（逐样本 ±交替）。
/// 判"有没有声音"要看窗口峰值，⛔ 不是某一个下标。
fn window_peak(samples: &[f32], center: usize) -> f32 {
    let lo = center.saturating_sub(HALF_WINDOW);
    let hi = (center + HALF_WINDOW).min(samples.len());
    samples[lo..hi]
        .iter()
        .fold(0.0_f32, |acc, v| acc.max(v.abs()))
}

#[test]
fn e1_release_changes_the_exported_bytes() {
    let fixture = Fixture::new("bytes");
    let with_release = fixture.write_sfz("rel_on", "0.3");
    let without = fixture.write_sfz("rel_off", "0");

    let (samples_on, digest_on) = render_via_sfz(&fixture, &with_release);
    let (samples_off, digest_off) = render_via_sfz(&fixture, &without);

    assert_ne!(
        digest_on, digest_off,
        "只差一个 `ampeg_release` 的两份 SFZ 必须导出不同字节（修前两者**逐字节相同**）"
    );

    // `E4`：按住段同值（释放段不该影响 note-off 之前）
    assert!(
        (HELD + HALF_WINDOW) < samples_on.len() && (HELD + HALF_WINDOW) < samples_off.len(),
        "导出帧数必须覆盖按住段的判据窗口"
    );
    let held_diff = samples_on[HELD - HALF_WINDOW..HELD + HALF_WINDOW]
        .iter()
        .zip(&samples_off[HELD - HALF_WINDOW..HELD + HALF_WINDOW])
        .fold(0.0_f32, |acc, (a, b)| acc.max((a - b).abs()));
    let held_on = window_peak(&samples_on, HELD);
    assert!(held_on > 0.1, "夹具必须出声：峰值 {held_on}");
    assert!(
        held_diff < 1e-6,
        "按住段（0.25 s ±5 ms）两份必须同值（释放段不该影响 note-off 之前）：最大差 {held_diff}"
    );

    // `E2`：有 release ⇒ 释放段中点仍有信号
    let tail_on = window_peak(&samples_on, AFTER_NOTE_OFF);
    assert!(
        tail_on > 1e-3,
        "有 `ampeg_release=0.3` ⇒ note-off 之后 0.15 s（释放段中点）必须仍有信号：峰值 {tail_on}"
    );
    // ⭐ 并**量化**它落在哪：线性释放 ⇒ 中点应≈按住段的一半（`(R−k)/R = 0.5`）。
    //    这条比"非零"强得多 —— 它同时钉住了"release 从 note-off 起算"与"线性曲线"。
    let ratio = tail_on / held_on;
    assert!(
        (0.35..0.65).contains(&ratio),
        "释放段中点应≈按住段的一半（线性释放 `(R−k)/R`）：实测 {ratio:.3}"
    );

    // `E3`：没有 release ⇒ note-off 之后逐位 0
    let tail_off = window_peak(&samples_off, AFTER_NOTE_OFF);
    assert_eq!(
        tail_off.to_bits(),
        0.0_f32.to_bits(),
        "没有 `ampeg_release` ⇒ note-off 之后必须**逐位 0**：峰值 {tail_off}"
    );
}
