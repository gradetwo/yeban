# `ui-mcp-dryrun-ime` 工作线台账 —— `dryRun`（D48）与 IME 合成态

> 工作树 `.worktrees/ui-mcp-dryrun-ime`（分支 `line/ui-mcp-dryrun-ime`，基线 `b7a45ae`）。
> 裁决：ADR-0001 **D48**（`yeban-ui-mcp` 的控制方法增加 **`dryRun`** 与 **IME 状态位**，
> 并与领域侧 `yeban_*` 工具**对齐**：同一个词必须同一个意思）。
> ⚠ 本工作线的基线是 `b7a45ae`，而 D48 的**正文**是集成者在后续提交 `ea1985a`
> （`docs/adr/ADR-0001-…md:502-505`，本线期间 `main` 已前进）写进 ADR 的；
> 本线按纪律**不改** `docs/adr/**`，因此本工作树里查不到 D48 的正文 —— 裁决内容与
> 派发给我的一致（上面那段就是逐字复述）。
> 本文件是本线的**唯一**台账；`docs/ledger/feature-alignment.md` 等共享文件按纪律未改动。

---

## 1. 方法 × 参数表（14 条方法，`dryRun` 只给会改状态的 7 条）

| # | 方法 | scope | 参数（✅ = 本线新增 `dryRun`） | 会改状态 | `dryRun` |
| ---: | :--- | :--- | :--- | :---: | :---: |
| 1 | `ui/methods` | `ui:read` | — | 否 | — |
| 2 | `ui/tree` | `ui:read` | `prefix?` / `source?` / `dynamicOnly?` | 否 | — |
| 3 | `ui/node` | `ui:read` | `elementId` | 否 | — |
| 4 | `ui/property` | `ui:read` | `elementId` / `name`（含**虚拟属性** `isComposing`） | 否 | — |
| 5 | `ui/dynamic_regions` | `ui:read` | — | 否 | — |
| 6 | `ui/screenshot` | `ui:screenshot` | `maskDynamic?` / `maxBytes?` | 否 | — |
| 7 | `ui/coverage` | `ui:read` | `ids` | 否 | — |
| 8 | `ui/dispatch_pointer_down` | `ui:inject` | `elementId` / `xOffset` / `yOffset` / `button` ✅ | 是 | ✅ |
| 9 | `ui/dispatch_pointer_move` | `ui:inject` | `x` / `y` ✅ | 是 | ✅ |
| 10 | `ui/dispatch_pointer_up` | `ui:inject` | `button` ✅ | 是 | ✅ |
| 11 | `ui/dispatch_key_press` | `ui:inject` | `keyCode` ✅ | 是 | ✅ |
| 12 | `ui/switch_main_view` | `app:admin` | `view` ✅ | 是 | ✅ |
| 13 | `ui/force_save` | `app:save` | ✅（原先无参数） | 是 | ✅ |
| 14 | `ui/reload_engine` | `app:reload-engine` | ✅（原先无参数） | 是 | ✅ |

**规则是可判定的**：`mutating == true` ⟺ 声明了 `dryRun`
（判据 `every_mutating_method_declares_dry_run_and_no_read_only_method_does`，
`crates/yeban-ui-mcp/src/methods.rs:785`）。只读方法收到 `dryRun` 会**响亮拒绝**
（`-32602`，M9 的"未知参数一律拒绝"口径），不静默忽略。

**为什么没有新增方法**：`ui/*` 的方法集是 `scripts/gates/check_feature_alignment.py`
与三方对齐矩阵的点名对象（"每一条 `ui/*` 方法都必须在本表里"），而
`docs/ledger/feature-alignment.md` 属**禁改**文件 ⇒ 第 15 条方法会让 `light` 门禁立刻变红。
因此 IME 状态位挂在**既有的只读方法** `ui/property` 上（见 §3）。

**方法数不变**：`METHOD_COUNT` 仍是 14（`crates/yeban-ui-mcp/src/lib.rs` 的判据
`crate_level_contract_holds` 钉着 `assert_eq!(METHOD_COUNT, 14)`）。

---

## 2. `dryRun` 的对齐证据（字段名 / 默认值 / 位置，逐条给了出处行号）

### 2.1 参数名与响应旗标：**再导出同一个常量**（不是照抄字面量）

| 本线 | 领域侧（唯一事实源） |
| :--- | :--- |
| `crates/yeban-ui-mcp/src/dry_run.rs:50` `pub use yeban_mcp::tools::DRY_RUN_PARAM;` | `crates/yeban-mcp/src/tools.rs:37` `pub const DRY_RUN_PARAM: &str = "dryRun";` |
| `crates/yeban-ui-mcp/src/dry_run.rs:53` `pub use yeban_mcp::dispatch::DRY_RUN_FLAG;` | `crates/yeban-mcp/src/dispatch.rs:58` `pub const DRY_RUN_FLAG: &str = "dryRun";` |

**为什么是再导出**：照抄一份 `"dryRun"` 字面量今天也能绿，但"领域侧改名"的那一天它**不会**变红。
再导出让"同一个词"由**编译器**保证（同一个 `const`），而不是靠注释约定。

### 2.2 共享字段名（`stateUnchanged` / `wouldChangeState` / `requiredScope` / `arguments` / `preview`）

本线定义的常量在 `crates/yeban-ui-mcp/src/dry_run.rs:56-70`；它们的**权威出处**是领域侧
`dry_run_result`（`crates/yeban-mcp/src/dispatch.rs:389-417`）：

| 字段 | 领域侧行号 | 本线常量 |
| :--- | :--- | :--- |
| `dryRun` | `dispatch.rs:395` | `DRY_RUN_FLAG`（再导出） |
| `stateUnchanged` | `dispatch.rs:414` | `STATE_UNCHANGED` |
| `wouldChangeState` | `dispatch.rs:402-405` | `WOULD_CHANGE_STATE` |
| `requiredScope` | `dispatch.rs:406-409` | `REQUIRED_SCOPE` |
| `arguments` | `dispatch.rs:410-413` | `ARGUMENTS` |
| `preview` | `dispatch.rs:415` | `PREVIEW` |

**判据（不是人眼比对）**：`payload_carries_every_domain_field_name_verbatim`
（`crates/yeban-ui-mcp/src/dry_run.rs:190`）拿**真实分发管线**产出的 dryRun 载荷
（`yeban_mcp::samples::dry_run_response_sample()`，`crates/yeban-mcp/src/samples.rs:269-313`，
走 `Dispatcher::handle_line` → `dry_run_result`）逐个断言"这个字段名在里面"。
改名注入（`stateUnchanged` → `state_unchanged`）让它**变红**（见 §5 注入 C）。

### 2.3 默认值 `false`

| 本线 | 领域侧 |
| :--- | :--- |
| `dry_run::requested`（`crates/yeban-ui-mcp/src/dry_run.rs:78-85`）：`…and_then(Value::as_bool).unwrap_or(false)` | `ToolCall::is_dry_run`（`crates/yeban-mcp/src/tools.rs:769-776`）同一写法 |

**判据**：`dry_run_defaults_to_false_exactly_like_the_domain`
（`crates/yeban-ui-mcp/src/dry_run.rs:151`）对**四种实参形状**
（无 `dryRun` / `false` / `true` / 别的参数）**逐形状**比较本线与**真** `yeban_mcp::tools::ToolCall::is_dry_run()`，
再加一条 `assert_eq!(DRY_RUN_PARAM, DOMAIN_PARAM)`。
领域侧的"默认 false"另有自己的判据（`crates/yeban-mcp/src/tools.rs:1165`）。

### 2.4 管线位置：与领域侧**同一顺序**

领域侧：鉴权 → scope → 解析/校验 → **dryRun 短路** → 幂等 → 执行
（`crates/yeban-mcp/src/dispatch.rs:300-360`，短路在 `:325-334`）。
本线：`crates/yeban-ui-mcp/src/service.rs:274-300` 的 `execute`（`dryRun` 在每个 mutating
分支里排在 `*_impl` **之前**返回），管线顺序写在该文件的模块头。

### 2.5 三处**有意**的不同（写在代码里，免得读者以为漏了）

1. **身份字段**：领域侧 `tool` / `specId`（一个工具一个规范 ID）↔ 本线 `method` / `specIds`
   （一条方法可以挂多个规范 ID，例如 `ui/screenshot` 挂 4 个）。
2. **不加 `sideEffect`**：领域侧的 `SideEffect` 是闭合三值
   （`read-only` / `project-state` / `disk`，`crates/yeban-mcp/src/tools.rs:250-276`），
   **没有"界面状态"这一档**（与 D29 的勘误同因）。硬塞 `project-state` 是谎报，
   因此本线只报共享的 `wouldChangeState`（语义 = `side_effect.is_side_effecting()`）。
3. **不加 `idempotencyKey`**：UI 控制面没有幂等缓存（也没有一次性令牌），
   D48 只要求 `dryRun`。

---

## 3. IME 状态位（`[UI-A11Y-002]`）：真实载体与两个方向的读数

### 3.1 真实载体（**不是**影子变量）

| 环节 | 位置 |
| :--- | :--- |
| 状态机本体（**唯一**的那一份） | `crates/yeban-app/src/input.rs:290-334` 的 `InputContext`（`composing` 字段在 `:292`，`is_composing()` 在 `:313`，`begin_composition` `:327`，`end_composition` `:332`，`set_focus` 的"焦点离开文本域自动结束合成" `:319-324`） |
| 判定防护的地方（同一字段） | `crates/yeban-app/src/input.rs:346-364` 的 `resolve`：文本域 + 合成态 ⇒ `Resolution::ConsumedByIme` |
| 执行面持有的句柄 | `crates/yeban-app/src/live_surface.rs:286` 的 `input: Rc<RefCell<InputContext>>`（与 `window.clone_strong()` 同构：**共享**，不是复制） |
| 观测位 | `UiSurface::ime_state`（`crates/yeban-ui-mcp/src/surface.rs:141`）← 实现在 `crates/yeban-app/src/live_surface.rs:603`，读的正是上面那个 `Rc` |
| 生产驱动点 | `LiveUi::input_context()`（`crates/yeban-app/src/live_surface.rs:792`）—— Slint 平台 IME 事件调它；**判据也走它** |
| 线格式名字映射 | 规范 §7.2 的 `is_composing` ↔ 线格式 `isComposing`（`crates/yeban-ui-mcp/src/ime.rs:51,54`，映射由 `camel_case` **算出来**，判据 `ime_field_is_the_wire_form_of_the_spec_field` `ime.rs:145`） |

**同一个对象三处共用**：`ime_state`（观测）、`preview_effect`（"这一键会被怎么处置"）、
`LiveUi::input_context`（驱动）。任何一处换成影子字段/常量，都会让判据 ⑤/⑥ 之一变红。

### 3.2 怎么观测（走**既有**只读方法，未新增方法）

```text
ui/property {"elementId": "transport-bpm-field", "name": "isComposing"}
  -> {"id":"transport-bpm-field","name":"isComposing","value":false,
      "focus":"main-canvas","specId":"UI-A11Y-002"}
```

- `elementId` 仍必须**真的在运行树里**（§12.2；找不到就是 `-32006`）——
  `crates/yeban-ui-mcp/src/service.rs:511-535`；
- `value` 是**原生 JSON 布尔**（载体是进程内的 IME 状态机，不是 Slint 响应式属性）；
- 执行面没有 IME 状态机时如实报 `-32005`（`data.kind = not-implemented`），**不编 `false`**；
- 可发现性：`ui/property` 的 `name` 参数描述里点名 `isComposing`
  （`crates/yeban-ui-mcp/src/methods.rs`，判据 `property_name_description_lists_every_virtual_property` `methods.rs:849`），
  且 `ui/methods` 顶层给出 `imeProperty`（`methods.rs:635`）。

### 3.3 两种状态的读数（本机**实测**，假执行面）

```text
非合成（画布聚焦）   -> value=false, focus=main-canvas
文本域聚焦 + begin   -> value=true,  focus=text-input      （与上一行**不同**；两方向都断言）
焦点回到画布         -> value=false                          （set_focus 的既有语义）
```

### 3.4 本线**刻意不做**的一件事（写给读者，免得当成漏掉）

`dispatch_key_press` 的**真调用**分发行为**没有改**：§7.2 的"彻底拦截冒泡分发"约束的是
**Slint 控件层**（真实用户的按键），而 UI 测试端口注入的按键正是用来**验证**那条防护的
（注入被吞掉就测不了了）。注入路径上 IME 状态的正确用法是**先问后做**：

```text
ui/dispatch_key_press {"keyCode":"Space","dryRun":true}
  -> preview.effect = {"keyCode":"Space","isComposing":true,"focus":"text-input",
                       "resolution":"consumed-by-ime"}     # 合成态
```

即"如果我按 Space，会不会被你吞掉" —— 这个问题由**真的** `InputContext::resolve` 回答。

**处置词表只有一份**（`crates/yeban-ui-mcp/src/ime.rs` 的 `RESOLUTIONS`）：
`consumed-by-ime` / `action` / `pass-through`。真执行面用 `InputContext::resolve` 的三档填它，
零 Slint 假面**只知道两档**（它没有 `[UI-A11Y-001]` 的扫描码表 ⇒ 非合成态一律
`pass-through`，**不冒充** `action`）—— 这个"知道得少"写在 `testing.rs` 的注释里，
取值本身两侧共用常量，因此同一个键不会在两个执行面上有两个名字
（判据 `key_resolution_vocabulary_is_closed_and_unique`，`ime.rs`）。
这条是**本线自己的一次返工**：第一版假面报 `"dispatched"`、真执行面报 `"action"`，
同一字段两个词 —— 正是"同一个词必须同一个意思"要禁止的形状。

---

## 4. 判据清单（本机**真跑**的部分）

| # | 判据 | 文件:行 | 钉住什么 |
| ---: | :--- | :--- | :--- |
| ① | `dry_run_leaves_the_state_untouched_for_every_mutating_method` | `service.rs:1819` | 7 条方法逐一：快照**逐字段**相同 + 执行面调用日志为空 + `ui/tree` 线上 JSON 逐字节相同 + 响应自证 |
| ② | `the_same_calls_without_dry_run_really_change_the_state` | `service.rs:1888` | 同一张表、同一个快照函数：`dryRun` 缺省时**确实**改状态（参数不是摆设） |
| ③ | `dry_run_preview_predicts_exactly_what_the_real_call_does` | `service.rs:1948` | 预览的实参 == 真调用交给执行面的实参；预览的 `effect` == 真做之后的状态读数 |
| ④ | `payload_carries_every_domain_field_name_verbatim` | `dry_run.rs:190` | 共享字段名与**真管线**产出的 dryRun 载荷逐字一致 |
| ④' | `dry_run_defaults_to_false_exactly_like_the_domain` | `dry_run.rs:151` | 默认值与**真** `ToolCall::is_dry_run()` 逐形状一致 |
| ⑤ | `ime_state_distinguishes_composing_from_not_composing` | `service.rs:2085` | 合成中 / 非合成中两个方向的观测值不同 |
| ⑥ | `ime_state_reads_the_real_state_machine_not_a_shadow_variable` | `service.rs:2012` | 观测位与"按键会被怎么处置"读**同一份**状态；焦点离开自动结束合成 |
| ⑦ | `every_mutating_method_declares_dry_run_and_no_read_only_method_does` | `methods.rs:785` | 适用范围 + `ui/methods` 的 `dryRunSupported` / `dryRunParam` 可发现性 |
| ⑦' | `methods_discovery_announces_the_ime_property_and_dry_run` | `service.rs:2334` | `ui/methods` **运行态**载荷自报 `dryRunParam` / `imeProperty` |
| ⑧ | `dry_run_does_not_leak_into_the_real_response_shapes` | `service.rs:2292` | `dryRun` 没有污染真调用的响应形状（既有 14 条的行为不变） |
| ⑨ | `dry_run_keeps_every_existing_error_code` | `service.rs:2123` | `-32601` / `-32602` / `-32003` / `-32006` / `-32009` / `-32005`（D25：不发明新码） |
| ⑩ | `dry_run_consumes_no_scope_and_no_token` | `service.rs:2238` | 同一 token 先 dryRun 后真调用都成功；授权集合一位不变 |
| — | `requested_is_total_and_the_snapshot_excludes_the_switch_itself` | `dry_run.rs:241` | 只有字面 `true` 算 dryRun；实参快照排除 `dryRun` 与 `_` 保留键 |
| — | `ime_field_is_the_wire_form_of_the_spec_field` / `camel_case_covers_the_shapes_we_use` / `focus_names_cover_every_variant` | `ime.rs:145,158,168` | 线格式名与规范名的对应关系是**算**出来的 |
| — | `property_name_description_lists_every_virtual_property` | `methods.rs:849` | 虚拟属性必须在 `ui/property` 的参数描述里被点名 |
| — | `spec_event_injection_methods_match_section_12_4` | `methods.rs`（已改） | §12.4 的四个注入方法参数逐一对应（`dryRun` 排最后且可选） |

**假面保真度（诚实边界）**：`crates/yeban-ui-mcp/src/testing.rs` 的 `FakeSurface` 现在会
"真的"改三个可观测状态（`arrangementView` / `saveEpoch` / `engineGeneration`）并持有 IME 状态，
因此判据 ①②③ 不是"对调用日志断言"。它**不**模拟 `LivePort` 的 `MissingGeometry` 前置
（`crates/yeban-ui-test-port/src/render.rs:513-521`），因为既有判据用无几何的
`track-0-fader` 钉 `visible: null` 语义（`service.rs` 的
`unknown_semantic_id_is_an_explicit_error_not_an_empty_success`）。`dryRun` 的只读前置校验
（`service.rs:551` 的 `ensure_geometry`）照**真执行面**的口径报 `-32009`。

### 4.1 CI 侧判据（含 Slint ⇒ 本机跑不了，见 §6）

| # | 判据 | 文件:行 | 钉住什么 |
| ---: | :--- | :--- | :--- |
| 13 | `dry_run_leaves_the_live_window_and_the_disk_untouched` | `crates/yeban-app/tests/live_ui_mcp.rs:1038` | 真实窗口上：`ui/tree` 逐字节不变、磁盘无文件、引擎代数不被 dryRun 推进（真调用拿到 `generation=1`）、预览与真调用的回执**逐字段一致** |
| 14 | `ime_composition_is_observable_on_the_live_window` | `live_ui_mcp.rs:1171` | 真实窗口上 IME 位 false → true → false；合成态下 dryRun 的 `Space` 报 `consumed-by-ime` |
| 15 | `dry_run_reports_the_same_failure_as_the_real_save` | `live_ui_mcp.rs:1238` | 未配置落点时 dryRun 与真调用**同码同话同 data**（`-32005`） |

---

## 5. 注入 → 变红 → 还原（本机**原始记录**）

复跑命令（本机探针，见 §6.1）：

```bash
bash /tmp/uimcp-probe2/build.sh                                     # 六步全绿（含本机 clippy）
CARGO_MANIFEST_DIR=$PWD/crates/yeban-ui-mcp /tmp/uimcp-probe2/uimcp_tests
```

| 注入 | 改了什么 | 期望红的判据 | 实测 |
| :--- | :--- | :--- | :--- |
| **A** | 删掉 `ui/force_save` 分支里的 `if dry_run { return self.preview(..) }`（dryRun 也执行） | ①③ | `85 passed; **2 failed**`：`dry_run_leaves_the_state_untouched_…`、`dry_run_preview_predicts_…` ✅ |
| **B** | 把假面的 `ime_state` 接成常量（`composing: false, focus: MainCanvas`） | ⑤⑥ | `85 passed; **2 failed**`：`ime_state_distinguishes_…`、`ime_state_reads_the_real_state_machine_…` ✅ |
| **C** | 把 `STATE_UNCHANGED` 改成 `"state_unchanged"` | ④ | `86 passed; **1 failed**`：`payload_carries_every_domain_field_name_verbatim` ✅ |
| **D** | 让预览瞎描述（`saveEpoch` 恒 42） | ③ | `86 passed; **1 failed**`：`dry_run_preview_predicts_…` ✅ |
| **E** | 把 `dry_run::requested` 的默认值改成 `true` | ②④'⑧⑩… | `74 passed; **13 failed**`：`dry_run_defaults_to_false_exactly_like_the_domain`、`requested_is_total_…`、`the_same_calls_without_dry_run_…`、`dry_run_does_not_leak_…`、`dry_run_consumes_no_scope_and_no_token`、`dry_run_preview_predicts_…`、`admin_reports_are_attached_…`、`notifications_are_silent_…`、`in_process_permission_is_a_second_independent_gate`、`event_injection_is_hard_denied_…`、`switch_main_view_rejects_a_view_name_outside_the_whitelist`、`live::tests::production_hard_denies_injection_before_the_surface_is_touched`、`transport::http::tests::ui_inject_is_still_hard_denied_in_production_over_http` ✅ |

**五条注入每次都还原并复跑**，还原后的读数是 `87 passed; 0 failed`（见 §6.1）。
注入 C 只有 ④ 变红是**设计如此**：本线自己改名时内部仍然自洽（自己产、自己读），
唯一能发现"与领域侧不同名"的就是那条**跨 crate 比对**的判据 —— 这正是它存在的原因。

### 5.1 CI 第 1 轮的判决与处置（**run 37254896937**，本线第 1 次推送的 tip）

判决：**红**（`rust (yeban-ui-mcp)` 的 clippy 腿；`rust (yeban-app)` 的 `live_ui_mcp` 目标）。
三条读数与处置（都不是"重跑看看"）：

| # | CI 报的 | 根因 | 处置 |
| ---: | :--- | :--- | :--- |
| 1 | `clippy::cmp_owned` @ `service.rs:2350`："this creates an owned instance just for comparison" | `entry["dryRunSupported"] == Value::from(true)` —— 拿 `Value` 与"为比较专门造的" `Value` 比 | 改成 `.as_bool() == Some(true)`（同一断言，不减弱） |
| 2 | `clippy::needless_borrow` @ `testing.rs:98`："creates a reference which is immediately dereferenced" | `fnv1a64(&self.image.pixels())` —— `pixels()` 已经返回 `&[u8]` | 去掉 `&`。**并把这条重新注入验证本机 clippy 腿**（§6.1，报错逐字相同） |
| 3 | `live_ui_mcp.rs:1211` 判据 14 失败：`ui/dispatch_key_press {"dryRun":true}` 拿到 **403 `forbidden-in-production`** | 判据用 `Permission::ReadOnly` 装配 ⇒ `RunMode::Production` ⇒ `ui:inject` 被**生产硬禁**。**这不是缺陷，是"`dryRun` 不绕过授权"的直接后果**（本机判据 ⑨ 的第 ④ 条钉着同一件事） | 判据改用 `Permission::Interactive`（测试模式），并在判据注释里写明这次的 403 就是那条性质的**实测证据** |

**同一轮 CI 顺带证实了三件事**（比"绿"更值钱，存档）：
1. `crates/yeban-app` 侧的新接线**真的编译过**（`Rc<RefCell<InputContext>>`、`ime_state`、
   `preview_effect`、`physical_key_of` / `key_resolution`）——本机做不到这一步；
2. 判据 13（`dry_run_leaves_the_live_window_and_the_disk_untouched`）**通过** ⇒ 真实窗口上
   `ui/tree` 逐字节不变、磁盘无文件、引擎代数只被真调用推进、预览与真调用逐字段一致，
   其中包括 `preview.effect.resolution == "action"`（真 `InputContext::resolve` 对
   `Space` + 画布焦点的回答）；
3. 判据 14 失败在第 1211 行（**按键 dryRun**），而它前面的两次 `ui/property isComposing`
   读数与 `transport-bpm-field` 的寻址**都已经过了** ⇒ IME 观测位在真窗口上可用、
   `transport-bpm-field` 确实在运行时树里。

---

## 6. 本机真跑 vs CI（严格区分）

### 6.1 本机**真跑**过的（可复跑）

| 命令 | 读数 |
| :--- | :--- |
| `bash scripts/gates/run-gates.sh light` | **exit 0**；`fmt` / 14 条机械红线守卫 / 规范 ID 审计（40 个 ID 全部存在于规范）/ ID 字典 / 决策清单 / 门禁状态表 / 阶段状态表 / **三方对齐矩阵**（66 行、10 工具、14 方法全部点名）/ 文档链接 / 依赖许可清单（679 行） |
| `cargo fmt --all`（`scripts/dev/local-env.sh` 提供的 `1.99.0` 工具链） | 只改动了本线 7 个文件，无无关格式漂移 |
| `/tmp/uimcp-probe2/build.sh`（**零 Slint 探针**） | `BUILD-EXIT=0`：六个目标全过（`-D warnings -D rust_2018_idioms`，含 `#![deny(missing_docs)]`）——(1) lib feature OFF、(2) lib feature ON、(3) 单元判据、(4) `tests/contract.rs`、(5) `example export_ui_samples`、**(6) 本机 clippy**（`clippy-driver` 直接按 rustc 参数跑 lib + 单元判据两个目标） |
| 本机 clippy 腿（第 1 轮 CI 之后新增，见 §5.1） | `clippy-driver --version` -> `clippy 0.1.99 (b940084d7e 2026-09-28)`。**为什么能在本机跑**：根 `Cargo.toml` 的 `[workspace.lints.clippy]` 是**空的** ⇒ CI 的 `cargo clippy` 用的是 clippy **默认** lint 集，而 `clippy-driver` 以 rustc 参数直接调用时用的是同一套。**实测对账**：把第 1 轮 CI 抓到的 `clippy::needless_borrow`（`testing.rs:98`）重新注入，本机这条腿给出**逐字相同**的错误（同消息、同行列）⇒ 它有牙。**不覆盖**：`--cap-lints`、依赖的 lint、`yeban-app` 的 clippy 目标 |
| `… /tmp/uimcp-probe2/uimcp_tests` | `**87 passed; 0 failed**`（本线在 `yeban-ui-mcp` 里新增 **19** 条 `#[test]`：`methods.rs` +2 / `service.rs` +9 / `dry_run.rs` +3 / `ime.rs` +4 / `surface.rs` +1；另有 3 条在 `yeban-app` 侧，见 §4.1） |
| `… /tmp/uimcp-probe2/contract_tests` | `**5 passed; 0 failed**` |
| `cargo build -p yeban-mcp --features mcp-http`（**在主仓**执行，见下） | 供探针链接的真 rlib（`target/debug/libyeban_mcp.rlib`；9.3 s，其中 `yeban-model`/`yeban-decode`/`yeban-render`/`yeban-mcp` 四个 crate 真的编译了，其余命中缓存） |

**探针怎么搭的**（与 `docs/ledger/ui-mcp-notes.md` §4.1 的五步探针同一套，两处不同）：

1. 源码指向**本工作树**（不是主仓 / 别的 worktree）；
2. `yeban-mcp` 的 rlib 复用**主仓**的 `target/`。**诚实边界**：`main` 在本线工作期间
   已前进到 `ea1985a`（别的线合入），因此那个 rlib 是从**比本线基线新**的
   `yeban-mcp` 源码构建的，不是从本工作树的 `b7a45ae` 版本构建的。判据依赖的三个东西
   （`tools::DRY_RUN_PARAM`、`dispatch::DRY_RUN_FLAG`、`samples::dry_run_response_sample`）
   在本工作树的基线源码里**逐条读过并写进 §2**，两侧都存在；探针因此证明的是"与本线读到的
   领域侧语义一致"，而"与**基线**版本的领域侧逐字节一致"由 CI 在基线源码上重跑同一批判据。
   （本工作树的 `target/` 是空的；在本工作树里重编 `yeban-mcp` 会拉起 `yeban-render` 的
   `rayon`/`hound`/`midly`，属本机纪律里该省的高耗编译。）

`yeban-ui-test-port` 的 Slint 依赖由探针的 `stub_lib.rs` **绕过**：它 `#[path]` 引入该 crate
的**零 Slint 子集**（`image`/`tree`/`port`/`mask`/`ssim`/`png`/`golden`）并只补一个
`render::fnv1a64` 的 shim。诚实边界与 `ui-mcp-notes.md` §4.1 的 ⚠ 逐字相同：
"两份实现是否真的逐位一致"由 CI 用**真** `render.rs` 判。

### 6.2 本机**没有跑**的（交给 CI）

- `cargo clippy -p yeban-ui-mcp --all-targets -- -D warnings` 的 **cargo 那一层**
  （它要编译 Slint；`run-gates.sh crate yeban-ui-mcp` 见 `slint` 会 `SKIP`）。
  ⚠ 但 `yeban-ui-mcp` 的 **clippy lint 本身**已由 §6.1 的第 (6) 步在本机覆盖
  （`clippy-driver` + 默认 lint 集，与 CI 同口径且经注入对账）；未覆盖的只剩
  `yeban-app` 的 clippy 目标与 `--cap-lints` 之类 cargo 侧设置；
- `cargo test -p yeban-ui-mcp`（同一原因）与 `--features ui-mcp-http` 的组合；
- **任何**编译到 `crates/yeban-app/**` 的命令（Slint + Tier-1 平台）⇒ §4.1 的 3 条 CI 判据
  与 `crates/yeban-app/src/live_surface.rs` 的接线**在本机只是"读过 + 手工核对"**
  （`cargo fmt` 会解析该文件，但**不做类型检查**）。这是本线最大的一块诚实边界：
  **app 侧接线与那 3 条判据的第一次真编译发生在 CI**。
- `cargo deny` / `--workspace` 全量 / 基准 / 模糊测试。

### 6.3 判决

| 轮次 | run id | tip | 判决 | 读数 |
| :--- | ---: | :--- | :--- | :--- |
| 1 | [37254896937](https://github.com/gradetwo/yeban/actions/runs/37254896937) | 本线第 1 次推送 | **红** | `rust (yeban-ui-mcp)` clippy 2 条 + `rust (yeban-app)` 的 `live_ui_mcp` 1 条（§5.1）；`checks` / `deny` / `plan` / `lockfile` 绿 |
| 2 | [37255336054](https://github.com/gradetwo/yeban/actions/runs/37255336054) | `e8ce77c`（§5.1 的三处修复） | **绿** | `✓ rust (yeban-app)` 4m5s / `✓ rust (yeban-ui-mcp)` 2m12s / `✓ checks` / `✓ deny` / `✓ lockfile` / `✓ plan`；`windows` 与 `rust (workspace 全量)` 不在受影响集合（0s skipped）。读回命令 `bash scripts/dev/ci-verdict.sh line/ui-mcp-dryrun-ime`（退出码 0；`--list` 同一行 `success`） |
| 3 | 见下一段 | **只改本文档**的补记（把上面那行判决写回台账） | 未读回 | 该提交不含任何源码/判据改动，但按 L32"未读回的判决记为 pending"，它的判决仍以 CI 为准 |

`bash scripts/dev/ci-verdict.sh line/ui-mcp-dryrun-ime` 读回；**未读回之前一律记 `pending`**。
本文件的读数只覆盖 §6.1，任何"CI 通过"的说法都必须带上 run id。

---

## 7. 未实现项 / needs

| # | 事项 | 处置 |
| ---: | :--- | :--- |
| N1 | **Slint 平台的 IME 事件尚未接到 `InputContext`**（`.slint` 侧没有 `is_composing` 信号；`transport.slint:139-140` 自己写着"先用 `role=text-input` 占位，真正的 Slint `TextInput` 随模型线落地"）。载体（`Rc<RefCell<InputContext>>`）与驱动点（`LiveUi::input_context`）**已就位**，只差事件源 | 记为 **pending**（属 `yeban-app` 的 Slint 接线切片，不是本线的裁决范围）。在它落地之前，`ui/property` 的 `isComposing` 只会反映**外部驱动**（判据/未来的事件源）写入的状态 —— 这正是规格 §7.2 第 1 条"Slint 控件层必须严格监听"要补的那一半 |
| N2 | `ui/dispatch_key_press` 的**真调用**不拦截合成态（见 §3.4 的理由） | 这是**设计决定**，不是缺口；若将来要"注入也走防护"，需要先裁决"注入路径的语义是模拟用户还是模拟平台事件"（属 `UI-MCP-001` 的签名扩张，需人类裁决） |
| N3 | 领域侧 `dryRun` 的**读取**用户（`yeban_propose_section` 等）尚未把 UI 控制面纳入；双 MCP 协同闭环（`ROAD-M4-008`）仍缺依赖边 | 与 `feature-alignment.md` 的"错位 8"同一件事；需要 `yeban-app` 引入非默认 feature 的 `yeban-mcp` 边（`.github/**` 属集成者） |
| N4 | 本线**没有**改 `schemas/**`：`ui/*` 的线上契约是 JSON-RPC 2.0 的自由结果对象，没有 schema（见 `crates/yeban-ui-mcp/src/samples.rs` 的 `methods_document`）。若今后要把它纳入契约，需先裁决（当前 `schemas/mcp-tools.schema.json` 的根是 `oneOf(ToolCall, ToolResponse)`，把 UI 结果包成 `ToolResponse` 属**类型错误**） | 无动作；`dryRun` 的字段名对齐已由判据 ④ 机械保证 |
| N5 | `docs/ledger/feature-alignment.md` 的"错位 8 / 错位 9"两行仍写着"人类决策中" | 按纪律**禁改**该文件；本线的结论需由**集成者**在有权限时更新（`ui/*` 的 `dryRun` 已落地、IME 状态位已可观测） |

---

## 8. 改动文件与净行数

见提交信息（`git show --stat`）。本线的**零新增依赖**结论可由
`git diff main...HEAD -- Cargo.toml Cargo.lock` 为空核验：`dry_run.rs` 用的
`yeban_mcp::{tools,dispatch}` 与 `serde_json` 都已在 `crates/yeban-ui-mcp/Cargo.toml` 里；
`ime.rs` 只用 `std`。
