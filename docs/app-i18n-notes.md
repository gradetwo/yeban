# 夜半桌面 UI 双语 i18n 设计与实施记录（line/ui-i18n）

负责人定案（Wang 2026-11-17）：夜半开分支完成双语 i18n。语言对 = zh-CN（默认）＋ en-US；
运行时可切换、不重启生效。产品口径对齐偃师 web 端 i18n：单一词表按中文原文精确匹配、
默认中文、英文显式切换、开关在顶栏、日志与诊断不翻译、覆盖判据守漏词条。

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
界面静态文案全部住在 .slint 侧，本期范围即 §1.1 的 585 条。

## 2. 机制选型

### 候选 A：Slint 内置 `@tr()` ＋ bundled translation（选定）

- build.rs：`CompilerConfiguration::with_bundled_translations("translations")`
  ＋ `with_default_translation_context(DefaultTranslationContext::None)`（关掉默认的
  组件名上下文 ⇒ 词表按中文原文单一键，与偃师口径一致）；
- 词表文件：`crates/yeban-app/translations/en-US/LC_MESSAGES/yeban-app.po`
  （域名 = CARGO_PKG_NAME = `yeban-app`，编译期由 rspolib 解析、逐串编进二进制）；
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

### 与 UiText/Tokens 体系的冲突面

`@tr()` 返回 `string`，与现有 `text:`/`title:` 等属性类型完全兼容，UiText 只是样式
组件、不经手文案，零冲突。Tokens 全局不新增语言状态（语言状态归宿主 Rust 持有，
经 MainWindow 属性下发给走带栏开关），tokens.slint 一行不改——theme_selection 判据
⑤ 对 tokens.slint 的字面量钉不受影响。

## 3. 切换入口与持久化

- 入口：走带栏右岛、Group K（TIPS 开关）之后新增 **Group L：语言开关**，
  照 Group K 的小开关形态（文字即当前态：中文态显示「中文」，英文态显示「EN」）。
  选走带栏的理由：与质感开关（拟物线在同一位置的先例）同为全局显示偏好，
  顶栏右岛已有成例（主题/质感/提示开关同区），用户找语言开关的心智位置一致；
  设置区在本应用里不存在成形的面板，不为一个开关新开面板。
- 接线：`TransportHeaderBar.language-en` 属性＋`toggle-language` 回调 →
  `MainWindow.language-en` 属性＋`toggle-language` 回调 → host.rs 处理器调用
  `select_bundled_translation` 并回写属性。另加 CLI `--lang <zh-CN|en-US>`
  （照 `--theme` 的解析与拒绝形态），启动时在建窗后显式应用一次。
- 持久化：**本仓没有用户设置持久化路径**（全仓无设置文件/配置目录读写，
  主题与质感同样是 CLI＋会话内开关）。本期只做会话内切换＋`--lang` 启动选择，
  在此明写；持久化待仓内有统一设置存储后一并接入，不在本期自造一份。

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
2. 词表完整性：源码中 `@tr` 的全部 msgid 集合必须与 en-US.po 的 msgid 集合**相等**
   （漏译、孤儿词条都红）；
3. 变异验证：随手加一条假硬编码中文串 ⇒ ① 红；删掉 ⇒ 绿（实施时实测并记回执）。

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
