# CI/CD 说明（GitHub Actions）

用户要求：**自动触发 + 手动触发双模式**，并且**避免在本机跑高耗 CPU 的任务**。
本文件说明这些 workflow 各自干什么、什么时候跑、需要什么凭据、判决怎么读回来。

---

## 1. 三个 workflow

| 文件 | 触发 | 用途 |
| :--- | :--- | :--- |
| `.github/workflows/ci.yml` | **自动**：`push` 到 `main` / `line/**`；`pull_request` 到 `main`；另加 `workflow_dispatch` | 主门禁：计划 → 格式/红线/schema → 受影响 crate 的 clippy+test → 锁文件 → cargo-deny |
| `.github/workflows/gates-manual.yml` | **手动**：`workflow_dispatch`（下拉选择门禁） | 昂贵/专项门禁：`inventory`（门禁清单）、`all-features`（全 feature 组合编译）、`bench`、`determinism`、`pending` |
| `.github/workflows/site-deploy.yml` | **自动**：`push` 到 `website`；另加 `workflow_dispatch`（可选 ref） | 官网 `yeban.wangda.today`：站点静态契约检查 → Chromium 浏览器核验（含截图归档）→ `wrangler deploy` 到 Cloudflare Workers |

### 为什么自动档只跑这些

自动档必须**够快**，否则多线并行会被它拖死。因此：

- 只跑受影响 crate（`scripts/dev/changed-crates.py` 推导，含下游闭包）；
- 不跑基准、不跑模糊测试（它们要么需要固定频率硬件，要么以小时计）；
- `concurrency.cancel-in-progress`：同一分支的新推送取消旧运行。

### 手动档的用法

```bash
# 需要 gh CLI 或网页 Actions 界面 → 选择 "Gates (手动)" → Run workflow
#   gate = inventory      只渲染门禁清单表（最快，用来确认哪些还没接线）
#   gate = all-features   全 feature 组合 clippy（能拦住 feature 门后的编译错误）
#   gate = bench          基准目标编译 + 运行（若有）
#   gate = determinism    L1/L2 对账 —— 目前 PENDING，会输出阻断项说明
```

---

## 2. 需要配置的 secrets（仓库 Settings → Secrets and variables → Actions）

| Secret | 用途 | 未配置时的行为 |
| :--- | :--- | :--- |
| `CLOUDFLARE_API_TOKEN` | `wrangler deploy` 发布官网 | 部署步骤**优雅跳过**并写提示，不报红 |
| `CLOUDFLARE_ACCOUNT_ID` | 同上 | 同上 |

`CLOUDFLARE_API_TOKEN` 需要的权限：**Workers Scripts:Edit**（读取 account 需要 Account Settings:Read）。

`wrangler.toml` 里的自定义域（`yeban.wangda.today`）与 account 归属由管理员按需填写；
未填写时 `wrangler deploy` 会发到默认 `*.workers.dev` 地址，仍然可用。

---

## 3. 判决怎么读回来（这是流程里最容易漏的一步）

```bash
scripts/dev/ci-verdict.sh                  # 当前分支最新一次 run
scripts/dev/ci-verdict.sh website          # 指定分支
scripts/dev/ci-verdict.sh --watch          # 轮询到 run 结束
GH_TOKEN=... scripts/dev/ci-verdict.sh --logs <run-id>   # 拉失败 job 的原始日志
```

仓库是**公开**的，所以匿名 REST API 就能读到 run / job / step 的状态与结论，不需要 `gh` CLI。
原始日志下载接口需要鉴权，因此可选的 `GH_TOKEN` 只在需要看编译错误细节时才用得上。

两个辅助手段保证"没有 token 也能定位问题"：

1. 每个自动档 job 在 `failure()` 时把最后 80 行构建输出写进 **job summary**；
2. `scripts/gates/run-gates.sh` 与 `cargo-local.sh` 都在本机就能复现同一族检查，重依赖部分除外。

**未读取的判决记为 `pending`，不得写成"通过"。**

---

## 3.1 官网门禁（`website` 分支）

官网有自己的两道门禁，都在 `site-deploy.yml` 里，且 `deploy` 依赖它们：

| job | 脚本 | 检查什么 |
| :--- | :--- | :--- |
| `check` | `scripts/check-site.mjs`（纯 Node，零依赖） | 10 组静态契约：词典键集相同、用到的键都存在、无死键、站内引用真实存在、无明文 http、切换控件与语义锚点齐全、必备文件与格式、`lang`/`hreflang`/`canonical`、品牌资产非空、无白名单外外部主机 |
| `visual` | `scripts/visual-check.mjs`（Playwright + Chromium） | 8 组浏览器断言：无控制台错误、i18n 无空占位、自动模式跟随系统配色、三态主题切换立刻生效、`?lang=en` 真的换语言与 `<html lang>`、390px 无横向溢出、区块锚点齐全、404 语义正确；并把全页截图上传为 artifact（保留 30 天） |

截图 artifact 的意义：**"没在渲染器里看过"就是没验证过**。有了 artifact，任何人（包括下一个 Agent）
都能事后回看版面，而不是只能相信断言。

## 4. 已接线 vs PENDING

完整的 `MUST-GATE-001..015` 与 `BASELINE-001..006` 状态表在
`Gates (手动)` → `inventory` 门禁里渲染，也登记在 `docs/DEVELOPMENT_LEDGER.md`。
一句话版本：

- **已接线**：fmt、clippy `-D warnings`、单元/属性测试、cargo-deny、确定性 `Cargo.lock`、
  11 条机械红线守卫（HashMap / GUI 依赖 / `0.0.0.0` / 大文件 / ASIO / 通配版本 / workspace 继承…）、
  JSON Schema 契约、工具链漂移断言。
- **PENDING**：实时回调零分配（MUST-GATE-001/012）、L1 bit-exact（002）、L2 跨架构（003）、
  Zip-Slip 与解压炸弹（006/007）、`.yeban.lock` 并发（008）、MCP 默认安全（009）、
  10,000 步撤销守恒（010）、cargo-fuzz（011）、采样指纹（014）、Golden 图来源（015）、
  以及全部 BASELINE 性能线。

PENDING 的共同原因只有两类：**被验证的功能还没实现**，或**需要固定频率的参考硬件**。
在条件具备之前，这些门禁明确"不通过"，而不是用一个永远绿的假 job 冒充通过。
