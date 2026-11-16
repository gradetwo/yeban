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

## D2 — 顺带发现：golden 重录机制当前**不生效**（2026-11-16 实测）

`gates-manual.yml` 的 `goldens` 档跑 `YEBAN_WRITE_GOLDEN=1 cargo test --test real_ui_tier1`
并上传 `crates/yeban-app/tests/golden/linux/`。实测 run `43829721705`（success，
artifact `golden-linux`）：

- artifact 的 5 张 PNG 与仓库**哈希完全相同**（`SAME` ×5）；
- artifact 的 `MANIFEST.txt` 仍记录 **2026-10-07** 的旧 run `37695374534`；
⇒ **上传的是仓库 checkout 里的旧图**，测试没有写出新图。

根因：
1. `regenerate_golden_if_asked` 只在 `src/test_port_adapter.rs:159`（截图路径）被调用，
   而 `real_ui_tier1.rs` **没有任何截图调用** ⇒ 该测试不产生图；
2. `assert_matches_golden` 里 `GOLDEN_COMPARE_ENABLED = false`（2026-11-12 Wang 定、
   UI 重构期禁用）⇒ 即使图变了也不比对。

影响：本轮视觉改动（选中高亮 / 片段标签 / 对比度）**只有源码层判据**看守，
⛔ 不能声称"golden 已验证"。

建议：让 `real_ui_tier1` 真正调用截图 API（或改用带截图的入口）→ 复核 5 张基准 →
恢复 `GOLDEN_COMPARE_ENABLED = true`。在此之前，任何"golden 重录"都是空操作。

## 实现要点与依赖（按可交付性排序）

### B1 波形真实峰值（中-大，2026-11-16 细化依赖）
- 需：离线 peak 分析缓存（按 block 的 min/max）→ 投影属性（如 `clip_waveform_peaks`）→
  `.slint` 按峰值画折线。
- **依赖（本轮核验）**：`yeban-app` 的 `UndoPort`（本地会话）**没有资产池**——
  `project.assets` 只存 `AssetMetadata`（hash/路径/长度）、`embedded_samples` 是
  `路径→hash`；资产字节只在 `yeban-mcp::domain::Domain::asset(&hash)`（控制面）
  或 `.yeban` 容器的 `assets/<hash>`。⇒ B1 必须先有一条**容器/控制面资产读取路径**
  （或导入时把 peaks 写进 `AssetMetadata`），否则投影层拿不到样本。
- **具体 API（2026-11-16 核验）**：`yeban_app::open::open_project_file` 只返回
  `YebanProjectV1`（**丢弃 assets**）；要拿资产字节需走
  `yeban_model::container::read_project_container(path)` 的 `archive.assets`
  （`Vec<(AssetHash, Vec<u8>)>` / `BTreeMap`），或控制面 `Domain::asset(&hash)`。
  ⇒ B1 要给 GUI 打开路径补一条「读 assets」的入口（或直接复用 container 读法）。
- 建议：导入/落盘时算 peaks 并落进会话 CAS 附加元数据（避免每次投影解码）。
- 验收：判据断言「音频片段有非空 peaks 数组且长度与片段宽度对应」+ 视觉回归。
- **落地（2026-11-16，两步）**：
  1. **链路**：`bridge::compute_waveform_peaks(pcm, bins)`（纯函数，每 bin 取 `|sample|` 最大值）
     ＋ `TimelineBlockData.waveform_peaks: [float]` ＋ `arrangement.slint` 按峰值画柱；
     空数组仍画「波形待分析 · 示意」（⛔ 不假装有波形）。
  2. **数据**：`ClipView.asset_hash` 把 `ClipContent::Audio.asset` 带出投影（投影层不解码、不读盘）
     ⇒ `waveform_cache`（后台线程、一批一线程、1024 bin 包络、按哈希缓存、失败记账不重试）
     ⇒ `host::pump_waveform_analysis` 挂在 16 ms 心跳上（打开工程**不是**唯一入口：
     录音落点 / 导入音频也会在运行期往 CAS 池加字节）。
     **资产字节来源 = 会话 CAS 池**（`UndoPort::replace_assets` / `insert_asset`，即
     `open_project_document_file` 的 `archive.assets`）—— 上面「依赖」一节担心的缺口由
     `#132` 的池装配补上，⛔ 不再需要另开「读 assets」的入口。
- **仍缺（如实登记）**：① 视觉回归**没有**覆盖到它（golden 比对仍禁用，见 D2），
  所以「画得对不对」只有源码判据 + 行为判据（`waveform_cache` 的单元测试）；
  ② 字节不在会话 CAS 池里的外部引用（只存在于控制面的资产）⇒ 停在「示意」。

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

### B5 分轨导出 GUI（中，2026-11-16 降级：后端已存在）
- **核验结论**：`in-process-mcp` 自 2026-11-12 起**默认开**（`Cargo.toml:270`
  `default = ["in-process-mcp"]`）；`rfd = "0.17.2"` 已是依赖；**后端现成**：
  `crate::export_stems::export_stems(...)`（与 MCP `yeban_export_stems` 同一份）。
- 实现：命令臂 `export-stems` → `rfd` 选目录 → 调 `export_stems`（放工作线程，
  复用 P1-A 模式）→ 回执。⛔ 不再只是「请用 CLI」。

### B6 多选框选（大，需裁决）
- 代码明确「marquee 未裁决」：空白拖拽当前是**滚动**语义；改成框选会改变既有交互。
- 需产品裁决：空白拖拽 = 滚动 还是 框选（或修饰键区分）。
- 裁决后再做：框选矩形 + 命中测试 + 多选状态（`selected_ulids` 已是数组）。

## 后果

- 正向：6 项从"散落在报告/口述"变成**有证据、有实现要点、有验收口径**的立项清单；
  P2-1 死代码本次已清。
- 负向 / 限制：B6 **仍未实现**（卡在产品裁决：空白拖拽 = 滚动 还是 框选）；B1/B2/B3/B5 已落地
  （B1 见上「落地（2026-11-16，两步）」）。⛔ 不把"仍是示意/请用 CLI"当作已修 ——
  本轮之后这三句都已从代码里消失（B2 画真曲线、B5 真导出、B1 有真峰值）。
