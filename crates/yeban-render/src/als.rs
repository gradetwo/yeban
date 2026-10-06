//! 实验性 Ableton Live Set (`.als`) 导出 [ARCH-FMT-002] [ROAD-M4-007]。
//!
//! 本模块只在**非默认** feature `experimental-als-export` 下存在
//! （见 `crates/yeban-render/Cargo.toml` 的 `[features]`）。AGENTS.md §2 红线 6 与
//! `scripts/guards/policy_check.py` 的 `FORBIDDEN_DEFAULT_FEATURES` 都点名了这个特性：
//! 官方默认 release 构建不得默认开启它。因此默认构建**既不编译本模块, 也不把 `flate2`
//! 链进依赖图**（`default` 保持为空）。
//!
//! ## 做什么
//!
//! 把一份 [`YebanProjectV1`] 映射成 Ableton 风格的 `LiveSet` XML（[`build_live_set_xml`]）,
//! 再用 `flate2` 封成 Gzip 字节（[`export_project`]）。**全程在内存里, 零文件系统 I/O** ——
//! 落盘与资产打包是调用方的事。
//!
//! ## 映射损失表（本模块的核心契约：**不静默丢东西**）
//!
//! [`export_project`] 与 [`build_live_set_xml`] 都**同时**返回一份 [`AlsLoss`] 列表,
//! 且每一条同时以 `<!-- yeban-loss ... -->` 注释写进 XML 本体 —— 表格与文件不会各说各话。
//! `reason` 的**前缀**是机器可读的两分法：
//!
//! | 前缀 | 含义 |
//! | :--- | :--- |
//! | [`LOSS_UNMAPPED_PREFIX`]（`未映射:`） | 该构造在产出的 XML 里**没有任何表示**（被丢掉或被近似替代） |
//! | [`LOSS_NOT_EQUIVALENT_PREFIX`]（`非等价:`） | 该构造**有**表示, 但等价性**未经证实**（元素词汇表 / 参数曲线未对账） |
//!
//! `bounced_to_audio` 的含义被精确限定为：**该构造唯一可能的等价映射是"音频冻结
//! (Audio Freeze) 烘焙成分轨音频"**（`[ARCH-FMT-002]` 表里"夜半原生内建合成器"那一格）。
//!
//! ⚠ **本切片不渲染那份音频**。渲染需要 `yeban-engine` 的样本源, 而本 crate **刻意不依赖
//! `yeban-engine`**（见 `crate::lib` 模块头）。因此对不可映射的设备, 这里只在 XML 里写一条
//! 标记并把该条记进损失表。⇒ `bounced_to_audio: true` 读作"**兜底路径是音频冻结**",
//! **不**读作"音频已经烘好了"。
//!
//! ## 映射规则（逐条可复核）
//!
//! | 夜半实体 | 本模块的落点 | 等价性 |
//! | :--- | :--- | :--- |
//! | `TrackKind::Midi` | `<MidiTrack>` | 轨道容器 |
//! | `TrackKind::Audio` | `<AudioTrack>` | 轨道容器 |
//! | `TrackKind::AuxReturn` | `<ReturnTrack>` | 轨道容器 |
//! | `TrackKind::Master` | `<MasterTrack>`（`<Tracks>` 之外的兄弟） | 轨道容器 |
//! | `TrackV3::name` | `<Name><EffectiveName/><UserName/></Name>` | 无损 |
//! | `TrackV3::volume_db` | `<MixerDevice><Volume><Manual/></Volume>` = `db_to_linear(..)` | **非等价**（Live 的归一化曲线不在仓库内, 见损失表） |
//! | `TrackV3::pan` | `<MixerDevice><Pan><Manual/></Pan>` | 无损（两侧都是 -1.0..=1.0） |
//! | `TrackV3::mute` | `<MixerDevice><Speaker><Manual/></Speaker>`（`!mute` = 轨道激活） | 无损 |
//! | `TrackV3::solo` | — | 有损（`未映射:`）: Live 的 Solo 元素形状未对账, 不发明 |
//! | `TrackV3::solo_safe` | — | 有损（`未映射:`） |
//! | `TrackV3::color` / `folder_id` | — | 有损（`未映射:`: Live 的颜色整数编码不在仓库内） |
//! | `TrackV3::devices` | `<DeviceChain>` 里只有标记 | **音频冻结兜底**（`bounced_to_audio: true`） |
//! | `TrackV3::macros` / `automation_lanes` | — | 有损（`未映射:`） |
//! | 摆放（`ClipPlacement`） | `<ArrangerAutomation><Events>` 下的 `<MidiClip>` / `<AudioClip>` | 见下 |
//! | 摆放起止 | `Time` / `Duration`（tick ÷ 960 = 拍） | 无损（960 PPQ 整数 → 拍, 见 [`TICKS_PER_BEAT`]） |
//! | `ClipContent::Midi` 的音符 | `<Notes><KeyTracks><KeyTrack MidiKey><MidiNoteEvent>` | Time/Duration/Velocity 无损 |
//! | 音符的 `probability` / `ratchet` / `micro_timing_ticks` / `slide` / `pitch_bend_curve` / `syllable` / `phonemes` | — | 有损（`未映射:`） |
//! | `ClipContent::Audio` | `<AudioClip>` + `<FileRef><RelativePath>` = `assets/{sha256}` | 有损: **资产不打包**（`未映射:`） |
//! | 音频片段 `gain_db` | — | 有损（`未映射:`） |
//! | `SectionV3` | `<Locators><Locator>`（名 + 起点） | `end_tick` 有损（`未映射:`） |
//! | `SceneV3` | `<Scenes><Scene>` | 名/速度无损; 颜色有损 |
//! | `RoutingGraph` 直达主总线且无增益的边 | 隐式的直线立体声输出 | 无损 |
//! | 发送 / 侧链 / 带增益 / 非直达的边 | — | 有损（`未映射:`） |
//! | `AudioConfig` / `Metadata` / `TransportConfig` / `assets` 索引 | — | 有损（`未映射:`） |
//!
//! ## **未验证**的部分（这是本特性的诚实边界）
//!
//! 仓库里**没有任何参考 `.als`**（`find . -name '*.als'` 命中 0）, 也没有 Ableton 的 XML
//! schema。本模块用到的元素名/属性名是公开 `.als` 形态的**最小重构**, 只覆盖
//! `[ARCH-FMT-002]` 映射表点名的词汇（`MidiTrack` / `Clip` / `Notes`、`MixerDevice` /
//! `Volume` / `Pan` / `Speaker`、`AudioTrack` / `AudioClip`）, 外加承载它们的容器元素
//! （`Ableton` / `LiveSet` / `Tracks` / `Scenes` / `Locators`）。因此：
//!
//! 1. **不声称**产物可被 Live 11/12 直接打开。这条"未验证"本身作为一条 `非等价:` 损失
//!    记进每一次导出的损失表（见 [`ELEMENT_VOCABULARY_CAVEAT`]）。
//! 2. 每一处"有表示但没对账"的映射都有对应的 `非等价:` 条目; 每一处丢弃都有 `未映射:` 条目。
//! 3. 判据（本模块的 `tests`）只断言**结构**（Gzip 魔数 / XML 良构 / 标签平衡 / 标签计数 /
//!    音轨名 / 损失表覆盖）—— 不断言"Live 能打开", 因为那需要一份参考 `.als`。
//!
//! ## 确定性
//!
//! 输出必须逐字节可复现（与 `[ARCH-DET-001/002]` 同一条纪律）：
//!
//! - 集合一律按 `BTreeMap` 键序迭代, **不出现 `HashMap`**（红线 4）;
//! - Gzip 头 `mtime` 显式钉为 `0`、OS 字节由 `flate2` 固定为 255, 因此**不含时间戳**;
//! - 压缩级别固定为 [`Compression::default`]。
//!
//! 判据 `filled_project_export_is_byte_deterministic` 钉住这一点。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use flate2::write::GzEncoder;
use flate2::{Compression, GzBuilder};
use yeban_model::{
    ClipContent, ClipPlacement, EntityId, MidiNote, PPQ, RoutingKind, TrackKind, TrackV3,
    YebanProjectV1,
};

/// Gzip 成员头的两个魔数字节（RFC 1952 的 `ID1` / `ID2`）。
pub const ALS_GZIP_MAGIC: [u8; 2] = [0x1F, 0x8B];

/// `.als` 文档的 XML 声明（文档一律 UTF-8）。
pub const ALS_XML_DECLARATION: &str = r#"<?xml version="1.0" encoding="UTF-8"?>"#;

/// 写进 `<Ableton>` 根元素的 `MajorVersion`。
///
/// ⚠ 这是公开 `.als` 形态的取值, **未经参考文件对账**（见模块头的"未验证"一节）。
pub const ABLETON_MAJOR_VERSION: u8 = 5;

/// 写进 `<Ableton>` 根元素的 `MinorVersion`（目标子集 = Live 11/12）。
pub const ABLETON_MINOR_VERSION: &str = "11.0";

/// 写进 `<Ableton>` 根元素的 `SchemaChangeCount`。
pub const ABLETON_SCHEMA_CHANGE_COUNT: u8 = 3;

/// 一拍等于多少 tick（= `yeban_model::PPQ` 的 960）。
///
/// 夜半的整数时钟是 960 PPQ（`[MODEL-AST-001]`）, `.als` 的 `Time` / `Duration` 以**拍**
/// 为单位。整数 → 拍的换算是精确的（960 的约数覆盖三连音与 128 分音符）。
pub const TICKS_PER_BEAT: u64 = PPQ;

/// `未映射:` —— 该构造在产出的 XML 里没有任何表示（被丢掉或被近似替代）。
pub const LOSS_UNMAPPED_PREFIX: &str = "未映射:";

/// `非等价:` —— 该构造有表示, 但等价性未经证实（词汇表 / 参数曲线未对账）。
pub const LOSS_NOT_EQUIVALENT_PREFIX: &str = "非等价:";

/// 每一次导出都会记录的那条"元素词汇表未对账"警告的 `reason` 文本（`[ARCH-FMT-002]`）。
///
/// 它是**保真度警告**而不是实体丢弃：`entity` 是 `project:<ulid>`。
pub const ELEMENT_VOCABULARY_CAVEAT: &str = concat!(
    "非等价: 元素词汇表 —— 本模块的 .als 元素名/属性名是公开 .als 形态的最小重构",
    "（`find . -name '*.als'` 在本仓库命中 0, 无参考文件可对账）, ",
    "因此不声称产物可被 Live 11/12 直接打开；",
    "涉及的元素族：Tracks/MidiTrack/AudioTrack/ReturnTrack/MasterTrack、",
    "ArrangerAutomation/Events/MidiClip/AudioClip、Notes/KeyTracks/KeyTrack/MidiNoteEvent、",
    "MixerDevice/Volume/Pan/Speaker、Scenes/Scene、Locators/Locator"
);

/// 一条"无法等价映射"的记录。
///
/// 这是 `[ARCH-FMT-002]`"映射损失对照表"的机器可读形态：每一条被降级的构造都必须在这里
/// 留痕, 并且 [`AlsExport::losses`] 与 [`AlsDocument::losses`] 总是与 XML 一起返回。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlsLoss {
    /// 被降级的夜半实体的**稳定文本寻址**（与 XML 里的 `yeban-loss` 注释逐字一致）。
    ///
    /// | 前缀 | 指向 |
    /// | :--- | :--- |
    /// | `project:` | 工程级构造（音频配置 / 元数据 / 传输 / 词汇表保真度 …） |
    /// | `track:` | 一条音轨 |
    /// | `device:` | 一个设备定义 |
    /// | `macro:` | 一个宏 |
    /// | `automation:` | 一条自动化泳道 |
    /// | `clip:` | 一个片段池条目（含其音符的表现属性） |
    /// | `clip-placement:` | 一次时间轴摆放 |
    /// | `routing-edge:` | 一条路由边 |
    /// | `asset:` | 一条 CAS 资产索引条目 |
    /// | `section:` / `scene:` | 一个段落 / 一个场景 |
    pub entity: String,
    /// 人话解释。**前缀**是机器可读的两分法：`未映射:` 或 `非等价:`（见模块头）。
    pub reason: String,
    /// `true` = 该构造唯一可能的等价映射是"音频冻结 (Audio Freeze)"。
    ///
    /// ⚠ 本切片只写标记, **不烘音频**（没有 `yeban-engine` 样本源）。见模块头。
    pub bounced_to_audio: bool,
}

/// 导出的产物：Gzip 字节 + 损失表 + 映射计数。
///
/// `bytes` 是完整的 `.als` 文件内容（Gzip 压缩的 XML）。**本模块不落盘**。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlsExport {
    /// Gzip 压缩后的 XML —— 即 `.als` 文件字节。
    pub bytes: Vec<u8>,
    /// 映射损失表。对任何含未映射构造的工程都非空（且从不静默为空）。
    pub losses: Vec<AlsLoss>,
    /// 写进 XML 的音轨条数（含 Master）。
    pub mapped_tracks: usize,
    /// 写进 XML 的片段条数（= 成功映射的摆放数）。
    pub mapped_clips: usize,
    /// 写进 XML 的音符事件数。
    pub mapped_notes: usize,
}

/// 未压缩的中间产物。
///
/// 存在两个理由：① 判据可以在不 gunzip 的情况下检查 XML; ② 将来给 CLI/控制面接线时,
/// "生成 XML"与"压缩"是两件可以各自验证的事。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlsDocument {
    /// 完整的 `LiveSet` XML 文本。
    pub xml: String,
    /// 映射损失表（与 [`AlsExport::losses`] 同一份实现产物）。
    pub losses: Vec<AlsLoss>,
    /// 同 [`AlsExport::mapped_tracks`]。
    pub mapped_tracks: usize,
    /// 同 [`AlsExport::mapped_clips`]。
    pub mapped_clips: usize,
    /// 同 [`AlsExport::mapped_notes`]。
    pub mapped_notes: usize,
}

/// 导出失败的原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AlsExportError {
    /// Gzip 封装失败（写入内存缓冲对内核无 I/O, 只可能来自压缩器自身）。
    Gzip(String),
}

impl fmt::Display for AlsExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gzip(message) => write!(f, "Gzip 封装失败: {message}"),
        }
    }
}

impl std::error::Error for AlsExportError {}

/// 把工程导出为 `.als` 字节, 并**同时**返回映射损失表。
///
/// # Errors
///
/// Gzip 封装失败 → [`AlsExportError::Gzip`]。
pub fn export_project(project: &YebanProjectV1) -> Result<AlsExport, AlsExportError> {
    let document = build_live_set_xml(project);
    let bytes = gzip(document.xml.as_bytes())?;
    Ok(AlsExport {
        bytes,
        losses: document.losses,
        mapped_tracks: document.mapped_tracks,
        mapped_clips: document.mapped_clips,
        mapped_notes: document.mapped_notes,
    })
}

/// 把工程映射成 `LiveSet` XML（未压缩）与映射损失表。
///
/// 纯函数、无 I/O、确定性（同一输入 ⇒ 逐字节相同的 `xml` 与 `losses`）。
#[must_use]
pub fn build_live_set_xml(project: &YebanProjectV1) -> AlsDocument {
    let mut builder = AlsBuilder::default();

    builder.out.push_str(ALS_XML_DECLARATION);
    builder.out.push('\n');
    builder.comment(&format!(
        "yeban-project id=\"{}\" writer=\"{}\" schema-version=\"{}\" feature=\"experimental-als-export\"",
        project.id.to_canonical_string(),
        project.writer_version,
        project.schema_version
    ));

    let opening = format!(
        r#"<Ableton MajorVersion="{ABLETON_MAJOR_VERSION}" MinorVersion="{ABLETON_MINOR_VERSION}" SchemaChangeCount="{ABLETON_SCHEMA_CHANGE_COUNT}" Creator="Yeban" Revision="1">"#
    );
    builder.open_with(&opening);
    builder.open("LiveSet");

    let title = project.title.clone();
    builder.leaf("Name", &title);
    let tempo = num(project.bpm);
    builder.leaf("Tempo", &tempo);
    let numerator = project.time_signature.numerator.to_string();
    let denominator = project.time_signature.denominator.to_string();
    builder.event(
        "TimeSignature",
        &[("Numerator", &numerator), ("Denominator", &denominator)],
    );

    builder.write_project_losses(project);
    builder.write_scenes(project);
    builder.write_locators(project);

    builder.open("Tracks");
    for (track_id, track) in &project.tracks {
        if track.kind != TrackKind::Master {
            builder.write_channel_track(project, *track_id, track);
        }
    }
    builder.close("Tracks");
    for (track_id, track) in &project.tracks {
        if track.kind == TrackKind::Master {
            builder.write_channel_track(project, *track_id, track);
        }
    }

    builder.close("LiveSet");
    builder.close("Ableton");

    AlsDocument {
        xml: builder.out,
        losses: builder.losses,
        mapped_tracks: builder.mapped_tracks,
        mapped_clips: builder.mapped_clips,
        mapped_notes: builder.mapped_notes,
    }
}

/// Gzip 封装。头部 `mtime` 显式钉为 `0`（不写入时间戳 ⇒ 可复现）。
fn gzip(xml: &[u8]) -> Result<Vec<u8>, AlsExportError> {
    use std::io::Write as _;

    let mut encoder: GzEncoder<Vec<u8>> = GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), Compression::default());
    encoder
        .write_all(xml)
        .map_err(|error| AlsExportError::Gzip(error.to_string()))?;
    encoder
        .finish()
        .map_err(|error| AlsExportError::Gzip(error.to_string()))
}

/// tick → 拍的文本（960 tick = 1 拍）。
fn beats(ticks: u64) -> String {
    num(ticks as f64 / TICKS_PER_BEAT as f64)
}

/// `f64` → 最短往返文本（不引入任何本地化/分组）。
fn num(value: f64) -> String {
    format!("{value}")
}

/// XML 属性值转义（双引号包裹）。
fn escape_attr(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
    out
}

/// XML 注释体：只禁 `--`, 且不得以 `-` 结尾（我们总是补一个空格再接 `-->`）。
fn comment_body(value: &str) -> String {
    value.replace("--", "- -")
}

/// 逐行拼 XML 的小工具（缩进 + 损失表 + 身份编号）。
#[derive(Default)]
struct AlsBuilder {
    out: String,
    losses: Vec<AlsLoss>,
    depth: usize,
    next_id: u32,
    mapped_tracks: usize,
    mapped_clips: usize,
    mapped_notes: usize,
}

impl AlsBuilder {
    /// 下一个 XML 身份编号（`.als` 的 `Id` 属性要求工程内唯一）。
    fn next_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        id
    }

    fn line(&mut self, text: &str) {
        for _ in 0..self.depth {
            self.out.push_str("  ");
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    /// 打开一个**无属性**的元素：`open("Tracks")` ⇒ `<Tracks>`。
    fn open(&mut self, tag: &str) {
        let text = format!("<{tag}>");
        self.line(&text);
        self.depth = self.depth.saturating_add(1);
    }

    /// 打开一个**带属性**的元素（开标签文本由调用方拼好，必须自带尖括号）。
    ///
    /// ⚠ 与 [`Self::open`] 分开而不是"猜字符串里有没有 `<`"：这种猜法一旦写错就是
    /// **XML 直接畸形**（本模块第一版就是这么错的 —— 判据 `check_xml` 当场抓到）。
    fn open_with(&mut self, opening: &str) {
        debug_assert!(opening.starts_with('<') && opening.ends_with('>'));
        self.line(opening);
        self.depth = self.depth.saturating_add(1);
    }

    fn close(&mut self, tag: &str) {
        self.depth = self.depth.saturating_sub(1);
        let text = format!("</{tag}>");
        self.line(&text);
    }

    /// 自闭合元素 + 属性表。
    fn event(&mut self, tag: &str, attrs: &[(&str, &str)]) {
        let mut text = format!("<{tag}");
        for &(name, value) in attrs {
            let escaped = escape_attr(value);
            text.push_str(&format!(" {name}=\"{escaped}\""));
        }
        text.push_str("/>");
        self.line(&text);
    }

    /// `<tag Value="..."/>` 形状的叶子元素。
    fn leaf(&mut self, tag: &str, value: &str) {
        self.event(tag, &[("Value", value)]);
    }

    fn name_block(&mut self, name: &str) {
        self.open("Name");
        self.leaf("EffectiveName", name);
        self.leaf("UserName", name);
        self.close("Name");
    }

    /// `<!-- ... -->`（注释体里的 `--` 会被拆开, 保证良构）。
    fn comment(&mut self, text: &str) {
        let body = comment_body(text);
        let line = format!("<!-- {body} -->");
        self.line(&line);
    }

    /// **记录一条损失**: 同时进损失表, 并在当前位置写一条 `yeban-loss` 注释。
    ///
    /// 一个调用点同时喂养"返回给调用方的表"与"文件本体", 因此两者不可能漂移。
    fn loss(&mut self, entity: String, reason: String, bounced_to_audio: bool) {
        let bounced = if bounced_to_audio { "true" } else { "false" };
        let entity_attr = escape_attr(&entity);
        let reason_attr = escape_attr(&reason);
        self.comment(&format!(
            "yeban-loss entity=\"{entity_attr}\" bounced-to-audio=\"{bounced}\" reason=\"{reason_attr}\""
        ));
        self.losses.push(AlsLoss {
            entity,
            reason,
            bounced_to_audio,
        });
    }

    fn project_entity(project: &YebanProjectV1) -> String {
        format!("project:{}", project.id.to_canonical_string())
    }

    /// 工程级降级项（顺序固定 ⇒ 损失表顺序确定）。
    fn write_project_losses(&mut self, project: &YebanProjectV1) {
        let entity = Self::project_entity(project);

        // ① 通道条音量曲线（只在真的搬运了非零增益时才警告）。
        if project.tracks.values().any(|track| track.volume_db != 0.0) {
            self.loss(
                entity.clone(),
                format!(
                    "{LOSS_NOT_EQUIVALENT_PREFIX} 通道条音量 —— Live 的 Volume/Manual 归一化曲线\
                     不在仓库内（无参考 .als 可对账）, 本切片写入线性幅度 `10^(dB/20)`\
                     （`crate::render::db_to_linear` 的结果）, 与 Live 的取值口径未对账"
                ),
                false,
            );
        }

        // ② 元素词汇表（每一次导出都记, 因为它对每一次导出的每一行都成立）。
        self.loss(entity.clone(), ELEMENT_VOCABULARY_CAVEAT.to_owned(), false);

        // ③ 工程音频配置。
        let config = &project.audio_config;
        self.loss(
            entity.clone(),
            format!(
                "{LOSS_UNMAPPED_PREFIX} 工程音频配置（sample_rate={:?} / block_size={:?} / \
                 bit_depth={:?} / pan_law={:?}）不写入本子集",
                config.sample_rate, config.block_size, config.bit_depth, config.pan_law
            ),
            false,
        );

        // ④ 工程元数据与署名。
        self.loss(
            entity.clone(),
            format!(
                "{LOSS_UNMAPPED_PREFIX} 工程元数据（description / created_at_unix_ms={} / \
                 modified_at_unix_ms={} / {} 个标签）与 author 不写入本子集",
                project.metadata.created_at_unix_ms,
                project.metadata.modified_at_unix_ms,
                project.metadata.tags.len()
            ),
            false,
        );

        // ⑤ 传输配置。
        self.loss(
            entity.clone(),
            format!(
                "{LOSS_UNMAPPED_PREFIX} 传输配置（metronome_enabled={} / count_in_bars={} / \
                 launch_quantization={:?}）不写入本子集",
                project.transport.metronome_enabled,
                project.transport.count_in_bars,
                project.transport.launch_quantization
            ),
            false,
        );

        // ⑥ 段落只有起点能进 Locator。
        if !project.sections.is_empty() {
            self.loss(
                entity.clone(),
                format!(
                    "{LOSS_UNMAPPED_PREFIX} 段落区间终点 —— Locator 只有起点, {} 个段落的 \
                     `end_tick` 未导出（下一段落的起点即上一段的终点）",
                    project.sections.len()
                ),
                false,
            );
        }

        // ⑦ `rng_seed` 是 `probability` 触发判定的跨机一致性依据, Live 侧无对应物。
        if project_uses_note_probability(project) {
            self.loss(
                entity.clone(),
                format!(
                    "{LOSS_UNMAPPED_PREFIX} 工程 rng_seed={} —— 它决定 probability 的触发判定\
                     （[MODEL-AST-005]）, Live 侧无对应物, 未写入本子集",
                    project.rng_seed
                ),
                false,
            );
        }

        // ⑧ 路由：只有"直达主总线且无增益"的边才等价于直线立体声输出。
        for edge in project.routing_graph.edges.values() {
            let direct = edge.gain_db.is_none()
                && edge.destination_node == project.master_bus_track_id
                && matches!(
                    edge.kind,
                    RoutingKind::TrackToBus | RoutingKind::BusToMaster
                );
            if direct {
                continue;
            }
            let source = edge.source_node.to_canonical_string();
            let destination = edge.destination_node.to_canonical_string();
            let reason = match edge.kind {
                RoutingKind::SendToAux => format!(
                    "{LOSS_UNMAPPED_PREFIX} 发送→辅助返回（{source} → {destination}, \
                     发送增益 {:?} dB）：本切片只映射直达主总线的立体声输出",
                    edge.gain_db
                ),
                RoutingKind::Sidechain => format!(
                    "{LOSS_UNMAPPED_PREFIX} 侧链边（{source} → {destination}）无法在 Live 子集里\
                     表达, 降级为直线立体声输出"
                ),
                RoutingKind::TrackToBus | RoutingKind::BusToMaster => format!(
                    "{LOSS_UNMAPPED_PREFIX} 非直线路由边（{source} → {destination}, \
                     类型 {:?}, 增益 {:?} dB）：本切片只映射直达主总线的无增益边",
                    edge.kind, edge.gain_db
                ),
            };
            self.loss(
                format!("routing-edge:{}", edge.id.to_canonical_string()),
                reason,
                false,
            );
        }

        // ⑨ CAS 资产：本切片不打包字节。
        for (hash, asset) in &project.assets {
            self.loss(
                format!("asset:{}", hash.as_str()),
                format!(
                    "{LOSS_UNMAPPED_PREFIX} CAS 资产未打包到导出目录 —— 资产字节不在工程文档里, \
                     片段只写相对路径 `assets/{}`（原路径 {}, {} 字节, 许可 {}）",
                    hash.as_str(),
                    asset.original_path,
                    asset.byte_len,
                    asset.license
                ),
                false,
            );
        }

        // ⑩ 片段池里没有被任何摆放引用的条目（否则它们会被静默丢掉）。
        let referenced: BTreeSet<EntityId> = project
            .tracks
            .values()
            .flat_map(|track| track.clips.values().map(|placement| placement.clip_id))
            .collect();
        for entry in project.clip_pool.values() {
            if referenced.contains(&entry.id) {
                continue;
            }
            self.loss(
                format!("clip:{}#{}", entry.name, entry.id.to_canonical_string()),
                format!(
                    "{LOSS_UNMAPPED_PREFIX} 片段池条目没有任何时间轴摆放引用它（池里 {}/{} 条\
                     被引用）⇒ 不导出",
                    referenced.len(),
                    project.clip_pool.len()
                ),
                false,
            );
        }
    }

    fn write_scenes(&mut self, project: &YebanProjectV1) {
        self.open("Scenes");
        for scene in project.scenes.values() {
            let scene_id = self.next_id();
            let opening = format!(r#"<Scene Id="{scene_id}">"#);
            self.open_with(&opening);
            self.name_block(&scene.name);
            let tempo = scene.tempo.unwrap_or(project.bpm);
            let tempo_text = num(tempo);
            self.leaf("Tempo", &tempo_text);
            if let Some(color) = &scene.color {
                self.loss(
                    format!("scene:{}#{}", scene.name, scene.id.to_canonical_string()),
                    format!(
                        "{LOSS_UNMAPPED_PREFIX} 场景色标 {color} 未导出（Live 的颜色整数编码\
                         不在仓库内）"
                    ),
                    false,
                );
            }
            if let Some(scene_tempo) = scene.tempo
                && (scene_tempo - project.bpm).abs() > f64::EPSILON
            {
                self.loss(
                    format!("scene:{}#{}", scene.name, scene.id.to_canonical_string()),
                    format!(
                        "{LOSS_NOT_EQUIVALENT_PREFIX} 场景独立速度 {scene_tempo}（工程 bpm 是 \
                         {}）写进了 `<Scene><Tempo>`, 但该元素的形状未对账",
                        project.bpm
                    ),
                    false,
                );
            }
            self.close("Scene");
        }
        self.close("Scenes");
    }

    fn write_locators(&mut self, project: &YebanProjectV1) {
        self.open("Locators");
        for section in project.sections.values() {
            let locator_id = self.next_id();
            let opening = format!(r#"<Locator Id="{locator_id}">"#);
            self.open_with(&opening);
            self.name_block(&section.name);
            let time = beats(section.start_tick);
            self.leaf("Time", &time);
            if let Some(color) = &section.color {
                self.loss(
                    format!(
                        "section:{}#{}",
                        section.name,
                        section.id.to_canonical_string()
                    ),
                    format!(
                        "{LOSS_UNMAPPED_PREFIX} 段落色标 {color} 未导出（Live 的颜色整数编码\
                         不在仓库内）"
                    ),
                    false,
                );
            }
            self.close("Locator");
        }
        self.close("Locators");
    }

    fn write_channel_track(
        &mut self,
        project: &YebanProjectV1,
        track_id: EntityId,
        track: &TrackV3,
    ) {
        let tag = match track.kind {
            TrackKind::Midi => "MidiTrack",
            TrackKind::Audio => "AudioTrack",
            TrackKind::AuxReturn => "ReturnTrack",
            TrackKind::Master => "MasterTrack",
        };
        self.mapped_tracks += 1;
        let xml_id = self.next_id();
        let opening = format!(r#"<{tag} Id="{xml_id}">"#);
        self.open_with(&opening);
        let track_entity = format!("track:{}#{}", track.name, track_id.to_canonical_string());
        self.name_block(&track.name);
        self.write_mixer(track);
        self.write_track_losses(&track_entity, track);
        self.write_device_chain(&track_entity, track);
        self.write_clips(project, track);
        self.close(tag);
    }

    fn write_mixer(&mut self, track: &TrackV3) {
        self.open("MixerDevice");
        self.open("Volume");
        let gain = crate::render::db_to_linear(track.volume_db).to_string();
        self.leaf("Manual", &gain);
        self.close("Volume");
        self.open("Pan");
        let pan = num(f64::from(track.pan));
        self.leaf("Manual", &pan);
        self.close("Pan");
        self.open("Speaker");
        let active = if track.mute { "false" } else { "true" };
        self.leaf("Manual", active);
        self.close("Speaker");
        self.close("MixerDevice");
    }

    fn write_track_losses(&mut self, track_entity: &str, track: &TrackV3) {
        if let Some(folder) = track.folder_id {
            self.loss(
                track_entity.to_owned(),
                format!(
                    "{LOSS_UNMAPPED_PREFIX} 折叠文件夹归属 folder_id={}（界面语义, Live 侧无对应物）",
                    folder.to_canonical_string()
                ),
                false,
            );
        }
        if let Some(color) = &track.color {
            self.loss(
                track_entity.to_owned(),
                format!(
                    "{LOSS_UNMAPPED_PREFIX} 音轨色标 {color} 未导出（Live 的颜色整数编码\
                     不在仓库内）"
                ),
                false,
            );
        }
        if track.solo {
            self.loss(
                track_entity.to_owned(),
                format!(
                    "{LOSS_UNMAPPED_PREFIX} 独奏标志 solo=true 未导出 —— Live 的 Solo 元素形状\
                     未在仓库内核实, 本切片不凭空发明它"
                ),
                false,
            );
        }
        if track.solo_safe {
            self.loss(
                track_entity.to_owned(),
                format!(
                    "{LOSS_UNMAPPED_PREFIX} 独奏安全 solo_safe=true 未导出（Live 子集里没有\
                     对应开关）"
                ),
                false,
            );
        }
        for (target, lane) in &track.automation_lanes {
            let label = automation_target_label(target);
            self.loss(
                format!("automation:{label}#{track_entity}"),
                format!(
                    "{LOSS_UNMAPPED_PREFIX} 自动化泳道 {label}（{} 个点, 读开关={}, \
                     写模式={:?}）未导出 —— 本切片不写 Live 的自动化包络",
                    lane.points.len(),
                    lane.read_enabled,
                    lane.write_mode
                ),
                false,
            );
        }
    }

    fn write_device_chain(&mut self, track_entity: &str, track: &TrackV3) {
        if track.devices.is_empty() && track.macros.is_empty() {
            return;
        }
        self.open("DeviceChain");
        for device in &track.devices {
            let device_entity =
                format!("device:{}#{}", device.name, device.id.to_canonical_string());
            self.loss(
                device_entity.clone(),
                format!(
                    "{LOSS_UNMAPPED_PREFIX} 设备 `{}`（{:?}）没有 Live 原生等价物 ⇒ 兜底为\
                     音频冻结 (Audio Freeze)。含 {} 个参数, bypassed={}；**本切片只写标记,\
                     未渲染音频**（渲染需要 yeban-engine 样本源, 本 crate 刻意不依赖它）",
                    device.name,
                    device.kind,
                    device.params.len(),
                    device.bypassed
                ),
                true,
            );
            if device.latency_samples != 0 {
                self.loss(
                    device_entity,
                    format!(
                        "{LOSS_UNMAPPED_PREFIX} ARCH-PDC-001 设备延迟 {} 采样点未导出\
                         （Live 子集无设备延迟申报位）",
                        device.latency_samples
                    ),
                    false,
                );
            }
        }
        for macro_parameter in &track.macros {
            self.loss(
                format!("macro:{}#{track_entity}", macro_parameter.name),
                format!(
                    "{LOSS_UNMAPPED_PREFIX} 宏 `{}`（值 {}, {} 条映射）未导出 —— Live 的 \
                     Macro/Rack 映射不在本子集内",
                    macro_parameter.name,
                    macro_parameter.value,
                    macro_parameter.mappings.len()
                ),
                false,
            );
        }
        self.close("DeviceChain");
    }

    fn write_clips(&mut self, project: &YebanProjectV1, track: &TrackV3) {
        self.open("ArrangerAutomation");
        self.open("Events");
        for placement in track.clips.values() {
            self.write_placement(project, placement);
        }
        self.close("Events");
        self.close("ArrangerAutomation");
    }

    fn write_placement(&mut self, project: &YebanProjectV1, placement: &ClipPlacement) {
        let placement_entity = format!("clip-placement:{}", placement.id.to_canonical_string());
        let Some(entry) = project.clip_pool.get(&placement.clip_id) else {
            self.loss(
                placement_entity,
                format!(
                    "{LOSS_UNMAPPED_PREFIX} 摆放指向的片段池条目 {} 不存在（悬挂引用）⇒ 该摆放\
                     不导出",
                    placement.clip_id.to_canonical_string()
                ),
                false,
            );
            return;
        };

        let start = beats(placement.start_tick);
        let duration = beats(placement.duration_ticks);
        let clip_entity = format!("clip:{}#{}", entry.name, entry.id.to_canonical_string());
        let xml_id = self.next_id();

        match &entry.content {
            ClipContent::Midi { notes } => {
                self.mapped_clips += 1;
                let opening =
                    format!(r#"<MidiClip Id="{xml_id}" Time="{start}" Duration="{duration}">"#);
                self.open_with(&opening);
                self.name_block(&entry.name);
                self.write_notes(&clip_entity, notes);
                self.close("MidiClip");
            }
            ClipContent::Audio { asset, gain_db } => {
                self.mapped_clips += 1;
                let opening =
                    format!(r#"<AudioClip Id="{xml_id}" Time="{start}" Duration="{duration}">"#);
                self.open_with(&opening);
                self.name_block(&entry.name);
                self.open("FileRef");
                let relative = format!("assets/{}", asset.as_str());
                self.leaf("RelativePath", &relative);
                self.close("FileRef");
                self.loss(
                    clip_entity,
                    format!(
                        "{LOSS_UNMAPPED_PREFIX} 音频片段增益 {gain_db} dB 未导出 —— Live 的\
                         片段增益元素形状未在仓库内核实, 本切片不凭空发明它"
                    ),
                    false,
                );
                self.close("AudioClip");
            }
        }

        if placement.muted {
            self.loss(
                placement_entity.clone(),
                format!(
                    "{LOSS_UNMAPPED_PREFIX} 静音摆放标记 muted=true 未导出（Live 的片段禁用\
                     元素形状未在仓库内核实）"
                ),
                false,
            );
        }
        let loop_config = placement.loop_config;
        let loop_covers_placement =
            loop_config.start_tick == 0 && loop_config.end_tick == placement.duration_ticks;
        if loop_config.enabled && !loop_covers_placement {
            self.loss(
                placement_entity,
                format!(
                    "{LOSS_UNMAPPED_PREFIX} 循环区间 {}..{} tick 与摆放跨度 0..{} tick 不同 ⇒ \
                     Live 的 Loop 元素不在本切片子集内, 重复播放会丢",
                    loop_config.start_tick, loop_config.end_tick, placement.duration_ticks
                ),
                false,
            );
        }
    }

    fn write_notes(&mut self, clip_entity: &str, notes: &BTreeMap<EntityId, MidiNote>) {
        let mut by_pitch: BTreeMap<u8, Vec<&MidiNote>> = BTreeMap::new();
        for note in notes.values() {
            by_pitch.entry(note.pitch).or_default().push(note);
        }

        self.open("Notes");
        self.open("KeyTracks");
        for (pitch, mut events) in by_pitch {
            events.sort_by(|left, right| {
                left.start_tick
                    .cmp(&right.start_tick)
                    .then_with(|| left.id.cmp(&right.id))
            });
            let opening = format!(r#"<KeyTrack MidiKey="{pitch}">"#);
            self.open_with(&opening);
            for note in events {
                self.mapped_notes += 1;
                let time = beats(note.start_tick);
                let duration = beats(note.duration_ticks);
                let velocity = note.velocity.to_string();
                self.event(
                    "MidiNoteEvent",
                    &[
                        ("Time", &time),
                        ("Duration", &duration),
                        ("Velocity", &velocity),
                    ],
                );
            }
            self.close("KeyTrack");
        }
        self.close("KeyTracks");
        self.close("Notes");

        let attributes = unsupported_note_attributes(notes);
        if !attributes.is_empty() {
            let affected = notes
                .values()
                .filter(|note| has_unsupported_note_attributes(note))
                .count();
            self.loss(
                clip_entity.to_owned(),
                format!(
                    "{LOSS_UNMAPPED_PREFIX} {affected}/{} 个音符携带 Live 子集无法表达的\
                     表现属性（{}）—— 只导出 Time/Duration/Velocity",
                    notes.len(),
                    attributes.join(", ")
                ),
                false,
            );
        }
    }
}

/// 自动化目标的**稳定文本标签**（损失表里用它寻址, 不用 `Debug` 的 `EntityId(Ulid(..))`）。
fn automation_target_label(target: &yeban_model::AutomationTarget) -> String {
    use yeban_model::AutomationTarget as Target;

    match target {
        Target::TrackVolume { track_id } => {
            format!("TrackVolume@{}", track_id.to_canonical_string())
        }
        Target::TrackPan { track_id } => {
            format!("TrackPan@{}", track_id.to_canonical_string())
        }
        Target::SendGain { track_id, edge_id } => format!(
            "SendGain@{}#{}",
            track_id.to_canonical_string(),
            edge_id.to_canonical_string()
        ),
        Target::DeviceParam {
            track_id,
            slot_index,
            param_index,
        } => format!(
            "DeviceParam@{}:slot={slot_index}:param={param_index}",
            track_id.to_canonical_string()
        ),
        Target::Macro {
            track_id,
            macro_index,
        } => format!(
            "Macro@{}:index={macro_index}",
            track_id.to_canonical_string()
        ),
    }
}

/// 工程里是否有音符使用 `probability`（决定 `rng_seed` 是否必须进损失表）。
fn project_uses_note_probability(project: &YebanProjectV1) -> bool {
    project
        .clip_pool
        .values()
        .any(|entry| match &entry.content {
            ClipContent::Midi { notes } => notes.values().any(|note| note.probability.is_some()),
            ClipContent::Audio { .. } => false,
        })
}

/// 音符上"Live 子集表达不了"的属性名（升序、去重；空 = 全部音符都能无损表达）。
fn unsupported_note_attributes(notes: &BTreeMap<EntityId, MidiNote>) -> Vec<&'static str> {
    let mut found = Vec::new();
    for note in notes.values() {
        for name in unsupported_attributes_of(note) {
            if !found.contains(&name) {
                found.push(name);
            }
        }
    }
    found.sort_unstable();
    found
}

/// 某个音符是否携带 Live 子集表达不了的属性。
fn has_unsupported_note_attributes(note: &MidiNote) -> bool {
    !unsupported_attributes_of(note).is_empty()
}

/// 单个音符上的"表达不了"属性名。
fn unsupported_attributes_of(note: &MidiNote) -> Vec<&'static str> {
    let mut names = Vec::new();
    if note.probability.is_some() {
        names.push("probability");
    }
    if note.ratchet.is_some() {
        names.push("ratchet");
    }
    if note.micro_timing_ticks.is_some() {
        names.push("micro_timing_ticks");
    }
    if note.slide.is_some() {
        names.push("slide");
    }
    if !note.pitch_bend_curve.is_empty() {
        names.push("pitch_bend_curve");
    }
    if note.syllable.is_some() {
        names.push("syllable");
    }
    if !note.phonemes.is_empty() {
        names.push("phonemes");
    }
    names
}

#[cfg(all(test, feature = "experimental-als-export"))]
mod tests {
    use super::*;
    use yeban_model::samples::{default_project, filled_project};

    /// 内存 gunzip（判据里**不碰文件系统**）。
    fn gunzip(bytes: &[u8]) -> String {
        use std::io::Read as _;

        let mut decoder = flate2::read::GzDecoder::new(bytes);
        let mut xml = String::new();
        decoder.read_to_string(&mut xml).expect("gunzip 到内存");
        xml
    }

    /// 允许出现的实体引用（我们自己只产出前四个）。
    const ENTITIES: [&str; 5] = ["&amp;", "&lt;", "&gt;", "&quot;", "&apos;"];

    /// 手写的**结构**检查器（刻意不引入 XML 依赖, 见 `crates/yeban-render/Cargo.toml`）。
    ///
    /// 检查 ① 标签配对（栈式）、② 注释不许含 `--`、③ 注释之外的 `&` 必须是合法实体引用。
    /// 返回根元素名。它不校验属性引号/命名空间 —— 这对"是不是一棵结构正确的 XML"足够,
    /// 而"Live 能不能打开"是另一件事（见模块头）。
    fn check_xml(xml: &str) -> Result<String, String> {
        let mut stack: Vec<String> = Vec::new();
        let mut root = String::new();
        let mut index = 0usize;
        let bytes = xml.as_bytes();
        while index < bytes.len() {
            if bytes[index] != b'<' {
                index += 1;
                continue;
            }
            let rest = &xml[index..];
            if rest.starts_with("<?") {
                let end = rest.find("?>").ok_or("未闭合的 <?...?>")?;
                index += end + 2;
                continue;
            }
            if rest.starts_with("<!--") {
                let end = rest.find("-->").ok_or("未闭合的注释")?;
                let body = &rest[4..end];
                if body.contains("--") {
                    return Err(format!("注释体含 `--`: {body}"));
                }
                index += end + 3;
                continue;
            }
            if rest.starts_with("<!") {
                let end = rest.find('>').ok_or("未闭合的 <!...>")?;
                index += end + 1;
                continue;
            }
            let end = rest.find('>').ok_or("未闭合的标签")?;
            let inner = &rest[1..end];
            index += end + 1;
            if let Some(name) = inner.strip_prefix('/') {
                let name = name.trim();
                match stack.pop() {
                    Some(open) if open == name => {}
                    other => return Err(format!("闭合标签 {name} 与 {other:?} 不匹配")),
                }
            } else if let Some(name) = inner.strip_suffix('/') {
                let name = name.trim();
                if name.is_empty() {
                    return Err("空的自闭合标签".to_owned());
                }
                if stack.is_empty() && root.is_empty() {
                    root = name.to_owned();
                }
            } else {
                let name = inner.split_whitespace().next().unwrap_or_default();
                if name.is_empty() {
                    return Err("空标签名".to_owned());
                }
                if stack.is_empty() && root.is_empty() {
                    root = name.to_owned();
                }
                stack.push(name.to_owned());
            }
        }
        if !stack.is_empty() {
            return Err(format!("未闭合的标签栈: {stack:?}"));
        }
        if root.is_empty() {
            return Err("没有根元素".to_owned());
        }
        check_escaping(xml)?;
        Ok(root)
    }

    /// 注释之外的每个 `&` 都必须是合法实体引用（转义没漏）。
    fn check_escaping(xml: &str) -> Result<(), String> {
        let mut rest = xml;
        while let Some(amp) = rest.find('&') {
            if let Some(comment_start) = rest.find("<!--")
                && comment_start < amp
            {
                let after = &rest[comment_start + 4..];
                let end = after.find("-->").ok_or("未闭合的注释")?;
                rest = &after[end + 3..];
                continue;
            }
            let tail = &rest[amp..];
            let matched =
                ENTITIES.iter().any(|entity| tail.starts_with(entity)) || tail.starts_with("&#");
            if !matched {
                let snippet: String = tail.chars().take(24).collect();
                return Err(format!("未转义的 `&`: {snippet}"));
            }
            rest = &tail[1..];
        }
        Ok(())
    }

    fn losses_matching<'a>(export: &'a AlsExport, prefix: &str) -> Vec<&'a AlsLoss> {
        export
            .losses
            .iter()
            .filter(|loss| loss.entity.starts_with(prefix))
            .collect()
    }

    /// 取出某个元素名的**首个开标签**（`<Name ...>`，含 `>`）。
    ///
    /// 判据刻意不断言 XML 身份编号（`Id`）—— 编号是实现的内部细节, 钉它只会制造
    /// 与实现耦合的脆弱判据; 断言"起点/跨度/计数/名字"才是契约。
    fn opening_tag<'a>(xml: &'a str, name: &str) -> &'a str {
        let needle = format!("<{name} ");
        let start = xml
            .find(&needle)
            .unwrap_or_else(|| panic!("缺少元素 {name}"));
        let end = xml[start..].find('>').expect("标签必须闭合");
        &xml[start..=start + end]
    }

    /// 判据 A/B/C（本切片的主判据）：`filled_project()` → 导出 → **内存 gunzip** →
    /// ① XML 结构良构; ② 音轨名与计数符合预期; ③ 损失表非空且覆盖所有未映射构造。
    #[test]
    fn filled_project_exports_gzip_xml_with_a_complete_loss_table() {
        let project = filled_project();
        let export = export_project(&project).expect("导出");

        // ---- (a) Gzip → 内存解压 → XML 结构 ----
        assert_eq!(export.bytes[..2], ALS_GZIP_MAGIC, "必须是 Gzip 成员");
        let xml = gunzip(&export.bytes);
        let root = check_xml(&xml).expect("XML 必须结构良构");
        assert_eq!(root, "Ableton");
        assert!(xml.contains("<LiveSet>"), "根下必须有 LiveSet");
        assert!(xml.contains(ALS_XML_DECLARATION), "必须以 XML 声明开头");

        // ---- (b) 音轨名与计数 ----
        for name in ["Master", "Lead", "Bass", "Aux Reverb"] {
            let needle = format!(r#"<EffectiveName Value="{name}"/>"#);
            assert!(xml.contains(&needle), "缺少音轨名: {name}");
        }
        assert_eq!(xml.matches("<MidiTrack ").count(), 1, "MIDI 轨数");
        assert_eq!(xml.matches("<AudioTrack ").count(), 1, "音频轨数");
        assert_eq!(xml.matches("<ReturnTrack ").count(), 1, "返回轨数");
        assert_eq!(xml.matches("<MasterTrack ").count(), 1, "主总线数");
        assert_eq!(export.mapped_tracks, 4, "映射音轨数（含 Master）");
        assert_eq!(export.mapped_clips, 2, "映射片段数（MIDI + 音频）");
        assert_eq!(export.mapped_notes, 4, "映射音符数");
        // ⚠ 数 `<MidiNoteEvent ` 而不是裸词：损失表的注释里也会点到这个元素名。
        assert_eq!(xml.matches("<MidiNoteEvent ").count(), 4, "音符事件数");
        for pitch in [60, 64, 67, 72] {
            let needle = format!(r#"<KeyTrack MidiKey="{pitch}">"#);
            assert!(xml.contains(&needle), "缺少键轨: {pitch}");
        }
        // 960 PPQ → 拍：第 2 个音符在 tick 960 = 1 拍, 时值 480 tick = 0.5 拍。
        assert!(
            xml.contains(r#"<MidiNoteEvent Time="1" Duration="0.5" Velocity="100"/>"#),
            "tick→拍 换算不对"
        );
        // 摆放起点/跨度：MIDI 摆放 0..3840 tick = 0..4 拍; 音频摆放 1920..2880 = 2..3 拍。
        let midi_clip = opening_tag(&xml, "MidiClip");
        assert!(
            midi_clip.contains(r#"Time="0""#) && midi_clip.contains(r#"Duration="4""#),
            "MIDI 摆放起止/跨度不对: {midi_clip}"
        );
        let audio_clip = opening_tag(&xml, "AudioClip");
        assert!(
            audio_clip.contains(r#"Time="2""#) && audio_clip.contains(r#"Duration="1""#),
            "音频摆放起止/跨度不对: {audio_clip}"
        );
        // 混音：-3 dB → 线性幅度; pan; mute=false ⇒ Speaker=true。
        let gain = crate::render::db_to_linear(-3.0).to_string();
        assert!(xml.contains(&format!(r#"<Manual Value="{gain}"/>"#)));
        assert!(xml.contains(r#"<Manual Value="-0.25"/>"#));
        assert!(xml.contains("<Speaker>"));

        // ---- (c) 损失表：非空, 且覆盖每一个"我没有等价映射"的构造 ----
        assert!(
            export.losses.len() >= 15,
            "损失表太短, 说明有静默丢失: {}",
            export.losses.len()
        );
        for loss in &export.losses {
            assert!(!loss.entity.is_empty(), "损失条目必须有实体寻址");
            assert!(
                loss.reason.starts_with(LOSS_UNMAPPED_PREFIX)
                    || loss.reason.starts_with(LOSS_NOT_EQUIVALENT_PREFIX),
                "reason 必须以两分法前缀开头: {}",
                loss.reason
            );
        }

        // 内建合成器 → 音频冻结兜底（唯一一个 bounced 条目）。
        let bounced: Vec<&AlsLoss> = export
            .losses
            .iter()
            .filter(|loss| loss.bounced_to_audio)
            .collect();
        assert_eq!(bounced.len(), 1, "只有内置合成器走音频冻结兜底");
        assert!(bounced[0].entity.contains("Yeban PolySynth"));
        assert!(xml.contains("yeban-loss"), "XML 本体必须带损失标记");

        // 逐项覆盖（每一项都对应映射表里的一格）。
        assert!(
            losses_matching(&export, "device:Yeban PolySynth#")
                .iter()
                .any(|loss| loss.bounced_to_audio),
            "设备未映射"
        );
        assert!(
            export
                .losses
                .iter()
                .any(|loss| loss.entity.starts_with("device:") && loss.reason.contains("PDC-001")),
            "设备延迟未登记"
        );
        assert!(
            losses_matching(&export, "macro:Brightness#").len() == 1,
            "宏未登记"
        );
        assert!(
            export
                .losses
                .iter()
                .any(|loss| loss.entity.starts_with("automation:")),
            "自动化泳道未登记"
        );
        assert!(
            export
                .losses
                .iter()
                .any(|loss| loss.entity.starts_with("routing-edge:")),
            "发送边未登记"
        );
        assert!(
            export
                .losses
                .iter()
                .any(|loss| loss.entity.starts_with("asset:")),
            "资产未打包未登记"
        );
        assert!(
            export
                .losses
                .iter()
                .any(|loss| loss.entity.starts_with("clip:")
                    && loss.reason.contains("syllable")
                    && loss.reason.contains("probability")
                    && loss.reason.contains("ratchet")
                    && loss.reason.contains("micro_timing_ticks")),
            "音符表现属性未登记"
        );
        assert!(
            export
                .losses
                .iter()
                .any(|loss| loss.reason.contains("音频片段增益")),
            "音频片段增益未登记"
        );
        assert!(
            export
                .losses
                .iter()
                .any(|loss| loss.reason == ELEMENT_VOCABULARY_CAVEAT),
            "元素词汇表保真度警告未登记"
        );
        assert!(
            export
                .losses
                .iter()
                .any(|loss| loss.reason.contains("end_tick")),
            "段落终点未登记"
        );
        assert!(
            export
                .losses
                .iter()
                .any(|loss| loss.reason.contains("solo_safe")),
            "solo_safe 未登记"
        );
        assert!(
            export
                .losses
                .iter()
                .any(|loss| loss.reason.contains("色标")),
            "色标未登记"
        );
    }

    /// 判据 D：输出逐字节可复现（无时间戳 / 无 `HashMap` 迭代序）。
    #[test]
    fn filled_project_export_is_byte_deterministic() {
        let project = filled_project();
        let first = export_project(&project).expect("第一次导出");
        let second = export_project(&project).expect("第二次导出");
        assert_eq!(first.bytes, second.bytes, "两次导出的字节必须相同");
        assert_eq!(first.losses, second.losses, "损失表也必须相同");
    }

    /// 判据 E：`bounced_to_audio` 不是恒真 —— 空工程没有任何"需要烘音频"的构造,
    /// 但损失表**仍然非空**（工程级构造照样会丢）。
    #[test]
    fn empty_project_has_no_audio_freeze_and_only_project_losses() {
        let export = export_project(&default_project()).expect("导出");
        let xml = gunzip(&export.bytes);
        assert_eq!(check_xml(&xml).expect("良构"), "Ableton");
        assert!(export.losses.iter().all(|loss| !loss.bounced_to_audio));
        assert!(
            export
                .losses
                .iter()
                .all(|loss| loss.entity.starts_with("project:")),
            "空工程只该有工程级损失"
        );
        assert!(!export.losses.is_empty());
        assert_eq!(export.mapped_tracks, 0);
        assert_eq!(export.mapped_clips, 0);
        assert_eq!(export.mapped_notes, 0);
    }

    /// 判据 F：转义与注释消毒是**真的在做**（否则上面那条"良构"判据可能只是恒真）。
    #[test]
    fn escaping_and_comment_sanitising_are_not_vacuous() {
        let mut project = filled_project();
        project.title = "A & B <\"x\">".to_owned();
        // 音轨名会进损失表的 `entity`（→ 注释）, 因此这是"含 `--` 的注释"的入口。
        for track in project.tracks.values_mut() {
            if track.kind == TrackKind::Midi {
                track.name = "Le--ad".to_owned();
            }
        }
        let document = build_live_set_xml(&project);

        // 属性值必须被转义（原始危险串一个都不许留）。
        assert!(
            document
                .xml
                .contains(r#"Value="A &amp; B &lt;&quot;x&quot;&gt;""#),
            "属性值未转义"
        );
        assert!(!document.xml.contains("A & B"));
        assert!(!document.xml.contains("<\"x\">"));
        // 注释里的 `--` 必须被拆开, 否则 XML 直接非法（check_xml 会红）。
        assert!(document.xml.contains("- -"), "注释里的 `--` 未消毒");
        check_xml(&document.xml).expect("转义 + 消毒后必须仍然良构");

        // 消毒函数本身的行为也要钉住（不是靠 check_xml 间接判断）。
        assert_eq!(comment_body("a--b-"), "a- -b-");
    }
}
