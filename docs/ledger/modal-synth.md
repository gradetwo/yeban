# 原生模态合成乐器（T-FIX-317）第一期：出处、选型与第二期建议

## 0. 出处与致敬（硬要求，逐模型）

| 对象 | 作者 / 许可 | 本项目怎么用 |
| :--- | :--- | :--- |
| **RipplerX** <https://github.com/tiagolr/ripplerx> | tiagolr，**GPL-3.0**（与夜半同许可） | 算法与拓扑参照：每模态一只带通谐振滤波器、噪声 / mallet 激励、至多 64 分音 / 共振器。⛔ 不搬 JUCE 框架代码、查找表类或插件外壳；按算法用 Rust 重写。 |
| **Rippler2**（tiagolr 的 JSFX）与 **Partials** <https://github.com/JoepVanlier/JSFX> | JoepVanlier，**MIT** | 技术血统（经 RipplerX README 的 Acknowledgments 转述）。本项目只作来源登记，未直接取 JSFX 代码。 |
| **MechanOdd** <https://github.com/odoare/MechanOdd> | odoare，**代码许可未声明** | ⚠ 只读其 README「Resonators: DSP and Mathematics」数学节作公式交叉对照；**不取一行代码**。板 / 膜的频率律、阻尼斜坡、`Q=1/(2ζ)`、增益补偿 `sqrt(ζ_ref/ζ)` 均逐项对照。 |
| **AAS Chromaphone**、**Ableton Collision** | 专有 | 模型层面的上游参照，经 RipplerX README 转述；本项目未 reverse engineer 专有实现。 |

逐模型来源见 `crates/yeban-dsp/src/modal.rs` 模块文档的表。

## 1. 选型审计（Wang 2026-11-24 口径）

三条判断：**① 对我们有用；② 能完善我们已有的功能；③ 能补全我们的缺口**。
重复项**不直接剔除**：先比覆盖的模型/参数面、音色与数值行为、性能与实时纪律、
判据完备程度；对方明显更好的学 / 搬 / 替换，不如或增益不大的才不搬。
**String / Beam / Membrane / Plate 是定案核心范围，本审计不删减不替换。**

现有能力清单（开工时现读）：SFZ 采样引擎（`yeban-sfz` / `yeban-engine`）、
鼓机（`yeban-dsp::drums`）、内置复音合成（`yeban-engine::synth` /
`yeban-dsp::polysynth`）、梯形滤波 / RBJ 通道条 EQ / 各种效果器
（`filter` / `shaping` / `comb` / `reverb` / `delay` / `compressor` 等）。
`yeban-dsp` / `yeban-engine` 中 **grep 模态 / 共振器零命中**（只有 drums/voice.rs:887
的一句注释提到铃的"两个主模态"），无现成模态合成实现。

| 候选 | 与夜半现有能力的关系 | 对比结论（重复项写明比了什么） | 裁决 |
| :--- | :--- | :--- | :--- |
| 模态滤波组（每模态 RBJ 带通 + 阻尼斜坡 + 增益补偿） | 现有 `voice_filter` 是 SFZ 截止频率型；`shaping::Biquad` 是通道条 EQ，都不是"逐模态 Q/耦合" | 覆盖面：现有无逐模态参数面；数值：RBJ 同式但用途不同；实时：本模块定长数组零分配；判据：新增频率律/衰减/确定性四组 | **搬**（基础设施，本期保留） |
| **String**（谐波列 + 刚度非谐性） | `polysynth` 的振荡器是谐波叠加，但无刚度非谐 / 模态敲击点 | 波形覆盖：振荡器有，敲击模态没有；判据：新律 | **搬**（定案核心） |
| **Beam**（`n`↔`n²` 连续变形） | 无同类 | — | **搬**（定案核心） |
| **Membrane**（二维 sqrt 律） | 鼓机有采样/合成鼓，但无二维模态律 | 鼓机覆盖固定鼓件；膜模型补连续音高与敲击点参数面 ⇒ 互补不重复 | **搬**（定案核心） |
| **Plate**（二维平方律） | 无同类；`reverb` 的 modal 不是共振器本体 | — | **搬**（定案核心） |
| 噪声 burst 激励 | `noise` / `noise_source` 有通用噪声 | 现有是无状态噪声源；这里需要**确定性 seed + 衰减包络 + 与模态触发绑定**的 burst ⇒ 参数面不同、互补 | **搬**（基础设施，本期最小形） |
| mallet 激励（短脉冲 + 低通成形） | `drums` 的瞬态包络相近但绑定固定鼓件 | 覆盖：mallet 的硬度/低通与模态敲击点联动是现有没有的；音色：冲激激励 vs 鼓机采样；判据：新增 | **搬**（基础设施，本期最小形） |
| Squared（`k²`） | 与 Beam 的 `mix=1`（`n²`）在频率律上重复 | 对比：Beam 提供 `0..1` 连续插值，覆盖面**严格大于**固定 `k²`；音色可完全覆盖；性能相同；判据合并进 beam 判据 ⇒ Squared 无增益 | **不搬**（被 Beam 覆盖；第二期若需独立 UI 标签再拆） |
| Drumhead（Bessel 根） | 与 Membrane 的二维律相近但使用 Bessel 零点 | 对比：Bessel 根表需要 64×64 常量或迭代求根，Nightly 端无现成实现；Membrane 的 `sqrt(m²+(n/a)²)/norm` 已覆盖"鼓皮"听感的大部分；收益 < 成本与判据负担 | **不搬**（后续期次可按需加 Bessel 表） |
| Marimba（采样频谱拟合表） | 与 Beam `mix≈1` 接近 | 对比：RipplerX 该表来自 Chromaphone 样本频谱分析（其 README 明言"couldn't program this one"），不是可推导公式；本项目不引入来源不明的拟合常量；Beam 已给 `n²` 主体 | **不搬**（公式不可审计） |
| Open tube / Closed tube（奇/偶谐波） | `polysynth` 的方波已有奇谐波列，但无共振器耦合 | 对比：本期单共振器无模式耦合，tube 主要用于 RipplerX 的 serial-coupling 频移；没有耦合时退化成谐波列，与 String 重复 | **不搬**（随第二期耦合再评估） |
| 双共振器耦合（serial / parallel） | 无同类，明确补缺口 | 但属第二期范围（本期只做单共振器） | **不搬（本期）**，第二期首选 |
| Audio In（侧链激励共振器） | 无同类，补缺口 | 需要新的侧链音频通路与声部管理接线，第一期不做 | **不搬（本期）**，第二期 |
| 预置 / GUI / 微调优 | 夜半已有完整 GUI 与预制体系 | 已有能力，重复 | **不搬**（用现有体系，不为模态另造一套） |
| JSFX 中同类项（JS3Osc / JSAdditiv / JSDrumsynth 等） | 与现有 oscillator / drums 重复 | 对比：已有实现的覆盖面与判据不弱于 JSFX 参考项；无增益 | **不搬** |

## 2. 本期已做 / 未做

**已做**：四种模型的本征频率律（逐项单测）、每模态 RBJ 带通 + 逐模态阻尼/耦合/
增益补偿、阻尼随频率斜坡、增益补偿钳位、噪声 burst 与 mallet 两个激励、
单共振器声部（MIDI 音高 → `f0`，velocity → 激励强度）、确定性逐样本 process。

**未做**：双共振器耦合（serial / parallel）、Audio In 侧链、GUI、预置、
Drumhead / Marimba / Tube / Squared 模型。

## 3. 第二期建议范围

1. 双共振器耦合（RipplerX 的 serial/parallel；Chromaphone 的物理耦合更完整，
   但其算法专有 ⇒ 只按 RipplerX 开源口径学习）；
2. Drumhead（Bessel 根表）与 Open/Closed tube，配合耦合才有完整意义；
3. Audio In 侧链激励，接夜半既有声部/侧链体系；
4. 把本期 DSP 声部接入引擎的内置乐器枚举与 GUI 参数面（复用现有预制体系）。
