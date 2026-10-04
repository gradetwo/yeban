//! `[BASELINE-001]` 的**离线渲染吞吐**测量。
//!
//! 规范 `docs/YEBAN_INDUSTRY_BENCHMARK_AND_OPEN_SOURCE_STUDY.md` 的 `BASELINE-001` 要求
//! "离线渲染 ≥ 100× 实时"（参考工程 A：32 轨 → 母线的星形路由）。
//! 本仓库此前**没有**任何基准，因此该目标一直以 `PENDING` 挂在账本上
//! （`crates/yeban-render/src/lib.rs` 的 crate 文档里也如实写着"未被本分支证实"）。
//!
//! ## 为什么是一个 example 而不是 `benches/` + criterion
//!
//! 1. **零新增依赖**：`criterion` 会拉进 `plotters` / `tinytemplate` 等一整棵树，
//!    而这里只需要"跑一遍、量时间、报数字"；依赖政策（ADR-0001 D5/D20/D21）要求新增包
//!    必须逐条裁决，收益不抵成本。
//! 2. **`#[test]` 里量不出有意义的数**：判据跑在 debug 断言与并行测试里；
//!    基准必须用 `--release` 且独占机器。
//! 3. **可被门禁直接调用**：`gates-manual.yml` 的 `bench` 档跑
//!    `cargo run --release -p yeban-render --example bench_render`，
//!    把输出写进 job summary —— 于是"测量"这件事有了固定入口。
//!
//! ## 读数怎么用（重要，避免把噪声当结论）
//!
//! GitHub 托管 runner 的 CPU 是**共享且型号不定**的，读数只能给**数量级**，
//! 不能作为"达标/不达标"的判定（`BASELINE-005` 的硬件往返时延更是完全无法在此测量）。
//! 因此本程序**只报告数字，不自己下结论**；结论写进 `docs/DEVELOPMENT_LEDGER.md`，
//! 并明确标注运行环境。要作为验收证据，必须在**指定的参考机器**上复跑同一命令。
//!
//! ## 用法
//!
//! ```text
//! cargo run --release -p yeban-render --example bench_render            # 默认 32 轨 / 30 秒
//! cargo run --release -p yeban-render --example bench_render -- 8 10    # 8 轨 / 10 秒
//! ```

use std::collections::BTreeMap;
use std::str::FromStr;
use std::time::Instant;

use yeban_model::{EntityId, RoutingEdge, RoutingGraph, RoutingKind};
use yeban_render::render::{AudioSource, BlockContext, RenderError, RenderOptions, RenderPlan};

/// Crockford Base32 字母表（与 `yeban-model` 的手写 ULID 编解码一致）。
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// 造一个合法的 ULID 文本（与仓库其它判据同一手法，避免依赖随机数）。
fn ulid(index: u32) -> EntityId {
    let mut text = [b'0'; 26];
    for position in 0..4 {
        text[25 - position] = CROCKFORD[((index >> (5 * position)) & 0x1F) as usize];
    }
    EntityId::from_str(core::str::from_utf8(&text).expect("ASCII")).expect("合法 ULID")
}

/// `splitmix64`：固定种子的确定性 PRNG（`[ARCH-DET-001]` 要求渲染不引入真熵源）。
const fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// 一个便宜但**不是常量**的样本源：每块做一点浮点运算，避免"常量填充被优化成 memset"
/// 让读数虚高。相位用整数帧号推进，避免浮点累加漂移。
struct ToneSource {
    seed: u64,
    phase: u64,
    step: u64,
}

impl ToneSource {
    fn new(seed: u64, step: u64) -> Self {
        Self {
            seed,
            phase: 0,
            step,
        }
    }
}

impl AudioSource for ToneSource {
    fn render_block(&mut self, _context: BlockContext, out: &mut [f32]) -> Result<(), RenderError> {
        for (index, sample) in out.iter_mut().enumerate() {
            // 每 8 个样本抖一次相位: 足够便宜, 又不是"整块同一个值"。
            if index.is_multiple_of(8) {
                self.phase = self.phase.wrapping_add(self.step);
            }
            let noise = (next(&mut self.seed) >> 40) as f32 / 16_777_216.0 - 0.5;
            let carrier = ((self.phase % 480) as f32 / 480.0) - 0.5;
            *sample = carrier * 0.5 + noise * 0.001;
        }
        Ok(())
    }
}

/// 参考工程 A 的形状：`tracks` 条轨道 → 一条母线（星形），母线即 master。
fn reference_project(tracks: u32) -> (RoutingGraph, EntityId, Vec<EntityId>) {
    let master = ulid(0xFFFF);
    let mut nodes = vec![master];
    let mut edges = BTreeMap::new();
    let mut sources = Vec::new();
    for index in 1..=tracks {
        let track = ulid(index);
        nodes.push(track);
        sources.push(track);
        let edge = RoutingEdge {
            id: ulid(0x8000 - index),
            source_node: track,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        };
        edges.insert(edge.id, edge);
    }
    (RoutingGraph { nodes, edges }, master, sources)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let tracks: u32 = args.next().map_or(32, |v| v.parse().expect("轨数"));
    let seconds: u64 = args.next().map_or(30, |v| v.parse().expect("秒数"));

    const SAMPLE_RATE: u32 = 48_000;
    const CHANNELS: usize = 2;

    let frames = seconds * u64::from(SAMPLE_RATE);
    let audio_seconds = frames as f64 / f64::from(SAMPLE_RATE);

    // 两个数据点: 单线程 与 Rayon 默认线程数。后者才是 `BASELINE-001` 关心的口径
    // （规范要的是"用满机器的离线渲染"), 前者用来暴露并行加速比。
    for threads in [Some(1usize), None] {
        let (graph, master, source_nodes) = reference_project(tracks);
        let options = RenderOptions {
            threads,
            ..RenderOptions::l1(frames, CHANNELS, SAMPLE_RATE, 0x5EED)
        };

        let mut plan = RenderPlan::compile(&graph, master, options).expect("图应可编译");
        let mut sources: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
        for (index, node) in source_nodes.iter().enumerate() {
            // 每条轨道给不同的相位步长, 让各轨的运算不完全相同(避免分支预测过于乐观)。
            let step = 1 + (index as u64 % 97);
            sources.insert(
                *node,
                Box::new(ToneSource::new(0x1234_5678 + index as u64, step)),
            );
        }

        let started = Instant::now();
        let output = plan.execute(sources).expect("渲染应成功");
        let wall = started.elapsed();

        let wall_seconds = wall.as_secs_f64();
        let realtime_x = if wall_seconds > 0.0 {
            audio_seconds / wall_seconds
        } else {
            f64::INFINITY
        };
        let digest = output
            .digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let label = match threads {
            Some(n) => n.to_string(),
            None => "auto".to_owned(),
        };

        // 机器可读的一行: 门禁把它原样写进 job summary, 人再抄进账本。
        println!(
            "BENCH baseline=001 name=render_reference_project_a tracks={tracks} \
             frames={frames} audio_seconds={audio_seconds:.3} channels={CHANNELS} \
             sample_rate={SAMPLE_RATE} threads={label} wall_ms={} realtime_x={realtime_x:.1} \
             longest_path_frames={} blocks={} digest={digest}",
            wall.as_millis(),
            output.longest_path_frames,
            output.blocks,
        );
    }

    // 参考工程的**工程侧**读数: 只打印, 不判断 —— 阈值判定属人类/账本。
    println!(
        "BENCH baseline=001 note=\"托管 runner 读数只给数量级; 达标判定需在指定参考机器上复跑\""
    );
}
