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
const PER_FRAME_ARRAY_REWRITE_BUDGET: usize = 30;
/// `host.rs` 的 `apply_view` 函数体里的属性写入点个数上限（现在 39）。
const APPLY_VIEW_WRITE_SITE_BUDGET: usize = 39;

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
