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
//! | ⑥ 预算 | 控件数逐档不变：钢琴卷帘 `20`、走带 `6`（1920 档）/`7`（1440/1024 档，多的是溢出钮）；调音台 `69` 个旧 id、设备架 / 轨路由 / 主总线段 / AI 右栏的旧 id **全是 0 个活节点**（注册表登记数不变 —— 死的是界面，不是登记）；素材池 `4` 个活节点（T-FIX-356 已入住活下坞） | 证"没有靠删控件换绿" |
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
//! 「容器 ⟂ 非子节点叶子」的显式断言（T-FIX-356 起该段已死，断言随之删除）。
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
//!
//! ---
//!
//! # ⭐ T-FIX-356（2026-11-24）：旧控制台死了，判据改钉活树
//!
//! 本文件在 T-FIX-356 之前钉的是旧控制台（`ConsoleTabs` 全家：调音台 / 素材池 /
//! 设备架 / 轨路由 / 主总线段 / 模块入口）与旧右栏（`ai-rail`）的几何；那些组件今天
//! 全在 `ui/app.slint:3116` 的 `if false` 兼容块里（`ConsoleTabs`）或另一处 `if false`
//! 块里（`ai-rail`），活树里一个节点都没有 —— 旧几何判据无从量起。本轮按 T-FIX-219
//! 的诚实形态改钉，活树实测（`tests/zzz_probe_tree.rs`，三档 × 三个控制台页签，
//! 2026-11-24，`LANG=en_US.UTF-8`）重推导的读数：
//!
//! | 段 | 活树实测（2026-11-24） |
//! | :--- | :--- |
//! | 钢琴卷帘 | 20 个控件（6 音符 → **5** 个：A2 那条不在演示工程里；`piano-roll-status` 标签是英文 `"Piano Roll Snap & Range Status"`；另有音域键 `pitch-range-down/up` 各 1） |
//! | 走带 | 带 = `y ∈ [40, 92)`（40px 标题栏之下）；常驻 6 个 ＋ 溢出钮只在 1440/1024 档（1920 档没有它）；其余 11 个旧 id 只活在注册表 `known_absent` 登记里 |
//! | 调音台 / 素材池 / 设备架 / 轨路由 / 主总线段 | **0 个活节点**：注册表登记 ＋ 死 `.slint` 声明 ＋ `if false` 门住 ＋ 活树缺席（四边契约面判据）；活的替代面是下控制台导轨带（见下） |
//! | AI 右栏 | **0 个活节点**：同上四边；死按钮 `ai-rail-musical-pr-button` 的回调 `open-musical-pr()` 仍能打开真的 `MusicalPrDrawer` |
//! | 编曲画布 | 画布右沿 = 窗口右沿（三档都如此，右栏已死）；1024 档第 6 行包头被画布裁出树（可滚，见 `arrangement_vertical_scroll.rs`）；`track-*-color-swatch` / `section-*-card` / `arrangement-ruler` / `arrangement-playhead` 全死 |
//! | 控制台导轨带 | 导轨顶 28px 带里只有 3 个键（最大化 / 宏 / 插件面板）；`piano-roll` 从导轨顶沿 + 42px 起；分隔线 5px、在导轨顶沿 − 35px；歌曲总览三键在导轨顶沿 − 30px |
//! | Session 7 轨截断 | 场景行数按画布高算（1024×800 档只画 3 行，不是 4 行）；截断报语文案按语言钉死（`apply_language(ZhCn)`） |
//! | 时光机模态 | 覆盖层 `(0, 40, W, H−80)`（上下各留 40px）；面板仍是 720×360 居中 |
//! | 提案抽屉 | `(W−384, 92, 372, H−172)`（右沿留 12px、下沿留 80px） |
//! | 引擎错误横幅 | `(16, 92, W−32, 36)`（顶栏之下 16px 边距）；横幅里多了一个 `engine-error-open-sound-library` 键 |
//!
//! 死段的"四边契约面"助手见 [`dead_id_contract`]：注册表登记（角色＋组件文件）＋
//! 死文件声明 ＋ `if false` 门 ＋ 活树缺席。删几何不断言是诚实，不是放宽 ——
//! 几何无从量起的段，量"它确实死了、死在哪、活的替代面在哪"。

#![allow(unused)]
use std::collections::{BTreeMap, BTreeSet};

use yeban_app::bridge::ViewState;
use yeban_app::elements::{ElementKind, ElementRegistry};
use yeban_app::host;
use yeban_app::samples::SAMPLE_LIBRARY_ENV;
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
/// 走带带：`y ∈ [40, 92)` —— 40px 标题栏之下、编曲画布顶沿（y=89…92）之上。
/// 旧 `TOP_BAR = 48` 是死文件 `ui/transport.slint` 的顶栏高；活顶栏
/// （`ui/transport_bar.slint`）的按钮在 y=50..78。
const TRANSPORT_BAND_Y: i32 = 40;
const TRANSPORT_BAND_H: u32 = 52;

/// 走带常驻的 6 个控件（活树实测，三档都有）。
const TRANSPORT_LIVE: &[&str] = &[
    "transport-stop-button",
    "transport-play-button",
    "transport-record-button",
    "transport-loop-button",
    "transport-timecode",
    "transport-tempo-value",
];

/// 只在窄档出现的溢出钮：1920 档走带装得下、不画；1440/1024 档在树里。
const TRANSPORT_OVERFLOW: &str = "transport-overflow-button";

/// 死走带 id：只活在注册表的 `known_absent` 登记里（理由见 `elements.rs`），
/// 活树里没有。`TRANSPORT_LIVE` 之外的旧 `TRANSPORT_WIDE` 成员全在这里。
const TRANSPORT_DEAD: &[&str] = &[
    "transport-bpm-field",
    "transport-branch-button",
    "transport-commit-button",
    "transport-revert-button",
    "transport-save-button",
    "transport-ai-proposal-badge",
    "transport-metronome-button",
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
    "pitch-range-down",
    "pitch-range-up",
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

/// 素材池（演示工程 `clip_pool` 2 条）：条目数 ＋ 两行 ＋ 导入入口。
const ASSET_CONTROLS: &[&str] = &[
    "asset-pool-count",
    "asset-pool-import-button",
    "asset-pool-row-0-row",
    "asset-pool-row-1-row",
];

/// 调音台的 69 个旧控件：3 个模块入口 ＋ 6 个主控控件 ＋ 6 条通道条 × 10。
/// （素材池 4 个已随 T-FIX-356 入住活下坞，见 `check_asset_pool`，不再是死 id。）
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
    assert_eq!(ids.len(), 69, "调音台控件清单长度变了 ⇒ 判据口径要重核");
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

/// ⭐ `#205`：把采样库根固定到**不存在**的路径（= CI 的形态）。
///
/// 本文件测的是"采样库**未安装**"时的装乐器行布局（`#187` 的 `#196` 形态）。
/// 已经下载过库的机器上 `sample-library-download-button` 会按产品行为隐藏 ⇒
/// 同一个判据会读到另一个状态。⇒ 每个窗口构造前先隔离库根。
///
/// ⚠ 用 [`std::sync::Once`]：`set_var` 在 Rust 2024 是 `unsafe`（要求没有并发读者）。
/// 本文件的全部 `ViewState::from_project` 都经 `live_window_for` /
/// `live_window_at_breakpoint` ⇒ 写发生在任何一次读之前，进程内只有这一次写。
fn isolate_uninstalled_library() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let path = std::env::temp_dir().join("yeban-pane-no-overlap-no-library");
        // SAFETY: 唯一一次写，且在所有窗口构造之前（见函数文档）。
        unsafe { std::env::set_var(SAMPLE_LIBRARY_ENV, path) };
    });
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
    isolate_uninstalled_library();
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

/// ⭐ T-FIX-356 契约面判据（死 UI 的诚实形态）—— 死段的四边：
///
/// ① 注册表登记了它（角色＋组件文件逐字，[`ElementRegistry::from_view`]）；
/// ② 声明它的 `.slint` 文件里确实有这句 `accessible-id`（或 `format!` 模板针）；
/// ③ 装配它的父组件在 `ui/app.slint` 里被 `if false` 门住（见
///    [`assert_console_tabs_gated`] / [`assert_ai_rail_gated`]）；
/// ④ 活树里没有它。
///
/// 几何无从量起的段，量"它确实死了、死在哪"—— 删几何不断言是诚实，不是放宽。
fn dead_id_contract(
    registry: &ElementRegistry,
    tree: &ControlTree,
    id: &str,
    kind: ElementKind,
    component: &str,
    label: &str,
) {
    let meta = registry
        .get(id)
        .unwrap_or_else(|| panic!("`{id}` 必须在装配时的注册表里（[UI-TEST-001]）"));
    assert_eq!(meta.kind, kind, "`{id}` 在注册表里的角色必须是 {kind:?}");
    assert_eq!(
        meta.component, component,
        "`{id}` 在注册表里登记的组件文件必须是 {component}"
    );
    assert!(
        !meta.known_absent,
        "`{id}` 必须是声明型登记（`add`），不是 `known_absent` 登记"
    );
    assert_eq!(meta.label, label, "`{id}` 在注册表里的标签必须逐字一致");
    assert!(
        tree.find_by_id(id).is_none(),
        "活树里不许再有 `{id}`（它只活在 `{component}` 里，见 T-FIX-356）"
    );
}

/// 死段的另一族：注册表自己就登记为 `known_absent`（理由存在 `component` 字段里，
/// 见 `elements.rs` 的 `add_known_absent`）—— 活树里同样没有它。
fn known_absent_contract(registry: &ElementRegistry, tree: &ControlTree, id: &str) {
    let meta = registry
        .get(id)
        .unwrap_or_else(|| panic!("`{id}` 必须在装配时的注册表里（[UI-TEST-001]）"));
    assert!(
        meta.known_absent,
        "`{id}` 必须是 `known_absent` 登记（注册表已承认它没有活控件）"
    );
    assert!(
        tree.find_by_id(id).is_none(),
        "活树里不许再有 `{id}`（注册表登记的缺席原因：{}）",
        meta.component
    );
}

/// 与 [`dead_id_contract`] 同款，但不钉标签 —— 用于标签由投影派生的 id
/// （`device-rack-count` 的 `\"N 台设备\"`、`*insert-reason` 等；标签的派生关系由
/// `elements.rs` 自己的覆盖判据钉，本文件只钉角色＋组件文件＋活树缺席）。
fn dead_id_no_label(
    registry: &ElementRegistry,
    tree: &ControlTree,
    id: &str,
    kind: ElementKind,
    component: &str,
) {
    let meta = registry
        .get(id)
        .unwrap_or_else(|| panic!("`{id}` 必须在装配时的注册表里（[UI-TEST-001]）"));
    assert_eq!(meta.kind, kind, "`{id}` 在注册表里的角色必须是 {kind:?}");
    assert_eq!(
        meta.component, component,
        "`{id}` 在注册表里登记的组件文件必须是 {component}"
    );
    assert!(
        !meta.known_absent,
        "`{id}` 必须是声明型登记（`add`），不是 `known_absent` 登记"
    );
    assert!(
        tree.find_by_id(id).is_none(),
        "活树里不许再有 `{id}`（它只活在 `{component}` 里，见 T-FIX-356）"
    );
}

/// 读一个 `.slint` 文件，断言它声明了给定的 `accessible-id`（字面量形态）。
fn dead_slint_declares(path: &str, id: &str) {
    let src = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
        .unwrap_or_else(|_| panic!("读 `{path}`"));
    assert!(
        src.contains(&format!("accessible-id: \"{id}\";")),
        "`{path}` 必须声明 `{id}`"
    );
}

/// 读一个 `.slint` 文件，断言它声明了给定的 `accessible-id` **模板针**
/// （`format!` 生成族在源码里是 `"track-" + track_index + "-fader"` 这类拼接，
/// 字面量 id 不会出现 —— 针就是那段拼接）。
fn dead_slint_declares_template(path: &str, needle: &str) {
    let src = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
        .unwrap_or_else(|_| panic!("读 `{path}`"));
    assert!(src.contains(needle), "`{path}` 必须声明模板针 `{needle}`");
}

/// 旧控制台（`ConsoleTabs` 全家）必须被 `ui/app.slint` 的 `if false` 兼容块门住。
fn assert_console_tabs_gated() {
    let app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 ui/app.slint");
    assert!(
        app.contains("if false : console_tabs := ConsoleTabs {"),
        "旧控制台必须被 `if false` 门住（`ui/app.slint` 兼容块）"
    );
}

/// 旧右栏 `ai-rail` 必须在 `ui/app.slint` 的 `if false` 块里（不是删了声明、
///
/// 而是声明本身被门住 —— 找 `ai-rail` 那句声明往前 300 字符内必须有 `if false`）。
fn assert_ai_rail_gated() {
    let app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 ui/app.slint");
    let pos = app
        .find("accessible-id: \"ai-rail\";")
        .expect("`ui/app.slint` 必须还声明 `ai-rail`（门住的声明，不是删掉的声明）");
    let head = &app[pos.saturating_sub(300)..pos];
    assert!(
        head.contains("if false"),
        "`ai-rail` 的声明必须在 `if false` 块里"
    );
}

/// 从一份投影建注册表（死段判据的登记侧 —— 与活树同一份工程）。
fn registry_for(project: &YebanProjectV1) -> ElementRegistry {
    let view = ViewState::from_project(project).expect("夹具工程必须能投影");
    ElementRegistry::from_view(&view)
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
/// `workspace-arrangement-canvas` 有 `clip: true`，其子节点即便像素被画布裁掉，
/// 包围盒也可能越过画布右沿落进右栏的段盒子。这些像素**⛔ 不与右栏重叠**。
/// 右栏的控件全部从 `ai-rail.x + 8` 起（左沿在盒子内）⇒ 这条取法把"像素真的在段内"
/// 与"包围盒越界"分开。这不是放宽：同一段内左沿在盒子里的控件**仍然逐对**断言
/// （见下面的读数）。
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
    // 标签是 en-US 决议（`LANG=en_US.UTF-8` 下 `@tr` 的英文形态，见 T-FIX-356）。
    assert_eq!(
        tree.find_by_id("piano-roll-status")
            .expect("状态行必须在场")
            .label,
        "Piano Roll Snap & Range Status"
    );
    // 20 个控件（工具 5 ＋ 状态 1 ＋ 播放头 1 ＋ 音域 2 ＋ 音符 5 ＋ 力度条 6）；
    // 音域键 `pitch-range-down/up`（2026-11-24 探针：三档全在，标签
    // "Shift Range Down/Up One Octave"）—— 旧清单漏了它俩，`assert_pairwise`
    // 的计数比对会把它们报成"树里多出"。对数由 `len()` 自证，不手算。
    (
        PIANO_CONTROLS.len(),
        PIANO_CONTROLS.len() * (PIANO_CONTROLS.len() - 1) / 2,
    )
}

/// 调音台（旧 `ConsoleTabs` 的 `MixerConsole`）：**0 个活节点**。
///
/// 四边契约面：69 个 id 全在注册表里（角色＋组件文件逐字）＋ 死文件声明 ＋
/// `ConsoleTabs` 被 `if false` 门住 ＋ 活树缺席。活的替代面是 Tab 1 下控制台的
/// 导轨带：导轨区里有 8 个非容器节点（3 个导轨键 ＋ 主输出单声道钮 ＋ 素材池 4 个——面板落在导轨区内），逐对不相交。
fn check_mixer_console(width: u32, height: u32) -> (usize, usize) {
    let project = yeban_model::samples::demo_project();
    let registry = registry_for(&project);
    let live = live_window_for(project, (width, height), MIXER_TAB);
    assert_eq!(live.ui().get_console_tab(), 1, "夹具必须是 Tab 1（调音台）");
    let tree = runtime_tree(live.ui());
    assert_console_tabs_gated();

    // ---- 四边之一二：注册表登记（角色＋组件文件）＋ 死文件声明。
    // 生成族（`track-{i}-*`）在源码里是 `"track-" + track_index + "-fader"` 这类
    // 模板针 —— 用模板针验声明，不拿字面量 id 去扫源码。
    for id in mixer_controls() {
        let (kind, component) = if id == "mixer-console" {
            (ElementKind::Region, "console/mixer_console.slint")
        } else if id.starts_with("module-") {
            (ElementKind::Button, "console/mixer_console.slint")
        } else if id.starts_with("mixer-master-") {
            let kind = match id.as_str() {
                "mixer-master-strip" => ElementKind::GroupBox,
                "mixer-master-color-swatch" => ElementKind::Image,
                "mixer-master-meter" => ElementKind::ProgressIndicator,
                "mixer-master-fader" => ElementKind::Slider,
                "mixer-master-pan" => ElementKind::Slider,
                _ => ElementKind::Button,
            };
            (kind, "console/mixer_console.slint")
        } else if id.starts_with("track-") && id.ends_with("-channel-strip") {
            (ElementKind::GroupBox, "console/mixer_console.slint")
        } else if id.ends_with("-pan") {
            (ElementKind::Slider, "console/mixer_console.slint")
        } else if id.ends_with("-meter") {
            (
                ElementKind::ProgressIndicator,
                "console/mixer_console.slint",
            )
        } else if id.ends_with("-fader") {
            (ElementKind::Slider, "console/mixer_console.slint")
        } else if id.ends_with("-sampler-path") {
            (ElementKind::TextInput, "console/mixer_console.slint")
        } else if id.ends_with("-mixer-color-swatch") {
            (ElementKind::Image, "console/mixer_console.slint")
        } else {
            (ElementKind::Button, "console/mixer_console.slint")
        };
        // 标签只对字面量 id 逐字钉；生成族的标签带轨道名/绑定态，由 `sfz_ui_selector.rs`
        // 等专项判据钉 —— 这里只钉角色＋组件文件＋活树缺席。
        let meta = registry
            .get(&id)
            .unwrap_or_else(|| panic!("`{id}` 必须在装配时的注册表里（[UI-TEST-001]）"));
        assert_eq!(meta.kind, kind, "`{id}` 在注册表里的角色必须是 {kind:?}");
        assert_eq!(
            meta.component, component,
            "`{id}` 在注册表里登记的组件文件必须是 {component}"
        );
        assert!(
            tree.find_by_id(&id).is_none(),
            "活树里不许再有 `{id}`（它只活在 `{component}` 里，见 T-FIX-356）"
        );
    }
    dead_slint_declares("ui/console/mixer_console.slint", "mixer-console");
    dead_slint_declares(
        "ui/console/mixer_console.slint",
        "module-export-midi-button",
    );
    dead_slint_declares_template(
        "ui/console/mixer_console.slint",
        "\"track-\" + track_index + \"-fader\"",
    );
    dead_slint_declares_template(
        "ui/console/mixer_console.slint",
        "\"track-\" + track_index + \"-sampler-path\"",
    );

    // （混音推子条是无语义 id 的 TouchArea，不进控件树 —— 见报告 §3。）
    // 活的替代面：Tab 1 下控制台导轨区 —— 8 个非容器节点（4 导轨键 ＋ 素材池 4 个），逐对不相交。
    // 素材池面板 (630, 784, 216, 258) 落在导轨区 (0, 736, 1920, 320) 内，故其 4 个子节点也与导轨带相交。
    let rail = bounds(&tree, "console-tab-rail");
    let dock = assert_pairwise(
        &tree,
        rail,
        Selection::Intersect,
        &[
            "console-maximize-button",
            "macro-open-button",
            "plugin-panel-open-button",
            "dock-master-mono",
            "asset-pool-count",
            "asset-pool-import-button",
            "asset-pool-row-0-row",
            "asset-pool-row-1-row",
        ],
        &format!("#186 调音台活面 {width}x{height}"),
    );
    report_line(&dock);
    let mono = bounds(&tree, "dock-master-mono");
    assert_eq!(
        (mono.0, mono.1 - rail.1),
        (863, 94),
        "[#186 {width}x{height}] 主输出单声道钮必须在导轨区内 (863, 导轨顶沿+94)——素材池面板右沿 846，钮在其右 17px"
    );

    let controls = mixer_controls().len();
    report_line(&format!(
        "[#186 调音台 {width}x{height}] {controls} 个旧 id 全是注册表登记＋死文件声明＋`if false` 门住＋活树缺席；活导轨区 8 节点 0 相交"
    ));
    (controls, 0)
}

/// 素材池（⭐ T-FIX-356 起**已入住活下坞**的混音台档，不再是死段）：4 个活节点。
///
/// 活段契约面：注册表登记（角色＋组件文件＋标签逐字，源语言）＋ 面板文件声明 ＋
/// 活树在场（面板＋4 个子节点）＋ 4 个非容器节点在面板内两两不相交。
/// 模型侧：演示工程的 `clip_pool` 仍有 2 条（素材池活的是界面，模型一直有素材）。
/// 运行时标签是 en-US 翻译（`2 Assets`），与注册表源语言标签（`素材 2 条`）不同 ——
/// 见 `arrangement_ruler_seek.rs` 的 i18n 说明；这里只钉注册表标签，不钉运行时标签。
fn check_asset_pool(width: u32, height: u32) -> (usize, usize) {
    let project = yeban_model::samples::demo_project();
    assert_eq!(
        project.clip_pool.len(),
        2,
        "演示工程的 clip_pool 必须是 2 条（素材池活的是界面，模型一直有素材）"
    );
    let view = ViewState::from_project(&project).expect("夹具工程必须能投影");
    let registry = ElementRegistry::from_view(&view);
    let live = live_window_for(project, (width, height), MIXER_TAB);
    let tree = runtime_tree(live.ui());
    assert_console_tabs_gated();
    let count_label = format!("素材 {} 条", view.asset_pool.len());
    for id in ASSET_CONTROLS {
        let (kind, label) = match *id {
            "asset-pool-panel" => (ElementKind::Region, "素材池"),
            "asset-pool-count" => (ElementKind::Text, count_label.as_str()),
            "asset-pool-import-button" => (
                ElementKind::Button,
                "导入音频（用文件对话框挑一个 WAV 并作为片段导入）",
            ),
            _ => (ElementKind::ListItem, ""),
        };
        let meta = registry
            .get(id)
            .unwrap_or_else(|| panic!("`{id}` 必须在装配时的注册表里（[UI-TEST-001]）"));
        assert_eq!(meta.kind, kind, "`{id}` 在注册表里的角色必须是 {kind:?}");
        assert_eq!(
            meta.component, "console/asset_pool_panel.slint",
            "`{id}` 在注册表里登记的组件文件必须是 console/asset_pool_panel.slint"
        );
        if !label.is_empty() {
            assert_eq!(
                meta.label, label,
                "`{id}` 在注册表里的标签必须逐字一致（实测 {:?}）",
                meta.label
            );
        }
        assert!(
            tree.find_by_id(id).is_some(),
            "活树里必须有 `{id}`（T-FIX-356 已入住活下坞混音台档）"
        );
    }
    // 面板本身：注册表登记（Region）＋ 活树在场；声明仍在原文件里（该文件已活，不再是死文件）。
    let panel_meta = registry
        .get("asset-pool-panel")
        .expect("asset-pool-panel 必须在注册表里");
    assert_eq!(
        panel_meta.kind,
        ElementKind::Region,
        "素材池面板在注册表里的角色必须是 Region"
    );
    assert!(
        tree.find_by_id("asset-pool-panel").is_some(),
        "活树里必须有 asset-pool-panel（T-FIX-356 已入住活下坞混音台档）"
    );
    dead_slint_declares("ui/console/asset_pool_panel.slint", "asset-pool-panel");
    // 活段几何：4 个非容器节点全在面板内、两两不相交（探针实测三档皆然；
    // 面板 (630, 784/604/504, 216, 258)，节点相对偏移固定，见合并收据）。
    let panel = bounds(&tree, "asset-pool-panel");
    for id in ASSET_CONTROLS {
        assert_inside(
            &tree,
            id,
            "asset-pool-panel",
            &format!("素材池 {width}x{height}"),
        );
    }
    let pairwise = assert_pairwise(
        &tree,
        panel,
        Selection::Intersect,
        &[
            "asset-pool-count",
            "asset-pool-import-button",
            "asset-pool-row-0-row",
            "asset-pool-row-1-row",
        ],
        &format!("#186 素材池活面 {width}x{height}"),
    );
    report_line(&pairwise);
    report_line(&format!(
        "[#186 素材池 {width}x{height}] 4 个活节点全是注册表登记＋面板文件声明＋活树在场＋面板内 0 相交；模型 clip_pool 仍是 2 条",
    ));
    (ASSET_CONTROLS.len(), 0)
}

/// 走带（活顶栏 `ui/transport_bar.slint`）：带内 6 个常驻 ＋ 窄档溢出钮。
///
/// 旧 `TRANSPORT_WIDE` 的其余 11 个 id 今天只活在注册表 `known_absent` 登记里
/// （见 [`TRANSPORT_DEAD`]）—— 由 [`check_transport_dead_ids`] 在主线程钉一次，
/// 这里只钉活带。
fn check_transport(width: u32, height: u32) -> (usize, usize) {
    let live = live_window((width, height), PIANO_TAB);
    let tree = runtime_tree(live.ui());
    let band: Bounds = (0, TRANSPORT_BAND_Y, width, TRANSPORT_BAND_H);
    let mut expected: Vec<&str> = TRANSPORT_LIVE.to_vec();
    if width < 1920 {
        expected.push(TRANSPORT_OVERFLOW);
    }
    let reading = assert_pairwise(
        &tree,
        band,
        Selection::Inside,
        &expected,
        &format!("#186 走带 {width}x{height}"),
    );
    report_line(&reading);

    // ---- 顺序：停 < 播 < 录 < 循环 < 时间码 < 速度读数（< 溢出钮）—— x 严格递增，
    // 且相邻两键之间至少留 1px（半开区间：边贴边不算交，但这里要求真缝隙）。
    let mut prev_right: Option<i32> = None;
    for id in &expected {
        let b = bounds(&tree, id);
        assert!(
            b.1 >= TRANSPORT_BAND_Y
                && b.1 + b.3 as i32 <= TRANSPORT_BAND_Y + TRANSPORT_BAND_H as i32,
            "[#186 {width}x{height}] `{id}` {b:?} 必须完全落在走带带内"
        );
        if let Some(right) = prev_right {
            assert!(
                b.0 > right,
                "[#186 {width}x{height}] `{id}` 左沿 {} 必须 > 上一键右沿 {right}（走带键横向不许交）",
                b.0
            );
        }
        prev_right = Some(b.0 + b.2 as i32);
    }
    // 1920 档溢出钮按设计不在树里（走带装得下）；1440/1024 档它在树里。
    assert_eq!(
        tree.find_by_id(TRANSPORT_OVERFLOW).is_some(),
        width < 1920,
        "[#186 {width}x{height}] 溢出钮的在场性必须恰好等于 `width < 1920`"
    );
    report_line(&format!(
        "[#186 走带 {width}x{height}] {} 个键横向严格递增、带内 0 相交",
        expected.len()
    ));
    (
        expected.len(),
        expected.len() * expected.len().saturating_sub(1) / 2,
    )
}

/// 死走带 id：注册表 `known_absent` 登记 ＋ 活树缺席（主线程跑，不用开窗口）。
fn check_transport_dead_ids() {
    let registry = registry_for(&yeban_model::samples::demo_project());
    let live = live_window((1440, 900), PIANO_TAB);
    let tree = runtime_tree(live.ui());
    for id in TRANSPORT_DEAD {
        known_absent_contract(&registry, &tree, id);
    }
    report_line(&format!(
        "[#186 走带死 id] {} 个旧走带 id 全是注册表 `known_absent` 登记、活树缺席",
        TRANSPORT_DEAD.len()
    ));
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
    // 死走带 id（注册表 `known_absent` 登记 ＋ 活树缺席）—— 主线程跑，不用开窗口。
    check_transport_dead_ids();
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

/// ⭐ T-FIX-356：旧段的几何清单（`CARD_AREA_*` / `INSTRUMENT_ROW` / `ROUTING_*` /
/// `AI_RAIL_*` / `MASTER_LEAVES_*` / `section_cards_for`）已删 —— 那些段今天
/// 0 个活节点，清单改由各段的契约面判据（注册表登记 ＋ 死文件声明 ＋ `if false`
/// 门住 ＋ 活树缺席）覆盖；清单本身见 `mixer_controls()`（调音台 69 个 id 的生成器）
///
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

/// 分组断言：清单里的 id **一个不少**，且组内**两两**包围盒不相交。
///
/// ⭐ 它与 [`assert_pairwise`] 的分工：`assert_pairwise` 判"段内非容器叶子"，
/// 本函数判**容器与容器**（轨道头）、以及**本来就不该混进叶子的同族控件**
/// （剪辑头 / 自动化泳道）。两边的"控件数 ＝ 清单"都是**硬断言**。
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
/// ① 设备链卡片区（旧 `ConsoleTabs` 的 `DeviceRack`）：**0 个活节点**。
///
/// 四边契约面：卡片区 id 全在注册表里（角色＋组件文件逐字，标签只对字面量钉；
/// 派生标签由 `elements.rs` 自己的覆盖判据钉）＋ 死文件声明 ＋ `ConsoleTabs`
/// 被 `if false` 门住 ＋ 活树缺席。模型侧：夹具的设备数如实读出来（演示工程 1 台、
/// 空链 0 台、只有主总线 0 台）—— 设备死的是界面，不是模型。
/// 活的替代面是 Tab 2 下控制台的导轨带（同调音台的活面，见 [`check_mixer_console`]）。
fn check_device_card_area(
    width: u32,
    height: u32,
    project: YebanProjectV1,
    tag: &str,
    device_count: usize,
) {
    // ---- 模型侧：夹具的设备数（`device_count` 参数必须与工程一致，防夹具漂移）。
    let model_devices: usize = project
        .tracks
        .values()
        .map(|track| track.devices.len())
        .sum();
    assert_eq!(
        model_devices, device_count,
        "[#187 卡片区 {tag} {width}x{height}] 夹具的设备数必须是 {device_count}（实得 {model_devices}）"
    );
    let registry = registry_for(&project);
    let live = live_window_for(project, (width, height), 2);
    assert_eq!(live.ui().get_console_tab(), 2, "夹具必须是 Tab 2（设备架）");
    let tree = runtime_tree(live.ui());
    assert_console_tabs_gated();

    // ---- 四边：注册表登记 ＋ 死文件声明 ＋ 活树缺席。
    for (id, kind, label) in [
        ("device-rack", ElementKind::Region, Some("设备机架")),
        ("device-rack-track-label", ElementKind::Text, None),
        ("device-rack-count", ElementKind::Text, None),
        (
            "device-rack-eq-curve",
            ElementKind::Image,
            Some("4 段 EQ 频响曲线"),
        ),
        (
            "device-insert-eq-button",
            ElementKind::Button,
            Some("装 EQ"),
        ),
        (
            "device-insert-reverb-button",
            ElementKind::Button,
            Some("装混响"),
        ),
        ("device-insert-reason", ElementKind::Text, None),
        (
            "clip-create-button",
            ElementKind::Button,
            Some("新建空的 MIDI 片段"),
        ),
        ("clip-create-reason", ElementKind::Text, None),
        (
            "device-rack-empty",
            ElementKind::Text,
            Some("无设备（点上方「装 EQ」或「装混响」）"),
        ),
        (
            "device-plugin-host-placeholder-card",
            ElementKind::ListItem,
            Some("商业插件宿主卡片 (v2.0.0, 未实现)"),
        ),
    ] {
        match label {
            Some(label) => dead_id_contract(
                &registry,
                &tree,
                id,
                kind,
                "console/device_rack.slint",
                label,
            ),
            None => dead_id_no_label(&registry, &tree, id, kind, "console/device_rack.slint"),
        }
    }
    // 卡片族（`device-{i}-card`）按条目登记：演示工程 1 台 ⇒ 只有 `device-0-card`。
    for index in 0..device_count {
        dead_id_no_label(
            &registry,
            &tree,
            &format!("device-{index}-card"),
            ElementKind::ListItem,
            "console/device_rack.slint",
        );
    }
    dead_slint_declares("ui/console/device_rack.slint", "device-rack");
    dead_slint_declares("ui/console/device_rack.slint", "device-rack-empty");

    // ---- 活的替代面：Tab 2 下控制台导轨区 —— 导轨键逐对不相交。
    // 空链时插件面板是空面板，`ui/lower_dock.slint:686` 的"＋ 插件与 ARA…"开钮
    // `plugin-panel-empty-open-button` 会进树（1920 档实测在场；小档内容区短、
    // 它被裁剪出树 ⇒ 不进两两判据，但只要在场就必须进，防"空面板的键与导轨键重叠"）。
    // 有设备时空面板不存在 ⇒ 它必须全程缺席。
    let rail = bounds(&tree, "console-tab-rail");
    let mut rail_controls: Vec<&str> = vec![
        "console-maximize-button",
        "macro-open-button",
        "plugin-panel-open-button",
    ];
    let empty_plugin_button = tree
        .find_by_id("plugin-panel-empty-open-button")
        .map(bounds_of);
    match (device_count, empty_plugin_button) {
        (0, Some(button)) => {
            assert!(
                intersect(rail, button),
                "[#187 卡片区 {tag} {width}x{height}] 空链的插件开钮在树里却落在导轨区外: {button:?}（导轨区 {rail:?}）"
            );
            rail_controls.push("plugin-panel-empty-open-button");
        }
        (0, None) => {}
        (_, Some(button)) => {
            panic!(
                "[#187 卡片区 {tag} {width}x{height}] 有 {device_count} 台设备时空插件面板的开钮不该在场: {button:?}"
            );
        }
        (_, None) => {}
    }
    let rail_count = rail_controls.len();
    report_line(&assert_pairwise(
        &tree,
        rail,
        Selection::Intersect,
        &rail_controls,
        &format!("#187 设备链活面 {tag} {width}x{height}"),
    ));
    report_line(&format!(
        "[#187 卡片区 {tag} {width}x{height}] 设备链旧 id 全是注册表登记＋死文件声明＋`if false` 门住＋活树缺席；模型设备 {device_count} 台不变；活导轨区 {rail_count} 节点 0 相交"
    ));
}

// ===========================================================================
// ② 装乐器行（`#163`）
// ===========================================================================
/// ② 装乐器行（旧 `ConsoleTabs` 的 `DeviceRack` 乐器行）：**0 个活节点**。
///
/// 四边契约面：`instrument-label` / `sample-library-*` / `instrument-install-*`
/// 全在注册表里 ＋ 死文件声明 ＋ `ConsoleTabs` 被 `if false` 门住 ＋ 活树缺席。
/// 模型侧：内置可装乐器清单为空（`installable_builtin_instruments` 0 条 ——
/// 乐器行死的是界面，不是"有乐器可装却不显示"）。
/// 注意：`isolate_uninstalled_library` 仍把采样库根指到不存在的路径（CI 形态），
/// 所以 `sample-library-download-button` 的注册表标签是"下载采样库"那一支。
fn check_instrument_row(width: u32, height: u32) {
    let project = yeban_model::samples::demo_project();
    let view = ViewState::from_project(&project).expect("演示工程必须能投影");
    assert!(
        view.instrument_names.is_empty(),
        "内置可装乐器清单必须是空的（否则装乐器行该有按钮）"
    );
    let registry = ElementRegistry::from_view(&view);
    let live = live_window((width, height), 2);
    let tree = runtime_tree(live.ui());
    assert_console_tabs_gated();

    for (id, kind, label) in [
        ("instrument-label", ElementKind::Text, None),
        (
            "sample-library-download-button",
            ElementKind::Button,
            Some("下载采样库"),
        ),
        ("sample-library-status", ElementKind::Text, None),
        (
            "instrument-install-status",
            ElementKind::Text,
            Some("装乐器的运行期回执（点击后出现；投影不携带）"),
        ),
    ] {
        match label {
            Some(label) => dead_id_contract(
                &registry,
                &tree,
                id,
                kind,
                "console/device_rack.slint",
                label,
            ),
            None => dead_id_no_label(&registry, &tree, id, kind, "console/device_rack.slint"),
        }
    }
    dead_slint_declares("ui/console/device_rack.slint", "instrument-label");
    report_line(&format!(
        "[#187 装乐器行 {width}x{height}] 乐器行旧 id 全是注册表登记＋死文件声明＋`if false` 门住＋活树缺席；内置清单 0 条"
    ));
}

// ===========================================================================
// ③ 轨路由四键区（`#160`）
// ===========================================================================
/// ③ 轨路由四键区（旧 `ConsoleTabs` 的 `DeviceRack` 轨路由段）：**0 个活节点**。
///
/// 四边契约面：`track-routing-*` / `track-create-*` / `track-delete-*` 全在注册表里
/// ＋ 死文件声明 ＋ `ConsoleTabs` 被 `if false` 门住 ＋ 活树缺席。模型侧：夹具的轨数
/// 如实读出来（演示工程 6 条非主总线轨 ＋ 主总线；只有主总线时 0 条展示轨）。
/// 活的替代面：建轨 / 删轨今天走宿主菜单与快捷键（不在控件树里 —— 见报告 §3）。
fn check_track_routing(width: u32, height: u32, project: YebanProjectV1, tag: &str) {
    let display_tracks = project
        .tracks
        .values()
        .filter(|track| track.kind != TrackKind::Master)
        .count();
    let registry = registry_for(&project);
    let live = live_window_for(project, (width, height), 2);
    let tree = runtime_tree(live.ui());
    assert_console_tabs_gated();

    for (id, kind, label) in [
        ("track-routing-panel", ElementKind::Region, Some("轨与路由")),
        ("track-routing-count", ElementKind::Text, None),
        ("master-bus-label", ElementKind::Text, None),
        (
            "track-create-midi-button",
            ElementKind::Button,
            Some("新建 MIDI 轨"),
        ),
        (
            "track-create-audio-button",
            ElementKind::Button,
            Some("新建音频轨"),
        ),
        (
            "track-create-aux-button",
            ElementKind::Button,
            Some("新建辅助返回轨"),
        ),
        (
            "track-delete-button",
            ElementKind::Button,
            Some("删除展示轨"),
        ),
        ("track-delete-reason", ElementKind::Text, None),
    ] {
        match label {
            Some(label) => dead_id_contract(
                &registry,
                &tree,
                id,
                kind,
                "console/device_rack.slint",
                label,
            ),
            None => dead_id_no_label(&registry, &tree, id, kind, "console/device_rack.slint"),
        }
    }
    dead_slint_declares("ui/console/device_rack.slint", "track-routing-panel");
    report_line(&format!(
        "[#187 轨路由 {tag} {width}x{height}] 轨路由旧 id 全是注册表登记＋死文件声明＋`if false` 门住＋活树缺席；模型展示轨 {display_tracks} 条不变"
    ));
}

// ===========================================================================
// ④ 编曲画布（`workspace-arrangement-canvas` / `clip-*` / `track-*` / 自动化泳道）
// ===========================================================================
/// ④ 编曲画布（活 `ui/arrangement.slint`）：画布右沿 = 窗口右沿（三档都如此 ——
/// 右栏 `ai-rail` 已死，画布不再被它夹住）；轨道头 / 头内子节点 / 剪辑头 / 自动化泳道
/// 分组逐对不相交；1024 档第 6 行包头被画布裁出树（可滚 —— 纵向滚动由
/// `arrangement_vertical_scroll.rs` 钉，这里如实登记"在树外"而不是"不存在"）。
/// 死件：`track-*-color-swatch`（头内没有色标节点）、`section-*-card`、标尺带
/// `arrangement-ruler`、播放头 `arrangement-playhead` —— 注册表登记在死文件
/// `workspace/arrangement_view.slint` 里，活树里没有。
fn check_arrangement(width: u32, height: u32) {
    let live = live_window((width, height), 0);
    let tree = runtime_tree(live.ui());

    let canvas = bounds(&tree, "workspace-arrangement-canvas");
    assert_eq!(
        canvas.0 + canvas.2 as i32,
        width as i32,
        "[#187 编曲画布 {width}x{height}] 画布右沿必须 = 窗口右沿（右栏已死，不再夹住画布）"
    );
    assert_eq!(
        canvas.1 + canvas.3 as i32,
        bounds(&tree, "splitter-workspace-console").1,
        "[#187 编曲画布 {width}x{height}] 画布底沿必须 = 分隔线顶沿"
    );

    // ---- 死件：注册表登记在死文件里，活树里没有；标尺带除外（已随 T-FIX-356 回到活树）。
    let registry = registry_for(&yeban_model::samples::demo_project());
    for id in [
        "arrangement-playhead",
        "section-0-card",
        "track-0-color-swatch",
    ] {
        assert!(
            tree.find_by_id(id).is_none(),
            "[#187 编曲画布 {width}] 活树里不许再有 `{id}`（它只活在死文件里）"
        );
    }
    // 标尺带已随 T-FIX-356 Group B 回到活树（slider，U-07 R2）：在场 ＋ 落在画布内。
    // 角色由 `arrangement_ruler_seek.rs` ⑨ 钉（slider）；这里只钉在场 ＋ 几何。
    assert!(
        tree.find_by_id("arrangement-ruler").is_some(),
        "[#187 编曲画布 {width}] 活树里必须有 arrangement-ruler（T-FIX-356 已恢复）"
    );
    assert_inside(
        &tree,
        "arrangement-ruler",
        "workspace-arrangement-canvas",
        &format!("编曲画布 {width}"),
    );
    assert_eq!(
        registry
            .get("arrangement-ruler")
            .expect("标尺带必须在注册表里")
            .component,
        "workspace/arrangement_view.slint",
        "标尺带在注册表里登记的组件文件必须是死文件 `workspace/arrangement_view.slint`（⚠ 该指针已陈旧：活声明现居 `ui/arrangement.slint`，见合并收据）"
    );

    // ---- ④-1 轨道头（容器组）：两两不相交。1024 档只有 5 行在树里
    // （第 6 行 y=453..507 落在画布底沿 421 之外 —— 被裁出树，不是被删）。
    let visible_headers: Vec<&str> = TRACK_HEADERS
        .iter()
        .copied()
        .filter(|id| tree.find_by_id(id).is_some())
        .collect();
    let expected_visible = if width < 1440 { 5 } else { 6 };
    assert_eq!(
        visible_headers.len(),
        expected_visible,
        "[#187 编曲画布 {width}] 在树里的轨道头必须是 {expected_visible} 个（1024 档第 6 行被画布裁出树）"
    );
    assert_eq!(
        tree.find_by_id("track-5-header").is_some(),
        width >= 1440,
        "[#187 编曲画布 {width}] `track-5-header` 的在场性必须恰好等于 `width >= 1440`"
    );
    let label = format!("编曲画布 轨道头 {width}x{height}");
    report_line(&assert_ids_pairwise(&tree, &visible_headers, &label));

    // ---- ④-2 头内子节点（叶子组）：两两不相交 ＋ 各自落在自己的头里。
    // 头内只有静音 / 独奏 / 录音臂三个键 —— `track-*-color-swatch` 已死（头内没有色标节点）。
    // 1024 档第 5 行（track-4）只有头在树里、三个键被画布底沿裁出树
    // （头 y=397..451 与画布底沿 421 相交，键 y=425..445 全在底沿之外 —— B 的顶栏修整
    // 让轨道整体下移 16px，修前键 y=409..429 与底沿相交故在树里）——
    // 子节点清单按在场过滤，并钉死每档的条数（1920/1440 档 18 条，1024 档 12 条）。
    let mut child_ids: Vec<String> = Vec::new();
    for track in 0..visible_headers.len() {
        for suffix in ["mute-button", "solo-button", "arm-button"] {
            let id = format!("track-{track}-{suffix}");
            if tree.find_by_id(&id).is_some() {
                child_ids.push(id);
            }
        }
    }
    let expected_children = if width < 1440 { 12 } else { 18 };
    assert_eq!(
        child_ids.len(),
        expected_children,
        "[#187 编曲画布 {width}] 在树里的轨道头子节点必须是 {expected_children} 个（1024 档 track-4 的三个键被画布底沿裁出树）"
    );
    let child_refs: Vec<&str> = child_ids.iter().map(String::as_str).collect();
    let label = format!("编曲画布 轨道头子节点 {width}x{height}");
    report_line(&assert_ids_pairwise(&tree, &child_refs, &label));
    for track in 0..visible_headers.len() {
        let header = format!("track-{track}-header");
        for suffix in ["mute-button", "solo-button", "arm-button"] {
            let child = format!("track-{track}-{suffix}");
            if tree.find_by_id(&child).is_none() {
                continue; // 1024 档 track-4 的键被画布底沿裁出树（条数已在上文钉死）
            }
            assert_inside(
                &tree,
                &child,
                &header,
                &format!("编曲画布 {width}x{height}"),
            );
        }
    }

    // ---- ④-3 剪辑头（`list-item` 叶子）/ ④-4 自动化泳道。
    // （章节卡 `section-*-card` 已死 —— 段落带是画布内的非寻址绘制，见 `section_color_swatch.rs`。）
    let label = format!("编曲画布 剪辑头 {width}x{height}");
    report_line(&assert_ids_pairwise(&tree, CLIP_HEADERS, &label));
    let label = format!("编曲画布 自动化泳道 {width}x{height}");
    report_line(&assert_ids_pairwise(&tree, AUTOMATION_LANES, &label));
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
/// ⑥ AI 右栏（`ai-rail` 全家 ＋ `module_actions.slint` 模块入口）：**0 个活节点**。
///
/// 四边契约面：`ai-rail*` 在注册表里（组件文件 `app.slint`）＋ `ui/app.slint` 里
/// 的声明被 `if false` 门住 ＋ 活树缺席；模块入口在注册表里（组件文件
/// `module_actions.slint`）＋ 死文件声明 ＋ 该文件没被任何活 `.slint` import ＋
/// 活树缺席。活的替代面：死按钮 `ai-rail-musical-pr-button` 的回调
/// `open-musical-pr()` —— 调它必须打开真的 `MusicalPrDrawer`（动作没死，只是
/// 按钮死了）。
fn check_ai_rail(width: u32, height: u32) {
    let project = yeban_model::samples::demo_project();
    let registry = registry_for(&project);
    let live = live_window((width, height), 0);
    let tree = runtime_tree(live.ui());
    assert_ai_rail_gated();

    for (id, kind, label) in [
        (
            "ai-rail",
            ElementKind::Complementary,
            "AI 协作栏: 意图生成与声学诊断",
        ),
        (
            "ai-rail-intent-button",
            ElementKind::Button,
            "用自然语言描述编曲意图",
        ),
        ("ai-rail-diagnose-button", ElementKind::Button, "声学诊断"),
        (
            "diagnostics-export-action",
            ElementKind::Button,
            "导出诊断包 (D56)",
        ),
        (
            "ai-rail-musical-pr-button",
            ElementKind::Button,
            "打开 AI 编曲提案审核抽屉",
        ),
    ] {
        dead_id_contract(&registry, &tree, id, kind, "app.slint", label);
    }
    for (id, kind, label) in [
        (
            "module-actions-title",
            ElementKind::Text,
            "模块入口：导出 WAV / 导入 MIDI / 装载 SFZ / 录音 / 硬件 MIDI 输入",
        ),
        (
            "module-export-wav-button",
            ElementKind::Button,
            "导出 WAV（打开采样率 / 位深 / 时长选项）",
        ),
        (
            "module-import-midi-button",
            ElementKind::Button,
            "导入 MIDI（把 SMF 的音符导入第一条非主总线音轨）",
        ),
        (
            "module-load-sfz-button",
            ElementKind::Button,
            "装载 SFZ 乐器（报告区域数并探测一个音符）",
        ),
        (
            "module-record-wav-button",
            ElementKind::Button,
            "录音（把控制侧测试音录成 24-bit WAV）",
        ),
        (
            "module-midi-in-button",
            ElementKind::Button,
            "硬件 MIDI 输入（采集物理端口事件，并把配对出的音符写进第一条非主总线音轨）",
        ),
        (
            "module-midi-in-status",
            ElementKind::Text,
            "最近一次硬件 MIDI 输入的采集结果（端口 / 事件数 / 不支持数 / 丢弃事件数 / 抖动 / 错误码）",
        ),
    ] {
        dead_id_contract(&registry, &tree, id, kind, "module_actions.slint", label);
    }
    dead_id_no_label(
        &registry,
        &tree,
        "module-last-status",
        ElementKind::Text,
        "module_actions.slint",
    );
    dead_slint_declares("ui/module_actions.slint", "module-export-wav-button");
    // `module_actions.slint` 是死文件：没有任何活 `.slint` import 它。
    for live_slint in [
        "ui/app.slint",
        "ui/arrangement.slint",
        "ui/transport_bar.slint",
    ] {
        let src = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(live_slint),
        )
        .unwrap_or_else(|_| panic!("读 `{live_slint}`"));
        assert!(
            !src.contains("module_actions"),
            "`{live_slint}` 不许 import 死文件 `module_actions.slint`"
        );
    }

    // ---- 活的替代面：死按钮的回调 `open-musical-pr()` 仍能打开真的提案抽屉。
    // `live_window_for` 只调 `build_main_window_with_console_tab`，不装
    // `wire_proposal_drawer` —— 而 `open-musical-pr` 的**唯一**注册者在
    // `wire_proposal_drawer`（`host.rs:7984`，`wire_view_callbacks` 里明确排除），
    // 不装它直调回调就是空操作（2026-11-24 实测：抽屉打不开）。先装真的接线
    // （`ProposalSource::none()`：零提案，与探针的空抽屉夹具一致），再调回调。
    assert!(
        tree.find_by_id("musical-pr-drawer").is_none(),
        "[#187 AI 右栏 {width}] 默认态提案抽屉必须不在树里"
    );
    host::wire_proposal_drawer(live.ui(), yeban_app::proposals::ProposalSource::none());
    live.ui().invoke_open_musical_pr();
    let drawer_tree = runtime_tree(live.ui());
    assert!(
        drawer_tree.find_by_id("musical-pr-drawer").is_some(),
        "[#187 AI 右栏 {width}] 调 `open-musical-pr()`（死按钮的回调）必须打开真的提案抽屉"
    );
    report_line(&format!(
        "[#187 AI 右栏 {width}x{height}] 右栏旧 id 全是注册表登记＋死文件声明＋`if false` 门住＋活树缺席；`open-musical-pr()` 仍能打开真的抽屉"
    ));
}

// ===========================================================================
// 判据 ⑤（设备链卡片区）：三档 ＋ 三种夹具
// ===========================================================================
#[test]
fn the_device_card_chain_has_no_two_controls_in_the_same_place() {
    for &(width, height) in TIERS {
        // (a) 演示工程：1 台设备（模型侧）；界面侧全死（见 `check_device_card_area`）。
        if let Err(payload) = std::thread::spawn(move || {
            check_device_card_area(
                width,
                height,
                yeban_model::samples::demo_project(),
                "演示",
                1,
            );
        })
        .join()
        {
            std::panic::resume_unwind(payload);
        }
        // (b) 空设备链（0 台）。
        if let Err(payload) = std::thread::spawn(move || {
            check_device_card_area(width, height, project_without_devices(), "空链", 0);
        })
        .join()
        {
            std::panic::resume_unwind(payload);
        }
        // (c) 只有主总线（0 台）。
        if let Err(payload) = std::thread::spawn(move || {
            check_device_card_area(width, height, project_master_only(), "只有主总线", 0);
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
            check_track_routing(width, height, yeban_model::samples::demo_project(), "演示");
        })
        .join()
        {
            std::panic::resume_unwind(payload);
        }
        if let Err(payload) = std::thread::spawn(move || {
            check_track_routing(width, height, project_master_only(), "只有主总线");
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
/// 控制台导轨带顶 28px 里的三个键（活树实测，三档 × 三个页签都有）。
/// 旧 `tab-*-button` 已死：导轨上没有页签按钮（页签切换走 `console-tab` 属性），
/// 注册表里 `tab-{name}-button` 仍登记在 `console_tabs.slint`（由 `elements.rs`
/// 自己的覆盖判据钉，本文件只钉活树里没有它们）。
const CONSOLE_RAIL: &[&str] = &[
    "console-maximize-button",
    "macro-open-button",
    "plugin-panel-open-button",
];

/// 歌曲总览三键（导轨顶沿 − 30px 的那条 22px 带里）。
const SONG_OVERVIEW_STRIP: &[&str] = &[
    "song-overview-capture-a-button",
    "song-overview-capture-b-button",
    "song-overview-compare-button",
];

/// ⭐ T-FIX-218：旧左栏清单生成器 `sidebar_controls` 已删 —— 它按已删的
/// `ui/sidebar.slint`（`sidebar-collapse-button` / `sidebar-search-field` /
/// `sidebar-category-{i}-button` / `sidebar-empty` / `sidebar-item-{i}`）
/// 生成可见控件清单，无调用者（活窗口版左栏判据 `check_sidebar` 一并删除；
/// 左栏判据现为 `the_sidebar_has_no_two_controls_in_the_same_place` 的
/// 契约面字符串判据，见该测试头注）。
///
/// ③ Session 视图的控件清单（⭐ `#190` 起**按画布装得下的列数**生成）：
/// `visible` 条轨道包头 ＋ 每列 `scenes` 个插槽 ＋ `scenes` 个场景色标 ＋ 场景列头
/// ＋ 返回编曲键 ＋（`visible < total` 时）截断报数 ＋ ⭐ `#283` 的场景启动回执
/// （`session-scene-launch-status`：恒在场、空串时不画字，但**是**一个非容器节点
/// ⇒ 本清单必须登记它，否则"段内控件数 ≠ 清单"会红）。
///
/// ⭐ 场景卡 `scene-launch-{i}-button` **不在**叶子清单里：它按设计包住自己的
/// 12×12 色标 `scene-{i}-color-swatch`（与 `#187` 的设备卡片同款）⇒ 进
/// [`scene_card_buttons`] 的容器组，由 [`assert_scene_cards`] 判"卡彼此不相交 ＋
/// 色标落在自己卡内"。
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
        ids.push(format!("scene-{scene}-color-swatch"));
    }
    ids.push("scene-launch-column-header".to_owned());
    ids.push("session-back-to-arrangement-button".to_owned());
    // ⭐ `#283`：场景启动的如实回执（不在 `#190` 的截断清单里，但恒在场）。
    ids.push("session-scene-launch-status".to_owned());
    if visible < total {
        ids.push("session-track-truncation-note".to_owned());
    }
    ids
}

/// ③ 场景卡（`button` 角色，按设计包住自己的色标）—— 容器组清单。
fn scene_card_buttons(scenes: usize) -> Vec<String> {
    (0..scenes)
        .map(|scene| format!("scene-launch-{scene}-button"))
        .collect()
}

/// ③ 场景卡容器组的正向证明：卡片**彼此**不相交，且每张卡的色标整个落在卡内
/// （排除出叶子集 ≠ 不检查）。
fn assert_scene_cards(tree: &ControlTree, scenes: usize, label: &str) {
    let cards = scene_card_buttons(scenes);
    let card_refs: Vec<&str> = cards.iter().map(String::as_str).collect();
    report_line(&assert_ids_pairwise(
        tree,
        &card_refs,
        &format!("场景卡组 {label}"),
    ));
    for scene in 0..scenes {
        assert_inside(
            tree,
            &format!("scene-{scene}-color-swatch"),
            &format!("scene-launch-{scene}-button"),
            label,
        );
    }
}

/// ③ 画布装得下的轨道列数 —— 与 `session_view.slint` 的 `visible-track-count` **同一条
/// 算式**（场景列左沿 `8 + 128N` ＋ 宽 `104` ⇒ `N = (画布宽 − 112) / 128`，向下取整）。
fn session_track_capacity(canvas_width: u32) -> usize {
    (canvas_width.saturating_sub(112) / 128) as usize
}

/// ③ 画布装得下的**场景行数** —— 与 `session_view.slint` 的 `visible-scene-count`
/// **同一条算式**（`T-FIX-226`）：首行 32 ＋ 每场景 68 ＋ 底带 86 ⇒
/// `n = floor((画布高 − 118) / 68)`，至少 1。
fn session_scene_capacity(canvas_height: u32) -> usize {
    (canvas_height.saturating_sub(118) / 68).max(1) as usize
}

/// ⑤ `UndoTreeModal` 6 个真实节点（`node-hidden: 2`）时的 10 个控件。
/// ⭐ [PATCH UI-REDO-01]（2026-11-09）：新增 `undo-tree-redo-button`（与撤销按钮对称），
/// 段内控件数随之 9 → 10；清单必须同步扩，否则 `assert_pairwise_owned` 的
/// "段内控件数 == 清单数" 会判红。按钮落在撤销按钮右侧、同一 y，`space-3` 留缝 ⇒ 不相交。
fn undo_tree_controls() -> Vec<String> {
    let mut ids = vec!["undo-tree-close-button".to_owned()];
    for index in 0..6 {
        ids.push(format!("undo-tree-node-{index}"));
    }
    ids.push("undo-tree-truncation-note".to_owned());
    ids.push("undo-tree-undo-button".to_owned());
    ids.push("undo-tree-redo-button".to_owned());
    ids
}

/// ⑤ 空提交链（`node-count: 0`）时的 4 个控件。
/// ⭐ [PATCH UI-REDO-01]：重做按钮与 `can-redo` 无关，任何节点数下都常驻 ⇒ 同步 +1。
const UNDO_TREE_EMPTY: &[&str] = &[
    "undo-tree-close-button",
    "undo-tree-empty-state",
    "undo-tree-undo-button",
    "undo-tree-redo-button",
];

/// ⑥ 零提案抽屉的 3 个控件（图例 / 空态 / 关闭键）。`accept` / `reject` 零提案时按设计不画。
const MUSICAL_PR_EMPTY: &[&str] = &[
    "musical-pr-close-button",
    "musical-pr-diff-legend",
    "musical-pr-empty-state",
];

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
    isolate_uninstalled_library();
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
            plugin: None,
        },
        DeviceDefinition {
            id: EntityId::new(),
            name: "主总线混响".to_owned(),
            kind: DeviceKind::InternalEffect,
            bypassed: true,
            params: vec![],
            latency_samples: 0,
            plugin: None,
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
            plugin: None,
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

// ===========================================================================
// ⭐ T-FIX-218：旧左栏活窗口判据 `check_sidebar` 已删 —— 它在已删的
// `ui/sidebar.slint` 树里找 `sidebar` / `sidebar-collapse-button` /
// `sidebar-search-field` / `sidebar-category-0-button`，无调用者（活窗口版
// 左栏判据随 `Sidebar` 组件一起退役；左栏现为 `left_panel.slint` 浏览器页，
// 契约面判据见 `the_sidebar_has_no_two_controls_in_the_same_place`）。
// `live_window_at_breakpoint` 仍被 Session 视图判据使用，保留。

// ===========================================================================
// ② 控制台标签导轨带 ＋ splitter 带
// ===========================================================================
/// ② 控制台导轨带 ＋ splitter 带（活 `ui/lower_dock.slint` 的 `UnifiedLowerDock`）：
/// 导轨顶 28px 带里只有 3 个键；`piano-roll` 从导轨顶沿 + 42px 起；分隔线 5px、
/// 在导轨顶沿 − 35px；歌曲总览三键在导轨顶沿 − 30px 的 22px 带里。
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

    // 带高 28 是设计值：导轨键底沿（导轨顶沿 + 4 + 24）恰好贴着 28px 带的下沿。
    let tab_bottom = CONSOLE_RAIL
        .iter()
        .map(|id| {
            let b = bounds(&tree, id);
            b.1 + b.3 as i32
        })
        .max()
        .expect("清单非空");
    assert_eq!(
        tab_bottom,
        rail.1 + 28,
        "[#189 导轨 {width}x{height}] 导轨控件底沿 {tab_bottom} 必须恰好贴住 28px 带的下沿 {}",
        rail.1 + 28
    );
    // Tab 内容（钢琴卷帘）从导轨顶沿 + 42px 起（28px 带 ＋ 14px 间隙）。
    let content = bounds(&tree, "piano-roll");
    assert_eq!(
        content.1,
        rail.1 + 42,
        "[#189 导轨 {width}x{height}] Tab 内容必须从导轨顶沿 + 42px 开始"
    );

    // ---- ⑦ `splitter-workspace-console`：5px 带，顶沿在导轨顶沿 − 35px；
    // 带里**只有它自己**。旧断言（4px、底沿贴住导轨顶沿）是旧布局的数。
    let splitter = bounds(&tree, "splitter-workspace-console");
    assert_eq!(
        (splitter.2, splitter.3),
        (width, 5),
        "[#189 splitter {width}x{height}] 分隔线必须是整窗宽 × 5px"
    );
    assert_eq!(
        splitter.1,
        rail.1 - 35,
        "[#189 splitter {width}x{height}] 分隔线顶沿必须在导轨顶沿 − 35px"
    );
    report_line(&assert_pairwise(
        &tree,
        splitter,
        Selection::Inside,
        &["splitter-workspace-console"],
        &format!("#189 分隔线带 {width}x{height}"),
    ));

    // ---- 歌曲总览三键：导轨顶沿 − 30px 起、22px 高，彼此不相交。
    for id in SONG_OVERVIEW_STRIP {
        let b = bounds(&tree, id);
        assert_eq!(
            (b.1, b.3),
            (rail.1 - 30, 22),
            "[#189 总览条 {width}x{height}] `{id}` 必须是 y=导轨顶沿−30、高 22px"
        );
    }
    report_line(&assert_ids_pairwise(
        &tree,
        SONG_OVERVIEW_STRIP,
        &format!("歌曲总览三键 {width}x{height}"),
    ));

    // ---- 死页签键：注册表仍登记，活树里没有。
    for id in [
        "tab-piano-roll-button",
        "tab-mixer-button",
        "tab-devices-button",
    ] {
        assert!(
            tree.find_by_id(id).is_none(),
            "[#189 导轨 {width}x{height}] 活树里不许再有 `{id}`（导轨上没有页签按钮）"
        );
    }
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
    // ⭐ T-FIX-228（T-FIX-226 复验否决）：本判据把截断报数的期望文案写死成中文 ⇒ 必须
    //   **先钉语言**。`build_main_window_with_console_tab` 不走启动路径的 `apply_language`，
    //   Slint 会按系统 locale 自动选词表（英文系统上首帧就是英文，见 host.rs 的
    //   `apply_language` 文档）⇒ 同一棵树、同一份断言在 `LANG=en_US.UTF-8` 下会读到
    //   `"1 more tracks and 2 more scenes not shown"`。产品代码不动，只把测试前提钉死。
    host::apply_language(live.ui(), yeban_app::cli::Language::ZhCn);
    live.ui().set_arrangement_view(false);
    let tree = runtime_tree(live.ui());
    let pane = bounds(&tree, "workspace-session-canvas");

    let compact = live.ui().get_compact();
    // ⭐ `#190`：清单按**画布装得下的列数**生成（演示工程 6 轨、4 个场景）。
    // ⭐ `T-FIX-226`：纵向同理 —— 画布高不够时只画装得下的场景（1024×700 ⇒ 2 行），
    //   清单与截断报数都按**实际画出**的场景数算。
    let capacity = session_track_capacity(pane.2);
    let total = 6;
    let visible = capacity.min(total);
    let scene_total = 4;
    let scene_capacity = session_scene_capacity(pane.3);
    let scenes = scene_capacity.min(scene_total);
    let expected = session_controls(visible, total, scenes);
    let expected_refs: Vec<&str> = expected.iter().map(String::as_str).collect();
    // 场景卡按设计包住自己的色标 ⇒ 从叶子集里摘掉，进下面的容器组。
    let excluded = scene_card_buttons(scenes);
    let label = format!(
        "#190 Session {width}x{height} breakpoint={breakpoint} compact={compact} \
         画布 {}x{}px ⇒ 容量 {capacity} 列 / 显示 {visible} 列；场景容量 {scene_capacity} 行 / \
         显示 {scenes} 行",
        pane.2, pane.3
    );
    report_line(&assert_pairwise_excluding(
        &tree,
        pane,
        Selection::Intersect,
        &expected_refs,
        &excluded,
        &label,
    ));
    assert_scene_cards(&tree, scenes, &label);

    // ---- ⭐ T-FIX-260：场景启动回执必须占满矩阵下方整行，右沿落在根右沿 - 8px。
    //   修前 `x: 8px + 128px * visible-track-count` ＋ 固定 320px ⇒ 三档分别只剩
    //   约 112 / 144 / 112px 可见，长原因被 elide 截成十来个字符。
    let launch_status = bounds(&tree, "session-scene-launch-status");
    assert_eq!(
        launch_status.0,
        pane.0 + 8,
        "[T-FIX-260 Session {width}x{height}] 回执左沿必须贴根左沿 + 8px"
    );
    assert_eq!(
        launch_status.2,
        pane.2 - 16,
        "[T-FIX-260 Session {width}x{height}] 回执宽必须是根宽 - 16px"
    );
    assert!(
        launch_status.0 + launch_status.2 as i32 <= pane.0 + pane.2 as i32 - 8,
        "[T-FIX-260 Session {width}x{height}] 回执右沿 {} 必须 ≤ 根右沿 {} - 8px",
        launch_status.0 + launch_status.2 as i32,
        pane.0 + pane.2 as i32
    );
    report_line(&format!(
        "[T-FIX-260 Session {width}x{height}] 场景回执 (x={} y={} w={} h={}) 右沿 {} ≤ 根右沿 {} - 8px",
        launch_status.0,
        launch_status.1,
        launch_status.2,
        launch_status.3,
        launch_status.0 + launch_status.2 as i32,
        pane.0 + pane.2 as i32
    ));

    // ---- ⭐ T-FIX-226 验收②：底带三件必须在运行时树里。
    //   修前 1024×700 下它们落在相对 y ≥ 312（画布高 272）⇒ 被根 `clip` 整块裁掉，
    //   最小窗口里没有可见的"返回编曲"入口、两条响亮登记也看不见。
    for id in [
        "session-back-to-arrangement-button",
        "session-scene-launch-status",
    ] {
        assert!(
            tree.find_by_id(id).is_some(),
            "[T-FIX-226 Session {width}x{height}] 底带 `{id}` 必须在运行时树里"
        );
    }
    // 装不下的场景行**不画**（不是"画出去被裁一半"）：热区跟着消失，且截断报数说出数量。
    for hidden in scenes..scene_total {
        assert!(
            tree.find_by_id(&format!("scene-launch-{hidden}-button"))
                .is_none(),
            "[T-FIX-226 Session {width}x{height}] 第 {hidden} 场景行装不下 ⇒ 不许半截留在树里"
        );
    }

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

    // ---- 判据 ②：截断报数（横向轨道 ＋ `T-FIX-226` 的纵向场景）必须逐字、且不许喊狼来了。
    let tracks_left = total - visible;
    let scenes_left = scene_total - scenes;
    let expected_note = match (tracks_left > 0, scenes_left > 0) {
        (true, true) => Some(format!(
            "还有 {tracks_left} 轨未显示 · 还有 {scenes_left} 个场景未显示"
        )),
        (true, false) => Some(format!("还有 {tracks_left} 轨未显示")),
        (false, true) => Some(format!("还有 {scenes_left} 个场景未显示")),
        (false, false) => None,
    };
    match expected_note {
        Some(expected_text) => {
            let note = tree
                .find_by_id("session-track-truncation-note")
                .expect("有装不下的轨道/场景 ⇒ 必须有截断报数");
            assert_eq!(
                note.label, expected_text,
                "[#190/T-FIX-226 Session {width}x{height}] 截断报数文案必须逐字"
            );
            report_line(&format!(
                "[#190 Session {width}x{height}] 画布 {}x{}px / 容量 {capacity} 列 / 显示 {visible} 列 / \
                 场景 {scene_capacity} 行 / 显示 {scenes} 行 / 场景列 (x={} y={} w={} h={}) 右沿 {} ≤ \
                 画布右沿 {}；截断报数 {:?}",
                pane.2,
                pane.3,
                scene_header.0,
                scene_header.1,
                scene_header.2,
                scene_header.3,
                scene_header.0 + scene_header.2 as i32,
                pane.0 + pane.2 as i32,
                note.label
            ));
        }
        None => {
            assert!(
                tree.find_by_id("session-track-truncation-note").is_none(),
                "[#190/T-FIX-226 Session {width}x{height}] 全都装得下 ⇒ 截断报数不许在场"
            );
            report_line(&format!(
                "[#190 Session {width}x{height}] 画布 {}x{}px / 全装得下 ⇒ 无截断提示",
                pane.2, pane.3
            ));
        }
    }

    // ---- 折叠档不变量（⭐ `T-FIX-226` 修正口径）
    //
    // 旧断言是 `画布宽 == 窗口宽 − 36` —— 那是**侧栏折叠成 36px 图标导轨**那一版的算式。
    // 活窗口实测（1024×700）：画布 `(261, 89, 763, 272)` ⇒ 左沿 261（240 侧栏 ＋ 缝隙）、
    // 右沿 1024 正好贴窗口右沿。旧算式与现行布局不符，且这条断言此前从未被执行过
    // （`check_session_view` 零调用）。改成不依赖某一版侧栏宽的不变量。
    if breakpoint && compact {
        assert_eq!(
            pane.0 + pane.2 as i32,
            width as i32,
            "[#189/T-FIX-226 Session {width}x{height}] 紧凑档画布右沿必须贴窗口右沿"
        );
        assert!(
            pane.0 > 0,
            "[#189/T-FIX-226 Session {width}x{height}] 画布左沿必须在侧栏之后（实得 {}）",
            pane.0
        );
    }
}

/// ③ ⭐ `#190` 的 7 轨夹具：第 7 轨在 1440 档（画布 920 ⇒ 容量 6）起必须被**截断报数**，
/// 而 1920 档（画布 1400 ⇒ 容量 10）全装得下、⛔ 不许出现提示。
fn check_session_track_truncation(width: u32, height: u32) {
    let live = live_window_for(project_with_seven_tracks(), (width, height), 0);
    // ⭐ T-FIX-228 同款：截断报语文案是 `@tr` 翻译串 ⇒ 先钉语言（否则 en_US 下
    // 读到英文，中文逐字断言会红）。与 `check_session_view` 同一个前提。
    host::apply_language(live.ui(), yeban_app::cli::Language::ZhCn);
    live.ui().set_arrangement_view(false);
    let tree = runtime_tree(live.ui());
    let pane = bounds(&tree, "workspace-session-canvas");

    let capacity = session_track_capacity(pane.2);
    let total = 7;
    let visible = capacity.min(total);
    // ⭐ T-FIX-226 同款：场景行数也按画布高算 —— 1024×800 档画布 332px 高 ⇒
    // 只画 3 行（旧代码写死 4 行，`slot-0-3-cell` 根本不在树里）。
    let scene_total = 4;
    let scene_capacity = session_scene_capacity(pane.3);
    let scenes = scene_capacity.min(scene_total);
    let label = format!("#190 Session 7 轨 {width}x{height}");

    // ---- 判据 ①：7 轨只画装得下的前 N 列；第 N+1 轨不在树里。
    let expected = session_controls(visible, total, scenes);
    let expected_refs: Vec<&str> = expected.iter().map(String::as_str).collect();
    let excluded = scene_card_buttons(scenes);
    report_line(&assert_pairwise_excluding(
        &tree,
        pane,
        Selection::Intersect,
        &expected_refs,
        &excluded,
        &format!(
            "{label} 画布 {}px ⇒ 容量 {capacity} 列 / 显示 {visible} 列 / 隐藏 {} 列",
            pane.2,
            total - visible
        ),
    ));
    assert_scene_cards(&tree, scenes, &label);
    assert!(
        tree.find_by_id(&format!("session-track-{visible}-header"))
            .is_none(),
        "[{label}] 容量 {capacity} 列 ⇒ `session-track-{visible}-header` 不许在树里"
    );
    // 装不下的场景行不画：热区跟着消失（与 `check_session_view` 同款）。
    for hidden in scenes..scene_total {
        assert!(
            tree.find_by_id(&format!("scene-launch-{hidden}-button"))
                .is_none(),
            "[{label}] 第 {hidden} 场景行装不下 ⇒ 不许半截留在树里"
        );
    }

    // ---- 判据 ②：截断报数**逐字** + 有 `accessible-id`；数字 = 7 − 显示列数
    // （＋场景维度的余数 —— 与 `check_session_view` 同一条文案算式）。
    if visible < total || scenes < scene_total {
        let note = tree
            .find_by_id("session-track-truncation-note")
            .expect("7 轨 > 容量或场景装不下 ⇒ 必须有截断报数");
        let tracks_left = total - visible;
        let scenes_left = scene_total - scenes;
        let expected_text = match (tracks_left > 0, scenes_left > 0) {
            (true, true) => {
                format!("还有 {tracks_left} 轨未显示 · 还有 {scenes_left} 个场景未显示")
            }
            (true, false) => format!("还有 {tracks_left} 轨未显示"),
            (false, true) => format!("还有 {scenes_left} 个场景未显示"),
            (false, false) => unreachable!("进到这里必有一维装不下"),
        };
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
/// ④ 主总线设备链段内（旧 `ConsoleTabs` 的 `DeviceRack` 第二段）：**0 个活节点**。
///
/// 四边契约面：主总线段 id 全在注册表里（角色＋组件文件逐字，标签只对字面量钉；
/// `rack-count` 的 `"N 台设备"` 等派生标签由 `elements.rs` 自己的覆盖判据钉）
/// ＋ 死文件声明 ＋ `ConsoleTabs` 被 `if false` 门住 ＋ 活树缺席。
/// 模型侧：夹具的主总线设备数如实读出来（演示 0 台 / 无主总线 0 台 / 2 台），且宿主
/// 仍把设备名注入主窗口（`get_master_device_names()` 的行数 = 设备数 —— 注入链
/// 没死，死的是渲染它的段）。
fn check_master_device_segment(
    width: u32,
    height: u32,
    project: YebanProjectV1,
    tag: &str,
    device_count: usize,
) {
    // ---- 模型侧：主总线设备数（`device_count` 参数必须与工程一致，防夹具漂移）。
    let model_devices = project
        .tracks
        .get(&project.master_bus_track_id)
        .map(|track| track.devices.len())
        .unwrap_or(0);
    assert_eq!(
        model_devices, device_count,
        "[#190 主总线第二段 {tag} {width}x{height}] 夹具的主总线设备数必须是 {device_count}（实得 {model_devices}）"
    );
    let registry = registry_for(&project);
    let live = live_window_for(project, (width, height), 2);
    assert_eq!(live.ui().get_console_tab(), 2, "夹具必须是 Tab 2（设备架）");
    let tree = runtime_tree(live.ui());
    assert_console_tabs_gated();

    // ---- 四边：注册表登记 ＋ 死文件声明 ＋ 活树缺席。
    for (id, kind, label) in [
        (
            "master-device-rack",
            ElementKind::Region,
            Some("主总线设备链"),
        ),
        ("master-device-track-label", ElementKind::Text, None),
        ("master-device-rack-count", ElementKind::Text, None),
        (
            "master-device-insert-eq-button",
            ElementKind::Button,
            Some("给主总线装 EQ"),
        ),
        (
            "master-device-insert-reverb-button",
            ElementKind::Button,
            Some("给主总线装混响"),
        ),
        ("master-device-eq-reason", ElementKind::Text, None),
        ("master-device-insert-reason", ElementKind::Text, None),
        (
            "master-device-rack-empty",
            ElementKind::Text,
            Some("无设备（点上方「装 EQ」或「装混响」）"),
        ),
        (
            "master-device-eq-panel",
            ElementKind::GroupBox,
            Some("主总线 EQ 低架增益 (eq_low_gain)"),
        ),
        ("master-device-eq-value", ElementKind::Text, None),
        (
            "master-device-eq-slider",
            ElementKind::Slider,
            Some("主总线 EQ 低架增益"),
        ),
        (
            "master-device-truncation-note",
            ElementKind::Text,
            Some("主总线设备链截断报数（还有几台未显示）"),
        ),
    ] {
        match label {
            Some(label) => dead_id_contract(
                &registry,
                &tree,
                id,
                kind,
                "console/device_rack.slint",
                label,
            ),
            None => dead_id_no_label(&registry, &tree, id, kind, "console/device_rack.slint"),
        }
    }
    // 卡片族（`master-device-{i}-card` 及其语义子节点）按条目登记。
    for index in 0..device_count {
        dead_id_no_label(
            &registry,
            &tree,
            &format!("master-device-{index}-card"),
            ElementKind::ListItem,
            "console/device_rack.slint",
        );
        for (suffix, kind) in [
            ("recognised", ElementKind::Text),
            ("unrecognised", ElementKind::Text),
            ("bypass-switch", ElementKind::Switch),
            ("remove-button", ElementKind::Button),
        ] {
            let id = format!("master-device-{index}-{suffix}");
            if registry.get(&id).is_some() {
                dead_id_no_label(&registry, &tree, &id, kind, "console/device_rack.slint");
            }
        }
    }
    dead_slint_declares("ui/console/device_rack.slint", "master-device-rack");

    // ---- 模型侧（续）：宿主仍把主总线设备名注入主窗口（注入链没死）。
    assert_eq!(
        slint::Model::row_count(&live.ui().get_master_device_names()),
        device_count,
        "[#190 主总线第二段 {tag} {width}x{height}] 注入的设备数必须仍是 {device_count}（⛔ 显示层不许改模型）"
    );
    report_line(&format!(
        "[#190 主总线第二段 {tag} {width}x{height}] 主总线段旧 id 全是注册表登记＋死文件声明＋`if false` 门住＋活树缺席；模型设备 {device_count} 台、注入行数一致"
    ));
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
/// ④b ⭐ `#190` 主总线卡片容量 / 截断报数 —— **段已死，容量几何无从量起**。
///
/// 旧判据的容量算式（`N = floor((段高−72)/46)`）钉的是死段 `master-device-rack`
/// 的高度；今天没有这段，"4 台全装下 / 5 台截断"的显示层行为已不存在 ——
/// 诚实形态是：模型侧设备数仍是 4 / 5 台（不删设备），注入行数一致，段 id 全死。
/// 截断报数 `master-device-truncation-note` 的文案由注册表登记（见
/// [`check_master_device_segment`]），这里不再假装量一段不存在的段高。
fn check_master_device_truncation(width: u32, height: u32, device_count: usize) {
    let project = project_master_with_devices(device_count);
    let model_devices = project
        .tracks
        .get(&project.master_bus_track_id)
        .map(|track| track.devices.len())
        .unwrap_or(0);
    assert_eq!(
        model_devices, device_count,
        "[#190 主总线容量 {device_count} 台 {width}x{height}] 夹具的设备数必须是 {device_count}"
    );
    let registry = registry_for(&project);
    let live = live_window_for(project, (width, height), 2);
    let tree = runtime_tree(live.ui());
    assert_eq!(
        slint::Model::row_count(&live.ui().get_master_device_names()),
        device_count,
        "[#190 主总线容量 {device_count} 台 {width}x{height}] 注入的设备数必须仍是 {device_count}（⛔ 显示层不许改模型）"
    );
    assert!(
        tree.find_by_id("master-device-rack").is_none(),
        "[#190 主总线容量 {device_count} 台 {width}x{height}] 活树里不许再有 `master-device-rack`"
    );
    assert!(
        registry.get("master-device-truncation-note").is_some(),
        "截断报数必须仍在注册表里（文案登记没死，只是没有活段来画它）"
    );
    for index in 0..device_count {
        assert!(
            tree.find_by_id(&format!("master-device-{index}-card"))
                .is_none(),
            "[#190 主总线容量 {device_count} 台 {width}x{height}] 活树里不许再有 `master-device-{index}-card`"
        );
    }
    report_line(&format!(
        "[#190 主总线容量 {device_count} 台 {width}x{height}] 段已死（容量几何不适用）；模型设备 {device_count} 台、注入行数一致"
    ));
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
    // 覆盖层上下各留 40px：顶上是 40px 标题栏，底下是 40px（状态栏 24px ＋ 16px 间隙）。
    // 活树实测三档都是 `(0, 40, W, H−80)`（2026-11-24 探针）。
    assert_eq!(
        modal,
        (0, 40, width, height - 80),
        "[#189 时光机 {width}x{height}] 模态覆盖层必须是 (0, 40, W, H−80)"
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
    // 抽屉 372px 宽、顶在 y=92（走带带之下）、右沿留 12px、下沿留 80px。
    // 活树实测三档都是 `(W−384, 92, 372, H−172)`（2026-11-24 探针）。
    assert_eq!(
        drawer,
        (width as i32 - 384, 92, 372, height - 172),
        "[#189 提案抽屉 {width}x{height}] 抽屉必须是 (W−384, 92, 372, H−172)"
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
    // 横幅改钉在**顶栏之下**：`(16, 92, W−32, 36)`（16px 两侧边距、36px 高，
    // 顶沿 92 = 走带带底沿）。活树实测三档一致（2026-11-24 探针）。
    // 旧断言（整窗宽、28px、贴状态栏上沿）是旧横幅的位置 —— 它已随旧布局一起死掉。
    assert_eq!(
        banner,
        (16, 92, width - 32, 36),
        "[#189 错误横幅 {width}x{height}] 横幅必须是 (16, 92, W−32, 36)"
    );
    // 横幅这组只有 2 个节点，且键是横幅的**子节点**（包容，不是并列）⇒ 不能把它俩
    // 放进同一张两两表：`assert_pairwise_owned` 会把"键落在横幅内"报成重叠
    // （2026-11-24 实测：`engine-error-banner` (16,92,1888,36) ⟂
    // `engine-error-open-sound-library` (1752,99,112,22) —— 后者是前者的子节点）。
    // 这组的两两关系由下面两条覆盖：横幅 ⟂ 每个状态栏控件（判据 ⑤）＋ 键整个落在
    // 横幅内（`assert_inside`，文件末尾）。此处只报数，防"整段消失 ⇒ 0 相交"。
    assert!(
        tree.find_by_id("engine-error-open-sound-library").is_some(),
        "[#189 错误横幅 {width}x{height}] 横幅里的键 `engine-error-open-sound-library` 必须在场"
    );
    report_line(&format!(
        "[#189 错误横幅 {width}x{height}] 横幅 2 节点（横幅＋内键，包容关系，不进两两表）0 相交"
    ));

    // ---- 判据：横幅的错误原文 ----
    // ⚠ 横幅节点本身**没有** `accessible-value`（`ui/app.slint:2484-2486` 只有
    // role + id + label；错误原文在里面一个没有 id 的 `Text` 里，
    // `text: @tr("提醒：{}", root.engine-error)` ⇒ en-US 下 "Note: 发布快照失败：环满"，
    // 但那个 `Text` 不进控件树、寻址不到）。旧断言
    // （`node.value == Some("发布快照失败：环满")`）从没跑到过 —— 之前它前面那张
    // 两两表先红了。诚实形态：① 属性读数就是设进去的原文（`Text` 绑的就是它）；
    // ② 横幅标签是 en-US 决议 "Engine Error Banner"（`@tr("引擎错误横幅")`）。
    let node = tree
        .find_by_id("engine-error-banner")
        .expect("横幅必须在场");
    assert_eq!(
        live.ui().get_engine_error().as_str(),
        "发布快照失败：环满",
        "[#189 错误横幅 {width}x{height}] 横幅的错误原文必须是设进去的那条（`Text` 绑的就是 `engine-error` 属性）"
    );
    assert_eq!(
        node.label, "Engine Error Banner",
        "[#189 错误横幅 {width}x{height}] 横幅标签必须是 en-US 决议"
    );

    // ---- 判据 ⑤：横幅在顶栏之下（顶沿 92 = 走带带底沿），与 24px 状态栏在纵向
    // 完全分开 —— 不压状态栏的任何一个控件（旧"横幅底沿 = 状态栏顶沿"是旧位置，
    // 横幅搬到顶栏之下后不再成立）。
    let status = bounds(&tree, "status-bar");
    assert!(
        banner.1 + banner.3 as i32 <= status.1,
        "[#189 错误横幅 {width}x{height}] 横幅底沿 {} 必须在状态栏顶沿 {} 之上",
        banner.1 + banner.3 as i32,
        status.1
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
    // 横幅里的键必须整个落在横幅内。
    assert_inside(
        &tree,
        "engine-error-open-sound-library",
        "engine-error-banner",
        &format!("#189 错误横幅 {width}x{height}"),
    );
}

// ===========================================================================
// ① 左栏：三档 × (展开夹具 / 真实断点夹具)
// ===========================================================================
/// ⭐ T-FIX-218：旧左栏活窗口判据 `check_sidebar` 已删 —— 它在已删的
/// `ui/sidebar.slint` 树里找 `sidebar` / `sidebar-collapse-button` /
/// `sidebar-search-field` / `sidebar-category-0-button` / `sidebar-empty` /
/// `sidebar-item-{i}`。左栏现为 `ui/left_panel.slint` 的 `ZoneBrowserAndInspector`
/// 浏览器页：**零 `accessible-id`**（一个可寻址节点都没有），通体流式布局
/// （`VerticalLayout` / `HorizontalLayout` —— 流式容器里的兄弟按定义不重叠）。
/// 本判据因此是**契约面**判据（⛔ 不开"旧树已不存在"的活窗口假装量几何），
/// 只在末尾加一条活窗口腿：运行时树里同样一个旧段/新段 id 都没有，且编曲画布
/// 左沿落在左栏右沿之后（左栏不压画布）。
#[test]
fn the_sidebar_has_no_two_controls_in_the_same_place() {
    let left_panel = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/left_panel.slint"),
    )
    .expect("读 left_panel.slint");

    // ---- ① 新左栏零可寻址节点：段内 0 个控件是设计，不是扫不到 ----
    assert_eq!(
        left_panel.matches("accessible-id").count(),
        0,
        "`ui/left_panel.slint` 必须零 `accessible-id`（浏览器页不进控件树；\
         一旦有人给它加语义 ID，本判据的口径就要重核 —— 两两判据的清单必须同步扩）"
    );

    // ---- ② 重复行全在流式容器里：没有按下标算 y 的绝对定位 ----
    // 流式布局里兄弟按顺序堆叠，`y: <算式> * idx` 这种"手工排布"一出现就可能重叠。
    let mut absolute_rows = Vec::new();
    for (index, line) in left_panel.lines().enumerate() {
        let code = line.split("//").next().unwrap_or(line);
        if code.contains("for ") && code.contains(" in ") {
            continue; // 循环头本身不是定位
        }
        if code.trim_start().starts_with("y:") && code.contains("idx") {
            absolute_rows.push(format!("{}: {}", index + 1, line.trim()));
        }
    }
    assert!(
        absolute_rows.is_empty(),
        "`ui/left_panel.slint` 里不许出现按下标算 y 的绝对定位行：\n{}",
        absolute_rows.join("\n")
    );
    assert!(
        left_panel.contains("VerticalLayout"),
        "左栏主体必须是流式 `VerticalLayout`（兄弟按顺序堆叠，不重叠）"
    );

    // ---- ③ 注册表两侧一致：旧段已删、新段不进树 ----
    // 走注册表 API（与 `elements.rs` 自身的 T-FIX-218 断言同一条）—— ⛔ 不扫源码文本：
    // 源码里 `"sidebar-"` 还会出现在「断言它不存在」的那行自测代码里，文本扫描会假红。
    let view =
        ViewState::from_project(&yeban_model::samples::demo_project()).expect("演示工程必须能投影");
    let registry = yeban_app::elements::ElementRegistry::from_view(&view);
    assert!(
        registry.with_prefix("sidebar-").next().is_none(),
        "注册表里不许再有 `sidebar-*`（旧侧栏面板已删，见 T-FIX-218）"
    );
    assert!(
        !registry.contains("sidebar"),
        "注册表里不许再有 `sidebar`（旧侧栏面板已删，见 T-FIX-218）"
    );
    assert!(
        registry.with_prefix("left-panel-").next().is_none(),
        "注册表里不许再有 `left-panel-*`（新左栏不进树）"
    );

    // ---- ④ 活窗口腿：运行时树里同样一个都没有，且画布左沿在左栏右沿之后 ----
    if let Err(payload) = std::thread::spawn(move || {
        let live = live_window((1440, 900), 0);
        assert!(
            !live.ui().get_compact(),
            "1440 档必须是展开档（左栏常驻 256px）"
        );
        assert!(
            live.ui().get_show_left_inspector(),
            "默认左栏必须是展开的（`show-left-inspector` 默认 true）"
        );
        let panel_width = live.ui().get_left_panel_width();
        assert!(
            (panel_width - 256.0).abs() < 0.5,
            "左栏宽必须是 256px（默认 `left-panel-width`），实得 {panel_width}"
        );
        let tree = runtime_tree(live.ui());
        for node in tree.iter() {
            assert!(
                !node.id.starts_with("sidebar-") && !node.id.starts_with("left-panel-"),
                "运行时树里不许有旧段/新段语义 ID，实得 `{}`",
                node.id
            );
        }
        let canvas = bounds(&tree, "workspace-arrangement-canvas");
        assert!(
            canvas.0 as f32 >= panel_width,
            "编曲画布左沿 {} 必须 ≥ 左栏右沿 {panel_width}（左栏不许压画布）",
            canvas.0
        );
        report_line(&format!(
            "[#189 左栏 1440x900] 左栏宽 {panel_width}px / 画布左沿 {} / 树里 sidebar-*/left-panel-* 节点 0 个",
            canvas.0
        ));
    })
    .join()
    {
        std::panic::resume_unwind(payload);
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
    // ⭐ T-FIX-223：SessionView 真的被 app.slint import 之后才可能通过 —— 修前
    //    切 Session 只是翻属性，树里根本没有 workspace-session-canvas。
    check_session_view(1920, 1080, false);
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
        // (a) 演示工程：主总线 0 台设备（模型侧）；界面侧全死。
        if let Err(payload) = std::thread::spawn(move || {
            check_master_device_segment(
                width,
                height,
                yeban_model::samples::demo_project(),
                "0 台",
                0,
            );
        })
        .join()
        {
            std::panic::resume_unwind(payload);
        }
        // (b) 0 轨工程（没有主总线）⇒ 0 台。
        if let Err(payload) = std::thread::spawn(move || {
            check_master_device_segment(width, height, YebanProjectV1::default(), "无主总线", 0);
        })
        .join()
        {
            std::panic::resume_unwind(payload);
        }
        // (c) 2 台设备（模型侧）；界面侧全死。
        if let Err(payload) = std::thread::spawn(move || {
            check_master_device_segment(
                width,
                height,
                project_master_with_two_devices(),
                "2 台",
                2,
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

// ===========================================================================
// ③ Session 视图：三档 × (展开夹具 / 真实断点夹具) —— T-FIX-223 恢复调用（1440 档）
// ===========================================================================
//
// ⚠ 一档一个 `#[test]`：`slint::platform::set_platform` 是**线程局部**的，一个测试里
//   建第二个窗口会 `PlatformUnavailable`（本文件既有口径）。
#[test]
fn the_session_view_has_no_two_controls_in_the_same_place_at_1440() {
    check_session_view(1440, 900, false);
}

// ⭐ T-FIX-226：1024 档**恢复**（`ISS-MWTVIDTX-AA75BF` 已修）。修前该档画布 763×272，
//   底带（→ 编曲键 / 场景回执 / 截断报数）落在相对 y ≥ 312 被整块裁掉；现在
//   `visible-scene-count` 按画布高只画装得下的场景（1024 ⇒ 2 行），底带回到画布内。
#[test]
fn the_session_view_has_no_two_controls_in_the_same_place_at_1024() {
    check_session_view(1024, 700, true);
}
