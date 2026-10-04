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

use crate::service::UiService;
use crate::surface::UiSurface;

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
}

pub(crate) fn fixture_tree() -> ControlTree {
    let mut tree = ControlTree::new();
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
    }))
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
        Ok(())
    }
    fn reload_engine_impl(&mut self) -> Result<(), PortError> {
        self.state
            .borrow_mut()
            .calls
            .push("reload_engine".to_owned());
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

pub(crate) fn calls(state: &Rc<RefCell<Fixture>>) -> Vec<String> {
    state.borrow().calls.clone()
}
