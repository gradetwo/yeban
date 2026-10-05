# `line/app-automation-ui` — 把自动化泳道**画出来**，并让 AI 在控件树里**读到它**

> 工作线：`app-automation-ui`（分支 `line/app-automation-ui`，worktree
> `.worktrees/app-automation-ui`，基线 main `42442af`）
> 地盘：`crates/yeban-app/**`（含 `src/**`、`ui/**`、`tests/**`）+ 本文件
> 上游：`line/model-automation`（`docs/ledger/model-automation-notes.md`）**只读**
> 规范来源：`[MODEL-AST-001]` `[MODEL-AST-002]` `[MODEL-AST-003]` `[ARCH-DET-001]`
> `[UI-TEST-001]` `[UI-NOTE-002]` `[MODEL-ISO-001]` `[MUST-GATE-015]`；
> 裁决依据：ADR-0001 **D28**（投影零 Slint / 唯一注入点）、**D22**（debug info 才能内省）、
> **D23**（SSIM 灵敏度边界）、**D43**（无兼容包袱）；风险：**RSK-31**（自动点密集）

---

## 0. 这次补的是哪一格

`yeban-model` 已经把自动化的**模型侧**落地了（泳道 / 采样点 / 曲线 / 读开关 / 写模式 /
取值域 / **唯一求值入口** `automation_value_at`，28 条判据 + CI 绿），但界面侧**一根曲线都没有**：

| 缺口 | 症状（实测） |
| :--- | :--- |
| 没有投影 | `crates/yeban-app/src/bridge.rs` 的 `ViewState` 里没有任何 `automation_lanes` 字段；`TrackV3::automation_lanes` 在 app 侧**一个引用都没有**（`grep -rn "automation" crates/yeban-app/` 只命中 `automation_lanes: BTreeMap::new()` 这一行夹具初始化） |
| 没有画 | `ui/workspace/arrangement_view.slint` 里没有 `Path`、没有自动化元素 |
| AI 读不到 | 控件树里没有"哪条轨的哪个参数现在是多少"的载体 ⇒ "AI 能读到自动化"没有可投影、可截图的证据 |

本线补这一格（Phase 3 的界面切片）。**判据的形状**是"机械证据"而不是"画了几条线"：
每条泳道一个**稳定语义 ID** + `accessible-label` 里带**单位与当前值**，且那个值必须等于
**模型唯一求值入口**的返回值。

---

## 1. 交付物 ↔ 规范 ID

| 交付物 | 规范 ID | 本线做了什么 |
| :--- | :--- | :--- |
| `crates/yeban-app/src/automation.rs`（**新建**，1651 行） | `[MODEL-AST-001]` `[MODEL-AST-002]` `[MODEL-AST-003]` `[ARCH-DET-001]` `[UI-NOTE-002]` `[UI-TEST-001]` `[MODEL-ISO-001]` | **零 Slint** 的自动化投影层：目标 / 单位 / 值域（含自适应）/ 采样点顶点 / 缓动折线 / `Path` 指令 / 读·写显示态 / 元素 ID / 无障碍标签；9 条本机可跑判据（含 4 处注入的靶子） |
| `crates/yeban-app/src/bridge.rs` | `[UI-GRID-001]` `[MODEL-AST-002]` | `ViewState::automation_lanes`（权威投影产物）+ 9 个平行数组访问器 + `from_project_with_zoom_and_cursor`（走带位置的接线点）+ `canonical_lines` 的泳道/顶点明细（确定性可比较的字节） |
| `crates/yeban-app/src/host.rs` | ADR-0001 **D28**（唯一注入点） | 9 个 `ui.set_automation_*` / `set_automation_path_commands`；**没有第二份注入实现** |
| `crates/yeban-app/ui/workspace/arrangement_view.slint` | `[UI-TEST-001]` `[UI-NOTE-002]` | 自动化曲线层：`Path`（SVG `commands` + 显式 `viewbox` + `fit: ImageFill.fill`）+ 轴文本 + 角标；元素 ID 由三段字面量拼出 |
| `crates/yeban-app/ui/app.slint` | `[UI-GRID-001]` | `MainWindow` 的 9 个注入属性 + 单向转发给 `ArrangementView` |
| `crates/yeban-app/src/elements.rs` | `[UI-TEST-001]` `[UI-MCP-001]` | 每条泳道登记 `track-{i}-automation-{目标键}-lane`（角色 `image`，标签 == 投影标签） |
| `crates/yeban-app/src/test_port_adapter.rs` | `[UI-TEST-001]` `[UI-MCP-002]` `[MUST-GATE-015]` `[MODEL-AST-002]` | Tier-1 运行时断言：默认视图 + `filled_project` 两侧的泳道数 / 标签 / 值 == 求值入口 / 读关可区分 / 录制臂可读 / 同窗口换工程换泳道 |
| `crates/yeban-app/src/lib.rs` | — | `pub mod automation;` |
| 本文件 | — | 字段表、复用证据、控件树→数值→截图区域、判据与注入、本机 vs CI、未实现项、needs |

**没有碰**：根 `Cargo.toml` / `Cargo.lock`（**零新增依赖**，实测 `git diff Cargo.lock` 为空）、
`.github/**`、`scripts/**`、`deny.toml`、`docs/DEVELOPMENT_LEDGER.md`、`docs/adr/**`、
`docs/YEBAN_*.md`、`schemas/**`、**其它任何 `crates/**`**（`yeban-model` 只读）、`spikes/**`、法务文件。

---

## 2. 投影字段表（模型 → 投影 → `.slint` → 控件树）

### 2.1 一条泳道的字段（`automation::AutomationLaneView`）

| 模型字段 / 入口 | 投影字段 | `.slint` 属性 | 控件树 / 判据 |
| :--- | :--- | :--- | :--- |
| `TrackV3::automation_lanes` 的键 `AutomationTarget` | `target`（原样保留，判据用它重调入口） | — | 元素 ID 的第三段 |
| —（目标的**唯一**命名口径） | `target_key` = `volume` / `pan` / `send-{edge}` / `device-{slot}-{param}` / `macro-{i}` | `automation-lane-target-keys[i]`（循环驱动数组） | `track-{i}-automation-{target_key}-lane` |
| —（目标的人类可读短名；设备参数 / 宏取文档里的**名字**，永不取值） | `target_label` | —（只进标签） | `accessible-label` 的中段 |
| 非主总线轨道在 `tracks` 里的序号（`BTreeMap` 键序） | `track_index` | `automation-lane-track-indexes[i]` | `track-{i}-…` 的 `{i}` |
| `TrackV3::id` / `name` | `track_id` / `track_name` | —（只进标签） | `accessible-label` 的前缀（`"Lead · 音量 …"`） |
| `AutomationLane::unit()` → `AutomationUnit::symbol()` | `unit` / `unit_symbol` | `automation-lane-axis-labels[i]` | 标签里的单位（`dB` / 空） |
| `AutomationLane::effective_domain()`（`None` ⇒ **曲线最值自适应**） | `domain_min` / `domain_max` / `domain_adaptive` | 轴文本（自适应时带「自适应」） | 轴标签：`dB [-60.0, 12.0]` / `[200.0, 4000.0] 自适应` |
| `AutomationLane::points_in_tick_order()` | `points: Vec<AutomationVertex>`（**每个采样点一个**） | —（`samples` 才是画出来的） | 判据 ② 的分母（== 模型条目数） |
| `AutomationPoint::tick` | `vertex.x` = `bridge::tick_to_px(tick, tpp)`（整数除法） | `automation-path-commands[i]` | 元素 `bounds.x`（CI 可读） |
| `AutomationPoint::value` | `vertex.y` = `(1 − fraction) × band_height` | 同上 | 元素 `bounds.y` |
| `automation_value_at(target, cursor_tick)`（**唯一求值入口**） | `value_at_cursor` | —（进标签） | 标签里的数值（判据逐位对账） |
| `AutomationTarget::static_value()` | `static_value` | —（进标签） | 读关 / 空泳道时的 `（静态 X）` |
| `AutomationTarget::validate_against()` | `target_reconciled` | —（进标签） | 目标不存在时的 `目标不存在` |
| `AutomationLane::read_enabled` | `read_enabled` | `automation-lane-read-enabled[i]`（描边色） | 标签里的 `读关闭` |
| `AutomationLane::write_mode` | `write_mode` / `write_label` / `badge` | `automation-lane-badges[i]`（角标） | 标签里的 `录制臂 {write_label}` |
| —（纵向等分，见 §2.2） | `band_y` / `band_height` | `automation-lane-band-ys[i]` / `-heights[i]` | 元素 `bounds` |
| —（拼接口径，见 §2.3） | `element_id` | 三段字面量拼出同一个串 | 元素 ID 本身 |
| —（标签口径） | `label` | `accessible-label` | 与投影**逐字相等**（两侧各一条判据） |
| —（折线顶点） | `samples` / `path_commands` | `commands:` 绑定 | 曲线真的被画出来（像素） |

### 2.2 纵轴与布局的**唯一**口径

```text
fraction(value) = (value − min) / (max − min)          # min == max 或非有限 ⇒ 0.5（带正中）
y_offset(value) = (1 − fraction) × band_height          # y 向下：值越大越靠上
usable          = 56 − 2 × 2                            # 车道高 − 上下内缩（AUTOMATION_BAND_INSET_PX）
band_height     = usable / 一条轨道上的泳道数
band_y          = 42 + 56 × track_index + 2 + band_height × band_index
```

- `42` / `56` 是 `arrangement_view.slint` 里**已经在用**的车道几何
  （`y: 42px + 56px * track_index`）。两处各写一份是**已知耦合**，由本机可跑的文本层判据
  `lane_element_ids_match_the_slint_template` 的最后两条断言对账
  （口径与 `app-completion` 的音高车道 `14px * lane_index` 完全同源）。
- **只钳像素，不钳值**：模型文档明确"`domain` 不是合法值钳位"（它只是显示域），
  因此值域外的采样点 `value` **原样保留**，只有 `y` 被钉进车道带内（判据 ③ 两侧都断言）。
- 值域外的值必须被钳进带内的**第二个理由**：0 面积 / 亚像素的元素会被上游裁剪语义过滤掉，
  于是"语义 ID 登记了却查不到"（`app-binding-notes.md` §4.1 踩过同一类问题）。
- 一条轨道多条泳道 ⇒ 按键序（`AutomationTarget::Ord`）**等分**成多条带。顺序确定、
  几何可判据；"多泳道叠加"不是静默丢数据。**没有累加**：`band_y` 只有一次乘法。

### 2.3 `tick → 像素` **复用了哪一个函数**（附证据）

**复用的是 `crate::bridge::tick_to_px`**（`crates/yeban-app/src/bridge.rs:279`）——
就是 clips / notes / 段落 / 小节线用的那一个（**没有第二套换算**）：

```text
$ grep -n "tick_to_px(" crates/yeban-app/src/*.rs        # 生产调用点（去掉测试与注释）
crates/yeban-app/src/automation.rs:441:    let x = tick_to_px(tick, ticks_per_pixel)? as f32;   ← 本线唯一调用点
crates/yeban-app/src/bridge.rs:651:  bar_positions.push(as_px(tick_to_px(tick, ticks_per_pixel)?));
crates/yeban-app/src/bridge.rs:1262: let x = as_px(tick_to_px(start_tick, ticks_per_pixel)?);   ← clip_view
crates/yeban-app/src/bridge.rs:1293: let x = as_px(tick_to_px(start_tick, ticks_per_pixel)?);   ← note_view
crates/yeban-app/src/bridge.rs:1333: let x = as_px(tick_to_px(section.start_tick, ticks_per_pixel)?);
```

判据（三处独立证据，全部本机真跑）：

1. `automation::tests::vertices_follow_the_points_in_tick_order` 对演示工程的**每一个**
   顶点断言 `vertex.x == tick_to_px(vertex.tick, DEFAULT_TICKS_PER_PIXEL)`；
2. 同一判据里 `x` 的单调性由 **tick 序**决定（`points` 非递减、`samples` 严格递增）；
3. **注入 B**（把 `x` 改成索引布局 `40.0 × index`）⇒ 该判据立刻变红（§5.2）。

### 2.4 "用的是同一个求值入口"的证据

`crates/yeban-app/src/automation.rs` **不 import `CurveType`、不写 `ease` / `interpolate`**
（`grep -n "CurveType\|\.ease(" automation.rs` 只命中 `mod tests` 里的夹具与文档注释）——
因此"app 里另写一份插值"在**类型层面**就无法发生，而不是靠注释承诺。

生产侧只有两个值来源：

```text
crates/yeban-app/src/automation.rs:508:  if let Ok(Some(value)) = project.automation_value_at(target, tick) { … }   ← 缓动细采样
crates/yeban-app/src/automation.rs:598:  .automation_value_at(target, cursor_tick)                                     ← "当前值"
```

采样点上的顶点直接用 `points_in_tick_order()` 给出的 `point.value` —— 那是模型文档点名的
画折线入口，且判据把它与 `lane.value_at(tick)` **逐位**对账（同 tick 多点时按
`(tick, point_id)` 最大者胜出的裁决对账，见判据 ②）。

---

## 3. `.slint` 的改动清单（以及为什么必须这么写）

| 文件 | 改动 | 为什么必须 |
| :--- | :--- | :--- |
| `ui/app.slint` | 9 个 `in property`（`automation-lane-target-keys` / `-track-indexes` / `-labels` / `-axis-labels` / `-band-ys` / `-band-heights` / `-read-enabled` / `-badges` / `automation-path-commands`）+ 转发给 `ArrangementView` | 数组必须由 Rust 侧 `ModelRc` 注入；界面里不留第二份数据 |
| `ui/workspace/arrangement_view.slint` | 见下 | **核心**：曲线、轴文本、角标、元素 ID |

曲线用 `Path` 的 SVG `commands`（上游语言参考：`commands` 是 `string`，坐标**不带单位**，
"operate within the imaginary coordinate system of the scalable path"）。三条与"坐标逐位等于
投影的像素"有关的细节是**查上游源码后写死**的（不是试出来的）：

1. **必须显式给 `viewbox-*` + `fit: ImageFill.fill`**：不写 viewbox 时上游用"命令包围盒"
   做 scale-to-fit（`Path.width` 的文档："If non-zero, the path will be scaled to fit into the
   specified width"），那会把**纵轴量程**变成"曲线自己撑满整个带"，值域映射当场失效。
   `viewbox-width: parent.width / 1px`（`length / length → float`）让 viewbox 恒等于元素尺寸
   ⇒ 缩放恒为 1.0。
2. **`width` / `height` 必须非零**：软件后端的 `draw_path` 先做 `should_draw(几何)`
   （`i-slint-renderer-software-1.18.1/lib.rs` 的 `draw_path`），0 面积元素**不画**。
3. **`clip: true` 挂在包裹的 `Rectangle` 上**：`clip` 只允许挂在 `Rectangle` / `Path` 上
   （上游 `passes/clip.rs:41`）；带内的裁剪由包裹元素负责，曲线不会溢到相邻轨道。

元素 ID 由**三段字面量**拼出（`[UI-TEST-001]` §12.2 的模板必须可被文本层对账）：

```slint
accessible-id: "track-" + root.automation-lane-track-indexes[lane_index]
             + "-automation-" + root.automation-lane-target-keys[lane_index] + "-lane";
```

它与 `automation::lane_element_id(track_index, target_key)` 拼出的串逐字相同；
两侧一致性由 `elements::slint_accessible_ids_and_registry_cover_each_other`（双向覆盖）
与本机文本层判据 `automation::tests::lane_element_ids_match_the_slint_template` 共同钉住。

**没有碰**：`ui/console/**`、`ui/dialogs/**`、`ui/sidebar.slint`、`ui/status_bar.slint`、
`ui/transport.slint`、`ui/tokens.slint`（一个令牌都没加）。

---

## 4. 控件树属性 → 数值 → 截图区域

### 4.1 属性与数值（**真实运行时控件树**，本机真跑）

下表是 `target/ui-test-port/app-introspect-observations.txt` 里的原文（本机在**真实
`i-slint-backend-testing` 平台 + 软件光栅化**上跑出来的观察值，不是投影字段的转述）：

```text
控件树计数: 注册表 214 条 / 运行时 104 条 / 运行时有而注册表无 0 条 / 注册表有而运行时无 110 条
重复族实测: track-*-automation-*-lane=3 (工程里 3 条)
可见重复族里可被语义 ID 寻址的样本: 12/12 [..., "track-0-automation-volume-lane",
                                            "track-0-automation-device-0-0-lane", ...]
[automation] 工程泳道 track-0-automation-volume-lane  -> label="鼓 · 音量 自动化 -3.2 dB · 录制臂 触碰"
[automation] 工程泳道 track-0-automation-device-0-0-lane -> label="鼓 · cutoff 自动化 读关闭（静态 1200.000）"
[automation] 工程泳道 track-1-automation-pan-lane      -> label="贝斯 · 声相 自动化 -1.000 · 录制臂 写入"
[automation] filled_project 泳道 track-0-automation-volume-lane -> label="Lead · 音量 自动化 -6.0 dB · 录制臂 触碰"
```

| 元素 ID | 角色 | 运行时 `accessible-label`（实测） | 它证明了什么 |
| :--- | :--- | :--- | :--- |
| `track-0-automation-volume-lane` | `image` | `鼓 · 音量 自动化 -3.2 dB · 录制臂 触碰` | 单位（`dB`）+ **模型入口在 tick 0 的值**（也是 `FADER_DB_LABELS[0]`）+ 写模式 |
| `track-0-automation-device-0-0-lane` | `image` | `鼓 · cutoff 自动化 读关闭（静态 1200.000）` | **读关可区分** + 自适应值域 + "退回静态值"（模型文档 §2.1 的 `Ok(None)` 分支） |
| `track-1-automation-pan-lane` | `image` | `贝斯 · 声相 自动化 -1.000 · 录制臂 写入` | 另一条轨道、双极单位、另一种写模式 |
| `track-0-automation-volume-lane`（`filled_project`） | `image` | `Lead · 音量 自动化 -6.0 dB · 录制臂 触碰` | 换工程 ⇒ 换标签（不是演示数据） |

- 数值来源：`automation_value_at(target, cursor_tick)`；静态投影的 `cursor_tick` 是
  `AUTOMATION_CURSOR_TICK = 0`（走带位置属会话运行态 `[MODEL-ISO-001]`，本线不发明；
  需要真实播放头时走 `ViewState::from_project_with_zoom_and_cursor`）。
- **未登记 0 条**：运行时树里的每一个元素都在注册表里 —— 包括本线新增的泳道
  （判据 ⑦ 的运行时侧因此是实测的，不是推断的）。

### 4.2 曲线在画面上的区域（**运行时 `bounds` 实测** + 像素验证）

`target/ui-test-port/app-runtime-control-tree.json` 里这些元素的真实包围盒（1920×1080 窗口）：

| 元素 | `bounds`（绝对像素） | 与投影的关系 |
| :--- | :--- | :--- |
| `workspace-arrangement-canvas` | `x=240 y=48 w=1400 h=583` | 工作区原点（左栏 240 + 顶栏 48） |
| `track-0-header` | `x=240 y=90 w=168 h=54` | 车道顶沿 = 画布 y **42** ⇒ 常量 `TRACK_LANE_TOP_PX = 42.0` 实测吻合 |
| `track-0-automation-volume-lane`（演示） | `x=408 y=92 w=1232 h=26` | 画布 y **44**、高 **26** = `(56 − 2×2) / 2`（该轨道两条带） |
| `track-0-automation-device-0-0-lane` | `x=408 y=118 w=1232 h=26` | 画布 y **70** = 第二条带（键序等分） |
| `track-1-automation-pan-lane` | `x=408 y=148 w=1232 h=52` | 画布 y **100**、高 **52** = `56 − 2×2`（该轨道一条带） |

⇒ 投影算出的 `band_y` / `band_height`（§2.2 的公式，本机判据逐位断言）与**真实渲染出来的
包围盒逐像素吻合**；x 恒为 `240 + 168 = 408`（时间轴偏移 168px，与 clips / notes 同款）。

**像素验证**（`app-model-driven-filled-project-1920x1080.png`，泳道 `(408,92,1232,52)`）：
描边色 `Tokens.gold-bright`（`#f7e6b0`）在带内共 **64** 列有像素、x 覆盖 `1 … 125`
（= 画布相对 `0 … 128`，即 `tick_to_px(0..3840)`）；把每列的平均 y 与"**模型值 → y 映射**"
逐点比较：

```text
 x   tick   实测y  模型y   模型值(dB)
32    960   12.00  12.32   -5.062
64   1920   11.00  10.83   -3.000
80   2400   10.00  10.04   -1.898
112  3360    9.00   8.85   -0.258
最大偏差 = 0.72 px（52px 带高，实测 64 列样本）
```

- 0.72px 的偏差与 1.5px 描边的抗锯齿同量级 ⇒ **曲线画在投影算出的坐标上**。
- **诚实边界**：这一条**不能**证明"值来自模型入口而不是线性插值" —— 对这条泳道
  （`SCurve`、值域 `[-60, 12]`、跨度 3840 tick），`SCurve` 与线性插值的**像素**差最大只有
  `6 dB × max|u(t) − t| / 72 dB × 52 px ≈ 0.42 px`，**亚像素、区分不了**。
  那条结论由**数值判据 ⑧**承担（`SCurve` 四分之一点 `u = 0.15625` vs 线性 `0.25`，
  在自适应量程 `[0, 1]` 下相差 ≈ 4.9px）—— 与 ADR-0001 **D23**"SSIM / 像素只对足够大的
  差异敏感"是同一个口径。
- 截图（7 张 PNG）与控件树 JSON 都落 `target/ui-test-port/`（`yeban_ui_test_port::artifact_dir()`），
  **不入库**；本机额外裁了一张曲线局部图 `target/ui-test-port/automation-curve-crop-filled.png`
  （同样是构建产物，仅供人眼复核）。

### 4.3 CI 侧新增的断言（Tier-1，见 §5.3）

| 断言 | 它排除什么 |
| :--- | :--- |
| 泳道元素数 == 工程泳道数（默认视图 **3** / `filled_project` **1**） | 排除了"只画了第一条"或"注册了但没进树" |
| 元素 `label` == 投影 `label`（逐字） | 排除了"界面自己拼了另一份文本" |
| 标签里的数值 == **判据自己调** `automation_value_at` 的返回值 | 排除了"投影里另写了一份插值"（判据不读投影的字段） |
| 读关的泳道标签含 `读关闭`、录制臂含 `触碰` / `写入` | 排除了"读写状态只画了颜色、MCP 读不到" |
| 同一活窗口上换工程 ⇒ 泳道数 1 ↔ 3、ID 集合不同 | 排除了"泳道是界面常量" |

---

## 5. 判据与注入记录

### 5.1 本机**真跑**的判据（`rustc --edition 2024 --test -D warnings`，零 Slint）

仓库之外 `/Users/crow/work/music/.app-automation-ui-harness/`，`lib.rs` 用 `#[path]` 指向
仓库**原件** `crates/yeban-app/src/{bridge,automation,scene,elements}.rs`（不是复制品）：

```bash
WT=/Users/crow/work/music/yeban/.worktrees/app-automation-ui
H=/Users/crow/work/music/.app-automation-ui-harness
source "$WT/scripts/dev/local-env.sh"
bash "$WT/scripts/dev/cargo-local.sh" build -p yeban-model --locked      # 5.63s
DEPS="$WT/target/debug/deps"
# `-C debug-assertions=on -C overflow-checks=on`：与 cargo 的 dev profile 对齐。
# **这一条不是装饰**：本线的一个真缺陷（`span × step` 溢出 ⇒ panic）只在开着
# overflow-checks 时才复现 —— 裸 `rustc` 默认**关**，会让那条判据静默通过（见 §5.2 注入 D）。
CARGO_MANIFEST_DIR="$WT/crates/yeban-app" rustc --edition 2024 --test -D warnings \
  -C debug-assertions=on -C overflow-checks=on \
  --crate-name yeban_app_local -L dependency="$DEPS" \
  --extern yeban_model="$DEPS/libyeban_model-610f97ad4a2723a8.rlib" "$H/lib.rs" -o "$H/run"
"$H/run"          # test result: ok. 52 passed; 0 failed（其中 automation:: 9 条）
```

| # | 判据 | 任务书要求 | 实测 |
| :-- | :--- | :--- | :--- |
| ① | `automation::tests::lane_count_follows_the_project` | ① 泳道元素数 == 工程里的泳道数 | `filled_project` 1 条 / 演示 3 条 == 两个工程 `automation_lanes` 的条目总数；元素 ID 唯一、格式良好、由 `(轨道序号, 目标键)` 唯一决定；空工程 0 条 |
| ② | `automation::tests::vertices_follow_the_points_in_tick_order` | ② 顶点数与采样点数一致且 x 单调（tick 有序） | `points.len() == 模型条目数`、tick 非递减；`samples` tick **严格**递增；每个采样点 tick 都在折线上；`x == tick_to_px(tick)` 逐位；**同 tick 两点** ⇒ `points` 两个顶点（同 x）、`samples` 一个顶点（胜者 == 入口） |
| ③ | `automation::tests::value_to_pixel_mapping_is_exact_and_adapts_when_the_domain_is_unknown` | ③ 纵轴映射正确（两个已知点 → y 的像素关系，含自适应） | `[-60,12]×52px`：`-60 → y 52`、`12 → y 0`、`-24 → y 26`、`-42 → y 39`；值域外**只钳像素不钳值**；退化区间 ⇒ 带正中且 `NaN` 不扩散；**自适应**：`DeviceParam`（模型里唯一没有固有值域的目标）⇒ `[200.0, 200.0]`（单点）/ `[0.0, 1.0]`，轴文本带「自适应」；显式取值域**不**被判为自适应 |
| ④ | `automation::tests::empty_single_point_and_absurd_lanes_behave_explicitly` | ④ 空泳道/单点泳道不 panic 且行为明确 | 空泳道：0 顶点、`commands == ""`、`value_at_cursor == None`、标签 `无采样点`；单点 `(1920, -7.5)`：2 个折线顶点（`0` 起保持 + 采样点）、处处 `-7.5`；**越界 tick ⇒ `Err(PixelOverflow)`**（不饱和/不回绕/不 panic）；`tpp = 0 ⇒ Err(ZeroTicksPerPixel)`；**荒谬跨度**（`tpp = 2^62` + 采样点 `0` 与 `2^63`）⇒ `Ok`、`samples` 恰好 `[0, 2^63÷8, 2^63]`（溢出的 `step` 被显式跳过）、每个顶点仍来自求值入口 |
| ⑤ | `automation::tests::read_disabled_lanes_are_distinguishable` | ⑤ `read_enabled=false` 的泳道在控件树里可区分 | 标签含 `读关闭`、角标含 `读关`、`value_at_cursor == None`（**界面没有自己判断读开关**：入口同样返回 `None`）、写模式 `Off` 时无录制臂角标；读开的泳道标签/角标里不得出现任何读关标记；`Touch` ⇒ `录制臂 触碰` 进标签 |
| ⑥ | `automation::tests::switching_the_project_changes_the_lanes` | ⑥ 换工程 ⇒ 泳道随之变化 | 演示 3 条 / `filled_project` 1 条，ID 集合不同；`filled` 恰好 `["track-0-automation-volume-lane"]`；空工程 0 条 |
| ⑦ | `automation::tests::registry_carries_one_element_per_lane` | ⑦ 注册表与控件树双向一致（未登记 0） | 注册表里 `-automation-…-lane` 条目数 == 泳道数；每条的角色 `image`、`component` 正确、标签与投影**逐字相等**、**不是**动态遮罩区；全部属于 `MODEL_DRIVEN_FAMILIES`（负向断言因此覆盖它们） |
| ⑦b | `elements::tests::slint_accessible_ids_and_registry_cover_each_other`（既有，本线保持绿） | ⑦ 的**文本层**一半 | `.slint` 的 `["track-", "-automation-", "-lane"]` 模板与注册表条目**互相覆盖** |
| ⑧ | `automation::tests::drawn_values_come_from_the_model_evaluation_entry` | ⑧ 一个已知 tick 的曲线值 == `automation_value_at` 的返回值 | 每个 `points` tick 的胜者 == `lane.value_at`；每个 `samples` 顶点 == `automation_value_at`（逐位）；`tick = 1920` ⇒ `-24.0`（== 入口）；**`SCurve` 的四分之一点** `250 ⇒ 0.15625` 且与线性插值 `0.25` 相差 `> 0.05`；自适应量程下 `y == (1 − 0.15625) × 带高`；`path_commands` 的顶点数与坐标可逐位复原（`{:.2}` 容差 0.0051） |
| ⑨ | `automation::tests::lane_element_ids_match_the_slint_template` | （判据的载体） | `.slint` 非注释代码里必须含三段字面量组合、`42px + 56px * track_index`、`commands: root.automation-path-commands[lane_index]`、`viewbox-width: parent.width / 1px`、`fit: ImageFill.fill` |

### 5.2 注入 ▸ 变红 ▸ 还原（4 次，全部本机真跑）

变异只作用在 harness 里的**副本**（`$H/mut/<name>/`），仓库文件全程未改动
（注入前后 `git status --short` 都只有本线的文件）；**四份副本都按当前源重建**
（不是早期版本的残留），并且编译时开着 `-C overflow-checks=on`。

| # | 注入（任务书建议的形态） | 变红的判据（实测全表） | 结果 |
| :-- | :--- | :--- | :--- |
| A | 投影里**另写一份线性插值**（仍然调模型入口，但丢弃结果、忽略 `curve`） | `drawn_values_come_from_the_model_evaluation_entry`（判据 ⑧）、`empty_single_point_and_absurd_lanes_behave_explicitly`（判据 ④ 也断言了入口一致性） | `50 passed; 2 failed` |
| B | `x` 换算改成**索引布局**（`40.0 × index`，第二套换算） | `vertices_follow_the_points_in_tick_order`（判据 ②）、`empty_single_point_and_absurd_lanes_behave_explicitly`（判据 ④ 断言了 `x == tick_to_px(tick)`） | `50 passed; 2 failed` |
| C | `effective_domain()` **写死**成音量量程（禁用自适应） | `value_to_pixel_mapping_is_exact_and_adapts_when_the_domain_is_unknown`（判据 ③）、`drawn_values_…`（⑧ 的自适应 `y`）、`empty_single_point_…`（④ 的自适应量程） | `49 passed; 3 failed` |
| **D** | **`span × step` 退回裸乘法**（去掉 `checked_mul`） | `empty_single_point_and_absurd_lanes_behave_explicitly`（判据 ④）—— 以 **`attempt to multiply with overflow` panic** 变红 | `51 passed; 1 failed` |

> 三条注入打在"求值""换算""量程"三条口径上；**第四条（D）打的是真缺陷**：
> 注入 D 的第一版**没有变红**，因为判据 ④ 当时把荒谬采样点放在 `u64::MAX`,
> 于是 `point_vertices` 的 `tick_to_px` 先返回 `Err`、`sample_vertices`
> **根本没被调用** —— 那条断言看着在测"不 panic"，其实什么都没测。
> 把采样点改成 `tpp = 2^62` 下的 `[0, 2^63]`（两个端点都还在 u32 像素内 ⇒ 细采样真的执行）
> 之后，注入 D 立刻以溢出 panic 变红。**"判据要能失败"这条又一次抓住了作者自己**。
> 还原后 52 条全绿。

### 5.2b 真红记录（**不是注入**：CI 抓到了作者写反的断言）

`run 37249203359`（tip `1609d58`）的 `rust (yeban-app)` 只红了一条，原文：

```text
thread 'criteria::project_projection_reaches_the_control_tree_and_the_pixels' panicked
  at crates/yeban-app/tests/../src/test_port_adapter.rs:1700:5:
读关的设备参数泳道只属于演示工程，不得出现在 filled_project 的树里
test result: FAILED. 8 passed; 1 failed
```

**归属（三种可能里选哪条，及依据）**：

| 可能 | 裁决 | 依据 |
| :--- | :--- | :--- |
| ① 投影真的违反了不变量（把设备参数泳道投进了 `filled_project`） | **否** | 同一条判据在更前面已经断言过 `lanes_in(&runtime) == 1`（`filled` 只有 `track-0-automation-volume-lane`）并通过了；纯 Rust 判据 `switching_the_project_changes_the_lanes` 也断言 `filled` 的泳道 ID 恰好是那一条。投影是对的。 |
| ② 那条假设过时了（`filled_project` 现在**本来就有**一条读关的设备参数泳道） | **否** | `yeban_model::samples::filled_project()` 里只有 1 条 `TrackVolume` 泳道（`read_enabled = true`、`write_mode = Touch`），没有设备参数泳道；假设没有过时。 |
| ③ **第三条路：断言本身把两棵树写反了** | **是** | 那一行断言的是 `!demo_runtime.contains("track-0-automation-device-0-0-lane")`，而提示文本写的是"不得出现在 `filled_project` 的树里" —— **变量是演示树、语义是工程 A 的树**。演示工程（`demo_project` 的轨道 0 有 device ⇒ 有读关的设备参数泳道）**本来就应该有它**，所以这条断言从写下的那一刻起就必红。 |

**修法**（不是删断言，而是变成**更强**的两条）：两个方向都断言 ——
演示树**必须**含 `track-0-automation-device-0-0-lane`、`filled_project` 的树**必须不**含它；
并在代码里留下这次教训的注释。它们由同一组实测数字支撑（§4.1：演示树里该元素的
标签是 `鼓 · cutoff 自动化 读关闭（静态 1200.000）`，而 `filled` 的树里只有一条音量泳道）。

> 与 `app-binding` §4.2 的"额外证据"同款：**判据真的能失败**，这次是它抓住了作者自己。
> 本线随后把 CI 的 `test` 腿在本机（真实 Slint 平台 + 暖 target）整跑了一遍：
> **154 passed / 0 failed**（§6.1 行 5），因此这一次的修复不是"猜着修的"。

### 5.3 交给 CI 的（本机**没有**验证）

- `.slint` 的**真实编译产物与运行时控件树内容**：本机只做了独立类型检查（§6.2），
  没有 `cargo test`（需要 Slint 完整 codegen）；
- Tier-1 的**像素**：曲线真的被光栅化（颜色数 / 非黑占比 / 指纹变化 / SSIM）；
- 运行时树里泳道元素的 `bounds`（本机只断言了投影算出的 `band_y` / 面积非零）；
- `cargo clippy -p yeban-app --all-targets -D warnings` 在 **CI 的干净 target** 上的结论
  （本机是复用主仓暖 target 的等价检查，§6.2）。

---

## 6. 本机真跑 vs CI（严格区分）

### 6.1 本机真跑（三条，都是真跑，不是"应该能跑"）

| # | 手段 | 命令 | 实测 |
| :-- | :--- | :--- | :--- |
| 1 | **零 Slint 纯逻辑判据** | `rustc --edition 2024 --test -D warnings -C debug-assertions=on -C overflow-checks=on`（§5.1） | `52 passed; 0 failed`（其中 9 条是本线新增）；**开着 overflow-checks** 才复现得了 §5.2 的注入 D |
| 2 | **`.slint` 独立类型检查**（本线新手法，§6.2） | `$H/slintcheck/check ui/app.slint` | `SLINT OK`（13 个 `.slint` 全在该图里） |
| 3 | **clippy 全目标（含 `--all-targets`）** | `CARGO_TARGET_DIR=<主仓暖 target> cargo-local.sh clippy -p yeban-app --all-targets --locked -- -D warnings` | `Finished` 退出码 0（零告警）；期间 `build.rs` **真的**把 `ui/app.slint` 编译了一遍，`host.rs` 的 9 个新 setter 与生成绑定对上了 |
| 4 | 轻量门禁 | `bash scripts/gates/run-gates.sh light` | `门禁通过 (mode=light)`（fmt / 14 条守卫 / 文档 / 许可清单） |
| **5** | **真实平台整跑**（CI 的 `test` 腿等价命令，复用暖 target） | `CARGO_TARGET_DIR=<暖 target> cargo-local.sh test -p yeban-app --all-targets --locked` | **154 passed / 0 failed**：lib **119**（含本线 9 条）+ bin 0 + `cli_contract` 12 + `live_ui_mcp` 12 + `open_project_file` 2 + **`real_ui_tier1` 9**（Tier-1：软件光栅化 + 真实控件树内省；含本线新增的全部泳道断言） |

### 6.2 本机新添的两种机械证据（值得 hoist）

**(a) `.slint` 独立类型检查器（秒级、零重编译）**：把主仓已编好的
`libslint_build-*.rlib` 直接用 `rustc --extern` 链到一个小程序上，调
`slint_build::compile_with_config(path, config.with_debug_info(true))`：

```bash
LIB=$(ls /Users/crow/work/music/yeban/target/debug/deps/libslint_build-*.rlib | head -1)
rustc --edition 2024 --crate-name slintcheck check.rs -L dependency=<主仓 deps> \
  --extern slint_build=$LIB -o check
CARGO_MANIFEST_DIR=$WT/crates/yeban-app OUT_DIR=<临时目录> ./check ui/app.slint   # => SLINT OK
```

它给出的是**上游编译器**的解析 + 类型检查结论（含 `for` 语法、属性类型、`Path`
的 `viewbox` / `fit` / `commands` 绑定、数组属性转发），因此"改了 `.slint` 却要在 CI
才发现语法错"（`app-binding` 第 1 轮就是这样丢了一轮）在本机就能拦住。
**本线用它验证了 4 个候选写法**（`background: transparent`、`length / length → float`、
`fit: ImageFill.fill`、`accessible-id` 的三段拼接），全部先证再写。

**(b) clippy 复用**暖 target：`CARGO_TARGET_DIR` 指向主仓的 `target/` 时，registry 依赖
（Slint 全家桶）按源码身份复用、只有本工作树的成员 crate 重编 ⇒ 一条
`clippy -p yeban-app --all-targets` 只用**分钟级**就把 `src/**` 与**全部测试目标**
（含 `tests/real_ui_tier1.rs` → `#[path]` 引入的 `test_port_adapter.rs`）对着**真实的
Slint 生成绑定**类型检查了一遍。本线靠它一次抓出两处真错：
`!(span > 0.0)`（`clippy::neg_cmp_op_on_partial_ord`）与
`assert_eq!(entry, lane.value_at_cursor)`（`f32` vs `Option<f32>`，E0277）。

> **诚实边界**：`(b)` 的结论**不能**替代 CI 的 clippy（同一套 lint 配置 + 干净 target），
> 也**不**跑 `cargo test`；`(a)` **不**做 codegen、不跑光栅化。像素与运行时树内容只有 CI 能判。

---

## 7. CI 判决

> 未读到的判决一律记 `pending`。判决由 `scripts/dev/ci-verdict.sh line/app-automation-ui` 读回。

| 轮 | commit | run | 结论 | 说明 |
| :-- | :--- | :--- | :--- | :--- |
| 1（代码） | `6713f92` | [37248336525](https://github.com/gradetwo/yeban/actions/runs/37248336525) | **被第 5 轮取代** | `checks` / `deny` / `lockfile` 绿，`plan` 与 `rust (yeban-app)` 已起；本机随后修了一个真缺陷（`span × step` 溢出）与一条**假判据**。 |
| 2（判据加固） | `d9d65b5` | — | **被第 5 轮取代** | 补"同 tick 多点"判据；同轮发现 `checked_mul` 缺口。 |
| 3（代码） | `1609d58` | [37249203359](https://github.com/gradetwo/yeban/actions/runs/37249203359) | **failure（`rust (yeban-app)`：Tier-1 8 绿 / 1 红）** | `clippy -D warnings` 绿、lib `119 passed`；红的是**我写反的一条断言**（§5.2b，`test_port_adapter.rs:1700`）。CI 一次就定位到行号与消息。 |
| 4（修复，**代码 tip**） | `1fbc99f` | 见 §7.1 | — | 把那条断言改成"两个方向都断言"（更强），并在本机真实平台上把整条 `test` 腿跑绿（154/154）。 |
| 5（docs-only） | 本文件所在提交 | 见 §7.1 | — | 只改本文件 ⇒ `plan` 判"受影响集合为空"，`rust` 腿按设计**跳过**；这一轮**没有**代码读数（与 `app-binding` §6.3 / `app-completion` §5.0 同款结论）。 |

### 7.1 读数（待填）

> 本节在本文件提交时按实际读回的判决填写；**未读回前一律 `pending`**。

---

## 8. 未实现项（如实登记，**不是**静默降级）

| # | 未实现 | 现状 | 归属 / 阻塞 |
| :-- | :--- | :--- | :--- |
| 1 | **写入方向**（拖动采样点 / 铅笔 / 橡皮 / 录制落点） | 只有"模型 → 视图"一个方向；没有任何 UI → 模型的回写入口 | `yeban-model::Op`（`SetAutomationPoint` 等）+ 指针捕获状态机（规范 §6.1），`[UI-NOTE-003]` |
| 2 | **拖动编辑与吸附** | 曲线是只读的；采样点没有独立的可拖拽元素（只有曲线元素本身） | 同上；960 PPQ 吸附在模型侧 |
| 3 | **多泳道叠加的"优先级 / 汇总"语义** | 一条轨道多泳道时按**键序等分**成多条带（各自独立画），**没有**"多个目标同时自动化一个参数"的合并语义 | 属模型层（`MacroMapping` / `ARCH-DSP-001` 的平滑），本线不发明 |
| 4 | **末点之后的保持段** | 只画到最后一个采样点；模型口径是"末点之后保持"，但"画到哪里"属于视口 / 工程长度 | `[UI-NOTE-001]` 视口裁剪 |
| 5 | **主总线轨道上的泳道** | **有意跳过**（编排视图没有主总线的车道行；主总线是调音台的通道条）。判据没有断言这一条 | 需要主总线车道行（`app-mixer` 那条线）或独立的主总线自动化视图 |
| 6 | **一条轨道 >52 条泳道** | 带高会 <1px ⇒ 元素可能被裁剪语义过滤掉（"登记了但查不到"） | `RSK-31`；需要泳道折叠 / 滚动（`[UI-NOTE-001]`） |
| 7 | **纵轴的刻度线 / 数值标签** | 只有一条轴**文本**（`dB [-60.0, 12.0]`），没有刻度线 | 视觉层；需要 `[UI-A11Y-004]` 的对比度预算 |
| 8 | **设备参数的物理单位** | 轴标签对 `DeviceParam` 只给量程（`[200.0, 4000.0]`），没有 `Hz` —— 单位只来自 `lane.unit()`（模型里 `ParameterValue::unit` 是自由文本、不参与 `AutomationUnit`） | 模型层：`AutomationUnit` 要不要带上设备参数的文本单位，是一次 ADR 级取舍 |
| 9 | **真实播放头** | 静态投影的求值 tick 是常量 `0`；`from_project_with_zoom_and_cursor` 已给出接线点，但**没有**接事件循环 | 走带位置属会话运行态（`[MODEL-ISO-001]` 第二层）+ `yeban-engine` |
| 10 | **曲线的高亮 / 选中 / 悬停提示** | 无 | 视觉层 + 会话运行态（选中项） |
| 11 | **`clippy` 在干净 target 上的读数** | 本机复用主仓暖 target（§6.2b） | CI 的 `rust (yeban-app)` 腿 |

---

## 9. needs（交给集成者 / 需要人类裁决）

1. **`docs/ledger/model-automation-notes.md` §5 的"界面"那一行可以标为已接入**：
   `unit()` / `effective_domain()` / `points_in_tick_order()` / `read_enabled` / `write_mode`
   五个接入点**全部**用上了，且本条备注的 §2.3 / §2.4 给了"没有第二份实现"的机械证据。
2. **`app-binding-notes.md` §8 未实现项 #5 / `app-completion-notes.md` §6 未实现项 #7
   （"自动化曲线 / 宏 / 设备链"）可以关掉"自动化曲线"这半句**：泳道已由工程驱动；
   `macros` / `devices` 仍不进视图（`device_rack.slint` 用 `scene::DEVICE_NAMES` 常量）。
3. **`docs/DEVELOPMENT_LEDGER.md`**：建议记一笔"界面侧自动化曲线已落地（投影 + 曲线 +
   控件树可读）"，并把 §6.2 的两种本机验证手法登记（见 needs 5）。
4. **`[UI-MCP-003]` 的分平台 Golden**：本线**没有**提交基准图（需要人类批准）。
   曲线的像素回归目前靠"指纹随工程变化 + 非黑占比 + 颜色数"这一族断言，不是 Golden 比对。
5. **hoist → `docs/DEV_WORKFLOW.md`**：本机验证"含 Slint 的 crate"的**第 4 / 第 5 种**手法
   （`§6.2`）：① 直接链 `libslint_build` 得到一个秒级 `.slint` 类型检查器；
   ② `CARGO_TARGET_DIR` 指向暖 target 跑 `clippy --all-targets`。两条都**不**跑 codegen、
   都**不**跑光栅化，因此都**不能**替代 CI 的 `cargo test` —— 但能在推送前拦住
   "语法 / 类型 / lint"这一整类浪费轮次的红点。本线实测抓到 **3 处真错**
   （`!(span > 0.0)` 的 clippy 违规、`f32` vs `Option<f32>` 的类型错、
   `span × step` 的溢出 panic）。
   **并与既有的 `rustc --test` 手法合并一条必要修正**：本机的 `rustc` 直接调用
   **默认关掉** `debug-assertions` / `overflow-checks`，而 cargo 的 dev profile **开着** ——
   因此"本机真跑全绿"与"CI 真跑"在**溢出与 `debug_assert!`** 这两类上口径不同。
   本线的真缺陷（`attempt to multiply with overflow`）正是靠补齐
   `-C debug-assertions=on -C overflow-checks=on` 才复现出来的。建议把这条写进方法论：
   **任何用裸 `rustc` 跑的判据都要显式补这两个 flag**。
6. **hoist → ADR-0001**：可复用约束两条 —— ① "曲线类图元必须显式给 `viewbox` + `fit`
   才谈得上"坐标 == 投影的像素""（不写就变成 scale-to-fit，纵轴量程失效）；
   ② "投影给界面的几何必须是**非零面积**的"（0 面积元素会被上游裁剪语义过滤掉，
   语义 ID 寻址退化成概率事件）。
7. **`AutomationUnit::Native` 要不要带设备参数的文本单位**（§8 #8）：建议一次 ADR 级裁决。

---

## 10. 修改文件清单（本线，8 个文件）

```text
crates/yeban-app/src/automation.rs                  | 1467 ++++++++++++++  (新建)
crates/yeban-app/src/bridge.rs                      |  304 +++-
crates/yeban-app/src/elements.rs                    |   19 +
crates/yeban-app/src/host.rs                        |   11 +
crates/yeban-app/src/lib.rs                         |    1 +
crates/yeban-app/src/test_port_adapter.rs           |  200 ++-
crates/yeban-app/ui/app.slint                       |   26 +
crates/yeban-app/ui/workspace/arrangement_view.slint|  101 ++
docs/ledger/app-automation-ui-notes.md              | 本文件
```

`Cargo.toml` / `Cargo.lock` **逐字节未变**（零新增依赖）；根级共享文件、其它 `crates/**`、
`schemas/**`、法务文件全部未动。
