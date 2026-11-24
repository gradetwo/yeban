# 夜半桌面 UI 九语 i18n 设计与实施记录（line/ui-i18n）

负责人定案（Wang 2026-11-17）：夜半开分支完成九语 i18n。语言 = zh-CN（默认）
＋ en-US、fr-FR、es-ES、pt-BR、ja-JP、ko-KR、ru-RU、de-DE；运行时可切换、
不重启生效；选择持久化、启动按优先级自动选语言。产品口径对齐偃师 web 端
i18n：单一词表按中文原文精确匹配、默认中文、其余八语显式切换、开关在顶栏、
日志与诊断不翻译、覆盖判据守漏词条。

## 1. 普查（基线 c6729977d，词法级提取，已剔注释）

### 1.1 .slint 侧（crates/yeban-app/ui/**/*.slint，26 个文件）

硬编码中文字符串字面量：**719 处出现 / 585 条唯一**。tokens.slint 为 0（纯样式令牌）。

| 文件 | 出现 | 唯一 | 文件 | 出现 | 唯一 |
|---|---:|---:|---|---:|---:|
| ui/app.slint | 80 | 80 | ui/dialogs/musical_pr_drawer.slint | 14 | 14 |
| ui/arrangement.slint | 76 | 57 | ui/dialogs/undo_tree_modal.slint | 25 | 23 |
| ui/clip_matrix.slint | 15 | 9 | ui/left_panel.slint | 47 | 46 |
| ui/console/asset_pool_panel.slint | 10 | 6 | ui/lower_dock.slint | 58 | 56 |
| ui/console/console_tabs.slint | 7 | 7 | ui/modals.slint | 22 | 22 |
| ui/console/device_rack.slint | 85 | 65 | ui/module_actions.slint | 16 | 16 |
| ui/console/mixer_console.slint | 36 | 28 | ui/sidebar.slint | 13 | 10 |
| ui/console/piano_roll.slint | 25 | 22 | ui/status_bar.slint | 10 | 9 |
| ui/control_room.slint | 9 | 9 | ui/transport.slint | 18 | 18 |
| ui/dialogs/export_wav_dialog.slint | 20 | 17 | ui/transport_bar.slint | 33 | 30 |
| ui/dialogs/humanize_dialog.slint | 23 | 18 | ui/workspace/arrangement_view.slint | 25 | 22 |
| ui/dialogs/macro_panel.slint | 7 | 6 | ui/workspace/session_view.slint | 15 | 11 |
|  |  |  | ui/workspace/song_overview_panel.slint | 30 | 28 |

> ⚠ 死文件提示（T-FIX-230）：上表中的 `ui/module_actions.slint`（16）、`ui/sidebar.slint`（13）、
> `ui/transport.slint`（18）、`ui/workspace/arrangement_view.slint`（25）、
> `ui/workspace/song_overview_panel.slint`（30）**均已不在活树中**（从未被 `app.slint` import），
> 这些 `@tr` 串不进入任何活界面、也不会出现在运行时语言包里。清单与替换表见 `docs/DEAD-SLINT-FILES.md`。

形态分类（按唯一串计）：

- 纯标签/提示串（直接 `@tr()` 包裹即可）：约 500 条；
- 拼接片段：82 条带首尾空格的片段（如 `"声像: "`、`" (左右拖拽调节，双击归中)"`），
  改造时合并为带 `{}` 占位符的整句 `@tr("声像: {} (…)", value)`，译文按整句写；
- Slint 插值串：3 条（`"更换轨道 #\{…} 音色 / 插入"` 类），改 `@tr("… #{} …", id)`；
- 复数形（`|` 分隔）：0 条，本期不需要 plural 词条；
- 与逻辑耦合的展示串：如走带栏调式名 `"C Minor (自然小调)"` 被 `==` 比较并参与循环切换——
  属性值保持中文原文作规范值，只在显示点做 `@tr` 映射，逻辑比较不动（详见 §5）。

### 1.2 Rust 侧（crates/yeban-app/src/**/*.rs）

中文字符串字面量共 4481 处 / 3613 条唯一，绝大多数是 CLI 输出、MCP 工具说明与结果、
错误与诊断文本、测试数据——按口径**全部排除**（日志与诊断不翻译）。
直接面向 UI 的部分只有经属性喂给界面的动态状态串（host.rs 的 toast/状态提示类），
其内容多为带数值的诊断与结果汇报，同属诊断类，本期排除并在此明写；
（例外：状态栏选区 / 音频配置两格是**常驻显示**而非瞬时提示，收尾复核时
改判须译，见 §9 补记。）
界面静态文案全部住在 .slint 侧，本期范围即 §1.1 的 585 条。

## 2. 机制选型

### 候选 A：Slint 内置 `@tr()` ＋ bundled translation（选定）

- build.rs：`CompilerConfiguration::with_bundled_translations("translations")`
  ＋ `with_default_translation_context(DefaultTranslationContext::None)`（关掉默认的
  组件名上下文 ⇒ 词表按中文原文单一键，与偃师口径一致）；
- 词表文件：`crates/yeban-app/translations/<locale>/LC_MESSAGES/yeban-app.po`
  （域名 = CARGO_PKG_NAME = `yeban-app`，编译期由 rspolib 解析、逐串编进二进制），
  九语共八份目标词表（zh-CN 是源语言，无词表目录）；
- 运行时：`slint::select_bundled_translation("en-US")` 切英文，
  `select_bundled_translation("")` 回源语言（中文）；缺词条时自动回退原文，不会显示空白。

**实测证据（2026-11-17，本机最小探针，Slint 1.18.1 与主仓同版本同配置）**：
独立探针窗（软件渲染）里同一组件实例先渲染中文帧 → 调
`select_bundled_translation("en-US")` 后回读绑定属性：`播放 → Play`、
tooltip 串同步变英文；像素差 zh→en = 2764 px；切回 `""` 后属性回 `播放`，
且往返后的中文帧与初始帧**逐像素一致**。即：已渲染界面即时全量生效，不需重建组件。
源码侧佐证：`i-slint-core-1.18.1/translations.rs` 的 `select_bundled_translation`
写 `translations_dirty` 索引属性，`translate_from_bundle` 在绑定求值时读该属性 ⇒
依赖登记成立，切换即标脏全部 `@tr` 绑定。

代价/注意：

- 构建期多一步 .po 解析（毫秒级）；二进制体积增加约等于译文总量（数十 KB）；
- `set_bundled_languages` 会在首个组件创建时按**系统 locale 自动选语言**——
  本应用默认必须是中文，所以宿主在建窗后必须显式应用一次语言选择（见 §3），
  把自动选择覆盖掉；不显式调用时英文系统上会首帧英文，与定案不符；
- 选中 en-US 时小数分隔符随 locale 变（`.`），与源语言同为 `.`，无影响；
- `@tr` 只接受字面量 msgid：动态拼装的串要在字面量产生点翻译（§1.1 的分类处理）。

### 候选 B：自建全局词表单例（仿偃师按中文原文键）＋绑定函数（未选）

- 形态：全局单例 `I18n { lang }` ＋ Rust 回调 `tr(原文, lang)` 查表，或生成的纯 Slint
  字典函数。切换靠绑定显式读 `lang` 属性触发重求值。
- 不选的原因：① 每处调用都要把 `lang` 喂进表达式才有依赖登记，漏一处即静默不刷新，
  而 A 的依赖登记由编译器统一生成、无漏点；② 词表完整性、缺词回退、格式化占位
  都要自研一遍，A 是现成且已在本仓 Slint 版本里验证过的同一套；③ 守卫判据两者
  都要扫源码，成本相同。B 的唯一优势是不动 build.rs，影响面小于 A 的收益。

### 第三语言成本（A）

新增 `translations/<lang>/LC_MESSAGES/yeban-app.po` 一份文件＋切换入口加一个取值，
零代码结构改动。词条以中文原文为键，天然与偃师词表口径互通。
2026-11-17 已按此成本一次扩七语（见 §10），无结构改动。

### 与 UiText/Tokens 体系的冲突面

`@tr()` 返回 `string`，与现有 `text:`/`title:` 等属性类型完全兼容，UiText 只是样式
组件、不经手文案，零冲突。Tokens 全局不新增语言状态（语言状态归宿主 Rust 持有，
经 MainWindow 属性下发给走带栏开关），tokens.slint 一行不改——theme_selection 判据
⑤ 对 tokens.slint 的字面量钉不受影响。

## 3. 切换入口与持久化

- 入口有两处，同调一个 `MainWindow.select-language(string)` 回调（入参为
  语言规范名，宿主接管：`host::apply_language_and_save` 应用＋写回设置）：
  ① 走带栏右岛 Group L：按钮显示当前语言**自称名**
  （`MainWindow.language-name`，宿主持有），点击弹出九语菜单
  （`PopupWindow`，9 项 × 32px，当前项名前打勾；先例见
  `ui/console/piano_roll.slint` 的右键菜单）；② 主菜单新增「设置」菜单，
  内列九语自称名（当前项标题前缀 `✓`；不用 `MenuItem.checked`——它在
  激活时自翻，与 `language-code` 绑定冲突，故拼标题前缀）。
  选走带栏＋主菜单双入口的理由：走带栏是找语言开关的心智位置（与质感
  开关同区），主菜单是 Wang 05:57 令明确要求的"菜单里能改"；两处都只
  发同一个回调，不各立接线。
  **2026-11-17 晚追加（Wang 口径变更）**：语言与质感两个切换入口统一进
  系统主菜单、不要走带栏按钮/chip——走带栏 Group L 语言 chip 与质感
  开关已同批撤销，「设置」菜单改为「语言」/「质感」两个子菜单（质感
  经新增 `MainWindow.select-texture` 回调 → `host::apply_texture`，
  会话内生效、不写设置文件，与质感既有口径一致）。本节上文的双入口
  描述自此失效，入口只剩主菜单一处；回调与持久化语义不变。
- 宿主侧语言状态：`MainWindow.language-code`（规范名）与 `language-name`
  （自称名）两个属性，`host::apply_language` 是唯一写入口；闭集显示标签
  （总览曲风档名、状态栏选区/音频配置两格）按 `host::current_language()`
  取词，切换时重标（见 §9）。
- CLI `--lang <语言>`（照 `--theme` 的解析与拒绝形态）：九语规范名与
  语言段简写（`fr`/`es`/`pt`/`ja`/`ko`/`ru`/`de`/`en`/`zh`）都接受；
  未知取值报 `UnknownLanguage`，绝不静默回退。
- 持久化（2026-11-17 05:57 Wang 令新增；此前"仓内无设置持久化路径"，
  本线新建）：设置文件 `yeban/settings.json`，放平台配置目录
  （Linux `$XDG_CONFIG_HOME/yeban/`，未设则 `$HOME/.config/yeban/`；
  macOS `$HOME/Library/Application Support/yeban/`；
  Windows `%APPDATA%/yeban/`；仓内此前无设置路径先例，全仓 grep
  `config_dir` / `settings.json` 零命中，故新建；目录名全平台统一小写
  `yeban`）。格式 `{"version": 1, "language": "fr-FR"}`，`version` 是给
  后续线的演进位；读不到 / 读坏 / 取值不在九语里 ⇒ 当作没有已存选择，
  不报错。实现见 `crates/yeban-app/src/settings.rs`（`serde_json::Value`
  手工读写，不引新依赖；写临时文件再 rename；`YEBAN_SETTINGS_PATH`
  环境变量可覆盖路径，仅供单测与取证工具隔离用，生产不设）。
- 启动优先级（定案，纯函数 `settings::resolve_startup_language`，单测钉死）：
  `--lang` 参数（显式给过才算，`Options.lang_given` 标记；临时覆盖、**不写回**）
  ＞ 已存选择 ＞ 系统 locale 自动检测 ＞ zh-CN。系统检测读进程环境
  `LC_ALL` ＞ `LC_MESSAGES` ＞ `LANG`（POSIX 优先级），映射到九语
  （`pt-PT` 落到 pt-BR——本线只有巴西葡语一支；`zh-TW` 落到 zh-CN——
  只有简体中文一支；认不出 / `C` / 空 ⇒ zh-CN）。检测复述 Slint 首帧
  自动选择（`set_bundled_languages` 按系统 locale 选词表），不是第二套
  检测。已知缺口：Windows 上 `LANG` 类变量通常未设，检测落回 zh-CN
  （Slint 自身在 Windows 上能读到系统语言，但 Rust 侧读不到；Windows
  用户首次启动为 zh-CN，菜单里选一次后即持久化）。缺口在此登记，不静默。

## 4. 范围与排除项

范围：全部 .slint 静态界面文案——标签、按钮、tooltip、面板标题、菜单、对话框、
状态栏静态文案（§1.1 的 585 条唯一串）。

排除（判据里以白名单＋理由登记）：

- 用户内容：轨道名、片段名、工程数据（本来就不是字面量）；
- 日志与诊断输出（Rust 侧全部，见 §1.2）；
- 专有名词与品牌名（如「夜半」产品名本体、Persona 名）；
- 纯符号与快捷键标注（`[F4]`、`Ctrl+K` 等 ASCII 内容本来就不含中文；
  混合串里的快捷键部分在译文中原样保留）；
- 语言开关自身的「中文」字样（语言自称名，不翻译）；
- 与逻辑耦合串的**属性值**本身（只译显示点，见 §1.1）。

## 5. 覆盖判据

新增 `crates/yeban-app/tests/i18n_coverage.rs`（仿 theme_selection.rs 的源码文本判据形态）：

1. 扫 `ui/**/*.slint`（词法级，剔注释）：任何含中文的字符串字面量若不是 `@tr(...)`
   的直接实参（或在白名单内），判红并列出文件:行与原文；
2. 词表完整性：源码中 `@tr` 的全部 msgid 集合必须与**八个目标语言**词表的
   msgid 集合逐一**相等**（漏译、孤儿词条、空译都红）；另逐语做删词条变异：
   每语删 `选区 无` 一条，判据必须当场点名该条（变异可抽样，全量对账在判据本体）；
3. 变异验证：随手加一条假硬编码中文串 ⇒ ① 红；删掉 ⇒ 绿（实施时实测并记回执）。

宿主闭集显示标签（四处：总览曲风档名、状态栏选区/音频配置两格、模板
选择器来源分组标题）不在本判据的 .slint 观测面内，各自由九语单测看守
（`src/song_overview.rs` / `src/host.rs` 内：九语档名表、空选区常量与
各语 `.po` 的 `选区 无` msgstr 逐字符对账、设备格后缀九语重标拿全部
采样率 × 位深组合验；`src/templates.rs` 内：九语分组标题逐条钉死）。

## 6. 实施批次

1. 调研文档（本文件）＋ build.rs 接 bundled translations ＋ 空词表骨架；
2. 探针结论落仓：host.rs 语言应用函数＋`--lang`＋走带栏 Group L 开关（先通链路）；
3. .slint 全量改造（按文件分批提交）＋ en-US.po 全量译文；
4. 覆盖判据＋变异验证；
5. 渲染证据：headless 出主窗口 zh/en 全帧图存 `~/workspace/i18n-shots/`，逐区自查。

## 7. 与 line/ui-skeuo-theme 的预期冲突面（将来 rebase 时）

- `ui/transport_bar.slint`：skeuo 在右岛末尾加了 Group L 质感开关；本线在其前/后加
  Group L 语言开关——同一区域两处插入，rebase 时需手工排序（预计小冲突）；
- `ui/app.slint`：两边都加 MainWindow 属性/回调（质感 vs 语言），相邻行冲突概率中；
- 其余 .slint 文件：skeuo 改材质令牌引用、本线改文案字面量，多数可自动合并，
  同一行同时被改的点（既有材质又有文案的 Text）会零星冲突；
- `crates/yeban-app/translations/` 与 `tests/i18n_coverage.rs` 为本线独有，无冲突。

## 9. 完工状态（2026-11-17，实施后补记）

- 路由结果：666 处 `@tr` 调用点（去注释后计数）、565 条唯一 msgid；八份目标词表的 msgid
  集合与源码精确相等且无空译（判据 ② 机械保证）。计数沿革：原 571 条；
  2026-11-17 rebase 到 main c58c70369 时，ui-header-cleanup 删除的六条
  冗余标题随删除成为孤儿词条，已从 .po 移除（→565）；九语扩展时 Group L
  两态提示合并为一条菜单提示（删 2 增 1，→564）；主菜单「设置」新增
  `设置` 一条 msgid（→565）。「素材池」因 accessible-label
  仍在而保留。
- 普查修正一处：钢琴卷帘的工具名（选择/铅笔/剪刀/力度/橡皮）是
  piano_roll.slint 内的 .slint 侧模型默认值，已随机械包裹覆盖；
  `"{}工具"` 的参数在英文态下读到的就是译后的工具名。
- Rust 侧闭集显示标签已覆盖：总览面板曲风档名
  （`song_overview::profile_display_name_for` 按 `host::current_language()`
  取名；`host::apply_language` 记录当前语言并在切换时用
  `relabel_profile_display` 重标已显示的那一个）。其余 Rust 侧文案仍按
  §4 排除（状态/诊断类），`.slint` 侧只译其框架词（如
  `@tr("音频引擎提示: {}", root.engine-error)`，参数原文不译）。
- 收尾补记（2026-11-17）：证据帧复核发现英文态状态栏残留两处中文——
  选区格（`选区 无` / `选区 … – …`）与音频配置格（`… · 目标 120 FPS`），
  都是 `host::publish_status_bar` 写出的**常驻读数**。裁定：状态栏是常驻
  UI 显示、不是日志，须译（§1.2 的诊断类归类只适用于瞬时提示）。修法照
  档名先例走宿主闭集：写点按 `host::current_language()` 取词（空选区英文
  常量 `STATUS_SELECTION_EMPTY_EN` 与 .po 中 `选区 无` 的 msgstr 逐字符
  对齐，单测对账；设备格整串仍由 bridge 投影期拼一次，写点只换
  `目标 / Target` 目标标签后缀，单测拿全部采样率 × 位深组合的真实投影
  串验漂移），`apply_language` 切换时用 `relabel_status_selection` /
  `relabel_status_device` 重标已显示的两格（认不出的形态保持原值）。
- 覆盖判据落地为 `crates/yeban-app/tests/i18n_coverage.rs`（三层：未路由
  硬编码红 / 词表集合精确相等且无空译 / 合成变异自检）。外部变异实测：
  往 status_bar.slint 加一条假硬编码中文 ⇒ 判据红并点名
  `ui/status_bar.slint:278`；删一条 .po 词条 ⇒ 判据红并点名缺条；两者
  恢复后全绿。
- 与 line/ui-skeuo-theme 的预期 rebase 冲突面：transport_bar.slint 右侧
  开关岛（拟物线在同区加质感开关，本线加 Group L 语言开关，同区两开关
  需人工排序）、app.slint 的 MainWindow 属性/回调区（两线各加属性与回调，
  文本相邻）、host.rs 的 apply_* 与回调接线区（两线各加接线）。其余文件
  两线基本不相交；translations/ 与 tests/i18n_coverage.rs 为本线独有。

## 10. 九语扩展与持久化补记（2026-11-17，Wang 05:31 / 05:57 两令）

- 语言：zh-CN（源）＋ en-US、fr-FR、es-ES、pt-BR（巴西变体）、ja-JP、
  ko-KR、ru-RU、de-DE，共九语八份目标词表，各 565 条、零空译（判据 ②
  八语对账＋逐语删 `选区 无` 变异点名）。
- 词表翻译：七份新词表各由一轮翻译写成（msgid 集与 en-US 逐条对齐、
  占位符 `{}`/`%s` 两侧一致；与英文相同者逐语 17–21 条，逐条核查均
  为行业借词或同形词，如 Piano Roll / Reverb / Preset / Clip /
  Transport，无懒复制）。
- 宿主闭集九语化：`Language` 九变体（规范名、别名、自称名、bundled
  selector）；状态栏选区格（空选区九串、前缀九串、重标九语互认）与
  音频配置格（后缀九串，含日语 `目標` 与中文 `目标` 简繁同义不同形）；
  总览曲风档名九语表与重标。九语单测看守（host.rs 三条、
  song_overview.rs 两条）。
- 切换入口：Group L 两态开关改为九语弹出菜单（按钮显示自称名，菜单
  9 项 × 32px，当前项打勾）；主菜单新增「设置」菜单列九语（当前项标
  题前缀 `✓`）；两处同调 `select-language(string)`，宿主
  `apply_language_and_save` 应用＋写回。
- 持久化与启动：新建 `src/settings.rs`，设置文件 `yeban/settings.json`
  （平台配置目录，见 §3；格式 `{"version": 1, "language": "fr-FR"}`）。
  启动优先级 `--lang` ＞ 已存选择 ＞ 系统检测 ＞ zh-CN（纯函数
  `resolve_startup_language` 单测钉死；检测读 `LC_ALL` /
  `LC_MESSAGES` / `LANG`，见 §3 缺口登记：Windows 检测落回 zh-CN）。
- 证据帧：`examples/i18n_shots.rs` 出六帧（zh / en / fr / ja / ru /
  zh-back，1600×900），fr/ja/ru 三帧须全对应语言无中文残留（含状态
  栏两格），往返切回 zh 逐像素无损；取证运行的设置写回指到输出目录
  临时文件。
- 后续线：窗口布局持久化**不在本线**。合入后父 agent 另开
  line/ui-window-state，用同一设置存储做窗口几何＋面板布局持久化、
  主菜单「设置」加「恢复默认布局」。（已落地，见 §11。）
- 已登记未修：① 「均分高度」按钮英文只显 'Even'（布局裁切，译文完整）；
  ② Windows 首次启动语言检测落回 zh-CN（见 §3）。

## 11. 窗口布局持久化补记（line/ui-window-state，2026-11-17）

- 存储演进：仍是同一个 `yeban/settings.json`、同一个 `src/settings.rs`。
  格式在 `version: 1` 内加键扩展 —— 新增可选键 `layout`（对象），
  `version` 不变。读者忽略未知键：旧构建读新文件 ⇒ 语言照读、布局
  忽略；新构建读旧文件 ⇒ `layout` 缺失即"没有已存布局"。保存走结构
  化重建（`Settings` 同时持有 `language` 与 `layout`），两条写回路径
  （`save_language` / `store_layout`）都是读-改-写，互不连坐。
- 纳入的布局量（普查自 `ui/app.slint`，只纳真实存在、可往返的）：
  窗口几何（x / y / width / height，物理像素，＋最大化标记）；
  五条可拖拽尺寸（左栏宽、片段矩阵宽、控制室宽、底部坞高、轨道头宽，
  钳位界与 .slint 拖拽界同一组数）；四个面板显隐（左检查器、片段
  矩阵、底部坞、控制室）；左栏标签页（inspector / browser）；底部坞
  控制台标签页（0–3）与全高开关；侧栏折叠。
- 未纳入（附理由）：轨道/片段/自动化泳道高度（随工程的数组状态）；
  自动化泳道显隐（编排视图选项）；全曲总览折叠（由窗口尺寸派生）；
  总览全屏、命令面板、热键浮层、和弦轨、瞬时提示（瞬时模式）；
  控制台高度读数（读数不是用户拖拽量）；侧栏分类/选中项/搜索词
  （内容选择状态）；左栏总线条显隐（静态装饰开关）。
- 坏数据降级（逐字段，`settings.rs` 单测钉住）：尺寸缺失/类型错 ⇒
  默认值，有数字 ⇒ 钳回拖拽界；标签页只认闭集、控制台标签只认
  0–=3，界外落回默认；窗口几何四数全有且在理智界内才整份采纳
  （半份几何可能把窗口丢到屏幕外）；`layout` 不是对象 ⇒ 当作没有
  已存布局。语言字段在任何布局损坏下不受影响。
- 恢复与写回：`main.rs` 在 `apply_language` 之后、事件循环之前经
  `host::apply_window_layout` 恢复（没有已存布局则不调用，.slint
  初值含 persona 派生的显隐初值不被覆盖）；事件循环正常返回后经
  `host::save_window_layout` 快照写回（崩溃退出不写回，下次用上上次
  布局）。`--headless-idle` 见证路径不经恢复/写回（固定尺寸测量）。
  最大化时 Slint 不暴露还原几何，快照里 size 是最大化尺寸，恢复以
  `maximized` 标记为准。
- 恢复默认布局：主菜单「设置」菜单末尾（语言九项之后）加
  「恢复默认布局」（`@tr` 新条，八份目标词表已补译，覆盖判据对账
  566 条）。宿主 `host::reset_window_layout`：面板回 .slint 初值、
  窗口取消最大化并回 preferred 1600×920（位置不动）、已存布局清掉
  （`store_layout(None)`），语言选择不经此路、不受影响。
- 判据：`tests/window_layout_state.rs` 三条（应用↔快照往返、恢复
  默认全链路含语言保留、.slint 初值↔settings 默认值/钳位界对账）
  ＋ `settings.rs` 单测五条（布局往返、旧文件兼容、逐字段降级、
  几何好值保留、读-改-写互不连坐）；窗口位置在软件测试端口无位置
  概念，只由 settings 存取往返钉住，桌面端由 winit 承载。

## 12. 工程模板 i18n 补记（line/project-templates，2026-11-17）

- 模板线（基线 c58c70369，早于 i18n 落地）rebase 到 main d10ab3867 后
  补齐：文件菜单两项（`从模板新建...` / `另存为模板...`）与两个对话框
  （`dialogs/template_chooser.slint` / `dialogs/save_as_template.slint`）
  的全部静态文案改走 `@tr`，新增 msgid 27 条，八份目标词表逐条补译
  （覆盖判据对账 566 → 593 条，零空译）。截断提示行的拼接字面量改
  `@tr("还有 {} 行未画出（列表一次最多画 64 行）", …)` 占位符形态。
- 宿主闭集第四处：选择器来源分组标题（工厂模板 / 我的模板）走
  `templates::TemplateSource::group_label_for` 按语言取名（九语单测在
  `src/templates.rs` 内逐条钉死）；行模型在打开选择器时按
  `host::current_language()` 构建，曲风 / 子曲风 / 模板名是用户内容、
  不译。
- 仍按 §4 排除（本期不译）：宿主写入的瞬时提示与诊断——另存为模板的
  分类提示行（`已有分类：…`，内容是用户分类名）与失败原因行
  （`templates.rs` 的校验 / 落盘错误串）。对话框内的静态框架词已全走
  `@tr`（含失败行的 accessible-label `另存为模板失败的原因`）。
