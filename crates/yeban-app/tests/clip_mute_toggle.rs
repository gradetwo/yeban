//! **剪辑静音**（`Cmd/Ctrl+M` 拨选中摆放的 `ClipPlacement::muted`）—— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 一条剪辑摆在编曲视图里，用户可以点它选中（`D-UI-15` 之前的切片）、可以拖它平移
//! （`clip-{ulid}-body`）、可以删它（`D-UI-15`）—— 但**没法只让它不出声**：
//! `Op::SetClipPlacementMuted` 在模型里早已存在、`yeban-engine::snapshot` 与
//! `yeban-render::{als, logic}` 都**真的**按 `muted` 不发声 / 不导出、MCP 侧
//! `yeban_mcp::domain::notes` 的 `NoteOp::SetClipPlacementMuted` 也早已能提交，
//! 而界面侧在此切片之前**一处都没有**（`grep -rn SetClipPlacementMuted crates/yeban-app/src`
//! 为空）⇒ "轨道能静音、剪辑不能"这一半接线缺口。
//!
//! 本切片把 `Cmd/Ctrl+M` 接成"只拨**选中的那条摆放**、一次按键恰好一步可撤销、结果画在
//! 包头上"的手势。轨静音（裸 `m`，作用域 `selected-track-id`）与剪辑静音（`Cmd/Ctrl+M`，
//! 作用域 `selected-clip-ulid`）是**两格键、两个作用域**，本文件同时钉住它们互不越界。
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（投影一致性） | 装配后 `clip-mutes` 与 `clip-ulids` **同长**且一格 `false` 都没有 ⇒ 默认帧无需重录，且数组是**逐剪辑**一位 |
//! | ②（写权威 ＋ 带读数） | `Cmd/Ctrl+M` ⇒ 只有那条摆放 `muted` 翻面（另一条**逐字段**不变）、片段池字节不变、提交 **+1**、选中态**留下**、`clip-mutes` 那一格跟着翻 |
//! | ③（画在包头上） | 静音的改变像素**全部**落在 `clip-{A}-header` 内（>0）；再按一次翻回 ⇒ 帧与基准**逐字节相同**（没有留痕） |
//! | ④（作用域不越界） | 同时选中轨与剪辑：裸 `m` 只翻**轨**的静音、剪辑一位不动；`Cmd/Ctrl+M` 只翻**剪辑**、轨不再动 —— 反向会带走用户正指着的东西 |
//! | ⑤（视图门控） | Session 视图里同一份选中态**不消费、0 提交、工程不变**；回编曲视图立刻翻面 —— 两臂只差 `arrangement-view` |
//! | ⑥（解析不到 ⇒ 不消费） | 陈旧 / 非法 / 空 三种选中态都 `consumed == false`、0 提交、⛔ 不动选中态 |
//! | ⑦（一步撤销 ＋ redo） | `Undo` 将工程逐字节放回（载荷带 `old_muted`），`Redo` 再翻过去 |
//! | ⑧（纯函数 ＋ 源码纪律） | `clip_mute_ops_for` 的 op 内容逐字段对账（含**已静音**的输入 ⇒ 翻回假）；`host.rs` 里 `Op::SetClipPlacementMuted` 的构造点**恰好一处** |
//! | ⑨（同序同长） | 投影层逐条验证 `clip_mutes()` 与 `clip_ulids()` **同源同序** ⇒ ② 里"按 `clip-ulids` 的位置取那一格"不会指错剪辑 |
//!
//! ## 注入分两格（本文件的边界）
//!
//! * **键**：`Cmd/Ctrl+M` 与裸 `m` 都经 `MainWindow::invoke_key_action` —— 那正是 `.slint` 的
//!   `key-handler` 派发快捷键时调的**同一个**回调，返回值就是"消费 / 不消费"。
//!   ⚠ 它⛔ 不是 `WindowEvent`：端口那侧的修饰键注入由 `tests/d_ui_3_modifier_chords.rs`
//!   那条链覆盖，本文件不重复证明"鼠标 / 键盘事件源 ⇒ 属性"，只证明"属性 ⇒ 权威 ⇒ 画面"；
//! * **选中**：走 `.slint` 包头 `clicked` 调的那一个 `clip-select` 回调（宿主自己校验身份）。
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
use yeban_app::host::clip_mute_ops_for;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UiAction, UndoPort, UndoSession};
use yeban_model::{ClipPlacement, EntityId, YebanProjectV1};
use yeban_ui_mcp::live::request_line;
use yeban_ui_mcp::methods::METHOD_NODE;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;
use yeban_ui_test_port::{Rect, Rgb8Image};

/// 提交时间戳（固定值 ⇒ 判据确定性）。
const NOW_MS: u64 = 1_700_000_000_000;

/// 剪辑静音键的**字面**（逻辑键 `Character('m')`，与 `.slint` 报来的 `event.text` 同形）。
const M_TEXT: &str = "m";

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
        UndoSession::open("<clip-mute>", "yeban-app", project.clone(), NOW_MS).expect("打开"),
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

/// 界面读到的**静音列**（`clip-mutes` —— 与 `clip-ulids` 同索引集，事实源是工程字段）。
fn clip_mutes(window: &MainWindow) -> Vec<bool> {
    // `ModelRc<bool>::iter()` 逐格给**拥有**的 `bool`（与 `tests/device_rack_projection.rs`
    // 读 `device-bypassed` 同一写法）。
    window.get_clip_mutes().iter().collect()
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

/// 按 `clip-ulids` 里的位置取那一格静音读数（身份 ⇒ 下标，⛔ 不直接按下标定身份）。
fn muted_at(window: &MainWindow, ulid: &str) -> Option<bool> {
    clip_ulids(window)
        .iter()
        .position(|id| id.as_str() == ulid)
        .map(|index| clip_mutes(window)[index])
}

/// 按身份取一条轨的静音态（轨与剪辑在同一个工程里，读法同款）。
fn track_mute(project: &YebanProjectV1, track_ulid: &str) -> Option<bool> {
    let id = EntityId::from_str(track_ulid).ok()?;
    project.tracks.get(&id).map(|track| track.mute)
}

/// 注入一次 `Cmd/Ctrl+M`（`text, shift, control, alt, meta` 的顺序与 `key-action` 回调逐位对齐）。
fn press_cmd_m(window: &MainWindow) -> bool {
    window.invoke_key_action(M_TEXT.into(), false, true, false, false)
}

/// 注入一次裸 `m`（轨静音那一格）。
fn press_bare_m(window: &MainWindow) -> bool {
    window.invoke_key_action(M_TEXT.into(), false, false, false, false)
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

/// 两帧的差异像素数，按"是否落在 `rect` 内"分成 `(矩形内, 矩形外)`。
fn diff_split(before: &Rgb8Image, after: &Rgb8Image, rect: Rect) -> (u64, u64) {
    assert_eq!(
        (before.width(), before.height()),
        (after.width(), after.height()),
        "两帧必须同尺寸"
    );
    let (x0, y0) = (i64::from(rect.x), i64::from(rect.y));
    let (x1, y1) = (x0 + i64::from(rect.width), y0 + i64::from(rect.height));
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

/// ① 投影一致性 ＋ "默认未静音"这一**前提**。
///
/// 这条本身不是功能，而是其余判据的地基：`clip-mutes` 若与 `clip-ulids` 不同长，下面所有
/// "按 `clip-ulids` 的位置取那一格"的读法都会越界；而起点若有**任何**一格已经是 `true`，
/// ③ 的"默认帧无需重录"就是空话。
#[test]
fn the_fixture_starts_unmuted_and_the_ui_carries_one_reading_per_clip() {
    let (window, port, _plane) = assemble();
    let painted = clip_ulids(&window);
    let mutes = clip_mutes(&window);
    assert!(
        painted.len() >= 2,
        "演示工程必须至少摆放两条剪辑才能判'只翻那一格': {painted:?}"
    );
    assert_eq!(
        mutes.len(),
        painted.len(),
        "`clip-mutes` 与 `clip-ulids` 必须**同长**（同索引集 ⇒ 界面按位置取读数不会错位）"
    );
    assert!(
        !mutes.contains(&true),
        "起点：整列都未静音（否则默认帧的'不变'无从谈起）: {mutes:?}"
    );
    let project = port.project();
    let authoritative = placements(&project);
    assert_eq!(
        authoritative.len(),
        painted.len(),
        "绘制清单条数 == 权威摆放条数（投影没漏、没造）"
    );
    assert!(
        !authoritative.iter().any(|(_, p)| p.muted),
        "起点：权威工程里也没有任何一条摆放是静音的"
    );
    report_line(&format!(
        "[clip-mute] 夹具：{} 条摆放、`clip-mutes` 同长且全 `false`（默认帧前提成立）",
        painted.len()
    ));
}

/// ② **主判据**：`Cmd/Ctrl+M` 只翻选中那条摆放的 `muted`，一次按键 = 恰好一次提交。
///
/// 这一臂同时钉住"写权威"与"把结果送回界面"两个方向：只断言工程会放过
/// "提交完不重投影"（模型改了、包头还亮着）这一类坏 —— 所以 `clip-mutes` 那一格必须一起断。
#[test]
fn cmd_m_on_a_selected_clip_placement_mutes_exactly_that_placement() {
    let (window, port, _plane) = assemble();
    assert!(
        window.get_arrangement_view(),
        "夹具前提：装配后默认是编曲视图（本判据的视图门控靠它）"
    );

    let painted = clip_ulids(&window);
    let (a, b) = (painted[0].clone(), painted[1].clone());
    select_clip(&window, &a);

    let project_before = port.project();
    let bytes_before = serde_json::to_vec(&project_before).expect("序列化基线");
    let pool_before = serde_json::to_vec(&project_before.clip_pool).expect("序列化池子");
    let placement_b = placement_by_ulid(&project_before, &b).expect("B 在场");
    let commits_before = port.display().commit_count;

    assert!(
        press_cmd_m(&window),
        "编曲视图里选中了剪辑 ⇒ `Cmd/Ctrl+M` 必须被消费"
    );

    let project_after = port.project();
    let a_after = placement_by_ulid(&project_after, &a).expect("`{a}` 还在（静音≠删除）");
    assert!(
        a_after.muted,
        "`Cmd/Ctrl+M` 必须把选中的摆放 `{a}` 拨成静音"
    );
    assert_eq!(
        placement_by_ulid(&project_after, &b),
        Some(placement_b),
        "另一条摆放 `{b}` 必须**逐字段**不变（含它的 `muted` 一位都不许动）"
    );
    let muted_after: Vec<&String> = placements(&project_after)
        .iter()
        .filter(|(_, placement)| placement.muted)
        .map(|(id, _)| id)
        .collect();
    assert_eq!(
        muted_after,
        vec![&a],
        "只能翻那**一条**摆放（其余剪辑一格都不许被顺手静音）"
    );
    assert_eq!(
        serde_json::to_vec(&project_after.clip_pool).expect("序列化"),
        pool_before,
        "拨静音⛔ 不得连带改动片段池（单位：序列化字节）"
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

    // ---- 视图读数跟着走：那一格翻面、其余仍 `false`、选中态**留下**（可连按第二下）----
    assert_eq!(
        muted_at(&window, &a),
        Some(true),
        "重投影后 `clip-mutes` 里 `{a}` 那一格必须为真（否则画面跟不上权威）"
    );
    assert_eq!(muted_at(&window, &b), Some(false), "`{b}` 那一格必须仍为假");
    assert_eq!(
        window.get_selected_clip_ulid().as_str(),
        a,
        "静音不销毁对象 ⇒ 选中态必须**留下**（与 `Delete` 那臂相反：删了才不可能还选中）"
    );

    report_line(&format!(
        "[clip-mute] `{a}` 经 `Cmd/Ctrl+M` 翻成静音；`{b}` 逐字段不变；池子字节不变（{} 字节）；\
         提交 +1；静音列 = {}；选中态留下",
        pool_before.len(),
        clip_mutes(&window)
            .iter()
            .map(|v| (if *v { '1' } else { '0' }))
            .collect::<String>()
    ));
}

/// ③ **画在包头上**：改变像素全部落在 `clip-{A}-header` 内，再按一次翻回后与基准逐字节相同。
///
/// 这一臂把 ② 的"读数跟上"再往前推一格：属性写了不等于用户看得见。而"翻回来帧逐字节
/// 相同"钉的是**开关**语义（⛔ 不是单向的"打上静音标记"—— 那会留下擦不掉的痕迹）。
#[test]
fn the_muted_state_is_drawn_on_that_clips_header_and_only_there() {
    let (window, port, mut plane) = assemble();
    let painted = clip_ulids(&window);
    let a = painted[0].clone();
    select_clip(&window, &a);

    let header = element_bounds(&mut plane, 10, &format!("clip-{a}-header"));
    assert!(
        header.width > 0 && header.height > 0,
        "包头必须有可见几何: {header:?}"
    );
    let baseline = plane.capture().expect("Tier-1 截图");

    assert!(press_cmd_m(&window), "夹具前提：这一键必须被消费");
    let muted_frame = plane.capture().expect("Tier-1 截图");
    let (inside, outside) = diff_split(&baseline, &muted_frame, header);
    assert!(
        inside > 0,
        "静音必须**真的画出来**（包头底色沉下去）：实测包头内 0 个差异像素，header={header:?}"
    );
    assert_eq!(
        outside, 0,
        "静音态的改变像素必须全部落在 `clip-{a}-header` 内: {header:?}"
    );

    // ---- 再按一次 = 翻回来 ⇒ 与基准帧逐字节相同 ----
    assert!(press_cmd_m(&window), "第二次同样被消费（开关不是单向标记）");
    assert_eq!(muted_at(&window, &a), Some(false), "`{a}` 必须翻回未静音");
    let restored = plane.capture().expect("Tier-1 截图");
    assert_eq!(
        restored.pixels(),
        baseline.pixels(),
        "翻回未静音后画面必须与基准**逐字节相同**（静音标记不留痕）"
    );
    // ⛔ 不重复断提交数（② 那臂已逐次钉住）；这里只复核**权威**与画面成对回到起点。
    assert_eq!(
        placement_by_ulid(&port.project(), &a).map(|placement| placement.muted),
        Some(false),
        "第二次按键后权威工程里 `{a}` 必须真的翻回未静音（⛔ 不只是界面读数假称）"
    );

    report_line(&format!(
        "[clip-mute] `{a}` 包头内 {inside} 个差异像素 / 矩形外 0；再按一次翻回，帧与基准逐字节相同"
    ));
}

/// ④ **作用域不越界**：裸 `m` 只动轨、`Cmd/Ctrl+M` 只动剪辑。
///
/// 这条是"反过来会怎样"的防线：两把键共享同一个字符 `m`，若分叉成"任意一把都翻两者"，
/// 用户想静音一条剪辑会顺手改掉整条轨的出声状态（或反之），而那**看起来**只是"按了一下 M"。
#[test]
fn bare_m_still_toggles_the_track_and_never_the_clip() {
    let (window, port, _plane) = assemble();
    let painted = clip_ulids(&window);
    let a = painted[0].clone();
    select_clip(&window, &a);
    let lead_id = window
        .get_track_ids()
        .iter()
        .map(|id| id.to_string())
        .next()
        .expect("夹具前提：工程里有轨");
    window.invoke_track_select(0);
    assert_eq!(
        window.get_selected_track_id().to_string(),
        lead_id,
        "夹具前提：裸 `m` 那一臂要有**选中轨**才有对象可翻"
    );
    let project_start = port.project();
    // 不假设夹具的轨静音起点（那是夹具的事，本臂要证的是**分叉**）：读现值，断它的反面。
    let track_mute_before = track_mute(&project_start, &lead_id).expect("夹具前提：选中轨在场");
    assert_eq!(muted_at(&window, &a), Some(false));
    let commits_before = port.display().commit_count;

    // ---- 裸 `m`：只翻轨 ----
    assert!(
        press_bare_m(&window),
        "有选中轨 ⇒ 裸 `m` 必须被消费（既有语义一位不动）"
    );
    let after_bare = port.project();
    assert_eq!(
        track_mute(&after_bare, &lead_id),
        Some(!track_mute_before),
        "裸 `m` 必须翻**轨**的静音（`ToggleTrackMute` 的既有语义：起点取反）"
    );
    assert_eq!(
        placement_by_ulid(&after_bare, &a).map(|p| p.muted),
        Some(false),
        "⚠ 裸 `m` ⛔ 不得顺手把选中的剪辑也静音"
    );
    assert_eq!(
        muted_at(&window, &a),
        Some(false),
        "剪辑的静音读数必须仍为假（画面不能自己猜一个）"
    );

    // ---- `Cmd/Ctrl+M`：只翻剪辑，轨不再动 ----
    assert!(press_cmd_m(&window), "有选中剪辑 ⇒ `Cmd/Ctrl+M` 必须被消费");
    let after_chord = port.project();
    assert_eq!(
        placement_by_ulid(&after_chord, &a).map(|p| p.muted),
        Some(true),
        "`Cmd/Ctrl+M` 必须翻**剪辑**的静音"
    );
    assert_eq!(
        track_mute(&after_chord, &lead_id),
        Some(!track_mute_before),
        "⚠ `Cmd/Ctrl+M` ⛔ 不得再把轨翻回去（两格键各管一个作用域）"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before + 2,
        "两次按键 = 两步可撤销的编辑（各一步，⛔ 不是一步改两处）"
    );

    report_line(&format!(
        "[clip-mute] 同时选中轨 `{lead_id}` 与剪辑 `{a}`：裸 `m` 只翻轨、`Cmd/Ctrl+M` 只翻剪辑；提交 2 步"
    ));
}

/// ⑤ 视图门控：Session 视图里那一键**不消费、不提交**，而**同一份选中态**回编曲视图立刻翻面。
#[test]
fn a_selection_left_over_from_the_arrangement_view_is_not_muted_in_session_view() {
    let (window, port, _plane) = assemble();
    let painted = clip_ulids(&window);
    let a = painted[0].clone();
    select_clip(&window, &a);
    let commits_before = port.display().commit_count;
    let bytes_before = serde_json::to_vec(&port.project()).expect("序列化");

    // ---- Session 视图（`arrangement-view = false`）：同样的键必须什么都不做 ----
    window.set_arrangement_view(false);
    assert!(
        !press_cmd_m(&window),
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

    // ---- 回到编曲视图：**同一份**选中态 ⇒ 同一键真的翻面 ----
    window.set_arrangement_view(true);
    assert!(
        press_cmd_m(&window),
        "正对照：唯一改变的是视图 ⇒ 编曲视图里这一键必须消费并翻面"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "编曲视图那一次 = 一次提交"
    );
    assert_eq!(
        placement_by_ulid(&port.project(), &a).map(|p| p.muted),
        Some(true),
        "编曲视图那一次必须真的把 `{a}` 拨成静音"
    );

    report_line(&format!(
        "[clip-mute] 同一份选中态 `{a}`：Session 视图不消费 / 0 提交 / 工程不变；\
         编曲视图消费 / +1 提交 / 那一格翻面"
    ));
}

/// ⑥ 解析不到 ⇒ **不消费**（陈旧身份、非法串、空选区三种）。
///
/// 与 `clip_delete_ops_for` / `delete_ops_for` 同一取向："把键吞掉却什么都不做，比不处理更糟"。
#[test]
fn a_stale_or_illegal_clip_ulid_is_not_consumed() {
    let (window, port, _plane) = assemble();
    let commits_before = port.display().commit_count;
    let bytes_before = serde_json::to_vec(&port.project()).expect("序列化");

    for stale in [stale_clip_ulid(), "not-a-real-clip".to_owned()] {
        window.set_selected_clip_ulid(stale.clone().into());
        assert!(
            !press_cmd_m(&window),
            "选中身份 `{stale}` 解析不到 ⇒ 必须是**不消费**（⛔ 不是'吞掉却什么都不做'）"
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
        !press_cmd_m(&window),
        "什么都没选 ⇒ `Cmd/Ctrl+M` 不得被消费（没有可判定的对象，猜一个是数据损失）"
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
        "[clip-mute] 陈旧 `{}` / 非法 `not-a-real-clip` / 空选区 三种都**不消费**、0 提交",
        stale_clip_ulid()
    ));
}

/// ⑦ **一步**撤销逐字节放回（载荷自带 `old_muted`），再 **redo** 又翻过去。
#[test]
fn one_undo_unmutes_the_placement_byte_for_byte() {
    let (window, port, _plane) = assemble();
    let painted = clip_ulids(&window);
    let a = painted[0].clone();
    select_clip(&window, &a);
    let bytes_before = serde_json::to_vec(&port.project()).expect("序列化");

    assert!(press_cmd_m(&window), "夹具前提：编曲视图里这一键必须消费");
    assert_ne!(
        serde_json::to_vec(&port.project()).expect("序列化"),
        bytes_before,
        "夹具前提：工程确实被改过"
    );

    let undone = port.perform(UiAction::Undo);
    assert!(undone.changed(), "撤销必须真的改了工程");
    assert_eq!(
        serde_json::to_vec(&port.project()).expect("序列化"),
        bytes_before,
        "**一步**撤销必须将工程逐字节放回翻面前（载荷里的 `old_muted` 就是它）"
    );
    assert_eq!(
        placement_by_ulid(&port.project(), &a).map(|p| p.muted),
        Some(false),
        "放回后 `{a}` 必须未静音"
    );
    assert_eq!(
        muted_at(&window, &a),
        Some(false),
        "重投影后静音列那格也必须翻回来（画面跟得上权威）"
    );

    let redone = port.perform(UiAction::Redo);
    assert!(redone.changed(), "redo 必须真的改了工程");
    assert_eq!(
        placement_by_ulid(&port.project(), &a).map(|p| p.muted),
        Some(true),
        "redo 后 `{a}` 再次静音"
    );
    assert_eq!(
        window.get_selected_clip_ulid().as_str(),
        a,
        "撤销 / redo 都⛔ 不回滚选中态（它是视图态，`selected-clip-ulid` 不进撤销栈）"
    );

    report_line(&format!(
        "[clip-mute] `{a}`：静音 → 一步撤销逐字节放回（{} 字节工程相同）→ redo 再翻过去",
        bytes_before.len()
    ));
}

/// ⑧ 纯函数臂 ＋ **源码纪律**（`clip_mute_ops_for` 是全 app 唯一的 `SetClipPlacementMuted` 构造点）。
///
/// 纯函数部分不建窗口（投影层判据同款）：喂真实工程与三种非法输入，逐条对账 op 内容。
/// 源码纪律部分钉的是"写入面只有一处"：日后若长出第二个构造点，两处就可能对"翻的是哪条摆放"
/// 给出不同答案 —— 那正是本文件要防的分叉形状。
#[test]
fn clip_mute_ops_for_is_the_only_set_clip_placement_muted_construction_site() {
    let project = yeban_model::samples::filled_project();
    let placed = placements(&project);
    let (a_ulid, a_placement) = placed[0].clone();
    let track_of_a = project
        .tracks
        .iter()
        .find(|(_, track)| track.clips.contains_key(&a_placement.id))
        .map(|(track_id, _)| *track_id)
        .expect("摆放必然住在某条轨上");

    let ops = clip_mute_ops_for(&project, &a_ulid);
    assert_eq!(ops.len(), 1, "一条选中剪辑 ⇒ **恰好一条** op");
    match &ops[0] {
        yeban_model::ops::Op::SetClipPlacementMuted {
            track_id,
            placement_id,
            old_muted,
            new_muted,
        } => {
            assert_eq!(*track_id, track_of_a, "op 必须指向摆放**所在**的那条轨");
            assert_eq!(
                *placement_id, a_placement.id,
                "op 的目标身份必须是被选中的那条"
            );
            assert_eq!(
                *old_muted, a_placement.muted,
                "`old_muted` 必须从**当前文档**读（模型的前置条件：与现值不等 ⇒ `OpStateMismatch`）"
            );
            assert_eq!(
                *new_muted, !a_placement.muted,
                "`new_muted` 必须是它的反面（拨开关，⛔ 不是恒置真）"
            );
        }
        other => panic!("只允许 `SetClipPlacementMuted`，实际 {other:?}"),
    }

    // ---- **已静音**的那条 ⇒ op 必须把它翻回**假**（证明这是开关、且 `old_muted` 从文档现读，
    //      ⛔ 不是恒置真的单向标记）----
    let mut muted_project = project.clone();
    let mut found = false;
    for track in muted_project.tracks.values_mut() {
        if let Some(placement) = track.clips.get_mut(&a_placement.id) {
            placement.muted = true;
            found = true;
        }
    }
    assert!(found, "夹具前提：能把 `{a_ulid}` 预先置成静音");
    let unmute_ops = clip_mute_ops_for(&muted_project, &a_ulid);
    assert_eq!(unmute_ops.len(), 1, "已静音的一条 ⇒ 同样恰好一条 op");
    match &unmute_ops[0] {
        yeban_model::ops::Op::SetClipPlacementMuted {
            old_muted,
            new_muted,
            placement_id,
            ..
        } => {
            assert_eq!(*placement_id, a_placement.id);
            assert!(
                *old_muted && !*new_muted,
                "已静音的输入必须给出 `old=true / new=false`（实测 old={old_muted} / new={new_muted}）"
            );
        }
        other => panic!("只允许 `SetClipPlacementMuted`，实际 {other:?}"),
    }

    // 三种"解析不到"⇒ 空批（⛔ 不动一个猜的对象）。
    let stale = stale_clip_ulid();
    for bogus in ["", "not-a-real-clip", stale.as_str()] {
        assert!(
            clip_mute_ops_for(&project, bogus).is_empty(),
            "非法 / 陈旧身份 `{bogus}` 必须解析成空批"
        );
    }

    // ---- 源码纪律：从 `src/host.rs` 现读，⛔ 不靠注释里的数字 ----
    let host_source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/host.rs"))
        .expect("读本 crate 的 host.rs");
    let construction_sites = host_source
        .match_indices("Op::SetClipPlacementMuted {")
        .count();
    assert_eq!(
        construction_sites, 1,
        "`host.rs` 里 `Op::SetClipPlacementMuted` 的构造点必须**恰好一处**（实测 {construction_sites}）"
    );
    let toggle_sites = host_source
        .match_indices("toggle_selected_clip_mute(")
        .count();
    assert_eq!(
        toggle_sites, 2,
        "`toggle_selected_clip_mute(` 必须只有**定义 ＋ 唯一调用点**两处（实测 {toggle_sites}）"
    );

    report_line(&format!(
        "[clip-mute] `clip_mute_ops_for` 对 `{a_ulid}` 给出 1 条 op（`old/new` 逐字段对账）；\
         3 种非法输入为空批；`host.rs` 里构造点 1 处、动作函数站点 2 处"
    ));
}

/// ⑨ 投影层前提复核：静音列与绘制清单**同源同序**（否则 ② 的"按位置取那一格"会指错剪辑）。
#[test]
fn the_projection_orders_the_mute_column_by_the_same_identity_as_the_paint_list() {
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("工程必须能投影");
    let ulids = view.clip_ulids();
    let mutes = view.clip_mutes();
    assert_eq!(
        mutes.len(),
        ulids.len(),
        "`clip_mutes()` 与 `clip_ulids()` 必须同长（同一轮 `clips` 迭代派生）"
    );
    // 逐条：把这一条摆放改成静音再投影，**只有**它那一格为真。
    for (index, ulid) in ulids.iter().enumerate() {
        let mut single = project.clone();
        let id = EntityId::from_str(ulid.as_str()).expect("投影出的身份是合法 ULID");
        for track in single.tracks.values_mut() {
            if let Some(placement) = track.clips.get_mut(&id) {
                placement.muted = true;
            }
        }
        let probe = ViewState::from_project(&single).expect("改完静音仍要能投影");
        let column = probe.clip_mutes();
        assert_eq!(
            column.len(),
            ulids.len(),
            "第 {index} 条（`{ulid}`）：投影后静音列长度不变"
        );
        assert!(column[index], "第 {index} 条 `{ulid}` 必须落在同一格上");
        assert!(
            !column
                .iter()
                .enumerate()
                .any(|(other, value)| other != index && *value),
            "只有第 {index} 条被改：其余 {} 格必须全为假",
            ulids.len() - 1
        );
    }

    report_line(&format!(
        "[clip-mute] {} 条剪辑逐条验证：`clip_mutes()` 与 `clip_ulids()` 同序同长、改一条只翻那一格",
        ulids.len()
    ));
}
