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
| `gh` CLI | **不可用** → CI 判决改用公开 REST API 读取（`scripts/dev/ci-verdict.sh`） | `command -v gh` |
| 仓库 | `git@github.com:gradetwo/yeban.git`，**public**，默认分支 `main` | `git ls-remote`, GitHub API |
| 沙箱限制 | 受限环境中 rustup/cargo 无法写 `~/.rustup`、`~/.cargo` | 实测报错 `Operation not permitted` |

**推论（已落地）**：受限环境下必须设 `RUSTUP_TOOLCHAIN=stable` + 工作区内 `CARGO_HOME`；
这两件事被 `scripts/dev/cargo-local.sh` 封装，不需要每次手敲。

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
| 品牌资产 | 母版拆出 **10 个 SVG 变体** + 10 个 PNG（深/浅 × 512/256/128/64/32） | `assets/brand/` |
| `Cargo.lock` | 已生成并提交（MUST-GATE-005 要求） | `git ls-files Cargo.lock` |

### 已知的本地无法验证项（必须由 CI 判定）

- `cargo clippy --workspace`（本机包装器拒绝 `--workspace`，按设计）；
- `cargo deny check`（本机未安装 cargo-deny，避免为它做重编译）；
- 任何 Slint / cpal / symphonia 相关编译。

---

## 4. 已被证明"能变红"的判据（SKILL 规则 2）

一条从没红过的判据等于"穿着测试外衣的注释"。以下判据都做过**故意违规 → 观察变红 → 还原**：

| 判据 | 故意违规 | 观察结果 | 还原后 |
| :--- | :--- | :--- | :--- |
| 守卫 G01（持久化 AST 零 HashMap） | 往 `crates/yeban-model/src/lib.rs` 追加 `pub type SneakyIndex = HashMap<u8, u8>;` | 红：`crates/yeban-model/src/lib.rs:39: HashMap/HashSet 违规` | 绿 |
| 守卫 G02（引擎层零 GUI 依赖） | 往 `crates/yeban-dsp/Cargo.toml` 的 `[dependencies]` 插入 `slint.workspace = true` | 红：`引擎层 crate yeban-dsp 引入了 GUI 依赖 slint` | 绿 |
| 守卫 G04（严禁 `0.0.0.0`） | 往 `crates/yeban-engine/src/lib.rs` 追加 `pub const BAD_BIND: &str = "0.0.0.0:9316";` | 红：`crates/yeban-engine/src/lib.rs:18` | 绿 |
| `yeban-model` 属性 | `ModelError` 一度派生 `Eq` 而字段含 `f32` → 编译红；`ulid 3.0` 无 `Ulid::new()` → 编译红 | 两次真实变红并修复（见 ADR-0001 D6） | 绿 |

**注意第一次 G02 的证伪过程**：第一次注入把 `slint.workspace = true` 追加到了文件**末尾**，
落进了 `[lints]` 表而不是 `[dependencies]`，守卫正确地没有报红——错的是注入，不是守卫。
这条记下来是因为"判据没红"和"违规没生效"必须区分开（SKILL 规则 7）。

### 尚未证明能变红的判据（诚实登记）

| 判据 | 为什么还没证明 |
| :--- | :--- |
| `scripts/dev/changed-crates.py` 的受影响集合推导 | 只在 `--base HEAD --head HEAD`（空 diff）下验证了保守回退路径；**非空 diff 的闭包推导要等第一次真实分支推送后再验证** |
| `scripts/dev/ci-verdict.sh` | 仓库还没有任何 workflow run，脚本尚未真实取回过一次判决 |
| `site-deploy.yml` 的部署路径 | 需要 Cloudflare 凭据，当前只会走"优雅跳过"分支 |
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
