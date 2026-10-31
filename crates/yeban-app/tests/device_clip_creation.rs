//! ⭐ 任务单 `#163` 的判据：**装设备 / 删设备 / 建空的 MIDI 片段**。
//!
//! ## 这条判据要证的句子（逐条对应工单 §2）
//!
//! | §2 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 装 EQ = Δ1 | `invoke_device_insert_eq()` ⇒ `commit_count +1`、`undoable +1`；读回设备数 +1、`kind == InternalEffect`、`params` 非空、**含 `eq_low_gain`**（值 = 契约缺省值 0.0） | 证"点击 → 一个 `Op::InsertDevice` → 一步撤销" |
//! | ② 装完 `#157` 的滑块可用 | 同一个活窗口：`device_insert_available` 前 `true`；装完 `eq_available` 由 `false` → `true`；`invoke_device_eq_grab(device-eq-device-id, 1.0)`（⭐ `#182` 按身份）＋ `invoke_device_eq_release()` ⇒ `commit_count +1`，读回值 = 契约上界 18.0 | 证"装完设备，那条滑块立刻可用"（⛔ 只证工程值变了，不证能听见） |
//! | ③ 装混响 = Δ1 | 同上；读回 `name == "Reverb"`、参数是 `reverb_*`（含 `reverb_wet` 0.25） | 证"同一形状能装第二种效果器" |
//! | ④ 删设备 = Δ1 | `invoke_device_remove(身份)` ⇒ `commit_count +1`；读回设备数 −1、身份消失 | 证"每台一个 ✕，且按**身份**寻址" |
//! | ⑤ 建 clip = Δ1 | `invoke_clip_create()` ⇒ `commit_count +1`；`clip_pool` +1、展示轨 `clips` +1、placement `start_tick=0` / `duration_ticks=15360`（4/4 的 4 小节）、文档 `validate() Ok` | 证"一个原子批次 = 一步撤销"；⛔ 不证钢琴卷帘里画出来 |
//! | ⑥ 空态 / 禁用 | 只有主总线的工程 ⇒ 两个可用性 `false`、原因**逐字**等于 `bridge` 的常量；直接调回调也 Δ0 | 证"不静默禁用、也不靠界面拦" |
//! | ⑦ 完整一首的头四步 | `default_project()`（只有主总线）⇒ `+ MIDI`（Δ1）⇒ 装 EQ（Δ1）⇒ 建 clip（Δ1），文档 `validate() Ok` | 证"新建工程 → 加轨 → 装设备 → 建 clip"这条链真的通 |
//!
//! 预算三件套（§2 ⑦）不在本文件：读数在 `tests/perf_draw_budget.rs`（写点 / 数组常量与原因同处）。
//!
//! ## 反向腿（去掉本片改动 ⇒ 哪条红）
//!
//! - 摘掉 `host::wire_device_rack` 里装设备那一段的 `commit_ops` ⇒ ①③ 的 `commit_count` 差 0；
//! - 摘掉 `host::wire_clip_create` 的 `commit_ops` ⇒ ⑤ 的 `commit_count` 差 0；
//! - 把 `clip_create_ops` 里的 `AddClipPlacement` 删掉 ⇒ ⑤ 的"展示轨 `clips` +1"红；
//! - 把 `device_remove_ops` 的按身份寻址改成按界面行下标 ⇒ ④ 在"身份消失"那条断言上仍绿、
//!   但 `device_remove_ops` 的单测（身份不存在 ⇒ 拒绝）会红；
//! - 把 `builtin_effect_device` 的 `params` 写成空 `Vec` ⇒ ①③ 的"params 非空"红。
//!
//! ## 它**不**证什么（见交付报告的"能验与不能验"）
//!
//! ⛔ 不证"装了 EQ / 混响就能听见"：本判据只证工程文档真的变了。实时侧的插入链是
//! `yeban-engine` 的事；⛔ 不证钢琴卷帘里能看见新片段（那是投影的另一族判据），
//! ⛔ 不证墙钟成本（要 GPU 档，本机纪律不允许）。

use std::collections::BTreeSet;
use std::rc::Rc;

use slint::Model as _;
use yeban_app::bridge::{
    BuiltinEffect, CLIP_CREATE_NO_TRACK_REASON, DEVICE_INSERT_NO_TRACK_REASON, ViewState,
    builtin_effect_device, device_insert_ops, device_remove_ops,
};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::EntityId;
use yeban_model::project::{ClipContent, DeviceKind, TrackKind, TrackV3, YebanProjectV1};
use yeban_model::samples::{default_project, demo_project, filled_project};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, report_line};

/// 窗口尺寸：与 `track_routing_crud.rs` 同款（同一块 Tab 3 面板需要高度余量）。
const WINDOW: Size = Size::new(1440, 1000);

/// 一次提交用的时间戳（与其余界面判据同一个值，⛔ 不引入时钟依赖）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 4/4 的 4 小节 = `4 × 4 × 960`（`PPQ = 960` 从模型读，⛔ 本文件不手抄 PPQ）。
fn four_bars_ticks(project: &YebanProjectV1) -> u64 {
    let bar = yeban_app::bridge::bar_length_ticks(project.time_signature).expect("拍号必须合法");
    bar * 4
}

/// 一条带撤销端口的 Tab 3 活窗口：接线走**生产同一个** `host::wire_device_rack` /
/// `host::wire_clip_create` / `host::wire_track_strip`。
fn live_window_with_undo(
    project: &YebanProjectV1,
    tag: &str,
) -> (LivePort<MainWindow>, Rc<UndoPort>) {
    let view = ViewState::from_project(project).expect("投影");
    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            format!("<判据 {tag}>"),
            "yeban-app",
            project.clone(),
            NOW_MS,
        )
        .expect("打开撤销会话"),
    ));
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window_with_console_tab(&view, &scene, 2)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    host::wire_device_rack(live.ui(), &port);
    host::wire_clip_create(live.ui(), &port);
    host::wire_track_strip(live.ui(), &port);
    (live, port)
}

/// 展示轨（身份序第一条非主总线轨）—— 与投影**同一条**规则。
fn display_track(project: &YebanProjectV1) -> &TrackV3 {
    project
        .tracks
        .values()
        .find(|track| track.id != project.master_bus_track_id)
        .expect("夹具必须有展示轨")
}

/// 只有主总线的合法工程（⑥ 的夹具）。
fn project_with_master_only() -> YebanProjectV1 {
    let mut project = YebanProjectV1::default();
    let master_id = EntityId::new();
    project.master_bus_track_id = master_id;
    project.tracks.insert(
        master_id,
        TrackV3 {
            id: master_id,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    project.routing_graph.nodes = vec![master_id];
    project.validate().expect("只剩主总线的工程必须合法");
    project
}

/// 契约里的 `(分组参数名序列, 每条的缺省值)` —— 从共享契约现读，⛔ 不手抄第二份。
fn contract_params(group_id: &str) -> (Vec<&'static str>, Vec<f32>) {
    let group = yeban_app::effect_params::EFFECT_GROUPS
        .iter()
        .find(|group| group.id == group_id)
        .expect("共享契约里必须有这个分组");
    #[allow(clippy::cast_possible_truncation)]
    (
        group.params.iter().map(|spec| spec.name).collect(),
        group
            .params
            .iter()
            .map(|spec| spec.default as f32)
            .collect(),
    )
}

/// 契约里某个参数的 `(min, max, default)`（f32）。
fn contract_spec(name: &str) -> (f32, f32, f32) {
    let (_, spec) = yeban_app::effect_params::recognise(name).expect("契约里必须有这个参数");
    #[allow(clippy::cast_possible_truncation)]
    (spec.min as f32, spec.max as f32, spec.default as f32)
}

// ===========================================================================
// 判据 ①：装 EQ = Δ1 ＋ 读回（设备 +1 / kind / 非空 params / eq_low_gain）
// ===========================================================================
#[test]
fn inserting_an_eq_commits_once_and_the_chain_carries_eq_low_gain() {
    let project = demo_project();
    let view = ViewState::from_project(&project).expect("投影");
    assert!(view.device_insert_available, "有展示轨 ⇒ 装设备可用");
    assert_eq!(view.device_insert_reason, "");
    let before_devices = display_track(&project).devices.len();
    assert_eq!(view.devices.len(), before_devices);

    let (live, port) = live_window_with_undo(&project, "#163 装 EQ");
    let before = port.display();
    live.ui().invoke_device_insert_eq();
    let after = port.display();
    assert_eq!(
        after.commit_count,
        before.commit_count + 1,
        "一次点击必须**恰好**一次 `commit_ops`（前 {}，后 {}）",
        before.commit_count,
        after.commit_count
    );
    assert_eq!(after.undoable, before.undoable + 1);

    let committed = port.project();
    let track = display_track(&committed);
    assert_eq!(
        track.devices.len(),
        before_devices + 1,
        "设备数必须 +1（追加到链尾）"
    );
    let device = track.devices.last().expect("必须有新设备");
    assert_eq!(device.name, "EQ");
    assert_eq!(device.kind, DeviceKind::InternalEffect);
    assert!(!device.bypassed);
    assert_eq!(device.latency_samples, 0);
    assert!(
        !device.params.is_empty(),
        "`params` 必须非空（⛔ 空表冒充设备）"
    );
    let (expected_names, expected_defaults) = contract_params("eq");
    let actual_names: Vec<&str> = device
        .params
        .iter()
        .map(|param| param.name.as_str())
        .collect();
    assert_eq!(
        actual_names, expected_names,
        "参数名必须逐条来自共享契约的 `eq` 分组"
    );
    let actual_defaults: Vec<f32> = device.params.iter().map(|param| param.value).collect();
    assert_eq!(
        actual_defaults, expected_defaults,
        "参数值必须逐条是契约的 `spec.default`（⛔ 手写数字）"
    );
    let eq = device
        .params
        .iter()
        .find(|param| param.name == "eq_low_gain")
        .expect("装上的 EQ 必须含 `eq_low_gain`");
    let (_, _, default_db) = contract_spec("eq_low_gain");
    assert_eq!(eq.value, default_db);
    assert_eq!(eq.unit.as_deref(), Some("dB"));
    assert_eq!(committed.validate(), Ok(()), "装完之后文档必须仍然自洽");

    let view = ViewState::from_project(&committed).expect("重投影");
    assert_eq!(view.device_names().last().map(String::as_str), Some("EQ"));
    assert!(view.device_eq_available(), "装完 EQ ⇒ 那一格滑块可用");
    assert_eq!(view.device_eq_gain_db(), default_db);
    assert_eq!(
        live.ui()
            .get_device_names()
            .iter()
            .last()
            .unwrap()
            .to_string(),
        "EQ",
        "界面数组必须跟着新设备变"
    );
    assert!(live.ui().get_device_eq_available());
    assert_eq!(
        slint::Model::row_count(&live.ui().get_device_ids()),
        track.devices.len(),
        "身份数组与设备数组必须同索引集"
    );

    report_line(&format!(
        "[#163 ①] 装 EQ：commit_count {} → {}（Δ1）；设备 {} → {}；kind={:?}；\
         params={} 条（eq_low_gain={} dB）；validate={:?}",
        before.commit_count,
        after.commit_count,
        before_devices,
        track.devices.len(),
        device.kind,
        device.params.len(),
        eq.value,
        committed.validate()
    ));
}

// ===========================================================================
// 判据 ②：装完 `#157` 的滑块可用（拖动 ⇒ 读回值变）
// ===========================================================================
#[test]
fn the_eq_slider_is_available_and_editable_after_installing_the_eq() {
    let project = demo_project();
    let view = ViewState::from_project(&project).expect("投影");
    assert!(
        !view.device_eq_available(),
        "夹具的展示轨只有 `cutoff` ⇒ 装之前没有可编辑的 eq_low_gain"
    );

    let (live, port) = live_window_with_undo(&project, "#163 EQ 滑块");
    assert!(!live.ui().get_device_eq_available());

    let before_install = port.display();
    live.ui().invoke_device_insert_eq();
    let after_install = port.display();
    assert_eq!(
        after_install.commit_count,
        before_install.commit_count + 1,
        "装 EQ 必须恰好一次提交"
    );
    assert!(
        live.ui().get_device_eq_available(),
        "装完 EQ ⇒ `device-eq-available` 必须变 true（#157 的滑块立刻可用）"
    );

    let (_, max_db, default_db) = contract_spec("eq_low_gain");
    assert_eq!(live.ui().get_device_eq_gain_db(), default_db);
    let before_drag = port.display();
    // ⭐ 任务单 `#182`：EQ 手势按**设备身份**寻址 ⇒ 从界面数组里取身份（⛔ 不传下标）。
    let eq_device_id = live.ui().get_device_eq_device_id();
    live.ui().invoke_device_eq_grab(eq_device_id, 1.0);
    live.ui().invoke_device_eq_release();
    let after_drag = port.display();
    assert_eq!(
        after_drag.commit_count,
        before_drag.commit_count + 1,
        "一次拖动 = 恰好一次提交"
    );

    let committed = port.project();
    let device = display_track(&committed).devices.last().expect("EQ");
    let eq = device
        .params
        .iter()
        .find(|param| param.name == "eq_low_gain")
        .expect("eq_low_gain");
    assert_eq!(eq.value, max_db, "拖到最右 ⇒ 读回值 = 契约上界");
    assert_eq!(live.ui().get_device_eq_gain_db(), max_db);

    report_line(&format!(
        "[#163 ②] 装完滑块：eq-available false → true；拖动 commit_count {} → {}（Δ1）；\
         eq_low_gain {} → {} dB（上界来自契约）",
        before_drag.commit_count, after_drag.commit_count, default_db, max_db
    ));
}

// ===========================================================================
// 判据 ③：装混响 = Δ1 ＋ 读回 `reverb_*`
// ===========================================================================
#[test]
fn inserting_a_reverb_commits_once_and_installs_the_reverb_params() {
    let project = demo_project();
    let before_devices = display_track(&project).devices.len();
    let (live, port) = live_window_with_undo(&project, "#163 装混响");
    let before = port.display();
    live.ui().invoke_device_insert_reverb();
    let after = port.display();
    assert_eq!(after.commit_count, before.commit_count + 1);
    assert_eq!(after.undoable, before.undoable + 1);

    let committed = port.project();
    let track = display_track(&committed);
    assert_eq!(track.devices.len(), before_devices + 1);
    let device = track.devices.last().expect("新设备");
    assert_eq!(device.name, "Reverb");
    assert_eq!(device.kind, DeviceKind::InternalEffect);
    let (expected_names, expected_defaults) = contract_params("reverb");
    let actual_names: Vec<&str> = device
        .params
        .iter()
        .map(|param| param.name.as_str())
        .collect();
    assert_eq!(actual_names, expected_names, "参数名必须来自 `reverb` 分组");
    let actual_defaults: Vec<f32> = device.params.iter().map(|param| param.value).collect();
    assert_eq!(actual_defaults, expected_defaults);
    assert!(
        !actual_names.is_empty(),
        "夹具必须非空（否则下面那条 `.all(..)` 恒真）"
    );
    assert!(actual_names.iter().all(|name| name.starts_with("reverb_")));
    let wet = device
        .params
        .iter()
        .find(|param| param.name == "reverb_wet")
        .expect("必须含 reverb_wet");
    let (_, _, wet_default) = contract_spec("reverb_wet");
    assert_eq!(wet.value, wet_default);
    assert_eq!(committed.validate(), Ok(()));

    let view = ViewState::from_project(&committed).expect("重投影");
    assert!(
        view.devices
            .last()
            .expect("混响")
            .recognised
            .contains("reverb_wet"),
        "投影的已识别读数必须含 `reverb_wet`（走共享契约的 `recognise`）"
    );

    report_line(&format!(
        "[#163 ③] 装混响：commit_count {} → {}（Δ1）；设备 {} → {}；params={} 条；\
         reverb_wet={}；recognised={:?}",
        before.commit_count,
        after.commit_count,
        before_devices,
        track.devices.len(),
        device.params.len(),
        wet.value,
        view.devices.last().expect("混响").recognised
    ));
}

// ===========================================================================
// 判据 ④：删设备 = Δ1 ＋ 读回（设备数 −1、身份消失）
// ===========================================================================
#[test]
fn removing_a_device_commits_once_and_shrinks_the_chain() {
    // `filled_project()` 的展示轨挂着一台真实的 `Yeban PolySynth`（夹具，不是被测行为）。
    let project = filled_project();
    let view = ViewState::from_project(&project).expect("投影");
    let before_devices = view.devices.len();
    assert!(before_devices >= 1, "夹具必须至少有一台设备");
    let removed_id = display_track(&project).devices[0].id;
    let removed_name = view.devices[0].name.clone();
    assert_eq!(
        view.device_ids()[0],
        removed_id.to_canonical_string(),
        "投影的设备身份必须与模型逐字一致"
    );

    let (live, port) = live_window_with_undo(&project, "#163 删设备");
    let before = port.display();
    live.ui()
        .invoke_device_remove(removed_id.to_canonical_string().into());
    let after = port.display();
    assert_eq!(
        after.commit_count,
        before.commit_count + 1,
        "一次点击必须**恰好**一次 `commit_ops`"
    );
    assert_eq!(after.undoable, before.undoable + 1);

    let committed = port.project();
    let track = display_track(&committed);
    assert_eq!(track.devices.len(), before_devices - 1, "设备数必须 −1");
    assert!(
        track.devices.iter().all(|device| device.id != removed_id),
        "被删的身份必须不在设备链里"
    );
    assert_eq!(committed.validate(), Ok(()), "删完之后文档必须仍然自洽");
    assert_eq!(
        slint::Model::row_count(&live.ui().get_device_names()),
        before_devices - 1
    );

    // 同一条身份再删一次 ⇒ 纯函数拒绝、Δ0 提交（界面禁用之外的第二道门）。
    assert_eq!(
        device_remove_ops(&committed, removed_id),
        Err(yeban_app::bridge::DEVICE_REMOVE_NOT_FOUND_REASON)
    );
    let before_second = port.display();
    live.ui()
        .invoke_device_remove(removed_id.to_canonical_string().into());
    assert_eq!(
        port.display().commit_count,
        before_second.commit_count,
        "删除一个已不存在的设备必须 Δ0"
    );

    report_line(&format!(
        "[#163 ④] 删设备：commit_count {} → {}（Δ1）；设备 {} → {}；被删={}「{}」；\
         再删一次 Δ0；validate={:?}",
        before.commit_count,
        after.commit_count,
        before_devices,
        track.devices.len(),
        removed_id.to_canonical_string(),
        removed_name,
        committed.validate()
    ));
}

// ===========================================================================
// 判据 ⑤：建空的 MIDI 片段 = Δ1 ＋ 读回（池 +1 / 轨 clips +1 / 0 / 15360 / Ok）
// ===========================================================================
#[test]
fn creating_an_empty_clip_commits_once_and_places_four_bars_at_zero() {
    let project = demo_project();
    let track_id = display_track(&project).id;
    let before_pool = project.clip_pool.len();
    let before_clips = display_track(&project).clips.len();
    let before_ids: BTreeSet<EntityId> = project.clip_pool.keys().copied().collect();
    let expected_duration = four_bars_ticks(&project);
    assert_eq!(expected_duration, 15_360, "4/4 的 4 小节必须 = 15360 tick");

    let (live, port) = live_window_with_undo(&project, "#163 建 clip");
    let before = port.display();
    live.ui().invoke_clip_create();
    let after = port.display();
    assert_eq!(
        after.commit_count,
        before.commit_count + 1,
        "一次点击必须**恰好**一次 `commit_ops`（一个原子批次）"
    );
    assert_eq!(after.undoable, before.undoable + 1);

    let committed = port.project();
    assert_eq!(committed.clip_pool.len(), before_pool + 1, "片段池必须 +1");
    let new_entry = committed
        .clip_pool
        .values()
        .find(|entry| !before_ids.contains(&entry.id))
        .expect("必须有一个新片段池条目");
    assert_eq!(new_entry.name, format!("Clip {}", before_pool + 1));
    let ClipContent::Midi { notes } = &new_entry.content else {
        panic!("新片段必须是 MIDI 内容");
    };
    assert!(notes.is_empty(), "新片段必须是**空** MIDI（0 个音符）");

    let track = committed.tracks.get(&track_id).expect("展示轨仍在");
    assert_eq!(track.clips.len(), before_clips + 1, "展示轨的摆放必须 +1");
    let placement = track
        .clips
        .values()
        .find(|placement| placement.clip_id == new_entry.id)
        .expect("新片段必须有一条摆放");
    assert_eq!(placement.start_tick, 0);
    assert_eq!(placement.duration_ticks, expected_duration);
    assert!(!placement.muted);
    assert_eq!(placement.validate(), Ok(()));
    assert_eq!(committed.validate(), Ok(()), "建完之后文档必须仍然自洽");

    // 再建一个 ⇒ 名字递增（`Clip N` 的 N 是池条目数 + 1，⛔ 不是身份）。
    let before_second = port.display();
    live.ui().invoke_clip_create();
    assert_eq!(port.display().commit_count, before_second.commit_count + 1);
    let committed_second = port.project();
    assert_eq!(committed_second.clip_pool.len(), before_pool + 2);
    assert!(
        committed_second
            .clip_pool
            .values()
            .any(|entry| entry.name == format!("Clip {}", before_pool + 2)),
        "第二个片段的名字必须递增"
    );
    assert_eq!(committed_second.validate(), Ok(()));

    report_line(&format!(
        "[#163 ⑤] 建 clip：commit_count {} → {}（Δ1）；clip_pool {} → {}；轨 clips {} → {}；\
         start_tick={}；duration_ticks={}；placement.validate={:?}；doc.validate={:?}",
        before.commit_count,
        after.commit_count,
        before_pool,
        before_pool + 1,
        before_clips,
        track.clips.len(),
        placement.start_tick,
        placement.duration_ticks,
        placement.validate(),
        committed_second.validate()
    ));
}

// ===========================================================================
// 判据 ⑥：空态 / 禁用（无展示轨 ⇒ 禁用 ＋ 逐字原因 ＋ 直接调用 Δ0）
// ===========================================================================
#[test]
fn the_action_buttons_are_disabled_with_a_verbatim_reason_without_a_track() {
    let project = project_with_master_only();
    let view = ViewState::from_project(&project).expect("投影");
    assert!(!view.device_insert_available);
    assert_eq!(view.device_insert_reason, DEVICE_INSERT_NO_TRACK_REASON);
    assert!(!view.clip_create_available);
    assert_eq!(view.clip_create_reason, CLIP_CREATE_NO_TRACK_REASON);
    assert_eq!(
        device_insert_ops(&project, BuiltinEffect::Eq),
        Err(DEVICE_INSERT_NO_TRACK_REASON)
    );
    assert_eq!(
        yeban_app::bridge::clip_create_ops(&project),
        Err(CLIP_CREATE_NO_TRACK_REASON)
    );

    // 注册表也按投影登记这三个按钮与两句逐字原因（可用性不改变"声明过"这一事实）。
    let registry = yeban_app::elements::ElementRegistry::from_view(&view);
    for id in [
        "device-insert-actions",
        "device-insert-eq-button",
        "device-insert-reverb-button",
        "clip-create-button",
        "device-insert-reason",
        "clip-create-reason",
    ] {
        assert!(registry.contains(id), "注册表必须登记 `{id}`");
    }

    let (live, port) = live_window_with_undo(&project, "#163 空态");
    assert!(!live.ui().get_device_insert_available());
    assert_eq!(
        live.ui().get_device_insert_reason().to_string(),
        DEVICE_INSERT_NO_TRACK_REASON
    );
    assert!(!live.ui().get_clip_create_available());
    assert_eq!(
        live.ui().get_clip_create_reason().to_string(),
        CLIP_CREATE_NO_TRACK_REASON
    );
    let before = port.display();
    live.ui().invoke_device_insert_eq();
    live.ui().invoke_device_insert_reverb();
    live.ui().invoke_clip_create();
    assert_eq!(
        port.display().commit_count,
        before.commit_count,
        "没有展示轨时三个手势都必须 Δ0（界面禁用之外，纯函数也拒绝）"
    );
    assert_eq!(port.project().tracks.len(), 1);
    assert_eq!(port.project().clip_pool.len(), 0);

    report_line(&format!(
        "[#163 ⑥] 空态：device-insert-available=false · reason={:?}；\
         clip-create-available=false · reason={:?}；三个回调 Δ0（commit_count={}）",
        DEVICE_INSERT_NO_TRACK_REASON, CLIP_CREATE_NO_TRACK_REASON, before.commit_count
    ));
}

// ===========================================================================
// 判据 ⑦：完整一首的头四步（新建工程 → 加轨 → 装设备 → 建 clip）
// ===========================================================================
#[test]
fn the_empty_project_can_grow_a_track_a_device_and_a_clip() {
    // `#161` 之后新建工程自带主总线 ⇒ 从 1 条轨（Master）开始。
    let project = default_project();
    assert_eq!(project.tracks.len(), 1, "新建工程必须只有主总线");
    assert_eq!(project.clip_pool.len(), 0);
    let view = ViewState::from_project(&project).expect("投影");
    assert!(view.track_create_available);
    assert!(!view.device_insert_available, "还没有音轨 ⇒ 装设备必须禁用");

    let (live, port) = live_window_with_undo(&project, "#163 完整链");
    // `UndoSession::open` 自己会留下第一条提交 ⇒ 这里只断言**增量**。
    let baseline = port.display().commit_count;
    live.ui().invoke_track_create_midi();
    assert_eq!(port.display().commit_count, baseline + 1, "加轨 = Δ1");
    assert!(
        live.ui().get_device_insert_available(),
        "加完轨 ⇒ 装设备立刻可用"
    );
    assert!(live.ui().get_clip_create_available());

    live.ui().invoke_device_insert_eq();
    assert_eq!(port.display().commit_count, baseline + 2, "装设备 = Δ1");
    live.ui().invoke_clip_create();
    assert_eq!(port.display().commit_count, baseline + 3, "建 clip = Δ1");

    let committed = port.project();
    assert_eq!(committed.tracks.len(), 2);
    let track = display_track(&committed);
    assert_eq!(track.devices.len(), 1);
    assert_eq!(track.devices[0].name, "EQ");
    assert_eq!(committed.clip_pool.len(), 1);
    assert_eq!(track.clips.len(), 1);
    let placement = track.clips.values().next().expect("摆放");
    assert_eq!(placement.start_tick, 0);
    assert_eq!(placement.duration_ticks, four_bars_ticks(&committed));
    assert_eq!(committed.validate(), Ok(()));

    report_line(&format!(
        "[#163 ⑦] 空工程 → 轨 → 设备 → clip：commit_count {} → {}（每步 Δ1）；\
         轨 {}；设备 {:?}；clip_pool {}；clips {}；duration_ticks {}；validate={:?}",
        baseline,
        port.display().commit_count,
        committed.tracks.len(),
        track.devices[0].name,
        committed.clip_pool.len(),
        track.clips.len(),
        placement.duration_ticks,
        committed.validate()
    ));
}

// ===========================================================================
// 纯函数腿：`builtin_effect_device` 的缺省值来自契约（不依赖窗口）
// ===========================================================================
#[test]
fn the_builtin_devices_take_every_default_from_the_shared_contract() {
    for effect in [BuiltinEffect::Eq, BuiltinEffect::Reverb] {
        let device = builtin_effect_device(effect).expect("契约必须含该分组");
        assert_eq!(device.name, effect.display_name());
        assert_eq!(device.kind, DeviceKind::InternalEffect);
        let (names, defaults) = contract_params(effect.group_id());
        let actual_names: Vec<&str> = device
            .params
            .iter()
            .map(|param| param.name.as_str())
            .collect();
        let actual_defaults: Vec<f32> = device.params.iter().map(|param| param.value).collect();
        assert_eq!(actual_names, names);
        assert_eq!(actual_defaults, defaults);
        assert!(
            !device.params.is_empty(),
            "夹具必须非空（否则下面那条 `.all(..)` 恒真）"
        );
        assert!(
            device.params.iter().all(|param| param.value.is_finite()),
            "所有缺省值必须有限（`DeviceDefinition::validate` 会拒绝非有限值）"
        );
        assert_eq!(device.validate(), Ok(()));
    }
    // 契约里的参数名是唯一事实源：本文件的期望值不是另一份表。
    assert_eq!(
        builtin_effect_device(BuiltinEffect::Eq)
            .expect("EQ")
            .params
            .iter()
            .filter(|param| param.name == "eq_low_gain")
            .count(),
        1
    );
}
