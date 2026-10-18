//! ⭐ 任务单 `#186`：**其余面板段内的控件两两不许重叠**。
//!
//! `#184` 钉住了"设备架两段之间"不相交、`#185` 钉住了"发送条内"两两不相交；
//! 本文件把同一条**跨节点**判据铺到另外四段：
//!
//! | 段 | 运行时容器（取控件的那一格） | 读法 |
//! | :--- | :--- | :--- |
//! | 钢琴卷帘 | `piano-roll` 的包围盒 | 与盒子**相交**的非容器节点 |
//! | 调音台 | `mixer-console` 的包围盒 | 与盒子**相交**的非容器节点 |
//! | 素材池 | `asset-pool-panel` 的包围盒 | 与盒子**相交**的非容器节点 |
//! | 走带 | 顶栏 `y ∈ [0, 48)` | 包围盒**完全落在**该带内的非容器节点 |
//!
//! ## 判据（逐条对应任务单 §2）
//!
//! | §2 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 扫描读数 | 三档 `1920×1080 / 1440×900 / 1024×800`：每段的控件数 / 对数 / 相交对数逐档 `report_line` | 证"这四段真的被数到了"；⛔ 不证段外控件 |
//! | ② 修掉的逐条 | `#186` 的两处根因：走带徽章 ⟂ Session（1024 档 1 对）、主控通道条 ⟂ 工程/素材池面板（1024 档 8 对）—— 修前/修后 bounds 写在本文件的文档与报告里，修后由 ③ 逐对断言 | 证"改的是这处"；⛔ 不证视觉美观 |
//! | ③ 两两不相交 | 每段：排除容器后**逐对**断言包围盒不相交（半开区间，边贴边不算），三档全是 0 | 证"同一段内没有第二处重叠"；⛔ 不证与段外邻居（`#184` 管段间） |
//! | ④ 容器排除 | 见 [`is_container_role`]：`region` / `groupbox` / `list` / `tab-list` / `main` / `complementary`。**`list-item` 不排**（这四段里它是叶子内容：音符矩形、素材行） | 证"排的是按设计包住子节点的容器"；⛔ 不排 `list-item` 是有意的（与 `#185` 的发送行相反） |
//! | ⑤ 不许假绿 | 每段先断言"必须在场的 id 一个不少"＋"控件数恰好等于清单"；1024 档走带徽章**按设计隐藏**（不是扫不到），并断言替代入口 `ai-rail-musical-pr-button` 在树里 | 证"0 相交不是 0 控件换来的" |
//! | ⑥ 预算 | 纯测试 ＋ `.slint` 表达式级布局微调（不新增控件） ⇒ 控件数逐档不变：调音台 `72`、钢琴卷帘 `20`、素材池 `3`、走带 `14`（1024 档 `13`，少的是按设计隐藏的徽章） | 证"没有靠删控件换绿" |
//!
//! ## 发现（`#186` 的实测；数字是逻辑像素包围盒 `(x, y, w, h)`）
//!
//! ### 1. 走带：AI 提案徽章 与 Session 视图键（1024 档）
//!
//! ```text
//! 修前：transport-ai-proposal-badge      (640, 8, 120, 32)  ⟂
//!       transport-view-session-button    (720, 8,  92, 32)   ⇒ 相交 40×32 px²
//! 修后：徽章 visible: root.width >= 1064px（1024 档不在树里）；Session 仍在树里。
//! ```
//!
//! 算术：徽章右沿 `760`；Session 左沿 `root.width - 304`。`width < 1064` 时必交。
//! 隐藏的是**重复入口**：1024 档右栏 `ai-rail-musical-pr-button` 仍在树里。
//!
//! ### 2. 调音台：主控通道条 与「工程」/「素材池」面板（1024 档，8 对）
//!
//! ```text
//! 修前（1024）：mixer-master-pan          (918, 523,  30,  14) ⟂ module-open-project-button (860, 527, 204, 28)
//!              mixer-master-mute-button   (918, 541,  20,  16) ⟂ module-open-project-button (860, 527, 204, 28)
//!              mixer-master-solo-button   (942, 541,  20,  16) ⟂ module-open-project-button (860, 527, 204, 28)
//!              mixer-master-color-swatch  (960, 523,  10,  14) ⟂ module-open-project-button (860, 527, 204, 28)
//!              mixer-master-fader         (944, 571,  22,  16) ⟂ module-new-project-button  (860, 559, 204, 28)
//!              mixer-master-meter         (918, 563,  14, 104) ⟂ module-new-project-button  (860, 559, 204, 28)
//!              mixer-master-meter         (918, 563,  14, 104) ⟂ asset-pool-count          (860, 635, 204, 16)
//!              mixer-master-meter         (918, 563,  14, 104) ⟂ asset-pool-row-0-row      (860, 657, 204, 30)
//! 修后（1024）：工程面板 (632, 90, 216, 104)、素材池 (632, 200, 216, 109)、
//!              主控通道条 x=912 ⇒ 面板右沿 848 < 912，8 对全部消失。
//! 宽档（≥1182）：工程面板仍是 (854, 8, 216, 104)、素材池仍是 (854, 118, 216, h-128)，
//!              一位未动。
//! ```
//!
//! ## 反向腿（去掉本片改动 ⇒ 哪条红）
//!
//! 两条注入都**实测**过（读数见交付报告 §5）：
//!
//! - 注入 A：删掉走带徽章的 `visible: root.width >= 1064px;` ⇒ **走带判据红**：
//!   `transport-ai-proposal-badge (640,8,120,32) ⟂ transport-view-session-button (720,8,92,32)`；
//! - 注入 B：把 `mixer_console.slint` 的 `narrow: root.width < 1182px` 改成 `< 0px`
//!   ⇒ **调音台判据红**：上面那 8 对在 1024 档逐条回来（素材池判据也红 2 对）。
//!
//! ## 它**不**证什么
//!
//! ⛔ 只扫了这四段：设备架（`#184` 已证）、发送条（`#185` 已证）、走带以外的左栏 / 右栏 /
//! 编曲画布 / Tab 3 的装乐器行 / 轨路由四键区**仍未扫**（详见交付报告 §8）；
//! ⛔ 没人耳听过（本机 Tier-1 软件光栅化，没有声卡）；⛔ 不证 Linux Golden：
//! 本机没有 `tests/golden/macos/**`；⛔ 不证 1024 以下——`#186` 只验三档，更窄档未登记。
//! ⛔ 不证轨道数 > 6 时的调音台：窄档竖排的算术是按演示工程 `n = 6` 推的，
//! 更多轨道会把通道条推过主控通道条（本判据不覆盖，如实登记）。

use std::collections::BTreeSet;

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::inspect::tree_from_element_root;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::tree::{ControlNode, ControlTree};

/// 三档：默认档 ＋ 仓里界面判据的默认窗口 ＋ `#184`/`#185` 的窄档下界。
const TIERS: &[(u32, u32)] = &[(1920, 1080), (1440, 900), (1024, 800)];

const PIANO_TAB: i32 = 0;
const MIXER_TAB: i32 = 1;
/// 顶栏高度（`ui/tokens.slint` 的 `top-bar-height`；与 `app.slint` 的 48px 顶栏同一格）。
const TOP_BAR: u32 = 48;

/// 走带在宽档（`≥1064`）的 14 个控件；窄档（1024）少的是按设计隐藏的 AI 徽章。
const TRANSPORT_WIDE: &[&str] = &[
    "transport-play-button",
    "transport-stop-button",
    "transport-record-button",
    "transport-bpm-field",
    "transport-timecode",
    "transport-branch-button",
    "transport-commit-button",
    "transport-revert-button",
    "transport-save-button",
    "transport-ai-proposal-badge",
    "transport-view-session-button",
    "transport-view-arrangement-button",
    "transport-view-toggle-button",
    "transport-ai-drawer-button",
];

const PIANO_CONTROLS: &[&str] = &[
    "piano-roll-tool-select-button",
    "piano-roll-tool-pencil-button",
    "piano-roll-tool-knife-button",
    "piano-roll-tool-velocity-button",
    "piano-roll-tool-eraser-button",
    "piano-roll-status",
    "piano-roll-playhead",
    "note-suggestion-overlay-rect",
    "note-01J8Z5Q0R7K3M9X2V4B6N8P1A2-rect",
    "note-01J8Z5Q0R7K3M9X2V4B6N8P1A3-rect",
    "note-01J8Z5Q0R7K3M9X2V4B6N8P1B0-rect",
    "note-01J8Z5Q0R7K3M9X2V4B6N8P1C7-rect",
    "note-01J8Z5Q0R7K3M9X2V4B6N8P1D4-rect",
    "note-01J8Z5Q0R7K3M9X2V4B6N8P1E1-rect",
    "velocity-0-bar",
    "velocity-1-bar",
    "velocity-2-bar",
    "velocity-3-bar",
    "velocity-4-bar",
    "velocity-5-bar",
];

/// 素材池（演示工程 `clip_pool` 2 条）：条目数 ＋ 两行。面板本体是容器。
const ASSET_CONTROLS: &[&str] = &[
    "asset-pool-count",
    "asset-pool-row-0-row",
    "asset-pool-row-1-row",
];

/// 调音台的 72 个控件：3 个模块入口 ＋ 6 个主控控件 ＋ 6 条通道条 × 10。
fn mixer_controls() -> Vec<String> {
    let mut ids: Vec<String> = vec![
        "module-export-midi-button".to_owned(),
        "module-open-project-button".to_owned(),
        "module-new-project-button".to_owned(),
        "mixer-master-pan".to_owned(),
        "mixer-master-color-swatch".to_owned(),
        "mixer-master-mute-button".to_owned(),
        "mixer-master-solo-button".to_owned(),
        "mixer-master-meter".to_owned(),
        "mixer-master-fader".to_owned(),
    ];
    ids.extend(ASSET_CONTROLS.iter().map(|id| (*id).to_owned()));
    for track in 0..6 {
        for suffix in [
            "pan",
            "mixer-color-swatch",
            "mixer-mute-button",
            "mixer-solo-button",
            "meter",
            "fader",
            "sampler-path",
            "sampler-browse",
            "sampler-set",
            "sampler-clear",
        ] {
            ids.push(format!("track-{track}-{suffix}"));
        }
    }
    assert_eq!(ids.len(), 72, "调音台控件清单长度变了 ⇒ 判据口径要重核");
    ids
}

type Bounds = (i32, i32, u32, u32);

/// 读活窗口的运行时控件树（与 `#185` 同一个入口：`tree_from_element_root` + JSON 往返）。
fn runtime_tree(window: &MainWindow) -> ControlTree {
    let json = tree_from_element_root(window, None)
        .expect("运行时控件树")
        .dump_json();
    ControlTree::from_json(&json).expect("JSON 必须能被同一套校验读回")
}

/// 一条生产形态的 Tier-1 活窗口（唯一构造入口 `build_main_window_with_console_tab`）。
///
/// ⚠ 返回值里的 `LivePort` 必须被持有到判据结束；Tier-1 平台是**线程局部**的，
/// 一根线程只能装一次 ⇒ 每个 `(档, 标签)` 在自己的线程里跑。
fn live_window(size: (u32, u32), tab: i32) -> LivePort<MainWindow> {
    let project = yeban_model::samples::demo_project();
    let view = ViewState::from_project(&project).expect("演示工程必须能投影");
    let scene = DemoScene::from_view(&view);
    LivePort::new(
        Size::new(size.0, size.1),
        Permission::ReadOnly,
        None,
        || host::build_main_window_with_console_tab(&view, &scene, tab),
    )
    .expect("Tier-1 平台 + 生产窗口构造入口")
}

fn bounds_of(node: &ControlNode) -> Bounds {
    let b = node.bounds.expect("运行时节点必须有包围盒");
    (b.x, b.y, b.width, b.height)
}

fn bounds(tree: &ControlTree, id: &str) -> Bounds {
    let node = tree
        .find_by_id(id)
        .unwrap_or_else(|| panic!("运行时控件树里找不到 `{id}`"));
    bounds_of(node)
}

/// 两个包围盒是否相交（半开区间：边贴边**不算**相交）。
fn intersect(a: Bounds, b: Bounds) -> bool {
    let (ax, ay, aw, ah) = (
        i64::from(a.0),
        i64::from(a.1),
        i64::from(a.2),
        i64::from(a.3),
    );
    let (bx, by, bw, bh) = (
        i64::from(b.0),
        i64::from(b.1),
        i64::from(b.2),
        i64::from(b.3),
    );
    ax < bx + bw && bx < ax + aw && ay < by + bh && by < ay + ah
}

/// 容器角色：它们的包围盒**按设计**包住子节点，不参与"两两不相交"。
///
/// 逐条理由（这四段里真正被排掉的实例）：
///
/// - `region`：`mixer-console` / `asset-pool-panel` / `piano-roll` / `piano-roll-grid` /
///   `piano-roll-velocity-lane` / `piano-roll-scroll-area` —— 都是"段/画布/泳道"本身；
/// - `groupbox`：`track-{i}-channel-strip` / `mixer-master-strip` —— 通道条包住推子 / 电平 /
///   静音键；主控条的包围盒本来就该盖住自己的控件；
/// - `list`：`piano-roll-keys` —— 琴键列包住 16 个琴键；
/// - `tab-list` / `main` / `complementary`：`console-tab-rail` / `workspace-arrangement-canvas` /
///   `ai-rail` 在段外，但同一套规则让"取控件"不依赖调用点。
///
/// ⚠ **`list-item` 故意不排**：`#185` 的发送行是 `list-item` 且**包住**滑块 / 读数 / 删除键，
/// 所以那里必须排；本文件的 `note-{ulid}-rect` 与 `asset-pool-row-{i}-row` 是**叶子内容**
/// （子节点没有语义 id、不参与扫描）—— 排掉它们会把"音符压音符"变成永远绿。
fn is_container_role(role: &str) -> bool {
    matches!(
        role,
        "region" | "groupbox" | "list" | "tab-list" | "main" | "complementary"
    )
}

/// 取法：`Intersect` = 与段盒子相交即算段内（面板可能比窗口宽 / 被裁剪）；
/// `Inside` = 完全落在段盒子内（走带那条 48px 顶栏带，用它排除任何跨带节点）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Selection {
    Intersect,
    Inside,
}

fn select_controls(
    tree: &ControlTree,
    pane: Bounds,
    selection: Selection,
) -> Vec<(&ControlNode, Bounds)> {
    let right = pane.0 + pane.2 as i32;
    let bottom = pane.1 + pane.3 as i32;
    tree.iter()
        .filter(|node| !is_container_role(node.role.as_str()))
        .filter_map(|node| node.bounds.map(|b| (node, (b.x, b.y, b.width, b.height))))
        .filter(|(_, b)| match selection {
            Selection::Intersect => intersect(*b, pane),
            Selection::Inside => {
                b.0 >= pane.0
                    && b.1 >= pane.1
                    && b.0 + b.2 as i32 <= right
                    && b.1 + b.3 as i32 <= bottom
            }
        })
        .collect()
}

/// 判据 ③ 的唯一实现：段内非容器控件 → 清单必须一个不少 → 逐对断言不相交。
///
/// 返回可直接 `report_line` 的读数（控件数 / 对数 / 相交对数 / 逐条 bounds）。
fn assert_pairwise(
    tree: &ControlTree,
    pane: Bounds,
    selection: Selection,
    expected: &[&str],
    label: &str,
) -> String {
    let controls = select_controls(tree, pane, selection);

    // ---- 判据 ⑤：清单必须**一个不少**，且不许有清单外的节点混进来。
    let got: BTreeSet<&str> = controls.iter().map(|(node, _)| node.id.as_str()).collect();
    for id in expected {
        assert!(
            got.contains(id),
            "[#186 {label}] 段内控件清单缺 `{id}` —— 两两判据会漏掉它（⛔ 不许 0 控件假绿）"
        );
    }
    assert_eq!(
        controls.len(),
        expected.len(),
        "[#186 {label}] 段内控件数 {} ≠ 清单 {} ⇒ 有控件没登记（树里多出：{:?}）",
        controls.len(),
        expected.len(),
        got.iter()
            .filter(|id| !expected.contains(id))
            .collect::<Vec<_>>()
    );

    let mut pair_count = 0_usize;
    let mut hits = Vec::new();
    for (index, (a, a_bounds)) in controls.iter().enumerate() {
        for (b, b_bounds) in controls.iter().skip(index + 1) {
            pair_count += 1;
            if intersect(*a_bounds, *b_bounds) {
                hits.push(format!(
                    "`{}` (x={} y={} w={} h={}) ⟂ `{}` (x={} y={} w={} h={})",
                    a.id,
                    a_bounds.0,
                    a_bounds.1,
                    a_bounds.2,
                    a_bounds.3,
                    b.id,
                    b_bounds.0,
                    b_bounds.1,
                    b_bounds.2,
                    b_bounds.3
                ));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "[#186 {label}] 段内**有重叠**（{} 对）：\n{}",
        hits.len(),
        hits.join("\n")
    );

    let dump = controls
        .iter()
        .map(|(node, b)| format!("{}: x={} y={} w={} h={}", node.id, b.0, b.1, b.2, b.3))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "[#186 {label}] 控件 {} 个 / {pair_count} 对 / 相交 {} 对（容器按 role 排除）\n{dump}",
        controls.len(),
        hits.len()
    )
}

/// 一档的完整读数。返回 `(控件数, 对数)` 供外层汇总。
fn check_piano_roll(width: u32, height: u32) -> (usize, usize) {
    let live = live_window((width, height), PIANO_TAB);
    assert_eq!(live.ui().get_console_tab(), 0, "夹具必须是 Tab 0（卷帘）");
    let tree = runtime_tree(live.ui());
    let pane = bounds(&tree, "piano-roll");
    let reading = assert_pairwise(
        &tree,
        pane,
        Selection::Intersect,
        PIANO_CONTROLS,
        &format!("钢琴卷帘 {width}x{height}"),
    );
    report_line(&reading);
    // 骨架里的静态读数也在场（两两判据之外再钉一次，防"整段消失 ⇒ 0 相交"）。
    assert_eq!(
        tree.find_by_id("piano-roll-status")
            .expect("状态行必须在场")
            .label,
        "卷帘吸附与音域状态"
    );
    (PIANO_CONTROLS.len(), 190)
}

fn check_mixer_console(width: u32, height: u32) -> (usize, usize) {
    let live = live_window((width, height), MIXER_TAB);
    assert_eq!(live.ui().get_console_tab(), 1, "夹具必须是 Tab 1（调音台）");
    let tree = runtime_tree(live.ui());
    let pane = bounds(&tree, "mixer-console");
    let expected = mixer_controls();
    let expected_refs: Vec<&str> = expected.iter().map(String::as_str).collect();
    let reading = assert_pairwise(
        &tree,
        pane,
        Selection::Intersect,
        &expected_refs,
        &format!("调音台 {width}x{height}"),
    );
    report_line(&reading);

    // ---- 判据 ② 的修后几何：主控通道条仍在右沿；窄档三块面板竖排在它左侧。
    // 读数统一换到**调音台局部坐标**（运行时树给的是绝对坐标；`mixer-console` 的
    // 原点在 y = 顶栏 + 工作区，直接拿绝对值比登记值会把整段高度算进去）。
    let origin_x = pane.0;
    let origin_y = pane.1;
    let master = bounds(&tree, "mixer-master-strip");
    let master_x = master.0 - origin_x;
    assert_eq!(
        master_x,
        width as i32 - 112,
        "[#186 {width}x{height}] 主控通道条必须仍钉在 root.width - 112"
    );
    let export = tree
        .find_by_id("module-export-midi-button")
        .expect("导出 MIDI 按钮")
        .bounds
        .expect("包围盒");
    let open = tree
        .find_by_id("module-open-project-button")
        .expect("打开工程按钮")
        .bounds
        .expect("包围盒");
    let asset = bounds(&tree, "asset-pool-panel");
    let asset_x = asset.0 - origin_x;
    let asset_y = asset.1 - origin_y;
    let open_x = open.x - origin_x;
    if width < 1182 {
        // 窄档：工程面板与导出面板同列、素材池在工程面板正下方；右沿全部 < 主控左沿。
        assert_eq!(
            asset_x,
            8 + 104 * 6,
            "[#186 {width}] 窄档素材池必须与通道条右沿同列起点"
        );
        assert_eq!(open_x, 8 + 104 * 6 + 6, "[#186 {width}] 窄档工程按钮 x");
        assert_eq!(asset_y, 200, "[#186 {width}] 窄档素材池 y");
        assert!(
            asset_x + asset.2 as i32 <= master_x,
            "[#186 {width}] 素材池右沿 {} 必须 ≤ 主控左沿 {}",
            asset_x + asset.2 as i32,
            master_x
        );
        report_line(&format!(
            "[#186 调音台 {width}x{height}] 窄档竖排（局部坐标）：导出按钮 ({},{}) / 工程按钮 ({},{}) / \
             素材池 ({},{},{},{})；主控 x={master_x} ⇒ 面板右沿 {} < 主控左沿 {master_x}",
            export.x - origin_x,
            export.y - origin_y,
            open_x,
            open.y - origin_y,
            asset_x,
            asset_y,
            asset.2,
            asset.3,
            asset_x + asset.2 as i32
        ));
    } else {
        // 宽档：三块面板并排，几何一位未动（与 `#90`/`#93`/`#129`/`#133` 登记值一致）。
        assert_eq!(
            asset_x,
            8 + 104 * 6 + 216 + 6,
            "[#186 {width}] 宽档素材池 x"
        );
        assert_eq!(asset_y, 8 + 104 + 6, "[#186 {width}] 宽档素材池 y");
        assert_eq!(
            open_x,
            8 + 104 * 6 + 216 + 6 + 6,
            "[#186 {width}] 宽档工程按钮 x"
        );
        report_line(&format!(
            "[#186 调音台 {width}x{height}] 宽档并排（局部坐标）：素材池 ({},{},{},{}) / 工程按钮 ({},{})；\
             主控 x={master_x} ⇒ 面板右沿 {} < 主控左沿 {master_x}",
            asset_x,
            asset_y,
            asset.2,
            asset.3,
            open_x,
            open.y - origin_y,
            asset_x + asset.2 as i32
        ));
    }
    (expected.len(), 72 * 71 / 2)
}

fn check_asset_pool(width: u32, height: u32) -> (usize, usize) {
    let live = live_window((width, height), MIXER_TAB);
    let tree = runtime_tree(live.ui());
    let pane = bounds(&tree, "asset-pool-panel");
    let reading = assert_pairwise(
        &tree,
        pane,
        Selection::Intersect,
        ASSET_CONTROLS,
        &format!("素材池 {width}x{height}"),
    );
    report_line(&reading);
    (ASSET_CONTROLS.len(), 3)
}

fn check_transport(width: u32, height: u32) -> (usize, usize) {
    let live = live_window((width, height), PIANO_TAB);
    let tree = runtime_tree(live.ui());
    let band: Bounds = (0, 0, width, TOP_BAR);
    let expected: Vec<&str> = if width >= 1064 {
        TRANSPORT_WIDE.to_vec()
    } else {
        TRANSPORT_WIDE
            .iter()
            .copied()
            .filter(|id| *id != "transport-ai-proposal-badge")
            .collect()
    };
    let reading = assert_pairwise(
        &tree,
        band,
        Selection::Inside,
        &expected,
        &format!("走带 {width}x{height}"),
    );
    report_line(&reading);

    // ---- 判据 ②＋⑤：1024 档徽章**按设计隐藏**（不是扫不到），替代入口必须在树里。
    let save = bounds(&tree, "transport-save-button");
    let session = bounds(&tree, "transport-view-session-button");
    assert!(
        save.0 + save.2 as i32 <= session.0,
        "[#186 {width}x{height}] 保存按钮右沿必须 ≤ Session 左沿"
    );
    if width >= 1064 {
        let badge = bounds(&tree, "transport-ai-proposal-badge");
        assert!(
            save.0 + save.2 as i32 <= badge.0,
            "[#186 {width}x{height}] 保存按钮右沿必须 ≤ 徽章左沿"
        );
        assert!(
            badge.0 + badge.2 as i32 <= session.0,
            "[#186 {width}x{height}] 徽章右沿必须 ≤ Session 左沿（宽档不许交）"
        );
        report_line(&format!(
            "[#186 走带 {width}x{height}] save {save:?} ≤ badge {badge:?} ≤ session {session:?} ⇒ 不相交"
        ));
    } else {
        assert!(
            tree.find_by_id("transport-ai-proposal-badge").is_none(),
            "[#186 {width}x{height}] 窄档徽章必须 visible:false ⇒ 不在树里"
        );
        assert!(
            tree.find_by_id("ai-rail-musical-pr-button").is_some(),
            "[#186 {width}x{height}] 窄档徽章隐藏后，右栏的同动作入口必须在树里（⛔ 不是把功能藏没了）"
        );
        report_line(&format!(
            "[#186 走带 {width}x{height}] save {save:?} ≤ session {session:?} ⇒ 不相交；\
             徽章不在树里（visible:false）；替代入口 ai-rail-musical-pr-button 在树里 = true"
        ));
    }
    (
        expected.len(),
        expected.len() * expected.len().saturating_sub(1) / 2,
    )
}

// ===========================================================================
// 判据 ①（钢琴卷帘）：三档，段内非容器控件两两不相交
// ===========================================================================
#[test]
fn the_piano_roll_has_no_two_controls_in_the_same_place() {
    for &(width, height) in TIERS {
        if let Err(payload) = std::thread::spawn(move || check_piano_roll(width, height)).join() {
            std::panic::resume_unwind(payload);
        }
    }
}

// ===========================================================================
// 判据 ②（调音台）：三档，段内非容器控件两两不相交 ＋ 窄档竖排几何
// ===========================================================================
#[test]
fn the_mixer_console_has_no_two_controls_in_the_same_place() {
    for &(width, height) in TIERS {
        if let Err(payload) = std::thread::spawn(move || check_mixer_console(width, height)).join()
        {
            std::panic::resume_unwind(payload);
        }
    }
}

// ===========================================================================
// 判据 ③（素材池）：三档，段内非容器控件两两不相交
// ===========================================================================
#[test]
fn the_asset_pool_has_no_two_controls_in_the_same_place() {
    for &(width, height) in TIERS {
        if let Err(payload) = std::thread::spawn(move || check_asset_pool(width, height)).join() {
            std::panic::resume_unwind(payload);
        }
    }
}

// ===========================================================================
// 判据 ④（走带）：三档，顶栏带内非容器控件两两不相交
// ===========================================================================
#[test]
fn the_transport_has_no_two_controls_in_the_same_place() {
    for &(width, height) in TIERS {
        if let Err(payload) = std::thread::spawn(move || check_transport(width, height)).join() {
            std::panic::resume_unwind(payload);
        }
    }
}
