//! ⭐ 任务单 `#192` 的判据：**编辑器级选中轨**（方案 A）。
//!
//! ## 这条判据要证的句子（逐条对应工单 §2）
//!
//! | §2 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 选中可切 | 投影从"兜底第一条"切到 `Some(B)` ⇒ `ViewState::selected_track == B`、设备架标题 / 设备行 / 发送行全部换成 B 的读数 | 证"面板真的换对象"；⛔ 不证人眼看过 |
//! | ② 选中 ⛔ 不是提交 | 点轨道头前后 `commit_count` **Δ0**、`undoable` **Δ0**、`UndoPort::fingerprint()`（工程容器字节的 SHA-256）**逐字相同**、`serde_json` 工程字节**逐字节相同** | 证"选中不进文档 / 不可撤销 / 不进摘要" |
//! | ③ 面板三处 | 设备架第一段（`device_track_label` ＋ `device_names`）／发送条（`send_destination_names`）／轨路由（删除目标身份 ＋ 可用性）**同一次**解析 | 证"三处不会各说一条轨"；⚠ 轨路由四键的**可见文案**与轨无关（`+ MIDI` / `+ 音频` / `+ 辅助返回` / `删除展示轨`），随选中变的是**删除目标** |
//! | ④ 兜底 | ① 未选 ⇒ 第一条；② 选中轨被删 ⇒ 回落；③ 选中轨被撤销掉 ⇒ 回落。三条都**不 panic** | 证"⛔ 不崩、⛔ 不空面板"（原因字段由 availability 给出） |
//! | ⑤ 铅笔跟选中 | 选 B 之后经**真实点击派发器**（`MainWindow::invoke_clicked`）画音符 ⇒ 音符进 **B 的片段**，A 的片段音符数不变 | 证"点击 → 一个 `Op::AddNote` → B 的 clip_pool 条目" |
//! | ⑥ 主总线不变 | 三档选中下 `master_device_label` / `master_devices` / `master` 逐字段相同 | 证"第二段仍固定 master" |
//! | ⑦ 无头可判 | Tier-1 活窗口 ＋ 无障碍默认动作（`track-{i}-header`）触发选中 ⇒ 宿主回调真的执行 | 证"手势有真实落点"；⛔ 不证像素（见文件尾的 1920 帧说明） |
//!
//! ## 反向腿（去掉本片改动 ⇒ 哪条红）
//!
//! - 把 `display_track` 的选中分支删掉（只留兜底）⇒ ① 红（选 B 之后面板仍是 Lead）；
//! - 让 `wire_track_select` 调一次 `commit_ops` ⇒ ② 的 `commit_count` / `unable` / 指纹红；
//! - 把 `wire_track_select` 里的 `set_selected_track_id` 删掉 ⇒ ①⑦ 红（`selected-track-id` 不变量）；
//! - 把铅笔的 `selected` 参数换成 `None` ⇒ ⑤ 红（音符落回 Lead 的片段）；
//! - 把兜底从 `display_track` 里删掉（选中身份查不到就返回 `None`）⇒ ④ 红（面板空掉）；
//! - 让 `apply_view` / `project_with_view_state` 漏掉选中轨 ⇒ ① 红（点完面板不换）。
//!
//! ## 它**不**证什么
//!
//! ⛔ 不证 1920×1080 基准帧（本机是 macOS，`golden/linux/` 的 5 张不参与本机判决）；
//! ⛔ 不证"人耳听到选中轨的声音"（选中态不是音频路由）；⛔ 不证人眼看过高亮 ——
//! 本设计**不画**选中高亮（默认帧零新像素），选中态的可见表现是**面板换对象**。

use std::collections::BTreeMap;
use std::rc::Rc;

use yeban_app::bridge::{
    TrackHeightLayout, ViewState, display_track, send_strip_track, track_delete_ops_on,
};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UiAction, UndoPort, UndoSession};
use yeban_model::ids::EntityId;
use yeban_model::project::{
    ClipContent, ClipPlacement, ClipPoolEntry, LoopConfig, TrackV3, YebanProjectV1,
};
use yeban_model::samples::filled_project;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::inspect::find_by_accessible_id;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

/// 窗口尺寸（与既有 Tier-1 判据同款；非零即合法）。
const WINDOW: Size = Size::new(1440, 900);

/// 一次提交用的时间戳（与其余界面判据同一个值，⛔ 不引入时钟依赖）。
const NOW_MS: u64 = 1_760_000_000_000;

/// `filled_project()` 的三条非主总线轨（身份升序 = 界面下标序）：
/// `0 = Lead`（MIDI，1 台设备，1 条发送）／`1 = Bass`（音频）／`2 = Aux Reverb`。
fn fixture_tracks(project: &YebanProjectV1) -> Vec<TrackV3> {
    project
        .tracks
        .values()
        .filter(|track| track.id != project.master_bus_track_id)
        .cloned()
        .collect()
}

/// 一条带撤销端口的活窗口（接线走生产同一个 `host::wire_track_select`）。
fn live_with_port(project: &YebanProjectV1) -> (LivePort<MainWindow>, Rc<UndoPort>) {
    let view = ViewState::from_project(project).expect("投影");
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据 #192>", "yeban-app", project.clone(), NOW_MS)
            .expect("打开撤销会话"),
    ));
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    host::wire_track_select(live.ui(), &port);
    (live, port)
}

/// 工程字节的**逐字节**读数（与 `UndoPort::fingerprint` 互补：那是容器指纹，
/// 这是工程 AST 的 JSON 字节）—— 用来证明"选中不进工程文档"。
fn project_bytes(project: &YebanProjectV1) -> Vec<u8> {
    serde_json::to_vec(project).expect("工程必须可序列化")
}

// ===========================================================================
// 判据 ①③⑥：三处面板同一次解析（纯投影，不需要窗口）
// ===========================================================================
#[test]
fn the_panels_serve_the_selected_track_not_always_the_first_one() {
    let project = filled_project();
    let tracks = fixture_tracks(&project);
    let lead = tracks[0].id;
    let bass = tracks[1].id;
    let aux = tracks[2].id;
    let layout = TrackHeightLayout::default();

    // ---- 兜底：没有明确选中 ⇒ 身份序第一条非主总线轨（旧行为）----
    let fallback = ViewState::from_project_with_layout(&project, &layout).expect("投影");
    assert_eq!(
        fallback.selected_track.as_deref(),
        Some(lead.to_canonical_string().as_str()),
        "未选 ⇒ selected_track 必须是身份序第一条非主总线轨"
    );
    assert_eq!(fallback.device_track_label, "轨 Lead · 设备链");
    assert_eq!(fallback.device_names(), vec!["Yeban PolySynth".to_owned()]);

    // ---- 选 Bass：设备架第一段 / 发送条 / 轨路由目标同时换 ----
    let bass_id = bass.to_canonical_string();
    let selected_bass = ViewState::from_project_with_zoom_layout_and_selection(
        &project,
        30,
        &layout,
        Some(bass_id.as_str()),
    )
    .expect("投影");
    assert_eq!(
        selected_bass.selected_track.as_deref(),
        Some(bass_id.as_str())
    );
    assert_eq!(selected_bass.device_track_label, "轨 Bass · 设备链");
    assert!(
        selected_bass.device_names().is_empty(),
        "Bass 没有设备 ⇒ 第一段必须空（⛔ 不显示 Lead 的 PolySynth）"
    );
    assert!(
        selected_bass.send_destination_names().is_empty(),
        "Bass 没有 SendToAux 边 ⇒ 发送条必须空（Lead 的那条发送 ⛔ 不能跟过来）"
    );
    let delete_target = display_track(&project, Some(bass_id.as_str())).expect("目标轨");
    let delete_ops = track_delete_ops_on(&project, Some(delete_target)).expect("删 Bass 的一批 Op");
    assert!(
        delete_ops.iter().any(|op| matches!(
            op,
            yeban_model::Op::RemoveTrack { track_id, .. } if *track_id == bass
        )),
        "轨路由的删除目标必须是选中的 Bass"
    );

    // ---- 选 Aux：标题与发送都换，且送出它的名字 ----
    let aux_id = aux.to_canonical_string();
    let selected_aux = ViewState::from_project_with_zoom_layout_and_selection(
        &project,
        30,
        &layout,
        Some(aux_id.as_str()),
    )
    .expect("投影");
    assert_eq!(
        selected_aux.selected_track.as_deref(),
        Some(aux_id.as_str())
    );
    assert_eq!(selected_aux.device_track_label, "轨 Aux Reverb · 设备链");

    // ---- 选 Lead（与兜底同一条）：读数与 fallback 逐字段相同 ----
    let lead_id = lead.to_canonical_string();
    let selected_lead = ViewState::from_project_with_zoom_layout_and_selection(
        &project,
        30,
        &layout,
        Some(lead_id.as_str()),
    )
    .expect("投影");
    assert_eq!(
        selected_lead.selected_track.as_deref(),
        Some(lead_id.as_str())
    );
    assert_eq!(
        selected_lead.device_track_label, fallback.device_track_label,
        "选中第一条轨与兜底是同一条轨 ⇒ 设备架标题逐字相同"
    );
    assert_eq!(selected_lead.device_names(), fallback.device_names());
    assert_eq!(
        selected_lead.send_destination_names(),
        vec!["Aux Reverb".to_owned()],
        "Lead 的那条 SendToAux（→ Aux Reverb）必须回到发送条"
    );

    // ---- ⑥ 主总线第二段：三档选中下逐字段不变 ----
    for view in [&fallback, &selected_bass, &selected_aux] {
        assert_eq!(view.master_device_label, fallback.master_device_label);
        assert_eq!(view.master_devices, fallback.master_devices);
        assert_eq!(
            view.master.as_ref().map(|m| &m.name),
            fallback.master.as_ref().map(|m| &m.name)
        );
        assert!(view.master_device_insert_available);
    }

    println!(
        "[#192 ①③⑥] 兜底 selected_track={}；Bass label={} devices={:?} sends={:?}；\
         Aux label={}；Lead sends={:?}；主总线 label={}（三档相同）",
        fallback.selected_track.as_deref().unwrap_or("<none>"),
        selected_bass.device_track_label,
        selected_bass.device_names(),
        selected_bass.send_destination_names(),
        selected_aux.device_track_label,
        selected_lead.send_destination_names(),
        fallback.master_device_label,
    );
}

// ===========================================================================
// 判据 ④：兜底的三条路径（未选 / 被删 / 被撤销掉）都不崩
// ===========================================================================
#[test]
fn a_missing_selection_falls_back_to_the_first_track_without_panicking() {
    let project = filled_project();
    let tracks = fixture_tracks(&project);
    let lead = tracks[0].id;
    let bass = tracks[1].id;
    let layout = TrackHeightLayout::default();
    let missing = EntityId::new().to_canonical_string();

    // ① 未选 = None；② 空串；③ 主总线身份；④ 一个不存在的身份 —— 四条都必须回落。
    for selected in [
        None,
        Some(""),
        Some(project.master_bus_track_id.to_canonical_string().as_str()),
        Some(missing.as_str()),
    ] {
        let view =
            ViewState::from_project_with_zoom_layout_and_selection(&project, 30, &layout, selected)
                .expect("投影");
        assert_eq!(
            view.selected_track.as_deref(),
            Some(lead.to_canonical_string().as_str()),
            "selected={selected:?} ⇒ 必须回落到身份序第一条（⛔ 主总线、⛔ 空面板）"
        );
        assert_eq!(view.device_track_label, "轨 Lead · 设备链");
    }

    // ⑤ 选中的轨被删掉之后：同一份"选中原文"仍在窗口上，但投影回落且不 panic。
    let bass_text = bass.to_canonical_string();
    let mut deleted = project.clone();
    deleted.tracks.remove(&bass);
    let after_delete = ViewState::from_project_with_zoom_layout_and_selection(
        &deleted,
        30,
        &layout,
        Some(bass_text.as_str()),
    )
    .expect("删轨之后仍必须能投影");
    assert_eq!(
        after_delete.selected_track.as_deref(),
        Some(lead.to_canonical_string().as_str()),
        "选中轨被删 ⇒ 回落（⛔ 不 panic、⛔ 不空面板）"
    );

    println!(
        "[#192 ④] 未选/空串/主总线/不存在身份/被删轨 ⇒ selected_track={}（五条同一出口）",
        after_delete.selected_track.as_deref().unwrap_or("<none>"),
    );
}

// ===========================================================================
// 判据 ④③（端口腿）：删轨 / 撤销掉选中的轨
// ===========================================================================
#[test]
fn deleting_or_undoing_the_selected_track_falls_back_through_the_authoritative_port() {
    let project = filled_project();
    let tracks = fixture_tracks(&project);
    let lead = tracks[0].id;
    let bass = tracks[1].id;
    let (live, port) = live_with_port(&project);
    // 生产同一个删轨写入面（`wire_track_strip` 的 `track-delete` 回调）。
    host::wire_track_strip(live.ui(), &port);

    // 选 Bass（与轨道头点击同一个回调）。
    live.ui().invoke_track_select(1);
    assert_eq!(
        live.ui().get_selected_track_id().to_string(),
        bass.to_canonical_string()
    );

    // 界面上的"删除展示轨"：目标必须跟着**选中轨**走（⛔ 不是永远第一条）。
    let commits_before = port.display().commit_count;
    live.ui().invoke_track_delete();
    assert_eq!(
        port.display().commit_count,
        commits_before + 1,
        "删轨 = 恰好一次提交"
    );
    assert!(
        !port.project().tracks.contains_key(&bass),
        "删掉的必须是选中的 Bass"
    );
    assert!(
        port.project().tracks.contains_key(&lead),
        "Lead ⛔ 不能被删"
    );

    // 宿主的下一次投影（与 `refresh_undo_window` 同一条 `project_with_view_state`）：
    let after_delete = host::project_with_view_state(&port.project(), live.ui()).expect("投影");
    assert_eq!(
        after_delete.selected_track.as_deref(),
        Some(lead.to_canonical_string().as_str()),
        "选中的 Bass 被删 ⇒ 面板回落 Lead"
    );
    host::apply_view(live.ui(), &after_delete, 1440.0, 0.0);
    // 窗口上的**原文**不清空（撤销后同身份回来即可恢复）—— 这是刻意的语义。
    assert_eq!(
        live.ui().get_selected_track_id().to_string(),
        bass.to_canonical_string(),
        "选中态原文保留：⛔ 不在删轨时静默改写用户的选择"
    );

    // 撤销删轨 ⇒ Bass 回来（身份相同）⇒ 选中态**自动恢复**，投影又服务 Bass。
    let outcome = port.perform(UiAction::Undo);
    assert!(outcome.changed(), "撤销必须真的改工程");
    let after_undo = host::project_with_view_state(&port.project(), live.ui()).expect("投影");
    assert_eq!(
        after_undo.selected_track.as_deref(),
        Some(bass.to_canonical_string().as_str()),
        "撤销删轨之后同身份回来 ⇒ 选中态恢复（同一条原文的另一种结果）"
    );

    println!(
        "[#192 ④③] 选 Bass ⇒ 删 ⇒ selected_track={}；撤销删轨 ⇒ selected_track={}；\
         窗口原文={}（保留）",
        after_delete.selected_track.as_deref().unwrap_or("<none>"),
        after_undo.selected_track.as_deref().unwrap_or("<none>"),
        live.ui().get_selected_track_id(),
    );
}

// ===========================================================================
// 判据 ②⑦：点轨道头选中 **⛔ 不是提交**（真实窗口 + 无障碍默认动作）
// ===========================================================================
#[test]
fn clicking_the_track_header_changes_the_panels_and_commits_nothing() {
    let project = filled_project();
    let tracks = fixture_tracks(&project);
    let bass = tracks[1].id;
    let (live, port) = live_with_port(&project);
    let ui = live.ui();

    let before_display = port.display();
    let before_fingerprint = port.fingerprint();
    let before_bytes = project_bytes(&port.project());

    // 真实手势：`track-1-header` 的无障碍默认动作（与 TouchArea 的 pointer down 同一个回调）。
    let handle = find_by_accessible_id(ui, "track-1-header")
        .expect("控件树里必须有 track-1-header（编排视图默认可见）");
    handle.invoke_accessible_default_action();

    assert_eq!(
        ui.get_selected_track_id().to_string(),
        bass.to_canonical_string(),
        "点轨道头 ⇒ 选中态写进窗口属性"
    );
    let view = host::project_with_view_state(&port.project(), ui).expect("投影");
    assert_eq!(
        view.selected_track.as_deref(),
        Some(bass.to_canonical_string().as_str()),
        "选中态进投影"
    );
    assert_eq!(view.device_track_label, "轨 Bass · 设备链");
    assert!(view.device_names().is_empty());

    // ⛔ 不是提交：提交数 / 可撤销步数 / 容器指纹 / 工程字节全部 Δ0。
    let after_display = port.display();
    assert_eq!(
        after_display.commit_count, before_display.commit_count,
        "选中⛔ 不能增加提交"
    );
    assert_eq!(
        after_display.undoable, before_display.undoable,
        "选中⛔ 不能增加撤销步"
    );
    assert_eq!(after_display.undone, before_display.undone);
    assert_eq!(
        port.fingerprint(),
        before_fingerprint,
        "选中⛔ 不能改工程容器指纹"
    );
    assert_eq!(
        project_bytes(&port.project()),
        before_bytes,
        "选中⛔ 不能改工程字节"
    );
    // 模型数组没动：工程的 tracks / clip_pool 逐字节相同（上面一行已经涵盖），
    // 这里再给一个"结构读数"便于报告。
    assert_eq!(port.project().tracks.len(), project.tracks.len());

    // 再点一次同一条轨 ⇒ 一位不动（⛔ 不白重投影）。
    let again = ui.get_selected_track_id().to_string();
    handle.invoke_accessible_default_action();
    assert_eq!(ui.get_selected_track_id().to_string(), again);

    println!(
        "[#192 ②⑦] track-1-header 默认动作 ⇒ selected={} label={}；\
         commit_count {}→{}／undoable {}→{}／fingerprint {:?}→{:?}／工程字节 {}→{}",
        ui.get_selected_track_id(),
        view.device_track_label,
        before_display.commit_count,
        after_display.commit_count,
        before_display.undoable,
        after_display.undoable,
        before_fingerprint,
        port.fingerprint(),
        before_bytes.len(),
        project_bytes(&port.project()).len(),
    );
}

// ===========================================================================
// 判据 ④③（撤销掉"新建的轨"）：选中轨被撤销掉 ⇒ 回落
// ===========================================================================
#[test]
fn undoing_the_creation_of_the_selected_track_falls_back() {
    let project = filled_project();
    let tracks = fixture_tracks(&project);
    let lead = tracks[0].id;
    let (live, port) = live_with_port(&project);
    let ui = live.ui();

    // 新建一条 MIDI 轨（权威端口）。
    let ops = yeban_app::bridge::track_create_ops(&project, yeban_app::bridge::NewTrackKind::Midi)
        .expect("建轨 op");
    let new_id = ops
        .iter()
        .find_map(|op| match op {
            yeban_model::Op::AddTrack { track } => Some(track.id),
            _ => None,
        })
        .expect("建轨批次里必须有 AddTrack");
    assert!(port.commit_ops(NOW_MS + 1, "create track", ops).is_ok());

    // 重新投影 + 注入（与 `refresh_undo_window` 同一条路），然后选中新轨。
    let view = host::project_with_view_state(&port.project(), ui).expect("投影");
    host::apply_view(ui, &view, 1440.0, 0.0);
    {
        use slint::Model as _;
        let index = ui
            .get_track_ids()
            .iter()
            .position(|id| id == new_id.to_canonical_string())
            .expect("新轨必须在 track-ids 里") as i32;
        ui.invoke_track_select(index);
    }
    assert_eq!(
        ui.get_selected_track_id().to_string(),
        new_id.to_canonical_string()
    );
    let selected_view = host::project_with_view_state(&port.project(), ui).expect("投影");
    assert_eq!(
        selected_view.selected_track.as_deref(),
        Some(new_id.to_canonical_string().as_str())
    );

    // 撤销"建轨"⇒ 那条轨不存在了；选中原文保留，但投影回落。
    let outcome = port.perform(UiAction::Undo);
    assert!(outcome.changed(), "撤销建轨必须真的改工程");
    assert!(!port.project().tracks.contains_key(&new_id));
    let after_undo = host::project_with_view_state(&port.project(), ui).expect("投影");
    assert_eq!(
        after_undo.selected_track.as_deref(),
        Some(lead.to_canonical_string().as_str()),
        "选中的轨被撤销掉 ⇒ 回落（⛔ 不 panic、⛔ 不空面板）"
    );
    host::apply_view(ui, &after_undo, 1440.0, 0.0);
    assert_eq!(
        ui.get_selected_track_id().to_string(),
        new_id.to_canonical_string(),
        "窗口原文仍是那条已不存在的身份（撤销回来即可恢复）"
    );

    println!(
        "[#192 ④③] 选中新轨 {} ⇒ 撤销建轨 ⇒ selected_track={}；UI 无 panic（下一帧已注入）",
        new_id.to_canonical_string(),
        after_undo.selected_track.as_deref().unwrap_or("<none>"),
    );
}

// ===========================================================================
// 判据 ⑤：铅笔写到**选中轨**的片段里（真实点击派发器）
// ===========================================================================
#[test]
fn the_pencil_writes_into_the_selected_tracks_clip() {
    // 夹具：给 Bass 加一条**自己的** MIDI 片段（与 Lead 的片段不同身份、不同内容池条目）。
    let mut project = filled_project();
    let tracks = fixture_tracks(&project);
    // Lead 的 MIDI 片段（`filled_project` 里唯一一条带音符的 clip_pool 条目）。
    let lead_clip_id = project
        .clip_pool
        .values()
        .find(|entry| entry.content.notes().is_some_and(|notes| !notes.is_empty()))
        .map(|entry| entry.id)
        .expect("filled_project 必须有一条带音符的 MIDI 片段");
    let bass = tracks[1].id;
    let bass_clip_id = EntityId::new();
    project.clip_pool.insert(
        bass_clip_id,
        ClipPoolEntry {
            id: bass_clip_id,
            name: "Bass Clip".to_owned(),
            content: ClipContent::Midi {
                notes: BTreeMap::new(),
            },
        },
    );
    let placement_id = EntityId::new();
    project
        .tracks
        .get_mut(&bass)
        .expect("Bass 必须在")
        .clips
        .insert(
            placement_id,
            ClipPlacement {
                id: placement_id,
                clip_id: bass_clip_id,
                start_tick: 0,
                duration_ticks: 3840,
                loop_config: LoopConfig::default(),
                muted: false,
            },
        );

    let (live, port) = live_with_port(&project);
    let ui = live.ui();
    host::wire_roll_edit(ui, &port);
    ui.set_active_tool(2); // 2 = 铅笔（`Tool::from_digit`）

    // 选 Bass（索引 1 = 身份序第二条非主总线轨），然后点一下卷帘。
    ui.invoke_track_select(1);
    assert_eq!(
        ui.get_selected_track_id().to_string(),
        bass.to_canonical_string()
    );
    let clicks_before = port.display().commit_count;
    // 宿主帧 `(0.5, 7.0)`：tick 0（1/16 吸附）＋ 第 0 条音高泳道
    // （`lane_at_y` 先减 `NOTE_INSET_Y_PX = 6`，所以 y 必须 ≥ 6）。
    let (px, py) = yeban_app::bridge::host_to_painted(0.5, 7.0);
    ui.invoke_clicked(px, py);

    let after = port.project();
    let bass_notes = after
        .clip_pool
        .get(&bass_clip_id)
        .and_then(|entry| entry.content.notes())
        .expect("Bass 的条目必须还是 MIDI");
    assert_eq!(
        bass_notes.len(),
        1,
        "点击之后 Bass 的片段里必须恰好多 1 个音符"
    );
    assert_eq!(
        after
            .clip_pool
            .get(&bass_clip_id)
            .and_then(|entry| entry.content.notes())
            .map(|notes| notes.values().next().map(|note| note.start_tick)),
        Some(Some(0)),
        "吸附到 tick 0（1/16 网格）"
    );
    assert_eq!(
        after
            .clip_pool
            .get(&lead_clip_id)
            .and_then(|entry| entry.content.notes())
            .map(BTreeMap::len),
        Some(4),
        "Lead 的片段音符数⛔ 不能因为选了 Bass 而变（仍是夹具的 4 个）"
    );
    assert_eq!(
        port.display().commit_count,
        clicks_before + 1,
        "一次铅笔点击 = 恰好一次提交（与选中态无关）"
    );

    println!(
        "[#192 ⑤] 选 Bass ⇒ invoke_clicked ⇒ Bass clip notes={}（tick 0）／Lead clip notes={}（不变）／\
         commit_count {}→{}",
        bass_notes.len(),
        after
            .clip_pool
            .get(&lead_clip_id)
            .and_then(|entry| entry.content.notes())
            .map_or(0, BTreeMap::len),
        clicks_before,
        port.display().commit_count,
    );
}

// ===========================================================================
// 判据 ⑦（源码腿）：手势落在**既有控件**上，且默认帧没有新像素
// ===========================================================================
#[test]
fn the_selection_gesture_reuses_the_existing_track_header() {
    let arrangement = include_str!("../ui/workspace/arrangement_view.slint");
    let shell = include_str!("../ui/app.slint");

    // 手势 = 轨道头**既有**的 TouchArea（pointer down）+ 同一个包头的无障碍默认动作。
    assert!(
        arrangement.contains("root.track-select(track_index);"),
        "轨道头 TouchArea 的按下必须调用 `track-select(track_index)`（复用既有可点控件，⛔ 不加新控件）"
    );
    assert!(
        arrangement.contains("accessible-action-default => { root.track-select(track_index); }"),
        "轨道头的无障碍默认动作必须是同一个回调（无头可判的落点）"
    );
    assert!(
        arrangement.contains(
            "accessible-item-selected: root.selected-track-id == root.track-ids[track_index];"
        ),
        "无障碍选中态必须按**身份**绑定（⛔ 不按下标：删轨 / 撤销会重排下标）"
    );
    // ⛔ 零新像素：选中态**不**绑定任何颜色 / 边框 / 背景 —— 默认帧一位不变。
    for forbidden in [
        "selected-track-id ? Tokens.",
        "selected-track-id ?",
        "track-selected",
        "selection-border",
    ] {
        assert!(
            !arrangement.contains(forbidden),
            "选中态⛔ 不许引入新的可见样式（默认帧会动像素）：命中 `{forbidden}`"
        );
    }
    // 根窗口只转发，⛔ 不自己算"当前轨"。
    assert!(shell.contains("callback track-select(int);"));
    assert!(shell.contains("in-out property <string> selected-track-id: \"\";"));
    assert!(shell.contains("track-select(track_index) => {"));
    assert!(shell.contains("selected-track-id: root.selected-track-id;"));

    println!(
        "[#192 ⑦] arrangement_view.slint 手势 = 既有 track-{{i}}-header（pointer down ＋ a11y 默认动作）；\
         选中态零视觉绑定；app.slint 只转发（scan {}+{} 字节）",
        arrangement.len(),
        shell.len(),
    );
}

// ===========================================================================
// 判据 ③（另一条腿）：同一次解析 —— `display_track` 是唯一实现
// ===========================================================================
#[test]
fn there_is_exactly_one_selection_resolution() {
    let project = filled_project();
    let tracks = fixture_tracks(&project);
    let bass = tracks[1].id;
    let bass_text = bass.to_canonical_string();

    // `display_track` 与各"兜底入口"在主语的解析上必须是同一个答案。
    assert_eq!(
        display_track(&project, Some(bass_text.as_str())).map(|t| t.id),
        Some(bass)
    );
    assert_eq!(
        display_track(&project, None).map(|t| t.id),
        send_strip_track(&project).map(|t| t.id),
        "无选中态时 `display_track` 的兜底必须与 `send_strip_track` 逐条相同"
    );
    // 选中主总线 / 空串 / 不存在的身份 ⇒ 同一个兜底答案。
    let fallback = display_track(&project, None).map(|t| t.id);
    for selected in [
        "",
        project.master_bus_track_id.to_canonical_string().as_str(),
        "01J8Z5Q0R7K3M9X2V4B6N8P1ZZ",
    ] {
        assert_eq!(
            display_track(&project, Some(selected)).map(|t| t.id),
            fallback,
            "selected={selected:?} ⇒ 必须回落到同一个兜底答案"
        );
    }

    println!(
        "[#192 ③] display_track：选 Bass={}；未选兜底={:?}；三个退化输入同一兜底",
        bass_text,
        fallback.map(|id| id.to_canonical_string()),
    );
}
