//! ⭐ 任务单 `#186`：**其余面板段内的控件两两不许重叠**。
//!
//! `#184` 钉住了"设备架两段之间"不相交、`#185` 钉住了"发送条内"两两不相交；
//! 本文件把同一条**跨节点**判据铺到另外四段：
//!
//! | 段 | 运行时容器（取控件的那一格） | 读法 |
//! | :--- | :--- | :--- |
//! | 钢琴卷帘 | `piano-roll` 的包围盒 | 与盒子**相交**的非容器节点 |
//! | 调音台 | `mixer-console` 的包围盒 | 与盒子**相交**的非容器节点 |
//! | 素材池 | `asset-pool-panel` 的包围盒 | 与盒子**相交**的非容器节点 |
//! | 走带 | 顶栏 `y ∈ [0, 48)` | 包围盒**完全落在**该带内的非容器节点 |
//!
//! ## 判据（逐条对应任务单 §2）
//!
//! | §2 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 扫描读数 | 三档 `1920×1080 / 1440×900 / 1024×800`：每段的控件数 / 对数 / 相交对数逐档 `report_line` | 证"这四段真的被数到了"；⛔ 不证段外控件 |
//! | ② 修掉的逐条 | `#186` 的两处根因：走带徽章 ⟂ Session（1024 档 1 对）、主控通道条 ⟂ 工程/素材池面板（1024 档 8 对）—— 修前/修后 bounds 写在本文件的文档与报告里，修后由 ③ 逐对断言 | 证"改的是这处"；⛔ 不证视觉美观 |
//! | ③ 两两不相交 | 每段：排除容器后**逐对**断言包围盒不相交（半开区间，边贴边不算），三档全是 0 | 证"同一段内没有第二处重叠"；⛔ 不证与段外邻居（`#184` 管段间） |
//! | ④ 容器排除 | 见 [`is_container_role`]：`region` / `groupbox` / `list` / `tab-list` / `main` / `complementary`。**`list-item` 不排**（这四段里它是叶子内容：音符矩形、素材行） | 证"排的是按设计包住子节点的容器"；⛔ 不排 `list-item` 是有意的（与 `#185` 的发送行相反） |
//! | ⑤ 不许假绿 | 每段先断言"必须在场的 id 一个不少"＋"控件数恰好等于清单"；1024 档走带徽章**按设计隐藏**（不是扫不到），并断言替代入口 `ai-rail-musical-pr-button` 在树里 | 证"0 相交不是 0 控件换来的" |
//! | ⑥ 预算 | 纯测试 ＋ `.slint` 表达式级布局微调（不新增控件） ⇒ 控件数逐档不变：调音台 `72`、钢琴卷帘 `20`、素材池 `3`、走带 `14`（1024 档 `13`，少的是按设计隐藏的徽章） | 证"没有靠删控件换绿" |
//!
//! ## 发现（`#186` 的实测；数字是逻辑像素包围盒 `(x, y, w, h)`）
//!
//! ### 1. 走带：AI 提案徽章 与 Session 视图键（1024 档）
//!
//! ```text
//! 修前：transport-ai-proposal-badge      (640, 8, 120, 32)  ⟂
//!       transport-view-session-button    (720, 8,  92, 32)   ⇒ 相交 40×32 px²
//! 修后：徽章 visible: root.width >= 1064px（1024 档不在树里）；Session 仍在树里。
//! ```
//!
//! 算术：徽章右沿 `760`；Session 左沿 `root.width - 304`。`width < 1064` 时必交。
//! 隐藏的是**重复入口**：1024 档右栏 `ai-rail-musical-pr-button` 仍在树里。
//!
//! ### 2. 调音台：主控通道条 与「工程」/「素材池」面板（1024 档，8 对）
//!
//! ```text
//! 修前（1024）：mixer-master-pan          (918, 523,  30,  14) ⟂ module-open-project-button (860, 527, 204, 28)
//!              mixer-master-mute-button   (918, 541,  20,  16) ⟂ module-open-project-button (860, 527, 204, 28)
//!              mixer-master-solo-button   (942, 541,  20,  16) ⟂ module-open-project-button (860, 527, 204, 28)
//!              mixer-master-color-swatch  (960, 523,  10,  14) ⟂ module-open-project-button (860, 527, 204, 28)
//!              mixer-master-fader         (944, 571,  22,  16) ⟂ module-new-project-button  (860, 559, 204, 28)
//!              mixer-master-meter         (918, 563,  14, 104) ⟂ module-new-project-button  (860, 559, 204, 28)
//!              mixer-master-meter         (918, 563,  14, 104) ⟂ asset-pool-count          (860, 635, 204, 16)
//!              mixer-master-meter         (918, 563,  14, 104) ⟂ asset-pool-row-0-row      (860, 657, 204, 30)
//! 修后（1024）：工程面板 (632, 90, 216, 104)、素材池 (632, 200, 216, 109)、
//!              主控通道条 x=912 ⇒ 面板右沿 848 < 912，8 对全部消失。
//! 宽档（≥1182）：工程面板仍是 (854, 8, 216, 104)、素材池仍是 (854, 118, 216, h-128)，
//!              一位未动。
//! ```
//!
//! ## 反向腿（去掉本片改动 ⇒ 哪条红）
//!
//! 两条注入都**实测**过（读数见交付报告 §5）：
//!
//! - 注入 A：删掉走带徽章的 `visible: root.width >= 1064px;` ⇒ **走带判据红**：
//!   `transport-ai-proposal-badge (640,8,120,32) ⟂ transport-view-session-button (720,8,92,32)`；
//! - 注入 B：把 `mixer_console.slint` 的 `narrow: root.width < 1182px` 改成 `< 0px`
//!   ⇒ **调音台判据红**：上面那 8 对在 1024 档逐条回来（素材池判据也红 2 对）。
//!
//! ## 它**不**证什么
//!
//! ⛔ `#186` **当时**只扫了这四段：设备架两段（`#184`）、发送条（`#185`）、
//! 以及本文件的四段；左栏 / 右栏 / 编曲画布 / 装乐器行 / 轨路由四键区
//! **当时未扫** —— 那五段由 `#187` 补扫（见下面的 `#187` 一节）；
//! ⛔ 没人耳听过（本机 Tier-1 软件光栅化，没有声卡）；⛔ 不证 Linux Golden：
//! 本机没有 `tests/golden/macos/**`；⛔ 不证 1024 以下——`#186` 只验三档，更窄档未登记。
//! ⛔ 不证轨道数 > 6 时的调音台：窄档竖排的算术是按演示工程 `n = 6` 推的，
//! 更多轨道会把通道条推过主控通道条（本判据不覆盖，如实登记）。
//!
//! ---
//!
//! # ⭐ 任务单 `#187`：**同一文件扩段** —— 剩余六段
//!
//! `#186` 的四段（钢琴卷帘 / 调音台 / 素材池 / 走带）之外，本单把同一条
//! **跨节点**判据铺到 `#186` 清单里剩下的段：
//!
//! | 段 | 段盒子（取控件的那一格） | 取法 | 清单 |
//! | :--- | :--- | :--- | :--- |
//! | ① 设备链卡片区 | 第一段左列 `x ∈ [0, 428)`、`y ∈ [插入行顶, 面板底)` | 相交 | 演示工程 7（1024 档 6）/ 空链 5 / 只有主总线 7 |
//! | ② 装乐器行 | `instrument-label` 那一行（20 ＋ 2 ＋ 14 ＝ 36px） | 相交 | 3（内置清单为空 ⇒ **0 个装乐器按钮**；⭐ `#196` 起同一行有"下载采样库"按钮 ＋ 未安装提示，如实登记） |
//! | ③ 轨路由四键区 | `track-routing-panel` 的包围盒（`#160`） | 相交 | 6 / 只有主总线时 7（多逐字原因） |
//! | ④ 编曲画布 | 三组：轨道头列 / 剪辑头 / 自动化泳道 / 章节卡 | 分组逐对 | 头 6、头内子节点 18、剪辑 3、泳道 3、章节 4/3/2 |
//! | ⑤ 状态栏 | 24px 带 `y ∈ [h−24, h)` | **Inside** | 4（提示态）/ 4（保存回执态） |
//! | ⑥ AI 右栏 | `ai-rail` 的包围盒 | 左沿在盒子内 | 12（1024 档 11，少的是被裁剪的 `module-last-status`） |
//!
//! ## 本单实测发现（修前 → 修后，逐条）
//!
//! ### 1. AI 右栏：导出诊断包 ⟂ 声学诊断卡（**三档全中**，20 × 272 px²）
//!
//! ```text
//! 修前（1920）：ai-rail-diagnose-button   (1648, 152, 272, 64) ⟂
//!              diagnostics-export-action (1648, 196, 272, 28)   ⇒ 相交 y 196..216
//! 修后（1920）：diagnose (1648,152,272,64) / export (1648,220,108,28) /
//!              musical-pr (1756,220,148,28) ⇒ export ⟂ musical-pr 也不交（x 分开）
//! ```
//!
//! 根因：`21592ff` 把导出键插在 `y: 148px`，而诊断卡住 `104..168`。竖向预算在
//! 1440 档只剩 7px ⇒ 不能整体下移，改成**同一行并排**（不占新高度）。
//!
//! ### 2. 设备链卡片区：插件占位卡 ⟂ 空态文案（空链时，16 × 420 px²）
//!
//! ```text
//! 修前（1024 空链）：device-rack-empty (8, 711, 420, 20) ⟂
//!                  device-plugin-host-placeholder-card (8, 715, 420, 22)
//! 修后（1024 空链）：empty (8,711,420,20) / placeholder (8,735,420,22) ⇒ 4px 间隙
//! 顺带（1024 演示 1 台设备）：占位卡原式 761..783 越过面板底 776（7px 被 clip 吃掉）
//!                  ⇒ 放不下就整张撤下（该档清单从 7 变 6，⛔ 不是静默消失）
//! ```
//!
//! ### 3. 状态栏：快捷键提示 ⟂ 音频配置（**只有 1024 档**，68 × 24 px²）
//!
//! ```text
//! 修前（1024）：status-bar-shortcut-tip (344, 776, 480, 24) ⟂
//!              status-bar-device       (756, 776, 256, 24)   ⇒ 相交 x 756..824
//! 修后（1024）：shortcut-tip (344,776,412,24) / device (756,776,256,24)
//!              ⇒ 右沿 756 = 左沿 756（边贴边，⛔ 不算相交）
//! 保存回执态同样收窄（两格共用 `tip-width`）；1920/1440 的 480px 一位未动。
//! ```
//!
//! ## 反向腿（去掉本片改动 ⇒ 哪条红；读数见交付报告 §5）
//!
//! - 注入 A：把 `status_bar.slint` 的 `tip-width` 换回 `480px` ⇒ **⑤ 红**（1024 档 1 对，bounds 同上）；
//! - 注入 B：把 `device_rack.slint` 的 `placeholder-y` 换回 `216 + 46n + 4` 并去掉
//!   `visible` ⇒ **① 红**（空链 1024 档 1 对；演示 1024 档另加"越出面板底"1 条）；
//! - 注入 C：把 `app.slint` 的导出键 `y` 换回 `148px` ⇒ **⑥ 红**（三档各 1 对）。
//!
//! ## 它**不**证什么（`#187` 追加）
//!
//! ⛔ 左栏 `sidebar*`、`console-tab-rail` / `tab-*`、Session 视图、模块动作面板的
//!   **横向**邻居（`module-*` 已由 `app.slint` 的绝对定位钉住，但本单只按右栏盒子判）；
//! ⛔ 1024 以下（`#186` 只验三档）；⛔ 没人耳听；⛔ 无 Linux Golden 重录。
//!
//! ---
//!
//! # ⭐ 任务单 `#189`：**最后剩下的面板段** —— 同一文件扩段
//!
//! `#187` §「它不证什么」逐条列出的未扫段，本单按优先级补扫：
//!
//! | 段 | 段盒子（取控件的那一格） | 取法 | 清单 |
//! | :--- | :--- | :--- | :--- |
//! | ① 左栏 `sidebar*` | `sidebar` 的包围盒 | 相交 | 展开 13 / 折叠 9 |
//! | ② `console-tab-rail` 带 ＋ `splitter-workspace-console` 带 | 导轨顶 28px / 分隔线 4px | **Inside** | 4 / 1 |
//! | ③ Session 视图 | `workspace-session-canvas` 的包围盒 | 相交 | 36（展开态 1024 只有 20，见下） |
//! | ④ 主总线设备链**段内子节点** | `master-device-rack` 的包围盒 | 相交＋容器裁决 | 6 / 7 / 6（三种夹具） |
//! | ⑤ `UndoTreeModal` | `undo-tree-panel` 的包围盒 | **id 前缀** ＋ Inside | 9（6 节点）/ 3（空链） |
//! | ⑥ `MusicalPrDrawer` | `musical-pr-drawer` 的包围盒 | **id 前缀** ＋ Inside | 3（零提案） |
//! | ⑦ `EngineErrorBanner` | `engine-error-banner` 的包围盒 | **id 前缀** ＋ Inside | 1 |
//!
//! ## 为什么 ⑤⑥⑦ 用 **id 前缀**取控件（而不是纯几何）
//!
//! 三个都是**覆盖层**（`app.slint` 末尾的「覆盖层」一节）：对话框的矩形与它**下面**的
//! 控制台 / 右栏在屏幕坐标里重叠 —— 但那是"盖住"，不是"同一段里两个控件抢同一格"。
//! 运行时树不会因为被盖住就删节点（`visible: false` 才删，覆盖层的兄弟不是 `false`），
//! 所以纯几何取法会把**两个段**混成一段。本单的取法：段内控件 = `id` 以该段前缀开头
//! **且**整个包围盒落在段盒子内；两条都写死（前缀少了会漏，盒子出界会红）。
//!
//! ## 本单实测发现（修前 → 修后）
//!
//! ### 1. 主总线设备链：装设备不可用原因 ⟂ 主总线 EQ 面板（三档全中，328 × 14 px²）
//!
//! ```text
//! 修前（1920 无主总线夹具）：master-device-insert-reason (848, 743, 528, 14) ⟂
//!                            master-device-eq-panel       (1040, 713, 328, 48)
//!                            ⇒ 相交 x 1040..1368 × y 743..757
//! 修后（1920）：reason (848, 743, 184, 14) ⇒ 右沿 1032 < 面板左沿 1040
//! 修前（1024）：reason (704, 575, 304, 14) ⟂ panel (888, 545, 112, 48)
//! 修后（1024）：reason (704, 575, 176, 14) ⇒ 右沿 880 < 面板左沿 888
//! ```
//!
//! 根因：原因行的宽度写的是 `parent.width - 16px`（整段宽），而 EQ 面板从
//! `master-rack-width - 328 - 16` 起 —— 面板是**不透明**的、在原因行**之后**绘制，
//! 于是原因行右侧 328px 被面板盖住。面板本身是 `groupbox`（按 role 被两两判据排除），
//! **它盖住的是别人的节点**，所以容器裁决救不了它：本单额外加了
//! 「容器 ⟂ 非子节点叶子」的显式断言（见 [`assert_container_clear_of`]）。
//! 修复只改宽度（`x`/`y`/高度一位未动）：`width: master-eq-panel-x - space-4 - space-4`。
//!
//! ### 2. `MusicalPrDrawer`：图例里「↑ 微调」与「遮罩 #000000」两行（4px 包围盒重叠）
//!
//! ```text
//! 修前（任意档，抽屉宽恒 360）：↑微调  y 16..32 × x 216..312
//!                             遮罩行 y 28..44 × x 152..344   ⇒ 竖向重叠 4px
//! 修后：遮罩行 y 40..56 ⇒ 与 ↑微调（16..32）、删除行（28..44）都不再相交
//! ```
//!
//! ⚠ 这两行是**装饰性文本**（没有 `accessible-id`，按 `inspect.rs` 的口径不进控件树
//! ⇒ **不能**被本文件的判据机械复判）。它是由"读源码 + 算包围盒"发现的，修后由
//! 布局算术钉住（y=40 ＋ 高 16 ≤ 图例高 68），**如实登记为不可判项**（报告 §9）。
//!
//! ## 反向腿（去掉本片改动 ⇒ 哪条红；读数见交付报告 §6）
//!
//! - 注入 A：把 `device_rack.slint` 原因行宽度换回 `parent.width - 16px`
//!   ⇒ **④ 红**：1920 / 1440 / 1024 三档各 1 对（bounds 同上）；
//! - 注入 B：把 `sidebar.slint` 分类标签的 `64px * category_index` 改成 `56px`
//!   ⇒ **① 红**：三档展开态各 2 对（`sidebar-category-0 ⟂ 1`、`1 ⟂ 2`，各 4 × 20 px²）。
//!
//! ## 它**不**证什么（`#189` 追加）
//!
//! ⛔ 主总线设备链 **> 4 台设备**：段高固定 260px、卡片 `72 + 46n` ⇒ 第 5 张卡起越过
//!   段底被 `clip` 掉（本单只验 0 / 1 / 2 台，未验 5+；这是溢出，不是重叠）；
//! ⛔ 装饰性文本（无 `accessible-id`）之间的包围盒重叠不可判（发现 1 处，见上）；
//! ⛔ 覆盖层"盖住下面"是设计（不判，也不许判成 0）；⛔ 1024 以下未测；⛔ 没人耳听。
//!
//! ---
//!
//! # ⭐ 任务单 `#190`：`#189` 登记的两条**溢出**缺口 —— 截断 ＋ 显式报数
//!
//! 产品裁决（人类）：**截断 ＋ 显式报数**，⛔ 不做滚动（`ScrollView` / 纵向可滚控制台会
//! 动 1920 基准帧且是大改；"装不下就明确告诉用户装了几台"诚实且便宜；滚动以后再加）。
//!
//! | 缺口 | 修前 | 修后 |
//! | :--- | :--- | :--- |
//! | ① 主总线卡片 | 段高 260、卡片 `72 + 46n` ⇒ 第 5 张 `256..300` 越过段底被 `clip` | 只画前 `N = floor((段高−72)/46)` 张（1920 档 **4**）；第 N+1 张不画；计数格改画 `"还有 N 台设备未显示"` |
//! | ② Session 场景列 | 场景列 `x = 8 + 128 × tracks.length` ⇒ 1440 档第 7 轨起 / 1024 展开态第 4 轨起被推出画布 | 列数 `N = floor((画布宽−112)/128)`；只画前 N 列；场景列由 N 定位（始终落在画布内）；下方报 `"还有 N 轨未显示"` |
//!
//! ## 判据（`#190` §2）
//!
//! | §2 | 断言 | 读数在哪 |
//! | :--- | :--- | :--- |
//! | ① 4 装得下 / 5 截断 | `check_master_device_truncation` 4 台 ⇒ 4 卡无提示；5 台 ⇒ 4 卡 ＋ 提示 | 报告 §4 |
//! | ② 文案逐字 ＋ id | `"还有 1 台设备未显示"` / `"还有 1 轨未显示"`；`master-device-truncation-note` / `session-track-truncation-note` | 上表 ＋ 运行时树 |
//! | ③ N 算出来 | 从运行时 `master-device-rack` 高度 / `workspace-session-canvas` 宽度反算，与 `.slint` 同一算术 | `report_line` |
//! | ④ 不重叠 | 5 台 / 7 轨两档夹具逐对 `0 相交`（提示计入清单） | `assert_pairwise*` |
//! | ⑥ 不动模型 | `master-device-names` / `track-names` 的 `row_count` 仍等于工程条目数 | `check_*_truncation` |
//!
//! **1920 基准帧**：`#315` 的回答 —— 演示工程主总线 0 台（≤ 4）、Session 6 轨（画布 1400
//! ⇒ 容量 10 ≥ 6）⇒ 两处的条件分支都不成立，1920 画面的几何/文案一位不动（见报告 §3）。

use std::collections::{BTreeMap, BTreeSet};

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_model::EntityId;
use yeban_model::project::{
    DeviceDefinition, DeviceKind, ParameterValue, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::inspect::tree_from_element_root;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::tree::{ControlNode, ControlTree};

/// 三档：默认档 ＋ 仓里界面判据的默认窗口 ＋ `#184`/`#185` 的窄档下界。
const TIERS: &[(u32, u32)] = &[(1920, 1080), (1440, 900), (1024, 800)];

const PIANO_TAB: i32 = 0;
const MIXER_TAB: i32 = 1;
/// 顶栏高度（`ui/tokens.slint` 的 `top-bar-height`；与 `app.slint` 的 48px 顶栏同一格）。
const TOP_BAR: u32 = 48;

/// 走带在宽档（`≥1064`）的 14 个控件；窄档（1024）少的是按设计隐藏的 AI 徽章。
const TRANSPORT_WIDE: &[&str] = &[
    "transport-play-button",
    "transport-stop-button",
    "transport-record-button",
    "transport-bpm-field",
    "transport-timecode",
    "transport-branch-button",
    "transport-commit-button",
    "transport-revert-button",
    "transport-save-button",
    "transport-ai-proposal-badge",
    "transport-view-session-button",
    "transport-view-arrangement-button",
    "transport-view-toggle-button",
    "transport-ai-drawer-button",
];

const PIANO_CONTROLS: &[&str] = &[
    "piano-roll-tool-select-button",
    "piano-roll-tool-pencil-button",
    "piano-roll-tool-knife-button",
    "piano-roll-tool-velocity-button",
    "piano-roll-tool-eraser-button",
    "piano-roll-status",
    "piano-roll-playhead",
    "note-suggestion-overlay-rect",
    "note-01J8Z5Q0R7K3M9X2V4B6N8P1A2-rect",
    "note-01J8Z5Q0R7K3M9X2V4B6N8P1A3-rect",
    "note-01J8Z5Q0R7K3M9X2V4B6N8P1B0-rect",
    "note-01J8Z5Q0R7K3M9X2V4B6N8P1C7-rect",
    "note-01J8Z5Q0R7K3M9X2V4B6N8P1D4-rect",
    "note-01J8Z5Q0R7K3M9X2V4B6N8P1E1-rect",
    "velocity-0-bar",
    "velocity-1-bar",
    "velocity-2-bar",
    "velocity-3-bar",
    "velocity-4-bar",
    "velocity-5-bar",
];

/// 素材池（演示工程 `clip_pool` 2 条）：条目数 ＋ 两行。面板本体是容器。
const ASSET_CONTROLS: &[&str] = &[
    "asset-pool-count",
    "asset-pool-row-0-row",
    "asset-pool-row-1-row",
];

/// 调音台的 72 个控件：3 个模块入口 ＋ 6 个主控控件 ＋ 6 条通道条 × 10。
fn mixer_controls() -> Vec<String> {
    let mut ids: Vec<String> = vec![
        "module-export-midi-button".to_owned(),
        "module-open-project-button".to_owned(),
        "module-new-project-button".to_owned(),
        "mixer-master-pan".to_owned(),
        "mixer-master-color-swatch".to_owned(),
        "mixer-master-mute-button".to_owned(),
        "mixer-master-solo-button".to_owned(),
        "mixer-master-meter".to_owned(),
        "mixer-master-fader".to_owned(),
    ];
    ids.extend(ASSET_CONTROLS.iter().map(|id| (*id).to_owned()));
    for track in 0..6 {
        for suffix in [
            "pan",
            "mixer-color-swatch",
            "mixer-mute-button",
            "mixer-solo-button",
            "meter",
            "fader",
            "sampler-path",
            "sampler-browse",
            "sampler-set",
            "sampler-clear",
        ] {
            ids.push(format!("track-{track}-{suffix}"));
        }
    }
    assert_eq!(ids.len(), 72, "调音台控件清单长度变了 ⇒ 判据口径要重核");
    ids
}

type Bounds = (i32, i32, u32, u32);

/// 读活窗口的运行时控件树（与 `#185` 同一个入口：`tree_from_element_root` + JSON 往返）。
fn runtime_tree(window: &MainWindow) -> ControlTree {
    let json = tree_from_element_root(window, None)
        .expect("运行时控件树")
        .dump_json();
    ControlTree::from_json(&json).expect("JSON 必须能被同一套校验读回")
}

/// 一条生产形态的 Tier-1 活窗口（唯一构造入口 `build_main_window_with_console_tab`）。
///
/// ⚠ 返回值里的 `LivePort` 必须被持有到判据结束；Tier-1 平台是**线程局部**的，
/// 一根线程只能装一次 ⇒ 每个 `(档, 标签)` 在自己的线程里跑。
fn live_window(size: (u32, u32), tab: i32) -> LivePort<MainWindow> {
    live_window_for(yeban_model::samples::demo_project(), size, tab)
}

/// 同 [`live_window`]，但换成指定的工程（`#187` 的空链 / 只有主总线两种夹具）。
fn live_window_for(project: YebanProjectV1, size: (u32, u32), tab: i32) -> LivePort<MainWindow> {
    let view = ViewState::from_project(&project).expect("演示工程必须能投影");
    let scene = DemoScene::from_view(&view);
    LivePort::new(
        Size::new(size.0, size.1),
        Permission::ReadOnly,
        None,
        || host::build_main_window_with_console_tab(&view, &scene, tab),
    )
    .expect("Tier-1 平台 + 生产窗口构造入口")
}

fn bounds_of(node: &ControlNode) -> Bounds {
    let b = node.bounds.expect("运行时节点必须有包围盒");
    (b.x, b.y, b.width, b.height)
}

fn bounds(tree: &ControlTree, id: &str) -> Bounds {
    let node = tree
        .find_by_id(id)
        .unwrap_or_else(|| panic!("运行时控件树里找不到 `{id}`"));
    bounds_of(node)
}

/// 两个包围盒是否相交（半开区间：边贴边**不算**相交）。
fn intersect(a: Bounds, b: Bounds) -> bool {
    let (ax, ay, aw, ah) = (
        i64::from(a.0),
        i64::from(a.1),
        i64::from(a.2),
        i64::from(a.3),
    );
    let (bx, by, bw, bh) = (
        i64::from(b.0),
        i64::from(b.1),
        i64::from(b.2),
        i64::from(b.3),
    );
    ax < bx + bw && bx < ax + aw && ay < by + bh && by < ay + ah
}

/// 容器角色：它们的包围盒**按设计**包住子节点，不参与"两两不相交"。
///
/// 逐条理由（这四段里真正被排掉的实例）：
///
/// - `region`：`mixer-console` / `asset-pool-panel` / `piano-roll` / `piano-roll-grid` /
///   `piano-roll-velocity-lane` / `piano-roll-scroll-area` —— 都是"段/画布/泳道"本身；
/// - `groupbox`：`track-{i}-channel-strip` / `mixer-master-strip` —— 通道条包住推子 / 电平 /
///   静音键；主控条的包围盒本来就该盖住自己的控件；
/// - `list`：`piano-roll-keys` —— 琴键列包住 16 个琴键；
/// - `tab-list` / `main` / `complementary`：`console-tab-rail` / `workspace-arrangement-canvas` /
///   `ai-rail` 在段外，但同一套规则让"取控件"不依赖调用点。
///
/// ⚠ **`list-item` 故意不排**：`#185` 的发送行是 `list-item` 且**包住**滑块 / 读数 / 删除键，
/// 所以那里必须排；本文件的 `note-{ulid}-rect` 与 `asset-pool-row-{i}-row` 是**叶子内容**
/// （子节点没有语义 id、不参与扫描）—— 排掉它们会把"音符压音符"变成永远绿。
fn is_container_role(role: &str) -> bool {
    matches!(
        role,
        "region" | "groupbox" | "list" | "tab-list" | "main" | "complementary"
    )
}

/// 取法：`Intersect` = 与段盒子相交即算段内（面板可能比窗口宽 / 被裁剪）；
/// `Inside` = 完全落在段盒子内（走带那条 48px 顶栏带、状态栏那条 24px 带，用它排除
/// 任何跨带节点）；
/// `LeftEdgeInside` = 与盒子相交**且左沿在盒子内**（⭐ `#187`）。
///
/// ⚠ `LeftEdgeInside` 为什么存在：运行时树给的是**未裁剪**的包围盒。编曲画布
/// `workspace-arrangement-canvas` 有 `clip: true`，它的越界子节点（`section-*-card`、
/// `arrangement-loop-brace`）在树里仍带着**越过画布右沿**的宽度 —— 1024 档
/// `arrangement-loop-brace (648,68,480,22)` 一直伸到 `1128`，而画布右沿是 `744`。
/// 这些像素被画布 clip 掉了，**⛔ 不与右栏重叠**，但它们的**包围盒**会落进右栏的段盒子。
/// 右栏的控件全部从 `ai-rail.x + 8` 起（左沿在盒子内），被裁剪的越界节点左沿在盒子外
/// ⇒ 这条取法把"像素真的在段内"与"包围盒越界"分开。这不是放宽：同一段内左沿在盒子里的
/// 控件**仍然逐对**断言（见下面的读数）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Selection {
    Intersect,
    Inside,
    LeftEdgeInside,
}

fn select_controls(
    tree: &ControlTree,
    pane: Bounds,
    selection: Selection,
) -> Vec<(&ControlNode, Bounds)> {
    let right = pane.0 + pane.2 as i32;
    let bottom = pane.1 + pane.3 as i32;
    tree.iter()
        .filter(|node| !is_container_role(node.role.as_str()))
        .filter_map(|node| node.bounds.map(|b| (node, (b.x, b.y, b.width, b.height))))
        .filter(|(_, b)| match selection {
            Selection::Intersect => intersect(*b, pane),
            Selection::Inside => {
                b.0 >= pane.0
                    && b.1 >= pane.1
                    && b.0 + b.2 as i32 <= right
                    && b.1 + b.3 as i32 <= bottom
            }
            Selection::LeftEdgeInside => intersect(*b, pane) && b.0 >= pane.0 && b.0 <= right,
        })
        .collect()
}

/// 判据 ③ 的唯一实现：段内非容器控件 → 清单必须一个不少 → 逐对断言不相交。
///
/// 返回可直接 `report_line` 的读数（控件数 / 对数 / 相交对数 / 逐条 bounds）。
fn assert_pairwise(
    tree: &ControlTree,
    pane: Bounds,
    selection: Selection,
    expected: &[&str],
    label: &str,
) -> String {
    assert_pairwise_excluding(tree, pane, selection, expected, &[], label)
}

/// 同 [`assert_pairwise`]，但先把 `excluded` 里的 id 从段内摘掉。
///
/// ⭐ `#187` 用它处理**按设计包住子节点**的 `list-item`（设备卡片 `device-{i}-card`）：
/// 它自己有语义 id、会与自己的子节点相交，但那是"容器包住内容"，不是缺陷。
/// ⛔ 被摘掉的 id 必须**逐条**在报告 §3 里给出理由，并且仍然参与"在场"断言
/// （见 [`assert_ids_pairwise`] 的容器组）。
fn assert_pairwise_excluding(
    tree: &ControlTree,
    pane: Bounds,
    selection: Selection,
    expected: &[&str],
    excluded: &[String],
    label: &str,
) -> String {
    let controls: Vec<(&ControlNode, Bounds)> = select_controls(tree, pane, selection)
        .into_iter()
        .filter(|(node, _)| !excluded.iter().any(|id| id == &node.id))
        .collect();

    // ---- 判据 ⑤：清单必须**一个不少**，且不许有清单外的节点混进来。
    let got: BTreeSet<&str> = controls.iter().map(|(node, _)| node.id.as_str()).collect();
    for id in expected {
        assert!(
            got.contains(id),
            "[{label}] 段内控件清单缺 `{id}` —— 两两判据会漏掉它（⛔ 不许 0 控件假绿）"
        );
    }
    assert_eq!(
        controls.len(),
        expected.len(),
        "[{label}] 段内控件数 {} ≠ 清单 {} ⇒ 有控件没登记（树里多出：{:?}）",
        controls.len(),
        expected.len(),
        got.iter()
            .filter(|id| !expected.contains(id))
            .collect::<Vec<_>>()
    );

    let mut pair_count = 0_usize;
    let mut hits = Vec::new();
    for (index, (a, a_bounds)) in controls.iter().enumerate() {
        for (b, b_bounds) in controls.iter().skip(index + 1) {
            pair_count += 1;
            if intersect(*a_bounds, *b_bounds) {
                hits.push(format!(
                    "`{}` (x={} y={} w={} h={}) ⟂ `{}` (x={} y={} w={} h={})",
                    a.id,
                    a_bounds.0,
                    a_bounds.1,
                    a_bounds.2,
                    a_bounds.3,
                    b.id,
                    b_bounds.0,
                    b_bounds.1,
                    b_bounds.2,
                    b_bounds.3
                ));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "[{label}] 段内**有重叠**（{} 对）：\n{}",
        hits.len(),
        hits.join("\n")
    );

    let dump = controls
        .iter()
        .map(|(node, b)| format!("{}: x={} y={} w={} h={}", node.id, b.0, b.1, b.2, b.3))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "[{label}] 控件 {} 个 / {pair_count} 对 / 相交 {} 对（容器按 role 排除）\n{dump}",
        controls.len(),
        hits.len()
    )
}

/// 一档的完整读数。返回 `(控件数, 对数)` 供外层汇总。
fn check_piano_roll(width: u32, height: u32) -> (usize, usize) {
    let live = live_window((width, height), PIANO_TAB);
    assert_eq!(live.ui().get_console_tab(), 0, "夹具必须是 Tab 0（卷帘）");
    let tree = runtime_tree(live.ui());
    let pane = bounds(&tree, "piano-roll");
    let reading = assert_pairwise(
        &tree,
        pane,
        Selection::Intersect,
        PIANO_CONTROLS,
        &format!("#186 钢琴卷帘 {width}x{height}"),
    );
    report_line(&reading);
    // 骨架里的静态读数也在场（两两判据之外再钉一次，防"整段消失 ⇒ 0 相交"）。
    assert_eq!(
        tree.find_by_id("piano-roll-status")
            .expect("状态行必须在场")
            .label,
        "卷帘吸附与音域状态"
    );
    (PIANO_CONTROLS.len(), 190)
}

fn check_mixer_console(width: u32, height: u32) -> (usize, usize) {
    let live = live_window((width, height), MIXER_TAB);
    assert_eq!(live.ui().get_console_tab(), 1, "夹具必须是 Tab 1（调音台）");
    let tree = runtime_tree(live.ui());
    let pane = bounds(&tree, "mixer-console");
    let expected = mixer_controls();
    let expected_refs: Vec<&str> = expected.iter().map(String::as_str).collect();
    let reading = assert_pairwise(
        &tree,
        pane,
        Selection::Intersect,
        &expected_refs,
        &format!("#186 调音台 {width}x{height}"),
    );
    report_line(&reading);

    // ---- 判据 ② 的修后几何：主控通道条仍在右沿；窄档三块面板竖排在它左侧。
    // 读数统一换到**调音台局部坐标**（运行时树给的是绝对坐标；`mixer-console` 的
    // 原点在 y = 顶栏 + 工作区，直接拿绝对值比登记值会把整段高度算进去）。
    let origin_x = pane.0;
    let origin_y = pane.1;
    let master = bounds(&tree, "mixer-master-strip");
    let master_x = master.0 - origin_x;
    assert_eq!(
        master_x,
        width as i32 - 112,
        "[#186 {width}x{height}] 主控通道条必须仍钉在 root.width - 112"
    );
    let export = tree
        .find_by_id("module-export-midi-button")
        .expect("导出 MIDI 按钮")
        .bounds
        .expect("包围盒");
    let open = tree
        .find_by_id("module-open-project-button")
        .expect("打开工程按钮")
        .bounds
        .expect("包围盒");
    let asset = bounds(&tree, "asset-pool-panel");
    let asset_x = asset.0 - origin_x;
    let asset_y = asset.1 - origin_y;
    let open_x = open.x - origin_x;
    if width < 1182 {
        // 窄档：工程面板与导出面板同列、素材池在工程面板正下方；右沿全部 < 主控左沿。
        assert_eq!(
            asset_x,
            8 + 104 * 6,
            "[#186 {width}] 窄档素材池必须与通道条右沿同列起点"
        );
        assert_eq!(open_x, 8 + 104 * 6 + 6, "[#186 {width}] 窄档工程按钮 x");
        assert_eq!(asset_y, 200, "[#186 {width}] 窄档素材池 y");
        assert!(
            asset_x + asset.2 as i32 <= master_x,
            "[#186 {width}] 素材池右沿 {} 必须 ≤ 主控左沿 {}",
            asset_x + asset.2 as i32,
            master_x
        );
        report_line(&format!(
            "[#186 调音台 {width}x{height}] 窄档竖排（局部坐标）：导出按钮 ({},{}) / 工程按钮 ({},{}) / \
             素材池 ({},{},{},{})；主控 x={master_x} ⇒ 面板右沿 {} < 主控左沿 {master_x}",
            export.x - origin_x,
            export.y - origin_y,
            open_x,
            open.y - origin_y,
            asset_x,
            asset_y,
            asset.2,
            asset.3,
            asset_x + asset.2 as i32
        ));
    } else {
        // 宽档：三块面板并排，几何一位未动（与 `#90`/`#93`/`#129`/`#133` 登记值一致）。
        assert_eq!(
            asset_x,
            8 + 104 * 6 + 216 + 6,
            "[#186 {width}] 宽档素材池 x"
        );
        assert_eq!(asset_y, 8 + 104 + 6, "[#186 {width}] 宽档素材池 y");
        assert_eq!(
            open_x,
            8 + 104 * 6 + 216 + 6 + 6,
            "[#186 {width}] 宽档工程按钮 x"
        );
        report_line(&format!(
            "[#186 调音台 {width}x{height}] 宽档并排（局部坐标）：素材池 ({},{},{},{}) / 工程按钮 ({},{})；\
             主控 x={master_x} ⇒ 面板右沿 {} < 主控左沿 {master_x}",
            asset_x,
            asset_y,
            asset.2,
            asset.3,
            open_x,
            open.y - origin_y,
            asset_x + asset.2 as i32
        ));
    }
    (expected.len(), 72 * 71 / 2)
}

fn check_asset_pool(width: u32, height: u32) -> (usize, usize) {
    let live = live_window((width, height), MIXER_TAB);
    let tree = runtime_tree(live.ui());
    let pane = bounds(&tree, "asset-pool-panel");
    let reading = assert_pairwise(
        &tree,
        pane,
        Selection::Intersect,
        ASSET_CONTROLS,
        &format!("#186 素材池 {width}x{height}"),
    );
    report_line(&reading);
    (ASSET_CONTROLS.len(), 3)
}

fn check_transport(width: u32, height: u32) -> (usize, usize) {
    let live = live_window((width, height), PIANO_TAB);
    let tree = runtime_tree(live.ui());
    let band: Bounds = (0, 0, width, TOP_BAR);
    let expected: Vec<&str> = if width >= 1064 {
        TRANSPORT_WIDE.to_vec()
    } else {
        TRANSPORT_WIDE
            .iter()
            .copied()
            .filter(|id| *id != "transport-ai-proposal-badge")
            .collect()
    };
    let reading = assert_pairwise(
        &tree,
        band,
        Selection::Inside,
        &expected,
        &format!("#186 走带 {width}x{height}"),
    );
    report_line(&reading);

    // ---- 判据 ②＋⑤：1024 档徽章**按设计隐藏**（不是扫不到），替代入口必须在树里。
    let save = bounds(&tree, "transport-save-button");
    let session = bounds(&tree, "transport-view-session-button");
    assert!(
        save.0 + save.2 as i32 <= session.0,
        "[#186 {width}x{height}] 保存按钮右沿必须 ≤ Session 左沿"
    );
    if width >= 1064 {
        let badge = bounds(&tree, "transport-ai-proposal-badge");
        assert!(
            save.0 + save.2 as i32 <= badge.0,
            "[#186 {width}x{height}] 保存按钮右沿必须 ≤ 徽章左沿"
        );
        assert!(
            badge.0 + badge.2 as i32 <= session.0,
            "[#186 {width}x{height}] 徽章右沿必须 ≤ Session 左沿（宽档不许交）"
        );
        report_line(&format!(
            "[#186 走带 {width}x{height}] save {save:?} ≤ badge {badge:?} ≤ session {session:?} ⇒ 不相交"
        ));
    } else {
        assert!(
            tree.find_by_id("transport-ai-proposal-badge").is_none(),
            "[#186 {width}x{height}] 窄档徽章必须 visible:false ⇒ 不在树里"
        );
        // ⭐ `#191`：折叠档（`width < 1180`，⭐ `#195` 之后 1024 / 800）的右栏是抽屉，
        // 默认不在树里 ⇒ 同动作入口经 `transport-ai-drawer-button` 打开抽屉后必须在场
        //（⛔ 不是把功能藏没了）。
        if live.ui().get_compact() {
            assert!(
                tree.find_by_id("transport-ai-drawer-button").is_some(),
                "[#186 {width}x{height}] 折叠档必须留下抽屉入口 `transport-ai-drawer-button`"
            );
            live.ui().set_ai_drawer_open(true);
            let drawer_tree = runtime_tree(live.ui());
            assert!(
                drawer_tree
                    .find_by_id("ai-rail-musical-pr-button")
                    .is_some(),
                "[#186 {width}x{height}] 打开抽屉后 `ai-rail-musical-pr-button` 必须在场（⛔ 不是把功能藏没了）"
            );
            report_line(&format!(
                "[#186 走带 {width}x{height} compact] save {save:?} ≤ session {session:?} ⇒ 不相交；\
                 徽章不在树里（visible:false）；打开抽屉后 ai-rail-musical-pr-button 在树里 = true"
            ));
        } else {
            assert!(
                tree.find_by_id("ai-rail-musical-pr-button").is_some(),
                "[#186 {width}x{height}] 窄档徽章隐藏后，右栏的同动作入口必须在树里（⛔ 不是把功能藏没了）"
            );
            report_line(&format!(
                "[#186 走带 {width}x{height}] save {save:?} ≤ session {session:?} ⇒ 不相交；\
                 徽章不在树里（visible:false）；替代入口 ai-rail-musical-pr-button 在树里 = true"
            ));
        }
    }
    (
        expected.len(),
        expected.len() * expected.len().saturating_sub(1) / 2,
    )
}

// ===========================================================================
// 判据 ①（钢琴卷帘）：三档，段内非容器控件两两不相交
// ===========================================================================
#[test]
fn the_piano_roll_has_no_two_controls_in_the_same_place() {
    for &(width, height) in TIERS {
        if let Err(payload) = std::thread::spawn(move || check_piano_roll(width, height)).join() {
            std::panic::resume_unwind(payload);
        }
    }
}

// ===========================================================================
// 判据 ②（调音台）：三档，段内非容器控件两两不相交 ＋ 窄档竖排几何
// ===========================================================================
#[test]
fn the_mixer_console_has_no_two_controls_in_the_same_place() {
    for &(width, height) in TIERS {
        if let Err(payload) = std::thread::spawn(move || check_mixer_console(width, height)).join()
        {
            std::panic::resume_unwind(payload);
        }
    }
}

// ===========================================================================
// 判据 ③（素材池）：三档，段内非容器控件两两不相交
// ===========================================================================
#[test]
fn the_asset_pool_has_no_two_controls_in_the_same_place() {
    for &(width, height) in TIERS {
        if let Err(payload) = std::thread::spawn(move || check_asset_pool(width, height)).join() {
            std::panic::resume_unwind(payload);
        }
    }
}

// ===========================================================================
// 判据 ④（走带）：三档，顶栏带内非容器控件两两不相交
// ===========================================================================
#[test]
fn the_transport_has_no_two_controls_in_the_same_place() {
    for &(width, height) in TIERS {
        if let Err(payload) = std::thread::spawn(move || check_transport(width, height)).join() {
            std::panic::resume_unwind(payload);
        }
    }
}

// ===========================================================================
// ⭐ 任务单 `#187`：剩余六段 —— 夹具、清单、通用分组断言
// ===========================================================================

/// 夹具：一条展示轨（MIDI，无设备）＋ 主总线 ⇒ **空设备链**（`device-rack-empty` 在场）。
fn project_without_devices() -> YebanProjectV1 {
    project_with_tracks(true)
}

/// 夹具：只有主总线（没有展示轨）⇒ 装乐器 / 装设备 / 建片段 / 删轨四条"逐字原因"都在场。
fn project_master_only() -> YebanProjectV1 {
    project_with_tracks(false)
}

fn project_with_tracks(with_display_track: bool) -> YebanProjectV1 {
    let master = EntityId::new();
    let track = EntityId::new();
    let mut tracks: BTreeMap<EntityId, TrackV3> = BTreeMap::new();
    if with_display_track {
        tracks.insert(
            track,
            TrackV3 {
                id: track,
                name: "展示轨".to_owned(),
                kind: TrackKind::Midi,
                ..TrackV3::default()
            },
        );
    }
    tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    YebanProjectV1 {
        master_bus_track_id: master,
        tracks,
        ..YebanProjectV1::default()
    }
}

/// ⭐ `#190` 的 7 轨夹具：演示工程（6 条非主总线轨 ＋ 主总线）再插一条空 MIDI 轨。
/// 1440 档画布 920 ⇒ 容量 6 ⇒ **第 7 轨**必须被截断报数（1920 档容量 10 ⇒ 全显示）。
fn project_with_seven_tracks() -> YebanProjectV1 {
    let mut project = yeban_model::samples::demo_project();
    let track = EntityId::new();
    project.tracks.insert(
        track,
        TrackV3 {
            id: track,
            name: "第 7 轨".to_owned(),
            kind: TrackKind::Midi,
            ..TrackV3::default()
        },
    );
    project
}

/// ① 设备链卡片区：演示工程（1 台设备，含未识别参数行）在宽档的 7 个叶子控件。
/// `device-0-card` **不在这里**：它是 `list-item` 且**按设计包住** recognised /
/// unrecognised / bypass / remove 四个语义子节点 ⇒ 属于容器组（见 `assert_ids_pairwise`）。
const CARD_AREA_DEMO_WIDE: &[&str] = &[
    "device-insert-eq-button",
    "device-insert-reverb-button",
    "clip-create-button",
    "device-0-unrecognised",
    "device-0-bypass-switch",
    "device-0-remove-button",
    "device-plugin-host-placeholder-card",
];

/// 同上的 1024 档：占位卡 `266..288` 越出面板底 `281` ⇒ **按设计撤下**（清单 7 → 6）。
const CARD_AREA_DEMO_NARROW: &[&str] = &[
    "device-insert-eq-button",
    "device-insert-reverb-button",
    "clip-create-button",
    "device-0-unrecognised",
    "device-0-bypass-switch",
    "device-0-remove-button",
];

/// ① 空设备链（三档同清单）：空态文案 ＋ 占位卡（让到 `240..262` 后不再压文案）。
const CARD_AREA_EMPTY: &[&str] = &[
    "device-insert-eq-button",
    "device-insert-reverb-button",
    "clip-create-button",
    "device-rack-empty",
    "device-plugin-host-placeholder-card",
];

/// ① 只有主总线：插设备 / 建片段两条**逐字原因**都在场（`#163` 的原因行）。
const CARD_AREA_MASTER_ONLY: &[&str] = &[
    "device-insert-eq-button",
    "device-insert-reverb-button",
    "clip-create-button",
    "device-insert-reason",
    "clip-create-reason",
    "device-rack-empty",
    "device-plugin-host-placeholder-card",
];

/// ② 装乐器行：内置清单为空（`installable_builtin_instruments` 0 条）⇒ 三个语义节点。
/// ⭐ 任务单 `#196`：清单为空时"下载采样库"按钮占右侧那一格，左下是"采样库未安装"提示
/// （`instrument-install-reason` 让位给它 —— 两句原因互斥可见，⛔ 不叠字）。
const INSTRUMENT_ROW: &[&str] = &[
    "instrument-label",
    "sample-library-download-button",
    "sample-library-status",
];

/// ③ 轨路由四键区：三个建轨键 ＋ 删除键 ＋ 两格读数（演示工程有展示轨 ⇒ 原因行不在场）。
const ROUTING_DEMO: &[&str] = &[
    "track-routing-count",
    "master-bus-label",
    "track-create-midi-button",
    "track-create-audio-button",
    "track-create-aux-button",
    "track-delete-button",
];

/// ③ 只有主总线：多一条"不可删除的逐字原因"，仍与四键不相交。
const ROUTING_MASTER_ONLY: &[&str] = &[
    "track-routing-count",
    "master-bus-label",
    "track-create-midi-button",
    "track-create-audio-button",
    "track-create-aux-button",
    "track-delete-button",
    "track-delete-reason",
];

/// ⑤ 状态栏：默认（没按过保存）⇒ 快捷键提示那一格在场。
const STATUS_BAR_DEFAULT: &[&str] = &[
    "status-bar-selection",
    "status-bar-chord",
    "status-bar-shortcut-tip",
    "status-bar-device",
];

/// ⑤ 状态栏：保存回执态 ⇒ 提示格换成回执格（两者 `visible` 互斥）。
const STATUS_BAR_SAVED: &[&str] = &[
    "status-bar-selection",
    "status-bar-chord",
    "status-bar-save-status",
    "status-bar-device",
];

/// ⑥ AI 右栏：宽档 12 个叶子控件。
const AI_RAIL_WIDE: &[&str] = &[
    "ai-rail-intent-button",
    "ai-rail-diagnose-button",
    "diagnostics-export-action",
    "ai-rail-musical-pr-button",
    "module-actions-title",
    "module-export-wav-button",
    "module-import-midi-button",
    "module-load-sfz-button",
    "module-record-wav-button",
    "module-midi-in-button",
    "module-midi-in-status",
    "module-last-status",
];

/// ⑥ AI 右栏 1024 档 11 个：`module-last-status` 住 `424..440`（右栏高 415）⇒
/// 完全落在裁剪区外 ⇒ 上游**不进运行时树**（与走带徽章同一种"按设计不在场"）。
const AI_RAIL_NARROW: &[&str] = &[
    "ai-rail-intent-button",
    "ai-rail-diagnose-button",
    "diagnostics-export-action",
    "ai-rail-musical-pr-button",
    "module-actions-title",
    "module-export-wav-button",
    "module-import-midi-button",
    "module-load-sfz-button",
    "module-record-wav-button",
    "module-midi-in-button",
    "module-midi-in-status",
];

/// ④ 轨道头（`list-item`，按设计包住色标 / 静音 / 独奏 ⇒ 容器组，单独两两断言）。
const TRACK_HEADERS: &[&str] = &[
    "track-0-header",
    "track-1-header",
    "track-2-header",
    "track-3-header",
    "track-4-header",
    "track-5-header",
];

/// ④ 剪辑头（`list-item` **叶子**：子节点没有语义 id ⇒ ⛔ 不排）。
const CLIP_HEADERS: &[&str] = &[
    "clip-01J8Z5Q0R7K3M9X2V4B6N8P1F9-header",
    "clip-01J8Z5Q0R7K3M9X2V4B6N8P1G6-header",
    "clip-01J8Z5Q0R7K3M9X2V4B6N8P1H3-header",
];

/// ④ 自动化泳道（`image` 角色，一条一泳道；彼此是相邻带，⛔ 不许交）。
const AUTOMATION_LANES: &[&str] = &[
    "track-0-automation-device-0-0-lane",
    "track-0-automation-volume-lane",
    "track-1-automation-pan-lane",
];

/// ④ 章节卡：逐档清单（演示工程的 4 张各 256px，从 `x = 408` 排到画布右沿）。
fn section_cards_for(width: u32) -> &'static [&'static str] {
    if width >= 1600 {
        &[
            "section-0-card",
            "section-1-card",
            "section-2-card",
            "section-3-card",
        ]
    } else if width >= 1200 {
        &["section-0-card", "section-1-card", "section-2-card"]
    } else {
        &["section-0-card", "section-1-card"]
    }
}

/// 分组断言：清单里的 id **一个不少**，且组内**两两**包围盒不相交。
///
/// ⭐ 它与 [`assert_pairwise`] 的分工：`assert_pairwise` 判"段内非容器叶子"，
/// 本函数判**容器与容器**（轨道头）、以及**本来就不该混进叶子的同族控件**
/// （剪辑头 / 自动化泳道 / 章节卡）。两边的"控件数 ＝ 清单"都是**硬断言**。
fn assert_ids_pairwise(tree: &ControlTree, ids: &[&str], label: &str) -> String {
    let mut controls: Vec<(&str, Bounds)> = Vec::with_capacity(ids.len());
    for id in ids {
        let node = tree.find_by_id(id).unwrap_or_else(|| {
            panic!("[#187 {label}] 组内缺 `{id}` —— 清单与运行时树不一致（⛔ 不许静默跳过）")
        });
        assert!(
            node.bounds.is_some(),
            "[#187 {label}] 组内 `{id}` 没有包围盒 ⇒ 判据会漏掉它"
        );
        controls.push((id, bounds_of(node)));
    }

    let mut pair_count = 0_usize;
    let mut hits = Vec::new();
    for (index, (a_id, a)) in controls.iter().enumerate() {
        for (b_id, b) in controls.iter().skip(index + 1) {
            pair_count += 1;
            if intersect(*a, *b) {
                hits.push(format!("`{a_id}` {a:?} ⟂ `{b_id}` {b:?}"));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "[#187 {label}] 组内有重叠（{} 对）：\n{}",
        hits.len(),
        hits.join("\n")
    );

    let dump = controls
        .iter()
        .map(|(id, b)| format!("{id}: x={} y={} w={} h={}", b.0, b.1, b.2, b.3))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "[#187 {label}] 控件 {} 个 / {pair_count} 对 / 相交 {} 对\n{dump}",
        controls.len(),
        hits.len()
    )
}

/// 包容断言：`child` 的包围盒必须整个落在 `parent` 之内（容器包住子节点的**正向**证明）。
fn assert_inside(tree: &ControlTree, child: &str, parent: &str, label: &str) {
    let c = bounds(tree, child);
    let p = bounds(tree, parent);
    assert!(
        c.0 >= p.0
            && c.1 >= p.1
            && c.0 + c.2 as i32 <= p.0 + p.2 as i32
            && c.1 + c.3 as i32 <= p.1 + p.3 as i32,
        "[#187 {label}] `{child}` {c:?} 必须落在 `{parent}` {p:?} 之内"
    );
}

// ===========================================================================
// ① 设备链卡片区
// ===========================================================================
fn check_device_card_area(
    width: u32,
    height: u32,
    project: YebanProjectV1,
    tag: &str,
    expected: &[&str],
    device_count: usize,
) {
    let live = live_window_for(project, (width, height), 2);
    assert_eq!(live.ui().get_console_tab(), 2, "夹具必须是 Tab 2（设备架）");
    let tree = runtime_tree(live.ui());

    let rack = bounds(&tree, "device-rack");
    let actions = bounds(&tree, "device-insert-actions");
    let rack_bottom = rack.1 + rack.3 as i32;
    // 段盒子：第一段左列 `0..428`，从插入行顶到面板底（含卡片链 / 空态 / 占位卡）。
    let pane: Bounds = (
        rack.0,
        actions.1,
        428,
        (rack_bottom - actions.1).max(0) as u32,
    );
    assert!(
        actions.1 >= rack.1 && rack_bottom > actions.1,
        "[#187 卡片区 {tag} {width}x{height}] 段盒子退化：rack {rack:?} / actions {actions:?}"
    );

    let excluded: Vec<String> = (0..device_count)
        .map(|index| format!("device-{index}-card"))
        .collect();
    let reading = assert_pairwise_excluding(
        &tree,
        pane,
        Selection::Intersect,
        expected,
        &excluded,
        &format!("#187 设备链卡片区 {tag} {width}x{height}"),
    );
    report_line(&reading);

    // ---- 判据 ⑤：占位卡要么在场且完整落在面板里，要么按设计撤下（⛔ 不留半张）。
    let placeholder = tree.find_by_id("device-plugin-host-placeholder-card");
    if expected.contains(&"device-plugin-host-placeholder-card") {
        let card = placeholder
            .and_then(|node| node.bounds)
            .expect("清单要求占位卡在场 ⇒ 它必须有包围盒");
        assert!(
            card.y + card.height as i32 <= rack_bottom,
            "[#187 卡片区 {tag} {width}x{height}] 占位卡底沿 {} 越过面板底 {rack_bottom} \
             （半张卡被 clip 吃掉）",
            card.y + card.height as i32
        );
    } else {
        assert!(
            placeholder.is_none(),
            "[#187 卡片区 {tag} {width}x{height}] 本档的清单说占位卡放不下、应整张撤下，\
             但它仍在树里（几何放不下时不许半画）"
        );
    }

    // ---- 容器组：设备卡片（`list-item` 包装）与占位卡（叶子）同族，逐个两两断言。
    let mut container_ids: Vec<String> = (0..device_count)
        .map(|index| format!("device-{index}-card"))
        .collect();
    if expected.contains(&"device-plugin-host-placeholder-card") {
        container_ids.push("device-plugin-host-placeholder-card".to_owned());
    }
    let container_refs: Vec<&str> = container_ids.iter().map(String::as_str).collect();
    report_line(&assert_ids_pairwise(
        &tree,
        &container_refs,
        &format!("设备卡片组 {tag} {width}x{height}"),
    ));

    // ---- 每张卡片的语义子节点必须落在卡内（容器被排除 ≠ 不检查）。
    for index in 0..device_count {
        let card_id = format!("device-{index}-card");
        for suffix in [
            "recognised",
            "unrecognised",
            "bypass-switch",
            "remove-button",
        ] {
            let child = format!("device-{index}-{suffix}");
            if tree.find_by_id(&child).is_some() {
                assert_inside(
                    &tree,
                    &child,
                    &card_id,
                    &format!("设备卡片区 {tag} {width}x{height}"),
                );
            }
        }
    }

    // ---- 防假绿：空链时空态文案逐字在场；有设备时它必须不在。
    if device_count == 0 {
        let empty = tree
            .find_by_id("device-rack-empty")
            .expect("空链必须有 `device-rack-empty`（⛔ 不许一片空白）");
        assert_eq!(empty.label, "无设备（点上方「装 EQ」或「装混响」）");
    } else {
        assert!(
            tree.find_by_id("device-rack-empty").is_none(),
            "[#187 卡片区 {tag} {width}x{height}] 有设备时空态文案必须在树外"
        );
    }
}

// ===========================================================================
// ② 装乐器行（`#163`）
// ===========================================================================
fn check_instrument_row(width: u32, height: u32) {
    let live = live_window((width, height), 2);
    let tree = runtime_tree(live.ui());

    let label = bounds(&tree, "instrument-label");
    // 行盒子：当前乐器（20）＋ 2px ＋ 不可用原因（14）＝ 36px。
    let pane: Bounds = (label.0, label.1, 428, 36);
    let reading = assert_pairwise(
        &tree,
        pane,
        Selection::Intersect,
        INSTRUMENT_ROW,
        &format!("#187 装乐器行 {width}x{height}"),
    );
    report_line(&reading);

    // ---- 防假绿：内置清单为空 ⇒ **0 个按钮**（这是读数，不是"扫不到"）。
    for index in 0..3 {
        let id = format!("instrument-install-{index}-button");
        assert!(
            tree.find_by_id(&id).is_none(),
            "[#187 装乐器行 {width}x{height}] 内置清单为空时不该有 `{id}`；\
             它出现了 ⇒ 清单与 `INSTRUMENT_ROW` 都要重核"
        );
    }
    let label_node = tree.find_by_id("instrument-label").expect("乐器读数行");
    assert_eq!(label_node.label, "未装乐器", "没装乐器时读数必须逐字如此");
    // ⭐ `#196`：0 个装乐器按钮时，界面**必须**给出"采样库未安装"的逐字提示与下载入口
    // （⛔ 不静默禁用；原 `instrument-install-reason` 已被这条更可操作的提示取代）。
    let status = tree
        .find_by_id("sample-library-status")
        .expect("0 个按钮时**必须**看得见逐字提示（⛔ 不静默禁用）");
    assert!(
        status.label.contains("采样库未安装"),
        "[#187 装乐器行 {width}x{height}] 提示必须逐字说出缺什么，实测: {}",
        status.label
    );
    assert!(
        tree.find_by_id("sample-library-download-button").is_some(),
        "[#187 装乐器行 {width}x{height}] 未安装时下载按钮必须在场"
    );
    // 行盒子 36px：读数行 20 ＋ 2 ＋ 原因行 14（与 `pane` 的取法同一个算术）。
    assert_eq!(pane.3, 36, "[#187 装乐器行 {width}x{height}] 行盒子高");
}

// ===========================================================================
// ③ 轨路由四键区（`#160`）
// ===========================================================================
fn check_track_routing(
    width: u32,
    height: u32,
    project: YebanProjectV1,
    tag: &str,
    expected: &[&str],
) {
    let live = live_window_for(project, (width, height), 2);
    let tree = runtime_tree(live.ui());

    let pane = bounds(&tree, "track-routing-panel");
    let reading = assert_pairwise(
        &tree,
        pane,
        Selection::Intersect,
        expected,
        &format!("#187 轨路由四键区 {tag} {width}x{height}"),
    );
    report_line(&reading);

    let midi = bounds(&tree, "track-create-midi-button");
    let audio = bounds(&tree, "track-create-audio-button");
    let aux = bounds(&tree, "track-create-aux-button");
    let delete = bounds(&tree, "track-delete-button");
    let pane_right = pane.0 + pane.2 as i32;

    // ---- 四键几何：左起 8px、等距 `宽 + 4`、末键右沿钉在 `面板右沿 − 8`。
    assert_eq!(midi.0 - pane.0, 8, "[#187 轨路由 {width}] 首键 x");
    assert_eq!(midi.1, audio.1, "[#187 轨路由 {width}] 四键必须同一行");
    assert_eq!(aux.1, delete.1, "[#187 轨路由 {width}] 四键必须同一行");
    assert_eq!(
        audio.0 - midi.0,
        midi.2 as i32 + 4,
        "[#187 轨路由 {width}] 键距必须 = 键宽 + 4"
    );
    assert_eq!(
        aux.0 - audio.0,
        audio.2 as i32 + 4,
        "[#187 轨路由 {width}] 键距必须 = 键宽 + 4"
    );
    assert_eq!(
        delete.0 - aux.0,
        aux.2 as i32 + 4,
        "[#187 轨路由 {width}] 键距必须 = 键宽 + 4"
    );
    assert_eq!(
        delete.0 + delete.2 as i32,
        pane_right - 8,
        "[#187 轨路由 {width}] 末键右沿必须钉在面板右沿 − 8"
    );
    assert_eq!(
        midi.1 - pane.1,
        22,
        "[#187 轨路由 {width}] 四键上沿必须钉在面板内 y=22"
    );
    assert_eq!(midi.3, 20, "[#187 轨路由 {width}] 四键高必须 20");
    assert_eq!(delete.3, 20, "[#187 轨路由 {width}] 删除键高必须 20");

    if expected.contains(&"track-delete-reason") {
        let reason = bounds(&tree, "track-delete-reason");
        assert!(
            reason.1 + reason.3 as i32 <= pane.1 + pane.3 as i32,
            "[#187 轨路由 {width}] 原因行底沿必须留在面板内"
        );
    }
}

// ===========================================================================
// ④ 编曲画布（`workspace-arrangement-canvas` / `clip-*` / `track-*` / 自动化泳道）
// ===========================================================================
fn check_arrangement(width: u32, height: u32) {
    let live = live_window((width, height), 0);
    let tree = runtime_tree(live.ui());

    let canvas = bounds(&tree, "workspace-arrangement-canvas");
    // ⭐ `#191`：`compact` 跟真实窗口宽走之后，右栏在折叠档（`width < 1180`，
    // ⭐ `#195` 之后即 1024 / 800）是**抽屉**
    //（默认 `ai-drawer-open = false` ⇒ 不在树里）⇒ 画布右沿不再等于右栏左沿，
    // 而是撑到窗口右沿。展开档仍按原不变量钉住（1440 也是展开档）。
    if live.ui().get_compact() {
        assert_eq!(
            canvas.0 + canvas.2 as i32,
            width as i32,
            "[#187 编曲画布 {width}x{height} compact] 折叠档右栏是抽屉，画布必须撑满到窗口右沿"
        );
    } else {
        let right_rail = bounds(&tree, "ai-rail");
        assert_eq!(
            canvas.0 + canvas.2 as i32,
            right_rail.0,
            "[#187 编曲画布 {width}x{height}] 画布右沿必须 = 右栏左沿（画布 clip 的边界）"
        );
    }

    // ---- ④-1 轨道头（容器组）：两两不相交。
    let label = format!("编曲画布 轨道头 {width}x{height}");
    report_line(&assert_ids_pairwise(&tree, TRACK_HEADERS, &label));

    // ---- ④-2 头内子节点（叶子组）：两两不相交 ＋ 各自落在自己的头里。
    let mut child_ids: Vec<String> = Vec::new();
    for track in 0..6 {
        for suffix in ["color-swatch", "mute-button", "solo-button"] {
            child_ids.push(format!("track-{track}-{suffix}"));
        }
    }
    let child_refs: Vec<&str> = child_ids.iter().map(String::as_str).collect();
    let label = format!("编曲画布 轨道头子节点 {width}x{height}");
    report_line(&assert_ids_pairwise(&tree, &child_refs, &label));
    for track in 0..6 {
        let header = format!("track-{track}-header");
        for suffix in ["color-swatch", "mute-button", "solo-button"] {
            let child = format!("track-{track}-{suffix}");
            assert_inside(
                &tree,
                &child,
                &header,
                &format!("编曲画布 {width}x{height}"),
            );
        }
    }

    // ---- ④-3 剪辑头（`list-item` 叶子，⛔ 不排）/ ④-4 自动化泳道 / ④-5 章节卡。
    let label = format!("编曲画布 剪辑头 {width}x{height}");
    report_line(&assert_ids_pairwise(&tree, CLIP_HEADERS, &label));
    let label = format!("编曲画布 自动化泳道 {width}x{height}");
    report_line(&assert_ids_pairwise(&tree, AUTOMATION_LANES, &label));
    let sections = section_cards_for(width);
    let label = format!("编曲画布 章节卡 {width}x{height}");
    report_line(&assert_ids_pairwise(&tree, sections, &label));

    // ---- 排除项（按设计覆盖，逐条登记在报告 §3）：
    //   `arrangement-ruler` 是标尺**带**，按设计包住 `arrangement-loop-brace`；
    //   `arrangement-playhead` 是 1px 竖向走带光标，按设计压在剪辑 / 泳道上。
    assert!(
        tree.find_by_id("arrangement-ruler").is_some(),
        "[#187 编曲画布 {width}] 标尺带必须在场"
    );
    assert!(
        tree.find_by_id("arrangement-loop-brace").is_some(),
        "[#187 编曲画布 {width}] 循环选区把手必须在场（它是标尺带**内**的滑块，按设计压在带上）"
    );
    // ⭐ `#191`：折叠档（1024）右栏变抽屉后画布撑到窗口右沿（944px 宽），播放头
    // x = 768 落在画布内 ⇒ 它在树里；只有**展开档**的 1024（画布 504px）才整条被 clip。
    if live.ui().get_compact() || width >= 1440 {
        let playhead = bounds(&tree, "arrangement-playhead");
        assert_eq!(
            playhead.2, 1,
            "[#187 编曲画布 {width}] 播放头必须是 1px 竖线"
        );
        assert_eq!(
            playhead.1,
            canvas.1 + 20,
            "[#187 编曲画布 {width}] 播放头上沿"
        );
    } else {
        // 1024 档：播放头 x = 画布左沿 + 528 = 768 > 画布右沿 744 ⇒ 完全被 clip ⇒ 不进树。
        assert!(
            tree.find_by_id("arrangement-playhead").is_none(),
            "[#187 编曲画布 {width}] 播放头整条落在画布外 ⇒ 必须在树外（⛔ 不许越界画出）"
        );
    }
}

// ===========================================================================
// ⑤ 状态栏（`status-bar*`）
// ===========================================================================
fn check_status_bar(width: u32, height: u32, save_status: bool) {
    let mut live = live_window((width, height), 0);
    if save_status {
        live.ui()
            .set_save_status("已保存工程 · 写盘 1 次 · 校验 0 项失败".into());
    }
    let tree = if save_status {
        live.refresh_tree(None).expect("运行时控件树").clone()
    } else {
        runtime_tree(live.ui())
    };

    let pane = bounds(&tree, "status-bar");
    assert_eq!(pane.3, 24, "[#187 状态栏 {width}] 带高必须是 24px");
    assert_eq!(
        pane.1 + pane.3 as i32,
        height as i32,
        "状态栏必须贴着窗口底沿"
    );

    let expected = if save_status {
        STATUS_BAR_SAVED
    } else {
        STATUS_BAR_DEFAULT
    };
    // ⚠ `status-bar` 本身是这一段的**段根**（`accessible-role: content-info`，不在
    // `is_container_role` 的容器表里）⇒ 按"段盒子本身不参与两两"的口径逐条摘掉；
    // 它是 `content-info`，⛔ 不是叶子控件。
    let reading = assert_pairwise_excluding(
        &tree,
        pane,
        Selection::Inside,
        expected,
        &["status-bar".to_owned()],
        &format!("#187 状态栏 {width}x{height}"),
    );
    report_line(&reading);

    // ---- 判据 ②：中间那一格的右沿必须 ≤ 音频配置格的左沿（半开区间：贴边不算交）。
    let mid_id = if save_status {
        "status-bar-save-status"
    } else {
        "status-bar-shortcut-tip"
    };
    let mid = bounds(&tree, mid_id);
    let device = bounds(&tree, "status-bar-device");
    assert_eq!(mid.0, pane.0 + 344, "[#187 状态栏 {width}] 中间格左沿");
    assert_eq!(
        device.0,
        pane.0 + width as i32 - 268,
        "[#187 状态栏 {width}] 音频配置格左沿"
    );
    assert!(
        mid.0 + mid.2 as i32 <= device.0,
        "[#187 状态栏 {width}] `{mid_id}` 右沿 {} 必须 ≤ `status-bar-device` 左沿 {}（实测相交 {}px）",
        mid.0 + mid.2 as i32,
        device.0,
        mid.0 + mid.2 as i32 - device.0
    );
    report_line(&format!(
        "[#187 状态栏 {width}x{height} 回执态={save_status}] `{mid_id}` {mid:?} ≤ `status-bar-device` {device:?} ⇒ 不相交"
    ));

    // ---- 两格 `visible` 互斥：同一时刻只有一格在树里（另一格不画）。
    let other = if save_status {
        "status-bar-shortcut-tip"
    } else {
        "status-bar-save-status"
    };
    assert!(
        tree.find_by_id(other).is_none(),
        "[#187 状态栏 {width}] 另一格 `{other}` 必须按 design 不在树里（两格共用同一块矩形）"
    );
}

// ===========================================================================
// ⑥ AI 右栏（`ai-rail*` ＋ 模块入口）
// ===========================================================================
fn check_ai_rail(width: u32, height: u32) {
    let live = live_window((width, height), 0);
    // ⭐ `#191`：折叠档的右栏是抽屉（`visible: compact && ai-drawer-open`）。要检查
    // 同一个 `AiRail` 实例，就先把它打开 —— 这同时把 `#189` 报告里"compact 下的
    // `AiRail` 抽屉实例未扫"这一格补上了。
    let drawer = live.ui().get_compact();
    if drawer {
        live.ui().set_ai_drawer_open(true);
    }
    let tree = runtime_tree(live.ui());

    let pane = bounds(&tree, "ai-rail");
    assert_eq!(
        pane.2, 280,
        "[#187 AI 右栏 {width} drawer={drawer}] 右栏/抽屉宽必须 280"
    );
    let expected = if width >= 1440 {
        AI_RAIL_WIDE
    } else {
        AI_RAIL_NARROW
    };
    // ⭐ `#191`：折叠档的抽屉是**覆盖层**（整窗高、压在画布/走带上）⇒ 几何取法
    // `LeftEdgeInside` 会把被盖住的下层控件也扫进来（实测 1440 档多出 5 个
    // `transport-*` / `status-bar-device` / `console-maximize-button`）。抽屉实例改用
    // **按清单**两两判据：它证明的仍是"右栏这一组控件互不相交"，并且清单缺项会红。
    let label = format!("#187 AI 右栏 {width}x{height} drawer={drawer}");
    let reading = if drawer {
        assert_ids_pairwise(&tree, expected, &label)
    } else {
        assert_pairwise(&tree, pane, Selection::LeftEdgeInside, expected, &label)
    };
    report_line(&reading);

    // ---- 判据 ②：诊断卡下沿 ≤ 导出键上沿；导出键与审查键同一行、左右分开。
    let diagnose = bounds(&tree, "ai-rail-diagnose-button");
    let export = bounds(&tree, "diagnostics-export-action");
    let musical_pr = bounds(&tree, "ai-rail-musical-pr-button");
    assert!(
        diagnose.1 + diagnose.3 as i32 <= export.1,
        "[#187 AI 右栏 {width}] 声学诊断卡底沿 {} 必须 ≤ 导出诊断包上沿 {}（修前相交 20px）",
        diagnose.1 + diagnose.3 as i32,
        export.1
    );
    assert_eq!(
        export.1, musical_pr.1,
        "[#187 AI 右栏 {width}] 导出键与审查键必须同一行"
    );
    assert_eq!(
        export.3, musical_pr.3,
        "[#187 AI 右栏 {width}] 导出键与审查键同高"
    );
    assert!(
        export.0 + export.2 as i32 <= musical_pr.0,
        "[#187 AI 右栏 {width}] 导出键右沿 {} 必须 ≤ 审查键左沿 {}",
        export.0 + export.2 as i32,
        musical_pr.0
    );
    assert_eq!(export.0 - pane.0, 8, "[#187 AI 右栏 {width}] 导出键左沿");
    assert_eq!(
        musical_pr.0 + musical_pr.2 as i32,
        pane.0 + pane.2 as i32 - 8,
        "[#187 AI 右栏 {width}] 审查键右沿必须钉在右栏右沿 − 8"
    );
    report_line(&format!(
        "[#187 AI 右栏 {width}x{height}] diagnose {diagnose:?} / export {export:?} / musical-pr {musical_pr:?} ⇒ 两键上下分开、左右分开"
    ));

    // ---- 判据 ⑤：模块入口标题**一位没动**（仍是右栏内 y=212），证明修法不靠下移。
    let title = bounds(&tree, "module-actions-title");
    assert_eq!(
        title.1 - pane.1,
        212,
        "[#187 AI 右栏 {width}] `module-actions-title` 必须仍在 y=212（⛔ 不靠整体下移换绿）"
    );

    // ---- 取法的守卫：1024 档 `arrangement-loop-brace` 的包围盒伸进右栏，但左沿在
    //     画布内（被画布 clip）⇒ `LeftEdgeInside` 必须把它挡在段外。
    if width == 1024 {
        let brace = bounds(&tree, "arrangement-loop-brace");
        assert!(
            brace.0 < pane.0,
            "[#187 AI 右栏 {width}] 这条取法的前提变了：`arrangement-loop-brace` 左沿 {} \
             不再落在右栏外 —— 请重核 `Selection::LeftEdgeInside` 的裁决",
            brace.0
        );
        assert!(
            brace.0 + brace.2 as i32 > pane.0,
            "[#187 AI 右栏 {width}] 前提也要求它**仍然**越界伸进右栏（否则这条守卫是空的）"
        );
    }

    // ---- 1024 档（**常驻栏**）：回执行被右栏裁剪区排掉（完全在树外），如实登记而不是静默。
    // ⭐ `#191`：折叠档的抽屉是 `height: root.height`（整窗高 800），回执行在树里 ——
    // 这条 415px 的裁剪只属于常驻栏实例，⛔ 不能套到抽屉上。
    if width < 1440 && !drawer {
        assert!(
            tree.find_by_id("module-last-status").is_none(),
            "[#187 AI 右栏 {width}] 1024 档常驻栏 `module-last-status`（424..440 > 栏高 415）必须在树外"
        );
    }
}

// ===========================================================================
// 判据 ⑤（设备链卡片区）：三档 ＋ 三种夹具
// ===========================================================================
#[test]
fn the_device_card_chain_has_no_two_controls_in_the_same_place() {
    for &(width, height) in TIERS {
        // (a) 演示工程：1 台设备；1024 档占位卡放不下 ⇒ 整张撤下（清单少一个）。
        let expected_wide: &[&str] = CARD_AREA_DEMO_WIDE;
        let expected_narrow: &[&str] = CARD_AREA_DEMO_NARROW;
        if let Err(payload) = std::thread::spawn(move || {
            // 三档里只有 1024 档的裁剪区（281px）放不下占位卡的 288px 底沿。
            let expected = if width >= 1440 {
                expected_wide
            } else {
                expected_narrow
            };
            check_device_card_area(
                width,
                height,
                yeban_model::samples::demo_project(),
                "演示",
                expected,
                1,
            );
        })
        .join()
        {
            std::panic::resume_unwind(payload);
        }
        // (b) 空设备链：空态文案 ＋ 占位卡（三档同清单）。
        if let Err(payload) = std::thread::spawn(move || {
            check_device_card_area(
                width,
                height,
                project_without_devices(),
                "空链",
                CARD_AREA_EMPTY,
                0,
            );
        })
        .join()
        {
            std::panic::resume_unwind(payload);
        }
        // (c) 只有主总线：两条"逐字原因"行都出现（三档同清单）。
        if let Err(payload) = std::thread::spawn(move || {
            check_device_card_area(
                width,
                height,
                project_master_only(),
                "只有主总线",
                CARD_AREA_MASTER_ONLY,
                0,
            );
        })
        .join()
        {
            std::panic::resume_unwind(payload);
        }
    }
}

// ===========================================================================
// 判据 ②（装乐器行）：三档
// ===========================================================================
#[test]
fn the_instrument_install_row_has_no_two_controls_in_the_same_place() {
    for &(width, height) in TIERS {
        if let Err(payload) = std::thread::spawn(move || check_instrument_row(width, height)).join()
        {
            std::panic::resume_unwind(payload);
        }
    }
}

// ===========================================================================
// 判据 ③（轨路由四键区）：三档 ＋ 只有主总线（原因行）一档
// ===========================================================================
#[test]
fn the_track_routing_buttons_have_no_two_controls_in_the_same_place() {
    for &(width, height) in TIERS {
        if let Err(payload) = std::thread::spawn(move || {
            check_track_routing(
                width,
                height,
                yeban_model::samples::demo_project(),
                "演示",
                ROUTING_DEMO,
            );
        })
        .join()
        {
            std::panic::resume_unwind(payload);
        }
        if let Err(payload) = std::thread::spawn(move || {
            check_track_routing(
                width,
                height,
                project_master_only(),
                "只有主总线",
                ROUTING_MASTER_ONLY,
            );
        })
        .join()
        {
            std::panic::resume_unwind(payload);
        }
    }
}

// ===========================================================================
// 判据 ④（编曲画布）：三档，分组两两不相交
// ===========================================================================
#[test]
fn the_arrangement_canvas_has_no_two_controls_in_the_same_place() {
    for &(width, height) in TIERS {
        if let Err(payload) = std::thread::spawn(move || check_arrangement(width, height)).join() {
            std::panic::resume_unwind(payload);
        }
    }
}

// ===========================================================================
// 判据 ⑤（状态栏）：三档 × (提示态 / 保存回执态)
// ===========================================================================
#[test]
fn the_status_bar_has_no_two_controls_in_the_same_place() {
    for &(width, height) in TIERS {
        for save_status in [false, true] {
            if let Err(payload) =
                std::thread::spawn(move || check_status_bar(width, height, save_status)).join()
            {
                std::panic::resume_unwind(payload);
            }
        }
    }
}

// ===========================================================================
// 判据 ⑥（AI 右栏）：三档
// ===========================================================================
#[test]
fn the_ai_rail_has_no_two_controls_in_the_same_place() {
    for &(width, height) in TIERS {
        if let Err(payload) = std::thread::spawn(move || check_ai_rail(width, height)).join() {
            std::panic::resume_unwind(payload);
        }
    }
}

// ===========================================================================
// ⭐ 任务单 `#189`：最后剩下的面板段
// ===========================================================================

/// ② 控制台标签导轨带（28px）的 4 个控件；顺序 = 屏幕顺序。
const CONSOLE_RAIL: &[&str] = &[
    "tab-piano-roll-button",
    "tab-mixer-button",
    "tab-devices-button",
    "console-maximize-button",
];

/// ① 左栏**展开**态（240px）的 13 个控件：折叠键 ＋ 搜索框 ＋ 3 个分类 ＋ 8 个条目。
const SIDEBAR_EXPANDED: &[&str] = &[
    "sidebar-collapse-button",
    "sidebar-search-field",
    "sidebar-category-0-button",
    "sidebar-category-1-button",
    "sidebar-category-2-button",
    "sidebar-item-0",
    "sidebar-item-1",
    "sidebar-item-2",
    "sidebar-item-3",
    "sidebar-item-4",
    "sidebar-item-5",
    "sidebar-item-6",
    "sidebar-item-7",
];

/// ① 左栏**折叠**态（36px）的 9 个控件：搜索框与三个分类按设计 `visible: false`。
const SIDEBAR_COMPACT: &[&str] = &[
    "sidebar-collapse-button",
    "sidebar-item-0",
    "sidebar-item-1",
    "sidebar-item-2",
    "sidebar-item-3",
    "sidebar-item-4",
    "sidebar-item-5",
    "sidebar-item-6",
    "sidebar-item-7",
];

/// ③ Session 视图的控件清单（⭐ `#190` 起**按画布装得下的列数**生成）：
/// `visible` 条轨道包头 ＋ 每列 `scenes` 个插槽 ＋ `scenes` 个场景键 ＋ 场景列头
/// ＋ 返回编曲键 ＋（`visible < total` 时）截断报数。
fn session_controls(visible: usize, total: usize, scenes: usize) -> Vec<String> {
    let mut ids = Vec::new();
    for track in 0..visible {
        ids.push(format!("session-track-{track}-header"));
    }
    for track in 0..visible {
        for scene in 0..scenes {
            ids.push(format!("slot-{track}-{scene}-cell"));
        }
    }
    for scene in 0..scenes {
        ids.push(format!("scene-launch-{scene}-button"));
    }
    ids.push("scene-launch-column-header".to_owned());
    ids.push("session-back-to-arrangement-button".to_owned());
    if visible < total {
        ids.push("session-track-truncation-note".to_owned());
    }
    ids
}

/// ③ 画布装得下的轨道列数 —— 与 `session_view.slint` 的 `visible-track-count` **同一条
/// 算式**（场景列左沿 `8 + 128N` ＋ 宽 `104` ⇒ `N = (画布宽 − 112) / 128`，向下取整）。
fn session_track_capacity(canvas_width: u32) -> usize {
    (canvas_width.saturating_sub(112) / 128) as usize
}

/// ④ 主总线第二段（演示工程 / 0 台设备）的 6 个叶子控件。
/// `master-device-eq-panel` 是 `groupbox`（包住自己的 value / slider / reason）⇒ 容器组。
/// `master-device-rack-empty` 在场（0 台）。
const MASTER_LEAVES_EMPTY: &[&str] = &[
    "master-device-track-label",
    "master-device-rack-count",
    "master-device-insert-eq-button",
    "master-device-insert-reverb-button",
    "master-device-eq-reason",
    "master-device-rack-empty",
];

/// ④ 0 轨工程（**没有主总线**）的 7 个叶子：多一行逐字原因 `master-device-insert-reason`。
const MASTER_LEAVES_NO_MASTER: &[&str] = &[
    "master-device-track-label",
    "master-device-rack-count",
    "master-device-insert-eq-button",
    "master-device-insert-reverb-button",
    "master-device-eq-reason",
    "master-device-insert-reason",
    "master-device-rack-empty",
];

/// ④ 2 台主总线设备的 11 个叶子：6 个段级控件 ＋ 两张卡片里的 5 个语义子节点
/// （第 1 台有 `eq_low_gain` ⇒ 已识别行；两台都有旁通键 / 删除键）。
/// 卡片本身（`list-item`，按设计包住这 5 个）不在叶子里 ⇒ 进下面的容器组。
const MASTER_LEAVES_TWO: &[&str] = &[
    "master-device-track-label",
    "master-device-rack-count",
    "master-device-insert-eq-button",
    "master-device-insert-reverb-button",
    "master-device-eq-value",
    "master-device-eq-slider",
    "master-device-0-recognised",
    "master-device-0-bypass-switch",
    "master-device-0-remove-button",
    "master-device-1-bypass-switch",
    "master-device-1-remove-button",
];

/// ④ 主总线 EQ 面板（`groupbox`）**自己的**语义子节点 —— 它们与面板相交是设计。
const MASTER_EQ_PANEL_CHILDREN: &[&str] = &[
    "master-device-eq-value",
    "master-device-eq-slider",
    "master-device-eq-reason",
];

/// ⑤ `UndoTreeModal` 6 个真实节点（`node-hidden: 2`）时的 9 个控件。
fn undo_tree_controls() -> Vec<String> {
    let mut ids = vec!["undo-tree-close-button".to_owned()];
    for index in 0..6 {
        ids.push(format!("undo-tree-node-{index}"));
    }
    ids.push("undo-tree-truncation-note".to_owned());
    ids.push("undo-tree-undo-button".to_owned());
    ids
}

/// ⑤ 空提交链（`node-count: 0`）时的 3 个控件。
const UNDO_TREE_EMPTY: &[&str] = &[
    "undo-tree-close-button",
    "undo-tree-empty-state",
    "undo-tree-undo-button",
];

/// ⑥ 零提案抽屉的 3 个控件（图例 / 空态 / 关闭键）。`accept` / `reject` 零提案时按设计不画。
const MUSICAL_PR_EMPTY: &[&str] = &[
    "musical-pr-close-button",
    "musical-pr-diff-legend",
    "musical-pr-empty-state",
];

/// ⑦ 引擎错误横幅 —— 它自己就是这一段的**唯一**语义节点（文字无名）。
const BANNER: &[&str] = &["engine-error-banner"];

/// 断点按**窗口宽**的活窗口（`#189` 的 compact 夹具）。
///
/// ⚠ ⭐ `#191` 起 `MainWindow.compact` 是 `width < Tokens.breakpoint-full-hd` 的表达式
/// 绑定，本函数对 `scene.viewport_*` 的覆写**不再改变断点**（`build_main_window` 只读
/// `timecode` / `branch_name` / `arrangement_by_default`）。因此它现在与 [`live_window`]
/// 是**同一个形态**：两套夹具跑同一个窗口宽，读数必须逐字相同 —— 这本身是一条判据
/// （证明"没有第三个写者"）。函数保留是为了不改 `#189`/`#190` 的夹具命名与差集口径。
///
/// ⭐ `#195`：阈值 `1920 → 1180` ⇒ 1440 落**展开**档（左栏 240 ＋ 右栏常驻 280），
/// 1024 / 800 仍折叠（左栏 36 ＋ 右栏抽屉）。
fn live_window_at_breakpoint(
    project: YebanProjectV1,
    size: (u32, u32),
    tab: i32,
) -> LivePort<MainWindow> {
    let view = ViewState::from_project(&project).expect("演示工程必须能投影");
    let mut scene = DemoScene::from_view(&view);
    scene.viewport_width = size.0;
    scene.viewport_height = size.1;
    LivePort::new(
        Size::new(size.0, size.1),
        Permission::ReadOnly,
        None,
        || host::build_main_window_with_console_tab(&view, &scene, tab),
    )
    .expect("Tier-1 平台 + 生产窗口构造入口")
}

/// ④ 夹具：**只有主总线**，上面装 2 台设备（第 1 台带 `eq_low_gain` ⇒ EQ 滑块可用；
/// 第 2 台旁通且无参数 ⇒ 只有旁通键 / 删除键）。
fn project_master_with_two_devices() -> YebanProjectV1 {
    let master = EntityId::new();
    let devices = vec![
        DeviceDefinition {
            id: EntityId::new(),
            name: "主总线 EQ".to_owned(),
            kind: DeviceKind::InternalEffect,
            bypassed: false,
            params: vec![ParameterValue {
                name: "eq_low_gain".to_owned(),
                value: 6.0,
                unit: Some("dB".to_owned()),
            }],
            latency_samples: 0,
        },
        DeviceDefinition {
            id: EntityId::new(),
            name: "主总线混响".to_owned(),
            kind: DeviceKind::InternalEffect,
            bypassed: true,
            params: vec![],
            latency_samples: 0,
        },
    ];
    let mut tracks: BTreeMap<EntityId, TrackV3> = BTreeMap::new();
    tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            devices,
            ..TrackV3::default()
        },
    );
    YebanProjectV1 {
        master_bus_track_id: master,
        tracks,
        ..YebanProjectV1::default()
    }
}

/// ⭐ `#190` 夹具：主总线上 **`count` 台**无参数设备（只用来量"装得下几张卡"）。
/// 设备名逐台不同，便于从标签读出"画的是哪一台"。
fn project_master_with_devices(count: usize) -> YebanProjectV1 {
    let master = EntityId::new();
    let devices = (0..count)
        .map(|index| DeviceDefinition {
            id: EntityId::new(),
            name: format!("主总线设备 {index}"),
            kind: DeviceKind::InternalEffect,
            bypassed: index % 2 == 1,
            params: vec![],
            latency_samples: 0,
        })
        .collect();
    let mut tracks: BTreeMap<EntityId, TrackV3> = BTreeMap::new();
    tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            devices,
            ..TrackV3::default()
        },
    );
    YebanProjectV1 {
        master_bus_track_id: master,
        tracks,
        ..YebanProjectV1::default()
    }
}
/// 覆盖层的取法：段盒子内 **且** `id` 以本段前缀开头（见文件头 §「为什么用 id 前缀」）。
fn select_owned<'a>(
    tree: &'a ControlTree,
    pane: Bounds,
    prefix: &str,
) -> Vec<(&'a ControlNode, Bounds)> {
    select_controls(tree, pane, Selection::Intersect)
        .into_iter()
        .filter(|(node, _)| node.id.starts_with(prefix))
        .collect()
}

/// 同 [`assert_pairwise`]，但控件来自 [`select_owned`]；**每个**控件都必须整个落在段盒子里
/// （前缀对了、坐标出界一样红 —— 前缀不是"放宽几何"）。
fn assert_pairwise_owned(
    tree: &ControlTree,
    pane: Bounds,
    prefix: &str,
    expected: &[&str],
    label: &str,
) -> String {
    let controls = select_owned(tree, pane, prefix);

    let got: BTreeSet<&str> = controls.iter().map(|(node, _)| node.id.as_str()).collect();
    for id in expected {
        assert!(
            got.contains(id),
            "[{label}] 段内控件清单缺 `{id}` —— 两两判据会漏掉它（⛔ 不许 0 控件假绿）"
        );
    }
    assert_eq!(
        controls.len(),
        expected.len(),
        "[{label}] 段内控件数 {} ≠ 清单 {} ⇒ 有控件没登记（树里多出：{:?}）",
        controls.len(),
        expected.len(),
        got.iter()
            .filter(|id| !expected.contains(id))
            .collect::<Vec<_>>()
    );

    let (px, py, pw, ph) = pane;
    let right = px + pw as i32;
    let bottom = py + ph as i32;
    for (node, b) in &controls {
        assert!(
            b.0 >= px && b.1 >= py && b.0 + b.2 as i32 <= right && b.1 + b.3 as i32 <= bottom,
            "[{label}] `{}` (x={} y={} w={} h={}) 越出段盒子 {pane:?}",
            node.id,
            b.0,
            b.1,
            b.2,
            b.3
        );
    }

    let mut pair_count = 0_usize;
    let mut hits = Vec::new();
    for (index, (a, a_bounds)) in controls.iter().enumerate() {
        for (b, b_bounds) in controls.iter().skip(index + 1) {
            pair_count += 1;
            if intersect(*a_bounds, *b_bounds) {
                hits.push(format!(
                    "`{}` (x={} y={} w={} h={}) ⟂ `{}` (x={} y={} w={} h={})",
                    a.id,
                    a_bounds.0,
                    a_bounds.1,
                    a_bounds.2,
                    a_bounds.3,
                    b.id,
                    b_bounds.0,
                    b_bounds.1,
                    b_bounds.2,
                    b_bounds.3
                ));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "[{label}] 段内**有重叠**（{} 对）：\n{}",
        hits.len(),
        hits.join("\n")
    );

    let dump = controls
        .iter()
        .map(|(node, b)| format!("{}: x={} y={} w={} h={}", node.id, b.0, b.1, b.2, b.3))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "[{label}] 控件 {} 个 / {pair_count} 对 / 相交 {} 对（id 前缀 + 段盒子）\n{dump}",
        controls.len(),
        hits.len()
    )
}

/// 容器裁决的**另一条腿**：容器被排除 ≠ 它盖别人不算重叠。
///
/// `master-device-eq-panel` 是 `groupbox`（包住自己的 value / slider / reason），所以它在
/// 两两判据里被排除；但它是**不透明**的、后绘制 ⇒ 它压住**非子节点**的叶子就是真重叠。
/// 这里逐个断言：容器 ⟂ 段内每个**不是它自己子节点**的叶子 都不相交。
fn assert_container_clear_of(
    tree: &ControlTree,
    container: &str,
    leaves: &[&str],
    own_children: &[&str],
    label: &str,
) {
    let panel = bounds(tree, container);
    for id in leaves {
        if own_children.contains(id) {
            continue;
        }
        let b = bounds(tree, id);
        assert!(
            !intersect(panel, b),
            "[{label}] 容器 `{container}` {panel:?} 压住了非子节点 `{id}` {b:?}"
        );
    }
}

// ===========================================================================
// ① 左栏 `sidebar*`：展开 / 折叠两态 × 三档
// ===========================================================================
fn check_sidebar(width: u32, height: u32, breakpoint: bool) {
    let live = if breakpoint {
        live_window_at_breakpoint(yeban_model::samples::demo_project(), (width, height), 0)
    } else {
        live_window((width, height), 0)
    };
    let compact = live.ui().get_compact();
    let expected: &[&str] = if compact {
        SIDEBAR_COMPACT
    } else {
        SIDEBAR_EXPANDED
    };
    let tree = runtime_tree(live.ui());
    let pane = bounds(&tree, "sidebar");
    assert_eq!(
        pane.2,
        if compact { 36 } else { 240 },
        "[#189 左栏 {width}x{height} breakpoint={breakpoint}] 栏宽与 `compact={compact}` 不一致"
    );
    report_line(&assert_pairwise(
        &tree,
        pane,
        Selection::Intersect,
        expected,
        &format!("#189 左栏 {width}x{height} breakpoint={breakpoint} compact={compact}"),
    ));

    // ---- 防假绿：折叠态隐藏的是**两个入口族**，不是整栏消失。
    let collapse = tree
        .find_by_id("sidebar-collapse-button")
        .expect("折叠键三档都必须在场");
    assert_eq!(
        collapse.label,
        if compact {
            "展开左侧资源栏"
        } else {
            "收起左侧资源栏"
        },
        "[#189 左栏 {width}x{height}] 折叠键的无障碍名必须随态切换"
    );
    for id in ["sidebar-search-field", "sidebar-category-0-button"] {
        assert_eq!(
            tree.find_by_id(id).is_some(),
            !compact,
            "[#189 左栏 {width}x{height}] `{id}` 的在场性必须恰好等于 `!compact`"
        );
    }
}

// ===========================================================================
// ② 控制台标签导轨带 ＋ splitter 带
// ===========================================================================
fn check_console_rail(width: u32, height: u32) {
    let live = live_window((width, height), PIANO_TAB);
    let tree = runtime_tree(live.ui());

    let rail = bounds(&tree, "console-tab-rail");
    let band: Bounds = (rail.0, rail.1, rail.2, 28);
    report_line(&assert_pairwise(
        &tree,
        band,
        Selection::Inside,
        CONSOLE_RAIL,
        &format!("#189 控制台导轨带 {width}x{height}"),
    ));

    // 带高 28 是设计值（`piano-roll` 从 `y: 28px` 起）；导轨内容不许探进内容区。
    let tab_bottom = CONSOLE_RAIL
        .iter()
        .map(|id| {
            let b = bounds(&tree, id);
            b.1 + b.3 as i32
        })
        .max()
        .expect("清单非空");
    assert!(
        tab_bottom <= rail.1 + 28,
        "[#189 导轨 {width}x{height}] 导轨控件底沿 {tab_bottom} 越过 28px 带的下沿 {}",
        rail.1 + 28
    );
    let content = bounds(&tree, "piano-roll");
    assert_eq!(
        content.1,
        rail.1 + 28,
        "[#189 导轨 {width}x{height}] Tab 内容必须从 28px 带下沿开始（⛔ 不许被导轨压住）"
    );

    // ---- ⑦ `splitter-workspace-console`：4px 带里**只有它自己**。
    let splitter = bounds(&tree, "splitter-workspace-console");
    assert_eq!(
        (splitter.2, splitter.3),
        (width, 4),
        "[#189 splitter {width}x{height}] 分隔线必须是整窗宽 × 4px"
    );
    assert_eq!(
        splitter.1 + 4,
        rail.1,
        "[#189 splitter {width}x{height}] 分隔线底沿必须贴住控制台导轨顶沿"
    );
    report_line(&assert_pairwise(
        &tree,
        splitter,
        Selection::Inside,
        &["splitter-workspace-console"],
        &format!("#189 分隔线带 {width}x{height}"),
    ));
}

// ===========================================================================
// ③ Session 视图：展开态 / 真实断点 × 三档
// ===========================================================================
fn check_session_view(width: u32, height: u32, breakpoint: bool) {
    let live = if breakpoint {
        live_window_at_breakpoint(yeban_model::samples::demo_project(), (width, height), 0)
    } else {
        live_window((width, height), 0)
    };
    live.ui().set_arrangement_view(false);
    let tree = runtime_tree(live.ui());
    let pane = bounds(&tree, "workspace-session-canvas");

    let compact = live.ui().get_compact();
    // ⭐ `#190`：清单按**画布装得下的列数**生成（演示工程 6 轨、4 个场景）。
    let capacity = session_track_capacity(pane.2);
    let total = 6;
    let visible = capacity.min(total);
    let expected = session_controls(visible, total, 4);
    let expected_refs: Vec<&str> = expected.iter().map(String::as_str).collect();
    report_line(&assert_pairwise(
        &tree,
        pane,
        Selection::Intersect,
        &expected_refs,
        &format!(
            "#190 Session {width}x{height} breakpoint={breakpoint} compact={compact} \
             画布 {}px ⇒ 容量 {capacity} 列 / 显示 {visible} 列",
            pane.2
        ),
    ));

    // ---- 判据 ①：装不下的列**不画**（不是画出去被 `clip` 掉、用户看不见）。
    if visible < total {
        let first_hidden = format!("session-track-{visible}-header");
        assert!(
            tree.find_by_id(&first_hidden).is_none(),
            "[#190 Session {width}x{height}] 容量 {capacity} 列 ⇒ `{first_hidden}` 不许在树里"
        );
        assert!(
            tree.find_by_id(&format!("slot-{visible}-0-cell")).is_none(),
            "[#190 Session {width}x{height}] 容量 {capacity} 列 ⇒ 第 {visible} 列的插槽不许在树里"
        );
    }

    // ---- 判据 ②：场景列由**显示列数**定位，右沿必须落在画布内（缺口②的反向证明）。
    let scene_header = bounds(&tree, "scene-launch-column-header");
    assert_eq!(
        scene_header.0 - pane.0,
        8 + 128 * visible as i32,
        "[#190 Session {width}x{height}] 场景列左沿必须是 `8 + 128 × 显示列数`"
    );
    assert!(
        scene_header.0 + scene_header.2 as i32 <= pane.0 + pane.2 as i32,
        "[#190 Session {width}x{height}] 场景列右沿 {} 越过画布右沿 {}",
        scene_header.0 + scene_header.2 as i32,
        pane.0 + pane.2 as i32
    );

    // ---- 判据 ②：演示工程 6 轨：显示 < 6 时必须有截断报数，否则不许喊狼来了。
    if visible < total {
        let note = tree
            .find_by_id("session-track-truncation-note")
            .expect("显示列数 < 6 ⇒ 必须有截断报数");
        assert_eq!(
            note.label,
            format!("还有 {} 轨未显示", total - visible),
            "[#190 Session {width}x{height}] 截断报数文案必须逐字"
        );
        report_line(&format!(
            "[#190 Session {width}x{height}] 画布 {}px / 容量 {capacity} 列 / 显示 {visible} 列 / \
             场景列 (x={} y={} w={} h={}) 右沿 {} ≤ 画布右沿 {}；截断报数 {:?}",
            pane.2,
            scene_header.0,
            scene_header.1,
            scene_header.2,
            scene_header.3,
            scene_header.0 + scene_header.2 as i32,
            pane.0 + pane.2 as i32,
            note.label
        ));
    } else {
        assert!(
            tree.find_by_id("session-track-truncation-note").is_none(),
            "[#190 Session {width}x{height}] 显示 {visible}/6 列 ⇒ 截断报数不许在场"
        );
        report_line(&format!(
            "[#190 Session {width}x{height}] 画布 {}px / 容量 {capacity} 列 / 显示 {visible} 列 / \
             场景列 (x={} y={} w={} h={}) 右沿 {} ≤ 画布右沿 {}；{total} 轨全显示 ⇒ 无截断提示",
            pane.2,
            scene_header.0,
            scene_header.1,
            scene_header.2,
            scene_header.3,
            scene_header.0 + scene_header.2 as i32,
            pane.0 + pane.2 as i32,
        ));
    }

    // ---- 折叠档（⭐ `#195` 之后只有 1024 / 800）：画布 `窗口宽 − 36`（右栏是抽屉、不占位）。
    // ⭐ `#195` 阈值 `1920 → 1180`：1440 现在**展开**（画布 920 = 窗口宽 − 240 − 280）⇒
    // 这条断言的口径从"所有`breakpoint`夹具"收窄成"真的 `compact` 的档"。
    if breakpoint && compact {
        assert_eq!(
            pane.2,
            width - 36,
            "[#189 Session {width}x{height}] 折叠态画布宽必须是 `窗口宽 − 36`"
        );
    }
}

/// ③ ⭐ `#190` 的 7 轨夹具：第 7 轨在 1440 档（画布 920 ⇒ 容量 6）起必须被**截断报数**，
/// 而 1920 档（画布 1400 ⇒ 容量 10）全装得下、⛔ 不许出现提示。
fn check_session_track_truncation(width: u32, height: u32) {
    let live = live_window_for(project_with_seven_tracks(), (width, height), 0);
    live.ui().set_arrangement_view(false);
    let tree = runtime_tree(live.ui());
    let pane = bounds(&tree, "workspace-session-canvas");

    let capacity = session_track_capacity(pane.2);
    let total = 7;
    let visible = capacity.min(total);
    let label = format!("#190 Session 7 轨 {width}x{height}");

    // ---- 判据 ①：7 轨只画装得下的前 N 列；第 N+1 轨不在树里。
    let expected = session_controls(visible, total, 4);
    let expected_refs: Vec<&str> = expected.iter().map(String::as_str).collect();
    report_line(&assert_pairwise(
        &tree,
        pane,
        Selection::Intersect,
        &expected_refs,
        &format!(
            "{label} 画布 {}px ⇒ 容量 {capacity} 列 / 显示 {visible} 列 / 隐藏 {} 列",
            pane.2,
            total - visible
        ),
    ));
    assert!(
        tree.find_by_id(&format!("session-track-{visible}-header"))
            .is_none(),
        "[{label}] 容量 {capacity} 列 ⇒ `session-track-{visible}-header` 不许在树里"
    );

    // ---- 判据 ②：截断报数**逐字** + 有 `accessible-id`；数字 = 7 − 显示列数。
    if visible < total {
        let note = tree
            .find_by_id("session-track-truncation-note")
            .expect("7 轨 > 容量 ⇒ 必须有截断报数");
        let expected_text = format!("还有 {} 轨未显示", total - visible);
        assert_eq!(
            note.label, expected_text,
            "[{label}] 截断报数文案必须逐字；实际 {:?}",
            note.label
        );
        let note_bounds = note.bounds.expect("截断报数必须有几何");
        assert!(
            note_bounds.y + note_bounds.height as i32 <= pane.1 + pane.3 as i32,
            "[{label}] 截断报数底沿 {} 越过画布底沿 {}",
            note_bounds.y + note_bounds.height as i32,
            pane.1 + pane.3 as i32
        );
        report_line(&format!(
            "[{label}] 截断报数 `session-track-truncation-note` label={:?} \
             bounds=(x={} y={} w={} h={})",
            note.label, note_bounds.x, note_bounds.y, note_bounds.width, note_bounds.height
        ));
    } else {
        assert!(
            tree.find_by_id("session-track-truncation-note").is_none(),
            "[{label}] 7 列全装得下 ⇒ 截断报数不许在场"
        );
    }

    // ---- 判据 ⑥：截断只是显示层 —— 注入的轨道名一个不少（工程没被删轨）。
    assert_eq!(
        slint::Model::row_count(&live.ui().get_track_names()),
        total,
        "[{label}] 截断后注入的轨道数必须仍是 {total}（⛔ 显示层不许改模型）"
    );
    report_line(&format!(
        "[{label}] 注入 `track-names` row_count={}（工程 7 轨不变）",
        slint::Model::row_count(&live.ui().get_track_names())
    ));
}

// ===========================================================================
// ④ 主总线设备链**段内**：三种夹具
// ===========================================================================
fn check_master_device_segment(
    width: u32,
    height: u32,
    project: YebanProjectV1,
    tag: &str,
    leaves: &[&str],
    cards: &[&str],
    child_pairs: &[(&str, &str)],
) {
    // 判据 ⑥ 的正向基线：**权威工程里**设备数（截断前后都必须等于它）。
    let total_devices = project
        .tracks
        .get(&project.master_bus_track_id)
        .map(|track| track.devices.len())
        .unwrap_or(0);
    let live = live_window_for(project, (width, height), 2);
    assert_eq!(live.ui().get_console_tab(), 2, "夹具必须是 Tab 2（设备架）");
    let tree = runtime_tree(live.ui());

    let rack = bounds(&tree, "master-device-rack");
    let label = format!("#190 主总线第二段 {tag} {width}x{height}");

    // ---- ⭐ `#190` 判据 ③：容量 `N` 由**段高 / 卡片步长**算出，⛔ 不写死。
    // `master-device-rack` 的高度就是段容器高度（与 `.slint` 的 `master-segment-height` 同值）。
    let capacity = ((rack.3 as i32 - 72) / 46).max(0) as usize;
    let visible = capacity.min(total_devices);
    report_line(&format!(
        "[{label}] 段高 {}px / 卡片步长 46px ⇒ N = floor(({} − 72) / 46) = {capacity} 台；\
         工程 {total_devices} 台 ⇒ 画 {visible} 张",
        rack.3, rack.3
    ));

    // ---- ⭐ `#190` 判据 ①＋②：第 N+1 张卡不画；截断报数**逐字**、有 id；计数格让位。
    assert_eq!(
        cards.len(),
        visible,
        "[{label}] 卡片组应有 {visible} 张（容量 {capacity} / 工程 {total_devices}）"
    );
    if total_devices > capacity {
        let hidden = total_devices - capacity;
        let note = tree
            .find_by_id("master-device-truncation-note")
            .expect("设备数 > 容量 ⇒ 必须有截断报数");
        let expected_text = format!("还有 {hidden} 台设备未显示");
        assert_eq!(
            note.label, expected_text,
            "[{label}] 截断报数文案必须逐字；实际 {:?}",
            note.label
        );
        assert!(
            tree.find_by_id(&format!("master-device-{capacity}-card"))
                .is_none(),
            "[{label}] 容量 {capacity} 台 ⇒ `master-device-{capacity}-card` 不许在树里"
        );
        assert!(
            tree.find_by_id("master-device-rack-count").is_none(),
            "[{label}] 截断时计数格让给报数（两者互斥）⇒ `master-device-rack-count` 不许在场"
        );
        let note_bounds = note.bounds.expect("截断报数必须有几何");
        assert!(
            note_bounds.y + note_bounds.height as i32 <= rack.1 + rack.3 as i32,
            "[{label}] 截断报数底沿 {} 越过段底 {}",
            note_bounds.y + note_bounds.height as i32,
            rack.1 + rack.3 as i32
        );
        report_line(&format!(
            "[{label}] 截断报数 `master-device-truncation-note` label={:?} \
             bounds=(x={} y={} w={} h={})；第 {capacity} 张卡不在树里",
            note.label, note_bounds.x, note_bounds.y, note_bounds.width, note_bounds.height
        ));
    } else {
        assert!(
            tree.find_by_id("master-device-truncation-note").is_none(),
            "[{label}] 设备数 {total_devices} ≤ 容量 {capacity} ⇒ 截断报数不许在场"
        );
        let count = tree
            .find_by_id("master-device-rack-count")
            .expect("未截断 ⇒ 计数格必须在场");
        assert_eq!(
            count.label,
            format!("{total_devices} 台设备"),
            "[{label}] 未截断时计数格文案必须逐字"
        );
    }

    // ---- 判据 ⑥：截断只是显示层 —— 注入的设备数不变（⛔ 不删设备）。
    assert_eq!(
        slint::Model::row_count(&live.ui().get_master_device_names()),
        total_devices,
        "[{label}] 截断后注入的设备数必须仍是 {total_devices}（⛔ 显示层不许改模型）"
    );
    report_line(&format!(
        "[{label}] 注入 `master-device-names` row_count={}（工程 {total_devices} 台不变）",
        slint::Model::row_count(&live.ui().get_master_device_names())
    ));

    // ---- 叶子：`list-item` 卡片按设计包住子节点 ⇒ 从叶子集里摘掉，进下面的容器组。
    let excluded: Vec<String> = cards.iter().map(|id| (*id).to_owned()).collect();
    report_line(&assert_pairwise_excluding(
        &tree,
        rack,
        Selection::Intersect,
        leaves,
        &excluded,
        &label,
    ));

    // ---- 容器组：卡片**彼此**两两不相交（卡片与卡片之间 2px 间隙）。
    if !cards.is_empty() {
        report_line(&assert_ids_pairwise(
            &tree,
            cards,
            &format!("主总线卡片组 {tag} {width}x{height}"),
        ));
    }
    // ---- 容器被排除 ≠ 不检查：每张卡片的语义子节点必须整个落在卡内。
    for (child, card) in child_pairs {
        assert_inside(&tree, child, card, &label);
    }

    // ---- ④ 的核心裁决：`groupbox` 面板包住自己的三个子节点（排除），但**不许**压别人。
    assert_inside(
        &tree,
        "master-device-eq-panel",
        "master-device-rack",
        &label,
    );
    for id in MASTER_EQ_PANEL_CHILDREN {
        if tree.find_by_id(id).is_some() {
            assert_inside(&tree, id, "master-device-eq-panel", &label);
        }
    }
    assert_container_clear_of(
        &tree,
        "master-device-eq-panel",
        leaves,
        MASTER_EQ_PANEL_CHILDREN,
        &label,
    );

    // ---- 卡片不许越过段底（固定 260px 高的段）。
    for card in cards {
        let b = bounds(&tree, card);
        assert!(
            b.1 + b.3 as i32 <= rack.1 + rack.3 as i32,
            "[{label}] `{card}` 底沿 {} 越过段底 {}（半张卡被 clip 吃掉）",
            b.1 + b.3 as i32,
            rack.1 + rack.3 as i32
        );
    }

    // ---- 判据 ⑤：面板左沿的单一算式（原因行的宽度就是按它收的）。
    let panel = bounds(&tree, "master-device-eq-panel");
    assert_eq!(
        panel.0 - rack.0,
        rack.2 as i32 - panel.2 as i32 - 16,
        "[{label}] 面板左沿必须是 `段宽 − 面板宽 − 16`（原因行宽度的唯一依据）"
    );
    if tree.find_by_id("master-device-insert-reason").is_some() {
        let reason = bounds(&tree, "master-device-insert-reason");
        assert!(
            reason.0 + reason.2 as i32 <= panel.0,
            "[{label}] 原因行右沿 {} 必须 ≤ 面板左沿 {}（⛔ 不许伸到面板底下）",
            reason.0 + reason.2 as i32,
            panel.0
        );
        report_line(&format!(
            "[{label}] 原因行 (x={} y={} w={} h={}) 右沿 {} ≤ 面板左沿 {} ⇒ 不相交",
            reason.0,
            reason.1,
            reason.2,
            reason.3,
            reason.0 + reason.2 as i32,
            panel.0
        ));
    }
}

// ===========================================================================
// ④b ⭐ `#190` 主总线卡片容量 / 截断报数：4 台与 5 台两档
// ===========================================================================
///
/// 判据（逐条对应 `#190` §2）：
/// ① 容量 `N = floor((段高 − 72) / 46)`（段高由运行时 `master-device-rack` 量出，⛔ 不写死）；
/// ② 只画前 `N` 张卡；第 `N+1` 张**不在树里**；
/// ③ `5 台 ⇒ 4 张卡 ＋ "还有 1 台设备未显示"`（1920 档；文案逐字、有 accessible-id）；
/// ④ 段内非容器叶子两两不相交（含截断报数）；
/// ⑥ 注入的设备数不变（显示层截断，⛔ 不删设备）。
fn check_master_device_truncation(width: u32, height: u32, device_count: usize) {
    let project = project_master_with_devices(device_count);
    let model_devices = project
        .tracks
        .get(&project.master_bus_track_id)
        .map(|track| track.devices.len())
        .unwrap_or(0);
    let live = live_window_for(project, (width, height), 2);
    let tree = runtime_tree(live.ui());
    let rack = bounds(&tree, "master-device-rack");
    let label = format!("#190 主总线容量 {device_count} 台 {width}x{height}");

    let capacity = ((rack.3 as i32 - 72) / 46).max(0) as usize;
    let visible = capacity.min(device_count);
    report_line(&format!(
        "[{label}] 段高 {}px / 卡片首行 72px / 步长 46px ⇒ N = floor(({} − 72) / 46) = {capacity}；\
         工程 {device_count} 台 ⇒ 画 {visible} 张 / 隐藏 {} 台",
        rack.3,
        rack.3,
        device_count.saturating_sub(visible)
    ));

    // ---- 判据 ①：卡片组恰好是前 `visible` 张；第 `visible` 张不在树里。
    let cards: Vec<String> = (0..visible)
        .map(|index| format!("master-device-{index}-card"))
        .collect();
    assert!(
        tree.find_by_id(&format!("master-device-{visible}-card"))
            .is_none(),
        "[{label}] 容量 {capacity} ⇒ `master-device-{visible}-card` 不许在树里"
    );

    // ---- 判据 ④：段内非容器叶子清单（卡片是 `list-item` 容器 ⇒ 摘出，进容器组）。
    let mut leaves: Vec<String> = [
        "master-device-track-label",
        "master-device-insert-eq-button",
        "master-device-insert-reverb-button",
        "master-device-eq-reason",
    ]
    .iter()
    .map(|id| (*id).to_owned())
    .collect();
    if device_count > capacity {
        leaves.push("master-device-truncation-note".to_owned());
    } else {
        leaves.push("master-device-rack-count".to_owned());
    }
    let mut child_pairs: Vec<(String, String)> = Vec::new();
    for index in 0..visible {
        leaves.push(format!("master-device-{index}-bypass-switch"));
        leaves.push(format!("master-device-{index}-remove-button"));
        child_pairs.push((
            format!("master-device-{index}-bypass-switch"),
            format!("master-device-{index}-card"),
        ));
        child_pairs.push((
            format!("master-device-{index}-remove-button"),
            format!("master-device-{index}-card"),
        ));
    }
    let leaf_refs: Vec<&str> = leaves.iter().map(String::as_str).collect();
    let card_refs: Vec<&str> = cards.iter().map(String::as_str).collect();
    report_line(&assert_pairwise_excluding(
        &tree,
        rack,
        Selection::Intersect,
        &leaf_refs,
        &cards,
        &label,
    ));
    report_line(&assert_ids_pairwise(
        &tree,
        &card_refs,
        &format!("主总线卡片组 {device_count} 台 {width}x{height}"),
    ));
    for (child, card) in &child_pairs {
        assert_inside(&tree, child, card, &label);
    }
    assert_container_clear_of(
        &tree,
        "master-device-eq-panel",
        &leaf_refs,
        MASTER_EQ_PANEL_CHILDREN,
        &label,
    );
    for card in &cards {
        let b = bounds(&tree, card);
        assert!(
            b.1 + b.3 as i32 <= rack.1 + rack.3 as i32,
            "[{label}] `{card}` 底沿 {} 越过段底 {}（半张卡被 clip 吃掉）",
            b.1 + b.3 as i32,
            rack.1 + rack.3 as i32
        );
    }

    // ---- 判据 ②＋③：截断报数逐字 / 在场性；未截断时计数格逐字。
    if device_count > capacity {
        let note = tree
            .find_by_id("master-device-truncation-note")
            .expect("设备数 > 容量 ⇒ 必须有截断报数");
        let expected_text = format!("还有 {} 台设备未显示", device_count - capacity);
        assert_eq!(
            note.label, expected_text,
            "[{label}] 截断报数文案必须逐字；实际 {:?}",
            note.label
        );
        assert!(
            tree.find_by_id("master-device-rack-count").is_none(),
            "[{label}] 截断时计数格必须让位（两者互斥）"
        );
        report_line(&format!(
            "[{label}] 卡片 {visible} 张；`master-device-truncation-note` label={:?}",
            note.label
        ));
    } else {
        assert!(
            tree.find_by_id("master-device-truncation-note").is_none(),
            "[{label}] {device_count} 台 ≤ 容量 {capacity} ⇒ 截断报数不许在场"
        );
        assert_eq!(
            tree.find_by_id("master-device-rack-count")
                .expect("未截断 ⇒ 计数格在场")
                .label,
            format!("{device_count} 台设备"),
            "[{label}] 未截断时计数格文案必须逐字"
        );
        report_line(&format!(
            "[{label}] 卡片 {visible} 张；无截断报数（计数格逐字 \"{device_count} 台设备\"）"
        ));
    }

    // ---- 判据 ⑥：显示层截断，模型里的设备数**不变**。
    assert_eq!(
        model_devices, device_count,
        "[{label}] 权威工程的设备数必须仍是 {device_count}"
    );
    assert_eq!(
        slint::Model::row_count(&live.ui().get_master_device_names()),
        device_count,
        "[{label}] 注入的设备数必须仍是 {device_count}（⛔ 不删设备）"
    );
}

// ===========================================================================
// ⑤ `UndoTreeModal`
// ===========================================================================
fn check_undo_tree(width: u32, height: u32, with_nodes: bool) {
    let live = live_window((width, height), 0);
    if with_nodes {
        let labels: Vec<slint::SharedString> = (0..6)
            .map(|index| slint::SharedString::from(format!("c{index} 提交")))
            .collect();
        live.ui().set_undo_node_count(6);
        live.ui()
            .set_undo_node_labels(slint::ModelRc::new(slint::VecModel::from(labels)));
        live.ui().set_undo_node_hidden(2);
    }
    live.ui().set_undo_tree_open(true);

    let tree = runtime_tree(live.ui());
    let modal = bounds(&tree, "undo-tree-modal");
    assert_eq!(
        modal,
        (0, 0, width, height),
        "[#189 时光机 {width}x{height}] 模态覆盖层必须铺满整窗"
    );
    let panel = bounds(&tree, "undo-tree-panel");
    assert_eq!(
        (panel.0, panel.1, panel.2, panel.3),
        (
            (width as i32 - 720) / 2,
            (height as i32 - 360) / 2,
            720,
            360
        ),
        "[#189 时光机 {width}x{height}] 面板必须 720×360 居中"
    );

    let expected: Vec<String> = if with_nodes {
        undo_tree_controls()
    } else {
        UNDO_TREE_EMPTY.iter().map(|id| (*id).to_owned()).collect()
    };
    let expected_refs: Vec<&str> = expected.iter().map(String::as_str).collect();
    report_line(&assert_pairwise_owned(
        &tree,
        panel,
        "undo-tree-",
        &expected_refs,
        &format!("#189 时光机 {width}x{height} nodes={with_nodes}"),
    ));

    // ---- 防假绿：空链画的是**空态**而不是 0 个节点；有链时节点数与注入一致。
    assert_eq!(
        tree.find_by_id("undo-tree-empty-state").is_some(),
        !with_nodes,
        "[#189 时光机 {width}x{height}] 空态的在场性必须恰好等于 `node-count == 0`"
    );
    assert_eq!(
        tree.find_by_id("undo-tree-truncation-note").is_some(),
        with_nodes,
        "[#189 时光机 {width}x{height}] 截断报数必须只在 `node-hidden > 0` 时在场"
    );
}

// ===========================================================================
// ⑥ `MusicalPrDrawer`
// ===========================================================================
fn check_musical_pr_drawer(width: u32, height: u32) {
    let live = live_window((width, height), 0);
    live.ui().set_musical_pr_open(true);
    let tree = runtime_tree(live.ui());

    let drawer = bounds(&tree, "musical-pr-drawer");
    assert_eq!(
        drawer,
        (width as i32 - 360, 0, 360, height),
        "[#189 提案抽屉 {width}x{height}] 抽屉必须 360px 宽、贴右沿、整窗高"
    );
    report_line(&assert_pairwise_owned(
        &tree,
        drawer,
        "musical-pr-",
        MUSICAL_PR_EMPTY,
        &format!("#189 提案抽屉 {width}x{height}"),
    ));

    // ---- 防假绿：零提案时**空态**在场、采纳/放弃按设计不在（假控件不留）。
    for id in ["musical-pr-accept-button", "musical-pr-reject-button"] {
        assert!(
            tree.find_by_id(id).is_none(),
            "[#189 提案抽屉 {width}x{height}] 零提案时 `{id}` 必须不在树里"
        );
    }
    assert_eq!(
        tree.find_by_id("musical-pr-diff-legend")
            .expect("图例必须在场")
            .bounds
            .expect("图例必须有几何")
            .height,
        68
    );
}

// ===========================================================================
// ⑦ `EngineErrorBanner`
// ===========================================================================
fn check_engine_error_banner(width: u32, height: u32) {
    let live = live_window((width, height), 0);
    live.ui()
        .set_engine_error(slint::SharedString::from("发布快照失败：环满"));
    let tree = runtime_tree(live.ui());

    let banner = bounds(&tree, "engine-error-banner");
    assert_eq!(
        banner,
        (0, height as i32 - 24 - 28, width, 28),
        "[#189 错误横幅 {width}x{height}] 横幅必须整窗宽、28px 高、紧贴状态栏上沿"
    );
    report_line(&assert_pairwise_owned(
        &tree,
        banner,
        "engine-error-",
        BANNER,
        &format!("#189 错误横幅 {width}x{height}"),
    ));

    let node = tree
        .find_by_id("engine-error-banner")
        .expect("横幅必须在场");
    assert_eq!(
        node.value.as_deref(),
        Some("发布快照失败：环满"),
        "[#189 错误横幅 {width}x{height}] `accessible-value` 必须是错误原文"
    );

    // ---- 判据 ⑤：横幅与 24px 状态栏**边贴边**，不压状态栏的任何一个控件。
    let status = bounds(&tree, "status-bar");
    assert_eq!(
        banner.1 + 28,
        status.1,
        "[#189 错误横幅 {width}x{height}] 横幅底沿必须等于状态栏顶沿"
    );
    for id in [
        "status-bar-selection",
        "status-bar-chord",
        "status-bar-shortcut-tip",
        "status-bar-device",
    ] {
        let b = bounds(&tree, id);
        assert!(
            !intersect(banner, b),
            "[#189 错误横幅 {width}x{height}] 横幅 {banner:?} ⟂ `{id}` {b:?}"
        );
    }
}

// ===========================================================================
// ① 左栏：三档 × (展开夹具 / 真实断点夹具)
// ===========================================================================
#[test]
fn the_sidebar_has_no_two_controls_in_the_same_place() {
    for &(width, height) in TIERS {
        for breakpoint in [false, true] {
            if width == 1920 && breakpoint {
                // ⭐ `#191` 起两套夹具都按窗口宽判断点 ⇒ 1920 下必然同形态，跳过重复那趟。
                continue;
            }
            if let Err(payload) =
                std::thread::spawn(move || check_sidebar(width, height, breakpoint)).join()
            {
                std::panic::resume_unwind(payload);
            }
        }
    }
}

// ===========================================================================
// ② 控制台导轨带 ＋ splitter 带：三档
// ===========================================================================
#[test]
fn the_console_tab_rail_has_no_two_controls_in_the_same_place() {
    for &(width, height) in TIERS {
        if let Err(payload) = std::thread::spawn(move || check_console_rail(width, height)).join() {
            std::panic::resume_unwind(payload);
        }
    }
}

// ===========================================================================
// ③ Session 视图：三档 × (展开夹具 / 真实断点夹具)
// ===========================================================================
#[test]
fn the_session_view_has_no_two_controls_in_the_same_place() {
    for &(width, height) in TIERS {
        for breakpoint in [false, true] {
            if width == 1920 && breakpoint {
                continue;
            }
            if let Err(payload) =
                std::thread::spawn(move || check_session_view(width, height, breakpoint)).join()
            {
                std::panic::resume_unwind(payload);
            }
        }
    }
}

// ===========================================================================
// ③b ⭐ `#190` Session 7 轨：截断报数 ＋ 段内 0 相交
// ===========================================================================
#[test]
fn the_session_view_reports_tracks_that_did_not_fit() {
    for &(width, height) in TIERS {
        if let Err(payload) =
            std::thread::spawn(move || check_session_track_truncation(width, height)).join()
        {
            std::panic::resume_unwind(payload);
        }
    }
}

// ===========================================================================
// ④ 主总线第二段段内：三档 × 三种夹具
// ===========================================================================
#[test]
fn the_master_device_segment_has_no_two_controls_in_the_same_place() {
    for &(width, height) in TIERS {
        // (a) 演示工程：主总线 0 台设备 ⇒ 空态在场；EQ 不可用 ⇒ 原因行在场。
        if let Err(payload) = std::thread::spawn(move || {
            check_master_device_segment(
                width,
                height,
                yeban_model::samples::demo_project(),
                "0 台",
                MASTER_LEAVES_EMPTY,
                &[],
                &[],
            );
        })
        .join()
        {
            std::panic::resume_unwind(payload);
        }
        // (b) 0 轨工程（没有主总线）⇒ 装设备原因行 ＋ EQ 原因行同时在场。
        if let Err(payload) = std::thread::spawn(move || {
            check_master_device_segment(
                width,
                height,
                YebanProjectV1::default(),
                "无主总线",
                MASTER_LEAVES_NO_MASTER,
                &[],
                &[],
            );
        })
        .join()
        {
            std::panic::resume_unwind(payload);
        }
        // (c) 2 台设备 ⇒ 卡片链（`list-item` 容器组）＋ 每张卡片的语义子节点。
        if let Err(payload) = std::thread::spawn(move || {
            check_master_device_segment(
                width,
                height,
                project_master_with_two_devices(),
                "2 台",
                MASTER_LEAVES_TWO,
                &["master-device-0-card", "master-device-1-card"],
                &[
                    ("master-device-0-recognised", "master-device-0-card"),
                    ("master-device-0-bypass-switch", "master-device-0-card"),
                    ("master-device-0-remove-button", "master-device-0-card"),
                    ("master-device-1-bypass-switch", "master-device-1-card"),
                    ("master-device-1-remove-button", "master-device-1-card"),
                ],
            );
        })
        .join()
        {
            std::panic::resume_unwind(payload);
        }
        // (d) ⭐ `#190`：4 台（1920 档容量 4 ⇒ 全装下、无提示）与 5 台（⇒ 4 张卡 ＋ 提示）。
        for device_count in [4_usize, 5] {
            if let Err(payload) = std::thread::spawn(move || {
                check_master_device_truncation(width, height, device_count);
            })
            .join()
            {
                std::panic::resume_unwind(payload);
            }
        }
    }
}

// ===========================================================================
// ⑤ 时光机：三档 × (6 节点 / 空链)
// ===========================================================================
#[test]
fn the_undo_tree_modal_has_no_two_controls_in_the_same_place() {
    for &(width, height) in TIERS {
        for with_nodes in [false, true] {
            if let Err(payload) =
                std::thread::spawn(move || check_undo_tree(width, height, with_nodes)).join()
            {
                std::panic::resume_unwind(payload);
            }
        }
    }
}

// ===========================================================================
// ⑥ 提案抽屉：三档（零提案）
// ===========================================================================
#[test]
fn the_musical_pr_drawer_has_no_two_controls_in_the_same_place() {
    for &(width, height) in TIERS {
        if let Err(payload) =
            std::thread::spawn(move || check_musical_pr_drawer(width, height)).join()
        {
            std::panic::resume_unwind(payload);
        }
    }
}

// ===========================================================================
// ⑦ 引擎错误横幅：三档
// ===========================================================================
#[test]
fn the_engine_error_banner_has_no_two_controls_in_the_same_place() {
    for &(width, height) in TIERS {
        if let Err(payload) =
            std::thread::spawn(move || check_engine_error_banner(width, height)).join()
        {
            std::panic::resume_unwind(payload);
        }
    }
}
