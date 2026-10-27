//! **错误报文的机器可读词表**（`data.reason`）的黄金表与零覆盖普查。
//!
//! ## 为什么要有这一份
//!
//! `mod-sfz` 的一次普查发现"44 个 `Display` 臂里 41 个**零断言**"、
//! `mod-theory` 实测"把 5 条 `Display` 改坏**全绿**"。本 crate 的错误出口
//! 用的是 `data.reason`（机器可读）而不是 `Display`，因此同一类风险在这里的形状是：
//! **一个 `reason` 写下了、也从没有任何判据提到它** —— 改坏它（改名、改条件、
//! 甚至把整条分支删掉换成别的 reason）没有任何东西会红。
//!
//! 本文件把这件事变成机制，用**三条**判据：
//!
//! | 判据 | 性质 | 它挡住的改动 |
//! | :--- | :--- | :--- |
//! | [`the_reason_vocabulary_is_the_published_list`] | 生产区**可分类**的 `reason` 集合 **==** 黄金表 | 偷偷改名 / 新增 / 删除一个 `reason` |
//! | [`every_reason_is_asserted_somewhere`] | 黄金表里的每一个都被**某条判据**提到（少数登记为不可达） | 新增一个零覆盖的 `reason` |
//! | [`every_reason_occurrence_is_classified_or_explicitly_registered`] | 每一次 `"reason"` 出现都必须分类；看不见的**逐条登记** | 换一种扫描器看不见的写法（#340 的盲区） |
//!
//! ⚠ 口径（刻意宽松，因此"零覆盖"是**下界**，不会把已覆盖的误报为缺口）：
//! 判据侧 = 任何一处 `"<reason>"` 字面量出现（`assert_eq!` / `json!` / 注释都算）。
//!
//! ## #340：为什么站点普查是必须的
//!
//! 旧扫描器只认 `"reason": "字面量"`（键后**紧跟**字符串）。生产区里 204 个
//! `"reason"` 键站点中有 **11 个**不是这个形状（变量 / `format!` / `map.insert` /
//! 外部枚举的 `as_str()`），旧实现**静默**漏掉它们 —— 守卫因此"绿"在看不见的地方。
//! 现在每次出现都被分类；`Dynamic`（运行时才定值）与 `ArgumentName`（只是参数名）
//! 必须逐条登记并写明退役条件，`Unclassified` 直接红并点名 file:line。
//! 新扫描器解析出了旧实现看不见的两条真实 reason：
//! `placementFieldNotApplicable` / `unknownPlacementField`（词表 144 → 146）。
//!
//! ## 登记为"本平台不可达"的那一个
//!
//! `indexTooLarge` 只在 `usize` 窄于 `u64` 的平台上可达
//! （`read_usize` 做 `usize::try_from(u64)`；64 位平台上这一步**永不失败**）。
//! 因此它在本机与 CI 的 64 位腿上不可观测，**如实登记**而不是硬造一条假判据。

//! ## 冻结摘要与文档表（R70②/R78④：两方向）
//!
//! `reason` 面的**规范文本**（文件按路径排序 → reason 按首次出现顺序 → 键按源码顺序）
//! 的 SHA-256 是下面这个字面量。判据 `the_reason_surface_digest_is_frozen_and_documented`
//! 同时核对**两个方向**：`sha256(规范文本) == 常量` **且** 本表 `contains(常量)`
//! （用 `include_str!` 把本文件读回来）—— 表与代码不许各说各话。
//!
//! | 面 | reason 数 | 规范行数 | SHA-256 |
//! | :--- | ---: | ---: | :--- |
//! | `reason`（**可分类**面：字面量 ＋ 可解析的字面量 `let` ＋ **调用点传播**，含顺序） | 207 | 906 | `4890ba25908a83444859edd023e7109b02eb5e33646f46a7b9633ae39443dd43` |
//!
//! ⚠ `ADR-0030`（`yeban_export_stems`）：两条新字面站点 `masterBusTrack` /
//! `notASourceNode`（各带 `trackId` / `name` / `reason` 三个同级键）进入规范文本
//! ⇒ 164 → 166 个 reason、742 → 750 行；**新摘要照抄判据自己的报出值**（本表里那一位
//! 与 `FROZEN_REASON_SURFACE_SHA256` 在同一次提交里一起换成新值 —— 两者不许各说各话）。
//! ⚠ 同一批还把生产区 `.rs` 文件分母从 **44** 推到 **45**（新增 `src/domain/stems.rs`）。
//!
//! ⚠ `#274`：摘要随"登记位置搬进 provider"重钉（集合不变、顺序变了）；
//! 见 `FROZEN_REASON_SURFACE_SHA256` 的沿革注释。
//! ⚠ `#340`：摘要**只覆盖可分类面**。运行时站点（`format!` / 转发入参 /
//! 外部枚举 `as_str()`）不贡献值，逐条登记在 `EXEMPT_DYNAMIC_REASON_SITES`。
//! ⚠ `#341`：三个 `as_str()` / `format!` 站点退役成 `match` 字面量 ⇒ 运行时站点
//! 9 → 6 处、可分类面 150 → 156 个 reason（摘要同批重钉）。

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use yeban_mcp::undo_session::read_rust_sources;

/// 本轮开工基线上**生产区**里**可分类**的全部 `reason` 值（排序）：
/// 字面量 ＋ 同函数内可解析的字面量 `let`（#340 起）＋ **调用点传播**出来的入参
/// （2026-10-28 起, 见 [`PROPAGATED_REASON_SITES`]）。⛔ 真看不见的运行时站点不进这张表，
/// 由 [`EXEMPT_DYNAMIC_REASON_SITES`] 逐条登记。
/// 这是黄金表：任何改名 / 新增 / 删除都必须**故意**改这一行。
pub const GOLDEN_REASONS: [&str; 203] = [
    "assetChannelLayout",
    "assetDecodeFailed",
    "assetHashMismatch",
    "assetMissing",
    "assetDeclaredDifferently",
    "assetInUse",
    "assetNotFound",
    "assetPatchIsEmpty",
    "assetNotInPool",
    "automationLaneNotFound",
    "automationPointNotFound",
    "automationTickAmbiguous",
    "bpmOutOfRange",
    // ⭐ `ADR-0019`：`setPitchBendPoint` 的 `cents` 只收整数音分。
    "centsMustBeInteger",
    // ⭐ MCP-TOOL-003：`setTempoPoint` / `setTimeSignaturePoint` 的三条字面站点
    // （tick 0 必拒、删除不存在的映射点必点名）。`unknownSetTempoPointField` /
    // `unknownSetTimeSignaturePointField` **不在**本表：它们作为参数传给
    // `reject_unknown_keys`，扫描器把它登记为**动态站点**（同族的
    // `unknownSetKeySignatureField` 等也是这个待遇）。
    "tempoMapZeroTick",
    "tempoPointNotFound",
    "timeSignaturePointNotFound",
    "clipAlreadyExists",
    "clipIdRequired",
    "createIsNotReadOnly",
    "createOnlyParameter",
    // ⭐ `ADR-0018`：`create: true` 与 `placement` 同给时，`kind` 只能是 `add`。
    "createPlacementKindMustBeAdd",
    "createRequiresAddOps",
    "deviceIdAlreadyInTrack",
    "deviceNameRequired",
    "deviceNotFound",
    "deviceSlotOutOfRange",
    "deviceTrackNotFound",
    "placementTrackNotFound",
    "clipPlacementNotFound",
    "domainMustBeObjectOrNull",
    "duplicateLaneTarget",
    "duplicateNoteId",
    "duplicateSceneTarget",
    "durationNotDerivable",
    "edgeIdRequired",
    // ⭐ `#268`（`ADR-0011`）：引擎音源装配失败（防御映射；合法工程上前面的
    // 探针编译会先拒绝同一批路由错误 ⇒ 本词主要用于"没有主总线 / 快照投影失败"）。
    "engineSource",
    // ⭐ `#290`：形态 A 的宿主口没有接上（形态 B / 引擎未起来）。
    "engineUnavailable",
    "framesExceedCap",
    "indexTooLarge",
    "invalidDeviceDefinition",
    // ⭐ `#290`：`sceneId` 不是合法 ULID。
    "invalidSceneId",
    "laneMustBeString",
    "lanePropertiesNotApplicableToPointRemoval",
    "latencyOutOfRange",
    "licenseNotAllowed",
    "licenseRequired",
    "licenseWhitelistUnavailable",
    "loudnessTargetMissed",
    "masterBusNodeCannotBeRemoved",
    "masterBusTrackCannotBeRemoved",
    // ⭐ `ADR-0030`：`yeban_export_stems` 的 D4 豁免点名（主总线 / 路由图里的非源节点轨）。
    "masterBusTrack",
    "notASourceNode",
    "midiInputAmbiguousPort",
    "midiInputConnectFailed",
    "midiInputEmptyPortName",
    "midiInputFeatureDisabled",
    "midiInputHostUnavailable",
    "midiInputNoPorts",
    "midiInputNoSuchPortIndex",
    "midiInputPortRequired",
    "midiInputPortTypeMismatch",
    "midiInputSecondsOutOfRange",
    "midiInputSecondsTypeMismatch",
    "midiInputUnknownPort",
    // ⭐ [G18]：`parse_scale` 的单边输入结构化拒绝（`domain/section_build.rs`）。
    "missingMode",
    "missingSpaceOrMode",
    "missingTonic",
    "moveRequiresPlacementId",
    "moveRequiresStartTick",
    "nonFiniteValue",
    "paramIndexOutOfRange",
    "pcmBudgetExceeded",
    // ⭐ `ADR-0019`：删除一个不存在的弯音点必须点名 tick（同 `tempoPointNotFound`）。
    "pitchBendPointNotFound",
    "placementAlreadyAtStartTick",
    "placementAlreadyExists",
    "placementClipMismatch",
    // ⭐ `#340`：这两条 reasons 一直在生产区里，只是旧扫描器**看不见** ——
    // 它们经 `"reason": reason` 给出（`let reason = ...`），旧实现只认
    // `"reason": "字面量"`。现在由可解析的 `let` 抽出，词表因此 144 → 146
    // （增的是**真实存在**的词，不是发明；两条都有 `src` 判据断言）。
    "placementFieldNotApplicable",
    "placementIdConflict",
    "placementNotFound",
    "placementWithoutTrack",
    "pointAddressIsAmbiguous",
    "pointAddressRequired",
    "projectAlreadyExists",
    "proposalNotFound",
    "recordDrainFailed",
    "removeClipIsNotPlacement",
    "removeClipTakesNoOtherOps",
    "removeRequiresPlacementId",
    "removeTakesNoProperties",
    "removeTrackIsNotPlacement",
    "removeTrackTakesNoOtherOps",
    "routingEdgeAlreadyExists",
    "routingEdgeNotFound",
    "routingNodeAlreadyExists",
    "routingNodeNotFound",
    "samplerUnreadable",
    "sceneAlreadyExists",
    // ⭐ `#290`：`sceneId` 缺失 / 队列满 / 场景 tempo 越界。
    "sceneIdRequired",
    "sceneLaunchQueueFull",
    "sceneNameRequiredWhenCreating",
    "sceneNotFound",
    "sceneTempoMustBeNumberOrNull",
    "sceneTempoOutOfRange",
    "secondsExceedCap",
    "sectionAlreadyExists",
    "sectionNameRequiredWhenCreating",
    "sectionTickMustBeUnsignedInteger",
    "sectionNotFound",
    "sfzInstrumentNotFound",
    "sfzNotUtf8",
    "sfzParseFailed",
    "sfzPathIsDirectory",
    "sfzReadFailed",
    "slotOutOfRange",
    "sourceAmbiguous",
    "sourceMissing",
    "staticLaneNotApplicable",
    "tempoUnusable",
    "trackNotFound",
    "unknownAddTrackField",
    "unknownAddRoutingNodeField",
    "unknownConnectRoutingField",
    "unknownDeviceField",
    "unknownDeviceKind",
    "unknownDeviceParamField",
    "unknownDisconnectRoutingField",
    "unknownEffectParam",
    "unknownFlagField",
    "unknownInsertDeviceField",
    "unknownLaneField",
    "unknownLaneTarget",
    "unknownLaunchQuantization",
    // ⭐ `#340`：同 `placementFieldNotApplicable`，经 `"reason": reason` 隐藏在
    // `let reason = if ... { ... } else { ... }` 后面；现在被可解析 `let` 抽出。
    "unknownPlacementField",
    "unknownPlacementKind",
    "unknownPointField",
    "unknownPointRemovalField",
    "unknownRemoveAssetField",
    "unknownRemoveClipField",
    "unknownRemoveDeviceField",
    "unknownRemoveRoutingNodeField",
    "unknownRemoveSceneField",
    "unknownRemoveSectionField",
    "unknownRemoveTrackField",
    "unknownRoutingGainField",
    "unknownSetAssetField",
    "unknownSetSceneField",
    "unknownSetSceneOpField",
    "unknownRoutingKind",
    "unknownStaticLane",
    "unknownTransportConfigField",
    "unknownWriteMode",
    "unknownTrackKind",
    "trackAlreadyExists",
    "valueMustBeBoolean",
    "valueMustBeNumberOrNull",
    "valueMustBeNumber",
    "valueMustBeNonEmptyString",
    "valueMustBeString",
    "valueMustBeUnsignedInteger",
    "valueOutOfRange",
    "writeModeMustBeString",
    // ⭐ **调用点传播片**（2026-10-28）：以下 41 条以前是**运行时盲区**（登记在
    // `EXEMPT_DYNAMIC_REASON_SITES`），现在由 [`PROPAGATED_REASON_SITES`] 从辅助函数的
    // **调用点实参**传播出来 ⇒ 进词表。25 个 `reject_unknown_keys` 的 `unknown*Field`
    // ＋ 8 个 `read_non_empty_string` 的空串理由 ＋ 8 个同参数的非字符串理由
    // （`…MustBeNonEmptyStringMustBeString`：入参本身已带后缀, 拼接又加一次 —— 真实输出, 如实登记）。
    "unknownKeySignatureField",
    "unknownSetAuthorField",
    "unknownSetClipGainField",
    "unknownSetClipNameField",
    "unknownSetClipPlacementDurationField",
    "unknownSetClipPlacementLoopField",
    "unknownSetClipPlacementMutedField",
    "unknownSetDeviceBypassedField",
    "unknownSetDeviceNameField",
    "unknownSetKeySignatureField",
    "unknownSetNoteExpressionField",
    "unknownSetNotePitchShapeField",
    "unknownSetNoteTimingField",
    "unknownSetPitchBendPointField",
    "unknownSetSectionField",
    "unknownSetSectionOpField",
    "unknownSetTempoField",
    "unknownSetTempoPointField",
    "unknownSetTimeSignatureField",
    "unknownSetTimeSignaturePointField",
    "unknownSetTitleField",
    "unknownSetTrackColorField",
    "unknownSetTrackNameField",
    "unknownSetTrackSamplerField",
    "unknownTransposeClipField",
    "assetLicenseMustBeNonEmptyString",
    "assetOriginalPathMustBeNonEmptyString",
    "sceneColorMustBeNonEmptyString",
    "sceneNameMustBeNonEmptyString",
    "sectionColorMustBeNonEmptyString",
    "sectionNameMustBeNonEmptyString",
    "trackColorMustBeNonEmptyString",
    "trackSamplerMustBeNonEmptyString",
    "assetLicenseMustBeNonEmptyStringMustBeString",
    "assetOriginalPathMustBeNonEmptyStringMustBeString",
    "sceneColorMustBeNonEmptyStringMustBeString",
    "sceneNameMustBeNonEmptyStringMustBeString",
    "sectionColorMustBeNonEmptyStringMustBeString",
    "sectionNameMustBeNonEmptyStringMustBeString",
    "trackColorMustBeNonEmptyStringMustBeString",
    "trackSamplerMustBeNonEmptyStringMustBeString",
];

/// 本平台（64 位 `usize`）**不可达**的 `reason`，逐条给机械理由。
///
/// 这张表要**刻意**维护：新增一个条目就是新增一处"没有判据能提到它"的登记。
pub const UNREACHABLE_ON_THIS_PLATFORM: [(&str, &str); 1] = [(
    "indexTooLarge",
    "`read_usize` 做 `usize::try_from(u64)`：64 位平台上 usize == u64 ⇒ 永不失败；\
     32 位平台上可达（因此不删这条防御，只登记）",
)];

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// 生产区 = 第一个以 `#[cfg(test)]` 开头的行**之前**（与 `undo_session::production_region`
/// 同一口径：整行开头匹配，不是 `find("#[cfg(test)]")`）。
fn production_region(text: &str) -> String {
    text.lines()
        .take_while(|line| !line.trim_start().starts_with("#[cfg(test)]"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 测试区 = 从第一个 `#[cfg(test)]` 行开始到文件末尾；没有则空串。
fn test_region(text: &str) -> String {
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if !inside && line.trim_start().starts_with("#[cfg(test)]") {
            inside = true;
        }
        if inside {
            out.push(line);
        }
    }
    out.join("\n")
}

/// [`reasons_in`] 的一次扫描结果。
///
/// ⚠ 为什么**必须**把"拒掉"也报出来：字符集过滤（见下）是**静默**的 ——
/// 一个带 `-` / `.` / `/` 的 reason 值会被**直接丢掉**，而不是变红。
/// 今天之所以还不算事故，只因为那条判据是**双向**的（丢一条 ⇒ `removed` 非空）；
/// 但"今天恰好被另一条判据兜住"⛔ 不等于"这里不静默"。
///
/// ⭐ 本仓口径：**代理失效 ⇒ 加强**（⛔ 不是放宽）。因此这里**保留**字符集，
/// 只把"拒掉了几条、分别是什么"变成**显式读数**，由判据
/// `the_reason_character_filter_rejects_nothing` 断言为 0。
struct Reasons {
    /// 通过字符集过滤的 reason（判据比对用的那个集合）。
    accepted: BTreeSet<String>,
    /// 被字符集**拒掉**的 reason 字面量（合法字母数字下划线之外）＋ 出处 ＋ 原因。
    rejected: Vec<(String, String, &'static str)>,
}

// ---------------------------------------------------------------------------
// #340：`"reason"` **站点**扫描器
// ---------------------------------------------------------------------------
//
// 旧实现只认 `"reason": "字面量"`：键后面必须**紧跟**一个字符串字面量。
// 于是这些真实形状被**静默忽略**（不是报错，是当它不存在）：
//
//   * `"reason": reason`（变量 / 函数参数 / 枚举 `as_str()`）
//   * `"reason": format!("{kind}RequiresPlacementId")`
//   * `preview.insert("reason".to_owned(), Value::from(reason.clone()))`
//   * `"reason": error.as_str()`
//
// 一个看不见的站点会让"词表一致"变成假绿。现在每个 `"reason"` 出现都必须落到
// 下面**六类**之一；`Dynamic`（运行时才算得出来）与 `ArgumentName`（只是参数名）
// 必须**逐条**登记，`Unclassified`（没有已知形状）直接判红并点名 file:line。
//
// ⭐ 2026-10-28 的**调用点传播片**把第三类 `Propagated` 补了进来：辅助函数把**自己的
// 入参**转给 `json!` 时, 值不再算"运行时才定" —— 扫描器读**调用点**的最后一个实参
// 就能定值（口径与限制见 [`PROPAGATED_REASON_SITES`] 上方那段）。

/// 一次 `"reason"` 出现的分类结果。
#[derive(Debug, Clone, PartialEq, Eq)]
enum ReasonOccurrenceKind {
    /// 值表达式是字符串字面量（直接写，或经 `Value::from(...)`）。
    Literal(String),
    /// 值由同一个函数里的 `let` 绑定，且初始式是单个字面量或 `if cond { "a" } else { "b" }`。
    Resolved(Vec<String>),
    /// 值是运行时算出来的（转发入参 / `format!` / 外部枚举的 `as_str()`），扫描器看不见。
    Dynamic(String),
    /// ⭐ 值是**调用点送进来的字面量入参**：本文件按辅助函数名把它**传播**回站点
    /// （见 [`PROPAGATED_REASON_SITES`]）。字段＝（辅助函数名, 固定后缀, 值集合）。
    /// 与 `Resolved` 的区别：那个的值在**同一个函数**里，这个的值来自**别的函数**的调用点。
    Propagated(String, String, Vec<String>),
    /// `"reason"` 是**参数名 / 入参键名**，不是 `data.reason` 的值键。
    ArgumentName(String),
    /// 没有任何已知形状 ⇒ 判据必须红（带 file:line）。
    Unclassified(String),
}

impl ReasonOccurrenceKind {
    /// 这条站点贡献的 reason 值；看不见的值**不贡献**（由 census 判据点名）。
    fn values(&self) -> Vec<String> {
        match self {
            Self::Literal(value) => vec![value.clone()],
            Self::Resolved(values) => values.clone(),
            Self::Propagated(_helper, _suffix, values) => values.clone(),
            Self::Dynamic(_) | Self::ArgumentName(_) | Self::Unclassified(_) => Vec::new(),
        }
    }
}

/// 生产区里一次 `"reason"` 出现：位置 ＋ 分类。
struct ReasonOccurrence {
    path: String,
    line: usize,
    offset: usize,
    kind: ReasonOccurrenceKind,
}

/// 把绝对路径折成 `src/` 之后的相对路径（豁免表按它匹配）。
fn relative_to_src(path: &str) -> String {
    // ⚠ **必须先归一化分隔符**：`Path` 在 Windows 上渲染成 `D:\a\...\src\domain\notes.rs`，
    // 而登记表用的是正斜杠相对路径（`domain/notes.rs`）。少了这一步，`rfind("/src/")` 返回
    // `None` ⇒ 退化成**整条绝对路径** ⇒ 每一条登记都对不上 ⇒ 普查在 Windows 上**全红**，
    // 而在 macOS/Linux 上全绿（2026-10-26 实测：`#340` 合并后 Windows 腿红了三次，
    // 而本地与 Linux 一直是绿的）。这类缺陷**本机永远抓不到**，所以判据必须喂它一条
    // 合成的 Windows 路径（见 `relative_to_src_normalises_windows_separators`）。
    let normalized = path.replace('\\', "/");
    normalized.rfind("/src/").map_or_else(
        || normalized.clone(),
        |at| normalized[at + "/src/".len()..].to_owned(),
    )
}

/// `relative_to_src` 在**两种分隔符**下必须给出同一个相对路径。
///
/// ⚠ 这条判据存在的唯一目的，是让**本机**（macOS/Linux）抓住一个**只在 Windows 上**
/// 出现的缺陷。`#340` 的普查在 Windows 上全红、在本机与 Linux 上全绿 —— 因为 `Path`
/// 在 Windows 上渲染成反斜杠，而登记表用正斜杠。本机跑不到 Windows ⇒ **喂它一条合成的
/// Windows 路径**，把平台差异变成一条不依赖平台的断言。
#[test]
fn relative_to_src_normalises_windows_separators() {
    assert_eq!(
        relative_to_src("/repo/crates/yeban-mcp/src/domain/notes.rs"),
        "domain/notes.rs",
        "正斜杠路径"
    );
    assert_eq!(
        relative_to_src(r"D:\a\yeban\yeban\crates\yeban-mcp\src\domain\notes.rs"),
        "domain/notes.rs",
        "Windows 绝对路径必须与正斜杠路径给出同一个相对路径"
    );
    assert_eq!(
        relative_to_src(r"crates\yeban-mcp\src\tools.rs"),
        "tools.rs",
        "Windows 相对路径"
    );
    // 不含 `/src/` 的路径：归一化后原样返回，⛔ panic、⛔ 不吞
    assert_eq!(relative_to_src(r"D:\a\b\Cargo.toml"), "D:/a/b/Cargo.toml");
}

/// 扫出 `"reason"` 的**每一次**出现并分类（注释／字符串内部不算）。
fn reason_occurrences(
    path: &str,
    text: &str,
    consts: &BTreeMap<String, String>,
) -> Vec<ReasonOccurrence> {
    let bytes = text.as_bytes();
    let needle = b"\"reason\"";
    let mask = skipped_offsets(text);
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(offset) = find(&bytes[from..], needle) {
        let at = from + offset;
        from = at + 1;
        // ⭐ R94：命中的 `"reason"` 若落在注释／字符串**内部** ⇒ 跳过。
        if is_skipped(&mask, at) {
            continue;
        }
        let mut cursor = at + needle.len();
        while cursor < bytes.len() && (bytes[cursor] == b' ' || bytes[cursor] == b'\t') {
            cursor += 1;
        }
        let kind = if bytes.get(cursor) == Some(&b':') {
            classify_value_expr(take_value_expr(text, cursor + 1), &text[..at], text, consts)
        } else if text[cursor..].starts_with("=>") {
            // 样本夹具按**参数名** `reason` 匹配，右侧是夹具入参，不是 `data.reason`。
            ReasonOccurrenceKind::ArgumentName(trimmed_line(text, at))
        } else if text[cursor..].starts_with(".to_owned()") {
            let mut next = cursor + ".to_owned()".len();
            while next < bytes.len() && bytes[next].is_ascii_whitespace() {
                next += 1;
            }
            if bytes.get(next) == Some(&b',') {
                classify_value_expr(take_value_expr(text, next + 1), &text[..at], text, consts)
            } else {
                ReasonOccurrenceKind::Unclassified(trimmed_line(text, at))
            }
        } else if bytes.get(cursor) == Some(&b',') && text[..at].trim_end().ends_with("insert(") {
            classify_value_expr(take_value_expr(text, cursor + 1), &text[..at], text, consts)
        } else if bytes.get(cursor) == Some(&b']') {
            let mut next = cursor + 1;
            while next < bytes.len() && (bytes[next] == b' ' || bytes[next] == b'\t') {
                next += 1;
            }
            if bytes.get(next) == Some(&b'=') && bytes.get(next + 1) != Some(&b'=') {
                classify_value_expr(take_value_expr(text, next + 1), &text[..at], text, consts)
            } else {
                ReasonOccurrenceKind::ArgumentName(trimmed_line(text, at))
            }
        } else if matches!(bytes.get(cursor), Some(&b',') | Some(&b')') | Some(&b';')) {
            ReasonOccurrenceKind::ArgumentName(trimmed_line(text, at))
        } else {
            ReasonOccurrenceKind::Unclassified(trimmed_line(text, at))
        };
        out.push(ReasonOccurrence {
            path: path.to_owned(),
            line: line_number(text, at),
            offset: at,
            kind,
        });
    }
    out
}

/// 从 `start` 起取一个**值表达式**：到顶层 `,` / `;` / 闭括号 / 换行为止。
fn take_value_expr(text: &str, start: usize) -> &str {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut index = start;
    let mut in_string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'(' | b'{' | b'[' => depth += 1,
            b')' | b'}' | b']' => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            b',' | b';' if depth == 0 => break,
            b'\n' if depth == 0 => break,
            _ => {}
        }
        index += 1;
    }
    text[start..index].trim()
}

/// 给值表达式分类；看不见的值统一带上**规范化后的原表达式**（豁免表按它匹配）。
///
/// ⭐ `region` 是**整段被扫描的源码**（`prefix` 只有站点**之前**的部分）：
/// 转发入参要靠**调用点**才能定值，而调用点通常在站点的**后面**。少了这一位，
/// `"reason": reason` 只能永远登记成运行时盲区（`#340` 的 `EXEMPT_DYNAMIC_REASON_SITES`）。
fn classify_value_expr(
    expr: &str,
    prefix: &str,
    region: &str,
    consts: &BTreeMap<String, String>,
) -> ReasonOccurrenceKind {
    match classify_value_inner(expr, prefix, region, consts) {
        ReasonOccurrenceKind::Dynamic(_) => ReasonOccurrenceKind::Dynamic(normalize_expr(expr)),
        other => other,
    }
}

fn classify_value_inner(
    expr: &str,
    prefix: &str,
    region: &str,
    consts: &BTreeMap<String, String>,
) -> ReasonOccurrenceKind {
    let expr = expr.trim();
    if let Some(inner) = direct_value_from_argument(expr) {
        return classify_value_inner(inner, prefix, region, consts);
    }
    // ⭐ 唯一的**组合**形状 `format!("{<入参>}<固定后缀>")`：后缀是字面量、入参可传播
    // ⇒ 值集合 = 调用点字面量各加同一个后缀。其余 `format!` 仍然看不见（⇒ `Dynamic`）。
    if let Some((ident, suffix)) = format_reason_composition(expr)
        && let Some((helper, values)) = propagated_reasons(prefix, region, ident, suffix)
    {
        return ReasonOccurrenceKind::Propagated(helper.to_owned(), suffix.to_owned(), values);
    }
    if expr.starts_with("format!(") {
        return ReasonOccurrenceKind::Dynamic(String::new());
    }
    if expr.starts_with('"') {
        if let Some((value, consumed)) = parse_string_literal(expr)
            && expr[consumed..].trim().is_empty()
        {
            return ReasonOccurrenceKind::Literal(value);
        }
        return ReasonOccurrenceKind::Dynamic(String::new());
    }
    let ident_end = expr
        .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .unwrap_or(expr.len());
    let ident = &expr[..ident_end];
    if !is_simple_ident(ident) {
        return ReasonOccurrenceKind::Dynamic(String::new());
    }
    let rest = expr[ident_end..].trim();
    if let Some(method) = rest.strip_prefix('.') {
        let method = method.trim_start();
        if let Some(after) = method
            .strip_prefix("clone()")
            .or_else(|| method.strip_prefix("to_owned()"))
            && after.trim().is_empty()
        {
            return resolve_reason_ident(ident, prefix, region, consts);
        }
        return ReasonOccurrenceKind::Dynamic(String::new());
    }
    if !rest.is_empty() {
        return ReasonOccurrenceKind::Dynamic(String::new());
    }
    resolve_reason_ident(ident, prefix, region, consts)
}

/// `Value::from(<arg>)` ⇒ `<arg>`（只接受**整个表达式**就是这一次调用）。
fn direct_value_from_argument(expr: &str) -> Option<&str> {
    let inner = expr.strip_prefix("Value::from(")?.strip_suffix(')')?;
    let inner = inner.trim();
    (!inner.is_empty()).then_some(inner)
}

/// 常量优先，其次同函数内的字面量 `let`，再次**调用点传播**；都不是 ⇒ `Dynamic`（⛔ 不猜）。
fn resolve_reason_ident(
    ident: &str,
    prefix: &str,
    region: &str,
    consts: &BTreeMap<String, String>,
) -> ReasonOccurrenceKind {
    if let Some(value) = consts.get(ident) {
        return ReasonOccurrenceKind::Literal(value.clone());
    }
    if let Some(values) = local_literal_binding(prefix, ident) {
        return ReasonOccurrenceKind::Resolved(values);
    }
    if let Some((helper, values)) = propagated_reasons(prefix, region, ident, "") {
        return ReasonOccurrenceKind::Propagated(helper.to_owned(), String::new(), values);
    }
    ReasonOccurrenceKind::Dynamic(String::new())
}

/// 解析一个 Rust 字符串字面量（含转义），返回（值, 消耗的字节数）。
fn parse_string_literal(text: &str) -> Option<(String, usize)> {
    let mut chars = text.char_indices();
    let (_, first) = chars.next()?;
    if first != '"' {
        return None;
    }
    let mut out = String::new();
    while let Some((index, ch)) = chars.next() {
        match ch {
            '"' => return Some((out, index + 1)),
            '\\' => {
                let (_, escaped) = chars.next()?;
                match escaped {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    other => out.push(other),
                }
            }
            other => out.push(other),
        }
    }
    None
}

fn normalize_expr(expr: &str) -> String {
    expr.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn line_number(text: &str, at: usize) -> usize {
    text[..at].bytes().filter(|byte| *byte == b'\n').count() + 1
}

fn trimmed_line(text: &str, at: usize) -> String {
    let start = text[..at].rfind('\n').map_or(0, |index| index + 1);
    let end = text[at..].find('\n').map_or(text.len(), |index| at + index);
    text[start..end].trim().to_owned()
}

fn is_simple_ident(value: &str) -> bool {
    let mut bytes = value.bytes();
    match bytes.next() {
        Some(first) if first.is_ascii_alphabetic() || first == b'_' => {}
        _ => return false,
    }
    bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// 最近的同名 `let`（同一函数内）若初始式**只由字面量组成**，返回其中的字面量。
///
/// 只接受两种形状：①单个字面量；②`if <ident> { "a" } else { "b" }`。
/// 其余（调用、`format!`、变量转发）一律算 `Dynamic` —— 宁可红，⛔ 不猜。
fn local_literal_binding(prefix: &str, ident: &str) -> Option<Vec<String>> {
    let needle = format!("let {ident}");
    let mut found: Option<Vec<String>> = None;
    let mut from = 0usize;
    while let Some(offset) = prefix[from..].find(&needle) {
        let at = from + offset;
        from = at + needle.len();
        let after = at + needle.len();
        let boundary_ok = prefix[after..]
            .chars()
            .next()
            .is_none_or(|ch| !(ch.is_ascii_alphanumeric() || ch == '_'));
        if !boundary_ok {
            continue;
        }
        if crosses_function_boundary(prefix, at) {
            // ⚠ 这里曾经是 `break`：先遇到**上一个函数**里的同名 `let` 就停，
            // 于是站点所在函数里的 `let`（在后面的行）永远轮不到 ⇒
            // `unknownPlacementField` 被静默漏掉（#340 第一轮实测红）。
            // 正确的做法是**跳过**跨函数的候选，继续往后找到同函数的那一个。
            continue;
        }
        // 最近的同名绑定说了算；它是动态的 ⇒ ⛔ 不许回退到更早的字面量绑定。
        found = let_initializer(prefix, after).and_then(literal_only_values);
    }
    found
}

fn crosses_function_boundary(prefix: &str, at: usize) -> bool {
    prefix[at..].lines().skip(1).any(|line| {
        let line = line.trim_start();
        line.starts_with("fn ")
            || line.starts_with("pub fn ")
            || line.starts_with("pub(crate) fn ")
            || line.starts_with("async fn ")
            || line.starts_with("const fn ")
    })
}

/// `let <ident> [ : 类型 ] = <初始式> ;` ⇒ 初始式。
fn let_initializer(prefix: &str, after_ident: usize) -> Option<&str> {
    let bytes = prefix.as_bytes();
    let mut index = after_ident;
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    if bytes.get(index) == Some(&b':') {
        // 跳过类型标注（类型里不会出现 `=`）。
        index += 1;
        while index < bytes.len() && bytes[index] != b'=' {
            index += 1;
        }
    }
    if bytes.get(index) != Some(&b'=') {
        return None;
    }
    index += 1;
    let start = index;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'(' | b'{' | b'[' => depth += 1,
            b')' | b'}' | b']' => depth -= 1,
            b';' if depth == 0 => return Some(prefix[start..index].trim()),
            _ => {}
        }
        index += 1;
    }
    None
}

fn literal_only_values(init: &str) -> Option<Vec<String>> {
    let init = init.trim();
    if init.starts_with('"') {
        let (value, consumed) = parse_string_literal(init)?;
        return init[consumed..].trim().is_empty().then(|| vec![value]);
    }
    let rest = init.strip_prefix("if ")?;
    let brace = rest.find('{')?;
    if !is_simple_ident(rest[..brace].trim()) {
        return None;
    }
    let (first, consumed) = take_braced_block(&rest[brace..])?;
    let after = rest[brace + consumed..].trim_start();
    let after = after.strip_prefix("else")?.trim_start();
    let (second, consumed) = take_braced_block(after)?;
    if !after[consumed..].trim().is_empty() {
        return None;
    }
    let mut values = literal_block_values(first)?;
    values.extend(literal_block_values(second)?);
    Some(values)
}

fn take_braced_block(text: &str) -> Option<(&str, usize)> {
    let bytes = text.as_bytes();
    if bytes.first() != Some(&b'{') {
        return None;
    }
    let mut depth = 0i32;
    let mut index = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&text[1..index], index + 1));
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn literal_block_values(block: &str) -> Option<Vec<String>> {
    let block = block.trim();
    let (value, consumed) = parse_string_literal(block)?;
    block[consumed..].trim().is_empty().then(|| vec![value])
}

// ---------------------------------------------------------------------------
// ⭐ 调用点传播：把"转发入参"的 reason 从**盲区**折回**可分类面**
// ---------------------------------------------------------------------------
//
// `EXEMPT_DYNAMIC_REASON_SITES` 原来登记着两类站点：辅助函数把**自己的入参**原样
// 转给 `json!`，或 `format!("{reason}MustBeString")` 在入参后面拼固定后缀。
// 扫描器只看站点**之前**的文本（`prefix`），因此调用点在后面时它什么都看不见。
// 本节的口径：
//
// * 站点所在函数的**最后一个形参**必须就是那个入参名（⛔ 不是别的同名变量）；
// * 该辅助函数在**同一份被扫描文本**里的每个调用点，最后一个实参必须是**字符串字面量**；
// * 表里登记的 `call_sites` / `literals` 是**全 crate 生产区**的读数，由
//   `the_propagated_reason_sites_publish_their_call_site_literals` 逐条核对
//   ⇒ 将来在**别的文件**里加一个调用点，本文件的调用点数就对不上 ⇒ 站点退回
//   `Dynamic` ⇒ 普查立刻点名"未登记的动态站点"（⛔ 不是静默少几个值）。

/// 一条**由调用点传播**的 `"reason"` 站点（`#340` 的退役成果：从盲区清单搬进可分类面）。
struct PropagatedReasonSite {
    /// `src/` 之后的路径。
    file: &'static str,
    /// **规范化**后的值表达式（与 `EXEMPT_DYNAMIC_REASON_SITES` 同一口径, 便于对照）。
    expr: &'static str,
    /// 该表达式在本文件里**传播成功**的站点数。
    occurrences: usize,
    /// 承载这个入参的辅助函数名。
    helper: &'static str,
    /// `format!` 的固定后缀；直接转发 ⇒ 空串。
    suffix: &'static str,
    /// 该辅助函数在**全 crate 生产区**的调用点数。
    call_sites: usize,
    /// 每个调用点的最后一个实参（字面量）加上 `suffix` 之后的**全部不重复值**（排序）
    /// ＝ 这一条站点贡献的 reason 词表。
    literals: &'static [&'static str],
}

/// ⭐ 实测 **3 个站点 / 2 个辅助函数 / 33 个调用点**（2026-10-28，`#340` 传播片开工现读）。
///
/// 这三条以前登记在 `EXEMPT_DYNAMIC_REASON_SITES`（运行时盲区），现在值**可分类**：
/// 词表因此从 162 涨到 **203**（＋25 个 `unknown*Field` ＋8 个 `*MustBeNonEmptyString`
/// ＋8 个 `*MustBeNonEmptyStringMustBeString`）。
///
/// ⚠ 那 8 个 `…MustBeString` 是**真实运行时输出**：`read_non_empty_string` 的入参本身
/// 已经带 `MustBeNonEmptyString` 后缀，非字符串分支又在后面拼 `MustBeString` ⇒
/// `sceneNameMustBeNonEmptyStringMustBeString`。本片只**如实登记**，⛔ 不改这些词
/// （改了就是改对客户端可见的故障契约，另案）。
const PROPAGATED_REASON_SITES: [PropagatedReasonSite; 3] = [
    PropagatedReasonSite {
        file: "domain/notes.rs",
        expr: "reason",
        occurrences: 1,
        helper: "reject_unknown_keys",
        suffix: "",
        call_sites: 25,
        literals: &[
            "unknownKeySignatureField",
            "unknownSetAuthorField",
            "unknownSetClipGainField",
            "unknownSetClipNameField",
            "unknownSetClipPlacementDurationField",
            "unknownSetClipPlacementLoopField",
            "unknownSetClipPlacementMutedField",
            "unknownSetDeviceBypassedField",
            "unknownSetDeviceNameField",
            "unknownSetKeySignatureField",
            "unknownSetNoteExpressionField",
            "unknownSetNotePitchShapeField",
            "unknownSetNoteTimingField",
            "unknownSetPitchBendPointField",
            "unknownSetSectionField",
            "unknownSetSectionOpField",
            "unknownSetTempoField",
            "unknownSetTempoPointField",
            "unknownSetTimeSignatureField",
            "unknownSetTimeSignaturePointField",
            "unknownSetTitleField",
            "unknownSetTrackColorField",
            "unknownSetTrackNameField",
            "unknownSetTrackSamplerField",
            "unknownTransposeClipField",
        ],
    },
    PropagatedReasonSite {
        file: "domain/notes.rs",
        expr: "reason",
        occurrences: 1,
        helper: "read_non_empty_string",
        suffix: "",
        call_sites: 8,
        literals: &[
            "assetLicenseMustBeNonEmptyString",
            "assetOriginalPathMustBeNonEmptyString",
            "sceneColorMustBeNonEmptyString",
            "sceneNameMustBeNonEmptyString",
            "sectionColorMustBeNonEmptyString",
            "sectionNameMustBeNonEmptyString",
            "trackColorMustBeNonEmptyString",
            "trackSamplerMustBeNonEmptyString",
        ],
    },
    PropagatedReasonSite {
        file: "domain/notes.rs",
        expr: "format!(\"{reason}MustBeString\")",
        occurrences: 1,
        helper: "read_non_empty_string",
        suffix: "MustBeString",
        call_sites: 8,
        literals: &[
            "assetLicenseMustBeNonEmptyStringMustBeString",
            "assetOriginalPathMustBeNonEmptyStringMustBeString",
            "sceneColorMustBeNonEmptyStringMustBeString",
            "sceneNameMustBeNonEmptyStringMustBeString",
            "sectionColorMustBeNonEmptyStringMustBeString",
            "sectionNameMustBeNonEmptyStringMustBeString",
            "trackColorMustBeNonEmptyStringMustBeString",
            "trackSamplerMustBeNonEmptyStringMustBeString",
        ],
    },
];

/// 整行开头是不是**函数声明**（与 `crosses_function_boundary` 同一口径）。
fn is_function_head(trimmed: &str) -> bool {
    [
        "fn ",
        "pub fn ",
        "pub(crate) fn ",
        "pub async fn ",
        "async fn ",
        "const fn ",
    ]
    .iter()
    .any(|prefix| trimmed.starts_with(*prefix))
}

/// `format!("{<ident>}<固定后缀>")` ⇒ (`ident`, 后缀)。
///
/// 只认**这一个**形状：`}` 之后到闭合引号之间⛔ 再出现 `{` / `}`。
/// 其余 `format!`（多个插值、插值里带表达式）仍然看不见 ⇒ 照旧 `Dynamic`。
fn format_reason_composition(expr: &str) -> Option<(&str, &str)> {
    let body = expr.strip_prefix("format!(\"{")?.strip_suffix("\")")?;
    let (ident, suffix) = body.split_once('}')?;
    if !is_simple_ident(ident) || suffix.contains('{') || suffix.contains('}') {
        return None;
    }
    Some((ident, suffix))
}

/// 从 `open`（必须指向 `(`）起做括号配对，返回（平衡内容, 闭括号之后的下标）。
fn balanced_parens(text: &str, open: usize) -> Option<(&str, usize)> {
    let bytes = text.as_bytes();
    if bytes.get(open) != Some(&b'(') {
        return None;
    }
    let mut depth = 0i32;
    let mut index = open;
    let mut in_string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'(' | b'{' | b'[' => depth += 1,
            b')' | b'}' | b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&text[open + 1..index], index + 1));
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

/// 顶层逗号切分（括号／字符串内部的逗号不算分隔）。
fn split_top_level_arguments(content: &str) -> Vec<&str> {
    let bytes = content.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut index = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'(' | b'{' | b'[' => depth += 1,
            b')' | b'}' | b']' => depth -= 1,
            b',' if depth == 0 => {
                out.push(content[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
        index += 1;
    }
    let tail = content[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

/// 站点**所在函数**的声明（名字 ＋ 形参表原文）：`prefix` 里最后一个整行开头的 `fn`。
fn enclosing_function(prefix: &str) -> Option<(&str, &str)> {
    let mut hit: Option<usize> = None;
    let mut offset = 0usize;
    for line in prefix.lines() {
        if is_function_head(line.trim_start()) {
            hit = Some(offset);
        }
        offset += line.len() + 1;
    }
    let tail = &prefix[hit?..];
    let after_keyword = "fn ".len() + tail.find("fn ")?;
    let rest = &tail[after_keyword..];
    let open = rest.find('(')?;
    let name = rest[..open].trim();
    is_simple_ident(name).then_some(())?;
    let (params, _) = balanced_parens(rest, open)?;
    Some((name, params))
}

/// 一次 `helper(...)` 调用点：位置、最后一个实参原文、解析出的字面量。
struct HelperCallSite {
    line: usize,
    argument: String,
    literal: Option<String>,
}

/// `region` 里对 `helper` 的**全部调用点**（声明行、注释、字符串内部都不算）。
fn helper_call_sites(region: &str, helper: &str) -> Vec<HelperCallSite> {
    let mask = skipped_offsets(region);
    // ⚠ 形状检查必须看**整行**：`region[..at]` 在声明行只剩 `"fn "`（针从名字开始）,
    // 用那一段判形状 ⇒ 声明被当成调用点（实测 26 = 25 真调用 ＋ 1 声明 ⇒ 传播整体失效）。
    let needle = format!("{helper}(");
    let (bytes, needle) = (region.as_bytes(), needle.as_bytes());
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(offset) = find(&bytes[from..], needle) {
        let at = from + offset;
        from = at + 1;
        if is_skipped(&mask, at) {
            continue;
        }
        if is_function_head(&trimmed_line(region, at)) {
            continue;
        }
        let Some((content, _)) = balanced_parens(region, at + helper.len()) else {
            continue;
        };
        let Some(argument) = split_top_level_arguments(content).last().copied() else {
            continue;
        };
        let literal = match parse_string_literal(argument) {
            Some((value, consumed)) if argument[consumed..].trim().is_empty() => Some(value),
            _ => None,
        };
        out.push(HelperCallSite {
            line: line_number(region, at),
            argument: normalize_expr(argument),
            literal,
        });
    }
    out
}

/// ⭐ 由调用点把转发入参 `ident` 折成它的值集合；任一条契约不满足 ⇒ `None`（⇒ 仍算 `Dynamic`）。
fn propagated_reasons(
    prefix: &str,
    region: &str,
    ident: &str,
    suffix: &str,
) -> Option<(&'static str, Vec<String>)> {
    let (helper, params) = enclosing_function(prefix)?;
    let entry = PROPAGATED_REASON_SITES
        .iter()
        .find(|entry| entry.helper == helper && entry.suffix == suffix)?;
    let last_parameter = *split_top_level_arguments(params).last()?;
    let parameter_name = last_parameter.split(':').next()?.trim();
    if parameter_name != ident {
        return None;
    }
    let sites = helper_call_sites(region, helper);
    if sites.len() != entry.call_sites || sites.iter().any(|site| site.literal.is_none()) {
        return None;
    }
    let mut values: Vec<String> = sites
        .iter()
        .filter_map(|site| site.literal.clone())
        .map(|value| format!("{value}{suffix}"))
        .collect();
    values.sort();
    values.dedup();
    if values.len() != entry.literals.len() {
        return None;
    }
    Some((entry.helper, values))
}

/// 生产区里 `helper` 的**声明**形参表原文（⛔ 调用点、注释、字符串内部都不算）。
fn function_parameters<'a>(region: &'a str, helper: &str) -> Option<&'a str> {
    let mask = skipped_offsets(region);
    let needle = format!("{helper}(");
    let (bytes, needle) = (region.as_bytes(), needle.as_bytes());
    let mut from = 0usize;
    while let Some(offset) = find(&bytes[from..], needle) {
        let at = from + offset;
        from = at + 1;
        if is_skipped(&mask, at) {
            continue;
        }
        if !is_function_head(&trimmed_line(region, at)) {
            continue;
        }
        let (params, _) = balanced_parens(region, at + helper.len())?;
        return Some(params);
    }
    None
}

/// 生产区里的 `const NAME: &str = "字面量";`（`pub` / `pub(crate)` 也收）。
///
/// ⚠ 按**名字**解析、不看模块路径：两个模块里同名常量会取后者。本仓今天没有
/// reason 由常量给出（这条路径由 `the_reason_site_scanner_classifies_every_known_shape`
/// 的合成样本覆盖）；真出现同名歧义时，把它改成带模块路径的键。
fn string_constants(files: &[(String, String)]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (_path, text) in files {
        for line in production_region(text).lines() {
            let line = line.trim();
            let body = line
                .strip_prefix("pub(crate) const ")
                .or_else(|| line.strip_prefix("pub const "))
                .or_else(|| line.strip_prefix("const "));
            let Some(body) = body else { continue };
            let Some((name, rest)) = body.split_once(':') else {
                continue;
            };
            if !is_simple_ident(name.trim()) {
                continue;
            }
            let Some(eq) = rest.find('=') else { continue };
            let value_expr = rest[eq + 1..].trim();
            let Some((value, consumed)) = parse_string_literal(value_expr) else {
                continue;
            };
            if value_expr[consumed..].trim() == ";" {
                out.insert(name.trim().to_owned(), value);
            }
        }
    }
    out
}

/// 抽出全部**可分类**的 reason 值，并**分别**报出接受与拒掉的数量。
///
/// 看不见的站点（`Dynamic` / `ArgumentName`）在这里不贡献值；由
/// `every_reason_occurrence_is_classified_or_explicitly_registered` 逐条点名。
fn reasons_in(source: &str, text: &str, consts: &BTreeMap<String, String>) -> Reasons {
    let mut accepted = BTreeSet::new();
    let mut rejected: Vec<(String, String, &'static str)> = Vec::new();
    for occurrence in reason_occurrences(source, text, consts) {
        for value in occurrence.kind.values() {
            if !value.is_empty()
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                accepted.insert(value);
            } else {
                let why = if value.is_empty() {
                    "emptyReasonValue"
                } else {
                    "reasonValueOutsideTheAlphanumericUnderscoreSet"
                };
                rejected.push((value, source.to_owned(), why));
            }
        }
    }
    Reasons { accepted, rejected }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len())
        .find(|index| &haystack[*index..*index + needle.len()] == needle)
}

// ---------------------------------------------------------------------------
// #340：看不见的站点**逐条登记**（⛔ 不是静默过滤）
// ---------------------------------------------------------------------------

/// 一条**运行时才算得出来**的 `"reason"` 站点。
///
/// 这些站点的值不是字面量：可能是**调用方送进来的入参**、`format!` 拼出来的、
/// 或**另一个 crate 的枚举**回答的。扫描器不猜 —— 逐条登记，并由
/// [`every_reason_occurrence_is_classified_or_explicitly_registered`] 断言
/// "实测站点集合 == 这张表**逐条**相等"（多一个、少一个、次数变了都红）。
struct DynamicReasonSite {
    /// `src/` 之后的路径（如 `domain/notes.rs`）。
    file: &'static str,
    /// **规范化**后的值表达式（空白折叠成单空格）。
    expr: &'static str,
    /// 该表达式在本文件出现的次数。
    occurrences: usize,
    /// 值从哪来（为什么静态看不见）。
    forwards: &'static str,
    /// 什么改动能让这一条**退役**（从表里删掉）。
    retires_when: &'static str,
}

/// 生产区实测 **3 次出现 / 3 个 distinct 表达式**（2026-10-28：`reject_unknown_keys` /
/// `read_non_empty_string` 的 3 处转发站点已改由 [`PROPAGATED_REASON_SITES`] 传播出值 ⇒
/// 6 → 3；剩下的每一条都是**真的**静态看不见）。
///
/// ⚠ 这张表是**显式盲区清单**，不是"这些值不重要"：每一条都写清值从哪来、怎么退役。
/// ⛔ 不许为了让判据变绿而把新的动态站点塞进来 —— 能给出常量/枚举的，先给出名字。
const EXEMPT_DYNAMIC_REASON_SITES: [DynamicReasonSite; 3] = [
    DynamicReasonSite {
        file: "domain/mod.rs",
        expr: "Value::from(reason.clone())",
        occurrences: 1,
        forwards: "`Plan::Reject` 预览里的 `reason`：这是 `yeban_reject_proposal` **调用方送进来的\
                   入参**（用户数据），不是本层产出的故障词。",
        retires_when: "把 `Plan::Reject.reason` 换成闭合枚举（每个值一个常量）；在此之前它没有\
                       可冻结的词汇表。",
    },
    DynamicReasonSite {
        file: "domain/mod.rs",
        expr: "reason",
        occurrences: 1,
        forwards: "`apply_reject` 的 `reason` 参数：同上，是**入库的用户数据**，经响应原样返回。",
        retires_when: "同上：改成闭合枚举/常量，而不是自由字符串。",
    },
    DynamicReasonSite {
        file: "domain/notes.rs",
        expr: "reason",
        occurrences: 1,
        forwards: "`resolved.unresolved_reason()`：词由**别的 crate**（`yeban_render::inserts::\
                   UnresolvedReason`）的 `as_str()` 给出。它的三个取值今天**已经**以字面量站点\
                   出现在词表里（`trackNotFound` / `slotOutOfRange` / `paramIndexOutOfRange`），\
                   但本站点静态看不见 —— 传播需要跨 crate 的枚举读数，不在本片边界内。",
        retires_when: "扫描器能解析跨 crate 的 `enum ...::as_str()` 匹配表时退役（届时这三个值\
                       的贡献已由字面量站点覆盖, 本条只是重复）。",
    },
];

/// 一处 `"reason"` 只是**参数名 / 入参键名**（⛔ 不是 `data.reason` 值键）的出现。
struct ReasonArgumentNameUse {
    file: &'static str,
    /// 该行里必须出现的稳定片段（同一行可能有别的 `"reason"`）。
    marker: &'static str,
    occurrences: usize,
    why: &'static str,
}

/// 生产区实测 **3 处**（2026-10-25，#340 开工）：它们都不是故障数据的 reason 值。
const REASON_ARGUMENT_NAME_USES: [ReasonArgumentNameUse; 3] = [
    ReasonArgumentNameUse {
        file: "tools.rs",
        marker: "param(\"reason\", \"string\"",
        occurrences: 1,
        why: "`yeban_reject_proposal` 的**工具参数名**（`MCP-TOOL` 契约里的入参），不是 `data.reason`。",
    },
    ReasonArgumentNameUse {
        file: "samples.rs",
        marker: "\"reason\" => Value::from(",
        occurrences: 1,
        why: "规范样本按**参数名**匹配 `reason`，右侧是夹具入参（`织体过密`），不是故障词。",
    },
    ReasonArgumentNameUse {
        file: "domain/mod.rs",
        marker: "arg_str(call, \"reason\")",
        occurrences: 1,
        why: "读取调用方入参的**键名** `reason`；值本身由调用方给出，不是本层的词。",
    },
];

// 站点普查的冻结读数。口径：`production_region` 里 `"reason"` 的**出现次数**
// （不是行数：一行里可能有两个；也不是 distinct 值数）。新增/删除任何站点都必须
// **故意**改这些数 —— 这是"看全了"这件事本身的机械契约。
// ⭐ [G18]：`domain/section_build.rs` 的三条单边输入结构化拒绝（`missingTonic` /
// `missingMode` / `missingSpaceOrMode`）各为一处字面 `"reason"` 键站点
// ⇒ 出现 221 → 224、键站点 218 → 221、字面站点 210 → 213。
// ⭐ `ADR-0030`：`domain/stems.rs` 的 `SkippedTrack::to_value()` 两个 `json!` 分支
// 各为一处字面 `"reason"` 键站点（`masterBusTrack` / `notASourceNode`）
// ⇒ 出现 224 → 226、键站点 221 → **223**（CI Run `40487059733` 的 `left:` 实测值）、
// 字面站点 213 → **215**（由恒等式 223 = 字面 215 ＋ 可解析 `let` 2 ＋ 运行时 6 收敛，
// 而 2 与 6 由本文件下方两条断言各自钉住 —— ⛔ 不是手点数出来的）。
// ⭐ **调用点传播片**（2026-10-28）：⛔ 不改**出现数**（生产区一个字符没动），
// 只将 3 处运行时站点**重新分类**成 `Propagated` ⇒ 恒等式换成
// 223 = 字面 215 ＋ 可解析 `let` 2 ＋ 运行时 3 ＋ 传播 3（四条读数各由一条断言钉住）。
const REASON_OCCURRENCES_TOTAL: usize = 226;
// ⭐ MCP-TOOL-003：键站点 208 → 216、字面站点 200 → 208 —— 新形态在
// `domain/notes.rs` / `tools.rs` 里新增的 8 处字面 `"reason"` 站点
// （tick 0、`bpmOutOfRange` 复用、`tempoPointNotFound`、`timeSignaturePointNotFound`
// 与描述文本里的键形状；逐文件拆分以扫描器读数为准，此处不手数）。
const REASON_KEY_SITES_TOTAL: usize = 223;
const REASON_LITERAL_SITES_TOTAL: usize = 215;
const REASON_RESOLVED_SITES_TOTAL: usize = 2;
const REASON_DYNAMIC_SITES_TOTAL: usize = 3;
const REASON_PROPAGATED_SITES_TOTAL: usize = 3;
const REASON_ARGUMENT_NAME_TOTAL: usize = 3;

/// 把"看不见的站点"折成失败字符串：未登记的 `Dynamic` / `ArgumentName` /
/// 任何 `Unclassified` **都点名 file:line**；陈旧的登记项同样变红。
fn unexplained_reason_sites(occurrences: &[ReasonOccurrence]) -> Vec<String> {
    let mut failures = Vec::new();
    let mut dynamic: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
    let mut propagated: BTreeMap<(String, String, String), Vec<usize>> = BTreeMap::new();
    let mut arguments = vec![0usize; REASON_ARGUMENT_NAME_USES.len()];
    for occurrence in occurrences {
        let file = relative_to_src(&occurrence.path);
        match &occurrence.kind {
            ReasonOccurrenceKind::Dynamic(expr) => dynamic
                .entry((file, expr.clone()))
                .or_default()
                .push(occurrence.line),
            ReasonOccurrenceKind::Propagated(helper, suffix, _values) => propagated
                .entry((file, helper.clone(), suffix.clone()))
                .or_default()
                .push(occurrence.line),
            ReasonOccurrenceKind::ArgumentName(context) => {
                let matched = REASON_ARGUMENT_NAME_USES.iter().position(|entry| {
                    entry.file == file.as_str() && context.contains(entry.marker)
                });
                match matched {
                    Some(index) => arguments[index] += 1,
                    None => failures.push(format!(
                        "{file}:{} 的 `\"reason\"` 只是参数名, 但没有登记（{context}）",
                        occurrence.line
                    )),
                }
            }
            ReasonOccurrenceKind::Unclassified(context) => failures.push(format!(
                "{file}:{} 无法归类 `\"reason\"` 站点（{context}）",
                occurrence.line
            )),
            ReasonOccurrenceKind::Literal(_) | ReasonOccurrenceKind::Resolved(_) => {}
        }
    }
    for ((file, expr), lines) in &dynamic {
        let at = lines.first().copied().unwrap_or_default();
        match EXEMPT_DYNAMIC_REASON_SITES
            .iter()
            .find(|entry| entry.file == file.as_str() && entry.expr == expr)
        {
            None => failures.push(format!(
                "{file}:{at} 的动态 `\"reason\"` 站点 `{expr}` 没有登记（出现 {} 次）",
                lines.len()
            )),
            Some(entry) if entry.occurrences != lines.len() => failures.push(format!(
                "{file}:{at} 的动态 `\"reason\"` 站点 `{expr}` 出现 {} 次，登记 {} 次",
                lines.len(),
                entry.occurrences
            )),
            Some(_) => {}
        }
    }
    for entry in EXEMPT_DYNAMIC_REASON_SITES {
        if !dynamic.contains_key(&(entry.file.to_owned(), entry.expr.to_owned())) {
            failures.push(format!(
                "{} 的登记项 `{}` 已经不存在（陈旧豁免, 必须删掉）",
                entry.file, entry.expr
            ));
        }
        assert!(
            !entry.forwards.trim().is_empty() && !entry.retires_when.trim().is_empty(),
            "`{}` 的动态站点登记必须写清值从哪来**且**怎么退役",
            entry.expr
        );
    }
    for entry in PROPAGATED_REASON_SITES {
        let key = (
            entry.file.to_owned(),
            entry.helper.to_owned(),
            entry.suffix.to_owned(),
        );
        match propagated.get(&key) {
            None => failures.push(format!(
                "{} 的传播登记 `{}`（后缀 `{}`）已经没有站点（陈旧, 必须删掉或恢复调用点）",
                entry.file, entry.helper, entry.suffix
            )),
            Some(lines) if entry.occurrences != lines.len() => failures.push(format!(
                "{} 的传播站点 `{}`（后缀 `{}`）出现 {} 次，登记 {} 次",
                entry.file,
                entry.helper,
                entry.suffix,
                lines.len(),
                entry.occurrences
            )),
            Some(_) => {}
        }
        // 表自身的一致性（⛔ 不靠手数）：排序、去重、非空、标识符形状、值不多于调用点。
        let mut sorted = entry.literals.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            sorted, entry.literals,
            "传播表 `{}`（后缀 `{}`）的 `literals` 必须**排序且去重**",
            entry.helper, entry.suffix
        );
        assert!(
            entry
                .literals
                .iter()
                .all(|value| is_identifier_reason(value)),
            "传播表 `{}` 里有不是标识符形状的值, 会被字符集静默拒掉",
            entry.helper
        );
        assert!(
            entry.call_sites >= entry.literals.len() && !entry.literals.is_empty(),
            "传播表 `{}`：调用点数必须 ≥ 去重后的值数, 且值集合非空",
            entry.helper
        );
    }
    for (index, entry) in REASON_ARGUMENT_NAME_USES.iter().enumerate() {
        assert!(!entry.why.trim().is_empty(), "参数名登记必须带理由");
        if arguments[index] != entry.occurrences {
            failures.push(format!(
                "{} 的参数名 `\"reason\"`（marker `{}`）出现 {} 次，登记 {} 次",
                entry.file, entry.marker, arguments[index], entry.occurrences
            ));
        }
    }
    failures
}

// ⭐ **显式豁免**：允许被字符集过滤**拒掉**的 reason 形态（每条带理由）。
///
/// 与 `EXEMPT_SCHEMA_KEYS` / `KNOWN_UNREACHABLE` 同型：⛔ 不许为了让判据变绿而
/// **放宽** [`reasons_in`] 的字符集 —— 那是把守卫减弱。若将来真出现一种**合法**的
/// 被拒形态（例如某个理由故意带 `.`），就在这里逐条登记并写明理由。
///
/// ⚠ **不是空的**（本条曾误写「今天为空」，与紧随其后的 `[(&str, &str); 6]` 自相矛盾、已更正）：
/// 实测**有 6 条** reason 值确实落在 `[A-Za-z0-9_]` **之外**，故逐条登记并给理由（见下）。
/// ⭐ 教训：这种「今天为空／全部如何如何」的自述**会随代码演进而变成假话**，而它读起来像保证。
const EXEMPT_REJECTED_REASONS: [(&str, &str); 6] = [
    // ⚠ **实测是 6 条 distinct / 7 次出现**（`织体过密` 出现 2 次）—— 逐条带理由
    // （读数是判据自己 `eprintln!` 报出来的，⛔ 不是我猜的）。
    //
    // ⭐ **它们全都不是故障词表里的词**，而是**另一个面**的数据；字符集把它们排除在
    // 词表之外是**正确**的（若把它们塞进 `GOLDEN_REASONS`，那是把非词表字符串污染词表）。
    // ⚠ 且其中 6 条**已经**落在**摘要面**里（`reason_surface_in` 不按字符集过滤）⇒
    // 改它们会被摘要判据抓住；因此本清单**不是**唯一的保护。
    (
        "no-history",
        "`yeban_undo` 的**响应数据** `reason`（`domain/mod.rs`）：这是「为什么不能撤销」的\
         回答，⛔ 不是故障词表的词。它已在**摘要面**里。",
    ),
    (
        "no-redo",
        "`yeban_redo` 的**响应数据** `reason`（`domain/mod.rs`）：同上，⛔ 不是词表的词。\
         它已在**摘要面**里。",
    ),
    (
        "织体过密",
        "**测试夹具**里 `yeban_reject_proposal` 的 `reason` **入参**（`domain/mod.rs` 的测试区）：\
         这是调用方送进来的**数据**，不是本层产出的词。它已在**摘要面**里。",
    ),
    (
        "反悔",
        "同上的测试夹具数据（`domain/mod.rs` 的测试区）。它已在**摘要面**里。",
    ),
    (
        "内存状态与磁盘一致; 传 force: true 可强制落盘",
        "`yeban_save_project` 的一条**诊断人话**（`domain/mod.rs`）：它带 `;` 与空格 ⇒ \
         落在字符集之外。⛔ 不是词表的词，但它已在**摘要面**里。",
    ),
    (
        "低于 BS.1770 的绝对门限 Γa 的读数不可达; 高于 0 LUFS 在 PCM 里不可达",
        "`domain/render.rs` 里一条**诊断人话**：带 `;` 与空格 ⇒ 落在字符集之外。\
         ⛔ 不是词表的词，但它已在**摘要面**里。",
    ),
];

/// ⭐ 字符集过滤**不允许**静默丢东西：被 [`reasons_in`] 拒掉的 reason 必须为 **0**。
///
/// ## 为什么必须有它
///
/// `reasons_in` 只接受"非空 ＋ 全在 `[A-Za-z0-9_]`"的值 —— 一个带 `-` / `.` / `/`
/// 的 reason 会被**直接丢掉**。今天之所以没变成事故，只因为
/// `the_reason_vocabulary_is_the_published_list` 是**双向**的（丢一条 ⇒ `removed`
/// 非空）；⚠ 但"今天恰好被另一条判据兜住"⛔ 不等于"这里不静默"：本判据把
/// **拒掉的数量**变成**显式读数**并断言为 0。
///
/// ⛔ 正解是把**静默**变**响亮**，⛔ 不是放宽字符集（那是减弱守卫）。
///
/// 注入（实测红）：在任意一条 reason 字面量里插一个 `.` ⇒ 这里报出那一条并变红。
#[test]
fn the_reason_character_filter_rejects_nothing() {
    let files = read_rust_sources(&[manifest_dir().join("src")]);
    assert!(!files.is_empty(), "必须真的读到源码");
    let consts = string_constants(&files);

    let mut rejected: Vec<(String, String, &'static str)> = Vec::new();
    let mut accepted_total = 0usize;
    for (_path, text) in &files {
        // ⚠ 生产区与测试区**都**要扫：过滤在两处是同一条代码路径。
        for region in [production_region(text), test_region(text)] {
            let scanned = reasons_in(_path, &region, &consts);
            accepted_total += scanned.accepted.len();
            rejected.extend(scanned.rejected);
        }
    }

    // ⚠ "抽不到东西"必须响亮失败（第 41 条）：一个失灵的扫描器会让下面"拒掉 0 条"
    // 永远成立。这里断言**接受侧**真的非空。
    assert!(
        accepted_total > 0,
        "接受侧抽出了 0 条 reason —— 扫描路径已失灵, ⛔ 不许静默通过"
    );

    let exempt: std::collections::BTreeSet<&str> = EXEMPT_REJECTED_REASONS
        .iter()
        .map(|(value, _)| *value)
        .collect();
    for (value, reason) in EXEMPT_REJECTED_REASONS {
        assert!(
            !reason.trim().is_empty(),
            "豁免项 `{value}` 必须带**非空**理由"
        );
    }
    // 豁免清单自己不得重复（重复会让"实测 distinct 条数"与清单长度对不上）。
    assert_eq!(
        exempt.len(),
        EXEMPT_REJECTED_REASONS.len(),
        "豁免清单不得有重复项"
    );
    let unexplained: Vec<&(String, String, &'static str)> = rejected
        .iter()
        .filter(|(value, _, _)| !exempt.contains(value.as_str()))
        .collect();
    assert!(
        unexplained.is_empty(),
        "被**字符集**静默拒掉的 reason（⛔ 不是放宽字符集, 而是修值或显式豁免）: {unexplained:?} \
         ⇒ 接受 {accepted_total} 条 / 拒掉 {} 条",
        rejected.len()
    );

    // 逐条读数：实测被拒的 **distinct** 值必须**恰好**等于豁免清单（多一条少一条都红）。
    let rejected_distinct: std::collections::BTreeSet<&str> = rejected
        .iter()
        .map(|(value, _, _)| value.as_str())
        .collect();
    assert_eq!(
        rejected_distinct.len(),
        EXEMPT_REJECTED_REASONS.len(),
        "被字符集拒掉的 **distinct** 值必须恰好等于豁免清单 \
         (接受 {accepted_total} 条 / 拒掉 {} 次出现 / {} 个 distinct)",
        rejected.len(),
        rejected_distinct.len()
    );
}

#[test]
fn the_reason_vocabulary_is_the_published_list() {
    let files = read_rust_sources(&[manifest_dir().join("src")]);
    assert!(!files.is_empty(), "必须真的读到源码");
    let consts = string_constants(&files);
    let mut found = BTreeSet::new();
    for (path, text) in &files {
        found.extend(reasons_in(path, &production_region(text), &consts).accepted);
    }
    let golden: BTreeSet<String> = GOLDEN_REASONS.iter().map(|r| (*r).to_owned()).collect();
    let added: Vec<&String> = found.difference(&golden).collect();
    let removed: Vec<&String> = golden.difference(&found).collect();
    assert!(
        added.is_empty() && removed.is_empty(),
        "生产区的 `reason` 词表与黄金表不一致。\n新增（要登记进 GOLDEN_REASONS）: {added:?}\n\
         消失（要删掉或改名）: {removed:?}"
    );
    assert_eq!(found.len(), GOLDEN_REASONS.len());
}

#[test]
fn every_reason_is_asserted_somewhere() {
    let src = read_rust_sources(&[manifest_dir().join("src")]);
    let tests = read_rust_sources(&[manifest_dir().join("tests")]);
    let consts = string_constants(&src);
    // 判据侧 = 单元判据区 + 集成判据文件（**宽松**：出现字面量即算"提到"）。
    let mut mentioned: BTreeSet<String> = BTreeSet::new();
    for (path, text) in &src {
        mentioned.extend(reasons_in(path, &test_region(text), &consts).accepted);
        for reason in GOLDEN_REASONS {
            if test_region(text).contains(&format!("\"{reason}\"")) {
                mentioned.insert(reason.to_owned());
            }
        }
    }
    for (_path, text) in &tests {
        for reason in GOLDEN_REASONS {
            if text.contains(&format!("\"{reason}\"")) {
                mentioned.insert(reason.to_owned());
            }
        }
    }
    let registered: BTreeSet<&str> = UNREACHABLE_ON_THIS_PLATFORM
        .iter()
        .map(|(r, _)| *r)
        .collect();
    let missing: Vec<&str> = GOLDEN_REASONS
        .iter()
        .copied()
        .filter(|reason| !mentioned.contains(*reason) && !registered.contains(reason))
        .collect();
    assert!(
        missing.is_empty(),
        "这些 `reason` 没有任何判据提到（要么补断言，要么登记进 \
         UNREACHABLE_ON_THIS_PLATFORM 并给机械理由）: {missing:?}"
    );
    // 登记表里的每一条都必须有非空理由，而且必须**真的**在黄金表里。
    for (reason, why) in UNREACHABLE_ON_THIS_PLATFORM {
        assert!(!why.is_empty(), "`{reason}` 的登记理由不得为空");
        assert!(GOLDEN_REASONS.contains(&reason), "`{reason}` 不在黄金表里");
    }
}

/// 每条 `reason` 在生产区**字面 `json!` 对象**里带的同级键（排序）。
/// ⚠ 口径：只收字面 `json!` 里的键；运行时 `map.insert(...)` 加进去的键不在内。
/// 空数组 = 该 reason 的 `data` 里**没有**同级键（或 Data 由别的构造方式给出）。
pub const GOLDEN_REASON_PAYLOADS: [(&str, &[&str]); 203] = [
    (
        "assetChannelLayout",
        &[
            "asset",
            "reason",
            "sourceChannels",
            "supported",
            "targetChannels",
        ],
    ),
    (
        "assetDecodeFailed",
        &[
            "asset",
            "decodeError",
            "detail",
            "encoderDelayFrames",
            "encoderPaddingFrames",
            "reason",
            "sourceFrames",
            "specId",
        ],
    ),
    ("assetHashMismatch", &["actual", "asset", "bytes", "reason"]),
    // ⭐ `#268`：两处出口（快照投影 / 逐轨 `arm_from_snapshot`）共用同一组键。
    ("engineSource", &["cause", "hint", "reason"]),
    // ⭐ `#290`：形态 A 的宿主口没有接上。
    (
        "engineUnavailable",
        &["form", "hint", "hostAttached", "reason"],
    ),
    (
        "assetMissing",
        &["asset", "declaredInIndex", "hint", "reason"],
    ),
    (
        "assetDeclaredDifferently",
        &["asset", "existing", "hint", "reason", "requested"],
    ),
    (
        "assetInUse",
        &["assetHash", "clipCount", "clipIds", "hint", "reason"],
    ),
    ("assetNotFound", &["assetHash", "hint", "reason"]),
    ("assetPatchIsEmpty", &["hint", "mutableFields", "reason"]),
    ("assetNotInPool", &["asset", "hint", "reason"]),
    ("automationLaneNotFound", &["lane", "reason"]),
    (
        "automationPointNotFound",
        &["lane", "pointId", "reason", "tick"],
    ),
    (
        "automationTickAmbiguous",
        &["candidatePointIds", "lane", "reason", "tick"],
    ),
    ("bpmOutOfRange", &["max", "min", "reason", "received"]),
    // ⭐ `ADR-0019`：`cents` 只收整数音分（键按源码顺序）。
    ("centsMustBeInteger", &["field", "reason", "received"]),
    // ⭐ MCP-TOOL-003：三条新字面站点的载荷形状（键按**源码顺序**）。
    ("tempoMapZeroTick", &["reason", "tick"]),
    ("tempoPointNotFound", &["reason", "tick", "hint"]),
    ("timeSignaturePointNotFound", &["reason", "tick", "hint"]),
    ("clipAlreadyExists", &["clipId", "hint", "reason"]),
    ("createIsNotReadOnly", &["reason"]),
    ("clipIdRequired", &["hint", "reason", "requiredByKinds"]),
    ("createOnlyParameter", &["parameters", "reason"]),
    (
        // ⭐ `ADR-0018`：`create: true` 时 `placement.kind` 只能是 `add`。
        "createPlacementKindMustBeAdd",
        &[
            "field",
            "hint",
            "placementKind",
            "reason",
            "supportedPlacementKinds",
        ],
    ),
    (
        "createRequiresAddOps",
        &["reason", "received", "supportedKindsWhenCreating"],
    ),
    (
        "deviceIdAlreadyInTrack",
        &["deviceId", "existingSlotIndex", "hint", "reason", "trackId"],
    ),
    ("deviceNameRequired", &["field", "reason"]),
    ("deviceNotFound", &["deviceId", "hint", "reason", "trackId"]),
    (
        "deviceSlotOutOfRange",
        &["field", "hint", "len", "reason", "value"],
    ),
    ("deviceTrackNotFound", &["hint", "reason", "trackId"]),
    ("placementTrackNotFound", &["hint", "reason", "trackId"]),
    (
        "clipPlacementNotFound",
        &["hint", "placementId", "reason", "trackId"],
    ),
    ("domainMustBeObjectOrNull", &["field", "reason"]),
    ("duplicateLaneTarget", &["lane", "reason"]),
    ("duplicateNoteId", &["noteId", "reason"]),
    ("duplicateSceneTarget", &["hint", "reason", "sceneId"]),
    (
        "durationNotDerivable",
        &["clipId", "field", "isMidi", "reason"],
    ),
    ("edgeIdRequired", &["field", "reason"]),
    ("framesExceedCap", &["frames", "maxFrames", "reason"]),
    ("indexTooLarge", &["field", "reason", "value"]),
    ("invalidDeviceDefinition", &["deviceId", "reason"]),
    // ⭐ `#290`：非法 `sceneId`。
    ("invalidSceneId", &["hint", "reason", "sceneId"]),
    ("laneMustBeString", &["allowed", "field", "reason"]),
    (
        "lanePropertiesNotApplicableToPointRemoval",
        &["reason", "supportedPointFields", "unsupportedFields"],
    ),
    ("latencyOutOfRange", &["field", "max", "reason", "value"]),
    (
        "licenseNotAllowed",
        &["allowedLicenses", "hint", "license", "reason"],
    ),
    ("licenseRequired", &["field", "hint", "reason"]),
    ("licenseWhitelistUnavailable", &["detail", "hint", "reason"]),
    (
        "loudnessTargetMissed",
        &[
            "algorithm",
            "deltaLu",
            "measuredIntegratedLufs",
            "measurementNote",
            "reason",
            "targetLufs",
            "toleranceLu",
        ],
    ),
    (
        "masterBusNodeCannotBeRemoved",
        &["hint", "masterBusTrackId", "nodeId", "reason"],
    ),
    (
        "masterBusTrackCannotBeRemoved",
        &["hint", "masterBusTrackId", "reason", "trackId"],
    ),
    // ⭐ `ADR-0030`：`yeban_export_stems` 的 `skippedTracks[]` 两条豁免登记（同形三键）。
    ("masterBusTrack", &["name", "reason", "trackId"]),
    ("notASourceNode", &["name", "reason", "trackId"]),
    (
        "midiInputAmbiguousPort",
        &["candidates", "field", "reason", "requested"],
    ),
    ("midiInputConnectFailed", &["port", "reason"]),
    ("midiInputEmptyPortName", &["field", "hint", "reason"]),
    (
        "midiInputFeatureDisabled",
        &["feature", "hint", "port", "reason"],
    ),
    ("midiInputHostUnavailable", &["port", "reason"]),
    ("midiInputNoPorts", &["ports", "reason"]),
    (
        "midiInputNoSuchPortIndex",
        &["field", "ports", "reason", "requested"],
    ),
    ("midiInputPortRequired", &["field", "ports", "reason"]),
    ("midiInputPortTypeMismatch", &["field", "reason"]),
    (
        "midiInputSecondsOutOfRange",
        &["default", "field", "maxInclusive", "minExclusive", "reason"],
    ),
    ("midiInputSecondsTypeMismatch", &["field", "reason"]),
    (
        "midiInputUnknownPort",
        &["field", "ports", "reason", "requested"],
    ),
    // ⭐ [G18]：三条单边拒绝的字面 `json!` 同级键（排序表）。
    ("missingMode", &["example", "modes", "note", "reason"]),
    ("missingSpaceOrMode", &["modes", "noteNames", "reason"]),
    ("missingTonic", &["example", "mode", "noteNames", "reason"]),
    (
        "moveRequiresPlacementId",
        &["field", "hint", "placementKind", "reason"],
    ),
    (
        "moveRequiresStartTick",
        &["field", "placementKind", "reason"],
    ),
    (
        "nonFiniteValue",
        &["field", "narrowedToF32", "reason", "value"],
    ),
    (
        "paramIndexOutOfRange",
        &["field", "paramIndex", "reason", "slotIndex", "trackId"],
    ),
    (
        "pcmBudgetExceeded",
        &["budget", "budgetDetail", "budgetGate", "reason"],
    ),
    // ⭐ `ADR-0019`：删除不存在的弯音点（键按源码顺序）。
    ("pitchBendPointNotFound", &["reason", "tick", "hint"]),
    (
        "placementAlreadyAtStartTick",
        &[
            "hint",
            "placementId",
            "placementKind",
            "reason",
            "startTick",
            "trackId",
        ],
    ),
    (
        "placementAlreadyExists",
        &["hint", "placementId", "reason", "trackId"],
    ),
    (
        "placementClipMismatch",
        &[
            "clipId",
            "placementClipId",
            "placementId",
            "placementKind",
            "reason",
            "trackId",
        ],
    ),
    // ⭐ `#340`：旧扫描器看不见 `"reason": reason`，这两条因此从未进过载荷表。
    // 键集合取自它们共同的 `json!` 对象（`reject_placement_fields` / 循环区间拒答）。
    (
        "placementFieldNotApplicable",
        &[
            "field",
            "hint",
            "placementKind",
            "reason",
            "supportedPlacementFields",
        ],
    ),
    (
        "placementIdConflict",
        &[
            "existing",
            "hint",
            "placementId",
            "reason",
            "requested",
            "trackId",
        ],
    ),
    (
        "placementNotFound",
        &[
            "hint",
            "placementCount",
            "placementId",
            "placementKind",
            "reason",
            "trackId",
        ],
    ),
    ("placementWithoutTrack", &["field", "reason"]),
    (
        "pointAddressIsAmbiguous",
        &["addressFields", "field", "reason"],
    ),
    (
        "pointAddressRequired",
        &["addressFields", "field", "reason"],
    ),
    ("projectAlreadyExists", &["path", "reason"]),
    (
        "proposalNotFound",
        &[
            "knownCount",
            "knownProposals",
            "listTool",
            "proposalId",
            "reason",
        ],
    ),
    ("recordDrainFailed", &["ioError", "reason"]),
    ("removeClipIsNotPlacement", &["hint", "reason"]),
    ("removeClipTakesNoOtherOps", &["hint", "opKinds", "reason"]),
    (
        "removeRequiresPlacementId",
        &["field", "hint", "placementKind", "reason"],
    ),
    (
        "removeTakesNoProperties",
        &["field", "properties", "reason"],
    ),
    ("removeTrackIsNotPlacement", &["hint", "reason", "trackId"]),
    (
        "removeTrackTakesNoOtherOps",
        &["hint", "opKinds", "reason", "trackId"],
    ),
    ("routingEdgeAlreadyExists", &["edgeId", "hint", "reason"]),
    ("routingEdgeNotFound", &["edgeId", "hint", "reason"]),
    ("routingNodeAlreadyExists", &["hint", "nodeId", "reason"]),
    ("routingNodeNotFound", &["hint", "nodeId", "reason"]),
    ("sceneAlreadyExists", &["hint", "reason", "sceneId"]),
    // ⭐ `#290`：缺 `sceneId` / 队列满。
    ("sceneIdRequired", &["hint", "reason"]),
    (
        "sceneLaunchQueueFull",
        &["capacity", "hostAttached", "pending", "reason"],
    ),
    (
        "sceneNameRequiredWhenCreating",
        &["field", "hint", "reason"],
    ),
    (
        "sceneNotFound",
        &["hint", "reason", "sceneCount", "sceneId"],
    ),
    (
        "sceneTempoMustBeNumberOrNull",
        &["field", "reason", "received"],
    ),
    // ⭐ `#290`：场景 tempo 覆盖越界 / 非有限。
    ("sceneTempoOutOfRange", &["reason", "sceneId", "tempoBpm"]),
    ("secondsExceedCap", &["maxSeconds", "reason", "seconds"]),
    ("sectionAlreadyExists", &["hint", "reason", "sectionId"]),
    (
        "sectionNameRequiredWhenCreating",
        &["field", "hint", "reason"],
    ),
    (
        "sectionTickMustBeUnsignedInteger",
        &["field", "reason", "received"],
    ),
    ("sectionNotFound", &["hint", "reason", "sectionId"]),
    (
        "sfzInstrumentNotFound",
        &["instrumentId", "loadedInstruments", "reason"],
    ),
    ("sfzNotUtf8", &["path", "reason", "validUpTo"]),
    ("sfzParseFailed", &["parser", "reason"]),
    ("sfzPathIsDirectory", &["path", "reason"]),
    ("sfzReadFailed", &["ioError", "path", "reason"]),
    // ⭐ `#221`：音轨绑定的采样器定义文件读不到/解析不了（渲染期唯一的新 reason）。
    (
        "samplerUnreadable",
        &["cause", "hint", "path", "reason", "resolvedPath", "tried"],
    ),
    (
        "slotOutOfRange",
        &["field", "paramIndex", "reason", "slotIndex", "trackId"],
    ),
    ("sourceAmbiguous", &["reason"]),
    ("sourceMissing", &["reason"]),
    (
        "staticLaneNotApplicable",
        &["allowed", "field", "reason", "received"],
    ),
    (
        "tempoUnusable",
        &["bpm", "field", "frames", "reason", "sampleRate"],
    ),
    ("trackNotFound", &["hint", "reason", "trackId"]),
    (
        "unknownDeviceField",
        &[
            "hint",
            "reason",
            "supportedDeviceFields",
            "unsupportedFields",
        ],
    ),
    (
        "unknownConnectRoutingField",
        &[
            "hint",
            "reason",
            "supportedConnectRoutingFields",
            "unsupportedFields",
        ],
    ),
    (
        "unknownAddRoutingNodeField",
        &[
            "hint",
            "reason",
            "supportedAddRoutingNodeFields",
            "unsupportedFields",
        ],
    ),
    (
        "unknownAddTrackField",
        &[
            "hint",
            "reason",
            "supportedAddTrackFields",
            "unsupportedFields",
        ],
    ),
    ("unknownDeviceKind", &["reason", "supportedDeviceKinds"]),
    (
        "unknownDeviceParamField",
        &[
            "hint",
            "reason",
            "supportedDeviceParamFields",
            "unsupportedFields",
        ],
    ),
    (
        "unknownDisconnectRoutingField",
        &[
            "hint",
            "reason",
            "supportedDisconnectRoutingFields",
            "unsupportedFields",
        ],
    ),
    (
        // ⭐ #175：拼错的设备参数名（编辑距离 ≤ 2）在写入侧被拒时的载荷。
        "unknownEffectParam",
        &[
            "deviceName",
            "hint",
            "reason",
            "suggestions",
            "unrecognisedParams",
        ],
    ),
    (
        "unknownFlagField",
        &["hint", "reason", "supportedFlagFields", "unsupportedFields"],
    ),
    (
        "unknownInsertDeviceField",
        &[
            "hint",
            "reason",
            "supportedDeviceFields",
            "supportedInsertDeviceFields",
            "unsupportedFields",
        ],
    ),
    (
        "unknownLaneField",
        &["hint", "reason", "supportedLaneFields", "unsupportedFields"],
    ),
    (
        "unknownLaneTarget",
        &["allowed", "field", "note", "reason", "received"],
    ),
    (
        "unknownLaunchQuantization",
        &["allowed", "field", "reason", "received"],
    ),
    (
        "unknownPlacementField",
        &[
            "field",
            "hint",
            "placementKind",
            "reason",
            "supportedPlacementFields",
        ],
    ),
    (
        "unknownPlacementKind",
        &[
            "field",
            "hint",
            "reason",
            "supportedPlacementKinds",
            "value",
        ],
    ),
    (
        "unknownPointField",
        &["reason", "supportedPointFields", "unsupportedFields"],
    ),
    (
        "unknownPointRemovalField",
        &[
            "hint",
            "reason",
            "supportedPointRemovalFields",
            "unsupportedFields",
        ],
    ),
    (
        "unknownRemoveAssetField",
        &[
            "hint",
            "reason",
            "supportedRemoveAssetFields",
            "unsupportedFields",
        ],
    ),
    (
        "unknownRemoveClipField",
        &[
            "hint",
            "reason",
            "supportedRemoveClipFields",
            "unsupportedFields",
        ],
    ),
    (
        "unknownRemoveDeviceField",
        &[
            "hint",
            "reason",
            "supportedRemoveDeviceFields",
            "unsupportedFields",
        ],
    ),
    (
        "unknownRemoveRoutingNodeField",
        &[
            "hint",
            "reason",
            "supportedRemoveRoutingNodeFields",
            "unsupportedFields",
        ],
    ),
    (
        "unknownRemoveSceneField",
        &[
            "hint",
            "reason",
            "supportedRemoveSceneFields",
            "unsupportedFields",
        ],
    ),
    (
        "unknownRemoveSectionField",
        &[
            "hint",
            "reason",
            "supportedRemoveSectionFields",
            "unsupportedFields",
        ],
    ),
    (
        "unknownRemoveTrackField",
        &[
            "hint",
            "reason",
            "supportedRemoveTrackFields",
            "unsupportedFields",
        ],
    ),
    (
        "unknownRoutingGainField",
        &[
            "hint",
            "reason",
            "supportedRoutingGainFields",
            "unsupportedFields",
        ],
    ),
    (
        "unknownSetAssetField",
        &[
            "hint",
            "reason",
            "supportedSetAssetFields",
            "unsupportedFields",
        ],
    ),
    (
        "unknownSetSceneField",
        &[
            "hint",
            "reason",
            "supportedSetSceneFields",
            "unsupportedFields",
        ],
    ),
    (
        "unknownSetSceneOpField",
        &[
            "hint",
            "reason",
            "supportedSetSceneOpFields",
            "unsupportedFields",
        ],
    ),
    (
        "unknownStaticLane",
        &["allowed", "field", "note", "reason", "received"],
    ),
    (
        "unknownRoutingKind",
        &["allowed", "field", "reason", "received"],
    ),
    (
        "unknownTrackKind",
        &["allowed", "field", "reason", "received"],
    ),
    (
        "unknownTransportConfigField",
        &[
            "reason",
            "supportedTransportConfigFields",
            "unsupportedFields",
        ],
    ),
    (
        "unknownWriteMode",
        &["allowed", "field", "reason", "received"],
    ),
    ("trackAlreadyExists", &["hint", "reason", "trackId"]),
    ("valueMustBeBoolean", &["field", "reason", "received"]),
    ("valueMustBeNumberOrNull", &["field", "reason", "received"]),
    ("valueMustBeNumber", &["field", "reason"]),
    (
        "valueMustBeNonEmptyString",
        &["field", "reason", "received"],
    ),
    ("valueMustBeString", &["field", "reason", "received"]),
    (
        "valueMustBeUnsignedInteger",
        &["field", "reason", "received"],
    ),
    ("writeModeMustBeString", &["allowed", "field", "reason"]),
    ("valueOutOfRange", &["field", "max", "reason", "received"]),
    // ⭐ **调用点传播片**（2026-10-28）：41 条由调用点传播出来的 reason, 载荷键**实测**
    // （`reject_unknown_keys` 的那 25 条带 `supportedFields` / `unsupportedFields`；
    // `read_non_empty_string` 的空串分支只有 `field`, 非字符串分支多一条 `received`）。
    (
        "unknownKeySignatureField",
        &["reason", "supportedFields", "unsupportedFields"],
    ),
    (
        "unknownSetAuthorField",
        &["reason", "supportedFields", "unsupportedFields"],
    ),
    (
        "unknownSetClipGainField",
        &["reason", "supportedFields", "unsupportedFields"],
    ),
    (
        "unknownSetClipNameField",
        &["reason", "supportedFields", "unsupportedFields"],
    ),
    (
        "unknownSetClipPlacementDurationField",
        &["reason", "supportedFields", "unsupportedFields"],
    ),
    (
        "unknownSetClipPlacementLoopField",
        &["reason", "supportedFields", "unsupportedFields"],
    ),
    (
        "unknownSetClipPlacementMutedField",
        &["reason", "supportedFields", "unsupportedFields"],
    ),
    (
        "unknownSetDeviceBypassedField",
        &["reason", "supportedFields", "unsupportedFields"],
    ),
    (
        "unknownSetDeviceNameField",
        &["reason", "supportedFields", "unsupportedFields"],
    ),
    (
        "unknownSetKeySignatureField",
        &["reason", "supportedFields", "unsupportedFields"],
    ),
    (
        "unknownSetNoteExpressionField",
        &["reason", "supportedFields", "unsupportedFields"],
    ),
    (
        "unknownSetNotePitchShapeField",
        &["reason", "supportedFields", "unsupportedFields"],
    ),
    (
        "unknownSetNoteTimingField",
        &["reason", "supportedFields", "unsupportedFields"],
    ),
    (
        "unknownSetPitchBendPointField",
        &["reason", "supportedFields", "unsupportedFields"],
    ),
    (
        "unknownSetSectionField",
        &["reason", "supportedFields", "unsupportedFields"],
    ),
    (
        "unknownSetSectionOpField",
        &["reason", "supportedFields", "unsupportedFields"],
    ),
    (
        "unknownSetTempoField",
        &["reason", "supportedFields", "unsupportedFields"],
    ),
    (
        "unknownSetTempoPointField",
        &["reason", "supportedFields", "unsupportedFields"],
    ),
    (
        "unknownSetTimeSignatureField",
        &["reason", "supportedFields", "unsupportedFields"],
    ),
    (
        "unknownSetTimeSignaturePointField",
        &["reason", "supportedFields", "unsupportedFields"],
    ),
    (
        "unknownSetTitleField",
        &["reason", "supportedFields", "unsupportedFields"],
    ),
    (
        "unknownSetTrackColorField",
        &["reason", "supportedFields", "unsupportedFields"],
    ),
    (
        "unknownSetTrackNameField",
        &["reason", "supportedFields", "unsupportedFields"],
    ),
    (
        "unknownSetTrackSamplerField",
        &["reason", "supportedFields", "unsupportedFields"],
    ),
    (
        "unknownTransposeClipField",
        &["reason", "supportedFields", "unsupportedFields"],
    ),
    ("assetLicenseMustBeNonEmptyString", &["field", "reason"]),
    (
        "assetOriginalPathMustBeNonEmptyString",
        &["field", "reason"],
    ),
    ("sceneColorMustBeNonEmptyString", &["field", "reason"]),
    ("sceneNameMustBeNonEmptyString", &["field", "reason"]),
    ("sectionColorMustBeNonEmptyString", &["field", "reason"]),
    ("sectionNameMustBeNonEmptyString", &["field", "reason"]),
    ("trackColorMustBeNonEmptyString", &["field", "reason"]),
    ("trackSamplerMustBeNonEmptyString", &["field", "reason"]),
    (
        "assetLicenseMustBeNonEmptyStringMustBeString",
        &["field", "reason", "received"],
    ),
    (
        "assetOriginalPathMustBeNonEmptyStringMustBeString",
        &["field", "reason", "received"],
    ),
    (
        "sceneColorMustBeNonEmptyStringMustBeString",
        &["field", "reason", "received"],
    ),
    (
        "sceneNameMustBeNonEmptyStringMustBeString",
        &["field", "reason", "received"],
    ),
    (
        "sectionColorMustBeNonEmptyStringMustBeString",
        &["field", "reason", "received"],
    ),
    (
        "sectionNameMustBeNonEmptyStringMustBeString",
        &["field", "reason", "received"],
    ),
    (
        "trackColorMustBeNonEmptyStringMustBeString",
        &["field", "reason", "received"],
    ),
    (
        "trackSamplerMustBeNonEmptyStringMustBeString",
        &["field", "reason", "received"],
    ),
];

/// **非标识符形状**的 `reason` 值（成功路径上的说明性字段），4 条。
///
/// 为什么单列一张表：上面 155 条的黄金表按口径只收**标识符形状**
/// （`[A-Za-z0-9_]+`）的值 —— 那是"错误分类用的机器可读 reason"。生产区里另有 4 个
/// `"reason"` 是**人话/带连字符**的（`no-history` / `no-redo` / 两条中文说明），
/// 它们同样进 `data`，改了没有任何东西会红。本表把它们也钉住，并**如实登记口径边界**。
pub const GOLDEN_INFO_REASON_PAYLOADS: [(&str, &[&str]); 4] = [
    ("no-history", &["reason", "undoable"]),
    ("no-redo", &["reason", "redoable"]),
    (
        "低于 BS.1770 的绝对门限 Γa 的读数不可达; 高于 0 LUFS 在 PCM 里不可达",
        &["maxLufs", "minLufs", "reason"],
    ),
    (
        "内存状态与磁盘一致; 传 force: true 可强制落盘",
        &[
            "bytes",
            "format",
            "path",
            "projectDigest",
            "reason",
            "saved",
            "skipped",
        ],
    ),
];

/// 标识符形状（错误分类用的 `reason`）。
fn is_identifier_reason(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// 每条 `reason` 的**同级 `data` 键**（只收字面 `json!` 对象里的键）。
///
/// 口径（与 [`GOLDEN_REASON_PAYLOADS`] 的表头逐字一致）：
/// * 生产区里每一处 `"reason": "<字面量>"` 都算一次采样点；
/// * 以该点**向前最近的 `json!(`** 为容器，用括号配对取出整个对象字面量；
/// * 只收**对象第一层**的字符串键（`depth == 2`：一层是 `json!(` 的圆括号，一层是
///   `{`）；
/// * 运行时 `map.insert(...)` 加进去的键**不在内**（那种形状的 reason 会读成空集，
///   本文件把它如实登记为 `&[]`）。
///
/// **全文区间掩码**（R94）：注释区间与字符串**内部**的字节下标为 `true`。
///
/// 口径：**开/闭引号不掩**（否则针自身的引号会被判成"在串里"，扫描器就再也读不到东西 ——
/// 本文件第五批实测：把引号一起掩掉，reason 数变成 0）；**换行永不掩**（否则行号与行结构
/// 都会漂移）；**只标区间、不删字符**（不用"抹掉内容"代替"跳过区间"，那会改变表达式语义 ⇒ 假红）。
fn skipped_offsets(text: &str) -> Vec<bool> {
    let bytes = text.as_bytes();
    let mut mask = vec![false; bytes.len()];
    let mut index = 0usize;
    let mut in_string = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if byte == b'\\' {
                if index + 1 < bytes.len() {
                    mask[index + 1] = true;
                }
                index += 2;
                continue;
            }
            if byte == b'"' {
                in_string = false;
                index += 1;
                continue;
            }
            if byte != b'\n' {
                mask[index] = true;
            }
            index += 1;
            continue;
        }
        if byte == b'"' {
            in_string = true;
            index += 1;
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
            while index < bytes.len() && bytes[index] != b'\n' {
                mask[index] = true;
                index += 1;
            }
            continue;
        }
        index += 1;
    }
    mask
}

/// 该字节是否落在注释／字符串**内部**（越界按"不在"处理）。
fn is_skipped(mask: &[bool], at: usize) -> bool {
    mask.get(at).copied().unwrap_or(false)
}

/// 每条 reason 的**字面 `json!` 同级键**（#340：值只取**可分类**的站点）。
///
/// 值来源与 [`reasons_in`] 完全同一条扫描器：字面量 ＋ 同函数内可解析的字面量 `let`
/// ＋ **调用点传播**。看不见的站点（`Dynamic` / `ArgumentName`）不贡献值 —— 它们由
/// `every_reason_occurrence_is_classified_or_explicitly_registered` 逐条点名，
/// ⛔ 不是在这里静默丢掉。
fn reason_payloads(
    text: &str,
    consts: &BTreeMap<String, String>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for occurrence in reason_occurrences("<payload>", text, consts) {
        let keys = text[..occurrence.offset]
            .rfind("json!(")
            .map_or_else(BTreeSet::new, |open| {
                json_object_keys(&text[open + "json!".len()..])
            });
        for reason in occurrence.kind.values() {
            out.entry(reason).or_default().extend(keys.iter().cloned());
        }
    }
    out
}

/// 从 `(` 开始做括号配对，取出**对象第一层**的字符串键。
///
/// 只吃 `(` / `{` / `[` 三种开括号（对象字面量与数组都会出现），并在遇到字符串时
/// 走一个小状态机处理 `\\` 转义 —— 与 `code_without_literals` 同一条口径。
fn json_object_keys(from_open_paren: &str) -> BTreeSet<String> {
    let bytes = from_open_paren.as_bytes();
    let mut keys = BTreeSet::new();
    let mut depth = 0i32;
    let mut index = 0usize;
    let mut current: Option<String> = None;
    let mut in_string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
                if let Some(text) = current.as_mut() {
                    text.push(char::from(byte));
                }
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
                // 是不是键：后面第一个非空白必须紧跟 `:`，且正好在对象第一层。
                let mut next = index + 1;
                while next < bytes.len() && bytes[next].is_ascii_whitespace() {
                    next += 1;
                }
                if depth == 2
                    && next < bytes.len()
                    && bytes[next] == b':'
                    && let Some(text) = current.take()
                {
                    keys.insert(text);
                }
                current = None;
            } else if let Some(text) = current.as_mut() {
                text.push(char::from(byte));
            }
            index += 1;
            continue;
        }
        match byte {
            b'"' => {
                in_string = true;
                current = Some(String::new());
            }
            b'(' | b'{' | b'[' => depth += 1,
            b')' | b'}' | b']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        index += 1;
    }
    keys
}

#[test]
fn the_reason_payload_shape_is_the_published_table() {
    let files = read_rust_sources(&[manifest_dir().join("src")]);
    let consts = string_constants(&files);
    let mut found: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (_path, text) in &files {
        for (reason, keys) in reason_payloads(&production_region(text), &consts) {
            found.entry(reason).or_default().extend(keys);
        }
    }
    let golden: BTreeMap<String, BTreeSet<String>> = GOLDEN_REASON_PAYLOADS
        .iter()
        .map(|(reason, keys)| {
            (
                (*reason).to_owned(),
                keys.iter().map(|key| (*key).to_owned()).collect(),
            )
        })
        .collect();
    let info: BTreeMap<String, BTreeSet<String>> = GOLDEN_INFO_REASON_PAYLOADS
        .iter()
        .map(|(reason, keys)| {
            (
                (*reason).to_owned(),
                keys.iter().map(|key| (*key).to_owned()).collect(),
            )
        })
        .collect();
    let mut diffs: Vec<String> = Vec::new();
    let compare = |table: &BTreeMap<String, BTreeSet<String>>,
                   found: &BTreeMap<String, BTreeSet<String>>,
                   diffs: &mut Vec<String>| {
        for (reason, keys) in table {
            match found.get(reason) {
                None => diffs.push(format!("`{reason}` 在生产区找不到了")),
                Some(actual) if actual != keys => {
                    let missing: Vec<&String> = keys.difference(actual).collect();
                    let extra: Vec<&String> = actual.difference(keys).collect();
                    diffs.push(format!("`{reason}`: 缺 {missing:?} / 多 {extra:?}"));
                }
                Some(_) => {}
            }
        }
        for reason in found.keys() {
            if !table.contains_key(reason) {
                diffs.push(format!("`{reason}` 没登记进黄金表"));
            }
        }
    };
    // 按**形状**分成两张表：标识符形状（错误分类）与说明性形状（成功路径）。
    let (ident_found, info_found): (BTreeMap<_, _>, BTreeMap<_, _>) = found
        .iter()
        .partition(|(reason, _keys)| is_identifier_reason(reason));
    let ident_found: BTreeMap<String, BTreeSet<String>> = ident_found
        .into_iter()
        .map(|(reason, keys)| (reason.clone(), keys.clone()))
        .collect();
    let info_found: BTreeMap<String, BTreeSet<String>> = info_found
        .into_iter()
        .map(|(reason, keys)| (reason.clone(), keys.clone()))
        .collect();
    compare(&golden, &ident_found, &mut diffs);
    compare(&info, &info_found, &mut diffs);
    assert!(
        diffs.is_empty(),
        "`reason` 的**载荷面**与黄金表不一致（集合对 ≠ 载荷对）：\n{}",
        diffs.join("\n")
    );
    // 三张表必须覆盖同一个全集，且两张载荷表加起来就是全集。
    assert_eq!(ident_found.len(), GOLDEN_REASONS.len());
    assert_eq!(GOLDEN_REASON_PAYLOADS.len(), GOLDEN_REASONS.len());
    assert_eq!(info_found.len(), GOLDEN_INFO_REASON_PAYLOADS.len());
    assert_eq!(
        ident_found.len() + info_found.len(),
        found.len(),
        "两张载荷表必须**恰好**覆盖生产区的全部 `reason`"
    );
}

/// **`reason` 面的冻结摘要**（R70②：`assert_eq!(第一次, 第二次)` 是**自比**，
/// 不是字节契约 —— 字节契约必须钉**字面摘要**）。
///
/// ⚠ 文档表（两方向之二）：本字面量必须同时出现在**本文件的模块文档表**里，
/// 判据用 `include_str!` 把本文件读回来核对（表与代码不许各说各话）。
/// 规范形式：**文件按路径排序** → 每个 `reason` 按**首次出现**顺序 →
/// 每个键按**源码顺序**（只收字面 `json!` 对象第一层的键）。
///
/// ⭐ `#274` 重钉：采样器轨的读盘失败登记从 `build_source` **搬进** provider
/// （`prepare_sampler_track`），并新增两条"没有武装记录 / 没有摊平表"的
/// `engineSource` 失败 ⇒ `reason` 的**首次出现顺序**变了（**集合不变**：
/// `engineSource` / `samplerUnreadable` 都是既有词），摘要因此按判据要求逐位更新。
/// 新摘要由判据自己报出后照抄（`18b8b4cb…`）。
///
/// ⭐ `#290` 重钉：新增 `yeban_launch_scene` ⇒ 5 个新 reason
/// （`engineUnavailable` / `invalidSceneId` / `sceneIdRequired` /
/// `sceneLaunchQueueFull` / `sceneTempoOutOfRange`）＋ `sceneNotFound` 多一个
/// `sceneCount` 键 ⇒ 148 个 reason / 668 行；新摘要
/// `480018afbd8c47dd9db4c4d3d4ffb4e082748ecbe44e3638547a2bbb8f9dd22a`
/// 同样由判据自己报出后照抄。
///
/// ⭐ `#340` 重钉：扫描器开始解析 `"reason": reason` 的**字面量 `let`**
/// （`let reason = "..."` / `let reason = if cond { "a" } else { "b" }`）⇒
/// 两条一直存在、但旧摘要**看不见**的 reason 进入规范文本：
/// `placementFieldNotApplicable` / `unknownPlacementField` ⇒ 148 → 150 个 reason、
/// 668 → 680 行（两条各带 5 个同级键）。这是**真实增读**，不是新增词：
/// 两条都有 `src` 判据断言（`notes.rs` 的 `inapplicable_and_misspelled_...` 等）。
/// 看不见的运行时站点（9 处 `Dynamic`）**不进**规范文本，由站点普查登记 ——
/// 本摘要因此是"**可分类**面"的字节契约，不是全集的伪装。新摘要
/// `6f13316b51ef974e2443a909c4eb9fdeeaf97139b11fef6da5365de4de40e259`
/// 由判据自己报出后照抄。
///
/// ⭐ `#341` 重钉：三个 `Dynamic` 站点退役成 `match` 字面量
/// （`import_audio.rs` 的 `error.as_str()`、`notes.rs` 的 `reason.as_str()` 与
/// `format!("{kind}RequiresPlacementId")`）⇒ 它们**一直产出**的 6 条 reason 第一次
/// 进入规范文本：`sourceMissing` / `sourceAmbiguous` / `slotOutOfRange` /
/// `paramIndexOutOfRange` / `moveRequiresPlacementId` / `removeRequiresPlacementId`
/// ⇒ 150 → 156 个 reason、680 → 706 行。新摘要
/// `36f254b59f3b307b9b3d0d2dc198cc9ac9a4ef9bd0783ce6871fb58d80a34e67`
/// 由判据自己报出后照抄。
///
/// ⭐ MCP-TOOL-003 重钉：`setTempoPoint` / `setTimeSignaturePoint` 落地 ⇒
/// 3 条新字面站点进入规范文本（`tempoMapZeroTick` × 3 处站点、
/// `tempoPointNotFound` / `timeSignaturePointNotFound` 各 1 处）
/// ⇒ 156 → 159 个 reason、706 → 717 行。新摘要
/// `5328f2d48db9937ccb00457aaf33d88b0c876b5493dd8e388eb82f17478546b8`
/// 由判据自己报出后照抄。
///
/// ⭐ `ADR-0018` 重钉：`yeban_edit_notes` 的 `create: true` + `placement` 组合放行 ⇒
/// `placementIsNotCreation` 退役、`createPlacementKindMustBeAdd` 登场
/// （reason 集合大小不变，仍是 159；新 reason 带 5 个同级键 ⇒ 717 → 720 行）。
/// 新摘要 `554c947ed735a9d3ce7712a65ed5e7bb21341d450eb97caca78f3e7ebaacc537`
/// 由判据自己报出后照抄。
///
/// ⭐ `ADR-0019` 重钉：`setPitchBendPoint` 落地 ⇒ 2 条新字面站点进入规范文本
/// （`centsMustBeInteger` / `pitchBendPointNotFound`，各带 3 个同级键）
/// ⇒ 159 → 161 个 reason、720 → 728 行。新摘要
/// `c5056c369e24f34a11ab7349d5af96f4a552f6431c1ebe9a8b18353c6635514d`
/// 由判据自己报出后照抄。
///
/// ⭐ `[G18]` 重钉：`parse_scale` 的单边输入结构化拒绝（`missingTonic` 4 键 /
/// `missingMode` 4 键 / `missingSpaceOrMode` 3 键，加 3 条 reason 行）
/// ⇒ 161 → 164 个 reason、728 → 742 行。新摘要
/// `59a44770d9b2fabba520fbcf08530534b61e69076809b87552d5c0aca08c38e6`
/// 由判据 `the_reason_surface_digest_is_frozen_and_documented`（CI Run 40462481785）
/// 报出后照抄。
///
/// ⭐ `ADR-0030` 重钉：新增 `src/domain/stems.rs` 的两条字面站点
/// （`masterBusTrack` / `notASourceNode`，各带 `trackId` / `name` / `reason` 三键）
/// ⇒ 164 → 166 个 reason、742 → 750 行。**摘要同批必须一起换**（上一批只换了
/// 两个规模读数，摘要留在 `[G18]` 那一位 ⇒ CI Run `40487059733` 三条判据当场红，
/// 而这正是本判据存在的理由：它宁可红，也不许"计数对了而字节没对"）。
/// 新摘要 `2df60826b5d11ed38bb979073bc3910125dcfb5ca2eacd3e232bfc223f7b0ea1`
/// 由该 run 里 `the_reason_surface_digest_is_frozen_and_documented` 的 `left:`
/// 报出后照抄（⛔ 不手算）。
///
/// ⭐ **调用点传播片**（2026-10-28）重钉：41 个传播出来的 reason 进入规范文本
/// （25 条带 `reason`/`supportedFields`/`unsupportedFields` 三键、8 条带 `field`/`reason`
/// 两键、8 条带 `field`/`reason`/`received` 三键 ⇒ ＋41 行 ＋115 键行）
/// ⇒ 166 → **207** 个 reason、750 → **906** 行。
/// 新摘要由**同一份扫描器代码**在零依赖侧算出：`/tmp/yeban_probe` 把本文件的顶层条目
/// **逐字**抽出来跑（⛔ 不重写逻辑），先用**改前**的码在**改前**的 `src/` 上复现
/// `2df60826…` / 166 / 750 证明探针忠实, 再用**改后**的码在**同一个** `src/` 上算出新值
/// （`src/` 的生产区一个字节没动 ⇒ 两位差值只可能来自扫描器）。
pub const FROZEN_REASON_SURFACE_SHA256: &str =
    "4890ba25908a83444859edd023e7109b02eb5e33646f46a7b9633ae39443dd43";

/// 规范形式里的 reason 数 / 规范行数（与摘要一起构成"两端"读数）。
pub const REASON_SURFACE_REASONS: usize = 207;
/// 规范行数（每个 reason 一行 ＋ 每个键一行）。
pub const REASON_SURFACE_LINES: usize = 906;

/// 单文件里的 `reason` 面：**保留出现顺序**的 `(reason, keys)`。
///
/// 与 [`reason_payloads`] 的唯一区别是"顺序"：那个返回 `BTreeMap`（排序 ⇒
/// 对键序不敏感），这个保留源码顺序 ⇒ 对**键序**敏感（摘要要的就是这一位）。
///
/// #340：值来源与 [`reasons_in`] 同一条扫描器，因此**可解析的字面量 `let`** 与
/// **调用点传播**（`"reason": reason`）都进这份规范文本；⛔ 真看不见的运行时站点
/// 由站点普查点名为**豁免**、不贡献值 —— 摘要因此是"**可分类**的 reason 面"，
/// 不是一个假装完整的全集（2026-10-28：3 处转发站点从豁免搬进传播 ⇒ 摘要重钉）。
fn reason_surface_in(text: &str, consts: &BTreeMap<String, String>) -> Vec<(String, Vec<String>)> {
    let region = production_region(text);
    let mut order: Vec<(String, Vec<String>)> = Vec::new();
    for occurrence in reason_occurrences("<surface>", &region, consts) {
        let keys = region[..occurrence.offset]
            .rfind("json!(")
            .map_or_else(Vec::new, |open| {
                json_object_key_order(&region[open + "json!".len()..])
            });
        for reason in occurrence.kind.values() {
            match order.iter_mut().find(|(name, _keys)| *name == reason) {
                Some((_name, merged)) => {
                    for key in &keys {
                        if !merged.contains(key) {
                            merged.push(key.clone());
                        }
                    }
                }
                None => order.push((reason, keys.clone())),
            }
        }
    }
    order
}

/// 与 [`json_object_keys`] 同一条括号配对／字符串状态机，但**保留顺序与重复**。
fn json_object_key_order(from_open_paren: &str) -> Vec<String> {
    let bytes = from_open_paren.as_bytes();
    let mut keys: Vec<String> = Vec::new();
    let mut depth = 0i32;
    let mut index = 0usize;
    let mut current: Option<String> = None;
    let mut in_string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
                if let Some(text) = current.as_mut() {
                    text.push(char::from(byte));
                }
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
                let mut next = index + 1;
                while next < bytes.len() && bytes[next].is_ascii_whitespace() {
                    next += 1;
                }
                if depth == 2
                    && next < bytes.len()
                    && bytes[next] == b':'
                    && let Some(text) = current.take()
                {
                    keys.push(text);
                }
                current = None;
            } else if let Some(text) = current.as_mut() {
                text.push(char::from(byte));
            }
            index += 1;
            continue;
        }
        match byte {
            b'"' => {
                in_string = true;
                current = Some(String::new());
            }
            b'(' | b'{' | b'[' => depth += 1,
            b')' | b'}' | b']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        index += 1;
    }
    keys
}

/// 把整个生产区折成**规范文本**（跨文件按路径排序，键序保留）。
fn reason_surface(files: &[(String, String)]) -> String {
    let consts = string_constants(files);
    let mut order: Vec<(String, Vec<String>)> = Vec::new();
    let mut sorted: Vec<&(String, String)> = files.iter().collect();
    sorted.sort_by(|left, right| left.0.cmp(&right.0));
    for (_path, text) in sorted {
        for (reason, keys) in reason_surface_in(text, &consts) {
            match order.iter_mut().find(|(name, _keys)| *name == reason) {
                Some((_name, merged)) => {
                    for key in keys {
                        if !merged.contains(&key) {
                            merged.push(key);
                        }
                    }
                }
                None => order.push((reason, keys)),
            }
        }
    }
    let mut lines: Vec<String> = Vec::new();
    for (reason, keys) in order {
        lines.push(reason);
        for key in keys {
            lines.push(format!("  {key}"));
        }
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// **R70②/R78④ 两端判据**：`sha256(规范文本) == 字面常量` **且** `文档表.contains(常量)`。
///
/// 为什么需要它：上一批的 `reason` 两张黄金表是**集合/载荷**契约（`BTreeMap` 排序），
/// 它们对**键序**不敏感 —— 把 `json!` 里两个键换个位置，语义表全绿而交付字节已经变了。
/// 本判据把整个 `reason` 面的规范文本（含**键序**）冻成字面摘要。
///
/// R56（先喂已知红＋已知绿）写在判据体内：① 同一份文本算两次必须相同（已知绿）；
/// ② 改 1 个字符 ⇒ 摘要必变（已知红）；③ 交换一个三行块的顺序 ⇒ 摘要必变（已知红，
/// 且证明摘要对键序敏感 —— 这正是语义表抓不到的那一位）。
///
/// 注入（实测红）：① 把某个 `json!` 里两个键**换个位置** ⇒ **只有本条红**
/// （语义两张表全绿）＝ 独有牙；② 改一个 reason 字面量 ⇒ 本条 ＋ 两张语义表红。
#[test]
fn the_reason_surface_digest_is_frozen_and_documented() {
    let files: Vec<(String, String)> = read_rust_sources(&[manifest_dir().join("src")]);
    // #176：新增 `src/output_schema.rs` ⇒ 40 → 41（`reason` 面的规范文本不含 reason 键，摘要不变）。
    // #271：新增 `src/domain/sample_roots.rs` ⇒ 41 → 42（同样不含 `reason` 键，摘要逐位不变）
    // #345：新增 `src/domain/import_midi.rs` ⇒ 43 → 44（同样不含 `reason` 键，摘要逐位不变）
    // `ADR-0030`：新增 `src/domain/stems.rs` ⇒ 44 → 45，**这次带两条新字面站点**
    // （`masterBusTrack` / `notASourceNode`）⇒ 摘要同批重钉。
    // ⇒ 本条只证明"分母变了一位"。
    assert_eq!(files.len(), 45, "生产区 `.rs` 文件数（规范形式的分母之一）");
    let canonical = reason_surface(&files);
    let digest = yeban_model::AssetHash::of_bytes(canonical.as_bytes())
        .as_str()
        .to_owned();

    // ① 已知绿：真源码的规范文本必须等于冻结的字面摘要。
    assert_eq!(
        digest, FROZEN_REASON_SURFACE_SHA256,
        "`reason` 面的规范文本摘要漂移了（改判据前先把新摘要写进常量**和**文档表）"
    );
    // ② 两方向之二：本文件的**文档表**必须包含同一个字面摘要。
    let own_source = include_str!("fault_vocabulary.rs");
    assert!(
        own_source.contains(FROZEN_REASON_SURFACE_SHA256),
        "文档表里没有这个摘要（表与代码各说各话）"
    );
    // ③ 已知绿（幂等）：同一份文本算两次必须相同。
    assert_eq!(
        digest,
        yeban_model::AssetHash::of_bytes(canonical.as_bytes()).as_str()
    );
    // ④ 已知红（1 个字符）：摘要必须变。
    let mut flipped = canonical.clone();
    flipped.replace_range(0..1, "X");
    assert_ne!(flipped, canonical, "翻转必须真的改了文本");
    assert_ne!(
        digest,
        yeban_model::AssetHash::of_bytes(flipped.as_bytes()).as_str(),
        "摘要对 1 个字符的改动必须敏感"
    );
    // ⑤ 已知红（键序）：交换一个真实存在的三行块 ⇒ 摘要必须变。
    let swapped = canonical.replacen(
        "  field\n  value\n  reason\n",
        "  value\n  field\n  reason\n",
        1,
    );
    assert_ne!(
        swapped, canonical,
        "规范文本里必须真有那个可交换的三行块（否则这条自证是空的）"
    );
    assert_ne!(
        digest,
        yeban_model::AssetHash::of_bytes(swapped.as_bytes()).as_str(),
        "摘要必须对**键序**敏感（语义表抓不到的那一位）"
    );
    // ⑥ 规范形式的两个规模读数。
    assert_eq!(
        canonical
            .lines()
            .filter(|line| !line.starts_with("  "))
            .count(),
        REASON_SURFACE_REASONS,
        "reason 数"
    );
    assert_eq!(canonical.lines().count(), REASON_SURFACE_LINES, "规范行数");
}

/// **R94：三个扫描器都跳过"注释区间"与"字符串内部"**。
///
/// 为什么需要它：原来三个扫描器只跳过"整行以 `//` 开头"，因此
/// ① 行尾注释里的引文、② 字符串字面量里的 `"reason": "x"` 都会被**当成真站点**。
/// 本判据先喂**合成样本**（已知红／已知绿），再回读真源码的两个规模读数。
///
/// 注入（实测红）：把 `is_skipped` 的判断删掉 ⇒ 合成样本读出 `fakeInString`/`fakeInComment`，红。
#[test]
fn the_reason_scanners_skip_comments_and_string_intervals() {
    // ⭐ R94 的两个已知红形态都在这一份样本里：行尾注释 ＋ 字符串字面量。
    let sample = "fn f() {\n    // 文档里写 \"reason\": \"fakeInComment\",\n    \
                  let s = \"示例: \\\"reason\\\": \\\"fakeInString\\\"\";\n    \
                  let data = serde_json::json!({ \"reason\": \"realOne\" });\n}\n";
    let consts = BTreeMap::new();
    let scanned = reasons_in("<sample>", sample, &consts);
    assert!(
        scanned.rejected.is_empty(),
        "合成样本里的 reason 都落在字符集内, ⛔ 不该有被拒的: {:?}",
        scanned.rejected
    );
    let found = scanned.accepted;
    assert_eq!(
        found.iter().map(String::as_str).collect::<Vec<_>>(),
        vec!["realOne"],
        "只有真站点该被读到（注释与字符串里的引文必须跳过）"
    );
    let payloads = reason_payloads(sample, &consts);
    assert_eq!(payloads.len(), 1, "载荷扫描器同样只该看到一个 reason");
    assert!(payloads.contains_key("realOne"));
    let surface = reason_surface_in(sample, &consts);
    assert_eq!(surface.len(), 1, "规范形式同理");
    assert_eq!(surface[0].0, "realOne");

    // 已知绿：区间掩码**不改变**真源码的读数（规模与内容都必须一致）。
    let files = read_rust_sources(&[manifest_dir().join("src")]);
    let canonical = reason_surface(&files);
    assert_eq!(
        canonical
            .lines()
            .filter(|line| !line.starts_with("  "))
            .count(),
        REASON_SURFACE_REASONS,
        "reason 数不得因为掩码而变"
    );
    assert_eq!(
        canonical.lines().count(),
        REASON_SURFACE_LINES,
        "规范行数不得变"
    );
    assert_eq!(
        yeban_model::AssetHash::of_bytes(canonical.as_bytes()).as_str(),
        FROZEN_REASON_SURFACE_SHA256,
        "加入区间掩码后摘要必须**逐位不变**（否则就是掩码改变了口径）"
    );
}

/// #340 已知形状的**全量自证**：旧扫描器只认 `"reason": "字面量"`，本判据把
/// 变量 / `format!` / 常量 / `map.insert`（两种） / `]=` / `]==` / `=>` / `Value::from` / 参数名逐个喂给它。
///
/// 已知红（实测）：把 [`reason_occurrences`] 换回旧实现 ⇒
/// `local_two` / `REASON_CONST` / `either` / `map.insert` 等读不到，本判据红。
/// 已知红（实测）：把 `resolve_reason_ident` 的常量分支删掉 ⇒ `constThree` 变
/// `Dynamic`，本判据红。
#[test]
fn the_reason_site_scanner_classifies_every_known_shape() {
    let sample = "const REASON_CONST: &str = \"constThree\";\n\
fn f(flag: bool) {\n\
    let local_two = \"localTwo\";\n\
    let either = if flag { \"branchA\" } else { \"branchB\" };\n\
    let dynamic = pick();\n\
    let _ = serde_json::json!({ \"reason\": \"literalOne\" });\n\
    map.insert(\"reason\".to_owned(), Value::from(\"mapFour\"));\n\
    map.insert(\"reason\", \"fromInsertBare\");\n\
    obj[\"reason\"] = Value::from(\"fromIndexAssign\");\n\
    let _ = obj[\"reason\"] == \"literal\";\n\
    let _ = match key { \"reason\" => Value::from(arg), _ => Value::Null };\n\
    let _ = serde_json::json!({ \"reason\": local_two });\n\
    let _ = serde_json::json!({ \"reason\": REASON_CONST });\n\
    let _ = serde_json::json!({ \"reason\": either });\n\
    let _ = serde_json::json!({ \"reason\": dynamic });\n\
    let _ = serde_json::json!({ \"reason\": format!(\"{prefix}Suffix\") });\n\
    param(\"reason\", \"string\", true, \"说明\");\n\
}\n";
    let consts = string_constants(&[("<sample>".to_owned(), sample.to_owned())]);
    assert_eq!(
        consts.get("REASON_CONST").map(String::as_str),
        Some("constThree"),
        "常量解析器必须真的读到 `const NAME: &str = \"...\";`"
    );
    let occurrences = reason_occurrences("<sample>", sample, &consts);
    assert_eq!(occurrences.len(), 12, "样本里恰好 12 次 `\"reason\"` 出现");
    let kind_at = |line: usize| {
        occurrences
            .iter()
            .find(|occurrence| occurrence.line == line)
            .map(|occurrence| occurrence.kind.clone())
    };
    assert_eq!(
        kind_at(6),
        Some(ReasonOccurrenceKind::Literal("literalOne".to_owned()))
    );
    assert_eq!(
        kind_at(7),
        Some(ReasonOccurrenceKind::Literal("mapFour".to_owned()))
    );
    assert_eq!(
        kind_at(8),
        Some(ReasonOccurrenceKind::Literal("fromInsertBare".to_owned()))
    );
    assert_eq!(
        kind_at(9),
        Some(ReasonOccurrenceKind::Literal("fromIndexAssign".to_owned()))
    );
    assert!(
        matches!(kind_at(10), Some(ReasonOccurrenceKind::ArgumentName(_))),
        "obj[\"reason\"] == ... 判定必须归到 ArgumentName (line 410 ]== 穿透到 ] 分支)"
    );
    assert!(
        matches!(kind_at(11), Some(ReasonOccurrenceKind::ArgumentName(_))),
        "\"reason\" => ... match 模式匹配必须归到 ArgumentName (line 394 => 分支)"
    );
    assert_eq!(
        kind_at(12),
        Some(ReasonOccurrenceKind::Resolved(vec!["localTwo".to_owned()]))
    );
    assert_eq!(
        kind_at(13),
        Some(ReasonOccurrenceKind::Literal("constThree".to_owned()))
    );
    assert_eq!(
        kind_at(14),
        Some(ReasonOccurrenceKind::Resolved(vec![
            "branchA".to_owned(),
            "branchB".to_owned()
        ]))
    );
    assert_eq!(
        kind_at(15),
        Some(ReasonOccurrenceKind::Dynamic("dynamic".to_owned()))
    );
    assert_eq!(
        kind_at(16),
        Some(ReasonOccurrenceKind::Dynamic(
            "format!(\"{prefix}Suffix\")".to_owned()
        ))
    );
    assert!(
        matches!(kind_at(17), Some(ReasonOccurrenceKind::ArgumentName(_))),
        "参数名 `reason` 必须归到 ArgumentName, ⛔ 不是故障词"
    );

    // 判据侧的读数：可分类的值真的进了词表；看不见的值**没有**被编造。
    let scanned = reasons_in("<sample>", sample, &consts);
    assert_eq!(
        scanned.accepted,
        [
            "branchA",
            "branchB",
            "constThree",
            "fromIndexAssign",
            "fromInsertBare",
            "literalOne",
            "localTwo",
            "mapFour"
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>(),
        "可解析的值必须全部进词表"
    );
    assert!(
        !scanned.accepted.contains("dynamic")
            && !scanned.accepted.contains("Suffix")
            && !scanned.accepted.contains("literal")
            && !scanned.accepted.contains("arg"),
        "运行时拼接的值与参数名 ⛔ 不许被当成字面量"
    );
}

/// #340 的**牙齿**：一个旧扫描器看不见、新扫描器也归类不了的站点必须**红**。
///
/// 覆盖双重防线：
/// 1. 未登记的动态站点（`{ "reason": pick_reason() }`，归为 `Dynamic`），必须报"没有登记"并点名 file:line；
/// 2. 真正的 `Unclassified` 站点（由 [`reason_occurrences`] 两个未分类分支构造）：
///    - 分支 (a)（line 404）：`.to_owned()` 后面未接逗号（如 `let key = "reason".to_owned();`）；
///    - 分支 (b)（line 419）：其它未知语法（如 `let matched = key == "reason" && ready;`）。
///
///    这两类站点 `values()` 必须为空（不污染词表），且 [`unexplained_reason_sites`] 必须点名
///    file:line 并报"无法归类 `\"reason\"` 站点（<行上下文>）"。
#[test]
fn an_unclassifiable_reason_site_is_red_naming_file_and_line() {
    // 1. 未登记的动态站点（Dynamic 分支）
    let dynamic_sample =
        "fn f() {\n    let data = serde_json::json!({ \"reason\": pick_reason() });\n}\n";
    let dynamic_occurrences = reason_occurrences(
        "crates/yeban-mcp/src/domain/example.rs",
        dynamic_sample,
        &BTreeMap::new(),
    );
    assert_eq!(dynamic_occurrences.len(), 1);
    assert_eq!(dynamic_occurrences[0].line, 2);
    assert!(
        matches!(&dynamic_occurrences[0].kind, ReasonOccurrenceKind::Dynamic(expr) if expr == "pick_reason()"),
        "带有 `:` 但非常量的动态站点必须归类为 Dynamic: {:?}",
        dynamic_occurrences[0].kind
    );
    assert!(
        dynamic_occurrences[0].kind.values().is_empty(),
        "动态站点的 values() 必须为空"
    );
    let dynamic_failures = unexplained_reason_sites(&dynamic_occurrences);
    assert!(
        dynamic_failures.iter().any(|failure| {
            failure.contains("domain/example.rs:2")
                && failure.contains("没有登记")
                && failure.contains("pick_reason()")
        }),
        "未登记的动态站点必须**带 file:line 与'没有登记'**变红: {dynamic_failures:?}"
    );

    // 2. 真正无法归类的站点 分支 (a)：.to_owned() 后面不是逗号
    let unclassified_branch_a_sample = "fn f() {\n    let key = \"reason\".to_owned();\n}\n";
    let occurrences_a = reason_occurrences(
        "crates/yeban-mcp/src/domain/example.rs",
        unclassified_branch_a_sample,
        &BTreeMap::new(),
    );
    assert_eq!(occurrences_a.len(), 1);
    assert_eq!(occurrences_a[0].line, 2);
    assert_eq!(
        occurrences_a[0].kind,
        ReasonOccurrenceKind::Unclassified("let key = \"reason\".to_owned();".to_owned())
    );
    assert!(
        occurrences_a[0].kind.values().is_empty(),
        "无法归类的站点 values() 必须为空"
    );
    let failures_a = unexplained_reason_sites(&occurrences_a);
    let expected_failure_a =
        "domain/example.rs:2 无法归类 `\"reason\"` 站点（let key = \"reason\".to_owned();）";
    assert!(
        failures_a
            .iter()
            .any(|failure| failure == expected_failure_a),
        ".to_owned() 未接逗号的未分类站点必须生成点名 file:line 的红条 `{expected_failure_a}`: {failures_a:?}"
    );

    // 3. 真正无法归类的站点 分支 (b)：后面是其它未知 token（例如 &&）
    let unclassified_branch_b_sample =
        "fn f(key: &str, ready: bool) {\n    let matched = key == \"reason\" && ready;\n}\n";
    let occurrences_b = reason_occurrences(
        "crates/yeban-mcp/src/domain/example.rs",
        unclassified_branch_b_sample,
        &BTreeMap::new(),
    );
    assert_eq!(occurrences_b.len(), 1);
    assert_eq!(occurrences_b[0].line, 2);
    assert_eq!(
        occurrences_b[0].kind,
        ReasonOccurrenceKind::Unclassified("let matched = key == \"reason\" && ready;".to_owned())
    );
    assert!(
        occurrences_b[0].kind.values().is_empty(),
        "无法归类的站点 values() 必须为空"
    );
    let failures_b = unexplained_reason_sites(&occurrences_b);
    let expected_failure_b = "domain/example.rs:2 无法归类 `\"reason\"` 站点（let matched = key == \"reason\" && ready;）";
    assert!(
        failures_b
            .iter()
            .any(|failure| failure == expected_failure_b),
        "其它未知语法的未分类站点必须生成点名 file:line 的红条 `{expected_failure_b}`: {failures_b:?}"
    );
}

/// 针对 #340 遗留的覆盖面缺口：自证 `ReasonOccurrenceKind::Unclassified` 的两个分支，
/// 以及所有关键语法分支（`=>`、裸 `insert`、`]=`、`]==`）。
#[test]
fn unclassified_reason_sites_and_all_key_syntaxes_are_covered() {
    let sample = "fn syntax_coverage() {\n\
    let to_owned_standalone = \"reason\".to_owned();\n\
    let other_token = key == \"reason\" && ready;\n\
    let _ = match key { \"reason\" => Value::from(arg), _ => Value::Null };\n\
    map.insert(\"reason\", \"fromInsertBare\");\n\
    obj[\"reason\"] = Value::from(\"fromIndexAssign\");\n\
    let _ = obj[\"reason\"] == \"literal\";\n\
}\n";
    let consts = BTreeMap::new();
    let occurrences = reason_occurrences("crates/yeban-mcp/src/domain/example.rs", sample, &consts);
    assert_eq!(occurrences.len(), 6, "覆盖 6 种边界语法的 6 处 reason 出现");

    // 1. .to_owned() 后面不是逗号 -> Unclassified
    assert_eq!(
        occurrences[0].kind,
        ReasonOccurrenceKind::Unclassified(
            "let to_owned_standalone = \"reason\".to_owned();".to_owned()
        )
    );
    assert!(occurrences[0].kind.values().is_empty());

    // 2. 其它非已知 token（&&） -> Unclassified
    assert_eq!(
        occurrences[1].kind,
        ReasonOccurrenceKind::Unclassified(
            "let other_token = key == \"reason\" && ready;".to_owned()
        )
    );
    assert!(occurrences[1].kind.values().is_empty());

    // 3. => match 分支 -> ArgumentName
    assert_eq!(
        occurrences[2].kind,
        ReasonOccurrenceKind::ArgumentName(
            "let _ = match key { \"reason\" => Value::from(arg), _ => Value::Null };".to_owned()
        )
    );
    assert!(occurrences[2].kind.values().is_empty());

    // 4. insert("reason", ...) -> Literal
    assert_eq!(
        occurrences[3].kind,
        ReasonOccurrenceKind::Literal("fromInsertBare".to_owned())
    );
    assert_eq!(
        occurrences[3].kind.values(),
        vec!["fromInsertBare".to_owned()]
    );

    // 5. obj["reason"] = ... -> Literal
    assert_eq!(
        occurrences[4].kind,
        ReasonOccurrenceKind::Literal("fromIndexAssign".to_owned())
    );
    assert_eq!(
        occurrences[4].kind.values(),
        vec!["fromIndexAssign".to_owned()]
    );

    // 6. obj["reason"] == ... -> ArgumentName
    assert_eq!(
        occurrences[5].kind,
        ReasonOccurrenceKind::ArgumentName("let _ = obj[\"reason\"] == \"literal\";".to_owned())
    );
    assert!(occurrences[5].kind.values().is_empty());

    // 两个 Unclassified 站点经 unexplained_reason_sites 均精确变红
    let failures = unexplained_reason_sites(&occurrences[0..2]);
    let expected_failure_1 = "domain/example.rs:2 无法归类 `\"reason\"` 站点（let to_owned_standalone = \"reason\".to_owned();）";
    let expected_failure_2 = "domain/example.rs:3 无法归类 `\"reason\"` 站点（let other_token = key == \"reason\" && ready;）";
    assert!(
        failures.iter().any(|f| f == expected_failure_1),
        "分支 1 必须精确生成'无法归类'报错: {failures:?}"
    );
    assert!(
        failures.iter().any(|f| f == expected_failure_2),
        "分支 2 必须精确生成'无法归类'报错: {failures:?}"
    );
}

/// #340 的反向自证：换上**字面量**的新 reason 仍然必须被词表判据抓住。
#[test]
fn a_reason_outside_the_frozen_set_is_still_a_violation() {
    let sample =
        "fn f() {\n    let _ = serde_json::json!({ \"reason\": \"notFrozenAnywhere\" });\n}\n";
    let scanned = reasons_in("<sample>", sample, &BTreeMap::new());
    assert!(scanned.rejected.is_empty());
    let golden: BTreeSet<String> = GOLDEN_REASONS
        .iter()
        .map(|reason| (*reason).to_owned())
        .collect();
    let added: Vec<&str> = scanned
        .accepted
        .difference(&golden)
        .map(String::as_str)
        .collect();
    assert_eq!(
        added,
        vec!["notFrozenAnywhere"],
        "冻结集之外的字面量必须被报成新增（这条判据证明词表比较没有失灵）"
    );
}

/// #340 的**站点普查**：生产区每一次 `"reason"` 出现都必须可分类，
/// 或逐条登记在两张显式表（[`EXEMPT_DYNAMIC_REASON_SITES`] /
/// [`REASON_ARGUMENT_NAME_USES`]）里。
///
/// 为什么这是 #340 的核心：旧扫描器在"值不是字面量"时**什么都不说** ——
/// 它不报错、不计数、不列清单，于是词表绿在一个它没看见的站台上。
/// 本判据让"看不见"必须**有人签字**：新站点 ⇒ 红并点名 file:line；
/// 旧登记项消失 ⇒ 红（陈旧豁免）。
///
/// 注入（实测红）：在 `automation.rs` 的生产区里加
/// `json!({ "reason": dynamic_reason })`（`dynamic_reason` 来自 `String::from(...)`）⇒
/// 旧守卫 6/6 全绿；本判据红并点名 `domain/automation.rs:855`。
#[test]
fn every_reason_occurrence_is_classified_or_explicitly_registered() {
    let files = read_rust_sources(&[manifest_dir().join("src")]);
    assert!(!files.is_empty(), "必须真的读到源码");
    let consts = string_constants(&files);
    let mut occurrences = Vec::new();
    for (path, text) in &files {
        occurrences.extend(reason_occurrences(path, &production_region(text), &consts));
    }

    // 先把"看不见的站点"变成失败读数（带 file:line），再报规模 —— 顺序很重要：
    // 只报计数的话，新站点的位置还得二次搜索。
    let failures = unexplained_reason_sites(&occurrences);
    assert!(
        failures.is_empty(),
        "站点分类失败（新站点必须可分类或**逐条**登记）:\n{}",
        failures.join("\n")
    );

    let literal = occurrences
        .iter()
        .filter(|occurrence| matches!(occurrence.kind, ReasonOccurrenceKind::Literal(_)))
        .count();
    let resolved = occurrences
        .iter()
        .filter(|occurrence| matches!(occurrence.kind, ReasonOccurrenceKind::Resolved(_)))
        .count();
    let dynamic = occurrences
        .iter()
        .filter(|occurrence| matches!(occurrence.kind, ReasonOccurrenceKind::Dynamic(_)))
        .count();
    let propagated = occurrences
        .iter()
        .filter(|occurrence| matches!(occurrence.kind, ReasonOccurrenceKind::Propagated(..)))
        .count();
    let argument = occurrences
        .iter()
        .filter(|occurrence| matches!(occurrence.kind, ReasonOccurrenceKind::ArgumentName(_)))
        .count();
    let unclassified = occurrences
        .iter()
        .filter(|occurrence| matches!(occurrence.kind, ReasonOccurrenceKind::Unclassified(_)))
        .count();

    assert_eq!(
        unclassified, 0,
        "`Unclassified` 必须是 0（上面已经逐条点名）"
    );
    assert_eq!(
        literal + resolved + dynamic + propagated,
        REASON_KEY_SITES_TOTAL,
        "`\"reason\"` **键站点**数（值键 + insert/索引赋值）"
    );
    assert_eq!(
        argument, REASON_ARGUMENT_NAME_TOTAL,
        "`\"reason\"` **参数名**出现数"
    );
    assert_eq!(
        literal + resolved + dynamic + propagated + argument + unclassified,
        REASON_OCCURRENCES_TOTAL,
        "`\"reason\"` 总出现数（键站点 + 参数名 + 未分类）"
    );
    assert_eq!(
        literal, REASON_LITERAL_SITES_TOTAL,
        "字面量值站点数（直接字面量或 `Value::from(\"…\")`）"
    );
    assert_eq!(
        resolved, REASON_RESOLVED_SITES_TOTAL,
        "可解析 `let` 站点数（`let reason = ...`）"
    );
    assert_eq!(dynamic, REASON_DYNAMIC_SITES_TOTAL, "已登记的运行时站点数");
    assert_eq!(
        propagated, REASON_PROPAGATED_SITES_TOTAL,
        "由**调用点**传播出值的站点数（⛔ 不是盲区, 见 `PROPAGATED_REASON_SITES`）"
    );
}

/// ⭐ **传播表自身的牙**：逐条核对 [`PROPAGATED_REASON_SITES`] 的四个读数
/// （声明唯一 ＋ 最后一个形参是 `reason: &str` ＋ 全 crate 调用点数 ＋ 每个调用点的
/// 最后一个实参都是**字符串字面量** ＋ 由此折出的值集合与登记表逐位相等）。
///
/// 为什么单独立一条：站点普查只核对**站点数** —— 调用点的最后一个实参一旦从字面量
/// 变成变量，扫描器会**退回** `Dynamic`，普查只能报"未登记的动态站点"（**结果**红）。
/// 本条报的是**原因**：点名哪一个辅助函数的哪一个调用点不再可传播。
#[test]
fn the_propagated_reason_sites_publish_their_call_site_literals() {
    let files = read_rust_sources(&[manifest_dir().join("src")]);
    for entry in PROPAGATED_REASON_SITES {
        let mut declared: Vec<(&String, &String)> = Vec::new();
        for (path, text) in &files {
            if function_parameters(&production_region(text), entry.helper).is_some() {
                declared.push((path, text));
            }
        }
        let locations: Vec<String> = declared
            .iter()
            .map(|(path, _text)| relative_to_src(path))
            .collect();
        assert_eq!(
            declared.len(),
            1,
            "`{}` 的声明应当**恰好一处**（实测 {locations:?}）；两处同名 ⇒ 传播按名字取值会串味",
            entry.helper
        );
        let (path, text) = declared[0];
        let production = production_region(text);
        let params = function_parameters(&production, entry.helper).expect("上面已经筛出声明");
        let last_parameter = split_top_level_arguments(params)
            .last()
            .copied()
            .unwrap_or_default();
        assert_eq!(
            normalize_expr(last_parameter),
            "reason: &str",
            "`{path}` 里 `{}` 的最后一个形参必须是 `reason: &str`（传播按**位置**取实参）",
            entry.helper
        );

        let mut sites: Vec<(String, HelperCallSite)> = Vec::new();
        for (call_path, call_text) in &files {
            let region = production_region(call_text);
            for site in helper_call_sites(&region, entry.helper) {
                sites.push((relative_to_src(call_path), site));
            }
        }
        assert_eq!(
            sites.len(),
            entry.call_sites,
            "`{}` 的调用点数与登记表不一致（登记 {}, 实测 {}）",
            entry.helper,
            entry.call_sites,
            sites.len()
        );
        let not_literal: Vec<String> = sites
            .iter()
            .filter(|(_file, site)| site.literal.is_none())
            .map(|(file, site)| {
                format!("{}:{} 的最后一个实参 `{}`", file, site.line, site.argument)
            })
            .collect();
        assert!(
            not_literal.is_empty(),
            "`{}` 有调用点的最后一个实参**不是**字符串字面量, 传播因此失效: {not_literal:?}",
            entry.helper
        );
        let mut values: Vec<String> = sites
            .iter()
            .filter_map(|(_file, site)| site.literal.clone())
            .map(|value| format!("{value}{}", entry.suffix))
            .collect();
        values.sort();
        values.dedup();
        assert_eq!(
            values, entry.literals,
            "`{}`（后缀 `{}`）传播出来的词表与登记表不一致",
            entry.helper, entry.suffix
        );
        for value in entry.literals {
            assert!(
                GOLDEN_REASONS.iter().any(|reason| reason == value),
                "传播出来的 `{value}` 没有登记进 `GOLDEN_REASONS`"
            );
        }
    }
}
