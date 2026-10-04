# 开发账本 (Development Ledger)

> SKILL 规则 9：**读数如果不带测法与时刻写进账本，下一个人一定会重新推导错。**
> 本文件记录（a）关键测量值及其测法，（b）已被证明"能变红"的判据，（c）尚未验证的 `pending` 清单。

---

## 1. 环境事实（测量时刻：2026-10-05）

| 项 | 值 | 测法 |
| :--- | :--- | :--- |
| 机器 | Apple M2 / macOS 27.0.1 (build 26A434) / arm64 | `uname -m`, `sw_vers` |
| 本机 Rust | `rustc 1.99.0 (b940084d7e 2026-09-28)`，rustup 只装了名为 `stable` 的工具链 | `rustc --version`, `rustup toolchain list` |
| 钉死工具链 | `rust-toolchain.toml` → `1.99.0`（与上同版本，因此本机 `RUSTUP_TOOLCHAIN=stable` 等价） | 文件内容 + 版本比对 |
| 本机 cargo | `cargo 1.99.0 (5f94df478 2026-08-27)` | `cargo --version` |
| Node / pnpm / wrangler | v26.10.0 / 12.4.1 / 4.136.1 | `--version` |
| 光栅化工具 | `rsvg-convert`、`magick`、`qlmanage` 可用；`cairosvg` 不可用 | `command -v` |
| `gh` CLI | **可用**：`gh 2.102.0`，已登录 `gradetwo`（首个 run 时还不可用，后由人类负责人配置） | `gh auth status` |
| `cargo-deny` | **本机可用**：0.20.2 预编译二进制（`brew`/`cargo install` 都不必，直接下 release tarball），放在仓库外的 `/Users/crow/work/music/.tooling/` | `<bin> --version` |
| 仓库 | `git@github.com:gradetwo/yeban.git`，**public**，默认分支 `main` | `git ls-remote`, GitHub API |
| 沙箱限制 | 受限环境中 rustup/cargo 无法写 `~/.rustup`、`~/.cargo`；`gh` 无法写 `~/.cache/gh` | 实测报错 `Operation not permitted` |

**推论（已落地）**：受限环境下必须设 `RUSTUP_TOOLCHAIN=stable` + 工作区内 `CARGO_HOME`，
`gh` 需要把 `XDG_CACHE_HOME` 指到工作区内。这三件事被 `scripts/dev/cargo-local.sh` 与
`scripts/dev/ci-verdict.sh` 封装，不需要每次手敲。

### 1.1 本机如何跑开源合规门禁（不必重编译）

```bash
mkdir -p /Users/crow/work/music/.tooling && cd /Users/crow/work/music/.tooling
curl -sL -o cd.tgz https://github.com/EmbarkStudios/cargo-deny/releases/download/0.20.2/cargo-deny-0.20.2-aarch64-apple-darwin.tar.gz
tar xzf cd.tgz && rm cd.tgz
# 之后:
YEBAN_CARGO_DENY=/Users/crow/work/music/.tooling/cargo-deny-0.20.2-aarch64-apple-darwin/cargo-deny \
CARGO_HOME=/Users/crow/work/music/.cargo-home RUSTUP_TOOLCHAIN=stable \
  bash scripts/gates/run-gates.sh deny
```

---

## 2. 依赖版本决策（测量时刻：2026-10-05，方法：crates.io API `max_stable_version`）

完整表与裁决理由见 `docs/adr/ADR-0001-workspace-topology-and-version-pinning.md` D5。

| crate | 版本 | 备注 |
| :--- | :--- | :--- |
| slint / slint-build | 1.18.1 | 要求 Rust ≥ 1.92 |
| i-slint-backend-testing | `=1.18.1` | 内部 crate，必须精确同版本 |
| cpal | 0.18.2 | |
| rtrb | 0.4.0 | |
| symphonia | 0.6.1 | MSRV 1.85 |
| rubato | 5.0.1 | |
| ulid | 3.0.0 | **无 `serde` feature**；`Ulid::generate()` 而非 `new()` |
| serde / serde_json | 1.0.229 / 1.0.151 | |
| thiserror | 2.0.21 | |
| sha2 | 0.11.0 | |
| proptest | 1.11.0 | 关掉默认 `fork`/`timeout` |
| criterion / iai-callgrind | 0.8.2 / 0.16.1 | |
| zip / flate2 / hound / midly / midir / notify / rstar / signalsmith-stretch | 8.6.0 / 1.1.10 / 3.5.1 / 0.5.3 / 0.11.0 / 8.2.0 / 0.13.0 / 0.1.3 | 未引用，未进 `Cargo.lock` |

**cargo-deny 配置格式核验**：2026-10-05 对照官方 `checks/advisories/cfg` 与 `checks/licenses/cfg`，
确认 `[advisories] vulnerability`、`[licenses] unlicensed` 等字段在当前 0.20.x **已移除，保留即报错**，
并据此重写了 `deny.toml`。

---

## 3. 本 sprint 的实测结果

| 项 | 结果 | 命令 |
| :--- | :--- | :--- |
| 工作区规模 | 14 个产品 crate + 9 个 spike crate，全部为 glob 成员 | `ls crates spikes` |
| `yeban-model` 测试 | **11 passed / 0 failed** | `bash scripts/dev/cargo-local.sh test -p yeban-model` |
| clippy（23 个轻量 crate） | **0 warning**（`-D warnings`） | `cargo clippy <23 × -p> --all-targets -- -D warnings` |
| 格式 | **clean** | `cargo fmt --all --check` |
| 机械红线守卫 | **11/11 通过** | `python3 scripts/guards/policy_check.py` |
| JSON Schema | **4/4 合法**（Draft 2020-12） | `python3 scripts/gates/validate_schemas.py` |
| cargo-deny（开源合规） | **advisories ok, bans ok, licenses ok, sources ok** | `YEBAN_CARGO_DENY=… bash scripts/gates/run-gates.sh deny` |
| 首次 CI（run 37216800773） | **22/23 job 绿**；唯一红的是 `deny`，原因是 `deny.toml` 的 TOML 表位置错误（见第 6 节），已修复 | `gh run view 37216800773` |
| 修复后 CI（run 37217037266） | **27/27 job 全绿**：plan / checks / lockfile / deny + 23 条 rust 矩阵腿 | `gh run view 37217037266` |
| `yeban-theory` 落地（run 37217942685） | **27/27 全绿**；94 条测试（71 单元 + 22 属性 + 1 doctest）；流派规则库 **182 条**（规范正文写 159，见 ADR-0001 D14） | `gh run view 37217942685` |
| `yeban-dsp` 落地（run 37217904082） | **27/27 全绿**；108 单元测试 + 1 doctest；13 个模块（含 2 个新写、11 个从 `synth-core` MIT 移植改写） | `gh run view 37217904082` |
| 引入 Slint 后的依赖规模 | 外部包 **62 → 579**；许可族新增 `BSL-1.0`（2 个 Windows-only 包）与 `GPL-3.0-only OR LicenseRef-Slint-*`（14 个 slint 包，取 GPL-3.0-only 分支）；无一包缺 license 字段 | `cargo metadata` + `scripts/gates/license_inventory.py` |
| `yeban-app` 首次真的编译 Slint（run 37218433961） | **`rust (yeban-app)` = success**：13 个 `.slint` 全部编译通过，`slint_build::compile` 生成代码可用，`slint::include_modules!()` 在嵌套模块里可行，clippy `-D warnings` 零告警，40 条单元测试绿 | `gh run view 37218433961` |
| `yeban-app` 全绿（run 37218791723，round 3） | **success**：含 `checks` 里新接的"依赖许可清单漂移检查"（该门禁第一次变红就抓到了 Slint 引入后清单未重生成） | `gh run view 37218791723` |
| **主分支 tip 全量真跑**（手动 `force_full`，run 37219320374） | **27/27 job success，0 failure**：计划器返回全部 **23** 条 rust 矩阵腿（含 `yeban-app` 真实编译 Slint），加上 checks / lockfile / deny / plan | `gh run view 37219320374` |
| **主分支当前 tip 全量真跑**（run 37219697954，含 sfz 合并与全部门禁改进） | **27/27 job success，0 failure** —— 这是"当前 tip 全绿"的最终证据 | `gh run view 37219697954` |
| 已合并工作线的退役 | 5 条线（model-core / theory-core / dsp-core / ui-shell / sfz-core）**先打归档标签 `line-archive/<name>` 再删除本地+远程分支**；远程只剩 `main` 与 `website`；5 条线的实现内容全部保留在 main 的合并提交里 | `git tag --list 'line-archive/*'`; `git ls-remote --heads origin` |
| `yeban-sfz` 落地（run 37219428588） | **27/27 全绿**；58 条测试（27 lib + 16 include 沙箱 + 12 畸形输入 + 3 doctest）；运行时依赖仅 `thiserror`，零 dev-dependency | `gh run view 37219428588` |
| 跨语言契约对账（本机实测全链路） | `cargo run -p yeban-model --example export_schema_samples` → 4 份样本 → `validate_schemas.py --samples-dir` → **4/4 通过**（ops 样本 23 变体 oneOf 对账通过） | 见 `docs/ledger/model-core-provenance.md` §4 |
| 官网分支首次 CI（run 37217762431，branch `website`） | **绿**：站点契约检查通过；部署步骤按设计"缺凭据 → 优雅跳过" | `gh run view 37217762431` |
| 品牌资产 | 母版拆出 **10 个 SVG 变体** + 10 个 PNG（深/浅 × 512/256/128/64/32） | `assets/brand/` |
| `Cargo.lock` | 已生成并提交（MUST-GATE-005 要求）；`cargo metadata --locked` 通过；**不含** slint/cpal/symphonia 等重依赖（spike 的重依赖暂时注释） | `git ls-files Cargo.lock`, `cargo metadata --locked` |

### 已知的本地无法验证项（必须由 CI 判定）

- `cargo clippy --workspace`（本机包装器拒绝 `--workspace`，按设计）；
- 任何 Slint / cpal / symphonia 相关编译；
- 跨架构（x86_64 ↔ AArch64）确定性对账与全部 BASELINE 性能读数。

（`cargo deny check` 原本也在此列；已通过"下预编译二进制 + `YEBAN_CARGO_DENY`"把它拉回本机可验证集合，
理由见第 6 节 L4。）

---

## 4. 已被证明"能变红"的判据（SKILL 规则 2）

一条从没红过的判据等于"穿着测试外衣的注释"。以下判据都做过**故意违规 → 观察变红 → 还原**：

| 判据 | 故意违规 | 观察结果 | 还原后 |
| :--- | :--- | :--- | :--- |
| 守卫 G01（持久化 AST 零 HashMap） | 往 `crates/yeban-model/src/lib.rs` 追加 `pub type SneakyIndex = HashMap<u8, u8>;` | 红：`crates/yeban-model/src/lib.rs:39: HashMap/HashSet 违规` | 绿 |
| 守卫 G02（引擎层零 GUI 依赖） | 往 `crates/yeban-dsp/Cargo.toml` 的 `[dependencies]` 插入 `slint.workspace = true` | 红：`引擎层 crate yeban-dsp 引入了 GUI 依赖 slint` | 绿 |
| 守卫 G04（严禁 `0.0.0.0`） | 往 `crates/yeban-engine/src/lib.rs` 追加 `pub const BAD_BIND: &str = "0.0.0.0:9316";` | 红：`crates/yeban-engine/src/lib.rs:18` | 绿 |
| `yeban-model` 属性 | `ModelError` 一度派生 `Eq` 而字段含 `f32` → 编译红；`ulid 3.0` 无 `Ulid::new()` → 编译红 | 两次真实变红并修复（见 ADR-0001 D6） | 绿 |
| 官网 `check-site.mjs` C1/C2 | 删掉 `en.json` 的 `nav.features` | 红：`en.json 缺少键` + `HTML 用到 nav.features 但 en.json 里没有` | 绿 |
| 官网 C2/C3 | 把 HTML 里的 `data-i18n="nav.stack"` 改成不存在的键 | 红：`HTML 用到 nav.nonexistent` ×2 + `词典里的 nav.stack 从未被使用` | 绿 |
| 官网 C6/C3 | 删掉"浅色"主题按钮 | 红：`缺少主题-浅色按钮` + `nav.theme_light 从未被使用` | 绿 |
| 官网 C4 | 把 logo 路径改成不存在的文件 | 红：`引用了不存在的站内路径` | 绿 |
| 官网 C7 | 从 `site.webmanifest` 删掉 `icons` | 红：`缺少字段: icons` | 绿 |
| 文档门禁 `check_docs_links.py` | README.md 里一个相对链接改成不存在的路径 | 红：`链接指向不存在的路径` | 绿 |
| 文档门禁 R2 | 去掉 README.md 里的语言互链 | 红：`README.md 没有链接到 README.zh-CN.md` | 绿 |
| 文档门禁 R3 | 把中文 README 的 logo 源写成不存在的文件 | 红：`缺少 yeban-dark-256.png` | 绿 |
| 官网 visual A8（CI 上真实抓到） | 无需注入：404 页接上 i18n 后原生的 "404" 字样被替换掉 | 红：`404 页 h1 不含 404: Page not found` | 修复后绿 |

**注意第一次 G02 的证伪过程**：第一次注入把 `slint.workspace = true` 追加到了文件**末尾**，
落进了 `[lints]` 表而不是 `[dependencies]`，守卫正确地没有报红——错的是注入，不是守卫。
这条记下来是因为"判据没红"和"违规没生效"必须区分开（SKILL 规则 7）。

### 尚未证明能变红的判据（诚实登记）

| 判据 | 为什么还没证明 |
| :--- | :--- |
| `scripts/dev/changed-crates.py` 的受影响集合推导 | 只在 `--base HEAD --head HEAD`（空 diff）下验证了保守回退路径；**非空 diff 的闭包推导要等第一次真实分支推送后再验证** |
| `scripts/dev/ci-verdict.sh` | 仓库还没有任何 workflow run，脚本尚未真实取回过一次判决 |
| `site-deploy.yml` 的真实部署路径 | 需要 Cloudflare 凭据，当前只验证了"缺凭据 → 优雅跳过"这条分支 |
| 官网的浏览器视觉核验 | **已在 CI 跑绿**（run 37217947487）：8 组断言通过，4 张全页截图作为 artifact 留存，并已下载人工过目（深色中文首页 / 浅色英文首页 / 移动端 390px / 404 页）。截图证据：`gh run download 37217947487 -n site-screenshots` |
| 依赖许可清单 `--check` | **已闭环**：62 包时本机绿；Slint 引入后 CI 真实变红一次（清单未重生成），重生成后 579 包绿；合并 ui-shell 后本机 `--check` 再次通过（635 行清单） |
| Linux 上 Slint 的系统库前置 | **已被真实判决覆盖**：apt 步骤加上后 `rust (yeban-app)` 转绿（run 37218433961 / 37218791723） |
| cargo-fuzz（MUST-GATE-011） | `crates/yeban-sfz/fuzz/` 目标已就位，但**从未执行**（本机禁止重活，CI 手动档尚未加 fuzz job）。sfz 工作线给了手动档命令，接线属集成者待办 |
| "RT 路径零分配"的机械证明 | 目前只有结构保证 + 容量不变量（`yeban-sfz` 的 voice_pool）与纪律，**没有**分配器计数的机械证明（MUST-GATE-001/012 仍未接线） |
| **DAW 界面从未被人眼或渲染器看过** | Slint 只是**编译通过**。没有截图、没有 SSIM、没有控件树断言 —— 那属于 `yeban-ui-test-port`（`UI-TEST-001/003`、`MUST-GATE-015`），尚未开始。布局比例、CJK 字体回退、无障碍树内容目前都只是"编译层面成立"。**这是本项目当前最大的未验证面。** |
| `gates-manual.yml` 的 `determinism` / `bench` | 目标功能未实现，只有 PENDING 说明路径 |

---

## 5. PENDING 清单（未验证 = 不许写成"通过"）

### 5.1 门禁

| 门禁 | 阻断原因 |
| :--- | :--- |
| MUST-GATE-001 / 012 实时回调零分配 | `yeban-engine` 未实现 |
| MUST-GATE-002 L1 bit-exact | `yeban-render` 未实现 + 需锁定 ISA 的固定频率机器 |
| MUST-GATE-003 L2 跨架构 < 1e-6 | 需 x86_64 与 AArch64 双 runner |
| MUST-GATE-006 / 007 Zip-Slip 与解压炸弹 | `.yeban` 容器未实现 |
| MUST-GATE-008 `.yeban.lock` 并发 | 锁未实现 |
| MUST-GATE-009 MCP 默认安全 | `yeban-mcp` HTTP 形态未实现 |
| MUST-GATE-010 10,000 步撤销守恒 | `Op` 全变体未实现 |
| MUST-GATE-011 cargo-fuzz | SFZ / JSON 解析器未实现 |
| MUST-GATE-014 323 款采样指纹 | 样本清单未落地（`assets/samples/` 仅有 ATTRIBUTION 骨架） |
| MUST-GATE-015 Golden 图来源 | `yeban-ui-test-port` 未实现 |
| BASELINE-001..006 | 需固定频率参考硬件（M2 Pro 12 核 / Ryzen 7840HS） |

### 5.2 工程债与待办

| 项 | 说明 |
| :--- | :--- |
| GitHub Actions 版本钉死 | 目前用 `actions/checkout@v4`、`EmbarkStudios/cargo-deny-action@v2`、`actions/setup-node@v4` 等**主版本标签**；供应链更严的做法是钉 commit SHA，属于待办 |
| 自托管 runner | BASELINE 与确定性对账需要固定频率机器；接入时间由人类决定（ADR-0001 待批准项 3） |
| CI 缓存 | 各矩阵腿会重复编译公共依赖；`Swatinem/rust-cache` 之类的缓存尚未接入 |
| `CONTRIBUTING.md` 的 MSRV 表述 | 正文写 "Rust 1.80+"，实际 MSRV 因 slint 1.18.1 定为 **1.92**（ADR-0001 D5）。文件不在红线名单内，但改动治理文档建议由人类确认 |
| UI/UX 规范的缺口 | 字体与 CJK 字体栈、间距/圆角/字号 scale、DPR 细则、Splitter 约束对象、24px 状态栏归属——见 ADR-0001 与 UI 摘要，实现前需按登记口径执行 |
| `schemas/mcp-tools.schema.json` 与 10 个工具的最终对账 | 需 `yeban-mcp` 实现后才能做真实对账 |

---

## 6. 教训与由此产生的规则 (Mistakes and the rules they produced)

SKILL 规则 10：**把自己的错误连同它产生的规则一起记下来**，这是最便宜的文档。以下四条都是本 sprint 真实踩到的。

### L1 — 生成的骨架里文档注释没加前缀，整个 crate 无法解析

- **现象**：用脚本批量生成 22 个 crate 的 `lib.rs` 时，只给标题行加了 `//!`，正文行是裸文本。
  于是 `cargo fmt` 报 `E0758: unterminated block doc-comment` —— 因为正文里的
  `` `synth/crates/synth-core/src/dsp/**` `` 含 `/**`，被当成块注释起始。
- **规则**：**批量生成代码后必须立刻过一遍解析器（fmt/clippy），不能只看文件"写出来了"。**
  生成器把内容写进文件 ≠ 内容是合法代码。
- **已落地**：修复脚本对每行补 `//!` 前缀，并用 `cargo fmt --all --check` 作为"全部文件可解析"的机器判据。

### L2 — TOML 表的**位置**语义：`[licenses.private]` 之后的键都属于它

- **现象**：`deny.toml` 里把 `[licenses.private]` 插在 `unused-allowed-license` 与 `allow` 之前，
  于是这两个键落进子表。首次 CI 的 `deny` job 报
  `error[unexpected-keys]: found 2 unexpected keys, expected: ["ignore", "ignore-sources", "registries"]`。
- **规则**：**子表一律放在父表所有标量键与数组之后。** 这条在 TOML 里是硬语义，不是风格问题。
- **代价与反思**：这个错误 100% 可以在本地发现——当时没做，是因为我判断"本机不装 cargo-deny"。
  这条错误直接催生了 L4。

### L3 — "判据没红"必须区分"违规没生效"

- **现象**：为证明守卫 G02（引擎层零 GUI 依赖）能变红，我把 `slint.workspace = true` **追加到
  `Cargo.toml` 末尾**；结果它落进了 `[lints]` 表，`[dependencies]` 根本没变，守卫正确地没报红。
  第一次的错误结论是"守卫有问题"，真相是**注入无效**。
- **规则**：**判据没变红时，先证明你的"故意破坏"真的生效了**（打印被改的那一段、或让破坏本身导致编译失败），
  再怀疑判据。SKILL 规则 7 说的就是这件事。

### L4 — 能把门禁拉到本机验证的，就不要留给 CI

- **现象**：L2 的 `deny.toml` 错误本可以在 30 秒内本地发现。当时的判断是"装 cargo-deny 要重编译，不划算"——
  但这个前提是错的：官方提供 **aarch64-apple-darwin 预编译二进制**，下载解压即可，零编译。
- **规则**：**在把某个门禁推给 CI 之前，先花两分钟确认它在本机是否真的不可行。**
  "需要重编译"和"需要本机没有的硬件"是两类完全不同的事：前者几乎总有绕行方案（预编译产物、
  单文件脚本、纯 Python 实现），后者才真正属于 CI/自托管 runner。
- **已落地**：`scripts/gates/run-gates.sh deny` 档位 + `YEBAN_CARGO_DENY` 环境变量；
  安装步骤写进 `docs/DEV_WORKFLOW.md` 与本文件 §1.1。
- **推论（避免单点阻塞）**：任何"只能靠 CI 判"的门禁，都要在 `docs/CI_CD.md` 里写清**为什么**
  本机做不了；写不出理由的，说明它本该在本机就能跑。

### L5 — `cmd | head` 会用 SIGPIPE 杀掉长命令（SKILL 规则 4 的另一种形态）

- **现象**：批量建工作树时写成 `bash scripts/dev/worktree.sh add "$line" main | head -2`。
  `worktree.sh` 打印到第二行时 `head` 退出，`git worktree add` 收到 SIGPIPE 被杀在**创建目录之前**——
  结果：分支建好了（`git branch` 能看到 `line/*`），工作树目录却不存在，而屏幕上的输出看起来"成功了"。
- **规则**：**不要把一个会做实际工作的命令管道到 `head`/`tail`。** 想看前几行就用 `--quiet` 或
  重定向到文件后再读。SKILL 规则 4 讲的是退出码被管道吞掉，这里是同一个根因的第二个后果：
  管道会把**下游提前退出**升级成上游进程被杀。
- **已落地**：改用 `> /tmp/wt-$line.log 2>&1` 再读文件；`scripts/dev/worktree.sh` 依然保持"失败就非零退出"。

### L6 — 我把门禁管道进了 `tail`，失败被吞掉，红提交被推上去

- **现象**：合并 model-core 之后我写了这样一条命令：
  `bash scripts/gates/run-gates.sh crate yeban-model 2>&1 | tail -6 && git commit ... && git push`。
  `run-gates.sh` 报了 `FAIL fmt (exit=1)`，但管道的退出码是 `tail` 的 `0`，
  于是 `&&` 链继续往下走 —— **一个 fmt 未通过的提交被推到了 main**。
- **这是 SKILL 规则 4 的原话**（"Never pipe a gate. `gate | tail` reports `tail`'s exit code"），
  我在 `docs/DEV_WORKFLOW.md` 里抄了这条规则，然后在同一个 sprint 里亲手违反它。
- **规则（加强版）**：门禁命令**永远**单独执行，或重定向到文件后读文件：
  `bash scripts/gates/run-gates.sh crate X > /tmp/gate.log 2>&1; echo $?; tail /tmp/gate.log`。
  如果要保留 `set -o pipefail` 的管道，必须显式 `set -o pipefail`，且**不允许**把这条管道
  后面再接 `&&` 去做提交/推送这类有副作用的事。
- **代价**：一次红推送（CI 的 checks job 会在 `cargo fmt --all --check` 变红），下一次提交修复。
- **已落地**：`scripts/gates/run-gates.sh` 与 `scripts/dev/cargo-local.sh` 的说明里都写了这一条；
  本账本把"我本人违反过"这件事留痕 —— 规则如果不记录违反记录，就会被当成建议而不是纪律。

### L7 — `gh` 的 run-log zip 被 `git add -A` 扫进了 main

- **现象**：`scripts/dev/ci-verdict.sh` 的 `prepare_gh()` 把 `XDG_CACHE_HOME` 指到了**仓库内**的 `.cache/`。
  随后 `gh run view --log` 在那里写下 `run-log-<id>.zip`，而我下一次 `git add -A` 就把它一起提交了
  （commit `9df4315`，文件 424,092 字节）。
- **发现方式**：不是我自己发现的 —— `model-core` 工作线在汇报里提醒"曾误把 ci-verdict.sh 落到仓库内的 gh 日志 zip 提交过一次"，
  我在主工作树 `git status` 里果然看到同一个文件。**这正是"多线 + 诚实汇报"的价值**。
- **规则**：任何**工具缓存目录**都必须落在仓库之外；`git add -A` 之前先看 `git status`。
  推论：把缓存目录指到仓库内的默认值，本身就是一颗定时炸弹。
- **已落地**：
  1. `ci-verdict.sh` / `local-env.sh` 的缓存目录改到**仓库之外**的工作区根（`/Users/crow/work/music/.cache`）；
  2. `.gitignore` 增加 `/.cache/`（防御性）；
  3. 新增守卫 **G12**（工具缓存目录与 `run-log-*.zip` 一律不得入库），并已实证"注入 → 变红 → 还原"；
  4. 本条修正同步更新了 README / CI_CD / DEV_WORKFLOW 里"11 条守卫"→"12 条"（SKILL 规则 5：改了计数就要跑推导计数的检查）。
- **历史残留（如实登记）**：该 424KB zip 仍在 main 的历史里（`9df4315`）。
  它只是**公开仓库**的 CI 日志，不含密钥，体积也远低于 10MB 红线，因此当前处置是"从顶端移除 + 登记 + 加守卫"，
  **不重写已推送的历史**。若人类要求彻底清除，需要一次 `git filter-repo` + 强制推送（会打乱所有克隆与 CI 记录），
  列入 `ROAD-M-1-005` 的历史清理项一并决定。

### L8 — 我写的"强制全量"手动档其实什么都没跑，而且报了 success

- **现象**：`ci.yml` 的 `workflow_dispatch` 有 `force_full` 输入，我把它实现成"把 diff base 挪到 `HEAD~1`"。
  在 tip 上跑（run 37219162892）时，`HEAD~1..HEAD` 只有一个 `docs/adr/**` 提交，
  于是计划器推导出"无受影响 crate"，`rust` 矩阵**整条被跳过**，而整个 run 仍然报 **success**（5 个 job：plan/checks/lockfile/deny + 1 skipped）。
- **为什么危险**：一个"声称跑全量、实际什么都没跑"的档位，比没有这个档位更糟 ——
  它会在需要"tip 全量绿"的场合给出一个假的绿。这跟 SKILL 规则 2（判据必须能失败）是同一类错误的另一面：
  **判据必须能真的执行**。
- **修复**：计划器新增 `--force-full`，直接返回**全部成员**（实测 0 → 23 个 crate）；
  workflow 在该输入为真时传这个开关，而不是挪 base。本机两种模式已对账：
  `HEAD~1 差异 -> 0 crates` vs `--force-full -> 23 crates, workspace_wide=true`。
- **规则**：任何"手动触发"的开关都要在本机看到它**真的改变了被测集合**，再相信它的判决。
  只看 run 的 conclusion 会被"跳过也算成功"骗过去。

### L9 — workflow YAML 里的双引号把我自己的门禁写坏了

- **现象**：给手动档加 `fuzz` 档位时，我在 `gates-manual.yml` 里写了一句
  `description: "… (MUST-GATE-011 是"千万次变异", 60s 只是冒烟)"` —— 内层双引号直接终止了外层字符串，
  YAML 解析失败。这类错误在 GitHub 上表现为「workflow 不出现 / 不触发」，比编译错误难排查得多。
- **规则**：**改 workflow 就要在本机跑一次 YAML 解析**。这条立刻被机械化成了守卫 **G13**
  （所有 workflow 必须合法 YAML、有触发条件、每个 job 有 `runs-on`），并已实证"注入 → 变红 → 还原"。
- **顺带**：G13 在装不到 PyYAML 时**出声跳过**而不是静默通过 —— 静默跳过等于假绿，
  这与 MUST-GATE-015 禁止用不渲染像素的后端产出 Golden 图是同一个道理。

