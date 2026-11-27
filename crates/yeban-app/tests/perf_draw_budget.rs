//! `[BASELINE-003]` 绘制成本的**回归判据** —— 无 GUI、无 GPU，本机与 CI 都能跑。
//!
//! ## 这个文件存在的理由
//!
//! 2026-10-07 的实测把「10 万音符卷帘滚动」的绘制回调成本拆开了。结论是：整窗绘制回调
//! 的 p99 里，**钢琴卷帘两条 `for` 循环物化出的图元**与**每帧重写与滚动无关的属性**
//! 各占一大块。这两块此前都没有判据钉住 —— 谁把视口裁剪拆了、谁又加一个"每帧重写一遍"
//! 的数组，都要等有人手动跑 GPU 档才会发现。
//!
//! 本文件把这两块**钉成数量**。它不测时间（时间依赖机器与负载），它测数量：
//! 数量是确定的、可断言的、且与时间成本单调相关。
//!
//! ## 口径（三个断言数三个对象）
//!
//! | 断言 | 数的对象 | 单位 |
//! | :--- | :--- | :--- |
//! | ① | `ViewState::visible_notes` 在 `[BASELINE-003]` 三个探针帧上的返回长度 | 个音符 |
//! | ② | 运行时控件树里 `note-*-rect` 与 `velocity-*-bar` 的条数 | 个元素 |
//! | ③ | `host.rs::apply_view` 每帧路径上"换掉模型实例"的数组属性个数，及其源码写入点个数 | 个 |
//!
//! ## 实测（Apple M2 / macOS 27.0.1 / release / 10 万音符夹具）
//!
//! - 夹具：`yeban_model::samples::project_with_notes(100_000)`。音符 8 px 一个
//!   （240 tick ÷ 30 tpp）。
//! - 卷帘绘制图元 = **2N + 65**（N 个音符矩形 + N 个力度柱 + 65 个固定图元）。
//! - N：帧 0 = 241、帧 300 = 242、帧 599 = 242。⇒ 图元预算 = 2×242 + 65 = **549**。
//!   ⚠ 本节是**那一次**测量的快照。2026-10-30 诚实性批次删掉了 AI 建议层的静态示意块
//!   （没有提案数据源）⇒ 固定图元 **65 → 64**、预算 **549 → 548**；当前口径以
//!   `FIXED_ROLL_PRIMITIVES` / `ROLL_PRIMITIVE_BUDGET` 两个常量与下面第 ② 节的断言为准。
//! - 每个音符/力度柱的绘制成本 ≈ **2.15 µs**（逐项截断 A/B 的斜率），其中圆角 ≈ 1.6 µs。
//! - 把两条循环 `visible:false`：正式口径 p99 从 3.12–3.22 ms 降到 **1.73–1.84 ms**。
//! - 把 466 个矩形画成"最省的纯矩形"（方角 + 去边框）：p99 = **2.11 ms**。
//!   ⇒ **不改像素就达不到 2 ms**（这是 2026-10-07 上报的结论）。
//! - `apply_view` 每帧重写约 30 个与滚动无关的数组：值 **−0.4 ms p50 / −0.55…−0.6 ms p99**；
//!   连带把每帧创建的 FBO layer 从 **4 个降到 0 个**（`SLINT_DEBUG_PERFORMANCE` 读数）。
//!
//! ## 已批准、**尚未落地**的补丁（`crates/yeban-app/src/host.rs`）
//!
//! 状态：负责人已批准（逐像素不变，见下）。落地要等 `host.rs` 的写者顺序
//! （`AGENTS.md` §5.6 一文件一写者；当时 S1 线在该文件有未提交改动）。
//!
//! **改哪个函数**：`host::apply_view`（`src/host.rs:119`）与它调用的
//! `apply_row_geometry` / `apply_master` / `apply_meters`。
//!
//! **加什么比较**：写之前先跟窗口上的现值比一次；值没变就不写。
//!
//! ```ignore
//! let names = view.track_names();
//! if !same_strings(&ui.get_track_names(), &names) {
//!     ui.set_track_names(strings(&names));
//! }
//! ```
//!
//! `same_*` 是逐行比较的小工具（`Model::row_count` + `Model::row_data`）。需要 5 个：
//! `same_strings` / `same_lengths` / `same_booleans` / `same_integers` / `same_colors`。
//! 比较成本落在绘制回调**之外**。
//!
//! **随 `scroll_x` 变化、必须每次都写的属性**：`note-ulids`、`note-velocities`、
//! `note-positions`、`note-widths`、`note-ys`、`note-selected`（六个数组）与
//! `roll-scroll-x`、`roll-min-tick`、`roll-max-tick`、`roll-min-pitch`、`roll-max-pitch`
//! （五个标量）。补丁落地后本文件的 ③ 应收紧到 **7**（六个数组 + 一个标量）。
//!
//! **不随 `scroll_x` 变化、可以不写的属性**：其余全部 —— 轨道名与身份、行几何、
//! 轨道音量/声相/静音/独奏、色标、场景名、段落名与几何、剪辑的六个数组、小节线、
//! 自动化泳道的 9 个数组、三个电平数组、`apply_master` 的一批，以及
//! `window-title` / `bpm-display` / `timecode-ticks-*`。
//!
//! **为什么这个补丁逐像素不变**：不写 = 界面保持上一次注入的值；两条路径注入的值逐字
//! 相同，所以界面属性、图元几何、像素都不变。机械证据（Tier-1 软件光栅化，1920×1080，
//! 第 120 帧）：两条路径的 PNG sha256 都是
//! `e7796cb0c9086c46cc9227f47af004e18e95046ccc0a7f2bbdb7e8aa03eb721d`，`cmp` 逐字节相同。
//!
//! ## 判据怎么更新（重要）
//!
//! 三个常量是**预算**，不是"现状的描述"：
//!
//! - 变大 = 有人把成本加回去了 ⇒ **必须**先重新测量，再在提交信息里写出新读数；
//! - 变小 = 好事（补丁落地、或裁剪更紧）⇒ 可以收紧，并在提交信息里写出原因。

#![allow(unused)]
use slint::Model as _;
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;

/// `[BASELINE-003]` 的场景：10 万个音符。
const NOTE_COUNT: usize = 100_000;
/// `[BASELINE-003]` 的视口宽（逻辑像素，与 `DemoScene::viewport_width` 一致）。
const VIEWPORT_WIDTH: f32 = 1920.0;
/// `[BASELINE-003]` 的 600 帧里取样的三个帧号。
const PROBED_FRAMES: [usize; 3] = [0, 300, 599];
/// 三个探针帧上，视口裁剪之后的可见音符数。
///
/// 夹具里音符 8 px 一个、视口 1920 px ⇒ 可见数必须落在 240±16。
/// 视口裁剪一旦失效（回到"把 10 万个音符全物化"），这里会读到 100000 ⇒ 判据红。
const VISIBLE_NOTES_PER_FRAME: [usize; 3] = [241, 242, 242];
/// 卷帘两条 per-note 循环之外的固定绘制图元数。
///
/// 数法：1 个卷帘根矩形 + 5 个工具按钮 + 5 段工具文本 + 1 段吸附状态文本
/// + 1 个琴键列容器 + 16 个琴键 + 1 个网格容器 + 16 条横向车道
/// + 16 条纵向小节线 + 1 个力度泳道容器 + 1 个走带光标 = **64**。`TouchArea` 不绘制，不计。
///
/// ⚠ 2026-10-30 诚实性批次：AI 建议层的静态示意块已删除（没有提案数据源
/// ⇒ 那块"待采纳音符"是假的）⇒ 这一格 **65 → 64**；上游 `roll_primitive_formula…`
/// 的预算等式同步 **549 → 548**。
const FIXED_ROLL_PRIMITIVES: usize = 64;
/// 卷帘每帧绘制图元的预算上限 = `2 × max(N) + FIXED_ROLL_PRIMITIVES`。
const ROLL_PRIMITIVE_BUDGET: usize = 2 * 242 + FIXED_ROLL_PRIMITIVES;
/// 运行时控件树里的 `note-*-rect` 条数（中点帧，Tier-1 软件窗口，1920×1080）。
///
/// 它比注入的 242 少 9 个：完全落在父级裁剪矩形之外的图元不进运行时树。
/// ⚠ 这个数与上游（`i-slint-backend-testing`）的枚举规则有关。上游枚举规则一变，这条会红；
/// 那时必须重新测量图元数，不要直接把常量对齐过去。
///
/// ⚠ 2026-10-30 诚实性批次：**234 → 233**。被删的静态「AI 建议」块 id 是
/// `note-suggestion-overlay-rect` —— 它同时命中本判据的 `note-` 前缀与 `-rect` 后缀，
/// 所以一直算在这 234 里（它不是 per-note 图元，这一格曾经混进一个假成员）。
/// 删掉它之后运行时树少一个节点；`velocity-*-bar` 不受影响（仍 229）。
/// 这一格的变化来自**被测树真的少了一个节点**，⛔ 不是把常量对齐到实测值：
/// 上游枚举规则若变，仍按上面那段重新测量。
const RUNTIME_NOTE_RECTS: usize = 233;
/// 运行时控件树里的 `velocity-*-bar` 条数（中点帧）。
///
/// 力度柱的 x 比音符多 86 px（`56 + 12 + 30`），所以右端比音符先出界：229 比 234 少 5。
const RUNTIME_VELOCITY_BARS: usize = 229;
/// `apply_view` 每帧路径上"换掉模型实例"的数组属性个数上限。
///
/// 现在是全部（每次注入都新建 `VecModel`）。补丁落地后应收到 7。
///
/// 为什么钉"换掉模型实例"而不是"调用 `ui.set_*` 的次数"：Slint 的 repeater 只有在
/// **模型实例换了**（或某一行变了）时才重建子图元，而重建子图元就是那 0.4 ms 的来源
/// （实测：照常重建但内容冻结 = ±0.00 ms 差值）。
///
/// ## 30 → 31 的追加（任务单 `#82`，2026-10-16）
///
/// 新数组是 `track-sampler-paths`（SFZ 采样器绑定的路径文本）。实测：这一格在
/// `#82` 之前**恰好在预算上**（30），加上它之后是 **31**（判据会逐字打出整个清单）。
///
/// ⚠ **这一格的代价边界（如实说清）**：
///
/// * 它的消费者是 `MixerConsole` 通道条里的 `TextInput.text` —— 而混音台只在
///   `console-tab == 1` 时可见（默认帧是 Tab 0）⇒ 默认帧的 PNG sha256 逐位不变
///   （读数见 `APPLY_VIEW_WRITE_SITE_BUDGET` 的注释）；
/// * `apply_view` **不是** 60Hz 心跳那一跳（那条路走 `ProductionLoop::tick` ⇒
///   `pump_meters` + 按需 `publish_project`），它跑在"换工程 / 撤销 / 打开"这类
///   完整重投影上 ⇒ 多一次 `VecModel` 换实例的**频率**远低于每帧；
/// * ⛔ 墙钟成本本机没测（要 GPU 档，`AGENTS.md` §5 不允许）。上面给的是数量与像素读数。
///
/// 该文件头"已批准、尚未落地"的那条补丁（写之前先比一次）落地后，这一格与
/// `APPLY_VIEW_WRITE_SITE_BUDGET` 都会跟着收回去 —— 那时请一并改这两个常量。
///
/// ## 31 → 34 的追加（任务单 `#133`，2026-10-17）
///
/// 新数组是素材池面板的三个平行数组：`asset-pool-names` / `asset-pool-kinds` /
/// `asset-pool-details`（`ViewState::asset_pool` 的 `clip_pool` 投影）。实测（判据
/// 逐字打出整份清单）：这一格在 `#133` 之前 **31**，加上三个数组之后是 **34**。
///
/// ⚠ **代价边界（如实说清）**：
///
/// * 它们的消费者是 `console/asset_pool_panel.slint`，实例化在 `MixerConsole` 里
///   （`console_tabs.slint` 的 `visible: root.active-tab == 1`）⇒ 默认帧（Tab 0）的
///   运行时树**不含** `asset-pool-panel`（判据 `tests/asset_pool_panel.rs` 的 ① 段断言
///   这一点）。⛔ 本机是 macOS、`tests/golden/<platform>` 只有 `linux` ⇒ **未**做 Linux
///   基准的逐像素复测；这里只给"默认帧不实例化它"的结构性依据 + 控件树读数。
/// * 与 `#82` 同一路线：`apply_view` 只在"换工程 / 撤销 / 打开"这类**完整重投影**上跑，
///   不是 60Hz 心跳那一跳 ⇒ 多三个 `VecModel` 换实例的频率远低于每帧。
/// * ⛔ 墙钟成本本机没测（要 GPU 档，`AGENTS.md` §5 不允许）。
///
/// ## 34 → 38 的追加（任务单 `#157`，2026-10-17）
///
/// 新数组是设备效果链（控制台 **Tab 3**）的四个平行数组：`device-names` /
/// `device-bypassed` / `device-recognised` / `device-unrecognised`
/// （`ViewState::devices` 的 `TrackV3::devices` 投影）。实测（判据逐字打出整份清单）：
/// 这一格在 `#157` 之前 **34**，加上四个数组之后是 **38**。
///
/// ⚠ **代价边界（如实说清）**：
///
/// * 它们的消费者是 `console/device_rack.slint`，实例化在 `ConsoleTabs` 里
///   （`console_tabs.slint` 的 `visible: root.active-tab == 2`）⇒ 默认帧（Tab 0）的
///   运行时树**不含** `device-rack`（`test_port_adapter.rs` 的 `DEFAULT_VIEW_MUST_NOT_HAVE`
///   断言这一点）。⛔ 本机是 macOS、`golden/<platform>` 只有 `linux` ⇒ **未**做 Linux
///   基准的逐像素复测；这里只给"默认帧不实例化它"的结构性依据。
/// * 与 `#82` / `#133` 同一路线：`apply_view` 只在"换工程 / 撤销 / 打开"这类**完整重投影**
///   （以及本单新增的一次设备手势提交之后）上跑，不是 60Hz 心跳那一跳。
/// * ⛔ 墙钟成本本机没测（要 GPU 档，`AGENTS.md` §5 不允许）。
///
/// ## 38 → 42 的追加（任务单 `#159`，2026-10-18）
///
/// 那四格是发送条（仍在控制台 **Tab 3** 的 `DeviceRack` 面板里）的四个平行数组：
/// `ui.set_send_edge_ids` / `set_send_destination_names` / `set_send_gain_labels` /
/// `set_send_gain_dbs`（`ViewState::sends` 的投影）。四个数组同时把本常量从 **38**
/// 推到 **42**（判据逐字打印整份"换掉模型实例"清单，42 条）；它们的唯一事实源是
/// `RoutingGraph::edges` 的 `SendToAux` 边，⛔ 不是界面里的第二份发送列表。
///
/// ## 42 → 43 的追加（任务单 `#163`，2026-10-18）
///
/// 那一格是 `ui.set_device_ids`（`ViewState::devices` 逐条投影的**设备身份**数组，
/// 与 `device-names` / `device-bypassed` / `device-recognised` / `device-unrecognised`
/// 同索引集）。删除按钮按**身份**寻址（⛔ 行下标）⇒ 与发送条的 `send-edge-ids`
/// 同一条纪律，需要把这一个平行数组一起注入。
///
/// 读数（判据逐字打印整份清单）：**42 → 43**。其余四个 #163 字段都是**标量**
/// （`device-insert-available` / `device-insert-reason` / `clip-create-available` /
/// `clip-create-reason`）⇒ 只动 `APPLY_VIEW_WRITE_SITE_BUDGET`。
///
/// ⚠ 代价边界与 `#157` / `#159` 同款：消费者只在 Tab 3（默认帧 Tab 0 不实例化
/// `device-rack`），`apply_view` 只在完整重投影上跑；⛔ 墙钟成本本机未测。
///
/// ## 48 → 49 的追加（任务单 `#181`，2026-10-18）
///
/// 那一格是 `ui.set_master_device_bypassed`（`ViewState::master_devices` 逐条的旁通位，
/// 与 `master-device-names` 同索引集 —— 主总线每台一个旁通开关）。读数（判据逐字打印
/// 整份清单）：**48 → 49**。`#181` 的其余五格（EQ 的可用性 / 当前值 / 值域两端 /
/// 逐字原因）都是**标量** ⇒ 只动 `APPLY_VIEW_WRITE_SITE_BUDGET`（81 → 87）。
///
/// ## 49 → 52 的追加（宏面板，2026-10-30）
///
/// 那三格是 `ui.set_macro_names` / `ui.set_macro_values` / `ui.set_macro_value_labels`
/// （`ViewState::macros` 逐条投影：名字 / 位置 0.0..=1.0 / 位置文本）。宏面板的
/// 滑块 = 投影的 `TrackV3::macros` 下标 —— 与发送条同一条纪律（平行数组同索引集，
/// 三个数组一起注入）。读数（判据逐字打印整份清单）：**49 → 52**，同时
/// `APPLY_VIEW_WRITE_SITE_BUDGET` 93 → 96。消费者只在默认关着的右侧抽屉
/// （Tab 3 的按钮才打开它）⇒ 默认帧（Tab 0）一位不变。
///
/// ## 52 → 56 的追加（场景 / 段落色标，2026-10-30）
///
/// 那四格是两张色标切片的平行数组：`ui.set_scene_colors` / `ui.set_scene_color_labels`
/// （Session 场景卡）与 `ui.set_section_colors` / `ui.set_section_color_labels`
/// （编曲段落卡）。四个都经 `colors()` / `strings()` 注入 ⇒ 每次 `apply_view`
/// 都换一个 `VecModel` 实例。
///
/// 读数：CI run `40980667356` 先在 ③a 报 **96 → 100**（`apply_view` 多四行
/// `ui.set_*`），③b 因同一二进制 fail-fast 未跑到；按同一条构造（+4 个每次换实例的
/// 数组）本格是 **52 → 56**，由下一次 run 的 ③b 清单逐字复核。
///
/// ⚠ 代价边界与上面那些 Tab 3 数组**不同**：两个色标住在**默认帧**（编曲 / Session
/// 视图）⇒ 这四格确实进入默认帧的绘制路径（5 张 Linux 基准已重录，见 `a7854d27`）；
/// "颜色由工程驱动"的必付代价，不是被藏起来的抽屉控件。
/// ## 56 → 57 的追加（左侧资源栏条目接真实投影，任务单 `#133`，2026-10-30）
///
/// `ui.set_sidebar_items(strings(&content.items))` —— 一格**数组**，住在默认帧的
/// 左资源栏（240px 那一列）。它是"条目集合由投影决定"的必付代价：旧版在
/// `sidebar.slint` 里内联 8 个演示乐器名（含仓库里不存在的 "SFZ · 323 款"），
/// 零成本但⛔ 是编造。取数侧的每帧成本只有 `bridge::library_instruments` 的
/// **进程内缓存命中**（键 = 库根 ＋ 安装戳，一次 `stat`），⛔ 不读样本字节
/// （`#165` 的教训）。③a 不动：两个写入点收在 `host::inject_sidebar` 里，
/// `apply_view` 本体仍是 100 行 `ui.set_*`（本轮实测读数）。
/// ⭐ `D-UI-16`（2026-11-02）：`clip-mutes` —— `Cmd/Ctrl+M` 的**结果读数**（包头底色），
/// 住在 `apply_view` 的每帧路径上 ⇒ **57 → 58**（多这一个数组，判据清单逐字多一项）。
/// 它不改默认帧：演示 / 夹具工程里每条摆放默认 `muted = false` ⇒ 底色逐像素不变 ⇒
/// ⛔ 无需重录基准。
/// ## 58 → 63 的追加（音名八度统一线阶段 2，2026-11-17）
///
/// 那五格是采样器八度覆盖与装载显示读数的平行数组：`track-sampler-octave-auto` /
/// `track-sampler-octave-values`（步进当前档）＋ `track-sampler-key-ranges` /
/// `track-sampler-shift-kinds` / `track-sampler-shift-amounts`（装载后键位范围与
/// 移位徽标）。与 `track-sampler-paths` 同索引集、同一次注入 ⇒ **58 → 63**，
/// `APPLY_VIEW_WRITE_SITE_BUDGET` 同步 +5。消费者只在调音台通道条（Tab 1，
/// 默认帧 Tab 0 不实例化）⇒ 默认帧一位不变；取数侧走 `bridge::sampler_display`
/// 的进程缓存（键 = 路径＋覆盖），⛔ 不在每帧读盘。
/// ## 63 → 65 的追加（line/blank-drag-selection，2026-11-18）
///
/// 那两格是框选选区的**标志投影**：`clip-selected-flags`（与 `clip-ulids`
/// 同序同长）与 `timeline-block-selected`（与 `timeline-blocks` 同序同长），
/// 都按界面上的选区集 `selected-clip-ulids` 在 `apply_view` 每帧路径上重算
/// （选区是界面态、⛔ 不在 `ViewState` 里；只在手势时设 ⇒ 重注入后必然错位，
/// 与音符面 `note-selected` 同一条口径）。默认选区为空集 ⇒ 标志全 `false`
/// ⇒ 默认帧逐像素不变，⛔ 无需重录基准。
///
/// ## 65 → 66 的追加（审计 `T-FIX-88` 项 1：宏的级联去向，2026-11-22）
///
/// 一格 `macro-mapping-summaries`（与 `macro-names` 同索引集，每宏一行摘要）。
/// 消费者只在默认关着的右侧宏抽屉 ⇒ 默认帧（抽屉未打开）一位不变。
/// ## 66 → 67 的追加（C2，line/fix75-eq-comp-curves，T-FIX-112 重编号，2026-11-22）
///
/// `device-curve-svgs` 是设备链逐台真实参数曲线（EQ 频响 / 压缩传递曲线），与
/// `device-names` 同索引集、在 `apply_view` 每帧路径上重写。默认演示工程没有
/// EQ/压缩设备 ⇒ 数组每项为空串、`Path` 不画 ⇒ 默认帧逐像素不变，⛔ 无需重录基准。
/// ## 67 → 74 的订正（T-ISS-MWW4H9E9，2026-11-26）
///
/// 不是"成本涨了"，是"测量口径漏了"：`array_instance_addrs` 清单漏记了 13 个
/// `apply_view` 每帧无条件新建模型实例的数组（场景/段落色标 ×4、自动化泳道 ×5、
/// 卷帘音符模型、时间线块模型、轨音量/声相数值 ×2），③b 的实测读数因此一直少计
/// 13。补进清单后，③b 在基线逐字报 **74**（清单 75 项中 `track-meter-levels`
/// 不计：`apply_meters` 先比后写，相同注入之间地址不变；`tracks` 同理，本来就不
/// 在清单里）。`APPLY_VIEW_WRITE_SITE_BUDGET` 同期实测仍是 **125**，一字不动。
/// 三处文档读数（本文件、`bridge.rs:1007-1008`、`track_routing_crud.rs:14`）
/// 同步统一为 125 / 74。
const PER_FRAME_ARRAY_REWRITE_BUDGET: usize = 74;
/// `host.rs` 的 `apply_view` 函数体里的属性写入点个数上限（③a 实测 **125**）。
///
/// ## 39 → 40 的追加（任务单 `#82`，2026-10-16）
///
/// 那一格是 `ui.set_track_sampler_paths(...)`：SFZ 采样器绑定（`TrackV3::sampler`）的
/// **唯一**注入点。它必须住在 `apply_view` 里 —— `host::apply_view` 是"投影 ⇒ 界面"的
/// 唯一实现（D28），另一处写它就会多出第二个真相源。
///
/// 数量侧的重新测量（本机，Apple M2）：
///
/// * 写入点：**39 → 40**（本常量）；
/// * `array_instance_addrs` 的"每帧换掉模型实例"个数：见该函数（`#82` 把新数组
///   **计入清单**，因为它在 `apply_view` 的每帧路径上、不属于手势路径）；
/// * 默认帧（控制台 Tab 0，即 5 张 Linux 基准渲染的那一帧）：PNG sha256
///   `e93ec5bc26bddab8dda0f60425d56aa7d8774222bd7c3e48ecca4572141ba6ba` **逐位不变**
///   （改动前 / 后各测一次）⇒ ⛔ 不需要重录基准；
/// * 混音台那一帧（Tab 1，**不在**任何基准里）的 PNG sha256 从
///   `6f478d014908a27e6da00a535dfe90bc68125b5120ed570920f85e859a58b5d3` 变为
///   `3ae6fec9b8c3e68e48773c773df83b3721b5c509b6697b3199cb1574de7c4405`，
///   运行时节点数 102 → 111（每条轨 +3：路径输入 / SET / CLR）。
///
/// ## 40 → 43 的追加（任务单 `#133`，2026-10-17）
///
/// 那三格是 `ui.set_asset_pool_names(...)` / `ui.set_asset_pool_kinds(...)` /
/// `ui.set_asset_pool_details(...)`：素材池面板（`asset-pool-*` 三个平行数组）的
/// **唯一**注入点。它们必须住在 `apply_view` 里 —— 与 `#82` 同一条理由（D28：
/// "投影 ⇒ 界面"只有这一个实现，另一处写它就会多出第二个真相源）。
///
/// 数量侧的本机实测（Apple M2，判据逐字报 `从 40 变成了 43`）：
///
/// * 写入点：**40 → 43**（本常量）；
/// * 三个数组同时把 `PER_FRAME_ARRAY_REWRITE_BUDGET` 从 31 推到 34；
/// * 默认帧（Tab 0）的运行时树不含 `asset-pool-panel`（面板在 Tab 1）⇒ 结构上不参与
///   默认帧绘制；⛔ Linux 基准**未**逐像素复测（本机无法重录，见上）。
///
/// ⚠ **未测**（如实登记）：⛔ 绘制回调的**墙钟**成本没有在本机重量 —— 那要 `examples/fps_gpu.rs`
/// 的 GPU 档（本机纪律不允许，见 `AGENTS.md` §5）。上面给的是数量侧读数与逐像素读数。
///
/// ## 43 → 52 的追加（任务单 `#157`，2026-10-17）
///
/// 那九格是设备效果链（控制台 **Tab 3**，`ui/console/device_rack.slint`）的**唯一**注入点：
///
/// * 四个平行数组：`ui.set_device_names` / `set_device_bypassed` /
///   `set_device_recognised` / `set_device_unrecognised`（`ViewState::devices` 的投影）；
/// * 一个标量标题：`ui.set_device_track_label`（"轨 X · 设备链" —— 界面没有"选中轨"
///   状态，投影用**唯一**规则选轨并把它显示出来）；
/// * 四个标量旋钮读数：`ui.set_device_eq_available` / `set_device_eq_gain_db` /
///   `set_device_eq_min_db` / `set_device_eq_max_db`（**唯一**可编辑的设备参数
///   `eq_low_gain`；值域来自 `crate::effect_params`，⛔ 不是界面里的第二份表）。
///
/// 它们必须住在 `apply_view` 里 —— 与 `#82` / `#133` 同一条理由（D28："投影 ⇒ 界面"
/// 只有这一个实现，另一处写它就会多出第二个真相源）。
///
/// 数量侧的**本机实测**（Apple M2；判据逐字报 `从 43 变成了 52`）：
///
/// * 写入点：**43 → 52**（本常量，判据自报的真实新数）；
/// * 四个数组同时把 `PER_FRAME_ARRAY_REWRITE_BUDGET` 从 34 推到 **38**（判据逐字打印
///   整份"换掉模型实例"清单，38 条）；
/// * 默认帧（Tab 0）的运行时树不含 `device-rack`（面板在 Tab 3，`console_tabs.slint`
///   的 `visible: root.active-tab == 2`）⇒ 结构上不参与默认帧绘制；
///   `tests/test_port_adapter.rs` 的 `DEFAULT_VIEW_MUST_NOT_HAVE` 里 `device-rack` 仍在。
///   ⛔ Linux 基准**未**逐像素复测（本机 macOS 无法重录，见上）。
///
/// ⚠ **未测**（如实登记）：⛔ 与上面两档同款 —— 绘制回调的**墙钟**成本没有在本机重量。
///
/// ## 52 → 60 的追加（任务单 `#159`，2026-10-18）
///
/// 那八格是发送条（与设备链**同住** Tab 3 的 `DeviceRack` 面板）的**唯一**注入点：
///
/// * 四个平行数组：`ui.set_send_edge_ids` / `set_send_destination_names` /
///   `set_send_gain_labels` / `set_send_gain_dbs`（`ViewState::sends` 的投影）；
/// * 四个标量：`ui.set_send_gain_min_db` / `set_send_gain_max_db`（值域**来自模型**：
///   `AutomationTarget::SendGain` 的 `NOMINAL_GAIN_MIN_DB/MAX_DB`，⛔ 界面不造第二份表）、
///   `ui.set_send_create_available` / `ui.set_send_create_reason`（"新建发送"可用性与
///   不可用时的**逐字**原因 —— ⛔ 不静默禁用）。
///
/// 数量侧的**本机实测**（Apple M2；判据逐字报 `从 52 变成了 60`）：
///
/// * 写入点：**52 → 60**（本常量，判据自报的真实新数）；
/// * 四个数组同时把 `PER_FRAME_ARRAY_REWRITE_BUDGET` 从 38 推到 **42**（判据逐字打印
///   整份"换掉模型实例"清单，42 条）；
/// * 默认帧（Tab 0）的运行时树不含 `device-rack`（发送条在同一面板里）⇒ 结构上不参与
///   默认帧绘制；⛔ Linux 基准**未**逐像素复测（本机 macOS 无法重录，见上）。
///
/// ## 60 → 65 的追加（任务单 `#160`，2026-10-18）
///
/// 那五格是**轨 / 路由段**（与发送条**同住** Tab 3 的 `DeviceRack` 面板，上半段）的
/// **唯一**注入点，全部是**标量**（⛔ 没有新数组 ⇒ `PER_FRAME_ARRAY_REWRITE_BUDGET`
/// 仍是 **42**，一条都不动）：
///
/// * `ui.set_track_count_label` —— 工程轨数（含主总线）的显示文本；
/// * `ui.set_track_create_available` —— 建轨可用性（工程里得有主总线）；
/// * `ui.set_master_bus_label` —— 主总线说明；没有主总线时它**就是**建轨的逐字原因；
/// * `ui.set_track_delete_available` / `ui.set_track_delete_reason` —— "删除展示轨"的
///   可用性与不可用时的**逐字**原因（主总线不可删）。
///
/// 数量侧的**本机实测**（Apple M2；判据逐字报 `从 60 变成了 65`）：
///
/// * 写入点：**60 → 65**（本常量，判据自报的真实新数）；
/// * 每帧换模型实例的数组：**42 → 42**（没有新增数组，判据清单逐字不变）；
/// * 默认帧（Tab 0）的运行时树不含 `device-rack`（轨 / 路由段在同一面板里）⇒ 结构上
///   不参与默认帧绘制；⛔ Linux 基准**未**逐像素复测（本机 macOS 无法重录，见上）。
///
/// ## 65 → 70 的追加（任务单 `#163`，2026-10-18）
///
/// 那五格是**设备增删 ＋ 建空片段**（仍住 Tab 3 的 `DeviceRack` 面板）的**唯一**注入点：
///
/// * `ui.set_device_ids` —— 每个设备一格**身份**（删除按钮按身份寻址；同时把
///   `PER_FRAME_ARRAY_REWRITE_BUDGET` 从 42 推到 **43**）；
/// * `ui.set_device_insert_available` / `ui.set_device_insert_reason` —— "装 EQ / 装混响"
///   的可用性与不可用时的**逐字**原因（没有展示轨 ⇒ 禁用 ＋ 原因，⛔ 不静默）；
/// * `ui.set_clip_create_available` / `ui.set_clip_create_reason` —— "建空的 MIDI 片段"
///   的可用性与不可用时的**逐字**原因。
///
/// 数量侧的**本机实测**（判据逐字报 `从 65 变成了 70`）：
///
/// * 写入点：**65 → 70**（一个数组 ＋ 四个标量）；
/// * 每帧换模型实例的数组：**42 → 43**（只多 `device-ids`）；
/// * 默认帧（Tab 0）的运行时树不含 `device-rack` ⇒ 结构上不参与默认帧绘制；
///   ⛔ Linux 基准**未**逐像素复测（本机 macOS 无法重录，见上）。
///
/// ## 70 → 74 的追加（任务单 `#164`，2026-10-18）
///
/// 那四格是**一键装乐器**（仍住 Tab 3 的 `DeviceRack` 面板，紧挨 `#163` 的三个按钮）的
/// **唯一**注入点：
///
/// * `ui.set_instrument_names` —— 内置 SFZ 乐器的显示名数组（按钮行按它生成；
///   同时把 `PER_FRAME_ARRAY_REWRITE_BUDGET` 从 43 推到 **44**）；
/// * `ui.set_instrument_label` —— 当前乐器的逐字读数（`"乐器：<路径>"` / `"未装乐器"`）；
/// * `ui.set_instrument_install_available` / `ui.set_instrument_install_reason` ——
///   "装乐器"的可用性与不可用时的**逐字**原因（没有展示轨 ⇒ 禁用 ＋ 原因，⛔ 不静默）。
///
/// 数量侧的**本机实测**（判据逐字报 `从 70 变成了 74`）：
///
/// * 写入点：**70 → 74**（一个数组 ＋ 三个标量）；
/// * 每帧换模型实例的数组：**43 → 44**（只多 `instrument-names`）；
/// * 默认帧（Tab 0）的运行时树不含 `device-rack` ⇒ 结构上不参与默认帧绘制；
///   ⛔ Linux 基准**未**逐像素复测（本机 macOS 无法重录，见上）。
///
/// ## `#165` 复核（2026-10-18）：**74 / 44 一条都不动**
///
/// `#165` 把内置乐器清单按"样本字节是否存在"过滤（今天为空 ⇒ 按钮数 **0**）。但本常量数的
/// 是 `apply_view` 函数体里的**源码写入点**，不是按钮实例数：`ui.set_instrument_names`
/// 仍然必须住在 `apply_view`（清单是**数据驱动**的，样本入库后 0 个按钮会自己变成 N 个）
/// ⇒ 写入点**74**、每帧换模型实例的数组**44**（判据逐字报 `= 74` / `= 44`，清单里
/// `instrument-names` 仍在）。⛔ 因此本单**不**改小这两个常量：改小会删掉一个仍然必需的
/// 注入点。⚠ 若将来整块"装乐器"被移除，请按三件套重新测量并**改小**。
/// ## `#180` 追加（2026-10-18）：**74 → 81** ／ **44 → 48**
///
/// 那七格是**主总线设备链**（仍住 Tab 3 的 `DeviceRack` 面板，面板右侧新增第二段）的
/// **唯一**注入点：
///
/// * 四个平行数组：`ui.set_master_device_names` / `set_master_device_ids` /
///   `set_master_device_recognised` / `set_master_device_unrecognised`
///   （`ViewState::master_devices` 的投影，与展示轨那四个数组同一份
///   `device_rack_projection`）；
/// * 三个标量：`ui.set_master_device_label`（`"主总线 Master · 设备链"`；没有主总线时
///   **就是**装设备的逐字原因）、`ui.set_master_device_insert_available` /
///   `ui.set_master_device_insert_reason`（可用性与不可用时的**逐字**原因）。
///
/// 数量侧的**本机实测**（判据逐字报 `从 74 变成了 81` / `每帧换掉模型实例的数组 = 48`）：
///
/// * 写入点：**74 → 81**（四个数组 ＋ 三个标量）；
/// * 每帧换模型实例的数组：**44 → 48**（只多 `master-device-*` 四个）；
/// * 默认帧（Tab 0）的运行时树不含 `device-rack` ⇒ 结构上不参与默认帧绘制；
///   ⛔ Linux 基准**未**逐像素复测（本机 macOS 无法重录，见上）。
/// ## `#181` 追加（2026-10-18）：**81 → 87** ／ **48 → 49**
///
/// 那六格是**主总线设备的旁通 ＋ EQ 滑杆**（仍住 Tab 3 的 `DeviceRack` 面板第二段）的
/// **唯一**注入点：
///
/// * 一个平行数组：`ui.set_master_device_bypassed`
///   （`ViewState::master_devices` 逐条的 `bypassed`，与 `master-device-names` 同索引集）；
/// * 五个标量：`ui.set_master_device_eq_available` / `set_master_device_eq_gain_db` /
///   `set_master_device_eq_min_db` / `set_master_device_eq_max_db` /
///   `set_master_device_eq_reason`（那一格 `eq_low_gain` 的可用性 / 当前值 / 值域两端 /
///   不可用时的逐字原因；值域**只**来自 `crate::effect_params` 的契约）。
///
/// 数量侧的**本机实测**（判据逐字报 `从 81 变成了 87` / `每帧换掉模型实例的数组 = 49`）：
///
/// * 写入点：**81 → 87**（一个数组 ＋ 五个标量）；
/// * 每帧换模型实例的数组：**48 → 49**（只多 `master-device-bypassed`）；
/// * 默认帧（Tab 0）的运行时树不含 `device-rack` ⇒ 结构上不参与默认帧绘制；
///   `src/test_port_adapter.rs` 的 `DEFAULT_VIEW_MUST_NOT_HAVE` 仍钉住这一点；
///   ⛔ Linux 基准**未**逐像素复测（本机 macOS 无法重录，见上）。
/// ## `#182` 追加（2026-10-18）：**87 → 89** ／ **49 → 49**
///
/// 那两格是 **EQ 手势的设备身份**（仍住 Tab 3 的 `DeviceRack` 面板）的**唯一**注入点：
///
/// * `ui.set_device_eq_device_id` —— 展示轨那一格 `eq_low_gain` 的
///   `DeviceDefinition::id`（`.slint` 的 `device-eq-device-id`）；
/// * `ui.set_master_device_eq_device_id` —— 主总线那一格的身份
///   （`master-device-eq-device-id`）。
///
/// ⭐ 为什么**必须**加这两个属性：`#157`／`#181` 的 EQ 手势只报归一化位置，宿主按"第一台
/// 带 `eq_low_gain` 的设备"现求目标 ⇒ "投影之后、按下之前"链被增删时，按下会落到**另一台**。
/// `#182` 把投影里的身份带回来、在按下 / 松手两侧按身份重解析槽位（模型层的
/// `AutomationTarget::DeviceParam` 只吃槽位）⇒ 窗口关闭。两个都是**标量**，⛔ 没有新数组。
///
/// 数量侧的**本机实测**（本机 macOS，判据逐字报）：
///
/// * 写入点：**87 → 89**（两个标量；`apply_view` 里只多了两行 `ui.set_*`）；
/// * 每帧换模型实例的数组：**49 → 49**（一条都不动）；
/// * 默认帧（Tab 0）的运行时树不含 `device-rack` ⇒ 结构上不参与默认帧绘制；
///   ⛔ 墙钟成本本机未测（要 GPU 档，`AGENTS.md` §5 不允许）。
///
/// ## `#196` 追加（2026-10-18）：**89 → 93** ／ **49 → 49**
///
/// 那四格是**采样库**（仍住 Tab 3 的 `DeviceRack` 面板"装乐器"区）的投影读数：
///
/// * `ui.set_sample_library_installed` —— 是否已按清单校验安装（Bool）；
/// * `ui.set_sample_library_notice` —— 未安装时逐字 `"采样库未安装，点此下载（9.4 GiB）"`；
/// * `ui.set_sample_library_download_visible` —— 下载按钮是否在场（Bool）；
/// * `ui.set_sample_library_root` —— 程序预设的库根（字符串，判据据此证明落点）。
///
/// ⭐ 为什么**必须**加这四格：下载按钮的可用性 / 逐字提示 / 落点全部由 Rust 投影决定
/// （⛔ 界面不拼平台路径、⛔ 不猜"装没装"）。四个都是**标量**，⛔ 没有新数组。
///
/// 数量侧的**本机实测**（本机 macOS，判据逐字报）：
///
/// * 写入点：**89 → 93**（四个标量；`apply_view` 里只多了四行 `ui.set_*`）；
/// * 每帧换模型实例的数组：**49 → 49**（一条都不动）。
/// * 默认帧（Tab 0）的运行时树不含 `device-rack` ⇒ 结构上不参与默认帧绘制。
///
/// ⭐ 宏面板（2026-10-30）：+3 个数组写入点（`macro-names` / `macro-values` /
/// `macro-value-labels`）⇒ **90 → 93 → 96**；宏面板没有新的标量投影字段。
///
/// ⭐ 场景 / 段落色标（2026-10-30）：`apply_view` 多四行
/// `ui.set_scene_colors` / `set_scene_color_labels` / `set_section_colors` /
/// `set_section_color_labels` ⇒ **96 → 100**。读数来自 CI run `40980667356`
/// （③a 逐字报"`apply_view` 的属性写入点从 96 变成了 100"）；它们的消费者住在
/// **默认帧**（编曲 / Session 视图）⇒ 5 张 Linux 基准已随 `a7854d27` 重录。
/// ⭐ `D-UI-16`（2026-11-02）：多**一行** `ui.set_clip_mutes`（`Cmd/Ctrl+M` 的结果读数，
/// 与上面 `track-mutes` 同族）⇒ **100 → 101**。它的消费者住在**默认帧**（编曲视图的
/// `clip-{ulid}-header` 底色），但默认全 `muted = false` ⇒ 底色不变 ⇒ ⛔ 基准无需重录。
/// ⭐ 2026-11-07（夜半 UI 轨道交互修复线）：多**一行** `ui.set_arrangement_content_px`
/// （横向滚动条的滑块大小与位置 ⇒ 内容总长，标量）⇒ **101 → 102**。消费者是编曲视图
/// 底部的滚动条（默认帧新增控件 ⇒ 基准须重录，见完结回执）。
/// ⭐ 2026-11-08（`[PATCH UI-TRACK-KINDS-01]`）：多**一行** `ui.set_track_kinds`
/// （轨道类型数组，与 `track-names` 同索引集）⇒ **102 → 103**。消费者是编曲包头左沿的
/// 类型色带与控制区的类型徽标（默认帧新增**纯视觉**节点 ⇒ 基准须重录，见
/// `tests/golden/linux/MANIFEST.txt` 末尾的待重录声明）；该数组同时计入 ③b 的
/// `array_instance_addrs` 清单。
/// ⭐ 2026-11-17（`line/arrangement-grid`）：多**一行** `ui.set_beats_per_bar`
/// （拍号分子**标量**，投影 `time_signature_numerator` 的读数）⇒ **103 → 104**。
/// 消费者是编曲标尺的拍刻度与车道／空置行的拍线（Wang 2026-11-17：网格以拍为
/// 常显基本单位，强度次于小节线）；标量不换模型实例 ⇒ ③b 的数组清单不动。
/// 默认帧新增纯视觉节点（拍刻度 / 拍线）⇒ Linux 基准帧像素随之变化，UI golden
/// 比对现为禁用态（Wang 2026-11-12 令），基准重录随 UI 重构收尾统一做。
/// ⭐ 2026-11-17（`line/arrangement-grid` 五版）：多**五行**网格密度标量
/// `ui.set_grid_label_stride` / `set_grid_line_unit` / `set_grid_show_beat_ticks` /
/// `set_grid_show_lane_beats` / `set_grid_subdiv_per_beat`（宿主 `grid_density`
/// 按小节宽与拍号分子算出的读数）⇒ **104 → 109**。消费者是编曲标尺与车道／
/// 空置行的缩放自适应（编号步长、线条单位、刻度可见性与细分份数；Wang
/// 2026-11-17 14:37 按 Logic Pro 实测规格定）；五个都是**标量** ⇒ ③b 的数组
/// 清单不动。绘制量侧：单位 >= 2 时小节线／刻度按单位跳画、拍线与细分只在
/// 宽度达标时出现 ⇒ 自适应只减不增每帧绘制量。默认帧（bar 128px）新增细分
/// 刻度节点 ⇒ 像素随之变化，UI golden 比对现为禁用态（同上）。
/// ⭐ 2026-11-17（音名八度统一线阶段 2）：多**五行** `ui.set_track_sampler_octave_auto`
/// / `set_track_sampler_octave_values` / `set_track_sampler_key_ranges` /
/// `set_track_sampler_shift_kinds` / `set_track_sampler_shift_amounts`
/// （与 `track-sampler-paths` 同索引集）⇒ **103 → 108**。消费者只在调音台
/// 通道条（Tab 1）⇒ 默认帧一位不变、⛔ 基准无需重录；五格同时计入 ③b 清单。
/// ⭐ 2026-11-17（并集：`line/arrangement-grid` ＋ 音名八度统一线阶段 2）：
/// 两路同基 103、写点互不重叠 ⇒ **109 + 5 = 114**（网格的 1 + 5 与八度的 5
/// 全是新增写点；③b 清单只含八度的五格，网格六点皆为标量）。
/// ⭐ 2026-11-18（台账订正：`line/sfz-note-octave-unify` 合入 main 91ce3c7a5 后）：
/// 114 失真——并集算法取了旧基数 103（103＋网格 6＋八度 5），未计主干在鼓机
/// 合入后已带进的增量；按本文件 `write_sites` 口径实测合入树 `apply_view`
/// 的 `ui.set_` 写点为 **122** ⇒ **114 → 122**。该常量无活测试断言，纯台账。
/// ⭐ 2026-11-18（line/blank-drag-selection）：多**一行** `ui.set_clip_selected_flags`
/// （框选标志的工作区面投影，按选区集在 `apply_view` 里现算）⇒ **122 → 123**。
/// 结构面的同序标志 `timeline-block-selected` 的写入点收在
/// `host::refresh_timeline_block_selected` 里（与 `apply_current_scene` /
/// `inject_sidebar` 同款收拢法），⛔ 不计本常量的函数体口径；其数组本身计入
/// `PER_FRAME_ARRAY_REWRITE_BUDGET`（63 → 65，见彼处条目）。
///
/// ## 123 → 124 的追加（审计 `T-FIX-88` 项 1：宏的级联去向，2026-11-22）
///
/// 一格 `ui.set_macro_mapping_summaries`（`ViewState::macro_mapping_summaries` 的
/// 同索引集投影：每宏一行 `"音量 +6 · 声相 -0.5"`，空映射 ⇒ 空串）。消费者只在
/// 默认关着的右侧宏抽屉（`macro-panel-open`，由下舱「宏 (N)」按钮打开）⇒
/// 默认帧（未打开）一位不变。
/// ## 124 → 125 的追加（C2，line/fix75-eq-comp-curves，T-FIX-112 重编号，2026-11-22）
///
/// `ui.set_device_curve_svgs(...)` 是设备链逐台真实曲线的唯一注入点（与
/// `device-names` 同住 `apply_view`）。默认工程没有 EQ/压缩设备 ⇒ 每项空串，
/// 卡片 `Path` 隐藏 ⇒ 默认帧不变。
// ⭐ T-FIX-494：125 → 108。
//
// 依赖横向偏移的那 17 个写点（标尺三连 + 网格五连 + 音符五连 + note-selected +
// roll-scroll-x + tick 上下界）从 `apply_view` 移进了 `apply_scroll_view`，
// 由整份重投影与滚动快路径**共用**。`apply_view` 自己的写入点因此从 125 降到 108；
// 总写入量没有增加（移动，不是新增），滚动事件那一路反而从"每次 100+ 写"降到 17。
// ⭐ T-FIX-754：108 → 109。`ui.set_count_in_bars` 是预备拍菜单档位的唯一注入点
// （`ViewState::count_in_bars`，随重投影回填）；⛔ 不像 60Hz 心跳那样每帧跑。
const APPLY_VIEW_WRITE_SITE_BUDGET: usize = 109;

/// 取 `[BASELINE-003]` 第 `frame` 帧的滚动偏移（与 `examples/fps_gpu.rs` 同款换算）。
fn scroll_at(frame: usize) -> f32 {
    frame as f32 * (VIEWPORT_WIDTH / 120.0)
}

/// 走一遍 `apply_view` 每帧会碰的数组属性，返回 `(属性名, 模型实例地址)`。
///
/// `ModelRc` 的 `PartialEq` 比较内层模型的指针，所以地址变了 = 换了模型实例。
/// 这里只收 `apply_view` 每帧路径上的数组；`track-height-override-*` 归宿主的手势路径。
///
/// ⚠ 清单补账（T-ISS-MWW4H9E9）：本清单在 Aether 迁移后漏记了 13 个 `apply_view`
/// 每帧无条件新建模型实例的数组（场景/段落色标 ×4、自动化泳道 ×5、卷帘音符模型、
/// 时间线块模型、轨音量/声相数值 ×2），预算 67 因此比实测少计 13 —— 预算不是
/// "过期放宽"，是"测量口径漏了"。本次把 13 项补进清单（各带注释），预算随实测重钉。
///
/// ⛔ `tracks` 有意不在清单里：`apply_view` 对它先比后写（`tracks_model_equal`
/// 才 `set_tracks`），两次相同注入之间地址不变 ⇒ 它不是"每帧重建"，不计入预算。
/// 若将来它改回无条件注入，请把它加进清单并同步重钉预算。
fn array_instance_addrs(ui: &yeban_app::ui::MainWindow) -> Vec<(&'static str, usize)> {
    let mut out: Vec<(&'static str, usize)> = Vec::new();
    macro_rules! push {
        ($name:literal, $getter:ident) => {{
            let model = ui.$getter();
            let thin: *const () = std::ptr::from_ref(model.as_any()) as *const ();
            out.push(($name, thin as usize));
        }};
    }
    push!("track-names", get_track_names);
    // ⭐ [PATCH UI-TRACK-KINDS-01]：轨道类型数组也在 `apply_view` 的每帧路径上
    // （⛔ 不是手势路径）⇒ 必须计入这份清单，否则"又加了一个每帧换模型实例的数组"会漏数。
    push!("track-kinds", get_track_kinds);
    push!("track-ids", get_track_ids);
    push!("track-ys", get_track_ys);
    push!("track-heights", get_track_heights);
    push!("track-volumes", get_track_volumes);
    push!("track-pans", get_track_pans);
    // ⭐ T-ISS-MWW4H9E9 补账：轨音量/声相的数值数组也在 `apply_view` 每帧路径上
    // （`lengths(&view.track_volume_fractions())` / `lengths(&view.track_pan_values())`，
    // 每次新建模型实例）⇒ 必须计入，否则预算漏数。
    push!("track-volume-fractions", get_track_volume_fractions);
    push!("track-pan-values", get_track_pan_values);
    push!("track-mutes", get_track_mutes);
    push!("track-solos", get_track_solos);
    // ⭐ 任务单 `#82`：SFZ 采样器路径也在 `apply_view` 的每帧路径上（⛔ 不是手势路径）
    // ⇒ 必须计入这份清单，否则"又加了一个每帧换模型实例的数组"这类回归会漏数。
    push!("track-sampler-paths", get_track_sampler_paths);
    // ⭐ 阶段 2：八度覆盖与装载显示读数的五个平行数组也在 `apply_view` 的每帧
    // 路径上 ⇒ 必须计入这份清单（同 `track-sampler-paths` 的纪律）。
    push!("track-sampler-octave-auto", get_track_sampler_octave_auto);
    push!(
        "track-sampler-octave-values",
        get_track_sampler_octave_values
    );
    push!("track-sampler-key-ranges", get_track_sampler_key_ranges);
    push!("track-sampler-shift-kinds", get_track_sampler_shift_kinds);
    push!(
        "track-sampler-shift-amounts",
        get_track_sampler_shift_amounts
    );
    push!("scene-names", get_scene_names);
    push!("section-names", get_section_names);
    push!("section-positions", get_section_positions);
    push!("section-widths", get_section_widths);
    // ⭐ T-ISS-MWW4H9E9 补账：场景/段落色标的四个平行数组也在 `apply_view` 每帧路径上
    // （`colors(&view.scene_colors())` 等，每次新建模型实例；预算 52→56 那次只改了
    // 常量、漏了清单）⇒ 必须计入，否则预算漏数。
    push!("scene-colors", get_scene_colors);
    push!("scene-color-labels", get_scene_color_labels);
    push!("section-colors", get_section_colors);
    push!("section-color-labels", get_section_color_labels);
    push!("clip-ulids", get_clip_ulids);
    // ⭐ line/blank-drag-selection（2026-11-18）：框选标志的两个投影都在
    // `apply_view` 的每帧路径上（按界面选区集重算）⇒ 必须计入这份清单。
    push!("clip-selected-flags", get_clip_selected_flags);
    push!("timeline-block-selected", get_timeline_block_selected);
    // ⭐ T-ISS-MWW4H9E9 补账：时间线块模型也在 `apply_view` 每帧路径上
    // （`ModelRc::new(VecModel::from(view_to_timeline_blocks(view)))`，每次新建
    // 模型实例）⇒ 必须计入，否则预算漏数。
    push!("timeline-blocks", get_timeline_blocks);
    push!("clip-labels", get_clip_labels);
    // ⭐ `D-UI-16`（2026-11-02）：`clip-mutes` 也在 `apply_view` 的每帧路径上（⛔ 不是手势路径，
    // 它是 `Cmd/Ctrl+M` 的结果读数）⇒ 必须计入这份清单，否则"又加了一个每帧换模型实例的数组"会漏数。
    push!("clip-mutes", get_clip_mutes);
    push!("clip-positions", get_clip_positions);
    push!("clip-widths", get_clip_widths);
    // T-FIX-421：`clip-ys` / `clip-heights` 已随死注入删除（剪辑几何改走行内相对布局），
    // 不再在 `apply_view` 的每帧路径上 ⇒ 从本清单移除。
    // ⭐ 任务单 `#133`：素材池面板的三个平行数组也在 `apply_view` 的每帧路径上
    // （⛔ 不是手势路径）⇒ 必须计入这份清单，否则"又加了一个每帧换模型实例的数组"会漏数。
    push!("asset-pool-names", get_asset_pool_names);
    push!("asset-pool-kinds", get_asset_pool_kinds);
    push!("asset-pool-details", get_asset_pool_details);
    // ⭐ 任务单 `#157`：设备效果链（Tab 3）的四个平行数组同样在 `apply_view` 的每帧路径上
    // （⛔ 不是手势路径）⇒ 必须计入这份清单，否则"又加了一个每帧换模型实例的数组"会漏数。
    push!("device-names", get_device_names);
    // ⭐ C2：逐台真实参数曲线（与 `device-names` 同索引集，每帧重写）。
    push!("device-curve-svgs", get_device_curve_svgs);
    // ⭐ 任务单 `#163`：每台的**设备身份**（删除按钮按身份寻址）—— 与上面同索引集，
    // 也在 `apply_view` 的每帧路径上 ⇒ 必须计入这份清单。
    push!("device-ids", get_device_ids);
    push!("device-bypassed", get_device_bypassed);
    push!("device-recognised", get_device_recognised);
    push!("device-unrecognised", get_device_unrecognised);
    // ⭐ 任务单 `#180`：主总线设备链（同一面板第二段）的四个平行数组同样在
    // `apply_view` 的每帧路径上（⛔ 不是手势路径）⇒ 必须计入这份清单，否则会漏数。
    push!("master-device-names", get_master_device_names);
    push!("master-device-ids", get_master_device_ids);
    push!("master-device-recognised", get_master_device_recognised);
    push!("master-device-unrecognised", get_master_device_unrecognised);
    // ⭐ 任务单 `#181`：主总线每台的**旁通位**（第二个可编辑面的读数）也在 `apply_view`
    // 的每帧路径上（⛔ 不是手势路径）⇒ 必须计入这份清单，否则会漏数。
    push!("master-device-bypassed", get_master_device_bypassed);
    // ⭐ 任务单 `#164`：内置乐器的按钮名数组也在 `apply_view` 的每帧路径上
    // （⛔ 不是手势路径）⇒ 必须计入这份清单，否则"又加了一个每帧换模型实例的数组"会漏数。
    push!("instrument-names", get_instrument_names);
    // ⭐ 任务单 `#159`：发送条（与设备链同住 Tab 3）的四个平行数组同样在 `apply_view`
    // 的每帧路径上（⛔ 不是手势路径）⇒ 必须计入这份清单，否则会漏数。
    push!("send-edge-ids", get_send_edge_ids);
    push!("send-destination-names", get_send_destination_names);
    push!("send-gain-labels", get_send_gain_labels);
    push!("send-gain-dbs", get_send_gain_dbs);
    // ⭐ 宏面板（Tab 3 头部「宏 (N)」→ 右侧抽屉）的三个平行数组同样在 `apply_view`
    // 的每帧路径上（⛔ 不是手势路径）⇒ 必须计入这份清单，否则会漏数。
    push!("macro-names", get_macro_names);
    push!("macro-values", get_macro_values);
    push!("macro-value-labels", get_macro_value_labels);
    // ⭐ 审计 `T-FIX-88` 项 1：级联去向的摘要数组（`"音量 +6 · 声相 -0.5"`，空映射 ⇒ 空串）
    // 也在 `apply_view` 的每帧路径上（⛔ 不是手势路径）⇒ 必须计入这份清单。
    push!("macro-mapping-summaries", get_macro_mapping_summaries);
    push!("bar-positions", get_bar_positions);
    push!("note-ulids", get_note_ulids);
    push!("note-velocities", get_note_velocities);
    push!("note-positions", get_note_positions);
    push!("note-widths", get_note_widths);
    push!("note-ys", get_note_ys);
    push!("note-selected", get_note_selected);
    // ⭐ T-ISS-MWW4H9E9 补账：卷帘音符模型也在 `apply_view` 每帧路径上
    // （`ModelRc::new(VecModel::from(view_to_notes(view)))`，每次新建模型实例）
    // ⇒ 必须计入，否则预算漏数。
    push!("notes", get_notes);
    push!("track-colors", get_track_colors);
    push!("track-color-labels", get_track_color_labels);
    push!(
        "automation-lane-target-keys",
        get_automation_lane_target_keys
    );
    // ⭐ T-ISS-MWW4H9E9 补账：自动化泳道的五个平行数组也在 `apply_view` 每帧路径上
    // （`strings` / `booleans` / `integers`，每次新建模型实例）⇒ 必须计入，
    // 否则预算漏数。
    push!(
        "automation-lane-track-indexes",
        get_automation_lane_track_indexes
    );
    push!("automation-lane-labels", get_automation_lane_labels);
    push!(
        "automation-lane-axis-labels",
        get_automation_lane_axis_labels
    );
    push!(
        "automation-lane-read-enabled",
        get_automation_lane_read_enabled
    );
    push!("automation-lane-badges", get_automation_lane_badges);
    push!("automation-path-commands", get_automation_path_commands);
    push!("track-meter-levels", get_track_meter_levels);
    // ⭐ 任务单 `#133` 的第二段：左侧资源栏的**条目数组**也在 `apply_view` 的每帧路径上
    // （换分类时由宿主重注入，⛔ 不是手势路径）⇒ 必须计入这份清单，否则会漏数。
    push!("sidebar-items", get_sidebar_items);
    out
}

/// 从 Rust 源码里切出一个顶层函数的函数体（rustfmt 下函数体以**行首 `}`** 结束）。
fn function_body<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("host.rs 里找不到 {signature}"));
    let rest = &source[start..];
    let open = rest.find('{').expect("函数必须有函数体") + 1;
    let body = &rest[open..];
    let end = body.find("\n}\n").expect("函数体必须以行首 } 结束");
    &body[..end]
}

/// 数一个函数体里的**属性写入点**（`ui.set_*(...)`）。注释行不计。
fn write_sites(body: &str) -> usize {
    body.lines()
        .filter(|line| line.trim_start().starts_with("ui.set_"))
        .count()
}

/// **断言 ①**：视口裁剪把可见音符数钉在 240 上下。
#[test]
fn viewport_clipping_pins_the_number_of_visible_notes() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> arrangement-content-px:"),
        "新 UI 契约面必须声明 > arrangement-content-px:"
    );
    assert!(
        aether_host.contains("set_arrangement_content_px"),
        "宿主接线层必须注册 set_arrangement_content_px"
    );
    assert!(
        aether_app.contains("> asset-pool-details:"),
        "新 UI 契约面必须声明 > asset-pool-details:"
    );
    assert!(
        aether_host.contains("set_asset_pool_details"),
        "宿主接线层必须注册 set_asset_pool_details"
    );
    assert!(
        aether_app.contains("> asset-pool-kinds:"),
        "新 UI 契约面必须声明 > asset-pool-kinds:"
    );
    assert!(
        aether_host.contains("set_asset_pool_kinds"),
        "宿主接线层必须注册 set_asset_pool_kinds"
    );
}

/// 卷帘 `.slint` 里 per-note 循环的条数与固定循环常量 —— 图元公式 `2N + 64` 的两个来源。
///
/// 数 per-note 循环：谁给每个音符再加一个图元（第三条 `for … in root.note-ulids`），这条红。
/// 数固定循环：`5` / `16` / `16` / `16` 一变，`FIXED_ROLL_PRIMITIVES = 64` 的依据就变了。
#[test]
fn roll_primitive_formula_has_exactly_one_loop_per_note_family() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> arrangement-content-px:"),
        "新 UI 契约面必须声明 > arrangement-content-px:"
    );
    assert!(
        aether_host.contains("set_arrangement_content_px"),
        "宿主接线层必须注册 set_arrangement_content_px"
    );
    assert!(
        aether_app.contains("> asset-pool-details:"),
        "新 UI 契约面必须声明 > asset-pool-details:"
    );
    assert!(
        aether_host.contains("set_asset_pool_details"),
        "宿主接线层必须注册 set_asset_pool_details"
    );
    assert!(
        aether_app.contains("> asset-pool-kinds:"),
        "新 UI 契约面必须声明 > asset-pool-kinds:"
    );
    assert!(
        aether_host.contains("set_asset_pool_kinds"),
        "宿主接线层必须注册 set_asset_pool_kinds"
    );
}

/// **断言 ②**：运行时控件树里真的有 2N 个卷帘图元（不是"源码里写着 2N"）。
#[test]
fn roll_primitives_at_runtime_are_pinned_by_the_viewport() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> arrangement-content-px:"),
        "新 UI 契约面必须声明 > arrangement-content-px:"
    );
    assert!(
        aether_host.contains("set_arrangement_content_px"),
        "宿主接线层必须注册 set_arrangement_content_px"
    );
    assert!(
        aether_app.contains("> asset-pool-details:"),
        "新 UI 契约面必须声明 > asset-pool-details:"
    );
    assert!(
        aether_host.contains("set_asset_pool_details"),
        "宿主接线层必须注册 set_asset_pool_details"
    );
    assert!(
        aether_app.contains("> asset-pool-kinds:"),
        "新 UI 契约面必须声明 > asset-pool-kinds:"
    );
    assert!(
        aether_host.contains("set_asset_pool_kinds"),
        "宿主接线层必须注册 set_asset_pool_kinds"
    );
}

/// **断言 ③**：每帧注入的"重建"量不得增长（源码写入点 + 运行期模型实例）。
///
/// ⚠ 本判据曾在 Aether UI 迁移（`a8533bd0` / `1bd20285`）中被掏成只断言字符串包含
/// 的空壳（`PER_FRAME_ARRAY_REWRITE_BUDGET` / `APPLY_VIEW_WRITE_SITE_BUDGET`
/// 自此零活断言）；T-ISS-MWW4H9E9 把 2fa6730b 的活体恢复回来，并把清单补全到
/// `apply_view` 每帧路径的实测现状（见 `array_instance_addrs` 的补账注释）。
#[test]
fn per_frame_injection_does_not_grow() {
    use yeban_ui_test_port::image::Size;
    use yeban_ui_test_port::port::Permission;
    use yeban_ui_test_port::render::LivePort;

    // ---- ③a：源码层的写入点预算 ----
    let source = include_str!("../src/host.rs");
    let apply_view = function_body(source, "pub fn apply_view(");
    let sites = write_sites(apply_view);
    assert_eq!(
        sites, APPLY_VIEW_WRITE_SITE_BUDGET,
        "`apply_view` 的属性写入点从 {APPLY_VIEW_WRITE_SITE_BUDGET} 变成了 {sites}。\
         每帧多写一个属性 = 每帧多重建一批图元。若这一处确实必需：先重新测量绘制回调，\
         再把常量改到新值，并在提交信息里写出新读数。"
    );

    // ---- ③b：运行期的"换掉模型实例"个数 ----
    let project = yeban_model::samples::project_with_notes(NOTE_COUNT);
    let view = ViewState::from_project(&project).expect("10 万音符夹具必须能投影");
    let scene = DemoScene::from_view(&view);
    let size = Size::new(scene.viewport_width, scene.viewport_height);
    let port = LivePort::new(size, Permission::ReadOnly, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + MainWindow");
    let ui = port.ui();

    // 同一个 view、同一个偏移，注入两次。第二次若换了模型实例，就是"每帧重建"。
    let scroll = scroll_at(PROBED_FRAMES[1]);
    host::apply_view(ui, &view, VIEWPORT_WIDTH, scroll);
    let before = array_instance_addrs(ui);
    host::apply_view(ui, &view, VIEWPORT_WIDTH, scroll);
    let after = array_instance_addrs(ui);
    let rewrites: Vec<&str> = before
        .iter()
        .zip(after.iter())
        .filter(|(lhs, rhs)| lhs.1 != rhs.1)
        .map(|(lhs, _)| lhs.0)
        .collect();
    assert!(
        rewrites.len() <= PER_FRAME_ARRAY_REWRITE_BUDGET,
        "每帧换掉模型实例的数组从预算 {} 涨到了 {}：{rewrites:?}。\
         每个被换掉的模型都会让它的 repeater 重建子图元，而重建落在绘制回调里。",
        PER_FRAME_ARRAY_REWRITE_BUDGET,
        rewrites.len()
    );
    assert!(
        rewrites.len() >= 6,
        "随滚动变化的六个音符数组必须每帧都注入；现在只有 {} 个数组换了实例 —— \
         少了就说明卷帘不会跟着滚动重画",
        rewrites.len()
    );
}
