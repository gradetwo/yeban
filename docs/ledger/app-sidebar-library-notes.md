# `sidebar-library` 工作线台账 —— 左侧资源栏条目接**真实投影**

- **台账类型**：报告条目 ↔ 代码落点 ↔ 判据 ↔ 未决项（**不是规范**）
- **工作线**：`line/sidebar-library`（worktree `.worktrees/sidebar-library`）
- **规范来源**：`UI-GRID-001`（视口/断点下的内容真实性）、`UI-TEST-001`（语义 ID 寻址）、
  `UI-A11Y-004`（屏读文案与点击源同落点）、`MODEL-ISO-001`（视图态与工程内容隔离）、
  `UI-MCP-003`（分平台基准）

---

## 1. 这一片修的是什么

`ui/sidebar.slint` 此前是**内联演示数据**：`for item_index in 8`（文件头注释自己写着
"真实资源索引要等扫描器落地"）。⇒ 界面上那 8 条与工程/库里真实存在的东西**无关**，
点哪一条都只写一个下标。

| 落点 | 改动 | 判据 |
| :--- | :--- | :--- |
| `src/bridge.rs` | `SidebarCategoryContent` ＋ `sidebar_category_content(view, category)`：分类 0 = 内置乐器库扫描、1 = 素材池（工程音频素材）、2 = 空态理由（模型/扫描缺号，⛔ 不发明条目） | 条数与投影**同源**由 `elements.rs` 的 `model_driven_families_scope_is_exact` 末尾派生断言钉住 |
| `src/host.rs` | 新 `wire_sidebar_contents(ui, Rc<RefCell<ViewState>>)`：分类点击 ⇒ 写 `sidebar-category` ＋ **重注入** `sidebar-items` / `sidebar-empty-reason` ＋ 把 `sidebar-item` 钳到**新集合**；条目点击 ⇒ 按**当次投影条数**钳位后写 `sidebar-item`。⇒ 这两条回调从 `wire_view_callbacks` 搬出（同一回调只有一个注册者）；旧常量 `SIDEBAR_ITEM_COUNT = 8` **删除** | `tests/sidebar_category.rs::clicking_a_sidebar_category_switches_the_view_state_and_the_highlight`、`tests/sidebar_item.rs::sidebar_items_are_the_projection_and_clicking_one_moves_only_the_view_state` |
| `ui/sidebar.slint` | 数组驱动（`for item_text[item_index] in root.items`）＋ `sidebar-empty` 理由节点 | 同上 |
| `src/main.rs` / `src/live_surface.rs` | 产品进程与无头装配调**同一个** `wire_sidebar_contents`（装配视图快照作回落） | 判据里注入的点击与真实点击同落点 |
| `src/elements.rs` | `("sidebar-item-", "")` 进 `EMPTY_DYNAMIC_TEMPLATES_ALLOWED`（**单段**模板 = 后缀空串） | 防陈旧检查复用同源形态判据 |

⚠ **为什么 `("sidebar-item-", "")` 是登记而非放行**：CI / 无头机器的采样库根没有下载
⇒ 分类 0 今天**确实** 0 条 ⇒ 模板实例化 0 次，与 `#165` 的
`instrument-install-*-button` 完全同构。空态另有 `sidebar-empty` 注册与判据。

**像素面**：默认帧**会变**（8 条演示字 ⇒ 真实投影 / 空态理由块）⇒ 本片**必须**重录
Linux 基准（`[UI-MCP-003]`），⛔ 与"零视觉新增元素"的那些片子不同。

## 2. 与主线漂移的合流（`1000fa00`，2026-11-01）

分支基线是 `c9e591e5`，主线此后领先 **241** 个提交。做法：`git merge origin/main`
**进分支**（⛔ 不做 9 个提交的 rebase —— 每个提交都要把同一批文件重解一遍）。
`git merge-tree` 预先量过冲突面：8 个文件重叠、其中 **4** 个真的内容冲突。

| 冲突文件 | 取舍 |
| :--- | :--- |
| `src/host.rs` | 三条回调的**唯一注册者**契约要同时满足两侧的搬迁：抽屉四条归主线新出的 `wire_proposal_drawer`，侧栏两条归本线新出的 `wire_sidebar_contents`，`wire_view_callbacks` 里两处都只剩"不在本函数"的说明。**手法**：先取主线的整份文件，再把本线 `git diff c9e591e5..b6eed1ca -- host.rs` 以 `git apply -3` 打上 ⇒ 原本 6 处**交织的函数体**冲突降到 2 处（⛔ 手工编织两份完整闭包，那种解法出错率极高） |
| `src/elements.rs` | 例外清单取**并集**（本线 `("sidebar-item-", "")` ＋ 主线两条 `musical-pr-proposal-*`），两侧理由都保留 |
| `src/main.rs` / `src/live_surface.rs` | 同一位置的**两个**装配调用都保留（`wire_sidebar_contents` ＋ `wire_proposal_drawer`）；回调归属文档表改成合并后的事实 |

本机已判定（⛔ 本机不做 Slint 编译 ⇒ 编译与判据一律交 CI）：
4 个解冲突文件 `rustfmt --edition 2024 --check` 通过；`scripts/guards/policy_check.py`
**20 条守卫全过**。

## 3. 判决与未决项

| 项 | 状态 |
| :--- | :--- |
| 合流后的 CI 判决（本分支 tip） | `pending` —— ⛔ 写成通过之前必须读回 `conclusion` 并数 `Running tests/` 里**有** `sidebar_item.rs` / `sidebar_category.rs` |
| 上一票为何不算数 | `b6eed1ca` 的 run 死在**链接期** `ld … signal 7 [Bus error]`（job `124408326215`）⇒ 按 §6.3 第 4 条那是"没跑到"，既不是失败也不是通过 |
| 基准重录 | **未做**。`tests/golden/linux/` 那 5 张是本分支对**旧 main** 录的；main 之后改了 `transport.slint` / `app.slint` / `console/mixer_console.slint` / `console/piano_roll.slint` ⇒ 默认帧字节必变。路径：`gates-manual.yml` 的 `gate=goldens`（`YEBAN_WRITE_GOLDEN=1 cargo test -p yeban-app --locked --test real_ui_tier1`）在 Linux 上生成 → 下载 artifact `golden-linux` → 复核后入库。本机生成不了（Linux 基准只能在 Linux 上录，macOS 走 CoreText 字形不同） |
| 与另两条线的顺序 | ⛔ 不与 `line/arrangement-scroll` / `line/mixer-horizontal-scroll` 并发推 `host.rs` 相关批次；那两条落地后本分支还要再并一次 main（增量很小），再走 `gate=goldens` |
