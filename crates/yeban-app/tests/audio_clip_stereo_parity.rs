//! T-FIX-574：同一工程的立体声音频片段，实时走带与离线导出必须保持同一份
//! 左右声道（逐声道 RMS 与相关系数一致），⛔ 不再把 `(L+R)/2` 复制给两边。
//!
//! 三组素材：L-only、R-only、反相（L=+0.5 / R=−0.5）。实时走产品装配点
//! `ProductionLoop::start_with_audio_source`；离线走 `--export-wav` 生产入口。
//! 轨声相设为非零（−0.5），让两条路径都走同一条等功率曲线；`pan == 0` 的
//! 离线快路径与实时居中律是既有差异，不属本单。

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use yeban_app::cli::{self, run_batch};
use yeban_app::host::ProductionLoop;
use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
use yeban_model::container::write_project_container;
use yeban_model::{
    AssetHash, ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, RoutingEdge,
    RoutingGraph, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::{read_plain_wav, write_plain_wav};

const RATE: u32 = 48_000;
const BPM: f64 = 120.0;
const START_TICK: u64 = 4_800;
const DURATION_TICKS: u64 = 4_800;
const ASSET_FRAMES: usize = 24_000;
const PAN: f32 = -0.5;
const I24_SCALE: f32 = 8_388_608.0;
const WINDOW_START: usize = 120_000;
const WINDOW_END: usize = 132_000;
const QUANTA: usize = 1_200;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-stereo-parity-{tag}-{}-{}",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

fn stereo_wav(dir: &Path, tag: &str, left: i16, right: i16) -> Vec<u8> {
    let path = dir.join(format!("{tag}.wav"));
    let samples: Vec<i16> = (0..ASSET_FRAMES).flat_map(|_| [left, right]).collect();
    write_plain_wav(
        &path,
        &PcmFormat::integer(2, RATE, 16),
        &PcmBuffer::Int16(samples),
    )
    .expect("写立体声夹具 WAV");
    std::fs::read(&path).expect("读立体声夹具 WAV")
}

fn project_with_audio(asset: AssetHash, pan: f32) -> YebanProjectV1 {
    let master = EntityId::new();
    let track = EntityId::new();
    let clip = EntityId::new();
    let placement = EntityId::new();
    let mut tracks = BTreeMap::new();
    tracks.insert(
        track,
        TrackV3 {
            id: track,
            name: "Audio".to_owned(),
            kind: TrackKind::Audio,
            pan,
            ..TrackV3::default()
        },
    );
    tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    let mut clip_pool = BTreeMap::new();
    clip_pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "take".to_owned(),
            content: ClipContent::Audio {
                asset,
                gain_db: 0.0,
            },
        },
    );
    let mut routing = RoutingGraph {
        nodes: vec![track, master],
        ..RoutingGraph::default()
    };
    let edge = EntityId::new();
    routing.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: track,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
            source_lane: None,
            dest_lane: None,
        },
    );
    let mut placements = BTreeMap::new();
    placements.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: START_TICK,
            duration_ticks: DURATION_TICKS,
            loop_config: LoopConfig::default(),
            muted: false,
            offset_samples: None,
            fade_in_ticks: None,
            fade_out_ticks: None,
            source_start_samples: None,
        },
    );
    let mut project = YebanProjectV1 {
        bpm: BPM,
        master_bus_track_id: master,
        tracks,
        routing_graph: routing,
        clip_pool,
        ..YebanProjectV1::default()
    };
    project.tracks.get_mut(&track).expect("音频轨在表里").clips = placements;
    project
}

fn pool_source(
    pool: Rc<RefCell<BTreeMap<AssetHash, Vec<u8>>>>,
) -> yeban_app::audio_inject::AudioAssetSource {
    Box::new(move |hash| pool.borrow().get(hash).cloned())
}

fn realtime_channels(
    project: &YebanProjectV1,
    hash: AssetHash,
    bytes: Vec<u8>,
) -> (Vec<f32>, Vec<f32>) {
    let pool: Rc<RefCell<BTreeMap<AssetHash, Vec<u8>>>> = Rc::new(RefCell::new(BTreeMap::new()));
    pool.borrow_mut().insert(hash, bytes);
    let production =
        ProductionLoop::start_with_audio_source(project, 0, Some(pool_source(Rc::clone(&pool))))
            .expect("产品装配点必须建起一代引擎");
    let engine = production.engine_handle();
    engine.borrow_mut().play();
    let channels = engine
        .borrow()
        .current_snapshot()
        .map_or(2, |snapshot| snapshot.channels().max(1));
    assert_eq!(channels, 2, "本判据要求立体声母线");
    let mut left = Vec::with_capacity(QUANTA * DEFAULT_BLOCK_FRAMES);
    let mut right = Vec::with_capacity(QUANTA * DEFAULT_BLOCK_FRAMES);
    let mut block = vec![0.0_f32; DEFAULT_BLOCK_FRAMES * 2];
    let mut host = engine.borrow_mut();
    let runtime = host
        .runtime_for_criteria()
        .expect("刚 reload 过的宿主必须有控制线程可用的运行时");
    for _ in 0..QUANTA {
        block.fill(0.0);
        runtime.process_quantum(&mut block, 2);
        for [left_sample, right_sample] in block.as_chunks::<2>().0 {
            left.push(*left_sample);
            right.push(*right_sample);
        }
    }
    (left, right)
}

fn offline_channels(
    dir: &Path,
    tag: &str,
    project: &YebanProjectV1,
    bytes: &[u8],
) -> (Vec<f32>, Vec<f32>) {
    let hash = AssetHash::of_bytes(bytes);
    let mut assets = BTreeMap::new();
    assets.insert(hash, bytes.to_vec());
    let container = write_project_container(project, b"", &assets).expect("写容器");
    let container_path = dir.join(format!("{tag}.yeban"));
    std::fs::write(&container_path, &container).expect("落容器文件");
    let out = dir.join(format!("{tag}.wav"));
    let options = cli::parse(&[
        "--headless".to_owned(),
        "--open".to_owned(),
        container_path.display().to_string(),
        "--export-wav".to_owned(),
        out.display().to_string(),
        "--wav-seconds".to_owned(),
        "8".to_owned(),
    ])
    .expect("命令行必须可解析");
    run_batch(&options).expect("生产导出入口必须成功");
    let (format, buffer) = read_plain_wav(&out).expect("导出 WAV 必须可回读");
    assert_eq!(format.channels, 2, "离线导出必须是立体声");
    let interleaved: Vec<f32> = match &buffer {
        PcmBuffer::Int24(values) => values.iter().map(|v| *v as f32 / I24_SCALE).collect(),
        other => panic!("默认导出必须是 24-bit 整数: {other:?}"),
    };
    let mut left = Vec::with_capacity(interleaved.len() / 2);
    let mut right = Vec::with_capacity(interleaved.len() / 2);
    for [left_sample, right_sample] in interleaved.as_chunks::<2>().0 {
        left.push(*left_sample);
        right.push(*right_sample);
    }
    (left, right)
}

fn rms(samples: &[f32], range: core::ops::Range<usize>) -> f32 {
    let sum: f64 = samples[range]
        .iter()
        .map(|sample| f64::from(*sample) * f64::from(*sample))
        .sum();
    (sum / (WINDOW_END - WINDOW_START) as f64).sqrt() as f32
}

fn correlation(left: &[f32], right: &[f32]) -> f32 {
    let dot: f64 = left[WINDOW_START..WINDOW_END]
        .iter()
        .zip(&right[WINDOW_START..WINDOW_END])
        .map(|(a, b)| f64::from(*a) * f64::from(*b))
        .sum();
    let norm_l: f64 = left[WINDOW_START..WINDOW_END]
        .iter()
        .map(|a| f64::from(*a) * f64::from(*a))
        .sum();
    let norm_r: f64 = right[WINDOW_START..WINDOW_END]
        .iter()
        .map(|b| f64::from(*b) * f64::from(*b))
        .sum();
    (dot / (norm_l.sqrt() * norm_r.sqrt())) as f32
}

#[test]
fn realtime_and_offline_agree_on_stereo_clip_channels() {
    let dir = scratch("parity");
    let cases: [(&str, i16, i16, f32); 3] = [
        ("left-only", 8_192, 0, 0.0),
        ("right-only", 0, 8_192, 0.0),
        ("inverse", 8_192, -8_192, -1.0),
    ];
    for (tag, left_i16, right_i16, expected_corr) in cases {
        let bytes = stereo_wav(&dir, tag, left_i16, right_i16);
        let hash = AssetHash::of_bytes(&bytes);
        let project = project_with_audio(hash.clone(), PAN);
        let realtime = realtime_channels(&project, hash, bytes.clone());
        let offline = offline_channels(&dir, tag, &project, &bytes);
        let rt_l = rms(&realtime.0, WINDOW_START..WINDOW_END);
        let rt_r = rms(&realtime.1, WINDOW_START..WINDOW_END);
        let of_l = rms(&offline.0, WINDOW_START..WINDOW_END);
        let of_r = rms(&offline.1, WINDOW_START..WINDOW_END);
        let rt_corr = correlation(&realtime.0, &realtime.1);
        let of_corr = correlation(&offline.0, &offline.1);
        println!(
            "[T-FIX-574/{tag}] 实时 RMS L/R={rt_l:.6}/{rt_r:.6} 离线={of_l:.6}/{of_r:.6} \
             corr 实时={rt_corr:.6} 离线={of_corr:.6}"
        );
        assert!(
            (rt_l - of_l).abs() <= 1e-4 + 0.01 * of_l.max(1e-6),
            "{tag}: 左声道 RMS 实时 {rt_l} vs 离线 {of_l}"
        );
        assert!(
            (rt_r - of_r).abs() <= 1e-4 + 0.01 * of_r.max(1e-6),
            "{tag}: 右声道 RMS 实时 {rt_r} vs 离线 {of_r}"
        );
        if left_i16 == 0 || right_i16 == 0 {
            if left_i16 == 0 {
                assert!(rt_l.abs() < 1e-6, "{tag}: 实时左声道必须静音，实际 {rt_l}");
            }
            if right_i16 == 0 {
                assert!(rt_r.abs() < 1e-6, "{tag}: 实时右声道必须静音，实际 {rt_r}");
            }
            // 单边为 0 时相关系数没有定义；离线 24-bit 抖动会留下 1e-8 量级噪声。
            continue;
        }
        assert!(
            (rt_corr - of_corr).abs() <= 2e-3,
            "{tag}: 相关系数实时 {rt_corr} vs 离线 {of_corr}"
        );
        assert!(
            (of_corr - expected_corr).abs() <= 2e-3,
            "{tag}: 离线相关系数 {of_corr}，期望 {expected_corr}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}
