//! 第十一轮 `[PATCH UI-FEEDBACK-05]`：**「可交互即可感知」全仓收官**——
//! 把第九/十轮开创的手型光标推广到剩余高频可点控件，并给主窗口分隔条补尺寸调整光标。
//!
//! 本轮用户原话：「git pull 后，继续仔细检查各种视图是否用户友好，操作是否用户友好，
//! 修复完善出具补丁」。`git pull` 后基线快进到 `6ce6aae2`（第九轮已由 `deffe88b` 合入、
//! `39c9d2e0` 修了 round9 判据的 CRLF 边界 panic、`5cf3c0c0` 加了无窗口导出）。
//! 逐文件核对：第九轮 WIP 与合入版逐字节一致 ⇒ 备份后对齐；第十轮补丁原样重建，
//! 上游的 `is_char_boundary` CRLF 修复完整保留。
//!
//! 双轴盘点（分视图族 × 分交互维度）+ 亲自复核后，全仓 108 个 `TouchArea` 中仍有
//! **43 个「有 `clicked` 却既无光标也无反馈」**。按 ROI 收敛出两组：
//!
//! | 组 | 区域 | 文件 | 缺口 | 改法 |
//! | :-- | :--- | :--- | :--- | :--- |
//! | ① | 高频可点控件（状态 tab / 导航 / 列表项 / 功能键） | 8 个 `.slint` ×37 | transport 两个视图 tab、Console 主 tab、sidebar 分类与素材项、素材池行、Device Rack 20 键、Mixer M/S 与浏览键、编曲轨道 M/S/R、声学诊断键：鼠标扫过仍是默认箭头 | 各 `TouchArea` 加 `mouse-cursor: MouseCursor.pointer;`（全仓 13→50） |
//! | ② | 主窗口分隔条 | `app.slint` ×1 | `splitter-touch` 是 `accessible-role: slider` 的拖拽调整控件（220–600px），4px 细线、零视觉提示，用户完全看不出能拖 | 加 `mouse-cursor: MouseCursor.row-resize;`（`i-slint-common-1.18.1::BuiltInMouseCursor::RowResize`，`cargo check` 编译探针已验） |
//!
//! ## 默认帧零像素漂移怎么保证
//!
//! * 光标不是渲染帧的一部分（第九轮已确立，`round9_action_feedback.rs` ② 同款）；
//!   ⛔ 本轮**只插入 `mouse-cursor` 属性行**，不改 `background` / `color` / `width` /
//!   `height` / `visible` / 任何 `accessible-*` / 任何 `clicked` 与 `pointer-event` 回调。
//! * 全仓 `has-hover` 仍 32、`grab` 仍 7 ⇒ 前两轮的反馈与拖拽成果一位未动。
//! * `row-resize` 落在 `splitter-touch` 内；它本来就是「不画任何像素」的输入面
//!   （注释明写「命中带上下各扩 2px：4px 的细线不好抓（⛔ 不改可视几何）」）。
//! * 本机 Linux golden 逐字节断言若红，按 `AGENTS.md` 用干净基线 A/B 取证后记
//!   `pending`，⛔ 不重录、⛔ 不放宽判据。
//!
//! ## 诚实登记（明确**不做**的边界，⑦ 钉住不可静默改动）
//!
//! * ⛔ **画布手势面**：`piano_roll.slint` 卷帘,Mixer 点空白选轨、编曲剪辑点选 / 拖动、
//!   标尺 seek、卷帘横滚——同一面上混着点选 / 画音符 / 拖滚动 / 双击复位，给单一
//!   光标会误导。`round10_action_feedback.rs` ② 还把整个 `piano_roll.slint` 钉成零光标，
//!   故**工具栏数字键本轮也刻意不加**（为 1 处光标改上轮判据语义不划算），留独立切片。
//! * ⛔ **禁用态 `not-allowed` 光标**：Device Rack 有 11 个 `enabled: root.*-available`
//!   按钮。查 `i-slint-core-1.18.1/items/input_items.rs:81`，
//!   `TouchArea::input_event_filter_before_children` 在 `!self.enabled()` 时直接
//!   `ForwardAndIgnore`，**根本不设置 `*cursor`** ⇒ 可用时显示手型、禁用时保持默认箭头，
//!   ⛔ 不误导、⛔ 也不假装有「禁止」提示。全仓 `MouseCursor.not-allowed` 保持 0。
//! * ⛔ **覆盖层「×」关闭键的 hover/pressed 反馈**：`macro_panel` 的 × 父格没有
//!   `background`，补反馈要新增渲染属性；`export_wav_dialog` 关闭格含
//!   `export-wav-close-focus := FocusScope` 与 Tab/Return 冒泡，撞
//!   `export_dialog_button_focus.rs` 轮转表。第十轮只给了 pointer，本轮不动。
//! * ⛔ **M/S 激活态字色**（第十轮 A/B 后撤回）：`arrangement_view.slint` 与干净基线
//!   逐字节一致，⑤ 与 `round10` ⑤ 双重钉住，⛔ 不借本轮把反色三元塞回去。
//! * ⛔ **`transport-commit-button`**：`src/elements.rs` 的
//!   `KNOWN_ACTIONLESS_CONTROLS` 刻意登记的占位（能力不存在、`ADR-0052` 未决），
//!   且 `elements.rs:3792` 与 `round8` ⑤ 把「今天不得有 `clicked`」钉成事实 ——
//!   本轮巡检时亲自复核过，**不是漏接线**，不动。
//! * ⛔ 本轮**零新增** `accessible-id` / `accessible-value` / 可寻址节点，
//!   全仓 239 / 55 由 ⑥ 逐文件对账。零新增 `experimental-*` / `mcp-*` feature、
//!   零 `Cargo.toml` 改动、零 golden PNG 改动。
//! * ⛔ 本文件刻意不用 `.all(` 适配器（`assertion_bounds_audit.rs` 的登记守卫会要求
//!   扩 `SOURCES`）；全是纯源码文本判据，不装 Slint 平台、不碰活窗口。
//! * 同步扩了 `round9_action_feedback.rs` ②（13→50）与 `round10_action_feedback.rs`
//!   ①（13→50）、②（20→58）：这是**新增合法光标**带来的预期增长，⛔ 不是放宽 ——
//!   仍是精确计数，且由本文件 ①③ 三方互相印证。带日期记录只追加、不改写。

const APP: &str = include_str!("../ui/app.slint");
const TRANSPORT: &str = include_str!("../ui/transport.slint");
const MODULE_ACTIONS: &str = include_str!("../ui/module_actions.slint");
const SIDEBAR: &str = include_str!("../ui/sidebar.slint");
const SESSION_VIEW: &str = include_str!("../ui/workspace/session_view.slint");
const ARRANGEMENT: &str = include_str!("../ui/workspace/arrangement_view.slint");
const SONG_OVERVIEW: &str = include_str!("../ui/workspace/song_overview_panel.slint");
const CONSOLE_TABS: &str = include_str!("../ui/console/console_tabs.slint");
const PIANO_ROLL: &str = include_str!("../ui/console/piano_roll.slint");
const MIXER_CONSOLE: &str = include_str!("../ui/console/mixer_console.slint");
const DEVICE_RACK: &str = include_str!("../ui/console/device_rack.slint");
const MUSICAL_PR_DRAWER: &str = include_str!("../ui/dialogs/musical_pr_drawer.slint");
const MACRO_PANEL: &str = include_str!("../ui/dialogs/macro_panel.slint");
const EXPORT_WAV_DIALOG: &str = include_str!("../ui/dialogs/export_wav_dialog.slint");
const HUMANIZE_DIALOG: &str = include_str!("../ui/dialogs/humanize_dialog.slint");
const UNDO_TREE_MODAL: &str = include_str!("../ui/dialogs/undo_tree_modal.slint");

/// 全仓 `.slint` 清单：与 `round8` / `round9` / `round10`、
/// `round6_addressable_roles.rs::SLINT_FILES`、`src/elements.rs::SLINT_MANIFEST`
/// 同一份 19 文件口径（编译期固定），⛔ 不用 `read_dir` 空转成绿。
const SLINT_FILES: &[(&str, &str)] = &[
    ("app.slint", APP),
    ("transport.slint", TRANSPORT),
    ("module_actions.slint", MODULE_ACTIONS),
    ("sidebar.slint", SIDEBAR),
    ("status_bar.slint", include_str!("../ui/status_bar.slint")),
    ("tokens.slint", include_str!("../ui/tokens.slint")),
    ("workspace/session_view.slint", SESSION_VIEW),
    ("workspace/arrangement_view.slint", ARRANGEMENT),
    ("workspace/song_overview_panel.slint", SONG_OVERVIEW),
    ("console/console_tabs.slint", CONSOLE_TABS),
    ("console/piano_roll.slint", PIANO_ROLL),
    ("console/mixer_console.slint", MIXER_CONSOLE),
    ("console/device_rack.slint", DEVICE_RACK),
    (
        "console/asset_pool_panel.slint",
        include_str!("../ui/console/asset_pool_panel.slint"),
    ),
    ("dialogs/musical_pr_drawer.slint", MUSICAL_PR_DRAWER),
    ("dialogs/macro_panel.slint", MACRO_PANEL),
    ("dialogs/export_wav_dialog.slint", EXPORT_WAV_DIALOG),
    ("dialogs/humanize_dialog.slint", HUMANIZE_DIALOG),
    ("dialogs/undo_tree_modal.slint", UNDO_TREE_MODAL),
];

/// 从标记行开始取 `count` 行。⛔ 刻意用 `lines()` 逐行拼接，不用字节算术切片：
/// 上游 `39c9d2e0` 刚修过 `round9` 判据在 Windows CRLF 检出下
/// `cancel + 6000` 落在多字节字符中间的 panic，本轮不复刻。
fn lines_after(src: &str, marker: &str, count: usize) -> String {
    let mut out = String::new();
    let mut taking = false;
    let mut taken = 0usize;
    for line in src.lines() {
        if !taking && line.contains(marker) {
            taking = true;
        }
        if taking {
            out.push_str(line);
            out.push('\n');
            taken += 1;
            if taken >= count {
                break;
            }
        }
    }
    assert!(
        !out.is_empty(),
        "判据标记 `{marker}` 未在源码中找到 ⇒ ⛔ 不许空窗口假绿"
    );
    out
}

/// ① 全仓手型光标收官：逐文件精确分布 + 全仓棘轮 = 50。
/// 删任一处 ⇒ 少一次，红；别处又加 ⇒ 多一次，也红。
#[test]
fn all_clickable_controls_now_show_a_pointer_cursor() {
    let cases: &[(&str, &str, usize)] = &[
        ("app.slint", APP, 1), // 声学诊断键
        (
            "console/asset_pool_panel.slint",
            include_str!("../ui/console/asset_pool_panel.slint"),
            1,
        ),
        ("console/console_tabs.slint", CONSOLE_TABS, 1), // Console 主 tab
        ("console/device_rack.slint", DEVICE_RACK, 20),  // 20 个主操作键
        ("console/mixer_console.slint", MIXER_CONSOLE, 7), // M/S ×4 + 浏览键 ×3
        ("dialogs/export_wav_dialog.slint", EXPORT_WAV_DIALOG, 3), // 第十轮
        ("dialogs/humanize_dialog.slint", HUMANIZE_DIALOG, 3), // 人性化弹窗：关闭 + 两枚应用
        ("dialogs/macro_panel.slint", MACRO_PANEL, 1),   // 第十轮
        ("dialogs/musical_pr_drawer.slint", MUSICAL_PR_DRAWER, 4), // 第九+十轮
        ("dialogs/undo_tree_modal.slint", UNDO_TREE_MODAL, 4), // 第十轮
        ("sidebar.slint", SIDEBAR, 2),                   // 分类 tab + 素材项
        ("transport.slint", TRANSPORT, 2),               // 两个视图状态 tab
        ("workspace/arrangement_view.slint", ARRANGEMENT, 3), // 轨道 M/S/R
        ("workspace/song_overview_panel.slint", SONG_OVERVIEW, 1), // 第九轮段落标签
    ];
    for (file, src, want) in cases {
        assert_eq!(
            src.matches("mouse-cursor: MouseCursor.pointer;").count(),
            *want,
            "{file}: 手型光标数量与第十一轮基线不符 ⇒ 删了或又新增未登记"
        );
    }
    let mut pointer_total = 0usize;
    for (_, src) in SLINT_FILES {
        pointer_total += src.matches("mouse-cursor: MouseCursor.pointer;").count();
    }
    assert_eq!(
        pointer_total, 53,
        "全仓手型光标 = 53（第九轮 2 + 第十轮 11 + 第十一轮 37 + 人性化弹窗 3）"
    );
    assert_eq!(SLINT_FILES.len(), 19, "判据清单仍须是 19 个 `.slint`");
    // 光标只补可发现性，⛔ 不替换任何按钮的行为落点（抽三处高频键逐字钉）。
    assert!(TRANSPORT.contains("clicked => { root.show-session(); }"));
    assert!(TRANSPORT.contains("clicked => { root.show-arrangement(); }"));
    assert!(MIXER_CONSOLE.contains("root.mixer-mute-toggle(track_index);"));
    assert!(ARRANGEMENT.contains("root.track-arm-toggle(track_index);"));
}

/// ② 主窗口分隔条终于看得出能拖：全仓唯一 1 处 `row-resize`，
/// 且必须落在 `splitter-touch` 内 —— 它是 `accessible-role: slider` 的垂直调整控件。
#[test]
fn the_splitter_is_discoverable_as_a_resizable_control() {
    let mut resize_total = 0usize;
    for (_, src) in SLINT_FILES {
        resize_total += src.matches("mouse-cursor: MouseCursor.row-resize;").count();
    }
    assert_eq!(
        resize_total, 1,
        "全仓恰好 1 处 `row-resize`（主窗口分隔条）"
    );

    let start = APP
        .find("splitter-touch := TouchArea {")
        .expect("`splitter-touch` 必须留在源文件里（`splitter_drag.rs` / 无障碍契约要它）");
    let tail = &APP[start..];
    let end = tail
        .find("pointer-event(event) => {")
        .expect("分隔条仍须用 `pointer-event` 处理拖拽");
    let block = &tail[..end];
    assert!(
        block.contains("mouse-cursor: MouseCursor.row-resize;"),
        "分隔条必须给垂直调整光标，否则 4px 细线用户根本看不出能拖"
    );
    assert!(
        !block.contains("clicked"),
        "分隔条是拖拽控件，⛔ 不许被改成一个「点一下就动」的按钮"
    );
    assert!(
        APP.contains("accessible-role: slider;")
            && APP.contains("accessible-id: \"splitter-workspace-console\";"),
        "分隔条的无障碍 slider 身份一位未动（⛔ 零新增可寻址节点）"
    );
    // `RowResize` 是 Slint 1.18.1 的真实变体（本轮 `cargo check` 编译探针已验）。
    assert!(
        !APP.contains("MouseCursor.not-allowed"),
        "⛔ 不假装有禁用态提示"
    );
}

/// ③ 前两轮的成果一位未动：grab 7 / has-hover 32 / cursor_total 58 逐文件对账。
#[test]
fn the_grab_and_hover_inventories_are_untouched() {
    let grab_cases: &[(&str, &str, usize)] = &[
        ("console/mixer_console.slint", MIXER_CONSOLE, 4),
        ("console/device_rack.slint", DEVICE_RACK, 2),
        ("dialogs/macro_panel.slint", MACRO_PANEL, 1),
    ];
    for (file, src, want) in grab_cases {
        assert_eq!(
            src.matches("mouse-cursor: MouseCursor.grab;").count(),
            *want,
            "{file}: 第十轮可拖光标被删或又新增未登记"
        );
    }
    let hover_cases: &[(&str, &str, usize)] = &[
        ("app.slint", APP, 2),
        ("transport.slint", TRANSPORT, 9),
        ("module_actions.slint", MODULE_ACTIONS, 5),
        ("console/console_tabs.slint", CONSOLE_TABS, 1),
        ("console/mixer_console.slint", MIXER_CONSOLE, 3),
        ("workspace/session_view.slint", SESSION_VIEW, 2),
        ("workspace/arrangement_view.slint", ARRANGEMENT, 4),
        ("workspace/song_overview_panel.slint", SONG_OVERVIEW, 6),
    ];
    for (file, src, want) in hover_cases {
        assert_eq!(
            src.matches("has-hover").count(),
            *want,
            "{file}: `has-hover` 次数与第十轮基线不符 ⇒ 本轮偷偷改了反馈"
        );
    }
    let (mut grab, mut hover, mut total) = (0usize, 0usize, 0usize);
    for (_, src) in SLINT_FILES {
        grab += src.matches("mouse-cursor: MouseCursor.grab;").count();
        hover += src.matches("has-hover").count();
        total += src.matches("mouse-cursor:").count();
    }
    assert_eq!(grab, 7, "全仓可拖光标恰好 7 处");
    assert_eq!(hover, 32, "全仓 `has-hover` 仍 32（本轮零反馈改动）");
    assert_eq!(
        total, 61,
        "全仓光标恰好 61 处 = 53 pointer + 7 grab + 1 row-resize"
    );
}

/// ④ 画布手势面刻意零光标：语义混合（点选/画音符/拖滚动/双击复位），
/// 给单一光标会把「能拖」误标成「能点」。
#[test]
fn canvas_gesture_surfaces_still_have_no_cursor() {
    assert!(
        !PIANO_ROLL.contains("mouse-cursor:"),
        "卷帘画布（含工具栏数字键）整个文件刻意零光标，`round10` ② 也这么钉"
    );
    // Mixer「点通道条空白选轨」与编曲「剪辑点选 / 拖动面」保持无光标。
    // ⛔ 全部用逐行窗口：字节算术 `+420` 在 CRLF 检出下会落在多字节字符中间（同上游
    // `39c9d2e0` 修过的 panic），且窗口必须真的取到内容（`lines_after` 内建断言）。
    let mixer_seg = lines_after(MIXER_CONSOLE, "// 点击通道条任意空白区域", 8);
    assert!(
        !mixer_seg.contains("mouse-cursor:"),
        "点空白选轨是画布手势面，刻意不加光标"
    );
    let clip_seg = lines_after(
        ARRANGEMENT,
        "// 点击面：零视觉、零算术 —— 只把**身份**原样报给宿主。",
        8,
    );
    assert!(
        !clip_seg.contains("mouse-cursor:"),
        "剪辑点选面刻意不加光标（同面还有双击）"
    );
    let drag_seg = lines_after(
        ARRANGEMENT,
        "for clip_ulid[clip_index] in root.clip-ulids : TouchArea {",
        12,
    );
    assert!(
        !drag_seg.contains("mouse-cursor:"),
        "剪辑拖动面刻意不加光标（避免把「能拖」误标成「能点」）"
    );
}

/// ⑤ 11 个禁用态按钮不假装修好：`enabled` 仍在、全仓零 `not-allowed`。
/// Slint 源码取证见文档头：disabled 时 `ForwardAndIgnore`，光标根本不生效
/// ⇒ 可用才显示手型、禁用保持默认箭头，⛔ 不误导、⛔ 不假装「禁止」提示。
#[test]
fn disabled_buttons_do_not_pretend_to_be_clickable() {
    assert_eq!(
        DEVICE_RACK.matches("enabled: root.").count(),
        11,
        "Device Rack 的 11 个条件可用按钮一位未动"
    );
    let mut not_allowed = 0usize;
    for (_, src) in SLINT_FILES {
        not_allowed += src.matches("MouseCursor.not-allowed").count();
    }
    assert_eq!(
        not_allowed, 0,
        "⛔ 本轮不引入 `not-allowed`：live 禁用态的光标行为需宿主层活体验证，不假装修好"
    );
    // 抽一个禁用键逐字钉：从「+ 音频」标签行往下取 16 行，完整覆盖该 TouchArea。
    // ⛔ 不用 `find(...)+220` 字节窗口：它在 CRLF 下可能 panic，且 `find` 只会命中
    // 第一处 `enabled`（+ MIDI 键），窗口里压根没有 audio 落点 ⇒ 会假红。
    let seg = lines_after(DEVICE_RACK, "text: \"+ 音频\";", 16);
    assert!(seg.contains("mouse-cursor: MouseCursor.pointer;"));
    assert!(seg.contains("enabled: root.routing-create-available;"));
    assert!(seg.contains("root.track-create-audio();"));
}

/// ⑥ 零新增可寻址节点 / 无障碍读数 / 唯一强调色：本轮 8 个改动文件逐文件对账
/// （＋人性化弹窗 1 案：12 ID / 5 读数）。
#[test]
fn zero_new_addressable_nodes_or_readouts() {
    let cases: &[(&str, &str, usize, usize)] = &[
        ("app.slint", APP, 8, 3),
        ("dialogs/humanize_dialog.slint", HUMANIZE_DIALOG, 12, 5),
        (
            "console/asset_pool_panel.slint",
            include_str!("../ui/console/asset_pool_panel.slint"),
            5,
            0,
        ),
        ("console/console_tabs.slint", CONSOLE_TABS, 3, 0),
        ("console/device_rack.slint", DEVICE_RACK, 63, 3),
        ("console/mixer_console.slint", MIXER_CONSOLE, 22, 7),
        ("sidebar.slint", SIDEBAR, 6, 1),
        ("transport.slint", TRANSPORT, 15, 4),
        ("workspace/arrangement_view.slint", ARRANGEMENT, 22, 5),
    ];
    for (file, src, ids, values) in cases {
        assert_eq!(
            src.matches("accessible-id:").count(),
            *ids,
            "{file}: 可寻址 ID 一位未动"
        );
        assert_eq!(
            src.matches("accessible-value:").count(),
            *values,
            "{file}: 读数一位未动（⛔ 不造第二份读数）"
        );
    }
    let (mut ids, mut values) = (0usize, 0usize);
    for (_, src) in SLINT_FILES {
        ids += src.matches("accessible-id:").count();
        values += src.matches("accessible-value:").count();
    }
    assert_eq!(
        ids, 240,
        "全仓可寻址 ID 仍 240（上游 Songmap Phase 2 之后 ＋ 卷帘右键菜单 2 ＋ 人性化弹窗 12 ＋ Songmap follow-ons (a) 频段独奏金框 1）"
    );
    assert_eq!(
        values, 55,
        "全仓无障碍读数仍 55（+ 人性化弹窗 5：时值/力度/种子/网格/强度）"
    );
}

/// ⑦ 刻意不做的边界：无动作占位、A/B 撤回的 M/S 字色、× 关闭键反馈、piano_roll。
#[test]
fn the_deliberate_boundaries_are_still_intact() {
    // `transport-commit-button` 是 `KNOWN_ACTIONLESS_CONTROLS` 登记的刻意占位。
    let start = TRANSPORT
        .find("commit_area := TouchArea {")
        .expect("`commit_area` 必须留在源文件里（`elements.rs` 双向契约要它）");
    let tail = &TRANSPORT[start..];
    let end = tail.find("}").expect("TouchArea 块必须闭合");
    assert!(
        !tail[..end].contains("clicked"),
        "`commit_area` 今天不得有 `clicked` —— 谁接上了就一并更新 `elements.rs:3792` 判据"
    );
    // 第十轮 A/B 后永久撤回的 M/S 激活态字色。
    assert_eq!(
        ARRANGEMENT.matches("? #060A14 : Tokens.ink-2;").count(),
        0,
        "⛔ M/S 字色反色三元必须不存在（A/B 实证会改默认帧，已撤回）"
    );
    assert_eq!(
        ARRANGEMENT.matches("color: Tokens.ink-2;").count(),
        8,
        "撤回后 8 处 `color: Tokens.ink-2;` 一位未动"
    );
    // 覆盖层 × 关闭键只给 pointer、没偷偷补 hover 反馈。
    assert!(MUSICAL_PR_DRAWER.contains("background: Tokens.bg-void;"));
    assert!(UNDO_TREE_MODAL.contains("background: Tokens.bg-void;"));
    assert!(EXPORT_WAV_DIALOG.contains("export-wav-close-focus := FocusScope"));
    // `piano_roll.slint` 整文件零光标（④ 与 `round10` ② 双重钉）。
    assert!(!PIANO_ROLL.contains("mouse-cursor:"));
}
