# origin-contract —— 把 `OpOrigin::McpEdit` 补进契约（追平 `schemas/**` 与代码的漂移）

> 工作线 `line/origin-contract`（worktree `.worktrees/origin-contract`，基线 `ff71c19`）。
> 上游漂移来源：`line/op-origin-mcp`（worktree `.worktrees/op-origin-mcp`，已并入 `ff71c19`）
> 在 `OpOrigin` 里新增 `McpEdit { agent_name: String }`，但该线禁改 `schemas/**`，
> 于是把漂移登记成 `crates/yeban-model/src/ops.rs` 的 `PENDING_CONTRACT_ORIGINS = ["McpEdit"]`。
> 本线拥有 `schemas/ops.schema.json` + `crates/yeban-model/**`，负责追平并清空该清单。
>
> **本文件（第 1 轮）按集成者的降档指令单独提交**：本轮只交这一份 notes，
> 代码改动（§4）已完成、本机已全绿，留到下一轮提交。据此本文件里的「已完成」都附了当场跑过的命令。

---

## 0. 结论（一段话）

1. **三条判据都没有写错**，不需要放宽任何 `additionalProperties` / `required`，也不需要改判据语义。
   集成者上次的「加分支 + 清空清单」之所以红，**唯一**原因是**分支插入的位置**：
   两条判据按**下标**读 `origin.oneOf`（`oneOf[0]` = 单元枚举、`oneOf[1]` = `McpProposal`）。
   把 `McpEdit` 对象分支插到**下标 0** ⇒ `origin.oneOf[0].enum` 根本不存在 ⇒ 3 条判据
   在同一个 `.expect("origin.oneOf[0].enum 必须是数组")` 上 panic（实测复现，§2）。
   把新分支**追加到末尾（下标 2）**即全绿（实测 110 passed / 0 failed，§5.1）。
2. **没有样本需要重冻结**：4 份规范样本的 sha256 在改动前后**逐字节相同**（实测，§5.3）；
   `export_all` 的期望值是「4 份文件名清单 + 两次导出逐字节相同」，不含内容哈希，
   也不含来源变体清单 —— 这条假设实测不成立（§1.4）。
3. **契约的「登记」位置是新的对象分支，不是 `origin.oneOf[0].enum`**：那个 enum 是
   **6 个单元变体**的白名单，McpEdit 是对象变体；把它塞进 enum 会让
   `serialized_units == unit_names`（ops.rs:2743）与 `unit_names.len() == 6`
   （samples.rs:773）当场红。
4. §4 给出**完整改动清单**（3 处 + 1 条新判据），每处都写了「为什么」。

---

## 1. 三条判据各自的真实要求（读源码 + 本机实测）

行号是**基线 `ff71c19`** 的行号（本线改动后 ops.rs 会整体下移，notes 里另注）。

| # | 判据 | 位置（`ff71c19`） | 它读契约的哪一段 | 断言什么 |
| :-- | :--- | :--- | :--- | :--- |
| ① | `op_variants_match_ops_schema_exactly` | `ops.rs:2457` | **只读 `op.oneOf`** | `op.oneOf[*].required[0]` 的集合**恰好 29 个**；`showcase_ops()` 的名字集合 == 该集合 ∪ `PENDING_CONTRACT_OPS` |
| ② | `origin_variants_match_ops_schema_origin_one_of` | `ops.rs:2714` | `origin.oneOf[0].enum`、`origin.oneOf[*].required`、`origin.oneOf[1].properties.McpProposal.required` | 单元变体集合相等；对象标签差集**恰好等于** `PENDING_CONTRACT_ORIGINS`（双向）；McpProposal 载荷键 == 契约 required |
| ③ | `ops_origin_shapes_match_the_origin_one_of` | `samples.rs:771` | `origin.oneOf[0].enum`（经 helper 的 `.expect`，`samples.rs:676`） | enum 恰好 **6** 项；默认样本来源是 enum 里的字符串；filled 样本来源是只带 `McpProposal` 一个键的对象 |
| ④ | `every_unit_origin_variant_is_in_the_contract_enum` | `samples.rs:795` | 同上 helper | 6 个**单元**变体逐个序列化成字符串且落在 enum 里，并能往返 |

### 1.1 ① `op_variants_match_ops_schema_exactly`（`ops.rs:2457`）

- 事实源是 `schema_op_variant_names()`（`ops.rs:2405`）：遍历 `op.oneOf`，取
  `branch["required"][0]` —— 即**每个 op 分支的标签键**。
- `ops.rs:2460` 断言 `contract.is_disjoint(&pending)`；`ops.rs:2465` 断言
  `contract.len() == 29`（**硬编码计数**）；`ops.rs:2487` 断言
  `implemented == contract ∪ pending`。
- **它一个字都不读 `origin`**。所以 `origin.oneOf` 怎么改都不会让它红。
  它唯一会因「分支位置」而红的形态是：分支被**误插进 `op.oneOf`** ⇒
  `contract.len()` 变 30 ⇒ 在 `ops.rs:2465` 红。
  ⇒ 集成者报告里的红名单含 `op_variants_match_ops_schema_exactly` 是**转述滑了一格**：
  本机复现出的 3 条红是 §2 的那三条（106 passed / **3** failed，与集成者给的计数一致，
  且 3 个 panic 位置逐个对上）。

### 1.2 ② `origin_variants_match_ops_schema_origin_one_of`（`ops.rs:2714`）

按执行顺序，它做了 6 件事：

| 位置 | 代码 | 断言 |
| :--- | :--- | :--- |
| `ops.rs:2716` | `schema["properties"]["origin"]["oneOf"][0]["enum"]` | **按下标 0** 取单元枚举，取不到 ⇒ `.expect("origin.oneOf[0].enum 必须是数组")` **panic** |
| `ops.rs:2728` | 遍历 `all_origin_variants()`（8 个） | 每个变体要么序列化成字符串、要么是**单键**对象；都能往返 |
| `ops.rs:2743` | `serialized_units == unit_names` | 单元变体**逐名相等**（多一个、少一个都红） |
| `ops.rs:2749` | 遍历 `oneOf[*]`，扁平化每个分支的 `required` | 契约侧对象标签集合 |
| `ops.rs:2762` | `contract_objects.is_disjoint(&pending)` | 清单里的标签**已经**出现在契约里 ⇒ 红「请把 PENDING_CONTRACT_ORIGINS 清空」 |
| `ops.rs:2767` | `serialized_objects − contract_objects == pending` | **恰好等于**语义：契约少一个分支 ⇒ 左值非空、右值为空 ⇒ 红 |
| `ops.rs:2780` | `contract_objects − serialized_objects` 必须为空 | 契约多一个枚举里没有的对象分支 ⇒ 红并点名 |
| `ops.rs:2804` | `oneOf[1]["properties"]["McpProposal"]["required"]` | **按下标 1** 取 McpProposal 的载荷 `required`，与 serde 实测载荷键集合相等 |

⇒ 这条判据的「位置依赖」有两处：`oneOf[0]` 必须是单元枚举，`oneOf[1]` 必须是 `McpProposal`。
它**没有**检查任何其它对象分支的**载荷**（`additionalProperties` / 字段类型 / required 都不看），
这正是本线新增第 5 条判据的理由（§4.2.3）。

### 1.3 ③④ 两条 samples 判据（`samples.rs:771` / `samples.rs:795`）

- 两条都先调 `schema_origin_unit_names()`（`samples.rs:671`），该 helper 读
  `schema["properties"]["origin"]["oneOf"][0]["enum"]`，`.expect(...)` 在 `samples.rs:676`。
- ③ `ops_origin_shapes_match_the_origin_one_of`：`unit_names.len() == 6`（`samples.rs:773`）；
  默认样本（`UserUi`）的序列化必须是 enum 里的字符串；filled 样本（`McpProposal`）必须是
  只带 `McpProposal` 一个键的对象，且载荷含 `proposal_id` / `agent_name`。
- ④ `every_unit_origin_variant_is_in_the_contract_enum`：把 6 个**单元**变体逐个序列化，
  断言每个名字都在 enum 里并能往返。
- 两条都**不检查** `McpEdit`（也不检查任何对象分支的载荷形状）。
- 它们红的**唯一**形态就是 helper 的 `.expect` panic（下标 0 不再是 `{"type":"string","enum":[...]}`），
  或 enum 被改动（6 项变多/变少、名字被改）。

### 1.4 我另外查过、并且**实测不成立**的三种「冻结」假设

| 假设 | 实测结论 |
| :--- | :--- |
| 「有冻结的样本 sha256」 | **不存在**。`crates/yeban-model` 里只有 `tests/schema_ratchet.rs:124` 的**注释**写了 `schemas/project.schema.json` 的 sha256；`ops.schema.json` 与 4 份样本都没有任何哈希冻结。`tests/no_compat.rs:641` 只断言「两次导出逐字节相同」 |
| 「`export_schema_samples` 有期望值要跟着改」 | **没有**。`tests/schema_ratchet.rs` 只覆盖 `project.schema.json`；`samples.rs:817` / `project.rs:2655` 断言的期望值是**文件名清单（恒 4 份）**与两次导出一致，不含内容清单 |
| 「契约 enum 需要登记 McpEdit」 | **不该登记**。`origin.oneOf[0].enum` 是 6 个单元变体的白名单；McpEdit 的正确登记位置是新的**对象分支**（§3.1）。塞进 enum 会让 §1.3 两条判据（`len()==6`、`serialized_units==unit_names`）当场红 |

---

## 2. 上次为什么红：本机复现实测（含左值/右值）

**复现手法**：在基线 `ff71c19` 上，把 `McpEdit` 对象分支插到 `origin.oneOf` 的
**下标 0**（即插在 `{"type":"string","enum":[...]}` 之前），并把
`PENDING_CONTRACT_ORIGINS` 清成 `[]` —— 这正是集成者描述的「加分支 + 清空清单」。

```
bash scripts/dev/cargo-local.sh test -p yeban-model      # exit=101
```

实测输出（原文，`/tmp/expA.log`）：

```
failures:

---- ops::tests::origin_variants_match_ops_schema_origin_one_of stdout ----

thread 'ops::tests::origin_variants_match_ops_schema_origin_one_of' panicked at crates/yeban-model/src/ops.rs:2719:18:
origin.oneOf[0].enum 必须是数组

---- samples::tests::every_unit_origin_variant_is_in_the_contract_enum stdout ----

thread 'samples::tests::every_unit_origin_variant_is_in_the_contract_enum' panicked at crates/yeban-model/src/samples.rs:676:14:
origin.oneOf[0].enum 必须是数组

---- samples::tests::ops_origin_shapes_match_the_origin_one_of stdout ----

thread 'samples::tests::ops_origin_shapes_match_the_origin_one_of' panicked at crates/yeban-model/src/samples.rs:676:14:
origin.oneOf[0].enum 必须是数组

failures:
    ops::tests::origin_variants_match_ops_schema_origin_one_of
    samples::tests::every_unit_origin_variant_is_in_the_contract_enum
    samples::tests::ops_origin_shapes_match_the_origin_one_of

test result: FAILED. 106 passed; 3 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
```

**左值/右值说明（诚实版）**：这里**没有** `assert_eq` 的 left/right —— 三条红都是
**对不存在的 JSON 路径取值的 panic**（`serde_json::Value` 的 `Index` 对缺失键返回
`Null`，`.as_array()` 得到 `None`，`.expect(...)` 抛出）。三条红的**根因是同一个**：
`origin.oneOf[0]` 变成了对象分支，于是 `.enum` 不存在。
（若把清空清单那一步去掉，`ops.rs:2762` 的 `is_disjoint` 也会红；但 panic 发生在
`ops.rs:2719`，早于它，所以看不到那条消息。）

**为什么「插入位置」是唯一变量**：把**同一段分支 JSON** 从下标 0 挪到**末尾**，
其它一切不动，三条判据立刻全绿（§5.1）。这也解释了为什么
`oneOf[0]` / `oneOf[1]` 的硬下标是这次事故的全部机制。

---

## 3. 修复方案（为什么是「追加到末尾」）

契约的 `origin.oneOf` 必须保持这个**顺序不变式**：

| 下标 | 内容 | 谁按下标读它 |
| :--- | :--- | :--- |
| `[0]` | `{"type":"string","enum":[6 个单元变体]}` | `ops.rs:2716`、`samples.rs:674` |
| `[1]` | `McpProposal` 对象分支 | `ops.rs:2804`（载荷 required） |
| `[2]` | **新增** `McpEdit` 对象分支 | 无（本线新判据按**标签名**定位） |

⇒ 新分支只能**追加到末尾**。这一点已写进契约自己的 `origin.description`
（「Branch ORDER is part of the contract: oneOf[0] … oneOf[1] … oneOf[2] …」），
让下一个改契约的人不必再踩一次。

---

## 4. 完整改动清单（每处 + 为什么）

改动面：**2 个文件 + 1 条新判据 + 本 notes**。零新增依赖，`Cargo.toml` / `Cargo.lock` 未动。

### 4.1 `schemas/ops.schema.json`（+23 / −1）

| # | 改动 | 为什么 |
| :--- | :--- | :--- |
| a | `origin.oneOf` **末尾**追加分支：`{"type":"object","required":["McpEdit"],"additionalProperties":false,"properties":{"McpEdit":{"type":"object","required":["agent_name"],"additionalProperties":false,"properties":{"agent_name":{"type":"string","description":"Direct-edit agent name (no proposal identity: McpEdit never creates a proposal)"}}}}}` | 判据①（任务书）：承认 `McpEdit`，两处 `additionalProperties:false`，载荷只许 `agent_name`。与 `McpProposal` 分支对称（后者也是「分支 required 标签键 + 载荷 required 字段」两层）。**追加到末尾**的理由见 §3 |
| b | `origin.description` 补上 `McpEdit carries {agent_name}` 与**顺序不变式** | 原先的描述只提 `McpProposal`，现在事实变了；顺序不变式是 §2 那次事故的机械成因，写在契约里比写在某个测试的注释里更能拦住下一个人 |

### 4.2 `crates/yeban-model/src/ops.rs`

| # | 改动 | 为什么 |
| :--- | :--- | :--- |
| 1 | `PENDING_CONTRACT_ORIGINS: [&str; 1] = ["McpEdit"]` → `[&str; 0] = []` | 契约已承认，差集变空；不清空则判据②的 `is_disjoint`（`ops.rs:2762`）红。doc 注释改成「已清空」并保留空数组（与 `PENDING_CONTRACT_OPS` 同族：下一次谁加了对象标签又来不及改契约，仍有地方可登记） |
| 2 | 模块头注释（`ops.rs:29-38`）与 `OpOrigin::McpEdit` 的变体注释（`ops.rs:75-78`）：「⚠ 契约欠账」→「契约已追平」，并写明**为什么分支必须在末尾**（附「插到下标 0 ⇒ 106 passed / 3 failed」的实测结论） | 陈旧注释 = 下一份漂移的温床。这两处原本明确写着「本变体尚未被契约承认」，现在这句话已经是假的 |
| 3 | **新增判据** `mcp_edit_origin_shape_matches_its_contract_branch` | 见 §4.2.3 |

#### 4.2.3 为什么新增一条判据（这是「不加也不行」的一处）

判据②只把对象分支当**标签**核对（`oneOf[*].required[0]` 的集合差），载荷形状只在
`McpProposal` 上硬编码查了一次（`oneOf[1]`，且只看键集合）。于是契约若被写成

```json
{ "McpEdit": { "type": "object" } }
```

（没有 `payload.required`、没有 `additionalProperties:false`）判据②**依然全绿** ——
而任务书判据①的牙齿恰恰是「只许 `agent_name`」。新增判据做的事：

1. 先冻住 serde 实测字节：`serde_json::to_string(McpEdit{agent_name:"yeban-mcp"})`
   == `{"McpEdit":{"agent_name":"yeban-mcp"}}`（与既有的
   `origin_wire_shape_is_frozen_byte_for_byte` 一致，两处互为佐证）；
2. **按标签名**（`required[0] == "McpEdit"`）在 `origin.oneOf` 里找分支 ——
   **刻意不写下标**：这次事故正是「下标被改动悄悄打偏」，而这条判据要能对
   「新分支该在哪个位置」保持中立；它的三个断言（存在、`additionalProperties:false` ×2、
   载荷形状）在任何位置都成立；
3. 断言分支层 `required == ["McpEdit"]`、`additionalProperties == false`；
4. 断言载荷层 `additionalProperties == false`、`required == ["agent_name"]`、
   字段形状（名字 → `type`）恰好是 `{agent_name: string}`；
5. **逐字段对照**：serde 实测载荷键集合 == 契约载荷 `required` == 契约载荷声明的
   `properties`（三集合相等 ⇒ 既没有「实现多写字段」，也没有「契约声明了没人写的字段」），
   并断言值确实是字符串、能往返。

它**不是**放宽任何既有判据，而是在既有三条之上**加严**；且不修改既有三条的任何一行语义。

### 4.3 我**没有**改的东西（以及为什么）

| 对象 | 为什么不动 |
| :--- | :--- |
| `origin.oneOf[0].enum` | 它的语义是「6 个**单元**变体」；McpEdit 是对象变体（§1.4） |
| 三条既有判据的任何一行 | 它们的语义正确，红是因为契约形状变了，不是因为判据写错（任务书纪律：不许改判据换绿） |
| 4 份规范样本 / `samples.rs` 的样本构造 | 样本里只出现 `UserUi` 与 `McpProposal`；本改动不改变它们的字节（实测 sha256 不变，§5.3）。所以**「因契约变更而合法重冻结」的样本数为 0** |
| `schemas/**` 的其它 3 个文件 | 本线不拥有；且本改动不涉及 |
| `crates/**`（除 `yeban-model`） | 不属于本线。**但 MCP 侧有 3 处陈述已过期 ⇒ 见 §7 的 needs** |

---

## 5. 证据：本机实测

### 5.1 三条判据全绿 + 清单清空后的全量

```
bash scripts/dev/cargo-local.sh test -p yeban-model      # exit=0
```

| 测试目标 | 结果 |
| :--- | :--- |
| `unittests src/lib.rs` | **110 passed**; 0 failed（基线 109 = 106 passed + 3 failed，本线新增 1 条 = 110） |
| `tests/automation.rs` | 28 passed; 0 failed |
| `tests/container_adversarial.rs` | 50 passed; 0 failed |
| `tests/container_roundtrip.rs` | 16 passed; 0 failed |
| `tests/model_isolation.rs` | 16 passed; 0 failed; 1 ignored |
| `tests/no_compat.rs` | 8 passed; 0 failed; 1 ignored |
| `tests/schema_ratchet.rs` | 9 passed; 0 failed; 1 ignored |
| Doc-tests（compile pass + compile fail） | 3 + 2 passed; 0 failed |

其中与本次改动直接相关的 4 条（新判据 + 集成者点名的三条）：

```
test ops::tests::origin_variants_match_ops_schema_origin_one_of ... ok
test ops::tests::mcp_edit_origin_shape_matches_its_contract_branch ... ok
test samples::tests::every_unit_origin_variant_is_in_the_contract_enum ... ok
test samples::tests::ops_origin_shapes_match_the_origin_one_of ... ok
```

### 5.2 `McpEdit` 的 serde 实测输出 ⇔ 契约形状逐字段对照（任务书判据③）

| 字段 | serde 实测（`serde_json::to_string`） | 契约分支 | 一致 |
| :--- | :--- | :--- | :--- |
| 外部标签键 | `McpEdit` | 分支 `required: ["McpEdit"]` + `properties` 里同名键 | ✅ |
| 分支层未知键 | 不会写出 | `additionalProperties: false` | ✅ |
| 载荷键集合 | `{agent_name}` | 载荷 `required: ["agent_name"]`、`properties: {agent_name}` | ✅ |
| 载荷未知键 | 不会写出 | 载荷 `additionalProperties: false` | ✅ |
| 值类型 | `"yeban-mcp"`（string） | `{"agent_name":{"type":"string"}}` | ✅ |
| 完整字节 | `{"McpEdit":{"agent_name":"yeban-mcp"}}` | 上述分支允许且仅允许的形状 | ✅ |

实测命令：`bash scripts/dev/cargo-local.sh test -p yeban-model mcp_edit_origin_shape_matches_its_contract_branch`
（该判据把上表逐行变成断言；`origin_wire_shape_is_frozen_byte_for_byte` 另有一份逐字节冻结表）。

**跨实现（Python jsonschema）一侧**：CI 的 `validate_schemas.py --samples-dir` 只读
`target/schema-samples/*.json`（**非递归** `glob("*.json")`，`validate_schemas.py:323`），
而 `export_all` 的 4 份样本里没有 McpEdit 实例 ⇒ **Python 侧目前看不到这个新分支**。
本线的处置是**不**动那 4 份样本（它们是被 3 条判据钉住「恒 4 份」的冻结产物），
跨实现证据由 §5.2 的表格（serde 实测 ⇔ 契约形状）承担。若集成者希望 CI 用 jsonschema
真正吃一遍 `McpEdit` 字节，那需要**另开一条**「第 5 份样本」的改动，会碰到
`samples.rs:817` / `project.rs:2655` / `no_compat.rs:641` 三处「恒 4 份」的期望值 ——
**本线不擅自改**，登记为 §7 needs-3。

### 5.3 既有来源标签的线上字节 / 样本逐字节不变（任务书判据④）

```
sha256sum target/schema-samples/*.json      # 改动前 / 改动后 各一次
diff before.txt after.txt                   # 空 ⇒ 逐字节相同
```

| 样本 | sha256 | 改动后 |
| :--- | :--- | :--- |
| `ops.default.json` | `d331a363621468c9135216ba00332e8ddc40f380b0a00360840ed07e55d14069` | 不变 |
| `ops.filled.json` | `9dcf3e5c98070062c6ede3613df4b40cd00ec88579fa794041d2b2cb00b82a4d` | 不变 |
| `project.default.json` | `4cf38f55e27d9ced3a011b4065d2113f211809003b4b312e677fc15e0e7bf725` | 不变 |
| `project.filled.json` | `960f03a2e0eda1073d9b9bb05fcc787ff24b0e76ce3a487d087e31f61198b486` | 不变 |

- `origin_wire_shape_is_frozen_byte_for_byte`（`ops.rs:2664`）把**8 个**来源变体的线上字节
  逐个冻住（7 个既有 + 新增的 McpEdit），本改动一字未动它。
- ⇒ **既有 7 个来源标签的线上字节不变；因契约变更而「合法重冻结」的样本 = 0 份**。
  契约自身的新增（那 23 行 JSON）**不是**样本，不进这个统计。

### 5.4 门禁（本机实跑）

```
bash scripts/gates/run-gates.sh light        # exit=0，门禁通过 (mode=light)
```

`light` 档实际跑过：`cargo fmt --all --check`、`policy_check.py`、7 条文档契约守卫、
`validate_schemas.py`、`license_inventory.py --check`。全部 `ok`，末尾打印
「门禁通过 (mode=light)」。

**这次门禁真抓到了一个东西（如实记录）**：新增判据第一次提交前，
`run-gates.sh light` 的 `fmt` 步报了真实 diff ——
`spec["type"].as_str().expect(...).to_owned()` 这一行超宽，rustfmt 要求折成
`.as_str()` / `.expect(...)` / `.to_owned()` 三行。已按 rustfmt 的写法改掉，复跑
`run-gates.sh light` = `exit=0`、`cargo-local.sh test -p yeban-model` = `exit=0`
（110 + 其它腿全绿，见 §5.1）。

**环境摩擦（供其它线参考）**：本机受限沙箱里**直接**调 `cargo fmt` 会死在
`rustup` 的临时目录（`could not create temp file /Users/crow/…/.rustup/tmp/…: Operation not permitted`）。
必须走本仓的包装器（`scripts/gates/run-gates.sh` / `scripts/dev/cargo-local.sh`），
它们会 source `scripts/dev/local-env.sh` 把 `CARGO_HOME` / `RUSTUP_TOOLCHAIN` 指到可写位置
（这就是 `cargo-local.sh` 头部 L30 那条纪律的由来）。

---

## 6. 注入：机制「有牙」的实测（任务书判据②）

三条注入都在**当场的代码状态下**真跑过，之后逐个还原（`sha256sum -c` 校验还原正确）。

### 注入 1 —— 契约**少**一个分支（删掉 `McpEdit`，清单已空）

```
bash scripts/dev/cargo-local.sh test -p yeban-model origin_variants_match_ops_schema_origin_one_of   # exit=101
```

```
panicked at crates/yeban-model/src/ops.rs:2777:9:
assertion `left == right` failed: 枚举里多出来的对象标签必须**恰好**是 PENDING_CONTRACT_ORIGINS;
若契约已补齐, 请把该清单清空;
只在枚举里而契约缺失: ["McpEdit"]
  left: {"McpEdit"}
 right: {}
```

⇒ **红，并给出 left/right**：机制真的在「契约落后于枚举」时报红（这就是
`PENDING_CONTRACT_ORIGINS` 存在的意义）。

### 注入 2 —— 契约**多**一个分支（追加一个枚举里没有的 `GhostEdit`）

```
panicked at crates/yeban-model/src/ops.rs:2790:9:
只在契约里而枚举缺失的对象标签: ["GhostEdit"]
```

⇒ **红并点名**：反方向的漂移（契约单方面发明一个标签）同样有牙。

### 注入 3 —— 契约已补齐但**清单没清空**（把 `McpEdit` 写回清单）

```
panicked at crates/yeban-model/src/ops.rs:2773:9:
契约已经补上了 {"McpEdit"} 里的对象分支 —— 请把 PENDING_CONTRACT_ORIGINS 清空
```

⇒ **红**：欠账清单不会腐烂成静默漂移（「契约补上了但没人清清单」也是错）。

---

## 7. needs（交给集成者 / 其它线的写者；本线不能改）

| # | 对象 | 事实 | 建议处置 |
| :-- | :--- | :--- | :--- |
| 1 | `crates/yeban-mcp/src/domain/automation.rs:288`（**生产响应载荷**） | `"note": "MCP 直接编辑 (不创建提案) ⇒ OpOrigin::McpEdit; 契约 origin.oneOf 尚未承认该分支 (needs)"` —— 追平后这半句已是**假话**，会随每个 `yeban_edit_automation` 响应发给 MCP 客户端 | MCP 线或集成者把 note 改成「契约 `origin.oneOf[2]` 已承认该分支」。无测试断言该字符串（`extension_tools.rs:470` 只断言 `kind`），所以它**不会**让 CI 红 —— 正因如此更容易烂掉 |
| 2 | `crates/yeban-mcp/src/domain/automation.rs:46`、`import_audio.rs:36`（模块文档） | 同样写着「契约 `origin.oneOf` 尚未承认该分支」 | 随 #1 一并改文档 |
| 3 | CI 的跨实现对账覆盖（可选） | `validate_schemas.py --samples-dir` 目前**看不到** `McpEdit` 字节（4 份样本里没有它；见 §5.2） | 若要 Python jsonschema 真吃一遍，需新增第 5 份样本，并同步 `samples.rs:817` / `project.rs:2655` / `no_compat.rs:641` 三处「恒 4 份」的期望值。**本线不擅自改**，请集成者裁决 |
| 4 | `docs/ledger/op-origin-mcp-notes.md` §7 的 needs-1 | 该 need（「请契约线给 `McpEdit` 一个对象分支」）**已满足** | 由该 notes 的维护者（op-origin-mcp 线 / 集成者）在自己的文件里记为已闭环；本线不改别人的 ledger |
| 5 | 判据②的**位置脆弱性**（观察，不是错误） | 它按 `oneOf[0]` / `oneOf[1]` 硬下标读契约（§1.2）。它**不会静默**：下标挪动 ⇒ `.expect` 当场 panic（§2 实测）。所以我不认为它「写错了」，也**没有**改它 | 若要更稳，可另开一线把它改成**按标签名**定位（本线新增的判据已经这么做）；这属于改既有判据的语义，需集成者裁决，本线不擅自做 |

**没有**任何一条判据被我判定为「写错了」⇒ 不触发任务书的「停下写 needs」流程。

---

## 8. 本机 vs CI（诚实边界）

| 面 | 本机（M2，`cargo-local.sh`） | CI |
| :--- | :--- | :--- |
| `yeban-model` 全量 test | ✅ 已跑（§5.1，exit=0） | 由 `rust (yeban-model)` 腿复跑 |
| `run-gates.sh light` | ✅ 已跑（fmt + 守卫 + 文档契约 + schema + 许可清单） | `checks` job 复跑 |
| workspace clippy `-D warnings` | ❌ 本机禁跑（重依赖） | 只在 CI |
| `cargo deny` | ❌ 本机禁跑 | 只在 CI |
| Python jsonschema 对账 `McpEdit` 字节 | ❌ 4 份样本里没有该实例（§5.2 / needs-3） | 同上 —— CI 也不会吃到 |
| `yeban-mcp` 腿 | ❌ 本机不编译该 crate | CI 复跑（它不读 `ops.schema.json`，本改动对它只有 §7 的文案影响） |

判决必须用 `scripts/dev/ci-verdict.sh line/origin-contract` **读回来**才算数；
本文件不写「CI 通过」。

---

## 9. 下一轮（代码改动按降档指令 stash，只等提交）

| 文件 | 状态 |
| :--- | :--- |
| `docs/ledger/origin-contract-notes.md` | **本轮提交**（本文件） |
| `schemas/ops.schema.json` | 已改并本机验证通过（§4.1），按降档指令 `git stash` 暂存（另存 `/tmp` 备份），下一轮 `git stash pop` 后提交 |
| `crates/yeban-model/src/ops.rs` | 同上（§4.2） |

净行数：本轮 notes-only 提交见提交后的 `git show --stat`。
下一轮代码提交的净行数：`schemas/ops.schema.json` **+23 / −1**；
`crates/yeban-model/src/ops.rs` **+138 / −13**（含新判据与注释追平）——
两处都在本机 `run-gates.sh light`（exit=0）与 `cargo-local.sh test -p yeban-model`（exit=0）下验证过。
