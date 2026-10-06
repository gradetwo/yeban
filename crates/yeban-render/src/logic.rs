//! 实验性 Logic Pro (`.logicx`) 导出 [ARCH-FMT-002] [ROAD-M4-007]。
//!
//! 本模块只在**非默认** feature `experimental-logic-export` 下存在
//! （见 `crates/yeban-render/Cargo.toml` 的 `[features]`）。AGENTS.md §2 红线 6 要求
//! 实验性导出不得进默认 release 构建，因此 `default` 保持为空：默认构建**既不编译本模块，
//! 也不新增任何依赖边**（本 feature 没有可选依赖 —— 见下面的"为什么不用 plist crate"）。
//!
//! ## 一个 `.logicx` 是**目录**，不是文件
//!
//! 实测（本机两个演示工程）：`.logicx` 是一个 bundle 目录，音乐住在
//! `Alternatives/<NNN>/ProjectData`（自定义分块二进制）与 `Alternatives/<NNN>/MetaData.plist`
//! （标准 plist）里；`Media/` 可能上 GB 而且本切片用不到。因此 [`build_bundle`] 产出
//! **路径 → 字节** 的映射；**建目录与落盘是调用方的事**（与 `als` 模块"落盘是调用方的事"
//! 同款纪律）：app 层的 `export_logic` 负责建目录并走同一份原子落盘实现。
//!
//! ## 实测的字节布局（`ProjectData`）
//!
//! | 事实 | 读数 |
//! | :--- | :--- |
//! | 根魔数 | `23 47 C0 AB` 在偏移 0 |
//! | 根头长度 | **0x18** = 24 字节 |
//! | **工程格式版本码** | `u16` **小端** 在偏移 **0x04**（见 [`LOGIC_FORMAT_VERSION_CODE`]） |
//! | 根头 0x06..0x0f | 十个**恒定**字节 `03 00 04 00 00 00 01 00 08 00` |
//! | 声明载荷长度 | `u32` **小端** 在偏移 0x10（= 文件长度 − 0x18） |
//! | 第一个 chunk 名 | 偏移 **0x18**（不是 0x14） |
//! | chunk 名存放 | **小端**：`Song` 落盘为 `gnoS`（0x67 0x6e 0x6f 0x53） |
//! | 记录头长度 | **0x24** = 36 字节（chunk 名在 +0，kind 在 +4，subtype 在 +6，cluster 在 +8，载荷长度 `u32` 小端在 +0x1c） |
//! | 记录头恒定字 | +0x16 = 2、+0x18 = 0、+0x1a = 2（格式版本码 ≥ 2509；≤ 2507 时 +0x1a 为 1） |
//! | 事件行长度 | **16** 字节；第 7 字节最高位 = 1 表示"续行"，0 表示"开新事件" |
//! | 音符字段 | 头行 +4 = 起始 tick，+0x0b = 力度，+0x0c = 音高；时值在**第一条续行**的 +0x0c |
//! | 速度 | `gnoS` 载荷在 0x3a6（权威槽）与 0x92（回退槽）存 `round(bpm × 10000)` |
//! | 拍号 | `qSvE` 载荷 +0x0b = 分母的以 2 为底指数，+0x0c = 分子 |
//! | region（`qeSM`）名字 | 载荷 **+0x10** = `u16` **小端字节数**，**+0x12** 起 UTF-8 名字，其后 4 字节 0（记录内 +0x34） |
//! | `gnoS` 载荷开头 | 嵌套 `#G` 子帧：`23 47 C0 AB` + 版本码 + `18 00 11 00` |
//!
//! ⚠️ **与简报/账本第 391 轮的一处偏差（实测更正）**：账本写"偏移 0x14 是 `gnoS`"，
//! 但本机两个演示工程（`Swing!.logicx/Alternatives/004`、`ocean eyes.logicx/Alternatives/001`）
//! 的实测都是 **0x18**：0x14 处是 4 个 0 字节，`gnoS` 紧跟在 0x18。0x18 也正是 groove
//! 读取器 `src/data/logicToArrangement.ts:369` 的 `let offset = 0x18;`。本模块按实测的
//! 0x18 实现。
//!
//! ⚠️ **账本第 402 轮记的"region 名在载荷 +0x34、真实文件在 +0x10/+0x12"不是字节差异，
//! 是参照系混淆**。实测两例演示的 **790/790** 条 `qeSM`（`ocean eyes` 245 条 /
//! `Swing!` 545 条）在载荷 `+0x10` 都是 `u16` 小端 = 名字的 **UTF-8 字节数**、`+0x12` 起是
//! 名字、名字后 4 字节全 0；本写入器产物的 region 载荷实测同样在 `+0x10`/`+0x12`（旧常量
//! `LOGIC_REGION_NAME_OFFSET = 0x34` 是**记录**内偏移，写入时减去 0x24 才落到载荷 `+0x10`，
//! 但它的文档曾写成"载荷偏移"，这正是误读的来源）。现在常量
//! [`LOGIC_REGION_NAME_PAYLOAD_OFFSET`] 只以载荷为参照系，并用实测字面量钉住。名字字段
//! **因此不登记**为偏差；载荷**其余部分**（+0x00 起 `2e 03`、长度 296..324、`ocean eyes`
//! 的部分记录在名字后还有一份空格前缀的副本）仍未重建，登记在 [`REGION_PAYLOAD_CAVEAT`]。
//!
//! ## 版本码为什么必须非零（外部测量）
//!
//! 负责人把本模块的产物交给 **Logic Pro 12.2**（本机安装的就是 12.2），得到的对话框是
//! *"The song you are trying to open is in **Logic 4 format (or earlier)**"*
//! （`com.apple.logic10 error 100`）。这说明 Logic **认出了**这是一份 Logic 工程，但把
//! 根头 `+0x04` 读成了最老的格式 —— 本写入器原先在那里写 0。现在写的是本机实测到的最新
//! 版本码（[`LOGIC_FORMAT_VERSION_CODE`] = `0x09D0`，来自本机 `Logic Pro 12.0.1` 存过的
//! 工程）。**这仍然不等于自研（synthesised）写入器的产物能被打开**：它被拒绝过两次
//! （账本第 403、405 轮；第 401 轮是更早的 "Logic 4 format (or earlier)" 拒绝）；
//! 被实测打开的只是 [`build_bundle_from_donor`] 的**供体拼接**产物
//! （本机 **Logic Pro 12.2**，负责人实测，2026-10-06）。
//!
//! ## 参考实现（**翻译逻辑，不抄代码**）
//!
//! 同类项目的写入器在 `/Users/crow/work/music/groove`（TypeScript）：
//!
//! * `mcp/arrangement.ts:1030` `exportMcpLogicProject` —— 一次导出返回哪几个文件；
//! * `mcp/arrangement.ts:1053` `importMcpLogicProject` + `src/data/logicToArrangement.ts`
//!   （`readRecords` 在 `:366`、事件行模型在 `:447`、`readTempo`/`readMeter` 在 `:600` 之后）；
//! * `src/data/arrangementToLogic.ts`（`songRecord` / `meterRecord` / `tempoRecord` /
//!   `regionRecord` / `logicNoteLines` / `arrangementToLogicFiles` / `logicProjectBundle`）；
//! * `mcp/registryProject.ts`（59 行工具注册）。
//!
//! 这些常量与记录形状**逐条**对照过 groove 的写入器与其读取器（互为逆运算）；
//! 唯一**有意**的差异是记录顺序：groove 写 `qSvE`(拍号) → `qSvE`(速度) → `gnoS`，
//! 本模块把 `gnoS` 放在**最前**，因为实测的真实工程第一个 chunk 就是 `gnoS`
//! （groove 的读取器按 chunk 名查找记录，与顺序无关）。
//!
//! ## 与真实工程的 chunk 家族对账（实测差集）
//!
//! 诊断判据 `logic_chunk_families_compare_side_by_side_when_the_demos_are_present`
//! 把本机两个 Apple 演示工程的 `ProjectData` 与本模块的产物
//! **逐 chunk** 并排列出（容器是**扁平的** chunk 流：两个真实文件与产物里都**没有**嵌套
//! chunk 标识符；`gnoS` 载荷虽以根魔数 `#G` 开头，其后并不是 chunk 流）。实测结果：
//!
//! * **真实工程（两例并集）= 27 个家族**，产物 = **3 个**（`Song` / `EvSq` / `MSeq`）；
//! * 差集 = **24 个**家族，即 [`MISSING_CHUNK_FAMILIES`]：`AFld`、`AuCO`、`AuCU`、`AuCn`、
//!   `AuEv`、`AuFl`、`AuRg`、`Clip`、`CorM`、`Envi`、`GAdd`、`GenM`、`Grid`、`Hypr`、
//!   `InSt`、`Layr`、`ScSt`、`SngO`、`Styl`、`Trak`、`Trns`、`TxSq`、`TxSt`、`Vide`；
//!   每次导出为每一个家族登记一条 `未映射:`（理由里点名 chunk；只有 `Trak` 与 `AuRg` 的定位
//!   有仓库内依据，其余 22 个明写"用途未证实"）。
//! * 反向（真实工程**没有**而产物有）在家族层面为空；但**字段层面**仍有两处实测偏差：
//!   记录头 +0x08..+0x14 的簇号与 `0xFFFF` / `0xFFFF0000` 哨兵（实测随格式版本变化）、
//!   region 的 `qeSM` / 音符 `qSvE` 的 subtype（与配对序列共享的序列号，无编号规则可推），
//!   以及 `gnoS` 载荷里嵌套 `#G` 子帧除实测前缀外的内容 —— 登记为
//!   [`CONTAINER_HEADER_CAVEAT`]（`非等价:`）；region 载荷里**除名字字段以外**的字节
//!   （`+0x00` 起 `2e 03`、载荷长 296..324、`ocean eyes` 的部分记录在名字后还有一份
//!   空格前缀副本）没有重建 —— 登记为 [`REGION_PAYLOAD_CAVEAT`]（`非等价:`）。
//!   根头的版本码与十个恒定字节、记录头 kind@+4 / +0x16 / +0x18 / +0x1a、`gnoS` 的
//!   subtype（`0xFFFF`）、拍号/速度 `qSvE` 的 subtype（1 / 3）、`gnoS` 的 `#G` 前缀，
//!   以及 region 的**名字字段**（载荷 +0x10/+0x12）
//!   **已按实测写入**，因此**不再**登记为偏差。
//!
//! 判据（`crates/yeban-render/src/logic.rs` 的 `tests`）：一个**无头确定性**判据断言
//! "写入家族 == [`WRITTEN_CHUNK_FAMILIES`]"且"差集逐条有损失条目"，另一个**只在两个本机
//! 演示工程都存在时**才跑的对账判据（不存在就 skip，CI 不受影响）断言实测并集 ==
//! [`MEASURED_REAL_CHUNK_FAMILIES`]；region 名字字段另有一条**用实测字面量**钉住的判据
//! （`region_name_field_sits_at_the_measured_payload_offsets`，含非 ASCII 名字以证明长度是
//! **字节数**），以及一条逐条核对真实 `qeSM` 名字字段的可选判据
//! （`real_demo_qesm_records_use_the_measured_name_field_when_present`，路径不存在即 skip）。
//!
//! ## 参考实现之二：`jonkubis/logicproformatwriter`（MIT）—— 本轮拿到了**语义**
//!
//! 第 403 轮把"下一步"写成一句话：去读开源参考 `jonkubis/logicproformatwriter`。那一轮网络可用，
//! 从 `raw.githubusercontent.com` 取回了它的 `PROJECTDATA_FORMAT.md`（**92,606 字节**，按
//! `stat -f%z` 口径）与它的 Logic **fixture** `fixtures/F0_baseline.logicx`（`ProjectData`
//! **127,689 字节**），放在 **`/tmp/logic-ref/`**。
//!
//! ⚠ **本轮（donor 路线）更正了上一轮的处置**：那份 fixture **不再只是临时材料** —— 它是
//! **MIT** 许可，允许随仓库分发（保留版权与许可声明），因此被**原样**收进
//! `crates/yeban-render/assets/logic-donor/`（连同上游 `LICENSE` 全文与出处说明 `README.md`）。
//! `PROJECTDATA_FORMAT.md` **仍然不进仓库**（它又大又是文档，本模块只引用它的短句结论）。
//! Apple 的演示工程**照旧不提交、不内嵌、不复制**。
//!
//! * **来源**：`https://github.com/jonkubis/logicproformatwriter`
//!   （`PROJECTDATA_FORMAT.md`、`fixtures/F0_baseline.logicx`）。
//! * **许可**：**MIT**（仓库根 `LICENSE`，1,066 字节，MIT 全文；原样放在供体目录里）。
//! * **用法**：语义与常量**只读**、短句引用；**没有拷贝它的代码、没有新增依赖**。
//!   donor 路线把它的夹具当作**数据**（`include_bytes!`）而不是代码。
//!
//! ### 三个语义问题的答案（参考实现 + 本机五份真实文件复核）
//!
//! 参照系：`F0` fixture（Logic 12.0.1）+ 本机四份工程（`Swing!` 2507、`ocean eyes` 2000、
//! `quiet` 2512、工厂模板 `01 Hip Hop` 2512）。**"五份全中"** 都指这五份。
//!
//! 1. **region（`qeSM`）怎么指向它所属的轨道？—— 不指向。** region 记录头 `+0x08` 是它
//!    **自己的对象号**（参考实现 §8.5：一个 MIDI region **就是**一条带自己 track-cluster
//!    index 的 `qeSM`，例 `0x1c0000`，与它自己的 `karT`/`qSvE` 同簇）。"属于哪条轨道"存在
//!    **轨道的** `qSvE` 里，是一条**摆放事件**：类型记号 `20`(MIDI)/`24`(音频)、位置 @+0x04
//!    = 34560 + tick、链接 id @+0x10（`0x58 + 序号×4`，与 region `qeSM` 的 `+0x108` 相同）、
//!    **1 起的轨道号 @+0x14**、region 链接 @+0x2c。位置**另存一份**在 region `qeSM` 的
//!    `+0x11c`（零起 tick），长度在 `+0x78`。本切片两处都没写 ⇒ 登记为
//!    [`REGION_PLACEMENT_UNMAPPED`]（`未映射:`）。
//! 2. **非空 `Trak` 载荷是什么、要几条、怎么排序？** 是 **Track 对象**，按记录头 `+0x08`
//!    的对象号分**三个子群**（上一轮"条数 799/753/118/179 与轨道数 76/42/10/34 不相等"的
//!    谜底就是这个：这一族不是一张表，而是三个子群；其中编排轨行那一群恰好是
//!    `NumberOfTracks + 1`）：
//!    * `0x00040000` = **编排轨行**（arrange Track row）。实测**非 0 载荷条数 − 1 恰等于
//!      `NumberOfTracks`，五份全中**（`F0` 2−1=1、`Swing!` 77−1=76、`ocean eyes` 43−1=42、
//!      `quiet` 11−1=10、`01 Hip Hop` 35−1=34）；多出来的那条是 **master 行**（载荷
//!      `+0x00 == 3`，载荷 `+0x08` 指向通道槽 `0x50`；其余行的 `+0x00 == 1`）。顺序 = 记录头
//!      `+0x12` 的单字节序号 `0..N-1`，master 排在最后（参考实现 §10.6.3(d)：新行插在 master
//!      之前，并把 master 的序号抬到 `T+1`）—— 实测 `quiet`/`01 Hip Hop` 的该组序号正是
//!      `0..=N` 连续。
//!    * `0x00080000` = **每个预分配混音槽一条 Track 对象**；载荷 `+0x08` = 槽号，记录头
//!      `+0x12` = 名次，**流的顺序就是轨道表顺序**（参考实现 §10.6.4 gate 4：名次设好后
//!      **必须按 `+0x12` 升序重排记录**）。实测 `quiet` 该组 40 条非 0、槽号互不相同。
//!    * 其余对象号下还有少量 `Trak`，多数载荷为 0。
//! 3. **记录头 `+0x08..+0x0b` 的"簇号"是什么？** 是**属主对象的 `u32` 对象号**
//!    （参考实现 §8.1/§10.6.2：`trackIndex × 0x40000`，等价于槽字节 `<< 16`；`ivnE` 的
//!    `idx` 例如 `0x580000` ⇒ 槽 `0x58`）。小的号是**保留的内部对象**：`0x00000000` 根/工程、
//!    `0x00040000` 编排、`0x00080000` 自动化根文件夹、`0x000c0000` Track Alternatives、
//!    `0x00100000` Global Harmonies …；`0x00480000` 以上是混音通道槽。**实测五份全中**：
//!    每条 `qeSM` 与它**配对的 `qSvE` 共享同一个对象号与同一个 subtype**（`F0` 13/13、
//!    `Swing!` 545/545、`ocean eyes` 245/245、`quiet` 65/65、`01 Hip Hop` 75/75）。
//!    它是**唯一**随记录变化的记录头字段（其余恒定：`+0x0c..0x0f` = `00 00 ff ff`、
//!    `+0x0e..0x11` = `ff ff ff ff`、`+0x16` = 2、`+0x18` = 0、`+0x1a` = 2/1）。
//!
//! ### 2026-10-06 本轮：轨道**来自供体**（donor）—— 负责人裁决选项 A
//!
//! 上一轮把"为什么仍然不写 `Trak`"换成了参考实现自己的结论：轨道**不能凭空合成**
//! （§10.6.1 原话：新建通道会触发 Logic 的混音器 / CoreMIDI Environment **扩张**，重新生成
//! time-UUID 并重排整个 `OCuA` 通道块，"pervasive and impractical to reproduce"）。它自己通过
//! Logic 验证的做法，是克隆一份 **Logic 存过的** donor 模板的通道簇。本轮负责人裁决走这条路
//! （选项 A），并且**换掉了"没有可再分发的 donor"这个前提**：
//!
//! * 参考实现自己的 Logic 夹具 `F0_baseline` 是 **MIT**（"Copyright (c) 2026 Jon Kubis"），
//!   许可允许随仓库分发。它被**原样**放进 `crates/yeban-render/assets/logic-donor/`
//!   （**129,595 字节**：`ProjectData` 127,689 + `MetaData.plist` 576 +
//!   `ProjectInformation.plist` 264 + 上游 `LICENSE` 1,066；另有出处说明 `README.md`），
//!   由 [`LOGIC_DONOR_PROJECT_DATA`] 用 `include_bytes!` 嵌进本（**非默认 feature 门控的**）模块。
//! * Apple 的演示工程**照旧不提交、不内嵌、不复制**；本机判据读它们时仍然**路径不存在即 skip**。
//!
//! ### donor 路线做了什么、没做什么
//!
//! [`project_data_from_donor`] / [`build_bundle_from_donor`] 以供体的 **527 条记录**为骨架，
//! **只改 4 条**（见 [`LOGIC_DONOR_PATCHED_RECORDS`]）：全局拍号 `qSvE`、全局速度 `qSvE`、
//! 被摆放 region 的 `qeSM` 名字、该 region 的配对音符 `qSvE`。其余 **523** 条**逐字节是供体的**
//! —— 包括 `ivnE`（12 条环境对象）、`OCuA`（376 条混音条）、`MneG`（1 条 Session-Player 状态）、
//! `karT`（22 条轨道对象）与 `gnoS` 正文（10,756 字节）。这条判据叫
//! `donor_cluster_is_carried_byte_for_byte_and_only_four_records_change`。
//!
//! **明确没做**（因此都进了损失表）：
//!
//! * **没有**按参考实现 §10.6.3 的 (a)–(f) 去**激活新的通道槽**：供体只携带
//!   [`LOGIC_DONOR_TRACK_COUNT`]（= 1）条编排轨行，因此本路线最多映射**一条** MIDI 轨的
//!   **第一个** MIDI 摆放；其它轨道整条登记为 `未映射:`，`MetaData.plist` 的 `NumberOfTracks`
//!   写的是**实际映射数**（不是工程轨道总数 —— 参考实现 §10.6.6 记这个数不符会让 Logic 拒绝）。
//! * **没有**改 `gnoS` 里内嵌的初始速度三连：供体是 **compact** `gnoS`（10,756 字节），
//!   参考实现给 settled 模板的槽位 `+0x92`/`+0xEA`/`+0x3A6` 在它里面**全是 0**（布局不同），
//!   而参考实现 §10.4 自己记 compact base 的速度/拍号写入器**只动独立的 `qSvE`**。
//!   实测的内嵌三连在载荷 `+0x6f`/`+0x73`/`+0xc6`（值 = 供体自己的 120 BPM），**原样保留**并登记。
//! * **没有**动供体的摆放（第 1 小节）与 region 自身起点：音符按 `38400 + 绝对 tick` 写进 region
//!   的 `qSvE`（实测语义：音符位置是 region 相对的，region 的绝对位置只由摆放与 `qeSM +0x11c`
//!   表达）。
//! * **打开结论（实测，勿外推）**：本机 **Logic Pro 12.2** 能打开这条**供体拼接**产物
//!   （负责人，2026-10-06）；自研（无供体）写入器的产物被拒绝过两次（账本第 403、405 轮）。
//!
//! 供体路线的损失表由 `LogicBuilder::write_donor_losses` 逐族产出；自研路线的
//! [`TRACK_OBJECTS_UNMAPPED`] 仍然描述**自研**产物（它一条 `karT` 都不写）。
//!
//! ### 参考实现同时暴露了本写入器**四处未登记的缺陷**，本轮改正
//!
//! 那四处都在**本写入器已经会写的**记录里，因此不需要新结构就能修：
//!
//! 1. **拍号 `qSvE` 载荷缺 16 字节尾**：原先写 80 字节；参考实现（§5）与实测（无变化工程
//!    都是 **96** 字节）要求 80 字节头 + 16 字节尾。已改，见 [`LOGIC_METER_PAYLOAD_LEN`]。
//! 2. **速度 `qSvE` 载荷缺 32 字节正文与 16 字节尾**：原先写 **16** 字节 —— 只有第一个字
//!    `0x60`、其余全 0，即**位置 0、速度 0**。实测五份文件的全局速度序列**全是 48 字节** =
//!    32 字节事件 + 16 字节尾，事件里的速度字等于各工程 `MetaData.plist` 的 BPM
//!    （120/115/145/120/70 BPM ⇒ 1,200,000/1,150,000/1,450,000/1,200,000/700,000）。
//!    已改，见 [`LOGIC_TEMPO_PAYLOAD_LEN`]。**这一处此前没有被任何损失条目登记**。
//! 3. **region 音符 `qSvE` 载荷缺 16 字节尾**：参考实现（§8.5）记"空 region 的 `qSvE`
//!    载荷就是那 16 字节尾；每个音符在尾之前加一个 32 字节事件（载荷 = 32·N + 16）"。
//!    已改，见 [`LOGIC_EVENT_SEQUENCE_TAIL`]。groove 的读取器把 `f1 00` 当运行结束标记，
//!    因此往返解析不受影响。
//! 4. **音符事件里三个常量字节没写**：参考实现 §8.5 的实测表给出 `+0x0f` 标志（`0x01`，
//!    最后一条音符带 `0x80`）、`+0x10` = `0x40`、`+0x17` = `0x89`；本写入器此前把这三处
//!    留成 0（其中 `+0x17` 只是"续行标志"的 `0x80`）。已按实测字面量改，见
//!    [`LOGIC_NOTE_FLAG`] / [`LOGIC_NOTE_FLAG_LAST`] / [`LOGIC_NOTE_CONST_10`] /
//!    [`LOGIC_NOTE_CONST_17`]。`+0x0a` 的"细力度"实测为 0（参考实现的
//!    `_enc_note_event(..., fine=0)` 也写 0），因此保持 0。
//!
//! ## 实测的 `Trak` 家族：上一轮的**布局**读数（本轮复核仍然成立，保留备查）
//!
//! 第 403 轮把 `Trak` 家族（**落盘**的四个字节是 `6b 61 72 54`，即简报里的 `karT`；按落盘字节反序解码
//! 才是可读名 `Trak`）在**四份**真实工程里逐字节量了一遍，而不是只看简报点名的两个演示：
//! 两个 Apple 演示工程（`Swing!` 版本码 **2507**、`ocean eyes` **2000**）与两份**与本写入器同版本码
//! 2512** 的工程（`~/Music/Logic/quiet`、工厂模板 `01 Hip Hop`）—— 因为第 402 轮的教训正是
//! **参照系**：我们的目标版本码是 2512，而两个演示分别是 2507 与 2000。
//!
//! **测法**：从 0x18 起**只按** 36 字节记录头的 `u32` 小端载荷长度（+0x1c）驱动走完整个文件。
//! 四份文件的行走终点都**恰好等于 EOF**。读数（字节数用 `stat -f%z` 口径）：
//!
//! | 读数 | `Swing!` (2507) | `ocean eyes` (2000) | `quiet` (2512) | `01 Hip Hop` (2512) |
//! | :--- | ---: | ---: | ---: | ---: |
//! | `ProjectData` 字节数 | 5,648,035 | 4,075,622 | 360,193 | 1,862,456 |
//! | 全文件记录条数 | 4,626 | 4,094 | 718 | 1,717 |
//! | `Trak` 记录条数 | 799 | 753 | 118 | 179 |
//! | 其中载荷长 **0** 的 | 545 | 245 | 65 | 75 |
//! | 其中载荷**非 0** 的 | 254（长 57） | 508（长 56） | 53（长 58） | 104（长 58） |
//! | ↳ `+0x08 == 0x40000` 组的非 0 条数（= 轨道数 + 1） | 77（76+1） | 43（42+1） | 11（10+1） | 35（34+1） |
//! | ↳ `+0x08 == 0x80000` 组的非 0 条数（每个混音槽一条） | 173 | 104 | 40 | 66 |
//! | 记录头 kind（+0x04） | **5** | **4** | **6** | **6** |
//! | 记录头 subtype（+0x06） | 23 | 23 | 23 | 23 |
//! | `MetaData.plist` `NumberOfTracks` | 76 | 42 | 10 | 34 |
//!
//! 记录头其余字节在四份文件里**实测恒定**（与其它家族共用同一套记录头）：+0x0c..0x0f =
//! `00 00 ff ff`、+0x0e..+0x11 = `ff ff ff ff`、+0x16 = 2、+0x18 = 0、+0x1a = 2（2512）/ 1
//! （2507、2000，与 [`LOGIC_RECORD_FIELD_1A`] 的实测规则独立吻合）；载荷长 0 的那一类
//! +0x12..+0x15 = `ff ff ff 7f`。只有 +0x08..+0x0b（**对象号**，见上文问题 3）与载荷非 0
//! 那类的 +0x12 随记录变化。kind 随版本走（2000→4、2507→5、2512→6），因此**不是**跨演示常量。
//!
//! 另外：**不存在嵌套的 `karT`**。落盘的 `6b 61 72 54` 在四份文件里的出现次数**恰好等于**
//! `Trak` 记录条数（799 / 753），而按**正序**拼写 `karT` 的四个字节 `54 72 61 6b` 出现 **0** 次；
//! 即这一族没有"里层还有一层 `karT`"的结构（round 394 的"容器是扁平的"在本族上独立复现）。
//!
//! 这一决定由两条判据钉住：无头的 `track_family_is_measured_but_deliberately_unwritten`
//! （钉住"产物里一条 `karT` 都没有"＋"损失条目点名了三个子群的读数"），与路径不存在即 skip 的
//! `real_demo_track_family_shape_matches_the_measurement_when_present`（用**实测字面量**钉住
//! 上表的每一格，并断言 `0x40000` 组非 0 条数 − 1 **恰等于**同目录 `MetaData.plist` 的
//! `NumberOfTracks` —— 这正是本轮语义答案的证据）。
//!
//! ## 映射损失表（**不静默丢东西**）
//!
//! [`build_bundle`] / [`project_data`] 与字节**同时**返回 [`LogicLoss`] 列表。
//! `reason` 的**前缀**是机器可读的两分法：
//!
//! | 前缀 | 含义 |
//! | :--- | :--- |
//! | [`LOSS_UNMAPPED_PREFIX`]（`未映射:`） | 该构造在产物里**没有任何表示** |
//! | [`LOSS_NOT_EQUIVALENT_PREFIX`]（`非等价:`） | 该构造**有**表示，但等价性**未经证实** |
//!
//! 与 `.als` 的 XML 注释不同，`ProjectData` 没有注释通道，因此整张表同时写进
//! `MetaData.plist` 的夜半扩展键 [`META_DATA_LOSS_KEY`]（一个字符串数组，每条 =
//! `"<entity>: <reason>"`）—— **报告给用户的那张表与落盘的这张表因此不会各说各话**。
//! 这不是一句设计意图：判据
//! `reported_loss_table_equals_the_embedded_plist_table_entry_for_entry` 把
//! `MetaData.plist` 的字节重新解一遍，与调用方拿到的 [`LogicBundle::losses`] 逐条
//! （条数、顺序、全文、`未映射:` / `非等价:` 分类）对账，任一侧漂移都会红。
//! ⚠ 这是一个**非 Logic 键**：Logic 忽略未知 plist 键这件事在本机**未验证**。
//!
//! ## 为什么不用 plist crate
//!
//! `MetaData.plist` 的实测形态是**标准二进制 plist**（`file` 报 `bplist00`）。
//! 写二进制 plist 的标准做法是引一个 `plist` crate；但本 feature 的验收条件之一是
//! **默认依赖图零变化**，而本机 registry 里没有该 crate（离线），因此这里按 bplist00
//! 规范自己写一个**最小、确定性**的编码器（[`encode_binary_plist`] 的调用方）。
//! 产物已用 Python `plistlib` 读回对账（见 `docs/DEVELOPMENT_LEDGER.md` 本轮记录）。
//!
//! ## 诚实边界（**没有证明什么**）
//!
//! 1. **只声称一件事：本机 Logic Pro 12.2 能打开 [`build_bundle_from_donor`] 的供体拼接产物**
//!    （负责人实测，2026-10-06）。**这不覆盖**自研（synthesised）写入器的产物：它被拒绝过两次
//!    （第 403、405 轮是同一个通用失败对话框；第 401 轮是更早的 "Logic 4 format (or earlier)" /
//!    `com.apple.logic10 error 100`），**也不覆盖其它 Logic 版本或其它机器**。
//!    仓库里也**不提交**任何 Apple 演示工程（它们有版权）。
//! 2. **自研路线不写轨道对象、不写 region 摆放链；供体路线写的是供体的**：自研产物只写
//!    `gnoS` / `qSvE` / `qeSM` 与 region 的音符序列，Logic 的 `Trak` 轨道家族（落盘字节
//!    `6b 61 72 54`）**没有写**（groove 的写入器同样如此），登记在
//!    [`TRACK_OBJECTS_UNMAPPED`] 与 [`REGION_PLACEMENT_UNMAPPED`]；供体路线
//!    （[`build_bundle_from_donor`]，也是 `--export-logic` 现在走的那条）把供体的 22 条 `karT`
//!    与两条摆放事件**原样带进来**，但**没有**激活任何新槽、也没有把 region 摆到我们自己的
//!    摆放下标上 —— 那一整族仍在损失表里逐族登记（`LogicBuilder::write_donor_losses`）。
//!    产物经 groove 的读取器可以往返，但 Logic 是否会据此显示轨道**未验证**。
//! 3. **自研路线只写 3/27 个实测 chunk 家族**：真实工程（两例并集）有 27 个家族，自研产物只写
//!    `Song` / `EvSq` / `MSeq`；其余 **24 个**家族（[`MISSING_CHUNK_FAMILIES`]：插件、
//!    混音、环境、自动化、视频、网格…）**逐族**进损失表，理由里点名 chunk。容器头里
//!    **仍未重建**的字段（记录头 +0x08..+0x14 的**对象号分配规则**、region 的 subtype、
//!    `gnoS` 子帧的正文）作为 [`CONTAINER_HEADER_CAVEAT`] 登记（`非等价:`），region 载荷里
//!    **除名字字段以外**的字节（`+0x00` 起 `2e 03`、载荷长 296..324、`ocean eyes` 的部分
//!    记录在名字后还有一份空格前缀副本）作为 [`REGION_PAYLOAD_CAVEAT`] 登记（`非等价:`）。
//!    region 的**名字字段**已按实测写入（载荷 +0x10/+0x12），**不**登记为偏差。
//!    供体路线走的是另一套损失条目（18 个家族**在产物里**但属于供体，逐族登记）。
//! 4. **本轮改正的四处 `qSvE` / 音符载荷形状**（拍号 80→96、速度 16→48、音符加 16 字节尾、
//!    音符事件三个常量字节）来自实测与 MIT 参考实现，因此**不**登记为偏差；但其中"速度
//!    `qSvE` 此前写的是速度 0"这一缺陷在上一轮之前**从未被任何损失条目登记**过 —— 这说明
//!    "逐条登记"的纪律仍有盲区，下一轮应当在每次拿到新参考材料时**重做一次逐字段对账**，
//!    而不是只补新发现的字段。
//! 5. **供体路线是"结构从哪来"的答案，本轮又有了"能打开"的测量**：本机 **Logic Pro 12.2**
//!    打开过供体拼接产物（负责人，2026-10-06）。但 `ProjectData` 的绝大部分字节是第三方夹具的，
//!    语义本仓库没有逐字段反推；供体**只带 1 条编排轨行** ⇒ 最多映射一条 MIDI 轨的**第一个**
//!    摆放，**没有通道槽激活**；`gnoS` 正文与根版本码 `0x09CF` 都原样保留供体的。
//!    结论**只覆盖本机 12.2**。
//!
//! ## 确定性
//!
//! 输出逐字节可复现：集合一律按 `BTreeMap` 键序迭代，**不出现 `HashMap`**（红线 4）；
//! 二进制 plist 的对象顺序由分配顺序固定，且不含时间戳/随机数。判据
//! `filled_project_bundle_is_byte_deterministic` 钉住这一点。

use std::collections::{BTreeMap, BTreeSet};

use yeban_model::{
    ClipContent, ClipPlacement, EntityId, MidiNote, PPQ, TrackKind, TrackV3, YebanProjectV1,
};

/// `.logicx` 根 chunk 的魔数（实测两例一致）。
pub const LOGIC_ROOT_MAGIC: [u8; 4] = [0x23, 0x47, 0xC0, 0xAB];

/// 根头长度（实测 **0x18** = 24 字节）。
pub const LOGIC_ROOT_HEADER: usize = 0x18;

/// 声明载荷长度（`u32` 小端）在根头里的偏移（实测 0x10）。
pub const LOGIC_DECLARED_LENGTH_OFFSET: usize = 0x10;

/// 根头里**工程格式版本码**（`u16` 小端）的偏移。参考实现
/// `jonkubis/logicproformatwriter`（MIT）的 `PROJECTDATA_FORMAT.md` §2 把它记作
/// "`+0x04` 2 version code"。
pub const LOGIC_FORMAT_VERSION_OFFSET: usize = 0x04;

/// 根头 0x06..0x0f 那十个**恒定字节**的偏移。
pub const LOGIC_ROOT_STABLE_OFFSET: usize = 0x06;

/// 根头的工程格式版本码（`u16` 小端）。
///
/// 实测（本机 `~/Music/Logic`、`~/Music/templates`、`~/Music/Logic Book Projects`、
/// `~/Music/Logic Pro Library.bundle`、`/Applications/Logic Pro.app/Contents/Resources/Project Templates`
/// 与 `/Library/Application Support/Logic/Logic Pro X Demosongs` 下的**全部** `ProjectData`）：
/// 该字段随 Logic 版本单调增大，且能与同工程 `Resources/ProjectInformation.plist` 的
/// `LastSavedFrom` 一一对上：
///
/// | `LastSavedFrom` | 版本码 |
/// | :--- | :--- |
/// | `Logic Pro X 10.2.4 (4369.43)` | `0x06DC` = 1756 |
/// | `Logic Pro X 10.4.0 (4905.7)` | `0x06EA` = 1770 |
/// | `Logic Pro X 10.5.1 (5299)` | `0x07D0` = 2000 |
/// | `Logic Pro X 10.7.0 (5533)` | `0x09C4` = 2500 |
/// | `Logic Pro X 10.8.1 (5906)` | `0x09CB` = 2507 |
/// | `Logic Pro X 11.0.1 (6029)` | `0x09CD` = 2509 |
/// | `Logic Pro 11.1.2 (6162)` | `0x09CE` = 2510 |
/// | `Logic Pro 11.2.2 (6387)` | `0x09CF` = 2511 |
/// | **`Logic Pro 12.0.1 (6590)`** | **`0x09D0` = 2512** |
///
/// 本机安装的 Logic Pro 是 **12.2**，但本机**没有任何 12.2 存过的工程**，因此这里取实测到的
/// 最新值 **`0x09D0`（Logic Pro 12.0.1）**，而不是替 12.2 猜一个专用值。参考实现那份格式文档
/// （由 Logic Pro 11.2.2 的差分夹具反推、产物经 Logic 打开验证）把 11.2.2 记为
/// `D0 09` / `CF 09`，与实测的 2511/2512 一致。
///
/// **写 0 的后果已被外部测量**：负责人把本写入器的产物交给 **Logic Pro 12.2**，得到的对话框是
/// *"The song you are trying to open is in **Logic 4 format (or earlier)**"*
/// （`com.apple.logic10 error 100`）。零即"最老的格式"，这是本字段必须非零的直接证据。
pub const LOGIC_FORMAT_VERSION_CODE: u16 = 0x09D0;

/// 根头 0x06..0x0f 的十个字节。**实测恒定**：本机全部真实 `ProjectData`
/// （10.2.4 … 12.0.1：七个演示工程 + 用户工程 + 工厂模板）与参考实现的产物
/// （版本码 2511）在这十个字节上**完全一致**；参考实现的格式文档也把它记作 "(stable)"。
///
/// ⚠ 这十个字节的**语义未证实**（本仓库只证明其取值恒定），因此只按实测值写出。
pub const LOGIC_ROOT_STABLE_FIELDS: [u8; 10] =
    [0x03, 0x00, 0x04, 0x00, 0x00, 0x00, 0x01, 0x00, 0x08, 0x00];

/// 第一个记录（chunk）的偏移 = [`LOGIC_ROOT_HEADER`]。
pub const LOGIC_FIRST_RECORD_OFFSET: usize = LOGIC_ROOT_HEADER;

/// 记录头长度（实测 0x24 = 36 字节）。
pub const LOGIC_RECORD_HEADER: usize = 0x24;

/// 记录头里 cluster 索引（`u32` 小端）的偏移（groove 读取器读 +8）。
pub const LOGIC_RECORD_CLUSTER_OFFSET: usize = 0x08;

/// 记录头里 **kind**（`u16` 小端）的偏移。参考实现的格式文档记作 "kind@+4"。
pub const LOGIC_RECORD_KIND_OFFSET: usize = 0x04;

/// 记录头里 **subtype**（`u16` 小端）的偏移。参考实现的格式文档记作 "subtype@+6"。
pub const LOGIC_RECORD_SUBTYPE_OFFSET: usize = 0x06;

/// 记录头 +0x16（`u16` 小端）：实测**本机全部真实记录恒为 2**，无例外。
pub const LOGIC_RECORD_FIELD_16: u16 = 2;

/// 记录头 +0x18（`u16` 小端）：实测恒为 0。
pub const LOGIC_RECORD_FIELD_18: u16 = 0;

/// 记录头 +0x1a（`u16` 小端）：实测格式版本码 `>= 2509` 时恒为 **2**，`<= 2507` 时为 1。
/// 本写入器的目标版本码是 [`LOGIC_FORMAT_VERSION_CODE`]（2512），因此写 2。
pub const LOGIC_RECORD_FIELD_1A: u16 = 2;

/// `gnoS`（`Song`）记录头的 kind：实测 2511/2512 为 **6**、2509 为 5、`<= 2507` 为 3。
pub const LOGIC_SONG_KIND: u16 = 6;

/// `gnoS` 记录头的 subtype：实测**恒为 `0xFFFF`**（本机全部真实工程与参考实现产物）。
pub const LOGIC_SONG_SUBTYPE: u16 = 0xFFFF;

/// `qSvE`（`EvSq`）记录头的 kind：实测**恒为 1**。
pub const LOGIC_SEQUENCE_KIND: u16 = 1;

/// 拍号 `qSvE` 的 subtype（载荷第一个字 = `0x30` 的那条）：实测**恒为 1**。
pub const LOGIC_METER_SUBTYPE: u16 = 1;

/// 速度 `qSvE` 的 subtype（载荷第一个字 = `0x60` 的那条）：实测**恒为 3**。
///
/// 实测依据：本机 10.5.1、10.8.1、11.2.2、12.0.1 四代工程里，`payload[0]=0x30`（拍号）的
/// `qSvE` 都是 subtype 1、`payload[0]=0x60`（速度）的 `qSvE` 都是 subtype 3 —— 与记录顺序
/// 无关（同一份文件里后者出现在别处时仍是 3）。本仓库只实测到 subtype 与 marker 的**配对**，
/// 不解释其语义。
pub const LOGIC_TEMPO_SUBTYPE: u16 = 3;

/// `qeSM`（`MSeq`，region）记录头的 kind：实测 2509+（11.0.1 / 11.2.2 / 12.0.1）恒为 **5**，
/// `<= 2507` 时为 2/3 —— 与 [`LOGIC_SONG_KIND`] 一样是"随格式版本走"的 kind。
pub const LOGIC_REGION_KIND: u16 = 5;

/// `gnoS` 载荷开头那段嵌套 `#G` 子帧前缀里，紧跟版本码的四个实测字节。
///
/// 实测：本机全部真实工程与参考实现产物在 `gnoS` 载荷 `+0x00` 都是
/// `23 47 C0 AB` + 版本码（`u16` 小端）+ `18 00 11 00`。本写入器只写这段实测前缀；
/// 子帧其后约 10 KB 的全局设置**没有重建**（参考实现同样把整个 `gnoS` 视为不透明、
/// 靠克隆 donor 而不重建），这一点保留在 [`CONTAINER_HEADER_CAVEAT`] 里。
pub const LOGIC_SONG_SUBFRAME_SUFFIX: [u8; 4] = [0x18, 0x00, 0x11, 0x00];

/// 记录头里载荷长度（`u32` 小端）的偏移（实测 +0x1c）。
pub const LOGIC_RECORD_SIZE_OFFSET: usize = 0x1c;

/// 工程/速度 chunk 的名字：`Song` 按**小端**存放即 `gnoS`（实测）。
pub const LOGIC_SONG_TAG: [u8; 4] = *b"gnoS";

/// [`LOGIC_SONG_TAG`] 对应的可读 ASCII —— 判据用它证明"chunk 名是字节反序的"。
pub const LOGIC_SONG_NAME: &[u8; 4] = b"Song";

/// 事件序列 chunk 的名字（拍号 / 速度 / 音符序列共用）。
pub const LOGIC_SEQUENCE_TAG: [u8; 4] = *b"qSvE";

/// region chunk 的名字。
pub const LOGIC_REGION_TAG: [u8; 4] = *b"qeSM";

/// 事件行长度（16 字节）。
pub const LOGIC_EVENT_LINE_SIZE: usize = 16;

/// 音符头行的状态字节（MIDI 风格的 `0x90`）。
pub const LOGIC_NOTE_STATUS: u8 = 0x90;

/// 事件行第 7 字节的续行标志位。
pub const LOGIC_CONTINUATION_FLAG: u8 = 0x80;

/// 每条 `qSvE`（事件序列）载荷**结尾**的 16 字节尾。
///
/// 实测：五份文件里 `qSvE` 载荷以此尾结束的比例是 `F0` 夹具 **13/13**、
/// `Swing!` **544/545**、`ocean eyes` **240/245**、`quiet` **65/65**、`01 Hip Hop` **75/75**
/// （少数例外是参考实现记载的 "settling" 变体）。
/// 参考实现 `jonkubis/logicproformatwriter`（MIT，`PROJECTDATA_FORMAT.md` §3）把它写成
/// "Each `qSvE` payload = `[events...]` + a **16-byte TAIL**
/// `F1 00 00 00 FF FF FF 3F 00 00 00 00 00 00 00 00`" —— 本常量就是那 16 个字节的实测字面量。
///
/// groove 的读取器把开头的 `f1 00` 当作"空序列/运行结束"标记（`EMPTY_SEQUENCE_MARKER`），
/// 因此写上这个尾**不会**让它的往返解析多出一条音符：测试用的 [`decode_note_lines`] 同样
/// 在第 7 字节（`0x3f`，最高位为 0）判定它不是续行、首字节 `0xf1 ≠ 0x90` 判定它不是音符。
pub const LOGIC_EVENT_SEQUENCE_TAIL: [u8; 16] = [
    0xF1, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0x3F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// 拍号序列载荷的第一个字（判定"这是拍号序列"的记号）。
pub const LOGIC_METER_MARKER: u32 = 0x30;

/// 速度序列载荷的第一个字（判定"这是速度序列"的记号）。
pub const LOGIC_TEMPO_MARKER: u32 = 0x60;

/// 拍号 `qSvE` 载荷里**头部**的长度。参考实现（MIT，§5）写
/// "`payload_size = 80 + 48*changes + 16`"，即 80 字节头 + 每条变化 48 字节 + 16 字节尾。
/// 因此**没有**拍号变化时载荷长 = 80 + 16 = [`LOGIC_METER_PAYLOAD_LEN`]（实测五份文件里的
/// 无变化工程都是 96 字节：`F0` 夹具、`quiet`、`ocean eyes`、`01 Hip Hop`）。
pub const LOGIC_METER_BODY_LEN: usize = 80;

/// 拍号 `qSvE` 载荷在**没有**拍号变化时的长度：80 字节头 + 16 字节尾（实测字面量 96）。
pub const LOGIC_METER_PAYLOAD_LEN: usize = 96;

/// 拍号头里分母的以 2 为底指数（`den = 2^exp`）的偏移。实测：`F0`/`quiet`/`01 Hip Hop` 的
/// 4/4 都是 `02 04` 落在 `+0x0b`/`+0x0c`。
pub const LOGIC_METER_DENOMINATOR_EXPONENT_OFFSET: usize = 0x0b;

/// 拍号头里分子的偏移（见 [`LOGIC_METER_DENOMINATOR_EXPONENT_OFFSET`]）。
pub const LOGIC_METER_NUMERATOR_OFFSET: usize = 0x0c;

/// **没有**拍号变化时头部 `+0x0f` 的标志位。参考实现（MIT，§5）记
/// "`flag@+0x0F` (0x80 if there are no changes)"，`set_meter_map` 正是这么写的，
/// 而它产出的工程被 Logic 打开过。
///
/// ⚠ 实测的**本机**取值并不统一：`F0` 夹具（12.0.1）是 `0x80`，而 `quiet`（0x09D0）与
/// `01 Hip Hop`（0x09D0）是 `0x00`、`ocean eyes`（0x07D0）是 `0x01`。参考实现明写这个字节
/// **不承载映射**（"These bytes do NOT affect the decoded map"）且两种形态都被 Logic 接受，
/// 因此这里写参考实现记录的"无变化"形态 `0x80`。
pub const LOGIC_METER_NO_CHANGE_FLAG: u8 = 0x80;

/// 一条速度事件的长度（`u32` 小端位置 + 7 个字段 = 32 字节）。
pub const LOGIC_TEMPO_EVENT_LEN: usize = 0x20;

/// 速度 `qSvE` 载荷在**单条**初始速度时的长度：32 字节事件 + 16 字节尾。
///
/// 实测五份文件里的全局速度序列**全部**是 48 字节：`F0` 夹具、`Swing!`、`ocean eyes`、
/// `quiet`、`01 Hip Hop`。参考实现（MIT，§4）写 "Payload = N events + 16-B tail;
/// `payload_size = 32N + 16`"。
pub const LOGIC_TEMPO_PAYLOAD_LEN: usize = 0x30;

/// 速度事件里的位置（`u64` 小端）＝**速度/记号原点** 38400（= [`LOGIC_NOTE_ORIGIN_TICKS`]）。
///
/// 实测五份文件的第一条速度事件在 `+0x04` 都是 `00 96 00 00 00 00 00 00`（= 38400）；
/// 参考实现（MIT，§3）记 "Tempo/marker-origin 38400 (= 34560 + 3840)"。
pub const LOGIC_TEMPO_POSITION_TICKS: u64 = LOGIC_NOTE_ORIGIN_TICKS;

/// 速度事件 `+0x0f` 的标志：`0x00` = 首个/初始事件（实测五份文件全为 `0x00`）。
pub const LOGIC_TEMPO_FLAG_FIRST: u8 = 0x00;

/// 速度事件 `+0x14..+0x18` 的恒定四个字节（实测五份文件一致）。
pub const LOGIC_TEMPO_EVENT_CONST_14: [u8; 4] = [0x00, 0x00, 0x40, 0x88];

/// 速度事件（32 字节）里速度值的偏移：`u32` 小端 = `round(bpm × 10000)`。
///
/// 实测：`F0`、`quiet`、`01 Hip Hop`（120 BPM）在 `+0x10` 都是 `80 4F 12 00`；
/// `Swing!`（115）/`ocean eyes`（145）/`01 Hip Hop`（70）只差这一个字。
pub const LOGIC_TEMPO_VALUE_OFFSET: usize = 0x10;

/// 速度事件 `+0x18` 的绝对时间缓存（`u32` 小端）。
///
/// 参考实现（MIT，§4）给出精确公式：`altpos = 7_200_000 + round(Σ …)`，原点 7,200,000 是
/// 1 小时 SMPTE、单位 2000/秒。首个事件在 tick 0 处累计时长为 0，因此就是 **7,200,000**
/// （实测字面量 `00 dd 6d 00`，五份文件一致）。
pub const LOGIC_TEMPO_ALT_POSITION_BASE: u32 = 7_200_000;

/// 一拍等于多少 tick（= 夜半的 [`PPQ`] = 960）。
pub const LOGIC_TICKS_PER_QUARTER: u64 = PPQ;

/// region 摆放的计数原点（九个 4/4 小节 = 34560 tick）；本切片只把它写进文档与损失表。
pub const LOGIC_REGION_ORIGIN_TICKS: u64 = 34_560;

/// 音符事件的计数原点（region 原点 + 一整个 4/4 小节 = 38400 tick，实测写入器加、读取器减）。
pub const LOGIC_NOTE_ORIGIN_TICKS: u64 = 38_400;

/// `gnoS` 载荷里速度的**权威槽**（`u32` 小端，`round(bpm × 10000)`）。
pub const LOGIC_SONG_TEMPO_SLOT_AUTHORITATIVE: usize = 0x3a6;

/// `gnoS` 载荷里速度的**回退槽**。
pub const LOGIC_SONG_TEMPO_SLOT_FALLBACK: usize = 0x92;

/// region（`qeSM`）**载荷**里名字字段的偏移：长度 `u16` **小端** @ **+0x10**，
/// UTF-8 名字字节 @ **+0x12**，名字之后 4 个 0 字节。
///
/// 实测（本机两例 Apple 演示工程，**790/790** 条 `qeSM` 一致）：
///
/// | 文件 | 版本码 | `qeSM` 条数 | 载荷 `+0x10` | 载荷 `+0x12` 起 |
/// | :--- | :--- | :--- | :--- | :--- |
/// | `ocean eyes.logicx/Alternatives/001` | `0x07D0`（10.5.1） | 245 | `u16` 小端 = 名字字节数 | 名字（UTF-8） |
/// | `Swing!.logicx/Alternatives/004` | `0x09CB`（10.8.1） | 545 | 同上 | 同上 |
///
/// 每一条的 `+0x10` 都恰好等于其后名字的 **UTF-8 字节数**（不是字符数），名字之后 4 字节
/// **全为 0** —— 载荷总长 296..324 字节，这一条对 790 条无一例外。
///
/// 长度口径 = **字节数**的独立证据：本机 **501** 份真实 `ProjectData` 里有 **1070** 条名字含
/// `>=0x80` 的字节，**全部**是合法 UTF-8，且长度字段等于字节数 —— 例：`未命名` 的
/// `e6 9c aa e5 91 bd e5 90 8d` 是 9 字节，长度字段就是 `09 00`。
///
/// 记录内偏移 = [`LOGIC_RECORD_HEADER`] + 0x10 = **0x34**。
///
/// ⚠ **账本第 402 轮那条"本写入器把 region 名放在载荷 +0x34、真实文件在 +0x10/+0x12"的差异
/// 是参照系混淆，不是字节差异**：0x34 是**记录**内偏移，载荷内偏移是 `0x34 − 0x24 = 0x10`；
/// 本写入器产物的 region 载荷实测就在 `+0x10/+0x12`。本常量现在只以**载荷**为参照系，避免同一误读
/// 再发生；判据 `region_name_field_sits_at_the_measured_payload_offsets` 用实测字面量钉住它。
pub const LOGIC_REGION_NAME_PAYLOAD_OFFSET: usize = 0x10;

/// `未映射:` —— 该构造在产物里没有任何表示。
pub const LOSS_UNMAPPED_PREFIX: &str = "未映射:";

/// `非等价:` —— 该构造有表示，但等价性未经证实。
pub const LOSS_NOT_EQUIVALENT_PREFIX: &str = "非等价:";

/// 每一次导出都会登记的那条"打开结论的适用范围"警告。
///
/// ⚠ 旧措辞（"无 ground truth"，并据此否认过打开结论）已被**实测**取代：
/// 本机 **Logic Pro 12.2** 能打开**供体拼接**（donor-spliced）产物（负责人，2026-10-06）。
/// 自研（synthesised）写入器的产物被拒绝过两次（账本第 403、405 轮），结论不跨版本、不跨机器 ——
/// 这条范围因此仍然作为 `非等价:` 损失逐次导出登记。
pub const LOGIC_OPEN_SCOPE_CAVEAT: &str = concat!(
    "非等价: 打开结论的适用范围 —— 本机 Logic Pro 12.2 实测能打开**供体拼接**（donor-spliced）产物",
    "（负责人，2026-10-06）：借用 MIT 供体 `F0_baseline` 的 527 条记录、按供体顺序、只改 4 条",
    "（全局拍号、全局速度、被摆放 region 的名字字段、该 region 的音符载荷）。这**不覆盖**自研",
    "（synthesised）写入器的产物 —— 它被拒绝过两次（账本第 403、405 轮），也不覆盖其它 Logic 版本或其它机器。",
    "ProjectData 的字节布局仍是按**实测**重建的（根魔数 `23 47 C0 AB` 在 0、根头 0x18 字节、",
    "声明载荷长度 u32 小端在 0x10、第一个 chunk 名在 0x18、36 字节记录头、16 字节事件行；",
    "chunk 名小端存放，`Song` 落盘为 `gnoS`）。"
);

/// 每一次导出都会登记的那条"轨道对象未写入"警告。
///
/// ⚠ **第 404 轮把这条从"语义未知"改成了"语义已知、但参考实现说不能凭空合成"。** 这是重点：
/// 上一轮拒绝写轨道表的理由是**不知道它是什么**；本轮从 MIT 参考实现
/// `jonkubis/logicproformatwriter` 的 `PROJECTDATA_FORMAT.md` 拿到了语义，并且**在本机五份
/// 真实文件上逐条复现了它**（`F0` 夹具 + 四份真实工程）。结论是**仍然不写**，但理由换了：
/// 参考实现明写轨道**不能**从零合成（§10.6.1），它自己的写入器一律从 Logic 存过的 donor
/// 模板做增量重放。理由里现在有可核对的读数，而不是"未知"。
///
/// 实测读数（五份文件，`Trak` 家族按记录头 `+0x08` 的对象号分组）：
/// * `+0x08 == 0x00040000` 组 = **编排轨行**：非 0 载荷条数 − 1 **恰等于** `NumberOfTracks`，
///   五份全中（`F0` 2−1=1、`Swing!` 77−1=76、`ocean eyes` 43−1=42、`quiet` 11−1=10、
///   `01 Hip Hop` 35−1=34）；多出来的那条是 master 行（载荷 `+0x00 == 3`，载荷 `+0x08`
///   指向通道槽 `0x50`）。
/// * `+0x08 == 0x00080000` 组 = **每个预分配混音槽一条 Track 对象**（载荷 `+0x08` = 槽号，
///   记录头 `+0x12` = 名次；**流的顺序就是轨道表顺序**）。
/// * 其余对象号下还有少量 `Trak`，多数载荷为 0。
///
/// 因此上一轮"条数与轨道数不相等"的谜底是：这一族**不是**一张表，而是**三个子群**；
/// 其中编排轨行那一群**恰好**是 `NumberOfTracks + 1`。但这仍然不支持本切片写它 ——
/// 参考实现（MIT，§10.6.1）的原话是：轨道不能凭空合成，因为新建通道会触发 Logic 的
/// 混音器 / CoreMIDI Environment **扩张**，重新生成 time-UUID 并重排整个 `OCuA` 通道块，
/// "pervasive and impractical to reproduce"；其通过 Logic 验证的做法是克隆一份
/// **Logic 存过的** donor 模板的通道簇（`ivnE` + `OCuA` + `gnoS` 注册表 + `MneG`）。
/// 本仓库的**自研**路线（[`project_data`] / [`build_bundle`]）不引入任何 donor
/// （Apple 演示工程有版权）；donor 路线（[`project_data_from_donor`] /
/// [`build_bundle_from_donor`]）用的是另一份 **MIT** 供体（`assets/logic-donor/`），
/// 它的登记由 `LogicBuilder::write_donor_losses` 逐族产出，**不**用本条。
pub const TRACK_OBJECTS_UNMAPPED: &str = concat!(
    "未映射: Logic 的 `Trak` 轨道家族（**落盘**四个字节 `6b 61 72 54`，即 `karT`）未写入 —— ",
    "本切片只写 `gnoS`(工程/速度)、`qSvE`(拍号与速度事件)、`qeSM`(region) 与 region 的音符序列。",
    "**语义本轮已由 MIT 参考实现 `jonkubis/logicproformatwriter`（`PROJECTDATA_FORMAT.md` §8.1/§10.6）",
    "给出，并在本机五份真实文件（F0 夹具 + Swing!/ocean eyes/quiet/01 Hip Hop）上复现**：",
    "这一族按记录头 `+0x08` 的对象号分三个子群 —— `0x00040000` 是**编排轨行**（非 0 载荷条数 − 1 ",
    "恰等于 `NumberOfTracks`：2−1=1、77−1=76、43−1=42、11−1=10、35−1=34，五份全中；多出的那条是 ",
    "master 行，载荷 +0x00 = 3、载荷 +0x08 指向通道槽 0x50），`0x00080000` 是**每个预分配混音槽一条 ",
    "Track 对象**（载荷 +0x08 = 槽号、记录头 +0x12 = 名次，流顺序即轨道表顺序），其余对象号下还有少量 ",
    "空载荷记录。上一轮「条数 799/753/118/179 与轨道数 76/42/10/34 不相等」的谜底就是这三个子群。",
    "记录头在这一族里实测恒定：kind 随版本 5/4/6、**subtype 恒 23**、+0x16 = 2、+0x18 = 0。",
    "**仍然不写的理由换成了参考实现的实测结论**：轨道不能凭空合成 —— 新建通道会触发混音器 / ",
    "Environment 扩张并重排整个 `OCuA` 通道块，参考实现自己也只能克隆 **Logic 存过的** donor 模板的 ",
    "通道簇（`ivnE` + `OCuA` + `gnoS` 注册表 + `MneG`）。本仓库的**自研**路线不引入任何这类 donor",
    "（Apple 演示工程有版权）⇒ 凭空写轨道表这条路**没有任何可用来源验证过**；",
    "⚠ **本条描述的是自研产物**：donor 路线（`build_bundle_from_donor`，也是 `--export-logic` 现在走的那条）",
    "把供体的 22 条 `karT`（含 2 条编排轨行 = 1 条轨道 + 1 条 master）与两条摆放事件**原样带进来**，",
    "它的登记是 `LogicBuilder::write_donor_losses` 的另一套逐族条目"
);

/// region **摆放链**未写入（`未映射:`）—— 这是本轮从参考实现读到的、关于"region 属于哪条轨道"
/// 的答案，也是本切片产物的一个真实缺口。
///
/// 参考实现（MIT，§8.1 / §8.5）与实测一致：**region 自己不指向轨道**。关系存在**轨道的**
/// `qSvE` 里，作为一条**摆放事件**：
///
/// | 字段 | 含义 |
/// | :--- | :--- |
/// | `+0x00` | 事件类型记号 `20 00 00 00`（MIDI）/ `24 00 00 00`（音频） |
/// | `+0x04` | `u32` 位置 = 34560 + tick（region 原点） |
/// | `+0x10` | `u32` 每条摆放的 id（`0x58 + 序号×4`），与 region `qeSM` 的 `+0x108` 相同 —— 这是链接 id |
/// | `+0x14` | **1 起的轨道号**（字节） |
/// | `+0x2c` | `u32` 指向该 region 的链接 |
///
/// region 自己的记录头 `+0x08` 是它**自己的**对象号（例：`0x1c0000` = 它自己的 `karT`+`qeSM`+`qSvE`
/// 那一簇），**不是**轨道号；位置还**另存一份**在 region `qeSM` 的 `+0x11c`（零起 tick），
/// 长度在 `+0x78`。
///
/// 本切片不写摆放事件、不写 region 的 `+0x78`/`+0x11c`，因此**region 在产物里无法落到任何轨道上**，
/// 位置也只由音符的绝对 tick 隐含。这不是静默省略：这一族要求的宿主（轨道簇）本切片没有写，
/// 写一条落在空处的摆放事件比不写更坏。
pub const REGION_PLACEMENT_UNMAPPED: &str = concat!(
    "未映射: region 的**摆放链**没有写入 —— region 自己不指向轨道，关系在**轨道的** `qSvE` 里：",
    "一条摆放事件（类型记号 `20`(MIDI)/`24`(音频)、位置 @+0x04 = 34560 + tick、",
    "链接 id @+0x10（与 region `qeSM` 的 +0x108 相同）、**1 起的轨道号 @+0x14**、region 链接 @+0x2c），",
    "位置另存一份在 region `qeSM` 的 +0x11c、长度在 +0x78（以上来自 MIT 参考实现 ",
    "`jonkubis/logicproformatwriter` 的 `PROJECTDATA_FORMAT.md` §8.1/§8.5，并与本机实测一致）。",
    "本切片不写摆放事件，也不写 `qeSM` 的 +0x78/+0x11c，因此 region 在产物里**无法落到任何轨道上**，",
    "位置只由音符的绝对 tick 隐含 —— 写一条落在空处的摆放事件比不写更坏"
);

/// region 自身起点字段的诚实说明（groove 记录的限制一并承接）。
pub const REGION_TIMING_CAVEAT: &str = concat!(
    "非等价: region 的自身起点字段一律写 0（实测真实工程里名字后的 u32 也是 0），",
    "因此 region 的摆放位置不由该字段表达；音符携带的是绝对 tick",
    "（placement.start_tick + note.start_tick + 38400）。",
    "参考实现（MIT，§8.5）给出位置真正存放的两处 —— 摆放事件的 +0x04 与 region `qeSM` 的 +0x11c —— ",
    "两处本切片都没写（见 `REGION_PLACEMENT_UNMAPPED`）。",
    "groove 记录的读取限制同样适用：ProjectData 里的一部分时限无法可靠读取，",
    "其读取器把每个 part 放在 beat 0 —— 本写入器不掩盖这一点"
);

/// region（`qeSM`）载荷里**除名字字段以外**的实测差异（`非等价:`）。
///
/// **名字字段本身不在这条里**：它已按实测写入（长度 `u16` 小端字节数 @ 载荷 `+0x10`、
/// UTF-8 名字 @ `+0x12`、其后 4 个 0；两例演示 790/790 条一致，见
/// [`LOGIC_REGION_NAME_PAYLOAD_OFFSET`]），并且账本第 402 轮把"载荷 +0x34"记成差异属于
/// **参照系混淆**（记录内 0x34 = 载荷内 0x10），不是字节差异 —— 因此这里**不登记**名字。
///
/// 仍然不同的是载荷的**其余部分**，实测如下：
///
/// * 真实 `qeSM` 载荷长 **296..324** 字节，`+0x00` 起是 `2e 03` 与一段随记录变化的字节
///   （例：`ocean eyes` 第一条 `2e 03 41 00 …`、`Swing!` 第一条 `2e 03 01 00 …`），
///   这段的语义**未反推**；
/// * `ocean eyes`（10.5.1）的部分记录在名字之后还有一份**空格前缀**的字符串副本 ——
///   例：第一条 `qeSM`（名字 `Untitled`，载荷 `+0x12..+0x19`）在载荷 `+0x2e` 是
///   `20 55 6e 74 69 74 6c 65 64 00`（`" Untitled\0"`）；`Swing!`（10.8.1）的 545 条里
///   537 条在名字之后没有连续 3 个以上可打印 ASCII 字节，没有一条被证实有同类副本。
///   它是"第二份名字"还是别的字段**未证实**；
/// * 本写入器的 region 载荷**只有**名字字段 = `0x16 + 名字的 UTF-8 字节数`（其余位置为 0）。
///
/// 这是"写了更少的内容"，不是静默省略：本切片不猜这段字节的语义，是否被接受**未经证实**。
pub const REGION_PAYLOAD_CAVEAT: &str = concat!(
    "非等价: region（`qeSM`）载荷除名字字段以外的内容没有重建 —— 名字字段已按实测写入",
    "（长度 u16 小端字节数 @载荷 +0x10、UTF-8 名字 @+0x12、其后 4 字节 0；两例演示 790/790 条一致，",
    "记录内偏移 0x34 = 0x24 + 0x10，因此账本第 402 轮记的 +0x34 差异是参照系混淆而非字节差异）；",
    "但真实载荷长 296..324 字节，+0x00 起是 `2e 03` 与一段随记录变化的字节（语义未反推），",
    "`ocean eyes`（10.5.1）的部分记录在名字后还有一份空格前缀的字符串副本；`Swing!`（10.8.1）的 545 条里未发现同类副本；",
    "本写入器的 region 载荷只有名字字段（0x16 + 名字字节数）。载荷其余字节的语义未证实，本切片不猜"
);

/// 本机两个 Apple 演示工程**实测**到的 chunk 家族（**解码后**的可读名，只记名字，不记内容）。
///
/// 测法（诊断判据 `logic_chunk_families_compare_side_by_side_when_the_demos_are_present`）：
/// 从 0x18 起按 36 字节记录头驱动走完整个文件
/// （tag 在 +0、cluster 在 +8、载荷长度 `u32` 小端在 +0x1c），把每个 tag 的四个字节
/// **反序**解码（落盘的 `gnoS` ⇒ 可读的 `Song`）。两个文件的家族集合**不同**
/// （`GenM` 只在 `Swing!`，`GAdd`/`Vide`/`Grid` 只在 `ocean eyes`），这里是并集。
///
/// ⚠ Apple 的演示工程**有版权、不进仓库**：这里只有从它们**测得**的 27 个名字，
/// 没有任何来自那些文件的字节或字符串内容。
pub const MEASURED_REAL_CHUNK_FAMILIES: [&str; 27] = [
    "AFld", "AuCO", "AuCU", "AuCn", "AuEv", "AuFl", "AuRg", "Clip", "CorM", "Envi", "EvSq", "GAdd",
    "GenM", "Grid", "Hypr", "InSt", "Layr", "MSeq", "ScSt", "SngO", "Song", "Styl", "Trak", "Trns",
    "TxSq", "TxSt", "Vide",
];

/// 本写入器**真正写进** `ProjectData` 的 chunk 家族（解码后可读名）。
///
/// 三个：`gnoS`(`Song`) + 拍号/速度两条 `qSvE`(`EvSq`) + 每个摆放一条 `qeSM`(`MSeq`)。
/// 判据 `emitted_chunk_families_are_exactly_the_declared_written_set` 把产物的实际家族
/// 与这个列表逐一对齐（写入器多写一族就红）。
pub const WRITTEN_CHUNK_FAMILIES: [&str; 3] = ["EvSq", "MSeq", "Song"];

/// "用途在本仓库内未证实"的家族共用的理由尾巴（22/24 条）。
const MISSING_FAMILY_TAIL_UNKNOWN: &str = concat!(
    "它的**用途在本仓库内未证实**——只实测到它在本机两个 Apple 演示工程里存在，",
    "未读到其字段语义，因此这里只说“没有写入”，不猜测它是什么"
);

/// 真实工程有、本写入器**不写**的 chunk 家族 = [`MEASURED_REAL_CHUNK_FAMILIES`] 减
/// [`WRITTEN_CHUNK_FAMILIES`]，共 **24** 个。每条在每次导出时登记一条 `未映射:` 损失，
/// 理由里点名 chunk（"Do not invent a reason"：只有 `Trak` 与 `AuRg` 的定位有仓库内依据，
/// 其余 22 个明写"用途未证实"）。
///
/// 二元组 = (解码后可读名, 定位说明)。
pub const MISSING_CHUNK_FAMILIES: [(&str, &str); 24] = [
    ("AFld", MISSING_FAMILY_TAIL_UNKNOWN),
    ("AuCO", MISSING_FAMILY_TAIL_UNKNOWN),
    ("AuCU", MISSING_FAMILY_TAIL_UNKNOWN),
    ("AuCn", MISSING_FAMILY_TAIL_UNKNOWN),
    ("AuEv", MISSING_FAMILY_TAIL_UNKNOWN),
    ("AuFl", MISSING_FAMILY_TAIL_UNKNOWN),
    (
        "AuRg",
        "音频 region 一族——groove 的 `docs/OPEN_WORK.md` 把落盘字节 `gRuA` 记为音频素材；\
         本仓库未进一步证实它的字段，本切片也不写 `Media/`",
    ),
    ("Clip", MISSING_FAMILY_TAIL_UNKNOWN),
    ("CorM", MISSING_FAMILY_TAIL_UNKNOWN),
    ("Envi", MISSING_FAMILY_TAIL_UNKNOWN),
    ("GAdd", MISSING_FAMILY_TAIL_UNKNOWN),
    ("GenM", MISSING_FAMILY_TAIL_UNKNOWN),
    ("Grid", MISSING_FAMILY_TAIL_UNKNOWN),
    ("Hypr", MISSING_FAMILY_TAIL_UNKNOWN),
    ("InSt", MISSING_FAMILY_TAIL_UNKNOWN),
    ("Layr", MISSING_FAMILY_TAIL_UNKNOWN),
    ("ScSt", MISSING_FAMILY_TAIL_UNKNOWN),
    ("SngO", MISSING_FAMILY_TAIL_UNKNOWN),
    ("Styl", MISSING_FAMILY_TAIL_UNKNOWN),
    (
        "Trak",
        "Logic 的轨道家族（**落盘**四个字节 `karT`）——**语义已由 MIT 参考实现 \
         `jonkubis/logicproformatwriter`（`PROJECTDATA_FORMAT.md` §8.1/§10.6）给出并在本机五份真实文件上复现**：\
         记录头 +0x08 的对象号把它分成三个子群 —— `0x00040000` 编排轨行（非 0 载荷条数 − 1 恰等于 \
         `NumberOfTracks`，多出的那条是 master）、`0x00080000` 每个预分配混音槽一条 Track 对象、\
         其余对象号下少量空载荷记录；上一轮「条数 799/753/118/179 与轨道数 76/42/10/34 不相等」的谜底即此。\
         仍然**不写**的理由是参考实现的结论：轨道不能凭空合成（新建通道会触发混音器 / Environment 扩张），\
         它自己也只能克隆 **Logic 存过的** donor 模板的通道簇（详见 `TRACK_OBJECTS_UNMAPPED`）",
    ),
    ("Trns", MISSING_FAMILY_TAIL_UNKNOWN),
    ("TxSq", MISSING_FAMILY_TAIL_UNKNOWN),
    ("TxSt", MISSING_FAMILY_TAIL_UNKNOWN),
    ("Vide", MISSING_FAMILY_TAIL_UNKNOWN),
];

/// 与真实工程**逐字节**对账后**仍然存在**的一处实测偏差（不是省略，是写了不同的值）。
///
/// 已经**不再是**偏差的字段（按本机实测改正，见各自的常量文档）：根头 4..0xf
/// （版本码 + 十个恒定字节）、记录头 kind@+4、`+0x16`、`+0x18`、`+0x1a`，`gnoS` 的 subtype
/// （`0xFFFF`）、拍号/速度 `qSvE` 的 subtype（1 / 3）、`gnoS` 载荷开头的 `#G` 子帧前缀，
/// 以及**第 404 轮改正的三处事件序列形状**：拍号 `qSvE` 载荷的 16 字节尾（80 → 96 字节）、
/// 速度 `qSvE` 载荷的 32 字节初始事件 + 16 字节尾（16 → 48 字节，此前速度字写的是 0）、
/// region 音符 `qSvE` 载荷的 16 字节尾。这些原先写错、现在按实测与 MIT 参考实现写入，不再登记。
///
/// 仍然写不同的值的是三处**没有反推出规则**的字段：记录头 `+0x08..+0x14` 的簇号（**含义已从参考
/// 实现读到**：`u32` 对象号 = 槽字节 `<< 16`；`qeSM` 与它配对的 `qSvE` 共享同一个对象号与 subtype，
/// 但**怎么给本写入器自己的 region 分配**这个号没有规则，本切片写的是 0/1/2… 的递增序号，
/// 不是实测的 `槽字节 << 16` 形态）与 `0xFFFF` / `0xFFFF0000` 哨兵（本机实测**随格式版本变化**：
/// 例如 `qSvE` 的 `+0x10` 在 2507 是 `0xFFFFFFFF`、在 2512 是 `0xFFFF0000`）；region 的 `qeSM`
/// 与音符 `qSvE` 的 subtype（实测是与配对序列共享的序列号 1/3/5/14/17/22/23/25，无编号规则可推，
/// 故保持 0）；以及 `gnoS` 载荷里嵌套 `#G` 子帧**除实测 10 字节前缀以外**的内容（真实工程约
/// 10 KB 全局设置，参考实现同样靠克隆 donor 而不重建）。这些**不猜**：登记为"有表示、
/// 等价性未经证实"，而不是静默省略。
pub const CONTAINER_HEADER_CAVEAT: &str = concat!(
    "非等价: 容器头仍有未重建的字段——根头版本码与 0x06..0x0f、每条记录的 kind@+4 与 ",
    "+0x16 / +0x18 / +0x1a、`gnoS` 的 subtype（0xFFFF）与拍号/速度 `qSvE` 的 subtype（1 / 3）、",
    "`gnoS` 载荷开头的 `#G` 子帧前缀，以及拍号/速度/音符 `qSvE` 载荷结尾的 16 字节尾与速度事件的 ",
    "32 字节正文（本轮按实测与 MIT 参考实现 `jonkubis/logicproformatwriter` 写入，见 ",
    "`LOGIC_EVENT_SEQUENCE_TAIL` / `LOGIC_TEMPO_PAYLOAD_LEN`），都已按本机实测的 Logic 12.0.1",
    "（格式版本码 0x09D0）取值写入；但记录头 +0x08..+0x14 的对象号**分配规则**没有反推出来",
    "（含义已读到：u32 对象号 = 槽字节 << 16，qeSM 与其配对 qSvE 共享对象号与 subtype；",
    "本切片写的是 0/1/2… 递增序号，不是实测形态），0xFFFF / 0xFFFF0000 哨兵实测随格式版本变化，",
    "region 的 `qeSM` 与音符 `qSvE` 的 subtype（与配对序列共享的序列号，实测 1/3/5/14/17/22/23/25，",
    "无编号规则可推）保持 0，`gnoS` 载荷里嵌套 `#G` 子帧除实测前缀外的内容（真实工程约 10 KB ",
    "全局设置）也没有重建。这是**写了不同的值**而不是省略：",
    "本切片不猜这些字段的语义，是否被接受**未经证实**"
);

/// 把落盘的四个 tag 字节反序解码成可读名：`gnoS` ⇒ `Song`。
#[must_use]
pub fn decode_chunk_tag(tag: &[u8; 4]) -> String {
    let mut bytes = *tag;
    bytes.reverse();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// 可读名对应的**落盘**四个字节（`Song` ⇒ `gnoS`），只用于损失表里点名。
fn stored_chunk_tag(name: &str) -> String {
    name.chars().rev().collect()
}

/// `MetaData.plist` 里承载整张损失表的**夜半扩展键**（字符串数组）。
///
/// ⚠ 这是非 Logic 键：Logic 忽略未知 plist 键在本机未验证，因此这条本身也被登记为损失。
pub const META_DATA_LOSS_KEY: &str = "YebanMappingLosses";

/// 一条"无法等价映射"的记录。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogicLoss {
    /// 被降级的夜半实体的稳定文本寻址。
    ///
    /// | 前缀 | 指向 |
    /// | :--- | :--- |
    /// | `project:` | 工程级构造（词汇表保真度 / 轨道表 / 段落 / 场景 / 路由 / 资产 …） |
    /// | `track:` | 一条音轨 |
    /// | `clip:` | 一个片段池条目 |
    /// | `clip-placement:` | 一次时间轴摆放 |
    pub entity: String,
    /// 人话解释。**前缀**是机器可读的两分法（见模块头）。
    pub reason: String,
}

/// `ProjectData` 的产物 + 损失表 + 映射计数。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogicProjectData {
    /// `Alternatives/<NNN>/ProjectData` 的完整字节。
    pub bytes: Vec<u8>,
    /// 映射损失表（对任何含未映射构造的工程都非空，且从不静默为空）。
    pub losses: Vec<LogicLoss>,
    /// 写进 `ProjectData` 的 region 条数（= 成功映射的 MIDI 摆放数）。
    pub mapped_regions: usize,
    /// 写进 `ProjectData` 的音符事件数。
    pub mapped_notes: usize,
}

/// 一个 `.logicx` bundle：**相对路径 → 字节**。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogicBundle {
    /// 包内相对路径 → 文件字节（`BTreeMap` ⇒ 迭代顺序确定）。
    pub files: BTreeMap<String, Vec<u8>>,
    /// 映射损失表（与 [`LogicProjectData::losses`] 同一份，且同时写进 `MetaData.plist`）。
    pub losses: Vec<LogicLoss>,
    /// 同 [`LogicProjectData::mapped_regions`]。
    pub mapped_regions: usize,
    /// 同 [`LogicProjectData::mapped_notes`]。
    pub mapped_notes: usize,
    /// 规范化后的 alternative 目录名（三位十进制，如 `"004"`）。
    pub alternative: String,
}

/// 取出 `qSvE` 音符序列体（供调用方/判据复用，不重新发明事件行模型）。
///
/// 返回 `(起始 tick, 时值 tick, 音高, 力度)`，起始 tick 已扣掉
/// [`LOGIC_NOTE_ORIGIN_TICKS`]（即回到"region 内相对 tick + 摆放起点"的口径）。
#[must_use]
pub fn decode_note_lines(body: &[u8]) -> Vec<(u64, u64, u8, u8)> {
    let mut notes = Vec::new();
    let mut at = 0usize;
    while at + LOGIC_EVENT_LINE_SIZE <= body.len() {
        let line = &body[at..at + LOGIC_EVENT_LINE_SIZE];
        if is_continuation(line) {
            break;
        }
        let mut next = at + LOGIC_EVENT_LINE_SIZE;
        while next + LOGIC_EVENT_LINE_SIZE <= body.len()
            && is_continuation(&body[next..next + LOGIC_EVENT_LINE_SIZE])
        {
            next += LOGIC_EVENT_LINE_SIZE;
        }
        if line[0] == LOGIC_NOTE_STATUS {
            let raw = u64::from(u32::from_le_bytes([line[4], line[5], line[6], line[7]]));
            let start = raw.saturating_sub(LOGIC_NOTE_ORIGIN_TICKS);
            let duration = if next > at + LOGIC_EVENT_LINE_SIZE {
                let continuation = &body[at + LOGIC_EVENT_LINE_SIZE..next];
                u64::from(u32::from_le_bytes([
                    continuation[0x0c],
                    continuation[0x0d],
                    continuation[0x0e],
                    continuation[0x0f],
                ]))
            } else {
                0
            };
            notes.push((start, duration, line[0x0c], line[0x0b]));
        }
        at = next;
    }
    notes
}

/// 一个事件行是否"续上一事件"（第 7 字节最高位）。
#[must_use]
pub fn is_continuation(line: &[u8]) -> bool {
    line.get(7)
        .is_some_and(|byte| byte & LOGIC_CONTINUATION_FLAG != 0)
}

/// 把工程映射成 `ProjectData` 字节 + 损失表（纯函数、无 I/O、确定性）。
#[must_use]
pub fn project_data(project: &YebanProjectV1) -> LogicProjectData {
    let mut builder = LogicBuilder::new();
    builder.records.push(song_record(project.bpm));
    builder.records.push(meter_record(
        project.time_signature.numerator,
        project.time_signature.denominator,
    ));
    builder.records.push(tempo_record(project.bpm));

    builder.write_project_losses(project);

    for track in project.tracks.values() {
        builder.write_track(project, track);
    }

    // 片段池里没有被任何摆放引用的条目（否则它们会被静默丢掉）。
    let referenced: BTreeSet<EntityId> = project
        .tracks
        .values()
        .flat_map(|track| track.clips.values().map(|placement| placement.clip_id))
        .collect();
    for entry in project.clip_pool.values() {
        if !referenced.contains(&entry.id) {
            builder.loss(
                format!("clip:{}#{}", entry.name, entry.id.to_canonical_string()),
                format!(
                    "{LOSS_UNMAPPED_PREFIX} 片段池条目没有被任何时间轴摆放引用\
                     （池里 {}/{} 条被引用）⇒ 不导出",
                    referenced.len(),
                    project.clip_pool.len()
                ),
            );
        }
    }

    let bytes = root_document(&builder.records);
    LogicProjectData {
        bytes,
        losses: builder.losses,
        mapped_regions: builder.mapped_regions,
        mapped_notes: builder.mapped_notes,
    }
}

/// `Alternatives/<NNN>/MetaData.plist`：标准二进制 plist（`bplist00`）。
///
/// 键集合取自 groove 的 `logicMetaDataPlist`（它按本机三个真实工程取并集），
/// 再补一个夜半扩展键 [`META_DATA_LOSS_KEY`]。**刻意不写** `SongKey` /
/// `SongGenderKey` / `SignatureKey`：夜半的工程没有调性字段，写 `"C"/"major"` 等于
/// 替作者声明一件他没说过的事（groove 的写入器同样刻意不写）。
#[must_use]
pub fn meta_data_plist(project: &YebanProjectV1, losses: &[LogicLoss]) -> Vec<u8> {
    meta_data_plist_with_track_count(project, losses, project.tracks.len())
}

/// 同 [`meta_data_plist`]，但 `NumberOfTracks` 由调用方给定。
///
/// 供体路线必须走这一条：供体只携带 [`LOGIC_DONOR_TRACK_COUNT`] 条编排轨行，把工程轨道**总数**
/// 写进去会让 Logic 按一个不存在的轨道数去找通道簇（参考实现 §10.6.6 记 `MetaData.plist` 的
/// `NumberOfTracks` 与文件不符会让 Logic **拒绝**该文件）。
#[must_use]
pub fn meta_data_plist_with_track_count(
    project: &YebanProjectV1,
    losses: &[LogicLoss],
    track_count: usize,
) -> Vec<u8> {
    use PlistValue::{Array, Bool, Integer, Real, Text};

    let empty = || Array(Vec::new());
    let empty_keys = [
        "PlaybackFiles",
        "UnusedAudioFiles",
        "AudioFiles",
        "AlchemyFiles",
        "QuicksamplerFiles",
        "UltrabeatFiles",
        "SamplerInstrumentsFiles",
        "ImpulsResponsesFiles",
        "VideoFiles",
    ];

    let mut entries: Vec<(String, PlistValue)> = vec![
        ("BeatsPerMinute".to_owned(), Real(project.bpm)),
        (
            "SongSignatureNumerator".to_owned(),
            Integer(i64::from(project.time_signature.numerator)),
        ),
        (
            "SongSignatureDenominator".to_owned(),
            Integer(i64::from(project.time_signature.denominator)),
        ),
        ("NumberOfTracks".to_owned(), Integer(track_count as i64)),
        (
            "SampleRate".to_owned(),
            Integer(i64::from(project.audio_config.sample_rate.hz())),
        ),
        ("FrameRateIndex".to_owned(), Integer(1)),
        ("SurroundFormatIndex".to_owned(), Integer(5)),
        ("Version".to_owned(), Integer(3)),
        ("isTimeCodeBased".to_owned(), Bool(false)),
        ("HasARAPlugins".to_owned(), Bool(false)),
        ("HasGrid".to_owned(), Bool(false)),
    ];
    for key in empty_keys {
        entries.push((key.to_owned(), empty()));
    }
    entries.push((
        META_DATA_LOSS_KEY.to_owned(),
        Array(
            losses
                .iter()
                .map(|loss| Text(format!("{}: {}", loss.entity, loss.reason)))
                .collect(),
        ),
    ));

    encode_binary_plist(&PlistValue::Dict(entries))
}

/// `Resources/ProjectInformation.plist`：标准二进制 plist，`ActiveVariant` 是**整数**。
///
/// 实测：真实工程的目录是 `004` 而该键是 `4`（`ocean eyes` 是 `001` / `1`），
/// 因此这里把同一个编号既写进目录名也写进这个键。
#[must_use]
pub fn project_information_plist(alternative_index: u32, variant_name: &str) -> Vec<u8> {
    use PlistValue::{Bool, Dict, Integer, Real, Text};

    let key = alternative_index.to_string();
    let names = || Dict(vec![(key.clone(), Text(variant_name.to_owned()))]);
    encode_binary_plist(&PlistValue::Dict(vec![
        (
            "ActiveVariant".to_owned(),
            Integer(i64::from(alternative_index)),
        ),
        ("BundleVersion".to_owned(), Real(2.0)),
        ("HasProjectFolder".to_owned(), Bool(false)),
        ("VariantNames".to_owned(), names()),
        ("VariantNamesV2".to_owned(), names()),
    ]))
}

/// `Alternatives/<NNN>/DisplayState.plist`：实测的五个键，值一律取空形态。
///
/// 本写入器没有屏幕可描述，凭空编造窗口尺寸是对一个从未存在过的界面的谎言；
/// 真实工程里的另外两个文件（`WindowImage.jpg` 缩略图、`DisplayStateArchive` 不透明归档）
/// **不写**，并登记为损失。
#[must_use]
pub fn display_state_plist() -> Vec<u8> {
    use PlistValue::{Array, Dict, Integer};

    encode_binary_plist(&PlistValue::Dict(vec![
        ("displayDataVersion".to_owned(), Integer(1)),
        ("docPreferences".to_owned(), Dict(Vec::new())),
        ("screenVisibleFrames".to_owned(), Array(Vec::new())),
        ("screensetCurrSlot".to_owned(), Integer(0)),
        ("screensetDictArray".to_owned(), Array(Vec::new())),
    ]))
}

/// 组装一个 `.logicx` bundle（纯函数，零文件系统 I/O）。
///
/// 写四个文件（与 groove 的 `logicProjectBundle` 同形）：`Alternatives/<alt>/ProjectData`、
/// 同目录的 `MetaData.plist` 与 `DisplayState.plist`，以及
/// `Resources/ProjectInformation.plist`（它点名哪个 alternative 是活动的 —— 真实工程的
/// 编号**不是**永远 `000`，所以这个文件必须写）。
///
/// `alternative` 允许 `"4"` 或 `"004"`，一律规范成三位；不可解析时退回 `"000"` 并登记一条损失。
#[must_use]
pub fn build_bundle(
    project: &YebanProjectV1,
    alternative: &str,
    variant_name: &str,
) -> LogicBundle {
    let track_count = project.tracks.len();
    assemble_bundle(
        project,
        alternative,
        variant_name,
        project_data(project),
        track_count,
    )
}

/// [`build_bundle`] 与 [`build_bundle_from_donor`] 共用的装配体：四个文件 + 损失表。
///
/// `track_count` 是写进 `MetaData.plist` `NumberOfTracks` 的那个数 —— 自研写入器传**工程轨道
/// 总数**（它的 region 不落轨，语义见模块头），供体路线传**实际映射的轨道数**（供体只有
/// [`LOGIC_DONOR_TRACK_COUNT`] 条编排轨行）。两者都不是"猜"：调用方各自给得出证据。
fn assemble_bundle(
    project: &YebanProjectV1,
    alternative: &str,
    variant_name: &str,
    data: LogicProjectData,
    track_count: usize,
) -> LogicBundle {
    let mut losses = data.losses;
    let index = alternative_index(alternative);
    let alternative_name = match index {
        Some(value) => format!("{value:03}"),
        None => {
            losses.push(LogicLoss {
                entity: project_entity(project),
                reason: format!(
                    "{LOSS_UNMAPPED_PREFIX} alternative 编号 `{alternative}` 不是 0..=999 的十进制整数\
                     ⇒ 目录退回 `000`（时间轴上的另一种静默：不发明一个编号）"
                ),
            });
            "000".to_owned()
        }
    };
    let index = index.unwrap_or(0);

    let mut files = BTreeMap::new();
    files.insert(
        format!("Alternatives/{alternative_name}/ProjectData"),
        data.bytes,
    );
    files.insert(
        format!("Alternatives/{alternative_name}/MetaData.plist"),
        meta_data_plist_with_track_count(project, &losses, track_count),
    );
    files.insert(
        format!("Alternatives/{alternative_name}/DisplayState.plist"),
        display_state_plist(),
    );
    files.insert(
        "Resources/ProjectInformation.plist".to_owned(),
        project_information_plist(index, variant_name),
    );

    LogicBundle {
        files,
        losses,
        mapped_regions: data.mapped_regions,
        mapped_notes: data.mapped_notes,
        alternative: alternative_name,
    }
}

/* ------------------------------------------------------------------ *
 * 供体模板（donor）路线 —— 参考实现**经 Logic 验证**的轨道来源
 * ------------------------------------------------------------------ */

/// 供体 `ProjectData` 的字节：`jonkubis/logicproformatwriter` 的 Logic 夹具 `F0_baseline`
/// （**MIT**，"Copyright (c) 2026 Jon Kubis"），随仓库分发在
/// `crates/yeban-render/assets/logic-donor/`（同目录 `LICENSE` = 上游 MIT 全文、
/// `README.md` = 来源与改动登记）。
///
/// 为什么必须是**别人存过的**文档：参考实现（MIT，§10.6.1）的实测结论是轨道**不能凭空合成** ——
/// 新建一条通道会触发 Logic 的混音器 / CoreMIDI Environment 扩张，重新生成 time-UUID 并重排
/// 整个 `OCuA` 通道块（"pervasive and impractical to reproduce"）。它自己验证过的做法是克隆
/// 一份 Logic 存过的 donor 的通道簇。本仓库因此改用**可再分发的 MIT 供体**，而不是 Apple 的
/// 演示工程（有版权，本仓库不提交、不读取、不复制）。
pub const LOGIC_DONOR_PROJECT_DATA: &[u8] =
    include_bytes!("../assets/logic-donor/Alternatives/000/ProjectData");

/// 供体 `ProjectData` 的字节数（实测，`stat -f%z` 口径）。
pub const LOGIC_DONOR_PROJECT_DATA_BYTES: usize = 127_689;

/// 供体 `ProjectData` 的 sha256（`shasum -a 256`，实测）。
pub const LOGIC_DONOR_SHA256: &str =
    "8a5ec7371e89f07fa53e873725c856bbe29eeef1dac14ff54df8b23962dde893";

/// 供体的根头格式版本码（实测 `0x09CF`）。
///
/// ⚠ 供体路线**保留供体自己的根头**（含这个版本码），不改写成 [`LOGIC_FORMAT_VERSION_CODE`]：
/// 版本码声明的是**这份文档的落盘格式**，而供体的记录是 2511 形态；改成一个更新的声明只会让
/// Logic 用更新的解析器去读更老的记录。自研写入器的产物仍写 `0x09D0`。
pub const LOGIC_DONOR_VERSION_CODE: u16 = 0x09CF;

/// 供体的记录条数（实测：按 36 字节记录头的载荷长度 `u32` 小端 @+0x1c 驱动走完
/// {@link LOGIC_DONOR_PROJECT_DATA_BYTES} 字节，恰好停在 EOF）。
pub const LOGIC_DONOR_RECORD_COUNT: usize = 527;

/// 供体 `MetaData.plist` 的 `NumberOfTracks`（实测 `1`），也是供体携带的**编排轨行**条数
/// （`Trak` 记录头 `+0x08 == 0x00040000` 的非 0 载荷 2 条 − 1 条 master 行）。
pub const LOGIC_DONOR_TRACK_COUNT: usize = 1;

/// 供体那个**被摆放的 MIDI region**（`qeSM`）的对象号（实测 `0x00e40000`）。
///
/// 实测依据：供体 13 条 `MSeq` 里只有它落在这个对象号上，且它的配对 `qSvE`（同对象号、
/// 同 subtype `14`）载荷就是空的 16 字节尾 —— 即"一个被摆放的空 MIDI region"。
pub const LOGIC_DONOR_REGION_CLUSTER: u32 = 0x00E4_0000;

/// 供体被摆放 region 的 `qeSM` **载荷**长度（实测 305 字节）。
pub const LOGIC_DONOR_REGION_PAYLOAD_BYTES: usize = 305;

/// 供体 region 载荷里名字字段之后的**第一个非零字段**的载荷偏移（实测 `+0x57`）。
pub const LOGIC_DONOR_REGION_FIRST_OTHER_FIELD: usize = 0x57;

/// 供体 region 载荷里名字字段的**原地容量**（字节）。
///
/// 名字长度 `u16` 在载荷 `+0x10`、名字从 `+0x12` 起，名字之后第一个非零字段在
/// `+0x57`，因此在 `+0x12` 处最多写 `0x57 − 0x12 = 69` 字节就不会移动供体在 `+0x57`
/// 及其后的任何字节。更长的名字会被截断并登记（`非等价:`）。
pub const LOGIC_DONOR_REGION_NAME_CAPACITY: usize =
    LOGIC_DONOR_REGION_FIRST_OTHER_FIELD - LOGIC_REGION_NAME_PAYLOAD_OFFSET - 2;

/// 供体 `gnoS` 的载荷长度（实测 10,756 字节 = 参考实现说的 compact 形态 ≈10,792 字节一族）。
pub const LOGIC_DONOR_SONG_PAYLOAD_BYTES: usize = 10_756;

/// 供体 `gnoS` 载荷里**实测**内嵌的初始速度三连的偏移（值 `round(120 × 10000)` = 1,200,000）。
///
/// ⚠ 这与参考实现给 settled 模板的槽位（`gnoS` 载荷 `+0x92`/`+0xEA`/`+0x3A6`）**不同**：
/// 供体里这三个槽位实测**全是 0**（compact 布局）。因此本模块**不**改 `gnoS` 的速度，
/// 只把这三处登记为供体的（参考实现 §10.4 也记 "compact base 的速度/拍号写入器只动独立的
/// `qSvE`、从不动 `gnoS`"）。
pub const LOGIC_DONOR_SONG_TEMPO_OFFSETS: [usize; 3] = [0x6F, 0x73, 0xC6];

/// 供体逐 chunk 家族的实测条数（合计 = [`LOGIC_DONOR_RECORD_COUNT`]）。
pub const LOGIC_DONOR_CHUNK_FAMILIES: [(&str, usize); 18] = [
    ("AuCO", 376),
    ("TxSt", 32),
    ("Styl", 32),
    ("Trak", 22),
    ("MSeq", 13),
    ("EvSq", 13),
    ("AuCn", 13),
    ("Envi", 12),
    ("Hypr", 3),
    ("SngO", 2),
    ("CorM", 2),
    ("Song", 1),
    ("InSt", 1),
    ("Layr", 1),
    ("ScSt", 1),
    ("Vide", 1),
    ("AuCU", 1),
    ("GenM", 1),
];

/// 供体路线**只改这 4 条**记录：两个全局 `qSvE`（拍号 / 速度）+ 被摆放 region 的 `qeSM` 名字
/// + 该 region 的配对音符 `qSvE`。其余记录逐字节是供体的。
pub const LOGIC_DONOR_PATCHED_RECORDS: usize = 4;

/// 供体记录（只读视图：整条记录 + 解码后的头字段）。
struct DonorRecord<'a> {
    full: &'a [u8],
    tag: [u8; 4],
    kind: u16,
    subtype: u16,
    cluster: u32,
}

impl DonorRecord<'_> {
    /// 载荷（记录头之后的部分）。
    fn payload(&self) -> &[u8] {
        &self.full[LOGIC_RECORD_HEADER..]
    }
}

/// 按 36 字节记录头的载荷长度（`u32` 小端 @+0x1c）走完供体的整个 chunk 流。
///
/// 行走**只**由载荷长度驱动；结束位置必须恰好等于 EOF（否则说明供体的形状与钉住的不符）。
fn donor_records(bytes: &[u8]) -> Result<Vec<DonorRecord<'_>>, String> {
    if bytes.len() < LOGIC_ROOT_HEADER || bytes[..4] != LOGIC_ROOT_MAGIC {
        return Err("根魔数不符".to_owned());
    }
    let mut out = Vec::new();
    let mut at = LOGIC_FIRST_RECORD_OFFSET;
    while at < bytes.len() {
        if at + LOGIC_RECORD_HEADER > bytes.len() {
            return Err(format!("记录头越过 EOF（偏移 {at:#x}）"));
        }
        let size = u32::from_le_bytes([
            bytes[at + LOGIC_RECORD_SIZE_OFFSET],
            bytes[at + LOGIC_RECORD_SIZE_OFFSET + 1],
            bytes[at + LOGIC_RECORD_SIZE_OFFSET + 2],
            bytes[at + LOGIC_RECORD_SIZE_OFFSET + 3],
        ]) as usize;
        let end = at + LOGIC_RECORD_HEADER + size;
        if end > bytes.len() {
            return Err(format!("载荷越过 EOF（记录 {at:#x}，长度 {size}）"));
        }
        out.push(DonorRecord {
            full: &bytes[at..end],
            tag: [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]],
            kind: u16::from_le_bytes([bytes[at + LOGIC_RECORD_KIND_OFFSET], bytes[at + 5]]),
            subtype: u16::from_le_bytes([bytes[at + LOGIC_RECORD_SUBTYPE_OFFSET], bytes[at + 7]]),
            cluster: u32::from_le_bytes([
                bytes[at + LOGIC_RECORD_CLUSTER_OFFSET],
                bytes[at + 9],
                bytes[at + 10],
                bytes[at + 11],
            ]),
        });
        at = end;
    }
    if at != bytes.len() {
        return Err("记录流没有恰好铺满文件".to_owned());
    }
    Ok(out)
}

/// 在供体里找**恰好一条**满足 `(tag, cluster, 载荷首字节)` 的记录。
///
/// 0 条或 >1 条都返回 `None`：供体的形状由判据钉住，多义时宁可退回自研写入器（并登记），
/// 也不猜。
fn donor_unique_index(
    records: &[DonorRecord<'_>],
    tag: [u8; 4],
    cluster: u32,
    first: Option<u8>,
) -> Option<usize> {
    let mut found = None;
    for (index, record) in records.iter().enumerate() {
        let first_ok = match first {
            Some(byte) => record.payload().first() == Some(&byte),
            None => true,
        };
        if record.tag == tag && record.cluster == cluster && first_ok {
            if found.is_some() {
                return None;
            }
            found = Some(index);
        }
    }
    found
}

/// 供体路线的产物：供体的根头 + 记录流（供体顺序）+ 映射计数。
struct DonorSplice {
    header: [u8; LOGIC_ROOT_HEADER],
    records: Vec<Vec<u8>>,
    mapped_regions: usize,
    mapped_notes: usize,
    mapped_tracks: usize,
    mapped_track_id: Option<EntityId>,
    /// 被映射摆放的起点（tick）；没有映射时 0。region 自身落在供体的第 1 小节，见损失表。
    mapped_start_tick: u64,
    /// 有音符的 tick 超出 `u32`（已饱和）时为真。
    clamped: bool,
    name_truncated: bool,
}

/// 供体的通道簇 + 我们自己的"歌"。
///
/// 记录流**保持供体的顺序**，只动四条记录（见 [`LOGIC_DONOR_PATCHED_RECORDS`]）：
/// 全局拍号 `qSvE`、全局速度 `qSvE`、被摆放 region 的 `qeSM` 名字、该 region 的配对音符 `qSvE`。
/// 返回 `None` 表示供体的形状与本模块钉住的实测不符（此时调用方退回自研写入器并登记）。
fn donor_splice(project: &YebanProjectV1) -> Option<DonorSplice> {
    let donor = LOGIC_DONOR_PROJECT_DATA;
    if donor.len() != LOGIC_DONOR_PROJECT_DATA_BYTES {
        return None;
    }
    let parsed = donor_records(donor).ok()?;
    if parsed.len() != LOGIC_DONOR_RECORD_COUNT {
        return None;
    }
    let meter_index = donor_unique_index(
        &parsed,
        LOGIC_SEQUENCE_TAG,
        0,
        Some(LOGIC_METER_MARKER as u8),
    )?;
    let tempo_index = donor_unique_index(
        &parsed,
        LOGIC_SEQUENCE_TAG,
        0,
        Some(LOGIC_TEMPO_MARKER as u8),
    )?;
    let region_index =
        donor_unique_index(&parsed, LOGIC_REGION_TAG, LOGIC_DONOR_REGION_CLUSTER, None)?;
    let note_index = donor_unique_index(
        &parsed,
        LOGIC_SEQUENCE_TAG,
        LOGIC_DONOR_REGION_CLUSTER,
        None,
    )?;
    if parsed[region_index].payload().len() != LOGIC_DONOR_REGION_PAYLOAD_BYTES {
        return None;
    }
    // 供体的形状不变量（参考实现 §10.6.2 / 问题 3）：每条 `qeSM` 与它配对的 `qSvE` 共享同一个
    // 对象号**与同一个 subtype**，且 kind 分别是 5 / 1（本机 2509+ 实测）。
    if parsed[region_index].kind != LOGIC_REGION_KIND
        || parsed[note_index].kind != LOGIC_SEQUENCE_KIND
        || parsed[region_index].subtype != parsed[note_index].subtype
        || parsed[meter_index].kind != LOGIC_SEQUENCE_KIND
        || parsed[tempo_index].kind != LOGIC_SEQUENCE_KIND
    {
        return None;
    }

    let mut records: Vec<Vec<u8>> = parsed.iter().map(|record| record.full.to_vec()).collect();

    // (1) 拍号：载荷 +0x0b = 分母的以 2 为底指数、+0x0c = 分子（实测，全长 96 字节不变）。
    let exponent = project.time_signature.denominator.trailing_zeros() as u8;
    records[meter_index][LOGIC_RECORD_HEADER + LOGIC_METER_DENOMINATOR_EXPONENT_OFFSET] = exponent;
    records[meter_index][LOGIC_RECORD_HEADER + LOGIC_METER_NUMERATOR_OFFSET] =
        project.time_signature.numerator;

    // (2) 速度：载荷 +0x10 的 u32 = round(bpm × 10000)（实测，全长 48 字节不变）。
    put_u32_le(
        &mut records[tempo_index],
        LOGIC_RECORD_HEADER + LOGIC_TEMPO_VALUE_OFFSET,
        tempo_ticks(project.bpm),
    );

    // (3) 我们自己的"歌"：供体只有 LOGIC_DONOR_TRACK_COUNT 条编排轨行，因此最多映射
    //     一条 MIDI 轨的**第一个** MIDI 摆放。
    let mut mapped: Option<(&TrackV3, &ClipPlacement, &yeban_model::ClipPoolEntry)> = None;
    'tracks: for track in project.tracks.values() {
        if track.kind != TrackKind::Midi {
            continue;
        }
        for placement in track.clips.values() {
            if let Some(entry) = project.clip_pool.get(&placement.clip_id)
                && matches!(entry.content, ClipContent::Midi { .. })
            {
                mapped = Some((track, placement, entry));
                break 'tracks;
            }
        }
    }

    let (region_name, lines, clamped) = match mapped {
        Some((track, placement, entry)) => {
            let name = if entry.name.is_empty() {
                track.name.clone()
            } else {
                entry.name.clone()
            };
            let notes = match &entry.content {
                ClipContent::Midi { notes } => notes,
                ClipContent::Audio { .. } => {
                    // `mapped` 的选择已经排除了音频内容；这里保持分支穷尽而不是 `unwrap`。
                    return None;
                }
            };
            let mut lines = Vec::with_capacity(notes.len());
            let mut clamped = false;
            for note in notes.values() {
                let absolute = placement
                    .start_tick
                    .checked_add(note.start_tick)
                    .and_then(|ticks| ticks.checked_add(LOGIC_NOTE_ORIGIN_TICKS));
                let start = match absolute.and_then(|ticks| u32::try_from(ticks).ok()) {
                    Some(value) => value,
                    None => {
                        clamped = true;
                        u32::MAX
                    }
                };
                let duration = match u32::try_from(note.duration_ticks) {
                    Ok(value) => value,
                    Err(_) => {
                        clamped = true;
                        u32::MAX
                    }
                };
                lines.push(WrittenNote {
                    start,
                    duration,
                    pitch: note.pitch,
                    velocity: note.velocity,
                });
            }
            (name, lines, clamped)
        }
        None => (String::new(), Vec::new(), false),
    };

    // (3) region 名字：原地写（载荷长度 305 不变 ⇒ 供体在 +0x57 及其后的字节一个都不动）。
    let name_bytes = utf8_prefix(&region_name, LOGIC_DONOR_REGION_NAME_CAPACITY).as_bytes();
    let name_truncated = name_bytes.len() < region_name.len();
    let name_at = LOGIC_RECORD_HEADER + LOGIC_REGION_NAME_PAYLOAD_OFFSET;
    put_u16_le(&mut records[region_index], name_at, name_bytes.len() as u16);
    for byte in &mut records[region_index]
        [name_at + 2..LOGIC_RECORD_HEADER + LOGIC_DONOR_REGION_FIRST_OTHER_FIELD]
    {
        *byte = 0;
    }
    records[region_index][name_at + 2..name_at + 2 + name_bytes.len()].copy_from_slice(name_bytes);

    // (4) region 的配对音符序列：**只换载荷**，记录头逐字节保留供体的（kind/subtype/cluster
    //     与 +0x0c..+0x16 的实测字）。
    let note_payload = note_lines(&lines);
    let mut rebuilt = records[note_index][..LOGIC_RECORD_HEADER].to_vec();
    rebuilt.extend_from_slice(&note_payload);
    put_u32_le(
        &mut rebuilt,
        LOGIC_RECORD_SIZE_OFFSET,
        note_payload.len() as u32,
    );
    records[note_index] = rebuilt;

    let mut header = [0u8; LOGIC_ROOT_HEADER];
    header.copy_from_slice(&donor[..LOGIC_ROOT_HEADER]);

    Some(DonorSplice {
        header,
        records,
        mapped_regions: usize::from(mapped.is_some()),
        mapped_notes: lines.len(),
        mapped_tracks: usize::from(mapped.is_some()),
        mapped_track_id: mapped.map(|(track, _, _)| track.id),
        mapped_start_tick: mapped.map_or(0, |(_, placement, _)| placement.start_tick),
        clamped,
        name_truncated,
    })
}

/// 用**给定的**根头（供体的 24 字节）+ 记录流拼出 `ProjectData`，只把声明长度改写成实际值。
///
/// 与 [`root_document`] 的唯一区别：后者写本模块自己的版本码与十个恒定字节，前者保留供体的。
fn root_document_from_header(header: &[u8], records: &[Vec<u8>]) -> Vec<u8> {
    let payload: usize = records.iter().map(Vec::len).sum();
    let mut out = vec![0u8; LOGIC_ROOT_HEADER + payload];
    out[..LOGIC_ROOT_HEADER].copy_from_slice(&header[..LOGIC_ROOT_HEADER]);
    put_u32_le(&mut out, LOGIC_DECLARED_LENGTH_OFFSET, payload as u32);
    let mut at = LOGIC_ROOT_HEADER;
    for entry in records {
        out[at..at + entry.len()].copy_from_slice(entry);
        at += entry.len();
    }
    out
}

/// 供体路线的 `ProjectData` + 实际映射的轨道数（`MetaData.plist` 的 `NumberOfTracks`）。
fn donor_project_data(project: &YebanProjectV1) -> (LogicProjectData, usize) {
    let mut builder = LogicBuilder::new();
    match donor_splice(project) {
        Some(splice) => {
            builder.write_donor_losses(project, &splice);
            let bytes = root_document_from_header(&splice.header, &splice.records);
            (
                LogicProjectData {
                    bytes,
                    losses: builder.losses,
                    mapped_regions: splice.mapped_regions,
                    mapped_notes: splice.mapped_notes,
                },
                splice.mapped_tracks,
            )
        }
        None => {
            let mut data = project_data(project);
            data.losses.insert(
                0,
                LogicLoss {
                    entity: project_entity(project),
                    reason: format!(
                        "{LOSS_NOT_EQUIVALENT_PREFIX} 供体 `ProjectData` 的形状与本模块钉住的实测不符\
                         （{} 字节 / {} 条记录 / 被摆放 region 的对象号 {:#010x}）⇒ 本次退回自研写入器，\
                         产物**不含**供体的通道簇",
                        LOGIC_DONOR_PROJECT_DATA_BYTES,
                        LOGIC_DONOR_RECORD_COUNT,
                        LOGIC_DONOR_REGION_CLUSTER
                    ),
                },
            );
            (data, 0)
        }
    }
}

/// 供体路线的 `ProjectData`：**供体的通道簇** + 我们的拍号 / 速度 / region 名 / 音符。
///
/// 其余一切（`Envi` / `AuCO` / `GenM` / `Trak` 轨道表、混音槽、环境对象、`gnoS` 正文）逐字节是
/// 供体的，并**逐族**登记在损失表里（[`LogicBuilder::write_donor_losses`]）。
#[must_use]
pub fn project_data_from_donor(project: &YebanProjectV1) -> LogicProjectData {
    donor_project_data(project).0
}

/// 供体路线的 bundle（四个文件），`MetaData.plist` 的 `NumberOfTracks` = **实际映射**的轨道数。
#[must_use]
pub fn build_bundle_from_donor(
    project: &YebanProjectV1,
    alternative: &str,
    variant_name: &str,
) -> LogicBundle {
    let (data, mapped_tracks) = donor_project_data(project);
    assemble_bundle(project, alternative, variant_name, data, mapped_tracks)
}

/* ------------------------------------------------------------------ *
 * 记录构造
 * ------------------------------------------------------------------ */

/// 逐记录拼 `ProjectData`。
struct LogicBuilder {
    records: Vec<Vec<u8>>,
    losses: Vec<LogicLoss>,
    mapped_regions: usize,
    mapped_notes: usize,
    next_cluster: u32,
}

impl LogicBuilder {
    fn new() -> Self {
        Self {
            records: Vec::new(),
            losses: Vec::new(),
            mapped_regions: 0,
            mapped_notes: 0,
            next_cluster: 1,
        }
    }

    fn loss(&mut self, entity: String, reason: String) {
        self.losses.push(LogicLoss { entity, reason });
    }

    fn write_project_losses(&mut self, project: &YebanProjectV1) {
        let entity = project_entity(project);
        self.loss(entity.clone(), LOGIC_OPEN_SCOPE_CAVEAT.to_owned());
        self.loss(entity.clone(), TRACK_OBJECTS_UNMAPPED.to_owned());
        self.loss(entity.clone(), REGION_PLACEMENT_UNMAPPED.to_owned());
        self.loss(entity.clone(), REGION_TIMING_CAVEAT.to_owned());
        self.loss(entity.clone(), REGION_PAYLOAD_CAVEAT.to_owned());
        self.loss_meta_data_key(&entity);
        self.loss_tempo_quantisation(&entity, project.bpm);
        self.loss_missing_key(&entity, project.tracks.len());
        self.loss(
            entity.clone(),
            format!(
                "{LOSS_UNMAPPED_PREFIX} 真实工程的另外三个 alternative 文件\
                 （`WindowImage.jpg`、`DisplayStateArchive`、`Undo Data.nosync`）不写 —— \
                 它们是不透明二进制/缩略图，本切片不伪造"
            ),
        );
        // 逐族点名真实工程有、本写入器不写的 chunk（实测差集，见 MISSING_CHUNK_FAMILIES）。
        for (name, role) in MISSING_CHUNK_FAMILIES {
            self.loss(
                entity.clone(),
                format!(
                    "{LOSS_UNMAPPED_PREFIX} 真实工程的 chunk 家族 `{name}`（落盘字节 `{}`）\
                     没有写入 —— {role}",
                    stored_chunk_tag(name)
                ),
            );
        }
        self.loss(entity.clone(), CONTAINER_HEADER_CAVEAT.to_owned());
        self.write_container_losses(&entity, project);
    }

    /// 工程级容器（段落 / 场景 / 片段池 / 资产 / 路由边）里**没有 Logic 侧对应物**的那些。
    ///
    /// 抽出来是因为供体路线也要登记同一批东西：供体的通道簇没有让它们长出 Logic 表示。
    fn write_container_losses(&mut self, entity: &str, project: &YebanProjectV1) {
        let mut unmapped_containers: Vec<(&str, usize)> = Vec::new();
        if !project.sections.is_empty() {
            unmapped_containers.push(("段落 sections", project.sections.len()));
        }
        if !project.scenes.is_empty() {
            unmapped_containers.push(("场景 scenes", project.scenes.len()));
        }
        if !project.clip_pool.is_empty() {
            unmapped_containers.push(("片段池 clip_pool", project.clip_pool.len()));
        }
        if !project.assets.is_empty() {
            unmapped_containers.push(("资产索引 assets", project.assets.len()));
        }
        if !project.routing_graph.edges.is_empty() {
            unmapped_containers.push(("路由边 routing_graph", project.routing_graph.edges.len()));
        }
        for (name, count) in unmapped_containers {
            self.loss(
                entity.to_owned(),
                format!("{LOSS_UNMAPPED_PREFIX} {name} 共 {count} 条没有 Logic 侧对应物 ⇒ 不导出"),
            );
        }
    }

    /// 一条轨道的**非 region** 属性（混音 / 设备 / 宏 / 自动化 / 色标 / `folder_id`）。
    ///
    /// 供体路线与自研路线共用：两条路线都写不出这些，区别只在于通道簇来自哪里。
    fn write_track_caveats(&mut self, entity: &str, track: &TrackV3) {
        if track.volume_db != 0.0 || track.pan != 0.0 || track.mute || track.solo || track.solo_safe
        {
            self.loss(
                entity.to_owned(),
                format!(
                    "{LOSS_UNMAPPED_PREFIX} 轨道混音（volume_db={} / pan={} / mute={} / solo={} / \
                     solo_safe={}）不在本切片子集内 ⇒ 只导出 region 与音符",
                    track.volume_db, track.pan, track.mute, track.solo, track.solo_safe
                ),
            );
        }
        if !track.devices.is_empty() {
            self.loss(
                entity.to_owned(),
                format!(
                    "{LOSS_UNMAPPED_PREFIX} 设备链 {} 个（{}）没有 Logic 侧的 AU/插件对象 ⇒ 不导出",
                    track.devices.len(),
                    track
                        .devices
                        .iter()
                        .map(|device| device.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
        }
        if !track.macros.is_empty() {
            self.loss(
                entity.to_owned(),
                format!(
                    "{LOSS_UNMAPPED_PREFIX} 宏 {} 个没有 Logic 侧对应物 ⇒ 不导出",
                    track.macros.len()
                ),
            );
        }
        if !track.automation_lanes.is_empty() {
            self.loss(
                entity.to_owned(),
                format!(
                    "{LOSS_UNMAPPED_PREFIX} 自动化泳道 {} 条（Logic 的自动化是另一族 chunk）⇒ 不导出",
                    track.automation_lanes.len()
                ),
            );
        }
        if let Some(color) = &track.color {
            self.loss(
                entity.to_owned(),
                format!(
                    "{LOSS_UNMAPPED_PREFIX} 轨道色标 {color} 的 Logic 整数编码不在仓库内 ⇒ 不导出"
                ),
            );
        }
        if track.folder_id.is_some() {
            self.loss(
                entity.to_owned(),
                format!(
                    "{LOSS_UNMAPPED_PREFIX} `folder_id`（仅界面折叠语义）没有 Logic 侧表示 ⇒ 不导出"
                ),
            );
        }
    }

    /// `MetaData.plist` 里的夜半扩展键这条（两条路线共用同一句）。
    fn loss_meta_data_key(&mut self, entity: &str) {
        self.loss(
            entity.to_owned(),
            format!(
                "{LOSS_NOT_EQUIVALENT_PREFIX} `{}` 是夜半写进 MetaData.plist 的**扩展键**，\
                 用于让整张损失表随文件走；Logic 忽略未知 plist 键这件事在本机未验证",
                META_DATA_LOSS_KEY
            ),
        );
    }

    /// 速度是 `u32` 的 `round(bpm × 10000)`，写不下的精度必须说出来。
    fn loss_tempo_quantisation(&mut self, entity: &str, bpm: f64) {
        let quantised = (bpm * 10_000.0).round() / 10_000.0;
        if (quantised - bpm).abs() > f64::EPSILON {
            self.loss(
                entity.to_owned(),
                format!(
                    "{LOSS_NOT_EQUIVALENT_PREFIX} 速度 {bpm} BPM 以 `round(bpm × 10000)` 整数写入，\
                     回读为 {quantised} BPM（差 {}）",
                    quantised - bpm
                ),
            );
        }
    }

    /// 调性三键刻意不写这条。
    fn loss_missing_key(&mut self, entity: &str, tracks: usize) {
        self.loss(
            entity.to_owned(),
            format!(
                "{LOSS_UNMAPPED_PREFIX} 调性（`SongKey`/`SongGenderKey`/`SignatureKey`）\
                 刻意不写 —— 夜半工程没有调性字段，写 \"C\"/\"major\" 等于替作者声明（工程 {tracks} 轨）"
            ),
        );
    }

    /// 供体路线的损失表：**逐族**说明哪些记录是供体的、哪些是我们的、哪些没写。
    ///
    /// 与自研路线共用 [`Self::loss_meta_data_key`] / [`Self::loss_tempo_quantisation`] /
    /// [`Self::loss_missing_key`] / [`Self::write_container_losses`]，因此两条路线的登记口径
    /// 不会各说各话。**替代**的是 `MISSING_CHUNK_FAMILIES` 那一族：供体路线那些家族**在产物里**，
    /// 只是属于供体。
    fn write_donor_losses(&mut self, project: &YebanProjectV1, splice: &DonorSplice) {
        let entity = project_entity(project);
        self.loss(entity.clone(), LOGIC_OPEN_SCOPE_CAVEAT.to_owned());
        self.loss(
            entity.clone(),
            format!(
                "{LOSS_NOT_EQUIVALENT_PREFIX} 本次产物是**供体克隆**：`ProjectData` 的 {} 条记录里只有 \
                 {} 条被改动（全局拍号 `qSvE`、全局速度 `qSvE`、被摆放 region 的 `qeSM` 名字、该 region \
                 的配对音符 `qSvE`），其余 {} 条**逐字节是供体的** —— 供体 = \
                 `jonkubis/logicproformatwriter` 的 Logic 夹具 `F0_baseline`（MIT，\
                 Copyright (c) 2026 Jon Kubis），sha256 `{}`，{} 字节，根头版本码 {:#06x}",
                LOGIC_DONOR_RECORD_COUNT,
                LOGIC_DONOR_PATCHED_RECORDS,
                LOGIC_DONOR_RECORD_COUNT - LOGIC_DONOR_PATCHED_RECORDS,
                LOGIC_DONOR_SHA256,
                LOGIC_DONOR_PROJECT_DATA_BYTES,
                LOGIC_DONOR_VERSION_CODE
            ),
        );
        // 逐族登记：这些家族**在产物里**，但描述的是供体的工程（这正是本路线的收益与代价）。
        for (name, count) in LOGIC_DONOR_CHUNK_FAMILIES {
            self.loss(
                entity.clone(),
                format!(
                    "{LOSS_NOT_EQUIVALENT_PREFIX} chunk 家族 `{name}`（落盘字节 `{}`）由供体**原样携带** \
                     {count} 条 —— 它描述的是供体的工程（1 条 `Inst 1` 轨 + 预分配混音槽 + 环境对象），\
                     不是我们的；本模块没有逐字段对账",
                    stored_chunk_tag(name)
                ),
            );
        }
        self.loss(
            entity.clone(),
            format!(
                "{LOSS_NOT_EQUIVALENT_PREFIX} `gnoS`（`Song`）载荷 {} 字节由供体原样携带（compact 形态）：\
                 它内嵌的初始速度三连（实测 1,200,000 = `round(120 × 10000)` 在载荷 {}）**没有**改成\
                 我们的 {} BPM —— 参考实现（§10.4）记 compact base 的速度/拍号写入器只动独立的 `qSvE`、\
                 从不动 `gnoS`；它为 settled 模板给的槽位 `+0x92`/`+0xEA`/`+0x3A6` 在供体里实测**全是 0**\
                 （compact 布局不同，因此没有可复用的规则）",
                LOGIC_DONOR_SONG_PAYLOAD_BYTES,
                LOGIC_DONOR_SONG_TEMPO_OFFSETS
                    .iter()
                    .map(|offset| format!("`+{offset:#04x}`"))
                    .collect::<Vec<_>>()
                    .join("/"),
                project.bpm
            ),
        );
        self.loss(
            entity.clone(),
            format!(
                "{LOSS_NOT_EQUIVALENT_PREFIX} 被摆放 region 的 `qeSM` 载荷除名字字段外的其余字节是供体的：\
                 载荷长 {} 保持不变，名字之后第一个非零字段在载荷 `{:#04x}`，名字字段的原地容量因此是 \
                 {} 字节；名字之前（`+0x00` 起 `70 03 01 00`）与之后（`+0x57`/`+0x58`/`+0x65`/`+0x99`/\
                 `+0xb7`/`+0xbc`/`+0xbd`/`+0xee`/`+0x120`/`+0x126`）的字节一个都没动",
                LOGIC_DONOR_REGION_PAYLOAD_BYTES,
                LOGIC_DONOR_REGION_FIRST_OTHER_FIELD,
                LOGIC_DONOR_REGION_NAME_CAPACITY
            ),
        );
        if splice.name_truncated {
            self.loss(
                entity.clone(),
                format!(
                    "{LOSS_NOT_EQUIVALENT_PREFIX} region 名超过供体名字字段的原地容量 {} 字节，\
                     已按 UTF-8 边界截断（移动名字之后的供体字节比截断更坏）",
                    LOGIC_DONOR_REGION_NAME_CAPACITY
                ),
            );
        }
        if splice.clamped {
            self.loss(
                entity.clone(),
                format!(
                    "{LOSS_NOT_EQUIVALENT_PREFIX} 有音符的 tick 超出 `u32` 能表示的范围\
                     （起始 = 摆放 + 音符 + {}），已饱和到 u32::MAX ⇒ 相对位置不再可信",
                    LOGIC_NOTE_ORIGIN_TICKS
                ),
            );
        }
        self.loss(
            entity.clone(),
            format!(
                "{LOSS_NOT_EQUIVALENT_PREFIX} region 的**摆放**与自身起点都是供体的：编排 `qSvE`\
                 （对象号 `0x00080000`）里那两条 `20 00 00 00` 事件（位置 `+0x04` = 34560）与 region \
                 `qeSM` 记录内 `+0x11c` = 0（第 1 小节）都没动；我们保留供体的第 1 小节摆放，把音符写成 \
                 `{} + 绝对 tick`（实测语义：音符位置是 region 相对的，region 的绝对位置只由摆放与 \
                 `qeSM +0x11c` 表达）。region 记录内 `+0x78` 的长度字段实测为 0，与 `quiet` 的两条含音符 \
                 region 相同，因此没有改；由此 region 的音符从第 1 小节起算，而不是从我们摆放的 {} tick \
                 起算（那一条没有可供体结构承载）",
                LOGIC_NOTE_ORIGIN_TICKS,
                splice.mapped_start_tick
            ),
        );
        self.loss_meta_data_key(&entity);
        self.loss_tempo_quantisation(&entity, project.bpm);
        self.loss_missing_key(&entity, project.tracks.len());
        self.loss(
            entity.clone(),
            format!(
                "{LOSS_UNMAPPED_PREFIX} 真实工程的另外三个 alternative 文件\
                 （`WindowImage.jpg`、`DisplayStateArchive`、`Undo Data.nosync`）不写 —— \
                 它们是不透明二进制/缩略图，本切片不伪造"
            ),
        );
        self.write_container_losses(&entity, project);
        self.write_donor_track_losses(project, splice);
    }

    /// 供体路线里**每一条夜半轨道**的登记：被映射的那条登记非 region 属性，其余整条登记为未映射。
    fn write_donor_track_losses(&mut self, project: &YebanProjectV1, splice: &DonorSplice) {
        for track in project.tracks.values() {
            let entity = format!("track:{}#{}", track.name, track.id.to_canonical_string());
            if Some(track.id) == splice.mapped_track_id {
                self.write_track_caveats(&entity, track);
                let midi_placements = track
                    .clips
                    .values()
                    .filter(|placement| {
                        project
                            .clip_pool
                            .get(&placement.clip_id)
                            .is_some_and(|entry| matches!(entry.content, ClipContent::Midi { .. }))
                    })
                    .count();
                if midi_placements > 1 {
                    self.loss(
                        entity.clone(),
                        format!(
                            "{LOSS_UNMAPPED_PREFIX} 供体只有 **1** 个可填的 MIDI region；本轨的 \
                             {midi_placements} 个 MIDI 摆放里只有第 1 个被映射 ⇒ 其余 {} 个不导出",
                            midi_placements - 1
                        ),
                    );
                }
                continue;
            }
            self.loss(
                entity,
                format!(
                    "{LOSS_UNMAPPED_PREFIX} 供体只携带 {} 条编排轨行（`Trak` 记录头 `+0x08 == \
                     0x00040000` 的非 0 载荷 2 条 − 1 条 master 行 = `MetaData.plist NumberOfTracks` \
                     {}）；我们的轨道（kind={:?}，摆放 {} 条）没有可供体插槽 ⇒ 整条不导出 —— \
                     参考实现（§10.6.1）的结论是新增通道会重排整个 `OCuA` 通道块，本切片不做插入",
                    LOGIC_DONOR_TRACK_COUNT,
                    LOGIC_DONOR_TRACK_COUNT,
                    track.kind,
                    track.clips.len()
                ),
            );
        }
    }

    fn write_track(&mut self, project: &YebanProjectV1, track: &TrackV3) {
        let entity = format!("track:{}#{}", track.name, track.id.to_canonical_string());
        match track.kind {
            TrackKind::Audio => {
                self.loss(
                    entity,
                    format!(
                        "{LOSS_UNMAPPED_PREFIX} 音频轨（TrackKind::Audio）没有 Logic 侧的 region \
                         表示 —— 本切片不写 `Media/`，也不写音频 region（摆放 {} 条）",
                        track.clips.len()
                    ),
                );
                return;
            }
            TrackKind::AuxReturn => {
                self.loss(
                    entity,
                    format!(
                        "{LOSS_UNMAPPED_PREFIX} 辅助返回轨（TrackKind::AuxReturn）没有 Logic 侧的 \
                         region 表示 ⇒ 不导出（摆放 {} 条）",
                        track.clips.len()
                    ),
                );
                return;
            }
            TrackKind::Master => {
                self.loss(
                    entity,
                    format!(
                        "{LOSS_UNMAPPED_PREFIX} 主总线轨（TrackKind::Master）不是 Logic 的 region，\
                         本切片不写混音/输出对象 ⇒ 不导出（工程 master_bus_track_id={}）",
                        project.master_bus_track_id.to_canonical_string()
                    ),
                );
                return;
            }
            TrackKind::Midi => {}
        }

        self.write_track_caveats(&entity, track);
        if track.clips.is_empty() {
            self.loss(
                entity,
                format!("{LOSS_UNMAPPED_PREFIX} MIDI 轨没有任何时间轴摆放 ⇒ 不产出 region"),
            );
            return;
        }
        for placement in track.clips.values() {
            self.write_placement(project, track, placement);
        }
    }

    fn write_placement(
        &mut self,
        project: &YebanProjectV1,
        track: &TrackV3,
        placement: &ClipPlacement,
    ) {
        let placement_entity = format!("clip-placement:{}", placement.id.to_canonical_string());
        let Some(entry) = project.clip_pool.get(&placement.clip_id) else {
            self.loss(
                placement_entity,
                format!(
                    "{LOSS_UNMAPPED_PREFIX} 摆放指向的片段池条目 {} 不存在（悬挂引用）⇒ 该摆放不导出",
                    placement.clip_id.to_canonical_string()
                ),
            );
            return;
        };
        let clip_entity = format!("clip:{}#{}", entry.name, entry.id.to_canonical_string());

        match &entry.content {
            ClipContent::Audio { asset, gain_db } => {
                self.loss(
                    clip_entity,
                    format!(
                        "{LOSS_UNMAPPED_PREFIX} 音频片段（资产 {}，增益 {gain_db} dB）没有 Logic 侧 \
                         region 表示，且本切片不写 `Media/` ⇒ 不导出",
                        asset.as_str()
                    ),
                );
            }
            ClipContent::Midi { notes } => {
                let cluster = self.next_cluster;
                self.next_cluster = self.next_cluster.wrapping_add(1);
                let name = if entry.name.is_empty() {
                    track.name.clone()
                } else {
                    entry.name.clone()
                };

                let mut lines: Vec<WrittenNote> = Vec::with_capacity(notes.len());
                let mut clamped = false;
                for note in notes.values() {
                    let absolute = placement
                        .start_tick
                        .checked_add(note.start_tick)
                        .and_then(|ticks| ticks.checked_add(LOGIC_NOTE_ORIGIN_TICKS));
                    let start = match absolute.and_then(|ticks| u32::try_from(ticks).ok()) {
                        Some(value) => value,
                        None => {
                            clamped = true;
                            u32::MAX
                        }
                    };
                    let duration = match u32::try_from(note.duration_ticks) {
                        Ok(value) => value,
                        Err(_) => {
                            clamped = true;
                            u32::MAX
                        }
                    };
                    lines.push(WrittenNote {
                        start,
                        duration,
                        pitch: note.pitch,
                        velocity: note.velocity,
                    });
                }
                if clamped {
                    self.loss(
                        clip_entity.clone(),
                        format!(
                            "{LOSS_NOT_EQUIVALENT_PREFIX} 有音符的 tick 超出 `u32` 能表示的范围\
                             （起始 = 摆放 + 音符 + 38400），已饱和到 u32::MAX ⇒ 相对位置不再可信"
                        ),
                    );
                }

                self.records.push(region_record(cluster, &name));
                // 音符 `qSvE` 的 subtype 在真实工程里与配对 `qeSM` 共享同一个序列号，
                // 本仓库没有反推出编号规则 ⇒ 与 `qeSM` 一样写 0（见 `region_record`）。
                self.records.push(record(
                    LOGIC_SEQUENCE_TAG,
                    LOGIC_SEQUENCE_KIND,
                    0,
                    cluster,
                    &note_lines(&lines),
                ));
                self.mapped_regions += 1;
                self.mapped_notes += lines.len();

                let attributes = unsupported_note_attributes(notes);
                if !attributes.is_empty() {
                    let affected = notes
                        .values()
                        .filter(|note| has_unsupported_attributes(note))
                        .count();
                    self.loss(
                        clip_entity,
                        format!(
                            "{LOSS_UNMAPPED_PREFIX} {affected}/{} 个音符携带本子集无法表达的表现属性\
                             （{}）—— 只导出起始/时值/音高/力度",
                            notes.len(),
                            attributes.join(", ")
                        ),
                    );
                }
            }
        }

        if placement.muted {
            self.loss(
                placement_entity.clone(),
                format!(
                    "{LOSS_UNMAPPED_PREFIX} 静音摆放标记 muted=true 没有 Logic 侧表示 ⇒ 不导出"
                ),
            );
        }
        let loop_config = placement.loop_config;
        let loop_covers =
            loop_config.start_tick == 0 && loop_config.end_tick == placement.duration_ticks;
        if loop_config.enabled && !loop_covers {
            self.loss(
                placement_entity,
                format!(
                    "{LOSS_UNMAPPED_PREFIX} 循环区间 {}..{} tick 与摆放跨度 0..{} tick 不同 ⇒ \
                     重复播放会丢",
                    loop_config.start_tick, loop_config.end_tick, placement.duration_ticks
                ),
            );
        }
    }
}

/// 音符表现属性里本子集无法表达的那些（按字母序、去重）。
fn unsupported_note_attributes(notes: &BTreeMap<EntityId, MidiNote>) -> Vec<&'static str> {
    let mut found = BTreeSet::new();
    for note in notes.values() {
        if note.probability.is_some() {
            found.insert("probability");
        }
        if note.ratchet.is_some() {
            found.insert("ratchet");
        }
        if note.micro_timing_ticks.is_some() {
            found.insert("micro_timing_ticks");
        }
        if note.slide.is_some() {
            found.insert("slide");
        }
        if !note.pitch_bend_curve.is_empty() {
            found.insert("pitch_bend_curve");
        }
        if note.syllable.is_some() {
            found.insert("syllable");
        }
        if !note.phonemes.is_empty() {
            found.insert("phonemes");
        }
    }
    found.into_iter().collect()
}

/// 一个音符是否携带任何本子集无法表达的属性。
fn has_unsupported_attributes(note: &MidiNote) -> bool {
    note.probability.is_some()
        || note.ratchet.is_some()
        || note.micro_timing_ticks.is_some()
        || note.slide.is_some()
        || !note.pitch_bend_curve.is_empty()
        || note.syllable.is_some()
        || !note.phonemes.is_empty()
}

/// 工程级实体的稳定寻址。
fn project_entity(project: &YebanProjectV1) -> String {
    format!("project:{}", project.id.to_canonical_string())
}

/// `gnoS`：速度写在两个固定槽里（`round(bpm × 10000)`，`u32` 小端）。
///
/// 载荷 `+0x00` 是实测的嵌套 `#G` 子帧前缀（[`LOGIC_ROOT_MAGIC`] + 版本码 +
/// [`LOGIC_SONG_SUBFRAME_SUFFIX`]）；子帧其余内容不重建，见 [`CONTAINER_HEADER_CAVEAT`]。
/// 记录头 `+0x08..+0x16` 是实测的 14 字节 `0xFF` 填充（不是 cluster 值）。
fn song_record(bpm: f64) -> Vec<u8> {
    let ticks = tempo_ticks(bpm);
    let mut body = vec![0u8; LOGIC_SONG_TEMPO_SLOT_AUTHORITATIVE - LOGIC_RECORD_HEADER + 4];
    body[..4].copy_from_slice(&LOGIC_ROOT_MAGIC);
    put_u16_le(&mut body, 4, LOGIC_FORMAT_VERSION_CODE);
    body[6..10].copy_from_slice(&LOGIC_SONG_SUBFRAME_SUFFIX);
    put_u32_le(
        &mut body,
        LOGIC_SONG_TEMPO_SLOT_AUTHORITATIVE - LOGIC_RECORD_HEADER,
        ticks,
    );
    put_u32_le(
        &mut body,
        LOGIC_SONG_TEMPO_SLOT_FALLBACK - LOGIC_RECORD_HEADER,
        ticks,
    );
    let mut out = record(
        LOGIC_SONG_TAG,
        LOGIC_SONG_KIND,
        LOGIC_SONG_SUBTYPE,
        0,
        &body,
    );
    for byte in &mut out[LOGIC_RECORD_CLUSTER_OFFSET..0x16] {
        *byte = 0xFF;
    }
    out
}

/// 拍号 `qSvE` 载荷**没有**拍号变化时的 80 字节头（**实测字面量**，`0x0b`/`0x0c` 由调用方写入）。
///
/// 字节来自本机实测且与 MIT 参考实现一致的文件：`quiet`
/// （`…/quiet.logicx/Alternatives/000/ProjectData`，版本码 0x09D0）与工厂模板 `01 Hip Hop`
/// 的 4/4 载荷在这 80 个字节上**逐字节相同**；`F0` 夹具（Logic 12.0.1，版本码 0x09CF）
/// 只差 `+0x0f` 一个字节（它写 `0x80`，本常量也写 `0x80`，见 [`LOGIC_METER_NO_CHANGE_FLAG`]）。
/// 这 80 个字节之后接 [`LOGIC_EVENT_SEQUENCE_TAIL`]。
const LOGIC_METER_HEADER_NO_CHANGE: [u8; LOGIC_METER_BODY_LEN] = [
    // +0x00
    0x30, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x04, 0x00, 0x00, 0x80,
    // +0x10
    0x30, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x88, 0xF6, 0xFF, 0x00, 0x00, 0x00, 0x96, 0x00, 0x00,
    // +0x20
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x88, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // +0x30
    0x32, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00,
    // +0x40
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x88, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// 拍号记录：**实测/参考实现**的 96 字节载荷 = 80 字节头 + 16 字节尾。
///
/// 头里 `+0x0b` 是分母的以 2 为底指数（`den = 2^exp`），`+0x0c` 是分子，
/// `+0x0f` 是 [`LOGIC_METER_NO_CHANGE_FLAG`]（本切片写不出拍号变化，因此恒为"无变化"形态）。
///
/// 参考实现（MIT，§5）把这个形状记为 `payload_size = 80 + 48*changes + 16`，其
/// `set_meter_map` 产出的工程被 Logic 打开过；实测五份文件里无变化的拍号载荷都是 96 字节。
fn meter_record(numerator: u8, denominator: u8) -> Vec<u8> {
    let mut body = Vec::with_capacity(LOGIC_METER_PAYLOAD_LEN);
    body.extend_from_slice(&LOGIC_METER_HEADER_NO_CHANGE);
    body[LOGIC_METER_DENOMINATOR_EXPONENT_OFFSET] = denominator.trailing_zeros() as u8;
    body[LOGIC_METER_NUMERATOR_OFFSET] = numerator;
    body.extend_from_slice(&LOGIC_EVENT_SEQUENCE_TAIL);
    record(
        LOGIC_SEQUENCE_TAG,
        LOGIC_SEQUENCE_KIND,
        LOGIC_METER_SUBTYPE,
        0,
        &body,
    )
}

/// 速度事件序列：**实测/参考实现**的 48 字节载荷 = 一条 32 字节事件 + 16 字节尾。
///
/// 事件布局（`F0` 夹具、`quiet`、`01 Hip Hop` 的 120 BPM 记录在这 32 个字节上逐字节相同；
/// `Swing!` 115 BPM、`ocean eyes` 145 BPM、`01 Hip Hop` 70 BPM 只差 `+0x10` 的速度字）：
///
/// | 偏移 | 内容 |
/// | :--- | :--- |
/// | +0x00 | `60 00 00 00`（[`LOGIC_TEMPO_MARKER`]） |
/// | +0x04 | `u64` 小端位置 = [`LOGIC_TEMPO_POSITION_TICKS`]（38400） |
/// | +0x0c | `7F 00 00` + [`LOGIC_TEMPO_FLAG_FIRST`] |
/// | +0x10 | `u32` 小端速度 = `round(bpm × 10000)` |
/// | +0x14 | [`LOGIC_TEMPO_EVENT_CONST_14`] |
/// | +0x18 | `u32` 小端 [`LOGIC_TEMPO_ALT_POSITION_BASE`] |
/// | +0x1c | 0 |
///
/// ⚠ 在本次改正之前，本函数写的是 **16 字节**载荷：只有第一个字 `0x60`、其余全 0 ——
/// 也就是说**位置 0、速度 0**，而且没有 16 字节尾。那一处**没有**被登记成偏差，
/// 是一个此前未被发现的缺陷；现在它按实测与 MIT 参考实现的形状写入。
fn tempo_record(bpm: f64) -> Vec<u8> {
    let ticks = tempo_ticks(bpm);
    let mut body = Vec::with_capacity(LOGIC_TEMPO_PAYLOAD_LEN);
    body.resize(LOGIC_TEMPO_EVENT_LEN, 0);
    put_u32_le(&mut body, 0, LOGIC_TEMPO_MARKER);
    body[0x04..0x0c].copy_from_slice(&LOGIC_TEMPO_POSITION_TICKS.to_le_bytes());
    body[0x0c] = 0x7F;
    body[0x0f] = LOGIC_TEMPO_FLAG_FIRST;
    put_u32_le(&mut body, LOGIC_TEMPO_VALUE_OFFSET, ticks);
    body[0x14..0x18].copy_from_slice(&LOGIC_TEMPO_EVENT_CONST_14);
    put_u32_le(&mut body, 0x18, LOGIC_TEMPO_ALT_POSITION_BASE);
    body.extend_from_slice(&LOGIC_EVENT_SEQUENCE_TAIL);
    record(
        LOGIC_SEQUENCE_TAG,
        LOGIC_SEQUENCE_KIND,
        LOGIC_TEMPO_SUBTYPE,
        0,
        &body,
    )
}

/// 速度的落盘整数形态：`u32` 小端 `round(bpm × 10000)`；写不下（非有限 / 越界）时写 0。
///
/// `gnoS` 的速度槽与速度事件序列共用这一个换算（此前是两段重复代码）。
fn tempo_ticks(bpm: f64) -> u32 {
    let raw = (bpm * 10_000.0).round();
    if raw.is_finite() && (0.0..=f64::from(u32::MAX)).contains(&raw) {
        raw as u32
    } else {
        0
    }
}

/// region 记录：载荷 `+0x10` = `u16` 小端**字节数**，`+0x12` 起 UTF-8 名字，名字后 4 字节 0
/// （见 [`LOGIC_REGION_NAME_PAYLOAD_OFFSET`] 的实测表；记录内偏移 = 0x24 + 0x10 = 0x34）。
///
/// ⚠ subtype（+0x06）在真实工程里是与配对 `qSvE` **共享的序列号**（实测同一 region 的
/// `qeSM` 与 `qSvE` 取同一个值，本机 12.0.1 工程里是 1、3、5、14、17、22、23、25 这些值），
/// 本仓库没有反推出编号规则，因此**不猜**：写 0，并把这一处登记在
/// [`CONTAINER_HEADER_CAVEAT`] 里（而不是假装它与实测一致）。
///
/// ⚠ 载荷**其余部分**（`+0x00..+0x0f`、`+0x1a` 之后）与真实记录不同 —— 真实载荷长 296..324
/// 字节且开头非零，本写入器只写名字字段。这一处登记在 [`REGION_PAYLOAD_CAVEAT`] 里。
fn region_record(cluster: u32, name: &str) -> Vec<u8> {
    let name_in_body = LOGIC_REGION_NAME_PAYLOAD_OFFSET;
    let name_bytes = utf8_prefix(name, usize::from(u16::MAX)).as_bytes();
    let mut body = vec![0u8; name_in_body + 2 + name_bytes.len() + 4];
    body[name_in_body..name_in_body + 2].copy_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    body[name_in_body + 2..name_in_body + 2 + name_bytes.len()].copy_from_slice(name_bytes);
    put_u32_le(&mut body, name_in_body + 2 + name_bytes.len(), 0);
    record(LOGIC_REGION_TAG, LOGIC_REGION_KIND, 0, cluster, &body)
}

/// 一个待写入的音符（tick 已换算成绝对位置 + 原点偏移）。
struct WrittenNote {
    start: u32,
    duration: u32,
    pitch: u8,
    velocity: u8,
}

/// 音符事件里**首个** 16 字节行的 `+0x0f` 标志：`0x01`（实测，参考实现 §8.5）。
pub const LOGIC_NOTE_FLAG: u8 = 0x01;

/// 音符事件里**最后一条**音符的 `+0x0f` 标志：`0x01 | 0x80`（实测，参考实现 §8.5）。
pub const LOGIC_NOTE_FLAG_LAST: u8 = 0x81;

/// 音符事件 `+0x10` 的恒定字节（实测 `0x40`，参考实现 §8.5 记 "const (=64)"）。
pub const LOGIC_NOTE_CONST_10: u8 = 0x40;

/// 音符事件第二条 16 字节行的第 7 字节（事件内 `+0x17`）：实测 `0x89`。
///
/// 最高位为 1 正是 groove 读取器判"续行"的依据（[`LOGIC_CONTINUATION_FLAG`]），因此
/// 本写入器与读取器、与参考实现三边一致；此前本写入器写 `0x80`（少了低 4 位）。
pub const LOGIC_NOTE_CONST_17: u8 = 0x89;

/// 音符序列：每个音符一行头（16 字节）+ **恰好一行**续行（16 字节）。
///
/// ⚠ 这是刻意的简化并写进模块头：真实工程里每个音符的续行数在 16…96 字节之间变化，
/// 而读取器只从**第一条**续行取时值，因此一条续行不丢信息 —— 但与 Logic 自己的形状
/// 不同，正是"读取器能容忍、Logic 未必"的那类差异。
///
/// 载荷**以 [`LOGIC_EVENT_SEQUENCE_TAIL`] 结束**：参考实现（MIT，§8.5）记
/// "Empty region qSvE payload = just the 16B `f1…3f` tail; each note adds a **32-byte event**
/// before the tail (payload = 32·N + 16)"。本次改正之前本函数**没有**写这个尾。
///
/// 事件里三个常量字节（`+0x0f` 标志、`+0x10` = 0x40、`+0x17` = 0x89）按参考实现 §8.5 的
/// **实测**字面量写入（`+0x0f` 在最后一条音符上带 `0x80`）；`+0x0a` 的"细力度"实测为 0，
/// 参考实现的 `_enc_note_event(..., fine=0)` 也写 0，因此这里保持 0。
fn note_lines(notes: &[WrittenNote]) -> Vec<u8> {
    let mut body = vec![0u8; notes.len() * LOGIC_EVENT_LINE_SIZE * 2];
    let last = notes.len().saturating_sub(1);
    for (index, note) in notes.iter().enumerate() {
        let head = index * LOGIC_EVENT_LINE_SIZE * 2;
        let continuation = head + LOGIC_EVENT_LINE_SIZE;
        body[head] = LOGIC_NOTE_STATUS;
        put_u32_le(&mut body, head + 4, note.start);
        body[head + 0x0b] = note.velocity.clamp(1, 127);
        body[head + 0x0c] = note.pitch;
        body[head + 0x0f] = if index == last {
            LOGIC_NOTE_FLAG_LAST
        } else {
            LOGIC_NOTE_FLAG
        };
        body[head + 0x10] = LOGIC_NOTE_CONST_10;
        body[continuation + 7] = LOGIC_NOTE_CONST_17;
        put_u32_le(&mut body, continuation + 0x0c, note.duration.max(1));
    }
    body.extend_from_slice(&LOGIC_EVENT_SEQUENCE_TAIL);
    body
}

/// 36 字节记录头 + 载荷：名字在 +0，kind 在 +4，subtype 在 +6，cluster 在 +8，
/// +0x16/+0x18/+0x1a 是实测的恒定字，载荷长度在 +0x1c。
fn record(tag: [u8; 4], kind: u16, subtype: u16, cluster: u32, body: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; LOGIC_RECORD_HEADER + body.len()];
    out[..4].copy_from_slice(&tag);
    put_u16_le(&mut out, LOGIC_RECORD_KIND_OFFSET, kind);
    put_u16_le(&mut out, LOGIC_RECORD_SUBTYPE_OFFSET, subtype);
    put_u32_le(&mut out, LOGIC_RECORD_CLUSTER_OFFSET, cluster);
    put_u16_le(&mut out, 0x16, LOGIC_RECORD_FIELD_16);
    put_u16_le(&mut out, 0x18, LOGIC_RECORD_FIELD_18);
    put_u16_le(&mut out, 0x1a, LOGIC_RECORD_FIELD_1A);
    put_u32_le(&mut out, LOGIC_RECORD_SIZE_OFFSET, body.len() as u32);
    out[LOGIC_RECORD_HEADER..].copy_from_slice(body);
    out
}

/// 把记录串成完整的 `ProjectData`：根头 0x18 字节 + 记录流。
///
/// 根头写**实测**的版本码（[`LOGIC_FORMAT_VERSION_CODE`]，+0x04）与十个恒定字节
/// （[`LOGIC_ROOT_STABLE_FIELDS`]，+0x06..0x10），声明长度写在 0x10。
fn root_document(records: &[Vec<u8>]) -> Vec<u8> {
    let payload: usize = records.iter().map(Vec::len).sum();
    let mut out = vec![0u8; LOGIC_ROOT_HEADER + payload];
    out[..4].copy_from_slice(&LOGIC_ROOT_MAGIC);
    put_u16_le(
        &mut out,
        LOGIC_FORMAT_VERSION_OFFSET,
        LOGIC_FORMAT_VERSION_CODE,
    );
    out[LOGIC_ROOT_STABLE_OFFSET..LOGIC_ROOT_STABLE_OFFSET + LOGIC_ROOT_STABLE_FIELDS.len()]
        .copy_from_slice(&LOGIC_ROOT_STABLE_FIELDS);
    put_u32_le(&mut out, LOGIC_DECLARED_LENGTH_OFFSET, payload as u32);
    let mut at = LOGIC_ROOT_HEADER;
    for entry in records {
        out[at..at + entry.len()].copy_from_slice(entry);
        at += entry.len();
    }
    out
}

/// `u16` 小端写到 `out[at..at+2]`。
fn put_u16_le(out: &mut [u8], at: usize, value: u16) {
    out[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

/// `u32` 小端写到 `out[at..at+4]`。
fn put_u32_le(out: &mut [u8], at: usize, value: u32) {
    out[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

/// 最长不超过 `max` 字节的 UTF-8 前缀（不切进一个字符的中间）。
fn utf8_prefix(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// alternative 编号：`0..=999` 的十进制整数。
fn alternative_index(alternative: &str) -> Option<u32> {
    let trimmed = alternative.trim();
    if trimmed.is_empty() || !trimmed.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    match trimmed.parse::<u32>() {
        Ok(value) if value <= 999 => Some(value),
        _ => None,
    }
}

/* ------------------------------------------------------------------ *
 * 最小二进制 plist (bplist00) 编码器
 * ------------------------------------------------------------------ */

/// plist 的取值子集（本模块用到的全部形态）。
#[derive(Clone, Debug, PartialEq)]
enum PlistValue {
    Bool(bool),
    Integer(i64),
    Real(f64),
    Text(String),
    Array(Vec<PlistValue>),
    Dict(Vec<(String, PlistValue)>),
}

/// 一个已分配、但还没编码的 plist 对象。
enum PlistNode {
    Bool(bool),
    Integer(i64),
    Real(f64),
    Text(String),
    Array(Vec<u32>),
    Dict(Vec<(u32, u32)>),
}

/// 两趟式编码：第一趟分配对象并收集引用，第二趟在知道 `objectRefSize` 之后编码。
#[derive(Default)]
struct PlistWriter {
    nodes: Vec<PlistNode>,
}

impl PlistWriter {
    fn alloc(&mut self, value: &PlistValue) -> u32 {
        let index = self.nodes.len() as u32;
        self.nodes.push(PlistNode::Bool(false));
        let node = match value {
            PlistValue::Bool(flag) => PlistNode::Bool(*flag),
            PlistValue::Integer(number) => PlistNode::Integer(*number),
            PlistValue::Real(number) => PlistNode::Real(*number),
            PlistValue::Text(text) => PlistNode::Text(text.clone()),
            PlistValue::Array(items) => {
                let mut refs = Vec::with_capacity(items.len());
                for item in items {
                    refs.push(self.alloc(item));
                }
                PlistNode::Array(refs)
            }
            PlistValue::Dict(entries) => {
                let mut sorted: Vec<&(String, PlistValue)> = entries.iter().collect();
                sorted.sort_by(|left, right| left.0.cmp(&right.0));
                let mut pairs = Vec::with_capacity(sorted.len());
                for (key, item) in sorted {
                    let key_ref = self.alloc(&PlistValue::Text(key.clone()));
                    let value_ref = self.alloc(item);
                    pairs.push((key_ref, value_ref));
                }
                PlistNode::Dict(pairs)
            }
        };
        self.nodes[index as usize] = node;
        index
    }
}

/// 把 plist 值编码成标准 `bplist00` 字节（确定性：对象顺序 = 分配顺序，无时间戳）。
fn encode_binary_plist(value: &PlistValue) -> Vec<u8> {
    let mut writer = PlistWriter::default();
    let top = writer.alloc(value);
    debug_assert_eq!(top, 0, "根对象必须是 0 号对象");
    let object_count = writer.nodes.len();
    let object_ref_size = width(object_count as u64);

    let mut body: Vec<u8> = Vec::new();
    let mut offsets: Vec<u64> = Vec::with_capacity(object_count);
    for node in &writer.nodes {
        offsets.push(8 + body.len() as u64);
        encode_node(node, object_ref_size, &mut body);
    }

    let offset_table_offset = 8 + body.len() as u64;
    let offset_int_size = width(offset_table_offset.max(offsets.last().copied().unwrap_or(0) + 1));

    let mut out =
        Vec::with_capacity(offset_table_offset as usize + object_count * offset_int_size + 32);
    out.extend_from_slice(b"bplist00");
    out.extend_from_slice(&body);
    for offset in &offsets {
        out.extend_from_slice(&be_bytes(*offset, offset_int_size));
    }
    let mut trailer = [0u8; 32];
    trailer[6] = offset_int_size as u8;
    trailer[7] = object_ref_size as u8;
    trailer[8..16].copy_from_slice(&(object_count as u64).to_be_bytes());
    trailer[16..24].copy_from_slice(&0u64.to_be_bytes());
    trailer[24..32].copy_from_slice(&offset_table_offset.to_be_bytes());
    out.extend_from_slice(&trailer);
    out
}

/// 编码一个对象（引用宽度已定）。
fn encode_node(node: &PlistNode, ref_size: usize, out: &mut Vec<u8>) {
    match node {
        PlistNode::Bool(true) => out.push(0x09),
        PlistNode::Bool(false) => out.push(0x08),
        PlistNode::Integer(value) => push_int(out, *value),
        PlistNode::Real(value) => {
            out.push(0x23);
            out.extend_from_slice(&value.to_bits().to_be_bytes());
        }
        PlistNode::Text(text) => push_string(out, text),
        PlistNode::Array(refs) => {
            push_length(out, 0xA0, refs.len());
            for reference in refs {
                out.extend_from_slice(&be_bytes(u64::from(*reference), ref_size));
            }
        }
        PlistNode::Dict(pairs) => {
            push_length(out, 0xD0, pairs.len());
            for (key, _) in pairs {
                out.extend_from_slice(&be_bytes(u64::from(*key), ref_size));
            }
            for (_, value) in pairs {
                out.extend_from_slice(&be_bytes(u64::from(*value), ref_size));
            }
        }
    }
}

/// 容器/字符串的长度标记；≥ 15 时按规范接一个整型对象。
fn push_length(out: &mut Vec<u8>, base: u8, count: usize) {
    if count < 15 {
        out.push(base | (count as u8));
    } else {
        out.push(base | 0x0f);
        push_int(out, count as i64);
    }
}

/// 内联整型对象（1/2/4/8 字节，大端）。
fn push_int(out: &mut Vec<u8>, value: i64) {
    if value >= 0 {
        let raw = value as u64;
        let (marker, size) = if raw <= u64::from(u8::MAX) {
            (0x10u8, 1usize)
        } else if raw <= u64::from(u16::MAX) {
            (0x11, 2)
        } else if raw <= u64::from(u32::MAX) {
            (0x12, 4)
        } else {
            (0x13, 8)
        };
        out.push(marker);
        out.extend_from_slice(&be_bytes(raw, size));
    } else {
        out.push(0x13);
        out.extend_from_slice(&(value as u64).to_be_bytes());
    }
}

/// ASCII → `0x5x`，其余 → UTF-16BE 的 `0x6x`（plist 的两种字符串形态）。
fn push_string(out: &mut Vec<u8>, text: &str) {
    if text.is_ascii() {
        push_length(out, 0x50, text.len());
        out.extend_from_slice(text.as_bytes());
    } else {
        let units: Vec<u16> = text.encode_utf16().collect();
        push_length(out, 0x60, units.len());
        for unit in units {
            out.extend_from_slice(&unit.to_be_bytes());
        }
    }
}

/// 大端定宽编码（`value` 的低 `size` 字节）。
fn be_bytes(value: u64, size: usize) -> Vec<u8> {
    value.to_be_bytes()[8 - size..].to_vec()
}

/// 表示 `value` 需要几个字节。
fn width(value: u64) -> usize {
    if value <= u64::from(u8::MAX) {
        1
    } else if value <= u64::from(u16::MAX) {
        2
    } else if value <= u64::from(u32::MAX) {
        4
    } else {
        8
    }
}

/* ------------------------------------------------------------------ *
 * 判据
 * ------------------------------------------------------------------ */

#[cfg(all(test, feature = "experimental-logic-export"))]
mod tests {
    use super::*;
    use std::str::FromStr as _;

    /// 稳定的夹具身份（与 `yeban_model::samples` 的 fixture 同款 ULID 前缀）。
    fn test_id(index: u128) -> EntityId {
        EntityId::from_str(&format!("01J8ZQ{index:020}")).expect("canonical fixture ulid")
    }

    /// 一个音符一个音高的小夹具：两条 `Midi` 轨（第二轨有一个非零摆放起点）。
    fn fixture_project() -> YebanProjectV1 {
        let mut project = YebanProjectV1 {
            title: "Logic Fixture".to_owned(),
            bpm: 128.0,
            time_signature: yeban_model::TimeSignature {
                numerator: 3,
                denominator: 4,
            },
            ..YebanProjectV1::default()
        };

        let first_clip = test_id(10);
        let mut clip = yeban_model::ClipPoolEntry {
            id: first_clip,
            name: "Piano".to_owned(),
            content: ClipContent::default(),
        };
        if let Some(notes) = clip.content.notes_mut() {
            for (index, (start, pitch, velocity)) in
                [(0_u64, 60_u8, 100_u8), (960, 64, 90), (1920, 67, 80)]
                    .into_iter()
                    .enumerate()
            {
                let note_id = test_id(100 + index as u128);
                let mut note = MidiNote::new(note_id, start, pitch, 480);
                note.velocity = velocity;
                notes.insert(note_id, note);
            }
        }
        project.clip_pool.insert(first_clip, clip);

        let placement = test_id(50);
        let mut lead = TrackV3 {
            id: test_id(1),
            name: "Lead".to_owned(),
            ..TrackV3::default()
        };
        lead.clips.insert(
            placement,
            ClipPlacement {
                id: placement,
                clip_id: first_clip,
                start_tick: 3_840,
                duration_ticks: 3_840,
                loop_config: yeban_model::LoopConfig::default(),
                muted: false,
            },
        );
        project.tracks.insert(lead.id, lead);
        project
    }

    /// 一个含**不可映射构造**的工程：音频轨、辅助返回轨、主总线、设备链、自动化、音频片段。
    fn unmappable_project() -> YebanProjectV1 {
        let mut project = fixture_project();

        let audio_clip = test_id(11);
        project.clip_pool.insert(
            audio_clip,
            yeban_model::ClipPoolEntry {
                id: audio_clip,
                name: "Kick".to_owned(),
                content: ClipContent::Audio {
                    asset: yeban_model::AssetHash::of_bytes(b"kick"),
                    gain_db: -1.5,
                },
            },
        );
        let audio_placement = test_id(51);
        let mut audio = TrackV3 {
            id: test_id(2),
            name: "Bass".to_owned(),
            kind: TrackKind::Audio,
            ..TrackV3::default()
        };
        audio.clips.insert(
            audio_placement,
            ClipPlacement {
                id: audio_placement,
                clip_id: audio_clip,
                start_tick: 0,
                duration_ticks: 960,
                loop_config: yeban_model::LoopConfig::default(),
                muted: true,
            },
        );
        project.tracks.insert(audio.id, audio);

        let aux = TrackV3 {
            id: test_id(3),
            name: "Aux".to_owned(),
            kind: TrackKind::AuxReturn,
            ..TrackV3::default()
        };
        project.tracks.insert(aux.id, aux);

        // 给 Lead 加一条自动化泳道与一个设备，证明"有表示但不完整"的那类也会登记。
        if let Some(lead) = project.tracks.get_mut(&test_id(1)) {
            lead.automation_lanes.insert(
                yeban_model::AutomationTarget::TrackVolume { track_id: lead.id },
                yeban_model::AutomationLane::implicit(yeban_model::AutomationTarget::TrackVolume {
                    track_id: lead.id,
                }),
            );
        }
        project
    }

    // ---- 测试用的最小读取器（照 groove 的 `logicToArrangement.ts` 翻译） ----

    struct Record {
        tag: [u8; 4],
        cluster: u32,
        body: Vec<u8>,
    }

    fn u32_le(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    }

    fn u16_le(bytes: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([bytes[at], bytes[at + 1]])
    }

    /// 所有记录的 `(偏移, tag)`，**只由载荷长度驱动**走完整个 chunk 流。
    fn record_offsets(bytes: &[u8]) -> Vec<(usize, [u8; 4])> {
        let mut out = Vec::new();
        let mut at = LOGIC_ROOT_HEADER;
        while at + LOGIC_RECORD_HEADER <= bytes.len() {
            let tag = [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]];
            let size = u32_le(bytes, at + LOGIC_RECORD_SIZE_OFFSET) as usize;
            out.push((at, tag));
            at += LOGIC_RECORD_HEADER + size;
        }
        out
    }

    fn read_records(bytes: &[u8]) -> Vec<Record> {
        assert_eq!(bytes[..4], LOGIC_ROOT_MAGIC, "根魔数");
        assert_eq!(
            u32_le(bytes, LOGIC_DECLARED_LENGTH_OFFSET) as usize,
            bytes.len() - LOGIC_ROOT_HEADER,
            "声明载荷长度必须等于 文件长度 − 0x18"
        );
        let mut records = Vec::new();
        let mut at = LOGIC_ROOT_HEADER;
        while at + LOGIC_RECORD_HEADER <= bytes.len() {
            let tag = [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]];
            let size = u32_le(bytes, at + LOGIC_RECORD_SIZE_OFFSET) as usize;
            let cluster = u32_le(bytes, at + LOGIC_RECORD_CLUSTER_OFFSET);
            let body = bytes[at + LOGIC_RECORD_HEADER..at + LOGIC_RECORD_HEADER + size].to_vec();
            records.push(Record { tag, cluster, body });
            at += LOGIC_RECORD_HEADER + size;
        }
        assert_eq!(at, bytes.len(), "记录流必须恰好铺满声明载荷");
        records
    }

    fn read_song_tempo(records: &[Record]) -> Option<f64> {
        let song = records.iter().find(|record| record.tag == LOGIC_SONG_TAG)?;
        for slot in [
            LOGIC_SONG_TEMPO_SLOT_AUTHORITATIVE,
            LOGIC_SONG_TEMPO_SLOT_FALLBACK,
        ] {
            if slot + 4 <= song.body.len() + LOGIC_RECORD_HEADER {
                // 记录体相对槽 = 载荷槽 − 0x24（groove 的 `readTempo` 读载荷，本测试读体）。
                let at = slot - LOGIC_RECORD_HEADER;
                if at + 4 <= song.body.len() {
                    let raw = u32_le(&song.body, at);
                    if raw > 0 && raw < 100_000_000 {
                        return Some(f64::from(raw) / 10_000.0);
                    }
                }
            }
        }
        None
    }

    fn read_meter(records: &[Record]) -> Option<(u8, u8)> {
        let meter = records.iter().find(|record| {
            record.tag == LOGIC_SEQUENCE_TAG
                && record.body.len() >= LOGIC_METER_BODY_LEN
                && u32_le(&record.body, 0) == LOGIC_METER_MARKER
        })?;
        let exponent = meter.body[0x0b];
        let numerator = meter.body[0x0c];
        Some((numerator, 1u8 << exponent))
    }

    fn read_region_name(record: &Record) -> String {
        let at = LOGIC_REGION_NAME_PAYLOAD_OFFSET;
        let length = usize::from(u16::from_le_bytes([record.body[at], record.body[at + 1]]));
        String::from_utf8(record.body[at + 2..at + 2 + length].to_vec()).expect("UTF-8")
    }

    // ---- 最小 bplist00 读取器（只覆盖本模块写出的形态） ----

    #[derive(Debug, PartialEq)]
    enum TestPlist {
        Bool(bool),
        Integer(i64),
        Real(f64),
        Text(String),
        Array(Vec<TestPlist>),
        Dict(BTreeMap<String, TestPlist>),
    }

    fn read_bplist(bytes: &[u8]) -> TestPlist {
        assert_eq!(&bytes[..8], b"bplist00", "二进制 plist 魔数");
        let trailer = &bytes[bytes.len() - 32..];
        let offset_int_size = usize::from(trailer[6]);
        let object_ref_size = usize::from(trailer[7]);
        let object_count = be_uint(&trailer[8..16], 8) as usize;
        let top = be_uint(&trailer[16..24], 8) as usize;
        let table = be_uint(&trailer[24..32], 8) as usize;
        let offsets: Vec<usize> = (0..object_count)
            .map(|index| {
                be_uint(&bytes[table + index * offset_int_size..], offset_int_size) as usize
            })
            .collect();
        read_object(bytes, &offsets, object_ref_size, top)
    }

    fn be_uint(bytes: &[u8], size: usize) -> u64 {
        let mut value = 0u64;
        for byte in bytes.iter().take(size) {
            value = (value << 8) | u64::from(*byte);
        }
        value
    }

    fn plist_length(bytes: &[u8], marker: u8, at: usize) -> (usize, usize) {
        let info = usize::from(marker & 0x0f);
        if info != 0x0f {
            return (info, at);
        }
        let int_marker = bytes[at];
        assert_eq!(int_marker >> 4, 1, "长度前缀必须是整型对象");
        let size = 1usize << (int_marker & 0x0f);
        (be_uint(&bytes[at + 1..], size) as usize, at + 1 + size)
    }

    fn read_object(bytes: &[u8], offsets: &[usize], ref_size: usize, index: usize) -> TestPlist {
        let mut at = offsets[index];
        let marker = bytes[at];
        at += 1;
        match marker >> 4 {
            0x0 => match marker {
                0x08 => TestPlist::Bool(false),
                0x09 => TestPlist::Bool(true),
                other => panic!("未知的简单标记 {other:#x}"),
            },
            0x1 => {
                let size = 1usize << (marker & 0x0f);
                TestPlist::Integer(be_uint(&bytes[at..], size) as i64)
            }
            0x2 => TestPlist::Real(f64::from_bits(be_uint(&bytes[at..], 8))),
            0x5 => {
                let (count, at) = plist_length(bytes, marker, at);
                TestPlist::Text(String::from_utf8(bytes[at..at + count].to_vec()).expect("ASCII"))
            }
            0x6 => {
                let (count, mut at) = plist_length(bytes, marker, at);
                let mut units = Vec::with_capacity(count);
                for _ in 0..count {
                    units.push(u16::from_be_bytes([bytes[at], bytes[at + 1]]));
                    at += 2;
                }
                TestPlist::Text(String::from_utf16(&units).expect("UTF-16"))
            }
            0xA => {
                let (count, mut at) = plist_length(bytes, marker, at);
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(read_object(
                        bytes,
                        offsets,
                        ref_size,
                        be_uint(&bytes[at..], ref_size) as usize,
                    ));
                    at += ref_size;
                }
                TestPlist::Array(items)
            }
            0xD => {
                let (count, mut at) = plist_length(bytes, marker, at);
                let mut keys = Vec::with_capacity(count);
                for _ in 0..count {
                    keys.push(be_uint(&bytes[at..], ref_size) as usize);
                    at += ref_size;
                }
                let mut dict = BTreeMap::new();
                for key in keys {
                    let value = read_object(
                        bytes,
                        offsets,
                        ref_size,
                        be_uint(&bytes[at..], ref_size) as usize,
                    );
                    at += ref_size;
                    let TestPlist::Text(name) = read_object(bytes, offsets, ref_size, key) else {
                        panic!("plist 字典的键必须是字符串");
                    };
                    dict.insert(name, value);
                }
                TestPlist::Dict(dict)
            }
            other => panic!("未知的 plist 标记族 {other:#x}"),
        }
    }

    fn dict_of(value: &TestPlist) -> &BTreeMap<String, TestPlist> {
        match value {
            TestPlist::Dict(dict) => dict,
            other => panic!("期望字典，得到 {other:?}"),
        }
    }

    /// 判据 (a)：产物根头与 chunk 名与实测布局一致，且 chunk 名是**字节反序**的。
    ///
    /// ⚠ 这里刻意写**实测字面量**（0x18 / 0x10 / `[0x23,0x47,0xC0,0xAB]` / `gnoS`），
    /// 而不是本模块自己的常量 —— 否则把常量改错时判据会跟着一起错，等于没有判据。
    #[test]
    fn project_data_header_and_chunk_ids_match_the_measured_layout() {
        let bundle = build_bundle(&fixture_project(), "004", "Fixture");
        let data = &bundle.files["Alternatives/004/ProjectData"];

        assert_eq!(
            data.len(),
            0x18 + u32_le(data, 0x10) as usize,
            "声明载荷长度（u32 小端 @0x10）必须等于 文件长度 − 0x18"
        );
        assert_eq!(&data[..4], &[0x23, 0x47, 0xC0, 0xAB], "根魔数");
        assert_eq!(
            &data[0x18..0x1c],
            &[0x67, 0x6e, 0x6f, 0x53],
            "实测第一个 chunk 名在 0x18，且是小端的 `gnoS`"
        );
        assert_ne!(&data[0x18..0x1c], b"Song", "chunk 名不是正序的 `Song`");
        assert_eq!(LOGIC_SONG_TAG, {
            let mut reversed = *LOGIC_SONG_NAME;
            reversed.reverse();
            reversed
        });

        let records = read_records(data);
        assert_eq!(records[0].tag, LOGIC_SONG_TAG);
        let tags: Vec<[u8; 4]> = records.iter().map(|record| record.tag).collect();
        assert!(tags.contains(&LOGIC_SEQUENCE_TAG), "必须有拍号/速度序列");
        assert!(tags.contains(&LOGIC_REGION_TAG), "必须有 region");
        assert_eq!(read_meter(&records), Some((3, 4)), "3/4 拍号必须回读");
        assert_eq!(read_song_tempo(&records), Some(128.0), "速度必须回读");
    }

    /// 判据：容器头写的是**本机实测的现代取值** —— 逐字段用**实测字面量**钉住。
    ///
    /// ⚠ 断言里刻意写**字面量**（`0x09D0`、`0x06..0x0f` 的十个字节、kind / subtype /
    /// +0x16 / +0x1a、`gnoS` 载荷前缀），**不引用本模块的常量** —— 把常量改错必须让本判据
    /// 变红，否则它只是一个"照镜子"的判据。
    ///
    /// 证据来源（全部本机实测，见 `LOGIC_FORMAT_VERSION_CODE` 的文档）：
    /// * 根头 `+0x04` = `0x09D0`（`Logic Pro 12.0.1 (6590)` 存过的工程）；
    /// * 根头 `0x06..0x0f` = `03 00 04 00 00 00 01 00 08 00`（10.2.4…12.0.1 全部一致）；
    /// * `gnoS`：kind 6 / subtype `0xFFFF` / `+0x08..+0x16` 是 14 字节 `0xFF`；
    /// * 记录头 `+0x16` = 2、`+0x18` = 0、`+0x1a` = 2（版本码 ≥ 2509）；
    /// * `gnoS` 载荷开头 = `#G` + 版本码 + `18 00 11 00`；
    /// * 拍号 `qSvE` kind/subtype = (1, 1)、速度 `qSvE` = (1, 3)、`qeSM` kind = 5。
    #[test]
    fn container_header_fields_carry_the_measured_modern_values() {
        let bundle = build_bundle(&fixture_project(), "000", "Header");
        let data = &bundle.files["Alternatives/000/ProjectData"];

        // 根头 +0x04..0x10：版本码 + 十个恒定字节。
        assert_eq!(
            &data[0x04..0x10],
            &[
                0xD0, 0x09, 0x03, 0x00, 0x04, 0x00, 0x00, 0x00, 0x01, 0x00, 0x08, 0x00
            ],
            "根头 +0x04 必须是实测的最新版本码 0x09D0（Logic Pro 12.0.1），\
             且 0x06..0x0f 是实测的十个恒定字节（写 0 = Logic 把它读成 Logic 4）"
        );

        // `gnoS`：实测第一条记录在 0x18。
        assert_eq!(&data[0x18..0x1c], b"gnoS", "第一条记录必须是 gnoS");
        let song = 0x18;
        assert_eq!(
            u16_le(data, song + 0x04),
            6,
            "gnoS 记录头 +4（kind）实测 2512 = 6"
        );
        assert_eq!(
            u16_le(data, song + 0x06),
            0xFFFF,
            "gnoS 记录头 +6（subtype）实测恒为 0xFFFF"
        );
        assert_eq!(
            &data[song + 0x08..song + 0x16],
            &[0xFF; 14],
            "gnoS 记录头 +0x08..+0x16 实测是 14 字节 0xFF 填充"
        );
        assert_eq!(u16_le(data, song + 0x16), 2, "记录头 +0x16 实测恒为 2");
        assert_eq!(u16_le(data, song + 0x18), 0, "记录头 +0x18 实测恒为 0");
        assert_eq!(
            u16_le(data, song + 0x1a),
            2,
            "记录头 +0x1a 实测格式版本码 >= 2509 恒为 2"
        );
        // `gnoS` 载荷（记录头之后）的嵌套 `#G` 子帧实测前缀。
        assert_eq!(
            &data[0x3c..0x3c + 10],
            &[0x23, 0x47, 0xC0, 0xAB, 0xD0, 0x09, 0x18, 0x00, 0x11, 0x00],
            "gnoS 载荷必须以实测的 #G 子帧前缀（#G + 版本码 + 18 00 11 00）开头"
        );

        // 拍号 / 速度 `qSvE` 与第一条 `qeSM`。
        let mut meter = None;
        let mut tempo = None;
        let mut region = None;
        for (at, tag) in record_offsets(data) {
            let body = at + LOGIC_RECORD_HEADER;
            if tag == *b"qSvE" && data[body..body + 4] == 0x30_u32.to_le_bytes() {
                meter = Some(at);
            }
            if tag == *b"qSvE" && data[body..body + 4] == 0x60_u32.to_le_bytes() {
                tempo = Some(at);
            }
            if tag == *b"qeSM" && region.is_none() {
                region = Some(at);
            }
        }
        let meter = meter.expect("必须有拍号 qSvE（载荷第一个字 0x30）");
        let tempo = tempo.expect("必须有速度 qSvE（载荷第一个字 0x60）");
        let region = region.expect("必须有 region qeSM");
        assert_eq!(
            (u16_le(data, meter + 4), u16_le(data, meter + 6)),
            (1, 1),
            "拍号 qSvE 的 kind/subtype 实测为 (1, 1)"
        );
        assert_eq!(
            (u16_le(data, tempo + 4), u16_le(data, tempo + 6)),
            (1, 3),
            "速度 qSvE 的 kind/subtype 实测为 (1, 3)"
        );
        assert_eq!(
            u16_le(data, region + 4),
            5,
            "region qeSM 的 kind 实测 2509+ 为 5"
        );
        for at in [meter, tempo, region] {
            assert_eq!(u16_le(data, at + 0x16), 2, "记录头 +0x16 实测恒为 2");
            assert_eq!(
                u16_le(data, at + 0x1a),
                2,
                "记录头 +0x1a 实测格式版本码 >= 2509 恒为 2"
            );
        }
    }

    /// 判据：拍号 / 速度 / 音符三条 `qSvE` 的**载荷逐字节**等于实测与 MIT 参考实现记录的形状。
    ///
    /// 断言里写的是**实测字面量**（五份文件的字节，`quiet` 0x09D0 与 `F0` 夹具 0x09CF 在无变化
    /// 时一致），**不引用本模块的常量** —— 把 `LOGIC_METER_HEADER_NO_CHANGE` /
    /// `LOGIC_TEMPO_PAYLOAD_LEN` / `LOGIC_EVENT_SEQUENCE_TAIL` 改错必须让本判据红。
    ///
    /// 三处此前**没有**被任何判决钉住的形状（本轮改正的缺陷）：
    /// 1. 拍号载荷原先是 **80** 字节、没有 16 字节尾；实测与参考实现（§5，
    ///    "`payload_size = 80 + 48*changes + 16`"）要求 96；
    /// 2. 速度载荷原先是 **16** 字节、位置与速度**都是 0**；实测五份全是 48 字节 =
    ///    32 字节事件 + 16 字节尾，事件里的速度字 = `round(bpm × 10000)`（夹具 128 BPM
    ///    ⇒ 1,280,000 = `00 88 13 00`），位置 = 38400 = `00 96 00 00 00 00 00 00`，
    ///    `+0x14` = `00 00 40 88`，`+0x18` = 7,200,000 = `00 dd 6d 00`；
    /// 3. 音符载荷末尾原先没有那 16 字节尾（参考实现 §8.5："payload = 32·N + 16"）。
    ///
    /// 负向实测：把速度事件的位置改成 0 ⇒ 本判据在 `+0x04` 的 8 个字节上红；
    /// 把拍号头上的 16 字节尾删掉 ⇒ 在载荷长度与逐字节比对两处红（本轮记录里有逐字失败行）。
    #[test]
    fn event_sequence_payloads_carry_the_measured_bytes() {
        let bundle = build_bundle(&fixture_project(), "000", "Sequences");
        let data = &bundle.files["Alternatives/000/ProjectData"];
        let records = read_records(data);

        let tail: [u8; 16] = [
            0xF1, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0x3F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ];

        // (1) 拍号：夹具是 3/4，因此 `+0x0c` 实测字面量是 `03`（分母指数 `+0x0b` = 2）。
        let meter = records
            .iter()
            .find(|record| {
                record.tag == LOGIC_SEQUENCE_TAG && record.body[..4] == 0x30_u32.to_le_bytes()
            })
            .expect("必须有拍号 qSvE");
        assert_eq!(
            meter.body.len(),
            96,
            "拍号载荷实测 96 = 80 字节头 + 16 字节尾"
        );
        #[rustfmt::skip]
        let expected_meter: Vec<u8> = vec![
            0x30, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x03, 0x00, 0x00, 0x80,
            0x30, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x88, 0xF6, 0xFF, 0x00, 0x00, 0x00, 0x96, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x88, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x32, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x88, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0xF1, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0x3F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(
            meter.body, expected_meter,
            "拍号载荷必须逐字节等于实测的 96 字节（80 字节头 + 16 字节尾）"
        );

        // (2) 速度：夹具 128 BPM ⇒ `round(128 × 10000)` = 1,280,000 = `00 88 13 00`。
        let tempo = records
            .iter()
            .find(|record| {
                record.tag == LOGIC_SEQUENCE_TAG && record.body[..4] == 0x60_u32.to_le_bytes()
            })
            .expect("必须有速度 qSvE");
        assert_eq!(
            tempo.body.len(),
            48,
            "速度载荷实测 48 = 32 字节事件 + 16 字节尾"
        );
        #[rustfmt::skip]
        let expected_tempo: Vec<u8> = vec![
            0x60, 0x00, 0x00, 0x00, 0x00, 0x96, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7F, 0x00, 0x00, 0x00,
            0x00, 0x88, 0x13, 0x00, 0x00, 0x00, 0x40, 0x88, 0x00, 0xDD, 0x6D, 0x00, 0x00, 0x00, 0x00, 0x00,
            0xF1, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0x3F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(
            tempo.body, expected_tempo,
            "速度载荷必须逐字节等于实测的 48 字节：位置 38400、速度 round(bpm×10000)、\
             +0x14 = 00 00 40 88、+0x18 = 7,200,000，后接 16 字节尾"
        );

        // (3) region 的音符序列：夹具 3 个音符 ⇒ 载荷 = 3×32 + 16 = 112，且以尾结束。
        let note = records
            .iter()
            .find(|record| {
                record.tag == LOGIC_SEQUENCE_TAG
                    && record
                        .body
                        .first()
                        .is_some_and(|byte| *byte == LOGIC_NOTE_STATUS)
            })
            .expect("必须有音符 qSvE");
        assert_eq!(
            note.body.len(),
            3 * 32 + 16,
            "音符载荷实测 = 32 × 音符数 + 16 字节尾（参考实现 §8.5）"
        );
        assert_eq!(
            &note.body[note.body.len() - 16..],
            &tail,
            "音符载荷必须以实测的 16 字节尾结束"
        );
    }

    /// 判据 (b)：空工程产出最小但合法的文档（只有速度/拍号/速度事件三个记录）。
    #[test]
    fn empty_project_produces_a_minimal_valid_document() {
        let bundle = build_bundle(&YebanProjectV1::default(), "000", "Empty");
        let data = &bundle.files["Alternatives/000/ProjectData"];
        let records = read_records(data);
        assert_eq!(records.len(), 3, "空工程 = gnoS + 拍号 + 速度事件");
        assert_eq!(records[0].tag, LOGIC_SONG_TAG);
        assert!(records.iter().all(|record| record.tag != LOGIC_REGION_TAG));
        assert_eq!(read_song_tempo(&records), Some(120.0));
        assert_eq!(read_meter(&records), Some((4, 4)));
        assert_eq!(bundle.mapped_regions, 0);
        assert_eq!(bundle.mapped_notes, 0);

        // 四个文件都在且非空。
        assert_eq!(bundle.files.len(), 4);
        for (path, bytes) in &bundle.files {
            assert!(!bytes.is_empty(), "{path} 不得为空");
        }
        let meta = &bundle.files["Alternatives/000/MetaData.plist"];
        let parsed = read_bplist(meta);
        let dict = dict_of(&parsed);
        assert_eq!(dict["NumberOfTracks"], TestPlist::Integer(0));
        assert_eq!(dict["BeatsPerMinute"], TestPlist::Real(120.0));
        assert_eq!(dict["SongSignatureNumerator"], TestPlist::Integer(4));
    }

    /// 判据 (c)：同一工程两次导出逐字节相同（含二进制 plist）。
    #[test]
    fn filled_project_bundle_is_byte_deterministic() {
        let project = yeban_model::samples::filled_project();
        let first = build_bundle(&project, "000", "Arrangement");
        let second = build_bundle(&project, "000", "Arrangement");
        assert_eq!(first.files.len(), second.files.len());
        for (path, bytes) in &first.files {
            assert_eq!(Some(bytes), second.files.get(path), "{path} 必须逐字节相同");
        }
        assert_eq!(first.losses, second.losses);
    }

    /// 判据 (d)：含不可映射构造的工程有**非空**损失表，两个前缀都出现，且表写进了文件。
    #[test]
    fn a_project_with_unmappable_features_has_a_non_vacuous_loss_table() {
        let empty = build_bundle(&YebanProjectV1::default(), "000", "Empty");
        let rich = build_bundle(&unmappable_project(), "000", "Rich");

        assert!(
            rich.losses
                .iter()
                .any(|loss| loss.reason.starts_with(LOSS_UNMAPPED_PREFIX)),
            "必须有 `未映射:` 条目"
        );
        assert!(
            rich.losses
                .iter()
                .any(|loss| loss.reason.starts_with(LOSS_NOT_EQUIVALENT_PREFIX)),
            "必须有 `非等价:` 条目"
        );
        // 有牙：空工程不该凭空长出这些条目（证明它们来自工程内容，不是常量）。
        assert!(
            !empty
                .losses
                .iter()
                .any(|loss| loss.reason.contains("音频轨")),
            "空工程不得有音频轨条目"
        );
        assert!(
            rich.losses
                .iter()
                .any(|loss| loss.reason.contains("音频轨")),
            "音频轨必须被点名"
        );
        assert!(
            rich.losses
                .iter()
                .any(|loss| loss.reason.contains("辅助返回轨")),
            "辅助返回轨必须被点名"
        );
        assert!(
            rich.losses
                .iter()
                .any(|loss| loss.reason.contains("自动化泳道")),
            "自动化泳道必须被点名"
        );

        // 表必须与文件同源：MetaData.plist 的扩展键逐条等于返回的损失表。
        let embedded: Vec<String> = rich
            .losses
            .iter()
            .map(|loss| format!("{}: {}", loss.entity, loss.reason))
            .collect();
        let parsed = read_bplist(&rich.files["Alternatives/000/MetaData.plist"]);
        let dict = dict_of(&parsed);
        let Some(TestPlist::Array(items)) = dict.get(META_DATA_LOSS_KEY) else {
            panic!("MetaData.plist 必须带 {META_DATA_LOSS_KEY}");
        };
        let written: Vec<String> = items
            .iter()
            .map(|item| match item {
                TestPlist::Text(text) => text.clone(),
                other => panic!("损失条目必须是字符串，得到 {other:?}"),
            })
            .collect();
        assert_eq!(written, embedded, "文件里的损失表必须与返回的逐条相同");
    }

    /// 判据 (e)：**报告出来的**损失表与 `MetaData.plist` 里**落盘的**那张表逐条相同。
    ///
    /// 这条补的是模块头那句"表与文件因此不会各说各话"：判据 (d) 只证明了
    /// "想写的 == 编解码回来的"，那是**写路径**的自洽；本判据把 [`LogicBundle::losses`]
    /// （CLI 报告的那张表，见 `crates/yeban-app/src/cli.rs` 的 `logic_loss_lines`）与
    /// **从字节重新解出来的** `YebanMappingLosses` 对账，覆盖：条数、顺序、每条的
    /// `"<entity>: <reason>"` 全文，以及 `未映射:` / `非等价:` 的分类。任一侧改动
    /// （漏一条、换序、改理由）都会红。
    ///
    /// 解码路径：本模块的 `read_bplist` —— 它是**独立于编码器**重新实现的 bplist00
    /// 读取器（自己走偏移表与 marker，不调用 `encode_binary_plist` 的 `PlistValue`
    /// 数据），但它与编码器同在一个文件、同一个 crate，仍不是第三方裁判；真正的外部
    /// 裁判要另起依赖（本机离线做不到，见模块头"为什么不用 plist crate"）。
    #[test]
    fn reported_loss_table_equals_the_embedded_plist_table_entry_for_entry() {
        // 自研路线。
        assert_plist_matches_losses(&build_bundle(&unmappable_project(), "000", "Reported"));
        // 供体路线：它的损失表是**另一套**条目（逐族登记"这条记录仍是供体的"），
        // 因此同一份逐条对账必须两条路线都跑，否则供体那边可以悄悄漂移。
        assert_plist_matches_losses(&build_bundle_from_donor(
            &unmappable_project(),
            "000",
            "Reported",
        ));
    }

    /// 落盘的 [`META_DATA_LOSS_KEY`] 与 `bundle.losses` 逐条（条数 / 顺序 / 全文 / 分类）相同。
    fn assert_plist_matches_losses(bundle: &LogicBundle) {
        assert!(
            !bundle.losses.is_empty(),
            "判据不得空转：含不可映射构造的工程必须至少有一条损失"
        );
        assert!(
            bundle
                .losses
                .iter()
                .any(|loss| loss.reason.starts_with(LOSS_UNMAPPED_PREFIX)),
            "表里必须有 `未映射:` 条目，否则分类对账是空转"
        );
        assert!(
            bundle
                .losses
                .iter()
                .any(|loss| loss.reason.starts_with(LOSS_NOT_EQUIVALENT_PREFIX)),
            "表里必须有 `非等价:` 条目，否则分类对账是空转"
        );

        let parsed = read_bplist(&bundle.files["Alternatives/000/MetaData.plist"]);
        let dict = dict_of(&parsed);
        let Some(TestPlist::Array(items)) = dict.get(META_DATA_LOSS_KEY) else {
            panic!("MetaData.plist 必须带 {META_DATA_LOSS_KEY} 且是字符串数组");
        };
        assert_eq!(
            items.len(),
            bundle.losses.len(),
            "落盘表条数必须等于报告表条数"
        );

        for (index, (item, loss)) in items.iter().zip(&bundle.losses).enumerate() {
            let TestPlist::Text(entry) = item else {
                panic!("第 {index} 条落盘损失必须是字符串，得到 {item:?}");
            };
            let (entity, reason) = entry
                .split_once(": ")
                .expect("落盘损失必须形如 `<entity>: <reason>`");
            assert_eq!(
                entity, loss.entity,
                "第 {index} 条落盘损失的 entity 与报告表不同（落盘 `{entity}` / 报告 `{}`）",
                loss.entity
            );
            assert_eq!(
                reason, loss.reason,
                "第 {index} 条落盘损失的 reason 与报告表不同（落盘 `{reason}` / 报告 `{}`）",
                loss.reason
            );
            let classified = if loss.reason.starts_with(LOSS_UNMAPPED_PREFIX) {
                "未映射"
            } else if loss.reason.starts_with(LOSS_NOT_EQUIVALENT_PREFIX) {
                "非等价"
            } else {
                panic!("第 {index} 条报告的损失没有可分类的前缀：`{entry}`");
            };
            assert_eq!(
                classified == "未映射",
                reason.starts_with(LOSS_UNMAPPED_PREFIX),
                "第 {index} 条（`{entry}`）的分类与报告表不同：报告是 `{classified}`，\
                 落盘的 reason 里没有对应的前缀"
            );
            assert_eq!(
                entry,
                &format!("{}: {}", loss.entity, loss.reason),
                "第 {index} 条落盘损失的内容与报告表不同"
            );
        }
    }

    /// 往返：写入的音符经"实测布局的读取器"读回（音高/力度/起点/时值）。
    #[test]
    fn written_notes_round_trip_through_the_measured_layout() {
        let bundle = build_bundle(&fixture_project(), "000", "Fixture");
        let data = &bundle.files["Alternatives/000/ProjectData"];
        let records = read_records(data);

        let region = records
            .iter()
            .find(|record| record.tag == LOGIC_REGION_TAG)
            .expect("必须有一个 region");
        assert_eq!(read_region_name(region), "Piano");
        let sequence = records
            .iter()
            .find(|record| {
                record.tag == LOGIC_SEQUENCE_TAG
                    && record.cluster == region.cluster
                    && record
                        .body
                        .first()
                        .is_some_and(|byte| *byte == LOGIC_NOTE_STATUS)
            })
            .expect("region 必须有音符序列");
        let notes = decode_note_lines(&sequence.body);
        assert_eq!(
            notes,
            vec![
                (3_840, 480, 60, 100),
                (4_800, 480, 64, 90),
                (5_760, 480, 67, 80),
            ],
            "摆放起点 (3840) + 音符内起点，时值 480，音高/力度逐条相同"
        );
        assert_eq!(bundle.mapped_regions, 1);
        assert_eq!(bundle.mapped_notes, 3);
    }

    /// 判据：region 名写在**实测的载荷偏移**上 —— 长度 `u16` 小端**字节数** @ 载荷 `+0x10`、
    /// UTF-8 名字 @ `+0x12`、其后 4 字节 0。断言里写**实测字面量**（不引用本模块的常量），
    /// 因此改偏移、改长度口径（字节 ↔ 字符）、改编码都会红。
    ///
    /// 名字用非 ASCII 的 `夜半`（6 字节 UTF-8），把"长度 = **字节数**而不是字符数"钉死：
    /// 若按字符数写会得到 `02 00`，这里的期望是 `06 00` 并逐字节核对 UTF-8。
    /// 实测依据见 [`LOGIC_REGION_NAME_PAYLOAD_OFFSET`]（两例演示 790/790 条 `qeSM` 一致）。
    #[test]
    fn region_name_field_sits_at_the_measured_payload_offsets() {
        let mut project = fixture_project();
        let clip_id = test_id(10);
        project
            .clip_pool
            .get_mut(&clip_id)
            .expect("夹具必须有片段池条目")
            .name = "夜半".to_owned();

        let bundle = build_bundle(&project, "000", "RegionName");
        let data = &bundle.files["Alternatives/000/ProjectData"];
        let records = read_records(data);
        let region = records
            .iter()
            .find(|record| record.tag == LOGIC_REGION_TAG)
            .expect("必须有 region qeSM");

        // 载荷 +0x10 = u16 小端字节数（`夜半` = e5 a4 9c e5 8d 8a = **6** 字节，不是 2 个字符）。
        assert_eq!(
            &region.body[0x10..0x12],
            &[0x06, 0x00],
            "region 载荷 +0x10 必须是实测的 u16 小端**字节数**（夜半 = 6 字节）"
        );
        assert_eq!(
            &region.body[0x12..0x18],
            &[0xE5, 0xA4, 0x9C, 0xE5, 0x8D, 0x8A],
            "region 载荷 +0x12 起必须是名字的 UTF-8 字节"
        );
        assert_eq!(
            &region.body[0x18..0x1c],
            &[0, 0, 0, 0],
            "名字之后的 4 个字节实测全为 0"
        );
        // 记录内偏移 = 0x24 + 0x10 = 0x34：两个参照系指向同一个字节。
        assert_eq!(
            LOGIC_RECORD_HEADER + LOGIC_REGION_NAME_PAYLOAD_OFFSET,
            0x34,
            "实测的记录内偏移是 0x34"
        );
        assert_eq!(read_region_name(region), "夜半");
    }

    /// **可选**判据：本机存在 Apple 演示工程时，用它核对同一套头部读数；不存在就跳过。
    ///
    /// ⚠ 演示工程**有版权、不进仓库**，CI 上不存在 ⇒ 本判据在 CI 里是 skip，绝不红。
    /// 它证明的是"本模块的常量与本机实测一致"，不是"Logic 能打开"。
    ///
    /// 除了魔数/声明长度/第一个 chunk，这里还核对**根头版本码非零且落在实测区间内**、
    /// **根头 0x06..0x0f 是那十个恒定字节**、以及**每条记录的 +0x16 都是 2** ——
    /// 这三条就是本模块写进产物的那些字段在本机真实文件里的 ground truth。
    #[test]
    fn local_demo_projects_match_the_measured_header_layout_when_present() {
        let demos = [
            "/Library/Application Support/Logic/Logic Pro X Demosongs/Swing!.logicx/Alternatives/004/ProjectData",
            "/Library/Application Support/Logic/Logic Pro X Demosongs/ocean eyes.logicx/Alternatives/001/ProjectData",
        ];
        let mut checked = 0usize;
        for demo in demos {
            let Ok(bytes) = std::fs::read(demo) else {
                continue;
            };
            assert_eq!(&bytes[..4], &LOGIC_ROOT_MAGIC, "{demo}");
            assert_eq!(
                u32_le(&bytes, LOGIC_DECLARED_LENGTH_OFFSET) as usize,
                bytes.len() - LOGIC_ROOT_HEADER,
                "{demo} 的声明长度"
            );
            assert_eq!(
                &bytes[LOGIC_FIRST_RECORD_OFFSET..LOGIC_FIRST_RECORD_OFFSET + 4],
                &LOGIC_SONG_TAG,
                "{demo} 的第一个 chunk 必须是 gnoS"
            );
            // 版本码：实测区间 0x06DC（10.2.4）…0x09D0（12.0.1）；这两个演示是 0x09CB/0x07D0。
            let version = u16_le(&bytes, LOGIC_FORMAT_VERSION_OFFSET);
            assert!(
                (0x06DC..=0x09D0).contains(&version),
                "{demo} 的根头版本码必须落在实测区间内，实测 {version:#06x}"
            );
            assert_eq!(
                &bytes[LOGIC_ROOT_STABLE_OFFSET..LOGIC_ROOT_STABLE_OFFSET + 10],
                &LOGIC_ROOT_STABLE_FIELDS,
                "{demo} 的根头 0x06..0x0f 必须是那十个恒定字节"
            );
            for (at, tag) in record_offsets(&bytes) {
                assert_eq!(
                    u16_le(&bytes, at + 0x16),
                    2,
                    "{demo} 的 {:?} 记录 +0x16 实测恒为 2",
                    String::from_utf8_lossy(&tag)
                );
                assert_eq!(
                    u16_le(&bytes, at + 0x18),
                    0,
                    "{demo} 的记录头 +0x18 实测恒为 0"
                );
            }
            checked += 1;
        }
        eprintln!("可选演示工程核对：{checked} 个文件存在并核对通过（不存在 = skip）");
    }

    /// **可选**判据：本机存在 Apple 演示工程时，逐条核对**全部** `qeSM` 记录的名字字段就是
    /// 本模块写入的布局 —— 载荷 `+0x10` = `u16` 小端 **UTF-8 字节数**、`+0x12` 起名字、
    /// 其后 4 字节全 0；路径不存在就 skip（CI 上不存在 ⇒ 绝不红）。
    ///
    /// 这条把"实测"变成可复跑的判据：它读真实文件的字节，但**不把任何字节或字符串内容写进
    /// 仓库**（只打印条数与偏移）。两例演示共 790 条 `qeSM`，全部命中同一布局。
    #[test]
    fn real_demo_qesm_records_use_the_measured_name_field_when_present() {
        let demos = [
            "/Library/Application Support/Logic/Logic Pro X Demosongs/Swing!.logicx/Alternatives/004/ProjectData",
            "/Library/Application Support/Logic/Logic Pro X Demosongs/ocean eyes.logicx/Alternatives/001/ProjectData",
        ];
        let mut present = 0usize;
        for demo in demos {
            let Ok(bytes) = std::fs::read(demo) else {
                continue;
            };
            present += 1;
            let mut regions = 0usize;
            for (at, tag, size, _cluster) in walk_chunk_run(&bytes, LOGIC_ROOT_HEADER, bytes.len())
            {
                if tag != LOGIC_REGION_TAG {
                    continue;
                }
                let payload = &bytes[at + LOGIC_RECORD_HEADER..at + LOGIC_RECORD_HEADER + size];
                assert!(
                    payload.len() >= 0x16,
                    "{demo} 的 qeSM 载荷只有 {} 字节，放不下 +0x10 的字段",
                    payload.len()
                );
                let length = usize::from(u16_le(payload, 0x10));
                assert!(
                    0x12 + length + 4 <= payload.len(),
                    "{demo} 的 qeSM 载荷 +0x10 长度字段 {length} 超出载荷（{} 字节）",
                    payload.len()
                );
                let name =
                    std::str::from_utf8(&payload[0x12..0x12 + length]).unwrap_or_else(|_| {
                        panic!("{demo} 的 qeSM 载荷 +0x12 起 {length} 字节不是合法 UTF-8")
                    });
                assert_eq!(
                    &payload[0x12 + length..0x12 + length + 4],
                    &[0, 0, 0, 0],
                    "{demo} 的名字 `{name}` 之后 4 字节实测必须全为 0"
                );
                regions += 1;
            }
            assert!(regions > 0, "{demo} 必须至少有 1 条 qeSM");
            eprintln!(
                "可选 qeSM 名字布局核对：{demo} —— {regions} 条记录，载荷 +0x10 = u16 小端字节数、\
                 +0x12 起 UTF-8 名字、其后 4 字节 0"
            );
        }
        if present == 0 {
            eprintln!("skip: 本机没有 Apple 演示工程（CI 上不存在 ⇒ 本判据不跑、绝不红）");
        }
    }

    // ---- chunk 家族对账（诊断） ----

    /// 走完一段 chunk 流（从 `base` 到 `limit`），返回 `(偏移, tag, size, cluster)`。
    ///
    /// **只由载荷长度驱动**，绝不扫描下一个 tag：载荷里可以出现任意四个字节。
    fn walk_chunk_run(
        bytes: &[u8],
        base: usize,
        limit: usize,
    ) -> Vec<(usize, [u8; 4], usize, u32)> {
        let mut out = Vec::new();
        let mut at = base;
        while at + LOGIC_RECORD_HEADER <= limit {
            let tag = [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]];
            let size = u32_le(bytes, at + LOGIC_RECORD_SIZE_OFFSET) as usize;
            let cluster = u32_le(bytes, at + LOGIC_RECORD_CLUSTER_OFFSET);
            out.push((at, tag, size, cluster));
            let end = at + LOGIC_RECORD_HEADER + size;
            if end > limit {
                break;
            }
            at = end;
        }
        out
    }

    /// 一段 chunk 流里**嵌套**的 chunk 家族（从"载荷以根魔数开头"的地方按同一套记录头再走一层）。
    ///
    /// 实测：真实工程与本产物都**没有**嵌套 chunk 标识符（`gnoS` 载荷虽以根魔数 `#G` 开头，
    /// 其后不是 chunk 流），因此本函数返回空——诊断判据把它**打印出来**而不是假装不存在。
    fn nested_chunk_families(bytes: &[u8], base: usize, limit: usize) -> Vec<String> {
        let mut found = Vec::new();
        for (at, _tag, size, _cluster) in walk_chunk_run(bytes, base, limit) {
            let body = at + LOGIC_RECORD_HEADER;
            let end = body + size;
            if size < LOGIC_ROOT_HEADER
                || end > bytes.len()
                || bytes[body..body + 4] != LOGIC_ROOT_MAGIC
            {
                continue;
            }
            for (nested_at, nested_tag, nested_size, _nested_cluster) in
                walk_chunk_run(bytes, body + LOGIC_ROOT_HEADER, end)
            {
                let fits = nested_at + LOGIC_RECORD_HEADER + nested_size <= end;
                if fits && nested_tag.iter().all(u8::is_ascii_graphic) {
                    found.push(decode_chunk_tag(&nested_tag));
                }
            }
        }
        found
    }

    /// 一份 `ProjectData` 实际写出的 chunk 家族（解码后可读名）。
    fn emitted_chunk_families(bytes: &[u8]) -> BTreeSet<String> {
        walk_chunk_run(bytes, LOGIC_ROOT_HEADER, bytes.len())
            .into_iter()
            .map(|(_at, tag, _size, _cluster)| decode_chunk_tag(&tag))
            .collect()
    }

    /// 解码：落盘字节 → 可读名（`gnoS` ⇒ `Song`）。
    #[test]
    fn chunk_tags_decode_by_reversing_their_stored_bytes() {
        assert_eq!(decode_chunk_tag(b"gnoS"), "Song");
        assert_eq!(decode_chunk_tag(b"qSvE"), "EvSq");
        assert_eq!(decode_chunk_tag(b"qeSM"), "MSeq");
        assert_eq!(decode_chunk_tag(&LOGIC_SONG_TAG), "Song");
        assert_eq!(stored_chunk_tag("Song"), "gnoS");
    }

    /// **无头确定性判据**：产物写出的 chunk 家族**只能是** [`WRITTEN_CHUNK_FAMILIES`]
    /// （有 region 的工程必须写满三个），且产物里没有嵌套 chunk 标识符（多写一族就红）。
    #[test]
    fn emitted_chunk_families_are_exactly_the_declared_written_set() {
        let declared: BTreeSet<&str> = WRITTEN_CHUNK_FAMILIES.into_iter().collect();
        for (project, alternative, expect_all) in [
            (YebanProjectV1::default(), "000", false),
            (fixture_project(), "000", true),
            (unmappable_project(), "001", true),
        ] {
            let bundle = build_bundle(&project, alternative, "Chunks");
            let data = &bundle.files[&format!("Alternatives/{alternative}/ProjectData")];
            let emitted = emitted_chunk_families(data);
            let emitted_refs: BTreeSet<&str> = emitted.iter().map(String::as_str).collect();
            let undeclared: Vec<&&str> = emitted_refs.difference(&declared).collect();
            assert!(
                undeclared.is_empty(),
                "{alternative}: 产物写出了未声明的 chunk 家族 {undeclared:?}"
            );
            if expect_all {
                assert_eq!(
                    emitted_refs, declared,
                    "{alternative}: 有 region 的工程必须写满声明的三个家族"
                );
            }
            assert!(
                nested_chunk_families(data, LOGIC_ROOT_HEADER, data.len()).is_empty(),
                "{alternative}: 产物里不得出现嵌套 chunk 标识符"
            );
        }
    }

    /// **无头确定性判据**：实测差集 `真实 − 产物` **恰好**等于 [`MISSING_CHUNK_FAMILIES`]，
    /// 且每个缺失家族都有一条**点名它**的 `未映射:` 损失条目（缺一条、或某族没进损失表就红）。
    #[test]
    fn every_missing_chunk_family_is_registered_in_the_loss_table() {
        let real: BTreeSet<&str> = MEASURED_REAL_CHUNK_FAMILIES.into_iter().collect();
        let written: BTreeSet<&str> = WRITTEN_CHUNK_FAMILIES.into_iter().collect();
        assert_eq!(
            real.len(),
            MEASURED_REAL_CHUNK_FAMILIES.len(),
            "实测家族列表不得重复"
        );
        assert_eq!(
            written.len(),
            WRITTEN_CHUNK_FAMILIES.len(),
            "写入家族列表不得重复"
        );
        let difference: BTreeSet<&str> = real.difference(&written).copied().collect();
        let declared: BTreeSet<&str> = MISSING_CHUNK_FAMILIES
            .iter()
            .map(|(name, _role)| *name)
            .collect();
        assert_eq!(
            declared.len(),
            MISSING_CHUNK_FAMILIES.len(),
            "缺失清单不得重复"
        );
        assert_eq!(declared, difference, "缺失清单必须恰好等于实测差集");

        let bundle = build_bundle(&fixture_project(), "000", "Losses");
        let data = &bundle.files["Alternatives/000/ProjectData"];
        let emitted = emitted_chunk_families(data);
        for (name, _role) in MISSING_CHUNK_FAMILIES {
            assert!(
                !emitted.contains(name),
                "{name} 是缺失家族，不该出现在产物里"
            );
            assert!(
                bundle.losses.iter().any(|loss| {
                    loss.reason.starts_with(LOSS_UNMAPPED_PREFIX) && loss.reason.contains(name)
                }),
                "缺失家族 {name} 必须有一条点名它的 `未映射:` 条目"
            );
        }
        assert!(
            bundle
                .losses
                .iter()
                .any(|loss| loss.reason == CONTAINER_HEADER_CAVEAT),
            "容器头的实测偏差必须登记为损失"
        );
        assert!(
            bundle
                .losses
                .iter()
                .any(|loss| loss.reason == REGION_PAYLOAD_CAVEAT),
            "region 载荷除名字字段以外的实测偏差必须登记为损失"
        );
    }

    /// **诊断判据（路径驱动：两个本机 Apple 演示工程都存在时才跑，否则干净 skip）**：
    /// 把真实工程与产物的顶层/嵌套 chunk 家族**并排**打印，并证明实测并集逐族等于声明列表。
    ///
    /// 选这种形态的理由：同一份测试二进制、零新目标、零新依赖，且本模块已有
    /// `local_demo_projects_match_the_measured_header_layout_when_present` 用同一套
    /// "路径不存在就 skip" 的纪律（CI 上没有这些路径 ⇒ 绝不红）。打印要 `-- --nocapture`：
    /// `cargo test -p yeban-render --features experimental-logic-export logic_chunk_families \
    ///   -- --nocapture`
    ///
    /// ⚠ Apple 的演示工程**有版权、不进仓库**：本判据只**读**它们的 chunk **名字**并打印，
    /// 不把任何字节或字符串内容写进仓库。
    #[test]
    fn logic_chunk_families_compare_side_by_side_when_the_demos_are_present() {
        let demos = [
            "/Library/Application Support/Logic/Logic Pro X Demosongs/Swing!.logicx/Alternatives/004/ProjectData",
            "/Library/Application Support/Logic/Logic Pro X Demosongs/ocean eyes.logicx/Alternatives/001/ProjectData",
        ];

        let bundle = build_bundle(&yeban_model::samples::filled_project(), "000", "Chunks");
        let ours_bytes = &bundle.files["Alternatives/000/ProjectData"];
        let ours = emitted_chunk_families(ours_bytes);
        let ours_nested = nested_chunk_families(ours_bytes, LOGIC_ROOT_HEADER, ours_bytes.len());
        eprintln!(
            "ours  Alternatives/000/ProjectData ({} B): {}",
            ours_bytes.len(),
            ours.iter().cloned().collect::<Vec<_>>().join(", ")
        );
        assert_eq!(
            ours.iter().map(String::as_str).collect::<BTreeSet<_>>(),
            WRITTEN_CHUNK_FAMILIES.into_iter().collect::<BTreeSet<_>>()
        );

        let mut measured: BTreeSet<String> = BTreeSet::new();
        let mut real_nested: BTreeSet<String> = BTreeSet::new();
        let mut present = 0usize;
        for demo in demos {
            let Ok(bytes) = std::fs::read(demo) else {
                continue;
            };
            present += 1;
            let records = walk_chunk_run(&bytes, LOGIC_ROOT_HEADER, bytes.len());
            let families: BTreeSet<String> = records
                .iter()
                .map(|(_at, tag, _size, _cluster)| decode_chunk_tag(tag))
                .collect();
            real_nested.extend(nested_chunk_families(
                &bytes,
                LOGIC_ROOT_HEADER,
                bytes.len(),
            ));
            measured.extend(families.iter().cloned());
            eprintln!(
                "real  {} ({} B, {} chunk records): {}",
                demo.split('/').nth(5).unwrap_or(demo),
                bytes.len(),
                records.len(),
                families.iter().cloned().collect::<Vec<_>>().join(", ")
            );
        }
        if present == 0 {
            eprintln!("skip: 本机没有 Apple 演示工程（CI 上不存在 ⇒ 本判据不跑、绝不红）");
            return;
        }
        eprintln!(
            "nested chunk identifiers — ours: [{}] ; real: [{}]",
            ours_nested.join(", "),
            real_nested.iter().cloned().collect::<Vec<_>>().join(", ")
        );

        let measured_refs: BTreeSet<&str> = measured.iter().map(String::as_str).collect();
        let declared: BTreeSet<&str> = MEASURED_REAL_CHUNK_FAMILIES.into_iter().collect();
        let unrecorded: Vec<&&str> = measured_refs.difference(&declared).collect();
        assert!(
            unrecorded.is_empty(),
            "实测到未登记的 chunk 家族（必须补进 MEASURED_REAL_CHUNK_FAMILIES）: {unrecorded:?}"
        );
        if present == demos.len() {
            assert_eq!(
                measured_refs, declared,
                "两个演示工程都存在时，实测并集必须逐族等于声明的 27 个家族"
            );
        }
        let written: BTreeSet<&str> = WRITTEN_CHUNK_FAMILIES.into_iter().collect();
        let difference: BTreeSet<&str> = measured_refs.difference(&written).copied().collect();
        eprintln!(
            "set difference (real has, ours does not): {} families: {}",
            difference.len(),
            difference.iter().copied().collect::<Vec<_>>().join(", ")
        );
    }

    // ---- `Trak` 轨道家族：布局已实测、语义未反推 ⇒ 刻意不写 ----

    /// **无头确定性判据（有牙）**：本切片**刻意不写** `Trak` 轨道家族，并把这一省略**点名登记**
    /// 在损失表里，理由里带本轮从 MIT 参考实现 + 五份真实文件得到的读数。
    ///
    /// 实测事实（见模块头"三个语义问题的答案"）：`Trak` 的**落盘**四个字节是 `6b 61 72 54`；
    /// 记录头 `+0x08` 的对象号把它分成三个子群 —— `0x00040000`（编排轨行，非 0 载荷条数 − 1
    /// 恰等于 `NumberOfTracks`）、`0x00080000`（每个预分配混音槽一条 Track 对象）、其余少量空载荷。
    /// 本判据用**实测字面量**（不是本模块的常量）钉住"不写"这个决定，并要求登记文本里带齐
    /// **可核对的读数**（两个对象号、五份文件的 −1 减法、master 槽 `0x50`、subtype 23、
    /// 参考实现的名字）—— 登记文本漂回"语义未知"就会红。
    ///
    /// 负向实测：让写入器多产一条落盘 `6b 61 72 54` 记录 ⇒ 本判据红；把登记文本里的子群读数
    /// 删掉 ⇒ 本判据红（两次都在本轮记录里有逐字失败行）。
    #[test]
    fn track_family_is_measured_but_deliberately_unwritten() {
        // 实测字面量（刻意不用模块常量：常量改错时判据会跟着一起错，等于没有判据）。
        const STORED_TRACK_TAG: [u8; 4] = [0x6b, 0x61, 0x72, 0x54];
        for (project, alternative) in [(fixture_project(), "000"), (unmappable_project(), "001")] {
            let bundle = build_bundle(&project, alternative, "Tracks");
            let data = &bundle.files[&format!("Alternatives/{alternative}/ProjectData")];
            let strays: Vec<usize> = read_records(data)
                .iter()
                .enumerate()
                .filter(|(_index, record)| record.tag == STORED_TRACK_TAG)
                .map(|(index, _record)| index)
                .collect();
            assert!(
                strays.is_empty(),
                "{alternative}: 产物里不得出现落盘 `karT` 记录 —— 参考实现（MIT）说轨道不能凭空合成，\
                 只能克隆 Logic 存过的 donor 的通道簇，而**自研**路线不引入这种 donor\
                 （donor 路线是另一条：`build_bundle_from_donor`）；\
                 实测多出的记录下标 {strays:?}"
            );
            assert!(
                !emitted_chunk_families(data).contains("Trak"),
                "{alternative}: `Trak` 不得进入写出家族集合"
            );
            let loss = bundle
                .losses
                .iter()
                .find(|loss| {
                    loss.reason.starts_with(LOSS_UNMAPPED_PREFIX)
                        && loss.reason.contains("Trak")
                        && loss.reason.contains("karT")
                })
                .unwrap_or_else(|| {
                    panic!("{alternative}: `Trak` 必须有一条点名它的 `未映射:` 损失条目")
                });
            // 登记文本必须带**本轮实测的读数**，而不是"语义未知"。
            for needle in [
                "0x00040000",
                "0x00080000",
                "2−1=1",
                "77−1=76",
                "43−1=42",
                "11−1=10",
                "35−1=34",
                "0x50",
                "subtype 恒 23",
                "jonkubis/logicproformatwriter",
            ] {
                assert!(
                    loss.reason.contains(needle),
                    "{alternative}: `Trak` 的损失条目必须写出实测读数 {needle:?}，\
                     实际登记文本为：{}",
                    loss.reason
                );
            }
        }
    }

    /// **路径驱动判据（两个本机 Apple 演示工程都存在时才跑，否则干净 skip）**：
    /// 用**实测字面量**钉住 `Trak` 家族在真实文件里的形状 —— 这正是"本切片为什么不写它"的证据，
    /// 也是本轮"三个语义问题的答案"在真实字节上的复核。
    ///
    /// 每份文件断言：字节数（`stat -f%z` 口径）、全文件记录条数（行走恰好铺满声明载荷）、`Trak` 条数、
    /// 载荷长 0 / 非 0 的条数与长度、记录头 kind 与 subtype（subtype 恒 23）、**正序 `karT` 四字节出现
    /// 0 次**（无嵌套）、以及**没有任何 `Trak` 载荷含"`u16` 长度（`2..=96`）+ 全可打印 ASCII + `\0`"
    /// 形态的名字字段**。
    ///
    /// **本轮新增（语义答案的证据）**：读同目录 `MetaData.plist` 的 `NumberOfTracks`，然后
    /// * 记录头 `+0x08 == 0x00040000` 组（编排轨行）的**非 0 载荷条数 − 1 必须恰好等于**
    ///   `NumberOfTracks`（`Swing!` 77−1=76、`ocean eyes` 43−1=42）；
    /// * 该组里**恰好一条** master 行（载荷 `+0x00 == 3` 且载荷 `+0x08 == 0x50`），其余行 `+0x00 == 1`；
    /// * 该组的记录头 `+0x12` 单字节序号排序后是 `0..=NumberOfTracks`（master 最后）；
    /// * `+0x08 == 0x00080000` 组（每个预分配混音槽一条 Track 对象）的非 0 条数等于实测字面量，
    ///   且载荷 `+0x08` 的槽号**互不相同**。
    ///
    /// ⚠ Apple 的演示工程**有版权、不进仓库**：本判据只**读**它们并按上面的字面量核对，
    /// 不把任何字节或字符串内容写进仓库。路径不存在就 `continue`（CI 上不存在 ⇒ 绝不红）；
    /// 两个都不存在时打印 skip 行。
    #[test]
    fn real_demo_track_family_shape_matches_the_measurement_when_present() {
        // (路径, 字节数, 全文件记录数, Trak 条数, Trak 空载荷条数, Trak 非 0 载荷条数,
        //  Trak 非 0 载荷长度, Trak kind, MetaData.plist NumberOfTracks,
        //  `+0x08 == 0x40000` 组非 0 条数（= 轨道数 + 1）, `+0x08 == 0x80000` 组非 0 条数)
        let demos = [
            (
                "/Library/Application Support/Logic/Logic Pro X Demosongs/Swing!.logicx/Alternatives/004/ProjectData",
                5_648_035u64,
                4_626usize,
                799usize,
                545usize,
                254usize,
                57u32,
                5u16,
                76u64,
                77usize,
                173usize,
            ),
            (
                "/Library/Application Support/Logic/Logic Pro X Demosongs/ocean eyes.logicx/Alternatives/001/ProjectData",
                4_075_622u64,
                4_094usize,
                753usize,
                245usize,
                508usize,
                56u32,
                4u16,
                42u64,
                43usize,
                104usize,
            ),
        ];
        // 实测字面量：`Trak` 的落盘四个字节；正序拼写 `karT` 在本族的任何位置都不出现。
        const STORED_TRACK_TAG: [u8; 4] = [0x6b, 0x61, 0x72, 0x54];
        const FORWARD_KART: [u8; 4] = [0x54, 0x72, 0x61, 0x6b];

        let mut present = 0usize;
        for (
            path,
            size,
            record_count,
            trak_total,
            trak_empty,
            trak_filled,
            filled_len,
            kind,
            plist_tracks,
            arrange_filled,
            slot_objects_filled,
        ) in demos
        {
            let Ok(bytes) = std::fs::read(path) else {
                continue;
            };
            present += 1;
            assert_eq!(bytes.len() as u64, size, "{path}: ProjectData 字节数");
            let offsets = record_offsets(&bytes);
            assert_eq!(offsets.len(), record_count, "{path}: 全文件记录条数");
            let end = offsets.last().map_or(LOGIC_ROOT_HEADER, |(at, _tag)| {
                *at + LOGIC_RECORD_HEADER + u32_le(&bytes, *at + LOGIC_RECORD_SIZE_OFFSET) as usize
            });
            assert_eq!(end, bytes.len(), "{path}: 记录流必须恰好铺满声明载荷");

            let trak: Vec<usize> = offsets
                .iter()
                .filter(|(_at, tag)| *tag == STORED_TRACK_TAG)
                .map(|(at, _tag)| *at)
                .collect();
            assert_eq!(trak.len(), trak_total, "{path}: Trak 记录条数");
            let empty = trak
                .iter()
                .filter(|at| u32_le(&bytes, *at + LOGIC_RECORD_SIZE_OFFSET) == 0)
                .count();
            let filled: Vec<usize> = trak
                .iter()
                .copied()
                .filter(|at| u32_le(&bytes, *at + LOGIC_RECORD_SIZE_OFFSET) != 0)
                .collect();
            assert_eq!(empty, trak_empty, "{path}: Trak 空载荷条数");
            assert_eq!(filled.len(), trak_filled, "{path}: Trak 非 0 载荷条数");
            assert!(
                filled
                    .iter()
                    .all(|at| u32_le(&bytes, *at + LOGIC_RECORD_SIZE_OFFSET) == filled_len),
                "{path}: Trak 非 0 载荷长度必须恒为实测的 {filled_len}"
            );
            assert!(
                trak.iter().all(|at| u16_le(&bytes, at + 4) == kind),
                "{path}: Trak 记录头 kind（+0x04）必须恒为实测的 {kind}"
            );
            assert!(
                trak.iter().all(|at| u16_le(&bytes, at + 6) == 23),
                "{path}: Trak 记录头 subtype（+0x06）实测恒为 23"
            );

            assert_eq!(
                occurrences(&bytes, &FORWARD_KART),
                0,
                "{path}: 实测没有嵌套的 `karT`（正序四字节 0 次）"
            );
            assert_eq!(
                occurrences(&bytes, &STORED_TRACK_TAG),
                trak_total,
                "{path}: 落盘 `karT` 四字节的出现次数必须恰好等于 Trak 记录条数（没有第二处）"
            );

            for at in &filled {
                let len = u32_le(&bytes, *at + LOGIC_RECORD_SIZE_OFFSET) as usize;
                let body = &bytes[*at + LOGIC_RECORD_HEADER..*at + LOGIC_RECORD_HEADER + len];
                if let Some(name) = printable_name_shaped_field(body) {
                    panic!(
                        "{path}: 实测 Trak 载荷里**没有**名字字段，但在偏移 {at} 的载荷里读到 {name:?}"
                    );
                }
            }

            let meta = path.replace("ProjectData", "MetaData.plist");
            let meta_bytes = std::fs::read(&meta).expect("演示工程的 MetaData.plist 必须可读");
            let plist = read_bplist(&meta_bytes);
            assert_eq!(
                dict_of(&plist).get("NumberOfTracks"),
                Some(&TestPlist::Integer(plist_tracks as i64)),
                "{path}: MetaData.plist 的 NumberOfTracks"
            );
            assert!(
                (trak_filled as u64) > plist_tracks,
                "{path}: 非 0 Trak 条数（{trak_filled}）必须严格大于轨道数（{plist_tracks}）—— \
                 因为这一族不是一张表，而是记录头 +0x08 分出的三个子群"
            );

            // ---- 本轮语义答案的证据：按记录头 +0x08 的对象号分子群 ----
            let cluster_of = |at: usize| u32_le(&bytes, at + LOGIC_RECORD_CLUSTER_OFFSET);
            let body_first = |at: usize| bytes[at + LOGIC_RECORD_HEADER];
            let body_word8 = |at: usize| u32_le(&bytes, at + LOGIC_RECORD_HEADER + 8);
            let rank_at = |at: usize| bytes[at + 0x12];

            let arrange: Vec<usize> = filled
                .iter()
                .copied()
                .filter(|at| cluster_of(*at) == 0x0004_0000)
                .collect();
            let slots: Vec<usize> = filled
                .iter()
                .copied()
                .filter(|at| cluster_of(*at) == 0x0008_0000)
                .collect();
            assert_eq!(
                arrange.len(),
                arrange_filled,
                "{path}: `+0x08 == 0x00040000` 组（编排轨行）的非 0 载荷条数"
            );
            assert_eq!(
                slots.len(),
                slot_objects_filled,
                "{path}: `+0x08 == 0x00080000` 组（每个预分配混音槽一条）的非 0 载荷条数"
            );
            assert_eq!(
                arrange.len() as u64 - 1,
                plist_tracks,
                "{path}: 编排轨行非 0 条数 − 1 必须**恰好等于** MetaData.plist 的 NumberOfTracks\
                 （多出来的那条是 master 行）—— 这是本轮从 MIT 参考实现读到、在真实字节上复核的语义"
            );
            let masters: Vec<usize> = arrange
                .iter()
                .copied()
                .filter(|at| body_first(*at) == 3)
                .collect();
            assert_eq!(
                masters.len(),
                1,
                "{path}: 编排轨行里必须**恰好一条** master 行（载荷 +0x00 == 3）"
            );
            assert_eq!(
                body_word8(masters[0]),
                0x50,
                "{path}: master 行的载荷 +0x08 必须指向实测的通道槽 0x50"
            );
            assert!(
                arrange
                    .iter()
                    .filter(|at| body_first(**at) != 3)
                    .all(|at| body_first(*at) == 1),
                "{path}: 非 master 的编排轨行载荷 +0x00 实测恒为 1"
            );
            let mut ranks: Vec<u8> = arrange.iter().map(|at| rank_at(*at)).collect();
            ranks.sort_unstable();
            assert_eq!(
                ranks,
                (0..=plist_tracks as u8).collect::<Vec<u8>>(),
                "{path}: 编排轨行的记录头 +0x12 单字节序号必须恰好是 0..=NumberOfTracks"
            );
            let mut slot_numbers: Vec<u32> = slots.iter().map(|at| body_word8(*at)).collect();
            let distinct = {
                slot_numbers.sort_unstable();
                slot_numbers.dedup();
                slot_numbers.len()
            };
            assert_eq!(
                distinct,
                slots.len(),
                "{path}: `+0x08 == 0x00080000` 组的载荷 +0x08 槽号必须互不相同"
            );

            eprintln!(
                "可选 Trak 家族核对：{path} —— {trak_total} 条（{trak_empty} 空 + {trak_filled} 非 0，\
                 长 {filled_len}），kind {kind}，subtype 23，NumberOfTracks {plist_tracks}"
            );
        }
        if present == 0 {
            eprintln!("skip: 本机没有 Apple 演示工程（CI 上不存在 ⇒ 本判据不跑、绝不红）");
        }
    }

    /// 一段字节里 `needle` 作为子串出现的次数（用来证明"某四个字节在本族里出现多少次"）。
    fn occurrences(haystack: &[u8], needle: &[u8]) -> usize {
        if needle.is_empty() || haystack.len() < needle.len() {
            return 0;
        }
        haystack
            .windows(needle.len())
            .filter(|window| *window == needle)
            .count()
    }

    /// 实测定义下的"名字形态字段"：`u16` 小端长度（`2..=96`）+ 全可打印 ASCII + `\0`。
    ///
    /// 四份真实工程的 **919** 条非 0 `Trak` 载荷在这个定义下命中 **0**；而 `MSeq` 的名字字段
    /// （载荷 `+0x10`/`+0x12`，见 [`LOGIC_REGION_NAME_PAYLOAD_OFFSET`]）会命中 —— 因此这个扫描器
    /// 能分辨"有名字"和"没名字"，不是恒假条件。
    fn printable_name_shaped_field(body: &[u8]) -> Option<String> {
        if body.len() < 3 {
            return None;
        }
        for at in 0..=body.len() - 3 {
            let length = usize::from(u16_le(body, at));
            if !(2..=96).contains(&length) || at + 2 + length > body.len() {
                continue;
            }
            if body[at + 2 + length - 1] != 0 {
                continue;
            }
            let text = &body[at + 2..at + 2 + length - 1];
            if text.is_empty() || !text.iter().all(|byte| (0x20..=0x7e).contains(byte)) {
                continue;
            }
            return Some(String::from_utf8_lossy(text).into_owned());
        }
        None
    }

    /* -------------------------------------------------------------- *
     * 供体（donor）路线
     * -------------------------------------------------------------- */

    /// 按 36 字节记录头的载荷长度把整个 chunk 流切成**整条记录**的切片。
    fn record_slices(bytes: &[u8]) -> Vec<&[u8]> {
        let mut out = Vec::new();
        let mut at = LOGIC_ROOT_HEADER;
        while at + LOGIC_RECORD_HEADER <= bytes.len() {
            let size = u32_le(bytes, at + LOGIC_RECORD_SIZE_OFFSET) as usize;
            assert!(
                at + LOGIC_RECORD_HEADER + size <= bytes.len(),
                "记录越过 EOF（偏移 {at:#x}）"
            );
            out.push(&bytes[at..at + LOGIC_RECORD_HEADER + size]);
            at += LOGIC_RECORD_HEADER + size;
        }
        assert_eq!(at, bytes.len(), "记录流必须恰好铺满文件");
        out
    }

    /// 一条记录的可读 tag（把落盘的四个字节反序）。
    fn readable_tag(record: &[u8]) -> String {
        let mut bytes = [record[0], record[1], record[2], record[3]];
        bytes.reverse();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// 供体路线**允许**与供体不同的那 4 条记录的下标。
    ///
    /// 用与实现**相同的规则**重新定位（tag + 对象号 + 载荷首字节），因此它不是把实现的答案抄一遍：
    /// 实现改了规则、却没改这 4 条，判据就会红。
    fn patched_record_indices(records: &[&[u8]]) -> Vec<usize> {
        let mut out = Vec::new();
        for (index, record) in records.iter().enumerate() {
            let tag = [record[0], record[1], record[2], record[3]];
            let cluster = u32_le(record, LOGIC_RECORD_CLUSTER_OFFSET);
            let first = record.get(LOGIC_RECORD_HEADER).copied();
            let meter = tag == LOGIC_SEQUENCE_TAG
                && cluster == 0
                && first == Some(LOGIC_METER_MARKER as u8);
            let tempo = tag == LOGIC_SEQUENCE_TAG
                && cluster == 0
                && first == Some(LOGIC_TEMPO_MARKER as u8);
            let region = tag == LOGIC_REGION_TAG && cluster == LOGIC_DONOR_REGION_CLUSTER;
            let notes = tag == LOGIC_SEQUENCE_TAG && cluster == LOGIC_DONOR_REGION_CLUSTER;
            if meter || tempo || region || notes {
                out.push(index);
            }
        }
        out
    }

    /// **判据（有牙）**：产物里供体的通道簇逐字节原样，且**只有** 4 条记录与供体不同。
    #[test]
    fn donor_cluster_is_carried_byte_for_byte_and_only_four_records_change() {
        let donor = record_slices(LOGIC_DONOR_PROJECT_DATA);
        let data = project_data_from_donor(&fixture_project());
        let output = record_slices(&data.bytes);

        assert_eq!(donor.len(), LOGIC_DONOR_RECORD_COUNT, "供体的实测记录条数");
        assert_eq!(
            output.len(),
            LOGIC_DONOR_RECORD_COUNT,
            "供体路线必须保留供体的记录条数"
        );
        assert_eq!(
            u16_le(&data.bytes, LOGIC_FORMAT_VERSION_OFFSET),
            LOGIC_DONOR_VERSION_CODE,
            "供体路线保留供体自己的根头版本码"
        );

        let patched = patched_record_indices(&donor);
        assert_eq!(
            patched.len(),
            LOGIC_DONOR_PATCHED_RECORDS,
            "只允许这 4 条记录不同，实测定位到 {patched:?}"
        );
        let changed: Vec<usize> = (0..donor.len())
            .filter(|index| donor[*index] != output[*index])
            .collect();
        assert_eq!(changed, patched, "**只有**这 4 条记录可以与供体不同");

        // 逐族：供体携带的通道簇 / 环境 / 轨道表在产物里**逐字节相同**。
        let mut seen: Vec<(&str, usize)> = Vec::new();
        for name in [
            "Envi", "AuCO", "GenM", "Trak", "InSt", "Layr", "ScSt", "SngO", "CorM", "Hypr", "Styl",
            "TxSt", "AuCn", "AuCU", "Vide",
        ] {
            let donor_rows: Vec<&[u8]> = donor
                .iter()
                .copied()
                .filter(|record| readable_tag(record) == name)
                .collect();
            let output_rows: Vec<&[u8]> = output
                .iter()
                .copied()
                .filter(|record| readable_tag(record) == name)
                .collect();
            assert_eq!(donor_rows.len(), output_rows.len(), "家族 `{name}` 的条数");
            for (left, right) in donor_rows.iter().zip(output_rows.iter()) {
                assert_eq!(left, right, "家族 `{name}` 必须逐字节等于供体");
            }
            seen.push((name, donor_rows.len()));
        }
        for (name, count) in LOGIC_DONOR_CHUNK_FAMILIES {
            let measured = seen
                .iter()
                .find(|(family, _)| *family == name)
                .map(|(_, count)| *count);
            if measured.is_some() {
                assert_eq!(measured, Some(count), "家族 `{name}` 的实测条数");
            }
        }
        // 通道簇的三族必须真的非零（否则"携带了供体"是空话）。
        assert!(seen.contains(&("AuCO", 376)), "OCuA 混音条 376 条");
        assert!(seen.contains(&("Envi", 12)), "ivnE 环境对象 12 条");
        assert!(seen.contains(&("GenM", 1)), "MneG 1 条");
        assert!(seen.contains(&("Trak", 22)), "karT 轨道家族 22 条");
    }

    /// **判据（有牙）**：我们的拍号 / 速度 / region 名 / 音符真的出现在产物里。
    #[test]
    fn donor_splice_writes_our_tempo_meter_region_name_and_notes() {
        let project = fixture_project();
        assert_eq!(
            project.tracks.len(),
            LOGIC_DONOR_TRACK_COUNT,
            "夹具必须与供体的编排轨容量一致，否则本条判据问的不是同一个问题"
        );
        let data = project_data_from_donor(&project);
        let records = read_records(&data.bytes);

        let tempo = records
            .iter()
            .find(|record| {
                record.tag == LOGIC_SEQUENCE_TAG
                    && record.cluster == 0
                    && u32_le(&record.body, 0) == LOGIC_TEMPO_MARKER
            })
            .expect("速度 qSvE");
        assert_eq!(
            u32_le(&tempo.body, LOGIC_TEMPO_VALUE_OFFSET),
            1_280_000,
            "速度字必须是 round(128 × 10000)"
        );
        assert_eq!(read_meter(&records), Some((3, 4)), "拍号必须是 3/4");

        let region = records
            .iter()
            .find(|record| {
                record.tag == LOGIC_REGION_TAG && record.cluster == LOGIC_DONOR_REGION_CLUSTER
            })
            .expect("被摆放的 region qeSM");
        assert_eq!(read_region_name(region), "Piano", "region 名必须是我们的");

        let notes = records
            .iter()
            .find(|record| {
                record.tag == LOGIC_SEQUENCE_TAG && record.cluster == LOGIC_DONOR_REGION_CLUSTER
            })
            .expect("region 的音符 qSvE");
        assert_eq!(notes.body.len(), 3 * 32 + 16, "载荷 = 32·N + 16");
        assert_eq!(
            decode_note_lines(&notes.body),
            vec![
                (3_840, 480, 60, 100),
                (4_800, 480, 64, 90),
                (5_760, 480, 67, 80)
            ],
            "三个音符必须逐字段读回（起始/时值/音高/力度）"
        );
        assert_eq!(data.mapped_regions, 1);
        assert_eq!(data.mapped_notes, 3);

        // `MetaData.plist` 的 `NumberOfTracks` = **实际映射**的轨道数（不是工程轨道总数）。
        let bundle = build_bundle_from_donor(&project, "000", "Fixture");
        let meta = read_bplist(
            bundle
                .files
                .get("Alternatives/000/MetaData.plist")
                .expect("MetaData.plist"),
        );
        assert_eq!(
            dict_of(&meta).get("NumberOfTracks"),
            Some(&TestPlist::Integer(1)),
            "NumberOfTracks 必须是映射数"
        );
    }

    /// **判据（有牙）**：供体路线也逐字节确定性（两次导出相同）。
    #[test]
    fn donor_splice_is_byte_deterministic() {
        let project = fixture_project();
        let first = project_data_from_donor(&project);
        let second = project_data_from_donor(&project);
        assert_eq!(
            first.bytes, second.bytes,
            "两次 `ProjectData` 必须逐字节相同"
        );
        assert_eq!(first.losses, second.losses, "损失表必须相同");
        let left = build_bundle_from_donor(&project, "000", "Fixture");
        let right = build_bundle_from_donor(&project, "000", "Fixture");
        assert_eq!(left.files, right.files, "四个文件必须逐字节相同");
    }

    /// **判据（有牙）**：仓库里的供体与 `include_bytes!` 的字节完全相同，且许可与出处随文件走。
    #[test]
    fn vendored_donor_matches_the_embedded_bytes_and_carries_its_licence() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/logic-donor");
        let on_disk =
            std::fs::read(base.join("Alternatives/000/ProjectData")).expect("仓库里的供体文件");
        assert_eq!(on_disk.len(), LOGIC_DONOR_PROJECT_DATA_BYTES, "供体字节数");
        assert_eq!(
            on_disk, LOGIC_DONOR_PROJECT_DATA,
            "仓库里的供体必须与 include_bytes! 的逐字节相同"
        );
        let licence = std::fs::read_to_string(base.join("LICENSE")).expect("上游 LICENSE");
        assert!(licence.contains("MIT License"), "上游许可是 MIT 全文");
        assert!(
            licence.contains("Copyright (c) 2026 Jon Kubis"),
            "必须保留上游版权行"
        );
        let note = std::fs::read_to_string(base.join("README.md")).expect("出处说明");
        assert!(
            note.contains("jonkubis/logicproformatwriter"),
            "必须点名上游仓库"
        );
        assert!(note.contains(LOGIC_DONOR_SHA256), "必须写下供体的 sha256");
        assert!(note.contains("MIT"), "必须写明许可");
    }

    /// **判据（有牙）**：供体路线的损失表**逐族**登记了哪些记录仍是供体的。
    #[test]
    fn donor_losses_register_what_remains_the_donors() {
        let data = project_data_from_donor(&fixture_project());
        let text = data
            .losses
            .iter()
            .map(|loss| format!("{}: {}", loss.entity, loss.reason))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains(LOGIC_DONOR_SHA256), "必须点名供体的 sha256");
        assert!(
            text.contains(&LOGIC_DONOR_PROJECT_DATA_BYTES.to_string()),
            "必须写下供体的字节数"
        );
        assert!(
            text.contains(&format!("{:#06x}", LOGIC_DONOR_VERSION_CODE)),
            "必须写下供体的根头版本码"
        );
        for (name, count) in LOGIC_DONOR_CHUNK_FAMILIES {
            assert!(text.contains(&format!("`{name}`")), "逐族点名 `{name}`");
            assert!(
                text.contains(&format!("**原样携带** {count} 条")),
                "家族 `{name}` 必须写出实测条数 {count}"
            );
        }
        assert!(text.contains("Inst 1"), "必须点名供体携带的那条轨道");
        assert!(
            text.contains("1,200,000"),
            "必须点名 `gnoS` 里没改的供体速度"
        );
        assert!(
            data.losses
                .iter()
                .any(|loss| loss.reason.starts_with(LOSS_NOT_EQUIVALENT_PREFIX)),
            "`非等价:` 必须出现"
        );
        assert!(
            data.losses
                .iter()
                .any(|loss| loss.reason.starts_with(LOSS_UNMAPPED_PREFIX)),
            "`未映射:` 必须出现"
        );
        assert!(
            !text.contains("没有写入"),
            "供体路线不得再声称这些 chunk 家族没有写入"
        );
    }

    /// **判据（有牙）**：供体只有 1 条编排轨行时，多出来的轨道必须被点名，且 `NumberOfTracks`
    /// 不许被抬到工程轨道总数。
    #[test]
    fn donor_splice_registers_tracks_it_cannot_carry() {
        let mut project = fixture_project();
        let second = TrackV3 {
            id: test_id(7),
            name: "Second".to_owned(),
            ..TrackV3::default()
        };
        project.tracks.insert(second.id, second);

        let data = project_data_from_donor(&project);
        let text = data
            .losses
            .iter()
            .map(|loss| format!("{}: {}", loss.entity, loss.reason))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Second"), "第二条轨道必须被点名");
        assert!(text.contains("没有可供体插槽"), "必须说清为什么它不导出");

        let bundle = build_bundle_from_donor(&project, "000", "Fixture");
        let meta = read_bplist(
            bundle
                .files
                .get("Alternatives/000/MetaData.plist")
                .expect("MetaData.plist"),
        );
        assert_eq!(
            dict_of(&meta).get("NumberOfTracks"),
            Some(&TestPlist::Integer(1)),
            "NumberOfTracks 必须是映射数 1，而不是工程轨道总数 2"
        );
    }
}
