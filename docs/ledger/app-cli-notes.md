# app-cli 工作线台账（命令行面：`--open` / `--save-as` / `--export-elements` / `--version`）

- **台账类型**：交付映射 / CLI 语法表 / 实测输出 / 判据清单 / 注入记录 / Quick Start 引用清单 / 未决项（**不是规范**）
- **工作线**：`line/app-cli`（worktree `yeban/.worktrees/app-cli`，基线 main `b8fe21e`）
- **所有者目录**：`crates/yeban-app/**`（本台账是唯一新增的共享区文档）

> ## ⚠ 修订（`line/app-no-compat`，main `632b0c0`，2026-10-04）
>
> ADR-0001 **D43**（1.0.0 之前没有历史包袱与兼容需求，发现问题 / 更优解直接推翻）落地第一刀：
> **裸 `project.json` 兼容读路径已删除**，`.yeban` 容器是**唯一**工程格式。
> - `--open <裸 project.json>` 由"能打开且 `format=project-json`"变成**明确拒绝**（退出码 `3`，
>   stderr 精确到 ``不是 `.yeban` 容器 (容器裁决: end-of-central-directory record not found)``）；
> - `format=` 只剩一个取值 `yeban-container`；`--help` 不再提裸 JSON；
> - 判据 39 / B10 是**反转**（不是删除），证据见 `docs/ledger/app-no-compat-notes.md`。
>
> 本台账下列条目已经就地改成新行为；历史记录保留但标注了"已删除"。

- **规范来源 (Normative)**：
  - `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md`：§5.3 `[ARCH-SEC-003]`（容器）/ `[ARCH-SEC-004]`（原子落盘）、
    §7 `[ARCH-UI-003]`（无头运行）、§8（crate 拓扑与依赖方向）
  - `docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md` §12.1 `[UI-TEST-003]`（无头）、§12.2 `[UI-TEST-001]`（语义元素 ID）
  - `docs/adr/ADR-0001-workspace-topology-and-version-pinning.md`：**D28**（`host.rs` 唯一注入点）、
    **D30**（容器：只写 stored；读法歧义用"拒绝"或"明示"消除）、**D22**（debug info 由构建期打开）、
    **D5**（版本唯一事实源 = 根 `Cargo.toml`）
  - `AGENTS.md` §2 红线 6（发行默认不开危险 feature）、§3 DoD、§5 执行环境纪律
- **本机纪律**：全程 `bash scripts/dev/cargo-local.sh`；`run-gates.sh crate yeban-app` 会 SKIP（含 Slint）。
  本机只跑 `run-gates.sh light` + 两条**零 Slint 探针**（见 §4.1），**没有**在本机编译 Slint。

---

## 1. 交付映射（文件 → 规范 ID → 做了什么）

| 文件 | 规范 ID | 内容 |
| :--- | :--- | :--- |
| `crates/yeban-app/src/cli.rs`（**新增**） | `ARCH-UI-003` `UI-TEST-001` `UI-A11Y-001/002` | 零 Slint 的命令行面：参数解析、用法文本、真实版本、当前工程装载、诚实报告、退出码契约、批处理执行。**全部** CLI 逻辑住在这里，`main.rs` 只剩分发 |
| `crates/yeban-app/src/main.rs`（改写为分发层） | `ARCH-UI-003` `UI-TEST-003` `ARCH-TOP-002` | `parse` → `SLINT_BACKEND=headless` 哨兵折算 → `wants_gui()` 分流；GUI 路径用 `--open` 的工程经 `host::build_main_window` 注入。模块头那段"SLINT_BACKEND=headless 不存在"的实测说明**原样保留** |
| `crates/yeban-app/src/open.rs`（新增公开入口 + 变体） | `ARCH-SEC-003` `MUST-GATE-006/007` `MODEL-AST-002` | `OpenedProject` / `open_project_document_file`：打开 `.yeban` 容器；`OpenError::NotAYebanContainer`（**不是容器**）；`read_capped` 抽出"读之前按 metadata 判上限 + 读回再判"。**修订（D43）**：`DocumentFormat` 枚举与裸 `project.json` 路径已删除，`format=` 只剩常量 `DOCUMENT_FORMAT = "yeban-container"` |
| `crates/yeban-app/src/save.rs`（抽出 + 新增） | `ARCH-SEC-004` | `write_file_atomically`（**唯一**落盘实现）、`save_archive_file`（`history.dag` + 资产池**保真**另存）；`save_project_file` 语义不变（`ui/force_save` 的接线一行未改） |
| `crates/yeban-app/tests/cli_contract.rs`（**新增**） | `ARCH-SEC-004` `ARCH-SEC-003` `ARCH-UI-003` `UI-TEST-001` | 12 条**真二进制**端到端判据（argv / stdout / stderr / 退出码 / 落盘后果），带 120s 超时护栏（防止某个"无窗口"开关回归后被错误送进事件循环而把 CI 挂死） |
| `crates/yeban-app/src/lib.rs` | — | 注册 `pub mod cli`；crate 文档补命令行面 |
| 本台账 | — | CLI 语法表 / 实测 / 判据 / 注入 / Quick Start 引用清单 |

**没有**改：根 `Cargo.toml`、`.github/**`、`scripts/**`、`deny.toml`、其它 `crates/**`、`spikes/**`、法务文件、
`README*.md`、`docs/DEVELOPMENT_LEDGER.md`、`docs/adr/**`、`docs/YEBAN_*.md`、`schemas/**`。
**没有新增任何第三方依赖**（连 `serde_json` 都没有 —— 旧裸 JSON 手法见 §5.1，**已随 D43 删除**）。

---

## 2. CLI 语法表

### 2.1 开关

| 开关 | 取值 | 语义 | 退出码 |
| :--- | :--- | :--- | :--- |
| `--open <path>` | 路径，**只能给一次** | 打开 `.yeban` 容器作为**当前工程**。容器拒绝的原因原样上报；**不是容器**（裸 `project.json` / 随机字节 / 空文件）⇒ 精确拒绝 | 失败 `3` |
| `--save-as <path>` | 路径，**只能给一次** | 把当前工程**原子**写成 `.yeban` 容器（同目录临时文件 → `fsync` → `rename` → 刷目录） | 失败 `4` |
| `--export-elements <path>` | 路径，**只能给一次** | 把语义元素注册表**原子**写到文件（内容与 `--dump-elements` 打到 stdout 的逐行相同） | 失败 `5` |
| `--dump-elements` | 无 | 元素注册表打到 stdout（每行一个元素，稳定顺序） | 失败 `1`（投影失败）|
| `--print-shortcuts` | 无 | 快捷键策略表（画布聚焦列 vs IME 合成态列） | 同上 |
| `--project-sample <default\|filled>` | `default`/`demo`/`filled`，可重复（**最后一个为准**） | "没有 `--open` 时"用哪个内置工程 | `2`（未知取值）|
| `--headless` / `-h`? | 无 | 无窗口自检路径 | — |
| `--help` / `-h` | 无 | 打印用法并退出 `0`（**短路**） | `0` |
| `--version` / `-V` | 无 | 打印 `yeban-app <workspace 版本>` 并退出 `0`（**短路**） | `0` |

`--headless` 的短形式**不存在**（`-h` 是 `--help`；这是 GNU 惯例，也在 `--help` 里写明）。
带取值的开关同时支持 `--opt value` 与 `--opt=value`；`--opt --other` 判为**缺取值**（`2`）。

### 2.2 组合语义（**这一节是可判据的契约**，判据 `combination_semantics_are_exactly_the_documented_table`）

| 组合 | 行为 |
| :--- | :--- |
| 无参数 | GUI：投影 → `host::build_main_window` → 事件循环 |
| `--open a.yeban` | GUI，当前工程 = 打开的那个文件（经 `host.rs` 的**唯一**注入点） |
| `--headless` | 无窗口：不读任何文件，用演示工程自检，打印 `headless ok` + 读数 |
| `--headless --open a.yeban` | 无窗口，但**真的**打开文件、真的投影、打印它的读数（无显示器环境的"打开这个工程"自检）|
| `--open 非容器文件`（裸 `project.json` / 空 / 随机字节） | **明确拒绝**：退出 `3`，stderr = ``不是 `.yeban` 容器 (容器裁决: end-of-central-directory record not found)``；stdout 为空。**没有**任何"打开成空工程"的分支（D43） |
| `--save-as b.yeban` | 无窗口；**没有** `--open` ⇒ 当前工程 = 内置样本，输出 `project-source: sample=…` 与 `saved: … from=sample=…` **明说** |
| `--open a --save-as b` | 另存为：**归档保真**（`history.dag` 与资产池一并写出，不是只写 `project.json`） |
| `--export-elements f` + `--save-as b` | 顺序固定：**先**导出，**再**保存；导出失败 ⇒ 不写工程（退出 `5`） |
| `--help` / `--version` + 其它任何参数 | 短路：立即打印并退出 `0`，其余参数（**含未知参数**）不再检查 |
| `--headless --headless` | 幂等（布尔开关允许重复）；`--open`/`--save-as`/`--export-elements` 重复 = `2` |
| `SLINT_BACKEND=headless`（环境变量，**单独**给即可） | 等价于 `--headless`，把它折算成"无窗口"（判据 B2b 用真进程验证它**单独**也不会开窗口） |

### 2.3 退出码

| 码 | 含义 |
| :--- | :--- |
| `0` | 成功（含 `--help` / `--version` / 无头自检完成） |
| `1` | 界面路径失败（无法创建窗口 / 事件循环异常 / 工程无法投影成界面） |
| `2` | 命令行用法错误（未知开关 / 缺取值 / 不支持取值 / 重复 / 未知样本） |
| `3` | `--open` 失败（读失败 / 超 4 GiB / **不是 `.yeban` 容器** / 容器拒绝：压缩法、Zip-Slip、炸弹、截断、CRC、缺件、非法 JSON …） |
| `4` | `--save-as` 失败（临时文件 / 刷盘 / 重命名任一步失败，或容器写出被拒） |
| `5` | `--export-elements` 失败 |

失败时 **stdout 为空**（判据 B4/B7 断言）：不允许"先打一半报告再报错"，那样脚本会读到半真半假的状态。

### 2.4 报告格式（诚实输出）

`key=value` 逐行（唯一自由文本字段是行尾 `title="…"`，最小转义）。**两个 counts 行的键不重名**，
所以脚本可以直接按键取值，不必先判断行归属：

```text
headless ok
opened: path=… bytes=… format=yeban-container                   (或 project-source: sample=…)
project: id=… bpm=120.00 ts=4/4 title="夜半 Yeban"
project-counts: tracks-all=7 master-track=1 scenes=4 sections=4 clips-pool=2 midi-notes=6 assets-indexed=0 asset-blobs=0 history-bytes=0
view-counts: tracks=6 master=1 clips=3 notes=6 sections=4 scenes=4 elements=211 dynamic-regions=14
headless: 未构造 MainWindow, 未初始化 Slint 后端, 未渲染任何像素 —— …
saved: path=… bytes=… history-bytes=… assets=… temp=… from=…
exported: path=… lines=… bytes=… temp=…
```

- `tracks-all` = **含**主总线的轨道数（模型结构）；`view-counts` 的 `tracks` = **不含**主总线的投影轨道数。
- `clips-pool` = 片段池条目数；`view-counts` 的 `clips` = 落到轨道上的**摆放**数（同一个池条目可被多处摆放 ⇒ 两者本来就不等）。
- `midi-notes` 与 `view-counts` 的 `notes` **同口径**（按音符身份去重，与 `bridge::from_project` 一致）。
- `temp=` 是本次真正用过的临时文件名（保存成功后它必须已不存在；判据会去查目录）。

---

## 3. 实测输出（真实 stdout / stderr / 退出码）

来源说明（**务必按这个口径引用**）：下面的片段来自 `crates/yeban-app/src/cli.rs` 与 `main.rs` 的
**同一条代码路径**，但被测进程是**本机零 Slint 探针二进制**（`/tmp/app-cli-harness/probe.rs`，
与 `main.rs` 的无窗口分发逐行同构）—— 因为本机不允许编译 Slint。
**真二进制那一半由 CI 判决，而且判决已经读回来了**（见 §4.1 的 CI 行与 §9 item 1）：
run `37242779089` 的 `rust (yeban-app)` job 里有一行原始日志
`Running tests/cli_contract.rs (target/debug/deps/cli_contract-…)` → `test result: ok. 12 passed; 0 failed`。

```text
$ yeban-app --version
yeban-app 0.0.1
exit=0

$ yeban-app --headless
headless ok
project-source: sample=default (内置演示工程; 未给 --open ⇒ 未读任何文件)
project: id=01J8Z5Q0R7K3M9X2V4B6N8P0P0 bpm=120.00 ts=4/4 title="夜半 Yeban"
project-counts: tracks-all=7 master-track=1 scenes=4 sections=4 clips-pool=2 midi-notes=6 assets-indexed=0 asset-blobs=0 history-bytes=0
view-counts: tracks=6 master=1 clips=3 notes=6 sections=4 scenes=4 elements=211 dynamic-regions=14
headless: 未构造 MainWindow, 未初始化 Slint 后端, 未渲染任何像素 —— 控件树断言需要 crates/yeban-ui-test-port 的 testing backend
exit=0

$ yeban-app --save-as /tmp/demo.yeban          # 没有 --open ⇒ 明说是演示工程
... (project-source: sample=default …)
saved: path=/tmp/demo.yeban bytes=6149 history-bytes=0 assets=0 temp=demo.yeban.tmp-01M44JR0H8VYYMJ5642J1JXR03 from=sample=default (内置演示工程, 不是从文件打开的)
exit=0

$ yeban-app --open /tmp/demo.yeban --headless  # 再打开它（字节数一致 ⇒ 真的读了那个文件）
headless ok
opened: path=/tmp/demo.yeban bytes=6149 format=yeban-container
project: id=01J8Z5Q0R7K3M9X2V4B6N8P0P0 bpm=120.00 ts=4/4 title="夜半 Yeban"
project-counts: tracks-all=7 master-track=1 scenes=4 sections=4 clips-pool=2 midi-notes=6 assets-indexed=0 asset-blobs=0 history-bytes=0
view-counts: tracks=6 master=1 clips=3 notes=6 sections=4 scenes=4 elements=211 dynamic-regions=14
headless: 未构造 MainWindow, …
exit=0

$ yeban-app --open /tmp/truncated.yeban --headless
(stderr) yeban-app: 打开 `/tmp/truncated.yeban` 失败: 容器被拒绝: end-of-central-directory record not found
exit=3

$ yeban-app --open /tmp/nope.yeban --headless
(stderr) yeban-app: 打开 `/tmp/nope.yeban` 失败: 无法读取 `/tmp/nope.yeban`: No such file or directory (os error 2)
exit=3

$ yeban-app --open /tmp/target.yeban --save-as /tmp/target.yeban   # 目录 chmod 555
(stderr) yeban-app: 保存到 `/tmp/target.yeban` 失败: 写临时文件 `/tmp/target.yeban.tmp-01M44JVDRH9H0DPJFNCM3BBQDD` 失败: Permission denied (os error 13)
exit=4      # 且旧文件字节未变（判据断言）

$ yeban-app --bogus
(stderr) yeban-app: 无法识别的参数 `--bogus`
(stderr) <空行> + 完整用法（含全部开关与退出码）
exit=2

$ yeban-app --open /tmp/demo.yeban --export-elements /tmp/e.txt --headless
exported: path=/tmp/e.txt lines=211 bytes=28221 temp=e.txt.tmp-01M44JVDS62S0XX2BSQ236A7D2
exit=0        # /tmp/e.txt 实测 211 行

$ yeban-app --open /tmp/project.json --headless     # 裸 project.json（从真容器里取出的那一份）
(stderr) yeban-app: 打开 `/tmp/project.json` 失败: `/tmp/project.json` 不是 `.yeban` 容器 (容器裁决: end-of-central-directory record not found)
exit=3        # stdout 为空：既不是"未知格式"，也不是"打开成空工程"（D43 删掉了兼容读路径）

$ unzip -l /tmp/demo.yeban                          # 写出来的是**标准 ZIP**（D30）
     5929  01-01-1980 00:00   project.json
        0  01-01-1980 00:00   history.dag
```

---

## 4. 判据清单

### 4.1 本机**真跑**（`rustc --edition 2024 --test -D warnings`，**零 Slint**）

探针在 `/tmp/app-cli-harness/`（**仓库之外**，不入库），用 `#[path]` 指向仓库原件 —— **不是复制品**：

| 步骤 | 命令 | 结果 |
| :--- | :--- | :--- |
| 零 Slint 半边单元判据（cli/bridge/scene/elements/input/open/save） | `bash /tmp/app-cli-harness/run.sh` | ✅ **95 passed; 0 failed** |
| 同上的一半的 clippy（`-D warnings -D clippy::all`） | `bash /tmp/app-cli-harness/clippy.sh` | ✅ **零告警** |
| `tests/cli_contract.rs`（12 条，被测进程 = 本机探针二进制） | `bash /tmp/app-cli-harness/verify.sh` | ✅ **12 passed; 0 failed** |
| 门禁 light | `bash scripts/gates/run-gates.sh light` | ✅ **门禁通过 (mode=light)** |
| **真二进制**判据（CI 唯一能判的那一半） | `cargo test -p yeban-app --all-targets --locked`（CI run **37242779089** 的 `rust (yeban-app)` job，3m56s） | ✅ **`tests/cli_contract.rs`: 12 passed; 0 failed**（CI 日志原文）；同 job 里 lib 单元判据 `110 passed; 0 failed`；`cargo clippy -p yeban-app --all-targets -- -D warnings` 零告警 |

> 探针用 `env CARGO_PKG_VERSION=<根 Cargo.toml 里读出的 workspace 版本>` 编译 —— 生产构建里这个值由
> Cargo 从**同一份清单**注入，因此 `env!("CARGO_PKG_VERSION")` 在两条路上是同一个事实源。

判据编号（`cli.rs` 单元判据 29–41，`tests/cli_contract.rs` B1–B11）：

| # | 判据 | 断言 |
| :--- | :--- | :--- |
| ① 29 / B1 | `--help` 覆盖全部新开关 + 短路 | 用法含每个开关与每个退出码；`--help --bogus` 仍 `0`；不打印握手行（按**独立成行**判定，因为用法文本里本来就引用了它） |
| ② 30 / B1 | `--version` 与 `Cargo.toml` 一致 | `version_text() == "yeban-app {CARGO_PKG_VERSION}"` **且**直接读根清单的 `[workspace.package] version` 对账 |
| ③ 31 / B3 | 真容器 ⇒ `--open` 的读数与工程一致 | `tracks-all` / `midi-notes` 由测试**独立数一遍**模型结构再比；`view-counts: notes` 同口径；`history-bytes` / `asset-blobs` / `format=` |
| ④ 32 / B4 | 截断容器 / 空文件 / 垃圾 / 随机字节 / 缺失文件 / **目录** | 退出码 `3`；stderr 带**精确**原因（容器裁决或"不是 `.yeban` 容器"）；stdout 为空；`code != 101`（不是 panic）；**绝不**退化成空工程 |
| ⑤ 33 / B5 | `--save-as` 写出的文件能被 `open_project_file` 读回 | 工程逐字段相等；**归档**（`history.dag` + 资产池）逐字段相等；打印 `bytes=` == 实际落盘字节 |
| ⑥ 34 / B9 | 只读目录 ⇒ 非零退出 + 原文件未被破坏 | 退出码 `4`；旧文件字节**一字未改**；无临时文件残留（注入 2 证明这条真的能区分"原子替换"与"就地覆盖"） |
| ⑦ 35 / B5 | 两次 `--save-as` 的确定性 | 两次文件字节 `==`；两次打印的 `bytes=` 相同；两次都用了临时文件（`temp=` 里的 ULID 不同 —— 它是"真的建了临时文件"的痕迹） |
| ⑧ 36 / B7 | 未知开关 / 缺取值 / 重复 / 未知样本 | 退出码 `2` + stderr 带原因**和**完整用法提示（**不许**静默忽略） |
| 37 / B6 | `--save-as` 无 `--open` ⇒ 演示工程**并明说** | `project-source: sample=default` + `未读任何文件` + `from=sample=default`；且**不**出现 `view-counts`（保存不需要投影） |
| 38 / B8 | `--export-elements` 与 `--dump-elements` 同源 | 落盘行集合 == stdout 元素行；`bytes=` == 文件长度；导出失败 ⇒ 退出 `5` 且**不**写工程 |
| 39 / B10（**反转**） | 裸 `project.json` **被明确拒绝**（D43） | 退出 `3` + stderr ``不是 `.yeban` 容器 (容器裁决: end-of-central-directory record not found)``；stdout 为空；同一份 JSON 的真容器照样能打开（拒绝的是容器边界，不是内容） |
| 40 | 组合语义表 | 8 种组合的 `wants_gui()` / `needs_projection()` 与 §2.2 逐格一致 |
| 41 | 报告自描述 | 引号转义；样本来源无 `opened:`；文件来源首行 `opened:` |
| B2 / B2b | 握手行与哨兵 | `--headless` ⇒ `headless ok` **恰好一行**；`SLINT_BACKEND=headless` **单独**也不开窗口（真进程 + 120s 超时护栏） |
| B11 | `--print-shortcuts` | 无窗口路径、退出 `0`、策略表内容在里面 |

### 4.2 交给 CI 判（本机**不能**判，逐条说清）

- 含 **Slint** 的一切：`main.rs` 的 GUI 分发、`host::build_main_window`、`.slint` 的语法与类型、
  `live_surface.rs`（app-mixer 线的管理动作）—— 本机不编译 Slint。
- **真二进制**的退出码与 stdout（`cargo test --workspace --all-targets` 里的 `tests/cli_contract.rs`）：
  本机跑的是同构探针进程，能证明**逻辑**，不能证明**链接后的产物**。
- `--open <path>`（**不带** `--headless`）这条 GUI 路径：需要显示器，CI 无显示器 ⇒
  **没有任何自动化判据覆盖它**（已登记为 needs）。它的报告行与失败诊断复用同一份实现。
- `cargo clippy -p yeban-app --all-targets -D warnings`（含 Slint 目标）与 `cargo fmt --all --check` 的 CI 版。

### 4.3 注入 → 变红 → 还原（**4 条，全部真做过**；本线追加的 2 条见 `app-no-compat-notes.md` §4）

方法：源文件备份到 `/tmp/app-cli-backup/`，注入后用两条探针重跑，记录红掉的判据名，
再从备份还原并 `cmp` 逐文件确认 identical（`md5` 亦与注入前一致：`cli.rs ae87a8cf…` / `save.rs db15d03c…` / `open.rs fd24772c…`）。
**注**：上表的 4 条是 app-cli 线在当时那版代码上做的；`line/app-no-compat` 又对**新代码**做了
2 条注入（把裸 JSON 分支加回来 / 把容器错误吞成空工程），红点记录与 `md5` 复核见
[`app-no-compat-notes.md`](app-no-compat-notes.md) §4。上表第 1 条的注入形态（吞错误 ⇒ 空工程）
在新代码上仍然红（4 条单元 + 2 条真进程），与本线注入 B 同族。

| # | 注入（任务建议的形态） | 改法 | 实测红点 |
| :-- | :--- | :--- | :--- |
| 1 | **容器错误吞掉返回空工程** | `cli::load_project` 在 `OpenError` 时返回 `YebanProjectV1::default()` 的归档 | `cli::tests::broken_inputs_fail_with_exit_code_three_and_a_precise_reason`（94 passed / **1 failed**）；真进程 `broken_inputs_exit_non_zero_without_panicking`（11 passed / **1 failed**）|
| 2 | **`--save-as` 直接写目标路径（非原子）** | `save::write_file_atomically` 改成 `std::fs::write(&path, bytes)`（临时文件建了又删、`rename` 整段拿掉） | `save::tests::a_read_only_directory_never_touches_the_existing_file`、`cli::tests::save_as_into_a_read_only_directory_fails_and_leaves_the_old_file_intact`（93 passed / **2 failed**）；真进程 `save_as_into_a_read_only_directory_exits_four_and_keeps_the_old_file`（11 / **1**，红点是 `left: 0, right: 4` —— 就地覆盖**成功了**，旧文件被破坏）|
| 3 | **`--version` 写死常量** | `version_text()` 恒返回 `"yeban-app 9.9.9"` | `cli::tests::version_matches_the_workspace_manifest`（94 / **1**）；真进程 `help_and_version_are_real_and_short_circuit`（11 / **1**）|
| 4 | **未知开关静默忽略** | `parse` 的 `other => Err(...)` 改成吞掉 | `cli::tests::unknown_and_malformed_arguments_are_usage_errors`（94 / **1**）；真进程 `unknown_switches_exit_two_with_usage_on_stderr`（11 / **1**）|

> 注入 2 的第一版**编不过**（`write_temp` 变 dead code 撞 `-D warnings`）：这说明"注入要能编译到运行期
> 才叫行为注入"。第二版保留 `write_temp` 的调用（结果丢弃）后，红点是**行为**的，不是编译的。
>
> 还原复核：`grep -rn 'INJECT' crates/` = **0 命中**；`cmp` 三个文件全部 byte-identical。

---

## 5. 边界（如实登记，**不是**静默降级）

### 5.1 ~~裸 `project.json` 是怎么"顺便"支持的~~ ⇒ **已删除**（ADR-0001 D43）

**旧行为（历史记录，已不存在）**：`open_project_document_file` 先按容器读；失败时若文件
"看起来仍像容器"就原样上报容器裁决，否则若首个非空白字节是 `{` 就把裸 JSON 在**内存里**
用 `write_container` 包成最小容器（`project.json` + 空 `history.dag`）再交给权威读取器，
形态用 `DocumentFormat::BareProjectJson` 明示（`format=project-json`）。

**为什么删**：D30 已经把容器定为唯一载体；D43（2026-10-04 负责人授权）明确"1.0.0 之前没有
历史包袱与兼容需求，发现问题 / 更优解**直接推翻**"。这条兼容路径的成本是实打实的：
一个只服务兼容的 `DocumentFormat` 枚举、一个只服务兼容的错误变体
（`NotAContainerNorJson`）、一条"看起来像 ZIP 就绝不掉进 JSON 分支"的补丁式判断、一条
`wrap_bare_project_json` 的成套逻辑，以及"同一份 JSON 有两种读法"的歧义面。

**新行为**（判据 39/B10 是**反转**来的，不是删掉的）：

1. 按容器读；成功 ⇒ `Ok`。
2. 失败且文件**有 ZIP 结构** ⇒ 原样上报容器裁决（截断 / 篡改的 `.yeban` 仍拿到精确错误码）。
3. 否则 ⇒ `OpenError::NotAYebanContainer`：**"不是 `.yeban` 容器"**，并携带容器原裁决
   （通常是 `EocdNotFound`）。裸 `project.json`、空文件、随机字节都走这一支。

任何一条失败路径都**不会**返回默认 / 空工程。判据 26/B10 覆盖"裸 JSON 被拒绝"，
判据 28/B4 覆盖"空 / 垃圾 / 随机字节 / 坏 JSON 各自精确报错"，
判据 20 覆盖"容器**内部**的坏 JSON ⇒ `InvalidProjectJson`"（这条**保留**：容器里的
`project.json` 仍然是权威工程文档）。

### 5.2 `--save-as` 保真 `history.dag` 与资产池

`save_project_file` 只拿得到一个 `YebanProjectV1`，因此它写空的 `history.dag`（app-mixer 线
`ui/force_save` 的既有语义**一行未改**）。命令行需要的是"打开再另存**不丢东西**"，
所以新增 `save_archive_file`：把打开时拿到的归档（工程 + `history.dag` + `assets/{sha256}`）
原样写回。已知代价：`write_project_container` 的签名要 `BTreeMap<AssetHash, Vec<u8>>`，
而 `ProjectArchive::assets` 是 `Vec<(AssetHash, Vec<u8>)>` ⇒ 大资产池多一次内存拷贝（见 §7 needs-2）。

### 5.3 GUI 里的"保存"与命令行的"保存"是两条入口、一份落盘实现

- 命令行：`cli::run_batch` → `save::save_archive_file`（批处理，无窗口）
- 控制面：`live_surface` 的 `ui/force_save` → `save::save_project_file`（app-mixer 线的接线，未改）
- 两者共用 `save::write_file_atomically`（**唯一**的"临时文件 → fsync → rename → 刷目录"实现）。

### 5.4 GUI 里改了工程再 `--save-as` 目前**不存在**

`main.rs` 的 UI 回调仍未接线（`wire_callbacks` 只打 stderr），而且在 GUI 路径上 `--save-as`
**不适用**：`--save-as` 会让进程走无窗口批处理（`Options::batch`）。因此 Quick Start 里
**只能**写"打开 → 另存/导出"这条路，**不能**写"在界面里改一改再保存"。这条已登记为未实现项。

---

## 6. **Quick Start 可以引用哪些命令**（给集成者写 README / 官网用）

> 前提（写文档时必须一起写出来，否则 Quick Start 就是假的）：
> 1. 二进制来自 `cargo build --release -p yeban-app`（产物 `target/release/yeban-app`）。
>    它**含 Slint** ⇒ 首次构建需要 GUI 工具链依赖；Linux 的准确 apt 清单见
>    `.github/workflows/ci.yml` 的 workspace job（`pkg-config libfontconfig1-dev libfreetype-dev
>    libxkbcommon-dev libwayland-dev libx11-dev libgl1-mesa-dev` 等）。
> 2. 下面标 ✅ 的命令**不需要显示器**，在任何机器（含 CI / 无头服务器）都能跑；
>    标 🖥 的命令需要显示器，**没有自动化判据覆盖**。
> 3. 版本以 `yeban-app --version` 为准；本仓库当前 workspace 版本是 `0.0.1`。

### 可以直接复制进 README / 官网的片段

```bash
# 版本与用法（真实版本；用法里含全部开关、组合语义与退出码）
yeban-app --version
yeban-app --help

# ① 无头自检：不构造窗口、不依赖显示器；打印 `headless ok` 与工程读数
yeban-app --headless

# ② 造一个演示工程文件（没有输入也能立刻拿到一个可用的 .yeban）
yeban-app --save-as demo.yeban

# ③ 打开一个工程并检查它（真的读文件；容器拒绝的原因原样打印，退出码非零）
yeban-app --open demo.yeban --headless

# ④ 另存为（原子落盘；history.dag 与资产池一并保真）
yeban-app --open demo.yeban --save-as copy.yeban

# ⑤ 导出语义元素清单，给脚本 / AI 消费（原子写文件；与 --dump-elements 同源）
yeban-app --open demo.yeban --export-elements elements.txt
yeban-app --open demo.yeban --dump-elements | head

# ⑥ 快捷键策略表（画布聚焦 vs IME 合成态）
yeban-app --print-shortcuts

# ⑦ 切换内置样本（default | filled）
yeban-app --project-sample filled --save-as filled.yeban

# ⑧ 规范里的原样命令行（SLINT_BACKEND=headless 是 yeban 自研哨兵值，Slint 1.18.1 无此后端）
SLINT_BACKEND=headless yeban-app --headless
```

### 逐条可引用性

| 命令 | 可写进 Quick Start？ | 依据 |
| :--- | :--- | :--- |
| `yeban-app --version` / `--help` | ✅ | 判据 ②/①（本机真跑 + CI 真进程） |
| `yeban-app --headless` | ✅ | 判据 B2（真进程，握手行恰好一行） |
| `SLINT_BACKEND=headless yeban-app --headless` | ✅ | 判据 B2b（**单独**给哨兵也不开窗口） |
| `yeban-app --save-as <path>`（无 `--open`） | ✅ | 判据 37/B6（输出**明说**用的是演示工程） |
| `yeban-app --open <path> --headless` | ✅ | 判据 ③/④（真容器读数一致；坏输入精确报错且非零退出） |
| `yeban-app --open <a> --save-as <b>` | ✅ | 判据 ⑤/⑦（归档保真 + 字节确定） |
| `yeban-app --open <path> --export-elements <f>` / `--dump-elements` | ✅ | 判据 38/B8（同源、行数一致） |
| `yeban-app --print-shortcuts` | ✅ | 判据 B11 |
| `yeban-app --project-sample filled --save-as <f>` | ✅ | 判据 37（`filled` 与 `default` 产出不同工程） |
| `yeban-app --open <path>`（**GUI**） | 🖥 只能写"需要显示器" | 无自动化判据（CI 无显示器）—— 见 needs-1 |
| 任何"在界面里改一改再保存"的流程 | ❌ **不要写** | UI→模型写入未接线（红线：不写没做到的） |

**必须写进文档的诚实说明（建议逐字采用）**：

- `--open` 打不开就**报错退出**（退出码 3），**不会**打开成空工程；
- `.yeban` 容器是**唯一**工程格式（ADR-0001 D43）：散落的 `project.json`、空文件、
  随机字节都被**明确拒绝**（退出码 3，stderr 说明"不是 `.yeban` 容器"）；
- `--save-as` 是**原子替换**（同目录临时文件 → fsync → rename），失败时**旧文件保持不变**；
- 没有 `--open` 时 `--save-as` 保存的是**内置演示工程**，输出里 `from=sample=…` 会明说；
- 无窗口命令（`--headless` / `--save-as` / `--export-elements` / `--dump-elements` /
  `--print-shortcuts`）都打印握手行 `headless ok`，因此脚本可以拿它当"没有进事件循环"的证据；
- 未知开关**一律退出 2 并打印用法**，不会被静默忽略。

---

## 7. 未实现项（如实登记）

| # | 未实现 | 现状 | 归属 / 阻塞 |
| :-- | :--- | :--- | :--- |
| 1 | **GUI 路径的自动化判据** | `--open <path>`（不带 `--headless`）会构造窗口并进事件循环；CI 无显示器 ⇒ 本线只交付了它的报告行与失败诊断（复用批处理那份实现），**没有**任何判据覆盖"窗口真的用打开的工程驱动了界面" | 需要 `yeban-ui-test-port` 的 testing backend（`[ARCH-UI-005]`）接到 `main.rs`，或人工在有显示器的机器上核对 |
| 2 | **`--save-as` 之后的"当前工程路径"会话状态** | 进程退出即结束；没有 `.yeban.lock`（`[ARCH-SEC-003]` 的 OS 建议锁）、没有 "保存为默认落点" | `yeban-services` / 会话层（`MODEL-ISO-001` 第二层）；app-mixer 线 needs-3 同族 |
| 3 | **`--open` 的 CLI 侧上限注入** | 命令行走 `ProjectOpenOptions::default()`（4 GiB + 容器默认闸门）；没有 `--max-bytes` 之类的开关 | 有需要再加；不要为了"看起来完整"凭空造开关 |
| 4 | **`.yeban` 以外的导入格式** | 只有 `.yeban` 容器（裸 `project.json` 读路径已随 D43 删除）；没有 `.mid` / `.als` / 音频导入 | 别的工作线（`yeban-decode` / `[ARCH-FMT-002]`）|
| 5 | **`--export-elements` 的格式开关（JSON / 过滤）** | 只有一种稳定文本格式（每行一个元素），与 `--dump-elements` 逐行相同 | 等 `[UI-TEST-001]` 的消费者（MCP / AI 工具）提出真实需求 |
| 6 | **GUI 里的 `--save-as`** | `--save-as` 一律走无窗口批处理；"界面改完再存"需要 UI→模型写入先接线 | 见 §5.4 与 §7 第 1 条 |
| 7 | **`--version` 的构建元数据** | 只有 `yeban-app <semver>`，没有 commit / 构建时间 | 需要构建期注入（`build.rs` 读 git）⇒ 会牵动 Cargo/cargo-deny 面，留给集成者裁决 |

---

## 8. needs（需要人类 / 其它线裁决）

1. **【集成者 / 文档】README 与官网的 Quick Start**：请引用 §6 的 ✅ 清单与"必须写进文档的诚实说明"。
   本线**不改** `README*.md` 与官网（集成者独占）。**关键**：不要写"在界面里改一改再保存"。
2. **【yeban-model / 人类裁决】`write_project_container` 的资产入参形态**：它要
   `&BTreeMap<AssetHash, Vec<u8>>`，而 `ProjectArchive::assets` 是 `Vec<(AssetHash, Vec<u8>)>`
   ⇒ `--save-as` 对大资产池多一次拷贝。建议加一个 `&[(AssetHash, &[u8])]` 形态的写出面
   （或让 `ProjectArchive` 直接带一个写出方法）。本线**不改** `yeban-model`。
3. **【人类裁决】CLI 是否是 stable interface**：退出码 `0..5`、报告字段名、`headless ok`
   现在被文档与脚本依赖。要不要把它们登记成"对外契约"（并在 ADR 里钉住）？
4. **【集成者】`docs/ledger/gate-status.md`**：本线**没有**改它（不属于本线证据的必改项）；
   若要把"CLI 的 `--open` 会如实转达容器裁决"写进 `MUST-GATE-006/007` 的证据列，请由集成者补。
5. **【集成者】`docs/ledger/app-binding-notes.md` §7 第 8 条**（"没有打开文件路径 / 不引入 serde_json"）
   现在**过时**了：`--open` 已落地，且**仍然**没有引入 `serde_json`（见 §5.1）。请集成者更新那一行。
6. **【人类裁决】`--headless` 是否该有短开关**：当前 `-h` = `--help`（GNU 惯例），
   因此 `--headless` 没有短形式。若将来要 `-H`，属 CLI 契约变更。

---

## 9. pending（未证实 / 已知的债）

1. ~~**真二进制的判决尚未读回**~~ **已读回**：`bash scripts/dev/ci-verdict.sh line/app-cli` ⇒
   run **37242779089**（commit `e63cf24`）= **success**；`rust (yeban-app)` job（ID 111554655614，3m56s）绿，
   日志里 `Running tests/cli_contract.rs` ⇒ `test result: ok. 12 passed; 0 failed`，lib 单元判据 `110 passed; 0 failed`。
   ✔ 这条不再是 pending —— 真二进制与 `CARGO_BIN_EXE_yeban-app` 的可获得性都由该 job 证实。
   （本台账这次修订是**纯文档**后续提交；判据、代码与 `Cargo.lock` 一字未动，因此不改变上面那次判决的适用范围。）
2. ~~`CARGO_BIN_EXE_yeban-app` 的可获得性~~ **已由 run 37242779089 证实**：该 job 的
   `cargo clippy -p yeban-app --all-targets -- -D warnings` 与 `cargo test … --all-targets` 都编译并执行了
   `tests/cli_contract.rs`，`env!("CARGO_BIN_EXE_yeban-app")` 正常注入（`env!` 的编译期语义因此被真实验证过）。
   无需 `option_env!` 退路。
3. `--export-elements` 的格式稳定性：本线承诺"与 `--dump-elements` 逐行相同"，
   但 `elements::dump_lines()` 的行格式由 app 侧自定（`[UI-TEST-001]` 只约束 ID 形状）⇒
   行格式变化会同时影响两处（同源，故不会互相漂移）。
4. 只读目录判据在**特权进程**（root）下无效：两条探针都会响亮地打印"本条判据无从判定"并跳过，
   不把"没测到"记成"通过"。CI 的 runner 是非特权用户 ⇒ 该判据在 CI 上真的会执行。

## 10. TODO(hoist)（交给集成者收进全局账本）

- [ ] 把本线的 CLI 契约（退出码 / 报告字段 / 握手行）登记进 `docs/DEVELOPMENT_LEDGER.md` 或 ADR（needs-3）。
- [ ] `docs/ledger/app-binding-notes.md` §7 第 8 条的"没有打开文件路径"改为指向本台账（needs-5）。
- [ ] 若 `yeban-mcp` 的 `store.rs` 落地，把 `save.rs` 换成对它的调用（app-mixer 线的 needs 同族）。
- [ ] README / 官网加 Quick Start 时引用 §6，并**逐字**带上"必须写进文档的诚实说明"。
