# ADR-0056 — BASELINE-002（常驻内存基线）的测量定义钉死：二进制 / 样本 / 度量三变量

- **状态**: **Proposed（待人类批准；本票不改代码、不改 ROADMAP 行、不改测量脚本）**
- **决定者**: 待人类裁决
- **关联**: 审评 2026-10-31 §6.2（空工程 `--headless-idle` 53.1 MB vs BASELINE-002 ≤ 35 MB）；
  `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:357`（BASELINE-002 行）；
  `docs/adr/ADR-0002-baseline-verdict-hardware.md`（判定硬件范围仍 Proposed，待 HD-49/HD-58）；
  `docs/ledger/baseline-headless-idle-notes.md`（`--headless-idle` 取样对象）；
  `docs/ledger/baseline-memory-notes.md` §3.2；
  `scripts/gates/measure_baseline_002.sh` + `scripts/gates/measure_rss.py`；
  `docs/DEVELOPMENT_LEDGER.md:2453`（旧 concession：量法尚未覆盖规范所指的对象）
- **日期**: 2026-10-31

## 背景（全部现读，⛔ 不采信报告的措辞）

规范行（ROADMAP:357）只给三样东西：对象「空工程空闲内存占用」、
方法「操作系统内存工作集（Working Set）统计」、门限「≤ 35 MB」。
它**没有**钉死三处决定读数可比性的变量：

1. **二进制**：哪个 profile、哪组 features。脚本头写「debug 也可以」——
   而 debug 与 release 的常驻差一个量级；审评的二进制还带了
   `in-process-mcp` + `ui-mcp-http`（**非官方缺省**：AGENTS.md §2 红线 6
   规定官方缺省发行版不得默认启用 `mcp-http` / `ui-mcp`）。
2. **样本**：脚本用 `--project-sample empty`（空工程）—— 这一点脚本已钉死，
   但规范行没写；审评的「空工程」与脚本的空工程是不是同一个样本，
   读规范行无法断定。
3. **度量**：脚本读的是 `resource.getrusage(RUSAGE_CHILDREN).ru_maxrss`
   （子进程峰值高水位）；审评报的是 VmRSS 中位数 + ru_maxrss 峰值；
   规范行写的「Working Set」在 Linux 上没有唯一对应的计数器。
   三个数不是同一个量，**不能直接比**。

现状因此是：审评的 53.1 MB（`--headless-idle` 空工程，VmRSS 中位数 +
ru_maxrss 峰值，非缺省 features）与门限 35 MB **无法判定**——
既不能判超标，也不能判达标。同样，台账里那组本机 debug 读数
（`--headless-idle` 空工程 34.34 / 34.28 MB，6 轨演示工程 35.2 MB）
是 **debug**  profile 下的，也不能直接当 release 口径的证据。
而 `DEVELOPMENT_LEDGER.md:2453` 的旧 concession（「量法尚未覆盖规范所指
的对象」）已被 `--headless-idle` 线的落地**部分推翻**：取样对象现在有了，
缺的是下面这张定义表。

## 决定（Proposed）

BASELINE-002 的**可比测量**定义为以下五条（门限本身不动，仍是 ≤ 35 MB）：

1. **二进制**：`cargo build --release -p yeban-app` 的**官方缺省**构建
   （不启用 `mcp-http` / `ui-mcp` / 任何 `experimental-*` features；
   启用它们时的读数必须**另行标注 features**，不得与本口径的数并列）。
2. **样本**：`--project-sample empty`（脚本现用的空工程）。
3. **路径**：`--headless-idle --idle-seconds N`（真 `MainWindow` + 逐行光栅化
   一帧当见证，空闲 N 秒；`--headless` 只报进程地板，**不是**规范对象）。
   `IDLE_SECONDS` 缺省 5（脚本现值）；改 N 时必须在读数旁注明。
4. **度量**：`measure_rss.py` 报的 `ru_maxrss` **峰值**（子进程高水位）。
   VmRSS 中位数、瞬时采样、其它工具的 Working Set 读数，**不是**本口径，
   只能作对照附记，不得直接与 35 MB 门限比。
5. **判定硬件**：**不由本票定**。沿用 ADR-0002 的待裁决状态（参考机 vs
   托管 runner，待 HD-49 / HD-58）：在人类裁决前，托管 runner 的读数
   一律是**量级证据**，不得作门禁判决；本票只钉死「同一台机器上可比」的那四条。

## 本票不做的事（⛔ 不是遗漏）

- **不改 ROADMAP:357 那一行**：规范行的措辞改动按 AGENTS.md §5.7 归人类批准；
  本票获批后，由集成者把上面五条写进那一行的「测量方法与测试规程」列。
- **不改 `scripts/gates/measure_baseline_002.sh`**：`scripts/**` 是集成者独占文件；
  本票获批后，由集成者把「debug 也可以」改成「判定用 release 缺省构建」，
  并在脚本头贴出本 ADR 的五条。
- **不更新 `DEVELOPMENT_LEDGER.md:2453` 的旧 concession**：同属集成者独占；
  获批后由集成者把那条改成「取样对象已落地（`--headless-idle`），
  定义见 ADR-0056，待按定义重测」。
- **不给审评的 53.1 MB 下 verdict**：非缺省 features + 非本口径度量 ⇒
  它在本定义下**不可比**；要 verdict，必须按上面五条在同一台机器上重测。

## 获批后要补的一次重测（不由本票跑）

按上面五条，在判定硬件（或其量级替代）上跑一次
`measure_baseline_002.sh -- <release 缺省二进制>`，把 ②（空工程 idle）
的读数与 35 MB 门限并列记录；6 轨演示工程的 35.2 MB（debug）也按
release 缺省重测一次—— 它此前已 over-target，release 下是过还是不过，
**现在不知道**，不许沿用 debug 数猜。
