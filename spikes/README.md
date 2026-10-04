# Phase 0 — 九大技术 Spike (ROAD-M0-001 … ROAD-M0-009)

本目录是**一次性可行性验证**的隔离区。每个 spike 是独立 crate，各自拥有自己的目录，
因此九条工作线可以并行推进而永不争抢文件 (docs/DEV_WORKFLOW.md)。

| Spike | 规范 ID | 验证目标 | 通过判据 | 状态 |
| :--- | :--- | :--- | :--- | :--- |
| [`spike-01-cpal-latency`](./spike-01-cpal-latency) | `ROAD-M0-001` | cpal 低时延跨平台驱动验证 | 30 分钟连续播放零 underrun/xrun；硬件往返 ≤ 5.0ms (64 采样点)。 | 未实现 |
| [`spike-02-spsc-retire`](./spike-02-spsc-retire) | `ROAD-M0-002` | 无锁 SPSC 与快照退役回收队列 | 100,000 事件/秒，零死锁、零 malloc、单事件 < 0.05ms、零丢弃。 | 未实现 |
| [`spike-03-slint-fps`](./spike-03-slint-fps) | `ROAD-M0-003` | Slint 最小宿主与 120 FPS 渲染管线 | 稳定 120 FPS；常驻内存 < 25MB。 | 未实现 |
| [`spike-04-opslog-model`](./spike-04-opslog-model) | `ROAD-M0-004` | ULID + BTreeMap 可逆操作日志 | 状态 100% 还原；单步 < 0.2ms；序列化字节序完全一致。 | 未实现 |
| [`spike-05-ui-test-port`](./spike-05-ui-test-port) | `ROAD-M0-005` | 自研无头 UI 测试端口 (Tier-3 兜底) | 无显示器环境可启动；JSON-RPC 往返 ≤ 15ms；截图导出 ≤ 50ms。 | 未实现 |
| [`spike-06-roll-virtualization`](./spike-06-roll-virtualization) | `ROAD-M0-006` | 10 万音符虚拟化钢琴卷帘 | 稳定 120 FPS (帧耗时 ≤ 8.3ms)；无内存泄漏。 | 未实现 |
| [`spike-07-mcp-lock`](./spike-07-mcp-lock) | `ROAD-M0-007` | 内嵌 MCP 与 `.yeban.lock` 互斥 | 外部 Agent 可驱动运行中的 DAW 并触发 UI 刷新；并发打开得到 `PROJECT_LOCKED`。 | 未实现 |
| [`spike-08-software-renderer`](./spike-08-software-renderer) | `ROAD-M0-008` | Tier-1 软件光栅化无头截图 | 无头内存中产出 1920x1080 像素级一致 PNG，且非零尺寸、非全黑 (MUST-GATE-015)。 | 未实现 |
| [`spike-09-snapshot-exchange`](./spike-09-snapshot-exchange) | `ROAD-M0-009` | 高频快照原子交换与退役队列压测 | 音频线程零 dealloc、零爆音、队列不溢出、内存曲线平坦 (MUST-GATE-012)。 | 未实现 |

结论与实测数字一律登记到 `docs/DEVELOPMENT_LEDGER.md`（SKILL 规则 9：读数必须写下来）。

---

## 现状：哪些 Spike 已被正式 crate "汲取"（honest bookkeeping）

Spike 的价值在于**得出可复用的结论**，而不是留一个能跑的目录。结论一旦被汲取进 `crates/`，
spike 本身就可以按需退役（`docs/DEV_WORKFLOW.md`「明确废弃」）。

| Spike | 状态 | 说明 |
| :--- | :--- | :--- |
| `spike-04-opslog-model` | **结论已汲取** | `crates/yeban-model` 已落地 ULID `EntityId` + `BTreeMap` 权威 AST + 可逆 `Op` 日志 + proptest 状态守恒（76 个测试，CI run 37217037266 / 37218243856 系列绿）。spike 目录保留为空壳，直到有人把"10,000 次撤销/重做"的实测读数单独跑出来登记。 |
| 其余 8 个 | **未开始** | 目录为空壳骨架，`[dependencies]` 里的重依赖仍是 `TODO(spike)` 注释。这也意味着一件事：**在 `line/ui-shell` 之前，整个仓库从未编译过 Slint**（第一次真的编译时暴露了 Linux fontconfig 前置条件，见 `docs/CI_CD.md` §3.2）。 |

`spike-04` 的"已汲取"不等于"已验收"：MUST-GATE-010 要求的 **10,000 步**属性测试仍需在 CI 的上限档位
真实跑过并把读数登记进账本；本机默认只跑小步数。

## 纪律提醒

- 一个 spike 一个目录，**永不共享文件** —— 九条线可以真正并行。
- 重依赖（slint / cpal / symphonia / rayon）一旦在某条 spike 里启用，**全工作区的 CI 都要为它付编译成本**；
  启用前请先确认它带来的结论值得这份成本。
- spike 的结论必须能被**机械判据**证伪（哪个数字、什么条件下变红），否则它只是演示。
