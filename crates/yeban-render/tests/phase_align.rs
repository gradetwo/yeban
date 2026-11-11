//! **手动相位对齐第一层**的离线判据：每轨延时＋极性反转（`RenderPlan` 末端）。
//!
//! 量的是**导出母带的样本**，不是"投影出来的参数"：
//!
//! | 判据 | 已知答案 |
//! | --- | --- |
//! | 延时 | 脉冲峰值位置精确移动 N 采样（`left[240] == 1.0`，之前全 0） |
//! | 极性反转 | 信号逐样本取反（`left[0] == -1.0`） |
//! | 负延时 | 相对提前：−240 的轨在 0 帧出声，0 的轨在 240 帧出声 |
//! | 恒等 | 有相位表（全恒等）与无相位表逐位相同 |
//! | 复合 | 延时＋反转同时生效（`left[240] == -1.0`） |
//!
//! 插入点在 `render_node` 的最后一级（插入链＋输出级之后、下游读节点
//! buffer 之前），因此判据用**整条离线链**（`compile_*_and_phase` →
//! `execute`）验证，不对 `DelayLine` 单测。

use std::collections::BTreeMap;

use yeban_model::{EntityId, RoutingEdge, RoutingGraph, RoutingKind, TrackV3};
use yeban_render::render::{
    AudioSource, BlockContext, PhaseAlign, RenderError, RenderOptions, RenderOutput, RenderPlan,
    project_phase_align,
};

/// 判据统一用 48 kHz。
const SR: u32 = 48_000;

/// 一帧脉冲：`[1.0, 0.0, …]`（峰值位置就是答案本身）。
const FRAMES: usize = 4_096;

/// 判据统一用的延时：240 采样 = 5 ms @ 48 kHz。
const DELAY: i32 = 240;

fn node(seed: u32) -> EntityId {
    let mut text = [b'0'; 26];
    const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    for position in 0..4 {
        text[25 - position] = ALPHABET[((seed >> (5 * position)) & 0x1F) as usize];
    }
    core::str::from_utf8(&text)
        .expect("ASCII")
        .parse()
        .expect("合法 ULID")
}

fn graph(nodes: &[EntityId], edges: &[(EntityId, EntityId, RoutingKind)]) -> RoutingGraph {
    let mut map = BTreeMap::new();
    for &(source_node, destination_node, kind) in edges {
        let id = EntityId::new();
        map.insert(
            id,
            RoutingEdge {
                id,
                source_node,
                destination_node,
                kind,
                gain_db: None,
            },
        );
    }
    RoutingGraph {
        nodes: nodes.to_vec(),
        edges: map,
    }
}

/// 把一条单声道缓冲复制到各声道的源。
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

/// 渲染一张"源节点 → master"图，返回母带。`sources` 按节点给单声道缓冲。
fn render(
    routing: &RoutingGraph,
    master: EntityId,
    phase: &BTreeMap<EntityId, PhaseAlign>,
    sources: BTreeMap<EntityId, Vec<f32>>,
) -> RenderOutput {
    let options = RenderOptions::l1(FRAMES as u64, 2, SR, 0x155).with_threads(1);
    let mut plan = RenderPlan::compile_with_latencies_and_inserts_and_phase(
        routing,
        master,
        options,
        &BTreeMap::new(),
        &BTreeMap::new(),
        phase,
    )
    .expect("编译");
    let sources = sources
        .into_iter()
        .map(|(node, mono)| {
            (
                node,
                Box::new(BufferSource::new(mono)) as Box<dyn AudioSource>,
            )
        })
        .collect();
    plan.execute(sources).expect("渲染")
}

/// 单帧脉冲（左＝右＝1.0，只在第 0 帧）。
fn pulse() -> Vec<f32> {
    let mut mono = vec![0.0f32; FRAMES];
    mono[0] = 1.0;
    mono
}

/// 左声道在帧 `frame` 的值。
fn left(samples: &[f32], frame: usize) -> f32 {
    samples[frame * 2]
}

/// 判据 1：延时 N 采样 ⇒ 脉冲峰值精确移动 N 采样。
#[test]
fn track_delay_shifts_impulse_by_exact_samples() {
    let master = node(0xFF);
    let track_node = node(1);
    let routing = graph(
        &[master, track_node],
        &[(track_node, master, RoutingKind::TrackToBus)],
    );
    let mut sources = BTreeMap::new();
    sources.insert(track_node, pulse());

    let delayed = render(
        &routing,
        master,
        &BTreeMap::from([(
            track_node,
            PhaseAlign {
                delay_samples: DELAY,
                invert_polarity: false,
            },
        )]),
        sources.clone(),
    );
    let plain = render(&routing, master, &BTreeMap::new(), sources);

    assert_eq!(left(&plain.samples, 0), 1.0, "无延时基线：第 0 帧就是脉冲");
    for frame in 0..DELAY as usize {
        assert_eq!(
            left(&delayed.samples, frame),
            0.0,
            "延时 {DELAY} 采样：前 {DELAY} 帧必须全静音（帧 {frame}）"
        );
    }
    assert_eq!(
        left(&delayed.samples, DELAY as usize),
        1.0,
        "延时 {DELAY} 采样：脉冲必须精确落在第 {DELAY} 帧"
    );
    for frame in (DELAY as usize + 1)..FRAMES {
        assert_eq!(
            left(&delayed.samples, frame),
            0.0,
            "脉冲之后必须静音（帧 {frame}）"
        );
    }
}

/// 判据 2：极性反转 ⇒ 信号逐样本取反。
#[test]
fn invert_polarity_negates_signal() {
    let master = node(0xFF);
    let track_node = node(1);
    let routing = graph(
        &[master, track_node],
        &[(track_node, master, RoutingKind::TrackToBus)],
    );
    let mut sources = BTreeMap::new();
    sources.insert(track_node, pulse());

    let inverted = render(
        &routing,
        master,
        &BTreeMap::from([(
            track_node,
            PhaseAlign {
                delay_samples: 0,
                invert_polarity: true,
            },
        )]),
        sources.clone(),
    );
    let plain = render(&routing, master, &BTreeMap::new(), sources);

    assert_eq!(left(&plain.samples, 0), 1.0, "基线：第 0 帧是 +1.0");
    assert_eq!(
        left(&inverted.samples, 0),
        -1.0,
        "反转后：第 0 帧必须是 −1.0"
    );
    for frame in 1..FRAMES {
        assert_eq!(
            left(&inverted.samples, frame),
            -left(&plain.samples, frame),
            "反转后每帧都必须取反（帧 {frame}）"
        );
    }
}

/// 判据 3：负延时 = 相对提前（最小值平移）。
///
/// A −240、B 0 ⇒ min = −240 ⇒ A 有效 0、B 有效 240：A 的脉冲在第 0 帧，
/// B 的脉冲在第 240 帧，两轨相对偏移精确保持 240 采样。
#[test]
fn negative_delay_advances_relatively() {
    let master = node(0xFF);
    let track_a = node(1);
    let track_b = node(2);
    let routing = graph(
        &[master, track_a, track_b],
        &[
            (track_a, master, RoutingKind::TrackToBus),
            (track_b, master, RoutingKind::TrackToBus),
        ],
    );
    let mut sources = BTreeMap::new();
    sources.insert(track_a, pulse());
    sources.insert(track_b, pulse());

    let rendered = render(
        &routing,
        master,
        &BTreeMap::from([
            (
                track_a,
                PhaseAlign {
                    delay_samples: -DELAY,
                    invert_polarity: false,
                },
            ),
            (
                track_b,
                PhaseAlign {
                    delay_samples: 0,
                    invert_polarity: false,
                },
            ),
        ]),
        sources,
    );

    assert_eq!(
        left(&rendered.samples, 0),
        1.0,
        "−240 的轨（有效 0）：第 0 帧只有 A 的脉冲"
    );
    for frame in 1..DELAY as usize {
        assert_eq!(
            left(&rendered.samples, frame),
            0.0,
            "中间帧必须静音（帧 {frame}）"
        );
    }
    assert_eq!(
        left(&rendered.samples, DELAY as usize),
        1.0,
        "0 的轨（有效 240）：脉冲必须精确落在第 {DELAY} 帧"
    );
}

/// 判据 4：恒等相位表（全 0、不反转）与无相位表逐位相同。
#[test]
fn identity_phase_is_bitwise_identical() {
    let master = node(0xFF);
    let track_node = node(1);
    let routing = graph(
        &[master, track_node],
        &[(track_node, master, RoutingKind::TrackToBus)],
    );
    // 非平凡信号：正弦突发（恒等路径一个样本都不许碰）。
    let mono: Vec<f32> = (0..FRAMES)
        .map(|index| {
            (0.5 * (2.0 * core::f64::consts::PI * 440.0 * index as f64 / f64::from(SR)).sin())
                as f32
        })
        .collect();
    let mut sources = BTreeMap::new();
    sources.insert(track_node, mono);

    let with_identity = render(
        &routing,
        master,
        &BTreeMap::from([(
            track_node,
            PhaseAlign {
                delay_samples: 0,
                invert_polarity: false,
            },
        )]),
        sources.clone(),
    );
    let plain = render(&routing, master, &BTreeMap::new(), sources);

    assert_eq!(
        with_identity.samples, plain.samples,
        "恒等相位表必须与无相位表逐位相同"
    );
}

/// 判据 5：延时＋反转复合。
#[test]
fn delay_and_invert_compose() {
    let master = node(0xFF);
    let track_node = node(1);
    let routing = graph(
        &[master, track_node],
        &[(track_node, master, RoutingKind::TrackToBus)],
    );
    let mut sources = BTreeMap::new();
    sources.insert(track_node, pulse());

    let rendered = render(
        &routing,
        master,
        &BTreeMap::from([(
            track_node,
            PhaseAlign {
                delay_samples: DELAY,
                invert_polarity: true,
            },
        )]),
        sources,
    );

    for frame in 0..DELAY as usize {
        assert_eq!(left(&rendered.samples, frame), 0.0, "延时段必须静音");
    }
    assert_eq!(
        left(&rendered.samples, DELAY as usize),
        -1.0,
        "第 {DELAY} 帧必须是反转后的脉冲 −1.0"
    );
}

/// 判据 6：`project_phase_align` 只收非恒等轨。
#[test]
fn project_phase_align_skips_identity_tracks() {
    let plain_track = node(1);
    let delayed_track = node(2);
    let inverted_track = node(3);
    let mut tracks = BTreeMap::new();
    tracks.insert(plain_track, TrackV3::default());
    tracks.insert(
        delayed_track,
        TrackV3 {
            delay_samples: Some(240),
            invert_polarity: None,
            ..TrackV3::default()
        },
    );
    tracks.insert(
        inverted_track,
        TrackV3 {
            delay_samples: None,
            invert_polarity: Some(true),
            ..TrackV3::default()
        },
    );

    let phase = project_phase_align(&tracks);
    assert_eq!(phase.len(), 2, "恒等轨不进表");
    assert_eq!(
        phase[&delayed_track],
        PhaseAlign {
            delay_samples: 240,
            invert_polarity: false,
        }
    );
    assert_eq!(
        phase[&inverted_track],
        PhaseAlign {
            delay_samples: 0,
            invert_polarity: true,
        }
    );
}
