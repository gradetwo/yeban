//! **真实界面上的** UI 控制面：装配 + 端到端读数（`ui/tree` → `ui/node` → `ui/screenshot`）。
//!
//! 规范来源 (Normative)：`[ARCH-UI-004]`（JSON-RPC 内省协议）、`[UI-TEST-001]`（语义寻址）、
//! `[UI-MCP-001]`（三级权限分层）、`[UI-MCP-002]`（动态区遮罩）、`[MUST-GATE-015]`
//! （Golden 必须由 Tier-1 软件光栅化产出，尺寸非零且非全黑）。
//!
//! ## 这个模块存在的理由（本工作线的核心）
//!
//! `crate::surface::PortAdapter` 是**接口**：它把"怎么拿像素"变成调用方注入的闭包，
//! 于是本 crate 保持零 Slint，可以在本机真跑。但接口本身不证明"AI 能在**活的
//! `MainWindow`** 上看见界面"。本模块补的就是那一步 —— 它把
//!
//! ```text
//!   UiSurface（真实窗口，见 yeban-app 的 src/live_surface.rs）
//!      └── UiService（鉴权 → 授权 → 参数 → 执行）
//!             └── 本模块的 ControlPlane：直接喂**线上形态的 JSON-RPC 文本**
//! ```
//!
//! 串成一条**可判据的**调用序列：[`ControlPlane::probe`]。它走的是
//! [`UiService::handle_line`]（原始文本入口，与 stdio / HTTP 同一条管线），
//! 因此在这条路上成立的性质（授权顺序、参数校验、错误码、线格式）与
//! 真实客户端拿到的**是同一套**。
//!
//! ## 为什么它仍是**零 Slint** 的（以及这换来了什么）
//!
//! 本模块只依赖 `UiSurface`（trait）、`serde_json`、`crate::base64` 与
//! `yeban_ui_test_port` 的零 Slint 子集。于是同一段 `probe` 逻辑有两处证据：
//!
//! | 跑在哪 | 执行面 | 证明了什么 |
//! | :--- | :--- | :--- |
//! | **本机**（`rustc --edition 2024 --test -D warnings` + `crate::testing::FakeSurface`） | 真像素（真 PNG 编码器）但假窗口 | 管线 / 线格式 / 证据链 / 授权顺序 / 错误码 |
//! | **CI**（`cargo test -p yeban-app --all-targets`，`crates/yeban-app/tests/live_ui_mcp.rs`） | `LivePort<MainWindow>`（Tier-1 软件光栅化 + 真控件树） | 上面每一项**加上**"像素与控件树真的来自产品窗口、且携带工程数据" |
//!
//! 两份证据的差别**只有执行面**，没有第二份 probe 实现（`docs/ledger/live-port-notes.md`）。
//!
//! ## 线格式的一处**实测**细节（读者会踩）
//!
//! `ui/tree` 的 `result.tree` 在线上**不是** `UiTree::to_json()` 的字节：服务侧是
//! `serde_json::to_value(&UiTree)`，而 `serde_json::Value::Object` 在未开
//! `preserve_order` 时是 `BTreeMap` ⇒ **键按字母序**排（`bounds, dynamicRegion, id, …`），
//! 而 `UiTree::to_json()` 按字段声明序（`id, role, label, …`）。两者都**逐字节稳定**，
//! 但不是同一串字节。本模块因此**解析回模型**再比对（`from_str::<UiTree>`），
//! 并把原始线上文本作为证据留档 —— 不做"把两串字节直接比"的脆弱断言。

use serde_json::{Map, Value};
use yeban_mcp::jsonrpc::Id;
use yeban_mcp::security::{BearerToken, Channel, RunMode, Scope, ScopeSet};

use yeban_ui_test_port::png::PNG_SIGNATURE;

use crate::base64;
use crate::service::UiService;
use crate::surface::{UiSurface, fnv1a64};
use crate::tree::{Coverage, UiNode, UiTree};

/// 本模块默认注入的只读作用域（§12.3 的 ReadOnly 层：树 / 属性 / 截图）。
pub const READ_ONLY_SCOPES: [Scope; 2] = [Scope::UiRead, Scope::UiScreenshot];

/// `[UI-MCP-001]` §12.3 的三级权限 → 作用域集合。
///
/// 为什么需要这张表而不是让调用方自己传：`Permission`（端口侧三级）与 `Scope`
/// （网络侧六级）是两张表，`yeban-ui-mcp` 的 `methods::port_operation_tier_matches_scope`
/// 把它们钉在一起。真实接线方只该说"我要哪一级"，映射写在这里（唯一一处）。
#[must_use]
pub fn scopes_for_permission(permission: yeban_ui_test_port::port::Permission) -> ScopeSet {
    use yeban_ui_test_port::port::Permission;
    match permission {
        Permission::ReadOnly => ScopeSet::from_scopes(READ_ONLY_SCOPES),
        Permission::Interactive => {
            ScopeSet::from_scopes([Scope::UiRead, Scope::UiScreenshot, Scope::UiInject])
        }
        Permission::Administrative => ScopeSet::from_scopes([
            Scope::UiRead,
            Scope::UiScreenshot,
            Scope::UiInject,
            Scope::AppSave,
            Scope::AppReloadEngine,
            Scope::AppAdmin,
        ]),
    }
}

/// 一次调用的结果（**不 leak** `yeban-mcp` 的类型，接线方因此不必依赖它）。
#[derive(Debug, Clone, PartialEq)]
pub struct CallResult {
    /// HTTP 形态的状态码（stdio 形态忽略它）。
    pub status: u16,
    /// 回显的 `id`（规范化为 JSON 文本：`1` / `"a"` / `null`）。
    pub id_echo: String,
    /// 错误码（成功时为 `None`）。
    pub code: Option<i64>,
    /// 错误消息（成功时为 `None`）。
    pub message: Option<String>,
    /// 机器可读的拒绝种类（`error.data.kind`，例如 `forbidden-in-production` /
    /// `element-not-found` / `port-permission-denied`）—— 判据据此断言**拒的理由**，
    /// 而不是去比对一句人话。
    pub kind: Option<String>,
    /// 成功结果（失败时为 `None`）。
    pub result: Option<Value>,
}

impl CallResult {
    /// 是否是错误响应。
    #[must_use]
    pub const fn is_error(&self) -> bool {
        self.code.is_some()
    }

    /// 变成 `Result`（`method` 只用于错误消息）。
    fn into_result(self, method: &str) -> Result<Value, ProbeError> {
        match (self.code, self.result) {
            (Some(code), _) => Err(ProbeError::RpcFailed {
                method: method.to_owned(),
                status: self.status,
                code,
                message: self.message.unwrap_or_default(),
            }),
            (None, Some(result)) => Ok(result),
            (None, None) => Err(ProbeError::Malformed {
                method: method.to_owned(),
                detail: "成功响应里没有 `result`（`result` 与 `error` 必须恰好一个）".to_owned(),
            }),
        }
    }
}

/// 控制面装配 / 端到端读数失败。
///
/// 每个变体都**指名**是哪一步、期望什么、实际什么 —— 判据失败时不需要"再跑一轮才知道"。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeError {
    /// 请求失败（JSON-RPC 错误）：状态码 + 错误码 + 消息。
    RpcFailed {
        /// 方法名。
        method: String,
        /// HTTP 形态状态码。
        status: u16,
        /// JSON-RPC 错误码。
        code: i64,
        /// 错误消息。
        message: String,
    },
    /// 响应形状不对（缺字段 / 类型不对 / `id` 没回显）。
    Malformed {
        /// 方法名。
        method: String,
        /// 具体是什么不对。
        detail: String,
    },
    /// 语义 ID 不在**运行时**控件树里。
    ElementNotInTree {
        /// 请求的语义 ID。
        id: String,
        /// 运行时树的节点数（帮助判断是"树空了"还是"ID 写错了"）。
        tree_count: usize,
    },
    /// 元素在树里，但标签里**没有**工程数据。
    LabelMismatch {
        /// 语义 ID。
        id: String,
        /// 期望出现的工程文本。
        expected: String,
        /// 实际的 `accessible-label`。
        actual: String,
    },
    /// `ui/node` 与 `ui/tree` 报的同一个节点不一致（两份事实源漂移）。
    NodeDisagreesWithTree {
        /// 语义 ID。
        id: String,
    },
    /// 截图证据链断裂（base64 不是真 PNG / 字节数或指纹或 IHDR 尺寸不符）。
    EvidenceBroken {
        /// 具体是哪一条对不上。
        detail: String,
    },
    /// 截图尺寸与期望不符。
    SizeMismatch {
        /// 期望的 `(宽, 高)`。
        expected: (u32, u32),
        /// 实际拿到的 `(宽, 高)`。
        actual: (u32, u32),
    },
}

impl core::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::RpcFailed {
                method,
                status,
                code,
                message,
            } => write!(f, "`{method}` 失败: HTTP {status} / 码 {code} / {message}"),
            Self::Malformed { method, detail } => write!(f, "`{method}` 响应形状不对: {detail}"),
            Self::ElementNotInTree { id, tree_count } => write!(
                f,
                "语义 ID `{id}` 不在运行时控件树里（树 {tree_count} 个节点）\
                 —— [UI-TEST-001] §12.2 只允许按语义 ID 寻址, 找不到就是找不到"
            ),
            Self::LabelMismatch {
                id,
                expected,
                actual,
            } => write!(
                f,
                "`{id}` 的标签是 {actual:?}, 不含工程数据 {expected:?} —— 界面读的可能不是工程"
            ),
            Self::NodeDisagreesWithTree { id } => write!(
                f,
                "`{id}`: `ui/tree` 与 `ui/node` 报的节点不一致（同一棵树的两条查询路径漂移了）"
            ),
            Self::EvidenceBroken { detail } => write!(f, "截图证据链断裂: {detail}"),
            Self::SizeMismatch { expected, actual } => write!(
                f,
                "[MUST-GATE-015] 截图尺寸不符: 期望 {}x{}, 实际 {}x{}",
                expected.0, expected.1, actual.0, actual.1
            ),
        }
    }
}

impl core::error::Error for ProbeError {}

/// 一次 `ui/screenshot` 的读数。
///
/// **每一个数字都来自发出去的那一份 PNG 字节**（或其 HTTP 形态的 base64）：
/// `png_bytes` / `fingerprint` / `ihdr_*` 是本地对字节重算的，
/// 因此它们同时钉住"证据说的帧 == 发出去的帧"。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenshotProbe {
    /// 是否请求了动态区遮罩。
    pub mask_dynamic: bool,
    /// 被置黑的动态区个数。
    pub masked_regions: usize,
    /// 遮罩是否**真的**生效（矩形内像素全是纯黑）。
    pub mask_effective: bool,
    /// 宽（像素）。
    pub width: u32,
    /// 高（像素）。
    pub height: u32,
    /// 非黑像素数（证据字段）。
    pub non_black_pixels: u64,
    /// 不同颜色数（证据字段）。
    pub distinct_colors: usize,
    /// PNG 字节数（证据字段）。
    pub png_bytes: usize,
    /// PNG 字节的 FNV-1a 指纹，线上是 16 位小写十六进制文本。
    pub fingerprint: String,
    /// 本地从 base64 解出来的 PNG 字节长度。
    pub decoded_bytes: usize,
    /// 从 PNG 的 IHDR 里**独立读出**的宽（不是 `width` 字段的回显）。
    pub ihdr_width: u32,
    /// 从 PNG 的 IHDR 里**独立读出**的高。
    pub ihdr_height: u32,
    /// PNG 字节（真字节：接线方可以落盘做人眼复核）。
    pub png: Vec<u8>,
}

impl ScreenshotProbe {
    /// 解析并**校验**`ui/screenshot` 的结果。
    ///
    /// 校验的是"证据链闭合"这一件事：base64 必须解出真 PNG、解出的字节数必须等于
    /// `pngBytes`、指纹必须等于对同一串字节重算的 FNV-1a、IHDR 里的尺寸必须等于
    /// `width`/`height`。任何一条不成立都**报错**，绝不"取一个看起来合理的数"。
    ///
    /// # Errors
    ///
    /// 字段缺失/类型不对（[`ProbeError::Malformed`]），或证据链断裂
    /// （[`ProbeError::EvidenceBroken`]）。
    pub fn parse(result: &Value) -> Result<Self, ProbeError> {
        let method = crate::methods::METHOD_SCREENSHOT;
        let mask_dynamic =
            bool_field(result, "maskDynamic").ok_or_else(|| ProbeError::Malformed {
                method: method.to_owned(),
                detail: "缺 `maskDynamic`".to_owned(),
            })?;
        let mask_effective =
            bool_field(result, "maskEffective").ok_or_else(|| ProbeError::Malformed {
                method: method.to_owned(),
                detail: "缺 `maskEffective`".to_owned(),
            })?;
        let masked_regions = usize_field(result, "maskedRegions", method)?;
        let width = u32_field(result, "width", method)?;
        let height = u32_field(result, "height", method)?;
        let non_black_pixels = u64_field(result, "nonBlackPixels", method)?;
        let distinct_colors = usize_field(result, "distinctColors", method)?;
        let png_bytes = usize_field(result, "pngBytes", method)?;
        let fingerprint = text_field(result, "fingerprint", method)?;
        let encoded = text_field(result, "pngBase64", method)?;

        let png = base64::decode(&encoded).map_err(|error| ProbeError::EvidenceBroken {
            detail: format!("`pngBase64` 不是合法 base64: {error}"),
        })?;
        if !png.starts_with(&PNG_SIGNATURE) {
            return Err(ProbeError::EvidenceBroken {
                detail: "`pngBase64` 解出来的字节不是 PNG（魔数不符）".to_owned(),
            });
        }
        if png.len() != png_bytes {
            return Err(ProbeError::EvidenceBroken {
                detail: format!("`pngBytes`={png_bytes} 与解出的字节数 {} 不符", png.len()),
            });
        }
        let recomputed = format!("{:016x}", fnv1a64(&png));
        if recomputed != fingerprint {
            return Err(ProbeError::EvidenceBroken {
                detail: format!(
                    "`fingerprint`={fingerprint} 与对同一串字节重算的 {recomputed} 不符"
                ),
            });
        }
        let (ihdr_width, ihdr_height) =
            png_ihdr_size(&png).ok_or_else(|| ProbeError::EvidenceBroken {
                detail: "PNG 不到 24 字节或 IHDR 块头缺失，读不出尺寸".to_owned(),
            })?;
        if (ihdr_width, ihdr_height) != (width, height) {
            return Err(ProbeError::EvidenceBroken {
                detail: format!(
                    "IHDR 里的 {ihdr_width}x{ihdr_height} 与 `width`/`height` 的 {width}x{height} 不符"
                ),
            });
        }
        if width == 0 || height == 0 {
            return Err(ProbeError::EvidenceBroken {
                detail: "[MUST-GATE-015] 尺寸为零".to_owned(),
            });
        }
        if non_black_pixels == 0 || non_black_pixels > u64::from(width) * u64::from(height) {
            return Err(ProbeError::EvidenceBroken {
                detail: format!(
                    "[MUST-GATE-015] `nonBlackPixels`={non_black_pixels} 越出 1..={} 的合法区间 \
                     （全黑帧必须在服务侧就被拒）",
                    u64::from(width) * u64::from(height)
                ),
            });
        }
        Ok(Self {
            mask_dynamic,
            masked_regions,
            mask_effective,
            width,
            height,
            non_black_pixels,
            distinct_colors,
            png_bytes,
            fingerprint,
            decoded_bytes: png.len(),
            ihdr_width,
            ihdr_height,
            png,
        })
    }

    /// 非黑占比（百分比，1 位小数）。
    #[must_use]
    pub fn non_black_percent(&self) -> f64 {
        let total = u64::from(self.width) * u64::from(self.height);
        if total == 0 {
            return 0.0;
        }
        (self.non_black_pixels as f64 * 1000.0 / total as f64).round() / 10.0
    }

    /// 一行可记录的证据（`report_line` / artifact 用）。
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "ui/screenshot: {}x{} maskDynamic={} regions={} maskEffective={} 非黑 {}/{} ({}%) 颜色 {} 种 \
             PNG {} 字节 (解码 {} 字节) 指纹 {} IHDR {}x{}",
            self.width,
            self.height,
            self.mask_dynamic,
            self.masked_regions,
            self.mask_effective,
            self.non_black_pixels,
            u64::from(self.width) * u64::from(self.height),
            self.non_black_percent(),
            self.distinct_colors,
            self.png_bytes,
            self.decoded_bytes,
            self.fingerprint,
            self.ihdr_width,
            self.ihdr_height,
        )
    }
}

/// 一次「AI 看界面」的完整读数：树 → 节点 → 截图，三件证据来自**同一个执行面**。
#[derive(Debug, Clone, PartialEq)]
pub struct LiveProbe {
    /// 执行面标识（真实接线时是 `LivePort` 适配器的名字）。
    pub surface: &'static str,
    /// 被查询的语义 ID。
    pub requested_id: String,
    /// 期望出现在标签里的**工程**文本。
    pub expected_label_fragment: String,
    /// 运行时控件树的投影（由线上的 `result.tree` 反序列化而来）。
    pub tree: UiTree,
    /// `ui/tree` 给的那串原始 JSON（线上形态，键为字母序）。
    pub tree_json: String,
    /// 按语义 ID 查到的节点（`ui/tree` 与 `ui/node` 两条路必须一致）。
    pub node: UiNode,
    /// `ui/node` 给的那串原始 JSON。
    pub node_json: String,
    /// 截图读数。
    pub screenshot: ScreenshotProbe,
}

impl LiveProbe {
    /// 多行证据（每行都能进 CI 日志与 artifact）。
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "[live-port] surface={} 树 {} 节点 (source={}) 查 `{}` -> role={} label={:?} \
             (期望含 {:?})\n[live-port] {}",
            self.surface,
            self.tree.count,
            self.tree.source.as_str(),
            self.requested_id,
            self.node.role,
            self.node.label,
            self.expected_label_fragment,
            self.screenshot.summary(),
        )
    }
}

/// [`ControlPlane::probe`] 的参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeOptions {
    /// 要查的语义 ID（`[UI-TEST-001]` §12.2：只能是语义 ID，不能是像素坐标）。
    pub element_id: String,
    /// 期望出现在该节点标签里的**工程数据**（例如 `filled_project()` 的轨道名 `Lead`）。
    pub expected_label_fragment: String,
    /// 是否让 `ui/screenshot` 遮掉动态区（默认 `true`，§12.5 的 MUST）。
    pub mask_dynamic: bool,
    /// 期望的截图像素尺寸（`[MUST-GATE-015]` 的"尺寸正确"）。
    pub expected_size: Option<(u32, u32)>,
}

impl ProbeOptions {
    /// 必填两项：语义 ID + 期望的工程文本。
    #[must_use]
    pub fn new(element_id: impl Into<String>, expected_label_fragment: impl Into<String>) -> Self {
        Self {
            element_id: element_id.into(),
            expected_label_fragment: expected_label_fragment.into(),
            mask_dynamic: true,
            expected_size: None,
        }
    }

    /// 要**原始帧**（不遮罩）—— `maskDynamic: false`。
    #[must_use]
    pub fn without_masking(mut self) -> Self {
        self.mask_dynamic = false;
        self
    }

    /// 钉住截图的像素尺寸。
    #[must_use]
    pub fn expecting_size(mut self, width: u32, height: u32) -> Self {
        self.expected_size = Some((width, height));
        self
    }
}

/// 把执行面装配成一个能应答 `ui/*` 的控制面（一个执行面 + 一套凭据/作用域/模式）。
///
/// 这是**接线方唯一需要的公开构造点**：它把 `BearerToken`（256-bit、`yeban-mcp` 生成）
/// 与 `ScopeSet`（§12.3 三级 → §7.2 六级的映射）收在里面，接线方不必依赖
/// `yeban-mcp`，也就不会自己去拼一套（拼错一套就是一条静默的越权路径）。
pub struct ControlPlane {
    service: UiService,
    token: BearerToken,
}

impl ControlPlane {
    /// 只读控制面：**生产模式** + §12.3 的 ReadOnly 层（树 / 属性 / 截图）。
    ///
    /// 这是本工作线的默认形态：AI 能看，不能动。
    #[must_use]
    pub fn read_only(surface: Box<dyn UiSurface>) -> Self {
        Self::with_scopes(surface, &READ_ONLY_SCOPES, RunMode::Production)
    }

    /// 测试模式的控制面：只读 + `ui:inject`（`RunMode::Test`）。
    ///
    /// 名字里带 `for_tests` 是刻意的：`ui:inject` 在**生产模式**被硬禁
    /// （`yeban_mcp::security::authorize` 的第一件事），因此这条构造点只该出现在
    /// 判据里，不该被生产接线顺手用上。
    #[must_use]
    pub fn interactive_for_tests(surface: Box<dyn UiSurface>) -> Self {
        Self::with_scopes(
            surface,
            &[Scope::UiRead, Scope::UiScreenshot, Scope::UiInject],
            RunMode::Test,
        )
    }

    /// 显式给出作用域与运行模式（其余构造点都走它）。
    #[must_use]
    pub fn with_scopes(surface: Box<dyn UiSurface>, scopes: &[Scope], mode: RunMode) -> Self {
        let generated = BearerToken::generate();
        let service = UiService::new(
            generated.token.clone(),
            ScopeSet::from_scopes(scopes.iter().copied()),
            mode,
            surface,
        );
        Self {
            service,
            token: generated.token,
        }
    }

    /// 期望的令牌（判据要用它构造 `Authorization` 头）。
    #[must_use]
    pub fn token(&self) -> &BearerToken {
        &self.token
    }

    /// `Authorization: Bearer <token>` 头的值。
    #[must_use]
    pub fn authorization(&self) -> String {
        format!("Bearer {}", self.token.expose())
    }

    /// 执行面标识。
    #[must_use]
    pub fn surface_name(&self) -> &'static str {
        self.service.surface_name()
    }

    /// 内层服务（只读）。
    #[must_use]
    pub fn service(&self) -> &UiService {
        &self.service
    }

    /// 内层服务（可变）—— 例如判据要读 `handled()` / `denied()` 计数。
    pub fn service_mut(&mut self) -> &mut UiService {
        &mut self.service
    }

    /// 用**已鉴权**的一行 JSON-RPC 文本调用（与 stdio / HTTP body 同一条入口）。
    pub fn try_line(&mut self, line: &str) -> CallResult {
        let authorization = self.authorization();
        self.try_line_with_authorization(Some(&authorization), line)
    }

    /// 用**指定**凭据调用（`None` = 完全没有 `Authorization` 头）。
    ///
    ///
    /// 未鉴权时 `handle_line` 会在解析请求体**之前**就返回 401，因此这个入口也是
    /// "鉴权先于解析"这条性质的判据入口。
    pub fn try_line_with_authorization(
        &mut self,
        authorization: Option<&str>,
        line: &str,
    ) -> CallResult {
        let outcome = self.service.handle_line(Channel::Http, authorization, line);
        let status = outcome.http_status;
        let Some(response) = outcome.response else {
            return CallResult {
                status,
                id_echo: "notification".to_owned(),
                code: None,
                message: None,
                kind: None,
                result: None,
            };
        };
        let error = response.error_object();
        CallResult {
            status,
            id_echo: id_text(&response.id),
            code: error.map(|error| error.code),
            message: error.map(|error| error.message.clone()),
            kind: error
                .and_then(|error| error.data.as_ref())
                .and_then(|data| data.get("kind"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            result: response.result.clone(),
        }
    }

    /// 构造并发送一次方法调用（`id` 固定为 1，便于逐字节对账）。
    ///
    /// # Errors
    ///
    /// JSON-RPC 失败（[`ProbeError::RpcFailed`]）或响应形状不对。
    pub fn call(&mut self, method: &str, params: Option<Value>) -> Result<Value, ProbeError> {
        self.call_with_id(1, method, params)
    }

    /// 同 [`ControlPlane::call`]，但显式给 `id`（判据要验证回显）。
    ///
    /// # Errors
    ///
    /// 同 [`ControlPlane::call`]；`id` 没被回显也算形状不对。
    pub fn call_with_id(
        &mut self,
        id: i64,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, ProbeError> {
        let line = request_line(id, method, params);
        let outcome = self.try_line(&line);
        if outcome.id_echo != id.to_string() {
            return Err(ProbeError::Malformed {
                method: method.to_owned(),
                detail: format!("`id` 没被回显: 期望 {id}, 实际 {}", outcome.id_echo),
            });
        }
        outcome.into_result(method)
    }

    /// `ui/methods`：能力发现（本进程注册了哪些方法、要什么 scope）。
    ///
    /// # Errors
    ///
    /// 请求失败。
    pub fn methods(&mut self) -> Result<Value, ProbeError> {
        self.call(crate::methods::METHOD_METHODS, None)
    }

    /// `ui/tree`：运行时语义控件树。
    ///
    /// # Errors
    ///
    /// 请求失败，或 `result.tree` 不是合法的树投影。
    pub fn tree(&mut self) -> Result<(UiTree, String), ProbeError> {
        let result = self.call(crate::methods::METHOD_TREE, None)?;
        let raw = field_string(&result, "tree", crate::methods::METHOD_TREE)?;
        let tree = parse_ui_tree(&raw)?;
        Ok((tree, raw))
    }

    /// `ui/coverage`：运行时树与一份注册表 ID 清单的**双向**覆盖。
    ///
    /// # Errors
    ///
    /// 请求失败，或结果不是合法的 `Coverage`。
    pub fn coverage(&mut self, registry_ids: &[String]) -> Result<Coverage, ProbeError> {
        let params = Value::Object(Map::from_iter([(
            "ids".to_owned(),
            Value::Array(registry_ids.iter().cloned().map(Value::from).collect()),
        )]));
        let result = self.call(crate::methods::METHOD_COVERAGE, Some(params))?;
        serde_json::from_value(result).map_err(|error| ProbeError::Malformed {
            method: crate::methods::METHOD_COVERAGE.to_owned(),
            detail: format!("`ui/coverage` 的结果无法反序列化: {error}"),
        })
    }

    /// **本工作线的端到端判据本体**：`ui/tree` → 按语义 ID 找节点 → `ui/node` →
    /// `ui/screenshot`，四步全部走真实的 JSON-RPC 文本入口。
    ///
    /// 它断言的性质（逐条）：
    /// 1. **树是可读的**：`result.tree` 能被反序列化回 `UiTree`，`count == nodes.len()`；
    /// 2. **语义 ID 查得到**：找不到 ⇒ [`ProbeError::ElementNotInTree`]（不是空成功）；
    /// 3. **两条查询路径一致**：`ui/node` 与 `ui/tree` 报同一个节点
    ///    （不一致 ⇒ [`ProbeError::NodeDisagreesWithTree`]）；
    /// 4. **节点携带工程数据**：`label` 必须含 `expected_label_fragment`
    ///    （⇒ [`ProbeError::LabelMismatch`]）—— 这一条把"界面读的是工程还是演示常量"
    ///    变成可判定的；
    /// 5. **截图是 Tier-1 真帧**：尺寸非零、非全黑、base64/字节数/指纹/IHDR 四者自洽
    ///    （[`ScreenshotProbe::parse`]）；
    /// 6. **尺寸正确**：给了 `expected_size` 就必须相等。
    ///
    /// # Errors
    ///
    /// 上面任何一条不成立。
    pub fn probe(&mut self, options: &ProbeOptions) -> Result<LiveProbe, ProbeError> {
        let surface = self.service.surface_name();
        let (tree, tree_json) = self.tree()?;
        if tree.count != tree.nodes.len() {
            return Err(ProbeError::Malformed {
                method: crate::methods::METHOD_TREE.to_owned(),
                detail: format!("`count`={} 与节点数 {} 不符", tree.count, tree.nodes.len()),
            });
        }
        let from_tree = find_semantic_node(&tree, &options.element_id)?.clone();

        let node_params = Value::Object(Map::from_iter([(
            "elementId".to_owned(),
            Value::from(options.element_id.clone()),
        )]));
        let node_result = self.call(crate::methods::METHOD_NODE, Some(node_params))?;
        let node_json = field_string(&node_result, "node", crate::methods::METHOD_NODE)?;
        let node = parse_ui_node(&node_json)?;
        if node != from_tree {
            return Err(ProbeError::NodeDisagreesWithTree {
                id: options.element_id.clone(),
            });
        }
        if !node.label.contains(&options.expected_label_fragment) {
            return Err(ProbeError::LabelMismatch {
                id: options.element_id.clone(),
                expected: options.expected_label_fragment.clone(),
                actual: node.label.clone(),
            });
        }

        let shot_params = Value::Object(Map::from_iter([(
            "maskDynamic".to_owned(),
            Value::from(options.mask_dynamic),
        )]));
        let shot_result = self.call(crate::methods::METHOD_SCREENSHOT, Some(shot_params))?;
        let screenshot = ScreenshotProbe::parse(&shot_result)?;
        if let Some(expected) = options.expected_size {
            let actual = (screenshot.width, screenshot.height);
            if actual != expected {
                return Err(ProbeError::SizeMismatch { expected, actual });
            }
        }

        Ok(LiveProbe {
            surface,
            requested_id: options.element_id.clone(),
            expected_label_fragment: options.expected_label_fragment.clone(),
            tree,
            tree_json,
            node,
            node_json,
            screenshot,
        })
    }
}

/// 在投影后的控件树里按语义 ID 精确查找（`[UI-TEST-001]` §12.2）。
///
/// # Errors
///
/// ID 不在树里 ⇒ [`ProbeError::ElementNotInTree`]，**绝不**返回 `None` 让调用方
/// 顺手 `unwrap_or_default()` 出一个空节点。
pub fn find_semantic_node<'a>(tree: &'a UiTree, id: &str) -> Result<&'a UiNode, ProbeError> {
    tree.find(id).ok_or_else(|| ProbeError::ElementNotInTree {
        id: id.to_owned(),
        tree_count: tree.count,
    })
}

/// 在某个 `for` 循环展开的族里找**第一个**成员（ID 升序里的第一个）。
///
/// `[UI-TEST-001]` 的三个族（`track-{i}-header` / `note-{ulid}-rect` /
/// `clip-{ulid}-header`）都是"前缀 + 身份 + 后缀"的形状；上游会把重复元素展开成
/// 每个实例一个子 `ItemTree`（实测见 `docs/ledger/app-introspect-notes.md` §6.1），
/// 因此族成员可以被语义 ID 逐个寻址。这里只做纯字符串匹配。
#[must_use]
pub fn find_family_member<'a>(tree: &'a UiTree, prefix: &str, suffix: &str) -> Option<&'a UiNode> {
    tree.nodes
        .iter()
        .find(|node| node.id.starts_with(prefix) && node.id.ends_with(suffix))
}

/// 族成员的个数（`prefix` + `suffix` 形状）。
#[must_use]
pub fn family_member_count(tree: &UiTree, prefix: &str, suffix: &str) -> usize {
    tree.nodes
        .iter()
        .filter(|node| node.id.starts_with(prefix) && node.id.ends_with(suffix))
        .count()
}

/// 把线上的 `result.tree` 解析回 [`UiTree`]。
///
/// # Errors
///
/// 不是合法的树投影。
pub fn parse_ui_tree(json: &str) -> Result<UiTree, ProbeError> {
    serde_json::from_str(json).map_err(|error| ProbeError::Malformed {
        method: crate::methods::METHOD_TREE.to_owned(),
        detail: format!("`result.tree` 无法反序列化: {error}"),
    })
}

/// 把线上的 `result.node` 解析回 [`UiNode`]。
///
/// # Errors
///
/// 不是合法的节点投影。
pub fn parse_ui_node(json: &str) -> Result<UiNode, ProbeError> {
    serde_json::from_str(json).map_err(|error| ProbeError::Malformed {
        method: crate::methods::METHOD_NODE.to_owned(),
        detail: format!("`result.node` 无法反序列化: {error}"),
    })
}

/// 组一行 JSON-RPC 请求文本（**唯一**的线格式组装点）。
#[must_use]
pub fn request_line(id: i64, method: &str, params: Option<Value>) -> String {
    let mut object = Map::new();
    object.insert("jsonrpc".to_owned(), Value::from("2.0"));
    object.insert("id".to_owned(), Value::from(id));
    object.insert("method".to_owned(), Value::from(method));
    if let Some(params) = params {
        object.insert("params".to_owned(), params);
    }
    Value::Object(object).to_string()
}

/// PNG 的 IHDR 尺寸：签名 8 字节 + 块长 4 字节 + 类型 4 字节 ⇒ 宽在 `[16,20)`、高在 `[20,24)`。
///
/// 这是**独立于编码器**的读法（对着字节读，而不是回显证据字段），因此它能抓住
/// "证据说 1920x1080、发出去的图是别的尺寸"这类错误。本仓库的 PNG 编码器只写不读
/// （见 `yeban-ui-test-port` 的 notes），所以这里只读 IHDR 这 8 个字节，不实现解码。
#[must_use]
pub fn png_ihdr_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Some((width, height))
}

/// `Id` → JSON 文本（`1` / `"a"` / `null`）。
fn id_text(id: &Id) -> String {
    match id {
        Id::Number(number) => number.to_string(),
        Id::Text(text) => Value::from(text.as_str()).to_string(),
        Id::Null => "null".to_owned(),
    }
}

/// 取一个必须存在的对象字段并序列化成文本。
fn field_string(result: &Value, field: &str, method: &str) -> Result<String, ProbeError> {
    result
        .get(field)
        .map(Value::to_string)
        .ok_or_else(|| ProbeError::Malformed {
            method: method.to_owned(),
            detail: format!("`result` 里缺 `{field}`"),
        })
}

/// 取布尔字段。
fn bool_field(result: &Value, field: &str) -> Option<bool> {
    result.get(field).and_then(Value::as_bool)
}

/// 取字符串字段。
fn text_field(result: &Value, field: &str, method: &str) -> Result<String, ProbeError> {
    result
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ProbeError::Malformed {
            method: method.to_owned(),
            detail: format!("`{field}` 缺失或不是字符串"),
        })
}

/// 取 `u32` 字段（`serde_json` 的数字是 `u64`；越界即形状不对）。
fn u32_field(result: &Value, field: &str, method: &str) -> Result<u32, ProbeError> {
    let raw = result
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| ProbeError::Malformed {
            method: method.to_owned(),
            detail: format!("`{field}` 缺失或不是非负整数"),
        })?;
    u32::try_from(raw).map_err(|_| ProbeError::Malformed {
        method: method.to_owned(),
        detail: format!("`{field}`={raw} 超出 u32"),
    })
}

/// 取 `u64` 字段。
fn u64_field(result: &Value, field: &str, method: &str) -> Result<u64, ProbeError> {
    result
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| ProbeError::Malformed {
            method: method.to_owned(),
            detail: format!("`{field}` 缺失或不是非负整数"),
        })
}

/// 取 `usize` 字段。
fn usize_field(result: &Value, field: &str, method: &str) -> Result<usize, ProbeError> {
    let raw = u64_field(result, field, method)?;
    usize::try_from(raw).map_err(|_| ProbeError::Malformed {
        method: method.to_owned(),
        detail: format!("`{field}`={raw} 超出 usize"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::ELEMENT_NOT_FOUND;
    use crate::testing::{FakeSurface, build_service_with_tree, fixture_tree, shared};
    use yeban_mcp::jsonrpc::FORBIDDEN;
    use yeban_ui_test_port::image::{Rect, Size};
    use yeban_ui_test_port::port::Permission;
    use yeban_ui_test_port::tree::{ControlNode, ControlTree, Role};

    /// 执行面标识（与 `PortAdapter` 的真实名字一致，让本机与 CI 的日志可以逐字对照）。
    const LIVE_NAME: &str = "tier1-live-port";

    /// 一个**工程数据驱动**的运行时树夹具：`track-0-header` 的标签带着工程里的轨道名
    /// `Lead`（`yeban_model::samples::filled_project()` 的第一条轨道名）。
    ///
    /// 本 crate **不依赖** `yeban-model`（见 `Cargo.toml` 的依赖方向注释），所以这里
    /// 抄的是那串**字面值**；CI 侧的真判据（`crates/yeban-app/tests/live_ui_mcp.rs`）
    /// 把它换成 `project.tracks[0].name`，因此"字面值是否等于工程"由 CI 钉住。
    fn project_like_tree() -> ControlTree {
        let mut tree = ControlTree::new();
        let mut add = |id: &str, role: &str, label: &str, bounds: Option<Rect>| {
            let node = ControlNode::new(id, Role::parse(role).expect("合法角色"), label);
            let node = match bounds {
                Some(rect) => node.with_bounds(rect),
                None => node,
            };
            tree.insert(node).expect("夹具 ID 唯一且合法");
        };
        add(
            "track-0-header",
            "list-item",
            "轨道 Lead",
            Some(Rect::new(0, 48, 128, 56)),
        );
        add("track-0-fader", "slider", "轨道 Lead 推子", None);
        add(
            "status-bar",
            "region",
            "状态栏",
            Some(Rect::new(0, 114, 200, 6)),
        );
        tree.insert(
            ControlNode::new(
                "transport-timecode",
                Role::parse("text").expect("角色"),
                "时间码",
            )
            .with_bounds(Rect::new(8, 4, 96, 16))
            .as_dynamic(),
        )
        .expect("插入");
        tree
    }

    /// 假执行面 + 控制面（本机零 Slint 的端到端装置）。
    fn fake_plane(permission: Permission) -> ControlPlane {
        let surface = FakeSurface {
            state: shared(permission),
            tree: project_like_tree(),
        };
        match permission {
            Permission::ReadOnly => ControlPlane::read_only(Box::new(surface)),
            _ => ControlPlane::interactive_for_tests(Box::new(surface)),
        }
    }

    /// 用 `PortAdapter` 包一层（与真实接线同形状：像素来自注入的闭包）。
    fn adapted_plane() -> ControlPlane {
        let inner = FakeSurface {
            state: shared(Permission::ReadOnly),
            tree: project_like_tree(),
        };
        let adapter = crate::surface::PortAdapter::new(inner, LIVE_NAME, |port: &FakeSurface| {
            port.capture_image()
        });
        ControlPlane::read_only(Box::new(adapter))
    }

    /// 判据 1（**端到端**）：树 → 语义 ID → 节点 → 截图，四步全部走线上 JSON-RPC 文本，
    /// 且每一步的数字都对得上。
    #[test]
    fn probe_reads_a_project_backed_node_and_a_real_png_end_to_end() {
        let mut plane = adapted_plane();
        assert_eq!(plane.surface_name(), LIVE_NAME);

        let options = ProbeOptions::new("track-0-header", "Lead").expecting_size(200, 120);
        let probe = plane.probe(&options).expect("端到端读数");

        // --- 树 ---
        assert_eq!(probe.tree.source.as_str(), "runtime");
        assert_eq!(probe.tree.count, probe.tree.nodes.len());
        assert_eq!(probe.tree.count, project_like_tree().len());
        assert!(
            probe.tree_json.contains("\"track-0-header\""),
            "线上文本里必须真的有这个 ID: {}",
            probe.tree_json
        );
        // 线格式的键是**字母序**（`serde_json::Value` = BTreeMap）—— 与 `UiTree::to_json()`
        // 的字段声明序不同。这里把这个实测性质钉住，免得有人"顺手"改掉一端。
        assert!(
            probe.tree_json.starts_with("{\"count\":") && probe.tree_json.contains(",\"nodes\":["),
            "线上投影的顶层键必须按字母序 (`count,nodes,source`): {}",
            &probe.tree_json[..probe.tree_json.len().min(80)]
        );

        // --- 节点 ---
        assert_eq!(probe.node.id, "track-0-header");
        assert_eq!(probe.node.role, "list-item");
        assert_eq!(probe.node.label, "轨道 Lead");
        assert_eq!(probe.node.bounds, Some(Rect::new(0, 48, 128, 56)));
        assert_eq!(
            probe.node.visible,
            Some(true),
            "运行时树有几何 ⇒ visible=true"
        );
        assert_eq!(
            probe.node_json,
            serde_json::to_value(&probe.node)
                .expect("序列化")
                .to_string(),
            "`ui/node` 的线上文本必须就是该节点的 JSON"
        );

        // --- 截图 ---
        let shot = &probe.screenshot;
        assert_eq!((shot.width, shot.height), (200, 120));
        assert_eq!((shot.ihdr_width, shot.ihdr_height), (200, 120));
        assert_eq!(shot.png_bytes, shot.decoded_bytes);
        assert!(
            shot.png.starts_with(&PNG_SIGNATURE),
            "发出去的必须是真的 PNG 字节"
        );
        assert!(shot.mask_dynamic, "默认必须遮罩 (§12.5 的 MUST)");
        assert_eq!(shot.masked_regions, 1, "夹具只登记了一个动态区");
        assert!(shot.mask_effective, "置黑必须真的发生在发出去的图里");
        assert!(shot.non_black_pixels > 0);
        assert_eq!(shot.fingerprint, format!("{:016x}", fnv1a64(&shot.png)));
        assert_eq!(shot.png.len(), shot.png_bytes);
        assert!(shot.non_black_percent() > 0.0);
        assert!(
            probe.summary().contains("ui/screenshot"),
            "证据行必须能被人一眼看懂"
        );
    }

    /// 判据 2：语义 ID 不在树里 ⇒ **显式错误**（不是空成功），并且服务侧的
    /// `ui/node` 对同一个 ID 也必须给 `-32006`（两层都诚实，谁来问都一样）。
    #[test]
    fn probe_reports_a_missing_semantic_id_as_an_explicit_error() {
        let mut plane = fake_plane(Permission::ReadOnly);
        let error = plane
            .probe(&ProbeOptions::new("track-9-header", "Lead"))
            .expect_err("不存在的 ID 必须报错");
        assert_eq!(
            error,
            ProbeError::ElementNotInTree {
                id: "track-9-header".to_owned(),
                tree_count: project_like_tree().len(),
            },
            "probe 必须在**树**这一层就点名(而不是往下走一个空节点)"
        );

        // 直接问服务的 `ui/node`：同一条"找不到"必须是 -32006 / HTTP 400。
        let line = request_line(
            3,
            crate::methods::METHOD_NODE,
            Some(Value::Object(Map::from_iter([(
                "elementId".to_owned(),
                Value::from("track-9-header"),
            )]))),
        );
        let outcome = plane.try_line(&line);
        assert_eq!(outcome.status, 400);
        assert_eq!(outcome.code, Some(ELEMENT_NOT_FOUND));
    }

    /// 判据 3：**标签里没有工程数据就是失败**（本工作线"AI 读到的标签 == 工程里的轨道名"
    /// 这一条的负向面）。
    #[test]
    fn probe_rejects_a_label_that_does_not_carry_the_project_data() {
        let mut plane = fake_plane(Permission::ReadOnly);
        // `Lead` 在；`鼓`（演示夹具的名字）不在 ⇒ 必须报 LabelMismatch 并**说出两边**。
        let error = plane
            .probe(&ProbeOptions::new("track-0-header", "鼓"))
            .expect_err("标签不含期望文本时必须报错");
        match error {
            ProbeError::LabelMismatch {
                id,
                expected,
                actual,
            } => {
                assert_eq!(id, "track-0-header");
                assert_eq!(expected, "鼓");
                assert_eq!(actual, "轨道 Lead");
            }
            other => panic!("期望 LabelMismatch, 实际 {other:?}"),
        }
    }

    /// 判据 4：全黑帧必须被**服务侧**拒（`[MUST-GATE-015]` 的非全黑门槛 → `-32008`）。
    #[test]
    fn an_all_black_frame_is_rejected_with_capture_failed() {
        let state = shared(Permission::ReadOnly);
        state.borrow_mut().image = yeban_ui_test_port::image::Rgb8Image::new(Size::new(200, 120));
        let surface = FakeSurface {
            state,
            tree: project_like_tree(),
        };
        let mut plane = ControlPlane::read_only(Box::new(surface));
        let error = plane
            .probe(&ProbeOptions::new("track-0-header", "Lead"))
            .expect_err("全黑帧必须被拒");
        match error {
            ProbeError::RpcFailed { code, status, .. } => {
                assert_eq!(code, crate::service::CAPTURE_FAILED);
                assert_eq!(status, 500);
            }
            other => panic!("期望 RpcFailed(CAPTURE_FAILED), 实际 {other:?}"),
        }
    }

    /// 判据 5：尺寸不符 ⇒ `SizeMismatch`（"尺寸正确"不是回显，是拿 IHDR 比出来的）。
    #[test]
    fn probe_checks_the_screenshot_size_against_the_expected_one() {
        let mut plane = fake_plane(Permission::ReadOnly);
        let error = plane
            .probe(&ProbeOptions::new("track-0-header", "Lead").expecting_size(1920, 1080))
            .expect_err("尺寸不符必须报错");
        assert_eq!(
            error,
            ProbeError::SizeMismatch {
                expected: (1920, 1080),
                actual: (200, 120),
            }
        );
    }

    /// 判据 6：同一执行面连续两次读数**逐字节相同**（线格式与证据链都是确定的）。
    #[test]
    fn two_probes_of_the_same_surface_are_byte_identical() {
        let mut plane = adapted_plane();
        let options = ProbeOptions::new("track-0-header", "Lead");
        let first = plane.probe(&options).expect("第一次");
        let second = plane.probe(&options).expect("第二次");
        assert_eq!(first.tree_json, second.tree_json);
        assert_eq!(first.node_json, second.node_json);
        assert_eq!(first.node, second.node);
        assert_eq!(first.screenshot, second.screenshot);
        assert_eq!(first.screenshot.fingerprint, second.screenshot.fingerprint);
    }

    /// 判据 7：生产模式下 `ui:inject` 被**硬禁**，而且执行面一次都没被调用。
    #[test]
    fn production_hard_denies_injection_before_the_surface_is_touched() {
        let state = shared(Permission::Administrative);
        let surface = FakeSurface {
            state: std::rc::Rc::clone(&state),
            tree: project_like_tree(),
        };
        let mut plane = ControlPlane::read_only(Box::new(surface));

        let line = request_line(
            7,
            crate::methods::METHOD_DISPATCH_KEY_PRESS,
            Some(Value::Object(Map::from_iter([(
                "keyCode".to_owned(),
                Value::from("Tab"),
            )]))),
        );
        let denied = plane.try_line(&line);
        assert!(denied.is_error(), "生产模式下注入必须被拒: {denied:?}");
        assert_eq!(denied.status, 403);
        assert_eq!(denied.code, Some(FORBIDDEN));
        assert_eq!(
            state.borrow().calls,
            Vec::<String>::new(),
            "硬禁必须发生在执行面之前 (§12.3 / ARCH-SEC-002)"
        );

        // 测试模式 + 显式授予 `ui:inject` ⇒ 真的落到执行面。
        let test_state = shared(Permission::Administrative);
        let test_surface = FakeSurface {
            state: std::rc::Rc::clone(&test_state),
            tree: project_like_tree(),
        };
        let mut test_plane = ControlPlane::interactive_for_tests(Box::new(test_surface));
        let accepted = test_plane.try_line(&line);
        assert!(!accepted.is_error(), "测试模式必须放行: {accepted:?}");
        assert_eq!(test_state.borrow().calls, ["key_press:Tab".to_owned()]);
    }

    /// 判据 7b：`kind` 是**机器可读**的拒绝种类（判据据此断言"拒的理由"）。
    #[test]
    fn call_result_carries_the_machine_readable_denial_kind() {
        let mut plane = fake_plane(Permission::ReadOnly);
        let line = request_line(
            7,
            crate::methods::METHOD_DISPATCH_KEY_PRESS,
            Some(Value::Object(Map::from_iter([(
                "keyCode".to_owned(),
                Value::from("Tab"),
            )]))),
        );
        let denied = plane.try_line(&line);
        assert_eq!(denied.kind.as_deref(), Some("forbidden-in-production"));
        let not_found = plane.try_line(&request_line(
            8,
            crate::methods::METHOD_NODE,
            Some(Value::Object(Map::from_iter([(
                "elementId".to_owned(),
                Value::from("nope"),
            )]))),
        ));
        assert_eq!(not_found.kind.as_deref(), Some("element-not-found"));
        // 成功时没有 kind。
        let ok = plane.try_line(&request_line(9, crate::methods::METHOD_TREE, None));
        assert_eq!(ok.kind, None);
        assert!(!ok.is_error());
    }

    /// 判据 8：未鉴权 ⇒ 401，且**请求体一个字节都不被解析**（拿不到 `-32700`）。
    #[test]
    fn unauthenticated_calls_are_rejected_before_parsing() {
        let mut plane = fake_plane(Permission::ReadOnly);
        let outcome = plane.try_line_with_authorization(None, "{不是 JSON-RPC");
        assert_eq!(outcome.status, 401);
        assert!(outcome.is_error());
        assert_eq!(outcome.id_echo, "null", "未鉴权时无从回显 id");
        assert!(outcome.result.is_none());
    }

    /// 判据 9：`ui/coverage` 的两个方向都能报出来（运行时 ⊆ 注册表 时 `unknown` 为空）。
    #[test]
    fn coverage_reports_both_directions_through_the_live_plane() {
        let mut plane = fake_plane(Permission::ReadOnly);
        let mut ids: Vec<String> = project_like_tree().ids().map(str::to_owned).collect();
        ids.push("side-panel-only-in-the-registry".to_owned());
        let coverage = plane.coverage(&ids).expect("ui/coverage");
        assert_eq!(coverage.registry_count, ids.len());
        assert_eq!(coverage.runtime_count, project_like_tree().len());
        assert_eq!(
            coverage.missing_at_runtime,
            ["side-panel-only-in-the-registry".to_owned()],
            "注册表有而运行时没有的必须被报出来"
        );
        assert!(
            coverage.unknown_at_runtime.is_empty(),
            "运行时树里不许有未登记的 ID: {:?}",
            coverage.unknown_at_runtime
        );
    }

    /// 判据 10：纯查询辅助 —— 族成员查找与"按 ID 查找"的错误形状。
    #[test]
    fn pure_lookup_helpers_report_the_exact_failure() {
        let tree = UiTree::from_runtime(&project_like_tree());
        assert_eq!(
            find_semantic_node(&tree, "track-0-header")
                .expect("存在")
                .label,
            "轨道 Lead"
        );
        assert_eq!(
            find_semantic_node(&tree, "track-0-nope"),
            Err(ProbeError::ElementNotInTree {
                id: "track-0-nope".to_owned(),
                tree_count: tree.count,
            })
        );
        assert_eq!(
            find_family_member(&tree, "track-", "-header")
                .expect("族里有成员")
                .id,
            "track-0-header"
        );
        assert_eq!(family_member_count(&tree, "track-", "-header"), 1);
        assert_eq!(family_member_count(&tree, "note-", "-rect"), 0);
        assert!(find_family_member(&tree, "note-", "-rect").is_none());
    }

    /// 判据 11：PNG IHDR 的读法是**对着字节**的（喂进去一段假 IHDR 也能读出来）。
    #[test]
    fn png_ihdr_reader_reads_the_bytes_not_the_evidence_fields() {
        let mut fake = PNG_SIGNATURE.to_vec();
        fake.extend_from_slice(&13_u32.to_be_bytes());
        fake.extend_from_slice(b"IHDR");
        fake.extend_from_slice(&1920_u32.to_be_bytes());
        fake.extend_from_slice(&1080_u32.to_be_bytes());
        assert_eq!(png_ihdr_size(&fake), Some((1920, 1080)));
        assert_eq!(png_ihdr_size(&PNG_SIGNATURE), None, "太短必须返回 None");
        let mut wrong_chunk = fake.clone();
        wrong_chunk[12..16].copy_from_slice(b"IDAT");
        assert_eq!(png_ihdr_size(&wrong_chunk), None, "块类型不是 IHDR 就得拒");
    }

    /// 判据 12：`scopes_for_permission` 与端口三级**逐级**对齐（不多不少）。
    ///
    /// 顺序不是书写顺序，也不是字母序：`Scope` 的 `Ord` 派生自**枚举声明顺序**
    /// （即 `ARCH-SEC-002` 的规范顺序），`ScopeSet` 的内部容器是 `BTreeSet`。
    #[test]
    fn scope_table_follows_the_port_permission_tiers() {
        assert_eq!(
            scopes_for_permission(Permission::ReadOnly).to_spec_string(),
            "ui:read,ui:screenshot"
        );
        assert_eq!(
            scopes_for_permission(Permission::Interactive).to_spec_string(),
            "ui:read,ui:screenshot,ui:inject"
        );
        assert_eq!(
            scopes_for_permission(Permission::Administrative).to_spec_string(),
            "ui:read,ui:screenshot,ui:inject,app:save,app:reload-engine,app:admin"
        );
    }

    /// 判据 13：装配点与"手写一遍"等价 —— `with_scopes` 的令牌真的能用，且
    /// `ui/methods` 走过真实管线后报出执行面名字。
    #[test]
    fn the_assembly_point_exposes_capabilities_and_the_surface_name() {
        let state = shared(Permission::ReadOnly);
        let (service, token) = build_service_with_tree(
            &state,
            ScopeSet::from_scopes([Scope::UiRead, Scope::UiScreenshot]),
            RunMode::Production,
            fixture_tree(),
        );
        // 手写装配（判据用）与公开装配点必须得到同样形状的 `ui/methods`。
        let mut plane = ControlPlane {
            service,
            token: token.clone(),
        };
        let methods = plane.methods().expect("ui/methods");
        assert_eq!(methods["service"], "yeban-ui-mcp");
        assert_eq!(methods["surface"], "fake-surface");
        assert_eq!(methods["mode"], "production");
        assert_eq!(methods["injectAllowed"], false);
        assert_eq!(plane.authorization(), format!("Bearer {}", token.expose()));
    }
}
