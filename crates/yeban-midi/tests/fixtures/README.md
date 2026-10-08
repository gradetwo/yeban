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
2. ⛔ 不证明 `.mxl`（ZIP/deflate）可读：**本节**的 7 个文件都是纯文本。
   `.mxl` 的只读导入在**第 7 节**（另一份夹具 + 另一条实现），与本节的 7 个文件无关。
3. ⛔ 不证明完整 MusicXML 4.0 语义：`forward` / `grace` / `unpitched` / `transpose`
   只被**登记为未实现**。
4. ⛔ 不证明上游仓库的**全部**文件可用：本票只取了 5 个文件，且逐个核对了 SHA-256。

## 6. `.mxl`（压缩 MusicXML）夹具与构造配方（本票新增）

本节的读者是 `crates/yeban-midi/tests/musicxml_contract.rs` 的 `mxl_*` 三条判据。
⛔ 本节**不**意味着 `.mxl` 可以被导入：**本节的票**（`a29d280`）只登记**代价与路线**，
**没有**实现 inflate。**导入本身**在**第 7 节**（另一票、另一份夹具、另一个模块）。

### 6.1 为什么需要一个**自造**的 `.mxl`

1. 既有判据 `mxl_zip_bytes_are_rejected_without_panicking` 用的是 **6 个合成字节**
   （`PK\x03\x04` + 手工拼的尾巴）——它不是容器，没有条目、没有 CRC、没有 deflate 数据。
2. 真 `.mxl` 只在**上游文件**里以**受版权保护**的作品出现（第 3 节与第 5.5 节）。
   因此本票的做法是：**把本目录已有的自造纯文本夹具打成容器**。版权仍是本仓库的。
3. 于是判据可以钉住"真容器的形状"（2 个条目 / 压缩法 8 / CRC-32 / 尺寸），
   而**不需要**提交任何外部 `.mxl`。

### 6.2 逐件登记（来源 · 许可 · SHA-256）

| 本目录文件 | 字节 | SHA-256 | 来源 | 许可 |
| :--- | ---: | :--- | :--- | :--- |
| `handmade_mvp_partwise.mxl` | 1435 | `70d3c8abe31258f6e6255e3a6a28ba1204a54ed01dfe79fe3c709aabcd88cd31` | 夜半项目自造（本票）：`handmade_mvp_partwise.musicxml` 的 deflate ZIP 容器 | 本仓库许可 |

容器内有 **2** 个条目：`META-INF/container.xml`（146 → 104 字节，CRC-32 `0xae69681f`）与
`score.xml`（2716 → 1095 字节，CRC-32 `0xcbb005a0`）。`score.xml` 的 CRC-32 与尺寸**等于**
`handmade_mvp_partwise.musicxml` 的 CRC-32 与尺寸 ⇒ 容器的载荷就是**同一份**纯文本夹具
（判据 `mxl_cost_is_pinned_by_the_container_fields_without_inflating` 每次运行都复核这一点）。

⚠️ **与本机那 6 个真 `.mxl` 的已知差异**：真文件（`/tmp/musicxml/**`，**未提交**）的
local header 里 `general purpose bit 11`（UTF-8 文件名标志）为 1（`flags=0x0800`），
本夹具为 0（`flags=0x0000`，python 的 `zipfile` 对纯 ASCII 名字不置该位）。
两者的压缩法（8）与条目布局（`META-INF/container.xml` + `score.xml`）相同。

### 6.3 构造配方（**确定性**：同一份输入 ⇒ 同一份字节）

用标准库 `zipfile` 打包，**固定**压缩级别（9）与时间戳（1980-01-01），并清掉 `external_attr`：

```python
import io, zipfile

CONTAINER = (
    '<?xml version="1.0" encoding="UTF-8"?>\n'
    "<container>\n  <rootfiles>\n"
    '    <rootfile full-path="score.xml">\n    </rootfile>\n'
    "  </rootfiles>\n</container>\n"
).encode()

buf = io.BytesIO()
with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
    for name, payload in (("META-INF/container.xml", CONTAINER),
                          ("score.xml", open("handmade_mvp_partwise.musicxml", "rb").read())):
        info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
        info.compress_type = zipfile.ZIP_DEFLATED
        info.create_system = 0
        info.external_attr = 0
        info.internal_attr = 0
        z.writestr(info, payload)
open("handmade_mvp_partwise.mxl", "wb").write(buf.getvalue())
```

复核：配方**连跑两次**的字节相同；写出的文件与已提交文件的 SHA-256 相同
（`shasum -a 256 handmade_mvp_partwise.mxl` = 上表的值）。

### 6.4 代价与路线的**量法**（数字本身登记在 `src/musicxml.rs` 的未实现清单第 1 条）

| 读数 | 怎么量（单位） |
| :--- | :--- |
| 真 `.mxl` 的条目名 / 压缩法 / 压缩前后字节数 | `python3 -B`：读 local file header（APPNOTE 4.3.7）与 `zipfile` 的 `infolist()`；单位 = **字节** / **条目数** |
| "膨胀结果 == 同名 `.musicxml`" | `zipfile.read("score.xml")` 与同名纯文本文件做 `==`（单位 = 布尔）；**逐件**一次，不是抽样 |
| DEFLATE 块的类型 / 最长匹配 / 最远匹配距离 | 本机手写的 raw-DEFLATE 解码器（`zlib.decompressobj(-15)` 为参照，输出必须逐字节相同才采信）；单位 = **块数** / **字节** |
| 路线 A 的依赖增量 | `cargo tree -p yeban-render --prefix none --offline \| sort -u \| wc -l`：104（无该特性）→ 109（`--features experimental-als-export`）；单位 = **不同的 `name version` 行数** |
| 路线 D 的依赖增量 | 独立探针 crate（`/tmp`，不入库）里 `cargo tree --prefix none --offline \| sort -u \| wc -l`：`zip`（`default-features = false`）= **9** 行（含根）；再加 `deflate-flate2` = **10** 行 |

### 6.5 本节**没有**证明什么

1. ⛔ 不证明 `.mxl` 可读：`src/musicxml.rs` 只吃纯文本字节，判据钉的是**明确拒绝**
   （导入在 `src/mxl.rs` + 本文件第 7 节）。
2. ⛔ 不证明本夹具代表**全部** `.mxl`：它只覆盖 2 个条目、deflate、无 data descriptor、
   无 ZIP64、无加密。本机那 6 个真文件也只是同一台机器上的一个样本。
3. ⛔ 不证明依赖增量的**构建时间或体积**：只数了 crate 条目数（代理指标，已标明单位）。
4. ⛔ 不证明"手写 inflate 是真的可行"：**本节的票**只有本机 Python 原型
   （6/6 与 `zlib` 逐字节相同）；**Rust** 实现由**第 7 节**的票交付。

## 7. `.mxl` 只读导入的夹具与配方（本票新增）

本节的读者是 `crates/yeban-midi/tests/musicxml_contract.rs` 的**导入**判据（`mxl_container_*` /
`mxl_import_*` / `mxl_rootfile_*` / `mxl_limits_*` / `mxl_container_fuzz_*`）与
`crates/yeban-midi/src/mxl.rs`（容器 + 手写 inflate）。

### 7.1 为什么还需要**第二份** `.mxl` 夹具

第 6 节那份（`1435` 字节）解出的两个 DEFLATE 流的**首块类型**由容器自己给出（判据
`mxl_import_readings_are_pinned_by_listing_the_containers` 逐位读数）：

| 第 6 节夹具的条目 | 首块 `BTYPE` | 覆盖的码表路径 |
| :--- | ---: | :--- |
| `META-INF/container.xml`（146 → 104 字节） | **1**（固定 Huffman） | 固定表 |
| `score.xml`（2716 → 1095 字节） | **2**（dynamic Huffman） | dynamic 表 |

⇒ 第 6 节已经覆盖 `BTYPE=1` 与 `BTYPE=2`，但那两个 `BTYPE=1` 的载荷只有 **146 字节**
（解出的 104 字节）。**本节**补一份 `score.xml` 走**固定 Huffman** 的容器
（载荷 **2716** 字节 ⇒ 固定表要跨过更多符号与匹配），使"固定表只在小输入上侥幸可用"这种
错法有判据。

### 7.2 逐件登记（来源 · 许可 · SHA-256）

| 本目录文件 | 字节 | SHA-256 | 来源 | 许可 |
| :--- | ---: | :--- | :--- | :--- |
| `handmade_mvp_partwise_deflate_fixed.mxl` | 1533 | `a65de35df05a11c36cc117f37ad4aa24819314ede57f9c0e0be4377d8cf2c0af` | 夜半项目自造（本票）：`handmade_mvp_partwise.musicxml` 的 **Z_FIXED** deflate ZIP 容器 | 本仓库许可 |

容器内有 **2** 个条目，两个的压缩法都是 **8**（deflate），且**首块都是 `BTYPE=1`**：
`META-INF/container.xml`（146 → 104 字节，CRC-32 `0xae69681f`）与
`score.xml`（2716 → 1193 字节，CRC-32 `0xcbb005a0`）。
`score.xml` 的 CRC-32 与未压缩长度**等于**第 6 节那份纯文本夹具
（⇒ 容器载荷就是同一份字节，与压缩级别无关）。

### 7.3 构造配方（**确定性**：同一份输入 ⇒ 同一份字节）

用 `zlib.compressobj(..., strategy=zlib.Z_FIXED)` 生成 DEFLATE 流，再按 APPNOTE 4.3.x
**手写** ZIP 三节（⚠️ 标准库 `zipfile` **不暴露** `strategy` ⇒ 本配方自己拼字节）：

```python
import struct, zlib
CONTAINER = (
    '<?xml version="1.0" encoding="UTF-8"?>\n'
    "<container>\n  <rootfiles>\n"
    '    <rootfile full-path="score.xml">\n    </rootfile>\n'
    "  </rootfiles>\n</container>\n"
).encode()

def deflate_fixed(data):
    c = zlib.compressobj(level=9, wbits=-15, strategy=zlib.Z_FIXED)
    return c.compress(data) + c.flush()

def build_zip(entries):          # entries: (name, payload, method, compressed_body)
    body, central = bytearray(), bytearray()
    for name, payload, method, blob in entries:
        crc = zlib.crc32(payload) & 0xffffffff
        offset = len(body)
        body += b"PK\x03\x04" + struct.pack("<HHHHHIIIHH", 20, 0, method, 0, 0,
                                             crc, len(blob), len(payload), len(name), 0)
        body += name + blob
        central += b"PK\x01\x02" + struct.pack("<HHHHHHIIIHHHHHII", 20, 20, 0, method, 0, 0,
                                                crc, len(blob), len(payload), len(name),
                                                0, 0, 0, 0, 0, offset)
        central += name
    cd_offset = len(body); body += central
    body += b"PK\x05\x06" + struct.pack("<HHHHIIH", 0, 0, len(entries), len(entries),
                                          len(central), cd_offset, 0)
    return bytes(body)

score = open("handmade_mvp_partwise.musicxml", "rb").read()
open("handmade_mvp_partwise_deflate_fixed.mxl", "wb").write(build_zip([
    (b"META-INF/container.xml", CONTAINER, 8, deflate_fixed(CONTAINER)),
    (b"score.xml", score, 8, deflate_fixed(score)),
]))
```

复核（本票实测，全部为**真**）：配方连跑两次字节相同；`score.xml` 的 DEFLATE 流首块的
`(BFINAL, BTYPE) = (1, 1)`；`zlib.decompress(stream, -15) == score`。

### 7.4 本节**没有**证明什么

1. ⛔ 不证明 `.mxl` 的**全部**形态可读：本节与第 6 节合起来只覆盖 `stored` 与 `deflate`
   两种压缩法、`BTYPE` 1 与 2，以及判据自造的 `BTYPE=0`（`tests/musicxml_contract.rs` 的
   `deflate_stored_block`，**不在**本目录）。
2. ⛔ 不证明 **ZIP64 / 加密 / 非 deflate 压缩法** 的**接受**：这三者只有**拒绝**判据。
   `data descriptor` 的**接受**由**第 8 节**钉住（本票新增）。
3. ⛔ 不证明本夹具代表真实生产者的输出：本机 6 个真 `.mxl`（`/tmp/musicxml/**`，**未提交**）
   与本节的两份都是**不同**生产者的样本，不是全集。
4. ⛔ 不证明 `.mxl` 导入已接入引擎 / MCP / 界面：本 crate 只提供**只读**解析。

## 8. `.mxl` 的两种"形状"夹具：data descriptor 与多块 DEFLATE（本票新增）

本节的读者是 `crates/yeban-midi/tests/musicxml_contract.rs` 的两条判据
`mxl_data_descriptor_container_is_read_from_the_central_directory` 与
`mxl_multiblock_deflate_stream_is_read_to_its_last_block`，以及
`crates/yeban-midi/src/mxl.rs`（边界 7 / 9）与 `crates/yeban-midi/src/mxl/inflate.rs`（多块流）。

### 8.1 为什么需要这两份

第 6 / 7 节的两份夹具与 6 个真 `.mxl` 都不含这两种形状。**实测**（本机，2026-10-09）：

| 读数 | 单位 | 值 | 怎么量 |
| :--- | :--- | ---: | :--- |
| general purpose flag 的 **bit 3** 置位的条目 | 条目 | **0 / 16** | 读每份 `.mxl` 的 local file header（偏移 6）与 central directory 条目（偏移 8）的 flags |
| 本地头的 `(CRC-32, 压缩长度, 未压缩长度)` 与中央目录**相等**的条目 | 条目 | **16 / 16** | 逐条目比本地头偏移 14 / 18 / 22 与中央目录偏移 16 / 20 / 24 |
| 只有**一个**顶层 DEFLATE 块（首块 `BFINAL=1`）的流 | 流 | **16 / 16** | 手写 raw-DEFLATE 块走查（固定表用 RFC 1951 §3.2.6 的码长、dynamic 块先解 §3.2.7 的码长表） |

⇒ 两种形状**只能自造**：`data descriptor` 的读取路径与多块链在已提交语料里**一次都没被接受过**。

### 8.2 逐件登记（来源 · 许可 · SHA-256）

| 本目录文件 | 字节 | SHA-256 | 来源 | 许可 |
| :--- | ---: | :--- | :--- | :--- |
| `handmade_mvp_partwise_data_descriptor.mxl` | 1467 | `e893179d52885d1520a366c5233c10382680fc11c8f32f59dd53aca2e30f0904` | 夜半项目自造（本票）：CPython `zipfile` 在**不可 seek** 的输出上写的 deflate 容器 | 本仓库许可 |
| `handmade_mvp_partwise_multiblock.mxl` | 1487 | `95d3ed7e5be06464cc6bbd1d0cb0b2041d3c3c2fc2bf5dceb7316edd55a22280` | 夜半项目自造（本票）：`score.xml` 走 `zlib.compressobj` + `Z_FULL_FLUSH` 的 deflate 容器 | 本仓库许可 |

两份都是 **2** 个条目、**2/2** deflate，载荷都是 `handmade_mvp_partwise.musicxml`
（`score.xml` 的 CRC-32 `0xcbb005a0`、未压缩长度 **2716** 与第 6 / 7 节一致）。

**`handmade_mvp_partwise_data_descriptor.mxl` 的容器字段**（判据逐项读数）：

| 条目 | 本地头偏移 | 本地头 bit 3 | 本地头 `(CRC, 压缩, 原始)` | 中央头 `(CRC, 压缩, 原始)` | 描述符偏移 |
| :--- | ---: | ---: | :--- | :--- | ---: |
| `META-INF/container.xml`（22 字节名） | 0 | 置位 | `(0, 0, 0)` | `(0xae69681f, 104, 146)` | 156 |
| `score.xml`（9 字节名） | 172 | 置位 | `(0, 0, 0)` | `(0xcbb005a0, 1095, 2716)` | 1306 |

描述符是 **16** 字节 = 签名 `PK\x07\x08` + 3 个 `u32` 小端；条目 0 的描述符在 `[156, 172)`
⇒ 下一条本地头正好落在 172（判据钉住这一对读数）。

**`handmade_mvp_partwise_multiblock.mxl` 的块结构**（`score.xml`，块走查读数）：

| # | `BFINAL` | `BTYPE` | 位区间 | 备注 |
| ---: | ---: | ---: | :--- | :--- |
| 1 | 0 | 2（dynamic） | `0..6684` | 不是最后一块，且结束在**半字节**上 |
| 2 | 0 | 0（stored） | `6684..6720` | `LEN=0`；`align_to_byte` 丢掉 4 位余量 |
| 3 | 1 | 2（dynamic） | `6720..9174` | 最后一块 |

### 8.3 构造配方（**确定性**：同一份输入 ⇒ 同一份字节）

```python
import io, struct, zipfile, zlib

SCORE = open("handmade_mvp_partwise.musicxml", "rb").read()
CONTAINER = (
    '<?xml version="1.0" encoding="UTF-8"?>\n'
    "<container>\n  <rootfiles>\n"
    '    <rootfile full-path="score.xml">\n    </rootfile>\n'
    "  </rootfiles>\n</container>\n"
).encode()

class NonSeekable(io.RawIOBase):        # zipfile 见 seekable()=False ⇒ 改用 data descriptor
    def __init__(self, f): self._f = f
    def writable(self): return True
    def seekable(self): return False
    def write(self, b): return self._f.write(b)

buf = io.BytesIO()
with zipfile.ZipFile(NonSeekable(buf), "w", zipfile.ZIP_DEFLATED) as z:
    for name, payload in (("META-INF/container.xml", CONTAINER), ("score.xml", SCORE)):
        info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))   # ⚠ 缺省是 localtime ⇒ 非确定
        info.compress_type = zipfile.ZIP_DEFLATED
        info.external_attr = 0
        z.writestr(info, payload)
open("handmade_mvp_partwise_data_descriptor.mxl", "wb").write(buf.getvalue())
```

多块那份沿用第 7 节的手写 ZIP 写出器（`build_zip`），只把 `score.xml` 的 DEFLATE 流换成
"两半之间插一次 `Z_FULL_FLUSH`"：

```python
c = zlib.compressobj(level=6, wbits=-15)
half = len(SCORE) // 2
blob = (c.compress(SCORE[:half]) + c.flush(zlib.Z_FULL_FLUSH)
        + c.compress(SCORE[half:]) + c.flush(zlib.Z_FINISH))
# container.xml 的流仍用 c2.compress(CONTAINER) + c2.flush()（单块）
```

复核（本票实测，全部为**真**）：配方连跑两次，两份文件的 SHA-256 与上表相同；
`zlib.decompress(流, -15) == SCORE`（2/2）；多块那份的首块 `(BFINAL, BTYPE) = (0, 2)`。

### 8.4 本节**没有**证明什么

1. ⛔ 不证明**任意** data descriptor 容器可读：只覆盖"bit 3 在本地头与中央头都置位、
   描述符带 `PK\x07\x08` 签名、16 字节"这一种形态；**不带签名**的 12 字节形态与 ZIP64 的
   20 / 24 字节描述符**没有**判据（本模块 ⛔ 不支持 ZIP64）。
2. ⛔ 不证明**任意**多块流可读：只覆盖"dynamic → stored(LEN=0) → dynamic"这一种链；
   其余组合（如 `stored` 开头、两块都是 `stored`、`BFINAL` 链更长）**没有**判据。
3. ⛔ 不证明 6 个真 `.mxl` 里**永远**没有这两种形状：那 6 个文件只是一个样本（
   `/tmp/musicxml/**`，**未提交**）。
4. ⛔ 不证明这两个生产者（CPython / zlib）的**版本间**输出稳定：夹具字节已冻结，
   判据读的是冻结字节，不是"重新跑配方"。
