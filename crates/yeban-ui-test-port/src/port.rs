//! UI 测试端口的**调用面与三级权限** —— `[UI-MCP-001]` / `[UI-TEST-002]`。
//!
//! 规范来源 (Normative)：
//! - `[UI-MCP-001]` UI/UX §12.3「UI MCP 三级权限分层安全模型」的三级：
//!   `ReadOnly`（默认）仅控件树检索、响应式属性读取、无头 Framebuffer 截图，
//!   **严禁**状态写操作与事件模拟；`Interactive` 再增加模拟指针/键盘分发；
//!   `Administrative` 再增加切换工作区主视图、强制保存、重载音频引擎。
//! - `[UI-TEST-002]` §12.4「交互事件模拟注入」：`dispatch_pointer_down(element_id, x_offset,
//!   y_offset, button)` / `dispatch_pointer_move(x, y)` / `dispatch_pointer_up(button)` /
//!   `dispatch_key_press(key_code)`。
//!
//! ## 本模块**不是** `yeban-ui-mcp`
//!
//! 绑 `127.0.0.1` 的 HTTP / JSON-RPC 服务、会话 Token、`~/.yeban/session.token` 的 `0600`
//! 权限，全部属于 `crates/yeban-ui-mcp`（`[ARCH-UI-004]`）。本模块只定义**进程内的调用面**
//! 与**权限判定**，因此它零网络、零 Slint 依赖，可以在本机把判据全部跑完。
//!
//! ## 权限闸门为什么写成 trait 的默认方法
//!
//! 如果把 `if permission >= Interactive` 留给每个实现者自己写，那么"某个实现忘了检查"
//! 就会变成一条静默的越权路径。这里反过来：**实现者只能实现 `*_impl`**，
//! 带权限的公开方法是 trait 的默认方法，闸门只有一处 —— [`authorize`] 这个纯函数。
//! 于是：
//! - 越权判据可以用一个假的端口（[`tests::FakePort`]）穷举三级 × 8 个操作；
//! - 把 [`authorize`] 改成"永远放行"，判据立刻变红（注入验证 (b)，见 notes）。

use crate::tree::ControlTree;

/// 权限层级。**默认必须是 [`Permission::ReadOnly`]**（§12.3 原文"ReadOnly (默认只读层)"）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Permission {
    /// 只读：树检索 / 属性读取 / 截图。
    #[default]
    ReadOnly,
    /// 用例交互：在只读之上增加事件注入。
    Interactive,
    /// 系统管理：在交互之上增加主视图切换、强制保存、引擎重载。
    Administrative,
}

impl Permission {
    /// 层级序（用于"至少需要某一级"的比较）。
    #[must_use]
    pub const fn rank(self) -> u8 {
        match self {
            Self::ReadOnly => 0,
            Self::Interactive => 1,
            Self::Administrative => 2,
        }
    }

    /// 字面名（与 §12.3 的命名一致）。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "ReadOnly",
            Self::Interactive => "Interactive",
            Self::Administrative => "Administrative",
        }
    }

    /// 当前层级是否**不低于** `required`。
    #[must_use]
    pub const fn at_least(self, required: Self) -> bool {
        self.rank() >= required.rank()
    }
}

impl core::fmt::Display for Permission {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 端口能执行的操作。新增操作时必须同时给出它要求的最低权限（[`Operation::required_permission`]），
/// 否则 [`Operation::ALL`] 的穷举判据会失败。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Operation {
    /// 取控件树（`ReadOnly`）。
    ReadTree,
    /// 读响应式属性（`ReadOnly`）。
    ReadProperty,
    /// 抓 Framebuffer 并编码 PNG（`ReadOnly`）。
    CaptureScreenshot,
    /// 注入指针事件（`Interactive`）。
    DispatchPointer,
    /// 注入键盘事件（`Interactive`）。
    DispatchKey,
    /// 切换工作区主视图（`Administrative`）。
    SwitchMainView,
    /// 强制执行工程保存（`Administrative`）。
    ForceSave,
    /// 重载音频引擎（`Administrative`）。
    ReloadEngine,
}

impl Operation {
    /// 全部操作，供"三级 × 全部操作"的穷举判据使用。
    pub const ALL: [Self; 8] = [
        Self::ReadTree,
        Self::ReadProperty,
        Self::CaptureScreenshot,
        Self::DispatchPointer,
        Self::DispatchKey,
        Self::SwitchMainView,
        Self::ForceSave,
        Self::ReloadEngine,
    ];

    /// 该操作要求的最低权限（§12.3 的三级划分，逐条对应）。
    #[must_use]
    pub const fn required_permission(self) -> Permission {
        match self {
            Self::ReadTree | Self::ReadProperty | Self::CaptureScreenshot => Permission::ReadOnly,
            Self::DispatchPointer | Self::DispatchKey => Permission::Interactive,
            Self::SwitchMainView | Self::ForceSave | Self::ReloadEngine => {
                Permission::Administrative
            }
        }
    }

    /// 机器可读名（用于错误消息与 JSON）。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadTree => "read_tree",
            Self::ReadProperty => "read_property",
            Self::CaptureScreenshot => "capture_screenshot",
            Self::DispatchPointer => "dispatch_pointer",
            Self::DispatchKey => "dispatch_key",
            Self::SwitchMainView => "switch_main_view",
            Self::ForceSave => "force_save",
            Self::ReloadEngine => "reload_engine",
        }
    }
}

impl core::fmt::Display for Operation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// §12.4 的指针按键。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerButton {
    /// 左键。
    Left,
    /// 中键。
    Middle,
    /// 右键。
    Right,
    /// 其它（触控笔/额外键）。
    Other,
}

impl PointerButton {
    /// 字面名。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Middle => "middle",
            Self::Right => "right",
            Self::Other => "other",
        }
    }
}

/// §12.4 的 `key_code`。
///
/// 规范点名的热键都在这里：`Tab`（视图切换）、`Shift+Enter`（采纳 AI 提案）、`Esc`（放弃草稿）。
/// `ShiftEnter` 是**一个 chord**，实现侧要按顺序分发 Shift 按下 + Return 按下（见 `render.rs`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCode {
    /// Tab：视图/焦点切换。
    Tab,
    /// Esc：放弃草稿。
    Escape,
    /// Enter / Return：提交。
    Return,
    /// 空格：走带播放/停止。
    Space,
    /// Backspace：删除。
    Backspace,
    /// Shift 单键（`ShiftEnter` 之外的裸修饰键）。
    Shift,
    /// Ctrl 单键。
    Control,
    /// Shift+Enter chord：采纳 AI 提案。
    ShiftEnter,
    /// 任意单个字符（字母数字等）。
    Character(char),
}

impl KeyCode {
    /// 解析规范里的字面写法（大小写不敏感；`Esc`/`Escape`、`Enter`/`Return` 都接受）。
    ///
    /// 解析失败返回 [`PortError::UnknownKey`] 而不是静默当成 `Character('?')`：
    /// 一个拼错的键名会让"热键没生效"变成"测试没生效"。
    pub fn parse(raw: &str) -> Result<Self, PortError> {
        let normalized = raw.trim();
        let lower = normalized.to_ascii_lowercase();
        let parsed = match lower.as_str() {
            "tab" => Self::Tab,
            "esc" | "escape" => Self::Escape,
            "enter" | "return" => Self::Return,
            "space" => Self::Space,
            "backspace" => Self::Backspace,
            "shift" => Self::Shift,
            "ctrl" | "control" => Self::Control,
            "shift+enter" | "shift+return" => Self::ShiftEnter,
            _ => {
                let mut chars = normalized.chars();
                match (chars.next(), chars.next()) {
                    (Some(ch), None) => Self::Character(ch),
                    _ => {
                        return Err(PortError::UnknownKey {
                            key: raw.to_owned(),
                        });
                    }
                }
            }
        };
        Ok(parsed)
    }

    /// 规范里的字面写法（与 [`KeyCode::parse`] 互为逆）。
    #[must_use]
    pub fn as_str(self) -> String {
        match self {
            Self::Tab => "Tab".to_owned(),
            Self::Escape => "Esc".to_owned(),
            Self::Return => "Enter".to_owned(),
            Self::Space => "Space".to_owned(),
            Self::Backspace => "Backspace".to_owned(),
            Self::Shift => "Shift".to_owned(),
            Self::Control => "Ctrl".to_owned(),
            Self::ShiftEnter => "Shift+Enter".to_owned(),
            Self::Character(ch) => ch.to_string(),
        }
    }
}

/// 端口错误。每次拒绝都必须**说清**要哪一级、当前是哪一级（§12.3 的可审计性）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortError {
    /// 权限不足。
    PermissionDenied {
        /// 被拒绝的操作。
        operation: Operation,
        /// 该操作要求的最低权限。
        required: Permission,
        /// 当前权限。
        actual: Permission,
    },
    /// 语义 ID 不在控件树里（§12.2：只能按语义 ID 寻址，找不到就是找不到）。
    UnknownElement {
        /// 目标 ID。
        id: String,
    },
    /// 元素存在但没有几何包围盒，无法把"元素内偏移"换算成窗口坐标。
    MissingGeometry {
        /// 目标 ID。
        id: String,
    },
    /// 键名无法解析。
    UnknownKey {
        /// 原始输入。
        key: String,
    },
    /// 截图 / 光栅化失败。
    Capture {
        /// 失败原因（来自 Tier-1 渲染层）。
        message: String,
    },
    /// 实现侧拒绝（例如引擎未加载、`.slint` 组件未构造）。
    Rejected {
        /// 原因。
        message: String,
    },
}

impl core::fmt::Display for PortError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::PermissionDenied {
                operation,
                required,
                actual,
            } => write!(
                f,
                "权限不足: 操作 `{operation}` 需要 {required}, 当前 {actual} [UI-MCP-001]"
            ),
            Self::UnknownElement { id } => write!(f, "控件树里没有语义 ID `{id}` [UI-TEST-001]"),
            Self::MissingGeometry { id } => {
                write!(
                    f,
                    "元素 `{id}` 没有几何包围盒, 无法把元素内偏移换算为窗口坐标"
                )
            }
            Self::UnknownKey { key } => write!(f, "无法解析的 key_code: `{key}` [UI-TEST-002]"),
            Self::Capture { message } => write!(f, "截图失败: {message} [MUST-GATE-015]"),
            Self::Rejected { message } => write!(f, "实现侧拒绝: {message}"),
        }
    }
}

impl core::error::Error for PortError {}

/// **权限判定的唯一实现**（纯函数：同输入恒同输出，不读环境、不读时钟、无副作用）。
///
/// §12.3 的三级模型在代码里就是这一处比较。CI 的越权判据与 `yeban-ui-mcp` 的 HTTP 层
/// 都应当调用它，而不是各写一遍。
pub fn authorize(actual: Permission, operation: Operation) -> Result<(), PortError> {
    let required = operation.required_permission();
    if actual.at_least(required) {
        Ok(())
    } else {
        Err(PortError::PermissionDenied {
            operation,
            required,
            actual,
        })
    }
}

/// UI 测试端口的调用面。
///
/// 实现者只实现 `*_impl`（真正的动作）+ 三个只读方法；带权限的公开方法是默认方法，
/// 闸门不可绕过。
pub trait UiTestPort {
    /// 当前权限。
    fn permission(&self) -> Permission;

    /// 控件树（`ReadOnly`）。
    fn tree(&self) -> &ControlTree;

    /// 抓 Framebuffer 并编码为 PNG（`ReadOnly`）。`[MUST-GATE-015]`：尺寸非零且非全黑。
    fn capture_png(&self) -> Result<Vec<u8>, PortError>;

    /// 读一个响应式属性（`ReadOnly`）。值一律以字符串返回，避免把 Slint 类型泄进这一层。
    fn read_property(&self, element_id: &str, name: &str) -> Result<String, PortError>;

    /// 注入指针按下（`Interactive`）。实现侧通常按 §12.4 把 `element_id + 偏移` 换算成窗口坐标。
    fn dispatch_pointer_down_impl(
        &mut self,
        element_id: &str,
        x_offset: f64,
        y_offset: f64,
        button: PointerButton,
    ) -> Result<(), PortError>;

    /// 注入指针移动（`Interactive`）。
    fn dispatch_pointer_move_impl(&mut self, x: f64, y: f64) -> Result<(), PortError>;

    /// 注入指针释放（`Interactive`）。
    fn dispatch_pointer_up_impl(&mut self, button: PointerButton) -> Result<(), PortError>;

    /// 注入键盘按下（`Interactive`）。
    fn dispatch_key_press_impl(&mut self, key: KeyCode) -> Result<(), PortError>;

    /// 切换工作区主视图（`Administrative`）。
    fn switch_main_view_impl(&mut self, view: &str) -> Result<(), PortError>;

    /// 强制执行工程保存（`Administrative`）。
    fn force_save_impl(&mut self) -> Result<(), PortError>;

    /// 重载音频引擎（`Administrative`）。
    fn reload_engine_impl(&mut self) -> Result<(), PortError>;

    /// 就地做一次权限判定（供实现侧在更复杂的动作前复用同一套规则）。
    fn authorize_here(&self, operation: Operation) -> Result<(), PortError> {
        authorize(self.permission(), operation)
    }

    /// 注入指针按下。`[UI-TEST-002]` §12.4 的 `dispatch_pointer_down(element_id, x_offset, y_offset, button)`。
    fn dispatch_pointer_down(
        &mut self,
        element_id: &str,
        x_offset: f64,
        y_offset: f64,
        button: PointerButton,
    ) -> Result<(), PortError> {
        self.authorize_here(Operation::DispatchPointer)?;
        self.dispatch_pointer_down_impl(element_id, x_offset, y_offset, button)
    }

    /// 注入指针移动。`[UI-TEST-002]` §12.4 的 `dispatch_pointer_move(x, y)`。
    fn dispatch_pointer_move(&mut self, x: f64, y: f64) -> Result<(), PortError> {
        self.authorize_here(Operation::DispatchPointer)?;
        self.dispatch_pointer_move_impl(x, y)
    }

    /// 注入指针释放。`[UI-TEST-002]` §12.4 的 `dispatch_pointer_up(button)`。
    fn dispatch_pointer_up(&mut self, button: PointerButton) -> Result<(), PortError> {
        self.authorize_here(Operation::DispatchPointer)?;
        self.dispatch_pointer_up_impl(button)
    }

    /// 注入键盘按下。`[UI-TEST-002]` §12.4 的 `dispatch_key_press(key_code)`。
    fn dispatch_key_press(&mut self, key: KeyCode) -> Result<(), PortError> {
        self.authorize_here(Operation::DispatchKey)?;
        self.dispatch_key_press_impl(key)
    }

    /// 切换工作区主视图（`Administrative`）。
    fn switch_main_view(&mut self, view: &str) -> Result<(), PortError> {
        self.authorize_here(Operation::SwitchMainView)?;
        self.switch_main_view_impl(view)
    }

    /// 强制保存（`Administrative`）。
    fn force_save(&mut self) -> Result<(), PortError> {
        self.authorize_here(Operation::ForceSave)?;
        self.force_save_impl()
    }

    /// 重载引擎（`Administrative`）。
    fn reload_engine(&mut self) -> Result<(), PortError> {
        self.authorize_here(Operation::ReloadEngine)?;
        self.reload_engine_impl()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{ControlNode, Role};

    /// 假端口：只记录"真实动作被调用了几次"。越权判据的关键在于**计数器必须保持 0**。
    #[derive(Default)]
    struct FakePort {
        permission: Permission,
        tree: ControlTree,
        calls: Vec<&'static str>,
        png: Vec<u8>,
    }

    impl FakePort {
        fn with(permission: Permission) -> Self {
            let mut tree = ControlTree::new();
            tree.insert(ControlNode::new(
                "track-0-fader",
                Role::parse("slider").expect("合法角色"),
                "轨道 0 推子",
            ))
            .expect("插入应当成功");
            Self {
                permission,
                tree,
                calls: Vec::new(),
                png: vec![0x89, b'P', b'N', b'G'],
            }
        }
    }

    impl UiTestPort for FakePort {
        fn permission(&self) -> Permission {
            self.permission
        }
        fn tree(&self) -> &ControlTree {
            &self.tree
        }
        fn capture_png(&self) -> Result<Vec<u8>, PortError> {
            Ok(self.png.clone())
        }
        fn read_property(&self, element_id: &str, name: &str) -> Result<String, PortError> {
            if !self.tree.contains(element_id) {
                return Err(PortError::UnknownElement {
                    id: element_id.to_owned(),
                });
            }
            Ok(format!("{name}=1.0"))
        }
        fn dispatch_pointer_down_impl(
            &mut self,
            _element_id: &str,
            _x_offset: f64,
            _y_offset: f64,
            _button: PointerButton,
        ) -> Result<(), PortError> {
            self.calls.push("pointer_down");
            Ok(())
        }
        fn dispatch_pointer_move_impl(&mut self, _x: f64, _y: f64) -> Result<(), PortError> {
            self.calls.push("pointer_move");
            Ok(())
        }
        fn dispatch_pointer_up_impl(&mut self, _button: PointerButton) -> Result<(), PortError> {
            self.calls.push("pointer_up");
            Ok(())
        }
        fn dispatch_key_press_impl(&mut self, _key: KeyCode) -> Result<(), PortError> {
            self.calls.push("key_press");
            Ok(())
        }
        fn switch_main_view_impl(&mut self, _view: &str) -> Result<(), PortError> {
            self.calls.push("switch_main_view");
            Ok(())
        }
        fn force_save_impl(&mut self) -> Result<(), PortError> {
            self.calls.push("force_save");
            Ok(())
        }
        fn reload_engine_impl(&mut self) -> Result<(), PortError> {
            self.calls.push("reload_engine");
            Ok(())
        }
    }

    /// 判据 1: 默认权限必须是 `ReadOnly`（§12.3 原文"默认只读层"），且操作→权限表逐条正确。
    #[test]
    fn default_permission_is_read_only_and_the_operation_table_is_exact() {
        assert_eq!(Permission::default(), Permission::ReadOnly);
        assert_eq!(Permission::ReadOnly.rank(), 0);
        assert!(Permission::Administrative.at_least(Permission::Interactive));
        assert!(!Permission::Interactive.at_least(Permission::Administrative));

        let expected = [
            (Operation::ReadTree, Permission::ReadOnly),
            (Operation::ReadProperty, Permission::ReadOnly),
            (Operation::CaptureScreenshot, Permission::ReadOnly),
            (Operation::DispatchPointer, Permission::Interactive),
            (Operation::DispatchKey, Permission::Interactive),
            (Operation::SwitchMainView, Permission::Administrative),
            (Operation::ForceSave, Permission::Administrative),
            (Operation::ReloadEngine, Permission::Administrative),
        ];
        assert_eq!(
            Operation::ALL.len(),
            expected.len(),
            "ALL 必须覆盖每一个操作"
        );
        for (operation, required) in expected {
            assert_eq!(operation.required_permission(), required, "{operation}");
        }
    }

    /// 判据 2: `ReadOnly` 下**每一个**写操作都必须被拒绝，且拒绝时**实现侧一次都没被调用**。
    ///
    /// 这条是"闸门只有一处"的证明：错误来自 [`authorize`]，而不是某个实现碰巧没做事。
    #[test]
    fn read_only_denies_every_write_and_never_reaches_the_impl() {
        let mut port = FakePort::with(Permission::ReadOnly);

        assert_eq!(
            port.dispatch_pointer_down("track-0-fader", 1.0, 2.0, PointerButton::Left),
            Err(PortError::PermissionDenied {
                operation: Operation::DispatchPointer,
                required: Permission::Interactive,
                actual: Permission::ReadOnly,
            })
        );
        assert_eq!(
            port.dispatch_pointer_move(5.0, 5.0),
            Err(PortError::PermissionDenied {
                operation: Operation::DispatchPointer,
                required: Permission::Interactive,
                actual: Permission::ReadOnly,
            })
        );
        assert_eq!(
            port.dispatch_pointer_up(PointerButton::Left),
            Err(PortError::PermissionDenied {
                operation: Operation::DispatchPointer,
                required: Permission::Interactive,
                actual: Permission::ReadOnly,
            })
        );
        assert_eq!(
            port.dispatch_key_press(KeyCode::Tab),
            Err(PortError::PermissionDenied {
                operation: Operation::DispatchKey,
                required: Permission::Interactive,
                actual: Permission::ReadOnly,
            })
        );
        assert!(port.switch_main_view("arrangement").is_err());
        assert!(port.force_save().is_err());
        assert!(port.reload_engine().is_err());
        assert!(
            port.calls.is_empty(),
            "越权时实现侧不得被调用: {:?}",
            port.calls
        );

        // 只读三件事仍然可用。
        assert_eq!(port.tree().len(), 1);
        assert_eq!(
            port.read_property("track-0-fader", "value").as_deref(),
            Ok("value=1.0")
        );
        assert!(!port.capture_png().expect("只读允许截图").is_empty());
    }

    /// 判据 3: `Interactive` 放行事件注入，但仍拒绝 `Administrative` 的三个操作。
    #[test]
    fn interactive_allows_injection_but_not_administration() {
        let mut port = FakePort::with(Permission::Interactive);
        port.dispatch_pointer_down("track-0-fader", 1.0, 2.0, PointerButton::Left)
            .expect("Interactive 应当允许指针注入");
        port.dispatch_pointer_move(3.0, 4.0)
            .expect("Interactive 应当允许指针移动");
        port.dispatch_pointer_up(PointerButton::Left)
            .expect("Interactive 应当允许指针释放");
        port.dispatch_key_press(KeyCode::ShiftEnter)
            .expect("Interactive 应当允许键盘注入");
        assert_eq!(
            port.calls,
            ["pointer_down", "pointer_move", "pointer_up", "key_press"]
        );

        assert_eq!(
            port.force_save(),
            Err(PortError::PermissionDenied {
                operation: Operation::ForceSave,
                required: Permission::Administrative,
                actual: Permission::Interactive,
            })
        );
        assert!(port.switch_main_view("session").is_err());
        assert!(port.reload_engine().is_err());
    }

    /// 判据 4: `Administrative` 放行全部 8 个操作 —— 穷举，不抽样。
    #[test]
    fn administrative_allows_every_operation() {
        for operation in Operation::ALL {
            assert_eq!(
                authorize(Permission::Administrative, operation),
                Ok(()),
                "{operation} 在 Administrative 下必须放行"
            );
        }
        let mut port = FakePort::with(Permission::Administrative);
        port.switch_main_view("arrangement").expect("应当放行");
        port.force_save().expect("应当放行");
        port.reload_engine().expect("应当放行");
        assert_eq!(
            port.calls,
            ["switch_main_view", "force_save", "reload_engine"]
        );
    }

    /// 判据 5: `authorize` 是纯函数 —— 三级 × 8 操作共 24 组判定必须与 `rank` 比较完全一致，
    /// 且重复调用结果恒同（无状态、无环境依赖）。
    #[test]
    fn authorize_is_a_pure_total_function() {
        for permission in [
            Permission::ReadOnly,
            Permission::Interactive,
            Permission::Administrative,
        ] {
            for operation in Operation::ALL {
                let first = authorize(permission, operation);
                let second = authorize(permission, operation);
                assert_eq!(first, second, "authorize 必须无状态");
                let expected_ok = permission.at_least(operation.required_permission());
                assert_eq!(first.is_ok(), expected_ok, "{permission} × {operation}");
            }
        }
    }

    /// 判据 6: `[UI-TEST-002]` §12.4 点名的热键字面写法必须能解析，且解析/打印互逆。
    #[test]
    fn spec_named_hotkeys_round_trip() {
        for (raw, expected) in [
            ("Tab", KeyCode::Tab),
            ("Esc", KeyCode::Escape),
            ("Escape", KeyCode::Escape),
            ("Enter", KeyCode::Return),
            ("Shift+Enter", KeyCode::ShiftEnter),
            ("shift+enter", KeyCode::ShiftEnter),
            ("Space", KeyCode::Space),
            ("Backspace", KeyCode::Backspace),
            ("a", KeyCode::Character('a')),
            ("7", KeyCode::Character('7')),
        ] {
            assert_eq!(KeyCode::parse(raw), Ok(expected), "解析 `{raw}`");
            assert_eq!(
                KeyCode::parse(&expected.as_str()),
                Ok(expected),
                "往返 `{raw}`"
            );
        }
        assert_eq!(
            KeyCode::parse("Tab+"),
            Err(PortError::UnknownKey {
                key: "Tab+".to_owned()
            })
        );
        assert_eq!(
            KeyCode::parse(""),
            Err(PortError::UnknownKey { key: String::new() })
        );
    }

    /// 判据 7: 找不到元素 / 没有几何时给出**具体**错误（而不是静默 return Ok）。
    #[test]
    fn unknown_element_is_reported_not_swallowed() {
        let port = FakePort::with(Permission::Administrative);
        assert_eq!(
            port.read_property("track-9-fader", "value"),
            Err(PortError::UnknownElement {
                id: "track-9-fader".to_owned()
            })
        );
    }
}
