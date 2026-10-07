# ADR-0004 — 编排视图的轨道高度、横向时间轴缩放与轨道折叠（每轨持久数据 vs 视图缩放级；文件夹实体 vs 视图语义）

- **状态**: **Proposed（待人类裁决）**
- **决定者**: 集成者（Agent）在负责人授权范围内提出；「视图状态住哪一层」「文件夹是不是一等实体」这两族口径的最终裁决归负责人
- **关联**: 负责人请求 = `docs/DEVELOPMENT_LEDGER.md` 第 418 轮（提交 `4cfd74f`，原话见下）；
  裁决先例 = `ADR-0001` `D10`（规范冲突/缺口写进 `docs/adr/` 而不是由实现者自选）、`D28`（纯函数投影 + 唯一注入点 + 整数位置）、`D43`（1.0.0 前无兼容包袱，缺键响亮失败）；
  规范 = `MODEL-ISO-001`（三层状态物理隔离）、`MODEL-AST-002/003`、`ROAD-M1-002`（`folder_id` **仅**做界面折叠）、`UI-TEST-001`（语义 ID 寻址）、`UI-GRID-002`（「折叠」的对象是**栏**不是轨道）、`UI-NOTE-001`（视口裁剪）、`UI-MCP-003` / `MUST-GATE-015`（分平台 Golden 与视觉回归）、`ARCH-DET-001`（写入确定性）；
  触点 = `crates/yeban-app/src/{automation,bridge,host,elements,test_port_adapter}.rs`、`crates/yeban-app/ui/workspace/arrangement_view.slint`、`crates/yeban-app/ui/console/piano_roll.slint`、`crates/yeban-model/src/{project,local_config,session,error}.rs`、`crates/yeban-model/tests/model_isolation.rs`、`crates/yeban-app/tests/golden/linux/`（5 张 PNG + `MANIFEST.txt`）；
  台账 = `docs/ledger/feature-alignment.md:77`（`folder_id` 当前登记为「模型已实现 / UI 无 / MCP 无」）、`docs/ledger/app-automation-ui-notes.md:466`（>52 条泳道时带高 <1px 的既有风险）
- **日期**: 2026-10-07

> **测量口径**（`AGENTS.md` §6）：本文每个数字都带**对象 + 单位 + 位置**。带 `file:line` 的事实都在本机读过；
> 凡属「名字命中」而非「已接线能力」的，一律照实说成名字。

## 背景（规范从未定义这三件事；代码把它们钉成了常量与名字）

### (a) 负责人点名的是什么

`docs/DEVELOPMENT_LEDGER.md` 第 418 轮（提交 `4cfd74f`）逐字记录：

> **tracks that resize conveniently in both directions**（Logic's track-height zoom vertically and horizontal timeline zoom）
> 和 **the track fold feature**（Logic's Track Stack / Folder collapsing a group of tracks into one row）。

并写了两条纪律：① **「每一条都是真状态的真交互，各自值得一个能变红的切片判据」**；
② **折叠带一个设计问题**——「折叠后的堆叠在标尺里显示什么、编排视图如何投影一条被隐藏的轨道」。

⇒ 负责人的原话把**两个不同机制**混在「两向缩放」这一个词里：**每轨拖拽**（作品值）与**全局缩放级**（视图值）。
本 ADR 的第一个问题（Q1）就是把这两者拆开。第 418 轮还记着一个**时序**约束：当时主题切片正在写同一批 `.slint`，
所以**实现排在主题之后**（一文件一写者）。

### (b) 规范的空隙（因此按 `ADR-0001 D10` 写在这里，不改 Normative 正文）

- 四份 Normative 规范**没有任何一处**定义「轨道高度」「编排视图的横向缩放」或「轨道折叠」。
  `UI-GRID-002`（architecture 与 UI/UX 两份各有同文）的「折叠」指的是**侧栏 36px 图标轨 / AI 抽屉**，
  对象是**栏**，不是轨道。
- 唯一与折叠有关的口径是 `ROAD-M1-002` 与 `TrackV3` 的文档注释（`project.rs:1113-1116`、字段在 `:1143`）：
  「`folder_id` **仅**用于界面层树状折叠，严禁承载音频信号语义」。它已经被登记：
  `docs/ledger/feature-alignment.md:77` 写的是「**已实现**（模型）/ **无**（UI：没有路由/连线视图）/ **部分**（MCP）」。
- 横向缩放更微妙：快捷键 `Z`（选区撑满视口）与 `Shift+Z`（全曲总览）**早已写在规范 §7.1**，
  而提交 `7c995e7`（2026-10-07）刚刚把快捷表改成给未落地动作打 `(未实现)` 标记——`host.rs:548`
  `action_has_implementation` 明确把 `ZoomToSelection` / `ZoomToFit` 列进「没有落点」的集合，
  判据 B11b（`cli_contract.rs:1088`）把快捷表 **18 条条目**与它逐条对账（其中 **6 条**标 `(未实现)`）。
  ⇒ 在这一刻，「缩放」在代码里是一个**被点名的空承诺**；这既是横向缩放该做的理由，也是它的第一条判据。

### (c) 实测的当前状态（每条都读过代码）

1. **轨道高度：只有名字，没有能力。** 行距常量仍在**一处**（投影侧）：
   `automation.rs:98` `TRACK_LANE_TOP_PX: f32 = 42.0`（逻辑像素）、`:104` `TRACK_LANE_HEIGHT_PX: f32 = 56.0`（逻辑像素）；
   行几何由 `bridge::track_rows()`（`bridge.rs:672`）算前缀和，`TrackView`（`bridge.rs:690-737`）与
   `ClipView`（`bridge.rs:778` 起）各自带 `y` / `height`，`host::apply_view`（`host.rs:119` 起）注入
   `track-ys` / `track-heights` / `clip-ys` / `clip-heights` 四个 `[length]` 数组；
   `ui/workspace/arrangement_view.slint` 的包头 / 车道 / 剪辑三处直接读注入值
   （`root.track-ys[track_index]` / `root.track-ys[lane_index]` / `root.clip-ys[clip_index]`），**不做行算术**。
   ——**S0 已落地（`ee7fad5`，2026-10-07）**：这里原写「行 `y` 今天是 **`.slint` 自己算的**」，那与 `D28` 第 3 条
   「`.slint` 侧零算术」的偏差**已被 S0 纠正**；旧的 `42px + 56px * track_index`（包头 / 车道 / 剪辑三处）
   与注入的 `clip-lanes` 数组都已删除。但**高度仍是常量**：今天没有每轨 `height_px`、也没有高度缩放级 ——
   那才是本 ADR 要裁决的能力（见 Q1/Q3）。
2. **横向缩放：投影支持，产品改不动。** `ViewState::ticks_per_pixel`（`bridge.rs:876`，单位：tick / 逻辑像素）
   是投影字段；`from_project_with_zoom` / `_and_cursor`（`bridge.rs:937/957`）接受**任意** `ticks_per_pixel`；
   判据 `note_positions_are_integer_derived_from_ticks`（`bridge.rs:2940` 起）在 **7 档**
   `[1, 3, 7, 30, 32, 120, 960]` 上逐档对账整数除法；默认 `DEFAULT_TICKS_PER_PIXEL = 30`（`bridge.rs:72`）。
   **`from_project_with_zoom*` 的生产调用点 = 0 个**：全部生产入口都走 `from_project`（即默认 30）——
   `host.rs:729/807/841`、`scene.rs:198`、`live_surface.rs:460/1112`、`cli.rs:1450`、`reproject.rs:159`。
   这把 `ticks_per_pixel` 变成一个「**有能力、无消费者**」的字段（本仓刚在 `7c995e7` 里为同族问题改过快捷表）。
3. **折叠：模型孤岛。** `TrackV3::folder_id: Option<EntityId>`（`project.rs:1143`）被持久化、被 `validate()` 查存在性；
   而 `crates/yeban-app` 里 `folder` 出现 **0** 次（`grep -rn folder crates/yeban-app/ | wc -l` ⇒ `0`）。
   没有文件夹实体，`TrackKind`（`project.rs:389`）只有 `Midi / Audio / AuxReturn / Master`（**4** 个变体，无 `Folder`）。
   `validate()`（`project.rs:1673-1676`）只查 `self.tracks.contains_key(&folder_id)`，而 `tracks` **含主总线**
   ⇒ **自指**（`folder_id == track.id`）、**环**（a→b→a）、**指向主总线**今天**全部通过**。
   这是第一个「顺着指针走」的实现会踩的陷阱。
4. **编排视图没有横向滚动，也没有 `Flickable`。** `ui/` 下 `Flickable` 只出现在 `tokens.slint:32` 的注释里；
   `arrangement_view.slint`（445 行）全文 **0** 处 `scroll`/`viewport` 命中。
   钢琴卷帘走的是**另一条已被判据钉住的路径**：`callback scroll-requested(length)`（`piano_roll.slint:40`）
   + 宿主拥有偏移并重新注入，其注释（`piano_roll.slint:39`）明确写着「**Flickable 会与宿主拥有的偏移双重计算, 故不用**」。
   `app.slint:336/582` 与 `console_tabs.slint:34/159` 是这条路径的既有接线。

### (d) 代码强制的约束（选项只能在它们之内）

| # | 判据 / 事实（位置） | 它强制什么 |
| :-- | :--- | :--- |
| 1 | `automation.rs:1660` 的 `lane_element_ids_match_the_slint_template` 断言 `.slint` 里**没有任何**行算术（`"42px + 56px *"`、`"56px * track_index"` 等形态全部禁止）、几何读注入的 `track-ys` / `track-heights` / `clip-ys` / `clip-heights`，且 `host.rs` **真的注入**这四组（S0 已按本表建议**改写**，不是绕过） | 任何变行高**按构造**让它变红。改写后的判别力**更强**：旧断言只钉住一个字符串，新断言钉住「唯一事实源」（`.slint` 零算术 + 注入面存在） |
| 2 | `automation.rs:750-753`：`band_height = (row.stride − 2×inset)/lane_count`、`band_y = row.y + inset + band_height × band_index`（S0 已把 `track_offset = TRACK_LANE_HEIGHT_PX × track_index` 换成投影的行几何） | 变行高时带几何必须随 `row.stride` / `row.y` 走；否则 9 个平行数组与真实行错位 |
| 3 | `automation.rs` 的 9 个平行数组由 `host.rs:190-198` 注入（8 个 `set_automation_lane_*` + `set_automation_path_commands`） | 这 9 个数组与行几何必须来自**同一份**事实 |
| 4 | `elements.rs:1247` `SLINT_MANIFEST: [&str; 13]`，与 `ui/` 实际文件集合逐项比对（`elements.rs:1298-1300`） | 新增任何 `.slint` 文件即红 |
| 5 | `elements.rs:1364` `slint_accessible_ids_and_registry_cover_each_other` | `.slint` 的 `accessible-id` 与 `ElementRegistry` **双向覆盖**；隐藏行若只在一侧消失即红 |
| 6 | `elements.rs:1081` `per_track_semantic_families_are_complete` | 每个 `track-{i}` 必须有 fader / header / mute-button / solo-button / channel-strip / meter / color-swatch **7 族** |
| 7 | `elements.rs:1418` `registry_follows_the_projected_project` | `track-{i}` 的 `i` 必须与投影 `ViewState.tracks` 的**下标**对应（演示工程 6 条轨道 ⇒ 有 `track-5-header`；filled 项目 3 条 ⇒ **没有** `track-3-header`）。「隐藏即从数组里删掉」会被它按构造打红 |
| 8 | `elements.rs:405/478/658` 三个族（session / arrangement / mixer）都按 `view.tracks.iter().enumerate()` 造 ID | 任何重编号都同时动三族语义 ID |
| 9 | `TrackView.index` 的文档（`bridge.rs:691`）：视图内序号（0 起，已排除主总线）**进 `track-{i}-*` 的语义 ID** | 下标就是可寻址身份，不是排版下标 |
| 10 | `bridge.rs:3331` 钉死 `PITCH_LANE_HEIGHT_PX == 14.0`（钢琴卷帘行高，逻辑像素） | 另一条纵向几何已有自己的常量与判据；**不要**把它和编排行高混用 |
| 11 | `model_isolation.rs:105` `FROZEN_PROJECT_KEY_PATH_SHA256` + `:109` `FROZEN_PROJECT_KEY_PATH_COUNT = 223`（条递归键路径）+ `:447` 的两份逐字节样本 + `:405` 顶层键表 | 工程里**加任何键**同时移动 4 处 |
| 12 | `model_isolation.rs:539` `session_state_has_no_serde_surface` | `session.rs` 真代码里不许出现 `serde`/`Serialize`/`Deserialize` ⇒ **第 2 层（会话态）按构造不能持久化** |
| 13 | `local_config.rs:498/529/581` 三处 `#[serde(deny_unknown_fields)]`、`:637` 版本闸门（只接受 `version <= LOCAL_CONFIG_VERSION`，当前 `= 1`）、`model_isolation.rs:112` `FROZEN_LOCAL_CONFIG_KEYS: [&str; 4]` | 第 3 层加字段要同时改契约、冻结键集与版本口径 |
| 14 | 5 张 Linux Tier-1 Golden（`tests/golden/linux/*.png`，1920×1080）+ `MANIFEST.txt` 的 run id / 时间戳 / 逐文件 sha256 | 任何**默认可视几何**变化都要按 `ADR-0003`「批准后的再生成程序」在 CI 上重做 5 张 |
| 15 | `test_port_adapter.rs:174` `assert_matches_golden`：平台目录不存在时打印「视觉回归**未被判定**（不等于通过）」并 `return`；本仓只有 `tests/golden/linux/` | **本机 macOS 判不了 golden**；本机绿不是证据 |
| 16 | `host.rs:548` `action_has_implementation` 把 `ZoomToSelection`/`ZoomToFit` 列为未落地；`cli_contract.rs:1088`（判据 B11b）把快捷表 18 条条目的 `implemented` 与它逐行对账 | 横向缩放一落地，`host.rs`、`cli.rs` 的表、B11b 必须**同一提交**一起动 |
| 17 | `docs/ledger/feature-alignment.md:77` | UI 侧做折叠会移动那一行的三方状态（那是集成者的登记动作，本 ADR 不改它） |

---

## 裁决一览

| # | 问题 | 建议 | 归负责人？ |
| :-- | :--- | :--- | :--- |
| Q1 | 轨道高度：每轨持久数据 / 单一全局缩放 / 两者 | **两者分层**：`TrackV3::height_px`（作品值）+ 全局高度缩放级（视图值，乘子） | **是**（`height_px` 是否进工程） |
| Q2 | 单位与边界；clamp 住哪 | **整数逻辑像素（`u32`）**；有意义的上/下界夹紧放**投影**（纯函数），模型只拒绝退化值 `0` | 否（`D28` 强制） |
| Q3 | 自动化带是否跟随每轨高度 | **必须跟随**，且与剪辑车道读**同一份**行几何 | 否（9 数组强制） |
| Q4 | 非工程状态住哪一层 | **第 2 层会话态（host 拥有）作为第一片**；第 1 层列为负责人裁决 | **是**（第 1 层那半） |
| Q5 | 横向缩放数字归谁 | **宿主/会话暂态作为第一片**（给 `from_project_with_zoom` 第一个生产调用点） | **是**（是否像 Logic 一样存进工程） |
| Q6 | 滚动是否与缩放同片 | **同片**；复用钢琴卷帘的 `scroll-requested` + 宿主偏移形状，**不引入 `Flickable`** | 否（可用性强制） |
| Q7 | 离散 vs 小数缩放级 | **离散整数阶梯**；投影仍接受任意 `tpp >= 1` | 否（`ZeroTicksPerPixel` 强制下界） |
| Q8 | 文件夹：组实体 vs 视图语义 | **视图语义（仅 `folder_id`）作为第一片**；`TrackKind::Folder` 登记为负责人裁决 | **是** |
| Q9 | 隐藏轨道如何投影 | **保留完整数组 + `visible`/`folded` + 投影供行几何**；**否决**「从数组移除」 | 否（`UI-TEST-001` 强制） |
| Q10 | 折叠行在标尺里显示什么 | **子轨并集跨度**（投影派生）；「画子剪辑」出局 | **是**（UX 口径） |
| Q11 | 折叠后混音台行为 | **保留折叠通道条**（单一 `track-names` 数组不动） | **是**（UX 口径） |
| Q12 | `collapsed` 位住哪 | **第 2 层会话态作为第一片**；与 Q5 **同一裁决**，不许一个进工程、一个留会话 | **是** |
| Q13 | 折叠的校验漏洞 | `validate()` 用**访问集**拒绝自指 / 环 / 指向主总线；不发明魔数深度上限 | 否（终止性强制） |

---

## 决定（Proposed）

### Q1 — 轨道高度是**每轨持久工程数据**、**单一全局高度缩放**，还是**两者**？

**问题**：负责人说「resize conveniently in both directions … track-height zoom」。这个「两向」是
① 逐轨拖拽该轨的上下边界（**作品值**），还是 ② 一个作用到所有轨的全局高度缩放级（**视图值**）？还是两者都要？

**选项与代价**：

- **A. 每轨持久 `TrackV3::height_px`（工程数据）** —— 触点：`project.rs` 新字段 + `validate()` +
  `schemas/project.schema.json`；`model_isolation.rs` 的 **223 条递归键路径**（`:109`）、其摘要（`:105`）、
  顶层键表（`:405`）与两份逐字节样本（`:447`）；`bridge.rs` 的 `TrackView` + `track_view()`；`host.rs` 注入；
  `arrangement_view.slint` 行几何。**schema：要**。**golden：落地 UI 后要**（5 张）。
  **代价**：每轨高度成为**作品的一部分**——同一份作品在别人机器上也是同一行高；但「我只是想把它看高一点」
  也会被写进文件、进 Commit 字节，`ARCH-DET-001`（写入确定性）的语义边界需要重新论证。
- **B. 单一全局高度缩放倍率（一条标量，作用于所有行）** —— 触点：投影 + 注入 + `.slint`；一条判据（整数、单调、夹紧）。
  **schema：若放工程则要；放会话则不要（但第 2 层不能持久化 ⇒ 重启即失）**。**golden：默认倍率 ≠100% 时要**。
  **代价**：**无法表达「这条鼓轨比那条总线高」**——而负责人原话里「两向」正是 Logic 的「逐轨拖拽 + 全局缩放级」。
  单一倍率把两个交互压成一个，等于砍掉一半。
- **C. 两者分层**：`TrackV3::height_px` 是**作品值**（author），全局高度缩放级是**视图乘子**（host/session），
  有效行高 = 投影里的一次整数运算（`clamp(height_px × 百分比 / 100)`，`checked_mul` 后整除，`D28`）。
  —— 触点与 schema 面同 A；**新增一条判据**：有效行高是整数、单调、且夹紧在命名边界内。
  **代价**：两个旋钮作用于同一个量 ⇒ 必须写下**优先序**（缩放是视图乘子，`height_px` 是作品值；公式只有一处，
  即投影；界面没有第二处），否则会生出两份事实源。

**代码强制**：`D28`（位置整数、投影是唯一注入点、`.slint` 零算术）；`TRACK_LANE_HEIGHT_PX` 今天是**常量**而非数据；
`bridge.rs:3331` 说明另一条纵向几何（钢琴卷帘 14px）有自己独立的常量与判据，**不能**与编排行高共用一个数。

**建议**：**C（分层，职责互斥）**。理由：① 负责人原话同时点名每轨拖拽与缩放级，二者语义不同（作品 vs 视图）；
② 代码已经把「投影接受外部参数」的形状给出来（`from_project_with_zoom` 是**入参**而不是字段），
全局缩放天然是投影的**输入**；③ A 单独不足（没有缩放级就做不了「一键看全局」），B 单独不足（丢失每轨差异）。
**需负责人确认**：`height_px` 是否进工程——那决定「行高是不是作品的一部分」。

### Q2 — 高度的单位是什么？**边界夹紧住在哪一层**，才能成为一条判据？

**问题**：高度用 `f32` 还是整数？上/下界的夹紧放模型 `validate()`、投影、还是 `.slint`？

**选项与代价**：

- **A. `f32` 逻辑像素 + 投影夹紧** —— 违反 `D28` 第 2 条（整数位置）；判据无法逐位复现。**代价**：`ticks_per_pixel = 30`
  那类「非 2 的幂 ⇒ 浮点坏掉」的教训会以另一种形式回来。
- **B. 整数逻辑像素（`u32`），夹紧在投影（纯函数）** —— 触点：`bridge.rs` 一个 `clamp` 纯函数 + 一条判据。
  **代价**：几乎没有；收益是本机可判——`D28` 第 1 条实测过：投影是零 Slint 依赖的纯函数，
  31 条判据能在本机用 `rustc --edition 2024 --test` 真跑（Slint 本机禁编译）。
- **C. 夹紧在模型 `validate()`** —— **代价**：把 UI 边界升格为**文档合法性**；而且 `from_project` **不调用** `validate()`
  （`bridge.rs` 的多条判据直接在未 `validate()` 的夹具上投影）⇒ 模型夹紧**挡不住投影路径**，
  结果会出现「两处边界」。**否决**（但保留下面那一条退化值检查）。
- **D. 夹紧在 `.slint`** —— 被 `D28` 第 3 条禁止（`.slint` 零算术），且本机判不了。**否决**。

**代码强制**：`D28` 第 2/3 条；`.slint` 的 `height:` 是 `length`，而投影今天输出的是**由整数派生**的位置
（`bridge.rs` 的 `NoteView.x/width` 就是整数除法的产物）。

**建议**：**B**。边界写成命名常量（`MIN_TRACK_HEIGHT_PX` / `MAX_TRACK_HEIGHT_PX` / `DEFAULT_TRACK_HEIGHT_PX = 56`
逻辑像素），由**投影**做有意义的夹紧，并配一条**本机可跑**的判据（越界输入 ⇒ 夹紧且夹紧后单调）。
模型侧**只拒绝退化值**：`height_px == 0` 是损坏而不是布局选择（`D43`「让损坏的文件响亮失败」）。
这样「边界」是一条能在本机变红的判据，而文档层仍然拒绝明显损坏的值。

### Q3 — 自动化曲线带是否跟随**每轨高度**？

**问题**：`automation.rs` 的带几何今天由 `TRACK_LANE_HEIGHT_PX` 派生。变行高后它跟不跟？

**选项与代价**：

- **A. 不跟随（band 仍按 56 逻辑像素常量）** —— **代价**：`track_offset = 56 × track_index` 与真实的（前缀和）行 `y`
  **必然**分叉；`host.rs:190-198` 注入的 **9 个平行数组**（`automation-lane-band-ys` / `-band-heights` 等）
  与真实行错位；`lane_element_ids_match_the_slint_template`（`automation.rs:1660`）与
  `automation.rs:1670` 的 ID 模板对账会红。**这不是「少做一个特性」，而是让界面自相矛盾。**
- **B. 跟随：投影先算每行 `y`（前缀和）与行高，带几何由**同一份**行几何派生** ——
  触点：`automation.rs` 的 `project_lanes*`（`automation.rs:687/705`；S0 新增的带行几何入口
  `project_lanes_with_rows` 在 `:723`）多一个「行几何」输入；
  `host.rs` 的注入面不变（9 个数组仍是投影产物）。**代价**：`automation.rs` 的函数签名与
  它的调用点（`automation.rs:955/1047/…` 大量传 `DEFAULT_TICKS_PER_PIXEL`）要改；**新增一条判据**：
  每个带的 `y` 落在其所在行的 `[row_y, row_y + row_height]` 之内。

**代码强制**：`automation.rs:750-753` 的公式；`automation.rs:1690-1703` 的文本层断言（S0 已改写为禁止行算术）；
`app-automation-ui-notes.md:466` 记录的既有风险（一条轨道 >52 条泳道时带高 <1px ⇒ 元素可能被裁剪语义过滤掉，
「登记了但查不到」）。

**建议**：**B**，而且带与剪辑车道**必须**从**同一个** `Rows` 值派生，**不是各算一次**——
否则「同一个 42/56 写两份」的旧病会在新形状里复发（`automation.rs:67-70` 的注释自己承认那是「已知的耦合」）。
这一条属于**共享前置切片 S0**（见切片顺序）。

### Q4 — 如果状态不是每工程的，它住在哪一层？

**问题**：高度缩放级、横向缩放、折叠位放哪？

**选项与代价**：

- **A. 第 2 层会话态（`session.rs`）** —— **代价**：**按构造不能持久化**（`model_isolation.rs:539` 机械断言
  `session.rs` 无 serde 字样；类型层还有 `compile_fail` 证据）；重启即失。**收益**：零 schema、零键路径、零字节样本。
- **B. 第 3 层本机配置（`local_config.rs`）** —— **代价**：`deny_unknown_fields`（3 处）+ `FROZEN_LOCAL_CONFIG_KEYS`
  （4 个键）+ 版本闸门（只接受 `version <= 1`）⇒ 加字段要同时改三处并考虑升版。**语义上也不对**：
  该文件的自我描述（`local_config.rs:578-581`）是「本文件由本实现独占写入」的**机器绑定**——
  声卡端口 / 外部编辑器路径 / 密钥链引用指针；**不是**视图偏好。
- **C. 第 1 层工程（`YebanProjectV1`）** —— **代价**：判据 11 的 4 处（223 条键路径、摘要、顶层键表、2 份字节样本）
  + `schemas/project.schema.json` **一次全动**。**收益**：同一份作品在任何机器上打开都是同一个视图（Logic 如此），
  且投影仍是「工程 + 一个参数」的纯函数。
- **D. 新建第 4 层「视图偏好文件」** —— **代价**：一个新的持久层需要它自己的 schema / 版本 / 原子落盘
  （`ARCH-SEC-004`）/ 并发锁（`.yeban.lock`）故事，为几个整数不成比例；且 `MODEL-ISO-001` 只定义了**3 层**，
  加第 4 层要先改规范。**否决**。

**代码强制**：A 的「不能持久化」是机械的；B 的「拒绝未知字段 + 版本闸门 + 冻结键集」是机械的；
C 的「4 处同时移动」是机械的；`MODEL-ISO-001` 定义了 3 层。

**建议**：**A（第 2 层，host 拥有）作为第一片**，并把 C 明确列为**负责人裁决项**。理由：
① 代码已经把横向缩放写成**投影的入参**（`from_project_with_zoom`），把偏移写成**宿主参数**
（`apply_view(ui, view, viewport_width, scroll_x)`；`piano_roll` 的 `roll-scroll-x` 由宿主注入并在 `host.rs` 回写）——
视图状态的既有形状就是「宿主拥有、重新注入」；② 第 2 层不能持久化是机械事实，所以 A 零 schema、零键路径、零字节样本；
③ 唯一被牺牲的是「重启后记得」，而那是**产品问题**，不是工程问题。

### Q5 — 横向缩放数字（`ticks_per_pixel`）归谁？

**问题**：`ticks_per_pixel` 的权威值住在工程、本机配置，还是宿主暂态？

**选项与代价**：

- **A. 工程（像 Logic）** —— 代价 = Q4-C 的全部（键路径 223→224、摘要、2 份字节样本、顶层键表、schema）；
  收益 = 同一作品跨机同视图。**真正的代价**是把「我在看哪一段」变成**作品状态**，
  于是 `ARCH-DET-001`（写入确定性）要重新论证。
- **B. 本机配置** —— 代价 = Q4-B 的全部；且这不是机器绑定事实。
- **C. 宿主 / 会话暂态** —— **代价**：零 schema；重启回到默认（30 tick / 逻辑像素）。
  **收益**：`from_project_with_zoom` **终于有生产调用点**——它今天有 **0** 个，这正是「有能力、无消费者」的最小接线。

**代码强制**：`from_project_with_zoom` 的签名（缩放是**入参**）；`DEFAULT_TICKS_PER_PIXEL = 30`；
判据 15（本机判不了 golden）。

**建议**：**C 作为第一片**，并把 A 标为**负责人裁决**（「保存后重开是否记得缩放」）。理由：
投影 API 的形状已经把缩放定义成「每次投影的一个参数」而不是工程的一部分；C 是唯一不动 schema、
不动冻结字节的选项，且它正好消除审计点出的「有能力、无消费者」。

### Q6 — 编排滚动是否与缩放**同一交付**？

**问题**：放大后若无滚动，内容被裁掉且**够不到**。滚动要不要和缩放一起做？

**选项与代价**：

- **A. 分两片（先缩放，滚动另做）** —— **代价**：缩放一落地就是**一个把内容藏起来且够不到的模式**；
  `arrangement_view.slint` 今天 **0** 处滚动；判据只能测投影数字，测不到「够得到」。
- **B. 同片**：沿用钢琴卷帘**已被判据钉住**的形状（`callback scroll-requested(length)` + 宿主拥有偏移 +
  投影侧减偏移后重新注入），**不引入 `Flickable`**（`piano_roll.slint:39` 的注释是证据：
  Flickable 与宿主偏移双重计算）。触点：`arrangement_view.slint`（新 callback + 一个可拖的滚动条/手势）、
  `app.slint` 的接线（`scroll-requested` 已有先例 `app.slint:336/582`）、`host.rs` 的 `apply_view`（已有一个
  `scroll_x` 参数，钢琴卷帘在用）、`bridge.rs` 里剪辑/小节线/自动化顶点的 x 减去偏移
  （`visible_notes` 已经是这个形状：「位置**相对视口**（投影侧已减去 `scroll_x`）」）。
  **代价**：一条新判据（偏移单调、越界不 panic、`px_to_tick` 口径与投影一致——`bridge.rs:412-420`
  已有把视口 x（含滚动偏移）换回 tick 的注释口径）；golden 只在**默认偏移 ≠ 0 或新增可视控件**时变。

**代码强制**：`arrangement_view.slint` 零滚动；`piano_roll` 的既有形状；`app.slint` 的 `scroll-requested` 先例；
`bridge.rs:412-420` 的滚动口径注释。

**建议**：**B——滚动与缩放必须同一交付**。理由：没有滚动的缩放不是「能力的一半」，
而是一个**让内容不可达**的模式（等于把内容删掉）；滚动路径已被钢琴卷帘证明、被判据钉住，复用它零新机制。

### Q7 — 离散还是小数缩放级？`>= 1` 从哪来？

**问题**：缩放级是一组离散整数，还是由基准乘小数？

**选项与代价**：

- **A. 离散整数阶梯**（例如 `1, 2, 3, 4, 6, 8, 12, 16, 24, 30, 32, 48, 64, 96, 120, 192, 240, 384, 480, 960`，单位 tick / 逻辑像素）
  —— **代价**：手感是「档位」不是连续；**收益**：每一档可枚举、可判据（单调、边界、默认 30 在其中），
  且 7 档已测集合 `[1, 3, 7, 30, 32, 120, 960]` 是它的真子集。
- **B. 由基准乘小数**（`tpp = round(30 / 2^(k/n))`） —— **代价**：引入浮点与舍入，
  而 `D28` 第 2 条的理由正是「浮点在非 2 的幂上会坏」（`ticks_per_pixel = 30` 不是 2 的幂；
  判据 `tick_to_px(30, 30) == 1`、`tick_to_px(6, 7) == 0` 就在那里）。
- **C. 连续小数缩放，投影取整** —— 同 B 且更差：界面持有浮点、投影只看到整数 ⇒ 用户看到的档位不可复现。

**代码强制**：`ticks_per_pixel: u64` 与 `BridgeError::ZeroTicksPerPixel`（`bridge.rs:957` 的
`if ticks_per_pixel == 0 { return Err(...) }`）⇒ **下界 = 1 是类型/判据强制的**；`tick_to_px` 是整数除法。

**建议**：**A**。阶梯定义在**一处**（`bridge.rs` 的常量数组），下界 1、上界一个命名常量；
而**投影仍然接受任意 `tpp >= 1`**（保持 7 档/属性判据可任意扫）——即**阶梯是界面/宿主的约束，不是投影的约束**。
这样 `ZeroTicksPerPixel` 只能来自 bug，不能来自界面。

### Q8 — 折叠：**组实体**还是**仅视图状态**？

**问题**：「文件夹」是什么？一条普通轨道被别的轨道用 `folder_id` 指过去，还是一个一等实体？

**选项与代价**：

- **A. 仅视图语义（今天 schema 的形状）**：文件夹就是一条**普通轨道**，别的轨道通过 `folder_id` 指向它；
  折叠是投影/界面的行为。**触点**：零 schema；`validate()` 加固（Q13）；投影多一个「可见性」概念。
  **代价**：「它是文件夹」是**隐式角色**（有孩子 ⇒ 是文件夹），没有类型保护；而且它**可以**持有设备/剪辑/自动化
  与混音通道条（因为它是普通轨道）——这可能不是想要的。**但代码已经就是这个形状**，
  `project.rs:1113-1116` 与 `ROAD-M1-002` 的口径明文说它**只**做界面折叠。
- **B. 加 `TrackKind::Folder`（组实体）** —— **触点**：`project.rs:389` 的枚举 +
  `schemas/project.schema.json` 的 `tracks.*.kind.enum`（今天 **4** 个值）+ **每一处 `match track.kind`**
  （`kind_name()`、混音、路由节点、引擎快照等）+ 可能的迁移；`kind` 是 required 键。
  **golden**：若图标位随之变化则要。**代价**：语义更诚实（文件夹不是轨道），
  但爆炸半径是整个 `kind` 的消费面；而且它会与「Track Stack 就是一条真轨道」的 Logic 语义产生分歧。
- **C. 独立的 `folders: BTreeMap<EntityId, Folder>` 实体** —— **代价**：最大——要复制树/索引/混音通道机制，
  且 `folder_id` 从「指向轨道」变成「指向文件夹」，又是一次 schema 迁移。**不作为第一片**。

**代码强制**：`TrackV3::folder_id` 已持久化且只校验存在；`TrackKind` 4 个变体；`kind` 在 schema 的
`enum` 与 `required` 里；判据 7（index ↔ model）。

**建议**：**A 作为第一片**，把 B 明确登记为**负责人裁决项**（「文件夹是不是一等实体、能不能持有设备/剪辑、
要不要出现在混音台」）。理由：① 模型**已经**持久化了 A 的形状，且规范口径（`ROAD-M1-002`）已明说它只做界面折叠；
② B 的爆炸半径（每一处 `match kind` + schema enum）远超这个交互本身需要；③ A 的弱点（隐式角色 + 环）
由 Q13 的 `validate()` 加固补上，而那条加固**无论如何都要做**（它今天是一个陷阱）。

### Q9 — 折叠后，**隐藏轨道如何投影**？

**问题**：被折叠隐藏的轨道在 `ViewState` 里怎么表示？

**选项与代价**：

- **A. 从 `ViewState.tracks` 里移除** —— **代码强制否决**：`TrackView.index` 的文档（`bridge.rs:691`）
  明写「进 `track-{i}-*` 的语义 ID」；`elements.rs:1418` 断言 index ↔ 投影下标；
  `elements.rs:405/478/658` 三个族都按 `view.tracks.iter().enumerate()` 造 ID；
  `automation-lane-track-indexes` 也携带 index（`arrangement_view.slint:381` 的 ID 模板）。
  移除 ⇒ 后面**每一条轨道的 ID 全部重编号**（编排 + 混音 + 会话三族），`UI-TEST-001` 的稳定语义寻址当场失效，
  按下标索引的选中/撤销状态会被**静默重定向**。
- **B. 保留整个数组，加 `visible`/`folded`，并给出投影的 `y`/`height`** —— **代价**：需要一条
  「隐藏行的有效高 = 0（或跳过排布）」的规则；判据 5（双向覆盖）与 6（7 族）要求**注册表与模板同步**
  （隐藏行仍登记，或两侧一起学可见性）。
- **C. 由投影给所有消费者供行几何**（`row-ys` / `row-heights` 那一族；把行几何从 `.slint` 搬进投影；S0 已落地为注入的 `track-ys` / `track-heights` / `clip-ys` / `clip-heights`）—— 与 B 是同一件事的两面：
  B 说「数组保留什么」，C 说「谁算 y」。**C 正是纵向变高需要的同一个改动。**

**代码强制**：A 被判据 5/6/7/8 + `UI-TEST-001` 按构造否决；
`arrangement_view.slint` 三处（包头 / 车道 / 剪辑）的**行算术**必须搬进投影（`D28` 第 3 条）；S0 已落地（`ee7fad5`），三处改读注入的 `track-ys` / `track-heights` / `clip-ys` / `clip-heights`。
代码**唯一**强制的是：`.slint` 模板与 `elements.rs` 必须保持**互相覆盖**——所以隐藏不能只在一侧发生。

**建议**：**B + C 合取**：数组**保留完整**（索引稳定），`visible`/`folded` 是投影字段，
`y`/`height` 由投影统一供给（前缀和；隐藏行高 0 或被跳过排布），
剪辑车道 / 自动化带 / 标尺全部读**同一份**行几何。明确**否决 A**。

### Q10 — 折叠行在标尺里显示什么？

**问题**：折叠成一行后，那条「堆叠行」在时间轴上画什么？

**选项与代价**：

- **A. 什么都不显示（只是一个包头）** —— **代价**：折叠与「空轨」在时间轴上无法区分。最便宜。
- **B. 子轨的并集跨度**（从子轨最早的 `start_px` 到最晚的 `end_px` 画一条区间） —— **代价**：
  投影多一条派生数组（并集跨度的 `x` / `width`），完全由既有 `ClipView` 推导；
  不需要新栈规则；`clip-{ulid}-*` 的 ID **不重复**（只画区间，不画子剪辑）。
- **C. 显示子轨的剪辑（堆叠）** —— **代价（关键）**：今天的 `Vec<ClipView>` 是**扁平**的，
  `ClipView::lane` 只是一个行号（`i32`）；要把子剪辑堆到折叠行上，就必须发明一条**重叠打包/堆叠规则**
  （投影里今天不存在，规范也没有定义）；而且每个剪辑会同时出现在「折叠行」与「子行」两处
  ⇒ 同一条 `clip-{ulid}-header` 出现两次，破坏 `UI-TEST-001` 的 ID 唯一性。**出局**。

**代码强制**：`ClipView` / `ClipView::lane` 扁平；判据 5（双向覆盖）与 `clip-{ulid}-*` 的唯一性；`elements.rs` 的 clip 族。

**建议**：**B**（并集跨度），**C 明确出局**（它需要的堆叠规则既不在代码里也不在规范里，且产生重复语义 ID）；
A 可作为第一片的最小步，但 B 只多一条派生数组，收益是「折叠 ≠ 空」。
区间必须在**投影**里算（`.slint` 零算术）。

### Q11 — 折叠后混音台怎么办？

**问题**：折叠的通道条在混音台里隐藏，还是保留？

**选项与代价**：

- **A. 也隐藏折叠通道条** —— **代价**：`host.rs` 只注入**一份** `track-names`（`host.rs:131` `set_track_names`），
  `app.slint:503/513/587` 把它同时给编排与混音；`mixer_console.slint:78` 按 `track-names` 迭代并造
  `track-{i}-channel-strip / -fader / -meter`。隐藏 ⇒ 混音台也要重编号（同 Q9-A 的破坏），
  **或者**引入第二条「混音可见」数组 + 一条新不变式。
- **B. 保留折叠通道条** —— **代价**：混音台显示一条在编排里看不见的轨道
  （可解释为「折叠是时间轴的手势，不是混音的路由」）；**零重编号、零新数组**。

**代码强制**：单一 `track_names()`（`bridge.rs:1138`）与 `host.rs` 的单次注入；判据 6（7 族）；
`mixer_console.slint:78` 的迭代。

**建议**：**B（保留）作为第一片**，把「混音台要不要隐藏」登记为一个**可延后**的问题
（它需要一条新的可见性维度与一条新不变式，属于第二个交互而不是折叠本身）。
理由：单一数组是当前唯一的规模事实源，动它就会把 Q9 的重编号问题**再复制一份**到混音台。

### Q12 — `collapsed` 位住哪？

**问题**：「这条轨道被折叠」这个位放哪？（第 2 层不能持久化）

**选项与代价**：

- **A. 第 2 层会话态（host 拥有，重启即失）** —— **代价**：零 schema；「重开后折叠丢失」。
  **机械强制**：第 2 层不能持久化（`model_isolation.rs:539`）。
- **B. 工程（折叠轨上的 `collapsed: bool`）** —— **代价**：Q4-C 的全部（223 条键路径、摘要、2 份字节样本、
  顶层键表、schema）；**收益**：折叠是作品状态（Logic 如此）。
- **C. 派生（不存位）** —— **不可能**：没有位就无法表达「持续折叠」这种状态。

**代码强制**：判据 11（4 处移动）；判据 12（A 不能持久化）。

**建议**：**A 作为第一片**；**A 与 Q5 必须同一裁决**——横向缩放与折叠位是**同一族**「视图状态」，
**不许一个进工程、一个留会话**（那会产生两套口径）。若负责人要「重开记得」，则**两个一起**进工程，
且**一次 schema 变更**带上高度/缩放/折叠三个键，而不是分三次移动冻结字节。

### Q13 — 折叠的**校验漏洞**：拒绝自指/环/主总线，还是只加访问集上限？

**现状**：`validate()`（`project.rs:1673-1676`）只查 `self.tracks.contains_key(&folder_id)`。
`tracks` 含主总线 ⇒ **自指**（`folder_id == track.id`）、**环**（a→b→a）、**指向主总线**
今天**全部通过**（三个都是构造上可证的，因为 `contains_key` 对这三种情况都为真）。

**选项与代价**：

- **A. `validate()` 拒绝自指 + 环 + 指向主总线** —— **代价**：需要一个新的 `ModelError` 变体
  （`error.rs`；加变体是加法，但任何**穷尽匹配**该枚举的地方要跟着改——实现前必须 grep 确认匹配面）；
  纯 `yeban-model`、本机可跑、**无 schema 变更**。
- **B. 只加访问集上限**（允许文件夹指向文件夹，深度不超过轨道数） —— **代价**：允许自指/环
  「通过校验但在投影里被截断」，把**文档缺陷**变成**静默降级**（违反 `D43` 的「响亮失败」）。
- **C. 不校验，投影自己防**（坏 `folder_id` 视为不折叠） —— **代价**：环会让任何「顺着指针走」的实现
  **不终止或静默截断**（审计点名的陷阱）；坏数据在文件里继续存在。**否决**。

**代码强制**：`validate()` 的现有形状；`tracks` 含 master；`ModelError` 是 thiserror 枚举；
`project.rs` 的 `validate()` 有本机单测。

**建议**：**A + B 合并**：用**访问集**实现（结构上保证 O(轨道数) 终止），同时拒绝自指、环与主总线目标。
**不发明**一个与结构无关的魔数深度上限——访问集本身就是深度上限（一条无环链最长 = 轨道数）。
这条是**纯模型、本机可跑**的，应当在**折叠第一片之前**落地（它是「顺着指针走」的前置条件）。

---

## 建议的切片顺序

负责人第 418 轮要求「各自一个切片、各带能变红的判据」。审计点名的那条共享事实
（「投影供行」既被纵向变高需要、又被折叠需要）决定了顺序：**共享改动必须最先**，
否则会先长出两套行模型，再被迫合并——那正是 `automation.rs:67-70` 自认的旧病。

| 切片 | 交付 | 依赖的裁决 | 落地什么 | schema | golden |
| :-- | :--- | :--- | :--- | :--- | :--- |
| **S0（共享前置，必须最先）— 已落地 `ee7fad5`（2026-10-07）** | 投影供行几何 | Q1（单位）、Q2（夹紧住投影）、Q3（带同行）、Q9-B/C | `ViewState` / `host` 供 `track-ys` / `track-heights` / `clip-ys` / `clip-heights`（前缀和）；`TrackView` / `ClipView` 带 `y`/`height`；`automation.rs` 的带从**同一份**行几何派生（新入口 `project_lanes_with_rows`）；`arrangement_view.slint` 三处改读注入数组；`lane_element_ids_match_the_slint_template` 的文本层断言改写成「`.slint` 不做行算术」 | **否** | **默认几何不变 ⇒ 5 张 Linux Tier-1 Golden 不变**（这条本身就是判据：重构必须逐字节保持默认帧） |
| **S1（纵向轨道高度）** —— **已落地 `483707b`（2026-10-07）**；**拖拽手势与 `ui/*` / `Action` 入口未随之落地，留给后续切片**（本 ADR 的裁决与代价一节**未改**，只标该切片的落地状态） | 每轨高度 + 全局高度缩放级 | Q1、Q2、Q3、Q4 | `row-heights` 由 `clamp(height_px × 百分比 / 100)` 得到；一个拖拽手势；S0 的**第一个真消费者** | 只在 Q1 = A/C 时要 | 只在默认几何或新增可视控件时变 |
| **S2（横向时间轴缩放 + 编排滚动）** | 滚动 + 缩放级 | Q5、Q6、Q7 | 宿主拥有 `ticks_per_pixel`（离散整数阶梯）+ `arrangement_scroll_x`；`from_project_with_zoom` **第一个生产调用点**；`arrangement_view.slint` 加 `scroll-requested`（复用钢琴卷帘形状，**不用 `Flickable`**）；剪辑/小节线/自动化顶点在投影里减偏移；`host.rs:548` + `cli.rs` 的表 + B11b **同提交**一起动 | 只在 Q5 = A 时要 | 只在默认缩放 ≠ 30 或新增可视控件时变 |
| **S3（轨道折叠）** | 折叠（含 Q13 前置） | Q8、Q9、Q10、Q11、Q12（+ Q13 先落） | `validate()` 加固（Q13，可先单独落）；投影加 `visible`/`folded` + 隐藏行高 0；标尺并集跨度（Q10-B）；混音台保留（Q11-B）；`collapsed` 位按 Q12 | 只在 Q12 = B 时要 | 折叠默认「不折叠」时可能不要；一旦新增折叠按钮即要（5 张） |

**顺序的理由**：

1. **S0 必须最先**，因为它是纵向变高与折叠**唯一共享**的改动（审计原话：
   "the projection-supplies-rows change is shared by vertical resizing and folding"）。
   先做它，两个特性才从**同一个**行几何派生；后做它，就会先长出两套行模型（编排一套、自动化一套、折叠又一套），
   然后被迫合并。S0 的验收标准是**默认帧逐字节不变**，所以它能在**零 Golden 再生成、零 schema 变更**下落地；
   而 `test_port_adapter.rs:174` 说明本机（macOS，无 `tests/golden/macos/`）**判不了** golden，
   所以「默认不变」这条判据把 S0 的回归风险压到最低。
2. **S1（纵向）是 S0 的第一个消费者**，紧随其后。理由：本仓刚在 `7c995e7` 里因为「能力没有被消费」
   而改过一次快捷表（`ZoomToSelection` / `ZoomToFit` 被标 `(未实现)`）——一个 S0 若不为任何特性服务，
   就是同一个错误的新实例。纵向变高是负责人点名的第一交互，也是**每轨**语义的落点。
3. **S2（横向缩放 + 滚动）独立于行几何**，所以它的位置与 S1 可换（两条线的触点不重叠：
   S1 在行几何，S2 在 x/滚动）。把它排在 S1 之后而不是之前，是因为它要同时动**三处**与
   「能力落地陈述」耦合的地方（`host.rs:548`、`cli.rs` 的 `implemented`、`cli_contract.rs:1088`），
   是一笔独立的、与行几何无关的账。**但滚动必须与缩放同片**（Q6）。
4. **S3（折叠）最后**，因为它**未决问题最多**（Q8–Q12），且两个副作用（标尺显示、混音台是否隐藏）
   各自会牵动新的数组/不变式；把 Q13 的校验加固放在它**之前**（或作为 S3 的第一步），
   是因为「顺着 `folder_id` 走」的代码在没有访问集时会不终止。
5. **贯穿三片的一条纪律**：Q5 与 Q12 是**同一族**问题（视图状态住哪）。若负责人裁定「进工程」，
   则三片**共用一次 schema 变更**（高度/缩放/折叠三个键一次加），使 **223 条递归键路径**、
   其摘要、**2 份逐字节样本**与顶层键表**只移动一次**，Golden 也只按 `ADR-0003` 的程序再生成一次。
   若裁定「留会话」，则三片**都**零 schema 变更。

---

## 待负责人裁决（本 ADR 不代答）

| 裁决 | 为什么不归实现者 | 实现者在此期间会做什么 |
| :--- | :--- | :--- |
| **Q1**（`height_px` 是否进工程） | 「行高是不是作品的一部分」是产品语义：两种读法是两个不同的产品（同一份作品是否跨机同形） | 按建议的 C 设计；若裁「仅全局缩放」则删掉 `height_px` 那一半 |
| **Q4 / Q5 / Q12**（视图状态住哪一层） | 「重开时记不记得我的缩放与折叠」是产品问题；工程上的 3 层已经固定，选哪层不是工程能替的 | 按建议全放第 2 层（零 schema）；若裁「进工程」则**三键一次**进 |
| **Q8**（文件夹是不是一等实体） | 它改变数据模型语义（文件夹能不能持有设备/剪辑、要不要出现在混音台），不只是代码 | 按建议 A（仅 `folder_id`）实现；B 的爆炸半径留待裁决 |
| **Q10 / Q11**（折叠行显示什么、混音台是否隐藏） | UX 产品决策；Agent 只给默认建议与代价 | 按建议 B / B 实现（并集跨度、混音台保留） |
| **Q13** 的*策略*（坏 `folder_id` 是报错还是降级） | 终止机制是工程强制；「报错 vs 降级」偏产品口径 | 按建议 A（拒绝 + 访问集，`D43`「响亮失败」） |

**工程裁决（不需要负责人）**：Q2（`D28` 强制整数位置与投影唯一注入点）、Q3（9 个平行数组强制同行）、
Q6（没有滚动的缩放让内容不可达）、Q7（`ZeroTicksPerPixel` 强制下界 1 + `D28` 的整数理由）、
Q9（判据 5/6/7/8 与 `UI-TEST-001` 强制索引稳定）、Q13 的终止机制（访问集）。

**登记动作**：把上述待裁决项登记为新的 `HD-nn` 会牵动 `docs/ledger/human-decisions.md` 的表头计数
与**四份活文档**的 HD 区间（`docs/README.md`、`docs/ledger/phase-status.md`、`docs/ledger/feature-alignment.md`
+ 清单自身），由 `scripts/gates/check_decisions.py` 机械对账。
那是**集成者 / 负责人**的登记动作；本 ADR 只登记在**索引表**（`docs/adr/README.md`），
**不擅自改动那些计数**（与 `ADR-0003`「登记状态」同一处置）。

---

## 代价（必写）

1. **每一片都有 Golden 风险**：三个交互都是「真状态 + 真交互 + 投影刷新」，任何**默认可视几何**或新增可视控件的
   改动都要按 `ADR-0003`「批准后的再生成程序」在 CI 上重做 **5 张** Linux Tier-1 PNG。
   本机 macOS **判不了**（`test_port_adapter.rs:174`：平台无基准 ⇒ 打印「未被判定」并 `return`）。
2. **持久化口径的代价是一次性的**：若负责人裁「进工程」，代价是**一次**键路径（223 条）/ 摘要 /
   2 份字节样本 / 顶层键表 / schema / 5 张 Golden 的联动移动；若裁「留会话」，代价是「重启后视图丢失」。
   两种代价都真实，但**只付一次**（三键一次加）比付三次便宜。
3. **S0 改写了一条既有判据**（`automation.rs:1690-1703` 的文本层断言）。这**不是**削弱门禁：
   断言要从「`.slint` 里有这串字面量」改成「`.slint` 不做行算术、行几何来自投影数组」；
   判别力必须**更强**（旧断言只钉住一个字符串，新断言钉住「唯一事实源」）。
4. **折叠第一片不隐藏混音台**（Q11-B），所以混音台会显示编排里看不见的轨道。
   这是**有意的**不对称，等 Q11 的裁决再改；代价是「折叠」在混音台一侧暂时不是一个完整的概念。
5. **S1 的默认行高必须仍是 56 逻辑像素**（`DEFAULT_TRACK_HEIGHT_PX = 56`），否则 5 张 Golden 必变、
   且「默认倍率 100%」这条设计承诺会从第一片起就丢掉。
6. 本 ADR **不实现任何东西**：无 Rust、无 `.slint`、无 schema、无 Golden、无台账改动。

## 备选方案及其代价（逐条否决的理由）

1. **单一全局高度缩放**（Q1-B）：无法表达每轨差异，砍掉负责人原话的一半。**否决**（但可作为纯视图的一部分保留）。
2. **夹紧放模型 `validate()`**（Q2-C）：`from_project` 不走 `validate()`，会留下「投影不设防」的第二条路径；
   且把 UI 边界升格为文档合法性。**否决**（保留「拒绝退化值 `0`」这一条）。
3. **隐藏即从数组移除**（Q9-A）：判据 5/6/7/8 与 `UI-TEST-001` 按构造禁止。**否决**。
4. **折叠行画子剪辑**（Q10-C）：需要一套今天不存在的堆叠规则，且产生重复的 `clip-{ulid}-*`。**否决**。
5. **混音台隐藏折叠通道**（Q11-A）：把 Q9 的重编号问题复制到混音台，或引入第二条可见性数组。**否决（第一片）**。
6. **加 `TrackKind::Folder`**（Q8-B）：爆炸半径是整个 `kind` 消费面 + schema enum + 迁移。**不作为第一片**（登记待裁）。
7. **第 4 层「视图偏好文件」**（Q4-D）：要补齐 schema / 版本 / 原子落盘 / 锁，且 `MODEL-ISO-001` 只有 3 层。**否决**。
8. **小数 / 连续缩放**（Q7-B/C）：与 `D28` 的整数理由冲突，且界面档位不可复现。**否决**。
9. **引入 `Flickable` 做编排滚动**：`piano_roll.slint:39` 的注释是证据（与宿主拥有的偏移双重计算）。**否决**；
   沿用既有的 `scroll-requested` + 宿主偏移形状。
10. **加 `#[ignore]` 或放宽任何门禁**：任务与 `AGENTS.md` §2 都禁止。**不做**。

## 可推翻性

本裁决**可被推翻**。负责人若选上表任一备选，本文件**就地更新**（不另开一份）：
把该备选写进对应问题的「建议」，并同步改写「代价」与「切片顺序」。

若负责人裁定「高度 / 缩放 / 折叠**都进工程**」（或都不进），受影响的 Q1 / Q4 / Q5 / Q12 一次性改写，
S0–S3 的 `schema` 与 `golden` 两列随之更新；反之若裁「都留会话」，则三片**都**零 schema 变更。

任何改判都会让「一次 schema 变更 + 一次 Golden 再生成」的成本**再发生一次**（如果已经按本裁决落地过）。

## 登记状态（供后续集成者接手）

本 ADR 登记在 `docs/adr/README.md` 的索引表（状态 `Proposed（待人类裁决）`）。

`docs/ledger/human-decisions.md` 的逐行计数与 `HD-01..HD-NN` 区间由 `scripts/gates/check_decisions.py`
机械对账；把本 ADR 的待裁决项登记为新的 `HD-nn` 会牵动**四份活文档**——那是集成者 / 负责人的登记动作，
本 ADR **不擅自改动那些计数**（与 `ADR-0003`「登记状态」同一处置）。

本 ADR **不改任何状态**（单位已注明）：

- Phase 4 的 **11 条**阶段要求：已完成 **7** / 部分 **4** / PENDING **0**（`docs/ledger/phase-status.md:121`）；
- 门禁表的 **21 条**：已接线 **19** / 部分 **0** / PENDING **2**（`docs/ledger/gate-status.md`，快照同）；
- `docs/ledger/human-decisions.md` 的 **52 项**：已裁决 **48** / 未决 **4**（`HD-48`、`HD-49`、`HD-50`、`HD-52`）；
- `docs/ledger/feature-alignment.md` 的三方对齐矩阵不变。
