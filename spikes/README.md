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
