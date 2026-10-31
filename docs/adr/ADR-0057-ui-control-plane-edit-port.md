# ADR-0057 — UI 控制面（`ui_mcp_serve` 装配）的编辑端口：先定会话生命周期，再接线

- **状态**: **Accepted（Wang 2026-10-31 ~22:30 授权「你自己择优决策」后裁决：选 A；本票仍不改代码，实现是下一阶段）**
- **决定者**: 待人类裁决
- **关联**: 审评 2026-10-31 P0-1（UI 控制面提交类控件全部静默无动作，零报错）；
  补丁 0001（`--ui-test-mode` + `YEBAN_ALLOW_UI_INJECT=1`，只分了执行面，**没有**修 P0-1）；
  `crates/yeban-app/src/ui_mcp_serve.rs:161`（`undo: None`）；
  `crates/yeban-app/src/live_surface.rs:1980`（写表面接线门）；
  `crates/yeban-app/src/live_surface.rs:834`（`save_now`）；
  `crates/yeban-app/src/live_surface.rs:963`（`open_project_now` 的 `undo.is_some()` 拒绝）；
  `crates/yeban-app/src/undo.rs:469`（`try_project`）；
  `crates/yeban-app/src/main.rs:968`（`open_undo_session`）；
  `crates/yeban-app/src/main.rs:1166`（`wire_transport`，仅 GUI 装配调用）；
  `crates/yeban-app/src/host.rs:1041-1048`（权威链注释）/ `:1552`（`wire_mixer_edit`）
- **日期**: 2026-10-31

## 背景（全部现读，⛔ 不采信报告的措辞）

审评的原话是「控制面提交类控件全灭」。仓库侧的真相分四层：

1. **写表面根本没接线**：`ui_mcp_serve.rs:161` 的 `LiveWiringOptions` 置
   `undo: None`（注释自称「本执行面只服务 `ui/*` 控制面」），而
   `live_surface.rs:1980` 把**整个**写表面（`wire_mixer_edit` /
   `wire_device_rack` / `wire_send_strip` / `wire_macro_panel` /
   `wire_bpm_commit` / `wire_track_strip` / `wire_clip_create` /
   `wire_roll_edit` / `wire_clip_drag`）都 gate 在
   `if let Some(undo)` 之后。`None` ⇒ 这些 Slint 回调**从未被挂上** ⇒
   注入的点击落在没有回调的元素上，静默无事。只读回调（视图切换等）
   是接了线的 —— 所以审评「能切 Tab」与「提交类全灭」**同时成立**，
   不是报告自相矛盾。
2. **走带是另一回事**：`wire_transport` 只在 `main.rs:1166`（GUI 装配）被调，
   服务端装配里**一次都没出现**；且服务端 `engine_quanta: 0`、
   `EngineHost::new()` 是空壳 —— 服务端即使接了走带回调也没有引擎可播。
   走带进服务端是**另一个 scope**（要引擎），不由本票带。
3. **补丁 0001 没有修它**：`--ui-test-mode` 只做了两件事——执行面分离
   （`into_control_plane` vs 生产面）与注入许可（env 缺省拒绝）。
   它**没有**给服务端装配一个撤销端口 ⇒ 测试模式下注入的编辑同样落到
   未接线的表面。以为「打了补丁就通了」是误读。
4. **P1-5（`dryRun` 的 `wouldChangeState: true`）不是独立 bug**：
   `dry_run.rs` 把它定义为「原则上会改状态」，指针派发的 `preview_effect`
   如实返回 `effect: null`（「指针事件的影响只有窗口自己知道」）。
   本票的接线落地后，强读法（`service.rs:2233` 的判据）自然处处成立；
   在此之前改 `dryRun` 语义会破坏已文档化的契约。⇒ 本票不碰 `dryRun`。

## 为什么不能直接「加一个 UndoPort-lite」（⛔ 本票的否决项）

只把 `undo` 从 `None` 改成 `Some(port)`，会同时踩中三处连贯性：

1. **会话从哪来**：GUI 的会话由 `main.rs:968` 的 `open_undo_session`
   （含 `history.dag` 恢复）建。服务端手里已经有 `cli::load_project` 的
   `loaded`（`ui_mcp_serve.rs` 约 `:141`）—— 正确做法是**同一条**
   `open_undo_session` 给服务端建会话，而不是现造一个轻量端口
   （轻量端口没有历史恢复，撤销/重做语义与 GUI 不一致）。
2. **保存会静默丢编辑（比现在更坏）**：`save_now`（`live_surface.rs:834`）
   在无权威时存 `save_project_file(&self.project)`—— 而 `self.project`
   是 `LiveAdminSurface` 的 Rust 侧缓存，只由 `apply_project` 换；
   `wire_*` 的提交只走 `commit_ops → refresh_undo_window`（`host.rs:1041-1048`），
   **只重投影窗口，不换 `self.project`**。⇒ 光加端口不改保存：
   注入的编辑在窗口上看得见，`ui/force_save` 一按就**按旧工程写盘**，
   编辑静默丢失 —— 比现在「什么都不发生」更坏。修法落点必须定：
   有端口时 `save_now` **优先 `port.try_project()`**（`undo.rs:469`），
   而不是事后在 `self.project` 上打补丁。
3. **`open_project_now` 会撒谎**：它今天 `undo.is_some()` 就拒绝，拒绝文案
   明写「请用**没有**撤销端口的装配（产品形态 `--enable-ui-mcp-http`
   就是这一种）」。给服务端加了端口却不改这条，测试装配就**换不了工程**，
   且文案变成假话。修法落点必须定：测试装配接受「不换工程」
   （启动时 `--open` 定死一个工程，文档写明），还是给测试装配一条
   「新会话」的换工程实现（`open_undo_session` 那一条，不是半吊子）。

好消息（接线后不需要 GUI 心跳）：`wire_mixer_edit` 的提交路径在内部
已经重投影（`commit_ops → refresh_undo_window`），视图刷新不依赖
GUI 进程的心跳；服务端自己的窗口就能看见提交结果。

## 决定（Proposed，请人类三选一或组合）

- **A（推荐）**：测试模式（`--ui-test-mode` + `YEBAN_ALLOW_UI_INJECT=1`）
  才建会话 —— 服务端用 `open_undo_session` 建与 GUI 同款会话；
  `save_now` 有端口时优先 `port.try_project()`；`open_project_now`
  在测试装配的行为与文案一并改（接受不换工程并写明，或实现新会话换工程）。
  生产面（`--enable-ui-mcp-http` 缺省）**保持只读**，且在方法规约里
  写明「无会话装配的提交类控件是惰性的」，不再让「静默无事」看起来像 bug。
- **B**：生产面也建会话（注入的编辑在生产面同样生效）。⛔ 本票不推荐：
  生产面无 env 门，等于把写入口默认暴露给任何持 Bearer 令牌者，
  安全模型要另议。
- **C**：维持现状（服务端永远只读），把「提交类控件在服务端装配上
  不可用」写进方法规约与审评回复，P0-1 以「设计如此」关闭。
  代价：UI 模式的端到端测试永远只能测视图切换，测不了编辑。

## 裁决（2026-10-31，Wang 授权择优决策后选 A）

选 A 的理由：B 把写入口默认暴露给持令牌者，安全模型要另议，不宜
在审评跟进支里顺手定；C 等于放弃 UI 模式的编辑测试面，P0-1 的实测
价值（注入编辑真的进工程）就永远立不住。A 的成本是测试面专属的
会话生命周期（三处连贯性），且与补丁 0001 的 env 门同向（测试才放行）。

**实现是下一阶段，不由本票带**：A 的三处连贯性（建会话、保存优先
`try_project()`、`open_project_now` 行为与文案）值得单独一支 + 单独 CI，
不塞进正在跑 CI 的修复支。补丁支与修复支合入后，再开新支实现 A；
实现时须带一条端到端判据：注入的编辑经 `ui/force_save` 落盘后重读一致
（今天论证过的「静默丢编辑」陷阱必须由判据钉死，不能只靠 code review）。

## 本票不做的事（⛔ 不是遗漏）

- **不碰走带**：服务端走带需要引擎，是另一个 scope；走带按钮在服务端
  装配上继续不可用，与本票无关。
- **不碰 `dryRun` 语义**：见背景 §4。
- **不改补丁 0001 的行为**：`--ui-test-mode` 的平面分离与 env 门保持原样；
  本票只决定「测试面里有没有会话」。
