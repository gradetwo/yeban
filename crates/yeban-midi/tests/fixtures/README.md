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

## 5. MusicXML 夹具（`musicxml` 只读导入票新增）

本节只描述**新增**的 7 个 MusicXML 夹具。它的读者是
`crates/yeban-midi/tests/musicxml_contract.rs`（集成判据）与本机探针
`crates/yeban-midi/examples/musicxml_probe.rs`。

### 5.1 为什么需要两种来源

1. **自造**夹具是判据的**必需**项：一个只读 parser 的判据不该只依赖外部文件。
   自造夹具的版权归夜半项目，可自由提交。
2. **外部**夹具补的是"自造夹具是照着自己的实现写的"这个洞：它的结构由别人（W3C 社区组
   测试套件）决定，因此能暴露"实现只认自己写出来的形状"。

### 5.2 逐件登记（来源 URL · 许可 · SHA-256）

| 本目录文件 | 字节 | SHA-256 | 来源 | 许可 |
| :--- | ---: | :--- | :--- | :--- |
| `w3c_03e_no_divisions.musicxml` | 1212 | `d993c4af8d773d7570ece7f31c17179f58181a301c366358243c27e9aa327bf4` | `w3c-cg/musicxmlTestSuite` @ `77c19f7e819154c70ca1a1992e80dcda8ff82fea` 的 `xmlFiles/03e-Rhythm-No-Divisions.musicxml` | MIT |
| `w3c_21a_chord_basic.musicxml` | 1440 | `c915fc752d4e2e942489e6e499e7c85227a8d939b60957bbd384572fbac078d9` | 同上，`xmlFiles/21a-Chord-Basic.musicxml` | MIT |
| `w3c_33b_spanners_tie.musicxml` | 1386 | `748af2952913be4023f6e43993cc92668fb41f9ca4d639a80c284f85c393f70e` | 同上，`xmlFiles/33b-Spanners-Tie.musicxml` | MIT |
| `w3c_41h_multi_part.musicxml` | 1339 | `674d864d68659bbece5136f2044b93623d9d26bc2b66ce81815dcaaaf27ca596` | 同上，`xmlFiles/41h-TooManyParts.musicxml` | MIT |
| `w3c_43a_piano_staff.musicxml` | 1596 | `820450630ce05ea3bbc53c5ac63c0a8ac6597d7db4c232201d54f0ca162d1503` | 同上，`xmlFiles/43a-PianoStaff.musicxml` | MIT |
| `handmade_mvp_partwise.musicxml` | 2716 | `f4ba1c23ae0d0324d9dcc6fa9cfafca08c693ec193e4da4c2df82d9b5cfd219d` | 夜半项目自造（本票） | 本仓库许可 |
| `handmade_tolerance.musicxml` | 2134 | `501c4f4602f66dd4d2156686bd59467dccc6419917a18a2420b232c63ef60c4f` | 夜半项目自造（本票） | 本仓库许可 |

外部文件的**下载 URL 形状**（`<commit>` = `77c19f7e819154c70ca1a1992e80dcda8ff82fea`）：

```text
https://raw.githubusercontent.com/w3c-cg/musicxmlTestSuite/<commit>/xmlFiles/<原文件名>
```

上游仓库首页：<https://github.com/w3c-cg/musicxmlTestSuite>
（`tests/README.md` 里被 `w3c/musicxml` 记为「the repo `musicxmlTestSuite` was generously
donated by Michael Scott Asato Cuthbert」）。

### 5.3 许可正文（MIT 要求随副本保留版权与许可声明）

上游 `LICENSE`（1090 字节，SHA-256 `1ad02d9267f7874d5b81f29e0bbc833c3e3399cd924a43696fa2a9b9f6b65c4c`）全文：

```text
MIT License

Copyright (c) 2016-2026 Michael Scott Asato Cuthbert

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

### 5.4 复核方式（本表的数字怎么来的）

- 字节数：`stat -f%z <文件>`（**不是** `du`）。
- SHA-256：`shasum -a 256 <文件>`。
- 与上游逐字节相同：`cmp <本目录文件> <上游 clone 的 xmlFiles/原文件名>` 无输出。
- 5 个外部文件都是从**纯文本** MusicXML 里取的，不含二进制块。

### 5.5 只看不取：被排除的来源与理由

| 来源 | 判定 | 理由 |
| :--- | :--- | :--- |
| `w3c/musicxml` 的 `tests/files/*.musicxml` | ✗ 不取 | 该仓库**没有** LICENSE 文件；`README.md` 只写"须签署相应许可协议"⇒ 许可不明。**拿不准就不取。** |
| Mutopia Project（`mutopiaproject.org`） | ✗ 不取 | 该站分发 `.ly` / `.mid` / `.pdf`；本票**未找到**任何 MusicXML 文件。 |
| Wikimedia Commons | ✗ 不取 | 按 `filetype:musicxml` 搜索返回 0 个文件（本票实测）。 |
| OpenScore Lieder（CC0 1.0 ✓） | ✗ 不取 | 该仓库分发 MuseScore `.mscx` 源文件；MusicXML 是**转换产物**（需本机 MuseScore），本机没有该转换器。 |
| `/tmp/musicxml/**`（12 个文件） | ✗ 不取 | 第 3 节已判定：其中含周杰伦等受版权作品；且 `.mxl` 属 ZIP。 |

### 5.6 ⚠️ `/tmp/musicxml` 的文件一个都没提交

`/tmp/musicxml` 的 12 个文件（**6** 个 `.musicxml` + **6** 个 `.mxl`；测法：`ls /tmp/musicxml | grep -c '\.musicxml$'` = 6、`grep -c '\.mxl$'` = 6）**只用于本机验证**
（读数见本票报告），**没有**任何文件被复制进本目录。第 3 节的那条判定**继续成立**。
本节的 7 个文件来自**另外两个**来源（W3C CG 测试套件、夜半自造），与 `/tmp/musicxml` 无关。

### 5.7 本节**没有**证明什么

1. ⛔ 不证明 MusicXML 导入已接入引擎 / MCP / 界面：本 crate 只提供**只读**解析。
2. ⛔ 不证明 `.mxl`（ZIP/deflate）可读：那**没有实现**（见 `src/musicxml.rs` 的未实现清单）。
3. ⛔ 不证明完整 MusicXML 4.0 语义：`forward` / `grace` / `unpitched` / `transpose`
   只被**登记为未实现**。
4. ⛔ 不证明上游仓库的**全部**文件可用：本票只取了 5 个文件，且逐个核对了 SHA-256。
