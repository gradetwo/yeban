# `ui-mcp` 工作线台账：落地清单、方法名出处、判据实测与未决项

- **台账类型**：交付映射 / 契约实测 / 判据清单 / 未决项（**不是规范**）
- **工作线**：`line/ui-mcp`（worktree `yeban/.worktrees/ui-mcp`，基线 main `80d3022`）
- **所有者目录**：`crates/yeban-ui-mcp/**`（本台账是唯一新增的共享区文档）
- **规范来源**：
  - `docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md` **§12 全文**
    （§12.2 `[UI-TEST-001]`、§12.3 `[UI-MCP-001]`、§12.4 `[UI-TEST-002]`、§12.5 `[UI-MCP-002]`/`[UI-MCP-003]`）
  - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §1.3（`[ARCH-UI-003/004/005]`）、
    §7.1/§7.2/§7.3（`[ARCH-SEC-002]` 六级 scope、`[MCP-DUAL-001]` 双 MCP）
  - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` `ROAD-M4-001/002/008`、
    `MUST-GATE-009` / `MUST-GATE-010` / `MUST-GATE-015`
  - `docs/adr/ADR-0001`：**D5**（版本事实源）、**D18**（`SLINT_BACKEND=headless` /
    `slint::testing::*` 不存在、`Rgb8Pixel` 位置、Testing Backend 的可见性语义）、
    **D21**（path 依赖免通配）、**D22**（debug info 必须构建期打开）、**D23**（SSIM 阈值语义）、
    **D24**（字体）、**D25**（契约承重：根 `oneOf(ToolCall, ToolResponse)`）
  - 兄弟线台账：`docs/ledger/mcp-core-notes.md`（**必读**：token/0600、六级 scope、
    `ui:inject` 生产硬禁、鉴权先于解析、手写环回 HTTP、`.meta.` 约定与双射守卫）、
    `docs/ledger/ui-test-port-notes.md` + `docs/ledger/app-introspect-notes.md`
    （Tier-1 光栅化、`ElementHandle` 遍历、`visible:` 的几何语义、遮罩 SSIM、D23/D24 的边界）

> 本文件回答五个问题：**我交付了什么对应哪条规范**、**方法名与参数从哪推导出来**、
> **每条判据怎么变红**、**哪些东西明确没做**、**需要集成者/人类裁决什么**。

---

## 1. 落地清单（文件 ↔ 规范 ID）

| 文件（相对 `crates/yeban-ui-mcp/`） | 规范 ID | 说明 |
| :--- | :--- | :--- |
| `src/tree.rs` | `UI-TEST-001` `UI-MCP-001` `UI-MCP-002` | 语义控件树的 **JSON 投影**（不是第二套模型）：稳定键序、`visible` 的证据语义、动态区矩形（带 `id`）、双向覆盖 |
| `src/methods.rs` | `UI-MCP-001` `UI-TEST-002` `ARCH-UI-004` | **14 条方法**的注册表：名字 / 参数 / 所需 scope / 进程内操作 / 规范出处签名 + 参数校验（未知参数拒绝，M9 口径） |
| `src/service.rs` | `UI-MCP-001` `ARCH-SEC-002` `MUST-GATE-009` | 管线：**方法解析 → 授权（生产硬禁 → token → scope）→ 参数校验 → 执行**；错误码映射（复用 8 个 + 新增 3 个） |
| `src/surface.rs` | `MUST-GATE-015` | 执行面 `UiSurface`（= `UiTestPort` + Tier-1 像素）、`ShotEvidence`（尺寸非零/非全黑/颜色数/指纹）、`PortAdapter`（零 Slint 的真实接线点）、`fnv1a64` |
| `src/transport/mod.rs` | `MUST-GATE-009` `ROAD-M4-001` | 两道开关（编译期 feature + 运行期 flag）、端点常量 |
| `src/transport/stdio.rs` | `ROAD-M4-002` | 逐行 JSON-RPC（空行跳过、解析错误独立成行、notification 无输出） |
| `src/transport/http.rs` | `ARCH-UI-004` `ARCH-SEC-002` `MUST-GATE-009` | 环回 `127.0.0.1:0` HTTP（**复用** `yeban-mcp` 的报文层）、Bearer、`405/404/411/413/431/400/505`、单线程串行 accept |
| `src/base64.rs` | — | 零依赖 Base64（JSON 里传 PNG 字节） |
| `src/samples.rs` + `examples/export_ui_samples.rs` | `MUST-GATE-010` `TEST-SPEC-005` | 3 份 **`.meta.` 文档样本** + 跨语言对账入口（`Reconcile`）+ 前缀依赖自检 |
| `src/testing.rs`（`#[cfg(test)]`） | — | 判据共用的**零 Slint** 假执行面（真像素 + 真 PNG 编码器） |
| `tests/contract.rs` | `MUST-GATE-009` `MUST-GATE-010` `MUST-GATE-015` | 跨 crate 的承重判据（manifest 开关、与 Tier-1 渲染器的指纹对账、跨语言样本对账、公开入口的契约形状） |
| `src/lib.rs` | 全部 | 模块地图 + "这是 UI 控制面，不是 Intent API" + 与 `yeban-mcp` 的分工与共同安全模型 |

**没有新增任何依赖。** `serde` / `serde_json` / `thiserror` 三件早已在根
`[workspace.dependencies]` 登记；`yeban-mcp` / `yeban-ui-test-port` 是同工作区 crate。
**唯一**的依赖表变化是 `[dev-dependencies] yeban-mcp = { workspace = true, features = ["mcp-http"] }`
（理由见 §3.2）—— 外部依赖包数仍是 **618**，没有新包。

**改动到的共享文件（集成者需要知道）**：

| 文件 | 改了什么 | 为什么不可避免 |
| :--- | :--- | :--- |
| `Cargo.lock` | `yeban-ui-mcp` 条目多了依赖边（含 dev 边） | `license_inventory.py` 用 `cargo metadata --locked`，锁文件不同步就红 |
| `docs/ledger/dependency-licenses.md` | 机器再生成（5 处"直接使用方"新增 `yeban-ui-mcp`，`Cargo.lock` 摘要哈希更新） | `run-gates.sh light/crate` 内含 `license_inventory.py --check`，不生成就红。**多线并行时这是冲突热点**；合并时以再生成结果为准 |

未改：根 `Cargo.toml`、`.github/**`、`scripts/**`、`deny.toml`、`schemas/**`、
`docs/DEVELOPMENT_LEDGER.md`、`docs/adr/**`、`docs/YEBAN_*.md`、其它 `crates/**`、
`spikes/**`、法务文件。

---

## 2. 方法名与参数从哪推导出来（**规范里没有 JSON-RPC 方法名**）

四份规范只给**能力**与**签名**，没有一条给出方法名。本线采用的名字因此是**工程裁决**，
规则三条，逐条可复核：

1. **前缀 `ui/`** —— 与 §12.3 的 scope 命名空间 `ui:*` 对齐；领域 MCP 用 MCP 标准的
   `tools/list` / `tools/call`，两个端点因此不会撞名（判据 `method_names_are_unique_and_well_formed`
   显式断言 `ui/nope` 与 `tools/list` 都不在注册表里）。
2. **`ui/` 之后是规范点名的能力名**；事件注入**逐字采用 §12.4 的函数名**，让
   "规范里写了什么"与"线上叫什么"可以逐字对账（判据
   `spec_event_injection_methods_match_section_12_4` 断言 `signature` 里含规范原文）。
3. **参数用 camelCase**（`elementId` / `xOffset` / `keyCode`），与领域契约既有的
   `idempotencyKey` / `sectionName` 风格一致；§12.4 写的是 Rust 侧形参名 `element_id`，
   两者的对应关系写在每条方法的 `signature` 字段里，**不是**两份名字各自漂移。

| # | 方法名 | 参数 | scope | §12 出处 / 规范 ID |
| ---: | :--- | :--- | :--- | :--- |
| 1 | `ui/methods` | — | `ui:read` | `[ARCH-UI-004]`（JSON-RPC 查询控件树…）、`[MCP-DUAL-001]`（能力发现） |
| 2 | `ui/tree` | `prefix?`, `source?`(=`runtime`), `dynamicOnly?` | `ui:read` | §12.3 `[UI-MCP-001]`「控件树结构检索」 |
| 3 | `ui/node` | `elementId` | `ui:read` | §12.2 `[UI-TEST-001]`「必须严格基于语义 Element ID 检索」 |
| 4 | `ui/property` | `elementId`, `name` | `ui:read` | §12.3 `[UI-MCP-001]`「响应式属性读取」 |
| 5 | `ui/dynamic_regions` | — | `ui:read` | §12.5 `[UI-MCP-002]`「根据元素树元数据…矩形包围盒」 |
| 6 | `ui/screenshot` | `maskDynamic?`(默认 **true**), `maxBytes?` | `ui:screenshot` | §12.5「Framebuffer 捕获接口…PNG 二进制流」+ `[MUST-GATE-015]` |
| 7 | `ui/coverage` | `ids` | `ui:read` | §12.2 `[UI-TEST-001]` + `[ARCH-UI-005]`（Testing Backend 的行为边界） |
| 8 | `ui/dispatch_pointer_down` | `elementId`, `xOffset`, `yOffset`, `button` | `ui:inject` | §12.4 原文签名 `dispatch_pointer_down(element_id, x_offset, y_offset, button)` |
| 9 | `ui/dispatch_pointer_move` | `x`, `y` | `ui:inject` | §12.4 `dispatch_pointer_move(x, y)` |
| 10 | `ui/dispatch_pointer_up` | `button` | `ui:inject` | §12.4 `dispatch_pointer_up(button)` |
| 11 | `ui/dispatch_key_press` | `keyCode` | `ui:inject` | §12.4 `dispatch_key_press(key_code)` |
| 12 | `ui/switch_main_view` | `view` | `app:admin` | §12.3 Administrative「允许切换工作区主视图」 |
| 13 | `ui/force_save` | — | `app:save` | §12.3 Administrative「强制执行工程保存」 |
| 14 | `ui/reload_engine` | — | `app:reload-engine` | §12.3 Administrative「重载音频引擎」 |

### 2.1 两处**本地裁决**（不是规范明写的，需要人类复核）

1. **`ui/switch_main_view` 挂 `app:admin`**：`ARCH-SEC-002` 的六级 scope 里**没有**
   "界面状态写"这一级（只有 `ui:read` / `ui:screenshot` / `ui:inject` / `app:save` /
   `app:reload-engine` / `app:admin`）。本线取"最近语义"= `app:admin`（领域全量权限），
   而不是把它塞进 `ui:inject`（那会让"切换主视图"在生产模式被硬禁，与 §12.3 的
   Administrative 定位冲突）。**这条是裁决，不是抄来的** —— 见 §8 needs-1。
2. **`ui/screenshot` 的 `maskDynamic` 默认 `true`**：§12.5 把动态区遮罩写成 MUST，
   而本方法的唯一用途就是视觉回归比对；要原始帧必须显式 `maskDynamic: false`。
   动态区缺包围盒时**报错**（`-32009`），不静默给一张未遮罩的图。

### 2.2 错误码：复用 8 个 + 新增 3 个

复用 `yeban_mcp::jsonrpc` 的常量（**不重新定义**，否则两套 MCP 会漂移）：
`-32700` / `-32600` / `-32601` / `-32602` / `-32603` / `-32001`（未鉴权）/
`-32003`（已鉴权未授权）/ `-32005`（能力尚未接线）。

本线新增（JSON-RPC 的 `-32000..=-32099` 是服务自定义区，与复用的那些不撞号，有判据）：

| 码 | 常量 | 含义 | HTTP |
| :--- | :--- | :--- | ---: |
| `-32006` | `ELEMENT_NOT_FOUND` | 语义 ID 不在控件树里 | 400 |
| `-32008` | `CAPTURE_FAILED` | Tier-1 抓帧/编码失败（含 `[MUST-GATE-015]` 两条门槛） | 500 |
| `-32009` | `GEOMETRY_UNAVAILABLE` | 元素存在但没有几何（`[UI-MCP-002]` 的遮罩无从执行） | 400 |

**绝不**把它们塞进 `schemas/mcp-tools.schema.json` 的 `ToolResponse.error.code`：
那是**领域**契约的闭合 enum（D25），UI 控制面的实现级状况不属于它
（与 mcp-core notes §2 M10 同一口径）。

---

## 3. 与 `yeban-mcp` 的分工：哪些是**复用**、哪些是**新写**

### 3.1 复用的（**一行都没重写**）

| 复用项 | 用在哪 | 为什么不重写 |
| :--- | :--- | :--- |
| `yeban_mcp::security::{Scope, ScopeSet, RunMode, Channel, Credential, AuthContext, Denial}` | 全线的授权模型 | 六级 scope 与运行模式是**共享**的安全词汇；两份实现必然漂移 |
| `yeban_mcp::security::{BearerToken, TokenFile}` | token 生成 / `~/.yeban/session.token` / `0600` | 256-bit 熵源、权限位校验、符号链接拒绝都在那边有判据 |
| `yeban_mcp::security::{authenticate, authorize}` | **唯一**的判定入口 | `authorize` 的内部顺序（生产硬禁 → token → scope）就是 `ARCH-SEC-002` 的落点；重写一遍等于给"顺手把硬禁挪到 token 后面"留后门 |
| `yeban_mcp::jsonrpc::{Request, Response, ErrorObject, Id, 8 个错误码常量}` | 线格式 | `id` 回显、"`result` 与 `error` 恰好一个"、notification 不回复这些不变量已经在那边钉住 |
| `yeban_mcp::dispatch::{Outcome, http_status_for}` | 返回值形状与状态码映射 | 两条 MCP 对 `-32700/-32601/-32001/-32003/-32005` 的 HTTP 状态码因此**逐字节相同**（判据在 http.rs 的测试里） |
| `yeban_mcp::transport::http::{parse_head, split_message, assert_loopback, assert_loopback_peer, HttpRequest, HttpResponse, HttpError, SUPPORTED_METHOD, BEARER_CHALLENGE, MAX_REQUEST_*}` | 环回 HTTP 的**报文层** | 环回判定与报文边界条件是安全边界；那边的 21 条判据直接为这边工作 |
| `yeban_mcp::samples::check_document_sample` | `.meta.` 文档样本自检 | "顶层不许出现 `name`/`arguments`/`status`"这条判别键规则只能有一处 |
| `yeban_ui_test_port::{tree, port, image, mask, png}` | 控件树模型、三级权限调用面、像素、遮罩、PNG | 见 §3.3：这是"投影"而不是"第二套模型" |

### 3.2 新写的（**以及为什么不能复用**）

| 新写项 | 行数（约） | 为什么 |
| :--- | :--- | :--- |
| `transport::http` 的 accept / 读写驱动 | ~70 | `yeban_mcp::transport::http::HttpServer` 把 `Mutex<yeban_mcp::dispatch::Dispatcher>` 钉在结构体里；本线的分发器是 `UiService`。改那边的结构体越过文件边界（`crates/yeban-mcp/**` 属 `line/mcp-core`） |
| 同上：`HttpError::outcome()` 的替代（4 行） | 4 | 那个方法在 `yeban-mcp` 里是**私有**的；状态码与 `kind` 仍取那边的公开访问器 |
| 方法注册表 / 参数校验 / 管线 | ~700 | UI 控制面的方法集与领域工具集是不同的契约 |
| 树投影 + `visible` 的证据语义 | ~200 | `ControlNode` 里没有 `visible`；**不编造**（见 §4.2） |
| Base64 | ~80 | JSON 里传 PNG 字节；加一个 crate 不值得（与 ui-test-port 手写 PNG 同源理由） |
| `fnv1a64` | 6 | `render.rs` 依赖 Slint，引用它会让整块逻辑无法在本机跑；CI 侧判据 `fingerprint_matches_the_tier1_renderer` 逐位对账 |

**`[dev-dependencies]` 那条为什么存在（这是本线唯一需要解释的依赖写法）**：
`yeban-mcp` 的 `transport::http` 是 `#[cfg(any(feature = "mcp-http", test))]` 编译的 ——
对**依赖方**来说 `cfg(test)` 永远为假，所以只有 `mcp-http` 能让它出现在本 crate 的构建里。
我又不想让 `ui-mcp-http`（我的 feature）成为编译 `transport/http.rs` 的**唯一**条件：
CI 的 crate 腿跑 `cargo clippy/test -p yeban-ui-mcp --all-targets`（不带 `--features`），
那段代码会**从未被编译过** —— 未编译的代码连语法错都能躲过门禁（`DEVELOPMENT_LEDGER` 的 L6）。
因此 dev-dependency 打开 `mcp-http`，我的 http 模块用
`#[cfg(any(feature = "ui-mcp-http", test))]` 门控。

**实测（本机，resolver `3` + edition 2024）**：我建了一个最小探针 crate 复现这个形状，
`cargo test -p b` 时 dev-dependency 的 feature **会**并进同一次构建（`a::heavy_fn()`
在 `b` 的测试目标里可用且真的跑到了那一条判据）；而 `cargo build -p b` **不会**打开它。
默认 release 构建里因此没有监听循环，红线 6 未被削弱
（判据 `manifest_default_features_do_not_enable_ui_mcp_http` + `transport` 的开关判据）。

### 3.3 控件树：为什么是**投影**而不是第二套模型

模型（`ControlTree` / `ControlNode` / `Role` / `Rect`）住在 `yeban_ui_test_port::tree`，
由 `yeban-app` 的适配器与 `inspect::tree_from_element_root` 两条路填充。本线的 `src/tree.rs`
只把它投影成线格式，多出来的只有两样模型里没有的东西：

- `source`（`runtime` / `registry`）：调用方据此知道 `visible` 的**证据强度**；
- `visible`：`runtime` 且有几何 ⇒ `true`；无几何 ⇒ **`null`**（不可断定）。

**永不输出 `false`**：上游遍历是"几何裁剪相交"的结论（`i-slint-core-1.18.1/item_tree.rs:410-419`，
见 ui-test-port notes §2 第 15 条），不可见的分支**根本不进树**；注册表又不知道可见性。
编造 `false` 会让调用方以为"这个元素存在但被隐藏了"。这条有判据
（`visible_is_null_when_there_is_no_geometry_evidence`：断言任何投影都不输出 `Some(false)`）。

---

## 4. 判据清单（本机 52 条 + CI 侧 5 条）

| 主题 | 条数 | 代表判据 |
| :--- | ---: | :--- |
| Base64 | 3 | `every_byte_value_round_trips`（全 256 个字节值 + 全长度） |
| 控件树投影 | 6 | `json_is_byte_stable_regardless_of_insertion_order`、`dynamic_region_rects_come_from_the_runtime_bounds`、`visible_is_null_when_there_is_no_geometry_evidence`、`filtering_keeps_id_order_and_source`、`coverage_reports_both_directions_deterministically` |
| 方法注册表 / 参数 | 5 | `method_names_are_unique_and_well_formed`、`port_operation_tier_matches_scope`、`spec_event_injection_methods_match_section_12_4`、`param_validation_rejects_every_malformed_shape`、`catalogue_is_a_document_and_matches_the_registry` |
| 执行面 / 像素证据 | 5 | `screenshot_evidence_requires_a_non_black_non_empty_frame`、`masking_happens_before_encoding`、`oversized_png_is_an_explicit_error`、`port_adapter_delegates_everything_and_injects_pixels` |
| 管线 / 安全（`service.rs`） | 13 | `unauthenticated_calls_are_rejected_and_the_raw_entry_parses_nothing`、`raw_entry_point_never_leaks_the_method_set`、`insufficient_scope_names_the_required_scope`、`event_injection_is_hard_denied_in_production_before_token_check`、`tree_method_result_is_byte_stable_across_two_calls`、`unknown_semantic_id_is_an_explicit_error_not_an_empty_success`、`screenshot_returns_a_real_non_black_png_with_evidence`、`dynamic_regions_are_masked_from_the_runtime_bounds`、`in_process_permission_is_a_second_independent_gate`、`methods_discovery_reports_scopes_and_inject_availability`、`coverage_reports_both_directions`、`notifications_are_silent_and_administration_is_honestly_reported`、`malformed_params_are_invalid_params` |
| stdio 传输 | 2 | `processes_lines_and_reports_errors_on_their_own_line`、`stdio_channel_cannot_inject_in_production` |
| 两道开关 | 2 | `http_is_off_unless_explicitly_enabled_at_runtime`、`enabling_http_without_the_feature_is_an_explicit_error` |
| 环回 HTTP | 7 | `bind_is_loopback_only_and_uses_a_dynamic_port`、`missing_and_wrong_tokens_are_rejected_with_401`、`ui_inject_is_still_hard_denied_in_production_over_http`、`protocol_errors_have_explicit_statuses`、`end_to_end_over_a_real_loopback_socket` |
| 样本导出 / 跨语言对账 | 7 | `exported_sample_set_is_exactly_the_three_documents`、`documents_do_not_look_like_contract_instances`、`export_is_byte_stable`、`documents_match_the_implementation`、`reconciliation_with_the_domain_contract_instances`、`ui_samples_alone_leave_the_prefix_without_a_real_instance`、`default_out_dir_is_the_workspace_target` |
| crate 级 | 2 | `crate_level_contract_holds`、`every_scope_has_at_least_one_method` |
| **CI 侧**（`tests/contract.rs`） | 5 | `manifest_default_features_do_not_enable_ui_mcp_http`、`fingerprint_matches_the_tier1_renderer`、`samples_reconcile_with_the_domain_contract`、`ui_samples_alone_are_meta_only_and_the_prefix_guard_says_so`、`public_entry_point_behaves_like_the_documented_contract` |

### 4.1 本机**真跑**的方法（零 Slint 探针）

本机纪律禁止编译 Slint（`run-gates.sh crate yeban-ui-mcp` 会自动 SKIP 本机档位），
但本 crate **自己的代码零 Slint**，所以整块逻辑可以在本机真跑。做法（与
`ui-test-port` notes §7.1 同一套，只是往上再套一层）：

```bash
# 1) 把 ui-test-port 的**零 Slint 子集**编译成一个真 rlib（源码用 #[path] 指到仓库真文件）
rustc --edition 2024 --crate-type lib --crate-name yeban_ui_test_port --cap-lints allow \
  -L dependency=target/debug/deps \
  --extern serde=...rlib --extern serde_json=...rlib stub_lib.rs -o /tmp/uimcp-probe/libyeban_ui_test_port.rlib
# 2) 用**真实**的 yeban_mcp rlib（本机 `cargo build -p yeban-mcp --features mcp-http` 产出）
# 3) 直接对 crates/yeban-ui-mcp/src/lib.rs 跑 --test
rustc --edition 2024 --test --crate-name yeban_ui_mcp -D warnings -D rust_2018_idioms \
  -L dependency=target/debug/deps -L dependency=/tmp/uimcp-probe \
  --extern yeban_ui_test_port=/tmp/uimcp-probe/libyeban_ui_test_port.rlib \
  --extern yeban_mcp=target/debug/deps/libyeban_mcp-<hash>.rlib --extern serde=... --extern serde_json=... --extern thiserror=... \
  --cfg 'feature="ui-mcp-http"' crates/yeban-ui-mcp/src/lib.rs -o /tmp/uimcp-probe/uimcp_tests
CARGO_MANIFEST_DIR=$PWD/crates/yeban-ui-mcp /tmp/uimcp-probe/uimcp_tests
```

**结果：`52 passed; 0 failed`，`-D warnings` 零告警**（含 `#![deny(missing_docs)]` /
`rust_2018_idioms` / clippy 无法在本机跑）。

**验证范围声明（重要）**：这 52 条覆盖 Base64 / 控件树投影 / 方法注册表 / 参数校验 /
管线与授权 / 三个传输 / 样本导出与**跨语言对账** / 像素证据与遮罩。
**不覆盖**：`tests/contract.rs` 的 5 条（它引用 `yeban_ui_test_port::render`，即 Slint）、
以及**任何**含 Slint 的编译（`cargo clippy -p yeban-ui-mcp --all-targets -- -D warnings`
与 `--features ui-mcp-http` 的组合）。那些**只能由 CI 判**。

### 4.2 跨语言契约对账（Python `jsonschema` 4.24.0 / Draft 2020-12）—— 本机**真跑过**

```text
[yeban-ui-mcp] 跨语言契约对账 (本线 3 份 .meta. + 领域真实例):
  [ok] mcp-tools.call.yeban_*.json ×10: 通过 mcp-tools.schema.json
  [ok] mcp-tools.response.dry-run.json: 通过 mcp-tools.schema.json
  [skip] mcp-tools.ui-{methods,security,tree-projection}.meta.json: 文档样本(.meta.), 不对账 schema
  契约校验通过 (4 份 schema)。

[yeban-ui-mcp] 只导出本线样本时的脚本结论 (期望红):
  退出码 Some(1) …
  契约校验未通过:
    - 前缀 `mcp-tools` 只有 .meta. 文档样本, 没有任何真实例 —— 该契约等于没有对账(全是 meta 就是假绿)
```

两条都是**实测**：前者是"本线 `.meta.` + 领域真实例"的绿；后者是"只有 `.meta.`"的**响亮红**
（这条依赖被判据 `ui_samples_alone_leave_the_prefix_without_a_real_instance` 钉住，
不是一句口头约定）。三条反证（都在判据里真跑，退出码为准）：

| # | 构造 | 期望 | 实测 |
| :--- | :--- | :--- | :--- |
| 1 | 混入非法实例 `{"anything":[1,2,3]}` | 红 | ✅ `Reconcile::Failed` |
| 2 | 只留 `.meta.` 文档样本 | 红（前缀守卫） | ✅ `Reconcile::Failed`，指名"只有 .meta." |
| 3 | 把 1 份文档改名成不带 `.meta.` 的"实例"（注入 F） | 红 | ✅ 5 条判据红（见 §5 F） |

> ⚠ 诚实边界：`jsonschema` 只在 **CI 的 `checks` 腿**装着（`rust` 腿不装）。
> 缺依赖时 `Reconcile::Skipped` 会打一行大写的 **LOUD SKIP**，判据**不算通过**也不伪装红 ——
> 与 mcp-core 的做法一致（notes §3.2 的诚实边界 (a)）。

### 4.3 本机**没做**的（纪律要求，交给 CI）

- `cargo clippy/test -p yeban-ui-mcp --all-targets`（含 Slint）、`--features ui-mcp-http` 的组合；
- 任何 `--workspace` 全量构建、基准、模糊测试；
- 从未在本机链接/运行过含 Slint 的二进制。

本机**通过**的门禁：

```text
bash scripts/gates/run-gates.sh light                     # exit 0, "门禁通过 (mode=light)"
  fmt / 13 条机械红线守卫 (G01..G13) / 文档 / 依赖许可清单 ✓
cargo metadata --locked                                   # exit 0
cargo-deny --all-features check                           # advisories ok, bans ok, licenses ok, sources ok
bash scripts/dev/cargo-local.sh build -p yeban-mcp --features mcp-http   # 供探针链接的真 rlib
```

---

## 5. 注入 → 变红 → 还原（**6 条，全部真做过**）

方法：把源文件备份到 `/tmp/uimcp-backup/`，注入后用零 Slint 探针重新编译并跑全部 52 条判据，
记录红掉的**判据名**，再从备份还原并跑一遍全绿（`diff` 逐字节确认复原）。

| # | 注入 | 变红的判据 | 还原 |
| :--- | :--- | :--- | :--- |
| **A** | `service.rs`：scope 判定永远放行（硬禁与 token 仍判） | `service::insufficient_scope_names_the_required_scope`、`service::event_injection_is_hard_denied_in_production_before_token_check` | ✅ 52/52 |
| **B** | `service.rs`：**删掉生产硬禁**（`authorize` 结果被丢弃，只留鉴权 + scope） | `service::event_injection_is_hard_denied_in_production_before_token_check`、`transport::http::ui_inject_is_still_hard_denied_in_production_over_http`、`transport::stdio::stdio_channel_cannot_inject_in_production` | ✅ 52/52 |
| **C** | `tree.rs`：投影节点顺序改成逆序（依赖插入顺序） | `tree::json_is_byte_stable_regardless_of_insertion_order`、`tree::filtering_keeps_id_order_and_source`、`tree::dynamic_region_rects_come_from_the_runtime_bounds`、`service::tree_method_result_is_byte_stable_across_two_calls`、`service::dynamic_regions_are_masked_from_the_runtime_bounds`、`samples::export_is_byte_stable`（共 6 条） | ✅ 52/52 |
| **D** | `tree.rs`：`mask_rects` 返回**硬编码坐标** | `tree::dynamic_region_rects_come_from_the_runtime_bounds`、`tree::role_label_parent_and_maskable_are_projected_faithfully`、`service::dynamic_regions_are_masked_from_the_runtime_bounds`、`transport::http::end_to_end_over_a_real_loopback_socket`（共 4 条） | ✅ 52/52 |
| **E** | `methods.rs`：未知参数被静默放行（M9 口径被破坏） | `service::malformed_params_are_invalid_params` | ✅ 52/52 |
| **F** | `samples.rs`：把一份文档样本改名成不带 `.meta.` 的"实例" | `samples::exported_sample_set_is_exactly_the_three_documents`、`samples::documents_do_not_look_like_contract_instances`、`samples::export_is_byte_stable`、`samples::reconciliation_with_the_domain_contract_instances`、`samples::ui_samples_alone_leave_the_prefix_without_a_real_instance`（共 5 条） | ✅ 52/52 |

**方法学留痕**：注入 C 的第一次尝试**编译失败**（`nodes.reverse()` 需要 `let mut nodes`），
我把它记在这里而不是当成"变红"——**编译错误不是判据**。第二次（C′）改对了绑定再跑，才拿到上表的结果。

---

## 6. 安全自查清单（逐条给证据）

| # | 要求 | 落点（代码） | 证据（判据） |
| ---: | :--- | :--- | :--- |
| 1 | 网络监听**默认关**（编译期 + 运行期两道开关） | `Cargo.toml` 的 `default = []`；`transport::plan_http_startup` | `transport::http_is_off_unless_explicitly_enabled_at_runtime`、`enabling_http_without_the_feature_is_an_explicit_error`、`tests/contract.rs::manifest_default_features_do_not_enable_ui_mcp_http` |
| 2 | 只绑 **`127.0.0.1`**，端口 `0` 动态分配 | `UiHttpServer::bind_loopback` + 复用 `assert_loopback` + 绑定后**回读** `local_addr()` | `transport::http::bind_is_loopback_only_and_uses_a_dynamic_port`（并用 `Ipv4Addr::UNSPECIFIED` 构造"绝不允许的地址"，**不写那个字面量** —— G04 是文本守卫，本判据不依赖它） |
| 3 | Bearer Token 256-bit，落 `~/.yeban/session.token`，权限恰好 **`0600`** | 复用 `yeban_mcp::security::{BearerToken, TokenFile}`（本线**无**自己的 token I/O） | `yeban-mcp` 的 11 条 token 判据；本线只判"缺失/错误/形状非法 ⇒ 401"（`transport::http::missing_and_wrong_tokens_are_rejected_with_401`、`service::unauthenticated_calls_are_rejected_and_the_raw_entry_parses_nothing`） |
| 4 | 请求必须带 `Authorization: Bearer <TOKEN>`，**不回退放开** | 复用 `authenticate`；`401` 带 `WWW-Authenticate: Bearer realm="yeban-mcp"` | 同第 3 条；另加 `end_to_end_over_a_real_loopback_socket`（真环回 socket） |
| 5 | 六级 scope + `app:admin` **不**隐含 `ui:*` | 复用 `ScopeSet::grants`；每条方法声明 `scope` | `service::insufficient_scope_names_the_required_scope`（逐条断言 `requiredScope`/`grantedScopes`，并断言 `app:admin` 不隐含三个 `ui:*`、`ui:*` 拿不到 `app:save`）、`methods::port_operation_tier_matches_scope` |
| 6 | `ui:inject` 生产**硬禁**，且判定**先于** token 校验 | 复用 `authorize`（它第一件事就是硬禁） | `service::event_injection_is_hard_denied_in_production_before_token_check`：生产模式下**无 / 错 / 合法**三种凭据都拿到 `403 forbidden-in-production`（不是 401）；测试模式 + 显式授予才落到执行面；另有 HTTP 与 stdio 两条端到端版本。**注入 B 证明它承重** |
| 7 | 鉴权**先于**解析（原始入口不泄漏方法集） | `UiService::handle_line` 先 `authenticate` 再 `Request::parse` | `service::raw_entry_point_never_leaks_the_method_set`（未鉴权调**不存在**的方法拿 401 而不是 -32601）、`unauthenticated_calls_are_rejected_and_the_raw_entry_parses_nothing`（错 token + 非法 JSON ⇒ 401，不是 -32700） |
| 8 | 纵深防御：进程内三级权限是**第二道**闸门 | `UiSurface: UiTestPort`（端口自己的 `authorize`） | `service::in_process_permission_is_a_second_independent_gate`（scope 全给 + 测试模式，`ReadOnly` 端口仍拒注入 → `403 port-permission-denied`，且实现侧一次都没被调用） |
| 9 | 截图必须**尺寸非零且非全黑** | `surface::encode_with_evidence` + `ShotEvidence` | `surface::screenshot_evidence_requires_a_non_black_non_empty_frame`、`service::screenshot_returns_a_real_non_black_png_with_evidence` |
| 10 | 无法执行的 MUST 要**报错**，不许静默降级 | `screenshot`/`dynamic_regions` 在动态区缺几何时报 `-32009` | `service::dynamic_regions_are_masked_from_the_runtime_bounds`（缺几何 ⇒ 报错；但显式 `maskDynamic:false` 时是调用方的选择，照常返回） |

---

## 7. CI 判决（读到什么写什么）

- 分支：`line/ui-mcp`，基线 main `80d3022`。
- 读取方式：`bash scripts/dev/ci-verdict.sh line/ui-mcp`（**只有 CI 的判决算数**；
  本机绿是参考，未读回来的判决一律记 `pending`）。

| 轮次 | 头部 | run id | 结论 |
| :--- | :--- | ---: | :--- |
| 1 | 见 §9 的提交 | 见 §7.1 | 见 §7.1 |

### 7.1 第 1 轮

- 状态：**见本文件提交之后的读数**（写入判决会让头部前进一格；本文件是文档改动，
  不影响 `checks` 的 fmt/守卫/契约对账，也不影响 `rust` 腿的 clippy/test）。
- 集成者需要先加的那一行见 §8 needs-0：**不加也能过**（本线的样本是 `.meta.`，
  但"只导出本线样本"会让 `checks` 腿在对账时**红**）——也就是说，
  **如果集成者把 `export_ui_samples` 接进 CI，必须接在 `export_mcp_samples` 之后**；
  如果**不**接，本线的样本导出就等于没有对账入口（判据
  `tests/contract.rs::samples_reconcile_with_the_domain_contract` 仍然会在
  `rust` 腿里自己跑一次对账，因此"不接 CI"的代价只是少一处独立确认，而不是空转）。

---

## 8. needs（需要别人做）

0. **【集成者】把样本导出接进 `ci.yml` 的 `checks` 腿**，位置必须在
   `export_mcp_samples` **之后**：

   ```yaml
   cargo run -p yeban-ui-mcp --locked --example export_ui_samples -- --out target/schema-samples
   ```

   理由（**顺序不能反**）：本线 3 份样本全是 `.meta.` 文档样本，对账脚本的前缀守卫要求
   `mcp-tools` 前缀至少有一份**真实例**，那份实例由 `export_mcp_samples` 提供。
   `.github/**` 是集成者独占文件，本线不改（理由与 §4.2 的实测一致）。
1. **【人类裁决】六级 scope 里没有"界面状态写"这一级**：`ui/switch_main_view` 目前挂
   `app:admin`（最近语义）。要不要新增一个 scope，或者明确它属于 `ui:inject`
   （从而在生产模式被硬禁），需要裁决 —— 本线不自造 scope。
2. **【人类裁决】§12.3 的三级权限与六级 scope 是否要合并**：目前是**两层**（端口 `Permission`
   与网络 `Scope`），本线把两张表钉在一起（判据 `port_operation_tier_matches_scope`），
   但"两层是不是规范的本意"没有明文。
3. **【app 侧接线】真实执行面的两行**（本线提供 `PortAdapter`，但不改 `yeban-app`）：

   ```rust
   let port = yeban_ui_test_port::render::LivePort::new(size, permission, Some(&registry), build)?;
   let surface = yeban_ui_mcp::PortAdapter::new(port, "tier1-live-port", |port| {
       port.window().capture().map_err(|e| yeban_ui_test_port::port::PortError::Capture { message: e.to_string() })
   });
   ```

   有了它，`ui/screenshot` / `ui/dynamic_regions` / 事件注入才会落到**真实界面**上。
4. **CI 侧需要一次"含 Slint 的编译"才算真判据**：本机从未编译过
   `crates/yeban-ui-mcp`（`run-gates.sh` 会 SKIP）。第一次 CI 判决是**唯一**能证明
   `PortAdapter` 与 `UiService` 的泛型/生命周期写法在真实依赖图上成立的东西。
5. **`ui/screenshot` 与 `ssim` 的关系**：本线**不做** PNG 解码，因此不提供"跨进程图-图比对"
   方法；进程内的"遮罩 + SSIM"仍由 `yeban-ui-test-port::{mask, ssim}` 提供
   （`compare_with_dynamic_masking`）。要不要在 UI 控制面上暴露一个"给两张 golden 路径做比对"
   的方法，取决于"基准图库怎么维护"这条更大的裁决（见 ui-test-port notes §10 第 5/6 条）。

---

## 9. 边界 / pending / TODO(hoist)

### 边界（**明确没做**，不是遗漏）

- **不实现 Tier-1 光栅化**：那是 `yeban-ui-test-port::render` 的事；本线只消费一张
  `Rgb8Image`，并断言它是非零且非全黑的。
- **不做 PNG 解码 / SSIM 比对**（见 needs-5）。
- **不接线 `yeban-app`**：`PortAdapter` 是接口，注入真实窗口是 app 侧的两行（needs-3）。
- **管理动作只做授权与转发**：`ui/force_save` / `ui/reload_engine` / `ui/switch_main_view`
  在端口未接线时**如实**返回 `-32005 NOT_IMPLEMENTED`（判据
  `notifications_are_silent_and_administration_is_honestly_reported`），绝不假装成功。
- **环回 HTTP 是单线程串行**（不是每连接一线程）：真实执行面持有 Slint 组件（**不是 `Send`**），
  要求 `Send` 会把真实接线排除掉。代价写在 `transport/http.rs` 的文件头。
- **不做 TLS / HTTP2 / keep-alive / chunked / 慢速攻击防护**（与 `yeban-mcp` 同款边界）。

### pending（未证实的、已知的债）

1. **本线 CI 是否真的绿** —— 见 §7；未读到判决前一律记 `pending`。
2. **含 Slint 的编译从未在本机发生过**（本机纪律）⇒ `PortAdapter` / `UiService` 在真实
   依赖图上的编译正确性只由 CI 判。
3. **`crates/yeban-app` 侧的接线未做**（needs-3）⇒ "UI 控制面能驱动真实界面"这件事
   **尚未被端到端证明**；目前证明的是"控制面 + 端口抽象 + 假执行面"这条链路。
4. **`docs/ledger/dependency-licenses.md` 被本线重新生成**（5 处"直接使用方" + 锁摘要哈希）。
   共享文件，多线并行时是冲突热点。
5. **`cargo clippy` 未在本机跑过**（需要 Slint）⇒ `clippy::all` 的告警只能由 CI 抓
   （ui-test-port 的经验：本机 `rustc` 不跑 lint 组，曾漏掉 `chunks_exact_to_as_chunks`
   与 `doc_overindented_list_items`）。已按那边的教训逐条防过（避免 `assert_eq!(x, true)`、
   避免 `bool` 比较、不写 `#[allow]`），但**第一次 CI 的 clippy 才是真判决**。

### TODO(hoist)

**没有。** 本线没有新增任何外部依赖（`serde` / `serde_json` / `thiserror` 早已在
`[workspace.dependencies]`；`yeban-mcp` / `yeban-ui-test-port` 是同工作区 crate），
因此根 `Cargo.toml` 不需要任何改动。
