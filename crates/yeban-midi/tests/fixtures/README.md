# `yeban-midi` 真素材夹具（公有领域）

本目录存放 `yeban-midi` 的**真文件**夹具。这些夹具补的是"只测拒绝、不测接受"的洞。

## 1. 为什么夹具放在这里（而不是 `assets/`）

1. 夹具的**唯一**读者是 `crates/yeban-midi/tests/real_world_smf.rs`。产品代码不读它们。
2. `include_bytes!` 相对本文件所在目录解析。因此测试不依赖运行时工作目录。
3. `assets/**` 的资源必须登记在 `assets/manifest.json`（`AGENTS.md` 第 2 节第 9 条）。
   该文件不在本工作线的允许改动范围内。放进 `assets/` 会强迫本线改动一个共享文件。
4. 三个文件合计 18099 字节，远低于 `AGENTS.md` 第 2 节第 9 条的 10 MB 上限。

## 2. 可提交性判定（本票最先守的一条）

**允许提交**的依据：曲目《致爱丽丝》（`Für Elise` / `WoO 59`）由 **Ludwig van Beethoven** 创作。
该作品属于**公有领域**。负责人已显式裁决：`Für Elise` 与 `fur_Elise_WoO59` 可以提交。

| 本目录文件 | 字节 | SHA-256 | 源文件（`/tmp/midi/`） |
| :--- | ---: | :--- | :--- |
| `fur_elise_woo59_384ppq_3mtrk.mid` | 7590 | `1c12c21c7bbf4cf163896732672648a69d497636059837abd153c71abe50215a` | `fur_Elise_WoO59.mid` |
| `fur_elise_480ppq_1mtrk.mid` | 3822 | `da184d44b77c817ae8239b5862ea6a7842a2e240dccc08f2e16a3866e646ae42` | `Für Elise 1 tracks.mid` |
| `fur_elise_480ppq_3mtrk.mid` | 6687 | `a80f39556afb851e7af5ee63858113d85484fe644bac63c02c1894f218b88984` | `Für Elise 2 tracks.mid` |

每个夹具与它的源文件**逐字节相同**（测法：`cmp <夹具> <源文件>`，三个都无输出）。

文件名里的 `Nppq` 与 `Nmtrk` 是**实测读数**，不是猜测（见 `real_world_smf.rs` 的断言）。
源文件名里的 `1 tracks` / `2 tracks` 数的是**发声轨道**：实测 `MTrk` chunk 数分别为 1 与 3
（`3 mtrk` 那条的第 0 条 `MTrk` 是 conductor，0 个音符）。

## 3. 未提交的文件（只在本机验证）

`/tmp/midi` 与 `/tmp/musicxml` 里其余文件**未**提交。判定与理由：

| 文件 | 判定 | 理由 |
| :--- | :--- | :--- |
| `THE BEATLES.Blackbird K.mid` | ✗ 不提交 | Beatles 作品受版权保护。 |
| `安静-周杰伦#930y_NNTranscription.mid` | ✗ 不提交 | 周杰伦作品受版权保护；`_NNTranscription` 后缀指向他人转写。 |
| `安静-周杰伦原版-周杰伦.mid` | ✗ 不提交 | 同上。 |
| `call-of-silence-…attack-on-titan…hiroyuki-sawano*.mid` | ✗ 不提交 | 商业动画改编曲，受版权保护。 |
| `Benson Boone - To Love Someone_NNTranscription.mid` | ✗ 不提交 | 商业发行曲目 + 他人转写。 |
| `Spark_Prairie_Fire_…_NNTranscription.mid` | ✗ 不提交 | `_NNTranscription` 后缀指向他人转写；来源未核实。 |
| `result.mid` | ✗ 不提交 | 来源与作者未核实。 |
| `钢琴标准版本.mid` | ✗ 不提交 | 来源与作者未核实。 |
| `明天会更好.mid` | ✗ 不提交 | 来源与作者未核实（同名歌曲为商业发行作品）。 |
| `敢当.mid` | ✗ 不提交 | 来源与作者未核实。 |
| `su_ming_hui_xiang_project.mid` | ✗ 不提交 | 来源与作者未核实；疑似私有工程。 |
| `1151907-20250804042109688fc4b53021d.ccmz.mid` | ✗ 不提交 | 文件名是哈希；来源与作者未核实。 |
| `/tmp/musicxml/**`（全部 12 个文件） | ✗ 不提交 | 见下条：本票不实现 MusicXML；且其中含周杰伦作品。 |

**不确定 ⇒ 不提交。** 上表里标记"来源与作者未核实"的文件，本票无法核实。因此本票不提交它们。

## 4. 本目录没有证明什么

1. 本目录**不**证明这些文件的转写者身份。文件名 `Für Elise 1 tracks` / `2 tracks` 暗示它们是
   **本机派生**的版本（同一份音符内容，两种 PPQ）。本票未能核实派生者是谁。
   若负责人认为这一点构成风险，请指示本票撤回相应夹具。
2. 本目录**不**证明 SMF 的全部语义被支持。夹具只覆盖格式 0 与格式 1、PPQ 384 与 480。
