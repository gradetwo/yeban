# 夜半 (Yeban) 开发工作流

本文件把 `docs/skills/yeban-dev-workflow/SKILL.md` 的规则落成**这个仓库里可执行的流程**。
SKILL 讲"为什么"，这里讲"怎么敲"。

---

## 0. 两条不可协商的前提

1. **本机是 Apple M2 开发/编辑机，不是构建农场。**
   全量 workspace 构建、Slint / cpal / symphonia 编译、基准、模糊测试、跨架构对账**一律在 CI 上跑**。
   本机只允许：`cargo fmt`、**单个无重依赖 crate** 的 `clippy/test`、全部守卫脚本。
2. **只有 CI 的判决算"绿"。** 本地绿是参考；未读取的 CI 判决一律记为 `pending`，不许写成"通过"。

**一条 land 的准入检查（两次实测事故之后立的规矩）**：`worktree.sh land <line>` 会合并分支，
而随后的 `rm --purge` 会**删掉工作树目录**。两次事故都是"线还在活跃时就被 land"：
一次是把该线正要用的树删了（它的 notes 回填没能提交），一次是合并后又出现新 tip 造成判决归属歧义。
⇒ **只有在同时满足三条时才 land**：① 该线**已报告完成**（或至少已明确暂停）；
② 它的工作树 **`git status --short` 干净**；③ 它**没有未读判决**。
另：合并一条线之后**不要**再往那条已合并的分支推提交 —— 需要补文档就**由集成者在 main 上补**
（`docs/ledger/mcp-no-compat-notes.md` §7.3 就是这么补的）。

这两条不是自律要求，是被脚本机械执行的。**"哪些活能在本机跑"必须由依赖图决定，不由自觉决定** —— `yeban-mcp` 自己的清单里一个重依赖都没有，但它依赖 `yeban-render` ⇒ 传递拉进 `rayon`/`hound`/`midly`；只看本 crate 清单的实现会**在本机真的编译**它们（由 `line/mcp-render` 的 needs-7 发现，第 7 轮已修）：

| 工具 | 作用 |
| :--- | :--- |
| `scripts/dev/cargo-local.sh` | 本机 cargo 包装器：拒绝 `--workspace`/`--all`，重定向 `CARGO_HOME` 到工作区内 |
| `scripts/gates/run-gates.sh light` | 格式 + 14 条机械红线守卫 + 文档链接门禁 + 许可清单检查，零编译，任何机器都能跑 |
| `scripts/gates/run-gates.sh crate <name>` | 在 light 基础上加该 crate 的 clippy/test；**若该 crate（传递地）含重依赖则自动拒绝并交给 GitHub CI**（判定见 `scripts/dev/heavy-deps.py`：0=含重依赖 / 1=不含 / 2=无法判定⇒保守跳过）。重依赖**挂在 feature 后面**的 crate（如 `yeban-engine` 的 cpal）不跳过，而是自动改用 `--no-default-features` 的**本机轻量变体**真跑（D19） |
| `scripts/gates/run-gates.sh full` | 全量门禁；只允许在 CI 上跑（本机需 `YEBAN_ALLOW_HEAVY=1` 显式放行） |
| `scripts/dev/ci-verdict.sh [branch]` | 把 CI 判决读回来（公开仓库匿名 REST API 即可） |

> **沙箱/受限环境提示**：若 rustup 无法写 `~/.rustup`（例如在受限沙箱中运行 Agent），
> `cargo-local.sh` 会设 `RUSTUP_TOOLCHAIN=stable` 复用已安装的同版本工具链，并把 `CARGO_HOME`
> 指向 `/Users/crow/work/music/.cargo-home`。在普通终端里直接 `cargo` 即可，rustup 会按
> `rust-toolchain.toml` 自动装好钉死的 `1.99.0`。

### 本机跑开源合规门禁（cargo-deny，零编译）

官方提供预编译二进制，**不要**为了它在本机做一次 `cargo install`（那是重编译，违反本机纪律）：

```bash
mkdir -p /Users/crow/work/music/.tooling && cd /Users/crow/work/music/.tooling
curl -sL -o cd.tgz https://github.com/EmbarkStudios/cargo-deny/releases/download/0.20.2/cargo-deny-0.20.2-aarch64-apple-darwin.tar.gz
tar xzf cd.tgz && rm cd.tgz

cd /Users/crow/work/music/yeban
YEBAN_CARGO_DENY=/Users/crow/work/music/.tooling/cargo-deny-0.20.2-aarch64-apple-darwin/cargo-deny \
CARGO_HOME=/Users/crow/work/music/.cargo-home RUSTUP_TOOLCHAIN=stable \
  bash scripts/gates/run-gates.sh deny
```

背景（为什么值得加这一档）：首次推送 CI 时 `deny` job 变红，原因只是 `deny.toml` 的 TOML 表位置写错，
而当时我判断"本机装不了 cargo-deny"——那个判断是错的。见 `docs/DEVELOPMENT_LEDGER.md` 第 6 节 L2/L4。

---

## 1. 多条工作线并行：一树一线，一文件一写者

```text
yeban/                        # main 工作树（集成）
├── .worktrees/               # 各条工作线的独立工作树（已 gitignore）
│   ├── model-core/           # line/model-core
│   ├── dsp-core/             # line/dsp-core
│   └── ui-shell/             # line/ui-shell
```

开一条线：

```bash
scripts/dev/worktree.sh add model-core          # 基于 main 建 line/model-core
cd .worktrees/model-core
bash scripts/gates/run-gates.sh light           # 格式 + 红线守卫
bash scripts/dev/cargo-local.sh test -p yeban-model
git push -u origin line/model-core              # 推送即自动触发 CI
```

落地与废弃：

```bash
scripts/dev/ci-verdict.sh line/model-core       # 先读判决，再谈合并
scripts/dev/worktree.sh land model-core         # --no-ff 合并进 main（保留线史）
scripts/dev/worktree.sh rm model-core --purge   # 显式废弃：删树 + 删分支
```

**硬规则**

- 两条线**永不**共用一棵工作树；一个文件同一时刻只能有一个写者。
- 工作线**短命**：小步落地，提交越小、变红时追查越短。
- 被取代的线**显式废弃**（打标签 → 打包 → 删分支）。陈旧分支是负债，不是备份。
- 冲突高发文件（根 `Cargo.toml`、`.github/**`、`scripts/**`、`docs/DEVELOPMENT_LEDGER.md`）
  由**集成者独占**；工作线通过新增目录/新 crate 来扩展，不改共享文件。这就是
  `members = ["crates/*", "spikes/*"]` 用 glob 的原因。

### 生成物冲突：不要手工合并，要重新生成

有些文件是**从真实状态生成的**，两条线同时改它们必然冲突（`docs/ledger/dependency-licenses.md`
已经真实撞过一次，`Cargo.lock` 同理）：

| 生成物 | 生成方式 | 冲突时的正确处置 |
| :--- | :--- | :--- |
| `Cargo.lock` | `cargo metadata`（或任意 `cargo` 命令） | 取一侧后重新生成，再断言 `cargo metadata --locked` 通过 |
| `docs/ledger/dependency-licenses.md` | `python3 scripts/gates/license_inventory.py` | 直接重新生成，然后 `--check` 通过 |

政策：**工作线只需保证自己分支上自洽**（自己能过 `--check`）；**集成者在每次改动依赖图的合并之后
统一重生成一次**。工作线不需要预测 main 的未来状态，也不应该手工合并这两个文件。

这条政策已经固化进工具：`scripts/dev/worktree.sh land <line>` 在合并成功后会自动跑一次
**合并后自检**（`cargo metadata --locked` + `license_inventory.py --check`），任一项不过就提示**先修再推**；
若合并本身冲突，它会直接打印上面三步的处置命令而不是让你手工合并生成物。

---

## 2. 单次变更的固定动作（顺序不能换）

1. **先测量**：读真实字节、真实文件、真实数字，把数字记进 `docs/DEVELOPMENT_LEDGER.md`（含测法与时刻）。
2. **选最小改动**，并说清它的**边界**——这次改动**不**证明什么。
3. **先写判据**，然后故意把代码改坏看它变红，再改回来。从没红过的判据是"穿着测试外衣的注释"。
4. **门禁**：`cargo fmt --check`；本机跑该 crate 的 clippy/test；跑**搜索出来的**那一族检查，不是凭记忆的那一族。
5. **提交**：信息里写明"什么没做"；推送；**读判决**。
6. **红**：读门禁自己说的话，修，重复。**绿**：把提交号 + 判决记进账本。
7. **没被证实的**一律进 `needs`/`pending` 清单，**绝不静默降级**。

四条特别容易违反的操作纪律：

- **绝不管道化门禁**：`gate | tail` 报的是 `tail` 的退出码。需要日志就 `set -o pipefail` + `tee`。
- **改了计数就要跑推导计数的检查**：加一个导出可能让"七种格式"这句话变假。
- **受影响集合要搜出来**，不能凭记忆（`scripts/dev/changed-crates.py` 就是干这个的）。
- **提交前跑类型检查，而不是用眼睛看**：打印出来再提交，就是放着一个类型错误进了主干。

---

## 3. 需求 ID 闭环

每个变更必须能指出它实现/修改了哪些规范 ID（`ARCH-*` / `MODEL-*` / `MCP-*` / `UI-*` / `ROAD-*`），
并在测试里留下对应映射。做不到就用 `needs` 说明为什么做不到。

提交信息模板：

```text
<主体: 做了什么> [<规范 ID>]

做了什么:
- ...

边界 (这次没有证明什么):
- ...

判据:
- <测试名 / 脚本>  (曾经在 <某次故意改动> 下变红)

needs / pending:
- ...
```

---

## 4. 目录约定

```text
yeban/
├── Cargo.toml              # workspace 唯一清单；依赖版本的唯一事实源（集成者独占）
├── Cargo.lock              # 必须提交（GPLv3 源码可追溯 + 可重现构建）
├── rust-toolchain.toml     # L1 确定性要求的工具链钉死
├── deny.toml               # cargo-deny 许可与依赖审计
├── AGENTS.md               # AI Agent 执行契约（红线清单）
├── crates/                 # 产品 crate，每个目录一个 crate（glob 成员）
├── spikes/                 # Phase 0 的一次性可行性验证，各自独立（glob 成员）
├── assets/                 # 品牌母版、字体/模型/采样登记表
├── schemas/                # JSON Schema 机器校验契约
├── scripts/
│   ├── dev/                # 本机工具：cargo 包装器、worktree、CI 判决、受影响集合
│   ├── gates/              # 门禁：run-gates.sh、schema 校验
│   ├── guards/             # 机械红线守卫（policy_check.py）
│   └── brand/              # 品牌资产再生成
├── docs/                   # 见 docs/README.md 的效力分级
│   ├── adr/                # 裁决留痕
│   └── ledger/             # 复用审计等账本附件
└── .github/workflows/      # 自动档 ci.yml / 手动档 gates-manual.yml / 官网 site-deploy.yml
```

**`crates/` 与 `spikes/` 下的每个目录都必须含合法 `Cargo.toml`**——glob 成员要求如此，
守卫 G09 会拦住"建了目录忘了清单"这种低级错误。

---

## 5. 人类审查关卡（Agent 不得代做）

`ROAD-M-1-006` 规定 4 项必须由人完成的动作：

1. 法务确认 GPLv3 §7 CLAP 例外条款措辞与 Slint 双授权声明；
2. 法务确认 ASIO SDK 的 GPLv3 选项与 Windows 分发策略；
3. `夜半 / Yeban` 在 USPTO / EUIPO / CNIPA 类 09、42 的商标查重；
4. BDFL 核对 `cargo deny` 报告与 `assets/samples/ATTRIBUTION.md` 后签署公开发布授权。

此外 `AGENTS.md` §2 红线 1 规定：未经人类负责人明确书面指示，Agent **不得**修改
`LICENSE`、`LEGAL.md`、`SECURITY.md`、`TRADEMARK.md`、`GOVERNANCE.md`、`NOTICE.md`。

### 口径数字必须可机械复核

**规则**：账本/状态表里写下的**数字类事实**（有几条工作线、几个 crate 有实现、门禁分布、远程有哪些分支…）
必须能被**一条只读命令**重新导出，否则它们会在我看不到的地方变成谎言。
- `bash scripts/dev/project-counters.sh` —— 治理计数（crate 规模 / ADR / 教训 / 归档标签 / 守卫 / 台账 / 提交 / 远程分支 / 门禁分布）；
- `bash scripts/dev/branch-hygiene.sh` —— "已合并的 `line/*` 是否还留在 origin"；
- `python3 scripts/gates/check_gate_status.py --summary` —— 门禁状态表（唯一事实源）。
实测漂移三次（归档标签 20→33、真有实现的 crate 13→11、"远程只剩 main+website"而实际 12 条待清），
所以这条不是洁癖：**人写的口径会漂移，机器写的不会。**

### CI 并发是**稀缺共享资源**：别用垃圾运行堵住它（第 14 轮实测）

**实测读数**（第 14 轮）：`in_progress` 长期只有 **1** 个，而 `queued` 一度到 **8** —— 其中 3 条是
**我已经删掉的测试分支**（`line/INJECT-hygiene` / `line/INJECT-h2` / `line/mcp-no-compat`）留下的排队运行。
也就是说：**我自己的临时分支在占用其他工作线的判决时间。**

**规则**：
1. **测试用的临时分支**：推送后**立刻取消它的 run**（`gh run cancel <id>`）**再删分支** —— 只删分支不会清掉已排队的运行；
   （若能在**本地裸仓库**上做这类端到端测试，就不要推 origin。）
2. **不要用零散推送刷 run**：`ci.yml` 的 `cancel-in-progress` 会取消同一 ref 的上一轮，但仍然要**占用排队槽位**。
   合并/改文档应当**攒成一批**再推（这一条同时服务 L23/L26 的"判决归属"纪律）。
3. **手动档与工作线判决抢同一个池子**：像 `fuzz`（上限 180 分钟）这样的长作业会**长时间占住一个 runner**，
   于是 4 条工作线的判决只能排队。⇒ **派发长手动档之前先看队列**；需要工作线判决时优先让队列空出来。
4. 这条与 `HD-38`（是否投入自托管固定频率 runner）**直接相关**：托管 runner 的并发上限是**真实瓶颈**，
   不只是"基准数值不准"的问题 —— 账本里已把它记成"预算决策"，现在它又多了一条论据。

### 推 main 的纪律（与工作线同一条，L32）

**推 main 之前先看 queue**：`gh run list --workflow=ci.yml --branch main --limit 3`。
若已有运行在排队/进行中，**要么等它出结论，要么把改动攒进下一批** —— 否则 `cancel-in-progress`
会让"排队 → 取消 → 再排队"循环下去，而**取消链上的 main 状态是未知的**（不是绿也不是红）。
实测代价：我让 main 累计 **28 个提交没有判决**，而当时我还以为"只是文档与脚本改动"——
但 `scripts/**` 会被 CI 的 `checks` job 真的执行，`.github/**` 更是直接决定 CI 行为。
**一次推送 = 一批改动。**

### 开线三步（缺一步就会出事）

**① 写 brief → ② 建工作树 → ③ 启动 subagent。** 顺序不能颠倒，也不能省第 ② 步。
- 第 ② 步的命令是 `bash scripts/dev/worktree.sh add <line> origin/main`。
  我第 34 轮开 `transport-engine` 时只做了 ①③，**忘了 ②** —— 幸好该线的工作树在我检查前已由别处补上，
  但那次如果我据此判断"线没起来"，就会得出**完全错误**的结论（我当时的输出确实写着"无树"）。
- ⇒ **状态检查必须区分"工作树不存在"与"检查写错了"**：先 `git worktree list` 看**注册表**，
  再 `cd` 进去看分支/脏树 —— 单看某一个路径的 `cd` 失败不足以断言"线不存在"。

### 工作树卫生（与分支卫生配套，第 35 轮清理）

**已退役的线不该留工作树目录**。实测积了两个：`.worktrees/engine-mix`（**已注册**、分支 `line/engine-mix`、
干净、已并入 main）与 `.worktrees/model-no-compat`（**未注册**的残留目录、停在 `main`、干净）。
两者都已清理，现在工作树 = `main` + 两条活跃线 + `website`。
**清理前必须逐条确认**（这一条是血的教训，见"land 准入检查"）：① 分支**已并入 main**；
② `git status --short` **干净**；③ **没有未读判决**；④ 若是"未注册的残留目录"，先用
`git worktree list --porcelain | grep -c <名字>` 确认它**不在注册表里**再删。

**退役一条线 = 四件事，缺一件就会留下不一致**（第 36 轮补上的第 ④ 件）：
① `land`（合并进 main）；② 打 `line-archive/<line>` 标签；③ 删本地分支 + 工作树；
④ **`git push origin --tags`**（否则归档标签只在本地，而它正是"删掉分支后历史仍在"的唯一凭据）。
实测两处漏洞：`line/engine-mix` **已合并却从未打归档标签**（我正在清理工作树时才发现，已补 `8b36e30`）；
`line-archive/phase-status` 与 `line-archive/schema-ratchet` **打了但从未推送**（远程没有）。
⇒ 现在归档标签 **40 个**（远程与本地一致），本地 `line/*` 分支只剩**活跃线**。
**核对口径**（三方一致才算干净）：`git worktree list | grep -c 'line/'` == `git branch --list 'line/*' | wc -l`
== 远程 `line/*` 的条数。

### 开线之前的"三问"（第 49 轮加的）

在写 brief 之前，对"我要补的那个能力"必须问完三件事，否则会开挖重复的线：
1. **它到底有没有实现？**（`grep` 类型/函数名，而不只是 `grep` 关键词）
2. **谁在调用它？**（`grep -rn` 全仓调用点；没有调用点 ≠ 没有实现 —— 见"零消费者"那族）
3. **有没有判据在断言它？**（`tests/`、`verify/`、CI 脚本里是否已被覆盖）

实测反例：我准备开 `golden-compare`（以为 `UI-MCP-003` 的 SSIM 比对没实现），
查完发现 `ssim.rs`（`SSIM_THRESHOLD = 0.98` + 窗口化 MSSIM + 拒绝默认值）与 `golden.rs`
（分平台路径形状 + 平台标签校验）**都已实现且被 `test_port_adapter.rs` 真实调用**，
**缺的只是基线 PNG 本身（人类素材）** ⇒ 那条线是**纯重复**，已撤掉。

## 开发过程中的性能关注（响应耗时 / CPU / 内存）—— **负责人裁决（第 86 轮）**

> 原文口径：**"开发过程中响应耗时、CPU 和内存占用也要关注，统计分析大头，然后优化大头（小投入大回报类型）。"**

### 1. 每次能力切片都要问三个数（而不是只在门禁里问）
| 维度 | 在本仓怎么量 | 已有工具/基线 |
| :--- | :--- | :--- |
| **响应耗时** | 端到端时延的 p50/p99（不要只报均值） | `BASELINE-004`（单步撤销时延 p99）、`spikes/` 的时延 harness、Tier-1 判据的运行时长 |
| **CPU** | 吞吐/实时余量（每量子耗时 vs 缓冲时长）、热点函数 | `BASELINE-001`（离线渲染吞吐）、`crates/yeban-render/examples/bench_render.rs`、`gates-manual.yml` 的 `bench` 档 |
| **内存** | **工作集高水位（`ru_maxrss`）** | `scripts/gates/measure_rss.py`（`--label` 为并排对照准备）、`BASELINE-002`（空工程空闲 ≤ 35 MB） |

### 2. 方法论：**先统计归因，再优化大头**（顺序不许颠倒）
1. **必须用数字归因**：把总耗时/总内存拆到具体子系统（渲染、音频线程、UI 重绘、模型/`BTreeMap`、字形缓存、分配器 arena、线程栈…），
   每项都要有**实测或结构性证据**；**禁止**写"可能是字体/可能是缓存"这种猜测。
2. **同一条二进制内消融**（本仓已验证有效）：让被测对象与地板在**同一个进程**里量，用**差值**抵消进程地板噪声
   —— 例：`--headless`(11.14 MB) → 建一个真 `MainWindow`(55.97 MB) → 加控件树+一帧(138.52 MB)，
   差值才是"这一块的代价"。
3. **只优化占比最大的那一两块**（**小投入大回报**）：把力气花在**排序后的头部**；对尾部（<5% 或落在噪声内）**明确放弃**并写下理由。
4. **也要记录"否定性结论"**（本仓实例）：模型全套 107 条判据只有 **7.72 MB** ⇒ **别往模型/`BTreeMap` 里找钱**；
   一条把整类方向关掉的结论，和一条优化同样值钱。
5. **优化必须留下前后对照**：同一条命令的优化前/后读数；`BASELINE-00x` 有阈值时给达标/未达标。
6. **不许用"换口径/换度量方式/去掉必要功能"来达标** —— 那与"空心绿"同类。
7. **低余量/低负载测不出来就换条件**：低频缺陷常常只在**带负载**时现形（本仓实例：某判据在轻载 3/3 绿、
   在 4 个 CPU 打满进程下第 14 次才红）⇒ 关键路径的性能/竞态判据**应带负载复跑**。
8. **每个"小于阈值"的断言都要有见证**：必须同时断言"仪器真的读到了**非平凡**的值"，
   否则读数函数返回常数时，所有 0 也会是 0（本项目 `MUST-GATE-001` 的注入 I4 换来的硬要求）。

### 3. 什么时候做
- **在本机可做**：纯逻辑/无重依赖 crate 的时延与内存；`measure_rss.py`；`--no-default-features` 的引擎侧读数。
- **必须交 CI**：`--workspace` 全量、`criterion`/`iai-callgrind` 基准、release 列、跨架构 —— 见 `docs/CI_CD.md` 与 AGENTS.md §5。
- **写进切片交付物**：任何切片报告都要含"**这三个数**：时延 / CPU（或吞吐余量）/ 内存高水位"，否则该切片**不算完成**。

## 阻塞与等待（**负责人裁决更新，第 88 轮**）

> 原文口径：**"去除之前避免单点阻塞的要求，可以 block 等待。"**

⇒ **此前"绝不单点阻塞、必须绕行/并行化/降级"的硬要求被负责人解除**。现在的口径：

1. **允许 `block 等待`**：当推进确实依赖某件事（CI 判决、人类裁决、外部服务/长任务）时，
   **可以**把目标标为 blocked 并等待，**不再**为了"不阻塞"而强行开新线、缩小范围或制造替代产物。
2. **仍然禁止的是"假进度"**：等待期间**不要**为了显得有产出而：改口径达标 · 把未读回的判决写成通过 ·
   把挂起当成功 · 写无法当场复核的改动。等待是允许的，**编造进展不是**。
3. **等待时仍要**：把"在等什么、等到什么程度算解除"写清楚（可复核），并把已完成的实测/裁决记账。

## Commit message language (human ruling, round 256)

**All commit messages must be written in English from now on.** Verbatim ruling: "另外以后commit 都用英文".

- Applies to every future commit in this repository (agent and human alike).
- Keep the established discipline inside the English text: state what was done, cite the exact command/run id evidence,
  and explicitly flag anything not verified (unread verdicts stay `pending`; skipped legs are never reported as passing).
- Existing history is **not** rewritten; Chinese commit messages already in history stay as they are.

## 报告语言与文体（负责人裁决，第 132 轮）

裁决原文：「接下来汇报和需要我做决策都用中文和ASD-STE100格式」。

**适用范围**：给负责人的**汇报**与**待决策请求**。

**语言**：中文。

**文体**：按 ASD-STE100（简化技术英语）的规则写中文。规则如下：

1. 一句一个意思。句子要短。说明句不超过 25 字。指令句不超过 20 字。
2. 指令句用祈使式。
3. 用主动语态。不要用被动语态。
4. 用简单时态。只用现在、过去、将来。不要用完成时。
5. 一个词只表示一个意思。全篇用同一个词表示同一个东西。
6. 不用俚语、成语、比喻。
7. 名词串不超过三个词。
8. 一段只讲一个主题。一段最多六句。
9. 复杂信息用纵向列表。
10. 数字与单位要写全。第一次出现的缩写要先定义。
11. 不用「可能」「大概」这类模糊词。要写实测值或写明「未测」。
12. 结论与证据分开写。证据要带命令或 run id。

**同时保留既有纪律**：判决只来自 CI。写清未读回的判决。写清否定性结果。

## 自主决策与"改规则"的授权（负责人裁决，第 146 轮）

裁决原文：「能你自己决策的事情就自己决策，和规则冲突的，考虑修改规则带来的受益和成本，如果受益远大于成本，就修改」。

**含义**：集成者可以自行裁决**不涉及红线**的事项。规则与目标冲突时，先算**受益与成本**。受益远大于成本时，**改规则**，并把改动、理由与证据一起写进账本。

**边界（不得自行改动）**：红线 1 至 9；`AGENTS.md` 的四份 Normative 规范与法务文件；人类已明示的裁决（除非你重新裁决）。

**第一次行使（同一轮）**：`run-gates.sh light` 原先**不含 clippy**，于是我给 `yeban-dsp` 加判据时本机全绿、CI 却因
`error: using chunks_exact with a constant chunk size` 让 `rust (yeban-dsp)` 与 `windows` **两条腿变红**（run `37327050901`）。
`AGENTS.md` DoD 第 1 条要求 clippy 零告警，§5.1 允许本机跑无重依赖 crate 的 clippy ⇒ **缺的不是许可，是默认路径**。
故新增 `scripts/gates/clippy-changed.sh` 并接进 `light`：只对**本次 git 改动涉及且不含重依赖**的 crate 跑
`clippy --all-targets -- -D warnings`；含重依赖的按既有纪律跳过并注明交给 CI。牙测：向 `yeban-dsp` 注入
`chunks_exact(常量)` 后该检查报错（与 CI 当时同一条 lint）。

