# 《编曲与配器手册》术语合并记录

## 数量

- 本册术语表：105 条（中英各一套，逐行对应）。
- `common/glossary-zh.tex` 原有：40 条；合并后：142 条。
- `common/glossary-en.tex` 原有：40 条；合并后：142 条。
- 实际新增：102 条；跳过重复：3 条。

## 去重与冲突

| 中文 | 本册 English | common 已有 | 处理 |
| :-- | :-- | :-- | :-- |
| 素材池 | material pool → asset pool | `asset pool` | 冲突：本册原用 material pool，common 用 asset pool；统一为 **asset pool**，正文与术语表同步改，common 不重复添加 |
| 采样 | sample | `sample` | 完全重复，跳过 |
| 分轨导出 | stem export | `stem export` | 完全重复，跳过 |

无内部重复：105 条中文词条与 105 条英文词条各自唯一。其余 102 条按本册章节顺序追加到两个 common 文件末尾，原有 40 条不动。

## 其他修整

- 本册 `ch00-plan.tex` 术语表原有 90 行以单反斜杠 `\` 结尾（不是 longtable 的 `\\` 行终止），已全部改为 `\\`；中英各 90 行。
- 术语表标题由“新增部分/New terms”改为“本册术语表/Glossary of this volume”，说明改为“已合并进共用术语表，重复/冲突见本记录”，避免合并后仍声称“common 尚未收录”。
