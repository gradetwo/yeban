# ADR-0054 — 编曲标尺点击 / 拖动的**网格**：吸附到最近小节线（平局向更晚一侧），还是逐 tick 自由定位

- **状态**: **Proposed**（待人类批准；⛔ 集成者代签。本票**已按负责人任务落地**——
  `~/zimmer-ui-mode-report.md` 的 `U-07 R1` 要求"点线间 ⇒ 跳到最近小节"，
  实现落在 `crates/yeban-app/src/host.rs` 的 `ruler_seek_tick`，判据见下；本票把
  **为什么是这个网格、代价是什么、怎么推翻**写成可批可驳的记录）
- **决定者**: 负责人任务（`U-07 R1`）＋ 集成者按既有 `bridge::snap_tick` 口径实现；
  人类可改判为拍网格 / 不吸附 / 带修饰键的自由定位
- **关联**: `UI-NOTE-002`（坐标双向映射：像素 ↔ tick 的同一网格）、`UI-A11Y-003`
  （屏读动作与点击源同落点）、`MODEL-ISO-001`（走带定位 = 会话运行态，⛔ 不进工程）、
  `ADR-0005`（界面⛔ 持有第二份权威）、`ADR-0010`（片段循环坐标语义；本票不碰片段）
- **实现落点**: `crates/yeban-app/src/host.rs`（`ruler_seek_tick` / `ruler_span_ticks` /
  `ruler_step_tick` / `ruler_bar_tick` / `wire_ruler_seek`）、`crates/yeban-app/ui/workspace/arrangement_view.slint`
  （`moved` 与三个无障碍动作）、判据 `crates/yeban-app/tests/arrangement_ruler_seek.rs`

## 背景（先测后改，`AGENTS.md` §6）

`line/ruler-seek` 落地的第一版是**逐 tick** 定位：像素列 → tick，点在哪一列就停在哪一列。
参考规范 `DESKTOP_UI_UX` §4.2 提到标尺，但那份自 2026-10-06 起是 **Advisory（非绑定）**；
Normative 两份文档（架构 / 路线图）里**没有**"点击标尺是否吸附"的规定。⇒ 这是一处
**规范缺口**，按 `AGENTS.md` §5.7 登记为 ADR，而不是默默选一种。

`~/zimmer-ui-mode-report.md` 的 `U-07 R1` 把用户可见后果写清楚了：线间点击落在
"半个小节"这种用户不想停留的位置。本票按该任务落 **D1（吸附到最近小节线）**。

## 决定

**D1（已落地）**：标尺点击先换算成 tick，再吸附到**最近的小节线**；网格值 = 投影自己的
`ViewState::bar_length_ticks`（`bridge::bar_length_ticks(project.time_signature)`），
⛔ 不在宿主重算拍号。吸附实现复用 `bridge::snap_tick`，因此与铅笔 / 剪刀 / 移动**同一个**
吸附口径：

1. 向**最近**的网格线取整；
2. **平局向更晚的一侧**（`grid_ticks` 为偶数时存在正中间，`snap_tick` 的文档与判据都钉了这一点）；
3. `bar_length_ticks == 0`（坏投影值）⇒ **不吸附**，原样返回（`snap_tick` 的零网格语义）；
4. 吸附发生在**夹右缘之前**：`min(span_ticks)` 是最后一步，因此点在网格右缘之右仍落在
   `bar_positions.len() × bar_length_ticks`，⛔ 不把光标送去没画网格的地方。

同一口径也用在三条**无障碍**入口（`U-07 R2`）与**拖动**（`U-07 R3`）：

| 入口 | 落点 | 判据 |
| :--- | :--- | :--- |
| 指针按下（`ruler-seek`） | 最近小节线 | ⑦ |
| 按住拖动（`moved`，与按下同一条回调） | 每个移动事件按同一网格连续定位 | ⑧ |
| 屏读「上一 / 下一小节」（increment / decrement） | `current ± 1 × bar_length_ticks`，夹 `[0, span]` | ⑨ |
| 屏读「定位到第 N 小节」（SetValue） | `(N − 1) × bar_length_ticks`，N **1 起**，夹 `span` | ⑨ |

## 代价（必须有）

1. **点击标尺不再能停在小节内部**。用户想要"第 3 小节第 2 拍"就只能用播放 / 卷帘等其他入口
   —— 本仓今天**没有**小节内定位的界面控件。若人类需要，`D2` 是给标尺加修饰键（`Alt+点击`）
   做自由定位，或用拍网格（`D3`）。
2. **右缘与吸附的先后是决定**。先把 tick 吸到"最近线"再夹右缘，与"先夹后吸"在最后一格
   附近差一个小节。选前者是为了与判据 ④（点出网格右缘 ⇒ 夹在右缘）共存：真实窗口里
   "右缘之右"的点击列必须先被吸附拉回，再夹。
3. **平局向上**是 `snap_tick` 的既有裁决，不是本片新选的；若人类要"平局向下"，改的是
   `bridge::snap_tick` 一处，所有吸附入口一起变（⛔ 回归到某个调用方）。
4. **拖动会连续发 `SeekTicks`**：每个 `moved` 事件一条命令（判据 ⑧ 只钉"按下一 + 移动 + 松手
   ⇒ 日志加两条"，不钉中间事件数）。引擎侧的 seek 是廉价的位置写，⛔ 不重建快照；
   但**每像素一个事件**的量级意味着长距离快速拖动会产生一串定位 —— 与"逐 tick 定位"同量级。

## 备选（本票⛔ 推荐，供人类改判）

- **`D2` 修饰键自由定位**：`Alt+点击`（或 `Shift+点击`）跳过吸附。代价：需要一条新键位约定
  与判据（今天键盘策略表里标尺没有修饰位）；且需回答"拖动时修饰键状态如何取"。
- **`D3` 拍网格**：吸附到 `bar_length_ticks / numerator`。代价：拍号变化（复合拍 / 分母不是 4）
  下"一拍"不是整除法；网格值要有第二个来源 = 新的规范缺口。
- **`D4` 不吸附（回到第一版）**：实现最简单，但正是 `U-07 R1` 报告的问题；
  撤销本票只需删掉 `ruler_seek_tick` 里的 `snap_tick` 一行与对应判据。

## 判据与推翻方式

- 判据 ⑦⑧⑨ 在 `crates/yeban-app/tests/arrangement_ruler_seek.rs`；纯函数判据覆盖
  吸附 / 平局 / 步进夹紧 / 第 N 小节换算 / 右缘饱和。
- 推翻 `D1` ⇒ 删 `ruler_seek_tick` 的吸附一行、⑨ 的 set-value/increment 语义随之改为自由定位，
  并同步本 ADR 状态；⛔ 另开一份 ADR。
- 本票**不**改 Normative 文档、**不**改 `bridge::snap_tick` 的平局口径、**不**新增键盘快捷键。
