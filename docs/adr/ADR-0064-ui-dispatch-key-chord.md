# ADR-0064 — 控制面的修饰键 chord 注入面（`ui/dispatch_key_chord`，缺口 D-UI-3）

> ⚠ **号位说明**（2026-11-01 rebase 到 main 时改号）：本票最初在工作线上登记为 `ADR-0061`。
> main 在那期间把 `0061` 给了乒乓延迟、`0062` 给磁带饱和、`0063` 给侧链键输入 ⇒ 按
> `docs/adr/README.md` 里既有的"后落地的一方改号"惯例（`c5e83a46` / `f0499302` 两次同族改号），
> 本票改记 **`ADR-0064`**。裁决内容一字未改。

- **状态**: **Proposed（待人类批准；⛔ 集成者代签）**
  —— 本票**不是**零代码票：实现已落在工作线 `line/zimmer-ui-write-surface`（未合流），
  理由是「模拟人类 UI 模式创作」这条路在当前能力面上走不通（见"为什么现在必须定"）。
  若人类否决 B，撤回的是**方法面**（`ui/dispatch_key_chord` 一条 + 其预览/判据），
  端口层的数据形状（`KeyChord`/`ChordKey`/`KeyChordModifiers`）可留作后续实现的底座。
- **决定者**: 待人类裁决
- **关联**: 规范 `[UI-TEST-002]` §12.4 第 2 条（`docs/YEBAN_DESKTOP_UI_UX_AND_INTERACTION_REDESIGN.md`，
  注意该文档整体于 2026-10-06 降级为 **Advisory**）；
  台账缺口 `docs/ledger/ui-mcp-dryrun-ime-notes.md` 行 **N2**（"chord 注入属 `UI-MCP-001`
  签名扩展，需人类裁决"）；
  `crates/yeban-ui-test-port/src/port.rs`（`KeyCode::ShiftEnter` 是**唯一**组合变体）、
  `crates/yeban-ui-test-port/src/render.rs`（`dispatch_key_press_impl` 的 Shift 序列）、
  `crates/yeban-app/src/live_surface.rs`（`dispatch_key_chord_impl` 新落点、
  `chord_resolution`）、`crates/yeban-app/src/host.rs`（`wire_keys` → `on_key_action`）、
  `crates/yeban-app/src/input.rs`（`LogicalKey::from_text`、`InputContext::resolve_logical`、
  键位表 resolve 顺序）、`crates/yeban-app/tests/live_ui_mcp.rs`（缩放判据第 ③ 步直调
  `invoke_key_action`，即本票要变成端口的同一格）、新增判据文件
  `crates/yeban-app/tests/d_ui_3_chord_injection.rs`
- **日期**: 2026-11-01

## 背景（全部现读，⛔ 不采信报告措辞）

1. **规范自己矛盾**：§12.4 第 2 条写的是
   「`dispatch_key_press(key_code)`：分发 `Tab`、`Shift+Enter`（采纳 AI 提案）、`Esc` 等热键」——
   签名里只有**一个** `key_code`，却要求分发一个**组合键**。
   实现照签名办：`KeyCode` 枚举把 `ShiftEnter` 做成**一个具名变体**
   （另有两个裸修饰键 `Shift`/`Control`，注释自称"裸修饰键"）。
   ⇒ 缺口不是"漏实现"，而是**签名表达不了 chord**：每支持一个新组合就得新增一个枚举变体
   ＋改一次 `dispatch_key_press_impl` 的 match ＋改一次 `parse`/`name`。
2. **现有组合走的是平台事件序列**：`render.rs` 的 `KeyCode::ShiftEnter` 分支依次
   `key_press(Key::Shift)` → `key_press(Key::Return)` → `key_release(Key::Shift)`
   （注释还专门警告"修饰键必须用完就放"）。这条路依赖 Slint 自己维护修饰键状态，
   且 `.slint` 的 `key-handler` 会把 `Shift+Z` 折成**文本** `"Z"`，
   而 `input::LogicalKey::from_text` 一律小写 ⇒ `Shift+Z` 与裸 `z` 在这条路上**不可区分**
   （裸 `z` = ZoomToSelection，`Shift+Z` = ZoomToFit，是两个不同动作）。
3. **唯一忠实落点已经在用，只是没进端口**：`host.rs` 的 `wire_keys` 把每次真实击键折成
   `on_key_action(text, shift, ctrl, alt, meta)`，`MainWindow::invoke_key_action` 就是这一格的
   可编程入口 —— 它携带**全部四个修饰键**、命中**同一张键位表**、走**同一个** UndoPort 提交路径。
   既有判据（`live_ui_mcp.rs` 缩放段第 ③ 步）正是直接调 `invoke_key_action("z", true, false, false, false)`
   并断言 `get_roll_ticks_per_pixel()` 跟着变 —— 也就是说"这条路能走通"**已被测过**，
   缺的只是把它挂到控制面上（该判据注释里写的"新增变体要动 `yeban-ui-test-port`，不在本切片的
   改动面"就是本票登记的这个 needs）。
4. **键位表规模**：`input.rs` 里带修饰键的动作不是个位数 —— `Shift+Z`（缩放到全曲）、
   `Shift+↑/↓`（±12 半音）、`Shift+←/→`（时值 ±1 网格）、`Alt+↑/↓`（选轨）、
   `Cmd/Ctrl+S|O`、`Cmd/Ctrl+Shift+S`、`Cmd/Ctrl+Alt+A`、`F7/F8/F9` ……
   逐个做成枚举变体不可扩展。

## 为什么现在必须定（D-UI-3 是硬阻塞，不是锦上添花）

目标之一是**用模拟人类 UI 的方式创作一首 3 分钟作品**并核验它。实测口径：

- 默认 `ticks_per_pixel` = 30，卷帘/编曲视口原点在 168px 一处 ⇒ 默认可见跨度约 **13.7 小节**；
- 剪辑（clip）在本工程创建路径上**恒为 4 小节、恒从 tick 0 起**；
- 编曲面**没有**横向滚动（`ui/arrangement_view.slint` 的注释自证"编曲视图故意不带 host scroll-x"）；
- 卷帘有拖拽滚动（`piano-roll-scroll-area` → `on_scroll_requested` → `apply_view`），
  但 `apply_view` 只用 `scroll_x` 更新 `visible_notes_scoped` 与 `set_roll_scroll_x`
  —— **剪辑位置不受影响** ⇒ 滚卷帘不会把编曲面后面的小节带进视野；
- 于是 180 s 的内容无法在 UI 模式里"看见并核对"，唯一机制是 `Shift+Z` = ZoomToFit
  （`bridge::ticks_per_pixel_to_fit(span, viewport)`，clamp 到 `MIN..=MAX_TICKS_PER_PIXEL = 1..=960`）。
- 替代方案「把 BPM 压到 ≤18 让 13.7 小节覆盖 3 分钟」被否：那不是音乐。

⇒ 没有 chord 注入，"模拟人类 UI 模式创作"这一半目标只能靠 `invoke_key_action` 的**测试内直调**
来冒充（那不是控制面），或者放弃。

## 三个选项（⛔ Agent 不代选）

- **A 维持现状**：只保留 `KeyCode::ShiftEnter` 一个组合变体，需要新 chord 时逐个加变体。
  成本：每个组合一次跨 crate 改动（枚举 + `parse` + `name` + `dispatch_key_press_impl` 的 match），
  且新变体仍走**平台事件序列**那条"Shift+Z 与 z 不可区分"的路 ⇒ 大部分组合加进来也是错的。
  本票**不推荐**，但它是零风险项。
- **B 新增一条方法面 `ui/dispatch_key_chord`（本票的实现所采）**：入参 `keyName` +
  四个布尔 `shift`/`ctrl`/`alt`/`meta`，落点 `MainWindow::invoke_key_action`。
  理由：`[UI-TEST-002]` 的 `dispatch_key_press` 签名已定且被广泛使用，往它身上挂第二套
  落地机制会让**同一个方法名对应两种事件语义**（平台事件 vs 回调直调），"注入了什么"
  将不可从方法名读出。分开一条，`ui/methods` 的注册表里两者的 `port_operation` 也各自可追。
  代价：**这是签名扩展** ⇒ 需人类批准（N2 的原话），且 `METHOD_COUNT` 17→18、
  `Operation::ALL` 10→11、mutating 方法 10→11（三处计数各有一条判据钉住，改时同步）。
- **C 扩展 `dispatch_key_press` 加可选修饰键参数**：不新增方法名，但同样要改规范签名，
  并把"平台事件/回调直调"两种机制藏在同一方法名后。比 B 省一行方法名，语义更糊 ⇒ 否。

## 采纳 B 时的实现边界（已按此实现，供批准时核对）

1. **权限不降级**：`Operation::DispatchKeyChord.required_permission() == Permission::Interactive`，
   且**与 `DispatchKey` 同一层级** ⇒ 生产模式硬禁、`ui:inject` scope 自动跟着走，
   不新开权限位。判据 `chord_injection_shares_the_key_permission_gate` 钉这一点。
2. **键位词表与真实击键同源**：`ChordKey::parse` 的每条返回都对应
   `input::LogicalKey::from_text` 能解析的那个码点（`\u{0008}`/`\u{0009}`/`\u{000a}`/`\u{001b}`/
   `\u{007f}`/`\u{f700..f703}`/`\u{f708..f70c}`/空格/ASCII 可见字符），
   判据 `every_named_chord_key_round_trips_and_carries_one_char` 逐个对账。
   解析不出来 ⇒ `PortError::UnknownKey`（⛔ 不猜一个键）。
3. **`consumed` 不是 `accepted`**：回执字段为 `dispatched`/`chord`/`consumed`。
   `consumed == false` 意味着"这一组合没有绑定、被放行给焦点系统"，
   ⛔ 不是调用失败。一个只有 `accepted: true` 的回执会把"没绑任何东西"与"快捷键生效"
   写成同一句话。
4. **零依赖方向不破**：`LivePort<T>` 是泛型、⛔ 不认识 `yeban-app` 的 `invoke_key_action`
   ⇒ `dispatch_key_chord_impl` 用 trait **默认实现**返回 `PortError::Rejected`，
   对外仍是既有 `-32005 NOT_IMPLEMENTED`（D25：⛔ 不发明错误码），
   与 `set_track_height_impl` / `open_project_impl` 同款。`PortAdapter` 因此**无需**转发改动。
5. **dryRun 双面对齐**：真实面与假面的 `preview_effect` 都报 `chord`/`isComposing`/`focus`/`resolution`，
   其中 IME 那一格读**同一份**输入状态（`[UI-A11Y-002]` 的两格口径不变）。
6. **不碰已文档化的方法**：§12.4 列出的 10 条 `ui/*` 一字未改，本票是纯加法。

## 判据（怎么证明它真的做了事，⛔ 不靠"看起来接上了"）

| 层 | 判据 | 断言的是 |
| --- | --- | --- |
| 端口 | `every_named_chord_key_round_trips_and_carries_one_char` | 每个具名键 `parse→text→parse` 稳定、`text()` 恰一个码点、别名可用、组合名（`"shift+z"`）在 `ChordKey` 层被拒 |
| 端口 | `chord_name_lists_modifiers_in_the_documented_order` | 名字顺序 `Cmd+Ctrl+Alt+Shift+<键>`；裸键无分隔符；`NONE.is_bare()` |
| 端口 | `chord_injection_shares_the_key_permission_gate` | 权限与 `DispatchKey` 同；ReadOnly ⇒ `PermissionDenied`；默认实现 ⇒ `Rejected` |
| 方法面 | `dry_run_preview_matches_the_effect_of_the_real_call`（既有判据扩一格） | 预览的 `chord` == 真做之后的名字、`isComposing` == 夹具读数、`resolution` 为放行 |
| 方法面 | `registry_counts`（既有） | `METHOD_COUNT == 18`、`mutating` 11 条 |
| 真实载体 | `crates/yeban-app/tests/d_ui_3_chord_injection.rs` | ①词表同源；②注入 `Shift+Z` 后 `get_roll_ticks_per_pixel()` 真的变成 `ceil(跨度/视口)`；③未绑定的组合 `consumed == false` 且一位状态没动 |

## 若批准 A（否决 B），需要一并处理

`ui/dispatch_key_chord` 整条撤回（方法 + `Operation` 变体 + 三面实现 + 判据 + 本票），
并把「模拟人类 UI 模式创作 3 分钟作品」这一目标改为**测试内直调 `invoke_key_action`**
的口径登记（即承认那不是控制面能力）。台账 N2 行随之关闭为"维持现状"。
