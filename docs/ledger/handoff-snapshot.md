# 交接快照（**由 `scripts/dev/render-handoff.py` 生成，请勿手改**）

数字与清单都直接取自 `gate-status.md` / `phase-status.md`；本页只做汇总，**不引入新事实**。

## 门禁（`docs/ledger/gate-status.md`）

- 已接线 **19** · 部分 **0** · PENDING **2**（共 21 条）

## 阶段项（`docs/ledger/phase-status.md`）

- 已完成 **18** · 部分 **23** · PENDING **6**（共 47 项）

## 仍未闭环的门禁（逐条）

- `BASELINE-003` — **PENDING**。理由：**2026-10-07 口径裁决（负责人批准，`HD-59`；见 `docs/adr/ADR-0002-baseline-verdict-hardware.md` 的「裁决」一节）—— 正式口径 = 绘制回调耗时（不含呈现），门限 = 规格自己的 `[UI-NOTE-001]` 步骤 ④ 的 ≤ 2 ms；墙钟帧周期降级为环境读数、不作判决。** ⚠ 本行此前那句"**有意挂起**（**负责人裁
- `BASELINE-006` — **PENDING**。理由：需要"生成 16 小节段落"的完整 MCP 往返统计；十个工具已能真做事，但**载荷统计未接**，且 Token 口径需人类裁决用哪个 tokenizer

## 仍未闭环的阶段项（逐条）

- `ROAD-M-1-003` — **PENDING**
- `ROAD-M-1-004` — **部分**
- `ROAD-M-1-005` — **部分**
- `ROAD-M-1-006` — **PENDING**
- `ROAD-M0-001` — **部分**
- `ROAD-M0-002` — **部分**
- `ROAD-M0-003` — **PENDING**
- `ROAD-M0-004` — **部分**
- `ROAD-M0-005` — **部分**
- `ROAD-M0-006` — **PENDING**
- `ROAD-M0-009` — **部分**
- `ROAD-M1-005` — **PENDING**
- `ROAD-M2-001` — **部分**
- `ROAD-M2-002` — **部分**
- `ROAD-M2-004` — **部分**
- `ROAD-M2-005` — **部分**
- `ROAD-M2-006` — **部分**
- `ROAD-M2-008` — **部分**
- `ROAD-M3-001` — **部分**
- `ROAD-M3-002` — **部分**
- `ROAD-M3-003` — **PENDING**
- `ROAD-M3-004` — **部分**
- `ROAD-M3-005` — **部分**
- `ROAD-M3-006` — **部分**
- `ROAD-M3-007` — **部分**
- `ROAD-M4-006` — **部分**
- `ROAD-M4-007` — **部分**
- `ROAD-M4-010` — **部分**
- `ROAD-M4-011` — **部分**

## 待人类决策（未闭环的门禁里, 属于负责人裁决的那几条）

逐条见上面「仍未闭环的门禁」。每条都写明选项。请负责人选一条。

## 读这份快照的纪律

- **判决只能来自 CI**（`ci.yml` 自动档 / `gates-manual.yml` 手动档），本页的数字**不是**判决。
- 逐腿要看 `conclusion` **与 `steps` 数**：`steps=0` 的 success 是空心绿。
- 每条「已完成/已接线」的背后应当有一条**可复跑**的命令或 run id；没有就回 `gate-status.md` 要。
