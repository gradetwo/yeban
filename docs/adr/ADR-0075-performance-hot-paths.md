# ADR-0075 — 性能实测报告（a87a480）复核：热路径深拷贝与 UI 线程阻塞（Proposed）

- **状态**: **P2-A 已落地（本 ADR 登记）；P1-A/B/C、P2-B、P3 待立项** — 架构改动
  依 `AGENTS.md §5.7` 需人类裁决，⛔ 不由 Agent 私自拍板线程模型。
- **决定者**: 待人类责任人裁定（P2-A 除外，属确定性局部优化）
- **关联**: `~/work/qoder/性能实测-a87a480.md`（2026-11-16，基线 `main a87a480`）；
  `AGENTS.md §5/§5.7`；代码事实源 `crates/yeban-app/src/host.rs`、`src/undo.rs`、
  `src/main.rs`
- **日期**: 2026-11-16

## 背景

性能实测报告给出 5 类问题（P1-A/B/C、P2-A、P2-B、P3）。本轮**逐条回到当前代码**
核验（⛔ 不拿报告行号当事实），并只落地**语义等价、可判据钉住**的局部优化；
涉及线程化/增量的架构改动登记待裁决。

## 逐条核验与处置

| 项 | 报告结论 | 当前代码核验 | 处置 |
|---|---|---|---|
| **P1-A** 导出 WAV 跑在 UI 线程 | 8s≈1.0s / 60s≈9.7s，3 分钟曲卡 25–30s | `on_export_wav`（host.rs）→ `export_wav_with_sfz_file` 同步，无 spawn/进度 | **待立项**：搬 `thread::spawn` + 复用既有 `module_receipt` 报进度；需定义取消语义与线程安全（`YebanProjectV1` 快照可 `Send` 性需核） |
| **P1-B** 保存同步写资产 | 录过 3–5 分钟音频按保存卡数百 ms–数秒 | `live.try_project()`+`graph()`+`assets()` → `save_action::dispatch_save` 同步 | **待立项**：资产增量写/工作线程；需保证原子落盘不退化（`ARCH-SEC-004`） |
| **P1-C** 打开工程同步读解析 | 带资产工程白屏卡 | `open_project_document_file` 在回调里同步，随后换会话+全量投影 | **待立项**：载入线程 + "正在打开…"；需定义载入失败回滚 |
| **P2-A** 拖动每帧深拷贝整工程 | 60–120 次/秒全克隆，「越拖越卡」 | `preview_clip_drag` → `try_project` → `session.project().clone()`（`UndoSession::project` 返回 `&`，clone 是为拿 owned） | **✅ 已修**（见 D1） |
| **P2-B** 60 Hz 心跳常驻 + 每帧字符串 | 空闲 0.6–3.5% 单核 | `HEARTBEAT_PERIOD_MS=16`（main.rs:93）、`apply_meters` 每帧 set 7 属性、BPM 每帧 `parse::<f32>()` | **待立项**：停播降频 10–15 Hz + BPM 解析缓存；需确认电平刷新手感不回退 |
| **P3** 编辑无增量 | 每次编辑全量克隆+全量投影 | `try_project()` + `apply_view` 全量 | **待立项**：增量投影是结构性改造（与 P2-A 同源，先把高频只读路径去 clone） |

## 决定 D1 — P2-A 已落地（确定性局部优化）

给 `UndoPort` 加**借用式读**：

```rust
pub fn with_project<R>(&self, f: impl FnOnce(&YebanProjectV1) -> R) -> Option<R> {
    match &*self.session.borrow() {
        UndoBackend::Local(session) => Some(f(session.project())),      // ⛔ 不 clone
        #[cfg(feature = "in-process-mcp")]
        UndoBackend::Authority(authority) => authority.project().map(|p| f(&p)),
    }
}
```

`preview_clip_drag` 改用 `with_project`，只取那一条 placement 的 `start_tick`
（原语义保持：无工程直接返回、摆放不存在则草稿归零）。

- **生产路径（Local）零深拷贝**：拖动 60–120 次/秒不再克隆整工程。
- **控制面路径（Authority）** 仍 clone 一次（控制面 API 只给快照），但那是
  非 UI 高频路径，如实登记。
- ⚠ 闭包在 `RefCell::borrow()` 期间执行：只做纯读/纯算，结果拿出去再写 UI
  （避免重入借 panic）—— 已写进方法文档与判据。

判据：`tests/perf_hot_path_no_full_clone.rs` 钉住「拖动预览含 `with_project`、
不含 `try_project`」与「Local 分支是 `f(session.project())` 而非 `.clone()`」。

## 建议的人类裁决点

1. **P1-A 导出线程化**是否立项（优先级最高：唯一会让用户误判"程序崩了"的）。
   需定：进度回调频率、取消语义、导出期间是否禁编辑。
2. **P1-B/P1-C** 是否立项（保存/打开异步 + 载入提示）。
3. **P2-B 心跳降频**是否接受（停播 10–15 Hz），以及 BPM/电平缓存的刷新口径。
4. **P3 增量投影**是否立项（最大面，需与 P2-A 之后的读写接口一起设计）。

## 后果

- 正向：拖动预览这条 60–120 次/秒的热路径不再深拷贝整工程，工程做大后
  「拖动掉帧/松手延迟」的结构性来源被移除，且有回归判据。
- 负向 / 限制：P1-A/B/C/P2-B/P3 **本轮未改**（架构/线程模型需裁决）；
  报告的实测数字（8s≈1.0s、60s≈9.7s、空闲 0.6–3.5%）是**基线证据**，
  改动后的性能需在 CI 基准或真机上重测，⛔ 不以源码改动充当"已变快"。
