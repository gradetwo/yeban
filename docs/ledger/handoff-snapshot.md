# 交接快照（**由 `scripts/dev/render-handoff.py` 生成，请勿手改**）

数字与清单都直接取自 `gate-status.md` / `phase-status.md`；本页只做汇总，**不引入新事实**。

## 门禁（`docs/ledger/gate-status.md`）

- 已接线 **17** · 部分 **1** · PENDING **3**（共 21 条）

## 阶段项（`docs/ledger/phase-status.md`）

- 已完成 **14** · 部分 **25** · PENDING **7**（共 46 项）

## 仍未闭环的门禁（逐条）

- `MUST-GATE-014` — **部分**（理由见 `gate-status.md` 对应行）
- `BASELINE-003` — **PENDING**（理由见 `gate-status.md` 对应行）
- `BASELINE-005` — **PENDING**（理由见 `gate-status.md` 对应行）
- `BASELINE-006` — **PENDING**（理由见 `gate-status.md` 对应行）

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
- `ROAD-M0-007` — **部分**
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
- `ROAD-M4-001` — **部分**
- `ROAD-M4-002` — **部分**
- `ROAD-M4-006` — **部分**
- `ROAD-M4-007` — **PENDING**
- `ROAD-M4-008` — **部分**
- `ROAD-M4-010` — **部分**

## 读这份快照的纪律

- **判决只能来自 CI**（`ci.yml` 自动档 / `gates-manual.yml` 手动档），本页的数字**不是**判决。
- 逐腿要看 `conclusion` **与 `steps` 数**：`steps=0` 的 success 是空心绿。
- 每条「已完成/已接线」的背后应当有一条**可复跑**的命令或 run id；没有就回 `gate-status.md` 要。
