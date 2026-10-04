# ADR-0001 — 工作区拓扑、命名与版本钉死

- **状态**: Proposed（在人类负责人批准前，Agent 一律照此执行）
- **日期**: 2026-10-05
- **依据**: `AGENTS.md` §1/§2/§4、`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §8、`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` §3
- **背景**: 四份 Normative 规范之间以及规范与现实之间存在若干必须裁决的冲突与缺口。本 ADR 把裁决、依据与代价一次写清，避免每条工作线各自发明一套答案。

---

## 裁决一览

| # | 问题 | 裁决 | 依据与代价 |
| :-- | :--- | :--- | :--- |
| D1 | `yeban-ui-test-port` / `yeban-ui-mcp` 在 architecture §8 的 workspace 树里**没有**，但 §0.4/§12 反复要求 | **列为正式 workspace 成员** `crates/yeban-ui-test-port`、`crates/yeban-ui-mcp` | 取"功能需求优先于目录清单"：§12 的 DoD（UI 变更双重验证）没有这两个 crate 无法闭环。代价：crate 数比 §8 多两个 |
| D2 | `yeban-app/ui/` 文件清单两处不一致：architecture §8 给 4 个文件，UI/UX §8 给 11 个 | **以 UI/UX §8 的 11 个 `.slint` 为准**，architecture 的 4 个名字被包含关系取代（`pianoroll.slint`→`console/piano_roll.slint`，`mixer.slint`→`console/mixer_console.slint`）；24px 状态栏新增 `status_bar.slint` | UI/UX 规范更晚、更细、且是 UI 领域的专门规范。代价：实现时不得再出现根目录级的 `pianoroll.slint` |
| D3 | `schema_version` 字面值：规范注释写"固定为 3"，类型名却是 `YebanProjectV1` | **首个稳定文档 schema 版本 = `1`**；`writer_version` 取应用语义版本字符串（当前 `0.0.1`） | 规范正文的 v1.0.0-rev1/rev2 已把 Groove v3 编号**整体重基**为 `v0.0.1` 起步/`v1.0.0` 首发；而 `3` 是重基前的遗留数字。且历史上**从未发布过** `schema_version = 2/3` 的文档，所以"迁移兼容"没有对象。代价：若确实存在外部 v3 文档，需补一个迁移器（`ROAD-M1-005` 已预留 `src/migration/`） |
| D4 | 性能衰退阈值冲突：roadmap §5.3/§6.4 写 5%，`AGENTS.md` DoD 4 写 3% | **合并阻断用 3%**（更严者胜）；5% 只作为发布列车的容忍带 | `AGENTS.md` 是 Agent 的执行契约，DoD 明文写 3%；规范冲突时取严不取宽，且不需要修改任何 Normative 文档 |
| D5 | 依赖版本未钉死（slint / cpal / clack / zip / hound …） | **唯一事实源 = 根 `Cargo.toml` 的 `[workspace.dependencies]`**，成员一律 `foo.workspace = true`；`i-slint-backend-testing` 用 `=x.y.z` 与 `slint` 严格同版本 | 版本决策集中一处，任何一条工作线都不需要为了改版本去动共享文件（多线并行的关键）。测量记录见 `docs/DEVELOPMENT_LEDGER.md` |
| D6 | `ulid` 3.0.0 没有 `serde` feature，且 API 是 `Ulid::generate()` 而非规范写的 `Ulid::new()` | `EntityId` 的 `Serialize/Deserialize` **手写**，直接满足"26 字符 Crockford Base32 + 大小写不敏感"；`EntityId::new()` 内部调 `Ulid::generate()` | 规范要求的是**行为**（26 字符、大小写不敏感），不是某个 crate 的 API 形状。把行为固定在自己的类型上，上游 API 变动不再影响工程文件格式。代价：`ulid` 升级时只需重新验证 `ids.rs` 的测试 |
| D7 | 基准性能矩阵要求固定频率参考机（M2 Pro 12 核 / Ryzen 7840HS），但 GitHub 托管 runner 频率不固定 | **正确性门禁**全跑 GitHub 托管 runner；**BASELINE-001..006 与 L1/L2 确定性对账**标记 `PENDING`，等自托管固定频率 runner 就位 | 用托管 runner 的读数去判定基准达标是自欺（SKILL「本地绿不是绿」的同类错误）。在没有合格硬件之前，这些门禁**明确不通过**而不是假装通过 |
| D8 | `ROAD-M-1-006` 正文列 4 条人类审核项，修订记录说"三项" | 按正文的 **4 条**执行（法务 GPLv3 §7 措辞、ASIO 法务确认、商标查重、BDFL 发布签名） | 正文比修订摘要详细，且 4 条都真实存在。代价：发布前的人类工作量按 4 条计 |
| D9 | Phase 0 的 9 个 Spike 放哪 | 独立 `spikes/spike-0N-*/` 成员 crate，**各自一个目录，永不共享文件** | 9 条工作线可以真正并行（SKILL：一个文件只能有一个写者）。代价：spike 代码不进产品 crate，验证结论需被"汲取"进 `crates/` 后 spike 才能退役 |

---

## D5 的落地细节（版本钉死）

测量时刻：**2026-10-05**，方法：`https://crates.io/api/v1/crates/<name>` 的 `max_stable_version`。

| crate | 钉死版本 | 备注 |
| :--- | :--- | :--- |
| slint / slint-build | 1.18.1 | 要求 Rust ≥ 1.92（因此工作区 MSRV 定为 1.92） |
| i-slint-backend-testing | `=1.18.1` | 内部 crate，不遵循 semver，必须精确同版本 |
| cpal | 0.18.2 | |
| rtrb | 0.4.0 | 唯一 UI↔音频无锁通道 |
| symphonia | 0.6.1 | MPL-2.0，MSRV 1.85 |
| rubato | 5.0.1 | |
| ulid | 3.0.0 | 见 D6 |
| serde / serde_json | 1.0.229 / 1.0.151 | |
| thiserror | 2.0.21 | |
| sha2 | 0.11.0 | CAS 摘要 |
| proptest | 1.11.0 | 关掉默认的 fork/timeout，纯数据模型不需要进程 fork |
| criterion / iai-callgrind | 0.8.2 / 0.16.1 | |
| zip / flate2 / hound / midly / midir / notify / rstar / signalsmith-stretch | 8.6.0 / 1.1.10 / 3.5.1 / 0.5.3 / 0.11.0 / 8.2.0 / 0.13.0 / 0.1.3 | 尚未被任何成员引用，因此在 `Cargo.lock` 里还看不到 |

**未采纳的替代**：`synth` 里手工重写的 `dsp/ladder.rs` 取代了 vendored DaisySP，方向正确但那是 C/C++ 混合；
夜半坚持纯 Rust，因此不引入 DaisySP / Soundpipe（见 `ledger/legacy-reuse-audit.md`）。

---

## D10（补充）— 本机与 CI 的分工，写进规范级别的纪律

- 本机是 Apple M2 **开发/编辑**环境。允许：`cargo fmt`、无重依赖 crate 的 `clippy/test`、全部守卫脚本。
- 禁止在本机跑：`--workspace` 全量构建、Slint / cpal / symphonia 编译、基准、模糊测试、跨架构对账。
- 上述限制由 `scripts/dev/cargo-local.sh` 与 `scripts/gates/run-gates.sh light` **机械执行**，不靠自觉。
- 任何"绿"只有来自 CI 的 run 才算数；未读取的判决记为 `pending`。

## 待人类批准/补充

1. 本 ADR 全部裁决（尤其 D3 的 `schema_version = 1` 与 D7 的 PENDING 策略）；
2. `ROAD-M-1-006` 的 4 条人类审核（法务措辞、ASIO、商标、发布签名）——Agent 不得代签；
3. 自托管固定频率 runner 的预算与接入时间（决定 BASELINE 与确定性门禁何时接线）；
4. `website` 分支的 Cloudflare 凭据（`CLOUDFLARE_API_TOKEN` / `CLOUDFLARE_ACCOUNT_ID`）。
