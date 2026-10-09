//! MusicXML (`.musicxml`) 的**只读**导入 MVP：手写极简 pull parser，**零新依赖**。
//!
//! ## 这个模块的规范出处（⚠️ 先说清楚：**规范未定义 MusicXML**）
//!
//! 本仓库的四份规范里 `MusicXML` 的命中数是 **0**（测法：`grep -c -i musicxml`
//! 逐份跑 `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` /
//! `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` /
//! `docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md` /
//! `docs/YEBAN_INDUSTRY_BENCHMARK_AND_OPEN_SOURCE_STUDY.md` + `docs/DEVELOPMENT_LEDGER.md`
//! / `docs/DEV_WORKFLOW.md` / `docs/CI_CD.md`，七份全是 0）。
//! 规范里唯一与 XML 有关的句子是 `[ARCH-FMT-002]` 的 `.als` **导出**
//! （`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:479`，实验性、基于 `flate2`）。
//!
//! ⇒ 因此本模块**不是**规范的实现，而是一条**工程选择**：
//! `docs/ledger/integration-rulings-notes.md:14` 的 R1 —— 「**不引入**新依赖。
//! 先做纯 `.musicxml` 只读 MVP，手写极简 pull parser」。
//! 依据同文件 `:33-38`：需求是一个白名单子集，手写 pull parser 可覆盖它。
//! ⛔ 本模块**不**声称 MusicXML 导入已接入引擎/界面（`docs/ledger/integration-rulings-notes.md:85`）。
//!
//! ## 做什么（白名单）
//!
//! 输入 `.musicxml` 的 UTF-8 字节，输出 [`MusicXmlScore`](crate::musicxml::MusicXmlScore)（**960 PPQ** 的只读结果）：
//!
//! | MusicXML | 行为 |
//! | :--- | :--- |
//! | `score-partwise` | 唯一的根元素；`score-timewise` 被拒绝 |
//! | `part-list/score-part@id` + `part-name` | 部件身份与名称 |
//! | `part@id` | 一条独立的**时间线**（cursor 从 0 开始） |
//! | `attributes/divisions` | 每四分音符的单位数；**缺省 = 1**（见下） |
//! | `attributes/time/beats` + `beat-type` | 拍号记录（`numerator` / `denominator_pow2`） |
//! | `direction/…/sound@tempo` | 速度记录（`mpqn = round(60_000_000 / bpm)`） |
//! | `note/pitch/step+alter+octave` | MIDI 音高（`(octave+1)*12 + step + alter`；C4 = 60） |
//! | `note/duration` | 时值（`divisions` 单位 ⇒ 960 PPQ tick） |
//! | `note/chord` | 与**上一个**音符同起点，不推进 cursor |
//! | `note/rest` | 不发声，但推进 cursor |
//! | `note/tie`（优先）与 `note/notations/tied` | `stop` 合并进同 `(voice, staff, key)` 且**恰好相接**的前一个音符 |
//! | `note/voice` + `note/staff` | 逐音符登记；也是延音配对的键 |
//! | `backup/duration` | cursor 回退（回退过头 ⇒ 明确 `Err`） |
//! | 其它标签 | **容错跳过并登记**在 [`MusicXmlScore::ignored_elements`](crate::musicxml::MusicXmlScore::ignored_elements) |
//!
//! `divisions` 的缺省值 **1** 有出处：上游 W3C 社区组测试套件
//! （`w3c-cg/musicxmlTestSuite`，MIT）的用例文件自己写着
//! 「No `<divisions>` element. The generally agreed default value is 1, and MusicXML
//! version 4.1 will mention this explicitly.」
//! —— 该文件的路径是**上游仓库**的 `xmlFiles/03e-Rhythm-No-Divisions.musicxml`
//! （本仓库的副本 = `tests/fixtures/w3c_03e_no_divisions.musicxml`，来源 / 许可 / SHA-256
//! 见 `tests/fixtures/README.md` 第 5 节）；**上游仓库**的 `schema/musicxml.xsd:5061`
//! 把该元素写成 `minOccurs="0"`（⛔ 本仓库没有 `schema/musicxml.xsd` 这个文件）。
//!
//! ## 未实现清单（⛔ 不许把这些读成"已支持"）
//!
//! 1. ⛔ **`.mxl`（ZIP/deflate 容器）**：**本模块**只吃**纯文本** `.musicxml`。
//!    `.mxl` 的字节不是 UTF-8 ⇒ 明确 `Err`（本仓库自造夹具的**字面**读数是
//!    `InvalidUtf8 { offset: 17 }`；判据见 `tests/musicxml_contract.rs`）。
//!    inflate 属另立票（`docs/ledger/integration-rulings-notes.md:37-38`）
//!    ⇒ ⭐ **那一票已交付**：容器层在 [`crate::mxl`]（`parse_mxl`，路线 B）。
//!    ⚠️ 本函数的**行为与判据一个字节都没改**：它仍然只吃纯文本。
//!
//!    **代价（本机实测，2026-10-08；量法与逐件读数见 `tests/fixtures/README.md` 第 6 节）**：
//!    6 个真 `.mxl`（本机 `/tmp/musicxml/**`，**未提交**）每个都是 **2** 个条目
//!    （`META-INF/container.xml` + `score.xml`），压缩法 **6/6 = 8**（deflate）；
//!    `score.xml` 膨胀后与同名 `.musicxml` **逐字节相同**（6/6）
//!    ⇒ 代价**全在"容器 + inflate"**，不在解析：膨胀结果可直接喂 [`parse_musicxml`](crate::musicxml::parse_musicxml)。
//!    6/6 的 `score.xml` DEFLATE 流是**单个 dynamic-Huffman 块**（BTYPE=2），最长匹配 258、
//!    最远匹配距离 29393..32502；上界 **32502 > 16384**（16 KiB）⇒ 16 KiB 窗口的捷径不够，
//!    必须支持 RFC 1951 的**完整 32 KiB 窗口**（RFC 1951 是**外部**规范，不在本仓库）。
//!
//!    **路线（⛔ 本票只登记代价，不引入任何依赖）**：
//!
//!    | # | 路线 | 实测代价 | 本票处置 |
//!    | :-: | :--- | :--- | :--- |
//!    | A | 加 `flate2`（根清单已登记第 79 行；`rust_backend` = 纯 Rust，无 C） | 依赖树**新增 5 个 crate**：`adler2` / `crc32fast` / `flate2` / `miniz_oxide` / `simd-adler32`（测法：`cargo tree -p yeban-render --prefix none \| sort -u` 的行数 104 → 109） | ⛔ 未采用（本票禁加依赖；**后一票实测**：这条依赖边还会强制改 `Cargo.lock` 并让 `scripts/gates/license_inventory.py --check` 变红 ⇒ 见 [`crate::mxl`] 的模块文档） |
//!    | B | 手写 raw-DEFLATE inflate（零依赖） | 需 stored + fixed + dynamic 三种块与 32 KiB 窗口；本机 Python 原型在 6/6 上与 `zlib` 输出**逐字节相同** | ✅ **后一票采用** ⇒ [`crate::mxl`]（Rust 实现 `src/mxl/inflate.rs`） |
//!    | C | 复用 `yeban_model::container` 的 ZIP 读取器 | **不通**：`read_zip` 是 `pub(crate)`，且**第一个**条目就按压缩法拒绝（`crates/yeban-model/src/container/zip.rs:486-490`） | 判据钉住今天的读数 |
//!    | D | 走根清单已登记的 `zip` crate | `zip`（`default-features = false`）自带 **8 个 crate**（探针 crate 的 `cargo tree` 行数 9，含根）；其 `deflate` 特性**额外**要 `zopfli`（本机 registry 缓存**没有** ⇒ 离线解析失败）且选 `flate2/zlib-rs`（与根清单登记的后端不同） | ⛔ 未采用 |
//!
//! 2. ⛔ **导出**：本模块只读。`.musicxml` 写出不存在。
//! 3. ⛔ **接线**：引擎 / MCP / 界面**都不**调用本模块（集成者裁决）。
//! 4. ⛔ **`forward` / `grace` / `unpitched` / `transpose`**：不实现，逐次登记在
//!    [`MusicXmlScore::unsupported_elements`](crate::musicxml::MusicXmlScore::unsupported_elements)。语义后果**明说**：
//!    - `forward` 被忽略 ⇒ 用了它的文件，其后 tick **会偏移**（不是"无影响"）；
//!    - `grace` 音符不发声、**不**推进 cursor；
//!    - `unpitched` 音符不发声，但按 `duration` 推进 cursor；
//!    - `transpose` 被忽略 ⇒ 移调乐器按**记谱音高**读入，与听到的音高相差 `chromatic`。
//! 5. ⛔ **完整 MusicXML 4.0 语义**：`key` / `clef` / `lyric` / `beam` / `slur` /
//!    `articulations` / `dynamics` / 反复与跳转（`repeat` / `ending` / `segno` / `coda`）/
//!    装饰音语义 / 力度（力度恒为 [`DEFAULT_VELOCITY`](crate::musicxml::DEFAULT_VELOCITY)）/ 多 `divisions` 变更
//!    （只认**第一次**出现的 `divisions`）都不实现。
//! 6. ⛔ **`cue` 音符不区分**（按普通音符处理）。
//! 7. ⛔ **延音配对只认"恰好相接"的同一个 `part` 内、同 `(voice, staff, key)` 的音符**；
//!    跨声部、跨越缺口、`let-ring` 都不配对（不配对时如实落成两个音符，⛔ 不猜）。
//!
//! ## 不 panic 的承诺
//!
//! 任意字节输入只产生 `Ok` 或 [`MusicXmlError`](crate::musicxml::MusicXmlError)：**没有 `unwrap` / `expect` /
//! 索引切片越界 / 算术溢出**。tokenizer 的游标每轮至少前进 1 字节 ⇒ 不可能死循环；
//! 嵌套深度 > [`MAX_DEPTH`](crate::musicxml::MAX_DEPTH) 立即 `Err` ⇒ 内存不随恶意嵌套线性放大。
//!
//! ## 分配（MusicXML **不在**音频线程 ⇒ 零分配不适用，但必须有界）
//!
//! - 元素名与属性值是**借用输入文本的切片**（零分配）。
//! - 文本只为 `is_text_element` 列的 10 个叶子元素累积（其余元素的文本直接丢弃）。
//! - `ignored_elements` / `unsupported_elements` 只按**不同的元素名**增长 ⇒
//!   内存 ≤ O(输入长度)（每个新名字至少占输入里的 1 字节）。
//! - 音符、部件、tempo 记录都随输入线性增长，且每个音符只存定长字段。
//! - 错误路径才分配（`String` 里的原始文本）。
//!
//! ## 确定性 [ARCH-DET-001]
//!
//! 同一份字节 ⇒ **逐位相同**的结果：全部映射用 `BTreeMap`（⛔ 无 `HashMap`），
//! 音符按 `(start_tick, key, voice, staff, duration_ticks)` 稳定排序，
//! tempo 记录按 `(tick, mpqn, numerator, denominator_pow2)` 排序。
//! 本模块**不**生成 `EntityId`（`EntityId::new()` 是 ULID = 时间 + 随机 ⇒ 不可确定），
//! 也**不**构造 `yeban_model::MidiNote`；落到 `MidiExport` 需要的身份由集成者分配。

use std::collections::BTreeMap;

use crate::midi::{DEFAULT_PPQ, MidiTempo};

/// 解析时使用的**默认力度**。
///
/// MusicXML 的 `<note>` 默认不带力度 ⇒ 本 MVP 用固定值。
/// `<sound dynamics="…">` 未被读取（见模块文档的未实现清单）。
pub const DEFAULT_VELOCITY: u8 = 80;

/// 元素嵌套深度上限（超过 ⇒ [`MusicXmlError::DepthExceeded`]）。
///
/// 真实乐谱的深度约 10 层（`score-partwise/part/measure/note/notations/tied`）。
/// 上限的作用是让"恶意深嵌套"变成 `Err` 而不是 O(输入) 的栈增长。
pub const MAX_DEPTH: usize = 256;

/// 一个导入的音符（960 PPQ）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MusicXmlNote {
    /// MIDI 音高 0..=127。
    pub key: u8,
    /// 力度（本 MVP 恒为 [`DEFAULT_VELOCITY`]）。
    pub velocity: u8,
    /// 起始绝对 tick（960 PPQ）。
    pub start_tick: u64,
    /// 时值（tick，非零：零时值的音符不产生）。
    pub duration_ticks: u64,
    /// MusicXML 的 `<voice>` 文本（缺省 1）。
    pub voice: u16,
    /// MusicXML 的 `<staff>` 文本（缺省 1）。
    pub staff: u16,
}

impl MusicXmlNote {
    /// 结束 tick（`start_tick + duration_ticks`）。
    ///
    /// 用饱和加法：字段是 `pub` 的，调用方可以构造出会溢出的组合，
    /// 而本模块**不 panic** 的承诺不能被一个公开的加法函数打破。
    /// 解析器自己产出的音符总是满足 `start_tick + duration_ticks <= u64::MAX`。
    #[must_use]
    pub const fn end_tick(self) -> u64 {
        self.start_tick.saturating_add(self.duration_ticks)
    }

    /// 规范化排序键 `(start_tick, key, voice, staff, duration_ticks)`。
    #[must_use]
    pub const fn sort_key(self) -> (u64, u8, u16, u16, u64) {
        (
            self.start_tick,
            self.key,
            self.voice,
            self.staff,
            self.duration_ticks,
        )
    }
}

/// 一个 `<part>`（独立时间线）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicXmlPart {
    /// `<part id="…">` 的属性值（缺失 ⇒ 空字符串）。
    pub id: String,
    /// `part-list/score-part[id]/part-name` 的文本（未登记 ⇒ 空字符串）。
    pub name: String,
    /// 该部件的音符，按 [`MusicXmlNote::sort_key`] 升序。
    pub notes: Vec<MusicXmlNote>,
}

/// 一次只读导入的结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicXmlScore {
    /// 文件声明的 `divisions`（每四分音符的单位数）；文件未声明时为 **1**。
    pub divisions: u32,
    /// 输出 tick 的时间分度，恒为 [`DEFAULT_PPQ`]（960）。
    pub ppq: u16,
    /// 速度/拍号记录，按 `(tick, mpqn, numerator, denominator_pow2)` 升序。
    ///
    /// 每个 tick 上**内容相同**的记录只保留一条（不凭空合成：一条记录要么只带
    /// `mpqn`、要么只带拍号，与 `parse_smf` 的 R5 口径一致）。
    pub tempos: Vec<MidiTempo>,
    /// 部件，按文件里 `<part>` 的出现顺序。
    pub parts: Vec<MusicXmlPart>,
    /// 白名单**之外**的元素名 ⇒ 出现次数（出现即容错跳过，只登记）。
    pub ignored_elements: BTreeMap<String, u64>,
    /// **已知但本 MVP 未实现**的元素名 ⇒ 出现次数
    /// （`forward` / `grace` / `unpitched` / `transpose`；不重复计入 `ignored_elements`）。
    pub unsupported_elements: BTreeMap<String, u64>,
}

impl MusicXmlScore {
    /// 全部部件的音符总数。
    #[must_use]
    pub fn note_count(&self) -> usize {
        self.parts.iter().map(|part| part.notes.len()).sum()
    }

    /// 全部音符的 `(最小 tick, 最大 tick)`；没有音符 ⇒ `None`。
    ///
    /// 两个端点都取 `start_tick` 与 `end_tick`。
    #[must_use]
    pub fn tick_range(&self) -> Option<(u64, u64)> {
        let mut range: Option<(u64, u64)> = None;
        for note in self.parts.iter().flat_map(|part| part.notes.iter()) {
            let start = note.start_tick;
            let end = note.end_tick();
            range = Some(match range {
                None => (start, end),
                Some((low, high)) => (low.min(start), high.max(end)),
            });
        }
        range
    }
}

/// 只读导入失败的原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MusicXmlError {
    /// 输入不是合法 UTF-8（`.mxl` 这类 ZIP 字节走这里）。
    InvalidUtf8 {
        /// 第一个非法字节的偏移。
        offset: usize,
    },
    /// 输入里没有任何元素。
    Empty,
    /// 根元素不是 `score-partwise`（本 MVP 只支持 partwise）。
    UnsupportedRoot {
        /// 实际看到的根元素名。
        root: String,
    },
    /// 结构非法（未闭合的注释/声明/标签、标签不配对等）。
    Malformed {
        /// 出问题的字节偏移。
        offset: usize,
        /// 一句话说明。
        detail: &'static str,
    },
    /// 元素嵌套超过 [`MAX_DEPTH`]。
    DepthExceeded {
        /// 出问题的字节偏移。
        offset: usize,
    },
    /// 文本里出现未知实体引用。
    UnknownEntity {
        /// 实体名（不含 `&` 与 `;`）。
        name: String,
    },
    /// 某个叶子元素的文本不是期望的数字。
    InvalidNumber {
        /// 元素名。
        element: &'static str,
        /// 原始文本。
        text: String,
    },
    /// `divisions` 为 0（XSD 的 `positive-divisions` 要求 > 0）。
    DivisionsNotPositive,
    /// `<alter>` 不是整数半音（本 MVP 不支持微分音）。
    UnsupportedAlter {
        /// 原始文本。
        text: String,
    },
    /// `<beat-type>` 不是 2 的幂（无法表达为 `denominator_pow2`）。
    UnsupportedBeatType {
        /// 原始值。
        value: u32,
    },
    /// `<sound tempo="…">` 不是正的有限数。
    InvalidTempo {
        /// 原始文本。
        text: String,
    },
    /// 音高不在 0..=127。
    PitchOutOfRange {
        /// 音级字母。
        step: char,
        /// 半音偏移。
        alter: i32,
        /// 八度。
        octave: i32,
    },
    /// `<note>` 出现在任何 `<part>` 之外。
    NoteOutsidePart,
    /// `backup` 回退超过当前位置。
    BackupUnderflow {
        /// 回退前的 tick。
        tick: u64,
        /// 回退量（tick）。
        amount: u64,
    },
    /// tick 加法溢出 `u64`。
    TickOverflow,
}

impl core::fmt::Display for MusicXmlError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidUtf8 { offset } => write!(f, "偏移 {offset} 处不是合法 UTF-8"),
            Self::Empty => f.write_str("输入里没有任何元素"),
            Self::UnsupportedRoot { root } => {
                write!(f, "根元素不是 score-partwise: {root}")
            }
            Self::Malformed { offset, detail } => write!(f, "偏移 {offset} 处结构非法: {detail}"),
            Self::DepthExceeded { offset } => {
                write!(f, "偏移 {offset} 处的嵌套深度超过 {MAX_DEPTH}")
            }
            Self::UnknownEntity { name } => write!(f, "未知实体引用: &{name};"),
            Self::InvalidNumber { element, text } => {
                write!(f, "<{element}> 不是数字: {text}")
            }
            Self::DivisionsNotPositive => f.write_str("<divisions> 必须 > 0"),
            Self::UnsupportedAlter { text } => write!(f, "<alter> 不是整数半音: {text}"),
            Self::UnsupportedBeatType { value } => {
                write!(f, "<beat-type> 不是 2 的幂: {value}")
            }
            Self::InvalidTempo { text } => write!(f, "<sound tempo> 非法: {text}"),
            Self::PitchOutOfRange {
                step,
                alter,
                octave,
            } => write!(f, "音高越界: step={step} alter={alter} octave={octave}"),
            Self::NoteOutsidePart => f.write_str("<note> 出现在 <part> 之外"),
            Self::BackupUnderflow { tick, amount } => {
                write!(f, "<backup> 在 tick {tick} 回退 {amount}（越界）")
            }
            Self::TickOverflow => f.write_str("tick 加法溢出 u64"),
        }
    }
}

impl std::error::Error for MusicXmlError {}

/// 解析 `.musicxml` 字节（**只读**，见模块文档的白名单与未实现清单）。
///
/// # Errors
///
/// 见 [`MusicXmlError`]。任意字节输入都**不**会 panic。
pub fn parse_musicxml(bytes: &[u8]) -> Result<MusicXmlScore, MusicXmlError> {
    let text = core::str::from_utf8(bytes).map_err(|error| MusicXmlError::InvalidUtf8 {
        offset: error.valid_up_to(),
    })?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    Parser::new(text).run()
}

// ---------------------------------------------------------------------------
// 内部实现
// ---------------------------------------------------------------------------

/// XML 的一次词法事件。
enum Event<'a> {
    /// 起始标签（`empty = true` 表示 `<x/>`）。
    Start {
        /// 元素名。
        name: &'a str,
        /// 名字之后的原始属性文本（用 [`attr_value`] 取值，不预先分配）。
        attrs: &'a str,
        /// 自闭合标签。
        empty: bool,
    },
    /// 结束标签。
    End {
        /// 元素名。
        name: &'a str,
    },
    /// 元素之间的文本。
    Text(&'a str),
}

/// 极简 pull tokenizer：一个游标 + 逐字符扫描，零正则、零依赖。
struct Tokenizer<'a> {
    /// 已核验为 UTF-8 的输入文本。
    text: &'a str,
    /// 下一个待扫描的字节偏移。
    pos: usize,
}

impl<'a> Tokenizer<'a> {
    const fn new(text: &'a str) -> Self {
        Self { text, pos: 0 }
    }

    /// 取下一个事件；EOF ⇒ `None`。
    ///
    /// 不变式：每次调用要么返回事件，要么把 `pos` 推到 `text.len()` ⇒ 不会死循环。
    fn next(&mut self) -> Result<Option<Event<'a>>, MusicXmlError> {
        let bytes = self.text.as_bytes();
        while self.pos < bytes.len() {
            let rest = &self.text[self.pos..];
            if rest.starts_with("<!--") {
                let end = rest
                    .find("-->")
                    .ok_or_else(|| self.malformed("注释未闭合"))?;
                self.pos += end + 3;
                continue;
            }
            if rest.starts_with("<?") {
                let end = rest
                    .find("?>")
                    .ok_or_else(|| self.malformed("处理指令未闭合"))?;
                self.pos += end + 2;
                continue;
            }
            if rest.starts_with("<![CDATA[") {
                let end = rest
                    .find("]]>")
                    .ok_or_else(|| self.malformed("CDATA 未闭合"))?;
                let body = &rest[9..end];
                self.pos += end + 3;
                return Ok(Some(Event::Text(body)));
            }
            if rest.starts_with("<!") {
                // 声明（`<!DOCTYPE …>`，可能带 `[ … ]` 内部子集）。**不联网取 DTD**，只跳过。
                let raw = rest.as_bytes();
                let mut depth = 0usize;
                let mut index = 2usize;
                let mut found = false;
                while index < raw.len() {
                    match raw[index] {
                        b'[' => depth += 1,
                        b']' => depth = depth.saturating_sub(1),
                        b'>' if depth == 0 => {
                            found = true;
                            break;
                        }
                        _ => {}
                    }
                    index += 1;
                }
                if !found {
                    return Err(self.malformed("声明未闭合"));
                }
                self.pos += index + 1;
                continue;
            }
            if rest.starts_with("</") {
                let end = rest
                    .find('>')
                    .ok_or_else(|| self.malformed("结束标签未闭合"))?;
                let name = rest[2..end].trim();
                self.pos += end + 1;
                return Ok(Some(Event::End { name }));
            }
            if rest.starts_with('<') {
                let raw = rest.as_bytes();
                let mut index = 1usize;
                let mut quote = 0u8;
                let mut found = false;
                while index < raw.len() {
                    let byte = raw[index];
                    if quote != 0 {
                        if byte == quote {
                            quote = 0;
                        }
                    } else if byte == b'"' || byte == b'\'' {
                        quote = byte;
                    } else if byte == b'>' {
                        found = true;
                        break;
                    }
                    index += 1;
                }
                if !found {
                    return Err(self.malformed("起始标签未闭合"));
                }
                let body = &rest[1..index];
                let trimmed = body.trim_end();
                let empty = trimmed.ends_with('/');
                let body = if empty {
                    trimmed.trim_end_matches('/').trim_end()
                } else {
                    body
                };
                let name_end = body
                    .find(|character: char| character.is_ascii_whitespace() || character == '/')
                    .unwrap_or(body.len());
                let name = &body[..name_end];
                let attrs = &body[name_end..];
                self.pos += index + 1;
                return Ok(Some(Event::Start { name, attrs, empty }));
            }
            // 文本段：到下一个 `<` 为止（`rest` 不以 `<` 开头 ⇒ `end >= 1`）。
            let end = rest.find('<').unwrap_or(rest.len());
            let chunk = &rest[..end];
            self.pos += end;
            if !chunk.trim().is_empty() {
                return Ok(Some(Event::Text(chunk)));
            }
        }
        Ok(None)
    }

    /// 当前位置的错误。
    fn malformed(&self, detail: &'static str) -> MusicXmlError {
        MusicXmlError::Malformed {
            offset: self.pos,
            detail,
        }
    }
}

/// 从原始属性文本里取一个属性的值（`"` 或 `'` 引号都认）。
///
/// 零分配：返回的是 `attrs` 的切片。
fn attr_value<'a>(attrs: &'a str, key: &str) -> Option<&'a str> {
    let bytes = attrs.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        while index < bytes.len() && !is_name_byte(bytes[index]) {
            index += 1;
        }
        let start = index;
        while index < bytes.len() && is_name_byte(bytes[index]) {
            index += 1;
        }
        let name = &attrs[start..index];
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index < bytes.len() && bytes[index] == b'=' {
            index += 1;
            while index < bytes.len() && bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            if index < bytes.len() && (bytes[index] == b'"' || bytes[index] == b'\'') {
                let quote = bytes[index];
                index += 1;
                let value_start = index;
                while index < bytes.len() && bytes[index] != quote {
                    index += 1;
                }
                let value = &attrs[value_start..index];
                if name == key {
                    return Some(value);
                }
                if index < bytes.len() {
                    index += 1;
                }
                continue;
            }
        }
        if name == key {
            return None;
        }
        if index == start {
            index += 1;
        }
    }
    None
}

/// XML 名字字符（ASCII 足够覆盖 MusicXML；`:` 留给命名空间前缀）。
const fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-' || byte == b'.' || byte == b':'
}

/// 解析中的 `<note>` 累加器。
#[derive(Clone, Copy, Debug)]
struct NoteBuilder {
    start_tick: u64,
    chord: bool,
    grace: bool,
    duration: u64,
    voice: u16,
    staff: u16,
    step: Option<u8>,
    alter: i32,
    octave: Option<i32>,
    tie_seen: bool,
    tie_stop: bool,
    tied_stop: bool,
}

impl NoteBuilder {
    const fn new(start_tick: u64) -> Self {
        Self {
            start_tick,
            chord: false,
            grace: false,
            duration: 0,
            voice: 1,
            staff: 1,
            step: None,
            alter: 0,
            octave: None,
            tie_seen: false,
            tie_stop: false,
            tied_stop: false,
        }
    }

    /// 音高（`step` 与 `octave` 都在才有）。
    const fn pitch(&self) -> Option<(u8, i32, i32)> {
        match (self.step, self.octave) {
            (Some(step), Some(octave)) => Some((step, self.alter, octave)),
            _ => None,
        }
    }

    /// `true` = 这个音符是延音的**收尾**（`<tie>` 优先；没有 `<tie>` 才看
    /// `<notations>/<tied>`）。
    ///
    /// 只登记"收尾"就够：配对的方向是"收尾去找前驱"，因此起点侧不需要状态。
    const fn is_tie_stop(&self) -> bool {
        if self.tie_seen {
            self.tie_stop
        } else {
            self.tied_stop
        }
    }
}

/// 解析器状态机。
struct Parser<'a> {
    tokenizer: Tokenizer<'a>,
    stack: Vec<&'a str>,
    pending: String,
    divisions: u32,
    divisions_seen: bool,
    root_seen: bool,
    tempos: Vec<MidiTempo>,
    parts: Vec<MusicXmlPart>,
    part_names: BTreeMap<String, String>,
    current_score_part: Option<String>,
    in_score_part: bool,
    in_part: bool,
    cursor: u64,
    last_note_start: u64,
    note: Option<NoteBuilder>,
    in_backup: bool,
    backup_duration: u64,
    beats: Option<u8>,
    beat_type: Option<u32>,
    ignored: BTreeMap<String, u64>,
    unsupported: BTreeMap<String, u64>,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            tokenizer: Tokenizer::new(text),
            stack: Vec::new(),
            pending: String::new(),
            divisions: 1,
            divisions_seen: false,
            root_seen: false,
            tempos: Vec::new(),
            parts: Vec::new(),
            part_names: BTreeMap::new(),
            current_score_part: None,
            in_score_part: false,
            in_part: false,
            cursor: 0,
            last_note_start: 0,
            note: None,
            in_backup: false,
            backup_duration: 0,
            beats: None,
            beat_type: None,
            ignored: BTreeMap::new(),
            unsupported: BTreeMap::new(),
        }
    }

    fn run(mut self) -> Result<MusicXmlScore, MusicXmlError> {
        let mut saw_element = false;
        while let Some(event) = self.tokenizer.next()? {
            match event {
                Event::Text(raw) => {
                    let wanted = self.stack.last().is_some_and(|top| is_text_element(top));
                    if wanted {
                        self.push_text(raw)?;
                    }
                }
                Event::Start { name, attrs, empty } => {
                    saw_element = true;
                    if !self.root_seen {
                        self.root_seen = true;
                        if name != "score-partwise" {
                            return Err(MusicXmlError::UnsupportedRoot {
                                root: name.to_owned(),
                            });
                        }
                    }
                    if !empty {
                        if self.stack.len() >= MAX_DEPTH {
                            return Err(MusicXmlError::DepthExceeded {
                                offset: self.tokenizer.pos,
                            });
                        }
                        self.stack.push(name);
                    }
                    self.pending.clear();
                    self.on_start(name, attrs)?;
                    if empty {
                        self.on_end(name)?;
                    }
                }
                Event::End { name } => {
                    match self.stack.pop() {
                        Some(open) if open == name => {}
                        _ => {
                            return Err(MusicXmlError::Malformed {
                                offset: self.tokenizer.pos,
                                detail: "结束标签与起始标签不配对",
                            });
                        }
                    }
                    self.on_end(name)?;
                    self.pending.clear();
                }
            }
        }
        if !self.stack.is_empty() {
            return Err(MusicXmlError::Malformed {
                offset: self.tokenizer.text.len(),
                detail: "标签未闭合",
            });
        }
        if !saw_element {
            return Err(MusicXmlError::Empty);
        }
        for part in &mut self.parts {
            part.notes.sort_by_key(|note| note.sort_key());
        }
        self.tempos.sort_by_key(|tempo| {
            (
                tempo.tick,
                tempo.microseconds_per_quarter,
                tempo.numerator,
                tempo.denominator_pow2,
            )
        });
        Ok(MusicXmlScore {
            divisions: self.divisions,
            ppq: DEFAULT_PPQ,
            tempos: self.tempos,
            parts: self.parts,
            ignored_elements: self.ignored,
            unsupported_elements: self.unsupported,
        })
    }

    /// 起始标签（或自闭合标签）的语义。
    fn on_start(&mut self, name: &str, attrs: &str) -> Result<(), MusicXmlError> {
        match name {
            "part" => {
                self.in_part = true;
                let id = attr_value(attrs, "id").unwrap_or_default().to_owned();
                let part_name = self.part_names.get(&id).cloned().unwrap_or_default();
                self.parts.push(MusicXmlPart {
                    id,
                    name: part_name,
                    notes: Vec::new(),
                });
                self.cursor = 0;
                self.last_note_start = 0;
            }
            "score-part" => {
                self.in_score_part = true;
                self.current_score_part = attr_value(attrs, "id").map(ToOwned::to_owned);
            }
            "note" => {
                self.note = Some(NoteBuilder::new(self.cursor));
            }
            "chord" => {
                let head = self.last_note_start;
                if let Some(note) = self.note.as_mut() {
                    note.chord = true;
                    note.start_tick = head;
                }
            }
            "grace" => {
                if let Some(note) = self.note.as_mut() {
                    note.grace = true;
                }
            }
            "forward" | "unpitched" | "transpose" => {}
            "backup" => {
                self.in_backup = true;
                self.backup_duration = 0;
            }
            "time" => {
                self.beats = None;
                self.beat_type = None;
            }
            "tie" => {
                if let Some(note) = self.note.as_mut() {
                    note.tie_seen = true;
                    // `continue` = 中间的音符：既收上一个延音，又开启下一个。
                    if let Some("stop" | "continue") = attr_value(attrs, "type") {
                        note.tie_stop = true;
                    }
                }
            }
            "tied" => {
                if let Some(note) = self.note.as_mut()
                    && let Some("stop" | "continue") = attr_value(attrs, "type")
                {
                    note.tied_stop = true;
                }
            }
            "sound" => {
                if let Some(text) = attr_value(attrs, "tempo") {
                    let bpm =
                        text.trim()
                            .parse::<f64>()
                            .map_err(|_| MusicXmlError::InvalidTempo {
                                text: text.to_owned(),
                            })?;
                    if !bpm.is_finite() || bpm <= 0.0 {
                        return Err(MusicXmlError::InvalidTempo {
                            text: text.to_owned(),
                        });
                    }
                    let mpqn = (60_000_000.0 / bpm).round().clamp(1.0, 16_777_215.0) as u32;
                    self.push_tempo(MidiTempo {
                        tick: self.cursor,
                        microseconds_per_quarter: Some(mpqn),
                        numerator: None,
                        denominator_pow2: None,
                    });
                }
            }
            _ => {}
        }
        if is_unsupported_element(name) {
            self.count_unsupported(name);
        } else if !is_whitelisted(name) {
            self.count_ignored(name);
        }
        Ok(())
    }

    /// 结束标签的语义（文本已经攒在 `pending` 里）。
    fn on_end(&mut self, name: &str) -> Result<(), MusicXmlError> {
        match name {
            "divisions" => {
                let value = self.parse_u32("divisions")?;
                if value == 0 {
                    return Err(MusicXmlError::DivisionsNotPositive);
                }
                if !self.divisions_seen {
                    self.divisions = value;
                    self.divisions_seen = true;
                }
            }
            "note" => self.finish_note()?,
            "backup" => {
                let amount = ticks_from_units(self.backup_duration, self.divisions)?;
                if self.cursor < amount {
                    return Err(MusicXmlError::BackupUnderflow {
                        tick: self.cursor,
                        amount,
                    });
                }
                self.cursor -= amount;
                self.in_backup = false;
                self.backup_duration = 0;
            }
            "forward" => {}
            "duration" => {
                let value = self.parse_u64("duration")?;
                if self.in_backup {
                    self.backup_duration = value;
                } else if let Some(note) = self.note.as_mut() {
                    note.duration = value;
                }
            }
            "step" => {
                let text = self.pending.trim();
                let step = match text.chars().next() {
                    Some(character) => {
                        step_index(character).ok_or(MusicXmlError::InvalidNumber {
                            element: "step",
                            text: text.to_owned(),
                        })?
                    }
                    None => {
                        return Err(MusicXmlError::InvalidNumber {
                            element: "step",
                            text: String::new(),
                        });
                    }
                };
                if let Some(note) = self.note.as_mut() {
                    note.step = Some(step);
                }
            }
            "alter" => {
                let text = self.pending.trim().to_owned();
                let alter = text
                    .parse::<f64>()
                    .map_err(|_| MusicXmlError::InvalidNumber {
                        element: "alter",
                        text: text.clone(),
                    })?;
                if !alter.is_finite() || alter.fract() != 0.0 {
                    return Err(MusicXmlError::UnsupportedAlter { text });
                }
                if let Some(note) = self.note.as_mut() {
                    note.alter = alter as i32;
                }
            }
            "octave" => {
                let text = self.pending.trim().to_owned();
                let octave = text
                    .parse::<i32>()
                    .map_err(|_| MusicXmlError::InvalidNumber {
                        element: "octave",
                        text: text.clone(),
                    })?;
                if let Some(note) = self.note.as_mut() {
                    note.octave = Some(octave);
                }
            }
            "voice" => {
                let voice = self.parse_u16("voice")?;
                if let Some(note) = self.note.as_mut() {
                    note.voice = voice;
                }
            }
            "staff" => {
                let staff = self.parse_u16("staff")?;
                if let Some(note) = self.note.as_mut() {
                    note.staff = staff;
                }
            }
            "beats" => self.beats = Some(self.parse_u8("beats")?),
            "beat-type" => self.beat_type = Some(self.parse_u32("beat-type")?),
            "time" => {
                if let (Some(beats), Some(beat_type)) = (self.beats, self.beat_type) {
                    if beat_type == 0 || !beat_type.is_power_of_two() {
                        return Err(MusicXmlError::UnsupportedBeatType { value: beat_type });
                    }
                    self.push_tempo(MidiTempo {
                        tick: self.cursor,
                        microseconds_per_quarter: None,
                        numerator: Some(beats),
                        denominator_pow2: Some(beat_type.trailing_zeros() as u8),
                    });
                }
                self.beats = None;
                self.beat_type = None;
            }
            "part" => self.in_part = false,
            "score-part" => {
                self.in_score_part = false;
                self.current_score_part = None;
            }
            "part-name" => {
                // `<part-name>` 允许出现在两处：`part-list/score-part` 里（声明名字）
                // 与 MusicXML 4.0 的 `<part>` 里（就地命名）。
                let text = self.pending.trim().to_owned();
                if self.in_score_part {
                    if let Some(id) = self.current_score_part.clone() {
                        self.part_names.insert(id, text);
                    }
                } else if self.in_part
                    && let Some(part) = self.parts.last_mut()
                {
                    part.name = text;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// 收尾一个 `<note>`。
    fn finish_note(&mut self) -> Result<(), MusicXmlError> {
        let Some(note) = self.note.take() else {
            return Ok(());
        };
        if note.grace {
            // 未实现：`grace` 不发声、不推进 cursor（见模块文档）。
            return Ok(());
        }
        let duration = ticks_from_units(note.duration, self.divisions)?;
        let Some((step, alter, octave)) = note.pitch() else {
            // `<rest>` 或 `<unpitched>`：不发声，但按 duration 推进 cursor。
            if !note.chord {
                self.cursor = tick_add(self.cursor, duration)?;
            }
            return Ok(());
        };
        let key = pitch_to_key(step, alter, octave)?;
        let tie_stop = note.is_tie_stop();
        let part = self
            .parts
            .last_mut()
            .ok_or(MusicXmlError::NoteOutsidePart)?;
        let end = tick_add(note.start_tick, duration)?;
        let mut merged = false;
        if tie_stop
            && let Some(existing) = part.notes.iter_mut().rev().find(|existing| {
                existing.key == key
                    && existing.voice == note.voice
                    && existing.staff == note.staff
                    && existing.end_tick() == note.start_tick
            })
        {
            existing.duration_ticks = end - existing.start_tick;
            merged = true;
        }
        if !merged {
            part.notes.push(MusicXmlNote {
                key,
                velocity: DEFAULT_VELOCITY,
                start_tick: note.start_tick,
                duration_ticks: duration,
                voice: note.voice,
                staff: note.staff,
            });
        }
        self.last_note_start = note.start_tick;
        if note.chord {
            self.cursor = self.cursor.max(end);
        } else {
            self.cursor = end;
        }
        Ok(())
    }

    /// 文本段的实体展开（只对白名单叶子元素调用）。
    fn push_text(&mut self, raw: &str) -> Result<(), MusicXmlError> {
        let mut rest = raw;
        while let Some(index) = rest.find('&') {
            self.pending.push_str(&rest[..index]);
            let tail = &rest[index..];
            let semi = tail.find(';').ok_or(MusicXmlError::Malformed {
                offset: self.tokenizer.pos,
                detail: "实体引用未闭合",
            })?;
            let entity = &tail[1..semi];
            match entity {
                "amp" => self.pending.push('&'),
                "lt" => self.pending.push('<'),
                "gt" => self.pending.push('>'),
                "quot" => self.pending.push('"'),
                "apos" => self.pending.push('\''),
                _ => {
                    let code = entity.strip_prefix('#').and_then(|digits| {
                        match digits.strip_prefix(['x', 'X']) {
                            Some(hex) => u32::from_str_radix(hex, 16).ok(),
                            None => digits.parse::<u32>().ok(),
                        }
                    });
                    match code.and_then(char::from_u32) {
                        Some(character) => self.pending.push(character),
                        None => {
                            return Err(MusicXmlError::UnknownEntity {
                                name: entity.to_owned(),
                            });
                        }
                    }
                }
            }
            rest = &tail[semi + 1..];
        }
        self.pending.push_str(rest);
        Ok(())
    }

    /// 登记一个白名单之外的元素。
    fn count_ignored(&mut self, name: &str) {
        if let Some(count) = self.ignored.get_mut(name) {
            *count += 1;
        } else {
            self.ignored.insert(name.to_owned(), 1);
        }
    }

    /// 登记一个"已知但未实现"的元素。
    fn count_unsupported(&mut self, name: &str) {
        if let Some(count) = self.unsupported.get_mut(name) {
            *count += 1;
        } else {
            self.unsupported.insert(name.to_owned(), 1);
        }
    }

    /// 同一 tick 上内容相同的记录只保留一条。
    fn push_tempo(&mut self, record: MidiTempo) {
        if !self.tempos.contains(&record) {
            self.tempos.push(record);
        }
    }

    fn parse_u64(&self, element: &'static str) -> Result<u64, MusicXmlError> {
        self.pending
            .trim()
            .parse::<u64>()
            .map_err(|_| MusicXmlError::InvalidNumber {
                element,
                text: self.pending.trim().to_owned(),
            })
    }

    fn parse_u32(&self, element: &'static str) -> Result<u32, MusicXmlError> {
        self.pending
            .trim()
            .parse::<u32>()
            .map_err(|_| MusicXmlError::InvalidNumber {
                element,
                text: self.pending.trim().to_owned(),
            })
    }

    fn parse_u16(&self, element: &'static str) -> Result<u16, MusicXmlError> {
        self.pending
            .trim()
            .parse::<u16>()
            .map_err(|_| MusicXmlError::InvalidNumber {
                element,
                text: self.pending.trim().to_owned(),
            })
    }

    fn parse_u8(&self, element: &'static str) -> Result<u8, MusicXmlError> {
        self.pending
            .trim()
            .parse::<u8>()
            .map_err(|_| MusicXmlError::InvalidNumber {
                element,
                text: self.pending.trim().to_owned(),
            })
    }
}

/// 需要读取文本的元素（其余元素的文本被丢弃，零分配）。
fn is_text_element(name: &str) -> bool {
    matches!(
        name,
        "step"
            | "alter"
            | "octave"
            | "duration"
            | "voice"
            | "staff"
            | "divisions"
            | "beats"
            | "beat-type"
            | "part-name"
    )
}

/// "已知但本 MVP 未实现"的元素（登记在 [`MusicXmlScore::unsupported_elements`](crate::musicxml::MusicXmlScore::unsupported_elements)）。
fn is_unsupported_element(name: &str) -> bool {
    matches!(name, "forward" | "grace" | "unpitched" | "transpose")
}

/// 白名单：本 MVP 会**按其位置决定行为**的元素。
///
/// 不在此列表里的元素：容错跳过（子元素仍会被遍历）+ 登记。
fn is_whitelisted(name: &str) -> bool {
    matches!(
        name,
        "score-partwise"
            | "part"
            | "measure"
            | "attributes"
            | "divisions"
            | "time"
            | "beats"
            | "beat-type"
            | "note"
            | "chord"
            | "pitch"
            | "rest"
            | "step"
            | "alter"
            | "octave"
            | "duration"
            | "tie"
            | "tied"
            | "voice"
            | "staff"
            | "backup"
            | "sound"
            | "notations"
            | "part-list"
            | "score-part"
            | "part-name"
    )
}

/// 音级字母 ⇒ `C=0 .. B=6`。
const fn step_index(character: char) -> Option<u8> {
    match character {
        'C' => Some(0),
        'D' => Some(2),
        'E' => Some(4),
        'F' => Some(5),
        'G' => Some(7),
        'A' => Some(9),
        'B' => Some(11),
        _ => None,
    }
}

/// MusicXML 音高 ⇒ MIDI 音高（C4 = 60）。
///
/// # Errors
///
/// 结果不在 `0..=127`（MIDI 音高的**可表达范围**，不是 `u8` 的范围）。
fn pitch_to_key(step: u8, alter: i32, octave: i32) -> Result<u8, MusicXmlError> {
    // 先升到 i64：`octave + 1` 用 i32 会在 `<octave>2147483647</octave>` 上溢出
    // （debug 构建直接 panic）—— 本模块不 panic 的承诺要求这里不能有裸加法。
    let key = (i64::from(octave) + 1) * 12 + i64::from(step) + i64::from(alter);
    if (0..=127).contains(&key) {
        Ok(key as u8)
    } else {
        Err(MusicXmlError::PitchOutOfRange {
            step: step_letter(step),
            alter,
            octave,
        })
    }
}

/// 音级下标 ⇒ 字母（错误上报用）。
const fn step_letter(step: u8) -> char {
    match step {
        0 => 'C',
        2 => 'D',
        4 => 'E',
        5 => 'F',
        7 => 'G',
        9 => 'A',
        _ => 'B',
    }
}

/// `divisions` 单位 ⇒ 960 PPQ tick（四舍五入）。
///
/// # Errors
///
/// 结果超过 `u64`。
fn ticks_from_units(units: u64, divisions: u32) -> Result<u64, MusicXmlError> {
    let numerator = u128::from(units) * u128::from(DEFAULT_PPQ);
    let denominator = u128::from(divisions);
    let rounded = (numerator + denominator / 2) / denominator;
    u64::try_from(rounded).map_err(|_| MusicXmlError::TickOverflow)
}

/// 不会溢出的 tick 加法。
fn tick_add(left: u64, right: u64) -> Result<u64, MusicXmlError> {
    left.checked_add(right).ok_or(MusicXmlError::TickOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(xml: &str) -> MusicXmlScore {
        parse_musicxml(xml.as_bytes()).expect("解析应当成功")
    }

    #[test]
    fn root_must_be_score_partwise() {
        let xml = "<score-timewise version=\"4.0\"></score-timewise>";
        assert_eq!(
            parse_musicxml(xml.as_bytes()),
            Err(MusicXmlError::UnsupportedRoot {
                root: "score-timewise".to_owned()
            })
        );
    }

    #[test]
    fn doctype_and_declaration_are_skipped_without_network() {
        let xml = "<?xml version=\"1.0\"?>\n\
             <!DOCTYPE score-partwise PUBLIC \"-//Recordare//DTD MusicXML 4.0 Partwise//EN\"\n\
             \"http://www.musicxml.org/dtds/partwise.dtd\" [ <!ENTITY x \"y\"> ]>\n\
             <!-- comment with <tags> --><score-partwise version=\"4.0\"></score-partwise>";
        let parsed = score(xml);
        assert_eq!(parsed.divisions, 1);
        assert!(parsed.parts.is_empty());
    }

    #[test]
    fn empty_input_is_an_error() {
        assert_eq!(parse_musicxml(b""), Err(MusicXmlError::Empty));
        assert_eq!(parse_musicxml(b"   \n"), Err(MusicXmlError::Empty));
    }

    #[test]
    fn non_utf8_is_an_error_not_a_panic() {
        assert_eq!(
            parse_musicxml(&[0x50, 0x4b, 0x03, 0x04, 0xff, 0x00]),
            Err(MusicXmlError::InvalidUtf8 { offset: 4 })
        );
    }

    #[test]
    fn entities_are_expanded_in_part_names() {
        let xml = "<score-partwise><part-list><score-part id=\"P1\">\
             <part-name>A &amp; B &#8212; &#x4e2d;</part-name></score-part></part-list>\
             <part id=\"P1\"></part></score-partwise>";
        let parsed = score(xml);
        assert_eq!(parsed.parts[0].name, "A & B — 中");
    }

    #[test]
    fn unknown_entity_is_an_error() {
        let xml = "<score-partwise><part-list><score-part id=\"P1\">\
             <part-name>&nbsp;</part-name></score-part></part-list></score-partwise>";
        assert_eq!(
            parse_musicxml(xml.as_bytes()),
            Err(MusicXmlError::UnknownEntity {
                name: "nbsp".to_owned()
            })
        );
    }

    #[test]
    fn attribute_values_use_both_quote_styles() {
        assert_eq!(attr_value(" id='P1' x=\"2\"", "id"), Some("P1"));
        assert_eq!(attr_value(" id='P1' x=\"2\"", "x"), Some("2"));
        assert_eq!(attr_value(" id='P1'", "missing"), None);
        assert_eq!(attr_value("", "id"), None);
    }

    #[test]
    fn unknown_elements_are_registered_and_still_traversed() {
        let xml = "<score-partwise><part id=\"P1\"><measure number=\"1\">\
             <attributes><divisions>1</divisions></attributes>\
             <note><pitch><step>C</step><octave>4</octave></pitch><duration>1</duration>\
             <notations><slur type=\"start\"/><tied type=\"start\"/></notations></note>\
             <note><pitch><step>C</step><octave>4</octave></pitch><duration>1</duration>\
             <notations><tied type=\"stop\"/></notations></note>\
             </measure></part></score-partwise>";
        let parsed = score(xml);
        // `<tied>` 在未知容器 `<notations>` 内也必须被读到 ⇒ 两个音符合并成一个。
        assert_eq!(parsed.note_count(), 1);
        assert_eq!(parsed.parts[0].notes[0].duration_ticks, 1920);
        assert_eq!(parsed.ignored_elements.get("slur"), Some(&1));
        assert_eq!(parsed.ignored_elements.get("slur"), Some(&1));
    }

    #[test]
    fn empty_tag_does_not_push_the_stack() {
        // `<x/>` 后紧跟 `</score-partwise>` ⇒ 若自闭合标签入栈，这里会报"不配对"。
        let xml = "<score-partwise><part-list><score-part id=\"P1\"><part-name>N</part-name>\
             </score-part></part-list><part id=\"P1\"/></score-partwise>";
        assert_eq!(score(xml).parts[0].name, "N");
    }

    #[test]
    fn deep_nesting_is_an_error() {
        let mut xml = String::from("<score-partwise>");
        for _ in 0..(MAX_DEPTH + 8) {
            xml.push_str("<x>");
        }
        assert!(matches!(
            parse_musicxml(xml.as_bytes()),
            Err(MusicXmlError::DepthExceeded { .. })
        ));
    }

    #[test]
    fn mismatched_tags_are_an_error() {
        let xml = "<score-partwise><part-list></part></score-partwise>";
        assert!(matches!(
            parse_musicxml(xml.as_bytes()),
            Err(MusicXmlError::Malformed { .. })
        ));
    }

    #[test]
    fn unclosed_tag_at_eof_is_an_error() {
        let xml = "<score-partwise><part-list>";
        assert!(matches!(
            parse_musicxml(xml.as_bytes()),
            Err(MusicXmlError::Malformed { .. })
        ));
    }

    #[test]
    fn divisions_zero_is_rejected() {
        let xml = "<score-partwise><part id=\"P1\"><measure><attributes><divisions>0</divisions>\
             </attributes></measure></part></score-partwise>";
        assert_eq!(
            parse_musicxml(xml.as_bytes()),
            Err(MusicXmlError::DivisionsNotPositive)
        );
    }

    #[test]
    fn pitch_octave_mapping_matches_c4_equals_60() {
        assert_eq!(pitch_to_key(0, 0, 4), Ok(60));
        assert_eq!(pitch_to_key(2, -1, 4), Ok(61));
        // MIDI 0 = C-1 ⇒ B-1 = 11（八度边界也要按同一公式）。
        assert_eq!(pitch_to_key(11, 0, -1), Ok(11));
        assert!(matches!(
            pitch_to_key(0, 0, 20),
            Err(MusicXmlError::PitchOutOfRange { .. })
        ));
    }

    #[test]
    fn tick_conversion_is_exact_for_the_corpus_divisions() {
        // divisions ∈ {1, 2, 12} 全部整除 960 ⇒ 期望是无损的整数换算。
        assert_eq!(ticks_from_units(1, 1), Ok(960));
        assert_eq!(ticks_from_units(1, 2), Ok(480));
        assert_eq!(ticks_from_units(1, 12), Ok(80));
        assert_eq!(ticks_from_units(3, 12), Ok(240));
        // 非整除时四舍五入，而不是截断成 0。
        assert_eq!(ticks_from_units(1, 7), Ok(137));
        assert_eq!(
            ticks_from_units(u64::MAX, 1),
            Err(MusicXmlError::TickOverflow)
        );
    }

    #[test]
    fn huge_durations_are_errors_not_panics() {
        let note = "<note><pitch><step>C</step><octave>4</octave></pitch>\
             <duration>6000000000000000000</duration></note>";
        let xml = format!(
            "<score-partwise><part id=\"P1\"><measure><attributes><divisions>1</divisions>\
             </attributes>{note}{note}{note}{note}</measure></part></score-partwise>"
        );
        assert_eq!(
            parse_musicxml(xml.as_bytes()),
            Err(MusicXmlError::TickOverflow)
        );
    }

    #[test]
    fn backup_before_the_measure_start_is_an_error() {
        let xml = "<score-partwise><part id=\"P1\"><measure>\
             <attributes><divisions>1</divisions></attributes>\
             <backup><duration>4</duration></backup></measure></part></score-partwise>";
        assert_eq!(
            parse_musicxml(xml.as_bytes()),
            Err(MusicXmlError::BackupUnderflow {
                tick: 0,
                amount: 3840
            })
        );
    }

    #[test]
    fn note_outside_a_part_is_an_error() {
        let xml = "<score-partwise><measure><note><pitch><step>C</step><octave>4</octave></pitch>\
             <duration>1</duration></note></measure></score-partwise>";
        assert_eq!(
            parse_musicxml(xml.as_bytes()),
            Err(MusicXmlError::NoteOutsidePart)
        );
    }

    #[test]
    fn unsupported_elements_are_registered_separately() {
        let xml = "<score-partwise><part id=\"P1\"><measure>\
             <attributes><divisions>1</divisions><transpose><chromatic>-2</chromatic></transpose>\
             </attributes>\
             <forward><duration>1</duration></forward>\
             <note><grace/><pitch><step>C</step><octave>4</octave></pitch></note>\
             <note><unpitched/><duration>1</duration></note>\
             </measure></part></score-partwise>";
        let parsed = score(xml);
        assert_eq!(parsed.unsupported_elements.get("transpose"), Some(&1));
        assert_eq!(parsed.unsupported_elements.get("forward"), Some(&1));
        assert_eq!(parsed.unsupported_elements.get("grace"), Some(&1));
        assert_eq!(parsed.unsupported_elements.get("unpitched"), Some(&1));
        // `grace` 不发声、`unpitched` 不发声（但按 duration 推进）⇒ 零音符。
        assert_eq!(parsed.note_count(), 0);
        assert_eq!(parsed.ignored_elements.get("chromatic"), Some(&1));
    }

    #[test]
    fn a_score_parses_the_same_way_twice() {
        let xml = "<score-partwise><part id=\"P1\"><measure number=\"1\">\
             <attributes><divisions>3</divisions><time><beats>6</beats><beat-type>8</beat-type></time>\
             </attributes>\
             <direction><sound tempo=\"132\"/></direction>\
             <note><pitch><step>F</step><alter>1</alter><octave>5</octave></pitch>\
             <duration>2</duration><voice>2</voice><staff>3</staff></note>\
             </measure></part></score-partwise>";
        assert_eq!(score(xml), score(xml));
    }

    #[test]
    fn tempo_records_are_deduplicated_by_content() {
        // 同 tick 的两条**内容相同**的记录合成一条；内容不同的保留两条。
        let xml = "<score-partwise><part id=\"P1\"><measure>\
             <direction><sound tempo=\"120\"/></direction>\
             <direction><sound tempo=\"120\"/></direction>\
             <direction><sound tempo=\"121\"/></direction>\
             </measure></part></score-partwise>";
        let parsed = score(xml);
        assert_eq!(parsed.tempos.len(), 2);
        assert_eq!(parsed.tempos[0].microseconds_per_quarter, Some(495_868));
        assert_eq!(parsed.tempos[1].microseconds_per_quarter, Some(500_000));
        assert!(parsed.tempos.iter().all(|tempo| tempo.tick == 0));
    }

    // -----------------------------------------------------------------------
    // 第二批判据（本票新增）：数值边界、配对方向、tick 累加溢出、拍号/调号极值
    //
    // 每条的"补的是哪个缺口"由同票的注入实测给出（字面替换表见提交正文）：
    // 这些替换在本节之前让本 crate 的全部判据保持**绿**。
    // -----------------------------------------------------------------------

    /// `divisions` 只认**第一次**出现的那个（模块文档的未实现清单第 5 条）。
    ///
    /// 注入实测（本票）：去掉"只认第一次"的开关后全绿 —— 既有夹具都只声明一次。
    #[test]
    fn only_the_first_divisions_element_wins() {
        let xml = "<score-partwise><part id=\"P1\"><measure>\
             <attributes><divisions>2</divisions></attributes>\
             <note><pitch><step>C</step><octave>4</octave></pitch><duration>1</duration></note>\
             <attributes><divisions>8</divisions></attributes>\
             <note><pitch><step>D</step><octave>4</octave></pitch><duration>1</duration></note>\
             </measure></part></score-partwise>";
        let parsed = score(xml);
        assert_eq!(parsed.divisions, 2, "⛔ 第二次的 divisions 不许覆盖第一次");
        let starts: Vec<u64> = parsed.parts[0]
            .notes
            .iter()
            .map(|note| note.start_tick)
            .collect();
        assert_eq!(starts, vec![0, 480], "1 unit = 960 / 2 = 480 tick");
    }

    /// `<sound tempo>` 拒绝非有限/非正数，并把 `mpqn` 钳在 `1..=0x00FF_FFFF`。
    ///
    /// 注入实测（本票）：去掉 `bpm.is_finite()` 后全绿（`inf` 会落成 `mpqn = 1`）；
    /// 把下钳从 `1.0` 放到 `0.0` 后也全绿（`1e300` 会落成 `mpqn = 0`）。
    #[test]
    fn sound_tempo_rejects_non_finite_and_clamps_both_ends() {
        let document = |tempo: &str| {
            format!(
                "<score-partwise><direction><sound tempo=\"{tempo}\"/></direction></score-partwise>"
            )
        };
        for tempo in ["inf", "-inf", "nan"] {
            assert_eq!(
                parse_musicxml(document(tempo).as_bytes()),
                Err(MusicXmlError::InvalidTempo {
                    text: tempo.to_owned()
                }),
                "{tempo} 不是正的有限数"
            );
        }
        assert_eq!(
            score(&document("1e300")).tempos[0].microseconds_per_quarter,
            Some(1),
            "极大 BPM 的 mpqn 下钳到 1 (⛔ 不是 0)"
        );
        assert_eq!(
            score(&document("1e-300")).tempos[0].microseconds_per_quarter,
            Some(0x00FF_FFFF),
            "u24 的上界"
        );
    }

    /// `<beats>` 是 `u8`：255 必须接受，256 必须拒绝；0 是 u8 的合法值
    /// （⚠️ 本模块**不**校验正数，这里钉住现状）。
    ///
    /// 注入实测（本票）：给 `parse_u8("beats")` 加一个 `.min(4)` 后全绿
    /// （既有夹具的 beats ∈ {3,4,6}）。
    #[test]
    fn beats_is_a_u8_and_its_boundary_is_enforced() {
        let document = |beats: &str| {
            format!(
                "<score-partwise><part id=\"P1\"><measure><attributes><time>\
                 <beats>{beats}</beats><beat-type>4</beat-type></time></attributes>\
                 </measure></part></score-partwise>"
            )
        };
        let parsed = score(&document("255"));
        assert_eq!(parsed.tempos[0].numerator, Some(255), "255 是 u8 的上界");
        assert_eq!(parsed.tempos[0].denominator_pow2, Some(2));
        assert_eq!(
            parse_musicxml(document("256").as_bytes()),
            Err(MusicXmlError::InvalidNumber {
                element: "beats",
                text: "256".to_owned()
            })
        );
        assert_eq!(
            score(&document("0")).tempos[0].numerator,
            Some(0),
            "⚠️ 现状: beats=0 被原样接受 (⛔ 不是承诺)"
        );
    }

    /// `<beat-type>` 的上界：`2^31` 是 `u32` 里最大的 2 的幂 ⇒ `denominator_pow2 = 31`；
    /// 非 2 的幂与超过 `u32` 的文本各自被明确拒绝。
    #[test]
    fn beat_type_is_accepted_up_to_the_power_of_two_boundary() {
        let document = |beat_type: &str| {
            format!(
                "<score-partwise><part id=\"P1\"><measure><attributes><time>\
                 <beats>3</beats><beat-type>{beat_type}</beat-type></time></attributes>\
                 </measure></part></score-partwise>"
            )
        };
        assert_eq!(
            score(&document("2147483648")).tempos[0].denominator_pow2,
            Some(31),
            "2^31"
        );
        assert_eq!(
            parse_musicxml(document("2147483649").as_bytes()),
            Err(MusicXmlError::UnsupportedBeatType {
                value: 2_147_483_649
            })
        );
        assert_eq!(
            parse_musicxml(document("4294967296").as_bytes()),
            Err(MusicXmlError::InvalidNumber {
                element: "beat-type",
                text: "4294967296".to_owned()
            })
        );
    }

    /// 只有一半的 `<time>`（只有 beats 或只有 beat-type）**不写** tempo 记录。
    #[test]
    fn a_half_filled_time_element_writes_no_record() {
        let xml = "<score-partwise><part id=\"P1\"><measure>\
             <attributes><time><beats>4</beats></time>\
             <time><beat-type>4</beat-type></time></attributes>\
             </measure></part></score-partwise>";
        assert!(score(xml).tempos.is_empty(), "两个字段都齐了才写记录");
    }

    /// `<key><fifths>`（调号）的边界值 `-7` / `0` / `+7`：白名单外 ⇒ 跳过并登记，
    /// **不**移调、**不**移动 tick、**不**进 tempo map。
    ///
    /// ⛔ `fifths` 不在本 MVP 的语义里（模块文档的未实现清单）：这里钉住的是
    /// "如实跳过"，不是"支持调号"。
    #[test]
    fn key_signature_fifths_are_ignored_at_their_boundaries() {
        let document = |fifths: i32| {
            format!(
                "<score-partwise><part id=\"P1\"><measure>\
                 <attributes><divisions>1</divisions><key><fifths>{fifths}</fifths></key>\
                 </attributes>\
                 <note><pitch><step>C</step><octave>4</octave></pitch><duration>1</duration></note>\
                 </measure></part></score-partwise>"
            )
        };
        for fifths in [-7, 0, 7] {
            let parsed = score(&document(fifths));
            assert_eq!(parsed.note_count(), 1, "fifths={fifths} 不改变音符数");
            assert_eq!(parsed.parts[0].notes[0].start_tick, 0);
            assert_eq!(parsed.parts[0].notes[0].key, 60, "调号不移调 (⛔ 不猜)");
            assert_eq!(parsed.ignored_elements.get("key"), Some(&1));
            assert_eq!(parsed.ignored_elements.get("fifths"), Some(&1));
            assert!(parsed.tempos.is_empty(), "调号不是 tempo map 记录");
        }
    }

    /// `pitch_to_key` 的两端：127 是最高可表达音高（G9），128 与 -1 都越界。
    ///
    /// 注入实测（本票）：把 `0..=127` 收成 `0..=126` 后全绿
    /// （既有判据没有站在 127 上的音高）。
    #[test]
    fn pitch_127_is_the_highest_expressible_key() {
        assert_eq!(pitch_to_key(7, 0, 9), Ok(127), "G9 = (9+1)*12 + 7 = 127");
        assert_eq!(
            pitch_to_key(7, 1, 9),
            Err(MusicXmlError::PitchOutOfRange {
                step: 'G',
                alter: 1,
                octave: 9
            }),
            "G#9 = 128 ⇒ 越界"
        );
        assert_eq!(pitch_to_key(0, 0, -1), Ok(0), "C-1 = MIDI 0");
        assert_eq!(
            pitch_to_key(0, -1, -1),
            Err(MusicXmlError::PitchOutOfRange {
                step: 'C',
                alter: -1,
                octave: -1
            }),
            "-1 ⇒ 越界"
        );
    }

    /// `ticks_from_units` 在**半个 tick** 处四舍五入（不是截断）。
    ///
    /// 注入实测（本票）：把 `(numerator + denominator / 2) / denominator`
    /// 换成 `numerator / denominator` 后全绿（既有断言的商都不是 `x.5`）。
    #[test]
    fn tick_conversion_rounds_half_up_at_the_half_tick() {
        assert_eq!(ticks_from_units(1, 128), Ok(8), "960/128 = 7.5 ⇒ 8");
        assert_eq!(ticks_from_units(3, 128), Ok(23), "2880/128 = 22.5 ⇒ 23");
        assert_eq!(ticks_from_units(1, 100), Ok(10), "960/100 = 9.6 ⇒ 10");
        assert_eq!(ticks_from_units(1, 200), Ok(5), "960/200 = 4.8 ⇒ 5");
    }

    /// tick 累加越过 `u64` 上界 ⇒ 明确 `TickOverflow`（⛔ 不是饱和、不是回绕）。
    ///
    /// 注入实测（本票）：把 `tick_add` 的 `checked_add` 换成 `saturating_add` 后全绿
    /// —— 既有判据的巨型 `duration` 在 `ticks_from_units` 那一步就先越界了。
    #[test]
    fn a_second_note_past_the_tick_axis_is_an_error_not_a_wrap() {
        // divisions = 1 ⇒ 1 unit = 960 tick。1e16 unit = 9.6e18 tick（装得进 u64），
        // 两颗这样的音符 ⇒ 第二颗的结束 tick 越过 u64 上界。
        let note = "<note><pitch><step>C</step><octave>4</octave></pitch>\
             <duration>10000000000000000</duration></note>";
        let xml = format!(
            "<score-partwise><part id=\"P1\"><measure>\
             <attributes><divisions>1</divisions></attributes>{note}{note}\
             </measure></part></score-partwise>"
        );
        assert_eq!(
            parse_musicxml(xml.as_bytes()),
            Err(MusicXmlError::TickOverflow)
        );
    }

    /// 延音配对只认**恰好相接**的前驱：中间隔了缺口就不合并。
    ///
    /// 注入实测（本票）：去掉 `existing.end_tick() == note.start_tick` 后全绿
    /// （既有夹具的延音都恰好相接）。
    #[test]
    fn a_tie_stop_pairs_only_with_an_exactly_adjacent_note() {
        // divisions = 2 ⇒ 1 unit = 480 tick（`duration` 的单位是 unit，不是 tick）。
        let xml = "<score-partwise><part id=\"P1\"><measure>\
             <attributes><divisions>2</divisions></attributes>\
             <note><pitch><step>C</step><octave>4</octave></pitch><duration>1</duration>\
             <tie type=\"start\"/></note>\
             <note><rest/><duration>1</duration></note>\
             <note><pitch><step>C</step><octave>4</octave></pitch><duration>1</duration>\
             <tie type=\"stop\"/></note>\
             </measure></part></score-partwise>";
        let parsed = score(xml);
        let notes: Vec<(u64, u64)> = parsed.parts[0]
            .notes
            .iter()
            .map(|note| (note.start_tick, note.duration_ticks))
            .collect();
        assert_eq!(
            notes,
            vec![(0, 480), (960, 480)],
            "中间 480 tick 的缺口 ⇒ 不许合并成一颗"
        );
    }

    /// 延音收尾延的是**最近写入**的那颗同键前驱（`rev()`），不是最早的那颗。
    ///
    /// 注入实测（本票）：把 `iter_mut().rev().find(..)` 换成 `iter_mut().find(..)`
    /// 后全绿（既有夹具的候选前驱只有一个）。
    #[test]
    fn a_tie_stop_extends_the_latest_matching_note() {
        // divisions = 4 ⇒ 1 unit = 240 tick。
        // A: 0..480；backup 240；B: 240..480（与 A 同键、同 end_tick）；
        // 收尾音符从 480 起 ⇒ rev 选 B（240..960），find 会选 A（0..960）。
        let xml = "<score-partwise><part id=\"P1\"><measure>\
             <attributes><divisions>4</divisions></attributes>\
             <note><pitch><step>C</step><octave>4</octave></pitch><duration>2</duration></note>\
             <backup><duration>1</duration></backup>\
             <note><pitch><step>C</step><octave>4</octave></pitch><duration>1</duration></note>\
             <note><pitch><step>C</step><octave>4</octave></pitch><duration>2</duration>\
             <tie type=\"stop\"/></note>\
             </measure></part></score-partwise>";
        let mut notes: Vec<(u64, u64)> = score(xml).parts[0]
            .notes
            .iter()
            .map(|note| (note.start_tick, note.duration_ticks))
            .collect();
        notes.sort_unstable();
        assert_eq!(
            notes,
            vec![(0, 480), (240, 720)],
            "收尾延的是最近写入的那颗 (240..960)"
        );
    }

    /// 和弦音符**不推进** cursor，也不许把它拉回去：cursor 取 `max`。
    ///
    /// 注入实测（本票）：把 `self.cursor.max(end)` 换成 `self.cursor = end` 后全绿
    /// （既有夹具的和弦都不短于当拍 cursor）。
    #[test]
    fn a_chord_note_shorter_than_the_cursor_does_not_rewind_it() {
        // divisions = 2 ⇒ 1 unit = 480 tick。第一颗 2 unit（960 tick）；和声 1 unit
        // （480 tick，比 cursor 短）；第三颗从 cursor 起。
        let xml = "<score-partwise><part id=\"P1\"><measure>\
             <attributes><divisions>2</divisions></attributes>\
             <note><pitch><step>C</step><octave>4</octave></pitch><duration>2</duration></note>\
             <note><chord/><pitch><step>E</step><octave>4</octave></pitch><duration>1</duration></note>\
             <note><pitch><step>G</step><octave>4</octave></pitch><duration>2</duration></note>\
             </measure></part></score-partwise>";
        let starts: Vec<(u64, u8)> = score(xml).parts[0]
            .notes
            .iter()
            .map(|note| (note.start_tick, note.key))
            .collect();
        assert_eq!(
            starts,
            vec![(0, 60), (0, 64), (960, 67)],
            "和声不推进 cursor; 第三颗从 960 起"
        );
    }

    /// 嵌套深度上界：`MAX_DEPTH` 是**字面** 256；恰好 256 层接受，257 层拒绝。
    ///
    /// 注入实测（本票）：把 `MAX_DEPTH` 从 256 降到 64 后，**只看常量**的判据会跟着
    /// 缩放而全绿 ⇒ 本判据因此把两边都写成字面值（`assert_eq!(MAX_DEPTH, 256)`
    /// 与固定深度的两份文档），常量一改就先红。
    #[test]
    fn the_depth_limit_is_256_and_inclusive_below_it() {
        assert_eq!(MAX_DEPTH, 256, "深度上限是字面读数, ⛔ 不许静默缩放");
        let document = |depth: usize| {
            format!(
                "<score-partwise>{}{}</score-partwise>",
                "<x>".repeat(depth),
                "</x>".repeat(depth)
            )
        };
        assert!(
            parse_musicxml(document(MAX_DEPTH - 1).as_bytes()).is_ok(),
            "根元素 + 255 层 = 256 层必须接受"
        );
        assert!(
            matches!(
                parse_musicxml(document(MAX_DEPTH).as_bytes()),
                Err(MusicXmlError::DepthExceeded { .. })
            ),
            "根元素 + 256 层 = 257 层必须拒绝"
        );
    }

    /// 判据 (类别④ 参数极值 / 类别① 越界输入): `<octave>` 取 `i32` 的极值时是明确的
    /// `PitchOutOfRange`，⛔ 不是 `octave + 1` 的溢出 panic。
    ///
    /// 补的是哪个缺口（本票注入实测）：把 `pitch_to_key` 的算术从 `i64` 降回 `i32`
    /// （注入 X1，即 `(i64::from(octave) + 1) * 12 + …` → `(octave + 1) * 12 + …`）
    /// 后，**118** 条判据全绿 ⇒ 代码注释里点名的那颗炸弹
    /// （`<octave>2147483647</octave>`）当时**没有判据**引爆过。
    #[test]
    fn an_extreme_octave_is_an_error_not_an_overflow() {
        for octave in ["2147483647", "-2147483648"] {
            let xml = format!(
                "<score-partwise><part id=\"P1\"><measure>\
                 <note><pitch><step>C</step><octave>{octave}</octave></pitch>\
                 <duration>1</duration></note></measure></part></score-partwise>"
            );
            assert!(
                matches!(
                    parse_musicxml(xml.as_bytes()),
                    Err(MusicXmlError::PitchOutOfRange { .. })
                ),
                "<octave>{octave}</octave> 必须是 PitchOutOfRange, 不是 panic"
            );
        }
    }

    /// 判据 (类别② 变更后沿用旧数据 / 类别⑤ 幂等性): 文件里出现**第二个**
    /// `<divisions>` 时，采用的是**第一次**出现的那个（模块文档的白名单写明了
    /// "只认第一次出现的 `divisions`"）。
    ///
    /// 补的是哪个缺口（本票注入实测）：把 `divisions_seen` 那道门去掉
    /// （注入 X5，`if !self.divisions_seen` → `if true`）后，**118** 条判据全绿
    /// ⇒ "第一次赢"当时没有判据；已提交夹具每个只有**一个** `<divisions>`
    /// （量法 = 对 `tests/fixtures/*.musicxml` 逐文件数 `<divisions>` 的出现次数）。
    #[test]
    fn two_divisions_elements_keep_the_first() {
        let xml = "<score-partwise><part id=\"P1\"><measure>\
             <attributes><divisions>4</divisions></attributes>\
             <note><pitch><step>C</step><octave>4</octave></pitch><duration>4</duration></note>\
             <attributes><divisions>8</divisions></attributes>\
             <note><pitch><step>D</step><octave>4</octave></pitch><duration>8</duration></note>\
             </measure></part></score-partwise>";
        let parsed = score(xml);
        assert_eq!(parsed.divisions, 4, "第一个 <divisions> 赢");
        assert_eq!(
            parsed.parts[0]
                .notes
                .iter()
                .map(|note| (note.key, note.duration_ticks))
                .collect::<Vec<_>>(),
            vec![(60, 960), (62, 1920)],
            "两个音符都按 divisions=4 换算: 4 个 unit = 960, 8 个 unit = 1920              （若第二个 divisions=8 生效, 读数会是 480 与 960）"
        );
    }

    /// 判据 (类别④ 参数极值 / 取整边界): `divisions` 单位 ⇒ 960 PPQ tick 的换算是
    /// **四舍五入**，半格向上。
    ///
    /// 补的是哪个缺口（本票注入实测）：把 `ticks_from_units` 的
    /// `(numerator + denominator / 2) / denominator` 换成截断的
    /// `numerator / denominator`（注入 X2）后，**118** 条判据全绿 ⇒ 已提交语料的
    /// `divisions ∈ {1, 2, 4, 12, 960}` 全部整除 960，因此取整规则**碰不到**
    /// （`ticks_from_units(1, 7)` 截断与四舍五入都给 137）。
    ///
    /// 量什么（单位 = tick）：`divisions = 1920` 时 1 个 unit 恰好是半格 ⇒ 进到 1；
    /// `divisions = 1921` 时略小于半格 ⇒ 回到 0。
    #[test]
    fn tick_conversion_rounds_half_up_at_the_boundary() {
        assert_eq!(ticks_from_units(1, 1920), Ok(1), "恰好半格 ⇒ 向上");
        assert_eq!(ticks_from_units(1, 1921), Ok(0), "略小于半格 ⇒ 向下");
        assert_eq!(ticks_from_units(1, 1280), Ok(1), "0.75 格 ⇒ 向上");
        assert_eq!(ticks_from_units(3, 1920), Ok(2), "1.5 格 ⇒ 向上");
    }

    /// 判据 (类别④ 参数极值): 嵌套深度上限是**恰好** `MAX_DEPTH` 层（含根元素）。
    ///
    /// 补的是哪个缺口（本票注入实测）：把 `stack.len() >= MAX_DEPTH` 改成 `>`
    /// （注入 X7，等于把上限放宽一层）后，**118** 条判据全绿 ⇒ 既有的
    /// `deep_nesting_is_an_error` 只造了 `MAX_DEPTH + 8` 层 ⇒ 多一层、少一层都
    /// 还是 `DepthExceeded`，上界的**位置**没有判据。
    #[test]
    fn the_depth_limit_is_exactly_max_depth() {
        let nested = |depth: usize| {
            let mut xml = String::from("<score-partwise>");
            for _ in 0..depth {
                xml.push_str("<x>");
            }
            for _ in 0..depth {
                xml.push_str("</x>");
            }
            xml.push_str("</score-partwise>");
            xml
        };
        // 根元素占 1 层 ⇒ 再嵌 MAX_DEPTH - 1 层刚好到上限。
        let at_limit = score(&nested(MAX_DEPTH - 1));
        assert_eq!(
            at_limit.ignored_elements.get("x"),
            Some(&(MAX_DEPTH as u64 - 1)),
            "刚好 MAX_DEPTH 层必须全部被读到"
        );
        assert!(
            matches!(
                parse_musicxml(nested(MAX_DEPTH).as_bytes()),
                Err(MusicXmlError::DepthExceeded { .. })
            ),
            "第 {MAX_DEPTH} 层嵌套必须被拒绝"
        );
    }
}
