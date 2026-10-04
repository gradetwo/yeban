//! # 跨架构 L1/L2 读数收据（`MUST-GATE-002` / `MUST-GATE-003` 的载体）
//!
//! 本文件是 `examples/export_l1_receipt.rs`（导出）与 `examples/compare_l1_receipts.rs`
//! （比较）**共享的纯逻辑**：收据的数据模型、确定性文本序列化、严格解析、以及判决。
//!
//! ## 为什么需要一份"收据"
//!
//! `MUST-GATE-003`（跨架构 L2 一致性 `< 1e-6`）长期 `PENDING`，理由始终是"没有第二个架构"。
//! GitHub 现在提供 `ubuntu-24.04-arm` runner，所以缺的**不是硬件，而是产物**：
//! 一份**规范化的、可跨机比较的读数**。本文件定义那个产物，并让
//! "x86_64 跑一次 + ARM 跑一次 → 比对"变成两条命令。
//!
//! ## 格式（`yeban-l1-receipt v1`，行式文本，UTF-8）
//!
//! ```text
//! yeban-l1-receipt v1                       # 必须是第一个非注释行
//! fixture reference-a                       # 夹具名
//! tracks 32
//! frames 8192
//! channels 2
//! sample_rate 48000
//! block_size 128                           # ADR-0001 D32 / ARCH-DET-001 的 128 帧块
//! threads auto                             # auto | <N>   —— 见下方"线程数为什么不算指纹"
//! seed 24301
//! gain_db none                             # none | <f32>  —— 夹具的边增益设置
//! latency 00000000000000000000000001 0     # <ULID> <帧>, 每个注入延迟的节点一行, 按键升序
//! target_arch aarch64
//! target_os macos
//! target_env -                             # 空值写 `-`（不写空值, 避免行尾空白）
//! target_endian little
//! target_pointer_width 64
//! target_triple aarch64-apple-darwin       # 来自 `rustc -vV` 的 host
//! rustc_version rustc 1.99.0 (b940084d7 2026-09-28)
//! digest <64 位小写十六进制>                # RenderOutput::digest（每个 f32 取位型做 SHA-256）
//! longest_path_frames 0
//! pdc_expected_frames 0                    # 导出侧**独立复算**的 L_max（见下）
//! sample_count 16384
//! abs_max 4.99999999999999978e-1
//! sum_squares 1.2e3
//! class E ieee-exact                       # 两个类别都必须声明
//! class T transcendental
//! op E add@master-serial-reduction         # 路径上的每一个运算, 带所在位置
//! op E mul@edge-gain-apply
//! op E div@tone-noise-normalize
//! op T libm::powf@db-to-linear             # 只有 gain_db != none 时才会出现
//! sample 0 E 3f000000,3f000000,...
//! ```
//!
//! 规则（解析器**严格**执行，未知字段/坏值一律报错而不是 panic）：
//!
//! - 空行与 `#` 开头的行被忽略；其余每行是 `<键><空格><值>`；
//! - 除 `rustc_version` 外，值的记号内**不允许空格**；`target_env` 的空值写 `-`；
//! - 数值一律十进制；统计量用 `{:.17e}`（17 位有效数字 ⇒ 可精确往返，且与机器无关）；
//! - 样本行是 `sample <起始下标> <类别码> <8 位小写十六进制位型>[,<...>]*`，
//!   一行最多 [`SAMPLES_PER_LINE`] 个样本，行内样本**必须同类**，行间下标**严格递增**；
//! - 收据必须覆盖 `0..sample_count` 的**全部**样本（不允许抽样 —— 见下方"为什么不做抽样"）。
//!
//! ## 为什么不做抽样（这是判据的完整性，不是性能问题）
//!
//! `MUST-GATE-003` 要判的是"**最大**样本绝对误差 `< 1e-6`"。抽样会把"某个没被抽到的
//! 样本差了 1e-3"变成绿 —— 那正是本仓库反复警惕的"假绿"（`docs/ledger/gate-status.md`
//! 的元判据、`L12/D25` 的空转教训）。因此收据携带**固定长度渲染的全部样本位型**，
//! 长度由夹具的 `frames` 决定（默认短到可以上 CI artifact，见 `export_l1_receipt.rs`）。
//!
//! ## ADR-0001 D32 如何体现
//!
//! D32 的裁决是**按运算类别分策**：
//!
//! 1. **IEEE 精确类**（`add`/`sub`/`mul`/`div`/`sqrt`/`abs`/`min`/`max`/钳位/整数→`f32` 转换）：
//!    跨架构**零容差** ⇒ 任何一个这类样本位型不同，判决直接 `FAIL`；
//! 2. **超越函数类**（`log`/`exp`/`powf`/`sin`/…）：给预算（D32 取 **4096 ulp**），
//!    且 `MUST-GATE-003` 另有一条**绝对**预算（`< 1e-6`，即 -120 dBFS）。
//!
//! 于是每一条样本都带一个**类别码**：`E` = 只由 IEEE 精确类运算决定；
//! `T` = 路径上有超越函数**参与**（哪怕最终那一步乘法本身是精确的 —— 这是 D32
//! "按类别分策"的核心：**污染会沿数据流传播，而预算只对受污染的量生效**）。
//! 判决顺序见 [`judge`]。
//!
//! ## 线程数为什么**不算**指纹
//!
//! `[ARCH-DET-002]` 的实测结论是"线程数不改变母带字节"。因此比较器**允许**两份收据的
//! `threads` 不同（并如实报告 `threads_differ=true`）—— 否则"1 线程 vs 4 线程 digest 相同"
//! 这条判据就无法用同一个比较器表达。其余指纹字段必须逐字段相同，否则两份收据
//! **不可比**（返回 [`JudgeError`]，退出码 2，而不是假装判了个结论）。
//!
//! ## 本模块零第三方依赖
//!
//! 只用 `core`/`std`（`BTreeMap` 而不是 `HashMap`：`crates/**` 的 G01 守卫禁止后者，
//! 而且确定性本来就要求有序容器）。因此它可以脱离 `rayon`/`hound`/`midly` 用
//! `rustc --edition 2024 --test` 在本机真跑：见 `examples/support/l1_receipt_tests.rs`。

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// 收据首行的模式串（含版本）。
pub const SCHEMA_LINE: &str = "yeban-l1-receipt v1";

/// `MUST-GATE-003` 的绝对误差预算：最大样本绝对误差必须**严格小于** 1e-6（-120 dBFS）。
pub const ABS_LIMIT: f64 = 1.0e-6;

/// ADR-0001 **D32** 给"超越函数类"的 ulp 预算。
pub const ULP_LIMIT: u64 = 4096;

/// 样本行一行最多打包多少个位型。
pub const SAMPLES_PER_LINE: usize = 16;

/// 判决报告里最多点名多少个越界样本（总数仍如实报告，不截断事实）。
pub const MAX_OFFENDERS: usize = 8;

/// IEEE 精确类的类别码。
const CODE_IEEE: char = 'E';
/// 超越函数类的类别码。
const CODE_TRANSCENDENTAL: char = 'T';

/// 写一行到 `String`；写 `String` 不会失败，因此这里的 `expect` 不可能触发。
macro_rules! emitln {
    ($out:expr, $($arg:tt)*) => {{
        writeln!($out, $($arg)*).expect("写入 String 不会失败")
    }};
}

/// 写一段（**不**换行）到 `String`。样本行需要多次追加，才用得上它。
macro_rules! emit {
    ($out:expr, $($arg:tt)*) => {{
        write!($out, $($arg)*).expect("写入 String 不会失败")
    }};
}

// ---------------------------------------------------------------------------
// 类别
// ---------------------------------------------------------------------------

/// 一条样本的**运算类别**（ADR-0001 D32 的两分法）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SampleClass {
    /// 只由 IEEE-754 完全规定的运算决定（加/减/乘/除/开方/比较/转换…）⇒ 跨架构零容差。
    IeeeExact,
    /// 路径上有超越函数参与（`powf`/`log`/`exp`/…）⇒ 只要求落在预算内。
    Transcendental,
}

impl SampleClass {
    /// 收据里的单字符类别码。
    #[must_use]
    pub const fn code(self) -> char {
        match self {
            Self::IeeeExact => CODE_IEEE,
            Self::Transcendental => CODE_TRANSCENDENTAL,
        }
    }

    /// 从类别码还原；未知码返回 `None`。
    #[must_use]
    pub const fn from_code(code: char) -> Option<Self> {
        match code {
            CODE_IEEE => Some(Self::IeeeExact),
            CODE_TRANSCENDENTAL => Some(Self::Transcendental),
            _ => None,
        }
    }

    /// 人读的名字（与 `class` 声明行一致）。
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::IeeeExact => "ieee-exact",
            Self::Transcendental => "transcendental",
        }
    }
}

/// 夹具的**边增益**设置。
///
/// 刻意用具名枚举而不是 `Option<f32>`：`None`（不调 `powf`）与 `Some(0.0)`（调 `powf`
/// 但结果恰为 1.0）在收据里必须是**可区分**的，否则"路径上有没有超越函数"这件事
/// 就被抹掉了 —— 而那正是 D32 分策的依据。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EdgeGain {
    /// 参考工程 A 的字面形态：`RoutingEdge::gain_db == None`（路径上无超越函数）。
    Identity,
    /// 每条边带 `Some(db)`；增益由 `libm::powf(10, db/20)` 得出（**超越函数类**）。
    Db(f32),
}

impl EdgeGain {
    /// 收据里的文本形式（`none` 或十进制）。
    #[must_use]
    pub fn to_token(self) -> String {
        match self {
            Self::Identity => "none".to_owned(),
            Self::Db(db) => db.to_string(),
        }
    }
}

/// 请求的线程数。`Auto` = 由 Rayon 决定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Threads {
    /// 由 Rayon 决定（收据**不**记录实际线程数：那是机器相关的）。
    Auto,
    /// 显式线程数。
    Fixed(usize),
}

impl Threads {
    /// 收据里的文本形式（`auto` 或十进制）。
    #[must_use]
    pub fn to_token(self) -> String {
        match self {
            Self::Auto => "auto".to_owned(),
            Self::Fixed(count) => count.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// 数据模型
// ---------------------------------------------------------------------------

/// 参与渲染的**工程/选项指纹**。除 `threads` 外，两份收据的指纹必须逐字段相同才可比。
#[derive(Clone, Debug, PartialEq)]
pub struct Fingerprint {
    /// 夹具名（本线固定 `reference-a`）。
    pub fixture: String,
    /// 轨道数。
    pub tracks: u32,
    /// 总帧数。
    pub frames: u64,
    /// 声道数。
    pub channels: usize,
    /// 采样率（Hz）。
    pub sample_rate: u32,
    /// 处理块大小（帧）[`crate::render::L1_BLOCK_SIZE`]。
    pub block_size: usize,
    /// 请求的线程数（**不参与可比性判定**，见模块文档）。
    pub threads: Threads,
    /// 确定性种子 [ARCH-DET-001]。
    pub seed: u64,
    /// 边增益设置。
    pub gain: EdgeGain,
    /// 注入的每节点自身延迟（帧）；键是节点身份的规范 ULID 文本。
    pub latency: BTreeMap<String, u32>,
}

/// 工具链与目标三元组：让收据**自证来自哪个架构**。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toolchain {
    /// `cfg!(target_arch)`（编译期常量，永远非空）。
    pub target_arch: String,
    /// `cfg!(target_os)`。
    pub target_os: String,
    /// `cfg!(target_env)`；空值在收据里写 `-`。
    pub target_env: String,
    /// `cfg!(target_endian)`。
    pub target_endian: String,
    /// `cfg!(target_pointer_width)`。
    pub target_pointer_width: String,
    /// 完整目标三元组：优先取 `rustc -vV` 的 `host:`，取不到则由 arch/os 合成。
    pub target_triple: String,
    /// `rustc -vV` 的 `release` + commit + 日期；取不到时写 `unknown`。
    pub rustc_version: String,
}

/// 路径上的一个运算及其类别。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathOp {
    /// 该运算的类别（D32 两分法）。
    pub class: SampleClass,
    /// `运算@所在位置`，如 `add@master-serial-reduction`。
    pub op: String,
}

/// 一条样本读数：下标 + 类别 + **位型**（不是数值：`-0.0`、NaN 载荷也是位级一致的一部分）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Probe {
    /// 在交错母带缓冲里的下标（`frame * channels + channel`）。
    pub index: u64,
    /// 该样本的运算类别。
    pub class: SampleClass,
    /// IEEE-754 `f32` 位型。
    pub bits: u32,
}

/// 一份完整的读数收据。
#[derive(Clone, Debug, PartialEq)]
pub struct Receipt {
    /// 工程/选项指纹。
    pub fingerprint: Fingerprint,
    /// 工具链与目标三元组。
    pub toolchain: Toolchain,
    /// `RenderOutput::digest`（每个 `f32` 取位型做 SHA-256）。
    pub digest: [u8; 32],
    /// `RenderPlan::longest_path_frames()`（PDC 关键路径，来源是渲染计划）。
    pub longest_path_frames: u32,
    /// 导出侧**独立复算**的 `L_max`（必须与上一字段相等，否则收据被拒 —— 这是判据 ⑧）。
    pub pdc_expected_frames: u32,
    /// 样本数（= `probes.len()` = `frames * channels`）。
    pub sample_count: u64,
    /// `max(|sample|)`，用 `f64` 顺序累加（IEEE 精确，机器无关）。
    pub abs_max: f64,
    /// `sum(sample^2)`，用 `f64` 顺序累加。
    pub sum_squares: f64,
    /// 路径上的全部运算（含类别声明）。
    pub ops: Vec<PathOp>,
    /// 全部样本（下标 0 起连续）。
    pub probes: Vec<Probe>,
}

impl Receipt {
    /// 收据的 64 位小写十六进制 digest 文本。
    #[must_use]
    pub fn digest_hex(&self) -> String {
        digest_hex(&self.digest)
    }

    /// 本收据是否声明了超越函数类运算。
    #[must_use]
    pub fn has_transcendental_ops(&self) -> bool {
        self.ops
            .iter()
            .any(|op| op.class == SampleClass::Transcendental)
    }

    /// **逐字段自洽性检查**（不是"格式对不对"，那是解析器的事）。
    ///
    /// 这是"收据不能自相矛盾"的硬判据：任何一条不成立，收据就不可用于对账。
    ///
    /// # Errors
    ///
    /// 返回人读的说明（调用方包成 [`ParseError`] 或直接打印）。
    pub fn validate(&self) -> Result<(), String> {
        let fingerprint = &self.fingerprint;
        if fingerprint.fixture.is_empty() {
            return Err("fixture 不能为空".to_owned());
        }
        if fingerprint.tracks == 0 {
            return Err("tracks 必须 >= 1".to_owned());
        }
        if fingerprint.frames == 0 {
            return Err("frames 必须 >= 1".to_owned());
        }
        if fingerprint.channels == 0 {
            return Err("channels 必须 >= 1".to_owned());
        }
        if fingerprint.sample_rate == 0 {
            return Err("sample_rate 必须 >= 1".to_owned());
        }
        if fingerprint.block_size == 0 {
            return Err("block_size 必须 >= 1".to_owned());
        }
        let expected = fingerprint
            .channels
            .checked_mul(usize::try_from(fingerprint.frames).unwrap_or(usize::MAX))
            .ok_or_else(|| "frames * channels 溢出".to_owned())?;
        if self.sample_count != expected as u64 {
            return Err(format!(
                "sample_count ({}) != frames * channels ({expected})",
                self.sample_count
            ));
        }
        if self.probes.len() as u64 != self.sample_count {
            return Err(format!(
                "样本表不完整: probes.len() = {}, sample_count = {}",
                self.probes.len(),
                self.sample_count
            ));
        }
        for (position, probe) in self.probes.iter().enumerate() {
            if probe.index != position as u64 {
                return Err(format!(
                    "样本表下标不连续: 第 {position} 项的下标是 {}（应为 {position}）",
                    probe.index
                ));
            }
            if !f32::from_bits(probe.bits).is_finite() {
                return Err(format!(
                    "样本 {} 非有限（位型 {:08x}）: 母带里出现 NaN/Inf 是渲染失败, 不是可对账的读数",
                    probe.index, probe.bits
                ));
            }
        }
        if !self.abs_max.is_finite() || self.abs_max < 0.0 {
            return Err(format!("abs_max 非法: {}", self.abs_max));
        }
        if !self.sum_squares.is_finite() || self.sum_squares < 0.0 {
            return Err(format!("sum_squares 非法: {}", self.sum_squares));
        }
        if self.ops.is_empty() {
            return Err("op 声明不能为空: 收据必须显式列出路径上的运算类别".to_owned());
        }
        if !self.ops.iter().any(|op| op.class == SampleClass::IeeeExact) {
            return Err("缺少 `op E ...`: 路径上不可能只有超越函数".to_owned());
        }
        let tainted = self
            .probes
            .iter()
            .any(|probe| probe.class == SampleClass::Transcendental);
        if tainted && !self.has_transcendental_ops() {
            return Err("收据自相矛盾: 有 `T` 类样本, 却没有任何 `op T ...` 声明".to_owned());
        }
        if self.longest_path_frames != self.pdc_expected_frames {
            return Err(format!(
                "PDC 读数不自洽: longest_path_frames = {} 而导出侧复算 = {} [ARCH-PDC-001]",
                self.longest_path_frames, self.pdc_expected_frames
            ));
        }
        if self.toolchain.target_arch.is_empty() {
            return Err("target_arch 不能为空 (收据必须自证架构)".to_owned());
        }
        if self.toolchain.target_triple.is_empty() {
            return Err("target_triple 不能为空 (收据必须自证架构)".to_owned());
        }
        Ok(())
    }
}

/// `[u8; 32]` → 64 位小写十六进制。
#[must_use]
pub fn digest_hex(digest: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in digest {
        write!(out, "{byte:02x}").expect("写入 String 不会失败");
    }
    out
}

/// 统计量：`(样本数, max|s|, Σs²)`，全部用 `f64` **顺序**累加（IEEE 精确、与机器无关）。
#[must_use]
pub fn stats_of(samples: &[f32]) -> (u64, f64, f64) {
    let mut abs_max = 0.0f64;
    let mut sum_squares = 0.0f64;
    for sample in samples {
        let value = f64::from(*sample);
        let magnitude = value.abs();
        if magnitude > abs_max {
            abs_max = magnitude;
        }
        sum_squares += value * value;
    }
    (samples.len() as u64, abs_max, sum_squares)
}

/// 把整条母带缓冲包成**同类**样本表（本线的两个夹具都只有一个类别）。
#[must_use]
pub fn probes_all(samples: &[f32], class: SampleClass) -> Vec<Probe> {
    samples
        .iter()
        .enumerate()
        .map(|(index, sample)| Probe {
            index: index as u64,
            class,
            bits: sample.to_bits(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 序列化
// ---------------------------------------------------------------------------

/// 把收据渲染成**逐字节确定**的文本（不含时间戳：同一个输入两次导出必须完全相同）。
#[must_use]
pub fn to_text(receipt: &Receipt) -> String {
    let mut out = String::new();
    emitln!(out, "{SCHEMA_LINE}");
    emitln!(out, "fixture {}", receipt.fingerprint.fixture);
    emitln!(out, "tracks {}", receipt.fingerprint.tracks);
    emitln!(out, "frames {}", receipt.fingerprint.frames);
    emitln!(out, "channels {}", receipt.fingerprint.channels);
    emitln!(out, "sample_rate {}", receipt.fingerprint.sample_rate);
    emitln!(out, "block_size {}", receipt.fingerprint.block_size);
    emitln!(out, "threads {}", receipt.fingerprint.threads.to_token());
    emitln!(out, "seed {}", receipt.fingerprint.seed);
    emitln!(out, "gain_db {}", receipt.fingerprint.gain.to_token());
    for (node, frames) in &receipt.fingerprint.latency {
        emitln!(out, "latency {node} {frames}");
    }
    let toolchain = &receipt.toolchain;
    emitln!(out, "target_arch {}", toolchain.target_arch);
    emitln!(out, "target_os {}", toolchain.target_os);
    emitln!(out, "target_env {}", empty_as_dash(&toolchain.target_env));
    emitln!(out, "target_endian {}", toolchain.target_endian);
    emitln!(
        out,
        "target_pointer_width {}",
        toolchain.target_pointer_width
    );
    emitln!(out, "target_triple {}", toolchain.target_triple);
    emitln!(out, "rustc_version {}", toolchain.rustc_version);
    emitln!(out, "digest {}", receipt.digest_hex());
    emitln!(out, "longest_path_frames {}", receipt.longest_path_frames);
    emitln!(out, "pdc_expected_frames {}", receipt.pdc_expected_frames);
    emitln!(out, "sample_count {}", receipt.sample_count);
    emitln!(out, "abs_max {:.17e}", receipt.abs_max);
    emitln!(out, "sum_squares {:.17e}", receipt.sum_squares);
    // 类别声明：**无条件**两条都写 —— "本路径没有超越函数"本身就是一个必须显式声明的结论。
    for class in [SampleClass::IeeeExact, SampleClass::Transcendental] {
        emitln!(out, "class {} {}", class.code(), class.name());
    }
    for op in &receipt.ops {
        emitln!(out, "op {} {}", op.class.code(), op.op);
    }
    let mut cursor = 0usize;
    while cursor < receipt.probes.len() {
        let class = receipt.probes[cursor].class;
        let mut end = cursor + 1;
        while end < receipt.probes.len()
            && receipt.probes[end].class == class
            && end - cursor < SAMPLES_PER_LINE
        {
            end += 1;
        }
        emit!(
            out,
            "sample {} {}",
            receipt.probes[cursor].index,
            class.code()
        );
        for (offset, probe) in receipt.probes[cursor..end].iter().enumerate() {
            if offset == 0 {
                emit!(out, " {:08x}", probe.bits);
            } else {
                emit!(out, ",{:08x}", probe.bits);
            }
        }
        out.push('\n');
        cursor = end;
    }
    out
}

/// 空串写 `-`（避免行尾空白，同时让"空值"这件事显式可见）。
fn empty_as_dash(value: &str) -> &str {
    if value.is_empty() { "-" } else { value }
}

// ---------------------------------------------------------------------------
// 解析
// ---------------------------------------------------------------------------

/// 解析失败：`line == 0` 表示"整份收据"层面的问题（缺字段、自洽性失败）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// 1 起的行号；0 = 整份收据。
    pub line: usize,
    /// 人读的说明。
    pub message: String,
}

impl ParseError {
    fn new(line: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            message: message.into(),
        }
    }
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.line == 0 {
            write!(f, "收据非法: {}", self.message)
        } else {
            write!(f, "收据第 {} 行非法: {}", self.line, self.message)
        }
    }
}

impl std::error::Error for ParseError {}

/// 严格解析一份收据文本。**不 panic**：任何坏输入都变成 [`ParseError`]。
///
/// # Errors
///
/// 见 [`ParseError`]。
#[allow(clippy::too_many_lines)]
pub fn parse(text: &str) -> Result<Receipt, ParseError> {
    let mut parser = Parser::default();
    for (offset, raw) in text.lines().enumerate() {
        let line = offset + 1;
        parser.last_line = line;
        let content = raw.strip_suffix('\r').unwrap_or(raw);
        if content.is_empty() || content.starts_with('#') {
            continue;
        }
        if !parser.seen_schema {
            if content != SCHEMA_LINE {
                return Err(ParseError::new(
                    line,
                    format!("首行必须是 `{SCHEMA_LINE}`, 实际是 `{content}`"),
                ));
            }
            parser.seen_schema = true;
            continue;
        }
        let Some((key, value)) = content.split_once(' ') else {
            return Err(ParseError::new(
                line,
                format!("行里没有空格分隔的键与值: `{content}`"),
            ));
        };
        parser.field(line, key, value)?;
    }
    parser.finish()
}

#[derive(Default)]
struct Parser {
    last_line: usize,
    seen_schema: bool,
    fixture: Option<String>,
    tracks: Option<u32>,
    frames: Option<u64>,
    channels: Option<usize>,
    sample_rate: Option<u32>,
    block_size: Option<usize>,
    threads: Option<Threads>,
    seed: Option<u64>,
    gain: Option<EdgeGain>,
    latency: BTreeMap<String, u32>,
    target_arch: Option<String>,
    target_os: Option<String>,
    target_env: Option<String>,
    target_endian: Option<String>,
    target_pointer_width: Option<String>,
    target_triple: Option<String>,
    rustc_version: Option<String>,
    digest: Option<[u8; 32]>,
    longest_path_frames: Option<u32>,
    pdc_expected_frames: Option<u32>,
    sample_count: Option<u64>,
    abs_max: Option<f64>,
    sum_squares: Option<f64>,
    declared_ieee: bool,
    declared_transcendental: bool,
    ops: Vec<PathOp>,
    probes: Vec<Probe>,
    last_probe_end: Option<u64>,
}

impl Parser {
    fn field(&mut self, line: usize, key: &str, value: &str) -> Result<(), ParseError> {
        match key {
            "fixture" => self.fixture = Some(token(line, key, value)?.to_owned()),
            "tracks" => self.tracks = Some(number(line, key, value)?),
            "frames" => self.frames = Some(number(line, key, value)?),
            "channels" => self.channels = Some(number(line, key, value)?),
            "sample_rate" => self.sample_rate = Some(number(line, key, value)?),
            "block_size" => self.block_size = Some(number(line, key, value)?),
            "threads" => self.threads = Some(parse_threads(line, value)?),
            "seed" => self.seed = Some(number(line, key, value)?),
            "gain_db" => self.gain = Some(parse_gain(line, value)?),
            "latency" => self.latency(line, value)?,
            "target_arch" => self.target_arch = Some(token(line, key, value)?.to_owned()),
            "target_os" => self.target_os = Some(token(line, key, value)?.to_owned()),
            "target_env" => {
                let raw = token(line, key, value)?;
                self.target_env = Some(if raw == "-" {
                    String::new()
                } else {
                    raw.to_owned()
                });
            }
            "target_endian" => self.target_endian = Some(token(line, key, value)?.to_owned()),
            "target_pointer_width" => {
                self.target_pointer_width = Some(token(line, key, value)?.to_owned());
            }
            "target_triple" => self.target_triple = Some(token(line, key, value)?.to_owned()),
            "rustc_version" => {
                if value.trim().is_empty() {
                    return Err(ParseError::new(line, "rustc_version 不能为空"));
                }
                self.rustc_version = Some(value.trim_end().to_owned());
            }
            "digest" => self.digest = Some(parse_digest(line, value)?),
            "longest_path_frames" => self.longest_path_frames = Some(number(line, key, value)?),
            "pdc_expected_frames" => self.pdc_expected_frames = Some(number(line, key, value)?),
            "sample_count" => self.sample_count = Some(number(line, key, value)?),
            "abs_max" => self.abs_max = Some(decimal(line, key, value)?),
            "sum_squares" => self.sum_squares = Some(decimal(line, key, value)?),
            "class" => self.class(line, value)?,
            "op" => self.op(line, value)?,
            "sample" => self.sample(line, value)?,
            other => {
                return Err(ParseError::new(
                    line,
                    format!("未知字段 `{other}`（拼写错误? 或是本解析器不认识的版本）"),
                ));
            }
        }
        Ok(())
    }

    fn class(&mut self, line: usize, value: &str) -> Result<(), ParseError> {
        let (code, name) = value.split_once(' ').ok_or_else(|| {
            ParseError::new(line, format!("class 需要 `<类别码> <名字>`: `{value}`"))
        })?;
        let mut chars = code.chars();
        let (Some(code), None) = (chars.next(), chars.next()) else {
            return Err(ParseError::new(
                line,
                format!("类别码必须是单个字符: `{code}`"),
            ));
        };
        let Some(class) = SampleClass::from_code(code) else {
            return Err(ParseError::new(
                line,
                format!("未知类别码 `{code}`（只认 D32 的 E / T）"),
            ));
        };
        if name != class.name() {
            return Err(ParseError::new(
                line,
                format!(
                    "类别 {code} 的名字必须是 `{}`, 实际是 `{name}`",
                    class.name()
                ),
            ));
        }
        match class {
            SampleClass::IeeeExact => self.declared_ieee = true,
            SampleClass::Transcendental => self.declared_transcendental = true,
        }
        Ok(())
    }

    fn op(&mut self, line: usize, value: &str) -> Result<(), ParseError> {
        let (code, op) = value.split_once(' ').ok_or_else(|| {
            ParseError::new(line, format!("op 需要 `<类别码> <运算@位置>`: `{value}`"))
        })?;
        let mut chars = code.chars();
        let (Some(code), None) = (chars.next(), chars.next()) else {
            return Err(ParseError::new(
                line,
                format!("类别码必须是单个字符: `{code}`"),
            ));
        };
        let Some(class) = SampleClass::from_code(code) else {
            return Err(ParseError::new(
                line,
                format!("未知类别码 `{code}`（只认 D32 的 E / T）"),
            ));
        };
        if op.trim().is_empty() || op.contains(' ') {
            return Err(ParseError::new(line, format!("运算名非法: `{op}`")));
        }
        self.ops.push(PathOp {
            class,
            op: op.to_owned(),
        });
        Ok(())
    }

    fn latency(&mut self, line: usize, value: &str) -> Result<(), ParseError> {
        let (node, frames) = value.split_once(' ').ok_or_else(|| {
            ParseError::new(line, format!("latency 需要 `<ULID> <帧数>`: `{value}`"))
        })?;
        if !is_ulid(node) {
            return Err(ParseError::new(
                line,
                format!("latency 的节点键必须是 26 位 Crockford ULID 文本: `{node}`"),
            ));
        }
        let frames: u32 = frames
            .parse()
            .map_err(|_| ParseError::new(line, format!("latency 的帧数非法: `{frames}`")))?;
        if self.latency.insert(node.to_owned(), frames).is_some() {
            return Err(ParseError::new(line, format!("latency 节点重复: `{node}`")));
        }
        Ok(())
    }

    fn sample(&mut self, line: usize, value: &str) -> Result<(), ParseError> {
        let mut parts = value.splitn(3, ' ');
        let (Some(base), Some(code), Some(csv)) = (parts.next(), parts.next(), parts.next()) else {
            return Err(ParseError::new(
                line,
                format!("sample 需要 `<起始下标> <类别码> <位型,...>`: `{value}`"),
            ));
        };
        let base: u64 = base
            .parse()
            .map_err(|_| ParseError::new(line, format!("sample 起始下标非法: `{base}`")))?;
        let mut chars = code.chars();
        let (Some(code), None) = (chars.next(), chars.next()) else {
            return Err(ParseError::new(
                line,
                format!("类别码必须是单个字符: `{code}`"),
            ));
        };
        let Some(class) = SampleClass::from_code(code) else {
            return Err(ParseError::new(
                line,
                format!("未知类别码 `{code}`（只认 D32 的 E / T）"),
            ));
        };
        if let Some(previous_end) = self.last_probe_end
            && base <= previous_end
        {
            return Err(ParseError::new(
                line,
                format!("样本下标必须严格递增: {base} 不大于上一行的末尾 {previous_end}"),
            ));
        }
        let mut index = base;
        for item in csv.split(',') {
            let bits = parse_bits(item).ok_or_else(|| {
                ParseError::new(line, format!("样本位型必须是 8 位小写十六进制: `{item}`"))
            })?;
            self.probes.push(Probe { index, class, bits });
            index += 1;
        }
        self.last_probe_end = Some(index - 1);
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn finish(self) -> Result<Receipt, ParseError> {
        let line = self.last_line;
        if !self.seen_schema {
            return Err(ParseError::new(line, format!("缺少首行 `{SCHEMA_LINE}`")));
        }
        if !self.declared_ieee || !self.declared_transcendental {
            return Err(ParseError::new(
                line,
                "必须同时声明 `class E ieee-exact` 与 `class T transcendental`",
            ));
        }
        let receipt = Receipt {
            fingerprint: Fingerprint {
                fixture: required(self.fixture, line, "fixture")?,
                tracks: required(self.tracks, line, "tracks")?,
                frames: required(self.frames, line, "frames")?,
                channels: required(self.channels, line, "channels")?,
                sample_rate: required(self.sample_rate, line, "sample_rate")?,
                block_size: required(self.block_size, line, "block_size")?,
                threads: required(self.threads, line, "threads")?,
                seed: required(self.seed, line, "seed")?,
                gain: required(self.gain, line, "gain_db")?,
                latency: self.latency,
            },
            toolchain: Toolchain {
                target_arch: required(self.target_arch, line, "target_arch")?,
                target_os: required(self.target_os, line, "target_os")?,
                target_env: required(self.target_env, line, "target_env")?,
                target_endian: required(self.target_endian, line, "target_endian")?,
                target_pointer_width: required(
                    self.target_pointer_width,
                    line,
                    "target_pointer_width",
                )?,
                target_triple: required(self.target_triple, line, "target_triple")?,
                rustc_version: required(self.rustc_version, line, "rustc_version")?,
            },
            digest: required(self.digest, line, "digest")?,
            longest_path_frames: required(self.longest_path_frames, line, "longest_path_frames")?,
            pdc_expected_frames: required(self.pdc_expected_frames, line, "pdc_expected_frames")?,
            sample_count: required(self.sample_count, line, "sample_count")?,
            abs_max: required(self.abs_max, line, "abs_max")?,
            sum_squares: required(self.sum_squares, line, "sum_squares")?,
            ops: self.ops,
            probes: self.probes,
        };
        receipt
            .validate()
            .map_err(|message| ParseError::new(line, message))?;
        Ok(receipt)
    }
}

fn required<T>(slot: Option<T>, line: usize, name: &str) -> Result<T, ParseError> {
    slot.ok_or_else(|| ParseError::new(line, format!("缺少必填字段 `{name}`")))
}

/// 单记号值（不含空格、非空）。
fn token<'a>(line: usize, key: &str, value: &'a str) -> Result<&'a str, ParseError> {
    if value.is_empty() || value.contains(' ') {
        return Err(ParseError::new(
            line,
            format!("`{key}` 需要单个不含空格的记号, 实际是 `{value}`"),
        ));
    }
    Ok(value)
}

fn number<T: std::str::FromStr>(line: usize, key: &str, value: &str) -> Result<T, ParseError> {
    let value = token(line, key, value)?;
    value
        .parse()
        .map_err(|_| ParseError::new(line, format!("`{key}` 不是合法数字: `{value}`")))
}

fn decimal(line: usize, key: &str, value: &str) -> Result<f64, ParseError> {
    let value = token(line, key, value)?;
    value
        .parse()
        .map_err(|_| ParseError::new(line, format!("`{key}` 不是合法浮点数: `{value}`")))
}

fn parse_threads(line: usize, value: &str) -> Result<Threads, ParseError> {
    if value == "auto" {
        return Ok(Threads::Auto);
    }
    let count: usize = value.parse().map_err(|_| {
        ParseError::new(line, format!("`threads` 必须是 `auto` 或正整数: `{value}`"))
    })?;
    if count == 0 {
        return Err(ParseError::new(line, "`threads` 不能是 0"));
    }
    Ok(Threads::Fixed(count))
}

fn parse_gain(line: usize, value: &str) -> Result<EdgeGain, ParseError> {
    if value == "none" {
        return Ok(EdgeGain::Identity);
    }
    let db: f32 = value
        .parse()
        .map_err(|_| ParseError::new(line, format!("`gain_db` 必须是 `none` 或 f32: `{value}`")))?;
    if !db.is_finite() {
        return Err(ParseError::new(
            line,
            format!("`gain_db` 非有限: `{value}`"),
        ));
    }
    Ok(EdgeGain::Db(db))
}

/// 8 位**小写**十六进制 `f32` 位型。
fn parse_bits(text: &str) -> Option<u32> {
    if text.len() != 8 {
        return None;
    }
    let mut value = 0u32;
    for byte in text.bytes() {
        let digit = match byte {
            b'0'..=b'9' => u32::from(byte - b'0'),
            b'a'..=b'f' => u32::from(byte - b'a') + 10,
            _ => return None,
        };
        value = (value << 4) | digit;
    }
    Some(value)
}

fn parse_digest(line: usize, value: &str) -> Result<[u8; 32], ParseError> {
    if value.len() != 64 {
        return Err(ParseError::new(
            line,
            format!("digest 必须是 64 位十六进制, 实际 {} 位", value.len()),
        ));
    }
    let mut out = [0u8; 32];
    let bytes = value.as_bytes();
    for (index, slot) in out.iter_mut().enumerate() {
        let hi = hex_nibble(bytes[index * 2]);
        let lo = hex_nibble(bytes[index * 2 + 1]);
        let (Some(hi), Some(lo)) = (hi, lo) else {
            return Err(ParseError::new(
                line,
                format!("digest 含非小写十六进制字符: `{value}`"),
            ));
        };
        *slot = (hi << 4) | lo;
    }
    Ok(out)
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

/// 26 位 Crockford Base32 ULID 文本（`I`/`L`/`O`/`U` 不在字母表里）。
fn is_ulid(text: &str) -> bool {
    text.len() == 26
        && text
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'A'..=b'H' | b'J' | b'K' | b'M' | b'N' | b'P'..=b'T' | b'V'..=b'Z'))
}

// ---------------------------------------------------------------------------
// 判决
// ---------------------------------------------------------------------------

/// 判决结论。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// 位级完全相同（`MUST-GATE-002` 的跨机器形态，或 `MUST-GATE-003` 的最好情况）。
    L1BitExact,
    /// 差异只落在超越函数类路径上，且同时满足 `MUST-GATE-003` 的绝对预算与 D32 的 ulp 预算。
    L2WithinBudget,
    /// 不通过（IEEE 精确类出现差异 / 绝对预算超 / D32 ulp 预算超）。
    Fail,
}

impl Verdict {
    /// 机器可读的判决名。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::L1BitExact => "L1-bit-exact",
            Self::L2WithinBudget => "L2-within-budget",
            Self::Fail => "FAIL",
        }
    }

    /// CI 退出码：`0` = 通过（含 L2 预算内），`1` = `FAIL`。
    #[must_use]
    pub const fn exit_code(self) -> i32 {
        match self {
            Self::L1BitExact | Self::L2WithinBudget => 0,
            Self::Fail => 1,
        }
    }
}

/// 一份被点名的越界样本（判据 ⑤ 的"点名"要求）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Offender {
    /// 样本下标。
    pub index: u64,
    /// 样本类别。
    pub class: SampleClass,
    /// 收据 A 的位型。
    pub a_bits: u32,
    /// 收据 B 的位型。
    pub b_bits: u32,
    /// 绝对误差。
    pub abs_diff: f64,
    /// ulp 距离。
    pub ulp: u64,
}

/// 判决结果（纯数据，可被任何调用方重新格式化）。
#[derive(Clone, Debug, PartialEq)]
pub struct Judgement {
    /// 结论。
    pub verdict: Verdict,
    /// 机器可读的原因码。
    pub reason: &'static str,
    /// 该判决服务哪条门禁（同目标 = `MUST-GATE-002`，跨目标 = `MUST-GATE-003`）。
    pub gate: &'static str,
    /// 两份收据是否来自同一个目标三元组。
    pub same_target: bool,
    /// 两份收据的 `threads` 是否不同（**不影响**判决，只如实报告）。
    pub threads_differ: bool,
    /// digest 是否相同。
    pub digest_equal: bool,
    /// 样本位型是否全同。
    pub samples_equal: bool,
    /// 参与逐样本比较的样本数。
    pub compared: u64,
    /// 最大绝对误差。
    pub max_abs_diff: f64,
    /// 最大绝对误差所在样本下标。
    pub max_abs_index: Option<u64>,
    /// 最大绝对误差所在样本类别。
    pub max_abs_class: Option<SampleClass>,
    /// 最大 ulp 距离。
    pub max_ulp: u64,
    /// 最大 ulp 距离所在样本下标。
    pub max_ulp_index: Option<u64>,
    /// IEEE 精确类上的差异样本数（必须为 0）。
    pub ieee_diffs: u64,
    /// 超越函数类上的差异样本数。
    pub transcendental_diffs: u64,
    /// 被点名的越界样本（前 [`MAX_OFFENDERS`] 个）。
    pub offenders: Vec<Offender>,
    /// 越界样本总数（不截断）。
    pub offenders_total: u64,
    /// 绝对预算是否满足。
    pub abs_budget_ok: bool,
    /// D32 的 ulp 预算是否满足。
    pub ulp_budget_ok: bool,
}

/// 两份收据**不可比**（而不是"比较结果是红"）：退出码 2。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JudgeError {
    /// 指纹/路径声明不同 —— 两份收据描述的不是同一件事，任何结论都是假的。
    Incomparable(String),
    /// 某一份收据自相矛盾。
    Malformed {
        /// 哪一份（`"a"` / `"b"`）。
        side: &'static str,
        /// 说明。
        message: String,
    },
}

impl core::fmt::Display for JudgeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Incomparable(message) => write!(f, "两份收据不可比: {message}"),
            Self::Malformed { side, message } => write!(f, "收据 {side} 自相矛盾: {message}"),
        }
    }
}

impl std::error::Error for JudgeError {}

impl JudgeError {
    fn incomparable(message: impl Into<String>) -> Self {
        Self::Incomparable(message.into())
    }
}

/// 比较两份收据并给出结构化判决。
///
/// 判决顺序（每一步都对应一条规范）：
///
/// 1. `digest` 与逐样本位型**都**相同 ⇒ [`Verdict::L1BitExact`]；
/// 2. 两者不一致（digest 同但样本异 / 反之）⇒ 收据自相矛盾 ⇒ [`JudgeError`]；
/// 3. 任何 **`E`（IEEE 精确类）** 样本不同 ⇒ [`Verdict::Fail`]（D32：这类跨架构零容差）；
/// 4. 最大绝对误差 `>= 1e-6` ⇒ [`Verdict::Fail`]（`MUST-GATE-003` 的预算）；
/// 5. 最大 ulp 距离 `> 4096` ⇒ [`Verdict::Fail`]（D32 的 ulp 预算）；
/// 6. 否则 ⇒ [`Verdict::L2WithinBudget`]。
///
/// # Errors
///
/// 见 [`JudgeError`]（此时**没有**判决可言，调用方应以退出码 2 处理）。
#[allow(clippy::too_many_lines)]
pub fn judge(a: &Receipt, b: &Receipt) -> Result<Judgement, JudgeError> {
    a.validate()
        .map_err(|message| JudgeError::Malformed { side: "a", message })?;
    b.validate()
        .map_err(|message| JudgeError::Malformed { side: "b", message })?;
    compare_fingerprints(&a.fingerprint, &b.fingerprint)?;
    if a.ops != b.ops {
        return Err(JudgeError::incomparable(
            "路径运算声明不同 —— 两份收据的类别语义不可比（先用同一个 exporter 重新导出）",
        ));
    }

    let mut judgement = Judgement {
        verdict: Verdict::Fail,
        reason: "",
        gate: if a.toolchain.target_triple == b.toolchain.target_triple {
            "MUST-GATE-002"
        } else {
            "MUST-GATE-003"
        },
        same_target: a.toolchain.target_triple == b.toolchain.target_triple,
        threads_differ: a.fingerprint.threads != b.fingerprint.threads,
        digest_equal: a.digest == b.digest,
        samples_equal: true,
        compared: a.probes.len() as u64,
        max_abs_diff: 0.0,
        max_abs_index: None,
        max_abs_class: None,
        max_ulp: 0,
        max_ulp_index: None,
        ieee_diffs: 0,
        transcendental_diffs: 0,
        offenders: Vec::new(),
        offenders_total: 0,
        abs_budget_ok: true,
        ulp_budget_ok: true,
    };

    for (probe_a, probe_b) in a.probes.iter().zip(&b.probes) {
        if probe_a.bits == probe_b.bits {
            continue;
        }
        judgement.samples_equal = false;
        let value_a = f32::from_bits(probe_a.bits);
        let value_b = f32::from_bits(probe_b.bits);
        let abs_diff = (f64::from(value_a) - f64::from(value_b)).abs();
        let ulp = ulp_distance(value_a, value_b);
        match probe_a.class {
            SampleClass::IeeeExact => judgement.ieee_diffs += 1,
            SampleClass::Transcendental => judgement.transcendental_diffs += 1,
        }
        if abs_diff > judgement.max_abs_diff {
            judgement.max_abs_diff = abs_diff;
            judgement.max_abs_index = Some(probe_a.index);
            judgement.max_abs_class = Some(probe_a.class);
        }
        if ulp > judgement.max_ulp {
            judgement.max_ulp = ulp;
            judgement.max_ulp_index = Some(probe_a.index);
        }
        judgement.offenders_total += 1;
        if judgement.offenders.len() < MAX_OFFENDERS {
            judgement.offenders.push(Offender {
                index: probe_a.index,
                class: probe_a.class,
                a_bits: probe_a.bits,
                b_bits: probe_b.bits,
                abs_diff,
                ulp,
            });
        }
    }

    if judgement.digest_equal != judgement.samples_equal {
        return Err(JudgeError::incomparable(
            "digest 与样本表互相矛盾（一个说相同、一个说不同）—— 收据被篡改或不完整",
        ));
    }
    judgement.abs_budget_ok = judgement.max_abs_diff < ABS_LIMIT;
    judgement.ulp_budget_ok = judgement.max_ulp <= ULP_LIMIT;

    if judgement.samples_equal {
        judgement.verdict = Verdict::L1BitExact;
        judgement.reason = "digests-and-samples-identical";
        return Ok(judgement);
    }
    if judgement.ieee_diffs > 0 {
        judgement.verdict = Verdict::Fail;
        judgement.reason = "ieee-exact-sample-differs";
        return Ok(judgement);
    }
    if !judgement.abs_budget_ok {
        judgement.verdict = Verdict::Fail;
        judgement.reason = "abs-budget-exceeded";
        return Ok(judgement);
    }
    if !judgement.ulp_budget_ok {
        judgement.verdict = Verdict::Fail;
        judgement.reason = "d32-ulp-budget-exceeded";
        return Ok(judgement);
    }
    judgement.verdict = Verdict::L2WithinBudget;
    judgement.reason = "transcendental-only-within-budget";
    Ok(judgement)
}

fn compare_fingerprints(a: &Fingerprint, b: &Fingerprint) -> Result<(), JudgeError> {
    if a.fixture != b.fixture {
        return Err(JudgeError::incomparable(format!(
            "fixture 不同: `{}` vs `{}`",
            a.fixture, b.fixture
        )));
    }
    if a.tracks != b.tracks || a.frames != b.frames || a.channels != b.channels {
        return Err(JudgeError::incomparable(format!(
            "工程形状不同: tracks {}/{}, frames {}/{}, channels {}/{}",
            a.tracks, b.tracks, a.frames, b.frames, a.channels, b.channels
        )));
    }
    if a.sample_rate != b.sample_rate || a.block_size != b.block_size {
        return Err(JudgeError::incomparable(format!(
            "采样/块长不同: sample_rate {}/{}, block_size {}/{}",
            a.sample_rate, b.sample_rate, a.block_size, b.block_size
        )));
    }
    if a.seed != b.seed {
        return Err(JudgeError::incomparable(format!(
            "种子不同: {} vs {}",
            a.seed, b.seed
        )));
    }
    if a.gain != b.gain {
        return Err(JudgeError::incomparable(format!(
            "边增益设置不同: {} vs {}",
            a.gain.to_token(),
            b.gain.to_token()
        )));
    }
    if a.latency != b.latency {
        return Err(JudgeError::incomparable(
            "注入的延迟表不同 —— PDC 补偿不同, 样本本就应当不同",
        ));
    }
    // 刻意**不**比较 threads: [ARCH-DET-002] 的实测结论就是"线程数不改变母带字节",
    // 比较器必须能表达"1 线程 vs 4 线程"这件事。
    Ok(())
}

/// IEEE-754 `f32` 之间的 ulp 距离（把位型映射成单调的整数序）。
///
/// `+0.0` 与 `-0.0` 的映射值相同（距离 0）—— 它们数值相等但位型不同，
/// 那种情况由**位级**比较负责，不由 ulp 负责。
fn ulp_distance(a: f32, b: f32) -> u64 {
    fn key(value: f32) -> i64 {
        let bits = value.to_bits() as i32;
        if bits < 0 {
            i64::from(i32::MIN) - i64::from(bits)
        } else {
            i64::from(bits)
        }
    }
    let (left, right) = (key(a), key(b));
    left.abs_diff(right)
}

// ---------------------------------------------------------------------------
// 报告
// ---------------------------------------------------------------------------

/// 把判决渲染成 `key=value` 文本（首行 `VERDICT <判决>`，便于 CI 直接 grep）。
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn report(a: &Receipt, b: &Receipt, judgement: &Judgement) -> String {
    let mut out = String::new();
    emitln!(out, "VERDICT {}", judgement.verdict.as_str());
    emitln!(out, "reason={}", judgement.reason);
    emitln!(out, "gate={}", judgement.gate);
    emitln!(out, "schema={SCHEMA_LINE}");
    emitln!(out, "fixture={}", a.fingerprint.fixture);
    emitln!(out, "target_a={}", a.toolchain.target_triple);
    emitln!(out, "target_b={}", b.toolchain.target_triple);
    emitln!(out, "same_target={}", judgement.same_target);
    emitln!(out, "threads_a={}", a.fingerprint.threads.to_token());
    emitln!(out, "threads_b={}", b.fingerprint.threads.to_token());
    emitln!(out, "threads_differ={}", judgement.threads_differ);
    emitln!(out, "gain_db={}", a.fingerprint.gain.to_token());
    emitln!(out, "transcendental_ops={}", a.has_transcendental_ops());
    emitln!(out, "digest_a={}", a.digest_hex());
    emitln!(out, "digest_b={}", b.digest_hex());
    emitln!(out, "digest_equal={}", judgement.digest_equal);
    emitln!(out, "samples_equal={}", judgement.samples_equal);
    emitln!(out, "sample_count={}", a.sample_count);
    emitln!(out, "compared_samples={}", judgement.compared);
    emitln!(out, "abs_max_a={:.17e}", a.abs_max);
    emitln!(out, "abs_max_b={:.17e}", b.abs_max);
    emitln!(out, "sum_squares_a={:.17e}", a.sum_squares);
    emitln!(out, "sum_squares_b={:.17e}", b.sum_squares);
    emitln!(out, "longest_path_frames_a={}", a.longest_path_frames);
    emitln!(out, "longest_path_frames_b={}", b.longest_path_frames);
    emitln!(out, "pdc_expected_frames_a={}", a.pdc_expected_frames);
    emitln!(out, "pdc_expected_frames_b={}", b.pdc_expected_frames);
    emitln!(out, "max_abs_diff={:.17e}", judgement.max_abs_diff);
    emitln!(
        out,
        "max_abs_diff_index={}",
        optional_index(judgement.max_abs_index)
    );
    emitln!(
        out,
        "max_abs_diff_class={}",
        optional_class(judgement.max_abs_class)
    );
    emitln!(out, "max_ulp_diff={}", judgement.max_ulp);
    emitln!(
        out,
        "max_ulp_diff_index={}",
        optional_index(judgement.max_ulp_index)
    );
    emitln!(out, "limit_abs={ABS_LIMIT:.17e}");
    emitln!(out, "limit_ulp={ULP_LIMIT}");
    emitln!(out, "abs_budget_ok={}", judgement.abs_budget_ok);
    emitln!(out, "ulp_budget_ok={}", judgement.ulp_budget_ok);
    emitln!(out, "ieee_exact_differences={}", judgement.ieee_diffs);
    emitln!(
        out,
        "transcendental_differences={}",
        judgement.transcendental_diffs
    );
    emitln!(out, "offenders_total={}", judgement.offenders_total);
    emitln!(out, "offenders_shown={}", judgement.offenders.len());
    for offender in &judgement.offenders {
        emitln!(
            out,
            "offender index={} class={} a={:08x} b={:08x} abs={:.17e} ulp={}",
            offender.index,
            offender.class.code(),
            offender.a_bits,
            offender.b_bits,
            offender.abs_diff,
            offender.ulp
        );
    }
    out
}

fn optional_index(index: Option<u64>) -> String {
    index.map_or_else(|| "-".to_owned(), |value| value.to_string())
}

fn optional_class(class: Option<SampleClass>) -> String {
    class.map_or_else(|| "-".to_owned(), |value| value.code().to_string())
}
