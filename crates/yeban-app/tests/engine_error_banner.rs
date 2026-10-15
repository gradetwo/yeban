//! 判据（任务单 `#86` **甲**）：**引擎错误横幅** —— 把"最近的引擎错误"投到界面上。
//!
//! ## 本单补的是哪一格
//!
//! | 层 | 现状（本单之前） |
//! | :--- | :--- |
//! | 引擎侧结构化错误 | `EngineHostError::Sfz` 等（`src/engine_host.rs`）✓ |
//! | 无头执行面的读数 | `#82`：`live_surface::engine_tick` ⇒ `EngineTick::publish_error` ✓ |
//! | 生产心跳的读数 | ⛔ 只 `eprintln!`（`ProductionLoop::tick` 不往外带） |
//! | **窗口里看得见** | ⛔ **完全没有** ⇒ 用户点了 SET 一个坏路径，界面毫无反应、他以为"没生效" |
//! | ⭐ **本单** | ⭐ `host::apply_engine_error`（**唯一写者**）⇒ `MainWindow.engine-error` ⇒ `engine-error-banner` |
//!
//! ## 判定量（每个数字都有单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | 树里有没有 `engine-error-banner` | 运行时控件树 JSON 的 `ControlTree::contains` | 布尔 |
//! | `value` | 该节点的 `accessible-value` 原文 | 字符串（错误原文） |
//! | `role` / `label` | 同上节点的 `accessible-role` / `accessible-label` | kebab-case 角色 / 中文标签 |
//! | `bounds` | 同上节点的几何包围盒 | 逻辑像素（x / y / 宽 / 高） |
//! | `published_revision` / `publish_error` | 一跳心跳的发布版本 / 失败原文 | 修订号 / 文本 |
//! | `readings` | 同一跳的回收读数（`None` = 没有引擎） | 计数（`pruned` / `drained`） |
//!
//! ## 注入（每一条都由本文件的判据打红；读数贴在本单交付报告里）
//!
//! * **注入 ①「不投影」**（删掉 `live_surface::engine_tick` 里那次
//!   `host::apply_engine_error` 调用，或把 `host::apply_engine_error` 的函数体改成 `{}`）⇒
//!   [`a_bad_engine_publish_shows_the_banner_and_a_good_one_clears_it`] 第 ② 段红
//!   （坏路径之后横幅**不在**树里）。
//! * **注入 ②「把 `Unchanged` 也当成功清屏」**（`EnginePublishOutcome::Unchanged` 分支改成
//!   写空串）⇒ [`an_unchanged_tick_does_not_wipe_the_banner`] 红
//!   （"没有工程可送"的那一跳把横幅擦掉了 —— 用户会在控制面关掉工程的同一瞬间
//!   丢掉那条错误原文）。
//! * **应绿腿**：[`an_error_free_session_never_shows_the_banner`] 在 ①② 两种注入下
//!   **必须照旧绿** —— 它断言的正是"没有错误 ⇒ 不在树里"，而注入 ① 只会让它更"不在"。
//!   ⚠ 这条腿是**反向对照**：它单独不能证明横幅接上了（那由第 ② 段负责），
//!   二者**一起**才钉住"不显示 ⇄ 显示"的两个方向。
//!
//! ## 为什么用**运行时**控件树而不是静态注册表
//!
//! `[UI-TEST-001]` 要的是"用户真的能寻址到它"。静态注册表（`ElementRegistry::from_view`）
//! 只知道"声明了什么"、没有 `accessible-value`、也**不**受 `visible` 过滤
//! ⇒ 它证明不了"成功时它不显示"。本文件因此走
//! `yeban_ui_test_port::inspect::tree_from_element_root`（**活窗口**的全树内省）。

use std::path::{Path, PathBuf};
use std::rc::Rc;

use yeban_app::bridge::ViewState;
use yeban_app::engine_host::EditMark;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::{EntityId, SamplerBinding, YebanProjectV1};
use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;
use yeban_ui_test_port::inspect::{find_by_accessible_id, tree_from_element_root};
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;
use yeban_ui_test_port::tree::ControlTree;

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的装配入口与心跳 ⇒ 其余项在本测试目标里是 dead_code
// （`-D warnings` 会判红）。同款见 `tests/sfz_ui_selector.rs`。
#[allow(dead_code)]
mod live;

use live::{LiveUi, LiveWiringOptions, build_live_ui_with};

/// 错误横幅的语义 ID（`ui/engine_error_banner.slint` ↔ `src/elements.rs` 两处必须逐字相同）。
const BANNER_ID: &str = "engine-error-banner";

/// 撤销会话的"现在"（与 `sfz_ui_selector.rs` 的常量同款：判据不需要墙钟）。
const NOW_MS: u64 = 1_760_000_000_000;

/// `24-bit` 满量程（夹具写入常量样本用；与 `sfz_ui_selector.rs` 同口径）。
const I24_SCALE: f32 = 8_388_608.0;

/// 临时目录（形态照 `sfz_ui_selector.rs` 的 `scratch`）。
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-engine-error-{tag}-{}-{}",
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

/// 工程里那条**带 MIDI 音符**的非主总线轨（与 `sfz_ui_selector.rs` 同一份口径）。
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

/// 活窗口 ＋ 撤销端口 ＋ 一代引擎的夹具（与 `sfz_ui_selector.rs` 的 `Fixture` 同一形状）。
struct Fixture {
    live: LiveUi,
    window: yeban_app::ui::MainWindow,
    port: Rc<UndoPort>,
    lead: EntityId,
    lead_index: usize,
}

impl Fixture {
    /// 装一条真窗口（控制台 Tab 1 = 混音台，采样器输入框因此**可见**）＋ 起一代引擎。
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

    /// 在输入框里"敲"一段文本（`[UI-TEST-002]` §12.4 的 `SetValue` 无障碍动作）。
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

    /// **没有工程可送**的那一跳（`src/main.rs` 的心跳在 `UndoPort::try_project()` 返回
    /// `None` 时的逐字形状：控制面关掉工程之后就是它）。
    ///
    /// ⚠ 与 [`Self::heartbeat`] 的差别**只有**第三个参数：标记照样是脏的
    /// （一次失败的发布不推进"已发布的标记"），但 `engine_tick` 的
    /// `if … && let Some(project)` 不成立 ⇒ 结局是 `Unchanged`。
    fn heartbeat_without_project(&mut self) -> live::EngineTick {
        let mark = EditMark::from_display(&self.port.display());
        self.live.engine_heartbeat(mark, None)
    }

    /// 经 **UI** 把那条轨的采样器绑定设成 `path`（敲入路径 → 按 SET）。
    fn set_sampler_through_the_ui(&self, path: &str) {
        self.type_path(path);
        self.press(&self.set_id());
        assert_eq!(
            self.sampler_of_lead().map(|binding| binding.path),
            Some(path.to_owned()),
            "⭐ UI 提交之后模型里的 `sampler` 必须真的变（⛔ 不是只改了界面）"
        );
    }
}

/// ⭐ 判据一（**本单的主判据**，`DoD §3.6`）：**坏路径 ⇒ 横幅出现且内容含该错误；
/// 好路径 ⇒ 横幅消失**。两个方向都在**无头控件树 JSON** 上判。
///
/// # 判定量
///
/// ① 起引擎后（还没出过错）：`engine-error-banner` **不在**运行时树里；
/// ② 经 UI 提交一条**不存在**的 SFZ 路径 ＋ 一跳心跳：该 ID **在**树里，`role == "banner"`、
///    `label == "引擎错误横幅"`、`value` **非空且含 `SFZ`**（= `EngineHostError::Sfz` 的原文）、
///    `bounds` 宽高非零且落在窗口内；
/// ③ 再经 UI 提交一条**读得到**的 SFZ 路径 ＋ 一跳心跳：该 ID **回到不在**树里
///    （"成功时消失"），且同一跳 `publish_error == None` / `published_revision == Some(_)`；
/// ④ 不阻塞主循环：③ 那一跳的 `readings` 是 `Some(_)`（引擎照常回收，横幅没把心跳打断）。
///
/// # 怎么变红
///
/// * **注入 ①**（不投影：删掉 `live_surface::engine_tick` 的 `apply_engine_error` 调用，
///   或把它函数体清空）⇒ ② 段红（横幅不在树里）；
/// * 删掉 `.slint` 的 `accessible-id` / `accessible-value` ⇒ ② 段的 `find_by_id` 或
///   `value` 断言红；
/// * 把 `visible: text != ""` 去掉 ⇒ ① 段红（没有错误时它也在树里）。
#[test]
fn a_bad_engine_publish_shows_the_banner_and_a_good_one_clears_it() {
    let dir = scratch("banner");
    let mut fx = Fixture::new("banner");
    let pitches = pitches_of(&fx.port.project(), fx.lead);

    // ---- ① 还没出过错 ⇒ 横幅不在运行时树里（`visible: text != ""` 为假）----
    let before = fx.tree();
    assert!(
        !before.contains(BANNER_ID),
        "⛔ 本次会话还没有过引擎错误 ⇒ `{BANNER_ID}` 不该在运行时树里（实得 {} 个节点）",
        before.len()
    );

    // ---- ② 一条真的不存在的路径 ⇒ 发布失败 ⇒ 横幅必须出现 ----
    let missing = dir.join("nope.sfz");
    let missing_text = missing.to_string_lossy().into_owned();
    assert!(!missing.exists(), "夹具必须真的不存在");
    fx.set_sampler_through_the_ui(&missing_text);
    let bad_tick = fx.heartbeat();
    report_line(&format!(
        "[engine-error/bad] 心跳: published={:?} error={:?}",
        bad_tick.published_revision, bad_tick.publish_error
    ));
    assert!(
        bad_tick.publish_error.is_some(),
        "坏路径必须给出可观测的发布错误（⛔ 不是静默保留上一版）"
    );

    let after_bad = fx.tree();
    let json = fx.tree_json();
    let node = after_bad.find_by_id(BANNER_ID).unwrap_or_else(|| {
        panic!(
            "⭐ 发布失败之后 `{BANNER_ID}` 必须在运行时树里（`DoD §3.6`）。\
             树里现有的 status-* 节点:\n{}",
            json.lines()
                .filter(|line| line.contains("status-") || line.contains("engine-error"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
    report_line(&format!(
        "[engine-error/bad] 横幅节点: role={} label={:?} value={:?} bounds={:?}",
        node.role.as_str(),
        node.label,
        node.value,
        node.bounds
    ));
    assert_eq!(node.role.as_str(), "banner", "横幅的 `accessible-role`");
    assert_eq!(node.label, "引擎错误横幅", "横幅的 `accessible-label`");
    let shown = node
        .value
        .as_deref()
        .unwrap_or_else(|| panic!("⭐ 横幅必须声明 `accessible-value`（错误原文）"));
    assert!(
        !shown.is_empty(),
        "⛔ 横幅的原文不得为空串：空串在 `.slint` 侧就是 `visible: false`，自相矛盾"
    );
    assert!(
        shown.contains("SFZ"),
        "⭐ 横幅的原文必须含 `SFZ`（= `EngineHostError::Sfz` 的 `Display` 原文），实得 {shown:?}"
    );
    assert_eq!(
        Some(shown),
        bad_tick.publish_error.as_deref(),
        "⭐ 界面上的原文必须**逐字**等于同一跳的 `publish_error`（⛔ 不是界面自己编的第二份）"
    );
    let bounds = node.bounds.expect("可见的横幅必须有几何");
    assert!(
        bounds.width > 0 && bounds.height > 0,
        "横幅的几何必须是实的（宽 {} × 高 {}）",
        bounds.width,
        bounds.height
    );
    report_line(&format!(
        "[engine-error/bad] 运行时树 {} 个节点；横幅 y={} h={}（窗口 1920×1080）",
        after_bad.len(),
        bounds.y,
        bounds.height
    ));
    assert!(
        bounds.y >= 0 && bounds.y + i32::try_from(bounds.height).expect("高") <= 1080,
        "横幅必须落在窗口内（y={} h={}）—— 落在外面等于用户看不见它",
        bounds.y,
        bounds.height
    );

    // ---- ③ 换成一条**读得到**的 SFZ ⇒ 发布成功 ⇒ 横幅消失 ----
    write_const_sample(&dir.join("const.wav"), 0.25);
    let good = write_sfz(&dir, "keys.sfz", &pitches, "const.wav");
    let good_text = good.to_string_lossy().into_owned();
    fx.set_sampler_through_the_ui(&good_text);
    let good_tick = fx.heartbeat();
    report_line(&format!(
        "[engine-error/good] 心跳: published={:?} error={:?}；摊平表={:?}",
        good_tick.published_revision,
        good_tick.publish_error,
        fx.live
            .engine_sfz_plan_readings(&fx.lead)
            .map(|(r, m, _)| (r, m)),
    ));
    assert!(
        good_tick.publish_error.is_none(),
        "好的路径不该失败: {:?}",
        good_tick.publish_error
    );
    assert!(
        good_tick.published_revision.is_some(),
        "⭐ 这一次必须真的换了一份快照（否则「成功」这句话没有依据）"
    );
    // ④ 不阻塞主循环：同一跳的回收读数照常被回报（横幅只写一个属性，不参与发布/回收）。
    //    ⚠ `EngineTick::readings` 是**结构体**不是 `Option`（"有没有引擎"由装配决定），
    //    因此这里的判据是"这一跳真的带回了一份回收读数"，而不是 `.is_some()`。
    report_line(&format!(
        "[engine-error/good] 回收读数: published={} pending_len={} pruned={} released={} \
         retired={} retire_pending={}",
        good_tick.readings.published,
        good_tick.readings.pending_len,
        good_tick.readings.pruned,
        good_tick.readings.released,
        good_tick.readings.retired,
        good_tick.readings.retire_pending,
    ));
    assert_eq!(
        good_tick.readings.published,
        good_tick.published_revision.expect("上面刚断言过"),
        "⛔ 同一跳的回收读数必须与发布读数**对得上**（横幅没有把心跳的某一腿挤掉）"
    );
    let after_good = fx.tree();
    assert!(
        !after_good.contains(BANNER_ID),
        "⭐ 发布**成功** ⇒ 横幅必须消失（回到不在运行时树里）。\
         实得它还在，value={:?}",
        after_good
            .find_by_id(BANNER_ID)
            .and_then(|n| n.value.clone())
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// ⭐ 判据二（**注入 ② 的靶子**）：**"没有发布"那一跳不得擦掉横幅**。
///
/// # 这一档在生产里**真的可达**（⛔ 不是造出来的状态）
///
/// 实测（本判据的第一版就栽在这里，如实登记）：**一次失败的发布不会推进"已发布的标记"**
/// ⇒ 坏路径之后**每一跳都会重试并再次失败**（读数一直是 `Failed`，不是 `Unchanged`）。
/// 所以 `Unchanged` 不能靠"用户没再改"来造。
///
/// 它真正可达的形状在 `src/main.rs` 的心跳里：
///
/// ```text
/// let project = if production.should_publish(mark) { port.try_project() } else { None };
/// ```
///
/// `UndoPort::try_project()` 在控制面关掉工程之后返回 `None`（`yeban_close_project`）⇒
/// 此时标记**仍然是脏的**（失败没推进它），但这一跳**没有工程可送** ⇒
/// `engine_tick` 的 `if … && let Some(project)` 不成立 ⇒ 结局正是 `Unchanged`。
/// 若把那一档也当"成功清屏"，用户会在关掉工程的同一瞬间丢掉那条错误原文。
///
/// # 判定量
///
/// * 坏路径 ＋ 第一跳：横幅在树里、原文 = `publish_error`；
/// * **标记仍是脏的**、但这一跳**没有工程可送**（`engine_heartbeat(mark, None)`）
///   ⇒ `published_revision == None`、`publish_error == None`（= `Unchanged`）：
///   横幅**仍在**树里，且原文**逐字未变**。
///
/// # 怎么变红
///
/// * **注入 ②**（`EnginePublishOutcome::Unchanged` 分支改成 `ui.set_engine_error("")`）
///   ⇒ 第二跳之后横幅不在树里。
#[test]
fn an_unchanged_tick_does_not_wipe_the_banner() {
    let dir = scratch("persist");
    let mut fx = Fixture::new("persist");
    let missing = dir.join("nope.sfz");
    let missing_text = missing.to_string_lossy().into_owned();
    fx.set_sampler_through_the_ui(&missing_text);

    let first = fx.heartbeat();
    let first_error = first
        .publish_error
        .clone()
        .expect("第一跳必须失败（坏路径）");
    let shown_first = fx
        .tree()
        .find_by_id(BANNER_ID)
        .and_then(|node| node.value.clone())
        .expect("第一跳之后横幅必须在树里");
    assert_eq!(
        shown_first, first_error,
        "横幅原文 = 那一跳的 `publish_error`"
    );

    // ---- 标记仍是脏的，但这一跳**没有工程可送** ⇒ 结局是 `Unchanged` ----
    // （生产形态：控制面关掉工程之后 `try_project()` 返回 `None`，见函数文档。）
    let second = fx.heartbeat_without_project();
    report_line(&format!(
        "[engine-error/persist] 第二跳（无工程可送）: published={:?} error={:?}；横幅 value={:?}",
        second.published_revision,
        second.publish_error,
        fx.tree()
            .find_by_id(BANNER_ID)
            .and_then(|n| n.value.clone()),
    ));
    assert_eq!(
        second.published_revision, None,
        "夹具前提：这一跳没有工程可送 ⇒ 不发布（否则这一条判据测的不是它想测的那一档）"
    );
    assert_eq!(
        second.publish_error, None,
        "夹具前提：这一跳没有发布 ⇒ 也谈不上失败"
    );
    let shown_second = fx
        .tree()
        .find_by_id(BANNER_ID)
        .and_then(|node| node.value.clone())
        .unwrap_or_else(|| {
            panic!(
                "⛔ 没有发布的那一跳**不得**擦掉横幅 —— 60Hz 心跳下那等于「闪一下就走」\
                 （上一跳的原文 {first_error:?}）"
            )
        });
    assert_eq!(
        shown_second, first_error,
        "⭐ 没有发布 ⇒ 横幅一位不动（原文必须逐字未变）"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// ⭐ 判据三（**应绿腿**，第 243 条）：**一场没有错误的会话里，横幅从头到尾不出现**。
///
/// # 为什么它必须绿
///
/// 它是"不显示 ⇄ 显示"里**反向**那一半。注入 ①（不投影）**不会**碰到它 ——
/// "从来没出现过"在"永远不出现"的实现下当然还成立。因此它单独**不能**证明横幅接上了；
/// 但它能钉住另一类错误：把 `visible` 去掉、或让 `engine-error` 初值不是空串、
/// 或让 `Unchanged` 去写一个非空的占位文本 ⇒ 它当场红。
///
/// # 判定量
///
/// * 起引擎后连跳两跳（都没编辑）：横幅两跳都不在树里；
/// * 提交一条**读得到**的 SFZ（一次**成功**的发布）后再跳：仍不在树里。
#[test]
fn an_error_free_session_never_shows_the_banner() {
    let dir = scratch("clean");
    write_const_sample(&dir.join("const.wav"), 0.25);
    let mut fx = Fixture::new("clean");
    let pitches = pitches_of(&fx.port.project(), fx.lead);
    let good = write_sfz(&dir, "keys.sfz", &pitches, "const.wav");
    let good_text = good.to_string_lossy().into_owned();

    for round in 0..2 {
        let tick = fx.heartbeat();
        assert!(
            tick.publish_error.is_none(),
            "第 {round} 跳不该有错误: {:?}",
            tick.publish_error
        );
        assert!(
            !fx.tree().contains(BANNER_ID),
            "⛔ 没有错误的第 {round} 跳里 `{BANNER_ID}` 不该在运行时树里"
        );
    }

    fx.set_sampler_through_the_ui(&good_text);
    let good_tick = fx.heartbeat();
    report_line(&format!(
        "[engine-error/clean] 成功发布: published={:?} error={:?}；横幅在树里={}",
        good_tick.published_revision,
        good_tick.publish_error,
        fx.tree().contains(BANNER_ID),
    ));
    assert!(
        good_tick.published_revision.is_some(),
        "这一跳必须真的发布成功（否则这条应绿腿没测到「成功 ⇒ 仍不显示」）"
    );
    assert!(
        !fx.tree().contains(BANNER_ID),
        "⛔ **成功**的发布之后横幅仍不得出现"
    );

    std::fs::remove_dir_all(&dir).ok();
}
