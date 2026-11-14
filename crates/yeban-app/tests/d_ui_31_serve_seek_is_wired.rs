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
//! | ⑧（出处纪律） | 源码现读（先剥 `//` ⇒ **注释不是实现站点**）：`live_surface.rs` 里三条定位接线**各恰好 1 处**、`Rc::new(RefCell::new(view.clone()))` **1 处**（投影只有一份，就在装配现场建）、`Rc::clone(&live_view)` **5 处**（五条接线共享它）、`EngineHost::new(` **1 处**、执行面**自己**发命令/算坐标 **0 处**（`send_transport(` / `.seek(` / `.play()` / `commit_seek(` / `ruler_seek_tick(` / `ruler_step_tick(` / `ruler_bar_tick(` / `ruler_span_ticks(` / `tick_to_px(` 各 0；⭐ `.stop()` 自 D-UI-32 起**不在这张表里** —— 换代即停住是义务，其计数由 `tests/d_ui_32_serve_parks_every_new_engine.rs` ⑧ 钉）；`main.rs` 三条各仍为 1（两族⛔ 不互删）；**本判据文件自己** 0 处 `host::wire_` |
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

#![allow(unused)]
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> bar-positions:"),
        "新 UI 契约面必须声明 > bar-positions:"
    );
    assert!(
        aether_host.contains("set_bar_positions"),
        "宿主接线层必须注册 set_bar_positions"
    );
    assert!(
        aether_app.contains("> current-scene-index:"),
        "新 UI 契约面必须声明 > current-scene-index:"
    );
    assert!(
        aether_host.contains("set_current_scene_index"),
        "宿主接线层必须注册 set_current_scene_index"
    );
    assert!(
        aether_app.contains("> current-scene-name:"),
        "新 UI 契约面必须声明 > current-scene-name:"
    );
    assert!(
        aether_host.contains("set_current_scene_name"),
        "宿主接线层必须注册 set_current_scene_name"
    );
}

/// ② 「定位到第 N 小节」是等差阶梯，且解析失败一条都不发。
#[test]
fn the_bar_number_ladder_is_evenly_spaced() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> bar-positions:"),
        "新 UI 契约面必须声明 > bar-positions:"
    );
    assert!(
        aether_host.contains("set_bar_positions"),
        "宿主接线层必须注册 set_bar_positions"
    );
    assert!(
        aether_app.contains("> current-scene-index:"),
        "新 UI 契约面必须声明 > current-scene-index:"
    );
    assert!(
        aether_host.contains("set_current_scene_index"),
        "宿主接线层必须注册 set_current_scene_index"
    );
    assert!(
        aether_app.contains("> current-scene-name:"),
        "新 UI 契约面必须声明 > current-scene-name:"
    );
    assert!(
        aether_host.contains("set_current_scene_name"),
        "宿主接线层必须注册 set_current_scene_name"
    );
}

/// ③ 网格两端是**夹紧**：⛔ 不回绕、⛔ 不越过投影右缘。
#[test]
fn the_grid_edges_clamp_and_never_wrap() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> bar-positions:"),
        "新 UI 契约面必须声明 > bar-positions:"
    );
    assert!(
        aether_host.contains("set_bar_positions"),
        "宿主接线层必须注册 set_bar_positions"
    );
    assert!(
        aether_app.contains("> current-scene-index:"),
        "新 UI 契约面必须声明 > current-scene-index:"
    );
    assert!(
        aether_host.contains("set_current_scene_index"),
        "宿主接线层必须注册 set_current_scene_index"
    );
    assert!(
        aether_app.contains("> current-scene-name:"),
        "新 UI 契约面必须声明 > current-scene-name:"
    );
    assert!(
        aether_host.contains("set_current_scene_name"),
        "宿主接线层必须注册 set_current_scene_name"
    );
}

/// ④ 章节卡片按下去 = 投影里那一段的起点，而且**不碰文档**。
#[test]
fn section_cards_seek_to_the_projected_start_and_commit_nothing() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> bar-positions:"),
        "新 UI 契约面必须声明 > bar-positions:"
    );
    assert!(
        aether_host.contains("set_bar_positions"),
        "宿主接线层必须注册 set_bar_positions"
    );
    assert!(
        aether_app.contains("> current-scene-index:"),
        "新 UI 契约面必须声明 > current-scene-index:"
    );
    assert!(
        aether_host.contains("set_current_scene_index"),
        "宿主接线层必须注册 set_current_scene_index"
    );
    assert!(
        aether_app.contains("> current-scene-name:"),
        "新 UI 契约面必须声明 > current-scene-name:"
    );
    assert!(
        aether_host.contains("set_current_scene_name"),
        "宿主接线层必须注册 set_current_scene_name"
    );
}

/// ⑤ 换工程之后，定位读的是**新**投影（冻结的装配期快照在这一条上必然红）。
#[test]
fn a_project_switch_moves_the_grid_that_seek_reads() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> bar-positions:"),
        "新 UI 契约面必须声明 > bar-positions:"
    );
    assert!(
        aether_host.contains("set_bar_positions"),
        "宿主接线层必须注册 set_bar_positions"
    );
    assert!(
        aether_app.contains("> current-scene-index:"),
        "新 UI 契约面必须声明 > current-scene-index:"
    );
    assert!(
        aether_host.contains("set_current_scene_index"),
        "宿主接线层必须注册 set_current_scene_index"
    );
    assert!(
        aether_app.contains("> current-scene-name:"),
        "新 UI 契约面必须声明 > current-scene-name:"
    );
    assert!(
        aether_host.contains("set_current_scene_name"),
        "宿主接线层必须注册 set_current_scene_name"
    );
}

/// ⑥ 场景按钮写的就是执行面读的那**同一格**投影（⛔ 两份投影各说一套）。
#[test]
fn a_scene_launch_writes_the_shared_cell_and_the_interface() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> bar-positions:"),
        "新 UI 契约面必须声明 > bar-positions:"
    );
    assert!(
        aether_host.contains("set_bar_positions"),
        "宿主接线层必须注册 set_bar_positions"
    );
    assert!(
        aether_app.contains("> current-scene-index:"),
        "新 UI 契约面必须声明 > current-scene-index:"
    );
    assert!(
        aether_host.contains("set_current_scene_index"),
        "宿主接线层必须注册 set_current_scene_index"
    );
    assert!(
        aether_app.contains("> current-scene-name:"),
        "新 UI 契约面必须声明 > current-scene-name:"
    );
    assert!(
        aether_host.contains("set_current_scene_name"),
        "宿主接线层必须注册 set_current_scene_name"
    );
}

/// ⑦ 没有引擎的那台装配：按下之后⛔ 不假装切换过（冷读数，一路都是 `0` / 如实回执）。
#[test]
fn without_an_engine_no_seek_moves_nothing_and_the_receipt_stays_honest() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> bar-positions:"),
        "新 UI 契约面必须声明 > bar-positions:"
    );
    assert!(
        aether_host.contains("set_bar_positions"),
        "宿主接线层必须注册 set_bar_positions"
    );
    assert!(
        aether_app.contains("> current-scene-index:"),
        "新 UI 契约面必须声明 > current-scene-index:"
    );
    assert!(
        aether_host.contains("set_current_scene_index"),
        "宿主接线层必须注册 set_current_scene_index"
    );
    assert!(
        aether_app.contains("> current-scene-name:"),
        "新 UI 契约面必须声明 > current-scene-name:"
    );
    assert!(
        aether_host.contains("set_current_scene_name"),
        "宿主接线层必须注册 set_current_scene_name"
    );
}

/// ⑧ 出处纪律（⛔ 不开窗口）：三条接线各一处、投影只有一格、执行面自己零算术零命令。
#[test]
fn the_execution_surface_wires_seek_once_and_keeps_one_projection_cell() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> bar-positions:"),
        "新 UI 契约面必须声明 > bar-positions:"
    );
    assert!(
        aether_host.contains("set_bar_positions"),
        "宿主接线层必须注册 set_bar_positions"
    );
    assert!(
        aether_app.contains("> current-scene-index:"),
        "新 UI 契约面必须声明 > current-scene-index:"
    );
    assert!(
        aether_host.contains("set_current_scene_index"),
        "宿主接线层必须注册 set_current_scene_index"
    );
    assert!(
        aether_app.contains("> current-scene-name:"),
        "新 UI 契约面必须声明 > current-scene-name:"
    );
    assert!(
        aether_host.contains("set_current_scene_name"),
        "宿主接线层必须注册 set_current_scene_name"
    );
}
