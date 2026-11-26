# 《混音手册》整册装配审计

- 基线：`line/incoming-doc-infra` `2d59bbff8069bc7e963425fa860ac1be8fed2189`
- 章节：10 章，中英各 10 个章节文件，共 20 个 `.tex`
- 主文件：`zh/main.tex`、`en/main.tex`，各 10 条 `\input{chapters/01-overview}` 至 `\input{chapters/10-from-rough-mix-to-finished}`，顺序与大纲一致
- 样章 `ch01-sample.tex` 已从两语言 `chapters/` 删除；第一章归位为 `chapters/01-overview.tex`

## 1. 结构检查

| 项目 | 中文 | 英文 |
|---|---|---|
| 章节文件 | 10 | 10 |
| `\chapter` | 10 | 10 |
| `\label` | 67 | 67 |
| 重复 `\label` | 0 | 0 |
| `\ref` / `\pageref` | 0 | 0 |
| 未解析引用 | 0 | 0 |
| 花括号平衡 | 通过 | 通过 |
| `\begin`/`\end` 配对 | 通过 | 通过 |

本册正文没有使用 `\ref`/`\pageref`；67 个标签是章/节/表标签，当前不存在悬空引用。装配未新增交叉引用，也未改动各章正文语义。

## 2. 截图占位

| 项目 | 中文 | 英文 |
|---|---|---|
| `\screenshot` 实例 | 23 | 23 |
| 唯一图名 | 23 | 23 |
| 两参数形式 | 23 | 23 |
| 单参数/三参数残留 | 0 | 0 |
| `SCREENSHOTS.tsv` 行数 | 23 | 23 |
| 图名与清单不一致 | 0 | 0 |

`SCREENSHOTS.tsv` 汇总 23 个唯一截图，含章节、中文/英文 caption、中文/英文拍摄步骤；第一章与第二章至第七章的单参数占位已统一为共用样式 `\screenshot{name}{caption}`，原拍摄步骤移入 TSV。

## 3. 帮助主题

| 项目 | 中文 | 英文 |
|---|---|---|
| 帮助锚点 | 71 | 71 |
| 唯一锚点 | 71 | 71 |
| 重复锚点 | 0 | 0 |
| `HELP-MAP.tsv` 行数 | 71 | 71 |
| 锚点与清单不一致 | 0 | 0 |

`HELP-MAP.tsv` 记录章号、所在小节与锚点；两语言锚点集合相同，未发现重复或空锚点。

## 4. 术语

| 项目 | 中文 | 英文 |
|---|---|---|
| `common/glossary-*.tex` 条目 | 124 | 124 |
| 重复条目 | 0 | 0 |
| `TERMS-MERGED.tsv` 条目 | 124 | 124 |
| 冲突/别名记录 | 10 条 | 10 条 |

术语来源为 `common/` 种子 40 条、首包 `TERMS.tsv` 与第 2–10 章回执；以中文词为键去重，冲突写入 `GLOSSARY-CONFLICTS.md`。`增益结构` 的 `gain structure` 与 `gain staging` 并存，表内保留首包译名，正文不强行替换。

## 5. 装配文件

- `docs/manual-latex/mixing/{zh,en}/main.tex`
- `docs/manual-latex/mixing/{zh,en}/chapters/01-overview.tex` 至 `10-from-rough-mix-to-finished.tex`
- `docs/manual-latex/mixing/SCREENSHOTS.tsv`
- `docs/manual-latex/mixing/HELP-MAP.tsv`
- `docs/manual-latex/mixing/TERMS-MERGED.tsv`
- `docs/manual-latex/mixing/GLOSSARY-CONFLICTS.md`
- `docs/manual-latex/common/glossary-zh.tex`
- `docs/manual-latex/common/glossary-en.tex`

## 6. 未做

- 未编译 PDF；按任务纪律只做静态结构/配对检查与 `git apply --check`。
- 未运行 cargo；本章为纯文档装配。
- 未实际拍摄 23 张截图；清单为待拍摄步骤。
- 未做逐页视觉排版检查；共用样式沿用 `line/incoming-doc-infra` 的 `yeban-manual.sty`。
