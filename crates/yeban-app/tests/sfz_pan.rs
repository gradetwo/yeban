//! 任务单 `#255`：**采样器路径的轨声相**（CLI 侧）—— 两条入口都必须与 MIDI 路径
//! **同一条曲线**。
//!
//! # 缺陷（`#251` 刻意留下的缺口）
//!
//! `#251` 只给 **MIDI 路径**（引擎乐器／最小音源）与**音频片段**接了声相；
//! **采样器路径**两条入口都不乘 pan：
//!
//! | 入口 | 修前 | 修后 |
//! | :--- | :--- | :--- |
//! | `--sfz <文件>`（整工程旁路） | `SfzTrackSource` 把单声道**铺到所有声道** | `with_pan(track_pan_gains)` ⇒ 等功率分离 |
//! | `TrackV3.sampler`（按轨绑定） | `render_core` 按"快照 `sfz()` 表里有它"分流成 `(1.0, 1.0)` | 统一走 `track_pan_of` ⇒ 等功率分离 |
//!
//! 两条都乘**同一个** [`yeban_app::audio_clip::track_pan_gains`]（`pan == 0` ⇒ `(1.0, 1.0)`），
//! 因此 `pan = 0` 的工程**逐位不变**。
//!
//! # 判据（每个数字都有单位）
//!
//! | 判据 | 量 | 单位 |
//! | :--- | :--- | :--- |
//! | [`the_cli_sampler_paths_pan_like_the_midi_path`] | 96 帧内的左右**峰值**比对与比值 | 满刻度线性幅度 / 无量纲比 |
//! | 同上（`pan = -1`） | 右声道峰值 | 必须**逐位 0** |
//! | [`a_centered_sampler_keeps_the_pre_wiring_bytes`] | 交错 `f32` 的位级摘要（`RenderProjectOutput::digest`） | 64 位十六进制 |
//!
//! ⚠ 不钉 WAV 的**绝对** `sha256`（机器相关，台账第 313 条）：正对照用**同一台机器**的
//! 修前／修后自比；本文件打印摘要供报告引用，但只断言结构性事实（左右同值、`pan = 0` 恒等）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;
use std::sync::Arc;

use yeban_app::export_wav::WavExportOptions;
use yeban_app::render_core::render_project_f32_with_assets;
use yeban_app::sfz_load::load_sfz_with_instrument;
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote, RoutingEdge,
    RoutingKind, SamplerBinding, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_render::dither::{BitDepth, PcmBuffer};
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;

fn id(tail: &str) -> EntityId {
    EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8P0{tail}")).expect("夹具 ULID 必须合法")
}

/// 临时夹具目录（`Drop` 删掉自己；⛔ 工作树里不留临时文件）。
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-255-{name}-{}-{}",
            std::process::id(),
            EntityId::new()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建夹具目录");
        // 常量 0.5 的 16-bit 单声道样本：左右比与样本内容无关（同一份 PCM）。
        let samples: Vec<i16> = (0..48_000).map(|_| 16_384).collect();
        write_plain_wav(
            dir.join("const.wav"),
            &PcmFormat::integer(1, 48_000, 16),
            &PcmBuffer::Int16(samples),
        )
        .expect("写夹具 WAV");
        Self { dir }
    }

    /// 一份**全覆盖**区域的 SFZ：夹具里任何音高都落到 `const.wav`。
    fn write_sfz(&self) -> PathBuf {
        let path = self.dir.join("sampler.sfz");
        std::fs::write(
            &path,
            "<region> sample=const.wav lokey=0 hikey=127 pitch_keycenter=60\n",
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

/// 一条 MIDI 轨（1 个音符，960 tick = 120 BPM 下的 0.5 s）→ Master 的工程。
///
/// `sampler = Some(path)` ⇒ 走**模型链条**（引擎 SFZ 分支）；`None` ⇒ 走 MIDI 振荡器路径
/// （同一条轨、同一个 `pan`，这是"与 MIDI 同曲线"的对照腿）。
fn project(pan: f32, sampler: Option<&Path>) -> YebanProjectV1 {
    let master = id("M1");
    let track_id = id("T1");
    let clip = id("K1");
    let placement = id("P1");
    let note = id("N1");
    let edge = id("E1");

    let mut project = YebanProjectV1 {
        id: id("Z1"),
        title: "sfz pan fixture".to_owned(),
        bpm: 120.0,
        rng_seed: 0x5945_4241_4E00_0255,
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
        pan,
        sampler: sampler.map(|path| SamplerBinding {
            path: path.display().to_string(),
        }),
        ..TrackV3::default()
    };
    track.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: 960,
            loop_config: LoopConfig::default(),
            muted: false,
        },
    );
    let mut notes = BTreeMap::new();
    notes.insert(note, MidiNote::new(note, 0, 60, 960));
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
        frames: 48_000,
        sample_rate: 48_000,
        channels: 2,
        // 32f ⇒ 落盘这一步不引入抖动；本判据只比左右比与位级恒等。
        bit_depth: BitDepth::Float32,
        use_engine_source: true,
        ..WavExportOptions::default()
    }
}

/// 交错 `f32` 的左右**峰值**（满刻度线性幅度）。
fn peaks(samples: &[f32]) -> (f64, f64) {
    let mut left = 0.0_f64;
    let mut right = 0.0_f64;
    for frame in samples.as_chunks::<2>().0 {
        left = left.max(f64::from(frame[0]).abs());
        right = right.max(f64::from(frame[1]).abs());
    }
    (left, right)
}

/// ⭐ 判据一／三／四：**两条采样器入口**都乘与 MIDI 路径**同一条**等功率曲线。
///
/// - `pan = -1` ⇒ 采样器两条入口的右声道都**逐位 0**、左声道 > 0（修前左右相等 ⇒ 本判据红）；
/// - `pan = -0.5` ⇒ 两条采样器入口的左右比 == MIDI 轨的左右比 == `tan(3π/8)`。
#[test]
fn the_cli_sampler_paths_pan_like_the_midi_path() {
    let fixture = Fixture::new("paths");
    let sfz_path = fixture.write_sfz();
    let (_, instrument) =
        load_sfz_with_instrument(&sfz_path, None, 48_000.0).expect("装载夹具 SFZ");
    let instrument = Arc::new(instrument);
    let expected = core::f64::consts::FRAC_PI_8.cos() / core::f64::consts::FRAC_PI_8.sin();

    for pan in [-1.0_f32, -0.5] {
        // ① 模型链条：`TrackV3.sampler` ⇒ 引擎 SFZ 分支 ⇒ `EngineTrackSource::with_pan`。
        let model = project(pan, Some(&sfz_path));
        let model_out = render_project_f32_with_assets(&model, options(), None, &[])
            .expect("模型链条必须能渲染");
        let (model_l, model_r) = peaks(&model_out.samples);

        // ② `--sfz` 旁路：整工程替换成 `SfzTrackSource` ⇒ `SfzTrackSource::with_pan`。
        let bypass = project(pan, None);
        let bypass_out = render_project_f32_with_assets(
            &bypass,
            options(),
            Some((Arc::clone(&instrument), Some(fixture.dir.clone()))),
            &[],
        )
        .expect("--sfz 旁路必须能渲染");
        let (bypass_l, bypass_r) = peaks(&bypass_out.samples);

        // ③ MIDI 振荡器路径（同一条轨、同一个 pan；`#251` 已接线）。
        let midi = project(pan, None);
        let midi_out = render_project_f32_with_assets(&midi, options(), None, &[])
            .expect("MIDI 路径必须能渲染");
        let (midi_l, midi_r) = peaks(&midi_out.samples);

        let model_ratio = model_l / model_r;
        let bypass_ratio = bypass_l / bypass_r;
        let midi_ratio = midi_l / midi_r;
        println!(
            "[#255/cli] pan={pan}: model L={model_l:.9} R={model_r:.9} ratio={model_ratio:.9}；\
             bypass L={bypass_l:.9} R={bypass_r:.9} ratio={bypass_ratio:.9}；\
             midi L={midi_l:.9} R={midi_r:.9} ratio={midi_ratio:.9}；\
             理论={expected:.9}；model-midi={:.3e} bypass-midi={:.3e}",
            (model_ratio - midi_ratio).abs(),
            (bypass_ratio - midi_ratio).abs()
        );

        // 两条采样器入口都必须真的出声（否则比值是 0/0 的空转）。
        assert!(
            model_l > 0.0,
            "pan={pan}: 模型链条左声道必须有信号：L={model_l}"
        );
        assert!(
            bypass_l > 0.0,
            "pan={pan}: --sfz 旁路左声道必须有信号：L={bypass_l}"
        );

        if pan == -1.0 {
            // ⭐ 满分离：右声道**逐位 0**（修前是"左右相同"）。
            assert_eq!(
                model_r.to_bits(),
                0.0_f64.to_bits(),
                "pan=-1: 模型链条的右声道必须逐位 0（修前左右相同）：R={model_r}"
            );
            assert_eq!(
                bypass_r.to_bits(),
                0.0_f64.to_bits(),
                "pan=-1: --sfz 旁路的右声道必须逐位 0（修前左右相同）：R={bypass_r}"
            );
            println!(
                "[#255/cli] pan=-1 满分离：model R(bits)={:#x} bypass R(bits)={:#x}",
                model_r.to_bits(),
                bypass_r.to_bits()
            );
        } else {
            // ⭐ 两个声道都必须有信号（否则比值是 0/0 的空转）。
            assert!(
                model_r > 0.0,
                "pan={pan}: 模型链条右声道必须有信号：R={model_r}"
            );
            assert!(
                bypass_r > 0.0,
                "pan={pan}: --sfz 旁路右声道必须有信号：R={bypass_r}"
            );
            // ⭐ 与 MIDI 路径同曲线。
            assert!(
                (model_ratio - midi_ratio).abs() < 1.0e-6,
                "pan={pan}: 模型链条必须与 MIDI 同曲线：{model_ratio:.9} vs {midi_ratio:.9}"
            );
            assert!(
                (bypass_ratio - midi_ratio).abs() < 1.0e-6,
                "pan={pan}: --sfz 旁路必须与 MIDI 同曲线：{bypass_ratio:.9} vs {midi_ratio:.9}"
            );
            assert!(
                (model_ratio - expected).abs() < 1.0e-6,
                "pan={pan}: 模型链条必须落在等功率曲线上：{model_ratio:.9} vs {expected:.9}"
            );
            assert!(
                (bypass_ratio - expected).abs() < 1.0e-6,
                "pan={pan}: --sfz 旁路必须落在等功率曲线上：{bypass_ratio:.9} vs {expected:.9}"
            );
        }
    }
}

/// ⭐ 判据五（正对照）：`pan = 0` 的采样器工程**逐位不变**。
///
/// 钉两件事：
/// - 左右声道的每个样本**逐位相同**（`pan = 0` 走 `(1.0, 1.0)` ⇒ 原来的单声道铺放分支）；
/// - 交错 `f32` 的位级摘要（`RenderProjectOutput::digest`）与**修前**同一夹具的摘要相同
///   —— 摘要由 `println!` 给出，⛔ 不写死成常量（台账第 313 条：WAV 绝对摘要与机器相关）。
#[test]
fn a_centered_sampler_keeps_the_pre_wiring_bytes() {
    let fixture = Fixture::new("center");
    let sfz_path = fixture.write_sfz();
    let project = project(0.0, Some(&sfz_path));
    let out = render_project_f32_with_assets(&project, options(), None, &[])
        .expect("pan = 0 的采样器工程必须能渲染");
    let (left, right) = peaks(&out.samples);
    assert!(left > 0.0, "正对照必须真的出声：peak={left}");
    assert_eq!(
        left.to_bits(),
        right.to_bits(),
        "pan = 0 ⇒ 左右峰值必须逐位相同：L={left} R={right}"
    );
    let mut differing = 0_usize;
    for frame in out.samples.as_chunks::<2>().0 {
        if frame[0].to_bits() != frame[1].to_bits() {
            differing += 1;
        }
    }
    println!(
        "[#255/cli] pan=0：L={left:.9} R={right:.9}；不同样本对数={differing}；\
         digest={}；frames={}；label={}",
        out.digest, out.frames, out.source_label
    );
    assert_eq!(
        differing, 0,
        "pan = 0 ⇒ 每个立体声帧的左右样本都必须逐位相同（⛔ 不乘 √2/2）"
    );

    // 同一工程再渲一次：摘要必须相同（排除"摘要里混进了不确定性"）。
    let again =
        render_project_f32_with_assets(&project, options(), None, &[]).expect("同工程第二次渲染");
    assert_eq!(out.digest, again.digest, "同一工程连渲两次摘要必须相同");
}
