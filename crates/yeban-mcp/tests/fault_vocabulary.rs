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

use std::collections::BTreeSet;
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
