# ADR-0007：红线 6 的发行特性措辞分写为两句（`asio` 与其余五个不同类）

- **状态**：**Accepted**（2026-10-15 由人类责任人裁决，选项"分两句写"）
- **相关**：`AGENTS.md` §2 红线 6、`crates/yeban-plugin-host` 的 `[MUST-GATE-013]`、
  `scripts/guards/policy_check.py` 的 `FORBIDDEN_DEFAULT_FEATURES`、台账 R513 起、R590–R591

## 背景

红线 6 原文用**同一句**禁止六个特性默认开启：

> 官方默认 release 构建中严禁默认开启 `mcp-http`、`ui-mcp`、`asio`、`experimental-vst3`、
> `experimental-als-export` 或 `experimental-logic-export`，上述特性仅限在特定开发或测试配置中按需开启

但这六个的**实况不同**（实测）：

| 类 | 特性 | 实况 |
| :--- | :--- | :--- |
| 甲 | `mcp-http`／`ui-mcp`／`experimental-als-export`／`experimental-logic-export` | **已定义**（各 `Cargo.toml` 有该 feature），且**确实默认关闭** |
| 乙 | **`asio`** | ⚠ **在任何 `Cargo.toml` 里都**尚未定义****；而 `crates/yeban-plugin-host` 的头注释（`[MUST-GATE-013]`）把它表述为**硬禁止**：「仓库内绝不允许出现 Steinberg ASIO SDK 头文件或专有代码」 |
| 丙 | `experimental-vst3` | ⚠ **尚未定义**；属**将来**的特性 |

⇒ 即：原句把「**永不定义**」（乙）与「**将来定义**」（丙）与「**已定义且默认关**」（甲）
写成了同一句。

## 裁决

**把红线 6 分写为两句**：

1. 前**五个**特性：**已定义／将来定义**，一旦定义即**严禁进默认**，仅在显式开发/测试配置中开启；
2. **`asio` 属另一类**：仓库内**绝不允许**出现 Steinberg ASIO SDK 头文件或专有代码 ⇒ **该特性不定义、不引入**。

## 依据

- **`[MUST-GATE-013]`**（`crates/yeban-plugin-host/src/lib.rs:6`）逐字把 ASIO 表述为**硬禁止**，
  而非"将来会有的 feature" —— 红线 6 原句与它**语义不一致**。
- 实测：`asio` 与 `experimental-vst3` **都不在任何 `Cargo.toml` 里**（`grep` 证据）。
- 机械防线**已就位**：`policy_check.py:56` 的 `FORBIDDEN_DEFAULT_FEATURES` **按名预留**两者 ⇒
  「一旦定义就禁入 `default`」这条已经能机械判；故本次只是**措辞澄清**，⛔ **不是新增约束**。

## 代价

- ⚠ **红线 6 从"一条"变成"一条 + 一段说明"** ⇒ 略长；⭐ 换来的是"永不定义"与"将来定义"**不再混同**。
- ⚠ **`asio` 的"不定义"目前仍无机械守卫**（它没有 `Cargo.toml` 条目可判）⇒
  ⭐ 现有防线只是"**若定义则禁进 default**"＋`G07`（仓库内无 ASIO SDK 文件）⇒
  ⚠ **"有人新加一个 `asio` feature 但不开默认"在 G07/G05 下都**不会红**** —— 本条如实登记此限度。
- ✅ 本次**未改任何守卫逻辑**，⛔ 未新增约束 ⇒ 对 CI 判决**无影响**。

## 推翻方式

人类改判时**更新本文件**（⛔ 不另开一份），并按新裁决改写 `AGENTS.md` §2 红线 6。
