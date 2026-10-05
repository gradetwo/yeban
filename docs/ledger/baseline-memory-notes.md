# baseline-memory 工作线账本（`BASELINE-002` 常驻内存基线 —— **带数字的归因**）

> 分支 `line/baseline-memory`，工作树 `.worktrees/baseline-memory`（main `ddd637c`），
> 地盘 `crates/yeban-app/**`、`crates/yeban-engine/**`、`crates/yeban-model/**`。
> 可新建：本文件。
> **禁改且一个字节都没碰**：`scripts/**`（测量脚本由集成者维护）、`.github/**`、`schemas/**`、
> 根 `Cargo.toml`/`Cargo.lock`、`deny.toml`、`docs/adr/**`、`docs/YEBAN_*.md`、
> `docs/DEVELOPMENT_LEDGER.md`、`README*.md`、法务文件、
> `docs/ledger/{gate-status,phase-status,feature-alignment,human-decisions}.md`。
>
> 规范原文（`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:354` **逐字**）：
>
> > | **[BASELINE-002]** | **常驻内存基线** | 空工程空闲内存占用 | 操作系统内存工作集（Working Set）统计 | **≤ 35 MB** | Phase 0 |
>
> `gate-status.md:42` 现记 **部分**。本轮**使命**（集成者第 2 轮收窄）：
> 不要求优化，只交**带数字的归因表** —— 说清"哪个子系统占多少 MB、每项凭什么这么说"，
> 以及"规范所指的对象当前**没有**被覆盖到的部分"。

---

## 1. 先给结论（三句话）

1. **现行 ① 判据的读数（`yeban-app --headless`）在本机 debug 下是 11.30–11.34 MB，稳在 35 MB 以内 —— 但它量的不是规范的对象。**
   `crates/yeban-app/src/main.rs:97-101` 走 `cli::run_batch`，**一个 Slint 对象都不构造**
   （该文件 23-55 行的模块文档自己写明了这一点，`cli.rs:87` 的边界行还会把它打到 stdout）。
2. **规范的对象一旦真的被构造出来，在本机 debug 下就已经超过 35 MB：**
   仅仅"建一个真实 `MainWindow`（软件平台、不渲染）"就把同一个进程从 **11.14 MB** 抬到 **55.97 MB**
   （`ru_maxrss` 口径），`footprint` 的可归属部分为 **34 MB**；再加"运行时控件树遍历 + 一帧
   1920×1080 Tier-1 光栅化"是 **138.52 MB**。
3. **这个数在 CI 上也从来没有被取过**：`bench` 档自加进 BASELINE-002 那一步（`448f865`，
   2026-10-05 08:28:35 +0800）之后**一次都没有被 dispatch**；唯一跑过 `bench` 的 run
   `37228045430`（2026-10-04T19:22Z）早于该提交，它的日志里 `BASELINE-002` 出现 **0** 次。

⇒ `BASELINE-002` 应继续记 **部分**：不是"值超了"，而是**"量法没覆盖规范对象，且这个对象至今没在任何一台机器上被正式量过"**。凭什么这么说，见 §3–§6。

---

## 2. 测法与复跑命令（先测量、再说话）

### 2.1 唯一的仪器

`scripts/gates/measure_rss.py`（集成者地盘，本轮**只读未改**）：

```bash
# 一条命令一个进程；RSS 取自 resource.getrusage(RUSAGE_CHILDREN).ru_maxrss
python3 scripts/gates/measure_rss.py --label <标签> --timeout <秒> -- <被测量命令...>
```

`BENCH baseline=002 label=… peak_rss_mb=… target_mb=35.0 child_exit=… verdict=…`

三条**与本轮结论直接相关**的仪器性质，全部本机实测（不是读注释得来的）：

| 性质 | 实测命令 | 实测输出 | 含义 |
| :--- | :--- | :--- | :--- |
| 仪器自身不占读数 | `measure_rss.py -- /bin/echo hi` | `peak_rss_mb=1.05` | `1.05 MB` 就是 `/bin/echo` 自己的 RSS ⇒ 仪器不把宿主进程算进去 |
| 读数**随机体变化**（不空转） | 同一仪器读 4 个不同机体 | `1.05 / 11.33 / 55.97 / 157.03` | 见 §7 注入 J3；若读数函数返回常数，这条梯度必不成立 |
| **`ru_maxrss` 是高水位，不是"最后一次"** | 先被测量一个分配 70 MiB 的子进程，再被测量 `/bin/echo` | `after BIG child: 82.30 MB` → `after TINY child: 82.30 MB` | 同一个宿主 python 进程里，第二条读数会**粘住**第一条的高水位 ⇒ "一条命令一个进程"是**承重设计**，不是风格 |

### 2.2 复跑全部读数（本机 M2，逐条可复现）

```bash
cd /Users/crow/work/music/yeban/.worktrees/baseline-memory
APP=/Users/crow/work/music/yeban/target/debug/yeban-app
BIN=/Users/crow/work/music/yeban/target/debug/deps/real_ui_tier1-b906813f78cac220

# ① 基线（规范 ① 的现行判据；此二进制由主仓已有的 debug 缓存产出，见 §2.3）
python3 scripts/gates/measure_rss.py --label app-headless --timeout 100 -- $APP --headless
# ② 进程地板（不加载任何工程）
python3 scripts/gates/measure_rss.py --label app-version  --timeout 100 -- $APP --version
# ③ 模型层参照
python3 scripts/gates/measure_rss.py --label model-empty  --timeout 100 -- \
  /Users/crow/work/music/yeban/target/debug/deps/yeban_model-51cb3c826a088766 \
  --exact samples::tests::default_project_sample_is_legal
# ④ 真实窗口（同一条二进制内的消融；无窗口 vs 有窗口 vs 窗口+一帧）
python3 scripts/gates/measure_rss.py --label win-none --timeout 100 -- $BIN --exact criteria::every_registry_role_is_a_slint_accessible_role
python3 scripts/gates/measure_rss.py --label win-one  --timeout 100 -- $BIN --exact criteria::ime_event_source_drives_the_input_context_and_swallows_bare_shortcuts
python3 scripts/gates/measure_rss.py --label win-frame --timeout 100 -- $BIN --exact criteria::runtime_control_tree_cross_check_against_the_registry
```

### 2.3 本机编译纪律与其代价（**必须和读数一起读**）

`yeban-app` 含 Slint ⇒ 本机**不做**首次全量构建、不用 `--workspace`。本轮**一次编译都没有发起**：

- 所有 app / 模型侧读数都来自**主仓已有的 debug 缓存**（`/Users/crow/work/music/yeban/target/debug/`），
  该缓存的 HEAD 与本工作树**同一个 commit**（`ddd637c`，工作树 `git status` 干净 ⇒ 源码逐字节相同）。
- 因此**本轮所有 app 读数都是 debug**。这不是缺陷，反而让结论更稳：**debug 是上界**
  （无优化、无 `debug-assertions=off`、符号表最大）。
- **release 列在本机不可得**（release 下 Slint 是首次全量构建，纪律禁止）⇒ 见 §8 的严格区分。

---

## 3. 基线读数（同一条命令，含重复性）

### 3.1 规范 ① 的现行判据 —— 三次重复

```text
$ for i in 1 2 3; do python3 scripts/gates/measure_rss.py --label repro-$i --timeout 50 -- $APP --headless; done
BENCH baseline=002 label=repro-1 peak_rss_mb=11.33 target_mb=35.0 child_exit=0 verdict=within-target
BENCH baseline=002 label=repro-2 peak_rss_mb=11.30 target_mb=35.0 child_exit=0 verdict=within-target
BENCH baseline=002 label=repro-3 peak_rss_mb=11.33 target_mb=35.0 child_exit=0 verdict=within-target
```

**极差 0.03 MB（0.27%）** ⇒ 判据 ③ 成立（同一命令两次读数在同一量级，差异已记录）。

### 3.2 逐层抬升（本节所有行都用**同一条** `measure_rss.py`）

| 被测命令（缩写） | `peak_rss_mb` | verdict | 规范 ID |
| :--- | ---: | :--- | :--- |
| `/bin/echo hi` | 1.05 | within-target | 仪器地板（证明仪器不加料） |
| 模型层测试二进制 `--list` | 2.56 | within-target | 模型 crate 的进程地板 |
| `yeban-model --exact samples::tests::default_project_sample_is_legal`（**空工程**样本） | 3.06 | within-target | `BASELINE-002` 的模型侧下界 |
| `yeban-model --exact samples::tests::export_all_writes_four_byte_stable_samples`（default + filled 全导出） | 4.70 | within-target | 模型侧参照 |
| **`yeban-model` 全套判据（107 条，全绿）** | **7.72** | within-target | 整个模型层的峰值上界 |
| `yeban-app --version`（不加载工程） | 8.53 | within-target | app 进程地板（含静态链接 Slint/cpal 的 dyld 绑定） |
| `yeban-app --headless`（**规范 ① 现行判据**） | **11.33** | within-target | `BASELINE-002` ① |
| `yeban-app --headless --project-sample filled` | 11.17 | within-target | ② 演示工程读数 |

⇒ **模型层（含 `BTreeMap` 结构本身）在本机是可忽略项**：空工程样本 3.06 MB 里有 2.56 MB 是进程地板，
真正属于"空工程数据"的约 **0.5 MB**；即便跑完 107 条模型判据，峰值也只有 **7.72 MB**。
app 侧的"演示工程 + 投影 + 报告"整块是 **11.33 − 8.53 = 2.80 MB**。

---

## 4. 逐项归因表（**核心产物**）

**测法**：在**同一条二进制**（`target/debug/deps/real_ui_tier1-b906813f78cac220`，工作线 HEAD 的
`cargo test -p yeban-app --all-targets` 产物）内，用 `--exact` 逐条跑不同判据 ——
**进程地板与测试框架开销在行间相互抵消**，于是每一行的差值可以归给"这条判据多构造了什么"。
所有行 `child_exit=0`（判据本身通过；见 §7 判据 ⑤）。

| # | 该项构造了什么（读源码确认，不是猜） | 读数 MB | 与本行上方的差值 | 证据命令（`$BIN` 同上，均带 `--exact`） |
| ---: | :--- | ---: | ---: | :--- |
| 0 | 纯注册表逻辑，**不建窗口** | 11.14 | — | `criteria::every_registry_role_is_a_slint_accessible_role` |
| 0b | 纯注册表：另三条同量级 | 11.22 / 11.42 / 11.44 | ±0.3 | `criteria_id_lists_are_all_real_registry_entries` / `static_tree_cannot_mask_without_geometry`（两次） |
| 1 | 注册表适配 + 控件树 JSON 往返，**不建窗口** | 11.50 / 12.08 | +0.4 / +0.9 | `adapter_mirrors_the_registry_one_to_one` / `control_tree_json_is_stable_and_round_trips` |
| 2 | **1 个真实 `MainWindow`**（`MinimalSoftwareWindow` 平台）+ `InputContext` | **55.97** | **+44.83** | `ime_event_source_drives_the_input_context_and_swallows_bare_shortcuts` |
| 3 | 1 个真窗口 + `EngineHost`（快照 + SPSC 电平通道）+ 时码读数 | 57.83 / 57.88 | +1.9 | `changing_the_time_signature_changes_the_reading_at_the_same_tick_both_ways` / `timecode_reads_the_projected_time_signature_for_three_signatures` |
| 3b | **2 个**真窗口 + 引擎（`TransportHarness::new()` 建一个，判据再建一个） | 57.62 | ≈ 第 2 行 | `an_unwired_window_changes_neither_the_engine_nor_the_display` |
| 4 | 窗口 + 引擎 + 走带回调接线 + 事件日志 | 81.78 | +24 | `transport_callbacks_really_drive_the_engine_and_the_display_follows_it` |
| 5 | 窗口 + 运行时控件树遍历 + 一帧 1920×1080 Tier-1 软件光栅化 | **138.52** | **+80.64** | `runtime_control_tree_cross_check_against_the_registry` |
| 5b | filled 工程 + 窗口 + 控件树 + 截图 | 114.81 | +57.0 | `project_projection_reaches_the_control_tree_and_the_pixels` |
| 6 | 上述 + **多次**截图 + PNG 证据链（`write_png`） | **157.03** | +18.5 | `live_main_window_renders_tier1_pixels_and_enforces_permissions` |
| 7 | **整套 15 条 Tier-1 判据共用一个进程**的峰值 | **255.25** | +98 | `$BIN`（不带 `--exact`；`15 passed; 0 failed`） |
| 8 | 跨二进制对照：`live_ui_mcp` 的完整控制面（`LiveControlPlane` + 端口适配器） | 143.42 | — | `live_ui_mcp-ed96c9b54ba14366 --exact live_control_plane_reads_the_project_backed_window_end_to_end` |

### 4.1 这张表怎么读（三条不许含糊）

1. **第 2 行是本轮最重要的一个数**：`+44.83 MB` 就是"**真窗口本体**"的代价，而且它是在
   **同一条二进制、同一个进程地板**下做差得到的 —— 排除了"测试框架更重"这个替代解释
   （该二进制的 `--list` 地板 8.33 MB 与 `yeban-app --version` 的 8.53 MB 同量级）。
2. **第 1–3 行与第 4–6 行是两个不同的账**：第 4–6 行里的"运行时控件树遍历 + Tier-1 截图 + PNG"
   属于**UI 测试/`[MUST-GATE-015]` 金标设施**，**发行 app 的空闲路径一次都不调用**
   （`main.rs` 的 GUI 路径只 `build_main_window` + 接线 + `run()`）。
   **别把这 80 MB 记到"空工程空闲占用"上** —— 它记的是"这套设施需要多少内存"。
3. 规范 ① 的对象 = **第 2–3 行**（窗口本体 + 引擎），在 debug 下 **≥55.97 MB**，
   已经 **超 35 MB 目标约 1.6 倍**。这才是"需要工程优化"的量级；
   而台账此前引用的 **73.25 MB 是"故意分配 ~73 MB"的注入对照，不是 app 的读数**
   （`git log -1 --format=%B 448f865` 与 `docs/DEVELOPMENT_LEDGER.md:1519` 逐字如此）。

---

## 5. 归因再下一层：`footprint` 的分类分解

`ps` 在本机受限环境里**不可用**（与 `measure_rss.py` 文件头记的老教训一致，本轮复测：
`ps -o rss= -p $$` 失败），但 **`/usr/bin/footprint` 可用**（实测见 §2.1 的 `Malloc Large 50 MB` 探针）。
用它按"脏页分类"给上面两个区间开箱（峰值采样，**非稳态分解**）：

| 场景 | footprint 峰值 | 其中 `Malloc Large` | 其中 `Malloc Small` | 其余（`__DATA_CONST` / Metadata / Stack / page table…） |
| :--- | ---: | ---: | ---: | ---: |
| **只建一个真窗口**（`ime_event_source_…`） | **34 MB** | 5.44 MB（2 region） | **25 MB**（9 region） | ≈3 MB |
| **窗口 + 控件树遍历 + 一帧光栅化**（`runtime_control_tree_cross_check_…`） | **113 MB** | **82 MB（10 region）** | 26 MB（9 region） | ≈4 MB |
| 整套 15 条判据峰值 | 219 MB | 166 MB（32 region） | 48 MB（21 region） | ≈4 MB |

**读法（每一条都有出处，不含"可能是字体"式猜测）**：

- **窗口本体 = Malloc Small 主导（25 MB / 9 region）** ⇒ 组件树由**大量小对象**构成
  （Slint item tree / properties / 回调 / `SharedString`）。这是"减窗口规模/懒建分支"能碰到的钱。
- **"遍历 + 一帧"这一段是 Malloc Large 主导（82 MB / 10 region，均值 ≈8.2 MB/region）**。
  仓内**已知存在**的同量级大块在 `crates/yeban-ui-test-port/src/render.rs` 的
  `capture()`：`SharedPixelBuffer::<Rgb8Pixel>::new(1920, 1080)`（1920×1080×3 ≈ 5.93 MiB）
  **加上** `buffer.as_bytes().to_vec()` 的同尺寸拷贝（再一次 ≈5.93 MiB）⇒ **≈11.9 MB / 次截图**。
  量级与 region 均值吻合。
  ⚠ **诚实边界**：82 MB 的**逐笔归属我没有验证**（那需要 `malloc` 栈记录 / `MallocStackLogging`）。
  本轮只说到"这一段由大块主导、且仓内有一条每帧 ~11.9 MB 的已知拷贝路径"，
  不声称"82 MB 全是帧缓冲"。
- **`ru_maxrss` 与 `footprint` 不是同一个口径**：同一条判据 `runtime_control_tree_cross_check_…`
  是 `ru_maxrss 138.52 MB` 而 `footprint 113 MB`（差 ≈25 MB）。
  差的来源是"常驻但不可归属"的部分（`__TEXT` 干净页、dyld shared cache 等）。
  ⇒ 引用时**必须写明口径**，两个数不许混着比。

---

## 6. 覆盖缺口清单（规范对象**没有**被覆盖到的部分）

| # | 缺口 | 证据（当场跑过） | 可行动的具体项 | 归属 |
| ---: | :--- | :--- | :--- | :--- |
| G1 | **窗口本体完全没被读数覆盖**：`--headless` 不构造任何 Slint 对象 | `crates/yeban-app/src/main.rs:97-101` + 运行输出里的边界行；§4 的 `+44.83 MB` 差值 | 需要一个**不依赖显示器、可控时长**的真窗口模式，见 needs-3 | app 侧（我） |
| G2 | **"空闲"没被覆盖**：现行命令是**短命**进程（跑完就退），规范要的是**长命进程的空闲值** | `docs/ledger/gate-status.md:42` 自己记的第 ② 条"仍差" | 同上 | app 侧（我） |
| G3 | **现有仪器读不到长命进程**：超时即 `kill` 且**不打印 BENCH 行** | `measure_rss.py --label long-lived-idle --timeout 3 -- /bin/sleep 30` ⇒ `[measure_rss] 超时 3.0s…`、`instrument_exit=3`、**零条 BENCH 行** | needs-1 | 脚本（集成者） |
| G4 | **"空工程"没被覆盖**：`--project-sample` 只有 `default`（=`bridge::demo_project()`，6 轨）与 `filled`；`default` **不是空工程** | `crates/yeban-app/src/cli.rs:203-231`；运行输出 `project-counts: tracks-all=7` | needs-2 | app 侧（我） |
| G5 | **release 列不存在** | 本机 release 需首次全量构建 Slint（纪律禁止）⇒ 见 §8 | needs-4 | CI（集成者） |
| G6 | **CI 上从未取过 ① 的读数** | run `37228045430` 的 bench 日志里 `BASELINE-002` 出现 **0** 次；27 次 `gates-manual` 里 bench 有 **26** 次 `skipped` | needs-4 | CI（集成者） |
| G7 | **README/文档里 `--headless` 被当作"UI 已自检"的读法** | 边界行 `cli.rs:87` 已经自己声明"未构造 MainWindow…" ⇒ 这条**不是**缺口，是**已声明**；列出以免有人把它当 ① 的达标记 | — | — |
| G8 | **噪声/宿主干扰** | §7 判据 ④ 的 600 MB 并发 hog 实测：读数 `11.34` vs 无 hog `11.30–11.33` ⇒ **不受影响**（`RUSAGE_CHILDREN` 只看自己 `wait()` 过的子进程） | — | — |

---

## 7. 判据（8 条）与注入（3 条 + 2 条补充），含**见证**判据

> 本轮**不改 `scripts/**`**（集成者可另行接线）。以下每条都是**当场跑过的命令**，
> 输出逐字抄录；**红→绿**的两条给出注入前后两次读数。

### 7.1 判据

| # | 判据 | 命令 | 本轮实测 | 结论 |
| ---: | :--- | :--- | :--- | :--- |
| ① | 空工程空闲读数 ≤35 MB（真跑） | `measure_rss.py -- $APP --headless` | `11.33 / 11.30 / 11.33` → `within-target` | **绿**（但见 G1/G2：量的不是规范对象） |
| ② | 演示工程读数 | 同上命令 + `--project-sample filled` | `11.33`（default 即演示工程）/ `11.17`（filled） | **绿**；规范对"演示工程"**没有**另给目标 ⇒ 按同一条 35 MB 记 |
| ③ | 测法可复跑（同量级 + 差异被记录） | ① 的三次重复 | 极差 **0.03 MB（0.27%）** | **绿** |
| ④ | 无关进程/宿主噪声被排除 | 起一个常驻 ~600 MB 的无关进程，再跑 ① | `peak_rss_mb=11.34`（与无 hog 的 11.30–11.33 同量级） | **绿**；并额外证明 `ru_maxrss` **只看自己 wait 过的子进程** |
| ⑤ | 优化/测量不改变行为（既有 Tier-1 与契约判据全绿） | `run-gates.sh light` + Tier-1 全套 + 模型全套 | light `门禁通过 (mode=light)` exit 0；`15 passed; 0 failed`；`107 passed; 0 failed`（三条 `child_exit=0`） | **绿**（本轮**零生产代码改动**，故为恒等变换） |
| ⑥ | 读数在**默认 feature** 与 **CI** 上可得 | 本机：读得到（debug，见 §2.3）；release：**本机不可得** | 见 §8 | **半绿**：默认 feature 可得（debug）；**release + CI 列 = SKIP + needs-4** |
| ⑦ | 内存增长不是"延迟到下一次分配才发生"（空转 N 秒不攀升） | 需要**长命**进程 + 不等待的采样 | 现有仪器**读不到长命进程**（G3：exit 3、无 BENCH 行） | **SKIP（如实）**：仪器与 app 侧都缺一环 ⇒ needs-1 + needs-3 |
| ⑧ | 门禁：`run-gates.sh light` | `bash scripts/gates/run-gates.sh light` | `门禁通过 (mode=light)`，exit **0** | **绿** |

**每条读数都带"被测量命令的原始退出码"**：`measure_rss.py` 把子进程退出码**透传**
（`return code`），且 BENCH 行里带 `child_exit=`。§4 全部 13 行都是 `child_exit=0`
⇒ "内存读到了"不会被误当成"判据通过了"。

### 7.2 注入（**必红→必还原**）

| 注入 | 做了什么 | 期望 | 实测（逐字） | 还原 |
| :--- | :--- | :--- | :--- | :--- |
| **J1 放大常驻** | 让被测量命令故意常驻 70 MiB | ① 变红 | `BENCH … label=INJECT-70MiB peak_rss_mb=82.09 … verdict=over-target` | 立刻换回 app：`label=INJECT-restored peak_rss_mb=11.34 … within-target` ⇒ **红→绿** |
| **J2 被测命令失败** | 让被测量命令 `exit 7` | 仪器**不得**只报内存而吞掉失败 | `BENCH … label=INJECT-child-fails peak_rss_mb=11.77 … child_exit=7 …`；`instrument_exit=7` | 换回 app：`child_exit=0`、`instrument_exit=0` |
| **J3 仪器空转（见证）** | 把被测对象从"什么都不做"逐级换成"真窗口""真窗口+一帧" | 读数**必须**随对象单调抬升；若读数函数返回常数则必红 | `1.05（/bin/echo）→ 11.33（app）→ 55.97（1 个真窗口）→ 157.03（窗口+多帧光栅化+PNG）` | 每一步都是**当场跑过的命令**；这 4 个数就是"仪器真的读到了非平凡的值"的机械保证 |
| J4（补充） | 先测大子进程、再测 `tiny` | 第二条**必须**粘住高水位（证明"一进程一命令"是承重的） | `after BIG child: 82.30 MB` → `after TINY child: 82.30 MB` | — |
| J5（补充） | 测一个长命空闲进程 | 现行仪器**读不到** | `--timeout 3 -- /bin/sleep 30` ⇒ `instrument_exit=3`、**无 BENCH 行** | — |

> **J3 是本项目 `MUST-GATE-001` 注入 I4 换来的硬要求的落地形态**：
> **每一条"断言 ≤ 阈值"的判据，都必须同时断言"仪器真的读到了与机体相关的非平凡值"。**
> 本轮的"见证"就是 §3.2 的逐层抬升表 —— 它把"仪器不空转"从一句承诺变成 4 个当场跑出来的数。

---

## 8. 本机 vs CI（**严格区分，不许互相冒充**）

| 维度 | 本机（Apple M2，macOS） | CI（ubuntu-latest） |
| :--- | :--- | :--- |
| 构建档 | **debug**（复用主仓已有缓存，本轮零编译） | `--release` |
| `yeban-app --headless` 读数 | **11.30–11.34 MB**（实测，3 次） | **不存在**（G6：bench 那一步从未跑过） |
| 真窗口读数 | **55.97–57.88 MB**（`ru_maxrss`）/ 34 MB（`footprint`），debug | **不存在** |
| 能否给出"≤35 MB"判决 | **不能**（还不是规范对象；且只有 debug） | **不能**（从未取过） |
| 平台噪声 | 已排除（判据 ④） | 托管 runner 绝对值只作数量级（`measure_rss.py` 文件头已声明） |

**"达标判定要在规范指定的参考机上复跑"这句话，到目前为止一次都没有被执行过** ——
既不在本机（对象不可得），也不在 CI（那一步从未 dispatch）。这不是推卸，是**可复核的事实**：

```text
$ gh api /repos/gradetwo/yeban/actions/workflows/gates-manual.yml/runs?per_page=60 --jq '.workflow_runs[].id'   # 27 个 run
$ # 逐个查 bench job：26 个 conclusion=skipped（steps=0），只有 37228045430 是 success（steps=9）
$ gh run view 37228045430 --log > /tmp/run-37228045430.log
$ grep -c "峰值常驻内存" /tmp/run-37228045430.log   →  0
$ grep -c "BASELINE-002"   /tmp/run-37228045430.log   →  0
$ git log -1 --format='%h %ad %s' --date=iso 448f865
448f865 2026-10-05 08:28:35 +0800 feat(bench): BASELINE-002 首个内存测量工具与读数…
```

⇒ 该 run（2026-10-04T19:22Z）**早于**引入 BASELINE-002 那一步的提交 ⇒ **日志里当然没有它**。

---

## 9. 结论：`BASELINE-002` 应记 **部分**（不是达标），凭什么

**记"部分"的三条理由，每条都有上面的实测支撑：**

1. **量法没覆盖规范对象**（G1/G2/G4）。规范 ① 问的是"**空工程空闲**常驻内存"
   即"一个活着的 DAW 进程，带窗口、带引擎、什么都不做"；现行判据量的是一个
   **不构造任何 Slint 对象、跑完即退**的进程，而且加载的是**演示工程**不是空工程。
   §4 把这两个对象的差值量成了 **+44.83 MB（同二进制消融）**。
2. **规范对象在 debug 下已超目标**：真窗口本体 `55.97 MB` > 35 MB。
   ⇒ 若 release 仍 >35 MB（**未知，见理由 3**），则这不是"改口径"能解决的，必须做工程
   （§5 已定位第一抓手：窗口本体的 Malloc Small ≈25 MB / 9 region；
   Tier-1 截图设施每帧 ≈11.9 MB 的已知拷贝，但那**不进发行空闲路径**）。
3. **release 列与 CI 列都不存在**（G5/G6）。本机因纪律不构建 release；CI 的 bench 档
   自 BASELINE-002 那一步加入后**从未 dispatch**。所以"是否达标"目前**在物理上没有依据**。

**什么时候可以改判**：① 出现 `gate=bench` 的一次 CI 判决（给出 release 列）；
② app 侧出现一个能构造真窗口并可空转 N 秒的路径（needs-3），于是 ⑦ 与"空闲"同时可得；
③ 用同一台参考机给出 **window-only** 读数 ≤35 MB。

---

## 10. needs（交给集成者，**每条都带证据与归属**）

| # | needs | 证据 | 归属 |
| ---: | :--- | :--- | :--- |
| 1 | `measure_rss.py` **读不到长命进程**：超时即 `kill`，且**不打印 BENCH 行**（`return 3` 早于打印）。规范要的是"空闲常驻"，需要一条"**采样而不等待**"的路径（新增 `--sample-seconds`：`SIGKILL` 之后**仍然打印** BENCH 行；或允许外部采样） | `measure_rss.py:78-82` + 实测 `--timeout 3 -- /bin/sleep 30` ⇒ exit 3、零 BENCH 行 | 脚本（集成者） |
| 2 | `--project-sample` 缺 `empty`。规范字面是"空工程"，而 `default` 是 `bridge::demo_project()`（6 轨） | `cli.rs:203-231`；实测 `tracks-all=7`；模型侧空工程样本实测 3.06 MB / 数据 ≈0.5 MB | app 侧（我，**需授权**） |
| 3 | 缺一个**不依赖显示器、可控时长**的真窗口空闲模式（`--headless-idle --idle-seconds N`）。**零新增依赖**：`yeban-app` 已启用 `slint` 的 `renderer-software`，`slint::platform::software_renderer::MinimalSoftwareWindow` 直接可用；仓内已有同款实现可参照（`crates/yeban-ui-test-port/src/render.rs:129-180`），**唯一注入点**仍是 `host::build_main_window`（D28） | `slint-1.18.1/lib.rs:485-492` 的 `pub mod software_renderer`；`crates/yeban-app/Cargo.toml` 已列 `renderer-software` | app 侧（我，**需授权**） |
| 4 | **需要一次 `gate=bench` 的 CI dispatch**：这是 release 列的唯一来源，也是 §8 那张表里三个"不存在"的唯一解 | 27 个 run 里 bench 有 26 次 `skipped`；唯一跑过的 37228045430 早于 `448f865` | CI（集成者） |
| 5 | **结论级**：**"≤35 MB 在当前架构下是否可达"本轮无法结论。** 能说的只有：debug 下**真窗口常驻 ≥55.97 MB**（`ru_maxrss`）/ **34 MB**（`footprint` 可归属部分）⇒ **debug 已超**；release 未知。若 release 仍 >35 MB，第一抓手是 §5 的窗口本体（Malloc Small 25 MB / 9 region）与组件树规模，而不是模型层（全模型套 7.72 MB） | §3.2 / §4 / §5 | 下一轮 |

---

## 11. 地盘声明与修改文件

| 文件 | 状态 | 净行数 |
| :--- | :--- | ---: |
| `docs/ledger/baseline-memory-notes.md` | **新建**（本文件） | 见提交里的 `git diff --cached --stat` |

**生产代码：零改动**（`crates/yeban-app/**`、`crates/yeban-engine/**`、`crates/yeban-model/**` 一个字节都没动）——
本轮使命是"先测量、把差距归因到可行动的具体项"，任何没有读数支撑的改代码都会污染基线。
**禁改目录**（`scripts/**`、`.github/**`、`schemas/**`、根 `Cargo.toml`/`Cargo.lock`、
`deny.toml`、`docs/adr/**`、`docs/YEBAN_*.md`、`docs/DEVELOPMENT_LEDGER.md`、`README*.md`、
法务文件、`docs/ledger/{gate-status,phase-status,feature-alignment,human-decisions}.md`）**均未触碰**。
