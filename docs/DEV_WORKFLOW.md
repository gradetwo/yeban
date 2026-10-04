# 夜半 (Yeban) 开发工作流

本文件把 `docs/skills/yeban-dev-workflow/SKILL.md` 的规则落成**这个仓库里可执行的流程**。
SKILL 讲"为什么"，这里讲"怎么敲"。

---

## 0. 两条不可协商的前提

1. **本机是 Apple M2 开发/编辑机，不是构建农场。**
   全量 workspace 构建、Slint / cpal / symphonia 编译、基准、模糊测试、跨架构对账**一律在 CI 上跑**。
   本机只允许：`cargo fmt`、**单个无重依赖 crate** 的 `clippy/test`、全部守卫脚本。
2. **只有 CI 的判决算"绿"。** 本地绿是参考；未读取的 CI 判决一律记为 `pending`，不许写成"通过"。

这两条不是自律要求，是被脚本机械执行的：

| 工具 | 作用 |
| :--- | :--- |
| `scripts/dev/cargo-local.sh` | 本机 cargo 包装器：拒绝 `--workspace`/`--all`，重定向 `CARGO_HOME` 到工作区内 |
| `scripts/gates/run-gates.sh light` | 格式 + 12 条机械红线守卫 + 文档链接门禁，零编译，任何机器都能跑 |
| `scripts/gates/run-gates.sh crate <name>` | 在 light 基础上加该 crate 的 clippy/test；**若该 crate 含重依赖则自动拒绝并交给 CI** |
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
