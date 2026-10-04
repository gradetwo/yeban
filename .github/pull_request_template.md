<!--
提交 PR 前请自查。本清单来自 AGENTS.md §3 (DoD) 与 docs/DEV_WORKFLOW.md。
勾不上的项不要勾 —— 勾不上的地方写进 needs 段落, 而不是假装完成。
-->

## 这次变更做了什么

<!-- 一句话说清。 -->

规范 ID：<!-- 例如 ARCH-RT-001, MODEL-AST-003, UI-GRID-002, ROAD-M1-001 -->

## 边界：这次**没有**证明什么

<!--
SKILL 规则: "It opens in the DAW" needs a machine with the DAW。
硬件回路时延、真实声卡行为、跨平台渲染等本机无法验证的，必须在此明说。
-->

## 判据（每一条都必须曾经变红过）

| 判据 | 故意改动后观察到的红 | 还原后 |
| :--- | :--- | :--- |
|  |  |  |

## DoD 自查

- [ ] `cargo fmt --all --check` 通过
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` 零告警（以 CI 判决为准）
- [ ] 单元 / 属性 / 集成测试全绿（以 CI 判决为准）
- [ ] `python3 scripts/guards/policy_check.py` 11 条守卫全过
- [ ] `cargo deny check` 通过（以 CI 判决为准）
- [ ] 未修改任何法务/治理文件（`LICENSE` / `LEGAL.md` / `SECURITY.md` / `TRADEMARK.md` / `GOVERNANCE.md` / `NOTICE.md`）
- [ ] 涉及 UI 的改动：语义 Element ID + 无头控件树 JSON 断言 + 已遮罩截图比对（`UI-TEST-*`）

## CI 判决

- [ ] 已用 `scripts/dev/ci-verdict.sh <branch>` **读过**判决，并把 run 链接贴在下面

<!-- run 链接 -->

## needs / pending

<!-- 没被证实的部分写这里, 并同步进 docs/DEVELOPMENT_LEDGER.md 的 pending 清单。 -->
