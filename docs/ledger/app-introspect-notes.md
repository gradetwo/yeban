# app-introspect 工作线备注 —— **真实界面**的首次 Tier-1 渲染 / 控件树 / 遮罩

- **工作线**: `line/app-introspect`（worktree `.worktrees/app-introspect`，基线 main `f05000e`）
- **日期**: 2026-10-05
- **目标（原始口径）**: 让 `crates/yeban-app` 的 13 个 `.slint` **第一次被 Tier-1 软件光栅化渲染出来**，
  产出可人眼复核的截图 + 可信的语义控件树。这是 ADR-0001 D18/D22 登记的本仓库**最大未验证面**。
- **授权的文件**: `crates/yeban-app/src/test_port_adapter.rs`（主战场）、
  `crates/yeban-app/Cargo.toml`（`[[test]]` / feature / dev-dependency 段）、
  `crates/yeban-app/tests/**`（新建）、`crates/yeban-ui-test-port/**`（小幅增补）、本文件。
  `build.rs` 未改（集成者已按 D22 打开 `with_debug_info(true)`）。
- **本机纪律**: 全程用 `scripts/dev/cargo-local.sh`；**没有**在本机编译 Slint（`run-gates.sh crate yeban-app` 自动 SKIP）。
  纯计算部分按上一条线的方法在本机真跑（§5）。

---

## 1. 交付物 ↔ 规范 ID

| 交付物 | 规范 ID | 本线做了什么 |
| :--- | :--- | :--- |
| `crates/yeban-app/src/test_port_adapter.rs` | `UI-TEST-001` `UI-MCP-001` `UI-MCP-002` `UI-MCP-003` `MUST-GATE-015` `ARCH-UI-005` **ADR-0001 D22/D24** | **修到能编译**（8 处错误）；真实 `MainWindow` 的三种状态 Tier-1 截图；运行时控件树 vs 注册表的**实测**覆盖判据；动态遮罩 + SSIM 判据；**D24 的"汉字非 tofu"量化判据**；`ReadOnly` 权限判据 |
| `crates/yeban-app/tests/real_ui_tier1.rs`（新建） | 同上 | 同一份判据的**第二个 cargo 目标**（自动发现 ⇒ CI 上真的执行；集成者已批准，见 §2） |
| `crates/yeban-app/Cargo.toml` | ADR-0001 D18/D21 | 新增 `[dev-dependencies] yeban-ui-test-port`（理由见 §2）；原 `[[test]]` + feature 设计**原样保留** |
| `crates/yeban-ui-test-port/src/render.rs` | `MUST-GATE-015` | 新增公开 `report_line()`，`report_evidence` / `report_capability` 改为委托它（唯一出口，语义不变） |
| `crates/yeban-ui-test-port/src/lib.rs` | — | 根重导出补 `report_line` |
| `docs/ledger/app-introspect-notes.md`（本文件） | — | 实测数字 / 判据 / 上游新发现 / needs |

测试命令（**集成者要点的那一条**）：

```bash
cargo test -p yeban-app --features ui-test-port --locked -- --nocapture   # 显式启用 feature 的原始命令
cargo test -p yeban-app --all-targets --locked                            # 现在也会跑同一批判据(见 §2)
```

---

## 2. 交付物 1：适配器可编译 —— 8 处错误全部定位并修掉

`cargo test -p yeban-app --features ui-test-port` 在 run 37223586792 上的实测失败：

| # | 错误 | 根因（实测核对源码/文件得出） | 修法 |
| :-- | :--- | :--- | :--- |
| 1-6 | `cannot find value 'image' in this scope` ×6 | `image` 只在**第一个** `#[test]` 里绑定过；第二个测试（`runtime_control_tree_cross_check_against_the_registry`）里 6 次使用它却从未截图 | 在第二个测试里显式 `let image = port.window().capture()`（截的正是判据要断言的同一帧） |
| 7 | `cannot find function 'report_evidence'` | 它确实是 `render.rs` 的**公开项**，但 `use` 清单里漏了 | `use` 里补 `report_evidence` |
| 8 | `cannot find function 'report_capability'` | 同上 | `use` 里补 `report_capability` |

**为什么"改到能编译"不是这一条的终点 —— 以及我因此做的第二个改动（需要集成者知情）**：

那个 `[[test]]` 目标挂在 `required-features = ["ui-test-port"]` 后面，而 CI 里没有任何一步启用该
feature ⇒ 即使修好了，`cargo clippy/test -p yeban-app --all-targets`（矩阵腿）与
`--workspace --all-targets`（全量腿）**都不会编译它**。也就是说这一轮 CI 会"绿着跳过"，
而集成者按 ledger 的计划加回那一步时才会第一次知道它能不能编译 —— 那正是这一条工作线要避免的
失败模式（"它从未被编译过"）。

处置（**只用自己授权内的文件**）：

- `crates/yeban-app/Cargo.toml` 增加 `[dev-dependencies] yeban-ui-test-port = { path = "../yeban-ui-test-port" }`；
- 新建 `crates/yeban-app/tests/real_ui_tier1.rs`（`tests/*.rs` 是自动发现目标），用
  `#[path = "../src/test_port_adapter.rs"] mod criteria;` 装入**同一份**判据源码
  ⇒ 不存在第二份实现，也就不会漂移；两个目标同时启用 feature 时同一批判据会跑两遍（可接受）。
- **没动** `[[test]] test_port_adapter` 与 `ui-test-port` feature：集成者那条命令
  `cargo test -p yeban-app --features ui-test-port --locked` 继续可用、继续跑同一批判据。

**红线 6 没有被削弱（实测证据，零编译）**：

```text
$ cargo tree -p yeban-app -e normal --locked | grep -c yeban-ui-test-port
0                                    # 常规/release 构建图里没有它
$ cargo tree -p yeban-app -e dev,normal --locked | grep -n yeban-ui-test-port
634:└── yeban-ui-test-port v0.0.1 (crates/yeban-ui-test-port)   # 只有测试目标拿到它
```

`Cargo.lock` **未变**（同一条 `yeban-ui-test-port` 依赖项早就在 lock 里，只是多了一个 dev 边），
因此依赖许可清单也不需要重新生成（`license_inventory.py --check` 在本机仍是绿的，见 §5.4）。

**集成者裁决（第 1 轮判决之后）**：**批准保留**，并且**不再加**那条 CI 步骤 ——
理由与我给的一致（"在加回之前判据仍然是没人跑的状态，正是账本批评的失败模式"）。
账本里"待重新加回 CI 步骤"的记载改成"由 app 侧自动发现测试覆盖，无需专用步骤"。
`[[test]] test_port_adapter` 与 `ui-test-port` feature 仍然保留（那条显式命令继续可用）。

**若将来要回退这个第二入口**：删掉 `crates/yeban-app/tests/real_ui_tier1.rs` 与
`Cargo.toml` 里的 `[dev-dependencies]` 两段即可（判据本身不动），代价是回到"必须先改
`ci.yml` 才能知道它能不能编译"。这是一个显式的取舍，不藏在提交里。

---

## 3. 上游 API 的新发现（补 ADR-0001 D18 / D22 / D23）

> 方法同上一条线：读 `$CARGO_HOME/registry/src/.../i-slint-*-1.18.1/` 的**实际源码**。
> 下面每条都给出文件与行号/函数名，便于复核。

| # | 结论 | 出处 |
| :-- | :--- | :--- |
| A1 | **`visible:` 在编译期被 lower 成注入的 `Clip` 元素**：`clip = !visible`、`is-visibility-clip: true`。因此"可见性"不是靠读 `visible` 属性，而是靠**裁剪矩形**生效 | `i-slint-compiler-1.18.1/passes/visible.rs`（`create_visibility_element()`） |
| A2 | **该 pass 跑在 `default_geometry` 之后**（顺序：`lower_layout` → `default_geometry` → … → `lower_property_to_element(opacity)` → **`visible`** → …）。注入的包裹元素由 `Element::make_rc()` 建（`geometry_props = Some(GeometryProps::new)` 但**没有任何几何绑定**），且 `visible.rs` **没有**调用 `adjust_geometry_for_injected_parent` ⇒ 它的几何恒为 `0×0` | `i-slint-compiler-1.18.1/passes.rs:171-198`；`object_tree.rs:1588-1593`（`make_rc`）；对比 `passes/visible.rs` 与 `passes/lower_property_to_element.rs:89`（后者**确实**调了 adjust） |
| A3 | 于是 `ItemRc::is_visible()` 的语义是：`visible == true` ⇒ 包裹元素 `clip=false` ⇒ 不裁剪、子元素照常可见；`visible == false` ⇒ `clip=true` 且裁剪矩形为空 ⇒ **整棵子树被过滤掉** | `i-slint-core-1.18.1/item_tree.rs:410-419`（`absolute_clip_rect_and_geometry`：`clip = ancestor_geom ∩ clip`，空矩形 ⇒ `is_visible()` false） |
| A4 | 上游自己的回归测试把 A3 钉死了：`visible: !condition` 的元素与它的子元素在 `condition = true` 时 `find_by_element_id(...).count() == 0` | `i-slint-backend-testing-1.18.1/search_api.rs:1223-1266`（`test_conditional`） |
| A5 | `query_descendants()` 与 `visit_descendants()` 走**同一个** `visit_descendants_impl`，所以"上游测试里查不到"可以直接外推到 `visit_descendants` | 同上 `search_api.rs:169-198`（`match_recursively` → `visit_descendants_impl`） |

**这三条对判据形状的影响（本线据此写的判据）**：`visible: false` 的分支**不会**出现在运行时树里
⇒ "注册表有而运行时无"必然非空（默认视图下 Session 视图整棵 / Mixer / DeviceRack / 两个模态都不在），
于是本线把判据写成 **运行时 ⊆ 注册表**（强）+ **缺失非空且 < 全部**（弱），
并把 7 个"必须不可见"的 ID 写成显式黑名单（`DEFAULT_VIEW_MUST_NOT_HAVE`）。

**规范要求但上游不支持（新增一条，建议并入 D18）**：UI/UX §12.2 隐含假设"上了 `accessible-id`
的节点都在无障碍树里可寻址"。实测上游语义是"**几何可见 + `visible` 语义**过滤后的集合"，
所以**规范不能把"控件树 = 声明的元素全集"当真**；`yeban-ui-test-port` 的静态注册表正是为此存在的
（无窗口也能跑，覆盖全集），两者必须交叉核对而不是二选一。

---

## 4. `[UI-TEST-001]` 的实测判据形状（按事实写，不为了让数字好看而放宽）

| 方向 | 判据 | 证据 |
| :--- | :--- | :--- |
| 运行时有 ⇒ 注册表有 | `coverage.unknown_at_runtime` **严格为空** | 注册表与 `.slint` 的模板级双向契约已由 `elements.rs::slint_accessible_ids_and_registry_cover_each_other` 钉住；运行时树里的每个 `accessible-id` 都来自这些 `.slint` |
| 注册表有 ⇒ 运行时未必有 | `missing_at_runtime` **非空** 且 `< static_tree.len()` | §3 A3/A4：`visible: false` 的子树被过滤；默认视图下 Session/Mixer/DeviceRack/两个模态整块不在树里 |
| 默认视图关键部件 | 16 个 ID 的**硬清单**（`DEFAULT_VIEW_MUST_HAVE`） | 全部是"单次声明 + 无裁剪风险"的部件 |
| 全面覆盖 | `VISIBLE_SINGLETONS`（39 条单例）覆盖率 **≥ 90%** | 定额而不是"必须 100%"：某块面板被邻居裁掉 1px 属几何口径问题；系统性失效（空树/整列不见）会掉到 0% |
| 重复族（`for` 循环） | 只断言"**每个可见族至少一个成员**可被语义 ID 寻址"（10 条样本），**全部实例的计数只打印** | 见下 |

**重复族为什么只按事实写（本线最重要的技术风险，来自上一条线的 pending #8）**：
`ElementHandle::accessible_id()` 对 `element_index != 0` 返回 `None`
（`i-slint-backend-testing-1.18.1/search_api.rs:745-748`），而 `for` 循环展开到底是
"每个实例一个独立子 ItemTree（`element_index == 0`，可寻址）"还是"单 ItemRc + `element_count > 1`"
（只有第一个实例可寻址）**在编译之前读不出来**。因此本线：
① 断言"至少一个成员可寻址"（这是 `[UI-TEST-001]` 的最低要求，无论上游是哪种展开都必须成立）；
② 把 `track-*-header` / `note-*-rect` / `clip-*-header` / `velocity-*-bar` / `section-*-card` /
`slot-*-cell` / `tab-*-button` / `sidebar-item-*` / `piano-roll-tool-*-button` 的**实测计数**
打印进 CI 日志与 `target/ui-test-port/app-introspect-observations.txt`（artifact）。

**实测结果（第 1 轮 CI，run 37224871698，见 §6.1）**：
`track-*-header=6`、`clip-*-header=3`、`section-*-card=4`、`tab-*-button=3`、`sidebar-item-*=8`、
`piano-roll-tool-*-button=5`、`velocity-*-bar=6`、`note-*-rect=7`（6 个音符 + 1 个 AI 建议块）
⇒ **每个重复实例都能被语义 ID 寻址**（不是只有第一个）。上一条线的 pending #8 因此关闭：
上游把重复元素展开成"每个实例一个子 ItemTree（`element_index == 0`）"，
`[UI-TEST-001]` 点名的三个族 `note-{ulid}-rect` / `clip-{ulid}-header` / `track-{i}-header` **全都可用**。
下限判据（"至少一个成员"）留在原地作为这个结论的下界 —— 它一旦变红就说明重复元素整族不可寻址。

---

## 5. 本机**真跑过**的判据（vs 交给 CI 的）

本机不许编译 Slint，但"纯计算那一半"可以像上一条线那样单独编译执行。
harness 在 **`<workspace>/.app-introspect-harness/`（不在仓库里）**，被验证的对象**不是复制品**：

- `pure_lib.rs`：`--crate-name yeban_ui_test_port` 建 rlib，`#[path]` 指向**仓库原件**
  `crates/yeban-ui-test-port/src/{image,tree,mask,ssim}.rs`（零 Slint 的四个模块）；
- `app_lib.rs`：`--crate-name yeban_app` 建 rlib，`#[path]` 指向**仓库原件**
  `crates/yeban-app/src/{scene,elements}.rs`；
- `extract.sh` / `extract_ids.sh`：把被判据依赖的三个函数与四份 ID 清单**逐字**从
  `test_port_adapter.rs` 抽出来（唯一改写：补 `pub ` 与 harness 侧的 `use` 前导），
  因此不存在"复制品与仓库版本漂移"的可能。

```bash
# 一次性: 建两个 rlib (零 Slint 编译, 秒级)
DEPS=/Users/crow/work/music/yeban/target/debug/deps
rustc --edition 2024 --crate-type rlib --crate-name yeban_ui_test_port -L dependency=$DEPS \
  --extern serde=$DEPS/libserde-30f71b3e94a36092.rlib \
  --extern serde_json=$DEPS/libserde_json-b95cb1045e4a234d.rlib \
  pure_lib.rs -o /tmp/libyeban_ui_test_port.rlib
rustc --edition 2024 --crate-type rlib --crate-name yeban_app \
  app_lib.rs -o /tmp/libyeban_app_local.rlib
# 判据: 9 条(计算/选块/遮罩+SSIM/inset/墨迹/真实截图对账) + 2 条(ID 清单)
./extract.sh && rustc --edition 2024 --test -D warnings -L dependency=$DEPS \
  --extern yeban_ui_test_port=/tmp/libyeban_ui_test_port.rlib main.rs -o /tmp/ai_pure_tests \
  && /tmp/ai_pure_tests --nocapture
./extract_ids.sh && rustc --edition 2024 --test -D warnings \
  --extern yeban_app=/tmp/libyeban_app_local.rlib idcheck.rs -o /tmp/id_check && /tmp/id_check
```

第 9 条判据（`ink_stats_matches_an_independent_decoder_on_the_real_screenshot`）用的是**真实截图**：
第 1 轮 CI 的 artifact 里那张 1920×1080 PNG，四个区域由 **Pillow**（独立解码器）按同一轮的
`app-runtime-control-tree.json` 的几何裁出来（`real_regions.txt` + 4 个 `.raw`）。
它同时证明"我们自产的 PNG 能被标准解码器读"（本仓库的编码器只写不读）。

### 5.1 结果（本机实测数字）

| 判据 | 实测 |
| :--- | :--- |
| 动态抖动**未遮罩** SSIM | **0.996130**（细窄动态区 ⇒ 拉不下 0.98，D23 的量化限定） |
| 动态抖动**遮罩后** SSIM | **1.000000**（精确相等；两图逐字节相同） |
| 成块静态回归（>40% 画面、暗 ⇒ 刷白）**未遮罩** | **0.621908** < 0.98 ✓ |
| 同一回归**遮罩后** | **0.621939** < 0.98 ✓（遮罩没把界面变成盲区） |
| 反向对照：0.48% 画面的小改动 | **0.994871** ≥ 0.98（**检不出** —— 这就是"候选必须 ≥5% 画面"的实测依据） |
| 39 条单例 ID / 16 条硬清单 / 10 条族样本 / 7 条黑名单 | 全部是注册表里**真实存在**的条目（`idcheck.rs`，2 条判据） |
| `overlap_area` / `mean_luma` 边界 | 相离/相切/包含/部分重叠/越界裁剪/黑白各半=127.5 全过 |
| `inset_rect` 边界 | 正常/退化(宽 8 收 4 ⇒ 0×0)/负坐标/0×0 全过 |
| `ink_stats` 合成用例 | 纯背景 ⇒ 0 墨迹；8 px 白块 ⇒ 墨迹 8、包围盒 4×2、2 色；背景色给错 ⇒ 整块 128 px 都是墨迹 |
| `ink_stats` vs **独立解码器**（真实截图） | 四块区域墨迹数/颜色数/**包围盒长宽**逐个相等（见 §6.2 的数字） |

### 5.2 变异测试（"从没红过的判据是注释"）

变异只作用在**生成物副本**（`extracted.rs` / `id_lists.rs`），仓库文件全程未被改动；
每次变异后都重新生成并确认恢复为绿。**11 处变异，11 处都让至少一条判据变红**。

| # | 注入 | 结果 |
| :-- | :--- | :--- |
| a | `pick_regression_target` 选**最小**候选而不是最大 | **红 1 条**（`regression_target_follows_d23`） |
| b | 改向恒为黑（不再按实测亮度选方向） | **红 1 条**（同上，暗面板必须往白改） |
| c | `mean_luma` 恒返回 0 | **红 1 条**（`mean_luma_matches_the_ssim_luma`） |
| d | 面积门槛反过来（只收 <5% 画面的候选） | **红 2 条**（`regression_target_follows_d23` + `masking_absorbs_jitter_and_keeps_the_regression`） |
| e | `overlap_area` 恒返回 0 | **红 1 条**（`overlap_area_handles_the_boundaries`） |
| f | 把"被动态区盖掉 ≥10% 就跳过"放宽成"有任何交集就跳过" | **红 1 条**（`regression_target_follows_d23`：最大的候选被误跳过） |
| g | 删掉"动态区不得当候选"的检查 | **红 1 条**（`regression_target_never_picks_a_dynamic_region`） |
| h | 把清单里的 `scene-launch-column-header` 改成 `scene-launch-column-head` | **红 1 条**（`every_listed_id_exists_in_the_registry`）—— 这正是本线真的犯过的一次手滑 |
| i | `ink_stats` 恒返回 `(0, None, 0)` | **红 2 条**（合成用例 + 真实截图对账） |
| j | `inset_rect` 原样返回（不收边） | **红 1 条**（`inset_rect_handles_degenerate_inputs`） |
| k | `INK_CHANNEL_TOLERANCE` 改成 255（什么都算不上墨迹） | **红 2 条**（同 i） |

### 5.3 本机**没有**验证的（交给 CI，逐条说清）

- `test_port_adapter.rs` 与 `tests/real_ui_tier1.rs` 的**编译正确性**（含 Slint ⇒ 本机禁止编译）；
- `ElementHandle` 的真实遍历结果：树有多少节点、哪些 ID 可寻址、重复族计数（§4）；
- Tier-1 光栅化的真实像素：尺寸 / 非黑占比 / 颜色数 / PNG 字节数 / 确定性；
- **真实界面上**的遮罩矩形与 SSIM 数字（本机用合成画面）；
- `clippy -D warnings`。

### 5.4 本机跑过的门禁

- `bash scripts/gates/run-gates.sh light`：**通过**（fmt / 13 条守卫 / 文档 / 依赖许可清单）。
- `cargo metadata --offline`：解析通过（`cargo-local.sh`）；`Cargo.lock` **无变化**。
- `cargo tree -p yeban-app -e normal --locked`：不含 `yeban-ui-test-port`（红线 6 的证据）。

---

## 6. CI 判决与**真实界面的实测数字**

> 未读到的判决一律记 `pending`。

| 轮 | commit | run | 结论 | 说明 |
| :-- | :--- | :--- | :--- | :--- |
| 1 | `53e2f93` | [37224871698](https://github.com/gradetwo/yeban/actions/runs/37224871698) | **success** | `plan` / `checks` / `lockfile` / `deny` / `rust (yeban-ui-test-port)` / **`rust (yeban-app)`** 全绿；`rust (workspace 全量)` 按设计跳过。`rust (yeban-app)` 真的执行了本线的判据：`running 8 tests` → `test result: ok. 8 passed`，并产出 artifact **`ui-screenshots-yeban-app`**（§6.1 的数字来自这一轮） |
| 2 | `c667fa2` | [37225490791](https://github.com/gradetwo/yeban/actions/runs/37225490791) | **failure（2 条，1 条是我的、1 条是 main 的）** | ① **我的**：`rust (workspace 全量)` 的 `clippy --workspace -D warnings` 死在 `error: constant TOKEN_BG_PANEL_ALT is never used` —— 第 2 轮我把对照元素从"Musical PR 卡"换成了 `status-bar-chord`，那个色值常量就没人用了。`test --workspace` 因此没跑，**CJK 数字与 D24 判据这一轮没有结果**。② **main 的**：`checks` 的"跨语言契约对账"死在 `no example target named export_mcp_samples in yeban-mcp package` —— D25 的 ci.yml 步骤先落地、mcp-core 的 example 后落地，`origin/main` 当时自己是红的（376… 见下），与本线无关。 |
| 3 | 见下 | `pending` | `pending` | 修掉 ①（把 `TOKEN_BG_PANEL_ALT` 用起来：多打印一块"混合卡"的墨迹），并 rebase 到已经补上 `export_mcp_samples` 的 `origin/main`（`101380c`）⇒ ② 也应消失 |

**第 2 轮的教训（写给后来的本机验证）**：本机 harness **抓不到"未使用常量"这类错误** ——
它只按名字抽取出"被判据引用到的"函数/常量，未被引用的项根本不会进 harness，
于是 `dead_code` 只在 CI 的 `-D warnings` 下暴露。这类"编译期才成立"的约束只有 CI 能判
（与本仓库既有的"只有 CI 能抓"清单同类：`clippy::chunks_exact_to_as_chunks`、`error[wildcard]`）。

**main 变红时的读法（本线第 2 轮实测）**：`checks` 里的"跨语言契约对账"步骤引用了
`cargo run -p yeban-mcp --example export_mcp_samples`，而该 example 当时只在 mcp-core 那条线的
分支上。判定"是不是我的错"的方法：`git ls-tree -r --name-only origin/main -- <路径>` +
`git log --oneline origin/main`（本线就是这么判定 ② 不是自己的）。


### 6.1 第 1 轮的实测数字（`gh run view --job 111502378733 --log` 取回）

```text
控件树计数: 注册表 184 条 / 运行时 95 条 / 运行时有而注册表无 0 条 / 注册表有而运行时无 89 条
运行时有而注册表无(必须为空): []
关键单例覆盖率: 39/39 = 100%（硬下限 90%）缺: []
重复族实测计数(观察值, 非断言): track-*-header=6, track-*-fader=0, track-*-meter=0,
  note-*-rect=7, clip-*-header=3, velocity-*-bar=6, section-*-card=4, slot-*-cell=0,
  tab-*-button=3, sidebar-item-*=8, piano-roll-tool-*-button=5
可见重复族里可被语义 ID 寻址的样本: 10/10
运行时动态区: 6 个（在画面内 6 个）, 总面积 18752 px = 画面的 0.9043%
[UI-MCP-003] 动态抖动未遮罩 SSIM=0.991640 / 遮罩后 SSIM=1.000000(passed=true);
  静态回归 1400x583@(240,48) 平均亮度 16.90 填 [255,255,255]: 未遮罩 SSIM=0.636483, 遮罩后 SSIM=0.636552(passed=false)
说明: 动态区只占画面 0.9043% ⇒ 未遮罩 SSIM=0.991640 拉不下 0.98（SSIM 口径, 非判据失效）
状态 A (Arrangement / 全展开 1920x1080): 1920x1080 (2073600 px), 非黑 2073600 (100%),
  颜色 2349 种, PNG 6222418 字节, 指纹 5e6020089976cb69
状态 A 两次截图逐字节相同: true
状态 B (Arrangement / compact): 1920x1080, 非黑 2073600 (100%), 颜色 2131 种, PNG 6222418 字节, 指纹 06bce1e2e0f6cec6
状态 C (Session / 全展开): 1920x1080, 非黑 2073600 (100%), 颜色 2235 种, PNG 6222418 字节, 指纹 14559b0cb92839d6
控件树 JSON: 注册表 184 条 -> app-registry-control-tree.json; 运行时(默认视图) 95 条 -> app-runtime-control-tree.json
```

读法（这张表就是本线最有价值的产物 —— "界面被渲染过"从口号变成了数字）：

| 项目 | 实测 | 结论 |
| :--- | :--- | :--- |
| 尺寸 | 1920×1080 = 2073600 px（三个状态一致） | `[MUST-GATE-015]` 的"尺寸非零"✓ |
| 非全黑 | 非黑 **2073600 / 2073600 = 100%** | `[MUST-GATE-015]` 的"非全黑"✓ |
| 颜色数 | A 2349 / B 2131 / C 2235 种 | 远不是"只画了背景" |
| PNG 字节 | 三个状态都是 6222418（stored deflate ⇒ 与内容无关的定长） | < 红线 9 的 10 MB ✓ |
| 确定性 | **同一状态连续两次截图逐字节相同** | 光栅化确定 ✓ |
| 三个状态互不相同 | 三个指纹互不相同（`5e60…` / `06bc…` / `1455…`） | Arrangement / compact / Session **都真的被渲染过**（不是同一张图复制三份） |
| 控件树 | 注册表 184 / 运行时 95（缺 89 = 全部不可见分支） | `[UI-TEST-001]` 的运行时⊆注册表**严格成立**（未登记 0 条） |
| 关键单例覆盖率 | **39/39 = 100%** | 默认视图里能声明到的单例一个不缺 |
| 重复族实例数 | `track-*-header=6`、`clip-*-header=3`、`section-*-card=4`、`tab-*-button=3`、`sidebar-item-*=8`、`piano-roll-tool-*-button=5`、`velocity-*-bar=6` | **`for` 循环的每一个实例都能被语义 ID 寻址** —— 上一条线的 pending #8 从"最重要的技术风险"变成**已证伪的风险**（见下） |
| 动态区 | 6 个（时间码 / 选区 / 和弦 / 设备 / 两个走带光标），合计 18752 px = 0.90% 画面 | 未遮罩 SSIM=0.991640 拉不下 0.98 ⇒ **"≥5% 画面"的前置断言是承重的**（否则这里会假红） |
| 遮罩 | 遮罩后 SSIM = **1.000000**（精确） | `[UI-MCP-002]` 完全吸收动态抖动 ✓ |
| 遮罩不遮瞎 | 静态回归（1400×583 = 39.4% 画面，暗 ⇒ 刷白）未遮罩 0.636483 / 遮罩后 0.636552 | `[UI-MCP-003]` 仍能检出静态缺陷 ✓ |

**上一条线的 pending #8 关闭**：`ElementHandle::accessible_id()` 对 `element_index != 0` 返回 `None`
这件事**不影响 `for` 循环的重复实例** —— 实测每个实例都能被语义 ID 找到（6/3/4/8/5 个）。
上游把重复元素展开成**每个实例一个子 ItemTree**（`element_index == 0`），因此
`[UI-TEST-001]` 的 `note-{ulid}-rect` / `clip-{ulid}-header` / `track-{i}-header` 三族**全都可用**。
本线因此把"至少一个成员可寻址"的下限判据留在原地（它是这个结论的下界），
并把"全实例计数"如实打印（上表）。

**artifact（人眼复核用，30 天）**：`ui-screenshots-yeban-app`
（`gh run download 37224871698 -n ui-screenshots-yeban-app`），内含：

- `app-main-window-arrangement-full-1920x1080.png`（默认演示视图）
- `app-main-window-arrangement-compact-1920x1080.png`（`compact` 断点：左栏 36px 导轨）
- `app-main-window-session-full-1920x1080.png`（Session 视图，`session_view.slint` 首次被渲染）
- `app-registry-control-tree.json` / `app-runtime-control-tree.json`（两份树，可逐条 diff）
- `app-introspect-observations.txt`（本线的全部实测观察值）

集成者已下载并把三张图**肉眼核对**过（原文：结构完整、语义正确、与规范 §1 的网格在视觉上对得上）。

### 6.2 D24「界面字体非 tofu」：两侧都有实测，判据因此是承重的

第 1 轮的 runner **没有**装 `fonts-noto-cjk`（D24 的 CI 改动晚于这一轮），于是截图里
**汉字整片没有被画出来**。这不是猜测 —— 用 **Pillow 独立解码**同一轮的 PNG，按元素树里的
几何裁片后量墨迹（规则：任一通道与该元素背景色相差 > 24）：

| 元素 | 内容 | 无 CJK 字体（第 1 轮 CI 实测） | 有 CJK 字体（本机 FreeType + 苹方/宋体，11px 量、按 10px 折算） |
| :--- | :--- | :--- | :--- |
| `ai-rail-diagnose-button` | 12 汉字 + `:`/`/` | **24 px** 墨迹（只剩 ASCII 标点） | **≈590 px** |
| `ai-rail-intent-button` | 14 汉字 + `:`/`…` | **6 px** | ≈620 px |
| `ai-rail-musical-pr-button` | 4 汉字 + 12 ASCII | **272 px** | ≈395 px |
| `status-bar-chord` | 2 汉字 + 5 ASCII | **119 px** | ≈225 px |
| `transport-bpm-field`（纯 ASCII 对照） | `BPM 120.00` | 264 px | 264 px（不含汉字 ⇒ 两栏相同） |

**结论（直接决定判据的形状）**：缺 CJK 字体时不是"画成豆腐块"，而是**什么都不画**
（连 `.notdef` 方框都没有）⇒ "墨迹 >= 150 px" 这条量化判据**真的能检出它**：
无字体 24 px（差 6×）、有字体 ≈590 px（余 4×）。
`ai-rail-diagnose-button`（12 汉字）与 `status-bar-chord`（2 汉字 + 5 ASCII）的**对比**判据
在两侧分别是 24/119 = 0.20 与 ≈590/225 ≈ 2.6 ⇒ 方向也毫无歧义。

**限制（明说，写进断言消息与 needs）**：它证明"汉字字形真的被画出来了"，
但**不能**逐字形比对（那需要一份人类批准的参考图样，D24 原文的"与已知 tofu 图样比对"）。
另外它**不**覆盖 `font-ui` 回退链里"用了哪个字体"（PingFang vs Noto 的字形差异只能由分平台 Golden 管）。

### 6.3 第 3 轮要验的三件事（第 2 轮因 clippy 提前中止，所以顺延）

1. **rebase 到含 D24 的 main** ⇒ 该 job 的 apt 步骤装上 `fonts-noto-cjk`
   ⇒ 汉字第一次真的被栅格化 ⇒ **三个状态的像素指纹必然与第 1 轮不同**
   （第 1 轮 `5e6020089976cb69` / `06bce1e2e0f6cec6` / `14559b0cb92839d6`）。
   指纹变了本身就是"字体确实生效"的证据；若指纹**没变**，说明装字体那一步无效（要查 apt 步骤）。
2. **新判据必须绿**：`cjk_ink >= 150` 且 `cjk_ink > reference_ink`（§6.2 两侧的余量）。
3. **`clippy --workspace -D warnings` 绿**（第 2 轮就是死在这里 —— 未使用常量）。

若第 3 轮仍然红：**残留一定不是本线的**（读法见上面"main 变红时的读法"），
此时本线的正确处置是**如实报告 + 不动别人独占的文件**，而不是去改 `.github/**` 或 `crates/yeban-mcp/**`。

---

## 7. 边界（本线**不做**什么）

1. **不判"界面好不好看"**：没有基准图，`[UI-MCP-003]` 的分平台 Golden 需要人类先提交基准；
   本线只证明"界面真的被渲染过、而且树和像素来自同一个实例"。
2. **不发明父节点**：运行时 `ElementHandle` 没有 parent 访问器、app 注册表也没有父字段
   ⇒ `ControlNode::parent` 恒为 `None`（沿上一条线的结论）。
3. **不给 `ControlNode` 加 `source` 字段**：`ElementMeta::component`（`.slint` 文件名）在
   test-port 的节点模型里没有对应字段，断言失败时无法从 JSON 直接映射回源码文件（模型演进，需集成者裁决）。
4. **不改 `ui/**.slint`**：本线只渲染与断言，不动界面（`session_view.slint` 的
   `workspace-session-canvas` 等元素留在原地）。
5. **不动 `.github/**`**：CI 步骤的取舍留给集成者（§2 已经让它不必再取舍，但仍由集成者定）。

---

## 8. needs（需要别人做 / 需要人类裁决）

0. ~~集成者裁决：是否保留 §2 的 `[dev-dependencies]` 第二入口~~ —— **已裁决（第 1 轮后）：保留**，
   且不再加 `ci.yml` 专用步骤（理由与回退办法见 §2）。
1. ~~`ci.yml` 的 feature 步骤~~ —— 集成者已决定**不加**：判据现在走自动发现的
   `tests/real_ui_tier1.rs`，`cargo test -p yeban-app --all-targets` 就会跑到它。
   `[[test]] test_port_adapter` 仍保留给显式 `--features ui-test-port` 的用法。
2. **`ControlNode::parent` 永远是 `None`**（§7.2）：需要上游暴露 `parent_item`，
   或 app 注册表补父 ID 字段。属规范/模型缺口，本线不发明。
3. **`ElementMeta::component` 进不了控件树 JSON**（§7.3）：建议给 `ControlNode` 加可选
   `source` 字段，或让断言脚本另外读 `elements.rs` 的注册表做映射。模型演进，需裁决。
4. **分平台 Golden 的尺寸策略**：stored deflate 下 1920×1080 ≈ 6.2 MB/张，
   接近红线 9 的 10 MB。本线的截图一律落在 `target/ui-test-port/`（不进仓库）；
   要在仓库里维护基准图，仍需裁决"多小的截图算合格基准"或"是否允许引入压缩器"。
5. ~~人类裁决：要不要用 `configure_test_fonts()`~~ —— **已由 ADR-0001 D24 裁决：不采纳**
   （内部 feature 会让测试字体与生产字体不一致）。本线的三张截图仍然是**字体依赖环境**的，
   因此只能做**人眼复核**，不能直接当基准图 —— 这是 D24"分平台 Golden"的直接后果。
6. ~~重复族语义寻址的定论~~ —— **已由第 1 轮 CI 实测关闭**：`for` 循环的每个实例都能被语义 ID
   寻址（6/3/4/8/5 个），`[UI-TEST-001]` 的三个族全都可用（见 §6.1）。
7. **D24「与已知 tofu 图样比对」这一层还没做**：§6.2 的判据证明"汉字被画出来了"，
   但**不能**逐字形比对。要真正区分"正确字形"与"错误的回退字形（例如全部落到某个只有
   假名的字体）"，需要一份**人类批准**的参考图样 + 分平台基准（与 needs 4 是同一件事的两面）。
8. **`fonts-noto-cjk` 的安装效果本身要由本线第 2 轮 CI 来证**（D24 自己写的 pending）。
   §6.3 给了判定方法：三个状态的像素指纹必须与第 1 轮**不同**，且新判据必须绿。

## 9. pending（未证实的、已知的债）

1. **第 2 轮 CI 判决**：见 §6 —— 未读到之前一律 `pending`。
2. **`slint` 在任何真实屏幕（winit/femtovg 后端）上的渲染**仍未验证 —— 本线只走
   `set_platform` + `MinimalSoftwareWindow` 的 Tier-1 无头路径（`[MUST-GATE-015]` 要求的正是它）。
3. **跨 runner 的指纹一致性**：第 1 轮只有一个数据点。"同一字体环境下 Tier-1 光栅化确定"
   这件事本线能给的证据是"同一进程内两次截图逐字节相同"（`true`）；跨 runner 的比较要等
   第 2 轮（且第 2 轮换了字体环境 ⇒ 与第 1 轮**本来就不该相同**，见 §6.3）。
4. **本线的 `clippy -D warnings`**：第 1 轮已验证绿（`rust (yeban-app)` job 里 clippy 先跑且通过）；
   第 2 轮新增的 `inset_rect` / `ink_stats` / 三处断言由第 2 轮 CI 判。

## 10. TODO(hoist) —— 建议集成者提升到共享账本 / ADR

1. **hoist → ADR-0001 D18**：§3 的 A1-A5 五条（`visible:` 的 lower 语义 + 上游回归测试背书）。
   它决定所有 UI 线的控件树判据形状，不该只活在本文件里。
2. **hoist → `docs/DEVELOPMENT_LEDGER.md`**：把 run 37223586792 那段"适配器编译失败、CI 暂时
   移除该步骤"的 pending，替换为**第 1 轮判决 + §6.1 的实测数字**，并把
   "验证命令是 `cargo test -p yeban-app --all-targets --locked`（不需要 feature）"写进命令表。
3. **hoist → needs**：~~`[UI-TEST-001]` 的重复元素寻址边界~~ —— 已关闭（§6.1）。
   替代它的是 D24 的第 2 层（"与已知 tofu 图样比对"，§8 第 7 条）。
4. **hoist → `docs/DEV_WORKFLOW.md`**（可选）：`required-features` 目标在 CI 里"绿着跳过"是
   一类系统性盲区（本线实测踩到）。建议在"多线纪律"里加一句：**判据必须挂在 CI 会执行的目标上**，
   否则它等于注释。
5. **hoist → ADR-0001 D24 的执行段**：`fonts-noto-cjk` 的安装效果已由本线第 2 轮给出判定方法
   （指纹必须变 + 墨迹从 24 px 变成数百 px）。建议把这组数字写进 D24，作为"环境依赖真的生效"
   的可核验判据。

