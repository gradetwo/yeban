# 负责人供体对（`logic-donor-owner/`）—— 出处与用途

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

**生成者**：本仓库负责人（人类责任人）。
**生成工具与版本**：**Logic Pro 12.2**（`Resources/ProjectInformation.plist` 的
`LastSavedFrom` = `Logic Pro 12.2 (6644)`）。
**生成日期**：**2026-10-06**。
**内容**：贝多芬《Für Elise》的两个片段，Logic 里的 region 名分别是 `up:` 与 `down:`；
两份工程是**同一个** Logic 版本、同一台机器、同一天存出来的，**唯一的差别是轨道数**
（1 条 vs 2 条）。两份的 `MetaData.plist` 的 `NumberOfTracks` 分别是 **1** 与 **2**，
因此这个差别**不是**从别处推断的，而是这两份文件自己声明的。

## 2. 为什么两份都收

单收 2 轨那份只能得到"一个 2 轨骨架"；**两份一起**才让**差分判据**在 CI 上可跑：
判据 `owner_donor_differential_matches_the_measured_recipe` 从这两份**字节本身**
重新量出"多一条轨道"这条激活配方（13 条记录 / 46,204 字节，见下表），
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

## 3. 它们**不是**什么

* **不是** Apple 的演示工程：`/Library/Application Support/Logic/...` 下的任何字节都
  **没有**进本仓库，读那些路径的判据一律"路径不存在即 skip"。
* **不是** MIT 材料：本目录**不**附 `LICENSE`；它不是第三方作品，不存在需要随文件分发的
  上游许可。分销本仓库即分销负责人的这份工作产物。
* **不含**音频素材：`.logicx` 的 `Media/` 一个字节都没收；`WindowImage.jpg`、
  `DisplayStateArchive`、`Undo Data.nosync` 也没有收（它们是不透明二进制/缩略图）。
* `.mid` 两份是 Logic **自己**导出的，用作**独立的**交叉核对（轨数、通道、音符数），
  **不是**本仓库写入器的产物。

## 4. 收进来的字节数

新增 **4** 个二进制文件 = `181,105 + 3,822 + 227,309 + 6,687` = **418,923** 字节
（＋本说明文件）。远低于 AGENTS.md §2 红线 9 的 10 MB 单文件上限。
