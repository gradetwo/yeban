# ui-shell 工作线备注 (API 核验记录 · 缺口 · 待决问题)

- **工作线**: `line/ui-shell`（worktree `.worktrees/ui-shell`）
- **日期**: 2026-10-05
- **范围**: 只覆盖 `crates/yeban-app/**`（Slint 宿主 + UI 骨架）。不碰 `yeban-ui-test-port` /
  `yeban-ui-mcp`（别的工作线），不改任何根级共享文件。
- **本机纪律**: 全程**未编译 Slint**。所有"能编译"的结论都是 CI 判的，不是本机判的。

---

## 1. 交付物 ↔ 规范 ID 映射

| 交付物 | 规范 ID | 说明 |
| :--- | :--- | :--- |
| `ui/tokens.slint` | 规范缺口（§1 几何、§7.4/§10.2 语义色、品牌色） | 网格几何 / 间距 / 圆角 / 字号 / 字体栈 / 品牌色的唯一事实源 |
| `ui/app.slint` | `UI-GRID-001` `UI-GRID-002` `UI-GRID-003` `UI-GRID-004` `ARCH-UI-001` `ARCH-TOP-002` `ARCH-UI-003` | 主窗口 + 全局网格 + 覆盖层（compact 抽屉 / Musical PR / 时光机） |
| `ui/transport.slint` | `UI-GRID-001`（48px 顶栏）`UI-TEST-001` `UI-A11Y-003` `ARCH-UI-002` | 走带 / BPM / 时间码 / 分支与提交 / AI 提案徽章 / 视图切换 |
| `ui/status_bar.slint` | `UI-GRID-001`（24px 状态栏）`UI-A11Y-003` | ADR-0001 D2 新增的组件 |
| `ui/sidebar.slint` | `UI-GRID-001` `UI-GRID-002`（36px 图标导轨）`UI-TEST-001` | 左侧资源栏 / 分类 / 资源条目 |
| `ui/workspace/session_view.slint` | 规范 §2.2 §2.3 `UI-TEST-001` | 剪辑矩阵 + 场景激发 + Back to Arrangement |
| `ui/workspace/arrangement_view.slint` | 规范 §4.2 §4.3 `UI-TEST-001` `UI-MCP-002` | 轨道包头 + 标尺 + 循环选区 + 章节卡片 + 走带光标 |
| `ui/console/console_tabs.slint` | `UI-GRID-001`（底部控制台 35–45%）`UI-TEST-001` | Tab 导轨 + 三页叠放 |
| `ui/console/piano_roll.slint` | 规范 §3.1 §3.3 §3.5 `UI-TEST-001` `UI-MCP-002` | 卷帘骨架 + 工具矩阵 + 音符图元 + 力度泳道 + AI 建议层 |
| `ui/console/mixer_console.slint` | 规范 §5.2 `UI-TEST-001` `UI-MCP-002` `ARCH-UI-002` | 通道条 + VU 电平 + 主控 |
| `ui/console/device_rack.slint` | 规范 §5.1 §5.3 `UI-NOTE-004` `UI-MCP-002` | 4 段 EQ + 设备卡片 + 插件宿主占位 |
| `ui/dialogs/musical_pr_drawer.slint` | 规范 §9.1 §10 `UI-A11Y-004` `UI-TEST-001` | AI 提案审核抽屉 + 视觉 Diff 图例（色相 + 几何双编码） |
| `ui/dialogs/undo_tree_modal.slint` | 规范 §10 `UI-A11Y-001` `UI-A11Y-003` `UI-TEST-001` | 版本 DAG 弹窗 |
| `src/input.rs` | `UI-A11Y-001` `UI-A11Y-002` `UI-A11Y-003` | 物理键 + IME 屏蔽状态机（纯 Rust，22 条判据） |
| `src/elements.rs` | `UI-TEST-001` `UI-MCP-001` `UI-MCP-002` | 语义元素注册表（184 条 / 14 个动态遮罩区域）+ 与 `.slint` 的双向契约判据 |
| `src/scene.rs` | `UI-GRID-002` `MODEL-AST-001`（ULID 文本形式） | 演示数据 + 断点函数 + ULID 校验 |
| `src/main.rs` | `ARCH-UI-003` `UI-TEST-003` `ARCH-TOP-002` | `--headless` 握手 / `--dump-elements` / `--print-shortcuts` / GUI 路径 |
| `build.rs` | ADR-0001 D2 | `slint_build::compile("ui/app.slint")` |

文件数核对：UI/UX §8 列 11 个 `.slint`，ADR-0001 D2 加 `status_bar.slint` = 12，本次再加
`tokens.slint`（主题层） = **13**。这条清单由
`elements::tests::slint_manifest_matches_adr_0001_d2` 机械钉死。

---

## 2. Slint 1.18.1 API 核验记录（写代码前逐条对过文档）

| # | 结论 | 出处 |
| :-- | :--- | :--- |
| 1 | `slint` 的 feature 名（本 crate 用到）：`std` / `compat-1-2` / `accessibility` / `backend-default` / `renderer-femtovg` / `renderer-software`。**默认集含 `std` 等 8 个**；根 `Cargo.toml` 写的是 `default-features = false`，因此成员这一层必须逐个补回 | <https://docs.rs/crate/slint/1.18.1/features> |
| 2 | `backend-default` 转发到 `i-slint-backend-selector/default`，后者 = `backend-winit` = `backend-winit-wayland` + `backend-winit-x11`。所以**不需要**手写那三个后端 feature | <https://docs.rs/crate/i-slint-backend-selector/1.18.1/features> |
| 3 | `slint_build::compile(path: impl AsRef<Path>) -> Result<(), CompileError>`；`CompileError` 在 `slint_build::CompileError`，`#[non_exhaustive]`，有 `Debug`。同族还有 `compile_with_config` / `compile_with_output_path` / `print_rustc_flags` | <https://docs.rs/slint-build/1.18.1/slint_build/> |
| 4 | `slint-build` 的 `compat-1-18` 是**强制特性**：缺了它 crate 直接 `compile_error!`（源码 `lib.rs:53-57`）。根里 `default-features = false` 关掉了它，所以成员必须补 | <https://docs.rs/slint-build/1.18.1/src/slint_build/lib.rs.html> |
| 5 | 生成物落点：`$OUT_DIR/<根文件 stem>.rs`，并把绝对路径写进 `cargo:rustc-env=SLINT_INCLUDE_GENERATED=...`（源码 515-559 行）。即 `ui/app.slint` → `$OUT_DIR/app.rs` | 同上 |
| 6 | `slint::include_modules!()` = `include!(env!("SLINT_INCLUDE_GENERATED"))`，作用是"包含生成代码并让导出的类型可实例化" | <https://docs.rs/slint/1.18.1/slint/macro.include_modules.html> |
| 7 | 生成组件的 `new()` 返回 `Result<Self, slint::PlatformError>`（不是 `Self`）。`slint-build` 文档里那个 `HelloWorld::new().run()` 的例子是 `ignore`（未测试）的旧写法 | <https://docs.slint.dev/latest/docs/rust/slint/docs/generated_code/struct.SampleComponent> |
| 8 | `slint::ComponentHandle` 的方法签名：`run(&self) -> Result<(), PlatformError>`、`show(&self) -> Result<(), PlatformError>`、`hide(&self) -> Result<(), PlatformError>`、`window(&self) -> &Window`、`global<'a, T>(&'a self) -> T`、`as_weak()`、`clone_strong()` | 同上 |
| 9 | 真实工程的用法佐证：`slint::include_modules!();` 之后直接 `App::new().unwrap()` / `app.global::<G>()` / `app.run().unwrap()`（且**没有** `use slint::ComponentHandle`） | <https://raw.githubusercontent.com/slint-ui/slint/v1.18.1/examples/gallery/main.rs> |
| 10 | **Slint 1.18.1 没有名为 `headless` 的后端。** `SLINT_BACKEND` 只接受后端名 `qt` / `winit` / `linuxkms`，可用 `-` 追加渲染器后缀（`winit-software` / `linuxkms-skia` / `winit-vello`）。`BackendSelector` 是它的编程等价物（`backend_name(String)` / `renderer_name(String)` / `select() -> Result<(), PlatformError>`），且**只接受已编译进去的后端** | <https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backends_and_renderers/> 与 <https://docs.rs/slint/1.18.1/slint/struct.BackendSelector.html> |
| 11 | `.slint` 无障碍属性**存在且名字如下**（`accessible-role` 是其他属性的前置条件）：`accessible-role` / `-label` / `-description` / `-checkable` / `-checked` / `-enabled` / `-expandable` / `-expanded` / **`-id`** / `-orientation` / `-live-region` / `-value` / `-value-minimum` / `-value-maximum` / `-value-step` / `-placeholder-text` / `-read-only` / `-item-selectable` / `-item-selected` / `-item-index` / `-item-count` | <https://docs.slint.dev/latest/docs/slint/reference/common/> |
| 12 | **`accessible-id` 就是"给自动化/测试用的唯一标识符"**，官方文档原话：*"A unique identifier for the element, used to identify widgets for automation and testing purposes."* 所以**规范 §12.2 要求的语义 Element ID 机制在 Slint 1.18 里是原生存在的** | 同上 |
| 13 | `AccessibleRole` 的合法取值（kebab-case）：`none` `button` `checkbox` `combobox` `groupbox` `image` `list` `slider` `spinbox` `tab` `tab-list` `tab-panel` `text` `table` `tree` `progress-indicator` `text-input` `switch` `list-item` `radio-button` `radio-group` `window-title-bar`，以及地标 `banner` `complementary` `content-info` `form` `main` `navigation` `region`（+ `search` 被截断未确认） | <https://docs.slint.dev/latest/docs/slint/reference/property-types/builtin-enums/> |
| 14 | `internal/core/accessibility.rs` 的 `AccessibleStringProperty` 里确有 `Id` 变体（kebab-case → `id`），与 `accessible-id` 对上；同文件还给出 `AccessibilityAction::{Default, Decrement, Increment, Expand, ReplaceSelectedText, SetValue, SetSelectionOffsets}` | <https://raw.githubusercontent.com/slint-ui/slint/v1.18.1/internal/core/accessibility.rs> |
| 15 | `i-slint-backend-testing` 的 `ElementHandle` 提供：`find_by_accessible_label` / `find_by_element_id` / `find_by_element_type_name`、`query_descendants` / `visit_descendants`、`accessible_id()` / `accessible_label()` / `accessible_role()` / `accessible_value*()` / `id()` / `type_name()` / `bases()` / `layout_kind()` / `size()` / `absolute_position()` / `computed_opacity()`，以及事件注入 `mock_single_click` / `single_click` / `double_click` / `drag` / `mock_drag` / `scroll` / `invoke_accessible_*`。`ElementRoot` 对**任何 `ComponentHandle`** 有 blanket impl | <https://docs.rs/i-slint-backend-testing/1.18.1/i_slint_backend_testing/struct.ElementHandle.html> |
| 16 | 注意 15 里的 `find_by_element_id` 用的是**限定元素 id**（`组件名::局部名`，来自 `foo := Rectangle {}`），**不是** `accessible-id`。两者是不同机制：前者是"源码里的命名节点"，后者是"语义业务 ID"。`ElementHandle::accessible_id()` 才能读到 `accessible-id` 的值 | 同上 |
| 17 | 颜色字面量"follow the syntax of CSS"；文档**没有**一句话写死 8 位十六进制的通道顺序。对策：`tokens.slint` 只用不透明的 6 位 `#rrggbb`，alpha 一律用 `.with-alpha(float) -> brush` 在使用点表达 | <https://docs.slint.dev/latest/docs/slint/reference/property-types/colors-and-brushes/> |
| 18 | 运算与类型：`+`/`-` 要求同类型（或一侧是纯数字）；`*` 是"值 × 数字"；`/` 可以"值 ÷ 数字"也可以"同单位相除得数字"。**没有取模运算符**（数字后的 `%` 是百分号单位），要用 `Math.mod` 或方法形式 `.mod(n)`。字符串拼接里 **数字会被自动转成字符串**，其他类型不会 | <https://docs.slint.dev/latest/docs/slint/reference/language/operators/>（本仓库另存了官方 `common.mdx` 里 `i.mod(2)` 的用法佐证） |
| 19 | **`&&` 与 `||` 是 non-associative**，混用是编译错误（`a && b \|\| c` 必须加括号）。为避免"同运算符串联是否合法"的不确定性，黑键判断改成查表 + `.mod()`，全仓库只剩 2 个操作数的 `&&` / `\|\|` | 同上 |
| 20 | **`root` 是每个组件最外层元素的自动 id**（`self` / `parent` / `root` 三个预定义名）；`-` 与 `_` 在标识符里可互换；全局单例用 `Name.property` 访问，可被别的文件 `import` | <https://docs.slint.dev/latest/docs/slint/reference/language/name-resolution/> 与 <https://docs.slint.dev/latest/docs/slint/reference/language/globals/> |
| 21 | 回调可以有参数与返回值（`callback with-args(int, string);`、`callback sum(int,int) -> int;`），处理器自己命名参数（`sum(a, b) => { a + b }`）；也支持回调别名 `callback clicked <=> area.clicked;`。本实现**故意只用无参回调**（更小的语法面） | <https://docs.slint.dev/latest/docs/slint/reference/language/callbacks/> |
| 22 | 布局子元素用 `min-width` / `max-width` / `preferred-*` / `horizontal-stretch` / `vertical-stretch` 约束尺寸（这些属性"valid on all visible items and can be used to specify constraints when used in layouts"），**不**在布局里直接写 `width` / `height` | <https://docs.slint.dev/latest/docs/slint/reference/layouts/overview/> |
| 23 | 全局 `global` 的成员可以是 `in` / `out` / `in-out` / private 属性、callback、function；**不能**含元素、`@children`、`animate`、`states`、`transitions`、`init` —— 每条都是编译错误 | <https://docs.slint.dev/latest/docs/slint/reference/language/globals/> |

### 本 crate 实际选定的 Slint feature 组合（集成者点名要求写清）

`crates/yeban-app/Cargo.toml` 选择（根 `Cargo.toml` 里 `slint` 是 `default-features = false`，
因此**必须逐个补回**默认集里的项）：

| feature | 为什么需要 | 上游依据 |
| :--- | :--- | :--- |
| `std` | 桌面程序；没有它连 `Result`/`String` 都没有 | <https://docs.rs/crate/slint/1.18.1/features> |
| `compat-1-2` | 强制兼容门（它自身再转发 `compat-1-18`）。缺了会 `compile_error!` | 同上 |
| `accessibility` | `UI-A11Y-003` 要求向 OS 无障碍树暴露控件元数据（转发到 `i-slint-backend-selector/accessibility` → `accesskit`） | 同上 |
| `backend-default` | 选后端：转发 `i-slint-backend-selector/default` = `backend-winit` = `backend-winit-wayland` + `backend-winit-x11`。**因此不需要手写那三个后端 feature** | <https://docs.rs/crate/i-slint-backend-selector/1.18.1/features> |
| `renderer-femtovg` | `UI-GRID-003` 要求的硬件加速渲染管线（OpenGL） | <https://docs.rs/crate/slint/1.18.1/features> |
| `renderer-software` | 无 GPU / 无头环境的可移植兜底（Slint 默认集之一） | 同上 |

**明确不启用**：`renderer-skia`（拖入 LLVM/clang，与本机纪律冲突）、`system-tray`（当前不需要）、
`mcp` / `live-preview` / `unstable-*`（`AGENTS.md` §5.7 与红线 6 精神：默认构建不引入开发期特性）。

**这些 feature 在 Linux 上对应的系统库前置**（决定 CI 必须装什么 —— 已由集成者落地）：

| 传递依赖 | 系统库 / 工具 |
| :--- | :--- |
| `yeslogic-fontconfig-sys`（经 `i-slint-common → fontique`，**非可选**） | `pkg-config` + `libfontconfig1-dev` |
| `freetype-sys`（字体光栅化） | `libfreetype-dev` |
| `x11-dl`（`backend-winit-x11`） | `libx11-dev` |
| `wayland-*` / `smithay-client-toolkit`（`backend-winit-wayland`） | `libwayland-dev` |
| `xkbcommon`（`winit`） | `libxkbcommon-dev` |
| `glutin` / EGL（`renderer-femtovg`） | `libgl1-mesa-dev` |
| （后续）`cpal`（ALSA，Phase 2） | `libasound2-dev` |

### 未能核验、但影响实现的项（已知风险）

> 下表在第 2 轮 CI（`rust (yeban-app)` = success）之后回填了"是否已被证实"。
> 该 job 真的在 Linux 上编译了 `.slint`、`slint_build` 生成代码与 `main.rs`，并跑了 clippy `-D warnings` 与全部单元测试。

| 项 | 状态 | 依据 / 兜底 |
| :--- | :--- | :--- |
| `slint::include_modules!()` 放在**嵌套模块**里是否可行 | **已证实可行** | 第 2 轮 `rust (yeban-app)` success（`pub mod ui { #![allow(…)] slint::include_modules!(); }` 编译通过） |
| 生成代码是否本身 lint 干净 | **已证实干净**（至少对 `clippy::all` + `missing_docs` 而言） | 同上；`allow` 仍保留，因为它同时还挡住了未来版本生成代码的漂移 |
| 生成组件的可见路径（`yeban_app::ui::MainWindow` 而非 `ui::App::MainWindow`） | **已证实** | 同上，`main.rs` 的 `use yeban_app::ui::MainWindow;` 编译通过 |
| `ComponentHandle::run(&ui)` 的 UFCS 写法 | **已证实** | 同上（用了 UFCS 以避免 `unused_import` 与 `-D warnings` 打架） |
| `accessible-orientation: horizontal/vertical`、`overflow: elide`、`accessible-*` 各属性名与类型 | **已证实** | 全部 13 个 `.slint` 编译通过 |
| 主题层（`export global` + `.with-alpha()` + 字符串拼接 + `length * int` 等） | **已证实** | 同上 |
| 8 位十六进制的通道顺序 | **仍未核验**（也不需要） | 实现完全不用 8 位十六进制：只用 6 位不透明色 + `.with-alpha()` |
| `i-slint-backend-testing` 的 `init_*` 函数名（`init_no_event_loop()` 等） | **仍未核验** | 本 crate **不依赖**该 crate；留给 `yeban-ui-test-port` 线 |
| 运行时行为（窗口真的出现、布局比例、CJK 字体回退、无障碍树内容） | **仍未核验** | 需要真实显示器与 `yeban-ui-test-port`；本工作线从未跑过 GUI |

---

## 3. 规范 vs Slint 1.18.1 的**实测不一致**（必须如实记录）

### 3.1 `SLINT_BACKEND=headless` 在 Slint 1.18.1 不存在

规范两处都写了这个字面值：

- `[ARCH-UI-003]`（`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` 222-228 行）
  `SLINT_BACKEND=headless YEBAN_UI_TEST_PORT=9315 cargo test -p yeban-app --features ui-test-port`
- `[UI-TEST-003]`（UI/UX 规范 §12.1）
  `SLINT_BACKEND=headless YEBAN_UI_TEST_PORT=9315 ./target/debug/yeban-app --headless`

实测（第 2 节第 10 条）：`SLINT_BACKEND` 只认 `qt` / `winit` / `linuxkms`（+ 渲染器后缀）。
**`headless` 不是 Slint 的后端名。** 规范自己也预见到了上游能力不足（§12.1 末段与 `ARCH-SLINT-001`
都写了"若 Slint 内部 headless 变种存在环境局限，系统以自研 `yeban-ui-test-port` / `yeban-ui-mcp` 兜底"）。

**本次实现的处置（不是静默降级，是显式记录的边界）**：

- yeban-app 把 `--headless` 与 `SLINT_BACKEND=headless` 都当作**自研哨兵值**，好处是规范里的命令行
  原文可跑，不必让规范迁就上游；
- 无头路径**不构造任何 Slint 对象**、不初始化后端、不进事件循环，只走纯 Rust 的场景/注册表路径，
  打印 `headless ok` 后退出 0；
- 因此它证明的是"CI 能在无显示器、无后端的环境里跑起来"，**不**证明"控件树是对的"。
  真正的控件树断言 + 截图比对要等 `crates/yeban-ui-test-port` 用 `i-slint-backend-testing`
  （`ARCH-UI-005`）接起来。

### 3.2 规范 §12.1 提到的 Skia 软件后端未启用

`renderer-skia` 会拖入 LLVM/clang 工具链，与本机"不跑高耗 CPU 任务"的纪律冲突。本次启用可移植的
`renderer-software`（Slint 默认集之一）作为无 GPU 兜底，并在 notes 里记为待决项。

### 3.3 `ARCH-UI-005` 里的 API 名与实际不符（供 `yeban-ui-test-port` 线参考）

规范写 `slint::testing::init_integration_test_backend()` / `slint::testing::send_mouse_click()` /
`slint::testing::send_keyboard_char()`。实际 crate 是 `i-slint-backend-testing`，事件注入挂在
`ElementHandle` 上（`mock_single_click` / `mock_drag` / `scroll` 等，见第 2 节第 15 条），
**没有** `slint::testing` 这个路径。规范写的名字**未核验**，本工作线不依赖它。

### 3.4 `< 1366` 宽的断点行为：规范未定义

规范只写了 ≥1920 与 1366–1919 两档。`DemoScene::is_compact()` 取保守解：`< 1366` 仍按折叠处理
（"更窄的屏幕需要更多折叠"是唯一不会破坏可用性的方向）。已写成可测函数 + 判据，等人类裁决。

---

## 4. 语义 Element ID：核验结论 + 双注册表设计

**结论**：Slint 1.18.1 **有**原生元素标识机制，而且是两个：

1. `accessible-id`（字符串属性）—— 语义上就是 DOM id，官方明确说给自动化/测试用；
2. 元素 `id`（`foo := Rectangle {}`）—— 由 `ElementHandle::find_by_element_id` 查，键是
   `组件名::局部名`。

`ui/` 下 79 处 `accessible-id` 全部按 §12.2 的四个族命名：
`track-{i}-fader` / `note-{ulid}-rect` / `clip-{ulid}-header` / `tab-{name}-button`，
其余部件用 `transport-*` / `sidebar-*` / `session-*` / `arrangement-*` / `mixer-*` / `device-*` /
`musical-pr-*` / `undo-tree-*` / `status-bar-*` / `ai-rail-*` 这类同族命名。

**为什么还要一个 Rust 侧注册表**（`src/elements.rs`，184 条）：`accessible-id` 只在**有窗口实例**
时才存在，而无头路径故意不构造窗口。注册表是纯 Rust、零 Slint 依赖的事实源，让 CI 能在没有显示器、
没有渲染后端的情况下断言"元素该在的都在、ID 都合法、动态区域都登记了"。有窗口实例时，
`yeban-ui-test-port` 用 `accessible_id()` / `find_by_element_id` 与它交叉核对。

**双向契约判据**（`slint_accessible_ids_and_registry_cover_each_other`）：把 `.slint` 里
`accessible-id:` 的右值拆成**字面段序列**（`"track-" + i + "-fader"` → `["track-", "-fader"]`），
要求注册表与 `.slint` **互相覆盖** —— 注册表不能凭空发明元素，UI 加了 ID 也不能忘记登记。
这条判据做过变异测试（把 `status-bar` 写成 `statusbar` 就变红）。

---

## 5. 主题层：规范缺口的补全与依据

UI/UX 规范定义了**网格几何**（§1.1/§1.2）与**视觉 Diff 语义色**（§10.2），但没有定义
字体栈 / CJK 回退 / 间距 / 圆角 / 字号 scale。`ui/tokens.slint` 把这些登记成唯一事实源：

| 类别 | 来源 |
| :--- | :--- |
| 网格几何（48/240/36/280/4/24、splitter 220–600、断点 1920/1366、弹性比 3:2） | 规范 §1.1 §1.2 原文数值，未做任何改动 |
| 品牌色（深色 `#060a14` `#151d38`、金 `#f7e6b0` `#e2c77e` `#b8933e`、浅色 `#fdfcfa` `#f0ebe3` `#1a2340` `#5a6b8a`） | `assets/brand/README.md`（从母版 SVG 提取，非目测） |
| 视觉 Diff 语义色（新增 `#22c55e` / 删除 `#ef4444` / 微调 `#f59e0b` / 遮罩 `#000000`） | 规范 §10.2 + `UI-MCP-002`（§12.5） |
| 间距 / 圆角 / 字号 scale | **规范缺口**，本工作线首次登记，等人类裁决 |
| 字体栈 `Inter, PingFang SC, Microsoft YaHei, Noto Sans CJK SC, Noto Sans, Helvetica Neue, Arial, sans-serif` | 任务要求"系统字体 + CJK 回退"，不打包任何字体文件 |
| 记录/正常/告警状态色 | 规范未定义，已登记待决 |

**浅色调色板只登记未接线**：运行时主题切换的机制（Window 属性 vs `Tokens.dark` vs 独立主题组件）
是待决项——不发明答案。

---

## 6. 本机做了什么 / 没做什么

**做了（零 Slint 编译）**：

| 命令 | 结果 |
| :--- | :--- |
| `bash scripts/dev/cargo-local.sh fmt --all` | 通过（格式化全部 `.rs`） |
| `bash scripts/gates/run-gates.sh light` | **通过**：`cargo fmt --all --check` + 11 条红线守卫全绿 |
| 环境：`scripts/dev/local-env.sh`（集成者新增） | `run-gates.sh` 与 `cargo-local.sh` 会自动 source 它，沙箱里自动切 `CARGO_HOME` / `RUSTUP_TOOLCHAIN`，不必再手动 export |
| 教训 L6（集成者踩过，后来者注意） | **不要把门禁管道到 `tail`/`head`**：管道的退出码是 `tail` 的 0，fmt 失败会被吞掉并推上去。本文件记录的所有门禁结果都没有管道化 |
| `python3 scripts/gates/license_inventory.py` + `--check` | 生成后 `--check` 通过（"依赖许可清单与依赖图一致 (635 行)"）。任何改动依赖图的线都会撞到这条门禁，它是**预期**行为 |
| `bash scripts/dev/ci-verdict.sh line/ui-shell` | 判决**用项目自带工具读回**（不是靠 `gh run list` 猜），输出见第 8 节 |
| `gh run view --job <id> --log` | 第 1、2 轮变红时用它读**真实构建错误**（不是猜），据此定位到 fontconfig 构建脚本与许可清单漂移 |
| `cargo-local.sh metadata --format-version 1` | 通过；`Cargo.lock` 从 84 个包涨到 **602** 个包（+5774/−290 行） |
| `cargo-deny --all-features check`（用 `docs/DEV_WORKFLOW.md` 里的预编译二进制） | `advisories ok, bans ok, **licenses FAILED**, sources ok` —— 见第 8 节 |
| `rustc --edition 2024 --test`（把 `scene.rs` / `input.rs` / `elements.rs` 三个**纯 Rust** 模块单独编译执行） | **40 条测试全绿**；`-D warnings -W missing-docs` 零告警 |
| 自写 `.slint` 静态 linter（imports 解析 + `Tokens.*` 名存在 + 自定义元素名 + `UiText` import + `accessible-*` 与 `accessible-role` 共存 + **84 个实例化点的属性/回调名对账**） | 通过（linter 自身也做过变异测试：拼错属性名 / 拼错回调名都会变红） |
| `cargo tree --target x86_64-unknown-linux-gnu -e features -i yeslogic-fontconfig-sys` | 证实 `fontique/fontconfig-dlopen` → `yeslogic-fontconfig-sys/dlopen` 这条 feature 边在 Linux 目标上真的成立（不编译也能证明 feature 解析结果） |
| `cargo tree -p yeban-app -i yeslogic-fontconfig-sys`（host = macOS） | `nothing to print` —— 证实 fontconfig 只存在于 Linux 目标图，`[target.'cfg(target_os = "linux")']` 限定是必要的 |

**没做（纪律要求，一律交给 CI）**：

- `cargo clippy -p yeban-app` / `cargo test -p yeban-app` —— 加了 slint 之后它们会触发数分钟的高 CPU 编译；
- 任何 `--workspace` 全量构建、基准、模糊测试；
- 从未在本机链接/运行过含 Slint 的二进制，**从未在真实显示器上看过这个界面**。

---

## 7. 判据与变异测试证据（SKILL 规则 3：从没红过的判据是注释）

变异测试在 `/tmp` 的独立副本里做（不动仓库），每条都先"改坏 → 确认红 → 改回 → 确认绿"。

| 判据 | 变异操作 | 结果 |
| :--- | :--- | :--- |
| `input::tests::space_while_composing_never_plays`（判据 1） | 删掉 `Focus::TextInput` 的 `composing` 分支 | **红**：`space_while_composing_never_plays` / `composing_swallows_every_bare_shortcut` / `digits_are_swallowed_in_a_text_field` 三条 FAILED |
| `input::tests::tab_in_text_input_does_not_switch_view`（判据 2） | 把 `Key::Tab` 改成无视焦点直接切视图 | **红**：`tab_switches_view_only_on_the_main_canvas` FAILED |
| `elements::tests::console_tab_buttons_cover_every_declared_tab`（判据 3） | 注册表少登记一个控制台标签 | **红**：该测试 FAILED |
| `elements::tests::iteration_order_is_sorted_and_stable`（红线 4 的精神） | `BTreeMap` → `HashMap` | **红**：迭代顺序断言 FAILED |
| `elements::tests::slint_accessible_ids_and_registry_cover_each_other` | `status_bar.slint` 里 `"status-bar"` → `"statusbar"` | **红**：双向覆盖断言 FAILED |
| `scene::tests::compact_breakpoint_matches_the_spec_table` | （`UI-GRID-002`）1920/1919/1366/1365 四个边界逐个钉死 | 未做变异（边界值本身即判据） |

全部 40 条测试在本机实跑通过，且上面的变异全部变红后恢复绿。

---

## 8. CI 判决

### 第 1 轮：`9639ad3` → run [`37218095154`](https://github.com/gradetwo/yeban/actions/runs/37218095154) —— **failure**

| job | 结果 | 说明 |
| :--- | :--- | :--- |
| `plan` | success | 因为 `Cargo.lock` 是 ROOT_TRIGGER，受影响集合推导为 **workspace_wide**（全量） |
| `checks`（fmt / 红线守卫 / schema） | **success** | 格式与 11 条守卫本地与 CI 一致 |
| `lockfile`（`cargo metadata --locked`） | **success** | `Cargo.lock` 与全部清单一致 |
| `rust (yeban-app)` | **failure** | 见下 |
| 其余 24 条 `rust (<crate>)` 矩阵腿 | success | 与本工作线无关的 crate 全绿 |
| `deny` | **failure** | 见下 |

**失败 1（依赖构建脚本，非本 crate 代码）**：

```
error: failed to run custom build command for `yeslogic-fontconfig-sys v6.0.1`
  process didn't exit successfully: .../build-script-build (exit status: 101)
```

根因链（用 `cargo tree --target x86_64-unknown-linux-gnu` 实测）：
`slint` → `i-slint-common` → `fontique` → `yeslogic-fontconfig-sys`。后者的
`build.rs` 默认用 pkg-config 探测系统 fontconfig，探不到就 `unwrap()` panic：

```rust
// yeslogic-fontconfig-sys-6.0.1/build.rs
let dlopen = std::env::var_os("RUST_FONTCONFIG_DLOPEN").is_some();
if dlopen { println!("cargo:rustc-cfg=feature=\"dlopen\""); }
if !(dlopen || cfg!(feature = "dlopen")) {
    pkg_config::find_library("fontconfig").unwrap();   // ← 死在这里
}
```

GitHub 托管 runner 上没有 fontconfig 的开发包。**这是本项目第一次真的编译 Slint**
（`spikes/` 里那 9 个 spike 的 slint 依赖都还停在 `TODO(spike)` 注释里，CI 从没编译过），
所以也是第一次撞上这个 Linux 构建前置条件。

**处置（集成者裁决 = 方案 B：由 CI 提供系统库）**

第 1 轮的处置初稿是在 `crates/yeban-app/Cargo.toml` 里加一段 Linux-only 的 feature 合并垫片
（`fontique = { version = "0.11.1", features = ["fontconfig-dlopen"] }`，即把
`yeslogic-fontconfig-sys` 从"链接 libfontconfig"切成"运行时 dlopen"）。**该垫片已按集成者裁决删除**：

- 集成者在 `main` 的 `.github/workflows/ci.yml` 的 rust 矩阵腿加了 apt 步骤：
  `pkg-config libfontconfig1-dev libfreetype-dev libxkbcommon-dev libwayland-dev libx11-dev
  libgl1-mesa-dev`（并**提前**加了 `libasound2-dev`，避免 Phase 2 的 cpal 线再踩同一个坑）。
  于是上游默认的"链接 libfontconfig"路径可以正常工作，不需要改链接方式。
- 删除理由（集成者裁决，本工作线认同）：① `fontique` 是 slint 的传递依赖，提升为直接依赖并钉版本
  会在 slint 升版时变成版本冲突/重复版本，属于"为绕开环境问题引入长期维护风险"；
  ② 本项目唯一允许写版本号的地方是根 `[workspace.dependencies]`，成员里写显式版本是例外，
  不该为一个环境垫片动用；③ Linux 上 dlopen 还是链接是**发行方式**问题，应当由人类开 ADR，
  不该藏在一条 UI 线的提交里。
- 教训：垫片当时能让 CI 真的编译（"交付物从未被编译过"的顾虑是真实的），但**环境前置属于环境**，
  正确落点是 CI 的系统依赖安装，而不是产品依赖图。

分支已 rebase 到含修复的 `main`（`deny.toml` 放行 BSL-1.0 + CI 装系统库 + 两条引擎线合并），
并按裁决删除了 fontconfig 垫片。两次 rebase 都把 `Cargo.lock` 让给 main 侧，随后用
`cargo metadata` 重新生成为与本分支清单一致（已验证 `cargo metadata --locked` 通过）——
否则 `lockfile` job 会红。

### 第 2 轮：`953b85b` → run [`37218433961`](https://github.com/gradetwo/yeban/actions/runs/37218433961) —— **failure（但只剩 1 条）**

| job | 结果 |
| :--- | :--- |
| `plan` / `lockfile` / `deny` | **success**（`deny` 因集成者放行 BSL-1.0 而通过） |
| **`rust (yeban-app)`** | **success** —— **`.slint` 编译通过、`slint_build` 生成代码可用、`main.rs` 对接生成 API 编译通过、clippy `-D warnings` 零告警、40 条单元测试全绿** |
| 其余 25 条 rust 矩阵腿 | success |
| `checks` | **failure** —— 见下 |

**这是本次最重要的一条结论**：规范里最不确定的三件事（`.slint` 语法、`slint::include_modules!()`
放在嵌套模块里、`ComponentHandle::run` 等生成 API 的实际签名）**全部被 CI 证实**。
本文件 §2 里那些"未核验"的风险项中的第 1、2 条据此关闭。

**`checks` 的失败原因（与本工作线代码无关，是新增的门禁）**：

```
依赖许可清单已漂移: docs/ledger/dependency-licenses.md 与真实依赖图不一致。
请运行 `python3 scripts/gates/license_inventory.py` 并提交结果。
```

集成者新增了 `scripts/gates/license_inventory.py --check`：它要求
`docs/ledger/dependency-licenses.md`（机器生成的依赖许可清单，内含 `Cargo.lock` 的 SHA-256）
与真实依赖图一致。本工作线引入 slint 依赖树后清单必然漂移 —— 这是该门禁**预期**的行为
（"加了依赖却没重新生成清单" 正是它要抓的）。已在本地重新生成并提交：
外部依赖 **579** 个，其中 slint 系 14 个包是
`GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0`（取 GPL-3.0-only 分支），
另 2 个是 BSL-1.0（`clipboard-win` / `error-code`）。
该生成器**只写** `docs/ledger/dependency-licenses.md` 这一个文件，不碰
`THIRD_PARTY_LICENSES.md`（后者是集成者独占的人工归属文件）。

**失败 2（根级法务策略，本工作线无权改）**：

```
advisories ok, bans ok, licenses FAILED, sources ok
error[rejected]: failed to satisfy license requirements
  clipboard-win v5.4.1  license = "BSL-1.0"
  error-code v3.4.0     license = "BSL-1.0"
    └── arboard v3.6.1 └── i-slint-backend-winit └── i-slint-backend-selector
        └── slint └── yeban-app
```

`clipboard-win` / `error-code` 是 Windows-only 的 crate，但 `cargo-deny` 默认对
**全平台目标**建图，而 `[licenses] allow` 里没有 `BSL-1.0`。修法是 `deny.toml` 的
`allow` 列表加一行 `"BSL-1.0",`。`deny.toml` 是根级共享文件，且"接纳一个新许可"属于
`AGENTS.md` §2 红线 2 / `ROAD-M-1-006` 的人类法务判断，Agent 不代签 —— 因此**只报告不动手**。

**集成者裁决**：已在 `main` 的 `deny.toml` 白名单加入 `"BSL-1.0"`，并把来源链路与判定依据
写进注释；同时把它列为 **ADR-0001 D17（Proposed）**交人类批准（否决时回滚该行）。
本工作线提出"接纳新许可属于人类判断"这一点被接受，作为政策异议记录在案。
集成者另给出更省事的判定办法：改白名单前先用 `cargo metadata --locked --format-version 1`
枚举**全部**外部包的 license 逐条比对（他在本分支上枚举了 579 个包，真正被拒的只有 BSL-1.0）。

### 第 3 轮：`05b2a1d` → run [`37218791723`](https://github.com/gradetwo/yeban/actions/runs/37218791723) —— **success**

判决用项目自带工具读回（`bash scripts/dev/ci-verdict.sh line/ui-shell`）：

```
✓ line/ui-shell CI · 37218791723
✓ checks (fmt / 红线守卫 / schema) in 47s
✓ deny (cargo-deny 开源合规) in 42s
✓ plan (受影响集合) in 5s
✓ lockfile (确定性 Cargo.lock) in 16s
- rust (${{ matrix.crate }}) in 0s      ← **被 plan 跳过**
```

**必须如实说清的一点**：这一轮 `rust` 矩阵腿是**被跳过**的，不是"跑过且绿"。
原因是 `scripts/dev/changed-crates.py` 的受影响集合推导：`05b2a1d` 相对 `953b85b`
**只改了两个 markdown**（`docs/ledger/dependency-licenses.md`、`docs/ledger/ui-shell-notes.md`），
而 `docs/` 不在 `ROOT_TRIGGERS` 里，于是 plan 得出"无受影响 crate"，矩阵腿按设计跳过。
实测核对（`git diff --name-only 953b85b..05b2a1d` + 对 `crates/ Cargo.toml Cargo.lock scripts/ .github/`
做 `git diff --stat`）：**代码逐字节相同**，没有任何 crate 源码差异。

因此本分支的完整判决账目是：

| 事实 | 证据 |
| :--- | :--- |
| Slint UI 在 Linux 上真的能编译（clippy `-D warnings` + 40 条测试） | run **37218433961** @ `953b85b`，job `rust (yeban-app)` = **success** |
| 依赖许可清单门禁通过（`license_inventory.py --check`） | run **37218791723** @ `05b2a1d`，job `checks` = **success** |
| 分支 tip 整体判决 | run **37218791723** = **success** |
| 其余 crate | run 37218433961 的 25 条 rust 矩阵腿全 success |

即：**代码的编译/测试判决落在 `953b85b`（那一轮唯一红的是 `checks`，与代码无关），
`checks` 的判决落在 `05b2a1d`（那一轮 rust 腿按设计跳过）。两轮合起来覆盖了全部 job。
没有出现"某一轮里代码和门禁同时绿"的单点，但两者的代码版本完全相同。**

### 第 4 轮：`a03c792`（本文件自身的 docs-only 提交）→ run [`37218916188`](https://github.com/gradetwo/yeban/actions/runs/37218916188) —— **success**

```
✓ plan in 6s   ✓ checks in 48s   ✓ deny in 40s   ✓ lockfile in 16s
- rust (${{ matrix.crate }}) in 0s     ← 同样被 plan 跳过（docs-only）
```

**账目到此为止（显式终止，不再无限回填）**：任何"只为回填上一轮 CI 判决"的提交本身也是
docs-only，因此会重复同一模式（`rust` 腿被跳过、其余四条门禁照跑）。所以：

- **代码的编译/测试判决** = run 37218433961 @ `953b85b`，`rust (yeban-app)` = success；
- **门禁（checks / deny / lockfile / plan）判决** = 此后每一轮 docs-only 提交都 success，
  最近三轮：37218791723 @ `05b2a1d`、37218916188 @ `a03c792`，以及记录本条的这一轮。
- 两者对应的代码版本完全相同（`git diff --name-only 953b85b..a03c792` 只有 `docs/ledger/*.md`）。

如果集成者希望看到 tip 上的一次**全量**绿（rust 腿也真跑而不是被受影响集合跳过），
`.github/workflows/ci.yml` 的 `workflow_dispatch` 有 `force_full` 输入 —— 那是手动档，由集成者决定。

---

## 9. needs（需要别人做）

1. ~~`deny.toml` 加 `BSL-1.0`~~ —— **已由集成者在 `main` 处置**（白名单放行 + 列为 ADR-0001 D17
   Proposed 交人类批准）。保留本条是为了留下判定依据与来源链路。原始记录：
   实测：Slint 的 winit 后端经 `arboard` 引入 `clipboard-win v5.4.1` 与 `error-code v3.4.0`，
   两者许可都是 `BSL-1.0`（Boost Software License 1.0：OSI approved + FSF Free/Libre，
   宽松、与 GPLv3 兼容），但不在 `deny.toml` 的 `[licenses] allow` 白名单里。
   补法：在 `allow` 列表里加一行 `"BSL-1.0",`。
   不加的话 CI 的 `deny` job 会红，而那不是本工作线代码的问题。
   （是否接纳 BSL-1.0 属于 `AGENTS.md` §2 红线 2 / `ROAD-M-1-006` 的人类法务判断，
   Agent 不代签，因此只报告不擅改。）
2. **人类裁决：主题层的间距/圆角/字号 scale**（本工作线首次登记，数值见 `tokens.slint`）；
   以及浅色主题的运行时切换机制。
3. **人类裁决：`< 1366` 宽的断点行为**（规范未定义；本实现取保守解：仍折叠）。
4. **人类裁决：`SLINT_BACKEND=headless` 的处理方式**——保留为 yeban 自研哨兵值（本实现），
   还是改成规范里写 `SLINT_BACKEND=winit-software` 之类上游真值。
5. **`yeban-ui-test-port` 线**：无头控件树断言 + 截图比对需要 `i-slint-backend-testing`
   （`ARCH-UI-005`）。本工作线**没有**给它留 IDE 端口 / JSON-RPC，那是它的地盘。
6. ~~人类裁决：Linux 上的 fontconfig 链接方式~~ —— **已裁决：方案 (B) 照常链接**。
   集成者在 `main` 的 CI rust 矩阵腿加了 apt 步骤（`pkg-config libfontconfig1-dev
   libfreetype-dev libxkbcommon-dev libwayland-dev libx11-dev libgl1-mesa-dev`，
   并提前加了 `libasound2-dev`），本工作线已**删除**那段 `fontique/fontconfig-dlopen` 垫片。
   保留本条是为了留下"环境前置属于环境，不属于产品依赖图"这条裁决依据。

---

## 10. pending（未证实的、已知的债）

1. **"能编译"未证实**：本机禁止编译 slint，`.slint` 语法、`slint_build` 生成代码、
   `main.rs` 对接生成 API 全部只经过文档核验 + 静态 linter。**编译与测试由 CI 判定。**
2. **从未在真实显示器上看过界面**：没有截图、没有 SSIM、没有控件树断言。无头路径故意不构造窗口。
3. **无任何真实绑定**：全部是演示数据。走带 / Op 归约 / AI 采纳 / 撤销栈 / 电平 SPSC（`ARCH-UI-002`）/
   视口裁剪与 R-Tree（§3.1）/ 960 PPQ 吸附（§3.2）/ 指针捕获状态机（§6.1）/ 拖拽 splitter（`UI-GRID-002`）
   / 双重 rid 焦点闭环（`UI-A11Y-003`）均未实现。回调只打一行 stderr，**故意**不做本地状态翻转
   （假实现比空实现更危险）。
4. **演示 ULID / 轨道名在两处重复**：`ui/workspace/arrangement_view.slint` 与
   `ui/console/piano_roll.slint` 内联了一份与 `src/scene.rs` 相同的常量数组 —— 因为 Slint 数组属性
   需要默认值才能独立渲染。模型线落地后改成 Rust 侧 `ModelRc` 单向注入，`.slint` 里只留空数组。
   当前的防漂移手段是 `elements.rs` 的 `ulid_families_match_the_scene_constants` 与双向 ID 契约判据。
5. **数组属性未从 Rust 注入**：`main.rs` 只设标量属性（bool/string/int）。`[string]` / `[float]`
   属性要用 `ModelRc<VecModel<_>>`，在无法本地编译的前提下风险偏高，故留到模型线。
6. **`accessible-id` 在重复元素上的唯一性**：规范要求 `for` 循环里每个实例给不同的 `accessible-id`
   （官方例子 `"btn-" + i`），本实现全部用索引/ULID 拼接，因此**结构上**唯一；
   但"运行时确实唯一"没有实测（需要窗口实例）。
7. **`AiRail` 被实例化两次**（常驻列 + compact 抽屉），两者 `visible` 条件互斥，
   因此无障碍树里同一时刻只有一个 `ai-rail`；这一点没有实测。
8. **`ARCH-UI-004` 的内省服务（`127.0.0.1` + 会话 Token + 三级权限）未实现**，
   属于 `yeban-ui-mcp` 线；本工作线只提供 `--dump-elements` 的离线清单。
9. **未启用 `renderer-skia`**：规范 §12.1 提到的 Skia 软件后端会拖入 LLVM/clang；本次用
   `renderer-software` 兜底。若 Tier 1 视觉回归（`UI-MCP-003`）要求 Skia，需要单独裁决。
10. ~~`deny` job 的预期红~~ —— **已解决**：集成者在 `main` 放行 `BSL-1.0` 后，
    `deny` 在 run 37218433961 与 37218791723 都是 success。
11. ~~Linux 构建前置条件~~ —— **已由集成者在 CI 统一加装系统依赖解决**（含提前装好的
    `libasound2-dev`，为 Phase 2 的 cpal 线省一次失败）。保留本条是为了让后来者知道
    "本项目第一次真正编译 Slint 时死在哪"，以及**系统库前置应当落在 CI，不要落在产品依赖图里**。
