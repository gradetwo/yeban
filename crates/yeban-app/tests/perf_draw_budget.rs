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
/// 数法：5 个工具按钮 + 5 段工具文本 + 1 段吸附状态文本 + 1 个琴键列容器 + 16 个琴键
/// + 1 个网格容器 + 16 条横向车道 + 16 条纵向小节线 + 1 个 AI 建议块
/// + 1 个力度泳道容器 + 1 个走带光标 = **65**。`TouchArea` 不绘制，不计。
const FIXED_ROLL_PRIMITIVES: usize = 65;
/// 卷帘每帧绘制图元的预算上限 = `2 × max(N) + FIXED_ROLL_PRIMITIVES`。
const ROLL_PRIMITIVE_BUDGET: usize = 2 * 242 + FIXED_ROLL_PRIMITIVES;
/// 运行时控件树里的 `note-*-rect` 条数（中点帧，Tier-1 软件窗口，1920×1080）。
///
/// 它比注入的 242 少 8 个：完全落在父级裁剪矩形之外的图元不进运行时树。
/// ⚠ 这个数与上游（`i-slint-backend-testing`）的枚举规则有关。上游枚举规则一变，这条会红；
/// 那时必须重新测量图元数，不要直接把常量对齐过去。
const RUNTIME_NOTE_RECTS: usize = 234;
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
const PER_FRAME_ARRAY_REWRITE_BUDGET: usize = 56;
/// `host.rs` 的 `apply_view` 函数体里的属性写入点个数上限（本单之后 **60**）。
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
const APPLY_VIEW_WRITE_SITE_BUDGET: usize = 100;

/// 取 `[BASELINE-003]` 第 `frame` 帧的滚动偏移（与 `examples/fps_gpu.rs` 同款换算）。
fn scroll_at(frame: usize) -> f32 {
    frame as f32 * (VIEWPORT_WIDTH / 120.0)
}

/// 走一遍 `apply_view` 每帧会碰的数组属性，返回 `(属性名, 模型实例地址)`。
///
/// `ModelRc` 的 `PartialEq` 比较内层模型的指针，所以地址变了 = 换了模型实例。
/// 这里只收 `apply_view` 每帧路径上的数组；`track-height-override-*` 归宿主的手势路径。
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
    push!("track-ids", get_track_ids);
    push!("track-ys", get_track_ys);
    push!("track-heights", get_track_heights);
    push!("track-volumes", get_track_volumes);
    push!("track-pans", get_track_pans);
    push!("track-mutes", get_track_mutes);
    push!("track-solos", get_track_solos);
    // ⭐ 任务单 `#82`：SFZ 采样器路径也在 `apply_view` 的每帧路径上（⛔ 不是手势路径）
    // ⇒ 必须计入这份清单，否则"又加了一个每帧换模型实例的数组"这类回归会漏数。
    push!("track-sampler-paths", get_track_sampler_paths);
    push!("scene-names", get_scene_names);
    push!("section-names", get_section_names);
    push!("section-positions", get_section_positions);
    push!("section-widths", get_section_widths);
    push!("clip-ulids", get_clip_ulids);
    push!("clip-labels", get_clip_labels);
    push!("clip-positions", get_clip_positions);
    push!("clip-widths", get_clip_widths);
    push!("clip-ys", get_clip_ys);
    push!("clip-heights", get_clip_heights);
    // ⭐ 任务单 `#133`：素材池面板的三个平行数组也在 `apply_view` 的每帧路径上
    // （⛔ 不是手势路径）⇒ 必须计入这份清单，否则"又加了一个每帧换模型实例的数组"会漏数。
    push!("asset-pool-names", get_asset_pool_names);
    push!("asset-pool-kinds", get_asset_pool_kinds);
    push!("asset-pool-details", get_asset_pool_details);
    // ⭐ 任务单 `#157`：设备效果链（Tab 3）的四个平行数组同样在 `apply_view` 的每帧路径上
    // （⛔ 不是手势路径）⇒ 必须计入这份清单，否则"又加了一个每帧换模型实例的数组"会漏数。
    push!("device-names", get_device_names);
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
    push!("bar-positions", get_bar_positions);
    push!("note-ulids", get_note_ulids);
    push!("note-velocities", get_note_velocities);
    push!("note-positions", get_note_positions);
    push!("note-widths", get_note_widths);
    push!("note-ys", get_note_ys);
    push!("note-selected", get_note_selected);
    push!("track-colors", get_track_colors);
    push!("track-color-labels", get_track_color_labels);
    push!(
        "automation-lane-target-keys",
        get_automation_lane_target_keys
    );
    push!("automation-path-commands", get_automation_path_commands);
    push!("track-meter-levels", get_track_meter_levels);
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
    let project = yeban_model::samples::project_with_notes(NOTE_COUNT);
    let total: usize = project
        .clip_pool
        .values()
        .filter_map(|entry| entry.content.notes())
        .map(std::collections::BTreeMap::len)
        .sum();
    assert_eq!(total, NOTE_COUNT, "夹具必须恰好有 10 万个音符");

    let view = ViewState::from_project(&project).expect("10 万音符夹具必须能投影");
    for (frame, expected) in PROBED_FRAMES.iter().zip(VISIBLE_NOTES_PER_FRAME) {
        let visible = view.visible_notes(scroll_at(*frame), VIEWPORT_WIDTH);
        assert_eq!(
            visible.ulids.len(),
            expected,
            "帧 {frame} 的可见音符数必须钉在 {expected}（视口裁剪的读数）"
        );
        assert!(
            visible.positions.len() == expected
                && visible.widths.len() == expected
                && visible.ys.len() == expected
                && visible.velocities.len() == expected,
            "帧 {frame}: 六个平行数组必须共用同一索引集"
        );
    }
    assert_eq!(
        ROLL_PRIMITIVE_BUDGET, 549,
        "卷帘每帧绘制图元预算必须钉在 549（= 2×242 + 65）；改这个常量必须先重新测量"
    );
}

/// 卷帘 `.slint` 里 per-note 循环的条数与固定循环常量 —— 图元公式 `2N + 65` 的两个来源。
///
/// 数 per-note 循环：谁给每个音符再加一个图元（第三条 `for … in root.note-ulids`），这条红。
/// 数固定循环：`5` / `16` / `16` / `16` 一变，`FIXED_ROLL_PRIMITIVES = 65` 的依据就变了。
#[test]
fn roll_primitive_formula_has_exactly_one_loop_per_note_family() {
    let source = include_str!("../ui/console/piano_roll.slint");
    let per_note_loops = source
        .lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with("for ") && line.contains("in root.note-ulids"))
        .count();
    assert_eq!(
        per_note_loops, 2,
        "卷帘只能有两条 per-note 循环（音符矩形 `:166` + 力度柱 `:222`）。\
         现在有 {per_note_loops} 条 ⇒ 每个音符的绘制图元数变了，`2N + {FIXED_ROLL_PRIMITIVES}` 作废。\
         若这一条确实必需：先重新测量绘制回调，再改这条判据与文件头的预算。"
    );
    for literal in [
        "for tool_index in 5 : Rectangle {",
        "for key_index in 16 : Rectangle {",
        "for lane_index in 16 : Rectangle {",
        "for grid_index in 16 : Rectangle {",
    ] {
        assert!(
            source.contains(literal),
            "固定图元计数 `FIXED_ROLL_PRIMITIVES = {FIXED_ROLL_PRIMITIVES}` 依赖这一行：{literal}"
        );
    }
}

/// **断言 ②**：运行时控件树里真的有 2N 个卷帘图元（不是"源码里写着 2N"）。
#[test]
fn roll_primitives_at_runtime_are_pinned_by_the_viewport() {
    use yeban_ui_test_port::image::Size;
    use yeban_ui_test_port::port::Permission;
    use yeban_ui_test_port::render::LivePort;

    let project = yeban_model::samples::project_with_notes(NOTE_COUNT);
    let view = ViewState::from_project(&project).expect("10 万音符夹具必须能投影");
    let scene = DemoScene::from_view(&view);
    let size = Size::new(scene.viewport_width, scene.viewport_height);

    // 先把滚动推进到中点帧，再抓运行时控件树 —— 树里的 `note-*` 就是**真的被物化**的图元。
    let mut port = LivePort::new(size, Permission::ReadOnly, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + MainWindow");
    host::apply_view(
        port.ui(),
        &view,
        VIEWPORT_WIDTH,
        scroll_at(PROBED_FRAMES[1]),
    );
    let tree = port.refresh_tree(None).expect("运行时控件树").clone();

    let notes = tree
        .with_prefix("note-")
        .filter(|node| node.id.ends_with("-rect"))
        .count();
    let velocities = tree
        .with_prefix("velocity-")
        .filter(|node| node.id.ends_with("-bar"))
        .count();
    assert_eq!(
        notes, RUNTIME_NOTE_RECTS,
        "运行时 `note-*-rect` 数必须钉在 {RUNTIME_NOTE_RECTS}。\
         视口裁剪失效会让它变成 {NOTE_COUNT} 这个量级。\
         它比注入的 {} 少一点，因为完全落在父级裁剪矩形之外的图元不进运行时树。",
        VISIBLE_NOTES_PER_FRAME[1]
    );
    assert_eq!(
        velocities, RUNTIME_VELOCITY_BARS,
        "运行时 `velocity-*-bar` 数必须钉在 {RUNTIME_VELOCITY_BARS}。\
         力度柱的 x 比音符多 86 px，所以右端比音符先出界。"
    );
    assert!(
        notes + velocities <= ROLL_PRIMITIVE_BUDGET,
        "卷帘两条 per-note 循环的图元数（{notes} + {velocities}）不得超过预算 {ROLL_PRIMITIVE_BUDGET}"
    );
}

/// **断言 ③**：每帧注入的"重建"量不得增长（源码写入点 + 运行期模型实例）。
#[test]
fn per_frame_injection_does_not_grow() {
    use yeban_ui_test_port::image::Size;
    use yeban_ui_test_port::port::Permission;
    use yeban_ui_test_port::render::LivePort;

    // ---- ③a：源码层的写入点预算 ----
    // ⚠ **行尾归一**：`function_body` 用多行字面量 `"\n}\n"` 找函数体结尾，而 Windows 工作树
    // （无 `.gitattributes` ＋ `core.autocrlf=true`）里源码字节是 CRLF ⇒ ⛔ 匹配不上。
    let source = include_str!("../src/host.rs").replace("\r\n", "\n");
    let apply_view = function_body(&source, "pub fn apply_view(");
    let sites = write_sites(apply_view);
    assert_eq!(
        sites, APPLY_VIEW_WRITE_SITE_BUDGET,
        "`apply_view` 的属性写入点从 {APPLY_VIEW_WRITE_SITE_BUDGET} 变成了 {sites}。\
         每帧多写一个属性 = 每帧多重建一批图元。若这一处确实必需：先重新测量绘制回调，\
         再把常量改到新值，并在提交信息里写出新读数。"
    );
    println!(
        "[perf ③a] apply_view 属性写入点（数 `ui.set_*` 行）= {sites}；预算 = {APPLY_VIEW_WRITE_SITE_BUDGET}"
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
    println!(
        "[perf ③b] 每帧换掉模型实例的数组 = {}；预算 = {PER_FRAME_ARRAY_REWRITE_BUDGET}；清单 = {rewrites:?}",
        rewrites.len()
    );
}
