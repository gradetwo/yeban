# UI 线 · `line/ui-modules` 笔记

## 0. 这一票要补的口子（**先核查，后动**）

用户的指认是：`ui/*.slint` 里已有 `export` / `open` / `save` 等回调，但**没有** SFZ 装载、
录音、导入 MIDI 的入口。本票先按 `AGENTS.md` §6.3「不要用词面判断交付」核查已有的
`export-*` 系列**实际**做了什么。

### 核查结论（带证据）

| 问题 | 实测证据 | 结论 |
| :--- | :--- | :--- |
| `ui/app.slint` 的 `export-diagnostics` 导出什么格式？ | `crates/yeban-app/src/host.rs:2869`（改前）`ui.on_export_diagnostics(move \|\| { port.perform(UiAction::ExportDiagnostics) })`；`crates/yeban-app/src/undo.rs:470` 落到 `yeban_diagnostics::export_diagnostics(&dir, inputs)` | **不是** WAV。是 env / git / 日志的诊断 **zip**（`[D56]`） |
| 界面里有没有 WAV 导出？ | `grep -rn "wav\|WAV" crates/yeban-app/ui/` 只命中 `transport.slint` 的录音键注释与 `tokens.slint` 的 `record-red` 色标 —— **零** WAV 入口 | **没有**。`--export-wav` 只有 CLI |
| 界面里有没有录音入口？ | `elements.rs` 的 `the_record_button_has_no_click_handler_and_recording_is_unimplemented` 把 `transport-record-button` 的**无点击处理器**钉成机械事实（`rec_area` 无 `clicked`、`transport.slint` 不声明 `callback record`、`app.slint` 不转发 `record =>`） | **没有**。而且既有的那个键承载的是 `[ARCH-REC-001]` 的**声卡**录音缺口，与 `record_wav.rs` 的"控制侧测试音录成 WAV"**不是同一件事** |
| 界面里有没有 SFZ / MIDI 导入？ | `grep -rn "sfz\|midi" crates/yeban-app/ui/`：`sidebar.slint:29` 只有一条**演示字符串** `"SFZ · 323 款采样预置"`（不是入口）；`piano_roll.slint` 的注释提到 MIDI 只是"这是 MIDI 卷帘" | **没有** |

⇒ 四个模块的 **Rust 侧实现都已存在**（`export_wav.rs` / `import_midi.rs` / `sfz_load.rs` /
`record_wav.rs`），缺的只有**界面入口**。本票**不重做**已覆盖的部分，只补真缺的四条。

### 边界的第二条：为什么**不改**顶栏的录音键

`elements.rs` 的两条判据把顶栏的 `commit` / `revert` / `branch` / `record` 四个键的
"无点击处理器"钉住了。本票**不**动它们（动它们 = 改 `[ARCH-REC-001]` 的能力裁决 +
改判据 + 可能重录基准），而是给**已有的** `record_wav.rs` 能力开一个**新的、独立的**
可见入口。两条各自留痕。

**（2026-10-31 附记，`line/record-key`）** 上文"顶栏录音键不点"是 #88 之前的事实。`#88`
把 `host::wire_record_at` 接上之后，顶栏 `transport-record-button` 已经落到**同一**回调
（`src/elements.rs` 的 `the_record_button_has_a_click_handler_and_a_wired_record_callback`
逐字钉着 `rec_area` 的 `clicked` 必须落到 `root.toggle-record()`）。本切片再补**键盘入口**：

- **键位**：裸 `R`（`Action::ToggleRecord`）。三条事实：① 裸 `r` 此前在
  `resolve_logical` 的任何一步里都不存在（实测）；② `command()` 分支里也没有 `r` ⇒
  `Cmd/Ctrl+R` 仍**放行**（"重载 / 刷新"是另一套界面决定，⛔ 不顺手发明）；
  ③ 文本框聚焦时整体放行（策略表第 2 步）—— 与裸 `M` / `S` 同一口径。
- **落点只有一个**：`host::apply_action` 的 `ToggleRecord` 臂只做
  `ui.invoke_toggle_record()` —— 与「录音」按钮是**同一个**回调（`wire_transport` →
  `wire_record_at`：设备源 / 预录静音 / 停录落盘 / 回执全在那里）。⛔ 没有第二条录音路径。
- **失败口径**：起录失败由既有 `RecordOutcomePort` 如实回报；录音器正被同一线程借用时
  `wire_record_at` 打一行 stderr 并令本次无效（⛔ 不 panic）⇒ 本臂恒消费这一键。
- **判据**：`tests/record_key_ui.rs`（活窗口上装根回调计数器：裸 `R` ⇒ 回调**恰好**一次、
  连按递增；`Shift` / `Ctrl` / `Alt` 变体放行且不惊动回调）；策略表新行由
  `tests/cli_contract.rs` 的 B11b 三面一致（表 / 解析 / 宿主）钉住；`src/input.rs` 另有
  解析级判据 `bare_r_toggles_record_and_does_not_disturb_its_neighbours`。

## 1. 新增入口与 Element ID 清单

| 入口（可见文案） | Element ID | 角色 | 落点（Rust） |
| :--- | :--- | :--- | :--- |
| 小节标题「模块入口」 | `module-actions-title` | text | —— |
| 导出 WAV | `module-export-wav-button` | button | `export_wav::export_wav_with_sfz_file` |
| 导入 MIDI | `module-import-midi-button` | button | `import_midi::import_midi_file`（经 `UndoPort`） |
| 装载 SFZ | `module-load-sfz-button` | button | `sfz_load::load_sfz_with_instrument` |
| 录音（测试音） | `module-record-wav-button` | button | `record_wav::record_test_tone` |
| 回执行 | `module-last-status` | text（动态区） | 宿主写 `MainWindow.module-status` |

**新增入口数 = 4**（按钮）；新增语义 ID = **6**。

新增文件：`crates/yeban-app/ui/module_actions.slint`（组件 `ModuleActions`）。
它在 `app.slint` 的 `AiRail` 里实例化于 **y = 212..384** —— 既有三条控件止于 y = 204，
因此**没有移动任何既有控件**（golden 的差异因此只应落在这一块新像素上）。

## 2. 目标路径从哪来（本票的**边界**）

宿主侧唯一解析点 = `host::ModulePaths::from_env()`，四个环境变量：
`YEBAN_UI_EXPORT_WAV_PATH` / `YEBAN_UI_IMPORT_MIDI_PATH` / `YEBAN_UI_LOAD_SFZ_PATH` /
`YEBAN_UI_RECORD_WAV_PATH`，秒数 = `YEBAN_UI_RECORD_SECONDS`（默认 8.0）。
缺省值落在**当前工作目录**下（`ui-export.wav` 等）—— ⛔ 不落系统临时目录
（那里的随机分量会让回执不可复现，而回执里的路径正是判据要断言的东西）。

⛔ **没有**原生文件对话框：用户不能在这里挑文件。那是**另一票**（需要 platform 侧的新能力）。

## 3. 回执格式（成功与失败**同一条**路径）

```text
module:action=export-wav:outcome=ok:path=…:frames=384000:sample_rate=48000:channels=2:bits=24:bytes=…:source=…:digest=…
module:action=load-sfz:outcome=error:error=sfz-load:message="读 SFZ 失败: …"
```

错误码是稳定的：`export-wav` / `import-midi` / `sfz-load` / `record-wav`（模块自身拒绝）、
`no-project`（没有工程读数）、`no-undo-port`（没有撤销会话，导入拒绝执行）。

## 4. golden 影响（**已按裁决重录**，见 §4.2）

本票给右栏下半区加了 6 个**静态**语义节点（4 个按钮 + 小标题 + 回执行）⇒ 默认帧像素改变
⇒ 5 张 Linux 基准**确定性过期**。R9 的状态栏那一笔已在 `a062c38` 重录过（MANIFEST 第 60-68 行），
所以本次的不一致**不是**继承的旧漂移 —— 归属由父提交的判决钉住：
**父提交 `b3f9f805` 自己的 run `38134313408` 的 `rust (yeban-app)` = success（steps=12，test 步跑过且绿）**。
本机是 macOS（`PlatformTag::current()` 指向仓库里**不存在**的 `golden/macos/`），
且 `AGENTS.md` §5.2 禁止在本机编译 slint ⇒ **生成只能在 CI（Linux）**。

### 4.1 新增控件的**实测**包围盒（判据自己打印的，⛔ 不是手算）

来源：`tests/ui_module_actions.rs` 的 `report_line`（`--nocapture` 可读；
默认帧尺寸 1920×1080，右栏起于 x = 1640）：

```text
[ui-modules] 运行时控件树 92 条；新入口 6 条如下:
[ui-modules]   module-actions-title        role=text   bounds=x 1648 y 260 w 272 h 16
[ui-modules]   module-export-wav-button    role=button bounds=x 1648 y 284 w 264 h 28
[ui-modules]   module-import-midi-button   role=button bounds=x 1648 y 316 w 264 h 28
[ui-modules]   module-load-sfz-button      role=button bounds=x 1648 y 348 w 264 h 28
[ui-modules]   module-record-wav-button    role=button bounds=x 1648 y 380 w 264 h 28
[ui-modules]   module-last-status          role=text   bounds=x 1648 y 416 w 264 h 16
```

⇒ **重录后的复核口径**：5 张基准与旧图的差异必须**全部落在 `x ∈ [1648, 1912)`、
`y ∈ [260, 432)`** 这一个矩形内，矩形之外**逐像素相同**。
（`AiRail` 的画布起于 `y = 252`：顶栏 48 + 中段第一个子控件 204。）

### 4.2 重录与复核（**已做**，2026-10-11）

- 生成：手动档 `gates-manual.yml` 的 `gate=goldens`，**run `38135994642`**（conclusion=success；
  job `goldens (生成分平台基准图集)` success steps=8，11:33:27Z–11:36:15Z；该步
  `YEBAN_WRITE_GOLDEN=1 cargo test -p yeban-app --locked --test real_ui_tier1` 报
  `22 passed; 0 failed; 1 ignored`）。artifact `golden-linux`（id 11701170437，476540 字节）。
- 入库：5 张 PNG 覆盖 `crates/yeban-app/tests/golden/linux/`，表由
  `python3 scripts/gates/check_golden_manifest.py --write-table` 从磁盘重写。
  **artifact 自带的那份 `MANIFEST.txt` 与仓库副本逐字节相同**（= 改动前的旧哈希）⇒ **未采用**。
- 复核（判定量 = **矩形外不一致像素数**；量法 = Pillow 解码成 1920×1080 RGB 后 numpy 逐通道比较）：

| 基准 | 差异像素 | 差异包围盒 | 矩形外 |
| :--- | ---: | :--- | ---: |
| `app-main-window-arrangement-compact-1920x1080.png` | **0**（逐字节相同） | —— | **0** |
| `app-main-window-arrangement-full-1920x1080.png` | 29,849 | (1648, 263)..(1911, 407) | **0** |
| `app-main-window-session-full-1920x1080.png` | 29,849 | (1648, 263)..(1911, 407) | **0** |
| `app-model-driven-demo-project-1920x1080.png` | 29,849 | (1648, 263)..(1911, 407) | **0** |
| `app-model-driven-filled-project-1920x1080.png` | 29,849 | (1648, 263)..(1911, 407) | **0** |

⇒ **`RECT_OUTSIDE_DIFF_TOTAL = 0`**。compact 那张**零差异**是预期的：compact 断点下右栏宽 0
（改由抽屉承担）⇒ 新入口不画 ⇒ 它的 sha256 与上一版**一位未改**（所以 `--write-table` 只改写了 **4** 行）。
⚠ 5 张的**字节数全等**（6,222,418）**不构成证据**（本仓 PNG 用存储式 deflate，1920×1080 恒为该长度）
—— 判内容的是 sha256 与上面的逐像素读数。
- 脚本：`/tmp/verify_rect.py <old_dir> <new_dir> 1648 1912 260 432`（旧图由
  `git show <重录前提交>:crates/yeban-app/tests/golden/linux/<name>.png` 导出）。

## 5. 读数（本机 macOS / Apple M2，debug 构建，命令可复跑）

指标口径（先说单位）：**每一次动作在 UI 线程回调里的墙钟毫秒**，取 `Instant::now()`
与动作返回之间的差；由 `host::module_trace` 在 `YEBAN_UI_MODULE_TRACE=1` 时打到 stderr。
它**只**在打开该变量时执行，默认关闭 ⇒ 产品与既有判据的确定性不受影响。

```bash
cd .worktrees/ui-modules
BIN=$(ls target/debug/deps/ | grep -E '^ui_module_actions-[0-9a-f]+$' | head -1)
for i in 1 2 3 4 5; do
  YEBAN_UI_MODULE_TRACE=1 RUST_MIN_STACK=16777216 target/debug/deps/$BIN --nocapture
done 2>&1 | grep -o 'action=[a-z-]* elapsed_ms=[0-9.]*'
```

**5 次实测（毫秒）**：

| 动作 | 5 次读数 | 中位数 |
| :--- | :--- | ---: |
| `export-wav`（引擎乐器，8 秒 / 48 kHz / 24-bit） | 346.360 / 340.018 / 351.218 / 341.469 / 345.426 | **345.426** |
| `export-wav`（**已装载 SFZ** 之后的同一次导出） | 177.433 / 180.492 / 179.381 / 182.374 / 178.263 | **179.381** |
| `import-midi`（最小 SMF，1 个音符） | 0.294 / 0.301 / 0.305 / 0.335 / 0.234 | **0.301** |
| `record-wav`（1 秒 / 48 kHz / 24-bit） | 27.144 / 27.000 / 28.215 / 27.340 / 26.002 | **27.144** |
| `load-sfz`（最小 SFZ，1 个区域 + 1 次探测） | 1.757 / 1.329 / 1.426 / 1.427 / 1.319 | **1.427** |
| `load-sfz`（源不存在 ⇒ 错误臂） | 0.037 / 0.056 / 0.088 / 0.047 / 0.024 | **0.047** |

**对照**（用户给的 CLI 读数，同一台机器）：`--export-wav` = **261.4 ms**；
`--sfz <f> --export-wav` = **138.4 ms**。⇒ 经 UI 触发的那两次导出**都慢约 30%**
（345.4 vs 261.4；179.4 vs 138.4）。本报告**不解释**这个差值（可能是 GUI 进程的
实时引擎线程、心跳定时器与 Slint 事件循环在同一进程里争 CPU），只报数。

### 5.1 峰值 RSS（口径见 `scripts/gates/measure_rss.py` 的文件头）

对象 = **一个进程**从启动到退出期间被 `wait()` 到的峰值 RSS（`ru_maxrss` 高水位；
macOS 报字节，脚本已归一到 MB）。它**不是**稳态空闲值，也**不是**"这四个动作的增量成本"
—— 进程地板（Slint + 引擎 + 字体）占大头。同二进制内消融（本仓既有方法）：

```bash
python3 scripts/gates/measure_rss.py --label ui-module-actions-4entries \
  -- target/debug/deps/ui_module_actions-<hash> --nocapture
python3 scripts/gates/measure_rss.py --label ui-module-actions-floor-no-render \
  -- target/debug/deps/ui_module_actions_no_port-<hash> --nocapture
```

| 消融档 | 这个进程做了什么 | 峰值 RSS |
| :--- | :--- | ---: |
| `ui-module-actions-4entries` | 活窗口 + 撤销会话 + **四个动作全跑**（含两次 8 秒 WAV 导出） | **65.12 MB** |
| `ui-module-actions-floor-no-render` | 活窗口 + **没有**撤销会话、**没有** WAV 导出（只做装载 / 录音 / 两条拒绝臂） | **56.11 MB** |

⇒ 差值 **9.01 MB** 是"撤销会话 + 两次 8 秒离线渲染（含 2.3 MB 输出缓冲两次）"这一块。
⚠ 两个档都不是 `BASELINE-002` 的对象（那是**空工程空闲**常驻内存，目标 ≤ 35 MB）
⇒ 这两行**不得**读成"BASELINE-002 达标/未达标"，脚本诚实地打了 `over-target` 只因为
它按那个阈值判。

## 6. needs / pending（如实登记，⛔ 不假装整条通）

1. **没有原生文件对话框**：四个入口的目标路径只能由宿主的环境变量给出
   （`YEBAN_UI_*_PATH`）。用户**不能**在界面上挑文件 / 挑目录。这是**另一票**
   （需要新的 platform 侧能力；本票的边界）。
2. **`record-wav` 是"控制侧测试音录成 WAV"**，⛔ **不是**声卡录音。
   `[ARCH-REC-001]` 要求的"点击录音 Arm 后从声卡物理输入捕获 PCM"在仓库里**仍不存在**，
   由 `elements.rs` 的 `the_record_button_has_no_click_handler_and_recording_is_unimplemented`
   继续钉着（顶栏 `transport-record-button` 本票**一位未改**）。
3. **本机 macOS 的 `engine_device_leg` 栈溢出**（既有测试，与本票改动无因果关系证据）：
   `cargo test -p yeban-app --all-targets` 在默认 8 MiB 线程栈下，
   `the_device_leg_carries_the_transport_position_of_the_control_driven_engine`
   报 `has overflowed its stack`（SIGABRT）。加 `RUST_MIN_STACK=16777216` 后
   **同一条测试通过**（5 passed）。本票**没有**验证"基线（未含本票改动）是否同样溢出"
   （那需要另一次全量构建）⇒ 登记为**未判定**，⛔ 不声称是本票引入的、也不声称不是。
   本票的四门因此用 `RUST_MIN_STACK=16777216` 跑（见 §5 的命令）。
4. ~~**golden 未重录**（见 §4）：5 张 Linux 基准仍需按手动档 `gates-manual.yml` 的
   `gate=goldens` 重录 + 人工复核。~~ ⇒ **2026-10-11 已结清**：run `38135994642` 重录、
   逐像素复核矩形外 = 0（见 §4.2）。
5. **`ui-mcp` 的 `module-*` ID 未进 `VISIBLE_SINGLETONS`**：那是 `src/test_port_adapter.rs`
   的一份分母清单（判别"整块面板被裁掉"）。本票**没有**动它 —— 新入口的"可寻址"由
   `tests/ui_module_actions.rs` 自己断言（点名的 6 个 ID + 运行时树对注册表的双向覆盖）。
   不把它塞进那份分母的理由：那份分母的缺失容忍度是 10%（39 × 10%），加 6 条会把
   容忍度摊薄到 13%；而本票的 6 条**已经被**更强的判据（精确点名）覆盖。

## 7. 后续更正（本次切片，2026-10-31，`line/ai-drawer-key`）—— AI 抽屉有了键盘入口

**缺口**：走带右上角的 AI 抽屉开关 `transport-ai-drawer-button` 只有鼠标 / 无障碍默认动作
两个入口，键盘面一格都没有。本切片补的**不是**抽屉能力（`wire_view_callbacks` 早已把
`on_toggle_ai_drawer` 接到 `ai-drawer-open`），而是**键盘入口**。

### 7.1 键位与理由（界面决定，⛔ 不是规范点名）

`Cmd/Ctrl+Alt+A` —— `a` 取 "AI"。三条事实（逐条实测，⛔ 推断）：

1. `command()` 分支里 `Cmd/Ctrl+Alt` 当时只有 `b` / `m` / `n` / `d` / 四个方向键
   ⇒ `a` 那格是空的（落到 `_ => PassThrough`）；
2. 裸 `a` 在第 6 步从未绑定；`Alt+A` 在第 4 步也没有（那里只有 `1` / `2` / `←` / `→`）；
3. `Cmd/Ctrl+Alt+Shift+A` 仍放行 —— 与"规范没写的不猜一个相近语义"这条纪律一致。

### 7.2 唯一落点

`host::apply_action` 的 `ToggleAiDrawer` 臂 ⇒ `ui.invoke_toggle_ai_drawer()` ⇒
`host::wire_view_callbacks` 注册的**同一条** `on_toggle_ai_drawer`（写 `ai-drawer-open`）。
键与按钮共用这一个回调，⛔ 不是第二份实现。纯视图态（`MODEL-ISO-001` 挥发性层）⇒
不改工程、不需要 `UndoPort`、**不在** `action_needs_engine` 集合里（无 `#212` 重入面）。

### 7.3 判据

| 层 | 文件 | 钉的事 |
| :--- | :--- | :--- |
| A 纯解析 | `src/input.rs` 的 `ctrl_alt_a_is_the_ai_drawer_entry_and_does_not_disturb_its_neighbours` | 物理码 / 逻辑键 / `Cmd` 侧三条入口同解；邻居与文本域放行 |
| B 活窗口 | `tests/ai_drawer_key_ui.rs` | 按键 ⇒ `ai-drawer-open` **真的翻转**；换计数器后回调恰好一次；邻居不消费 |
| 表 / 宿主对账 | `tests/cli_contract.rs` 的 B11b | `cli.rs` 的 `SHORTCUTS` 表与 `--print-shortcuts`、`action_has_implementation` 三面一致（表条数由该判据的 `rows.len()` 断言，⛔ 不在这里手抄） |

## 8. 后续更正（本次切片，2026-10-31，`line/ai-drawer-key`）—— 键盘能换轨了

**缺口**：编曲区的选中轨**只能靠鼠标点轨道行**换（`selected-track-id` 的唯一写者是
`host::wire_track_select`，入口是 `arrangement_view.slint` 的轨道头与无障碍默认动作）。
键盘面没有任何选轨入口 ⇒ 键盘-only 用户被困在投影兜底的第一条轨上。

### 8.1 键位与理由（界面决定，⛔ 不是规范点名）

`Alt+↑` / `Alt+↓` —— 三条事实（逐条实测，⛔ 推断）：

1. `resolve_logical` 第 4 步（`Alt` 分支）当时只有 `1` / `2` / `←` / `→`，`↑/↓` 落在
   `_ => PassThrough` ⇒ 加它不覆盖任何既有绑定；
2. 裸 `↑/↓`（网格 / 半音微调）、`Shift+↑/↓`（八度）、`Cmd/Ctrl+Alt+↑/↓`（音量/声相）
   与 `Alt+←/→`（1 tick）的既有语义一格不动（`input.rs` 与 `cli_contract` 两处钉住）；
3. 文本域聚焦时 `Alt` 分支先被 `Focus::TextInput` 策略拦住 ⇒ 输入框里不会误换轨。

### 8.2 唯一落点

`host::apply_action` 的 `SelectTrackBy` 臂读窗口上的 `track-ids`（生产注入的身份表，
⛔ 不拿装配期快照猜）算出邻居下标，只做 `ui.invoke_track_select(target)` ⇒ 与鼠标点
轨道行是**同一条** `host::wire_track_select`（写 `selected-track-id` + 按当前横向偏移
重投影）。选中态是**视图态**（`MODEL-ISO-001` 挥发性层）⇒ 不改工程、**不进撤销栈**。

边界：`track-ids` 为空 ⇒ 不消费；当前没有选中轨 ⇒ 两向都落到第一条；顶到两端 ⇒ 仍消费
（下发夹紧后的同一条下标，回调自己的"同一条轨不重投影"早退）。

### 8.3 判据

| 层 | 文件 | 钉的事 |
| :--- | :--- | :--- |
| A 纯解析 | `src/input.rs` 的 `alt_up_and_down_move_the_track_selection_and_do_not_disturb_its_neighbours` | 物理码 / 逻辑键两条入口同解（含方向）；裸键 / `Shift` / `Cmd+Alt` / `Alt+←→` 保持原义；文本域放行 |
| B 活窗口 | `tests/track_select_keyboard.rs` | 走完选中态与两端停住；回调身份与夹紧（记录器读到 `2, 2, 1, 0`）；邻居组合不动选中态；空工程不消费 |
| 表 / 宿主对账 | `tests/cli_contract.rs` 的 B11b | 两行新快捷键（`Alt+↑` / `Alt+↓`）三面一致（表 / 解析 / 宿主） |
