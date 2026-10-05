# D56 实现简报（可直接执行）

来源：负责人指令「UI和MCP都要有采集调试信息及相关文件然后压缩包导出的功能，在遇到特殊问题时候，手工调用后采集信息回来复现和排查」。
规格全文：`docs/DEVELOPMENT_LEDGER.md` 第 151 轮。侦察结论：第 152、157、158、159、160 轮。本文件把它们压成一份**可执行清单**。

## 已完成（无需重做）

- **引擎侧共享采集器**：`crates/yeban-engine/src/diagnostics.rs`。
  - `BundleInputs` / `BundleEntry` / `BundleReport` / `DiagError` / `redact` / `export_diagnostics`。
  - `MANIFEST.txt` 由**同一份**写进 zip 的条目列表生成（sha256 判据比对真实产物）。
  - CI 验证：run `37332094472` @ `f25b488`，`rust (workspace 全量)` = **success `steps=10`**（`steps > 0`，非空心绿）。
  - 依赖 `zip 8.6` / `flate2 1.1.10` / `sha2 0.11` **全部为根清单预留** ⇒ 零新增依赖。

## 待做（全部无未知量）

### 1. `crates/yeban-mcp/src/tools.rs` —— 四处**耦合**修改

| 位置 | 现值 | 改为 |
| :--- | :--- | :--- |
| `TOOL_COUNT`（第 61 行）| 15 | 16 |
| `EXTENSION_TOOL_COUNT`（第 67 行）| 5 | 6 |
| `EXTENSION_NAMES`（第 80 行）| 5 项 | 追加 `yeban_export_diagnostics` |
| `TOOLS`（第 467 行）| 15 条 | 追加一条 `ToolSpec` |

第 1075 行断言 `TOOLS.len() == TOOL_COUNT` ⇒ 四处**必须一起改**。

`ToolSpec` 照 `yeban_query_engine_state`（第 713–728 行）的形状写：

```rust
ToolSpec {
    spec_id: "MCP-TOOL-EXT-DIAGNOSTICS",
    name: "yeban_export_diagnostics",
    summary: "把调试信息与相关文件采集并导出成 zip 诊断包 (人工触发, 供复现排查)",
    scope: Scope::AppAdmin,
    side_effect: SideEffect::ReadOnly,   // 只写独立文件, 不改工程状态
    params: &[param("outDir", "string", false, "输出目录; 缺省为进程当前目录")],
    errors: &[ErrorCode::InvalidArgument],
}
```

### 2. `crates/yeban-mcp/src/domain/diagnostics.rs`（新建）

两阶段契约（照 `domain/import_audio.rs`：`plan` 第 247 行、`apply` 第 471 行）：

- `pub fn plan(...)` —— 校验 `outDir` 可用；从领域取 `engine-state.json`、`config.json`、`logs/`、`crashes/`、`project/`
  （`project/` **默认空** = 不含工程，隐私默认）；组装成一个类型化的"计划"结构。
- `pub fn apply(domain: &mut super::Domain, planned: &DiagnosticsExport) -> Result<ToolResponse, Fault>`
  —— 调 `yeban_engine::diagnostics::export_diagnostics(...)`，把 `BundleReport` 映射为 `ToolResponse`
  （返回 `path` / `bytes` / `sha256` / `entries`）。

**决定（第 160 轮）**：zip **落盘在 `apply`** —— 管线把副作用放那里；它不改工程状态，所以 `side_effect` 仍是 `ReadOnly`。

### 3. `crates/yeban-mcp/src/domain/mod.rs` —— 两处 match

- `plan(...)`（第 2216 行调用处附近）加分支。
- `apply(...)`（第 2220 行附近）加分支。
- 并在文件顶部按既有写法 `mod diagnostics;`。

### 4. 判据（每条都要能失败）

| # | 判据 | 做法 |
| :--- | :--- | :--- |
| 1 | 必需条目在场 | 用 `zip` 读回包，断言 `MANIFEST.txt` / `env.txt` / `git.txt` / `engine-state.json` / `config.json` 存在 |
| 2 | 逐项 sha256 一致 | 读回每个条目，重算 sha256，与 `MANIFEST.txt` 内记录比对 |
| 3 | 脱敏 | 扫描包内**所有**文件，断言不出现真实 `$HOME` 字符串、不出现 `token`/`secret` 字面值 |
| 4 | 两入口共用同一实现 | UI 的 `Operation` 与 MCP 工具各调一次，除时间戳与路径外 MANIFEST 逐字段一致（**UI 半边待做**）|
| 5 | 牙测 | 从采集列表去掉一个条目 ⇒ 判据 1 变红 |

判据 1–3、5 可先落在 `crates/yeban-mcp/tests/extension_tools.rs`（该文件已走 `tools/call` 路径）。

### 5. 还要检查的两处

- `crates/yeban-mcp/tests/stdio_e2e.rs` 断言工具数 `>= 15` ⇒ 16 仍成立；但 D46 的**名字清单**应加上新工具名。
- `schemas/mcp-tools.schema.json` 看起来描述工具**形状**而非列出工具（第 158 轮查过，未穷尽）。若它确实约束名字，`checks` 腿会报。

## 验证顺序（照第 144 轮的教训）

1. `bash scripts/dev/cargo-local.sh test -p yeban-mcp`（若判为含重依赖则交给 CI）。
2. `bash scripts/dev/cargo-local.sh clippy -p yeban-mcp --all-targets -- -D warnings` —— **必须**跑；
   `light` 现在会自动对改动涉及的轻 crate 跑 clippy（`scripts/gates/clippy-changed.sh`），但仍要亲眼看它绿。
3. 加了依赖才需要 `python3 scripts/gates/license_inventory.py`（本次**不需要**，D56 无新依赖）。
4. `bash scripts/gates/run-gates.sh light`。
5. 派发 CI（`gh workflow run ci.yml --ref main`）并**专门读 crate 腿**：`rust (workspace 全量)` 必须 success 且 `steps > 0`。
   —— 第 155 轮的教训：`steps=0` 的 success 是空心绿，不算判决。

## 纪律提醒

- 提交信息用**英文**。
- 任何一处不确定就写进账本，不要"看起来有"。
- 未读回的判决**不是**通过。

---

## 第 161 轮补：接线的**精确锚点**（读完 `domain/mod.rs` 后确定）

`import_audio` 的完整接线模式如下。照抄它，`yeban_export_diagnostics` 就能一次接上。

### A. `domain/import_audio.rs` 的两阶段签名（照抄形状）

```rust
pub fn plan(project: &YebanProjectV1, assets: &dyn AssetStore, arguments: &Map<String, Value>) -> Result<AudioImport, Fault>
pub fn apply(domain: &mut super::Domain, import: &AudioImport) -> Result<ToolResponse, Fault>
```

⇒ 诊断侧对应：

```rust
pub struct DiagnosticsExport { pub out_dir: PathBuf, /* 组装好的引擎/配置/日志条目 */ }
pub fn plan(arguments: &Map<String, Value>) -> Result<DiagnosticsExport, Fault>   // 无需工程 ⇒ 不取 project
pub fn apply(domain: &mut super::Domain, planned: &DiagnosticsExport) -> Result<ToolResponse, Fault>
```

### B. `Plan` 枚举要加一个变体

现有形状（`domain/mod.rs`，`plan_import_audio` 的返回值）：

```rust
Plan::ImportAudio { import: Box<AudioImport> }
```

⇒ 加：

```rust
Plan::Diagnostics { export: Box<DiagnosticsExport> }
```

### C. 名字 → 包装函数的 match（`domain/mod.rs:1231`）

```rust
"yeban_import_audio" => plan_import_audio(domain, call),
```

⇒ 加一行 `"yeban_export_diagnostics" => plan_export_diagnostics(domain, call),`

### D. 包装函数（`domain/mod.rs:1621`）

```rust
fn plan_import_audio(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    let project = require_active(domain)?;
    let import = import_audio::plan(project, domain, &call.arguments)?;
    Ok(Plan::ImportAudio { import: Box::new(import) })
}
```

⇒ 诊断的包装函数**不需要** `require_active`（它不读工程）：

```rust
fn plan_export_diagnostics(_domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {
    let export = diagnostics::plan(&call.arguments)?;
    Ok(Plan::Diagnostics { export: Box::new(export) })
}
```

### E. `apply` 侧

`apply(domain, planned)` 里对 `Plan::ImportAudio { import }` 的分支是唯一改状态的地方。

⇒ 加：

```rust
Plan::Diagnostics { export } => diagnostics::apply(domain, export),
```

### F. 文件顶部

按既有写法加 `mod diagnostics;`（与 `mod import_audio;` 同级）。

### 因此最终改动清单（六处，全部有锚点）

| # | 文件 | 改动 |
| :--- | :--- | :--- |
| 1 | `domain/diagnostics.rs`（新建）| `DiagnosticsExport` + `plan` + `apply` |
| 2 | `domain/mod.rs` | `mod diagnostics;` |
| 3 | `domain/mod.rs` | `Plan::Diagnostics` 变体 |
| 4 | `domain/mod.rs:1231` 附近 | 名字 → 包装函数 |
| 5 | `domain/mod.rs:1621` 附近 | 包装函数本体（**不调** `require_active`）|
| 6 | `domain/mod.rs` 的 `apply` | `Plan::Diagnostics` 分支 |

外加 `tools.rs` 的四处耦合修改（见上）。**没有任何未知量。**

