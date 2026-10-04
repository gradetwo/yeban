//! # yeban-render — 离线渲染与导出
//!
//! Rayon 多核并行离线母带渲染器、RF64/BW64 + BEXT 自研写入器、TPDF 抖动与
//! MIDI 0/1 导出。**完全离线、无 GUI、无实时约束**: 这一层可以任意分配、任意阻塞,
//! 但它对**声学结果**的承诺与实时引擎一样硬。
//!
//! ## 这一层的三条契约
//!
//! 1. **离线运行**: 没有声卡、没有时钟、没有实时回调。输入是一份工程 + 一组注入的
//!    样本源, 输出是 Master 母带缓冲与文件字节。
//! 2. **可并行**: 每个声部/轨道的合成与效果链由 Rayon 工作窃取池并行渲染
//!    ([ROAD-M4-004])。
//! 3. **汇聚顺序确定**: 汇聚到父总线与 Master 时, 求和**严格按基于 `EntityId` 的
//!    确定性字典序单线程固定顺序串行累加** ([ARCH-DET-002])。理由是浮点加法不满足
//!    结合律, `(a + b) + c != a + (b + c)`; 一旦顺序取决于线程完成时刻, 同一份输入
//!    在不同运行下会得到最低有效位不同的母带。
//!
//! 可执行的判据: `render::tests::output_is_bit_identical_across_thread_counts`
//! —— 同一输入 + 1/2/4/8 线程 ⇒ SHA-256 位级摘要相同; 以及
//! `verify/pure_modules.rs` 里那条不依赖 rayon 的端到端镜像判据。
//!
//! ## 确定性契约的落地位置（[ARCH-DET-001] 逐条对应）
//!
//! | L1 条件 | 本 crate 的落点 |
//! | :--- | :--- |
//! | 固定统一处理块大小 (128 采样点) | [`render::L1_BLOCK_SIZE`] / [`render::RenderOptions::l1`] |
//! | 固定求值顺序 | [`sum::reduce_ordered`] + 编译期排好的 `incoming` 顺序 |
//! | 不使用平台 `libm` | 增益换算走 `libm::powf`（[`render::db_to_linear`]）; 抖动只用 IEEE 754 精确指定的 `round` |
//! | 固定种子的 PRNG | [`rng::DeterministicDitherRng`] / [`rng::dither_seed`], 种子由调用方给 |
//! | 位级哈希可比 | [`render::RenderOutput::digest_of`]（SHA-256 over IEEE-754 位型） |
//!
//! ## 模块地图
//!
//! | 模块 | 内容 | 规范 ID |
//! | :--- | :--- | :--- |
//! | [`render`] | 拓扑分层并行调度 + 按 `EntityId` 字典序的确定性串行归约 | `ROAD-M4-004/005`, `ARCH-DET-002` |
//! | [`pdc`] | 关键路径延迟分析与环形延迟线（**待 engine 线提供后改为复用**） | `ARCH-PDC-001/002` |
//! | [`rf64`] | RF64/BW64 容器与 `bext`(v1/v2) 的自研读写（零第三方依赖） | `ARCH-FMT-001` |
//! | [`dither`] | TPDF 抖动与 16/24/32f 位深转换 | `ARCH-FMT-001`, `ARCH-DET-001` |
//! | [`wav`] | 普通 RIFF WAV 的读写, 用 `hound` 当独立第三方裁判 | `ARCH-FMT-001` |
//! | [`midi`] | SMF 0/1 导出与回读, 含独立 VLQ/chunk 字节级核验 | `ARCH-FMT-001 §5.5` |
//! | [`vlq`] | MIDI 可变长度量的零依赖参考编解码 | `ARCH-FMT-001 §5.5` |
//! | [`sum`] | 确定性有序归约核（刻意不依赖 rayon） | `ARCH-DET-002` |
//! | [`rng`] | `yeban_dsp::noise::Rng` 到抖动接口的适配与种子派生 | `ARCH-DET-001` |
//!
//! ## 没有依赖 `yeban-engine`（刻意的）
//!
//! 实时引擎线 (`line/engine-rt`) 与这一层并行开发。为了避免两条线争抢同一个
//! crate, 采样源以 [`render::AudioSource`] trait **注入**; PDC 算法则在本 crate 有
//! 一份最小同构实现（[`pdc`]）。这两处都是显式的 `needs` 待办, 见
//! `docs/ledger/render-master-notes.md`。
//!
//! ## 边界（这一层没有证明什么）
//!
//! - 未接入真实的合成器/效果链/侧链键控; 样本源是注入的 trait。
//! - `bext` 只支持版本 1/2 的读写; BW64 的 `axml`/`bxml`/`sxml`/`chna` XML chunk 未实现。
//! - `yeban_model::DeviceDefinition` 目前没有 `latency_samples` 字段, 因此 PDC 延迟
//!   必须由调用方显式注入（[`render::RenderPlan::compile_with_latencies`]）。
//! - 没有 `criterion` 基准, 因此 [BASELINE-001] 的 "≥ 100× 实时" **未被本分支证实**。
//!
//! ## 规范来源 (Normative)
//!
//! - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §4（离线渲染管线）、§5（持久化
//!   与格式）、`ARCH-DET-001/002`、`ARCH-PDC-001/002`、`ARCH-DSP-001`
//! - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` `ROAD-M4-004/005/006`、`BASELINE-001`
//! - `crates/yeban-model/src/project.rs`（`RoutingGraph` 的 `BTreeMap` 顺序）、
//!   `crates/yeban-model/src/ids.rs`（`EntityId` 的字典序即文本序）
#![forbid(unsafe_code)]
//!
#![deny(missing_docs)]

pub mod dither;
pub mod midi;
pub mod pdc;
pub mod render;
pub mod rf64;
pub mod rng;
pub mod sum;
pub mod vlq;
pub mod wav;

#[cfg(test)]
mod contract_tests {
    use std::collections::BTreeMap;
    use std::str::FromStr;

    use yeban_model::{EntityId, RoutingEdge, RoutingGraph, RoutingKind};

    use crate::dither::{BitDepth, PcmBuffer, quantize};
    use crate::render::{AudioSource, BlockContext, RenderError, RenderOptions, RenderPlan};
    use crate::rf64::{Bext, ContainerKind, ContainerPlan, PcmFormat};

    const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

    fn ulid(index: u32) -> EntityId {
        let mut text = [b'0'; 26];
        for position in 0..4 {
            text[25 - position] = CROCKFORD[((index >> (5 * position)) & 0x1F) as usize];
        }
        EntityId::from_str(core::str::from_utf8(&text).expect("ASCII")).expect("合法 ULID")
    }

    /// 常量样本源（契约测试用，确定性显然）。
    struct Constant(f32);

    impl AudioSource for Constant {
        fn render_block(
            &mut self,
            context: BlockContext,
            out: &mut [f32],
        ) -> Result<(), RenderError> {
            out.fill(self.0);
            let _ = context;
            Ok(())
        }
    }

    /// 32 轨 -> Master 的星形工程（[BASELINE-001] 的"参考工程 A"形状）。
    fn star(tracks: u32) -> (RoutingGraph, EntityId, Vec<EntityId>) {
        let master = ulid(0xFFFF);
        let mut nodes = vec![master];
        let mut edges = BTreeMap::new();
        let mut sources = Vec::new();
        for index in 1..=tracks {
            let track = ulid(index);
            nodes.push(track);
            sources.push(track);
            let created = RoutingEdge {
                id: ulid(0x8000 - index),
                source_node: track,
                destination_node: master,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            };
            edges.insert(created.id, created);
        }
        (RoutingGraph { nodes, edges }, master, sources)
    }

    fn constant_sources(
        sources: &[EntityId],
        value: f32,
    ) -> BTreeMap<EntityId, Box<dyn AudioSource>> {
        sources
            .iter()
            .map(|&node| {
                let source: Box<dyn AudioSource> = Box::new(Constant(value));
                (node, source)
            })
            .collect()
    }

    /// 判据 A: 归约核与共享的 DSP 原语 (`yeban_dsp::block::accumulate`) **逐位等价**。
    ///
    /// 渲染路径只走 `sum`（为了零依赖与本机可验证), 因此必须有一条判据钉住两者不会
    /// 漂移 —— 否则"离线渲染器与实时引擎同序"这句话就失去依据。
    #[test]
    fn sum_kernel_matches_the_shared_dsp_primitive_bit_for_bit() {
        let input = [0.1f32, -0.25, 1.0e7, -1.0e-7, 3.5, -0.0];
        for gain in [1.0f32, 0.5, -2.0, 1.0e-8] {
            let mut ours = [0.0f32; 6];
            let mut theirs = [0.0f32; 6];
            crate::sum::accumulate_into(&input, &mut ours, gain);
            yeban_dsp::block::accumulate(&input, &mut theirs, gain);
            assert_eq!(
                ours.map(f32::to_bits),
                theirs.map(f32::to_bits),
                "gain = {gain}"
            );
        }
    }

    /// 判据 B: 位级摘要确实是"对 IEEE-754 位型做 SHA-256"（自己再算一遍对账）。
    #[test]
    fn render_digest_is_sha256_over_ieee754_bits() {
        use sha2::{Digest, Sha256};
        let samples = [0.0f32, -0.0, 1.0, -1.0, f32::MIN_POSITIVE];
        let mut hasher = Sha256::new();
        for sample in samples {
            hasher.update(sample.to_bits().to_le_bytes());
        }
        let expected: [u8; 32] = hasher.finalize().into();
        assert_eq!(crate::render::RenderOutput::digest_of(&samples), expected);
    }

    /// 判据 C (**端到端契约**): 渲染 → TPDF 抖动 → RF64 + BEXT 落盘 → 读回,
    /// 并且 1/2/4 线程产出的**文件字节**逐位相同。
    ///
    /// 这是把 [ARCH-DET-002] 与 [ARCH-FMT-001] 串起来的那条判据: 如果汇聚顺序受
    /// 线程数影响, 差异会一路穿过抖动与容器, 最终体现在文件哈希上。
    #[test]
    fn full_lint_to_master_chain_is_thread_count_invariant() {
        let (routing, master, sources) = star(32);
        let frames = 1_024u64;
        let channels = 2usize;
        let sample_rate = 48_000u32;
        let seed = 0x0BAD_C0DE_DEAD_BEEF;

        let mut reference: Option<Vec<u8>> = None;
        for threads in [1usize, 2, 4] {
            let options =
                RenderOptions::l1(frames, channels, sample_rate, seed).with_threads(threads);
            let mut plan = RenderPlan::compile(&routing, master, options).expect("编译");
            let output = plan
                .execute(constant_sources(&sources, 0.01))
                .expect("渲染");
            assert_eq!(output.blocks, 8, "1024 / 128");

            // 母带用**按节点派生**的种子抖动: 种子由 (工程种子, Master 身份) 决定,
            // 因此完全可复现, 且不需要任何隐式熵源 [ARCH-DET-001]。
            let mut rng = crate::rng::dither_rng_for(seed, master);
            let pcm = quantize(&output.samples, BitDepth::Int24, &mut rng);
            let payload = pcm.to_le_bytes();
            let format = PcmFormat::integer(channels as u16, sample_rate, 24);
            let plan = ContainerPlan::for_payload(
                ContainerKind::Rf64,
                format,
                payload.len() as u64,
                frames,
                Some(Bext::for_project(
                    "01J8ZK9WQ7F5N2V4B6C8D0E1F2",
                    "2026-10-05",
                    "13:37:00",
                )),
            );
            let mut file = Vec::new();
            crate::rf64::write_container(&mut file, &plan, &payload).expect("写容器");

            let parsed = crate::rf64::parse_container(&file).expect("读回");
            assert_eq!(parsed.kind, ContainerKind::Rf64);
            assert_eq!(&file[parsed.data.clone()], payload.as_slice());
            assert_eq!(parsed.sizes.sample_count, frames);
            assert_eq!(
                parsed.bext.expect("有 bext").originator_reference,
                "01J8ZK9WQ7F5N2V4B6C8D0E1F2"
            );
            assert_eq!(
                crate::rf64::chunk_order(&file).expect("chunk 顺序"),
                vec![*b"ds64", *b"fmt ", *b"bext", *b"data"]
            );

            let previous = reference.replace(file);
            if let Some(expected) = previous {
                let current = reference.as_ref().expect("刚写入");
                assert_eq!(
                    current, &expected,
                    "{threads} 线程产出的文件与 1 线程不同 —— L1 bit-exact 已破"
                );
            }
        }
    }

    /// 判据 D: `PcmBuffer` 与 `PcmFormat` 的位深约定一致（24-bit 是 3 字节, 不是 4）。
    #[test]
    fn container_payload_length_agrees_with_the_buffer_depth() {
        let buffer = PcmBuffer::Int24(vec![0; 10]);
        assert_eq!(buffer.byte_len(), 30);
        assert_eq!(buffer.to_le_bytes().len(), 30);
        let format = PcmFormat::integer(2, 48_000, 24);
        assert_eq!(format.block_align(), 6);
        assert_eq!(format.bytes_per_sample(), 3);
        assert!(buffer.byte_len() % usize::from(format.block_align()) == 0);
    }
}
