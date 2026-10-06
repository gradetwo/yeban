# `logic-donor` — 供体模板（第三方，MIT，**随仓库分发**）

> **一句话**：这里是 `jonkubis/logicproformatwriter` 的 Logic 夹具 `F0_baseline.logicx`
> 的**原样副本**，用来给实验性 Logic Pro 导出器提供一份 **Logic 自己存过的**通道簇 / 轨道表 /
> `gnoS` 注册表 —— 参考实现（同源，MIT）的实测结论是这些结构**不能凭空合成**（§10.6.1）。

## 1. 来源与许可（逐字）

- **上游仓库**：<https://github.com/jonkubis/logicproformatwriter>
- **上游路径**：`fixtures/F0_baseline.logicx`
- **许可**：**MIT**（上游根目录 `LICENSE`，**1,066 字节**，全文原样放在同目录
  [`LICENSE`](LICENSE)）
- **版权行（原样）**：`Copyright (c) 2026 Jon Kubis`
- **下载方式**：`raw.githubusercontent.com` 上的原始文件（不是打包产物、不是转码结果）
- **本目录里的文件是上游文件的字节副本**：判据
  `vendored_donor_matches_the_embedded_bytes_and_carries_its_licence`
  （`crates/yeban-render/src/logic.rs`）把 `Alternatives/000/ProjectData` 读回来与
  `include_bytes!` 嵌入的那一份逐字节比较，并断言 `LICENSE` 含 `MIT License` 与
  `Copyright (c) 2026 Jon Kubis`、本文件含上游仓库名与供体的 sha256。

## 2. 逐文件清单（大小口径 = `stat -f%z`，单位：字节）

| 包内路径 | 字节数 | sha256 |
| :--- | ---: | :--- |
| [`Alternatives/000/ProjectData`](Alternatives/000/ProjectData) | **127,689** | `8a5ec7371e89f07fa53e873725c856bbe29eeef1dac14ff54df8b23962dde893` |
| [`Alternatives/000/MetaData.plist`](Alternatives/000/MetaData.plist) | **576** | `0062fdbbf9b00925f6e3d0854d59eb64a5a53641c8c14afe4c168b5ed9515900` |
| [`Resources/ProjectInformation.plist`](Resources/ProjectInformation.plist) | **264** | `2916b2ced85893d8489eccaa1e2862e8ab2533bbd3553f127d25abdce1a8c8c9` |
| [`LICENSE`](LICENSE) | **1,066** | `38ecea03ed94eb49490db4a362f889c61d1435a1a7877e34ce2b6bd25c110548` |

**合计 129,595 字节**（不含本文件）。四个文件都是上游字节的**逐字节副本**；
`Alternatives/000/ProjectData` 是唯一被 `include_bytes!` 嵌进 `yeban-render` 的那一份
（另外三个只被判据读，用来核对供体的 `NumberOfTracks` 与包内布局）。

`Alternatives/000/MetaData.plist` 是上游夹具的 `MetaData.plist` 直接放在
bundle 相对位置上（上游把 `MetaData.plist` 与 `ProjectInformation.plist` 平铺在 fixture
目录里；这里按 `.logicx` 的**真实包内布局**归类，内容一个字节没动）。

## 3. 改了什么

**磁盘上：什么都没改**（`cmp` 逐字节相同，见上表的 sha256）。

导出器在**内存里**改四条记录（不改文件、不回写本目录）：

1. 全局拍号 `qSvE`（对象号 `0`，载荷首字 `0x30`）⇒ 载荷 `+0x0b`（分母指数）/ `+0x0c`（分子）；
2. 全局速度 `qSvE`（对象号 `0`，载荷首字 `0x60`）⇒ 载荷 `+0x10` 的 `u32` = `round(bpm × 10000)`；
3. 被摆放的 MIDI region `qeSM`（对象号 `0x00e40000`）⇒ 名字字段（载荷 `+0x10` 长度 / `+0x12` 名字，
   **原地**写，载荷长 305 不变，因此名字之后第一个非零字段 `+0x57` 及其后的字节一个都不动）；
4. 该 region 的配对音符 `qSvE`（同对象号）⇒ 载荷换成我们的音符（`32·N + 16`），
   **记录头逐字节保留供体的**。

供体根头的版本码 `0x09CF` **保留**（版本码声明的是这份文档的落盘格式，供体记录是 2511 形态）。
其余 **523 / 527** 条记录逐字节是供体的，并**逐族**登记在导出器的损失表里
（见 `crates/yeban-render/src/logic.rs` 的 `LogicBuilder::write_donor_losses`）。

## 4. 为什么是它，而不是 Apple 的演示工程

本机 `/Library/Application Support/Logic/Logic Pro X Demosongs/` 下的 Apple 演示工程是
**有版权的**：本仓库**不提交、不内嵌、不复制**它们，任何判据读它们都**只在路径存在时才跑**
（不存在即打印 skip）。而本目录这一份是 **MIT**，许可允许随仓库分发（保留版权与许可声明即可），
因此它成为唯一**可以进仓库**的 Logic 存过的 donor。

## 5. 诚实边界

- **本目录不声称 Logic Pro 能打开任何产物。** 负责人已用本机 Logic Pro 12.2 打开过导出产物
  三次：第一次报 "Logic 4 format (or earlier)"，第二、三次是同一个通用失败对话框。
  供体路线改变的是"结构从哪来"，不是"已经能打开"。
- `ProjectData` 的内部语义**没有**被本仓库完整反推：哪些字段是载荷、哪些是缓存、哪些是
  Logic 每次保存重算的，本目录不回答。参考实现的 `PROJECTDATA_FORMAT.md`（92,606 字节）是
  它自己的再工程记录，本仓库只读它的结论、不拷它的代码、不新增任何依赖。
