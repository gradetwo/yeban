//! **`--enable-ui-mcp-http` 那份装配自己接了定位族** —— `D-UI-31` 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 在**无头执行面**（`build_live_ui_with` ⇒ 人真的拿来作曲的那份装配）里，标尺上那一次
//! **按下 / 拖动**、屏读器的**「上一 / 下一小节」**与**「定位到第 N 小节」**、章节卡片、
//! 场景按钮，都必须把定位命令发给**本装配自己**的那个 `EngineHost`，并且用**本装配
//! 当前那一格投影**去算落点 —— 而⛔ 不是由判据替它接好线、或替它把投影喂进去之后再"能用"。
//!
//! ## 修前的形状（实测读数）
//!
//! 三条接线（`host::wire_ruler_seek` / `host::wire_section_seek` / `host::wire_scene_launch`）
//! 在修前只住在产品 GUI 里。现读（剥掉 `//` 行之后的**代码**，单位：处）：
//! `main.rs` = **各 1 处**（共 3 处）、修前的 `live_surface.rs` = **各 0 处**。
//! Slint 的 `on_*` 没注册者就是**空操作**，于是界面上：
//!
//! 1. ⛔ 点标尺 / 拖标尺 = 播放头不动（`ruler-seek` 没人听）；
//! 2. ⛔ 屏读器的增减与 `SetValue` = 不动（`ruler-step` / `ruler-seek-bar` 没人听）；
//! 3. ⛔ 章节卡片按下去 = 不动（`section-seek` 没人听）；
//! 4. ⛔ 场景按钮按下去 = 状态行连一句"没送到"都不给（`scene-launch` 没人听，
//!    而 `ui/launch_scene` 那条 MCP 工具腿**是**通的 ⇒ "`ui/launch_scene` 能做、
//!    界面上点不动"这种最难查的形状）。
//!
//! ⚠ 更深的一层是**投影的存法**：修前 `LiveAdminSurface.view` 是**裸值**，而
//! `wire_sidebar_contents` / `wire_clip_scope` 各拿一份 `Rc<new(RefCell::new(view.clone()))`
//! ⇒ 装配期一共造了**三份**互不相干的投影，其中两份永远停在装配那个工程上
//! （换工程后 `apply_project` 只换裸值）。这三条定位接线要的正是"从**当前**投影现读"，
//! 所以本票把那两半一起改：一份 `Rc<RefCell<ViewState>>`，五条接线共用（`Rc::clone`）。
//!
//! ⚠ **为什么此前没有判据抓到它**：`tests/arrangement_ruler_seek.rs` /
//! `tests/arrangement_section_seek.rs` / `tests/scene_launch_wiring.rs` 三条链**各自**新建
//! 一份 `Rc<RefCell<EngineHost>>` 与一份 `Rc<RefCell<ViewState>>` 再补接线 ⇒
//! "serve 装配接没接、接的是不是**自己**那一格投影"从外面永远读不出来。
//! 而 `tests/d_ui_11_serve_header_gestures.rs` 的例外清单把这三条记成
//! "需要活投影 `Rc<RefCell<ViewState>>`（本装配存裸值）"⇒ **清单把缺陷掩护成了结构**。
//! 本票同时把这三条从那张表里**删掉**（本文件即其"装得出来"的证据）。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（按下 ⇒ 本装配的宿主） | 前置读数 `commands_applied = 1`（装载即停住那一条 `Stop`，`D-UI-32`）；在第 3 条小节线上按一次 ⇒ 同一宿主 `+1` 条、位置离开 0、界面镜面 `playhead-ticks` 跟着回写；再用屏读器 `SetValue` 定位到"第 3 小节"⇒ **落点与指针那条完全相同**（两条入口、一个换算）。⭐ 修前红在第一件事上（无注册者 ⇒ 计数一直停在装载那一条） |
//! | ②（小节号是等差的） | `"1"` ⇒ 0、`"2"`、`"3"` ⇒ 三步**严格递增**且两次差分**相等**（⛔ 不写死任何 tick 常量）；每一步 `+1` 条命令；`"not-a-bar"` ⇒ 一条都不发、位置不动 |
//! | ③（夹在网格两端，⛔ 不回绕） | 从 0 往左一步 ⇒ 仍是 0；一步、两步 ⇒ 差分相等；`"9999"` ⇒ 停在右缘 `r > 0`；从 `r` 再往右三步 ⇒ **仍是 `r`**（回绕会把它送回左边） |
//! | ④（章节卡片 = 投影里的那个起点，且不碰文档） | 逐个下标按下去 ⇒ 每次恰好 `+1` 条命令**且**引擎位置 `== live.view_section_start_tick(i)`（期望值从**本装配那一格投影**现读）；整段之后 `commit_count` **Δ0**（定位是会话运行态，`MODEL-ISO-001` 第二层） |
//! | ⑤（换工程之后读的是**新**投影） | 装配用 `filled_project`（2 段 / 1 景）⇒ 下标 2 越界；`apply_project(demo_project)`（4 段 / 4 景）⇒ 界面上的章节卡行数从 2 变 4、`view_section_start_tick(2)` 从 `None` 变 `Some(tick>0)`，按下卡片 ⇒ 位置**正是**那个 `tick`；同一个下标 2 的场景按钮现在报的是**新工程**那条场景的名字。⭐ 冻结的装配期快照在这里必然红（早退 ⇒ 位置停在 0） |
//! | ⑥（`scene-launch` 写的是**同一格**投影） | 启动态：`current-scene-index = -1`、状态行 `""`、`view_current_scene_name() = None`；按一次 ⇒ 状态行 `已排队：{名字}`、界面 `current-scene-index = 0`、`current-scene-name = 名字`，而**执行面自己那一格**投影也说同一个名字（⛔ 不是两份投影各说一套）；`apply_project` 之后回到 `-1` / `None` / `""`（完整重投影 ⇒ 读数作废） |
//! | ⑦（没有引擎时如实冷读，⛔ 不假装） | 不起引擎的装配：指针按下 / `SetValue` / 章节卡片 ⇒ 命令计数恒 `0`、位置恒 `0`、界面镜面恒 `0`；场景按钮 ⇒ 状态行如实说"没有送到引擎"，`current-scene-index` 仍 `-1`，投影里仍没有当前场景 |
//! | ⑧（出处纪律） | 源码现读（先剥 `//` ⇒ **注释不是实现站点**）：`live_surface.rs` 里三条定位接线**各恰好 1 处**、`Rc::new(RefCell::new(view.clone()))` **1 处**（投影只有一份，就在装配现场建）、`Rc::clone(&live_view)` **5 处**（五条接线共享它）、`EngineHost::new(` **1 处**、执行面**自己**发命令/算坐标 **0 处**（`send_transport(` / `.seek(` / `.play()` / `.stop()` / `commit_seek(` / `ruler_seek_tick(` / `ruler_step_tick(` / `ruler_bar_tick(` / `ruler_span_ticks(` / `tick_to_px(` 各 0）；`main.rs` 三条各仍为 1（两族⛔ 不互删）；**本判据文件自己** 0 处 `host::wire_` |
//!
//! ## 注入（每一条都由本文件的判据打红）
//!
//! * **注入 ①「删掉装配里那三句 `wire_*`」**（回到修前形状）⇒ ①②③④⑤⑥ 全红（无注册者 ⇒
//!   计数恒 0）＋ ⑧ 红（三处计数从 1 变 0）；
//! * **注入 ②「投影改回裸值、接线拿装配期快照 `view.clone()`」**⇒ ⑤ 红（新工程的段落
//!   在下标 2 越界 ⇒ 位置停在 0）＋ ⑥ 红（`apply_project` 换裸值 ⇒ 执行面那一格还说旧名字）
//!   ＋ ⑧ 红（`Rc::new(RefCell::new(view.clone()))` 从 1 变 2 或 3）；
//! * **注入 ③「`apply_project` 里补一句重接三条定位接线」**（本票**不该**重接）⇒ ⑧ 红
//!   （每条计数从 1 变 2）；
//! * **注入 ④「在 `live_surface.rs` 的心跳里顺手 `seek` 一次」**⇒ ① 的前置读数 `0` 变 `>0`
//!   ＋ ⑧ 的 `.seek(` / `commit_seek(` 计数 `0` 变 `1`；
//! * **注入 ⑤「判据侧自己补一次 `host::wire_ruler_seek`」**⇒ ⑧ 红（本文件 `host::wire_`
//!   计数 `0` 变 `1`）—— 这条纪律的意义：接线必须是装配自己的事。
//!
//! ## 如实登记的边界（⛔ 不假装整条链都通了）
//!
//! 1. **不证明"听得到"**：位置与命令读数取自引擎的走带镜面（RT→控制侧），逐样本那一侧
//!    由 `crates/yeban-engine` 的判据负责；
//! 2. **不证明场景"已经在下一小节线生效"**：⑥ 证的是"命令送到了引擎 + 三份读数一致"
//!    （`SceneLaunchDisposition::Direct`）；生效帧与 `retune` 由
//!    `tests/mcp_scene_launch.rs` / `tests/scene_launch_wiring.rs` 那族判；
//! 3. **不注入真指针**：本文件直接 `invoke_*` 回调（与 `D-UI-30` 同一口径），命中测试与
//!    像素→局部坐标那一段由 `tests/arrangement_ruler_seek.rs` 判 —— 那里的 `Rig` 自己接线，
//!    所以它⛔ 覆盖不到"装配接没接"；
//! 4. **不判 `Roll` 的横向滚动偏移**：`ruler-seek` 报的是**带内局部**像素，滚动由 `.slint`
//!    侧处理（本票⛔ 不扩范围）；
//! 5. **⛔ 不动 `wire_scene_launch` 的量化档**：`launch_quantization` 从投影现读，本文件
//!    不改走带配置 ⇒ 不引入对 `Bar` / `Off` 的新假设。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部且每线程只能装配一次；每例一新线程由 libtest
//! 负责 ⇒ 本文件可以有多个 `#[test]`（与 `tests/clip_mute_toggle.rs` 同一做法）。
//! ⑧ 不建窗口（纯读源码），因此⛔ 不碰平台。
//!
//! ## 本机不编译重依赖（`AGENTS.md` §5）
//!
//! Slint 是重依赖 ⇒ 本文件在本机**不编译**；编译与判决交 CI。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的装配入口、走带读数与那三格投影读数 ⇒ 其余项在本测试
// 目标里是 dead_code（`-D warnings` 会判红）。同款见 `tests/engine_error_banner.rs`。
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use live::{LiveUi, LiveWiringOptions, build_live_ui_with};
// `.iter()` / `row_count` 都在 `Model` trait 上（同款 `tests/arrangement_ruler_seek.rs`）。
use slint::{ComponentHandle, Model as _};
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;

/// 提交时间戳（固定值 ⇒ 判据确定性）。
const NOW_MS: u64 = 1_700_000_000_000;

/// 装配一台**带权威会话**的活窗口。
///
/// `start = true` ⇒ 顺手起一代引擎（控制驱动 ⇒ 走带命令会被应用）。
///
/// ⛔ 本函数**不调**任何 `host::wire_*`：接线必须是装配自己的事（那是本票的命题，
/// 与 `tests/d_ui_30_serve_transport_is_wired.rs` 第一条纪律同款）。
fn assemble(tag: &str, start: bool) -> (LiveUi, Rc<UndoPort>, MainWindow) {
    let project = yeban_model::samples::filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open(tag, "yeban-app", project.clone(), NOW_MS).expect("打开会话"),
    ));
    let mut live = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            // ⚠ 0 = 起引擎时⛔ 不推量子 ⇒ 每一条读数都只可能来自「装配」或「按键」。
            engine_quanta: 0,
            undo: Some(Rc::clone(&port)),
            ..LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + Tier-1 执行面");
    if start {
        live.start_engine().expect("起一代引擎");
    }
    let window = live.ui().clone_strong();
    (live, port, window)
}

/// 剥掉以 `//` 开头的行 ⇒ 只留**代码**（注释里的字面量⛔ 不算实现站点）。
fn code_only(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 界面上第 `index` 条**小节线**的列（读的是投影注入进窗口的那份模型，⛔ 判据不算像素）。
///
/// ⚠ 行数走 UFCS：`ModelRc` 上没有 `.rows()`（既有口径见
/// `tests/arrangement_ruler_seek.rs` 与 `tests/d_ui_1_serve_write_surface.rs`）。
fn bar_line_px(window: &MainWindow, index: usize) -> f32 {
    let model = window.get_bar_positions();
    assert!(
        index < slint::Model::row_count(&model),
        "夹具的投影必须画出第 {index} 条小节线（实得 {} 条）",
        slint::Model::row_count(&model)
    );
    model.iter().nth(index).expect("逐项必有值")
}

/// ① 标尺上那一次按下，把定位命令发给了**本装配自己的**宿主（判据侧一行接线都没做）。
#[test]
fn a_ruler_press_seeks_the_assemblys_own_host() {
    let (live, _port, window) = assemble("<d-ui-31-ruler>", true);

    // ---- 前置读数：这台装配只发过**装载即停住**那一条（`D-UI-32`）----
    assert_eq!(
        live.engine_transport_commands_applied(),
        1,
        "① 前提：装配 + 起引擎⛔ 不许自发「替用户定位」的命令，但**必须**自发一条 `Stop` \
         （换代即停住，`D-UI-32`；实测已应用 {} 条）—— 这条读数一旦 > 1，后面所有 `+1` \
         都失去意义",
        live.engine_transport_commands_applied()
    );
    assert_eq!(
        live.engine_transport_position_ticks(),
        0,
        "① 前提：起引擎 + 停住 ⇒ 位置在 0（⛔ 自由跑会把播放头自己走出去）"
    );

    // 落在第 3 条小节线上（= 屏读器口径的"第 3 小节"）。
    let column = bar_line_px(&window, 2);
    assert!(
        column > 0.0,
        "① 第 3 条小节线必须在第 0 列之右（实测 {column} px）"
    );

    window.invoke_ruler_seek(column);

    assert_eq!(
        live.engine_transport_commands_applied(),
        2,
        "① ⭐ 一次按下必须**恰好**再发一条命令到本装配的宿主（实测累计 {} 条 = 装载那条 \
         `Stop` + 这一条）—— 增量 0 条正是修前「`ruler-seek` 没有注册者」的形状",
        live.engine_transport_commands_applied()
    );
    let pointer_tick = live.engine_transport_position_ticks();
    assert!(
        pointer_tick > 0,
        "① 播放头必须离开 0（⛔ 点了不动 = 界面看起来是台死机器；实测 {pointer_tick} tick）"
    );
    assert_eq!(
        window.get_playhead_ticks(),
        i32::try_from(pointer_tick).expect("夹具的 tick 必须在 i32 内"),
        "① 界面上的播放头必须回写成引擎读数（`apply_transport` 是唯一写者）"
    );

    // 两条入口必须给同一个落点：指针在第 3 条小节线 ⇄ 屏读器"定位到第 3 小节"。
    window.invoke_ruler_seek_bar("3".into());
    assert_eq!(
        live.engine_transport_commands_applied(),
        3,
        "① `SetValue` 也是**一条**命令（实测累计 {} 条）",
        live.engine_transport_commands_applied()
    );
    assert_eq!(
        live.engine_transport_position_ticks(),
        pointer_tick,
        "① 同一个小节的两条入口必须落在**同一 tick**（指针 {} ⇄ 屏读器 {}）—— 不等就说明\
         其中一条自己另算了一遍网格",
        pointer_tick,
        live.engine_transport_position_ticks()
    );

    report_line(&format!(
        "[d-ui-31] ① 标尺按下 ⇒ 本装配宿主 +1 条命令、位置 {pointer_tick} tick；\
         屏读器同小节落点一致"
    ));
}

/// ② 「定位到第 N 小节」是等差阶梯，且解析失败一条都不发。
#[test]
fn the_bar_number_ladder_is_evenly_spaced() {
    let (live, _port, window) = assemble("<d-ui-31-bar-ladder>", true);

    window.invoke_ruler_seek_bar("1".into());
    let first = live.engine_transport_position_ticks();
    assert_eq!(first, 0, "② 第 1 小节起点恒为 tick 0（实测 {first}）");

    window.invoke_ruler_seek_bar("2".into());
    let second = live.engine_transport_position_ticks();
    window.invoke_ruler_seek_bar("3".into());
    let third = live.engine_transport_position_ticks();

    assert!(
        second > first && third > second,
        "② 小节号必须**严格递增**（实测 {first} / {second} / {third}）"
    );
    assert_eq!(
        third - second,
        second - first,
        "② 两次差分必须相等（一小节 = 一小节；实测 {}/{})",
        second - first,
        third - second
    );
    assert_eq!(
        live.engine_transport_commands_applied(),
        4,
        "② 三次按下 = 三条命令（实测累计 {} 条 = 装载那条 `Stop` + 这三条）",
        live.engine_transport_commands_applied()
    );

    // 解析失败 ⇒ 什么都不发（⛔ 不是"当成第 0 小节"，那会把用户的输入吞成一次跳转）。
    let before = live.engine_transport_commands_applied();
    window.invoke_ruler_seek_bar("not-a-bar".into());
    assert_eq!(
        live.engine_transport_commands_applied(),
        before,
        "② 非数字之后⛔ 一条命令都不许发（实测增量 {}）",
        live.engine_transport_commands_applied()
            .saturating_sub(before)
    );
    assert_eq!(
        live.engine_transport_position_ticks(),
        third,
        "② 非数字之后位置一位不动"
    );

    report_line(&format!(
        "[d-ui-31] ② 小节阶梯 0 / {second} / {third}（差分相等），坏输入零命令"
    ));
}

/// ③ 网格两端是**夹紧**：⛔ 不回绕、⛔ 不越过投影右缘。
#[test]
fn the_grid_edges_clamp_and_never_wrap() {
    let (live, _port, window) = assemble("<d-ui-31-clamp>", true);

    // 左端：从 0 往左一步 ⇒ 仍在 0（回绕会跳到右缘，那是最容易在产品里被当成"能用"的错）。
    window.invoke_ruler_step(-1);
    assert_eq!(
        live.engine_transport_position_ticks(),
        0,
        "③ 在第 1 小节往左一步必须**停在 0**（⛔ 不回绕到右缘）"
    );

    // 步长与 ② 的阶梯同口径：连续两步的差分相等（⛔ 不写死 tick）。
    window.invoke_ruler_step(1);
    let one_step = live.engine_transport_position_ticks();
    window.invoke_ruler_step(1);
    let two_steps = live.engine_transport_position_ticks();
    assert!(one_step > 0, "③ 一步必须真的移动（实测 {one_step} tick）");
    assert_eq!(
        two_steps.saturating_sub(one_step),
        one_step,
        "③ 两步 = 两格（实测差分 {} vs {}",
        two_steps.saturating_sub(one_step),
        one_step
    );

    // 从两步处往左退回起点 ⇒ 仍然不越界。
    window.invoke_ruler_step(-2);
    assert_eq!(
        live.engine_transport_position_ticks(),
        0,
        "③ 退两步必须正好回到 0，⛔ 不得为负或回绕"
    );

    // 右缘：越界的小节号 ⇒ 夹在网格右缘；此后继续往右一步 ⇒ 仍钉在右缘。
    window.invoke_ruler_seek_bar("9999".into());
    let edge = live.engine_transport_position_ticks();
    assert!(edge > 0, "③ 网格右缘必须离开 0（实测 {edge} tick）");
    for _ in 0..3 {
        window.invoke_ruler_step(1);
    }
    assert_eq!(
        live.engine_transport_position_ticks(),
        edge,
        "③ 在右缘继续往右三步必须**钉住**（实测 {}）—— 变小就是回绕",
        live.engine_transport_position_ticks()
    );

    report_line(&format!(
        "[d-ui-31] ③ 左端停在 0 / 步长等差 {one_step} tick / 右缘钉在 {edge} tick（不回绕）"
    ));
}

/// ④ 章节卡片按下去 = 投影里那一段的起点，而且**不碰文档**。
#[test]
fn section_cards_seek_to_the_projected_start_and_commit_nothing() {
    let (live, port, window) = assemble("<d-ui-31-section>", true);

    let commits_before = port.display().commit_count;
    let mut checked = 0_usize;
    let mut index = 0_usize;
    while index < 16 {
        // 期望值从**本装配那一格投影**现读（⛔ 判据不重算段落表）。
        let Some(tick) = live.view_section_start_tick(index) else {
            break;
        };
        let before = live.engine_transport_commands_applied();
        let number = i32::try_from(index).expect("夹具的下标在 i32 内");
        window.invoke_section_seek(number);

        assert_eq!(
            live.engine_transport_commands_applied(),
            before + 1,
            "④ 卡片 {index} 必须**恰好**发一条定位命令（实测增量 {}）",
            live.engine_transport_commands_applied()
                .saturating_sub(before)
        );
        assert_eq!(
            live.engine_transport_position_ticks(),
            tick,
            "④ ⭐ 卡片 {index} 之后引擎位置必须等于**活投影**里那一段的起点 \
             （期望 {tick}，实测 {}）",
            live.engine_transport_position_ticks()
        );
        assert_eq!(
            window.get_playhead_ticks(),
            i32::try_from(tick).expect("夹具的 tick 必须在 i32 内"),
            "④ 界面上的播放头必须跟着回写（卡片 {index}）"
        );

        checked += 1;
        index += 1;
    }
    assert!(
        checked >= 2,
        "④ 夹具至少要有两段才谈得上「按卡片跳转」（实测 {checked} 段）"
    );

    // ⛔ 定位是会话运行态（`MODEL-ISO-001` 第二层）⇒ 一条 `Op` 都不许提交。
    assert_eq!(
        port.display().commit_count,
        commits_before,
        "④ {checked} 次章节跳转之后撤销栈⛔ 一位都不涨（实测增量 {}）—— 涨了就把「用户按了\
         一下标尺」写成了一次可撤销的工程编辑",
        port.display().commit_count.saturating_sub(commits_before)
    );

    // 越界下标 ⇒ 一条都不发（⛔ 不是"跳到第一段"）。
    let before = live.engine_transport_commands_applied();
    window.invoke_section_seek(i32::try_from(checked).expect("夹具的下标在 i32 内"));
    assert_eq!(
        live.engine_transport_commands_applied(),
        before,
        "④ 越界卡片下标必须零命令（实测增量 {}）",
        live.engine_transport_commands_applied()
            .saturating_sub(before)
    );

    report_line(&format!(
        "[d-ui-31] ④ {checked} 张章节卡片逐段命中，撤销栈 Δ0，越界下标零命令"
    ));
}

/// ⑤ 换工程之后，定位读的是**新**投影（冻结的装配期快照在这一条上必然红）。
#[test]
fn a_project_switch_moves_the_grid_that_seek_reads() {
    let (mut live, _port, window) = assemble("<d-ui-31-switch>", true);

    // ---- 装配工程（`filled_project`）的形状：两段 / 一景 ⇒ 下标 2 在这一格投影里不存在 ----
    assert!(
        live.view_section_start_tick(2).is_none(),
        "⑤ 前提：装配工程只有两段（实测第 3 段起点 = {:?}）",
        live.view_section_start_tick(2)
    );
    assert!(
        live.view_scene_name(2).is_none(),
        "⑤ 前提：装配工程只有一景（实测第 3 景名字 = {:?}）",
        live.view_scene_name(2)
    );
    let cards_before = slint::Model::row_count(&window.get_section_names());
    assert_eq!(
        cards_before, 2,
        "⑤ 前提：装配工程在界面上必须是 2 张章节卡（实测 {cards_before} 张）—— \
         这个数一变，下面那条「越界 ⇒ 有值」就没有区分力了"
    );

    live.apply_project(&yeban_model::samples::demo_project())
        .expect("⑤ 换工程（同一个活窗口上的重投影）");

    // 界面一侧：卡片数组跟着新工程走。
    let cards_after = slint::Model::row_count(&window.get_section_names());
    assert!(
        cards_after > cards_before,
        "⑤ 换工程之后界面上的章节卡必须换到新工程的条数（{cards_before} → {cards_after}）"
    );

    // 投影一侧：同一个下标现在**有值**了，而且与换代后的停留位置不同
    // （否则下面那条"位置必须等于它"就只是巧合）。
    let tick = live
        .view_section_start_tick(2)
        .expect("⑤ ⭐ 换工程之后活投影必须有第 3 段（装配期快照在这里必然还是 None）");
    assert!(
        tick > 0,
        "⑤ 夹具的第 3 段起点必须离开 0（实测 {tick} tick）"
    );
    let parked = live.engine_transport_position_ticks();
    assert_ne!(
        tick, parked,
        "⑤ 前提：第 3 段起点必须与换代后的停留位置不同（两处都是 {tick} ⇒ 本条失去区分力）"
    );

    let before = live.engine_transport_commands_applied();
    window.invoke_section_seek(2);
    assert_eq!(
        live.engine_transport_commands_applied(),
        before + 1,
        "⑤ 换工程之后按下卡片仍必须发命令（实测增量 {}）",
        live.engine_transport_commands_applied()
            .saturating_sub(before)
    );
    assert_eq!(
        live.engine_transport_position_ticks(),
        tick,
        "⑤ ⭐ 按下新工程的第 3 张卡片 ⇒ 位置必须是**新**那一段的起点（期望 {tick}，实测 {}）；\
         停在 0 就是接线里那份投影还指着装配期工程",
        live.engine_transport_position_ticks()
    );

    // 场景那条腿：同一个下标现在指着**新工程**的场景。
    let name = live
        .view_scene_name(2)
        .expect("⑤ 换工程之后活投影必须有第 3 景");
    window.invoke_scene_launch(2);
    let status = window.get_scene_launch_status().to_string();
    assert!(
        status.contains(&name),
        "⑤ 状态行必须报**新**工程那景的名字「{name}」（实测「{status}」）"
    );
    assert_eq!(
        window.get_current_scene_index(),
        2,
        "⑤ 界面镜面的「当前场景」必须是第 3 景（修前根本没有注册者 ⇒ 恒为 -1）"
    );

    report_line(&format!(
        "[d-ui-31] ⑤ 换工程：卡片 {cards_before}→{cards_after} 张、第 3 段起点 {tick} tick 命中、\
         第 3 景「{name}」上屏"
    ));
}

/// ⑥ 场景按钮写的就是执行面读的那**同一格**投影（⛔ 两份投影各说一套）。
#[test]
fn a_scene_launch_writes_the_shared_cell_and_the_interface() {
    let (mut live, _port, window) = assemble("<d-ui-31-scene>", true);

    // ---- 启动态：谁都没启动过场景 ----
    assert_eq!(
        window.get_current_scene_index(),
        -1,
        "⑥ 启动态：界面没有当前场景"
    );
    assert_eq!(
        window.get_scene_launch_status().to_string(),
        "",
        "⑥ 启动态：状态行必须是空串（不占视觉空间，也不谎报）"
    );
    assert_eq!(
        live.view_current_scene_name(),
        None,
        "⑥ 启动态：执行面那一格投影也说没有当前场景"
    );

    let name = live
        .view_scene_name(0)
        .expect("⑥ 夹具至少要有第一个场景（读的是本装配那一格投影）");
    window.invoke_scene_launch(0);

    let status = window.get_scene_launch_status().to_string();
    assert!(
        status.starts_with("已排队：") && status.contains(&name),
        "⑥ ⭐ 按下场景按钮之后状态行必须如实回执「已排队：{name}」（实测「{status}」）—— \
         修前这条回调没有注册者 ⇒ 永远是空串，而 MCP 的 `ui/launch_scene` 却能用"
    );
    assert_eq!(
        window.get_current_scene_index(),
        0,
        "⑥ 界面镜面必须写进下标 0"
    );
    assert_eq!(
        window.get_current_scene_name().to_string(),
        name,
        "⑥ 界面镜面的场景名必须来自同一格投影"
    );
    assert_eq!(
        live.view_current_scene_name().as_deref(),
        Some(name.as_str()),
        "⑥ ⭐ 执行面**自己那一格**投影必须与界面镜面同名（不等 = 接线读的是装配期快照，\
         而投影换了）"
    );

    // 完整重投影 ⇒ 当前场景作废（同一条写者 `apply_current_scene`，⛔ 不各清一份）。
    live.apply_project(&yeban_model::samples::filled_project())
        .expect("⑥ 换工程");
    assert_eq!(
        live.view_current_scene_name(),
        None,
        "⑥ 换工程之后当前场景必须作废（实测 {:?}）",
        live.view_current_scene_name()
    );
    assert_eq!(
        window.get_current_scene_index(),
        -1,
        "⑥ 换工程之后界面镜面也必须回到 -1"
    );
    assert_eq!(
        window.get_scene_launch_status().to_string(),
        "",
        "⑥ 换工程之后上一次的「已排队」⛔ 不得继续挂在状态行上"
    );

    report_line("[d-ui-31] ⑥ 场景启动：回执 + 界面镜面 + 投影那一格三处一致，换工程一起作废");
}

/// ⑦ 没有引擎的那台装配：按下之后⛔ 不假装切换过（冷读数，一路都是 `0` / 如实回执）。
#[test]
fn without_an_engine_no_seek_moves_nothing_and_the_receipt_stays_honest() {
    let (live, _port, window) = assemble("<d-ui-31-cold>", false);

    window.invoke_ruler_seek(bar_line_px(&window, 1));
    window.invoke_ruler_seek_bar("2".into());
    window.invoke_ruler_step(1);
    window.invoke_section_seek(0);

    assert_eq!(
        live.engine_transport_commands_applied(),
        0,
        "⑦ 没起引擎 ⇒ 一条命令都⛔ 不可能被应用（`send_transport` 在 `!transport_ready()` \
         时如实返回冷读数）；实测 {}",
        live.engine_transport_commands_applied()
    );
    assert_eq!(
        live.engine_transport_position_ticks(),
        0,
        "⑦ 没起引擎 ⇒ 位置读数必须是 0（⛔ 界面自己算一个假位置）"
    );
    assert_eq!(
        window.get_playhead_ticks(),
        0,
        "⑦ 界面镜面⛔ 不得比引擎多说一句（拿冷读数写成非 0 就是假状态）"
    );

    // 场景那条腿必须**如实**说没送到（把它做成"看起来启动了"是死按钮的另一种形态）。
    window.invoke_scene_launch(0);
    let status = window.get_scene_launch_status().to_string();
    assert!(
        status.contains("没有送到引擎"),
        "⑦ 无引擎时状态行必须如实报告（实测「{status}」）"
    );
    assert_eq!(
        window.get_current_scene_index(),
        -1,
        "⑦ 无引擎 ⇒ 当前场景⛔ 不得被写进投影"
    );
    assert_eq!(
        live.view_current_scene_name(),
        None,
        "⑦ 无引擎 ⇒ 投影里也⛔ 不得有当前场景（回执与读数必须一起诚实）"
    );

    report_line("[d-ui-31] ⑦ 无引擎装配：四条定位入口零命令、零位置、回执如实");
}

/// ⑧ 出处纪律（⛔ 不开窗口）：三条接线各一处、投影只有一格、执行面自己零算术零命令。
#[test]
fn the_execution_surface_wires_seek_once_and_keeps_one_projection_cell() {
    // ⚠ 这个前缀是**拼**出来的（`concat!` 的两段之间隔着 `", "` ⇒ 本文件的**代码**里不会出现
    // 完整的那一串）：否则最后那条"判据自己不替装配接线"的自计数会数到本文件里那些**搜索词**
    // 自己，永远红。声明在本函数第一条语句**之前**（`clippy::items_after_statements` 是 deny 级）。
    const WIRE_PREFIX: &str = concat!("host::", "wire_");

    let live = code_only(include_str!("../src/live_surface.rs"));
    let main = code_only(include_str!("../src/main.rs"));
    let itself = code_only(include_str!("d_ui_31_serve_seek_is_wired.rs"));

    // 接线点：各**恰好一处**（在装配里）。0 处正是本票修掉的缺陷，2 处就是重复注册者。
    for name in ["ruler_seek", "section_seek", "scene_launch"] {
        let token = format!("{WIRE_PREFIX}{name}(");
        assert_eq!(
            live.matches(token.as_str()).count(),
            1,
            "⑧ `live_surface.rs` 代码里 `{token}` 必须是**恰好 1 处**（实测 {} 处）—— \
             装配里 0 处 = 界面上点不动，2 处 = 两个注册者（Slint 是覆盖语义，但那是运气，\
             不是设计）",
            live.matches(token.as_str()).count()
        );
        // 产品那条腿必须还在（两族⛔ 不得互删）。
        assert_eq!(
            main.matches(token.as_str()).count(),
            1,
            "⑧ `main.rs` 代码里 `{token}` 必须仍是 1 处 —— 实测 {} 处",
            main.matches(token.as_str()).count()
        );
    }

    // 投影只许有一格：`RefCell` 只在装配现场建**一次**，五条接线靠 `Rc::clone` 共享。
    assert_eq!(
        live.matches("Rc::new(RefCell::new(view.clone()))").count(),
        1,
        "⑧ `live_surface.rs` 里造投影格的语句必须**恰好 1 处**（实测 {} 处）—— 修前这里是\
         **2 处**（资源栏与剪辑作用域各造一份装配期快照），加上裸值就是三份事实源",
        live.matches("Rc::new(RefCell::new(view.clone()))").count()
    );
    assert_eq!(
        live.matches("Rc::clone(&live_view)").count(),
        5,
        "⑧ `Rc::clone(&live_view)` 必须是 **5 处**（资源栏 / 剪辑作用域 / 标尺 / 段落 / 场景，\
         实测 {} 处）—— 少一处就是那条接线没拿共享格"
    );
    // 判据证据那份必须是**值**拷贝（⛔ 不是把 `Rc` 分出去，否则证据跟着执行面一起变）。
    assert_eq!(
        live.matches("self.surface.view.borrow().clone()").count(),
        2,
        "⑧ `into_control_plane` / `into_production_control_plane` 两处取值拷贝必须各 1 处\
         （实测 {} 处）",
        live.matches("self.surface.view.borrow().clone()").count()
    );
    assert_eq!(
        live.matches("EngineHost::new(").count(),
        1,
        "⑧ `EngineHost::new(` 必须**恰好 1 处**（实测 {} 处）—— 第二处就是「界面上按的」与\
         「判据读的」两份走带",
        live.matches("EngineHost::new(").count()
    );

    // 执行面自己不发送命令、不算坐标：算术⛔ 只住在 `host` / `bridge` 里。
    for token in [
        "send_transport(",
        ".seek(",
        ".play()",
        ".stop()",
        "stop_and_rewind(",
        "commit_seek(",
        "queue_scene_launch(",
        "ruler_seek_tick(",
        "ruler_step_tick(",
        "ruler_bar_tick(",
        "ruler_span_ticks(",
        "tick_to_px(",
        "roll_ticks_per_pixel(",
    ] {
        assert_eq!(
            live.matches(token).count(),
            0,
            "⑧ `live_surface.rs` 代码里 `{token}` 必须 0 处（实测 {} 处）—— 执行面自己发命令\
             或重算网格，就等于替用户按了一次键，①②③④ 的前置读数与增量都失去意义",
            live.matches(token).count()
        );
    }

    // ⛔ 判据不替装配接线：本文件的代码里一处接线调用都不许有（注入 ⑤ 的机械防线）。
    assert_eq!(
        itself.matches(WIRE_PREFIX).count(),
        0,
        "⑧ 本判据文件里宿主接线调用的完整前缀必须 0 处（实测 {} 处）—— 判据补接线 = 把本票\
         要证的那件事偷偷做掉，之后这条链再断也读不出来",
        itself.matches(WIRE_PREFIX).count()
    );

    report_line(
        "[d-ui-31] ⑧ 源码纪律：三条接线各 1 处 / main 各 1 处、投影格 1 份 + 共享 5 处、\
         值拷贝 2 处、宿主 1 份、执行面自发命令与自算坐标 0 处、判据自接 0 处",
    );
}
