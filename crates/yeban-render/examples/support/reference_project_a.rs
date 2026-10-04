//! **参考工程 A 的唯一构造**（`BASELINE-001` 性能基准与 `MUST-GATE-002/003`
//! 确定性读数的**共用夹具**）。
//!
//! ## 为什么把它抽出来（而不是各写一份）
//!
//! `examples/bench_render.rs` 与 `examples/export_l1_receipt.rs` 必须读**同一个**
//! 参考工程 A：否则"基准工程的读数"与"确定性门禁的读数"是两个不同的东西，
//! 而 `MUST-GATE-002/003` 的对账就失去意义。夹具（图构造 + 样本源算法）因此只有
//! 这一份，两个 example 都用 `#[path]` 引入它 —— `bench_render.rs` 与本文件
//! **不能漂移**，因为它们就是同一段代码。
//!
//! 本文件**不是** cargo 目标：`examples/` 下只有 `*.rs` 与 `*/main.rs` 会被自动发现，
//! `examples/support/` 里没有 `main.rs`，因此不成为 example（已实测 `cargo metadata`）。
//!
//! ## 渲染路径（刻意与基准同源）
//!
//! 本文件只提供**夹具**（`RoutingGraph` + `AudioSource` 实现）。真正的渲染一律走
//! `yeban_render::render::RenderPlan::compile*` / `execute` 与
//! `RenderOutput::digest_of` —— 本线**没有**、也不会另造一条渲染路径。
//!
//! ## 样本源为什么是"便宜的确定性伪随机 + 方波载波"
//!
//! 与 `bench_render.rs` 的原始理由一致：常量填充会被优化成 `memset`，让吞吐读数虚高；
//! 相位用整数帧号推进，避免浮点累加漂移（[ARCH-DET-001] 要求渲染不引入真熵源）。

use std::collections::BTreeMap;
use std::str::FromStr as _;

use yeban_model::{EntityId, RoutingEdge, RoutingGraph, RoutingKind};
use yeban_render::render::{AudioSource, BlockContext, RenderError};

/// Crockford Base32 字母表（与 `yeban-model` 的手写 ULID 编解码一致）。
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// 造一个合法的 ULID 文本（与仓库其它判据同一手法，避免依赖随机数）。
///
/// 末 4 位按 5 位一组大端编码 `index`，因此 `ulid(a) < ulid(b)` 当且仅当 `a < b`。
pub fn ulid(index: u32) -> EntityId {
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

/// 参考工程 A 的样本源：每块做一点浮点运算，避免"常量填充被优化成 memset"。
///
/// 路径上的**全部**运算都属于 ADR-0001 **D32 的 IEEE 精确类**：
/// `add` / `sub` / `mul` / `div` / 整数→`f32` 转换（转换是 IEEE 正确舍入的）。
/// 没有任何超越函数 ⇒ 若 D32 成立，跨架构应当**逐位相同**。
pub struct ToneSource {
    seed: u64,
    phase: u64,
    step: u64,
}

impl ToneSource {
    /// 用固定种子与固定相位步长造一条轨道的样本源。
    pub fn new(seed: u64, step: u64) -> Self {
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
///
/// `gain_db` 直接写进每条边的 `RoutingEdge::gain_db`（`None` = 字面形态）。
/// 注意 `None` 与 `Some(0.0)` **不等价**：`render::db_to_linear` 对 `None` 走
/// "直接 1.0"的分支，而 `Some(0.0)` 虽然也返回 `1.0`，但那是 `libm::powf` 的结果 ——
/// 本线刻意让收据记录 `EdgeGain` 而不是 `Option<f32>`，就是为了不让这两者混为一谈。
pub fn reference_project(
    tracks: u32,
    gain_db: Option<f32>,
) -> (RoutingGraph, EntityId, Vec<EntityId>) {
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
            gain_db,
        };
        edges.insert(edge.id, edge);
    }
    (RoutingGraph { nodes, edges }, master, sources)
}

/// 参考工程 A 的**确定性**样本源装配：第 `index` 条轨道给不同的相位步长，
/// 让各轨的运算不完全相同（避免分支预测过于乐观）。种子固定。
#[must_use]
pub fn tone_sources(source_nodes: &[EntityId]) -> BTreeMap<EntityId, Box<dyn AudioSource>> {
    let mut sources: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
    for (index, node) in source_nodes.iter().enumerate() {
        let step = 1 + (index as u64 % 97);
        sources.insert(
            *node,
            Box::new(ToneSource::new(0x1234_5678 + index as u64, step)),
        );
    }
    sources
}
