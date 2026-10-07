//! UI 控制面的 **JSON-RPC 方法集**：名字、参数、所需 scope 与规范出处。
//!
//! ## 方法名是怎么推导出来的（规范里没有给 JSON-RPC 方法名）
//!
//! 四份规范点名的只有**能力**与**签名**，没有一条给出 JSON-RPC 的方法名：
//!
//! | 出处 | 原文说了什么 |
//! | :--- | :--- |
//! | UI/UX §12.2 `[UI-TEST-001]` | 语义元素 ID 寻址（`track-{i}-fader` 等四个族） |
//! | UI/UX §12.3 `[UI-MCP-001]` | 三级权限：只读层的三件事 = 控件树结构检索 / 响应式属性读取 / Framebuffer 截图 |
//! | UI/UX §12.4 `[UI-TEST-002]` | `dispatch_pointer_down(element_id, x_offset, y_offset, button)`、`dispatch_pointer_move(x, y)`、`dispatch_pointer_up(button)`、`dispatch_key_press(key_code)` |
//! | UI/UX §12.5 `[UI-MCP-002]` | 比对前按**元素树元数据**取高频刷新组件的矩形包围盒并置黑 |
//! | 架构 §1.3 `[ARCH-UI-004]` | "AI Agent 可通过 JSON-RPC 查询控件树，提取坐标、尺寸、可见性及自定义绑定状态" |
//! | 架构 §7.3 `[MCP-DUAL-001]` | 双 MCP：领域 MCP 与 UI 测试适配层是两个 JSON-RPC 端点 |
//!
//! 因此本线采用的名字**是工程裁决**，规则只有三条，逐条可复核：
//!
//! 1. **前缀 `ui/`**：与 §12.3 的 scope 命名空间 `ui:*` 一致；`yeban-mcp` 用的是 `tools/*`
//!    （MCP 的标准方法名），两个端点因此不会撞名；
//! 2. **`ui/` 之后是 §12.4 / §12.2 点名的能力名**：`tree` / `property` / `screenshot` /
//!    `dynamic_regions`，事件注入**逐字采用 §12.4 的函数名**
//!    （`dispatch_pointer_down` / `dispatch_pointer_move` / `dispatch_pointer_up` /
//!    `dispatch_key_press`）—— 这样"规范里写了什么"与"线上叫什么"可以逐字对账。
//!    ⚠ **一条例外（本切片新增）**：`ui/set_track_height`。规范**没有**给它名字与签名 ——
//!    它是 `ADR-0004` S1（轨道高度）的纵向入口在控制面上的工程裁决；命名与参数沿用本节的
//!    三条规则，但"逐字来自 §12.4"这句话对它不成立（原文写在该条的
//!    [`MethodSpec::signature`] 里）；
//! 3. **参数用 camelCase**（`elementId` / `xOffset` / `keyCode`），与领域契约
//!    `schemas/mcp-tools.schema.json` 的既有风格一致（`idempotencyKey` / `sectionName`）。
//!    规范 §12.4 写的是 Rust 侧形参名 `element_id`，两者的对应关系写在每条方法的
//!    [`MethodSpec::signature`] 里，**不是**两份名字各自漂移。
//!
//! ## scope 与三级权限（[UI-MCP-001]）的关系
//!
//! §12.3 的 `ReadOnly` / `Interactive` / `Administrative` 住在
//! `yeban_ui_test_port::port::Permission`（进程内调用面），而**网络**上的授权粒度是
//! `yeban_mcp::security::Scope` 的六级（`ARCH-SEC-002`）。两者不是同一层，因此每条方法
//! 同时声明：
//!
//! - [`MethodSpec::scope`]：**网络层**要求的作用域（本线判定的对象）；
//! - [`MethodSpec::port_operation`]：对应的**进程内**操作（由 `yeban-ui-test-port` 再判一次）。
//!
//! 于是"用 `read` 的 scope 调注入"与"用 `Administrative` 的权限实现一个 `ui:read` 方法"
//! 都会在**两层中的某一层**被拒 —— 这是纵深防御，不是冗余。判据
//! `port_operation_tier_matches_scope` 钉住两张表不许漂移。

use serde_json::{Map, Value};

use yeban_mcp::security::Scope;
use yeban_ui_test_port::port::Operation;

use crate::dry_run::DRY_RUN_PARAM;
use crate::ime::IME_FIELD;

/// 方法名：能力发现（列出本服务的方法集）。
pub const METHOD_METHODS: &str = "ui/methods";
/// 方法名：读控件树。
pub const METHOD_TREE: &str = "ui/tree";
/// 方法名：按语义 ID 查单个节点（`[UI-TEST-001]` §12.2）。
pub const METHOD_NODE: &str = "ui/node";
/// 方法名：读响应式属性（`[UI-MCP-001]` §12.3 只读层第三件事）。
pub const METHOD_PROPERTY: &str = "ui/property";
/// 方法名：列出动态区（`[UI-MCP-002]` §12.5）。
pub const METHOD_DYNAMIC_REGIONS: &str = "ui/dynamic_regions";
/// 方法名：抓 Tier-1 截图（`[MUST-GATE-015]`）。
pub const METHOD_SCREENSHOT: &str = "ui/screenshot";
/// 方法名：运行时树与注册表的双向覆盖（`[UI-TEST-001]`）。
pub const METHOD_COVERAGE: &str = "ui/coverage";
/// 方法名：§12.4 的 `dispatch_pointer_down`。
pub const METHOD_DISPATCH_POINTER_DOWN: &str = "ui/dispatch_pointer_down";
/// 方法名：§12.4 的 `dispatch_pointer_move`。
pub const METHOD_DISPATCH_POINTER_MOVE: &str = "ui/dispatch_pointer_move";
/// 方法名：§12.4 的 `dispatch_pointer_up`。
pub const METHOD_DISPATCH_POINTER_UP: &str = "ui/dispatch_pointer_up";
/// 方法名：§12.4 的 `dispatch_key_press`。
pub const METHOD_DISPATCH_KEY_PRESS: &str = "ui/dispatch_key_press";
/// 方法名：§12.3 `Administrative` 的"切换工作区主视图"。
pub const METHOD_SWITCH_MAIN_VIEW: &str = "ui/switch_main_view";
/// 方法名：§12.3 `Administrative` 的"强制执行工程保存"。
pub const METHOD_FORCE_SAVE: &str = "ui/force_save";
/// 方法名：§12.3 `Administrative` 的"重载音频引擎"。
pub const METHOD_RELOAD_ENGINE: &str = "ui/reload_engine";
/// 方法名：`ADR-0004` S1 的**纵向入口**（轨道头拖拽手势）在控制面上的孪生。
///
/// 规范里**没有**这个方法名与签名 —— 它是工程裁决（理由与口径写在方法表的 `signature` 里）。
pub const METHOD_SET_TRACK_HEIGHT: &str = "ui/set_track_height";

/// 方法名：CLI `--open` 在控制面上的孪生 —— **在正在运行的执行面上换一份当前工程**。
///
/// 规范里**没有**这个方法名与签名 —— 它是工程裁决（理由与口径写在方法表的 `signature` 里），
/// 与 [`METHOD_SET_TRACK_HEIGHT`] 同一族。它补的缺口是：产品/控制面**在进程已经跑起来之后**
/// 无法换工程（`grep -rn "ui/open_project" crates/` 在本切片之前 0 命中）。
pub const METHOD_OPEN_PROJECT: &str = "ui/open_project";

/// 「未知参数一律拒绝」的例外前缀（MCP 保留键），与 `yeban-mcp` 的口径一致。
///
/// 静默忽略一个拼错的参数会让调用方以为自己的意图生效了 —— 那是最坏的一种"成功"
/// （`docs/ledger/mcp-core-notes.md` §2 M9）。
pub const RESERVED_PARAM_PREFIX: char = '_';

/// 一个参数声明。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamSpec {
    /// 线格式名字（camelCase）。
    pub name: &'static str,
    /// JSON 类型（`string` / `integer` / `number` / `boolean` / `array` / `object`）。
    pub json_type: &'static str,
    /// 是否必填。
    pub required: bool,
    /// 取值白名单（`None` = 不限制；只对字符串参数用）。
    pub allowed: Option<&'static [&'static str]>,
    /// 说明（进 `ui/methods` 的 `description`）。
    pub description: &'static str,
}

/// 一条方法声明。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MethodSpec {
    /// 线格式方法名。
    pub name: &'static str,
    /// 本方法实现的规范 ID（必须都能在 `docs/YEBAN_*.md` 里 grep 到）。
    pub spec_ids: &'static [&'static str],
    /// **网络层**要求的作用域（[`Scope`]）。
    pub scope: Scope,
    /// **进程内**对应的操作（由 `yeban-ui-test-port` 再判一次）；`None` = 该动作没有端口操作。
    pub port_operation: Option<Operation>,
    /// 参数声明（顺序 = 文档顺序；校验时不区分顺序）。
    pub params: &'static [ParamSpec],
    /// 规范里的原始签名 / 措辞（让"参数名从哪来"可逐字复核）。
    pub signature: &'static str,
    /// 一句话说明。
    pub description: &'static str,
    /// 是否属于"改变界面/工程状态"的一类（`true` 的都必须有 scope 兜着）。
    pub mutating: bool,
}

const NO_PARAMS: &[ParamSpec] = &[];
const PREFIX: ParamSpec = ParamSpec {
    name: "prefix",
    json_type: "string",
    required: false,
    allowed: None,
    description: "只返回语义 ID 以该前缀开头的节点（§12.2 的族查询, 例如 `track-`）",
};
const TREE_SOURCE: ParamSpec = ParamSpec {
    name: "source",
    json_type: "string",
    required: false,
    allowed: Some(&["runtime"]),
    description: "树来源; 目前只提供 `runtime`（静态注册表由调用方自己在本地投影）",
};
const DYNAMIC_ONLY: ParamSpec = ParamSpec {
    name: "dynamicOnly",
    json_type: "boolean",
    required: false,
    allowed: None,
    description: "只返回 `[UI-MCP-002]` 的高频刷新区",
};
const ELEMENT_ID: ParamSpec = ParamSpec {
    name: "elementId",
    json_type: "string",
    required: true,
    allowed: None,
    description: "目标语义 ID（§12.2 的四个族之一）",
};
const PROPERTY_NAME: ParamSpec = ParamSpec {
    name: "name",
    json_type: "string",
    required: true,
    allowed: None,
    description: "属性名（`yeban_ui_test_port::inspect::property_of` 支持的清单 —— 含 `[ARCH-UI-004]` 的 `value` = `accessible-value` 原文与 `checked` = `accessible-checked` 的 \"true\"/\"false\"; 这两个是**可选**属性, 元素没声明时如实报错, 而 `ui/node` 的对应字段是 null + 本控制面的虚拟属性 `isComposing` = `[UI-A11Y-002]` 的 IME 合成态, 返回原生布尔）",
};
const MASK_DYNAMIC: ParamSpec = ParamSpec {
    name: "maskDynamic",
    json_type: "boolean",
    required: false,
    allowed: None,
    description: "比对前是否按 `[UI-MCP-002]` 把动态区置黑（**默认 true**）",
};
const MAX_BYTES: ParamSpec = ParamSpec {
    name: "maxBytes",
    json_type: "integer",
    required: false,
    allowed: None,
    description: "PNG 体积上限（默认 10 MiB = 仓库单文件上限）",
};
const REGISTRY_IDS: ParamSpec = ParamSpec {
    name: "ids",
    json_type: "array",
    required: true,
    allowed: None,
    description: "静态注册表的语义 ID 清单（用于双向覆盖核对）",
};
const X_OFFSET: ParamSpec = ParamSpec {
    name: "xOffset",
    json_type: "number",
    required: true,
    allowed: None,
    description: "元素内 x 偏移（§12.4 的 `x_offset`）",
};
const Y_OFFSET: ParamSpec = ParamSpec {
    name: "yOffset",
    json_type: "number",
    required: true,
    allowed: None,
    description: "元素内 y 偏移（§12.4 的 `y_offset`）",
};
const X_ABS: ParamSpec = ParamSpec {
    name: "x",
    json_type: "number",
    required: true,
    allowed: None,
    description: "窗口坐标 x（§12.4 的 `x`）",
};
const Y_ABS: ParamSpec = ParamSpec {
    name: "y",
    json_type: "number",
    required: true,
    allowed: None,
    description: "窗口坐标 y（§12.4 的 `y`）",
};
const BUTTON: ParamSpec = ParamSpec {
    name: "button",
    json_type: "string",
    required: true,
    allowed: Some(&["left", "middle", "right", "other"]),
    description: "指针按键（§12.4 的 `button`）",
};
const KEY_CODE: ParamSpec = ParamSpec {
    name: "keyCode",
    json_type: "string",
    required: true,
    allowed: None,
    description: "键名（§12.4 的 `key_code`；解析口径 = `yeban_ui_test_port::port::KeyCode::parse`）",
};
const HEIGHT_PX: ParamSpec = ParamSpec {
    name: "heightPx",
    json_type: "integer",
    required: true,
    allowed: None,
    description: "这条轨道的**基准**行高（整数逻辑像素；`0` = 取消这条覆盖，回到默认 56）。\
                  生效值仍是投影的 `clamp(基准 × 百分比 / 100, 16, 320)`（无效的负值报 `-32602`）",
};
const VIEW: ParamSpec = ParamSpec {
    name: "view",
    json_type: "string",
    required: true,
    // 白名单 = 两条主视图（`app.slint` 的 `arrangement-view` 是一个 `bool`，只有两个取值）。
    // 非法值由**参数校验**拒绝 ⇒ `-32602 INVALID_PARAMS`，而不是执行面的
    // `-32005 NOT_IMPLEMENTED`（那是"能力没接线"）。两者混在一起会让调用方以为
    // 控制面没实现主视图切换 —— 而它已经实现了（`crates/yeban-app/src/live_surface.rs`）。
    allowed: Some(&["arrangement", "session"]),
    description: "目标主视图名（`arrangement` = 线性编曲 / `session` = 触发矩阵）",
};
/// `ui/open_project` 的目标路径（`path`，必填）。
///
/// 取值不是白名单：它是一个**文件系统路径**，合法性只能由执行面自己判
/// （`crates/yeban-app/src/open.rs` 的 `open_project_file`：读失败 / 超上限 / 容器拒绝）。
/// 与 `yeban_open_project` 的 `path` 同名同义（`crates/yeban-mcp/src/domain/mod.rs:1620`）。
const PATH: ParamSpec = ParamSpec {
    name: "path",
    json_type: "string",
    required: true,
    allowed: None,
    description: "要打开的 `.yeban` 容器路径；不存在 / 不是容器 / 容器坏了 ⇒ 如实报错，\
                  当前工程**一位不动**",
};
/// `ui/open_project` 的 `saveFirst`（可选布尔，**默认 true**）。
///
/// 默认值与语义都取自既有工具 `yeban_close_project`（`crates/yeban-mcp/src/domain/mod.rs:1813`
/// 的 `arg_bool(call, "saveFirst", true)`）：**先保存当前工程，再放开它**。
/// 这里同理：换工程会丢弃内存里那一份，因此默认先把旧工程落盘到它自己的路径。
/// 没有落点时（样本形态 / 从没保存过）**必须显式给 `false`** —— 否则拒绝，
/// 不会静默丢弃用户的未保存改动。
const SAVE_FIRST: ParamSpec = ParamSpec {
    name: "saveFirst",
    json_type: "boolean",
    required: false,
    allowed: None,
    description: "换工程之前先把当前工程落盘到它自己的路径（默认 true，与 `yeban_close_project` \
                  的 `saveFirst` 同义）；当前工程没有落点时必须显式给 false（拒绝静默丢弃）",
};
/// **`dryRun`（先问后做）** 的参数声明 —— ADR-0001 **D48**。
///
/// 名字取自 [`DRY_RUN_PARAM`]，而它是 `yeban_mcp::tools::DRY_RUN_PARAM` 的**再导出**
/// （`crates/yeban-mcp/src/tools.rs:37`）⇒ "同一个词"由编译器保证，不是靠注释约定。
/// 默认 `false`，与领域侧逐字一致（判据 [crate::dry_run] 的
/// `dry_run_defaults_to_false_exactly_like_the_domain`）。
///
/// **只出现在会改状态的方法上**（`mutating == true` 的 9 条）：只读方法收到它会被
/// 未知参数规则**响亮拒绝**（`-32602`），而不是静默忽略 —— 理由见 [crate::dry_run] 的模块头。
const DRY_RUN: ParamSpec = ParamSpec {
    name: DRY_RUN_PARAM,
    json_type: "boolean",
    required: false,
    allowed: None,
    description: "只读模拟校验（默认 false）: 与 `yeban_*` 工具同义 —— 只回报**将要发生什么**, \
                  不改任何状态、不消耗任何权限/幂等键",
};

/// 全部方法，**注册表顺序 = 文档顺序**（不依赖任何容器迭代顺序）。
pub const METHODS: [MethodSpec; 16] = [
    MethodSpec {
        name: METHOD_METHODS,
        spec_ids: &["ARCH-UI-004", "MCP-DUAL-001"],
        scope: Scope::UiRead,
        port_operation: None,
        params: NO_PARAMS,
        signature: "架构 §1.3 [ARCH-UI-004]: AI Agent 可通过 JSON-RPC 查询控件树…",
        description: "列出本控制面的方法集、所需 scope 与参数（能力发现）",
        mutating: false,
    },
    MethodSpec {
        name: METHOD_TREE,
        spec_ids: &["UI-MCP-001", "ARCH-UI-004", "UI-TEST-001"],
        scope: Scope::UiRead,
        port_operation: Some(Operation::ReadTree),
        params: &[PREFIX, TREE_SOURCE, DYNAMIC_ONLY],
        signature: "UI/UX §12.3 [UI-MCP-001] ReadOnly: 仅允许控件树结构检索…",
        description: "读语义控件树（稳定 JSON：节点按语义 ID 升序）",
        mutating: false,
    },
    MethodSpec {
        name: METHOD_NODE,
        spec_ids: &["UI-TEST-001"],
        scope: Scope::UiRead,
        port_operation: Some(Operation::ReadTree),
        params: &[ELEMENT_ID],
        signature: "UI/UX §12.2 [UI-TEST-001]: 必须严格基于语义 Element ID 检索",
        description: "按语义 ID 查单个节点；找不到时报 ELEMENT_NOT_FOUND（不返回空成功）",
        mutating: false,
    },
    MethodSpec {
        name: METHOD_PROPERTY,
        spec_ids: &["UI-MCP-001", "ARCH-UI-004"],
        scope: Scope::UiRead,
        port_operation: Some(Operation::ReadProperty),
        params: &[ELEMENT_ID, PROPERTY_NAME],
        signature: "UI/UX §12.3 [UI-MCP-001] ReadOnly: 响应式属性读取",
        description: "读一个响应式属性（值以字符串返回, 不把 UI 框架类型泄进协议）",
        mutating: false,
    },
    MethodSpec {
        name: METHOD_DYNAMIC_REGIONS,
        spec_ids: &["UI-MCP-002"],
        scope: Scope::UiRead,
        port_operation: Some(Operation::ReadTree),
        params: NO_PARAMS,
        signature: "UI/UX §12.5 [UI-MCP-002]: 根据元素树元数据自动获取高频刷新组件的矩形包围盒",
        description: "列出 `[UI-MCP-002]` 的动态区及其**运行时包围盒**",
        mutating: false,
    },
    MethodSpec {
        name: METHOD_SCREENSHOT,
        spec_ids: &[
            "UI-MCP-001",
            "UI-MCP-002",
            "MUST-GATE-015",
            "ARCH-SLINT-001",
        ],
        scope: Scope::UiScreenshot,
        port_operation: Some(Operation::CaptureScreenshot),
        params: &[MASK_DYNAMIC, MAX_BYTES],
        signature: "UI/UX §12.5: 调用渲染后端的 Framebuffer 捕获接口, 编码为 PNG 二进制流并返回",
        description: "抓 Tier-1 帧缓冲并返回 base64 PNG + 像素证据（尺寸非零且非全黑）",
        mutating: false,
    },
    MethodSpec {
        name: METHOD_COVERAGE,
        spec_ids: &["UI-TEST-001", "ARCH-UI-005"],
        scope: Scope::UiRead,
        port_operation: Some(Operation::ReadTree),
        params: &[REGISTRY_IDS],
        signature: "UI/UX §12.2 [UI-TEST-001] + 架构 §1.3 [ARCH-UI-005] (Testing Backend 的行为边界)",
        description: "运行时树与静态注册表 ID 清单的双向覆盖核对",
        mutating: false,
    },
    MethodSpec {
        name: METHOD_DISPATCH_POINTER_DOWN,
        spec_ids: &["UI-TEST-002", "UI-MCP-001"],
        scope: Scope::UiInject,
        port_operation: Some(Operation::DispatchPointer),
        params: &[ELEMENT_ID, X_OFFSET, Y_OFFSET, BUTTON, DRY_RUN],
        signature: "UI/UX §12.4: dispatch_pointer_down(element_id, x_offset, y_offset, button)",
        description: "在目标语义元素内的指定偏移处注入鼠标按下",
        mutating: true,
    },
    MethodSpec {
        name: METHOD_DISPATCH_POINTER_MOVE,
        spec_ids: &["UI-TEST-002", "UI-MCP-001"],
        scope: Scope::UiInject,
        port_operation: Some(Operation::DispatchPointer),
        params: &[X_ABS, Y_ABS, DRY_RUN],
        signature: "UI/UX §12.4: dispatch_pointer_move(x, y)",
        description: "注入鼠标移动到窗口坐标",
        mutating: true,
    },
    MethodSpec {
        name: METHOD_DISPATCH_POINTER_UP,
        spec_ids: &["UI-TEST-002", "UI-MCP-001"],
        scope: Scope::UiInject,
        port_operation: Some(Operation::DispatchPointer),
        params: &[BUTTON, DRY_RUN],
        signature: "UI/UX §12.4: dispatch_pointer_up(button)",
        description: "注入鼠标释放（复用最后一次按下/移动的位置）",
        mutating: true,
    },
    MethodSpec {
        name: METHOD_DISPATCH_KEY_PRESS,
        spec_ids: &["UI-TEST-002", "UI-MCP-001", "UI-A11Y-002"],
        scope: Scope::UiInject,
        port_operation: Some(Operation::DispatchKey),
        params: &[KEY_CODE, DRY_RUN],
        signature: "UI/UX §12.4: dispatch_key_press(key_code)；UI/UX §7.2 [UI-A11Y-002]: \
                    合成态 (is_composing == true) 下彻底拦截单键快捷键的冒泡分发",
        description: "注入键盘按键（`Tab` / `Shift+Enter` / `Esc` 等）",
        mutating: true,
    },
    MethodSpec {
        name: METHOD_SWITCH_MAIN_VIEW,
        spec_ids: &["UI-MCP-001", "ARCH-UI-004"],
        scope: Scope::AppAdmin,
        port_operation: Some(Operation::SwitchMainView),
        params: &[VIEW, DRY_RUN],
        signature: "UI/UX §12.3 [UI-MCP-001] Administrative: 允许切换工作区主视图",
        description: "切换工作区主视图（Administrative 层）",
        mutating: true,
    },
    MethodSpec {
        name: METHOD_FORCE_SAVE,
        spec_ids: &["UI-MCP-001", "ARCH-SEC-002"],
        scope: Scope::AppSave,
        port_operation: Some(Operation::ForceSave),
        params: &[DRY_RUN],
        signature: "UI/UX §12.3 [UI-MCP-001] Administrative: 强制执行工程保存",
        description: "强制执行工程保存（Administrative 层；scope `app:save`）",
        mutating: true,
    },
    MethodSpec {
        name: METHOD_SET_TRACK_HEIGHT,
        spec_ids: &["UI-MCP-001", "UI-TEST-001", "ARCH-UI-004"],
        // `Interactive` 层（`ui:inject`）：它改的是**视图态**（行高），与事件注入同层；
        // 生产模式下 `ui:inject` 族被硬禁（与 `ui/dispatch_*` 逐字相同），因此这条入口
        // 只在测试 / 交互层可用 —— 产品窗口里的用户入口是**拖拽手势**本身。
        scope: Scope::UiInject,
        port_operation: Some(Operation::SetTrackHeight),
        params: &[ELEMENT_ID, HEIGHT_PX, DRY_RUN],
        signature: "规范里没有这个方法名与签名 —— 它是 `ADR-0004` S1（轨道高度）的纵向入口在\
                    控制面上的工程裁决；寻址沿用 §12.2 [UI-TEST-001] 的语义 ID（`track-{i}-header`），\
                    参数名沿用 §12.3 [UI-MCP-001] 的 camelCase 风格，权限落在 Interactive 层",
        description: "设置一条轨道的基准行高（`0` = 取消覆盖）；回执里给出回读的行几何",
        mutating: true,
    },
    MethodSpec {
        name: METHOD_RELOAD_ENGINE,
        spec_ids: &["UI-MCP-001", "ARCH-SEC-002"],
        scope: Scope::AppReloadEngine,
        port_operation: Some(Operation::ReloadEngine),
        params: &[DRY_RUN],
        signature: "UI/UX §12.3 [UI-MCP-001] Administrative: 重载音频引擎",
        description: "重载音频引擎（Administrative 层；scope `app:reload-engine`）",
        mutating: true,
    },
    MethodSpec {
        name: METHOD_OPEN_PROJECT,
        spec_ids: &["UI-MCP-001", "ARCH-SEC-002", "ARCH-UI-004"],
        // `Administrative` 层：换的是**工程整体**（轨道集合 / 走带 / 路由图），
        // 随后要换注册表、作废旧电平、重建引擎。与只改视图态的 `ui/set_track_height`
        // 不是一个量级，因此不落在 `ui:inject`。
        //
        // 六级 scope 里**没有**"换工程"这一级（`ADR-0001` **D29** 已把"没有界面状态写这一级"
        // 登记为待人类裁决），因此按 D29 的既有口径把管理类动作挂在 `app:admin` 上
        // （与 `ui/switch_main_view` 同一个 scope）。
        scope: Scope::AppAdmin,
        port_operation: Some(Operation::OpenProject),
        params: &[PATH, SAVE_FIRST, DRY_RUN],
        signature: "规范里没有这个方法名与签名 —— 它是 CLI `--open`（`crates/yeban-app/src/open.rs`）\
                    在控制面上的工程裁决：在**正在运行的**执行面上换一份当前工程。参数名沿用 \
                    §12.3 [UI-MCP-001] 的 camelCase 风格（`path` 与 `yeban_open_project` 同名同义；\
                    `saveFirst` 与 `yeban_close_project` 同名同义、默认值也相同），\
                    权限沿用 D29 给管理类动作的 `app:admin`",
        description: "在正在运行的执行面上换一份当前工程（`path`；`saveFirst` 默认 true）；\
                      回执里给出回读的轨道数与界面行数（长度契约为证）",
        mutating: true,
    },
];

/// 方法总数（注册表长度，供判据与文档引用）。
pub const METHOD_COUNT: usize = METHODS.len();

impl MethodSpec {
    /// 本方法是否声明了 **`dryRun`**（ADR-0001 **D48**）。
    ///
    /// **从参数表派生**（`params` 里有 [`DRY_RUN_PARAM`] 就是有），不另建一张"哪些方法支持
    /// dryRun"的表 —— 两张表必然会漂移，而漂移的方向恰好是"`ui/methods` 说有、执行分支说没有"。
    ///
    /// 判据 `every_mutating_method_declares_dry_run_and_no_read_only_method_does` 钉住
    /// `supports_dry_run() == mutating`。
    #[must_use]
    pub fn supports_dry_run(&self) -> bool {
        self.params.iter().any(|param| param.name == DRY_RUN_PARAM)
    }
}

/// 按名字取方法声明。
#[must_use]
pub fn method(name: &str) -> Option<&'static MethodSpec> {
    METHODS.iter().find(|spec| spec.name == name)
}

/// 全部方法名（注册表顺序；判据用它做集合相等）。
#[must_use]
pub fn names() -> Vec<&'static str> {
    METHODS.iter().map(|spec| spec.name).collect()
}

/// 参数校验错误（人话，进 JSON-RPC 错误对象的 `data.detail`）。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{detail}")]
pub struct ParamError {
    /// 说明。
    pub detail: String,
}

/// 校验并归一化参数。
///
/// 规则（每条都有判据）：
///
/// 1. `params` 缺省 = 空对象；**数组形式一律拒绝**（MCP 的位置参数在 UI 控制面上没有意义，
///    而且静默接受会让"传错了形状"变成一次成功的调用）；
/// 2. **未知参数一律拒绝**，`_` 前缀的 MCP 保留键除外（M9 口径）；
/// 3. 必填缺失 / 类型不符 / 不在白名单 ⇒ 拒绝，且说明里点名是哪一个。
///
/// # Errors
///
/// 上述任一条成立。
pub fn validate_params(
    spec: &'static MethodSpec,
    params: Option<&Value>,
) -> Result<Map<String, Value>, ParamError> {
    let reject = |detail: String| Err(ParamError { detail });
    let object = match params {
        None => Map::new(),
        Some(Value::Object(map)) => map.clone(),
        Some(Value::Array(_)) => {
            return reject(format!(
                "`{}` 只接受**对象**形式的 params（收到数组）",
                spec.name
            ));
        }
        Some(other) => {
            return reject(format!(
                "`{}` 的 params 必须是对象, 收到 {}",
                spec.name,
                type_name(other)
            ));
        }
    };

    for key in object.keys() {
        if key.starts_with(RESERVED_PARAM_PREFIX) {
            continue;
        }
        if !spec.params.iter().any(|param| param.name == key) {
            return reject(format!(
                "`{}` 不接受参数 `{key}`（未知参数被拒绝, \
                 免得拼错的参数被静默忽略）; 合法参数: {}",
                spec.name,
                legal_params(spec)
            ));
        }
    }

    for param in spec.params {
        match object.get(param.name) {
            None => {
                if param.required {
                    return reject(format!(
                        "`{}` 缺少必填参数 `{}`（类型 {}）",
                        spec.name, param.name, param.json_type
                    ));
                }
            }
            Some(value) => {
                if !json_type_matches(param.json_type, value) {
                    return reject(format!(
                        "`{}` 的参数 `{}` 类型不符: 期望 {}, 收到 {}",
                        spec.name,
                        param.name,
                        param.json_type,
                        type_name(value)
                    ));
                }
                if let (Some(allowed), Some(text)) = (param.allowed, value.as_str())
                    && !allowed.contains(&text)
                {
                    return reject(format!(
                        "`{}` 的参数 `{}` 取值非法: `{text}`（合法值: {}）",
                        spec.name,
                        param.name,
                        allowed.join(", ")
                    ));
                }
            }
        }
    }
    Ok(object)
}

/// 合法参数清单（错误信息里用，顺序 = 声明顺序）。
fn legal_params(spec: &MethodSpec) -> String {
    if spec.params.is_empty() {
        return "<无>".to_owned();
    }
    spec.params
        .iter()
        .map(|param| param.name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// `true` 表示 `value` 满足声明的 JSON 类型。
///
/// `integer` 接受任何整数值的 `Number`（`1.0` 也算 —— JSON 里没有整数/浮点的类型区分，
/// 只有"这个数的值是不是整数"）；`number` 接受任何 `Number`。
#[must_use]
pub fn json_type_matches(json_type: &str, value: &Value) -> bool {
    match json_type {
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.as_f64().is_some_and(|number| number.is_finite()),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => false,
    }
}

/// JSON 值的类型名（错误信息里用）。
#[must_use]
pub fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// `ui/methods` 的返回载荷：方法注册表的机器可读快照。
///
/// 顶层**没有** `name` / `arguments` / `status`（那是 `schemas/mcp-tools.schema.json`
/// 的契约实例判别键）—— 本对象是**文档/清单**，不是领域契约实例。因此样本导出时它必须
/// 叫 `*.meta.json`（`.meta.` 约定，见 [`crate::samples`] 与
/// `docs/ledger/mcp-core-notes.md` §2 M12/M13）。
#[must_use]
pub fn catalogue() -> Value {
    let methods = METHODS
        .iter()
        .map(|spec| {
            let mut tool = Map::new();
            tool.insert("name".to_owned(), Value::from(spec.name));
            tool.insert(
                "specIds".to_owned(),
                Value::Array(spec.spec_ids.iter().map(|id| Value::from(*id)).collect()),
            );
            tool.insert("requiredScope".to_owned(), Value::from(spec.scope.as_str()));
            tool.insert(
                "portOperation".to_owned(),
                Value::from(spec.port_operation.map_or("<none>", Operation::as_str)),
            );
            tool.insert("mutating".to_owned(), Value::from(spec.mutating));
            // 能力发现（ADR-0001 D48 的可发现性要求）：调用方**不用读文档**就能知道
            // 哪些方法支持"先问后做"。字段名与领域侧 `tools/list` 的
            // `annotations.dryRunSupported` **逐字相同**（`crates/yeban-mcp/src/tools.rs:636`）。
            tool.insert(
                "dryRunSupported".to_owned(),
                Value::from(spec.supports_dry_run()),
            );
            tool.insert("signature".to_owned(), Value::from(spec.signature));
            tool.insert("description".to_owned(), Value::from(spec.description));
            tool.insert(
                "params".to_owned(),
                Value::Array(
                    spec.params
                        .iter()
                        .map(|param| {
                            let mut entry = Map::new();
                            entry.insert("name".to_owned(), Value::from(param.name));
                            entry.insert("type".to_owned(), Value::from(param.json_type));
                            entry.insert("required".to_owned(), Value::from(param.required));
                            if let Some(allowed) = param.allowed {
                                entry.insert(
                                    "allowed".to_owned(),
                                    Value::Array(
                                        allowed.iter().map(|value| Value::from(*value)).collect(),
                                    ),
                                );
                            }
                            entry.insert("description".to_owned(), Value::from(param.description));
                            Value::Object(entry)
                        })
                        .collect(),
                ),
            );
            Value::Object(tool)
        })
        .collect::<Vec<_>>();

    let mut root = Map::new();
    root.insert("service".to_owned(), Value::from("yeban-ui-mcp"));
    root.insert("methodCount".to_owned(), Value::from(METHOD_COUNT));
    root.insert(
        "reservedParamPrefix".to_owned(),
        Value::from(RESERVED_PARAM_PREFIX.to_string()),
    );
    // 与领域侧注册表样本的 `dryRunParam` 同名同义（`crates/yeban-mcp/src/samples.rs:138`）。
    root.insert("dryRunParam".to_owned(), Value::from(DRY_RUN_PARAM));
    // `[UI-A11Y-002]` 的 IME 合成态（`ui/property` 的虚拟属性名）。
    root.insert("imeProperty".to_owned(), Value::from(IME_FIELD));
    root.insert("methods".to_owned(), Value::Array(methods));
    Value::Object(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ime::VIRTUAL_PROPERTIES;

    /// 判据 1: 方法名唯一、非空、形如 `ui/<小写下划线>`；注册表顺序 == 文档顺序。
    #[test]
    fn method_names_are_unique_and_well_formed() {
        let names = names();
        assert_eq!(names.len(), METHOD_COUNT);
        assert_eq!(METHOD_COUNT, 16);
        let unique: std::collections::BTreeSet<&str> = names.iter().copied().collect();
        assert_eq!(unique.len(), names.len(), "方法名不得重复");

        for spec in &METHODS {
            let rest = spec
                .name
                .strip_prefix("ui/")
                .unwrap_or_else(|| panic!("方法 `{}` 必须以 `ui/` 开头", spec.name));
            assert!(!rest.is_empty());
            assert!(
                rest.bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'_'),
                "方法名主体必须是小写 + 下划线: {}",
                spec.name
            );
            assert_eq!(method(spec.name).map(|found| found.name), Some(spec.name));
            assert!(!spec.spec_ids.is_empty(), "{} 必须挂规范 ID", spec.name);
            assert!(!spec.description.is_empty());
            assert!(!spec.signature.is_empty());
        }
        assert!(method("ui/nope").is_none());
        assert!(method("tools/list").is_none(), "不得与领域 MCP 撞名");
    }

    /// 判据 2: **两条权限表的 tier 必须一致** —— scope 与进程内 `Operation` 不许漂移。
    ///
    /// 注入验证（把某条注入方法的 scope 改成 `ui:read`）会让本判据变红。
    #[test]
    fn port_operation_tier_matches_scope() {
        for spec in &METHODS {
            match spec.scope {
                Scope::UiRead => assert!(
                    matches!(
                        spec.port_operation,
                        None | Some(Operation::ReadTree | Operation::ReadProperty)
                    ),
                    "{} 是只读 scope 却挂了写操作 {:?}",
                    spec.name,
                    spec.port_operation
                ),
                Scope::UiScreenshot => assert_eq!(
                    spec.port_operation,
                    Some(Operation::CaptureScreenshot),
                    "{}",
                    spec.name
                ),
                Scope::UiInject => assert!(
                    matches!(
                        spec.port_operation,
                        Some(
                            Operation::DispatchPointer
                                | Operation::DispatchKey
                                | Operation::SetTrackHeight
                        )
                    ),
                    "{} 是注入 scope 却没挂注入操作",
                    spec.name
                ),
                Scope::AppSave => assert_eq!(spec.port_operation, Some(Operation::ForceSave)),
                Scope::AppReloadEngine => {
                    assert_eq!(spec.port_operation, Some(Operation::ReloadEngine));
                }
                Scope::AppAdmin => {
                    // `app:admin` 是**管理类动作**的挂靠 scope（`ADR-0001` D29：六级 scope 里
                    // 没有"界面状态写"与"换工程"这两级）。它今天挂着两条方法：切换主视图与
                    // `ui/open_project`。断言写成**闭集**（枚举变体）而不是"什么操作都行"
                    // —— 一张挂错 scope 的表仍然会红。
                    assert!(
                        matches!(
                            spec.port_operation,
                            Some(Operation::SwitchMainView | Operation::OpenProject)
                        ),
                        "{} 是 app:admin 却挂了 {:?}",
                        spec.name,
                        spec.port_operation
                    );
                }
            }
            assert_eq!(
                spec.mutating,
                !matches!(spec.scope, Scope::UiRead | Scope::UiScreenshot),
                "{} 的 `mutating` 必须与 scope 一致",
                spec.name
            );
        }
    }

    /// 判据 3: §12.4 点名的四个注入方法**逐字**存在，且参数与规范签名一一对应
    /// （`dryRun` 是 ADR-0001 D48 追加的**可选**公共参数，排在规范参数之后）。
    #[test]
    fn spec_event_injection_methods_match_section_12_4() {
        for (name, params, signature_fragment) in [
            (
                METHOD_DISPATCH_POINTER_DOWN,
                &["elementId", "xOffset", "yOffset", "button", "dryRun"][..],
                "dispatch_pointer_down(element_id, x_offset, y_offset, button)",
            ),
            (
                METHOD_DISPATCH_POINTER_MOVE,
                &["x", "y", "dryRun"][..],
                "dispatch_pointer_move(x, y)",
            ),
            (
                METHOD_DISPATCH_POINTER_UP,
                &["button", "dryRun"][..],
                "dispatch_pointer_up(button)",
            ),
            (
                METHOD_DISPATCH_KEY_PRESS,
                &["keyCode", "dryRun"][..],
                "dispatch_key_press(key_code)",
            ),
        ] {
            let spec = method(name).expect("§12.4 的方法必须在注册表里");
            assert_eq!(spec.scope, Scope::UiInject, "{name} 必须是 ui:inject");
            assert!(
                spec.signature.contains(signature_fragment),
                "{name} 的 signature 必须逐字引用规范: {}",
                spec.signature
            );
            assert_eq!(
                spec.params
                    .iter()
                    .map(|param| param.name)
                    .collect::<Vec<_>>(),
                params,
                "{name} 的参数必须与规范签名一一对应（dryRun 排最后）"
            );
            assert!(
                spec.params
                    .iter()
                    .filter(|param| param.name != DRY_RUN_PARAM)
                    .all(|param| param.required),
                "{name}: 规范点名的参数全部必填; 只有 dryRun 可选"
            );
            assert!(spec.supports_dry_run(), "{name} 会改状态 ⇒ 必须有 dryRun");
        }
    }

    /// 判据 6（**ADR-0001 D48**）: `dryRun` 的**可发现性**与**适用范围**。
    ///
    /// - 会改状态的 8 条方法**全部**声明 `dryRun`，且它是**可选布尔**；
    /// - 只读的 7 条方法**一条都不声明**（只读没有副作用可短路，见 [`crate::dry_run`]）；
    /// - `ui/methods` 的自描述里能**直接看出**哪些方法支持它（`dryRunSupported` +
    ///   顶层 `dryRunParam`），调用方不必读文档。
    ///
    /// 注入验证（把 `ui/force_save` 的 `params` 改回 `NO_PARAMS`，或把
    /// `supports_dry_run` 改成 `true`）会让本判据变红。
    #[test]
    fn every_mutating_method_declares_dry_run_and_no_read_only_method_does() {
        let mut with = Vec::new();
        let mut without = Vec::new();
        for spec in &METHODS {
            assert_eq!(
                spec.supports_dry_run(),
                spec.mutating,
                "{}: `mutating` 与 `dryRun` 支持必须同时成立（D48 只给会改状态的方法）",
                spec.name
            );
            if spec.mutating {
                let param = spec
                    .params
                    .iter()
                    .find(|param| param.name == DRY_RUN_PARAM)
                    .unwrap_or_else(|| panic!("{} 会改状态却没有 dryRun", spec.name));
                assert_eq!(param.json_type, "boolean", "{}", spec.name);
                assert!(
                    !param.required,
                    "{} 的 dryRun 必须可选（默认 false）",
                    spec.name
                );
                assert!(param.allowed.is_none(), "{}: 布尔参数不做白名单", spec.name);
                with.push(spec.name);
            } else {
                without.push(spec.name);
            }
        }
        assert_eq!(with.len(), 9, "会改状态的方法: {with:?}");
        assert_eq!(without.len(), 7, "只读方法: {without:?}");
        assert_eq!(with.len() + without.len(), METHOD_COUNT);

        // 可发现性：`ui/methods` 的清单里逐条能看出支持与否。
        let snapshot = catalogue();
        assert_eq!(snapshot["dryRunParam"], DRY_RUN_PARAM);
        assert_eq!(snapshot["imeProperty"], IME_FIELD);
        let listed = snapshot["methods"].as_array().expect("数组");
        assert_eq!(listed.len(), METHOD_COUNT);
        for (spec, entry) in METHODS.iter().zip(listed) {
            assert_eq!(
                entry["dryRunSupported"],
                Value::from(spec.mutating),
                "{} 的可发现性旗标错了",
                spec.name
            );
            let declared = entry["params"]
                .as_array()
                .expect("数组")
                .iter()
                .any(|param| param["name"] == DRY_RUN_PARAM);
            assert_eq!(
                declared, spec.mutating,
                "{} 的参数表与旗标不一致",
                spec.name
            );
        }
    }

    /// 判据 7: 虚拟属性（IME）在 `ui/property` 的参数描述里**被点名** —— 否则
    /// 调用方只能靠猜"这个控制面支不支持读 `isComposing`"。
    ///
    /// 描述是 `&'static str`（不能在运行时 format），因此这里用**判据**把
    /// "描述必须列出每一个虚拟属性"钉住：把 `IME_FIELD` 改名而忘了改描述，本判据变红。
    #[test]
    fn property_name_description_lists_every_virtual_property() {
        let property = method(METHOD_PROPERTY).expect("注册表里有");
        let name = property
            .params
            .iter()
            .find(|param| param.name == "name")
            .expect("有 `name` 参数");
        for virtual_property in VIRTUAL_PROPERTIES {
            assert!(
                name.description.contains(virtual_property),
                "`ui/property` 的 `name` 描述必须点名虚拟属性 `{virtual_property}`: {}",
                name.description
            );
        }
    }

    /// 判据 4: 参数校验的四条规则 —— 数组拒绝、未知参数拒绝、必填缺失拒绝、类型/白名单拒绝；
    /// `_` 前缀的保留键放行。
    #[test]
    fn param_validation_rejects_every_malformed_shape() {
        let spec = method(METHOD_NODE).expect("存在");
        assert!(validate_params(spec, None).is_err(), "缺必填必须拒绝");
        assert!(
            validate_params(spec, Some(&serde_json::json!([1, 2]))).is_err(),
            "数组形式必须拒绝"
        );
        assert!(
            validate_params(spec, Some(&serde_json::json!("x"))).is_err(),
            "非对象必须拒绝"
        );
        assert!(
            validate_params(spec, Some(&serde_json::json!({"id": "track-0-fader"}))).is_err(),
            "拼错的参数名必须拒绝"
        );
        assert!(
            validate_params(spec, Some(&serde_json::json!({"elementId": 7}))).is_err(),
            "类型不符必须拒绝"
        );
        assert!(
            validate_params(
                spec,
                Some(&serde_json::json!({"_meta": {"trace": 1}, "elementId": "track-0-fader"}))
            )
            .is_ok(),
            "`_` 前缀的 MCP 保留键必须放行"
        );

        let down = method(METHOD_DISPATCH_POINTER_DOWN).expect("存在");
        let error = validate_params(
            down,
            Some(&serde_json::json!({
                "elementId": "track-0-fader", "xOffset": 1, "yOffset": 2, "button": "leftt"
            })),
        )
        .expect_err("白名单外的取值必须拒绝");
        assert!(error.detail.contains("leftt"), "{}", error.detail);
        assert!(
            validate_params(
                down,
                Some(&serde_json::json!({
                    "elementId": "track-0-fader", "xOffset": 1, "yOffset": 2.5, "button": "left"
                }))
            )
            .is_ok()
        );

        let screenshot = method(METHOD_SCREENSHOT).expect("存在");
        assert!(validate_params(screenshot, None).is_ok(), "全可选参数");
        assert!(
            validate_params(screenshot, Some(&serde_json::json!({"maxBytes": 1.5}))).is_err(),
            "integer 参数不接受非整数"
        );
        let coverage = method(METHOD_COVERAGE).expect("存在");
        assert!(
            validate_params(coverage, Some(&serde_json::json!({"ids": "track-0-fader"}))).is_err(),
            "array 参数不接受字符串"
        );
    }

    /// 判据 5: `ui/methods` 的载荷是**清单**而不是契约实例（顶层不含 `name`/`arguments`/`status`），
    /// 且 16 条方法与注册表逐条对应（顺序也一致）。
    #[test]
    fn catalogue_is_a_document_and_matches_the_registry() {
        let snapshot = catalogue();
        let object = snapshot.as_object().expect("对象");
        for forbidden in ["name", "arguments", "status"] {
            assert!(
                !object.contains_key(forbidden),
                "清单顶层不得出现契约实例判别键 `{forbidden}`"
            );
        }
        assert_eq!(object["service"], "yeban-ui-mcp");
        assert_eq!(object["methodCount"], METHOD_COUNT);
        let methods = snapshot["methods"].as_array().expect("数组");
        assert_eq!(methods.len(), METHOD_COUNT);
        for (spec, entry) in METHODS.iter().zip(methods) {
            assert_eq!(entry["name"], spec.name);
            assert_eq!(entry["requiredScope"], spec.scope.as_str());
            assert_eq!(
                entry["params"].as_array().expect("数组").len(),
                spec.params.len()
            );
        }
        // 两次构造必须逐字节相同（清单要能当稳定样本导出）。
        let again = catalogue();
        assert_eq!(snapshot, again);
    }
}
