# ADR-0073 — report-3.md 第四轮实测 16 项问题的 ROI 复核裁决（Proposed）

- **状态**: **Proposed（待人类批准）** — 依 `AGENTS.md §5.7`，本 ADR 只登记
  「规范缺口 / 观测结论冲突」的复核证据与建议裁决，**不擅自改写 Normative 文档、
  不臆造引擎层修复**。
- **决定者**: 待人类责任人裁定
- **关联**: `~/work/qoder/report-3.md`（2026-11-15，基线 `764c65096`）、
  `~/work/qoder/细节实测-3ff6914.md`、`~/work/qoder/控件逐一体检-3ff6914.md`；
  规范 `AGENTS.md §5.5/§5.7/§6.3/§6.4`；代码事实源 `crates/yeban-app/**`、
  `crates/yeban-model/src/ops.rs`
- **日期**: 2026-11-15

## 背景

`report-3.md` 是盒上 `yanshi-5` 的**只读**第四轮深度实测，基线 `764c65096`。
它列出 §7 共 16 项「未修 / 现存」问题。本轮用两类**当前 tip 直接证据**逐项复核：

1. **当前代码复核**：工作树已 rebase 到 `origin/main`（tip `db8873f7`）并叠加
   多轮 UI 修复。`git diff 764c6509..HEAD` 显示 19 个改动文件全部集中在
   `crates/yeban-app/**` 的 UI 文案 / 展示层与新增判据，`host.rs` 仅 3 行
   UI 接线（Tokens 平台门控、F3/F7/F8 去重、`file-save-as`）——
   **引擎 / 导出 / 卷帘 / 控制室 / 设备布局 / 播放头 / 人性化逻辑一行未碰**。
   故 §3 表格对 N1/N4/N11/N12/Q1/Q2/Q5 的「未修」判定对当前 tip 仍成立。
2. **真实交付工程复核**（用户 2026-11-15 提供，零编译解析 `.yeban` ZIP 容器）：
   - `夜半霓虹_v6_电影质感版.zip` → `夜半霓虹.yeban` 108 MB：
     19 轨 / 17 片段 / 1494 音符 / 53 内嵌样本（相对路径）/ 11 段落；
     `history.dag` 163 commit、3742 op、21 个 `Op` variant。
   - `霞飞路夜雨_交付包.zip` → `霞飞路夜雨.yeban` 209 MB：
     44 轨 / 40 片段 / 6228 音符 / 8 段落 / `history.dag` 空；
     工程说明自陈 SFZ 为**本机绝对路径**、换机需 `assets/resolved`（6.3 GB）重定位。
   - 两工程音符 id **重复数 = 0** ⇒ §7 #15 的 zimmer 重复 id 是**特定工程数据**，
     不是普遍缺陷，且不在本仓库代码内。

## 复核结论：16 项分三类

### C1 — 探针走回调层造成的观测假象（**建议裁决：不修，改钉判据**）

`report-3.md` 探针用 `invoke_*` / 直接回调层驱动，绕过 `.slint` 的
`TouchArea` / 真实按钮参数 / 真实作用域，于是观测到与真实用户不同的行为。
逐条当前代码证据：

| 项 | 报告结论 | 当前代码事实 | 裁决 |
|---|---|---|---|
| #2 N4 八度移 1 半音 | 每步只移 1 半音，需连点 12 次 | `ui/console/piano_roll.slint:254/279` 按钮**已传 `±12`**；`host.rs:19906` 的 `current + delta` 本就按半音数相加；判据 `aether_piano_roll.rs:276/792` 亦传 `±12`。探针传 `±1` 才得 1 半音 | 不修 |
| #3 N11 create 240 / 铅笔 960 | 两个默认时长不一致 | 铅笔写模型 `duration_ticks = 960`（PPQ/4×4），被 `aether_piano_roll.rs:1206` **逐字钉死**；报告里的「240」是 `host.rs:17655` 演示网格的 `step_span: 2` / `ROLL_SNAP_GRID_TICKS = 240`（1/16 吸附格），**不是音符时长**。step-grid 与 tick 口径混淆 | 不修 |
| #9 控制室 toast 恒报全关 | 点 DIM/SIDE 后 toast 全关 | `control_room.slint` 的 TouchArea 用 `<=>` **自行翻转** `is-dim/is-mono/is-side`，宿主 `on_control_room_toggle_*` 只读状态生成 toast；探针走回调层绕过翻转 ⇒ 状态恒 false。报告自注「开关本体在无头环境不可听辨」 | 不修（双写者设计是真实交互所需） |
| #14 人性化缺省零变化 | 缺省 apply 后音符零变化 | `host.rs:9177` `seed_humanize_fields` **已填非零缺省**（10/10/240/100）；`host.rs:9305` apply **真调 `commit_ops` + 重投影**。零变化是连带 #1 的 `note_ulids = 0` 作用域 / 探针未选中含音符片段 | 不修 |

依 `AGENTS.md §6.4`（「参考文档的警告是证据」「未证的语义要登记，不要补齐」）
与 §6.1（先说指标再下结论）：这 4 项**改了反而会破坏真实交互与既有判据**，
本轮不碰。建议后续以「真实按钮参数 / 真实作用域」的活窗口判据补钉，而非改产品代码。

### C2 — 已闭合 / 已修 / 仓库外数据（**建议裁决：无需动作**）

| 项 | 结论 | 证据 |
|---|---|---|
| #16 测试覆盖缺口（14 目标盒上编不过、尖端无 CI run） | **已由 CI 闭合** | `43744117695` success：`rust (yeban-app)` 日志里 `cli_contract`/`export_wav_*`/`lyrics_export`/`midi_round_trip`/`migrate_groove`/`sample_editor`/`theme_selection`/`track_sampler_*` 全部 `Running tests/` 出现；414 次 Running、**0 FAILED / 0 编译错** |
| #10 toast 文案截断 | **已修** | `ui/modals.slint:411` `wrap: word-wrap`（`ui_toast_legibility.rs` 2 判据钉住）；#10 的「不清零」半项见 C3 |
| #12 设备机架 245px 窄列 | **当前代码已自适应** | `device_rack.slint:385` `first-rack-width: min(824px, master-rack-x - …)`、卡片 `width: 420px`、右侧 `first-side-*` 一列已铺开；报告 245px 是基线目测 |
| #15 zimmer 重复音符 id | **工程数据、非分支代码** | 两个真实交付工程重复 id 均为 0 |

### C3 — 真实但仍需真机 / 人类裁决（**建议裁决：登记，不在本轮臆造修复**）

| 项 | 性质 | 为何不本轮修 |
|---|---|---|
| #1 N1 索引式音符作用域（首要） | 引擎逻辑：`host.rs:18682` `aether_note_ref` 选中片段分支仍回退 `live_view.notes.get(index)`；真实工程 `note_ulids` 恒 0 | 需真实工程 + 活窗口复现作用域填充链路并定「按选中片段寻址」语义；本机禁重依赖编译（§5.2），CI 无 108 MB 资产 |
| #4 N12 默认导出过 0 dBFS | 报告自标「主干共有」 | 需音频真机 + 响度口径裁决（limiter 默认值 / 拦截策略） |
| #5 Q5 导出静音 / 样本缺失不诚实 | 主干共有；霞飞路工程说明自陈 SFZ 绝对路径换机缺失 | 需 6.3 GB `assets/resolved` 真机 + 「跳过该轨 vs 整体失败」策略裁决 |
| #6 Q1 winter 保存静音不保留 | 环境性（SFZ 相对路径缺失） | 同上 |
| #7 Q2 旗舰接线 32/41 | 缺 9 个 `wire_*`（7 个文件对话框族 + 2 个硬件 MIDI 腿） | 无头文件对话框不可复刻，需真机 / 权限裁决 |
| #8 无障碍树可达性缺失 | 结构性：206 静态 id 仅 7–21 个在活树 | 需活窗口 + UI 测试端口重构，属独立能力切片 |
| #10 半项 toast 不清零 | `update_toast` 无自动隐藏；但 `aether_real_project_live.rs:572` **要求 toast 持久** | 自动清零会与现有判据冲突，需产品裁决「超时时长 / 是否自动消失」 |
| #11 命令面板不在可达树 | 与 #8 同源 | 报告自注未以截图确认视觉打开态 |
| #12 半项设备 ID 文本截断 | `device_name` 用 `overflow: elide`，但 `accessible-label` 已含全名 | 视觉 elide 是有意设计；改 wrap 会挤压 44px 卡片三行文本 |
| #13 卷帘播放头无效果 | 报告自注「边界宽仅 1px，不排除指针命中口径」 | 需真机确认命中口径，非确定缺陷 |

## 决定（本轮实际落地）

### D1 — 新增 `crates/yeban-model/tests/real_project_ops_variant_compat.rs`

零编译 smoke 用 11-06 的旧 `yeban-mcp` 打开 v6 工程时命中**真实事故**：
`history.dag` 的 `SetTrackPreset` 不被旧构建识别 ⇒ 整个工程打不开。当前
`ops.rs` 已支持，但**没有任何判据**钉住「真实交付件里出现过的 Ops variant
必须被当前代码声明并分派」。判据把两件真工程实测出的 **21 个 variant** 固化为
源码断言（枚举声明 + `kind_name` 分派 + `SetTrackPreset` 定点回归），
纯 `include_str!`、`rustc --test` 可直跑、CI `yeban-model` 腿真跑。
这是把「用真实工程验证」落成可回归门禁的**唯一确定性、零风险**修复。

### D2 — 本 ADR 登记 ROI 分层

按 `AGENTS.md §5.7` 把「报告结论 vs 当前代码 / 真实工程证据」的冲突留痕，
标 `Proposed` 交人类批准；C1/C2/C3 的裁决建议如上，**不擅自改写 Normative
文档、不臆造引擎层修复**。

## 建议的人类裁决点

1. C1 四项的「探针口径」是否确认接受为**非缺陷**（若否，需指定活窗口判据口径）。
2. C3 的 #1（N1 作用域）是否立项为独立能力切片，并授权用真实工程 + CI 验证。
3. #4/#5 的导出响度 / 样本缺失策略：默认是否加 limiter、缺失轨「跳过 vs 失败」。
4. #10 toast 是否改为「超时自动消失」，以及超时时长（会动 `aether_real_project_live.rs:572`）。

## 后果

- 正向：真实交付件的 Ops variant 兼容性从此有回归门禁；报告 16 项有了
  「假象 / 已闭合 / 待裁决」的权威分层，后续 Agent 不会按错误结论改坏代码。
- 负向 / 限制：C3 的引擎与行为层缺陷**本轮未修**；它们需要真机、真实工程
  资产注入 CI（>10 MB 红线，见 `AGENTS.md §2.9`）与人类语义裁决，不能靠
  本机源码扫描臆造。
