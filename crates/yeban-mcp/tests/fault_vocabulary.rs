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
//! 本文件把这件事变成机制，用**两条**判据：
//!
//! | 判据 | 性质 | 它挡住的改动 |
//! | :--- | :--- | :--- |
//! | [`the_reason_vocabulary_is_the_published_list`] | 生产区的 `reason` 字面量集合 **==** 黄金表 | 偷偷改名 / 新增 / 删除一个 `reason` |
//! | [`every_reason_is_asserted_somewhere`] | 黄金表里的每一个都被**某条判据**提到（少数登记为不可达） | 新增一个零覆盖的 `reason` |
//!
//! ⚠ 口径（刻意宽松，因此"零覆盖"是**下界**，不会把已覆盖的误报为缺口）：
//! 判据侧 = 任何一处 `"<reason>"` 字面量出现（`assert_eq!` / `json!` / 注释都算）。
//!
//! ## 登记为"本平台不可达"的那一个
//!
//! `indexTooLarge` 只在 `usize` 窄于 `u64` 的平台上可达
//! （`read_usize` 做 `usize::try_from(u64)`；64 位平台上这一步**永不失败**）。
//! 因此它在本机与 CI 的 64 位腿上不可观测，**如实登记**而不是硬造一条假判据。

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use yeban_mcp::undo_session::read_rust_sources;

/// 本轮开工基线上**生产区**里出现的全部 `reason` 字面量（排序）。
/// 这是黄金表：任何改名 / 新增 / 删除都必须**故意**改这一行。
pub const GOLDEN_REASONS: [&str; 84] = [
    "assetChannelLayout",
    "assetDecodeFailed",
    "assetHashMismatch",
    "assetMissing",
    "assetNotInPool",
    "automationLaneNotFound",
    "automationPointNotFound",
    "automationTickAmbiguous",
    "clipAlreadyExists",
    "createIsNotReadOnly",
    "createOnlyParameter",
    "createRequiresAddOps",
    "deviceIdAlreadyInTrack",
    "deviceNameRequired",
    "deviceNotFound",
    "deviceSlotOutOfRange",
    "deviceTrackNotFound",
    "domainMustBeObjectOrNull",
    "duplicateLaneTarget",
    "duplicateNoteId",
    "duplicateSceneTarget",
    "durationNotDerivable",
    "edgeIdRequired",
    "indexTooLarge",
    "invalidDeviceDefinition",
    "laneMustBeString",
    "lanePropertiesNotApplicableToPointRemoval",
    "latencyOutOfRange",
    "loudnessTargetMissed",
    "masterBusNodeCannotBeRemoved",
    "masterBusTrackCannotBeRemoved",
    "moveRequiresStartTick",
    "nonFiniteValue",
    "pcmBudgetExceeded",
    "placementAlreadyAtStartTick",
    "placementAlreadyExists",
    "placementClipMismatch",
    "placementIdConflict",
    "placementIsNotCreation",
    "placementNotFound",
    "placementWithoutTrack",
    "pointAddressIsAmbiguous",
    "pointAddressRequired",
    "projectAlreadyExists",
    "removeClipIsNotPlacement",
    "removeClipTakesNoOtherOps",
    "removeTakesNoProperties",
    "removeTrackIsNotPlacement",
    "removeTrackTakesNoOtherOps",
    "routingEdgeNotFound",
    "routingNodeNotFound",
    "sceneAlreadyExists",
    "sceneNameRequiredWhenCreating",
    "sceneNotFound",
    "sceneTempoMustBeNumberOrNull",
    "sectionNotFound",
    "staticLaneNotApplicable",
    "tempoUnusable",
    "trackNotFound",
    "unknownDeviceField",
    "unknownDeviceKind",
    "unknownDeviceParamField",
    "unknownDisconnectRoutingField",
    "unknownFlagField",
    "unknownInsertDeviceField",
    "unknownLaneField",
    "unknownLaneTarget",
    "unknownPlacementKind",
    "unknownPointField",
    "unknownPointRemovalField",
    "unknownRemoveClipField",
    "unknownRemoveDeviceField",
    "unknownRemoveRoutingNodeField",
    "unknownRemoveSceneField",
    "unknownRemoveSectionField",
    "unknownRemoveTrackField",
    "unknownRoutingGainField",
    "unknownSetSceneField",
    "unknownSetSceneOpField",
    "unknownStaticLane",
    "unknownWriteMode",
    "valueMustBeBoolean",
    "valueMustBeNumber",
    "writeModeMustBeString",
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

/// 扫出 `"reason": "<字面量>"` 的全部字面量。
fn reasons_in(text: &str) -> BTreeSet<String> {
    let bytes = text.as_bytes();
    let needle = b"\"reason\"";
    let mut out = BTreeSet::new();
    let mut from = 0usize;
    while let Some(offset) = find(&bytes[from..], needle) {
        let start = from + offset + needle.len();
        let mut index = start;
        // 跳过空白与冒号
        while index < bytes.len() && (bytes[index] == b' ' || bytes[index] == b':') {
            index += 1;
        }
        if index < bytes.len() && bytes[index] == b'"' {
            let value_start = index + 1;
            if let Some(end) = bytes[value_start..].iter().position(|byte| *byte == b'"') {
                let value = &text[value_start..value_start + end];
                if !value.is_empty()
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                {
                    out.insert(value.to_owned());
                }
            }
        }
        from = start;
    }
    out
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len())
        .find(|index| &haystack[*index..*index + needle.len()] == needle)
}

#[test]
fn the_reason_vocabulary_is_the_published_list() {
    let files = read_rust_sources(&[manifest_dir().join("src")]);
    assert!(!files.is_empty(), "必须真的读到源码");
    let mut found = BTreeSet::new();
    for (_path, text) in &files {
        found.extend(reasons_in(&production_region(text)));
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
    // 判据侧 = 单元判据区 + 集成判据文件（**宽松**：出现字面量即算"提到"）。
    let mut mentioned: BTreeSet<String> = BTreeSet::new();
    for (_path, text) in &src {
        mentioned.extend(reasons_in(&test_region(text)));
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
pub const GOLDEN_REASON_PAYLOADS: [(&str, &[&str]); 84] = [
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
    (
        "assetMissing",
        &["asset", "declaredInIndex", "hint", "reason"],
    ),
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
    ("clipAlreadyExists", &["clipId", "hint", "reason"]),
    ("createIsNotReadOnly", &["reason"]),
    ("createOnlyParameter", &["parameters", "reason"]),
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
    ("domainMustBeObjectOrNull", &["field", "reason"]),
    ("duplicateLaneTarget", &["lane", "reason"]),
    ("duplicateNoteId", &["noteId", "reason"]),
    ("duplicateSceneTarget", &["hint", "reason", "sceneId"]),
    (
        "durationNotDerivable",
        &["clipId", "field", "isMidi", "reason"],
    ),
    ("edgeIdRequired", &["field", "reason"]),
    ("indexTooLarge", &["field", "reason", "value"]),
    ("invalidDeviceDefinition", &["deviceId", "reason"]),
    ("laneMustBeString", &["allowed", "field", "reason"]),
    (
        "lanePropertiesNotApplicableToPointRemoval",
        &["reason", "supportedPointFields", "unsupportedFields"],
    ),
    ("latencyOutOfRange", &["field", "max", "reason", "value"]),
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
    (
        "moveRequiresStartTick",
        &["field", "placementKind", "reason"],
    ),
    (
        "nonFiniteValue",
        &["field", "narrowedToF32", "reason", "value"],
    ),
    (
        "pcmBudgetExceeded",
        &["budget", "budgetDetail", "budgetGate", "reason"],
    ),
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
    ("placementIsNotCreation", &["hint", "reason"]),
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
    ("removeClipIsNotPlacement", &["hint", "reason"]),
    ("removeClipTakesNoOtherOps", &["hint", "opKinds", "reason"]),
    (
        "removeTakesNoProperties",
        &["field", "properties", "reason"],
    ),
    ("removeTrackIsNotPlacement", &["hint", "reason", "trackId"]),
    (
        "removeTrackTakesNoOtherOps",
        &["hint", "opKinds", "reason", "trackId"],
    ),
    ("routingEdgeNotFound", &["edgeId", "hint", "reason"]),
    ("routingNodeNotFound", &["hint", "nodeId", "reason"]),
    ("sceneAlreadyExists", &["hint", "reason", "sceneId"]),
    (
        "sceneNameRequiredWhenCreating",
        &["field", "hint", "reason"],
    ),
    ("sceneNotFound", &["hint", "reason", "sceneId"]),
    (
        "sceneTempoMustBeNumberOrNull",
        &["field", "reason", "received"],
    ),
    ("sectionNotFound", &["hint", "reason", "sectionId"]),
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
        "unknownWriteMode",
        &["allowed", "field", "reason", "received"],
    ),
    ("valueMustBeBoolean", &["field", "reason", "received"]),
    ("valueMustBeNumber", &["field", "reason"]),
    ("writeModeMustBeString", &["allowed", "field", "reason"]),
];

/// **非标识符形状**的 `reason` 值（成功路径上的说明性字段），4 条。
///
/// 为什么单列一张表：上面 84 条的黄金表按口径只收**标识符形状**
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
fn reason_payloads(text: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let joined = text;
    let bytes = joined.as_bytes();
    // 逐行找 `"reason"`，但跳过注释行（文档注释里写了 `"reason": "x"` 不算）。
    let mut line_start = 0usize;
    while line_start < joined.len() {
        let line_end = joined[line_start..]
            .find('\n')
            .map_or(joined.len(), |skip| line_start + skip);
        let line = &joined[line_start..line_end];
        let trimmed = line.trim_start();
        if !trimmed.starts_with("//") {
            let mut cursor = 0usize;
            while let Some(offset) = find(&line.as_bytes()[cursor..], b"\"reason\"") {
                let at = cursor + offset;
                let mut index = at + b"\"reason\"".len();
                while index < line.len()
                    && (line.as_bytes()[index] == b' ' || line.as_bytes()[index] == b':')
                {
                    index += 1;
                }
                if index < line.len() && line.as_bytes()[index] == b'"' {
                    let value_start = index + 1;
                    if let Some(len) = line.as_bytes()[value_start..]
                        .iter()
                        .position(|b| *b == b'"')
                    {
                        let reason = line[value_start..value_start + len].to_owned();
                        let absolute = line_start + at;
                        out.entry(reason).or_default();
                        if let Some(open) = joined[..absolute].rfind("json!(") {
                            let keys = json_object_keys(&joined[open + "json!".len()..]);
                            out.get_mut(&line[value_start..value_start + len])
                                .expect("刚插入")
                                .extend(keys);
                        }
                    }
                }
                cursor = at + 1;
            }
        }
        if line_end >= joined.len() {
            break;
        }
        line_start = line_end + 1;
        let _ = bytes;
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
    let mut found: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (_path, text) in &files {
        for (reason, keys) in reason_payloads(&production_region(text)) {
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
