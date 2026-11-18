//! 侧链**同层**轨→轨求值的音频级判据（逐件＋侧链线期 1，设计 §5.2 第 1 项）。
//!
//! `key_sidechain_audio.rs` 钉的是跨层键（kick 层 0 → bus 层 1）。本文件钉
//! 期 1 放宽后的同层情形：kick 轨与 pad 轨**同为层 0**，键边 kick → pad，
//! pad 轨自己的通道条压缩器被 kick 闪避。修前同层边一律不求值（收据计
//! 未求值），本判据在修前必红、修后必绿。
//!
//! 图（层号 0 / 1 / 2）：
//!
//! ```text
//!   kick ──(-80 dB)──► bus ──► master   ← kick 的音频边只为把它留在保留子图里
//!   pad  ────────────► bus              ← pad 是稳态 1 kHz 正弦（后半程才出声）
//!   kick ──Sidechain─► pad              ← 键：同层，驱动 pad 通道条的检波器
//! ```
//!
//! 隔离设计：kick 的节目在两渲之间逐位相同（同一源、同一音频边），pad 前半程
//! 静音 ⇒ 前半程两渲必须**逐位相同**（差值只许出现在 pad 的信号上）；后半程
//! 量 pad 的闪避深度与恢复。

use std::collections::{BTreeMap, BTreeSet};

use yeban_model::{
    DeviceDefinition, DeviceKind, EntityId, ParameterValue, RoutingEdge, RoutingGraph, RoutingKind,
};
use yeban_render::inserts::InsertChain;
use yeban_render::render::{AudioSource, BlockContext, RenderError, RenderOptions, RenderPlan};

const SR: u32 = 48_000;
const FRAMES: usize = 48_000;
/// pad 出声的起点（前半程静音，作逐位对照段）。
const PAD_START: usize = 24_000;

fn ulid(index: u32) -> EntityId {
    let mut text = [b'0'; 26];
    const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    for position in 0..4 {
        text[25 - position] = ALPHABET[((index >> (5 * position)) & 0x1F) as usize];
    }
    core::str::from_utf8(&text)
        .expect("ASCII")
        .parse()
        .expect("合法 ULID")
}

fn master() -> EntityId {
    ulid(0xFFFF)
}
fn pad() -> EntityId {
    ulid(1)
}
fn bus() -> EntityId {
    ulid(2)
}
fn kick() -> EntityId {
    ulid(3)
}
fn key_edge() -> EntityId {
    ulid(100)
}

fn param(name: &str, value: f32) -> ParameterValue {
    ParameterValue {
        name: name.to_owned(),
        value,
        unit: None,
    }
}

/// pad 轨上的通道条：阈值 −18 dB、8:1、硬膝、检波 10 ms、起振 1 ms、释放 30 ms。
/// 阈值刻意高于 pad 自身的 RMS（−23 dB）：无键对照里 pad 不过阈、GR = 0，
/// 间隔中段的差才只量「键的闪避有没有恢复」，而不是检波源不同带来的恒定
/// 偏移；键（60 Hz 爆发，RMS −4 dB）越阈约 14 dB，闪避约 12 dB。
/// 释放比 `key_sidechain_audio.rs` 的 bus 夹具（100 ms）快：本判据要在 kick
/// 间隔中段量到恢复 ≤ 1 dB，间隔只有 250 ms。
fn strip_device() -> DeviceDefinition {
    let params = [
        ("compressor_enabled", 1.0),
        ("threshold_db", -18.0),
        ("ratio", 8.0),
        ("knee_db", 0.0),
        ("detector_s", 0.01),
        ("attack_s", 0.001),
        ("release_s", 0.03),
        ("makeup_db", 0.0),
    ];
    DeviceDefinition {
        id: EntityId::new(),
        name: "CS · pad".to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params: params
            .iter()
            .map(|(name, value)| param(name, *value))
            .collect(),
        latency_samples: 0,
        plugin: None,
    }
}

fn graph() -> RoutingGraph {
    let mut edges = BTreeMap::new();
    let mut add = |id: EntityId,
                   source_node: EntityId,
                   destination_node: EntityId,
                   kind: RoutingKind,
                   gain_db: Option<f32>| {
        edges.insert(
            id,
            RoutingEdge {
                id,
                source_node,
                destination_node,
                kind,
                gain_db,
                source_lane: None,
                dest_lane: None,
            },
        );
    };
    add(ulid(10), pad(), bus(), RoutingKind::TrackToBus, None);
    // kick 只用来出键：到 bus 的音频边压到 −80 dB（幅度 9e-5，量级上不污染
    // pad 的 RMS 读数），否则它自己会盖住读数。
    add(
        ulid(11),
        kick(),
        bus(),
        RoutingKind::TrackToBus,
        Some(-80.0),
    );
    add(ulid(12), bus(), master(), RoutingKind::TrackToBus, None);
    add(key_edge(), kick(), pad(), RoutingKind::Sidechain, Some(0.0));
    RoutingGraph {
        nodes: vec![master(), pad(), bus(), kick()],
        edges,
    }
}

struct BufferSource {
    mono: Vec<f32>,
    position: usize,
}

impl BufferSource {
    fn new(mono: Vec<f32>) -> Self {
        Self { mono, position: 0 }
    }
}

impl AudioSource for BufferSource {
    fn render_block(&mut self, context: BlockContext, out: &mut [f32]) -> Result<(), RenderError> {
        for frame in 0..context.frames {
            let value = self.mono.get(self.position + frame).copied().unwrap_or(0.0);
            for channel in 0..context.channels {
                out[frame * context.channels + channel] = value;
            }
        }
        self.position += context.frames;
        Ok(())
    }
}

/// kick：每 12000 帧一次 41 ms 的 60 Hz 爆发（幅度 0.9），其余静音。
fn kick_buffer(frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|index| {
            let phase = index % 12_000;
            if phase >= 2_000 {
                return 0.0;
            }
            let t = 2.0 * core::f64::consts::PI * 60.0 * phase as f64 / f64::from(SR);
            (0.9 * t.sin()) as f32
        })
        .collect()
}

/// pad：前半程静音，后半程稳态 1 kHz 正弦（幅度 0.1）。
fn pad_buffer(frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|index| {
            if index < PAD_START {
                return 0.0;
            }
            let t = 2.0 * core::f64::consts::PI * 1000.0 * index as f64 / f64::from(SR);
            (0.1 * t.sin()) as f32
        })
        .collect()
}

/// 渲一遍。`keyed` = pad 已武装（键求值）；否则 pad 未武装（键声明了但不求值）。
/// 返回（母带样本， 已求值键边数）。
fn render(keyed: bool) -> (Vec<f32>, usize) {
    let routing = graph();
    let mut inserts = BTreeMap::new();
    inserts.insert(
        pad(),
        InsertChain::from_devices(&[strip_device()], SR, 120.0),
    );
    let options = RenderOptions::l1(FRAMES as u64, 2, SR, 0x155).with_threads(1);
    let mut plan = RenderPlan::compile_with_latencies_and_inserts(
        &routing,
        master(),
        options,
        &BTreeMap::new(),
        &inserts,
    )
    .expect("编译");
    // 前提自检：kick 与 pad 同层（层 0），bus 在下一层。
    let levels = plan.node_levels();
    assert_eq!(levels[&kick()], levels[&pad()], "kick 与 pad 必须同层");
    assert_eq!(
        levels[&bus()],
        levels[&pad()] + 1,
        "bus 必须在 pad 的下一层"
    );
    let armed: BTreeSet<EntityId> = if keyed {
        [pad()].into_iter().collect()
    } else {
        BTreeSet::new()
    };
    plan.attach_key_armed_nodes(&armed);
    let evaluated = plan.key_evaluated_edge_ids().len();
    let mut sources: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
    sources.insert(kick(), Box::new(BufferSource::new(kick_buffer(FRAMES))));
    sources.insert(pad(), Box::new(BufferSource::new(pad_buffer(FRAMES))));
    let samples = plan.execute(sources).expect("渲染").samples;
    (samples, evaluated)
}

fn rms_db(samples: &[f32], start: usize, end: usize) -> f64 {
    let mut sum = 0.0f64;
    for frame in start..end {
        for channel in 0..2 {
            let value = f64::from(samples[frame * 2 + channel]);
            sum += value * value;
        }
    }
    10.0 * (sum / ((end - start) as f64 * 2.0)).log10()
}

#[test]
fn same_layer_track_key_ducks_the_pad_track() {
    let (keyed, evaluated) = render(true);
    let (plain, plain_evaluated) = render(false);

    // 收据口径：声明 1 条、求值 1 条 ⇒ 未求值计数为 0
    // （MCP 的 sidechainEdgesDeclaredWithoutKeyInput = 声明 − 已求值）。
    assert_eq!(evaluated, 1, "同层键边必须求值（修前这里是 0）");
    assert_eq!(plain_evaluated, 0, "未武装时键边不得求值");

    // 差值只许出现在 pad 的信号上：pad 静音的前半程，两渲逐位相同
    // （kick 的节目、bus 的汇合在两渲之间没有任何理由不同）。
    for (index, (a, b)) in keyed[..PAD_START * 2]
        .iter()
        .zip(plain[..PAD_START * 2].iter())
        .enumerate()
    {
        assert_eq!(
            a.to_bits(),
            b.to_bits(),
            "前半程第 {index} 个样本必须逐位相同（键只许改 pad 的增益）"
        );
    }

    // pad 出声后的两次 kick（24000、36000）：起音后 10–30 ms 内 pad 被压 ≥ 6 dB；
    // 间隔中段（起音后 125–175 ms，释放 30 ms 已收敛）恢复到 ≤ 1 dB。
    for onset in [24_000usize, 36_000] {
        let duck =
            rms_db(&keyed, onset + 480, onset + 1_440) - rms_db(&plain, onset + 480, onset + 1_440);
        println!("起音 {onset}：闪避深度 {duck:.2} dB");
        assert!(
            duck <= -6.0,
            "起音 {onset} 后 30 ms 内 pad 必须被压 6 dB 以上，实测 {duck:.2} dB"
        );
        let recovery = rms_db(&keyed, onset + 6_000, onset + 8_400)
            - rms_db(&plain, onset + 6_000, onset + 8_400);
        println!("起音 {onset}：间隔中段残差 {recovery:.2} dB");
        assert!(
            recovery.abs() <= 1.0,
            "起音 {onset} 的间隔中段 pad 必须恢复到 1 dB 以内，实测 {recovery:.2} dB"
        );
    }
}
