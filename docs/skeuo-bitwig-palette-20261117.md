# 拟物（physical）主题 Bitwig 对色表（2026-11-17）

负责人定（2026-11-17 18:57）：拟物版本的各种控件颜色和背景色改用
Bitwig 截图色板；19:00 修正：控件与硬件面的物理质感透叠保留该用就用，
只有编辑区域不用透明、走实色。本表是对色线（`line/ui-skeuo-palette`）
的采样、映射、对比度复核与透叠清点的落盘记录。

截图：`~/workspace/user/files/62A1C94A-5EAE-4CC8-99F7-DBF71A62FEC4-photo-1AC8F945-C9F8-4A5E-A297-84029A38C190.jpg`
（Bitwig Studio 深色界面，1170×653 JPEG；采样 = PIL 全图量化 +
六分区量化 + 饱和像素扫描，`np.asarray` 后 `.astype()` 口径）。

## 1. 截图采样主色

| 角色 | 采样值 | 采样依据 |
| :-- | :-- | :-- |
| 窗口/面板底（深） | `#141A1F` | 全图量化 6.0%，多分区复现 |
| 面板底 | `#1B2329` 一带（`#1F272E`/`#22282F`） | 编曲区/混音区分区前列 |
| 抬升面 | `#21272E`、`#262F37`（量化簇 `#262D33`/`#2C3339`） | 启动器/推子区分区 |
| 控件面 | `#2E3A41` 一带（簇 `#323B43`/`#34474E`/`#3E464D`） | DSP 弹窗/推子帽分区 |
| 描边 | `#31353B` / 强 `#404B51` | 分区边缘簇 |
| 深青（设备面/选中底） | `#1B4848`（全图 4.6%）、`#15413B` | DSP 弹窗分区 11.9% |
| 青绿 accent | `#67B8A5`、亮 `#30B8A0`（簇 `#20A070`/`#30D0C0`） | 饱和扫描前列 |
| 蓝青（副 accent） | `#4C7D81`（簇 `#408090`/`#5090A0`） | 编曲区分区 |
| 亮青（选中/走带） | `#29B7D3` 级（簇 `#50C0D0`） | 混音区饱和扫描 |
| 电平绿 | `#40D070` 级 | 混音区饱和扫描 |
| 亮文字 | `#C0C0C0` 级（近白低饱和簇） | 顶栏/弹窗亮像素扫描 |
| 弱文字 | `#606060`–`#707880` 级 | 启动器中值低饱和簇 |

## 2. 令牌映射（采样值 → 令牌 → physical 旧值 → 新值）

改动只落 `crates/yeban-app/ui/tokens.slint` §10/§11/§12 的
**physical 分支**与 `app.slint` 窗口底一处质感三元；flat 分支逐字
不动，扁平主题一色未改。physical 是一套独立色板：原 brand/yeban
两支分叉的 physical 停点收敛为同一组 Bitwig 值。

| 令牌 | 采样来源 | physical 旧值 | physical 新值 |
| :-- | :-- | :-- | :-- |
| `panel-brush`（面板） | 面板底 `#1B2329`、抬升 `#262F37`、底 `#141A1F` | brand `#1A2342/#151D38/#121A30`；yeban `#171F2A/#131A22/#0F151D` | `#262F37 / #1B2329 45% / #141A1F` |
| `panel-alt-brush`（交替面板） | 抬升面上一档 | brand `#202A52/#1B2447/#161E3A`；yeban `#1A232F/#161E28/#121922` | `#2E3740 / #232B33 45% / #1B2329` |
| `shell-brush`（外壳/容器） | 顶栏 `#20282F`、容器 `#171F26` | brand `#111A33/#0D1326/#0A0F1F`；yeban `#151B23/#11161C/#0D1218` | `#20282F / #171F26 45% / #11161C` |
| `raised-brush`（按键/推子帽） | 控件面 `#2E3A41`、顶光 `#3E4A54` | brand `#35477C/#283763/#1A2547`；yeban `#2E3C4E/#222D3B/#141B24` | `#3E4A54 / #2E3A41 50% / #232B31` |
| `void-brush`（凹陷/井） | 深底 `#0D1116`/`#10151A` | brand `#04060C/#060A14/#0A1020`；yeban `#0A0E12/#0E1216/#131920` | `#0A0E12 / #0E1318 55% / #131A20` |
| `knob-brush`（旋帽径向） | 旋钮本体 `#323B43`/`#293036` | brand `#3C4F7D/#222F52/#0E1526`；yeban `#414F61/#25303D/#11171E` | `#3A454F / #252E36 55% / #12181E` |
| `knob-ring`（旋钮座环） | 深座 | brand `#070C16`；yeban `#090E13` | `#0A0E12`（两支统一） |
| `glass-brush`（屏面） | 深青屏 `#1B4848` 压暗 | `#0B1017 / #05070B 62%` | `#14201D / #090F12 62%` |
| `accent-brush`（开关选中底/滑块头） | 青绿 accent `#67B8A5`/`#30B8A0` | brand `#DCBC66/#BD9C44/#8A6C2A`（金）；yeban `#97B4CC/#7290AB/#4F6C88`（蓝灰） | `#74BFA9 / #3FA38B 50% / #276B5A` |
| `divider-row`（轨道区结构线，§11） | 描边强档 `#404B51`（提明度后） | `#43536A` | `#47535C`（见 §3 提明度说明） |
| `track-head-brush`（轨道头，§12） | 轨道头 `#1B2329` 家族 | `#121828 / #0B0E16 45% / #070A10` | `#232C34 / #1B2329 45% / #141A1F` |
| `roll-key-white-brush`（白键，§12） | 白键浅蓝灰 | `#F2F5FA / #E4E9F2 55% / #CCD5E4` | `#D2D7DB / #B7BDC2 55% / #989FA5` |
| `roll-key-black-brush`（黑键行，§12） | 黑键深蓝灰 | `#202634 / #191E2B 55% / #12151F` | `#242D33 / #1B2329 55% / #131A1F` |
| `roll-key-cap-brush`（黑键帽，§12） | 黑键帽更深一档 | `#141926 / #0B0E15 60% / #05070C` | `#1A2229 / #11171D 60% / #0A0F14` |
| 主窗口底（`app.slint` 三元） | 窗口底 `#141A1F` | physical 时仍走 `Theme.bg-canvas`（persona 值） | physical `#141A1F`；flat 仍 `Theme.bg-canvas` |

未改：§6 配色令牌（`bg-*`/`ink-*`/`gold*`/`selection`/`playing`/
`record-red` 等——配色主题维度，非质感维度）、语义色
（`ok-green`/`warn-amber`/diff 色）、LED 电平分区色、旋钮指示色
（调用点按参数语义注入）、`Theme` persona 家族。

## 3. 对比度复核（WCAG 2.1 相对亮度公式实算，§7.3 口径）

| 配对 | 比值 | 门槛 | 结论 |
| :-- | :-- | :-- | :-- |
| `ink-0 #D7DEE1` on 面板中停 `#1B2329` | 11.69:1 | 文本 4.5 | ✓ |
| `ink-0` on 外壳中停 `#171F26` | 12.24:1 | 4.5 | ✓ |
| `ink-0` on 控件面中停 `#2E3A41` | 8.58:1 | 4.5 | ✓ |
| `ink-0` on 屏面底停 `#090F12` | 13.90:1 | 4.5 | ✓ |
| `ink-1 #8D99A5` on 面板/轨道头中停 | 5.48:1 | 4.5 | ✓ |
| `ink-1` on 控件面中停 `#2E3A41` | 4.02:1 | 4.5 | △ 边际（见下注） |
| 开关选中字 `bg-void #0E1216` on `accent-brush` 中停 `#3FA38B` | 6.11:1 | 4.5 | ✓ |
| `accent-brush` 中停 on 面板（图形） | 5.17:1 | 图形 3.0 | ✓ |
| `playing #6E9488` on 面板（图形） | 4.74:1 | 3.0 | ✓ |
| `selection #8E86A6` on 面板（文字） | 4.63:1 | 4.5 | ✓ |
| `ok-green #3FA96A` on 屏面（图形） | 6.39:1 | 3.0 | ✓ |
| `divider-row #47535C` on 面板 | 2.13:1 | 发丝线（无硬门槛） | 见下 |
| `ink-2 #5A6672` on 面板中停 | 2.71:1 | 3.0 | 既有登记缺口（见下） |

提明度处理（Bitwig 原色不达标、同色相只提明度）：
- `divider-row`：采样原值 `#404B51` 对新面板底仅 1.78:1，低于旧
  拟物值 `#43536A` 的可读性（2.03:1）；同色相提明度至 `#47535C`
  （2.13:1），恢复旧可读性。发丝结构线无 §7.3 硬门槛，此为
  「不低于旧值」的平移处理。

如实登记的边际项（非本次引入的归零项）：
- `ink-1` on 控件面 4.02:1：Bitwig 控件面（`#2E3A41`）比旧拟物
  控件面（`#222D3B`，4.7:1）亮一档所致；实际落在帽面上的文字
  以 `ink-0`（8.58:1）为主，`ink-1` 多落在面板（5.48:1）。
- `ink-2` on 面板 2.71:1：`ink-2` 是既有登记的霜灰缺口
  （`theme_selection.rs` 的 `OWNER_TEXT_FAINT_SHORTFALL`，对
  yeban 面板 2.99:1），属配色维度，本线不动 `ink-*`。

## 4. 控件与硬件面透叠标注（19:00 口径：保留，仅标注）

| 位置 | 透叠 | 用途 |
| :-- | :-- | :-- |
| `tokens.slint` `bevel-light` | 白 0.14 | 斜面亮边（上/左） |
| `tokens.slint` `bevel-dark` | 黑 0.55 | 斜面暗边（下/右） |
| `tokens.slint` `recess-dark` / `recess-light` | 黑 0.65 / 白 0.08 | 凹槽内影近似 |
| `tokens.slint` `knob-highlight` | 白 0.16 | 旋帽左上哑光高光弧 |
| `tokens.slint` `glass-reflection` | 白 0.035 | 屏面斜向反光（全屏一道） |
| `tokens.slint` `glass-glow-alpha` | 同色 0.18 | 屏内曲线辉光底层（信息发光） |
| `tokens.slint` `soft-sheen` | 白 0.09 | 按键顶光 |
| `tokens.slint` 阴影 alpha | 黑 0.38 / 0.55 / 0.32 | 面板/控件/按下态落影 |
| `controls/fader.slint:120` | 白 0.28（仅 physical） | 推子帽中线 |
| `controls/knob.slint:103` | `ink-2` 0.62（physical 档） | 未扫过刻度段 |
| `controls/knob.slint:192` | `bg-panel` 0.55 | 禁用罩（仅禁用态） |
| `controls/led_meter.slint:95/97/109` | 分区色 0.13 / 0.20、白 0.55 | 电平未亮段/亮段辉光/段面 gloss（信息面，两模式同） |

## 5. 编辑区域透叠清点（19:00 口径：只清点编辑区域）

口径：内容底与标记应走实色；hover/拖拽/框选反馈沿用既有豁免；
数据编码 opacity 按 2026-11-17 去透明定案只登记。

| 文件/位置 | 点 | 处置 |
| :-- | :-- | :-- |
| `console/piano_roll.slint` | with-alpha / opacity / 8 位 hex 均为 0 处 | 已实色，无点可清 |
| `workspace/session_view.slint` | 同上 0 处 | 已实色，无点可清 |
| `workspace/song_overview_panel.slint:253` | LUFS 能量柱 `opacity 0.22` | 数据层叠画，既有登记类，不改 |
| 同上 `:278/:290` | 包络带 0.20 / 偏离告警 0.18 | 同上 |
| 同上 `:302` | 张力线深色护城河 0.55 | 线条衬底，既有登记类，不改 |
| 同上 `:360/:422/:477` | 视口窗 0.10 / 拖拽手柄 0.7 / 读数牌 0.92 | 视口指示与读数牌，既有登记类，不改 |
| 同上 `:528/:556` | 密度 `duty×0.25` / 段落线 0.5 | opacity 编码数据（登记规则：接线时用亮度阶梯），不改 |
| `clip_matrix.slint:44/:59` | 新增/停止钮 hover 底 `accent 0.25` / `#EF444433` | hover 豁免；格面本体已实色（去透明线已改） |
| `arrangement.slint:1190/:1192` | 片段底 `trk.color 0.12`、未选片段边 `trk.color 0.55` | **网格线领地（文件级避让，本线不动）**：网格支九版已改片段底为轨道色纯色、去同色边框，待其合入 |
| `arrangement.slint:1236/:1248/:1281` | 波形基准线 0.20 / 峰值柱 0.85 / MIDI 分界线 0.15（均 `trk.color`） | 同上：网格支十版已改同色相派生实色标记（`clip_marker_color`），待其合入 |
| `arrangement.slint:808/:809/:819/:1359` | 选中行底 `accent 0.08`、选中边 0.70/0.80 | 同上：网格支七/八版已改提亮承载＋实色边，待其合入 |
| `arrangement.slint:390` | 悬浮缩放控件板 `#0B0E16CC`（α≈0.8） | 画布上悬浮控件板，同文件避让，仅清点 |
| `arrangement.slint:858/:1397` | 轨道头音色徽章底 `#00000050`（hover 转实色） | 轨道头（硬件面），且同文件避让，仅清点 |
| `arrangement.slint:186` 等 | 和弦开关 `accent 0.25`、和弦预置片 8 位 hex 淡底/边 | 工具行控件面＋同文件避让，仅清点 |
| `tokens.slint` §11 `divider-grid` / `divider-grid-strong` | physical 分支 `line 0.55` / `line-strong 0.8` | **网格线令牌（交叉点）**：网格十一版正在改网格线色，本线不动 |
| `tokens.slint` §11 `divider-row-highlight` | 白 0.05 | 结构线刻蚀高光，同 §11 交叉，本线不动 |
| `tokens.slint` §11 `ruler-brush` | physical 为 `bg-panel→bg-shell` 渐变 | 标尺带在 physical 下仍随配色令牌、未走 Bitwig 刷（交叉点登记） |

结论：本线可动的编辑面文件（卷帘/Session/Clip 矩阵/总览）在
physical 下没有**由质感层引入**的内容透叠；编曲区画布的透叠点
全部落在网格线在途分支的改造范围内，本线只清点、不落码。

## 6. 已知缺口（登记，不在本线扩围）

新对话框未接质感三元（physical 下仍显配色主题面，非 Bitwig）：
`dialogs/startup_chooser.slint`（裸 bg 15 处）、
`dialogs/sound_library_dialog.slint`（11 处）、
`dialogs/lessons_drawer.slint`（5 处）、
`dialogs/onboarding_overlay.slint`（4 处）、
`dialogs/template_chooser.slint`（4 处）、
`dialogs/save_as_template.slint`（3 处）、
`dialogs/help_manual.slint`（3 处）、
`dialogs/help_shortcuts.slint`（2 处）。
已接三元的对话框（export_wav / humanize / macro_panel /
musical_pr_drawer / undo_tree_modal）随 `panel-brush` 自动对色。
建议后续单开「新对话框质感接线」线，按三期同款三元口径补齐。
