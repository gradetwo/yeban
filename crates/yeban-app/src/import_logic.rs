//! 真导入 `.logicx`：把 Logic Pro 工程 bundle 的**区域摆放 + 音符**映射成
//! `YebanProjectV1`，并原子落成一个可 `--headless --open` 的 `.yeban` 容器
//! （任务单 `#149`，刀 2；`[ARCH-FMT-002]` / `[ROAD-M4-007]`）。
//!
//! ## 这不是探针
//!
//! `#143` / `#148` 的 `yeban_render::logic_read` 是**只读探针**；本模块是它的
//! **消费者**：调用 [`yeban_render::logic_read::import_bundle`]（同一套记录行走、
//! 同一套 `rest+0xe0` / `rest+0x3c`、同一个 `decode_note_lines`）后，把读数建成本仓
//! 唯一的工程模型 `YebanProjectV1`，再用 [`crate::save::save_project_file`]
//! 原子落盘。⛔ 本模块不写第二份 `ProjectData` 解析器、⛔ 不写第二份音符解码器。
//!
//! ## 映射（逐项，⛔ 不发明字段）
//!
//! | Logic 侧（8 台 Apple demo 实测） | 本仓侧 |
//! | :--- | :--- |
//! | `karT` 且对象号 `0x0004_0000` 且 `payload[0] == 1` 的条数 | 轨道条数（外加一条 `Master`） |
//! | `qeSM` 非内部、长度 ≠ 7,680,000、长度 ≠ 0、长度 ∈ [60, 2,196,480]（`#150`） | 一个 `ClipPoolEntry` + 一个 `ClipPlacement` |
//! | 区域起点 `rest + 0xe0`（960 PPQ tick） | `ClipPlacement::start_tick`（**原值**，⛔ 不对齐到拍） |
//! | 区域长度 `rest + 0x3c`（960 PPQ tick） | `ClipPlacement::duration_ticks`（**原值**，⛔ 不扩展到音符） |
//! | 配对 `qSvE` 的 `decode_note_lines` | `MidiNote`（`start_tick` 是**区域相对** tick，与模型的 `placement.start + note.start` 口径一致） |
//! | 摆放事件 `+0x14`（1 起轨道号；槽位 `+0x20` 与位置 `+0x04` 都与区域相等） | `ClipPlacement` 落到对应轨（**实测**；`#151`） |
//! | `MetaData.plist` 的 `BeatsPerMinute` / 拍号 / `SampleRate` | 顶层 `bpm` / `time_signature` / `audio_config.sample_rate` |
//!
//! ## 轨道归属：实测（`#151`）＋ 余下 N 条仍 provisional
//!
//! **能绑定的都绑定到实测轨道。** 轨道号**不在** `qeSM` 载荷里（`rest + 0x00..0x100`
//! 的 `u8`/`u16`/`u32` 全扫过 8/8 台，没有字段同时满足「1 ≤ 值 ≤ 轨数」与「每台 ≥ 2
//! 个不同值」）；它只在**摆放事件** `+0x14`：`qSvE` 载荷里 80 字节、`+0x00 == 0x20`
//! 的事件，`+0x04 = 34560 + 区域起点`、`+0x20` = 区域槽（`qeSM` 记录头 `+0x08` 的对象号
//! `>> 16`）。绑定判据是三条**同时**成立：槽位相等、位置相等、`1 ≤ 轨道号 ≤ 轨数`；
//! 同一槽位出现矛盾读数 ⇒ 不绑定（⛔ 不猜）。8 台实测 **651/864** 条可导入区域有
//! 可用的摆放事件（逐台读数见 `yeban_render::logic_read` 的模块头表格）。
//!
//! ⚠ **余下 213 条没有摆放事件**（形态与同一位置的 take／Track Alternatives 一致）
//! ⇒ 这 213 条**仍按 provisional 轮转**：第 `i` 条可导入区域（按 `qeSM` 记录流顺序，
//! 0 起）落到第 `i % 轨道数` 条非 master 轨。损失表逐条计数，⛔ 不静默、⛔ 不假装
//! 它们也是实测的。
//!
//! ## 两条摆放裁决（照 `#149` 的成文裁决，⛔ 不另问）
//!
//! 1. **loop/RBA 邻接的 1 tick 偏差**（如 `193919 = 193920 − 1`）⇒ **按原值使用**，
//!    ⛔ 不对齐到拍（对齐会改数据）。
//! 2. **区域长度短于音符内容**（如 `Bell` 30720 vs 46915）⇒ **按长度裁剪**：
//!    `ClipPlacement::duration_ticks = 区域长度`，⛔ **不**把长度扩展到音符末端。
//!    ⚠ 实现口径：音符内容**原样保留**（`MidiNote` 一条不删；总数因此与 `#148` 的
//!    `notes` 逐项相等），播放被摆放窗口裁掉的那部分由引擎的
//!    `placement_start + note.start_tick` 口径承担。`notes_outside_region` 是这条
//!    裁切的**读数**（Swing! = 168 条），逐条打在报告行里，⛔ 不静默。
//!
//! ## 损失表（四级 + 两条边界读数，逐条打印）
//!
//! 每一条都是 `logic-import-loss:` 一行；**至少**这四条逐字出现：
//! `AuRg` / `GenM` / `自动化` / `轨道归属`（后者写明实测绑定数与余下的 provisional 数）。
//!
//! | 级别 | 种类 | 处理 |
//! | :--- | :--- | :--- |
//! | region | `AuRg` 音频区域 | 只计数，⛔ 不导入（载荷语义未勘察） |
//! | channel | `GenM` 通道状态 | 只计数，⛔ 不导入（混音器状态） |
//! | control | 自动化曲线 / `RBA Sequence` | 内部区域，⛔ 不导入 |
//! | binding | **轨道归属（实测 + 余下 provisional）** | 651/864 条按摆放事件 `+0x14` 落轨；余下 213 条轮转（`#151`） |
//! | region | 空区域哨兵（7,680,000） | 跳过（起点恒 0、音符恒 0） |
//! | region | **区域长度越界**（`#150`） | 长度不在 `[60, 2,196,480]` tick ⇒ ⛔ 不进工程，**逐条**打印 |
//! | boundary | 长度短于音符内容 | 摆放按长度；音符原样保留（读数） |
//!
//! `#150` 实测：8 台 demo 共 **17 条**越界 —— **11 条**长度 1,073,626,623
//! （≈ 10.7 亿 tick ≈ 559,000 拍）＋ **6 条**长度 4 tick。两者都逐条打印成
//! `level=boundary kind=length-out-of-range start=… length=…`，⛔ 不进工程、⛔ 不静默。
//!
//! ## 版权与只读
//!
//! 输入是 Apple 演示工程（有版权）：本模块**只读**输入，产物只写到调用方给的
//! `--save-as`；⛔ 不复制 demo、⛔ 不把 demo 或解析结果入库。输入路径不存在 ⇒
//! 打印"无 demo、跳过"并成功返回（路径门控，⛔ 不红）；路径在但解析失败 ⇒ **红**。
//!
//! ## 确定性（同一台导入两次 ⇒ 容器逐字节相同）
//!
//! `EntityId` **不用** `EntityId::new()`（ULID 含时间与随机位）：本模块用一个
//! **序号分配器**（`01J8ZQ` + 20 位十进制）按固定顺序发号；工程时间戳是一个固定常量
//! （⛔ 不声称是真实创作时间，只为可复现）；`BTreeMap` 与容器写出本身已是确定性的
//! （`write_project_container` 的既有契约）。因此同一份输入两次导入得到**同字节**容器。

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;

use yeban_model::ids::EntityId;
use yeban_model::music::MidiNote;
use yeban_model::project::{
    ClipContent, ClipPlacement, ClipPoolEntry, DEFAULT_RNG_SEED, DEFAULT_WRITER_VERSION,
    LoopConfig, MIN_READER_VERSION, ProjectAudioConfig, ProjectMetadata, RoutingEdge, RoutingGraph,
    RoutingKind, SCHEMA_VERSION, SampleRate, TimeSignature, TrackKind, TrackV3, TransportConfig,
    YebanProjectV1,
};
use yeban_render::logic_read::{
    LOGIC_REGION_LENGTH_MAX, LOGIC_REGION_LENGTH_MIN, LogicImport, import_bundle,
};

use crate::save::{SaveError, SaveReport, save_project_file};

/// 导入产物的固定时间戳（Unix 毫秒）。
///
/// ⚠ 它**不是**"这个工程创建于 2025-10-05"的事实主张：Logic bundle 的元数据里没有
/// 这一层可移植语义，而确定性判据（同一台两次导入 ⇒ 容器同字节）要求一个**常量**。
/// 用 `0` 会与"1970 年创建"不可区分（`ProjectMetadata` 的字段文档明确反对），
/// 因此取一个显式的、带注释的占位常量。
pub const IMPORTED_AT_UNIX_MS: u64 = 1_760_000_000_000;

/// 报告里逐条打印的前 N 条摆放（`(start_tick, duration_ticks)` 的对照供人与判据读）。
pub const PLACEMENT_REPORT_LIMIT: usize = 8;

/// 导入失败的全部原因（每一种都**响亮**上报，⛔ 绝不"写了个空容器也算成功"）。
#[derive(Debug)]
pub enum ImportLogicError {
    /// 输入路径存在，但既不是 `.logicx` bundle 也不是含 bundle 的目录。
    NotABundle {
        /// 输入路径。
        path: PathBuf,
    },
    /// 目录存在，但里面没有一个 `.logicx` bundle（⛔ 不静默跳过）。
    NoBundles {
        /// 输入目录。
        path: PathBuf,
    },
    /// 读 demo 字节失败 / 结构不合法 / 区域字段越过载荷。
    ReadDemo {
        /// bundle 路径。
        bundle: PathBuf,
        /// `logic_read` 的原样文案。
        detail: String,
    },
    /// `MetaData.plist` 的采样率不是规范枚举之一。
    UnsupportedSampleRate {
        /// 实际读到的 Hz。
        value: u32,
    },
    /// 一个轨道都没有（没有可分配区域的落点）。
    NoTracks {
        /// bundle 路径。
        bundle: PathBuf,
    },
    /// 建出的工程未通过 `YebanProjectV1::validate()`。
    InvalidProject {
        /// 模型层的原样裁决。
        detail: String,
    },
    /// 原子落盘失败。
    Save {
        /// 目标路径。
        path: PathBuf,
        /// 保存层的原样裁决。
        source: Box<SaveError>,
    },
}

impl fmt::Display for ImportLogicError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotABundle { path } => write!(
                formatter,
                "`{}` 既不是 `.logicx` bundle（缺 Resources/ProjectInformation.plist），\
                 也不是包含 `.logicx` 的目录",
                path.display()
            ),
            Self::NoBundles { path } => write!(
                formatter,
                "`{}` 存在，但里面没有一个 `.logicx` bundle（⛔ 不静默跳过）",
                path.display()
            ),
            Self::ReadDemo { bundle, detail } => {
                write!(
                    formatter,
                    "读 Logic 工程 `{}` 失败: {detail}",
                    bundle.display()
                )
            }
            Self::UnsupportedSampleRate { value } => write!(
                formatter,
                "采样率 {value} Hz 不是规范枚举之一（44100/48000/88200/96000/192000）"
            ),
            Self::NoTracks { bundle } => write!(
                formatter,
                "`{}` 里没有一条 `karT`（payload[0] == 1）轨道 ⇒ 没有可分配区域的落点",
                bundle.display()
            ),
            Self::InvalidProject { detail } => {
                write!(formatter, "导入产出的工程未通过模型校验: {detail}")
            }
            Self::Save { path, source } => {
                write!(formatter, "保存到 `{}` 失败: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for ImportLogicError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Save { source, .. } => Some(source.as_ref()),
            Self::NotABundle { .. }
            | Self::NoBundles { .. }
            | Self::ReadDemo { .. }
            | Self::UnsupportedSampleRate { .. }
            | Self::NoTracks { .. }
            | Self::InvalidProject { .. } => None,
        }
    }
}

impl ImportLogicError {
    /// 进程退出码（与 [`crate::cli::CliError`] 同一张码表，⛔ 不发明新码）。
    ///
    /// * 落盘失败 ⇒ [`crate::cli::EXIT_SAVE`]（4）；
    /// * 其余（读 demo / 路径 / 采样率 / 无轨道 / 模型校验）⇒ [`crate::cli::EXIT_OPEN`]（3）。
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Save { .. } => crate::cli::EXIT_SAVE,
            Self::NotABundle { .. }
            | Self::NoBundles { .. }
            | Self::ReadDemo { .. }
            | Self::UnsupportedSampleRate { .. }
            | Self::NoTracks { .. }
            | Self::InvalidProject { .. } => crate::cli::EXIT_OPEN,
        }
    }
}

/// 确定性 `EntityId` 发号器（`01J8ZQ` + 20 位十进制序号）。
///
/// 为什么不用 `EntityId::new()`：ULID 的低位含随机数、高位是当前毫秒 —— 同一台
/// 导入两次会得到**不同**的工程字节（判据 4 会红）。序号分配器让"第几个实体"
/// 与身份一一对应，且升序与创建顺序同序（`BTreeMap` 迭代顺序因此稳定）。
struct IdAllocator {
    next: u128,
}

impl IdAllocator {
    const fn new() -> Self {
        Self { next: 0 }
    }

    fn next_id(&mut self) -> EntityId {
        let text = format!("01J8ZQ{:020}", self.next);
        self.next += 1;
        EntityId::from_str(&text).expect("固定前缀 + 20 位十进制必然是合法 ULID")
    }
}

/// 一次成功导入的读数（报告行的唯一来源）。
struct ImportReport {
    /// 输入路径（调用方给的那个）。
    input: PathBuf,
    /// 实际导入的 bundle。
    bundle: PathBuf,
    /// `logic_read` 的字节读数。
    data: LogicImport,
    /// 建出的轨道条数（含 master）。
    project_tracks: usize,
    /// 建出的摆放条数。
    placements: usize,
    /// 建出的片段池条数。
    clips: usize,
    /// 建出的音符条数。
    project_notes: usize,
    /// 每条摆放的 `(区域序号, 轨道序号, 起点, 长度, 音符数)`。
    placement_rows: Vec<(usize, usize, u64, u64, usize)>,
    /// 保存读数。
    saved: SaveReport,
}

impl ImportReport {
    fn lines(&self) -> Vec<String> {
        let data = &self.data;
        let mut lines = Vec::with_capacity(12 + PLACEMENT_REPORT_LIMIT + 8);
        lines.push(crate::cli::HEADLESS_HANDSHAKE.to_owned());
        lines.push(format!(
            "logic-import-source: input={} bundle={} alternative={}",
            crate::cli::quoted(&self.input.display().to_string()),
            crate::cli::quoted(&self.bundle.display().to_string()),
            data.alternative,
        ));
        // 与 `#148` 的 `logic-demo:` 行同一组读数（轨 / 区域 / 音符 / AuRg），
        // 判据因此可以逐项对照，而不用在两套口径之间换算。
        lines.push(format!(
            "logic-import: bundle={} alternative={} sample_rate={} bpm={} tracks={} \
             regions={} non_internal={} notes={} au_rg={}",
            data.bundle,
            data.alternative,
            data.sample_rate,
            data.bpm,
            data.tracks,
            data.regions_all,
            data.regions,
            data.notes,
            data.audio_regions,
        ));
        lines.push(format!(
            "logic-import-counts: imported-regions={} placements={} project-tracks={} \
             project-notes={} clips-pool={} internal={} automation={} rba={} gen_m={} \
             skipped-empty={} skipped-zero-length={} skipped-out-of-range={} unpaired-notes={} \
             notes-outside-region={} measured-track-regions={} provisional-track-regions={}",
            data.imported_regions.len(),
            self.placements,
            self.project_tracks,
            self.project_notes,
            self.clips,
            data.internal_regions,
            data.automation_regions,
            data.rba_sequences,
            data.channel_states,
            data.skipped_empty_regions,
            data.skipped_zero_length_regions,
            data.out_of_range_regions.len(),
            data.unpaired_notes,
            data.notes_outside_region,
            data.measured_track_regions(),
            data.provisional_track_regions(),
        ));
        lines.push(format!(
            "logic-import-placements: count={} reported={}",
            self.placement_rows.len(),
            self.placement_rows.len().min(PLACEMENT_REPORT_LIMIT),
        ));
        for (index, track, start, length, notes) in
            self.placement_rows.iter().take(PLACEMENT_REPORT_LIMIT)
        {
            let name = data
                .imported_regions
                .get(*index)
                .map_or("", |region| region.name.as_str());
            lines.push(format!(
                "logic-import-placement: index={index} track={track} name={} start_tick={start} \
                 duration_ticks={length} notes={notes}",
                crate::cli::quoted(name),
            ));
        }
        lines.push(format!("logic-import-losses: count={}", self.loss_count()));
        for loss in self.losses() {
            lines.push(format!("logic-import-loss: {loss}"));
        }
        lines.push(format!(
            "saved: path={} bytes={} temp={}",
            crate::cli::quoted(&self.saved.path.display().to_string()),
            self.saved.bytes,
            crate::cli::quoted(&self.saved.temp_name),
        ));
        lines
    }

    /// 损失表条数（与 [`Self::losses`] 逐条对应；⛔ 不写死数字，
    /// 否则加了条目就会与 `losses()` 漂移）。
    fn loss_count(&self) -> usize {
        self.losses().len()
    }

    /// 损失表（四级 + 两条边界读数；⛔ 不静默丢掉任何一类）。
    fn losses(&self) -> Vec<String> {
        let data = &self.data;
        let mut losses = vec![
            format!(
                "level=region kind=AuRg count={} 音频区域未导入（只计数；载荷语义未勘察）",
                data.audio_regions
            ),
            format!(
                "level=channel kind=GenM count={} 通道状态未导入（混音器 / 通道状态）",
                data.channel_states
            ),
            format!(
                "level=control kind=自动化曲线 count={} 自动化曲线未导入（*Automation={} + RBA={}）",
                data.internal_regions, data.automation_regions, data.rba_sequences
            ),
            format!(
                "level=binding kind=轨道归属 实测：{}/{} 条可导入区域按摆放事件 `+0x14` 绑定；\
                 余下 {} 条区域仍 provisional（没有摆放事件，按记录流顺序轮转到 {} 条非 master 轨）",
                data.measured_track_regions(),
                data.imported_regions.len(),
                data.provisional_track_regions(),
                data.tracks
            ),
            format!(
                "level=region kind=空区域哨兵 count={} 长度 7,680,000 的区域跳过\
                 （起点恒 0、音符恒 0；不是真实长度）",
                data.skipped_empty_regions
            ),
            format!(
                "level=region kind=长度越界 count={} 长度不在 [{}, {}] tick 内的区域跳过\
                 （#150；逐条列在下面）",
                data.out_of_range_regions.len(),
                LOGIC_REGION_LENGTH_MIN,
                LOGIC_REGION_LENGTH_MAX,
            ),
            format!(
                "level=boundary kind=长度裁剪 count={} 起点 ≥ 区域长度的音符；\
                 摆放按区域长度裁剪、音符内容原样保留（⛔ 不扩展到音符）",
                data.notes_outside_region
            ),
        ];
        // `#150`：越界样本**逐条**列出（⛔ 不静默、⛔ 不只报总数）。
        for sample in &data.out_of_range_regions {
            losses.push(format!(
                "level=boundary kind=length-out-of-range start={} length={} name={} notes={} \
                 区域长度越界（不在 [{}, {}] tick），⛔ 不进工程",
                sample.start_tick,
                sample.length_ticks,
                crate::cli::quoted(&sample.name),
                sample.notes.len(),
                LOGIC_REGION_LENGTH_MIN,
                LOGIC_REGION_LENGTH_MAX,
            ));
        }
        losses
    }
}

/// 执行一次真导入：读 bundle ⇒ 建 `YebanProjectV1` ⇒ 原子写成 `.yeban` ⇒ 返回报告行。
///
/// 输入可以是：
/// * 一台 `.logicx` bundle（目录里有 `Resources/ProjectInformation.plist`）；
/// * 一个**包含** `.logicx` 的目录（取字典序第一台，确定性；报告里打 `input=` 与
///   `bundle=` 两个路径，⛔ 不让人误以为导入了整个目录）。
///
/// **路径门控**：输入路径不存在 ⇒ 打印一行"无 demo、跳过"并成功返回（⛔ 不红），
/// **不写**输出文件。路径在但读不懂 ⇒ [`ImportLogicError`]（红）。
///
/// # Errors
///
/// 见 [`ImportLogicError`]：路径类型 / 无 bundle / 读 demo / 采样率 / 无轨道 /
/// 模型校验 / 落盘任一档失败都响亮上报。
pub fn run(input: &Path, output: &Path) -> Result<Vec<String>, ImportLogicError> {
    if !input.exists() {
        // 路径门控：CI / 其它机器没有 Apple demo ⇒ 这不是失败。
        return Ok(vec![format!(
            "logic-import: 无 demo（{} 不存在），跳过（0 台，不红）",
            input.display()
        )]);
    }

    let bundle = resolve_bundle(input)?;
    let data = import_bundle(&bundle).map_err(|detail| ImportLogicError::ReadDemo {
        bundle: bundle.clone(),
        detail,
    })?;
    let (project, project_tracks, placements, clips, project_notes, placement_rows) =
        build_project(&data)?;
    project
        .validate()
        .map_err(|error| ImportLogicError::InvalidProject {
            detail: error.to_string(),
        })?;

    let saved = save_project_file(&project, output).map_err(|source| ImportLogicError::Save {
        path: output.to_path_buf(),
        source: Box::new(source),
    })?;

    Ok(ImportReport {
        input: input.to_path_buf(),
        bundle,
        data,
        project_tracks,
        placements,
        clips,
        project_notes,
        placement_rows,
        saved,
    }
    .lines())
}

/// 解析输入：bundle 直接用；目录取字典序第一台 `.logicx`。
fn resolve_bundle(input: &Path) -> Result<PathBuf, ImportLogicError> {
    if input
        .join("Resources")
        .join("ProjectInformation.plist")
        .is_file()
    {
        return Ok(input.to_path_buf());
    }
    if !input.is_dir() {
        return Err(ImportLogicError::NotABundle {
            path: input.to_path_buf(),
        });
    }
    let mut bundles: Vec<PathBuf> = std::fs::read_dir(input)
        .map_err(|_| ImportLogicError::NoBundles {
            path: input.to_path_buf(),
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().and_then(|ext| ext.to_str()) == Some("logicx") && path.is_dir()
        })
        .collect();
    bundles.sort();
    bundles
        .into_iter()
        .next()
        .ok_or_else(|| ImportLogicError::NoBundles {
            path: input.to_path_buf(),
        })
}

/// 把字节读数建成本仓的工程模型（纯函数，⛔ 不落盘）。
///
/// 返回 `(工程, 轨道总数含 master, 摆放数, 片段池数, 音符数, 逐条摆放读数)`。
#[allow(clippy::type_complexity)]
fn build_project(
    data: &LogicImport,
) -> Result<
    (
        YebanProjectV1,
        usize,
        usize,
        usize,
        usize,
        Vec<(usize, usize, u64, u64, usize)>,
    ),
    ImportLogicError,
> {
    if data.tracks == 0 {
        return Err(ImportLogicError::NoTracks {
            bundle: PathBuf::from(&data.bundle),
        });
    }
    let sample_rate = SampleRate::from_hz(data.sample_rate).map_err(|_| {
        ImportLogicError::UnsupportedSampleRate {
            value: data.sample_rate,
        }
    })?;

    let mut ids = IdAllocator::new();
    let project_id = ids.next_id();
    let master_id = ids.next_id();

    let mut tracks: BTreeMap<EntityId, TrackV3> = BTreeMap::new();
    tracks.insert(
        master_id,
        TrackV3 {
            id: master_id,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    let mut track_ids: Vec<EntityId> = Vec::with_capacity(data.tracks);
    for index in 0..data.tracks {
        let id = ids.next_id();
        track_ids.push(id);
        tracks.insert(
            id,
            TrackV3 {
                id,
                name: format!("Track {:03}", index + 1),
                kind: TrackKind::Midi,
                ..TrackV3::default()
            },
        );
    }

    let mut clip_pool: BTreeMap<EntityId, ClipPoolEntry> = BTreeMap::new();
    let mut placement_rows: Vec<(usize, usize, u64, u64, usize)> =
        Vec::with_capacity(data.imported_regions.len());
    let mut placements = 0usize;
    let mut project_notes = 0usize;
    // ⚠ `data.imported_regions` 已由 `import_bundle` 的**逐条**长度判据
    // （`#150`，`[LOGIC_REGION_LENGTH_MIN, LOGIC_REGION_LENGTH_MAX]`）过滤过；
    // 本模块⛔ 不再写第二份判据（会与读取侧漂移），越界样本由读取侧的
    // `out_of_range_regions` 交给损失表逐条打印。
    for (index, region) in data.imported_regions.iter().enumerate() {
        let clip_id = ids.next_id();
        let placement_id = ids.next_id();
        let mut notes: BTreeMap<EntityId, MidiNote> = BTreeMap::new();
        for note in &region.notes {
            let note_id = ids.next_id();
            let mut model_note = MidiNote::new(
                note_id,
                note.start_tick,
                note.pitch,
                note.duration_ticks.max(1),
            );
            model_note.velocity = note.velocity;
            notes.insert(note_id, model_note);
        }
        project_notes += notes.len();
        clip_pool.insert(
            clip_id,
            ClipPoolEntry {
                id: clip_id,
                name: region.name.clone(),
                content: ClipContent::Midi { notes },
            },
        );

        // `#151`：优先用**实测**轨道号（摆放事件 `+0x14`，与区域槽位/位置都相等）。
        // 落点索引 = 轨道号 − 1（轨道号 1 起，`track_ids` 0 起）。
        // 余下的区域（没有摆放事件 / 读数矛盾 / 轨道号越界）**仍 provisional**：
        // 第 i 条可导入区域落到第 `i % 轨道数` 条非 master 轨（公开规则，⛔ 不猜）。
        let measured_index = region
            .track_1based
            .and_then(|track| usize::try_from(track).ok())
            .and_then(|track| track.checked_sub(1))
            .filter(|index| *index < track_ids.len());
        let track_index = measured_index.unwrap_or(index % track_ids.len());
        let track = tracks
            .get_mut(&track_ids[track_index])
            .expect("track_ids 与 tracks 同源");
        track.clips.insert(
            placement_id,
            ClipPlacement {
                id: placement_id,
                clip_id,
                start_tick: u64::from(region.start_tick),
                duration_ticks: u64::from(region.length_ticks),
                loop_config: LoopConfig::default(),
                muted: false,
            },
        );
        placements += 1;
        placement_rows.push((
            index,
            track_index,
            u64::from(region.start_tick),
            u64::from(region.length_ticks),
            notes_count(&clip_pool, clip_id),
        ));
    }

    let mut nodes: Vec<EntityId> = Vec::with_capacity(data.tracks + 1);
    nodes.push(master_id);
    nodes.extend(track_ids.iter().copied());
    let mut edges: BTreeMap<EntityId, RoutingEdge> = BTreeMap::new();
    for track_id in &track_ids {
        let edge_id = ids.next_id();
        edges.insert(
            edge_id,
            RoutingEdge {
                id: edge_id,
                source_node: *track_id,
                destination_node: master_id,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        );
    }

    let project = YebanProjectV1 {
        schema_version: SCHEMA_VERSION,
        min_reader_version: MIN_READER_VERSION,
        writer_version: DEFAULT_WRITER_VERSION.to_owned(),
        id: project_id,
        title: data.bundle.clone(),
        author: String::new(),
        bpm: data.bpm,
        time_signature: TimeSignature {
            numerator: u8::try_from(data.numerator).map_err(|_| {
                ImportLogicError::InvalidProject {
                    detail: format!("拍号分子 {} 越过 u8", data.numerator),
                }
            })?,
            denominator: u8::try_from(data.denominator).map_err(|_| {
                ImportLogicError::InvalidProject {
                    detail: format!("拍号分母 {} 越过 u8", data.denominator),
                }
            })?,
        },
        audio_config: ProjectAudioConfig {
            sample_rate,
            ..ProjectAudioConfig::default()
        },
        rng_seed: DEFAULT_RNG_SEED,
        metadata: ProjectMetadata {
            description: format!(
                "Imported from Logic Pro project `{}`；{}/{} 条区域按摆放事件 +0x14 绑定到实测轨道，\
                 余下 {} 条仍 provisional（无摆放事件）",
                data.bundle,
                data.measured_track_regions(),
                data.imported_regions.len(),
                data.provisional_track_regions(),
            ),
            created_at_unix_ms: IMPORTED_AT_UNIX_MS,
            modified_at_unix_ms: IMPORTED_AT_UNIX_MS,
            tags: vec![
                "logic-import".to_owned(),
                "measured-track-binding".to_owned(),
            ],
        },
        transport: TransportConfig::default(),
        sections: BTreeMap::new(),
        tracks,
        master_bus_track_id: master_id,
        routing_graph: RoutingGraph { nodes, edges },
        scenes: BTreeMap::new(),
        clip_pool,
        assets: BTreeMap::new(),
    };
    Ok((
        project,
        data.tracks + 1,
        placements,
        placements,
        project_notes,
        placement_rows,
    ))
}

/// 片段池里一个片段的音符数（用于逐条摆放读数）。
fn notes_count(clip_pool: &BTreeMap<EntityId, ClipPoolEntry>, clip_id: EntityId) -> usize {
    clip_pool
        .get(&clip_id)
        .and_then(|entry| entry.content.notes())
        .map_or(0, BTreeMap::len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_render::logic_read::ImportRegion;

    /// 造一份只用于映射判据的**合成**读数（⛔ 不含任何 demo 字节）。
    fn synthetic(tracks: usize, regions: Vec<ImportRegion>) -> LogicImport {
        LogicImport {
            bundle: "Synthetic".to_owned(),
            alternative: "000".to_owned(),
            sample_rate: 48_000,
            bpm: 120.0,
            numerator: 4,
            denominator: 4,
            tracks,
            regions_all: regions.len(),
            internal_regions: 0,
            automation_regions: 0,
            rba_sequences: 0,
            regions: regions.len(),
            notes: regions.iter().map(|region| region.notes.len()).sum(),
            audio_regions: 0,
            channel_states: 0,
            imported_regions: regions,
            skipped_empty_regions: 0,
            skipped_zero_length_regions: 0,
            out_of_range_regions: Vec::new(),
            unpaired_notes: 0,
            notes_outside_region: 0,
        }
    }

    fn region(name: &str, start: u32, length: u32, notes: usize) -> ImportRegion {
        ImportRegion {
            name: name.to_owned(),
            start_tick: start,
            length_ticks: length,
            notes: (0..notes)
                .map(|index| yeban_render::logic_read::ImportNote {
                    start_tick: u64::try_from(index).unwrap() * 240,
                    duration_ticks: 240,
                    pitch: 60,
                    velocity: 100,
                })
                .collect(),
            // 合成读数没有摆放事件 ⇒ 落回 provisional 轮转（`#151` 的 fallback 判据）。
            track_1based: None,
        }
    }

    /// 判据（映射 + provisional fallback）：没有实测轨道号的区域按记录流顺序轮转落轨；
    /// 起点/长度**原值**；音符数与每条区域的音符数逐项相等；工程通过模型校验。
    #[test]
    fn regions_are_placed_at_the_measured_ticks_and_bound_round_robin() {
        let data = synthetic(
            3,
            vec![
                region("A", 0, 3_840, 2),
                region("B", 19_391, 960, 0),
                region("C", 7_680, 15_360, 3),
                region("D", 30_720, 960, 1),
            ],
        );
        let (project, tracks, placements, clips, notes, rows) =
            build_project(&data).expect("合成读数必须能建工程");
        assert_eq!(project.validate(), Ok(()), "建出的工程必须通过模型校验");
        assert_eq!(tracks, 4, "3 条内容轨 + 1 条 master");
        assert_eq!(placements, 4);
        assert_eq!(clips, 4);
        assert_eq!(notes, 6);
        assert_eq!(
            rows.iter().map(|row| row.0).collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
        assert_eq!(
            rows.iter().map(|row| row.1).collect::<Vec<_>>(),
            vec![0, 1, 2, 0],
            "第 i 条区域落到第 i % 3 条非 master 轨（provisional）"
        );
        assert_eq!(
            rows.iter().map(|row| (row.2, row.3)).collect::<Vec<_>>(),
            vec![(0, 3_840), (19_391, 960), (7_680, 15_360), (30_720, 960)],
            "起点/长度原值：⛔ 不对齐到拍、⛔ 不扩展到音符"
        );
    }

    /// 判据（确定性）：同一份读数建两次 ⇒ 工程 JSON **逐字节相同**。
    #[test]
    fn building_the_same_reading_twice_is_byte_identical() {
        let data = synthetic(2, vec![region("A", 0, 960, 2), region("B", 960, 1_920, 1)]);
        let (first, ..) = build_project(&data).expect("第一次");
        let (second, ..) = build_project(&data).expect("第二次");
        assert_eq!(
            serde_json::to_vec(&first).unwrap(),
            serde_json::to_vec(&second).unwrap(),
            "确定性判据：两次建工程的 JSON 必须逐字节相同（EntityId 来自序号分配器）"
        );
    }

    /// 判据（损失表）：四条必需条目逐字出现在输出里。
    #[test]
    fn the_loss_table_names_the_four_required_kinds_verbatim() {
        let data = LogicImport {
            audio_regions: 7,
            channel_states: 9,
            internal_regions: 11,
            automation_regions: 6,
            rba_sequences: 5,
            skipped_empty_regions: 3,
            notes_outside_region: 2,
            ..synthetic(1, vec![region("A", 0, 960, 1)])
        };
        let (project, project_tracks, placements, clips, project_notes, rows) =
            build_project(&data).expect("合成读数");
        let report = ImportReport {
            input: PathBuf::from("in"),
            bundle: PathBuf::from("in/Synthetic.logicx"),
            data,
            project_tracks,
            placements,
            clips,
            project_notes,
            placement_rows: rows,
            saved: SaveReport {
                path: PathBuf::from("out.yeban"),
                bytes: 1,
                temp_name: "tmp".to_owned(),
            },
        };
        let text = report.lines().join("\n");
        for required in ["AuRg", "GenM", "自动化", "轨道归属"] {
            assert!(
                text.contains(required),
                "损失表必须逐字包含 `{required}`：\n{text}"
            );
        }
        assert!(
            text.contains("余下 1 条区域仍 provisional"),
            "损失表必须写明余下的 provisional 条数：\n{text}"
        );
        assert_eq!(project.validate(), Ok(()));
        assert_eq!(report.loss_count(), report.losses().len());
    }

    /// 判据（`#151`）：有实测轨道号的区域落到**实测**轨；没有的才轮转。
    ///
    /// 轨道号是 **1 起**（`track_1based`），落点索引 = 轨道号 − 1；越界/矛盾的读数
    /// （`0`、`tracks+1`）**不得**被当成实测轨道（⛔ 不越界 panic、⛔ 不猜）。
    #[test]
    fn measured_track_numbers_take_precedence_over_the_provisional_rotation() {
        let mut measured = region("M", 0, 960, 1);
        measured.track_1based = Some(3);
        let mut out_of_range = region("X", 960, 960, 1);
        out_of_range.track_1based = Some(4); // 只有 3 条非 master 轨 ⇒ 越界
        let mut zero = region("Z", 1_920, 960, 1);
        zero.track_1based = Some(0); // 轨道号 1 起 ⇒ 0 非法
        let data = synthetic(
            3,
            vec![measured, out_of_range, zero, region("P", 2_880, 960, 1)],
        );
        let (project, .., rows) = build_project(&data).expect("合成读数");
        assert_eq!(project.validate(), Ok(()));
        assert_eq!(
            rows.iter().map(|row| row.1).collect::<Vec<_>>(),
            vec![2, 1, 2, 0],
            "第 1 条落到 3−1 = 2；第 2 条越界 ⇒ 1 % 3 = 1；第 3 条 0 非法 ⇒ 2 % 3 = 2；第 4 条无实测 ⇒ 3 % 3 = 0"
        );
        assert_eq!(data.measured_track_regions(), 1);
        assert_eq!(data.provisional_track_regions(), 3);
    }

    /// 判据（`#150`）：越界样本**逐条**写进损失表（⛔ 只报一个总数）。
    #[test]
    fn the_loss_table_names_every_out_of_range_sample() {
        let mut data = synthetic(1, vec![region("A", 0, 960, 0)]);
        // 合成的两条 = `#150` 实测的两个垃圾字面量（Swing! 的记录流前两条）。
        data.out_of_range_regions = vec![region("", 0, 4, 0), region("TRASH", 0, 1_073_626_623, 0)];
        let (project, project_tracks, placements, clips, project_notes, rows) =
            build_project(&data).expect("合成读数");
        let report = ImportReport {
            input: PathBuf::from("in"),
            bundle: PathBuf::from("in/Synthetic.logicx"),
            data,
            project_tracks,
            placements,
            clips,
            project_notes,
            placement_rows: rows,
            saved: SaveReport {
                path: PathBuf::from("out.yeban"),
                bytes: 1,
                temp_name: "tmp".to_owned(),
            },
        };
        let text = report.lines().join("\n");
        assert_eq!(placements, 1, "越界样本 ⛔ 不进工程（只有 1 条合法区域）");
        assert!(text.contains("level=region kind=长度越界 count=2"));
        for sample in [
            "level=boundary kind=length-out-of-range start=0 length=4 ",
            "level=boundary kind=length-out-of-range start=0 length=1073626623 ",
        ] {
            assert!(
                text.contains(sample),
                "损失表必须逐条打印 `{sample}`：\n{text}"
            );
        }
        assert_eq!(project.validate(), Ok(()));
        assert_eq!(report.loss_count(), report.losses().len());
    }
}
