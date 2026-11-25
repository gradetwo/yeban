# 乐器册术语合并与冲突清单

- 合并日期：2026-11-26
- 源：`instruments/TERMS.tsv` 与第 1–8 章回执的“新增术语”
- 目标：`common/glossary-zh.tex`、`common/glossary-en.tex`
- 结果：两版各 104 条（合并前各 40 条，新增 64 条；`插入效果` 已存在，去重跳过）

## 1. 去重

| 术语 | 处理 |
|---|---|
| 插入效果 / insert effect | `common` 已有，跳过，不重复追加 |
| 轨道 / track、片段 / clip、混音台 / mixer 等通用词 | `common` 已有，未从乐器册重复导入 |
| 预设 / preset | `common` 已有；只新增其下位的“乐器预制”“工厂合成器预设” |

## 2. 近义与冲突

| 对象 | 冲突/关系 | 处理 |
|---|---|---|
| 主输出 / master output；主总线 / master bus | 相近但不等价：前者是输出级，后者是最终汇流总线 | 两条都保留；正文用“主总线”指路由实体，用“主输出”指输出级 |
| 调式 / mode；模态 / modal | 英文都接近 mode，索引会撞 | “调式”保留 `mode`；“模态”改用 `modal`，不共用索引 |
| 乐器 / instrument；音源 / sound source | 粒度不同：音源是引擎分派类别，乐器是轨上发声设备 | 两条都保留；正文用“音源”讲分派次序，用“乐器”讲设备身份 |
| 预设 / preset；乐器预制 / instrument preset；工厂合成器预设 / factory synth preset | 上下位关系，不是同义词 | 三条都保留；不把“乐器预制”简写成“预设” |
| 固定 ADSR / fixed ADSR；固定包络 / fixed envelope | 前者是具体 5/80/0.7/50 ms 参数，后者是练习章对同一事实的泛称 | 两条都保留；正文以“固定 ADSR”为准 |
| 采样 / sample；采样器 / sampler；SFZ 音色库 / SFZ instrument library | 对象不同（素材、设备、库） | 三条都保留 |

## 3. 状态修正

| 术语 | 原状态 | 合并后口径 |
|---|---|---|
| 鼓机 / drum machine | 已核实（引擎侧），界面待核 | 第 6 章已核实界面入口与鼓组生成，按“引擎＋界面”使用 |
| 插件乐器 / plugin instrument | 待核 | 第 7 章已核实为“仅登记；实时不执行、离线导出/整曲预览执行”，按此边界使用 |
| 物理建模乐器 / physical-modeling instrument | 尚未提供（待核） | 保留术语，但正文明确“只有 DSP 核心、未接成可挂轨乐器” |
| LFO、调制矩阵 | 尚未提供 | 保留术语，正文不写假控件 |
| 音色边界 / voice boundary | 第 3 章新增 | 已并入；用于区分预设名与真实引擎能力 |
