//! Logic Pro (`.logicx`) bundle 的**只读探针** [ARCH-FMT-002] / #143。
//!
//! 本模块只在**非默认** feature `experimental-logic-import` 下存在
//! （见 `crates/yeban-render/Cargo.toml` 的 `[features]`）。AGENTS.md §2 红线 6 要求
//! 实验性互操作能力不得进默认 release 构建，因此 `default` 保持为空。
//!
//! ## ⛔ 本模块不写工程
//!
//! 本模块**不写**工程、**不**产出 `.yeban` 文档、**不**创建片段、**不**触碰任何
//! demo 文件。它只读 Apple 演示工程的字节：`inspect_*` 把实测读数
//! （[`LogicInspection`]）交给判据，[`import_bundle`] 把"要建工程的那些字节"
//! （区域摆放 + 音符）交给**导入器**。真正的导入器住在 `yeban-app` 的
//! `import_logic`（建 `YebanProjectV1` + 原子落盘）—— 本模块不认识模型 crate。
//!
//! ## 读的是什么（8/8 Apple demo 实测，见 `#139`）
//!
//! | 文件 | 读取口径 |
//! | :--- | :--- |
//! | `Resources/ProjectInformation.plist` | bplist00 字典的 `ActiveVariant`（int）⇒ 目录是 `Alternatives/{:03}`，⛔ **不硬编码 `000`**；该键缺失时（`#194` 实测：负责人的 `track3.logicx` 没有它）**只在 `Alternatives/` 下恰好一个三位目录时**按目录唯一性取那个目录，0 个/多个仍报错 |
//! | `Alternatives/<NNN>/MetaData.plist` | bplist00 字典的**七个必需键**：`NumberOfTracks` / `BeatsPerMinute` / `SampleRate` / `SongSignatureNumerator` / `SongSignatureDenominator` / `SongKey` / `Version` |
//! | `Alternatives/<NNN>/ProjectData` | 自研分块二进制：根头 0x18 字节、单条记录头 0x24 字节（载荷长 `u32` 小端 @+0x1c），**按载荷长度走到 EOF**；⛔ 不用魔数/EOF 猜切分（魔数在文件里出现 2–3 次） |
//!
//! 计数口径（每条都是**实测**钉住的，判据在 `tests/logic_inspect.rs`）：
//!
//! * **轨道**：落盘 tag `karT`（可读名 `Trak`）且对象号 `0x0004_0000` 的记录里，
//!   `payload[0] == 1` 的条数。`payload[0] == 0` 的条**不是轨道**（不过滤会让
//!   Colors 从 135 变成 138）；`payload[0] == 3` 恰一条 master（也**不计入**）。
//! * **区域**：落盘 tag `qeSM`（可读名 `MSeq`）的记录。名字在载荷 `+0x10` 是
//!   `u16` 小端**字节数**、`+0x12` 起是 UTF-8。**内部区域**（以字面 `*Automation`
//!   结尾，或名字恰为 `RBA Sequence`）不计入 [`LogicInspection::regions`]，
//!   只计入 [`LogicInspection::internal_regions`]。⚠ 名字恰为 `Automation` 的条
//!   **不是**内部区域（8 台 demo 里有 6 台各有一条；把它过滤掉会让 1074 变成 1068）。
//! * **音符**：所有 `qSvE`（可读名 `EvSq`）载荷交给
//!   [`crate::logic::decode_note_lines`] —— **复用**仓内既有的事件行模型，⛔ 不复制。
//!   只有 `payload[0] == 0x90` 的事件行会被算成音符，因此拍号/速度的 `qSvE`
//!   （载荷 96 / 48 字节）不会污染计数。
//! * **`AuRg`**：落盘 tag `gRuA` 的记录条数；`#152` 起同时逐条解出**名字**
//!   （载荷 `+0x4a`）、**长度帧**（`+0x16`）与**文件对象号**（记录头 `+0x08`，
//!   与 `lFuA` 同号）。⚠ 起点**不在** `AuRg` 载荷里：它在音频摆放事件 `+0x04`。
//!
//! ## 区域摆放：起点 `rest+0xe0`、长度 `rest+0x3c`（`#148` 实测，8/8）
//!
//! `qeSM` 载荷里除名字之外的字段落在名字之后的定长结构 `rest`（起点
//! [`region_rest_start`]）上。把它按偏移逐项扫过（`#148`），**只有一对**字段同时
//! 满足「起点、长度、单位」三条约束：
//!
//! | 字段 | 偏移 | 类型 | 单位 | 实测依据 |
//! | :--- | ---: | :--- | :--- | :--- |
//! | **起点** | `rest + 0xe0` | `u32` 小端 | **960 PPQ tick** | 等于编排 `qSvE` 摆放事件 `+0x04 − 34560`：Swing! 的 **17/17** 个非零摆放逐条相等 |
//! | **长度** | `rest + 0x3c` | `u32` 小端 | **960 PPQ tick** | `起点 + 长度` 与同轨下一条区域的起点连续：Swing! **17/17 对**成立（逐条 `start+length == next.start`）；有音符区域的上界比 8/8 台为 0.5517…1.0571 |
//!
//! 这两处正是 MIT 参考实现 §8.5 的**记录内** `+0x11c` / `+0x78` 的 `rest` 相对形式：
//! `记录内 = 0x24 + rest 起点 + rest 偏移`；供体 `down:` 的 rest 起点是 `0x18`
//! ⇒ `0x18 + 0x24 + 0xe0 = 0x11c`、`0x18 + 0x24 + 0x3c = 0x78`。
//!
//! ⚠ 读数口径（8 台 demo，逐条见 `tests/logic_inspect.rs`）：
//!
//! * **空区域**的长度是哨兵 [`LOGIC_REGION_EMPTY_LENGTH`] = **7,680,000**
//!   （= 8000 拍 = 2000 小节），且起点恒 0、音符恒 0 ⇒ 它不是真实长度，
//!   ⛔ 不得拿它算上界。
//! * 非空区域的长度有**有效范围** [`LOGIC_REGION_LENGTH_MIN`]..=[`LOGIC_REGION_LENGTH_MAX`]
//!   （60…2,196,480 tick；倍数的依据写在那两个常量的文档里）。8 台实测 **17 条**
//!   越界：**11 条**同一字面量 1,073,626,623（≈ 10.7 亿 tick ≈ 559,000 拍）＋ **6 条**
//!   4 tick ⇒ 导入时逐条打印 `kind=length-out-of-range` 并挡在工程之外（`#150`）。
//! * 记录流顺序是**对象创建顺序**，不是时间顺序 ⇒ 全局「区域起点非递减」**不成立**
//!   （8 台按 bundle 名排序的下降数：**95 / 76 / 77 / 105 / 104 / 45 / 6 / 149**；
//!   ⛔ 不要把它当判据）。把编排摆放事件按**轨道**分组后才是 0（Swing! 可解析的 170 组、0 处）。
//! * 有音符的区域里，`起点+长度` 对 `起点+最后音符结束` 的比值在 8/8 台是
//!   **0.5517…1.0571**（不是 1.0 整：Logic 的区域长度允许比内容短/长）。
//!
//! ## 轨道归属：摆放事件 `+0x14`（`#151` 实测；**不是** `qeSM` 字段）
//!
//! `qeSM` 载荷里**没有**轨道号字段：`rest + 0x00..0x100` 的 `u8` / `u16` / `u32`
//! 逐偏移扫过 8/8 台，**没有任何一个**同时满足「值落在 `1..=轨数`」与「每台 ≥ 2 个
//! 不同值」（`#151` 报告列出了被排除的候选：`rest+0xd8` 的小值以 `4`/`8`/`0xc` 为主
//! —— 最大到 12,472，落在 `1..=轨数` 的只有 4/116…157/200，不是 8/8；`rest+0xec` 只有
//! `0` 与 `0x10000`/`0x7fff_0000` 这类值，8/8 台的合法数都是 **0** —— 两者都不是轨道号）。
//! 轨道号只在**摆放事件**里：`qSvE` 载荷中定长 80 字节、`+0x00 == 0x20`（MIDI）的事件，
//!
//! | 字段 | 偏移 | 类型 | 实测读数（8/8 台） |
//! | :--- | ---: | :--- | :--- |
//! | 位置 | `+0x04` | `u32` 小端 | `34560 + 区域起点 tick`；与绑定区域的 `rest+0xe0` **逐条相等** |
//! | 链接 id | `+0x10` | `u32` 小端 | demo 实测 `80 / 88 / 92 / …`（步长 4） |
//! | **轨道号** | `+0x14` | **单字节** | **1 起**；绑定命中里 `1 ≤ 轨道号 ≤ 轨数` **651/651** |
//! | 区域槽 | `+0x20` | `u32` 小端 | = `qeSM` 记录头 `+0x08` 的对象号 `>> 16`；位置也与 `rest+0xe0` 相等 |
//!
//! 还有三个逐事件恒定字（8/8 台每一个 MIDI 摆放事件都相同，用于把别的 `qSvE`
//! 事件行排除掉）：`+0x17 == 0x89`、`+0x1c == 0x3fff_ffff`、`+0x24 == 0x8800_0000`。
//! `+0x24 == 0x8800_0000` 同时是 **MIDI 与音频摆放的区分**：音频事件是 `0xbc00_0000`。
//!
//! 绑定规则（可伪证、⛔ 不猜）：事件与区域**同时**满足 `+0x20` 槽相等、位置相等、
//! `1 ≤ 轨道号 ≤ 轨数` 才绑定；同一槽位出现互相矛盾的读数 ⇒ **不绑定**。
//!
//! ⚠ **不是每条可导入区域都有摆放事件**（8 台实测，`#151`）：
//!
//! | demo | 可导入区域 | 绑定到实测轨道 | 余下（无摆放事件） |
//! | :--- | ---: | ---: | ---: |
//! | Colors | 155 | 89 | 66 |
//! | MONTERO | 41 | 37 | 4 |
//! | MONTERO - Spatial Audio | 41 | 37 | 4 |
//! | Manzana | 113 | 107 | 6 |
//! | Manzana - Spatial Audio | 113 | 107 | 6 |
//! | Spatial Audio Demo Grid | 106 | 5 | 101 |
//! | Swing! | 174 | 164 | 10 |
//! | ocean eyes | 121 | 105 | 16 |
//! | **合计** | **864** | **651** | **213** |
//!
//! 余下的 213 条**没有任何摆放事件**（它们的起点与**另一条**已摆放区域相同；
//! 8 台里「名字与别的区域重复」的条数 / 余下条数 = **65/66、3/4、3/4、2/6、2/6、
//! 24/101、8/10、15/16** ⇒ 形态与"同一位置的多个 take／Track Alternatives"一致；
//! 本条**只登记**，⛔ 不发明它们属于哪条轨道）。因此导入器对它们保留轮转并逐条计数
//! （`#151` 的损失表行）。
//!
//! ## 版权（⛔ 不许提交 demo 或解析结果）
//!
//! Apple 演示工程有版权。本模块**只读** `/Library/Application Support/Logic/Logic Pro X
//! Demosongs/`（或调用方给的目录），判据在目录不存在时打印"无 demo、跳过"并返回，
//! **不红**；目录存在但解析失败 ⇒ **必须红**（⛔ 不静默跳过）。
//!
//! ## bplist00 读取子集
//!
//! 仓内只有**编码器**（`crate::logic` 的 `encode_binary_plist`）；读侧是本模块新写的
//! 最小实现，覆盖 dict / array / ASCII string / UTF-16 string / int / real / bool /
//! data。它按 trailer 的 `offset_int_size` / `object_ref_size` / 偏移表走对象，
//! 不依赖编码器的对象布局。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::str;

use crate::logic::{
    LOGIC_ARRANGE_MIDI_MARKER, LOGIC_FIRST_RECORD_OFFSET, LOGIC_KART_TAG,
    LOGIC_PLACEMENT_LINK_ID_OFFSET, LOGIC_PLACEMENT_POSITION_OFFSET, LOGIC_PLACEMENT_REGION_OFFSET,
    LOGIC_PLACEMENT_TRACK_OFFSET, LOGIC_RECORD_CLUSTER_OFFSET, LOGIC_RECORD_HEADER,
    LOGIC_RECORD_SIZE_OFFSET, LOGIC_REGION_LENGTH_FROM_REST, LOGIC_REGION_NAME_PAYLOAD_OFFSET,
    LOGIC_REGION_ORIGIN_TICKS, LOGIC_REGION_TAG, LOGIC_ROOT_HEADER, LOGIC_ROOT_MAGIC,
    LOGIC_SEQUENCE_TAG, decode_note_lines, region_rest_start,
};

/// Apple 演示工程在 macOS 上的**只读**默认位置（`#139` 实测，8/8）。
///
/// ⚠ 这是 Apple 的版权材料：本模块只读它，⛔ 不复制、不提交、不写回。
pub const DEFAULT_DEMOS_DIR: &str = "/Library/Application Support/Logic/Logic Pro X Demosongs";

/// `ProjectData` 里可读名为 `AuRg` 的音频区域记录（落盘字节是反序的 `gRuA`）。
///
/// `#152` 起逐条解析：名字 `+0x4a`、长度帧 `+0x16`、文件槽位 = 记录头对象号。
const AUDIO_REGION_TAG: [u8; 4] = *b"gRuA";

/// `AuRg` 音频区域载荷里名字字段的偏移：`u16` **小端字节数** @ **+0x4a**，
/// UTF-8 名字 @ **+0x4c**（`#152` 实测，8/8 台 **4,104/4,104** 条一致）。
pub const AUDIO_REGION_NAME_PAYLOAD_OFFSET: usize = 0x4A;

/// `AuRg` 音频区域载荷里**长度**的偏移：`u32` 小端，单位 = **音频帧**（不是 tick）。
///
/// `#152` 实测依据：① 源文件头声明的帧数 ≥ 该值（8/8 台、全部可解文件 0 例外）；
/// ② 用**素材自身采样率**换成 tick（`frames × 960 × bpm / (60 × rate)`）后，
/// Swing! **32/181**、Manzana **14/24** 条恰好落在 960 tick 的整数倍上（手算对照见报告）。
pub const AUDIO_REGION_LENGTH_FRAMES_OFFSET: usize = 0x16;

/// 音频摆放事件（`qSvE` 载荷里 `+0x00 == 0x24` 的 80 字节事件）里**区域槽**的偏移：
/// `u32` 小端 = `AuRg` 记录头 `+0x08` 的对象号 `>> 16`（`#152`）。
///
/// ⚠ MIDI 摆放事件的槽位在 `+0x20`，音频在 **`+0x2c`**（`+0x20` 在音频事件里不是槽位；
/// 实测它常是 `0xffff_ffff` 或一个与 `AuRg` 对象号无关的计数）。
pub const AUDIO_PLACEMENT_LINK_OFFSET: usize = 0x2C;

/// 音频摆放事件的类型标记（`qSvE` 载荷 `+0x00`）。
pub const AUDIO_PLACEMENT_MARKER: u32 = 0x24;

/// `lFuA`（可读名 `AuFl`）—— 音频文件引用记录（落盘字节是反序的 `lFuA`）。
const AUDIO_FILE_TAG: [u8; 4] = *b"lFuA";

/// 名字以这个字面后缀结尾的区域是 Logic 的自动化泳道，不计入音乐区域。
const AUTOMATION_NAME_SUFFIX: &str = "*Automation";

/// 这个名字的区域是 Logic 的 RBA 序列，不计入音乐区域。
const RBA_SEQUENCE_NAME: &str = "RBA Sequence";

/// `qeSM` 载荷里**区域起点**的偏移（相对名字之后的 `rest` 起点）：
/// `u32` 小端，单位 = **960 PPQ tick**。
///
/// `#148` 实测依据（三条）：① 与编排 `qSvE` 摆放事件的 `+0x04 − 34560` 逐条相等
/// （Swing! 17/17 非零）；② 有音符区域的 `起点+长度` 上界比 8/8 台为 0.5517…1.0571；
/// ③ 换算成记录内偏移 `0x24 + 0x18 + 0xe0 = 0x11c`，正是 MIT 参考实现 §8.5 的起点字段。
pub const LOGIC_REGION_START_FROM_REST: usize = 0xE0;

/// **空区域**的长度哨兵：7,680,000 tick = 8000 拍 = 2000 小节（4/4）。
///
/// 8 台 demo 里 114…486 条/台的空 `qeSM`（起点 0、音符 0，绝大多数是
/// `*Automation`）都用这个值；它**不是**真实长度，⛔ 不得参与上界计算。
pub const LOGIC_REGION_EMPTY_LENGTH: u32 = 7_680_000;

/// 一条**非空**区域长度的**下界**：60 tick = 1/64 拍（960 PPQ 下 960 / 16 = 60）。
///
/// `#150` 实测依据（8 台 demo，881 条"非内部、非空哨兵、非零"区域，逐条读数见
/// `tests/logic_inspect.rs`）：
///
/// * 8 台里有 **6 台**各带一条**逐字相同**的垃圾区域 `("", start=0, length=4, notes=0)`
///   （Swing! / MONTERO / MONTERO - Spatial Audio / Spatial Audio Demo Grid / Manzana /
///   Manzana - Spatial Audio）。4 tick = 1/240 拍，比 960 PPQ 能被记谱的最小音符
///   （1/64 拍 = 60 tick）还小 **15 倍** ⇒ 它装不下任何音符，不是真实长度。
/// * 8 台里**最小的一条正常区域**是 **432** tick（MONTERO 的 `post hum`，2 条），
///   是下界的 **7.2 倍**；第二小是 **732** tick（Colors 的 `MIDI Region`，3 条）。
///
/// ⇒ 下界取"能被记谱的最小音符（1/64 拍）"，**不是**"刚好挡住 4"：任何 ≥ 1/64 拍的
/// 真实区域（8/8 台实测最小 432）都保留，只有比最小音符还短的 4 tick 被挡。
pub const LOGIC_REGION_LENGTH_MIN: u32 = 60;

/// 一条**非空**区域长度的**上界**：2,196,480 tick = 2288 拍 = 572 小节（4/4）。
///
/// 依据是 **4 × 实测最大的正常长度**（8 台 demo 的 881 条候选逐条排序，去掉 11 条
/// 10.7 亿的垃圾后 = **870 条**）：
///
/// | 读数（`start+length`，tick） | 值 | 依据 |
/// | :--- | ---: | :--- |
/// | p50 | 192,000 | 870 条正常值 |
/// | p90 | 384,960 | 同上 |
/// | p99 | 491,520 | 同上 |
/// | p999 | 545,280 | 同上 |
/// | **最大正常值** | **549,120** = 572 拍 | MONTERO 的 `post hum`（同一值也是最大正常**长度**） |
/// | 垃圾值 | 1,073,626,623 = 1,118,361 拍 | 6/8 台共 **11 条**（`TRASH` 与名字恰为 `Automation` 的条），逐字相同的 `u32` |
///
/// ⇒ 上界 = **4 × 549,120 = 2,196,480**：比实测最大正常值留 **4.0 倍**余量
/// （2288 拍 ≈ 572 小节，比任何真实歌曲都长），而垃圾值比上界还大 **488.8 倍**
/// ⇒ 这道判据有预测力（⛔ 不是"刚好让 8 台通过"）。
///
/// ⚠ 只挡**已观测到**的量级（10.7 亿 tick）；别的垃圾形态仍可能漏 —— 见 `#150` 报告。
pub const LOGIC_REGION_LENGTH_MAX: u32 = 2_196_480;

/// 一条区域长度是否落在 [`LOGIC_REGION_LENGTH_MIN`]..=[`LOGIC_REGION_LENGTH_MAX`]。
///
/// ⚠ 空哨兵 [`LOGIC_REGION_EMPTY_LENGTH`] 与长度 0 由调用方**先行**排除
/// （它们各有一张自己的损失表条目），本函数只判"非空区域"的取值范围。
#[must_use]
pub const fn is_region_length_in_range(length_ticks: u32) -> bool {
    // 写成 `wrapping_sub <= 跨度`（等价于 `MIN <= x <= MAX`，更小的值会回绕成巨大值）：
    // `const fn` 里 `RangeInclusive::contains` 还不能用，这样也避开 `manual_range_contains`。
    length_ticks.wrapping_sub(LOGIC_REGION_LENGTH_MIN)
        <= LOGIC_REGION_LENGTH_MAX - LOGIC_REGION_LENGTH_MIN
}

/// 一条 `qeSM` 区域的**摆放读数**（起点 + 长度 + 该区域自己的音符跨度）。
///
/// 只读、纯字节：全部来自 `ProjectData` 的 `qeSM` 载荷与其后配对的 `qSvE`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionPlacement {
    /// 区域名（载荷 `+0x10` 的 `u16` 字节数 + `+0x12` 起的 UTF-8）。
    pub name: String,
    /// 区域起点（`rest + 0xe0`，960 PPQ tick）。
    pub start_tick: u32,
    /// 区域长度（`rest + 0x3c`，960 PPQ tick）；空区域是
    /// [`LOGIC_REGION_EMPTY_LENGTH`]。
    pub length_ticks: u32,
    /// 该区域的音符条数（其后 `qSvE` 里 [`decode_note_lines`] 的条数）。
    pub notes: usize,
    /// 该区域最后一个音符的**区域相对**结束 tick（`max(起始 + 时值)`）；无音符 = 0。
    pub note_span: u64,
    /// 是否是 Logic 的内部区域（`*Automation` / `RBA Sequence`）。
    pub internal: bool,
}

impl RegionPlacement {
    /// `起点 + 长度`（tick）。空区域的哨兵长度也会被算进去，调用方须先排除
    /// [`Self::is_empty`]。
    #[must_use]
    pub fn end_tick(&self) -> u64 {
        u64::from(self.start_tick) + u64::from(self.length_ticks)
    }

    /// 是否是**空区域**（长度 = 哨兵，起点 0、音符 0）。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.length_ticks == LOGIC_REGION_EMPTY_LENGTH
    }

    /// 绝对的最后音符结束 tick（`起点 + note_span`）。
    #[must_use]
    pub fn note_end_tick(&self) -> u64 {
        u64::from(self.start_tick) + self.note_span
    }
}

/// bplist00 魔数（8 字节，含版本）。
const BPLIST_MAGIC: &[u8; 8] = b"bplist00";

/// bplist00 文件末尾固定 32 字节的 trailer。
const BPLIST_TRAILER_LEN: usize = 32;

/// 递归解析 plist 对象的最大深度（防恶意/损坏输入的无限递归）。
const BPLIST_MAX_DEPTH: usize = 64;

/// 一台 demo 的**只读**读数。
///
/// 所有字段都来自字节本身：plist 七键来自 `MetaData.plist`，计数来自 `ProjectData`
/// 的记录流。`#[derive(PartialEq)]` 让判据可以逐台整结构对照。
#[derive(Debug, Clone, PartialEq)]
pub struct LogicInspection {
    /// bundle 目录名（去掉 `.logicx` 后缀），如 `Colors`。
    pub bundle: String,
    /// `ActiveVariant` 对应的三位目录名（如 `000` / `003` / `004`）。
    pub alternative: String,
    /// `MetaData.plist` 的 `SampleRate`（Hz）。
    pub sample_rate: u32,
    /// `MetaData.plist` 的 `BeatsPerMinute`。
    pub bpm: f64,
    /// `MetaData.plist` 的 `NumberOfTracks`（**声明值**）。
    pub declared_tracks: usize,
    /// `payload[0] == 1` 的 `karT` 记录条数（本模块的**实测**轨道数）。
    pub tracks: usize,
    /// 若**不**过滤 `payload[0] == 0` 的非轨道条，会得到的条数。
    ///
    /// 只为"过滤有牙"的判据保留：Colors 的 135 vs 138 就是这一对读数。
    pub tracks_including_zero_rows: usize,
    /// `payload[0] == 3` 的 `karT` 记录条数（应恰为 1，master）。
    pub master_tracks: usize,
    /// 所有 `qeSM` 记录条数（**不过滤**内部区域）。
    pub regions_all: usize,
    /// 被过滤掉的内部区域条数（`*Automation` 后缀 + `RBA Sequence`）。
    pub internal_regions: usize,
    /// 过滤内部区域后的区域条数。
    pub regions: usize,
    /// `qSvE` 音符事件数（[`crate::logic::decode_note_lines`] 的口径）。
    pub notes: usize,
    /// `AuRg`（落盘 `gRuA`）记录条数（`#152` 起同时逐条解析到 `LogicImport`）。
    pub audio_regions: usize,
    /// 成功读出摆放字段（起点/长度）的 `qeSM` 条数（应等于 [`Self::regions_all`]）。
    pub region_placements: usize,
    /// 所有区域起点的最大值（`rest+0xe0`，960 PPQ tick）。
    pub region_start_max: u32,
    /// **有音符**区域的 `起点 + 长度` 最大值（tick；哨兵长度不计入）。
    pub region_end_max: u64,
    /// **有音符**区域的 `起点 + 最后音符结束` 最大值（tick）。
    pub note_end_max: u64,
    /// `MetaData.plist` 的 `SongSignatureNumerator`。
    pub numerator: u32,
    /// `MetaData.plist` 的 `SongSignatureDenominator`。
    pub denominator: u32,
    /// `MetaData.plist` 的 `SongKey`（如 `C` / `Bb`）。
    pub song_key: String,
    /// `MetaData.plist` 的 `Version`。
    pub version: i64,
}

impl LogicInspection {
    /// 打印成**一行**（CLI 与判据共用同一格式，避免两处各写一份）。
    #[must_use]
    pub fn report_line(&self) -> String {
        let mut line = String::new();
        let _ = write!(
            line,
            "logic-demo: bundle={} alternative={} sample_rate={} bpm={} tracks={} \
             regions={} non_internal={} notes={} au_rg={} region_placements={} \
             region_start_max={} region_end_max={} note_end_max={}",
            self.bundle,
            self.alternative,
            self.sample_rate,
            self.bpm,
            self.tracks,
            self.regions_all,
            self.regions,
            self.notes,
            self.audio_regions,
            self.region_placements,
            self.region_start_max,
            self.region_end_max,
            self.note_end_max,
        );
        line
    }
}

/// 一组 demo 的合计（CLI 的合计行与判据的合计断言共用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LogicTotals {
    /// demo 台数。
    pub demos: usize,
    /// `payload[0] == 1` 的轨道条数合计。
    pub tracks: usize,
    /// 所有 `qeSM` 条数合计（过滤前）。
    pub regions_all: usize,
    /// 非内部区域条数合计。
    pub regions: usize,
    /// 音符条数合计。
    pub notes: usize,
    /// `AuRg` 条数合计。
    pub audio_regions: usize,
}

impl LogicTotals {
    /// 从逐台读数求和。
    #[must_use]
    pub fn of(inspections: &[LogicInspection]) -> Self {
        let mut totals = Self {
            demos: inspections.len(),
            ..Self::default()
        };
        for inspection in inspections {
            totals.tracks += inspection.tracks;
            totals.regions_all += inspection.regions_all;
            totals.regions += inspection.regions;
            totals.notes += inspection.notes;
            totals.audio_regions += inspection.audio_regions;
        }
        totals
    }

    /// 合计行（CLI 的最后一行）。
    #[must_use]
    pub fn report_line(&self) -> String {
        format!(
            "logic-total: demos={} tracks={} regions={} non_internal={} notes={} au_rg={}",
            self.demos, self.tracks, self.regions_all, self.regions, self.notes, self.audio_regions,
        )
    }
}

/// 读一台 bundle 的 `ActiveVariant` 与对应的 `Alternatives/<NNN>` 目录。
///
/// **`ActiveVariant` 缺失时的回退（`#194` 实测新增）**：负责人的 `track3.logicx`
/// （Logic Pro 12.2，`2026-10-18`，`HasProjectFolder = false`）的
/// `Resources/ProjectInformation.plist` **没有** `ActiveVariant` 键，只有
/// `VariantNames = {"0" = "track3"}`，而 `Alternatives/` 下**恰好一个**三位目录 `000`。
/// 这种情况下按**目录唯一性**取那一个目录 —— 仍然**不硬编码 `000`**：
/// 0 个或多于 1 个三位目录都**仍然报错**（错误串点名缺的键与实际目录数）。
fn active_alternative(bundle: &Path) -> Result<(String, PathBuf), String> {
    let project_information =
        read_plist(&bundle.join("Resources").join("ProjectInformation.plist"))?;
    let project_information = project_information.as_dict("ProjectInformation.plist")?;
    let alternative = match project_information.get("ActiveVariant") {
        Some(active_variant) => {
            let active_variant = active_variant
                .as_int()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| format!("{}: ActiveVariant 不是非负整数", bundle.display()))?;
            format!("{active_variant:03}")
        }
        None => single_alternative_dir(bundle)?,
    };
    let alternative_dir = bundle.join("Alternatives").join(&alternative);
    Ok((alternative, alternative_dir))
}

/// `ActiveVariant` 缺失时：`Alternatives/` 下**恰好一个**三位数字目录 ⇒ 用它。
///
/// 这是 [`active_alternative`] 的回退；它把"哪个 alternative 是活动的"从"plist 声明"
/// 降级成"目录唯一性"，因此**只在唯一时才成立**，并且错误串会点名缺的键与目录读数。
fn single_alternative_dir(bundle: &Path) -> Result<String, String> {
    let alternatives = bundle.join("Alternatives");
    let mut found: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(&alternatives) {
        for entry in entries.flatten() {
            let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if is_dir && name.len() == 3 && name.bytes().all(|byte| byte.is_ascii_digit()) {
                found.push(name.to_owned());
            }
        }
    }
    found.sort();
    match found.as_slice() {
        [only] => Ok(only.clone()),
        _ => Err(format!(
            "{}: 缺少必需键 ActiveVariant（`Resources/ProjectInformation.plist`），\
             且 `Alternatives/` 下不是恰好一个三位目录（实测 {} 个：{:?}）\
             ⇒ 无法确定活动 alternative（⛔ 不硬编码 000）",
            bundle.display(),
            found.len(),
            found
        )),
    }
}

/// 一台 bundle 的**元数据**（`ProjectInformation.plist` + `MetaData.plist` 的七键）。
///
/// [`inspect_bundle`] 与 [`import_bundle`] 共用这一份读取 —— ⛔ 不写第二份
/// "`ActiveVariant` ⇒ 目录 ⇒ 七键"的实现（两处一旦漂移，读数与导入就会对不上）。
struct BundleMeta {
    /// `ActiveVariant` 对应的三位目录名。
    alternative: String,
    /// `Alternatives/<NNN>` 目录。
    alternative_dir: PathBuf,
    /// 采样率（Hz）。
    sample_rate: u32,
    /// 速度（BPM）。
    bpm: f64,
    /// `NumberOfTracks` 声明值。
    declared_tracks: usize,
    /// 拍号分子。
    numerator: u32,
    /// 拍号分母。
    denominator: u32,
    /// 调性。
    song_key: String,
    /// 格式版本。
    version: i64,
}

/// 读一台 bundle 的 [`BundleMeta`]。
fn bundle_meta(bundle: &Path) -> Result<BundleMeta, String> {
    let (alternative, alternative_dir) = active_alternative(bundle)?;

    let meta_path = alternative_dir.join("MetaData.plist");
    let meta = read_plist(&meta_path)?;
    let meta = meta.as_dict("MetaData.plist")?;

    let sample_rate = required(meta, "SampleRate", &meta_path)?
        .as_int()
        .and_then(|value| u32::try_from(value).ok())
        .filter(|rate| *rate > 0)
        .ok_or_else(|| format!("{}: SampleRate 不是正整数", meta_path.display()))?;
    let bpm = required(meta, "BeatsPerMinute", &meta_path)?
        .as_f64()
        .filter(|value| value.is_finite() && *value > 0.0)
        .ok_or_else(|| format!("{}: BeatsPerMinute 不是正的有限数", meta_path.display()))?;
    let declared_tracks = required(meta, "NumberOfTracks", &meta_path)?
        .as_int()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| format!("{}: NumberOfTracks 不是非负整数", meta_path.display()))?;
    let numerator = required(meta, "SongSignatureNumerator", &meta_path)?
        .as_int()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| format!("{}: SongSignatureNumerator 不是正整数", meta_path.display()))?;
    let denominator = required(meta, "SongSignatureDenominator", &meta_path)?
        .as_int()
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            format!(
                "{}: SongSignatureDenominator 不是正整数",
                meta_path.display()
            )
        })?;
    let song_key = required(meta, "SongKey", &meta_path)?
        .as_str()
        .ok_or_else(|| format!("{}: SongKey 不是字符串", meta_path.display()))?
        .to_owned();
    let version = required(meta, "Version", &meta_path)?
        .as_int()
        .ok_or_else(|| format!("{}: Version 不是整数", meta_path.display()))?;
    Ok(BundleMeta {
        alternative,
        alternative_dir,
        sample_rate,
        bpm,
        declared_tracks,
        numerator,
        denominator,
        song_key,
        version,
    })
}

/// 一条正在累积音符的 `qeSM` 区域（`qeSM` 与下一个 `qeSM` 之间配对的 `qSvE`
/// 都算这条区域的音符）。
struct PendingRegion {
    /// 区域起点（`rest + 0xe0`）。
    start_tick: u32,
    /// 区域长度（`rest + 0x3c`）。
    length_ticks: u32,
    /// 已数到的音符条数。
    notes: usize,
    /// 最后一个音符的区域相对结束 tick。
    note_span: u64,
}

impl PendingRegion {
    /// 把这条区域的读数并进汇总。
    fn into_summary(self, summary: &mut RegionSummary) {
        summary.regions += 1;
        summary.start_max = summary.start_max.max(self.start_tick);
        if self.notes > 0 {
            if self.length_ticks != LOGIC_REGION_EMPTY_LENGTH {
                summary.end_max = summary
                    .end_max
                    .max(u64::from(self.start_tick) + u64::from(self.length_ticks));
            }
            summary.note_end_max = summary
                .note_end_max
                .max(u64::from(self.start_tick) + self.note_span);
        }
    }
}

/// 摆放读数的汇总（`inspect_bundle` 填进 [`LogicInspection`]）。
#[derive(Default)]
struct RegionSummary {
    /// 读出摆放字段的区域条数。
    regions: usize,
    /// 起点最大值。
    start_max: u32,
    /// 有音符区域的 `起点 + 长度` 最大值（哨兵长度不计入）。
    end_max: u64,
    /// 有音符区域的 `起点 + 最后音符结束` 最大值。
    note_end_max: u64,
}

/// 读一台 `.logicx` bundle（目录）并数出读数。
///
/// # Errors
///
/// 目录/文件缺失、bplist00 结构不合法、`ProjectData` 记录头与长度不自洽、
/// 区域名不是合法 UTF-8、或七个必需键缺失 —— 全部是**错误**，⛔ 不静默跳过。
pub fn inspect_bundle(bundle: &Path) -> Result<LogicInspection, String> {
    let bundle_name = bundle
        .file_name()
        .and_then(|name| name.to_str())
        .map_or_else(|| bundle.display().to_string(), str::to_owned);
    let bundle_name = bundle_name.strip_suffix(".logicx").unwrap_or(&bundle_name);

    let BundleMeta {
        alternative,
        alternative_dir,
        sample_rate,
        bpm,
        declared_tracks,
        numerator,
        denominator,
        song_key,
        version,
    } = bundle_meta(bundle)?;

    let project_data_path = alternative_dir.join("ProjectData");
    let bytes = fs::read(&project_data_path)
        .map_err(|error| format!("读 {} 失败: {error}", project_data_path.display()))?;
    let records = walk_records(&bytes)
        .map_err(|error| format!("{} 的记录流不合法: {error}", project_data_path.display()))?;

    let mut tracks = 0usize;
    let mut zero_rows = 0usize;
    let mut master_tracks = 0usize;
    let mut regions_all = 0usize;
    let mut internal_regions = 0usize;
    let mut notes = 0usize;
    let mut audio_regions = 0usize;
    let mut summary = RegionSummary::default();
    let mut pending: Option<PendingRegion> = None;

    for record in &records {
        if record.tag == LOGIC_KART_TAG && record.object == LOGIC_ARRANGE_OBJECT {
            match record.payload.first().copied() {
                Some(1) => tracks += 1,
                Some(0) => zero_rows += 1,
                Some(3) => master_tracks += 1,
                _ => {}
            }
        } else if record.tag == LOGIC_REGION_TAG {
            regions_all += 1;
            let name = region_name(record.payload, &project_data_path)?;
            if is_internal_region(name) {
                internal_regions += 1;
            }
            if let Some(done) = pending.take() {
                done.into_summary(&mut summary);
            }
            let (start_tick, length_ticks) = region_timing(record.payload, &project_data_path)?;
            pending = Some(PendingRegion {
                start_tick,
                length_ticks,
                notes: 0,
                note_span: 0,
            });
        } else if record.tag == LOGIC_SEQUENCE_TAG {
            let decoded = decode_note_lines(record.payload);
            notes += decoded.len();
            if let Some(region) = pending.as_mut() {
                region.notes += decoded.len();
                for (start, duration, _, _) in decoded {
                    region.note_span = region.note_span.max(start + duration);
                }
            }
        } else if record.tag == AUDIO_REGION_TAG {
            audio_regions += 1;
        }
    }
    if let Some(done) = pending.take() {
        done.into_summary(&mut summary);
    }

    Ok(LogicInspection {
        bundle: bundle_name.to_owned(),
        alternative,
        sample_rate,
        bpm,
        declared_tracks,
        tracks,
        tracks_including_zero_rows: tracks + zero_rows,
        master_tracks,
        regions_all,
        internal_regions,
        regions: regions_all - internal_regions,
        notes,
        audio_regions,
        region_placements: summary.regions,
        region_start_max: summary.start_max,
        region_end_max: summary.end_max,
        note_end_max: summary.note_end_max,
        numerator,
        denominator,
        song_key,
        version,
    })
}

/// 扫描一个目录下所有 `*.logicx` bundle。
///
/// **路径门控**：`dir` 不存在 ⇒ `Ok(None)`（调用方打印"无 demo、跳过"并返回，
/// ⛔ 不红）。目录存在但读不动、里面没有 `.logicx`、或任意一台解析失败 ⇒ `Err`
/// （⛔ 不静默跳过 —— 那会把"demo 在但解析坏了"伪装成"demo 不在"）。
///
/// # Errors
///
/// 目录存在但不可读 / 没有 `.logicx` / 任意 bundle 的解析失败。
pub fn inspect_demos(dir: &Path) -> Result<Option<Vec<LogicInspection>>, String> {
    if !dir.exists() {
        return Ok(None);
    }
    if !dir.is_dir() {
        return Err(format!("{} 不是目录", dir.display()));
    }
    let entries =
        fs::read_dir(dir).map_err(|error| format!("读目录 {} 失败: {error}", dir.display()))?;
    let mut bundles: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("读目录 {} 的条目失败: {error}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() && path.extension().and_then(|ext| ext.to_str()) == Some("logicx") {
            bundles.push(path);
        }
    }
    bundles.sort();
    if bundles.is_empty() {
        return Err(format!(
            "{} 存在，但里面没有一个 `.logicx` bundle（⛔ 不静默跳过）",
            dir.display()
        ));
    }
    let mut inspections = Vec::with_capacity(bundles.len());
    for bundle in bundles {
        inspections.push(
            inspect_bundle(&bundle).map_err(|error| format!("{}: {error}", bundle.display()))?,
        );
    }
    Ok(Some(inspections))
}

/// 读一台 `.logicx` bundle 的**区域摆放**：每条 `qeSM` 一条 [`RegionPlacement`]。
///
/// 这是 `#148` 的测量入口 —— **仍然只读**：不写工程、不造 `.yeban`、不改 demo。
/// 每条区域的起点/长度来自 [`region_timing`]，音符跨度来自它后面、下一个 `qeSM`
/// 之前的那条 `qSvE`（与 [`decode_note_lines`] 同一口径）。
///
/// # Errors
///
/// 与 [`inspect_bundle`] 同：文件缺失、bplist00/记录流不合法、区域名不是 UTF-8、
/// 或 `rest + 0xe0` / `rest + 0x3c` 越过载荷。
pub fn region_placements(bundle: &Path) -> Result<Vec<RegionPlacement>, String> {
    let (_, alternative_dir) = active_alternative(bundle)?;
    let project_data_path = alternative_dir.join("ProjectData");
    let bytes = fs::read(&project_data_path)
        .map_err(|error| format!("读 {} 失败: {error}", project_data_path.display()))?;
    let records = walk_records(&bytes)
        .map_err(|error| format!("{} 的记录流不合法: {error}", project_data_path.display()))?;

    let mut placements: Vec<RegionPlacement> = Vec::new();
    let mut current: Option<RegionPlacement> = None;
    for record in &records {
        if record.tag == LOGIC_REGION_TAG {
            if let Some(done) = current.take() {
                placements.push(done);
            }
            let name = region_name(record.payload, &project_data_path)?;
            let (start_tick, length_ticks) = region_timing(record.payload, &project_data_path)?;
            current = Some(RegionPlacement {
                name: name.to_owned(),
                start_tick,
                length_ticks,
                notes: 0,
                note_span: 0,
                internal: is_internal_region(name),
            });
        } else if record.tag == LOGIC_SEQUENCE_TAG
            && let Some(region) = current.as_mut()
        {
            let decoded = decode_note_lines(record.payload);
            region.notes += decoded.len();
            for (start, duration, _, _) in decoded {
                region.note_span = region.note_span.max(start + duration);
            }
        }
    }
    if let Some(done) = current.take() {
        placements.push(done);
    }
    Ok(placements)
}

/// 一台 `.logicx` bundle 的**摆放事件**（只读探针，`#151`）。
///
/// 返回记录流顺序里**所有**满足 MIDI 摆放签名的 80 字节事件；绑定规则由调用方施加
/// （见 [`ImportRegion::track_1based`] 与 [`import_bundle`]）。
///
/// # Errors
///
/// 与 [`inspect_bundle`] 同。
pub fn placement_events(bundle: &Path) -> Result<Vec<PlacementEvent>, String> {
    let (_, alternative_dir) = active_alternative(bundle)?;
    let project_data_path = alternative_dir.join("ProjectData");
    let bytes = fs::read(&project_data_path)
        .map_err(|error| format!("读 {} 失败: {error}", project_data_path.display()))?;
    let records = walk_records(&bytes)
        .map_err(|error| format!("{} 的记录流不合法: {error}", project_data_path.display()))?;
    Ok(placement_events_in(&records))
}

/// `payload[at..at+4]` 的 `u32` 小端（调用方保证 `at + 4 <= len`）。
fn le_u32(payload: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([
        payload[at],
        payload[at + 1],
        payload[at + 2],
        payload[at + 3],
    ])
}

/// 从记录流里扫出**全部 MIDI 摆放事件**（`#151`）。
///
/// 判据是**逐偏移的字节常量**（8/8 台实测），不是"按 80 字节网格切"：
/// 真实的编排 `qSvE` 里 MIDI（`0x20`）与音频（`0x24`）事件混排，音频事件后面还跟着
/// 自己的载荷，网格会立刻错位（`#151` 报告里有错位的逐字节读数）。因此这里只认
/// 四个常量都对的偏移：`+0x00 == 0x20`、`+0x17 == 0x89`、`+0x1c == 0x3fff_ffff`、
/// `+0x24 == 0x8800_0000`，并要求 `+0x04 ≥ 34560`（区域原点）。
fn placement_events_in(records: &[RecordView<'_>]) -> Vec<PlacementEvent> {
    let origin = u32::try_from(LOGIC_REGION_ORIGIN_TICKS).unwrap_or(u32::MAX);
    let marker = LOGIC_ARRANGE_MIDI_MARKER.to_le_bytes();
    let mut events = Vec::new();
    for record in records {
        if record.tag != LOGIC_SEQUENCE_TAG {
            continue;
        }
        let payload = record.payload;
        let last = payload
            .len()
            .saturating_sub(LOGIC_PLACEMENT_REGION_OFFSET + 4);
        for at in 0..=last {
            if payload[at..at + 4] != marker {
                continue;
            }
            if at + 0x28 > payload.len() {
                break;
            }
            if payload[at + 0x17] != PLACEMENT_EVENT_CONST_17
                || le_u32(payload, at + 0x1c) != PLACEMENT_EVENT_CONST_1C
                || le_u32(payload, at + 0x24) != PLACEMENT_EVENT_CONST_24_MIDI
            {
                continue;
            }
            let position_tick = le_u32(payload, at + LOGIC_PLACEMENT_POSITION_OFFSET);
            if position_tick < origin {
                continue;
            }
            events.push(PlacementEvent {
                link_id: le_u32(payload, at + 0x10),
                track_1based: u32::from(payload[at + LOGIC_PLACEMENT_TRACK_OFFSET]),
                position_tick,
                region_slot: le_u32(payload, at + LOGIC_PLACEMENT_REGION_OFFSET),
            });
        }
    }
    events
}

/// 槽位 → `(轨道号, 位置)` 的实测绑定表；同一槽位出现**互相矛盾**的读数 ⇒ 记进
/// 第二个返回值（歧义集合），调用方据此**不绑定**（⛔ 不挑一个猜）。
fn measured_bindings(records: &[RecordView<'_>]) -> (BTreeMap<u32, (u32, u32)>, BTreeSet<u32>) {
    let mut by_slot: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    let mut ambiguous: BTreeSet<u32> = BTreeSet::new();
    for event in placement_events_in(records) {
        let value = (event.track_1based, event.position_tick);
        match by_slot.get(&event.region_slot) {
            Some(existing) if *existing != value => {
                ambiguous.insert(event.region_slot);
            }
            Some(_) => {}
            None => {
                by_slot.insert(event.region_slot, value);
            }
        }
    }
    (by_slot, ambiguous)
}

/// `GenM`（可读名 `GenM`；落盘四个字节反过来是 `MneG`）—— 通道状态记录的 tag。
///
/// 导入器**不消费**它：本切片只计数并写进损失表（通道状态 / 混音器状态的含义未勘察）。
const CHANNEL_STATE_TAG: [u8; 4] = *b"MneG";

/// 一条可导入音符 —— [`decode_note_lines`] 的 `(起始, 时值, 音高, 力度)` 四元组。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportNote {
    /// 区域相对起始 tick（`decode_note_lines` 已扣掉 `LOGIC_NOTE_ORIGIN_TICKS`）。
    pub start_tick: u64,
    /// 时值（tick）。
    pub duration_ticks: u64,
    /// 音高（0..=127）。
    pub pitch: u8,
    /// 力度（0..=127）。
    pub velocity: u8,
}

/// 一条**摆放事件**（`qSvE` 载荷里定长 80 字节的 `20 00 00 00` 事件）的实测读数。
///
/// `#151`：这是"区域 → 轨道"唯一的实测来源。`qeSM` 载荷里**没有**轨道号字段
/// （`rest + 0x00..0x100` 的 `u8`/`u16`/`u32` 全扫过，8/8 台无一满足
/// 「1 ≤ 值 ≤ 轨数」且「每台 ≥ 2 个不同值」）；轨道号只在事件的 `+0x14`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlacementEvent {
    /// 事件 `+0x10` 的链接 id（`u32` 小端；demo 实测 `80 / 88 / 92 / …`，步长 4）。
    pub link_id: u32,
    /// 事件 `+0x14` 的**1 起**轨道号（单字节）。
    pub track_1based: u32,
    /// 事件 `+0x04` 的位置（`u32` 小端 = `34560 + 区域起点 tick`）。
    pub position_tick: u32,
    /// 事件 `+0x20` 指向的**区域槽**（`u32` 小端 = `qeSM` 记录头 `+0x08` 的
    /// 对象号 `>> 16`）。
    pub region_slot: u32,
}

/// 摆放事件 `+0x17` 的实测常量（8/8 台每一个 MIDI 摆放事件都是 `0x89`）。
const PLACEMENT_EVENT_CONST_17: u8 = 0x89;
/// 摆放事件 `+0x1c` 的实测常量（8/8 台每一个 MIDI 摆放事件都是 `0x3fff_ffff`）。
const PLACEMENT_EVENT_CONST_1C: u32 = 0x3fff_ffff;
/// 摆放事件 `+0x24` 的实测常量：**MIDI** 是 `0x8800_0000`，音频是 `0xbc00_0000`。
const PLACEMENT_EVENT_CONST_24_MIDI: u32 = 0x8800_0000;
/// 音频摆放事件 `+0x24` 的实测常量（`#152`；MIDI 是 `0x8800_0000`）。
const PLACEMENT_EVENT_CONST_24_AUDIO: u32 = 0xbc00_0000;

/// 一条 `AuRg` 音频区域记录（只读读数）。
///
/// `#152` 实测：**起点不在 `AuRg` 载荷里**（8 台 4,104 条逐条扫过 `+0x00..+0x4a` 的
/// `u8`/`u16`/`u32`，没有任何偏移等于摆放事件的 `+0x04 − 34560`）。起点只在**音频摆放
/// 事件** [`ImportAudioPlacement`] 的 `+0x04`；本记录只提供**长度**（帧）与**文件槽位**
/// （记录头 `+0x08` 的对象号，与 `lFuA` 的对象号相同）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportAudioRegion {
    /// 区域显示名（载荷 `+0x4a` 的 `u16` 字节数 + `+0x4c` 起的 UTF-8）。
    pub name: String,
    /// 记录头 `+0x08` 的对象号（`>> 16` = 音频摆放事件 `+0x2c` 的区域槽）。
    pub object: u32,
    /// 同一对象号下的 take 序号（记录头 `+0x0c` 的 `u32 >> 16`；`0` = 第一条）。
    pub take_index: u32,
    /// 长度（载荷 `+0x16` 的 `u32`，单位 = **音频帧**）。
    pub length_frames: u32,
}

/// 一条 `lFuA` 音频文件引用（`#152`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportAudioFile {
    /// 记录头 `+0x08` 的对象号（与引用它的 `AuRg` 记录同号）。
    pub object: u32,
    /// 文件名（载荷 `+0x08` 的 `u16` **字符数** + `+0x0a` 起的 UTF-16LE）。
    pub name: String,
    /// 载荷 `+0x00` 的 `u32`：Logic 记录的**文件字节数**（与磁盘文件对照的独立读数）。
    pub byte_size: u32,
}

/// 一条**音频摆放事件**（`qSvE` 里 `+0x00 == 0x24` 的 80 字节事件，`#152`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportAudioPlacement {
    /// 区域起点（`+0x04` − 34560，**960 PPQ tick**）。
    pub start_tick: u32,
    /// `+0x14` 的 1 起轨道号。
    pub track_1based: u32,
    /// `+0x2c` 的区域槽（= `AuRg` 对象号 `>> 16`）。
    pub region_slot: u32,
    /// `+0x10` 的链接 id（读数；与 MIDI 摆放同一族编号）。
    pub link_id: u32,
}

/// 一条**可导入**的 `qeSM` 区域（非内部、非空区域哨兵、非零长度）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRegion {
    /// 区域名。
    pub name: String,
    /// 区域起点（`rest + 0xe0`，960 PPQ tick）。
    pub start_tick: u32,
    /// 区域长度（`rest + 0x3c`，960 PPQ tick）。
    pub length_ticks: u32,
    /// 该区域的音符（`decode_note_lines` 原样；区域**相对** tick）。
    pub notes: Vec<ImportNote>,
    /// **实测**轨道号（1 起），来自与之槽位/位置都相等的摆放事件 `+0x14`。
    ///
    /// `None` = 这条区域**没有**可用的摆放事件（8 台共 213/864 条；形态与同一
    /// 位置的 take／Track Alternatives 一致）⇒ 调用方对它保留 provisional 轮转，
    /// ⛔ 不猜。绑定判据见 [`ImportRegion::is_bound_to_measured_track`]。
    pub track_1based: Option<u32>,
}

impl ImportRegion {
    /// 这条区域是否绑定到了**合法**的实测轨道（`1 ≤ 轨道号 ≤ 轨数`）。
    #[must_use]
    pub fn is_bound_to_measured_track(&self, track_count: usize) -> bool {
        self.track_1based.is_some_and(|track| {
            usize::try_from(track).is_ok_and(|track| (1..=track_count).contains(&track))
        })
    }
}

/// 一台 `.logicx` 的**导入读取**（仍然是纯字节、只读）。
///
/// 与 [`LogicInspection`] 的区别只有一条：本结构**带上音符与可导入区域的字节读数**，
/// 供导入器建工程；它⛔ 不写工程、⛔ 不写 demo、⛔ 不产出 `.yeban`。
#[derive(Debug, Clone, PartialEq)]
pub struct LogicImport {
    /// bundle 目录名（去掉 `.logicx` 后缀）。
    pub bundle: String,
    /// `ActiveVariant` 对应的三位目录名。
    pub alternative: String,
    /// `MetaData.plist` 的 `SampleRate`（Hz）。
    pub sample_rate: u32,
    /// `MetaData.plist` 的 `BeatsPerMinute`。
    pub bpm: f64,
    /// 拍号分子。
    pub numerator: u32,
    /// 拍号分母。
    pub denominator: u32,
    /// `payload[0] == 1` 的 `karT` 记录条数。
    pub tracks: usize,
    /// 所有 `qeSM` 条数（过滤前）。
    pub regions_all: usize,
    /// 内部区域条数（`*Automation` + `RBA Sequence`）。
    pub internal_regions: usize,
    /// 名字以 `*Automation` 结尾的区域条数。
    pub automation_regions: usize,
    /// 名字恰为 `RBA Sequence` 的区域条数。
    pub rba_sequences: usize,
    /// 非内部区域条数（含空区域哨兵）。
    pub regions: usize,
    /// 所有 `qSvE` 的音符条数（[`decode_note_lines`] 口径）。
    pub notes: usize,
    /// `AuRg`（落盘 `gRuA`）记录条数（`#152` 起**逐条解析**，不再只计数）。
    pub audio_regions: usize,
    /// 全部 `AuRg` 区域的逐条读数（记录流顺序）。
    pub audio_region_records: Vec<ImportAudioRegion>,
    /// 全部 `lFuA` 音频文件引用（记录流顺序；对象号与 `AuRg` 同族）。
    pub audio_files: Vec<ImportAudioFile>,
    /// 全部**音频摆放事件**（`qSvE` 里 `24 00 00 00` 的 80 字节事件，记录流顺序）。
    pub audio_placements: Vec<ImportAudioPlacement>,
    /// `GenM`（落盘 `MneG`）记录条数（只计数）。
    pub channel_states: usize,
    /// 可导入区域（非内部、长度 ≠ 空区域哨兵、长度 ≠ 0、长度落在
    /// [`LOGIC_REGION_LENGTH_MIN`]..=[`LOGIC_REGION_LENGTH_MAX`]；**记录流顺序**）。
    pub imported_regions: Vec<ImportRegion>,
    /// 因空区域哨兵（[`LOGIC_REGION_EMPTY_LENGTH`]）被跳过的条数。
    pub skipped_empty_regions: usize,
    /// 因长度为 0 被跳过的条数（模型要求摆放时值非零）。
    pub skipped_zero_length_regions: usize,
    /// 因长度**越界**（不在 [`LOGIC_REGION_LENGTH_MIN`]..=[`LOGIC_REGION_LENGTH_MAX`]）
    /// 被跳过的区域，**逐条**保留（记录流顺序）。
    ///
    /// `#150` 实测：8 台里有 17 条 —— 11 条长度 1,073,626,623（≈ 10.7 亿 tick；
    /// 960 PPQ 下 ≈ 1,118,361 拍）与 6 条长度 4 tick。它们⛔ 不进工程，也⛔ 不静默：
    /// 导入器的损失表逐条打印
    /// `level=boundary kind=length-out-of-range start=… length=…`。
    pub out_of_range_regions: Vec<ImportRegion>,
    /// 落在任何 `qeSM` 之外（或 `qeSM` 之前）的音符条数。
    pub unpaired_notes: usize,
    /// 可导入区域里**起点 ≥ 区域长度**的音符条数（读数；内容原样保留）。
    pub notes_outside_region: usize,
}

impl LogicImport {
    /// 一个音频槽位对应的 **take 0** 区域（同一文件对象的多条 take 取序号最小的一条）。
    #[must_use]
    pub fn audio_region_for_slot(&self, slot: u32) -> Option<&ImportAudioRegion> {
        self.audio_region_records
            .iter()
            .filter(|region| region.object >> 16 == slot)
            .min_by_key(|region| (region.take_index, region.name.len()))
    }

    /// 一个音频槽位对应的文件引用（`lFuA` 对象号 == `AuRg` 对象号）。
    #[must_use]
    pub fn audio_file_for_slot(&self, slot: u32) -> Option<&ImportAudioFile> {
        let object = slot << 16;
        self.audio_files.iter().find(|file| file.object == object)
    }

    /// 成功绑定到**合法**实测轨道（`1 ≤ 轨道号 ≤ 轨数`）的可导入区域条数（`#151`）。
    #[must_use]
    pub fn measured_track_regions(&self) -> usize {
        self.imported_regions
            .iter()
            .filter(|region| region.is_bound_to_measured_track(self.tracks))
            .count()
    }

    /// 仍按 provisional 轮转的可导入区域条数（`#151`；= 总数 − 实测绑定数）。
    #[must_use]
    pub fn provisional_track_regions(&self) -> usize {
        self.imported_regions.len() - self.measured_track_regions()
    }
}

/// 读一台 `.logicx` 的**可导入**字节（只读）。
///
/// 口径与 [`inspect_bundle`] / [`region_placements`] 完全一致：同一套记录行走、同一套
/// 区域名/摆放字段、同一个 [`decode_note_lines`]。新增的只有"把音符与区域一起交出来"。
///
/// **跳过**：内部区域（`*Automation` / `RBA Sequence`）、空区域哨兵
/// （`length == LOGIC_REGION_EMPTY_LENGTH`）、长度为 0 的区域、长度**越界**的区域
/// （不在 [`LOGIC_REGION_LENGTH_MIN`]..=[`LOGIC_REGION_LENGTH_MAX`]，`#150`）。
/// 内部区域 / 空哨兵 / 零长度各有自己的计数；越界的那类**逐条**留在
/// [`LogicImport::out_of_range_regions`]（⛔ 不静默丢弃）。其余**全部**保留，
/// 包括没有音符的区域 —— 它们是编排里真实存在的区域。
///
/// # Errors
///
/// 与 [`inspect_bundle`] 同：文件缺失、bplist00/记录流不合法、区域名不是 UTF-8、
/// 或 `rest + 0xe0` / `rest + 0x3c` 越过载荷。
pub fn import_bundle(bundle: &Path) -> Result<LogicImport, String> {
    let bundle_name = bundle
        .file_name()
        .and_then(|name| name.to_str())
        .map_or_else(|| bundle.display().to_string(), str::to_owned);
    let bundle_name = bundle_name.strip_suffix(".logicx").unwrap_or(&bundle_name);

    let meta = bundle_meta(bundle)?;
    let project_data_path = meta.alternative_dir.join("ProjectData");
    let bytes = fs::read(&project_data_path)
        .map_err(|error| format!("读 {} 失败: {error}", project_data_path.display()))?;
    let records = walk_records(&bytes)
        .map_err(|error| format!("{} 的记录流不合法: {error}", project_data_path.display()))?;

    // `#151`：轨道号只来自摆放事件的 `+0x14`（`qeSM` 载荷里没有这个字段）。
    // 绑定表与歧义槽位先算出来，再逐区域施加"槽位 + 位置 + 轨道号合法"三条判据。
    let (placement_by_slot, ambiguous_slots) = measured_bindings(&records);
    let tracks = records
        .iter()
        .filter(|record| {
            record.tag == LOGIC_KART_TAG
                && record.object == LOGIC_ARRANGE_OBJECT
                && record.payload.first().copied() == Some(1)
        })
        .count();

    let mut regions_all = 0usize;
    let mut internal_regions = 0usize;
    let mut automation_regions = 0usize;
    let mut rba_sequences = 0usize;
    let mut notes = 0usize;
    let mut audio_regions = 0usize;
    let mut audio_region_records: Vec<ImportAudioRegion> = Vec::new();
    let mut audio_files: Vec<ImportAudioFile> = Vec::new();
    let mut channel_states = 0usize;
    let mut all_regions: Vec<ImportRegion> = Vec::new();

    for record in &records {
        if record.tag == LOGIC_REGION_TAG {
            regions_all += 1;
            let name = region_name(record.payload, &project_data_path)?;
            if is_internal_region(name) {
                internal_regions += 1;
                if name.ends_with(AUTOMATION_NAME_SUFFIX) {
                    automation_regions += 1;
                } else {
                    rba_sequences += 1;
                }
            }
            let (start_tick, length_ticks) = region_timing(record.payload, &project_data_path)?;
            let region_slot = record.object >> 16;
            let origin = u32::try_from(LOGIC_REGION_ORIGIN_TICKS).unwrap_or(u32::MAX);
            let track_1based = placement_by_slot
                .get(&region_slot)
                .filter(|_| !ambiguous_slots.contains(&region_slot))
                .and_then(|&(track, position)| {
                    (position.checked_sub(origin) == Some(start_tick)
                        && usize::try_from(track).is_ok_and(|t| (1..=tracks).contains(&t)))
                    .then_some(track)
                });
            all_regions.push(ImportRegion {
                name: name.to_owned(),
                start_tick,
                length_ticks,
                notes: Vec::new(),
                track_1based,
            });
        } else if record.tag == LOGIC_SEQUENCE_TAG {
            let decoded = decode_note_lines(record.payload);
            notes += decoded.len();
            if let Some(region) = all_regions.last_mut() {
                region.notes.extend(decoded.into_iter().map(
                    |(start_tick, duration_ticks, pitch, velocity)| ImportNote {
                        start_tick,
                        duration_ticks,
                        pitch,
                        velocity,
                    },
                ));
            }
        } else if record.tag == AUDIO_REGION_TAG {
            audio_regions += 1;
            audio_region_records.push(audio_region_record(record, &project_data_path)?);
        } else if record.tag == AUDIO_FILE_TAG {
            audio_files.push(audio_file_record(record, &project_data_path)?);
        } else if record.tag == CHANNEL_STATE_TAG {
            channel_states += 1;
        }
    }

    let paired_all: usize = all_regions.iter().map(|region| region.notes.len()).sum();
    let mut imported_regions: Vec<ImportRegion> = Vec::new();
    let mut out_of_range_regions: Vec<ImportRegion> = Vec::new();
    let mut skipped_empty_regions = 0usize;
    let mut skipped_zero_length_regions = 0usize;
    let mut notes_outside_region = 0usize;
    for region in all_regions {
        if is_internal_region(&region.name) {
            continue;
        }
        if region.length_ticks == LOGIC_REGION_EMPTY_LENGTH {
            skipped_empty_regions += 1;
            continue;
        }
        if region.length_ticks == 0 {
            skipped_zero_length_regions += 1;
            continue;
        }
        if !is_region_length_in_range(region.length_ticks) {
            // ⛔ 不静默：样本原样留在读数里，导入器逐条打印。
            out_of_range_regions.push(region);
            continue;
        }
        let length = u64::from(region.length_ticks);
        notes_outside_region += region
            .notes
            .iter()
            .filter(|note| note.start_tick >= length)
            .count();
        imported_regions.push(region);
    }

    // 落在任何区域之外的音符：按"总数 − 各区域之和"算，⛔ 不猜它们属于谁。
    let unpaired_notes = notes.saturating_sub(paired_all);
    let audio_placements = audio_placement_events_in(&records);

    Ok(LogicImport {
        bundle: bundle_name.to_owned(),
        alternative: meta.alternative,
        sample_rate: meta.sample_rate,
        bpm: meta.bpm,
        numerator: meta.numerator,
        denominator: meta.denominator,
        tracks,
        regions_all,
        internal_regions,
        automation_regions,
        rba_sequences,
        regions: regions_all - internal_regions,
        notes,
        audio_regions,
        audio_region_records,
        audio_files,
        audio_placements,
        channel_states,
        imported_regions,
        skipped_empty_regions,
        skipped_zero_length_regions,
        out_of_range_regions,
        unpaired_notes,
        notes_outside_region,
    })
}

/// 从一条 `AuRg`（落盘 `gRuA`）记录读出名字/对象号/take/长度帧。
fn audio_region_record(record: &RecordView<'_>, path: &Path) -> Result<ImportAudioRegion, String> {
    let name_at = AUDIO_REGION_NAME_PAYLOAD_OFFSET;
    let length_at = name_at
        .checked_add(2)
        .ok_or_else(|| format!("{}: 音频区域名偏移溢出", path.display()))?;
    let length_bytes = record
        .payload
        .get(name_at..length_at)
        .ok_or_else(|| format!("{}: AuRg 载荷短于 +0x4c", path.display()))?;
    let name_len = usize::from(u16::from_le_bytes([length_bytes[0], length_bytes[1]]));
    let name_end = length_at
        .checked_add(name_len)
        .ok_or_else(|| format!("{}: AuRg 名字长度溢出", path.display()))?;
    let raw = record.payload.get(length_at..name_end).ok_or_else(|| {
        format!(
            "{}: AuRg 名字越过载荷（+0x4c + {} > {}）",
            path.display(),
            name_len,
            record.payload.len()
        )
    })?;
    let name = str::from_utf8(raw)
        .map_err(|error| format!("{}: AuRg 名字不是 UTF-8: {error}", path.display()))?;
    let frames_at = AUDIO_REGION_LENGTH_FRAMES_OFFSET;
    let raw_frames = record
        .payload
        .get(frames_at..frames_at + 4)
        .ok_or_else(|| format!("{}: AuRg 载荷短于 +0x1a", path.display()))?;
    let length_frames =
        u32::from_le_bytes([raw_frames[0], raw_frames[1], raw_frames[2], raw_frames[3]]);
    Ok(ImportAudioRegion {
        name: name.to_owned(),
        object: record.object,
        take_index: record.sub >> 16,
        length_frames,
    })
}

/// 从一条 `lFuA`（可读名 `AuFl`）记录读出文件名与 Logic 记录的文件字节数。
fn audio_file_record(record: &RecordView<'_>, path: &Path) -> Result<ImportAudioFile, String> {
    let payload = record.payload;
    if payload.len() < 0x0a {
        return Err(format!("{}: lFuA 载荷短于 +0x0a", path.display()));
    }
    let chars = usize::from(u16::from_le_bytes([payload[0x08], payload[0x09]]));
    let end = 0x0ausize
        .checked_add(
            chars
                .checked_mul(2)
                .ok_or_else(|| format!("{}: lFuA 文件名字节数溢出", path.display()))?,
        )
        .ok_or_else(|| format!("{}: lFuA 文件名偏移溢出", path.display()))?;
    if end > payload.len() {
        return Err(format!(
            "{}: lFuA 文件名越过载荷（+0x0a + {} > {}）",
            path.display(),
            chars * 2,
            payload.len()
        ));
    }
    let units: Vec<u16> = (0..chars)
        .map(|index| {
            let at = 0x0a + index * 2;
            u16::from_le_bytes([payload[at], payload[at + 1]])
        })
        .collect();
    let name = String::from_utf16(&units)
        .map_err(|error| format!("{}: lFuA 文件名不是合法 UTF-16: {error}", path.display()))?;
    let byte_size = le_u32(payload, 0);
    Ok(ImportAudioFile {
        object: record.object,
        name,
        byte_size,
    })
}

/// 扫出全部**音频摆放事件**（`qSvE` 载荷里 `+0x00 == 0x24` 的 80 字节事件）。
///
/// 判据与 MIDI 摆放同一族（`#151`/`#152`）：`+0x17 == 0x89`、`+0x1c == 0x3fff_ffff`、
/// `+0x24 == 0xbc00_0000`，并要求 `+0x04 ≥ 34560`。槽位在 `+0x2c`（**不是** `+0x20`）。
fn audio_placement_events_in(records: &[RecordView<'_>]) -> Vec<ImportAudioPlacement> {
    let origin = u32::try_from(LOGIC_REGION_ORIGIN_TICKS).unwrap_or(u32::MAX);
    let marker = AUDIO_PLACEMENT_MARKER.to_le_bytes();
    let mut events = Vec::new();
    for record in records {
        if record.tag != LOGIC_SEQUENCE_TAG {
            continue;
        }
        let payload = record.payload;
        let last = payload
            .len()
            .saturating_sub(AUDIO_PLACEMENT_LINK_OFFSET + 4);
        for at in 0..=last {
            if payload[at..at + 4] != marker {
                continue;
            }
            if at + 0x28 > payload.len() {
                break;
            }
            if payload[at + 0x17] != PLACEMENT_EVENT_CONST_17
                || le_u32(payload, at + 0x1c) != PLACEMENT_EVENT_CONST_1C
                || le_u32(payload, at + 0x24) != PLACEMENT_EVENT_CONST_24_AUDIO
            {
                continue;
            }
            let position_tick = le_u32(payload, at + LOGIC_PLACEMENT_POSITION_OFFSET);
            if position_tick < origin {
                continue;
            }
            events.push(ImportAudioPlacement {
                start_tick: position_tick - origin,
                track_1based: u32::from(payload[at + LOGIC_PLACEMENT_TRACK_OFFSET]),
                region_slot: le_u32(payload, at + AUDIO_PLACEMENT_LINK_OFFSET),
                link_id: le_u32(payload, at + LOGIC_PLACEMENT_LINK_ID_OFFSET),
            });
        }
    }
    events
}

/// `karT` 记录里"编排轨对象"的对象号（记录头 +0x08 的 `u32` 小端）。
const LOGIC_ARRANGE_OBJECT: u32 = 0x0004_0000;

/// 一台 demo 的区域名是否是 Logic 的**内部**区域（自动化泳道 / RBA 序列）。
fn is_internal_region(name: &str) -> bool {
    name.ends_with(AUTOMATION_NAME_SUFFIX) || name == RBA_SEQUENCE_NAME
}

/// 从 `qeSM` 载荷读出区域名（`+0x10` 是 `u16` 小端字节数，`+0x12` 起 UTF-8）。
fn region_name<'a>(payload: &'a [u8], path: &Path) -> Result<&'a str, String> {
    let name_at = LOGIC_REGION_NAME_PAYLOAD_OFFSET;
    let length_at = name_at
        .checked_add(2)
        .ok_or_else(|| format!("{}: 区域名偏移溢出", path.display()))?;
    let length_bytes = payload
        .get(name_at..length_at)
        .ok_or_else(|| format!("{}: 区域载荷短于 +0x12", path.display()))?;
    let name_len = usize::from(u16::from_le_bytes([length_bytes[0], length_bytes[1]]));
    let name_end = length_at
        .checked_add(name_len)
        .ok_or_else(|| format!("{}: 区域名长度溢出", path.display()))?;
    let raw = payload.get(length_at..name_end).ok_or_else(|| {
        format!(
            "{}: 区域名越过载荷（+0x12 + {} > {}）",
            path.display(),
            name_len,
            payload.len()
        )
    })?;
    str::from_utf8(raw)
        .map_err(|error| format!("{}: 区域名不是合法 UTF-8: {error}", path.display()))
}

/// 从 `qeSM` 载荷读出**摆放**两个字段：`(起点, 长度)`，单位都是 960 PPQ tick。
///
/// 偏移相对名字之后的 `rest` 起点（[`region_rest_start`]）：
/// 起点 `rest + 0xe0`（[`LOGIC_REGION_START_FROM_REST`]）、
/// 长度 `rest + 0x3c`（[`LOGIC_REGION_LENGTH_FROM_REST`]，与 `logic.rs` 里
/// 供体实测同一常量）。
fn region_timing(payload: &[u8], path: &Path) -> Result<(u32, u32), String> {
    let name = region_name(payload, path)?;
    let rest = region_rest_start(name.len());
    let start = read_u32(payload, rest, LOGIC_REGION_START_FROM_REST, path, "起点")?;
    let length = read_u32(payload, rest, LOGIC_REGION_LENGTH_FROM_REST, path, "长度")?;
    Ok((start, length))
}

/// 读 `payload[rest + offset..][..4]` 的 `u32` 小端；越界时报出**哪个字段**。
fn read_u32(
    payload: &[u8],
    rest: usize,
    offset: usize,
    path: &Path,
    what: &str,
) -> Result<u32, String> {
    let at = rest
        .checked_add(offset)
        .ok_or_else(|| format!("{}: 区域{what}偏移溢出", path.display()))?;
    let end = at
        .checked_add(4)
        .ok_or_else(|| format!("{}: 区域{what}偏移溢出", path.display()))?;
    let raw = payload.get(at..end).ok_or_else(|| {
        format!(
            "{}: 区域{what}越过载荷（rest {rest:#x} + {offset:#x} + 4 > {}）",
            path.display(),
            payload.len()
        )
    })?;
    Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

/// 一条按 36 字节记录头切出来的记录（只保留本探针用到的字段）。
#[derive(Debug)]
struct RecordView<'a> {
    /// 落盘的四个 tag 字节（可读名要反序）。
    tag: [u8; 4],
    /// 记录头 +0x08 的对象号（`u32` 小端）。
    object: u32,
    /// 记录头 +0x0c 的次级编号（`AuRg` 用它存 take 序号 `<< 16`）。
    sub: u32,
    /// 记录头之后的载荷。
    payload: &'a [u8],
}

/// 按载荷长度走完 `ProjectData` 的记录流，要求**恰好到 EOF**。
///
/// 行走只由记录头 +0x1c 的 `u32` 小端载荷长度驱动 —— 魔数在文件里会出现 2–3 次，
/// 用它/EOF 猜切分会把记录切错。
fn walk_records(bytes: &[u8]) -> Result<Vec<RecordView<'_>>, String> {
    if bytes.len() < LOGIC_ROOT_HEADER {
        return Err(format!("短于根头 {} 字节", LOGIC_ROOT_HEADER));
    }
    if bytes[..4] != LOGIC_ROOT_MAGIC {
        return Err(format!("根魔数不符: {:02X?}", &bytes[..4]));
    }
    let declared =
        u32::from_le_bytes([bytes[0x10], bytes[0x11], bytes[0x12], bytes[0x13]]) as usize;
    if declared.checked_add(LOGIC_ROOT_HEADER) != Some(bytes.len()) {
        return Err(format!(
            "声明载荷长 {declared} + 根头 {LOGIC_ROOT_HEADER} ≠ 文件长 {}",
            bytes.len()
        ));
    }

    let mut records = Vec::new();
    let mut at = LOGIC_FIRST_RECORD_OFFSET;
    while at < bytes.len() {
        let header_end = at
            .checked_add(LOGIC_RECORD_HEADER)
            .ok_or_else(|| format!("记录头偏移溢出: {at:#x}"))?;
        if header_end > bytes.len() {
            return Err(format!("记录头越过 EOF（偏移 {at:#x}）"));
        }
        let size = u32::from_le_bytes([
            bytes[at + LOGIC_RECORD_SIZE_OFFSET],
            bytes[at + LOGIC_RECORD_SIZE_OFFSET + 1],
            bytes[at + LOGIC_RECORD_SIZE_OFFSET + 2],
            bytes[at + LOGIC_RECORD_SIZE_OFFSET + 3],
        ]) as usize;
        let end = header_end
            .checked_add(size)
            .ok_or_else(|| format!("载荷长度溢出（记录 {at:#x}，长度 {size}）"))?;
        if end > bytes.len() {
            return Err(format!("载荷越过 EOF（记录 {at:#x}，长度 {size}）"));
        }
        let object = u32::from_le_bytes([
            bytes[at + LOGIC_RECORD_CLUSTER_OFFSET],
            bytes[at + LOGIC_RECORD_CLUSTER_OFFSET + 1],
            bytes[at + LOGIC_RECORD_CLUSTER_OFFSET + 2],
            bytes[at + LOGIC_RECORD_CLUSTER_OFFSET + 3],
        ]);
        let sub = u32::from_le_bytes([
            bytes[at + LOGIC_RECORD_CLUSTER_OFFSET + 4],
            bytes[at + LOGIC_RECORD_CLUSTER_OFFSET + 5],
            bytes[at + LOGIC_RECORD_CLUSTER_OFFSET + 6],
            bytes[at + LOGIC_RECORD_CLUSTER_OFFSET + 7],
        ]);
        records.push(RecordView {
            tag: [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]],
            object,
            sub,
            payload: &bytes[header_end..end],
        });
        at = end;
    }
    if at != bytes.len() {
        return Err("记录流没有恰好铺满文件".to_owned());
    }
    Ok(records)
}

// ---------------------------------------------------------------------------
// 最小 bplist00 读取子集（读侧；仓内原先只有编码器）
// ---------------------------------------------------------------------------

/// 本模块用到的 plist 取值子集。
#[derive(Debug, Clone, PartialEq)]
enum PlistValue {
    /// `true` / `false`（marker `0x08` / `0x09`）。
    Bool(bool),
    /// 有符号整数（marker `0x1*`；8 字节按二进制补码解释）。
    Int(i64),
    /// IEEE-754 浮点（marker `0x2*`；4 或 8 字节）。
    Real(f64),
    /// ASCII 或 UTF-16BE 字符串。
    String(String),
    /// 二进制 data（marker `0x4*`）。
    Data(Vec<u8>),
    /// 数组（marker `0xA*`）。
    Array(Vec<PlistValue>),
    /// 字典（marker `0xD*`；键必须是字符串）。
    Dict(BTreeMap<String, PlistValue>),
    /// `null`（marker `0x00`）。
    Null,
}

impl PlistValue {
    /// 取整数；`Int` 之外返回 `None`（不把 real 静默截成 int）。
    fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(value) => Some(*value),
            _ => None,
        }
    }

    /// 取浮点；`Int` 精确提升，`Real` 原样（只在这两处需要数值时用）。
    fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Int(value) => Some(*value as f64),
            Self::Real(value) => Some(*value),
            _ => None,
        }
    }

    /// 取字符串。
    fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    /// 要求顶层是字典（plist 的根对象）。
    fn as_dict(&self, what: &str) -> Result<&BTreeMap<String, PlistValue>, String> {
        match self {
            Self::Dict(dict) => Ok(dict),
            other => Err(format!("{what} 的根对象不是字典: {other:?}")),
        }
    }
}

/// 读一个 bplist00 文件的根对象。
fn read_plist(path: &Path) -> Result<PlistValue, String> {
    let bytes = fs::read(path).map_err(|error| format!("读 {} 失败: {error}", path.display()))?;
    let plist = BinaryPlist::parse(&bytes)
        .map_err(|error| format!("{} 不是合法的 bplist00: {error}", path.display()))?;
    plist
        .root()
        .map_err(|error| format!("{} 的根对象不合法: {error}", path.display()))
}

/// 从 bplist00 的 trailer/偏移表得到一个对象图。
struct BinaryPlist<'a> {
    /// 原始文件字节。
    bytes: &'a [u8],
    /// 每个对象的绝对偏移。
    offsets: Vec<usize>,
    /// 对象引用的字节宽度（trailer[7]）。
    object_ref_size: usize,
    /// 根对象下标（trailer 的 top object）。
    top: usize,
}

impl<'a> BinaryPlist<'a> {
    /// 解析 trailer 与偏移表；不解析对象内容。
    fn parse(bytes: &'a [u8]) -> Result<Self, String> {
        if bytes.len() < BPLIST_MAGIC.len() + BPLIST_TRAILER_LEN {
            return Err("短于 bplist00 头 + trailer".to_owned());
        }
        if &bytes[..BPLIST_MAGIC.len()] != BPLIST_MAGIC {
            return Err("缺少 bplist00 魔数".to_owned());
        }
        let trailer_at = bytes.len() - BPLIST_TRAILER_LEN;
        let trailer = &bytes[trailer_at..];
        let offset_int_size = usize::from(trailer[6]);
        let object_ref_size = usize::from(trailer[7]);
        if !(1..=8).contains(&offset_int_size) {
            return Err(format!("offset_int_size = {offset_int_size} 不在 1..=8"));
        }
        if !(1..=8).contains(&object_ref_size) {
            return Err(format!("object_ref_size = {object_ref_size} 不在 1..=8"));
        }
        let object_count = usize::try_from(be_uint(&trailer[8..16], 8))
            .map_err(|_| "对象数超出 usize".to_owned())?;
        let top = usize::try_from(be_uint(&trailer[16..24], 8))
            .map_err(|_| "根对象下标超出 usize".to_owned())?;
        let table = usize::try_from(be_uint(&trailer[24..32], 8))
            .map_err(|_| "偏移表位置超出 usize".to_owned())?;
        if object_count == 0 {
            return Err("对象数为 0".to_owned());
        }
        if top >= object_count {
            return Err(format!("根对象下标 {top} >= 对象数 {object_count}"));
        }
        let table_len = object_count
            .checked_mul(offset_int_size)
            .ok_or_else(|| "偏移表长度溢出".to_owned())?;
        let table_end = table
            .checked_add(table_len)
            .ok_or_else(|| "偏移表末尾溢出".to_owned())?;
        if table_end > trailer_at {
            return Err(format!(
                "偏移表末尾 {table_end} 越过 trailer 起点 {trailer_at}"
            ));
        }
        let mut offsets = Vec::with_capacity(object_count);
        for index in 0..object_count {
            let at = table + index * offset_int_size;
            let offset =
                usize::try_from(be_uint(&bytes[at..at + offset_int_size], offset_int_size))
                    .map_err(|_| "对象偏移超出 usize".to_owned())?;
            if offset < BPLIST_MAGIC.len() || offset >= trailer_at {
                return Err(format!("对象 {index} 的偏移 {offset} 越界"));
            }
            offsets.push(offset);
        }
        Ok(Self {
            bytes,
            offsets,
            object_ref_size,
            top,
        })
    }

    /// 解析根对象。
    fn root(&self) -> Result<PlistValue, String> {
        self.object_at(self.top, 0)
    }

    /// 解析一个对象；`depth` 是递归深度护栏。
    ///
    /// `chunks_exact(2)` 是 UTF-16 两字节单元的既有写法；clippy 1.99 的新
    /// `chunks_exact_to_as_chunks` 纯风格提示在这里显式放行，⛔ 不为一个提示
    /// 改读侧的字节解析。
    #[allow(clippy::chunks_exact_to_as_chunks)]
    fn object_at(&self, index: usize, depth: usize) -> Result<PlistValue, String> {
        if depth > BPLIST_MAX_DEPTH {
            return Err(format!("对象嵌套超过 {BPLIST_MAX_DEPTH} 层"));
        }
        let at = *self
            .offsets
            .get(index)
            .ok_or_else(|| format!("对象下标 {index} 越界"))?;
        let marker = *self
            .bytes
            .get(at)
            .ok_or_else(|| format!("对象 {index} 在偏移 {at} 处越过 EOF"))?;
        let body = at + 1;
        match marker >> 4 {
            0x0 => match marker {
                0x00 => Ok(PlistValue::Null),
                0x08 => Ok(PlistValue::Bool(false)),
                0x09 => Ok(PlistValue::Bool(true)),
                other => Err(format!("未知的简单标记 {other:#04x}")),
            },
            0x1 => {
                let size = 1usize << (marker & 0x0f);
                if size > 8 {
                    return Err(format!("整数宽度 {size} 字节不受支持"));
                }
                let raw = be_uint(self.slice(body, size)?, size);
                Ok(PlistValue::Int(raw as i64))
            }
            0x2 => {
                let size = 1usize << (marker & 0x0f);
                let raw = self.slice(body, size)?;
                match size {
                    4 => Ok(PlistValue::Real(f64::from(f32::from_bits(
                        be_uint(raw, 4) as u32
                    )))),
                    8 => Ok(PlistValue::Real(f64::from_bits(be_uint(raw, 8)))),
                    other => Err(format!("实数宽度 {other} 字节不受支持")),
                }
            }
            // 日期（`0x3*`）：本探针不读日期，但把 8 字节原样当 real 返回，
            // 这样遇到带日期的 plist 也不会假装"格式非法"。
            0x3 => {
                let size = 1usize << (marker & 0x0f);
                if size != 8 {
                    return Err(format!("日期宽度 {size} 字节不受支持"));
                }
                Ok(PlistValue::Real(f64::from_bits(be_uint(
                    self.slice(body, 8)?,
                    8,
                ))))
            }
            0x4 => {
                let (count, at) = self.length(marker, body)?;
                Ok(PlistValue::Data(self.slice(at, count)?.to_vec()))
            }
            0x5 => {
                let (count, at) = self.length(marker, body)?;
                let raw = self.slice(at, count)?;
                let text = str::from_utf8(raw)
                    .map_err(|error| format!("ASCII 字符串不是合法 UTF-8: {error}"))?;
                Ok(PlistValue::String(text.to_owned()))
            }
            0x6 => {
                let (count, at) = self.length(marker, body)?;
                let bytes = self.slice(at, count.saturating_mul(2))?;
                let units: Vec<u16> = bytes
                    .chunks_exact(2)
                    .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                    .collect();
                let text = String::from_utf16(&units)
                    .map_err(|error| format!("UTF-16 字符串非法: {error}"))?;
                Ok(PlistValue::String(text))
            }
            0xA => {
                let (count, at) = self.length(marker, body)?;
                let refs = self.slice(at, count.saturating_mul(self.object_ref_size))?;
                let mut items = Vec::with_capacity(count);
                for index in 0..count {
                    let child = be_uint(&refs[index * self.object_ref_size..], self.object_ref_size)
                        as usize;
                    items.push(self.object_at(child, depth + 1)?);
                }
                Ok(PlistValue::Array(items))
            }
            0xD => {
                let (count, at) = self.length(marker, body)?;
                let ref_bytes = count
                    .checked_mul(self.object_ref_size)
                    .and_then(|value| value.checked_mul(2))
                    .ok_or_else(|| "字典引用表长度溢出".to_owned())?;
                let refs = self.slice(at, ref_bytes)?;
                let mut dict = BTreeMap::new();
                for index in 0..count {
                    let key_ref =
                        be_uint(&refs[index * self.object_ref_size..], self.object_ref_size)
                            as usize;
                    let value_ref = be_uint(
                        &refs[(count + index) * self.object_ref_size..],
                        self.object_ref_size,
                    ) as usize;
                    let key = match self.object_at(key_ref, depth + 1)? {
                        PlistValue::String(key) => key,
                        other => {
                            return Err(format!("字典键不是字符串: {other:?}"));
                        }
                    };
                    let value = self.object_at(value_ref, depth + 1)?;
                    if dict.insert(key.clone(), value).is_some() {
                        return Err(format!("字典键重复: {key}"));
                    }
                }
                Ok(PlistValue::Dict(dict))
            }
            other => Err(format!("未知的 plist 标记族 {other:#x}")),
        }
    }

    /// 读一个对象的"长度"：低半字节 ≠ `0xF` 时就是它；否则后面跟一个整型对象。
    fn length(&self, marker: u8, at: usize) -> Result<(usize, usize), String> {
        let info = usize::from(marker & 0x0f);
        if info != 0x0f {
            return Ok((info, at));
        }
        let int_marker = *self
            .bytes
            .get(at)
            .ok_or_else(|| "扩展长度越过 EOF".to_owned())?;
        if int_marker >> 4 != 0x1 {
            return Err(format!("扩展长度的前缀不是整数: {int_marker:#04x}"));
        }
        let size = 1usize << (int_marker & 0x0f);
        if size > 8 {
            return Err(format!("扩展长度整数宽度 {size} 字节不受支持"));
        }
        let raw = be_uint(self.slice(at + 1, size)?, size);
        let count = usize::try_from(raw).map_err(|_| "长度超出 usize".to_owned())?;
        Ok((count, at + 1 + size))
    }

    /// 按绝对偏移取一段字节，越界即错误。
    fn slice(&self, at: usize, len: usize) -> Result<&'a [u8], String> {
        let end = at
            .checked_add(len)
            .ok_or_else(|| "对象范围溢出".to_owned())?;
        self.bytes
            .get(at..end)
            .ok_or_else(|| format!("对象范围 {at}..{end} 越过 EOF"))
    }
}

/// 大端无符号整数（bplist00 的偏移/计数都是大端）。
fn be_uint(bytes: &[u8], size: usize) -> u64 {
    let mut value = 0u64;
    for byte in bytes.iter().take(size) {
        value = (value << 8) | u64::from(*byte);
    }
    value
}

/// 取字典里的必需键；缺失时报出**是哪个文件**的哪个键。
fn required<'a>(
    dict: &'a BTreeMap<String, PlistValue>,
    key: &str,
    path: &Path,
) -> Result<&'a PlistValue, String> {
    dict.get(key)
        .ok_or_else(|| format!("{}: 缺少必需键 {key}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `#194` 实测：`ActiveVariant` 缺失时，**恰好一个**三位目录才算数。
    ///
    /// 这条判据只测回退本身（不造 bplist）：0 个、1 个、2 个目录三种形态，
    /// 正向必须取到 `000`，反向必须**报错**（⛔ 不静默挑一个）。
    #[test]
    fn single_alternative_dir_requires_exactly_one_candidate() {
        let dir = tempfile::tempdir().expect("tempdir");
        // 0 个：必须是错误。
        let empty = dir.path().join("empty");
        std::fs::create_dir_all(empty.join("Alternatives")).expect("mkdir");
        let error = single_alternative_dir(&empty).expect_err("0 个目录必须报错");
        assert!(
            error.contains("ActiveVariant"),
            "错误串必须点名缺的键: {error}"
        );

        // 1 个：取它（不硬编码 000 —— 这里故意用 `007`）。
        let one = dir.path().join("one");
        std::fs::create_dir_all(one.join("Alternatives/007")).expect("mkdir");
        assert_eq!(
            single_alternative_dir(&one).expect("唯一目录必须可用"),
            "007"
        );

        // 2 个：仍然必须报错（唯一性不成立）。
        let two = dir.path().join("two");
        std::fs::create_dir_all(two.join("Alternatives/000")).expect("mkdir");
        std::fs::create_dir_all(two.join("Alternatives/001")).expect("mkdir");
        let error = single_alternative_dir(&two).expect_err("两个目录必须报错");
        assert!(
            error.contains("实测 2 个"),
            "错误串必须写出目录读数: {error}"
        );

        // 非三位 / 非数字目录不参与（例如 `Alternatives/backup`）。
        let mixed = dir.path().join("mixed");
        std::fs::create_dir_all(mixed.join("Alternatives/000")).expect("mkdir");
        std::fs::create_dir_all(mixed.join("Alternatives/backup")).expect("mkdir");
        assert_eq!(
            single_alternative_dir(&mixed).expect("非三位目录必须被忽略"),
            "000"
        );
    }

    /// 造一条最小 `qeSM` 载荷：名字字段 + `rest` 里的起点/长度。
    fn synthetic_region(name: &str, start: u32, length: u32) -> Vec<u8> {
        let rest = region_rest_start(name.len());
        let mut payload = vec![0u8; rest + LOGIC_REGION_START_FROM_REST + 8];
        let at = LOGIC_REGION_NAME_PAYLOAD_OFFSET;
        payload[at..at + 2].copy_from_slice(&u16::try_from(name.len()).unwrap().to_le_bytes());
        payload[at + 2..at + 2 + name.len()].copy_from_slice(name.as_bytes());
        let start_at = rest + LOGIC_REGION_START_FROM_REST;
        payload[start_at..start_at + 4].copy_from_slice(&start.to_le_bytes());
        let length_at = rest + LOGIC_REGION_LENGTH_FROM_REST;
        payload[length_at..length_at + 4].copy_from_slice(&length.to_le_bytes());
        payload
    }

    /// `#148` 的**偏移钉住**：起点在 `rest+0xe0`、长度在 `rest+0x3c`，
    /// 对奇数名 / 偶数名都成立（`rest` 起点按偶数载荷偏移补齐）。
    #[test]
    fn region_timing_reads_the_measured_offsets_for_odd_and_even_names() {
        let path = Path::new("synthetic/ProjectData");
        for (name, start, length) in [
            ("up:", 0, 30_720),
            ("down:", 30_720, 15_360),
            ("ABC", 103_680, 69_120),
            ("Untitled", 380_160, 7_680_000),
        ] {
            let payload = synthetic_region(name, start, length);
            assert_eq!(
                region_timing(&payload, path).unwrap(),
                (start, length),
                "名字 {name:?}（{} 字节）的起点/长度",
                name.len()
            );
        }
    }

    /// **有牙**：名字长度变 1 字节 ⇒ `rest` 起点平移 ⇒ 固定载荷偏移会读错。
    ///
    /// `"ABC"`（3 字节）rest 起点 `0x16`，`"AB"`（2 字节）是 `0x14`；把同一份
    /// 起点值写在各自的 `rest+0xe0` 上，若实现误用固定偏移，其中一条必错。
    #[test]
    fn region_timing_moves_with_the_name_length_and_rejects_short_payloads() {
        let path = Path::new("synthetic/ProjectData");
        let odd = synthetic_region("ABC", 960, 1_920);
        let even = synthetic_region("AB", 1_920, 960);
        assert_ne!(region_rest_start(3), region_rest_start(2));
        assert_eq!(region_timing(&odd, path).unwrap(), (960, 1_920));
        assert_eq!(region_timing(&even, path).unwrap(), (1_920, 960));
        assert!(
            region_timing(&[0u8; 0x20], path).is_err(),
            "短载荷必须报错，⛔ 不得读越界或返回 0"
        );
    }

    /// 空区域的长度哨兵与 `end_tick` 的边界：哨兵必须能被 `is_empty()` 识别，
    /// 调用方据此**不把** 7,680,000 当真实长度。
    #[test]
    fn empty_regions_use_the_measured_sentinel() {
        let empty = RegionPlacement {
            name: "*Automation".to_owned(),
            start_tick: 0,
            length_ticks: LOGIC_REGION_EMPTY_LENGTH,
            notes: 0,
            note_span: 0,
            internal: true,
        };
        assert!(empty.is_empty());
        assert_eq!(empty.end_tick(), 7_680_000);
        let normal = RegionPlacement {
            name: "Clip".to_owned(),
            start_tick: 30_720,
            length_ticks: 15_360,
            notes: 1,
            note_span: 15_360,
            internal: false,
        };
        assert!(!normal.is_empty());
        assert_eq!(normal.end_tick(), 46_080);
        assert_eq!(normal.note_end_tick(), 46_080);
    }
}
