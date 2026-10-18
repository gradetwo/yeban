# 负责人供体（`logic-donor-owner/`）—— 出处与用途

本目录里的字节**不是第三方材料**，因此**没有**沿用 `../logic-donor/`（MIT，
`jonkubis/logicproformatwriter` 的夹具）那一套许可声明。这里的每一份都由**本仓库负责人**
（Yeban 项目的人类责任人）自己生成并交给本仓库，用于 `ROAD-M4-011`
（实验性 Logic Pro `.logicx` 导出）的**多轨导出**能力。

## 1. 它们是什么

| 路径 | 内容 | 字节数（`stat -f%z`） | sha256 |
| :--- | :--- | ---: | :--- |
| `furelise-1track/ProjectData` | 1 条 MIDI 轨工程的 Logic `ProjectData` | 181,105 | `c18c913ec21ef9eb005638e2fa5008bd912f003ef932611d6f905b50abb52953` |
| `furelise-1track/export.mid` | **同一个工程**由 Logic 自己导出的标准 MIDI 文件 | 3,822 | `da184d44b77c817ae8239b5862ea6a7842a2e240dccc08f2e16a3866e646ae42` |
| `furelise-2tracks/ProjectData` | 2 条 MIDI 轨工程的 Logic `ProjectData` | 227,309 | `cfeabcfc11c5f001edfb48d5711cbccb57944aa23c22bd926483c704dde36db6` |
| `furelise-2tracks/export.mid` | **同一个工程**由 Logic 自己导出的标准 MIDI 文件 | 6,687 | `a80f39556afb851e7af5ee63858113d85484fe644bac63c02c1894f218b88984` |
| `track3-3tracks/ProjectData` | **3 条轨道**工程的 Logic `ProjectData`（第 3 条是 `control track`） | 282,527 | `192d440eccd64943f5fc92d06df7c4f99e0e85c5e2507760b065b77183fe4bb8` |
| `track3-3tracks/export.mid` | **同一个工程**由 Logic 自己导出的标准 MIDI 文件 | 6,788 | `653cf2e3a3a4040390ca0a9be721a192fe4d8b29407c618057693bf52a121414` |
| `track3-3tracks/MetaData.plist` | **同一个工程**的 Logic `MetaData.plist`（只给读回判据用） | 1,806 | `092e5470e7de7ea586868e42e411f626da48a87a58cbdcaf16c59f34b7178e42` |
| `track3-3tracks/ProjectInformation.plist` | **同一个工程**的 Logic `ProjectInformation.plist`（只给读回判据用；**无 `ActiveVariant`**） | 258 | `741cc6775e2662eac659334e9f4b86ab11313e0c68246c892a73010cd464d2fd` |

**生成者**：本仓库负责人（人类责任人）。
**生成工具与版本**：**Logic Pro 12.2**（`Resources/ProjectInformation.plist` 的
`LastSavedFrom` = `Logic Pro 12.2 (6644)`）。
**生成日期**：1 轨 / 2 轨两份 **2026-10-06**；3 轨那份 **2026-10-18**。
**内容**：贝多芬《Für Elise》的两个片段，Logic 里的 region 名分别是 `up:` 与 `down:`；
三份工程是**同一个** Logic 版本、同一台机器存出来的，**轨道数**分别是 1 / 2 / 3。
三份的 `MetaData.plist` 的 `NumberOfTracks` 分别是 **1** / **2** / **3**，
因此这个差别**不是**从别处推断的，而是这些文件自己声明的。

⚠ **3 轨那份的第 3 条轨道没有音符**：它是一条 `control track`（region 名字逐字
`control track`，对象号 `0x001c_0000`）。Logic **自己**导出的
`track3-3tracks/export.mid` 有 **4** 个 `MTrk`：1 条纯 meta 指挥轨 + `control track`
（0 条 note-on）+ `up:`（517）+ `down:`（388）。本仓库把它当作**第 3 个可填的 region 槽**
（它与 `up:` / `down:` 同形：`qeSM kind=5 / subtype=23`、配对 `qSvE kind=1 / subtype=23`、
变长名字 + `rest`），而不是"供体自带第 3 条音符轨"的证据。

## 2. 为什么这些都收

单收 2 轨那份只能得到"一个 2 轨骨架"；**1 轨 + 2 轨两份一起**才让 `1→2` 的**差分判据**
在 CI 上可跑：判据 `owner_donor_differential_matches_the_measured_recipe` 从这两份
**字节本身**重新量出"多一条轨道"这条激活配方（13 条记录 / 46,204 字节，见下表），
因此配方抄错、或常量与文件漂移，判据会红 —— 而不是靠一份口头记录。

| 家族（可读名） | 1 轨 | 2 轨 | 差 |
| :--- | ---: | ---: | ---: |
| `AuCO` | 286 | 286 | 0 |
| `Envi` | 44 | 45 | **+1** |
| `AuCU` | 11 | 19 | **+8** |
| `Trak` | 29 | 31 | **+2** |
| `MSeq` | 17 | 18 | **+1** |
| `EvSq` | 17 | 18 | **+1** |
| 其余 12 族 | — | — | 0 |
| **合计** | **494** | **507** | **+13** |

字节：`227,309 − 181,105` = **46,204**，逐项可归因（详见模块文档
`crates/yeban-render/src/logic.rs` 的"负责人供体对"一节的算术表）。

### 2.1 `2→3` 的差分（`HD-52` / #194）

同样从**两份字节本身**量（`owner_donor_three_track_differential`），格式与上表一致：

| 家族（可读名） | 2 轨 | 3 轨 | 差 | 字节差（含 36-B 记录头） |
| :--- | ---: | ---: | ---: | ---: |
| `AuCO` | 286 | 286 | 0 | +16（内容） |
| `Envi` | 45 | 46 | **+1** | +519 |
| `AuCU` | 19 | 22 | **+3** | +54,211（结构 48,672 + 内容 5,539） |
| `Trak` | 31 | 33 | **+2** | +130 |
| `MSeq` | 18 | 19 | **+1** | +353（结构 347 + 工程名 6） |
| `EvSq` | 18 | 19 | **+1** | +292 |
| `Song` | 1 | 1 | 0 | +40（内容） |
| `GenM` | 1 | 1 | 0 | −343（内容） |
| 其余 10 族 | — | — | 0 | 0 |
| **合计** | **507** | **515** | **+8** | **+55,218** |

**怎么剔除内容差异**（逐条说明）：

1. 两份工程的前两个 region（`up:` / `down:`）的 `qSvE` 音符载荷**逐字节相同**
   （16,592 + 12,448 字节）⇒ 音符内容**没有**参与这条差分，家族**条数**差只可能来自
   第 3 条轨道的结构记录。
2. 条数差因此就是结构增量：`Trak` +2、`Envi` +1、`MSeq` +1、`EvSq` +1、`AuCU` +3。
3. 字节差里，**结构**部分逐条可归因 = **49,960**：`Trak` +2 = 130（1 条 58 字节编排行 94
   + 1 条 0 字节 `Trak` 36）、`Envi` +1 = 519、`EvSq` +1 = 292（`control track` region
   176+36=212 加编排放置 `qSvE` 176→256 的第三条摆放事件 +80）、`MSeq` +1 = 347
   （`control track` region 311+36）、`AuCU` +3 = 48,672（192 / 256 / 48,116 + 36）。
4. 其余 **5,258** 字节是内容差异、已逐项对账：`Song` +40、`GenM` −343、
   `MSeq` 工程名（`r Elise 2 tracks` → `track3`）+6、`AuCO` +16、既有 `AuCU` 载荷漂移
   +5,539（例：3,422 → 3,191、1,921 → 1,431）。它们描述的是"两份工程不是同一个工程"，
   而不是"多一个槽"。
5. `AuCU` +3 的**条数**本身就是内容相关的（通道条装了几个组件），所以它只在
   "这份 3 轨供体"上成立；1→2 那份普通乐器轨是 +8。两段的**结构核**相同（`Trak` +2、
   `Envi` +1、`MSeq` +1、`EvSq` +1），差的是 `AuCU` 的内容。

## 3. 它们**不是**什么

* **不是** Apple 的演示工程：`/Library/Application Support/Logic/...` 下的任何字节都
  **没有**进本仓库，读那些路径的判据一律"路径不存在即 skip"。
* **不是** MIT 材料：本目录**不**附 `LICENSE`；它不是第三方作品，不存在需要随文件分发的
  上游许可。分销本仓库即分销负责人的这份工作产物。
* **不含**音频素材：`.logicx` 的 `Media/` 一个字节都没收；`WindowImage.jpg`、
  `DisplayStateArchive`、`Undo Data.nosync` 也没有收（它们是不透明二进制/缩略图）。
  3 轨那份的 `Media/` 里有约 275 MB 的 Alchemy / Studio Grand Piano 采样，**一个字节都没收**。
* `.mid` 三份是 Logic **自己**导出的，用作**独立的**交叉核对（轨数、通道、音符数），
  **不是**本仓库写入器的产物。

## 4. 收进来的字节数

**8** 个二进制文件 = `181,105 + 3,822 + 227,309 + 6,687 + 282,527 + 6,788 + 1,806 + 258` =
**710,302** 字节（＋本说明文件）。单文件最大 **282,527** 字节，远低于 AGENTS.md §2 红线 9 的
10 MB 单文件上限。

⚠ `track3-3tracks/MetaData.plist` 与 `ProjectInformation.plist` 是**供体自己的**元数据，
**不是**本模块的产物：它们只用于"用生产读取器读回"的判据（`logic_read` 需要
`MetaData.plist` 的七个必需键，其中 `SongKey` 是本模块刻意不写的；而这份
`ProjectInformation.plist` **没有 `ActiveVariant`**，正好钉住读取器的目录唯一性回退）。
它们**不**随 `.logicx` 导出。
