# model-no-compat 工作线笔记 —— ADR-0001 **D43** 反序列化严格性

- **工作线**：`line/model-no-compat`（工作树 `.worktrees/model-no-compat`，基线 `main` = `632b0c0`）
- **决策来源**：[`../adr/ADR-0001-workspace-topology-and-version-pinning.md`](../adr/ADR-0001-workspace-topology-and-version-pinning.md) **D43**（负责人 2026-10-04）
- **交付范围**：`crates/yeban-model/**` + 本笔记。`schemas/**` 由集成者独占，本线**只给 contract 清单**（§5）。
- **不解除的红线**：确定性（`BTreeMap` / 逐字节可复现）、`.yeban` 容器安全闸门（Zip-Slip / 炸弹 / 上限）、许可、`#![forbid(unsafe_code)]`、"只有 CI 判决算绿"。

---

## 0. 诚实的来路说明（先读这一条）

> **本线由前任执行者开始、由接手者完成；前任没有提交、没有写 notes、没有留下任何报告。**

前任在工作树里留下了 3 个未提交的源文件改动（`git diff --numstat` = `+161 −61`：`project.rs` /`commit.rs` /`music.rs`），并在原位写了 `[ADR-0001 D43]` 注释解释"这是语义 / 这是必需"。**改动方向是对的**，但前任漏掉了三件比改动本身更要紧的事：

1. **它没有把与 D43 相冲突的既有判据改掉**，因此它离开时工作树是**红的**（§1）；
2. **它没有任何证据**：没有 `tests/no_compat.rs` 这类逐字段判据，也没有把真实错误文本留档；
3. **它没有给集成者 contract 清单**，而 `schemas/project.schema.json` 当时（现在仍然）比实现宽松得多。

接手者的工作：**复审分类（§2）→ 补齐证据（§4）→ 修掉与 D43 冲突的旧判据（§1.2）→ 给出 contract 清单（§5）→ 本机真跑 + 注入验证（§6）→ 提交与 CI 判决（§7）**。
本笔记的每一句"绿"都来自接手者的真跑记录；凡未真跑的一律标 `pending` / `needs`。

---

## 1. 接手时的实测状态：**不是全绿**

### 1.1 前任的结论不成立（实测）

前任对外的说法是"`cargo-local.sh test -p yeban-model` 全绿"。接手者实测**4 条红**（`--no-fail-fast`，逐条列出）：
```
running 107 tests
test project::tests::device_latency_samples_defaults_to_zero_and_round_trips ... FAILED
thread '...' panicked at crates/yeban-model/src/project.rs:2434:42:
缺少 latency_samples 的旧设备必须仍可读: Error("missing field `bypassed`", line: 5, column: 9)
test result: FAILED. 106 passed; 1 failed

running 28 tests   (tests/automation.rs)
test value_domain_and_write_mode_json_shape_is_stable ... FAILED
  panicked at crates/yeban-model/tests/automation.rs:653:5      // assert!(!object.contains_key("read_enabled"), "默认 true 不落盘")
test legacy_lane_json_reads_with_defaults_and_reserializes_byte_identically ... FAILED
  panicked at crates/yeban-model/tests/automation.rs:689:73     // 旧 JSON 必须可读
test legacy_project_document_without_the_new_fields_is_still_readable ... FAILED
  panicked at crates/yeban-model/tests/automation.rs:729:65     // 旧工程必须可读
test result: FAILED. 25 passed; 3 failed
```

这 4 条**全部**是"旧工程兼容"这一旧政策的判据 —— 也就是说，前任改了**实现**，却没有改**判据**。这不是"测试过时"，而是"实现与它自己的契约相反"：D43 要求这些字段必需，而判据要求它们可缺省。两者只能活一个。

**"107 + 28 + 50 + 16 全绿"这句话是从哪里来的**（这一条值得单独记，因为它解释了为什么会误判）：它**逐字**出现在另一条线的历史台账 [`model-automation-notes.md`](model-automation-notes.md) 第 296–297 行：

```
cargo test -p yeban-model 全绿：**107**（lib）+ **28**（automation）+ **50**（container_adversarial）+ **16**（container_roundtrip）。
```

那是 `line/model-automation` 在 **default 还都在**的年代写下的记录（同一个文件的第 374 行还把"删掉三个新字段的 `#[serde(default)]`"登记成一次**需要还原的注入**）。也就是说：这个数字是**历史台账的引用**，不是对前任改动之后那棵树的**实测**。本笔记的规则因此是：凡"绿"必须给出**本次真跑**的命令与计数（§6.1），台账引用一律不算证据。

**集成者后来的根因确认（教训 L30，`main` `42442af`）**：那个"全绿"前提是**假绿**，两个原因叠加 ——
① 旧的 `scripts/dev/cargo-local.sh` 用 `$(dirname "${BASH_SOURCE[0]}")/../..` 定位仓库；当它被**主仓绝对路径**调用时会 `cd` 回**主仓**，于是编译并测试的是**主仓**的代码，而不是 worktree；
② 当时只 grep `test result:` 行且**没有** `--no-fail-fast`，因此"只报了 N 条"被当成了"只有 N 条"。
两者都已修（`cargo-local.sh` 改为按**当前目录**的 `git rev-parse --show-toplevel` 定位并**打印实际工作区路径**）。
本笔记的实测**不受①影响**：接手者是用**相对路径**（在 worktree 里执行 `bash scripts/dev/cargo-local.sh …`）调用的，且第一次跑就看到了 §1.1 那 4 条红的**旧政策断言**——如果它测的是主仓，那 4 条会**通过**（主仓那时还带着 default）。

### 1.2 接手者怎么处理的

| 旧判据 | 处置 | 新判据 |
| :--- | :--- | :--- |
| `project::tests::device_latency_samples_defaults_to_zero_and_round_trips`（`src/project.rs`） | **反转为 D43 判据**（不删） | `device_latency_bypass_and_params_are_required_and_round_trip`：`bypassed` / `params` / `latency_samples` 逐个缺键 ⇒ 错误点名；**显式 `0` 仍合法且能往返** |
| `tests/automation.rs` ④b | **改断言方向** | `read_enabled` / `write_mode` 现在**始终落盘**；仅 `domain=None` 不落盘 |
| `tests/automation.rs` ⑤a（`legacy_lane_json_...`） | **反转为 D43 判据** | `lane_missing_read_enabled_write_mode_or_points_is_rejected`：缺三个必需键逐个报错；写全后往返逐字节不变 |
| `tests/automation.rs` ⑤b（`legacy_project_document_...`） | **反转为 D43 判据** | `project_document_with_a_legacy_shaped_lane_is_rejected`：嵌套的旧形状泳道同样必须被拒 |

**没有删除任何判据**：4 条旧判据全部原地反转成"反向断言 + 更严格的错误文本断言"。删除是掩盖，反转才是证据。
`tests/automation.rs` 的文件头与第 ⑤ 节标题也从"旧工程兼容"改成"ADR-0001 D43 反序列化严格性"。

> ⚠ **跨线影响（needs-4）**：另一条线的账本 [`model-automation-notes.md`](model-automation-notes.md) 把"三个新字段可缺省"当成契约写进了 **9 处**（含判据表 ⑤a/⑤b 的旧函数名、第 296–297 行的"全绿"数字、以及把"删掉 default"登记成需还原的注入）。该账本不在本线权限内 —— 逐处行号见 §7 needs-4。

### 1.3 顺手修掉的 3 处"文档已经说谎"

| 位置 | 旧文（已成谎言） | 新文 |
| :--- | :--- | :--- |
| `src/project.rs` `AutomationWriteMode` 文档 | "`Off` 是默认值：**旧工程读到的**是'不录制'" | "`Off` 仍是 `Default` 派生值（内存构造用），但 D43 之后**不再**是反序列化兜底；它是一个必须被显式写出的**选择**" |
| `src/project.rs` `AutomationWriteMode::is_off` 文档 | "`serde` 的 `skip_serializing_if` 需要 `&self` 形式"（该属性已被删） | "隐式泳道判据之一" |
| `src/samples.rs` 填充样本注释 | "**旧工程可以完全没有它们**（见 `tests/automation.rs` 的旧工程兼容判据）" | "D43 之后 `read_enabled`/`write_mode`/`points` **必需**，只有 `domain` 是 `Option` 语义默认；严格性判据见 `tests/no_compat.rs`" |

---

## 2. 逐项复审表（接手者复审，不盲信前任）

### 2.1 总量台账（机械核对）

测法：对基线 `632b0c0` 与工作树分别 `grep -c '^\s*#\[serde(default'`。

| 文件 | 基线 `632b0c0` | 复审后 | 删掉 | 保留 |
| :--- | ---: | ---: | ---: | ---: |
| `crates/yeban-model/src/project.rs` | 49 | 11 | **38** | 11 |
| `crates/yeban-model/src/music.rs` | 7 | 7 | **0** | 7 |
| `crates/yeban-model/src/commit.rs` | 3 | 0 | **3** | 0 |
| **合计** | **59** | **18** | **41** | **18** |

三分类（互斥且穷尽，41 + 13 + 5 = 59）：

| 分类 | 处数 | 判据 | 例子（各举 2 例） |
| :--- | ---: | :--- | :--- |
| **必需**（默认值会**伪造一个作者从未做过的选择**，故要求显式写出） | **41** | D43 第 1/2 条 | ① `TransportConfig::metronome_enabled`：缺键会**静默关掉节拍器**；② `DeviceDefinition::latency_samples`：`0` 与"这台设备零延迟"这一**合法上报值**不可区分，而 PDC 相位对齐依赖这个数字 |
| **天然可选**（`Option<T>`，`None` 是模型自己定义的一等状态） | **13** | D43 第 2 条明确豁免 | ① `AutomationLane::domain`：`None` = "派生自目标"，信息不丢失（`effective_domain()` 有回落路径）；② `MidiNote::probability`：`None` = "必然触发"，与 `Some(1.0)` 语义等价但**可区分** |
| **语义默认**（空串 / 空集 = "没有注记"的确定编码） | **5** | D43 第 2 条"空集=无标签这类" | ① `ProjectMetadata::tags`：空集 = "没有标签"；② `MidiNote::phonemes`：空集 = "无音素"，且与写入侧 `skip_serializing_if = "Vec::is_empty"` **对偶** |

### 2.2 复审结论

> **接手者逐个复核了全部 59 处，未发现误删或误留。** 前任的 41 处删除**全部**落在"非 `Option`、非注记"的字段上；18 处保留**全部**落在"`Option<T>`"或"自由文本注记 / 空集即无"两类里。判据是按**语义完整性**（"缺了它还能不能无歧义地还原作者意图"）而不是"删得多"给的。

另外机械核对了两条容易出事的性质：

1. **"省略写"与"宽容读"严格成对**。机械核对（实测命令与结果）：

   | 测法 | 结果 | 含义 |
   | :--- | :--- | :--- |
   | `grep -rc 'serde(default, skip_serializing_if' src/` | **15**（`project.rs` 8 + `music.rs` 7 + `commit.rs` 0） | 这 15 个字段**会**在默认值上被省略，因此**必须**保留 `default`；它们全是 `Option::is_none`（13 处）或 `Vec::is_empty`（2 处：`pitch_bend_curve` / `phonemes`） |
   | `grep -rn '    #\[serde(default)\]' src/` | **3**（`ProjectMetadata::description` / `::tags` / `YebanProjectV1::author`） | 这 3 个字段**总是**写出（`""` / `[]`），但读取器仍接受缺失 |
   | 15 + 3 | **18** = §2.1 的保留总数 | 每一个被省略的键都保留着宽容读；反过来，**没有任何"必需字段"带 `skip_serializing_if`** —— 那会写出自己都读不了的文件（注入 3 实测） |

2. **前任删掉的两个辅助函数没有留下死代码**：`default_true()` / `is_true()` 已连同 `#[serde(default = "default_true", skip_serializing_if = "is_true")]` 一起删除（`grep -rn 'default_true\|is_true' src/` = 0 命中）；`AutomationWriteMode::is_off()` 保留，因为 `AutomationLane::validate()` 仍在调用它（实测 `grep -rn 'is_off()' src/` = 1 处调用，定义在 `impl` 内），不是死代码。

### 2.3 三个"我考虑过推翻、但最终维持"的判断（留给集成者复核）

这三处是复审中**唯一**需要动用判断而非机械对错的地方。我把它们写出来，而不是藏在一句"全部站得住"后面。

| 字段 | 前任的处置 | 反面论证 | 接手者的裁定与理由 |
| :--- | :--- | :--- | :--- |
| `CommitGraph::depths` | **必需**（删 default） | 代码自己写着它是"深度缓存""可以重新推导" —— 若"可重算"就算语义可选，那它该保留 default | **维持必需**。理由：模型里**不存在** load 时重算深度的路径，`depth_of()` 在缺键时只能给出误导性的 `CommitNotFound`；而写入器 100% 会写出它（`genesis`/`append`/`fork_anonymous` 都维护），所以缺键**只可能**来自截断。**但**如果将来要引入"load 时重算"，正确做法是"保留 default + 重算"，而不是把 default 删掉 —— 这一点登记在此，交集成者知悉 |
| `MidiNote::pitch_bend_curve` / `phonemes` | **保留 default** | D43 说"宽容读与省略写必须成对取消"；也可以选择"要求必需 + 总是写出" | **维持保留**。理由：这两个字段与写入侧 `skip_serializing_if = "Vec::is_empty"` 是**格式自身的对偶**（空集 = "无弯音/无音素"，属 D43 第 2 条豁免的"空集=无"）；改成必需会让**每个音符**多写两个键，换来零语义增量。判据 ②③④ 会同时钉住这个对偶：一旦单边改动，写入器输出立刻读不回来 |
| `ProjectMetadata::description` / `tags`、`YebanProjectV1::author` | **保留 default** | 同族的 `created_at_unix_ms` 被改成了必需，为什么注记可以留 | **维持保留**。理由：时间戳的 `0` 与**真实时间**不可区分（默认值销毁信息）；而空串/空集在注记字段上**就是**"没有注记"的确定编码，不伪造任何事实、不改变任何工程行为。两者不是"松紧不一"，而是 D43 自己划的那条线 |

---

## 3. 被删掉 default 的字段清单（= 完整 contract 输入）

按对象分组，**共 41 项**。`路径` 列是"相对工程 JSON 根"的路径，`*` 表示"按确定顺序取第一个能让剩余路径成立的子值"（`BTreeMap` 键序 / 数组下标 / 枚举变体）。

| # | 对象 | 字段 | 填充样本里的路径 | 真实错误文本（`serde_json::from_str::<YebanProjectV1>`，逐字） |
| ---: | :--- | :--- | :--- | :--- |
| 1 | `YebanProjectV1` | `rng_seed` | `rng_seed` | ``missing field `rng_seed` at line 1 column 4975`` |
| 2 | `YebanProjectV1` | `metadata` | `metadata` | ``missing field `metadata` at line 1 column 4844`` |
| 3 | `YebanProjectV1` | `transport` | `transport` | ``missing field `transport` at line 1 column 4920`` |
| 4 | `YebanProjectV1` | `sections` | `sections` | ``missing field `sections` at line 1 column 4750`` |
| 5 | `YebanProjectV1` | `tracks` | `tracks` | ``missing field `tracks` at line 1 column 2846`` |
| 6 | `YebanProjectV1` | `master_bus_track_id` | `master_bus_track_id` | ``missing field `master_bus_track_id` at line 1 column 4955`` |
| 7 | `YebanProjectV1` | `routing_graph` | `routing_graph` | ``missing field `routing_graph` at line 1 column 4396`` |
| 8 | `YebanProjectV1` | `scenes` | `scenes` | ``missing field `scenes` at line 1 column 4899`` |
| 9 | `YebanProjectV1` | `clip_pool` | `clip_pool` | ``missing field `clip_pool` at line 1 column 4055`` |
| 10 | `YebanProjectV1` | `assets` | `assets` | ``missing field `assets` at line 1 column 4759`` |
| 11 | `ProjectMetadata` | `created_at_unix_ms` | `metadata/created_at_unix_ms` | ``missing field `created_at_unix_ms` at line 1 column 1570`` |
| 12 | `ProjectMetadata` | `modified_at_unix_ms` | `metadata/modified_at_unix_ms` | ``missing field `modified_at_unix_ms` at line 1 column 1569`` |
| 13 | `TransportConfig` | `metronome_enabled` | `transport/metronome_enabled` | ``missing field `metronome_enabled` at line 1 column 4954`` |
| 14 | `TransportConfig` | `count_in_bars` | `transport/count_in_bars` | ``missing field `count_in_bars` at line 1 column 4962`` |
| 15 | `TransportConfig` | `launch_quantization` | `transport/launch_quantization` | ``missing field `launch_quantization` at line 1 column 4952`` |
| 16 | `DeviceDefinition` | `bypassed` | `tracks/*/devices/0/bypassed` | ``missing field `bypassed` at line 1 column 3866`` |
| 17 | `DeviceDefinition` | `params` | `tracks/*/devices/0/params` | ``missing field `params` at line 1 column 3768`` |
| 18 | `DeviceDefinition` | `latency_samples` | `tracks/*/devices/0/latency_samples` | ``missing field `latency_samples` at line 1 column 3862`` |
| 19 | `MacroParameter` | `name` | `tracks/*/macros/0/name` | ``missing field `name` at line 1 column 4095`` |
| 20 | `MacroParameter` | `value` | `tracks/*/macros/0/value` | ``missing field `value` at line 1 column 4103`` |
| 21 | `MacroParameter` | `mappings` | `tracks/*/macros/0/mappings` | ``missing field `mappings` at line 1 column 3976`` |
| 22 | `AutomationLane` | `points` | `tracks/*/automation_lanes/0/points` | ``missing field `points` at line 1 column 3163`` |
| 23 | `AutomationLane` | `read_enabled` | `tracks/*/automation_lanes/0/read_enabled` | ``missing field `read_enabled` at line 1 column 3364`` |
| 24 | `AutomationLane` | `write_mode` | `tracks/*/automation_lanes/0/write_mode` | ``missing field `write_mode` at line 1 column 3363`` |
| 25 | `AutomationPoint` | `curve` | `tracks/*/automation_lanes/0/points/*/curve` | ``missing field `curve` at line 1 column 3151`` |
| 26 | `LoopConfig` | `enabled` | `tracks/*/clips/*/loop_config/enabled` | ``missing field `enabled` at line 1 column 3566`` |
| 27 | `LoopConfig` | `start_tick` | `tracks/*/clips/*/loop_config/start_tick` | ``missing field `start_tick` at line 1 column 3566`` |
| 28 | `LoopConfig` | `end_tick` | `tracks/*/clips/*/loop_config/end_tick` | ``missing field `end_tick` at line 1 column 3565`` |
| 29 | `ClipPoolEntry` | `name` | `clip_pool/*/name` | ``missing field `name` at line 1 column 1152`` |
| 30 | `ClipContent::Midi` | `notes` | `clip_pool/*/content/Midi/notes` | ``missing field `notes` at line 1 column 471`` |
| 31 | `ClipContent::Audio` | `gain_db` | `clip_pool/*/content/Audio/gain_db` | ``missing field `gain_db` at line 1 column 1292`` |
| 32 | `ClipPlacement` | `loop_config` | `tracks/*/clips/*/loop_config` | ``missing field `loop_config` at line 1 column 3549`` |
| 33 | `ClipPlacement` | `muted` | `tracks/*/clips/*/muted` | ``missing field `muted` at line 1 column 3597`` |
| 34 | `TrackV3` | `solo_safe` | `tracks/*/solo_safe` | ``missing field `solo_safe` at line 1 column 2951`` |
| 35 | `TrackV3` | `devices` | `tracks/*/devices` | ``missing field `devices` at line 1 column 2956`` |
| 36 | `TrackV3` | `macros` | `tracks/*/macros` | ``missing field `macros` at line 1 column 2957`` |
| 37 | `TrackV3` | `automation_lanes` | `tracks/*/automation_lanes` | ``missing field `automation_lanes` at line 1 column 2947`` |
| 38 | `TrackV3` | `clips` | `tracks/*/clips` | ``missing field `clips` at line 1 column 2958`` |
| 39 | `CommitGraph` | `commits` | `history.dag` | ``missing field `commits` at line 1 column 27`` |
| 40 | `CommitGraph` | `branches` | `history.dag` | ``missing field `branches` at line 1 column 26`` |
| 41 | `CommitGraph` | `depths` | `history.dag` | ``missing field `depths` at line 1 column 28`` |

**容器路径上的报错形状**（`.yeban` 里的 `project.json`，判据 ⑤ 实际断言的那一条）：

```
`project.json` is not a valid YebanProjectV1: missing field `rng_seed` at line 1 column 4975
```

**这张表怎么复现**（不是手抄，是脚本生成的）：

```bash
cargo test -p yeban-model --test no_compat -- --ignored --nocapture
```

生成器是 `#[ignore]` 的 `print_exact_error_text_table_for_the_notes`（刻意不进 CI 门禁，只用于刷新本表）。

> **注意**：错误文本里的 `at line 1 column N` 来自**紧凑序列化**的填充样本，因此 `N` 是确定的。两张表里 `enabled` 与 `start_tick` 的列号相同（`3566`）不是抄错 —— 它们相邻且短，serde 在同一个对象成员位置上报。

---

## 4. 判据清单（`crates/yeban-model/tests/no_compat.rs`，8 条 + 1 条 `#[ignore]`）

| # | 判据（函数名） | 测的是什么 | 覆盖量 |
| :-- | :--- | :--- | :--- |
| ① | `every_removed_default_is_now_required_with_exact_field_name` | 从**完整规范样本**里逐个删掉 §3 的 38 个项目字段 ⇒ `serde_json::from_str::<YebanProjectV1>` 必须失败，且错误文本**逐个**含 ``missing field `字段名` `` | 38/41（逐字段一条断言，失败信息带字段名 + 路径 + 实测文本） |
| ①b | `commit_graph_removed_defaults_are_required` | `CommitGraph::{commits,branches,depths}` 同样逐个必需 | 3/41 |
| ①c | `print_exact_error_text_table_for_the_notes`（`#[ignore]`） | 生成 §3 的表；不进 CI | — |
| ② | `tolerated_defaults_are_accepted_when_absent_and_take_declared_values` | 18 个**保留 default** 的字段：缺键 ⇒ **成功**且取到**声明默认值**（`author`→`""`、`tags`→`[]`、`unit`→`None`、`domain`→`None` 且回落到目标固有值域、`folder_id`/`color`→`None`、`tempo`→`None`、`gain_db`→`None`、`MidiNote` 七字段全默认且**不落盘**） | 18/18 |
| ③ | `removing_a_tolerated_default_from_the_filled_project_still_parses` | 从**完整工程**里**显式删掉**保留 default 的键（13 条在样本里真实存在的路径）⇒ 仍然可读且 `validate() == Ok` | 13（另 5 条在样本里本来就不落盘，由 ② 覆盖） |
| ④ | `production_writer_bytes_round_trip_exactly` | 本写入器产出的**字节** → 读回 → 再写出，**逐字节相同**；紧凑与美化两种口径都测；`filled_project` + `default_project` | 2 文档 × 2 口径 |
| ⑤ | `yeban_container_round_trips_the_strict_project` | `.yeban` 容器：写入确定性（两次写入逐字节相同）+ 读回后工程相等 + `history.dag` 逐字节相同 + 把缺 `rng_seed` 的工程装进容器必须被拒 | 4 断言 |
| ⑥ | `schema_version_mismatch_is_rejected_and_never_migrated` | `schema_version = 2` ⇒ `SchemaVersionTooNew{found:2,supported:1}`；`min_reader_version = 2` ⇒ `ReaderTooOld{required:2,actual:1}`；本版本 ⇒ `Ok`；缺 `schema_version` ⇒ ``missing field `schema_version` ``；并钉死 `SCHEMA_VERSION == READER_SCHEMA_VERSION == 1`（版本只用于**拒绝**，无迁移路径） | 6 断言 |
| ⑦ | `canonical_samples_are_still_legal_and_byte_stable` | `samples::export_all` 四份样本写盘成功（内含 `validate()` + `check_readable()`）、两次导出**逐字节相同**、两份工程样本读回后 `validate()`/`check_readable()` 通过且**再序列化逐字节等于原文**（含结尾换行） | 4 样本 |

**判据设计上的两条硬约束**（这两条就是"宽容读与省略写必须成对取消"的机械形式）：

- **④ 是"没有过度收紧"的总闸**：写入器省略的每一个键，读取器必须接受。任何一个被省略的键变成必需，④ 立刻红（注入 3 实测：8 条判据红了 7 条）。
- **①/② 是"没有过度放松"的总闸**：任何一个设计上必需的键被允许缺省，① 立刻红（注入 1 实测）。任何一个语义可选的键被改成必需，② 立刻红（注入 2 实测）。

### 4.1 注入记录（注入 → 变红 → 还原，全部实测）

| # | 注入 | 命令 | 结果（真实输出片段） | 还原 |
| :-- | :--- | :--- | :--- | :--- |
| 1 | 在 `TransportConfig::metronome_enabled` 上**加回** `#[serde(default)]`（放松一个必需字段） | `cargo-local.sh test -p yeban-model --test no_compat` | `every_removed_default_is_now_required_with_exact_field_name ... FAILED`<br>`过度宽容：缺 `metronome_enabled`（路径 transport/metronome_enabled）竟然读成功了` | ✅ 已删回（`git diff` 复核无残留） |
| 2 | 从 `YebanProjectV1::author` 上**删掉** `#[serde(default)]`（过度收紧一个语义默认字段） | 同上 | `tolerated_defaults_are_accepted_when_absent_and_take_declared_values ... FAILED`<br>`缺 author 必须可读: Error("missing field `author`", line: 0, column: 0)`<br>`removing_a_tolerated_default_from_the_filled_project_still_parses ... FAILED`<br>`过度收紧：删掉可选键 `author` 之后工程读不回来了：missing field `author`` | ✅ 已加回 |
| 3 | 在 `MidiNote::phonemes` 上**删掉** `#[serde(default)]` 但**保留** `skip_serializing_if = "Vec::is_empty"`（破坏写省/读宽对偶） | 同上 | `test result: FAILED. 1 passed; 7 failed`<br>`production_writer_bytes_round_trip_exactly`：`filled_project 读不回来：missing field `phonemes` at line 1 column 4041`<br>`yeban_container_round_trips_the_strict_project`：`读回容器必须成功: InvalidProjectJson { detail: "missing field `phonemes` at line 1 column 4041" }`<br>`canonical_samples_...`：`project.filled.json 读不回来：missing field `phonemes` at line 251 column 13` | ✅ 已加回 |

> 注入 3 是这次复审里最有价值的一条：它证明**写入器读不了自己的输出**这种事故会被门禁当场抓住（④⑤⑦ 同时红），而不是等到用户打开工程时才发现。

---

## 5. Contract 清单（交给集成者改 `schemas/project.schema.json`）

> `schemas/**` 由集成者独占，本线只给清单，不落手。清单里的路径以 `schemas/project.schema.json` 的 JSON 结构为准。
> **总原则**：schema 的 `required` 应当**等于实现要求的集合**（既不能更松 —— 会漏过坏文件；也不能更紧 —— 会拒绝本写入器的合法输出）。
>
> ### ⚠ 两条同样重要：§5.1–§5.3 是"该收紧的"，**§5.4 是"绝不能收紧的"**
>
> 集成者明确要求把豁免清单放在显眼位置（原话：**豁免比收紧更容易出错：收紧了语义上可选的字段就是 bug**）。
> 因此：**§5.4 的 18 个字段一律不得进任何 `required`** —— 其中 15 个本写入器**会在默认值上省略**
> （`Option::None` 或空集 + `skip_serializing_if`），另 3 个（`author` / `description` / `tags`）本写入器总是写出但**读取器接受缺失**。
> 一旦把 §5.4 的字段写进 `required`，本写入器自己的输出就会被 schema 判为非法。判据 ④（§4）是这条原则的机械形式。

### 5.1 根对象 `required` 应新增（7 项）

**要新增的 7 个键**：`rng_seed`、`metadata`、`transport`、`sections`、`master_bus_track_id`、`scenes`、`assets`。
**保持不动的 11 个键**：`schema_version`、`min_reader_version`、`writer_version`、`id`、`title`、`bpm`、`time_signature`、`audio_config`、`tracks`、`clip_pool`、`routing_graph`。

改完之后的根 `required`（按实现的结构体声明序，作为建议顺序）：

```json
"required": [
  "schema_version", "min_reader_version", "writer_version", "id", "title",
  "bpm", "time_signature", "audio_config",
  "rng_seed", "metadata", "transport", "sections",
  "tracks", "master_bus_track_id", "routing_graph",
  "scenes", "clip_pool", "assets"
]
```

`tracks` / `clip_pool` / `routing_graph` 此前是"**契约要求必需、实现却 `#[serde(default)]` 静默违反契约**"，这一轮已对齐：实现现在与契约一致，无需再改这三个键。

### 5.2 各对象 `required` 应新增的字段（按对象分组）

| JSON Schema 位置 | `required` 应新增 | 备注 |
| :--- | :--- | :--- |
| `properties.metadata.required` | `["created_at_unix_ms","modified_at_unix_ms"]` | **`description` / `tags` 不得进 required**（D43 豁免） |
| `properties.transport.required` | `["metronome_enabled","count_in_bars","launch_quantization"]` | 用户提示里的字段名正确，但**括号挂错了对象**：它们属于 `transport`，**不是** `audio_config`。`audio_config.required` 保持 `["sample_rate","block_size","bit_depth","pan_law"]` 不动 |
| `properties.tracks.additionalProperties.required` | 加 `"solo_safe"`、`"devices"`、`"macros"`、`"automation_lanes"`、`"clips"` | 现有 `["id","name","kind","volume_db","pan","mute","solo"]` 全部保留；**`folder_id`/`color` 不得进** |
| `properties.tracks.additionalProperties.properties.devices.items.required` | `["id","name","kind","bypassed","params","latency_samples"]` | 整个 `devices` 目前**不在 schema 里**，见 5.3 |
| `…devices.items.properties.params.items.required` | `["name","value"]` | **`unit` 不得进** |
| `…tracks.additionalProperties.properties.macros.items.required` | `["name","value","mappings"]` | |
| `…macros.items.properties.mappings.items.required` | `["target","depth"]` | 这两项**本来就没有 default**，列全以免遗漏 |
| `…tracks.additionalProperties.properties.automation_lanes.items.required` | `["target","points","read_enabled","write_mode"]` | ⚠ `automation_lanes` 是**数组**（不是对象）；**`domain` 不得进** |
| `…automation_lanes.items.properties.points.additionalProperties.required` | `["id","tick","value","curve"]` | `points` 是**对象**（`BTreeMap`，键为点身份） |
| `…tracks.additionalProperties.properties.clips.additionalProperties.required` | `["id","clip_id","start_tick","duration_ticks","loop_config","muted"]` | |
| `…clips.additionalProperties.properties.loop_config.required` | `["enabled","start_tick","end_tick"]` | |
| `properties.clip_pool.additionalProperties.required` | `["id","name","content"]` | `clip_pool` 现在是空壳（只有 `"type":"object"`） |
| `properties.clip_pool.additionalProperties.properties.content.oneOf[Midi].required` | `["notes"]` | `ClipContent` 是**外部标签枚举**：`{"Midi":{…}}` / `{"Audio":{…}}` |
| `properties.clip_pool.additionalProperties.properties.content.oneOf[Audio].required` | `["asset","gain_db"]` | |
| `…content.Midi.properties.notes.additionalProperties.required` | `["id","start_tick","duration_ticks","pitch","velocity"]` | **7 个表现力字段不得进 required**，见 5.4 |
| `properties.sections.additionalProperties.required` | `["id","name","start_tick","end_tick"]` | **`color` 不得进** |
| `properties.scenes.additionalProperties.required` | `["id","name"]` | **`tempo`/`color` 不得进** |
| `properties.assets.additionalProperties.required` | `["hash","original_path","byte_len","media_kind","license"]` | JSON 对象键是 64 位小写 hex SHA-256 |
| `properties.routing_graph.properties.edges.items.required` | `["id","source_node","destination_node","kind"]` | 现在是 `{"type":"object"}` 空壳；**`gain_db` 不得进** |

### 5.3 schema 里**根本不存在**、需要新增 `properties` 的对象

以下对象在 `schemas/project.schema.json` 里连 `properties` 都没有（因此"改 required"无从下手，必须新增）：

| 对象 | 实现类型 | 形态要点 |
| :--- | :--- | :--- |
| `properties.metadata` | `ProjectMetadata` | 4 个键；`created_at_unix_ms`/`modified_at_unix_ms` 为 `integer`（`minimum: 0`） |
| `properties.transport` | `TransportConfig` | `launch_quantization` 的枚举 = `["Off","Bar","TwoBars","FourBars","EightBars"]`；`count_in_bars` 为 `integer`（`minimum: 0`） |
| `properties.sections` | `BTreeMap<EntityId, SectionV3>` | 键为 ULID 文本；`additionalProperties` 见 5.2 |
| `properties.scenes` | `BTreeMap<EntityId, SceneV3>` | `tempo` 为 `["number","null"]` 且 `20.0..=999.0`；`color` 为 `["string","null"]` |
| `properties.assets` | `BTreeMap<AssetHash, AssetMetadata>` | `media_kind` 枚举 = `["Audio","Midi","ImpulseResponse"]` |
| `properties.tracks.additionalProperties.properties.{color,devices,macros,automation_lanes,clips}` | `TrackV3` | **`automation_lanes` 是数组**（`automation_lane_map` 手写 serde：序列化时按目标键序展开成数组，反序列化时收拢回 `BTreeMap` 并拒绝重复目标） |
| `properties.clip_pool.additionalProperties.properties.content` | `ClipContent` | 外部标签枚举，`oneOf` 两支 |
| `properties.routing_graph.properties.edges.items.properties.{id,source_node,destination_node,kind,gain_db}` | `RoutingEdge` | `kind` 枚举 = `["TrackToBus","BusToMaster","SendToAux","Sidechain"]`；`gain_db` = `["number","null"]` |
| `properties.tracks.additionalProperties.properties.devices.items…`（含 `params`） | `DeviceDefinition` / `ParameterValue` | `kind` 枚举 = `["InternalInstrument","InternalEffect","ExternalInstrument","ExternalEffect"]` |

顺带：`properties.tracks.additionalProperties.properties.solo_safe` 已存在（作为可选属性），**只需把它加进 required 即可**。

### 5.4 ⚠ **绝不能**加进任何 `required` 的字段（D43 豁免集，18 项）

这些字段**本写入器会在默认值上省略**（`Option::None` + `skip_serializing_if`，或空集）。一旦 schema 要求它们，**本写入器自己的输出就会被 schema 判为非法** —— 那比"漏检"更糟：它会制造一批"实现与契约互相指责"的假事故。

| 对象 | 不得 required 的字段 | 写入侧行为 |
| :--- | :--- | :--- |
| 根 | `author` | 总是写出（`""`），但**读取器接受缺失** |
| `metadata` | `description`, `tags` | 总是写出（`""` / `[]`），读取器接受缺失 |
| `tracks[*].devices[*].params[*]` | `unit` | `None` ⇒ **不落盘** |
| `tracks[*].automation_lanes[*]` | `domain` | `None` ⇒ **不落盘** |
| `tracks[*]` | `folder_id`, `color` | `None` ⇒ **不落盘** |
| `sections[*]` | `color` | `None` ⇒ **不落盘** |
| `scenes[*]` | `tempo`, `color` | `None` ⇒ **不落盘** |
| `routing_graph.edges[*]` | `gain_db` | `None` ⇒ **不落盘**（对比 `clip_pool[*].content.Audio.gain_db`：那里是裸 `f32`，所以**是**必需） |
| `clip_pool[*].content.Midi.notes[*]` | `probability`, `ratchet`, `micro_timing_ticks`, `slide`, `pitch_bend_curve`, `syllable`, `phonemes` | `None` / 空集 ⇒ **不落盘**（稀疏编码） |

一句话记忆法：**"写入器会省略的键，schema 一律不得 required"**；反过来，"schema required 的键，写入器必须总是写出、读取器必须要求存在"。判据 ④ 是这条原则的机械形式。

---

## 6. 本机真跑 vs CI：严格区分

### 6.1 本机真跑（**已绿**，接手者亲自执行）

**重要前提**：以下每一条跑的都是**本工作树** `/Users/crow/work/music/yeban/.worktrees/model-no-compat`（`7243409` 提交后的树，再 merge `origin/main` `42442af`）。判读依据有两条：(a) 第一次 `cargo test` 直接复现了 §1.1 那 4 条**旧政策断言的红** —— 若测的是主仓，它们会通过；(b) merge 后 `cargo-local.sh` 已按集成者的修复**打印实际工作区路径**，与上面一致。

| 命令 | 结果 |
| :--- | :--- |
| `bash scripts/gates/run-gates.sh light` | ✅ `门禁通过 (mode=light)`；14 条守卫全 ok；文档检查通过（`docs` 与 `licenses` 同绿）；依赖许可清单与依赖图一致（679 行，**无漂移**） |
| **`bash scripts/gates/run-gates.sh crate yeban-model`** | ✅ **`门禁通过 (mode=crate)`**（含 fmt → 14 条守卫 → 文档 → 许可 → `clippy --all-targets -D warnings` → `test`）。**本条曾经因 `scripts/**` 的 bash 3.2 空数组 bug 跑不了，见 §7 needs-1（已由 `main` `42442af` 修复）。** `crate` 档的 test 输出：`107 + 28 + 50 + 16 + 8 = 209 passed / 0 failed`（+1 `#[ignore]`） |
| `cargo clippy -p yeban-model --all-targets -- -D warnings` | ✅ 零告警（`Finished dev profile`） |
| `cargo test -p yeban-model`（经 `scripts/dev/cargo-local.sh`） | ✅ **107 + 28 + 50 + 16 + 8 = 209 passed，0 failed**（另有 1 条 `#[ignore]`，即错误文本生成器） |
| `cargo fmt --all --check` | ✅ |
| 注入 3 条（§4.1） | ✅ 3/3 按预期变红，且已全部还原 |

### 6.2 本机**未能**跑（**不是绿**，必须当 pending 或 needs 处理）

| 项 | 原因 | 谁来做 |
| :--- | :--- | :--- |
| `bash scripts/gates/run-gates.sh full` | 本机纪律禁止（`--workspace` 全量 / deny / schema 校验） | CI |
| 下游 crate 的编译与测试：`yeban-app` / `yeban-render` / `yeban-engine` / `yeban-mcp` / `yeban-decode` / `yeban-ui-mcp` | 全部含重依赖（实测 `heavy-deps.py` 退出码 0 = 含重依赖），本机不编译 | **CI（已判：见 §8.3，7 条下游腿全绿）** |
| `cargo deny` / `validate_schemas.py` / Windows 平台分支 | 本机不跑 | **CI（已判：见 §8.3）** |
| ~~`run-gates.sh crate yeban-model`~~ | ~~bash 3.2 空数组 bug~~ | ✅ **已解决**：集成者 `main` `42442af` 修好（`${extra[@]+"${extra[@]}"}`），并扩展 G14 守卫抓同类运行时可移植性（它当场又抓到 `ci-verdict.sh:32` 的 `"${AUTH[@]}"`）；本机复跑 `门禁通过 (mode=crate)` |

### 6.3 为什么"本机绿"不等于"绿"

本机唯一能真跑的 crate 是 `yeban-model`（零重依赖）。**下游 6 个 crate 全部有重依赖**，它们的编译/测试只能由 CI 判决。特别地：本线把字段从"可缺省"改成"必需"，属于**契约收紧**，其风险主要集中在**下游是否有人手写工程 JSON**。接手者已做静态排查（§7 needs-5），但**只有 CI 的 `cargo test --workspace --all-targets` 才算证据**。

---

## 7. 边界 / needs / pending

### needs（需要别人动手，本线无权或不属本线范围）

| # | 对象 | 内容 | 依据 |
| :-- | :--- | :--- | :--- |
| **needs-1** | ~~`scripts/gates/run-gates.sh:155`~~（**集成者独占**） | ✅ **已解决（`main` `42442af`，集成者修复并确认是本线的"仪器缺陷"）**。bug 原文：`run "clippy[$crate]" cargo clippy -p "$crate" --all-targets "${extra[@]}" …` 在 `set -u` + **bash 3.2** 下，当 `extra=()`（即**除 `yeban-engine` 外的所有 crate**）时报 `extra[@]: unbound variable` 并退出 1。最小复现：`/bin/bash -c 'set -uo pipefail; extra=(); printf "[%s]\n" "${extra[@]}"'`。修法：`${extra[@]+"${extra[@]}"}`；集成者同时把 **G14 扩展成运行时检查**（含 `set -…u` 的脚本里 `"${arr[@]}"` 必须写成 `${arr[@]+"${arr[@]}"}`），当场又抓到 `scripts/dev/ci-verdict.sh:32` 的 `"${AUTH[@]}"`（无 token 时读判决的脚本自己会炸），一并修好。**本机已复跑 `run-gates.sh crate yeban-model` = ✅ 门禁通过 (mode=crate)** | 实测（修复前红线 + 修复后复跑全绿） |
| **needs-2** | `schemas/project.schema.json`（**集成者独占**） | 按 §5 清单把 7 个根字段 + 各对象 `required` 收紧，并新增 §5.3 的 9 组 `properties`。**特别注意 §5.4 的 18 项不得进 required** | 本笔记 §5 即 contract |
| **needs-3** | `crates/yeban-render/src/render.rs:341,417-418,1290,1322-1325` + `crates/yeban-engine/src/graph.rs:36,150-151,881,911,925` + `crates/yeban-render/examples/support/export_pipeline.rs:150`（**他线范围**） | **文档/注释语义已过期**：它们仍写"`latency_samples == 0` 在模型层的定义是'**未上报**'"。D43 之后 `0` 是**真实的零延迟上报值**，"未上报"这个状态**已不存在**（缺字段直接报错）。**代码算术不受影响**（`0` 仍按 0 参与 `saturating_add` 求和），因此**不需要改行为**，但要改文档与"未上报"这个词。`docs/ledger/render-master-notes.md:218` 曾把"0 与真零延迟不可区分"登记为**已知边界**并建议"改成 `Option<u32>`" —— D43 用**另一种方式**解决了它（要求显式写出），该行也应更新 | 实测 grep |
| **needs-4** | [`model-automation-notes.md`](model-automation-notes.md)（**他线账本**） | 该账本把"三个新字段可缺省"当成**契约**写进了 9 处，D43 之后全部反了。逐处：① 第 **27** 行"新字段全部 `#[serde(default)]` + 默认值不落盘"；② 第 **54** 行 `read_enabled` 的"`default = true`，`skip_serializing_if = is_true`"；③ 第 **55** 行 `write_mode` 的"`default`，`skip_serializing_if = is_off`"；④ 第 **204** 行裁决 A5"默认 `true / Off / None`，且默认值不落盘"；⑤ 第 **281** 行判据 ④b"取值域排序/**默认值不落盘**/非法输入被拒"；⑥ 第 **282–283** 行判据 ⑤a/⑤b 仍以**旧函数名**登记"旧工程兼容"（已改名为 `lane_missing_read_enabled_write_mode_or_points_is_rejected` / `project_document_with_a_legacy_shaped_lane_is_rejected`）；⑦ 第 **296–297** 行"`cargo test -p yeban-model` 全绿：107+28+50+16"（本笔记 §1.1 已说明这句话的来历）；⑧ 第 **301–302** 行"`read_enabled` 是默认值 `true` ⇒ 不落盘"；⑨ 第 **374–383** 行 §7"注入 3：删掉三个新字段的 `#[serde(default)]`"—— 该注入在 D43 之后**就是目标状态**，不该再被登记为"需还原的红"。**本线不改他线账本**，只登记 | 实测 grep（行号已逐条 `sed -n` 复核） |
| **needs-5** | `docs/adr/ADR-0001-workspace-topology-and-version-pinning.md:127`（**人类负责人/集成者**） | 该行仍写"`latency_samples`（`yeban-model`，`#[serde(default)]` 取 0 表示'未上报'）"。它是 D43 之前的记录，**不应由本线改写决策记录**，但需在 ADR 的修订记录里注明已被 D43 覆盖 | 实测 grep |

### 边界（本线明确不碰）

| 项 | 说明 |
| :--- | :--- |
| `schemas/**` | 集成者独占；本线只给 §5 清单 |
| `scripts/**`、`.github/**`、`Cargo.toml`、`docs/DEVELOPMENT_LEDGER.md` | 集成者独占（needs-1 因此只报告不修改） |
| `crates/yeban-app/**`、`crates/yeban-render/**`、`crates/yeban-engine/**`、`crates/yeban-mcp/**`、`schemas/**` 的**语义/文档**一致性 | needs-3/needs-4 只报告 |
| 零新增依赖 | ✅ 本线未改 `Cargo.toml` / `Cargo.lock` / 许可清单（`run-gates.sh light` 的 licenses 检查已实测"与依赖图一致"） |

### pending（一律以"是否读到判决"为准）

| 项 | 状态 |
| :--- | :--- |
| 本线**代码 tip**（`7243409`）的判决 | ✅ **已读回 = success**（run `37246707068`，12 绿 / 0 红 / 1 跳过）—— 见 §8.3 |
| 本线**文档 + merge tip** 的判决 | **pending**（第 2 轮，推送后回填 §8.3） |
| 下游 6 个重依赖 crate 的编译/测试 | ✅ **已判绿**（`rust (yeban-app/mcp/render/engine/ui-mcp/decode)` 六条腿全绿） |
| `cargo deny` / `validate_schemas.py` / Windows 平台分支 | ✅ **已判绿**（`deny` 51s / `checks` 40s（含 jsonschema 对账）/ `windows` 1m32s） |
| `rust (workspace 全量)` | **未被跑**（plan 按"只跑受影响集合"跳过）—— 这不是"绿"，是"未判"；但它覆盖的 crate 已由上面 7 条腿各自跑过 |

---

## 8. 提交与 CI 判决

### 8.1 修改文件清单与净行数

**A. 本线的净贡献 —— 代码提交 `7243409`（6 个文件，全部在 `crates/yeban-model/` 内）**

| 文件 | 净变化 | 归属 |
| :--- | ---: | :--- |
| `crates/yeban-model/src/project.rs` | `+193 −79` | 前任（38 处 default 删除 + 注释）＋接手者（旧判据反转 + 3 处文档订正） |
| `crates/yeban-model/src/commit.rs` | `+9 −3` | 前任（3 处 default 删除 + 注释） |
| `crates/yeban-model/src/music.rs` | `+10 −0` | 前任（仅新增说明注释；**7 处 default 全部保留**） |
| `crates/yeban-model/src/samples.rs` | `+4 −3` | 接手者（订正"旧工程可以完全没有它们"的过期注释） |
| `crates/yeban-model/tests/automation.rs` | `+66 −34` | 接手者（④b 改断言方向；⑤a/⑤b 反转为 D43 判据；文件头与节标题） |
| `crates/yeban-model/tests/no_compat.rs` | **新增 678 行** | 接手者（8 条判据 + 1 条 `#[ignore]` 生成器） |
| **代码合计** | **+960 −119**（5 改 1 增） | |

前任遗留的 3 文件 `+161 −61` 全部包含在上面（未回退任何一处分类决策）。

**B. 文档提交（第 2 轮）**：`docs/ledger/model-no-compat-notes.md`（本笔记，新增）。

**C. merge `origin/main` `42442af`（**不是本线的改动**）**：41 个文件 / `+7732 −493`，来自 `line/app-no-compat`、`line/audio-render`、`line/engine-mix`、`line/engine-sound`、`line/model-automation` 等线的落地与集成者的仪器修复。其中与本线相关的只有：

- `scripts/dev/cargo-local.sh`（假绿根因修复 + 打印实际工作区）、`scripts/gates/run-gates.sh`（`extra[@]` 修复）、`scripts/guards/policy_check.py`（G14 运行时检查）、`scripts/dev/ci-verdict.sh`（`AUTH[@]` 修复）；
- `Cargo.lock`（+1 行，`yeban-mcp` 增 `yeban-decode`；**零新增外部包**）与 `docs/ledger/dependency-licenses.md`（Cargo.lock 哈希）—— 归属核查见 §8.4。

### 8.2 提交前复核（L24/L26/L28 纪律）

- `git add -A` **之前**先看 `git diff --cached --stat`，并确认行数与本节一致（行数异常 = 数据丢失，立刻停）。**已执行**：`+960 −119` / 6 文件 / 无 `Cargo.*`、无 `scripts/**`、无 `schemas/**`、无 `crates/yeban-app/**`。
- 提交后立刻 `git show --name-only 7243409` 与 `wc -l` 复核关键文件。**已执行**：文件清单 = 上面 6 个；`git show 7243409:crates/yeban-model/src/{project,commit,music}.rs | grep -c '^\s*#\[serde(default'` = **11 / 0 / 7**；`no_compat.rs` = **678** 行。
- **零新增依赖**：本线**没有**改 `Cargo.toml` / `Cargo.lock` / `THIRD_PARTY_LICENSES.md` / `docs/ledger/dependency-licenses.md`（merge 之后 `Cargo.lock` 与许可清单出现 diff，已按"先查清"逐条归属到 main，见 §8.4）。

### 8.3 CI 判决（**已读回**，不是 pending）

推送节奏按 L23/L26 执行：**先推代码，读回判决，再推文档**（本笔记与 merge 同一轮）。

**第 1 轮 = 代码 tip（commit `7243409`）= ✅ success**

```text
run id  : 37246707068   （line/model-no-compat, push 触发, tip 7243409）
结论    : success —— 12 绿 / 0 红 / 1 跳过（rust (workspace 全量) 由 plan 跳过）
  ✓ plan (受影响集合)                 6s
  ✓ checks (fmt / 红线守卫 / schema)  40s
  ✓ lockfile (确定性 Cargo.lock)      18s
  ✓ deny (cargo-deny 开源合规)        51s
  ✓ rust (yeban-model)               1m8s    ← 本线的目标（含 tests/no_compat.rs 的 8 条判据）
  ✓ rust (yeban-app)                 4m3s
  ✓ rust (yeban-mcp)                 45s
  ✓ rust (yeban-render)              59s
  ✓ rust (yeban-engine)              50s
  ✓ rust (yeban-ui-mcp)              2m33s
  ✓ rust (yeban-decode)              47s
  ✓ windows (yeban-mcp / yeban-model 的平台分支)  1m32s
  - rust (workspace 全量)             跳过（plan：只跑受影响集合）
```

读回方式：`bash scripts/dev/ci-verdict.sh line/model-no-compat`（退出码 `0`）。
这条判决覆盖 §8.2 列出的全部风险项：**7 条下游腿 + Windows 平台分支 + deny + schema 对账 + lockfile 全部绿**，因此"契约收紧会不会打坏下游"这个问题由 CI（而不是本机推测）给了答案。

**第 2 轮 = 文档 + merge `origin/main` `42442af` 的 tip**：_（由接手者在推送后回填）_
—— 该轮包含本笔记、`origin/main` 的仪器修复（`42442af`）、以及 `git merge` 带入的 `Cargo.lock`／`docs/ledger/dependency-licenses.md` 变更（**均来自 main，不是本线**，见 §8.4）。

### 8.4 `Cargo.lock` / 许可清单的变更归属（"若变了先查清"）

合并 `origin/main` 之后 `Cargo.lock` **确实变了**，因此按要求查清：

| 事实 | 证据 |
| :--- | :--- |
| **本线的代码提交 `7243409` 没有碰依赖** | `git show --name-only 7243409` = 6 个文件，全部在 `crates/yeban-model/` 下；无 `Cargo.toml` / `Cargo.lock` / 许可清单 |
| `Cargo.lock` 的 +1 行来自 **main** | `git diff 632b0c0..origin/main -- Cargo.lock` = `yeban-mcp` 的依赖清单新增 `"yeban-decode"`（**workspace 内部 crate**，来自 `line/audio-render` 的"音频片段真进母带"），**没有新增任何外部第三方包** |
| 许可清单只改了 `Cargo.lock` 的哈希 | `dependency-licenses.md` 的唯一改动是 `Cargo.lock` SHA-256 前 16 位 `99fda2dd47a2c6f6` → `03bcf6fb3c36b54d`；外部依赖包数仍是 **618** |
| 机械核对 | `run-gates.sh light` 的 `license_inventory.py --check` = ✅「依赖许可清单与依赖图一致 (679 行)」（merge 后复跑） |
| 仍然零新增依赖 | ✅ 本线引入的第三方依赖 = **0**；`proptest`/`serde_json` 等均为既有 dev/regular 依赖，未改 `Cargo.toml` |

---

## 9. 一句话总结

**前任留下了正确的方向、错误的"全绿"结论和零证据；接手者复审了全部 59 处 `#[serde(default)]` 分类（未发现误删误留，登记了 3 处判断）、把 4 条与 D43 相反的旧判据原地反转、补上 8 条机械判据（41 个必需字段逐字段点名 + 18 个语义默认逐字段取默认值 + 字节往返 + 容器 + 版本门 + 规范样本）、用 3 次注入证明判据真的会红，并把 `schemas/**` 需要的 contract 清单（含集成者特别要求的 18 项豁免清单）和 5 条跨线 needs 交了出去。**

**代码 tip 的 CI 判决 = ✅ success（run `37246707068`，12 绿 / 0 红 / 1 跳过）** —— 其中 **7 条下游腿（app / mcp / render / engine / ui-mcp / decode / model）+ Windows 平台分支 + deny + schema 对账 + lockfile 全部绿**，因此"契约收紧会不会打坏下游"由 CI 而非本机推测给了答案。

另外，`needs-1`（`run-gates.sh` 的 bash 3.2 空数组 bug）与"假绿前提"的**根因**都已被集成者在 `main` `42442af` 修掉并回写（教训 L30）；本线已 merge 该修复并复跑 `run-gates.sh crate yeban-model` = ✅ 门禁通过 (mode=crate)。
