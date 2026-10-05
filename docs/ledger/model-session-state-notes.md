# model-session-state 工作线 notes —— `MODEL-ISO-001` **缺失的那两层**

> 工作线: `line/model-session-state`（工作树 `.worktrees/model-session-state`，基线 `main` `1f551fd`）
> 拥有: `crates/yeban-model/**`
> 新增: `crates/yeban-model/src/session.rs`、`crates/yeban-model/src/local_config.rs`、
> `crates/yeban-model/tests/model_isolation.rs`、本文件
> 修改: `crates/yeban-model/src/lib.rs`（模块与再导出）
> **未碰**: `schemas/**`、根 `Cargo.toml`、`Cargo.lock`、`.github/**`、`scripts/**`、`deny.toml`、
> `docs/adr/**`、`docs/YEBAN_*.md`、其它 `crates/**`、其它 `docs/ledger/**` 台账

---

## 0. 一句话结论

接手时 `MODEL-ISO-001` 的**三层里只有第一层存在**（`YebanProjectV1`），
第二层（`SessionRuntimeState`）与第三层（`LocalMachineConfig`）**在代码里根本没有类型**。
本线把两层补齐，并把"**严禁持久化**"与"**凭据只存引用**"做成了**可机械证明**的判据：

| 承诺 | 机械证据（不是注释） |
| :--- | :--- |
| 会话态严禁持久化 | 类型层：`SessionRuntimeState` **没有** `Serialize`，`compile_fail` doc-test 常驻且带**必须编译通过**的对照；字节层：工程逐字节样本 SHA-256 冻结；结构层：递归键路径集合冻结；源码层：`session.rs` 真代码零 `serde` 记号（扫描器有合成输入对照） |
| 会话态改不动工程 | 改 `playhead` / `is_playing` / 任务 / PID / 视窗 / 撤销游标之后，`serde_json::to_string(&project)` **一位不变**；反面对照：改工程 `bpm` 必须变红 |
| 本机配置不进工程 | 容器条目集合必须是 `project.json` + `history.dag`；容器 `project.json` 递归键名里不得出现配置字段名；容器**原始字节**里不得出现配置里的引用名 / 设备名；工程目录里不得多出配置文件 |
| 凭据只存引用指针 | `SecretRef` 序列化形状是**一个 JSON 字符串**；`SecretMaterial` 没有 `Serialize`（`compile_fail` doc-test）；`Debug` 打码；假密钥探针证明本体不入盘 |
| keychain 未接入 = 明确错误 | 未知后端 ⇒ `UnknownBackend`；已登记但未接入的后端与默认 `unavailable` ⇒ `BackendUnavailable`，**绝不静默成功、绝不 panic** |

---

## 1. 接手时的一手事实（复核过，不重复挖）

| 事实 | 复核方式 | 结论 |
| :--- | :--- | :--- |
| 持久化层存在 | 读 `crates/yeban-model/src/project.rs:1501` | `YebanProjectV1` 含 19 个顶层键，`TransportConfig` 等在位 |
| 会话层不存在 | 接手前 `grep -rn "struct SessionRuntimeState" crates/` | **无命中** |
| 本机配置层不存在 | 接手前 `grep -rn "struct LocalMachineConfig" crates/` | **无命中** |
| `D45` 的游标无处安放 | 读 `crates/yeban-model/src/commit.rs:181-202` | `UndoCursor` 已存在并自述"属于会话运行态…刻意不实现 `Serialize`"，但**没有任何一层能装它** |
| 走带的 `playhead` / `is_playing` 没有归宿 | 读 `crates/yeban-engine/src/transport.rs:343-379` | `Transport::seek(tick)` / `is_playing()` 是运行时事实，模型层无对应类型 |
| `0600` / keychain 无类型可挂 | 全仓库检索 | 三层里最敏感的一层完全空白 |

⇒ 本线的接口形状**由 `UndoCursor` 的既有自述与规范原文决定**，没有发明新语义：
`SessionRuntimeState` 直接**托管** `UndoCursor`，于是 `D45` 那条线不再需要"临时把游标放在别处"。

---

## 2. 交付后的三层

```
crates/yeban-model/src/
  project.rs       ← 第 1 层 ProjectDocument（YebanProjectV1）—— 已有，本线未改一字
  session.rs       ← 第 2 层 SessionRuntimeState            —— 本线新增
  local_config.rs  ← 第 3 层 LocalMachineConfig             —— 本线新增
```

三层落在**三个不同模块**，由判据 ① 用 `std::any::type_name` 机械断言（不是靠约定）。
依赖方向是单向的：`session.rs` 只 `use crate::commit` 与 `crate::ids`，
**不** `use crate::project`；`local_config.rs` 不依赖另外两层。

### 2.1 第 2 层 `SessionRuntimeState`（`crates/yeban-model/src/session.rs`）

| 字段 | 类型 | 语义 | 为什么不许进工程 |
| :--- | :--- | :--- | :--- |
| `playhead_ticks` | `u64` | 播放头位置，**960 PPQ 整数 tick** | 同一份作品在两台机器上的"上次听到哪儿"必然不同 |
| `is_playing` | `bool` | 走带是否在播 | 打开工程时是否在播是会话事实 |
| `tasks` | `BTreeMap<TaskId, TaskProgress>` | 任务进度（类别 / 完成数 / 总数 / 可否取消 / 标签） | 进程内 IO 进度，重启即失效；`label` 可能含本机路径 |
| `plugin_processes` | `BTreeMap<EntityId, PluginProcess>` | 插件实例 → 宿主子进程快照（`pid`、`sandboxed`） | PID 是操作系统级运行时事实，跨机无意义 |
| `open_windows` | `BTreeSet<WindowId>` | 视窗打开状态（含 `PluginEditor(EntityId)`） | 显示会话状态 |
| `undo_cursor` | `UndoCursor` | 撤销游标 [ADR-0001 `D45`] | 规范明令属会话运行态 |

配套类型（全部 `BTreeMap` / `BTreeSet`，红线 4；全部不实现 `Serialize`）：

| 类型 | 形态 | 说明 |
| :--- | :--- | :--- |
| `TaskId` | 私有字段 `u64` 的 newtype | 刻意**不用** `EntityId`：给它跨进程稳定身份会诱导后续把它写进文件 |
| `TaskKind` | 6 个单元变体枚举 | `ProjectLoad` / `ProjectSave` / `AssetImport` / `PluginScan` / `Render` / `Export` |
| `TaskProgress` | 结构体 | `percent()` 用整数运算（**无浮点**）；`total_units == 0` 由 `validate()` 拒绝 |
| `PluginProcess` | 结构体 | `pid: u32`、`sandboxed: bool` |
| `WindowId` | 枚举（含载荷变体） | `Arrangement` / `PianoRoll` / `Mixer` / `Browser` / `PluginEditor(EntityId)` |
| `SessionStateError` | thiserror 枚举 | `TaskTotalZero`、`TaskProgressOutOfRange`（越界即 `Err`，绝不 panic） |

行为 API（关键几条）：`ticks_per_beat()`（= 960）、`ticks_per_bar(numerator, denominator)`
（整数运算，不整除时返回 `None` 而不是取整）、`playhead_bar_and_offset()`、`seek_ticks()`、
`advance_ticks()`（饱和加法，不 wrap）、`plugin_pids()`（去重 + 升序）、
`open_window()` / `close_window()` / `windows_in_order()`、`set_task()` / `task_progress()`、`validate()`。

### 2.2 第 3 层 `LocalMachineConfig`（`crates/yeban-model/src/local_config.rs`）

| 字段 | 类型 | 语义 |
| :--- | :--- | :--- |
| `version` | `u32` | 本机配置 schema 版本（[`LOCAL_CONFIG_VERSION`] = 1，独立于工程 `SCHEMA_VERSION`） |
| `audio_binding` | `AudioPortBinding` | 声卡物理端口绑定 |
| `external_editors` | `BTreeMap<EditorRole, ExternalEditor>` | 外部编辑器角色 → **绝对路径** + 参数 |
| `cloud_tokens` | `BTreeMap<String, SecretRef>` | 云服务商 → **密钥链引用指针**（绝不含密钥本体） |

| 子类型 | 字段 | 说明 |
| :--- | :--- | :--- |
| `AudioPortBinding` | `input_device` / `output_device`（`Option<String>`）、`input_ports` / `output_ports`（`BTreeMap<u32, String>`） | 端口序号 → 端口标识；`None` 表示"未绑定"（语义上等于系统默认） |
| `EditorRole` | `Waveform` / `Score` / `Sample` | 角色是编译期已知集合 |
| `ExternalEditor` | `absolute_path: PathBuf`、`arguments: Vec<String>` | 相对路径**拒绝**（`EditorPathNotAbsolute`）；参数顺序是语义，故用 `Vec` 而非集合 |

落盘 API：`path_for_home(home)` / `default_path()`（`HOME` on Unix、`USERPROFILE` on Windows）、
`to_json()` / `from_json()`、`save_to(path)` / `load_from(path)`、`load_default()`、`validate()`。
`#[serde(deny_unknown_fields)]` 是刻意的：本文件由本实现独占写入，多出未知字段只可能意味着
"被手改"或"来自未来版本" ⇒ 拒绝而不是静默丢弃。

### 2.3 凭据三兄弟

| 类型 | 是什么 | 能不能落盘 | 证据 |
| :--- | :--- | :--- | :--- |
| `SecretRef` | **条目名**（指针），字段私有，`new()` 校验（非空、≤128 字节、禁空白与控制字符） | 能（且**只**有它能） | 序列化形状是一个 JSON 字符串；判据 ⑦ 断言 `is_string()` |
| `SecretMaterial` | **密钥本体**，`from_bytes` / `from_text` / `expose()` | **不能** | 无 `Serialize` / `Deserialize`（`compile_fail` doc-test）；`Debug` 只打印长度（`SecretMaterial(<redacted N bytes>)`）；`Drop` 用安全代码 `slice::fill(0)` 做 best-effort 清零 |
| `SecretStore` | 后端边界（`put` / `get` / `delete` / `backend_name`） | — | trait；默认实现 `UnavailableSecretStore` 每个方法都返回明确错误 |

---

## 3. "会话态严禁持久化"的四类机械证据

### 3.1 类型层：会话态没有 `Serialize`（编译失败测试 + 必须编译通过的对照）

`crates/yeban-model/src/session.rs` 的 `SessionRuntimeState` 文档里常驻两条 doc-test：

```text
compile_fail: fn assert_serializable<T: serde::Serialize>() {}
              assert_serializable::<yeban_model::SessionRuntimeState>();     ⇒ 必须编译失败
对照（必须编译成功）: assert_serializable::<yeban_model::YebanProjectV1>();
```

**为什么"编译失败"能证明是我们想要的原因，而不是探针写错了**（三段论，都可机械复核）：

1. `SessionRuntimeState` 确实存在且可构造 —— 判据 ⑤ 用 `SessionRuntimeState::default()` 并改其字段；
2. `assert_serializable` 这个 bound 确实能用 —— 对照 doc-test 在 `YebanProjectV1` 上**编译通过**；
3. 唯一差别是 `T`。⇒ 失败只可能来自 `SessionRuntimeState: !Serialize`。

实测（本机真跑，见 §9）：`cargo test -p yeban-model --doc` 输出
`2 passed` 的 `compile fail` 组与 `3 passed` 的正向组。同样的机制也钉住 `SecretMaterial`。

### 3.2 字节层：工程逐字节样本冻结（判据 ④）

`crates/yeban-model/tests/model_isolation.rs` 冻结了两个规范样本的 SHA-256：

| 样本 | 冻结 SHA-256 |
| :--- | :--- |
| `serde_json::to_vec(default_project())` | `6bd76fcba2ad050dad2f3d5d8e9b92399d23636a2d2fd79911f42f29f61430f0` |
| `serde_json::to_vec(filled_project())` | `0218fa7d620d48e23363f3a49621d8af647f6abb10f8423feeaebafce057265b` |

同时断言 `to_vec` 与 `to_string` 的输出逐字节一致（防止"两套序列化入口"）。
**重新冻结的唯一入口**是 `#[ignore]` 的 `print_frozen_isolation_table`：

```text
bash scripts/dev/cargo-local.sh test -p yeban-model --test model_isolation -- --ignored --nocapture
```

诚实边界：workspace 版本号（`writer_version = env!("CARGO_PKG_VERSION")`）一变，这两条也会红。
失败信息里写明了判别方法：**diff 只来自版本号 ⇒ 合法升级（重新冻结）；diff 里出现新键 ⇒ 违规**。

### 3.3 结构层：递归键路径集合冻结（判据 ② / ③）

| 项 | 冻结值 |
| :--- | :--- |
| 顶层键（19 个，已排序） | `assets` `audio_config` `author` `bpm` `clip_pool` `id` `master_bus_track_id` `metadata` `min_reader_version` `rng_seed` `routing_graph` `scenes` `schema_version` `sections` `time_signature` `title` `tracks` `transport` `writer_version` |
| 递归键路径数量 | 223 |
| 递归键路径集合 SHA-256 | `067b6c04205ba26cc1ff049c8bccd03938841b7e81736194ac18b4413febdfbc` |

"递归键路径"= 对象键用 `.` 连接、数组元素折叠成 `[]` 的路径集合。
它拦得住**嵌套**字段（顶层键判据拦不住的那种）。这两条判据**自带反空洞对照**：
往工程 `Value` 里合成插入一个 `session_playhead_ticks` ⇒ 摘要与计数必须当场变红。

### 3.4 源码层：`session.rs` 真代码零 `serde` 记号（判据 ⑥）

判据用 `include_str!("../src/session.rs")` **剥掉整行注释**后断言真代码里没有
`serde` / `Serialize` / `Deserialize` / `HashMap` / `HashSet` / `f32` / `f64`。
为什么必须剥注释：类型层的 `compile_fail` 证据**写在注释里**，那里面必然出现 `Serialize`；
注释里的编译期证据由 doctest 阶段独立执行。

**判据自身有牙**（同一条判据里的对照，缺一不可）：

| 对照 | 期望 |
| :--- | :--- |
| 同一个扫描器扫 `project.rs` | **必须**命中 `Serialize` / `Deserialize`（否则"没命中"可能只是扫描器坏了） |
| 同一个扫描器扫 `local_config.rs` | **必须**命中 `Serialize`（它就该可持久化） |
| 合成输入 `#[derive(Serialize)] struct X;` | 必须命中 |
| 合成输入 `struct X { m: HashSet<u8> }` | 必须命中 |
| 合成输入 `// 注释里的 Serialize 不算` | 必须**不**命中 |

---

## 4. 本机配置在工程之外 + `0600` 实测

| 判据 | 断言 | 实测结果 |
| :--- | :--- | :--- |
| 落盘位置 | `path_for_home(home)` == `home/.yeban/config.json`；`default_path()` 是绝对路径且**不以** `CARGO_MANIFEST_DIR` 开头 | 绿 |
| 不落进工程目录 | 写容器 + 容器往返之后，工程目录的 `read_dir` 清单必须**恰好**是 `["song.yeban"]` | 绿 |
| 文件权限 | `metadata.permissions().mode() & 0o777 == 0o600` | 绿（Unix） |
| 目录权限 | 配置目录 `& 0o777 == 0o700` | 绿（Unix） |
| **已存在的宽权限文件被收紧** | 预置 `0644` 文件 ⇒ `save_to` 之后必须是 `0600` | 绿 —— 这条专门堵住"`OpenOptions::mode` 对已存在文件是空转"这个真实缺陷类 |
| 非 Unix | 如实 `eprintln!` SKIP，并写明 **Windows ACL 不等价于 0600、此判据未执行** | 非 Unix 平台 SKIP（不假装绿） |

实现要点（`crates/yeban-model/src/local_config.rs::save_to`）：
目录用 `DirBuilder::mode(0o700)` 递归创建；文件用 `OpenOptions::mode(0o600)` **在创建时就带上权限**
（不是先建再 chmod，避免权限过宽的时间窗）；写完后**再显式** `set_permissions(0600)` 覆盖"已存在文件"的情形；
`load_from` **不**做静默 chmod（那会掩盖"这份文件曾被别人读过"的事实）。

---

## 5. 密钥只存引用：证据与诚实边界

**探针**（判据 ⑦）：显式假密钥
`sk-PROBE-5f4d3c2b1a0F9E8D7C6B5A4f3e2d1c0b-DO-NOT-PERSIST` 放进内存后端 `ProbeSecretStore`，
配置里只放 `SecretRef::new("yeban/cloud/anthropic")`，然后 `save_to` 落盘：

| 断言 | 结果 |
| :--- | :--- |
| 文件里**有**引用名 `yeban/cloud/anthropic` | 绿（否则无从找回密钥） |
| 文件里**没有**假密钥（`PROBE` / `sk-` / 整串字节） | 绿 |
| `cloud_tokens` 的每个值都是 JSON **字符串**（结构上没有承载 material 的位置） | 绿 |
| `SecretRef` 的 `serde_json::to_value` 形状是字符串 | 绿 |
| `format!("{:?}", material)` 不含密钥且含 `redacted` | 绿 |
| 探针非空转：内存后端 `get` 必须真的取回假密钥 | 绿 |
| 探针有牙：故意把假密钥拼进 JSON 后，`contains_subslice` 必须命中 | 绿 |

**诚实边界（不吹）**：

1. `SecretRef` 装的是**名字**。如果有人在 UI 边界把真实密钥**当名字**粘进来，类型层拦不住 ——
   类型层能保证的是"**结构上只有一个字符串出口**"，不能保证"这个字符串的内容不是密钥"。
   缓解手段（校验：非空、≤128 字节、禁空白与控制字符）只能拦住"多行 PEM 之类"的形态，
   见 needs N5。
2. `SecretMaterial` 的 `Drop` 清零是 **best-effort**：编译器优化、swap、换页都可能留下副本。
   密码学意义的擦除需要 `zeroize` 类依赖 ⇒ 人类裁决，本线不引入。
3. 本线**没有**任何真实 keychain 读写 —— 这是刻意的（见 §6）。

---

## 6. keychain 后端的边界（trait + 默认不可用实现）

```text
pub trait SecretStore {
    fn backend_name(&self) -> &'static str;
    fn put(&self, reference: &SecretRef, material: &SecretMaterial) -> Result<(), SecretStoreError>;
    fn get(&self, reference: &SecretRef) -> Result<SecretMaterial, SecretStoreError>;
    fn delete(&self, reference: &SecretRef) -> Result<(), SecretStoreError>;
}
```

| 后端名 | `secret_store_for(name)` 的行为 | `put` / `get` / `delete` 的行为 |
| :--- | :--- | :--- |
| `unavailable`（`DEFAULT_SECRET_BACKEND`） | `Ok(UnavailableSecretStore)` | 每个方法 `Err(BackendUnavailable)` |
| `os-keychain` / `macos-keychain` / `windows-credential-manager` / `secret-service` | `Ok(UnavailableSecretStore::for_known_backend(...))` —— **认识名字但没接进来** | 每个方法 `Err(BackendUnavailable { backend, detail })` |
| 任何别的名字 | **`Err(UnknownBackend { requested, known })`** | 不适用（拿不到后端） |

**为什么这样切**：真实 OS Keychain 集成需要**新的外部依赖**
（`security-framework` / `keyring` / `windows` / `secret-service` 之类）⇒
按 `D51`（内部边裁决）的边界，跨依赖的决定必须由人类 / 集成者做。
所以本线只固定**边界与失败语义**，把后端留给后续线；由此产生的三条不可协商的性质：

1. 未知后端 **不静默降级**成默认后端（否则"配置写错了"会看起来像"密钥存好了"）；
2. 未接入后端 **不静默成功**（否则会有人误以为密钥已安全保存）；
3. 任何失败路径都**不 panic**（判据 ⑫ 逐后端实测）。

`SecretStore` 刻意**不加** `Send + Sync` 约束：那会替后续线做一个它可能不想要的裁决
（后端句柄是否跨线程）。这是登记在 needs N1 的显式悬置项。

---

## 7. 判据清单（`crates/yeban-model/tests/model_isolation.rs`，16 条 + 1 条 `#[ignore]`）

| # | 判据（函数名） | 钉住什么 | 反空洞对照 |
| :-- | :--- | :--- | :--- |
| ① | `three_layers_live_in_three_distinct_modules` | 三层是三个模块（`type_name` 断言） | 三个模块名互不相同 |
| ② | `project_json_top_level_keys_are_frozen` | 工程顶层键集合 = 19 个冻结键 | — |
| ③ | `project_json_recursive_key_paths_are_frozen` | 递归键路径计数 223 + 集合 SHA-256 | 合成插入一个会话键 ⇒ 必须变红 |
| ④ | `project_json_byte_samples_are_frozen` | 两个样本的逐字节 SHA-256；`to_vec` 与 `to_string` 一致 | 改工程 `title` ⇒ 必须变红 |
| ⑤ | `session_mutation_cannot_move_project_bytes` | 改会话（含 `undo_cursor.skip`）后工程字节/文本一位不变 | 改工程 `bpm` ⇒ 必须变红 |
| ⑥ | `session_state_has_no_serde_surface` | `session.rs` 真代码零 `serde` / `Serialize` / `HashMap` / `HashSet` | 对 `project.rs`、`local_config.rs` 的正向命中 + 5 条合成输入对照 |
| ⑦ | `local_config_is_absent_from_the_project_container` | 容器条目集合 / 递归键名 / **原始字节**里都没有本机配置 | 故意把配置塞进容器 ⇒ 字节探针必须命中 |
| ⑧ | `local_config_stays_outside_after_container_round_trip` | 容器往返后配置仍在本机路径、工程目录只有容器、配置逐字段不变 | — |
| ⑨ | `local_config_file_is_0600` | 文件 `0600` + 目录 `0700` + **已存在 0644 被收紧** | `0o644 != 0o600` 探针；非 Unix 如实 SKIP |
| ⑩ | `local_config_round_trip_is_field_exact_and_deterministic` | 两次写字节相同、乱序插入字节相同、往返逐字段相等 | 改一个字段 ⇒ 字节必须变；6 条坏输入必须被拒 |
| ⑪ | `secret_material_never_reaches_disk` | 假密钥不入盘、只有引用名、`SecretRef` 是字符串、`Debug` 打码 | 内存后端必须真的取回假密钥；故意拼假密钥必须命中 |
| ⑫ | `unknown_secret_backend_is_an_explicit_error` | 未知后端 `UnknownBackend`；默认与未接入后端 `BackendUnavailable`（不 panic） | 逐后端断言错误变体与 `backend` 名 |
| ⑬ | `playhead_is_integer_ticks_at_960_ppq` | `playhead_ticks` 绑到 `u64`（改浮点即编译失败）、`PPQ == 960`、整数小节运算、饱和加法 | `session.rs` 真代码零 `f32` / `f64` |
| ⑭ | `window_and_pid_sets_are_deterministic` | 乱序插入 ⇒ 视窗升序、PID 去重升序、幂等；任务进度整数百分比与 `validate()` 边界 | 相邻元素严格升序断言；两次遍历相同 |
| ⑮ | `crate_keeps_forbid_unsafe_and_zero_gui_deps` | `#![forbid(unsafe_code)]` 在位；清单零 GUI 依赖、零 keychain 类依赖 | 清单确实被读到（含 `serde`、`[dependencies]`）；属性探针对照 |
| ⑯ | `editor_path_absoluteness_follows_platform_semantics` | 编辑器路径绝对性是**平台语义**：相对路径全平台拒；POSIX 绝对路径 Unix 接受；盘符路径 Windows 接受；无盘符的"有根路径"Windows **拒** | 两个 `is_absolute()` 事实先断言，再断言校验结果（见 §9.2 的真实事故） |
| ⑰ | `print_frozen_isolation_table`（`#[ignore]`） | 重新冻结常量的**唯一**取证入口 | — |

另有 **2 条 `compile_fail` doc-test + 3 条正向 doc-test**（§3.1），随 `cargo test -p yeban-model` 一起跑。

---

## 8. 注入记录（4 条：注入 ⇒ 变红 ⇒ 还原）

每条都在**提交前**做过、并把源码还原到逐字节相同（下方 sha256 是**还原后**的最终值）。

### 注入 ①：把会话态字段加进 `YebanProjectV1`

- 动作：`project.rs` 的 `YebanProjectV1` 加 `#[serde(default)] pub session_playhead_ticks: u64`，
  `samples.rs` 的字面量补字段（否则编译不过）。
- 期望：判据 ①（键集合）红。
- **实测红 4 条**：
  - `project_json_top_level_keys_are_frozen`：`left` 多出 `session_playhead_ticks`；
  - `project_json_recursive_key_paths_are_frozen`：计数 `224` vs 冻结 `223`；
  - `project_json_byte_samples_are_frozen`：`39e77d29…` vs 冻结 `6bd76fcb…`；
  - `local_config_is_absent_from_the_project_container`：容器里 `project.json` 键集合也多了它。
- 还原复核：`project.rs` = `2ac19d347acd38c61b15ad3b15d44ed8c431a185d967b1b4c6171e19f08ddb90`（与注入前备份相同），
  `samples.rs` = `99d56109d3da037c2a87a98c2da3f8af523cfdb0fc40c8f26a2b81b1c80725ae`（相同）。

### 注入 ②：把本机配置写进工程容器

- 动作：`container/mod.rs::write_project_container` 追加第三个条目
  `local/config.json`，内容含 `"anthropic":"yeban/cloud/anthropic"`。
- 期望：判据 ⑦（本机配置不进容器）红。
- **实测红 2 条**，且**字节探针**直接命中：
  `工程容器字节里出现了本机配置内容: "yeban/cloud/anthropic"`
  （另一条 `local_config_stays_outside_after_container_round_trip` 因容器往返也红）。
- 还原复核：`container/mod.rs` = `3a8a7276c1f27f6164d877eab216c8522195cd463759b6c1669245397cb3c5d7`（与备份相同）。
- 复核时把"字节探针"断言**前移**到"条目集合"断言之前，正是为了这次注入能证明
  **内容检测器**（而不只是条目名检测器）有牙。

### 注入 ③：把密钥本体写进文件

- 动作：`local_config.rs::to_json` 注入一个额外键 `token_material`，值是假密钥字符串。
- 期望：判据 ⑪（密钥 material 不入盘）红。
- **实测红 4 条**，其中判据 ⑪ 精确命中：
  `文件里出现了假密钥片段 ⇒ 密钥本体泄漏`；另有本机配置键集合断言
  `{"audio_binding","cloud_tokens","external_editors","token_material","version"}` vs 冻结的 4 键。
- 还原复核：`local_config.rs` = `0babbdebb02a5afcd8169565d64fe046c5ca5598526041ff7729b496262d2c2d`（与备份相同）。

### 注入 ④：把 `BTreeSet` 换成 `HashSet`

- 动作：`session.rs` 的 `open_windows: BTreeSet<WindowId>` 与 `plugin_pids() -> BTreeSet<u32>`
  改成 `HashSet`（红线 4 违规形态）。
- 期望：判据 ⑭ 红（或直接违反红线 4）。
- **实测红 3 处**：
  - 判据 ⑭ `window_and_pid_sets_are_deterministic`：
    `left: [Browser, Arrangement, Mixer, PluginEditor(...), PianoRoll, PluginEditor(...)]`
    vs `right:` 升序序列；
  - 判据 ⑥ `session_state_has_no_serde_surface`：源码棘轮命中 `HashSet`（这是刻意的第二道网）；
  - 守卫 `scripts/guards/policy_check.py` **G01 [MODEL-AST-003] 4 处违规**并列出 4 行。
- 还原复核：`session.rs` = `59537a2d078e2e4657d28c88d0e585749c14e20c1f3bd02d6a35918f049e8ccd`（与备份相同）。

**四条注入全部还原，`git status` 里只剩本线应有的文件**（`lib.rs` 一行改动 + 四个新文件）。

---

## 9. 本机真跑 vs CI（严格区分）

### 9.1 本机（Apple M2，`bash scripts/dev/cargo-local.sh`，工作树自己的脚本路径）

| 命令 | 结果 |
| :--- | :--- |
| `bash scripts/dev/cargo-local.sh check -p yeban-model` | `Finished`（0 error） |
| `bash scripts/dev/cargo-local.sh clippy -p yeban-model --all-targets -- -D warnings` | `Finished`（0 warning） |
| `bash scripts/dev/cargo-local.sh test -p yeban-model` | 107 单元 + 28 + 50 + 16 + **16** + 8 + 9 集成 + 3 + **2** doctest 全绿；3 个 `#[ignore]` 取证入口 |
| `bash scripts/gates/run-gates.sh light` | **门禁通过 (mode=light)**：fmt / 14 条守卫 / 文档契约 / 许可清单 |
| `bash scripts/gates/run-gates.sh crate yeban-model` | **门禁通过 (mode=crate)**：上述 + clippy + 全量测试（含 doctest） |

本机**没有**跑（按 `AGENTS.md` §5 纪律交给 CI）：`--workspace` 全量构建与测试、
`cargo deny check`、jsonschema 契约校验、Windows/Linux 平台分支、UI 无头视觉回归。
⇒ 本机绿**只是参考**，判决以 CI 为准。

### 9.2 CI 判决（**如实登记，包含一次真实的红**）

| run id | tip | 结论 | 明细 |
| :--- | :--- | :--- | :--- |
| 37268168161 | `35f8ad0` | **RED** | `deny` / `checks` / `lockfile` / `plan` / `rust (yeban-model)` / 其余 6 条 rust 腿 **全绿**；**`windows` 腿红**：`test` 步骤 exit 101，3 条 `local_config*` 判据失败（同一次运行里 `model_isolation` 的其它 **12 条判据在 Windows 上全绿**，`session*` 与键集合 / 逐字节 / 容器判据一条没红） |

**根因**（取自 `bash scripts/dev/ci-verdict.sh --logs 37268168161` 的原始日志，不是猜测）：

```text
panicked at crates\yeban-model\tests\model_isolation.rs:651:34:
写本机配置: EditorPathNotAbsolute { path: "/Applications/Audacity.app/Contents/MacOS/Audacity" }
```

即：**判据夹具自己写死了 macOS 路径**。`Path::is_absolute()` 是**平台相关**的 ——
在 Windows 上 `/Applications/...` 只是"有根"（rooted），**不是**绝对路径；
于是 `LocalMachineConfig::to_json()` 正确地拒绝了它，而夹具在 `.expect("序列化")` 上 panic。
**生产侧的"外部编辑器必须是绝对路径"校验行为完全正确**，那三条红是**夹具的缺陷**：

| 集成者当时给的候选原因 | 实测裁定 |
| :--- | :--- |
| 1. `0600` 权限在 Windows 不可用 | **不成立**：`PermissionsExt` 与 `LOCAL_CONFIG_FILE_MODE` 都在 `#[cfg(unix)]` 内，Windows 走 `#[cfg(not(unix))]` 的 SKIP 分支；Windows 腿的 `clippy -D warnings` **通过**（编译无问题），红的只有测试运行时 |
| 2. `~/.yeban` 在 CI 上不可写 / HOME 不同 | **不成立**：三条判据全部使用**显式临时目录**（`TempDir::new` + `path_for_home`），只有 `default_path()` 是只读探测；失败发生在 `save_to` 的**校验阶段**，不是 IO |
| 3. 判据写死了"本机平台" | **成立，但位置更精确**：写死的是**夹具数据**（编辑器路径），不是断言也不是落盘逻辑 |

**修法（不放宽任何断言）**：

1. 夹具改为**平台正确**：`WAVEFORM_EDITOR_PATH` / `SCORE_EDITOR_PATH` 用 `#[cfg(windows)]` 给盘符路径、
   `#[cfg(not(windows))]` 给 POSIX 路径；相对路径夹具 `RELATIVE_EDITOR_PATH` 全平台通用。
2. **新增判据 ⑯** 把平台语义**显式断言**出来（而不是让夹具"碰巧"过）：
   Windows 上"只有根、没有盘符"的 `\Program Files\...` 必须 `is_absolute() == false` 且被 `validate()` 拒绝；
   Unix 上 `/usr/bin/vi` 必须被接受。该常量只在 `#[cfg(windows)]` 下存在（在 Unix 上它是绝对路径，留着就是谎言）。
3. 容器泄漏探针同时扫描**实际夹具路径字节**，夹具换平台后探针不会静默失效。
4. 生产代码（`local_config.rs`）**一字未改** —— 这次修复没有动被测对象。
5. `session` 侧设计与实现**一字未改**（集成者预审通过的部分）。

**未采用的做法（按口径明确排除）**：把 `assert!` 放宽成"看情况"、让 SKIP 伪装成通过、
或把判据改成"仅 Unix 执行"（那会让 Windows 侧的路径语义**永远无人验证** ——
而它恰恰是刚刚真实出事的地方）。

修复后的判决（**已用 `ci-verdict.sh` 读回，不是预写**）：

| run id | tip | 结论 |
| :--- | :--- | :--- |
| 37268168161 | `35f8ad0` | **RED**（`windows` 腿 3 条 `local_config*`，根因=夹具写死 macOS 路径） |
| **37268589046** | **`4d497ef`** | **GREEN** —— `checks` / `deny` / `lockfile` / `plan` / **`windows`** / 7 条 `rust` 腿**全部 ✓**（windows 腿 2m1s，`rust (yeban-model)` 1m7s） |

读取命令：

```text
bash scripts/dev/ci-verdict.sh --watch line/model-session-state
```

**登记这次判决的文档提交本身又会产生一个新 tip**，它的判决在本文件落笔时是 `pending`；
按纪律**不预写"通过"**，需要时用同一条命令读回即可。`rust (workspace 全量)` 一直是 `-`（skipped）：
该腿只在宽运行时才跑，本线是窄运行。

## 10. 修改文件与净行数

| 文件 | 状态 | 行数 |
| :--- | :--- | :--- |
| `crates/yeban-model/src/session.rs` | 新增 | 389 |
| `crates/yeban-model/src/local_config.rs` | 新增 | 828 |
| `crates/yeban-model/tests/model_isolation.rs` | 新增 | 1412（含 Windows 修复新增的 136 行） |
| `crates/yeban-model/src/lib.rs` | 修改 | +18 / -2 |
| `docs/ledger/model-session-state-notes.md` | 新增 | 本文件 |
| 合计 | — | 约 +2650 行（其中判据 1412 行） |

**零新增依赖**：`crates/yeban-model/Cargo.toml` 与根 `Cargo.toml` / `Cargo.lock` 一字未改
（`run-gates.sh light` 的 `licenses` 步骤仍然一致：679 行）。

---

## 11. needs（交集成者 / 后续线）

| # | 需要什么 | 属于谁 | 为什么必须由它裁决 |
| :-- | :--- | :--- | :--- |
| **N1** | **真实 OS Keychain 后端**（macOS Keychain / Windows Credential Manager / Secret Service） | 人类 / 集成者（新外部依赖） | 需要 `security-framework` / `keyring` / `windows` / `secret-service` 之类**新依赖**，并要决定 `SecretStore` 是否加 `Send + Sync`、句柄生命周期、测试策略（CI 上没有真 keychain） |
| **N2** | `SecretMaterial` 的**密码学擦除** | 人类 / 集成者 | `zeroize` 是外部依赖；当前只有安全代码的 best-effort 清零 |
| **N3** | **`D45` 撤销入口接第 2 层** | `crates/yeban-store` / MCP 线 | `UndoCursor` 的**唯一合法容器现在是 `SessionRuntimeState::undo_cursor`**。撤销入口应持有 `SessionRuntimeState`（进程内），**不得**把 `skip` 写进 `Commit` / 快照。`yeban_model::UndoCursor` 的公开 API 未变，所以那条线只需换"把游标放哪儿" |
| **N4** | **走带接第 2 层** | `crates/yeban-engine` + app 层 | `Transport` 的 `playhead` / `is_playing` 是**引擎实例**的事实；会话态是**文档会话**的事实。建议方向：app 层作为镜像写入方（`SessionRuntimeState::seek_ticks` / `set_playing`），`yeban-model` **不得**反向依赖 `yeban-engine`（否则第 2 层会被拖进实时线程语义）。`yeban-engine` 若需要自己的运行时镜像，应**复制**而非依赖本 crate |
| **N5** | **UI 边界的密钥形态校验** | app / MCP 线 | 类型层只能保证"`SecretRef` 是一个字符串"。若用户把真实密钥当条目名粘进来，需要在输入边界加启发式校验（PEM 块、长随机串）并提示"这里要填的是条目名" |
| **N6** | **Windows ACL 权限判据** | 人类 / 集成者 | Windows 没有 POSIX 权限位；`0600` 判据在非 Unix **如实 SKIP**。若要在 Windows 上等价保证，需要 ACL 判据（新依赖或 `icacls` 外部调用） |
| **N7** | `schemas/**` 无需变更 | — | 本线**没有**改动任何契约：第 2 层不落盘，第 3 层不落工程容器，`project.schema.json` / `ops.schema.json` / `mcp-tools.schema.json` 一字未动。若后续要把"本机配置"暴露给 MCP 工具，那需要**新增**契约条目，属那条线的 needs |

---

## 12. 残余风险与未验证项（不假装绿）

| 项 | 状态 |
| :--- | :--- |
| Windows 平台编译与测试 | **本机未验证**（无 Windows），**并且已经真实红过一次**（run `37268168161`，见 §9.2）。首次红的根因是**夹具写死 macOS 路径**，已修并新增判据 ⑯ 显式钉住平台语义。**这次红同时提供了 Windows 侧的正向证据**：同一次运行里 `model_isolation` 的 12 条判据（键集合 / 递归键路径 / 逐字节样本 / 会话态 / 容器 / 密钥引用 / 后端错误 / BTreeSet 顺序 / forbid(unsafe) / 三层模块）在 Windows 上**全绿**，Windows 腿的 `clippy -D warnings` 也通过 ⇒ 平台分支（`#[cfg(unix)]` / `#[cfg(not(unix))]`）与跨平台字节稳定性**已被真机验证过**，只剩修复后的复跑 |
| 真实 keychain 读写 | **刻意未实现**（N1），只固定边界与失败语义 |
| 密码学擦除 | **刻意未实现**（N2），只有 best-effort |
| 权限位在其它 Unix（Linux CI）上的行为 | 本机（macOS）实测绿；Linux 行为由 CI 的 ubuntu 腿判定 —— `OpenOptions::mode` + `set_permissions` 都是 POSIX 语义，理论上一致 |
| `writer_version` 导致字节样本合法变红 | 已在失败信息里写明判别步骤（§3.2） |
| 判据 ⑪ 的"假密钥"探针 | 探针是**合成字符串**，不是真实凭据；它证明的是"material 没有落盘路径"，不是"某个真实 key 没被写" |
