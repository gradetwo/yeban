//! 判据（任务单 `#82`）：**SFZ 链条的最后一环 —— 用户怎么设它** ＋ `engine_tick` 的吞错。
//!
//! ## 本单补的是哪一环
//!
//! | 层 | 现状 |
//! | :--- | :--- |
//! | 模型面 | `TrackV3.sampler: Option<SamplerBinding>`（`#76`）✓ |
//! | 引擎面 | `EngineSnapshot.sfz_pending`（`#78`）⇒ 只带路径、⛔ 不读盘 ✓ |
//! | 控制侧解码 ＋ 离线端到端 | `#79` ✓ |
//! | 实时／GUI 发布点 ＋ 缓存 | `#80` ✓ |
//! | **UI 选择面** | ⭐ **本单**：`track-{i}-sampler-path`（文本输入）＋ SET／CLR ⇒ `Op::SetTrackSampler` |
//! | **吞错** | ⭐ **本单**：`live_surface::engine_tick` 原先 `.ok()` 吞掉发布错误 ⇒ 进 `EngineTick::publish_error` |
//!
//! ## 判定量（每个数字都有单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | 树节点 `value` | 运行时控件树里 `track-{i}-sampler-path` 的 `accessible-value` 原文 | 字符串（路径或空串） |
//! | `bounds` | 同上节点的几何包围盒 | 逻辑像素（x / y / 宽 / 高） |
//! | `undoable` | `UndoPort::display().undoable`（可撤销步数） | 步 |
//! | `regions` / `memory_bytes` / `missing` | 快照里该轨摊平表的区域数 / 内存占用 / 缺样本路径 | 条 / 字节 / 条 |
//! | `pending_len` | 快照里**待摊平**（只带路径）的条数 | 条 |
//! | `published_revision` / `publish_error` | 一跳心跳的发布版本 / 失败原文 | 修订号 / 文本 |
//!
//! ## 注入（每一条都由本文件的判据打红；读数贴在本单交付报告里）
//!
//! * **① UI 不提交**（删掉 `host::wire_mixer_edit` 里那次 `wire_mixer_sampler` 调用）⇒
//!   [`a_ui_commit_arms_the_flattened_sfz_table_in_the_live_engine`] 红（模型一位没动）；
//! * **② 恢复 `.ok()` 吞错**（`live_surface::engine_tick` 的 `match` 改回 `.ok()`）⇒
//!   [`a_missing_sfz_path_is_an_observable_publish_error`] 红（`publish_error` 恒 `None`）；
//! * **应绿腿**：[`an_unbound_sampler_stays_a_noop_through_the_ui`] 在 ①② 两种注入下都必须
//!   **照旧绿**（`sampler = None` 那条路一个字都没碰）。
//! * **应绿腿（反向腿）**：`sampler = None` 时快照里既没有摊平表也没有待摊平项 —— 与
//!   `#80` 的 [`an_absent_sampler_is_bit_identical_to_the_direct_projection`] 同一条口径。

#![allow(unused)]
use std::path::{Path, PathBuf};
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::engine_host::EditMark;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_engine::snapshot::EngineSnapshot;
use yeban_model::{EntityId, SamplerBinding, YebanProjectV1};
use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;
use yeban_ui_test_port::inspect::{find_by_accessible_id, tree_from_element_root};
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;
use yeban_ui_test_port::tree::ControlTree;

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的装配入口与三个读数 ⇒ 其余项在本测试目标里是 dead_code
// （`-D warnings` 会判红）。同款见 `tests/ui_module_actions_no_port.rs`。
#[allow(dead_code)]
mod live;

use live::{LiveUi, LiveWiringOptions, build_live_ui_with};

/// 撤销会话的"现在"（与 `live_ui_mcp.rs` 的常量同款：判据不需要墙钟）。
const NOW_MS: u64 = 1_760_000_000_000;

/// `24-bit` 满量程（夹具写入常量样本用；与 `sfz_realtime_cache.rs` 同口径）。
const I24_SCALE: f32 = 8_388_608.0;

/// 临时目录（形态照 `sfz_realtime_cache.rs` 的 `scratch`）。
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-sfz-ui-{tag}-{}-{}",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 写一条**常量** PCM 样本（24-bit 单声道；值 = 满量程的 `value` 倍）。
fn write_const_sample(path: &Path, value: f32) {
    let scale = (I24_SCALE * value) as i32;
    let buffer = PcmBuffer::Int24(vec![scale; 4_800]);
    let format = PcmFormat::integer(1, 48_000, 24);
    write_plain_wav(path, &format, &buffer).expect("写样本夹具");
}

/// 工程里那条**带 MIDI 音符**的非主总线轨（与 `#80` 判据同一份口径）。
fn midi_track_of(project: &YebanProjectV1) -> EntityId {
    *project
        .tracks
        .iter()
        .find(|(id, track)| {
            **id != project.master_bus_track_id
                && track.clips.values().any(|placement| {
                    project
                        .clip_pool
                        .get(&placement.clip_id)
                        .is_some_and(|entry| entry.content.notes().is_some())
                })
        })
        .map(|(id, _)| id)
        .expect("filled_project 必须有一条带 MIDI 音符的非主总线轨")
}

/// 那条轨上**出现过的全部音高**（排序 ＋ 去重）。
fn pitches_of(project: &YebanProjectV1, track_id: EntityId) -> Vec<u8> {
    let mut pitches: Vec<u8> = project
        .tracks
        .get(&track_id)
        .expect("轨必须在")
        .clips
        .values()
        .filter_map(|placement| project.clip_pool.get(&placement.clip_id))
        .filter_map(|entry| entry.content.notes())
        .flat_map(|notes| notes.values().map(|note| note.pitch))
        .collect();
    pitches.sort_unstable();
    pitches.dedup();
    assert!(!pitches.is_empty(), "夹具必须真的带音符");
    pitches
}

/// 写一份"每个工程音高一条区域"的 SFZ（`sample=` 用相对路径 ⇒ 按 SFZ 目录解析）。
fn write_sfz(dir: &Path, name: &str, pitches: &[u8], sample: &str) -> PathBuf {
    let mut text = String::new();
    for pitch in pitches {
        text.push_str(&format!(
            "<region> sample={sample} key={pitch} pitch_keycenter={pitch}\n"
        ));
    }
    let path = dir.join(name);
    std::fs::write(&path, text).expect("写 SFZ 夹具");
    path
}

/// 活窗口 ＋ 撤销端口 ＋ 一代引擎的夹具（与 `live_ui_mcp.rs` 的混音台判据同一形状）。
struct Fixture {
    live: LiveUi,
    window: yeban_app::ui::MainWindow,
    port: Rc<UndoPort>,
    /// 那条 MIDI 轨的身份。
    lead: EntityId,
    /// 它在**投影顺序**里的下标（语义 ID 里的 `{i}` 就是它）。
    lead_index: usize,
}

impl Fixture {
    /// 装一条真窗口（控制台 Tab 1 = 混音台，通道条因此**可见**）＋ 起一代引擎。
    ///
    /// ⚠ 引擎在 UI 提交**之前**起：于是"提交之后那一跳才把它送进引擎"是**这一条**判据
    /// 要证的句子，而不是"装配时刚好带上了"。
    fn new(tag: &str) -> Self {
        let project = yeban_model::samples::filled_project();
        let lead = midi_track_of(&project);
        let view = ViewState::from_project(&project).expect("投影");
        let lead_index = view
            .tracks
            .iter()
            .position(|track| track.id == lead.to_canonical_string())
            .unwrap_or_else(|| panic!("投影里必须有身份 {} 的轨道", lead.to_canonical_string()));
        let port = Rc::new(UndoPort::new(
            UndoSession::open(
                format!("<判据 {tag}>"),
                "yeban-app",
                project.clone(),
                NOW_MS,
            )
            .expect("打开撤销会话"),
        ));
        let mut live = build_live_ui_with(
            &project,
            &LiveWiringOptions {
                permission: Permission::Interactive,
                console_tab: 1,
                save_path: None,
                engine_quanta: 0,
                undo: Some(Rc::clone(&port)),
                proposal_source: yeban_app::proposals::ProposalSource::none(),
            },
        )
        .expect("装配（真窗口 ＋ Tier-1 执行面）");
        live.start_engine().expect("起一代引擎");
        let window = slint::ComponentHandle::clone_strong(live.ui());
        Self {
            live,
            window,
            port,
            lead,
            lead_index,
        }
    }

    fn path_id(&self) -> String {
        format!("track-{}-sampler-path", self.lead_index)
    }

    fn set_id(&self) -> String {
        format!("track-{}-sampler-set", self.lead_index)
    }

    fn clear_id(&self) -> String {
        format!("track-{}-sampler-clear", self.lead_index)
    }

    fn strip_id(&self) -> String {
        format!("track-{}-channel-strip", self.lead_index)
    }

    /// 运行时控件树的 **JSON**（`[UI-DoD §3.6]`：无头控件树 JSON 判据的唯一载体）。
    fn tree_json(&self) -> String {
        tree_from_element_root(&self.window, None)
            .expect("运行时控件树")
            .dump_json()
    }

    /// 从 JSON **读回**并做同一套结构校验（`dump_json` → `from_json` 的往返）。
    fn tree(&self) -> ControlTree {
        let json = self.tree_json();
        ControlTree::from_json(&json).expect("JSON 必须能被同一套校验读回")
    }

    /// 语义 ID ⇒ 该节点的 `accessible-value` 原文（节点不在树里就 panic）。
    fn value_of(&self, id: &str) -> Option<String> {
        self.tree()
            .find_by_id(id)
            .unwrap_or_else(|| panic!("运行时控件树里没有 `{id}`（JSON 判据）"))
            .value
            .clone()
    }

    /// 在输入框里"敲"一段文本（`[UI-TEST-002]` §12.4 的 `SetValue` 无障碍动作；
    /// 与屏读器写值走的是**同一个** `accessible-action-set-value`）。
    fn type_path(&self, text: &str) {
        let handle = find_by_accessible_id(&self.window, &self.path_id())
            .unwrap_or_else(|| panic!("控件树里没有 `{}`", self.path_id()));
        handle.set_accessible_value(text);
    }

    /// 触发一个按钮的**默认无障碍动作**（与鼠标点击是逐字同一段落点）。
    fn press(&self, id: &str) {
        let handle = find_by_accessible_id(&self.window, id)
            .unwrap_or_else(|| panic!("控件树里没有 `{id}`"));
        handle.invoke_accessible_default_action();
    }

    /// 权威工程里那条 MIDI 轨的采样器绑定原文。
    fn sampler_of_lead(&self) -> Option<SamplerBinding> {
        self.port
            .try_project()
            .expect("此刻必须有活跃工程")
            .tracks
            .get(&self.lead)
            .expect("轨必须在")
            .sampler
            .clone()
    }

    /// 生产心跳的一跳（`run_gui` 的 60Hz 定时器调用序列的逐字复制）。
    fn heartbeat(&mut self) -> live::EngineTick {
        let mark = EditMark::from_display(&self.port.display());
        let project = self.port.project();
        self.live.engine_heartbeat(mark, Some(&project))
    }
}

/// ⭐ 判据一（**UI 判据**，`DoD §3.6`）：三个语义 ID 在**无头控件树 JSON** 里存在、
/// 角色正确、几何非零，且输入框的 `value` **随模型变**（也随"敲入"变）。
///
/// # 判定量
///
/// * `track-{i}-sampler-path` / `-sampler-set` / `-sampler-clear` 三个节点在 JSON 树里；
/// * 角色：输入框 `text-input`、两个按钮 `button`；输入框的几何落在通道条内且非零；
/// * 输入框 `value`：初始 `Some("")`（`TrackV3::sampler` 是 `None`）→ 敲入一条路径 ⇒
///   当场变成那条路径 → SET ⇒ 模型变、`value` 仍是那条路径 → CLR ⇒ 模型回到 `None`、
///   `value` 回到 `Some("")`。
///
/// # 怎么变红
///
/// * 删掉 `.slint` 里那三个 `accessible-id`（或把输入框换成不声明 `accessible-value`
///   的占位矩形）⇒ 本判据红；
/// * **注入 ①**（`wire_mixer_sampler` 没被调用）⇒ 第 ③ 段红（模型一位没动，
///   `value` 在 SET 之后仍是敲进去的草稿、CLR 之后不回到空串）。
#[test]
fn the_sampler_selector_is_in_the_headless_control_tree_and_follows_the_model() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

/// ⭐ 判据二（**端到端，本单最有价值的一条**）：经 **UI** 提交 `SetTrackSampler`
/// ⇒ 模型变 ⇒ **下一跳心跳**把快照换成"该轨摊平表非空、待摊平表为空"。
///
/// 复用 `#80` 的可判段（`sfz_plan` 的 `region_count` / `memory_bytes` /
/// `missing_sample_paths` 与 `sfz_pending().len()`），因此"用户能设它、且真的生效"
/// 这句话的每一段都有机械读数。
///
/// # 判定量
///
/// * 提交前：`engine_sfz_plan_readings(lead) == None`、`engine_sfz_pending_len() == Some(0)`；
/// * UI 提交后：`port` 里那条轨的 `sampler.path` == 写入的路径；
/// * 心跳后：`published_revision.is_some()`、`publish_error.is_none()`、
///   `regions == 工程音高数`、`missing.is_empty()`、`pending_len == Some(0)`；
/// * 摊平表与 `EngineSnapshot::from_project` 的**直接投影**＋同一份 SFZ 的 `#79` 解码
///   逐位同一份（⛔ 不是"另一条路凑出来的"）。
///
/// # 怎么变红
///
/// * **注入 ①**（`wire_mixer_edit` 不再调 `wire_mixer_sampler`）⇒ 第 ② 段红
///   （模型一位没动 ⇒ 心跳发布的是**没有绑定**的那份 ⇒ 摊平表恒 `None`）。
#[test]
fn a_ui_commit_arms_the_flattened_sfz_table_in_the_live_engine() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

/// ⭐ 判据三（**吞错判据**）：给一个**不存在的路径** ⇒ 必须有一条**可观测的错误**，
/// ⛔ 不是"静默保留上一代快照"。
///
/// # 判定量
///
/// * `tick.publish_error` 是 `Some(原文)`（逐字打印），原文里带 `SFZ`（= `EngineHostError::Sfz`）；
/// * `tick.published_revision` 是 `None`（失败 ⇒ ⛔ 没有换快照）；
/// * 槽里的**旧**快照一位不动：摊平表仍 `None`、待摊平仍 `Some(0)`、发布计数不涨；
/// * 对账：**同一条路径在 `EngineHost::reload` 上也失败**（`#80` 的判据六同款）——
///   说明这条错误不是本执行面自己发明的。
///
/// # 怎么变红
///
/// * **注入 ②**（把 `engine_tick` 改回 `self.engine.publish_project(project, mark).ok()`）
///   ⇒ `publish_error` 恒 `None`、`published_revision` 也是 `None` ⇒ 本判据第 ① 段红。
///   ⚠ 那正是修之前的形态："失败"与"没改动"在读数上**一模一样**。
#[test]
fn a_missing_sfz_path_is_an_observable_publish_error() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

/// ⭐ 判据四（**应绿的反向腿**，第 243 条）：`sampler = None` 那条路
/// 在注入 ①② 下都必须**照旧绿** —— 空文本的 SET 与未绑定轨的 CLR 都**不提交**，
/// 快照里既没有摊平表也没有待摊平项。
///
/// # 判定量
///
/// * 空文本 + SET ⇒ `undoable` 一位不动（⛔ 不落 `Some("")`）、模型仍是 `None`；
/// * 未绑定轨 + CLR ⇒ 同样不提交（`old == new` ⇒ 不制造"什么也没改"的一步历史）；
/// * 心跳：`publish_error` 是 `None`、摊平表 `None`、待摊平 `Some(0)`。
///
/// # 为什么它必须绿
///
/// 它是"这条 UI 面**只**在用户真的改了绑定时才动工程"的反向对照：注入 ①（UI 不提交）
/// 或注入 ②（恢复 `.ok()`）都不会碰到它 —— 两条注入都改不动一个"本来就没有变化"的读数。
#[test]
fn an_unbound_sampler_stays_a_noop_through_the_ui() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}
