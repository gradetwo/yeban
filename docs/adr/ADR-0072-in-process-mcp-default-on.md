# ADR-0072 — `in-process-mcp` 改默认开（app 发行物链接 MCP 监听代码）

- **状态**: Accepted
- **决定者**: 负责人 Wang（2026-11-12 裁决；agent 先呈报坏处清单，负责人知情后点选整包方案）
- **关联**: `MUST-GATE-009`；`crates/yeban-app/Cargo.toml`（`[features] default`）；
  `scripts/guards/policy_check.py`（`FORBIDDEN_DEFAULT_FEATURES`，账本第 800 轮登记）；
  `.github/workflows/release.yml`、`.github/workflows/nightly.yml`；
  `crates/yeban-app/tests/cli_contract.rs` B7b／B7d／B7e／B7f。
- **日期**: 2026-11-12

## 背景

`--export-stems`、`--export-song-overview`、`--export-song-overview-html`、`--apply-ops`、
`--edit-samples`、`--slice-audio`、`--export-lyrics` 等批处理出口全挂在 `yeban-app` 的
`in-process-mcp` feature 下。默认构建里给这些开关只得到 `*NotCompiled` 用法错误，
批处理用法必须显式加 `--features in-process-mcp`。

负责人要求这批开关在默认构建里直接可用，并点选整包方案（乙）：整个 `in-process-mcp`
设默认开，含其转发的 `yeban-mcp/mcp-http` 监听代码；不拆 feature、不只开导出子集。

此前的口径相反：MUST-GATE-009 以「编译期关」为第一道开关 —— 默认发行物不链接
`yeban-mcp` 依赖图，`policy_check.py` 的 `FORBIDDEN_DEFAULT_FEATURES` 把
`in-process-mcp` 列名禁止进 `default`。本 ADR 推翻的只有这一条腿。

## 决定

### D1: `default = ["in-process-mcp"]`

`crates/yeban-app/Cargo.toml` 的 `[features]` 加 `default = ["in-process-mcp"]`。
`--no-default-features` 仍可整组关掉，关掉后的产物行为与本 ADR 之前一致。

### D2: 守卫名单移除一条，其余不变

`scripts/guards/policy_check.py` 的 `FORBIDDEN_DEFAULT_FEATURES` 移除
`in-process-mcp` 一条并留裁决注记。`mcp-http`、`ui-mcp`、`asio`、
`experimental-vst3`、`experimental-als-export`、`experimental-logic-export`、
`experimental-logic-import` 不变。

### D3: 运行时第二道开关不变

编译进来不等于在监听。不带 `--enable-mcp-http` 且未设 `YEBAN_MCP_HTTP=1` 时
不监听。监听仍只绑 `127.0.0.1`（AGENTS.md §2 红线 5 不变），会话令牌文件
`0600`、`ui:inject` 先于 token 的硬禁不变，进程内 112 条判据面不因此改动。

### D4: MUST-GATE-009 的 app 侧断言反转

`release.yml` 的依赖图断言改新口径：默认档 `yeban-mcp` 命中必须 ≥ 1；
负对照 `--no-default-features` 命中必须为 0 且依赖图指标必须变小；
`yeban-ui-mcp`／`yeban-ui-test-port` 在默认档仍必须为 0。产物级拒绝判据改查
仍默认关的三个开关（`--enable-ui-mcp-http`、`--export-als`、`--export-logic`）。
`yeban-mcp` 二进制自身的 `mcp-http` 仍默认关，`scripts/gates/check_release_defaults.sh`
不受本裁决影响。

### D5: 拒绝判据的编译条件不变

`cli_contract.rs` 的 B7b／B7d／B7e／B7f 本就是
`#[cfg(not(feature = "in-process-mcp"))]`：默认构建里自动不编译，继续守
`--no-default-features` 构建的点名拒绝（退出码 2 且点名 feature）。

## 代价（如实记录，负责人知情后裁决）

- 默认发行物链接环回 HTTP 监听代码，攻击面变大。
- 纵深防御由两道（编译期关＋运行时关）减为一道（运行时关）。
- app 默认构建多编 `yeban-mcp` 整个依赖图：发行体积与构建时长变大，
  增量以 release／nightly 构建实测为准，不预估。
- app 发行与 `yeban-mcp` 耦合变紧：mcp 侧的破坏性改动直接波及 app 默认构建。
- 对外解释成本变高：默认发行物内含 HTTP 控制面，安全审计须多解释一层运行时开关。

## 回滚

`Cargo.toml` 删 `default` 一行、守卫名单恢复 `in-process-mcp` 一条、
`release.yml` 断言改回旧形，即回到本 ADR 之前的状态。无数据与格式迁移。
