# 《编曲与配器手册》术语合并记录

## 2026-11-27 T-FIX-877 流行独立章

- 新增 4 条：`情绪曲线 / emotional curve`、`耳糖 / ear candy`、`流行 / pop`、`托普线 / topline`。
- 本册术语表：128 → 132 条；共用术语表中文 423 → 427 条、英文 422 → 426 条。
- 英文 `流行` 一行释义中的中文已清除；中英行序保持对应。
## 2026-11-27 T-FIX-878 摇滚独立章

- 新增 4 条：`失真吉他 / distorted guitar`、`静响对比 / quiet-loud dynamics`、`吉他墙 / guitar wall`、`摇滚 / rock`。
- 本册术语表：128 → 132 条；共用术语表中文 423 → 427 条、英文 422 → 426 条。
- 中英行序保持对应；本章不新增重复词条。

## 数量（2026-11-26 曲风章之后）

- 本册术语表：**128 条**（中英各一套，逐行对应；前十章 105 条＋曲风章 23 条）。
- `common/glossary-zh.tex`：**389 条**（本次曲风章追加前 366 条）。
- `common/glossary-en.tex`：**389 条**（本次曲风章追加前 366 条）。
- 本次新增：**23 条**；跳过重复：0 条。

## 本次新增（第 11 章，曲风与配器）

按英文键插入共用术语表，中英行号保持同序：

- 曲风 / genre
- 编制 / instrumentation
- 律动型 / groove pattern
- 循环动机 / riff
- 钩子 / hook
- 主调织体 / homophony
- 复调织体 / polyphony
- 织体分层 / stratified texture
- 声部密度 / voice density
- 音域 / range
- 移调 / transposition
- 适奏音区 / tessitura
- 留白 / space
- 固定音型 / ostinato
- 升压段 / build-up
- 电子落点 / drop
- 侧链 / sidechain
- 摇摆 / swing
- 行走贝斯 / walking bass
- 强和弦 / power chord
- 蓝调音 / blue note
- 影视配乐 / film score
- 国风 / guofeng

本册 `ch00-plan.tex` 的术语表同步追加同样 23 行；章节内第一次出现时用 `\term{中文}{English}` 建索引。

## 历史合并（第 1—10 章）

- 第 1—10 章本册术语表：105 条（中英各一套，逐行对应）。
- 第 1—10 章合并时，`common/glossary-zh.tex` 与 `common/glossary-en.tex` 各由 40 条增至 142 条；实际新增 102 条，跳过重复 3 条。
- 后续其他分册继续并入共用术语表；本次曲风章追加前，两份 common 均为 366 条。

## 去重与冲突（第 1—10 章）

| 中文 | 本册 English | common 已有 | 处理 |
| :-- | :-- | :-- | :-- |
| 素材池 | material pool → asset pool | `asset pool` | 冲突：本册原用 material pool，common 用 asset pool；统一为 **asset pool**，正文与术语表同步改，common 不重复添加 |
| 采样 | sample | `sample` | 完全重复，跳过 |
| 分轨导出 | stem export | `stem export` | 完全重复，跳过 |

无内部重复：105 条中文词条与 105 条英文词条各自唯一。其余 102 条按本册章节顺序追加到两个 common 文件末尾，原有 40 条不动。

## 其他修整

- 本册 `ch00-plan.tex` 术语表原有 90 行以单反斜杠 `\` 结尾（不是 longtable 的 `\\` 行终止），已全部改为 `\\`；中英各 90 行。
- 术语表标题由“新增部分/New terms”改为“本册术语表/Glossary of this volume”，说明改为“已合并进共用术语表，重复/冲突见本记录”，避免合并后仍声称“common 尚未收录”。
- 第 11 章新增术语按英文键插入 alphabetic 位置，不整体重排既有 366 条，避免无意义的大 diff。
