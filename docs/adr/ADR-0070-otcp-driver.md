# ADR-0070：OTCP 音高校正驱动（yeban-otcp）

| 字段 | 值 |
| :--- | :--- |
| 状态 | **Accepted（Wang 2026-11-11 07:19 ROI 授权下派发实施；设计稿 `~/workspace/opentune/DESIGN-otcp-driver.md` 为权威依据）** |
| 日期 | 2026-11-11 |
| 分支 | `line/yeban-otcp-driver`（自 main `c83d1723`） |
| 设计稿 | `~/workspace/opentune/DESIGN-otcp-driver.md`（637 行）；评估 `~/workspace/research/yeban-otcp-scoping.md`（GO，有条件） |

## 背景

OpenTune API 分支暴露公开 OTCP 协议（JSON-RPC 2.0，TCP 环回）：导入音频 →
F0 分析 → 检测音符 → `autoTune` 按调性 snap → 导出校正音频。探查结论
（2026-11-06 复判，`RECOMMEND ADOPT`）：片段 72/76、全轨 107/110 在
±75 音分内，时序滞后 ≤0.62 ms，无新增伪影；单片段 8 秒、全轨 28 秒。
VST3 路走不通（宿主调研 2026-11-06：插件能加载但无参数通道，调性/模式
设不进去；每轨一个效果；延迟不补偿；无进程内隔离）。OTCP 绕过全部四项。

## 决策

### D1：新 crate `yeban-otcp` 拥有驱动；`yeban-mcp` 只做薄适配层；`yeban-plugin-host` 不动

- `yeban-otcp`：8 个模块（client / discovery / supervisor / pipeline /
  recovery / measure / config / error），依赖只有 `std` + `serde_json`
  （无 tokio——MCP 工具本就跑在工作线程上；SVS 客户端同款阻塞式）。
- `yeban-mcp`：`src/domain/otcp.rs` 适配层（实参关、片段解析、源制备、
  后台线程提交、收据轮询、提案交付、`fault_for` 唯一错误映射）。
- `yeban-model`：只加一个 `Op::SetClipAsset`（换音频片段的资产引用；
  旧资产索引保留，历史仍引用它）。
- `yeban-plugin-host` / `yeban-services` 不动；不接 `vocal_pitch_route.rs`
  的神经后端 trait，不进渲染链。

### D2：一次校正任务独占一个临时 OpenTune 服务端，永不共享

- 任务 temp 目录（`yeban-otcp/<job_id>/`）+ 控制目录（0700 校验）；
  启动 nonce 每任务新鲜（16–128 字符，永不记日志）；`session.hello`
  2000 ms 内；`session.info` 门：7 项必需能力 + 构建提交 ≥ 9ef45f9
  （下限提示，**自测记录才是真门**）。
- 关闭全路径一致：取消未终态任务 → 等终态 → 消模态 → `app.quit` →
  等 10 秒 → SIGTERM → 等 5 秒 → SIGKILL → 回收 → 删 temp 目录。
  `Drop` 守卫兜底杀子进程。
- 并发上限 2（可配，默认 2）；单任务单线程顺序跑。暖池不在 v1。

### D3：`RENDER_FAILED(render_chunk_failed)` 大声失败，什么都不写，永不静默回落干声

- 导出任务失败 → 走模态恢复环 → 报结构化错误（`block` +
  `note_gap` + 构建提交 + 自测记录 + 提示）→ 任务失败 → **不写**
  干声导出、不建资产、源片段不动、收据记失败与精确复现。
- 无自动 workaround：不重试同导出（服务端已重试过一次，确定性失败）、
  不切 clip、不垫静音、不改音符时值。

## 行为变更（写在前面）

- 校正后的音轨得到**新音频资产**（旧资产保留，历史仍引用它）；换资产
  只走提案路径（默认只出提案，`autoMerge: true` 当场合并）。
- 校正**只离线**，永不碰实时 VST3 路径；P0-1 神经腿的默认 OFF 不变，
  本 crate 不改任何默认值。
- 音符表是 OpenTune **检测**到的音符（过切分：22 音符对 16 音符乐句），
  不是乐谱音符；收据如实记检测数 + 调用方给出的乐谱数。

## 与设计稿的偏差（实施中定案，逐条留痕）

1. **自测夹具是合成信号，不是时光切片 A**。设计稿 §6.3 点名时光切片 A
   作自测夹具；驱动不能把一段真人演唱当运行时资产打包。实施改为内存
   合成夹具（两段 440 Hz 正弦各 1.0 秒，中间 0.5 秒静音，48 kHz 单声道
   PCM16）：测的是同一机制（渲染块落在无声音频里起步），不是同一输入。
   真实的切片 A 仍是 T2 真二进制验收测试的输入。
2. **没有第 4 个 status 工具**。设计稿 §6.3/§8.6 提到 status 工具，
   §7 只定义 3 个工具。status = 每个工具响应 + 任务收据里的 `server`
   块（构建提交 + 自测记录 + 可达性），不再另开工具。
3. **音符表工具读收据，不派生服务端**。检测音符只在导入 + F0 后的
   活会话里存在；v1 里 `yeban_correct_pitch_notes` 读最近一次校正任务
   的收据（片段寻址走片段→任务索引，内容寻址扫任务目录）。从未跑过
   任务的片段查不到音符表（`otcpNotesUnavailable`）。
4. **`dryRun` 与 `predictOnly` 分开**。公共 `dryRun` 标志被 `dispatch.rs`
   对**全部**工具拦截（只做规划预览，永不提交），所以真干跑不能走它。
   新设工具实参 `predictOnly`（布尔，默认 false）：真跑后台任务
   （导入 → F0 → snap → 重读音符表），只回预测音符表，不导出、不交付。
5. **总预算耗尽 → `otcpNotReady`**（细节写"预算耗尽"），不另加第 20 个
   reason；**`export_dry_stretched` → `otcpTimeGridUnsupported`**；
   服务端 `INVALID_ARGUMENT` / `INTERNAL` / `TIMEOUT` /
   `MODEL_UNAVAILABLE` / `CANCELLED` → `otcpServerGone`（服务端不可用）。
6. **自测门先跑、不拦任务**。首次提交任务时，若该构建无自测记录，
   驱动在同一服务端上先跑自测、记下结果，再继续任务；自测 fail
   不拦任务（无缝隙素材仍能校正），只在错误里标注记录。
   （父任务"先过自测门再接任务"与设计稿"自测永不拦任务启动"的调和。）
7. **MCP 进程死亡带走工作线程**：工作线程是进程内线程，MCP 进程死
   则线程死，子进程 OpenTune 可能成孤儿。收据里记子进程 pid，
   事后可手工回收；`Drop` 守卫只覆盖进程内放弃。
8. **音符表响应不带 `params`**。设计稿 §7 的音符形状含 `params`
  （retuneSpeed / vibratoDepth / vibratoRateHz / pitchDriftScale）；
   v1 的 `Note` 只存 7 个字段（下标 / 起止 / 检测音高 / 目标音高 /
   偏移 / 是否校正），响应面按 camelCase 展开这 7 个。参数往返存活
   是设计稿 §9 待办，待真机量测后定是否补进 `Note`。
9. **恢复循环覆盖 onboarding 遮罩，且每次写调用前都跑**。设计稿
   §4.2 的循环只写了对话框；真机实测（yanshi-5，9ef45f9）证明新鲜
   服务端启动即有首跑 onboarding（confirm 对话框 closeCommits: false
   + `onboarding.showing: true`），只清对话框不够——须另发
   `ui.dismiss {target: "onboarding"}`（需 `ui.onboardingClose` 能力，
   该构建有）。驱动在启动后先清一次（§1.4 验 `ui.getState` ），每次
   写调用（import / autoTune / editNotes / export）前经 `write_call`
   再跑循环，写调用本身报 `BUSY(modal_open)` 时恢复一次并重试一次。
   解散只改本次任务临时数据目录里的 `onboardingShown` 偏好，关机即删
   目录，用户侧无残留。另：`ui.dismiss` 结果的 `sideEffects.dialogClosed`
   真形状是对象 `{dialogs: [{kind, trackId}], revert: null}`，不是字符串
   数组（首版按假设写错，mock 已按实测改）。

## 不做（v1 之外）

P0-1 逐 chunk HiFi-GAN ONNX 后端、GUI/实时校正、暖池、多 clip、检测器
加固、F0 八度消歧、`regenerate` / `notesOnly` 模式、按乐谱逐音符校正
（v1.1 扩展）、立体声下混（立体声源响亮拒绝，不静默下混）。

## 许可边界

- 仓库里**没有任何 OpenTune 代码**，不打包 OpenTune 二进制与声码器权重。
  crate 只驱动用户自备的 API 分支 Standalone 子进程（走公开协议）。
- OpenTune 是 AGPL-3.0，声码器权重是 CC BY-NC-SA 4.0；Yeban 是
  GPL-3.0-only。本 crate 不链接 OpenTune 代码，只做协议驱动，许可边界干净。
- 用户须自备 API 分支构建：普通 v1.8.2 发布版没有监听器，`session.info`
  门会响亮拒绝（`otcpUnsupportedBuild`，报实际提交）。

## 验证

- `crates/yeban-otcp/tests/mock_server.rs`：9 组（mock 服务端）。
- `crates/yeban-otcp/tests/real_binary.rs`：13 组 `#[ignore]`（T1–T13，
  需真二进制；交接见 `crates/yeban-otcp/tests/REAL_BINARY_HANDOVER.md`）。
- `crates/yeban-mcp/tests/otcp_tools.rs`：10 组（MCP 适配层）。
- `cargo clippy -p yeban-otcp -p yeban-mcp --all-targets -- -D warnings` 零告警；
  `cargo fmt --check` 干净；`cargo deny check` 干净（零新增依赖）。
- 工具总数 38 → 41（`tools.rs` / schema / `param_bounds_guard` /
  `fault_vocabulary` / `op_reachability` 五处同步钉住）。

## 待决

- Xvfb vs 真显示器（设计稿 §9 Q1）：独立服务端在 Xvfb 下跑编辑器、
  `ui.getState` 可用——待 T1–T13 真机跑确认并记录。
- 首次校正音频交 Wang 耳验——耳朵是最终裁决，数字不是。
- `regenerate` / `notesOnly` 模式、颤音/漂移参数往返存活、冷启动耗时
  （首轮 F0 等待约 60 秒）——待真机量测后定。
