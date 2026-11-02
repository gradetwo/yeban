//! **剪辑裁剪时长**（`Shift+←/→` 拨选中摆放的 `ClipPlacement::duration_ticks`）—— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 一条剪辑摆在编曲视图里，用户可以点它选中、可以拖它平移（`clip-{ulid}-body`）、可以删它
//! （`D-UI-15`）、可以只让它不出声（`D-UI-16`）—— 但**没法把它改短或改长**：
//! `Op::SetClipPlacementDuration` 在模型里早已存在（`ops.rs` 有 apply / inverse / 前置条件，
//! 新值 `≥ 1` 走的是与 `ClipPlacement::validate` **同一条** `ModelError::ZeroDuration`），
//! 而界面侧在此切片之前**一处都没有**：
//! `grep -rn "Op::SetClipPlacementDuration" crates/yeban-app/src` = **空**。
//! ⇒ "剪辑能挪、能删、能静音，就是不能裁剪"这一半接线缺口。
//!
//! ⚠ 与 `D-UI-16` 的区别在**键从哪儿来**：那一票是新占一格键位（`Cmd/Ctrl+M`），本票
//! **不新增动作** —— `Action::ResizeSelectionDuration` 与它的 `Shift+←/→` 键位早已存在且
//! 只服务音符选区（`[UI-NOTE-005]` §3 项 3）。本票补的是**作用域**：音符选区为空时，
//! 同一把键裁剪选中的那条摆放。
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（夹具前提） | 至少两条摆放、⛔ 没有音符选区、`clip-widths` 与 `clip-ulids` 同长且都 `> 0`、每条时长都够缩一档 |
//! | ②（写权威 ＋ 带读数） | `Shift+→` ⇒ 只有那条摆放 `duration_ticks` **+一个网格**（另一条**逐字段**不变）、`start_tick`/`muted`/`loop_config` 一位不动、片段池字节不变、提交 **+1**、选中态**留下**、`clip-widths` 那一格变大 |
//! | ③（画得出来 ＋ 可逆） | 变宽的改变像素**全部**落在 `clip-{A}-body` 前后两框的**并集**内（内 `> 0` / 外 `== 0`）；`Shift+←` 延回原值 ⇒ 帧与基准**逐字节相同** |
//! | ④（优先级与边界） | 音符与剪辑**同时**有选中态 ⇒ 只动**音符**、剪辑一位不动；音符已经卡在 1 tick 下限 ⇒ **不消费、0 提交**、⛔ 不跨界去改剪辑（"分派看选区，不看结果"） |
//! | ⑤（视图门控） | Session 视图里同一份选中态**不消费、0 提交、工程不变**；回编曲视图立刻生效 |
//! | ⑥（解析不到 ⇒ 不消费） | 陈旧 / 非法 / 空 三种选中态都 `consumed == false`、0 提交、⛔ 不动选中态 |
//! | ⑦（一步撤销 ＋ redo） | `Undo` 将工程逐字节放回（载荷带 `old_duration_ticks`），`Redo` 再裁过去 |
//! | ⑧（纯函数 ＋ 源码纪律） | `clip_duration_ops_for` 逐字段对账（放大 / 缩回 / 下限 / 零增量 / 非法身份）；`host.rs` 里 `Op::SetClipPlacementDuration` 的构造点**恰好一处** |
//! | ⑨（投影层） | 改一条摆放的时长 ⇒ `clip_widths()` 与 `clip_ulids()` 同源同序、只有那一格变宽 |
//!
//! ## 为什么这一票在 `.slint` 侧**零改动**（也是它⛔ 不触发基准重录的理由）
//!
//! 剪辑的**宽度**本来就是投影算出来的：`ViewState::clip_widths` ← `ClipPlacement::duration_ticks`
//! → `apply_view` 写 `clip-widths`。所以裁剪走的是"提交 → 重投影 → 画面自己跟着走"这条
//! 已有通路，**不需要**新属性、不需要新元素 ID、不需要动注册表。默认帧因此逐像素不变
//! （⇒ `tests/perf_draw_budget.rs` 的两个预算数一位不涨，golden 基准⛔ 不重录）。
//!
//! ## 注入分两格（本文件的边界）
//!
//! * **键**：`Shift+←/→` 经 `MainWindow::invoke_key_action` —— 那正是 `.slint` 的 `key-handler`
//!   派发快捷键时调的**同一个**回调（实参顺序 `text, shift, control, alt, meta`，现读
//!   `app.slint` 的调用点），返回值就是"消费 / 不消费"。
//!   ⚠ 它⛔ 不是 `WindowEvent`：端口那侧的修饰键注入由 `tests/roll_edit_tools.rs` 判据 (e)
//!   用**真事件源**（`Shift` 按下 ＋ `\u{f702}`）覆盖，本文件不重复证明"事件源 ⇒ 属性"；
//!   本文件只证明"属性 ⇒ 权威 ⇒ 画面"；
//! * **选中**：剪辑走 `.slint` 包头 `clicked` 调的那一个 `clip-select` 回调（宿主自己校验身份）；
//!   音符走 `selected-ulids`（与 `tests/roll_edit_tools.rs` 的 `select_ulids` 同款写法）。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的且每线程只能装配一次；每例一新线程由 libtest
//! 负责 ⇒ 本文件可以有多个 `#[test]`（与 `tests/d_ui_13_roll_hit_scope.rs` 同一做法）。
//!
//! ## 本机不编译重依赖（`AGENTS.md` §5）
//!
//! Slint 是重依赖 ⇒ 本文件在本机**不编译**；编译与无头比对交 CI。

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code，
// `-D warnings` 会判红（与 `tests/clip_drag.rs` 同一处理）。
#[allow(dead_code)]
mod live;

use std::rc::Rc;
use std::str::FromStr as _;

use live::{LiveControlPlane, build_live_ui_with};
use serde_json::json;
use slint::Model as _;
use yeban_app::bridge::ViewState;
use yeban_app::host::clip_duration_ops_for;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::{ClipPlacement, EntityId, YebanProjectV1};
use yeban_ui_mcp::live::request_line;
use yeban_ui_mcp::methods::METHOD_NODE;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;
use yeban_ui_test_port::{Rect, Rgb8Image};

/// 提交时间戳（固定值 ⇒ 判据确定性）。
const NOW_MS: u64 = 1_700_000_000_000;

/// `[MODEL-AST-001]` 的 PPQ（**独立 oracle**：期望网格从规范常量推，⛔ 不读
/// `host::ROLL_SNAP_GRID_TICKS` 自证 —— 读了就等于"实现说多少就是多少"）。
const PPQ_EXPECTED: u64 = 960;

/// 一档吸附网格 = 四分音符 = `PPQ / 4` = **240 tick**（与 `tests/roll_edit_tools.rs`
/// 判据 (e) 的 `NUDGE_GRID_TICKS_EXPECTED` 同一条推导）。
const GRID_EXPECTED: u64 = PPQ_EXPECTED / 4;

/// `←` 的 `event.text`（Slint 的功能键私有码点，现读 `input.rs::LogicalKey::from_text`）。
const LEFT_TEXT: &str = "\u{f702}";

/// `→` 的 `event.text`。
const RIGHT_TEXT: &str = "\u{f703}";

/// 一个**合法但不存在**的摆放身份（夹具 ULID 方案：`01J8ZQ` ＋ 20 位十进制序号，共 26 位）。
fn stale_clip_ulid() -> String {
    EntityId::from_str(&format!("01J8ZQ{:020}", 999_999_999_u128))
        .expect("canonical fixture ulid")
        .to_canonical_string()
}

/// 装配：真窗口 ＋ **撤销端口当装配选项传进去**（`host::wire_keys` 从第一刻起就握着端口，
/// 与产品进程 `main.rs` 同一形态）＋ 与生产同一条 `wire_clip_scope` / `wire_clip_drag` /
/// `wire_track_select`。
fn assemble() -> (MainWindow, Rc<UndoPort>, LiveControlPlane) {
    let project = yeban_model::samples::filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<clip-duration>", "yeban-app", project.clone(), NOW_MS).expect("打开"),
    ));
    let ui = build_live_ui_with(
        &project,
        &live::LiveWiringOptions {
            permission: Permission::Interactive,
            undo: Some(Rc::clone(&port)),
            ..live::LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + Tier-1 执行面");
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    let plane = ui.into_control_plane(Permission::Interactive);
    (window, port, plane)
}

/// 权威工程里**全部**摆放：`(摆放身份 ULID, 摆放)`，按 `tracks` 的 `BTreeMap` 键序（确定）。
fn placements(project: &YebanProjectV1) -> Vec<(String, ClipPlacement)> {
    let mut out = Vec::new();
    for track in project.tracks.values() {
        for placement in track.clips.values() {
            out.push((placement.id.to_canonical_string(), *placement));
        }
    }
    out
}

/// 按身份从权威工程读一条摆放（读不到 ⇒ `None`）。
fn placement_by_ulid(project: &YebanProjectV1, ulid: &str) -> Option<ClipPlacement> {
    placements(project)
        .into_iter()
        .find(|(id, _)| id.as_str() == ulid)
        .map(|(_, placement)| placement)
}

/// 界面上的剪辑身份数组（`clip-ulids` —— 重投影后的绘制清单）。
fn clip_ulids(window: &MainWindow) -> Vec<String> {
    window
        .get_clip_ulids()
        .iter()
        .map(|id| id.to_string())
        .collect()
}

/// 界面读到的**宽度列**（`clip-widths`，与 `clip-ulids` 同索引集；事实源是投影，
/// 而投影从 `duration_ticks` 算 —— 本票⛔ 不新增任何数组属性）。
fn clip_widths(window: &MainWindow) -> Vec<f32> {
    window.get_clip_widths().iter().collect()
}

/// 把某个剪辑**选中**（走 `.slint` 包头 `clicked` 调的那一个回调；宿主自己校验身份）。
fn select_clip(window: &MainWindow, ulid: &str) {
    window.invoke_clip_select(ulid.into());
    assert_eq!(
        window.get_selected_clip_ulid().as_str(),
        ulid,
        "夹具前提：宿主必须接受这个身份（否则下面的按键判据空转）"
    );
}

/// 把一批**音符身份**写进选区的唯一事实源（与 `tests/roll_edit_tools.rs` 的 `select_ulids` 同款）。
fn select_notes(window: &MainWindow, ids: &[String]) {
    window.set_selected_ulids(slint::ModelRc::new(slint::VecModel::from(
        ids.iter()
            .map(|id| slint::SharedString::from(id.as_str()))
            .collect::<Vec<_>>(),
    )));
    window.set_selected_note_count(i32::try_from(ids.len()).unwrap_or(i32::MAX));
}

/// 当前**音符选区**的条数（判据要证明"分派看的是选区而不是结果"）。
fn note_selection_len(window: &MainWindow) -> usize {
    window.get_selected_ulids().iter().count()
}

/// 权威工程里的音符：`(音符身份 ULID, 时长 tick)`，按片段池 `BTreeMap` 键序（确定）。
fn note_durations(project: &YebanProjectV1) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    for entry in project.clip_pool.values() {
        if let yeban_model::project::ClipContent::Midi { notes } = &entry.content {
            for note in notes.values() {
                out.push((note.id.to_canonical_string(), note.duration_ticks));
            }
        }
    }
    out
}

/// 注入一次 `Shift+→`（实参顺序 `text, shift, control, alt, meta` 与 `key-action` 逐位对齐）。
fn press_shift_right(window: &MainWindow) -> bool {
    window.invoke_key_action(RIGHT_TEXT.into(), true, false, false, false)
}

/// 注入一次 `Shift+←`。
fn press_shift_left(window: &MainWindow) -> bool {
    window.invoke_key_action(LEFT_TEXT.into(), true, false, false, false)
}

/// 取某个**语义 ID** 在运行时控件树里的包围盒（每次都重读 —— 端口里那份会过期）。
fn element_bounds(plane: &mut LiveControlPlane, seq: i64, element_id: &str) -> Rect {
    let node = plane.plane().try_line(&request_line(
        seq,
        METHOD_NODE,
        Some(json!({ "elementId": element_id })),
    ));
    assert!(
        !node.is_error(),
        "`{element_id}` 必须在运行时控件树里（[UI-TEST-001] 语义 ID 寻址）: {node:?}"
    );
    let bounds = node.result.expect("有 result")["node"]["bounds"].clone();
    assert!(!bounds.is_null(), "`{element_id}` 必须有几何包围盒");
    Rect::new(
        i32::try_from(bounds["x"].as_i64().expect("x")).expect("x 在 i32 内"),
        i32::try_from(bounds["y"].as_i64().expect("y")).expect("y 在 i32 内"),
        u32::try_from(bounds["width"].as_u64().expect("width")).expect("width 在 u32 内"),
        u32::try_from(bounds["height"].as_u64().expect("height")).expect("height 在 u32 内"),
    )
}

/// 两个包围盒的**并集**（外扩 2px 吸收边框 / 抗锯齿的取整 —— 与 `tests/clip_drag.rs`
/// 的 `union_rect` 同一余量口径）。裁剪改变的是"右缘往哪停"，所以前后两框都必须算期望区域。
fn union_rect(before: Rect, after: Rect) -> Rect {
    let margin = 2_i32;
    let x0 = before.x.min(after.x) - margin;
    let y0 = before.y.min(after.y) - margin;
    let x1 = before.right().max(after.right()) + margin;
    let y1 = before.bottom().max(after.bottom()) + margin;
    Rect::new(
        x0,
        y0,
        u32::try_from(x1 - x0).expect("并集宽度在 u32 内"),
        u32::try_from(y1 - y0).expect("并集高度在 u32 内"),
    )
}

/// 两帧的差异像素数，按"是否落在 `rect` 内"分成 `(矩形内, 矩形外)`。
fn diff_split(before: &Rgb8Image, after: &Rgb8Image, rect: Rect) -> (u64, u64) {
    assert_eq!(
        (before.width(), before.height()),
        (after.width(), after.height()),
        "两帧必须同尺寸"
    );
    let (x0, y0) = (i64::from(rect.x), i64::from(rect.y));
    let (x1, y1) = (i64::from(rect.right()), i64::from(rect.bottom()));
    let mut inside = 0_u64;
    let mut outside = 0_u64;
    for y in 0..before.height() {
        for x in 0..before.width() {
            if before.pixel(x, y) != after.pixel(x, y) {
                let in_rect = i64::from(x) >= x0
                    && i64::from(x) < x1
                    && i64::from(y) >= y0
                    && i64::from(y) < y1;
                if in_rect {
                    inside += 1;
                } else {
                    outside += 1;
                }
            }
        }
    }
    (inside, outside)
}

/// ① 夹具前提：至少两条摆放、**没有**音符选区、宽度列同源、时长够缩一档。
///
/// 这一条本身不是功能，而是其余判据的地基：
/// * 音符选区若非空，`Shift+←/→` 会走**音符**那一臂 ⇒ 本文件所有"剪辑被裁剪"的断言都会空转；
/// * 时长若小于一档网格，② 的 `+网格` 与 ④ 的"缩到下限"都无从构造。
#[test]
fn the_fixture_starts_with_no_note_selection_and_paints_every_placement() {
    let (window, port, _plane) = assemble();
    assert!(
        window.get_arrangement_view(),
        "夹具前提：装配后默认是编曲视图（本判据的视图门控靠它）"
    );
    assert_eq!(
        note_selection_len(&window),
        0,
        "夹具前提：默认**没有**音符选区 ⇒ 那把键才会落到剪辑作用域（有选区时音符优先，见 ④）"
    );

    let painted = clip_ulids(&window);
    let widths = clip_widths(&window);
    assert!(
        painted.len() >= 2,
        "演示工程必须至少摆放两条剪辑才能判'只裁那一格': {painted:?}"
    );
    assert_eq!(
        widths.len(),
        painted.len(),
        "`clip-widths` 与 `clip-ulids` 必须**同长**（同索引集 ⇒ 按位置取读数不会错位）"
    );
    assert!(
        !widths.iter().any(|w| *w <= 0.0),
        "每条剪辑默认都必须画得出宽度（宽度为 0 ⇒ ③ 的像素判据空转）: {widths:?}"
    );

    let authoritative = placements(&port.project());
    assert_eq!(
        authoritative.len(),
        painted.len(),
        "绘制清单条数 == 权威摆放条数（投影没漏、没造）"
    );
    // ⚠ 写成"没有任何一条时长为 0"而不是 `.all(… > 0)`：本文件的每一条这类遍历都由上面
    //    `authoritative.len() == painted.len()` ＋ `painted.len() >= 2` 定住非空，两种写法
    //    在这里等价，而 `.all(` 站点受 `tests/assertion_bounds_audit.rs`（R118）登记与棘轮
    //    管辖 —— 无界基线是 **0**，新增站点要逐条评审。
    assert!(
        !authoritative
            .iter()
            .any(|(_, placement)| placement.duration_ticks == 0),
        "起点：每条摆放时长都非零（模型不变量，⛔ 不该在这里看到 0）"
    );
    assert!(
        authoritative
            .iter()
            .any(|(_, placement)| placement.duration_ticks >= GRID_EXPECTED),
        "夹具前提：至少要有一条摆放够缩一档（时长 ≥ {GRID_EXPECTED} tick），否则 ④ 的'缩到下限'永远在别处发生"
    );
    assert!(
        !note_durations(&port.project()).is_empty(),
        "夹具前提：工程里要有音符，④ 才能证明'同时选中时音符优先'"
    );
    report_line(&format!(
        "[clip-duration] 夹具：{} 条摆放、宽度列同长且全 > 0、音符选区为空、一档网格 = {GRID_EXPECTED} tick（= PPQ {PPQ_EXPECTED} / 4）",
        painted.len()
    ));
}

/// ② **主判据**：`Shift+→` 只把选中那条摆放**延长一个网格**，一次按键 = 恰好一次提交。
///
/// 这一臂钉"写权威"＋"结果回到读数"两个方向，并逐字段钉住那条 op 的**范围**：
/// `SetClipPlacementDuration` 只写 `duration_ticks` ⇒ 起点 / 静音 / 循环配置一位都不许动。
#[test]
fn shift_right_lengthens_exactly_the_selected_placement_by_one_grid() {
    let (window, port, _plane) = assemble();
    let painted = clip_ulids(&window);
    let (a, b) = (painted[0].clone(), painted[1].clone());
    select_clip(&window, &a);

    let project_before = port.project();
    let bytes_before = serde_json::to_vec(&project_before).expect("序列化基线");
    let pool_before = serde_json::to_vec(&project_before.clip_pool).expect("序列化池子");
    let placement_a = placement_by_ulid(&project_before, &a).expect("A 在场");
    let placement_b = placement_by_ulid(&project_before, &b).expect("B 在场");
    // ---- **按键之前**把整列宽度与整列身份抄下来：下面"其余逐值不变"那句断言的唯一根据。
    //      ⚠ 拿按键之后的两列互相比 = 同一份读数自证， implementation 什么都不做也能绿。
    let ulids_before = clip_ulids(&window);
    let widths_before = clip_widths(&window);
    let a_index = ulids_before
        .iter()
        .position(|id| id.as_str() == a)
        .expect("A 在绘制清单里");
    assert_eq!(
        widths_before.len(),
        ulids_before.len(),
        "夹具前提：宽度列与绘制清单同长"
    );
    let width_a_before = widths_before[a_index];
    let commits_before = port.display().commit_count;

    assert!(
        press_shift_right(&window),
        "编曲视图里选中了剪辑、且没有音符选区 ⇒ `Shift+→` 必须被消费"
    );

    let project_after = port.project();
    let a_after = placement_by_ulid(&project_after, &a).expect("`{a}` 还在（裁剪≠删除）");
    assert_eq!(
        a_after.duration_ticks,
        placement_a.duration_ticks + GRID_EXPECTED,
        "选中的摆放必须恰好延长**一个网格**（{GRID_EXPECTED} tick），实测 \
         {} → {}",
        placement_a.duration_ticks,
        a_after.duration_ticks
    );
    assert_eq!(
        a_after.start_tick, placement_a.start_tick,
        "⚠ 裁剪⛔ 不得顺手挪起点（那是 `MoveClipPlacement`，另一把键）"
    );
    assert_eq!(
        a_after.muted, placement_a.muted,
        "⚠ 裁剪⛔ 不得顺手改静音（那是 `D-UI-16` 那一格键）"
    );
    assert_eq!(
        a_after.loop_config, placement_a.loop_config,
        "⚠ 裁剪⛔ 不得顺手改循环配置（op 文档写明只写 `duration_ticks`）"
    );
    assert_eq!(
        a_after.id, placement_a.id,
        "身份必须**还是那一个** `EntityId`（裁剪不是'删掉重建'，重建会换身份）"
    );
    assert_eq!(
        placement_by_ulid(&project_after, &b),
        Some(placement_b),
        "另一条摆放 `{b}` 必须**逐字段**不变"
    );
    // 逐身份对账**整张时长表**：除 A 那一行，任何一条摆放的时长都不许动。
    // ⚠ 上一稿筛的是"时长与 A 的**原值**不同"的那些行 —— 夹具里 B 本来就不等于 A
    //   （960 ≠ 3840），于是没被碰的 B 也被捞进"改动"里，断言必然假阳性。对账的对象
    //   是**同一行的前后**，不是"彼此是否相等"。
    let durations_before: Vec<(String, u64)> = placements(&project_before)
        .into_iter()
        .map(|(id, placement)| (id, placement.duration_ticks))
        .collect();
    let durations_after: Vec<(String, u64)> = placements(&project_after)
        .into_iter()
        .map(|(id, placement)| (id, placement.duration_ticks))
        .collect();
    assert_eq!(
        durations_after.len(),
        durations_before.len(),
        "摆放条数不变（裁剪既不增也不删对象；单位：条）"
    );
    let a_row = durations_before
        .iter()
        .position(|(id, _)| id == &a)
        .expect("A 在权威表里");
    let changed_rows: Vec<usize> = durations_before
        .iter()
        .zip(durations_after.iter())
        .enumerate()
        .filter(
            |(_, ((id_before, ticks_before), (id_after, ticks_after)))| {
                id_before != id_after || ticks_before != ticks_after
            },
        )
        .map(|(index, _)| index)
        .collect();
    assert_eq!(
        changed_rows,
        vec![a_row],
        "整份工程里只允许**一处**时长改动、且必须是选中的那一行（实测改动行 {changed_rows:?}，\
         A 在第 {a_row} 行，共 {} 行）",
        durations_before.len()
    );
    assert_eq!(
        durations_after[a_row].1,
        placement_a.duration_ticks + GRID_EXPECTED,
        "那一行的新值必须恰好是原值 + 一个网格"
    );
    assert_eq!(
        serde_json::to_vec(&project_after.clip_pool).expect("序列化"),
        pool_before,
        "裁摆放长度⛔ 不得连带改动**片段池**（池里的片段内容与被裁的那条摆放无关；单位：序列化字节）"
    );
    assert_ne!(
        serde_json::to_vec(&project_after).expect("序列化"),
        bytes_before,
        "正对照：工程确实变了（否则上面那些相等只是没动手）"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "一次按键 = **恰好一步**可撤销的编辑"
    );

    // ---- 读数跟上：那一格变宽、**其余每一格逐值等于按键之前抄下的那一张**、选中态留下 ----
    let ulids_after = clip_ulids(&window);
    let widths_after = clip_widths(&window);
    assert_eq!(
        ulids_after, ulids_before,
        "绘制清单必须**逐位同一**（裁剪不销毁对象，也⛔ 不得顺手重排清单）"
    );
    assert_eq!(
        widths_after.len(),
        widths_before.len(),
        "宽度列与清单同长（同一轮投影派生；单位：格）"
    );
    for (index, width_after) in widths_after.iter().enumerate() {
        let width_before = widths_before[index];
        if index == a_index {
            assert!(
                *width_after > width_before,
                "延长后 `{a}` 那一格必须变大（否则画面跟不上权威）：{width_before} → {width_after}"
            );
        } else {
            assert_eq!(
                *width_after, width_before,
                "第 {index} 格（`{}`）的宽度必须**逐值**等于按键之前 —— 一条都不许被顺手裁",
                ulids_after[index]
            );
        }
    }
    assert_eq!(
        window.get_selected_clip_ulid().as_str(),
        a,
        "裁剪不销毁对象 ⇒ 选中态必须**留下**（用户可以连按第二下继续裁）"
    );

    report_line(&format!(
        "[clip-duration] `{a}` 经 `Shift+→` 延长 {GRID_EXPECTED} tick（{} → {}）；`{b}` 逐字段不变；\
         池子字节不变（{} 字节）；提交 +1；宽度 {} → {}；选中态留下",
        placement_a.duration_ticks,
        a_after.duration_ticks,
        pool_before.len(),
        width_a_before,
        widths_after[a_index]
    ));
}

/// ②b 反向：`Shift+←` 把同一条摆放**缩短一个网格**（同一个动作的另一格键）。
#[test]
fn shift_left_shortens_the_selected_placement_by_the_same_grid() {
    let (window, port, _plane) = assemble();
    let a = clip_ulids(&window)[0].clone();
    select_clip(&window, &a);
    let before = placement_by_ulid(&port.project(), &a).expect("A 在场");
    assert!(
        before.duration_ticks >= GRID_EXPECTED,
        "夹具前提：这条摆放得够缩一档（实测 {} tick < {GRID_EXPECTED}）",
        before.duration_ticks
    );
    let widths_before = clip_widths(&window);
    let width_before = widths_before[0];

    assert!(
        press_shift_left(&window),
        "同样必须被消费（`Shift+←` 是裁剪的另一半）"
    );
    let after = placement_by_ulid(&port.project(), &a).expect("`{a}` 还在");
    assert_eq!(
        after.duration_ticks,
        before.duration_ticks - GRID_EXPECTED,
        "`Shift+←` 必须恰好缩短一个网格"
    );
    // 钉的是**读数跟着权威变小**：拿"宽度列里 A 那一格"和"宽度列里 A 那一格"对比是同一份
    // 读数自证（上一稿就是这个形状），implementation 一动不动也绿。
    assert_eq!(
        clip_ulids(&window)[0],
        a,
        "缩短不改变清单顺序 ⇒ 仍是第 0 格（否则下面比的是别的剪辑）"
    );
    let width_after = clip_widths(&window)[0];
    assert!(
        width_after < width_before,
        "`{a}` 那一格必须变窄：{width_before} → {width_after}"
    );
    report_line(&format!(
        "[clip-duration] `{a}` 经 `Shift+←` 缩短 {GRID_EXPECTED} tick（{} → {}）；宽度 {} → {}",
        before.duration_ticks, after.duration_ticks, width_before, width_after
    ));
}

/// ③ **画得出来 ＋ 可逆**：变宽只发生在并集内，延回原值后帧与基准**逐字节相同**。
///
/// 为什么区域是"前后两框的并集"而不是"原来那块剪辑"：裁剪改变的就是右缘往哪停，
/// 新盖住的那几条像素列本来就在旧框**之外**，把它们判成"越界"是判据写错，不是实现错。
#[test]
fn the_resize_is_drawn_within_the_union_of_the_two_bodies_and_leaves_no_trace() {
    let (window, port, mut plane) = assemble();
    let a = clip_ulids(&window)[0].clone();
    select_clip(&window, &a);
    let duration_before = placement_by_ulid(&port.project(), &a)
        .expect("A 在场")
        .duration_ticks;
    let body_id = format!("clip-{a}-body");

    let body_before = element_bounds(&mut plane, 10, &body_id);
    assert!(
        body_before.width > 0 && body_before.height > 0,
        "裁剪面必须有可见几何: {body_before:?}"
    );
    let baseline = plane.capture().expect("Tier-1 截图");

    assert!(press_shift_right(&window), "夹具前提：这一键必须被消费");
    let body_after = element_bounds(&mut plane, 11, &body_id);
    assert!(
        body_after.width > body_before.width,
        "延长后剪辑体的包围盒必须更宽（否则像素判据在空转）: {body_before:?} → {body_after:?}"
    );
    let resized = plane.capture().expect("Tier-1 截图");
    let area = union_rect(body_before, body_after);
    let (inside, outside) = diff_split(&baseline, &resized, area);
    assert!(
        inside > 0,
        "裁剪必须**真的画出来**：实测并集区域 {area:?} 内 0 个差异像素"
    );
    assert_eq!(
        outside, 0,
        "裁剪态的改变像素必须全部落在 `clip-{a}-body` 前后两框的并集内（{area:?}）"
    );

    // ---- `Shift+←` 延回原值 ⇒ 与基准帧逐字节相同（不留痕）----
    assert!(
        press_shift_left(&window),
        "第二下同样被消费（裁剪是来回拨，不是单向标记）"
    );
    assert_eq!(
        placement_by_ulid(&port.project(), &a).map(|placement| placement.duration_ticks),
        Some(duration_before),
        "第二下必须把时长**精确回到按键之前那一个值**（⛔ 不是'差不多' —— 留痕的判据是下面那句逐字节）"
    );
    let restored = plane.capture().expect("Tier-1 截图");
    assert_eq!(
        restored.pixels(),
        baseline.pixels(),
        "延回原值后画面必须与基准**逐字节相同**（裁剪不留痕）"
    );
    assert_eq!(
        element_bounds(&mut plane, 12, &body_id),
        body_before,
        "回到原时长后包围盒也必须回到原样"
    );

    report_line(&format!(
        "[clip-duration] `{a}` 变宽：并集 {area:?} 内 {inside} 个差异像素 / 外 0；延回原值后帧与基准逐字节相同"
    ));
}

/// ④ **优先级与边界**：同时有音符选区 ⇒ 只动音符；音符卡在下限 ⇒ **不跨界**去裁剪辑。
///
/// 这一臂钉的是本票最容易写坏的一处：分派必须看**选区**（`selected-ulids` 是否为空），
/// ⛔ 不能看"这一批 op 是否为空"。后者的形状是"用户指着 1 tick 的音符按 `Shift+←`，
/// 音符没动，剪辑被顺手裁短了" —— 一次按键带走另一个用户**没指着**的对象。
#[test]
fn the_note_scope_wins_and_a_floor_bound_note_never_leaks_into_the_clip() {
    let (window, port, _plane) = assemble();
    let a = clip_ulids(&window)[0].clone();
    select_clip(&window, &a);
    let placement_before = placement_by_ulid(&port.project(), &a).expect("A 在场");

    // ---- 挑一颗音符进选区 ----
    let notes = note_durations(&port.project());
    let (note_id, note_duration) = notes[0].clone();
    select_notes(&window, std::slice::from_ref(&note_id));
    assert_eq!(note_selection_len(&window), 1);
    let commits_before = port.display().commit_count;

    // ---- 两格都有选中态 ⇒ `Shift+→` 只动**音符** ----
    assert!(
        press_shift_right(&window),
        "有音符选区 ⇒ 这一键仍被消费（走音符那一臂）"
    );
    let project_after_note = port.project();
    let durations_after = note_durations(&project_after_note);
    assert_eq!(
        durations_after
            .iter()
            .find(|(id, _)| id == &note_id)
            .map(|(_, duration)| *duration),
        Some(note_duration + GRID_EXPECTED),
        "选中的音符必须被延长一个网格（既有语义一位不动）"
    );
    assert_eq!(
        placement_by_ulid(&project_after_note, &a),
        Some(placement_before),
        "⚠ 音符优先时⛔ 不得同时裁剪辑（两格作用域各自的键）"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "一次按键 = 一步编辑"
    );

    // ---- 把那颗音符压到 1 tick 下限 ----
    let mut presses = 0_usize;
    while note_durations(&port.project())
        .iter()
        .find(|(id, _)| id == &note_id)
        .map(|(_, duration)| *duration)
        != Some(1)
    {
        presses += 1;
        assert!(
            presses <= 40,
            "压到下限的构造在 40 次内没收敛，判据前提不成立"
        );
        assert!(
            press_shift_left(&window),
            "第 {presses} 次 `Shift+←` 仍应有音符可压（选区非空且未到底）"
        );
    }
    let commits_at_floor = port.display().commit_count;
    assert_eq!(
        placement_by_ulid(&port.project(), &a),
        Some(placement_before),
        "夹具前提：一路压音符的过程中剪辑始终没被碰过"
    );

    // ---- 关键一臂：选区**非空**但已在下限 ⇒ op 批为空 ⇒ **不消费**、⛔ 不跨界 ----
    assert!(
        !press_shift_left(&window),
        "音符已在 1 tick 下限 ⇒ 这一键必须**不消费**（⛔ 不是'吞掉键却去裁剪辑'）"
    );
    assert_eq!(
        port.display().commit_count,
        commits_at_floor,
        "⛔ 跨界裁剪辑必然是多出来的一次提交"
    );
    let duration_now =
        placement_by_ulid(&port.project(), &a).map(|placement| placement.duration_ticks);
    assert_eq!(
        placement_by_ulid(&port.project(), &a),
        Some(placement_before),
        "剪辑时长必须一位不动（实测 {duration_now:?}）"
    );

    report_line(&format!(
        "[clip-duration] 同时选中音符 `{note_id}` 与剪辑 `{a}`：`Shift+→` 只动音符（+{GRID_EXPECTED}）；\
         压到 1 tick 后再按 `Shift+←` ⇒ 不消费、0 提交、剪辑一位不动"
    ));
}

/// ⑤ 视图门控：Session 视图里这一键**不消费、不提交**，而**同一份选中态**回编曲视图立刻生效。
#[test]
fn a_selection_left_over_from_the_arrangement_view_is_not_resized_in_session_view() {
    let (window, port, _plane) = assemble();
    let a = clip_ulids(&window)[0].clone();
    select_clip(&window, &a);
    let placement_before = placement_by_ulid(&port.project(), &a).expect("A 在场");
    let commits_before = port.display().commit_count;
    let bytes_before = serde_json::to_vec(&port.project()).expect("序列化");

    // ---- Session 视图（`arrangement-view = false`）：同样的键必须什么都不做 ----
    window.set_arrangement_view(false);
    assert!(
        !press_shift_right(&window),
        "Session 视图里没有'选中的剪辑'这一格 ⇒ 拿陈旧读数改东西必须是**不消费**"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before,
        "Session 视图里那一次按键不得提交任何 op"
    );
    assert_eq!(
        serde_json::to_vec(&port.project()).expect("序列化"),
        bytes_before,
        "Session 视图里那一次按键不得改动工程"
    );
    assert_eq!(
        window.get_selected_clip_ulid().as_str(),
        a,
        "不消费时⛔ 也不得顺手改动选中态（那是另一件事）"
    );

    // ---- 回到编曲视图：**同一份**选中态 ⇒ 同一键真的裁剪 ----
    window.set_arrangement_view(true);
    assert!(
        press_shift_right(&window),
        "正对照：唯一改变的是视图 ⇒ 编曲视图里这一键必须消费并裁剪"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "编曲视图那一次 = 一次提交"
    );
    assert_eq!(
        placement_by_ulid(&port.project(), &a).map(|placement| placement.duration_ticks),
        Some(placement_before.duration_ticks + GRID_EXPECTED),
        "编曲视图那一次必须真的把 `{a}` 裁长一个网格"
    );

    report_line(&format!(
        "[clip-duration] 同一份选中态 `{a}`：Session 视图不消费 / 0 提交 / 工程不变；\
         编曲视图消费 / +1 提交 / 时长 +{GRID_EXPECTED}"
    ));
}

/// ⑥ 解析不到 ⇒ **不消费**（陈旧身份、非法串、空选区三种）。
///
/// 与 `clip_delete_ops_for` / `clip_mute_ops_for` 同一取向："把键吞掉却什么都不做，比不处理更糟"。
#[test]
fn a_stale_or_illegal_clip_ulid_is_not_resized() {
    let (window, port, _plane) = assemble();
    let commits_before = port.display().commit_count;
    let bytes_before = serde_json::to_vec(&port.project()).expect("序列化");

    for stale in [stale_clip_ulid(), "not-a-real-clip".to_owned()] {
        window.set_selected_clip_ulid(stale.clone().into());
        assert!(
            !press_shift_right(&window),
            "选中身份 `{stale}` 解析不到 ⇒ 必须是**不消费**（⛔ 不是'吞掉却什么都不做'）"
        );
        assert!(
            !press_shift_left(&window),
            "缩短方向同样必须**不消费**（`{stale}`）"
        );
        assert_eq!(
            port.display().commit_count,
            commits_before,
            "解析不到时不得提交任何 op（`{stale}`）"
        );
        assert_eq!(
            window.get_selected_clip_ulid().as_str(),
            stale.as_str(),
            "解析不到时⛔ 不得顺手清掉那个陈旧读数（那是另一件事，不归这一键）"
        );
    }

    // ---- 没有任何选中 ⇒ 同样不消费 ----
    window.set_selected_clip_ulid("".into());
    assert!(
        !press_shift_right(&window),
        "什么都没选 ⇒ `Shift+→` 不得被消费（没有可判定的对象，猜一条是数据损失）"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before,
        "无选区时零提交"
    );
    assert_eq!(
        serde_json::to_vec(&port.project()).expect("序列化"),
        bytes_before,
        "上面每一臂都必须逐字节不动工程"
    );

    report_line(&format!(
        "[clip-duration] 陈旧 `{}` / 非法 `not-a-real-clip` / 空选区 三种都**不消费**、0 提交",
        stale_clip_ulid()
    ));
}

/// ⑦ **一步**撤销逐字节放回（载荷自带 `old_duration_ticks`），再 **redo** 又裁过去。
///
/// ⚠ 走的是**人用的那一格键**（`Ctrl+Z` / `Ctrl+Shift+Z`），⛔ 不是 `port.perform(..)`：
/// 端口的 `perform` 是纯模型入口（`undo.rs` 自己写着"端口碰不到 Slint"），它**不会**重投影，
/// 于是拿它撤销之后再读 `clip-widths` 会读到裁剪后的旧值 —— 那是入口的性质，不是缺陷。
/// 键路径经 [`apply_action`] 的 `Action::Undo | Action::Redo` 那一臂，末尾调
/// `refresh_undo_window(ui, port, true)` ⇒ 这一臂才**真的**能断言"画面跟着回退"。
/// 注入写法（`("z", shift, ctrl, alt, meta)`）与 `live_ui_mcp.rs` 里已绿的 `Cmd+Z` 判据逐位一致。
#[test]
fn one_undo_restores_the_duration_byte_for_byte() {
    let (window, port, _plane) = assemble();
    let a = clip_ulids(&window)[0].clone();
    select_clip(&window, &a);
    let before = placement_by_ulid(&port.project(), &a).expect("A 在场");
    let bytes_before = serde_json::to_vec(&port.project()).expect("序列化");
    let widths_at_rest = clip_widths(&window);

    assert!(
        press_shift_right(&window),
        "夹具前提：编曲视图里这一键必须消费"
    );
    assert_ne!(
        serde_json::to_vec(&port.project()).expect("序列化"),
        bytes_before,
        "夹具前提：工程确实被改过"
    );
    let widths_resized = clip_widths(&window);
    assert!(
        widths_resized[0] > widths_at_rest[0],
        "夹具前提：宽度列在裁完之后确实跟上了（{} → {}），否则下面'回退'一句是空话",
        widths_at_rest[0],
        widths_resized[0]
    );

    // ---- 撤销：权威逐字节放回 **且** 宽度列跟着回去 ----
    assert!(
        window.invoke_key_action("z".into(), false, true, false, false),
        "`Cmd/Ctrl+Z` 必须被消费"
    );
    assert_eq!(
        serde_json::to_vec(&port.project()).expect("序列化"),
        bytes_before,
        "**一步**撤销必须将工程逐字节放回裁剪前（载荷里的 `old_duration_ticks` 就是它）"
    );
    assert_eq!(
        placement_by_ulid(&port.project(), &a),
        Some(before),
        "放回后 `{a}` 必须**逐字段**是原样（⛔ 不是'时长对了但身份换了'）"
    );
    assert_eq!(
        clip_widths(&window),
        widths_at_rest,
        "撤销后宽度列必须**逐值回到裁之前那一版**（画面停在裁剪后的样子 = 用户会以为撤销没生效）"
    );

    // ---- redo：再裁过去，且宽度列回到第一次裁完的那一版 ----
    assert!(
        window.invoke_key_action("z".into(), true, true, false, false),
        "`Cmd/Ctrl+Shift+Z` 必须被消费（redo 是同一把键的 shift 变体）"
    );
    assert_eq!(
        placement_by_ulid(&port.project(), &a).map(|placement| placement.duration_ticks),
        Some(before.duration_ticks + GRID_EXPECTED),
        "redo 后 `{a}` 再次被裁长一个网格"
    );
    assert_eq!(
        clip_widths(&window),
        widths_resized,
        "redo 后宽度列必须回到第一次裁完那一版（逐值）"
    );
    assert_eq!(
        window.get_selected_clip_ulid().as_str(),
        a,
        "撤销 / redo 都⛔ 不回滚选中态（它是视图态，不是工程态）"
    );

    report_line(&format!(
        "[clip-duration] 一步撤销逐字节放回（{} → {} → 放回 {}）；`Ctrl+Shift+Z` 再裁过去；宽度列两次都跟上",
        before.duration_ticks,
        before.duration_ticks + GRID_EXPECTED,
        before.duration_ticks
    ));
}

/// ⑧ **纯函数**逐字段对账 ＋ **源码纪律**（构造点恰好一处）。
#[test]
fn clip_duration_ops_for_is_the_only_set_clip_placement_duration_construction_site() {
    let project = yeban_model::samples::filled_project();
    let painted: Vec<String> = placements(&project).into_iter().map(|(id, _)| id).collect();
    let a_ulid = painted[0].clone();
    let a_placement = placement_by_ulid(&project, &a_ulid).expect("A 在场");

    // ---- 延长：op 内容逐字段对账 ----
    let ops = clip_duration_ops_for(&project, &a_ulid, GRID_EXPECTED as i64);
    assert_eq!(ops.len(), 1, "选中一条摆放 + 一档增量 ⇒ 恰好一条 op");
    match &ops[0] {
        yeban_model::ops::Op::SetClipPlacementDuration {
            track_id,
            placement_id,
            old_duration_ticks,
            new_duration_ticks,
        } => {
            assert_eq!(
                *placement_id, a_placement.id,
                "载荷里的摆放身份必须就是选中的那一条"
            );
            assert!(
                project.tracks.contains_key(track_id),
                "载荷里的轨 `{track_id}` 必须在工程里（双定位，⛔ 不按下标）"
            );
            assert_eq!(
                *old_duration_ticks, a_placement.duration_ticks,
                "`old_duration_ticks` 必须**从权威现读**（前置条件靠它挡陈旧载荷）"
            );
            assert_eq!(
                *new_duration_ticks,
                a_placement.duration_ticks + GRID_EXPECTED,
                "`new_duration_ticks` 必须恰好是 `old + 一档网格`"
            );
        }
        other => panic!("只允许 `SetClipPlacementDuration`，实际 {other:?}"),
    }

    // ---- 下限：缩到 1 tick 之后⛔ 不再产出 op（而不是产出 `0`）----
    let mut floor_project = project.clone();
    let mut found_floor = false;
    for track in floor_project.tracks.values_mut() {
        if let Some(placement) = track.clips.get_mut(&a_placement.id) {
            placement.duration_ticks = 1;
            found_floor = true;
        }
    }
    assert!(found_floor, "夹具前提：能把 `{a_ulid}` 预先压到 1 tick");
    assert!(
        clip_duration_ops_for(&floor_project, &a_ulid, -(GRID_EXPECTED as i64)).is_empty(),
        "已在下限 ⇒ 必须是**空批**（⛔ 不是 `new = 0`，那会被模型的 `ZeroDuration` 拒）"
    );
    assert_eq!(
        clip_duration_ops_for(&floor_project, &a_ulid, GRID_EXPECTED as i64)
            .iter()
            .count(),
        1,
        "同一份夹具在**延长**方向仍应产出 op（下限只挡缩短）"
    );

    // ---- 零增量 ⇒ 空批 ----
    assert!(
        clip_duration_ops_for(&project, &a_ulid, 0).is_empty(),
        "`delta == 0` ⇒ 空批（没有变化就不该有提交）"
    );

    // ---- 三种"解析不到"⇒ 空批（⛔ 不动一条猜的对象）----
    let stale = stale_clip_ulid();
    for bogus in ["", "not-a-real-clip", stale.as_str()] {
        assert!(
            clip_duration_ops_for(&project, bogus, GRID_EXPECTED as i64).is_empty(),
            "非法 / 陈旧身份 `{bogus}` 必须解析成空批"
        );
    }

    // ---- 源码纪律：从 `src/host.rs` 现读，⛔ 不靠注释里的数字 ----
    let host_source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/host.rs"))
        .expect("读本 crate 的 host.rs");
    let construction_sites = host_source
        .match_indices("Op::SetClipPlacementDuration {")
        .count();
    assert_eq!(
        construction_sites, 1,
        "`host.rs` 里 `Op::SetClipPlacementDuration` 的构造点必须**恰好一处**（实测 {construction_sites}）"
    );
    let gesture_sites = host_source
        .match_indices("resize_selected_clip_duration(")
        .count();
    assert_eq!(
        gesture_sites, 2,
        "`resize_selected_clip_duration(` 必须只有**定义 ＋ 唯一调用点**两处（实测 {gesture_sites}）"
    );

    report_line(&format!(
        "[clip-duration] `clip_duration_ops_for` 对 `{a_ulid}` 逐字段对账；下限 / 零增量 / 3 种非法输入为空批；\
         `host.rs` 里构造点 1 处、动作函数站点 2 处"
    ));
}

/// ⑨ 投影层前提复核：宽度与身份**同源同序**，改一条时长只翻那一格宽度。
///
/// ② / ③ 都在读 `clip-widths`，它们依赖"按 `clip-ulids` 的位置取宽度不会取错剪辑"。
/// 这条在纯投影层（⛔ 不起窗口）逐条钉住它。
#[test]
fn the_projection_orders_the_width_column_by_the_same_identity_as_the_paint_list() {
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let ulids = view.clip_ulids();
    let widths = view.clip_widths();
    assert_eq!(
        widths.len(),
        ulids.len(),
        "`clip_widths()` 与 `clip_ulids()` 必须同长（同一轮 `clips` 迭代派生）"
    );
    for (index, ulid) in ulids.iter().enumerate() {
        let mut single = project.clone();
        let id = EntityId::from_str(ulid.as_str()).expect("投影出的身份是合法 ULID");
        for track in single.tracks.values_mut() {
            if let Some(placement) = track.clips.get_mut(&id) {
                placement.duration_ticks += GRID_EXPECTED;
            }
        }
        let probe = ViewState::from_project(&single).expect("改完时长仍要能投影");
        let probed = probe.clip_widths();
        assert_eq!(
            probed.len(),
            ulids.len(),
            "第 {index} 条（`{ulid}`）：投影后宽度列长度不变"
        );
        assert!(
            probed[index] > widths[index],
            "第 {index} 条 `{ulid}` 改长之后必须落在同一格上且更宽（{} → {}）",
            widths[index],
            probed[index]
        );
        for (other, width) in probed.iter().enumerate() {
            if other != index {
                assert_eq!(
                    *width, widths[other],
                    "只有第 {index} 条被改：第 {other} 格的宽度必须逐值不变"
                );
            }
        }
    }

    report_line(&format!(
        "[clip-duration] {} 条剪辑逐条验证：`clip_widths()` 与 `clip_ulids()` 同序同长、改一条只宽那一格",
        ulids.len()
    ));
}
