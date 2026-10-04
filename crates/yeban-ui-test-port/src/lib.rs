//! # yeban-ui-test-port — 无头 UI 测试端口（Tier-1 软件光栅化 + 语义控件树）
//!
//! 本 crate 补的是项目当前最大的**未验证缺口**：`crates/yeban-app` 的 13 个 `.slint` 此前只被
//! 证明过"编译通过"，从未被渲染器或人眼看过（ADR-0001 D18 的代价段原文）。
//! `AGENTS.md` §3 DoD 6 要求 UI 变更必须通过**无头控件树 JSON 断言** + **已遮罩的无头截图比对**，
//! 这两件事分别由本 crate 的 [`inspect`]/[`tree`] 与 [`render`]/[`mask`]/[`ssim`] 提供。
//!
//! 规范来源 (Normative)：
//! - UI/UX 规范 §12 全文：`[UI-TEST-001]`（语义寻址）、`[UI-TEST-002]`（事件注入）、
//!   `[UI-TEST-003]`（无头运行）、`[UI-MCP-001]`（三级权限）、`[UI-MCP-002]`（动态遮罩）、
//!   `[UI-MCP-003]`（分平台 Golden，SSIM ≥ 0.98）；
//! - 架构规范 `[ARCH-UI-004]`（内省协议与权限）、`[ARCH-UI-005]`（Testing Backend 的行为边界）、
//!   `[ARCH-SLINT-001]`（上游能力核验与三层兜底）；
//! - 路线图 `[ROAD-M0-005]`（Spike 5 自研测试端口）、`[ROAD-M0-008]`（Spike 8 Tier-1 光栅化）、
//!   `[ROAD-M3-007]`（视觉回归）、`[MUST-GATE-015]`（Golden 必须由 Tier-1 产出，尺寸非零且非全黑）。
//!
//! ## 依赖方向（架构硬约束，不许反向）
//!
//! ```text
//! yeban-app ──(feature `ui-test-port`, 默认关闭)──> yeban-ui-test-port
//! ```
//!
//! 架构规范 §8 的 crate 清单就是这么写的，因此**本 crate 绝不依赖 `yeban-app`**
//! （否则成环）。代价是本 crate 不能复用 `yeban_app::elements::ElementKind` /
//! `is_well_formed_id`：控件树的数据模型、角色取值、ID 格式校验全部自持（[`tree`]），
//! 由 app 侧的适配器（`crates/yeban-app/src/test_port_adapter.rs`）单向转换。
//!
//! ## 两条无头路径的分工（ADR-0001 D18）
//!
//! | 目标 | 走哪条 | 为什么 |
//! | :--- | :--- | :--- |
//! | 控件树 / 属性 / 几何断言 | `i-slint-backend-testing` 的 `ElementHandle`（[`inspect`]） | 官方内部 crate（`=1.18.1` 精确版本），只做属性断言 |
//! | Golden 截图 | `slint::platform::Platform` + `SoftwareRenderer`（[`render`]） | `[MUST-GATE-015]`：Testing Backend **不渲染像素**，禁止用它产出 Golden |
//!
//! 两者都**不需要物理显示器**（CI 上没有 X11/Wayland）。
//! 规范 `[ARCH-UI-005]` 写的 `slint::testing::init_integration_test_backend()` /
//! `send_mouse_click()` / `send_keyboard_char()` 在上游 1.18.1 **不存在**（连 `slint::testing`
//! 这个路径都没有）；`SLINT_BACKEND=headless` 也**不存在**（只认 `qt`/`winit`/`linuxkms`）。
//! 本 crate 因此完全不依赖环境变量：平台是用 `slint::platform::set_platform` **装进去**的。
//!
//! ## 模块地图
//!
//! | 模块 | 依赖 Slint? | 职责 |
//! | :--- | :--- | :--- |
//! | [`image`] | 否 | RGB8 图像、`Rect` 几何、"非全黑"判定 —— 其余模块的公共底座 |
//! | [`tree`] | 否 | 语义控件树模型 + 稳定 JSON + 双向覆盖 + 动态区标记继承 |
//! | [`png`] | 否 | 零依赖 PNG 编码（stored deflate），决定性字节 |
//! | [`mask`] | 否 | `[UI-MCP-002]` 动态区域置黑 |
//! | [`ssim`] | 否 | `[UI-MCP-003]` mean SSIM，默认阈值 0.98 |
//! | [`port`] | 否 | `[UI-MCP-001]`/`[UI-TEST-002]` 调用面 + 三级权限纯函数 |
//! | [`golden`] | 否 | `[UI-MCP-003]` 分平台 Golden 路径约定与"严禁混用"检查 |
//! | [`render`] | **是** | Tier-1 软件光栅化截图 + `LivePort`（活窗口上的端口实现） |
//! | [`inspect`] | **是** | `ElementHandle` → 控件树 |
//!
//! **为什么前 7 个模块必须与 Slint 物理隔离**：本机纪律禁止编译 Slint
//! （`scripts/gates/run-gates.sh crate <name>` 见到 `slint` 就跳过本机档位）。
//! 隔离之后，控件树 / 遮罩 / SSIM / 权限 / PNG 这五族判据可以在**本机**用
//! `rustc --edition 2024 --test -D warnings` 直接编译执行（做法与命令见
//! `docs/ledger/ui-test-port-notes.md`），CI 只需要再证明"Slint 真的能把像素写进这个缓冲"。
//!
//! ## 本 crate 的边界（它不做什么）
//!
//! - **不实现网络服务**：绑 `127.0.0.1` 的 HTTP/JSON-RPC、会话 Token、
//!   `~/.yeban/session.token` 的 `0600` 权限属于 `crates/yeban-ui-mcp`（`[ARCH-UI-004]`）。
//!   本 crate 只提供**进程内**的调用面与权限判定。
//! - **不碰引擎层**：`Administrative` 的三个动作（切换主视图 / 强制保存 / 重载引擎）只定义
//!   接口与权限；真实接线需要 `yeban-model` / `yeban-engine` 句柄，那会污染依赖方向。
//! - **不做像素级"跨平台一致"承诺**：`[UI-MCP-003]` 明确要求**分平台**维护 Golden
//!   （字体光栅化差异），[`golden`] 提供的正是"严禁混用"的检查。
//! - **不含 `unsafe`**：本文件与全部模块都没有 `unsafe`；也**没有**声明
//!   `#![forbid(unsafe_code)]`，因为 `slint!` 宏展开的生成代码是否含 `unsafe`
//!   在本机无法核验（不能编译 Slint），而红线 8 只对 model/theory/dsp/render 强制。
//!   这一点已登记在 notes 的 pending 里。

#![deny(missing_docs)]
#![deny(rust_2018_idioms)]

pub mod golden;
pub mod image;
pub mod inspect;
pub mod mask;
pub mod png;
pub mod port;
pub mod render;
pub mod ssim;
pub mod tree;

pub use golden::{GOLDEN_ROOT, GoldenError, PlatformTag};
pub use image::{ImageError, LumaImage, MASK_COLOR, Rect, Rgb8Image, Size};
pub use inspect::{
    find_by_accessible_id, install_testing_backend, node_from_handle, role_name,
    tree_from_element_root,
};
pub use mask::{apply_masks, equal_after_masking, mask_is_effective, mask_rects_from_tree, masked};
pub use png::{PngError, REPO_MAX_FILE_BYTES, encode_rgb8, encode_rgb8_limited, encoded_len};
pub use port::{KeyCode, Operation, Permission, PointerButton, PortError, UiTestPort, authorize};
pub use render::{
    GoldenEvidence, LivePort, RenderError, Tier1Window, artifact_dir, compare_with_dynamic_masking,
    golden_evidence, report_capability, report_evidence, write_artifact,
};
// 注意：**不**把 `ssim::ssim` 函数提升到 crate 根 —— 那会让根作用域同时出现模块 `ssim`
// 与函数 `ssim`（虽然分属类型/值两个命名空间，可以编译，但对读者是纯粹的困惑）。
// 调用点请写 `yeban_ui_test_port::ssim::ssim(..)`。
pub use ssim::{SSIM_THRESHOLD, SsimError, Verdict, compare};
pub use tree::{
    ControlNode, ControlTree, Coverage, KNOWN_ROLES, Role, TreeError, is_well_formed_id,
};

/// 本 crate 覆盖的规范 ID（供 `--dump-elements` 一类的清单与审计脚本引用）。
pub const IMPLEMENTED_SPEC_IDS: &[&str] = &[
    "ARCH-SLINT-001",
    "ARCH-UI-004",
    "ARCH-UI-005",
    "MUST-GATE-015",
    "ROAD-M0-005",
    "ROAD-M0-008",
    "ROAD-M3-007",
    "TEST-SPEC-004",
    "UI-MCP-001",
    "UI-MCP-002",
    "UI-MCP-003",
    "UI-TEST-001",
    "UI-TEST-002",
    "UI-TEST-003",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// 判据: crate 级契约 —— 角色清单非空、规范 ID 清单无重复且都形如 `族-编号`。
    #[test]
    fn crate_level_contract_holds() {
        assert_eq!(KNOWN_ROLES.len(), 30);
        assert_eq!(SSIM_THRESHOLD, 0.98, "UI-MCP-003 明文要求 SSIM ≥ 0.98");
        assert_eq!(GOLDEN_ROOT, "tests/golden");
        assert_eq!(REPO_MAX_FILE_BYTES, 10 * 1024 * 1024);

        let unique: std::collections::BTreeSet<&str> =
            IMPLEMENTED_SPEC_IDS.iter().copied().collect();
        assert_eq!(unique.len(), IMPLEMENTED_SPEC_IDS.len(), "规范 ID 不得重复");
        for id in IMPLEMENTED_SPEC_IDS {
            let (family, number) = id.split_once('-').expect("规范 ID 必须含 `-`");
            assert!(!family.is_empty() && !number.is_empty(), "{id}");
            assert!(family.chars().all(|ch| ch.is_ascii_uppercase()), "{id}");
        }
    }

    /// 判据: 分级权限的**默认值**是 `ReadOnly`（`[UI-MCP-001]` §12.3 原文"默认只读层"）。
    #[test]
    fn default_permission_is_read_only() {
        assert_eq!(Permission::default(), Permission::ReadOnly);
        assert!(authorize(Permission::default(), Operation::DispatchKey).is_err());
    }
}
