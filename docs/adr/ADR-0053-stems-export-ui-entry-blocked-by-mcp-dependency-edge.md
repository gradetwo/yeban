# ADR-0053 — 界面上的「导出分轨」入口⛔ 接不了：它唯一诚实的落点 `yeban_mcp::domain::stems` 在**默认发行构建里不存在**（MUST-GATE-009 的第一道开关把它连同整张依赖图一起关掉），而"再做一份分轨实现"是第二个真相源

- **状态**: **Proposed**（待人类批准；⛔ 集成者代签。本票**不改任何代码、不改任何 `.slint`、不重录任何基准**，
  只把「导出分轨的 GUI 入口」这一票的**架构阻塞**与其代价写成可批可驳的选项）
- **决定者**: 待人类裁决（集成者提出）
- **关联**: `ADR-0030`（分轨导出的**工具面**裁决，本票⛔ 推翻它、只补它没写的那一半——**界面**入口）、
  `ADR-0001` **D47**（导出的唯一出口＝ CLI/离线语义）、**D51**（工作区**内部**依赖边可以新增，但生成物必须机械重生成、
  同时只允许一条线碰根清单）、`ADR-0005`（GUI 进程内⛔ 第二份权威）、`ADR-0011`（`yeban-render`→`yeban-engine`
  那条"为了共享音源而开的内部边"先例）、`ADR-0052`（三个"画了但没有任何动作"的控件怎么处置——本票若走 `D3` 会**新增第四名**）、
  `AGENTS.md` §2 **红线 6** 与 `MUST-GATE-009`（发行特性默认关）、`ARCH-DSP-003`（分轨＝ 32-bit Float 的口径来源）、
  `UI-A11Y-003`（屏读动作与点击源同落点）、`UI-MCP-003`（分平台 Golden；本票的候选座位**零像素改动**，理由见下）
- **实现落点**: 取决于批哪一条 —— `D1` ⇒ `crates/yeban-app/Cargo.toml` ＋ **根 `Cargo.toml`/`Cargo.lock`** ＋
  `.github/workflows/release.yml:402-445` 那条 MUST-GATE-009 断言 ＋ `docs/ledger/dependency-licenses.md`；
  `D2` ⇒ **新成员 crate**（根 `Cargo.toml`）＋ `crates/yeban-mcp/src/domain/stems.rs` 拆分 ＋ `crates/yeban-app/src/host.rs`；
  `D3` ⇒ `crates/yeban-app/ui/**` ＋ `src/elements.rs` 的 `KNOWN_ACTIONLESS_CONTROLS`；
  `D4` ⇒ `crates/yeban-app/src/`（第二份分轨实现）；`D5` ⇒ **零代码**，只改 `docs/ledger/feature-alignment.md:161` 那格的措辞。

## 背景（先测后改，`AGENTS.md` §6）

**这个数字在数什么**：单位＝**个包**，对象＝`yeban-mcp` 的 `[dependencies]` 里**尚未**出现在
`yeban-app` 的 `[dependencies]` 中的那些**内部 crate**（用于估 `D1` 会把多少东西带进默认发行构建）。
⚠ 这条**不是** `cargo tree` 的实测读数 —— 本机 `cargo` 今天跑不动（见下面「本票⛔ 本地实测的部分」），
所以它是**清单级**（Cargo.toml 逐条对读）的上界，真实包数增量留给 CI 的 `graph_metric` 读回。

现读（2026-10-31，`master` = `0189d9b3`）：

| 事实 | 现读证据 |
| --- | --- |
| 分轨的**唯一**实现住在 `yeban-mcp` 的领域层（`validate` / `export` / `write` / `stem_file_name`） | `crates/yeban-mcp/src/domain/stems.rs`（**764** 行）：`validate` `:87`、`export` `:390`、`write` `:521`、`stem_file_name` `:689` |
| `yeban-app` 只有在**非默认 feature** 下才看得见 `yeban-mcp` | `crates/yeban-app/Cargo.toml:189` `yeban-mcp = { path = "../yeban-mcp", optional = true }`；`:259` `in-process-mcp = ["dep:yeban-mcp", "yeban-mcp/mcp-http"]`；`src/lib.rs:176-185` 把 `mcp_mount` / `reproject` 两个模块整块 `#[cfg(feature = "in-process-mcp")]` 关掉 |
| 这条开关是**机械钉住的**，不是文档愿望 | `.github/workflows/release.yml:402` 那一步（`MUST-GATE-009 证据 ① 依赖图指标`）在 `:434` 断言 `[ "$DEFAULT_MCP" = "0" ]`，失败文案逐字是「默认依赖图里出现了 yeban-mcp ⇒ `in-process-mcp` 被编进了发行物」；`:437-438` 还带**正对照**（打开 feature 后命中必须 ≥1、图指标必须变大）⇒ "0 命中"不可能是 grep 写错 |
| 台账已经把这一格登记为「**有意**不做」 | `docs/ledger/feature-alignment.md:161` 的系统列逐字：「无｜GUI 无导出控件（**有意**：`ADR-0001` D47 把导出定为 CLI/离线语义）」，同一行的 `计划` 列写着「UI 侧的导出对话框仍属后续切片」 |
| 本仓库对「GUI 要用 MCP 侧逻辑」已有**三个**先例，且**没有**一个走"加默认依赖" | ① 共享**叶子 crate**：`yeban-diagnostics` 被 `crates/yeban-app/Cargo.toml:125` 与 `crates/yeban-mcp/Cargo.toml:35` **同时**依赖，GUI 侧调用点 `src/undo.rs:622`（`yeban_diagnostics::export_diagnostics`）；② `#[path]` **字面同一份源码**：`src/undo.rs:50` `#[path = "../../yeban-mcp/src/undo_session.rs"]`、`src/project_lock.rs:17`（锁）；③ **内部依赖边**：`ADR-0001` **D51**（`line/app-export-midi` 为 `--export-midi` 开的 `yeban-app → yeban-render` 边） |
| 分轨那套逻辑**搬不动**（这是阻塞的根因） | `crates/yeban-mcp/src/domain/stems.rs:42-44` 只 import 四样：`super::error::Fault`、`super::{render, store}`、`crate::tools::ErrorCode`。其中 `domain/render.rs` = **5539** 行、`domain/store.rs` = **1287** 行、`tools.rs` = **2632** 行 —— 而 `export` 真正需要的 `render::build` / `guard_output_path` / `write_stem_container` 与 `store::write_project_atomic`、`Fault` 的**故障词汇表**（`crates/yeban-mcp/tests/fault_vocabulary.rs` 的黄金表）全在那三个文件里 |
| GUI 侧**已有**一条同族的「导出」入口族，座位纪律可复用 | `src/host.rs:6916` `wire_module_actions`（同一面板的 `export-wav` / `export-midi` / `import-midi` / `load-sfz` / `record` / `midi-in` 等），回执唯一写入点 `apply_module_receipt` ⇒ `ui` 的 `module-status` 通道（`src/host.rs:6849-6850`） |

## 这一票当时是怎么打算的（登记，因为 `D1`/`D2` 一旦批准可以直接接着做）

已经想清楚并**写在纸面上**、但一行代码都⛔ 没提交的部分：

1. **座位**：`MixerConsole`（控制台 **Tab 1**）「工程导出」面板里，与 `#90` 甲那颗「导出 MIDI」**同一行并排**——
   面板内宽 `216 - 2×space-3 = 204px` ⇒ 两枚各 `99px` ＋ 一个 `6px` 间隙 ＝ `204px`，
   因此面板的高/宽/`x`/`y` 与 `#186` 的窄档算术**一位不动**（`tests/pane_no_overlap.rs` 登记的坐标不变）。
   ⇒ `MixerConsole` 在默认帧（Tab 0）**一个像素都不画** ⇒ `[UI-MCP-003]` 的 **5** 张 Linux 基准逐位不变 ⇒
   本票**不需要** `gate=goldens` 重录（这与 `ADR-0052` 里"任何用户可见处置都要重录 5 张"的代价**不同**，值得单独指出）。
2. **线程形态**：`N + 1` 次渲染⛔ 不在 UI 线程上跑（否则界面卡死，与 `midi-in` 那票同一个用户可见缺陷）。
   按下先**同步**写 `outcome=pending` 进 `module-status`，渲染在工作线程，终态经一条
   `stems-drain-requested` 回调回到 UI 线程落界面 —— 逐字复用 `src/host.rs:7384-7440` 与
   `:3011/:3066` 那两处已有的 `*-drain-requested` 跨线程形状。
3. **落点**：`host::wire_stems_export` ⇒ `yeban_mcp::domain::stems::{validate, export, write}`，
   目录由宿主给（`YEBAN_UI_EXPORT_STEMS_DIR`，缺省 `ui-stems`），回执字段与工具面对齐（`stems[]` / `skippedTracks[]` / `stemsBasis.renders`）。
4. **⛔ 不另写分轨渲染器**：这一条是本票的硬要求，也正是它把自己逼进了下面的死路。

工作区里那份未提交的 `.slint` 改动（三文件 ＋88 行：`ui/app.slint` / `ui/console/console_tabs.slint` /
`ui/console/mixer_console.slint` 的 `export-stems` 回调链与那颗按钮）已**回退**，补丁留在
`/tmp/yeban-stems-ui-wip/slint-export-stems.patch`（**易失**——它只是"当时的形状"，⛔ 交付物；
批准 `D1`/`D2` 后按上面 1–3 重写即可）。回退的理由与 `D3` 有关，见下。

## 本票⛔ 本地实测的部分（如实登记）

- `cargo tree -p yeban-app -e normal --locked | grep -c yeban-mcp` 这个**判决性读数**今天在本机**没能亲自跑**：
  `cargo` 本身起不来 —— `error: could not download nonexistent rust version 1.99.0-aarch64-unknown-linux-gnu`
  （镜像 `https://mirrors.aliyun.com/rustup/...` 返回 **404**）。
- 因此本票的阻塞结论建立在**源码级**证据上（`optional = true` ＋ 整块 `#[cfg(feature)]` ＋
  `release.yml:434` 那条断言），这三样**不需要编译**就足以判定"默认构建里 `yeban_mcp` 这个名字不存在"。
- `D1` 的真实依赖图增量（几个包、`Cargo.lock` 是否出现**新 package 段落**）**必须**由 CI 的
  `graph_metric` / `crate_hits` 与 `cargo deny` 读回，本票一律记 **pending**，不写成"通过"。

## 为什么⛔ 一行接线就能补上这个入口（三条，全部现读）

1. **默认构建里没有那个函数**。`crates/yeban-app/src/host.rs` 若要 `use yeban_mcp::domain::stems`，
   `cargo build -p yeban-app`（默认 feature）在**名字解析**这一步就失败 —— 不是链接期、不是运行期降级，
   是**编不出来**。仓库里已有的三处 `use yeban_mcp::…`（`src/undo.rs:58-63`、`src/reproject.rs`、`src/mcp_mount.rs`）
   **全部**站在 `#[cfg(feature = "in-process-mcp")]` 后面，正是同一件事的反面证明。
2. **"按钮先画出来、以后再接"在这个仓库里是红线**。`src/elements.rs:4208` 的 `KNOWN_ACTIONLESS_CONTROLS`
   与常驻判据 `controls_without_any_click_source_are_exactly_the_known_empty_ones` 钉的是"有交互角色却没有点击源"的集合；
   本票那颗按钮**有** `clicked` 与 `accessible-action-default`，只是终端的 `MainWindow.export-stems`
   **没有任何宿主 handler** ⇒ 它能**骗过**那条判据，却是一个真的按下去没反应的键。
   这正是 `ADR-0052` 花整篇登记的那类"假控件"，只是更隐蔽。⇒ 当时的 `.slint` 改动**必须**回退（已回退）。
3. **做第二份实现＝第二个真相源**。界面若自己实现"选轨/命名/防撞/`N+1` 帧数钉死/跳过语义"，
   同一个工程经 `yeban_export_stems` 与经那颗按钮会得到**不同文件名或不同跳过读数**，
   而 `ADR-0030` 的命名与豁免规则已经被 `crates/yeban-mcp/tests/export_stems.rs`（十条）与
   `fault_vocabulary.rs` 的黄金表钉住。这与 `ADR-0005`／`ADR-0049` 反复防的事是同一个。

## 五条路线（各自的代价；⛔ 本票不推荐任何一条）

- **`D1` 给 `yeban-app` 开一条**默认**的内部边 `→ yeban-mcp`（只开 `dep:yeban-mcp`，⛔ 不连带 `mcp-http`）**
  技术上最短：界面复用**逐字同一份** `domain::stems`，`ADR-0030` 的语义一位不动。
  代价：① 直接推翻 `release.yml:434` 那条断言与 `AGENTS.md` §2 红线 6 的**那句理由**
  （`Cargo.toml:178` 注释逐字写的是"默认 release 二进制里既没有监听循环，**也没有 yeban-mcp 这一整个依赖图**"）——
  需要人类明确改判"默认构建可以含 yeban-mcp 的领域层，只要不含 `mcp-http` 监听"；
  ② 发行构建变大（清单级上界：新增 `yeban-mcp` 及其独有的内部依赖 `yeban-theory`、`yeban-record`；
  `serde`/`serde_json`/`thiserror`/`libm` 需 CI 现读是否已在图里）；③ 根 `Cargo.toml`/`Cargo.lock` ＝ 集成者排他，
  按 **D51** 必须机械重生成并同批改 `docs/ledger/dependency-licenses.md`；④ 那三个 `#[cfg(feature = "in-process-mcp")]`
  模块的注释口径要一起改，否则文档与依赖图说谎。
- **`D2` 把分轨的**批次核心**抽成共享叶子 crate（`yeban-diagnostics` 的同款做法）**
  唯一同时保住"一份实现"和"发行图不变小"的路。代价：① 要先把 `export` 从 `domain/render.rs`（**5539** 行）里
  切出它真正依赖的那部分（`build` / `guard_output_path` / `write_stem_container` / `AssetStore`），
  并把 `Fault`/`ErrorCode` 换成领域中立的错误类型（`yeban-mcp` 再包回 `Fault`，⛔ 让黄金表漂移）；
  ② 新增 workspace 成员 ⇒ 根 `Cargo.toml`（集成者排他）＋ `cargo deny` 重跑；③ **跨线冲突**：
  `crates/yeban-mcp/**` 今天由**另一条线**（MCP 缺口）在写，一个文件同一时刻只能有一个写者。
  ⇒ 需要先与 MCP 那条线排队，⛔ 并行硬改。
- **`D3` 界面入口本身按 feature 门控**（只有 `in-process-mcp` 构建里才画这颗按钮）
  代价：默认发行物里要么没有这颗键（用户在 Tab 1 看到一排导出键里缺"分轨"，与工具面能力不匹配），
  要么画了但终端无宿主 handler ⇒ 就是上面第 2 条那个**骗过判据的假控件**。
  ⚠ 若走"画了但不接"，必须**同时**把它登记进 `KNOWN_ACTIONLESS_CONTROLS`（`[&str; 3]` → `[&str; 4]`）
  并更新 `ADR-0052`（它现在把这三键当作**全集**在钉）。本票视这条为**最差**选项，但仍列出来，因为它最便宜。
- **`D4` 界面侧第二份分轨实现**（`crate::render_core::render_project_f32*` ＋ `save::write_file_atomically`）
  代价：违反本票"⛔ 第二份真相源"的第 3 条；`stem_file_name`、`SkipReason::MasterBusTrack` / `NotASourceNode`、
  `N+1` 基线帧数、批内存在性预检这**四样**都会出现两份。⛔ 建议，⛔ 建议即便人类想省 `D2` 的工时也要先登记"哪一份赢"。
- **`D5` 承认这一格⛔ 属于界面**：把 `feature-alignment.md:161` 系统列的措辞从
  「GUI 无导出控件（有意：`ADR-0001` D47 的 CLI/离线语义）」**补一句**写明"⛔ 仅因 D47，另因 `ADR-0053` 的依赖边阻塞"，
  并把 `#33` 这一票**关掉**（不是"暂缓"）。零代码、零基准改动、零排队。
  代价：用户在界面上仍然拿不到分轨（只能 CLI/MCP）；而 `ADR-0030` 的动机（"调用方必须自己跑 N 次"）
  对**不写命令行的用户**仍然成立 ⇒ 这是一个**产品优先级**判断，⛔ 集成者能定。

## 待人类批准的具体问题（逐条可批可驳）

1. `AGENTS.md` §2 红线 6 与 `Cargo.toml:178` 那句理由——**"默认构建不含 yeban-mcp 整张图"是硬约束，还是只是为了"不含监听循环"的手段**？
   这一问直接决定 `D1` 可不可行；`D1` 若被驳回，`D2` 是**唯一**还能保住单一实现的路线。
2. `ADR-0030` 落地时把 GUI 那半登记为"有意不做"（`feature-alignment.md:161`）—— 那条"有意"的**理由**是否包含本票新登记的依赖边阻塞？
   若不包含，是否要按 `D5` 重写那一格？
3. 若批 `D2`：`crates/yeban-mcp/**` 的**排队顺序**（另一条线正在改同一 crate）由谁裁定，以及新成员 crate 的名字与归属（`yeban-stems`？并进 `yeban-render`？）。
4. 若批 `D3`：是否接受"默认发行物里有一个只在特定构建可用的控件"，并要求它进 `KNOWN_ACTIONLESS_CONTROLS` ＋ 与 `ADR-0052` 同批。

## 一句话结论

「导出分轨」的界面入口⛔ 缺的是**一条依赖边**，⛔ 一个按钮：它要复用的那份实现在默认发行构建里**根本不存在**
（`optional = true` ＋ `#[cfg(feature = "in-process-mcp")]` ＋ `release.yml:434` 三重现读），
而替代它的任何做法都要么把 `yeban-mcp` 编进发行物（改红线 6 的理由），要么做第二份真相源（违反 `ADR-0005`），
要么画一颗按下去没反应的键（正是 `ADR-0052` 在登记的病）。⇒ 本票**不动代码**，把四条路线交给人类。
