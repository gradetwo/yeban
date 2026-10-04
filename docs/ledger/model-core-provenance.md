# `model-core` 工作线：移植来源与决策记录

- **工作树**: `/Users/crow/work/music/yeban/.worktrees/model-core`（分支 `line/model-core`）
- **拥有范围**: `crates/yeban-model/**`；本文件 `docs/ledger/model-core-provenance.md`
- **未触碰**（多线纪律）: 根 `Cargo.toml` / `Cargo.lock`（本次无需改动） / `AGENTS.md` /
  `README.md` / `.github/**` / `scripts/**` / `docs/DEVELOPMENT_LEDGER.md` /
  `schemas/**` / 其它 `crates/**` / `spikes/**` / 全部法务文件
- **规范依据**: `AGENTS.md` §2 红线 3/4/8、§3 DoD；
  `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §2（`MODEL-AST-001..005,007`、`MODEL-ISO-001`）、
  §6（`ARCH-OPS-001/002`）；`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md`
  `ROAD-M1-001..006`、`MUST-GATE-010`；`docs/adr/ADR-0001-*.md` D3/D6/D10

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
| `crates/yeban-model/src/lib.rs` | 挂载 `commit`/`music`/`ops`/`project` 并 `pub use` 主要类型 | — |

**测试映射**: 76 个单元/属性测试，全部内联在对应模块的 `#[cfg(test)]` 中
（`cargo test -p yeban-model` 一栏可见逐条测试名）。

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

## 3. 决策记录

### D1 —— `writer_version` 取语义版本字符串（`String`），与 `project.schema.json` 冲突

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

### D2 —— `Op` 全集缺"删除段落/场景"，本线补齐 `RemoveSection` / `RemoveScene`

- **规范缺口**: 架构 §6.1 的 `SetSection { old_section: Option<SectionV3>, new_section: SectionV3 }`
  用 `old_section == None` 表达"新建"，但全集里**没有**删除段落/场景的变体。
  于是"新建段落"这一步的逆操作无法表达，`MUST-GATE-010`（状态树逆向幂等性）**必然失败**。
  `SetScene` 完全同理。
- **处置**: 补 `Op::RemoveSection { section_id, previous_section }` 与
  `Op::RemoveScene { scene_id, previous_scene }`；`SetSection{old: None}` 的逆就是
  `RemoveSection`。这是"补齐缺口"而不是"发明新语义"，两个变体的语义完全由 `SetSection`
  的逆定义，无自由度。
- **需要人类/集成者确认**: 是否接受把这 2 个变体写回架构 §6.1（本线不得改 Normative 文档）。

### D3 —— `RoutingGraph.edges`：内存 `BTreeMap`、JSON 数组

- 红线 4 / `MODEL-AST-003` 要求持久化实体集合是 `BTreeMap`；
  `schemas/project.schema.json` 要求 `routing_graph.edges` 是 **array**。
- 两者同时满足：内存仍是 `BTreeMap<EntityId, RoutingEdge>`，序列化时按键升序展开为数组
  （`impl Serialize for RoutingGraph`）。数组顺序因此是确定的，同一工程两次导出逐字节相同
  （`project::tests::routing_edges_serialize_in_key_order_and_round_trip`）。
- `nodes` 保持数组 + 元素为字符串（契约要求），反序列化时校验端点都在 `nodes` 里。

### D4 —— `TrackV3.automation_lanes` 的 JSON 形态是数组

`serde_json` 的对象键必须是字符串，而 `AutomationTarget` 是结构化枚举（序列化为对象），
当不了键。因此该 `BTreeMap<AutomationTarget, AutomationLane>` 通过
`automation_lane_map` 序列化为**按键升序的数组**，反序列化收拢回 `BTreeMap`
（重复 target 即拒绝）。内存形态仍满足红线 4。

### D5 —— `OpOrigin::McpProposal` 带载荷，与 `ops.schema.json` 的 `origin` 字符串枚举冲突

- 架构 §6.1 要求 `OpOrigin::McpProposal { proposal_id, agent_name }`（审计需要）；
  `schemas/ops.schema.json` 把 `origin` 声明为 7 值**字符串** enum。
- **本线取舍**: 按架构实现（保留载荷）。因此 `origin` 对 `McpProposal` 会序列化成
  外部标签对象 `{"McpProposal": {...}}`，**不满足** ops 契约的 `origin` 字符串枚举。
- **本线未产出 `ops.<name>.json` 样本**（避免交付一个明知会失败的样本）；
  `ops.schema.json` 的这处冲突留给集成者/人类裁决：或者放宽 `origin`
  （`oneOf: [enum, object]`），或者采纳"origin 只存字符串 + 载荷另置字段"的替代设计。

### D6 —— `ops.schema.json` 的 `oneOf` 缺 9 个变体（实测）

契约的 `op.oneOf` 只列了 14 个变体：`AddNote`、`DeleteNote`、`MoveNote`、
`AddClipPlacement`、`RemoveClipPlacement`、`MoveClipPlacement`、`AddTrack`、`RemoveTrack`、
`ConnectRouting`、`DisconnectRouting`、`SetRoutingGain`、`SetParam`、`SetMacro`、`Batch`。

**未列出的 9 个**（架构 §6.1 有，本线也实现了）：`ModifyNoteVelocity`、`InsertDevice`、
`RemoveDevice`、`SetAutomationPoint`、`RemoveAutomationPoint`、`SetSection`、`SetScene`，
以及 D2 补齐的 `RemoveSection`、`RemoveScene`。

由于 `oneOf` 要求"恰好匹配一个"子模式，这 9 个变体在契约下会匹配 **0** 个子模式而整份文档判违规。
`ops::tests::op_variant_json_keys_match_ops_schema` 把"契约已列出的 14 个"与"契约缺口
的 9 个"都显式列成断言，缺口不会被静默吞掉。

### D7 —— 属性测试的规模由环境变量与 CI 决定

- `YEBAN_PROPTEST_CASES=<n>` 显式指定操作序列长度；
- 否则若存在 `CI` 环境变量 → `10_000` 步（满足 `MUST-GATE-010` 的"10,000 步"）；
- 否则本机默认 `256` 步（用户硬性要求：本机不跑高耗 CPU 任务）。
- 因为 `.github/**` 归集成者所有，本线**没有**改 CI 配置，而是让"CI 环境自动放大"，
  无需修改任何共享文件。实测：本机 `CI=true cargo test -p yeban-model` 耗时约 16s。
- `ProptestConfig` 里设 `failure_persistence: None`：避免 proptest 在 `src/` 下写
  `proptest-regressions/` 目录污染源码树。

### D8 —— 随机操作序列用"种子驱动的状态感知随机游走"，而不是纯策略

每一步的载荷都**从当前文档**推导，因此生成的操作**必然可应用**（测试里
`op.apply(...).expect(...)` 把这条当作不变量断言）。这样 10,000 步全部是有效步骤，
不存在"绝大多数操作被跳过导致判据空洞化"的风险。proptest 负责种子生成与收缩。

### D9 —— `UndoCursor` 刻意不实现 `Serialize`/`Deserialize`

连续 `Cmd+Z` 需要一个游标记住"已经撤销到哪儿"。它属于**会话运行态**
（`MODEL-ISO-001` 的第二层），严禁持久化，因此从类型上就不给它 serde 实现。

### D10 —— `SetParam` 拒绝 `AutomationTarget::SendGain`

发送增益是 `Option<f32>`（`None` = 单位增益）。若允许 `SetParam` 写它，`None` 与
`Some(0.0)` 无法区分，撤销无法精确还原。因此 `SetParam` 对 `SendGain` 目标返回
`ModelError::AutomationTargetNotApplicable`，发送增益必须走 `Op::SetRoutingGain`
（保留 `Option` 语义）。有专门测试覆盖。

### D11 —— `Op::Batch` 的原子性用"克隆体整体模拟 + 一次性提交"

`Batch::apply` 在 `doc.clone()` 上按序应用全部子操作，全部成功才 `*doc = probe`。
失败时原文档**分毫不动**（`ops::tests::batch_is_atomic_when_a_sub_op_fails` 覆盖），
对应 `ARCH-OPS-002` 的"AI 提案一键撤销"。
`Batch::precondition` 因此是真空真（原子性检查只做一次，不做两遍全文档克隆）。

### D12 —— 生产代码不使用 `HashMap`/`HashSet`

`scripts/guards/policy_check.py` 的 G01 只扫描 `#[cfg(test)]` 之前的代码；
本线在**测试里也不用**哈希容器，全部用 `BTreeMap`/`Vec`，以免留下"测试能用生产不能用"
的灰区。

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
# 3) 测试（本机 256 步）
YEBAN_PROPTEST_CASES=256 bash scripts/dev/cargo-local.sh test -p yeban-model
#    -> test result: ok. 76 passed; 0 failed
# 4) CI 档规模（10,000 步 × 32 cases）
CI=true bash scripts/dev/cargo-local.sh test -p yeban-model                 # 约 16s, 全绿
# 5) 强制门禁（fmt + 11 条红线守卫 + clippy -D warnings + test）
bash scripts/gates/run-gates.sh crate yeban-model                          # exit 0, "门禁通过"
# 6) 跨实现 schema 对账（Python jsonschema 4.24 / Draft 2020-12）
python3 scripts/gates/validate_schemas.py --samples-dir target/schema-samples
#    -> 仅 writer_version 一项类型分歧（见 D1）
```

### CI 判决（GitHub Actions，`line/model-core`）

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
2. **`project.schema.json` 未 100% 通过**：`writer_version` 一项类型分歧（D1）。
   `validate_schemas.py --samples-dir` 因此是红的；CI 的 `checks` job 目前**不带**
   `--samples-dir`，所以这条不会让 CI 变红 —— 但这是"CI 没查"，不是"满足契约"。
3. **`ops.schema.json` 未通过**：`origin` 的 `McpProposal` 载荷（D5）与 9 个缺失变体（D6）。
   本线没有产出 ops 样本，因此没有把这条变成可执行判据。
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

1. **需要人类裁决（规范冲突）**: D1（`writer_version` 类型）、D5（`origin` 的
   `McpProposal` 载荷 vs 字符串 enum）、D6（ops 契约缺 9 变体）、D2（是否把
   `RemoveSection`/`RemoveScene` 写回架构 §6.1）。本线不得改 Normative 文档与
   `schemas/**`，因此全部留痕在此。
2. **需要集成者决定**: 是否把 `validate_schemas.py --samples-dir target/schema-samples`
   接进 CI 的 `checks` job。当前它只在 `run-gates.sh full` 里被调用（且不带样本目录），
   所以样本对账**没有**被 CI 执行。若接线，必须先裁决 D1。
3. **需要集成者上提**: **无**（TODO(hoist) 为空；本线零新增依赖）。
4. **样本产物的位置**: 样本写到 `<repo>/target/schema-samples/project.{default,filled}.json`，
   而 `target/` 被 gitignore，因此样本**不进版本库**、由测试每次重新生成（逐字节稳定，
   有 `export_schema_samples_to_target` 断言）。
5. **本机跑 `run-gates.sh` 需要环境变量**: 裸 `cargo` 在受限沙箱里会因为 rustup 无法写
   `~/.rustup` 而在 `fmt` 步就失败。本线的做法是调用前导出
   `CARGO_HOME=/Users/crow/work/music/.cargo-home RUSTUP_TOOLCHAIN=stable`
   （与 `scripts/dev/cargo-local.sh` 完全相同的两个变量），**未修改脚本本身**。
   若人类希望 `run-gates.sh` 自己就能在沙箱里跑，需要集成者给它加同样的兜底。
6. **未实现的能力切片**（留给后续）：`ROAD-M1-004` 存储引擎、`ROAD-M1-005` 迁移器、
   `Op` 的 JSON Schema 补齐、以及把 `probabilistic` 触发与渲染引擎对账的集成测试。
