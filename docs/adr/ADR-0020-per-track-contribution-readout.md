# ADR-0020 — `yeban_render_master` 报逐轨贡献（`contributions`，消除 WALL ⑦）

- **状态**: **Accepted（集成者依 2026-10-25 人类常设授权裁定）**
- **决定者**: 集成者（Agent）
- **关联**: `MCP-TOOL-008`（`yeban_render_master` 契约）；
  `crates/yeban-mcp/src/domain/render.rs`（`RenderRequest` / `RenderArtifact` /
  `track_contributions`）、`crates/yeban-mcp/src/domain/mod.rs`（`plan_render_master`）、
  `crates/yeban-mcp/src/tools.rs`、`crates/yeban-mcp/src/output_schema.rs`；
  `schemas/mcp-tools.schema.json`；`WALL ⑦`（`creation_acceptance.rs`：
  "分轨贡献/差值没有一眼读法"）；`MODEL-AST-003`（`BTreeMap` 行序确定性）
- **日期**: 2026-10-27

## 背景

`crates/yeban-mcp/tests/creation_acceptance.rs` 的墙清单里，**WALL ⑦** 记录：

> `yeban_render_master` 的 `sources[].peak` 已经给了**一次渲染**里的逐源峰值，
> 但"拿掉这条轨后母带变了多少"没有读数：逐轨**贡献**只能靠 N 次静音/独奏渲染 +
> 本地逐样本比较（本文件 4 次静音 + 2 次独奏）。这不是工具错误，是测量成本；
> 它说明工具面没有"分轨贡献/差值"的一眼读法。

这是 agent 混音时的**核心读数**：知道每条轨在母带里留下多少能量，才能回答
"这条轨是不是根本没响"、"这两条轨是不是叠在一起把低频堆爆了"。
`sources[].peak` 回答不了：它是**源节点输出**（不含该轨插入链、路由边增益与母带增益），
且**不是**"拿掉这条轨，母带差多少"。

调用方自己做这件事的代价：每条轨一次 `yeban_edit_notes` 静音 + 一次 `yeban_render_master`
+ 一次还原，再把 WAV 读回本地逐样本比较 —— 一次混音决策要 `3N + 1` 次往返与一份
本地解码器。

## 决定

### D1 新增可选实参 `contributions`（布尔，缺省 `false`）

- `false`／缺省 ⇒ **不额外渲染**，响应里**不出现** `contributions` /
  `contributionsBasis` 两个键；写盘字节与加这个键之前逐字节相同
  （`sha256` / `masterDigest` 一位不变）。
- 非布尔 ⇒ `INVALID_PARAMETER_RANGE`（⛔ 不静默当 `false`）。

### D2 量什么（对象 ＋ 单位）

工具内做 `1 + N` 次**额外**渲染（`N` = 未静音的非 master 轨数）：

1. **基线** = 本请求关掉归一化（`normalize = false`）、目标响度
   （`target_lufs_bits = None`）、显式路径（`path = None`），帧数**钉成**本次母带的
   `frames`（静音"最晚结束"的那条轨不许让时间轴变短，否则两次渲染长度不同、差值无意义）。
   采样率 / 格式 / 位深与调用方完全一致。
2. 对每条**非 master** 轨，克隆工程、只把该轨 `mute = true`，再渲染一次。
3. 两次母带都取**最终 24-bit PCM 负载**（`bytes[header_bytes .. header_bytes +
   payload_bytes]`，小端、符号扩展、除以 `2^23`），逐样本比：

   - `deltaRmsDbfs` = `20·log10( RMS(reference − muted) )`
   - `deltaPeakDbfs` = `20·log10( max |reference − muted| )`

   单位 dBFS（满量程 = 0 dBFS）。逐样本差恒为 0 ⇒ 两个值都是 `null`
   （⛔ 不报 `-inf`）。

行序按 `project.tracks` 的 `BTreeMap` 身份序 —— 跨进程、跨重启一致 [MODEL-AST-003]。

### D3 诚实的边界

- 渲染**之前**已经是 `mute = true` 的轨 ⇒ `alreadyMuted: true` 且两个差值 `null`，
  **不**为它渲染（"拿掉一个已经被拿掉的"没有可测的变化，⛔ 不假装量过）。
- 工程里有别的轨 solo 时，静音一条**非 solo** 轨不会改变母带 ⇒ 该轨差额为 `null`。
- 基线**刻意不归一化**：归一化会把差值按各自不同的增益缩放，于是"变了多少"变成
  "归一化算法变了多少"。请求带 `normalize: true` 时，响应里的母带字节仍是归一化后的
  （D1 不变），但 `contributionsBasis.normalize` **恒为 `false`**，指明差值取的是
  未归一化基线。
- 它证明"这条轨在母带里留下多少能量"，⛔ **不证明**"这条轨在音乐上该不该被听见"。

### D4 响应形状

```json
{
  "contributions": [
    {"trackId": "01…", "name": "Lead", "alreadyMuted": false,
     "deltaRmsDbfs": -18.4, "deltaPeakDbfs": -9.1},
    {"trackId": "01…", "name": "Pad", "alreadyMuted": true,
     "deltaRmsDbfs": null, "deltaPeakDbfs": null}
  ],
  "contributionsBasis": {
    "method": "对每条非 master 轨各渲染一次\"只静音该轨\", 与同一份基线逐样本比",
    "metric": "两次母带样本逐样本差的 RMS / 峰值, 单位 dBFS (20·log10, 满量程 = 0 dBFS)",
    "domain": "解码后的 24-bit PCM 负载",
    "normalize": false,
    "frames": 96000,
    "renders": 3,
    "nullMeans": "…",
    "doesNotProve": "这条轨在音乐上该不该被听见"
  }
}
```

- `contributionsBasis.renders` = 本次**实际**额外渲染次数（`1 + 未静音轨数`），
  让调用方知道这个读数的成本。
- `output_schema.rs` 里两个键是**可选**属性（⛔ 不进 `required`），因为缺省时它们不存在。
  ⚠ 因此本工具在 `outputSchema` 下也有**两条合法形状**（缺省 `false` 无这两键、
  `true` 有）：`validate_complete`（"声明的每个字段都出现"）只能用
  `contributions: true` 的响应跑（`tests/props_176.rs` 已改成这样），
  缺省形状用 `validate`（`tests/render_master.rs` 的侧链判据同步改法）。

### D5 机械读数同步（本次一并改钉）

- `crates/yeban-mcp/tests/creation_acceptance.rs`：`WALLS` 4 → 3（删 ⑦，编号保持历史原样）；
- `crates/yeban-mcp/tests/render_master.rs`：新增
  `contributions_match_an_independent_mute_render`（工具内读数 == 独立"真静音 + 再渲染"
  的本地逐样本差，误差 < 1e-9 dB；开关不改 `sha256` / `masterDigest` / 磁盘字节）与
  `a_pre_muted_track_reports_null_contributions`（已静音轨 ⇒ `alreadyMuted` + `null`；
  归一化请求仍用未归一化基线）；
- `tools.rs` 注册表新增 `contributions` 参数、`schemas/mcp-tools.schema.json` 的
  `DocumentedToolArguments.$defs.yeban_render_master` 同步登记（缺省 `false`）；
- `crates/yeban-mcp/tests/props_176.rs`：全字段反校的那次渲染显式带
  `contributions: true`（并把两键加入抽查字段），使 `validate_complete` 覆盖完整形状；
  `crates/yeban-mcp/tests/render_master.rs` 的侧链 schema 判据改用 `validate`。

## 代价

1. **成本是 `1 + N` 次完整渲染**：本项目的一轨 ≈ 一次母带渲染的时间；100 轨工程 =
   101 次渲染。它是**可选**读数（缺省一次都不多渲染），且成本如实写进
   `contributionsBasis.renders`；`dryRun` 也会真的做（与既有 `dryRun` 真渲染的口径一致）。
2. **差值取自最终 24-bit 负载**：小于 1 LSB 的变化只会表现为"样本差恰为 0 或 ±1 LSB"，
   因此 `null` 的语义是"逐样本完全相等"，不是"物理上绝对无声"。
3. **非归一化基线 ≠ 写盘母带**（当 `normalize: true` 时）：两者增益不同。这是刻意的
   —— 贡献读数必须在同一线性尺度上比；调用方若想复核，应在同一份未归一化请求上做。
4. **不覆盖"逐源瞬时电平"**：`sources[].peak` 仍是源节点输出（不含插入链/路由边/
   母带增益）；两者回答不同问题，不能互相替代。
5. **总线轨也在清单里**：凡是 `tracks` 里非 master 的轨都有行（包括总线轨），因为
   "静音总线"确实会改变母带；不按 `TrackKind` 过滤，避免出现"清单里少了一条轨"
   这种无法解释的空洞。

## 待人类

无。本裁决只新增一个**可选**实参 + 一个**可选**响应键，缺省路径逐字节不变；
若人类日后要求"逐轨贡献必须异步/限流"，在 D4 的 `renders` 读数旁边加闸门即可，
不影响本裁决的语义。
