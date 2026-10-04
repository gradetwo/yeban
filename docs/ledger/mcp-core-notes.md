# `mcp-core` 工作线台账：落地清单、契约实测、判据与未决项

- **台账类型**：交付映射 / 契约实测 / 判据清单 / 未决项（**不是规范**）
- **工作线**：`line/mcp-core`（worktree `yeban/.worktrees/mcp-core`）
- **所有者目录**：`crates/yeban-mcp/**`（本台账是唯一新增的文档文件）
- **规范来源**：
  - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §7.1 / §7.2（`ARCH-SEC-002`、`MCP-TOOL-001..010`）
  - `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` `ROAD-M4-001/002/003`、`MUST-GATE-009`
  - `schemas/mcp-tools.schema.json`（工具名集合 / `dryRun` / `idempotencyKey` / 错误码的机器契约）
  - `docs/adr/ADR-0001-workspace-topology-and-version-pinning.md`（D5 版本事实源、D20/D21 依赖政策、**D25 契约修复**）
  - 兄弟线样板：`crates/yeban-model/src/samples.rs`（样本导出）、`crates/yeban-model/src/ids.rs`（手写 serde）

> 本文件回答五个问题：**我交付了什么对应哪条规范**、**契约实测出来到底长什么样**、
> **每条判据怎么变红**、**哪些东西明确没做**、**需要谁裁决什么**。

## 0. 本轮时间线（两轮）

| 轮次 | 内容 |
| :--- | :--- |
| 第 1 轮 | 交付全量实现 + 108 条判据；**实测出两处契约缺陷**（错误码 enum 装不下规范并集、schema 根不引用 `definitions`）并如实登记为待裁决；CI run **37225147123 全绿**（checks / deny / lockfile / rust workspace 全量） |
| 集成者 | 按 **ADR-0001 D25** 在 main 修好契约（错误码联集 20 值 + 根 `oneOf($ref ToolCall, $ref ToolResponse)`），并把 mcp 样本导出接进 `ci.yml` |
| 第 2 轮（本轮） | rebase 到 main；**判据从"钉住缺口"升级成"实现集合 == 契约集合"**；目录样本改成 `ToolResponse` 形状（新根下裸对象会被拒）；新增"故意违法样本必须让契约变红"的**承重**判据；110 条判据全绿 |

---

## 1. 落地清单（文件 ↔ 规范 ID）

| 文件 | 规范 ID | 说明 |
| :--- | :--- | :--- |
| `src/tools.rs` | `MCP-TOOL-001..010`、`ROAD-M4-003` | 十个 `yeban_*` 工具的 `const` 注册表（规范顺序）+ 参数 schema + 副作用分级 + 错误码目录（联集 20 + 实现级 1）+ `ToolCall`/`ToolResponse` |
| `src/security.rs` | `ARCH-SEC-002`、`MUST-GATE-009` | 256-bit token 生成、`~/.yeban/session.token` 的 `0600` 读写与**权限校验**、六级 scope、`ui:inject` 生产硬禁、`authenticate`/`authorize` 唯一判定入口 |
| `src/jsonrpc.rs` | `MCP-TOOL-001..010` | JSON-RPC 2.0 最小实现，手写 `Serialize`（`id` 回显、`result`/`error` 恰好其一、notification 不回复） |
| `src/dispatch.rs` | `MCP-TOOL-001..010` | 解析 → 鉴权（**先于解析**）→ scope → 工具分发 → `dryRun` 短路 → `BTreeMap` 幂等去重 |
| `src/transport/stdio.rs` | `ROAD-M4-002` | 逐行 JSON-RPC 批处理（空行跳过、解析错误独立成行、notification 无输出） |
| `src/transport/http.rs` | `ARCH-SEC-002`、`MUST-GATE-009`、`ROAD-M4-001` | `TcpListener::bind(127.0.0.1:0)` + 线程手写最小 HTTP/1.1（`POST` only、`Content-Length` 必填、超长拒绝、Bearer 校验） |
| `src/transport/mod.rs` | `MUST-GATE-009` | 两道开关的纯函数判定（编译期 feature + 运行期显式开关） |
| `src/bin/yeban-mcp.rs` | `ROAD-M4-002` | 双形态 CLI：stdio 批处理 / `--print-token` / `--enable-mcp-http`；退出码 0/1/2 |
| `src/samples.rs` + `examples/export_mcp_samples.rs` | `MUST-GATE-010`、`TEST-SPEC-005` | 12 份规范样本导出（**非测试**入口，与 `yeban-model` 同一做法） |
| `tests/contract.rs` | `MCP-TOOL-001..010`、`MUST-GATE-010` | 直接读 `schemas/mcp-tools.schema.json` 的**承重**判据（集合相等 + 根 `oneOf` + 故意违法样本必红） |
| `src/lib.rs` | — | 模块地图 + 默认安全模型 + 契约对账现状 + 实现状态 |

**没有新增任何依赖。** `serde` / `serde_json` / `thiserror` 三件都在根 `[workspace.dependencies]`
里已登记（`yeban-model` 已在用），因此**没有 `TODO(hoist)`**。
HTTP 传输拒绝 `tiny_http` 一类的小依赖：需求只是"把一行 JSON 从环回 socket 搬进
`Dispatcher` 再搬回去"，手写 ~300 行换来的是逐行可审计的安全边界（见
`src/transport/http.rs` 的文件头说明）。

**改动到的共享文件（集成者需要知道）**：

| 文件 | 改了什么 | 为什么不可避免 |
| :--- | :--- | :--- |
| `Cargo.lock` | `yeban-mcp` 包条目多了 3 条依赖边 | `license_inventory.py` 用 `cargo metadata --locked`，锁文件不同步就红 |
| `docs/ledger/dependency-licenses.md` | 机器再生成（rebase 后重跑一次），`yeban-mcp` 被加为 serde / serde_json / thiserror 的直接依赖方 | `run-gates.sh light/crate` 档位内含 `license_inventory.py --check`，不生成就红 |

未改：根 `Cargo.toml`、`.github/**`、`scripts/**`、`deny.toml`、`schemas/**`、
`docs/DEVELOPMENT_LEDGER.md`、`docs/adr/**`、`docs/YEBAN_*.md`、其它 `crates/**`、`spikes/**`、法务文件。

---

## 2. 决策记录（本地编号 M1..M11）

### M1 —— HTTP 传输用 `std::net` 手写，零新增依赖

`TcpListener::bind(("127.0.0.1", 0))` + 一线程一连接（`std::thread::scope`）。
支持 `Content-Length`、只接受 `POST`、拒绝超长、校验 `Authorization`。
**明确不做**（写在 `http.rs` 文件头，不是"忘了"）：`chunked`/`Transfer-Encoding`、
HTTP/2、TLS、keep-alive、请求流水线、慢速攻击防护。

### M2 —— `http` 模块的 `cfg` 里为什么要有一个 `test`

任务书要求 `#[cfg(feature = "mcp-http")]`。**只**用这一门会有个更糟的后果：
CI 默认档位不传 `--all-features`（见 `.github/workflows/ci.yml` 的 `rust` 矩阵腿：
`cargo clippy/test -p <crate> --all-targets --locked`），于是这段最危险的代码
**在本仓库的 CI 里从未被编译过** —— 未编译的代码连语法错都能躲过门禁，
`crates/yeban-app/src/test_port_adapter.rs` 已经付过一次学费（`ci.yml` 里那段注释）。

所以落成 `#[cfg(any(feature = "mcp-http", test))]`：

- `cfg(test)` 下模块参与编译 ⇒ 21 条 HTTP 判据与 `clippy -D warnings` 都真的跑到了；
- **真正的监听循环** `HttpServer::serve_forever` 仍然只在 `feature = "mcp-http"` 下存在
  ⇒ 默认 release 构建里既没有监听循环，也没有任何链接进来的 HTTP 代码；
- `MUST-GATE-009` 的"默认关闭"由 `Cargo.toml` 的 `default = []` + 运行期开关两层保证，
  没有被这个决定削弱（判据 `manifest_default_features_do_not_enable_mcp_http` 钉住）。

### M3 —— 鉴权**先于**解析（未鉴权调用方拿不到解析反馈）

`Dispatcher::handle_line`（stdio 与 HTTP body 共用的原始文本入口）在解析 JSON
**之前**先过 token 闸门：缺 token / 错 token / 形状非法 ⇒ `401`，`id` 为 `null`，
**请求体一个字节都不会被解释**。带合法 token 但 JSON 非法时才回到 `400 / -32700`。

- 好处：未鉴权调用方无法用"400 还是 401"的差异探测本机解析器或工具集；
- 代价：401 响应里的 `id` 只能是 `null`（规范对"没能确定 id"的形状）；
- `Dispatcher::handle`（已解析的请求）仍然回显 `id`，两条入口的差异有判据钉住
  （`authentication_precedes_parsing_on_the_raw_entry_point`）。

早期版本的实测反例：`curl -X POST ... -d '{}'` 在没有 token 时拿到 **400**。
现在拿到 **401**（见 §4.3 的实测表）。这是"判据驱动出来的修改"，不是猜的。

### M4 —— HTTP 线上路径：方法 / 端点先判，再读体

`handle_stream` 的顺序是 `方法 → 端点 → Content-Length → 体 → 分发`。
早期版本先要 `Content-Length`，于是一个不带体的 `GET` 会拿到 `411` 而不是 `405`
（`handle_text` 那条路径却给 `405` —— 两套口径）。回归判据见 §4.4 的 C 行与
`stream_path_checks_method_before_content_length`。

### M5 —— `app:admin` **不**隐含任何 `ui:*`

`ScopeSet::grants` 三条规则：精确包含 ⇒ 通过；`required` 是 `ui:*` ⇒ 只有精确包含才通过；
`required` 是 `app:*` ⇒ `app:admin` 是超集。
理由：`ui:inject` 的"生产硬禁"如果是"有 admin 就隐含"的话，硬禁就变成一句空话。

### M6 —— `ui:inject` 硬禁的检查顺序在生产模式里**先于** token 校验

`authorize` 的第一件事就是 `required.is_production_forbidden() && mode.is_production()`。
于是"顺手拿一个**合法** token 去注入事件"在生产模式下也一定被拒
（`forbidden-in-production`，403），而不是"先过了 token 再说"。

### M7 —— `dryRun` 不写幂等缓存；空 `idempotencyKey` 按"未提供"处理

- `dryRun` 只做参数校验 + 回报"将要做什么"，**不**记录幂等键 ——
  否则一个 `dryRun` 会把它后面的真调用顶掉（判据
  `dry_run_short_circuits_without_recording_idempotency`）。
- 契约没有给 `idempotencyKey` 任何 `minLength`；把空串当成"必须去重"会让所有
  老客户端莫名其妙命中同一条缓存。因此空串 = 未提供，**且这条选择有判据与文档**
  （`empty_idempotency_key_is_treated_as_absent`）。

### M8 —— 幂等重放用**当前** `id` 重新组装响应

缓存里存的是"不带 `id` 的响应载荷"。重放时用当前请求的 `id` 组装，
并把整条响应包进 `{"replayed": true, "response": {...}}` 信封 ——
让调用方**看得见**自己拿到的是缓存。若把首次响应原样吐回去，就会回显一个陈旧的 `id`。

### M9 —— 未知参数被拒绝，`_` 前缀的 MCP 保留键除外

`ToolSpec::validate_arguments` 拒绝拼错的参数（例如 `dryrun`）。
静默忽略一个拼错的参数会让 Agent 以为自己的意图生效了 —— 那是最坏的一种"成功"。

### M10 —— "尚未实现"走 JSON-RPC `-32005`，不伪造 `ToolResponse`

契约的 `ToolResponse.error.code` 是**闭合** enum（D25 之后是 20 值，仍然闭合），
而 `NOT_IMPLEMENTED` 不在其中（也不该在：它不是领域失败，是"这条能力还没接线"）。
所以**实现级**状况一律走 JSON-RPC 错误对象；领域失败才走 `ToolResponse`。
判据 `implementation_error_codes_equal_the_contract_enum_exactly` 里有一条显式断言：
`NOT_IMPLEMENTED` 不许进契约路径（实测：把它塞进 `ToolResponse.error.code` 会被契约拒）。

### M11 —— 错误码判据从"钉住缺口"升级成"集合相等"

第 1 轮 `ErrorCode::SCHEMA_CONTRACT` 是 schema 原来的 7 值，判据把
"规范并集 − schema = 13 个码"钉成实测常量（让缺口可见）。
**ADR-0001 D25 把契约扩成联集 20 值之后**，这条判据的形态必须换，否则它会变成
"永远为真的旧快照"。现在是：

- `ErrorCode::SCHEMA_CONTRACT`（20）与 `schemas/mcp-tools.schema.json` 的 enum
  **集合完全相等**（双向包含 + 计数）——契约少一个 / 实现多一个都会红；
- 规范表格的 16 个必须**全部**落在契约里（缺口回来就红）；
- 契约 − 表格 == 那 4 个 schema 原有码（`ENTITY_NOT_FOUND` / `INVALID_PARAMETER_RANGE` /
  `PERMISSION_DENIED` / `ROUTING_CYCLE_DETECTED`），清单是实测常量。

---

## 3. 契约实测与发现

### 3.1 错误码缺口：**已由 ADR-0001 D25 关闭**（历史留痕）

第 1 轮实测：

| 来源 | 集合 | 数量 |
| :--- | :--- | ---: |
| `schemas/mcp-tools.schema.json` → `definitions.ToolResponse.properties.error.properties.code.enum` | 闭合枚举 | **7** |
| `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §7.2 表格逐工具列出的并集 | 并集 | **16** |
| 交集 | `PROJECT_LOCKED` / `IO_ERROR` / `PROPOSAL_NOT_FOUND` | 3 |

**当时 schema 装不下的 13 个**：`FILE_NOT_FOUND`、`DISK_FULL`、`NO_ACTIVE_PROJECT`、
`INVALID_FIELD_SELECTOR`、`STYLE_NOT_FOUND`、`CYCLE_DETECTED`、`CLIP_NOT_FOUND`、
`OUT_OF_RANGE`、`TRACK_NOT_FOUND`、`INDEX_OUT_OF_BOUNDS`、`RENDER_FAILED`、`BUSY`、`CONFLICT`。
后果是：**任何一个真实的领域失败都会产出一份 schema 判为非法的 `ToolResponse`。**

第 1 轮的处置（已作为方法论写进 D25）：

1. 两套集合都实现进 `ErrorCode`；
2. 缺口清单进机器可读样本；
3. 判据把缺口钉成实测常量；
4. 留一条"契约被修好时提醒升级口径"的判据。

**D25 的裁决**：取**联集 20 值**（原有 7 个一个不删 —— `PERMISSION_DENIED` 还是 scope
强制的必需码；规范并集一个不缺）。本轮已按 §M11 升级判据。

另有一处**未合并、留给人裁决**的命名重复：`CYCLE_DETECTED`（表格）与
`ROUTING_CYCLE_DETECTED`（schema 原有）看起来是同一件事的两种写法，两者都在 enum 里。

实测（`jsonschema` 4.24.0 / Draft 2020-12，本轮）：

```text
PASS   | 修复前会非法的领域失败: DISK_FULL
PASS   | 修复前会非法的领域失败: CONFLICT
REJECT | 实现级码不许伪装成响应  ({"status":"error","error":{"code":"NOT_IMPLEMENTED"}})
REJECT | 裸对象仍被拒            ({"anything":[1,2,3]})
REJECT | 未知工具名仍被拒        ({"name":"yeban_not_a_tool","arguments":{...}})
REJECT | 两个形状都像 (oneOf 必须恰好一个)
```

### 3.2 schema 根：从"空转"变**承重**（ADR-0001 D25）

第 1 轮实测：根只有 `$id` / `$schema` / `definitions` / `description` / `title` / `type`，
**没有** `properties` / `$ref` / `allOf`。Draft 2020-12 下"任意对象"都通过根校验：

```text
{"name": "完全不在枚举里的工具", "arguments": {"nonsense": 1}} -> PASS(空转)
{"error": {"code": "FILE_NOT_FOUND"}}                        -> PASS(空转)
{"anything": [1, 2, 3]}                                      -> PASS(空转)
```

**D25 把根改成 `oneOf($ref ToolCall, $ref ToolResponse)`**，契约因此真的在判定。
本轮随之做了两件必要修改：

1. **目录样本必须改成 `ToolResponse` 形状**：`mcp-tools.registry.json` 与
   `mcp-tools.error-codes.json` 原本是裸对象（`{"count":…}` / `{"all":[…]}`），
   在新根下**会被拒**。现在都包成 `{"status":"success","data":{…}}`
   （`oneOf` 下它们只匹配 `ToolResponse`）。这不是妥协，是契约变承重后的直接后果，
   而且用 4 条判据钉住（`both_catalogue_samples_are_tool_responses` 等）。
2. **判据从"提醒"变成"守卫"**：
   - `contract_root_is_one_of_tool_call_and_tool_response`（结构事实：
     根只有 `oneOf`，两个分支分别 `$ref` 到 `ToolCall` / `ToolResponse`）；
   - `contract_rejects_a_deliberately_invalid_sample`（**真跑**
     `validate_schemas.py --samples-dir`：12 份合法样本必须全过；
     混进 `{"anything":[1,2,3]}` 必须变红；未知工具名的 `ToolCall` 也必须变红）；
   - `exported_call_samples_are_contract_shaped_tool_calls` 里加了一条 `oneOf` 语义断言：
     每份样本必须**恰好**是两种形状之一（`name`/`arguments` 与 `status` 不能同时出现）。

> ⚠ 诚实边界：`contract_rejects_a_deliberately_invalid_sample` 需要 `python3 + jsonschema`。
> CI 的 **rust 矩阵腿**不装 jsonschema（只有 `checks` job 装），因此缺依赖时它会打印
> **响亮的 SKIP** 而不是伪造绿。真正的跨语言对账在 `checks` job 里跑 ——
> 那里既装了 jsonschema，也已被集成者接上 mcp 样本导出（§4.2）。

### 3.3 G04 是**文本**守卫，不是语义守卫

`scripts/guards/policy_check.py` 的 G04 只在非注释行里找 `0.0.0.0` **字面量**。实测：

| 注入 | G04 | 运行期判据 |
| :--- | :--- | :--- |
| `"0.0.0.0:0".parse()` | **FAIL**（`http.rs:468`，1 处） | — |
| `Ipv4Addr::UNSPECIFIED` | **ok**（不含字面量） | `bind_is_loopback_only_and_uses_a_dynamic_port` + 一串 http 判据 **FAILED** |

结论：**真正承重的是 `assert_loopback` 的 `is_loopback()` 判据 + 绑定后回读 `local_addr()`**，
G04 只是第一道便宜的网。这条写在这里，免得下一条线以为"G04 绿 = 绑定安全"。
（本 crate 的 `src/` 里刻意不出现那个字面量；测试里用 `Ipv4Addr::UNSPECIFIED`
构造"绝不允许的地址"，正是为了让这条判据不靠文本匹配。）

### 3.4 `tools/list` 的 `inputSchema` 由注册表**派生**，不存在第二份手写 schema

`ToolSpec::input_schema()` 直接从 `all_params()` 生成 `{type, properties, required,
additionalProperties: false}`。契约里的 `arguments.properties` 只声明了 `dryRun` /
`idempotencyKey`；逐工具参数在架构 §7.2 的表格里。两处合成一份，**唯一事实源 = 注册表**。

---

## 4. 判据实测（本地，命令 + 结果）

### 4.1 强制门禁

```bash
bash scripts/gates/run-gates.sh crate yeban-mcp      # exit 0, "门禁通过 (mode=crate)"
```

| 步骤 | 结果 |
| :--- | :--- |
| `cargo fmt --all --check` | ok |
| 机械红线守卫 13 条（G01..G13） | 全部 ok（含 G04 / G05） |
| 文档链接与 README 双语契约 | 通过（38 个文件，6 条**保护文件**警告不阻断，与本次改动无关） |
| 依赖许可清单漂移检查 | 与依赖图一致 |
| `clippy -p yeban-mcp --all-targets -- -D warnings` | 零告警 |
| `cargo test -p yeban-mcp` | **96 + 14 = 110 条判据全绿** |

另外单独跑过（feature 组合）：

```bash
bash scripts/dev/cargo-local.sh clippy -p yeban-mcp --all-targets --features mcp-http -- -D warnings  # exit 0
bash scripts/dev/cargo-local.sh test   -p yeban-mcp --features mcp-http                              # 96 + 14 全绿
bash scripts/dev/cargo-local.sh fmt --all --check                                                    # exit 0
```

### 4.2 跨语言契约对账（Python `jsonschema` 4.24.0 / Draft 2020-12）—— **现在真的在判定**

```bash
bash scripts/dev/cargo-local.sh run -p yeban-mcp --example export_mcp_samples -- --out target/schema-samples
python3 scripts/gates/validate_schemas.py --samples-dir target/schema-samples
# -> 12/12 样本 [ok] 通过 mcp-tools.schema.json；契约校验通过 (4 份 schema)

# 反证: 故意违法的样本必须变红
python3 -c "import json;json.dump({'anything':[1,2,3]},open('target/schema-samples/mcp-tools.bogus.json','w'))"
python3 scripts/gates/validate_schemas.py --samples-dir target/schema-samples   # exit 1
#   - mcp-tools.bogus.json: 违反 mcp-tools.schema.json @ <根>: ... is not valid under any of the given schemas
```

CI 侧：集成者已在 `ci.yml` 的 `checks` job 里接上
`cargo run -p yeban-mcp --locked --example export_mcp_samples -- --out target/schema-samples`
（在 model 那条之后、`validate_schemas.py --samples-dir` 之前）。
**这条对账现在是承重的**：`FILE_NOT_FOUND` / `DISK_FULL` 一类领域失败码不再被误判为非法。

### 4.3 二进制形态实跑（stdio + 环回 HTTP）

stdio（`printf` 五行喂进去）：

```text
{"jsonrpc":"2.0","id":1,"method":"tools/list"}                                -> id=1, result.tools 长度 10
{"jsonrpc":"2.0","id":2,"method":"tools/call", ... dryRun:true}               -> id=2, result.data.dryRun = true
{"jsonrpc":"2.0","id":3,"method":"tools/call", "yeban_nope"}                  -> id=3, error.code = -32004
{oops                                                                          -> id=null, error.code = -32700
{"jsonrpc":"2.0","method":"tools/list"}   (notification)                       -> 无输出
stderr: 处理 5 行, 写出 4 条响应 (跳过 0 个空行, 模式 production)
```

环回 HTTP（`--features mcp-http` + `--enable-mcp-http`，真实 `curl`，服务监听
`http://127.0.0.1:<动态端口>/mcp`）：

| 请求 | 状态码 |
| :--- | ---: |
| `GET /mcp` | **405**（带 `Allow: POST`） |
| `POST /mcp` 无 token（含 `{}` 这种非法体） | **401**（带 `WWW-Authenticate: Bearer realm="yeban-mcp"`） |
| `POST /mcp` 错 token | **401** |
| `POST /mcp` 对 token，`tools/list` | **200**，`id` 回显 `"live"`，`tools` 长度 10 |
| `POST /mcp` 对 token，`yeban_query_project` | **501**（`-32005 NOT_IMPLEMENTED`，如实报未实现） |
| `POST /nope` | **404** |
| `Content-Length: 2097152` | **413** |

令牌文件：`--print-token` 生成后 `-rw-------`（`600`）；手工 `chmod 644` 后再读 ⇒
`error: 令牌文件权限不安全: ... 是 644, 必须恰好是 600`，退出码 1。

启动开关：`--enable-mcp-http`（未编译该 feature）⇒ 退出码 **2**；
`--scopes ui:write` ⇒ 退出码 **2** 并列全合法值；`--help` ⇒ 0。

### 4.4 注入 → 变红 → 还原（**每条都真做过**）

方法：把源文件备份到 `/tmp/mcp-mut/`，注入后用 `bash scripts/dev/cargo-local.sh test -p yeban-mcp`
跑，记录红掉的判据名，再从备份还原并跑一遍全绿 + `fmt --check` 确认复原。

| # | 注入 | 变红的判据 | 还原 |
| :--- | :--- | :--- | :--- |
| **A** | 注册表里把 `yeban_reject_proposal` 改名成 `yeban_delete_proposal` | `every_registered_tool_name_is_in_the_contract_enum`、`every_contract_enum_name_is_registered_in_the_registry`、`exported_call_samples_are_contract_shaped_tool_calls`（+ 同批 3 条） | ✅ |
| **B** | `Scope::is_production_forbidden` 恒为 `false` | `security::ui_inject_is_forbidden_in_production`、`dispatch::ui_inject_is_hard_denied_in_production_through_the_pipeline`、`transport::http::ui_inject_is_still_hard_denied_in_production_over_http_context` | ✅ |
| **C** | `bind_loopback` 地址改成 `Ipv4Addr::UNSPECIFIED` | `transport::http::bind_is_loopback_only_and_uses_a_dynamic_port` + 9 条 http 判据 | ✅ |
| **C′** | 把字面量 `"0.0.0.0:0"` 写进 `http.rs` | 守卫 **G04 FAIL**（`http.rs:468`），`policy_check.py` 退出码 1 | ✅ |
| **D** | `authenticate` 里 `Credential::Missing => Ok(())` | `security::missing_and_wrong_tokens_are_rejected_on_every_channel`、`transport::http::missing_token_is_rejected_with_401`、`dispatch::authentication_precedes_parsing_on_the_raw_entry_point` | ✅ |
| **E** | 幂等重放分支改成永不命中（`None::<&CachedOutcome>`） | `dispatch::idempotency_replays_the_same_result_with_the_current_id`、`dispatch::distinct_keys_do_not_collide_and_cache_is_ordered` | ✅ |
| **F** | `ToolCall::is_dry_run` 改成永远读不到 `dryRun` | `dispatch::dry_run_short_circuits_without_recording_idempotency`、`dispatch::read_only_tool_dry_run_says_state_is_unchanged` | ✅ |
| **G** | 把契约联集里的 `DISK_FULL` 换成重复的 `BUSY`（模拟契约/实现漂移） | `implementation_error_codes_equal_the_contract_enum_exactly`（`assertion left == right failed`） | ✅ |
| **H** | 注册表样本退回裸对象（去掉 `ToolResponse` 外壳） | `samples::both_catalogue_samples_are_tool_responses`、`samples::registry_sample_lists_every_tool_and_the_two_common_params`、`samples::export_writes_twelve_byte_stable_samples`、`samples::export_to_target_feeds_the_python_reconciliation` | ✅ |

G/H 是**本轮新增**的两条：它们钉住的正是"契约变成承重之后"才会犯的错
（实现与契约枚举漂移、样本形状不再匹配根 `oneOf`）。

### 4.5 判据清单（按主题，共 110 条）

| 主题 | 条数 | 代表判据 |
| :--- | ---: | :--- |
| 工具注册表 / 参数 / 副作用 | 8 | `registry_has_exactly_ten_tools_in_spec_order`、`per_tool_error_codes_cover_every_documented_code_exactly` |
| 契约对账（读 `schemas/`） | 14 | `tool_name_sets_are_equal_and_in_the_same_order`、`implementation_error_codes_equal_the_contract_enum_exactly`、`contract_rejects_a_deliberately_invalid_sample` |
| token 生成 / 文件权限 | 11 | `generated_token_is_256_bit_hex`、`token_file_with_loose_permissions_is_refused`、`debug_format_never_leaks_the_token` |
| scope / `ui:inject` | 7 | `ui_inject_is_forbidden_in_production`、`app_admin_never_implies_ui_scopes` |
| JSON-RPC | 9 | `id_is_echoed_verbatim_for_all_three_kinds`、`error_object_carries_code_message_and_data` |
| 分发 / dryRun / 幂等 | 17 | `dry_run_short_circuits_without_recording_idempotency`、`idempotency_replays_the_same_result_with_the_current_id` |
| stdio 传输 | 4 | `skips_blank_lines_and_reports_parse_errors_on_their_own_line` |
| HTTP 传输 | 21 | `missing_token_is_rejected_with_401`、`end_to_end_over_a_real_loopback_socket`、`stream_path_checks_method_before_content_length` |
| 样本导出 | 9 | `export_writes_twelve_byte_stable_samples`、`error_codes_sample_matches_the_union_contract`、`both_catalogue_samples_are_tool_responses` |

### 4.6 CI 判决

- 第 1 轮（`7d0b0c4`）：run **37225147123** —— `checks` / `deny` / `lockfile` /
  `rust (workspace 全量)` **全部 ✓**。注意那一轮的 `checks` 跑的仍是**修复前**的 schema，
  所以当时 `--samples-dir` 对 mcp 样本还是空转的（集成者随后修好并接线）。
- 第 2 轮（本轮 rebase 后）：见 §7。

---

## 5. 未决项 / pending（**不许当成"已完成"读**）

| # | 项 | 性质 |
| :--- | :--- | :--- |
| P1 | 十个工具的**领域实现**尚未接线，一律返回 `-32005 NOT_IMPLEMENTED` | 已知缺口；分发/鉴权/dryRun/幂等/传输是真实现 |
| P2 | 冷启动 **≤ 20ms** 未测量（`docs/.../§7.1` 给形态 B 定的目标） | **未验证**；没有 `iai-callgrind` 打点，也没有 CI 侧启动耗时判据 |
| P3 | ~~CI 未接 mcp 样本导出~~ | **已由集成者关闭**（`ci.yml` 的 `checks` job 已接线） |
| P4 | ~~错误码契约冲突（13 个码没有家）~~ | **已由 ADR-0001 D25 关闭**（联集 20 值）；D25 标为 `Proposed`，**待人类追认** |
| P5 | ~~schema 根不引用 `definitions`~~ | **已由 ADR-0001 D25 关闭**（根改成 `oneOf`），判据已升级成承重形态 |
| P6 | `.yeban.lock` 排他锁（`ARCH-SEC-001`）完全没实现 | 属于 `yeban-model` / `yeban-app` 的所有者；本线只在错误码里留了 `PROJECT_LOCKED` |
| P7 | 形态 A 的"内嵌进 `yeban-app` 进程"只做出了**能力**（库 + 环回 HTTP 服务），**没有**接线到 `yeban-app` | 需要 app 侧工作线；本线不改其它 crate |
| P8 | `docs/ledger/dependency-licenses.md` 被本线重新生成 | 共享文件，多线并行时**冲突热点**；集成者合并时以再生成结果为准 |
| P9 | 非 Unix 平台只有"明确 `UnsupportedPlatform`"，没有 ACL 实现 | 规范要求"Windows ACL 仅限当前用户"；本线在非 Unix 上是**明确拒绝**而非静默放过 |
| P10 | D25 保留的 4 个 schema 原有码（`ROUTING_CYCLE_DETECTED` / `ENTITY_NOT_FOUND` / `INVALID_PARAMETER_RANGE` / `PERMISSION_DENIED`）**是否全都留着**由人类一并裁决 | ADR 原文已写明；本线按 D25 现状实现，并把这 4 个钉成实测常量 |
| P11 | `contract_rejects_a_deliberately_invalid_sample` 在缺 `jsonschema` 的环境里是**响亮的 SKIP** | CI 的 `rust` 腿没有 jsonschema；真跑在 `checks` 腿（已接线） |

---

## 6. 一句话总结

> 第 1 轮实测出的两处契约缺陷（错误码 enum 装不下规范并集、schema 根不引用 `definitions`）
> 已由集成者按 **ADR-0001 D25** 修好，本轮的判据也从"钉住缺口 / 提醒升级"升级成
> **"实现集合 == 契约集合"与"故意违法样本必须让契约变红"** —— 契约现在是承重的，
> 而"它是否承重"本身也有判据。仍然需要人类追认的只剩 D25 里的两处取舍：
> 联集 20 值这个扩法，以及 4 个 schema 原有码是否全部保留。

---

## 7. 本轮 CI 判决（读到什么写什么）

读取方式：`bash scripts/dev/ci-verdict.sh --watch line/mcp-core`

| 轮次 | run id | 头部 | 结论 |
| :--- | ---: | :--- | :--- |
| 第 1 轮（D25 修复前） | `37225147123` | `7d0b0c4` | **全绿**：`plan` / `checks` / `deny` / `lockfile` / `rust (workspace 全量)` 全部 ✓ |
| 第 2 轮（rebase + D25 升级） | `37225665357` | `b8f61f7` | **全绿**：同上五个 job 全部 ✓（`rust (matrix)` 0s 跳过，改动是 workspace 宽，走了 `rust-workspace` 腿 4m8s） |

**关键证据 —— 跨语言对账这次真的在 CI 里跑了**（`checks` job 的原始日志）：

```text
cargo run -p yeban-mcp --locked --example export_mcp_samples -- --out target/schema-samples
     Running `target/debug/examples/export_mcp_samples --out target/schema-samples`
target/schema-samples/mcp-tools.registry.json
target/schema-samples/mcp-tools.error-codes.json
target/schema-samples/mcp-tools.call.yeban_open_project.json
… (共 12 份)
[ok] mcp-tools.call.yeban_close_project.json: 通过 mcp-tools.schema.json
[ok] mcp-tools.call.yeban_edit_notes.json: 通过 mcp-tools.schema.json
… (12/12 全过)
```

> 本文件自身是**文档改动**，因此"记录判决"这个动作会让头部前进一格。
> 表中第 2 轮的结论对应的是**紧邻本节的代码头部** `b8f61f7`；
> 记录判决的那次文档提交只改了本文件，不影响任何判据（`checks` 的 fmt/守卫/契约对账
> 与 `rust` 腿的 clippy/test 都与文档内容无关）。
