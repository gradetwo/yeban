//! **工程保存前的插件状态重抓钩子**（T-FIX-534）。
//!
//! 任务单 T-FIX-534（源自 T-FIX-483 审计立的 ISS-MWWZ1FH6-8303FA）：`PluginBinding::state`
//! 此前恒为**装载时刻**的一次性快照（`plugin_panel::load_device` 的 T-FIX-345 注释自证
//! "将来编辑器持有活实例后，保存工程前应再调用同一个 save_state"）。本补丁把那条注释
//! 落成就绪路径：
//!
//! 1. [`yeban_app::plugin_live`]：活实例登记表（`(track_id, device_id)` 键）＋ 保存前
//!    **重抓钩子** `refresh_plugin_states_before_save`；
//! 2. `wire_save_with_engine` 的保存闭包在构造 `SaveRequest` **之前**调钩子（活会话臂）；
//!    登记表为空 ⇒ 空转（今天的常态：GUI 还没有常驻编辑器）；
//! 3. 失败口径：某台 `save_state` 失败 ⇒ 该设备**保留旧快照**继续保存，但失败逐台
//!    **响亮**渲染（提示条 ＋ 模块回执，⛔ 静默存旧快照）。
//!
//! ## 判据表（红对照：修前钩子不存在，1/2/3 都不可能成立）
//!
//! | # | 判据 | 钉住的口径 |
//! | :--- | :--- | :--- |
//! | ① | 登记的活实例 `save_state` 落进绑定（增益 0.8 → 0.5，字节级对账） | 有活实例时保存前重抓并落新状态 |
//! | ② | 失败实例 ⇒ 旧快照原位不动 ＋ 失败逐台带设备名与错误原文 | 失败可见且不静默存旧快照 |
//! | ③ | 空登记表 ⇒ 零改动零失败（空转，今天的常态路径） | 无活实例时钩子空转但路径就绪 |
//! | ④ | 源码契约：保存闭包确实调了钩子（`host.rs` 含 `plugin_live::refresh_plugin_states_before_save`） | 钩子接进工程保存流程，不是孤岛 API |
//!
//! ## 夹具与线程模型
//!
//! 插件用**进程内桩**（`stub:sine` / [`yeban_plugin_host::StubInstrument`]——
//! `yeban-plugin-host` 无重依赖，与 GUI 装载同一套 [`yeban_app::plugin_panel::load_device`]）；
//! 登记表是 `thread_local`，libtest 每例一新线程 ⇒ 各判据互不串台。
//!
//! ## 本机不编译重依赖（`AGENTS.md` §5）
//!
//! `yeban-app` 是 Slint 重依赖 crate ⇒ 本文件在本机**不编译**；编译与执行交 CI，
//! 本地结果只是参考。

#![allow(unused)]

use std::str::FromStr as _;

use yeban_app::plugin_live::{
    has_live_instances, refresh_plugin_states_before_save, register_live_instance,
    unregister_live_instance,
};
use yeban_app::plugin_panel::load_device;
use yeban_model::samples::filled_project;
use yeban_model::{EntityId, YebanProjectV1};
use yeban_plugin_host::{
    HostError, MidiEvent, ParamInfo, PluginInstance, PluginState, STUB_GAIN_ID, StubInstrument,
};

/// 给工程第一台轨装上一台桩乐器（经 GUI 装载同一入口 ⇒ 绑定带装载时刻状态快照）。
/// 返回 `(工程, 轨 id, 设备 id, 装载时刻快照字节)`。
fn project_with_stub_device() -> (YebanProjectV1, EntityId, EntityId, Vec<u8>) {
    let mut project = filled_project();
    let track_id = *project.tracks.keys().next().expect("夹具至少一条轨");
    let device = load_device("stub:sine", None, "instrument").expect("桩必须能验证加载");
    let device_id = device.id;
    let load_snapshot = device
        .plugin
        .as_ref()
        .expect("装载即带状态快照（T-FIX-345）")
        .state
        .clone();
    project
        .tracks
        .get_mut(&track_id)
        .expect("轨存在")
        .devices
        .push(device);
    (project, track_id, device_id, load_snapshot)
}

/// 一台 `save_state` 必败的实例（失败口径的判据替身；其余方法给真实的最小实现）。
struct FailingInstance;
impl PluginInstance for FailingInstance {
    fn name(&self) -> &str {
        "判据:必败实例"
    }
    fn is_active(&self) -> bool {
        false
    }
    fn activate(&mut self, _sample_rate: f64, _max_block: usize) -> Result<(), HostError> {
        Ok(())
    }
    fn deactivate(&mut self) {}
    fn latency_samples(&self) -> u32 {
        0
    }
    fn parameters(&mut self) -> Result<Vec<ParamInfo>, HostError> {
        Ok(Vec::new())
    }
    fn parameter_value(&mut self, _id: u32) -> Option<f64> {
        None
    }
    fn set_parameter(&mut self, _id: u32, _value: f64) -> Result<(), HostError> {
        Err(HostError::Other("判据替身不接受参数".to_owned()))
    }
    fn save_state(&mut self) -> Result<PluginState, HostError> {
        Err(HostError::Other("判据注入：save_state 必败".to_owned()))
    }
    fn restore_state(&mut self, _state: &PluginState) -> Result<(), HostError> {
        Err(HostError::Other("判据替身不恢复状态".to_owned()))
    }
    fn process(
        &mut self,
        _events: &[MidiEvent],
        _frames: usize,
        _out_l: &mut [f32],
        _out_r: &mut [f32],
    ) -> Result<(), HostError> {
        Err(HostError::NotActivated)
    }
}

// ===========================================================================
// ① 有活实例：保存前重抓并落新状态
// ===========================================================================

#[test]
fn a_registered_live_instance_state_lands_in_the_binding_on_refresh() {
    let (mut project, track_id, device_id, load_snapshot) = project_with_stub_device();
    assert!(!load_snapshot.is_empty(), "装载时刻快照非空（前提）");
    assert!(
        !has_live_instances(),
        "前提：登记表初始为空（thread_local ⇒ 本线程）"
    );

    // 编辑器持有活实例：增益从 0.8 改到 0.5（StubInstrument 的 save_state = gain LE 字节）。
    let mut live = StubInstrument::new();
    live.set_parameter(STUB_GAIN_ID, 0.5)
        .expect("桩增益参数可直设（停用态）");
    register_live_instance(&track_id, &device_id, Box::new(live));

    let report = refresh_plugin_states_before_save(&mut project);
    assert_eq!(report.refreshed, 1, "恰一台重抓成功");
    assert!(
        report.failures.is_empty(),
        "无失败，实测 {:?}",
        report.failures
    );

    // 绑定状态 ≠ 装载时刻快照，且解码回读 = 0.5 的 LE 字节（不是"随便写了点什么"）。
    let binding = project.tracks[&track_id]
        .devices
        .iter()
        .find(|d| d.id == device_id)
        .expect("设备仍在")
        .plugin
        .as_ref()
        .expect("绑定仍在");
    assert_ne!(
        binding.state, load_snapshot,
        "重抓后绑定必须不再是装载时刻快照"
    );
    let decoded = yeban_plugin_host::PluginState::decode(&binding.state)
        .expect("重抓产物必须能按 YBPGST01 解码");
    assert_eq!(
        decoded.processor,
        0.5_f32.to_le_bytes(),
        "处理器段必须就是 0.5 的 LE 字节（增益真的落进去了）"
    );

    // 编辑器关闭 ⇒ 注销后再保存：登记表回空，钩子回到空转。
    assert!(unregister_live_instance(&track_id, &device_id).is_some());
    assert!(!has_live_instances());
    let report = refresh_plugin_states_before_save(&mut project);
    assert_eq!(report.refreshed, 0, "注销后无重抓");
    assert!(report.failures.is_empty());
}

// ===========================================================================
// ② 失败实例：旧快照保留 ＋ 失败逐台可见（设备名 ＋ 错误原文）
// ===========================================================================

#[test]
fn a_failing_instance_keeps_the_old_snapshot_and_reports_loudly() {
    let (mut project, track_id, device_id, load_snapshot) = project_with_stub_device();
    register_live_instance(&track_id, &device_id, Box::new(FailingInstance));

    let report = refresh_plugin_states_before_save(&mut project);
    assert_eq!(report.refreshed, 0, "失败 ⇒ 不计成功");
    assert_eq!(report.failures.len(), 1, "失败必须逐台上报");
    let (name, error) = &report.failures[0];
    assert_eq!(name, "stub:sine", "失败条目点名设备名");
    assert!(
        error.contains("save_state 必败"),
        "失败条目带错误原文，实测 {error:?}"
    );

    // ⛔ 不静默存旧快照：失败设备保留**原字节**，且这就是可见回执要说的事。
    let binding = project.tracks[&track_id]
        .devices
        .iter()
        .find(|d| d.id == device_id)
        .expect("设备仍在")
        .plugin
        .as_ref()
        .expect("绑定仍在");
    assert_eq!(
        binding.state, load_snapshot,
        "失败 ⇒ 绑定保持装载时刻快照（旧字节一位不改）"
    );

    unregister_live_instance(&track_id, &device_id);
}

// ===========================================================================
// ③ 空登记表：钩子空转（今天的常态路径）
// ===========================================================================

#[test]
fn an_empty_registry_is_a_no_op() {
    let (mut project, track_id, device_id, load_snapshot) = project_with_stub_device();
    assert!(!has_live_instances(), "前提：登记表为空");

    let report = refresh_plugin_states_before_save(&mut project);
    assert_eq!(report.refreshed, 0, "空转 ⇒ 零重抓");
    assert!(report.failures.is_empty(), "空转 ⇒ 零失败");
    let binding = project.tracks[&track_id]
        .devices
        .iter()
        .find(|d| d.id == device_id)
        .expect("设备仍在")
        .plugin
        .as_ref()
        .expect("绑定仍在");
    assert_eq!(
        binding.state, load_snapshot,
        "空转 ⇒ 绑定一位不改（装载时刻快照原样）"
    );
}

// ===========================================================================
// ④ 源码契约：保存闭包确实调了钩子（钩子接进工程保存流程，不是孤岛 API）
// ===========================================================================

#[test]
fn the_save_flow_wires_the_refresh_hook() {
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_host.contains("crate::plugin_live::refresh_plugin_states_before_save"),
        "保存闭包必须调用重抓钩子（T-FIX-534 的落点）"
    );
    assert!(
        aether_host.contains("已重抓"),
        "重抓成功必须有可见回执（响亮口径）"
    );
    assert!(
        aether_host.contains("插件状态重抓失败"),
        "重抓失败必须有可见回执（⛔ 静默存旧快照）"
    );
    let lib = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"),
    )
    .expect("读 lib.rs");
    assert!(
        lib.contains("pub mod plugin_live"),
        "登记表与钩子必须是公共模块（编辑器将来登记活实例的入口）"
    );
}
