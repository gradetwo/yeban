//! **字段写点可达性登记（S3 第一片）** —— 把「哪些持久化字段在 `Op` 全集里**一个触点都没有**」
//! 这件事**机械化**，而不是留在一次性的口头普查里。
//!
//! [MCP-TOOL-005] · [MODEL-AST-002]
//!
//! # 这个数字在数什么（先说指标）
//!
//! 对象＝ [`crate::ops::Op`] 的**产品区**（`ops.rs` 里 `#[cfg(test)]` 之前的全部文本：
//! `apply`／`validate`／`invert`／`kind_name` 都在其中）；单位＝**形如 `.<field>` 的引用次数**
//! （不是行数、不是字符数）。次数为 **0** ⇒ 该字段在整个 `Op` 机器里**连读都没被读过**，
//! 更没有任何写点。
//!
//! # 为什么用「零触点」而不是「有没有写」
//!
//! 2026-10-29 实测：想要更宽的口径（"这字段能被改吗"）时，词法扫描在**本仓库**不可构造 ——
//! 三种假阴性机理各有一处真实证据：
//!
//! 1. **rustfmt 链式换行**：写点形如 `doc\n    .tracks\n    .insert(...)`
//!    （`src/ops.rs` 的 `.tracks` 单独成行），逐行匹配把"可达"读成"不可达"；
//! 2. **领域 helper**：`Self::AddTrack { track } => doc.insert_track(track.clone())`
//!    （`ops.rs:3043`）—— 字段名 `tracks` 根本不出现在 `apply` 臂里；
//! 3. **局部绑定后赋值**：`SetMacro` 先 `let track = doc.track_mut(track_id)?`，
//!    再 `track.macros.get_mut(*macro_index)`、最后写 `macro_parameter.value`
//!    （`ops.rs:3247..3256`）—— 赋值左侧是**局部变量**的字段，不是 `macros`。
//!
//! ⇒ 反过来「**整个产品区连 `.<field>` 都没有**」是**够用且可靠**的方向：上面三种形态
//! 全都要求先出现 `.<field>`，所以零触点结论不受这些机理影响。⚠ **本判据的已知盲点**
//! 登记在下面第二条判据的文档里（helper 若把写点搬到 `project.rs`，本扫描看不见）。
//!
//! # 这片⛔ 不回答"该不该补"
//!
//! 下面钉住的**五**个字段，每个都**只**登记为「现状：`Op` 面零触点」。是否要做成产品能力
//! （轨道文件夹、工程种子可改、描述/标签可编辑…）是**人类裁决**，留痕进 `docs/adr/`，
//! 不由本文件发明答案。判据的作用：等谁真去加那条 `Op` 时，这里会**响亮地变红**，
//! 逼他把登记改过来，而不是让"改不了"这件事重新变成口口相传。
//!
//! ⭐ **本片是词法口径，另有差分口径**：`src/ops.rs` 测试模块里 2026-10-29 落地的那组
//! 差分判据（`every_op_variant_is_covered_by_the_write_differential_scan` 等七条，
//! 判"施加每个变体之后**值变了没**"，含一条用**人造写者**自证检测器不瞎的判据；
//! 2026-10-30 又补**判据七＝末端字段棘轮**，因为只钉顶层根键会放行"根键不变、子字段多写"
//! 那一类漂移 —— 那张表的读数与⛔ 本片的词法读数**。
//! 两片口径不同、互相独立 ⇒ 谁真去补那条 `Op`，两片会**同时**红，**必须一起改**，
//! ⛔ 删任一片来让它绿。

use std::collections::BTreeMap;

/// 被扫描的唯一权威文本：编译期固定，⛔ 不用 `read_dir`／运行期拼路径。
const OPS: &str = include_str!("../src/ops.rs");

/// **零触点字段**（现状登记）。每一项是 `.<字段名>` 在产品区出现 **0** 次。
///
/// 理由一栏写的是**为什么现在是零**，不是"永远该是零"：
/// - `metadata`：`ProjectMetadata` 的 `description` / `tags` / 时间戳，`Op` 面只能改顶层
///   `title`（`SetTitle`）与 `author`（`SetAuthor`）；
/// - `rng_seed`：工程随机种子只在**建造／导入**时给值（`import_logic.rs` 那条路径），
///   开完工程就固定；
/// - `audio_config`：采样率／块长／位深／pan law 四个字段无 `Op`（很可能是产品边界，
///   ⚠ 未裁）；
/// - `folder_id`：轨道文件夹结构——`SetTrackName`／`SetTrackMute`／`SetTrackSolo` 三处
///   注释明文「⛔ 不碰 `folder_id`」；
/// - `solo_safe`：同上，只在注释里出现，没有触点。
const ZERO_TOUCH_FIELDS: [(&str, &str); 5] = [
    (
        "metadata",
        "只能改顶层 title/author；description/tags/时间戳无 Op",
    ),
    ("rng_seed", "只在建造/导入时给值，开工程后不可改"),
    (
        "audio_config",
        "采样率/块长/位深/pan law 无 Op（产品边界未裁）",
    ),
    ("folder_id", "三处注释明文 ⛔ 不碰；文件夹结构不可表达"),
    ("solo_safe", "只出现在注释里，没有任何触点"),
];

/// **探针字段**：这些字段**有**触点，用来证明扫描器不是瞎的（非真空下界）。
///
/// ⚠ 选这三个不是因为它们"重要"，而是因为它们的触点**恰好覆盖了三种假阴性机理里
/// 词法扫描仍能看见的那一类**：`routing_graph` 有 12 处 `.routing_graph`，
/// `macros` 有 5 处，`master_bus_track_id` 有 1 处（`ops.rs:1634` 的自洽性校验）。
const PROBE_FIELDS: [&str; 3] = ["routing_graph", "macros", "master_bus_track_id"];

/// 产品区文本：`#[cfg(test)]` 之前。锚点找不到就直接 panic（⛔ 静默给空集）。
fn production_region() -> String {
    let cut = OPS.find("#[cfg(test)]").unwrap_or_else(|| {
        panic!("抽取规则失效：`ops.rs` 里找不到锚点 `#[cfg(test)]` ⇒ 无法界定产品区")
    });
    OPS[..cut].to_owned()
}

/// 去掉注释后的产品区。口径：整行以 `//` 起始的丢掉，行尾 `//` 之后的部分丢掉。
///
/// ⚠ 不处理块注释——本文件靠 [`no_block_comments_in_production_region`] 把这条简化
/// **钉成前提**，而不是假设它成立。
fn uncommented(region: &str) -> String {
    region
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                String::new()
            } else {
                line.split("//").next().unwrap_or("").to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 数 `.<name>` 的出现次数，带标识符边界（`.macros` 不算 `.macros2`）。
fn count_dotted_sites(text: &str, name: &str) -> usize {
    let bytes = text.as_bytes();
    let needle = name.as_bytes();
    let mut hits = 0usize;
    let mut from = 0usize;
    while let Some(rel) = text[from..].find('.') {
        let at = from + rel;
        let value_at = at + 1;
        let after = value_at + needle.len();
        let name_matches = bytes.get(value_at..after) == Some(needle);
        // 字段名后一个字节必须**不是**标识符字符；已到文末同样算边界成立。
        let boundary_ok = match bytes.get(after) {
            None => true,
            Some(c) => !c.is_ascii_alphanumeric() && *c != b'_',
        };
        if name_matches && boundary_ok {
            hits += 1;
        }
        from = at + 1;
    }
    hits
}

#[test]
fn no_block_comments_in_production_region() {
    // 把 `uncommented()` 的简化前提钉住：**去注释之后**若还出现 `/*`，逐行口径就不再成立，
    // 必须先改扫描器再改这里。
    //
    // ⚠ 为什么检查作用在去注释**之后**：现读 `ops.rs:31` 的 `//!` 行里写着
    // ``schemas/**`` —— 那是对 shell glob 的引用，不是块注释起始。对**原文**断言会被它
    // 假红（本轮实测过一次）。
    let text = uncommented(&production_region());
    assert!(
        !text.contains("/*"),
        "去注释后的产品区仍出现 `/*` ⇒ `uncommented()` 的逐行简化失效，请改扫描器（本判据是它的承重前提）"
    );
    assert!(
        production_region().contains("fn apply"),
        "产品区里没有 `fn apply` ⇒ 界定方式失效（扫到的是空区或错区）"
    );
}

#[test]
fn the_zero_touch_fields_are_still_untouched_and_the_list_is_not_degenerate() {
    let text = uncommented(&production_region());
    // 非真空下界：判据清单不许被悄悄删空。
    assert!(
        ZERO_TOUCH_FIELDS.len() >= 5,
        "零触点清单只剩 {} 项 ⇒ 先确认是不是有人删了条目而不是补了 Op",
        ZERO_TOUCH_FIELDS.len()
    );
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for (field, _reason) in ZERO_TOUCH_FIELDS {
        counts.insert(field, count_dotted_sites(&text, field));
    }
    let nonzero: Vec<(&str, usize)> = counts
        .iter()
        .filter(|(_f, c)| **c > 0)
        .map(|(f, c)| (*f, *c))
        .collect();
    assert!(
        nonzero.is_empty(),
        "这些字段在 `Op` 产品区里出现了写/读点: {nonzero:?} ⇒ 有人加了对应 `Op` \
         —— 请把它们从 `ZERO_TOUCH_FIELDS` 移到探针表，并把『现状登记』升级成正式裁决"
    );
}

#[test]
fn the_probe_fields_have_sites_so_the_scanner_is_alive() {
    let text = uncommented(&production_region());
    for field in PROBE_FIELDS {
        let hits = count_dotted_sites(&text, field);
        assert!(
            hits > 0,
            "探针字段 `{field}` 读到 0 处触点 ⇒ 扫描器或产品区界定已失效（本判据专门防这条判据假绿）"
        );
    }
}

/// 冻结读数（棘轮基线）：探针字段的触点次数**不许莫名下降**。
///
/// 为什么钉这个：`routing_graph` 的 12 处里包含 `apply` 与 `validate` 两侧。若有人把路由
/// 改成走 helper（机理 2），词法读数会**下降而功能不变**，本判据先红，逼他要么把口径改掉
/// 并说明，要么保留触点——而不是让扫描器静默失去意义。
#[test]
fn the_probe_site_counts_do_not_silently_drop() {
    let text = uncommented(&production_region());
    const FROZEN: [(&str, usize); 3] = [
        ("routing_graph", 12),
        ("macros", 5),
        ("master_bus_track_id", 1),
    ];
    for (field, floor) in FROZEN {
        let hits = count_dotted_sites(&text, field);
        assert!(
            hits >= floor,
            "`{field}` 触点从冻结下界 {floor} 掉到 {hits} ⇒ 写点被藏进 helper 或被删，\
             请先说明是哪一种（本判据是上条判据的下界补充）"
        );
    }
}
