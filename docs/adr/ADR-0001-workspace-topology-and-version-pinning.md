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

### D10（补充）— `schemas/*.json` 与规范冲突时，以**规范**为准，改契约

`schemas/` 是机器校验契约，它必须描述我们真正要的东西。当契约与规范/ADR 冲突时：

1. **收紧**契约到规范要求的形状（不是放宽成 union 去兼容一个从未存在过的历史格式）；
2. 在契约里留下指向裁决的注释或 description；
3. 把冲突本身记进本 ADR（不许悄悄改一边）。

### D11 — `writer_version` 是语义版本字符串，不是整数

- **冲突**：`schemas/project.schema.json` 原本声明 `writer_version: integer`；架构 §2.2 与 D3 裁决它是
  形如 `"0.0.1"` 的**字符串**。`yeban-model` 的规范样本对账实测：只有这一项红，其余 required/pattern/range/enum 全过。
- **裁决**：契约改为 `type: string` + semver 形状的 `pattern`（**不是** `["integer","string"]`）。
- **代价**：若真有外部整数版本号文档，需要迁移器（`ROAD-M1-005` 预留了 `src/migration/`）。

### D12 — `Op` 全集补两个删除变体：`RemoveSection` / `RemoveScene`

- **缺口**：架构 §6.1 用 `SetSection { old_section: Option<_> }` 表达"新建"（`None`），但全集中**没有**
  删除段落/场景的变体 ⇒ 新建这一步的逆操作无法表达 ⇒ `MUST-GATE-010`（10,000 步撤销守恒）必然失败。
- **裁决**：补 `Op::RemoveSection` / `Op::RemoveScene`，语义完全由 `SetSection` 的逆定义（无自由度）。
- **同步**：`schemas/ops.schema.json` 的 `op.oneOf` 由 14 个变体补齐到 **23 个**（覆盖架构 §6.1 全集 + 这两个）。
- **待人类批准**：这是对规范 Op 全集的**扩展**，需要回写进架构 §6.1。Agent 不擅自改规范正文。

### D13 — `OpOrigin::McpProposal` 用**外部标签对象**序列化

- **冲突**：`schemas/ops.schema.json` 原本把 `origin` 声明为 7 值字符串 enum，但 `McpProposal` 携带
  `{ proposal_id, agent_name }` 结构化载荷。
- **裁决**：`origin` 的契约改为 `oneOf`：6 个单元变体仍是纯字符串（`"UserUi"` …），
  `McpProposal` 序列化为外部标签对象 `{"McpProposal":{"proposal_id":"…","agent_name":"…"}}`。
  这既保留了 serde 的默认（externally tagged）行为，也让契约与实现一致。
- **代价**：消费方必须按 `oneOf` 处理两种形状；换来的是不丢载荷。

### D14 — 流派规则库的数量以**实测值**为准，规范正文的"159 种"视为下限

- **冲突**：规范正文多处写"159 种流派规则"，而 `yeban-theory` 实际落地 **182 条**（全部自行编码、按来源分类登记）。
- **裁决**：数量由 crate 内的机械判据钉住（`library_size_is_pinned_to_the_measured_number`），
  规范里的"159"读作"**至少** 159 种"；实际数字记入 `docs/DEVELOPMENT_LEDGER.md` 与
  `docs/ledger/theory-core-notes.md`。**不修改规范正文**。
- **代价**：对外文案不得再写死任何数字（`Cargo.toml` 的 description 已改为不带数字的"流派规则库"）。

### D15 — `PPQ = 960` 允许在两个 crate 各有一份，但必须被对账

- **现状**：`yeban-model` 与 `yeban-theory` 各定义一份同值常量；为这一个整数新增
  `theory → model` 的依赖不划算（会让纯函数层依赖整个数据模型）。
- **裁决**：允许重复，但**必须**有独立对账：集成属性测试以"4/4 一小节 = 3840 tick"验证两者一致。
  日后再出现第三份时必须同样对账，否则合并为单一来源。

### D16 — 循环点微平滑窗以**规范公式**为准（π 版），不是对称 Hann 窗

- **冲突**：规范 §3.3 给的是 `w(n)=½[1−cos(πn/(N−1))]`（`w(0)=0, w(N−1)=1`，N=64 时 `w(32)≈0.5125`），
  而早期任务书里的示例判据 `w[0]==w[N−1]==0, w[N/2]≈1` 描述的是 2π 对称 Hann 窗。
- **裁决**：**以 Normative 规范为准**（π 版），"首尾相接无跳变"由**互补窗对**实现
  （fade-in 用 `w`，fade-out 用 `w` 的镜像）。差异已记入 `docs/ledger/dsp-core-provenance.md` §5.1。
- **待人类确认**：若产品意图是对称 Hann 窗，需要改规范 §3.3 —— 那属于人类对规范的修改，Agent 不代改。

### D17 — 依赖许可白名单接纳 `BSL-1.0`（Proposed，待人类法务追认）

- **背景**：引入 Slint 后，`clipboard-win` / `error-code`（← `arboard` ← winit/slint，Windows 目标）
  声明 `BSL-1.0`（Boost Software License 1.0），被 `cargo deny` 拒绝。
- **裁决**：加入白名单。判定依据：BSL-1.0 是 OSI 认证 + FSF Free/Libre + 宽松 + 与 GPLv3 兼容，
  **不属于** AGENTS.md §2 红线 2 禁止的三类（非商业限制 / 专有不可再分发 / 不兼容 GPLv3）。
- **诚实声明**：提出该修改的是 Agent（集成者）。"接纳一个新许可"在精神上属于 `ROAD-M-1-006`
  的人类判断范畴，因此本裁决状态为 `Proposed`：人类可以否决，否决时回滚 `deny.toml` 中该行，
  并改由 CI 安装 `libfontconfig`/改用其它剪贴板方案来绕行。
- **同类前置**：Slint 在 Linux 上还需要系统 `fontconfig` 开发库（见 `docs/CI_CD.md` §3.2），
  已由 CI 统一安装，而不是在各 crate 里加 feature 垫片。

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

1. 本 ADR 全部裁决（尤其 D3 的 `schema_version = 1`、D7 的 PENDING 策略、D12 对 `Op` 全集的扩展、
   D16 的窗函数口径、D17 的 BSL-1.0 接纳）；
2. `ROAD-M-1-006` 的 4 条人类审核（法务措辞、ASIO、商标、发布签名）——Agent 不得代签；
3. 自托管固定频率 runner 的预算与接入时间（决定 BASELINE 与确定性门禁何时接线）；
4. `website` 分支的 Cloudflare 凭据（`CLOUDFLARE_API_TOKEN` / `CLOUDFLARE_ACCOUNT_ID`）；
5. `LEGAL.md` / `GOVERNANCE.md` 里 6 处失效的 `file:///home/crow/work/agy/review/...` 绝对链接
   （AGENTS.md §2 红线 1 禁止 Agent 修改这些文件，因此 `scripts/gates/check_docs_links.py`
   对它们**只告警不阻断**，等人类负责人修复）。
