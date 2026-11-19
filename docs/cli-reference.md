# CLI 参考：非导出族开关（`yeban-app`）

> 为什么单独一份：这些开关在 `yeban-app --help` 里存在，但在 190 份文档里**一次都没写过**
> （2026-11-19 审计发现 15 个）。本文件只补**没有文档面的族**；导出 / 保存 / MCP 那几族
> 见 `docs/tool-availability.md` 与 `--help` 本身。
>
> ⚠ 口径：本文件的开关清单来自 `yeban-app --help` 的**现读**；⛔ 不要在这里抄行号。
> 用法错误的统一退出码：**2**（`--help` 末段逐条列了组合规则）。

## 1. 采样切片（`--slice-audio` 族）

把一段音频切成采样器 region：onset 检测切到字 → 变形 → 4/10 ms 淡入淡出。
产物 = `<stem>_<nnn>.wav`（32 位浮点，与输入同采样率同声道）＋ `<stem>.sfz`（每片一个 `<region>`，
根音从 `--slice-base-key` 起逐片 +1）。

| 开关 | 作用 | 缺省 / 域 |
| :--- | :--- | :--- |
| `--slice-audio <input.wav>` | 主开关（必填输入） | — |
| `--slice-out <dir>` | 输出目录（**必填**；单独给 = 用法错误） | — |
| `--slice-transform <spec>` | 变形：`none` / `tape:<0.25..=4>`（变速变调）/ `shift:<±1200 音分>`（移调，时长不变）/ `stretch:<0.25..=4>`（拉伸，音高不变） | `none` |
| `--slice-base-key <0..127>` | 第 0 片根音；第 i 片 = base + i，超出 127 ⇒ 失败 | `60`（C4） |
| `--slice-cuts <csv 秒>` | 显式切点（覆盖 onset 检测；第 0 帧起切，什么都不丢） | onset 检测 |
| `--slice-min-gap-ms <ms>` | onset 检测的最小切点间隔 | `80` |
| `--slice-threshold-db <db>` | onset 检测的绝对底线（dBFS） | `-50` |

## 2. 样本编辑（`--edit-samples` 族）

对样本做**层合成 + 归一 + 可选音准校正**，产出报告与编辑后的样本。

| 开关 | 作用 | 缺省 / 域 |
| :--- | :--- | :--- |
| `--edit-samples` | 主开关 | — |
| `--sample-edit-out <dir>` | 报告与编辑后样本的落点（不存在即创建） | 必填 |
| `--sample-edit-rms <dbfs>` | 目标 RMS | `-18`（域 `-60..=0`） |
| `--sample-edit-layers <list>` | 合成哪些伪造力度层：`none` / `mf` / `f` / `mf,f` | `mf,f` |
| `--sample-edit-no-tune` | 跳过音准校正（只做层合成与归一） | 关 |
| `--sample-edit-report-only` | 只测算、只写报告，**不写音频**（预览口径） | 关 |

## 3. 旧工程迁移（`--migrate-groove`）

把旧 Groove Web 的 **JSON** 工程包（`.groove`，`format=groove-project` v1/v2）迁移成 `.yeban` 容器。

- 必须与 `--save-as <new.yeban>` **成对**给出（⛔ 不默认落点）；
- **只读**旧文件（迁移前后逐字节比对）；输出行：`migrated-groove:` / `migrated-groove-counts:` /
  `migrated-groove-loss:`（有损字段逐条）/ `migrated:`；
- `format=groove-package` 或坏 JSON ⇒ **响亮报错（退出码 3）**，⛔ 不产半成品；
- 不与别的动作组合（组合 = 用法错误）。

## 4. 整曲概览的口径修饰（`--overview-score-only`）

`--export-song-overview` / `--export-song-overview-html` 的修饰：**跳过整曲渲染，只出谱面分析**
（回执 `render=score-only`，JSON 多一个 `renderMode` 字段）。
⚠ 单独给 = 用法错误（它只改那一份产物的口径，没有东西可修饰）。

## 5. 渲染线程（`--render-threads`）

`--export-wav` 的显式渲染线程数，与 MCP `render_master.threads` 同量纲：

- 缺省 = 多线程默认路径（可用并行度）；
- 给了就透传给渲染器，**输出逐字节相同**（并行不改结果）；
- 必须是**正**整数（0 线程没有语义 ⇒ 用法错误）。

## 6. 分轨选轨（`--stems-track-ids`）

`--export-stems` 的选轨修饰：逗号分隔的 ULID 清单，只导这些轨（与 MCP `trackIds` 同义，顺序无关）。

- 缺省 = 全部非 master 源节点轨；
- 空清单 / 空条目 ⇒ 用法错误（退出码 **2**）；
- 非法 ULID、工程中不存在的身份 ⇒ 后端**响亮拒绝**（退出码 **5**），⛔ 不静默丢轨。

## 7. 组合规则速查

单独给即用法错误的修饰（`--help` 末段逐条列出）：`--slice-out` / `--slice-transform` /
`--slice-base-key` / `--slice-cuts` / `--slice-min-gap-ms` / `--slice-threshold-db`（都要跟 `--slice-audio`）、
`--sample-edit-*`（都要跟 `--edit-samples`）、`--overview-score-only`（要跟 `--export-song-overview*`）、
`--stems-track-ids`（要跟 `--export-stems`）、`--render-threads`（要跟 `--export-wav`）。
