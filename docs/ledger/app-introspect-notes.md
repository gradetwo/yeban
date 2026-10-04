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
| `crates/yeban-app/src/test_port_adapter.rs` | `UI-TEST-001` `UI-MCP-001` `UI-MCP-002` `UI-MCP-003` `MUST-GATE-015` `ARCH-UI-005` | **修到能编译**（8 处错误）；真实 `MainWindow` 的三种状态 Tier-1 截图；运行时控件树 vs 注册表的**实测**覆盖判据；动态遮罩 + SSIM 判据；`ReadOnly` 权限判据 |
| `crates/yeban-app/tests/real_ui_tier1.rs`（新建） | 同上 | 同一份判据的**第二个 cargo 目标**（自动发现 ⇒ CI 上真的执行） |
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

**若集成者不采纳这个第二入口**：删掉 `crates/yeban-app/tests/real_ui_tier1.rs` 与
`Cargo.toml` 里的 `[dev-dependencies]` 两段即可（判据本身不动），代价是回到"必须先在
`ci.yml` 里加回 feature 步骤才可能知道它能不能编译"。这是一个显式的取舍，不藏在提交里。

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

**实测计数在 §6 的 CI 判决里逐条记录** —— 这一条一旦有数，"重复元素能否逐个语义寻址"就从
pending 变成事实，`[UI-TEST-001]` 可用/不可用的边界也就有了定论。

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
# 判据: 6 条(计算/选块/遮罩+SSIM) + 2 条(ID 清单)
./extract.sh && rustc --edition 2024 --test -D warnings -L dependency=$DEPS \
  --extern yeban_ui_test_port=/tmp/libyeban_ui_test_port.rlib main.rs -o /tmp/ai_pure_tests \
  && /tmp/ai_pure_tests --nocapture
./extract_ids.sh && rustc --edition 2024 --test -D warnings \
  --extern yeban_app=/tmp/libyeban_app_local.rlib idcheck.rs -o /tmp/id_check && /tmp/id_check
```

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

### 5.2 变异测试（"从没红过的判据是注释"）

变异只作用在**生成物副本**（`extracted.rs` / `id_lists.rs`），仓库文件全程未被改动；
每次变异后都重新生成并确认恢复为绿。

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

### 5.3 本机**没有**验证的（交给 CI，逐条说清）

- `test_port_adapter.rs` 与 `tests/real_ui_tier1.rs` 的**编译正确性**（含 Slint ⇒ 本机禁止编译）；
- `ElementHandle` 的真实遍历结果：树有多少节点、哪些 ID 可寻址、重复族计数（§4）；
- Tier-1 光栅化的真实像素：尺寸 / 非黑占比 / 颜色数 / PNG 字节数 / 确定性；
- 真实界面上的遮罩矩形与 SSIM 数字（本机用的是合成画面，见 §5.1 的说明）；
- `clippy -D warnings`。

### 5.4 本机跑过的门禁

- `bash scripts/gates/run-gates.sh light`：**通过**（fmt / 13 条守卫 / 文档 / 依赖许可清单）。
- `cargo metadata --offline`：解析通过（`cargo-local.sh`）；`Cargo.lock` **无变化**。
- `cargo tree -p yeban-app -e normal --locked`：不含 `yeban-ui-test-port`（红线 6 的证据）。

---

## 6. CI 判决与**真实界面的实测数字**

> 这一节在读到判决后逐轮追加；未读到的判决一律记 `pending`。

| 轮 | commit | run | 结论 | 说明 |
| :-- | :--- | :--- | :--- | :--- |
| 1 | `pending` | `pending` | `pending` | 说明：本线**第一次**让判据真的进入 CI 的可执行目标（见 §2），所以这一轮同时是"能编译吗"和"数字是多少"的首次判决 |

**判据脚本读回来的实测数字会贴在这里**（`scripts/dev/ci-verdict.sh line/app-introspect` +
`gh run view <run-id> --log-failed` / `--log`）：

- `[MUST-GATE-015]` 三个状态的 Golden 证据（尺寸 / 非黑占比 / 颜色数 / PNG 字节数 / 指纹）；
- 控件树计数（注册表 N / 运行时 M / 未登记 U / 缺失 K）与各重复族计数；
- 可见单例覆盖率；
- 遮罩与 SSIM 的四组数字。

产出 artifact（人眼复核用，30 天）：`ui-screenshots-workspace` 或 `ui-screenshots-yeban-app`
（取决于 plan 走全量腿还是矩阵腿），内含：

- `app-main-window-arrangement-full-1920x1080.png`（默认演示视图）
- `app-main-window-arrangement-compact-1920x1080.png`（`compact` 断点：左栏 36px 导轨）
- `app-main-window-session-full-1920x1080.png`（Session 视图，`session_view.slint` 首次被渲染）
- `app-registry-control-tree.json` / `app-runtime-control-tree.json`（两份树，可逐条 diff）
- `app-introspect-observations.txt`（本线的全部实测观察值）

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

0. **集成者裁决：是否保留 §2 的 `[dev-dependencies]` 第二入口。**
   保留 ⇒ 判据在本线 CI 里真的执行（这一轮的判决因此有内容）；删除 ⇒ 回到"必须先改
   `ci.yml` 才知道能不能编译"。两条路本线都已给出（删除步骤见 §2 末尾）。
1. **`ci.yml` 的 feature 步骤**：`cargo test -p yeban-app --features ui-test-port --locked`
   现在**可以加回**了（判据已在 `--all-targets` 下真实跑过，见 §6）。
   加了它，`[[test]] test_port_adapter` 那条显式目标也会被执行（两个目标跑同一批判据）。
2. **`ControlNode::parent` 永远是 `None`**（§7.2）：需要上游暴露 `parent_item`，
   或 app 注册表补父 ID 字段。属规范/模型缺口，本线不发明。
3. **`ElementMeta::component` 进不了控件树 JSON**（§7.3）：建议给 `ControlNode` 加可选
   `source` 字段，或让断言脚本另外读 `elements.rs` 的注册表做映射。模型演进，需裁决。
4. **分平台 Golden 的尺寸策略**：stored deflate 下 1920×1080 ≈ 6.2 MB/张，
   接近红线 9 的 10 MB。本线的截图一律落在 `target/ui-test-port/`（不进仓库）；
   要在仓库里维护基准图，仍需裁决"多小的截图算合格基准"或"是否允许引入压缩器"。
5. **人类裁决：要不要用 `configure_test_fonts()` 消除跨平台字体差异**（上一条线 §2 第 21 条）。
   本线的三张截图**字体不确定**（系统字体回退），因此它们只能做**人眼复核**，
   不能直接当基准图 —— 这一点是 `[UI-MCP-003]` 分平台要求的直接后果。
6. **重复族语义寻址的定论**（§4）：本线的实测计数会给出事实；若上游确实只让第一个实例可寻址，
   则 `[UI-TEST-001]` 点名的 `note-{ulid}-rect` / `clip-{ulid}-header` / `track-{i}-header`
   需要替代机制（`find_by_element_id` + 索引，或路径选择器），**需人类裁决**。

## 9. pending（未证实的、已知的债）

1. **CI 判决**：见 §6 —— 未读到之前一律 `pending`。
2. **`slint` 在任何真实屏幕（winit/femtovg 后端）上的渲染**仍未验证 —— 本线只走
   `set_platform` + `MinimalSoftwareWindow` 的 Tier-1 无头路径（`[MUST-GATE-015]` 要求的正是它）。
3. **`render.rs` / `inspect.rs` 的编译**只经过"逐条对源码"的核验，第一次真判决在 CI。
4. **本线的 `clippy -D warnings`**：新增/改写的判据代码没在本机过 clippy（含 Slint）。
   上一条线的教训（`chunks_exact_to_as_chunks`、`useless_conversion`）说明本机 clippy 探针
   只覆盖零 Slint 的模块；本线新增的 Slint 侧代码靠 CI。

## 10. TODO(hoist) —— 建议集成者提升到共享账本 / ADR

1. **hoist → ADR-0001 D18**：§3 的 A1-A5 五条（`visible:` 的 lower 语义 + 上游回归测试背书）。
   它决定所有 UI 线的控件树判据形状，不该只活在本文件里。
2. **hoist → `docs/DEVELOPMENT_LEDGER.md`**：把 run 37223586792 那段"适配器编译失败、CI 暂时
   移除该步骤"的 pending，替换为**本线 CI 判决 + 实测数字**（§6），并把"要验证它必须跑
   `cargo test -p yeban-app --all-targets --locked`"写进命令表。
3. **hoist → needs**：`[UI-TEST-001]` 的重复元素寻址边界（§8 第 6 条）是**规范级**问题，
   一旦 §6 给出计数就应升级为需人类裁决项。
4. **hoist → `docs/DEV_WORKFLOW.md`**（可选）：`required-features` 目标在 CI 里"绿着跳过"是
   一类系统性盲区（本线实测踩到）。建议在"多线纪律"里加一句：**判据必须挂在 CI 会执行的目标上**，
   否则它等于注释。
