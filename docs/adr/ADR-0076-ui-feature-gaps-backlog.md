# ADR-0076 — UI 功能缺口立项：波形峰值 / BUS COMP / 剪贴板 / 退出 / 分轨导出 / 框选（Accepted-立项）

- **状态**: **已立项（人类 2026-11-16「这几个反馈的问题也检查和安排上」+ 授权 Agent
  自行立项推进）**。本 ADR 只做**现状取证 + 实现要点 + 依赖**登记；每项的落地按
  独立能力切片推进，⛔ 不把"标注示意"当已修。
- **决定者**: 人类授权立项，Agent 排期实施
- **关联**: `~/work/qoder/未修复问题清单-0f181bd.md`；`ADR-0073`（Q2 文件对话框族）、
  `ADR-0074`（P2-1 死代码）；`AGENTS.md §5.7/§6.4`
- **日期**: 2026-11-16

## 现状取证（2026-11-16 当前 tip 逐条 grep / 读码）

| # | 项 | 现状证据 |
|---|---|---|
| B1 | 波形真实峰值缺失 | `ui/arrangement.slint:1156` 注释「真实峰值（按 block 的离线分析缓存）尚未接入」；`:1183` accessible-label 仍写「峰值分析尚未接入，当前为 0dB 基准线占位」 |
| B2 | BUS COMP 仍「示意」 | `ui/left_panel.slint:859` `text: "BUS COMP · 示意"` |
| B3 | 复制 / 粘贴 / 剪切 = 0 | `ui/app.slint` 三词各 0 处；`host.rs` 无 `edit-copy`/`edit-paste`/`edit-cut` 命令臂 |
| B4 | 菜单无「退出」 | `ui/app.slint`「退出」0 处；仓库无 `quit_event_loop`/`process::exit` 先例 |
| B5 | 分轨导出仅 CLI | `ui/app.slint:763` 文案「请用 CLI」；`host.rs:18632` 只打开 WAV 面板 + toast |
| B6 | 多选框选未触及 | `host.rs:13952` 注释「空白处拖拽语义一位不改（仍是滚动；marquee 未裁决）⇒ ⛔ 不做框选」 |

## 已落地（本次）

**D1 — P2-1 死代码删除**：`wire_aether_callbacks` 里的 `ui.on_trigger_shortcut` 白名单
处理器被 `wire_aether_actions` 的后写覆盖，从未执行 ⇒ 删除。⛔ 行为零变更。
判据 `tests/dead_code_removed.rs` 钉住「全仓只赋值一次」。

## 实现要点与依赖（按可交付性排序）

### B1 波形真实峰值（中）
- 需：离线 peak 分析缓存（按 block 的 min/max）→ 投影属性（如 `clip_waveform_peaks`）→
  `.slint` 按峰值画折线。
- 依赖：分析在导入/落盘时做（或后台线程），缓存按 asset hash 存；**不可在绘制时算**。
- 验收：判据断言「音频片段有非空 peaks 数组且长度与片段宽度对应」+ 视觉回归。

### B2 BUS COMP 真实曲线（中）
- 需：压缩器参数（threshold/ratio/attack/release/makeup）投影到 left_panel，按传递函数
  画压缩曲线（与 EQ 的 `eq_curve_svg` 同路，新增 `comp_curve_svg`）。
- 依赖：总线设备参数可读（`DeviceDefinition.params` 已存在）；无参数时保持「示意」。
- 验收：参数变化 ⇒ 曲线变化（判据比对两条不同参数的 SVG 不相同）。

### B3 复制 / 粘贴 / 剪切（中-大）
- 需：剪贴板模型（`Clipboard { placements: Vec<...>, notes: Vec<...> }` 进程内）+
  三个命令臂 + 菜单项 + 快捷键（`Cmd/Ctrl+C/X/V`，需确认不与文本编辑冲突——
  文本焦点下应 PassThrough，解析表已有 TextInput 放行机制）。
- 依赖：`Op` 层已有 `AddClipPlacement`/`AddClip`/`AddNote` ⇒ 粘贴 = 批量提交这些 Op（一步撤销）。
- 验收：复制一个片段 → 粘贴到另一轨 ⇒ 片段数 +1 且内容相等；剪切 = 复制 + 删除（一步撤销）。

### B4 菜单「退出」（小-中）
- 需：确定关闭语义。Slint 1.18 可用 `slint::quit_event_loop()`（使 `ComponentHandle::run`
  返回）；或 `ui.window().hide()`。
- **依赖裁决**：未保存改动的确认弹窗（`unsaved-changes` 状态已有？需核）——
  ⛔ 不能静默丢数据。建议先做「有未保存改动 ⇒ 提示，否则直接退」。
- 验收：菜单项存在 + 命令臂调用退出 API（判据钉接线）；真机验证退出。

### B5 分轨导出 GUI（大）
- 同 `ADR-0073` Q2：需要文件对话框族（`rfd` 或 Slint 原生），且 `yeban_export_stems`
  在默认发行构建里由 `in-process-mcp` 特性门控（`ADR-0053`）。
- 建议：先确认发行构建是否含该能力；包含则接 GUI 入口，否则维持 CLI 文案（现状诚实）。

### B6 多选框选（大，需裁决）
- 代码明确「marquee 未裁决」：空白拖拽当前是**滚动**语义；改成框选会改变既有交互。
- 需产品裁决：空白拖拽 = 滚动 还是 框选（或修饰键区分）。
- 裁决后再做：框选矩形 + 命中测试 + 多选状态（`selected_ulids` 已是数组）。

## 后果

- 正向：6 项从"散落在报告/口述"变成**有证据、有实现要点、有验收口径**的立项清单；
  P2-1 死代码本次已清。
- 负向 / 限制：B1/B2/B3/B5/B6 **本轮未实现**（各自是独立能力切片）；B6 还卡在产品
  裁决；B5 卡在发行特性。⛔ 不把"仍是示意/请用 CLI"当作已修。
