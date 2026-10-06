# `app-projection` 工作线台账 —— 时间码按工程拍号 + Slint→`InputContext` 的 IME 事件源

> 工作树 `.worktrees/app-projection`（分支 `line/app-projection`，基线 `main` `c53b7f3`）。
> 本线的出题方给了两条**待核实**的判断（时间码可能写死 4/4、Slint 侧 IME 事件源未接）。
> 按纪律**先核实再动手**：下面 §1 是逐条复核的结论与证据行号，其中一条的**理由**是过时的。

---

## 1. 核实结论（先照实复核，不照抄派发结论）

### 1.1 时间码：**确实写死 4/4**（判断成立），但"因为 `TimeSignature` 没被投影"这个理由**过时**

实测命令：`grep -n 'time_signature' crates/yeban-app/src/*.rs`（改动前）。

| 事实 | 行号（改动前） | 原文 |
| :--- | :--- | :--- |
| 时间码的拍号被刻成常量 | `crates/yeban-app/src/host.rs:205` | `const BEATS_PER_BAR: u64 = 4;` |
| 格式化函数只吃 tick | `crates/yeban-app/src/host.rs:212-218` | `timecode_for_ticks(ticks: u64) -> String`（`ticks_per_bar = PPQ * BEATS_PER_BAR`） |
| 唯一调用点 | `crates/yeban-app/src/host.rs:233` | `ui.set_timecode(timecode_for_ticks(reading.position_ticks).into())` |
| 那句注释 | `crates/yeban-app/src/host.rs:203-204` | "`TimeSignature` 已经在模型里，但它**没有**被投影进 `ViewState`（登记为 needs）" |

**这句注释与事实相反**：改前 `ViewState` 早就带着拍号：

| 事实 | 行号（改动前） | 原文 |
| :--- | :--- | :--- |
| 投影结构里有分子 / 分母 / 显示文本 | `crates/yeban-app/src/bridge.rs:480`、`482`、`484` | `pub time_signature_numerator: u8,` / `pub time_signature_denominator: u8,` / `pub time_signature_display: String,` |
| 投影时真的写进去 | `crates/yeban-app/src/bridge.rs:660-666` | `time_signature_numerator: project.time_signature.numerator,` … `format!("{}/{}", …)` |
| 一小节 tick 数也已投影 | `crates/yeban-app/src/bridge.rs:573`、`487` | `let bar_ticks = bar_length_ticks(project.time_signature)?;` / `pub bar_length_ticks: u64,` |

`grep -n 'time_signature' crates/yeban-app/src/host.rs` 在改动前**一条都没有** ——
所以"时间码很可能仍按写死的 4/4 格式化"这个判断是**对的**，"理由"是**过时**的。

另一处独立旁证：`docs/ledger/transport-engine-notes.md:335`（另一条线的台账）把这件债写成

> `needs`：把 `time_signature` 投影进 `ViewState`（模型字段已在，改动属投影层）

它的前半句在本线开始时**已经不成立**（拍号早在 `app-binding` 线就进了 `ViewState`），
后半句（时间码没读它）成立。本线按"needs 的**现象**成立、**归因**过时"处理，
并把结论登记在 §6 的 N5（那份台账属另一条线，本线不改）。

### 1.2 Slint 侧 IME 事件源：**确实未接**（判断成立）—— 但上游**有**信号，只缺注入面

| 事实 | 行号（改动前） | 原文 |
| :--- | :--- | :--- |
| 只有一句注释 | `crates/yeban-app/ui/transport.slint:143-144` | "参数敲入类控件: 规范 §7.2 明文要求这类控件挂 is_composing 防护。/ 这里先用 role=text-input 占位; 真正的 Slint TextInput 与数值校验随模型线落地。" |
| 占位矩形（不是真编辑面） | `crates/yeban-app/ui/transport.slint:146-190` | `Rectangle { accessible-role: text-input; … }` |
| 全仓 `TextInput` 出现次数 | `grep -rn 'TextInput' crates/yeban-app/ui/*.slint` | **0 处**（改前） |
| 载体已就位 | `crates/yeban-app/src/input.rs:290-333` | `InputContext`（`set_focus` / `begin_composition` / `end_composition`） |
| 注入面已就位 | `crates/yeban-app/src/live_surface.rs:797` | `pub fn input_context(&self) -> Rc<RefCell<InputContext>>` |
| 观测位已就位 | `crates/yeban-ui-mcp/src/ime.rs:41-42` | `ui/property {"elementId": "transport-bpm-field", "name": "isComposing"}` |

**上游核验（关键：信号是有的，只是"注入面"没有）**：

| 环节 | 事实 | 出处 |
| :--- | :--- | :--- |
| 编译期暴露给 `.slint` | `out property <string> preedit-text;`（注释原文 "Internal, undocumented property, only exposed for IME"） | `i-slint-compiler-1.18.1/builtin_elements.rs:2323` |
| 平台 → 运行时 | `WinitWindowEvent::Ime(winit::event::Ime::Preedit(..))` → `InternalKeyEvent { event_type: KeyEventType::UpdateComposition, preedit_text, .. }` | `i-slint-backend-winit-1.18.1/winitwindowadapter.rs:1429-1437` |
| 运行时写属性 | `self.preedit_text.set(event.preedit_text.clone())` | `i-slint-core-1.18.1/items/text.rs:1185` |
| 失焦清零 | `FocusEvent::FocusOut` → `self.preedit_text.set(Default::default())` | `i-slint-core-1.18.1/items/text.rs:1260` 附近 |

**没有公开注入面**（这条把判据的**上限**钉死了，见 §6 N1）：

- `slint::platform::WindowEvent` 是 `#[non_exhaustive]` 公开枚举，**没有**合成变体
  （`i-slint-core-1.18.1/platform.rs:367-470`；合成走 `#[doc(hidden)]` 的 `Internal` 变体）；
- 真正携带 preedit 的 `i_slint_core::input::InternalKeyEvent` / `KeyEventType`
  **没有被再导出**：`slint` 的 `private_unstable_api::re_exports` 只导出
  `input::{FocusEvent, FocusReason, InputEventResult, KeyEvent, KeyEventResult, KeyboardModifiers, Keys, MouseEvent, key_codes::Key, make_keys}`
  （`slint-1.18.1/private_unstable_api.rs:179-182`），`i-slint-backend-testing` 的
  `internal_tests` 只把 `InternalEvent` / `BackendMouseEvent` 提上来
  （`i-slint-backend-testing-1.18.1/internal_tests.rs:9-13`）；
- `WindowInner::process_key_input` 是 `pub(crate)`（`i-slint-core-1.18.1/window.rs:1248`）。

结论：**上游有信号 ⇒ 不许报 SKIP**；但**判据只能在 `.slint` 回调那一格注入**
（下游一格），平台级注入记 needs（§6 N1），并且这条限制被 §4 的判据 ⑥ 当场打印出来。

---

## 2. 改动（文件 × 做了什么）

| 文件 | 改动 | 规范 / 裁决 |
| :--- | :--- | :--- |
| `crates/yeban-app/src/bridge.rs` | 新增 `TimecodeGrid`（拍号 → 每拍 / 每小节 tick）与 `timecode_for_ticks(grid, ticks)`（`BBB.BB.TTT` 的**唯一实现**）；`ViewState::timecode_grid()` / `timecode_at()` | `[MODEL-AST-001]` `[MODEL-ISO-001]` ADR-0001 D28（投影层零 Slint） |
| `crates/yeban-app/src/host.rs` | 删掉 `const BEATS_PER_BAR = 4` 与写死的 `timecode_for_ticks`；`apply_view` 注入 `timecode-ticks-{beat,bar}`；`apply_transport` 用投影网格格式化；新增 `wire_input`（IME 事件源 → `InputContext`） | `[UI-A11Y-002]` `[ARCH-UI-002]` D28 |
| `crates/yeban-app/ui/app.slint` | `MainWindow` 加两个注入属性 `timecode-ticks-beat/bar`、两个回调 `ime-composition-changed` / `ime-focus-changed`，并把 `Transport` 的同名回调转发上来 | `[UI-GRID-001]` `[UI-A11Y-002]` |
| `crates/yeban-app/ui/transport.slint` | BPM 占位矩形里放**真的** `TextInput`（`bpm-input`）：`changed preedit-text` / `changed has-focus` 通知两个回调；编辑面**不挂** `accessible-id`（不进运行时树，`ui/coverage` 不受影响） | `[UI-A11Y-002]` §7.2、`[UI-TEST-001]` §12.2 |
| `crates/yeban-app/src/live_surface.rs` | 装配时 `host::wire_input(&window, Rc::clone(&input))`（与 `input_context()` / `ui/property isComposing` 共用**同一个** `Rc`）；新增 `LiveUi::ui()`（判据在真事件源上注入的驱动点） | `[UI-A11Y-002]`、D28 |
| `crates/yeban-app/src/main.rs` | 生产路径也接上事件源（`wire_input` + `N2` 裁决 (1) 的 `wire_keys`，带真 `UndoPort`）；§6 N2 的消费者缺口已闭环，残余写在那一行 | `[UI-A11Y-002]` |
| `crates/yeban-app/src/test_port_adapter.rs` | 4 条新 Tier-1 判据（时间码 ×3 + IME ×2 见 §4）；旧的 `host::timecode_for_ticks(..)` 调用改成与**投影**对账 | `[UI-TEST-001]` `[UI-A11Y-002]` |
| `crates/yeban-app/tests/live_ui_mcp.rs` | 1 条新判据（判据 14b）：从**事件源**驱动，经 `ui/property` / `ui/dispatch_key_press` 观测 | `[UI-A11Y-002]` `[UI-MCP-001]` |

**零新增依赖**：`Cargo.toml` / `Cargo.lock` 一行未改（`git diff --stat` 里没有它们）。
时间码的算术复用模型层已有的 `yeban_model::SessionRuntimeState::ticks_per_bar`
（`crates/yeban-model/src/session.rs:239-250`）与 `ticks_per_beat()`
（`session.rs:232`）—— 没有第二份"一小节多少 tick"。

---

## 3. 实测读数（活窗口，Tier-1 软件光栅化 + 真引擎）

命令（本机，复用主仓暖 target）：

```text
CARGO_TARGET_DIR=/Users/crow/work/music/yeban/target \
  bash scripts/dev/cargo-local.sh test -p yeban-app --test real_ui_tier1 --locked \
  -- --nocapture timecode
```

界面读数与投影读数**逐字相同**（`界面=` 是活窗口的 `timecode` 属性，`投影=` 是 `ViewState::timecode_at`）：

| 拍号 | tick | 界面读数 | 投影读数 | 注入的小节 tick | 注入的每拍 tick |
| :--- | ---: | :--- | :--- | ---: | ---: |
| 4/4 | 960 | `001.02.000` | `001.02.000` | 3840 | 960 |
| 4/4 | 3840 | `002.01.000` | `002.01.000` | 3840 | 960 |
| 4/4 | 8000 | `003.01.320` | `003.01.320` | 3840 | 960 |
| 3/4 | 960 | `001.02.000` | `001.02.000` | 2880 | 960 |
| 3/4 | 3840 | `002.02.000` | `002.02.000` | 2880 | 960 |
| 3/4 | 8000 | `003.03.320` | `003.03.320` | 2880 | 960 |
| 6/8 | 960 | `001.03.000` | `001.03.000` | 2880 | 480 |
| 6/8 | 3840 | `002.03.000` | `002.03.000` | 2880 | 480 |
| 6/8 | 8000 | `003.05.320` | `003.05.320` | 2880 | 480 |

**同一 tick、拍号改变（引擎位置一位不动）**：

```text
[timecode] 同一 tick 3840: 4/4=002.01.000 3/4=002.02.000 回到 4/4=002.01.000（引擎位置始终 3840）
```

**IME（事件源 → 状态机 → 行为）**：

```text
[ime] 事件源(Slint 回调) → InputContext: 合成 false→true→false 全部对得上;
      合成态下 Space=ConsumedByIme 焦点在敲入控件时 Space=PassThrough 焦点回画布后 Space=Action(PlayPause)
[ime-skip] 平台级 `Ime::Preedit` 注入在 Slint 1.18.1 上没有公开入口
      (`WindowEvent` 无合成变体; `InternalKeyEvent`/`KeyEventType` 未被再导出;
       `WindowInner::process_key_input` 是 pub(crate); 加 `i-slint-core` 依赖被本线约束禁止)
      ⇒ 判据覆盖到 `.slint` 回调这一格; 上游信号本身的接线证据 =
      ui/transport.slint 的 `changed preedit-text`（本线源码级核验），平台级注入记为 needs
```

---

## 4. 判据（派发的 ①–⑧ 逐条对应）

### 4.1 交付判据

| 派发项 | 判据（函数名） | 位置 | 断言了什么 |
| :--- | :--- | :--- | :--- |
| ① 拍号 4/4 与 3/4 下同一 tick 的读数正确 | `timecode_reads_the_projected_time_signature_for_three_signatures` | `src/test_port_adapter.rs`（Tier-1） | 三种拍号 × 3 个 tick 与**手算表**逐个相等；同时断言"界面读数 == 投影读数"与"注入的两个整数 == 投影的网格" |
| ①（纯逻辑侧） | `timecode_is_formatted_from_the_time_signature_not_from_four_four` | `src/bridge.rs` | 4/4 / 3/4 / 6/8 / 7/8 × 多个位置（含 `001.07.479` 这种拍内边界）；并把 `TimecodeGrid` 与 `SessionRuntimeState::ticks_per_bar` 逐项对账 |
| ② 拍号改变 ⇒ 读数随之变（**两方向**） | `changing_the_time_signature_changes_the_reading_at_the_same_tick_both_ways` | `src/test_port_adapter.rs`（Tier-1） | 4/4 → 3/4 → 4/4，**引擎位置不变**（`position_ticks == 3840` 被断言） |
| ②（纯逻辑侧） | `the_reading_at_one_tick_depends_on_the_time_signature_in_both_directions` | `src/bridge.rs` | 两个方向都 `assert_ne!`（防单向巧合） |
| ③ 读数来自投影（不是界面自己算） | 判据 ① 的"界面读数 == 投影读数"两条 `assert_eq!` + `the_projection_reading_follows_the_project_time_signature_and_degrades_honestly` | 两处 | 手算表是**独立**来源；投影是唯一事实源 |
| ④ IME 两态可区分（`false→true→false`） | `ime_composition_flows_from_the_slint_event_source_into_the_control_plane`（`tests/live_ui_mcp.rs` 判据 14b） | MCP 只读观测 | `ui/property {"name":"isComposing"}` 经 `false → true → false`，且与 `idle` 读数逐次比较 |
| ④（Tier-1 侧） | `ime_event_source_drives_the_input_context_and_swallows_bare_shortcuts` | `src/test_port_adapter.rs`（Tier-1） | 同上，直接对 `InputContext` 断言（与 MCP 读的是同一个对象） |
| ⑤ 防护真的拦住合成中的按键（**行为断言**） | 同上两条 | 两处 | `Resolution::ConsumedByIme` / `ui/dispatch_key_press dryRun` 的 `"consumed-by-ime"`；**并且**断言非合成态下同一键是 `Action(PlayPause)`（否则"吞键"可能只是恒定值） |
| ⑥ 无合成事件源时如实 SKIP 并打印原因 | `an_unwired_ime_event_source_changes_nothing_and_the_skip_is_reported` | `src/test_port_adapter.rs`（Tier-1） | 负向对照（没接线 ⇒ 位一位不动，接线后同一个调用立刻生效 ⇒ `wire_input` 是唯一边）+ 打印 §1.2 的 SKIP 原因 |
| ⑦ 既有 Tier-1 判据全绿 | 见 §5.1 | 本机全量 | 167 lib + 14 `cli_contract` + 16 `live_ui_mcp` + 2 `open_project_file` + 15 `real_ui_tier1` + 10 `undo_wiring_ui`，**0 failed** |
| ⑧ 门禁 | `bash scripts/gates/run-gates.sh light` | 本机 | `门禁通过 (mode=light)` |

### 4.2 三条注入 → 变红 → 还原（每条都当场跑过）

| # | 注入（临时改一处） | 期望 | 实测（原文摘录） | 还原 |
| :--- | :--- | :--- | :--- | :--- |
| A | `host.rs` 的 `timecode_grid_of` 改成 `TimecodeGrid::from_time_signature(4, 4)`（= 退回写死 4/4） | 时间码两条判据红 | `3/4 的 tick 3840 读数必须是 002.02.000（手算表） left: "002.01.000" right: "002.02.000"`；另一条 `left: "002.01.000" right: "002.02.000"` | ✅ 已还原（`git diff` 无 `INJECTION` 残留） |
| B | `host.rs` 的 `apply_view` **不再注入** `timecode-ticks-{beat,bar}`（两个属性留 0 ⇒ 网格 `None`） | 时间码判据红（且表现为**如实**的 tick 文本，不是假 4/4） | `4/4 的 tick 960 读数必须是 001.02.000（手算表） left: "tick 960" right: "001.02.000"` | ✅ 已还原 |
| C | `host.rs` 的 `wire_input` 把 `on_ime_composition_changed` 改成 no-op | IME 判据红，且**旧判据（直接驱动 `InputContext`）仍绿** | `assertion left == right failed: 合成态必须由**事件源**点亮 left: Bool(false) right: true`；Tier-1 侧两条同时红 | ✅ 已还原 |

注入 C 的副产物是一条**判别力证据**：`ime_composition_is_observable_on_the_live_window`
（判据 14，直接写 `InputContext`）在注入 C 下**依然通过** —— 也就是说，
只有本线新增的那条判据能抓住"事件源没接线"。这正是上一版把 need 留成
"元素/状态机存在 ≠ 事件源已接"的原因。

---

## 5. 本机真跑 vs CI（严格区分）

### 5.1 本机真跑（全部复用**主仓暖 target**，增量有界；编译时间如实登记）

| # | 命令 | 实测 | 编译时间（`time`） |
| :--- | :--- | :--- | :--- |
| 1 | `CARGO_TARGET_DIR=<主仓 target> cargo-local.sh check -p yeban-app --all-targets --locked` | 0 错 0 警告（第一次：含 `build.rs` 真跑一遍 slint-build 编译改动后的 `.slint`） | **8.8 s**；改完再跑 **1.5 s** |
| 2 | 同上 + `clippy --all-targets --locked -- -D warnings` | `Finished`，零告警 | **7.2 s** |
| 3 | 同上 + `test -p yeban-app --all-targets --locked` | **224 passed / 0 failed**（167 + 14 + 16 + 2 + 15 + 10），见 §4.1 ⑦ | **17–24 s**（`fmt` 之后最后一次 **21.8 s**） |
| 4 | 单测过滤跑（`--nocapture timecode` / `ime` / `changing_the_time_signature`） | §3 的三段实测输出 | 秒级（无重编译） |
| 5 | `bash scripts/gates/run-gates.sh light` | `门禁通过 (mode=light)`（fmt / 守卫 / 文档 / 许可清单） | 秒级 |

**关于纪律**：集成者已裁决本线**允许**"复用已建缓存、增量有界"的本机编译来验证
Tier-1 / 契约判据。本线**没有**做 Slint 的首次全量构建：`CARGO_TARGET_DIR` 指向
主仓已建好的 `target/`，Slint 全家桶按源码身份复用，只有本工作树的成员 crate 与
`.slint` 的 codegen 重编（冷 `--all-targets` 全链路 24 s、增量 1.5–7 s 即证）。

### 5.2 本机**没有**验证的（交给 CI）

- `cargo clippy --workspace --all-targets -D warnings` 在**干净 target** 上的结论
  （本机是复用暖 target 的等价检查，`-p yeban-app` 而非 `--workspace`）；
- 其它 crate 的判据（本线只碰 `crates/yeban-app/**`，且没有改任何公共契约的形状
  —— `host::timecode_for_ticks` 是**删掉**而不是改签名，只有本 crate 的调用点）；
- `deny.toml` / 依赖图（本线零新增依赖，`Cargo.toml`、`Cargo.lock` 未改）；
- 平台级 IME preedit 的端到端（上游无公开注入面，见 §6 N1）。

---

## 6. needs（本线做不到、或刻意不做的）

| # | needs | 状态与理由 |
| :--- | :--- | :--- |
| N1 | **平台级 IME preedit 注入面**：Slint 1.18.1 有合成信号（`TextInput.preedit-text`），但没有公开注入面（§1.2 三条事实）。要端到端覆盖"平台事件 → preedit → 回调"，需要二选一：<br>(a) 集成者批准给 `yeban-app` 加 `i-slint-core = "=1.18.1"` 的 **dev-dependency**（新依赖，超出本线"零新增依赖"约束，故未做）；<br>(b) 上游把 `InternalKeyEvent` / `KeyEventType` 再导出（`slint::platform` 下）。<br>在此之前，判据覆盖到 `.slint` 回调这一格，并由判据 ⑥ 当场打印原因 | **pending（集成者裁决）**。本线不造假信号、不加依赖 |
| N2 | **GUI 路径上的键盘源**（原题：`InputContext` 还没有读者；根因：Slint 不暴露物理键码）—— ✅ **已闭环（2026-10-06，按 `open-questions.md` 问题 1 的裁决 (1) 执行）**。<br>**裁决**：**GUI 绑逻辑键**；无头端口**保留物理码判据**；残余逐条记录（本行）。<br>**GUI 现在覆盖什么**：`ui/app.slint` 的 `forward-focus: key-handler;` + `key-handler := FocusScope { key-pressed(event) => root.key-action(event.text, 四个修饰位) }` 是**唯一**键盘事件源；`host::wire_keys` → `input::LogicalKey::from_text`（词表取自 Slint 的 `key_codes`：`\t`/`\n`/`\u{1b}`/`\u{7f}`/`\u{f708}`/`\u{f709}`/空格/可打印字符，统一小写化）→ `InputContext::resolve_logical` → `host::apply_action`。`InputContext::resolve`（物理入口）**就是** `resolve_logical(key.logical(), ..)` ⇒ 只有一张策略表（反漂移判据 `physical_and_logical_entries_resolve_identically`）。生产 `main.rs` 传真 `UndoPort`（撤销族经 `undo::dispatch_key` 唯一下发点，与 D45 的按钮同一条链）；无头执行面 `live_surface.rs` 传 `None`（无撤销会话 ⇒ 如实 `reject`）。**判据**：`tests/live_ui_mcp.rs` 判据 16（端口注入逻辑键 `"3"` ⇒ `active-tool` 真的变 3；注入 `Tab` ⇒ 切视图且被消费；未绑定键与无会话的 `Cmd+Z` 如实 `reject`；负向实测：摘掉 `wire_keys` 即变红）+ 零 Slint 的 `undo.rs::the_logical_key_chain_really_rolls_the_project_back`。<br>**端口物理码判据仍覆盖的情形**（逻辑绑定**结构上**表达不了，因此一条都没删）：① **与布局无关的键位意图** —— `PhysicalKey::KeyZ` 在 AZERTY 上仍是"Z 那个**键位**"，按下去照旧触发同一动作；逻辑绑定看到的是该布局打出来的**字符**（`w`），无法表达"键位"；② **Shift 改变了字符的键** —— 美式 `Shift+1` 的 `text` 是 `"!"`，逻辑词表**刻意不折回** `"1"`（`LogicalKey::from_text` 的文档与判据 `logical_keys_are_parsed_from_the_slint_text_vocabulary` 钉住这一点），因此 `Alt+1`/`Alt+2` 这类组合在逻辑路径上可能解析不出来，只有物理码判据能稳定覆盖；③ **物理修饰键身份**（`Shift+Enter` 这类 chord、裸修饰键的按下/释放状态由平台给出，不经文本推断）。承载它们的判据：`src/live_surface.rs::physical_key_of` + `key_resolution`（`ui/dispatch_key_press` 的 `dryRun.resolution`）、`src/test_port_adapter.rs` 的 IME 门控与 `active-tool` 判据、`src/undo.rs::perform_key` 的物理码整链判据 —— **全部原样通过**。<br>**如实登记的残余**：① 键盘源需要 Slint 焦点 —— `forward-focus` 在窗口建立时把焦点给到 `key-handler`（判据 16 的端口注入就是这一点的机械证据：**不需要先点界面**）；焦点被 BPM 敲入框拿走时，单键快捷键按 §7.1/§7.2 **归文本控件**（那是规范语义，不是缺口），回到这里靠 Slint 的焦点轮转（`Tab`）或程序性 `focus()`，**没有**"自动回焦"的判据（留给后续 need）。② 逻辑键路径**不覆盖** `[UI-NOTE-005]` 的方向键等尚未接线的键（`from_text` 返回 `None` ⇒ 不消费） | **已闭环**（裁决 (1) 已执行；判据可失败（负向实测）；残余写在这里，不假装全绿） |
| N3 | `bpm-input` 的数值校验 / 提交回模型（`edited` → `bpm`）未接线 —— 上一版注释里"随模型线落地"的那半句仍然成立 | **pending**（属模型写入线） |
| N4 | 复合拍号的小节内**分组**（如 6/8 = 2 组 × 3 个八分）显示口径未定义：本线采用"一拍 = 一个分母音符"（6/8 的一小节 = 6 拍）。规范没有钉死，本线不擅自发明 | **pending（人类裁决）**，现状已在 §3 留痕 |
| N5 | 两份别的线的台账里，本线闭环的那条 need 需要**由集成者**标注闭环：`docs/ledger/transport-engine-notes.md:335`（need 6，理由已过时，见 §1.1）与 `docs/ledger/ui-mcp-dryrun-ime-notes.md` 的 N1（"Slint 平台的 IME 事件尚未接到 InputContext"） | **禁改**（一树一线 / 一个写者），请集成者更新 |
| N6 | `.slint` 里新 `TextInput` 的**视觉回归**：本机只断言了"截图非全黑 / 交互改变像素"，没有按 `[UI-MCP-003]` 的分平台 Golden 复核 BPM 编辑面的排版（CI 才有 Golden 腿） | **pending（CI）** |

---

## 7. CI 判决

判决由 `scripts/dev/ci-verdict.sh line/app-projection` 读回；**未读到的判决一律记 `pending`**。
本线提交后一次性 push，判决见本文件随后的更新（或集成者的合并记录）。
