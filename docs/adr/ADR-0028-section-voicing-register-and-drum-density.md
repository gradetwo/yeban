# ADR-0028 — 理论段 3 声部低音区修正；`drumDensity` 档位（normal / sparse）

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定）**
- **决定者**: 集成者（Agent）
- **关联**: `MCP-TOOL-005`（`yeban_propose_section`）、`ARCH-DET-001`（L1 确定性）、
  `ADR-0016`（`materialSource` / `progression` 缺省逐字节兼容契约）、`ADR-0017`（鼓/旋律追加）、
  `ADR-0026`（生成声部分级，本线基线 `4b1ea510` 已含）；
  `crates/yeban-mcp/src/domain/section_build.rs`
  （`SECTION_THREE_VOICES`、`theory_contents`、`DrumDensity`、`drum_contents`）、
  `crates/yeban-mcp/src/tools.rs`、`schemas/mcp-tools.schema.json`
- **日期**: 2026-10-27

## 背景

2026-10-27 的 MCP 创作链路审计（G8）实测两处粒度缺口：

1. **音域**：`materialSource:"theory"` 的 3 声部骨架用 `yeban-theory` 出厂约束
   `DEFAULT_THREE_VOICES`（C3–C5 / C4–C6 / C5–C7）。三段的紧凑性目标把每个声部
   都往各自音区的**顶部**贴，实测落点（单位：MIDI 音高值）约 **67–72** ——
   一个叫 "Bass" 的声部实际坐在 G4/C5 区间，听感上没有低音声部。
2. **鼓密度**：`includeDrums:true` 只有满密度一档（`GenreRule::drum_pattern`
   全量击点），想要稀疏鼓点只能生成后逐音符 `edit_notes` 删，两轮往返。

## 探测证据（先测后改，测量纪律 §6）

独立探测工程（`/tmp` 一次性 crate，只依赖 `yeban-theory` 公开面）对同一组
`ChordSpan` 跑 `voice_leading::realize`，读数（MIDI 音高值）：

| 低音区约束 | bass 落点 | 结论 |
|---|---|---|
| 出厂 C3–C5 | 67–72 | 无低音 |
| 只放宽下限 C2–C4 | 59–60 | **不够** —— 紧凑性目标仍贴上沿 |
| 收紧上沿 C2–C3 | **47–48**（B2/C3） | 达标 |

⇒ 病因是**上沿**，不是下沿；单放宽下限换不来低音。

## 决定

### D1 段落专用三声部约束（不改 `yeban-theory`）

`section_build.rs` 新增 `static SECTION_THREE_VOICES: [VoiceRange; 3]` ＝
**C2–C3 / G3–G5 / C5–C7**（低/中/高），`theory_contents` 的 3 声部分支改传
自构 `VoicingConstraints { ranges: &SECTION_THREE_VOICES, max_voice_jump: 12,
max_total_movement: 24 }`（跳幅/总位移沿用出厂值）。4 声部分支不动
（`FOUR_VOICES` 本就横跨低音区）。

- **模块边界**：约束是 `yeban-theory` 的公开数据结构，MCP 侧就地构造即可，
  ⛔ 不为消费方口味去改引擎层 crate（红线 3 的纪律同样适用于"顺手塞默认值"）。
- **确定性**：静态数组、无随机、无时间依赖 ⇒ `ARCH-DET-001` 不受影响。
- **代价（有意的）**：3 声部理论段的低音音级会变（例：I 和弦的 bass 从 G 变 C）。
  凡 pin 住这些音高字段的测试读数都需要重录 —— 这是修正本身，不是回归。

### D2 `drumDensity` 可选键（缺省 = 逐字节不变）

`PlanOptions.drum_density: Option<&str>`，工具面 `yeban_propose_section` 新增
可选字符串参数 `drumDensity`，接受 `"normal"`（缺省）/ `"sparse"`：

- `sparse` ＝ **只保留 `GenreRule::drum_pattern` 里已登记为重音（`accent`）的击点**。
  ⛔ 不发明第二张节奏表、不改击点 id（`hit_index` 沿用原索引）与力度映射
  （重音 112）；⇒ sparse 输出必然是 normal 输出的真子集。
- 该流派的重音登记为空 ⇒ 显式 `INVALID_PARAMETER_RANGE`（带 `hint`：换
  `"normal"` 或换流派），⛔ 不静默产出空鼓轨、不擅自补击点。
- 守卫：`drumDensity` 只允许与 `includeDrums:true` 同给，否则拒绝（带 `hint`）；
  未知取值拒绝并列出 `accepted: ["normal","sparse"]`。
- 缺省（键缺席）与 `"normal"` 路径**逐字节相同** —— 沿用 `ADR-0016` D2 的
  "新可选键不改旧输出" 契约。

## 没做的（明确排除）

- 声部节奏粒度（每声部注值密度）与 `progression` 无关的更多音域旋钮：审计 G8
  列了但本片不扩接口面，等真实使用反馈再定形状。
- `yeban-theory` 侧新增"段落预设约束"常量：同上，模块边界。

## 编号说明

`ADR-0027` 已被并行工作线 `line/timbre-selection`（G5）在其分支上预留；
本线取 `0028`，落地顺序以合并进 main 的先后为准。
