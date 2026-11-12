//! 第六轮 UI 完善判据（`[PATCH UI-A11Y-007]`，2026-11-09，基线 `f2963f44`）。
//!
//! ## 本轮要证的三句话
//!
//! 1. **全仓闭环**：`ui/**/*.slint` 的 **238 个可寻址节点块**（带 `accessible-id`
//!    的元素）里，本轮之前恰好有 **4 个**没写显式 `accessible-role`；补齐后
//!    **0 缺口**。本判据把「有 id ⇒ 必须有显式 role」升级为**全仓常驻门禁**，
//!    以后谁新增可寻址节点却忘了 role，立刻红。
//! 2. **声明面 ↔ 注册表逐字对齐**：那 4 个节点在 `src/elements.rs` 的注册表里
//!    早已被钉成 `ElementKind::Text`（3 个）／`ElementKind::TextInput`（1 个），
//!    执行面 `ui/tree` 的角色正是从注册表投影的；但 `.slint` 声明面没写 role
//!    ⇒ 两个面**不一致**。本轮补上的 role 字面必须与注册表的 kind 映射一致
//!    （`Text` → `text`、`TextInput` → `text-input`）。
//! 3. **零副作用**：本轮只加 4 行 `accessible-role`（＋ 4 行说明注释），
//!    ⛔ 不新增 `accessible-id`、⛔ 不新增 `accessible-value`（不造第二份读数）、
//!    ⛔ 不新增快捷键承诺、⛔ 不动任何几何 ⇒ 默认帧逐像素不变、金帧不受影响。
//!
//! ## 为什么这 4 个是真缺口（而不是「Slint 会自动补」）
//!
//! * `UiText` / `UiTitleText` 都 `inherits Text`；Slint 里 `Text` 的默认
//!   `accessible-role` 是 `none` ⇒ 未显式声明时**不进原生无障碍树**，真实屏幕
//!   阅读器读不到「模块入口」标题、MIDI 采集结果与动作回执这三行。同文件的
//!   `module-clipping-warning` / `module-import-losses` 早已显式写 `accessible-role: text`
//!   ⇒ 这 3 个是**同文件内的漏网孤例**，不是设计选择。
//! * `track-{i}-sampler-path` 是 `TextInput`：全仓 7 个 `TextInput` 里，其余
//!   5 个带 id 的都显式写了 `accessible-role: text-input`，只有它漏了 ⇒ 同样是孤例。
//!   （`transport.slint` 那个 `TextInput` **无 id**，不进执行面／注册表，不在本轮范围。）
//! * ⚠ 诚实登记：本仓的**执行面 `ui/tree` 是从注册表投影的**，并不消费 `.slint`
//!   的 `accessible-role` ⇒ 本轮修复**不会改变任何既有判据的读数**。它修的是
//!   **声明面与注册表的一致性**，以及**真实 Slint 应用运行时原生无障碍树**的可见性；
//!   这两者本仓都没有运行时读数，故判据落在**源码文本**上（与 `[UI-A11Y-006]`
//!   同款口径），⛔ 不冒充「人耳可闻」。
//!
//! ## 解析口径（⛔ 不数总数比值，逐块判定）
//!
//! 先把 `//` 行注释与 `"..."` 字符串字面量按**等长字节**替换成空格（注释里提到
//! `accessible-id` 的字样、字符串里的花括号都不算结构），再做大括号配对；每个
//! `{...}` 块只取**本层**文本（更深层子块的内容丢弃），本层含 `accessible-id:`
//! 即为可寻址节点，本层必须**同时**含 `accessible-role:`。
//! 清单用编译期固定的 `include_str!`（与 `elements.rs::SLINT_MANIFEST` 同款纪律，
//! ⛔ 不用 `read_dir` 做主体扫描，避免夹具缺失导致真空），并由第 ④ 条判据把
//! 两份清单钉成同步。
//!
//! ## 平台约束与其它纪律
//!
//! * 全部是**纯源码文本判据**，⛔ 不装 Slint 活窗口（无需 `set_platform`）；
//! * 刻意⛔ 不用 `all` 适配器（`assertion_bounds_audit.rs` 的登记守卫会要求扩 SOURCES）；
//! * 带日期记录只追加、不改写（本文件是新增判据，不改既有历史登记）。

/// 被扫文件：**编译期固定**，与 `src/elements.rs` 的 `SLINT_MANIFEST: [&str; 19]`
/// 逐字对应。UI 加一个 `.slint` 文件必须同时扩这两处（第 ④ 条判据会判红）。
const SLINT_FILES: &[(&str, &str)] = &[
    ("app.slint", include_str!("../ui/app.slint")),
    ("transport.slint", include_str!("../ui/transport.slint")),
    ("status_bar.slint", include_str!("../ui/status_bar.slint")),
    ("sidebar.slint", include_str!("../ui/sidebar.slint")),
    ("tokens.slint", include_str!("../ui/tokens.slint")),
    (
        "workspace/session_view.slint",
        include_str!("../ui/workspace/session_view.slint"),
    ),
    (
        "workspace/arrangement_view.slint",
        include_str!("../ui/workspace/arrangement_view.slint"),
    ),
    (
        "workspace/song_overview_panel.slint",
        include_str!("../ui/workspace/song_overview_panel.slint"),
    ),
    (
        "console/console_tabs.slint",
        include_str!("../ui/console/console_tabs.slint"),
    ),
    (
        "console/piano_roll.slint",
        include_str!("../ui/console/piano_roll.slint"),
    ),
    (
        "console/mixer_console.slint",
        include_str!("../ui/console/mixer_console.slint"),
    ),
    (
        "console/device_rack.slint",
        include_str!("../ui/console/device_rack.slint"),
    ),
    (
        "console/asset_pool_panel.slint",
        include_str!("../ui/console/asset_pool_panel.slint"),
    ),
    (
        "dialogs/musical_pr_drawer.slint",
        include_str!("../ui/dialogs/musical_pr_drawer.slint"),
    ),
    (
        "dialogs/undo_tree_modal.slint",
        include_str!("../ui/dialogs/undo_tree_modal.slint"),
    ),
    (
        "dialogs/export_wav_dialog.slint",
        include_str!("../ui/dialogs/export_wav_dialog.slint"),
    ),
    (
        "dialogs/humanize_dialog.slint",
        include_str!("../ui/dialogs/humanize_dialog.slint"),
    ),
    (
        "dialogs/macro_panel.slint",
        include_str!("../ui/dialogs/macro_panel.slint"),
    ),
    (
        "module_actions.slint",
        include_str!("../ui/module_actions.slint"),
    ),
];

const ELEMENTS_RS: &str = include_str!("../src/elements.rs");
const MODULE_ACTIONS: &str = include_str!("../ui/module_actions.slint");
const MIXER_CONSOLE: &str = include_str!("../ui/console/mixer_console.slint");

/// 把 `//` 行注释与 `"..."` 字符串字面量按**等长字节**替换成 ASCII 空格。
///
/// 中文注释／字符串的多字节序列被整体换成同长度的空格，仍是合法 UTF-8；
/// 未替换区保持原字节 ⇒ 输出与输入的**字节位置一一对应**，可安全复用原文索引。
fn strip_comments_and_strings(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = bytes.to_vec();
    let mut index = 0;
    while index < bytes.len() {
        if index + 1 < bytes.len() && bytes[index] == b'/' && bytes[index + 1] == b'/' {
            let mut end = index;
            while end < bytes.len() && bytes[end] != b'\n' {
                end += 1;
            }
            for slot in &mut out[index..end] {
                *slot = b' ';
            }
            index = end;
        } else if bytes[index] == b'"' {
            let mut end = index + 1;
            while end < bytes.len() {
                if bytes[end] == b'\\' {
                    end += 2;
                    continue;
                }
                if bytes[end] == b'"' {
                    end += 1;
                    break;
                }
                end += 1;
            }
            let end = end.min(bytes.len());
            for slot in &mut out[index..end] {
                *slot = b' ';
            }
            index = end;
        } else {
            index += 1;
        }
    }
    String::from_utf8(out).expect("等长字节替换不破坏 UTF-8")
}

/// 所有配对成功的 `{...}` 区间（按 `}` 出现顺序）。
fn matching_blocks(txt: &str) -> Vec<(usize, usize)> {
    let mut stack: Vec<usize> = Vec::new();
    let mut pairs = Vec::new();
    for (index, byte) in txt.as_bytes().iter().enumerate() {
        match *byte {
            b'{' => stack.push(index),
            b'}' => {
                if let Some(start) = stack.pop() {
                    pairs.push((start, index));
                }
            }
            _ => {}
        }
    }
    pairs
}

/// 取 `[start, end]` 块的**本层**文本：更深子块的内容被丢弃。
///
/// 块起始 `{` 使深度 `0 → 1`，本层属性都在深度 1；子块 `{` 使深度 ≥ 2 ⇒ 不收。
fn own_layer(txt: &str, start: usize, end: usize) -> String {
    let bytes = txt.as_bytes();
    let mut depth = 0_i32;
    let mut own = Vec::new();
    for byte in &bytes[start..=end] {
        if *byte == b'{' {
            depth += 1;
        }
        if depth <= 1 {
            own.push(*byte);
        }
        if *byte == b'}' {
            depth -= 1;
        }
    }
    String::from_utf8_lossy(&own).into_owned()
}

/// 扫描一个 `.slint`：`(可寻址节点块数, 缺显式 role 的块位置列表)`。
fn scan(name: &str, src: &str) -> (usize, Vec<String>) {
    let clean = strip_comments_and_strings(src);
    let pairs = matching_blocks(&clean);
    let mut total = 0;
    let mut gaps = Vec::new();
    for (start, end) in &pairs {
        let layer = own_layer(&clean, *start, *end);
        if layer.contains("accessible-id:") {
            total += 1;
            if !layer.contains("accessible-role:") {
                let line = src.as_bytes()[..*start]
                    .iter()
                    .filter(|byte| **byte == b'\n')
                    .count()
                    + 1;
                gaps.push(format!("{name}:{line}"));
            }
        }
    }
    (total, gaps)
}

/// 找到原文里含 `needle` 的**最内层**块，返回它的本层文本。
fn own_layer_for_needle(src: &str, needle: &str) -> Option<String> {
    let position = src.find(needle)?;
    let clean = strip_comments_and_strings(src);
    let pairs = matching_blocks(&clean);
    let mut best: Option<(usize, usize)> = None;
    for (start, end) in &pairs {
        if *start < position && position < *end {
            let smaller = match best {
                Some((bs, be)) => (*end - *start) < (be - bs),
                None => true,
            };
            if smaller {
                best = Some((*start, *end));
            }
        }
    }
    let (start, end) = best?;
    Some(own_layer(&clean, start, end))
}

/// 在 `elements.rs` 里 `needle` 之后的短窗口内找 `ElementKind::*` 并映射成 role 字面。
///
/// 会跳过「注释里先出现同名字符串」的情形：逐次往后找，直到窗口里真有 kind。
fn registry_kind_after(els: &str, needle: &str) -> Option<&'static str> {
    let mut offset = 0;
    while let Some(rel) = els.get(offset..)?.find(needle) {
        let position = offset + rel;
        let tail = &els[position..];
        let mut cut = 320_usize.min(tail.len());
        while cut > 0 && !tail.is_char_boundary(cut) {
            cut -= 1;
        }
        let window = strip_comments_and_strings(&tail[..cut]);
        if window.contains("ElementKind::TextInput") {
            return Some("text-input");
        }
        if window.contains("ElementKind::Text") {
            return Some("text");
        }
        offset = position + needle.len();
    }
    None
}

// =========================================================================
// ① 全仓常驻门禁：有 `accessible-id` 的节点必须显式声明 `accessible-role`
// =========================================================================

#[test]
fn every_addressable_node_declares_an_explicit_accessible_role() {
    let mut total = 0;
    let mut gaps: Vec<String> = Vec::new();
    for (name, src) in SLINT_FILES {
        let (count, found) = scan(name, src);
        total += count;
        gaps.extend(found);
    }
    assert!(
        total >= 200,
        "非真空：必须真的扫过大批可寻址节点（实测 {total}，下限 200）"
    );
    assert_eq!(
        total, 239,
        "全仓可寻址节点块数应与本轮基线一致；变了要复核清单与解析口径（Songmap follow-ons (a) 频段独奏金框 1，带 accessible-role: image）"
    );
    assert!(
        gaps.is_empty(),
        "[UI-A11Y-007] 每个带 `accessible-id` 的节点都必须显式声明 `accessible-role`；\
         缺 role 的块（文件:行）: {gaps:?}"
    );
}

// =========================================================================
// ② 本轮 4 个节点：声明面 role 字面 == 注册表 ElementKind 映射
// =========================================================================

#[test]
fn the_four_round6_nodes_carry_the_role_matching_their_registry_kind() {
    let cases = [
        (
            MODULE_ACTIONS,
            "accessible-id: \"module-actions-title\";",
            "\"module-actions-title\"",
            "text",
        ),
        (
            MODULE_ACTIONS,
            "accessible-id: \"module-midi-in-status\";",
            "\"module-midi-in-status\"",
            "text",
        ),
        (
            MODULE_ACTIONS,
            "accessible-id: \"module-last-status\";",
            "\"module-last-status\"",
            "text",
        ),
        (
            MIXER_CONSOLE,
            "accessible-id: \"track-\" + track_index + \"-sampler-path\";",
            "\"track-{track_index}-sampler-path\"",
            "text-input",
        ),
    ];
    for (src, slint_needle, els_needle, expected) in cases {
        let layer = own_layer_for_needle(src, slint_needle)
            .unwrap_or_else(|| panic!("找不到节点块: {slint_needle}"));
        let declaration = format!("accessible-role: {expected};");
        assert!(
            layer.contains(&declaration),
            "`{slint_needle}` 所在块必须显式声明 `{declaration}`；本层文本: {layer:?}"
        );
        let kind = registry_kind_after(ELEMENTS_RS, els_needle)
            .unwrap_or_else(|| panic!("注册表里找不到 {els_needle} 的 ElementKind"));
        assert_eq!(
            kind, expected,
            "声明面 role 与注册表 ElementKind 必须一致（{els_needle}）"
        );
    }
}

// =========================================================================
// ③ 零副作用：id / value 计数一位未动
// =========================================================================

#[test]
fn round6_adds_no_new_addressable_ids_or_value_readouts() {
    assert_eq!(
        MODULE_ACTIONS.matches("accessible-id:").count(),
        10,
        "module_actions.slint 的 10 个可寻址 ID 一位未动"
    );
    assert_eq!(
        MODULE_ACTIONS.matches("accessible-value:").count(),
        4,
        "4 条读数一位未动（⛔ 不造第二份读数）"
    );
    assert_eq!(
        MIXER_CONSOLE.matches("accessible-id:").count(),
        22,
        "mixer_console.slint 的 22 个可寻址 ID 一位未动"
    );
    assert_eq!(
        MIXER_CONSOLE.matches("accessible-value:").count(),
        7,
        "7 条读数一位未动（⛔ 不造第二份读数）"
    );
}

// =========================================================================
// ④ 判据清单 ↔ 注册表清单同步（门禁完备性）
// =========================================================================

#[test]
fn the_judged_manifest_stays_in_sync_with_the_registry_manifest() {
    let start = ELEMENTS_RS
        .find("const SLINT_MANIFEST")
        .expect("elements.rs 必须有 SLINT_MANIFEST");
    let end = ELEMENTS_RS[start..]
        .find("];")
        .expect("SLINT_MANIFEST 必须以 ]; 收尾")
        + start;
    let manifest = &ELEMENTS_RS[start..end];
    assert!(
        manifest.contains("[&str; 19]"),
        "注册表清单长度钉在 19；改了要同步本判据的 SLINT_FILES"
    );
    assert_eq!(SLINT_FILES.len(), 19, "本判据清单也要是 19");

    // 注册表侧声明的 19 个文件名（跳过整行注释）。
    let declared: Vec<&str> = manifest
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with("//") {
                return None;
            }
            let first = trimmed.find('"')?;
            let rest = &trimmed[first + 1..];
            let last = rest.find('"')?;
            let value = &rest[..last];
            if value.ends_with(".slint") {
                Some(value)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(declared.len(), 19, "注册表清单里应恰好解析出 19 个 .slint");

    for name in &declared {
        assert!(
            SLINT_FILES.iter().any(|(entry, _)| entry == name),
            "注册表清单里的 `{name}` 必须也在本判据的 SLINT_FILES 里"
        );
    }
    for (name, _) in SLINT_FILES {
        assert!(
            declared.contains(name),
            "本判据清单里的 `{name}` 必须也在注册表 SLINT_MANIFEST 里"
        );
    }
}
