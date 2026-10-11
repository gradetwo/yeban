# ADR-0006 — 物理 MIDI 输入接入（`midir`）

- **状态**: **`Proposed`**（⭐ 依 `AGENTS.md` §5.7：Agent ⛔ 不得私自引入依赖，也不得擅自改写 Normative 文档 ⇒ 本裁决**待人类批准**后方可落地；⭐ 批准前**不得**改 `Cargo.toml`／`Cargo.lock`）
- **日期**: 2026-10-11
- **依据**: `AGENTS.md` §2 红线 2（依赖许可）、红线 6（发行特性）、红线 7（实时音频）、红线 8（`forbid(unsafe_code)`）；§5.7（规范缺口与冲突）；`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md`（录音与外部协同族 `ARCH-REC-*`／`ARCH-EXT-*`）；`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` **RSK-22**；`deny.toml` `[licenses].allow`；`docs/ledger/dependency-licenses.md`
- **背景**: ⭐ 四个模块的接入已完成（① SFZ ② 导出音频 ③ MIDI 输入 ④ 录音，⭐ 均有 CLI 入口 ＋ 真读数 ＋ 台账 R307–R314）⚠ **但 ③ 只做到"**无新依赖的那一层**"**：⭐ 现状是**从 `.mid` 文件读入并把每个 op 标为 `OpOrigin::MidiInput`**（⭐ 可脚本化、⭐ 可判据化）⇒ ⭐ **⛔ 尚未接硬件**。⭐ 而 `RSK-22` 点名：⭐ 「操作系统定时器精度不足导致物理 MIDI 输入时钟抖动」⇒ ⭐ 手段 = **硬件时间戳（`midir` 原始时间戳）换算至最近 960 PPQ Tick**，⭐ 验收线 = **「MIDI 录音音符时序抖动 > 5 Ticks」为不可接受**。⭐ 本 ADR 把"是否引入 `midir`、⭐ 引到哪、⭐ 怎么保证红线"一次写清，⭐ ⛔ 避免工作线各自发明。

---

## 裁决一览

| # | 问题 | 裁决 | 依据与代价 |
| :-- | :--- | :--- | :--- |
| D1 | 是否引入硬件 MIDI 输入库 | **引入 `midir`（钉 `0.11.1`）**，⭐ 写进根 `Cargo.toml` 的 `[workspace.dependencies]`，⭐ 成员一律 `midir.workspace = true` | 许可证 **MIT** ⇒ 已在 `deny.toml` `allow` 内 ✓（⭐ 许可证、版本、依赖树见"实测事实"节）。**代价**：新增一条平台 FFI 边界依赖 ⇒ 增加 CI 编译面与审计面 |
| D2 | 宿主 crate（⭐ 放哪） | **新建 `crates/yeban-midi-io`**（⭐ 或经批准后并入既有 `yeban-midi`），⭐ ⛔ **不得放进 `yeban-model`／`yeban-dsp`／`yeban-theory`／`yeban-render`**（红线 8 的 `forbid(unsafe_code)` 面） | 红线 3 禁的是 GUI 依赖（`midir` ⛔ 不是 GUI）；⭐ 红线 8 允许 `unsafe` **仅**出现在"已审计的底层驱动与 FFI 边界" ⇒ ⭐ **独立 crate 才能把审计面收在一处** |
| D3 | 实时安全（红线 7） | ⭐ **`midir` 回调 = OS 的 MIDI 线程（⭐ 不是我们的音频线程）**，⭐ 但**同样按红线 7 对待**：⭐ 回调内 **⛔ 零分配、⛔ 零 I/O、⛔ 不加锁** ⇒ ⭐ **只把定长事件写进构造期预分配的无锁 SPSC 环**，⭐ 由**控制侧**线程消费 | ⭐ 本仓**已有**该范式：⭐ `crates/yeban-engine/src/ring.rs` 的 `event_channel(capacity)`（⭐ 建在 `rtrb` 上）与 `crates/yeban-engine/src/record.rs` 的 `record_channel(...)`（⭐ 近期刚修过"环满留半帧"⇒ **整帧原子**）⇒ ⭐ **复用，⛔ 不另造** |
| D4 | 时间戳 → Tick 的口径 | ⭐ **必须用 `midir` 的原始时间戳**（⭐ ⛔ 不启用 `avoid_timestamping`），⭐ 换算到 **960 PPQ**；⭐ **换算基准（⭐ 会话起点）必须显式定义并记录**，⭐ ⛔ 不得隐式用"收到回调的时刻" | `RSK-22` 的手段与验收线（**抖动 > 5 Ticks 不可接受**）。⚠ **本 ADR 不规定换算实现的细节**（⭐ 属实现期），⭐ 但**要求实现时给出可判定的抖动读数** |
| D5 | 发行特性门（红线 6） | ⭐ 新增 feature（⭐ 建议 `midi-input`）⭐ **⛔ 不得进 default**；⭐ **release 默认构建不得启用** | 与 `mcp-http`／`ui-mcp`／`asio`／`experimental-*` 同族对待（红线 6 逐字列出这些 ⇒ 精神一致） |
| D6 | 平台后端 | ⭐ 跟随 `midir` 默认：⭐ **macOS `coremidi`／Linux `alsa`／Windows `windows`+`parking_lot`**；⭐ ⛔ **不启用 `jack` feature**（⭐ 会让 `jack-sys` 进树 ＋ 需系统 JACK） | ⭐ `jack` 属**可选** ⇒ ⭐ 不开即不进树（⭐ 见"实测事实"节） |
| D7 | 新增依赖的许可证 | ⭐ **唯一新增的运行时依赖是 `coremidi`（MIT）**；⭐ 其余平台依赖**已在本仓清单中**（⭐ 均为 MIT／Apache-2.0 系） | 逐项实测见下节；⭐ `env_logger` 是 `midir` **自身的 dev-dependency**（⭐ 不进我们的依赖树）；⭐ `jack-sys`／`bitflags ^2` 仅 `jack` feature 需要（⭐ D6 已排除） |
| D8 | 未过 `cargo deny` 前可否先行接线 | ⛔ **不可以**。⭐ 本 ADR 批准后仍需：⭐ 改 `Cargo.toml` ⇒ ⭐ 重生成 `docs/ledger/dependency-licenses.md` ⇒ ⭐ **CI 的 `deny` 作业绿** ⇒ ⭐ 方可继续 | `AGENTS.md` §2 红线 2（"严禁引入未通过 `cargo deny check` 的依赖"）＋ §5.5（**只有 CI 判决算数**） |

---

## 实测事实（⭐ 本 ADR 写前的核验，⭐ 逐项可复现）

⭐ **核验对象**：`midir` **0.11.1**（⭐ `crates.io` 元数据；⭐ 发布于 2026-10-10）；⭐ 其依赖树取自 `crates.io` 的 `…/midir/0.11.1/dependencies`。

| 事实 | 读数 |
| :--- | :--- |
| `midir` 许可证 | **MIT** ⇒ ⭐ 在 `deny.toml` `[licenses].allow` 内 ✓ |
| `midir` 规模 | 4024 行 Rust ／ 17 文件 ⇒ ⭐ 小 |
| `midir` 默认特性 | `default = []`（⭐ 后端按目标平台自动带入） |
| 可选特性 | `jack`（⭐ 拉 `jack-sys`／`libc`／`bitflags ^2`）、`winrt`、`avoid_timestamping`、`coremidi_send_timestamped` |
| macOS 必需 | **`coremidi ^0.9.0`** ⇒ ⭐ 实测 `coremidi` 0.9.2 许可证 = **MIT**（⭐ 2807 行／16 文件、⭐ 无 features、⭐ `rust_version 1.75`） |
| Windows 必需 | `windows >=0.59.0, <=0.62` ＋ `parking_lot ^0.12.1` |
| Linux 必需 | `alsa >=0.9.0, <0.13` ＋ `libc ^0.2.21` |
| 恒定额外 | `log ^0.4` |
| ⭐ 已在 `docs/ledger/dependency-licenses.md` 中 | `alsa 0.11.0`（Apache-2.0/MIT）、`parking_lot 0.12.5`、`windows 0.62.2`、`log 0.4.34`、`libc 0.2.190`、`bitflags 1.3.2`、`cc 1.6.0`、`jni 0.22.4`、`ndk-context 0.1.1`、`web-sys 0.3.106`、`wasm-bindgen 0.2.129`、`js-sys 0.3.106` ⇒ ⭐ **均已是本仓传递依赖** ✓ |
| ⭐ 清单中**未见**的 | `coremidi`（⭐ 即 **D7 的唯一新增**）、`env_logger`（⭐ 仅 midir 的 dev-dep ⇒ ⛔ 不进树）、`jack-sys`（⭐ 仅 `jack` feature ⇒ ⛔ 不开） |

---

## 验收判据（⭐ 落地时必须给出，⭐ 本 ADR 先立口径）

1. ⭐ ⭐ **可判定的抖动读数**：⭐ 对 `RSK-22` 的验收线（⭐ **> 5 Ticks 不可接受**）给出**本机实测**：⭐ 例：⭐ 同一硬件输入的 N 个事件相对 960 PPQ 网格的偏差分布（⭐ 报 **p50／p99／max**），⭐ ⭐ **并写明**它数的是什么**（⭐ 对象 ＋ 单位 ＋ 基准从哪来）**。
2. ⭐ **回调零分配**：⭐ 沿用／扩展 `rt.rs`／`MUST-GATE-001` 的**既有零分配判据口径**（⭐ ⛔ 不自创一套），⭐ 并说明**怎么检测**的。
3. ⭐ **无硬件时的行为**：⭐ 无端口／端口断开／权限不足 ⇒ ⭐ **报结构化错误而⛔ 不 panic**（⭐ ⛔ 不阻塞启动）。
4. ⭐ **离线可判定性**：⭐ 硬件相关部分 ⛔ **本机 CI 无法覆盖** ⇒ ⭐ 必须有一条**不依赖真硬件**的判据（⭐ 例：⭐ 事件 → Tick 换算的纯函数判据 ＋ ⭐ 环的语义判据），⭐ ⭐ **并如实登记"真硬件路径在本仓 CI 上不可验"**（⭐ ⛔ 不假装验过）。
5. ⭐ **许可证闭环**：⭐ `cargo deny check` 绿 ＋ ⭐ `docs/ledger/dependency-licenses.md` 已重生成（⭐ `license_inventory.py`，⭐ ⛔ 不带 `--check`）＋ ⭐ 该清单漂移检查（`checks` 作业）绿。

## 代价与风险

- ⚠ **新增 FFI 审计面**：⭐ `midir` ＋ `coremidi` 内含 `unsafe`（⭐ 那是**依赖内部**，⭐ 红线 8 约束的是**我们的** crate）⇒ ⭐ 我方代码仍需**独立 crate** 承载（D2），⭐ 且若我方言及 `unsafe` ⇒ ⭐ **必须附 `// SAFETY:` 证明**。
- ⚠ **平台差异**：⭐ Linux 走 `alsa`（⭐ 需系统 ALSA 开发库）；⭐ 若 CI 的 Linux 腿缺该库 ⇒ ⭐ **需把 feature 置于 CI 默认之外**（⭐ 见 D5），⭐ 并**在 CI 上显式验证一次该 feature 能编**（⭐ 否则"引入即绿"是假绿）。
- ⚠ **`RSK-22` 的 5 Ticks 是硬线**：⭐ 若实测抖动超线 ⇒ ⭐ 依 §6.4 纪律（"参考文档的警告是证据"），⭐ **不得把超线读数写成"接近达标"**，⭐ 而应**登记为阻塞**并回到裁决。
- ⚠ **本 ADR 之外的依赖**：⭐ 若实现期发现还需别的库（⭐ 例：⭐ USB-MIDI 直驱、⭐ 虚拟端口）⇒ ⭐ **另立 ADR**，⭐ ⛔ 不在本 ADR 内扩张。

## 待人类批准的事项

1. ⭐ **是否批准引入 `midir` 0.11.1**（⭐ D1）＋ **是否为它新建 `crates/yeban-midi-io`**（⭐ D2）。
2. ⭐ **feature 名与默认开关**（⭐ 建议 `midi-input`，⭐ 默认关）（⭐ D5）。
3. ⭐ **`RSK-22` 的抖动验收如何量化入库**（⭐ p50／p99／max ＋ 样本量）（⭐ 验收判据 1）。
