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

## 3.2 Linux runner 的系统前置依赖（Slint 相关）

引入 Slint 之后，**编译阶段**就需要系统库：`yeslogic-fontconfig-sys` 的 build script 用 pkg-config 找
fontconfig，缺了它会在"编译依赖"时直接红 —— 那不是代码错，而是 runner 缺系统库。

因此 `.github/workflows/ci.yml` 的 `rust` 矩阵腿与 `gates-manual.yml` 的 `all-features` 档位都会先执行：

```bash
sudo apt-get install -y --no-install-recommends \
  pkg-config libfontconfig1-dev libfreetype-dev \
  libxkbcommon-dev libwayland-dev libx11-dev libgl1-mesa-dev \
  libasound2-dev fonts-noto-cjk
```

- `libasound2-dev`：`yeban-engine` 经 `cpal` 链接 ALSA。
- `fonts-noto-cjk`：**运行时环境依赖，不是仓库资产**（见 ADR-0001 D24）。没有它，中文会渲染成豆腐块，
  而 Golden 会"稳定地"记录这个错误（比随机失败更危险）。实测：无字体时汉字区域墨迹 24 px，
  装上后 648 px，`cjk_ink >= 150` 已成为判据。

另外两处只有 Slint 才会触发的合规现实（都已在 main 上处理）：

| 现象 | 原因 | 处置 |
| :--- | :--- | :--- |
| `cargo deny` 报 `BSL-1.0` 被拒 | `clipboard-win` / `error-code`（← `arboard` ← winit/slint 剪贴板，Windows 目标） | BSL-1.0 是 OSI + FSF 认证的宽松许可、GPLv3 兼容，已加入 `deny.toml` 白名单并注明来源链路 |
| 依赖包数量从 62 跳到 ~579 | Slint + winit + fontique 的传递依赖 | 许可清单重新生成（`scripts/gates/license_inventory.py`），`--check` 会在漂移时变红 |

## 3.3 跨语言契约对账（`checks` 腿）

`checks` 腿里有一步把 **Rust 侧写出的字节** 与 **Python 侧读的契约** 对账：

```bash
cargo run -p yeban-model --example export_schema_samples -- --out target/schema-samples
cargo run -p yeban-mcp   --example export_mcp_samples   -- --out target/schema-samples
python3 scripts/gates/validate_schemas.py --repo-assets --samples-dir target/schema-samples
```

三条容易踩的规则（都付过学费）：

1. **新增一个带 JSON 契约的 crate，就要在这里加一行导出**，否则它的契约"只是定义了一堆没人引用的类型"。
2. **契约的根必须真的约束样本**。`mcp-tools.schema.json` 的根曾经不引用 `definitions`，
   于是 `{"anything":[1,2,3]}` 都能通过 —— 那是**空转的假绿**（ADR-0001 D25）。
   自查方式：**喂一个故意非法的样本，确认它被拒**。
3. **文档样本用 `.meta.json` 后缀**：注册表快照/缺口清单这类"统计"不是契约实例，
   用同一套 `oneOf` 根校验它们属于类型错误。校验器会跳过 `.meta.` 并**要求每个前缀至少 1 份真实例**
   （否则"全是 meta"= 该契约没有对账）。

`--repo-assets` 是另一件事：它读**仓库自己的** `assets/**/manifest.json`，校验结构并**逐项重算 SHA-256
与 `size_bytes` 和磁盘对账**。此前这些清单**从未被任何门禁读过**（红线 9 的"登记"没有机械保护）。
`optional: true` 的条目（例如 18MB 的 ONNX 权重）在文件缺失时不算错，但会打印"未随仓库分发"的条数。

## 3.4 无头 UI 截图与基准读数（artifact / job summary）

- `rust` 矩阵腿与 `rust (workspace 全量)` 腿都会上传 `target/ui-test-port/` 作为 artifact
  （`ui-screenshots-*`，保留 30 天）。**没有它，"界面被渲染器看过"只能停在"断言通过"这一层。**
- 手动档 `bench` 会跑 `cargo run --release -p yeban-render --example bench_render`，
  把 `BENCH …` 行写进 job summary。**托管 runner 的读数只能给数量级**，
  不能用来判定 `BASELINE-001` 的"≥100× 实时"是否达标；DoD 4 的"衰退 ≤3%"需要**可比固定硬件**，
  在本 CI 上**不可判定**（记为 pending）。不要把读数读成"通过"。

## 4. 已接线 vs PENDING

完整的 `MUST-GATE-001..015` 与 `BASELINE-001..006` 状态表在
`Gates (手动)` → `inventory` 门禁里渲染，也登记在 `docs/DEVELOPMENT_LEDGER.md`。
一句话版本：

- **已接线**：fmt、clippy `-D warnings`、单元/属性测试、cargo-deny、确定性 `Cargo.lock`、
  14 条机械红线守卫（HashMap / GUI 依赖 / `0.0.0.0` / 大文件 / ASIO / 通配版本 / workspace 继承 / 工具缓存不入库 / workflow YAML 合法…）、
  JSON Schema 契约（含 `.meta.` 约定与承重根）、**资产清单 + 逐项 SHA-256 对账**、工具链漂移断言、
  无头 UI 截图 artifact、`BASELINE-001` 的数量级测量入口。
- **PENDING**：实时回调零分配（MUST-GATE-001/012）、L1 bit-exact（002）、L2 跨架构（003）、
  Zip-Slip 与解压炸弹（006/007）、`.yeban.lock` 并发（008）、MCP 默认安全（009）、
  10,000 步撤销守恒（010）、cargo-fuzz（011）、采样指纹（014）、Golden 图来源（015）、
  以及**除 `BASELINE-001` 数量级读数之外**的全部 BASELINE 性能线（含 DoD 4 的 3% 回归阈值）。

PENDING 的共同原因只有两类：**被验证的功能还没实现**，或**需要固定频率的参考硬件**。
在条件具备之前，这些门禁明确"不通过"，而不是用一个永远绿的假 job 冒充通过。

## 读判决时最容易骗过自己的四个坑（实测，都是本项目踩过的）

1. **空心绿**：`plan` 判定受影响 crate 集合为空时（纯文档/资产改动），`rust` 腿会**静默跳过**。
   此时总 `conclusion=success` **不构成任何代码证据**。⇒ 读判决必须看**各腿**的 `conclusion` 与 `steps`，
   不只看总结果；必要时用 `python3 scripts/dev/changed-crates.py --base <A> --head <B>` 复核 `plan` 的理由。
2. **合并 commit 的判决要单独取**：merge commit 常只改受版本控制的文件而 `plan` 未命中 crate ⇒
   被合并的代码可能**从未被编译过**。落地一条线之后，务必确认"**该 tip 上真的跑过 code 腿**"；
   必要时像 D46 那次一样派发手动档（`gh workflow run gates-manual.yml -f gate=windows`）取真判决。
3. **"没判定" ≠ "判失败"**：门禁与脚本的退出码必须是三态（0 通过 / 1 失败 / **2 无法判定**），
   并把 2 显式判红而不是当成通过（`fetch-samples.py`、`audit-upstream-paths.py`、`check_release_defaults.sh` 都是这么写的）。
4. **缓存别落进仓库**：`gh run view --log` 会在 `XDG_CACHE_HOME` 下落 `gh/run-log-*.zip`；
   相对路径会把它们写进仓库，而 `G12` 扫**文件系统**会一票否决。⇒ 一律用**仓库外绝对路径**。

### 第五个坑：**"挂住的绿"（本仓已复现两次）**

**症状**：`gh run view <id> --json status,conclusion` 长时间返回 `in_progress` / 空 conclusion，
`updatedAt` **不再前进**，且某条腿停在 **`Post Run actions/checkout@v4 [pending]`**（post-job 清理）。
`gh run cancel` 会**长时间不响应**（post-cleanup 阶段收不到取消），最终才落为 `cancelled`。

**实测两次**（都不是失败，只是挂住）：
- `line/origin-contract` 的 `6f771a7` 轮：`rust (workspace 全量)` 与 `windows`；
- `line/engine-mirror-race` 的 `37281806141` 轮：**`rust (yeban-app)`**（同轮 `rust (yeban-engine)` 已 success）。

**判定**：**不是"慢"，是 post-job 清理被孤儿进程挡住** —— 某个测试**派生的子进程仍持有 stdout/stderr**，
Actions 的清理会一直等它 ⇒ 作业既不成功也不失败，判决要等**作业超时**（`rust` 作业 `timeout-minutes: 60`）。
（同一现象在日志里还会表现为 `Cleaning up orphan processes`。）

**处置（按序）**：
1. **不要**把它当成"还在跑"继续等；先看 `updatedAt` 与最后一步是不是 `Post Run …`；
2. `gh run cancel <id>`；若取消不落地，**推一个新提交**（`concurrency: cancel-in-progress` 会顺带收掉挂起那次）；
3. **根因修复**（已列为待办的独立任务）：找出**泄漏子进程的测试**（`yeban-app` 与 workspace 全量测试里
   凡是 `Command::spawn` / 线程 / Slint 事件循环处），判据必须是"**测试结束后没有存活子进程持有 stdout**"
   —— **不许**用 `#[ignore]` 或缩短超时来掩盖。

**为什么值得单列**：它直接吃掉本项目的**判决带宽**（一次挂起 = 最多 60 分钟没有判决），
而本项目的核心纪律是"**未读回的判决等于没有判决**"。

## 结构性纪律：别让"绿"里混进没有 crate 证据的 success（实测，2026-10 两个判决）

**背景**：`ci.yml` 的 `concurrency: cancel-in-progress: true`（按 ref）会让**新推送取消在飞的 run**。
实测 main 最近 40 条 run：`rust (workspace 全量)` 腿有 **10 条连续 `cancelled/steps=10`**；
而其余 `success` 几乎全是 **docs-only ⇒ crate 腿 `skipped steps=0`**；最近一次该腿真跑完且绿是更早的一条。
⇒ **"main 绿"这个集合长期混进大量零 crate 证据的 success** —— 这是"空心绿"的**系统性版本**。

**已做的修复**: `ci.yml` 改为 `cancel-in-progress: ${{ github.ref != 'refs/heads/main' }}`
—— 分支上仍取消（省额度），**main 上排队等它跑完**（不掐掉证据）。

**要确定性拿到"workspace 全量"判决，只有两条可靠路径**（实测：`rerun` **不是**可靠开关 —— 两次 rerun 一次 wide 一次 narrow）：
1. **推一个碰 `crates/**` 的提交**（或根级触发器：`Cargo.toml`/`Cargo.lock`/`schemas/**`/`scripts/**`/`.github/**`/`deny.toml`/`rust-toolchain`）
   ⇒ `plan` 判 `workspace_wide` ⇒ 跑 `rust (workspace 全量)`；
2. **用 `gates-manual.yml`** 派一发（它的 `cancel-in-progress: false`，不会被别的推送掐掉）。

**读判决的铁律（合起来看）**：
- 看**各腿**的 `conclusion` **与 `steps` 数**，不只看整轮 `status/conclusion`（`steps=0` 的 success 不是证据）；
- **"挂住的绿"的可靠判据**是 job 的 `steps[].started_at` 是否还在前进 —— 只看 `updatedAt`、或看"最后一步是不是 `Post Run …`"都会误判
  （**未开始的步骤也会显示成 `Post Run …[pending]`**）；
- `gh run cancel` 在 post-job 阶段**长时间不响应**；取消挂起 run 后，同 ref 的新 run 会一直 `queued` 直到它真的被终止；
- **等判决期间不要推送** —— `cancel-in-progress` 会把证据 run 掐掉（本项目的实测代价：掐掉过**唯一**那条证据）；
- **`gh` 的日志缓存要落在仓库外**：`XDG_CACHE_HOME=/Users/crow/work/music/.cache gh run view --log`，
  否则 `.cache/gh/run-log-*.zip` 会落进仓库，被 `G12 [仓库卫生]` 一票否决（它扫**文件系统**，不是索引）。

**两个已确认的完成态判决**（可作为"全量绿"的基线）: `37283699896` @ `aac62e8`、`37284571290` @ `77d201f`
—— 两者的 `rust (workspace 全量)` 均 **success `steps=10`**、`windows` **success `steps=8`**。

## 两个新踩到的坑（本会话实测，已机械化其一）

### 第六个坑：**本地绿、却根本发不出去**（job id 非法）

我给手动档加 `goldens` 档时，把中文写成了 job 的**键**：
```yaml
  goldens (生成分平台基准图集):     # ← GitHub 直接拒绝整个 workflow
```
GitHub 的报错是
`The identifier 'goldens (生成分平台基准图集)' is invalid. IDs may only contain alphanumeric characters, '_', and '-'`。
而**本地一切正常**：`python3 -c "import yaml"` 能解析（它容忍非 ASCII 映射键）、`run-gates.sh light` 也全绿 ——
因为 G13 当时只校验"能解析 + 每个 job 有 `runs-on`"，**没校验 job 键的字符集**。

**修法（已做）**: G13 增加 `[A-Za-z_][A-Za-z0-9_-]{0,99}` 的 job 键判据，报错信息里直接写"中文请放进 `name:`"。
配了牙测：注入 `goldens 中文:` ⇒ `[FAIL] G13 ... job id 不合法`；还原 ⇒ 通过。

**教训**: "能解析"不等于"平台接受"。凡是要**交给外部系统**的配置（workflow、schema、清单），
本地判据必须校验**该外部系统的接受规则**，而不是"我这边没报错"。

### 第七个坑：失败**看不到原因**，是因为输出被重定向进了 step summary

`bench` 档连续两次失败、日志里**一行输出都没有**。我据此先后诊断为"进程在第一条 echo 前就死了"和"超时" ——
**两次都错**。真实原因是该步骤末尾写着
```yaml
} | tee /tmp/rss-summary.md >> "$GITHUB_STEP_SUMMARY"
```
于是整块 stdout 被重定向进 step summary 文件，而 `gh run view --log-failed` **不会**返回 summary 内容。

**修法（已做）**: 先 `} 2>&1 | tee "$LOG"`（进日志），再把 `$LOG` 追加进 summary。

**教训**: 一个"可能失败"的步骤**必须**把输出留在**日志**里。诊断"无输出"时，第一件事是问**它的 stdout 去哪了**，
而不是猜挂起/瞬时崩溃；另外，在 CI 日志里 grep 时要**排除被回显的脚本体**（它按构造就含有同样的词 ——
我曾用 `grep timeout` 命中脚本里字面的 `TIMEOUT=900`，把它当成"真的超时了"）。

### 第八个坑：**总是运行**的步骤把输出只送进 step summary ⇒ 日志里一无所见

第七个坑讲的是"某一步失败时看不到原因"；这一条是它的**系统性版本**：仓库里曾有多处步骤写成

```yaml
} >> "$GITHUB_STEP_SUMMARY"
```

于是**整块输出只进 job summary**，`gh run view --log` 只回显脚本体。实测撞到过两次：`pending` 档（列 PENDING 清单的那一步）
与 `inventory` 档（渲染门禁表的那一步）—— **它们的全部产物都在 summary 里**，日志里读不到任何东西。

**判据（已并入 G13，守卫数仍是 14）**: **总是运行**的步骤若 `run` 里含 `GITHUB_STEP_SUMMARY` 却**不含 `tee`** ⇒ 报警。
`if: failure()` 守卫的"失败摘要"步骤**豁免** —— 它们抄的是失败步骤**自己日志里已有**的内容。

**修法**: 先 `} 2>&1 | tee "$LOG"`，再把**同一份字节** `cat` 进 summary（summary 内容不变，日志从此可读）。

**这条判据第一次运行就抓到 4 处真站点**（bench/windows/determinism/site-deploy），修完复跑为 0 —— 牙测不是人造注入，
而是**真站点**。

### 方法教训（比坑本身更值钱）：**数命中不等于评估严重度**

我先用 grep 数出"13 处 summary 重定向"，然后**手工分类**得出"只有 3 处总是运行、其余是失败复制品"。
结果 G13 按 YAML 正确读取后指出：**我手工分类把两处判错了** —— 我向前回溯找最近一条 `if: failure()`，
于是把**前一个步骤**的守卫算到了它们头上。

⇒ 本会话第三次（第 92、100、126 轮）"手搓扫描把自己扫的东西描述错"。**写判据比读文件可靠**：
机械判据第一次跑就对了，而我的手读错了两次。凡是**可机械判定**的规则，就该落进 `scripts/guards/`，
而不是靠下一次仔细阅读。

