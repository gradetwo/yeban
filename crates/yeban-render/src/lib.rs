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
//! | [`pdc`] | 关键路径延迟分析与环形延迟线（**引擎已有 PDC API, 但两者不是同一个式子 —— 见该模块文档的状态一节**） | `ARCH-PDC-001/002` |
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

    /// 逐轨**不同**的常量源（幅度随轨道序号变化）。
    ///
    /// 为什么不能用"整块同值"的常量源: 一组**相等**的加数无论按什么
    /// 顺序相加都得到同一串部分和, 因此"归约顺序被线程数改变"这一类缺陷在这组输入上
    /// **不可观测**。实测（注入: 线程数大于 1 时按反序归约）时, 本判据仍然全绿, 而
    /// `render::tests::output_is_bit_identical_across_thread_counts` 与
    /// `tests/l1_digest_contract.rs::thread_counts_do_not_change_the_digest` 同时变红 ——
    /// 说明当时的夹具对"汇聚顺序随线程数变化"没有判别力。逐轨取不同幅度后, 反序累加
    /// 会改变部分和的舍入, 同一注入即被本判据抓到。
    ///
    /// 幅度刻意都落在 `[-1.0, 1.0]` 内, 以免量化阶段把差异掩盖在饱和里。
    fn distinct_constant_sources(sources: &[EntityId]) -> BTreeMap<EntityId, Box<dyn AudioSource>> {
        sources
            .iter()
            .enumerate()
            .map(|(index, &node)| {
                let source: Box<dyn AudioSource> = Box::new(Constant(0.001 * (index as f32 + 1.0)));
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
    ///
    /// 夹具用**逐轨不同**的常量（[`distinct_constant_sources`]）: 整块同值会让
    /// "顺序被线程数改变"这个缺陷不可观测（见该函数的文档与实测）。
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
                .execute(distinct_constant_sources(&sources))
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

    /// 判据 (**诊断文案黄金表**): 本 crate 默认构建里 5 个错误枚举的**全部 45 个变体**
    /// 的 `Display` 文案, 逐条钉死在同一张表里。
    ///
    /// # 为什么是一张**表**（本机实测的缺口）
    ///
    /// `grep -rn 'to_string()' crates/yeban-render/src/{rf64,wav,render,pdc,mastering}.rs`
    /// 只命中**产线**代码: 5 个 `Display` 实现的文案在判据里**零断言**。因此
    /// `Display` 可以整段改坏而全量判据全绿 —— 这是 `mod-theory` 的实测同款
    /// （5 条 `Display` 改坏, 一条判据都不红）。文案是**调用方读到的唯一诊断信息**,
    /// 也是 `yeban-mcp` 工具面返回给调用方的文本, 因此它是对外契约。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: 5 个枚举的**全部**变体（单位: 一个变体）, 每个给一个固定的构造值。
    /// 读数: 对该值 `to_string()` 得到的字符串（单位: 一个 UTF-8 字符串）。
    /// 覆盖面由数组长度 `45` 与下面那条去重断言一起守住。
    ///
    /// # 表的来源
    ///
    /// 右列是**本机实测读数**（一个临时探针在 `469cedd` 上打印全部 45 条, 探针已删）,
    /// 不是从实现里抄的表达式。表的用途正是: **以后任何一字改动都必须显式改这张表**。
    ///
    /// # 非空证明
    ///
    /// ① 45 条文案里**恰有一对相同** —— `Rf64Error::ZeroChannels` 与
    /// `RenderError::ZeroChannels` 都是 `声道数为 0` ⇒ 去重后是 **44** 条
    /// （下面把这两个数与那一对一起断言）;
    /// ② 五个枚举各自的变体数必须是 **18 / 4 / 12 / 3 / 8** —— 哪个枚举少一个变体进表就红;
    /// ③ 其中 6 条带运行时参数（`Truncated` / `DataSizeMismatch` /
    /// `UnrepresentableBlockAlign` / `UnsupportedFormatTag` / `BadStartTimecode` /
    /// `NonFiniteSamples`）⇒ 改插值而不是改字面量也会红。
    #[test]
    fn every_diagnostic_message_is_pinned_in_one_golden_table() {
        use crate::mastering::MasterExportError;
        use crate::pdc::PdcError;
        use crate::rf64::Rf64Error;
        use crate::wav::WavError;

        let cases: [(&str, String, &str); 45] = [
            (
                "Rf64::Io",
                Rf64Error::Io("磁盘满了".to_owned()).to_string(),
                r#"I/O 失败: 磁盘满了"#,
            ),
            (
                "Rf64::NotWaveContainer",
                Rf64Error::NotWaveContainer.to_string(),
                r#"不是 RIFF/RF64/BW64 的 WAVE 容器"#,
            ),
            (
                "Rf64::Truncated",
                Rf64Error::Truncated {
                    what: "data 负载",
                    got: 7,
                }
                .to_string(),
                r#"data 负载 被截断 (只有 7 字节)"#,
            ),
            (
                "Rf64::BadDs64Len",
                Rf64Error::BadDs64Len(12).to_string(),
                r#"ds64 chunk 长度应为 28, 实际 12"#,
            ),
            (
                "Rf64::MissingDs64",
                Rf64Error::MissingDs64.to_string(),
                r#"RF64/BW64 文件缺少 ds64 chunk"#,
            ),
            (
                "Rf64::MissingFmt",
                Rf64Error::MissingFmt.to_string(),
                r#"缺少 fmt chunk"#,
            ),
            (
                "Rf64::MissingData",
                Rf64Error::MissingData.to_string(),
                r#"缺少 data chunk"#,
            ),
            (
                "Rf64::BadFmtLen",
                Rf64Error::BadFmtLen(40).to_string(),
                r#"fmt chunk 长度不受支持: 40"#,
            ),
            (
                "Rf64::UnsupportedFormatTag",
                Rf64Error::UnsupportedFormatTag(0x0002).to_string(),
                r#"不受支持的格式标签: 0x0002"#,
            ),
            (
                "Rf64::ZeroChannels",
                Rf64Error::ZeroChannels.to_string(),
                r#"声道数为 0"#,
            ),
            (
                "Rf64::ZeroSampleRate",
                Rf64Error::ZeroSampleRate.to_string(),
                r#"采样率为 0"#,
            ),
            (
                "Rf64::ZeroBitsPerSample",
                Rf64Error::ZeroBitsPerSample.to_string(),
                r#"位深为 0"#,
            ),
            (
                "Rf64::DataSizeMismatch",
                Rf64Error::DataSizeMismatch {
                    declared: 8,
                    actual: 4,
                }
                .to_string(),
                r#"声明的 data 负载长度是 8 字节, 实际交进来的是 4 字节"#,
            ),
            (
                "Rf64::UnrepresentableBlockAlign",
                Rf64Error::UnrepresentableBlockAlign {
                    channels: 21_846,
                    bytes_per_sample: 3,
                }
                .to_string(),
                r#"声道布局无法表示: 21846 声道 × 3 字节/样本 超过 nBlockAlign 的 u16 上限"#,
            ),
            (
                "Rf64::UnsupportedBextVersion",
                Rf64Error::UnsupportedBextVersion(3).to_string(),
                r#"不受支持的 bext 版本: 3"#,
            ),
            (
                "Rf64::BextLoudnessVersionMismatch",
                Rf64Error::BextLoudnessVersionMismatch {
                    version: 2,
                    has_loudness: false,
                }
                .to_string(),
                r#"bext 版本 2 必须带 EBU R128 响度字段, 实际却没有: 读取器对这个版本恒给出同一个取值, 写出去就是一个往返不等的容器"#,
            ),
            (
                "Rf64::UnrepresentableBextField",
                Rf64Error::UnrepresentableBextField {
                    field: "Description",
                }
                .to_string(),
                r#"bext 的 Description 字段取值含 NUL: 读取器在 NUL 处停止（或裁掉尾随 NUL）, 写出去的值与读回来的值不是同一个字符串"#,
            ),
            (
                "Rf64::BadStartTimecode",
                Rf64Error::BadStartTimecode("25:00:00".to_owned()).to_string(),
                r#"起始时间码 "25:00:00" 不是 HH:MM:SS（时分秒必须各自在合法范围内）"#,
            ),
            (
                "Wav::Hound",
                WavError::Hound("hound 说不行".to_owned()).to_string(),
                r#"hound 失败: hound 说不行"#,
            ),
            (
                "Wav::FormatMismatch",
                WavError::FormatMismatch {
                    expected: "Float32 (32 位)".to_owned(),
                    got: "16 位, 整数".to_owned(),
                }
                .to_string(),
                r#"格式不匹配: 期望 Float32 (32 位), 实际 16 位, 整数"#,
            ),
            (
                "Wav::UnsupportedDepth",
                WavError::UnsupportedDepth(8).to_string(),
                r#"不支持的位深: 8"#,
            ),
            (
                "Wav::RejectedFormat",
                WavError::RejectedFormat {
                    field: "channels",
                    detail: "0".to_owned(),
                }
                .to_string(),
                r#"格式不可用: channels = 0"#,
            ),
            (
                "Render::InvalidGraph",
                RenderError::InvalidGraph("环".to_owned()).to_string(),
                r#"路由图非法: 环"#,
            ),
            (
                "Render::MasterNotInGraph",
                RenderError::MasterNotInGraph(ulid(0xFFFF)).to_string(),
                r#"Master 节点不在路由图里: 00000000000000000000001ZZZ"#,
            ),
            (
                "Render::Pdc",
                RenderError::Pdc("有环".to_owned()).to_string(),
                r#"PDC 分析失败: 有环"#,
            ),
            (
                "Render::ZeroChannels",
                RenderError::ZeroChannels.to_string(),
                r#"声道数为 0"#,
            ),
            (
                "Render::ZeroBlockSize",
                RenderError::ZeroBlockSize.to_string(),
                r#"块大小为 0"#,
            ),
            (
                "Render::ZeroFrames",
                RenderError::ZeroFrames.to_string(),
                r#"总帧数为 0"#,
            ),
            (
                "Render::SizeOverflow",
                RenderError::SizeOverflow {
                    what: "block_size * channels",
                }
                .to_string(),
                r#"block_size * channels 溢出 usize"#,
            ),
            (
                "Render::MissingSource",
                RenderError::MissingSource(ulid(0xFFFF)).to_string(),
                r#"节点 00000000000000000000001ZZZ 没有注册样本源"#,
            ),
            (
                "Render::SourceOnBusNode",
                RenderError::SourceOnBusNode(ulid(0xFFFF)).to_string(),
                r#"节点 00000000000000000000001ZZZ 有入边, 不能注册样本源"#,
            ),
            (
                "Render::UnknownNode",
                RenderError::UnknownNode(ulid(0xFFFF)).to_string(),
                r#"节点 00000000000000000000001ZZZ 不在渲染计划里（未知或被剪枝）"#,
            ),
            (
                "Render::ThreadPool",
                RenderError::ThreadPool("线程池炸了".to_owned()).to_string(),
                r#"线程池构建失败: 线程池炸了"#,
            ),
            (
                "Render::Source",
                RenderError::Source {
                    node: ulid(0xFFFF),
                    message: "音源炸了".to_owned(),
                }
                .to_string(),
                r#"样本源 00000000000000000000001ZZZ 失败: 音源炸了"#,
            ),
            (
                "Pdc::Cycle",
                PdcError::Cycle {
                    remaining: vec!["a".to_owned(), "b".to_owned()],
                }
                .to_string(),
                r#"路由图有环, 剩余节点: ["a", "b"]"#,
            ),
            (
                "Pdc::UnknownNode",
                PdcError::UnknownNode {
                    node: "ghost".to_owned(),
                }
                .to_string(),
                r#"边的端点不在节点表里: ghost"#,
            ),
            (
                "Pdc::MasterNotInGraph",
                PdcError::MasterNotInGraph {
                    master: "nowhere".to_owned(),
                }
                .to_string(),
                r#"Master 节点不在图里: nowhere"#,
            ),
            (
                "Mst::UnsupportedSampleRate",
                MasterExportError::UnsupportedSampleRate(22_050).to_string(),
                r#"响度计量不支持 22050 Hz（内置档位: 44.1/48/88.2/96 kHz）"#,
            ),
            (
                "Mst::NotStereo",
                MasterExportError::NotStereo(4).to_string(),
                r#"母带导出只支持立体声, 实际 4 声道"#,
            ),
            (
                "Mst::RaggedInterleavedBuffer",
                MasterExportError::RaggedInterleavedBuffer(7).to_string(),
                r#"交错缓冲长度 7 不是偶数（立体声的帧必须成对）"#,
            ),
            (
                "Mst::UnsupportedBextVersion",
                MasterExportError::UnsupportedBextVersion(3).to_string(),
                r#"bext 版本 3 的字段表未核验（读取器只接受 1 与 2）; 写出去就会产出一个本 crate 读不回来的容器"#,
            ),
            (
                "Mst::UnrepresentableBextField",
                MasterExportError::UnrepresentableBextField {
                    field: "Originator",
                }
                .to_string(),
                r#"bext 的 Originator 字段取值含 NUL: 读取器在 NUL 处停止（或裁掉尾随 NUL）, 写出去的元数据与请求的不是同一个值"#,
            ),
            (
                "Mst::BextCannotCarryLoudness",
                MasterExportError::BextCannotCarryLoudness(1).to_string(),
                r#"bext 版本 1 没有 EBU R128 响度字段, 无法承载实测响度 [ARCH-FMT-001]"#,
            ),
            (
                "Mst::NonFiniteSamples",
                MasterExportError::NonFiniteSamples {
                    index: 3,
                    value: f32::NAN,
                }
                .to_string(),
                r#"母带缓冲的第 3 个样本是 NaN（非有限）; 导出会静默丢弃它, 因此拒绝"#,
            ),
            (
                "Mst::Container",
                MasterExportError::Container(Rf64Error::ZeroChannels).to_string(),
                r#"容器写入失败: 声道数为 0"#,
            ),
        ];
        for (label, actual, golden) in &cases {
            assert_eq!(actual.as_str(), *golden, "诊断文案变了: {label}");
        }

        // 覆盖面: 45 个变体, 按枚举分别是 18 / 4 / 12 / 3 / 8。
        assert_eq!(cases.len(), 45, "黄金表必须覆盖全部 45 个变体");
        for (prefix, expected) in [
            ("Rf64::", 18usize),
            ("Wav::", 4),
            ("Render::", 12),
            ("Pdc::", 3),
            ("Mst::", 8),
        ] {
            assert_eq!(
                cases
                    .iter()
                    .filter(|(label, _, _)| label.starts_with(prefix))
                    .count(),
                expected,
                "{prefix} 的变体数（表里少了一个变体）"
            );
        }

        // 文案的唯一一处重合: 两个不同的枚举都用了 `声道数为 0` ⇒ 去重后 44 条。
        let mut golden_texts: Vec<&str> = cases.iter().map(|(_, _, golden)| *golden).collect();
        golden_texts.sort_unstable();
        golden_texts.dedup();
        assert_eq!(
            golden_texts.len(),
            44,
            "45 条文案里恰有一对相同 ⇒ 去重后必须是 44"
        );
        assert_eq!(
            Rf64Error::ZeroChannels.to_string(),
            RenderError::ZeroChannels.to_string(),
            "唯一的重合就是这两个枚举的 `声道数为 0`"
        );
    }

    /// 判据 (**错误链与三条 `From` 转换**): `source()` 只有一条边, 三条转换必须**原样
    /// 保留**内层文案。
    ///
    /// # 为什么需要它（与上一条同族的缺口）
    ///
    /// `Display` 有表之后, 仍然有两类"改坏了没人管"的形态:
    /// ① `source()` —— 把它从 `Some` 改成 `None`（或反过来）会让错误链断掉, 而
    /// `Display` 一字不变; ② `From` 转换 —— 把内层文案丢掉（例如换成一句 "转换失败"）
    /// 同样不动任何既有判据。
    ///
    /// # 量的是什么（对象 + 单位）
    ///
    /// 对象: 8 个错误值（单位: 一个值）。读数: `source()` 是否 `Some`（布尔）、
    /// `source()` 的文案（字符串）与三条转换后的**完整**文案（字符串）。
    ///
    /// # 非空证明
    ///
    /// 8 个值里**恰有 1 个** `source()` 是 `Some`（`Container`）, 其余 7 个是 `None`
    /// —— 因此"一律 `Some`"与"一律 `None`"两种改法都能被这条判据分开。
    #[test]
    fn error_source_chains_and_conversions_are_pinned() {
        use crate::mastering::MasterExportError;
        use crate::pdc::PdcError;
        use crate::render::RenderError;
        use crate::rf64::Rf64Error as F;
        use crate::wav::WavError;
        use std::error::Error as _;

        // ① source() 的唯一一条边。
        let chained = MasterExportError::Container(F::MissingFmt);
        assert_eq!(
            chained.source().map(ToString::to_string),
            Some("缺少 fmt chunk".to_owned()),
            "Container 变体必须把内层错误挂在错误链上"
        );
        let no_chain = [
            MasterExportError::UnsupportedSampleRate(22_050),
            MasterExportError::NotStereo(4),
            MasterExportError::RaggedInterleavedBuffer(7),
            MasterExportError::UnsupportedBextVersion(3),
            MasterExportError::UnrepresentableBextField {
                field: "Originator",
            },
            MasterExportError::BextCannotCarryLoudness(1),
            MasterExportError::NonFiniteSamples {
                index: 3,
                value: f32::NAN,
            },
        ];
        assert_eq!(no_chain.len(), 7);
        for error in &no_chain {
            assert!(error.source().is_none(), "{error} 不该有错误链");
        }
        // 另外四个枚举的错误链**全部**为空（它们只实现 Display）。
        assert!(F::Io("x".to_owned()).source().is_none());
        assert!(F::MissingFmt.source().is_none());
        assert!(WavError::UnsupportedDepth(8).source().is_none());
        assert!(RenderError::ZeroChannels.source().is_none());
        assert!(
            PdcError::UnknownNode {
                node: "g".to_owned()
            }
            .source()
            .is_none()
        );

        // ② From<io::Error> for Rf64Error: 内层文案原样保留。
        let io_error = std::io::Error::other("boom");
        assert_eq!(
            F::from(io_error).to_string(),
            "I/O 失败: boom",
            "From<io::Error> 必须保留内层文案"
        );

        // ③ From<PdcError> for RenderError: 内层 Display 原样接到前缀后面。
        let pdc = PdcError::UnknownNode {
            node: "ghost".to_owned(),
        };
        assert_eq!(pdc.to_string(), "边的端点不在节点表里: ghost");
        assert_eq!(
            RenderError::from(pdc).to_string(),
            "PDC 分析失败: 边的端点不在节点表里: ghost",
            "From<PdcError> 必须保留内层文案"
        );

        // ④ From<hound::Error> for WavError: 前缀 + 第三方自己的 Display（内层不许丢）。
        let hound_error = hound::Error::FormatError("坏格式");
        let inner = hound_error.to_string();
        let converted = WavError::from(hound_error).to_string();
        assert_eq!(
            converted,
            format!("hound 失败: {inner}"),
            "From<hound::Error> 必须原样保留第三方的 Display"
        );
        assert!(
            !inner.is_empty(),
            "第三方 Display 不能是空串, 否则上面那条是空判据"
        );
    }
}
