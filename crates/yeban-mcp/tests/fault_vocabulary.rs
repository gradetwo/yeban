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

//! ## 冻结摘要与文档表（R70②/R78④：两方向）
//!
//! `reason` 面的**规范文本**（文件按路径排序 → reason 按首次出现顺序 → 键按源码顺序）
//! 的 SHA-256 是下面这个字面量。判据 `the_reason_surface_digest_is_frozen_and_documented`
//! 同时核对**两个方向**：`sha256(规范文本) == 常量` **且** 本表 `contains(常量)`
//! （用 `include_str!` 把本文件读回来）—— 表与代码不许各说各话。
//!
//! | 面 | reason 数 | 规范行数 | SHA-256 |
//! | :--- | ---: | ---: | :--- |
//! | `reason`（字面 `json!` 第一层键，含顺序） | 136 | 610 | `41e1c7a3d50b11f845a38add94411fb360efacf2063ce64e64455d7ffa52ffcb` |

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use yeban_mcp::undo_session::read_rust_sources;

/// 本轮开工基线上**生产区**里出现的全部 `reason` 字面量（排序）。
/// 这是黄金表：任何改名 / 新增 / 删除都必须**故意**改这一行。
pub const GOLDEN_REASONS: [&str; 134] = [
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
    "clipAlreadyExists",
    "createIsNotReadOnly",
    "createOnlyParameter",
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
    "framesExceedCap",
    "indexTooLarge",
    "invalidDeviceDefinition",
    "laneMustBeString",
    "lanePropertiesNotApplicableToPointRemoval",
    "latencyOutOfRange",
    "licenseNotAllowed",
    "licenseRequired",
    "licenseWhitelistUnavailable",
    "loudnessTargetMissed",
    "masterBusNodeCannotBeRemoved",
    "masterBusTrackCannotBeRemoved",
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
    "recordDrainFailed",
    "removeClipIsNotPlacement",
    "removeClipTakesNoOtherOps",
    "removeTakesNoProperties",
    "removeTrackIsNotPlacement",
    "removeTrackTakesNoOtherOps",
    "routingEdgeAlreadyExists",
    "routingEdgeNotFound",
    "routingNodeAlreadyExists",
    "routingNodeNotFound",
    "sceneAlreadyExists",
    "sceneNameRequiredWhenCreating",
    "sceneNotFound",
    "sceneTempoMustBeNumberOrNull",
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
    "unknownFlagField",
    "unknownInsertDeviceField",
    "unknownLaneField",
    "unknownLaneTarget",
    "unknownLaunchQuantization",
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

/// 扫出 `"reason": "<字面量>"` 的全部字面量，并**分别**报出接受与拒掉的数量。
///
/// 过滤规则（**保留**，⛔ 不放宽）：值必须非空、且全部落在 `[A-Za-z0-9_]`。
/// 被它拒掉的值进 [`Reasons::rejected`]，⛔ 不再静默丢弃。
fn reasons_in(source: &str, text: &str) -> Reasons {
    let bytes = text.as_bytes();
    let needle = b"\"reason\"";
    let mut accepted = BTreeSet::new();
    let mut rejected: Vec<(String, String, &'static str)> = Vec::new();
    let mask = skipped_offsets(text);
    let mut from = 0usize;
    while let Some(offset) = find(&bytes[from..], needle) {
        let at = from + offset;
        // ⭐ R94：命中的 `"reason"` 若落在注释／字符串**内部** ⇒ 跳过（提取仍用原文）。
        if is_skipped(&mask, at) {
            from = at + needle.len();
            continue;
        }
        let start = at + needle.len();
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
                    accepted.insert(value.to_owned());
                } else {
                    // ⚠ 这里曾经是**静默丢弃**。现在它进显式读数，由判据断言为 0。
                    rejected.push((
                        value.to_owned(),
                        source.to_owned(),
                        if value.is_empty() {
                            "emptyReasonValue"
                        } else {
                            "reasonValueOutsideTheAlphanumericUnderscoreSet"
                        },
                    ));
                }
            }
        }
        from = start;
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

    let mut rejected: Vec<(String, String, &'static str)> = Vec::new();
    let mut accepted_total = 0usize;
    for (_path, text) in &files {
        // ⚠ 生产区与测试区**都**要扫：过滤在两处是同一条代码路径。
        for region in [production_region(text), test_region(text)] {
            let scanned = reasons_in(_path, &region);
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
    let mut found = BTreeSet::new();
    for (path, text) in &files {
        found.extend(reasons_in(path, &production_region(text)).accepted);
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
    for (path, text) in &src {
        mentioned.extend(reasons_in(path, &test_region(text)).accepted);
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
pub const GOLDEN_REASON_PAYLOADS: [(&str, &[&str]); 134] = [
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
    ("recordDrainFailed", &["ioError", "reason"]),
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
    ("routingEdgeAlreadyExists", &["edgeId", "hint", "reason"]),
    ("routingEdgeNotFound", &["edgeId", "hint", "reason"]),
    ("routingNodeAlreadyExists", &["hint", "nodeId", "reason"]),
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

fn reason_payloads(text: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let joined = text;
    let bytes = joined.as_bytes();
    // R94：逐命中点判断是否落在注释／字符串内部（⛔ 不再只跳过"整行以 `//` 开头"）。
    let mask = skipped_offsets(joined);
    let mut line_start = 0usize;
    while line_start < joined.len() {
        let line_end = joined[line_start..]
            .find('\n')
            .map_or(joined.len(), |skip| line_start + skip);
        let line = &joined[line_start..line_end];
        {
            let mut cursor = 0usize;
            while let Some(offset) = find(&line.as_bytes()[cursor..], b"\"reason\"") {
                let at = cursor + offset;
                if is_skipped(&mask, line_start + at) {
                    cursor = at + 1;
                    continue;
                }
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

/// **`reason` 面的冻结摘要**（R70②：`assert_eq!(第一次, 第二次)` 是**自比**，
/// 不是字节契约 —— 字节契约必须钉**字面摘要**）。
///
/// ⚠ 文档表（两方向之二）：本字面量必须同时出现在**本文件的模块文档表**里，
/// 判据用 `include_str!` 把本文件读回来核对（表与代码不许各说各话）。
/// 规范形式：**文件按路径排序** → 每个 `reason` 按**首次出现**顺序 →
/// 每个键按**源码顺序**（只收字面 `json!` 对象第一层的键）。
pub const FROZEN_REASON_SURFACE_SHA256: &str =
    "41e1c7a3d50b11f845a38add94411fb360efacf2063ce64e64455d7ffa52ffcb";

/// 规范形式里的 reason 数 / 规范行数（与摘要一起构成"两端"读数）。
pub const REASON_SURFACE_REASONS: usize = 138;
/// 规范行数（每个 reason 一行 ＋ 每个键一行）。
pub const REASON_SURFACE_LINES: usize = 619;

/// 单文件里的 `reason` 面：**保留出现顺序**的 `(reason, keys)`。
///
/// 与 [`reason_payloads`] 的唯一区别是"顺序"：那个返回 `BTreeMap`（排序 ⇒
/// 对键序不敏感），这个保留源码顺序 ⇒ 对**键序**敏感（摘要要的就是这一位）。
fn reason_surface_in(text: &str) -> Vec<(String, Vec<String>)> {
    let region = production_region(text);
    let mut order: Vec<(String, Vec<String>)> = Vec::new();
    let mask = skipped_offsets(&region);
    let mut line_start = 0usize;
    while line_start < region.len() {
        let line_end = region[line_start..]
            .find('\n')
            .map_or(region.len(), |skip| line_start + skip);
        let line = &region[line_start..line_end];
        {
            let mut cursor = 0usize;
            while let Some(offset) = find(&line.as_bytes()[cursor..], b"\"reason\"") {
                let at = cursor + offset;
                if is_skipped(&mask, line_start + at) {
                    cursor = at + 1;
                    continue;
                }
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
                        .position(|byte| *byte == b'"')
                    {
                        let reason = line[value_start..value_start + len].to_owned();
                        let absolute = line_start + at;
                        let mut keys: Vec<String> = Vec::new();
                        if let Some(open) = region[..absolute].rfind("json!(") {
                            keys = json_object_key_order(&region[open + "json!".len()..]);
                        }
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
                cursor = at + 1;
            }
        }
        if line_end >= region.len() {
            break;
        }
        line_start = line_end + 1;
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
    let mut order: Vec<(String, Vec<String>)> = Vec::new();
    let mut sorted: Vec<&(String, String)> = files.iter().collect();
    sorted.sort_by(|left, right| left.0.cmp(&right.0));
    for (_path, text) in sorted {
        for (reason, keys) in reason_surface_in(text) {
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
    assert_eq!(files.len(), 38, "生产区 `.rs` 文件数（规范形式的分母之一）");
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
    let scanned = reasons_in("<sample>", sample);
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
    let payloads = reason_payloads(sample);
    assert_eq!(payloads.len(), 1, "载荷扫描器同样只该看到一个 reason");
    assert!(payloads.contains_key("realOne"));
    let surface = reason_surface_in(sample);
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
