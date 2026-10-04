# ui-test-port 工作线备注 (API 核验 · 判据 · 缺口 · 待决问题)

- **工作线**: `line/ui-test-port`（worktree `.worktrees/ui-test-port`）
- **日期**: 2026-10-05
- **范围**: `crates/yeban-ui-test-port/**`（主力）+ 仅两个被明确授权的 app 侧改动：
  新增 `crates/yeban-app/src/test_port_adapter.rs`、在 `crates/yeban-app/Cargo.toml` 里加
  `ui-test-port` feature（默认关闭）+ 可选依赖。**未改** `yeban-app` 的 `.slint` / `main.rs` /
  `input.rs` / `elements.rs` / `lib.rs`。
- **本机纪律**: 全程**未编译 Slint**。与 Slint 无关的 7 个模块在本机用 `rustc --test` 真跑过
  （§7）；任何"能编译"的结论都由 CI 判。

---

## 1. 交付物 ↔ 规范 ID 映射

| 交付物 | 规范 ID | 说明 |
| :--- | :--- | :--- |
| `src/tree.rs` | `UI-TEST-001` `UI-MCP-001` `UI-MCP-002` | 语义控件树模型（ID/角色/标签/包围盒/动态区）+ 稳定 JSON + 双向覆盖 + 动态标记继承 |
| `src/render.rs` | `ARCH-SLINT-001` `ROAD-M0-008` `ROAD-M3-007` `MUST-GATE-015` `UI-TEST-002` `UI-MCP-001` | Tier-1 软件光栅化截图 + `GoldenEvidence`（尺寸非零/非全黑）+ `LivePort` |
| `src/mask.rs` | `UI-MCP-002` | 动态区域置黑 + "遮罩确实生效"自检 |
| `src/ssim.rs` | `UI-MCP-003` | mean SSIM（7×7 均匀窗，C1/C2 标准值，默认阈值 0.98） |
| `src/port.rs` | `UI-MCP-001` `UI-TEST-002` | 三级权限（纯函数 `authorize`）+ §12.4 的调用面 + 权限闸门默认方法 |
| `src/inspect.rs` | `ARCH-UI-005` `UI-TEST-001` | `ElementHandle` → 控件树；角色映射；属性投影 |
| `src/png.rs` | `UI-MCP-003` `MUST-GATE-015` | 零依赖 PNG（stored deflate），字节确定性 |
| `src/image.rs` | `MUST-GATE-015` `UI-MCP-002` | RGB8 图像 / `Rect` / 非全黑判定（其余模块的零 Slint 底座） |
| `src/golden.rs` | `UI-MCP-003` | `tests/golden/<platform>/<name>.png` 约定 + 跨平台混用检查 |
| `src/lib.rs` | 全部 | crate 边界、依赖方向、模块地图、`pub use`、顶层契约判据 |
| `crates/yeban-app/src/test_port_adapter.rs` | `UI-TEST-001` `UI-MCP-002` `MUST-GATE-015` | app 注册表 → 控件树的适配器 + **真的把 13 个 `.slint` 渲染一次**的端到端判据 |
| `crates/yeban-app/Cargo.toml` | ADR-0001 D18 | `ui-test-port` feature（默认关闭）+ 可选依赖 + `[[test]]` 目标 |

---

## 2. 上游 API 核验记录（写代码前逐条对过**源码**/文档）

> 方法：先读 `$CARGO_HOME/registry/src/.../i-slint-*-1.18.1/` 里的**实际源码**
> （本机缓存里就有 1.18.1 的全部 `i-slint-*` 源码），再用 docs.rs 交叉核对。
> 这比只读文档可靠：文档里 `#[doc(hidden)]` 与 `#[non_exhaustive]` 的东西常常看不见。

| # | 结论 | 出处 |
| :-- | :--- | :--- |
| 1 | `slint::platform::Platform` 在 1.18.1 **只有一个必需方法** `create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError>`；其余 9 个（`run_event_loop` / `new_event_loop_proxy` / `duration_since_start` / `click_interval` / `cursor_flash_cycle` / `set_clipboard_text` / `clipboard_text` / `debug_log` / `open_url`）都有默认实现。**自研平台不需要实现事件循环** | <https://docs.rs/slint/1.18.1/slint/platform/trait.Platform.html> + `i-slint-core-1.18.1/platform.rs:28-46` |
| 2 | `set_platform(Box<dyn Platform + 'static>) -> Result<(), SetPlatformError>`；**已设置过则 Err**。`SetPlatformError` 在 `slint::platform::SetPlatformError`（`i-slint-core-1.18.1/platform.rs:239`） | <https://docs.rs/slint/1.18.1/slint/platform/fn.set_platform.html> |
| 3 | `MinimalSoftwareWindow::new(RepaintBufferType) -> Rc<Self>`；`draw_if_needed(&self, impl FnOnce(&SoftwareRenderer)) -> bool`（**只有需要重绘时才调回调**） | `i-slint-renderer-software-1.18.1/minimal_software_window.rs:24,39` |
| 4 | `SoftwareRenderer::render(&self, &mut [impl TargetPixel], pixel_stride: usize) -> PhysicalRegion` | `i-slint-renderer-software-1.18.1/lib.rs:538` |
| 5 | **`slint::Rgb8Pixel` 存在**：`pub type Rgb8Pixel = Rgb<u8>`（字段 `r/g/b`，`#[repr(C)]`），并且 `impl TargetPixel for rgb::Rgb<u8>` 是上游提供的 —— 所以 `render(&mut [Rgb8Pixel], stride)` 直接可用 | <https://docs.rs/slint/1.18.1/slint/type.Rgb8Pixel.html> + `i-slint-renderer-software-1.18.1/draw_functions.rs:861` |
| 6 | **`slint::platform::software_renderer` 模块里没有 `Rgb8Pixel`**（只有 `Rgb565Pixel` / `Rgb565BigEndianPixel` / `PremultipliedRgbaColor`）；`Rgb8Pixel` 在 **crate 根**。任务书里写的"`Rgb8Pixel` 的确切签名"要按 crate 根那个类型别名理解 | <https://docs.rs/slint/1.18.1/slint/platform/software_renderer/index.html> |
| 7 | `SharedPixelBuffer<Rgb8Pixel>::new(w,h)` / `.width()` / `.make_mut_slice()` / `.as_bytes()`（`as_bytes` 需要 `Pixel: Pod + ComponentBytes<u8>`，`Rgb<u8>` 满足） | `i-slint-core-1.18.1/graphics/image.rs:71-110` |
| 8 | `RepaintBufferType::NewBuffer` = *"The full window is always redrawn. No attempt at partial rendering will be made."* —— Golden 要的就是它（`ReusedBuffer` 会只画脏区，第二次截图可能是空的） | <https://docs.rs/slint/1.18.1/slint/platform/software_renderer/enum.RepaintBufferType.html> |
| 9 | `WindowEvent`（`i-slint-core-1.18.1/platform.rs:367`）：`PointerPressed{position,button}` / `PointerReleased{position,button}` / `PointerMoved{position}` / `PointerScrolled{..}` / `KeyPressed{text}` / `KeyPressRepeated{text}` / `KeyReleased{text}` / … | 上游源码 |
| 10 | `Window::dispatch_event(&self, WindowEvent)`（出错 panic）；**`try_dispatch_event` 已 deprecated**（`-D warnings` 下会红），要"带结果"的版本用 `dispatch_event_with_result` | `i-slint-core-1.18.1/api.rs:633,659` |
| 11 | `Platform::create_window_adapter` 返回**同一个** `Rc<MinimalSoftwareWindow>` 是标准做法；上游自己的回归测试就是这么驱动软件窗口的：`msw.window().request_redraw(); msw.draw_if_needed(|r| { r.render(buffer.make_mut_slice(), stride) })` | `i-slint-renderer-software-1.18.1/minimal_software_window.rs:111-133` |
| 12 | **平台是线程局部的**。上游注释原文：*"Each test runs on its own thread, so the thread-local global context is unset here."* ⇒ 每个测试线程只能 `set_platform` 一次；`cargo test` 一个测试一个线程，所以"一个大测试里做完所有 Slint 动作"是正确用法 | `i-slint-renderer-software-1.18.1/minimal_software_window.rs:141-142` |
| 13 | `i-slint-backend-testing` 的公开入口**不是**规范写的 `slint::testing::init_integration_test_backend()`，而是：`init_no_event_loop()` / `init_integration_test_with_mock_time()` / `init_integration_test_with_system_time()` / `mock_elapsed_time()` / `get_mocked_time()` / `set_system_accent_color()` / `configure_test_fonts()`（**后者在 `internal` feature 后面**）。`init_no_event_loop` 的文档明说"每个测试线程可以用自己的后端" | `i-slint-backend-testing-1.18.1/lib.rs:37,56,72,86,103` <https://docs.rs/i-slint-backend-testing/1.18.1/i_slint_backend_testing/> |
| 14 | `ElementHandle` 的**只读**方法：`accessible_id()` / `accessible_label()` / `accessible_role()` / `id()` / `type_name()` / `bases()` / `size()` / `absolute_position()` / `computed_opacity()` / `is_valid()` / `query_descendants()` / `visit_descendants()` / `find_by_element_id()` / `find_by_element_type_name()`；**没有** `find_by_accessible_id`（按语义 ID 找要自己用 `query_descendants().match_predicate(...)`） | `i-slint-backend-testing-1.18.1/search_api.rs:320-960` |
| 15 | `visit_descendants` 只访问**后代**（不含自身），且**跳过几何不可见的元素**：`is_visible()` 的实现是"绝对裁剪矩形与元素几何相交"（`i-slint-core-1.18.1/item_tree.rs:410-419`），**不是**读 `visible` 属性 | 上游源码 |
| 16 | `ElementRoot` 对**任何** `ComponentHandle` 有 blanket impl（`search_api.rs:115`），且只依赖 `WindowInner::from_pub(window).component()` ⇒ **只读遍历与平台类型无关**，可以在 Tier-1 软件平台下用（本线的 `LivePort` 就是这么做的：同一个实例同时给控件树**和**像素） | `i-slint-backend-testing-1.18.1/search_api.rs:103-120` |
| 17 | `AccessibleRole` 在 1.18.1 共 **30** 个取值（含 8 个 landmark 与 `search`），`#[non_exhaustive]`；**上游没有运行时的 enum→kebab-case 字符串 API**（kebab 化发生在编译期 `i-slint-compiler-1.18.1/builtin_elements.rs:79` 的 `kebab()`），因此本 crate 自己映射（`inspect::role_name`）并用判据把它钉在 `tree::KNOWN_ROLES` 上 | `i-slint-common-1.18.1/enums.rs:466-530` |
| 18 | `slint` 的 `compat-1-18` 是**强制**的：`slint-1.18.1/lib.rs:209-213` 有 `compile_error!`；`compat-1-2 = ["compat-1-18"]` ⇒ 本 crate 的 `slint` feature 里必须写 `compat-1-2` | 上游源码 |
| 19 | **`accessibility` feature 不是 `accessible-id` 的前提**：`ElementHandle::accessible_id()` → `ItemRc::accessible_string_property(Id)`，而 `i-slint-core-1.18.1/item_tree.rs` 里这条路径**没有任何 `cfg(feature = "accessibility")` 门**。因此本 crate **不启用** `accessibility`（省掉整个 accesskit 依赖树） | 上游源码（`grep 'cfg(feature = "accessibility")' item_tree.rs` = 0 命中） |
| 20 | `slint!` 宏（`pub use slint_macros::slint;`，proc macro）：**输入要过 Rust 词法器**，因此 `#0b…` 与 `#<digits>e<非十六进制>` 两种颜色字面量会**编译不过**；`px` 之类的单位后缀没问题。本线的内联夹具一律用 `rgb(r, g, b)` 绕开整类风险 | `slint-macros-1.18.1/lib.rs:360-378` |
| 21 | `configure_test_fonts()`（`internal` feature）会把字体集合换成**内嵌 NotoSans**（`system_fonts: false`），并且它改的是 `ctx.font_context()` —— **与软件光栅化共用同一个 font context**。也就是说：**理论上可以让 Tier-1 截图也变成字体确定的**，从而跨平台基准图可比 | `i-slint-backend-testing-1.18.1/lib.rs:101-150` |
| 22 | **`ElementHandle::accessible_id()` 对 `element_index != 0` 直接返回 `None`**（`search_api.rs:745-748`）。而 `visit_descendants` 会对每个 `ItemRc` 展开 `element_count` 个句柄（`collect_elements`）。`for` 循环展开的重复元素到底走"独立子 ItemTree"（每个实例 `element_index == 0`，语义 ID 可用）还是"单 ItemRc + element_count"（只有第一个实例有语义 ID），**从源码读不出来**（`generate_repeated_component` 给重复元素建了 `sub_tree`，但 `element_count` 的行为取决于是否原生重复项）。⚠️ **这直接决定 `[UI-TEST-001]` 对 `note-{ulid}-rect` / `clip-{ulid}-header` / `track-{i}-header` 是否真的可用** | `i-slint-backend-testing-1.18.1/search_api.rs:330-345,745-748`；`i-slint-compiler-1.18.1/generator/rust.rs:2719-2726` |
| 23 | 元素没有声明 `accessible-id` 时 `accessible_id()` 返回 **`None`**（不是 `Some("")`）：生成的 `accessible_string_property` 只为**真正声明过** `accessible-*` 的元素生成分支，其余落到 `_ => None` | `i-slint-compiler-1.18.1/generator/rust.rs:1524-1540,2016-2019` |
| 24 | ⚠️ **`slint!` 是 proc macro，它的每一条编译器 warning（甚至 note）都会被展开成 `#[deprecated] const WARNING: () = (); WARNING`** ⇒ CI 的 `-D warnings` 会把"善意提示"变成硬错误。实测：第一次 CI 就死在 `Exported component 'PortFixture' doesn't inherit Window. This is deprecated` 上（`clippy -D warnings`）。**这个机制只作用于 `slint!`**：`slint_build`（`.slint` 文件路径）把同一批诊断当 cargo warning 打印，所以 `yeban-app` 的 13 个 `.slint` 不受影响 | `i-slint-compiler-1.18.1/diagnostics.rs:575-594`；`passes/check_public_api.rs:63` |
| 25 | `export component X inherits Window` 才是不触发上述告警的形状 —— `yeban-app/ui/app.slint:154` 的 `MainWindow` 正是如此。本线的夹具因此改成 `inherits Window` | 仓库内 `crates/yeban-app/ui/app.slint` |
| 27 | ⚠️ **`ElementHandle` 的遍历要求被内省的 `.slint` 在编译期带 debug info，而它默认是关闭的**：`let debug_info = std::env::var_os("SLINT_EMIT_DEBUG_INFO").is_some();`。没有它时 `item.element_count()` 返回 `None` ⇒ `visit_descendants` 一个元素都访问不到 ⇒ **控件树恒为空**（本线实测：CI run 37221680724 打印 `ControlTree { nodes: {} }`）。开法二选一：`slint_build::compile_with_config(.., CompilerConfiguration::new().with_debug_info(true))` 或构建期设 `SLINT_EMIT_DEBUG_INFO=1`。**`slint!` 内联宏只能走环境变量那条**，所以本线的夹具改成了 `.slint` 文件 + `build.rs`（见 §2 第 28 条） | `i-slint-compiler-1.18.1/lib.rs:282`；`i-slint-backend-testing-1.18.1/search_api.rs:62`；实测 run 37221680724 |
| 28 | 因此本线的**夹具形态**是工程裁决而不是偏好：`crates/yeban-ui-test-port/ui/fixture.slint` + `build.rs`（`compile_with_config(..with_debug_info(true))`）+ `include_modules!()`，而**不是** `slint::slint!`。三条理由：① 宏无法打开 debug info（第 27 条）；② 宏的每条编译器警告都会变成 `#[deprecated]`，在 `-D warnings` 下是硬错误（第 24 条）；③ `.slint` 文件不过 Rust 词法器，可以写正常的 `#rrggbb` | 本仓库 `crates/yeban-ui-test-port/build.rs` |
| 26 | **Cargo 会给"只有 path、没有 version"的依赖隐式补 `*`** ⇒ `deny.toml` 的 `[bans] wildcards = "deny"` 报 `error[wildcard]`。实测：第二次 CI 的 `deny` job 死在这里。**本机 `run-gates.sh light` 的 G10 守卫抓不到**（它只看字符串形式的 `*`），所以这是一条"只有 CI 能抓"的坑 —— 已在 notes §7.0 记为本机探针抓不到的类别，靠本机 cargo-deny 补 | `deny.toml:69-71`；CI run 37221429630 的 deny job |

### 规范 vs 上游 1.18.1：本线**新增**的三处发现（补 ADR-0001 D18）

ADR-0001 D18 已记录 `SLINT_BACKEND=headless` 不存在、`slint::testing::*` 不存在。本线再补三条：

1. **`Rgb8Pixel` 的位置**：UI/UX §12.1 与 `[ARCH-SLINT-001]` 只写"`SoftwareRenderer` + Framebuffer"，
   没有指明像素类型；实际上 `Rgb8Pixel` **不在 `software_renderer` 模块里**（那里只有 RGB565），
   而在 `slint` crate 根。照 `software_renderer::Rgb8Pixel` 写会编译失败。
2. **`i-slint-backend-testing` 的入口名**：§12.1 与 `ARCH-UI-005` 写的 `init_integration_test_backend()`
   不存在，可用的是 `init_no_event_loop()`（每线程一次）等四个入口。
3. **Testing Backend 的遍历语义**：`visit_descendants` 是**几何可见性过滤**后的集合
   （`is_visible()` = 裁剪矩形相交），不是"声明过的元素全集"。因此
   **"运行时控件树 ⊆ 静态注册表"是能成立的强判据；反过来"注册表 ⊆ 运行时树"只在当前可见分支上成立**
   （默认视图下会话视图画布等元素根本不在树里）。这一点直接决定了 app 侧判据的形状（见 §6）。

| 29 | ⚠️ **SSIM（7×7 均匀窗、C1/C2 标准值）对两类变化几乎免疫**，这直接影响 `[UI-MCP-003]` 的阈值能不能当门禁用（本机用真实实现量化过）：<br>· **细长条**：8×88 的竖条整条换成纯色 ⇒ 未遮罩 SSIM = **0.9999**；<br>· **等亮度换色**：60×88 的块（22% 画面）换成亮度相近的平色 ⇒ 未遮罩 SSIM = **0.9967**；<br>· **亮度差大的成块变化**：同一块掉到黑 ⇒ 未遮罩 SSIM = **0.7040**；<br>· 抹掉一个 64×40 的平色块 ⇒ **0.8481**。<br>结论：0.98 阈值**不是**"任何看得见的差异都会红"。规范 §12.5 那句"关键静态视觉缺陷（如元素缺失、布局错位）可 100% 灵敏检出"**需要限定**：只有当变化带来足够的**局部亮度/对比度改变**时才成立。DAW 界面是大量纯色面板 ⇒ 这条限制是真实的（元素搬到等亮度的另一块纯色上，SSIM 可能察觉不到）。**对策**（本线采纳）：判据里"必须被检出"的那一类改动，必须挑**成块 + 亮度差大**的变化（本线夹具用"RTA 掉到地板(黑)"），扁平/窄条类变化只用于"遮罩必须吸收"的方向 | 本机预演：`/Users/crow/work/music/.uitp-harness/sim_final.rs`（用仓库里的 `ssim.rs` 真跑）；实现口径见 `src/ssim.rs` 模块文档 |

### 有意**未**采纳的上游能力（附理由）

- `configure_test_fonts()` / `internal` feature：它能让截图字体跨平台确定，是**削弱 `[UI-MCP-003]`
  分平台 Golden 要求**的一条路。但（a）它是官方**内部** crate 的 `internal` feature，
  （b）"要不要放弃分平台基准"是规范级裁决，不是一条 UI 线能定的。**只报告不动手**，见 §10 needs。
- `render_by_line`：本机内存足够整帧渲染，`render()` 更简单也更少一处出错可能。
- `Window::take_snapshot()`：能直接给 `Rgba8`，但它对 `MinimalSoftwareWindow` 的行为上游没有承诺
  （文档只说"may need to re-render"），而 `draw_if_needed + render` 有上游自己的回归测试背书。

---

## 3. SSIM 口径（写死，不许"大致等价"）

| 项目 | 取值 | 依据 |
| :--- | :--- | :--- |
| 灰度 | `Y = 0.299R + 0.587G + 0.114B` | ITU-R BT.601-7；与 `skimage.color.rgb2gray` 同口径 |
| 窗口 | **7×7 均匀窗**，步长 1 | `skimage.metrics.structural_similarity` 默认 `win_size=7` + `gaussian_weights=False` |
| 边界 | **valid 卷积**（不做 padding） | 各库 padding 策略不同，会引入"平台无关但版本相关"的抖动；valid 最可复现 |
| 局部统计 | `μ=Σx/N`，`σ²=Σx²/N−μ²`，`σxy=Σxy/N−μxμy`，`N=49` | Wang et al. 2004, IEEE TIP 13(4), §III 式 (13) |
| 常数 | `C1=(0.01·255)²`, `C2=(0.03·255)²` | 同论文式 (13) 的 `K1=0.01, K2=0.03, L=255` |
| 汇总 | 逐窗 SSIM 的算术平均（MSSIM） | 同论文 §III.C |
| 阈值 | **0.98** | `[UI-MCP-003]` UI/UX §12.5 原文 |

- 实现：5 条列和的滑窗（`O(宽×高)` 时间、`O(宽)` 内存），全程 `f64`、运算顺序固定
  ⇒ 同一输入**逐位可复现**（有判据钉住）。
- 与"直译论文式"的朴素实现做过对账：**不要求逐位相同**（滑窗重排了求和顺序，浮点加法不满足结合律），
  口径是 `|差| < 1e-9`。逐位相同只作为"同一实现重复调用"的判据。
- 出处 URL：<https://doi.org/10.1109/TIP.2003.819861>、
  <https://scikit-image.org/docs/stable/api/skimage.metrics.html#skimage.metrics.structural_similarity>、
  <https://scikit-image.org/docs/stable/api/skimage.color.html#skimage.color.rgb2gray>。
- **选择 7×7 均匀窗本身是工程裁决**：一旦选定就写进 `ssim.rs` 的模块文档与这里，
  阈值判据把它钉死，避免"换窗口凑绿"。

---

## 4. PNG 决策：自己写（stored deflate），不加 `png` crate

三条**可核验**的理由（不是"少个依赖更优雅"）：

1. **字节确定**：stored deflate 的字节完全确定（无 zlib 版本/优化级别变量），
   于是"PNG 的哈希"可以当稳定标识用（`render::GoldenEvidence::fingerprint`）。
   引入 `png`/`flate2` 会让压缩字节随版本变化，最便宜的那种判据就失效了。
2. **零许可与零依赖图漂移**：本线已经因为引入 `slint` / `i-slint-backend-testing` 必须重生成
   一次依赖许可清单；没有理由再加一层。
3. **可被独立解码器验证**：stored deflate 的 IDAT 能被标准 zlib `inflate` 解开 ⇒
   "我们写的是合法 PNG"这条判据能在**本机**真跑（见 §7 第 4 条）。

**代价（必须说清）**：不做熵编码 ⇒ 体积 ≈ `宽×高×3`（1080p ≈ 6.2 MB）。
仓库红线（`AGENTS.md` §2 红线 9 / 守卫 G06）禁止提交 >10 MB 的未登记文件，因此：

- `encode_rgb8_limited(image, limit)` 让"超过自设上限"变成**显式错误**，而不是某天被 G06 拦下；
- 截图一律落在 `target/ui-test-port/`（`.gitignore` 已忽略），**不进仓库**；
- 真要提交进仓库的基准图请用小尺寸或走带压缩的产出路径 —— 见 §10 needs。

---

## 5. 依赖方向与"为什么适配器是一个 `[[test]]` 目标"

- 方向：`yeban-app --(feature ui-test-port)--> yeban-ui-test-port`，**绝不反向**（会成环）。
  因此控件树数据模型住在 test-port 侧，适配器住在 app 侧。
- 把 `src/test_port_adapter.rs` 变成 `lib` 模块需要在 `crates/yeban-app/src/lib.rs` 里加
  `#[cfg(feature = "ui-test-port")] pub mod test_port_adapter;` —— **`lib.rs` 不在本线授权范围**
  （本线只被授权新增该文件 + 改 `Cargo.toml`）。因此改用 Cargo 的标准做法：
  `[[test]] name = "test_port_adapter" path = "src/test_port_adapter.rs"` +
  `required-features = ["ui-test-port"]`。
- 后果（诚实记录）：CI 的 `rust (yeban-app)` 矩阵腿**不加** `--features ui-test-port`，
  所以这个目标被 Cargo **跳过**，适配器与"真的渲染 13 个 `.slint`"的判据在本线 CI 上
  **不会被执行**。命令是：

  ```bash
  cargo test -p yeban-app --features ui-test-port --test test_port_adapter --locked -- --nocapture
  ```

  **这是本线最主要的 pending。** 建议集成者二选一：给 CI 的 `yeban-app` 腿加第二步
  `cargo test -p yeban-app --features ui-test-port --test test_port_adapter`；或者用
  `workflow_dispatch` 手动档跑一次并记账。

---

## 6. app 侧判据的形状（为什么不是"全覆盖"）

上游 Testing Backend 的遍历是**几何可见性过滤**（§2 第 15 条）。因此：

| 方向 | 断言 | 为什么成立 |
| :--- | :--- | :--- |
| 运行时有 ⇒ 注册表有（`unknown_at_runtime`） | **严格为空** | `.slint` 与注册表的双向契约已由 `elements.rs` 的判据钉住；出现未登记 ID 就是契约破裂 |
| 注册表有 ⇒ 运行时未必有（`missing_at_runtime`） | **非空且 < 注册表总数** | `visible: false` / 被裁剪 / 虚拟化视口外的元素本来就不该在树里（例如默认视图下会话视图画布） |
| 默认视图必需部件 | 10 个 ID 的**显式白名单**（transport / status-bar / arrangement / ai-rail） | 这些部件在 `arrangement_view = true, compact = false` 下必然可见 |
| 静态回归仍被检出 | 未遮罩 SSIM < 0.98 只对**占画面 ≥1%** 的动态区成立 | SSIM 是逐窗均值：1px 宽的走带光标改 ~100 个像素时均值仍是 1.000000 —— 这是口径，不是判据失效。夹具判据（改动区占画面 8.9%）覆盖该方向 |

---

## 6.5 CI 上真的产出了什么（`[MUST-GATE-015]` 的实测数字）

第 4 轮 CI（run 37222243555，job `rust (workspace 全量)`）里，夹具判据打印出的**像素证据**：

```text
Tier-1 Golden: 160x100 (16000 px), 非黑 16000 (100%), 颜色 61 种, PNG 48168 字节, 指纹 a8c9dec1c9835a5c
```

即（这一轮夹具还是 160x100 的旧布局）：

| 项目 | 实测 |
| :--- | :--- |
| 尺寸 | `160x100`（载体是 `SoftwareRenderer` + `MinimalSoftwareWindow`，**无任何物理显示器**） |
| 非全黑 | 非黑像素 **16000 / 16000 = 100%**，不同颜色 **61** 种 ⇒ `[MUST-GATE-015]` 的两条门槛都满足 |
| PNG | 48168 字节（stored deflate），FNV-1a64 指纹 `a8c9dec1c9835a5c` |
| 两次截图 | **逐字节相同**（同一次运行里连续两次 `capture()` 的像素完全一致 ⇒ 光栅化是确定的） |
| 控件树 | 运行时抠出 **6 个节点**，与夹具注册表双向覆盖闭合（debug info 打开之后） |

**没有 artifact 可下载**：CI 的 rust 腿没有 `upload-artifact` 步骤，PNG 落在 runner 的
`target/ui-test-port/` 里随容器销毁。要拿图只能在有显示环境的机器上跑
`cargo test -p yeban-ui-test-port --features ui-test-port`（本 crate 的夹具不需要 feature，
直接 `cargo test -p yeban-ui-test-port` 即可），产物路径会打印在测试输出里；
`render::report_evidence()` 会把上面的数字**直写进程 fd 2**，因此在 CI 日志里可见
（`cargo test` 会捕获 `eprintln!`，通过的测试什么都看不到 —— 一条"数字型"门禁的数字必须看得见）。

## 7. 本机判据（**真跑过**）

### 7.0 clippy 探针（**这一条救了至少一轮 CI**）

零 Slint 依赖的 7 个模块还能再进一步：在 `/tmp` 建一个**探针 crate**（`#[path]` 指向仓库源码 +
`serde`/`serde_json` + 与根 `Cargo.toml` 相同的 `[lints]` 策略），用**真的 clippy** 跑一遍：

```bash
cd /tmp/uitp-clippy && cargo clippy --offline --all-targets --quiet   # 期望 0 输出 / exit 0
```

它抓到了两条本机 `rustc`（不跑 lint 组）抓不到的 `clippy::all` 违规：

1. `clippy::chunks_exact_to_as_chunks`（`clippy::all`，clippy 1.99 新增）：
   `pixels.chunks_exact(3)` / `chunks_exact_mut(3)` 必须改写成 `as_chunks::<3>()` / `as_chunks_mut::<3>()`，
   否则 CI 的 `clippy -D warnings` 直接红（5 处）；
2. `clippy::doc_overindented_list_items`：文档列表的续行缩进超过标记宽度（1 处）。

**结论**：本机虽然不能编译 Slint，但"零 Slint 模块"这一半可以用真实 clippy 完全验完。
这一半包含了本线 ~80% 的代码量。

另外**依赖图相关的门禁在本机也能真跑**（零编译）：仓库文档里给了预编译的 cargo-deny
（`/Users/crow/work/music/.tooling/cargo-deny-0.20.2-aarch64-apple-darwin/cargo-deny`），
用与 CI 完全相同的调用即可：

```bash
cargo-deny --all-features check     # 期望: advisories ok, bans ok, licenses ok, sources ok
```

它抓到了一条 `clippy` 与 `run-gates.sh light` **都抓不到**的坑：`[bans] wildcards = "deny"`
会因为"只有 path、没有 version"的依赖而报 `error[wildcard]`（见 §2 第 26 条）。

### 7.1 方法：用 `rustc --test` 单独编译零 Slint 的 7 个模块

Slint 无关的 7 个模块（`image` / `tree` / `png` / `mask` / `ssim` / `port` / `golden`）可以在本机
绕开"不能编译 Slint"的限制。用的是仓库 `target/debug/deps` 里**已经编好**的 `serde` / `serde_json`
rlib（`cargo` 无需任何新下载）：

```bash
# harness 不在仓库里: <workspace>/.uitp-harness/main.rs (用 #[path] 指向仓库源码)
DEPS=/Users/crow/work/music/yeban/target/debug/deps
rustc --edition 2024 --test -D warnings -L dependency=$DEPS \
  --extern serde=$DEPS/libserde-30f71b3e94a36092.rlib \
  --extern serde_json=$DEPS/libserde_json-b95cb1045e4a234d.rlib \
  main.rs -o /tmp/uitp_pure_tests && /tmp/uitp_pure_tests
```

结果：**43 条判据全绿，`-D warnings` 零告警**（含 `#![deny(missing_docs)]` / `rust_2018_idioms`）。

**验证范围声明（重要）**：这 43 条覆盖
控件树 JSON / 遮罩 / SSIM / 三级权限 / PNG 编码 / Golden 路径，
**不覆盖** `render.rs` 与 `inspect.rs`（两者依赖 Slint，本机不能编译）⇒
Tier-1 光栅化、"非全黑"、`ElementHandle` 遍历、`LivePort` 全部**交给 CI**。

### 7.2 独立解码器验证 PNG（不是"自己的解码器能读自己写的"）

用标准 `zlib` 解开自产 PNG 的 IDAT，并逐字节比对扫描线：

```
chunks: [('IHDR', 13), ('IDAT', 9275), ('IEND', 0)]
IHDR: 64 48 depth 8 colortype 2 compression 0 filter 0 interlace 0
inflated 9264 bytes; expected 9264
pixel data identical to expected: True
png sha256: 393b4fa57fb96eb2
```

外部工具再确认一次：`file` → `PNG image data, 64 x 48, 8-bit/color RGB, non-interlaced`；
macOS `sips`（ImageIO，完全独立的解码器）→ `pixelWidth: 64 / pixelHeight: 48 / format: png`。

### 7.3 注入验证（"从没红过的判据是注释"）

做法：把 7 个模块复制到 `/tmp/uitp-mutant/src/`，在**副本**上改坏、跑、确认红、恢复、确认绿。
仓库文件全程未被变异。

| # | 注入 | 结果 |
| :-- | :--- | :--- |
| (a) | `mask::apply_masks` 改成立即 `return 0`（遮罩不生效） | **红 4 条**：`mask::mask_writes_pure_black_into_the_region`、`mask::changes_inside_the_mask_do_not_change_the_verdict`、`mask::out_of_bounds_rects_are_clamped_and_never_fake_effectiveness`、`ssim::masking_absorbs_dynamic_jitter_but_keeps_static_regressions` |
| (b) | `port::authorize` 改成永远 `Ok(())`（ReadOnly 也放行 `dispatch_*`） | **红 3 条**：`port::read_only_denies_every_write_and_never_reaches_the_impl`、`port::authorize_is_a_pure_total_function`、`port::interactive_allows_injection_but_not_administration` |
| (c) | `tree::merge_dynamic_flags_from` 改成空实现 + `golden::check_platform_tag` 的跨平台分支失效 | **红 2 条**：`tree::dynamic_flags_flow_from_registry_into_the_runtime_tree`、`golden::cross_platform_goldens_are_rejected` |
| (d) | 删掉 `ControlTree::insert` 里的角色校验（让反序列化可以绕过 `Role::parse`） | **红 1 条**：`tree::json_round_trip_is_lossless_and_validated` |

恢复后四次都回到 **43 passed / 0 failed**。

### 7.4 判据清单（19 条，按模块）

`image` 5 条、`tree` 7 条、`png` 5 条、`mask` 5 条、`ssim` 7 条、`port` 7 条、`golden` 5 条
（共 43 个 `#[test]`）；外加 `lib.rs` 2 条 crate 级契约判据（只在本机 harness 里跑了等价副本）。
CI 上另有 `render.rs` 的 1 条端到端判据（Tier-1 光栅化 + 内联 `.slint` + 控件树 + 遮罩 + SSIM + 权限）
与 `inspect.rs` 的 2 条（角色全集映射、几何取整）。

---

## 8. 本机**没做**的（纪律要求，交给 CI）

- `cargo clippy/test -p yeban-ui-test-port`：含 Slint，本机禁止（`run-gates.sh` 会跳过并指向 CI）。
- `cargo test -p yeban-app --features ui-test-port ...`：同样含 Slint。
- 任何 `--workspace` 全量构建、基准、模糊测试。
- 从未在本机链接/运行过含 Slint 的二进制。

`bash scripts/gates/run-gates.sh light` 本机**通过**（fmt / 12 条红线守卫 / 文档 / 依赖许可清单）。
`python3 scripts/gates/license_inventory.py` 已在本分支重新生成
（外部依赖包数仍是 **579**，没有新包；只有 4 行"直接使用方"归属变化 + `Cargo.lock` 摘要）。
`cargo metadata --locked` 通过（`Cargo.lock` 只增加了两个 workspace 成员的依赖列表项）。

---

## 9. 交付物里"没实现"的部分（不是遗漏，是边界）

- `Administrative` 的三个动作（切换主视图 / 强制保存 / 重载引擎）：`render::LivePort` 一律返回
  `PortError::Rejected { message }` 并说明原因 —— 真实接线需要 `yeban-model` / `yeban-engine`
  句柄，那会污染依赖方向。**权限判定本身是完整的**（有穷举判据）。
- 网络服务（HTTP/JSON-RPC、会话 Token）：明确属于 `crates/yeban-ui-mcp`（`[ARCH-UI-004]`）。
- 基准图库：`src/golden.rs` 只提供**路径约定 + 跨平台混用检查**；`tests/golden/**` 是空的 ——
  基准图必须由人在对应平台上生成并提交（Agent 不该在没有人类确认的情况下把某次渲染固化成基准）。
- `ControlNode::parent`：模型里有这个字段（§12.2 允许"组件层级选择器"寻址），但**两个来源都给不出**：
  上游 `ElementHandle` 没有 parent 访问器，app 的 `ElementMeta` 也没有父字段。
  因此目前两条路都填 `None` —— **不编造层级**。见 §10 needs。

---

## 9.5 与集成者裁决的关系（记录一次**有证据的分歧**）

集成者在第 2 轮后给的设计裁决是"控件树的权威事实源是注册表；运行时查询是可选的、必须安装
testing backend 的能力；能力缺失时出声跳过"。本线**采纳前两条**（`dump_json()` 的权威来自注册表；
运行时交叉核对拆成独立判据），但对第三点与根因判定保留了**实测证据**：

| 分歧点 | 实测证据 | 本线处置 |
| :--- | :--- | :--- |
| "运行时树为空是因为无头路径故意不构造窗口 / 必须安装 testing backend" | `ElementHandle` 的**只读**遍历只依赖 `WindowInner::from_pub(window).component()`（`ElementRoot` 对任何 `ComponentHandle` 有 blanket impl，`search_api.rs:103-120`），**与平台类型无关**。本线的 `render::LivePort` 就是在**软件光栅化平台**下同一个实例里既抓树又抓像素。空树的真因是**编译期 debug info 默认关闭**（`i-slint-compiler-1.18.1/lib.rs:282`），上游自己的报错文案就是这么说的（`search_api.rs:62`） | 用 `build.rs` + `ui/fixture.slint` + `with_debug_info(true)` **真的修好了**本 crate 的夹具（CI 第 4 轮验证），而不是降级成 skip |
| "能力缺失时出声跳过、不算作通过" | 本 crate 的夹具已不需要跳过；而 `yeban-app` 侧的缺口会连带挡住属性读取（§12.3）、动态遮罩（§12.5）与事件注入所需绝对坐标（§12.4） | 采纳"出声"（新增 `report_capability()`：一行 `RUNTIME-TREE-CAPABILITY: …` 直写进程 fd 2，绕过 libtest 捕获）**并且**让 app 侧那条判据**红**（`SKIP` + `return` 会让它算作通过，与"不得算作通过"冲突；`#[ignore]` 无法表达"运行时才知道的能力缺失"） |

如果人类裁决"运行时控件树不是本阶段的能力、事件注入推迟"，那么
`runtime_control_tree_cross_check_against_the_registry` 应当被**整体删除**（而不是改成 SKIP）——
删除是一次显式的范围收缩，SKIP 则会让能力悄悄烂掉。

## 10. needs（需要别人做）

0. **【最高优先】`crates/yeban-app/build.rs` 必须打开编译期 debug info** ——
   否则 `[ARCH-UI-005]` / `[UI-TEST-001]` / `[UI-MCP-002]` 在**真实界面**上完全无法执行：
   `ElementHandle` 拿不到任何元素，控件树恒为空（§2 第 27 条）。
   修法二选一（`build.rs` 不在本线授权范围内，本线不代改）：
   - `slint_build::compile_with_config("ui/app.slint", CompilerConfiguration::new().with_debug_info(true))`
     （推荐；本 crate 的 `build.rs` 已经是这个形状，可直接照抄）；
   - 或 CI 构建步骤加 `SLINT_EMIT_DEBUG_INFO=1`。
   注意：只改 `.slint` 或只改测试都**没用**，必须是**编译期**开关。
1. **CI 需要真的跑 app 侧那一条判据**（本线最大的缺口）：给 `.github/workflows/ci.yml` 的
   `rust (yeban-app)` 腿加一步
   `cargo test -p yeban-app --features ui-test-port --test test_port_adapter --locked -- --nocapture`，
   或用 `workflow_dispatch` 手动档跑一次并记账。`.github/**` 是集成者独占文件，本线不改。
2. **让适配器成为 `lib` 模块**（可选）：在 `crates/yeban-app/src/lib.rs` 加
   `#[cfg(feature = "ui-test-port")] pub mod test_port_adapter;`。同样不在本线授权范围。
3. **`ElementMeta::component`（`.slint` 文件名）在控件树模型里没有对应字段** ⇒
   断言失败时无法从 JSON 直接映射回源码文件。补法二选一：给 `ControlNode` 加一个可选 `source` 字段，
   或让断言脚本另外读 `elements.rs` 的注册表做映射。属于模型演进，需要集成者裁决。
4. **`ControlNode::parent` 永远是 `None`**（见 §9）：需要上游暴露 `parent_item`，或 app 注册表补
   父 ID 字段。属规范/模型缺口，本线不发明。
5. **人类裁决：要不要用 `configure_test_fonts()` 消除跨平台字体差异**（§2 第 21 条）。
   若采纳，`[UI-MCP-003]` 的"分平台 Golden"要求可以放宽为"单套基准 + 平台白名单"，
   但代价是依赖官方内部 crate 的 `internal` feature（随时可能变），且 emoji/CJK 覆盖面由内嵌字体决定。
6. **基准图库的尺寸策略**：stored deflate 下 1080p ≈ 6.2 MB，接近红线 9 的 10 MB。
   若要在仓库里维护基准图，需要裁决"多小的截图算合格基准"或"是否允许引入压缩器"。
7. ~~规范措辞：`SLINT_BACKEND=headless` 与 `slint::testing::*`~~ —— 已由 ADR-0001 D18 记录；
   本线新增的 `Rgb8Pixel` 位置 / `init_no_event_loop` 名字 / 遍历的可见性语义三条，建议并入 D18。

## 11. pending（未证实的、已知的债）

1. **本线 CI 是否真的绿** —— 见 §12 的判决；未读到判决前一律记 `pending`。
2. **`render.rs` / `inspect.rs` 的编译正确性**：本机不能编译 Slint，这两个模块只经过
   "逐条对源码"的核验。第一次 CI 才是真判决。
3. **`slint!` 内联夹具在 Linux 上的实际渲染结果**（字体回退、CJK 缺失、颜色）未知 ——
   本机不能跑；CI 只判"尺寸非零且非全黑 + 颜色数 ≥3"，不判"好不好看"。
4. **app 侧端到端判据（13 个 `.slint` 真的被渲染）从未被执行过**（见 §5/§10 第 1 条）。
   在它被执行之前，"界面被渲染过"这件事对 `yeban-app` 仍**未证实**。
   而且现在已知：**即使跑了，控件树那一半也会红**，因为 `yeban-app/build.rs` 没开
   debug info（§10 第 0 条）。判据已按"先出像素证据、再报控件树阻塞"的顺序写好，
   所以它失败时仍然会打印/落盘 Tier-1 截图的证据。
5. **`#![forbid(unsafe_code)]` 未加**：`slint!` 宏展开的生成代码是否含 `unsafe` 在本机无法核验；
   红线 8 只对 model/theory/dsp/render 强制。核验办法：在 CI 上试着加上该属性编译一次
   （若通过就加上）。
6. **未启用 `renderer-skia`**（同 ui-shell 的结论）：拖入 LLVM/clang，与本机纪律冲突。
7. **`mask_is_effective` 的语义**：矩形全部落在图像外时返回 `false`（"生效"无从证明），
   调用方必须自己区分"没遮到"与"没有矩形"。
8. **重复元素（`for` 循环）能否用语义 ID 寻址 —— 未核验（本线最重要的技术风险）**：
   见 §2 第 22 条。`ElementHandle::accessible_id()` 对 `element_index != 0` 返回 `None`。
   若上游把 `for` 展开成"单 ItemRc + element_count"，则
   `note-{ulid}-rect` / `clip-{ulid}-header` / `track-{i}-header` 里只有**第一个**实例能被
   `[UI-TEST-001]` 的语义 ID 找到 —— 那会是规范级的缺陷（`[UI-TEST-001]` 明文要求这些族可寻址），
   需要人类裁决替代方案（`find_by_element_id` 限定 id + `element_index`，或改用路径/层级选择器）。
   **验证办法**：跑一次 app 侧端到端判据，看它打印的
   `观察值: 运行时树里 track-*-header = …` 是否为 0（该观察值**故意不是断言**，
   因为断言一个尚未核验的上游行为会制造假红）。
