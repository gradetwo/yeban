//! 判据共用的**零 Slint** 假执行面（`#[cfg(test)]`）。
//!
//! 放在独立模块里是为了让 `service` 与 `transport::*` 的判据共用**同一份**假面 ——
//! 否则每个模块都要抄一遍 `UiTestPort` 的 12 个方法，抄错一次就会让"判据通过"变成
//! "假面写错了"。
//!
//! 它画的是**真实像素**（`yeban_ui_test_port::image::Rgb8Image` + 那边手写的真 PNG
//! 编码器），因此截图相关判据不是"对着 mock 断言 mock"；只有 Slint 那一层没有进来
//! （这正是本机纪律允许本文件在本机 `rustc --test` 真跑的原因）。

#![cfg(test)]

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::Value;

use crate::ime::{ImeFocus, ImeState};
use crate::service::UiService;
use crate::surface::{AdminReport, PreviewArguments, PreviewEffect, ReportValue, UiSurface};

use yeban_mcp::jsonrpc::{ErrorObject, Request};
use yeban_mcp::security::{BearerToken, Channel, RunMode, Scope, ScopeSet};
use yeban_ui_test_port::image::{Rect, Rgb8Image, Size};
use yeban_ui_test_port::port::{KeyCode, Permission, PointerButton, PortError};
use yeban_ui_test_port::tree::{ControlNode, ControlTree, Role};

/// 夹具的共享状态：测试与假执行面共用（图像 / 权限 / 调用日志 / 管理动作开关）。
///
/// 控件树**不**在这里：`UiTestPort::tree` 要求返回 `&ControlTree`，需要真实生命周期，
/// 因此执行面自己持有一份拷贝（构造时同步）。测试要换树就换一个执行面。
pub(crate) struct Fixture {
    pub(crate) permission: Permission,
    pub(crate) image: Rgb8Image,
    pub(crate) calls: Vec<String>,
    pub(crate) reject_admin: bool,
    /// 下一个管理动作要交出的**结构化回执**（`app-mixer` 工作线新增）。
    ///
    /// 假执行面不"真的"做事，但它必须能扮演"会交回执的执行面"，否则服务层的
    /// "把回执挂进 `result.report`"这条路径在本机（零 Slint）就没有判据覆盖。
    pub(crate) report: Option<AdminReport>,
    // ------------------------------------------------------------------
    // `ui-mcp-dryrun-ime` 工作线（ADR-0001 **D48** / `[UI-A11Y-002]`）新增的
    // **可观测状态**。它们存在的唯一理由是让"dryRun 前后状态逐字段相同"与
    // "dryRun 的预览 == 真做之后的状态"成为**可断言**的事实 ——
    // 一个只有调用日志的假面证明不了"状态没变"（日志是"被问了什么"，不是"状态是什么"）。
    // ------------------------------------------------------------------
    /// `ui/switch_main_view` 真的会写的那个开关（真执行面上是 `MainWindow.arrangement-view`）。
    pub(crate) arrangement_view: bool,
    /// `ui/force_save` 的保存轮次（真执行面上是 `LiveAdminSurface::save_epoch`）。
    pub(crate) save_epoch: u64,
    /// `ui/reload_engine` 的引擎代数（真执行面上是 `EngineHost` 的 generation）。
    pub(crate) engine_generation: u64,
    /// `[UI-A11Y-002]` 的 IME 合成态 —— **同一份**状态既喂给 `ime_state`（观测）
    /// 又喂给 `preview_effect`（"这一键会被怎么处置"），因此不是影子变量。
    pub(crate) ime_composing: bool,
    /// IME 状态机的焦点分类。
    pub(crate) ime_focus: ImeFocus,
    /// `ui/set_track_height` 真的会写的那个数（真执行面上是 `MainWindow` 的
    /// `track-height-override-pxs`，`ADR-0004` S1）。
    pub(crate) track_height_px: u32,
    /// `ui/open_project` 真的会打开的那个路径（真执行面上是 `LiveAdminSurface` 换掉的
    /// 那一份工程；这里只记路径 —— 假面没有容器层）。
    pub(crate) opened_path: Option<String>,
    /// **探针**：读方法在返回前调了几次 [`UiSurface::refresh_runtime_tree`]。
    ///
    /// 它是仪器，**不是**领域状态 ⇒ 刻意**不进** [`Fixture::snapshot`]：
    /// `snapshot` 是 `dryRun` 的对照文档（"只会改状态的那 9 条方法有没有越界"），
    /// 把一个读路径的计数器塞进去会改掉那份对照的含义。
    pub(crate) refreshes: usize,
    /// 下一次刷新要换上的运行时树（`None` = 树不动）。
    ///
    /// 判据用它证明"刷新真的发生在**读之前**"：预置一棵带探针节点的树，
    /// 紧接着读一次 `ui/tree` 就应当看见那个探针。真执行面上"换树"由
    /// `LiveAdminSurface::refresh_tree` 从**活窗口**重抓完成。
    pub(crate) refreshed_tree: Option<ControlTree>,
    /// 探针序号（每调一次 [`set_refreshed_tree_with_probe`] 自增）。
    ///
    /// 它的存在是为了让"每一条读方法都刷新"可以被**逐条**判定 ——
    /// 理由见 [`set_refreshed_tree_with_probe`] 的文档。
    pub(crate) refresh_serial: usize,
}

impl Fixture {
    /// `[UI-A11Y-002]` 的驱动点（与 `yeban_app::input::InputContext::begin_composition`
    /// 同名同义）：真的开始合成。
    pub(crate) fn begin_composition(&mut self) {
        self.ime_composing = true;
    }

    /// 合成结束（候选词上屏或取消）。
    pub(crate) fn end_composition(&mut self) {
        self.ime_composing = false;
    }

    /// 切换焦点（与 `InputContext::set_focus` 同语义：焦点离开文本域 ⇒ 结束合成态）。
    pub(crate) fn set_focus(&mut self, focus: ImeFocus) {
        self.ime_focus = focus;
        if focus != ImeFocus::TextInput {
            self.ime_composing = false;
        }
    }

    /// **逐字段状态快照**（`dryRun` 判据的对照物）。
    ///
    /// 它只包含**可变的**状态：权限 / 三个真动作的状态 / IME 状态 / 调用日志 / 回执 /
    /// 像素指纹。控件树不在里面 —— `UiTestPort::tree` 返回 `&ControlTree`，
    /// 假面根本没有可变入口（真执行面上那棵树会被 `refresh_tree` 换掉，
    /// 因此 CI 侧的判据用 `ui/tree` 的**线上 JSON** 快照去对，见
    /// `crates/yeban-app/tests/live_ui_mcp.rs`）。
    pub(crate) fn snapshot(&self) -> Value {
        serde_json::json!({
            "permission": self.permission.as_str(),
            "arrangementView": self.arrangement_view,
            "saveEpoch": self.save_epoch,
            "engineGeneration": self.engine_generation,
            "trackHeightPx": self.track_height_px,
            "openedPath": self.opened_path,
            "imeComposing": self.ime_composing,
            "imeFocus": self.ime_focus.as_str(),
            "calls": self.calls,
            "hasReport": self.report.is_some(),
            "imageFingerprint": crate::surface::fnv1a64(self.image.pixels()),
        })
    }
}

pub(crate) fn fixture_tree() -> ControlTree {
    let mut tree = ControlTree::new();
    // ⚠ `track-0-fader` **刻意**没有几何包围盒：既有判据用它钉住 `[UI-TEST-001]` 的
    // `visible: null` 证据语义（"元素在树里但没有几何 ⇒ 可见性不可断定"，
    // 见 `service.rs` 的 `unknown_semantic_id_is_an_explicit_error_not_an_empty_success`）。
    // 因此本假面**不**模拟 `LivePort::dispatch_pointer_down_impl` 的 `MissingGeometry`
    // 前置（真执行面在 `crates/yeban-ui-test-port/src/render.rs:513-521` 有它）——
    // `dryRun` 的只读前置校验会照真执行面的口径报 `-32009`，这一点写在
    // `docs/ledger/ui-mcp-dryrun-ime-notes.md` 的"假面保真度"一节。
    tree.insert(ControlNode::new(
        "track-0-fader",
        Role::parse("slider").expect("合法角色"),
        "轨道 0 推子",
    ))
    .expect("插入");
    tree.insert(
        ControlNode::new(
            "mixer-vu-track-0",
            Role::parse("progress-indicator").expect("合法角色"),
            "VU 0",
        )
        .with_bounds(Rect::new(40, 60, 8, 64))
        .as_dynamic(),
    )
    .expect("插入");
    tree.insert(
        ControlNode::new(
            "transport-timecode",
            Role::parse("text").expect("合法角色"),
            "时间码",
        )
        .with_bounds(Rect::new(8, 4, 96, 16))
        .as_dynamic(),
    )
    .expect("插入");
    tree
}

/// 树的某个节点换一份几何（判据 ⑦ 用它证明遮罩矩形来自树而不是常量）。
pub(crate) fn tree_with_bounds(id: &str, bounds: Option<Rect>) -> ControlTree {
    let source = fixture_tree();
    let mut tree = ControlTree::new();
    for node in source.iter() {
        let mut node = node.clone();
        if node.id == id {
            node.bounds = bounds;
        }
        tree.insert(node).expect("插入");
    }
    tree
}

/// 刷新探针的 ID 前缀（完整 ID = 前缀 + 序号，见 [`set_refreshed_tree_with_probe`]）。
pub(crate) const REFRESH_PROBE_PREFIX: &str = "refresh-probe-";

/// 预置"下一次刷新要换上的树"：在夹具树之外**多一个** [`REFRESH_SENTINEL_ID`]。
///
/// 哨兵是**动态区且有几何**：于是它可以同时钉住六条读路径 ——
/// `ui/tree` 的节点集合、`ui/node` / `ui/property` 的按 ID 寻址、
/// `ui/dynamic_regions` 的一格、`ui/coverage` 的 `missingAtRuntime`、
/// `ui/screenshot` 的 `maskedRegions`。
///
/// ## ⚠ 探针 ID 为什么带**序号**（这条是从一次"没红的注入"换来的）
///
/// 第一版用一个**固定** ID。它有一个致命弱点：`ui/tree` 先读、它刷新了，于是
/// **后面**每一条读方法的树里都**已经有**那个固定 ID 了 —— 此时把 `ui/node` 的刷新
/// 单独摘掉，判据**仍然绿**（实测）。也就是说那种写法只证明了"树被刷新过至少一次"，
/// 没有证明"**每一条**读方法都刷新"。
///
/// 序号把每一版探针区分开：判据对每一条方法都要求"看见**这一版**的新探针、
/// 且看不见上一版"。摘掉任何一条的刷新 ⇒ 它读到的是上一版或原始树 ⇒ 红。
pub(crate) fn set_refreshed_tree_with_probe(state: &Rc<RefCell<Fixture>>) -> String {
    let serial = {
        let mut state = state.borrow_mut();
        state.refresh_serial += 1;
        state.refresh_serial
    };
    let id = format!("{REFRESH_PROBE_PREFIX}{serial}");
    let mut tree = fixture_tree();
    tree.insert(
        ControlNode::new(
            id.as_str(),
            Role::parse("progress-indicator").expect("合法角色"),
            "刷新探针",
        )
        .with_bounds(Rect::new(0, 0, 4, 4))
        .as_dynamic(),
    )
    .expect("插入");
    state.borrow_mut().refreshed_tree = Some(tree);
    id
}

/// 上一版探针的 ID（`set_refreshed_tree_with_probe` 的序号 − 1）。
pub(crate) fn previous_probe_id(state: &Rc<RefCell<Fixture>>) -> String {
    let serial = state.borrow().refresh_serial;
    format!("{REFRESH_PROBE_PREFIX}{}", serial.saturating_sub(1))
}

/// 探针读数：读方法一共触发了几次刷新（见 [`Fixture::refreshes`]）。
pub(crate) fn refreshes(state: &Rc<RefCell<Fixture>>) -> usize {
    state.borrow().refreshes
}

pub(crate) fn fixture_image() -> Rgb8Image {
    let mut image = Rgb8Image::new(Size::new(200, 120));
    image.fill_rect(Rect::new(0, 0, 200, 120), [24, 26, 32]);
    image.fill_rect(Rect::new(40, 60, 8, 64), [80, 220, 120]);
    image
}

pub(crate) fn shared(permission: Permission) -> Rc<RefCell<Fixture>> {
    Rc::new(RefCell::new(Fixture {
        permission,
        image: fixture_image(),
        calls: Vec::new(),
        reject_admin: false,
        report: None,
        arrangement_view: false,
        save_epoch: 0,
        engine_generation: 0,
        ime_composing: false,
        ime_focus: ImeFocus::MainCanvas,
        // 与投影的默认基准行高同值（`bridge::DEFAULT_TRACK_HEIGHT_PX = 56`）——
        // 假面不认识那个常量（零依赖方向），因此这里写死同一个数并说明来源。
        track_height_px: 56,
        opened_path: None,
        refreshes: 0,
        refreshed_tree: None,
        refresh_serial: 0,
    }))
}

/// 当前状态的一份**逐字段快照**（`dryRun` 判据的对照物）。
pub(crate) fn snapshot(state: &Rc<RefCell<Fixture>>) -> Value {
    state.borrow().snapshot()
}

/// 调用日志（假执行面收到的动作，按发生顺序）。
pub(crate) fn calls(state: &Rc<RefCell<Fixture>>) -> Vec<String> {
    state.borrow().calls.clone()
}

/// 假执行面：**零 Slint**，因此本文件的全部判据都能在本机真跑。
///
/// 它画的是真实像素（`Rgb8Image` + `yeban-ui-test-port` 的真 PNG 编码器），
/// 所以截图那几条判据不是"对着 mock 断言 mock"。
pub(crate) struct FakeSurface {
    pub(crate) state: Rc<RefCell<Fixture>>,
    pub(crate) tree: ControlTree,
}

impl UiSurface for FakeSurface {
    fn surface_name(&self) -> &'static str {
        "fake-surface"
    }
    fn capture_image(&self) -> Result<Rgb8Image, PortError> {
        Ok(self.state.borrow().image.clone())
    }

    /// 假面的"刷新运行时树"：探针 +1，并在夹具预置了下一棵树时把它换进 `self.tree`。
    ///
    /// 它**不**写 [`Fixture::calls`]：`calls` 记的是"假面收到的**动作**"，
    /// 而刷新是一次读内部的缓存更新 —— 混进去会让既有的"注入日志逐字相等"判据
    /// 变成在数一次读的开销。默认（`refreshed_tree == None`）树一位不动，
    /// 因此既有的 `tree_method_result_is_byte_stable_across_two_calls` 等判据逐字不变。
    fn refresh_runtime_tree(&mut self) -> Result<(), PortError> {
        let mut state = self.state.borrow_mut();
        state.refreshes += 1;
        if let Some(next) = state.refreshed_tree.take() {
            self.tree = next;
        }
        Ok(())
    }

    /// 交出（并取走）夹具里预置的回执 —— 与真实执行面同语义。
    fn take_admin_report(&mut self) -> Option<AdminReport> {
        self.state.borrow_mut().report.take()
    }

    /// `[UI-A11Y-002]` 的 IME 合成态 —— 读的是夹具里**那一份**状态
    /// （驱动点是 [`Fixture::begin_composition`] / [`Fixture::set_focus`]）。
    fn ime_state(&self) -> Option<ImeState> {
        let state = self.state.borrow();
        Some(ImeState {
            composing: state.ime_composing,
            focus: state.ime_focus,
        })
    }

    /// `dryRun` 的只读影响预览：与 `*_impl` **共用同一份夹具状态**，
    /// 因此"预览说的"与"真做之后的"可以直接比对（判据 `dry_run_preview_…`）。
    ///
    /// 这里刻意**不**调用任何 `*_impl`、也不碰 `calls` —— 一个会留下痕迹的
    /// "只读预览"会让 `dry_run_leaves_the_state_untouched_…` 立刻变红。
    fn preview_effect(
        &self,
        method: &str,
        arguments: &PreviewArguments,
    ) -> Result<Option<PreviewEffect>, PortError> {
        let state = self.state.borrow();
        let effect = match method {
            crate::methods::METHOD_SWITCH_MAIN_VIEW => {
                let view = arguments.text("view");
                PreviewEffect::new(vec![
                    (
                        "view",
                        ReportValue::Text(view.unwrap_or_default().to_owned()),
                    ),
                    // 将要写进去的那个开关（真执行面上是 `MainWindow.arrangement-view`）。
                    (
                        "arrangementView",
                        ReportValue::Bool(view == Some("arrangement")),
                    ),
                ])
            }
            crate::methods::METHOD_FORCE_SAVE => {
                if state.reject_admin {
                    // "这次真调用一定会失败" —— 与 `force_save_impl` **同一句话**。
                    return Err(PortError::Rejected {
                        message: "强制保存需要工程存储层".to_owned(),
                    });
                }
                PreviewEffect::new(vec![(
                    "saveEpoch",
                    ReportValue::Uint(state.save_epoch.saturating_add(1)),
                )])
            }
            crate::methods::METHOD_RELOAD_ENGINE => PreviewEffect::new(vec![(
                "generation",
                ReportValue::Uint(state.engine_generation.saturating_add(1)),
            )]),
            // `ADR-0004` S1：将要写的**基准**行高 + 当前读数（只读回读）。
            crate::methods::METHOD_SET_TRACK_HEIGHT => {
                let requested = arguments.number("heightPx").map_or(0, |value| value as u64);
                PreviewEffect::new(vec![
                    (
                        "currentBasePx",
                        ReportValue::Uint(u64::from(state.track_height_px)),
                    ),
                    ("requestedPx", ReportValue::Uint(requested)),
                ])
            }
            // `[UI-A11Y-002]`：这一键在当前 IME/焦点状态下会被怎么处置。
            //
            // ⚠ 假面**只知道两档**：它没有 `[UI-A11Y-001]` 的扫描码表（那是
            // `yeban-app::input` 的知识），因此非合成态一律报 `pass-through`，
            // **不冒充** `action`（真执行面用 `InputContext::resolve` 回答：
            // `Space` 在画布上是 `action`）。取值本身来自 `crate::ime` 的**唯一**词表，
            // 两个执行面因此不会各发明一个名字（判据
            // `key_resolution_vocabulary_is_closed_and_unique`）。
            crate::methods::METHOD_DISPATCH_KEY_PRESS => {
                let consumed = state.ime_composing && state.ime_focus == ImeFocus::TextInput;
                PreviewEffect::new(vec![
                    ("isComposing", ReportValue::Bool(state.ime_composing)),
                    (
                        "focus",
                        ReportValue::Text(state.ime_focus.as_str().to_owned()),
                    ),
                    (
                        "resolution",
                        ReportValue::Text(
                            if consumed {
                                crate::ime::RESOLUTION_CONSUMED_BY_IME
                            } else {
                                crate::ime::RESOLUTION_PASS_THROUGH
                            }
                            .to_owned(),
                        ),
                    ),
                ])
            }
            // `ui/open_project`：将要打开的路径 + 当前已打开的路径（**只读**回读）。
            //
            // 假面没有容器层（零依赖方向：它不认识 `yeban-app` 的 `open_project_file`），
            // 因此它**不**校验路径存在与否、也**不**模拟 `saveFirst` 的落盘 ——
            // 那些是真实载体 `LiveAdminSurface::open_project_now` 的职责，它们由
            // `crates/yeban-app/tests/live_ui_mcp.rs` 的端到端判据覆盖。这里只保证
            // "预览说的路径"与"真做之后夹具里的路径"是同一个。
            crate::methods::METHOD_OPEN_PROJECT => {
                let path = arguments.text("path").unwrap_or_default();
                PreviewEffect::new(vec![
                    ("path", ReportValue::Text(path.to_owned())),
                    (
                        "saveFirst",
                        ReportValue::Bool(arguments.is_true("saveFirst").unwrap_or(true)),
                    ),
                    (
                        "currentPath",
                        state
                            .opened_path
                            .clone()
                            .map_or(ReportValue::Text(String::new()), ReportValue::Text),
                    ),
                ])
            }
            // 指针事件的影响只有窗口自己知道 ⇒ 如实报 `None`（不编造）。
            _ => return Ok(None),
        };
        Ok(Some(effect))
    }
}

impl yeban_ui_test_port::port::UiTestPort for FakeSurface {
    fn permission(&self) -> Permission {
        self.state.borrow().permission
    }
    fn tree(&self) -> &ControlTree {
        &self.tree
    }
    fn capture_png(&self) -> Result<Vec<u8>, PortError> {
        yeban_ui_test_port::png::encode_rgb8_limited(
            &self.state.borrow().image,
            yeban_ui_test_port::png::REPO_MAX_FILE_BYTES,
        )
        .map_err(|error| PortError::Capture {
            message: error.to_string(),
        })
    }
    fn read_property(&self, element_id: &str, name: &str) -> Result<String, PortError> {
        if !self.tree.contains(element_id) {
            return Err(PortError::UnknownElement {
                id: element_id.to_owned(),
            });
        }
        match name {
            "value" | "width" | "height" => Ok(format!("{name}=1")),
            other => Err(PortError::Rejected {
                message: format!("不支持的属性名 `{other}`"),
            }),
        }
    }
    fn dispatch_pointer_down_impl(
        &mut self,
        element_id: &str,
        x_offset: f64,
        y_offset: f64,
        button: PointerButton,
    ) -> Result<(), PortError> {
        self.state.borrow_mut().calls.push(format!(
            "pointer_down:{element_id}:{x_offset}:{y_offset}:{}",
            button.as_str()
        ));
        Ok(())
    }
    fn dispatch_pointer_move_impl(&mut self, x: f64, y: f64) -> Result<(), PortError> {
        self.state
            .borrow_mut()
            .calls
            .push(format!("pointer_move:{x}:{y}"));
        Ok(())
    }
    fn dispatch_pointer_up_impl(&mut self, button: PointerButton) -> Result<(), PortError> {
        self.state
            .borrow_mut()
            .calls
            .push(format!("pointer_up:{}", button.as_str()));
        Ok(())
    }
    fn dispatch_key_press_impl(&mut self, key: KeyCode) -> Result<(), PortError> {
        self.state
            .borrow_mut()
            .calls
            .push(format!("key_press:{}", key.as_str()));
        Ok(())
    }
    fn switch_main_view_impl(&mut self, view: &str) -> Result<(), PortError> {
        let mut state = self.state.borrow_mut();
        if state.reject_admin {
            return Err(PortError::Rejected {
                message: "主视图切换需要模型绑定".to_owned(),
            });
        }
        state.calls.push(format!("switch_main_view:{view}"));
        // **真的**改状态（真执行面上是 `MainWindow.arrangement-view`）——
        // 少了这一步, "dryRun 前后状态相同"与"真调用确实改了状态"两条判据
        // 都会退化成"对调用日志断言", 证明不了任何状态语义。
        state.arrangement_view = view == "arrangement";
        Ok(())
    }
    fn force_save_impl(&mut self) -> Result<(), PortError> {
        let mut state = self.state.borrow_mut();
        if state.reject_admin {
            return Err(PortError::Rejected {
                message: "强制保存需要工程存储层".to_owned(),
            });
        }
        state.calls.push("force_save".to_owned());
        state.save_epoch = state.save_epoch.saturating_add(1);
        Ok(())
    }
    fn reload_engine_impl(&mut self) -> Result<(), PortError> {
        let mut state = self.state.borrow_mut();
        state.calls.push("reload_engine".to_owned());
        state.engine_generation = state.engine_generation.saturating_add(1);
        Ok(())
    }
    /// `ui/set_track_height`：**真的**改夹具里的那个数（真执行面上是视图态 + 重投影）。
    ///
    /// 它是**一条**带默认实现的 `*_impl`（见 `UiTestPort::set_track_height_impl` 的
    /// 文档：底层 crate 不认识上层 app 的行高属性）。假面覆写它，于是"dryRun 前后状态
    /// 逐字段相同"与"真调用确实改了状态"这两条判据在这一条方法上也成立。
    fn set_track_height_impl(&mut self, element_id: &str, height_px: u32) -> Result<(), PortError> {
        let mut state = self.state.borrow_mut();
        state
            .calls
            .push(format!("set_track_height:{element_id}:{height_px}"));
        state.track_height_px = height_px;
        Ok(())
    }
    /// `ui/open_project`：**真的**改夹具里"当前打开的路径"那个数（真执行面上是
    /// `LiveAdminSurface` 换掉整份工程 + 重投影）。
    ///
    /// 与 [`Self::set_track_height_impl`] 同款：底层 crate 不认识"当前工程"这个概念，
    /// 因此真实载体在上层；假面覆写它，好让 D48 的三条判据在这一条方法上也成立。
    fn open_project_impl(&mut self, path: &str, save_first: bool) -> Result<(), PortError> {
        let mut state = self.state.borrow_mut();
        state
            .calls
            .push(format!("open_project:{path}:{save_first}"));
        state.opened_path = Some(path.to_owned());
        Ok(())
    }
}

pub(crate) fn build_service_with_tree(
    state: &Rc<RefCell<Fixture>>,
    granted: ScopeSet,
    mode: RunMode,
    tree: ControlTree,
) -> (UiService, BearerToken) {
    let token = BearerToken::generate().token;
    let surface = FakeSurface {
        state: Rc::clone(state),
        tree,
    };
    (
        UiService::new(token.clone(), granted, mode, Box::new(surface)),
        token,
    )
}

pub(crate) fn build_service(
    state: &Rc<RefCell<Fixture>>,
    granted: ScopeSet,
    mode: RunMode,
) -> (UiService, BearerToken) {
    build_service_with_tree(state, granted, mode, fixture_tree())
}

pub(crate) fn build_read_service(state: &Rc<RefCell<Fixture>>) -> (UiService, BearerToken) {
    build_service(
        state,
        ScopeSet::from_scopes([Scope::UiRead, Scope::UiScreenshot]),
        RunMode::Production,
    )
}

pub(crate) fn authorization(token: &BearerToken) -> String {
    format!("Bearer {}", token.expose())
}

pub(crate) fn request(method: &str, params: Value) -> Request {
    // `params: null` 不是合法 JSON-RPC（只能是对象或数组），因此这里**省略**它。
    let mut object = serde_json::json!({"jsonrpc": "2.0", "id": 7, "method": method});
    if !params.is_null() {
        object["params"] = params;
    }
    Request::parse(&object.to_string()).expect("构造请求")
}

pub(crate) fn result_of(service: &mut UiService, token: &BearerToken, request: &Request) -> Value {
    let header = authorization(token);
    let outcome = service.handle(Channel::Http, Some(&header), request);
    assert_eq!(outcome.http_status, 200, "期望成功: {outcome:?}");
    outcome.response.expect("有响应").result.expect("有 result")
}

pub(crate) fn error_of(
    service: &mut UiService,
    token: &BearerToken,
    request: &Request,
) -> ErrorObject {
    let header = authorization(token);
    let outcome = service.handle(Channel::Http, Some(&header), request);
    assert!(outcome.http_status >= 400, "期望失败: {outcome:?}");
    outcome
        .response
        .expect("有响应")
        .error_object()
        .expect("有 error")
        .clone()
}
