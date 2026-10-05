# `BASELINE-003` 10 万音符场景 —— 实现简报

目标：把**真实钢琴卷帘视图**接进已存在的计时循环，得到 p50/p99 并与门限 **p99 ≤ 8.3 ms** 对比。
规格见 `docs/DEVELOPMENT_LEDGER.md` 第 136 轮；计时路径与放置位置见第 145 / 148 / 149 轮。

## 已就位（无需重做）

- **计时循环**：`crates/yeban-ui-test-port/src/render.rs` 的测试 `frame_time_path_produces_a_real_distribution_and_non_trivial_frames`
  —— 一次 `LivePort::new(size, permission, registry, build)`，然后 600 次 `Instant::now()` + `window.request_redraw()` + `window.capture()`，
  打印 p50/p99/max 与 `golden_evidence(&image).summary()` 见证。夹具上实测 p50 0.912 ms / p99 1.111 ms。
- **真实视图与模型**在 `yeban-app`（`src/test_port_adapter.rs` 已经在驱动真实主窗口并写产物）。
- **约束**：`Tier1Window::install` 设的是进程/线程级 Slint platform ⇒ **每个测试线程只能装一次**。
- **判据口径**：夹具上的数字**不能**当门禁结论 —— 门禁要的是 10 万音符滚动。

## 需要写的两件

### 1. 10 万音符夹具（`yeban-model`）

已知锚点：

| 事实 | 位置 |
| :--- | :--- |
| 现成小工程 | `crates/yeban-model/src/samples.rs:136` `pub fn filled_project() -> YebanProjectV1` |
| 轨道构造助手 | 同文件 `midi_track(id)`（`filled_project` 用它建 lead 轨）|
| 音符结构 | `crates/yeban-model/src/music.rs:159` `pub struct MidiNote { id, start_tick, duration_ticks, pitch, velocity, ... }` |
| 夹具 id 生成 | 同文件 `fixture_id(n)`（`EntityId`）|

**待确认的两点**（一次读即可，不要猜）：

1. 如何**可变地**取到某个 clip 的音符集合（`clip_pool` 的 values → `content.notes()`；需要 `iter_mut`/`get_mut` 的可用路径）。
2. 生成**唯一** `EntityId` 的正式构造器（`fixture_id` 是测试用具；正式路径可能是 `EntityId::new()`）。

生成器建议签名与判据：

```rust
/// 造一个含 `note_count` 个音符的工程（确定性：同一 count 得到同一字节）。
pub fn project_with_notes(note_count: usize) -> YebanProjectV1;
```

- 判据 A：`project_with_notes(100_000)` 的音符总数**恰好** 100_000（不是"大约"）。
- 判据 B：**确定性** —— 调两次，两次的规范化 JSON 逐字节相同。
- 判据 C：性能 —— 生成本身不得成为瓶颈（若 > 1 s，注明实测值，别装作没有）。

### 2. `yeban-app` 的计时用例

- 用真实主窗口 + `project_with_notes(100_000)`，循环 600 帧，每帧把滚动推进 1/120 秒。
- 打印 p50/p99/max + 见证（帧数、音符数、每帧非黑像素数）。
- **不要**在 `ci.yml` 断言：托管 runner 的帧率读数不算数（`spikes/README.md` 第 36 行）。
- 判决走**手动档 `fps`**：派发一次、收集数字、按 `HD-45`（接受参考机自适应刷新率读数）判定。
- 牙测：关掉虚拟化视口再渲染同一视图，p99 必须超过 8.3 ms。

## 纪律提醒

- 本机跑 `test` **与 clippy**（第 144 轮的教训：漏 clippy 曾让 CI 两条腿变红）；`light` 现在会自动 clippy 轻量改动 crate。
- 加依赖要重生成许可清单（`python3 scripts/gates/license_inventory.py`）。
- 读测试输出**看尾部**、数 `FAILED`，不要 `head`（第 165 轮的教训）。
- 提交信息用英文；未读回的判决不是通过。
