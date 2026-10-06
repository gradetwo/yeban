# 夜半 (Yeban) 文档索引

本目录同时住着三类文档，读之前先分清它们的**效力等级**，否则会拿参考材料当硬规范用。

## 1. Normative —— 权威实现基准（不可协商）

| 文档 | 内容 | 版本 |
| :--- | :--- | :--- |
| [YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md](./YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md) | 线程拓扑、960 PPQ 数据模型、实时音频红线、PDC、`.yeban` 容器与锁、Ops Log / Commit DAG、双 MCP 拓扑 | `v1.0.0-rev2` |
| [YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md](./YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md) | 六阶段里程碑、9 个 Spike、`ROAD-*` 能力切片、`MUST-GATE-001..015`、`BASELINE-001..006`、`RSK-01..38` | `v1.0.0-rev2` |
| [YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md](./YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md) | 网格几何、Session/Arrangement 双视图、钢琴卷帘、快捷键与无障碍、无头视觉回归、`UI-*` 需求族 | `v1.0.0-rev1` |
| [YEBAN_INDUSTRY_BENCHMARK_AND_OPEN_SOURCE_STUDY.md](./YEBAN_INDUSTRY_BENCHMARK_AND_OPEN_SOURCE_STUDY.md) | 竞品解构、框架选型论证、Rust 音频生态盘点、许可矩阵 | `v1.0.0-rev2` |

`README.md`（仓库根）与 `AGENTS.md` 是**派生的执行契约**：README 给人类看，AGENTS.md 给 AI Agent 看。

## 2. 操作手册 —— 我们怎么干活

| 文档 | 回答什么问题 |
| :--- | :--- |
| [DEV_WORKFLOW.md](./DEV_WORKFLOW.md) | 一条工作线怎么开、怎么建 worktree、本机能跑什么不能跑什么、怎么才算"完成" |
| [CI_CD.md](./CI_CD.md) | GitHub Actions 有哪些 workflow、何时触发、需要哪些 secret、判决怎么读回来 |
| [../skills/yeban-dev-workflow/SKILL.md](./skills/yeban-dev-workflow/SKILL.md) | 十条用血换来的规则（测量优先、判据必须先能变红、绝不管道化门禁……） |

## 3. 判例与账本 —— 规范有缺口或冲突时我们怎么裁的

| 文档 | 内容 |
| :--- | :--- |
| [adr/](./adr) | Architecture Decision Record。规范之间冲突、或规范未定义时必须做的裁决，全部留痕 |
| [DEVELOPMENT_LEDGER.md](./DEVELOPMENT_LEDGER.md) | 测量账本：每个关键数字、它的测法、测量时刻；以及尚未验证的 `pending` 清单 |
| [ledger/legacy-reuse-audit.md](./ledger/legacy-reuse-audit.md) | 对 `groove` / `synth` 两个历史代码库的复用审计（结论 + 许可风险） |
| [ledger/gate-status.md](./ledger/gate-status.md) | **"现在到底什么算绿"的唯一去处**：`MUST-GATE-001..015` 与 `BASELINE-001..006` 逐条给出状态 + 可复跑证据 + 还差什么。由 `scripts/gates/check_gate_status.py` 机械守卫 |
| [ledger/phase-status.md](./ledger/phase-status.md) | **"每个阶段项做没做完"的唯一去处**：`ROAD-M-1-001..ROAD-M4-011`（47 项）逐条给出状态（已完成 / 部分 / PENDING）+ 可复跑证据 + 还缺哪一部分，并给逐阶段汇总计数。由 `scripts/gates/check_phase_status.py` 机械守卫 |
| [ledger/feature-alignment.md](./ledger/feature-alignment.md) | **"三方暴露对齐"的唯一去处**：73 行功能逐个给出「系统（实现/计划 + crate·文件·规范 ID）/ UI（有/无 + 元素 ID·`.slint`·`ui/*` 方法）/ MCP（有/无 + 工具名·参数名）/ 缺口（原因 / 计划 / 状态）」＋六类对齐汇总，以及建表时发现的**具体错位**（有能力零调用者、假阻塞、零消费者、回调未接线）。由 `scripts/gates/check_feature_alignment.py` 机械守卫 |
| [ledger/human-decisions.md](./ledger/human-decisions.md) | **需要人类裁决的唯一入口**：`HD-01..HD-50`，每项含选项 / 建议 / 不决定的后果（含"当前处置"，因此不构成单点阻塞）。由 `scripts/gates/check_decisions.py` 机械守卫 |
| [ledger/](./ledger) | 各工作线的实测台账（`*-notes.md`）：上游 API 核验、口径表、注入→变红记录、本机 vs CI 的分界、pending/needs |

## 效力顺序

规范之间冲突时，按此顺序裁决，并把裁决写进 `adr/`：

1. `AGENTS.md` 的绝对禁止项清单（红线）—— 任何情况下不得越过；
2. 四份 Normative 规范；
3. `adr/` 中已记录的判例；
4. 其余一切（含旧 Groove v3 时代的措辞）。
