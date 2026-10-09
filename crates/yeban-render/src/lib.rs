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
//! | [`mastering`] | 母带导出的 LRA（响度范围）测量、响度归一化预设, 以及把实测响度写进 `bext` 的导出落点；真峰值按**母带口径 16×** 过采样测量 | `ARCH-FMT-001`（`bext` 响度块） |
//! | [`wav`] | 普通 RIFF WAV 的读写, 用 `hound` 当独立第三方裁判 | `ARCH-FMT-001` |
//! | [`midi`] | SMF 0/1 导出与回读, 含独立 VLQ/chunk 字节级核验 | `ARCH-FMT-001 §5.5` |
//! | `als`（feature `experimental-als-export`） | 实验性 Ableton `.als` 导出：Gzip XML + 映射损失表 | `ARCH-FMT-002`, `ROAD-M4-007` |
//! | `logic`（feature `experimental-logic-export`） | 实验性 Logic Pro `.logicx` bundle 导出：自研分块 `ProjectData` + bplist00 `MetaData.plist` + 映射损失表 | `ARCH-FMT-002`, `ROAD-M4-007` |
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
//! - PDC 延迟由调用方显式注入（[`render::RenderPlan::compile_with_latencies`]）;
//!   [`render::track_latencies`] 提供了从 `DeviceDefinition::latency_samples`
//!   ([ARCH-PDC-001], main `8f40290` 补上) 推导该映射的标准做法。
//! - 没有 `criterion` 基准, 因此 [BASELINE-001] 的 "≥ 100× 实时" **未被本分支证实**。
//! - 实验性 `.als` 导出（feature `experimental-als-export`, 见 `als` 模块）只在显式开启时
//!   存在, 且**未与任何参考 `.als` 对账**：它产出 Ableton 风格的 Gzip XML 与一份
//!   **映射损失表**, 但**不声称**产物可被 Live 11/12 直接打开。不可等价映射的构造一律
//!   进损失表（内置设备标记为"音频冻结兜底", 但本切片**不渲染那份音频**）。
//! - 实验性 Logic Pro `.logicx` 导出（feature `experimental-logic-export`, 见 `logic` 模块）
//!   只在显式开启时存在: 它按**本机实测**的字节布局（根头 0x18、记录头 0x24、16 字节事件行、
//!   chunk 名小端存放）产出 `Alternatives/<NNN>/ProjectData` 与标准二进制 plist **字节**
//!   （`logic::build_bundle`, 纯内存、零文件系统 I/O; 建目录与落盘是 app 层 `export_logic`
//!   的事, 与 `als` 同款纪律）, 并返回映射损失表。**打开结论（实测，勿外推）**：本机
//!   **Logic Pro 12.2** 能打开**供体拼接**（`logic::build_bundle_from_donor`）产物（负责人实测,
//!   2026-10-06）; 自研（无供体）写入器的产物被拒绝过两次（账本第 403、405 轮）, 结论不覆盖
//!   其它 Logic 版本或其它机器; 自研路线不写 Logic 的轨道对象与混音/插件/自动化一族 chunk,
//!   全部进损失表（供体路线携带供体的那些记录并逐族登记）。本 feature **没有可选依赖**, 因此默认
//!   依赖图一位不变。
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
// 母带导出的**LRA 测量**与**响度归一化预设**（本 crate 新增; 补 `yeban-dsp` 侧
// `N4`（LRA 未实现）与仓库里"没有响度归一化"这两个缺口, 见该模块头的规范状态一节）。
pub mod mastering;
// 账本第 283/284 轮：编解码已**下移**到独立 crate `yeban-midi`（MCP 也能依赖它, 见该 crate 的文档）。
// 这里**再导出**同一个模块 ⇒ `yeban_render::midi::*` 的既有调用方（app 与其判据）一行不改。
pub use yeban_midi::midi;
pub mod pdc;
pub mod render;
pub mod rf64;
pub mod rng;
pub mod sum;
// 与 `midi` 同理：VLQ 编解码随 SMF 一起下移到 `yeban-midi`，此处再导出以保持路径可用。
pub use yeban_midi::vlq;
pub mod wav;
// **响度计量**（`ITU-R BS.1770-4` 的 K 加权 + 门限积分）住在本 crate 早已依赖的
// `yeban-dsp` 里。这里**再导出同一个模块**，而不是让 `yeban-mcp` 直接加一条
// `yeban-dsp` 依赖边：`scripts/gates/check_mcp_dependency_direction.py` 明文禁止 MCP
// 直接依赖音频栈，而"把共享件下移到 model 级 crate"这条路对本切片不成立
// —— 响度是 DSP 数学（K 加权双二阶 + 门限积分），不是数据模型。
//
// 于是路径是：`yeban-render`（MCP 已依赖）**转发** `yeban-dsp` 的**同一份**实现。
// 这不是第二份实现：`D46` 的"不许有第二份求值"管的是语义复制，这里一个表达式都没有
// 复制（详见 `crates/yeban-mcp/src/domain/render.rs` 的"响度目标"一节）。
pub use yeban_dsp::loudness;
// 实验性 Ableton Live Set (`.als`) 导出 [ARCH-FMT-002] [ROAD-M4-007]：
// **只在非默认 feature `experimental-als-export` 下存在**（AGENTS.md §2 红线 6）。
// 默认构建既不编译这个模块, 也不把 `flate2` 链进依赖图 —— 判据是
// `cargo check -p yeban-render`（默认）与 `--features experimental-als-export` 都能编过,
// 而 `cargo tree -p yeban-render` 在默认构建里没有 `flate2`。
#[cfg(feature = "experimental-als-export")]
pub mod als;
// 实验性 Logic Pro (`.logicx`) bundle 导出 [ARCH-FMT-002] [ROAD-M4-007]：
// **只在非默认 feature `experimental-logic-export` 下存在**（AGENTS.md §2 红线 6）。
// 与 `als` 不同, 本 feature **没有可选依赖**（bplist00 编码器是本 crate 自研的）,
// 因此默认构建与带 feature 构建的 `cargo tree -p yeban-app -e normal --locked` 逐包相同。
#[cfg(feature = "experimental-logic-export")]
pub mod logic;

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

    /// 997 Hz 正弦样本源（幅度线性）。
    ///
    /// 相位由 `first_frame` 决定而不是由内部状态累加: 因此同一个源无论被调度成
    /// 多少个块、按什么顺序, 产出的样本都一样 —— 这条判据要的是信号, 不是调度。
    struct Tone(f64);

    impl AudioSource for Tone {
        fn render_block(
            &mut self,
            context: BlockContext,
            out: &mut [f32],
        ) -> Result<(), RenderError> {
            let rate = f64::from(context.sample_rate);
            for (frame, slot) in out.chunks_exact_mut(context.channels).enumerate() {
                let index = context.first_frame + frame as u64;
                let value =
                    (self.0 * (core::f64::consts::TAU * 997.0 * index as f64 / rate).sin()) as f32;
                slot.fill(value);
            }
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
        let output = hasher.finalize();
        let mut expected = [0u8; 32];
        for (slot, byte) in expected.iter_mut().zip(output.iter()) {
            *slot = *byte;
        }
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
        assert!(
            buffer
                .byte_len()
                .is_multiple_of(usize::from(format.block_align()))
        );
    }

    /// 判据 E (**端到端导出落点**): 一次真实渲染（[`RenderPlan::execute`]）的产物直接
    /// 落成一个 RF64 文件, 且文件里的 `bext` 写着**归一化之后实测**的响度。
    ///
    /// 手算: 单轨 997 Hz、幅度 0.1（= −20 dBFS）⇒ **−20 LUFS**; 流媒体预设的目标是
    /// −14 LUFS ⇒ 增益 **+6 dB** ⇒ `bext` 的 `LoudnessValue` 应是 **−1400**
    /// （0.01 LUFS 刻度）, 真峰值 ≈ −14 dBTP（−1400, 0.01 dBTP 刻度）。
    ///
    /// 这条判据把 [ARCH-DET-002]（渲染）、[ARCH-FMT-001]（容器 + `bext` 响度元数据）
    /// 与本切片的导出落点串在一根线上: 任何一环把响度写成哨兵, 这里就变红。
    #[test]
    fn a_rendered_master_lands_in_a_file_with_a_measured_loudness_block() {
        let (routing, master, sources) = star(1);
        let frames = 96_000u64; // 2 s @ 48 kHz
        let options = RenderOptions::l1(frames, 2, 48_000, 0x0BAD_C0DE_DEAD_BEEF);
        let mut plan = RenderPlan::compile(&routing, master, options).expect("编译");

        let mut injected: BTreeMap<EntityId, Box<dyn AudioSource>> = BTreeMap::new();
        for node in &sources {
            injected.insert(*node, Box::new(Tone(0.1)));
        }
        let mut output = plan.execute(injected).expect("渲染");
        assert_eq!(output.samples.len(), 96_000 * 2);

        let metadata = Bext::for_project("01J8ZK9WQ7F5N2V4B6C8D0E1F2", "2026-10-08", "13:37:00");
        let mut rng = crate::rng::dither_rng_for(options.seed, master);
        let export = crate::mastering::export_master(
            48_000,
            &mut output,
            crate::mastering::ExportPreset::streaming(),
            BitDepth::Int24,
            ContainerKind::Rf64,
            &metadata,
            &mut rng,
        )
        .expect("导出");

        assert_eq!(
            export.outcome.bound,
            crate::mastering::GainBound::LoudnessTarget
        );
        assert!(
            (export.outcome.gain_db - 6.0).abs() < 0.05,
            "实际 {} dB",
            export.outcome.gain_db
        );

        let parsed = crate::rf64::parse_container(&export.file).expect("读回");
        assert_eq!(parsed.sizes.sample_count, 96_000);
        assert_eq!(parsed.format.bits_per_sample, 24);
        let loudness = parsed
            .bext
            .expect("导出必须带 bext")
            .loudness
            .expect("版本 2 必须带 EBU R128 响度块");
        assert_eq!(
            loudness.loudness_value,
            crate::rf64::Loudness::from_lufs(export.outcome.after.integrated_lufs)
        );
        assert!(
            (f32::from(loudness.loudness_value) / 100.0 + 14.0).abs() < 0.05,
            "手算 −1400, 实际 {}",
            loudness.loudness_value
        );
        assert!(
            (f32::from(loudness.max_true_peak_level) / 100.0 + 14.0).abs() < 0.15,
            "手算 ≈ −1400, 实际 {}",
            loudness.max_true_peak_level
        );
    }
}
