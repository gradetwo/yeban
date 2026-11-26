//! 活插件实例登记表 ＋ 工程保存前的**状态重抓钩子**（T-FIX-534）。
//!
//! ## 为什么要有这一层
//!
//! `PluginBinding::state` 此前只在**装载时刻**抓一次快照（[`crate::plugin_panel::load_device`]），
//! 注释自证："当前 GUI 还没有常驻插件编辑器 ⇒ 这里抓到的是**装载时刻**的状态；将来编辑器
//! 持有活实例后，保存工程前应再调用同一个 `save_state`。" 本模块把那条注释里的"将来"
//! 落成**就绪的路径**：
//!
//! 1. **登记表**（[`register_live_instance`] / [`unregister_live_instance`]）：编辑器将来
//!    持有活实例时按 `(track_id, device_id)` 登记；今天还没有任何写者 ⇒ 登记表恒空。
//! 2. **重抓钩子**（[`refresh_plugin_states_before_save`]）：工程保存流程在把工程交给
//!    `SaveRequest` **之前**调用 —— 登记表为空 ⇒ **空转**（零分配路径、一位不改）；命中
//!    ⇒ 调与装载时刻**同一个** `save_state`，把插件自己写回的状态落进绑定。
//!
//! ## 失败口径（默认响亮）
//!
//! 某台实例 `save_state` 失败 ⇒ 该设备**保留旧快照**继续保存（保存主流程不因一台插件
//! 中断），但失败**逐台可见**（[`RefreshReport::failures`]，GUI 侧经 `report_notice`
//! 报错态提示条 ＋ 模块回执）——⛔ 不静默存旧快照、⛔ 不把失败混进成功计数。
//!
//! ## 线程模型
//!
//! 登记表是**本线程**（UI 线程）`thread_local`：活实例由编辑器在 UI 线程创建/持有，
//! 保存闭包（`ui.on_save_project`）同样跑在 UI 线程，同线程读写无竞争；与
//! `CLIP_CLIPBOARD` / `CLIP_DRAG_GROUP` 同一取向（只活 UI 线程、⛔ 不跨线程）。

use std::cell::RefCell;
use std::collections::BTreeMap;

use yeban_model::{EntityId, YebanProjectV1};
use yeban_plugin_host::PluginInstance;

/// 登记键：`(track_id, device_id)` 的 canonical 字符串对 —— 与工程里
/// `TrackV3.id` / `DeviceDefinition.id` 一一对应（查找时逐位匹配）。
type LiveKey = (String, String);

thread_local! {
    /// 活插件实例登记表：**编辑器将来持有活实例时的唯一写点**（今天恒空）。
    static LIVE_INSTANCES: RefCell<BTreeMap<LiveKey, Box<dyn PluginInstance>>> =
        RefCell::new(BTreeMap::new());
}

/// 登记一台活实例（编辑器持有活实例后的**登记入口**；键重复 ⇒ 覆盖旧实例）。
pub fn register_live_instance(
    track_id: &EntityId,
    device_id: &EntityId,
    instance: Box<dyn PluginInstance>,
) {
    LIVE_INSTANCES.with(|slot| {
        slot.borrow_mut().insert(
            (
                track_id.to_canonical_string(),
                device_id.to_canonical_string(),
            ),
            instance,
        );
    });
}

/// 取下一台活实例（编辑器关闭 / 设备删除时的**取走入口**；取走即注销）。
pub fn unregister_live_instance(
    track_id: &EntityId,
    device_id: &EntityId,
) -> Option<Box<dyn PluginInstance>> {
    LIVE_INSTANCES.with(|slot| {
        slot.borrow_mut().remove(&(
            track_id.to_canonical_string(),
            device_id.to_canonical_string(),
        ))
    })
}

/// 登记表当前是否有活实例（保存钩子的**空转判据**；测试与诊断用）。
#[must_use]
pub fn has_live_instances() -> bool {
    LIVE_INSTANCES.with(|slot| !slot.borrow().is_empty())
}

/// 一次重抓的报告：成功台数 ＋ 逐台失败（设备名 ＋ 错误原文）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RefreshReport {
    /// `save_state` 成功并已写进 `PluginBinding::state` 的台数。
    pub refreshed: usize,
    /// 失败的台（**保留旧快照**继续保存）：`(设备名, 错误原文)`。
    pub failures: Vec<(String, String)>,
}

/// 工程保存前的**插件状态重抓钩子**（GUI 保存流程在构造 `SaveRequest` 之前调用）。
///
/// 对工程里每台带 `PluginBinding` 的设备：登记表命中同名键 ⇒ 调用该实例的
/// `save_state`（与装载时刻**同一个**方法）并把编码字节写回绑定；未命中 ⇒ 该设备
/// 保持既有快照（今天就是全部设备的常态——登记表恒空 ⇒ 本函数整体空转）。
///
/// 失败的设备**保留旧快照**（见模块文档"失败口径"），失败逐台进 [`RefreshReport::failures`]
/// ⇒ 调用方（`wire_save_with_engine` 的保存闭包）响亮渲染，⛔ 静默吞。
#[must_use]
pub fn refresh_plugin_states_before_save(project: &mut YebanProjectV1) -> RefreshReport {
    let mut report = RefreshReport::default();
    if !has_live_instances() {
        // 空转：登记表为空 ⇒ 工程一位不改（今天 GUI 没有常驻实例，这是常态路径）。
        return report;
    }
    // 第一遍（只读）：收齐全部插件设备的（track, device）键 ＋ 设备名。
    let mut keys: Vec<(String, String, String)> = Vec::new();
    for (track_id, track) in project.tracks.iter() {
        for device in &track.devices {
            if device.plugin.is_some() {
                keys.push((
                    track_id.to_canonical_string(),
                    device.id.to_canonical_string(),
                    device.name.clone(),
                ));
            }
        }
    }
    for (track_key, device_key, device_name) in keys {
        // 在登记表的 `with` 闭包**内**完成 `save_state`：可变借用随闭包结束释放，
        // ⛔ 不把指向登记表内部的引用/指针带出来（无 unsafe）。
        let captured = LIVE_INSTANCES.with(|slot| {
            slot.borrow_mut()
                .get_mut(&(track_key.clone(), device_key.clone()))
                .map(|instance| instance.save_state())
        });
        let Some(result) = captured else {
            // 键在工程里但登记表没有这台设备（未登记 / 中途注销）⇒ 保持旧快照。
            continue;
        };
        match result {
            Ok(state) => {
                // 第二遍（可变）：按键定位设备，把编码字节写回绑定。
                let encoded = state.encode();
                if let Some((_, track)) = project
                    .tracks
                    .iter_mut()
                    .find(|(id, _)| id.to_canonical_string() == track_key)
                    && let Some(device) = track
                        .devices
                        .iter_mut()
                        .find(|d| d.id.to_canonical_string() == device_key)
                    && let Some(binding) = device.plugin.as_mut()
                {
                    binding.state = encoded;
                    report.refreshed += 1;
                }
            }
            Err(error) => {
                // 失败 ⇒ 该设备保留旧快照，失败逐台上报（响亮，⛔ 静默）。
                report.failures.push((device_name, error.to_string()));
            }
        }
    }
    report
}
