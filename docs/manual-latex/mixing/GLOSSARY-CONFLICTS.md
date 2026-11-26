# 《混音手册》术语合并与冲突清单

- 合并后条目：124 条（中英各一份，按同一顺序写入 `common/glossary-zh.tex` / `common/glossary-en.tex`）。
- 合并来源：`common/` 种子 40 条、首包 `TERMS.tsv`、第 2–10 章回执的新增术语。
- 去重规则：以中文词为键；同词不同英文记冲突；同英文不同中文也记冲突。

## 冲突与别名

| 中文 | 保留英文 | 另一英文/来源 | 处理 |
|---|---|---|---|
| 均衡 | equalization | EQ（TERMS.tsv） | 保留 equalization；EQ 作界面缩写，不另立条目 |
| 压缩 | compression | compressor（TERMS.tsv） | 保留压缩/compression；设备名另立“压缩器 / compressor” |
| 门限 | gate | 门限器（CH04） | 保留“门限 / gate”；设备语境可用“门限器” |
| 发送 | send | 发送式（CH05） | 保留“发送 / send”；路由方式用“发送式” |
| 主总线 | master bus | 母线（CH10） | 保留“主总线 / master bus”；“母线”为同义简称 |
| 增益结构 | gain structure | gain staging（CH09/CH10） | 保留首包 `gain structure`；正文两种说法并存，不强行替换 |
| 中频峰 | mid peak | 中峰（CH03） | 保留“中频峰 / mid peak”；“中峰”为简称 |
| 干湿比 | wet/dry mix | 湿路电平（CH05） | 保留“干湿比”；“湿路电平”描述同一参数 |
| 辅助返回 | auxiliary return | 辅助返回轨（CH09） | 保留“辅助返回”；“辅助返回轨”指同一条轨 |
| 检波器 | detector | 检波窗口（CH10） | 保留“检波器 / detector”；“检波窗口”为同义表述 |

> 以上别名不另立条目，正文可互用；`TERMS-MERGED.tsv` 保留 124 条规范条目。
