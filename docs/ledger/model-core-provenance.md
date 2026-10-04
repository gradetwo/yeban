# `model-core` 工作线：移植来源与决策记录

- **工作树**: `/Users/crow/work/music/yeban/.worktrees/model-core`（分支 `line/model-core`）
- **拥有范围**: `crates/yeban-model/**`；本文件 `docs/ledger/model-core-provenance.md`
- **未触碰**（多线纪律）: 根 `Cargo.toml` / `Cargo.lock`（本次无需改动） / `AGENTS.md` /
  `README.md` / `.github/**` / `scripts/**` / `docs/DEVELOPMENT_LEDGER.md` /
  `schemas/**` / 其它 `crates/**` / `spikes/**` / 全部法务文件
- **规范依据**: `AGENTS.md` §2 红线 3/4/8、§3 DoD；
  `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §2（`MODEL-AST-001..005,007`、`MODEL-ISO-001`）、
  §6（`ARCH-OPS-001/002`）；`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md`
  `ROAD-M1-001..006`、`MUST-GATE-010`；`docs/adr/ADR-0001-*.md` D3/D6/D10/D11/D12/D13

> **2026-10-05 第二轮（合并后切片）**：本线已被集成者合并进 `main`，且 §3 的四处冲突
> 全部裁决落地（`schemas/project.schema.json` 与 `schemas/ops.schema.json` 已更新）。
> 本轮新增了**非测试**的样本导出入口，并把这轮的对账口径记在 §3.1 / §4.6。

---

## 1. 落地清单（文件 ↔ 规范 ID）

| 文件 | 内容 | 规范 ID |
| :--- | :--- | :--- |
| `crates/yeban-model/src/music.rs` | `MidiNote` 全字段 + 全部范围校验 + `triggers()` 确定性概率触发 + `SlideConfig` / `CurveType` | `MODEL-AST-005`、`ARCH-DET-001` |
| `crates/yeban-model/src/project.rs` | `YebanProjectV1` 及全部子类型、版本门 `check_readable()`、`validate()`、`RoutingGraph`（内存 BTreeMap ↔ JSON 数组）、样本导出 | `MODEL-AST-002/003/004/007`、`MODEL-ISO-001`、`ROAD-M1-001/002`、`RSK-16` |
| `crates/yeban-model/src/ops.rs` | `OpOrigin`、`StampedOp`、`Op` 全 23 变体、`precondition`/`apply`/`invert`/`apply_inverse`、`Batch` 原子性、`proptest` 状态守恒 | `ARCH-OPS-001`、`ROAD-M1-003/006`、`MUST-GATE-010`、`TEST-SPEC-001` |
| `crates/yeban-model/src/commit.rs` | `Commit`、`CommitGraph`、`CommitDraft`、`UndoCursor`、每 256 提交快照策略、匿名分支分叉、跨 Commit 边界连续撤销 | `ARCH-OPS-002`、`ROAD-M1-003` |
| `crates/yeban-model/src/error.rs` | 补齐范围/存在性/逆操作类错误变体 | `AGENTS.md` §3 DoD 3 |
| `crates/yeban-model/src/ids.rs` | 新增 `AssetHash` / `ContentHash` 的 `Display`（`thiserror` 的 `{key}` 插值需要它） | `MODEL-AST-007` |
| `crates/yeban-model/src/samples.rs` | **规范样本的唯一实现**：`default_project` / `filled_project` / `default_stamped_op` / `filled_stamped_op` + `export_all`（写前先做 Rust 侧结构校验） | `TEST-SPEC-005`、`MUST-GATE-010` |
| `crates/yeban-model/examples/export_schema_samples.rs` | **非测试**可执行入口：`--out <dir>`（默认 `target/schema-samples`），与测试走**同一个** `samples::export_all` | `TEST-SPEC-005` |
| `crates/yeban-model/src/lib.rs` | 挂载 `commit`/`music`/`ops`/`project`/`samples` 并 `pub use` 主要类型 | — |

**测试映射**: 86 个单元/属性测试，全部内联在对应模块的 `#[cfg(test)]` 中
（`cargo test -p yeban-model` 一栏可见逐条测试名）。
其中**契约驱动**的两条判据（`ops::tests::op_variants_match_ops_schema_exactly`、
`origin_variants_match_ops_schema_origin_one_of`）与
`samples::tests::ops_samples_match_the_ops_contract_variant_list` **直接读
`schemas/ops.schema.json`**，不手抄第二份变体清单。

---

## 2. 移植来源（provenance）

**没有从任何第三方项目移植代码。** 本线全部实现为原创 Rust，无 vendored 代码、无
C/C++ 混合。

唯一"外来算法"是 **splitmix64**（Steele/Lea 等人的 SplitMix 论文，公有领域）：

- 手写实现在 `crates/yeban-model/src/music.rs::splitmix64`，共 5 行，纯整数运算；
- 为什么手写而不引依赖：规范要求 `probability` 结合 `rng_seed + note.id` 计算
  （`MODEL-AST-005`），一旦引入平台 RNG 或线程局部 PRNG 状态，同一工程在两台机器上
  会渲染出不同音频，直接违反 `ARCH-DET-001`（L1/L2 声学确定性契约）。
  手写 5 行整数函数比"引入 `rand_xoshiro` + `rand_core` 并逐版本审计其 cross-platform
  保证"更容易证明确定性。
- 正确性证据：`music::tests::splitmix64_matches_published_vector` 钉住公开测试向量
  （种子 0 → `0xE220A8397B1DCDAF`，种子 1 → `0x910A2DEC89025CC1`，用独立 Python 复算过）。

**第三方依赖**: 本线**未新增任何依赖**。`serde` / `serde_json` / `sha2` / `thiserror` /
`ulid`（运行期）与 `proptest`（测试期）全部沿用根 `Cargo.toml`
`[workspace.dependencies]` 里已登记的版本，因此：

> **TODO(hoist): 无。** 本次不需要集成者把任何版本上提到根 `workspace.dependencies`。
> `Cargo.lock` 也因此**没有变化**（`cargo metadata --locked` 仍然成立）。

> 备注：根 `[workspace.dependencies]` 已登记 `rand_xoshiro = 0.8.1` 与 `libm = 0.2.16`，
> 但本线**刻意不使用**它们（理由见上）。这两条登记未被本线消费，不影响 lockfile。

---

## 3. 决策记录（本地编号 M1..M12）

> **编号约定**：本节是**本工作线自己**的决策编号 `M1..M12`。
> `docs/adr/ADR-0001-*.md` 的 `D11/D12/D13` 是合并后由集成者落地的**契约裁决**（见 §3.1）。
> 两套编号刻意区分，避免"D12 到底指哪一个"的歧义。

### M1 —— `writer_version` 取语义版本字符串（`String`），与 `project.schema.json` 冲突

- **冲突**: `schemas/project.schema.json` 把 `writer_version` 声明为
  `{"type": "integer", "minimum": 1}`；而 `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md`
  §2.2 写 `pub writer_version: String`（注释 `"3.0.0"`），
  `docs/adr/ADR-0001-*.md` D3 明确裁决"`writer_version` 取应用语义版本字符串（当前 `0.0.1`）"。
- **取舍**: 按架构正文 + ADR D3（更晚、更专门的裁决）实现为 `String`。
  依据 `AGENTS.md` §1：Normative 文档 > 机器契约草稿；且 ADR 是规范冲突的裁决留痕场所。
- **实测影响面（Python jsonschema 4.24 + Draft 2020-12）**:

  ```text
  $ python3 scripts/gates/validate_schemas.py --samples-dir target/schema-samples
    - project.default.json: 违反 project.schema.json @ writer_version: '0.0.1' is not of type 'integer'
    - project.filled.json:  违反 project.schema.json @ writer_version: '0.0.1' is not of type 'integer'
  ```

  **分歧恰好只有这一项**：其余 required 键、`id` 的 ULID pattern、`bpm`/`pan` 范围、
  拍号分子分母、全部 enum、`tracks`/`clip_pool` 为 JSON 对象、`routing_graph.edges`
  为 JSON 数组，全部通过。
- **机械化的"分歧不许变大"判据**: `project::tests::only_writer_version_diverges_from_the_schema_type_table`
  把手抄的契约类型表与 serde 实际输出的 JSON 类型逐项对账，**任何新增的类型漂移都会变红**
  （已用"把 `title` 的期望类型改成 `integer`"验证过它会红）。
- **给集成者的一行修法（二选一，本线无权改 `schemas/`）**:
  1. 放宽契约：`"writer_version": { "type": ["integer", "string"], ... }`（推荐，保住 ADR 裁决）；
  2. 改裁决：把 ADR D3 改成整数版本号，本线把字段改成 `u32` 即可（改动 1 个字段 + 1 条测试）。

### M2 —— `Op` 全集缺"删除段落/场景"，本线补齐 `RemoveSection` / `RemoveScene`

- **规范缺口**: 架构 §6.1 的 `SetSection { old_section: Option<SectionV3>, new_section: SectionV3 }`
  用 `old_section == None` 表达"新建"，但全集里**没有**删除段落/场景的变体。
  于是"新建段落"这一步的逆操作无法表达，`MUST-GATE-010`（状态树逆向幂等性）**必然失败**。
  `SetScene` 完全同理。
- **处置**: 补 `Op::RemoveSection { section_id, previous_section }` 与
  `Op::RemoveScene { scene_id, previous_scene }`；`SetSection{old: None}` 的逆就是
  `RemoveSection`。这是"补齐缺口"而不是"发明新语义"，两个变体的语义完全由 `SetSection`
  的逆定义，无自由度。
- **需要人类/集成者确认**: 是否接受把这 2 个变体写回架构 §6.1（本线不得改 Normative 文档）。

### M3 —— `RoutingGraph.edges`：内存 `BTreeMap`、JSON 数组

- 红线 4 / `MODEL-AST-003` 要求持久化实体集合是 `BTreeMap`；
  `schemas/project.schema.json` 要求 `routing_graph.edges` 是 **array**。
- 两者同时满足：内存仍是 `BTreeMap<EntityId, RoutingEdge>`，序列化时按键升序展开为数组
  （`impl Serialize for RoutingGraph`）。数组顺序因此是确定的，同一工程两次导出逐字节相同
  （`project::tests::routing_edges_serialize_in_key_order_and_round_trip`）。
- `nodes` 保持数组 + 元素为字符串（契约要求），反序列化时校验端点都在 `nodes` 里。

### M4 —— `TrackV3.automation_lanes` 的 JSON 形态是数组

`serde_json` 的对象键必须是字符串，而 `AutomationTarget` 是结构化枚举（序列化为对象），
当不了键。因此该 `BTreeMap<AutomationTarget, AutomationLane>` 通过
`automation_lane_map` 序列化为**按键升序的数组**，反序列化收拢回 `BTreeMap`
（重复 target 即拒绝）。内存形态仍满足红线 4。

### M5 —— `OpOrigin::McpProposal` 带载荷，与 `ops.schema.json` 的 `origin` 字符串枚举冲突

- 架构 §6.1 要求 `OpOrigin::McpProposal { proposal_id, agent_name }`（审计需要）；
  `schemas/ops.schema.json` 把 `origin` 声明为 7 值**字符串** enum。
- **本线取舍**: 按架构实现（保留载荷）。因此 `origin` 对 `McpProposal` 会序列化成
  外部标签对象 `{"McpProposal": {...}}`，**不满足** ops 契约的 `origin` 字符串枚举。
- **本线未产出 `ops.<name>.json` 样本**（避免交付一个明知会失败的样本）；
  `ops.schema.json` 的这处冲突留给集成者/人类裁决：或者放宽 `origin`
  （`oneOf: [enum, object]`），或者采纳"origin 只存字符串 + 载荷另置字段"的替代设计。

### M6 —— `ops.schema.json` 的 `oneOf` 缺 9 个变体（实测）

契约的 `op.oneOf` 只列了 14 个变体：`AddNote`、`DeleteNote`、`MoveNote`、
`AddClipPlacement`、`RemoveClipPlacement`、`MoveClipPlacement`、`AddTrack`、`RemoveTrack`、
`ConnectRouting`、`DisconnectRouting`、`SetRoutingGain`、`SetParam`、`SetMacro`、`Batch`。

**未列出的 9 个**（架构 §6.1 有，本线也实现了）：`ModifyNoteVelocity`、`InsertDevice`、
`RemoveDevice`、`SetAutomationPoint`、`RemoveAutomationPoint`、`SetSection`、`SetScene`，
以及 M2 补齐的 `RemoveSection`、`RemoveScene`。

由于 `oneOf` 要求"恰好匹配一个"子模式，这 9 个变体在契约下会匹配 **0** 个子模式而整份文档判违规。
`ops::tests::op_variant_json_keys_match_ops_schema` 把"契约已列出的 14 个"与"契约缺口
的 9 个"都显式列成断言，缺口不会被静默吞掉。

### M7 —— 属性测试的规模由环境变量与 CI 决定

- `YEBAN_PROPTEST_CASES=<n>` 显式指定操作序列长度；
- 否则若存在 `CI` 环境变量 → `10_000` 步（满足 `MUST-GATE-010` 的"10,000 步"）；
- 否则本机默认 `256` 步（用户硬性要求：本机不跑高耗 CPU 任务）。
- 因为 `.github/**` 归集成者所有，本线**没有**改 CI 配置，而是让"CI 环境自动放大"，
  无需修改任何共享文件。实测：本机 `CI=true cargo test -p yeban-model` 耗时约 16s。
- `ProptestConfig` 里设 `failure_persistence: None`：避免 proptest 在 `src/` 下写
  `proptest-regressions/` 目录污染源码树。

### M8 —— 随机操作序列用"种子驱动的状态感知随机游走"，而不是纯策略

每一步的载荷都**从当前文档**推导，因此生成的操作**必然可应用**（测试里
`op.apply(...).expect(...)` 把这条当作不变量断言）。这样 10,000 步全部是有效步骤，
不存在"绝大多数操作被跳过导致判据空洞化"的风险。proptest 负责种子生成与收缩。

### M9 —— `UndoCursor` 刻意不实现 `Serialize`/`Deserialize`

连续 `Cmd+Z` 需要一个游标记住"已经撤销到哪儿"。它属于**会话运行态**
（`MODEL-ISO-001` 的第二层），严禁持久化，因此从类型上就不给它 serde 实现。

### M10 —— `SetParam` 拒绝 `AutomationTarget::SendGain`

发送增益是 `Option<f32>`（`None` = 单位增益）。若允许 `SetParam` 写它，`None` 与
`Some(0.0)` 无法区分，撤销无法精确还原。因此 `SetParam` 对 `SendGain` 目标返回
`ModelError::AutomationTargetNotApplicable`，发送增益必须走 `Op::SetRoutingGain`
（保留 `Option` 语义）。有专门测试覆盖。

### M11 —— `Op::Batch` 的原子性用"克隆体整体模拟 + 一次性提交"

`Batch::apply` 在 `doc.clone()` 上按序应用全部子操作，全部成功才 `*doc = probe`。
失败时原文档**分毫不动**（`ops::tests::batch_is_atomic_when_a_sub_op_fails` 覆盖），
对应 `ARCH-OPS-002` 的"AI 提案一键撤销"。
`Batch::precondition` 因此是真空真（原子性检查只做一次，不做两遍全文档克隆）。

### M12 —— 生产代码不使用 `HashMap`/`HashSet`

`scripts/guards/policy_check.py` 的 G01 只扫描 `#[cfg(test)]` 之前的代码；
本线在**测试里也不用**哈希容器，全部用 `BTreeMap`/`Vec`，以免留下"测试能用生产不能用"
的灰区。

---

### 3.1 ADR-0001 的裁决落地（本线第二轮）

集成者把 §3 的四处冲突全部裁决并改进了 `schemas/`（本线不得改契约，故只记录结果）：

| ADR | 裁决 | 本线随之的动作 |
| :--- | :--- | :--- |
| **D11** | `project.schema.json` 的 `writer_version` 改为 `type: string` + semver `pattern`（**收紧**，不是 `["integer","string"]` 联合） | 一行代码未改（实现本来就是 `String`）；把 `projects.rs` 里那张手抄的"契约类型表"删掉，改为**直接读契约文件** |
| **D12** | `Op` 全集补 `RemoveSection` / `RemoveScene`；`ops.schema.json` 的 `op.oneOf` 由 14 个补齐到 **23 个**；架构 §6.1 的回写列入"待人类批准" | `ops.rs` 的模块文档从"缺口留痕待确认"改为"已裁决"；契约一致性测试改为读契约文件 |
| **D13** | `ops.schema.json` 的 `origin` 改为 `oneOf`：6 个单元变体是纯字符串，`McpProposal` 是外部标签对象 `{"McpProposal":{"proposal_id":…,"agent_name":…}}` | `OpOrigin` 的序列化形状**未改一行**（serde 默认就是外部标签），新增 `origin_variants_match_ops_schema_origin_one_of` 钉住它 |

**结论**：`project.*` 与 `ops.*` 四份样本现在**全部**通过 Python jsonschema 对账，实测输出：

```text
$ python3 scripts/gates/validate_schemas.py --samples-dir /tmp/yeban-samples
[ok] ops.default.json: 通过 ops.schema.json
[ok] ops.filled.json: 通过 ops.schema.json
[ok] project.default.json: 通过 project.schema.json
[ok] project.filled.json: 通过 project.schema.json
契约校验通过 (4 份 schema)。
```

### 3.2 样本导出：**非测试**入口与对账口径

样本的**唯一实现**是 `crates/yeban-model/src/samples.rs::export_all`，它有两个入口：

1. `crates/yeban-model/examples/export_schema_samples.rs`
   （`cargo run -p yeban-model --example export_schema_samples -- --out <dir>`）；
2. `project::tests::export_schema_samples_to_target`（本机门禁顺带产出，CI 目前用它喂对账）。

两者调用同一个函数，因此不存在"测试里那套"与"CI 里那套"两份实现。
`export_all` 在写盘**之前**先做 Rust 侧结构校验（工程样本 `validate()` + `check_readable()`、
op 样本"恰好一个变体键"），因此磁盘上不会出现一份连自己都不合法的样本。

**对账文件名约定**（`scripts/gates/validate_schemas.py` 的 `SAMPLE_SCHEMA_MAP`）：

| 文件 | schema | 内容 |
| :--- | :--- | :--- |
| `project.default.json` | `schemas/project.schema.json` | `YebanProjectV1::default()` |
| `project.filled.json` | `schemas/project.schema.json` | 填满全部子类型的工程 |
| `ops.default.json` | `schemas/ops.schema.json` | `UserUi` + `SetSection`（可作用在 default 工程上） |
| `ops.filled.json` | `schemas/ops.schema.json` | `McpProposal` + 原子 `Batch`（可作用在 filled 工程上、且可逆） |

**样本对自洽性**（`samples.rs` 的测试）：`ops.default.json` 能 clean 地作用在
`project.default.json` 上，`ops.filled.json` 能作用在 `project.filled.json` 上**并精确撤销**。
样本因此不只是"形状合法"，还是一组真实的、可逆的领域操作。

**CI 现状**：`checks` job 的"跨语言契约对账"步骤目前仍然靠 `cargo test -p yeban-model`
生成样本、再跑 `validate_schemas.py --samples-dir target/schema-samples`。
新增的 example 入口是给集成者切换用的（`.github/**` 归集成者所有，本线不改）。

---

## 4. 判据实测（本地，命令 + 结果）

本机受限于沙箱（rustup 无法写 `~/.rustup`），因此统一按
`scripts/dev/cargo-local.sh` 的方式提供工具链与 `CARGO_HOME` 环境变量：

```bash
export CARGO_HOME=/Users/crow/work/music/.cargo-home RUSTUP_TOOLCHAIN=stable

# 1) 格式化
bash scripts/dev/cargo-local.sh fmt --all                                  # exit 0
# 2) clippy（零告警）
bash scripts/dev/cargo-local.sh clippy -p yeban-model --all-targets -- -D warnings   # exit 0
# 3) 测试（本机 128 步）
YEBAN_PROPTEST_CASES=128 bash scripts/dev/cargo-local.sh test -p yeban-model
#    -> test result: ok. 86 passed; 0 failed
# 4) CI 档规模（10,000 步 × 32 cases）
CI=true bash scripts/dev/cargo-local.sh test -p yeban-model                 # 约 16s, 全绿
# 5) 强制门禁（fmt + 11 条红线守卫 + clippy -D warnings + test）
bash scripts/gates/run-gates.sh crate yeban-model                          # exit 0, "门禁通过"
# 6) 非测试入口导出样本（本轮新增）
bash scripts/dev/cargo-local.sh run -p yeban-model \
    --example export_schema_samples -- --out /tmp/yeban-samples
#    -> /tmp/yeban-samples/{project.default,project.filled,ops.default,ops.filled}.json
# 7) 跨实现 schema 对账（Python jsonschema 4.24 / Draft 2020-12）
python3 scripts/gates/validate_schemas.py --samples-dir /tmp/yeban-samples
#    -> 4 份样本全部通过；第一轮（契约修好前）唯一的类型分歧 writer_version 已按
#       ADR-0001 D11 修好。
```

### CI 判决（GitHub Actions，`line/model-core`）

#### 第二轮（合并后切片：非测试导出入口 + ops 样本）

- **run**: `37218504984` · CI · push · **conclusion: success**（28 个 job 全绿，
  `checks` 43s、`rust (yeban-model)` 41s、`deny` 32s，其余 12–15s）
- **关键取证**：`checks` job 的"跨语言契约对账"步骤在 CI 上真的跑了两侧 ——
  Rust serde 写样本、Python jsonschema 读契约：

  ```text
  [ok] ops.default.json: 通过 ops.schema.json
  [ok] ops.filled.json: 通过 ops.schema.json
  [ok] project.default.json: 通过 project.schema.json
  [ok] project.filled.json: 通过 project.schema.json
  契约校验通过 (4 份 schema)。
  ```

  这一步的 `cargo test -p yeban-model` 也留下 `test result: ok. 86 passed; 0 failed;
  ... finished in 13.84s` —— GitHub Actions 默认 `CI=true`，所以那次跑的正是
  **10,000 步**的操作序列。

#### 第一轮（合并前的原始交付）

- **run**: `37217900378` · CI · push · **conclusion: success**（全部 27 个 job 绿，
  含 `rust (yeban-model)` 42s、`checks (fmt/红线守卫/schema)` 14s、`lockfile` 12s、`deny` 47s）
- 该 run 的 `plan` 步骤因为首推分支的 `github.event.before` 是全零 sha 而保守退化为
  **全工作区矩阵**（27 个 crate 全跑），仍然全绿。
- `rust (yeban-model)` 的 test 步骤日志:

  ```text
  test ops::tests::state_tree_is_conserved_under_reverse_undo ... ok
  test result: ok. 76 passed; 0 failed; ... finished in 10.65s
  ```

  GitHub Actions 默认设置 `CI=true`，因此这次跑的正是 **10,000 步**的操作序列
  （本机小档 256 步时整套测试仅 0.02s，10.65s 的量级本身就证明大档被启用）。
- 判决读取方式: `bash scripts/dev/ci-verdict.sh line/model-core`（不是"本地绿"）。

### 判据"先能变红"的三条实证（改坏 → 变红 → 改回）

1. **逆操作正确性（状态守恒）**
   把 `structural_inverse(ModifyNoteVelocity)` 的第二项改成 `new_vel: *new_vel`（即撤销不还原力度）：

   ```text
   Test failed: 撤销第 21 步 (ModifyNoteVelocity) 时求逆失败:
     `ModifyNoteVelocity` does not match the document state (payload / post-condition mismatch).
   minimal failing input: seed = 34472935493095
   test result: FAILED. 0 passed; 1 failed
   ```

   证明 `ops::tests::state_tree_is_conserved_under_reverse_undo` 这条属性判据真的能失败。

2. **范围校验（越界即报错）**
   把 `MidiNote::validate` 里的 pitch 检查删掉：

   ```text
   thread 'music::tests::pitch_out_of_range_is_rejected' panicked:
   assertion `left == right` failed
     left: Ok(())
    right: Err(PitchOutOfRange { value: 128 })
   ```

3. **契约漂移（类型分歧不许变大）**
   把 `SCHEMA_TOP_LEVEL_TYPES` 里 `title` 的期望类型改成 `integer`（模拟契约漂移）：

   ```text
   assertion `left == right` failed: 与 schemas/project.schema.json 的类型分歧必须恰好是已留痕的 writer_version 一项
     left: ["writer_version: schema=integer, actual=string", "title: schema=integer, actual=string"]
    right: ["writer_version: schema=integer, actual=string"]
   ```

   三处改坏后均已还原，最终 `run-gates.sh crate yeban-model` 全绿。

---

## 5. 边界（这次**没有**证明什么）

1. **CI 判决只覆盖这一次推送**：run `37217900378` 全绿，但它证明的是"这次提交在本机纪律下可编译、
   可过门禁"；本文件中"未证明"的其余各条（容器、迁移、基准、fuzz、跨架构对账）依旧没有证明。
2. **契约对账现在是绿的，但生成入口仍是测试**：`project.*` 与 `ops.*` 四份样本
   100% 通过 Python jsonschema（ADR-0001 D11/D12/D13 修好契约之后，见 §3.1/§3.2）。
   但 CI 目前仍用 `cargo test -p yeban-model` 的副作用生成样本 —— 新加的
   `examples/export_schema_samples.rs` 是给集成者切换用的，本线不改 `.github/**`。
3. **`ops.schema.json` 的对账口径来自本线自己**：ops 样本的 `op.oneOf` 一致性由
   `ops::tests::op_variants_match_ops_schema_exactly` 直接读契约文件断言（23 个分支
   与 23 个变体一一对应）。这仍然是"同一份 schema + 同一批变体"的自证，
   **不是**第三方实现的对账。
4. **没有做**：`.yeban` 容器/原子落盘/锁文件（`ROAD-M1-004`）、历史迁移器（`ROAD-M1-005`）、
   跨架构确定性对账（`ARCH-DET-002`）、基准（`BASELINE-*`）、`cargo-fuzz`（`MUST-GATE-011`）。
5. **没有证明**：`triggers()` 的概率分布在统计意义上"正确"（只断言了 p=0.5 在 512 个种子上
   落在宽裕的 1/3..2/3 带内，以及不同身份不给出完全相同的判定序列）；
   也没有证明它与未来真实渲染引擎的取值一致（那属于 `yeban-engine` 的对账）。
6. **没有证明**：`CommitGraph` 的深度缓存与真实存储引擎的快照树一致（快照哈希在模型层是
   一个**确定性替身**：`default_snapshot_hash` 对 ops 做 Debug 文本的 SHA-256）。
7. **没有证明**：并发/多写者安全（模型是单写者纯数据结构，锁由存储层负责）。
8. **没有证明**：撤销是 O(1) 或满足 ≤0.2ms 的单步预算（`ARCH-OPS-002` 写了 0.2ms，
   本机未做基准，`BASELINE-*` 归 CI/自托管 runner）。

---

## 6. needs / pending

1. **需要人类批准（规范文本回写）**: `Op` 全集补了 `RemoveSection` / `RemoveScene`
   （ADR-0001 **D12**），契约已同步，但**架构 §6.1 的正文还没回写** —— 这是对规范 Op 全集的
   扩展，Agent 不擅自改 Normative 文档。请人类把这两个变体写进
   `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §6.1。
2. **需要集成者决定（生成入口切换）**: CI 的 `checks` job 已经接上
   `validate_schemas.py --samples-dir target/schema-samples`，但样本仍由
   `cargo test -p yeban-model` 的副作用产出。建议把生成步骤换成
   `cargo run -p yeban-model --example export_schema_samples -- --out target/schema-samples`，
   这样"导出失败"会以**导出**的失败语义报出来，而不是伪装成测试失败（这正是本轮新增该入口的原因；
   `.github/**` 归集成者所有，本线不改）。
3. **需要集成者上提**: **无**（TODO(hoist) 为空；本线零新增依赖，`Cargo.lock` 无变化）。
4. **样本产物的位置与入库策略**: 样本写到 `<repo>/target/schema-samples/`，而 `target/` 被
   gitignore，因此样本**不进版本库**、每次重新生成（逐字节稳定，有
   `samples::tests::export_all_writes_four_byte_stable_samples` 与本机门禁的
   `export_schema_samples_to_target` 双重断言）。若集成者希望样本入库做 diff 审计，
   需要另行决定存放路径与 `.gitignore` 例外。
5. **本机环境自适应**: `scripts/dev/local-env.sh`（main 上新增）已解决"受限沙箱里裸 `cargo`
   在 fmt 步就失败"的问题，`run-gates.sh` / `cargo-local.sh` 都会 source 它 ——
   本轮已按新脚本跑门禁，**不再需要手动导出 `CARGO_HOME` / `RUSTUP_TOOLCHAIN`**。
   另外它把 `XDG_CACHE_HOME` 指到仓库外的 `.worktrees/.cache`，本线此前误把
   `ci-verdict.sh` 的 gh 日志 zip 提交进仓库（已重写该提交剔除），此问题不会再复发。
6. **未实现的能力切片**（留给后续）：`ROAD-M1-004` 存储引擎、`ROAD-M1-005` 迁移器、
   以及把 `probabilistic` 触发与渲染引擎对账的集成测试。
