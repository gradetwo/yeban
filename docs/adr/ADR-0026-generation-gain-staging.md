# ADR-0026 — 生成声部出厂即分级；种子轨电平 −12 dBFS

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定）**
- **决定者**: 集成者（Agent）
- **关联**: `MCP-TOOL-005`（`yeban_propose_section`）、`MCP-TOOL-001`（`yeban_create_project`）、
  `ARCH-DET-001`（L1 确定性：默认值是文档的一部分）；
  `crates/yeban-mcp/src/domain/section_build.rs`（`VOLUME_*_DB`、`part_volume_db`）、
  `crates/yeban-mcp/src/domain/project_create.rs`（`SEED_TRACK_VOLUME_DB`）、
  `crates/yeban-mcp/tests/creation_acceptance.rs`
  （判据 `generated_sections_arrive_pre_staged_and_render_without_clipping`）
- **日期**: 2026-10-27

## 背景

2026-10-27 的 MCP 创作链路审计（G6）实测：一个 8 小节理论段
（`yeban_create_project` → `yeban_propose_section` → `yeban_render_master`）
直接渲染 **`clippedSamples 527020/768000`（68.6%）、`truePeakDbfs +14.79`**
（鼓 +9.02 dBFS、各声部约 0 dBFS）。原因是**两级默认都是 0 dBFS 出厂**：

1. `section_build.rs` 给生成声部一律 `volume_db = 0.0`；
2. `project_create.rs` 的种子 MIDI 轨（默认 Motif C4 D4 E4 G4，满幅正弦）
   同样 0 dBFS，实测**单源**真峰值 **+0.44 dBFS**。

⇒ 每首生成的曲子在首听之前都要人工逐轨 `setParam` TrackVolume；
而"创建即可听"是本工具面的承诺。这不是混音审美问题，是默认值缺口。

## 决定

### D1 生成声部按角色分级（`section_build.rs` 四常数）

`Melody −22 / Drums −25 / Bass −25 / 内声部 −28`（dBFS）。

**数值不是从公式推的，是按渲染真值调出来的**（测量纪律 §6）：

| 轮次 | 档位 | 实测 clippedSamples | 实测 truePeakDbfs |
|---|---|---|---|
| 修复前 | 0/0/0/0 | 527020 | +14.79 |
| 第一轮 | −12/−15/−18 | 942 | +3.52 |
| 第二轮 | −22/−25/−28 | 60 → 0（配 D2 后） | +1.24 → −7.45（配 D2 后） |

第二轮 −10 dB 只把峰值拉下 2.28 dB —— **非线性**，剩余贡献来自 D2 的种子轨。
角色相对结构（前景最响、低音/鼓次之、铺底最低）保持不变。

### D2 种子轨出厂电平 `SEED_TRACK_VOLUME_DB = −12.0`

判据线是"零削波 **且** 主控真峰值 ≤ −6.0 dBFS"（给手写叠加内容留余量；
`#335` 与本次审计的手工 workaround 同量级）。第一轮只分级生成轨时，
`sources[].peakDbfs` 逐源读数（`ADR-0020` 的能力）显示**主控源是种子轨**
（−9 档实测 −8.56 dBFS，生成声部都在 −16 以下）⇒ 生成段的相对结构被一条
演示轨盖住。种子轨 −9 档实测主控 −5.81 > −6.0 ⇒ 定到 −12，实测主控
**−7.45 dBFS、`clippedSamples = 0`**。调用方随后 `setParam` TrackVolume
照旧覆盖，`seed` 参数路径不受影响。

### D3 判据与锚点边界

- 判据测试读工具面渲染回执的 `clippedSamples` / `truePeakDbfs`（单位：帧 / dBFS），
  并逐角色断言四条生成轨的 `volume_db` 与常数表一致；
- `#335` 的 `EXPECTED_LUFS = −12.74` 锚点**不受影响**：`build_piece` 对四条
  piece 轨都显式 `setParam` TrackVolume（−9/−12/−15/−12），种子默认值被覆盖；
- 鼓单源 +9.02 dBFS 是波形配方本身（重音力度），压 25 dB 后其余量仍由 D1 的
  相对档保证，不在实时链做任何自动增益（不触碰 `ARCH-RT-*`）。

## 代价

- 首听响度整体比旧出厂低约 20 dB —— 对"叠轨余量"是收益，对"单轨试听音量"
  是变化；`setParam` 仍是唯一覆盖入口，无新 API。
- 四常数按"当前波形配方 + 当前理论段密度"标定；未来若 `osc*_wave` 默认变化
  （G5）或鼓力度模型变化，判据测试会先红（这是它存在的意义）。

## 编号说明

`ADR-0024`（G4 线）与 `ADR-0025`（G7 线）在本线分叉前已分配；本文件取 `0026`，
落地顺序以合并进 main 的先后为准。
